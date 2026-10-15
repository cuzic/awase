//! 使い捨てのスパイク(ブランチ `exp/mode-key-recheck`、develop にはマージしない)。
//!
//! 不具合報告 01M4QSFH1B の「英数を押しても入力モードのずれが解消しない」への対策案 A・B を、環境変数
//! `AWASE_EXP_MODEKEY` で切り替えて GitHub Actions(`sc-exp-modekey-*`)で比べる。カンマ区切り。
//! - `a`: 打鍵が止まった後に1回だけ実 IME を読み直し、予測した入力モードと食い違えば観測を採る(窓内は従来どおり予測に任せる)。
//! - `b1`: 読めない窓で、状態依存のトグルキー(英数など)の予測の入力モードを適用せず、追跡(`track.conv`)も捨てる。
//! - `b2`: 同じ条件で、入力モードを `Unknown` に戻す(1周目で効果なし)。
//! - `z`: `a` と同じ読み直しを、入力モード(英数かどうか)の軸だけにする(開閉の読みは採らない。実 Chrome で `a` が悪化した対策)。
//! - `y`: 読めない窓で、古い HwndCache の ON を復元しない(押下より前の明示 OFF より古い・経過が [`Y_AGE_MS`] を超える)。
//! - `x`: 読めない窓の GJI で、打鍵の結果(GJI の候補ウィンドウが出たか)から実 IME が組み立て中かを推定し、belief を直す。
//! 未設定・空は従来の挙動。

use std::sync::OnceLock;

/// 実験フラグ(プロセス起動時に1回だけ読む)。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ExpModeKey {
    pub recheck_after_idle: bool,
    pub toggle_guard_keep: bool,
    pub toggle_guard_unknown: bool,
    pub recheck_eisu_only: bool,
    pub cache_age_guard: bool,
    pub typing_feedback: bool,
}

/// `y` の probe 値(ms)。CI の再現手順(フォーカスを外して戻す)の経過より短くして、機構が働くかを見る。本番値ではない。
pub const Y_AGE_MS: u64 = 200;
/// `x` の待ち時間(ms): 先頭の文字キーから、候補ウィンドウが出るかを見るまで。
pub const X_WAIT_MS: u64 = 600;

/// 環境変数の文字列を解釈する(単体テスト用に分離)。
#[must_use]
pub fn parse(s: &str) -> ExpModeKey {
    let mut f = ExpModeKey::default();
    for t in s.split(',').map(str::trim) {
        match t {
            "a" => f.recheck_after_idle = true,
            "b1" => f.toggle_guard_keep = true,
            "b2" => f.toggle_guard_unknown = true,
            "z" => f.recheck_eisu_only = true,
            "y" => f.cache_age_guard = true,
            "x" => f.typing_feedback = true,
            _ => {}
        }
    }
    f
}

#[must_use]
pub fn flags() -> ExpModeKey {
    static F: OnceLock<ExpModeKey> = OnceLock::new();
    *F.get_or_init(|| parse(&std::env::var("AWASE_EXP_MODEKEY").unwrap_or_default()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_comma_separated_flags() {
        assert_eq!(parse(""), ExpModeKey::default());
        assert!(parse("a").recheck_after_idle);
        let f = parse("a, b1");
        assert!(f.recheck_after_idle && f.toggle_guard_keep && !f.toggle_guard_unknown);
        assert!(parse("b2").toggle_guard_unknown);
        let f = parse("z,y,x");
        assert!(f.recheck_eisu_only && f.cache_age_guard && f.typing_feedback);
        assert_eq!(parse("zzz"), ExpModeKey::default());
    }
}
