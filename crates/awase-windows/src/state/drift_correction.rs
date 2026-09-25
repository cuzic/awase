//! drift correction（`desired_open` ≠ 観測 の補正）の**判定本体**。
//!
//! 以前は `ImeStateHub::check_drift_correction`（`state/platform_state.rs`）の本体だったが、
//! `platform_state.rs` は `#[cfg(windows)]` のため Linux ホストの
//! `cargo test -p awase-windows` から呼べなかった。判定が読むのは `ImeModel`（ungated）の
//! `desired_open()`・`last_intent`・`observations` だけなので、本体をこの ungated モジュールへ
//! **そのまま**移し、`ImeStateHub::check_drift_correction` は委譲だけにした
//! （判定ロジックは1行も変えていない。`tests/closed_loop_scenarios.rs` が Linux で呼ぶため）。
//!
//! 読み取り専用の純粋関数であり、belief（`desired_open`/`input_mode`）へは書かない
//! （`.claude/rules/ime-belief-architecture.md` の書き込み点は `ImeModel::reduce()` のまま）。

use super::ime_event::{ObservationConfidence, ObservationSource};
use super::ime_model::ImeModel;

/// [`check_drift_correction`] の戻り値（BUG-113残置課題）。
///
/// 旧 `(bool, bool, u64)` タプルから構造体化したのは、`ir_apply_drift_correction`
/// （`runtime/ime_refresh.rs`）が `ConvOpenInference` 由来の drift を
/// 「明示意図エピソードあたり1送信」に絞る際の根拠（`source`）を、呼び出し元が
/// 別途 `most_recent_trusted()` を再計算せずに受け取れるようにするため
/// （独立再計算は BUG-110 と同型の構造的欠陥、`resolve_warmup_ime_on` の doc 参照）。
/// `confidence` は診断ログ専用で判定には使わない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DriftCorrection {
    pub desired: bool,
    pub observed: bool,
    pub duration_ms: u64,
    pub source: ObservationSource,
    pub confidence: ObservationConfidence,
}

/// desired ≠ observed ドリフトが補正閾値を超えているか判定し、超えていれば補正情報を返す。
///
/// 戻り値: 補正が必要な場合 `Some(DriftCorrection { .. })`。
/// `explicit_intent`: `ImeStateHub::explicit_intent`（= `model.last_intent` の `target`）の値をそのまま渡す。
///
/// BUG-113残置課題（2026-09-06）: 従来 `(bool, bool, u64)` タプルだったが、
/// `ir_apply_drift_correction`側でconv由来drift（`ConvOpenInference`）を
/// 「明示意図エピソードあたり1送信」に絞るために`source`/`confidence`を
/// 追加した構造体に変えた。**判定ロジック自体は1行も変えていない**——
/// `resolve_warmup_ime_on`が同じ述語を`matches!(.., Some(DriftCorrection
/// { desired: false, observed: true, .. }))`として使うため、旧
/// `Some((false, true, _))`とビット同値であること（ADR-132/INV-B1'）。
#[must_use]
pub fn check_drift_correction(
    model: &ImeModel,
    now: std::time::Instant,
    explicit_intent: Option<bool>,
) -> Option<DriftCorrection> {
    let desired = model.desired_open();

    let dur = model.observations.drift_duration(now)?;
    // last_intent は UserImeSetIntent / UserImeToggleIntent のみが設定する。
    // PanicReset / HwndCacheRestored は設定しないため、is_some() で十分。
    // SyncKey / PhysicalImeKey / Command は全て閾値 0 (即時補正) の対象。
    let is_strong_intent = model.last_intent.is_some();
    let threshold = if explicit_intent == Some(desired) && is_strong_intent {
        0
    } else {
        u128::from(crate::tuning::DRIFT_CORRECTION_THRESHOLD_MS)
    };
    if dur.as_millis() < threshold {
        return None;
    }

    let max_age = std::time::Duration::from_millis(crate::tuning::DRIFT_CORRECTION_OBS_MAX_AGE_MS);
    let trusted = model.observations.most_recent_trusted(now)?;
    if trusted.age(now) > max_age {
        return None;
    }
    // ConvOpenInference（conv ビットからの間接推測、KatakanaShadowOff/
    // NativeToggleShadowOff 由来）は、明示的なユーザー意図が一度も無い間は単独で
    // drift correction を発火させない。desired_open のデフォルト値（起動直後等、
    // last_intent が一度も設定されていない状態）を conv 由来の推論だけで
    // actuate すると、ユーザーが望んでもいない ON/OFF の押し付けになりかねない。
    // 明示意図がある場合（BUG-19 再発の本来のシナリオ: ユーザーが OFF にした
    // 直後に conv がまだ native/katakana を示す）はこの gate を素通りし、
    // 既存の `desired`（ユーザーの意図した値）が正しく再適用される。
    //
    // BUG-110 追補7〜9（issue #189）: `HeuristicDefault`（観測ゼロの安全
    // デフォルト、`reset_stale_ime_on_for_imm_broken` が Imm32Unavailable
    // ウィンドウ入場時に記録する）でも全く同じ構造の問題が起きる——
    // `FocusChanged` で `last_intent` がクリアされた直後に新しいウィンドウの
    // `HeuristicDefault` 観測を record すると、`desired`（生の
    // `desired_open()`、別ウィンドウでの古い明示操作の残留）と食い違い、
    // drift correction がこの弱い観測1件を理由に実 IME へ書き込んでしまう。
    // （撤去済みの）`apply_force_on_for_imm_broken`（`effective_open()` 経由で同じ
    // `HeuristicDefault` を信頼していた）と反対方向の書き込みを競って短時間に
    // 往復していた。
    //
    // ここに含めるかどうかの判断基準は「`ObservationSource::authority()`
    // が `BeliefOnly` かどうか」ではない——`authority()` は `HwndCache`/
    // `FocusProbe`/`ConvBitsInference`/`GjiIoInference` も含む6バリアント
    // を持ち、判断基準として使うには広すぎる（opus-adversarial-consult
    // 指摘）。正しい基準は**「外部観測の裏付けが一切ない、awase 自身の
    // 推測であること」**——これを満たすのは `ConvOpenInference`（conv
    // ビットからの間接推測）と `HeuristicDefault`（観測ゼロの安全
    // デフォルト）の2つだけ。同じ `BeliefOnly` でも性質が違う残り4つを
    // 対象外とする理由は個別に検討済みで、いずれも「まだ調べていないから」
    // ではない:
    // - `ConvBitsInference`/`GjiIoInference` は input_mode 専用ソースで
    //   open/close 観測として `most_recent_trusted()` に到達しない
    //   （`PerSourceObservations::get`/`set` が None/no-op を返す、
    //   `authority()` 自身の doc 参照）——追加しても到達しないデッドコード
    //   が増えるだけ。
    // - `HwndCache` が運ぶ値は `HwndCacheRestored` が `desired_open` に
    //   書く値と同一のため `trusted.open == desired` となり、下の等値
    //   チェックで既に `None` になる（今日は無害）。将来その不変条件が
    //   崩れたときに正当な補正経路を黙って殺す副作用だけが残るため、
    //   あえて含めない。
    // - `FocusProbe` は推測ではなく実 IMC 読み取り（Low confidence なのは
    //   hwnd の曖昧性ゆえ、BUG-91 由来）。これを抑止すると BUG-16/BUG-20
    //   型の固着（belief と実 IME が乖離したまま補正されない）を再導入する
    //   リスクがあり、実機再現なしに含めるべきではない。
    if matches!(
        trusted.source,
        ObservationSource::ConvOpenInference | ObservationSource::HeuristicDefault
    ) && explicit_intent.is_none()
    {
        return None;
    }
    if trusted.open == desired {
        return None;
    }

    Some(DriftCorrection {
        desired,
        observed: trusted.open,
        duration_ms: dur.as_millis() as u64,
        source: trusted.source,
        confidence: trusted.confidence,
    })
}
