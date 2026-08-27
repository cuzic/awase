//! `apply_focus_probe`/`apply_effective_ime`（`runtime/key_pipeline.rs`）に埋め込まれて
//! いた `match status { .. }` の決定ロジックを純粋関数として抽出したもの
//! （PR 109 コードレビュー指摘3）。
//!
//! 抽出前は `FocusProbeOpenStatus` の分岐内で `self.apply_effective_ime(..)` /
//! `self.release_detect_state_guard_if(..)` という副作用呼び出しが直接書かれており、
//! 「何を記録し、いつ guard を解除するか」という決定と「実際に書き込む」という
//! 副作用が同じ場所に混在していた。この関数はその決定だけを型として切り出す
//! ——挙動不変のリファクタであり、分岐条件は逐語的に移植した。
//!
//! `ObservedOpenValue` の private フィールド規律（ADR-106 決定2、BUG-92）は
//! ここでも守る: [`plan_focus_probe`] は `FocusProbeOpenStatus::Read` から受け取った
//! 値をそのまま運ぶだけで、`bool` から `ObservedOpenValue` を新規構築する経路は
//! 持たない。

use super::observation_store::{FocusProbeOpenStatus, ObservedOpenValue};

/// [`plan_focus_probe`] の決定結果。呼び出し元はこれを見て実際の書き込み
/// （`write_focus_probe` / `release_detect_state_guard_if`）を行う薄いインタプリタになる。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FocusProbeEffect {
    /// 観測を記録する。`release_guard` が `true` なら合わせて
    /// `release_detect_state_guard_if(true)` を呼ぶ（`effective.get()` と同値）。
    Record {
        effective: ObservedOpenValue,
        release_guard: bool,
    },
    /// grace 期間中の `false` 観測のため記録しない（reason はログ用）。
    /// guard には一切触れない。
    Suppressed { reason: &'static str },
    /// プロファイルが IMM32 open status を読めないため観測不能。記録は一切
    /// 行わないが、`release_guard` の条件でだけ guard 解除を試みる。
    NotObservable { release_guard: bool },
}

/// `FocusProbeOpenStatus` から [`FocusProbeEffect`] を決定する（挙動不変抽出）。
///
/// - `status`: `FocusProbeOpenStatus::classify` の結果。
/// - `is_japanese_ime`: `probe.is_japanese_ime`（`effective()` の合成に使う）。
/// - `grace_any`: `FocusProbeGraceFlags::any()`（warmup/GJI I/O grace のいずれかが
///   有効か）。
/// - `grace_primary_reason`: `FocusProbeGraceFlags::primary_reason()`。`grace_any`
///   が `false` のときは参照されない（呼び出し元は常に計算して渡してよい——
///   `primary_reason()` 自体は純粋・低コスト）。
/// - `shadow_on`: `NotObservable` 分岐でのみ使う shadow fallback 値
///   （`effective_open()`）。
#[must_use]
pub(crate) fn plan_focus_probe(
    status: FocusProbeOpenStatus,
    is_japanese_ime: bool,
    grace_any: bool,
    grace_primary_reason: &'static str,
    shadow_on: bool,
) -> FocusProbeEffect {
    match status {
        FocusProbeOpenStatus::Read(on) => {
            let effective = on.effective(is_japanese_ime);
            if !effective.get() && grace_any {
                FocusProbeEffect::Suppressed {
                    reason: grace_primary_reason,
                }
            } else {
                FocusProbeEffect::Record {
                    effective,
                    release_guard: effective.get(),
                }
            }
        }
        FocusProbeOpenStatus::NotObservable(_profile) => FocusProbeEffect::NotObservable {
            release_guard: is_japanese_ime && shadow_on,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::focus::class_names::AppImeProfile;

    fn read(on: bool) -> FocusProbeOpenStatus {
        let status = FocusProbeOpenStatus::classify(Some(on), AppImeProfile::Standard);
        assert!(matches!(status, FocusProbeOpenStatus::Read(_)));
        status
    }

    /// テストの期待値を組み立てる唯一の合法経路。`ObservedOpenValue` はフィールド
    /// private のため（ADR-106 決定2、BUG-92）、`bool` から直接構築できない——
    /// `FocusProbeOpenStatus::classify` の `Read` 分岐 → `.effective()` を経由する。
    fn expected_effective(on: bool, is_japanese_ime: bool) -> ObservedOpenValue {
        let FocusProbeOpenStatus::Read(value) = read(on) else {
            unreachable!("Standard + Some(on) は必ず Read になる")
        };
        value.effective(is_japanese_ime)
    }

    #[test]
    fn standard_ime_on_true_records_and_releases_guard() {
        let effect = plan_focus_probe(read(true), true, false, "gji-io", false);
        assert_eq!(
            effect,
            FocusProbeEffect::Record {
                effective: expected_effective(true, true),
                release_guard: true,
            }
        );
    }

    #[test]
    fn standard_ime_on_true_is_not_suppressed_during_grace() {
        // effective=true は `!effective.get()` が false になるため、grace_any の
        // 値に関わらず Suppressed には倒れない。
        let effect = plan_focus_probe(read(true), true, true, "warmup", false);
        assert_eq!(
            effect,
            FocusProbeEffect::Record {
                effective: expected_effective(true, true),
                release_guard: true,
            }
        );
    }

    #[test]
    fn standard_ime_on_false_during_grace_is_suppressed() {
        for reason in ["warmup", "gji-io"] {
            let effect = plan_focus_probe(read(false), true, true, reason, false);
            assert_eq!(
                effect,
                FocusProbeEffect::Suppressed { reason },
                "reason={reason}"
            );
        }
    }

    #[test]
    fn standard_ime_on_false_without_grace_records_without_releasing_guard() {
        let effect = plan_focus_probe(read(false), true, false, "gji-io", false);
        assert_eq!(
            effect,
            FocusProbeEffect::Record {
                effective: expected_effective(false, true),
                release_guard: false,
            }
        );
    }

    #[test]
    fn standard_ime_on_true_with_non_japanese_ime_collapses_to_false() {
        // on=true でも is_japanese_ime=false なら effective() が false へ合成する。
        let effect = plan_focus_probe(read(true), false, false, "gji-io", false);
        assert_eq!(
            effect,
            FocusProbeEffect::Record {
                effective: expected_effective(true, false),
                release_guard: false,
            }
        );
    }

    #[test]
    fn not_observable_releases_guard_only_when_japanese_and_shadow_on() {
        for profile in [AppImeProfile::TsfNative, AppImeProfile::Imm32Unavailable] {
            for is_japanese_ime in [true, false] {
                for shadow_on in [true, false] {
                    let status = FocusProbeOpenStatus::classify(Some(false), profile);
                    assert!(matches!(status, FocusProbeOpenStatus::NotObservable(_)));
                    let effect =
                        plan_focus_probe(status, is_japanese_ime, false, "gji-io", shadow_on);
                    assert_eq!(
                        effect,
                        FocusProbeEffect::NotObservable {
                            release_guard: is_japanese_ime && shadow_on,
                        },
                        "profile={profile:?} is_japanese_ime={is_japanese_ime} shadow_on={shadow_on}"
                    );
                    assert!(
                        !matches!(effect, FocusProbeEffect::Record { .. }),
                        "NotObservable status は絶対に Record にならない: profile={profile:?}"
                    );
                }
            }
        }
    }

    /// 入力空間の全数表（3 probe_ime_on × 3 profile × 2 is_japanese_ime × 2 grace_any ×
    /// 2 shadow_on = 72 ケース）。ただし `classify()` 後の挙動は実質5状態
    /// （`Read(true)` / `Read(false)` / `NotObservable`×3profile）に潰れるため、
    /// これは「挙動の全数」ではなく「入力の全数」の網羅であることに注意。
    /// `grace_primary_reason` 軸はここでは固定（`Suppressed` のペイロードにしか
    /// 効かないため）——`standard_ime_on_false_during_grace_is_suppressed` が
    /// reason の伝播を別途カバーする。
    #[test]
    fn plan_focus_probe_matrix() {
        const GRACE_REASON: &str = "warmup";
        for probe_ime_on in [Some(true), Some(false), None] {
            for profile in [
                AppImeProfile::Standard,
                AppImeProfile::Imm32Unavailable,
                AppImeProfile::TsfNative,
            ] {
                for is_japanese_ime in [true, false] {
                    for grace_any in [true, false] {
                        for shadow_on in [true, false] {
                            let status = FocusProbeOpenStatus::classify(probe_ime_on, profile);
                            let effect = plan_focus_probe(
                                status,
                                is_japanese_ime,
                                grace_any,
                                GRACE_REASON,
                                shadow_on,
                            );
                            let label = format!(
                                "probe_ime_on={probe_ime_on:?} profile={profile:?} \
                                 is_japanese_ime={is_japanese_ime} grace_any={grace_any} \
                                 shadow_on={shadow_on}"
                            );
                            match status {
                                FocusProbeOpenStatus::Read(on) => {
                                    // `.effective()` を経由せず素の bool 演算で独立に
                                    // 期待値を組み立てる（同じコードパスの二重化を避ける）。
                                    let effective_bool = on.get() && is_japanese_ime;
                                    if !effective_bool && grace_any {
                                        assert_eq!(
                                            effect,
                                            FocusProbeEffect::Suppressed {
                                                reason: GRACE_REASON
                                            },
                                            "{label}"
                                        );
                                    } else {
                                        assert_eq!(
                                            effect,
                                            FocusProbeEffect::Record {
                                                effective: on.effective(is_japanese_ime),
                                                release_guard: effective_bool,
                                            },
                                            "{label}"
                                        );
                                    }
                                }
                                FocusProbeOpenStatus::NotObservable(_) => {
                                    assert_eq!(
                                        effect,
                                        FocusProbeEffect::NotObservable {
                                            release_guard: is_japanese_ime && shadow_on,
                                        },
                                        "{label}"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
