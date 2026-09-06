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
    ///
    /// `self.file` が `None`（前回の open/再open に失敗している）場合は、
    /// ローテーション判定より先に再openを試みる。`written_since_open` の
    /// 閾値判定だけに再open条件を委ねると、一度でも open に失敗した時点で
    /// `written_since_open` が0のまま凍りつき、以後 `maybe_rotate` が常に
    /// 早期returnして**再open が永久に試みられなくなる**（PRコードレビューで
    /// 検出。旧 `env_logger` にあった「ファイルが開けない場合は stderr
    /// フォールバック」に相当する代替の可視化が無いままログが無言で死ぬ）。
    fn maybe_rotate(&mut self) {
        if self.file.is_none() {
            self.file = Self::open_file(&self.path).ok();
            self.written_since_open = Self::current_file_len(&self.path);
            return;
        }
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

    /// 既存ファイルの実サイズを返す（無ければ0）。`awase.log` は
    /// `OpenOptions::append(true)` で開くため、プロセス再起動をまたいで
    /// 中身が引き継がれる。`written_since_open` を常に0から数え始めると、
    /// 常駐トレイアプリの日常的な再起動（設定変更・アップデート等）を
    /// またいだ蓄積を検知できず、1セッションあたりの書き込み量が閾値未満の
    /// ユーザーでは `awase.log` がローテーションされないまま無制限に育つ
    /// （PRコードレビューで検出）。
    fn current_file_len(path: &std::path::Path) -> u64 {
        fs::metadata(path).map_or(0, |m| m.len())
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
        // `Mutex` poisoning（過去に lock 保持中に panic した）が起きても、
        // ログ機能そのものを永久に沈黙させない（PRコードレビュー指摘、B-3と
        // 同種の「一度の異常で以後ずっと無音」を避けるための回復）。
        let n = {
            let mut guard = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            guard.maybe_rotate();
            let Some(file) = guard.file.as_mut() else {
                // open_file が失敗した直後（maybe_rotate内で再open済みだが失敗）。
                // 次回 write() 呼び出し時に maybe_rotate が再open を再試行する。
                return Ok(buf.len());
            };
            let n = file.write(buf)?;
            guard.written_since_open += n as u64;
            n
        };
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut guard = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let written_since_open = RotatingLogState::current_file_len(&path);
        let state = Arc::new(Mutex::new(RotatingLogState {
            file: RotatingLogState::open_file(&path).ok(),
            path,
            written_since_open,
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
        let mut guard = state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(file) = guard.file.as_mut() {
            let _ = file.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read as _;

    /// テストごとに衝突しない一時パスを返す。呼び出し元が使い終わったら
    /// 本体・`.old` とも削除すること。
    fn unique_temp_path(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "awase_logging_test_{tag}_{}_{nanos}.log",
            std::process::id()
        ))
    }

    /// B-2 回帰テスト: 既存ファイルにサイズがある状態で `init_global` すると、
    /// `written_since_open` がそのサイズから始まる（0から数え直さない）。
    /// 常駐アプリの日常的な再起動をまたいだ肥大化を検知するために必須。
    #[test]
    fn init_global_seeds_written_since_open_from_existing_file_size() {
        let path = unique_temp_path("seed_from_size");
        fs::write(&path, vec![b'x'; 1234]).unwrap();

        let writer = RotatingLogWriter::init_global(path.clone());
        let written = writer.state.lock().unwrap().written_since_open;

        let _ = fs::remove_file(&path);
        assert_eq!(
            written, 1234,
            "既存ファイルサイズがwritten_since_openの初期値に反映されていない(B-2)"
        );
    }

    /// B-3 回帰テスト: `file` が `None`（前回のopen失敗を模す）の状態から
    /// `maybe_rotate` を呼ぶと、`written_since_open` を凍りつかせず再open を
    /// 試みる。修正前は `written_since_open` が0のまま固定され、
    /// 「次回再試行する」というコメントの約束が構造的に果たされなかった。
    #[test]
    fn maybe_rotate_recovers_when_file_handle_is_none() {
        let path = unique_temp_path("recover_from_none");
        fs::write(&path, vec![b'x'; 42]).unwrap();

        let mut state = RotatingLogState {
            path: path.clone(),
            file: None, // open失敗を模す
            written_since_open: 0,
            max_bytes: MAX_LOG_BYTES,
        };
        state.maybe_rotate();

        let _ = fs::remove_file(&path);
        assert!(
            state.file.is_some(),
            "file=Noneからmaybe_rotateがファイルを再openしていない(B-3)"
        );
        assert_eq!(
            state.written_since_open, 42,
            "再open時にwritten_since_openが実ファイルサイズに同期していない(B-2/B-3)"
        );
    }

    /// ローテーション自体の基本動作: 閾値超過で `.old` へ退避し、新規ファイルは
    /// 空から始まる。
    #[test]
    fn maybe_rotate_moves_to_backup_and_starts_fresh_file_when_over_threshold() {
        let path = unique_temp_path("rotate_over_threshold");
        let backup = {
            let mut s = path.as_os_str().to_os_string();
            s.push(".old");
            PathBuf::from(s)
        };
        fs::write(&path, b"old-content").unwrap();

        let mut state = RotatingLogState {
            file: RotatingLogState::open_file(&path).ok(),
            path: path.clone(),
            written_since_open: 100, // 閾値超過を模す
            max_bytes: 10,
        };
        state.maybe_rotate();
        // BufWriter経由で実ファイルへ反映させるため明示flushしてから読む。
        if let Some(f) = state.file.as_mut() {
            f.flush().unwrap();
        }

        let mut backup_content = String::new();
        File::open(&backup)
            .unwrap()
            .read_to_string(&mut backup_content)
            .unwrap();

        let result = (
            backup_content,
            fs::metadata(&path).map(|m| m.len()).unwrap_or(0),
            state.written_since_open,
        );

        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&backup);

        assert_eq!(result.0, "old-content", "旧内容が.oldへ退避されていない");
        assert_eq!(result.1, 0, "ローテーション後の新規ファイルが空でない");
        assert_eq!(
            result.2, 0,
            "ローテーション後にwritten_since_openがリセットされていない"
        );
    }
}
