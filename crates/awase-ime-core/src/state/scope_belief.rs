//! ADR-282 候補 Z の純粋な核: IME の保持単位(スコープ)ごとに「最後に受け入れた主張」をラッチする belief。
//!
//! **V0(擬似 IME・擬似窓の上で S′ と同じオラクルで比べる)専用で、本番からは呼ばない**(ADR-282 §6.4)。
//! 本番の belief(`ImeModel`・`ImeStateHub`)は変えない。
//!
//! 規則(ADR-282 §4 の要約。所有者の決定 O1〜O7 に従う):
//! - スコープの行は窓切替で消さない。戻ればその主張がそのまま効く(O3・O4。寿命で後退しない = I6)。
//! - 明示操作(利用者の明示 ON/OFF・表のセルで決まるモードキー)は常に主張を覆す(O3)。値は押した後の状態(L1′)。
//! - 観測は、読めない窓の IMM 系(反証不可、BUG-223(a)・BUG-219)を捨て、Low は主張なし・既定だけを覆し、
//!   Medium 以上は明示操作の柵(`KEY_EFFECT_SETTLE_MS`)より後に読み始めたものだけが覆す。古い High は新しい Medium に勝たない。
//! - 不確実なとき(主張なし)は ON(かな)に倒す(O2)。読める窓でも最初の観測が来るまでは同じ。
//! - 入場時に開を引き継ぐ窓の種類(O5: MS-IME 本体の Win32 EDIT → Windows Terminal)は、直前のスコープの開の主張が
//!   この窓の主張より新しければ開を引き継ぐ(開だけ。閉・変換モードは引き継がない)。それ以外は独立(O6)。
//!
//! 時刻は呼び出し元が渡す単調な ms(壁時計を読まない)。

use super::ime_event::{ObservationConfidence, ObservationSource};
use awase::engine::{AssumedReason, InputModeState};

/// スコープの鍵(V0 では擬似窓のスレッド。本番で使うなら (pid, tid) の正規化が要る、ADR-282 §4.1)。
pub type ScopeKey = u32;

/// 入力モードの軸の主張(O1: かな = ひらがな・カタカナ)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeClass {
    Kana,
    Eisu,
    KanaInputMethod,
}

/// 主張の根拠。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Basis {
    /// 明示操作(押した後の状態)。
    Explicit,
    /// 受け入れた観測。
    Observed(ObservationSource, ObservationConfidence),
    /// 入場時に直前のスコープの開を引き継いだ(O5)。
    Inherited,
}

impl Basis {
    /// golden・分類に出す短い名前。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Explicit => "z-explicit",
            Self::Observed(_, ObservationConfidence::High) => "z-observed-high",
            Self::Observed(_, ObservationConfidence::Medium) => "z-observed-medium",
            Self::Observed(_, ObservationConfidence::Low) => "z-observed-low",
            Self::Inherited => "z-inherited",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Claim<T> {
    value: T,
    basis: Basis,
    /// 証拠の時刻(観測は読み取り開始時刻)。
    at_ms: u64,
    /// この時刻より前に読み始めた観測では覆さない(明示操作の時刻 + settle)。
    fence_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct ScopeRow {
    open: Option<Claim<bool>>,
    mode: Option<Claim<ModeClass>>,
}

/// 窓の種類のうち、Z の規則に効くもの。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowFacts {
    /// IMM 系の観測(`ObserverPoll`・`ImmCrossProbe`・`FocusProbe`)を反証に使える窓か(信頼表、ADR-282 §4.4)。
    pub imm_trusted: bool,
    /// 入場時に直前のスコープの開を引き継ぐ窓の種類か(O5)。
    pub inherit_open_on_entry: bool,
}

/// Z に流す事象。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeEvent {
    /// フォーカスがスコープ `scope` の窓へ移った(同じスコープの別の窓でもよい)。
    Focus {
        scope: ScopeKey,
        facts: WindowFacts,
        at_ms: u64,
    },
    /// 明示操作。値は押した後の状態(変化が無くても記録する、L1′)。
    Explicit {
        open: Option<bool>,
        mode: Option<ModeClass>,
        at_ms: u64,
    },
    /// 観測。`imm_family` は IMM 系の源か(読めない窓では捨てる)。
    Observed {
        open: Option<bool>,
        mode: Option<ModeClass>,
        source: ObservationSource,
        confidence: ObservationConfidence,
        imm_family: bool,
        read_started_ms: u64,
    },
}

/// Z の読み出し(Engine の文脈の入力)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScopeView {
    pub open: bool,
    pub input_mode: InputModeState,
    /// 開閉の根拠(`Basis::label`、主張なしは `z-default`)。
    pub basis: &'static str,
}

/// 候補 Z の belief(スコープ表)。
#[derive(Debug, Clone, Default)]
pub struct ScopeBelief {
    rows: Vec<(ScopeKey, ScopeRow)>,
    focused: Option<(ScopeKey, WindowFacts)>,
    settle_ms: u64,
}

impl ScopeBelief {
    /// `settle_ms` は明示操作の後に観測を受け付けない柵(本番の `KEY_EFFECT_SETTLE_MS` を渡す)。
    #[must_use]
    pub const fn new(settle_ms: u64) -> Self {
        Self {
            rows: Vec::new(),
            focused: None,
            settle_ms,
        }
    }

    fn row(&self, scope: ScopeKey) -> ScopeRow {
        self.rows
            .iter()
            .find(|(k, _)| *k == scope)
            .map(|(_, r)| *r)
            .unwrap_or_default()
    }

    fn row_mut(&mut self, scope: ScopeKey) -> &mut ScopeRow {
        if let Some(i) = self.rows.iter().position(|(k, _)| *k == scope) {
            &mut self.rows[i].1
        } else {
            self.rows.push((scope, ScopeRow::default()));
            &mut self.rows.last_mut().expect("pushed").1
        }
    }

    /// 事象で状態を更新する(唯一の書き込み口)。
    pub fn evolve(&mut self, event: ScopeEvent) {
        match event {
            ScopeEvent::Focus {
                scope,
                facts,
                at_ms: _,
            } => {
                let prev = self.focused.map(|(k, _)| k);
                self.focused = Some((scope, facts));
                if facts.inherit_open_on_entry {
                    if let Some(prev) = prev.filter(|p| *p != scope) {
                        if let Some(src) = self.row(prev).open.filter(|c| c.value) {
                            let row = self.row_mut(scope);
                            let newer = row.open.is_none_or(|c| c.at_ms < src.at_ms);
                            if newer {
                                row.open = Some(Claim {
                                    value: true,
                                    basis: Basis::Inherited,
                                    at_ms: src.at_ms,
                                    fence_ms: src.fence_ms,
                                });
                            }
                        }
                    }
                }
            }
            ScopeEvent::Explicit { open, mode, at_ms } => {
                let Some((scope, _)) = self.focused else {
                    return;
                };
                let fence_ms = at_ms.saturating_add(self.settle_ms);
                let row = self.row_mut(scope);
                if let Some(v) = open {
                    row.open = Some(Claim {
                        value: v,
                        basis: Basis::Explicit,
                        at_ms,
                        fence_ms,
                    });
                }
                if let Some(m) = mode {
                    row.mode = Some(Claim {
                        value: m,
                        basis: Basis::Explicit,
                        at_ms,
                        fence_ms,
                    });
                }
            }
            ScopeEvent::Observed {
                open,
                mode,
                source,
                confidence,
                imm_family,
                read_started_ms,
            } => {
                let Some((scope, facts)) = self.focused else {
                    return;
                };
                if imm_family && !facts.imm_trusted {
                    return;
                }
                let basis = Basis::Observed(source, confidence);
                let row = self.row_mut(scope);
                if let Some(v) = open {
                    if accepts(row.open.as_ref(), confidence, read_started_ms) {
                        row.open = Some(Claim {
                            value: v,
                            basis,
                            at_ms: read_started_ms,
                            fence_ms: read_started_ms,
                        });
                    }
                }
                if let Some(m) = mode {
                    if accepts(row.mode.as_ref(), confidence, read_started_ms) {
                        row.mode = Some(Claim {
                            value: m,
                            basis,
                            at_ms: read_started_ms,
                            fence_ms: read_started_ms,
                        });
                    }
                }
            }
        }
    }

    /// フォーカス中のスコープの belief。主張なしは ON・かな(O2)。開で入力モードの主張が無ければかな(§4.6)。
    #[must_use]
    pub fn view(&self) -> ScopeView {
        let row = self.focused.map(|(k, _)| self.row(k)).unwrap_or_default();
        let (open, basis) = match row.open {
            Some(c) => (c.value, c.basis.label()),
            None => (true, "z-default"),
        };
        let input_mode = match row.mode.map(|c| (c.value, c.basis)) {
            Some((ModeClass::Eisu, _)) => InputModeState::ObservedEisu,
            Some((ModeClass::KanaInputMethod, _)) => InputModeState::ObservedKana,
            Some((ModeClass::Kana, Basis::Observed(..))) => InputModeState::ObservedRomaji,
            Some((ModeClass::Kana, _)) | None => InputModeState::AssumedRomaji {
                reason: AssumedReason::KeyEffectPrediction,
            },
        };
        ScopeView {
            open,
            input_mode,
            basis,
        }
    }
}

/// 観測 1 つが今の主張を覆すか(ADR-282 §4.4 の表)。
fn accepts<T>(
    current: Option<&Claim<T>>,
    confidence: ObservationConfidence,
    read_started_ms: u64,
) -> bool {
    let Some(c) = current else {
        return true;
    };
    match confidence {
        // Low は主張なし・既定だけを覆す(既定は主張なしで表す)。明示操作・観測の主張は覆さない。
        ObservationConfidence::Low => false,
        ObservationConfidence::Medium | ObservationConfidence::High => {
            if read_started_ms < c.fence_ms {
                return false;
            }
            match c.basis {
                Basis::Explicit | Basis::Inherited => true,
                // 新しいほうが勝つ。同時刻なら信頼度の高いほう(古い High は新しい Medium に勝たない、ADR-233 A')。
                Basis::Observed(_, cur) => {
                    read_started_ms > c.at_ms || (read_started_ms == c.at_ms && confidence >= cur)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const READABLE: WindowFacts = WindowFacts {
        imm_trusted: true,
        inherit_open_on_entry: false,
    };
    const UNREADABLE: WindowFacts = WindowFacts {
        imm_trusted: false,
        inherit_open_on_entry: false,
    };

    fn focus(z: &mut ScopeBelief, scope: ScopeKey, facts: WindowFacts, at_ms: u64) {
        z.evolve(ScopeEvent::Focus {
            scope,
            facts,
            at_ms,
        });
    }

    fn explicit(z: &mut ScopeBelief, open: bool, at_ms: u64) {
        z.evolve(ScopeEvent::Explicit {
            open: Some(open),
            mode: None,
            at_ms,
        });
    }

    fn poll(z: &mut ScopeBelief, open: bool, at_ms: u64) {
        z.evolve(ScopeEvent::Observed {
            open: Some(open),
            mode: None,
            source: ObservationSource::ObserverPoll,
            confidence: ObservationConfidence::Medium,
            imm_family: true,
            read_started_ms: at_ms,
        });
    }

    /// O3・O4: 窓 A の明示 OFF は、B へ移って戻っても A に残る(寿命で後退しない)。B は独立で既定の ON。
    #[test]
    fn explicit_op_latches_per_scope_across_window_switches() {
        let mut z = ScopeBelief::new(170);
        focus(&mut z, 1, READABLE, 0);
        explicit(&mut z, false, 10);
        focus(&mut z, 2, UNREADABLE, 20);
        assert!(z.view().open, "B は主張なしで ON(O2)");
        explicit(&mut z, true, 30);
        focus(&mut z, 1, READABLE, 3_600_000);
        assert!(!z.view().open, "A の OFF は時間が経っても残る(I6)");
        assert_eq!(z.view().basis, "z-explicit");
    }

    /// 読めない窓の IMM 系の観測は捨てる(BUG-223(a)・BUG-219)。読める窓では柵の後の Medium 1 つで覆る。
    #[test]
    fn imm_observation_is_ignored_in_unreadable_windows_and_fenced_in_readable_ones() {
        let mut z = ScopeBelief::new(170);
        focus(&mut z, 2, UNREADABLE, 0);
        explicit(&mut z, true, 10);
        poll(&mut z, false, 500);
        assert!(z.view().open, "読めない窓の IMM は反証不可");
        focus(&mut z, 1, READABLE, 600);
        explicit(&mut z, true, 700);
        poll(&mut z, false, 760);
        assert!(z.view().open, "柵(700 + 170)の内側の観測は覆さない");
        poll(&mut z, false, 900);
        assert!(!z.view().open, "柵の後の Medium 1 つで覆る");
    }

    /// O5: 入場時に開を引き継ぐ窓は、直前のスコープの開が新しければ開になる。閉は引き継がない。
    #[test]
    fn inherit_open_on_entry_takes_only_a_newer_open() {
        let wt = WindowFacts {
            imm_trusted: false,
            inherit_open_on_entry: true,
        };
        let mut z = ScopeBelief::new(170);
        focus(&mut z, 3, wt, 0);
        explicit(&mut z, false, 5);
        focus(&mut z, 1, READABLE, 10);
        explicit(&mut z, true, 20);
        focus(&mut z, 3, wt, 30);
        assert!(
            z.view().open,
            "EDIT の開(20)が WT の閉(5)より新しいので引き継ぐ"
        );
        assert_eq!(z.view().basis, "z-inherited");
        explicit(&mut z, false, 40);
        focus(&mut z, 1, READABLE, 50);
        explicit(&mut z, false, 60);
        focus(&mut z, 3, wt, 70);
        assert!(!z.view().open, "閉は引き継がない(WT の自分の閉のまま)");
    }

    /// 古い High は新しい Medium に勝たない(ADR-233 A')。Low は主張を覆さない。
    #[test]
    fn newer_medium_beats_older_high_and_low_never_overrides_a_claim() {
        let mut z = ScopeBelief::new(170);
        focus(&mut z, 1, READABLE, 0);
        z.evolve(ScopeEvent::Observed {
            open: Some(false),
            mode: None,
            source: ObservationSource::ImmCrossProbe,
            confidence: ObservationConfidence::High,
            imm_family: true,
            read_started_ms: 100,
        });
        poll(&mut z, true, 200);
        assert!(z.view().open);
        z.evolve(ScopeEvent::Observed {
            open: Some(false),
            mode: None,
            source: ObservationSource::FocusProbe,
            confidence: ObservationConfidence::Low,
            imm_family: true,
            read_started_ms: 300,
        });
        assert!(z.view().open, "Low は観測の主張を覆さない");
    }
}
