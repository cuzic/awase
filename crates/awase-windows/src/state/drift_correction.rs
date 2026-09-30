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
/// （`runtime/ime_refresh.rs`）が drift の根拠（`source`）を、呼び出し元が
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
/// `ConvOpenInference` は根拠にしない（BUG-173 追補3）。`resolve_warmup_ime_on` が同じ述語を
/// `matches!(.., Some(DriftCorrection { desired: false, observed: true, .. }))` として使う（ADR-132/INV-B1'）。
#[must_use]
pub fn check_drift_correction(
    model: &ImeModel,
    now: std::time::Instant,
    explicit_intent: Option<bool>,
) -> Option<DriftCorrection> {
    let desired = model.desired_open();

    // ADR-212 P6: **ユーザーの明示操作の書き込みが届かなかったときの再試行だけ**を残す。`desired_open` が明示意図
    // （`explicit_intent`）と一致しないとき（古い desired の補正、窓キャッシュの復元〈`HwndCacheRestored`〉の押し付け、
    // 観測ゼロの安全デフォルト）は、awase が自分の推測を実 IME へ書くことになるので補正しない（「awase は IME に書かない」）。
    // 実機の過去ログ(dragonflyg4)では drift 補正の書き込み 169 件が全て「IME を OFF にする」方向で、開ける方向は 0 件だった。
    if explicit_intent != Some(desired) {
        return None;
    }

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
    // `ConvOpenInference` は drift correction の根拠にしない（下記）。選んだ後に捨てると、同じ Medium の他ソースの
    // 正当な観測まで覆い隠すので、選ぶ前に除外する。
    let trusted = model
        .observations
        .most_recent_trusted_excluding(now, &[ObservationSource::ConvOpenInference])?;
    if trusted.age(now) > max_age {
        return None;
    }
    // ConvOpenInference（conv ビットからの間接推測、KatakanaShadowOff/NativeToggleShadowOff 由来）は drift correction の
    // 根拠にしない（BUG-173 追補3 / Opus 発火削減 D4。上の `most_recent_trusted_excluding` で除外済み）。conv の NATIVE ビットは
    // IME を閉じても残る持続的な設定で（BUG-172・BUG-68）、`VK_IME_OFF` を何度送っても観測が変わらない（反証不能）。
    // 「conv-mode を actuation のゲートに使わない」方針とも矛盾する。journal 01M3NJ784NKMH120HM6QGKF7W7 では、ユーザー自身の
    // Ctrl+無変換（`VK_IME_OFF`）の 106ms 後に、この推測が根拠の drift correction が同じ `VK_IME_OFF` を重ねて送っていた。
    // 開閉を読む手段が無い TsfNative×GJI では、最初の OFF が失われても自動では再送せずユーザーの押し直しに委ねる（受動化）。
    // HeuristicDefault（観測ゼロの安全デフォルト、`reset_stale_ime_on_for_imm_broken` が Imm32Unavailable
    // ウィンドウ入場時に記録する）は、明示的なユーザー意図が一度も無い間は単独で drift correction を
    // 発火させない（BUG-110 追補7〜9・issue #189: `FocusChanged` で `last_intent` がクリアされた直後に
    // 新しいウィンドウの `HeuristicDefault` 観測を record すると、別ウィンドウでの古い明示操作の残留である
    // `desired` と食い違い、弱い観測1件を理由に実 IME へ書き込んでしまっていた）。
    if trusted.source == ObservationSource::HeuristicDefault && explicit_intent.is_none() {
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
