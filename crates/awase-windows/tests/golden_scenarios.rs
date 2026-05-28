//! Golden Scenario テスト
//!
//! 過去のバグ修正で対処した既知シナリオを event sequence として固定し、
//! 新モデル (`ImeModel`) のリファクタリングで挙動が壊れないようにする。
//!
//! ## シナリオ一覧
//!
//! 1. LINE/Qt KANJI ON/OFF
//! 2. Ctrl+無変換 直後の Ctrl KeyUp で再アクティベートしない (b7d4cdb 防波堤)
//! 3. Ctrl+変換 IME-ON が直後に解除されない (6fce373 防波堤)
//! 4. Chrome/Edge no-imm32 で IME OFF が機能
//! 5. WezTerm TSF cold start
//! 6. Focus change 直後の stale false が desired を override しない
//! 7. panic_reset 直後の stale poll が state を壊さない
//! 8. **最重要**: stale async apply が新しい intent を壊さない (generation 照合)
//!
//! ## 実装状況
//!
//! 現在の shadow reducer (Step 3 まで) で検証可能なものは assert する。
//! ChordStarted/Ended (Step 4), ApplyRequested/Succeeded (Step 7), Barrier
//! 機構 (Step 5) が必要なものは `#[ignore]` で skip する。

use std::time::Instant;

use awase_windows::focus::class_names::AppImeProfile;
use awase_windows::state::ime_event::{
    EventTime, HwndId, ImeEvent, ImeEventEnvelope, IntentSource, ObservationConfidence,
    ObservationSource,
};
use awase_windows::state::ime_model::ImeModel;

// ── テストヘルパー ────────────────────────────────────────────────

fn envelope(seq: u64, event: ImeEvent) -> ImeEventEnvelope {
    ImeEventEnvelope {
        time: EventTime {
            seq,
            monotonic: Instant::now(),
            tick_ms: seq * 10,
        },
        event,
    }
}

fn user_intent(target: bool, source: IntentSource) -> ImeEvent {
    ImeEvent::UserImeSetIntent { target, source }
}

fn observer_reported(open: bool, source: ObservationSource) -> ImeEvent {
    ImeEvent::ObserverReported {
        open,
        source,
        hwnd: HwndId::NULL,
        confidence: ObservationConfidence::Medium,
    }
}

fn focus_changed(profile: AppImeProfile) -> ImeEvent {
    ImeEvent::FocusChanged {
        from: None,
        to: HwndId(0x1234),
        profile,
    }
}

/// event sequence を順に reduce し、最終 model を返す。
fn run_reducer(events: Vec<ImeEvent>) -> ImeModel {
    let mut model = ImeModel::new();
    for (idx, event) in events.into_iter().enumerate() {
        model.reduce(&envelope(idx as u64 + 1, event));
    }
    model
}

// ── シナリオ 1: LINE/Qt KANJI ON/OFF ────────────────────────────

#[test]
fn scenario_1_line_qt_kanji_on_off() {
    // KANJI 押下 → IME OFF (physical key)
    let model = run_reducer(vec![
        focus_changed(AppImeProfile::Standard),
        user_intent(false, IntentSource::PhysicalImeKey),
    ]);
    assert!(!model.desired_open, "KANJI で IME OFF");

    // 続けて KANJI 押下 → IME ON
    let model = run_reducer(vec![
        focus_changed(AppImeProfile::Standard),
        user_intent(false, IntentSource::PhysicalImeKey),
        user_intent(true, IntentSource::PhysicalImeKey),
    ]);
    assert!(model.desired_open, "もう一度 KANJI で IME ON");
}

// ── シナリオ 2: Ctrl+無変換 直後の Ctrl KeyUp で再アクティベートしない ─

#[test]
#[ignore = "Step 4 で ChordStarted/Ended Barrier を実装後に有効化"]
fn scenario_2_ctrl_muhenkan_does_not_reactivate_on_ctrl_keyup() {
    // 期待 sequence (Step 4 実装後):
    // 1. ChordStarted { kind: CtrlMuhenkanImeOff }
    // 2. UserImeSetIntent { target: false, source: SyncKey }
    // 3. ObserverReported { open: true, ... } ← stale
    // 4. ChordEnded { ... }
    //
    // 期待: model.desired_open == false (stale observer に override されない)
    todo!("Step 4 で実装");
}

// ── シナリオ 3: Ctrl+変換 IME-ON が直後に解除されない ─────────

#[test]
#[ignore = "Step 4 で ChordStarted/Ended Barrier を実装後に有効化"]
fn scenario_3_ctrl_henkan_does_not_deactivate_immediately() {
    todo!("Step 4 で実装");
}

// ── シナリオ 4: Chrome/Edge no-imm32 で IME OFF ──────────────

#[test]
fn scenario_4_chrome_no_imm32_ime_off_works() {
    let model = run_reducer(vec![
        focus_changed(AppImeProfile::Imm32Unavailable),
        user_intent(false, IntentSource::Command), // Ctrl+無変換 由来の SetOpenRequest
    ]);
    assert!(!model.desired_open, "Chrome でも IME OFF intent が効く");
    // AppImePolicy が Imm32Unavailable に切り替わっていることを確認
    assert!(
        !matches!(
            model.app_policy.actuator_kind,
            awase_windows::state::app_ime_policy::ImeActuatorKind::ImmCross
                | awase_windows::state::app_ime_policy::ImeActuatorKind::Standard
                | awase_windows::state::app_ime_policy::ImeActuatorKind::TsfNative
        ),
        "Chrome は Imm32Unavailable profile"
    );
}

// ── シナリオ 5: WezTerm TSF cold start ─────────────────────────

#[test]
fn scenario_5_wezterm_tsf_profile_policy() {
    let model = run_reducer(vec![focus_changed(AppImeProfile::TsfNative)]);
    assert!(
        !model.app_policy.owns_physical_kanji,
        "WezTerm では物理 KANJI を awase 所有しない (TSF が処理)"
    );
}

// ── シナリオ 6: Focus change 直後の stale false が desired を override しない ─

#[test]
fn scenario_6_focus_change_stale_false_does_not_override_desired() {
    // 1. ユーザーが IME ON にした
    // 2. フォーカス変更
    // 3. 直後に observer が false (stale) を返す
    // → desired_open は true のままであるべき
    let model = run_reducer(vec![
        user_intent(true, IntentSource::PhysicalImeKey),
        focus_changed(AppImeProfile::Standard),
        observer_reported(false, ObservationSource::ObserverPoll),
    ]);
    // 絶対ルール: observer は desired を変えない
    // ただしフォーカス変更で last_intent は clear されるため、
    // desired_open は前のままでも last_intent が None になる
    assert!(
        model.desired_open,
        "観測が desired を上書きしない (絶対ルール)"
    );
}

// ── シナリオ 7: panic_reset 直後の stale poll が state を壊さない ──

#[test]
fn scenario_7_panic_reset_then_stale_poll_does_not_corrupt() {
    // panic_reset は ImeModel level では「Recovery intent で desired_open=true」+
    // observation clear で表現される。
    // その直後の stale false poll が desired を壊さないことを確認。
    let model = run_reducer(vec![
        user_intent(true, IntentSource::Recovery),
        observer_reported(false, ObservationSource::ObserverPoll),
    ]);
    assert!(model.desired_open, "Recovery intent 後の stale が壊さない");
    assert!(model.observations.drift.is_some(), "drift は記録される");
}

// ── シナリオ 8: stale async apply が newer intent を壊さない ─────

#[test]
#[ignore = "Step 7 で ApplyRequested/Succeeded を実装後に有効化"]
fn scenario_8_stale_async_apply_does_not_corrupt_newer_intent() {
    // 期待 sequence (Step 7 実装後):
    // T1: apply true requested generation=10
    // T2: user intent false generation=11
    // T3: apply true succeeded generation=10 ← stale
    //
    // 期待:
    // - model.desired_open == false (generation=11 の intent が勝つ)
    // - model.applied_open == None (gen=10 success は無視)
    todo!("Step 7 で実装");
}

// ── 追加: ドリフト追跡の動作確認 ──────────────────────────────

#[test]
fn drift_tracking_reflects_intent_observer_mismatch() {
    let model = run_reducer(vec![
        user_intent(true, IntentSource::PhysicalImeKey),
        observer_reported(false, ObservationSource::ObserverPoll),
    ]);
    let drift = model.observations.drift.expect("drift が記録される");
    assert_eq!(drift.desired, true);
    assert_eq!(drift.observed, false);
}

#[test]
fn drift_cleared_when_observation_agrees_with_desired() {
    let model = run_reducer(vec![
        user_intent(true, IntentSource::PhysicalImeKey),
        observer_reported(false, ObservationSource::ObserverPoll),
        observer_reported(true, ObservationSource::ObserverPoll), // 一致
    ]);
    assert!(
        model.observations.drift.is_none(),
        "observation が desired と一致したら drift clear"
    );
}

// ── 追加: AppImePolicy の切り替え ─────────────────────────────

#[test]
fn focus_change_updates_app_policy() {
    let model = run_reducer(vec![
        focus_changed(AppImeProfile::Imm32Unavailable),
        focus_changed(AppImeProfile::TsfNative),
    ]);
    assert!(
        !model.app_policy.owns_physical_kanji,
        "TsfNative では owns_kanji=false"
    );
}

#[test]
fn focus_change_clears_intent_and_observations() {
    let model = run_reducer(vec![
        user_intent(false, IntentSource::SyncKey),
        observer_reported(true, ObservationSource::Gji),
        focus_changed(AppImeProfile::Standard),
    ]);
    assert!(model.last_intent.is_none(), "intent は focus 変更で clear");
    assert!(
        model.observations.per_source.gji.is_none(),
        "observation も focus 変更で clear"
    );
}
