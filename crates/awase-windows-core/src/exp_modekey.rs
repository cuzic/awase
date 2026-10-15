//! 使い捨てのスパイク(ブランチ `exp/mode-key-recheck`、develop にはマージしない)。
//!
//! 不具合報告 01M4QSFH1B の「英数を押しても入力モードのずれが解消しない」への対策案 A・B を、環境変数
//! `AWASE_EXP_MODEKEY` で切り替えて GitHub Actions(`sc-exp-modekey-*`)で比べる。カンマ区切り。
//! - `a`: 打鍵が止まった後に1回だけ実 IME を読み直し、予測した入力モードと食い違えば観測を採る(窓内は従来どおり予測に任せる)。
//! - `b1`: 読めない窓で、状態依存のトグルキー(英数など)の予測の入力モードを適用せず、追跡(`track.conv`)も捨てる。
//! - `b2`: 同じ条件で、入力モードを `Unknown` に戻す。
//! 未設定・空は従来の挙動。

use std::sync::OnceLock;

/// 実験フラグ(プロセス起動時に1回だけ読む)。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ExpModeKey {
    pub recheck_after_idle: bool,
    pub toggle_guard_keep: bool,
    pub toggle_guard_unknown: bool,
}

/// 環境変数の文字列を解釈する(単体テスト用に分離)。
#[must_use]
pub fn parse(s: &str) -> ExpModeKey {
    let mut f = ExpModeKey::default();
    for t in s.split(',').map(str::trim) {
        match t {
            "a" => f.recheck_after_idle = true,
            "b1" => f.toggle_guard_keep = true,
            "b2" => f.toggle_guard_unknown = true,
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
        assert_eq!(parse("zzz"), ExpModeKey::default());
    }
}
