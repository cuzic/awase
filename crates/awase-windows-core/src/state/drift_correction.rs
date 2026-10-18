//! drift correction（`desired_open` ≠ 観測 の補正）の**判定本体**。
//!
//! 以前は `ImeStateHub::check_drift_correction`（`state/platform_state.rs`）の本体だったが、
//! `platform_state.rs` は `#[cfg(windows)]` だった時期があり、Linux ホストの
//! `cargo test -p awase-windows` から呼べなかった。判定が読むのは `ImeModel`（ungated）の
//! `desired_open()`・`last_intent`・`observations` だけなので、本体をこの ungated モジュールへ移した
//! （`tests/closed_loop_scenarios.rs` が Linux で呼ぶ）。明示意図は `last_intent` から本関数の中で作る
//! （呼び出し側が別の値を渡せない）。FCIS E1 で、補正しない理由を [`NoDrift`] で返す
//! [`evaluate_drift`] を判定本体にし、不要になった閾値の分岐と `HeuristicDefault` 単独の除外
//! （明示意図の判定が先に返すので到達できなかった）を撤去した。
//!
//! 読み取り専用の純粋関数であり、belief（`desired_open`/`input_mode`）へは書かない
//! （`.claude/rules/ime-belief-architecture.md` の書き込み点は `ImeModel::reduce()` のまま）。

use super::ime_event::{ObservationConfidence, ObservationSource};
use super::ime_model::ImeModel;

/// [`evaluate_drift`] の成功時の値（BUG-113残置課題）。
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
///
/// **付け方の規則**: 複数の述語を順に見て決まる決定は、**その決定を最後に決めた述語の読み取り元**を根拠にする
/// （例: `GiveUp(StillParked)` は上限・クールダウンを通った後で「新しい読み戻しが無い」が決めたので `FreshRead`）。
/// 新しい理由を足すときは、実コードのどの値を読んで返すかから根拠を選ぶ（`basis()` は網羅的な `match`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OmissionBasis {
    /// IME の信念（`belief.is_japanese_ime`）。
    Belief,
    /// ユーザーのエンジン有効設定（`engine.is_user_enabled()`）。信念ではなく設定。
    EngineSetting,
    /// 明示意図（`last_intent` の `target`）。`desired_open` が一致しない推測は IME へ書かない（ADR-212 P6）。
    ExplicitIntent,
    /// 観測ストアの乖離の追跡（`drift_duration`）・信頼できる観測の有無・鮮度・値。
    Observation,
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

impl OmissionBasis {
    /// journal・ログ用の名前(ADR-250 段階 1)。`?`/`%` を使わずに出すための固定文字列で、網羅的な `match`
    /// なので variant を足すと型で更新漏れが分かる。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Belief => "Belief",
            Self::EngineSetting => "EngineSetting",
            Self::ExplicitIntent => "ExplicitIntent",
            Self::Observation => "Observation",
            Self::FocusSettle => "FocusSettle",
            Self::Warrant => "Warrant",
            Self::AttemptBudget => "AttemptBudget",
            Self::Cooldown => "Cooldown",
            Self::FreshRead => "FreshRead",
        }
    }
}

/// 補正が要るずれが無かった理由。[`evaluate_drift`] の各早期 return に 1 対 1 で対応する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoDrift {
    /// `desired_open` が明示意図と一致しない（古い desired・窓キャッシュの復元・観測ゼロの安全デフォルト）。
    NotExplicitIntent,
    /// 乖離の追跡が無い（観測が `desired` と一致している、またはまだ無い）。
    NotDrifting,
    /// `ConvOpenInference` を除いた信頼できる観測が無い。
    NoTrustedObservation,
    /// 最も信頼できる観測が古すぎる（`DRIFT_CORRECTION_OBS_MAX_AGE_MS`）。
    StaleObservation,
    /// 最も信頼できる観測が `desired` と一致している。
    ObservationMatchesDesired,
}

impl NoDrift {
    /// journal・ログ用の名前(ADR-250 段階 1)。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::NotExplicitIntent => "NotExplicitIntent",
            Self::NotDrifting => "NotDrifting",
            Self::NoTrustedObservation => "NoTrustedObservation",
            Self::StaleObservation => "StaleObservation",
            Self::ObservationMatchesDesired => "ObservationMatchesDesired",
        }
    }

    #[must_use]
    pub const fn basis(self) -> OmissionBasis {
        match self {
            Self::NotExplicitIntent => OmissionBasis::ExplicitIntent,
            Self::NotDrifting
            | Self::NoTrustedObservation
            | Self::StaleObservation
            | Self::ObservationMatchesDesired => OmissionBasis::Observation,
        }
    }
}

/// desired ≠ observed ドリフトが補正閾値を超えているか判定し、超えていれば補正情報を、
/// 超えていなければその理由を返す（判定本体。[`check_drift_correction`] は結果の `ok()`）。
///
/// `ConvOpenInference` は根拠にしない（BUG-173 追補3）。
pub fn evaluate_drift(
    model: &ImeModel,
    now: std::time::Instant,
) -> Result<DriftCorrection, NoDrift> {
    let desired = model.desired_open();
    // 明示意図（`ImeStateHub::explicit_intent` と同じ値）。引数で受けず、ここで `last_intent` から作る。
    let explicit_intent = model.last_intent.as_ref().map(|i| i.target);

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
    // 継続時間の閾値は持たない（即時補正）。明示意図は `last_intent` から作っているので、上の判定を通った
    // 時点で `last_intent` は必ず在り、かつての閾値は常に 0 だった（`last_intent` 無しの閾値
    // `DRIFT_CORRECTION_THRESHOLD_MS` と `HeuristicDefault` 単独の除外は、この判定が先に返すので到達できなかった。
    // issue #189 の状況は `last_intent` が消えて明示意図が無いのでここで止まる）。

    let max_age = std::time::Duration::from_millis(crate::tuning::DRIFT_CORRECTION_OBS_MAX_AGE_MS);
    // `ConvOpenInference` は drift correction の根拠にしない（下記）。選んだ後に捨てると、同じ Medium の他ソースの
    // 正当な観測まで覆い隠すので、選ぶ前に除外する。
    let trusted = model
        .observations
        .most_recent_trusted_excluding(now, &[ObservationSource::ConvOpenInference])
        .ok_or(NoDrift::NoTrustedObservation)?;
    if trusted.age(now) > max_age {
        // 診断(ADR-233 の「drift が古い ICP で止まる疑い」の測定): 補正を見送った根拠の観測。明示意図があり乖離が続いている
        // ときだけ来る(上の 2 つの早期 return を通った後)ので頻度は低い。挙動は変えない。
        tracing::debug!(
            "[drift-skip] StaleObservation source={:?} confidence={:?} age_ms={} observed={} desired={desired}",
            trusted.source,
            trusted.confidence,
            trusted.age(now).as_millis(),
            trusted.open,
        );
        return Err(NoDrift::StaleObservation);
    }
    // ConvOpenInference（conv ビットからの間接推測、KatakanaShadowOff/NativeToggleShadowOff 由来）は drift correction の
    // 根拠にしない（BUG-173 追補3 / Opus 発火削減 D4。上の `most_recent_trusted_excluding` で除外済み）。conv の NATIVE ビットは
    // IME を閉じても残る持続的な設定で（BUG-172・BUG-68）、`VK_IME_OFF` を何度送っても観測が変わらない（反証不能）。
    // 「conv-mode を actuation のゲートに使わない」方針とも矛盾する。journal 01M3NJ784NKMH120HM6QGKF7W7 では、ユーザー自身の
    // Ctrl+無変換（`VK_IME_OFF`）の 106ms 後に、この推測が根拠の drift correction が同じ `VK_IME_OFF` を重ねて送っていた。
    // 開閉を読む手段が無い TsfNative×GJI では、最初の OFF が失われても自動では再送せずユーザーの押し直しに委ねる（受動化）。
    // HeuristicDefault（観測ゼロの安全デフォルト）単独の補正は、BUG-110 追補7〜9・issue #189 で止めた。今は
    // 明示意図が無ければ最初の判定（`NotExplicitIntent`）で返るので、ここで個別に除外する必要は無い。
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

/// [`evaluate_drift`] の結果から理由を捨てたもの。本番の呼び出し元は `evaluate_drift` に移った。
/// 閉ループのハーネス（`tests/support/harness.rs`）が使う。
#[must_use]
pub fn check_drift_correction(
    model: &ImeModel,
    now: std::time::Instant,
) -> Option<DriftCorrection> {
    evaluate_drift(model, now).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::ime_event::UserIntentSource;
    use crate::state::ime_model::RecordedIntent;
    use crate::state::observation_store::ImeDrift;
    use std::time::Instant;

    fn drifting_since(model: &mut ImeModel, started_at: Instant) {
        model.observations.drift = Some(ImeDrift { started_at });
    }

    /// 理由と根拠は、実際の入力から評価した結果で確かめる（対応表を写さない）。観測が要る理由は
    /// `platform_state` のテスト（`evaluate_drift_reports_*`）が確かめる。
    fn with_intent(model: &mut ImeModel, target: bool) {
        model.last_intent = Some(RecordedIntent {
            target,
            source: UserIntentSource::Command,
            at_ms: 0,
        });
    }

    #[test]
    fn reasons_follow_the_early_returns_in_order() {
        let now = Instant::now();
        let mut model = ImeModel::default();
        let desired = model.desired_open();

        // 明示意図が無い／desired と違う → 他の事実（乖離の追跡）より先に見る。
        drifting_since(&mut model, now);
        let r = evaluate_drift(&model, now);
        assert_eq!(r, Err(NoDrift::NotExplicitIntent));
        assert_eq!(r.unwrap_err().basis(), OmissionBasis::ExplicitIntent);
        with_intent(&mut model, !desired);
        assert_eq!(evaluate_drift(&model, now), Err(NoDrift::NotExplicitIntent));

        // 明示意図は一致するが乖離の追跡が無い。
        with_intent(&mut model, desired);
        model.observations.drift = None;
        let r = evaluate_drift(&model, now);
        assert_eq!(r, Err(NoDrift::NotDrifting));
        assert_eq!(r.unwrap_err().basis(), OmissionBasis::Observation);

        // 追跡はあるが、信頼できる観測が無い。
        drifting_since(&mut model, now);
        let r = evaluate_drift(&model, now);
        assert_eq!(r, Err(NoDrift::NoTrustedObservation));
        assert_eq!(r.unwrap_err().basis(), OmissionBasis::Observation);
        assert_eq!(check_drift_correction(&model, now), None);
    }
}
