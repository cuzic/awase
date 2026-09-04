#![allow(clippy::all, clippy::pedantic, clippy::nursery)]
//! BUG-110/ADR-132 Phase 2（`WarmupImeOn::from_applied_or_belief_unless_off_drift`
//! の `off_drift_active` ゲート）が依存する唯一の性質を、**Linux で実行できる**
//! `ImeModel` + `ObservationStore`（いずれも ungated）だけで固定する。
//!
//! ## なぜ別ファイルなのか
//!
//! ゲートの配線本体（`ImeStateHub::resolve_warmup_ime_on`、
//! `state/platform_state.rs`）は `#[cfg(windows)]` であり、Linux の
//! `cargo test -p awase-windows` では1件もコンパイルされない。ゲートの
//! 判定本体 `check_drift_correction` も同じ理由で Linux から直接は
//! 呼べない。しかし v2 設計が正しく動く根拠は「`ObservationStore` の
//! drift 追跡が `ImeEvent::FocusChanged` の reduce で確実にクリアされる」
//! という、ungated な層だけで完結する事実1点に集約される
//! （`ImeStateHub::check_drift_correction` は
//! `self.shadow_model.observations.drift_duration(now)` が `None` を
//! 返した瞬間に `None` を返し、warmup ゲートは開く）。ここではその
//! 1点を Linux CI で機械的に固定する。
//!
//! ## v1 → v2 の設計変遷（このテストが再発を防ぐ対象）
//!
//! ADR-132 Phase 2 の v1 は `desired_open()`（`ImeModel` の生 belief
//! フィールド）を warmup のゲートに使う案だったが、opus-adversarial-consult
//! の premortem レビューで blocker と判定された: `desired_open` は
//! `ImeEvent::FocusChanged` の reduce アームでクリアされず、フォーカスを
//! 跨いで stale なまま残る（`ime_model.rs:611-640` に `desired_open` への
//! 代入が無いことは本ファイルの `desired_open_survives_focus_change`
//! が固定する）。これを cross-window の warmup 判断に使うと、正当な
//! cold-start ケース（別ウィンドウで実際に IME が開いている）まで
//! 抑止してしまい BUG-02 型のリテラル化を再導入する。
//!
//! v2 はゲートを `desired_open` ではなく `check_drift_correction()` の
//! 戻り値（`ObservationStore` の drift 追跡が focus-scoped）に変更した。
//! `drift_focus_scope_is_cleared_on_focus_changed` がその focus-scoping
//! 性質そのものを固定する。

use std::time::{Duration, Instant};

use awase_windows::state::conv_classify::ConvSyncReason;
use awase_windows::state::evidence::{ConvOpenInference, Observed};
use awase_windows::state::ime_event::{
    EventTime, HwndId, ImeEvent, ImeEventEnvelope, ImePolicyProfile, UserIntentSource,
};
use awase_windows::state::ime_model::ImeModel;

const WINDOW_A: HwndId = HwndId(0xA001);
const WINDOW_B: HwndId = HwndId(0xB002);

fn reduce(model: &mut ImeModel, seq: u64, tick_ms: u64, time: Instant, event: ImeEvent) {
    model.reduce(&ImeEventEnvelope {
        time: EventTime {
            seq,
            monotonic: time,
            tick_ms,
        },
        event,
    });
}

fn focus_changed(to: HwndId, focus_epoch: u64) -> ImeEvent {
    ImeEvent::FocusChanged {
        from: None,
        to,
        profile: ImePolicyProfile::TsfNative,
        focus_epoch,
    }
}

/// 観測（open=true）を1件届ける。`intent_store_effective_open.rs` と同じ
/// 本番構築経路（`Observed::<ConvOpenInference>::from_conv`）を使う——ここでは
/// drift 追跡（`update_drift`）の入力として使うだけで、`ConvOpenInference` の
/// 権威（`BeliefOnly`）自体はこのテストの対象外（`check_drift_correction`の
/// 権威フィルタは Windows-gated 側の別の検証事項）。
fn observed_open_true(focus_epoch: u64) -> ImeEvent {
    ImeEvent::ObserverReported(
        Observed::<ConvOpenInference>::from_conv(
            ConvSyncReason::NativeToggleShadowOff,
            true,
            HwndId::NULL,
            focus_epoch,
        )
        .into(),
    )
}

/// T2 (最重要): `ImeEvent::FocusChanged` は `ObservationStore` の drift 追跡を
/// 必ずクリアする。これが v2 のゲートが focus-scoped であることの根拠そのもの。
///
/// シナリオ: 窓Aで明示 OFF → 矛盾する観測が届き drift が確立 → 窓Bへ
/// フォーカス変更（BUG-110 のシナリオでは「同一窓内で last_intent が
/// FocusChanged 待ちのままロックされる」が、ここではその手前の性質——
/// FocusChanged が実際に起きた**その瞬間**に drift が消えること——を
/// 検証する）。
#[test]
fn drift_focus_scope_is_cleared_on_focus_changed() {
    let mut model = ImeModel::default();
    let t0 = Instant::now();

    reduce(&mut model, 1, 0, t0, focus_changed(WINDOW_A, 1));
    reduce(
        &mut model,
        2,
        100,
        t0,
        ImeEvent::UserImeSetIntent {
            target: false,
            source: UserIntentSource::Command,
        },
    );
    assert!(!model.desired_open(), "明示 OFF 直後は desired_open=false");

    // 矛盾する観測（open=true）が届き、drift が確立する。
    let t1 = t0 + Duration::from_millis(50);
    reduce(&mut model, 3, 150, t1, observed_open_true(1));
    assert!(
        model.observations.drift_duration(t1).is_some(),
        "desired=false と observed=true が矛盾するため drift が確立しているはず"
    );

    // フォーカス変更（新しい窓Bへ）。
    let t2 = t1 + Duration::from_millis(10);
    reduce(&mut model, 4, 160, t2, focus_changed(WINDOW_B, 2));

    assert!(
        model.observations.drift_duration(t2).is_none(),
        "FocusChanged 直後は drift が必ずクリアされていなければならない \
         （v2 のゲート = off_drift_active が、新しいフォーカスでは常に false \
         になり、cross-window の cold-start warmup を抑止しないことの根拠）"
    );
}

/// T3: `desired_open` は `FocusChanged` でクリアされず、フォーカスを跨いで
/// stale なまま残る（v1 が blocker と判定された性質そのものを記録として固定する）。
///
/// このテストが red になった場合、`ImeEvent::FocusChanged` の reduce に
/// `desired_open` のリセットが追加されたということであり、ADR-132 v1 で
/// 却下された「`desired_open` を warmup のゲートに使う」設計を再検討する
/// 余地が生まれたことを意味する（ADR に記録された議論を必ず読んでから
/// 再検討すること）。
#[test]
fn desired_open_survives_focus_change() {
    let mut model = ImeModel::default();
    let t0 = Instant::now();

    reduce(&mut model, 1, 0, t0, focus_changed(WINDOW_A, 1));
    reduce(
        &mut model,
        2,
        100,
        t0,
        ImeEvent::UserImeSetIntent {
            target: false,
            source: UserIntentSource::Command,
        },
    );
    assert!(!model.desired_open());

    reduce(&mut model, 3, 200, t0, focus_changed(WINDOW_B, 2));

    assert!(
        !model.desired_open(),
        "desired_open は FocusChanged でクリアされず、次の明示書き込みまで \
         stale なまま残る（ADR-132 v1 の blocker、ADR-132 v2 が desired_open \
         ではなく drift の focus-scoping を使う理由）"
    );
}

/// Q1 で受け入れたトレードオフ: フォーカス変更直後はゲートが開く
/// （cold-start warmup を守る）が、新しい窓で desired（stale）と食い違う
/// 観測が積み重なると、drift が再確立されゲートが再び閉じる。
///
/// この挙動は「修正」ではなく意図的に受け入れたトレードオフとして
/// ADR-132 に明記されている——次の担当者が「cross-window は無条件に
/// 守られている」と誤読しないよう、ここで機械的に固定する。
#[test]
fn drift_re_establishes_in_new_window_when_stale_desired_conflicts_with_fresh_observation() {
    let mut model = ImeModel::default();
    let t0 = Instant::now();

    reduce(&mut model, 1, 0, t0, focus_changed(WINDOW_A, 1));
    reduce(
        &mut model,
        2,
        100,
        t0,
        ImeEvent::UserImeSetIntent {
            target: false,
            source: UserIntentSource::Command,
        },
    );

    // 窓Bへフォーカス変更。直後は drift クリア済み（gate 開、warmup 許可）。
    let t1 = t0 + Duration::from_millis(10);
    reduce(&mut model, 3, 110, t1, focus_changed(WINDOW_B, 2));
    assert!(model.observations.drift_duration(t1).is_none());

    // 窓Bで新しい観測（open=true）が届く。desired_open は stale な false の
    // ままなので、この観測と食い違い drift が再確立する。
    let t2 = t1 + Duration::from_millis(5);
    reduce(&mut model, 4, 115, t2, observed_open_true(2));

    assert!(
        model.observations.drift_duration(t2).is_some(),
        "窓Bでも stale な desired_open=false と新しい observed=true が \
         食い違うため drift が再確立される。v2 はこの状態でも warmup を \
         抑止する（自己矛盾を一貫した誤りへ変えるトレードオフ、ADR-132 \
         Phase 2 決定参照）"
    );
}
