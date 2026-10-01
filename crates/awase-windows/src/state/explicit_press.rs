//! 明示的な IME キー押下 1 回の「配送」の純粋な決定関数（ADR-208 L0）。
//!
//! # 何のためのモジュールか
//!
//! ADR-208 の保証（INV-L1: 明示キーの非リピート物理 KeyDown 1 回につき、IME へ届く開閉の作用は**ちょうど 1 つ**。
//! 物理キーが届く〈Allow〉か、awase が書く〈write〉かのどちらか一方）を、Win32 抜きで Linux から全列挙できるように、
//! 今ある判断を**合成**して 1 つの純粋関数 [`explicit_press_delivery_with`] にしたもの。新しい判断は足していない
//! （L0 は挙動を変えない切り出しとテスト基盤。現状の穴は反例として全列挙テスト側が数える）。
//!
//! 合成している部品（すべて本番と同じコード）:
//!
//! - 物理キーの配送: [`PhysicalKeyDisposition::plan_core`](super::physical_disposition::PhysicalKeyDisposition::plan_core)
//!   （`runtime/transport.rs::PhysicalKeyDisposition::plan` の本体。`plan` はこれを呼ぶ殻）
//! - shadow toggle の昇格: [`select_shadow_intent`]（`runtime/key_pipeline.rs::kp_stage_shadow_ime_toggle` が呼ぶ）、
//!   `ShadowImeAction::resolve`、`vk::should_upgrade_is_japanese_ime`
//! - Engine の明示 SetOpen の chord フィルタ: [`engine_set_open_filtered_by_chord`]
//!   （`state/platform_state.rs::handle_engine_set_open` が呼ぶ）
//! - 書き込みの gate・授権・機構選択・already-matched 省略: `decide_gate`・`issue_open_warrant`（[`WarrantJudge`] 経由）・
//!   `decide_chain`・`decide_attempt`・`shadow_toggle_demotes_applied`（`state/ime_actuation_decision.rs`）
//!
//! # 現状の順序の再現（循環を崩さない）
//!
//! 本番の `kp_run_inner` は「shadow 昇格 → `plan`（`shadow_toggled` を入力に取る）」の順で、`plan` の結果を shadow 側は
//! 参照しない（ADR-208 D4 が解こうとしている循環）。本関数は**同じ順序**で再現する。L0 では本番側は本関数を呼ばない
//! （呼ぶのは `plan` の殻化で核を共有する範囲まで）。
//!
//! # 授権（`issue_open_warrant`）の差し込み
//!
//! 授権は `IntentStore`・`ObservationStore` に依存する。`ObservationStore` へ観測を入れる口は本番コードから呼ぶことを
//! 禁じられている（`AnyObservation::restored_from_journal`、`architecture_guard`）ため、本モジュールは授権の判定を
//! [`WarrantJudge`] として差し込ませる。テストは合成ストアで本物の `issue_open_warrant` を呼ぶ判定器を渡す
//! （`tests/explicit_press_exhaustive.rs`）。L1 以降で本番が本関数を呼ぶときは、live の `WarrantContext` から
//! 判定する実装を渡す。
//!
//! # モデルの前提（推測を含む。ADR-208 §5(a) と同じ単純化）
//!
//! - 機構チェーンは先頭の機構だけを見る（`Failed` のフォールスルーは機構の失敗であり、配送の判断ではない）。
//! - Engine の明示 SetOpen は常に出る（`ime_set_open_effects` が belief と一致していても足す）。
//! - 書き込みの前に `desired_open` と IntentStore が押下の向きに更新される（`write_physical_key` /
//!   `write_set_open_request` + `record_explicit_intent`。後者は `current_focus==None` では no-op）。
//!   chord フィルタで落ちた Engine の OFF は `desired_open` も IntentStore も更新しない（`desired_open` は
//!   belief と同じと仮定する）。
//! - 完了後の `applied` は `record_ime_apply_result`（generation なし）の意味論: 送信した/AlreadyMatched は
//!   `Confirmed(向き)`、`NotOwned`/`Unwarranted`/`UnsafeToToggle` は不変。
//! - 実 IME の応答（[`ime_after_press`]）: Allow で届いた物理キーは IME が意味どおり処理する（絶対キーは向き、
//!   トグルは反転）。awase の書き込みはその向きに設定する（二重 actuation のときは書き込みが後）。

use awase::engine::InputModeState;
use awase::types::{
    ImeRelevance, KeyClassification, KeyEventType, ModifierState, RawKeyEvent, ScanCode,
    ShadowImeAction,
};

use crate::focus::class_names::AppImeProfile;
use crate::state::actuation_chain::WriteMechanism;
use crate::state::ime_actuation_decision::{
    decide_attempt, decide_chain, decide_gate, shadow_toggle_demotes_applied, DecisionInputs,
    DecisionSite, GateResult,
};
use crate::state::ime_event::ImePolicyProfile;
use crate::state::ime_kind::ImeKindId;
use crate::state::physical_disposition::PhysicalKeyDisposition;

// ── 本番と共有する純粋な判断（key_pipeline / platform_state が呼ぶ）────────────────────

/// shadow toggle の意図ソース（`kp_stage_shadow_ime_toggle` の routing 用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadowIntentKind {
    /// config 由来の同期キー
    SyncKey,
    /// 物理 KANJI キー
    PhysicalImeKey,
}

/// shadow toggle へ昇格させる意図を選ぶ（`runtime/key_pipeline.rs::kp_stage_shadow_ime_toggle` から切り出した判断）。
///
/// 同期キー（`sync_direction`）> 静的に冪等な開閉キー（0x16/0x1A、ADR-207: `is_japanese_ime` を問わない）>
/// 日本語 IME のときだけ `shadow_action`（F13〜F24 の役割由来 Toggle は自動リピートの Down では昇格させない、
/// ADR-199 決定18(ii)）。どれにも当たらなければ `None`（昇格しない）。
///
/// `is_japanese_ime` は呼び出し時点の belief（0xF0〜0xF4 の物理受信による上げは呼び出し側が先に反映する）。
#[must_use]
pub(crate) fn select_shadow_intent(
    event: &RawKeyEvent,
    is_japanese_ime: bool,
) -> Option<(ShadowImeAction, ShadowIntentKind)> {
    if let Some(a) = event.ime_relevance.sync_direction {
        return Some((a, ShadowIntentKind::SyncKey));
    }
    if let Some(a) = event
        .ime_relevance
        .shadow_action
        .filter(|_| crate::vk::is_static_idempotent_open_key(event.vk_code))
    {
        // ADR-207: VK_IME_ON/OFF（0x16/0x1A）は IME の種類に依らず冪等なので、`is_japanese_ime`
        // （awase のワーカースレッドの HKL 由来で偽になりうる）を問わず採用する。`keys.ime_detect`
        // の既定（IMEオン/IMEオフ）を空にしても、従来 sync 既定が担っていた追随を保つ。
        return Some((a, ShadowIntentKind::PhysicalImeKey));
    }
    if is_japanese_ime {
        return event
            .ime_relevance
            .shadow_action
            // ADR-199 決定18(ii): F13〜F24 の役割由来 Toggle は自動リピートの Down では昇格させない
            // （物理の F13 はリピートし、`kp_stage_shadow_ime_toggle` はリピートを区別しないので、
            // そのままではリピートのたびに開閉が反転する）。0xF3/0xF4・0x19 の挙動は変えない。
            .filter(|_| !(event.was_down && crate::vk::is_role_fkey(event.vk_code)))
            .map(|a| (a, ShadowIntentKind::PhysicalImeKey));
    }
    None
}

/// Engine の明示 SetOpen が chord フィルタ（belief/`desired_open`/IntentStore を更新しない）に落ちるか
/// （`state/platform_state.rs::handle_engine_set_open` から切り出した判断）。
///
/// chord transaction（Ctrl+無変換）中の二次 IME OFF 要求だけが対象。**フィルタされても effect は executor へ流れる**
/// （ADR-213 P2d-2 で strip を撤去した。実書き込みの可否は gate・授権・already-matched が決める）。
#[must_use]
pub(crate) const fn engine_set_open_filtered_by_chord(chord_active: bool, target: bool) -> bool {
    chord_active && !target
}

// ── 入力型 ───────────────────────────────────────────────────────────────────

/// `applied`（実 IME へ最後に書いた/確認した open の記録、`AppliedImeState` の値だけを写したもの）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AppliedKnowledge {
    /// フォーカス直後・起動時。
    Unknown,
    /// ImmCross async の楽観的事前更新。
    Optimistic(bool),
    /// 実 apply 完了・確認済み。
    Confirmed(bool),
}

impl AppliedKnowledge {
    /// 全 5 値。
    pub const ALL: [Self; 5] = [
        Self::Unknown,
        Self::Optimistic(false),
        Self::Optimistic(true),
        Self::Confirmed(false),
        Self::Confirmed(true),
    ];

    /// `AppliedImeState::applied_open` 相当（Optimistic も含む。Unknown は None）。
    #[must_use]
    pub const fn open(self) -> Option<bool> {
        match self {
            Self::Unknown => None,
            Self::Optimistic(v) | Self::Confirmed(v) => Some(v),
        }
    }
}

/// 窓プロファイル（`AppImeProfile` 4 値 + `ImePolicyProfile` の Plain/Unknown。後者 2 つは現状到達不能だが
/// `caps` 表では ImmCross と同一でなければならない〈INV-44〉ので、全列挙に含めて固定する）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PressProfile {
    ImmCross,
    Plain,
    Unknown,
    ImmUnavailable,
    TsfNative,
    InputRelay,
}

impl PressProfile {
    /// 全 6 値。
    pub const ALL: [Self; 6] = [
        Self::ImmCross,
        Self::Plain,
        Self::Unknown,
        Self::ImmUnavailable,
        Self::TsfNative,
        Self::InputRelay,
    ];

    /// `PhysicalKeyDisposition::plan` / `decide_gate` が見るプロファイル。
    #[must_use]
    pub const fn app_profile(self) -> AppImeProfile {
        match self {
            Self::ImmCross | Self::Plain | Self::Unknown => AppImeProfile::Standard,
            Self::ImmUnavailable => AppImeProfile::Imm32Unavailable,
            Self::TsfNative => AppImeProfile::TsfNative,
            Self::InputRelay => AppImeProfile::InputRelay,
        }
    }

    /// `AppImePolicy::from_profile` / `caps` が見るプロファイル。
    #[must_use]
    pub fn policy_profile(self) -> ImePolicyProfile {
        match self {
            Self::Plain => ImePolicyProfile::Plain,
            Self::Unknown => ImePolicyProfile::Unknown,
            other => ImePolicyProfile::from(other.app_profile()),
        }
    }

    /// 実 IME の open 状態を直接読めない（`FeedbackPolicy::Blind`）プロファイルか。
    #[must_use]
    pub const fn is_blind(self) -> bool {
        matches!(self, Self::ImmUnavailable | Self::TsfNative)
    }
}

/// 明示キーの種別（ADR-208 監査 §1 の分類表の 12 種）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ExplicitKey {
    /// VK_IME_ON(0x16)。絶対 ON。shadow 昇格（`is_japanese_ime` を問わない、ADR-207）。
    StaticOn,
    /// VK_IME_OFF(0x1A)。絶対 OFF。同上。
    StaticOff,
    /// 半角/全角(0xF3/0xF4、役割 Toggle)。物理受信で `is_japanese_ime` を上げる（ADR-093）。
    HzToggle,
    /// 漢字(0x19、Toggle)。`is_japanese_ime` が真のときだけ shadow 昇格。
    Kanji,
    /// F13〜F24 の役割由来 Toggle（ADR-199 決定18）。
    RoleFkeyToggle,
    /// 同期キー（`keys.ime_detect`、ON 方向。IME の VK で `shadow_action` も持つ。昇格は同期キーが優先）。
    SyncOn,
    /// 同期キー（OFF 方向）。
    SyncOff,
    /// 同期キー（Toggle 方向）。
    SyncToggle,
    /// 英数/カタカナ/ひらがな(0xF0〜0xF2)。物理のみ（awase は書かない）。0xF0〜0xF4 の物理受信で `is_japanese_ime` を上げる。
    PhysOnlyMode,
    /// 役割を持たない無変換/変換。物理のみ（GJI 自身が処理、BUG-115）。
    ThumbPlain,
    /// Engine の明示 ON（Ctrl+変換・`keys.ime_on`・単独タップ）。Consume、書き込みは `SetOpen(true)`。
    EngineOn,
    /// Engine の明示 OFF（Ctrl+無変換・`keys.ime_off`・単独タップ）。
    EngineOff,
}

/// キーの意味（収束条件の判定用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyMeaning {
    /// 絶対指定（向きが決まっている）。
    Absolute(bool),
    /// トグル（実 IME の反転）。
    Toggle,
    /// 開閉の意図を持たない（物理のみ）。
    NoIntent,
}

/// 押下がたどる経路。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PressPath {
    /// hook の shadow toggle ステージ → `plan`（書き込みは `kp_shadow_actuate`）。
    Shadow,
    /// Engine の `SetOpen(ExplicitUserAction)`（`handle_engine_set_open` → executor）。
    Engine(bool),
}

impl ExplicitKey {
    /// 全 12 種。
    pub const ALL: [Self; 12] = [
        Self::StaticOn,
        Self::StaticOff,
        Self::HzToggle,
        Self::Kanji,
        Self::RoleFkeyToggle,
        Self::SyncOn,
        Self::SyncOff,
        Self::SyncToggle,
        Self::PhysOnlyMode,
        Self::ThumbPlain,
        Self::EngineOn,
        Self::EngineOff,
    ];

    /// キーの意味。
    #[must_use]
    pub const fn meaning(self) -> KeyMeaning {
        match self {
            Self::StaticOn | Self::SyncOn | Self::EngineOn => KeyMeaning::Absolute(true),
            Self::StaticOff | Self::SyncOff | Self::EngineOff => KeyMeaning::Absolute(false),
            Self::HzToggle | Self::Kanji | Self::RoleFkeyToggle | Self::SyncToggle => {
                KeyMeaning::Toggle
            }
            Self::PhysOnlyMode | Self::ThumbPlain => KeyMeaning::NoIntent,
        }
    }

    const fn path(self) -> PressPath {
        match self {
            Self::EngineOn => PressPath::Engine(true),
            Self::EngineOff => PressPath::Engine(false),
            _ => PressPath::Shadow,
        }
    }

    /// 物理（非注入・非リピート）KeyDown として hook が組み立てるイベント。Engine 経路のキーは
    /// 配送が常に Consume なので `plan` には渡さない（`None`）。
    fn event(self) -> Option<RawKeyEvent> {
        let (vk, shadow_action, sync_direction) = match self {
            Self::StaticOn => (crate::vk::VK_IME_ON, Some(ShadowImeAction::TurnOn), None),
            Self::StaticOff => (crate::vk::VK_IME_OFF, Some(ShadowImeAction::TurnOff), None),
            Self::HzToggle => (
                crate::vk::VK_DBE_SBCSCHAR,
                Some(ShadowImeAction::Toggle),
                None,
            ),
            Self::Kanji => (crate::vk::VK_KANJI, Some(ShadowImeAction::Toggle), None),
            Self::RoleFkeyToggle => (crate::vk::VK_F13, Some(ShadowImeAction::Toggle), None),
            // 同期キー（`keys.ime_detect`）は `enrich_ime_relevance` が `sync_direction` を付ける。ここでは IME の VK
            // （かな 0x15 等。静的分類で `shadow_action` も付く）を設定した構成を表す。IME の VK でない任意の VK を
            // 設定した構成は、物理キーが IME に何も作用しない（Allow しても二重 actuation にならない）ので扱わない。
            Self::SyncOn => (
                crate::vk::VK_KANA,
                Some(ShadowImeAction::TurnOn),
                Some(ShadowImeAction::TurnOn),
            ),
            Self::SyncOff => (
                crate::vk::VK_KANA,
                Some(ShadowImeAction::TurnOff),
                Some(ShadowImeAction::TurnOff),
            ),
            Self::SyncToggle => (
                crate::vk::VK_KANA,
                Some(ShadowImeAction::Toggle),
                Some(ShadowImeAction::Toggle),
            ),
            Self::PhysOnlyMode => (crate::vk::VK_DBE_ALPHANUMERIC, None, None),
            Self::ThumbPlain => (crate::vk::VK_NONCONVERT, None, None),
            Self::EngineOn | Self::EngineOff => return None,
        };
        Some(RawKeyEvent {
            was_down: false,
            vk_code: vk,
            scan_code: ScanCode(0),
            event_type: KeyEventType::KeyDown,
            extra_info: 0,
            timestamp: 0,
            key_classification: KeyClassification::Passthrough,
            physical_pos: None,
            ime_relevance: ImeRelevance {
                shadow_action,
                sync_direction,
                is_sync_key: sync_direction.is_some(),
                ..ImeRelevance::default()
            },
            modifier_key: None,
            modifier_snapshot: ModifierState::default(),
            left_thumb_down_snapshot: None,
            right_thumb_down_snapshot: None,
            injected: false,
        })
    }

    /// このキーの物理受信が `is_japanese_ime` を上げるか（`vk::should_upgrade_is_japanese_ime`、ADR-093）。
    fn upgrades_is_japanese(self) -> bool {
        self.event()
            .is_some_and(|e| crate::vk::should_upgrade_is_japanese_ime(e.injected, e.vk_code))
    }
}

/// 押下直前の内部状態（配送の判断に効くものだけ）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PressState {
    /// `ImeModel::effective_open()`（shadow toggle の `current`）。
    pub belief_open: bool,
    pub applied: AppliedKnowledge,
    /// `belief.is_japanese_ime()`（押下前）。
    pub is_japanese_ime: bool,
    pub profile: PressProfile,
    pub ime_kind: ImeKindId,
    /// `ImeModel::current_focus()` が `Some` か。
    pub current_focus_known: bool,
    /// 鮮度内（3s）の Actuating 観測の open 値（`ObservationStore::derive_actuating`）。
    pub actuating_obs: Option<bool>,
    /// 現在のフォーカス対象に対する有効な `IntentStore` の明示意図。
    pub intent: Option<bool>,
    /// GJI candidate SHOW の desync 証拠（`tsf::observer::candidate_was_seen()`）。
    pub candidate_was_seen: bool,
    /// `is_ctrl_ime_chord_active()`（Ctrl+無変換の chord transaction 中）。
    pub ctrl_chord: bool,
    /// Win キー押下中（`UnsafeToToggle`）。
    pub win_held: bool,
}

impl PressState {
    /// 状態空間の全列挙（キー種別を除く。約 3.5 万通り）。順序は決定的。
    pub fn all() -> impl Iterator<Item = Self> {
        const B: [bool; 2] = [false, true];
        const OBS: [Option<bool>; 3] = [None, Some(false), Some(true)];
        let mut v = Vec::with_capacity(2 * 5 * 2 * 6 * 2 * 2 * 3 * 3 * 2 * 2 * 2);
        for belief_open in B {
            for applied in AppliedKnowledge::ALL {
                for is_japanese_ime in B {
                    for profile in PressProfile::ALL {
                        for ime_kind in [ImeKindId::Gji, ImeKindId::MsIme] {
                            for current_focus_known in B {
                                for actuating_obs in OBS {
                                    for intent in OBS {
                                        for candidate_was_seen in B {
                                            for ctrl_chord in B {
                                                for win_held in B {
                                                    v.push(Self {
                                                        belief_open,
                                                        applied,
                                                        is_japanese_ime,
                                                        profile,
                                                        ime_kind,
                                                        current_focus_known,
                                                        actuating_obs,
                                                        intent,
                                                        candidate_was_seen,
                                                        ctrl_chord,
                                                        win_held,
                                                    });
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        v.into_iter()
    }

    /// 実機で起こりうる組み合わせか。Blind プロファイル（Chrome/Edge/WT 等）は実 IME の open を直接読めないので、
    /// Actuating な観測は構造的に存在しない。反例の件数を「全空間」と「起こりうる空間」で並べるために使う。
    #[must_use]
    pub const fn is_plausible(&self) -> bool {
        !(self.profile.is_blind() && self.actuating_obs.is_some())
    }
}

// ── 出力型 ───────────────────────────────────────────────────────────────────

/// 物理キーの配送。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Physical {
    /// 元の物理キーを OS（IME）へ届ける。
    Allow,
    /// 元の物理キーを握りつぶす（awase が代わりに書く前提）。
    Suppress,
    /// エンジンが消費する（Engine 経路。書き込みは executor）。
    Consume,
}

/// 書き込みが無い（または送られなかった）理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElisionReason {
    /// 書き込んだ（`write` が `Some`）。
    Written,
    /// 昇格しなかった（shadow 昇格の条件に当たらない。`is_japanese_ime` が偽の漢字等、または意図を持たないキー）。
    NotPromoted,
    /// shadow no-op（belief が既にキーの向き。書かない）。
    ShadowNoop,
    /// InputRelay の窓: awase は actuation を所有しない（`decide_gate` が `NotOwned`）。
    InputRelayNotOwned,
    /// 授権（`issue_open_warrant`）が下りない。
    Unwarranted,
    /// GjiDirect の already-matched（`applied` が向きと一致していて省略）。
    AlreadyMatched,
    /// Win キー押下中（`UnsafeToToggle`。その押下だけで状態は変わらない）。
    WinHeld,
}

/// 1 押下の配送の決定結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Delivery {
    pub physical: Physical,
    /// awase が書く open 値（`None` = 書かない）。
    pub write: Option<bool>,
    pub reason: ElisionReason,
    /// 押下が belief に採用した向き（昇格しなかった/Engine でフィルタされたら `None`）。
    pub target: Option<bool>,
    /// shadow toggle が belief を倒したか（`plan` の入力）。
    pub shadow_toggled: bool,
    /// 押下後の belief。
    pub belief_after: bool,
    /// 押下後に IntentStore が持つ（フォーカス対象の）明示意図。
    pub intent_after: Option<bool>,
}

/// 授権（`issue_open_warrant`）の問い合わせ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WarrantRequest {
    pub requested: bool,
    /// フォーカス対象が既知か（`false` なら対象は `HwndId::NULL` で、IntentStore の Step 1 は外れる）。
    pub target_known: bool,
    /// 押下の書き込みの前に反映済みの `is_japanese_ime`。
    pub is_japanese_ime: bool,
    pub policy_profile: ImePolicyProfile,
    /// 書き込み時点の `desired_open`（Step 4c の OwnSsot の根拠）。
    pub desired_open: bool,
    /// 書き込み時点で IntentStore がフォーカス対象に持つ明示意図。
    pub intent: Option<bool>,
    /// 鮮度内の Actuating 観測の open 値。
    pub actuating_obs: Option<bool>,
}

/// 授権の判定器。本番は live の `WarrantContext`、テストは合成ストアで `issue_open_warrant` を呼ぶ。
pub trait WarrantJudge {
    /// `issue_open_warrant(requested, ..)` が `Some`（授権あり）か。
    fn warranted(&self, req: &WarrantRequest) -> bool;
}

// ── 決定関数 ─────────────────────────────────────────────────────────────────

/// 書き込みの試行（gate → 授権 → 先頭機構の already-matched 判定）。`shadow_on` は view の `ControlLog.shadow_on`。
fn attempt_write(
    state: &PressState,
    open: bool,
    shadow_on: Option<bool>,
    req: WarrantRequest,
    judge: &impl WarrantJudge,
) -> (Option<bool>, ElisionReason) {
    let inputs = DecisionInputs {
        profile: state.profile.app_profile(),
        kind: state.ime_kind,
        shadow_on,
        belief_input_mode: InputModeState::Unknown,
        candidate_was_seen: state.candidate_was_seen,
    };
    if decide_gate(inputs) == GateResult::NotOwned {
        return (None, ElisionReason::InputRelayNotOwned);
    }
    if !judge.warranted(&req) {
        return (None, ElisionReason::Unwarranted);
    }
    let mechanism: WriteMechanism = decide_chain(inputs)[0];
    let (_, command) = decide_attempt(inputs, DecisionSite::Sync, mechanism, open);
    if command.is_none() {
        return (None, ElisionReason::AlreadyMatched);
    }
    if state.win_held {
        return (None, ElisionReason::WinHeld);
    }
    (Some(open), ElisionReason::Written)
}

/// 明示キー 1 押下の配送を決める（授権の判定器を差し込む形。上のモジュール doc 参照）。
///
/// # Panics
///
/// 起きない（Shadow 経路のキーは必ず `RawKeyEvent` を持つ。`ExplicitKey::event` が `None` なのは Engine 経路のキーだけ）。
#[must_use]
pub fn explicit_press_delivery_with(
    state: &PressState,
    key: ExplicitKey,
    judge: &impl WarrantJudge,
) -> Delivery {
    let is_japanese_ime = state.is_japanese_ime || key.upgrades_is_japanese();
    let policy_profile = state.profile.policy_profile();
    match key.path() {
        PressPath::Engine(target) => {
            let filtered = engine_set_open_filtered_by_chord(state.ctrl_chord, target);
            let desired_open = if filtered { state.belief_open } else { target };
            let intent = match (filtered, state.current_focus_known) {
                (_, false) => None,
                (true, true) => state.intent,
                (false, true) => Some(target),
            };
            let req = WarrantRequest {
                requested: target,
                target_known: state.current_focus_known,
                is_japanese_ime,
                policy_profile,
                desired_open,
                intent,
                actuating_obs: state.actuating_obs,
            };
            // executor は `applied_snapshot` をそのまま渡す（shadow 経路の降格は無い）。
            let (write, reason) = attempt_write(state, target, state.applied.open(), req, judge);
            Delivery {
                physical: Physical::Consume,
                write,
                reason,
                target: (!filtered).then_some(target),
                shadow_toggled: false,
                belief_after: if filtered { state.belief_open } else { target },
                intent_after: intent,
            }
        }
        PressPath::Shadow => {
            let event = key
                .event()
                .expect("Shadow 経路のキーは必ず RawKeyEvent を持つ");
            let intent_kind = select_shadow_intent(&event, is_japanese_ime);
            let resolved = intent_kind.map(|(action, _)| action.resolve(state.belief_open));
            let shadow_toggled = resolved.is_some_and(|new_val| new_val != state.belief_open);
            let physical = match PhysicalKeyDisposition::plan_core(
                &event,
                state.profile.app_profile(),
                shadow_toggled,
                state.ime_kind,
            ) {
                PhysicalKeyDisposition::Allow => Physical::Allow,
                PhysicalKeyDisposition::Suppress => Physical::Suppress,
            };
            let belief_after = resolved.unwrap_or(state.belief_open);
            let Some(new_val) = resolved else {
                return Delivery {
                    physical,
                    write: None,
                    reason: ElisionReason::NotPromoted,
                    target: None,
                    shadow_toggled,
                    belief_after,
                    intent_after: state.intent,
                };
            };
            // 昇格した押下は `write_physical_key`/`write_sync_key` が意図を記録する（フォーカス不明なら no-op）。
            let intent_after = if state.current_focus_known {
                Some(new_val)
            } else {
                None
            };
            if !shadow_toggled {
                return Delivery {
                    physical,
                    write: None,
                    reason: ElisionReason::ShadowNoop,
                    target: Some(new_val),
                    shadow_toggled,
                    belief_after,
                    intent_after,
                };
            }
            let req = WarrantRequest {
                requested: new_val,
                target_known: state.current_focus_known,
                is_japanese_ime,
                policy_profile,
                desired_open: new_val,
                intent: intent_after,
                actuating_obs: state.actuating_obs,
            };
            // `kp_shadow_actuate`: `applied` が向きと一致するなら view の `shadow_on` を未知にする（M1、PR #408）。
            let applied_open = state.applied.open();
            let shadow_on = if shadow_toggle_demotes_applied(applied_open, new_val) {
                None
            } else {
                applied_open
            };
            let (write, reason) = attempt_write(state, new_val, shadow_on, req, judge);
            Delivery {
                physical,
                write,
                reason,
                target: Some(new_val),
                shadow_toggled,
                belief_after,
                intent_after,
            }
        }
    }
}

// ── 状態遷移モデル ────────────────────────────────────────────────────────────

/// 押下後の実 IME の open 状態（モデルの前提はモジュール doc 参照）。
#[must_use]
pub fn ime_after_press(real_open: bool, key: ExplicitKey, d: &Delivery) -> bool {
    let mut r = real_open;
    if d.physical == Physical::Allow {
        r = match key.meaning() {
            KeyMeaning::Absolute(t) => t,
            KeyMeaning::Toggle => !r,
            KeyMeaning::NoIntent => r,
        };
    }
    if let Some(o) = d.write {
        r = o;
    }
    r
}

/// 押下後の内部状態（belief・applied・is_japanese_ime・IntentStore・candidate_was_seen）。
#[must_use]
pub fn state_after_press(state: &PressState, key: ExplicitKey, d: &Delivery) -> PressState {
    // 送信した → 完了で `Confirmed(向き)`。AlreadyMatched も `record_confirmed(open)`。それ以外（NotOwned/Unwarranted/
    // WinHeld/書かない）は不変。GjiDirect の OFF 送信は candidate_was_seen を消費する（ADR-171）。
    let confirmed = match d.write {
        Some(open) => Some(open),
        None if d.reason == ElisionReason::AlreadyMatched => {
            d.target.or_else(|| key_direction(key))
        }
        None => None,
    };
    PressState {
        belief_open: d.belief_after,
        is_japanese_ime: state.is_japanese_ime || key.upgrades_is_japanese(),
        intent: d.intent_after,
        applied: confirmed.map_or(state.applied, AppliedKnowledge::Confirmed),
        candidate_was_seen: state.candidate_was_seen
            && !(d.write == Some(false) && state.ime_kind == ImeKindId::Gji),
        ..*state
    }
}

const fn key_direction(key: ExplicitKey) -> Option<bool> {
    match key.meaning() {
        KeyMeaning::Absolute(t) => Some(t),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 授権を常に下ろす/下ろさない判定器（本物の `issue_open_warrant` を使う判定器は
    /// `tests/explicit_press_exhaustive.rs`）。
    struct Always(bool);
    impl WarrantJudge for Always {
        fn warranted(&self, _req: &WarrantRequest) -> bool {
            self.0
        }
    }

    fn base() -> PressState {
        PressState {
            belief_open: false,
            applied: AppliedKnowledge::Unknown,
            is_japanese_ime: true,
            profile: PressProfile::ImmUnavailable,
            ime_kind: ImeKindId::Gji,
            current_focus_known: true,
            actuating_obs: None,
            intent: None,
            candidate_was_seen: false,
            ctrl_chord: false,
            win_held: false,
        }
    }

    #[test]
    fn state_space_size_is_pinned() {
        // belief 2 × applied 5 × japanese 2 × profile 6 × kind 2 × focus 2 × obs 3 × intent 3 ×
        // candidate 2 × chord 2 × win 2
        assert_eq!(
            PressState::all().count(),
            2 * 5 * 2 * 6 * 2 * 2 * 3 * 3 * 2 * 2 * 2
        );
        assert_eq!(ExplicitKey::ALL.len(), 12);
    }

    #[test]
    fn engine_on_in_gji_blind_is_elided_when_applied_matches() {
        // S-1: Engine 経由の絶対キーは `applied` が向きと一致すると GjiDirect の already-matched で省略される。
        let mut s = base();
        s.applied = AppliedKnowledge::Confirmed(true);
        let d = explicit_press_delivery_with(&s, ExplicitKey::EngineOn, &Always(true));
        assert_eq!(d.physical, Physical::Consume);
        assert_eq!(d.write, None);
        assert_eq!(d.reason, ElisionReason::AlreadyMatched);
    }

    #[test]
    fn shadow_toggle_demotes_applied_so_the_write_is_not_elided() {
        // #408: shadow 経路は同じ状態でも降格して書く。
        let mut s = base();
        s.applied = AppliedKnowledge::Confirmed(true);
        let d = explicit_press_delivery_with(&s, ExplicitKey::StaticOn, &Always(true));
        assert_eq!(d.physical, Physical::Suppress);
        assert_eq!(d.write, Some(true));
        assert_eq!(d.reason, ElisionReason::Written);
    }

    #[test]
    fn static_open_keys_are_promoted_even_when_not_japanese() {
        // ADR-207: 0x16/0x1A は is_japanese_ime を問わず昇格する。
        let mut s = base();
        s.is_japanese_ime = false;
        let d = explicit_press_delivery_with(&s, ExplicitKey::StaticOn, &Always(true));
        assert!(d.shadow_toggled);
        // 一方、漢字(0x19)は is_japanese_ime が偽だと昇格しない。
        let d = explicit_press_delivery_with(&s, ExplicitKey::Kanji, &Always(true));
        assert_eq!(d.reason, ElisionReason::NotPromoted);
    }

    #[test]
    fn hz_key_upgrades_is_japanese_ime_before_promotion() {
        // ADR-093: 0xF3/0xF4 の物理受信は is_japanese_ime を上げてから昇格判定する。
        let mut s = base();
        s.is_japanese_ime = false;
        let d = explicit_press_delivery_with(&s, ExplicitKey::HzToggle, &Always(true));
        assert!(d.shadow_toggled);
        assert!(state_after_press(&s, ExplicitKey::HzToggle, &d).is_japanese_ime);
    }

    #[test]
    fn chord_filtered_engine_off_leaves_belief_and_intent_but_still_attempts_the_write() {
        let mut s = base();
        s.belief_open = true;
        s.ctrl_chord = true;
        s.intent = Some(true);
        let d = explicit_press_delivery_with(&s, ExplicitKey::EngineOff, &Always(true));
        assert_eq!(d.target, None);
        assert!(d.belief_after);
        assert_eq!(d.intent_after, Some(true));
        assert_eq!(d.write, Some(false));
    }

    #[test]
    fn input_relay_engine_key_is_consumed_and_not_written() {
        // L-5: 現状は Consume のまま誰も書かない（ADR-208 決定3 で素通しへ変える）。
        let mut s = base();
        s.profile = PressProfile::InputRelay;
        let d = explicit_press_delivery_with(&s, ExplicitKey::EngineOn, &Always(true));
        assert_eq!(d.physical, Physical::Consume);
        assert_eq!(d.reason, ElisionReason::InputRelayNotOwned);
        // shadow 経路のキーは Allow（`plan` が InputRelay を常に Allow にする）。
        let d = explicit_press_delivery_with(&s, ExplicitKey::StaticOn, &Always(true));
        assert_eq!(d.physical, Physical::Allow);
    }

    #[test]
    fn select_shadow_intent_matches_the_pre_extraction_branches() {
        // 切り出し前の `kp_stage_shadow_ime_toggle` の 3 分岐を、キー種別ごとに固定する。
        let ev = |k: ExplicitKey| k.event().expect("event");
        // 同期キーは is_japanese_ime を問わず SyncKey。
        for jp in [false, true] {
            assert_eq!(
                select_shadow_intent(&ev(ExplicitKey::SyncOn), jp),
                Some((ShadowImeAction::TurnOn, ShadowIntentKind::SyncKey))
            );
        }
        // 0x16/0x1A は is_japanese_ime を問わず PhysicalImeKey。
        for jp in [false, true] {
            assert_eq!(
                select_shadow_intent(&ev(ExplicitKey::StaticOff), jp),
                Some((ShadowImeAction::TurnOff, ShadowIntentKind::PhysicalImeKey))
            );
        }
        // 漢字・半角/全角・F13 は is_japanese_ime が真のときだけ。
        for k in [
            ExplicitKey::Kanji,
            ExplicitKey::HzToggle,
            ExplicitKey::RoleFkeyToggle,
        ] {
            assert_eq!(select_shadow_intent(&ev(k), false), None, "{k:?}");
            assert_eq!(
                select_shadow_intent(&ev(k), true),
                Some((ShadowImeAction::Toggle, ShadowIntentKind::PhysicalImeKey)),
                "{k:?}"
            );
        }
        // F13 の自動リピート Down は昇格しない。
        let mut rep = ev(ExplicitKey::RoleFkeyToggle);
        rep.was_down = true;
        assert_eq!(select_shadow_intent(&rep, true), None);
        // 同じリピートでも 0xF3 は昇格する（0xF3/0xF4・0x19 の挙動は変えない）。
        let mut rep = ev(ExplicitKey::HzToggle);
        rep.was_down = true;
        assert!(select_shadow_intent(&rep, true).is_some());
        // 意図を持たないキーは常に None。
        for k in [ExplicitKey::PhysOnlyMode, ExplicitKey::ThumbPlain] {
            assert_eq!(select_shadow_intent(&ev(k), true), None, "{k:?}");
        }
    }

    #[test]
    fn engine_chord_filter_only_drops_off() {
        assert!(engine_set_open_filtered_by_chord(true, false));
        assert!(!engine_set_open_filtered_by_chord(true, true));
        assert!(!engine_set_open_filtered_by_chord(false, false));
        assert!(!engine_set_open_filtered_by_chord(false, true));
    }
}
