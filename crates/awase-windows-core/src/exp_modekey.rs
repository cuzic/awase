//! 使い捨てのスパイク(ブランチ `exp/mode-key-recheck`、develop にはマージしない)。
//!
//! 不具合報告 01M4QSFH1B の「英数を押しても入力モードのずれが解消しない」への対策案 A・B を、環境変数
//! `AWASE_EXP_MODEKEY` で切り替えて GitHub Actions(`sc-exp-modekey-*`)で比べる。カンマ区切り。
//! - `a`: 打鍵が止まった後に1回だけ実 IME を読み直し、予測した入力モードと食い違えば観測を採る(窓内は従来どおり予測に任せる)。
//! - `b1`: 読めない窓で、状態依存のトグルキー(英数など)の予測の入力モードを適用せず、追跡(`track.conv`)も捨てる。
//! - `b2`: 同じ条件で、入力モードを `Unknown` に戻す(1周目で効果なし)。
//! - `z`: `a` と同じ読み直しを、入力モード(英数かどうか)の軸だけにする(開閉の読みは採らない。実 Chrome で `a` が悪化した対策)。
//! - `y`: 読めない窓で、古い HwndCache の ON を復元しない(押下より前の明示 OFF より古い・経過が [`Y_AGE_MS`] を超える)。
//! - `b1e`: `b1` と同じだが、英数キー(`TableKey::Eisu`)だけに限る(ATOK のひらがな・入力中の無変換に当てない)。
//! - `b3`: 直接観測の窓の「予測が付いたキーは読みを採らない」(ADR-188 追記6)を、予測が値を持つ軸だけにする(持たない軸の読みは採る)。
//! - `noread`: IME 状態の読み(`read_ime_state_full`)をすべて失敗させる。一太郎(`ImmGetDefaultIMEWnd`=NULL で読み自体が不可)の模擬。
//! - `b1o`: `b1e`(英数キーだけ)を、窓内の IME の読みが実際に効いている窓(直近 [`OBSERVABLE_MS`] に読みが成功した)だけに適用する。
//!   観測の無い窓では belief が追随せず、従来の予測は belief が正しい多数派で当たるので、そこでは B1 を入れない(4周目の noread の結果)。
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
    pub toggle_guard_eisu_only: bool,
    pub per_axis_follow: bool,
    pub no_read: bool,
    pub toggle_guard_observable: bool,
}

/// `y` の probe 値(ms)。CI の再現手順(フォーカスを外して戻す)の経過より短くして、機構が働くかを見る。本番値ではない。
pub const Y_AGE_MS: u64 = 200;
/// 読みが「効いている」とみなす期間(ms): 直近にこの期間内で `conversion_mode` の読みが成功したこと。
pub const OBSERVABLE_MS: u64 = 30_000;

static LAST_READ_OK_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn now_ms() -> u64 {
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    // 0 は「一度も読めていない」の意味にするので、1 から数える。
    u64::try_from(
        START
            .get_or_init(std::time::Instant::now)
            .elapsed()
            .as_millis(),
    )
    .unwrap_or(u64::MAX - 1)
        + 1
}

/// IME の読み(conv)が成功したことを記録する(`read_ime_state_full` が呼ぶ)。
pub fn note_read_ok() {
    LAST_READ_OK_MS.store(now_ms(), std::sync::atomic::Ordering::Relaxed);
}

/// 直近 [`OBSERVABLE_MS`] に読みが成功したか(一度も無ければ偽)。
#[must_use]
pub fn observable() -> bool {
    let last = LAST_READ_OK_MS.load(std::sync::atomic::Ordering::Relaxed);
    last != 0 && now_ms().saturating_sub(last) <= OBSERVABLE_MS
}

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
            "b1e" => f.toggle_guard_eisu_only = true,
            "b3" => f.per_axis_follow = true,
            "noread" => f.no_read = true,
            "b1o" => f.toggle_guard_observable = true,
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
