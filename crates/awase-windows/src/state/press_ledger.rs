//! 「この押下で既に書いた」の記録（`last_written_press`）と、同一押下の二重送信の防御（ADR-208 決定2 D1）。
//!
//! # 何を防ぐか
//!
//! 明示キーの 1 押下は、複数の経路（hook の shadow toggle と、Engine の `SetOpen`）が同じ打鍵に反応しうる
//! （sync キーが `keys.ime_on/off` にも割り当てられている等）。以前は経路ごとに `applied` の already-matched 省略が
//! 二重送信の防波堤だったが、ADR-208 D1 は stale な `applied` による省略（S-1）を押下の書き込みから外す。
//! そのため「この押下で既に書いた」ことを **押下 ID（`PressId`）** で記録し、同じ押下の後続の経路を省く
//! （BUG-113 の二重送信防止を `applied` から押下 ID へ移す）。
//!
//! # 予約は order の発行時点
//!
//! ImmCross の書き込みは async で、完了は WM 経由で後から届く。完了時に記録すると、同じ打鍵の Engine の `SetOpen`
//! （同期に評価される）が先に来て二重に送る。そのため [`PressLedger::claim`] は **order を発行する直前**に呼ぶ。
//! **非同期**の書き込みが書けなかった場合（UnsafeToToggle/Failed）は予約を解かない（完了が後から届き、同一押下内の再試行は
//! しない。次の押下で直る = INV-L2。ADR-208 の例外）。**同期**の書き込みは書けたかを同じ呼び出しで知っているので、
//! 何も送らなかったとき（`outcome_sent_nothing`）だけ [`PressLedger::release`] で解く（同じ押下の次の経路が書ける）。
//!
//! # 衝突（同じ押下で向きが違う 2 経路）の優先順位
//!
//! 現状の評価順は hook の shadow → Engine の `on_input` → executor の `SetOpen` で、shadow が先に書く。
//! shadow が書いた後に、同じ押下で逆向きの `SetOpen` が Engine から来たら **Engine の明示コンボを優先して上書きする**
//! （[`PressClaim::ConflictEngineWins`]。Engine 側はユーザーが設定した `keys.*` で、shadow は `sync_direction` や
//! 静的な開閉キー由来）。向きが同じなら省く（[`PressClaim::Duplicate`]）。shadow が後に来る順序は現状ありえないが、
//! 来たら先に書いた Engine を保つ（[`PressClaim::ConflictKept`]）。
//!
//! 純粋（Win32・`ImeStateHub` に依存しない）で、本番（`ImeStateHub::claim_press_write`）と全列挙テスト
//! （`explicit_press` のモデル）が同じ [`PressLedger`] を呼ぶ。

use awase::platform::ImeOpenOutcome;
use awase::types::PressId;

/// 同じ押下で既に書いた（`Duplicate`/`ConflictKept`）ため**書かなかった** Engine の `SetOpen` が完了へ流す outcome。
///
/// `AlreadyMatched` を返してはならない: `ImeEvent::from_apply_outcome` が `ImeApplySucceeded` にし、`handle_engine_set_open`
/// の pending 世代と一致して受理され、**書いていない押下が `applied=Confirmed(open)` になる**（先行の書き込みが
/// UnsafeToToggle・async の Failed だったとき嘘の確認になり、TsfNative では S-1 を L1 自身が作る。PR #419 Opus M-1）。
/// 「送っていない」outcome（`UnsafeToToggle`。`record_ime_apply_result` が pending だけ解放し `applied` を動かさない）を返す。
pub const DUPLICATE_OUTCOME: ImeOpenOutcome = ImeOpenOutcome::UnsafeToToggle;

/// この outcome は何も送っていない（VK も IMM もメッセージも出していない）か。同期の書き込みでこれなら、同一押下の
/// 予約を解いてよい（次の経路〈同じ押下の Engine 等〉が改めて書ける。PR #419 Opus M-2）。`Failed`（一部送った可能性）・
/// `AlreadyMatched`（実 IME が既に向き）は解かない。
#[must_use]
pub const fn outcome_sent_nothing(outcome: ImeOpenOutcome) -> bool {
    matches!(
        outcome,
        ImeOpenOutcome::UnsafeToToggle | ImeOpenOutcome::NotOwned | ImeOpenOutcome::Unwarranted
    )
}

/// 書き込みを起案する経路（衝突の優先順位に使う）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PressSource {
    /// hook の shadow toggle（`kp_shadow_actuate`）。
    Shadow,
    /// Engine の `ImeEffect::SetOpen`（executor の `dispatch_ime_set_open`）。
    Engine,
}

impl PressSource {
    /// ログ・journal 用の名前。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Shadow => "shadow",
            Self::Engine => "engine",
        }
    }
}

/// [`PressLedger::claim`] の結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PressClaim {
    /// 押下 ID が無い（自動リピート・drift correction 等）。記録に触れず、従来どおり `applied` の already-matched 省略に任せる。
    Unpressed,
    /// この押下で最初の書き込み。予約した。
    Fresh,
    /// この押下で同じ向きを既に予約済み。書かずに省く（BUG-113: 二重送信を防ぐ）。
    Duplicate,
    /// この押下で逆向きを既に予約済みだが、Engine の明示コンボが優先するので書く（予約を Engine の向きに更新した）。
    ConflictEngineWins {
        /// 先に予約されていた向き（shadow 側）。
        reserved: bool,
    },
    /// この押下で逆向きを既に予約済みで、後から来た経路（shadow）は優先されないので書かない（予約は保つ）。
    ConflictKept {
        /// 保たれた予約の向き。
        reserved: bool,
    },
}

impl PressClaim {
    /// この claim の経路が書き込みを進めてよいか。
    #[must_use]
    pub const fn writes(self) -> bool {
        matches!(
            self,
            Self::Unpressed | Self::Fresh | Self::ConflictEngineWins { .. }
        )
    }

    /// 同じ押下の別経路と衝突したか（向きが違う場合）。テストだけが使う(手書きの `[press-ledger]` ログ行の撤去後、本番の利用者は無い)。
    #[cfg(test)]
    #[must_use]
    pub const fn is_conflict(self) -> bool {
        matches!(
            self,
            Self::ConflictEngineWins { .. } | Self::ConflictKept { .. }
        )
    }

    /// ログ・journal 用の名前。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Unpressed => "unpressed",
            Self::Fresh => "fresh",
            Self::Duplicate => "duplicate",
            Self::ConflictEngineWins { .. } => "conflict_engine_wins",
            Self::ConflictKept { .. } => "conflict_kept",
        }
    }
}

/// 直近に書き込みを予約した押下と向き（`last_written_press`）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PressLedger {
    last: Option<(PressId, bool)>,
}

impl PressLedger {
    /// 直近の予約（押下 ID と向き）。
    #[must_use]
    pub const fn last_written(&self) -> Option<(PressId, bool)> {
        self.last
    }

    /// 押下 `press` の向き `open` の予約を解く（同期の書き込みが何も送らなかったとき。`outcome_sent_nothing`）。
    /// 現在の予約が `(press, open)` と一致するときだけ解く（衝突で上書きされた別向きの予約や別の押下には触れない）。
    /// 戻り値: 解いたか。
    pub fn release(&mut self, press: Option<PressId>, open: bool) -> bool {
        match (press, self.last) {
            (Some(p), Some((lp, lo))) if p == lp && lo == open => {
                self.last = None;
                true
            }
            _ => false,
        }
    }

    /// 押下 `press` の向き `open` の書き込みを予約する（order の発行直前に呼ぶ）。
    ///
    /// 押下 ID が無い（`None`）ときは何も記録しない。違う押下の予約は上書きする（押下 ID は単調増加で、
    /// 古い押下が後から来ても新しい押下の予約を壊さないよう「最後の予約」だけを持つ）。
    pub fn claim(&mut self, press: Option<PressId>, open: bool, source: PressSource) -> PressClaim {
        let Some(press) = press else {
            return PressClaim::Unpressed;
        };
        match self.last {
            Some((p, reserved)) if p == press => {
                if reserved == open {
                    PressClaim::Duplicate
                } else if source == PressSource::Engine {
                    self.last = Some((press, open));
                    PressClaim::ConflictEngineWins { reserved }
                } else {
                    PressClaim::ConflictKept { reserved }
                }
            }
            _ => {
                self.last = Some((press, open));
                PressClaim::Fresh
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_only_drops_the_exact_reservation_so_the_next_route_can_write() {
        let mut l = PressLedger::default();
        l.claim(p(1), true, PressSource::Shadow);
        assert!(!l.release(p(1), false), "向きが違う予約は解かない");
        assert!(!l.release(p(2), true), "別の押下の予約は解かない");
        assert!(!l.release(None, true));
        assert!(l.release(p(1), true));
        assert_eq!(l.last_written(), None);
        // 解いた後は、同じ押下の次の経路が Fresh として書ける（「絶対指定は 1 回」）。
        assert_eq!(l.claim(p(1), true, PressSource::Engine), PressClaim::Fresh);
    }

    #[test]
    fn only_outcomes_that_sent_nothing_release_the_reservation() {
        use ImeOpenOutcome::*;
        for o in [UnsafeToToggle, NotOwned, Unwarranted] {
            assert!(outcome_sent_nothing(o), "{o:?}");
        }
        // 一部送った可能性（Failed）・実 IME が既に向き（AlreadyMatched）・書いた（Applied*）は解かない。
        for o in [Applied, AppliedWithoutSendInput, AlreadyMatched, Failed] {
            assert!(!outcome_sent_nothing(o), "{o:?}");
        }
    }

    /// M-1: 書かなかった重複の完了は「送っていない」outcome。`wrote_open_state` でも `AlreadyMatched` でもない。
    #[test]
    fn duplicate_completion_is_a_not_sent_outcome() {
        assert!(outcome_sent_nothing(DUPLICATE_OUTCOME));
        assert!(!DUPLICATE_OUTCOME.wrote_open_state());
        assert_ne!(DUPLICATE_OUTCOME, ImeOpenOutcome::AlreadyMatched);
    }

    fn p(n: u64) -> Option<PressId> {
        Some(PressId::new(n))
    }

    #[test]
    fn unpressed_never_touches_the_ledger_and_always_writes() {
        let mut l = PressLedger::default();
        assert_eq!(
            l.claim(None, true, PressSource::Engine),
            PressClaim::Unpressed
        );
        assert_eq!(l.last_written(), None);
        // 押下の予約があっても、リピート等（press なし）は従来どおり書く判断に回す。
        l.claim(p(1), true, PressSource::Shadow);
        assert_eq!(
            l.claim(None, true, PressSource::Engine),
            PressClaim::Unpressed
        );
        assert_eq!(l.last_written(), Some((PressId::new(1), true)));
        assert!(PressClaim::Unpressed.writes());
    }

    #[test]
    fn first_claim_for_a_press_is_fresh_and_reserves() {
        let mut l = PressLedger::default();
        assert_eq!(l.claim(p(5), false, PressSource::Shadow), PressClaim::Fresh);
        assert_eq!(l.last_written(), Some((PressId::new(5), false)));
        assert!(PressClaim::Fresh.writes());
    }

    #[test]
    fn same_press_same_direction_is_a_duplicate_from_either_route() {
        for (first, second) in [
            (PressSource::Shadow, PressSource::Engine),
            (PressSource::Engine, PressSource::Shadow),
            (PressSource::Shadow, PressSource::Shadow),
            (PressSource::Engine, PressSource::Engine),
        ] {
            for open in [false, true] {
                let mut l = PressLedger::default();
                assert_eq!(l.claim(p(2), open, first), PressClaim::Fresh);
                let c = l.claim(p(2), open, second);
                assert_eq!(c, PressClaim::Duplicate, "{first:?}→{second:?} open={open}");
                assert!(!c.writes(), "同じ押下の同じ向きは書かない（BUG-113）");
                assert_eq!(l.last_written(), Some((PressId::new(2), open)));
            }
        }
    }

    #[test]
    fn engine_overrides_an_earlier_opposite_shadow_write() {
        for shadow_open in [false, true] {
            let mut l = PressLedger::default();
            assert_eq!(
                l.claim(p(3), shadow_open, PressSource::Shadow),
                PressClaim::Fresh
            );
            let c = l.claim(p(3), !shadow_open, PressSource::Engine);
            assert_eq!(
                c,
                PressClaim::ConflictEngineWins {
                    reserved: shadow_open
                }
            );
            assert!(c.writes() && c.is_conflict());
            // 予約は Engine の向きに更新される（以降の同押下の Engine 再送は Duplicate）。
            assert_eq!(l.last_written(), Some((PressId::new(3), !shadow_open)));
            assert_eq!(
                l.claim(p(3), !shadow_open, PressSource::Engine),
                PressClaim::Duplicate
            );
        }
    }

    #[test]
    fn shadow_never_overrides_an_earlier_opposite_engine_write() {
        let mut l = PressLedger::default();
        assert_eq!(l.claim(p(4), true, PressSource::Engine), PressClaim::Fresh);
        let c = l.claim(p(4), false, PressSource::Shadow);
        assert_eq!(c, PressClaim::ConflictKept { reserved: true });
        assert!(!c.writes() && c.is_conflict());
        assert_eq!(l.last_written(), Some((PressId::new(4), true)));
    }

    #[test]
    fn a_new_press_replaces_the_reservation_even_with_the_same_direction() {
        let mut l = PressLedger::default();
        l.claim(p(7), true, PressSource::Shadow);
        // 別の押下は、前の押下と同じ向きでも新規（stale な applied ではなく押下単位で数える）。
        assert_eq!(l.claim(p(8), true, PressSource::Shadow), PressClaim::Fresh);
        assert_eq!(l.last_written(), Some((PressId::new(8), true)));
    }

    /// 一つの押下の全パターン（先後 × 向き）で、書き込みは「Engine の向き」に収束し、同じ向きの二重送信は無い。
    #[test]
    fn per_press_writes_converge_to_the_engine_direction_without_same_direction_doubles() {
        for shadow_open in [false, true] {
            for engine_open in [false, true] {
                let mut l = PressLedger::default();
                let mut writes = Vec::new();
                // 現状の評価順: shadow → Engine。
                if l.claim(p(9), shadow_open, PressSource::Shadow).writes() {
                    writes.push(shadow_open);
                }
                if l.claim(p(9), engine_open, PressSource::Engine).writes() {
                    writes.push(engine_open);
                }
                assert_eq!(
                    writes.last().copied(),
                    Some(engine_open),
                    "最終の書き込みは Engine の向き"
                );
                if shadow_open == engine_open {
                    assert_eq!(writes.len(), 1, "同じ向きは 1 回だけ");
                } else {
                    assert_eq!(
                        writes,
                        vec![shadow_open, engine_open],
                        "逆向きは Engine が上書き"
                    );
                }
            }
        }
    }
}
