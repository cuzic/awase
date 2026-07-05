//! Golden Scenario テスト
//!
//! 過去のバグ修正で対処した既知シナリオを event sequence として固定し、
//! 新モデル (`ImeModel`) のリファクタリングで挙動が壊れないようにする。
//!
//! ## シナリオ一覧
//!
//! 1. LINE/Qt KANJI ON/OFF
//! 2. Ctrl+無変換 直後の Ctrl KeyUp で再アクティベートしない (b7d4cdb 防波堤)
//! 3. Ctrl+変換 IME ON が直後に解除されない (6fce373 防波堤)
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

use awase_windows::state::ime_event::{
    ChordKind, EventTime, HwndId, ImeEvent, ImeEventEnvelope, ImePolicyProfile, UserIntentSource,
    ObservationConfidence, ObservationSource,
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

fn user_intent(target: bool, source: UserIntentSource) -> ImeEvent {
    ImeEvent::UserImeSetIntent { target, source }
}

fn observer_reported(open: bool, source: ObservationSource) -> ImeEvent {
    ImeEvent::ObserverReported {
        open,
        source,
        hwnd: HwndId::NULL,
        confidence: ObservationConfidence::Medium,
        focus_epoch: 0,
    }
}

fn focus_changed(profile: ImePolicyProfile) -> ImeEvent {
    ImeEvent::FocusChanged {
        from: None,
        to: HwndId(0x1234),
        profile,
        focus_epoch: 1,
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
        focus_changed(ImePolicyProfile::ImmCross),
        user_intent(false, UserIntentSource::PhysicalImeKey),
    ]);
    assert!(!model.desired_open(), "KANJI で IME OFF");

    // 続けて KANJI 押下 → IME ON
    let model = run_reducer(vec![
        focus_changed(ImePolicyProfile::ImmCross),
        user_intent(false, UserIntentSource::PhysicalImeKey),
        user_intent(true, UserIntentSource::PhysicalImeKey),
    ]);
    assert!(model.desired_open(), "もう一度 KANJI で IME ON");
}

// ── シナリオ 2: Ctrl+無変換 直後の Ctrl KeyUp で再アクティベートしない ─

#[test]
fn scenario_2_ctrl_muhenkan_does_not_reactivate_on_ctrl_keyup() {
    // Ctrl+無変換 IME OFF → chord 中の stale observer は desired を壊さない
    let model = run_reducer(vec![
        ImeEvent::ChordStarted {
            kind: ChordKind::CtrlMuhenkanImeOff,
        },
        user_intent(false, UserIntentSource::SyncKey),
        // chord 中の stale observer (Ctrl KeyUp 由来等)
        observer_reported(true, ObservationSource::ObserverPoll),
        ImeEvent::ChordEnded {
            kind: ChordKind::CtrlMuhenkanImeOff,
        },
    ]);
    assert!(!model.desired_open(), "Ctrl+無変換 IME OFF が維持される");
    assert!(
        model.input_barrier.is_none(),
        "ChordEnded で barrier が clear"
    );
}

// ── シナリオ 3: Ctrl+変換 IME ON が直後に解除されない ─────────

#[test]
fn scenario_3_ctrl_henkan_does_not_deactivate_immediately() {
    let model = run_reducer(vec![
        // 前提: IME OFF 状態
        user_intent(false, UserIntentSource::PhysicalImeKey),
        // Ctrl+変換 IME ON
        ImeEvent::ChordStarted {
            kind: ChordKind::CtrlHenkanImeOn,
        },
        user_intent(true, UserIntentSource::SyncKey),
        observer_reported(false, ObservationSource::ObserverPoll), // stale
        ImeEvent::ChordEnded {
            kind: ChordKind::CtrlHenkanImeOn,
        },
    ]);
    assert!(model.desired_open(), "Ctrl+変換 IME ON が維持される");
}

// ── シナリオ 4: Chrome/Edge no-imm32 で IME OFF ──────────────

#[test]
fn scenario_4_chrome_no_imm32_ime_off_works() {
    let model = run_reducer(vec![
        focus_changed(ImePolicyProfile::Imm32Unavailable),
        user_intent(false, UserIntentSource::Command), // Ctrl+無変換 由来の SetOpenRequest
    ]);
    assert!(!model.desired_open(), "Chrome でも IME OFF intent が効く");
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
    let model = run_reducer(vec![focus_changed(ImePolicyProfile::TsfNative)]);
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
        user_intent(true, UserIntentSource::PhysicalImeKey),
        focus_changed(ImePolicyProfile::ImmCross),
        observer_reported(false, ObservationSource::ObserverPoll),
    ]);
    // 絶対ルール: observer は desired を変えない
    // ただしフォーカス変更で last_intent は clear されるため、
    // desired_open は前のままでも last_intent が None になる
    assert!(
        model.desired_open(),
        "観測が desired を上書きしない (絶対ルール)"
    );
}

// ── シナリオ 7: panic_reset 直後の stale poll が state を壊さない ──

#[test]
fn scenario_7_panic_reset_then_stale_poll_does_not_corrupt() {
    // panic_reset は ImeModel level では PanicReset イベント（desired_open=true に戻す、
    // last_intent は設定しない）で表現される。
    // その直後の stale false poll が desired を壊さないことを確認。
    let model = run_reducer(vec![
        ImeEvent::PanicReset { target: true },
        observer_reported(false, ObservationSource::ObserverPoll),
    ]);
    assert!(model.desired_open(), "PanicReset 後の stale poll が desired を壊さない");
    assert!(model.last_intent.is_none(), "PanicReset は last_intent を設定しない");
    assert!(model.observations.drift.is_some(), "drift は記録される");
}

// ── シナリオ 8: stale async apply が newer intent を壊さない (最重要) ─────

#[test]
fn scenario_8_stale_async_apply_does_not_corrupt_newer_intent() {
    // T1: apply true requested generation=10
    // T2: user intent false (gen は user side では別管理)
    // T3: apply true succeeded generation=10 ← stale (newer intent 後の old apply 完了)
    //
    // 期待:
    // - desired_open == false (T2 の intent が勝つ)
    // - applied_open == None (gen=10 の success は無視)
    let model = run_reducer(vec![
        ImeEvent::ImeApplyRequested {
            target: true,
            generation: 10,
            ctrl_held: false,
        },
        user_intent(false, UserIntentSource::PhysicalImeKey),
        // 新しい intent で別の apply が発生 (gen=11) して pending を上書きする想定
        ImeEvent::ImeApplyRequested {
            target: false,
            generation: 11,
            ctrl_held: false,
        },
        // 古い gen=10 の success が遅れて到着
        ImeEvent::ImeApplySucceeded {
            target: true,
            generation: 10,
        },
    ]);
    assert_eq!(model.desired_open(), false, "newer intent (gen=11) が勝つ");
    assert_eq!(
        model.applied.applied_open(), None,
        "stale gen=10 の success は無視 (generation 照合)"
    );
    assert!(
        model.pending.is_some(),
        "gen=11 の pending はまだ生きている"
    );
}

#[test]
fn apply_succeeded_with_matching_generation_updates_applied() {
    // 正常系: 同 generation の success で applied_open がセットされる
    let model = run_reducer(vec![
        ImeEvent::ImeApplyRequested {
            target: true,
            generation: 5,
            ctrl_held: false,
        },
        ImeEvent::ImeApplySucceeded {
            target: true,
            generation: 5,
        },
    ]);
    assert_eq!(model.applied.applied_open(), Some(true));
    assert!(model.pending.is_none(), "成功後 pending clear");
}

// ── 追加: ドリフト追跡の動作確認 ──────────────────────────────

#[test]
fn drift_tracking_reflects_intent_observer_mismatch() {
    let model = run_reducer(vec![
        user_intent(true, UserIntentSource::PhysicalImeKey),
        observer_reported(false, ObservationSource::ObserverPoll),
    ]);
    assert!(model.observations.drift.is_some(), "drift が記録される");
    assert_eq!(model.desired_open(), true, "desired は intent の true");
    assert_eq!(
        model.observations.per_source.observer_poll.map(|o| o.open),
        Some(false),
        "observer は false を報告"
    );
}

#[test]
fn drift_cleared_when_observation_agrees_with_desired() {
    let model = run_reducer(vec![
        user_intent(true, UserIntentSource::PhysicalImeKey),
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
        focus_changed(ImePolicyProfile::Imm32Unavailable),
        focus_changed(ImePolicyProfile::TsfNative),
    ]);
    assert!(
        !model.app_policy.owns_physical_kanji,
        "TsfNative では owns_kanji=false"
    );
}

#[test]
fn focus_change_clears_intent_and_observations() {
    let model = run_reducer(vec![
        user_intent(false, UserIntentSource::SyncKey),
        observer_reported(true, ObservationSource::Gji),
        focus_changed(ImePolicyProfile::ImmCross),
    ]);
    assert!(model.last_intent.is_none(), "intent は focus 変更で clear");
    assert!(
        model.observations.per_source.gji.is_none(),
        "observation も focus 変更で clear"
    );
}

// ── シナリオ 9: HwndCacheRestored は desired_open を復元するが観測で上書きされる ─

// HwndCacheRestored はユーザー意図ではないため has_user_explicit_intent() = false のまま。
// 後続の実観測が effective_open() を上書きできることを確認する。
// （UserImeSetIntent との対比が設計の核心: キャッシュ復元 vs 能動的意図）
#[test]
fn scenario_9_hwnd_cache_restored_can_be_overridden_by_observation() {
    // フォーカス変更でキャッシュから desired_open=false を復元
    let model = run_reducer(vec![
        ImeEvent::HwndCacheRestored { target: false },
        // 実際の API 観測が IME ON を返す（実 IME 状態はキャッシュと異なる）
        ImeEvent::ObserverReported {
            open: true,
            source: ObservationSource::ImmGetOpenStatus,
            hwnd: HwndId::NULL,
            confidence: ObservationConfidence::High,
            focus_epoch: 0,
        },
    ]);
    assert!(
        !model.desired_open(),
        "desired_open はキャッシュの復元値 (false) のまま変わらない"
    );
    assert!(
        model.last_intent.is_none(),
        "HwndCacheRestored は last_intent を設定しない"
    );
    assert!(
        model.effective_open(),
        "High 観測が effective_open を上書きする（has_user_explicit_intent=false のため）"
    );
}

// ── シナリオ 10: HwndCacheRestored と UserImeSetIntent の動作の違い ──────────

// UserImeSetIntent は last_intent を設定するため、
// 後続の観測があっても effective_open() は desired_open を優先する。
// HwndCacheRestored は last_intent を設定しないため、観測が優先される。
// この違いが「キャッシュ復元はユーザーの能動的操作ではない」という設計の証明。
#[test]
fn scenario_10_user_intent_blocks_observation_but_hwnd_cache_does_not() {
    let stale_observation = ImeEvent::ObserverReported {
        open: true,
        source: ObservationSource::ObserverPoll,
        hwnd: HwndId::NULL,
        confidence: ObservationConfidence::Medium,
        focus_epoch: 0,
    };

    // UserImeSetIntent: ユーザーが IME OFF を明示した → 観測で上書きされない
    let model_intent = run_reducer(vec![
        user_intent(false, UserIntentSource::SyncKey),
        stale_observation.clone(),
    ]);
    assert!(
        !model_intent.effective_open(),
        "UserImeSetIntent(false) 後は explicit intent があるため、Medium 観測は effective_open を変えない"
    );

    // HwndCacheRestored: キャッシュから IME OFF を復元した → 観測で上書きされる
    let model_cache = run_reducer(vec![
        ImeEvent::HwndCacheRestored { target: false },
        stale_observation,
    ]);
    assert!(
        model_cache.effective_open(),
        "HwndCacheRestored(false) 後は explicit intent がないため、Medium 観測が effective_open を上書きする"
    );
}
