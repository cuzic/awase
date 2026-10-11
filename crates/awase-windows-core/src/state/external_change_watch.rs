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
    /// ADR-256（BUG-199）: 直接観測の窓の arm 時点でまだ IME に届いていない awase 自身の開閉の書き込み（書いた値）。
    /// 窓内で同じ値を読むまで、窓内の読みを採らない（[`direct_read_held_by_awase_write`]）。同じ値を読んだら `None` に戻す。
    /// 窓が切れたら窓ごと消える。
    awase_write: Option<bool>,
    /// ADR-256: `awase_write` のために、保持しなければ belief を動かしていた読みを採らなかった回数（ログ・計測用）。
    /// 窓が `awase_write` 未確認のまま切れたときにこの回数を出し、保持が利用者の操作（awase の書き込みと逆向き）を
    /// 窓の全体で止めた頻度を数える（ADR-256 の既知の限界）。
    effective_holds: u16,
}

/// awase 自身が SendInput で IME へ送った開閉の書き込みのうち、まだ同じ値の読みで確かめられていないもの（ADR-256）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingWrite<S> {
    scope: S,
    open: bool,
    at_ms: u64,
}

/// 外部変化の監視窓と、直近の読みの記録。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExternalChangeWatch<S: Copy + PartialEq> {
    armed: Option<Armed<S>>,
    /// 全 refresh の入口で記録する直近の読み（スコープ付き）。基準値の初期値になる。
    last_read: Option<(S, bool)>,
    /// ADR-256: まだ読みで確かめられていない awase 自身の開閉の書き込み。次の直接観測の窓の arm で 1 回だけ窓へ引き継ぐ。
    pending_write: Option<PendingWrite<S>>,
}

impl<S: Copy + PartialEq> ExternalChangeWatch<S> {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            armed: None,
            last_read: None,
            pending_write: None,
        }
    }

    /// ADR-256（BUG-199）: awase 自身が SendInput で IME へ開閉を書いた（`ImeOpenOutcome::Applied`）ら呼ぶ。
    /// 届くのが窓より遅れうる書き込みを「未確認」として控える。新しい書き込みは古いものを置き換える。
    ///
    /// 完了が書き込みの順に届くことを前提にする（直接観測の窓〈`Imm32Unavailable`〉で効く書き込みは同期の GjiDirect /
    /// MsImeDirect だけで、async の open chain〈ImmCross が先頭の窓〉は走らない）。async の書き込みを足すときは見直すこと。
    /// スコープ `ForegroundScope::INVALID` どうしは等しいので、前景窓が取れない間の書き込みと arm は互いに引き継ぐ（実害はほぼ無い）。
    pub fn note_awase_open_write(&mut self, scope: S, open: bool, now_ms: u64) {
        self.pending_write = Some(PendingWrite {
            scope,
            open,
            at_ms: now_ms,
        });
    }

    /// ADR-256: prefetch 済みの開閉の読みで、未確認の awase の書き込みを確かめる（窓の有無によらず毎回呼ぶ）。
    ///
    /// - 同じスコープで書き込みと同じ値を読んだら、控え（`pending_write`）と、窓へ引き継いだもの（`awase_write`）を解く。
    /// - 別のスコープの読みなら控えを捨てる（フォーカスが移った。戻ってきた窓へ古い控えを引き継がない）。
    /// - 読めなかった（`None`）ときは何もしない。
    ///
    /// 戻り値は解いたもの（ログ用）。
    pub fn confirm_awase_write(&mut self, scope: S, read: Option<bool>) -> AwaseWriteConfirm {
        let Some(v) = read else {
            return AwaseWriteConfirm::default();
        };
        let mut confirmed = AwaseWriteConfirm::default();
        if let Some(p) = self.pending_write {
            if p.scope != scope {
                self.pending_write = None;
            } else if p.open == v {
                self.pending_write = None;
                confirmed.pending = Some(v);
            }
        }
        if let Some(a) = self.armed.as_mut() {
            if a.scope == scope && a.awase_write == Some(v) {
                a.awase_write = None;
                confirmed.carried = Some(v);
            }
        }
        confirmed
    }

    /// ADR-256: フォーカスが移った（プロセス変更・同一プロセス内の hwnd 変更）。控えと、窓へ引き継いだ書き込みを捨てる
    /// （控えは「awase の書き込みがこの入力コンテキストに届くか」の主張なので、別のコンテキストへ持ち越さない）。
    pub fn clear_awase_writes(&mut self) {
        self.pending_write = None;
        if let Some(a) = self.armed.as_mut() {
            a.awase_write = None;
        }
    }

    /// ADR-256: 保持しなければ belief を動かしていた読みを、引き継いだ書き込みのために採らなかったことを数える。
    pub fn note_effective_hold(&mut self) {
        if let Some(a) = self.armed.as_mut() {
            a.effective_holds = a.effective_holds.saturating_add(1);
        }
    }

    /// ADR-256: 窓へ引き継いだ書き込みが未確認のまま、窓が切れた（またはスコープが変わった）か（消費しない。この後の
    /// `direct_live` 等が窓を捨てる）。そうなら（書き込みの値, 効果のあった保持の回数）。ログ・計測用。
    #[must_use]
    pub fn expired_unconfirmed_awase_write(
        &self,
        scope: S,
        now_ms: u64,
        window_ms: u64,
    ) -> Option<(bool, u16)> {
        let a = self.armed?;
        let w = a.awase_write?;
        let gone = a.scope != scope || now_ms.saturating_sub(a.last_arm_ms) > window_ms;
        gone.then_some((w, a.effective_holds))
    }

    /// まだ読みで確かめられていない awase の書き込みの控えがあるか（ログ用）。
    #[must_use]
    pub const fn has_pending_awase_write(&self) -> bool {
        self.pending_write.is_some()
    }

    /// 直接観測の窓へ引き継いだ、まだ確かめられていない awase の書き込み（ログ・テスト用）。窓が無ければ `None`。
    #[must_use]
    pub fn carried_awase_write(&self) -> Option<bool> {
        self.armed.and_then(|a| a.awase_write)
    }

    /// 控えを取り出す（1 回限り）。同じスコープで、書き込みから `window_ms` 以内なら値を返す。古い・別スコープなら捨てて `None`。
    fn take_pending_write(&mut self, scope: S, now_ms: u64, window_ms: u64) -> Option<bool> {
        let p = self.pending_write.take()?;
        pending_write_in_flight(p.scope == scope, now_ms.saturating_sub(p.at_ms), window_ms)
            .then_some(p.open)
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
        self.arm_with(scope, now_ms, window_ms, None);
    }

    /// ADR-188: 物理のモードキー通過／FSM 再送出で直接観測の窓を開く。生きている同じスコープの窓があれば Direct に
    /// 格上げして延ばす（基準値は保つ。Direct の窓は基準値を使わない）。
    ///
    /// `hold_admitted`（ADR-256、[`awase_write_hold_admission`]）: 窓を開いたキーが IME の状態を変えないと確かめられたか。
    /// `true` のときだけ未確認の awase の書き込みを窓へ引き継ぐ。`false`（役割があるかもしれないキー）なら控えを捨て、
    /// 生きている窓が引き継いでいた書き込みも解く（そのキーで利用者が IME を変えたかもしれないので、窓内の読みを採る）。
    pub fn arm_direct(&mut self, scope: S, now_ms: u64, window_ms: u64, hold_admitted: bool) {
        self.arm_with(scope, now_ms, window_ms, Some(hold_admitted));
    }

    fn arm_with(&mut self, scope: S, now_ms: u64, window_ms: u64, direct: Option<bool>) {
        // ADR-256: 直接観測の窓の arm（延長を含む）だけが、未確認の awase の書き込みを引き継ぐ（1 回限り）。
        // 外部注入の監視窓（ADR-205、基準値あり、`direct == None`）の arm では控えを取り出さない。
        // 役割があるかもしれないキーの arm（`Some(false)`）は控えを捨て、引き継いでいた書き込みも解く。
        let carried = match direct {
            Some(true) => self.take_pending_write(scope, now_ms, window_ms),
            Some(false) => {
                self.pending_write = None;
                None
            }
            None => None,
        };
        let release = direct == Some(false);
        let direct = direct.is_some();
        if let Some(a) = self.armed.as_mut() {
            let alive = a.scope == scope && now_ms.saturating_sub(a.last_arm_ms) <= window_ms;
            if alive {
                let cap = a
                    .first_arm_ms
                    .saturating_add(window_ms.saturating_mul(MAX_EXTENSION_FACTOR));
                a.last_arm_ms = now_ms.min(cap);
                a.direct |= direct;
                if carried.is_some() || release {
                    a.awase_write = carried;
                }
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
            awase_write: carried,
            effective_holds: 0,
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

/// [`ExternalChangeWatch::confirm_awase_write`] が解いたもの（ログ用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AwaseWriteConfirm {
    /// 窓へ引き継ぐ前の控えを、この値の読みで解いた。
    pub pending: Option<bool>,
    /// 窓へ引き継いだ書き込みを、窓内のこの値の読みで解いた。
    pub carried: Option<bool>,
}

/// ADR-256（BUG-199）: 控えた awase の書き込みを、直接観測の窓の arm で引き継ぐか。
///
/// 同じスコープで、書き込みから arm まで `window_ms`（直接観測の窓の長さ）未満のときだけ。これは arm までの間隔の上限で、
/// 保持そのものは arm から窓の終わり（延長すれば最初の arm から最大 `window_ms * 3`）まで続く。上限に窓の長さを使うのは、
/// 「通過したキーの効果が読みに現れるまで」という窓が既に置いている想定（ADR-187 の実測最大 62ms の約 5 倍）を、awase 自身が
/// 送ったキーにも当てるため（新しい時間定数を足さない）。これより古い控えは、読みが止まっている間（明示意図があると読めない窓の
/// ポーリングは止まる）に残っただけのもので、引き継ぐと利用者のモードキーへの追随を窓の全体で止めてしまう。
#[must_use]
pub const fn pending_write_in_flight(same_scope: bool, age_ms: u64, window_ms: u64) -> bool {
    same_scope && age_ms < window_ms
}

/// ADR-256（所有者決定 2026-10-11「役割のないキーに限定」）: 直接観測の窓を開いたキーについて、未確認の awase の
/// 書き込みを窓へ引き継いでよいか（保持を効かせてよいか）と、効かせない理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldAdmission {
    /// GJI の表で、このキー（F13〜F24、無修飾）の行がどこにも無い: キーは IME の状態を変えないので、窓内の「書き込みと逆の値」は
    /// awase の書き込みが未到達なだけ。保持する。
    Admitted,
    /// 表にこのキーの行がある（利用者がこのキーで IME を変えたかもしれない）。
    KeyHasFunction,
    /// 修飾付きの打鍵（表の修飾付きの行・IME の既定動作を確かめていない）。
    Modified,
    /// GJI 以外（Microsoft IME 本体は F キーの表を読まない）・F13〜F24 以外・表が読めない・overlay・未知の設定。
    Unknown,
}

impl HoldAdmission {
    /// 保持を効かせるか。
    #[must_use]
    pub const fn admitted(self) -> bool {
        matches!(self, Self::Admitted)
    }
}

/// [`HoldAdmission`] を決める純関数（ADR-256）。役割を確かめられなければ保持しない（逆向きの筋書き
/// 〈利用者が役割のあるキーで IME を開いた〉を壊さない側へ倒す）。
///
/// - `kind`: 直接観測の窓の IME 種別。GJI だけが対象（Microsoft IME 本体は F キーの表を読まない）。
/// - `fkey`: GJI の表で、このキーがどこかに行を持つか（`KeyEffectKeymap::gji_fkey_function`。キーマップが読めなければ `None`）。
/// - `modified`: Shift/Ctrl/Alt/Win のどれかを押したままか。
#[must_use]
pub fn awase_write_hold_admission(
    kind: crate::state::ime_kind::ImeKindId,
    fkey: Option<awase_gji_config::role::FKeyFunction>,
    modified: bool,
) -> HoldAdmission {
    use awase_gji_config::role::FKeyFunction;
    if !matches!(kind, crate::state::ime_kind::ImeKindId::Gji) {
        return HoldAdmission::Unknown;
    }
    if modified {
        return HoldAdmission::Modified;
    }
    match fkey {
        Some(FKeyFunction::NoFunction) => HoldAdmission::Admitted,
        Some(FKeyFunction::HasFunction) => HoldAdmission::KeyHasFunction,
        Some(FKeyFunction::Unknown) | None => HoldAdmission::Unknown,
    }
}

/// ADR-256（BUG-199）: 直接観測の窓内の読み（`read`）を、窓へ引き継いだ awase の書き込み（`carried`）のために採らないか。
///
/// 引き継いだ書き込みがあり、読みがその値でない（逆の値・読めなかった）間は採らない。窓より遅く IME に届く awase の書き込み
/// （例: 物理 Ctrl+無変換の OFF、GJI の処理遅延）を、処理前の古い読みで覆さないため。同じ値を読めば
/// [`ExternalChangeWatch::confirm_awase_write`] が先に `carried` を解くので、以後は従来どおり照合する。
#[must_use]
pub fn direct_read_held_by_awase_write(carried: Option<bool>, read: Option<bool>) -> bool {
    carried.is_some_and(|w| read != Some(w))
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

/// 直接観測（ADR-188）を適用する窓の IME 種別（ADR-244 D2）。
///
/// `Imm32Unavailable` の窓で、GJI または同定済みの Microsoft IME 本体（`table_ime_kind()`）のときだけ `Some`。
/// ATOK・第三者 IME・IMM32 HKL のみ・起動直後の未検出（`table_ime_kind() == None`）と、`Imm32Unavailable` 以外の窓は `None`。
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

/// Microsoft IME 本体の直接観測で採る開閉の軸（ADR-244 D4 の方向限定の緩和）。**「閉→開」だけ**を採る。
///
/// - 「開→閉」（`Some(false)`）は採らない。ADR-205 が MS-IME × 実 Chrome で観測した誤読は「開いているのに 0」の
///   向き（run 36548761653 の `imeoff-ext-msime-native`）で、採るとトグル中などに閉と誤読して Engine が OFF のまま残る。
/// - 「閉→開」（`Some(true)`）は採る。逆向きの誤読（閉じているのに 1）は観測されていない: run 38059015712
///   （MS-IME 本体 × 実 Chrome、12 構成）でモードキー直後〜次の確認までの読みは、実状態が閉の 647 件がすべて
///   `Some(false)`、開の 1341 件がすべて `Some(true)`（他は `None`）。Custom 表に予測の無いコードを持つ利用者で、
///   直接入力 → 変換/無変換で IME は開くのに Engine が OFF のまま残る失敗を直す。
#[must_use]
pub fn ms_ime_native_open_axis(open: Option<bool>) -> Option<bool> {
    open.filter(|o| *o)
}

/// [`classify_direct_read`] に IME 種別の軸の絞り込みを足したもの（ADR-244 D4）。
///
/// - GJI: 開閉・英数の両軸（ADR-188 のとおり）。
/// - Microsoft IME 本体: 英数の軸と、開閉の軸のうち**「閉→開」の向きだけ**（[`ms_ime_native_open_axis`]）。
///   MS-IME × 実 Chrome の開閉の読みは「開いているのに 0」が観測されたことがある（ADR-205）ので、「開→閉」は採らず、
///   トグル中に閉と誤読して追随し Engine が OFF のまま残る失敗を踏まない。
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
            open: ms_ime_native_open_axis(follow.open),
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
        w.arm_direct(1, 1000, W, true);
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
        w.arm_direct(1, 1100, W, true);
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
        w.arm_direct(1, 1000, W, true);
        assert!(!w.direct_live(2, 1010, W), "スコープ違いは破棄");
        let mut w = ExternalChangeWatch::<u32>::new();
        w.arm_direct(1, 1000, W, true);
        assert!(!w.direct_live(1, 1000 + W + 1, W));
        assert_eq!(w.last_arm_ms(), None, "切れた窓は破棄済み");
    }

    #[test]
    fn last_arm_ms_tracks_re_arm() {
        let mut w = ExternalChangeWatch::<u32>::new();
        assert_eq!(w.last_arm_ms(), None);
        w.arm_direct(1, 1000, W, true);
        assert_eq!(w.last_arm_ms(), Some(1000));
        w.arm_direct(1, 1100, W, true);
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
        assert_eq!(
            direct_watch_kind(true, Some(ImeKindId::Gji)),
            Some(ImeKindId::Gji)
        );
        assert_eq!(
            direct_watch_kind(true, Some(ImeKindId::MsIme)),
            Some(ImeKindId::MsIme)
        );
        // ATOK・第三者 IME・IMM32 HKL のみ・起動直後の未検出は `table_ime_kind() == None`。
        assert_eq!(direct_watch_kind(true, None), None);
        // Imm32Unavailable 以外の窓（Win32 EDIT、TsfNative 等）は対象外。
        assert_eq!(direct_watch_kind(false, Some(ImeKindId::Gji)), None);
        assert_eq!(direct_watch_kind(false, Some(ImeKindId::MsIme)), None);
    }

    /// ADR-244 D4: MS-IME 本体は英数の軸と「閉→開」の開閉だけ。閉の読みでは何も追随しない（「開いているのに 0」の型を踏まない）。
    #[test]
    fn ms_ime_native_follows_eisu_and_only_closed_to_open() {
        use crate::state::ime_kind::ImeKindId;
        // トグル中(belief: 開・英数)に変換/英数/ひらがなでかなへ戻った: NATIVE の読み → 英数を外す。
        assert_eq!(
            classify_direct_read_for(ImeKindId::MsIme, Some(true), Some(25), true, true),
            DirectFollow {
                open: None,
                eisu: Some(false)
            }
        );
        // 閉の読み(無変換): 開閉の軸は採らない。英数の軸も conv を見ないので None。
        assert!(
            classify_direct_read_for(ImeKindId::MsIme, Some(false), Some(0), true, true).is_none()
        );
        // belief が閉で読みが開・NATIVE（Custom 表の予測の無いコードで 直接入力 → 変換/無変換、run 38059015712）:
        // 「閉→開」の向きなので開閉の軸を採る。英数の軸は belief も読みも英数でない → なし。
        assert_eq!(
            classify_direct_read_for(ImeKindId::MsIme, Some(true), Some(25), false, false),
            DirectFollow {
                open: Some(true),
                eisu: None
            }
        );
        // 閉→開 で半角英数（NATIVE なし）を読んだら両軸。
        assert_eq!(
            classify_direct_read_for(ImeKindId::MsIme, Some(true), Some(16), false, false),
            DirectFollow {
                open: Some(true),
                eisu: Some(true)
            }
        );
        // belief が開で読みが閉（かな → 変換/無変換で閉じた場合も、「開いているのに 0」の誤読の場合も）: 開閉の軸は採らない。
        assert!(
            classify_direct_read_for(ImeKindId::MsIme, Some(false), Some(25), true, false)
                .is_none()
        );
        // 読めない(None)なら何もしない。
        assert!(classify_direct_read_for(ImeKindId::MsIme, None, Some(25), false, false).is_none());
        // 開で半角英数を読んだら英数を採る(軸の絞り込みは英数を妨げない)。
        assert_eq!(
            classify_direct_read_for(ImeKindId::MsIme, Some(true), Some(16), true, false),
            DirectFollow {
                open: None,
                eisu: Some(true)
            }
        );
    }

    /// MS-IME 本体の開閉の軸は「閉→開」(`Some(true)`)だけを通し、「開→閉」(`Some(false)`)は捨てる。
    #[test]
    fn ms_ime_native_open_axis_keeps_only_closed_to_open() {
        assert_eq!(ms_ime_native_open_axis(Some(true)), Some(true));
        assert_eq!(ms_ime_native_open_axis(Some(false)), None);
        assert_eq!(ms_ime_native_open_axis(None), None);
    }

    /// ADR-244: 全角英数（`conv=0x18`、NATIVE ビットなし）は英数のまま。トグル中（belief 英数）は追随なし＝トグルを手放さない。
    #[test]
    fn full_width_alnum_conv_0x18_stays_eisu_for_ms_ime_native() {
        use crate::state::ime_kind::ImeKindId;
        assert!(
            classify_direct_read_for(ImeKindId::MsIme, Some(true), Some(0x18), true, true)
                .is_none()
        );
        assert_eq!(
            classify_direct_read_for(ImeKindId::MsIme, Some(true), Some(0x18), true, false),
            DirectFollow {
                open: None,
                eisu: Some(true)
            }
        );
    }

    /// ADR-244 D4（#595）の表テスト: 読み（開閉 3 値 × conv 5 値）× belief（開閉 × 英数）の全組み合わせで、
    /// - GJI は [`classify_direct_read`] と完全に同じ（D4 の緩和は GJI に影響しない）。
    /// - MS-IME 本体の英数の軸は [`classify_direct_read`] と同じ（従来どおり）。
    /// - MS-IME 本体の開閉の軸は「閉→開」（`Some(true)`）だけで、`Some(false)`（開→閉）は決して返さない。
    ///   読めない（`None`）・belief と同じ読みでは開閉を追随しない。
    #[test]
    fn classify_direct_read_for_table_over_all_reads_and_beliefs() {
        use crate::state::ime_kind::ImeKindId;
        let opens = [None, Some(true), Some(false)];
        // かな(0x19)・カタカナ(0x1B)・半角英数(0x10)・全角英数(0x18)・0・読めない
        let convs = [
            Some(0x19),
            Some(0x1B),
            Some(0x10),
            Some(0x18),
            Some(0),
            None,
        ];
        for read_open in opens {
            for read_conv in convs {
                for belief_open in [true, false] {
                    for belief_eisu in [true, false] {
                        let ctx = format!(
                            "read_open={read_open:?} read_conv={read_conv:?} \
                             belief_open={belief_open} belief_eisu={belief_eisu}"
                        );
                        let base =
                            classify_direct_read(read_open, read_conv, belief_open, belief_eisu);
                        let gji = classify_direct_read_for(
                            ImeKindId::Gji,
                            read_open,
                            read_conv,
                            belief_open,
                            belief_eisu,
                        );
                        assert_eq!(gji, base, "GJI は従来どおり: {ctx}");
                        let ms = classify_direct_read_for(
                            ImeKindId::MsIme,
                            read_open,
                            read_conv,
                            belief_open,
                            belief_eisu,
                        );
                        assert_eq!(ms.eisu, base.eisu, "MS-IME の英数の軸は従来どおり: {ctx}");
                        let want_open = (read_open == Some(true) && !belief_open).then_some(true);
                        assert_eq!(ms.open, want_open, "MS-IME の開閉は閉→開だけ: {ctx}");
                        assert_ne!(ms.open, Some(false), "MS-IME は開→閉を採らない: {ctx}");
                    }
                }
            }
        }
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
            DirectFollow {
                open: Some(true),
                eisu: Some(true)
            }
        );
    }

    // ── ADR-256（BUG-199）: 届く前の awase 自身の書き込みを、直接観測の窓内の古い読みで覆さない ──

    /// 純関数の表: 引き継ぎが無ければ採る（従来どおり）。引き継いだ書き込みと同じ値なら採る（確認済み）。
    /// 逆の値・読めなかった（`None`）なら採らない。
    #[test]
    fn direct_read_held_by_awase_write_table() {
        for (carried, read, held) in [
            (None, Some(true), false),
            (None, Some(false), false),
            (None, None, false),
            (Some(false), Some(false), false),
            (Some(true), Some(true), false),
            (Some(false), Some(true), true),
            (Some(true), Some(false), true),
            (Some(false), None, true),
        ] {
            assert_eq!(
                direct_read_held_by_awase_write(carried, read),
                held,
                "carried={carried:?} read={read:?}"
            );
        }
    }

    /// 引き継ぐのは同じスコープで、書き込みから窓の長さ未満のときだけ（窓の長さと同じ想定を共用する）。
    #[test]
    fn pending_write_in_flight_requires_same_scope_and_age_below_window() {
        assert!(pending_write_in_flight(true, 0, W));
        assert!(
            pending_write_in_flight(true, 94, W),
            "BUG-199 の実測 78〜94ms"
        );
        assert!(pending_write_in_flight(true, W - 1, W));
        assert!(!pending_write_in_flight(true, W, W));
        assert!(!pending_write_in_flight(true, 30_000, W));
        assert!(!pending_write_in_flight(false, 0, W));
    }

    /// BUG-199 の再現: arm の 90ms 前に awase が OFF を書き、窓内は処理前の「開」を読み続ける。窓の間は保持され、
    /// 窓が切れたら窓ごと消える。控えは 1 回限りで、次の窓へは引き継がない。
    #[test]
    fn awase_write_before_arm_is_carried_and_held_until_window_expires() {
        let mut w = ExternalChangeWatch::<u32>::new();
        w.note_awase_open_write(1, false, 910);
        w.arm_direct(1, 1000, W, true);
        assert_eq!(w.carried_awase_write(), Some(false));
        for t in [1020, 1080, 1140, 1274] {
            assert_eq!(
                w.confirm_awase_write(1, Some(true)),
                AwaseWriteConfirm::default()
            );
            assert!(w.direct_live(1, t, W));
            assert!(direct_read_held_by_awase_write(
                w.carried_awase_write(),
                Some(true)
            ));
        }
        // 読めなかった読みは何も解かない。
        assert_eq!(w.confirm_awase_write(1, None), AwaseWriteConfirm::default());
        assert_eq!(w.carried_awase_write(), Some(false));
        // 窓が切れたら窓ごと消える。
        assert!(!w.direct_live(1, 1301, W));
        assert_eq!(w.carried_awase_write(), None);
        // 控えは 1 回限り（次の窓には引き継がない）。
        w.arm_direct(1, 1400, W, true);
        assert_eq!(w.carried_awase_write(), None);
    }

    /// 窓内で書き込みと同じ値を読んだら保持を解き、以後は従来どおり照合する（外部の変化には追随できる）。
    #[test]
    fn matching_read_in_window_releases_the_carried_write() {
        let mut w = ExternalChangeWatch::<u32>::new();
        w.note_awase_open_write(1, false, 950);
        w.arm_direct(1, 1000, W, true);
        assert_eq!(
            w.confirm_awase_write(1, Some(false)),
            AwaseWriteConfirm {
                pending: None,
                carried: Some(false)
            }
        );
        assert_eq!(w.carried_awase_write(), None);
        assert!(!direct_read_held_by_awase_write(
            w.carried_awase_write(),
            Some(true)
        ));
    }

    /// arm の前に同じ値を読めば、控えはその時点で解け、窓へ引き継がない。
    #[test]
    fn matching_read_before_arm_confirms_the_pending_write() {
        let mut w = ExternalChangeWatch::<u32>::new();
        w.note_awase_open_write(1, false, 950);
        assert_eq!(
            w.confirm_awase_write(1, Some(false)),
            AwaseWriteConfirm {
                pending: Some(false),
                carried: None
            }
        );
        w.arm_direct(1, 1000, W, true);
        assert_eq!(w.carried_awase_write(), None);
    }

    /// 逆の値の読みは控えを解かない（処理前の古い読みでありうる）。
    #[test]
    fn opposite_read_before_arm_keeps_the_pending_write() {
        let mut w = ExternalChangeWatch::<u32>::new();
        w.note_awase_open_write(1, false, 950);
        assert_eq!(
            w.confirm_awase_write(1, Some(true)),
            AwaseWriteConfirm::default()
        );
        w.arm_direct(1, 1000, W, true);
        assert_eq!(w.carried_awase_write(), Some(false));
    }

    /// 書き込みから窓の長さ以上たった控えは引き継がない（読みが止まっている間に残っただけ）。取り出した控えは捨てる。
    #[test]
    fn stale_pending_write_is_not_carried() {
        let mut w = ExternalChangeWatch::<u32>::new();
        w.note_awase_open_write(1, false, 1000 - W);
        w.arm_direct(1, 1000, W, true);
        assert_eq!(w.carried_awase_write(), None);
    }

    /// 別のスコープの読みが来たら控えを捨てる。別のスコープの arm では引き継がない。
    #[test]
    fn pending_write_respects_scope() {
        let mut w = ExternalChangeWatch::<u32>::new();
        w.note_awase_open_write(1, false, 950);
        w.arm_direct(2, 1000, W, true);
        assert_eq!(w.carried_awase_write(), None);

        let mut w = ExternalChangeWatch::<u32>::new();
        w.note_awase_open_write(1, false, 950);
        assert_eq!(
            w.confirm_awase_write(2, Some(true)),
            AwaseWriteConfirm::default()
        );
        w.arm_direct(1, 1000, W, true);
        assert_eq!(
            w.carried_awase_write(),
            None,
            "別のスコープの読みで控えを捨てた"
        );
    }

    /// 外部注入の監視窓（ADR-205）の arm は控えを取り出さない。後から同じ窓が直接観測へ格上げ（延長）されたら引き継ぐ。
    /// 窓の延長の時点で、窓の途中に書いた控えも引き継ぐ（延長で `last_arm_ms` が書き込みより後ろへずれ、R3 から外れても守る）。
    #[test]
    fn only_direct_arm_takes_the_pending_write_including_extension() {
        let mut w = ExternalChangeWatch::<u32>::new();
        w.note_awase_open_write(1, true, 950);
        w.arm(1, 1000, W);
        assert_eq!(
            w.carried_awase_write(),
            None,
            "ADR-205 の arm では取り出さない"
        );
        w.arm_direct(1, 1050, W, true);
        assert_eq!(
            w.carried_awase_write(),
            Some(true),
            "格上げの arm で引き継ぐ"
        );

        let mut w = ExternalChangeWatch::<u32>::new();
        w.arm_direct(1, 1000, W, true);
        w.note_awase_open_write(1, false, 1100);
        w.arm_direct(1, 1150, W, true);
        assert_eq!(w.last_arm_ms(), Some(1150));
        assert_eq!(w.carried_awase_write(), Some(false));
        // 延長で控えが無ければ、引き継いだものは変えない。
        w.arm_direct(1, 1200, W, true);
        assert_eq!(w.carried_awase_write(), Some(false));
    }

    /// 新しい書き込みは古い控えを置き換える。
    #[test]
    fn newer_awase_write_replaces_the_pending_one() {
        let mut w = ExternalChangeWatch::<u32>::new();
        w.note_awase_open_write(1, false, 900);
        w.note_awase_open_write(1, true, 950);
        assert_eq!(
            w.confirm_awase_write(1, Some(false)),
            AwaseWriteConfirm::default()
        );
        w.arm_direct(1, 1000, W, true);
        assert_eq!(w.carried_awase_write(), Some(true));
    }

    /// レビュー S3: フォーカスが移ったら控えも引き継いだ書き込みも捨てる。
    #[test]
    fn clear_awase_writes_drops_pending_and_carried() {
        let mut w = ExternalChangeWatch::<u32>::new();
        w.note_awase_open_write(1, false, 950);
        w.clear_awase_writes();
        w.arm_direct(1, 1000, W, true);
        assert_eq!(w.carried_awase_write(), None);

        let mut w = ExternalChangeWatch::<u32>::new();
        w.note_awase_open_write(1, false, 950);
        w.arm_direct(1, 1000, W, true);
        w.clear_awase_writes();
        assert_eq!(w.carried_awase_write(), None);
        assert!(w.direct_live(1, 1010, W), "窓そのものは残す");
    }

    /// レビュー M1: 未確認のまま窓が切れたことと、効果のあった保持の回数を出す。確認済み・窓の間は出さない。
    #[test]
    fn expired_unconfirmed_awase_write_reports_effective_holds() {
        let mut w = ExternalChangeWatch::<u32>::new();
        w.note_awase_open_write(1, false, 950);
        w.arm_direct(1, 1000, W, true);
        w.note_effective_hold();
        w.note_effective_hold();
        assert_eq!(
            w.expired_unconfirmed_awase_write(1, 1300, W),
            None,
            "窓の間"
        );
        assert_eq!(
            w.expired_unconfirmed_awase_write(1, 1301, W),
            Some((false, 2))
        );
        assert_eq!(
            w.expired_unconfirmed_awase_write(2, 1100, W),
            Some((false, 2)),
            "スコープが変わった"
        );
        // 確認済みなら出さない。
        let mut w = ExternalChangeWatch::<u32>::new();
        w.note_awase_open_write(1, false, 950);
        w.arm_direct(1, 1000, W, true);
        w.confirm_awase_write(1, Some(false));
        assert_eq!(w.expired_unconfirmed_awase_write(1, 1301, W), None);
    }

    /// 所有者決定（役割のないキーに限定）: 判定の表。GJI × 無修飾 × 行なしだけ保持する。
    #[test]
    fn awase_write_hold_admission_table() {
        use crate::state::ime_kind::ImeKindId::{Gji, MsIme};
        use awase_gji_config::role::FKeyFunction::{HasFunction, NoFunction, Unknown};
        for (kind, fkey, modified, want) in [
            (Gji, Some(NoFunction), false, HoldAdmission::Admitted), // BUG-199 の F18
            (Gji, Some(HasFunction), false, HoldAdmission::KeyHasFunction), // 逆向きの F17(IME 有効化)
            (Gji, Some(NoFunction), true, HoldAdmission::Modified),         // Shift+F18 等
            (Gji, Some(Unknown), false, HoldAdmission::Unknown), // 無変換/変換・overlay・未知の設定
            (Gji, None, false, HoldAdmission::Unknown),          // キーマップが読めない
            (MsIme, Some(NoFunction), false, HoldAdmission::Unknown), // MS-IME 本体
        ] {
            let got = awase_write_hold_admission(kind, fkey, modified);
            assert_eq!(got, want, "{kind:?} {fkey:?} modified={modified}");
            assert_eq!(got.admitted(), want == HoldAdmission::Admitted);
        }
    }

    /// 役割があるかもしれないキーで窓を開いたら、控えを引き継がず捨てる。生きている窓の引き継ぎも解く。
    #[test]
    fn non_admitted_direct_arm_drops_pending_and_releases_carried() {
        let mut w = ExternalChangeWatch::<u32>::new();
        w.note_awase_open_write(1, false, 950);
        w.arm_direct(1, 1000, W, false);
        assert_eq!(w.carried_awase_write(), None);
        w.arm_direct(1, 1010, W, true);
        assert_eq!(w.carried_awase_write(), None, "控えは捨てた");

        let mut w = ExternalChangeWatch::<u32>::new();
        w.note_awase_open_write(1, false, 950);
        w.arm_direct(1, 1000, W, true);
        assert_eq!(w.carried_awase_write(), Some(false));
        w.arm_direct(1, 1100, W, false);
        assert_eq!(w.carried_awase_write(), None, "役割のあるキーの延長で解く");
        assert!(w.direct_live(1, 1150, W));
    }
}
