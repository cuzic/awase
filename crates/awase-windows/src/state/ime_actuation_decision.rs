//! ADR-163 Part A（`docs/adr/163-actuation-decision-io-separation-and-replay-harness.md`）
//! 「決定関数の粒度」節・TH1b。
//!
//! actuation合流点（`ImeController::apply`・`runtime/open_chain.rs`の3関数・
//! `runtime/executor.rs::dispatch_ime_set_open`）が「何を送るか」を決める部分を、
//! Win32呼び出しから切り離した純粋関数として提供する。**このモジュール自体は
//! まだどこからも呼ばれない**（TH1b-1: 追加のみ、配線は別タスクTH1b-2）。
//! 既存の挙動を1行も変えていないことを、このファイルのユニットテストで
//! 実装元のコード（`ime_controller.rs`の4戦略・[`decide_needs_romaji_pre_write`]
//! （2026-09-10に`state/actuation_chain.rs`から移動）・
//! `runtime/executor.rs::dispatch_ime_set_open`のconv_after_open判定）と
//! 1対1で突き合わせて固定する。

use awase::engine::InputModeState;
use awase::types::VkCode;

use crate::focus::class_names::AppImeProfile;
use crate::state::actuation_chain::WriteMechanism;
use crate::state::app_ime_policy::caps;
use crate::state::conv_after_open::ConvAfterOpenId;
use crate::state::ime_kind::ImeKindId;
use crate::state::key_sequence_policy::{self, ImeOperation, KeyMechanism};

/// 戦略選択・VK選択・already-matched判定に実際に効く4値（ADR-163「決定入力の最小化」）。
///
/// `ImeControlView`（windows-gated）が運ぶ残りのフィールド（`class_name`/
/// `focus_gen`/`composition_active`等）は診断ログ専用で決定には効かないため、
/// あえてここには含めない——`ImeControlView`自体はungate化しない
/// （ADR-163 round2 T5）。windows側に`impl From<&ImeControlView<'_>> for
/// DecisionInputs`を後で追加し、そこから本モジュールの関数を呼ぶ。
///
/// # この型のフィールドを増やす前に読むこと（ADR-163 Part D 決定D8）
///
/// `DecisionInputs`（および`ActuationDecisionRecord`/`AttemptRecord`）は
/// [`journal.rs::JournalEntry::ActuationDecision`](../../journal/enum.JournalEntry.html)
/// 経由でbug report（ADR-095）の`journal_json`に相乗りし、実ユーザー環境から
/// 収集される。現状は打鍵の生の文字・ローマ字・かなを一切含まず、アプリ名や
/// ウィンドウクラス名（`class_name`）も上記のとおり意図的に除外されている。
/// **将来「診断のため`class_name`も載せよう」のような1行を追加すると、この
/// 除外という唯一の防壁を素通りして、ユーザーが何のソフトを使っているかを
/// 送信するチャネルに変質する**（`bug_report.rs`の`BugReportGjiKeymapSummary`が
/// 残す同種の警告と同じ構造の罠）ため、フィールド追加は録取される情報の変化を
/// 都度この観点で見直すこと。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DecisionInputs {
    pub profile: AppImeProfile,
    pub kind: ImeKindId,
    /// `ControlLog.shadow_on`。`None` = 未知（BUG-113: `bool`に潰さないこと）。
    pub shadow_on: Option<bool>,
    pub belief_input_mode: InputModeState,
}

/// `ImeController::apply`/`run_open_chain_async`/`dispatch_ime_set_open`冒頭の
/// InputRelayゲートの判定結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GateResult {
    /// InputRelayプロファイル: awaseはactuationを所有しない（issue #136/BUG-90決定4）。
    NotOwned,
    Proceed,
}

/// この決定がどのactuation合流点で行われたか（ADR-163 round3 U2）。
///
/// `fix-requires-evidence.md`の「IME actuation合流点」表が挙げる5箇所のうち
/// 本ADRが対象とする4関数+これらが内部で辿る経路を表す。`ImmCrossWrite`は
/// `runtime/open_chain.rs::imm_cross_write`、`FallbackWrite`は同`fallback_write`、
/// `RunOpenChainAsync`は同`run_open_chain_async`冒頭のゲート、`DispatchImeSetOpen`は
/// `runtime/executor.rs::dispatch_ime_set_open`。`ReassertExplicitPhysicalKey`/
/// `ForceOnRomajiCorrection`は記録専用ラベルであり、command計算へは使わない
/// （ADR-163 Part D B1）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DecisionSite {
    Sync,
    ImmCrossWrite,
    FallbackWrite,
    RunOpenChainAsync,
    DispatchImeSetOpen,
    ReassertExplicitPhysicalKey,
    ForceOnRomajiCorrection,
}

/// 1機構分の「何を送るか」の決定結果（実I/Oは含まない）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum MechanismCommand {
    /// 同期`ImmCrossProcessStrategy::apply`が呼ぶ`set_ime_open_cross_process(open)`相当。
    SetOpenCrossProcessSync(bool),
    /// 非同期・宛先未捕獲の`set_ime_open_cross_process_async(open)`相当
    /// （`ImmCrossOp::Untargeted`、shadow-toggle OFF経路）。
    SetOpenCrossProcessAsyncUntargeted(bool),
    /// 非同期・宛先捕獲済みの`set_ime_open_then_conv_for_target`相当
    /// （`ImmCrossOp::Targeted`）。
    SetOpenThenConvForTarget {
        open: bool,
        conv_after_open: ConvAfterOpenId,
    },
    /// `send_ime_mode_key(vk)`相当（GjiDirect/MsImeDirect）。
    SendVk(VkCode),
    /// `post_kanji_toggle_to_focused()`相当。
    PostKanjiToggle,
}

/// `ImeController::apply`/`run_open_chain_async`/`dispatch_ime_set_open`冒頭の
/// InputRelayゲート（`view.focus.profile == AppImeProfile::InputRelay`）と同一の判定。
#[must_use]
pub(crate) const fn decide_gate(inputs: DecisionInputs) -> GateResult {
    if matches!(inputs.profile, AppImeProfile::InputRelay) {
        GateResult::NotOwned
    } else {
        GateResult::Proceed
    }
}

/// sync経路（`ImeController::apply`）が使う機構チェーン。
/// `ime_controller.rs::caps_chain_for`と同一（`caps(profile.into(), kind).chain`）。
/// async経路（`open_chain.rs`）は`WriteMechanism::ALL`固定のまま変更しない
/// （ADR-163 round2 T2、ADR-159の理由により意図的に非対称）。
#[must_use]
pub(crate) fn decide_chain(inputs: DecisionInputs) -> &'static [WriteMechanism] {
    caps(inputs.profile.into(), inputs.kind).chain
}

/// `GjiDirectStrategy::apply`のalready-matched判定
/// （旧`ime_controller.rs::gji_direct_already_matches`と同一）。
#[must_use]
const fn gji_direct_already_matches(shadow_on: Option<bool>, open: bool) -> bool {
    matches!(shadow_on, Some(v) if v == open)
}

/// IME ON の直前に ROMAN ビットを補完する同期 IMC write が要るか
/// （ADR-089 §6 Phase C item 12 = ADR-086 INV-14 の未移行分の是正）。
///
/// 2026-09-10、`state/actuation_chain.rs`から本モジュールへ移動しリネームした
/// （旧名`needs_romaji_pre_write`）。
/// 「`decide_chain`/`decide_attempt`と同じ判断入力を扱うのに命名も配置も
/// 揃っていなかった」ことが動機——`decide_attempt`（下記）が`WriteMechanism`/
/// `open`/`kind`/`belief_input_mode`を受けて本関数をそのまま呼ぶ、隣接する
/// 「1機構分の判断」の一部である。挙動は移動前と1バイトも変えていない。
///
/// **すぐ下の[`decide_dispatch_conv_after_open`]とは意図的に別の条件式である**
/// （ADR-163 round2 R5「次点候補」参照）。本関数は`mechanism ∈ {ImmCross,
/// MsImeDirect}`かつ`kind == MsIme`の場合のみ真になるが、
/// `decide_dispatch_conv_after_open`は`open`と`belief_input_mode`だけで決まり、
/// mechanism/kind条件を一切持たない。統合を試みると実際に差分が出る可能性が
/// 高いとADR-163が既に指摘済みであり、**この2つを1つの条件式へ統合しないこと**
/// （統合はADR-163「今後の議論」4番の別タスク）。
///
/// # なぜこの述語がここ（ungated）にあるのか
///
/// Phase C 以前、この条件は `ime_controller.rs` の 2 つの戦略の中に**別々に**
/// 書かれていた（`ImmCrossProcessStrategy::apply` と
/// `MsImeDirectStrategy::apply`。どちらも `crate::ime::set_ime_romaji_mode()` を
/// 直接呼んでいた）。どちらも Win32 FFI と同居していたため Linux から
/// 条件を検査できず、`output/conv_actuation.rs` の doc が
/// 「ADR-086 Phase 1〜2 の『7 経路』の数え漏れ」と書いていた 2 経路そのもので
/// あった。Phase C で **書き込み口を 1 箇所（`ime_controller::apply_mechanism`）に
/// 統合**し、その発火条件だけをここへ純粋関数として切り出した。
///
/// # 条件（Phase C 以前と同値であること）
///
/// - `open == true` のときだけ（OFF 方向は ROMAN を触らない）。
/// - 機構が `ImmCross` または `MsImeDirect` のときだけ
///   （`GjiDirect` / `KanjiToggle` は元から ROMAN を書かない）。
/// - `kind == MsIme` のときだけ。旧 `ImmCrossProcessStrategy` は
///   `active_ime_kind == MicrosoftIme` を明示的に見ており、旧
///   `MsImeDirectStrategy` は見ていなかったが、`MsImeDirect` の
///   `is_applicable` 自体が `MicrosoftIme` を要求するため**同値**である
///   （`apply_mechanism` は `is_applicable` が真の機構に対してしか呼ばれない）。
/// - `belief_input_mode != ObservedKana` のときだけ——ユーザーが意図的に
///   かな入力を選んでいる状態を ROMAN で上書きしない（既存の保護、
///   `runtime/mod.rs::force_on_and_correct_romaji` の N2 も参照）。
#[must_use]
pub(crate) const fn decide_needs_romaji_pre_write(
    mechanism: WriteMechanism,
    open: bool,
    kind: ImeKindId,
    belief_input_mode: InputModeState,
) -> bool {
    open && matches!(
        mechanism,
        WriteMechanism::ImmCross | WriteMechanism::MsImeDirect
    ) && matches!(kind, ImeKindId::MsIme)
        && !matches!(belief_input_mode, InputModeState::ObservedKana)
}

/// `runtime/executor.rs::dispatch_ime_set_open`が`ImmCrossOp::Targeted`を組み立てる際の
/// conv_after_open判定（実コード該当箇所のコメントでは「issue #138診断」節の直前）。
///
/// **すぐ上の[`decide_needs_romaji_pre_write`]とは意図的に別の条件式である**（ADR-163
/// round2 R5「次点候補」参照）。詳細はそちらのdocコメント参照。
#[must_use]
pub(crate) fn decide_dispatch_conv_after_open(
    inputs: DecisionInputs,
    open: bool,
) -> ConvAfterOpenId {
    if open && !matches!(inputs.belief_input_mode, InputModeState::ObservedKana) {
        ConvAfterOpenId::Write(None)
    } else {
        ConvAfterOpenId::Skip
    }
}

/// 1 attempt分（1機構への1回のwrite判断）の決定。
///
/// 戻り値の1つ目は「この機構をwriteする前にROMAN補完(`romaji_pre_write`、
/// `send_ime_control(IMC_SETCONVERSIONMODE)`)を行うか」——`apply_mechanism`が
/// `strategy_for(mechanism).apply()`の前に呼ぶ既存の`romaji_pre_write`関数と
/// 同一の判定（[`decide_needs_romaji_pre_write`]をそのまま呼ぶだけ）。2つ目が
/// `MechanismCommand`（`None` = already-matchedで送信しない）。
///
/// `WriteMechanism::ImmCross`は`site == Sync`の場合のみここで決定する
/// （`SetOpenCrossProcessSync`）。`ImmCrossWrite`/`FallbackWrite`/
/// `RunOpenChainAsync`/`DispatchImeSetOpen`でのImmCrossは、`ImmCrossOp`
/// （宛先捕獲の有無）という`DecisionInputs`に無い情報（呼び出し元が
/// `.await`前に構築する`ActuationTarget`）に依存するため、呼び出し元が
/// `decide_dispatch_conv_after_open`等を使って別途組み立てる
/// （ADR-163 round3 U1のdocコメント参照）。ここでは`None`を返す
/// （="この関数の責務外"、パニックにはしない）。
#[must_use]
pub(crate) fn decide_attempt(
    inputs: DecisionInputs,
    site: DecisionSite,
    mechanism: WriteMechanism,
    open: bool,
) -> (bool, Option<MechanismCommand>) {
    let romaji_pre_write =
        decide_needs_romaji_pre_write(mechanism, open, inputs.kind, inputs.belief_input_mode);
    let command = match (mechanism, site) {
        (WriteMechanism::ImmCross, DecisionSite::Sync) => {
            Some(MechanismCommand::SetOpenCrossProcessSync(open))
        }
        (WriteMechanism::ImmCross, _) => None,
        (WriteMechanism::GjiDirect, _) => {
            if gji_direct_already_matches(inputs.shadow_on, open) {
                None
            } else {
                Some(MechanismCommand::SendVk(key_sequence_policy::ime_key_for(
                    KeyMechanism::GjiDirect,
                    ImeOperation::from_open(open),
                )))
            }
        }
        (WriteMechanism::MsImeDirect, _) => {
            Some(MechanismCommand::SendVk(key_sequence_policy::ime_key_for(
                KeyMechanism::MsImeDirect,
                ImeOperation::from_open(open),
            )))
        }
        (WriteMechanism::KanjiToggle, _) => Some(MechanismCommand::PostKanjiToggle),
    };
    (romaji_pre_write, command)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(
        profile: AppImeProfile,
        kind: ImeKindId,
        shadow_on: Option<bool>,
        belief_input_mode: InputModeState,
    ) -> DecisionInputs {
        DecisionInputs {
            profile,
            kind,
            shadow_on,
            belief_input_mode,
        }
    }

    // ── decide_gate ──────────────────────────────────────────────────────

    #[test]
    fn gate_not_owned_for_input_relay() {
        let i = inputs(
            AppImeProfile::InputRelay,
            ImeKindId::Gji,
            None,
            InputModeState::Unknown,
        );
        assert_eq!(decide_gate(i), GateResult::NotOwned);
    }

    #[test]
    fn gate_proceeds_for_non_input_relay_profiles() {
        for profile in [
            AppImeProfile::Standard,
            AppImeProfile::Imm32Unavailable,
            AppImeProfile::TsfNative,
        ] {
            let i = inputs(profile, ImeKindId::Gji, None, InputModeState::Unknown);
            assert_eq!(decide_gate(i), GateResult::Proceed, "{profile:?}");
        }
    }

    // ── decide_chain ─────────────────────────────────────────────────────

    #[test]
    fn chain_matches_caps_table() {
        for profile in [
            AppImeProfile::Standard,
            AppImeProfile::Imm32Unavailable,
            AppImeProfile::TsfNative,
        ] {
            for kind in ImeKindId::ALL {
                let i = inputs(profile, kind, None, InputModeState::Unknown);
                assert_eq!(
                    decide_chain(i),
                    caps(profile.into(), kind).chain,
                    "{profile:?} {kind:?}"
                );
            }
        }
    }

    // ── decide_needs_romaji_pre_write（ADR-089 Phase C item 12 / ADR-086 INV-14、
    //    2026-09-10に state/actuation_chain.rs から移動）──

    use awase::engine::AssumedReason;

    const ALL_INPUT_MODES: [InputModeState; 5] = [
        InputModeState::ObservedRomaji,
        InputModeState::ObservedKana,
        InputModeState::ObservedEisu,
        InputModeState::AssumedRomaji {
            reason: AssumedReason::ImmBridgeBroken,
        },
        InputModeState::Unknown,
    ];

    /// Phase C 以前の 2 戦略の条件と同値であることを全数で固定する。
    ///
    /// 旧条件:
    /// - `ImmCrossProcessStrategy::apply`:
    ///   `open && active_ime_kind == MicrosoftIme && belief != ObservedKana`
    /// - `MsImeDirectStrategy::apply`: `open && belief != ObservedKana`
    ///   （`is_applicable` が `MicrosoftIme` を要求するため kind 条件は暗黙）
    #[test]
    fn needs_romaji_pre_write_condition_matches_the_pre_phase_c_strategies() {
        for mechanism in WriteMechanism::ALL {
            for open in [true, false] {
                for kind in ImeKindId::ALL {
                    for mode in ALL_INPUT_MODES {
                        let expected =
                            open && matches!(
                                mechanism,
                                WriteMechanism::ImmCross | WriteMechanism::MsImeDirect
                            ) && kind == ImeKindId::MsIme
                                && mode != InputModeState::ObservedKana;
                        assert_eq!(
                            decide_needs_romaji_pre_write(mechanism, open, kind, mode),
                            expected,
                            "{mechanism:?} open={open} {kind:?} {mode:?}"
                        );
                    }
                }
            }
        }
    }

    /// GJI 経路では ROMAN 補完を一切行わない（Phase C 以前も同じ）。
    #[test]
    fn needs_romaji_pre_write_never_fires_for_gji_mechanisms() {
        for mechanism in [WriteMechanism::GjiDirect, WriteMechanism::KanjiToggle] {
            for kind in ImeKindId::ALL {
                assert!(!decide_needs_romaji_pre_write(
                    mechanism,
                    true,
                    kind,
                    InputModeState::Unknown
                ));
            }
        }
    }

    /// `ObservedKana`（ユーザーが意図的にかな入力を選んだ状態）は上書きしない。
    #[test]
    fn needs_romaji_pre_write_respects_observed_kana() {
        assert!(!decide_needs_romaji_pre_write(
            WriteMechanism::MsImeDirect,
            true,
            ImeKindId::MsIme,
            InputModeState::ObservedKana
        ));
    }

    // ── decide_dispatch_conv_after_open（executor.rsの「第3のROMAN判定」の固定）──

    #[test]
    fn dispatch_conv_after_open_writes_roman_only_when_opening_and_not_observed_kana() {
        let i = inputs(
            AppImeProfile::Standard,
            ImeKindId::MsIme,
            None,
            InputModeState::Unknown,
        );
        assert_eq!(
            decide_dispatch_conv_after_open(i, true),
            ConvAfterOpenId::Write(None)
        );
    }

    #[test]
    fn dispatch_conv_after_open_skips_when_closing() {
        let i = inputs(
            AppImeProfile::Standard,
            ImeKindId::MsIme,
            None,
            InputModeState::Unknown,
        );
        assert_eq!(
            decide_dispatch_conv_after_open(i, false),
            ConvAfterOpenId::Skip
        );
    }

    #[test]
    fn dispatch_conv_after_open_skips_when_observed_kana_even_if_opening() {
        let i = inputs(
            AppImeProfile::Standard,
            ImeKindId::MsIme,
            None,
            InputModeState::ObservedKana,
        );
        assert_eq!(
            decide_dispatch_conv_after_open(i, true),
            ConvAfterOpenId::Skip
        );
    }

    #[test]
    fn dispatch_conv_after_open_ignores_mechanism_and_kind_unlike_decide_needs_romaji_pre_write() {
        // GJI kind・open=true・非ObservedKana でも Write(None) になる
        // （decide_needs_romaji_pre_write なら kind==MsIme 条件で false になる場面）。
        let i = inputs(
            AppImeProfile::Standard,
            ImeKindId::Gji,
            None,
            InputModeState::Unknown,
        );
        assert_eq!(
            decide_dispatch_conv_after_open(i, true),
            ConvAfterOpenId::Write(None)
        );
        assert!(!decide_needs_romaji_pre_write(
            WriteMechanism::ImmCross,
            true,
            ImeKindId::Gji,
            InputModeState::Unknown
        ));
    }

    // ── decide_attempt: GjiDirect ────────────────────────────────────────

    #[test]
    fn gji_direct_skips_when_shadow_already_matches() {
        let i = inputs(
            AppImeProfile::Standard,
            ImeKindId::Gji,
            Some(true),
            InputModeState::Unknown,
        );
        let (_, cmd) = decide_attempt(i, DecisionSite::Sync, WriteMechanism::GjiDirect, true);
        assert_eq!(cmd, None);
    }

    #[test]
    fn gji_direct_sends_vk_when_shadow_unknown() {
        // shadow_on == None（未知）は「確認済みOFF」ではないため送信する
        // （BUG-113、bool に潰さないことの直接のテスト）。
        let i = inputs(
            AppImeProfile::Standard,
            ImeKindId::Gji,
            None,
            InputModeState::Unknown,
        );
        let (_, cmd) = decide_attempt(i, DecisionSite::Sync, WriteMechanism::GjiDirect, true);
        assert_eq!(
            cmd,
            Some(MechanismCommand::SendVk(key_sequence_policy::ime_key_for(
                KeyMechanism::GjiDirect,
                ImeOperation::Open
            )))
        );
    }

    #[test]
    fn gji_direct_sends_vk_when_shadow_mismatches() {
        let i = inputs(
            AppImeProfile::Standard,
            ImeKindId::Gji,
            Some(false),
            InputModeState::Unknown,
        );
        let (_, cmd) = decide_attempt(i, DecisionSite::Sync, WriteMechanism::GjiDirect, true);
        assert!(cmd.is_some());
    }

    // ── decide_attempt: GjiDirect（OFF方向、open=false）──────────────────────
    //
    // 上記3件（open=true）に対する対称ケース。削除された旧
    // `ime_controller.rs`側のテスト（`gji_direct_already_matches_treats_
    // unknown_shadow_as_not_matched`/`gji_direct_apply_off_is_already_matched_
    // when_shadow_already_off`）はBUG-113の本来の症状であるOFF方向を直接
    // 検証していたが、この3件（open=true専用）だけではその回帰を検知
    // できなかった（/code-review PR#195指摘）。

    #[test]
    fn gji_direct_skips_when_shadow_already_matches_close_direction() {
        let i = inputs(
            AppImeProfile::Standard,
            ImeKindId::Gji,
            Some(false),
            InputModeState::Unknown,
        );
        let (_, cmd) = decide_attempt(i, DecisionSite::Sync, WriteMechanism::GjiDirect, false);
        assert_eq!(cmd, None);
    }

    #[test]
    fn gji_direct_sends_vk_when_shadow_unknown_close_direction() {
        // BUG-113 Blocker: shadow_on == None（未知）は open=false 方向でも
        // 「確認済みOFF」と誤認してはならない。`unwrap_or(false)` で bool に
        // 潰していた旧実装はここを壊していた（docs/known-bugs.md BUG-113）。
        let i = inputs(
            AppImeProfile::Standard,
            ImeKindId::Gji,
            None,
            InputModeState::Unknown,
        );
        let (_, cmd) = decide_attempt(i, DecisionSite::Sync, WriteMechanism::GjiDirect, false);
        assert_eq!(
            cmd,
            Some(MechanismCommand::SendVk(key_sequence_policy::ime_key_for(
                KeyMechanism::GjiDirect,
                ImeOperation::Close
            )))
        );
    }

    #[test]
    fn gji_direct_sends_vk_when_shadow_mismatches_close_direction() {
        let i = inputs(
            AppImeProfile::Standard,
            ImeKindId::Gji,
            Some(true),
            InputModeState::Unknown,
        );
        let (_, cmd) = decide_attempt(i, DecisionSite::Sync, WriteMechanism::GjiDirect, false);
        assert!(cmd.is_some());
    }

    // ── decide_attempt: MsImeDirect（already-matched判定を持たない）──────────

    #[test]
    fn ms_ime_direct_always_sends_regardless_of_shadow() {
        for shadow_on in [None, Some(true), Some(false)] {
            let i = inputs(
                AppImeProfile::TsfNative,
                ImeKindId::MsIme,
                shadow_on,
                InputModeState::Unknown,
            );
            let (_, cmd) = decide_attempt(i, DecisionSite::Sync, WriteMechanism::MsImeDirect, true);
            assert!(cmd.is_some(), "shadow_on={shadow_on:?}");
        }
    }

    // ── decide_attempt: KanjiToggle（常に送信）──────────────────────────────

    #[test]
    fn kanji_toggle_always_sends() {
        let i = inputs(
            AppImeProfile::Standard,
            ImeKindId::MsIme,
            Some(true),
            InputModeState::Unknown,
        );
        let (_, cmd) = decide_attempt(i, DecisionSite::Sync, WriteMechanism::KanjiToggle, true);
        assert_eq!(cmd, Some(MechanismCommand::PostKanjiToggle));
    }

    // ── decide_attempt: ImmCross ─────────────────────────────────────────

    #[test]
    fn imm_cross_sync_site_sends_set_open_cross_process_sync() {
        let i = inputs(
            AppImeProfile::Standard,
            ImeKindId::MsIme,
            None,
            InputModeState::Unknown,
        );
        let (_, cmd) = decide_attempt(i, DecisionSite::Sync, WriteMechanism::ImmCross, true);
        assert_eq!(cmd, Some(MechanismCommand::SetOpenCrossProcessSync(true)));
    }

    #[test]
    fn imm_cross_non_sync_sites_return_none_command() {
        // これらのsiteでのImmCrossコマンドは呼び出し側がImmCrossOpの形状
        // （宛先捕獲の有無）に応じて別途組み立てる。decide_attemptの責務外。
        let i = inputs(
            AppImeProfile::Standard,
            ImeKindId::MsIme,
            None,
            InputModeState::Unknown,
        );
        for site in [
            DecisionSite::ImmCrossWrite,
            DecisionSite::FallbackWrite,
            DecisionSite::RunOpenChainAsync,
            DecisionSite::DispatchImeSetOpen,
            DecisionSite::ReassertExplicitPhysicalKey,
            DecisionSite::ForceOnRomajiCorrection,
        ] {
            let (_, cmd) = decide_attempt(i, site, WriteMechanism::ImmCross, true);
            assert_eq!(cmd, None, "{site:?}");
        }
    }

    // ── decide_attempt: romaji_pre_write の bool は decide_needs_romaji_pre_write と一致 ──

    #[test]
    fn romaji_pre_write_flag_matches_decide_needs_romaji_pre_write() {
        for mechanism in [
            WriteMechanism::ImmCross,
            WriteMechanism::GjiDirect,
            WriteMechanism::MsImeDirect,
            WriteMechanism::KanjiToggle,
        ] {
            for kind in ImeKindId::ALL {
                for open in [true, false] {
                    for belief_input_mode in [InputModeState::Unknown, InputModeState::ObservedKana]
                    {
                        let i = inputs(AppImeProfile::Standard, kind, None, belief_input_mode);
                        let (flag, _) = decide_attempt(i, DecisionSite::Sync, mechanism, open);
                        assert_eq!(
                            flag,
                            decide_needs_romaji_pre_write(mechanism, open, kind, belief_input_mode),
                            "{mechanism:?} {kind:?} open={open} {belief_input_mode:?}"
                        );
                    }
                }
            }
        }
    }
}
