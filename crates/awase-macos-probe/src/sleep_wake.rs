//! `sleep-wake` サブコマンド: スリープ/ウェイクをまたいだ常駐診断ログ。
//!
//! NSWorkspace の willSleep / didWake（＋ screens sleep/wake・power-off）通知を購読しつつ、
//! 稼働中 CGEventTap の [`TapHealth`](crate::tap::TapHealth) と
//! [`MacPermissions`](crate::permissions::MacPermissions) を一定間隔で JSONL に記録する。
//!
//! # なぜ live tap を張るのか
//!
//! このサブコマンドの診断価値は「スリープ/ウェイクで CGEventTap が無効化されるか、復帰するか、
//! generation が増えるか」を観測することにある。そのため [`crate::tap::spawn_tap_session`] で
//! 実タップを専用スレッドに起こし、その [`TapHealthHandle`](crate::tap::TapHealthHandle) を
//! 定期スナップショットで lock-free に読む。permissions もスリープ後に TCC が再要求されうるため
//! 併記する。
//!
//! # 検証範囲
//!
//! 通知購読・ログ記録コードはここで完成させるが、**実際の複数回スリープ/ウェイク・画面ロック・
//! 1 時間以上の常駐検証は GitHub Actions runner では不可能**（runner は実際に sleep しない）。
//! 長時間常駐検証は実機検証ゲート（#19）に委譲する。
//!
//! 引数パースと JSON 整形は純粋ロジックとして Linux 上でも単体テストできる。実 API を叩く部分は
//! `#[cfg(target_os = "macos")]` に隔離し、それ以外のホストでは「未対応」を返す。

// NSWorkspace observer の登録・通知定数アクセスに Objective-C FFI（unsafe）が必要。
// unsafe は全て imp(macos) 側の SAFETY コメント付きブロックに閉じ込めてある。
#![allow(unsafe_code)]

use std::path::PathBuf;
use std::time::Duration;

const DEFAULT_INTERVAL_SECS: u64 = 30;
const DEFAULT_LOG_CAPACITY: usize = 1024;

/// `sleep-wake` の設定。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SleepWakeConfig {
    /// 定期スナップショットの間隔。
    pub interval: Duration,
    /// JSONL 出力先。`None` なら stdout。
    pub jsonl_path: Option<PathBuf>,
    /// タップを listen-only（イベントを一切改変しない）で張るか。常駐して長時間動かすため
    /// 既定は `true`（システムを乱さない安全側）。`--active` で完全タップに切り替える。
    pub listen_only: bool,
}

impl Default for SleepWakeConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(DEFAULT_INTERVAL_SECS),
            jsonl_path: None,
            listen_only: true,
        }
    }
}

/// `sleep-wake` の引数列をパースする。
///
/// # Errors
/// 未知のフラグ、値欠落、数値パース失敗時に説明文字列を返す。
pub fn parse_sleep_wake_args(args: &[&str]) -> Result<SleepWakeConfig, String> {
    let mut cfg = SleepWakeConfig::default();
    let mut idx = 0;
    while idx < args.len() {
        match args[idx] {
            "--interval-secs" => {
                idx += 1;
                let value = args
                    .get(idx)
                    .copied()
                    .ok_or_else(|| "--interval-secs requires a value".to_owned())?;
                let secs = value
                    .parse::<u64>()
                    .map_err(|_| format!("--interval-secs expects an integer, got {value}"))?;
                if secs == 0 {
                    return Err("--interval-secs must be greater than 0".to_owned());
                }
                cfg.interval = Duration::from_secs(secs);
            }
            "--jsonl" => {
                idx += 1;
                let value = args
                    .get(idx)
                    .copied()
                    .ok_or_else(|| "--jsonl requires a value".to_owned())?;
                cfg.jsonl_path = Some(PathBuf::from(value));
            }
            "--active" => cfg.listen_only = false,
            other => return Err(format!("unknown sleep-wake argument: {other}")),
        }
        idx += 1;
    }
    Ok(cfg)
}

// --- 純粋な JSON 整形（macOS 実行時と単体テストの両方で使う）------------------------------
// 補間する値は整数か固定の &'static str のみ（ユーザー入力を通さない）なので、JSON
// エスケープは不要。

#[cfg(any(target_os = "macos", test))]
use crate::permissions::{MacPermissions, PermissionStatus};
#[cfg(any(target_os = "macos", test))]
use crate::tap::{TapDisableReason, TapHealth, TapState};

#[cfg(any(target_os = "macos", test))]
const fn permission_str(status: PermissionStatus) -> &'static str {
    match status {
        PermissionStatus::Granted => "granted",
        PermissionStatus::NotGranted => "not_granted",
        PermissionStatus::Unsupported => "unsupported",
    }
}

#[cfg(any(target_os = "macos", test))]
const fn tap_state_str(state: TapState) -> &'static str {
    match state {
        TapState::Starting => "starting",
        TapState::Healthy => "healthy",
        TapState::Disabled(TapDisableReason::Timeout) => "disabled_timeout",
        TapState::Disabled(TapDisableReason::UserInput) => "disabled_user_input",
        TapState::Reenabling => "reenabling",
        TapState::Bypassed => "bypassed",
        TapState::Failed => "failed",
    }
}

#[cfg(any(target_os = "macos", test))]
fn permissions_json(perms: MacPermissions) -> String {
    format!(
        "\"listen_events\":\"{}\",\"post_events\":\"{}\",\"accessibility_client\":\"{}\"",
        permission_str(perms.listen_events),
        permission_str(perms.post_events),
        permission_str(perms.accessibility_client),
    )
}

#[cfg(any(target_os = "macos", test))]
fn tap_health_json(health: &TapHealth) -> String {
    format!(
        "\"tap_state\":\"{}\",\"tap_generation\":{},\"tap_timeout_disables\":{},\"tap_user_input_disables\":{}",
        tap_state_str(health.state),
        health.generation,
        health.disabled_by_timeout_count,
        health.disabled_by_user_input_count,
    )
}

#[cfg(any(target_os = "macos", test))]
fn snapshot_json_line(
    wall_clock_nanos: u64,
    monotonic_nanos: u64,
    perms: MacPermissions,
    health: &TapHealth,
) -> String {
    format!(
        "{{\"record\":\"sleep_wake_snapshot\",\"wall_clock_nanos\":{wall_clock_nanos},\"monotonic_nanos\":{monotonic_nanos},{},{}}}",
        permissions_json(perms),
        tap_health_json(health),
    )
}

#[cfg(any(target_os = "macos", test))]
fn event_json_line(wall_clock_nanos: u64, monotonic_nanos: u64, event: &str) -> String {
    format!(
        "{{\"record\":\"sleep_wake_event\",\"wall_clock_nanos\":{wall_clock_nanos},\"monotonic_nanos\":{monotonic_nanos},\"event\":\"{event}\"}}"
    )
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use std::path::Path;
    use std::time::Duration;

    /// 非 macOS ホストでは NSWorkspace 通知も CGEventTap も無い。引数だけ検証して未対応を返す。
    ///
    /// # Errors
    /// 引数パース失敗時、および常に「unsupported platform」。
    pub fn run_sleep_wake(args: &[&str]) -> anyhow::Result<()> {
        super::parse_sleep_wake_args(args).map_err(|e| anyhow::anyhow!(e))?;
        anyhow::bail!("sleep-wake is only supported on macOS")
    }

    /// 非 macOS スタブ。
    ///
    /// # Errors
    /// 常に「unsupported platform」。
    pub fn run_sleep_wake_monitor(
        _log_path: &Path,
        _poll_interval: Duration,
    ) -> anyhow::Result<()> {
        anyhow::bail!("sleep-wake is only supported on macOS")
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::{event_json_line, snapshot_json_line, DEFAULT_LOG_CAPACITY};
    use crate::focus::FocusState;
    use crate::permissions;
    use crate::report::{wall_clock_now_nanos, EventLogger};
    use crate::runtime::{
        setup_main_application, with_autorelease_pool, workspace_notification_center,
    };
    use crate::tap::{self, TapHealthHandle, TapLocationArg, TapPlacementArg};
    use objc2::rc::Retained;
    use objc2::runtime::{NSObject, NSObjectProtocol};
    use objc2::{define_class, msg_send, sel, AnyThread, DefinedClass};
    use objc2_app_kit::{
        NSWorkspaceDidWakeNotification, NSWorkspaceScreensDidSleepNotification,
        NSWorkspaceScreensDidWakeNotification, NSWorkspaceWillPowerOffNotification,
        NSWorkspaceWillSleepNotification,
    };
    use objc2_foundation::NSNotification;
    use std::io::Write;
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    const STOP_POLL_SLICE: Duration = Duration::from_millis(200);

    /// スレッド安全な JSONL シンク。イベント observer（メインスレッド）と定期スナップショット
    /// スレッドの両方から書く。いずれも低頻度なので `Mutex` で十分。
    struct SleepWakeLog {
        sink: Mutex<Box<dyn Write + Send>>,
        start: Instant,
    }

    impl SleepWakeLog {
        fn new(sink: Box<dyn Write + Send>) -> Self {
            Self {
                sink: Mutex::new(sink),
                start: Instant::now(),
            }
        }

        fn monotonic_nanos(&self) -> u64 {
            u64::try_from(self.start.elapsed().as_nanos()).unwrap_or(u64::MAX)
        }

        fn write_line(&self, line: &str) {
            if let Ok(mut sink) = self.sink.lock() {
                let _ = writeln!(sink, "{line}");
                let _ = sink.flush();
            }
        }
    }

    struct Ivars {
        log: Arc<SleepWakeLog>,
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "AwaseProbeSleepWakeObserver"]
        #[ivars = Ivars]
        struct SleepWakeObserver;

        impl SleepWakeObserver {
            #[unsafe(method(workspaceWillSleep:))]
            fn will_sleep(&self, _notification: &NSNotification) {
                self.record("will_sleep");
            }

            #[unsafe(method(workspaceDidWake:))]
            fn did_wake(&self, _notification: &NSNotification) {
                self.record("did_wake");
            }

            #[unsafe(method(screensDidSleep:))]
            fn screens_did_sleep(&self, _notification: &NSNotification) {
                self.record("screens_did_sleep");
            }

            #[unsafe(method(screensDidWake:))]
            fn screens_did_wake(&self, _notification: &NSNotification) {
                self.record("screens_did_wake");
            }

            #[unsafe(method(willPowerOff:))]
            fn will_power_off(&self, _notification: &NSNotification) {
                self.record("will_power_off");
            }
        }

        unsafe impl NSObjectProtocol for SleepWakeObserver {}
    );

    impl SleepWakeObserver {
        fn new(log: Arc<SleepWakeLog>) -> Retained<Self> {
            let this = Self::alloc().set_ivars(Ivars { log });
            unsafe { msg_send![super(this), init] }
        }

        fn record(&self, event: &str) {
            with_autorelease_pool(|| {
                let log = &self.ivars().log;
                let line = event_json_line(wall_clock_now_nanos(), log.monotonic_nanos(), event);
                log.write_line(&line);
                log::info!("sleep-wake event: {event}");
            });
        }
    }

    /// 登録した observer を保持し、drop 時に登録解除するガード。
    struct ObserverGuard {
        observer: Retained<SleepWakeObserver>,
    }

    impl std::fmt::Debug for ObserverGuard {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("ObserverGuard").finish_non_exhaustive()
        }
    }

    impl Drop for ObserverGuard {
        fn drop(&mut self) {
            let center = workspace_notification_center();
            // SAFETY: 自分が登録した observer を外すだけ。observer はこの呼び出し中有効。
            unsafe { center.removeObserver(&self.observer) };
        }
    }

    fn install_observer(log: Arc<SleepWakeLog>) -> ObserverGuard {
        let observer = SleepWakeObserver::new(log);
        let center = workspace_notification_center();
        // SAFETY: 各 selector は define_class の対応メソッドと一致。通知名は AppKit が公開する
        // extern static。observer は返り値のガードが登録中ずっと生かし続ける。
        unsafe {
            center.addObserver_selector_name_object(
                &observer,
                sel!(workspaceWillSleep:),
                Some(NSWorkspaceWillSleepNotification),
                None,
            );
            center.addObserver_selector_name_object(
                &observer,
                sel!(workspaceDidWake:),
                Some(NSWorkspaceDidWakeNotification),
                None,
            );
            center.addObserver_selector_name_object(
                &observer,
                sel!(screensDidSleep:),
                Some(NSWorkspaceScreensDidSleepNotification),
                None,
            );
            center.addObserver_selector_name_object(
                &observer,
                sel!(screensDidWake:),
                Some(NSWorkspaceScreensDidWakeNotification),
                None,
            );
            center.addObserver_selector_name_object(
                &observer,
                sel!(willPowerOff:),
                Some(NSWorkspaceWillPowerOffNotification),
                None,
            );
        }
        ObserverGuard { observer }
    }

    fn write_snapshot(log: &SleepWakeLog, health: &TapHealthHandle) {
        let perms = permissions::check_all();
        let snapshot = health.snapshot();
        let line = snapshot_json_line(
            wall_clock_now_nanos(),
            log.monotonic_nanos(),
            perms,
            &snapshot,
        );
        log.write_line(&line);
    }

    fn periodic_loop(
        log: &SleepWakeLog,
        stop: &AtomicBool,
        interval: Duration,
        health: &TapHealthHandle,
    ) {
        while !stop.load(Ordering::SeqCst) {
            write_snapshot(log, health);
            let mut slept = Duration::ZERO;
            while slept < interval && !stop.load(Ordering::SeqCst) {
                let slice = STOP_POLL_SLICE.min(interval.saturating_sub(slept));
                std::thread::sleep(slice);
                slept += slice;
            }
        }
    }

    fn open_sink(log_path: Option<&Path>) -> anyhow::Result<Box<dyn Write + Send>> {
        match log_path {
            Some(path) => {
                let file = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)?;
                Ok(Box::new(std::io::BufWriter::new(file)))
            }
            None => Ok(Box::new(std::io::stdout())),
        }
    }

    /// 常駐モニタの本体。`log_path=None` なら stdout へ、`listen_only=true` なら
    /// イベントを改変しないタップを張る。メインスレッドから呼ぶこと。
    fn run_monitor(
        log_path: Option<&Path>,
        poll_interval: Duration,
        listen_only: bool,
    ) -> anyhow::Result<()> {
        let app = setup_main_application()?;

        let log = Arc::new(SleepWakeLog::new(open_sink(log_path)?));

        // 稼働中タップを専用スレッドに起こし、その健全性を定期スナップショットで読む。
        // logger / focus を渡すが、tap-dump 単体同様 focus observer は張らないので focus 帰属は
        // 既定のまま（統合パスが同一 FocusState を共有すれば付く）。
        let logger = Arc::new(EventLogger::new(DEFAULT_LOG_CAPACITY));
        let focus = FocusState::new(Arc::clone(&logger));
        let session = tap::spawn_tap_session(
            TapLocationArg::Session,
            TapPlacementArg::Head,
            listen_only,
            logger,
            focus,
        );
        let health = session.health_handle();

        let stop = Arc::new(AtomicBool::new(false));
        let periodic = {
            let log = Arc::clone(&log);
            let stop = Arc::clone(&stop);
            let health = health.clone();
            std::thread::Builder::new()
                .name("awase-sleepwake-log".to_owned())
                .spawn(move || periodic_loop(&log, &stop, poll_interval, &health))
                .expect("failed to spawn sleep-wake snapshot thread")
        };

        let _guard = install_observer(Arc::clone(&log));
        write_snapshot(&log, &health);
        log::info!(
            "sleep-wake resident logging started (interval={poll_interval:?}, Ctrl-C to stop)"
        );

        // メインスレッドの run loop に入り、terminate されるまでブロックする。
        app.run();

        stop.store(true, Ordering::SeqCst);
        let _ = periodic.join();
        session.shutdown();
        Ok(())
    }

    /// `sleep-wake` サブコマンド本体。`log_path` へ JSONL を書き、`poll_interval` ごとに
    /// スナップショットする常駐モニタ（listen-only タップ）。メインスレッドから呼ぶこと。
    ///
    /// CLI 経由の呼び出しは引数パース済みの [`run_sleep_wake`] が担うため、現状これを
    /// 直接呼ぶ呼び出し元は無い（プログラム的な呼び出し・将来のテストハーネス向け）。
    ///
    /// # Errors
    /// メインスレッド以外から呼ばれた場合、または `log_path` を開けなかった場合。
    #[allow(dead_code)]
    pub fn run_sleep_wake_monitor(log_path: &Path, poll_interval: Duration) -> anyhow::Result<()> {
        run_monitor(Some(log_path), poll_interval, true)
    }

    /// CLI 引数から設定を解釈して常駐モニタを起動する薄いラッパ（`--jsonl` 省略時 stdout、
    /// `--active` で完全タップ）。
    ///
    /// # Errors
    /// 引数パース失敗、メインスレッド外呼び出し、または出力ファイルを開けなかった場合。
    pub fn run_sleep_wake(args: &[&str]) -> anyhow::Result<()> {
        let cfg = super::parse_sleep_wake_args(args).map_err(|e| anyhow::anyhow!(e))?;
        run_monitor(cfg.jsonl_path.as_deref(), cfg.interval, cfg.listen_only)
    }
}

// run_sleep_wake_monitor: 型付きAPI。CLI からは run_sleep_wake 経由でのみ呼ばれる。
#[allow(unused_imports)]
pub use imp::{run_sleep_wake, run_sleep_wake_monitor};

#[cfg(test)]
mod tests {
    use super::{
        event_json_line, parse_sleep_wake_args, permission_str, snapshot_json_line,
        tap_health_json, tap_state_str, SleepWakeConfig,
    };
    use crate::permissions::{MacPermissions, PermissionStatus};
    use crate::tap::{TapDisableReason, TapHealth, TapState};
    use std::path::PathBuf;
    use std::time::Duration;

    #[test]
    fn defaults_are_listen_only_30s_stdout() {
        let cfg = parse_sleep_wake_args(&[]).unwrap();
        assert_eq!(cfg, SleepWakeConfig::default());
        assert_eq!(cfg.interval, Duration::from_secs(30));
        assert!(cfg.listen_only);
        assert_eq!(cfg.jsonl_path, None);
    }

    #[test]
    fn parses_all_flags() {
        let args = [
            "--interval-secs",
            "5",
            "--jsonl",
            "/tmp/sw.jsonl",
            "--active",
        ];
        let cfg = parse_sleep_wake_args(&args).unwrap();
        assert_eq!(cfg.interval, Duration::from_secs(5));
        assert_eq!(cfg.jsonl_path, Some(PathBuf::from("/tmp/sw.jsonl")));
        assert!(!cfg.listen_only);
    }

    #[test]
    fn zero_interval_is_rejected() {
        assert!(parse_sleep_wake_args(&["--interval-secs", "0"]).is_err());
    }

    #[test]
    fn unknown_flag_and_missing_value_error() {
        assert!(parse_sleep_wake_args(&["--nope"]).is_err());
        assert!(parse_sleep_wake_args(&["--jsonl"]).is_err());
        assert!(parse_sleep_wake_args(&["--interval-secs", "x"]).is_err());
    }

    #[test]
    fn permission_and_state_strings() {
        assert_eq!(permission_str(PermissionStatus::Granted), "granted");
        assert_eq!(permission_str(PermissionStatus::NotGranted), "not_granted");
        assert_eq!(permission_str(PermissionStatus::Unsupported), "unsupported");
        assert_eq!(tap_state_str(TapState::Healthy), "healthy");
        assert_eq!(
            tap_state_str(TapState::Disabled(TapDisableReason::Timeout)),
            "disabled_timeout"
        );
        assert_eq!(
            tap_state_str(TapState::Disabled(TapDisableReason::UserInput)),
            "disabled_user_input"
        );
        assert_eq!(tap_state_str(TapState::Bypassed), "bypassed");
    }

    #[test]
    fn snapshot_line_has_expected_fields() {
        let perms = MacPermissions {
            listen_events: PermissionStatus::Granted,
            post_events: PermissionStatus::NotGranted,
            accessibility_client: PermissionStatus::Unsupported,
        };
        let mut health = TapHealth::new();
        health.mark_enabled(std::time::Instant::now());
        health.mark_disabled(TapDisableReason::Timeout, std::time::Instant::now());
        let line = snapshot_json_line(111, 222, perms, &health);
        assert!(line.starts_with("{\"record\":\"sleep_wake_snapshot\""));
        assert!(line.contains("\"wall_clock_nanos\":111"));
        assert!(line.contains("\"monotonic_nanos\":222"));
        assert!(line.contains("\"listen_events\":\"granted\""));
        assert!(line.contains("\"post_events\":\"not_granted\""));
        assert!(line.contains("\"accessibility_client\":\"unsupported\""));
        assert!(line.contains("\"tap_state\":\"disabled_timeout\""));
        assert!(line.contains("\"tap_timeout_disables\":1"));
        assert!(line.ends_with('}'));
    }

    #[test]
    fn event_line_has_expected_fields() {
        let line = event_json_line(7, 8, "will_sleep");
        assert_eq!(
            line,
            "{\"record\":\"sleep_wake_event\",\"wall_clock_nanos\":7,\"monotonic_nanos\":8,\"event\":\"will_sleep\"}"
        );
    }

    #[test]
    fn tap_health_json_shape() {
        let health = TapHealth::new();
        let json = tap_health_json(&health);
        assert!(json.contains("\"tap_state\":\"starting\""));
        assert!(json.contains("\"tap_generation\":0"));
    }
}
