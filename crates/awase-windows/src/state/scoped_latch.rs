/// 「あるスコープ S の中でだけ有効な、一度きりの予約」を表す汎用 latch。
///
/// `peek` はスコープ不一致をその場で失効させる。呼び出し側に「disarm し忘れ」を
/// 残さないのが本型の存在意義であり、スコープを持たないまま次の1キーを
/// 待ち続ける形を構造的に不可能にする。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ScopedOneShot<S: Copy + PartialEq, T: Copy = ()> {
    armed: Option<(S, T)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScopeCheck<T> {
    /// 予約なし。
    NotArmed,
    /// 予約はあったがスコープが変わっていた。この呼び出しで失効させた。
    Expired,
    /// 予約が有効。payload を返す（消費はしない）。
    Live(T),
}

impl<S: Copy + PartialEq, T: Copy> ScopedOneShot<S, T> {
    pub(crate) const fn new() -> Self {
        Self { armed: None }
    }

    pub(crate) fn arm(&mut self, scope: S, payload: T) {
        self.armed = Some((scope, payload));
    }

    pub(crate) fn peek(&mut self, now: S) -> ScopeCheck<T> {
        match self.armed {
            None => ScopeCheck::NotArmed,
            Some((scope, _)) if scope != now => {
                self.armed = None;
                ScopeCheck::Expired
            }
            Some((_, payload)) => ScopeCheck::Live(payload),
        }
    }

    pub(crate) fn disarm(&mut self) -> Option<T> {
        self.armed.take().map(|(_, payload)| payload)
    }

    pub(crate) const fn is_armed(&self) -> bool {
        self.armed.is_some()
    }
}

impl<S: Copy + PartialEq, T: Copy> Default for ScopedOneShot<S, T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{ScopeCheck, ScopedOneShot};

    #[test]
    fn one_shot_scope_lifecycle_is_explicit() {
        let mut latch = ScopedOneShot::<u32, u8>::new();
        assert_eq!(latch.peek(1), ScopeCheck::NotArmed);
        assert_eq!(latch.disarm(), None);

        latch.arm(10, 42);
        assert!(latch.is_armed());
        assert_eq!(latch.peek(10), ScopeCheck::Live(42));
        assert!(latch.is_armed());

        assert_eq!(latch.peek(11), ScopeCheck::Expired);
        assert!(!latch.is_armed());
        assert_eq!(latch.peek(10), ScopeCheck::NotArmed);
    }

    #[test]
    fn disarm_is_idempotent() {
        let mut latch = ScopedOneShot::<u32, u8>::new();
        latch.arm(7, 9);
        assert_eq!(latch.disarm(), Some(9));
        assert_eq!(latch.disarm(), None);
        assert_eq!(latch.peek(7), ScopeCheck::NotArmed);
    }

    #[test]
    fn notification_round_trip_keeps_latch_until_a_foreign_key_is_evaluated() {
        let mut latch = ScopedOneShot::<(u32, isize), ()>::new();
        let terminal = (100, 0x111);
        let toast = (200, 0x222);

        latch.arm(terminal, ());
        assert_eq!(latch.peek(terminal), ScopeCheck::Live(()));
        assert_eq!(latch.peek(terminal), ScopeCheck::Live(()));

        assert_eq!(latch.peek(toast), ScopeCheck::Expired);
        assert_eq!(latch.peek(terminal), ScopeCheck::NotArmed);
    }
}
