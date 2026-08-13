//! BUG-51 追補 v3 の中核（`IntentStore` による `effective_open()` 上書き）の
//! **Linux で実行できる**回帰テスト。
//!
//! ## なぜ別ファイルなのか
//!
//! 同じシナリオのテストは `src/state/platform_state.rs` の `mod tests`（`cfg(test)`）
//! にもある。しかし `state/platform_state.rs` は `state/mod.rs` で
//! `#[cfg(windows)]` されており、**Linux の `cargo test -p awase-windows` では
//! 1 件もコンパイルされない**。実機（Windows）でテストを走らせられない環境で
//! 書かれた BUG-51 追補は、結果として「Windows クロスチェックの型検査しか
//! 受けていない」状態だった。ここでは判定本体
//! （`IntentStore::resolve_effective_open()`、ungated な `state/intent_store.rs`）と
//! `ImeModel`（同じく ungated）を実際に組み合わせ、Linux CI で毎回走らせる。
//!
//! ## 再現する実機バグ（2026-08-11、`docs/known-bugs.md` BUG-51 追補）
//!
//! MS-IME / TsfNative（Windows Terminal 等）で明示 IME OFF（Ctrl+無変換）の直後に
//! フォーカス変更が起きると、`ImeModel.last_intent` が無条件にクリアされ、直後に
//! 届いた `ConvOpenInference`（conv=NATIVE を「open」と誤読する BUG-55 由来の
//! 壊れた観測）**1 件だけ**で `ImeModel::effective_open()` が true に反転する。
//! `Engine::compute_state` はこれを `ctx.ime_on` として直接使うため、実 IME は
//! OFF のままなのに Engine だけが再活性化する。
//!
//! ADR-089 Phase A（`ObservationStore` の Actuating/Belief 二プール分離）を
//! 経てもこの反転は残る——`ImeModel::effective_open()` は belief 値なので
//! `derive_any()`（両プール）を使っており、`ConvOpenInference` は
//! `BeliefPool` に居ながら Medium 単独合意で採用されるためである。
//! したがって BUG-51 追補の `IntentStore` 上書きは Phase A 後も必要。

use std::time::Instant;

use awase_windows::state::conv_classify::ConvSyncReason;
use awase_windows::state::evidence::{ConvOpenInference, Observed};
use awase_windows::state::ime_event::{
    EventTime, HwndId, ImeEvent, ImeEventEnvelope, ImePolicyProfile, UserIntentSource,
};
use awase_windows::state::ime_model::ImeModel;
use awase_windows::state::intent_store::IntentStore;
use awase_windows::state::TickMs;

const TARGET: HwndId = HwndId(0x1234);
const OTHER: HwndId = HwndId(0x5678);

fn reduce(model: &mut ImeModel, seq: u64, tick_ms: u64, event: ImeEvent) {
    model.reduce(&ImeEventEnvelope {
        time: EventTime {
            seq,
            monotonic: Instant::now(),
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

/// TsfNative の壊れた conv 由来 open 推論（`NativeToggleShadowOff`）。
///
/// **本番の観測構築経路そのもの**を使う: `ImeStateHub::report_conv_open_inference()`
/// は `Observed::<evidence::ConvOpenInference>::from_conv(reason, open, HwndId::NULL,
/// focus_epoch).into()` を `ObserverReported` として dispatch する
/// （`state/platform_state.rs`）。ここではその式をそのまま組み立てる。
///
/// `report_conv_open_inference()` 自体を呼ばないのは、それが
/// `#[cfg(windows)]` な `ImeStateHub` の `pub(crate)` メソッドであり、
/// Linux の統合テスト（別クレート扱い）からは型としても存在しないため。
/// 代わりに、その中身のうち**観測の作り方**（どの witness 構築子を通り、
/// confidence が何になるか）を完全に共有する。リプレイ用バックドア
/// （`AnyObservation::restored_from_journal`、ADR-089 §2.1）は使わない——
/// 任意の source/confidence を後から手で書けてしまい、`from_conv` が
/// `Medium` を固定しているという本番側の性質を検証しなくなるため。
fn broken_conv_open_inference(focus_epoch: u64) -> ImeEvent {
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

/// 明示 IME OFF（Ctrl+無変換 等）。実機では
/// `handle_engine_set_open(ExplicitUserAction)` が belief を書き、
/// `record_explicit_intent()` が `IntentStore` に記録する 2 段構え。
fn explicit_off(model: &mut ImeModel, store: &mut IntentStore, seq: u64, tick_ms: u64) {
    reduce(
        model,
        seq,
        tick_ms,
        ImeEvent::UserImeSetIntent {
            target: false,
            source: UserIntentSource::Command,
        },
    );
    store.record(TARGET, false, UserIntentSource::Command, TickMs(tick_ms));
}

/// 実機再現手順そのもの: 明示 OFF → 同一対象への `FocusChanged`（`last_intent`
/// 消失）→ 壊れた `ConvOpenInference` 1 件。
///
/// 生の `ImeModel::effective_open()` は true へ反転する（**退行の証拠として
/// 明示的にアサートする**——ここが false になったら BUG-51 の前提が変わって
/// おり、本テストの意味を再確認すること）。`IntentStore` を重ねた
/// `resolve_effective_open()` は false を維持する。
#[test]
fn broken_conv_inference_alone_does_not_flip_effective_open() {
    let mut model = ImeModel::default();
    let mut store = IntentStore::default();

    reduce(&mut model, 1, 0, focus_changed(TARGET, 1));
    explicit_off(&mut model, &mut store, 2, 100);
    assert!(!model.effective_open(), "明示 OFF 直後は false");

    // 同一対象へのフォーカス再構築（スリープ復帰・BUG-57 型の一瞬のフォーカス
    // 奪取など）が last_intent と観測プールをクリアする。
    reduce(&mut model, 3, 200, focus_changed(TARGET, 2));

    // 壊れた conv 観測が 1 件だけ届く。
    reduce(&mut model, 4, 300, broken_conv_open_inference(2));

    assert!(
        model.effective_open(),
        "退行の証拠: IntentStore 抜きの生の ImeModel::effective_open() は \
         ConvOpenInference 1 件（Medium 単独合意）だけで true に反転する"
    );

    let decision =
        store.resolve_effective_open(model.current_focus(), model.effective_open(), TickMs(300));
    assert!(
        !decision.value,
        "IntentStore を重ねた effective_open() は同一対象なら明示 OFF 意図を \
         維持し、Engine の ctx.ime_on が誤って true に反転しない"
    );
    assert_eq!(
        decision.intent.map(|i| i.source),
        Some(UserIntentSource::Command),
        "上書きの根拠として、記録された明示意図そのものが返る"
    );
}

/// `record_explicit_intent()` を経由しない belief 書き込みだけでは
/// `IntentStore` に何も残らない（BUG-51 追補 v3 修正1b）。
///
/// v1 では `dispatch_event(UserImeSetIntent{Command})` 自体が record して
/// いたため、conv 由来の内部同期（`EngineSync::DirectInput` →
/// `handle_engine_set_open` → `write_set_open_request`）が「壊れた conv 読み
/// 1 件」を `FocusChanged` を生き延びる偽の明示意図として永続化していた
/// （pre-mortem #1 角度2）。
#[test]
fn belief_write_without_record_leaves_intent_store_empty() {
    let mut model = ImeModel::default();
    let store = IntentStore::default();

    reduce(&mut model, 1, 0, focus_changed(TARGET, 1));
    reduce(
        &mut model,
        2,
        100,
        ImeEvent::UserImeSetIntent {
            target: false,
            source: UserIntentSource::Command,
        },
    );
    reduce(&mut model, 3, 200, focus_changed(TARGET, 2));
    reduce(&mut model, 4, 300, broken_conv_open_inference(2));

    let decision =
        store.resolve_effective_open(model.current_focus(), model.effective_open(), TickMs(300));
    assert!(
        decision.value,
        "record_explicit_intent を経由しない belief 書き込みだけでは \
         IntentStore に何も残らず、通常どおり観測（conv, true）へフォールバックする"
    );
    assert!(decision.intent.is_none());
}

/// 対象が違えば漏れない（ADR-087 INV-24(b) の2段判定、BUG-26 非退行）。
#[test]
fn intent_does_not_leak_to_a_different_target() {
    let mut model = ImeModel::default();
    let mut store = IntentStore::default();

    reduce(&mut model, 1, 0, focus_changed(TARGET, 1));
    explicit_off(&mut model, &mut store, 2, 100);

    // 別ウィンドウへの本物のフォーカス変更。
    reduce(&mut model, 3, 200, focus_changed(OTHER, 2));
    reduce(&mut model, 4, 300, broken_conv_open_inference(2));

    let decision =
        store.resolve_effective_open(model.current_focus(), model.effective_open(), TickMs(300));
    assert!(
        decision.value,
        "別対象では IntentStore のエントリを使わず、その対象の観測に従う"
    );
    assert!(decision.intent.is_none());
}

/// OFF 意図の TTL 超過後は無期限固着せず観測へフォールバックする
/// （ADR-087 §7 round4 M-A、`HwndImeCache` と同型の有界設計）。
#[test]
fn off_intent_stops_overriding_after_its_ttl() {
    let mut model = ImeModel::default();
    let mut store = IntentStore::default();

    reduce(&mut model, 1, 0, focus_changed(TARGET, 1));
    explicit_off(&mut model, &mut store, 2, 0);
    reduce(&mut model, 3, 0, focus_changed(TARGET, 2));
    reduce(&mut model, 4, 0, broken_conv_open_inference(2));

    let shadow = model.effective_open();
    assert!(shadow, "前提: 生の belief は conv 観測で true");

    let off_ttl = awase_windows::tuning::EXPLICIT_OFF_INTENT_TTL_MS;
    assert!(
        !store
            .resolve_effective_open(model.current_focus(), shadow, TickMs(off_ttl))
            .value,
        "TTL ちょうどまでは明示 OFF 意図が勝つ"
    );
    assert!(
        store
            .resolve_effective_open(model.current_focus(), shadow, TickMs(off_ttl + 1))
            .value,
        "TTL を超えたら固着せず観測ベースの belief へフォールバックする"
    );
}

/// フォーカス未確定（`current_focus() == None`）では上書きしない。
/// `ImeStateHub::effective_open()` の `Option<HwndId>` 分岐の固定。
#[test]
fn unknown_focus_never_overrides() {
    let mut model = ImeModel::default();
    let mut store = IntentStore::default();
    store.record(TARGET, false, UserIntentSource::Command, TickMs(0));

    reduce(&mut model, 1, 0, broken_conv_open_inference(0));
    assert_eq!(model.current_focus(), None, "FocusChanged 未発生");

    let decision =
        store.resolve_effective_open(model.current_focus(), model.effective_open(), TickMs(100));
    assert!(decision.value, "対象が分からなければ上書きしない");
    assert!(decision.intent.is_none());
}
