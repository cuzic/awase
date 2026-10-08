//! 外部から IME の開閉が変えられたことを、読めない窓（`Imm32Unavailable`）で観測する監視窓の状態機械
//! （ADR-205、BUG-172）。
//!
//! 外部プロセスが注入した IME キー（目印なし）の直後だけ短い窓を開き、その窓の中で prefetch 済みの開閉の読み
//! （`IMC_GETOPENSTATUS`）が「基準値」から変わったことを観測したときだけ、実状態への追随を求める。
//! 「Chrome は常に 0」のような環境では読みが変わらないので何も起きない（偽の OFF を採用しない）。
//! Win32 に依存しない（スコープ `S` を外から渡す）ので `#[cfg(windows)]` 外でユニットテストできる。

/// 窓の延長の上限（最初の arm から数えて、窓の何倍まで延ばすか）。注入が続く環境で窓が切れなくなるのを防ぐ。
const MAX_EXTENSION_FACTOR: u64 = 2;

/// 監視窓の判定結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeVerdict {
    /// 窓が無い・切れた・スコープ違い・読めなかった・基準値と同じ。何もしない。
    NoEvidence,
    /// 窓の中で基準値と違う値を読んだ。実状態がこの値へ変わったので追随する。
    Changed(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Armed<S> {
    scope: S,
    first_arm_ms: u64,
    last_arm_ms: u64,
    /// 基準値。arm 前の直近の読み（同じスコープ）か、窓の中の最初の読み。
    baseline: Option<bool>,
    /// ADR-188: 物理のモードキー通過／FSM 再送出で開いた「直接観測」の窓。基準値を使わず、窓内の読みを belief と
    /// 照合する（`observe` の基準値照合は行わず、窓も閉じない）。外部注入（ADR-205）と重なったら Direct が勝つ。
    direct: bool,
}

/// 外部変化の監視窓と、直近の読みの記録。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExternalChangeWatch<S: Copy + PartialEq> {
    armed: Option<Armed<S>>,
    /// 全 refresh の入口で記録する直近の読み（スコープ付き）。基準値の初期値になる。
    last_read: Option<(S, bool)>,
}

impl<S: Copy + PartialEq> ExternalChangeWatch<S> {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            armed: None,
            last_read: None,
        }
    }

    /// 直近の読みを記録する（`None`〈読めなかった〉は記録しない）。`observe` の後に呼ぶ。
    pub fn record_read(&mut self, scope: S, read: Option<bool>) {
        if let Some(v) = read {
            self.last_read = Some((scope, v));
        }
    }

    /// 外部注入の IME キーを見たら呼ぶ。同じスコープの窓が生きていれば基準値を保ったまま延ばす
    /// （延長は最初の arm から `window_ms * 2` までで、窓の寿命は最大でその時点から `window_ms` 後＝`window_ms * 3`）。そうでなければ新しく開き、基準値は直近の読み（同じスコープ）。
    pub fn arm(&mut self, scope: S, now_ms: u64, window_ms: u64) {
        self.arm_with(scope, now_ms, window_ms, false);
    }

    /// ADR-188: 物理のモードキー通過／FSM 再送出で直接観測の窓を開く。生きている同じスコープの窓があれば Direct に
    /// 格上げして延ばす（基準値は保つ。Direct の窓は基準値を使わない）。
    pub fn arm_direct(&mut self, scope: S, now_ms: u64, window_ms: u64) {
        self.arm_with(scope, now_ms, window_ms, true);
    }

    fn arm_with(&mut self, scope: S, now_ms: u64, window_ms: u64, direct: bool) {
        if let Some(a) = self.armed.as_mut() {
            let alive = a.scope == scope && now_ms.saturating_sub(a.last_arm_ms) <= window_ms;
            if alive {
                let cap = a
                    .first_arm_ms
                    .saturating_add(window_ms.saturating_mul(MAX_EXTENSION_FACTOR));
                a.last_arm_ms = now_ms.min(cap);
                a.direct |= direct;
                return;
            }
        }
        let baseline = self.last_read.filter(|(s, _)| *s == scope).map(|(_, v)| v);
        self.armed = Some(Armed {
            scope,
            first_arm_ms: now_ms,
            last_arm_ms: now_ms,
            baseline,
            direct,
        });
    }

    /// 直接観測の窓（ADR-188）が生きているか（消費しない）。スコープ違い・切れた窓は破棄して `false`。
    pub fn direct_live(&mut self, scope: S, now_ms: u64, window_ms: u64) -> bool {
        self.live(scope, now_ms, window_ms) && self.armed.is_some_and(|a| a.direct)
    }

    /// 開いている窓の最後の arm 時刻（ms）。窓が無ければ `None`。awase 自身の書き込みがこの時刻以後にあったかの
    /// 比較に使う（ADR-188 R3）。
    #[must_use]
    pub fn last_arm_ms(&self) -> Option<u64> {
        self.armed.map(|a| a.last_arm_ms)
    }

    /// 開いている窓の基準値(ログ用)。窓が無い・基準値が無いなら `None`。
    #[must_use]
    pub fn baseline(&self) -> Option<bool> {
        self.armed.and_then(|a| a.baseline)
    }

    /// 窓が生きているか（消費しない）。スコープが変わった・窓が切れたなら破棄して `false`。
    pub fn live(&mut self, scope: S, now_ms: u64, window_ms: u64) -> bool {
        self.remaining_ms(scope, now_ms, window_ms).is_some()
    }

    /// 窓の残り時間（ms）。無い・切れた・スコープ違いなら `None`（破棄する）。
    pub fn remaining_ms(&mut self, scope: S, now_ms: u64, window_ms: u64) -> Option<u64> {
        let a = self.armed?;
        let age = now_ms.saturating_sub(a.last_arm_ms);
        if a.scope != scope || age > window_ms {
            self.armed = None;
            return None;
        }
        Some(window_ms - age)
    }

    /// prefetch 済みの読みを判定する。窓の中で基準値と違う値を読んだら `Changed`（窓を閉じる）。
    /// 基準値が無ければ最初の読みを基準値にする（変化とは扱わない）。
    pub fn observe(
        &mut self,
        scope: S,
        now_ms: u64,
        window_ms: u64,
        read: Option<bool>,
    ) -> ChangeVerdict {
        if !self.live(scope, now_ms, window_ms) {
            return ChangeVerdict::NoEvidence;
        }
        let Some(v) = read else {
            return ChangeVerdict::NoEvidence;
        };
        let Some(a) = self.armed.as_mut() else {
            return ChangeVerdict::NoEvidence;
        };
        if a.direct {
            // ADR-188: 直接観測の窓は基準値で照合せず、窓も閉じない（`classify_direct_read` が belief と照合する）。
            return ChangeVerdict::NoEvidence;
        }
        match a.baseline {
            None => {
                a.baseline = Some(v);
                ChangeVerdict::NoEvidence
            }
            Some(b) if b == v => ChangeVerdict::NoEvidence,
            Some(_) => {
                self.armed = None;
                ChangeVerdict::Changed(v)
            }
        }
    }
}

/// `IME_CMODE_NATIVE`（conversion mode の bit0）。日本語入力（かな・カタカナ）なら立ち、半角英数なら立たない。
pub const IME_CMODE_NATIVE: u32 = 0x0001;

/// 直接観測（ADR-188）の追随判定。`None` の軸は追随しない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirectFollow {
    /// 実状態の開閉が belief と違うとき、その値。
    pub open: Option<bool>,
    /// 実状態の英数かどうかが belief と違うとき、その値（`true` = 英数、`false` = 日本語入力＝ローマ字想定）。
    pub eisu: Option<bool>,
}

impl DirectFollow {
    /// 追随する軸が無いか。
    #[must_use]
    pub const fn is_none(&self) -> bool {
        self.open.is_none() && self.eisu.is_none()
    }
}

/// 窓内の読み（`read_open`・`read_conv`）を belief（`belief_open`・`belief_eisu`）と照合し、追随する軸を返す純関数。
///
/// - 開閉は `read_open` が belief と違うときだけ。
/// - 英数かどうかは **NATIVE ビットだけ**で決める。実 Chrome×GJI の ROMAN ビットは当てにならない（計測: 実際は
///   ローマ字入力なのに `conv=9`〈ROMAN なし〉が 300ms 続いた、ADR-188 R2）ので見ない。ここから `ObservedKana`/
///   `ObservedRomaji` は作らない（呼び出し側は `ObservedEisu` か `AssumedRomaji` のみ）。
/// - 閉じている（`read_open == Some(false)`）とき、または開閉が読めないときは conv を見ない（閉じた IME の conv は意味が無い）。
#[must_use]
pub fn classify_direct_read(
    read_open: Option<bool>,
    read_conv: Option<u32>,
    belief_open: bool,
    belief_eisu: bool,
) -> DirectFollow {
    let open = read_open.filter(|o| *o != belief_open);
    let eisu = match (read_open, read_conv) {
        (Some(true), Some(conv)) => {
            let want_eisu = conv & IME_CMODE_NATIVE == 0;
            (want_eisu != belief_eisu).then_some(want_eisu)
        }
        _ => None,
    };
    DirectFollow { open, eisu }
}

/// 直接観測（ADR-188）を適用する窓の IME 種別（ADR-244 D2）。`Imm32Unavailable` の窓で、GJI または同定済みの
/// Microsoft IME 本体（`table_ime_kind()`）のときだけ `Some`。ATOK・第三者 IME・IMM32 HKL のみ・起動直後の未検出
/// (`table_ime_kind() == None`)と、`Imm32Unavailable` 以外の窓は `None`。
///
/// ADR-205 の外部変化の監視（開閉の基準値を持つ）は GJI 限定のまま——こちらは基準値を持たず窓内の現在値だけを見る。
#[must_use]
pub const fn direct_watch_kind(
    imm32_unavailable: bool,
    table_kind: Option<crate::state::ime_kind::ImeKindId>,
) -> Option<crate::state::ime_kind::ImeKindId> {
    if imm32_unavailable {
        table_kind
    } else {
        None
    }
}

/// [`classify_direct_read`] に IME 種別の軸の絞り込みを足したもの（ADR-244 D4）。
///
/// - GJI: 開閉・英数の両軸（ADR-188 のとおり）。
/// - Microsoft IME 本体: **英数の軸だけ**。MS-IME × 実 Chrome の開閉の読みは「開いているのに 0」が観測された
///   ことがある（ADR-205）ので、トグル中に閉と誤読して追随し Engine が OFF のまま残る失敗を踏まない。
#[must_use]
pub fn classify_direct_read_for(
    kind: crate::state::ime_kind::ImeKindId,
    read_open: Option<bool>,
    read_conv: Option<u32>,
    belief_open: bool,
    belief_eisu: bool,
) -> DirectFollow {
    use crate::state::ime_kind::ImeKindId;
    let follow = classify_direct_read(read_open, read_conv, belief_open, belief_eisu);
    match kind {
        ImeKindId::Gji => follow,
        ImeKindId::MsIme => DirectFollow {
            open: None,
            eisu: follow.eisu,
        },
    }
}

impl<S: Copy + PartialEq> Default for ExternalChangeWatch<S> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: u64 = 300;

    fn armed_after_open_read() -> ExternalChangeWatch<u32> {
        let mut w = ExternalChangeWatch::<u32>::new();
        w.record_read(1, Some(true));
        w.arm(1, 1000, W);
        w
    }

    /// 第0段の実測（run 36545236017）: 注入の 32ms 後の最初の読みが既に 0。arm 前の直近の 1 が基準になり Changed(false)。
    #[test]
    fn first_read_already_closed_after_prior_open_read_is_a_change() {
        let mut w = armed_after_open_read();
        assert_eq!(
            w.observe(1, 1032, W, Some(false)),
            ChangeVerdict::Changed(false)
        );
    }

    /// 「常に 0」の環境: 直近の読みも 0 なので変化ではない。
    #[test]
    fn constant_zero_environment_never_changes() {
        let mut w = ExternalChangeWatch::<u32>::new();
        w.record_read(1, Some(false));
        w.arm(1, 1000, W);
        assert_eq!(
            w.observe(1, 1032, W, Some(false)),
            ChangeVerdict::NoEvidence
        );
        assert_eq!(
            w.observe(1, 1092, W, Some(false)),
            ChangeVerdict::NoEvidence
        );
    }

    /// 直近の読みが無いとき、窓内の最初の読みが基準値。その後の変化を拾う。
    #[test]
    fn without_prior_read_first_in_window_read_is_baseline() {
        let mut w = ExternalChangeWatch::<u32>::new();
        w.arm(1, 1000, W);
        assert_eq!(w.observe(1, 1020, W, Some(true)), ChangeVerdict::NoEvidence);
        assert_eq!(
            w.observe(1, 1080, W, Some(false)),
            ChangeVerdict::Changed(false)
        );
    }

    /// 開く方向（0→1）も同じ規則で拾う。
    #[test]
    fn opening_direction_is_also_a_change() {
        let mut w = ExternalChangeWatch::<u32>::new();
        w.record_read(1, Some(false));
        w.arm(1, 1000, W);
        assert_eq!(
            w.observe(1, 1040, W, Some(true)),
            ChangeVerdict::Changed(true)
        );
    }

    #[test]
    fn expired_window_and_scope_change_yield_no_evidence() {
        let mut w = armed_after_open_read();
        assert_eq!(
            w.observe(1, 1000 + W + 1, W, Some(false)),
            ChangeVerdict::NoEvidence
        );
        let mut w = armed_after_open_read();
        assert_eq!(
            w.observe(2, 1010, W, Some(false)),
            ChangeVerdict::NoEvidence
        );
        // スコープ違いで窓は破棄済み
        assert_eq!(
            w.observe(1, 1020, W, Some(false)),
            ChangeVerdict::NoEvidence
        );
    }

    /// 別スコープの直近の読みは基準値にしない（別窓の値を持ち込まない）。
    #[test]
    fn last_read_from_another_scope_is_not_a_baseline() {
        let mut w = ExternalChangeWatch::<u32>::new();
        w.record_read(9, Some(true));
        w.arm(1, 1000, W);
        assert_eq!(
            w.observe(1, 1030, W, Some(false)),
            ChangeVerdict::NoEvidence
        );
    }

    #[test]
    fn unreadable_read_yields_no_evidence_and_keeps_window() {
        let mut w = armed_after_open_read();
        assert_eq!(w.observe(1, 1030, W, None), ChangeVerdict::NoEvidence);
        assert_eq!(
            w.observe(1, 1090, W, Some(false)),
            ChangeVerdict::Changed(false)
        );
    }

    /// 連続 arm: 同じスコープなら基準値を保ったまま延ばす。延長には上限がある。
    #[test]
    fn re_arm_keeps_baseline_and_extension_is_capped() {
        let mut w = armed_after_open_read();
        w.arm(1, 1200, W); // 0xF0 up + 0xF2 down のような連続注入
        assert_eq!(
            w.observe(1, 1400, W, Some(false)),
            ChangeVerdict::Changed(false),
            "1000 に arm、1200 に再 arm → 窓は 1500 まで。基準値の 1 は保たれる"
        );
        // 上限: 最初の arm(1000)から 2W(=600) を超えて延ばせない
        let mut w = armed_after_open_read();
        for t in [1250, 1500, 1750] {
            w.arm(1, t, W);
        }
        assert_eq!(
            w.remaining_ms(1, 1950, W),
            None,
            "延長は最初の arm から 2W(=1600)まで。窓は 1900 で切れる"
        );
    }

    #[test]
    fn same_value_reads_do_not_close_the_window() {
        let mut w = armed_after_open_read();
        assert_eq!(w.observe(1, 1020, W, Some(true)), ChangeVerdict::NoEvidence);
        assert_eq!(
            w.observe(1, 1100, W, Some(false)),
            ChangeVerdict::Changed(false)
        );
        // Changed で窓は閉じる
        assert_eq!(w.observe(1, 1120, W, Some(true)), ChangeVerdict::NoEvidence);
    }

    // ── ADR-188: 直接観測の窓 ──

    #[test]
    fn direct_window_never_closes_and_baseline_observe_is_skipped() {
        let mut w = ExternalChangeWatch::<u32>::new();
        w.record_read(1, Some(true));
        w.arm_direct(1, 1000, W);
        assert!(w.direct_live(1, 1030, W));
        // 基準値(true)と違う読みでも、Direct の窓では ADR-205 の差分照合は行わず窓も閉じない。
        assert_eq!(
            w.observe(1, 1032, W, Some(false)),
            ChangeVerdict::NoEvidence
        );
        assert!(w.direct_live(1, 1100, W));
        assert!(w.direct_live(1, 1290, W));
    }

    #[test]
    fn baseline_window_is_not_direct_and_direct_wins_when_both_armed() {
        let mut w = armed_after_open_read();
        assert!(!w.direct_live(1, 1010, W));
        w.arm_direct(1, 1100, W);
        assert!(
            w.direct_live(1, 1110, W),
            "生きている窓へ Direct を足すと格上げ"
        );
        // 後から Baseline の arm が来ても Direct のまま
        w.arm(1, 1150, W);
        assert!(w.direct_live(1, 1160, W));
    }

    #[test]
    fn direct_window_expires_and_respects_scope() {
        let mut w = ExternalChangeWatch::<u32>::new();
        w.arm_direct(1, 1000, W);
        assert!(!w.direct_live(2, 1010, W), "スコープ違いは破棄");
        let mut w = ExternalChangeWatch::<u32>::new();
        w.arm_direct(1, 1000, W);
        assert!(!w.direct_live(1, 1000 + W + 1, W));
        assert_eq!(w.last_arm_ms(), None, "切れた窓は破棄済み");
    }

    #[test]
    fn last_arm_ms_tracks_re_arm() {
        let mut w = ExternalChangeWatch::<u32>::new();
        assert_eq!(w.last_arm_ms(), None);
        w.arm_direct(1, 1000, W);
        assert_eq!(w.last_arm_ms(), Some(1000));
        w.arm_direct(1, 1100, W);
        assert_eq!(w.last_arm_ms(), Some(1100));
    }

    /// R2: NATIVE ビットだけで英数を決める。計測の実値: 直接入力→無変換=かな ON で `open=true conv=9`(ROMAN なし)が
    /// 300ms 続いたが、実際はローマ字入力。ROMAN が無くても英数とは見なさない。
    #[test]
    fn classify_uses_native_bit_only() {
        assert!(classify_direct_read(Some(true), Some(9), true, false).is_none());
        assert!(classify_direct_read(Some(true), Some(25), true, false).is_none());
        assert_eq!(
            classify_direct_read(Some(true), Some(16), true, false),
            DirectFollow {
                open: None,
                eisu: Some(true)
            }
        );
        assert_eq!(
            classify_direct_read(Some(true), Some(25), true, true),
            DirectFollow {
                open: None,
                eisu: Some(false)
            }
        );
        // 全角カタカナ(0x13、NATIVE あり)も英数ではない
        assert!(classify_direct_read(Some(true), Some(0x13), true, false).is_none());
    }

    #[test]
    fn classify_ignores_conv_when_closed_or_unreadable() {
        assert_eq!(
            classify_direct_read(Some(false), Some(16), true, false),
            DirectFollow {
                open: Some(false),
                eisu: None
            }
        );
        assert!(classify_direct_read(Some(false), Some(16), false, false).is_none());
        assert!(classify_direct_read(None, Some(16), true, false).is_none());
        assert_eq!(
            classify_direct_read(Some(false), None, true, false),
            DirectFollow {
                open: Some(false),
                eisu: None
            }
        );
        assert!(classify_direct_read(Some(true), None, true, false).is_none());
    }

    /// M6: 1 回の読みで開閉と英数の両軸を追随する(閉→開・英数)。
    #[test]
    fn classify_follows_both_axes_at_once() {
        assert_eq!(
            classify_direct_read(Some(true), Some(16), false, false),
            DirectFollow {
                open: Some(true),
                eisu: Some(true)
            }
        );
    }

    /// ADR-244 D2: 直接観測の窓は Imm32Unavailable かつ GJI／同定済み MS-IME 本体だけ。
    #[test]
    fn direct_watch_kind_requires_imm32_unavailable_and_a_table_ime() {
        use crate::state::ime_kind::ImeKindId;
        assert_eq!(direct_watch_kind(true, Some(ImeKindId::Gji)), Some(ImeKindId::Gji));
        assert_eq!(direct_watch_kind(true, Some(ImeKindId::MsIme)), Some(ImeKindId::MsIme));
        // ATOK・第三者 IME・IMM32 HKL のみ・起動直後の未検出は `table_ime_kind() == None`。
        assert_eq!(direct_watch_kind(true, None), None);
        // Imm32Unavailable 以外の窓（Win32 EDIT、TsfNative 等）は対象外。
        assert_eq!(direct_watch_kind(false, Some(ImeKindId::Gji)), None);
        assert_eq!(direct_watch_kind(false, Some(ImeKindId::MsIme)), None);
    }

    /// ADR-244 D4: MS-IME 本体は英数の軸だけ。閉の読みでは何も追随しない（「開いているのに 0」の型を踏まない）。
    #[test]
    fn ms_ime_native_follows_only_the_eisu_axis() {
        use crate::state::ime_kind::ImeKindId;
        // トグル中(belief: 開・英数)に変換/英数/ひらがなでかなへ戻った: NATIVE の読み → 英数を外す。
        assert_eq!(
            classify_direct_read_for(ImeKindId::MsIme, Some(true), Some(25), true, true),
            DirectFollow { open: None, eisu: Some(false) }
        );
        // 閉の読み(無変換): 開閉の軸は採らない。英数の軸も conv を見ないので None。
        assert!(classify_direct_read_for(ImeKindId::MsIme, Some(false), Some(0), true, true).is_none());
        // belief が閉で読みが開・NATIVE のとき、開閉の軸は採らず英数の軸だけ（belief は英数でない → なし）。
        assert!(classify_direct_read_for(ImeKindId::MsIme, Some(true), Some(25), false, false).is_none());
        // 開で半角英数を読んだら英数を採る(軸の絞り込みは英数を妨げない)。
        assert_eq!(
            classify_direct_read_for(ImeKindId::MsIme, Some(true), Some(16), true, false),
            DirectFollow { open: None, eisu: Some(true) }
        );
    }

    /// ADR-244 D4: GJI は従来どおり両軸（ADR-188 の挙動を変えない）。
    #[test]
    fn gji_keeps_both_axes() {
        use crate::state::ime_kind::ImeKindId;
        assert_eq!(
            classify_direct_read_for(ImeKindId::Gji, Some(false), Some(25), true, false),
            classify_direct_read(Some(false), Some(25), true, false)
        );
        assert_eq!(
            classify_direct_read_for(ImeKindId::Gji, Some(true), Some(16), false, false),
            DirectFollow { open: Some(true), eisu: Some(true) }
        );
    }
}
