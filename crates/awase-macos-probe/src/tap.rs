//! CGEventTap のライフサイクル管理（[`TapState`] / [`TapHealth`] / disabled 復旧）と、
//! `tap-dump` / `tap-recover` サブコマンドの実処理。
//!
//! 設計は project_macos_probe_interfaces.md の「CGEventTap復旧: ContextChangeバリアント
//! 名を一般化」を参照。要点:
//!
//! - タップ callback は C 境界を越える。Rust の panic を C に伝播させないため、本体は
//!   必ず [`std::panic::catch_unwind`] で包む。panic 時はイベントを素通しして継続する。
//! - `disabled`（timeout / user-input）を検知したら、まず未解放の synthetic キーを
//!   best-effort で key-up 送出（stuck key 防止）してから再有効化する。generation は
//!   「正常に開始した入力ストリーム世代」で、再有効化を確認できたときにインクリメントする。
//! - 短時間に閾値回数 disabled が繰り返されたら再有効化を諦めて [`TapState::Bypassed`] に
//!   移行し、ユーザーに通知する（暴走ループでシステムを重くしないため）。
//!
//! 状態遷移・bypass 判定・引数パースは純粋ロジックとして Linux 上でも単体テストできる。
//! 実 API（`CGEventTapCreate` 等）を叩く部分は全て `#[cfg(target_os = "macos")]` に隔離し、
//! それ以外のホストでは「未対応」を返してコンパイルを通す。

// 型名が module 名 `tap` を反復する（TapState/TapHealth 等）が、これは
// project_macos_probe_interfaces.md が定めた確定インタフェース名なのでそのまま使う。
#![allow(clippy::module_name_repetitions)]
// CGEventTap の生成・有効化・callback 境界は Carbon/CoreGraphics の unsafe FFI が必須。
// unsafe は全て platform(macos) 側の SAFETY コメント付きブロックに閉じ込めてある。
// crate 全体の `-D warnings` ゲート下で unsafe_code=warn がエラー化しないよう許可する
// （tis_sys / permissions / runtime / focus と同じ規約）。
#![allow(unsafe_code)]

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// タップが無効化された理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TapDisableReason {
    /// callback が制限時間を超過して OS に切られた（`kCGEventTapDisabledByTimeout`）。
    Timeout,
    /// ユーザー入力の連打等で OS に切られた（`kCGEventTapDisabledByUserInput`）。
    UserInput,
}

/// タップの現在状態。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TapState {
    /// 生成直後、まだ有効化確認前。
    Starting,
    /// 有効に稼働中。
    Healthy,
    /// 無効化された（理由付き）。
    Disabled(TapDisableReason),
    /// 再有効化を試行中。
    Reenabling,
    /// 再有効化を諦めて素通し状態（暴走防止）。
    Bypassed,
    /// 生成・有効化に失敗して機能していない。
    Failed,
}

/// タップの健全性スナップショット。
///
/// `generation` は「正常に開始した入力ストリーム世代」。初回有効化で 1 になり、
/// disabled からの再有効化を確認できるたびにインクリメントする。診断ログの
/// `tap_generation` はこの値で、断絶をまたいだイベントを区別するために使う。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TapHealth {
    pub generation: u64,
    pub state: TapState,
    pub disabled_by_timeout_count: u64,
    pub disabled_by_user_input_count: u64,
    pub last_disabled_at: Option<Instant>,
    pub last_enabled_at: Option<Instant>,
}

impl TapHealth {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            generation: 0,
            state: TapState::Starting,
            disabled_by_timeout_count: 0,
            disabled_by_user_input_count: 0,
            last_disabled_at: None,
            last_enabled_at: None,
        }
    }

    /// 有効化（初回・再有効化とも）が確認できたときに呼ぶ。generation を進め Healthy にする。
    pub const fn mark_enabled(&mut self, now: Instant) {
        self.generation += 1;
        self.state = TapState::Healthy;
        self.last_enabled_at = Some(now);
    }

    /// callback が disabled イベントを観測したときに呼ぶ。理由別カウンタを進める。
    pub const fn mark_disabled(&mut self, reason: TapDisableReason, now: Instant) {
        match reason {
            TapDisableReason::Timeout => self.disabled_by_timeout_count += 1,
            TapDisableReason::UserInput => self.disabled_by_user_input_count += 1,
        }
        self.last_disabled_at = Some(now);
        self.state = TapState::Disabled(reason);
    }

    /// 再有効化の試行に入ったことを記録する。
    pub const fn mark_reenabling(&mut self) {
        self.state = TapState::Reenabling;
    }

    /// 再有効化を諦めた（bypass）ことを記録する。
    pub const fn mark_bypassed(&mut self) {
        self.state = TapState::Bypassed;
    }

    /// 生成・有効化に失敗したことを記録する。
    pub const fn mark_failed(&mut self) {
        self.state = TapState::Failed;
    }
}

impl Default for TapHealth {
    fn default() -> Self {
        Self::new()
    }
}

/// [`TapState`] を atomic ミラー用の `u8` に符号化する。driver スレッドが lock-free に
/// 状態を読めるようにするための表現で、`Disabled` の理由は落とす（カウンタ側で保持する）。
#[must_use]
pub const fn state_code(state: TapState) -> u8 {
    match state {
        TapState::Starting => STATE_STARTING,
        TapState::Healthy => STATE_HEALTHY,
        TapState::Disabled(_) => STATE_DISABLED,
        TapState::Reenabling => STATE_REENABLING,
        TapState::Bypassed => STATE_BYPASSED,
        TapState::Failed => STATE_FAILED,
    }
}

pub const STATE_STARTING: u8 = 0;
pub const STATE_HEALTHY: u8 = 1;
pub const STATE_DISABLED: u8 = 2;
pub const STATE_REENABLING: u8 = 3;
pub const STATE_BYPASSED: u8 = 4;
pub const STATE_FAILED: u8 = 5;

/// 短時間に閾値回数 disabled が起きたら bypass すべきと判定する検出器。
///
/// disabled のたびに [`Self::record`] を呼ぶ。`window` より古い記録は捨て、`window`
/// 内の記録数が `threshold` 以上になったら `true`（bypass せよ）を返す。
#[derive(Debug)]
struct BypassDetector {
    window: Duration,
    threshold: usize,
    recent: VecDeque<Instant>,
}

impl BypassDetector {
    const fn new(window: Duration, threshold: usize) -> Self {
        Self {
            window,
            threshold,
            recent: VecDeque::new(),
        }
    }

    /// `now` に disabled が起きたと記録し、bypass すべきなら `true` を返す。
    fn record(&mut self, now: Instant) -> bool {
        self.recent.push_back(now);
        while let Some(&front) = self.recent.front() {
            if now.duration_since(front) > self.window {
                self.recent.pop_front();
            } else {
                break;
            }
        }
        self.recent.len() >= self.threshold
    }
}

/// `--location` の値。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TapLocationArg {
    /// `kCGSessionEventTap`（既定）。
    #[default]
    Session,
    /// `kCGHIDEventTap`。
    Hid,
    /// `kCGAnnotatedSessionEventTap`。
    Annotated,
}

/// `--placement` の値。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TapPlacementArg {
    /// `kCGHeadInsertEventTap`（既定。他タップより前で観測・改変できる）。
    #[default]
    Head,
    /// `kCGTailAppendEventTap`。
    Tail,
}

/// `tap-dump` の設定。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TapDumpConfig {
    pub location: TapLocationArg,
    pub placement: TapPlacementArg,
    pub listen_only: bool,
    pub jsonl_path: Option<PathBuf>,
    pub duration: Option<Duration>,
}

/// `tap-recover` の設定。安全装置（`auto_exit_after`）は既定で有効。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TapRecoverConfig {
    pub delay: Duration,
    pub once: bool,
    pub listen_only: bool,
    pub auto_exit_after: Option<Duration>,
}

impl Default for TapRecoverConfig {
    fn default() -> Self {
        Self {
            delay: Duration::from_millis(DEFAULT_RECOVER_DELAY_MS),
            once: false,
            listen_only: false,
            auto_exit_after: Some(Duration::from_secs(DEFAULT_RECOVER_AUTO_EXIT_SECS)),
        }
    }
}

const DEFAULT_RECOVER_DELAY_MS: u64 = 1000;
const DEFAULT_RECOVER_AUTO_EXIT_SECS: u64 = 10;
const DEFAULT_LOG_CAPACITY: usize = 4096;
const BYPASS_WINDOW: Duration = Duration::from_secs(5);
const BYPASS_THRESHOLD: usize = 5;
const WRITER_FLUSH_INTERVAL: Duration = Duration::from_millis(250);
const STATUS_POLL: Duration = Duration::from_millis(200);

fn take_value<'a>(args: &[&'a str], idx: &mut usize, flag: &str) -> Result<&'a str, String> {
    *idx += 1;
    args.get(*idx)
        .copied()
        .ok_or_else(|| format!("{flag} requires a value"))
}

/// `tap-dump` の引数列をパースする。
///
/// # Errors
/// 未知のフラグ、値を要求するフラグの値欠落、数値/列挙値のパース失敗時に説明文字列を返す。
pub fn parse_tap_dump_args(args: &[&str]) -> Result<TapDumpConfig, String> {
    let mut cfg = TapDumpConfig::default();
    let mut idx = 0;
    while idx < args.len() {
        match args[idx] {
            "--location" => {
                cfg.location = parse_location(take_value(args, &mut idx, "--location")?)?;
            }
            "--placement" => {
                cfg.placement = parse_placement(take_value(args, &mut idx, "--placement")?)?;
            }
            "--listen-only" => cfg.listen_only = true,
            "--jsonl" => {
                cfg.jsonl_path = Some(PathBuf::from(take_value(args, &mut idx, "--jsonl")?));
            }
            "--duration" => {
                let secs = parse_u64(take_value(args, &mut idx, "--duration")?, "--duration")?;
                cfg.duration = Some(Duration::from_secs(secs));
            }
            other => return Err(format!("unknown tap-dump argument: {other}")),
        }
        idx += 1;
    }
    Ok(cfg)
}

/// `tap-recover` の引数列をパースする。
///
/// # Errors
/// 未知のフラグ、値欠落、数値のパース失敗時に説明文字列を返す。
pub fn parse_tap_recover_args(args: &[&str]) -> Result<TapRecoverConfig, String> {
    let mut cfg = TapRecoverConfig::default();
    let mut idx = 0;
    while idx < args.len() {
        match args[idx] {
            "--delay-ms" => {
                let ms = parse_u64(take_value(args, &mut idx, "--delay-ms")?, "--delay-ms")?;
                cfg.delay = Duration::from_millis(ms);
            }
            "--once" => cfg.once = true,
            "--listen-only" => cfg.listen_only = true,
            "--auto-exit-after-secs" => {
                let secs = parse_u64(
                    take_value(args, &mut idx, "--auto-exit-after-secs")?,
                    "--auto-exit-after-secs",
                )?;
                cfg.auto_exit_after = Some(Duration::from_secs(secs));
            }
            other => return Err(format!("unknown tap-recover argument: {other}")),
        }
        idx += 1;
    }
    Ok(cfg)
}

fn parse_location(value: &str) -> Result<TapLocationArg, String> {
    match value {
        "session" => Ok(TapLocationArg::Session),
        "hid" => Ok(TapLocationArg::Hid),
        "annotated" => Ok(TapLocationArg::Annotated),
        other => Err(format!(
            "--location must be session|hid|annotated, got {other}"
        )),
    }
}

fn parse_placement(value: &str) -> Result<TapPlacementArg, String> {
    match value {
        "head" => Ok(TapPlacementArg::Head),
        "tail" => Ok(TapPlacementArg::Tail),
        other => Err(format!("--placement must be head|tail, got {other}")),
    }
}

fn parse_u64(value: &str, flag: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .map_err(|_| format!("{flag} expects a non-negative integer, got {value}"))
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::{parse_tap_dump_args, parse_tap_recover_args};

    /// 非 macOS ホストでは実タップを張れない。引数の妥当性だけ検証して未対応を通知する。
    ///
    /// # Errors
    /// 引数パースに失敗した場合。
    pub fn run_tap_dump(args: &[&str]) -> anyhow::Result<()> {
        parse_tap_dump_args(args).map_err(|e| anyhow::anyhow!(e))?;
        log::error!("tap-dump is only supported on macOS");
        Ok(())
    }

    /// 非 macOS ホストでは実タップを張れない。引数の妥当性だけ検証して未対応を通知する。
    ///
    /// # Errors
    /// 引数パースに失敗した場合。
    pub fn run_tap_recover(args: &[&str]) -> anyhow::Result<()> {
        parse_tap_recover_args(args).map_err(|e| anyhow::anyhow!(e))?;
        log::error!("tap-recover is only supported on macOS");
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::{
        state_code, BypassDetector, TapDisableReason, TapDumpConfig, TapHealth, TapLocationArg,
        TapPlacementArg, TapRecoverConfig, TapState, BYPASS_THRESHOLD, BYPASS_WINDOW,
        DEFAULT_LOG_CAPACITY, STATE_BYPASSED, STATE_DISABLED, STATE_FAILED, STATE_HEALTHY,
        STATE_REENABLING, STATUS_POLL, WRITER_FLUSH_INTERVAL,
    };
    use crate::focus::FocusState;
    use crate::keys;
    use crate::output;
    use crate::permissions::{self, PermissionStatus};
    use crate::report::{self, EventLogger};
    use crate::runtime;
    use crate::synthetic::{SyntheticEventOrigin, SyntheticPressedKeys};
    use objc2_core_foundation::{kCFRunLoopDefaultMode, CFMachPort, CFRetained, CFRunLoop};
    use objc2_core_graphics::{
        CGEvent, CGEventField, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
        CGEventTapProxy, CGEventType,
    };
    use std::cell::{Cell, RefCell};
    use std::ffi::c_void;
    use std::path::PathBuf;
    use std::ptr::NonNull;
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    /// driver スレッドが lock-free に読む健全性ミラー。callback（run loop スレッド）が書き、
    /// driver が定期ポーリングで読む。
    #[derive(Debug, Default)]
    struct SharedTapHealth {
        generation: AtomicU64,
        timeout_count: AtomicU64,
        user_input_count: AtomicU64,
        state_code: AtomicU8,
        /// 最後の disabled 理由（0=なし, 1=timeout, 2=user-input）。`Disabled` 状態を
        /// [`TapHealth`] に復元する際に理由を取り戻すためのミラー。
        last_reason: AtomicU8,
        bypassed: AtomicBool,
    }

    impl SharedTapHealth {
        /// 共有 atomic から [`TapHealth`] スナップショットを組み立てる。別スレッド
        /// （sleep-wake の定期スナップショット等）から lock-free に呼べる。`Instant`
        /// タイムスタンプは atomic に持たないため `None`（読み手が自前で時刻を打つ）。
        fn snapshot(&self) -> TapHealth {
            let state = match self.state_code.load(Ordering::Relaxed) {
                STATE_HEALTHY => TapState::Healthy,
                STATE_DISABLED => {
                    TapState::Disabled(reason_from_code(self.last_reason.load(Ordering::Relaxed)))
                }
                STATE_REENABLING => TapState::Reenabling,
                STATE_BYPASSED => TapState::Bypassed,
                STATE_FAILED => TapState::Failed,
                _ => TapState::Starting,
            };
            TapHealth {
                generation: self.generation.load(Ordering::Relaxed),
                state,
                disabled_by_timeout_count: self.timeout_count.load(Ordering::Relaxed),
                disabled_by_user_input_count: self.user_input_count.load(Ordering::Relaxed),
                last_disabled_at: None,
                last_enabled_at: None,
            }
        }
    }

    const fn reason_from_code(code: u8) -> TapDisableReason {
        match code {
            2 => TapDisableReason::UserInput,
            _ => TapDisableReason::Timeout,
        }
    }

    /// 稼働中タップの健全性を lock-free に読むためのクローン可能なハンドル。
    /// [`TapSession::health_handle`] から得て、別スレッドへ渡して定期スナップショットに使う。
    #[derive(Debug, Clone)]
    pub struct TapHealthHandle {
        shared: Arc<SharedTapHealth>,
    }

    impl TapHealthHandle {
        /// 現在の [`TapHealth`] スナップショット（lock-free）。
        #[must_use]
        pub fn snapshot(&self) -> TapHealth {
            self.shared.snapshot()
        }
    }

    /// 専用スレッドで稼働するタップのセッションハンドル。ブロックせずにタップを起こし、
    /// 健全性を読み、明示的に停止できる。ctx はプロセス寿命ぶんリークするため、logger /
    /// focus はセッション側で保持しなくてもタップが生かし続ける。
    #[derive(Debug)]
    pub struct TapSession {
        handle: runtime::RunLoopThreadHandle,
        shared: Arc<SharedTapHealth>,
    }

    impl TapSession {
        /// 現在の [`TapHealth`] スナップショット。呼び出し元（sleep_wake.rs）は
        /// クローン可能な [`Self::health_handle`] 経由で別スレッドから読むため、
        /// セッション所有者自身がこれを直接呼ぶ場面が今のところ無い。
        #[allow(dead_code)]
        #[must_use]
        pub fn health(&self) -> TapHealth {
            self.shared.snapshot()
        }

        /// 別スレッドから健全性を読むためのクローン可能ハンドルを得る。
        #[must_use]
        pub fn health_handle(&self) -> TapHealthHandle {
            TapHealthHandle {
                shared: Arc::clone(&self.shared),
            }
        }

        /// run loop ワーカを停止し、join する。
        pub fn shutdown(mut self) {
            self.handle.shutdown();
        }
    }

    /// ブロックせずにタップを起こし、[`TapSession`] を返す。`logger` / `focus` は
    /// callback ctx に move されて共有される（tap-dump/recover と同じ EventLogger /
    /// FocusState を渡せば、診断ログと focus 帰属を統合できる）。sleep-wake 常駐ログが使う。
    #[must_use]
    pub fn spawn_tap_session(
        location: TapLocationArg,
        placement: TapPlacementArg,
        listen_only: bool,
        logger: Arc<EventLogger>,
        focus: Arc<FocusState>,
    ) -> TapSession {
        let shared = Arc::new(SharedTapHealth::default());
        let runtime_cfg = TapRuntimeConfig {
            location: location_of(location),
            placement: placement_of(placement),
            options: options_for(listen_only),
            recover_delay: Duration::ZERO,
            recover_once: false,
        };
        let shared_for_setup = Arc::clone(&shared);
        let handle = runtime::spawn_run_loop_thread("awase-tap-session", move || {
            let ctx = Box::new(TapCallbackContext::new(
                logger,
                shared_for_setup,
                focus,
                Duration::ZERO,
                false,
            ));
            install_tap(ctx, &runtime_cfg);
        });
        TapSession { handle, shared }
    }

    /// callback 用の共通設定（dump/recover 双方が組み立てる）。
    struct TapRuntimeConfig {
        location: CGEventTapLocation,
        placement: CGEventTapPlacement,
        options: CGEventTapOptions,
        recover_delay: Duration,
        recover_once: bool,
    }

    /// C callback に `user_info` として渡すコンテキスト。run loop スレッドからのみ触る
    /// フィールド（`RefCell`/`Cell`）と、driver と共有する `Arc<SharedTapHealth>` を持つ。
    ///
    /// tap の寿命ぶん生かす必要があり、診断バイナリは有限時間で終了するので、生成後は
    /// リークして（`Box::into_raw`）プロセス終了時に OS 回収に任せる。
    struct TapCallbackContext {
        logger: Arc<EventLogger>,
        shared: Arc<SharedTapHealth>,
        focus: Arc<FocusState>,
        origin: SyntheticEventOrigin,
        pressed: RefCell<SyntheticPressedKeys>,
        health: RefCell<TapHealth>,
        bypass: RefCell<BypassDetector>,
        tap_port: RefCell<Option<CFRetained<CFMachPort>>>,
        start: Instant,
        recover_delay: Duration,
        recover_once: bool,
        recovered_once: Cell<bool>,
    }

    impl TapCallbackContext {
        fn new(
            logger: Arc<EventLogger>,
            shared: Arc<SharedTapHealth>,
            focus: Arc<FocusState>,
            recover_delay: Duration,
            recover_once: bool,
        ) -> Self {
            Self {
                logger,
                shared,
                focus,
                origin: SyntheticEventOrigin::new(),
                pressed: RefCell::new(SyntheticPressedKeys::new()),
                health: RefCell::new(TapHealth::new()),
                bypass: RefCell::new(BypassDetector::new(BYPASS_WINDOW, BYPASS_THRESHOLD)),
                tap_port: RefCell::new(None),
                start: Instant::now(),
                recover_delay,
                recover_once,
                recovered_once: Cell::new(false),
            }
        }

        /// `health`（純粋モデル）の現在値を共有 atomic にミラーする。
        fn mirror(&self) {
            let health = self.health.borrow();
            self.shared
                .generation
                .store(health.generation, Ordering::Relaxed);
            self.shared
                .timeout_count
                .store(health.disabled_by_timeout_count, Ordering::Relaxed);
            self.shared
                .user_input_count
                .store(health.disabled_by_user_input_count, Ordering::Relaxed);
            self.shared
                .state_code
                .store(state_code(health.state), Ordering::Relaxed);
            self.shared.bypassed.store(
                matches!(health.state, TapState::Bypassed),
                Ordering::Relaxed,
            );
            let reason_code = match health.state {
                TapState::Disabled(TapDisableReason::UserInput) => 2u8,
                TapState::Disabled(TapDisableReason::Timeout) => 1u8,
                _ => 0u8,
            };
            self.shared
                .last_reason
                .store(reason_code, Ordering::Relaxed);
        }

        fn on_enabled_confirmed(&self) {
            self.health.borrow_mut().mark_enabled(Instant::now());
            self.mirror();
        }

        /// disabled を観測したときの復旧手順。best-effort key-up → 再有効化。閾値超過なら bypass。
        fn on_disabled(&self, reason: TapDisableReason) {
            self.health
                .borrow_mut()
                .mark_disabled(reason, Instant::now());
            self.mirror();

            let should_bypass = self.bypass.borrow_mut().record(Instant::now());
            let once_exhausted = self.recover_once && self.recovered_once.get();
            if should_bypass || once_exhausted {
                self.health.borrow_mut().mark_bypassed();
                self.mirror();
                log::error!(
                    "event tap disabled ({reason:?}); giving up and entering Bypassed \
                     (bypass_threshold_hit={should_bypass}, once_exhausted={once_exhausted}). \
                     Check for a conflicting event tap or system pressure."
                );
                return;
            }

            self.health.borrow_mut().mark_reenabling();
            self.mirror();

            // 1) stuck key 防止: 未解放 synthetic キーへ tagged key-up を送る。
            {
                let mut pressed = self.pressed.borrow_mut();
                output::release_all_best_effort(&mut pressed, &self.origin);
            }

            // 2) （recover 実験用）再有効化前の待機。dump では 0。
            if !self.recover_delay.is_zero() {
                std::thread::sleep(self.recover_delay);
            }

            // 3) 再有効化して確認。
            let port = self.tap_port.borrow();
            let Some(port) = port.as_ref() else {
                self.health.borrow_mut().mark_failed();
                self.mirror();
                log::error!("cannot re-enable tap: mach port not set");
                return;
            };
            CGEvent::tap_enable(port, true);
            if CGEvent::tap_is_enabled(port) {
                self.recovered_once.set(true);
                self.on_enabled_confirmed();
                let generation = self.shared.generation.load(Ordering::Relaxed);
                log::warn!("event tap re-enabled after {reason:?} (generation now {generation})");
            } else {
                self.health.borrow_mut().mark_failed();
                self.mirror();
                log::error!("event tap re-enable failed after {reason:?}");
            }
        }

        fn record_event(&self, event_type: CGEventType, event: &CGEvent) {
            // FFI 読み出しだけをここで行い、レコード化・解釈は keys.rs の純粋ロジックへ委譲する。
            let raw = keys::RawKeyEvent {
                event_type: event_type.0,
                keycode: u16::try_from(CGEvent::integer_value_field(
                    Some(event),
                    CGEventField::KeyboardEventKeycode,
                ))
                .unwrap_or(0),
                flags: CGEvent::flags(Some(event)).0,
                autorepeat: CGEvent::integer_value_field(
                    Some(event),
                    CGEventField::KeyboardEventAutorepeat,
                ) != 0,
                cg_event_timestamp: CGEvent::timestamp(Some(event)),
                source_user_data: CGEvent::integer_value_field(
                    Some(event),
                    CGEventField::EventSourceUserData,
                ),
            };
            let ctx = keys::CaptureContext {
                monotonic_nanos: u64::try_from(self.start.elapsed().as_nanos()).unwrap_or(u64::MAX),
                wall_clock_nanos: report::wall_clock_now_nanos(),
                thread_id: current_thread_id(),
                tap_generation: self.shared.generation.load(Ordering::Relaxed),
                focus_epoch: self.focus.focus_epoch(),
                bundle_index: self.focus.current_bundle_index(),
                is_synthetic: self.origin.is_self_event(raw.source_user_data),
            };
            // 人間可読な「生キーイベントダンプ」（既定 off の trace。有効時のみ整形される）。
            log::trace!(
                "{} keycode={} mods={:?} synthetic={}",
                raw.kind().as_str(),
                raw.keycode,
                raw.modifiers(),
                ctx.is_synthetic,
            );
            self.logger.try_push(keys::build_event_record(raw, ctx));
        }
    }

    fn current_thread_id() -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        std::thread::current().id().hash(&mut hasher);
        hasher.finish()
    }

    const fn event_mask() -> u64 {
        (1u64 << CGEventType::KeyDown.0)
            | (1u64 << CGEventType::KeyUp.0)
            | (1u64 << CGEventType::FlagsChanged.0)
    }

    /// C からの callback。panic を境界外へ漏らさないため必ず catch_unwind で包む。
    unsafe extern "C-unwind" fn tap_callback(
        _proxy: CGEventTapProxy,
        event_type: CGEventType,
        event: NonNull<CGEvent>,
        user_info: *mut c_void,
    ) -> *mut CGEvent {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // SAFETY: user_info は install_tap が Box::into_raw で作った ctx への
            //         有効なポインタで、tap の寿命ぶん生存する。触るのは run loop
            //         スレッドのみ。
            let ctx = unsafe { &*(user_info.cast::<TapCallbackContext>()) };
            if event_type == CGEventType::TapDisabledByTimeout {
                ctx.on_disabled(TapDisableReason::Timeout);
            } else if event_type == CGEventType::TapDisabledByUserInput {
                ctx.on_disabled(TapDisableReason::UserInput);
            } else {
                // SAFETY: event は OS が渡す有効な CGEvent。
                let event_ref = unsafe { event.as_ref() };
                ctx.record_event(event_type, event_ref);
            }
        }));
        if outcome.is_err() {
            log::error!(
                "panic inside tap callback was caught at the C boundary; passing event through"
            );
        }
        // 観測専用（診断）なのでイベントは常に素通しする。
        event.as_ptr()
    }

    const fn location_of(arg: TapLocationArg) -> CGEventTapLocation {
        match arg {
            TapLocationArg::Session => CGEventTapLocation::SessionEventTap,
            TapLocationArg::Hid => CGEventTapLocation::HIDEventTap,
            TapLocationArg::Annotated => CGEventTapLocation::AnnotatedSessionEventTap,
        }
    }

    const fn placement_of(arg: TapPlacementArg) -> CGEventTapPlacement {
        match arg {
            TapPlacementArg::Head => CGEventTapPlacement::HeadInsertEventTap,
            TapPlacementArg::Tail => CGEventTapPlacement::TailAppendEventTap,
        }
    }

    const fn options_for(listen_only: bool) -> CGEventTapOptions {
        if listen_only {
            CGEventTapOptions::ListenOnly
        } else {
            CGEventTapOptions::Default
        }
    }

    /// run loop スレッド上でタップを生成し、このスレッドの run loop に載せて有効化する。
    /// 成否は `shared` に反映する。ctx は成功時リーク、失敗時に回収する。
    fn install_tap(ctx: Box<TapCallbackContext>, cfg: &TapRuntimeConfig) {
        let ctx_ptr = Box::into_raw(ctx);
        // SAFETY: callback は正しく実装しており、ctx_ptr はここから tap の寿命ぶん有効。
        let port = unsafe {
            CGEvent::tap_create(
                cfg.location,
                cfg.placement,
                cfg.options,
                event_mask(),
                Some(tap_callback),
                ctx_ptr.cast::<c_void>(),
            )
        };
        let Some(port) = port else {
            // SAFETY: 直前に into_raw したポインタを一度だけ回収する。
            let ctx = unsafe { Box::from_raw(ctx_ptr) };
            ctx.health.borrow_mut().mark_failed();
            ctx.mirror();
            log::error!(
                "CGEventTapCreate returned null — likely missing Input Monitoring / \
                 Accessibility permission, or an unsupported tap location"
            );
            return;
        };

        // SAFETY: ctx_ptr は上で有効。ここはまだ run loop 開始前で単一スレッド。
        let ctx_ref = unsafe { &*ctx_ptr };
        ctx_ref.tap_port.replace(Some(port.clone()));

        let Some(source) = CFMachPort::new_run_loop_source(None, Some(&port), 0) else {
            ctx_ref.health.borrow_mut().mark_failed();
            ctx_ref.mirror();
            log::error!("CFMachPortCreateRunLoopSource returned null");
            return;
        };
        if let Some(run_loop) = CFRunLoop::current() {
            // SAFETY: kCFRunLoopDefaultMode は CoreFoundation の有効なグローバル定数。
            let mode = unsafe { kCFRunLoopDefaultMode };
            run_loop.add_source(Some(&source), mode);
        }
        CGEvent::tap_enable(&port, true);
        if CGEvent::tap_is_enabled(&port) {
            ctx_ref.on_enabled_confirmed();
            log::info!("event tap installed and enabled");
        } else {
            ctx_ref.health.borrow_mut().mark_failed();
            ctx_ref.mirror();
            log::error!("event tap created but failed to enable");
        }
        // run loop が source を保持する。CFRetained を drop しても登録は残るが、
        // プロセス寿命ぶん明示的に生かすため forget する。
        std::mem::forget(source);
    }

    fn preflight_listen_permission() {
        let perms = permissions::check_all();
        if perms.listen_events != PermissionStatus::Granted {
            log::warn!(
                "listen_events permission is {:?}; CGEventTapCreate will likely fail. \
                 Run `permissions request listen` first.",
                perms.listen_events
            );
        }
    }

    fn log_summary(shared: &SharedTapHealth, logger: &EventLogger) {
        log::info!(
            "tap summary: generation={} state_code={} timeout_disables={} user_input_disables={} \
             bypassed={} dropped_logs={} pending_logs={}",
            shared.generation.load(Ordering::Relaxed),
            shared.state_code.load(Ordering::Relaxed),
            shared.timeout_count.load(Ordering::Relaxed),
            shared.user_input_count.load(Ordering::Relaxed),
            shared.bypassed.load(Ordering::Relaxed),
            logger.dropped_log_count(),
            logger.pending_len(),
        );
    }

    fn drain_to_stdout(logger: &EventLogger) {
        let mut stdout = std::io::stdout().lock();
        let _ = logger.drain_and_write(&mut stdout);
    }

    /// タップを張って run loop スレッドで回し、`deadline` まで（None なら無期限）観測する。
    /// jsonl 未指定時は stdout へ JSONL を drain する。
    fn run_tap(
        runtime_cfg: TapRuntimeConfig,
        jsonl_path: Option<PathBuf>,
        deadline: Option<Instant>,
    ) -> anyhow::Result<()> {
        preflight_listen_permission();

        let logger = Arc::new(EventLogger::new(DEFAULT_LOG_CAPACITY));
        let writer = match jsonl_path {
            Some(path) => Some(logger.spawn_writer(path, WRITER_FLUSH_INTERVAL)?),
            None => None,
        };
        let shared = Arc::new(SharedTapHealth::default());
        // FocusState は同じ EventLogger を共有して bundle を intern する。tap-dump 単体では
        // NSWorkspace observer を張らないので epoch/bundle は既定のままだが、focus observer を
        // 張る統合パスがこの同一 FocusState を共有すれば、打鍵ごとに正しい focus 帰属が付く。
        let focus = FocusState::new(Arc::clone(&logger));

        let logger_for_setup = Arc::clone(&logger);
        let shared_for_setup = Arc::clone(&shared);
        let focus_for_setup = Arc::clone(&focus);
        let delay = runtime_cfg.recover_delay;
        let once = runtime_cfg.recover_once;
        let handle = runtime::spawn_run_loop_thread("awase-tap", move || {
            let ctx = Box::new(TapCallbackContext::new(
                logger_for_setup,
                shared_for_setup,
                focus_for_setup,
                delay,
                once,
            ));
            install_tap(ctx, &runtime_cfg);
        });

        loop {
            std::thread::sleep(STATUS_POLL);
            if writer.is_none() {
                drain_to_stdout(&logger);
            }
            if shared.state_code.load(Ordering::Relaxed) == STATE_FAILED {
                break;
            }
            if deadline.is_some_and(|dl| Instant::now() >= dl) {
                break;
            }
        }

        let mut handle = handle;
        handle.shutdown();
        if let Some(writer) = writer {
            writer.stop();
        } else {
            drain_to_stdout(&logger);
        }
        log_summary(&shared, &logger);
        Ok(())
    }

    /// # Errors
    /// 引数パース失敗、または jsonl 出力ファイルを開けなかった場合。
    pub fn run_tap_dump(args: &[&str]) -> anyhow::Result<()> {
        let cfg: TapDumpConfig =
            super::parse_tap_dump_args(args).map_err(|e| anyhow::anyhow!(e))?;
        let runtime_cfg = TapRuntimeConfig {
            location: location_of(cfg.location),
            placement: placement_of(cfg.placement),
            options: options_for(cfg.listen_only),
            recover_delay: Duration::ZERO,
            recover_once: false,
        };
        let deadline = cfg.duration.map(|d| Instant::now() + d);
        run_tap(runtime_cfg, cfg.jsonl_path, deadline)
    }

    /// # Errors
    /// 引数パース失敗、または内部の run loop 設定に失敗した場合。
    pub fn run_tap_recover(args: &[&str]) -> anyhow::Result<()> {
        let cfg: TapRecoverConfig =
            super::parse_tap_recover_args(args).map_err(|e| anyhow::anyhow!(e))?;
        let runtime_cfg = TapRuntimeConfig {
            location: CGEventTapLocation::SessionEventTap,
            placement: CGEventTapPlacement::HeadInsertEventTap,
            options: options_for(cfg.listen_only),
            recover_delay: cfg.delay,
            recover_once: cfg.once,
        };
        let deadline = cfg.auto_exit_after.map(|d| Instant::now() + d);
        run_tap(runtime_cfg, None, deadline)
    }
}

pub use platform::{run_tap_dump, run_tap_recover};
// TapSession: sleep_wake.rs は spawn_tap_session() の戻り値を型注釈なしで受け取り
// health_handle() だけ使うため、型名としての import 自体は未使用。
#[allow(unused_imports)]
#[cfg(target_os = "macos")]
pub use platform::{spawn_tap_session, TapHealthHandle, TapSession};

#[cfg(test)]
mod tests {
    use super::{
        parse_tap_dump_args, parse_tap_recover_args, state_code, BypassDetector, TapDisableReason,
        TapDumpConfig, TapHealth, TapLocationArg, TapPlacementArg, TapRecoverConfig, TapState,
        STATE_BYPASSED, STATE_DISABLED, STATE_FAILED, STATE_HEALTHY, STATE_REENABLING,
        STATE_STARTING,
    };
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    #[test]
    fn tap_dump_defaults() {
        assert_eq!(parse_tap_dump_args(&[]).unwrap(), TapDumpConfig::default());
    }

    #[test]
    fn tap_dump_full_flags() {
        let args = [
            "--location",
            "hid",
            "--placement",
            "tail",
            "--listen-only",
            "--jsonl",
            "/tmp/out.jsonl",
            "--duration",
            "30",
        ];
        let cfg = parse_tap_dump_args(&args).unwrap();
        assert_eq!(cfg.location, TapLocationArg::Hid);
        assert_eq!(cfg.placement, TapPlacementArg::Tail);
        assert!(cfg.listen_only);
        assert_eq!(cfg.jsonl_path, Some(PathBuf::from("/tmp/out.jsonl")));
        assert_eq!(cfg.duration, Some(Duration::from_secs(30)));
    }

    #[test]
    fn tap_dump_unknown_arg_errors() {
        assert!(parse_tap_dump_args(&["--nope"]).is_err());
    }

    #[test]
    fn tap_dump_missing_value_errors() {
        assert!(parse_tap_dump_args(&["--location"]).is_err());
    }

    #[test]
    fn tap_dump_bad_enum_errors() {
        assert!(parse_tap_dump_args(&["--placement", "sideways"]).is_err());
    }

    #[test]
    fn tap_recover_defaults_have_safety_exit() {
        let cfg = parse_tap_recover_args(&[]).unwrap();
        assert_eq!(cfg, TapRecoverConfig::default());
        assert_eq!(cfg.delay, Duration::from_millis(1000));
        assert_eq!(cfg.auto_exit_after, Some(Duration::from_secs(10)));
        assert!(!cfg.once);
    }

    #[test]
    fn tap_recover_flags() {
        let args = [
            "--delay-ms",
            "250",
            "--once",
            "--listen-only",
            "--auto-exit-after-secs",
            "5",
        ];
        let cfg = parse_tap_recover_args(&args).unwrap();
        assert_eq!(cfg.delay, Duration::from_millis(250));
        assert!(cfg.once);
        assert!(cfg.listen_only);
        assert_eq!(cfg.auto_exit_after, Some(Duration::from_secs(5)));
    }

    #[test]
    fn health_generation_increments_on_enable() {
        let mut health = TapHealth::new();
        assert_eq!(health.generation, 0);
        assert_eq!(health.state, TapState::Starting);
        let now = Instant::now();
        health.mark_enabled(now);
        assert_eq!(health.generation, 1);
        assert_eq!(health.state, TapState::Healthy);
        health.mark_disabled(TapDisableReason::Timeout, now);
        assert_eq!(health.state, TapState::Disabled(TapDisableReason::Timeout));
        health.mark_reenabling();
        health.mark_enabled(now);
        assert_eq!(health.generation, 2);
    }

    #[test]
    fn health_counts_by_reason() {
        let mut health = TapHealth::new();
        let now = Instant::now();
        health.mark_disabled(TapDisableReason::Timeout, now);
        health.mark_disabled(TapDisableReason::Timeout, now);
        health.mark_disabled(TapDisableReason::UserInput, now);
        assert_eq!(health.disabled_by_timeout_count, 2);
        assert_eq!(health.disabled_by_user_input_count, 1);
    }

    #[test]
    fn bypass_triggers_at_threshold_within_window() {
        let mut detector = BypassDetector::new(Duration::from_secs(5), 3);
        let base = Instant::now();
        assert!(!detector.record(base));
        assert!(!detector.record(base + Duration::from_millis(100)));
        assert!(detector.record(base + Duration::from_millis(200)));
    }

    #[test]
    fn bypass_prunes_outside_window() {
        let mut detector = BypassDetector::new(Duration::from_secs(1), 3);
        let base = Instant::now();
        assert!(!detector.record(base));
        assert!(!detector.record(base + Duration::from_secs(2)));
        // 3s 後は 1s 窓に 1 件だけ残るので閾値未満。
        assert!(!detector.record(base + Duration::from_secs(3)));
    }

    #[test]
    fn state_codes_are_distinct() {
        assert_eq!(state_code(TapState::Starting), STATE_STARTING);
        assert_eq!(state_code(TapState::Healthy), STATE_HEALTHY);
        assert_eq!(
            state_code(TapState::Disabled(TapDisableReason::Timeout)),
            STATE_DISABLED
        );
        assert_eq!(state_code(TapState::Reenabling), STATE_REENABLING);
        assert_eq!(state_code(TapState::Bypassed), STATE_BYPASSED);
        assert_eq!(state_code(TapState::Failed), STATE_FAILED);
    }
}
