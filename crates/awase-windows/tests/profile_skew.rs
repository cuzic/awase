#![allow(clippy::all, clippy::pedantic, clippy::nursery)]
//! DD3-01 の再現: 殻の live `app_profile` と核の `ImeModel.app_policy` が、同一プロセス内の
//! クラス変化（`FocusChanged` を送らない経路）でずれ、drift correction が両方を並べて読むと
//! caps の表に無い組み合わせ（StrategyChain × `Read`＝回数で打ち切らない）になる。
//!
//! 殻の `ir_observe_drift_facts`（`runtime/ime_refresh.rs`）が作る `DriftFacts` を、同じ式で純粋に組む:
//!   `default_policy` = `model.app_policy.default_feedback`（核・`FocusChanged`/起動時にしか更新されない）
//!   `imm_cross`      = `live_profile.can_use_imm32_cross_process()`（殻・クラスが変わるたびに更新される）
//! Linux で動く（`#[cfg(windows)]` の外）。belief の更新経路には触れない（テストのみ）。

use std::time::{Duration, Instant};

use awase_windows::focus::class_names::AppImeProfile;
use awase_windows::state::app_ime_policy::{caps, AppImePolicy};
use awase_windows::state::drift_correction::DriftCorrection;
use awase_windows::state::drift_plan::{
    decide_drift_plan, ActuationSnapshot, DriftFacts, DriftPlan, DriftStep, GiveUpPark, SendPath,
};
use awase_windows::state::event_origin::Generation;
use awase_windows::state::hub_clock::HubClock;
use awase_windows::state::ime_actuation::FeedbackPolicy;
use awase_windows::state::ime_event::{
    HwndId, ImeEvent, ImePolicyProfile, ObservationConfidence, ObservationSource,
};
use awase_windows::state::ime_kind::ImeKindId;
use awase_windows::state::platform_state::ImeStateHub;
use awase_windows::state::TickMs;

const TICK_BASE: u64 = 1_000_000;

fn live_profile_for_class(class: &str) -> AppImeProfile {
    AppImeProfile::from_class_name(class)
}

/// 本物の `ImeStateHub` に、窓へ入った時点の profile で `FocusChanged` を流す（プロセスが変わったときの経路）。
fn hub_after_focus_changed(live_at_entry: AppImeProfile) -> ImeStateHub {
    let mut hub = ImeStateHub::with_clock(
        HubClock::manual(TICK_BASE, Instant::now()),
        quanta::Clock::new(),
    );
    hub.dispatch_event(
        ImeEvent::FocusChanged {
            from: Some(HwndId(0x1000)),
            to: HwndId(0x1001),
            profile: ImePolicyProfile::from(live_at_entry),
            focus_epoch: 2,
        },
        TickMs(TICK_BASE),
    );
    hub
}

/// 殻の `ir_observe_drift_facts` と同じ式で `DriftFacts` を組む。
fn facts_like_shell(
    hub: &ImeStateHub,
    live: AppImeProfile,
    active: Option<ActuationSnapshot>,
    now: Instant,
) -> DriftFacts {
    DriftFacts {
        now,
        engine_enabled: true,
        japanese_ime: true,
        drift: Ok(DriftCorrection {
            desired: true,
            observed: false,
            duration_ms: 3000,
            source: ObservationSource::ObserverPoll,
            confidence: ObservationConfidence::Medium,
        }),
        settling: false,
        active,
        default_policy: hub.model().app_policy.default_feedback,
        imm_cross: live.can_use_imm32_cross_process(),
        warrant_would_block: false,
        diag_already_notified: false,
        converged: false,
        fresh_evidence_after_giveup: false,
    }
}

#[test]
fn same_process_class_change_leaves_policy_read_while_live_profile_is_unreadable() {
    // 1. プロセス P の窓 A（Explorer 本体、Standard）に入る → FocusChanged(ImmCross)。
    let at_entry = live_profile_for_class("CabinetWClass");
    assert_eq!(at_entry, AppImeProfile::Standard);
    let hub = hub_after_focus_changed(at_entry);
    assert!(matches!(
        hub.model().app_policy.default_feedback,
        FeedbackPolicy::Read { .. }
    ));

    // 2. 同じ P の窓 B（XAML ホスト）へクラスが変わる。pid が同じなので FocusChanged は来ない
    //    （`advance_focus_tracking` の process_changed は pid 比較）。殻の live profile だけが変わる。
    let live_now = live_profile_for_class("XamlExplorerHostIslandWindow");
    assert!(!live_now.can_use_imm32_cross_process());
    let caps_for_live = caps(ImePolicyProfile::from(live_now), ImeKindId::Gji);
    assert!(matches!(
        caps_for_live.feedback,
        FeedbackPolicy::Blind { .. }
    ));

    // 核の policy は Read のまま（殻の live profile が表す Blind と食い違う）。
    assert_ne!(
        hub.model().app_policy,
        AppImePolicy::from_profile(ImePolicyProfile::from(live_now)),
        "DD3-01: app_policy は同一プロセス内のクラス変化に追随しない"
    );
}

#[test]
fn drift_plan_resends_without_cap_in_the_skewed_combination() {
    let now = Instant::now();
    let live = live_profile_for_class("XamlExplorerHostIslandWindow");

    // 既に 50 回送った試行（BUG-43 型の無限再送が起きていれば到達する回数）。
    let mk_active = |policy: FeedbackPolicy| ActuationSnapshot {
        target: true,
        policy,
        attempts: 50,
        sent_at: now - Duration::from_secs(5),
        gave_up_at: None,
        origin: policy.origin(Generation::INITIAL),
    };

    // ずれた組み合わせ: Explorer 本体で入場 → XAML へクラス変化。
    let skewed_hub = hub_after_focus_changed(live_profile_for_class("CabinetWClass"));
    let p = skewed_hub.model().app_policy.default_feedback;
    let skewed = decide_drift_plan(&facts_like_shell(
        &skewed_hub,
        live,
        Some(mk_active(p)),
        now,
    ));
    let DriftPlan::Act(a) = skewed else {
        panic!("Act を期待: {skewed:?}")
    };
    // 現状: StrategyChain で送り続ける（Read は回数で打ち切らない）。
    assert_eq!(a.step, DriftStep::Send(SendPath::StrategyChain));

    // 対照: 入場時から XAML だった場合（policy=Blind）は同じ試行回数で打ち切る。
    let consistent_hub = hub_after_focus_changed(live);
    let p = consistent_hub.model().app_policy.default_feedback;
    let consistent = decide_drift_plan(&facts_like_shell(
        &consistent_hub,
        live,
        Some(mk_active(p)),
        now,
    ));
    let DriftPlan::Act(b) = consistent else {
        panic!("Act を期待: {consistent:?}")
    };
    assert!(
        matches!(b.step, DriftStep::GiveUp(GiveUpPark::FirstTime)),
        "同じ窓・同じ試行回数でも入場時の分類しだいで結果が違う: {:?}",
        b.step
    );
}

// ── DD5-03: フォーカス時の偽の Confirmed が、押下に由来しない ON を省かせる ─────────────────────

use awase::engine::InputModeState;
use awase_windows::state::actuation_chain::WriteMechanism;
use awase_windows::state::applied_mirror_plan::may_mirror_applied_on_focus;
use awase_windows::state::ime_actuation_decision::{decide_attempt, DecisionInputs, DecisionSite};
use awase_windows::state::ime_model::ImeModel;

/// `ir_post_focus_change_snapshot` の写しの判断（`may_mirror_applied_on_focus`）と、その後の
/// GJI 直接送信の already-matched 判断（`decide_attempt`）を、窓のクラスごとに通す。
/// 戻り値: (写しが走ったか, `SetOpen { press: None }` の ON が書かれるか)。
fn focus_then_engine_on(class: &str, profile: AppImeProfile) -> (bool, bool) {
    let tsf_native = profile.is_effectively_tsf_native(class);
    let mut model = ImeModel::new();
    // 窓の中を読んでいない安全既定値 ON（`reset_stale_ime_on_for_imm_broken` の Low の値）が effective_open。
    let effective_open = true;
    let mirrored = may_mirror_applied_on_focus(tsf_native, false);
    if mirrored {
        model.confirm_applied(effective_open, 1_000_000);
    }
    let inputs = DecisionInputs::from_facts(
        profile,
        ImeKindId::Gji,
        model.applied.applied_open(),
        InputModeState::Unknown,
        false,
    );
    let (_, command) = decide_attempt(inputs, DecisionSite::Sync, WriteMechanism::GjiDirect, true);
    (mirrored, command.is_some())
}

#[test]
fn dd5_03_static_imm32_unavailable_window_gets_a_false_confirmed_and_engine_on_is_elided() {
    // Chrome（静的な一覧で Imm32Unavailable。窓の IMM は読んでいない）。
    let chrome = live_profile_for_class("Chrome_WidgetWin_1");
    assert_eq!(chrome, AppImeProfile::Imm32Unavailable);
    let (mirrored, written) = focus_then_engine_on("Chrome_WidgetWin_1", chrome);
    assert!(
        mirrored,
        "静的 Imm32Unavailable の窓へ applied=Confirmed が写される"
    );
    assert!(
        !written,
        "DD5-03: 偽の Confirmed(true) のせいで、押下に由来しない ON（EngineOn コンボ）が already-matched で省かれる"
    );

    // 対照: XAML ホスト・InputSite は is_effectively_tsf_native なので写されず、ON は書かれる。
    for class in [
        "XamlExplorerHostIslandWindow",
        "Windows.UI.Input.InputSite.WindowClass",
    ] {
        let p = live_profile_for_class(class);
        let (mirrored, written) = focus_then_engine_on(class, p);
        assert!(!mirrored, "{class}: 写されない");
        assert!(written, "{class}: ON は書かれる");
    }
}
