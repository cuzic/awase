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

/// 補正しない・送らない決定の**根拠**（E1）。判断に実際に使った材料を、実コードの読み取り元で名づけたもの。
///
/// 理由の enum（[`NoDrift`]、`drift_plan` の `DriftIdle`/`GiveUpPark`/`DriftStep`）が `basis()` でこれを返す。
/// 新しい判断の材料を足す場所ではなく、既存の判断が「何を見て省略したか」を診断（ログ）で読めるようにする分類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OmissionBasis {
    /// 信念（`belief.is_japanese_ime`）とユーザーのエンジン有効設定。
    Belief,
    /// 明示意図（`last_intent` の `target`）。`desired_open` が一致しない推測は IME へ書かない（ADR-212 P6）。
    ExplicitIntent,
    /// 観測ストアの乖離の追跡（`drift_duration`）・信頼できる観測の有無・鮮度・値。
    Observation,
    /// 乖離の継続時間（`DRIFT_CORRECTION_THRESHOLD_MS`）。
    DriftWindow,
    /// フォーカス遷移の settle 中。
    FocusSettle,
    /// 授権（`OpenWarrant`）が下りない（BUG-163）。
    Warrant,
    /// 試行回数の上限（`Blind` の `max_attempts`、`attempts` と世代 `origin.epoch` は歩調が同じ）。
    AttemptBudget,
    /// 打ち切り後の再武装クールダウン（BUG-68）。
    Cooldown,
    /// 送信（`sent_at`）または打ち切り（`gave_up_at`）以降の読み戻し（`ObservationStore::read_back`）。
    FreshRead,
}

/// 補正が要るずれが無かった理由。[`evaluate_drift`] の各早期 return に 1 対 1 で対応する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoDrift {
    /// `desired_open` が明示意図と一致しない（古い desired・窓キャッシュの復元・観測ゼロの安全デフォルト）。
    NotExplicitIntent,
    /// 乖離の追跡が無い（観測が `desired` と一致している、またはまだ無い）。
    NotDrifting,
    /// 乖離の継続時間が閾値に満たない。
    BelowThreshold,
    /// `ConvOpenInference` を除いた信頼できる観測が無い。
    NoTrustedObservation,
    /// 最も信頼できる観測が古すぎる（`DRIFT_CORRECTION_OBS_MAX_AGE_MS`）。
    StaleObservation,
    /// 観測が `HeuristicDefault` だけで、明示意図が一度も無い（BUG-110 追補7〜9・issue #189）。
    HeuristicDefaultOnly,
    /// 最も信頼できる観測が `desired` と一致している。
    ObservationMatchesDesired,
}

impl NoDrift {
    #[must_use]
    pub const fn basis(self) -> OmissionBasis {
        match self {
            Self::NotExplicitIntent => OmissionBasis::ExplicitIntent,
            Self::BelowThreshold => OmissionBasis::DriftWindow,
            Self::NotDrifting
            | Self::NoTrustedObservation
            | Self::StaleObservation
            | Self::HeuristicDefaultOnly
            | Self::ObservationMatchesDesired => OmissionBasis::Observation,
        }
    }
}

/// desired ≠ observed ドリフトが補正閾値を超えているか判定し、超えていれば補正情報を、
/// 超えていなければその理由を返す（判定本体。[`check_drift_correction`] は結果の `ok()`）。
///
/// `explicit_intent`: `ImeStateHub::explicit_intent`（= `model.last_intent` の `target`）の値をそのまま渡す。
///
/// `ConvOpenInference` は根拠にしない（BUG-173 追補3）。`resolve_warmup_ime_on` が同じ述語を
/// `matches!(.., Some(DriftCorrection { desired: false, observed: true, .. }))` として使う（ADR-132/INV-B1'）。
pub fn evaluate_drift(
    model: &ImeModel,
    now: std::time::Instant,
    explicit_intent: Option<bool>,
) -> Result<DriftCorrection, NoDrift> {
    let desired = model.desired_open();

    // ADR-212 P6: **ユーザーの明示操作の書き込みが届かなかったときの再試行だけ**を残す。`desired_open` が明示意図
    // （`explicit_intent`）と一致しないとき（古い desired の補正、窓キャッシュの復元〈`HwndCacheRestored`〉の押し付け、
    // 観測ゼロの安全デフォルト）は、awase が自分の推測を実 IME へ書くことになるので補正しない（「awase は IME に書かない」）。
    // 実機の過去ログ(dragonflyg4)では drift 補正の書き込み 169 件が全て「IME を OFF にする」方向で、開ける方向は 0 件だった。
    if explicit_intent != Some(desired) {
        return Err(NoDrift::NotExplicitIntent);
    }

    let dur = model
        .observations
        .drift_duration(now)
        .ok_or(NoDrift::NotDrifting)?;
    // last_intent は UserImeSetIntent / UserImeToggleIntent のみが設定する。
    // PanicReset / HwndCacheRestored は設定しないため、is_some() で十分。
    // SyncKey / PhysicalImeKey / Command は全て閾値 0 (即時補正) の対象。
    // （ここへ来る時点で `explicit_intent == Some(desired)` は成立済み。）
    let threshold = if model.last_intent.is_some() {
        0
    } else {
        u128::from(crate::tuning::DRIFT_CORRECTION_THRESHOLD_MS)
    };
    if dur.as_millis() < threshold {
        return Err(NoDrift::BelowThreshold);
    }

    let max_age = std::time::Duration::from_millis(crate::tuning::DRIFT_CORRECTION_OBS_MAX_AGE_MS);
    // `ConvOpenInference` は drift correction の根拠にしない（下記）。選んだ後に捨てると、同じ Medium の他ソースの
    // 正当な観測まで覆い隠すので、選ぶ前に除外する。
    let trusted = model
        .observations
        .most_recent_trusted_excluding(now, &[ObservationSource::ConvOpenInference])
        .ok_or(NoDrift::NoTrustedObservation)?;
    if trusted.age(now) > max_age {
        return Err(NoDrift::StaleObservation);
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
        return Err(NoDrift::HeuristicDefaultOnly);
    }
    if trusted.open == desired {
        return Err(NoDrift::ObservationMatchesDesired);
    }

    Ok(DriftCorrection {
        desired,
        observed: trusted.open,
        duration_ms: dur.as_millis() as u64,
        source: trusted.source,
        confidence: trusted.confidence,
    })
}

/// [`evaluate_drift`] の結果から理由を捨てたもの。`ImeStateHub::check_drift_correction`・閉ループのハーネス・
/// `resolve_warmup_ime_on` の述語が使う。
#[must_use]
pub fn check_drift_correction(
    model: &ImeModel,
    now: std::time::Instant,
    explicit_intent: Option<bool>,
) -> Option<DriftCorrection> {
    evaluate_drift(model, now, explicit_intent).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::observation_store::ImeDrift;
    use std::time::{Duration, Instant};

    fn drifting_since(model: &mut ImeModel, started_at: Instant) {
        model.observations.drift = Some(ImeDrift { started_at });
    }

    /// 理由は `evaluate_drift` の早期 return と 1 対 1（観測の記録が要る 3 種は `platform_state` の
    /// `check_drift_correction_*` テストと閉ループのシナリオが、`check_drift_correction` 経由で固定する）。
    #[test]
    fn reasons_follow_the_early_returns_in_order() {
        let now = Instant::now();
        let mut model = ImeModel::default();
        let desired = model.desired_open();

        // 明示意図が無い／desired と違う → 他の事実（乖離の追跡）より先に見る。
        drifting_since(&mut model, now);
        assert_eq!(
            evaluate_drift(&model, now, None),
            Err(NoDrift::NotExplicitIntent)
        );
        assert_eq!(
            evaluate_drift(&model, now, Some(!desired)),
            Err(NoDrift::NotExplicitIntent)
        );

        // 明示意図は一致するが乖離の追跡が無い。
        model.observations.drift = None;
        assert_eq!(
            evaluate_drift(&model, now, Some(desired)),
            Err(NoDrift::NotDrifting)
        );

        // 追跡はあるが、`last_intent` が無い（閾値あり）ので継続時間が足りない。
        drifting_since(&mut model, now);
        assert!(model.last_intent.is_none());
        assert_eq!(
            evaluate_drift(&model, now, Some(desired)),
            Err(NoDrift::BelowThreshold)
        );

        // 閾値を超えたが、信頼できる観測が無い。
        // （`Instant` の減算を避け、起点から進めた時刻で評価する。）
        let later = now + Duration::from_millis(crate::tuning::DRIFT_CORRECTION_THRESHOLD_MS + 50);
        assert_eq!(
            evaluate_drift(&model, later, Some(desired)),
            Err(NoDrift::NoTrustedObservation)
        );
        assert_eq!(check_drift_correction(&model, later, Some(desired)), None);
    }

    #[test]
    fn every_reason_names_a_basis() {
        use OmissionBasis::*;
        for (reason, basis) in [
            (NoDrift::NotExplicitIntent, ExplicitIntent),
            (NoDrift::NotDrifting, Observation),
            (NoDrift::BelowThreshold, DriftWindow),
            (NoDrift::NoTrustedObservation, Observation),
            (NoDrift::StaleObservation, Observation),
            (NoDrift::HeuristicDefaultOnly, Observation),
            (NoDrift::ObservationMatchesDesired, Observation),
        ] {
            assert_eq!(reason.basis(), basis, "{reason:?}");
        }
    }
}
