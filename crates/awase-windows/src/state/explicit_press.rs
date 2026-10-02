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
//!   `decide_chain`・`decide_attempt`・`explicit_press_shadow_on`（`state/ime_actuation_decision.rs`）
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
//! - 完了後の `applied` は**実物の遷移を呼ぶ**: shadow 経路は `ime_model::apply_result_effective_open`
//!   （`record_ime_apply_result` の generation=None 分岐の純粋部）と `ImeModel::confirm_applied`、Engine 経路は
//!   `ImeModel::reduce`（`ImeApplyRequested` → `ImeEvent::from_apply_outcome`、`completion_can_update_applied` を含む）。
//! - 実 IME の応答（[`ime_after_press`]）: 前提 A1（[`A1_KEYS`]）のキーを Allow で配送すれば IME が意味どおり処理する
//!   （絶対キーは向き、トグルは反転）。それ以外のキーの配送では何も起きないとみなす。awase の書き込みはその向きに設定する。

use awase::engine::InputModeState;
use awase::types::{
    ImeRelevance, KeyClassification, KeyEventType, ModifierState, RawKeyEvent, ScanCode,
    ShadowImeAction,
};

use crate::focus::class_names::AppImeProfile;
use crate::state::actuation_chain::WriteMechanism;
use crate::state::ime_actuation_decision::{
    decide_attempt, decide_chain, decide_gate, engine_press_unknowns_applied,
    explicit_press_shadow_on, DecisionInputs, DecisionSite, GateResult,
};
use crate::state::ime_event::ImePolicyProfile;
use crate::state::ime_kind::ImeKindId;
use crate::state::ime_model::{apply_result_effective_open, AppliedImeState, ImeModel};
use crate::state::physical_disposition::PhysicalKeyDisposition;
use crate::state::press_ledger::{
    outcome_sent_nothing, PressLedger, PressSource, DUPLICATE_OUTCOME,
};
use crate::state::ApplyGeneration;
use awase::platform::ImeOpenOutcome;
use awase::types::PressId;

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
/// `shadow_action`（0x19・0xF3/0xF4・F13〜F24 の役割由来 Toggle。F13〜F24 は自動リピートの Down では昇格させない、
/// ADR-199 決定18(ii)）。どれにも当たらなければ `None`（昇格しない）。
///
/// **`shadow_action` の昇格は、非リピートの押下（押下 ID を持つ order、ADR-208 決定2 D2・L2）なら `is_japanese_ime` を問わない。**
/// 以前は偽（probe の誤答・ワーカースレッドの HKL、ADR-207）の間は 0x19 が昇格せず、IC では Suppress されるのに書かれず固着した
/// （L-4、S-2）。リピート（`was_down`、押下 ID なし）は従来どおり `is_japanese_ime` が真のときだけ（押下 ID を持たない order は
/// 授権が `is_japanese_ime` を問うので、昇格して belief だけ反転し書かれない食い違いを作らない）。`is_japanese_ime` は
/// belief のスコープ判定（0xF0〜0xF4 の物理受信で上げる ADR-093）のままで、上げる規則は変えない。
///
/// **0x19 だけは、`is_japanese_ime` の代わりに「IME が TIP で同定済み」（`ime_identified`、本番は `table_ime_kind().is_some()`）を条件に
/// 残す**（所有者決定）。F13〜F24・0xF3/0xF4 の役割由来 `shadow_action` は IME 未同定なら `enrich_key_role` が付けない（`None`）ので
/// 同定で守られるが、0x19 は GJI 以外では hook の静的値（Toggle）のまま（`kanji_shadow_action` の `KeepStatic`）なので、条件を外すと
/// 英語 IME・IME 無しの窓・US 配列の Alt+` の 0x19 でも belief 反転と書き込みが起きる。同定済みなら `is_japanese_ime` の誤判定でも昇格する。
///
/// `is_japanese_ime` は呼び出し時点の belief（0xF0〜0xF4 の物理受信による上げは呼び出し側が先に反映する）。
#[must_use]
pub(crate) fn select_shadow_intent(
    event: &RawKeyEvent,
    is_japanese_ime: bool,
    ime_identified: bool,
) -> Option<(ShadowImeAction, ShadowIntentKind)> {
    if let Some(a) = event.ime_relevance.sync_direction {
        return Some((a, ShadowIntentKind::SyncKey));
    }
    if let Some(a) = event
        .ime_relevance
        .shadow_action
        .filter(|_| crate::vk::is_static_idempotent_open_key(event.vk_code))
    {
        // ADR-207: VK_IME_ON/OFF（0x16/0x1A）は IME の種類に依らず冪等なので、`is_japanese_ime()`
        // （awase のワーカースレッドの HKL 由来で偽になりうる）を問わず採用する。
        return Some((a, ShadowIntentKind::PhysicalImeKey));
    }
    // 非リピートの押下は `is_japanese_ime` を問わない。ただし 0x19 は GJI 以外では hook の静的 Toggle のままなので、
    // IME が TIP で同定済みのときだけ昇格する（英語 IME・IME 無しの窓・US 配列の Alt+` に副作用を及ぼさない）。
    let is_kanji = matches!(
        crate::vk::VkCodeExt::ime_kind(event.vk_code),
        Some(crate::vk::ImeKeyKind::Kanji)
    );
    if is_japanese_ime || (!event.was_down && (!is_kanji || ime_identified)) {
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
    /// `Imm32Unavailable` に分類されるが実質 TSF ネイティブのクラス（Windows Terminal の
    /// `CASCADIA_HOSTING_WINDOW_CLASS` 等、`AppImeProfile::is_effectively_tsf_native`）。
    ImmUnavailableTsfClass,
    TsfNative,
    InputRelay,
}

impl PressProfile {
    /// 全 7 値。
    pub const ALL: [Self; 7] = [
        Self::ImmCross,
        Self::Plain,
        Self::Unknown,
        Self::ImmUnavailable,
        Self::ImmUnavailableTsfClass,
        Self::TsfNative,
        Self::InputRelay,
    ];

    /// `PhysicalKeyDisposition::plan` / `decide_gate` が見るプロファイル。
    #[must_use]
    pub const fn app_profile(self) -> AppImeProfile {
        match self {
            Self::ImmCross | Self::Plain | Self::Unknown => AppImeProfile::Standard,
            Self::ImmUnavailable | Self::ImmUnavailableTsfClass => AppImeProfile::Imm32Unavailable,
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
        matches!(
            self,
            Self::ImmUnavailable | Self::ImmUnavailableTsfClass | Self::TsfNative
        )
    }

    /// `AppImeProfile::is_effectively_tsf_native(class_name)` に相当（Engine 経路の段階制御の入力）。
    #[must_use]
    pub const fn is_effectively_tsf_native(self) -> bool {
        matches!(self, Self::ImmUnavailableTsfClass | Self::TsfNative)
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
    /// 漢字(0x19、Toggle)。shadow 昇格（`is_japanese_ime` を問わない、ADR-208 D2）。
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

/// 前提 A1 が成り立つキーの表（ADR-208 決定1）。成り立たないキー（任意の sync キー・漢字 0x19・F13 等）は
/// 「Allow で配送してよいキー」から外れ、Suppress + write 側に寄せる。0xF3/0xF4 は GJI 学習表で開閉トグルとされる場合。
pub const A1_KEYS: [ExplicitKey; 4] = [
    ExplicitKey::StaticOn,
    ExplicitKey::StaticOff,
    ExplicitKey::HzToggle,
    ExplicitKey::PhysOnlyMode,
];

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

    /// 前提 A1（ADR-208 決定1）: 物理キーを配送すれば IME がキーの意味どおりに処理する。成り立つのは 0x16/0x1A・
    /// F2/0xF0 等の物理のみのモード・GJI 学習表で開閉トグルとされる 0xF3/0xF4。任意の sync キー・漢字(0x19)・
    /// 役割由来の F13 等・未学習の構成は成り立たない（それらを Allow で配送するのは INV-L1 の (i) として認めない）。
    #[must_use]
    pub fn a1_holds(self) -> bool {
        A1_KEYS.contains(&self)
    }

    /// shadow toggle 経路（hook の昇格 → `plan`）のキーか（`false` は Engine の SetOpen 経路）。
    #[must_use]
    pub const fn is_shadow_path(self) -> bool {
        matches!(self.path(), PressPath::Shadow)
    }

    /// 対象押下（ADR-208 決定1）になりうるキーか。`shadow_action`/`sync_direction` を持つか、Engine が SetOpen を出す。
    /// 役割を持たない無変換/変換は物理のみで awase は何も決めないので対象外。
    #[must_use]
    pub const fn is_target_press_key(self) -> bool {
        !matches!(self, Self::ThumbPlain)
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
    fn event(self, was_down: bool) -> Option<RawKeyEvent> {
        self.event_for(was_down, false)
    }

    /// [`Self::event`] の、0x19 を受動（`kanji_passive`、ADR-208 L2 M-1: `enrich_key_role` が `shadow_action` を付けない）にできる版。
    fn event_for(self, was_down: bool, kanji_passive: bool) -> Option<RawKeyEvent> {
        let (vk, shadow_action, sync_direction) = match self {
            Self::StaticOn => (crate::vk::VK_IME_ON, Some(ShadowImeAction::TurnOn), None),
            Self::StaticOff => (crate::vk::VK_IME_OFF, Some(ShadowImeAction::TurnOff), None),
            Self::HzToggle => (
                crate::vk::VK_DBE_SBCSCHAR,
                Some(ShadowImeAction::Toggle),
                None,
            ),
            Self::Kanji => (
                crate::vk::VK_KANJI,
                (!kanji_passive).then_some(ShadowImeAction::Toggle),
                None,
            ),
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
            was_down,
            press_id: None,
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
        self.event(false)
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
    /// TIP で IME が同定済みか（本番は `tsf_obs().table_ime_kind().is_some()`: GJI、または CLSID で同定できた MS-IME 本体）。
    /// 偽（英語 IME・IME 無しの窓・第三者 IME・起動直後）は `ime_kind` が `MsIme`（既定の推定）のときだけ取りうる。
    /// 0x19 の shadow 昇格の条件（ADR-208 D2、所有者決定）。
    pub ime_identified: bool,
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
    /// 自動リピートの Down か（対象押下は非リピートのみ。リピートは `press=None` で従来の `applied` 省略に任せる）。
    pub was_down: bool,
}

impl PressState {
    /// 状態空間の全列挙（キー種別を除く。120,960 通り）。順序は決定的。
    pub fn all() -> impl Iterator<Item = Self> {
        const B: [bool; 2] = [false, true];
        const OBS: [Option<bool>; 3] = [None, Some(false), Some(true)];
        let mut v = Vec::with_capacity(2 * 5 * 2 * 7 * 2 * 2 * 3 * 3 * 2 * 2 * 2 * 2);
        for belief_open in B {
            for applied in AppliedKnowledge::ALL {
                for is_japanese_ime in B {
                    for profile in PressProfile::ALL {
                        for (ime_kind, ime_identified) in [
                            (ImeKindId::Gji, true),
                            (ImeKindId::MsIme, true),
                            (ImeKindId::MsIme, false),
                        ] {
                            for current_focus_known in B {
                                for actuating_obs in OBS {
                                    for intent in OBS {
                                        for candidate_was_seen in B {
                                            for ctrl_chord in B {
                                                for win_held in B {
                                                    for was_down in B {
                                                        v.push(Self {
                                                            belief_open,
                                                            applied,
                                                            is_japanese_ime,
                                                            profile,
                                                            ime_kind,
                                                            ime_identified,
                                                            current_focus_known,
                                                            actuating_obs,
                                                            intent,
                                                            candidate_was_seen,
                                                            ctrl_chord,
                                                            win_held,
                                                            was_down,
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
        }
        v.into_iter()
    }

    /// 先頭の書き込み機構が GjiDirect か（GjiDirect だけが `applied` の already-matched 省略を持つ。
    /// L1 の自動リピートで「同じ向きの VK を追い送りしない」が `applied` 省略に頼れるかの判定に使う）。
    #[must_use]
    pub fn chain_head_is_gji_direct(&self) -> bool {
        let inputs = DecisionInputs {
            profile: self.profile.app_profile(),
            kind: self.ime_kind,
            shadow_on: None,
            belief_input_mode: InputModeState::Unknown,
            candidate_was_seen: false,
        };
        decide_chain(inputs)[0] == WriteMechanism::GjiDirect
    }

    /// 先頭の書き込み機構が ImmCross か（ImmCross は非同期で完了が後から届くので、予約を解けない）。
    #[must_use]
    pub fn chain_head_is_imm_cross(&self) -> bool {
        let inputs = DecisionInputs {
            profile: self.profile.app_profile(),
            kind: self.ime_kind,
            shadow_on: None,
            belief_input_mode: InputModeState::Unknown,
            candidate_was_seen: false,
        };
        decide_chain(inputs)[0] == WriteMechanism::ImmCross
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
    /// 同じ押下で既に同じ向きを予約済み（`PressLedger::claim` が `Duplicate`/`ConflictKept`。ADR-208 L1 D1、BUG-113）。
    /// Engine 経路は `AlreadyMatched` を返して完了へ流し、shadow 経路は何も返さない（`applied` を動かさない）。
    AlreadyWrittenThisPress,
}

/// 1 押下の配送の決定結果（現状の本番の判断をそのまま記録した形。二重・空振りも表せる）。
///
/// INV-L1 は「(i) 物理を配送する か (ii) awase が書く か、ちょうど一方」。この型は現状の違反も記録するために両方を持てる
/// ので、INV-L1 が成り立つ決定だけを表す型 [`Resolution`] へ [`Delivery::resolve`] で写し、写せないものを
/// [`Violation`]（反例）として返す。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Delivery {
    pub physical: Physical,
    /// awase が書く open 値（`None` = 書かない）。
    pub write: Option<bool>,
    pub reason: ElisionReason,
    /// 書き込みを試みた open 値（gate/授権/省略で書かなかった場合も含む。試みなければ `None`）。
    pub requested: Option<bool>,
    /// 押下が belief に採用した向き（昇格しなかった/Engine でフィルタされたら `None`）。
    pub target: Option<bool>,
    /// `plan` に渡した `shadow_toggled`（書く決定をしたか）。
    pub shadow_toggled: bool,
    /// 押下後の belief。
    pub belief_after: bool,
    /// 押下後に IntentStore が持つ（フォーカス対象の）明示意図。
    pub intent_after: Option<bool>,
    /// この押下で `last_written_press` に予約した向き（押下 ID を持つ押下が order を起案したときだけ `Some`。
    /// 同一押下の後続の経路〈`explicit_press_delivery_after`〉の `claimed` に渡す。ADR-208 L1 D1）。
    pub reserved: Option<bool>,
}

/// INV-L1 を満たす決定（配送か書き込みの**ちょうど一方**）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// 物理キーを配送する（IME がキーの意味どおり処理することは前提 A1）。
    PassThrough,
    /// awase が書く。物理キーは `physical`（Suppress/Consume）で握る。
    Write { physical: Physical, open: bool },
}

/// INV-L1 の違反（反例）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Violation {
    /// 物理キーが届き、かつ awase も書く（二重 actuation）。
    Both { open: bool },
    /// 物理キーは届かず、awase も書かない（二重の空振り）。
    Neither(ElisionReason),
}

impl Delivery {
    /// INV-L1 の型への写像。
    ///
    /// # Errors
    ///
    /// 配送と書き込みが両方、またはどちらも無いとき `Violation`。
    pub const fn resolve(&self) -> Result<Resolution, Violation> {
        match (self.physical, self.write) {
            (Physical::Allow, None) => Ok(Resolution::PassThrough),
            (Physical::Allow, Some(open)) => Err(Violation::Both { open }),
            (physical, Some(open)) => Ok(Resolution::Write { physical, open }),
            (_, None) => Err(Violation::Neither(self.reason)),
        }
    }

    /// 書き込みを試みた結果の `ImeOpenOutcome`（完了の遷移モデル用。試みていなければ `None`）。
    #[must_use]
    pub const fn outcome(&self) -> Option<ImeOpenOutcome> {
        match self.reason {
            ElisionReason::Written => Some(ImeOpenOutcome::Applied),
            ElisionReason::AlreadyMatched => Some(ImeOpenOutcome::AlreadyMatched),
            // 書かなかった重複は「送っていない」outcome（本番の executor と同じ定数。`AlreadyMatched` だと applied が嘘の Confirmed になる）。
            ElisionReason::AlreadyWrittenThisPress => Some(DUPLICATE_OUTCOME),
            ElisionReason::Unwarranted => Some(ImeOpenOutcome::Unwarranted),
            ElisionReason::InputRelayNotOwned => Some(ImeOpenOutcome::NotOwned),
            ElisionReason::WinHeld => Some(ImeOpenOutcome::UnsafeToToggle),
            ElisionReason::NotPromoted | ElisionReason::ShadowNoop => None,
        }
    }
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
    /// 押下 ID を持つ order か（`ActuationOrder.press.is_some()`）。真なら授権は `issue_press_warrant`
    /// （`is_japanese_ime`・`target_known` を問わない、ADR-208 D2・D3）、偽なら `issue_open_warrant`。
    pub explicit_press: bool,
}

/// 授権の判定器。本番は live の `WarrantContext`、テストは合成ストアで `issue_open_warrant` を呼ぶ。
pub trait WarrantJudge {
    /// `issue_open_warrant(requested, ..)` が `Some`（授権あり）か。
    fn warranted(&self, req: &WarrantRequest) -> bool;
}

/// `plan` と shadow 判断の循環の解き方（ADR-208 決定2 D4）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryMode {
    /// 現状の本番: shadow が belief を倒した押下だけ書く（`plan` の入力 `shadow_toggled` はその結果）。
    Legacy,
    /// 固定点（D4、L3 で本番へ）: `plan(shadow_toggled=false)` を先に評価し、Suppress なら no-op でも書いて
    /// `shadow_toggled=true` として後段の `plan(true)` を評価する。押下 ID（L1）は含まない。
    FixedPoint,
    /// 押下 ID（ADR-208 L1、**L2 前**。`P1-PreL2` で L2 の効果を件数で比べるためだけに残す参考）: L1 の押下 ID（下の
    /// [`Self::PressId`] と同じ）に、L2 前の授権（`is_japanese_ime`・`current_focus` を問う `issue_open_warrant`）と
    /// shadow 昇格の `is_japanese_ime` 条件（0x19・0xF3/F4・F13）を残したもの。
    PressIdBeforeL2,
    /// 押下 ID（ADR-208 L1・L2、**現在の本番**）: 非リピートの押下（`was_down=false`）は `press=Some` として、(1) 書き込みの order の
    /// 発行直前に `PressLedger::claim` で予約し（同じ押下で同じ向きなら書かない・Engine は逆向きなら上書きする）、
    /// (2) view の `shadow_on` を `explicit_press_shadow_on` で未知にする（Engine 経路と shadow 経路の両方）。
    /// リピート（`was_down=true`）は `press=None`: 予約せず、`applied` の already-matched 省略のまま
    /// （shadow 経路も従来の無条件降格をやめる）。`plan` と shadow 判断の循環は Legacy のまま。
    PressId,
    /// L1 の押下 ID と D4 の固定点の両方（L3 で本番がこの形になる）。
    PressIdFixedPoint,
}

impl DeliveryMode {
    /// D4 の固定点を適用するか。
    #[must_use]
    pub const fn is_fixed_point(self) -> bool {
        matches!(self, Self::FixedPoint | Self::PressIdFixedPoint)
    }

    /// L1 の押下 ID を適用するか（`press=Some` の押下で予約と applied の未知化を行う）。
    #[must_use]
    pub const fn has_press_id(self) -> bool {
        matches!(
            self,
            Self::PressIdBeforeL2 | Self::PressId | Self::PressIdFixedPoint
        )
    }

    /// L2 の授権（押下の order は `issue_press_warrant`、`is_japanese_ime`・`current_focus` を問わない。D2・D3）と
    /// shadow 昇格の `is_japanese_ime` 条件の撤去（D2）を適用するか。
    #[must_use]
    pub const fn has_l2(self) -> bool {
        matches!(self, Self::PressId | Self::PressIdFixedPoint)
    }
}

/// ADR-208 L3a（D4）の段階: shadow の **no-op 分岐の書き込み**（belief が既に押下の向きと一致しているのに、物理が Suppress される窓で
/// 実 IME へ書く）を TsfNative の窓（WezTerm/Windows Terminal 等）へ適用するか。
///
/// TsfNative × GJI では、押下ごとに単発の `VK_IME_ON/OFF` が出ると BUG-124 型の「@」（WT × GJI × PSReadLine）を誘発しうる。
/// [`crate::state::ime_actuation_decision::ENGINE_PRESS_UNKNOWNS_APPLIED_IN_TSF_NATIVE`] と同じく、実機 A/B（L3'）の後で `true` にして解禁する。
pub(crate) const SHADOW_NOOP_WRITES_IN_TSF_NATIVE: bool = false;

/// shadow の no-op 分岐で書くか（D4 の固定点の手順2。`plan(shadow_toggled=false)` が Suppress の昇格した押下だけ）。
/// 本番の `kp_shadow_noop_write` と全列挙モデル（`DeliveryMode::PressIdFixedPoint`）が共有する。
/// `has_press` は非リピートの押下（押下 ID あり）。リピートは従来どおり no-op では書かない。
#[must_use]
pub(crate) const fn shadow_noop_write_wanted(
    has_press: bool,
    effectively_tsf_native: bool,
    plan0_suppress: bool,
) -> bool {
    has_press && plan0_suppress && (!effectively_tsf_native || SHADOW_NOOP_WRITES_IN_TSF_NATIVE)
}

// ── 決定関数 ─────────────────────────────────────────────────────────────────

/// 全列挙モデルが 1 押下に付ける押下 ID（値に意味は無い。`PressLedger` は ID の等価だけを見る）。
const MODEL_PRESS: PressId = PressId::new(1);

/// 書き込みの試行（gate → 押下の予約 → 授権 → 先頭機構の already-matched 判定）。`shadow_on` は view の
/// `ControlLog.shadow_on`。`claim` は Engine 経路の押下の予約（本番の `executor::dispatch_ime_set_open` は gate の後・
/// order の発行前に `claim_press_write` を呼ぶ。shadow 経路は `kp_shadow_actuate` の冒頭で gate より前に予約するので
/// ここには渡さない）。
fn attempt_write(
    state: &PressState,
    open: bool,
    shadow_on: Option<bool>,
    req: WarrantRequest,
    judge: &impl WarrantJudge,
    claim: Option<(&mut PressLedger, PressSource)>,
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
    if let Some((ledger, source)) = claim {
        if !ledger.claim(Some(MODEL_PRESS), open, source).writes() {
            return (None, ElisionReason::AlreadyWrittenThisPress);
        }
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
    mode: DeliveryMode,
) -> Delivery {
    explicit_press_delivery_after(state, key, judge, mode, None)
}

/// [`explicit_press_delivery_with`] に、同じ押下で先に予約された向き（`claimed`）を渡す版（ADR-208 L1 D1）。
///
/// 同一押下に shadow と Engine の 2 経路が来る構成（sync キーが `keys.ime_on/off` にも割り当てられている
/// 等、BUG-113）で、後から来る経路の判断を前の経路の予約から決める。`claimed` は `mode.has_press_id()` かつ非リピート
/// （押下 ID を持つ）ときだけ意味を持つ。
///
/// # Panics
///
/// 起きない（`explicit_press_delivery_with` と同じ）。
#[must_use]
pub fn explicit_press_delivery_after(
    state: &PressState,
    key: ExplicitKey,
    judge: &impl WarrantJudge,
    mode: DeliveryMode,
    claimed: Option<bool>,
) -> Delivery {
    // 押下 ID を持つ押下か（L1）。自動リピートの Down はフックが `press_id=None` にする。
    let has_press = mode.has_press_id() && !state.was_down;
    let mut ledger = PressLedger::default();
    if let (true, Some(open)) = (has_press, claimed) {
        ledger.claim(Some(MODEL_PRESS), open, PressSource::Shadow);
    }
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
                explicit_press: has_press && mode.has_l2(),
            };
            // executor は `applied_snapshot` を渡す。L1（押下 ID あり）は `applied` が向きと一致していても未知にして
            // already-matched 省略を外す（D1。`explicit_press_applied_pair` と同じ `explicit_press_shadow_on`）。
            let shadow_on = if has_press
                && engine_press_unknowns_applied(state.profile.is_effectively_tsf_native())
            {
                explicit_press_shadow_on(state.applied.open(), target)
            } else {
                state.applied.open()
            };
            let (write, reason) = attempt_write(
                state,
                target,
                shadow_on,
                req,
                judge,
                has_press.then_some((&mut ledger, PressSource::Engine)),
            );
            Delivery {
                physical: Physical::Consume,
                write,
                reason,
                requested: Some(target),
                target: (!filtered).then_some(target),
                shadow_toggled: false,
                belief_after: if filtered { state.belief_open } else { target },
                intent_after: intent,
                reserved: has_press
                    .then(|| ledger.last_written().map(|(_, open)| open))
                    .flatten(),
            }
        }
        PressPath::Shadow => {
            // L2 M-1: 未同定かつ `is_japanese_ime` 偽の 0x19 は受動（`enrich_key_role`）。物理は `plan` が素通しにする。
            let kanji_passive = mode.has_l2()
                && key == ExplicitKey::Kanji
                && crate::state::key_effect_runtime::kanji_passive_when_unidentified(
                    state.ime_identified,
                    is_japanese_ime,
                );
            let event = key
                .event_for(state.was_down, kanji_passive)
                .expect("Shadow 経路のキーは必ず RawKeyEvent を持つ");
            let intent_kind = select_shadow_intent(&event, is_japanese_ime, state.ime_identified)
                .filter(|(_, kind)| {
                    // L2 前（参考）: `shadow_action` 由来の昇格（0x19・0xF3/F4・F13）は `is_japanese_ime` が真のときだけ。
                    mode.has_l2()
                        || is_japanese_ime
                        || *kind == ShadowIntentKind::SyncKey
                        || crate::vk::is_static_idempotent_open_key(event.vk_code)
                });
            let resolved = intent_kind.map(|(action, _)| action.resolve(state.belief_open));
            let flips = resolved.is_some_and(|new_val| new_val != state.belief_open);
            let app_profile = state.profile.app_profile();
            // 固定点の手順（D4）: 1. plan(shadow_toggled=false) を評価する。
            let plan0 =
                PhysicalKeyDisposition::plan_core(&event, app_profile, false, state.ime_kind);
            // 2. 書く決定。Legacy は belief が倒れたときだけ。FixedPoint は昇格した押下で plan0 が Suppress なら
            //    no-op でも書く（Suppress した物理キーに誰も応答しない二重の空振りを避ける）。
            let plan0_suppress = plan0 == PhysicalKeyDisposition::Suppress;
            let noop_write = if mode.has_press_id() {
                // 本番（L3a）と同じ判断を共有する（押下 ID あり・TsfNative は L3' まで除外）。
                mode.is_fixed_point()
                    && shadow_noop_write_wanted(
                        has_press,
                        state.profile.is_effectively_tsf_native(),
                        plan0_suppress,
                    )
            } else {
                mode.is_fixed_point() && plan0_suppress
            };
            let write_wanted = flips || (resolved.is_some() && noop_write);
            // 3. 後段の plan(shadow_toggled=write_wanted)。
            let physical = match PhysicalKeyDisposition::plan_core(
                &event,
                app_profile,
                write_wanted,
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
                    requested: None,
                    target: None,
                    shadow_toggled: write_wanted,
                    belief_after,
                    intent_after: state.intent,
                    reserved: None,
                };
            };
            // 昇格した押下は `write_physical_key`/`write_sync_key` が意図を記録する（フォーカス不明なら no-op）。
            let intent_after = state.current_focus_known.then_some(new_val);
            if !write_wanted {
                return Delivery {
                    physical,
                    write: None,
                    reason: ElisionReason::ShadowNoop,
                    requested: None,
                    target: Some(new_val),
                    shadow_toggled: false,
                    belief_after,
                    intent_after,
                    reserved: None,
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
                explicit_press: has_press && mode.has_l2(),
            };
            // `kp_shadow_actuate` の冒頭: order の発行前に押下を予約する（gate より前）。同じ押下で既に同じ向き
            // （または先着の Engine の逆向き）を予約済みなら書かない（BUG-113）。リピート（`press=None`）は予約しない。
            if has_press {
                let claim = ledger.claim(Some(MODEL_PRESS), new_val, PressSource::Shadow);
                if !claim.writes() {
                    return Delivery {
                        physical,
                        write: None,
                        reason: ElisionReason::AlreadyWrittenThisPress,
                        requested: Some(new_val),
                        target: Some(new_val),
                        shadow_toggled: write_wanted,
                        belief_after,
                        intent_after,
                        reserved: ledger.last_written().map(|(_, open)| open),
                    };
                }
            }
            // `kp_shadow_actuate`: `applied` が向きと一致するなら view の `shadow_on` を未知にする（M1、PR #408）。
            // L1 では押下 ID を持つ押下だけ（リピートは従来の `applied` の already-matched 省略。`explicit_press_applied_pair`）。
            let applied_open = state.applied.open();
            let shadow_on = if mode.has_press_id() && state.was_down {
                applied_open
            } else {
                explicit_press_shadow_on(applied_open, new_val)
            };
            let (write, reason) = attempt_write(state, new_val, shadow_on, req, judge, None);
            Delivery {
                physical,
                write,
                reason,
                requested: Some(new_val),
                target: Some(new_val),
                shadow_toggled: write_wanted,
                belief_after,
                intent_after,
                reserved: has_press
                    .then(|| ledger.last_written().map(|(_, open)| open))
                    .flatten(),
            }
        }
    }
}

/// 同一押下で shadow 経路と Engine の `SetOpen` の両方が来る構成の書き込み（**L0 の現状、押下 ID なし**）。
///
/// sync キーが `keys.ime_on/off` にも割り当てられている等。Engine 側の executor は、押下前の `applied` を見ると仮定する
/// （完了の反映は押下の処理後）。戻り値は `[shadow の write, Engine の write]`。
#[must_use]
pub fn dual_route_writes(
    state: &PressState,
    shadow_key: ExplicitKey,
    engine_key: ExplicitKey,
    judge: &impl WarrantJudge,
) -> [Option<bool>; 2] {
    dual_route_writes_ledger_only(state, shadow_key, engine_key, judge, DeliveryMode::Legacy)
}

/// [`dual_route_writes`] の `mode` 指定版で、**予約（`PressLedger`）だけ**で二重送信を防ぐ防御線の評価
/// （Engine への静的な事前問い合わせ〈`dual_route_writes_with`〉が効かなかったときに残る分岐）。
///
/// L1 では、shadow 経路が order を発行した時点で予約した向きを Engine 経路に渡す（`Delivery::reserved` →
/// `explicit_press_delivery_after` の `claimed`）。**同期**の書き込み（先頭機構が ImmCross でない）が何も送らなかった
/// （`outcome_sent_nothing`）ときは予約を解く（本番の `release_press_write`。PR #419 Opus M-2）。非同期（ImmCross 先頭）は解けない。
#[must_use]
pub fn dual_route_writes_ledger_only(
    state: &PressState,
    shadow_key: ExplicitKey,
    engine_key: ExplicitKey,
    judge: &impl WarrantJudge,
    mode: DeliveryMode,
) -> [Option<bool>; 2] {
    let d_shadow = explicit_press_delivery_with(state, shadow_key, judge, mode);
    let after = state_after_press(state, shadow_key, &d_shadow);
    let engine_state = PressState {
        applied: state.applied,
        ..after
    };
    let claimed = reservation_after_route(state, &d_shadow);
    let d_engine = explicit_press_delivery_after(&engine_state, engine_key, judge, mode, claimed);
    [d_shadow.write, d_engine.write]
}

/// ある経路の決定 `d` の後に、同じ押下の次の経路へ残る予約の向き。同期の書き込み（先頭機構が ImmCross でない）が何も
/// 送らなかった（`outcome_sent_nothing`）なら予約を解いて `None`（本番の `release_press_write`）、それ以外は `d.reserved`。
#[must_use]
pub fn reservation_after_route(state: &PressState, d: &Delivery) -> Option<bool> {
    let released =
        !state.chain_head_is_imm_cross() && d.outcome().is_some_and(outcome_sent_nothing);
    if released {
        None
    } else {
        d.reserved
    }
}

/// [`dual_route_writes_ledger_only`] に Engine への静的な事前問い合わせを加えた本番の評価。
///
/// 事前問い合わせは `Engine::matches_ime_set_open`（PR #419 Opus M-4）。Engine が同じ打鍵の `SetOpen` を出すキーでは shadow は昇格も書き込みもせず、Engine だけが書く
/// （`[None, Engine の write]`）。リピート（押下 ID なし）の従来の挙動は L0 のまま。
#[must_use]
pub fn dual_route_writes_with(
    state: &PressState,
    shadow_key: ExplicitKey,
    engine_key: ExplicitKey,
    judge: &impl WarrantJudge,
    mode: DeliveryMode,
) -> [Option<bool>; 2] {
    if !mode.has_press_id() {
        return dual_route_writes_ledger_only(state, shadow_key, engine_key, judge, mode);
    }
    let _ = shadow_key; // shadow は Engine が担うキーでは何もしない（belief も触らない）。
    let d_engine = explicit_press_delivery_after(state, engine_key, judge, mode, None);
    [None, d_engine.write]
}

// ── 状態遷移モデル ────────────────────────────────────────────────────────────

/// 押下後の実 IME の open 状態（モデルの前提はモジュール doc 参照。物理キーの処理は前提 A1 のキーだけ。InputRelay は中継先の IME が処理するので全キー成立とみなす）。
#[must_use]
pub fn ime_after_press(
    real_open: bool,
    key: ExplicitKey,
    profile: PressProfile,
    d: &Delivery,
) -> bool {
    let mut r = real_open;
    // 配送した物理キーを IME が意味どおり処理するのは前提 A1 が成り立つキーだけ（それ以外は何も起きないとみなす）。
    if d.physical == Physical::Allow && (key.a1_holds() || profile == PressProfile::InputRelay) {
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

const fn model_applied(a: AppliedKnowledge) -> AppliedImeState {
    match a {
        AppliedKnowledge::Unknown => AppliedImeState::Unknown,
        AppliedKnowledge::Optimistic(v) => AppliedImeState::Optimistic(v),
        AppliedKnowledge::Confirmed(v) => AppliedImeState::Confirmed { open: v, at_ms: 1 },
    }
}

const fn knowledge_of(a: AppliedImeState) -> AppliedKnowledge {
    match a {
        AppliedImeState::Unknown => AppliedKnowledge::Unknown,
        AppliedImeState::Optimistic(v) => AppliedKnowledge::Optimistic(v),
        AppliedImeState::Confirmed { open, .. } => AppliedKnowledge::Confirmed(open),
    }
}

/// 書き込みの完了後の `applied`。**手で模倣せず実物の遷移を通す**: shadow 経路（generation なし）は
/// `apply_result_effective_open`（`record_ime_apply_result` の generation=None 分岐の純粋部）と
/// `ImeModel::confirm_applied`、Engine 経路（generation あり）は `ImeModel::apply_engine_request_and_completion`（`reduce` の
/// `ImeApplyRequested` → `ImeEvent::from_apply_outcome`（`completion_can_update_applied` を含む）。
fn applied_after(state: &PressState, key: ExplicitKey, d: Delivery) -> AppliedKnowledge {
    let (Some(outcome), Some(open)) = (d.outcome(), d.requested) else {
        return state.applied;
    };
    // shadow 経路が同じ押下の予約済みで書かなかった場合は、完了（`on_ime_apply_complete`）を呼ばない（`applied` は不変）。
    // Engine 経路は `AlreadyMatched` を返して完了へ流す（`applied` は向きに Confirmed）。
    if d.reason == ElisionReason::AlreadyWrittenThisPress && matches!(key.path(), PressPath::Shadow)
    {
        return state.applied;
    }
    let mut oracle = ImeModel::with_applied(model_applied(state.applied));
    match key.path() {
        PressPath::Shadow => {
            if let Some(effective) = apply_result_effective_open(open, outcome) {
                oracle.confirm_applied(effective, 2);
            }
        }
        PressPath::Engine(_) => {
            let generation = ApplyGeneration::new(1).expect("1 は非ゼロ");
            oracle.apply_engine_request_and_completion(open, outcome, generation);
        }
    }
    knowledge_of(oracle.applied)
}

/// 押下後の内部状態（belief・applied・is_japanese_ime・IntentStore・candidate_was_seen）。
#[must_use]
pub fn state_after_press(state: &PressState, key: ExplicitKey, d: &Delivery) -> PressState {
    PressState {
        belief_open: d.belief_after,
        is_japanese_ime: state.is_japanese_ime || key.upgrades_is_japanese(),
        intent: d.intent_after,
        applied: applied_after(state, key, *d),
        // GjiDirect の OFF 送信は candidate_was_seen を消費する（ADR-171）。
        candidate_was_seen: state.candidate_was_seen
            && !(d.write == Some(false) && state.ime_kind == ImeKindId::Gji),
        ..*state
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::key_sequence_policy;
    use crate::vk::VkCodeExt as _;

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
            ime_identified: true,
            current_focus_known: true,
            actuating_obs: None,
            intent: None,
            candidate_was_seen: false,
            ctrl_chord: false,
            win_held: false,
            was_down: false,
        }
    }

    #[test]
    fn state_space_size_is_pinned() {
        // belief 2 × applied 5 × japanese 2 × profile 7 × (kind, 同定) 3 × focus 2 × obs 3 × intent 3 ×
        // candidate 2 × chord 2 × win 2 × was_down 2
        assert_eq!(
            PressState::all().count(),
            2 * 5 * 2 * 7 * 3 * 2 * 3 * 3 * 2 * 2 * 2 * 2
        );
        assert_eq!(ExplicitKey::ALL.len(), 12);
    }

    #[test]
    fn engine_on_in_gji_blind_is_elided_when_applied_matches() {
        // S-1: Engine 経由の絶対キーは `applied` が向きと一致すると GjiDirect の already-matched で省略される。
        let mut s = base();
        s.applied = AppliedKnowledge::Confirmed(true);
        let d = explicit_press_delivery_with(
            &s,
            ExplicitKey::EngineOn,
            &Always(true),
            DeliveryMode::Legacy,
        );
        assert_eq!(d.physical, Physical::Consume);
        assert_eq!(d.write, None);
        assert_eq!(d.reason, ElisionReason::AlreadyMatched);
    }

    #[test]
    fn shadow_toggle_demotes_applied_so_the_write_is_not_elided() {
        // #408: shadow 経路は同じ状態でも降格して書く。
        let mut s = base();
        s.applied = AppliedKnowledge::Confirmed(true);
        let d = explicit_press_delivery_with(
            &s,
            ExplicitKey::StaticOn,
            &Always(true),
            DeliveryMode::Legacy,
        );
        assert_eq!(d.physical, Physical::Suppress);
        assert_eq!(d.write, Some(true));
        assert_eq!(d.reason, ElisionReason::Written);
    }

    #[test]
    fn static_open_keys_are_promoted_even_when_not_japanese() {
        // ADR-207: 0x16/0x1A は is_japanese_ime を問わず昇格する。
        let mut s = base();
        s.is_japanese_ime = false;
        let d = explicit_press_delivery_with(
            &s,
            ExplicitKey::StaticOn,
            &Always(true),
            DeliveryMode::Legacy,
        );
        assert!(d.shadow_toggled);
        // 漢字(0x19)は L0（Legacy）では is_japanese_ime が偽だと昇格せず（IC で固着した、L-4）、L2 の押下 ID あり
        // （ADR-208 D2）では is_japanese_ime を問わず昇格する。
        let d = explicit_press_delivery_with(
            &s,
            ExplicitKey::Kanji,
            &Always(true),
            DeliveryMode::Legacy,
        );
        assert_eq!(d.reason, ElisionReason::NotPromoted);
        let d = explicit_press_delivery_with(
            &s,
            ExplicitKey::Kanji,
            &Always(true),
            DeliveryMode::PressId,
        );
        assert!(d.shadow_toggled);
        assert_eq!(d.write, Some(true));
    }

    #[test]
    fn hz_key_upgrades_is_japanese_ime_before_promotion() {
        // ADR-093: 0xF3/0xF4 の物理受信は is_japanese_ime を上げてから昇格判定する。
        let mut s = base();
        s.is_japanese_ime = false;
        let d = explicit_press_delivery_with(
            &s,
            ExplicitKey::HzToggle,
            &Always(true),
            DeliveryMode::Legacy,
        );
        assert!(d.shadow_toggled);
        assert!(state_after_press(&s, ExplicitKey::HzToggle, &d).is_japanese_ime);
    }

    #[test]
    fn chord_filtered_engine_off_leaves_belief_and_intent_but_still_attempts_the_write() {
        let mut s = base();
        s.belief_open = true;
        s.ctrl_chord = true;
        s.intent = Some(true);
        let d = explicit_press_delivery_with(
            &s,
            ExplicitKey::EngineOff,
            &Always(true),
            DeliveryMode::Legacy,
        );
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
        let d = explicit_press_delivery_with(
            &s,
            ExplicitKey::EngineOn,
            &Always(true),
            DeliveryMode::Legacy,
        );
        assert_eq!(d.physical, Physical::Consume);
        assert_eq!(d.reason, ElisionReason::InputRelayNotOwned);
        // shadow 経路のキーは Allow（`plan` が InputRelay を常に Allow にする）。
        let d = explicit_press_delivery_with(
            &s,
            ExplicitKey::StaticOn,
            &Always(true),
            DeliveryMode::Legacy,
        );
        assert_eq!(d.physical, Physical::Allow);
    }

    #[test]
    fn select_shadow_intent_matches_the_pre_extraction_branches() {
        // 切り出し前の `kp_stage_shadow_ime_toggle` の 3 分岐を、キー種別ごとに固定する（L2: 非リピートの
        // `shadow_action` 昇格は `is_japanese_ime` を問わない）。
        let ev = |k: ExplicitKey| k.event(false).expect("event");
        // 同期キーは is_japanese_ime を問わず SyncKey。
        for jp in [false, true] {
            assert_eq!(
                select_shadow_intent(&ev(ExplicitKey::SyncOn), jp, false),
                Some((ShadowImeAction::TurnOn, ShadowIntentKind::SyncKey))
            );
        }
        // 0x16/0x1A は is_japanese_ime を問わず PhysicalImeKey。
        for jp in [false, true] {
            assert_eq!(
                select_shadow_intent(&ev(ExplicitKey::StaticOff), jp, false),
                Some((ShadowImeAction::TurnOff, ShadowIntentKind::PhysicalImeKey))
            );
        }
        // 漢字・半角/全角・F13 の非リピート押下も is_japanese_ime を問わず昇格する（ADR-208 D2）。
        for k in [
            ExplicitKey::Kanji,
            ExplicitKey::HzToggle,
            ExplicitKey::RoleFkeyToggle,
        ] {
            for jp in [false, true] {
                // 0x19 は TIP 同定済みのときだけ（所有者決定）。他は同定を問わない。
                assert_eq!(
                    select_shadow_intent(&ev(k), jp, true),
                    Some((ShadowImeAction::Toggle, ShadowIntentKind::PhysicalImeKey)),
                    "{k:?} jp={jp}"
                );
                let unidentified = select_shadow_intent(&ev(k), jp, false);
                if k == ExplicitKey::Kanji && !jp {
                    assert_eq!(unidentified, None, "0x19 は未同定・非日本語では昇格しない");
                } else {
                    assert!(unidentified.is_some(), "{k:?} jp={jp}");
                }
            }
        }
        // リピート（押下 ID なし）は従来どおり is_japanese_ime が真のときだけ（偽で昇格すると belief だけ反転し、
        // 授権が下りず書かれない食い違いを作る）。
        let rep = |k: ExplicitKey| {
            let mut e = ev(k);
            e.was_down = true;
            e
        };
        assert_eq!(
            select_shadow_intent(&rep(ExplicitKey::Kanji), false, true),
            None
        );
        assert!(select_shadow_intent(&rep(ExplicitKey::Kanji), true, false).is_some());
        // F13 の自動リピート Down は昇格しない。
        assert_eq!(
            select_shadow_intent(&rep(ExplicitKey::RoleFkeyToggle), true, true),
            None
        );
        // 同じリピートでも 0xF3 は昇格する（0xF3/0xF4・0x19 の挙動は変えない）。
        assert!(select_shadow_intent(&rep(ExplicitKey::HzToggle), true, true).is_some());
        // 意図を持たないキーは常に None。
        for k in [ExplicitKey::PhysOnlyMode, ExplicitKey::ThumbPlain] {
            assert_eq!(select_shadow_intent(&ev(k), true, true), None, "{k:?}");
        }
    }

    #[test]
    fn engine_chord_filter_only_drops_off() {
        assert!(engine_set_open_filtered_by_chord(true, false));
        assert!(!engine_set_open_filtered_by_chord(true, true));
        assert!(!engine_set_open_filtered_by_chord(false, false));
        assert!(!engine_set_open_filtered_by_chord(false, true));
    }

    /// 旧 `plan` + 旧 shadow 判断（ADR-208 L0 の切り出し前の合成。`shadow_toggled` を belief が倒れたかで決め、
    /// `plan` を 1 回だけ評価する）の出力。新しい `explicit_press_delivery_with(.., Legacy)` と同じ全列挙空間で比べる。
    #[derive(Debug, PartialEq, Eq)]
    struct RefOut {
        physical: Physical,
        write: Option<bool>,
        reason: ElisionReason,
        target: Option<bool>,
        shadow_toggled: bool,
        belief_after: bool,
        intent_after: Option<bool>,
    }

    /// 要求の各ビットから決まる擬似判定器（授権が下りる/下りないを偏りなく混ぜる）。
    struct Hashed;
    impl WarrantJudge for Hashed {
        fn warranted(&self, r: &WarrantRequest) -> bool {
            let bits = u32::from(r.requested)
                | u32::from(r.target_known) << 1
                | u32::from(r.is_japanese_ime) << 2
                | u32::from(r.desired_open) << 3
                | r.intent.map_or(0, |v| 1 + u32::from(v)) << 4
                | r.actuating_obs.map_or(0, |v| 1 + u32::from(v)) << 6;
            (bits.wrapping_mul(2_654_435_761) >> 7) & 1 == 1
        }
    }

    fn legacy_reference(state: &PressState, key: ExplicitKey, judge: &impl WarrantJudge) -> RefOut {
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
                    explicit_press: false,
                };
                // executor は `applied_snapshot` をそのまま渡す（shadow 経路の降格は無い）。
                let (write, reason) =
                    attempt_write(state, target, state.applied.open(), req, judge, None);
                RefOut {
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
                    .event(state.was_down)
                    .expect("Shadow 経路のキーは必ず RawKeyEvent を持つ");
                let intent_kind = legacy_select_shadow_intent(&event, is_japanese_ime);
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
                    return RefOut {
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
                    return RefOut {
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
                    explicit_press: false,
                };
                // `kp_shadow_actuate`: `applied` が向きと一致するなら view の `shadow_on` を未知にする（M1、PR #408）。
                let applied_open = state.applied.open();
                let shadow_on = explicit_press_shadow_on(applied_open, new_val);
                let (write, reason) = attempt_write(state, new_val, shadow_on, req, judge, None);
                RefOut {
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

    /// L0 の「挙動を変えない」の機械的な確認: 旧 `plan` + 旧 shadow 判断の合成と、新 `explicit_press_delivery_with(.., Legacy)`
    /// を、全列挙空間（状態 120,960 × キー 12 × 3 種の判定器）で比べて差分 0。
    #[test]
    fn legacy_mode_matches_the_pre_extraction_composition_everywhere() {
        let judges: [&dyn Fn(&PressState, ExplicitKey) -> (RefOut, Delivery); 3] = [
            &|s, k| {
                (
                    legacy_reference(s, k, &Always(true)),
                    explicit_press_delivery_with(s, k, &Always(true), DeliveryMode::Legacy),
                )
            },
            &|s, k| {
                (
                    legacy_reference(s, k, &Always(false)),
                    explicit_press_delivery_with(s, k, &Always(false), DeliveryMode::Legacy),
                )
            },
            &|s, k| {
                (
                    legacy_reference(s, k, &Hashed),
                    explicit_press_delivery_with(s, k, &Hashed, DeliveryMode::Legacy),
                )
            },
        ];
        let mut checked = 0u64;
        for s in PressState::all() {
            for key in ExplicitKey::ALL {
                for judge in judges {
                    let (old, new) = judge(&s, key);
                    let new = RefOut {
                        physical: new.physical,
                        write: new.write,
                        reason: new.reason,
                        target: new.target,
                        shadow_toggled: new.shadow_toggled,
                        belief_after: new.belief_after,
                        intent_after: new.intent_after,
                    };
                    assert_eq!(old, new, "{key:?} {s:?}");
                    checked += 1;
                }
            }
        }
        assert_eq!(checked, 120_960 * 12 * 3);
    }

    /// 固定点（D4）: Legacy との差は「昇格した no-op で plan(false) が Suppress の押下が書く」ことだけ。
    /// その結果 S-3（shadow no-op で Suppress のみ）は 0 件になり、Allow の no-op は書かない（INV-L1）。
    #[test]
    fn fixed_point_differs_only_by_writing_suppressed_noops() {
        for s in PressState::all() {
            for key in ExplicitKey::ALL {
                let old = explicit_press_delivery_with(&s, key, &Hashed, DeliveryMode::Legacy);
                let new = explicit_press_delivery_with(&s, key, &Hashed, DeliveryMode::FixedPoint);
                if old.reason == ElisionReason::ShadowNoop && old.physical == Physical::Suppress {
                    assert_ne!(new.reason, ElisionReason::ShadowNoop, "{key:?} {s:?}");
                    assert_eq!(new.physical, Physical::Suppress, "{key:?} {s:?}");
                } else if old.reason == ElisionReason::ShadowNoop {
                    // Allow の no-op は書かない（物理が届く）。
                    assert_eq!(old, new, "{key:?} {s:?}");
                } else {
                    assert_eq!(old, new, "{key:?} {s:?}");
                }
            }
        }
    }

    #[test]
    fn resolve_maps_inv_l1() {
        let d = |physical, write| Delivery {
            physical,
            write,
            reason: ElisionReason::Written,
            requested: None,
            target: None,
            shadow_toggled: false,
            belief_after: false,
            intent_after: None,
            reserved: None,
        };
        assert_eq!(
            d(Physical::Allow, None).resolve(),
            Ok(Resolution::PassThrough)
        );
        assert_eq!(
            d(Physical::Suppress, Some(true)).resolve(),
            Ok(Resolution::Write {
                physical: Physical::Suppress,
                open: true
            })
        );
        assert_eq!(
            d(Physical::Allow, Some(false)).resolve(),
            Err(Violation::Both { open: false })
        );
        assert!(matches!(
            d(Physical::Consume, None).resolve(),
            Err(Violation::Neither(_))
        ));
    }

    #[test]
    fn completion_transition_goes_through_the_real_model() {
        // Engine 経路: generation あり。reduce の完了受理（pending 一致）で Confirmed になる。
        let mut s = base();
        s.applied = AppliedKnowledge::Optimistic(false);
        let d = explicit_press_delivery_with(
            &s,
            ExplicitKey::EngineOn,
            &Always(true),
            DeliveryMode::Legacy,
        );
        assert_eq!(
            state_after_press(&s, ExplicitKey::EngineOn, &d).applied,
            AppliedKnowledge::Confirmed(true)
        );
        // 授権が下りない押下は完了で applied を動かさない（NotSent）。
        let d = explicit_press_delivery_with(
            &s,
            ExplicitKey::EngineOn,
            &Always(false),
            DeliveryMode::Legacy,
        );
        assert_eq!(
            state_after_press(&s, ExplicitKey::EngineOn, &d).applied,
            s.applied
        );
        // shadow 経路: generation なし。
        let d = explicit_press_delivery_with(
            &s,
            ExplicitKey::StaticOn,
            &Always(true),
            DeliveryMode::Legacy,
        );
        assert_eq!(
            state_after_press(&s, ExplicitKey::StaticOn, &d).applied,
            AppliedKnowledge::Confirmed(true)
        );
    }

    #[test]
    fn apply_result_effective_open_is_the_record_ime_apply_result_table() {
        use ImeOpenOutcome as O;
        for open in [false, true] {
            for o in [O::Applied, O::AppliedWithoutSendInput, O::AlreadyMatched] {
                assert_eq!(apply_result_effective_open(open, o), Some(open));
            }
            assert_eq!(apply_result_effective_open(open, O::Failed), Some(!open));
            for o in [O::UnsafeToToggle, O::NotOwned, O::Unwarranted] {
                assert_eq!(apply_result_effective_open(open, o), None);
            }
        }
    }

    // ── 逐語コピー（L0 限定のテスト用の意図的な重複。L1 以降で削除してよい）──────────────────────
    //
    // 出所: develop 2481948d（ADR-208 L0 の切り出し前）の `crates/awase-windows/src/runtime/transport.rs`
    // （`thumb_or_role_fkey_disposition`・`plan`・`is_role_toggle_hz_key_down`）と
    // `crates/awase-windows/src/runtime/key_pipeline.rs::kp_stage_shadow_ime_toggle` の intent 選択（951〜973 行）。
    // 本体は逐語。差は (1) 名前に `legacy_` を付けた、(2) `plan` の `ActiveImeKind` 引数を（`ImeKindId` へ変換済みの）
    // `ImeKindId` にし、冒頭の `.into()` 1 行を除いた、(3) intent 選択の `self.platform_state.ime.belief.is_japanese_ime()`
    // を引数 `is_japanese_ime` にし、`IntentKind` を `ShadowIntentKind` にした、の機械的な置換だけ。
    // doc コメントは落とした。新実装（`plan_core`・`select_shadow_intent`・`engine_set_open_filtered_by_chord`）との
    // 差分 0 を `new_implementations_match_the_verbatim_legacy_copies` が全列挙で確かめる。

    impl PhysicalKeyDisposition {
        fn legacy_thumb_or_role_fkey_disposition(
            event: &RawKeyEvent,
            shadow_toggled: bool,
        ) -> Option<Self> {
            let suppress = if matches!(
                event.vk_code,
                crate::vk::VK_CONVERT | crate::vk::VK_NONCONVERT
            ) {
                false
            } else if crate::vk::is_role_fkey(event.vk_code) {
                let first_down = event.event_type == KeyEventType::KeyDown && !event.was_down;
                // 役割由来の昇格（`shadow_action` あり）で書いたときだけ。同期キー（`keys.ime_detect`）由来の
                // `shadow_toggled` では書いたことにしない（`shadow_action` は付かない、Opus レビュー PR #328）。
                let role_action = event.ime_relevance.shadow_action.is_some();
                if first_down {
                    shadow_toggled && role_action
                } else {
                    role_action
                }
            } else {
                return None;
            };
            Some(if suppress {
                Self::Suppress
            } else {
                Self::Allow
            })
        }

        /// 物理キーを OS に届けるかどうかの純粋関数。
        ///
        /// **F2 (VK_DBE_HIRAGANA)**: 常に Allow（BUG-173）。以前は TSF mode かつ
        /// `f2_warmup_owned=true`（GJI 戦略）で Suppress していたが、ADR-100 決定2 で
        /// warmup が `VK_IME_ON` 単発になり「代わりに F2 を再送する」契約が崩れていた。
        /// 詳細は下の F2 分岐のコメント参照。
        ///
        /// **KANJI 関連キー**:
        /// - ImmCross プロファイル: Down/Up 共に Suppress（spurious 連鎖を構造的に遮断）
        /// - それ以外（Imm32Unavailable / TsfNative）: `apply-ime` が `GjiDirectStrategy` /
        ///   `MsImeDirectStrategy` で実際に actuate する場合（`ime_actuation_owned`）のみ、
        ///   shadow_toggle 発火時 KeyDown と全 KeyUp を Suppress。
        ///   **例外: 半角/全角（0xF3 SBCSCHAR / 0xF4 DBCSCHAR。0xF2 HIRAGANA は上の専用分岐で
        ///   別処理）のうち、awase が beliefに基づく開閉トグルとして書くキー
        ///   （`Runtime::enrich_key_role` が役割から `Some(Toggle)` を付けたもの、ADR-199 決定8。GJI は `config1.db` から逆算、MS-IME本体は仕様固定。ただし採用中の学習表が
        ///   半角/全角を開閉トグルでないと示すと`shadow_action`が付かず、この分岐の前に Down/Up とも Allow、ADR-195追記）の
        ///   KeyDown は `shadow_toggled` に関わらず常に Suppress**（`ime_actuation_owned`
        ///   の場合）。NICOLA の物理「IME ON」キー（scan 0x70）は、IME が既に目的の状態に
        ///   ある時に押されると `VK_DBE_HIRAGANA` (0xF2) の代わりに `VK_DBE_*` を生成する
        ///   ことがあり、素通しすると awase が書く開閉に加えて実 IME が同じキーを能動的に
        ///   処理する二重 actuation になる（2026-08-05 実機、BUG-46/BUG-52）。
        ///   **ADR-191（撤去後）**: awase が書かない英数(0xF0)・カタカナ(0xF1)・ひらがな(0xF2)
        ///   などは Suppress せず OS（IME）へ素通しする（`shadow_action` を持たないので
        ///   `is_kanji_event` 判定で Allow）。BUG-116/ADR-137 の「Shift+0xF1 だけ Allow」の
        ///   特例は、0xF1 が常に Allow になったため撤去した。
        ///
        /// `ime_actuation_owned` を profile 単独ではなく `ActiveImeKind` からも導出するのは、
        /// TsfNative（Windows Terminal 等）で GJI が起動している場合に awase 自身の
        /// `SendInput(VK_IME_ON/OFF)`（`GjiDirectStrategy`）と、素通しされた元の物理 KANJI 系
        /// キーの reinject が **二重に actuate** してしまうため（BUG-46）。旧実装は
        /// `profile.should_pass_physical_key()`（TsfNative で常に true）のみで判定しており、
        /// 「TSF が KANJI を正しく処理する」という前提が `GjiDirectStrategy` の全プロファイル
        /// 適用化（`ime_controller.rs`）より前のまま残っていたことが原因だった。
        fn legacy_plan(
            event: &RawKeyEvent,
            profile: AppImeProfile,
            shadow_toggled: bool,
            kind: ImeKindId,
        ) -> Self {
            // InputRelay: この窓は入力面ではなく、awase は actuation を所有しない
            // （issue #136 / BUG-90 決定4）。物理 IME キーは常に Allow。
            if profile == AppImeProfile::InputRelay {
                return Self::Allow;
            }

            // F2 (VK_DBE_HIRAGANA): 常に Allow（BUG-173）。
            //
            // 旧実装は「TSF mode かつ GJI 戦略（`f2_warmup_owned`）なら Suppress」だった。この
            // Suppress は「awase 自身が warmup として物理 F2 の代わりに SendInput(F2) を再送する」
            // 契約（double-F2 防止）とセットの設計だったが、ADR-100 決定2（2026-08-22）で eager
            // warmup の送信キーが `VK_DBE_HIRAGANA` から `VK_IME_ON` 単発（open 軸のみ）へ変わった
            // 時点で契約が崩れていた。物理 F2 は消されるのに、代わりに届くのは open 軸だけで
            // charset 軸（カタカナ→ひらがな）は戻らない「食い逃げ」になり、IME belief が OFF の
            // ときは埋め合わせ（`kp_restore_hiragana_for_suppressed_mode_key`、`effective_open`
            // 必須）も見送られて物理ひらがなキーが完全に無反応になった（ADR-137 M-6、
            // BUG-173: GJI + Windows Terminal でカタカナから物理ひらがなキーで戻れない）。
            //
            // awase は物理 F2 の代わりに何も送らない（cold 化と GjiFsm 通知だけ、
            // `WindowsPlatform::composition_native_f2_down`）ので、物理 F2 を素通ししても二重 actuation に
            // ならない（conv は GJI 自身が物理キーとして処理する）。判定は VK だけで決まる。
            if event.vk_code == crate::vk::VK_DBE_HIRAGANA {
                return Self::Allow;
            }

            // BUG-136 (issue #136): 他プロセスの SendInput (LLKHF_INJECTED) 由来のイベントは、
            // key_pipeline.rs::kp_stage_shadow_ime_toggle (BUG-14) が shadow_toggled への
            // 昇格を既に禁止しているため、awase 自身が actuate することはない。
            // 「解釈しない入力は消費しない」— awase が actuate しないのに物理キーだけ
            // Suppress すると、OS 側にも awase 側にも誰も IME を切り替えない
            // 「二重の空振り」になる（PowerToys Mouse Without Borders 等の正規リレー
            // ツールでリモート側の英数/かなキーが完全に無反応になる、ADR-119 参照）。
            //
            // この early return は下の ImmCross アーム（`profile.can_use_imm32_
            // cross_process()` → 無条件 Suppress）よりも先に来るため、ImmCross
            // アプリでも injected イベントは貫通する。ImmCross の無条件 Suppress は
            // 「spurious 連鎖の構造的遮断」（`feedback_immcross_owns_kanji`
            // の設計原則 — ImmCross アプリには物理 IME キーを見せない）という別種の
            // 保護だが、injected イベントは shadow_toggled を発火させないため awase
            // 自身が actuate することはなく、spurious 連鎖の前提（awase の自
            // actuation と物理キー通過の競合）がそもそも成立しない。したがって
            // ここを貫通させても `feedback_immcross_owns_kanji` が防ごうとした
            // リスクは再現しない（ADR-119 決定1参照）。
            if event.injected {
                debug_assert!(
                    !shadow_toggled,
                    "injected イベントで shadow_toggled が立つのは設計違反 \
                 (BUG-14 ガード kp_stage_shadow_ime_toggle が必ず false にする)"
                );
                return Self::Allow;
            }

            // 無変換/変換（ADR-141、C2対策）: shadow_action は belief 追随専用
            // （follow-only）であり、物理配送は既定で Allow する。C2対策で
            // これら2キーにも`shadow_action`（`enrich_ime_relevance`経由の
            // shadow_action override）が付くようになったため、対策なしだと
            // 下の`is_kanji_event`判定を抜けてKANJI関連VK同様にSuppressされ
            // うる——GJI自身がこの物理キーを見てIMEを切り替えることに
            // 依存している設計（BUG-115）なので、Suppressすると「OS側にも
            // awase側にも誰もIMEを切り替えない二重の空振り」（ADR-119と同型）
            // になる。VK_DBE_HIRAGANA等の静的KANJIキーと異なり、無変換/変換は
            // 既定では awase自身がactuationを所有する対象ではない（delegate/
            // shadow-toggleのどちらが処理する場合もbelief追随のみで、OS側の
            // 実際の切替はGJI自身が物理キー配送を通じて行う）ため、
            // `is_kanji_event`判定より前でこの分岐を置く。
            //
            // 例外（旧 ADR-153 決定1 M19）は ADR-206 で撤去した: 生キーを届けない責務は、開閉を書く打鍵では
            // エンジンの `Decision::Consume` が負う（`thumb_or_role_fkey_disposition` の doc 参照）。
            if let Some(disposition) =
                Self::legacy_thumb_or_role_fkey_disposition(event, shadow_toggled)
            {
                return disposition;
            }

            let is_kanji_event = event.ime_relevance.shadow_action.is_some();
            if !is_kanji_event {
                return Self::Allow;
            }
            let suppress = if profile.can_use_imm32_cross_process() {
                // ImmCross: KANJI 関連 VK は原則 Down/Up 共に Suppress。
                // 0xF2 HIRAGANA は上の専用分岐で常に先に Allow になる（BUG-173。MS-IME 本体が物理 F2 で開く
                // 経路も残る、ADR-190）。
                true
            } else {
                // apply-ime が GjiDirect/MsImeDirect で実際に actuate する場合のみ、
                // shadow_toggle 発火時 KeyDown + 全 KeyUp を Suppress（BUG-46）。
                let ime_actuation_owned = key_sequence_policy::gji_direct_applicable(kind)
                    || key_sequence_policy::ms_ime_direct_applicable(kind);
                // 半角/全角 (0xF3 SBCSCHAR / 0xF4 DBCSCHAR。0xF2 HIRAGANA は上の専用分岐で
                // 既に処理済みのためここには来ない) の KeyDown は、**awase が beliefに基づく
                // 開閉トグルとして書くキー**（`enrich_key_role` が役割から `Some(Toggle)` を付けた 0xF3/0xF4、
                // ADR-199 決定8）に限り、`shadow_toggled` に関わらず常に Suppress。
                // （採用中のGJI学習表が半角/全角を開閉トグルでないと示す場合は`shadow_action`が付かず、
                // 上の`is_kanji_event`判定でDown/UpともAllow済みでここに来ない。ADR-195追記）
                // 素通しすると、awase が書く開閉に加えて実 IME が同じキーを能動的に処理する
                // 二重 actuation になる（BUG-46/BUG-52）。
                //
                // **ADR-191（撤去後）**: 英数(0xF0)・カタカナ(0xF1)は awase が書かない
                // （`shadow_action` を持たない）。実 IME に処理させて Engine は観測に追随する
                // ので、Suppress してはならない——握りつぶすと OS にも awase にも誰も何もしない
                // 「二重の空振り」になる。この2キーは上の `is_kanji_event` 判定で既に Allow だが、
                // 判定の根拠を「awase が書くキー」に揃えるため、ここでも VK を列挙せず
                // 役割由来の `shadow_action`（`Some(Toggle)`）で決める（BUG-116/ADR-137 の Shift+0xF1 の特例は、
                // 0xF1 が常に Allow になったため不要になり撤去した）。
                //
                // 設定 `dbe_mode_key_policy`（Passthrough で本条件を外す隠し設定）は撤去した
                // （ADR-191、レビュー指摘B-M3）: 0xF3/0xF4 は `enrich_key_role` で（役割が無い・採用中の学習表が
                // 開閉トグルでないと示す場合を除き）`Toggle` の `shadow_action` を持ち
                // `shadow_toggled` で Suppress されるため、
                // Passthrough を選んでも 0xF3/0xF4 は Suppress のままで、それ以外のキーには
                // そもそも効かない、実質死んだ設定だった。旧 config.toml にキーが残っていても
                // 未知キーとして無視され警告は出ない（`src/config.rs` のテストで固定）。
                let is_dbe_mode_key_down = legacy_is_role_toggle_hz_key_down(event);
                ime_actuation_owned
                    && (shadow_toggled
                        || is_dbe_mode_key_down
                        || matches!(event.event_type, KeyEventType::KeyUp))
            };
            if suppress {
                Self::Suppress
            } else {
                Self::Allow
            }
        }
    }

    fn legacy_is_role_toggle_hz_key_down(event: &RawKeyEvent) -> bool {
        event.event_type == KeyEventType::KeyDown
            && matches!(
                event.ime_relevance.shadow_action,
                Some(ShadowImeAction::Toggle)
            )
            && matches!(
                event.vk_code.ime_kind(),
                Some(crate::vk::ImeKeyKind::DbeSbcsChar | crate::vk::ImeKeyKind::DbeDbcsChar)
            )
    }

    fn legacy_select_shadow_intent(
        event: &RawKeyEvent,
        is_japanese_ime: bool,
    ) -> Option<(ShadowImeAction, ShadowIntentKind)> {
        let intent_kind = if let Some(a) = event.ime_relevance.sync_direction {
            Some((a, ShadowIntentKind::SyncKey))
        } else if let Some(a) = event
            .ime_relevance
            .shadow_action
            .filter(|_| crate::vk::is_static_idempotent_open_key(event.vk_code))
        {
            // ADR-207: VK_IME_ON/OFF（0x16/0x1A）は IME の種類に依らず冪等なので、`is_japanese_ime()`
            // （awase のワーカースレッドの HKL 由来で偽になりうる）を問わず採用する。`keys.ime_detect`
            // の既定（IMEオン/IMEオフ）を空にしても、従来 sync 既定が担っていた追随を保つ。
            Some((a, ShadowIntentKind::PhysicalImeKey))
        } else if is_japanese_ime {
            event
                .ime_relevance
                .shadow_action
                // ADR-199 決定18(ii): F13〜F24 の役割由来 Toggle は自動リピートの Down では昇格させない
                // （物理の F13 はリピートし、`kp_stage_shadow_ime_toggle` はリピートを区別しないので、
                // そのままではリピートのたびに開閉が反転する）。0xF3/0xF4・0x19 の挙動は変えない。
                .filter(|_| !(event.was_down && crate::vk::is_role_fkey(event.vk_code)))
                .map(|a| (a, ShadowIntentKind::PhysicalImeKey))
        } else {
            None
        };
        intent_kind
    }

    /// 切り出し前の `handle_engine_set_open` の条件（`self.is_ctrl_ime_chord_active() && !target`）。
    const fn legacy_engine_filter(chord_active: bool, target: bool) -> bool {
        chord_active && !target
    }

    /// 逐語コピーとの差分 0（L0 の「挙動を変えない」の、移動後の同一関数どうしの比較を避けた確認）。
    /// 役割分担: このテストは「変更点以外が逐語コピーと同じ」ことの確認（`expected_jp` は新実装の条件式の再掲で、変更点の意味は
    /// 検証しない）。変更点そのもの（0x19 は同定済み、0xF3/F4/F13 は `is_japanese_ime` を問わない、未同定の 0x19 は受動）の期待値は
    /// `select_shadow_intent_matches_the_pre_extraction_branches` と
    /// `tests/explicit_press_exhaustive.rs::kanji_is_promoted_when_identified_and_passed_through_when_unidentified` が直接固定する。
    ///
    /// 全 `PressState`（120,960）× 12 キー（イベントを持つ 10 種）× KeyDown/KeyUp × `shadow_toggled` × 4 プロファイル × 2 種別で
    /// 配送を、`is_japanese_ime` で intent 選択を、chord × target でフィルタ条件を比べる。
    #[test]
    fn new_implementations_match_the_verbatim_legacy_copies() {
        let mut plan_cases = 0u64;
        for s in PressState::all() {
            for key in ExplicitKey::ALL {
                let Some(mut ev) = key.event(s.was_down) else {
                    continue;
                };
                // L2（ADR-208 D2）: 新実装が逐語コピーと違うのは、`is_japanese_ime` が偽の**非リピート**の
                // `shadow_action` 昇格（0xF3/F4・F13 は無条件、0x19 は TIP 同定済みのときだけ。「真だったとしたら」の結果になる）。
                let is_kanji = key == ExplicitKey::Kanji;
                let expected_jp =
                    s.is_japanese_ime || (!ev.was_down && (!is_kanji || s.ime_identified));
                assert_eq!(
                    select_shadow_intent(&ev, s.is_japanese_ime, s.ime_identified),
                    legacy_select_shadow_intent(&ev, expected_jp),
                    "{key:?} {s:?}"
                );
                if s.is_japanese_ime || ev.was_down {
                    assert_eq!(
                        select_shadow_intent(&ev, s.is_japanese_ime, s.ime_identified),
                        legacy_select_shadow_intent(&ev, s.is_japanese_ime),
                        "{key:?} {s:?}"
                    );
                }
                for event_type in [KeyEventType::KeyDown, KeyEventType::KeyUp] {
                    ev.event_type = event_type;
                    for toggled in [false, true] {
                        let new = PhysicalKeyDisposition::plan_core(
                            &ev,
                            s.profile.app_profile(),
                            toggled,
                            s.ime_kind,
                        );
                        let old = PhysicalKeyDisposition::legacy_plan(
                            &ev,
                            s.profile.app_profile(),
                            toggled,
                            s.ime_kind,
                        );
                        assert_eq!(new, old, "{key:?} {event_type:?} toggled={toggled} {s:?}");
                        plan_cases += 1;
                    }
                }
            }
            for target in [false, true] {
                assert_eq!(
                    engine_set_open_filtered_by_chord(s.ctrl_chord, target),
                    legacy_engine_filter(s.ctrl_chord, target)
                );
            }
        }
        assert_eq!(plan_cases, 120_960 * 10 * 2 * 2);
    }
}

#[cfg(test)]
mod shadow_noop_write_tests {
    use super::shadow_noop_write_wanted;

    #[test]
    fn writes_only_for_a_suppressed_non_repeat_press_outside_tsf_native() {
        // 押下 ID あり・plan0 が Suppress・TsfNative でない → 書く。
        assert!(shadow_noop_write_wanted(true, false, true));
        // plan0 が Allow（物理が IME に届く窓）なら書かない（INV-L1 の「ちょうど一方」、BUG-113）。
        assert!(!shadow_noop_write_wanted(true, false, false));
        // リピート（押下 ID なし）は書かない。
        assert!(!shadow_noop_write_wanted(false, false, true));
        // TsfNative は L3'（BUG-124 の実機 A/B）まで書かない。
        assert!(!shadow_noop_write_wanted(true, true, true));
    }
}
