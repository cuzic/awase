//! 診断ログ基盤（ProbeEnvironment／リングバッファ／JSONL 出力）。
//! 設計は project_macos_probe_interfaces.md の「report.rs: 環境情報の記録」を参照。
//!
//! # 2 層設計
//!
//! ホットパス（CGEventTap callback 内）と I/O パスを分離する。
//!
//! * **層 1 — 記録（ホット）**: [`EventLogger::try_push`] は callback 文脈から呼ばれる。
//!   ブロックせず、大きな確保もせず、I/O もしない。`Copy` な [`EventRecord`] を
//!   容量固定のリングバッファへ入れるだけ。満杯・ロック競合時は `dropped_log_count`
//!   を増分してレコードを捨てる（決してブロックしない）。
//! * **層 2 — 書き出し（コールド）**: 通常スレッドから [`EventLogger::drain_and_write`]
//!   を呼ぶ（または [`EventLogger::spawn_writer`] で背景スレッド化）。ここで初めて
//!   JSON 直列化とファイル I/O を行う。出力は JSONL（1 行 1 レコード）。
//!
//! `serde` は依存に加えず、フラットなフィールドだけを手書きで JSON 直列化する。

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// 実機ごとの環境情報。開始時に 1 度だけ記録し、複数機の結果比較に使う。
///
/// OS 由来のフィールド（`macos_version` / `macos_build`）は macOS 上でしか実値が
/// 取れないため、非 macOS ホストではプレースホルダ文字列になる。`architecture` は
/// `std::env::consts::ARCH`（ビルドターゲット由来）なので任意ホストで有効。
/// tap 関連フィールドは probe しても得られないため、呼び出し側が構築時に与える。
#[derive(Debug, Clone)]
pub struct ProbeEnvironment {
    pub macos_version: String,
    pub macos_build: String,
    pub architecture: String,
    pub executable_path: PathBuf,
    pub bundle_identifier: Option<String>,
    pub process_id: u32,
    pub tap_location: String,
    pub tap_placement: String,
    pub tap_options: String,
    pub event_mask: u64,
    pub post_location: String,
}

/// 呼び出し側が [`ProbeEnvironment::capture`] に渡す tap 設定の記述。
///
/// probe では得られない（CGEventTap 構築時の設定値そのものである）ため、
/// 呼び出し側で分かっている文字列/数値をそのまま渡す。
#[derive(Debug, Clone)]
pub struct ProbeTapConfig {
    pub tap_location: String,
    pub tap_placement: String,
    pub tap_options: String,
    pub event_mask: u64,
    pub post_location: String,
}

impl ProbeEnvironment {
    /// OS 由来フィールドを実機から取得しつつ、tap 設定は呼び出し側の値で埋める。
    #[must_use]
    pub fn capture(tap: ProbeTapConfig) -> Self {
        let (macos_version, macos_build) = os_version_build();
        Self {
            macos_version,
            macos_build,
            architecture: std::env::consts::ARCH.to_owned(),
            executable_path: std::env::current_exe().unwrap_or_default(),
            bundle_identifier: bundle_identifier(),
            process_id: std::process::id(),
            tap_location: tap.tap_location,
            tap_placement: tap.tap_placement,
            tap_options: tap.tap_options,
            event_mask: tap.event_mask,
            post_location: tap.post_location,
        }
    }

    /// 環境情報を 1 行の JSON オブジェクト（JSONL の 1 レコード）として直列化する。
    #[must_use]
    pub fn to_json_line(&self) -> String {
        let mut s = String::with_capacity(256);
        s.push_str("{\"record\":\"environment\"");
        json_str_field(&mut s, "macos_version", &self.macos_version);
        json_str_field(&mut s, "macos_build", &self.macos_build);
        json_str_field(&mut s, "architecture", &self.architecture);
        json_str_field(
            &mut s,
            "executable_path",
            &self.executable_path.to_string_lossy(),
        );
        match &self.bundle_identifier {
            Some(id) => json_str_field(&mut s, "bundle_identifier", id),
            None => s.push_str(",\"bundle_identifier\":null"),
        }
        let _ = write!(s, ",\"process_id\":{}", self.process_id);
        json_str_field(&mut s, "tap_location", &self.tap_location);
        json_str_field(&mut s, "tap_placement", &self.tap_placement);
        json_str_field(&mut s, "tap_options", &self.tap_options);
        let _ = write!(s, ",\"event_mask\":{}", self.event_mask);
        json_str_field(&mut s, "post_location", &self.post_location);
        s.push('}');
        s
    }
}

/// ホットパスのイベントレコード。CGEventTap callback 内で構築・push される。
///
/// **`Copy` を保つのが最重要制約**。文字列を持たせない — bundle identifier は
/// [`EventLogger::intern_bundle`] で side-table に登録した u32 index
/// (`bundle_index`) として保持し、drain 時に解決する。フォーカス変更のような
/// 低頻度イベントで intern し、ホットパスでは index を書くだけ。
///
/// フィールドは callback から直接構築できるよう公開する。値はレコード生成側
/// (`keys.rs` / `tap.rs` / `focus.rs`) が観測時点で埋める。
#[derive(Debug, Clone, Copy)]
pub struct EventRecord {
    /// 単調増加クロック（ns 相当）。プロセス基準の経過時間。
    pub monotonic_nanos: u64,
    /// UNIX epoch からの壁時計時刻（ns）。[`wall_clock_now_nanos`] 参照。
    pub wall_clock_nanos: u64,
    /// CGEvent 自身のタイムスタンプ（`CGEventGetTimestamp` 生値）。
    pub cg_event_timestamp: u64,
    /// callback を実行しているスレッドの識別子。
    pub thread_id: u64,
    /// このイベントを観測した入力ストリーム世代（`TapHealth::generation`）。
    pub tap_generation: u64,
    /// 観測時点の focus epoch（遅延到着イベントの並べ替え診断用）。
    pub focus_epoch: u64,
    /// `CGEventGetIntegerValueField(kCGEventSourceUserData)` の生値。synthetic 照合に使う。
    pub source_user_data: i64,
    /// `CGEventType` の生値。
    pub event_type: u32,
    /// 仮想キーコード（`kCGKeyboardEventKeycode`）。
    pub keycode: u16,
    /// 修飾フラグ（`CGEventFlags` 生値）。
    pub flags: u64,
    /// bundle side-table への index。未指定は [`EventRecord::NO_BUNDLE`]。
    pub bundle_index: u32,
    /// オートリピート（`kCGKeyboardEventAutorepeat`）か。
    pub autorepeat: bool,
    /// 自己生成イベントと判定されたか（`SyntheticEventOrigin::is_self_event`）。
    pub is_synthetic: bool,
}

impl EventRecord {
    /// `bundle_index` の「bundle 未指定」を表す番兵値。
    pub const NO_BUNDLE: u32 = u32::MAX;

    fn write_json_line(&self, ids: &[String], out: &mut String) {
        out.push_str("{\"record\":\"event\"");
        let _ = write!(out, ",\"monotonic_nanos\":{}", self.monotonic_nanos);
        let _ = write!(out, ",\"wall_clock_nanos\":{}", self.wall_clock_nanos);
        let _ = write!(out, ",\"cg_event_timestamp\":{}", self.cg_event_timestamp);
        let _ = write!(out, ",\"thread_id\":{}", self.thread_id);
        let _ = write!(out, ",\"tap_generation\":{}", self.tap_generation);
        let _ = write!(out, ",\"focus_epoch\":{}", self.focus_epoch);
        let _ = write!(out, ",\"source_user_data\":{}", self.source_user_data);
        let _ = write!(out, ",\"event_type\":{}", self.event_type);
        let _ = write!(out, ",\"keycode\":{}", self.keycode);
        let _ = write!(out, ",\"flags\":{}", self.flags);
        let _ = write!(out, ",\"autorepeat\":{}", self.autorepeat);
        let _ = write!(out, ",\"is_synthetic\":{}", self.is_synthetic);
        if self.bundle_index == Self::NO_BUNDLE {
            out.push_str(",\"bundle_identifier\":null");
        } else {
            match ids.get(self.bundle_index as usize) {
                Some(id) => json_str_field(out, "bundle_identifier", id),
                None => out.push_str(",\"bundle_identifier\":null"),
            }
        }
        out.push('}');
    }
}

/// bundle identifier 文字列を index に写像する side-table（drain 時に解決）。
#[derive(Debug, Default)]
struct BundleTable {
    ids: Vec<String>,
    lookup: HashMap<String, u32>,
}

/// 2 層診断ロガー。ホットパスの [`Self::try_push`] と、通常スレッドの
/// [`Self::drain_and_write`] を提供する。`Arc` 共有して複数スレッドから使える。
#[derive(Debug)]
pub struct EventLogger {
    capacity: usize,
    buffer: Mutex<Vec<EventRecord>>,
    bundles: Mutex<BundleTable>,
    dropped: AtomicU64,
}

impl EventLogger {
    /// 容量 `capacity` のリングバッファでロガーを作る。
    ///
    /// # Panics
    /// `capacity` が 0 の場合。
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "capacity must be non-zero");
        Self {
            capacity,
            buffer: Mutex::new(Vec::with_capacity(capacity)),
            bundles: Mutex::new(BundleTable::default()),
            dropped: AtomicU64::new(0),
        }
    }

    /// ホットパス用の非ブロッキング push。
    ///
    /// callback 文脈から呼ぶ。ロック競合（`try_lock` 失敗）またはバッファ満杯時は
    /// レコードを捨て `dropped_log_count` を増分するだけで、決してブロックしない。
    /// 追加できたとき `true`、捨てたとき `false` を返す。
    pub fn try_push(&self, record: EventRecord) -> bool {
        if let Ok(mut buf) = self.buffer.try_lock() {
            if buf.len() < self.capacity {
                buf.push(record);
                return true;
            }
        }
        self.dropped.fetch_add(1, Ordering::Relaxed);
        false
    }

    /// bundle identifier を side-table に登録し index を返す（重複は同一 index）。
    ///
    /// フォーカス変更などの低頻度イベントで呼ぶことを想定。ホットパスでは
    /// 返った index を [`EventRecord::bundle_index`] に入れるだけにする。
    /// テーブルが `u32` の限界を超えた場合は [`EventRecord::NO_BUNDLE`] を返す。
    pub fn intern_bundle(&self, bundle_id: &str) -> u32 {
        let mut table = lock_recover(&self.bundles);
        if let Some(&idx) = table.lookup.get(bundle_id) {
            return idx;
        }
        let Ok(idx) = u32::try_from(table.ids.len()) else {
            return EventRecord::NO_BUNDLE;
        };
        table.ids.push(bundle_id.to_owned());
        table.lookup.insert(bundle_id.to_owned(), idx);
        idx
    }

    /// これまでに捨てられたレコード数。
    #[must_use]
    pub fn dropped_log_count(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// 現在バッファに滞留しているレコード数（主にテスト・監視用）。
    #[must_use]
    pub fn pending_len(&self) -> usize {
        lock_recover(&self.buffer).len()
    }

    /// バッファのレコードを取り出し、JSONL として `out` に書き出す。
    ///
    /// 通常スレッドから呼ぶ（層 2）。バッファ取り出しは短時間ロックで行い、
    /// JSON 直列化と I/O はロック外で実施する。書き出したレコード数を返す。
    ///
    /// # Errors
    /// `out` への書き込みが失敗した場合。
    pub fn drain_and_write<W: Write>(&self, out: &mut W) -> io::Result<usize> {
        let drained = {
            let mut buf = lock_recover(&self.buffer);
            std::mem::replace(&mut *buf, Vec::with_capacity(self.capacity))
        };
        if drained.is_empty() {
            return Ok(0);
        }
        // ids のスナップショットを取り、直列化中は bundle ロックを保持しない。
        let ids = lock_recover(&self.bundles).ids.clone();
        let mut text = String::with_capacity(drained.len() * 160);
        for record in &drained {
            record.write_json_line(&ids, &mut text);
            text.push('\n');
        }
        out.write_all(text.as_bytes())?;
        Ok(drained.len())
    }

    /// 背景スレッドで一定間隔ごとに [`Self::drain_and_write`] を回す。
    ///
    /// 返る [`WriterHandle`] を `stop()`（または drop）すると、最終 drain を
    /// 実施してからスレッドを終了する。ファイルは追記モードで開く。
    ///
    /// # Errors
    /// `path` を開けなかった場合。
    pub fn spawn_writer(
        self: &Arc<Self>,
        path: PathBuf,
        interval: Duration,
    ) -> io::Result<WriterHandle> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        let logger = Arc::clone(self);
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        let join = std::thread::spawn(move || {
            let mut writer = io::BufWriter::new(file);
            while !stop_thread.load(Ordering::Relaxed) {
                let _ = logger.drain_and_write(&mut writer);
                let _ = writer.flush();
                std::thread::sleep(interval);
            }
            let _ = logger.drain_and_write(&mut writer);
            let _ = writer.flush();
        });
        Ok(WriterHandle {
            stop,
            join: Some(join),
        })
    }
}

/// [`EventLogger::spawn_writer`] が返す背景ライタの制御ハンドル。drop で停止する。
#[derive(Debug)]
pub struct WriterHandle {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl WriterHandle {
    /// 背景ライタに停止を指示し、最終 drain 完了まで待ち合わせる。
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

impl Drop for WriterHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// UNIX epoch からの現在時刻を ns で返す（壁時計）。ホットパスで
/// [`EventRecord::wall_clock_nanos`] を埋めるためのヘルパ。
#[must_use]
pub fn wall_clock_now_nanos() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX))
}

/// Mutex を poison から回復して lock する（診断ロガーはデータが一貫していれば
/// poison を致命扱いしない）。
fn lock_recover<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// `,"key":"<escaped value>"` を `out` に追記する。
fn json_str_field(out: &mut String, key: &str, value: &str) {
    out.push(',');
    json_escape(key, out);
    out.push(':');
    json_escape(value, out);
}

/// JSON 文字列リテラル（両端の `"` 込み）として `s` を `out` に追記する。
fn json_escape(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(target_os = "macos")]
fn os_version_build() -> (String, String) {
    use objc2_foundation::NSProcessInfo;

    let info = NSProcessInfo::processInfo();
    let v = info.operatingSystemVersion();
    let version = format!("{}.{}.{}", v.majorVersion, v.minorVersion, v.patchVersion);
    // operatingSystemVersionString は "Version 14.5 (Build 23F79)" 形式。
    // 括弧内 "Build XXXX" から build 番号を抽出する。
    let full = info.operatingSystemVersionString().to_string();
    let build = full
        .split_once("Build ")
        .and_then(|(_, rest)| rest.split(')').next())
        .map_or_else(|| full.clone(), |b| b.trim().to_owned());
    (version, build)
}

#[cfg(not(target_os = "macos"))]
fn os_version_build() -> (String, String) {
    (
        "unknown (non-macos host)".to_owned(),
        "unknown (non-macos host)".to_owned(),
    )
}

#[cfg(target_os = "macos")]
fn bundle_identifier() -> Option<String> {
    use objc2_foundation::NSBundle;

    let bundle = NSBundle::mainBundle();
    bundle.bundleIdentifier().map(|id| id.to_string())
}

#[cfg(not(target_os = "macos"))]
const fn bundle_identifier() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_record(keycode: u16, bundle_index: u32) -> EventRecord {
        EventRecord {
            monotonic_nanos: 1,
            wall_clock_nanos: 2,
            cg_event_timestamp: 3,
            thread_id: 4,
            tap_generation: 5,
            focus_epoch: 6,
            source_user_data: -7,
            event_type: 10,
            keycode,
            flags: 0x1_0000,
            bundle_index,
            autorepeat: false,
            is_synthetic: true,
        }
    }

    #[test]
    fn push_until_full_increments_dropped() {
        let logger = EventLogger::new(3);
        assert!(logger.try_push(sample_record(0, EventRecord::NO_BUNDLE)));
        assert!(logger.try_push(sample_record(1, EventRecord::NO_BUNDLE)));
        assert!(logger.try_push(sample_record(2, EventRecord::NO_BUNDLE)));
        // 4 個目以降は満杯で捨てられる。
        assert!(!logger.try_push(sample_record(3, EventRecord::NO_BUNDLE)));
        assert!(!logger.try_push(sample_record(4, EventRecord::NO_BUNDLE)));
        assert_eq!(logger.dropped_log_count(), 2);
        assert_eq!(logger.pending_len(), 3);
    }

    #[test]
    fn drain_returns_records_in_order_then_empties() {
        let logger = EventLogger::new(8);
        for k in 0..5u16 {
            assert!(logger.try_push(sample_record(k, EventRecord::NO_BUNDLE)));
        }
        let mut out: Vec<u8> = Vec::new();
        let n = logger.drain_and_write(&mut out).unwrap();
        assert_eq!(n, 5);
        let text = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 5);
        for (k, line) in lines.iter().enumerate() {
            assert!(line.contains(&format!("\"keycode\":{k}")), "line: {line}");
            assert!(line.starts_with("{\"record\":\"event\""));
        }
        // drain 後は空。
        assert_eq!(logger.pending_len(), 0);
        let mut again: Vec<u8> = Vec::new();
        assert_eq!(logger.drain_and_write(&mut again).unwrap(), 0);
        assert!(again.is_empty());
    }

    #[test]
    fn intern_bundle_dedups_and_resolves_at_drain() {
        let logger = EventLogger::new(8);
        let a1 = logger.intern_bundle("com.apple.Safari");
        let a2 = logger.intern_bundle("com.apple.Safari");
        let b = logger.intern_bundle("com.google.Chrome");
        assert_eq!(a1, a2);
        assert_ne!(a1, b);

        logger.try_push(sample_record(0, a1));
        logger.try_push(sample_record(1, b));
        logger.try_push(sample_record(2, EventRecord::NO_BUNDLE));

        let mut out: Vec<u8> = Vec::new();
        logger.drain_and_write(&mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[0].contains("\"bundle_identifier\":\"com.apple.Safari\""));
        assert!(lines[1].contains("\"bundle_identifier\":\"com.google.Chrome\""));
        assert!(lines[2].contains("\"bundle_identifier\":null"));
    }

    #[test]
    fn json_escape_handles_quotes_backslash_and_controls() {
        let mut out = String::new();
        json_escape("a\"b\\c\nd\te\r\u{08}\u{0c}\u{01}z", &mut out);
        assert_eq!(out, "\"a\\\"b\\\\c\\nd\\te\\r\\b\\f\\u0001z\"");
    }

    #[test]
    fn json_escape_plain_and_unicode_passthrough() {
        let mut out = String::new();
        json_escape("かなカナ 🍎 abc", &mut out);
        // BMP 外・非 ASCII はエスケープせずそのまま通す（制御文字のみエスケープ）。
        assert_eq!(out, "\"かなカナ 🍎 abc\"");
    }

    #[test]
    fn environment_json_line_is_single_line_and_labeled() {
        let env = ProbeEnvironment::capture(ProbeTapConfig {
            tap_location: "HID".to_owned(),
            tap_placement: "HeadInsert".to_owned(),
            tap_options: "Default".to_owned(),
            event_mask: 0b11,
            post_location: "HID".to_owned(),
        });
        let line = env.to_json_line();
        assert!(line.starts_with("{\"record\":\"environment\""));
        assert!(line.ends_with('}'));
        assert!(!line.contains('\n'));
        assert!(line.contains("\"architecture\":"));
        assert!(line.contains("\"tap_location\":\"HID\""));
        assert!(line.contains("\"event_mask\":3"));
    }

    #[test]
    fn writer_thread_flushes_records_to_file() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("awase-probe-writer-{}.jsonl", std::process::id()));
        let _ = std::fs::remove_file(&path);

        let logger = Arc::new(EventLogger::new(64));
        let handle = logger
            .spawn_writer(path.clone(), Duration::from_millis(5))
            .unwrap();
        for k in 0..10u16 {
            logger.try_push(sample_record(k, EventRecord::NO_BUNDLE));
        }
        // stop は最終 drain を待ち合わせる。
        handle.stop();

        let content = std::fs::read_to_string(&path).unwrap();
        let line_count = content.lines().filter(|l| !l.is_empty()).count();
        assert_eq!(line_count, 10, "content: {content}");
        let _ = std::fs::remove_file(&path);
    }
}
