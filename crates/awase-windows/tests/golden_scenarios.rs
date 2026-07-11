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
//! 9. HwndCacheRestored は desired_open を復元するが観測で上書きされる
//! 10. HwndCacheRestored と UserImeSetIntent の動作の違い
//! 11. Edge stale-ObservedEisu からの物理かなキー回復 (UserImeOnEisuReset)
//! 12. GJI I/O 観測 (GjiIoInference) が ObservedEisu を自己回復する
//! 13. hwnd キャッシュ復元は stale ObservedEisu をそのまま再注入しない (cache_restore_eisu_guard)
//! 14. IME が既に open のまま TurnOn キーを受けても ObservedEisu から回復する (UserTurnOnEisuReset)
//! 15. 左Shift単独タップによる「IME-ON 半角英数」持続トグル: ObservedEisu へ切替えても
//!     IME は open のまま維持され (SetOpen を経由しない)、トグルOFF で romaji-capable に
//!     復帰する (UserHalfWidthAlnumToggle)
//!
//! ## 実装状況
//!
//! 現在の shadow reducer (Step 3 まで) で検証可能なものは assert する。
//! ChordEnded (Step 4), ApplyRequested/Succeeded (Step 7), Barrier
//! 機構 (Step 5) が必要なものは `#[ignore]` で skip する。

use std::time::Instant;

use awase_windows::state::ime_event::{
    ChordKind, EventTime, HwndId, ImeEvent, ImeEventEnvelope, ImePolicyProfile,
    ObservationConfidence, ObservationSource, UserIntentSource,
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
    // Ctrl+無変換 IME OFF → chord 中の stale observer は desired を壊さない。
    // chord barrier は production と同じく ImeApplyRequested
    // { target:false, ctrl_held:true } が立てる（旧 ChordStarted は 2026-07-06
    // 到達不能パス監査 B2 で撤去 — production dispatch サイトが無かった）。
    let model = run_reducer(vec![
        user_intent(false, UserIntentSource::SyncKey),
        ImeEvent::ImeApplyRequested {
            target: false,
            generation: 1,
            ctrl_held: true,
        },
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
    // Ctrl+変換 IME ON: production では ON 側 chord barrier は張らず、
    // ImeApplyRequested { target:true } は既存 chord をむしろ即時解除する
    // （Ctrl 離さず 無変換→変換 を通すための設計）。stale observer への耐性は
    // barrier ではなく intent + confidence ルールが担うことをこのシナリオで固定する。
    let model = run_reducer(vec![
        // 前提: IME OFF 状態
        user_intent(false, UserIntentSource::PhysicalImeKey),
        // Ctrl+変換 IME ON
        user_intent(true, UserIntentSource::SyncKey),
        ImeEvent::ImeApplyRequested {
            target: true,
            generation: 1,
            ctrl_held: true,
        },
        observer_reported(false, ObservationSource::ObserverPoll), // stale
    ]);
    assert!(model.desired_open(), "Ctrl+変換 IME ON が維持される");
    assert!(
        model.input_barrier.is_none(),
        "IME ON 要求は chord barrier を張らない"
    );
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
    assert!(
        model.desired_open(),
        "PanicReset 後の stale poll が desired を壊さない"
    );
    assert!(
        model.last_intent.is_none(),
        "PanicReset は last_intent を設定しない"
    );
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
        model.applied.applied_open(),
        None,
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

// ── シナリオ 11: Edge stale-ObservedEisu からの物理かなキー回復 ──────────────

// 2026-07-06 MS Edge (Imm32Unavailable/Blacklist) で実発生した循環デッドロックの固定:
// belief が ObservedEisu に固着すると engine が NotRomajiInput で inactive になり、
// activation 側の救済 (PostSetOpenEisuReset) は Decision 経由 SetOpen(true) 限定のため
// 発火できない。物理かなキーによる shadow toggle OFF→ON には UserImeOnEisuReset が
// 対で配線され、「IME ON + romaji-capable」まで回復することをモデル層で保証する。
// 経路×救済の対応表は src/state/eisu_recovery.rs の module doc を参照。
#[test]
fn scenario_11_edge_stale_eisu_recovers_via_physical_ime_key() {
    use awase::engine::{AssumedReason, InputModeState};
    use awase_windows::state::ime_event::{InputModeApplyResult, InputModeApplyStrategy};
    use awase_windows::state::TickMs;

    // 1. Edge へフォーカス + キャッシュ復元で stale な ObservedEisu を引き継ぐ
    let deadlocked = run_reducer(vec![
        focus_changed(ImePolicyProfile::Imm32Unavailable),
        ImeEvent::InputModeApplied {
            mode: InputModeState::ObservedEisu,
            strategy: InputModeApplyStrategy::CacheRestore,
            result: InputModeApplyResult::Applied,
            at: TickMs(0),
        },
        // 2. ユーザーが物理かなキーで IME ON
        user_intent(true, UserIntentSource::PhysicalImeKey),
    ]);
    // ここまでが「詰み」状態の再現: IME ON でも input_mode が Eisu のままだと
    // engine は NotRomajiInput で活性化できない
    assert!(deadlocked.effective_open(), "物理キーで IME ON にはなる");
    assert!(
        !deadlocked.input_mode().is_romaji_capable(),
        "救済なしでは ObservedEisu が残り engine が活性化できない (バグの再現)"
    );

    // 3. shadow toggle OFF→ON に配線された UserImeOnEisuReset が発火
    //    (判定: state/eisu_recovery::eisu_reset_on_ime_on)
    let mut events = vec![
        focus_changed(ImePolicyProfile::Imm32Unavailable),
        ImeEvent::InputModeApplied {
            mode: InputModeState::ObservedEisu,
            strategy: InputModeApplyStrategy::CacheRestore,
            result: InputModeApplyResult::Applied,
            at: TickMs(0),
        },
        user_intent(true, UserIntentSource::PhysicalImeKey),
    ];
    events.push(ImeEvent::InputModeApplied {
        mode: InputModeState::AssumedRomaji {
            reason: AssumedReason::AppKindExcluded,
        },
        strategy: InputModeApplyStrategy::UserImeOnEisuReset,
        result: InputModeApplyResult::Applied,
        at: TickMs(0),
    });
    let recovered = run_reducer(events);
    assert!(recovered.effective_open(), "IME ON が維持される");
    assert!(
        recovered.input_mode().is_romaji_capable(),
        "UserImeOnEisuReset で romaji-capable に回復し engine が活性化できる"
    );
}

// ── シナリオ 12: GJI I/O 観測 (GjiIoInference) が ObservedEisu を自己回復する ──

// Blacklist アプリでは IMM query がスキップされ idle-conv-check も TsfNative 限定のため、
// stale ObservedEisu を訂正する観測経路がない。フォーカス後の GJI 変換 I/O は
// 「英数モードではない」ことの真正の外部証拠であり、Medium confidence の
// InputModeObserved として belief を訂正できることを固定する
// (判定: state/eisu_recovery::gji_io_eisu_correction)。
#[test]
fn scenario_12_gji_io_inference_corrects_stale_eisu() {
    use awase::engine::{AssumedReason, InputModeState};
    use awase_windows::state::ime_event::{InputModeApplyResult, InputModeApplyStrategy};
    use awase_windows::state::TickMs;

    let model = run_reducer(vec![
        focus_changed(ImePolicyProfile::Imm32Unavailable),
        ImeEvent::InputModeApplied {
            mode: InputModeState::ObservedEisu,
            strategy: InputModeApplyStrategy::CacheRestore,
            result: InputModeApplyResult::Applied,
            at: TickMs(0),
        },
        ImeEvent::InputModeObserved {
            mode: InputModeState::AssumedRomaji {
                reason: AssumedReason::ImmBridgeBroken,
            },
            source: ObservationSource::GjiIoInference,
            confidence: ObservationConfidence::Medium,
            at: TickMs(0),
        },
    ]);
    assert!(
        model.input_mode().is_romaji_capable(),
        "Medium confidence の GjiIoInference が ObservedEisu を訂正する"
    );
}

// ── シナリオ 13: hwnd キャッシュ復元は stale ObservedEisu をそのまま再注入しない ──

// 2026-07-09 MS Edge で実発生: Uwp⇔TsfNative フォーカス往復のたびに、最大 1 時間前の
// ObservedEisu キャッシュスナップショットが apply_hwnd_cache_restore で無条件に
// 再注入され、eisu guard (correction_for_imm_broken は ObservedEisu を意図的に対象外)
// に阻まれて engine が inactive のまま固着し続けた。cache_restore_eisu_guard が
// キャッシュ経路専用にこの ObservedEisu を AssumedRomaji へ倒すことを固定する。
#[test]
fn scenario_13_hwnd_cache_restore_does_not_reinject_stale_eisu() {
    use awase::engine::InputModeState;
    use awase_windows::state::eisu_recovery::cache_restore_eisu_guard;
    use awase_windows::state::ime_event::{InputModeApplyResult, InputModeApplyStrategy};
    use awase_windows::state::TickMs;

    // 修正前の生キャッシュ値をそのまま適用した場合の再現（バグの固定）
    let unguarded = run_reducer(vec![
        focus_changed(ImePolicyProfile::Imm32Unavailable),
        ImeEvent::InputModeApplied {
            mode: InputModeState::ObservedEisu,
            strategy: InputModeApplyStrategy::CacheRestore,
            result: InputModeApplyResult::Applied,
            at: TickMs(0),
        },
    ]);
    assert!(
        !unguarded.input_mode().is_romaji_capable(),
        "無条件のキャッシュ復元は ObservedEisu を再注入し engine を詰ませる (バグの再現)"
    );

    // apply_hwnd_cache_restore が実際に使う cache_restore_eisu_guard を経由した場合
    let guarded_mode = cache_restore_eisu_guard(InputModeState::ObservedEisu);
    let guarded = run_reducer(vec![
        focus_changed(ImePolicyProfile::Imm32Unavailable),
        ImeEvent::InputModeApplied {
            mode: guarded_mode,
            strategy: InputModeApplyStrategy::CacheRestore,
            result: InputModeApplyResult::Applied,
            at: TickMs(0),
        },
    ]);
    assert!(
        guarded.input_mode().is_romaji_capable(),
        "cache_restore_eisu_guard が stale ObservedEisu を AssumedRomaji に倒し、\
         engine が activation 可能な状態を維持する"
    );
}

// ── シナリオ 14: IME が既に open のまま TurnOn キーを受けても ObservedEisu から回復する ──

// eisu_reset_on_ime_on は OFF→ON 遷移でのみ発火するため、IME が既に open な状態で
// ユーザーが TurnOn 系キー（ひらがな/かな 等）を押しても遷移が起きず救済されない
// （2026-07-09 MS-IME/Edge で実発生）。UserTurnOnEisuReset がこのケースを別途救済する。
#[test]
fn scenario_14_turn_on_while_open_recovers_stale_eisu() {
    use awase::engine::{AssumedReason, InputModeState};
    use awase_windows::state::eisu_recovery::eisu_reset_on_turn_on_while_open;
    use awase_windows::state::ime_event::{InputModeApplyResult, InputModeApplyStrategy};
    use awase_windows::state::TickMs;

    // IME は open のまま (物理キーで既に ON 済み)、conv だけが Eisu に固着
    let deadlocked = run_reducer(vec![
        focus_changed(ImePolicyProfile::Imm32Unavailable),
        user_intent(true, UserIntentSource::PhysicalImeKey),
        ImeEvent::InputModeObserved {
            mode: InputModeState::ObservedEisu,
            source: ObservationSource::ObserverPoll,
            confidence: ObservationConfidence::Medium,
            at: TickMs(0),
        },
    ]);
    assert!(deadlocked.effective_open(), "IME は open のまま");
    assert!(
        !deadlocked.input_mode().is_romaji_capable(),
        "OFF→ON 遷移を伴わないため UserImeOnEisuReset は発火せず ObservedEisu が残る \
         (バグの再現)"
    );

    // TurnOn キー (ひらがな等) 受信 → eisu_reset_on_turn_on_while_open が発火
    let new_mode = eisu_reset_on_turn_on_while_open(true, deadlocked.input_mode())
        .expect("ObservedEisu かつ TurnOn action なら救済値が返る");
    assert_eq!(
        new_mode,
        InputModeState::AssumedRomaji {
            reason: AssumedReason::AppKindExcluded
        }
    );

    let mut events = vec![
        focus_changed(ImePolicyProfile::Imm32Unavailable),
        user_intent(true, UserIntentSource::PhysicalImeKey),
        ImeEvent::InputModeObserved {
            mode: InputModeState::ObservedEisu,
            source: ObservationSource::ObserverPoll,
            confidence: ObservationConfidence::Medium,
            at: TickMs(0),
        },
    ];
    events.push(ImeEvent::InputModeApplied {
        mode: new_mode,
        strategy: InputModeApplyStrategy::UserTurnOnEisuReset,
        result: InputModeApplyResult::Applied,
        at: TickMs(0),
    });
    let recovered = run_reducer(events);
    assert!(recovered.effective_open(), "IME ON が維持される");
    assert!(
        recovered.input_mode().is_romaji_capable(),
        "UserTurnOnEisuReset で romaji-capable に回復し engine が活性化できる"
    );
}

/// シナリオ15: 左Shift単独タップによる「IME-ON 半角英数」持続トグル
/// (`kp_stage_shift_conv_guard` の belief 側の核心部分)。
///
/// `kp_shift_conv_guard_key_up` が単独タップを検知すると `InputModeApplied
/// { mode: ObservedEisu, strategy: UserHalfWidthAlnumToggle }` を dispatch する。
/// この belief 変更は `SetOpen` effect を一切経由しないため、`Engine::compute_state`
/// は `Inactive(NotRomajiInput)` で engine を素通りモードにしつつ、`effective_open()`
/// は true のまま維持される（`transition_activation` の `suppress_set_open` 分岐、
/// `src/engine/engine.rs:98-112,165-188`）。もう一度タップすると
/// `kp_restore_kana_from_half_width` が `AssumedRomaji { UserHalfWidthAlnumToggleOff }`
/// へ戻し、romaji-capable に復帰する。
#[test]
fn scenario_15_half_width_alnum_toggle_keeps_ime_open_while_engine_goes_inactive() {
    use awase::engine::{AssumedReason, InputModeState};
    use awase_windows::state::ime_event::{InputModeApplyResult, InputModeApplyStrategy};
    use awase_windows::state::TickMs;

    // 前提: IME は既に ON でローマ字入力可能（通常のかな入力中）。
    let toggled_on = run_reducer(vec![
        focus_changed(ImePolicyProfile::Imm32Unavailable),
        user_intent(true, UserIntentSource::PhysicalImeKey),
        ImeEvent::InputModeApplied {
            mode: InputModeState::ObservedEisu,
            strategy: InputModeApplyStrategy::UserHalfWidthAlnumToggle,
            result: InputModeApplyResult::Applied,
            at: TickMs(0),
        },
    ]);
    assert!(
        toggled_on.effective_open(),
        "半角英数トグルON は SetOpen を経由しないため IME は open のまま維持される"
    );
    assert!(
        !toggled_on.input_mode().is_romaji_capable(),
        "ObservedEisu へ切り替わり engine は NotRomajiInput で inactive になる"
    );

    // もう一度タップ（トグルOFF）。
    let toggled_off = run_reducer(vec![
        focus_changed(ImePolicyProfile::Imm32Unavailable),
        user_intent(true, UserIntentSource::PhysicalImeKey),
        ImeEvent::InputModeApplied {
            mode: InputModeState::ObservedEisu,
            strategy: InputModeApplyStrategy::UserHalfWidthAlnumToggle,
            result: InputModeApplyResult::Applied,
            at: TickMs(0),
        },
        ImeEvent::InputModeApplied {
            mode: InputModeState::AssumedRomaji {
                reason: AssumedReason::UserHalfWidthAlnumToggleOff,
            },
            strategy: InputModeApplyStrategy::UserHalfWidthAlnumToggle,
            result: InputModeApplyResult::Applied,
            at: TickMs(10),
        },
    ]);
    assert!(toggled_off.effective_open(), "IME ON は一貫して維持される");
    assert!(
        toggled_off.input_mode().is_romaji_capable(),
        "トグルOFF で romaji-capable に復帰し engine が再度活性化できる"
    );
}
