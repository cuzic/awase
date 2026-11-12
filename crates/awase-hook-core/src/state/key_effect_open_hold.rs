//! ADR-292 P: `KeyEffectPrediction`(モードキー直後、mozc-shm実測の裏取り前)が未照合の間、
//! Engineに入る前の物理キーを保持する状態機械(「第3のhold」、`focus_switch_hold`/
//! `resume_hold`と同じ形)。殻は `awase-windows/src/runtime/key_effect_open_hold.rs`。
//!
//! 背景([BUG-233](../../../../../docs/known-bugs/BUG-233.md)): モードキー予測だけで
//! `effective_open`をtrueに倒した直後、mozc-shmの実測が追いつく前にEngineがローマ字へ
//! 変換して送信すると、実際にはIMEが開いていない場合にリテラルとして出力されてしまう。
//! ローマ字はEngineがbelief=開を前提に変換した**結果**であり、予測が外れたと分かった後に
//! 元の物理キーへ戻す手段が無いため、変換**前**の物理キーを保持し、解放時のbeliefに従って
//! (`INPUT_DEFER.replay_later`で)Engineへ流し直す設計にする。
//!
//! 純粋: 時刻・前面窓のスコープは引数で受け、保持する打鍵 `T` は呼び出し側の型
//! (殻では `RawKeyEvent`)。ここは次の 3 つだけを持つ。
//!
//! 1. **始める条件**(ADR-292 T4・U2・U5): [`KeyEffectOpenHold::arm`] が (a) GJI の証拠が無い、
//!    (b) 前面窓のスレッドの直前の mozc-shm の読みが受け入れ(`Outcome::Accepted`、`active`込み)、
//!    (c) 予測が「開」で、その予測(`pred.at_ms`で識別)についてまだ保持していない、を確かめて**武装**する。
//!    保持そのものは、武装の後に来る**次の**非リピートの KeyDown から始まる(予測を立てた打鍵自身は
//!    通常どおり処理済み)。予測ごとに 1 回だけ(U2: 解放の理由を問わず、同じ予測で掛け直さない)。
//! 2. **保持・解放・窓の照合**: 保持の中の Down に対応する Up・リピートは足す。保持の外で既に処理された
//!    Down の Up は足さず通す(`half_width_resume_hold` と同じ契機の表)。上限に達したらその打鍵を末尾に
//!    含めて解放する。前面窓が保持の開始時と違えば `Discard`。
//! 3. **解放の判定**([`judge`]): mozc-shm の読み(`decide_use` の結果)・予測がまだ未照合か・保持の
//!    経過時間から、保持を続けるか・どの理由で解放するかを決める。`decide_use` の判定そのもの
//!    (G3 の baseline を含む)はここに持たない——呼び出し側が [`hold_baseline_ms`] で起点を作って
//!    既存の `report_mozc_shm`(→`decide_use`)を呼ぶ(ADR-292 round4 T1、新しい判定関数は作らない)。

use crate::state::foreground_scope::ForegroundScope;
use crate::state::half_width_resume_hold::KeyFacts;
use crate::state::mozc_shm::ShmUse;
use crate::state::TickMs;

/// 保持の上限(打鍵数)。`FOCUS_SWITCH_HOLD_CAP`と同じ値(ADR-292 round3 S5)。
pub const KEY_EFFECT_OPEN_HOLD_CAP: usize = 8;

/// 解放の結果。保持の中身(`T`)を返す。
#[derive(Debug, PartialEq, Eq)]
pub enum Released<T> {
    /// 保持が無い、または `hold_id` が今の保持と違う(古い札の通知)。何もしない。
    Nothing,
    /// 前面窓が保持の開始時と同じ。順序どおり再生してよい
    /// (再生後にbeliefをどうするかは呼び出し側がどの理由で解放したかで決める)。
    Replay(Vec<T>),
    /// 前面窓が保持の開始時と違うので捨てる(別窓へ誤配送しない)。
    /// 黙って捨ててはならない(ADR-292 R6(ii)の訂正と同型、BUG-074型の無断消失を避ける)
    /// ——呼び出し側が利用者へ通知する責任を持つ。
    Discard(Vec<T>),
}

/// 打鍵が来たときの扱い。
#[derive(Debug, PartialEq, Eq)]
pub enum OnKey<T> {
    /// 保持の対象でない打鍵。呼び出し側が通常どおり処理する(打鍵を返す)。
    Pass(T),
    /// 武装していたので、この打鍵から保持を始めた(呼び出し側は解放判定のタイマーを張り、消費済みとして扱う)。
    Started(u64),
    /// 保持に足した(呼び出し側は消費済みとして扱う)。
    Held,
    /// 上限に達した。この打鍵を列の末尾に含めて解放する(窓の照合は `release` と同じ)。
    Release(Released<T>),
}

/// 武装([`KeyEffectOpenHold::arm`])に使う事実。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArmFacts {
    /// (a) GJI の証拠が無い(`tsf::observer::gji_evidence_absent`)。
    pub gji_evidence_absent: bool,
    /// (b) 前面窓のスレッドの直前の mozc-shm の読みが `Outcome::Accepted`(`active == 1` を含む)。
    /// `mozc_shm.polling`(`Rejected` 等でも真)では判定しない(ADR-292 round4 U5)。
    pub shm_accepted_for_focus: bool,
    /// (c) 未照合の予測が「開」で、その時刻(`pred.at_ms`)。開でない・予測が無いなら `None`。
    pub predicted_open_at: Option<TickMs>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Armed {
    scope: ForegroundScope,
    pred_at_ms: TickMs,
}

#[derive(Debug)]
struct Active<T> {
    id: u64,
    scope: ForegroundScope,
    /// モードキー予測が立った時刻(`pred.at_ms`)。G3のbaseline計算(呼び出し側)の起点。
    pred_at_ms: TickMs,
    /// 保持を始めた時刻(期限の判定に使う)。
    started_at: TickMs,
    items: Vec<(KeyFacts, T)>,
}

/// `KeyEffectPrediction`未照合中の物理キー保持。同時に1つだけ。
#[derive(Debug)]
pub struct KeyEffectOpenHold<T> {
    active: Option<Active<T>>,
    armed: Option<Armed>,
    /// 最後に保持を始めた予測の時刻(U2「予測ごとに1回」)。
    last_held_pred: Option<TickMs>,
    next_id: u64,
}

impl<T> Default for KeyEffectOpenHold<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> KeyEffectOpenHold<T> {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            active: None,
            armed: None,
            last_held_pred: None,
            next_id: 1,
        }
    }

    #[must_use]
    pub const fn is_holding(&self) -> bool {
        self.active.is_some()
    }

    #[must_use]
    pub const fn is_armed(&self) -> bool {
        self.armed.is_some()
    }

    #[must_use]
    pub fn hold_id(&self) -> Option<u64> {
        self.active.as_ref().map(|a| a.id)
    }

    /// 保持しているモードキー予測の時刻(`pred.at_ms`)。G3のbaseline計算に使う。
    #[must_use]
    pub fn pred_at_ms(&self) -> Option<TickMs> {
        self.active.as_ref().map(|a| a.pred_at_ms)
    }

    /// 保持を始めた時刻(ログ・期限用)。
    #[must_use]
    pub fn started_at(&self) -> Option<TickMs> {
        self.active.as_ref().map(|a| a.started_at)
    }

    /// 保持中の件数(ログ用)。
    #[must_use]
    pub fn len(&self) -> usize {
        self.active.as_ref().map_or(0, |a| a.items.len())
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 予測が「開」に倒れた直後(`apply_key_effect_prediction` の後)に呼ぶ。条件 (a)(b)(c) が全て揃えば
    /// 武装して `true`(次の非リピートの KeyDown から保持を始める)。既に保持中なら武装しない(保持中の
    /// モードキーの再押下は保持の中にあり、再生の時点で改めて予測・武装される。ADR-292 round3 T2)。
    pub fn arm(&mut self, facts: ArmFacts, scope: ForegroundScope) -> bool {
        if self.active.is_some() {
            return false;
        }
        let ok = facts.gji_evidence_absent && facts.shm_accepted_for_focus;
        let Some(pred_at_ms) = facts.predicted_open_at.filter(|_| ok) else {
            self.armed = None;
            return false;
        };
        if self.last_held_pred == Some(pred_at_ms) {
            // U2: この予測では既に 1 回保持した(解放の理由を問わず掛け直さない)。
            self.armed = None;
            return false;
        }
        self.armed = Some(Armed { scope, pred_at_ms });
        true
    }

    /// 武装を解く(フォーカス変化・panic・設定再読み込み)。保持中の打鍵には触れない。
    pub fn disarm(&mut self) {
        self.armed = None;
    }

    /// 打鍵が来た。
    ///
    /// - 保持中: 保持の中の Down に対応する Up・リピートは足す。保持の外の Down の Up・リピートは `Pass`。
    ///   新しい Down は足し、上限を超えたらこの打鍵を末尾に含めて解放する(追い越させない)。
    ///   モードキーの再押下(ADR-292 round3 T2)も区別せず保持される。
    /// - 武装中: 予測がまだ同じ時刻で「開」(`pending_open_pred_at`)かつ前面窓が武装時と同じなら、非リピートの
    ///   KeyDown から保持を始める(`Started`)。予測が照合済み・別の予測に替わった・窓が変わったなら武装を解いて
    ///   `Pass`。Up・リピートは `Pass`(武装は残す)。
    /// - それ以外: `Pass`。
    pub fn on_key(
        &mut self,
        facts: KeyFacts,
        item: T,
        current: ForegroundScope,
        pending_open_pred_at: Option<TickMs>,
        now: TickMs,
    ) -> OnKey<T> {
        if let Some(mut active) = self.active.take() {
            let has_down_of_same_key = active
                .items
                .iter()
                .any(|(f, _)| f.is_down && f.identity == facts.identity);
            if !facts.is_down || facts.is_repeat {
                if has_down_of_same_key {
                    active.items.push((facts, item));
                    self.active = Some(active);
                    return OnKey::Held;
                }
                self.active = Some(active);
                return OnKey::Pass(item);
            }
            active.items.push((facts, item));
            if active.items.len() > KEY_EFFECT_OPEN_HOLD_CAP {
                return OnKey::Release(Self::finish(active, current));
            }
            self.active = Some(active);
            return OnKey::Held;
        }
        let Some(armed) = self.armed else {
            return OnKey::Pass(item);
        };
        if pending_open_pred_at != Some(armed.pred_at_ms) || armed.scope != current {
            self.armed = None;
            return OnKey::Pass(item);
        }
        if !facts.is_down || facts.is_repeat {
            return OnKey::Pass(item);
        }
        self.armed = None;
        self.last_held_pred = Some(armed.pred_at_ms);
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        self.active = Some(Active {
            id,
            scope: armed.scope,
            pred_at_ms: armed.pred_at_ms,
            started_at: now,
            items: vec![(facts, item)],
        });
        OnKey::Started(id)
    }

    /// 解放する。`hold_id` が `Some` なら今の保持と一致するときだけ(古い札の通知を無視する)。
    /// 前面窓が保持の開始時と違えば `Discard`。解放する理由(開いた/閉で確定/期限/読めなく
    /// なった等)は呼び出し側の責務——ここでは窓の照合と中身の取り出しだけを行う。
    pub fn release(&mut self, hold_id: Option<u64>, current: ForegroundScope) -> Released<T> {
        let Some(active) = self.active.take() else {
            return Released::Nothing;
        };
        if hold_id.is_some_and(|id| id != active.id) {
            self.active = Some(active);
            return Released::Nothing;
        }
        Self::finish(active, current)
    }

    fn finish(active: Active<T>, current: ForegroundScope) -> Released<T> {
        let items = active.items.into_iter().map(|(_, item)| item).collect();
        if active.scope == current {
            Released::Replay(items)
        } else {
            Released::Discard(items)
        }
    }
}

/// 解放の理由(ログと、解放後の通知の要否の判断に使う)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseReason {
    /// mozc-shm の読みが「開」で `Use`(予測と一致して確認できた)。belief はそのまま正しい。
    ConfirmedOpen,
    /// mozc-shm の読みが「閉」で `Use`(settle 経過後に食い違いが確定)。belief は `report_mozc_shm` の追随で
    /// 閉に直っている。ADR-292 R6(iv): 利用者に知らせてから流し直す(閉の belief で素通しとして処理される)。
    ObservedClosed,
    /// 読めない(`Unreadable`・`Rejected`・`active == 0`・無効アプリ)。ADR-292 round4 U1: 即座に解放して
    /// 現状(P なし)の挙動に戻す。`judge_compose` の `ImeNotActive`(閉の読みを含む)は流用しない。
    Unreadable,
    /// 予測が別の経路(通常の 500ms の読み・別の観測・新しい予測)で照合済み・置き換わった。belief は既に
    /// 観測由来なのでそのまま流す。
    PredictionSettled,
    /// 期限([`judge`] の `max_ms`)までに確定しなかった。belief は変えずに流す(現状の挙動と同じ)。
    Deadline,
    /// 保持の上限。
    Cap,
    /// フォーカスが変わった(窓が違えば `Discard`)。
    FocusChanged,
    /// `WM_PANIC_RESET`。
    PanicReset,
    /// 設定の再読み込み。
    ConfigReload,
}

impl ReleaseReason {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::ConfirmedOpen => "ConfirmedOpen",
            Self::ObservedClosed => "ObservedClosed",
            Self::Unreadable => "Unreadable",
            Self::PredictionSettled => "PredictionSettled",
            Self::Deadline => "Deadline",
            Self::Cap => "Cap",
            Self::FocusChanged => "FocusChanged",
            Self::PanicReset => "PanicReset",
            Self::ConfigReload => "ConfigReload",
        }
    }

    /// 利用者へ知らせる必要がある解放か(ADR-292 R6(iv)・期限。今は `tracing::warn!` のログだけ)。
    #[must_use]
    pub const fn needs_notice(self) -> bool {
        matches!(self, Self::ObservedClosed | Self::Deadline)
    }
}

/// 保持の解放判定の結論。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// 保持を続ける(短周期の読みを張り直す)。
    Keep,
    /// 解放する。
    Release(ReleaseReason),
}

/// 保持の解放判定(短周期の読みごと)。
///
/// `read` は、今回の読みを `hold_baseline_ms` の起点で `report_mozc_shm`(→`decide_use`)に通した結果。
/// `None` は読めなかった(受け入れなかった)。`prediction_pending` は、保持を始めた予測(同じ `pred.at_ms`)の
/// 「開」がまだ未照合で残っているか。`held_ms` は保持の経過時間、`max_ms` は期限。
///
/// 優先順位: 読めない → 即座解放(U1)。`Use` → 開なら確認、閉なら R6(iv)。`Held`・`Split` は判断しない
/// (保持継続)。そのうえで予測が既に照合済みなら解放、期限を過ぎていれば解放。
#[must_use]
pub const fn judge(
    read: Option<ShmUse>,
    prediction_pending: bool,
    held_ms: u64,
    max_ms: u64,
) -> Verdict {
    match read {
        None => return Verdict::Release(ReleaseReason::Unreadable),
        Some(ShmUse::Use { open: true, .. }) => {
            return Verdict::Release(ReleaseReason::ConfirmedOpen)
        }
        Some(ShmUse::Use { open: false, .. }) => {
            return Verdict::Release(ReleaseReason::ObservedClosed)
        }
        Some(ShmUse::Held | ShmUse::Split) => {}
    }
    if !prediction_pending {
        return Verdict::Release(ReleaseReason::PredictionSettled);
    }
    if held_ms >= max_ms {
        return Verdict::Release(ReleaseReason::Deadline);
    }
    Verdict::Keep
}

/// 保持の解放判定に使う `decide_use` の起点(ADR-292 round3 T1)。
///
/// ローリング baseline(`hook::last_operation_tick_ms`)は保持(抑止)した打鍵でも進むので使わない。
/// 予測の時刻と、awase が実際に IME の開閉/モードを動かす目的で送信した時刻(S1、`ime_actuation_send_tick_ms`・
/// `imm_cross_write_tick_ms`)の大きい方。保持中の打鍵はどちらにも入らない。
#[must_use]
pub const fn hold_baseline_ms(pred_at_ms: TickMs, ime_actuation_baseline_ms: u64) -> u64 {
    if pred_at_ms.0 > ime_actuation_baseline_ms {
        pred_at_ms.0
    } else {
        ime_actuation_baseline_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(pid: u32, hwnd: isize) -> ForegroundScope {
        ForegroundScope { pid, hwnd }
    }

    const fn down(identity: u64) -> KeyFacts {
        KeyFacts {
            identity,
            is_down: true,
            is_repeat: false,
        }
    }

    const fn up(identity: u64) -> KeyFacts {
        KeyFacts {
            identity,
            is_down: false,
            is_repeat: false,
        }
    }

    const fn repeat(identity: u64) -> KeyFacts {
        KeyFacts {
            identity,
            is_down: true,
            is_repeat: true,
        }
    }

    const fn arm_facts(pred_at: u64) -> ArmFacts {
        ArmFacts {
            gji_evidence_absent: true,
            shm_accepted_for_focus: true,
            predicted_open_at: Some(TickMs(pred_at)),
        }
    }

    /// 武装して、次の Down `k` から保持を始めた状態。
    fn started() -> (KeyEffectOpenHold<&'static str>, u64) {
        let mut h = KeyEffectOpenHold::new();
        assert!(h.arm(arm_facts(100), scope(1, 10)));
        let OnKey::Started(id) =
            h.on_key(down(1), "k", scope(1, 10), Some(TickMs(100)), TickMs(120))
        else {
            panic!("start");
        };
        (h, id)
    }

    /// 保持も武装も無いとき、打鍵はそのまま返る(通常どおり処理する)。
    #[test]
    fn idle_passes_everything_through() {
        let mut h: KeyEffectOpenHold<&str> = KeyEffectOpenHold::new();
        assert_eq!(
            h.on_key(down(1), "k", scope(1, 10), Some(TickMs(100)), TickMs(1)),
            OnKey::Pass("k")
        );
        assert!(!h.is_holding());
        assert_eq!(h.release(None, scope(1, 10)), Released::Nothing);
    }

    /// 始める条件 (a)(b)(c) のどれかが欠ければ武装しない(T4・U5)。
    #[test]
    fn arm_requires_all_start_conditions() {
        let mut h: KeyEffectOpenHold<&str> = KeyEffectOpenHold::new();
        let base = arm_facts(100);
        for facts in [
            ArmFacts {
                gji_evidence_absent: false,
                ..base
            },
            ArmFacts {
                shm_accepted_for_focus: false,
                ..base
            },
            ArmFacts {
                predicted_open_at: None,
                ..base
            },
        ] {
            assert!(!h.arm(facts, scope(1, 10)), "{facts:?}");
            assert!(!h.is_armed());
        }
        assert!(h.arm(base, scope(1, 10)));
        assert!(h.is_armed());
    }

    /// 武装の後、Up(予測を立てたモードキー自身の Up 等)・リピートでは始めず、次の Down から始める。
    #[test]
    fn hold_starts_at_the_next_key_down_not_at_an_up() {
        let mut h = KeyEffectOpenHold::new();
        assert!(h.arm(arm_facts(100), scope(1, 10)));
        assert_eq!(
            h.on_key(
                up(9),
                "mode-up",
                scope(1, 10),
                Some(TickMs(100)),
                TickMs(110)
            ),
            OnKey::Pass("mode-up")
        );
        assert_eq!(
            h.on_key(
                repeat(9),
                "mode-rep",
                scope(1, 10),
                Some(TickMs(100)),
                TickMs(111)
            ),
            OnKey::Pass("mode-rep")
        );
        assert!(h.is_armed());
        assert!(matches!(
            h.on_key(down(1), "k", scope(1, 10), Some(TickMs(100)), TickMs(120)),
            OnKey::Started(_)
        ));
        assert_eq!(h.started_at(), Some(TickMs(120)));
        assert_eq!(h.pred_at_ms(), Some(TickMs(100)));
    }

    /// 次の打鍵までに予測が照合済み(または別の予測に置き換わった)・窓が変わったなら、始めずに武装を解く。
    #[test]
    fn stale_arm_is_dropped_without_holding() {
        let mut h = KeyEffectOpenHold::new();
        assert!(h.arm(arm_facts(100), scope(1, 10)));
        assert_eq!(
            h.on_key(down(1), "k", scope(1, 10), None, TickMs(120)),
            OnKey::Pass("k")
        );
        assert!(!h.is_armed());

        assert!(h.arm(arm_facts(200), scope(1, 10)));
        assert_eq!(
            h.on_key(down(1), "k", scope(1, 10), Some(TickMs(250)), TickMs(260)),
            OnKey::Pass("k")
        );
        assert!(!h.is_armed());

        assert!(h.arm(arm_facts(300), scope(1, 10)));
        assert_eq!(
            h.on_key(down(1), "k", scope(2, 20), Some(TickMs(300)), TickMs(310)),
            OnKey::Pass("k")
        );
        assert!(!h.is_armed());
    }

    /// U2: 同じ予測(`pred.at_ms`)では、解放後に掛け直さない。新しい予測なら再び対象になる。
    #[test]
    fn hold_is_once_per_prediction() {
        let (mut h, id) = started();
        assert!(matches!(
            h.release(Some(id), scope(1, 10)),
            Released::Replay(_)
        ));
        assert!(!h.arm(arm_facts(100), scope(1, 10)));
        assert_eq!(
            h.on_key(down(2), "o", scope(1, 10), Some(TickMs(100)), TickMs(150)),
            OnKey::Pass("o")
        );
        assert!(h.arm(arm_facts(400), scope(1, 10)));
    }

    /// 保持中は武装しない(再押下したモードキーは保持の中。再生の時点で改めて予測・武装される、T2)。
    #[test]
    fn cannot_arm_while_holding() {
        let (mut h, _) = started();
        assert!(!h.arm(arm_facts(130), scope(1, 10)));
        assert_eq!(h.len(), 1);
    }

    /// 解放の列は保持した順序どおり。モードキーの再押下(T2)も区別せず保持される。保持の中の Down の Up は足し、
    /// 保持の外の Down の Up は通す。
    #[test]
    fn release_preserves_order_and_pairs_ups_with_held_downs() {
        let (mut h, id) = started();
        assert_eq!(
            h.on_key(
                up(9),
                "mode-up",
                scope(1, 10),
                Some(TickMs(100)),
                TickMs(121)
            ),
            OnKey::Pass("mode-up")
        );
        assert_eq!(
            h.on_key(up(1), "k-up", scope(1, 10), Some(TickMs(100)), TickMs(122)),
            OnKey::Held
        );
        assert_eq!(
            h.on_key(
                down(9),
                "re-mode",
                scope(1, 10),
                Some(TickMs(100)),
                TickMs(123)
            ),
            OnKey::Held
        );
        assert_eq!(
            h.release(Some(id), scope(1, 10)),
            Released::Replay(vec!["k", "k-up", "re-mode"])
        );
        assert!(!h.is_holding());
    }

    /// 上限に達した新しい打鍵は、契機の打鍵を列の末尾に含めて全て解放する(追い越させない)。
    #[test]
    fn cap_releases_trigger_key_at_the_end() {
        let (mut h, _) = started();
        for (i, name) in ["k2", "k3", "k4", "k5", "k6", "k7", "k8"]
            .into_iter()
            .enumerate()
        {
            assert_eq!(
                h.on_key(
                    down(10 + i as u64),
                    name,
                    scope(1, 10),
                    Some(TickMs(100)),
                    TickMs(130)
                ),
                OnKey::Held
            );
        }
        assert_eq!(h.len(), KEY_EFFECT_OPEN_HOLD_CAP);
        let out = h.on_key(down(99), "k9", scope(1, 10), Some(TickMs(100)), TickMs(140));
        assert_eq!(
            out,
            OnKey::Release(Released::Replay(vec![
                "k", "k2", "k3", "k4", "k5", "k6", "k7", "k8", "k9"
            ]))
        );
        assert!(!h.is_holding());
    }

    /// 前面窓が保持の開始時と違えば、理由を問わず保持の中の打鍵はすべて `Discard`
    /// (別窓へ誤配送しない。利用者への通知は呼び出し側の責務、ADR-292 R6(ii)参照)。
    #[test]
    fn scope_change_discards_everything_regardless_of_release_reason() {
        let (mut h, id) = started();
        h.on_key(down(2), "o", scope(1, 10), Some(TickMs(100)), TickMs(130));
        assert_eq!(
            h.release(Some(id), scope(2, 20)),
            Released::Discard(vec!["k", "o"])
        );
    }

    /// 古い札(別の世代)の通知は、新しい保持を解放しない。
    #[test]
    fn stale_hold_id_does_not_release_a_new_hold() {
        let (mut h, old_id) = started();
        assert!(matches!(
            h.release(Some(old_id), scope(1, 10)),
            Released::Replay(_)
        ));
        assert!(h.arm(arm_facts(200), scope(1, 10)));
        let OnKey::Started(new_id) =
            h.on_key(down(3), "r", scope(1, 10), Some(TickMs(200)), TickMs(210))
        else {
            panic!("start");
        };
        assert_ne!(old_id, new_id);
        assert_eq!(h.release(Some(old_id), scope(1, 10)), Released::Nothing);
        assert!(h.is_holding());
        assert!(matches!(
            h.release(Some(new_id), scope(1, 10)),
            Released::Replay(_)
        ));
    }

    /// 解放判定の表(U1・R6(iv)・T1)。
    #[test]
    fn judge_table() {
        use ReleaseReason as R;
        use Verdict::{Keep, Release};
        let held = Some(ShmUse::Held);
        let split = Some(ShmUse::Split);
        let open = Some(ShmUse::Use {
            open: true,
            disagrees: false,
        });
        let closed = Some(ShmUse::Use {
            open: false,
            disagrees: true,
        });
        // U1: 読めないときだけ即座解放。「閉」の読みは即座解放しない(settle 前は Held で保持継続)。
        assert_eq!(judge(None, true, 0, 300), Release(R::Unreadable));
        assert_eq!(judge(held, true, 20, 300), Keep);
        assert_eq!(judge(split, true, 20, 300), Keep);
        assert_eq!(judge(open, true, 20, 300), Release(R::ConfirmedOpen));
        assert_eq!(judge(closed, true, 200, 300), Release(R::ObservedClosed));
        // 読みが確定しないまま、予測が他の経路で照合済み → 流す。
        assert_eq!(judge(held, false, 20, 300), Release(R::PredictionSettled));
        // 期限。
        assert_eq!(judge(held, true, 300, 300), Release(R::Deadline));
        assert_eq!(judge(split, true, 301, 300), Release(R::Deadline));
        // 確定した読みは期限より優先。
        assert_eq!(judge(open, true, 999, 300), Release(R::ConfirmedOpen));
    }

    /// T1: 保持の起点は予測の時刻と S1 の送信打刻の大きい方。保持中の打鍵(ローリング baseline)は入らない。
    /// 予測後に Mozc が書いた「開」の読み(`update_tick_ms` > 起点)は、保持中にどれだけ打鍵が来ても
    /// `decide_use` の高速路で `Use` になる。
    #[test]
    fn hold_baseline_ignores_held_keystrokes() {
        use crate::state::mozc_shm::{decide_use, CompositionMode, ShmState};
        assert_eq!(hold_baseline_ms(TickMs(100), 50), 100);
        assert_eq!(hold_baseline_ms(TickMs(100), 180), 180);
        let shm_open = ShmState {
            version: 5,
            tsf_open: true,
            tsf_mode: CompositionMode::from_u32(1),
            effective_open: true,
            effective_mode: CompositionMode::from_u32(1),
        };
        let pred_at = TickMs(1_000);
        let baseline = hold_baseline_ms(pred_at, 0);
        // Mozc がモードキーの 40ms 後に開いて書いた。保持中の打鍵でローリング baseline は 1_090 まで進んだが、使わない。
        let rolling = 1_090;
        let update_tick = 1_040;
        let now = 1_100;
        assert_eq!(
            decide_use(
                shm_open,
                update_tick,
                now,
                rolling,
                rolling,
                170,
                true,
                true
            ),
            ShmUse::Held,
            "ローリング baseline だと保持中の打鍵で解けない(T1 の欠陥)"
        );
        assert!(matches!(
            decide_use(
                shm_open,
                update_tick,
                now,
                baseline,
                baseline,
                170,
                true,
                true
            ),
            ShmUse::Use { open: true, .. }
        ));
        // 閉の読み(食い違い)は、起点から settle 経過までは Held、経過後に Use(R6(iv))。
        let shm_closed = ShmState {
            tsf_open: false,
            effective_open: false,
            ..shm_open
        };
        assert_eq!(
            decide_use(shm_closed, 900, 1_100, baseline, baseline, 170, true, false),
            ShmUse::Held
        );
        assert!(matches!(
            decide_use(shm_closed, 900, 1_170, baseline, baseline, 170, true, false),
            ShmUse::Use { open: false, .. }
        ));
    }

    /// 世代は増え、0 にならない。
    #[test]
    fn hold_ids_are_monotonic_and_nonzero() {
        let mut h: KeyEffectOpenHold<u8> = KeyEffectOpenHold::new();
        let mut last = 0;
        for i in 0..5_u64 {
            assert!(h.arm(arm_facts(100 + i), scope(1, 1)));
            let OnKey::Started(id) =
                h.on_key(down(1), 0, scope(1, 1), Some(TickMs(100 + i)), TickMs(i))
            else {
                panic!("start");
            };
            assert!(id > last);
            last = id;
            let _ = h.release(None, scope(1, 1));
        }
    }
}
