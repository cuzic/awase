mod conv_actuation;
#[cfg(windows)]
pub(crate) mod engine_window;
pub(crate) mod executor;
mod focus_tracker;
mod focus_tracking;
mod ime_actuation;
mod ime_coordinator;
mod ime_refresh;
mod key_pipeline;
// ADR-089 §2.3 Phase B: ImmCross を機構チェーンの要素として実行する非同期経路。
pub(crate) mod message_handlers;
pub(crate) mod open_chain;
pub(crate) mod outbox;
mod transport;

pub(crate) use transport::{PassthroughQueue, PhysicalKeyDisposition};

use crate::focus::FocusKind;
use awase::config::ValidatedConfig;
use awase::engine::{
    Engine, EngineCommand, InputContext, InputModeState, KanaLockHysteresis, ModeKeyConfig,
    SpecialKeyCombos, TextKeyConfig,
};
use awase::ngram::NgramModel;
use awase::types::Timestamp;
use awase::types::{ContextChange, RawKeyEvent, VkCode};

use crate::focus::cache::DetectionSource;
use crate::focus::classifier::InjectionHint;
use crate::platform::WindowsPlatform;
use crate::runtime::executor::ImeApplyPair;
use crate::vk::VkCodeExt as _;
use awase::platform::PlatformRuntime as _;

/// ADR-192 決定3b: `keys.ime_on/off/toggle` の bare 無変換/変換を、coreへ渡す
/// OS非依存のopen軸操作へ事前分類する。通常の特殊キー照合と同じく方向固定を
/// toggleより優先し、onをoffより先に評価する。
pub(crate) fn thumb_forced_open_actions(
    special: &SpecialKeyCombos,
) -> (
    Option<awase::types::ShadowImeAction>,
    Option<awase::types::ShadowImeAction>,
) {
    (
        special.bare_ime_action(crate::vk::VK_NONCONVERT),
        special.bare_ime_action(crate::vk::VK_CONVERT),
    )
}

/// ADR-206 決定4: 非推奨の `*_solo_tap_ime_action`（親指キーに割り当てられているものだけ）を、
/// 該当キーの bare コンボ（`keys.ime_on/off/toggle` に単独で書いたのと同じ）としてメモリ上の照合表へ移す。
/// `SpecialKeyCombos` を組み立てた直後、`thumb_forced_open_actions` を求める**前**に呼ぶこと。
pub(crate) fn migrate_legacy_solo_tap_actions(
    general: &awase::config::GeneralConfig,
    special: &mut SpecialKeyCombos,
) {
    let (muhenkan, henkan) = general.legacy_thumb_solo_tap_actions();
    if let Some(action) = muhenkan {
        special.set_bare_ime_action_if_absent(crate::vk::VK_NONCONVERT, action);
    }
    if let Some(action) = henkan {
        special.set_bare_ime_action_if_absent(crate::vk::VK_CONVERT, action);
    }
}

#[cfg(test)]
mod adr192_tests {
    use super::*;
    use awase::config::ParsedKeyCombo;
    use awase::types::ShadowImeAction;

    fn combo(vk: VkCode, ctrl: bool, shift: bool, alt: bool) -> ParsedKeyCombo {
        ParsedKeyCombo {
            ctrl,
            shift,
            alt,
            vk,
        }
    }

    #[test]
    fn bare_convert_keys_are_classified_with_direction_priority() {
        let special = SpecialKeyCombos {
            engine_on: vec![],
            engine_off: vec![],
            ime_on: vec![combo(crate::vk::VK_CONVERT, false, false, false)],
            ime_off: vec![combo(crate::vk::VK_NONCONVERT, false, false, false)],
            ime_toggle: vec![
                combo(crate::vk::VK_CONVERT, false, false, false),
                combo(crate::vk::VK_NONCONVERT, false, false, false),
            ],
        };
        assert_eq!(
            thumb_forced_open_actions(&special),
            (
                Some(ShadowImeAction::TurnOff),
                Some(ShadowImeAction::TurnOn)
            )
        );
    }

    #[test]
    fn modified_or_non_convert_combos_are_not_forced_thumb_actions() {
        let special = SpecialKeyCombos {
            engine_on: vec![],
            engine_off: vec![],
            ime_on: vec![combo(crate::vk::VK_CONVERT, true, false, false)],
            ime_off: vec![combo(crate::vk::VK_NONCONVERT, false, true, false)],
            ime_toggle: vec![combo(VkCode(0x20), false, false, false)],
        };
        assert_eq!(thumb_forced_open_actions(&special), (None, None));
    }
}

/// `GeneralConfig::muhenkan_solo_tap_dedicated_fn_key`（ADR-091 §D3.2）を
/// `VkCode` に解決する。`bootstrap.rs`（起動時）と `apply_config_update`
/// （reload 時）の両方から呼ぶ。
///
/// `Some(name)` なのに `VkCode::from_name` が解決できない場合（誤字・
/// `"F21"` のような短縮形など）は、専用 Fn キー変換が黙って無効化される
/// （＝設定前と同じ挙動に留まる、安全側）が、原因が分かるよう警告ログを出す。
pub(crate) fn resolve_dedicated_fn_key(name: Option<&str>) -> (Option<VkCode>, Option<String>) {
    let Some(name) = name else {
        return (None, None);
    };
    let resolved = VkCode::from_name(name);
    let warning = resolved.is_none().then(|| {
        format!(
            "general.muhenkan_solo_tap_dedicated_fn_key = {name:?} を VK 名として解決できませんでした。\
             専用 Fn キー変換は無効のままです（\"VK_F18\" または \"F18\" のような VK 名が必要）"
        )
    });
    (resolved, warning)
}

/// IME 状態と修飾キースナップショットから `InputContext` を構築する。
///
/// `modifiers` はフック時点でキャプチャした `ModifierState` を渡すこと。
/// タイマー等のイベント非同期パスでは呼び出し元が `read_os_modifiers()` で取得する。
///
/// `ime_on` は呼び出し元が `platform_state.ime.effective_open()` を評価して渡す。
/// `input_mode` は `ImeStateHub::input_mode()`（SSOT = `shadow_model.input_mode`）から取得する。
/// `is_japanese_ime` は `ImeBelief::is_japanese_ime()` から取得する。
/// `composing` は呼び出し元が `tsf::observer::ime_composition_active_now()` を評価して渡す。
#[must_use]
pub const fn build_input_context(
    ime_on: bool,
    input_mode: InputModeState,
    is_japanese_ime: bool,
    composing: bool,
    modifiers: &awase::engine::ModifierState,
    left_thumb_down: Option<Timestamp>,
    right_thumb_down: Option<Timestamp>,
) -> InputContext {
    InputContext {
        ime_on,
        input_mode,
        is_japanese_ime,
        composing,
        modifiers: *modifiers,
        left_thumb_down,
        right_thumb_down,
    }
}
use awase::yab::YabLayout;

use crate::hook::CallbackResult;
use executor::DecisionExecutor;

// ── LayoutEntry（名前付きレイアウトエントリ）──

/// レイアウト設定一式を保持する構造体
#[derive(Debug)]
pub struct LayoutEntry {
    pub name: String,
    pub layout: YabLayout,
}

pub(crate) struct NonEmptyLayouts(Vec<LayoutEntry>);

impl NonEmptyLayouts {
    pub(crate) fn new(layouts: Vec<LayoutEntry>) -> Option<Self> {
        (!layouts.is_empty()).then_some(Self(layouts))
    }

    pub(crate) fn names(&self) -> Vec<String> {
        self.0.iter().map(|e| e.name.clone()).collect()
    }

    pub(crate) fn into_vec(self) -> Vec<LayoutEntry> {
        self.0
    }

    pub(crate) fn as_slice(&self) -> &[LayoutEntry] {
        &self.0
    }
}

impl LayoutEntry {
    /// `default_layout`（`config.general.default_layout`、`.yab` 拡張子付き）に
    /// 一致するレイアウトのインデックスを返す。一致するものが無ければ `0` に
    /// フォールバックする（`layouts` が空の場合は呼び出し元の責任で扱うこと）。
    ///
    /// 識別は `name`（ファイル名、拡張子抜き）で行う。`.yab` 内部の名前行は
    /// 自由記述でありファイル名と一致する保証が無いため比較に使わないこと
    /// （2026-07-29 実機バグ: かつて内部名前行で比較しており、default_layout が
    /// 一致する内部名を持つファイルが存在しない場合、常に先頭レイアウトへ
    /// 無言でフォールバックしていた）。起動時（`bootstrap::select_default_layout`）・
    /// 設定リロード時（`Runtime::reload_layouts`）の両方から同じロジックを使う。
    ///
    /// `default_layout` に一致するファイルが見つからない場合（存在しない、または
    /// 読込/パースに失敗して `layouts` に含まれていない）、`nicola_keytop` が
    /// あればそちらへフォールバックする（新規インストールの既定と同じ、
    /// BUG-104）。無ければソート順先頭（`0`）にフォールバックする。呼び出し元
    /// (`bootstrap::warn_layout_fallback`) がこのフォールバック発生をユーザーへ
    /// モーダルで通知する。
    ///
    /// 比較は大文字小文字を無視する（Windows のファイルシステムが大文字小文字を
    /// 区別しないため。`default_layout = "nicola.YAB"` のような設定でも
    /// `nicola.yab` ファイルに正しく一致させる。/code-review 指摘: PR #131
    /// の `warn_layout_fallback` 追加で、この不一致が「読込失敗」の誤警告として
    /// 可視化されてしまう問題が見つかった）。
    #[must_use]
    pub fn resolve_index(layouts: &[Self], default_layout: &str) -> usize {
        let default_name = strip_yab_extension(default_layout);
        layouts
            .iter()
            .position(|e| e.name.eq_ignore_ascii_case(default_name))
            .or_else(|| layouts.iter().position(|e| e.name == "nicola_keytop"))
            .unwrap_or(0)
    }
}

/// `.yab` 拡張子を大文字小文字を無視して取り除く。
/// `LayoutEntry::resolve_index` と `bootstrap::warn_layout_fallback`
/// （どちらも `default_layout` の設定名とファイル名(拡張子抜き)を比較する）が
/// 同じロジックを共有するための唯一の実装。
#[must_use]
pub fn strip_yab_extension(name: &str) -> &str {
    // `to_ascii_lowercase()` で判定してから元の文字列を長さでスライスする。
    // 判定が true ということは末尾4バイトが確実に ASCII（'.'+y/a/b の3文字）
    // であることを意味するため、`name.len() - 4` は常に char boundary になる
    // （マルチバイト文字を含む名前(例: "NICOLA＋確定.yab")でも安全。
    // 逆に `name[name.len()-4..]` を先にスライスして判定する実装は、
    // ".yab" で終わらない非ASCII文字列に対して境界外パニックの危険がある）。
    if name.len() >= 4 && name.to_ascii_lowercase().ends_with(".yab") {
        &name[..name.len() - 4]
    } else {
        name
    }
}

/// `[[post_bypass]]` 設定のコンパイル済みエントリ。
///
/// Ctrl+`vk` が PassThrough になった直後、`process`/`class` が一致していれば
/// `platform_state.gate.post_bypass` latch をセットする。
#[derive(Debug, Clone)]
pub(crate) struct PostBypassEntry {
    pub(crate) vk: VkCode,
    /// 小文字化済みプロセス名フィルタ（空=全アプリ）
    pub(crate) process: String,
    /// 小文字化済みクラス名フィルタ（空=全クラス）
    pub(crate) class: String,
}

impl PostBypassEntry {
    pub(crate) fn matches(&self, vk: VkCode, process: &str, class: &str) -> bool {
        self.vk == vk
            && (self.process.is_empty() || process.to_lowercase().contains(self.process.as_str()))
            && (self.class.is_empty() || class.to_lowercase().contains(self.class.as_str()))
    }
}

/// アプリケーションランタイム。
///
/// Engine (判断) と DecisionExecutor (実行) を保持し、配線する。
/// OS イベントの受け取り → Observer → Engine → Executor のパイプラインを駆動する。
///
/// # アーキテクチャ（Facade パターン）
///
/// `Runtime` は以下の論理コンポーネントへの Facade として機能する：
///
/// - [`focus_tracker::FocusTracker`] — フォーカス追跡・IMM 能力学習・sync key 補完
/// - [`ime_coordinator::ImeCoordinator`] — IME apply・パニック回復の調停
///
/// コンポーネント間の相互参照はなく、`Runtime` を介してのみ通信する。
///
/// 注意: 判断ロジックを追加しないこと。判断は Engine が担う。
pub struct Runtime {
    engine: Engine,
    executor: DecisionExecutor,
    pub platform: WindowsPlatform,
    layouts: Vec<LayoutEntry>,
    /// フォーカス追跡・IMM 能力学習・sync key 補完
    focus_tracker: focus_tracker::FocusTracker,
    /// Platform 層の全状態
    platform_state: crate::PlatformState,
    /// 全キーマップルール（アプリフィルタ前）
    all_keymaps: crate::keymap::KeymapTable,
    /// post_bypass コンパイル済みルール一覧
    pub(crate) post_bypass_rules: Vec<PostBypassEntry>,
    /// IME apply・パニック回復の調停
    ime_coordinator: ime_coordinator::ImeCoordinator,
    /// 進行中の IME actuation 試行（ADR-080）。`desired` 変化・`FocusChanged`・
    /// `Resolution` 確定でのみ破棄・再構築する（`runtime/ime_actuation.rs`）。
    active_actuation: Option<ime_actuation::Actuation>,
    /// BUG-113残置課題(2026-09-06): `ConvOpenInference`由来のdrift correctionを
    /// 「明示ユーザー意図エピソードあたり実送信1回」に絞るラッチ
    /// (`state/ime_actuation.rs::decide_conv_inference_drift`)。
    /// **`active_actuation`とライフサイクルを共有しない**——`discard_actuation()`や
    /// `ir_notify_focus_changed`ではリセットしない。Windows Terminal等のXAML/UWP
    /// InputSite子ウィンドウが無操作でも出すフォーカスイベントでこのラッチが
    /// 周期的にリセットされると、実機で確認した「無操作のままVK_IME_OFF×5連射が
    /// 25秒〜4分43秒間隔で再発する」症状が再燃する（docs/known-bugs.md BUG-113参照）。
    conv_drift_latch: Option<crate::state::ime_actuation::ConvDriftEpisode>,
    /// `config1.db` のキーマップ（打鍵時予測用）のキャッシュ。打鍵ごとに読み直さない。
    key_effect_keymap: crate::state::key_effect_predictor::KeymapCache,
    /// 直前のOS読み取り（`OsPoll`）で観測（`ime_on`）を得られたか。時間切れ・空振りは`false`。
    /// 通過マークの窓の間の読み直し間隔（成功なら60ms、失敗なら窓の終了時の1回）に使う。
    last_ime_read_ok: bool,
    /// Microsoft IME本体用（レジストリのキー割り当ての版で読み直す。GJIの`key_effect_keymap`とは別のキャッシュ）。
    key_effect_keymap_native: crate::state::key_effect_predictor::KeymapCache,
    state_dependent_key_warning: crate::state::state_dependent_key_warning::WarningTracker,
    state_dependent_key_warning_dialog:
        crate::state::state_dependent_key_warning::WarningDialogTracker,
    warn_state_dependent_mode_keys: bool,
    /// 単独タップがIMEへ素通しされる親指キーのVK（状態依存キー警告の対象を絞る）。
    passthrough_thumb_mode_keys: Vec<VkCode>,
    /// ADR-195段階4: `<config dir>/keymap-learn-table.json`（段階3永続化）の実行時読込キャッシュ。
    /// `KeyEffectPredicted`（belief更新）に使う。actuationの許可リストは広げない（ADR-195決定(A)）が、
    /// 半角/全角の固定セットの`shadow_action=Toggle`を**外す**方向にだけ参照する（ADR-195追記、
    /// `derive_key_shadow_action`）。
    key_effect_runtime_table: crate::state::key_effect_runtime::RuntimeTableCache,
    /// `config.general.use_learned_keymap_table`（opt-out、既定true）。
    use_learned_keymap_table: bool,
    /// `config.general.predict_henkan_open_in_unreadable_windows`（ADR-209、既定true）。
    predict_henkan_open_in_unreadable_windows: bool,
    /// 役割判定の候補キー（ADR-199決定18(i)、旧ADR-195追記の`hz_toggle_omit_latch`を一般化）の物理キー押下ごとの
    /// 「この打鍵の最終的な`shadow_action`」を、KeyDownで確定して KeyUp まで持ち越すラッチ
    /// （`(scan_code, 判定)`）。学習表の再読込がDownとUpの間に起きても、Down=Allow・Up=Suppressで
    /// KeyDown だけがOSに残る形にしないため。識別は vk でなく scan_code（`VK_DBE_*`はDown/Upでvkが
    /// 変わりうる、BUG-131/132）。**Upで消さず上書きのみ**にする（救済窓・drain 経路の再入で同じ打鍵が
    /// 2回 `enrich_key_role` を通りうる）。
    key_role_latch: Option<(
        awase::types::ScanCode,
        Option<awase::types::ShadowImeAction>,
    )>,
    /// 専用Fnキー変換モード（`muhenkan_solo_tap_dedicated_fn_key`、ADR-091
    /// §D3.2、config.toml による手動設定のみ）が現在有効なら、その vk。
    /// `recompute_active_keymaps` が `[[keymap]]` との衝突チェックに使う
    /// （ADR-114「未解決の疑問」5 対応）。
    muhenkan_dedicated_fn_key_vk: Option<VkCode>,
    /// `config.general.left_thumb_key`/`right_thumb_key` のいずれかが
    /// Space（`VK_SPACE`）か。`true` の場合、MS-IME レジストリ自動検出の
    /// Shift+Space トグルは `engine.set_ime_toggle_auto_keys` へ反映しない
    /// （Space 親指キーの Shift リテラル送出機能との衝突を避けるため、
    /// Opus コードレビュー指摘）。`apply_config_update`/起動時に反映される。
    /// `keys.ime_toggle`（明示設定）とのマッチ判定自体は `Engine` の
    /// `special_keys` が直接持つため、`Runtime` 側に対応するフィールドは
    /// 不要（2026-08-16 ユーザー判断: 明示設定は自動検出キーと併用され、
    /// 一方を排他しない）。
    space_is_thumb_key: bool,
    /// 前回`msime_key_assignment::check_and_warn`が警告を出した割当て内容
    /// （bit0=変換, bit1=無変換、ADR-164フェーズ2、旧
    /// `msime_key_assignment::windows_impl::LAST_WARNED`）。同じ内容で
    /// 繰り返しポップアップを出さないためのデデュープ。`None`＝未警告
    /// （競合が解消された観測でリセットされるため、割当てを解除→再度
    /// 有効化した場合は再警告される）。
    msime_key_assignment_warned: Option<u8>,
    /// BugReport 診断用: 現在ロード済みの `GeneralConfig.keyboard_model`。
    keyboard_model: awase::scanmap::KeyboardModel,
    /// トレイ右クリック時の更新確認を有効にするか。
    pub(crate) update_check_enabled: bool,
    /// OS かな入力ロック検知の通知ヒステリシス。
    kana_lock_hysteresis: KanaLockHysteresis,
    /// hook watchdog が「フック詰まり」を検知した時点でサンプリングした
    /// OS のかな入力ロック状態(前回値、ログの重複抑止用の診断専用メモ)。
    ///
    /// `kana_lock_hysteresis` とは完全に独立。3秒周期のwatchdogサンプルを
    /// 混ぜると、無打鍵でも誤ってトレイ警告が発火しうる。
    watchdog_kana_edge: Option<awase::engine::KanaLockReading>,
    /// ADR-132 Phase 1: drift GiveUp のトレイ通知は1フォーカスにつき1回に制限する。
    drift_giveup_notified_this_focus: bool,
    /// ADR-132 Phase 1 診断用: 直近の GiveUp 通知区間の開始時刻。
    drift_giveup_started_at: Option<std::time::Instant>,
    /// issue #165（hook_starved）自己修復用（2026-09-28追記）。`bootstrap.rs`が
    /// `install_hook()`直後に`set_hook_guard`で格納する（起動時は必ず`Some`）。
    /// `reinstall_keyboard_hook_for_watchdog`がwatchdog検知時にドロップ→
    /// 再インストールして差し替える。ここに保持する理由は、`HookGuard`の
    /// ライフタイムを`Runtime`（`RUNTIME`グローバル、プロセス終了まで生存）に
    /// 揃えることで、watchdogタイマー（`with_app`経由、`Runtime`にしか
    /// アクセスできない）から直接差し替えられるようにするため
    /// （`bootstrap.rs::run`のローカル変数のままでは他所から触れない）。
    hook_guard: Option<crate::hook::HookGuard>,
    /// `[diagnostics] hook_self_heal`（既定 true）。issue #165 自己修復の
    /// ビルド無しキルスイッチ。`state::hook_watchdog::decide` へそのまま渡す。
    hook_self_heal_enabled: bool,
    /// カナリア確認済みの本物のstarvation再インストールを、現在の
    /// hook_starved episode（`hook::hook_alive_tick_ms()`が自然回復するまでの
    /// 連続区間）で何回試みたか。`note_hook_watchdog_recovered`が`0`に
    /// リセットする。`state::hook_watchdog::backoff_delay_ms`の入力
    /// （opus round2 B1(ii)、旧`hook_watchdog_episode_attempted: bool`を置換）。
    hook_watchdog_confirmed_attempt_count: u32,
    /// 次に自己修復（カナリア送信）を試みてよい tick_ms。`None`なら即座に
    /// 試みてよい。カナリア確認済みの再インストール成功/失敗どちらでも
    /// `reinstall_keyboard_hook_for_watchdog`が更新する
    /// （`state::hook_watchdog::backoff_delay_ms`、opus round2 B1(ii)）。
    hook_watchdog_next_retry_at_ms: Option<u64>,
    /// 自己修復（カナリア確認済みの再インストール）を試行した tick_ms の履歴
    /// （レート上限判定用、`state::hook_watchdog::THRASH_WINDOW_MS`より古い
    /// エントリは`reinstall_keyboard_hook_for_watchdog`が随時刈り取る）。
    hook_watchdog_reinstall_history_ms: Vec<u64>,
    /// `WM_WTSSESSION_CHANGE`（`WTS_SESSION_LOCK`/`WTS_SESSION_UNLOCK`）から
    /// 更新する、現在セッションがロック中かの永続フラグ。issue #165 自己修復の
    /// F2ガード（ロック中は再インストールしても意味が無い）に使う。
    session_locked: bool,
    /// issue #165 自己修復のカナリア（`hook::send_hook_watchdog_canary`）を
    /// 送信した tick_ms（opus round2 B1(i)）。`Some`の間は確認待ち
    /// （`TIMER_HOOK_WATCHDOG_CANARY_CHECK`発火まで）で、多重送信を防ぐ
    /// ガードにも使う。確認タイマー発火時に`confirm_hook_watchdog_canary`が
    /// `take()`してクリアする。
    hook_watchdog_canary_sent_at_ms: Option<u64>,
    /// カナリア送信**前**に読んだ`hook::hook_alive_tick_ms()`のスナップ
    /// ショット（opus round1 B1）。`hook_watchdog_canary_sent_at_ms`と常に
    /// 同時にSome/Noneが揃う。`canary_confirmed_starved`の基準値として使う
    /// ——送信「時刻」を基準にすると`GetTickCount64`の分解能（約15.6ms）に
    /// 負けて誤検知するため、代わりにこの「最後にフックが呼ばれた時刻」の
    /// 古い値（この分岐に入る時点で既に5秒以上古い）を基準にする。
    hook_watchdog_canary_baseline_alive_ms: Option<u64>,
    /// `stale_ms<=5000`（フック生存を確認できた）が連続した watchdog tick 数。
    /// `stale_ms>5000`のtickで0にリセットされる。
    /// `state::hook_watchdog::RECOVERY_CONFIRM_TICKS`に達して初めて
    /// `note_hook_watchdog_recovered`（バックオフ/thrash履歴のリセット）を
    /// 実行する（PR #349コードレビュー指摘: 1回のflickerで丸ごとリセット
    /// されないようにするため、`note_hook_watchdog_tick_alive`参照）。
    hook_watchdog_consecutive_alive_ticks: u32,
}

impl std::fmt::Debug for Runtime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Runtime").finish_non_exhaustive()
    }
}

/// `ime_diagnostic` が必要とする Runtime の読み取り専用スナップショット。
#[derive(Clone)]
pub(crate) struct RuntimeDiagnosticSnapshot {
    pub focus_pid: u32,
    pub focus_class: String,
    pub shadow_ime_on: bool,
    pub shadow_is_romaji: bool,
    pub shadow_is_japanese: bool,
    pub last_focus_change_ms: u64,
    pub last_hook_activity_ms: u64,
    pub app_profile: String,
}

impl Runtime {
    #[allow(unsafe_code)] // read_os_modifiers() が Win32 GetKeyState を呼ぶ
    pub(crate) fn build_ctx(&self) -> InputContext {
        // SAFETY: `read_os_modifiers` は Win32 `GetKeyState` を呼ぶのみで副作用はない。
        //         メインスレッドから呼ばれるため、スレッド要件を満たしている。
        let mut modifiers = unsafe { crate::observer::focus_observer::read_os_modifiers() };
        // Alt なりすまし中は本物の Alt 押下を無視する（hook.rs の
        // `is_alt_impersonation_active` doc 参照。ここを直さないと、hook.rs 側の
        // RawKeyEvent.modifier_snapshot は正しく補正されていても、
        // bypass_reason() が実際に見る PhysicalKeyState.modifiers はこの
        // build_ctx() の戻り値から来る（別経路）ため、なりすましたキーが
        // 常に OsModifierHeld でバイパスされてしまう）。
        if crate::hook::is_alt_impersonation_active() {
            modifiers.alt = false;
        }
        let (left_thumb_down, right_thumb_down) = crate::hook::thumb_down_timestamps();
        build_input_context(
            self.platform_state.ime.effective_open(),
            self.platform_state.ime.input_mode(),
            self.platform_state.ime.belief.is_japanese_ime(),
            crate::tsf::observer::ime_composition_active_now(),
            &modifiers,
            left_thumb_down,
            right_thumb_down,
        )
    }

    /// output 層が注入モードを決定するために呼ぶ公開 API。
    ///
    /// focus の `injection_hint()` と `platform_state.app_kind` を組み合わせて
    /// `InjectionHint` を返す。output 層はこのメソッドのみを呼び、
    /// focus/classify の内部型に直接アクセスしない。
    #[must_use]
    pub fn injection_hint(&self) -> (InjectionHint, crate::focus::AppKind) {
        (
            self.platform.injection_hint(),
            self.platform_state.focus.app_kind,
        )
    }

    /// 現在のフォーカス同一性（epoch + hwnd、`probe_admission::ImmLikeTicket::admit`
    /// の照合用、ADR-106 決定3）。両軸を同時に必要とする呼び出し元はこちらを使う。
    #[must_use]
    pub(crate) fn focus_fence(&self) -> crate::state::probe_admission::FocusFence {
        crate::state::probe_admission::FocusFence {
            epoch: self.platform_state.focus.focus_epoch,
            hwnd: crate::state::ime_event::HwndId(self.platform.focus.current.hwnd),
        }
    }

    /// 現在のフォーカスエポック。`focus_fence().epoch` の薄いラッパー
    /// ——epoch と hwnd のペアリング/鮮度が意味を持たない（片方だけで十分な）
    /// 呼び出し元向け。PR 109 コードレビュー軽微3の指摘により、現時点で
    /// epoch 単独の呼び出し元は無いが API として意図的に残す
    /// （`focus_hwnd()` と対称、Task3-c 参照）。
    #[must_use]
    #[allow(dead_code)]
    pub(crate) fn focus_epoch(&self) -> crate::state::probe_admission::FocusEpoch {
        self.focus_fence().epoch
    }

    /// 現在のフォーカス hwnd。`focus_fence().hwnd` の薄いラッパー
    /// ——epoch と hwnd のペアリング/鮮度が意味を持たない（片方だけで十分な）
    /// 呼び出し元向け。
    #[must_use]
    pub(crate) fn focus_hwnd(&self) -> crate::state::ime_event::HwndId {
        self.focus_fence().hwnd
    }

    // ── 実 actuation の起案（ADR-090 §2.A A-1、INV-47）────────────────────

    /// 実 actuation 入口が 1 件の指示を起案する（shadow モード）。
    ///
    /// `strategy` は「どの入口が起案したか」を表す識別子で、A-1 の shadow ログ
    /// （`[warrant-shadow]`）と journal から入口を区別するために使う。
    /// **入口ごとに一意な文字列にすること**——A-2 は「`would_have_blocked` が
    /// ゼロだった入口から順に強制へ倒す」ので、入口が識別できないと分割できない
    /// （ADR-090 §6 ステップ 7）。
    ///
    /// 時刻は `state/` 層へ注入する規約に従い、ここ（`runtime/`）で取得する。
    fn issue_actuation_order(
        &self,
        open: bool,
        strategy: &'static str,
    ) -> crate::state::actuation_chain::ActuationOrder {
        let origin = crate::state::event_origin::EventOrigin::new(
            crate::state::event_origin::EventSource::SelfActuated { strategy },
            crate::state::event_origin::Generation::INITIAL,
        );
        self.issue_actuation_order_with_origin(open, origin)
    }

    /// 呼び出し元が既に `EventOrigin` を持っている場合（drift correction 等）。
    fn issue_actuation_order_with_origin(
        &self,
        open: bool,
        origin: crate::state::event_origin::EventOrigin,
    ) -> crate::state::actuation_chain::ActuationOrder {
        let now = std::time::Instant::now();
        let now_ms = crate::state::TickMs(crate::hook::current_tick_ms());
        self.platform_state
            .ime
            .issue_actuation_order(open, origin, now, now_ms)
    }

    /// 現在フォーカス中のアプリが IMM32 クロスプロセス制御を使えるか返す。
    ///
    /// ADR-158 TE3: `AppImeProfile::can_use_imm32_cross_process`が観測用の
    /// `#[actuation_choke_point]`を付けた際に`const fn`ではなくなったため、
    /// 以前ここにあった`#[expect(clippy::missing_const_for_fn)]`（「const化できる」
    /// というclippy提案の抑制）は不要になった。
    ///
    /// `#[track_caller]`（opus code review S3で追加）: このメソッドは薄いラッパで、
    /// `AppImeProfile::can_use_imm32_cross_process`が`std::panic::Location::caller()`で
    /// 記録する呼び出し元は「直近1段」のみ。このラッパに`#[track_caller]`が無いと、
    /// このラッパ経由の呼び出しがすべて`runtime/mod.rs`のこの行として記録され、
    /// 真の呼び出し元（このラッパをさらに呼んでいる側）が観測ログから消える。
    #[must_use]
    #[track_caller]
    pub fn can_use_imm32_cross_process(&self) -> bool {
        self.platform
            .current_app_profile()
            .can_use_imm32_cross_process()
    }

    /// ADR-205: 外部変化の監視窓（arm・追随の両方）を適用する窓か。`Imm32Unavailable`（Chrome 等）かつ有効な IME が
    /// GJI のときだけ。InputRelay（awase が actuation を所有しない、BUG-90 決定4）と TsfNative（読みが `None`）は対象外。
    /// MS-IME × 実 Chrome の開閉の読みは IME が開いている間も 0 で信用できず（CI 実測: run 36548761653 `imeoff-ext-msime-native` の trace）、GJI 以外への切替後に古い基準値が残る偽 OFF を
    /// 避けるため、開く・閉じるの両方向とも GJI に限る（round: PR #377 Opus レビュー 1・2）。
    #[must_use]
    pub fn external_change_watch_applies(&self) -> bool {
        self.platform.current_app_profile()
            == crate::focus::class_names::AppImeProfile::Imm32Unavailable
            && crate::tsf::observer::tsf_obs().active_ime_kind()
                == crate::tsf::observer::ActiveImeKind::GoogleJapaneseInput
    }

    /// IMM 検出の前後ミス数から、クラス名単位の IMM 能力をキャッシュに記録する。
    ///
    /// 判定は [`FocusTracker::decide_imm_capability`]（純粋関数）に委譲し、
    /// ここではクラス名取得・キャッシュ書き込み・ログの I/O のみ行う。
    pub fn learn_imm_capability_from_miss(&mut self, miss_before: u32, miss_after: u32) {
        if !self.platform.focus.is_focused() {
            return;
        }
        let process_name = self.platform.focus.process_name().to_owned();
        if process_name.is_empty() {
            return;
        }
        let class_name = self.platform.focus.class_name().to_owned();
        let current = self
            .platform
            .focus
            .imm_capability(&process_name, &class_name);
        if let Some(new_cap) = focus_tracker::FocusTracker::decide_imm_capability(
            miss_before,
            miss_after,
            crate::IME_DETECT_MISS_THRESHOLD,
            current,
        ) {
            tracing::info!(
                "IMM capability learned: {process_name}/{class_name} → {new_cap:?} (miss {miss_before}→{miss_after})"
            );
            self.platform
                .learn_imm_capability(process_name, class_name, new_cap);
        }
    }

    /// IME 関連の事前分類情報を sync key 設定で補完する。
    ///
    /// 実処理は [`focus_tracker::FocusTracker::enrich_ime_relevance`] に委譲する。バッチ前処理
    /// （`handle_wm_drain_output_queue`）もここを呼ぶ。候補キーの役割（`shadow_action`）は
    /// **触らない**（[`Self::enrich_key_role`]が `kp_run_inner` で付ける。バッチ前処理との間で
    /// `shadow_action` を読む経路が無いことは ADR-199 T4 で確認済み）。
    pub fn enrich_ime_relevance(&mut self, event: &mut RawKeyEvent) {
        self.focus_tracker.enrich_ime_relevance(event);
    }

    /// 役割判定の候補キー（[`crate::vk::is_role_candidate`]）の打鍵に、役割由来の `shadow_action` を
    /// 付け外しする（ADR-199 決定8。ADR-189/191 の「半角/全角は固定でToggle」を、ユーザーの IME 設定から
    /// 逆算した役割に置き換えた）。`kp_run_inner` の冒頭から呼ぶ。これが`shadow_action`の唯一の上書き点
    /// （`tests/architecture_guard.rs::ime_relevance_shadow_action_writes_are_accounted_for`が
    /// このファイル内の書き込み箇所数を1に固定している）。
    ///
    /// 修飾付きはIME側で別意味を持ちうるので、無修飾の物理キーだけ。観測に依存しないので、
    /// 読めないアプリ（TsfNative）でも効く。ひらがな・カタカナ・英数は入力モードも動かしうるので
    /// 候補外（生のままIMEへ通して追随する、ADR-187のfollow）。
    ///
    /// **配線範囲は半角/全角(0xF3/0xF4)と F13〜F24（決定18）**。無変換/変換（決定16）は `shadow_action` でなく
    /// 単独タップ確定点なのでここでは扱わない（T10）。
    ///
    /// 判定は打鍵ごとのラッチ（[`crate::state::key_effect_runtime::latch_step`]）で KeyDown に確定し、
    /// 同じ物理キーの KeyUp・オートリピート（`was_down`）はそれを使う（Down=Allow・Up=Suppress の非対称防止）。
    /// **F13〜F24 だけの違い**（決定18）: (a) 最初の Down の判定は暫定で、`kp_stage_shadow_ime_toggle` の直後に
    /// [`Self::settle_fkey_role_latch`] が「実際に書いたか」で上書きする。(b) Up・リピートでラッチの scan が
    /// 一致しないとき（別キーがラッチを上書きした）は役割で判定し直さず `None`（Allow）にする。
    /// (c) injected の打鍵には付けない。
    pub fn enrich_key_role(&mut self, event: &mut RawKeyEvent) {
        use crate::state::key_effect_runtime::{latch_step, passive_without_lookup};
        use awase::types::KeyEventType;
        // 全打鍵で通る経路なので、候補キーでないものは修飾キーと IME 種別を見る前に抜ける。
        // 0x19（Alt+半角/全角、ADR-202）は候補集合の外（Alt 付きで届き無修飾ガードを通らない）で、専用に扱う。
        let is_kanji = matches!(event.vk_code.ime_kind(), Some(crate::vk::ImeKeyKind::Kanji));
        if is_kanji && event.injected {
            return; // injected は付けない。静的 Toggle のまま（現行と同じ）。
        }
        if !is_kanji && !crate::vk::is_role_candidate(event.vk_code) {
            return;
        }
        let is_fkey = crate::vk::is_role_fkey(event.vk_code);
        let is_hz = matches!(
            event.vk_code.ime_kind(),
            Some(crate::vk::ImeKeyKind::DbeSbcsChar | crate::vk::ImeKeyKind::DbeDbcsChar)
        );
        // 無変換/変換（決定16）は別の入口（T10）。
        if !is_fkey && !is_hz && !is_kanji {
            return;
        }
        let is_up = event.event_type == KeyEventType::KeyUp;
        let fresh_down = event.event_type == KeyEventType::KeyDown && !event.injected;
        let reuse = is_up || (fresh_down && event.was_down);
        let vk = event.vk_code;
        let m = event.modifier_snapshot;
        let modified = m.ctrl || m.alt || m.shift || m.win;
        let injected = event.injected;
        let has_sync = event.ime_relevance.sync_direction.is_some();
        let static_action = event.ime_relevance.shadow_action; // hook が付けた静的値（0x19 は Toggle）
                                                               // 修飾付き・IME 未同定の打鍵も `None` の判定として**ラッチに記録する**（早期 return しない）。
                                                               // 記録しないと、Ctrl を押したまま半角/全角を Down（判定なし=Allow）→ Ctrl を先に離す →
                                                               // 半角/全角の Up がラッチ空で判定をやり直し `Some(Toggle)`（=Suppress）になり、
                                                               // Down=Allow・Up=Suppress の非対称（BUG-131/132 型）になる（Opus レビュー、PR #326）。
        let (action, latch) = latch_step(
            self.key_role_latch,
            reuse,
            fresh_down,
            event.scan_code,
            || {
                if is_kanji {
                    return self.kanji_shadow_action(vk, static_action, m);
                }
                if passive_without_lookup(is_fkey, modified, injected, reuse, has_sync) {
                    return None;
                }
                // 役割を求められる IME は GJI と、CLSID で同定できた Microsoft IME 本体だけ。GJI 未検出・
                // 第三者 IME・IMM32 HKL のみでは付けず、生キーを通して観測に追随する（レビュー round2 NB3、決定6-3）。
                let ime = crate::tsf::observer::tsf_obs().table_ime_kind()?;
                self.derive_key_shadow_action(ime, vk)
            },
        );
        self.key_role_latch = latch;
        event.ime_relevance.shadow_action = action;
    }

    /// 0x19 の `shadow_action`（ADR-202）。GJI のときだけ `Hankaku/Zenkaku` 行から求める（修飾の扱いは
    /// [`crate::state::key_effect_runtime::kanji_role_plan`]）。GJI 以外は hook の静的値のまま（決定2）。
    fn kanji_shadow_action(
        &mut self,
        vk: VkCode,
        static_action: Option<awase::types::ShadowImeAction>,
        m: awase::types::ModifierState,
    ) -> Option<awase::types::ShadowImeAction> {
        use crate::state::ime_kind::ImeKindId;
        use crate::state::key_effect_runtime::{kanji_role_plan, KanjiRolePlan};
        let is_gji = crate::tsf::observer::tsf_obs().table_ime_kind() == Some(ImeKindId::Gji);
        match kanji_role_plan(is_gji, m.ctrl, m.shift, m.win) {
            KanjiRolePlan::KeepStatic => static_action,
            KanjiRolePlan::Passive => None,
            KanjiRolePlan::Derive => self.derive_key_shadow_action(ImeKindId::Gji, vk),
        }
    }

    /// F13〜F24 の最初の Down（非injected・`!was_down`）で、`kp_stage_shadow_ime_toggle` が**実際に開閉を書いたか**
    /// （役割由来の昇格で`shadow_toggled`）をラッチへ上書きし、一致する Up でラッチを捨てる
    /// （[`crate::state::key_effect_runtime::settle_fkey_latch`]、ADR-199 決定18(i)）。書かなかった打鍵（`is_japanese_ime` が偽・
    /// belief が更新されなかった等）の自動リピートと Up は `None`＝Allow になり、Down だけ Suppress・Up だけ
    /// Suppress の非対称や、書かないのに握りつぶす二重の空振りを作らない。イベント自身の`shadow_action`は
    /// 触らない（配送は `plan` が最初の Down では `shadow_toggled` を見る）。
    pub(crate) fn settle_fkey_role_latch(&mut self, event: &RawKeyEvent, shadow_toggled: bool) {
        use awase::types::KeyEventType;
        if !crate::vk::is_role_fkey(event.vk_code) || event.injected {
            return;
        }
        // 「書いた」は役割由来（`shadow_action` あり）の昇格だけ。同期キー（`keys.ime_detect`）や修飾付きで
        // `shadow_toggled` が立っても、`shadow_action` は付いていない（`passive_without_lookup`）ので書いたことにしない。
        let wrote = shadow_toggled && event.ime_relevance.shadow_action.is_some();
        self.key_role_latch = crate::state::key_effect_runtime::settle_fkey_latch(
            self.key_role_latch,
            event.event_type == KeyEventType::KeyDown && !event.was_down,
            event.event_type == KeyEventType::KeyUp,
            event.scan_code,
            wrote,
        );
    }

    /// 無変換/変換の親指キーの KeyDown で、Engine に渡す**役割由来**の open 軸操作を設定し直す
    /// （ADR-199 決定16、ADR-206 の訂正: 役割由来は単独タップの `ModeKeyConfig` が Passthrough のときだけ発火するので、
    /// config.toml の bare `keys.ime_*` 由来の `set_thumb_forced_open_actions`〈設定に関係なく発火〉とは別の入力
    /// `set_thumb_role_open_actions` に渡す。bare がある側は役割を引かない）。`kp_run_inner` の `engine.on_input` より前から呼ぶ。
    ///
    /// - 対象は非リピートの KeyDown だけ（決定16）。Up・リピートは押下時に決めた値のまま。役割を引くのは非 injected のときだけで、
    ///   injected の Down は config 由来へ戻す。押した側の値だけを書き、もう一方は触らない。
    /// - 打鍵ごとに求め直すので、IME を切り替えたときに古い役割が残らない（決定8 と同じ考え方）。役割が無ければ
    ///   config 由来だけ（無ければ `None`＝従来どおり受動）に戻す。
    /// - 役割は [`Self::derive_key_shadow_action`]（GJI の `config1.db` の逆算・学習表による狭め・config との重なり）。
    ///   MS-IME 本体は `KeyAssignmentMuhenkan`/`Henkan == 2`（トグル、T12）のときだけ役割が付く（T17 Phase 4、
    ///   `KeyEffectKeymap::msime_native_key_role`）。入力中・変換中・候補窓でも除外しない（所有者決定 2026-09-29）。
    ///   発火は ADR-206 の role_open_action（単独タップの ModeKeyConfig が Passthrough のときだけ。Suppress は IME を動かさない。エンジン活性時のみ）。修飾付きの押下では役割を求めない。
    /// - `shadow_action` は付けない（付けると `transport.rs` の先行 Allow と awase の書き込みで二重 actuation、BUG-46 型）。
    ///   物理配送は `Decision::Consume`（PendingThumb）に任せる。発火は FSM が単独タップと解決したときだけ（チョード優先）。
    pub(crate) fn enrich_thumb_key_role(&mut self, event: &RawKeyEvent) {
        use awase::types::KeyEventType;
        let is_muhenkan = event.vk_code == crate::vk::VK_NONCONVERT;
        if !(is_muhenkan || event.vk_code == crate::vk::VK_CONVERT)
            || event.event_type != KeyEventType::KeyDown
            || event.was_down
        {
            return;
        }
        let m = event.modifier_snapshot;
        let modified = m.ctrl || m.alt || m.shift || m.win;
        let ime = crate::tsf::observer::tsf_obs().table_ime_kind();
        let vk = event.vk_code;
        let configured = self.engine.bare_ime_action(vk);
        // injected の Down も設定し直す（役割は引かず config 由来へ戻す）: 早期 return すると、直前の物理打鍵で
        // 決めた役割を引き継いでしまう（BUG-14 の原則・決定8「古い役割を残さない」、PR #331 Opus レビュー）。
        // InputRelay の窓（RDP/VM/PowerToys MWB 等、ADR-119）は actuation を所有しない（`decide_gate` が NotOwned）。
        // ここで役割を付けると、エンジンが生キーを Consume したのに何も送られず、リモート側の IME に届かない
        // （何度押しても変わらない=固着、ADR-206 決定3・7）ので、役割は付けず config の bare だけにする。
        let input_relay = matches!(
            self.platform.current_app_profile(),
            crate::focus::class_names::AppImeProfile::InputRelay
        );
        // bare（`configured`）がある側は config が勝つ（役割は付けない、ADR-199 Q2）。
        let action = if configured.is_some() {
            None
        } else {
            crate::state::key_effect_runtime::thumb_forced_action(
                None,
                ime.is_some() && !input_relay,
                modified,
                event.injected,
                || ime.and_then(|ime| self.derive_key_shadow_action(ime, vk)),
            )
        };
        // **押した側だけ**書く。もう一方の押下中の値を巻き込んで変えない。
        let (muhenkan, henkan) = self.engine.thumb_role_open_actions();
        if is_muhenkan {
            self.engine.set_thumb_role_open_actions(action, henkan);
        } else {
            self.engine.set_thumb_role_open_actions(muhenkan, action);
        }
    }

    /// `vk`（無修飾の候補キー）の役割由来の`shadow_action`（ADR-199 決定4・6・8）。取得（I/O・キャッシュ）だけを
    /// ここで行い、規則の組み合わせは純関数
    /// [`crate::state::key_effect_runtime::key_shadow_action`]（ホストテストあり）に任せる。
    ///
    /// キーマップ・学習表の取得は予測経路（`kp_predict_key_effect`）と同じインスタンス・同じ引数
    /// （`KeymapCache::get_gji`/`get_native`）なので、間引きも共通で I/O は増えない。
    /// `table_ime_kind` で分岐するのは打鍵の時点の同定なので、IME を切り替えたときに古い役割が残らない。
    /// 役割そのものの判定は`ime`に応じて`KeyEffectKeymap::gji_key_role`／`msime_native_key_role`を
    /// 選ぶだけで、`key_shadow_action`自体はIME種別を見ない（T17 opusレビュー、合流点を1つに保つ）。
    fn derive_key_shadow_action(
        &mut self,
        ime: crate::state::ime_kind::ImeKindId,
        vk: VkCode,
    ) -> Option<awase::types::ShadowImeAction> {
        use crate::state::ime_kind::ImeKindId;
        use crate::state::key_effect_predictor::TableKey;
        use crate::state::key_effect_runtime::{hz_omit_may_apply, key_shadow_action};
        let explicit_overlap = self.engine.has_bare_ime_combo(vk);
        let now_ms = crate::hook::current_tick_ms();
        let keymap = match ime {
            ImeKindId::Gji => self.key_effect_keymap.get_gji(now_ms),
            ImeKindId::MsIme => self.key_effect_keymap_native.get_native(now_ms),
        };
        let use_learned = self.use_learned_keymap_table;
        let contradiction = match (
            hz_omit_may_apply(use_learned),
            keymap,
            TableKey::from_vk(vk.0),
        ) {
            (true, Some(keymap), Some(key)) => {
                self.key_effect_runtime_table.get_for_keymap(now_ms, keymap);
                self.key_effect_runtime_table
                    .toggle_contradiction(key)
                    .is_some()
            }
            _ => false,
        };
        let keymap_role = keymap.map(|k| match ime {
            ImeKindId::Gji => k.gji_key_role(vk.0),
            ImeKindId::MsIme => k.msime_native_key_role(vk.0),
        });
        key_shadow_action(explicit_overlap, keymap_role, use_learned, contradiction)
    }

    /// `ImeCoordinator::pending_ime_off_rescue` を取り出し、`TIMER_IME_OFF_RESCUE` をキャンセルする。
    ///
    /// `.take()` と `timer.kill()` は常にペアで呼ぶ必要があるため一元化する。
    pub fn take_ime_off_rescue_pending(&mut self) -> Option<RawKeyEvent> {
        self.platform.timer.kill(crate::TIMER_IME_OFF_RESCUE);
        self.ime_coordinator.pending_ime_off_rescue.take()
    }

    /// `ImeCoordinator::pending_ime_off_rescue` をセットし、`TIMER_IME_OFF_RESCUE` を起動する。
    ///
    /// `.pending = Some(event)` と `timer.set()` は常にペアで呼ぶ必要があるため一元化する。
    pub fn set_ime_off_rescue_pending(&mut self, event: RawKeyEvent) {
        self.ime_coordinator.pending_ime_off_rescue = Some(event);
        self.platform.timer.set(
            crate::TIMER_IME_OFF_RESCUE,
            std::time::Duration::from_millis(50),
        );
    }

    pub fn execute_decision(&mut self, decision: awase::engine::Decision) -> CallbackResult {
        let (callback, sync_outcomes, stripped_set_open) = self.executor.execute_from_loop(
            &mut self.platform,
            &mut self.platform_state.ime,
            decision,
        );
        self.dispatch_outcomes(sync_outcomes);
        if stripped_set_open.is_some() {
            // settle 中に握りつぶした SetOpen は自然には再発行されない
            // （Engine::prev_activation は遷移確定済みのため）。既存の
            // 他の settle 対応経路（撤去済みの apply_force_on_for_imm_broken 等）と同じ「settle 明けに
            // refresh で再試行」パターンで確実に一度だけ再同期する。
            self.schedule_settle_retry("SetOpen stripped from execute_from_loop decision");
        }
        callback
    }

    /// IME apply 完了後の後処理 SSOT。sync / async 両経路から呼ばれる。
    ///
    /// - D: generation 照合で `ImeApplySucceeded` / `ImeApplyFailed` を dispatch
    /// - E: `post_ime_refresh` で IME 状態ポーリングをスケジュール
    ///
    /// sync 経路では `execute_one` が `post_apply_ime_open`（B）を済ませた後、
    /// 呼び出し元が sync_outcomes ループ経由でここへ来る。
    /// async 経路では spawn_local 内で B を済ませた後に直接呼ばれる。
    ///
    /// `reason`（ADR-086 §4 INV-18、Phase 3 item 2）は「なぜこの apply が
    /// 起きたか」を申告する必須引数。`Option` にしない——デフォルトを許すと
    /// provenance が欠落する呼び出しが紛れ込む。`record_ime_apply_result` は
    /// `generation.is_some()` のときだけ `ImeEvent` を dispatch するため
    /// （force 系の適用は generation を持たずこの経路を通らない）、`reason` は
    /// ジャーナルへ直接記録することで force 系も含めた全経路の provenance を
    /// 一意に残す。
    #[tracing::instrument(level = "debug", skip_all, fields(open = open, ?outcome, ?generation, ?reason))]
    pub fn on_ime_apply_complete(
        &mut self,
        open: bool,
        outcome: awase::platform::ImeOpenOutcome,
        generation: Option<crate::state::ApplyGeneration>,
        reason: crate::state::ime_event::OpenApplyReason,
    ) {
        use awase::platform::TsfComposition as _;

        self.platform_state
            .ime
            .journal
            .record(crate::journal::JournalEntry::ImeOpenApplied {
                open,
                outcome,
                reason,
            });

        // E: UnsafeToToggle でも必ずスケジュールする。UnsafeToToggle は
        // Win-held 等の genuine skip に加え、ADR-086 の `ActuationOutcome::Aborted`
        // （フォーカス移動/世代不一致で書き込みを中止）や capture 失敗も含む。
        // 特に `Aborted(GenStale)`（同一ウィンドウのままフォーカス世代だけ進んだ
        // ケース）は意図（IME open/close）自体は依然有効なのに、それを再試行する
        // 自然なトリガー（新しいフォーカス変更）が発生しない。以前はここで早期
        // return しており、次に無関係なイベントが来るまで無期限に取りこぼされ
        // 得た（opus レビュー指摘 F3、2026-08-08 是正、`ime::
        // set_ime_open_then_conv_for_target` の doc 参照）。
        self.platform.post_ime_refresh();

        // BUG-34 横展開 D-prep: UnsafeToToggle をここで早期 return すると、
        // generation 付きで立てた pending が record_ime_apply_result まで
        // 届かず永久に残留する（以後の別 generation の完了が全て stale
        // 判定され続ける固着になる）。record_ime_apply_result 自身が
        // UnsafeToToggle を判別して pending だけ解放し applied は動かさない
        // ため、ここでは早期 return せず必ず通す。

        // C+D: ImeModel write-back + generation 照合 dispatch
        let acceptance = self.platform_state.ime.record_ime_apply_result(
            open,
            outcome,
            generation,
            crate::hook::current_tick_ms(),
        );

        // B: composition warm/cold 更新。stale apply 完了は GJI/Composition に伝播させない。
        if acceptance.drives_composition_side_effects() {
            self.platform.on_ime_applied(open, outcome);
        }
    }

    /// sync path の outcome リストを一括 dispatch する。
    pub(crate) fn dispatch_outcomes(&mut self, outcomes: Vec<ImeApplyPair>) {
        for completion in outcomes {
            self.on_ime_apply_complete(
                completion.open,
                completion.outcome,
                completion.generation,
                completion.reason,
            );
        }
    }

    /// 現在の shadow model から `ImeControlView` を構築する。
    pub(crate) fn shadow_ime_control_view(&self) -> crate::state::ImeControlView<'_> {
        let mut view = self
            .platform
            .build_ime_control_view(self.platform_state.ime.model().applied_pair());
        view.belief_input_mode = self.platform_state.ime.input_mode();
        view
    }

    /// エンジンの有効/無効を切り替え、Decision を実行する
    pub fn toggle_engine(&mut self) {
        // 「無効化された瞬間」を捉えるスナップショットは on_command より前で
        // 取る必要がある — on_command 自体が NicolaFsm::toggle_enabled で
        // is_user_enabled を同期的に書き換えるため、execute_decision の中で
        // 読むと既に更新後の値になってしまい判定が常に false になる
        // （issue #137 3周目のレビューで指摘・修正）。
        let was_user_enabled = self.engine.is_user_enabled();
        let ctx = self.build_ctx();
        let decision = self.engine.on_command(EngineCommand::ToggleEngine, &ctx);
        self.execute_decision(decision);
        if was_user_enabled && !self.engine.is_user_enabled() {
            // エンジンが無効化されている間は romaji VK を送信しないため
            // kp_stage_kana_lock_warn のサンプリング自体が止まる。無効化前の
            // 警告状態がトレイに固着し続け、実際のON/OFF状態が確認できなく
            // なるのを避けるため、ここでヒステリシスとトレイ表示の両方を
            // 一律リセットする。
            self.kana_lock_hysteresis = KanaLockHysteresis::new();
            self.drift_giveup_notified_this_focus = false;
            self.drift_giveup_started_at = None;
            self.platform.tray.set_kana_lock_warned(false);
        }
    }

    /// エンジンを無条件で ON にする（トグルではなく強制）。
    /// トレイの「状態をリセット」等、現在の ON/OFF に関わらず必ず有効化したい場合に使う。
    pub fn force_engine_on(&mut self) {
        let ctx = self.build_ctx();
        let decision = self.engine.on_command(EngineCommand::ForceEngineOn, &ctx);
        self.execute_decision(decision);
    }

    /// 外部コンテキスト喪失時にエンジンの保留状態を安全にフラッシュする。
    pub fn invalidate_engine_context(&mut self, reason: ContextChange) {
        let ctx = self.build_ctx();
        let decision = self
            .engine
            .on_command(EngineCommand::InvalidateContext(reason), &ctx);
        self.execute_decision(decision);
    }

    /// IME 状態とフォーカス状態を一括で再観測し、Engine に通知する。
    ///
    /// フォーカスデバウンス後・500ms ポーリング・may_change_ime 後など、
    /// 全ての IME/フォーカス更新がこのメソッドに集約される（ADR 028）。
    ///
    /// 処理フロー:
    /// 1. 現在のフォーカス先を取得・分類（focus_kind, app_kind 更新）
    /// 2. 前面プロセスが変わった場合は Engine に FocusChanged（flush あり）
    /// 3. IME 状態を再取得して Preconditions を更新
    /// 4. Engine に RefreshState（active 状態の遷移検知）
    /// 5. 次回ポーリングを自動スケジュール
    ///
    /// メッセージループ上で呼ぶこと（ブロッキング OK）。
    pub fn refresh_ime_state_cache(&mut self) {
        self.run_ime_refresh();
    }

    /// IME リフレッシュを非同期タスクとしてスポーン。
    /// with_app の外でフェッチを行い、完了後に with_app で適用する。
    pub fn spawn_ime_refresh(&mut self) {
        self.platform.timer.kill(crate::TIMER_IME_REFRESH);

        // NOTE: ここで send_eager_tsf_warmup() を呼ばない。
        // focus_transition_pending=true の時点では injection_mode が前ウィンドウ（WezTerm 等）
        // の stale な Tsf のままであり、新しいウィンドウが Chrome/Edge の場合に誤って
        // VK_DBE_HIRAGANA を送信して Chrome の IME を ON にしてしまうバグがあった。
        // eager warmup は post_focus_change_snapshot (run_with_prefetched 内) で injection_mode
        // 確定後に正しく送信される。

        win32_async::spawn_local(async {
            let focus = crate::focus::probe::run_focus_probe_async().await;
            let snap = crate::ime::read_ime_state_full_async().await;
            let _ = crate::with_app(|app| {
                app.run_ime_refresh_with_prefetched(focus, &snap);
                app.settle_tsf_gate_after_refresh();
            });
        });
    }

    /// 統合 IME リフレッシュタイマーをスケジュール（リセット）する。
    ///
    /// 既存のタイマーをキャンセルして `delay_ms` 後に再設定する。
    /// フォーカス変更(50ms) / ポーリング(500ms) / 即時(0ms) を統一的に扱う。
    pub fn schedule_ime_refresh(&mut self, delay_ms: u64) {
        self.platform.timer.set(
            crate::TIMER_IME_REFRESH,
            std::time::Duration::from_millis(delay_ms),
        );
    }

    /// フォーカス復帰後 resync（report `01M0VGJ2M5KQHD1D9V7HAMBHNT`）のハード期限
    /// タイマーをスケジュールする。resync 完了（`kp_trigger_focus_resync`）が
    /// この期限より先に `FocusResyncGate` を閉じれば、このタイマーが発火しても
    /// `open_if_current` が世代不一致/既 close で `false` を返すため無害。
    pub(crate) fn schedule_focus_resync_deadline(&mut self) {
        self.platform.timer.set(
            crate::TIMER_FOCUS_RESYNC,
            std::time::Duration::from_millis(crate::tuning::FOCUS_RESYNC_DEADLINE_MS),
        );
    }

    /// settle 期間中に IME apply/decision をスキップしたとき、settle 明けに refresh で
    /// 一度だけ再試行する「確立済みパターン」（`executor::strip_ime_set_open_if_settling`
    /// doc 参照）を一元化する。
    ///
    /// 遅延は settle 残余の上限（= `focus_settle_ms()`）+ タイマー粒度マージン 50ms。
    /// `reason` はログの `[focus-settle] {reason} → ...` に埋め込まれる、呼び出し元ごとの
    /// 説明文（例: `"drift correction skipped (settling)"`）。
    pub fn schedule_settle_retry(&mut self, reason: &str) {
        let retry_ms = self.platform_state.ime.focus_settle_ms() + 50;
        tracing::debug!("[focus-settle] {reason} → {retry_ms}ms 後に refresh で再試行");
        self.schedule_ime_refresh(retry_ms);
    }

    /// `ImeEvent::InputModeApplied`（`result` は常に `Applied`）の dispatch を一元化する。
    ///
    /// awase 自身の能動的な input_mode 訂正（`InputModeApplyStrategy` 参照）は、常に
    /// `result: Applied` 固定・5 フィールドの構築が `mode`/`strategy`/`tick_ms` だけ
    /// 違う形で複数箇所に複製されていた。`Skipped` を構築する経路は `state/ime_model.rs`
    /// 内の別経路専用でありここでは扱わない。
    pub fn apply_input_mode_correction(
        &mut self,
        mode: InputModeState,
        strategy: crate::state::ime_event::InputModeApplyStrategy,
        tick_ms: crate::state::TickMs,
    ) {
        self.platform_state.ime.dispatch_event(
            crate::state::ime_event::ImeEvent::InputModeApplied {
                mode,
                strategy,
                result: crate::state::ime_event::InputModeApplyResult::Applied,
                at: tick_ms,
            },
            tick_ms,
        );
    }

    /// ポーリング間隔設定に従って次回 IME リフレッシュをスケジュールする。
    pub fn reschedule_ime_refresh(&mut self) {
        // TsfNative は read_ime_state_full が常に None、GJI も predates-focus-change でスキップ。
        // explicit_intent の有無に関わらずポーリングで得られる情報がないため常に停止する。
        // explicit_intent が確定している他プロファイルも同様に停止。
        // 再開トリガー: フォーカス変更 / may_change_ime キー（20ms タイマー）/
        // `kp_apply_conv_engine_sync` の ReportOpenInference（BUG-51、20ms）。
        //
        // NOTE: `conv_mode_policy = force` に応じてこの早期 return をスキップする
        // 例外が過去に存在した（`apply_force_on_for_imm_broken` の周期 force-ON
        // 再送を同じリフレッシュ連鎖に相乗りさせるため）。2026-08-17、ADR-094 で
        // force ポリシー自体を撤去したのに伴い削除した。`apply_force_on_for_imm_broken`
        // は常時この早期 return の影響を受けていた（その後 `f83084b3` で関数ごと撤去済み）。
        let is_tsf_native = self
            .platform
            .current_app_profile()
            .is_effectively_tsf_native(self.platform.focus.class_name());
        if is_tsf_native {
            return;
        }
        // ADR-205: 外部注入の IME キー直後の監視窓が生きている間は、明示意図の有無に関わらず読み直しを予約する
        // （明示意図があると下の分岐でポーリングが止まり、窓の中の読みが届かない）。
        let now_for_watch = crate::hook::current_tick_ms();
        if let Some(remaining) = self
            .platform_state
            .ime
            .external_change_watch_remaining_ms(now_for_watch)
        {
            self.schedule_ime_refresh(crate::tuning::MODE_KEY_PASS_REREAD_MS.min(remaining + 1));
            return;
        }
        // ADR-187: 無変換/変換の生キー通過後、窓が有効な間は follow の読み直しを予約する。通常のポーリング間隔で
        // 上書きしない。意図を捨てた後は`explicit_intent()`が`None`になるため、ここで上書きすると読み直しが
        // 窓(300ms)より後(既定500ms)に飛び、最初の観測が古い状態を読んだ回で追随できない。
        // 直前の読み取りが成功したなら`MODE_KEY_PASS_REREAD_MS`ごと、失敗したなら窓の終了時の1回に絞る
        // （`mode_key_pass_next_read_ms`。失敗する環境で60msごとに読むとprobeが重なり、3回連続失敗で
        // `imm-learning`が窓を誤って降格する。BUG-158）。窓が切れた直後のtickで`ir_stage_notify`が古い意図を
        // 捨てる（意図が残ってポーリングが止まらない）。
        // 読めない窓（`Imm32Unavailable`等）では読み取り自体ができず、意図が読み取りで訂正される見込みが無い
        // ので、通過マークの読み直しも窓終了時の意図の破棄もしない（意図はbeliefの唯一の手がかりとして残る。
        // 破棄するとCIのblind条件でEngineずれが0→22〜25%に悪化した）。
        let now_ms = crate::hook::current_tick_ms();
        if !self.can_use_imm32_cross_process() {
            // 立てた時点で読めた通過が、途中の降格で読めない窓になったときは、窓の終了時に1回だけ起こして
            // 古い意図を捨てる（`ir_stage_notify`）。起こさないと通過マークが有効な間は何も予約されず、
            // 意図が残ってポーリングが止まる（BUG-151原因③、レビュー round2 A-N2）。
            if let Some(remaining) = self.platform_state.ime.mode_key_pass_expiry_wait_ms(now_ms) {
                self.schedule_ime_refresh(remaining + 1);
                return;
            }
            // 読めない窓は従来どおり（ADR-187）: 明示意図があれば停止、通過マークが有効な間は上書きしない。
            if self.platform_state.ime.explicit_intent().is_some()
                || self.platform_state.ime.mode_key_pass_mark_live(now_ms)
            {
                return;
            }
        } else if let Some(remaining) = self
            .platform_state
            .ime
            .mode_key_pass_window_remaining_ms(now_ms)
        {
            self.schedule_ime_refresh(crate::state::mode_key_pass::mode_key_pass_next_read_ms(
                self.last_ime_read_ok,
                remaining,
                crate::tuning::MODE_KEY_PASS_REREAD_MS,
            ));
            return;
        }
        if self.platform_state.ime.explicit_intent().is_some() {
            return;
        }
        self.schedule_ime_refresh(u64::from(self.platform_state.focus.ime_poll_interval_ms));
    }

    /// `spawn_ime_refresh` の async タスク内で IME リフレッシュ後に TsfGate を遷移させる。
    ///
    /// `run_ime_refresh_with_prefetched` 完了後に呼ぶ。`last_focus_info` が更新済みのため
    /// `injection_hint` を読んで正しい TsfGate 状態に遷移できる。
    fn settle_tsf_gate_after_refresh(&mut self) {
        // PendingWarmup 以外（Probing/Ready/Bypass）なら空 Vec が返る。
        // confirm_tsf は PendingWarmup/Bypass → Probing、bypass_tsf は PendingWarmup/Probing → Bypass。
        let is_tsf = matches!(self.platform.injection_hint(), InjectionHint::ForceTsf);
        let held = if is_tsf {
            self.platform.confirm_tsf()
        } else {
            // ここで belief（IME open observation）を書いてはならない。
            // かつて ce45b82 が「非TSFウィンドウには日本語IMEが存在しない」という誤った
            // 前提で write_focus_probe(false) の偽観測を注入していたが、Edge/Chrome
            // （Imm32Unavailable, injection=Unicode）は非TSF注入かつ日本語IME有効であり、
            // 実観測経路を持たないため偽 Low false が most_recent_trusted() 経由で belief を
            // 支配し、フォーカス約500ms後（次ポーリング）に Engine が必ず OFF になった
            // （docs/known-bugs.md BUG-07）。ce45b82 の元バグ（Win+X メニューの1文字
            // ショートカットが NICOLA 変換される）は、現在は classify.rs の既知 NonText
            // クラス判定 + message_handlers.rs の NonText パススルーが belief と独立に防ぐ。
            self.platform.bypass_tsf()
        };
        self.platform.timer.kill(crate::TIMER_TSF_GATE);
        if !held.is_empty() {
            tracing::debug!(
                "[tsf-gate] draining {} held keys via INPUT_DEFER",
                held.len()
            );
            crate::INPUT_DEFER.replay_later(held);
        }
    }

    /// IME を実際に ON/OFF する直接呼び出し（`Decision`/`Effect` を経由しない経路）が、
    /// フォーカス遷移の settle 期間中に実行されるべきでないかどうかを判定する。
    ///
    /// `execute_decision` 経由の `Decision` ベースの経路は
    /// `Executor::execute_from_loop` が一括でガードするが、`platform.set_ime_open` を
    /// 直接呼ぶ経路（`ir_apply_drift_correction` 等。撤去済みの
    /// `apply_force_on_for_imm_broken`/`try_force_on_bootstrap` も同型だった）は `Decision`/`Effect` という抽象を経由しないため
    /// そちらのガードが効かない。これらの呼び出し元は実行前に必ずこれを確認すること。
    ///
    /// 2026-07-05: Alt+Tab 中間ウィンドウへの一瞬のフォーカス中に、これらの直接呼び出しが
    /// settle 前の不安定な状態に基づいて IME を実際に切り替えてしまうバグの修正。
    ///
    /// **2026-08-21（ADR-098 決定2/4、BUG-69）訂正**: 旧記載にあった
    /// `apply_ime_open_with_applied` / `ir_post_focus_change_snapshot` 内の
    /// 「GJI 強制 ON ブロック」は、到達不能だったため決定2 で撤去済み
    /// （メソッド自体も削除）。同関数内の「IME OFF 強制ブロック」（enforce-OFF）
    /// は、そもそもここに含めるべきではなかった——`Standard`（ImmCross）
    /// でしか実効せず、到達までに `ImeDiagnosticSnapshot::capture()` が
    /// 最大 ~250ms ブロックしうる（ImmCross の settle は 100ms）ため、
    /// defer チェックを足すと「診断キャプチャがどれだけブロックしたか」で
    /// 発火が決まる非決定的な挙動になり、正常時は不発・ハング時だけ発火する
    /// という意図と正反対になる。加えて `ImeDiagnosticSnapshot::capture` は
    /// BUG-34 が撤去/非同期化の対象とする同族の同期 `SendMessageTimeoutW` を
    /// 含むため、BUG-34 が進めばこのゲートは静かに「常に不発」へ反転する。
    /// （この enforce-OFF ブロック自体は 2026-09-25 に撤去した。docs/adr/191-calibration-experiments.md「A/B-1」。）
    pub(crate) fn ime_apply_should_defer(&self) -> bool {
        self.platform_state
            .ime
            .is_focus_transition_settling(std::time::Instant::now())
    }

    /// 設定リロード時にレイアウト一覧を再スキャンし、`default_layout` に追従させる。
    ///
    /// 設定画面の「適用」（再起動なしの即時反映）でレイアウト切り替えが効かない、
    /// という報告（2026-07-29）に対応するもの。それまで `reload_config` は
    /// スレッショルド・キー設定等は再読込していたが、レイアウトだけは対象外で、
    /// 再起動しない限り反映されなかった。
    ///
    /// レイアウトが実質変わっていない場合は `switch_layout` を呼ばない。
    /// `EngineCommand::SwapLayout` は保留中のキーを flush する副作用があるため、
    /// 内容が変わっていないのに設定リロードのたびにタイピング中のキーを
    /// 確定させてしまうことを避ける。
    pub(crate) fn reload_layouts(&mut self, layouts: Vec<LayoutEntry>, default_layout: &str) {
        let Some(layouts) = NonEmptyLayouts::new(layouts) else {
            tracing::warn!("reload_layouts: no layouts found, keeping current layout");
            return;
        };

        let names = layouts.names();
        let index = LayoutEntry::resolve_index(layouts.as_slice(), default_layout);
        let target_name = layouts.as_slice()[index].name.clone();
        let unchanged = self.platform.tray.current_layout_name() == target_name;

        self.layouts = layouts.into_vec();
        self.platform.tray.set_layout_names(names);

        if unchanged {
            return;
        }

        self.switch_layout(index);
    }

    /// 配列を動的に切り替える
    pub fn switch_layout(&mut self, index: usize) {
        let Some(entry) = self.layouts.get(index) else {
            tracing::warn!("Layout index {index} out of range");
            return;
        };

        let name = entry.name.clone();
        let decision = self.engine.on_command(
            EngineCommand::SwapLayout(entry.layout.clone()),
            &self.build_ctx(),
        );
        self.execute_decision(decision);

        self.platform.tray.set_layout_name(&name);

        tracing::info!("Switched layout to: {name}");
    }

    /// 手動アプリオーバーライドのトグル処理
    pub fn toggle_app_override(&mut self) {
        let current = self.platform_state.focus.focus_kind;
        let new_kind = if current == FocusKind::TextInput {
            FocusKind::NonText
        } else {
            FocusKind::TextInput
        };

        self.platform_state.focus.focus_kind = new_kind;

        // Update learning cache
        if self.platform.focus.is_focused() {
            let pid = self.platform.focus.pid();
            let cls = self.platform.focus.class_name().to_owned();
            self.platform
                .focus
                .cache_insert(pid, cls, new_kind, DetectionSource::UserOverride);
        }

        // If demoted to NonText, flush engine pending
        if new_kind == FocusKind::NonText {
            self.invalidate_engine_context(ContextChange::FocusChanged);
        }

        // バルーン通知を表示
        self.platform.tray.show_balloon(
            "awase",
            if new_kind == FocusKind::TextInput {
                "テキスト入力モードに切り替えました"
            } else {
                "バイパスモードに切り替えました"
            },
        );

        let mode_str = if new_kind == FocusKind::TextInput {
            "TextInput (engine enabled)"
        } else {
            "NonText (engine bypassed)"
        };
        tracing::info!("Manual focus override: → {mode_str}");
    }

    /// Sync key 後に遅延されたキーを再処理する。
    ///
    /// sync key で guard が起動された後、KeyUp で OS が IME を切り替えてから呼ばれる。
    /// guard 解除 → IME 状態 refresh → Engine 通知 → バッファキー再処理。
    /// メッセージループ上で呼ぶこと（ブロッキング OK）。
    #[allow(unsafe_code)] // poll_and_classify_ime() が Win32 IMM API を呼ぶ
    pub fn process_deferred_keys(&mut self) {
        // Guard を解除し、保留キーを回収
        let keys = self.platform_state.gate.sync_key_gate.deactivate();
        tracing::debug!("IME guard OFF (process_deferred_keys)");

        // Refresh IME state (Observer → ImeObservations → Preconditions)
        // SAFETY: `poll_and_classify_ime` は Win32 IMM API（`ImmGetContext` 等）を呼ぶ unsafe fn。
        //         メッセージループ上（メインスレッド）から呼ばれるためスレッド要件を満たす。
        let observer_out = unsafe {
            crate::observer::ime_observer::poll_and_classify_ime(
                self.platform_state.ime.effective_open(),
                self.platform_state.ime.is_force_on_guard_active(),
                self.platform_state.ime.input_mode(),
                self.platform_state.ime.belief.prev_conversion_mode(),
                self.platform.focus.process_name(),
            )
        };
        let tick_ms = crate::state::TickMs(crate::hook::current_tick_ms());
        let accepted =
            crate::state::probe_admission::AcceptedObservation::for_sync(self.focus_fence());
        self.platform_state
            .ime
            .apply_ime_update(&observer_out, tick_ms, accepted);

        // LastAppliedImeState を OS 観測値に同期する。
        // 物理 Kanji キー（sync key）は apply_ime_open を経由しないため last_applied が更新されない。
        // last_applied が stale なまま Engine が activate → SetOpen(true) へ進むと、
        // 直後の force-on / focus-resync が古い状態を根拠に動く。観測済みのOS状態で
        // mirrorしてから戻すことで、物理キー起点の状態変化をモデルへ反映する。
        //
        // ADR-098 決定5: この関数（`process_deferred_keys`）自体は `SyncKeyGate::
        // activate()`/`try_push()` の呼び出し元が現状ゼロのため本番到達不能——
        // 到達すれば、直前の `poll_and_classify_ime` の新鮮な観測を経由せず
        // `effective_open()`（belief、explicit-intent 分岐が優先される）を
        // そのまま書く点に注意。将来 sync key gate を復活させる際は
        // `focus_tracking.rs:409` と同型のプロファイル分岐を検討すること。
        let observed_ime_on = self.platform_state.ime.effective_open();
        self.platform_state
            .ime
            .record_confirmed(observed_ime_on, tick_ms.0);
        tracing::debug!("[process-deferred] applied_open → {observed_ime_on} (sync with OS poll)");

        // Engine に IME 状態変化を即通知する（deferred keys の有無にかかわらず）。
        {
            let ctx = self.build_ctx();
            let decision = self.engine.on_command(EngineCommand::RefreshState, &ctx);
            self.execute_decision(decision);
        }

        if keys.is_empty() {
            return;
        }

        tracing::debug!("Processing {} deferred key(s) after IME toggle", keys.len());

        for (event, _phys) in keys {
            // Build fresh context with updated preconditions
            let ctx = self.build_ctx();
            let decision = self.engine.on_input(event, &ctx);
            self.execute_decision(decision);
        }
    }

    // ── app/ 境界 API（private フィールドへのアクセスを app/ に許可しない）──

    /// Runtime を初期化して返す。
    #[expect(clippy::too_many_arguments)]
    pub(crate) fn new(
        engine: Engine,
        executor: DecisionExecutor,
        platform: WindowsPlatform,
        layouts: Vec<LayoutEntry>,
        sync_toggle_keys: Vec<VkCode>,
        sync_on_keys: Vec<VkCode>,
        sync_off_keys: Vec<VkCode>,
        platform_state: crate::PlatformState,
        all_keymaps: crate::keymap::KeymapTable,
        post_bypass_rules: Vec<PostBypassEntry>,
    ) -> Self {
        Self {
            engine,
            executor,
            platform,
            layouts,
            focus_tracker: focus_tracker::FocusTracker::new(
                sync_toggle_keys,
                sync_on_keys,
                sync_off_keys,
            ),
            platform_state,
            all_keymaps,
            post_bypass_rules,
            ime_coordinator: ime_coordinator::ImeCoordinator::new(),
            active_actuation: None,
            conv_drift_latch: None,
            key_effect_keymap: crate::state::key_effect_predictor::KeymapCache::default(),
            last_ime_read_ok: true,
            key_effect_keymap_native: crate::state::key_effect_predictor::KeymapCache::default(),
            state_dependent_key_warning:
                crate::state::state_dependent_key_warning::WarningTracker::default(),
            state_dependent_key_warning_dialog:
                crate::state::state_dependent_key_warning::WarningDialogTracker::default(),
            warn_state_dependent_mode_keys: true,
            passthrough_thumb_mode_keys: Vec::new(),
            key_effect_runtime_table: crate::state::key_effect_runtime::RuntimeTableCache::default(
            ),
            use_learned_keymap_table: true,
            predict_henkan_open_in_unreadable_windows: true,
            key_role_latch: None,
            muhenkan_dedicated_fn_key_vk: None,
            space_is_thumb_key: false,
            msime_key_assignment_warned: None,
            keyboard_model: awase::scanmap::KeyboardModel::default(),
            update_check_enabled: true,
            kana_lock_hysteresis: KanaLockHysteresis::new(),
            watchdog_kana_edge: None,
            drift_giveup_notified_this_focus: false,
            drift_giveup_started_at: None,
            hook_guard: None,
            hook_self_heal_enabled: true,
            hook_watchdog_confirmed_attempt_count: 0,
            hook_watchdog_next_retry_at_ms: None,
            hook_watchdog_reinstall_history_ms: Vec::new(),
            session_locked: false,
            hook_watchdog_canary_sent_at_ms: None,
            hook_watchdog_canary_baseline_alive_ms: None,
            hook_watchdog_consecutive_alive_ticks: 0,
        }
    }

    pub(crate) const fn keyboard_model(&self) -> awase::scanmap::KeyboardModel {
        self.keyboard_model
    }

    pub(crate) const fn set_keyboard_model(&mut self, model: awase::scanmap::KeyboardModel) {
        self.keyboard_model = model;
    }

    pub(crate) const fn set_predict_henkan_open_in_unreadable_windows(&mut self, enabled: bool) {
        self.predict_henkan_open_in_unreadable_windows = enabled;
    }

    pub(crate) const fn set_update_check_enabled(&mut self, enabled: bool) {
        self.update_check_enabled = enabled;
    }

    pub(crate) const fn set_warn_state_dependent_mode_keys(&mut self, enabled: bool) {
        self.warn_state_dependent_mode_keys = enabled;
    }

    /// 状態依存キー警告の対象にする親指キーを、設定（抑止・専用Fnキー・bare `keys.ime_*`）から決める。
    /// bare の開閉（旧 `*_solo_tap_ime_action` の移行分を含む）を持つキーは awase が単独タップを消費する。
    /// IME 設定由来の役割は打鍵ごとにしか求まらず静的に判定できないため、ここでは含めない
    /// （役割のあるキーで警告が出ることがあるが、警告は案内であり動作には影響しない）。
    pub(crate) fn set_passthrough_thumb_mode_keys(
        &mut self,
        general: &awase::config::GeneralConfig,
    ) {
        use awase::engine::ModeKeyConfig;
        // bare の開閉は `set_thumb_forced_open_actions`（起動時・`apply_config_update` の冒頭）が設定済み。
        let (muhenkan_bare, henkan_bare) = self.engine.thumb_forced_open_actions();
        self.passthrough_thumb_mode_keys =
            crate::state::state_dependent_key_warning::passthrough_thumb_vks(
                ModeKeyConfig::from_legacy_bools(
                    general.muhenkan_solo_tap_ignore_composing_guard,
                    general.muhenkan_solo_tap_always_suppress,
                )
                .is_passthrough(),
                general.muhenkan_solo_tap_dedicated_fn_key.is_some() || muhenkan_bare.is_some(),
                ModeKeyConfig::from_legacy_bools(
                    general.henkan_solo_tap_ignore_composing_guard,
                    general.henkan_solo_tap_always_suppress,
                )
                .is_passthrough(),
                henkan_bare.is_some(),
            );
    }

    /// ADR192-T5: 状態依存キー警告の判定に使う、採用中の学習表のセル。予測器
    /// （`kp_predict_key_effect`）と同じ`RuntimeTableCache`・同じ検証キーで引くので、
    /// 予測器が使う表と警告が見る表は常に一致する（学習表が無効/未採用なら`None`＝同梱表）。
    fn learned_cells_for_warning(
        &mut self,
        now_ms: u64,
        keymap: Option<&crate::state::key_effect_predictor::KeyEffectKeymap>,
    ) -> Option<Vec<crate::state::key_effect_predictor::Cell>> {
        let keymap = keymap?;
        if !self.use_learned_keymap_table {
            return None;
        }
        self.key_effect_runtime_table
            .get_for_keymap(now_ms, keymap)
            .map(<[_]>::to_vec)
    }

    pub(crate) fn check_state_dependent_mode_keys(&mut self, google_ime: bool) {
        let (left, right) = crate::hook::thumb_vk_codes();
        let gji_stamp = google_ime
            .then(crate::gji_charset_autodetect::config1_db_stamp)
            .flatten();
        let now_ms = crate::hook::current_tick_ms();
        let warnings = if google_ime {
            let keymap = crate::gji_charset_autodetect::read_key_effect_keymap();
            let learned = self.learned_cells_for_warning(now_ms, keymap.as_ref());
            self.state_dependent_key_warning.detect_gji(
                self.warn_state_dependent_mode_keys,
                gji_stamp,
                keymap.as_ref(),
                learned.as_deref(),
                [left, right],
                &self.passthrough_thumb_mode_keys,
            )
        } else {
            // 予測経路・役割判定経路と同じ構築関数を経由する（別々にレジストリを読んで解釈を
            // ずらさないため、ADR-199 T17 opusレビュー M3）。
            let (keymap, bits) =
                crate::msime_key_assignment::read_key_effect_keymap_native_with_reassignment_bits();
            let learned = self.learned_cells_for_warning(now_ms, Some(&keymap));
            self.state_dependent_key_warning.detect_msime(
                self.warn_state_dependent_mode_keys,
                bits,
                Some(&keymap),
                learned.as_deref(),
                [left, right],
                &self.passthrough_thumb_mode_keys,
            )
        };
        for warning in &warnings {
            tracing::warn!(
                "[state-dependent-mode-key] kind={:?} keys={:?}: {}",
                warning.kind,
                warning.keys,
                warning.message
            );
        }
        let requests = self
            .state_dependent_key_warning_dialog
            .select(google_ime, gji_stamp, &warnings);
        for request in requests {
            use crate::state::state_dependent_key_warning::WarningDialogAction;
            let question = match request.action {
                WarningDialogAction::OpenAwaseSettings => {
                    "いますぐawaseの設定を開いて、冪等なIME ON/OFF設定へ置き換えますか？"
                }
                WarningDialogAction::OpenMsImeSettings => {
                    "いますぐWindowsのIME設定を開いて、競合するキー割り当てを解除しますか？"
                }
            };
            let text = format!(
                "{}\n\n対象キー: {:?}\n\n{question}",
                request.warning.message, request.warning.keys,
            );
            let on_yes = move || match request.action {
                WarningDialogAction::OpenAwaseSettings => {
                    crate::app::launch_settings_with_args(["--adr192-mode-key-warning".to_owned()]);
                }
                WarningDialogAction::OpenMsImeSettings => {
                    crate::msime_key_assignment::open_ime_settings();
                }
            };
            crate::msime_key_assignment::spawn_yes_dialog(
                "awase - 状態依存のIMEモードキー",
                text,
                on_yes,
            );
        }
    }

    /// `config.general.half_width_alnum_toggle` を反映する。起動時と reload 時の
    /// 両方から呼び、BUG-25 GJI entry の kill switch を即時に効かせる。
    pub(crate) fn set_half_width_alnum_toggle_policy(
        &mut self,
        policy: awase::config::HalfWidthAlnumTogglePolicy,
    ) {
        self.platform_state.gate.half_width_alnum.set_policy(policy);
    }

    /// 専用Fnキー変換モード（`muhenkan_solo_tap_dedicated_fn_key`、ADR-091
    /// §D3.2、config.toml による手動設定のみ）を反映する。起動時
    /// （`bootstrap.rs`）と `apply_config_update`（reload 時）の両方から呼ぶ。
    pub(crate) fn set_muhenkan_dedicated_fn_key_config(&mut self, vk: Option<VkCode>) {
        self.engine.set_muhenkan_solo_tap_dedicated_fn_key(vk);
        self.muhenkan_dedicated_fn_key_vk = vk;
        if let Some(vk) = vk {
            self.platform_state
                .keymap
                .active_keymaps
                .warn_if_vk_conflicts(
                    vk,
                    "muhenkan_solo_tap_dedicated_fn_key",
                    crate::keymap::KeymapConflictLevel::Warn,
                );
        }
    }

    /// `msime_key_assignment::check_and_warn`の警告デデュープ値を`packed`に
    /// 更新し、更新前の値を返す（ADR-164フェーズ2、旧`LAST_WARNED`の
    /// swap操作に対応）。
    pub(crate) fn swap_msime_key_assignment_warned(&mut self, packed: u8) -> Option<u8> {
        self.msime_key_assignment_warned.replace(packed)
    }

    /// `msime_key_assignment::check_and_warn`の警告デデュープ値を未警告へ
    /// 戻す（ADR-164フェーズ2、旧`LAST_WARNED`のstore(NOT_WARNED)に対応）。
    pub(crate) fn reset_msime_key_assignment_warned(&mut self) {
        self.msime_key_assignment_warned = None;
    }

    /// `muhenkan_solo_tap_dedicated_fn_key`（config.tomlによる手動設定）が
    /// 有効かどうか。BUG-115: 無変換が親指キーとして設定されておりGJI側の
    /// IME意味論も検出された場合、これが有効だと無変換側の
    /// delegate-to-open-axisが優先順位で黙って死ぬ
    /// （`resolve_pending_thumb_as_single`、専用Fnキーが最優先）ため、
    /// 警告を出すかどうかの判定に使う。
    #[must_use]
    pub(crate) const fn muhenkan_dedicated_fn_key_configured(&self) -> bool {
        self.muhenkan_dedicated_fn_key_vk.is_some()
    }

    /// `config.general.left_thumb_key`/`right_thumb_key` 由来のキャッシュを
    /// 更新する（ADR-092 決定D Step4a）。起動時（`bootstrap.rs`）と
    /// `apply_config_update`（reload 時）の両方から呼ぶ。
    pub(crate) fn set_space_is_thumb_key(&mut self, space_is_thumb_key: bool) {
        self.space_is_thumb_key = space_is_thumb_key;
    }

    /// `sync_ime_toggle_auto_detect`（`message_handlers.rs`）が Shift+Space の
    /// 自動検出を反映すべきかの判定に使う。
    #[must_use]
    pub(crate) const fn space_is_thumb_key(&self) -> bool {
        self.space_is_thumb_key
    }

    /// トレイアイコンの HWND を返す。
    pub(crate) const fn tray_hwnd(&self) -> windows::Win32::Foundation::HWND {
        self.platform.tray.hwnd()
    }

    /// ウィンドウフォーカス変更イベントを処理する（`win_event_proc` から呼ぶ）。
    pub(crate) fn on_window_focus_event(
        &mut self,
        hwnd_id: crate::state::ime_event::HwndId,
        now: std::time::Instant,
    ) {
        self.platform_state
            .ime
            .try_set_focus_transition_barrier(hwnd_id, now);

        // デバウンスタイマー（~50ms）が完了する前にキーが来た場合でも injection_mode が
        // 正しくなるよう、フォーカス変更直後に新ウィンドウの class/pid から同期更新する。
        // WezTerm(ForceTsf) → Chrome 等の遷移でも hint を新ウィンドウから引くため stale にならない。
        {
            let hwnd = hwnd_id.to_hwnd();
            let class_name = crate::focus::classify::get_class_name_string(hwnd);
            if !class_name.is_empty() {
                let pid = crate::focus::classify::get_window_process_id(hwnd);
                let new_app_kind = crate::observer::focus_observer::detect_app_kind(&class_name);
                let hint = self.platform.injection_hint_for(pid, &class_name);
                let new_mode = crate::output::types::InjectionMode::from((hint, new_app_kind));
                self.platform.update_injection_mode(new_mode);
                tracing::debug!(
                    "[focus-sync] hwnd=0x{:X} class={class_name:?} \
                     app_kind={new_app_kind:?} hint={hint:?} → mode={new_mode:?}",
                    hwnd_id.0
                );

                // BUG-37: この `EVENT_OBJECT_FOCUS` 経路（Ctrl+T 新規タブ等、同一プロセス内の
                // フォーカス移動を含む）は belief（desired_open/effective_open）を一切触らない。
                // 判定ロジックは `should_reprime_on_lightweight_focus_sync` のドキュメント参照
                // （唯一の訂正チャネルである物理 IME キー押下が shadow-toggle の no-op に
                // 握り潰される問題を、真のフォーカス変更と同じ再プライム機構で補う）。
                // cold mark 自体は次に実際に入力するまで何も送信しない遅延フラグなので、
                // Chrome の連続フォーカスイベントで何度呼ばれても実害はない
                // （詳細は docs/known-bugs.md BUG-37）。
                // issue #136 / BUG-90 決定4: `self.platform.focus`（`FocusTracker`）
                // から正規ルートで取得する（プロセスグローバルは
                // `ime.rs::read_ime_state_fast`（`self` を持たない）専用）。
                // `AppImeProfile::resolve` が relay_apps 空の場合に
                // `get_process_name`（Win32 ハンドルを開くコストがかかる）の
                // 呼び出し自体を省略する（`ime.rs::read_ime_state_fast` と
                // 共通化、`/code-review` 指摘）。
                let relay_apps = self.platform.focus.input_relay_apps();
                let profile =
                    crate::focus::classify::AppImeProfile::resolve(&class_name, relay_apps, || {
                        crate::focus::classify::get_process_name(pid)
                    });
                if profile.should_reprime_on_lightweight_focus_sync(
                    &class_name,
                    self.platform_state.ime.effective_open(),
                ) {
                    tracing::debug!(
                        "[focus-sync] belief=ON かつ実状態を問い合わせられないプロファイル \
                         (profile={profile:?}) → 次の入力で再プライムするため cold mark"
                    );
                    self.platform.mark_composition_cold_focus_change();
                }
            }
        }

        self.platform.on_focus_change_tsf();
        self.platform.timer.set(
            crate::TIMER_TSF_GATE,
            std::time::Duration::from_millis(crate::tsf::WARMUP_TIMEOUT_MS),
        );
        let debounce_ms = u64::from(self.platform_state.focus.focus_debounce_ms);
        self.schedule_ime_refresh(debounce_ms);
    }

    /// フックウォッチドッグタイマーを起動する（3 秒）。
    pub(crate) fn start_hook_watchdog(&mut self) {
        self.platform.timer.set(
            crate::TIMER_HOOK_WATCHDOG,
            std::time::Duration::from_secs(3),
        );
    }

    /// `bootstrap.rs`が起動時に`install_hook()`直後へ1回だけ呼ぶ。以後は
    /// `reinstall_keyboard_hook_for_watchdog`が差し替える。
    pub(crate) fn set_hook_guard(&mut self, guard: crate::hook::HookGuard) {
        self.hook_guard = Some(guard);
    }

    /// `bootstrap.rs::run`終了時（`run_message_loop`/`cleanup`の後）に呼び、
    /// フックを明示的に解除する（旧来の`drop(hook_guard)`と同じタイミング）。
    pub(crate) fn drop_hook_guard(&mut self) {
        self.hook_guard = None;
    }

    /// `[diagnostics] hook_self_heal`を反映する（起動時`bootstrap.rs`、
    /// リロード時`apply_config_update`の両方から呼ぶ）。
    pub(crate) const fn set_hook_self_heal_enabled(&mut self, enabled: bool) {
        self.hook_self_heal_enabled = enabled;
    }

    /// `WM_WTSSESSION_CHANGE`（`handle_wts_session_change`）から呼び、セッション
    /// ロック状態を更新する（issue #165 自己修復 F2ガード用）。
    pub(crate) const fn set_session_locked(&mut self, locked: bool) {
        self.session_locked = locked;
    }

    /// hook watchdog が stale_ms<=5000（＝フック生存を確認できた）と判定した
    /// tick（`message_handlers.rs`の`TIMER_HOOK_WATCHDOG`分岐、else側）で
    /// 呼ぶ。`state::hook_watchdog::RECOVERY_CONFIRM_TICKS`連続でこれが
    /// 呼ばれて初めて実際に`note_hook_watchdog_recovered`（バックオフ/
    /// thrash履歴のリセット）へ進む。
    ///
    /// PR #349コードレビュー指摘: 以前は`note_hook_watchdog_recovered`を
    /// stale_ms<=5000の**最初の1tick**で即座に呼んでいたため、他プロセスの
    /// フックが打鍵を断続的にしか握りつぶさない「flicker」型のstarvation
    /// では、1回フックが生き返っただけで段階的バックオフが丸ごと0へ戻り、
    /// このケースのために存在するはずの段階的抑制が機能しなかった。
    pub(crate) fn note_hook_watchdog_tick_alive(&mut self) {
        self.hook_watchdog_consecutive_alive_ticks =
            self.hook_watchdog_consecutive_alive_ticks.saturating_add(1);
        if self.hook_watchdog_consecutive_alive_ticks
            >= crate::state::hook_watchdog::RECOVERY_CONFIRM_TICKS
        {
            self.note_hook_watchdog_recovered();
        }
    }

    /// hook watchdog が stale_ms>5000（＝フックが生存確認できていない）と
    /// 判定したtickで呼ぶ。「連続してフック生存を確認できたtick数」の
    /// カウント（[`note_hook_watchdog_tick_alive`]参照）を途切れさせる。
    pub(crate) const fn note_hook_watchdog_tick_not_alive(&mut self) {
        self.hook_watchdog_consecutive_alive_ticks = 0;
    }

    /// 次に hook_starved を検知したときは新しい episode として扱われ、
    /// バックオフ/thrash履歴の起点がリセットされる（opus round2 B1(ii)、
    /// 旧`hook_watchdog_episode_attempted`ラッチの後継）。
    /// [`note_hook_watchdog_tick_alive`]経由でのみ呼ぶこと（直接呼ぶと
    /// flicker耐性が失われる）。
    const fn note_hook_watchdog_recovered(&mut self) {
        self.hook_watchdog_confirmed_attempt_count = 0;
        self.hook_watchdog_next_retry_at_ms = None;
    }

    /// issue #165（hook_starved）の自己修復トリガー判定（opus-adversarial-consult
    /// round1・round2 指摘対応版）。
    ///
    /// `message_handlers.rs`のhook_starved分岐から、`stale_ms>5000 &&
    /// os_idle_ms<5000`成立時に呼ぶ。環境情報（昇格/セッションロック/
    /// secure desktop/relayソフト/フック有無/バックオフ/thrash上限）を集めて
    /// `state::hook_watchdog::decide`（純粋関数）へ渡し、`SendCanary`が返った
    /// 場合のみ`send_hook_watchdog_canary`を呼ぶ（round2 B1(i)、即座の
    /// 再インストールではなくカナリア確認を経る）。それ以外のバリアントは
    /// 全て「何もしない」を意味し、呼び出し元がスキップ理由のログに使う。
    pub(crate) fn evaluate_hook_watchdog(
        &mut self,
        now_ms: u64,
    ) -> crate::state::hook_watchdog::HookWatchdogAction {
        let is_elevated_foreground =
            !crate::is_elevated() && crate::hook::foreground_window_is_elevated();
        let is_secure_desktop = crate::hook::is_secure_desktop_active();
        let process_name = self.platform.focus.process_name();
        let is_relay_or_remap_foreground = self.platform.focus.is_app_disabled()
            || crate::state::app_suppression::matches_disabled_app(
                self.platform.focus.input_relay_apps(),
                process_name,
            )
            || crate::app::is_relay_or_remap_software_process(process_name);
        let reinstalls_in_window = crate::state::hook_watchdog::count_within_window(
            &self.hook_watchdog_reinstall_history_ms,
            now_ms,
            crate::state::hook_watchdog::THRASH_WINDOW_MS,
        );
        let action = crate::state::hook_watchdog::decide(
            self.hook_self_heal_enabled,
            is_elevated_foreground,
            self.session_locked,
            is_secure_desktop,
            is_relay_or_remap_foreground,
            self.hook_guard.is_some(),
            now_ms,
            self.hook_watchdog_next_retry_at_ms,
            reinstalls_in_window,
            crate::state::hook_watchdog::THRASH_LIMIT,
        );
        match action {
            crate::state::hook_watchdog::HookWatchdogAction::SendCanary => {
                self.send_hook_watchdog_canary(now_ms);
            }
            crate::state::hook_watchdog::HookWatchdogAction::ReinstallWithoutCanary => {
                // opus round1 M1: フック不在時はカナリアを経由しない
                // （確認相手が無く、Ctrl漏れ/ジグラー化を招くため）。
                // PR #349コードレビュー指摘: `decide`はこの経路をバックオフ/
                // thrash上限の対象外としているため、`record_thrash=false`で
                // 履歴を汚染しない。
                self.reinstall_keyboard_hook_for_watchdog(now_ms, false);
            }
            _ => {}
        }
        action
    }

    /// issue #165 自己修復 round2 B1(i): カナリア（自己注入 Ctrl down+up）を
    /// 送信し、`state::hook_watchdog::CANARY_CONFIRM_MS`後に
    /// `confirm_hook_watchdog_canary`で結果を判定できるよう一発タイマーを
    /// 起動する。
    ///
    /// 既に確認待ち（前回のカナリアがまだ`TIMER_HOOK_WATCHDOG_CANARY_CHECK`を
    /// 待っている）なら二重送信・二重タイマーを避けるため何もしない。3秒周期の
    /// watchdog tickに対し確認は`CANARY_CONFIRM_MS`（既定200ms）で完了する
    /// はずなので、通常はここに到達しない防御的ガード。
    fn send_hook_watchdog_canary(&mut self, now_ms: u64) {
        if let Some(sent_at_ms) = self.hook_watchdog_canary_sent_at_ms {
            // opus round1 m2: `SetTimer`（`TIMER_HOOK_WATCHDOG_CANARY_CHECK`）が
            // 失敗する（戻り値未検査）、またはUSERオブジェクト枯渇等で
            // `WM_TIMER`自体が届かないと、`confirm_hook_watchdog_canary`が
            // 一度も呼ばれず確認待ちフラグが永久に残り、以後の自己修復が
            // 完全に止まる。`CANARY_CONFIRM_MS`の10倍を過ぎてもまだ
            // 確認待ちのままなら、確認処理が失われたとみなして古い状態を
            // 破棄し、新しいカナリアを送り直す。
            let confirm_lost_threshold_ms =
                crate::state::hook_watchdog::CANARY_CONFIRM_MS.saturating_mul(10);
            if now_ms.saturating_sub(sent_at_ms) < confirm_lost_threshold_ms {
                tracing::debug!("[hook-watchdog] カナリア確認待ち中のため送信をスキップ");
                return;
            }
            tracing::warn!(
                "[hook-watchdog] カナリア確認が{}ms以上届いていない（確認タイマー \
                 消失の疑い）、状態を破棄して送り直します",
                now_ms.saturating_sub(sent_at_ms)
            );
        }
        self.hook_watchdog_canary_sent_at_ms = Some(now_ms);
        // opus round1 B1: 基準値は送信「前」の`hook_alive_tick_ms()`
        // （この分岐に入る時点で既に5秒以上古い値）。送信「時刻」
        // （`now_ms`）を基準にすると`GetTickCount64`の分解能（約15.6ms）に
        // 負けて誤検知する。
        self.hook_watchdog_canary_baseline_alive_ms = Some(crate::hook::hook_alive_tick_ms());
        crate::hook::send_hook_watchdog_canary();
        self.platform.timer.set(
            crate::TIMER_HOOK_WATCHDOG_CANARY_CHECK,
            std::time::Duration::from_millis(crate::state::hook_watchdog::CANARY_CONFIRM_MS),
        );
    }

    /// issue #165 自己修復 round2 B1(i): `TIMER_HOOK_WATCHDOG_CANARY_CHECK`
    /// 発火時に`message_handlers.rs`から呼ぶ。カナリア送信後に
    /// `hook::hook_alive_tick_ms()`が進んでいなければ真の hook_starved と
    /// 確定し、実際の再インストールへ進む。進んでいれば「hookは生きている
    /// がユーザーが実キーを打っていないだけ」の偽陽性と分かり、
    /// バックオフ/thrash履歴を一切消費せずスキップする。
    pub(crate) fn confirm_hook_watchdog_canary(&mut self, now_ms: u64) {
        let Some(canary_sent_at_ms) = self.hook_watchdog_canary_sent_at_ms.take() else {
            // 通常は起こらない（確認タイマーはカナリア送信時にしか起動しない）。
            return;
        };
        // `hook_watchdog_canary_sent_at_ms`と常に同時にSome/Noneが揃う
        // （どちらも`send_hook_watchdog_canary`でのみSomeになる）。
        let baseline_hook_alive_ms = self
            .hook_watchdog_canary_baseline_alive_ms
            .take()
            .unwrap_or(canary_sent_at_ms);
        let hook_alive_tick_ms_after = crate::hook::hook_alive_tick_ms();
        if crate::state::hook_watchdog::canary_confirmed_starved(
            hook_alive_tick_ms_after,
            baseline_hook_alive_ms,
        ) {
            tracing::warn!(
                "[hook-watchdog] カナリア({}ms前送信)が届かず確認 → 真の \
                 hook_starved と判定、再インストールします",
                now_ms.saturating_sub(canary_sent_at_ms)
            );
            self.reinstall_keyboard_hook_for_watchdog(now_ms, true);
        } else {
            tracing::debug!(
                "[hook-watchdog] カナリアが届いた（フックは生存中）→ \
                 誤検知として再インストールをスキップ"
            );
        }
    }

    /// issue #165（hook_starved）の自己修復本体。
    ///
    /// `confirm_hook_watchdog_canary`がカナリア不着＝本物のstarvationと
    /// 確定した場合のみ呼ばれる（round2 B1: 誤検知ではepisodeラッチ/
    /// thrash履歴を一切消費しない設計）。`WH_KEYBOARD_LL`はLIFO（最後に
    /// 登録したフックが最初に呼ばれる）で配送されるため、旧フックを
    /// `UnhookWindowsHookEx`してから新しく`SetWindowsHookExW`し直すと、
    /// このタイミング以降にチェーンへ割り込んでいた他プロセスのフックより
    /// 手前（先頭）に戻れる。失われた打鍵は戻せないが、同じ停止が続くのを
    /// 防ぐ。
    ///
    /// `install_hook()`が失敗した場合はフック無しの状態になりうる。この場合
    /// `state::hook_watchdog::decide`の`hook_guard_present=false`分岐が
    /// バックオフ/thrash上限をバイパスするため、次のwatchdog tick（3秒後）で
    /// 即座に再試行される（opus round2 M5: 旧実装はエピソードラッチが
    /// 立ったまま二度とフックが来ないため永久にリトライされなかった）。
    /// ここでpanicはしない——フック関連の失敗で常駐アプリを丸ごと落とすのは
    /// 実害が大きすぎる。
    ///
    /// `record_thrash`: バックオフ/thrash履歴を更新するか。`true`は
    /// `confirm_hook_watchdog_canary`（カナリア確認済み、`decide`の
    /// `hook_guard_present=true`分岐がこの履歴を見て次回の
    /// SkipBackoffPending/SkipThrashLimitを判定する）から呼ばれた場合。
    /// `false`は`ReinstallWithoutCanary`（フック不在時の直接再試行、`decide`は
    /// `hook_guard_present=false`の間バックオフ/thrash上限を無条件バイパス
    /// する設計）から呼ばれた場合——PR #349コードレビュー指摘: 以前は
    /// この経路でも無条件に履歴を積んでいたため、フック不在が続いた後に
    /// 復旧しても、フック不在中に積み上がった履歴のせいで直後の本物の
    /// starvationがSkipThrashLimit/SkipBackoffPendingで最長1時間直らない
    /// 「予算の汚染」が起きていた。
    fn reinstall_keyboard_hook_for_watchdog(&mut self, now_ms: u64, record_thrash: bool) {
        if record_thrash {
            // バックオフ/thrash履歴は「カナリア確認済みで実際に試行した」事実
            // そのものを記録する（install_hook()の成否に関わらず）。
            let backoff_ms = crate::state::hook_watchdog::backoff_delay_ms(
                self.hook_watchdog_confirmed_attempt_count,
            );
            self.hook_watchdog_confirmed_attempt_count =
                self.hook_watchdog_confirmed_attempt_count.saturating_add(1);
            self.hook_watchdog_next_retry_at_ms = Some(now_ms.saturating_add(backoff_ms));
            self.hook_watchdog_reinstall_history_ms.push(now_ms);
            // 履歴は thrash 判定用の直近分だけで十分。THRASH_WINDOW_MS より古い
            // エントリを刈り取り、無期限に肥大化しないようにする。
            self.hook_watchdog_reinstall_history_ms.retain(|&t| {
                now_ms.saturating_sub(t) < crate::state::hook_watchdog::THRASH_WINDOW_MS
            });
        }

        // 旧ガードをここで明示的にdropしてから新規installする
        // （両方生存する瞬間を作らない。`WM_QUIT`→スレッドjoin→
        // `UnhookWindowsHookEx`が完了してから次のSetWindowsHookExWへ進む）。
        self.hook_guard = None;
        match crate::hook::install_hook() {
            Ok(guard) => {
                self.hook_guard = Some(guard);
                // issue #165 自己修復 F4（round2 M2で根拠づけを訂正）: この
                // 関数はカナリアで本物のstarvationと確認できたときにしか
                // 呼ばれないため（誤検知では呼ばれない）、握りつぶされていた
                // 間のKeyUp消失で物理キーラッチ（Ctrl/Shift）がスタックした
                // まま残る（BUG-78/BUG-48と同型）前提が実際に成り立つ。
                // `reset_physical_key_state()`（全256 VK無条件クリア）は
                // 誤検知時にも呼ばれていた旧実装では「押されたままのCtrlが
                // stateだけfalseになりCtrl+Cがローマ字文字と合成される」
                // 新しい事故を生んでいたため、Ctrl/Shiftのみを対象にする
                // narrow版に切り替えた。
                crate::hook::clear_hook_latches_for_watchdog_reinstall();
                self.platform_state.keymap.keymap_latch.release_all();
                tracing::warn!(
                    "[hook-watchdog] キーボードフックを再インストールしました（issue #165 自己修復）"
                );
            }
            Err(e) => {
                tracing::error!(
                    "[hook-watchdog] キーボードフックの再インストールに失敗しました: {e}"
                );
            }
        }
    }

    /// UIA ワーカースレッドへの送信チャネルを登録する。
    pub(crate) fn set_uia_sender(
        &mut self,
        tx: std::sync::mpsc::Sender<crate::focus::uia::SendableHwnd>,
    ) {
        self.platform.set_uia_sender(tx);
    }

    /// システムトレイのバルーン通知を表示する。
    pub(crate) fn show_tray_balloon(&mut self, title: &str, text: &str) {
        self.platform.tray.show_balloon(title, text);
    }

    /// 診断画面が必要とする状態を一括スナップショットとして返す。
    pub(crate) fn diagnostic_snapshot(&self) -> RuntimeDiagnosticSnapshot {
        let (focus_pid, focus_class) = if self.platform.focus.is_focused() {
            (
                self.platform.focus.pid(),
                self.platform.focus.class_name().to_owned(),
            )
        } else {
            (0, String::new())
        };
        RuntimeDiagnosticSnapshot {
            focus_pid,
            focus_class,
            shadow_ime_on: self.platform_state.ime.effective_open(),
            shadow_is_romaji: self.platform_state.ime.input_mode().is_romaji_capable(),
            shadow_is_japanese: self.platform_state.ime.belief.is_japanese_ime(),
            last_focus_change_ms: self.platform_state.focus.last_focus_change_ms,
            last_hook_activity_ms: self.platform_state.gate.last_hook_activity_ms,
            app_profile: format!("{:?}", self.platform.current_app_profile()),
        }
    }

    /// 設定リロード時に Runtime の全パラメータを一括更新する。
    ///
    /// FSM パラメータ・出力モード・同期キー・特殊キーコンボ・
    /// アプリオーバーライドをアトミックに適用する。
    pub(crate) fn apply_config_update(
        &mut self,
        config: &ValidatedConfig,
        special_keys: SpecialKeyCombos,
        sync_toggle: Vec<VkCode>,
        sync_on: Vec<VkCode>,
        sync_off: Vec<VkCode>,
    ) -> Vec<String> {
        // 解決できなかった項目の警告（ADR-201 決定2）。呼び出し元（`reload_config`）が診断へ流す。
        let mut warnings: Vec<String> = Vec::new();
        let ctx = self.build_ctx();
        let forced_open_actions = thumb_forced_open_actions(&special_keys);
        self.engine
            .set_thumb_forced_open_actions(forced_open_actions.0, forced_open_actions.1);
        self.engine.set_thumb_role_open_actions(None, None);
        let _ = self.engine.on_command(
            EngineCommand::UpdateFsmParams {
                threshold_ms: config.general.simultaneous_threshold_ms,
                confirm_mode: config.general.confirm_mode,
                speculative_delay_ms: config.general.speculative_delay_ms,
                timing_margin_percent: config.general.timing_margin_percent,
                min_overlap_margin_percent: config.general.min_overlap_margin_percent,
            },
            &ctx,
        );
        self.platform_state.focus.focus_debounce_ms = config.general.focus_debounce_ms;
        self.platform_state.focus.ime_poll_interval_ms = config.general.ime_poll_interval_ms;
        self.use_learned_keymap_table = config.general.use_learned_keymap_table;
        self.predict_henkan_open_in_unreadable_windows =
            config.general.predict_henkan_open_in_unreadable_windows;
        self.set_keyboard_model(config.general.keyboard_model);
        self.set_update_check_enabled(config.general.update_check);
        self.set_warn_state_dependent_mode_keys(config.general.warn_state_dependent_mode_keys);
        self.set_hook_self_heal_enabled(config.diagnostics.hook_self_heal);
        self.set_half_width_alnum_toggle_policy(config.general.half_width_alnum_toggle);
        crate::hook::set_swallow_alt_kana_mode_switch(
            config.general.swallow_alt_kana_input_method_switch,
        );
        self.focus_tracker.sync_toggle_keys = sync_toggle;
        self.focus_tracker.sync_on_keys = sync_on;
        self.focus_tracker.sync_off_keys = sync_off;
        let _ = self.engine.on_command(
            EngineCommand::ReloadKeys {
                special: special_keys,
            },
            &ctx,
        );
        self.platform
            .focus
            .reset_overrides(crate::focus::classifier::ForceOverrides::new(
                config.app_overrides.clone(),
            ));
        self.platform.focus.cache_reset();
        // disable_apps がリロードで変わった場合に備え、現在のフォーカス先で
        // 無効化状態を再評価する（BUG-78 対策の一部）。
        if self.platform.focus.is_focused() {
            let pid = self.platform.focus.pid();
            self.apply_app_disable_transition(pid, false);
        }
        if let (Some((left, left_alt_impersonates)), Some((right, right_alt_impersonates))) = (
            crate::hook::resolve_thumb_key(&config.general.left_thumb_key),
            crate::hook::resolve_thumb_key(&config.general.right_thumb_key),
        ) {
            crate::hook::set_thumb_vk_codes(left, right);
            crate::hook::set_alt_impersonation_enabled(
                left_alt_impersonates,
                right_alt_impersonates,
            );
            let space_thumb_vk = [left, right]
                .into_iter()
                .find(|&vk| vk == crate::vk::VK_SPACE);
            self.engine.set_space_thumb_config(
                space_thumb_vk,
                TextKeyConfig {
                    ignore_composing_guard: config.general.space_thumb_ignore_composing_guard,
                    shift_literal: config.general.space_thumb_shift_literal,
                },
            );
            let muhenkan_vk = [left, right]
                .into_iter()
                .find(|&vk| vk == crate::vk::VK_NONCONVERT);
            let henkan_vk = [left, right]
                .into_iter()
                .find(|&vk| vk == crate::vk::VK_CONVERT);
            self.engine.set_thumb_key_solo_tap_config(
                muhenkan_vk,
                ModeKeyConfig::from_legacy_bools(
                    config.general.muhenkan_solo_tap_ignore_composing_guard,
                    config.general.muhenkan_solo_tap_always_suppress,
                ),
                henkan_vk,
                ModeKeyConfig::from_legacy_bools(
                    config.general.henkan_solo_tap_ignore_composing_guard,
                    config.general.henkan_solo_tap_always_suppress,
                ),
            );
            let manual_fn_key = config.general.muhenkan_solo_tap_dedicated_fn_key.as_deref();
            let (fn_key, fn_key_warning) = resolve_dedicated_fn_key(manual_fn_key);
            warnings.extend(fn_key_warning);
            self.set_muhenkan_dedicated_fn_key_config(fn_key);
            self.set_passthrough_thumb_mode_keys(&config.general);
            self.set_space_is_thumb_key(crate::state::alt_impersonation::is_thumb_key_vk(
                &config.general.left_thumb_key,
                &config.general.right_thumb_key,
                crate::vk::VK_SPACE,
            ));
            let enter_thumb_vk = [left, right]
                .into_iter()
                .find(|&vk| vk == crate::vk::VK_RETURN);
            self.engine.set_enter_thumb_config(
                enter_thumb_vk,
                TextKeyConfig {
                    ignore_composing_guard: config.general.enter_thumb_ignore_composing_guard,
                    shift_literal: config.general.enter_thumb_shift_literal,
                },
            );
            self.engine
                .set_thumb_shift_faces_enabled(crate::app::thumb_shift_faces_enabled_for(
                    left, right,
                ));
            tracing::info!(
                "Thumb keys updated: left={:?}, right={:?}",
                config.general.left_thumb_key,
                config.general.right_thumb_key,
            );
        } else {
            tracing::warn!(
                "Invalid thumb key names: left={:?}, right={:?}",
                config.general.left_thumb_key,
                config.general.right_thumb_key,
            );
        }
        // [[keymap]] の再構築（ADR-114 決定8）。`resolve_thumb_key` の if-let
        // ブロックの**外・後**に置くこと——ブロック内に置くと上の `else`
        // （"Invalid thumb key names"）に落ちたときに親指 vk が確定せず
        // reload が丸ごとスキップされる。`hook::thumb_vk_codes()` は
        // if-let の成否に関わらず現在キャッシュされている値（bootstrap
        // または直近の成功した reload の値）を返すため、ここで安全に使える。
        let (left_thumb_vk, right_thumb_vk) = crate::hook::thumb_vk_codes();
        let (all_keymaps, keymap_warnings) =
            crate::keymap::KeymapTable::new(&config.keymaps, left_thumb_vk, right_thumb_vk);
        warnings.extend(keymap_warnings);
        self.all_keymaps = all_keymaps;
        self.recompute_active_keymaps();
        tracing::info!(
            "Config applied: threshold={}ms, speculative_delay={}ms",
            config.general.simultaneous_threshold_ms,
            config.general.speculative_delay_ms,
        );
        warnings
    }

    /// `active_keymaps` を `all_keymaps` から再計算する（ADR-114 決定8）。
    ///
    /// フォーカス変更時（`focus_tracking.rs::enter_focus_scope`）と
    /// `reload_config` 経路（`apply_config_update`）の両方から呼ぶ、
    /// 唯一の書き込み点。書き込み点を2つに増やさない（`enter_focus_scope`
    /// が過去に同種の重複を統合した経緯と同じ理由）。
    fn recompute_active_keymaps(&mut self) {
        let process_name = self.platform.focus.process_name().to_owned();
        self.platform_state.keymap.active_keymaps = self.all_keymaps.filter_active(&process_name);
        tracing::debug!(
            "[keymap] active rules recomputed: {} rule(s) for process={:?}",
            self.platform_state.keymap.active_keymaps.len(),
            process_name,
        );
        // 専用Fnキー（実行時に確定する vk）との衝突も、フォーカス変更・reload の
        // たびに再チェックする（`warn_if_vk_conflicts` の呼び出しを両 setter
        // だけに限ると、setter 呼び出し時点の active_keymaps でしか判定できず、
        // フォーカス変更や reload で新しく衝突するルールが有効になった場合を
        // 見逃す、ADR-114 実装レビュー指摘）。
        if let Some(vk) = self.muhenkan_dedicated_fn_key_vk {
            self.platform_state
                .keymap
                .active_keymaps
                .warn_if_vk_conflicts(
                    vk,
                    "muhenkan_solo_tap_dedicated_fn_key",
                    crate::keymap::KeymapConflictLevel::Debug,
                );
        }
    }

    /// n-gram モデルをエンジンに適用する。
    pub(crate) fn set_ngram_model(&mut self, model: NgramModel) {
        let ctx = self.build_ctx();
        let _ = self
            .engine
            .on_command(EngineCommand::SetNgramModel(model), &ctx);
    }

    /// Output が積んだ `RuntimeRequest` を drain して処理する。
    ///
    /// キー処理境界（`WM_EXECUTE_EFFECTS` / `WM_DRAIN_OUTPUT_QUEUE` 末尾）で呼ぶ。
    ///
    /// `Output` はキー注入中に `with_app` を再入させられないため、IME リフレッシュ・
    /// TSF プローブ起動などの Runtime 操作を `RuntimeOutbox` に積んでおき、
    /// ここで一括実行する（H-4-b: `StartTsfProbe` が Chrome cold パスから積まれる）。
    pub(crate) fn drain_runtime_requests(&mut self) {
        use crate::runtime::outbox::RuntimeRequest;
        let requests = self.platform.output.take_pending_requests();
        if requests.is_empty() {
            return;
        }
        tracing::debug!("[runtime-outbox] {} request(s) を drain", requests.len());
        for request in requests {
            match request {
                RuntimeRequest::StartTsfProbe => {
                    tracing::debug!("[runtime-outbox] StartTsfProbe → pending TSF timer 適用");
                    if let Some(cmd) = self.platform.output.pending_tsf_timer() {
                        self.platform.apply_timer_command(cmd);
                    }
                }
            }
        }
    }

    /// パニックリセット: IME 関連キー連打で発動する緊急リセット。
    ///
    /// エンジン状態・IME・修飾キー・フック・キャッシュをすべて初期状態に戻す。
    /// メッセージループ上で呼ぶこと（ブロッキング OK）。
    #[allow(unsafe_code)] // cancel_ime_composition() が Win32 IMM API を呼ぶ
    pub fn panic_reset(&mut self) {
        tracing::warn!("Panic reset triggered!");

        // 1. エンジンの保留状態をフラッシュ
        self.invalidate_engine_context(ContextChange::InputLanguageChanged);

        // 2. IME 未確定文字列をキャンセル → OFF → ON
        // SAFETY: `cancel_ime_composition` は Win32 IMM API を呼ぶ unsafe fn。
        //         `panic_reset` はメッセージループ上（メインスレッド）から呼ばれるため安全。
        unsafe { cancel_ime_composition() };
        // OFF → ON を順序保証付きで実行する。`WindowsPlatform::set_ime_open` は
        // 内部で spawn_local して fire-and-forget するため、2 連発で呼ぶと async race で
        // 順序が逆転しうる (true→false の終端で IME OFF のまま残るリスク)。単一の
        // spawn_local タスク内で 2 回 await する形にして OFF → ON を直列化する。
        if self.can_use_imm32_cross_process() {
            win32_async::spawn_local(async {
                let _ = crate::ime::set_ime_open_cross_process_async(false).await;
                let _ = crate::ime::set_ime_open_cross_process_async(true).await;
                // カタカナ・半角カタカナ状態でリセットした場合でもひらがなに戻す
                let _ = crate::ime::set_ime_hiragana_mode_cross_process_async().await;
            });
        }

        // 3. 全修飾キーの KeyUp を送信（スタック解消）
        // send_all_modifier_key_ups() は自己注入 SendInput (INJECTED_MARKER) のため
        // is_self_injected フィルタでフックの PHYSICAL_KEY_STATE 更新まで届かない
        // (ADR-054 由来の隙間、2026-07-09 発見)。OS 側の modifier は解放されるが
        // awase 内部の物理キー shadow は解放されないままだったため、明示的にリセットする。
        send_all_modifier_key_ups();
        crate::hook::reset_physical_key_state();
        // [[keymap]] latch も同じ理由で解放する（ADR-114 決定4「latch
        // 漏れ対策」経路5）。
        self.platform_state.keymap.keymap_latch.release_all();

        // 4. PlatformState を全面リセット
        // panic_reset 直後に refresh_ime_state_cache() が走ると、ここで書いた
        // ime_on=true を stale な observe() 結果が即座に上書きしてしまう。
        // force_on_guard で 1 サイクルだけ保護し、次の検出成功時に自然に解除する。
        let tick_ms = crate::state::TickMs(crate::hook::current_tick_ms());
        self.platform_state.ime.apply_panic_reset(tick_ms);
        // Step 4: chord barrier も clear (旧 ctrl_bypass_hold 相当)
        self.platform_state.ime.clear_input_barrier();
        self.platform_state.gate.sync_key_gate.clear();

        // 6. IME 状態を再取得
        self.refresh_ime_state_cache();

        // 7. バルーン通知
        self.platform
            .tray
            .show_balloon("awase", "状態をリセットしました");
    }
}

/// 全修飾キーの KeyUp を `SendInput` で送信する。
///
/// Shift, Ctrl, Alt, Win の左右それぞれに対して KeyUp を送り、
/// スタックした修飾キー状態を解消する。
fn send_all_modifier_key_ups() {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VIRTUAL_KEY,
    };

    // VK_SHIFT(0x10), VK_CONTROL(0x11), VK_MENU(0x12),
    // VK_LWIN(0x5B), VK_RWIN(0x5C),
    // VK_LSHIFT(0xA0), VK_RSHIFT(0xA1),
    // VK_LCONTROL(0xA2), VK_RCONTROL(0xA3),
    // VK_LMENU(0xA4), VK_RMENU(0xA5)
    use crate::vk::{
        VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU, VK_RCONTROL, VK_RMENU,
        VK_RSHIFT, VK_RWIN, VK_SHIFT,
    };
    const MODIFIER_VKS: [VkCode; 11] = [
        VK_SHIFT,
        VK_CONTROL,
        VK_MENU,
        VK_LWIN,
        VK_RWIN,
        VK_LSHIFT,
        VK_RSHIFT,
        VK_LCONTROL,
        VK_RCONTROL,
        VK_LMENU,
        VK_RMENU,
    ];

    let inputs: Vec<INPUT> = MODIFIER_VKS
        .iter()
        .map(|&vk| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(vk.0),
                    wScan: 0,
                    dwFlags: KEYEVENTF_KEYUP,
                    time: 0,
                    dwExtraInfo: crate::output::INJECTED_MARKER,
                },
            },
        })
        .collect();

    // OutputActiveGuard: SendInput 実行中にユーザーキーが届いた場合、
    // フックが RUNTIME 借用中（panic_reset の with_app 内）で再入しないよう
    // OUTPUT_GATE.active=true で INPUT_DEFER に退避する。
    let _guard = crate::tsf::probe_bridge::OutputActiveGuard::begin();
    let _ = crate::win32::send_input_safe(&inputs);
    tracing::debug!("Sent KeyUp for all modifier keys");
}

/// IME の未確定文字列をキャンセルする。
///
/// # Safety
/// Win32 IMM API (`ImmGetContext`, `ImmNotifyIME`, `ImmReleaseContext`) を呼び出す。
/// メインスレッドから呼ぶこと。
#[allow(unsafe_code)]
unsafe fn cancel_ime_composition() {
    use std::mem::size_of;
    use windows::Win32::UI::Input::Ime::{ImmNotifyIME, NOTIFY_IME_ACTION, NOTIFY_IME_INDEX};
    use windows::Win32::UI::WindowsAndMessaging::{GetGUIThreadInfo, GUITHREADINFO};

    // `GetForegroundWindow()` は外側の CASCADIA_HOSTING_WINDOW_CLASS を返すが、
    // WezTerm などでは実際の IME コンテキストは子ウィンドウ
    // (Windows.UI.Input.InputSite.WindowClass) に紐付いている。
    // `GetGUIThreadInfo(0)` でフォアグラウンドスレッドの hwndFocus を取得することで
    // InputSite HWND を得る。
    let mut info = GUITHREADINFO {
        cbSize: size_of::<GUITHREADINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: `GetGUIThreadInfo` はメインスレッドから呼ぶ安全なクエリ。
    //         tid=0 はフォアグラウンドスレッドを意味する。
    if unsafe { GetGUIThreadInfo(0, &raw mut info) }.is_err() {
        return;
    }
    let hwnd = info.hwndFocus;
    if hwnd.0.is_null() {
        return;
    }
    // SAFETY: `hwnd` は直上で NULL でないことを確認済み。
    //         `ImmContextGuard` は RAII で `ImmReleaseContext` を呼ぶため、
    //         コンテキストリークは発生しない。
    let Some(ctx) = (unsafe { crate::imm::ImmContextGuard::new(hwnd) }) else {
        tracing::debug!(
            "[ctrl-bypass] ImmGetContext returned NULL for hwnd={hwnd:?}, cancel skipped"
        );
        return;
    };
    // NI_COMPOSITIONSTR = 0x15, CPS_CANCEL = 0x04
    // SAFETY: `ctx.himc()` は `ImmContextGuard` が保持する有効な HIMC。
    //         `NI_COMPOSITIONSTR`/`CPS_CANCEL` は未確定文字列キャンセルの標準的な呼び出し。
    let ok = unsafe {
        ImmNotifyIME(
            ctx.himc(),
            NOTIFY_IME_ACTION(0x15),
            NOTIFY_IME_INDEX(0x04),
            0,
        )
    };
    tracing::debug!(
        "[ctrl-bypass] ImmNotifyIME(CPS_CANCEL) hwnd={hwnd:?} → {}",
        ok.as_bool()
    );
}

#[cfg(test)]
mod layout_entry_tests {
    use super::LayoutEntry;
    use awase::scanmap::KeyboardModel;
    use awase::yab::YabLayout;

    fn entry(name: &str) -> LayoutEntry {
        LayoutEntry {
            name: name.to_string(),
            layout: YabLayout::parse("", KeyboardModel::Jis).unwrap(),
        }
    }

    #[test]
    fn resolve_index_matches_by_file_name_with_or_without_yab_suffix() {
        let layouts = [entry("nicola"), entry("my_nicola")];
        assert_eq!(LayoutEntry::resolve_index(&layouts, "my_nicola.yab"), 1);
        assert_eq!(LayoutEntry::resolve_index(&layouts, "my_nicola"), 1);
        assert_eq!(LayoutEntry::resolve_index(&layouts, "nicola.yab"), 0);
    }

    #[test]
    fn resolve_index_prefers_nicola_keytop_when_default_layout_not_found() {
        // BUG-104: 独自レイアウトの読込失敗(存在しない/UTF-8でない等)時、
        // ソート順先頭ではなく nicola_keytop があればそちらへ寄せる。
        let layouts = [entry("nicola"), entry("nicola_keytop"), entry("nicola_us")];
        assert_eq!(
            LayoutEntry::resolve_index(&layouts, "NICOLA＋確定.yab"),
            1,
            "should fall back to nicola_keytop, not sort-order-first nicola"
        );
    }

    #[test]
    fn resolve_index_matches_case_insensitive_extension_and_stem() {
        // /code-review 指摘（PR #131）: default_layout の拡張子/ステムの
        // 大文字小文字がファイル名と食い違っても、実際に読み込めているファイル
        // を「読込失敗」と誤警告してはいけない（Windows のファイルシステムは
        // 大文字小文字を区別しないため）。
        let layouts = [entry("nicola"), entry("my_nicola")];
        assert_eq!(LayoutEntry::resolve_index(&layouts, "nicola.YAB"), 0);
        assert_eq!(LayoutEntry::resolve_index(&layouts, "NICOLA.yab"), 0);
        assert_eq!(LayoutEntry::resolve_index(&layouts, "My_Nicola.YaB"), 1);
    }

    #[test]
    fn strip_yab_extension_is_safe_for_non_ascii_names_without_yab_suffix() {
        // 境界外パニック回避の回帰テスト: ".yab" で終わらないマルチバイト
        // 文字列に対して byte-index の char boundary パニックを起こさないこと。
        assert_eq!(super::strip_yab_extension("設定.toml"), "設定.toml");
        assert_eq!(super::strip_yab_extension("あ"), "あ");
        assert_eq!(
            super::strip_yab_extension("NICOLA＋確定.yab"),
            "NICOLA＋確定"
        );
    }

    #[test]
    fn resolve_index_falls_back_to_first_entry_when_no_name_matches_and_no_keytop() {
        let layouts = [entry("nicola"), entry("my_nicola")];
        assert_eq!(
            LayoutEntry::resolve_index(&layouts, "does_not_exist.yab"),
            0
        );
    }
}
