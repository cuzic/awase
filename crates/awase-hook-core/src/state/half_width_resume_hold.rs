//! ADR-245 追補 2 R2: MS-IME 本体の戻り待ちの復元(`FromResume`)で、戻った窓の最初の打鍵を保持し、R1 の観測が
//! 記録されたら(または期限・上限・契機で)順序どおり再生する状態機械。
//!
//! 純粋: 時刻・前面窓のスコープは引数で受け、保持する打鍵 `T` は呼び出し側の型(殻では `RawKeyEvent`)。
//! 打鍵を捨てるのは前面窓が保持の開始時と違うときだけ(`release` の窓の照合。ADR-245 追補 2 の契機の表)。
//!
//! 契機の表(ADR 決定 4):
//! - 保持の中の Down に対応する Up・リピートは保持に足す。保持の外で既に OS に届いた Down の Up は足さず通す
//!   (押されたまま残さない・順序は Down を持つ打鍵だけに適用)。
//! - 新しい Down は保持に足す。上限に達したら、その Down も同じ列の末尾に含めて全て解放する(追い越させない)。
//! - 解放(`release`)は、保持の世代 `hold_id` が合うときだけ(古い札の通知が新しい保持を解放しない)。
//!   前面窓が保持の開始時と違えば、保持の中の打鍵はすべて捨てる。違わなければ順序どおり再生する。

use crate::state::foreground_scope::ForegroundScope;
use crate::state::TickMs;

/// 保持の上限(打鍵数)。復元の観測の記録は典型 ~7ms なので、通常は 1〜2 件で解放される。
pub const RESUME_HOLD_CAP: usize = 4;

/// 保持している 1 打鍵の事実(判断に使う分だけ)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyFacts {
    /// 物理キーの同一性。Down と Up で同じ値になること(`vk` ではなく scan 由来。左右の Alt/Ctrl は別の値)。
    pub identity: u64,
    /// KeyDown か(false = KeyUp)。
    pub is_down: bool,
    /// OS の自動リピートによる Down か(`RawKeyEvent::was_down`)。
    pub is_repeat: bool,
}

/// 解放(または通知)の理由。完了の札の通知とログに使う。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseReason {
    /// R1 の観測が記録された。
    Observed,
    /// 復元タスクが `ActuationTarget::capture` に失敗した。
    CaptureFailed,
    /// 復元タスクが `still_owner` 偽(新しい hold が始まった)で終わった。
    NotOwner,
    /// 1 回目の書き込みが `Failed`/`Aborted` だった。
    FirstWriteFailed,
    /// R1 の読みで `ime_on` が `None` だった。
    ReadNone,
    /// R1 の記録が admission で棄却された。
    Rejected,
    /// 札が通知されずに破棄された(保険)。
    Dropped,
    /// `HALF_WIDTH_RESUME_HOLD_MAX_MS` の一発タイマー。
    Deadline,
    /// 保持が上限に達した。
    Cap,
    /// フォーカスの変化(`ir_notify_focus_changed`)。
    FocusChanged,
    /// `WM_PANIC_RESET`。
    PanicReset,
    /// 設定の再読み込み。
    ConfigReload,
}

impl ReleaseReason {
    /// `WM_RESUME_HOLD_DONE` の lparam に載せる符号。
    #[must_use]
    pub const fn code(self) -> isize {
        match self {
            Self::Observed => 0,
            Self::CaptureFailed => 1,
            Self::NotOwner => 2,
            Self::FirstWriteFailed => 3,
            Self::ReadNone => 4,
            Self::Rejected => 5,
            Self::Dropped => 6,
            Self::Deadline => 7,
            Self::Cap => 8,
            Self::FocusChanged => 9,
            Self::PanicReset => 10,
            Self::ConfigReload => 11,
        }
    }

    /// `code` の逆。未知の符号は `Dropped`(通知の経路の保険と同じ扱い)。
    #[must_use]
    pub const fn from_code(code: isize) -> Self {
        match code {
            0 => Self::Observed,
            1 => Self::CaptureFailed,
            2 => Self::NotOwner,
            3 => Self::FirstWriteFailed,
            4 => Self::ReadNone,
            5 => Self::Rejected,
            7 => Self::Deadline,
            8 => Self::Cap,
            9 => Self::FocusChanged,
            10 => Self::PanicReset,
            11 => Self::ConfigReload,
            _ => Self::Dropped,
        }
    }

    /// ログ用の短い名前(CI のログ断片検査が数える)。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Observed => "Observed",
            Self::CaptureFailed => "CaptureFailed",
            Self::NotOwner => "NotOwner",
            Self::FirstWriteFailed => "FirstWriteFailed",
            Self::ReadNone => "ReadNone",
            Self::Rejected => "Rejected",
            Self::Dropped => "Dropped",
            Self::Deadline => "Deadline",
            Self::Cap => "Cap",
            Self::FocusChanged => "FocusChanged",
            Self::PanicReset => "PanicReset",
            Self::ConfigReload => "ConfigReload",
        }
    }
}

/// 解放の結果。保持の中身(`T`)を返す。
#[derive(Debug, PartialEq, Eq)]
pub enum Released<T> {
    /// 保持が無い、または `hold_id` が今の保持と違う(古い札の通知)。何もしない。
    Nothing,
    /// 順序どおり再生する。
    Replay(Vec<T>),
    /// 前面窓が保持の開始時と違うので捨てる。
    Discard(Vec<T>),
}

/// 保持中に打鍵が来たときの扱い。
#[derive(Debug, PartialEq, Eq)]
pub enum OnKey<T> {
    /// 保持が無い、または保持に足さない打鍵。呼び出し側が通常どおり処理する(打鍵を返す)。
    Pass(T),
    /// 保持に足した(呼び出し側は `Consumed` を返す)。
    Held,
    /// 上限に達した。この打鍵を列の末尾に含めて解放する(窓の照合は `release` と同じ)。
    Release(Released<T>),
}

#[derive(Debug)]
struct Active<T> {
    id: u64,
    scope: ForegroundScope,
    started_at: TickMs,
    items: Vec<(KeyFacts, T)>,
}

/// 最初の打鍵の保持。同時に 1 つだけ。
#[derive(Debug)]
pub struct ResumeHold<T> {
    active: Option<Active<T>>,
    next_id: u64,
}

impl<T> Default for ResumeHold<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> ResumeHold<T> {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            active: None,
            next_id: 1,
        }
    }

    #[must_use]
    pub const fn is_holding(&self) -> bool {
        self.active.is_some()
    }

    #[must_use]
    pub fn hold_id(&self) -> Option<u64> {
        self.active.as_ref().map(|a| a.id)
    }

    /// 保持の開始時刻(ログの hold_ms 用)。
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

    /// 保持を始める。既に保持中なら始めず `None`(呼び出し側は通常どおり処理する)。
    /// 始めたら新しい `hold_id` を返す(完了の札に持たせる)。
    pub fn start(
        &mut self,
        scope: ForegroundScope,
        now: TickMs,
        facts: KeyFacts,
        item: T,
    ) -> Option<u64> {
        if self.active.is_some() {
            return None;
        }
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        self.active = Some(Active {
            id,
            scope,
            started_at: now,
            items: vec![(facts, item)],
        });
        Some(id)
    }

    /// 保持中に打鍵が来た。保持が無ければ `Pass`。
    pub fn on_key(&mut self, facts: KeyFacts, item: T, current: ForegroundScope) -> OnKey<T> {
        // 保持を一旦取り出し、続けるなら戻す(`expect` を使わない)。
        let Some(mut active) = self.active.take() else {
            return OnKey::Pass(item);
        };
        let has_down_of_same_key = active
            .items
            .iter()
            .any(|(f, _)| f.is_down && f.identity == facts.identity);
        if !facts.is_down || facts.is_repeat {
            // Up・リピート: 対応する Down が保持の中にあるときだけ足す(無ければ保持の外で既に届いた Down のもの)。
            if has_down_of_same_key {
                active.items.push((facts, item));
                self.active = Some(active);
                return OnKey::Held;
            }
            self.active = Some(active);
            return OnKey::Pass(item);
        }
        // 新しい Down。
        active.items.push((facts, item));
        if active.items.len() > RESUME_HOLD_CAP {
            // 契機の打鍵も同じ列の末尾に含めて解放する(追い越させない)。
            return OnKey::Release(Self::finish(active, current));
        }
        self.active = Some(active);
        OnKey::Held
    }

    /// 解放する。`hold_id` が `Some` なら今の保持と一致するときだけ(古い札の通知を無視する)。
    /// 前面窓が保持の開始時と違えば捨てる。
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
        let items: Vec<T> = active.items.into_iter().map(|(_, t)| t).collect();
        if active.scope == current {
            Released::Replay(items)
        } else {
            Released::Discard(items)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(pid: u32, hwnd: isize) -> ForegroundScope {
        ForegroundScope { pid, hwnd }
    }

    const A: u64 = 0x11;
    const B: u64 = 0x22;

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

    fn started() -> (ResumeHold<&'static str>, u64) {
        let mut h = ResumeHold::new();
        let id = h
            .start(scope(1, 10), TickMs(100), down(A), "A-down")
            .expect("start");
        (h, id)
    }

    /// 保持が無いとき、打鍵はそのまま返る(通常どおり処理する)。
    #[test]
    fn idle_passes_everything_through() {
        let mut h: ResumeHold<&str> = ResumeHold::new();
        assert_eq!(h.on_key(down(A), "x", scope(1, 10)), OnKey::Pass("x"));
        assert!(!h.is_holding());
        assert_eq!(h.release(None, scope(1, 10)), Released::Nothing);
    }

    /// 保持中は同じ保持を二重に始めない。
    #[test]
    fn cannot_start_twice() {
        let (mut h, _) = started();
        assert_eq!(h.start(scope(1, 10), TickMs(101), down(B), "B"), None);
        assert_eq!(h.len(), 1);
    }

    /// 解放の列は保持した順序どおり(Down → Up → 次の Down)。
    #[test]
    fn release_preserves_order() {
        let (mut h, id) = started();
        assert_eq!(h.on_key(up(A), "A-up", scope(1, 10)), OnKey::Held);
        assert_eq!(h.on_key(down(B), "B-down", scope(1, 10)), OnKey::Held);
        assert_eq!(
            h.release(Some(id), scope(1, 10)),
            Released::Replay(vec!["A-down", "A-up", "B-down"])
        );
        assert!(!h.is_holding());
    }

    /// 保持の外で既に OS に届いた Down の Up は保持に足さず通す(押されたまま残さない)。
    #[test]
    fn up_of_key_pressed_before_hold_passes() {
        let (mut h, _) = started();
        assert_eq!(h.on_key(up(B), "B-up", scope(1, 10)), OnKey::Pass("B-up"));
        assert_eq!(h.len(), 1);
    }

    /// リピートは同じ同一性の Down が保持の中にあれば足し、無ければ(B で押されたままのキー)通す。
    #[test]
    fn repeat_is_held_only_with_matching_down() {
        let (mut h, _) = started();
        assert_eq!(h.on_key(repeat(A), "A-rep", scope(1, 10)), OnKey::Held);
        assert_eq!(
            h.on_key(repeat(B), "B-rep", scope(1, 10)),
            OnKey::Pass("B-rep")
        );
    }

    /// 上限に達した新しい Down は、契機の打鍵を列の末尾に含めて全て解放する(追い越させない)。
    #[test]
    fn cap_releases_trigger_key_at_the_end() {
        let (mut h, _) = started();
        for (i, name) in ["k2", "k3", "k4"].into_iter().enumerate() {
            assert_eq!(
                h.on_key(down(100 + i as u64), name, scope(1, 10)),
                OnKey::Held
            );
        }
        assert_eq!(h.len(), RESUME_HOLD_CAP);
        let out = h.on_key(down(999), "k5", scope(1, 10));
        assert_eq!(
            out,
            OnKey::Release(Released::Replay(vec!["A-down", "k2", "k3", "k4", "k5"]))
        );
        assert!(!h.is_holding());
    }

    /// 前面窓が保持の開始時と違えば、保持の中の打鍵はすべて捨てる(別窓へ誤配送しない)。
    #[test]
    fn scope_change_discards_everything() {
        let (mut h, id) = started();
        h.on_key(up(A), "A-up", scope(1, 10));
        assert_eq!(
            h.release(Some(id), scope(2, 20)),
            Released::Discard(vec!["A-down", "A-up"])
        );
    }

    /// 同じプロセスの別窓へ移った場合も捨てる(スコープは pid と hwnd の組)。
    #[test]
    fn same_process_other_window_discards() {
        let (mut h, _) = started();
        assert_eq!(
            h.release(None, scope(1, 11)),
            Released::Discard(vec!["A-down"])
        );
    }

    /// 上限での解放にも窓の照合が掛かる。
    #[test]
    fn cap_release_also_checks_scope() {
        let (mut h, _) = started();
        for i in 0..3 {
            h.on_key(down(100 + i), "k", scope(1, 10));
        }
        assert!(matches!(
            h.on_key(down(999), "k5", scope(9, 9)),
            OnKey::Release(Released::Discard(_))
        ));
    }

    /// 古い札(別の世代)の通知は、新しい保持を解放しない。
    #[test]
    fn stale_hold_id_does_not_release_a_new_hold() {
        let (mut h, old_id) = started();
        assert!(matches!(
            h.release(Some(old_id), scope(1, 10)),
            Released::Replay(_)
        ));
        let new_id = h
            .start(scope(1, 10), TickMs(200), down(B), "B-down")
            .expect("start");
        assert_ne!(old_id, new_id);
        assert_eq!(h.release(Some(old_id), scope(1, 10)), Released::Nothing);
        assert!(h.is_holding());
        assert!(matches!(
            h.release(Some(new_id), scope(1, 10)),
            Released::Replay(_)
        ));
    }

    /// 捨てた Down に対応する Up が後から来ても、保持は無いので通る(OS から見れば Down の無い Up で無害)。
    #[test]
    fn up_after_discard_passes() {
        let (mut h, _) = started();
        let _ = h.release(None, scope(2, 20));
        assert_eq!(h.on_key(up(A), "A-up", scope(2, 20)), OnKey::Pass("A-up"));
    }

    /// 理由の符号は往復できる(完了の札の通知に載せる)。未知の符号は Dropped。
    #[test]
    fn reason_code_roundtrip() {
        for r in [
            ReleaseReason::Observed,
            ReleaseReason::CaptureFailed,
            ReleaseReason::NotOwner,
            ReleaseReason::FirstWriteFailed,
            ReleaseReason::ReadNone,
            ReleaseReason::Rejected,
            ReleaseReason::Dropped,
            ReleaseReason::Deadline,
            ReleaseReason::Cap,
            ReleaseReason::FocusChanged,
            ReleaseReason::PanicReset,
            ReleaseReason::ConfigReload,
        ] {
            assert_eq!(ReleaseReason::from_code(r.code()), r, "{}", r.label());
        }
        assert_eq!(ReleaseReason::from_code(99), ReleaseReason::Dropped);
    }

    /// 世代は増え、0 にならない。
    #[test]
    fn hold_ids_are_monotonic_and_nonzero() {
        let mut h: ResumeHold<u8> = ResumeHold::new();
        let mut last = 0;
        for i in 0..5 {
            let id = h.start(scope(1, 1), TickMs(i), down(A), 0).expect("start");
            assert!(id > last);
            last = id;
            let _ = h.release(None, scope(1, 1));
        }
    }
}
