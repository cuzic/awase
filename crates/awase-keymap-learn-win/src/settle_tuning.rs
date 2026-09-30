//! `settle()`と`clear_edit()`の待ち時間の設定。
//!
//! 既定値は windows-latest(MS-IME本体)のA/B実測で決めた(各3回、run 36650865715):
//! 1押下あたり68〜77ms→44〜53ms(約33%短縮)、検証精度は平均0.967で同等、学習表の
//! 差は実行間ばらつき(0〜4セル/143)の範囲内。従来値(すべて40ms)へ戻して比較する
//! ための診断用フラグ(`--quiet-after-change-ms=N`、`--clear-edit-pump-ms=N`)も残す。
//!
//! OS非依存なのでLinuxでもユニットテストできる。

/// 変化なし確定の静止待ち(ms)。従来の固定値のまま。
pub const DEFAULT_QUIET_MS: u64 = 40;
/// 変化検出後の静止待ち(ms)。実測: 変化つきsettle約2800回で再変化0件
/// (`hist=inter_change`が全て0)。40msは待ち過ぎだったため10msへ。
pub const DEFAULT_QUIET_AFTER_CHANGE_MS: u64 = 10;
/// `clear_edit()`後のpump(ms)。実測: pump中にIME状態が変わった回数が約700回で0件。
pub const DEFAULT_CLEAR_EDIT_PUMP_MS: u64 = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SettleTuning {
    /// 最後の変化から、この時間変化が無ければ静止と見なす(既に1回以上変化した場合)。
    /// 実測(windows-latest MS-IME本体、3回反復): 変化つきsettle約2800回で、最初の
    /// 変化の後にもう一度変化した例は0件だった(`hist=inter_change`が全て0)。
    pub quiet_after_change_ms: u64,
    /// settle開始から、一度も変化が無いまま「変化なし」と確定するまでの待ち。
    /// 最初の変化が10〜40msで届く例が約6%あるため、短縮しない。
    pub quiet_no_change_ms: u64,
    /// `clear_edit()`の後に回すメッセージポンプの長さ。
    pub clear_edit_pump_ms: u64,
}

impl Default for SettleTuning {
    fn default() -> Self {
        Self {
            quiet_after_change_ms: DEFAULT_QUIET_AFTER_CHANGE_MS,
            quiet_no_change_ms: DEFAULT_QUIET_MS,
            clear_edit_pump_ms: DEFAULT_CLEAR_EDIT_PUMP_MS,
        }
    }
}

impl SettleTuning {
    /// コマンドライン引数から診断用の上書きを読む。不正な値は無視して既定値を使う。
    #[must_use]
    pub fn from_args<I, S>(args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut t = Self::default();
        for arg in args {
            let arg = arg.as_ref();
            if let Some(v) = parse_flag(arg, "--quiet-after-change-ms=") {
                t.quiet_after_change_ms = v;
            } else if let Some(v) = parse_flag(arg, "--clear-edit-pump-ms=") {
                t.clear_edit_pump_ms = v;
            }
        }
        t
    }
}

fn parse_flag(arg: &str, prefix: &str) -> Option<u64> {
    arg.strip_prefix(prefix)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_shortens_only_the_waits_that_measurements_showed_unneeded() {
        let t = SettleTuning::default();
        assert_eq!(
            (
                t.quiet_after_change_ms,
                t.quiet_no_change_ms,
                t.clear_edit_pump_ms
            ),
            (10, 40, 5)
        );
    }

    #[test]
    fn flags_override_only_their_own_field() {
        let t =
            SettleTuning::from_args(["x", "--quiet-after-change-ms=10", "--clear-edit-pump-ms=5"]);
        assert_eq!(t.quiet_after_change_ms, 10);
        assert_eq!(t.clear_edit_pump_ms, 5);
        assert_eq!(
            t.quiet_no_change_ms, 40,
            "変化なしの待ちはフラグで動かせない"
        );
    }

    #[test]
    fn invalid_values_fall_back_to_default() {
        let t = SettleTuning::from_args(["--quiet-after-change-ms=abc", "--clear-edit-pump-ms="]);
        assert_eq!(t, SettleTuning::default());
    }

    #[test]
    fn flags_can_restore_the_previous_fixed_values_for_ab_comparison() {
        let t = SettleTuning::from_args(["--quiet-after-change-ms=40", "--clear-edit-pump-ms=40"]);
        assert_eq!(
            (
                t.quiet_after_change_ms,
                t.quiet_no_change_ms,
                t.clear_edit_pump_ms
            ),
            (40, 40, 40)
        );
    }
}
