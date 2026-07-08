//! 入力ソース（キーボードレイアウト / IME）の観測と日本語／英数分類。
//!
//! 設計は project_macos_probe_interfaces.md の「3. 入力ソース観測とIME分類」に従う。要点:
//!
//! - 観測には [`InputSourceObservation::focus_epoch`] を必ず含める。フォーカス変更直後に
//!   前の入力ソース変更通知が遅延到着する順序問題を診断できるようにするため（初期設計で
//!   欠落しレビューで指摘された）。epoch は [`crate::focus::FocusState`] の lock-free
//!   `focus_epoch()` から採る。
//! - 候補一覧は `is_keyboard_category()` かつ `IsEnabled == true` かつ
//!   `IsSelectCapable == true` でフィルタする（親 input method と配下 input mode が
//!   別要素になるケースを弾く）。
//! - 変更通知（`kTISNotifySelectedKeyboardInputSourceChanged` /
//!   `kTISNotifyEnabledKeyboardInputSourcesChanged`）は **再問い合わせのトリガーとして
//!   のみ**使い、通知内容を直接状態にしない。これらは NSWorkspace の center ではなく
//!   Distributed Notification Center に流れる（[`mac`] 参照）。
//! - [`InputSourceRegistry`] は単一マップ（2 つの `HashSet` ではない）。同一 ID を別モードで
//!   登録すると単純に上書きされ、両モードへの二重登録という設定ミスが構造的に起きない。
//!   [`InputSourceRegistry::classify`] は登録に無い ID を **fail-open** で
//!   [`JapaneseInputMode::Unknown`] にし、ID 文字列からの推測は一切しない。
//!
//! 実 TIS 呼び出しは [`crate::tis_sys`] に隔離されており、そこが非 macOS で空を返す
//! フォールバックを持つため、観測・分類・レジストリのロジックは任意ホストで
//! コンパイル・単体テストできる。実 IME での確認は実機検証ゲート（#19）に委ねる。

// Distributed / workspace notification observer 登録（objc2 selector 呼び出し）に
// unsafe が必須。実 unsafe は `mod mac` に閉じ込めてある。
#![allow(unsafe_code)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use crate::tis_sys::{self, InputSourceHandle};

/// 観測列全体で単調増加する連番のソース。`observe_current` が観測ごとに 1 つ払い出す。
static OBSERVATION_SEQ: AtomicU64 = AtomicU64::new(0);

/// 観測を発生させたきっかけ。通知は再問い合わせのトリガーとしてのみ使うため、
/// 「何が再取得を促したか」を観測に添えて記録する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservationTrigger {
    /// フォーカス変更（アプリ切替）を機に現在ソースを再取得した。
    PollOnFocus,
    /// TIS の変更通知を受けて再取得した。
    ChangeNotification,
    /// 明示的な問い合わせ（CLI 一覧表示・起動時スナップショット）。
    ManualQuery,
}

impl ObservationTrigger {
    /// ログ表示用の安定した短い文字列。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PollOnFocus => "poll-on-focus",
            Self::ChangeNotification => "change-notification",
            Self::ManualQuery => "manual-query",
        }
    }
}

// RegisteredInputMode / JapaneseInputMode / InputSourceRegistry は Phase M3
// （ユーザーが入力ソースを日本語/英数として登録する設定機能）向けの型で、
// それを読み書きする CLI サブコマンドはまだ無い。config.toml 連携時に配線される。

/// レジストリに登録される入力モード分類（登録値そのもの）。
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegisteredInputMode {
    /// 日本語入力（かな漢字変換）。
    Japanese,
    /// 半角英数（ASCII 直接入力）。
    Alphanumeric,
}

#[allow(dead_code)]
impl RegisteredInputMode {
    /// ログ表示用の安定した短い文字列。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Japanese => "japanese",
            Self::Alphanumeric => "alphanumeric",
        }
    }
}

/// [`InputSourceRegistry::classify`] の結果。未登録は [`Unknown`](Self::Unknown)。
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JapaneseInputMode {
    /// 日本語入力として登録済み。
    Japanese,
    /// 半角英数として登録済み。
    Alphanumeric,
    /// レジストリに無い（fail-open。ID 文字列から推測はしない）。
    Unknown,
}

#[allow(dead_code)]
impl JapaneseInputMode {
    /// ログ表示用の安定した短い文字列。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Japanese => "japanese",
            Self::Alphanumeric => "alphanumeric",
            Self::Unknown => "unknown",
        }
    }
}

/// 列挙された 1 つの入力ソースの静的属性（現在値でも観測メタでもない候補）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputSourceCandidate {
    pub source_id: String,
    pub localized_name: Option<String>,
    pub languages: Vec<String>,
    pub input_mode_id: Option<String>,
    pub bundle_id: Option<String>,
    pub is_ascii_capable: Option<bool>,
}

/// ある時点の「現在選択中の入力ソース」観測。候補の静的属性に観測メタ
/// （`focus_epoch` / `observation_seq` / `trigger` / `observed_at`）を添えたもの。
#[derive(Debug, Clone)]
pub struct InputSourceObservation {
    pub source_id: String,
    pub localized_name: Option<String>,
    pub languages: Vec<String>,
    pub is_ascii_capable: Option<bool>,
    pub category: Option<String>,
    pub source_type: Option<String>,
    pub input_mode_id: Option<String>,
    pub bundle_id: Option<String>,
    pub is_enabled: Option<bool>,
    pub is_select_capable: Option<bool>,
    /// 観測を取得した時刻（プロセス基準の単調時計）。
    #[allow(dead_code)] // まだ読む CLI サブコマンドが無い（診断出力の将来拡張用）
    pub observed_at: Instant,
    /// 観測時点の focus epoch（遅延到着通知の並べ替え診断用）。
    pub focus_epoch: u64,
    /// 観測列内で単調増加する連番。
    pub observation_seq: u64,
    /// この観測を促したトリガー。
    pub trigger: ObservationTrigger,
}

/// 入力ソース ID → 入力モードの単一マップ。
///
/// 2 つの `HashSet`（japanese_ids / alphanumeric_ids）に分けると同一 ID を両方に
/// 登録する設定ミスが隠れるため、単一マップにする。同一 ID を別モードで登録すると
/// 単純に上書きされ、二重登録は構造的に起こらない。
#[allow(dead_code)]
#[derive(Debug, Default, Clone)]
pub struct InputSourceRegistry {
    entries: HashMap<String, RegisteredInputMode>,
}

#[allow(dead_code)]
impl InputSourceRegistry {
    /// 空のレジストリを作る。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// ID をモードに登録する。同一 ID の再登録は上書き（単一マップなので二重登録に
    /// ならない）。
    pub fn register(&mut self, source_id: impl Into<String>, mode: RegisteredInputMode) {
        self.entries.insert(source_id.into(), mode);
    }

    /// 観測の `source_id` からモードを引く。登録に無ければ **fail-open** で
    /// [`JapaneseInputMode::Unknown`]（ID 文字列からの推測はしない）。
    #[must_use]
    pub fn classify(&self, observation: &InputSourceObservation) -> JapaneseInputMode {
        match self.entries.get(&observation.source_id) {
            Some(RegisteredInputMode::Japanese) => JapaneseInputMode::Japanese,
            Some(RegisteredInputMode::Alphanumeric) => JapaneseInputMode::Alphanumeric,
            None => JapaneseInputMode::Unknown,
        }
    }

    /// 登録件数。
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 登録が空か。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// ハンドルから候補（静的属性）を作る。ID が取得できないものは `None`。
fn candidate_from_handle(handle: &InputSourceHandle) -> Option<InputSourceCandidate> {
    let source_id = handle.source_id()?;
    Some(InputSourceCandidate {
        source_id,
        localized_name: handle.localized_name(),
        languages: handle.languages(),
        input_mode_id: handle.input_mode_id(),
        bundle_id: handle.bundle_id(),
        is_ascii_capable: handle.is_ascii_capable(),
    })
}

/// 有効なキーボード入力ソースを列挙する。
///
/// `is_keyboard_category()` かつ `IsEnabled == true` かつ `IsSelectCapable == true` で
/// フィルタする。非 macOS では空 `Vec`。
#[must_use]
pub fn list_enabled_input_sources() -> Vec<InputSourceCandidate> {
    tis_sys::create_input_source_list(false)
        .iter()
        .filter(|handle| {
            handle.is_keyboard_category()
                && handle.is_enabled() == Some(true)
                && handle.is_select_capable() == Some(true)
        })
        .filter_map(candidate_from_handle)
        .collect()
}

/// 現在選択中のキーボード入力ソースを観測する。
///
/// `focus_epoch` は呼び出し側（通常 [`crate::focus::FocusState::focus_epoch`]）が渡す。
/// `observation_seq` は module 内 [`OBSERVATION_SEQ`] から払い出す。非 macOS / 取得
/// 不能時は `None`。
#[must_use]
pub fn observe_current(
    focus_epoch: u64,
    trigger: ObservationTrigger,
) -> Option<InputSourceObservation> {
    let handle = tis_sys::copy_current_input_source()?;
    let source_id = handle.source_id()?;
    let observation_seq = OBSERVATION_SEQ.fetch_add(1, Ordering::SeqCst);
    Some(InputSourceObservation {
        source_id,
        localized_name: handle.localized_name(),
        languages: handle.languages(),
        is_ascii_capable: handle.is_ascii_capable(),
        category: handle.category(),
        source_type: handle.source_type(),
        input_mode_id: handle.input_mode_id(),
        bundle_id: handle.bundle_id(),
        is_enabled: handle.is_enabled(),
        is_select_capable: handle.is_select_capable(),
        observed_at: Instant::now(),
        focus_epoch,
        observation_seq,
        trigger,
    })
}

/// `Option<&str>` を表示用に整形する（`None` は `-`）。
fn opt_str(value: Option<&str>) -> &str {
    value.unwrap_or("-")
}

/// `Option<bool>` を表示用に整形する（`None` は `-`）。
const fn opt_bool(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "true",
        Some(false) => "false",
        None => "-",
    }
}

/// 現在値観測を 1 行で表示する。
fn print_observation(obs: &InputSourceObservation) {
    println!(
        "[input] seq={} trigger={} focus_epoch={} id={} name={:?} ascii={} langs={:?} \
         enabled={} select={} category={} type={} input_mode={} bundle={}",
        obs.observation_seq,
        obs.trigger.as_str(),
        obs.focus_epoch,
        obs.source_id,
        opt_str(obs.localized_name.as_deref()),
        opt_bool(obs.is_ascii_capable),
        obs.languages,
        opt_bool(obs.is_enabled),
        opt_bool(obs.is_select_capable),
        opt_str(obs.category.as_deref()),
        opt_str(obs.source_type.as_deref()),
        opt_str(obs.input_mode_id.as_deref()),
        opt_str(obs.bundle_id.as_deref()),
    );
}

/// 候補 1 件を index 付きで 1 行表示する。
fn print_candidate(index: usize, candidate: &InputSourceCandidate) {
    println!(
        "  [{index}] id={} name={:?} ascii={} langs={:?} input_mode={} bundle={}",
        candidate.source_id,
        opt_str(candidate.localized_name.as_deref()),
        opt_bool(candidate.is_ascii_capable),
        candidate.languages,
        opt_str(candidate.input_mode_id.as_deref()),
        opt_str(candidate.bundle_id.as_deref()),
    );
}

/// 有効な入力ソースの現在値と一覧を 1 度だけ表示する（初回スナップショット）。
fn print_snapshot(focus_epoch: u64) {
    match observe_current(focus_epoch, ObservationTrigger::ManualQuery) {
        Some(obs) => print_observation(&obs),
        None => println!("[input] current input source unavailable (or not running on macOS)"),
    }
    let sources = list_enabled_input_sources();
    println!("enabled keyboard input sources: {}", sources.len());
    for (index, candidate) in sources.iter().enumerate() {
        print_candidate(index, candidate);
    }
}

/// `input-sources` サブコマンドの実体。現在値と有効な入力ソース一覧を表示する。
///
/// TIS の列挙自体は権限不要なので macOS CI ランナーでも実行できる（非 macOS では
/// 空一覧のメッセージのみ）。
///
/// # Errors
/// 現状は失敗しないが、将来の I/O 失敗に備えて `Result` を返す。
// dispatch 側で run_input_watch_cli / run_focus_watch と同じ `Result` シグネチャに
// 揃えるため、現状 Err を返さなくても Result を保つ。
#[allow(clippy::unnecessary_wraps)]
pub fn run_input_sources_cli() -> anyhow::Result<()> {
    print_snapshot(0);
    Ok(())
}

#[cfg(target_os = "macos")]
pub use mac::run_input_watch_cli;

/// [`run_input_watch_cli`] の非 macOS スタブ。
///
/// # Errors
/// 非 macOS では常に `Err`（unsupported platform）。
#[cfg(not(target_os = "macos"))]
pub fn run_input_watch_cli() -> anyhow::Result<()> {
    anyhow::bail!("input-watch is only supported on macOS")
}

#[cfg(target_os = "macos")]
mod mac {
    //! `input-watch` の実体。現在値を表示し、以下を **再問い合わせのトリガーとして
    //! のみ**使う:
    //!
    //! - TIS の 2 種類の変更通知（`kTISNotify*`）: Distributed Notification Center。
    //!   TIS はこれらを `CFNotificationCenterGetDistributedCenter()` 相当の center へ
    //!   投げる。objc2-foundation の `NSDistributedNotificationCenter` は同じ center を
    //!   購読するため、C コールバック方式の生 FFI を足さずに済む（NSWorkspace の center
    //!   や既定の `NSNotificationCenter` はこれらを受け取らない）。
    //! - アプリのアクティベーション（NSWorkspace の center）: focus 変更を機に現在
    //!   ソースを再取得（[`ObservationTrigger::PollOnFocus`]）。
    //!
    //! focus_epoch は focus.rs の [`FocusState`] を共有して採る。focus.rs 側の observer が
    //! epoch を更新し、こちらの observer が再取得する。

    use std::io::Write;
    use std::sync::Arc;

    use objc2::rc::Retained;
    use objc2::runtime::{NSObject, NSObjectProtocol};
    use objc2::{define_class, msg_send, sel, AnyThread, DefinedClass};
    use objc2_app_kit::NSWorkspaceDidActivateApplicationNotification;
    use objc2_foundation::{NSDistributedNotificationCenter, NSNotification, NSString};

    use crate::focus::{install_focus_observer, FocusState};
    use crate::report::EventLogger;
    use crate::runtime::{
        setup_main_application, with_autorelease_pool, workspace_notification_center,
    };
    use crate::tis_sys;

    use super::{
        list_enabled_input_sources, observe_current, print_candidate, print_observation,
        print_snapshot, ObservationTrigger,
    };

    struct Ivars {
        focus: Arc<FocusState>,
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "AwaseProbeInputSourceObserver"]
        #[ivars = Ivars]
        struct InputSourceObserver;

        impl InputSourceObserver {
            #[unsafe(method(selectedInputSourceChanged:))]
            fn selected_changed(&self, _notification: &NSNotification) {
                with_autorelease_pool(|| {
                    self.handle_repoll(ObservationTrigger::ChangeNotification);
                });
            }

            #[unsafe(method(enabledInputSourcesChanged:))]
            fn enabled_changed(&self, _notification: &NSNotification) {
                with_autorelease_pool(Self::handle_enabled_changed);
            }

            #[unsafe(method(applicationActivated:))]
            fn application_activated(&self, _notification: &NSNotification) {
                with_autorelease_pool(|| {
                    self.handle_repoll(ObservationTrigger::PollOnFocus);
                });
            }
        }

        unsafe impl NSObjectProtocol for InputSourceObserver {}
    );

    impl InputSourceObserver {
        fn new(focus: Arc<FocusState>) -> Retained<Self> {
            let this = Self::alloc().set_ivars(Ivars { focus });
            unsafe { msg_send![super(this), init] }
        }

        /// 現在ソースを再取得して 1 行表示する（`trigger` は呼び出し文脈が指定）。
        fn handle_repoll(&self, trigger: ObservationTrigger) {
            let epoch = self.ivars().focus.focus_epoch();
            match observe_current(epoch, trigger) {
                Some(obs) => print_observation(&obs),
                None => println!(
                    "[input] re-poll ({}) but current source unavailable",
                    trigger.as_str()
                ),
            }
            let _ = std::io::stdout().flush();
        }

        /// 有効ソース一覧が変わったので再列挙して表示する。
        fn handle_enabled_changed() {
            let sources = list_enabled_input_sources();
            println!(
                "[input] enabled sources changed: {} enabled keyboard source(s)",
                sources.len()
            );
            for (index, candidate) in sources.iter().enumerate() {
                print_candidate(index, candidate);
            }
            let _ = std::io::stdout().flush();
        }
    }

    /// 登録した observer を保持し、drop 時に両 center から登録解除するガード。
    pub struct InputSourceObserverGuard {
        observer: Retained<InputSourceObserver>,
    }

    impl std::fmt::Debug for InputSourceObserverGuard {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("InputSourceObserverGuard")
                .finish_non_exhaustive()
        }
    }

    impl Drop for InputSourceObserverGuard {
        fn drop(&mut self) {
            let distributed = NSDistributedNotificationCenter::defaultCenter();
            // SAFETY: 自分が登録した observer を外すだけ。observer はこの呼び出し中有効。
            unsafe { distributed.removeObserver_name_object(&self.observer, None, None) };
            let workspace = workspace_notification_center();
            // SAFETY: 同上（NSWorkspace の center から activation 購読を解除）。
            unsafe { workspace.removeObserver(&self.observer) };
        }
    }

    /// TIS 変更通知（distributed）とアプリ activation（workspace）を購読する observer を
    /// 登録し、生存を管理するガードを返す。メインスレッドから呼ぶこと。
    fn install_input_source_observer(
        focus: Arc<FocusState>,
    ) -> anyhow::Result<InputSourceObserverGuard> {
        let selected_name = tis_sys::selected_input_source_changed_notification_name()
            .ok_or_else(|| anyhow::anyhow!("TIS selected-changed notification name unavailable"))?;
        let enabled_name = tis_sys::enabled_input_sources_changed_notification_name()
            .ok_or_else(|| anyhow::anyhow!("TIS enabled-changed notification name unavailable"))?;
        let observer = InputSourceObserver::new(focus);

        let distributed = NSDistributedNotificationCenter::defaultCenter();
        let selected = NSString::from_str(&selected_name);
        let enabled = NSString::from_str(&enabled_name);
        // SAFETY: selector は InputSourceObserver が NSNotification を取る形で定義した
        // ものと一致。name は TIS が distributed center へ投げる通知名。observer は
        // 返り値のガードが登録中ずっと生かし続ける。
        unsafe {
            distributed.addObserver_selector_name_object(
                &observer,
                sel!(selectedInputSourceChanged:),
                Some(&selected),
                None,
            );
            distributed.addObserver_selector_name_object(
                &observer,
                sel!(enabledInputSourcesChanged:),
                Some(&enabled),
                None,
            );
        }

        let workspace = workspace_notification_center();
        // SAFETY: selector は applicationActivated: と一致。name は NSWorkspace の
        // アクティベーション通知の extern static。同じ observer を使う。
        unsafe {
            workspace.addObserver_selector_name_object(
                &observer,
                sel!(applicationActivated:),
                Some(NSWorkspaceDidActivateApplicationNotification),
                None,
            );
        }

        Ok(InputSourceObserverGuard { observer })
    }

    /// `input-watch` サブコマンドの実体。現在値を表示し、変更通知・focus 変更で
    /// ライブ更新する。メインスレッドで `NSApplication` のイベントループに入り、
    /// terminate までブロックする。
    ///
    /// # Errors
    /// メインスレッド以外から呼ばれた場合、または TIS 通知名が取得できない場合。
    pub fn run_input_watch_cli() -> anyhow::Result<()> {
        let app = setup_main_application()?;
        let logger = Arc::new(EventLogger::new(256));
        let focus = FocusState::new(logger);
        let _focus_guard = install_focus_observer(Arc::clone(&focus), false);
        let _input_guard = install_input_source_observer(Arc::clone(&focus))?;

        println!("input-watch: current input source + change/focus notifications (Ctrl-C to stop)");
        print_snapshot(focus.focus_epoch());
        let _ = std::io::stdout().flush();

        app.run();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(source_id: &str) -> InputSourceObservation {
        InputSourceObservation {
            source_id: source_id.to_owned(),
            localized_name: None,
            languages: Vec::new(),
            is_ascii_capable: None,
            category: None,
            source_type: None,
            input_mode_id: None,
            bundle_id: None,
            is_enabled: Some(true),
            is_select_capable: Some(true),
            observed_at: Instant::now(),
            focus_epoch: 0,
            observation_seq: 0,
            trigger: ObservationTrigger::ManualQuery,
        }
    }

    #[test]
    fn trigger_as_str_is_stable() {
        assert_eq!(ObservationTrigger::PollOnFocus.as_str(), "poll-on-focus");
        assert_eq!(
            ObservationTrigger::ChangeNotification.as_str(),
            "change-notification"
        );
        assert_eq!(ObservationTrigger::ManualQuery.as_str(), "manual-query");
    }

    #[test]
    fn registry_register_and_classify() {
        let mut reg = InputSourceRegistry::new();
        assert!(reg.is_empty());
        reg.register("com.apple.keylayout.US", RegisteredInputMode::Alphanumeric);
        reg.register(
            "com.apple.inputmethod.Kotoeri.Japanese",
            RegisteredInputMode::Japanese,
        );
        assert_eq!(reg.len(), 2);
        assert_eq!(
            reg.classify(&observation("com.apple.keylayout.US")),
            JapaneseInputMode::Alphanumeric
        );
        assert_eq!(
            reg.classify(&observation("com.apple.inputmethod.Kotoeri.Japanese")),
            JapaneseInputMode::Japanese
        );
    }

    #[test]
    fn classify_unregistered_is_fail_open_unknown() {
        let reg = InputSourceRegistry::new();
        // 未登録 ID は Unknown（ID 文字列から「Japanese」等と推測しない）。
        assert_eq!(
            reg.classify(&observation("com.apple.inputmethod.Kotoeri.Japanese")),
            JapaneseInputMode::Unknown
        );
    }

    #[test]
    fn register_same_id_overwrites() {
        let mut reg = InputSourceRegistry::new();
        reg.register("id.a", RegisteredInputMode::Japanese);
        reg.register("id.a", RegisteredInputMode::Alphanumeric);
        // 単一マップなので二重登録にならず、最後の登録で上書きされる。
        assert_eq!(reg.len(), 1);
        assert_eq!(
            reg.classify(&observation("id.a")),
            JapaneseInputMode::Alphanumeric
        );
    }

    #[test]
    fn list_enabled_does_not_panic() {
        // 非 macOS ではフォールバックで空。macOS では実ソースが返る（CI で別途検証）。
        #[cfg(not(target_os = "macos"))]
        assert!(list_enabled_input_sources().is_empty());
        let _ = list_enabled_input_sources();
    }
}
