//! `awase.log` 用の自前ローテーション付き `BufWriter`（ADR-139 決定2）。
//!
//! `tracing-appender::rolling` は時間ベース（MINUTELY/HOURLY/DAILY/…）のローテーション
//! しか持たず、決定2の実測根拠（747MB/単一セッション、`docs/adr/125-*.md:287`）に合わない。
//! さらにローテーション後のファイル名が日付サフィックス付きになるため、
//! `app::bug_report_log_path()`/`app_log_path.exists()` が前提とする「固定パス
//! `awase.log` が常に存在する」という契約を壊す。ここでは固定パスを維持したまま、
//! 書き込みバイト数が閾値を超えるたびに `awase.log` → `awase.log.old` へ1世代だけ
//! リネームする軽量な自前ローテーションを実装する。
//!
//! Windows では開いたままの `File` を `rename` できない（`FILE_SHARE_DELETE` 無しでは
//! `ERROR_SHARING_VIOLATION`）ため、ローテーション時は必ず
//! 「flush → 内側 `File` を drop → rename → 新規 `File` を開いて差し替え」の順序を守る。

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

/// 単一世代ローテーションの閾値。747MB の実測（`RUST_LOG=debug` の長時間デバッグ
/// セッション、`docs/adr/125-*.md:287`）に対し、通常運用で数日〜数週間気付かずに
/// 溜まっても許容できる範囲として 20MB を採用する（タイミング定数ではないため
/// `.claude/rules/tuning-constants.md` の実測義務の対象外）。
const MAX_LOG_BYTES: u64 = 20 * 1024 * 1024;

struct RotatingLogState {
    path: PathBuf,
    file: Option<BufWriter<File>>,
    written_since_open: u64,
    max_bytes: u64,
}

impl RotatingLogState {
    fn open_file(path: &std::path::Path) -> io::Result<BufWriter<File>> {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(BufWriter::new(file))
    }

    fn backup_path(&self) -> PathBuf {
        let mut s = self.path.as_os_str().to_os_string();
        s.push(".old");
        PathBuf::from(s)
    }

    /// 書き込みバイト数が閾値を超えていれば1世代だけローテーションする。
    /// 呼び出しのたびに閾値と比較するだけなので、起動時に限らずセッション中も
    /// 継続的にチェックされる（別途ウォッチドッグ等を必要としない）。
    fn maybe_rotate(&mut self) {
        if self.written_since_open < self.max_bytes {
            return;
        }
        if let Some(mut f) = self.file.take() {
            let _ = f.flush();
            // f をここで drop してハンドルを閉じる（Windows で rename するために必須）。
        }
        let backup = self.backup_path();
        let _ = fs::remove_file(&backup);
        let _ = fs::rename(&self.path, &backup);
        self.file = Self::open_file(&self.path).ok();
        self.written_since_open = 0;
    }
}

/// `RotatingLogState` を共有する `Write` ハンドル。`tracing_subscriber::fmt::MakeWriter`
/// はイベントごとにこれを1つ生成し、書き込み直後に drop する。
pub(crate) struct RotatingLogHandle {
    state: Arc<Mutex<RotatingLogState>>,
    /// WARN 以上のイベントでは drop 時に自動 flush する
    /// （クラッシュ直前の最も価値の高い行を `BufWriter` 内に残さないため）。
    flush_on_drop: bool,
}

impl Write for RotatingLogHandle {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let Ok(mut guard) = self.state.lock() else {
            return Ok(buf.len());
        };
        guard.maybe_rotate();
        let Some(file) = guard.file.as_mut() else {
            // 開き直しに失敗している場合は黙って捨てる（stderr フォールバックは無い —
            // GUI サブシステムでは他に出しようがないため、次回 maybe_rotate の
            // 再試行に賭ける）。
            return Ok(buf.len());
        };
        let n = file.write(buf)?;
        guard.written_since_open += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        let Ok(mut guard) = self.state.lock() else {
            return Ok(());
        };
        guard.file.as_mut().map_or(Ok(()), BufWriter::flush)
    }
}

impl Drop for RotatingLogHandle {
    fn drop(&mut self) {
        if self.flush_on_drop {
            let _ = self.flush();
        }
    }
}

/// `tracing_subscriber::fmt` に渡す `MakeWriter`。
#[derive(Clone)]
pub(crate) struct RotatingLogWriter {
    state: Arc<Mutex<RotatingLogState>>,
}

impl RotatingLogWriter {
    /// `awase.log` を開いてグローバルハンドル（[`flush_log_writer`] が参照する）に
    /// 登録し、`tracing_subscriber::fmt` の writer として返す。
    pub(crate) fn init_global(path: PathBuf) -> Self {
        let state = Arc::new(Mutex::new(RotatingLogState {
            file: RotatingLogState::open_file(&path).ok(),
            path,
            written_since_open: 0,
            max_bytes: MAX_LOG_BYTES,
        }));
        // 2度目の初期化（テスト等）では既存ハンドルを保持したままにする。
        let _ = LOG_WRITER_STATE.set(Arc::clone(&state));
        Self { state }
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for RotatingLogWriter {
    type Writer = RotatingLogHandle;

    fn make_writer(&'a self) -> Self::Writer {
        RotatingLogHandle {
            state: Arc::clone(&self.state),
            flush_on_drop: false,
        }
    }

    fn make_writer_for(&'a self, meta: &tracing::Metadata<'_>) -> Self::Writer {
        RotatingLogHandle {
            state: Arc::clone(&self.state),
            flush_on_drop: meta.level() <= &tracing::Level::WARN,
        }
    }
}

static LOG_WRITER_STATE: OnceLock<Arc<Mutex<RotatingLogState>>> = OnceLock::new();

/// 不具合報告の起動（`launch_bug_report`）直前に呼ぶ。`awase.log` を読むのは
/// `--applog` 引数で起動される別プロセス（awase-settings.exe）であり、awase.exe が
/// 保持する `BufWriter` の内容にはそちらから触れられないため、awase.exe 側で
/// 明示的に flush してから起動する必要がある。
///
/// この関数の実装内では `tracing::*!` マクロを呼ばないこと —
/// `RotatingLogHandle`/`RotatingLogState` が握る `Mutex` は再入不可であり、
/// ログ出力側も同じ Mutex を取得しようとするため、flush の内側でログを出すと
/// 自己デッドロックする。
pub(crate) fn flush_log_writer() {
    if let Some(state) = LOG_WRITER_STATE.get() {
        if let Ok(mut guard) = state.lock() {
            if let Some(file) = guard.file.as_mut() {
                let _ = file.flush();
            }
        }
    }
}
