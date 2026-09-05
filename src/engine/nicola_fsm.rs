//! NicolaFsm: 同時打鍵判定 FSM（timed-fsm ベース）

use smallvec::{smallvec, SmallVec};
use timed_fsm::{Response, ShiftReduceParser};

use crate::config::{ConfirmMode, GeneralConfig};
use crate::engine::input_tracker::PhysicalKeyState;
use crate::engine::output_history::{OutputEntry, OutputHistory};
use crate::ngram::NgramModel;
use crate::scanmap::PhysicalPos;
use crate::types::{
    ContextChange, KeyAction, KeyEventType, RawKeyEvent, ScanCode, SpecialKey, Timestamp, VkCode,
};
use crate::yab::{YabFace, YabLayout, YabValue};

use super::consecutive_counter::ConsecutiveSoloCounter;
use super::fsm_types::{
    BypassReason, ClassifiedEvent, ComposingHint, EngineState, Face, IdleIntent, KeyClass,
    ModeKeyConfig, OutputUpdate, ParseAction, PendingKey, PendingThumbData, ResolvedAction,
    SoloTapAction, TextKeyConfig, ThumbSide, TimerIntent, TIMER_PENDING, TIMER_SPECULATIVE,
};
use super::retro_eval_stats::{self, RetroEvalStats};
use super::timing::{self, DecisionPhase};

/// AdaptiveTiming モードで連続打鍵と判定する閾値（マイクロ秒）
pub(super) const CONTINUOUS_KEYSTROKE_THRESHOLD_US: u64 = 80_000;

/// ソロ連打トリガーの連打間隔上限（マイクロ秒）
const SOLO_OFF_TIMEOUT_US: u64 = 400_000;

/// ソロ連打でエンジン OFF を発動する必要連打回数
///
/// 3 回だと、スリープ復帰直後など IME が混乱した状態で焦って無変換キーを
/// 連打しただけで誤発火し、「緊急脱出」のつもりが逆に engine を止めてしまう
/// 事例が実機で発生した（2026-07-08、conv がカタカナへ固定＋shadow OFF から
/// の復帰を試みて無変換を連打した結果、user_enabled が意図せず false に）。
/// 5 回に引き上げて誤発火しにくくする。
const SOLO_OFF_TRIGGER_COUNT: u32 = 5;

/// `char_thumb_chord_confirmed`（重なり不足判定）の本番既定マージン（ADR-112 決定1）。
///
/// `Engine::on_input` の Phase 0 が KeyUp を FSM に一切届けていなかった
/// リグレッション（BUG-101）の修正（ADR-112 決定2）により、`char1_released_at`
/// が初めて実際に埋まるようになる。決定2適用前は `char1_released_at` が
/// 恒久的に `None` で `overlap_only_verdict` が常に `Some(true)`（＝重なり
/// 十分＝同時打鍵確定）を返していたため、この 0%（＝常に重なり十分と同じ
/// 効果）は「経路修正だけを先に land し、判定内容そのものは変えない」という
/// 意図の値である。`timing.rs` の単体テストが検証する
/// `MIN_OVERLAP_MARGIN_PERCENT`（15%）とは独立——あちらはアルゴリズム自体の
/// 正しさを、こちらは「決定2 land 直後の実運用でどう振る舞わせるか」を表す。
/// 実機ソーク後、実測付きで別コミット/別ADRとして引き締める
/// （`docs/adr/112-keyup-lifecycle-fsm-delivery.md` 決定3、本ADRのスコープ外）。
const RUNTIME_MIN_OVERLAP_MARGIN_PERCENT: u64 = 0;

/// `Response` の型エイリアス
type Resp = Response<KeyAction, usize>;

/// `SmallVec` 中の各 `KeyAction` を、`Sequence` なら中身を展開し、それ以外
/// はそのまま1要素として並べた `Vec` に変換する（ADR-115 決定5）。
/// `Sequence` を非ネストにする不変条件（決定4）により、この展開は1階層で
/// 完結する。`.into_vec()` を直接呼ぶ箇所を作らないこと——この関数が
/// 唯一の変換点（実際に打鍵列が「開く」のはここだけに閉じる）。
/// ADR-120 決定0a 項目2: Phase2 の `score_a`/`score_b` それぞれを
/// `NEG_INFINITY`/ゼロ/有限 の3値に独立して分類し、対応するカウンタを+1する。
/// `score_a`/`score_b` は互いに独立な値なので、同じ3値分類でも別々に数える
/// （どちらか一方だけ `NEG_INFINITY` のケースを区別できるようにするため）。
fn record_score_bucket(stats: &mut RetroEvalStats, is_score_a: bool, score: Option<f32>) {
    let Some(score) = score else {
        return;
    };
    if score == f32::NEG_INFINITY {
        if is_score_a {
            stats.score_a_neg_infinity_count += 1;
        } else {
            stats.score_b_neg_infinity_count += 1;
        }
    } else if score == 0.0 {
        if is_score_a {
            stats.score_a_zero_count += 1;
        } else {
            stats.score_b_zero_count += 1;
        }
    } else if is_score_a {
        stats.score_a_finite_count += 1;
    } else {
        stats.score_b_finite_count += 1;
    }
}

fn flatten_actions(actions: SmallVec<[KeyAction; 2]>) -> Vec<KeyAction> {
    let mut out = Vec::with_capacity(actions.len());
    for action in actions {
        match action {
            KeyAction::Sequence(items) => out.extend(items),
            other => out.push(other),
        }
    }
    out
}

impl From<&YabValue> for KeyAction {
    fn from(value: &YabValue) -> Self {
        match value {
            // kana が解決済みの場合は Char で直接出力。
            // Unicode モードでは IME を経由せず直接送信、VK モードでは
            // send_char_as_vk が kana_to_romaji で逆引きして batched 送信する。
            YabValue::Romaji { kana: Some(ch), .. } => Self::Char(*ch),
            // kana 未解決（拗音など単一 char に収まらないケース）は VK 経由でフォールバック
            YabValue::Romaji { romaji, kana: None } => Self::Romaji(romaji.clone()),
            YabValue::Literal(s) => s.chars().next().map_or(Self::Suppress, Self::Char),
            YabValue::KeySequence(s) => Self::KeySequence(s.clone()),
            YabValue::Special(sk) => Self::SpecialKey(*sk),
            YabValue::Vk(vk) => Self::Key(*vk),
            YabValue::CtrlChord { vk, .. } => Self::CtrlChord(*vk),
            YabValue::Sequence(items) => Self::Sequence(items.iter().map(Self::from).collect()),
            // resolve_keystroke_syntax（ADR-115 決定3）の呼び出し漏れがあると
            // ここへ到達しうる。unreachable!() は使わず安全側に倒す
            // （ADR-104「型で保証されない unreachable! の除去」に整合）。
            YabValue::InlineSequence { .. } | YabValue::MacroRef(_) => {
                log::error!(
                    "[yab] 未解決の新構文がエンジンに到達した \
                     — resolve_keystroke_syntax の呼び出し漏れ: {value:?}"
                );
                Self::Suppress
            }
            YabValue::None => Self::Suppress,
        }
    }
}

#[cfg(test)]
/// `YabValue` をエンジン出力用の `KeyAction` に変換する（`From` トレイトへの委譲）。
pub(crate) fn yab_value_to_action(value: &YabValue) -> KeyAction {
    KeyAction::from(value)
}

/// 配列変換エンジン（状態機械 + 同時打鍵判定）
#[allow(missing_debug_implementations)]
#[allow(clippy::struct_excessive_bools)]
pub struct NicolaFsm {
    /// 配列定義（.yab ベース）
    pub(crate) layout: YabLayout,

    /// エンジンの状態（データ付き enum）
    pub(crate) state: EngineState,

    /// 同時打鍵の判定閾値（マイクロ秒）
    pub(crate) threshold_us: u64,

    /// エンジンの有効/無効
    pub(crate) enabled: bool,

    /// n-gram モデル（None なら固定閾値にフォールバック）
    pub(crate) ngram_model: Option<NgramModel>,

    /// 3キー仲裁のタイミングマージン（%）。`timing::TIMING_MARGIN_PERCENT`
    /// （既定30）を初期値とし、`set_timing_margins` で上書きできる
    /// （`GeneralConfig::timing_margin_percent` 参照）。ADR-112 決定1のような
    /// 安全上の制約は無く、`bootstrap` が起動直後に必ず一度 `set_timing_margins`
    /// を呼ぶため、実質的な既定値は `GeneralConfig::timing_margin_percent`
    /// （config.rs、既定30）が決める。
    pub(crate) timing_margin_percent: u64,

    /// 重なり不足判定のマージン（%）。構築直後の初期値は
    /// `RUNTIME_MIN_OVERLAP_MARGIN_PERCENT`（0、ADR-112決定1）だが、
    /// `bootstrap` が起動直後に必ず一度 `set_timing_margins` を呼ぶため、
    /// 実運用での既定値は実質的に `GeneralConfig::min_overlap_margin_percent`
    /// （config.rs）が決める。**この設定項目のユーザー向け既定値も 0 のまま
    /// にしてある**（決定1の「実機ソークで確認するまで重なり不足判定を
    /// 無効化する」という安全側の意図を、設定可能にした後も壊さないため）。
    /// 実機ソーク後、実測付きで別コミット/別ADRとして両方の既定値を
    /// 引き締める（決定3、本ADRのスコープ外）。テストで
    /// `set_min_overlap_margin_percent_for_test` を使うと、このモジュールの
    /// 単体テストが検証する `MIN_OVERLAP_MARGIN_PERCENT`（15%）相当の
    /// アルゴリズム挙動を NicolaFsm 統合レベルでも検証できる。
    pub(crate) min_overlap_margin_percent: u64,

    /// 確定モード
    pub(crate) confirm_mode: ConfirmMode,

    /// 投機出力までの待機時間（マイクロ秒）
    pub(crate) speculative_delay_us: u64,

    /// 直前のキー押下時刻（AdaptiveTiming 用）
    pub(crate) last_key_timestamp: Option<Timestamp>,

    /// 直前のキーとの間隔（マイクロ秒）。on_key_down の冒頭で算出。
    pub(crate) last_key_gap_us: Option<u64>,

    /// 出力履歴（押下中キーの追跡と直近出力の記録を統合管理）
    pub(crate) output_history: OutputHistory,

    /// 最新の物理キー状態スナップショット（`on_event` の冒頭で更新される）
    pub(crate) phys: PhysicalKeyState,

    /// 消費済み左親指キーの押下タイムスタンプ。
    /// `phys.left_thumb_down` と一致すれば消費済み、不一致なら未消費。
    /// 新しい押下や KeyUp で物理状態が変われば自動的に不一致になるため、
    /// 明示的なリセットが不要。
    pub(crate) left_thumb_consumed: Option<Timestamp>,

    /// 消費済み右親指キーの押下タイムスタンプ（左と同様）。
    pub(crate) right_thumb_consumed: Option<Timestamp>,

    /// 親指+小指シフト複合面を有効にするか。
    ///
    /// 親指キー自体が Shift 修飾キーに割り当てられている場合は、親指押下だけで
    /// Shift が立つため false にする。`new()` は `left_thumb_vk`/`right_thumb_vk`
    /// が実際に Shift かどうかを（Platform 層依存の判定になるため）core 層だけでは
    /// 判定できず、`false`（無効・従来面のみ）で初期化する。**Platform 層は
    /// 起動直後に必ず `set_thumb_shift_faces_enabled()` を呼んで実際の値を
    /// 設定すること**（`crates/awase-windows/src/app/bootstrap.rs`・
    /// `crates/awase-linux/src/main.rs`・`crates/awase-macos/src/main.rs` 参照）。
    /// 呼び忘れても「複合面が使えない」に留まり、親指自体が Shift のケースで
    /// 複合面が誤って有効化される（誤出力）方向には倒れない——`true` を既定にすると
    /// 呼び忘れ時にこの誤出力が起こるため、意図的に安全側の `false` を既定にしている。
    thumb_shift_faces_enabled: bool,

    /// ソロ確定の連続回数を追跡する汎用カウンター（親指キーに割り当てた
    /// `engine_off_solo_repeat_vk` 用、`PendingThumb` 経由でのみ更新される）。
    solo_counter: ConsecutiveSoloCounter,

    /// `engine_off_solo_repeat_vk` が親指キー（`left_thumb_key`/`right_thumb_key`）
    /// と異なる VK の場合専用の連続回数カウンター。`solo_counter` とは
    /// 独立: `handle_bypass`（`BypassReason::Passthrough` 経路）でのみ更新され、
    /// `PendingThumb` の状態遷移には一切関与しない。親指キーに割り当てた
    /// 場合はこのキーが `KeyClass::Passthrough` に分類されないため、
    /// 両カウンターが同時に動くことはない（`bypass_reason`/`decide_idle` 等の
    /// 分類の時点で排他）。
    engine_off_extra_solo_counter: ConsecutiveSoloCounter,

    /// `engine_off_extra_solo_counter` 用の物理押下トラッカー。`None` =
    /// `engine_off_solo_repeat_vk`（親指キー以外に割り当てた場合）が現在押下されて
    /// いない。`Some(suppressed)` = 現在押下中で、その KeyDown を suppress
    /// したか（true）素通ししたか（false）を覚えている。OS のオートリピート
    /// KeyDown を新規タップとして誤カウントしないためのガードと、対応する
    /// KeyUp（`on_key_up`）を KeyDown と同じ判定にする（J↓/J↑ 対称化）の
    /// 両方に使う。
    engine_off_extra_key_suppressed: Option<bool>,

    /// ソロ N 連打でエンジン OFF を発動するキー（VkCode(0) = 機能無効）。
    engine_off_solo_repeat_vk: VkCode,

    /// ソロ連打でのエンジン OFF 要求フラグ（1ショット）。
    engine_off_requested: bool,

    /// `left_thumb_key`/`right_thumb_key` のいずれかが Space (`VK_SPACE`) に
    /// 割り当てられている場合、その VK コード。どちらも Space でなければ `None`。
    ///
    /// 実際の VK 番号（Windows の magic hex）は Platform 層（各 `vk.rs`）の
    /// 責務であり、core はここで渡された値と等値比較するだけで「Space かどうか」
    /// を判定する（`GeneralConfig::space_thumb_ignore_composing_guard` 等参照）。
    space_thumb_vk: Option<VkCode>,

    /// Space 親指キー単独タップの設定（ADR-092 決定B、`TextKeyConfig`）。
    /// `space_thumb_vk` が `None` なら無効。
    text_key_space: TextKeyConfig,

    /// `left_thumb_key`/`right_thumb_key` のいずれかが無変換 (`VK_NONCONVERT`) に
    /// 割り当てられている場合、その VK コード。`space_thumb_vk` と同様、実際の VK
    /// 番号は Platform 層の責務で、core は等値比較のみ行う。
    muhenkan_vk: Option<VkCode>,

    /// 無変換キー単独タップの設定（ADR-092 決定B、`ModeKeyConfig`）。
    /// `muhenkan_vk` が `None` なら無効。既定値は idle/composing とも
    /// `GuardAction::Suppress`（composing の有無を問わず無変換単独タップは
    /// 常に無視する）。
    ///
    /// MS-IME は「キーとタッチのカスタマイズ」で無変換キー単独打鍵に既定で
    /// 「かな切替」（IME オン相当）を割り当てている（`msime_key_assignment.rs`
    /// 参照）。awase が composing していない場面で無変換の生 VK を素通しすると、
    /// この既定割当てに横取りされて awase の管理外で IME モードが切り替わる
    /// （2026-08-07 実機: 無変換単独タップ直後に `VK_DBE_ALPHANUMERIC`→
    /// `VK_DBE_HIRAGANA` が非注入で観測され、shadow toggle が IME を ON にした）。
    /// 既定 Suppress は、この経路を完全に断つため composing 中かどうかを問わず
    /// 無変換単独タップを常に抑制する。
    mode_key_muhenkan: ModeKeyConfig,

    /// 専用 Fn キー変換モード（ADR-091 §D3.2）。`Some(vk)` なら、無変換単独タップ
    /// 確定時に `mode_key_muhenkan` による既存の抑制/パススルー判定を一切経由せず、
    /// 常にこの Fn キーを送出する（composing の有無を問わない、belief 不要）。
    /// `GeneralConfig::muhenkan_solo_tap_dedicated_fn_key` に対応する。`None`
    /// （既定）なら無効で従来通り。
    ///
    /// `ModeKeyConfig` には統合しない（ADR-092 決定B）——`gji_charset_autodetect`
    /// が実行時に独立して自動検出・設定する値であり、`set_thumb_key_solo_tap_config`
    /// によるconfig reloadのたびに上書き消去されると自動検出機能（ADR-091 F21）が
    /// 壊れるため、専用の独立フィールド・独立setterのまま維持する。
    muhenkan_solo_tap_dedicated_fn_key: Option<VkCode>,

    /// MS-IME レジストリ/GJI config1.db の宣言に基づき、無変換キー単独タップを
    /// IME open 軸への操作へ肩代わりする（ADR-092 決定D Step4b）。`Some(action)`
    /// なら、`mode_key_muhenkan`/`muhenkan_solo_tap_dedicated_fn_key` のいずれよりも
    /// 後・`ModeKeyConfig` ベースの抑制/パススルー判定より前に評価される
    /// （`dedicated_fn_key` が最優先、その次にこれ）。`ModeKeyConfig` には
    /// 統合しない——`dedicated_fn_key` と同じ理由（自動検出由来の値が
    /// config reload で消去されるのを防ぐ）。
    muhenkan_delegate_to_open_axis: Option<crate::types::ShadowImeAction>,

    /// `muhenkan_delegate_to_open_axis` と対称（変換キー用）。
    henkan_delegate_to_open_axis: Option<crate::types::ShadowImeAction>,

    /// `left_thumb_key`/`right_thumb_key` のいずれかが Hiragana に割り当てられて
    /// いる場合、その VK コード。実際の VK 番号は Platform 層の責務であり、
    /// core は渡された値と等値比較するだけ。
    hiragana_vk: Option<VkCode>,

    /// `hiragana_vk` が単独タップとして確定したときの IME open 軸 delegate。
    hiragana_delegate_to_open_axis: Option<crate::types::ShadowImeAction>,

    /// `hiragana_vk` と対称（Katakana キー用）。
    katakana_vk: Option<VkCode>,

    /// `hiragana_delegate_to_open_axis` と対称（Katakana キー用）。
    katakana_delegate_to_open_axis: Option<crate::types::ShadowImeAction>,

    /// `resolve_pending_thumb_as_single` が `DelegateToOpenAxis` 相当の判定を
    /// 下した直後、`Engine` 層が次の `on_input`/`on_timeout` で取り出すまで
    /// 保持するワンショットの副作用要求（ADR-092 決定D Step4b）。
    /// `engine_off_requested`（`:119`、`take_engine_off_requested`）と同型。
    /// `NicolaFsm`/`ParseAction`/`ResolvedAction` は IME への副作用を出す経路を
    /// 持たないため、このワンショットチャネルだけが唯一の伝達経路になる。
    ime_open_requested: Option<crate::types::ShadowImeAction>,

    /// `left_thumb_key`/`right_thumb_key` のいずれかが変換 (`VK_CONVERT`) に
    /// 割り当てられている場合、その VK コード。`muhenkan_vk` と同様の扱い。
    henkan_vk: Option<VkCode>,

    /// 変換キー単独タップの設定（ADR-092 決定B、`ModeKeyConfig`）。
    /// `henkan_vk` が `None` なら無効。`mode_key_muhenkan` と対称
    /// （BUG-58 関連調査で発覚: 変換キーには従来この抑制手段が無く、composing
    /// していない場面では常に生 VK_CONVERT が送出されていた。MS-IME は
    /// 「キーとタッチのカスタマイズ」で変換キー単独打鍵に既定で「再変換」を
    /// 割り当てており、設定次第では IME-オン相当の割当ても可能なため、無変換と
    /// 同じ横取りリスクがある）。
    mode_key_henkan: ModeKeyConfig,

    /// `left_thumb_key`/`right_thumb_key` のいずれかが Enter (`VK_RETURN`) に
    /// 割り当てられている場合、その VK コード。`space_thumb_vk` と同様、実際の VK
    /// 番号は Platform 層の責務で、core は等値比較のみ行う。
    enter_thumb_vk: Option<VkCode>,

    /// Enter 親指キー単独タップの設定（ADR-092 決定B、`TextKeyConfig`）。
    /// `enter_thumb_vk` が `None` なら無効。
    ///
    /// Enter は IME 変換候補の確定という正規機能を持つため、既定値は Space と同じ
    /// `ignore_composing_guard: true`（常時送出）。無変換/変換と異なり `false` を
    /// 既定にすると、変換候補ウィンドウ表示中に Enter 単独タップが丸ごと抑制され、
    /// 変換確定そのものができなくなってしまう。
    text_key_enter: TextKeyConfig,

    /// ADR-120 決定0a: 3キー仲裁の判定過程・その後の訂正発生を観測する累積カウンタ。
    /// 実際の変換結果には一切影響しない（読み取り専用の観測用フィールド）。
    retro_eval_stats: RetroEvalStats,

    /// ADR-120 決定0a 項目7: 「直近の決定」をカテゴリ別に独立して保持する。
    /// 単一スロットにすると、同一ターン内の2回目の `update_history` 等で
    /// Phase2 の訂正シグナルが即座に Baseline へ上書きされ、訂正ヒストグラムが
    /// bucket 0 に集中してしまう（Opusレビュー blocker 所見2）。
    last_decision: Option<LastDecision>,

    /// ADR-120 決定0a 項目4/7 (should-fix所見S1/S2/S5対応で統合): 3キー仲裁
    /// 決定（Phase1/Phase2/NoNgramいずれでも）「自身の出力」をあと何回
    /// スキップすればBaseline計上から除外を終えてよいか。出力の種類を
    /// 問わずデクリメントする（Char/Romaji以外の出力ではスキップが進まず
    /// 次の実打鍵の出力を誤って飲み込んでいたバグの修正、所見S2）。
    /// `measure_since`はPhase2決定のときだけSomeで、項目4「後続1かな確定」
    /// までの経過ms計測の起点になる（Phase1/NoNgramではBaseline除外のみ
    /// 行い時間計測はしない、所見S5）。
    own_decision_output: Option<OwnDecisionOutput>,

    /// ADR-120 決定0a 項目2c: Phase2決定直後、残り何打鍵(KeyDown)を「親指の
    /// 有無」観測窓として見るか。2から開始し、通常のCharキーKeyDownで
    /// デクリメント、親指KeyDownが来たら即座に破棄
    /// （`no_thumb_followup_count` を増やさない）、0に達したら
    /// `no_thumb_followup_count` を+1。`own_decision_output` とは独立した
    /// 状態にする（同じ pending_decision を共用しない）。
    thumb_watch_window: Option<ThumbWatchWindow>,

    /// ADR-120 決定0a 項目7(a)専用: 物理 BACKSPACE の VK コード。Platform 層が
    /// 判定して渡す（`space_thumb_vk` 等と同様、実際の VK 番号は Platform 層の
    /// 責務）。`None`（既定）なら項目7(a)の集計は行わない
    /// （`crates/awase-linux`/`crates/awase-macos` はまだ配線しない）。
    backspace_vk: Option<VkCode>,

    /// ADR-120 決定0a 項目7(a) (should-fix所見S4対応): 物理 BACKSPACE が
    /// 現在押下中かどうか。OS オートリピートによる KeyDown 再送を新規タップ
    /// として二重計上しないためのガード（`engine_off_extra_key_suppressed`
    /// と同型、`handle_bypass`/`on_key_up` 参照）。
    backspace_down: bool,
}

struct ThumbSoloSpecialHandling {
    dedicated_fn_key: Option<VkCode>,
    delegate_to_open_axis: Option<crate::types::ShadowImeAction>,
    mode_key_config: Option<ModeKeyConfig>,
    injected_guarded_delegate: bool,
}

/// ADR-120 決定0a 項目2c の観測窓状態。`last_vk` は直前にこの窓を消費した
/// キーで、OS のオートリピート（同一キーの連続 KeyDown 再送）が「新規タップ」
/// として二重にカウントされるのを防ぐガードに使う（`engine_off_extra_key_suppressed`
/// と同型のリピート抑止、`handle_bypass` 参照）。
#[derive(Debug, Clone, Copy)]
struct ThumbWatchWindow {
    remaining: u8,
    last_vk: Option<VkCode>,
    /// `last_vk` が現在も押下中かどうか（`/code-review` 指摘対応）。単純な
    /// `last_vk == event.vk_code` の等値比較だけでは、同じ物理キーを
    /// 間を置かず2回連続で「本当に」タップした場合（オートリピートでは
    /// なく別々のKeyDown+KeyUpペア2回）も誤って1回のオートリピートと
    /// みなし窓が閉じない。KeyUpで明示的にクリアすることで、押下中の
    /// 再送か新規タップかを正確に区別する。
    last_vk_down: bool,
}

/// ADR-120 決定0a 項目7: カテゴリ別の「直近の決定」タイムスタンプ。
#[derive(Debug, Clone, Copy, Default)]
#[allow(clippy::struct_field_names)] // 3フィールドとも「いつ」を表す `_at` が最も明確
struct LastDecision {
    phase2_at: Option<Timestamp>,
    phase1_at: Option<Timestamp>,
    baseline_at: Option<Timestamp>,
}

/// ADR-120 決定0a 項目7 の決定カテゴリ（`mark_decision` 引数）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DecisionKind {
    Phase2,
    Phase1,
    Baseline,
}

/// ADR-120 決定0a 項目4/7: 3キー仲裁決定「自身の出力」を追跡する統一状態
/// （should-fix所見S1/S2/S5対応）。`remaining`は出力の種類を問わず
/// 1出力ごとに1減算する。`remaining`が0になった後、`measure_since`が
/// `Some`なら次の「かな」出力（Char/Romaji）で項目4の経過msヒストグラムへ
/// 記録する。
#[derive(Debug, Clone, Copy)]
struct OwnDecisionOutput {
    remaining: u8,
    measure_since: Option<Timestamp>,
}

// ── 公開 API ──
impl NicolaFsm {
    #[must_use]
    pub fn new(
        layout: YabLayout,
        _left_thumb_vk: VkCode,
        _right_thumb_vk: VkCode,
        threshold_ms: u32,
        confirm_mode: ConfirmMode,
        speculative_delay_ms: u32,
    ) -> Self {
        Self {
            layout,
            state: EngineState::Idle,
            threshold_us: u64::from(threshold_ms) * 1000,
            enabled: true,
            ngram_model: None,
            timing_margin_percent: timing::TIMING_MARGIN_PERCENT,
            min_overlap_margin_percent: RUNTIME_MIN_OVERLAP_MARGIN_PERCENT,
            confirm_mode,
            speculative_delay_us: u64::from(speculative_delay_ms) * 1000,
            last_key_timestamp: None,
            last_key_gap_us: None,
            output_history: OutputHistory::new(),
            phys: PhysicalKeyState::empty(),
            left_thumb_consumed: None,
            right_thumb_consumed: None,
            // 既定は無効（安全側）。Platform 層が起動直後に
            // set_thumb_shift_faces_enabled() で実際の値を設定する（フィールドの
            // doc コメント参照）。
            thumb_shift_faces_enabled: false,
            solo_counter: ConsecutiveSoloCounter::new(SOLO_OFF_TIMEOUT_US),
            engine_off_extra_solo_counter: ConsecutiveSoloCounter::new(SOLO_OFF_TIMEOUT_US),
            engine_off_extra_key_suppressed: None,
            engine_off_solo_repeat_vk: VkCode(0),
            engine_off_requested: false,
            // 既定値は GeneralConfig::default() と揃える（Space 未割当 / ガード類は
            // 有効）。実際の Space VK は Platform 層が set_space_thumb_config() で
            // 明示的に配線する（config.rs の doc 参照）。
            space_thumb_vk: None,
            text_key_space: TextKeyConfig {
                ignore_composing_guard: true,
                shift_literal: true,
            },
            // 無変換/変換の VK は Platform 層が set_thumb_key_solo_tap_config() で
            // 明示的に配線するまで None。ガード既定値は GeneralConfig::default() と
            // 揃えて ignore_composing_guard=false, always_suppress=true 相当
            // （従来通り composing の有無を問わず抑制）。
            muhenkan_vk: None,
            mode_key_muhenkan: ModeKeyConfig::from_legacy_bools(false, true),
            muhenkan_solo_tap_dedicated_fn_key: None,
            muhenkan_delegate_to_open_axis: None,
            henkan_delegate_to_open_axis: None,
            hiragana_vk: None,
            hiragana_delegate_to_open_axis: None,
            katakana_vk: None,
            katakana_delegate_to_open_axis: None,
            ime_open_requested: None,
            henkan_vk: None,
            mode_key_henkan: ModeKeyConfig::from_legacy_bools(false, true),
            // Enter の VK は Platform 層が set_enter_thumb_config() で明示的に配線する
            // まで None。ガード既定値は GeneralConfig::default() と揃えて Space と
            // 同じ ignore_composing_guard=true（composing 中も変換確定/改行として素通し）。
            enter_thumb_vk: None,
            text_key_enter: TextKeyConfig {
                ignore_composing_guard: true,
                shift_literal: true,
            },
            retro_eval_stats: RetroEvalStats::default(),
            last_decision: None,
            own_decision_output: None,
            thumb_watch_window: None,
            backspace_vk: None,
            backspace_down: false,
        }
    }

    /// Idle 状態に遷移するヘルパー
    const fn go_idle(&mut self) {
        self.state = EngineState::Idle;
    }

    /// `TimerIntent` を `Vec<TimerCommand<usize>>` に変換するヘルパー
    pub(crate) fn timer_cmds(&self, intent: TimerIntent) -> Vec<timed_fsm::TimerCommand<usize>> {
        intent.to_commands(self.threshold_us, self.speculative_delay_us)
    }

    /// 保留中のキーを安全に解消し、Idle 状態に戻す。
    ///
    /// 外部コンテキスト変更（IMEオフ、エンジン無効化、言語切替、レイアウト差替え等）
    /// 時に呼ぶ。現在の外部コンテキストではもう待てないので、保留を解消して安全側に倒す。
    ///
    /// # 事後条件
    /// - `state` は `Idle`
    /// - `TIMER_PENDING` / `TIMER_SPECULATIVE` は停止済み（Response に含まれる）
    /// - 再入しても no-op（Idle → 空の Response）
    /// - 出力は二重送信されない（SpeculativeChar は既に出力済みなので何もしない）
    ///
    /// 呼び出し側は戻り値の `Response` を `dispatch()` で処理すること。
    ///
    /// `composing` は `PendingThumb` の Space フォールバック判定（composing 中でも
    /// 生 VK_SPACE を送出する例外）に使う。**`Trusted(bool)` を渡してよいのは、
    /// 呼び出し元がこの `composing` 値を「保留キーが入力された時点と同一のウィンドウ/
    /// コンテキスト」のものだと保証できる場合のみ**（同一イベント処理内の割り込み、
    /// 直近の `self.phys.composing` 等）。
    ///
    /// `FocusChanged`（フォーカス変更）や `InvalidateContext`（IME OFF・言語切替等の
    /// 外部コンテキスト喪失）経由のフラッシュは、`InputContext::composing` を
    /// 呼び出し時点で読み直す設計上、**既に切り替わった後の新しいウィンドウ**の状態を
    /// 指している（`Runtime::ir_notify_focus_changed` は `detect_and_update_focus()` で
    /// フォーカスを切り替えた後に `build_ctx()` を呼ぶ）。この場合は `Unknown` を渡す。
    /// `Unknown` では Space 例外も含め無条件 suppress する（フォーカス切替後に別ウィンドウへ
    /// 生 VK_SPACE 等が誤注入されるのを防ぐ安全側の選択。過去に類似の focus 遷移バグを
    /// 繰り返してきたため — `docs/known-bugs.md` 参照）。
    ///
    /// # Panics
    ///
    /// Panics if internal state is inconsistent (e.g. `PendingChar` phase
    /// without a stored `pending_char`). This indicates a logic error.
    pub fn flush_pending(&mut self, reason: ContextChange, composing: ComposingHint) -> Resp {
        let old_state = std::mem::replace(&mut self.state, EngineState::Idle);
        let was_idle = matches!(old_state, EngineState::Idle);

        let response = match old_state {
            EngineState::Idle => {
                // Already idle — no-op
                Response::consume()
            }
            EngineState::PendingChar(pending) => {
                // 保留中の文字キーを通常面で単独確定
                let resolved = self.resolve_pending_char_as_single(&pending);
                self.update_history_imprecise(
                    resolved.output,
                    self.last_key_timestamp.unwrap_or(0),
                );
                Response::emit(flatten_actions(resolved.actions))
            }
            EngineState::PendingThumb(thumb) => {
                // 保留中の親指キーを単独確定。composing を信頼できない場合は
                // Space 例外も含め無条件 suppress する（上記 doc 参照）。
                let (resolved, ime_open_request) = match composing {
                    ComposingHint::Trusted(c) => self.resolve_pending_thumb_as_single(
                        thumb.scan_code,
                        thumb.vk_code,
                        thumb.modifier_key,
                        thumb.injected,
                        c,
                    ),
                    ComposingHint::Unknown => (
                        ResolvedAction {
                            actions: SmallVec::new(),
                            output: OutputUpdate::None,
                        },
                        None,
                    ),
                };
                if ime_open_request.is_some() {
                    self.ime_open_requested = ime_open_request;
                }
                self.update_history_imprecise(
                    resolved.output,
                    self.last_key_timestamp.unwrap_or(0),
                );
                Response::emit(flatten_actions(resolved.actions))
            }
            EngineState::PendingCharThumb {
                char_key,
                thumb,
                char1_released_at,
            } => {
                // ContextChange による異常系 flush では現在の Shift 状態を面解決に
                // 使わない。フォーカス変更後の modifier snapshot は別ウィンドウの
                // 状態を指しうるため、通常の親指面で安全側に倒す（ADR-097 決定2 #9）。
                // 重なり不足判定（confirms_char_thumb_chord）もここでは適用しない
                // ——異常系 flush は「今ある情報で即座に確定する」経路であり、
                // 通常の 2 鍵解決（KeyUp/タイムアウト経由）とは別軸のため。
                let resolved = self.resolve_char_thumb_as_simultaneous(&char_key, thumb.face());
                self.update_history_imprecise(
                    resolved.output,
                    self.last_key_timestamp.unwrap_or(0),
                );
                let mut actions = resolved.actions;
                if char1_released_at.is_some() {
                    // char1 は既に物理的に離されている → Key 出力があれば KeyUp も追加
                    self.append_key_up_for(&mut actions, char_key.scan_code);
                }
                Response::emit(flatten_actions(actions))
            }
            EngineState::SpeculativeChar(_) => {
                // 既に投機出力済み → 出力は正しかったとみなす。何も追加しない。
                Response::consume()
            }
        };

        // タイミング状態・ソロ連打カウンターもリセット
        self.last_key_timestamp = None;
        self.last_key_gap_us = None;
        self.solo_counter.reset();

        // ADR-120 決定0a (`/code-review` 指摘): `own_decision_output`/
        // `thumb_watch_window`/`last_decision` はコンテキスト喪失
        // （フォーカス変更・IME OFF・言語切替・レイアウト差替え・エンジン
        // 無効化）を跨いで持ち越してはならない——別アプリ/別コンテキストで
        // 打たれた次の実キー入力を、直前アプリのPhase2決定の「後続かな」
        // として誤帰属してしまう（時間による上限が無く、境界も無い汚染）。
        // `ContextChange::BypassKey`（`handle_bypass` 自身が非idle時に呼ぶ
        // 通常のflush）はコンテキスト喪失ではない——ここで汎用リセットすると
        // `engine_off_extra_key_suppressed`（`:636-642`）と同じ罠になり、
        // 決定直後の普通の打鍵継続で追跡状態が即座に消えてしまう。
        if !matches!(reason, ContextChange::BypassKey) {
            self.own_decision_output = None;
            self.thumb_watch_window = None;
            self.last_decision = None;
        }

        if !was_idle {
            log::info!(
                "flush_pending({:?}): flushed {} action(s)",
                reason,
                response.actions.len()
            );
        }

        // 全タイマー停止を付与
        response
            .with_kill_timer(TIMER_PENDING)
            .with_kill_timer(TIMER_SPECULATIVE)
    }

    /// エンジンの有効/無効を切り替える。
    ///
    /// 無効化時は保留キーをフラッシュする。
    /// 戻り値の `Resp` を `dispatch()` で処理すること（タイマー停止 + 保留キー確定）。
    pub fn toggle_enabled(&mut self) -> (bool, Resp) {
        let mut flush_resp = self.flush_pending(
            ContextChange::EngineDisabled,
            ComposingHint::Trusted(self.phys.composing),
        );
        self.enabled = !self.enabled;
        self.clear_output_history_appending_releases(&mut flush_resp);
        // `solo_counter`（`flush_pending` 内で毎回リセット、:421）とは異なり、
        // こちらは `flush_pending` 内で汎用リセットしない: `handle_bypass` 自身が
        // 非 idle 時に `ContextChange::BypassKey` で `flush_pending` を呼ぶため、
        // 汎用化すると `engine_off_extra_solo_counter` を record 直後に同一呼び出し内で
        // 消してしまい 1〜4 回目のタップが毎回カウントリセットされる
        // （2026-08-26 コードレビュー指摘、report1）。enable トグルという単一の
        // 有意なコンテキスト断絶点でのみリセットする。
        self.engine_off_extra_solo_counter.reset();
        self.engine_off_extra_key_suppressed = None;
        // ADR-120 決定0a 項目7(a) (所見NB1対応、最低ラインの保険): 主たる
        // 復帰経路は `on_key_down` の自己修復（他キーのKeyDownでクリア）だが、
        // エンジン無効化という単一の有意なコンテキスト断絶点でも念のため
        // クリアしておく。
        self.backspace_down = false;
        // 物理キー状態（modifiers, thumb_down）は InputTracker が常に追跡しているため、
        // ここでのリセットは不要。
        log::info!(
            "Engine {}",
            if self.enabled { "enabled" } else { "disabled" }
        );
        (self.enabled, flush_resp)
    }

    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// ADR-120 決定0a: 3キー仲裁の判定過程・訂正発生を観測する累積カウンタを返す。
    /// 起動からの累積値であり、実際の変換結果には一切影響しない。
    #[must_use]
    pub const fn retro_eval_stats(&self) -> &RetroEvalStats {
        &self.retro_eval_stats
    }

    /// ADR-120 決定0a 項目7(a)専用: 物理 BACKSPACE の VK コードを設定する
    /// （Platform 層が `crate::vk::VK_BACK` 等との等値比較で判定して渡す）。
    /// 未呼び出し（既定 `None`）の場合、項目7(a)の集計は行わない
    /// （項目7(b)/7(c)の配列セル由来 Backspace/Escape 計上には影響しない）。
    pub const fn set_backspace_vk(&mut self, vk: Option<VkCode>) {
        self.backspace_vk = vk;
    }

    /// 診断用: 現在の FSM 状態を短い文字列で返す。
    #[must_use]
    pub fn debug_state_label(&self) -> String {
        self.state.debug_label()
    }

    /// エンジンの有効/無効を明示的に設定する。
    ///
    /// 現在の状態と同じ場合は何もしない。
    /// 無効化時は保留キーをフラッシュする。
    /// 戻り値の `Resp` を `dispatch()` で処理すること。
    pub fn set_enabled(&mut self, enable: bool) -> (bool, Resp) {
        if self.enabled == enable {
            return (self.enabled, Response::pass_through());
        }
        self.toggle_enabled()
    }

    /// 同時打鍵判定の閾値を更新する（ミリ秒指定）。
    /// ソロ N 連打でエンジン OFF を発動するキーを設定する。
    /// `VkCode(0)` を渡すと機能を無効にする。
    pub const fn set_engine_off_solo_repeat_vk(&mut self, vk: VkCode) {
        self.engine_off_solo_repeat_vk = vk;
    }

    /// Space 親指キーのフォールバック挙動を設定する。
    ///
    /// `space_thumb_vk` は `left_thumb_key`/`right_thumb_key` のいずれかが
    /// Space (`VK_SPACE`) に解決された場合の VK コード（Platform 層が
    /// `crate::vk::VK_SPACE` との等値比較で判定し渡す）。どちらも Space でなければ
    /// `None` を渡すこと。`config` は `GeneralConfig::space_thumb_ignore_composing_guard`/
    /// `space_thumb_shift_literal` にそのまま対応する（ADR-092 決定B、`TextKeyConfig`）。
    pub const fn set_space_thumb_config(
        &mut self,
        space_thumb_vk: Option<VkCode>,
        config: TextKeyConfig,
    ) {
        self.space_thumb_vk = space_thumb_vk;
        self.text_key_space = config;
    }

    /// 無変換/変換キー単独タップの composing 中ガードの扱いを設定する。
    ///
    /// `muhenkan_vk`/`henkan_vk` は `left_thumb_key`/`right_thumb_key` がそれぞれ
    /// 無変換/変換に解決された場合の VK コード（Platform 層が判定して渡す）。
    /// 割り当てられていなければ `None` を渡すこと。`muhenkan`/`henkan` の各フィールド
    /// は `GeneralConfig` の同名フィールド（`muhenkan_solo_tap_ignore_composing_guard`/
    /// `muhenkan_solo_tap_always_suppress`/`henkan_solo_tap_ignore_composing_guard`/
    /// `henkan_solo_tap_always_suppress`）から`ModeKeyConfig::from_legacy_bools`で
    /// 変換した値を渡す（ADR-092 決定B。`ThumbKeySoloTapGuard` へのグルーピングは
    /// `clippy::fn_params_excessive_bools` 対策だった、BUG-58 関連調査参照）。
    ///
    /// **専用Fnキー（`muhenkan_solo_tap_dedicated_fn_key`）はこの呼び出しでは
    /// 変更されない。** `set_muhenkan_solo_tap_dedicated_fn_key` で独立に設定する
    /// こと——`gji_charset_autodetect` による実行時の自動検出値をこの呼び出しで
    /// 上書き消去しないための意図的な分離。
    pub const fn set_thumb_key_solo_tap_config(
        &mut self,
        muhenkan_vk: Option<VkCode>,
        muhenkan: ModeKeyConfig,
        henkan_vk: Option<VkCode>,
        henkan: ModeKeyConfig,
    ) {
        self.muhenkan_vk = muhenkan_vk;
        self.mode_key_muhenkan = muhenkan;
        self.henkan_vk = henkan_vk;
        self.mode_key_henkan = henkan;
    }

    /// 無変換単独タップの専用 Fn キー変換モード（ADR-091 §D3.2）を設定する。
    ///
    /// `Some(vk)` を渡すと、`muhenkan_vk` の単独タップ確定時に
    /// `set_thumb_key_solo_tap_config` で設定した抑制/パススルー判定を経由せず
    /// 常に `vk` を送出する（`resolve_pending_thumb_as_single` 参照）。`None`
    /// （既定）なら無効。`set_thumb_key_solo_tap_config` とは独立して呼び出せる。
    pub const fn set_muhenkan_solo_tap_dedicated_fn_key(&mut self, vk: Option<VkCode>) {
        self.muhenkan_solo_tap_dedicated_fn_key = vk;
    }

    /// 無変換/変換キー単独タップの IME open 軸への肩代わり（ADR-092 決定D
    /// Step4b）を設定する。`Some(action)` を渡すと、`muhenkan_vk`/`henkan_vk`
    /// の単独タップ確定時に `mode_key_muhenkan`/`mode_key_henkan` による
    /// 抑制/パススルー判定を経由せず、`action` を `ime_open_requested`
    /// （`Engine` が次の `on_input`/`on_timeout` で取り出す）へセットする。
    /// `None`（既定）なら無効。専用Fnキー（`muhenkan_solo_tap_dedicated_fn_key`）
    /// より優先度が低い（両方 `Some` の場合は専用Fnキーが勝つ）。
    pub const fn set_muhenkan_delegate_to_open_axis(
        &mut self,
        action: Option<crate::types::ShadowImeAction>,
    ) {
        self.muhenkan_delegate_to_open_axis = action;
    }

    /// `set_muhenkan_delegate_to_open_axis` と対称（変換キー用）。
    pub const fn set_henkan_delegate_to_open_axis(
        &mut self,
        action: Option<crate::types::ShadowImeAction>,
    ) {
        self.henkan_delegate_to_open_axis = action;
    }

    /// Hiragana/Katakana が現在の親指キーなら、その VK を Platform 層から渡す。
    /// core は生 VK 定数を持たず、ここで渡された値との等値比較のみを行う。
    pub const fn set_hiragana_katakana_thumb_key_config(
        &mut self,
        hiragana_vk: Option<VkCode>,
        katakana_vk: Option<VkCode>,
    ) {
        self.hiragana_vk = hiragana_vk;
        self.katakana_vk = katakana_vk;
    }

    /// Hiragana 親指キー単独タップの IME open 軸 delegate を設定する。
    pub const fn set_hiragana_delegate_to_open_axis(
        &mut self,
        action: Option<crate::types::ShadowImeAction>,
    ) {
        self.hiragana_delegate_to_open_axis = action;
    }

    /// Katakana 親指キー単独タップの IME open 軸 delegate を設定する。
    pub const fn set_katakana_delegate_to_open_axis(
        &mut self,
        action: Option<crate::types::ShadowImeAction>,
    ) {
        self.katakana_delegate_to_open_axis = action;
    }

    #[must_use]
    pub const fn hiragana_delegate_to_open_axis(&self) -> Option<crate::types::ShadowImeAction> {
        self.hiragana_delegate_to_open_axis
    }

    #[must_use]
    pub const fn katakana_delegate_to_open_axis(&self) -> Option<crate::types::ShadowImeAction> {
        self.katakana_delegate_to_open_axis
    }

    /// `resolve_pending_thumb_as_single` がセットした IME open 軸への副作用
    /// 要求を取り出す（1ショット、ADR-092 決定D Step4b）。`Engine::on_input`/
    /// `on_timeout` が呼ぶ。
    pub const fn take_ime_open_requested(&mut self) -> Option<crate::types::ShadowImeAction> {
        self.ime_open_requested.take()
    }

    fn thumb_solo_special_handling(&self, vk_code: VkCode) -> ThumbSoloSpecialHandling {
        if self.muhenkan_vk == Some(vk_code) {
            ThumbSoloSpecialHandling {
                dedicated_fn_key: self.muhenkan_solo_tap_dedicated_fn_key,
                delegate_to_open_axis: self.muhenkan_delegate_to_open_axis,
                mode_key_config: Some(self.mode_key_muhenkan),
                injected_guarded_delegate: false,
            }
        } else if self.henkan_vk == Some(vk_code) {
            ThumbSoloSpecialHandling {
                dedicated_fn_key: None,
                delegate_to_open_axis: self.henkan_delegate_to_open_axis,
                mode_key_config: Some(self.mode_key_henkan),
                injected_guarded_delegate: false,
            }
        } else if self.hiragana_vk == Some(vk_code) {
            ThumbSoloSpecialHandling {
                dedicated_fn_key: None,
                delegate_to_open_axis: self.hiragana_delegate_to_open_axis,
                mode_key_config: None,
                injected_guarded_delegate: true,
            }
        } else if self.katakana_vk == Some(vk_code) {
            ThumbSoloSpecialHandling {
                dedicated_fn_key: None,
                delegate_to_open_axis: self.katakana_delegate_to_open_axis,
                mode_key_config: None,
                injected_guarded_delegate: true,
            }
        } else {
            ThumbSoloSpecialHandling {
                dedicated_fn_key: None,
                delegate_to_open_axis: None,
                mode_key_config: None,
                injected_guarded_delegate: false,
            }
        }
    }

    /// Enter 親指キーのフォールバック挙動を設定する。
    ///
    /// `enter_thumb_vk` は `left_thumb_key`/`right_thumb_key` のいずれかが
    /// Enter (`VK_RETURN`) に解決された場合の VK コード（Platform 層が
    /// `crate::vk::VK_RETURN` との等値比較で判定し渡す）。どちらも Enter でなければ
    /// `None` を渡すこと。`config` は `GeneralConfig::enter_thumb_ignore_composing_guard`/
    /// `enter_thumb_shift_literal` にそのまま対応する（ADR-092 決定B、`TextKeyConfig`）。
    pub const fn set_enter_thumb_config(
        &mut self,
        enter_thumb_vk: Option<VkCode>,
        config: TextKeyConfig,
    ) {
        self.enter_thumb_vk = enter_thumb_vk;
        self.text_key_enter = config;
    }

    /// 親指+小指シフト複合面を有効化/無効化する。
    ///
    /// 親指キー自体が Shift 修飾キーに割り当てられている構成では、親指押下だけで
    /// Shift レベルが立つため、Platform 層は false を渡すこと。
    pub const fn set_thumb_shift_faces_enabled(&mut self, enabled: bool) {
        self.thumb_shift_faces_enabled = enabled;
    }

    /// ソロ連打によるエンジン OFF 要求を取り出す（1ショット）。
    pub(super) fn take_engine_off_requested(&mut self) -> bool {
        std::mem::take(&mut self.engine_off_requested)
    }

    pub fn set_threshold_ms(&mut self, ms: u32) {
        self.threshold_us = u64::from(ms) * 1000;
    }

    /// 確定モードと投機出力の待機時間を更新する。
    pub fn set_confirm_mode(&mut self, mode: ConfirmMode, speculative_delay_ms: u32) {
        self.confirm_mode = mode;
        self.speculative_delay_us = u64::from(speculative_delay_ms) * 1000;
    }

    /// n-gram モデルを設定する。
    ///
    /// 設定すると、同時打鍵判定の閾値が候補文字の出現頻度に応じて動的に調整される。
    pub fn set_ngram_model(&mut self, model: NgramModel) {
        self.ngram_model = Some(model);
    }

    /// タイミング判定器を構築するヘルパー
    pub(crate) fn timing_judge(&self) -> timing::TimingJudge<'_> {
        timing::TimingJudge::new(
            self.threshold_us,
            self.ngram_model.as_ref(),
            self.output_history.recent_kana(timing::NGRAM_CONTEXT_SIZE),
        )
        .with_margins(self.timing_margin_percent, self.min_overlap_margin_percent)
    }

    /// 3キー仲裁・重なり判定のタイミングマージンを更新する
    /// （`GeneralConfig::timing_margin_percent`/`min_overlap_margin_percent`）。
    /// `bootstrap` が起動直後に必ず一度呼ぶため、これが実運用での実質的な
    /// 既定値決定点になる（`min_overlap_margin_percent` フィールドの doc 参照）。
    pub fn set_timing_margins(
        &mut self,
        timing_margin_percent: u32,
        min_overlap_margin_percent: u32,
    ) {
        self.timing_margin_percent = u64::from(timing_margin_percent);
        self.min_overlap_margin_percent = u64::from(min_overlap_margin_percent);
    }

    /// `GeneralConfig` の調整可能フィールドを一括反映する。
    ///
    /// プラットフォームエントリポイント（`bootstrap.rs`/`awase-linux`/
    /// `awase-macos`）がそれぞれ個別に `set_timing_margins` 呼び出しを
    /// コピペしていたが、この重複自体が `awase-linux`/`awase-macos` での
    /// 呼び忘れの原因になった（/code-review指摘、PR #127、7回目）。
    /// 将来 `GeneralConfig` に他のFSM調整項目が増えたら、ここに追加すれば
    /// 全プラットフォームへ自動的に反映される。
    pub fn apply_general_config(&mut self, config: &GeneralConfig) {
        self.set_timing_margins(
            config.timing_margin_percent,
            config.min_overlap_margin_percent,
        );
    }

    /// テスト用: 重なり不足判定のマージンだけを上書きする
    /// （`timing_margin_percent` には触れない、bare `NicolaFsm` レベルの
    /// 既存テスト群が構築直後の値に依存しているため）。
    /// 本番既定は `RUNTIME_MIN_OVERLAP_MARGIN_PERCENT`（ADR-112決定1）。
    #[cfg(test)]
    pub(crate) fn set_min_overlap_margin_percent_for_test(&mut self, pct: u64) {
        self.min_overlap_margin_percent = pct;
    }

    /// 配列を動的に差し替える。保留中のキーがあれば安全にフラッシュする。
    pub fn swap_layout(&mut self, layout: YabLayout) -> Resp {
        let mut flush_resp = self.flush_pending(
            ContextChange::LayoutSwapped,
            ComposingHint::Trusted(self.phys.composing),
        );
        self.layout = layout;
        self.clear_output_history_appending_releases(&mut flush_resp);
        flush_resp
    }

    /// `output_history` を丸ごとクリアする際、`pending_releases` に残っていた
    /// `KeyAction::Key(vk)` エントリの `KeyUp(vk)` を `resp.actions` に追記して
    /// から破棄する（`toggle_enabled`/`swap_layout` 共通、ADR-112コードレビュー
    /// 指摘）。素朴に `output_history.clear()` するだけだと、注入済みVKに
    /// 対応するKeyUpが二度と送られず、エンジン無効化/配列切替のタイミングで
    /// キーを押しっぱなしにしていた場合にOS側で押されっぱなしになる
    /// （stuck keyの再発）。
    fn clear_output_history_appending_releases(&mut self, resp: &mut Resp) {
        let keyups = self.output_history.drain_pending_releases_as_keyups();
        resp.actions.extend(keyups);
        // drain_pending_releases_as_keyups が pending_releases を既に空にして
        // いるため、ここでは committed のみを clear する（clear() の二重
        // clear だと曖昧に見える、/code-review 指摘）。
        self.output_history.clear_committed();
    }
}

// ── 内部ユーティリティ ──
impl NicolaFsm {
    /// Face 列挙値に対応する YabFace への参照を返す
    pub(crate) const fn get_face(&self, face: Face) -> &YabFace {
        match face {
            Face::Normal => &self.layout.normal,
            Face::LeftThumb => &self.layout.left_thumb,
            Face::RightThumb => &self.layout.right_thumb,
            Face::Shift => &self.layout.shift,
            Face::LeftThumbShift => &self.layout.left_thumb_shift,
            Face::RightThumbShift => &self.layout.right_thumb_shift,
        }
    }

    /// scan_code から PhysicalPos を経由して YabFace を引き、`KeyAction` と
    /// 事前解決済みの仮名文字を返す。
    #[allow(clippy::unused_self)]
    pub(crate) fn lookup_face(
        &self,
        pos: Option<PhysicalPos>,
        face: &YabFace,
    ) -> Option<(KeyAction, Option<char>)> {
        let value = face.get(&pos?)?;
        let kana = match value {
            YabValue::Romaji { kana, .. } => *kana,
            YabValue::Literal(s) => s.chars().next(),
            _ => None,
        };
        Some((KeyAction::from(value), kana))
    }

    /// `Face` に対応する面でキー位置を引き、仮名文字のみを返す。
    fn lookup_kana_at(&self, pos: Option<PhysicalPos>, face: Face) -> Option<char> {
        self.lookup_face(pos, self.get_face(face))
            .and_then(|(_, k)| k)
    }

    /// `pos` が現在の Shift レベルで複合面（LeftThumbShift/RightThumbShift）に
    /// 定義されているかを返す。`classify_idle_intent` が「和音の成立を複合面定義の
    /// あるキーに限って待つ」ために使う（下記参照）。
    fn thumb_shift_face_defines(&self, pos: Option<PhysicalPos>) -> bool {
        let Some(pos) = pos else { return false };
        self.thumb_shift_faces_enabled
            && (self.layout.left_thumb_shift.contains_key(&pos)
                || self.layout.right_thumb_shift.contains_key(&pos))
    }

    /// 親指側と現在の Shift レベルから、部分定義に対応した実際の親指面を解決する。
    ///
    /// 複合面が無い、またはそのキー位置が未定義なら従来の親指面へフォールバックする。
    /// `YabValue::None` は `YabFace::contains_key` 上は定義ありとして扱い、明示的に
    /// フォールバックを遮断する。
    fn resolve_thumb_face(&self, side: ThumbSide, pos: Option<PhysicalPos>) -> Option<Face> {
        let pos = pos?;
        let shift_held = self.thumb_shift_faces_enabled && self.phys.modifiers.shift;
        let preferred = Face::resolve(Some(side), shift_held);
        if self.get_face(preferred).contains_key(&pos) {
            return Some(preferred);
        }
        let fallback = Face::resolve(Some(side), false);
        if self.get_face(fallback).contains_key(&pos) {
            Some(fallback)
        } else {
            None
        }
    }

    /// `PendingCharThumb` 状態で char1+thumb を同時打鍵として解決し、アクション列と OutputUpdate を返す。
    ///
    /// 親指キーの物理押下状態を「消費」する。消費後は `active_thumb_side()` が `None` を
    /// 返すようになり、後続のキーが同じ親指押下で二重にシフトされるのを防ぐ。
    fn resolve_char_thumb_as_simultaneous(
        &mut self,
        char_key: &PendingKey,
        thumb_face: Face,
    ) -> ResolvedAction {
        if let Some((action, kana)) = self.lookup_face(char_key.pos, self.get_face(thumb_face)) {
            // 親指キーを「消費」: 同じ物理押下で後続キーがシフトされないようにする
            self.consume_thumb(thumb_face);
            let output = OutputUpdate::record(char_key.scan_code, &action, kana);
            ResolvedAction {
                actions: smallvec![action],
                output,
            }
        } else {
            // 親指面に定義がない場合は文字キーを単独確定
            self.resolve_pending_char_as_single(char_key)
        }
    }

    /// 親指キーを同時打鍵に「消費済み」とマークし、同じ押下の再利用を防ぐ。
    ///
    /// 現在の物理押下タイムスタンプを記録する。物理状態が変われば（新しい KeyDown
    /// や KeyUp）タイムスタンプが不一致になり、自動的に「未消費」に戻る。
    /// 同時打鍵として消費されたことはソロ連打ではないため、ソロ連打カウンターをリセットする。
    const fn consume_thumb(&mut self, face: Face) {
        match face.thumb_side() {
            Some(ThumbSide::Left) => self.left_thumb_consumed = self.phys.left_thumb_down,
            Some(ThumbSide::Right) => self.right_thumb_consumed = self.phys.right_thumb_down,
            None => {} // 親指面以外は消費対象なし
        }
        self.solo_counter.reset();
    }

    pub(crate) const fn enter_pending_char(&mut self, key: PendingKey) {
        self.state = EngineState::PendingChar(key);
    }

    pub(crate) const fn enter_pending_thumb(&mut self, thumb: PendingThumbData) {
        self.state = EngineState::PendingThumb(thumb);
    }

    const fn enter_pending_char_thumb(&mut self, char_key: PendingKey, thumb: PendingThumbData) {
        self.state = EngineState::PendingCharThumb {
            char_key,
            thumb,
            char1_released_at: None,
        };
    }

    /// 投機出力の開始を試みる（ADR-115 決定7）。`Sequence`（複数出力・
    /// 複数 composition unit）は `retract_bs_count` が前提とする
    /// 「BACKSPACE 1発で取り消せる」性質を持たないため拒否する。
    /// 呼び出し元は戻り値 `false` を「投機せず、`PendingChar` のまま
    /// Wait モード相当の確定を待つ」として扱うこと（`go_idle()`+
    /// `pass_through()` は使わない——打鍵列の消失・生VK漏洩を招く）。
    ///
    /// 既に引いた `action` を受け取ることで二重 `lookup_face` を避ける
    /// ——呼び出し元の `face` 変数と本関数がハードコードする面がズレる
    /// 余地も消える。
    pub(crate) const fn enter_speculative_char(
        &mut self,
        key: PendingKey,
        action: &KeyAction,
    ) -> bool {
        if matches!(action, KeyAction::Sequence(_)) {
            return false;
        }
        self.state = EngineState::SpeculativeChar(key);
        true
    }

    /// output_history から `scan_code` のエントリを取り出し、Key(vk) なら KeyUp(vk) を `actions` に追記する。
    ///
    /// Char/Romaji は Down+Up 一括送信済みのため、Key(vk) のみが追記対象。
    fn append_key_up_for(&mut self, actions: &mut SmallVec<[KeyAction; 2]>, scan_code: ScanCode) {
        if let Some(KeyAction::Key(vk)) = self.output_history.remove_by_scan(scan_code) {
            actions.push(KeyAction::KeyUp(vk));
        }
    }

    /// アクション列・consumed フラグ・タイマー指示から `Response` を組み立てる
    pub(crate) fn build_response(
        &self,
        actions: SmallVec<[KeyAction; 2]>,
        consumed: bool,
        timer: TimerIntent,
    ) -> Resp {
        let mut response = if actions.is_empty() && consumed {
            Response::consume()
        } else if actions.is_empty() {
            Response::pass_through()
        } else {
            Response::emit(flatten_actions(actions))
        };
        response.timers = self.timer_cmds(timer);
        response
    }
}

/// `timed_fsm::ParseAction` の具象型エイリアス（ShiftReduceParser 実装用）。
type TieredParseAction = timed_fsm::ParseAction<KeyAction, ClassifiedEvent, usize, OutputUpdate>;

// ── ShiftReduceParser 実装 ──
impl ShiftReduceParser for NicolaFsm {
    type Action = KeyAction;
    type Token = ClassifiedEvent;
    type TimerId = usize;
    type ReduceRecord = OutputUpdate;

    fn decide(&mut self, token: &ClassifiedEvent) -> TieredParseAction {
        let local = self.decide_and_transition(token);
        match local {
            ParseAction::Shift { timer } => TieredParseAction::Shift {
                timers: self.timer_cmds(timer),
            },
            ParseAction::Reduce {
                actions,
                record,
                timer,
            } => TieredParseAction::Reduce {
                actions: flatten_actions(actions),
                record,
                timers: self.timer_cmds(timer),
            },
            ParseAction::ReduceAndContinue {
                actions,
                record,
                remaining,
            } => TieredParseAction::ReduceAndContinue {
                actions: flatten_actions(actions),
                record,
                remaining,
            },
            ParseAction::PassThrough { timer } => TieredParseAction::PassThrough {
                timers: self.timer_cmds(timer),
            },
        }
    }

    fn on_reduce(&mut self, record: OutputUpdate) {
        // `ShiftReduceParser::on_reduce` はトレイトのシグネチャ上イベント固有の
        // タイムスタンプを受け取れない。`on_key_down` 冒頭の `update_timing` が
        // `last_key_timestamp` を現在処理中のイベントの値へ同期的に更新して
        // いるため、`parse()` 経由でこの `on_reduce` に至る通常経路では現在の
        // イベントのタイムスタンプと一致する（ADR-120 決定0a の集計専用の
        // 精度要件であり、厳密な取りこぼしが実害を持つ他の用途とは異なる）。
        self.update_history(record, self.last_key_timestamp.unwrap_or(0));
    }
}

// ── KeyDown ディスパッチ ──
impl NicolaFsm {
    /// AdaptiveTiming 用: 直前キーとの間隔を算出してタイムスタンプを更新する
    fn update_timing(&mut self, event: &RawKeyEvent) {
        self.last_key_gap_us = self
            .last_key_timestamp
            .map(|prev| event.timestamp.saturating_sub(prev));
        self.last_key_timestamp = Some(event.timestamp);
    }

    /// Shift 面を使うべきかどうかを判定する
    const fn should_use_shift_plane(&self, ev: &ClassifiedEvent) -> bool {
        self.phys.modifiers.shift && !ev.key_class.is_thumb()
    }

    /// ADR-120 決定0a 項目2c: Phase2決定直後の「親指の有無」観測窓を消費する。
    /// 実際の変換結果には一切影響しない。
    fn observe_thumb_watch_window(&mut self, event: &RawKeyEvent) {
        let Some(mut window) = self.thumb_watch_window.take() else {
            return;
        };
        let ev = self.phys.classified;
        if ev.key_class.is_thumb() {
            // 親指が来た → 窓不成立、破棄（no_thumb_followup_count は増やさない）
            self.retro_eval_stats.thumb_watch_window_thumb_arrived_count += 1;
            return;
        }
        // 所見S6: 窓が数えるのは「後続の打鍵」であって「親指以外の何か」では
        // ない。`KeyClass::Passthrough`（Shift/Ctrl/矢印/BACKSPACE等の修飾・
        // ナビゲーションキー）を1打鍵として消費すると、実質1文字しか
        // 打っていないのに窓が閉じてしまい `no_thumb_followup_count` を
        // 過大評価する（ADR-120の判断点1ゲート(iii)がこの値で機構全体の
        // 可否を左右するため、過大評価は「作らなくてよい機構を誤って作る」
        // 方向に倒れる）。`Char` キーだけを消費対象にする。
        if ev.key_class != KeyClass::Char {
            self.thumb_watch_window = Some(window);
            return;
        }
        if window.last_vk == Some(event.vk_code) && window.last_vk_down {
            // OS のオートリピートによる同一キーの KeyDown 再送
            // （まだ物理的に押下中、`on_key_up` でクリアされていない）。
            // 新規タップではないためカウントを進めず、窓の残数もそのまま
            // 維持する（`handle_bypass` の `engine_off_extra_key_suppressed`
            // と同型）。
            self.thumb_watch_window = Some(window);
            return;
        }
        window.last_vk = Some(event.vk_code);
        window.last_vk_down = true;
        if window.remaining <= 1 {
            self.retro_eval_stats.no_thumb_followup_count += 1;
        } else {
            window.remaining -= 1;
            self.thumb_watch_window = Some(window);
        }
    }

    fn on_key_down(&mut self, event: &RawKeyEvent) -> Resp {
        self.update_timing(event);

        // ADR-120 決定0a 項目7(a) (所見NB1対応): 物理BACKSPACE以外のKeyDownが
        // 来たら `backspace_down` を自己修復的にクリアする。エンジンが
        // 非活性化（IME OFF・非日本語IMEアプリへのフォーカス移動等）した
        // 際、物理BACKSPACEのKeyUpが `Engine::on_input` Phase 2 で
        // `Decision::pass_through()` として捨てられ `on_key_up` に一切
        // 到達しないことがあり、`on_key_up` 側のクリアだけに頼ると
        // `backspace_down` が `true` のまま永久に固着し、以後の項目7(a)集計が
        // 無言で全滅する（2026-08-26 の `engine_off_extra_key` ラッチ固着と
        // 同型のバグ）。OSオートリピートは「最後に押されたキー」だけを
        // 再送するため、他キーのKeyDownはBACKSPACEが既に離されたことの
        // 十分な証拠になり、新しいタイミング定数も不要
        // （`.claude/rules/tuning-constants.md` に抵触しない）。
        //
        // **`flush_pending` の汎用リセットには入れないこと**: `handle_bypass`
        // はBACKSPACE自身の処理中に（保留キーがあれば）`flush_pending` を
        // 呼ぶため、セットした同じ呼び出し内でクリアされてしまい
        // オートリピート抑止が丸ごと無効化される（`toggle_enabled` の
        // `:636-642` が記録する2026-08-26 report1と同型の罠）。
        if !self.backspace_vk.is_some_and(|vk| event.vk_code == vk) {
            self.backspace_down = false;
        }

        self.observe_thumb_watch_window(event);

        // Bypass check: modifiers, IME control, OS shortcuts.
        // Handled before the parser loop because bypass needs consumed=false
        // even when flush actions are emitted.
        let ev = self.phys.classified;
        if let Some(reason) = self.bypass_reason(&ev) {
            return self.handle_bypass(&ev, reason, event.injected);
        }

        self.parse(ev)
    }

    /// 状態とイベントに基づいてアクションを決定し、状態遷移を行う
    fn decide_and_transition(&mut self, ev: &ClassifiedEvent) -> ParseAction {
        // State-based dispatch (bypass is handled in on_key_down before entering the loop)
        match self.state {
            EngineState::Idle => self.decide_idle(ev),
            EngineState::PendingChar(_) => self.decide_pending_char(ev),
            EngineState::PendingThumb(_) => self.decide_pending_thumb(ev),
            EngineState::PendingCharThumb { .. } => self.step_pending_char_thumb_3key(ev),
            EngineState::SpeculativeChar(_) => self.decide_speculative(ev),
        }
    }

    /// Shift 面で Reduce する共通ヘルパー
    ///
    /// `.yab` の Shift 面に定義された値をそのまま IME 経由で確定出力する
    /// （通常の Reduce 経路と同じ、`lookup_face` が返す `KeyAction`/kana をそのまま使う）。
    /// 定義が無いキーは OS に素通しする。
    ///
    /// 2026年3月導入（`72bd118`）の Shift 面ルーティング機構そのもの。
    /// 「Shift 押しっぱなしで IME-ON 半角英数 hold」（BUG-15、`shift_plane_halfwidth`）の
    /// PassThrough/EmitText 分岐はここに実装されていたが、左Shift単独タップによる
    /// 持続トグル方式へ置き換えたため撤去した（2026-07-11、`docs/known-bugs.md`
    /// BUG-15 参照）。撤去後は本関数がこの本来の姿（.yab の値をそのまま Reduce）に戻る。
    fn shift_face_reduce(&self, ev: &ClassifiedEvent) -> ParseAction {
        let face = self.get_face(Face::Shift);
        if let Some((action, kana)) = self.lookup_face(ev.pos, face) {
            ParseAction::Reduce {
                actions: smallvec![action.clone()],
                record: OutputUpdate::record(ev.scan_code, &action, kana),
                timer: TimerIntent::CancelAll,
            }
        } else {
            ParseAction::PassThrough {
                timer: TimerIntent::Keep,
            }
        }
    }

    /// Space 親指キーを Shift と同時に押した場合、同時打鍵判定を一切試みず
    /// 即座にリテラルなスペースとして送出すべきかを判定する。
    ///
    /// NICOLA の小指シフト面（Shift 単独系）と親指シフト（同時打鍵系）はそもそも
    /// 組み合わせない設計のため、Shift 押下中の Space 親指キーを `PendingThumb` に
    /// 入れず即座に素通しにしても、通常の同時打鍵判定と衝突しない。
    const fn is_space_thumb_shift_literal(&self, ev: &ClassifiedEvent) -> bool {
        self.text_key_space.shift_literal
            && self.phys.modifiers.shift
            && ev.key_class.is_thumb()
            && matches!(self.space_thumb_vk, Some(vk) if vk.0 == ev.vk_code.0)
    }

    /// Enter 親指キーを Shift と同時に押した場合、同時打鍵判定を一切試みず
    /// 即座にリテラルな Enter（Shift+Enter のソフト改行）として送出すべきかを判定する。
    /// `is_space_thumb_shift_literal` と同じ理由付け（NICOLA の小指シフト面とは
    /// 組み合わせない設計）。
    const fn is_enter_thumb_shift_literal(&self, ev: &ClassifiedEvent) -> bool {
        self.text_key_enter.shift_literal
            && self.phys.modifiers.shift
            && ev.key_class.is_thumb()
            && matches!(self.enter_thumb_vk, Some(vk) if vk.0 == ev.vk_code.0)
    }

    /// Idle 状態でのキー到着時の意図を分類する（純粋関数）。
    fn classify_idle_intent(&self, ev: &ClassifiedEvent) -> IdleIntent {
        // Shift+Space literal: 明示的なスペース入力のエスケープハッチ（最優先）。
        if self.is_space_thumb_shift_literal(ev) {
            return IdleIntent::PassThrough;
        }
        // Shift+Enter literal: 明示的なソフト改行のエスケープハッチ（同上）。
        if self.is_enter_thumb_shift_literal(ev) {
            return IdleIntent::PassThrough;
        }
        // Active thumb combo
        if !ev.key_class.is_thumb() {
            if let Some(side) = self.active_thumb_side() {
                if let Some(face) = self.resolve_thumb_face(side, ev.pos) {
                    return IdleIntent::ActiveThumb(face);
                }
                // 親指面に定義がない → Shift 面または確定モードに委譲（fall through）
            }
        }
        // Shift plane（複合面がこの位置を定義しているときだけ、和音の成立を待つために譲る。
        // それ以外は ADR-097 決定2どおり即座に Shift 面へ確定する——shift_face_reduce
        // 自身が未定義キーを PassThrough させるため、ここで追加のガードは要らない）
        if self.should_use_shift_plane(ev) && !self.thumb_shift_face_defines(ev.pos) {
            return IdleIntent::ShiftPlane;
        }
        // Non-layout key
        if !ev.key_class.is_thumb() && !self.is_layout_key(ev.pos) {
            return IdleIntent::PassThrough;
        }
        // Confirm mode dispatch
        IdleIntent::ConfirmMode
    }

    /// Idle 状態でのキー押下処理
    fn decide_idle(&mut self, ev: &ClassifiedEvent) -> ParseAction {
        match self.classify_idle_intent(ev) {
            IdleIntent::ShiftPlane => self.shift_face_reduce(ev),
            IdleIntent::ActiveThumb(face) => self.reduce_active_thumb(ev, face),
            IdleIntent::PassThrough => ParseAction::PassThrough {
                timer: TimerIntent::Keep,
            },
            IdleIntent::ConfirmMode => self.dispatch_confirm_mode(ev),
        }
    }

    /// 未消費の親指キーが押下中の場合に親指面で即時確定する。
    fn reduce_active_thumb(&mut self, ev: &ClassifiedEvent, face: Face) -> ParseAction {
        if let Some((action, kana)) = self.lookup_face(ev.pos, self.get_face(face)) {
            // 親指を消費: 同じ押下で後続キーが二重シフトされるのを防ぐ
            self.consume_thumb(face);
            ParseAction::Reduce {
                actions: smallvec![action.clone()],
                record: OutputUpdate::record(ev.scan_code, &action, kana),
                timer: TimerIntent::CancelAll,
            }
        } else {
            // classify_idle_intent が lookup 成功を確認済みなのでここには来ないが、
            // 安全側に倒して確定モードに委譲する。
            self.dispatch_confirm_mode(ev)
        }
    }

    /// PendingChar 状態でのキー押下処理
    fn decide_pending_char(&mut self, ev: &ClassifiedEvent) -> ParseAction {
        match ev.key_class {
            KeyClass::LeftThumb | KeyClass::RightThumb => self.step_pending_char_thumb(ev),
            KeyClass::Char => self.step_pending_char_char(ev),
            KeyClass::Passthrough => {
                // Passthrough key (e.g. ENTER) arrived while a char is pending.
                // Flush the pending char first so it reaches IME before the passthrough key.
                let pending = self.state.expect_pending_char();
                log::debug!(
                    "[passthrough-flush] pending=PendingChar(vk={:#04x}) → flush, then reprocess passthrough_vk={:#04x} ts={}us",
                    pending.vk_code.0,
                    ev.vk_code.0,
                    ev.timestamp,
                );
                self.go_idle();
                let resolved = self.resolve_pending_char_as_single(&pending);
                resolved.into_reduce_and_continue(*ev)
            }
        }
    }

    /// PendingThumb 状態でのキー押下処理
    fn decide_pending_thumb(&mut self, ev: &ClassifiedEvent) -> ParseAction {
        match ev.key_class {
            KeyClass::Char => self.step_pending_thumb_char(ev),
            KeyClass::LeftThumb | KeyClass::RightThumb => self.step_pending_thumb_thumb(ev),
            KeyClass::Passthrough => {
                // Passthrough key arrived while a thumb is pending.
                // Flush the pending thumb first so it reaches IME before the passthrough key.
                let thumb = self.state.expect_pending_thumb();
                log::debug!(
                    "[passthrough-flush] pending=PendingThumb(vk={:#04x}) → flush, then reprocess passthrough_vk={:#04x} ts={}us",
                    thumb.vk_code.0,
                    ev.vk_code.0,
                    ev.timestamp,
                );
                self.go_idle();
                let (resolved, ime_open_request) = self.resolve_pending_thumb_as_single(
                    thumb.scan_code,
                    thumb.vk_code,
                    thumb.modifier_key,
                    thumb.injected,
                    self.phys.composing,
                );
                if ime_open_request.is_some() {
                    self.ime_open_requested = ime_open_request;
                }
                resolved.into_reduce_and_continue(*ev)
            }
        }
    }

    /// SpeculativeChar 状態でのキー押下処理
    fn decide_speculative(&mut self, ev: &ClassifiedEvent) -> ParseAction {
        match ev.key_class {
            KeyClass::LeftThumb | KeyClass::RightThumb => self.step_speculative_thumb(ev),
            KeyClass::Char | KeyClass::Passthrough => {
                // 投機出力は正しかった → Idle に戻って再処理
                self.go_idle();
                ParseAction::ReduceAndContinue {
                    actions: SmallVec::new(),
                    record: OutputUpdate::None,
                    remaining: *ev,
                }
            }
        }
    }
}

// ── 同時打鍵解決 ──
impl NicolaFsm {
    /// 投機出力を取り消して新しい出力に差し替える。
    ///
    /// 前提: IME は完結済みローマ字を1つの変換単位として扱うため、
    /// BACKSPACE 1発で投機出力全体を削除できる。
    ///
    /// `RetractAndRecord` を使うことで、retract と record を `update_history()` で
    /// アトミックに処理し、この関数を副作用のない純粋な構築関数にする。
    fn retract_and_replace(
        pending: PendingKey,
        new_action: &KeyAction,
        kana: Option<char>,
    ) -> ParseAction {
        let actions = smallvec![
            KeyAction::SpecialKey(SpecialKey::Backspace),
            new_action.clone(),
        ];
        ParseAction::Reduce {
            actions,
            record: OutputUpdate::RetractAndRecord(OutputEntry {
                scan_code: pending.scan_code,
                romaji: new_action.romaji().to_owned(),
                kana,
                action: new_action.clone(),
            }),
            timer: TimerIntent::CancelAll,
        }
    }

    /// 投機出力済み状態で親指キーが到着した場合の処理。
    ///
    /// `SpeculativeChar` 状態では通常面の文字が既に IME に送信されている。
    /// 親指キーが閾値時間内に到着した場合、`retract_and_replace()` で出力を差し替える。
    ///
    /// 閾値超過時や親指面に定義がない場合は、投機出力は正しかったとみなし、
    /// Idle に戻って親指キーを新規イベントとして再処理する。
    fn step_speculative_thumb(&mut self, ev: &ClassifiedEvent) -> ParseAction {
        let pending = self.state.expect_speculative_char();
        let side = if ev.key_class.is_left_thumb() {
            ThumbSide::Left
        } else {
            ThumbSide::Right
        };

        // Look up what the simultaneous keystroke would produce
        if let Some(face) = self.resolve_thumb_face(side, pending.pos) {
            if let Some((thumb_action, thumb_kana)) =
                self.lookup_face(pending.pos, self.get_face(face))
            {
                if self
                    .timing_judge()
                    .is_simultaneous(pending.timestamp, ev.timestamp, thumb_kana)
                {
                    // Within threshold → retract speculative output + emit thumb face

                    // 親指を消費: 同じ押下で後続キーが二重シフトされるのを防ぐ
                    self.consume_thumb(face);
                    self.go_idle();
                    return Self::retract_and_replace(pending, &thumb_action, thumb_kana);
                }
                // Outside threshold → speculative was correct, process thumb as new key
            }
        }
        // No thumb face entry → speculative was correct
        // Go idle and re-process the thumb key
        self.go_idle();
        ParseAction::ReduceAndContinue {
            actions: SmallVec::new(),
            record: OutputUpdate::None,
            remaining: *ev,
        }
    }

    /// PendingChar + 親指キー → 同時打鍵候補（閾値内なら PendingCharThumb、超過なら flush+新規）
    fn step_pending_char_thumb(&mut self, ev: &ClassifiedEvent) -> ParseAction {
        let pending = self.state.expect_pending_char();
        // 親指面で保留文字キーの候補を取得し閾値を調整
        let side = if ev.key_class.is_left_thumb() {
            ThumbSide::Left
        } else {
            ThumbSide::Right
        };
        let candidate = self
            .resolve_thumb_face(side, pending.pos)
            .and_then(|face| self.lookup_face(pending.pos, self.get_face(face)));
        let candidate_kana = candidate.as_ref().and_then(|(_, kana)| *kana);

        if self
            .timing_judge()
            .is_simultaneous(pending.timestamp, ev.timestamp, candidate_kana)
        {
            // 保留=文字, 到着=親指 → PendingCharThumb へ遷移（3 鍵目を待つ）
            self.enter_pending_char_thumb(
                pending,
                PendingThumbData {
                    scan_code: ev.scan_code,
                    vk_code: ev.vk_code,
                    is_left: ev.key_class.is_left_thumb(),
                    timestamp: ev.timestamp,
                    injected: ev.injected,
                    modifier_key: ev.modifier_key,
                },
            );
            return ParseAction::Shift {
                timer: TimerIntent::Pending,
            };
        }

        // 時間超過 → 前の保留を単独確定し、今回のキーを再処理
        self.go_idle();
        let resolved = self.resolve_pending_char_as_single(&pending);
        resolved.into_reduce_and_continue(*ev)
    }

    /// PendingChar + 文字キー → 前の保留を単独確定し、今回のキーを再処理
    fn step_pending_char_char(&mut self, ev: &ClassifiedEvent) -> ParseAction {
        let pending = self.state.expect_pending_char();
        self.go_idle();
        let resolved = self.resolve_pending_char_as_single(&pending);
        resolved.into_reduce_and_continue(*ev)
    }

    /// PendingThumb + 文字キー → 同時打鍵候補（閾値内なら即時確定、超過なら flush+新規）
    fn step_pending_thumb_char(&mut self, ev: &ClassifiedEvent) -> ParseAction {
        let thumb = self.state.expect_pending_thumb();
        // 親指面で到着文字キーの候補を取得し閾値を調整
        let pending_face = self.resolve_thumb_face(thumb.side(), ev.pos);
        let candidate = pending_face.and_then(|face| self.lookup_face(ev.pos, self.get_face(face)));
        let candidate_kana = candidate.as_ref().and_then(|(_, kana)| *kana);

        if self
            .timing_judge()
            .is_simultaneous(thumb.timestamp, ev.timestamp, candidate_kana)
        {
            if let Some((action, kana)) = candidate {
                // 保留=親指, 到着=文字 → 同時打鍵
                // 親指を消費: 同じ押下で後続キーが二重シフトされるのを防ぐ
                if let Some(face) = pending_face {
                    self.consume_thumb(face);
                }
                self.go_idle();
                return ParseAction::Reduce {
                    actions: smallvec![action.clone()],
                    record: OutputUpdate::record(ev.scan_code, &action, kana),
                    timer: TimerIntent::CancelAll,
                };
            }
        }

        // 時間超過 or 候補なし → 前の保留を単独確定し、今回のキーを再処理
        self.go_idle();
        let (resolved, ime_open_request) = self.resolve_pending_thumb_as_single(
            thumb.scan_code,
            thumb.vk_code,
            thumb.modifier_key,
            thumb.injected,
            self.phys.composing,
        );
        if ime_open_request.is_some() {
            self.ime_open_requested = ime_open_request;
        }
        resolved.into_reduce_and_continue(*ev)
    }

    /// PendingThumb + 親指キー → 前の保留を単独確定し、今回のキーを再処理
    fn step_pending_thumb_thumb(&mut self, ev: &ClassifiedEvent) -> ParseAction {
        let thumb = self.state.expect_pending_thumb();
        self.go_idle();
        let (resolved, ime_open_request) = self.resolve_pending_thumb_as_single(
            thumb.scan_code,
            thumb.vk_code,
            thumb.modifier_key,
            thumb.injected,
            self.phys.composing,
        );
        if ime_open_request.is_some() {
            self.ime_open_requested = ime_open_request;
        }
        resolved.into_reduce_and_continue(*ev)
    }

    /// OutputUpdate に基づいて出力履歴を更新する共通ヘルパー。
    ///
    /// `now` は ADR-120 決定0a の集計（項目4・項目7）専用のタイムスタンプで、
    /// `self.last_key_timestamp` の使い回しではなく呼び出し元が握っている
    /// イベント固有の値を渡すこと（過小評価バイアスを避けるため）。
    pub(crate) fn update_history(&mut self, output: OutputUpdate, now: Timestamp) {
        self.record_retro_eval_stats(&output, now, true);
        self.apply_output_history(output);
    }

    /// `update_history` と同じだが、`now` がタイムアウト/flush等の経路由来で
    /// 実際のイベント発生時刻ではない（`self.last_key_timestamp` の使い回し）
    /// 場合に使う（should-fix所見B2対応）。項目4「後続1かな確定」の経過ms
    /// 計測は、この不正確な `now` では誤って過小評価される
    /// （PairWithChar1分岐で char2 がタイムアウト経由で単独確定した場合に
    /// elapsed が常に0msになっていたバグ）ため記録せず破棄する。
    /// `mark_decision`/`record_user_correction`（項目7）は許容できる精度の
    /// 低下として引き続き記録する——これらは分オーダーの `STALE_ATTRIBUTION_MS`
    /// 窓判定であり、項目4ほど小さい時間スケールの精度を要求しないため。
    fn update_history_imprecise(&mut self, output: OutputUpdate, now: Timestamp) {
        self.record_retro_eval_stats(&output, now, false);
        self.apply_output_history(output);
    }

    fn apply_output_history(&mut self, output: OutputUpdate) {
        match output {
            OutputUpdate::Record(entry) => {
                self.output_history.push(entry);
            }
            OutputUpdate::RetractAndRecord(entry) => {
                self.output_history.retract_and_record(entry);
            }
            OutputUpdate::None => {}
        }
    }

    /// ADR-120 決定0a 項目4・7・7(b)・7(c): `output` が実際の履歴に反映される
    /// 前に、集計専用カウンタを更新する。実際の変換結果には一切影響しない。
    /// `precise` が `false` の場合、項目4の経過ms計測は記録しない
    /// （`update_history_imprecise` 参照、所見B2対応）。
    fn record_retro_eval_stats(&mut self, output: &OutputUpdate, now: Timestamp, precise: bool) {
        let Some(entry) = output.entry_ref() else {
            return;
        };

        // 項目7除外条件の判定は own_decision_output を変異させる前に行う
        // （このentry自身が「3鍵仲裁の決定自身の出力」の途中かどうかは、
        // 今回の消費より前の状態で決まる）。
        let is_own_decision_output = self.own_decision_output.is_some_and(|s| s.remaining > 0);

        // 項目7(b)(c): 配列セル由来のBackspace/Escape出力。
        // **`/code-review` 指摘**: 3キー仲裁決定「自身の出力」が
        // `SpecialKey::Backspace`/`Escape` に解決される .yab 配列も存在しうる
        // （例: 決定自体がBackspaceを送出する配列セル）。この場合の出力は
        // ユーザーの訂正操作ではなく決定そのものの結果なので、
        // `is_own_decision_output` の間は計上しない
        // （elapsed≈0msの見せかけの自己訂正がbucket 0に混入するのを防ぐ）。
        if !is_own_decision_output {
            match &entry.action {
                KeyAction::SpecialKey(SpecialKey::Backspace) => self.record_user_correction(now),
                KeyAction::SpecialKey(SpecialKey::Escape) => {
                    self.retro_eval_stats.escape_output_count += 1;
                }
                _ => {}
            }
        }

        // 項目4/7: own_decision_output の消費。出力の種類を問わず1出力ごとに
        // 1減算する（所見S2対応——以前はChar/Romaji以外の出力ではデクリメント
        // しておらず、`Key(vk)`/`Suppress`等の自前出力があるとスキップが
        // 残ったまま次の実打鍵の出力を誤って飲み込んでいた）。
        let is_kana_output = matches!(entry.action, KeyAction::Char(_) | KeyAction::Romaji(_));
        if let Some(mut own) = self.own_decision_output.take() {
            if own.remaining > 0 {
                own.remaining -= 1;
                self.own_decision_output = Some(own);
            } else if let Some(since) = own.measure_since {
                if is_kana_output {
                    // remaining==0まで消費済み、かつ今回が最初の「かな」出力
                    // = これが「後続1かな確定」（Phase2決定のみ measure_since が Some）。
                    if precise {
                        let elapsed_ms = now.saturating_sub(since) / 1000;
                        let bucket = retro_eval_stats::bucket_index(elapsed_ms);
                        self.retro_eval_stats.followup_elapsed_ms_histogram[bucket] += 1;
                    } else {
                        // 所見B2: 不正確な now では記録しない。計測は破棄する
                        // （欠測として扱う——誤って elapsed≈0 に丸め込まない）。
                        // 所見NN2: 欠測の発生自体は数えておく（残差ゼロで
                        // phase2_reached と突き合わせられるようにする）。
                        self.retro_eval_stats.followup_dropped_imprecise_count += 1;
                    }
                    // own_decision_output は None のまま（既に take 済み）
                } else {
                    // まだ「かな」ではない非対象出力（Suppress等）。計測継続。
                    self.own_decision_output = Some(own);
                }
            }
            // remaining==0 かつ measure_since==None（Phase1/NoNgram決定）:
            // 何もしない。own_decision_output は None のまま
            // （Baseline除外の役目は済んだので以後は通常の判定に戻る）。
        }

        // 項目7: このentryが「非曖昧な確定」に該当する場合のみBaselineへ計上。
        // 除外条件:
        //   - RetractAndRecord の新エントリ（投機出力の差し替え）
        //   - 3キー仲裁決定（Phase1/Phase2/NoNgramいずれも）「自身の出力」の
        //     最中（own_decision_output が remaining>=1 で保持されている間、
        //     所見S5対応——以前はPhase2決定にしか効かず、Phase1決定の自前
        //     出力がBaselineへ混入していた）
        //   - 上のmatchで既に計上した Backspace/Escape そのもの
        let is_backspace_or_escape = matches!(
            entry.action,
            KeyAction::SpecialKey(SpecialKey::Backspace | SpecialKey::Escape)
        );
        if !output.is_retract_and_record() && !is_own_decision_output && !is_backspace_or_escape {
            self.mark_decision(DecisionKind::Baseline, now);
        }
    }

    /// ADR-120 決定0a 項目7: カテゴリ別「直近の決定」タイムスタンプを更新し、
    /// 対応する分母カウンタを+1する。
    fn mark_decision(&mut self, kind: DecisionKind, now: Timestamp) {
        let last = self.last_decision.get_or_insert_with(Default::default);
        match kind {
            DecisionKind::Phase2 => {
                last.phase2_at = Some(now);
                self.retro_eval_stats.phase2_decisions_total += 1;
            }
            DecisionKind::Phase1 => {
                last.phase1_at = Some(now);
                self.retro_eval_stats.phase1_decisions_total += 1;
            }
            DecisionKind::Baseline => {
                last.baseline_at = Some(now);
                self.retro_eval_stats.baseline_decisions_total += 1;
            }
        }
    }

    /// ADR-120 決定0a 項目7(a)(b): ユーザー訂正操作（物理BACKSPACE or 配列セル
    /// 由来のBackspace出力）が発生した際、カテゴリ別に独立してstale判定した
    /// うえで訂正ヒストグラムへ計上する。1回の訂正が複数カテゴリに計上される
    /// ことを許容する（直近Phase1決定と直近Phase2決定の両方が窓内、など）。
    /// 計上したカテゴリは即座にクリアする（所見S3対応——クリアしないと
    /// BACKSPACE連打・長押しのたびに同じ決定へ何度も計上され、訂正の
    /// 「発生有無」ではなく「消した文字数」を数える集計になってしまう）。
    fn record_user_correction(&mut self, now: Timestamp) {
        let Some(last) = self.last_decision.as_mut() else {
            return;
        };
        if let Some(t) = last.phase2_at.take() {
            let elapsed = now.saturating_sub(t) / 1000;
            if elapsed < retro_eval_stats::STALE_ATTRIBUTION_MS {
                let bucket = retro_eval_stats::bucket_index(elapsed);
                self.retro_eval_stats.phase2_correction_histogram[bucket] += 1;
            }
        }
        if let Some(t) = last.phase1_at.take() {
            let elapsed = now.saturating_sub(t) / 1000;
            if elapsed < retro_eval_stats::STALE_ATTRIBUTION_MS {
                let bucket = retro_eval_stats::bucket_index(elapsed);
                self.retro_eval_stats.phase1_correction_histogram[bucket] += 1;
            }
        }
        if let Some(t) = last.baseline_at.take() {
            let elapsed = now.saturating_sub(t) / 1000;
            if elapsed < retro_eval_stats::STALE_ATTRIBUTION_MS {
                let bucket = retro_eval_stats::bucket_index(elapsed);
                self.retro_eval_stats.baseline_correction_histogram[bucket] += 1;
            }
        }
    }

    /// 保留中の文字キーを単独打鍵として解決し、アクション列と OutputUpdate を返す
    fn resolve_pending_char_as_single(&self, pending: &PendingKey) -> ResolvedAction {
        if let Some((action, kana)) = self.lookup_face(pending.pos, self.get_face(Face::Normal)) {
            let output = OutputUpdate::record(pending.scan_code, &action, kana);
            ResolvedAction {
                actions: smallvec![action],
                output,
            }
        } else {
            // Normal face に定義なし（yab に明示的に '無' がある場合は lookup_face が
            // Some(Suppress) を返すため、ここには来ない）。
            // 配列定義外のキー → Key(vk_code) でそのまま通す
            let action = KeyAction::Key(pending.vk_code);
            let output = OutputUpdate::record(pending.scan_code, &action, None);
            ResolvedAction {
                actions: smallvec![action],
                output,
            }
        }
    }

    /// 保留中の親指キーを単独打鍵として解決し、アクション列と `OutputUpdate` を返す。
    ///
    /// NICOLA では親指キー (無変換 / 変換) は文字キーとの同時打鍵専用であり、
    /// 単独打鍵は本質的に誤打鍵 / 親指の離しの遅れ / 文字キーが間に合わなかった等の
    /// 偶発的なケース。元の VK_NONCONVERT / VK_CONVERT を OS に送ってしまうと
    /// IME 側で カタカナトグル等の副作用（Microsoft IME のデフォルト挙動）が起こり、
    /// 入力モードが意図せず切り替わる。
    ///
    /// したがって composing 中は何も送出しない（suppress）。これにより親指の単独打鍵は
    /// composing 中は完全に無視され、IME に対して透明になる。Engine が無効な場合は
    /// hook 層で bypass されてここには来ないので、Windows 全般での 無変換 / 変換 キー
    /// 機能は composing していない場面では引き続き使える。
    ///
    /// **Space の例外**: `space_thumb_vk` に一致し `text_key_space.ignore_composing_guard`
    /// （`TextKeyConfig`、ADR-092 決定B）が true の場合、composing 中でも常に生 VK_SPACE
    /// を送出する。MS-IME/Google 日本語入力とも Space による「変換候補送り」は正規機能
    /// であり、無変換/変換と同じ理由（かな/カタカナ切替・再変換の誤発火防止）で
    /// composing 中に抑制すると、通常の変換操作そのものが壊れるため。
    ///
    /// **無変換/変換**: `muhenkan_vk`/`henkan_vk` に一致した場合、`mode_key_muhenkan`/
    /// `mode_key_henkan`（`ModeKeyConfig`、ADR-092 決定B）が idle/composing それぞれの
    /// 行動（`Suppress`/`Passthrough`）を総関数として決める。既定値は idle/composing
    /// とも `Suppress`（従来通り常時抑制）で、ユーザーが明示的に緩めた場合のみ
    /// かな/カタカナ切替等の副作用リスクを引き受けて素通しさせる。専用Fnキー
    /// （`muhenkan_solo_tap_dedicated_fn_key`）が設定されている場合は
    /// `ModeKeyConfig` より優先される（上記コード参照）。
    ///
    /// **Enter の例外**: `enter_thumb_vk` に一致し `text_key_enter.ignore_composing_guard`
    /// が true の場合も、Space と同じ理由（IME 変換候補確定は正規機能であり、
    /// 無変換/変換と同じガードを適用すると通常の変換確定操作が丸ごと壊れる）で
    /// composing 中でも常に生 VK_RETURN を送出する。既定値は `true`。
    ///
    /// タイムアウト経路（`timeout_pending_thumb`）とフラッシュ経路（`flush_pending`、
    /// `decide_pending_thumb` の Passthrough 割り込み、`step_pending_thumb_char`/
    /// `step_pending_thumb_thumb`）の双方から共通で呼ぶ。以前はフラッシュ経路が
    /// `composing`/VK 種別を一切見ずに常時 suppress していたため、フォーカス変更や
    /// 別キー割り込みで Space が消えることがあった（この不整合を解消するために
    /// `composing`/`modifier_key` を明示的に受け取る形にした）。
    ///
    /// 戻り値の第2要素は、無変換/変換単独タップが IME open 軸への肩代わり
    /// （ADR-092 決定D Step4b、`DelegateToOpenAxis`）に該当した場合の
    /// `ShadowImeAction`。呼び出し元（`&mut self` のメソッド）はこれを
    /// `self.ime_open_requested` へセットすること（このメソッド自体は `&self`
    /// のため直接セットできない）。
    fn resolve_pending_thumb_as_single(
        &self,
        scan_code: ScanCode,
        vk_code: VkCode,
        modifier_key: Option<crate::types::ModifierKey>,
        injected: bool,
        composing: bool,
    ) -> (ResolvedAction, Option<crate::types::ShadowImeAction>) {
        // 親指キーが OS 修飾キー（Ctrl/Shift/Alt/Meta）に割り当てられている場合は
        // composing に関わらず常に suppress する（Alt 単独送出の副作用回避）。
        if modifier_key.is_some() {
            return (
                ResolvedAction {
                    actions: SmallVec::new(),
                    output: OutputUpdate::None,
                },
                None,
            );
        }

        // 無変換/変換の優先順位（ADR-092 決定B/決定D Step4b）:
        // 1. 専用Fnキー（`muhenkan_solo_tap_dedicated_fn_key`、ADR-091 §D3.2）
        // 2. IME open 軸への肩代わり（`*_delegate_to_open_axis`、決定D Step4b）
        // 3. `ModeKeyConfig` ベースの Suppress/Passthrough
        // 1・2 はいずれも `ModeKeyConfig` の外側で独立に判定する——config reload で
        // `ModeKeyConfig` が丸ごと再設定されても、自動検出由来のこれらの値が
        // 消去されないようにするため。GJI の config1.db にこの Fn キーを
        // Composition/Conversion 時の `SwitchKanaType` としてバインドしておく
        // ことで、GJI が自身の内部状態を見てかな形状をトグルする
        // （awase 側は belief を持たない）。
        let special = self.thumb_solo_special_handling(vk_code);

        if let Some(fn_key) = special.dedicated_fn_key {
            let action = KeyAction::Key(fn_key);
            let output = OutputUpdate::record(scan_code, &action, None);
            return (
                ResolvedAction {
                    actions: smallvec![action],
                    output,
                },
                None,
            );
        }
        if let Some(open_axis_action) = special.delegate_to_open_axis.filter(|_| {
            // Hiragana/Katakana は MS-IME/CTF から注入されうるため、注入された
            // 偽の単独タップでは delegate を発火させない。ここでは suppress せず
            // 既定分岐へ落とし、キー自体は従来どおり OS へ届く余地を残す。
            !(special.injected_guarded_delegate && injected)
        }) {
            // composing 中は fail-closed に倒す。誤って true でも suppress に落ちるだけだが、
            // 誤って false で TurnOff/Toggle(→OFF) すると composition を復旧不能に破棄する。
            if !composing {
                return (
                    ResolvedAction {
                        actions: SmallVec::new(),
                        output: OutputUpdate::None,
                    },
                    Some(open_axis_action),
                );
            }
            // fallthrough: ModeKeyConfig.composing（既定 Suppress）へ委ねる。
        }
        if let Some(mode_key_config) = special.mode_key_config {
            let action = SoloTapAction::from(mode_key_config.for_composing(composing));
            let resolved = match action {
                SoloTapAction::Suppress => ResolvedAction {
                    actions: SmallVec::new(),
                    output: OutputUpdate::None,
                },
                SoloTapAction::Passthrough => {
                    let action = KeyAction::Key(vk_code);
                    let output = OutputUpdate::record(scan_code, &action, None);
                    ResolvedAction {
                        actions: smallvec![action],
                        output,
                    }
                }
                // dedicated_fn_key は上で既に処理済みのため、ここには来ない
                // （`SoloTapAction::from(GuardAction)` は Suppress/Passthrough
                // のいずれかしか生成しない）。
                SoloTapAction::DedicatedFnKey(fn_key) => {
                    let action = KeyAction::Key(fn_key);
                    let output = OutputUpdate::record(scan_code, &action, None);
                    ResolvedAction {
                        actions: smallvec![action],
                        output,
                    }
                }
            };
            return (resolved, None);
        }

        // Space/Enter（TextKeyConfig、正規機能キー）。無変換/変換の ModeKeyConfig
        // とは別の総関数——composing 中も既定で素通しする点が異なる。
        let is_space_with_fallback =
            self.space_thumb_vk == Some(vk_code) && self.text_key_space.ignore_composing_guard;
        let is_enter_with_fallback =
            self.enter_thumb_vk == Some(vk_code) && self.text_key_enter.ignore_composing_guard;
        let ignore_composing_guard = is_space_with_fallback || is_enter_with_fallback;
        if composing && !ignore_composing_guard {
            return (
                ResolvedAction {
                    actions: SmallVec::new(),
                    output: OutputUpdate::None,
                },
                None,
            );
        }

        let action = KeyAction::Key(vk_code);
        let output = OutputUpdate::record(scan_code, &action, None);
        (
            ResolvedAction {
                actions: smallvec![action],
                output,
            },
            None,
        )
    }

    /// 3 鍵仲裁で char1+thumb を優先するかを判定する（純粋関数）。
    ///
    /// `TimingJudge::three_key_pairing`（d1/d2 タイミング比較 + bigram/trigram
    /// n-gram タイブレーク）で判定する。char1 の release 有無は見ない。
    ///
    /// **旧実装との違い（issue #140 / BUG-105）**: 以前はここに
    /// `if char1_released_at.is_some() { return false; }` という早期return が
    /// あり、char1 が既に離されていれば `three_key_pairing` を一切呼ばず無条件で
    /// char2 側を優先していた（PR #85, `b1e7474e`。「char2 到着が char1 を諦める
    /// 直接証拠になる」という理由付けで、`char_thumb_chord_confirmed`（2鍵ケース）
    /// が重なりマージン＋n-gramタイブレークで復帰の余地を残すのとは意図的に
    /// 非対称にしていた）。
    ///
    /// この早期returnは、report `01M1GDQVBET5DBX3MY4BRGQFW1`（2026-09-02、
    /// 「しょうにん」と入力したかったが「しいゔにん」になった）で誤りと判明し
    /// 撤回した。実測: d1(char1→thumb)=11.7ms、d2(thumb→char2)=100.8ms、
    /// overlap(char1とthumbの物理的重なり)=94.85ms。d1 が極めて短く同時打鍵の
    /// 意図が明白であるにも関わらず、char1 がchar2到着前に離されていたという
    /// だけで無条件に char2 側（'ゔ'）が採用され「い」+「ゔ」という誤出力に
    /// なっていた。app.log の `[engine-input] vk=0x41 KeyDown ...
    /// state=PendingCharThumb(...,released_at=Some(...))` → `send_keys:
    /// actions=[Char('い'), Char('ゔ')]` という実ログが、この early return 経由
    /// の3鍵パス（`on_input`、タイムアウト経由ではない）が実際に踏まれたことを
    /// 直接裏付けている。
    ///
    /// なお、PR #85 のこの判断が実運用で検証されたことは一度も無かった
    /// （`docs/adr/112-keyup-lifecycle-fsm-delivery.md` BUG-101/決定2 land
    /// （2026-08-31）以前は `char1_released_at` が KeyUp配送のリグレッションで
    /// 恒久的に `None` だったため、この早期return自体が実運用で到達不能
    /// だった）。したがって今回の変更は「実データで検証済みだった設計判断を
    /// 覆す」のではなく「一度も検証されないまま眠っていた分岐を、実データで
    /// 初めて検証した」結果である。
    ///
    /// **`char_thumb_chord_confirmed`（2鍵ケース）とはもともと非対称ではない**:
    /// 本番既定 `min_overlap_margin_percent = 0`（ADR-112決定1、決定3で恒久固定
    /// 確定済み）の下では、2鍵ケースの `overlap_only_verdict` も
    /// `char1_released_at` の値に関わらず常に `Some(true)` を即returnし
    /// n-gramタイブレークには到達しない。つまり修正前の時点で
    /// `char1_released_at` がペアリングの結論を左右する箇所はこの早期return
    /// 1箇所だけであり、削除はむしろ KeyUp/タイムアウト/flush の他3経路との
    /// 挙動整合を回復する変更である。
    ///
    /// **棄却した代案（overlap ベースの早期return）**: 「char1 との物理的重なり
    /// （`thumb.timestamp`〜`char1_released_at`）が不足するときだけ char2 を
    /// 優先する」という代案（2鍵ケースの `overlap_only_verdict` 相当を3鍵にも
    /// 適用）も検討したが、`min_overlap_margin_percent = 0` が恒久固定のため
    /// 現状は「常に重なり十分」に退化し blanket 削除と完全に同一の挙動になる
    /// （no-op）。有効化には3鍵専用の重なりマージンの実測データが要り、
    /// `.claude/rules/tuning-constants.md` の実測義務に抵触する。
    ///
    /// **既知の限界（残る誤判定クラス）**: `three_key_pairing` は
    /// `char1_ts`/`thumb_ts`/`char2_ts` という3つの keydown タイムスタンプのみを
    /// 見る純粋関数で、release時刻の概念を構造的に持たない。したがって
    /// 「char1 との重なりは乏しいが d1(down-to-down) はたまたま短い」ケース
    /// （例: char1↓0ms→thumb↓12ms→char1↑20ms（重なり8ms）→char2↓62ms）を
    /// 原理的に区別できず、誤って char1+thumb を優先しうる。上記の理由により
    /// 今回はこれを許容する。将来この限界が実機で顕在化した場合、安易に
    /// 「char1解放済みなら char2 優先」を再導入しないこと——それは今回撤回した
    /// 早期returnの再発であり、`docs/experiments.md` エントリ01
    /// （IME OFFキー選択が5日で6回反転した事例）と同じ轍を踏む。
    fn compute_prefer_char1(
        &mut self,
        pending: &PendingKey,
        thumb: &PendingThumbData,
        ev: &ClassifiedEvent,
    ) -> (bool, DecisionPhase) {
        self.retro_eval_stats.three_key_total += 1;
        let thumb_face = self.resolve_thumb_face(thumb.side(), pending.pos);
        let judge = self.timing_judge();
        let char1_thumb_kana = thumb_face.and_then(|face| self.lookup_kana_at(pending.pos, face));
        let char1_single_kana = self.lookup_kana_at(pending.pos, Face::Normal);
        let char2_thumb_kana = self
            .resolve_thumb_face(thumb.side(), ev.pos)
            .and_then(|face| self.lookup_kana_at(ev.pos, face));
        let (result, trace) = judge.three_key_pairing_traced(
            pending.timestamp,
            thumb.timestamp,
            ev.timestamp,
            char1_thumb_kana,
            char1_single_kana,
            char2_thumb_kana,
        );
        match trace.phase {
            DecisionPhase::NoNgram => self.retro_eval_stats.no_ngram_count += 1,
            DecisionPhase::Phase1 => {
                self.retro_eval_stats.phase1_reached += 1;
                self.mark_decision(DecisionKind::Phase1, ev.timestamp);
            }
            DecisionPhase::Phase2 => {
                self.retro_eval_stats.phase2_reached += 1;
                record_score_bucket(&mut self.retro_eval_stats, true, trace.score_a);
                record_score_bucket(&mut self.retro_eval_stats, false, trace.score_b);
                let char2_single_kana = self.lookup_kana_at(ev.pos, Face::Normal);
                if char2_single_kana.is_some_and(retro_eval_stats::is_hiragana) {
                    self.retro_eval_stats.char2_normal_hiragana_count += 1;
                }
                self.mark_decision(DecisionKind::Phase2, ev.timestamp);
            }
        }
        (result == timing::ThreeKeyResult::PairWithChar1, trace.phase)
    }

    /// char1側の解決結果を確定させる: history へ即座に反映し、char1 が既に
    /// 物理的に離されている場合は失われた KeyUp を補う。
    ///
    /// **なぜ必要か（issue #140 / BUG-105）**: char1 の物理 KeyUp は
    /// `handle_key_up_pending_char_thumb` で既に Consume 済みで OS に届かない
    /// （`char1_released_at` をSomeにするためだけの内部フラグ立て）。char1側の
    /// 出力が `KeyAction::Key(vk)`（レイアウトの一部の面にしか定義が無い
    /// キー、`is_layout_key`参照）の場合、対応する `KeyUp` を明示的に積まないと
    /// OS 側で押しっぱなし扱い（stuck key）になる。`flush_pending`・
    /// `handle_key_up_pending_char_thumb`・`timeout_pending_char_thumb` の
    /// 3経路は元々この補完を行っていたが、3鍵仲裁の全分岐（本関数の
    /// 呼び出し元）には無かった穴で、早期return削除により到達頻度が
    /// 「稀」から「高速打鍵の常態」へ上がるため揃えて塞ぐ。
    ///
    /// **呼び出し側で `update_history` を重複させないこと**: この関数は
    /// `resolved.output` を即座に `update_history` へ渡して確定させる
    /// （`ParseAction::Reduce`/`ReduceAndContinue` の `record` フィールド経由で
    /// `ShiftReduceParser::parse` が `decide()` の戻り値から `on_reduce` 越しに
    /// 遅延適用する通常の経路は使わない——`append_key_up_for` が依存する
    /// `output_history.remove_by_scan` は、対象エントリが `update_history` 済み
    /// でなければ何も見つけられないため、`record` 経由の遅延適用のままでは
    /// この関数内で KeyUp を積めない）。呼び出し元は戻り値の `record` に
    /// 必ず `OutputUpdate::None` を使うこと。
    fn commit_char1_output(
        &mut self,
        resolved: ResolvedAction,
        char1_scan: ScanCode,
        char1_released_at: Option<Timestamp>,
        now: Timestamp,
    ) -> SmallVec<[KeyAction; 2]> {
        self.update_history(resolved.output, now);
        let mut actions = resolved.actions;
        if char1_released_at.is_some() {
            self.append_key_up_for(&mut actions, char1_scan);
        }
        actions
    }

    /// char1+thumb を同時打鍵として確定し、`remaining` を再処理する `ReduceAndContinue` を返す。
    fn reduce_char_thumb_and_continue(
        &mut self,
        pending: PendingKey,
        thumb_face: Option<Face>,
        remaining: ClassifiedEvent,
        char1_released_at: Option<Timestamp>,
    ) -> ParseAction {
        let resolved = match thumb_face {
            Some(face) => self.resolve_char_thumb_as_simultaneous(&pending, face),
            None => self.resolve_pending_char_as_single(&pending),
        };
        let actions = self.commit_char1_output(
            resolved,
            pending.scan_code,
            char1_released_at,
            remaining.timestamp,
        );
        ParseAction::ReduceAndContinue {
            actions,
            record: OutputUpdate::None,
            remaining,
        }
    }

    /// `PendingCharThumb` 状態で新しいキーが到着した場合の 3 鍵仲裁処理
    ///
    /// char1 → thumb → char2 の並びで、親指キーを char1 と char2 のどちらに
    /// ペアリングするかを決定する。判定基準:
    ///
    /// 1. タイミング: d1 (char1→thumb) vs d2 (thumb→char2)
    /// 2. n-gram スコア: char1+thumb の出力候補 vs char2+thumb の出力候補
    ///
    /// タイミング差が小さいとき（どちらとも取れる場合）は n-gram スコアで
    /// より自然な日本語になるほうを選ぶ。
    fn step_pending_char_thumb_3key(&mut self, ev: &ClassifiedEvent) -> ParseAction {
        let (pending, thumb, char1_released_at) = self.state.expect_pending_char_thumb();
        let thumb_face = self.resolve_thumb_face(thumb.side(), pending.pos);
        self.go_idle();

        // 新しい親指キーが来た → char1+thumb を同時打鍵として確定し、新しい親指を再処理
        if ev.key_class.is_thumb() {
            return self.reduce_char_thumb_and_continue(
                pending,
                thumb_face,
                *ev,
                char1_released_at,
            );
        }

        // char2 が来た → 3 鍵仲裁
        let (prefer_char1, decision_phase) = self.compute_prefer_char1(&pending, &thumb, ev);

        // ADR-120 決定0a 項目7 (should-fix所見S1/S5対応): char2+thumb面が
        // 実際に定義されているかをここで先読みし、このターンで実際に何回
        // update_history が呼ばれるか（＝このターンの3鍵仲裁自身の出力を
        // 何回スキップすればBaseline計上から除外できるか）を確定してから
        // 状態を仕込む。以前は PairWithChar2 なら無条件に2回としていたが、
        // 親指面に char2 の定義が無いフォールバック分岐では実際には
        // char1単独の1回しか出力されず、スキップ数が1つ余って次の実打鍵の
        // 出力を誤って飲み込んでいた（所見S1）。
        //
        // 決定フェーズを問わず（Phase1/Phase2/NoNgramいずれでも、所見S5対応）
        // 「このターンの3鍵仲裁自身の出力」をBaseline計上から除外する。
        // 項目4の経過ms計測（`measure_since`）は Phase2 のときだけ有効にする。
        let char2_thumb_face = self.resolve_thumb_face(thumb.side(), ev.pos);
        let char2_face_lookup =
            char2_thumb_face.and_then(|face| self.lookup_face(ev.pos, self.get_face(face)));
        let own_output_count = if prefer_char1 || char2_face_lookup.is_none() {
            1
        } else {
            2
        };
        // nice-to-have所見N1対応: 前の決定の「後続1かな確定」計測が完了しない
        // まま、この新しい決定に上書きされて失われる場合を数える
        // （remaining==0まで消費済み=自前出力は終わっていた、かつ
        // measure_since=Some=Phase2で計測継続中だった場合のみ「失われた」）。
        if self
            .own_decision_output
            .is_some_and(|s| s.remaining == 0 && s.measure_since.is_some())
        {
            self.retro_eval_stats.followup_overwritten_count += 1;
        }
        self.own_decision_output = Some(OwnDecisionOutput {
            remaining: own_output_count,
            measure_since: (decision_phase == DecisionPhase::Phase2).then_some(ev.timestamp),
        });
        if decision_phase == DecisionPhase::Phase2 {
            // nice-to-have所見N2対応: 前の窓が未消化のまま上書きされる場合を数える
            // （項目2cの正しい分母 = no_thumb + thumb_arrived + abandoned）。
            if self.thumb_watch_window.is_some() {
                self.retro_eval_stats.thumb_watch_window_abandoned_count += 1;
            }
            self.thumb_watch_window = Some(ThumbWatchWindow {
                remaining: 2,
                last_vk: None,
                last_vk_down: false,
            });
        }

        if prefer_char1 {
            // char1+thumb = 同時打鍵、char2 は再処理
            return self.reduce_char_thumb_and_continue(
                pending,
                thumb_face,
                *ev,
                char1_released_at,
            );
        }

        // char1 = 単独、char2+thumb = 同時打鍵（または char2 単独）
        let char1_resolved = self.resolve_pending_char_as_single(&pending);
        let mut actions = self.commit_char1_output(
            char1_resolved,
            pending.scan_code,
            char1_released_at,
            ev.timestamp,
        );
        if let (Some(face), Some((action, kana))) = (char2_thumb_face, char2_face_lookup) {
            self.consume_thumb(face);
            actions.push(action.clone());
            return ParseAction::Reduce {
                actions,
                record: OutputUpdate::record(ev.scan_code, &action, kana),
                timer: TimerIntent::CancelAll,
            };
        }
        // 親指面に char2 の定義がない → char1 単独確定、char2 を再処理
        ParseAction::ReduceAndContinue {
            actions,
            record: OutputUpdate::None,
            remaining: *ev,
        }
    }
}

// ── KeyUp 処理 ──
impl NicolaFsm {
    fn on_key_up(&mut self, event: &RawKeyEvent) -> Resp {
        // phys.classified は on_key_down 側で使用済み

        // ADR-120 決定0a 項目7(a) (should-fix所見S4対応): 物理BACKSPACEが
        // 離されたら押下状態をクリアする（`handle_bypass` のオートリピート
        // 抑止ガードの対称KeyUp側）。
        if self.backspace_vk.is_some_and(|vk| event.vk_code == vk) {
            self.backspace_down = false;
        }

        // ADR-120 決定0a 項目2c (`/code-review` 指摘対応): 観測窓が
        // 追跡中のキーが離されたら `last_vk_down` をクリアする
        // （`observe_thumb_watch_window` のオートリピート判定の対称KeyUp
        // 側。同じ物理キーを間を置かず2回連続で本当にタップした場合を、
        // 誤ってオートリピートとみなさないようにするため）。
        if let Some(window) = self.thumb_watch_window.as_mut() {
            if window.last_vk == Some(event.vk_code) {
                window.last_vk_down = false;
            }
        }

        // PendingCharThumb 状態での KeyUp 処理
        if let EngineState::PendingCharThumb {
            char_key, thumb, ..
        } = self.state
        {
            if event.vk_code == char_key.vk_code || event.vk_code == thumb.vk_code {
                return self.handle_key_up_pending_char_thumb(event);
            }
        }

        // SpeculativeChar 状態で投機出力キーが離された場合 → 出力確定（Idle へ遷移）
        if let EngineState::SpeculativeChar(pending) = self.state {
            if event.vk_code == pending.vk_code {
                self.go_idle();
                // output_history から対応するキーの KeyUp を処理
                return self.release_only(event);
            }
        }

        // 保留中のキーが離された場合、保留を単独確定
        if self.is_pending_key(event.vk_code) {
            return self.handle_key_up_pending(event);
        }

        // `engine_off_solo_repeat` を親指キー以外に割り当てた場合の対称化:
        // `handle_bypass` が KeyDown 側で suppress/passthrough のどちらと
        // 判定したかをそのまま KeyUp にも適用する（J↓/J↑ 非対称防止、下の
        // OsModifierHeld 対称化と同じ理由）。現在の modifier 状態には依存
        // しない（KeyDown 時点の判定を優先する）。
        if self.engine_off_solo_repeat_vk.0 != 0 && event.vk_code == self.engine_off_solo_repeat_vk
        {
            if let Some(suppressed) = self.engine_off_extra_key_suppressed.take() {
                return if suppressed {
                    self.build_response(SmallVec::new(), true, TimerIntent::CancelAll)
                } else {
                    Response::pass_through()
                };
            }
        }

        // OS modifier (Ctrl/Alt/Win) 保持中: on_key_down と対称にバイパス。
        //
        // 旧実装は「output_history の中身に反応しない（誤 Suppress 防止）」
        // ためこの分岐で無条件 pass_through していたが、掃除もしないため
        // pending_releases のエントリが永久に残り stuck key が再発する上に
        // （/code-review 指摘）、Key(vk) 型エントリの KeyUp(vk) も送られず
        // OS 側で押されっぱなしになる、Char/Romaji 型は誤って生 KeyUp を
        // OS へ漏らす、という3つの問題を抱えていた。
        //
        // 「誤 Suppress 防止」が守っていたのは、当時 output_history が単一の
        // 無制限 Vec で KeyUp 整合性用途と n-gram 文脈用途を兼ねており、
        // 別キーの残骸に誤って反応しうる状況だった（ADR-112決定0参照）。
        // 決定0で pending_releases を分離した現在、この分岐に来る時点の
        // scan_code は「この物理キー自身が直前に Consume されて記録した
        // エントリ」以外にはあり得ない（bypass 側の KeyDown は
        // handle_bypass が自分の scan_code のエントリを先に掃除するため）。
        // よって OS modifier 保持の有無を問わず、通常の release_only と
        // 全く同じロジックで安全に解放できる——`state` を経由しない chord
        // 判定なしの掃除、という release_only の性質はここでも保たれている
        // （OS modifier 有無で分岐する必要が無くなったため、if 文は撤去した。
        // /code-review 指摘: 分岐両辺が同一の release_only(event) を返す
        // だけの死んだ条件分岐になっていた）。
        self.release_only(event)
    }

    /// 保留中キーの vk_code と一致するか判定する
    fn is_pending_key(&self, vk_code: VkCode) -> bool {
        match self.state {
            EngineState::PendingChar(pending) => pending.vk_code == vk_code,
            EngineState::PendingThumb(thumb) => thumb.vk_code == vk_code,
            EngineState::Idle
            | EngineState::PendingCharThumb { .. }
            | EngineState::SpeculativeChar(_) => false,
        }
    }

    /// char1+thumb を同時打鍵として確定してよいかを判定する（重なり + n-gram タイブレーク）。
    ///
    /// `thumb_face` は呼び出し元が既に解決済みのものを渡す（chord 確定時にも必要な
    /// ため、ここで二重に解決しない）。重なりだけで確定できる場合（大半のケース）は
    /// `TimingJudge` の構築（`recent_kana` の `Vec` 確保）やかな引きを行わない
    /// （`timing::overlap_only_verdict` 参照。keystroke-rate のホットパスでの
    /// 無駄な確保・配列面引きを避けるため）。
    fn char_thumb_chord_confirmed(
        &self,
        pending: &PendingKey,
        thumb: &PendingThumbData,
        thumb_face: Option<Face>,
        char1_released_at: Option<Timestamp>,
    ) -> bool {
        if let Some(verdict) = timing::overlap_only_verdict(
            self.threshold_us,
            thumb.timestamp,
            char1_released_at,
            self.min_overlap_margin_percent,
        ) {
            return verdict;
        }
        let chord_kana = thumb_face.and_then(|face| self.lookup_kana_at(pending.pos, face));
        let solo_kana = self.lookup_kana_at(pending.pos, Face::Normal);
        self.timing_judge().confirms_char_thumb_chord(
            thumb.timestamp,
            char1_released_at,
            chord_kana,
            solo_kana,
        )
    }

    /// PendingCharThumb 状態で char1 または thumb が離された場合の処理
    fn handle_key_up_pending_char_thumb(&mut self, event: &RawKeyEvent) -> Resp {
        let (pending, thumb, char1_released_at) = self.state.expect_pending_char_thumb();

        // char1 の最初の KeyUp → フラグを立てて待機継続。
        // 後から char2 が来れば「char1 単独 + char2+thumb 同時」と確実に判定できる。
        if event.vk_code == pending.vk_code && char1_released_at.is_none() {
            self.state = EngineState::PendingCharThumb {
                char_key: pending,
                thumb,
                char1_released_at: Some(event.timestamp),
            };
            return self.build_response(SmallVec::new(), true, TimerIntent::Keep);
        }

        self.go_idle();
        let thumb_face = self.resolve_thumb_face(thumb.side(), pending.pos);
        let confirmed_chord =
            self.char_thumb_chord_confirmed(&pending, &thumb, thumb_face, char1_released_at);

        if !confirmed_chord {
            // 重なり不足 → 同時打鍵ではなく char1・thumb をそれぞれ単独打鍵として確定する
            return self.resolve_char_and_thumb_as_separate_solos(
                &pending,
                &thumb,
                event.vk_code == thumb.vk_code,
                event.timestamp,
                true,
            );
        }

        // char1+thumb を同時打鍵として確定する
        let resolved = match thumb_face {
            Some(face) => self.resolve_char_thumb_as_simultaneous(&pending, face),
            None => self.resolve_pending_char_as_single(&pending),
        };
        self.update_history(resolved.output, event.timestamp);
        let mut actions = resolved.actions;

        // どの物理キーが離されたかに応じて char1 の KeyUp 追記を判定
        let key_up_scan = if event.vk_code == pending.vk_code {
            // char1 が再度離された (char1_released_at=Some 済み)
            Some(event.scan_code)
        } else if char1_released_at.is_some() {
            // thumb が離された + char1 は既に物理的に離されている
            Some(pending.scan_code)
        } else {
            // thumb が離された + char1 はまだ押下中 → KeyUp 不要
            None
        };
        if let Some(scan) = key_up_scan {
            self.append_key_up_for(&mut actions, scan);
        }
        self.build_response(actions, true, TimerIntent::CancelAll)
    }

    /// 重なり不足で同時打鍵を確定しなかった場合、char1・thumb をそれぞれ単独打鍵として
    /// 確定する（`TimingJudge::confirms_char_thumb_chord` が false を返した場合専用）。
    ///
    /// char1 はこのパスに来る時点で必ず既に物理的に離されているため、その KeyUp は
    /// 常に追記する。thumb 自身は `thumb_released_now` が true（呼び出し元が thumb
    /// 自身の KeyUp イベントを処理中）の場合のみ即座に KeyUp を追記する——タイムアウト
    /// 経由（thumb はまだ押下中）ならまだ実 KeyUp が来ていないため、後から届く実際の
    /// KeyUp イベントに解決を委ねる。
    ///
    /// タイムアウト経由（`thumb_released_now=false`）では thumb はまだ物理的に
    /// 押されたままなので、`left_thumb_consumed`/`right_thumb_consumed` を明示的に
    /// 更新して「消費済み」にする。これを怠ると、この関数が既に単独打鍵として
    /// 確定した直後に次のキーが到着した際、`active_thumb_side()` が同じ物理押下を
    /// 未消費の親指とみなし、既に単独打鍵として出力済みの thumb を次のキーとの
    /// 同時打鍵の相方として二重に使ってしまう。`consume_thumb()` は「同時打鍵に
    /// 使われた」前提で `solo_counter` をリセットする副作用を持つため、ここでは
    /// 使わない（thumb は同時打鍵ではなく単独打鍵として確定するため）。
    fn resolve_char_and_thumb_as_separate_solos(
        &mut self,
        char_key: &PendingKey,
        thumb: &PendingThumbData,
        thumb_released_now: bool,
        now: Timestamp,
        precise: bool,
    ) -> Resp {
        let char1_resolved = self.resolve_pending_char_as_single(char_key);
        if precise {
            self.update_history(char1_resolved.output, now);
        } else {
            self.update_history_imprecise(char1_resolved.output, now);
        }
        let mut actions = char1_resolved.actions;
        self.append_key_up_for(&mut actions, char_key.scan_code);

        match thumb.side() {
            ThumbSide::Left => self.left_thumb_consumed = self.phys.left_thumb_down,
            ThumbSide::Right => self.right_thumb_consumed = self.phys.right_thumb_down,
        }

        // ソロ連打によるエンジン OFF トリガーチェック（timeout_pending_thumb と同一
        // ロジック）。thumb はここで同時打鍵ではなく単独打鍵として確定するため、
        // ソロ連打カウンターの対象になる。
        if self.engine_off_solo_repeat_vk.0 != 0 && thumb.vk_code == self.engine_off_solo_repeat_vk
        {
            let count = self.solo_counter.record(thumb.vk_code, thumb.timestamp);
            if count >= SOLO_OFF_TRIGGER_COUNT {
                self.solo_counter.reset();
                self.engine_off_requested = true;
                // N 回目は thumb 側の送出のみ suppress する（char1 の出力は維持）
                return self.build_response(actions, true, TimerIntent::CancelAll);
            }
        } else {
            self.solo_counter.reset();
        }

        let (thumb_resolved, ime_open_request) = self.resolve_pending_thumb_as_single(
            thumb.scan_code,
            thumb.vk_code,
            thumb.modifier_key,
            thumb.injected,
            self.phys.composing,
        );
        if ime_open_request.is_some() {
            self.ime_open_requested = ime_open_request;
        }
        if precise {
            self.update_history(thumb_resolved.output, now);
        } else {
            self.update_history_imprecise(thumb_resolved.output, now);
        }
        actions.extend(thumb_resolved.actions);
        if thumb_released_now {
            self.append_key_up_for(&mut actions, thumb.scan_code);
        }
        self.build_response(actions, true, TimerIntent::CancelAll)
    }

    /// 保留中のキーが離された場合、保留を単独確定して KeyUp を処理する
    fn handle_key_up_pending(&mut self, event: &RawKeyEvent) -> Resp {
        let old_state = std::mem::replace(&mut self.state, EngineState::Idle);

        let (resolved, ime_open_request) = match old_state {
            EngineState::PendingChar(pending) => {
                (self.resolve_pending_char_as_single(&pending), None)
            }
            EngineState::PendingThumb(thumb) => self.resolve_pending_thumb_as_single(
                thumb.scan_code,
                thumb.vk_code,
                thumb.modifier_key,
                thumb.injected,
                self.phys.composing,
            ),
            EngineState::Idle
            | EngineState::PendingCharThumb { .. }
            | EngineState::SpeculativeChar(_) => {
                log::error!(
                    "unexpected state in handle_key_up_pending: {:?}",
                    self.state
                );
                (
                    ResolvedAction {
                        actions: SmallVec::new(),
                        output: OutputUpdate::None,
                    },
                    None,
                )
            }
        };
        if ime_open_request.is_some() {
            self.ime_open_requested = ime_open_request;
        }
        self.update_history(resolved.output, event.timestamp);
        let mut result = resolved.actions;
        self.append_key_up_for(&mut result, event.scan_code);
        // Unicode 文字 (Char) は Down+Up 一括送信済みなので KeyUp 追加不要
        self.build_response(result, true, TimerIntent::CancelAll)
    }

    /// output_history から対応する注入済みキーを探してリリースする。
    ///
    /// `self.state`（chord判定の途中状態）には一切触れない純粋な後始末——
    /// `release_only` としてエンジン非活性時（`Engine::on_input` Phase 2、
    /// ADR-112決定2）からも直接呼ばれる。「コンテキストを失ったら同時打鍵
    /// 判定を再開しない」という方針上、非活性時は`state`を経由する通常の
    /// `on_key_up`ディスパッチを一切通さず、この関数だけを呼ぶ。
    pub(crate) fn release_only(&mut self, event: &RawKeyEvent) -> Resp {
        if let Some(action) = self.output_history.remove_by_scan(event.scan_code) {
            return match action {
                // Unicode 文字やローマ字列の場合、KeyUp は不要（押下時に入力完了）
                KeyAction::Char(_) | KeyAction::Romaji(_) => self.build_response(
                    smallvec![KeyAction::Suppress],
                    true,
                    TimerIntent::CancelAll,
                ),
                KeyAction::Key(vk) => self.build_response(
                    smallvec![KeyAction::KeyUp(vk)],
                    true,
                    TimerIntent::CancelAll,
                ),
                // それ以外（SpecialKey/KeySequence/Suppress/KeyUp、および
                // ADR-115 で追加した Sequence/CtrlChord）は今日と同じく
                // pass_through——`Char`/`Romaji`/`Key` 以外はいずれも
                // 「解放すべき片割れを持たない」という点で既存の
                // `SpecialKey`/`KeySequence`/`Suppress` と同じ扱いが
                // 一貫している（`Sequence` は許可リストで生 `Key` を
                // 禁止済み、`CtrlChord` は1回の `SendInput` で
                // 自己完結、ともに解放対象が無い）。網羅 match にする
                // ことで、将来 `KeyAction` に variant が増えた際に
                // コンパイラが対応漏れを検出する（意味的な挙動は変え
                // ない——ワイルドカード `_` を明示列挙に置き換えた
                // だけ）。
                KeyAction::SpecialKey(_)
                | KeyAction::KeySequence(_)
                | KeyAction::Suppress
                | KeyAction::Sequence(_)
                | KeyAction::CtrlChord(_)
                | KeyAction::KeyUp(_) => Response::pass_through(),
            };
        }
        Response::pass_through()
    }

    /// コンテキスト喪失（フォーカス変更・非活性化）時に `output_history` の
    /// `pending_releases` を全て強制解放する。`KeyAction::Key(vk)` 型の
    /// エントリに対応する `KeyUp(vk)` アクションを返す（それ以外は黙って除去）。
    ///
    /// `Engine`（`check_active_transition`/`handle_focus_changed`）が
    /// `KeyLifecycle::flush_pending_key_ups` と同期して呼ぶこと（ADR-112
    /// コードレビュー指摘）——`active_keys` だけを drain して `pending_releases`
    /// を放置すると、対応する KeyUp がその後 `UpDuty::None` として素通りする
    /// ようになり、二度と掃除されないまま stuck key が再発する。
    pub(crate) fn release_all_pending_output(&mut self) -> Vec<KeyAction> {
        self.output_history.drain_pending_releases_as_keyups()
    }
}

// ── タイムアウト処理 ──
impl NicolaFsm {
    /// PendingChar タイムアウト：文字キーを単独打鍵として確定する
    fn timeout_pending_char(&mut self, pending: &PendingKey) -> Resp {
        let resolved = self.resolve_pending_char_as_single(pending);
        self.update_history_imprecise(resolved.output, self.last_key_timestamp.unwrap_or(0));
        self.build_response(resolved.actions, true, TimerIntent::CancelAll)
    }

    /// PendingThumb タイムアウト：親指キーを単独打鍵として確定する
    fn timeout_pending_thumb(
        &mut self,
        scan_code: ScanCode,
        vk_code: VkCode,
        timestamp: Timestamp,
        composing: bool,
        modifier_key: Option<crate::types::ModifierKey>,
        injected: bool,
    ) -> Resp {
        // ソロ連打によるエンジン OFF トリガーチェック
        if self.engine_off_solo_repeat_vk.0 != 0 && vk_code == self.engine_off_solo_repeat_vk {
            let count = self.solo_counter.record(vk_code, timestamp);
            if count >= SOLO_OFF_TRIGGER_COUNT {
                self.solo_counter.reset();
                self.engine_off_requested = true;
                // N 回目は suppress（OS への VK 送出を防ぐ）
                return self.build_response(SmallVec::new(), true, TimerIntent::CancelAll);
            }
        } else {
            self.solo_counter.reset();
        }

        // scan_code には物理キーの実スキャンコードを使う。
        // 以前は ScanCode(u32::from(vk_code.0)) という合成値を使っていたが、
        // VK_CONVERT (VK=0x1C) の合成スキャンコードが Enter の物理スキャンコード (0x1C) と
        // 衝突し、後から Enter KeyUp が来たときに誤って KeyUp(VK_CONVERT) が送出されていた。
        //
        // suppress/送出の判定（composing ガード・Space 例外・OS 修飾キーガード）は
        // resolve_pending_thumb_as_single に委譲し、flush 経路と挙動を統一する。
        let (resolved, ime_open_request) = self.resolve_pending_thumb_as_single(
            scan_code,
            vk_code,
            modifier_key,
            injected,
            composing,
        );
        if ime_open_request.is_some() {
            self.ime_open_requested = ime_open_request;
        }
        self.update_history_imprecise(resolved.output, self.last_key_timestamp.unwrap_or(0));
        self.build_response(resolved.actions, true, TimerIntent::CancelAll)
    }

    /// PendingCharThumb タイムアウト：char1+thumb の同時打鍵を確定を試みる。
    /// 重なり不足（`char_thumb_chord_confirmed` が false）なら
    /// `resolve_char_and_thumb_as_separate_solos` に委譲し、代わりに char1・thumb を
    /// それぞれ単独打鍵として確定する。
    fn timeout_pending_char_thumb(
        &mut self,
        char_key: &PendingKey,
        thumb: &PendingThumbData,
        char1_released_at: Option<Timestamp>,
    ) -> Resp {
        let thumb_face = self.resolve_thumb_face(thumb.side(), char_key.pos);
        let confirmed_chord =
            self.char_thumb_chord_confirmed(char_key, thumb, thumb_face, char1_released_at);

        if !confirmed_chord {
            // 重なり不足 → 同時打鍵ではなく char1・thumb をそれぞれ単独打鍵として確定する。
            // thumb はまだ押下中（だからこそタイムアウトした）なので、thumb 自身の
            // KeyUp は後から届く実イベントに委ねる（thumb_released_now=false）。
            return self.resolve_char_and_thumb_as_separate_solos(
                char_key,
                thumb,
                false,
                self.last_key_timestamp.unwrap_or(0),
                false,
            );
        }

        let resolved = match thumb_face {
            Some(face) => self.resolve_char_thumb_as_simultaneous(char_key, face),
            None => self.resolve_pending_char_as_single(char_key),
        };
        self.update_history_imprecise(resolved.output, self.last_key_timestamp.unwrap_or(0));
        let mut actions = resolved.actions;
        if char1_released_at.is_some() {
            // char1 は既に物理的に離されている → Key 出力があれば KeyUp も追加
            if let Some(KeyAction::Key(vk)) = self.output_history.remove_by_scan(char_key.scan_code)
            {
                actions.push(KeyAction::KeyUp(vk));
            }
        }
        self.build_response(actions, true, TimerIntent::CancelAll)
    }

    /// TwoPhase モード: Phase 1 の短い待機がタイムアウトした場合の処理。
    ///
    /// 親指キーが Phase 1 内に到着しなかったので、投機出力（Phase 2）に遷移する。
    /// 通常面の文字を出力し、`SpeculativeChar` 状態に入る。
    /// 残りの閾値時間（`threshold_us - speculative_delay_us`）で `TIMER_PENDING` を設定する。
    ///
    /// Phase 2 に入った後、残り時間内に親指キーが到着すれば
    /// `step_speculative_thumb()` が BACKSPACE で投機出力を取り消す。
    /// `TIMER_PENDING` が満了すれば投機出力は正しかったとみなし、Idle に戻る。
    fn on_timeout_speculative(&mut self) -> Resp {
        match self.state {
            EngineState::PendingChar(pending) => {
                // Output normal face speculatively
                let face = Face::Normal;
                if let Some((action, kana)) = self.lookup_face(pending.pos, self.get_face(face)) {
                    let remaining_us = self.threshold_us.saturating_sub(self.speculative_delay_us);
                    if self.enter_speculative_char(pending, &action) {
                        // Emit the speculative output + set TIMER_PENDING for remaining time
                        self.update_history_imprecise(
                            OutputUpdate::record(pending.scan_code, &action, kana),
                            self.last_key_timestamp.unwrap_or(0),
                        );
                        self.build_response(
                            smallvec![action],
                            true,
                            TimerIntent::Phase2Transition { remaining_us },
                        )
                    } else {
                        // Sequence（決定7）: 投機を諦め、PendingChar を維持
                        // したまま actions 無しで残り時間ぶんタイマーを
                        // 張り直す。`state` は変更していない（`match
                        // self.state` はコピーで読んだだけ）ので
                        // `PendingChar` のまま残る。TIMER_PENDING が
                        // 本来の満了時刻に達すれば既存の on_timeout →
                        // timeout_pending_char が確定を行う——確定ロジック
                        // をここで手書き複製する必要が無い（Wait モードと
                        // 確定経路が構造的に1本に揃う）。満了前に親指
                        // キーが来れば PendingChar のままなので
                        // step_pending_char_thumb が chord 判定を行う
                        // （r4 の「その場で確定出力」= CancelAll が
                        // 招いていた受付窓の縮小を回避する）。
                        self.build_response(
                            SmallVec::new(),
                            false,
                            TimerIntent::Phase2Transition { remaining_us },
                        )
                    }
                } else {
                    self.go_idle();
                    Response::pass_through().with_kill_timer(TIMER_SPECULATIVE)
                }
            }
            // Other states shouldn't have TIMER_SPECULATIVE active
            other => {
                log::warn!("TIMER_SPECULATIVE fired in unexpected state: {other:?}");
                Response::pass_through().with_kill_timer(TIMER_SPECULATIVE)
            }
        }
    }
}

// ── バイパス ──
impl NicolaFsm {
    /// 親指キーが消費済み（同時打鍵に使用済み）かどうかを返す。
    ///
    /// 消費タイムスタンプが現在の物理押下と一致すれば消費済み。
    /// 物理状態が変わると自動的に不一致になるため、明示的なリセットは不要。
    fn is_thumb_consumed(&self, face: Face) -> bool {
        let (phys_down, consumed) = match face.thumb_side() {
            Some(ThumbSide::Left) => (self.phys.left_thumb_down, self.left_thumb_consumed),
            Some(ThumbSide::Right) => (self.phys.right_thumb_down, self.right_thumb_consumed),
            None => return false,
        };
        phys_down.is_some() && consumed == phys_down
    }

    /// 現在押下中かつ未消費の親指キーの側を返す。
    fn active_thumb_side(&self) -> Option<ThumbSide> {
        if self.phys.left_thumb_down.is_some() && !self.is_thumb_consumed(Face::LeftThumb) {
            Some(ThumbSide::Left)
        } else if self.phys.right_thumb_down.is_some() && !self.is_thumb_consumed(Face::RightThumb)
        {
            Some(ThumbSide::Right)
        } else {
            None
        }
    }

    /// いずれかの配列面に非 None の出力定義があるキーかどうか。
    ///
    /// YabValue::None（'無'）は「その面では出力なし」を明示するが配列キーではないため除外する。
    /// これにより、全面が '無' のキーはパススルー扱いとなり、
    /// Shift面など一部に定義がある場合のみ NICOLA 処理対象となる。
    pub(crate) fn is_layout_key(&self, pos: Option<PhysicalPos>) -> bool {
        let Some(pos) = pos else {
            return false;
        };
        let has_output =
            |face: &YabFace| face.get(&pos).is_some_and(|v| !matches!(v, YabValue::None));
        has_output(self.get_face(Face::Normal))
            || has_output(self.get_face(Face::LeftThumb))
            || has_output(self.get_face(Face::RightThumb))
            || has_output(self.get_face(Face::Shift))
            || has_output(self.get_face(Face::LeftThumbShift))
            || has_output(self.get_face(Face::RightThumbShift))
    }

    /// キーイベントがエンジン処理をバイパスすべきかを判定する
    fn bypass_reason(&self, ev: &ClassifiedEvent) -> Option<BypassReason> {
        if ev.key_class == KeyClass::Passthrough {
            return Some(BypassReason::Passthrough);
        }
        if ev.is_ime_control {
            return Some(BypassReason::ImeControl);
        }
        if self.phys.modifiers.is_os_modifier_held() {
            return Some(BypassReason::OsModifierHeld);
        }
        None
    }

    /// バイパス理由に基づいて保留キーをフラッシュしつつパススルーする
    ///
    /// 全てのバイパス理由で同一の処理: 保留があればフラッシュ、元のキーは OS にパススルー。
    /// consumed=false を維持するため ParseAction ループの外で直接 Resp を返す。
    fn handle_bypass(
        &mut self,
        ev: &ClassifiedEvent,
        reason: BypassReason,
        injected: bool,
    ) -> Resp {
        // バイパスされたキーの output_history エントリを削除する。
        // OsModifierHeld で J↓ がバイパスされた後、modifier が J↑ より先にリリースされると
        // on_key_up の is_os_modifier_held() チェックが通らず、output_history に前回の
        // NICOLA 組み合わせのエントリが残っていると J↑ が誤って Suppress される。
        self.output_history.remove_by_scan(ev.scan_code);

        // ADR-120 決定0a 項目7(a): 物理BACKSPACE単独を
        // ユーザー訂正操作として計上する。`Ctrl+BS`/`Alt+BS`（単語削除等の
        // 別操作）は対照群比較を歪めるため除外する（should-fix所見7）。
        // `Shift+BS` は除外しない——Windows のテキスト入力では
        // `Shift+Backspace` は通常の1文字削除と同義であり（`Ctrl`/`Alt`の
        // ような別操作ではない）、除外すると Shift 保持中の訂正だけが
        // 系統的に取りこぼされる（`/code-review` 指摘）。
        //
        // **所見B1対応**: `bypass_reason`（下記参照）は `KeyClass::Passthrough`
        // を修飾キーの有無より優先して返すため、`BypassReason::OsModifierHeld`
        // では絶対に来ない——VK_BACK は `scanmap.rs` に物理位置が無く常に
        // `Passthrough` に分類されるので、`Ctrl+BS`/`Alt+BS` も
        // `BypassReason::Passthrough` のままここに到達する。したがって
        // `reason == Passthrough` だけでは除外できず、この下の `is_word_delete`
        // で明示的に判定する必要がある（Opusレビュー指摘、「`OsModifierHeld`
        // なので除外される」という以前のコメントは誤りだった）。
        //
        // **所見S4対応**: OS オートリピート（押しっぱなし）による KeyDown
        // 再送を新規タップとして二重計上しないよう、`backspace_down` で
        // 押下状態を追跡する（`engine_off_extra_key_suppressed` と同型の
        // ガード、KeyUp 側は `on_key_up` でクリアする）。
        //
        // **`/code-review` 指摘（injected除外）**: 外部ツール・マクロ・IME
        // 自身の内部補正機構等が `SendInput` 等で合成した BACKSPACE は
        // ユーザーの実訂正操作ではないため除外する（BUG-14/ADR-119 の
        // `event.injected` 除外原則、`engine.rs::is_bare_thumb` 参照）。
        if matches!(reason, BypassReason::Passthrough) && !injected {
            if let Some(vk) = self.backspace_vk {
                if ev.vk_code == vk {
                    let is_word_delete = self.phys.modifiers.ctrl
                        || self.phys.modifiers.alt
                        || self.phys.modifiers.win;
                    if !is_word_delete && !self.backspace_down {
                        self.record_user_correction(ev.timestamp);
                    }
                    self.backspace_down = true;
                }
            }
        }

        // ソロ N 連打エンジン OFF（`engine_off_solo_repeat` を親指キー以外の
        // VK に割り当てた場合、例: VK_INSERT）。親指キーに割り当てた場合は
        // `resolve_char_and_thumb_as_separate_solos`/`timeout_pending_thumb` が
        // `solo_counter` で担当するためここには来ない——親指キーは
        // `KeyClass::LeftThumb`/`RightThumb` に分類され `bypass_reason` が
        // `Passthrough` を返さないため、両カウンターが同時に動くことはない。
        let is_extra_trigger_key = matches!(reason, BypassReason::Passthrough)
            && self.engine_off_solo_repeat_vk.0 != 0
            && ev.vk_code == self.engine_off_solo_repeat_vk;
        if is_extra_trigger_key {
            if let Some(already_suppressed) = self.engine_off_extra_key_suppressed {
                // OS のオートリピートによる KeyDown 再送（キーを押しっぱなし）。
                // 新規タップではないためカウントは増やさず、直近の判定
                // （suppress/passthrough）をそのまま維持する——押しっぱなしの
                // 間に勝手にカウントが進んで意図せず5回に達することを防ぐ。
                if already_suppressed {
                    return self.build_response(SmallVec::new(), true, TimerIntent::CancelAll);
                }
                // 素通し判定だったリピートは下の通常パスへ落として素通しする。
            } else {
                // 新規物理押下。Ctrl/Alt/Shift/Win のいずれかが同時に押されて
                // いる場合は「ソロ」タップではない（例: Ctrl+Insert はコピーの
                // OS ショートカット）ため、カウント対象外かつストリークを
                // リセットする。`bypass_reason` は Passthrough を最優先で返す
                // ため、通常の OsModifierHeld 判定はここには効かない
                // （`bypass_reason` 参照）——ここで明示的に見る必要がある。
                let is_solo = !(self.phys.modifiers.ctrl
                    || self.phys.modifiers.alt
                    || self.phys.modifiers.shift
                    || self.phys.modifiers.win);
                let suppressed = if is_solo {
                    let count = self
                        .engine_off_extra_solo_counter
                        .record(ev.vk_code, ev.timestamp);
                    if count >= SOLO_OFF_TRIGGER_COUNT {
                        self.engine_off_extra_solo_counter.reset();
                        self.engine_off_requested = true;
                        true
                    } else {
                        false
                    }
                } else {
                    self.engine_off_extra_solo_counter.reset();
                    false
                };
                // KeyUp 側（`on_key_up`）がこの押下と同じ判定を再現できるよう
                // 記録しておく（J↓/J↑ 非対称防止）。押下が離されたら
                // `on_key_up` 側で `None` に戻す。
                self.engine_off_extra_key_suppressed = Some(suppressed);
                if suppressed {
                    // N 回目はこのキー自体の送出のみ suppress する（1〜N-1回目は
                    // ここに来ず、下の通常パスでそのまま素通しされるため通常の
                    // VK 動作は変わらない）。保留があれば通常どおりフラッシュする。
                    if self.state.is_idle() {
                        return self.build_response(SmallVec::new(), true, TimerIntent::CancelAll);
                    }
                    let flush = self.flush_pending(
                        ContextChange::BypassKey,
                        ComposingHint::Trusted(self.phys.composing),
                    );
                    let mut resp =
                        self.build_response(SmallVec::new(), true, TimerIntent::CancelAll);
                    resp.actions = flush.actions;
                    resp.timers = flush.timers;
                    return resp;
                }
            }
        } else {
            self.engine_off_extra_solo_counter.reset();
        }

        if self.state.is_idle() {
            return Response::pass_through();
        }
        log::debug!(
            "handle_bypass: vk=0x{:02X} reason={:?} state={}",
            ev.vk_code.0,
            reason,
            self.state.debug_label(),
        );
        let flush = self.flush_pending(
            ContextChange::BypassKey,
            ComposingHint::Trusted(self.phys.composing),
        );
        let mut resp = Response::pass_through();
        resp.actions = flush.actions;
        resp.timers = flush.timers;
        resp
    }
}

// ── イベント処理エントリポイント ──
impl NicolaFsm {
    /// キーイベントを処理する。
    ///
    /// `phys` は `InputTracker::process()` が返した物理キー状態スナップショット。
    /// 内部メソッドは `self.phys` フィールド経由でこの状態を参照する。
    pub fn on_event(&mut self, event: RawKeyEvent, phys: &PhysicalKeyState) -> Resp {
        self.phys = *phys;
        // 親指消費フラグのリセットは不要: タイムスタンプ比較で自動判定される

        if !self.enabled {
            return Response::pass_through();
        }

        match event.event_type {
            KeyEventType::KeyDown => self.on_key_down(&event),
            KeyEventType::KeyUp => self.on_key_up(&event),
        }
    }

    /// タイマー満了時の処理。
    ///
    /// `phys` は `InputTracker` の最新スナップショット。
    /// タイマー発火時点の正確な物理キー状態を反映する。
    /// `composing` は IME composition が現在進行中か（`InputContext::composing` 由来）。
    pub fn on_timeout(
        &mut self,
        timer_id: usize,
        phys: &PhysicalKeyState,
        composing: bool,
    ) -> Resp {
        self.phys = *phys;
        match timer_id {
            TIMER_SPECULATIVE => return self.on_timeout_speculative(),
            TIMER_PENDING => {}
            _ => return Response::pass_through(),
        }

        let old_state = std::mem::replace(&mut self.state, EngineState::Idle);

        match old_state {
            EngineState::Idle => {
                // Spurious timeout — state already transitioned to Idle.
                // pass_through to avoid suppressing unrelated keys.
                Response::pass_through().with_kill_timer(TIMER_PENDING)
            }
            EngineState::PendingChar(pending) => self.timeout_pending_char(&pending),
            EngineState::PendingThumb(thumb) => self.timeout_pending_thumb(
                thumb.scan_code,
                thumb.vk_code,
                thumb.timestamp,
                composing,
                thumb.modifier_key,
                thumb.injected,
            ),
            EngineState::PendingCharThumb {
                char_key,
                thumb,
                char1_released_at,
            } => self.timeout_pending_char_thumb(&char_key, &thumb, char1_released_at),
            // 投機出力済み → タイムアウト = 親指キー未到着 → 投機出力は正しかった → Idle へ
            EngineState::SpeculativeChar(_) => Response::consume().with_kill_timer(TIMER_PENDING),
        }
    }
}

// ── lookup_face / lookup_kana_at: private ヘルパーの直接テスト ──
//
// lookup_kana_at は module-private (pub(crate) ではない) なので、他モジュールの
// engine::tests からは呼べない。ここに同一モジュール内のテストとして置く。
#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_fsm() -> NicolaFsm {
        NicolaFsm::new(
            YabLayout {
                name: "test".to_string(),
                normal: YabFace::new(),
                left_thumb: YabFace::new(),
                right_thumb: YabFace::new(),
                shift: YabFace::new(),
                left_thumb_shift: YabFace::new(),
                right_thumb_shift: YabFace::new(),
            },
            VkCode(0x1D),
            VkCode(0x1C),
            100,
            ConfirmMode::Wait,
            30,
        )
    }

    #[test]
    fn lookup_face_extracts_kana_from_romaji_value() {
        // YabValue::Romaji { kana: Some(ch), .. } のアームが削除されて `_ => None` に
        // フォールすると、kana が None になってしまう（KeyAction 自体は
        // From<&YabValue> 側で独立に 'か' になるため、kana だけが壊れる）。
        let fsm = make_test_fsm();
        let pos = PhysicalPos::new(0, 0);
        let mut face = YabFace::new();
        face.insert(
            pos,
            YabValue::Romaji {
                romaji: "ka".to_string(),
                kana: Some('か'),
            },
        );
        let (action, kana) = fsm.lookup_face(Some(pos), &face).unwrap();
        assert!(matches!(action, KeyAction::Char('か')));
        assert_eq!(kana, Some('か'), "Romaji value の kana がそのまま返るべき");
    }

    #[test]
    fn lookup_face_romaji_without_resolved_kana_returns_none() {
        // kana: None の Romaji（拗音等）では、アームが削除されても偶然 None になり
        // 区別できないため、kana: Some の場合と対で確認しておく。
        let fsm = make_test_fsm();
        let pos = PhysicalPos::new(0, 0);
        let mut face = YabFace::new();
        face.insert(
            pos,
            YabValue::Romaji {
                romaji: "kya".to_string(),
                kana: None,
            },
        );
        let (_, kana) = fsm.lookup_face(Some(pos), &face).unwrap();
        assert_eq!(kana, None);
    }

    #[test]
    fn hiragana_delegate_to_open_axis_fires_on_non_injected_solo_tap() {
        let mut fsm = make_test_fsm();
        let hiragana_vk = VkCode(0x70);
        fsm.set_hiragana_katakana_thumb_key_config(Some(hiragana_vk), None);
        fsm.set_hiragana_delegate_to_open_axis(Some(crate::types::ShadowImeAction::TurnOff));
        let (resolved, request) =
            fsm.resolve_pending_thumb_as_single(ScanCode(0x39), hiragana_vk, None, false, false);
        assert!(resolved.actions.is_empty());
        assert_eq!(request, Some(crate::types::ShadowImeAction::TurnOff));
    }

    #[test]
    fn katakana_delegate_to_open_axis_fires_on_non_injected_solo_tap() {
        let mut fsm = make_test_fsm();
        let katakana_vk = VkCode(0x71);
        fsm.set_hiragana_katakana_thumb_key_config(None, Some(katakana_vk));
        fsm.set_katakana_delegate_to_open_axis(Some(crate::types::ShadowImeAction::Toggle));
        let (resolved, request) =
            fsm.resolve_pending_thumb_as_single(ScanCode(0x39), katakana_vk, None, false, false);
        assert!(resolved.actions.is_empty());
        assert_eq!(request, Some(crate::types::ShadowImeAction::Toggle));
    }

    #[test]
    fn hiragana_delegate_to_open_axis_ignores_injected_solo_tap() {
        let mut fsm = make_test_fsm();
        let hiragana_vk = VkCode(0x70);
        fsm.set_hiragana_katakana_thumb_key_config(Some(hiragana_vk), None);
        fsm.set_hiragana_delegate_to_open_axis(Some(crate::types::ShadowImeAction::TurnOn));
        let (resolved, request) =
            fsm.resolve_pending_thumb_as_single(ScanCode(0x39), hiragana_vk, None, true, false);
        assert!(matches!(resolved.actions.as_slice(), [KeyAction::Key(vk)] if *vk == hiragana_vk));
        assert_eq!(request, None);
    }

    #[test]
    fn hiragana_delegate_to_open_axis_none_falls_back_to_default_passthrough() {
        let mut fsm = make_test_fsm();
        let hiragana_vk = VkCode(0x70);
        fsm.set_hiragana_katakana_thumb_key_config(Some(hiragana_vk), None);
        let (resolved, request) =
            fsm.resolve_pending_thumb_as_single(ScanCode(0x39), hiragana_vk, None, false, false);
        assert!(matches!(resolved.actions.as_slice(), [KeyAction::Key(vk)] if *vk == hiragana_vk));
        assert_eq!(request, None);
    }

    // `timeout_pending_thumb`（PendingThumbタイムアウト経路）は
    // `resolve_pending_thumb_as_single`とは別の呼び出し口であり、
    // `PendingThumbData::injected`を正しく引き継がないと、注入された
    // 偽の単独タップがタイムアウト経由でdelegateを発火させてしまう
    // （/codex-review指摘、"pending-thumb timeout path"でのBUG-14
    // ガードバイパス）。直接呼び出し経路（上記テスト群）だけでなく、
    // タイムアウト経路も独立して固定する。
    #[test]
    fn timeout_pending_thumb_ignores_injected_solo_tap() {
        let mut fsm = make_test_fsm();
        let hiragana_vk = VkCode(0x70);
        fsm.set_hiragana_katakana_thumb_key_config(Some(hiragana_vk), None);
        fsm.set_hiragana_delegate_to_open_axis(Some(crate::types::ShadowImeAction::TurnOn));
        let resp = fsm.timeout_pending_thumb(ScanCode(0x39), hiragana_vk, 0, false, None, true);
        assert!(
            matches!(resp.actions.as_slice(), [KeyAction::Key(vk)] if *vk == hiragana_vk),
            "injectedな単独タップはPassthroughへフォールバックするはず、実際: {:?}",
            resp.actions
        );
        assert_eq!(
            fsm.take_ime_open_requested(),
            None,
            "injectedな単独タップがタイムアウト経由でdelegateを発火させてはならない"
        );
    }

    #[test]
    fn timeout_pending_thumb_fires_delegate_for_non_injected_solo_tap() {
        let mut fsm = make_test_fsm();
        let hiragana_vk = VkCode(0x70);
        fsm.set_hiragana_katakana_thumb_key_config(Some(hiragana_vk), None);
        fsm.set_hiragana_delegate_to_open_axis(Some(crate::types::ShadowImeAction::TurnOff));
        let resp = fsm.timeout_pending_thumb(ScanCode(0x39), hiragana_vk, 0, false, None, false);
        assert!(
            resp.actions.is_empty(),
            "delegate発火時はactionsが空のはず、実際: {:?}",
            resp.actions
        );
        assert_eq!(
            fsm.take_ime_open_requested(),
            Some(crate::types::ShadowImeAction::TurnOff),
            "非injectedな単独タップはタイムアウト経由でもdelegateが正しく発火するはず"
        );
    }

    #[test]
    fn lookup_kana_at_returns_kana_for_romaji_value_on_normal_face() {
        // lookup_kana_at -> None に置換されても、Some(Default::default())
        // (= Some('\0')) に置換されても、この具体的な非ヌル文字と食い違うため検出できる。
        let mut fsm = make_test_fsm();
        let pos = PhysicalPos::new(1, 1);
        fsm.layout.normal.insert(
            pos,
            YabValue::Romaji {
                romaji: "ka".to_string(),
                kana: Some('か'),
            },
        );
        assert_eq!(fsm.lookup_kana_at(Some(pos), Face::Normal), Some('か'));
    }

    #[test]
    fn lookup_kana_at_returns_none_for_undefined_position() {
        let fsm = make_test_fsm();
        let pos = PhysicalPos::new(3, 3);
        assert_eq!(fsm.lookup_kana_at(Some(pos), Face::Normal), None);
    }

    // ── ADR-115: 打鍵列機能 ──

    #[test]
    fn enter_speculative_char_rejects_sequence_without_changing_state() {
        let mut fsm = make_test_fsm();
        let pos = PhysicalPos::new(0, 0);
        let key = PendingKey {
            pos: Some(pos),
            scan_code: ScanCode(1),
            vk_code: VkCode(0x41),
            timestamp: 0,
        };
        let sequence_action = KeyAction::Sequence(vec![KeyAction::Char('あ')]);
        let accepted = fsm.enter_speculative_char(key, &sequence_action);
        assert!(
            !accepted,
            "Sequence must be rejected by the speculative guard"
        );
        assert!(
            !matches!(fsm.state, EngineState::SpeculativeChar(_)),
            "state must not transition to SpeculativeChar when guard rejects"
        );
    }

    #[test]
    fn enter_speculative_char_accepts_non_sequence_action() {
        let mut fsm = make_test_fsm();
        let pos = PhysicalPos::new(0, 0);
        let key = PendingKey {
            pos: Some(pos),
            scan_code: ScanCode(1),
            vk_code: VkCode(0x41),
            timestamp: 0,
        };
        let accepted = fsm.enter_speculative_char(key, &KeyAction::Char('あ'));
        assert!(accepted);
        assert!(matches!(fsm.state, EngineState::SpeculativeChar(_)));
    }

    #[test]
    fn release_only_treats_ctrl_chord_and_sequence_as_pass_through_like_special_key() {
        // 決定6: Sequence/CtrlChord は Char/Romaji/Key と違い解放すべき
        // 片割れを持たないため、既存の SpecialKey/KeySequence/Suppress と
        // 同じ pass_through 扱いになる（意味的な挙動は変えない、網羅
        // match化のみ）。
        for action in [
            KeyAction::Sequence(vec![KeyAction::Char('あ')]),
            KeyAction::CtrlChord(VkCode(0x4D)),
        ] {
            let mut fsm = make_test_fsm();
            let scan = ScanCode(1);
            fsm.output_history.push(OutputEntry {
                scan_code: scan,
                romaji: String::new(),
                kana: None,
                action,
            });
            let ev = RawKeyEvent {
                vk_code: VkCode(0x41),
                scan_code: scan,
                event_type: KeyEventType::KeyUp,
                extra_info: 0,
                timestamp: 0,
                key_classification: crate::types::KeyClassification::Char,
                physical_pos: None,
                ime_relevance: crate::types::ImeRelevance::default(),
                modifier_key: None,
                modifier_snapshot: crate::types::ModifierState::default(),
                injected: false,
            };
            let r = fsm.release_only(&ev);
            assert!(r.actions.is_empty(), "pass_through must not emit actions");
            assert!(!r.consumed, "pass_through must not consume the event");
        }
    }

    #[test]
    fn on_timeout_speculative_with_sequence_cell_keeps_pending_char_and_rearms_timer() {
        // 決定7: TwoPhase の Phase1→Phase2 タイムアウト時、対象セルが
        // Sequence だと enter_speculative_char が拒否するため、Phase2への
        // 遷移（SpeculativeChar化 + 即時出力）を諦め、PendingChar を維持した
        // まま残り時間で TIMER_PENDING を張り直す（actions無し・consumed=false）。
        // これにより確定は既存の timeout_pending_char 経路に一本化され、
        // 満了前に親指キーが来れば chord 判定の受付窓が縮まらない
        // （Opus実装後レビュー M3: この分岐に既存テストが無かった）。
        let mut fsm = make_test_fsm();
        let pos = PhysicalPos::new(0, 0);
        fsm.layout.normal.insert(
            pos,
            YabValue::Sequence(vec![YabValue::Literal("あ".to_string())]),
        );
        let pending = PendingKey {
            pos: Some(pos),
            scan_code: ScanCode(1),
            vk_code: VkCode(0x41),
            timestamp: 0,
        };
        fsm.state = EngineState::PendingChar(pending);

        let resp = fsm.on_timeout_speculative();

        assert!(resp.actions.is_empty(), "must not emit any actions");
        assert!(!resp.consumed, "must not mark the timeout as consumed");
        assert!(
            matches!(fsm.state, EngineState::PendingChar(_)),
            "state must remain PendingChar, not transition to SpeculativeChar, got {:?}",
            fsm.state
        );
        let expected_remaining_us = fsm.threshold_us.saturating_sub(fsm.speculative_delay_us);
        let expects_rearmed_pending_timer = resp.timers.iter().any(|cmd| {
            matches!(
                cmd,
                timed_fsm::TimerCommand::Set {
                    id,
                    duration,
                } if *id == TIMER_PENDING
                    && *duration == std::time::Duration::from_micros(expected_remaining_us)
            )
        });
        assert!(
            expects_rearmed_pending_timer,
            "must rearm TIMER_PENDING with remaining_us={expected_remaining_us}, got {:?}",
            resp.timers
        );
    }

    // ── ADR-120 決定0a: `/code-review` 指摘の回帰テスト ──
    // `own_decision_output`/`OwnDecisionOutput` は private フィールドなので、
    // このファイル内の（同一モジュールの）テストからのみ直接操作できる。

    #[test]
    fn retro_eval_stats_own_decision_backspace_output_not_counted_as_correction() {
        // 所見4対応の回帰テスト: 3キー仲裁決定「自身の出力」が
        // .yab配列によって SpecialKey::Backspace に解決される場合、
        // ユーザーの実訂正操作ではないため訂正カウンタへ計上してはならない。
        let mut fsm = make_test_fsm();
        fsm.own_decision_output = Some(OwnDecisionOutput {
            remaining: 1,
            measure_since: None,
        });
        fsm.last_decision = Some(LastDecision {
            phase2_at: Some(0),
            phase1_at: None,
            baseline_at: None,
        });
        fsm.update_history(
            OutputUpdate::record(
                ScanCode(0),
                &KeyAction::SpecialKey(SpecialKey::Backspace),
                None,
            ),
            1_000,
        );
        let stats = fsm.retro_eval_stats();
        assert_eq!(
            stats.phase2_correction_histogram.iter().sum::<u64>(),
            0,
            "決定自身の出力であるBackspaceは訂正操作として計上してはならない"
        );
        assert_eq!(
            stats.baseline_decisions_total, 0,
            "決定自身の出力なのでBaselineにも計上されないはず"
        );
    }

    #[test]
    fn retro_eval_stats_genuine_backspace_correction_still_counted_after_own_output_consumed() {
        // 上のテストと対で確認する: own_decision_output が消化済み（決定自身の
        // 出力ではない）状態で来たBackspaceは、通常どおり訂正として計上される。
        let mut fsm = make_test_fsm();
        fsm.own_decision_output = None;
        fsm.last_decision = Some(LastDecision {
            phase2_at: Some(0),
            phase1_at: None,
            baseline_at: None,
        });
        fsm.update_history(
            OutputUpdate::record(
                ScanCode(0),
                &KeyAction::SpecialKey(SpecialKey::Backspace),
                None,
            ),
            1_000,
        );
        let stats = fsm.retro_eval_stats();
        assert_eq!(stats.phase2_correction_histogram.iter().sum::<u64>(), 1);
    }

    #[test]
    fn retro_eval_stats_stale_attribution_boundary_excludes_exact_ms() {
        // 所見3対応の回帰テスト: STALE_ATTRIBUTION_MS ちょうど(1600ms)は
        // 除外され(elapsed < STALE_ATTRIBUTION_MSに変更)、1599msは計上される。
        // 修正前は `elapsed <= STALE_ATTRIBUTION_MS` だったため、1600msの
        // 一点だけが bucket 6 に紛れ込みうる不整合な状態だった。
        let mut fsm = make_test_fsm();
        fsm.last_decision = Some(LastDecision {
            phase2_at: Some(0),
            phase1_at: None,
            baseline_at: None,
        });
        // ちょうど1600ms（境界値そのもの）→ 除外される。
        fsm.record_user_correction(1_600_000);
        assert_eq!(
            fsm.retro_eval_stats()
                .phase2_correction_histogram
                .iter()
                .sum::<u64>(),
            0,
            "elapsed==STALE_ATTRIBUTION_MSちょうどは除外されるはず"
        );

        fsm.last_decision = Some(LastDecision {
            phase2_at: Some(0),
            phase1_at: None,
            baseline_at: None,
        });
        // 1599ms（境界未満）→ 計上される、bucket 5(800<=x<1600)。
        fsm.record_user_correction(1_599_000);
        assert_eq!(
            fsm.retro_eval_stats().phase2_correction_histogram,
            [0, 0, 0, 0, 0, 1, 0],
            "elapsed=1599msはbucket 5に計上されるはず"
        );
    }
}
