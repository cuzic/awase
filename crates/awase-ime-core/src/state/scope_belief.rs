//! ADR-282 候補 Z の純粋な核: IME の保持単位(スコープ)ごとに「最後に受け入れた主張」をラッチする belief。
//!
//! 所有者の決定 O8(2026-10-26)で本番へ進める。V1 では殻が shadow として並走させ、旧の belief との食い違いを journal に
//! 残すだけで、本番の belief(`ImeModel`・`ImeStateHub`)・Engine・IME への書き込みは変えない(ADR-282 §9)。
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
//! - 変換モード・変換中の段階の追跡(`KeyTrack`)もスコープの行に持つ(ADR-282 §4.5、191 spec §3.4)。窓切替で捨てない。
//!
//! 時刻は呼び出し元が渡す単調な ms(壁時計を読まない)。

use super::ime_event::{ImeEvent, ObservationConfidence, ObservationSource};
use super::key_effect_predictor::{KeyTrack, Prediction, Stage, TableKey};
use super::probe_admission::FocusEpoch;
use awase::engine::{AssumedReason, InputModeState};

/// スコープの鍵。本番は [`scope_key`] で (pid, フォーカス先のスレッド) から作る(ADR-282 §4.1)。擬似窓はスレッドの番号。
pub type ScopeKey = u64;

/// スコープの鍵を作る(ADR-282 §4.1)。スレッドが取れたら (pid, tid)、取れなければ窓のハンドル(最上位ビットで区別する)。
#[must_use]
pub fn scope_key(pid: u32, tid: Option<u32>, hwnd: usize) -> ScopeKey {
    tid.map_or((1 << 63) | (hwnd as u64 & !(1 << 63)), |tid| {
        (u64::from(pid) << 32) | u64::from(tid)
    })
}

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
    /// 実測の事前(新しいスレッドは閉で始まる、ADR-282 U2 の案)。主張なしと同じく、観測 1 つで覆る。
    MeasuredPrior,
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
            Self::MeasuredPrior => "z-measured-prior",
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
    /// 変換モード・変換中の段階の追跡(予測の入力)。
    track: KeyTrack,
}

/// 窓の種類のうち、Z の規則に効くもの。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowFacts {
    /// IMM 系の観測(`ObserverPoll`・`ImmCrossProbe`・`FocusProbe`)を反証に使える窓か(信頼表、ADR-282 §4.4)。
    pub imm_trusted: bool,
    /// 入場時に直前のスコープの開を引き継ぐ窓の種類か(O5)。
    pub inherit_open_on_entry: bool,
    /// このスコープに初めて入場し、awase の起動後に作られたスレッドと分かっているか(実測の事前「新しいスレッドは閉で始まる」の
    /// 条件。ADR-282 U2。`ScopeBelief::with_new_thread_prior` のときだけ使う)。
    pub new_thread: bool,
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
    /// 明示操作。値は押した後の状態(変化が無くても記録する、L1′)。`track` は押した後の追跡(`None` は変えない)。
    Explicit {
        open: Option<bool>,
        mode: Option<ModeClass>,
        track: Option<KeyTrack>,
        at_ms: u64,
    },
    /// 開閉・入力モードの主張にならない打鍵(表に無いキー)の、押した後の追跡。
    Track { track: KeyTrack },
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
    /// フォーカス中のスコープの追跡(予測の入力)。
    pub track: KeyTrack,
}

/// 候補 Z の belief(スコープ表)。
#[derive(Debug, Clone, Default)]
pub struct ScopeBelief {
    rows: Vec<(ScopeKey, ScopeRow)>,
    focused: Option<(ScopeKey, WindowFacts)>,
    settle_ms: u64,
    new_thread_prior: bool,
}

impl ScopeBelief {
    /// `settle_ms` は明示操作の後に観測を受け付けない柵(本番の `KEY_EFFECT_SETTLE_MS` を渡す)。
    #[must_use]
    pub const fn new(settle_ms: u64) -> Self {
        Self {
            rows: Vec::new(),
            focused: None,
            settle_ms,
            new_thread_prior: false,
        }
    }

    /// ADR-282 U2 の案: 新しいスレッドの窓(`WindowFacts::new_thread`)に初めて入場したとき、主張なしなら閉の実測の事前を置く。
    /// U2 は所有者の判断待ちで、V0 の比較のための変種。
    #[must_use]
    pub const fn with_new_thread_prior(settle_ms: u64) -> Self {
        Self {
            rows: Vec::new(),
            focused: None,
            settle_ms,
            new_thread_prior: true,
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
        let found = self.rows.iter().position(|(k, _)| *k == scope);
        let i = found.unwrap_or_else(|| {
            self.rows.push((scope, ScopeRow::default()));
            self.rows.len() - 1
        });
        &mut self.rows[i].1
    }

    /// 事象で状態を更新する(唯一の書き込み口)。
    pub fn evolve(&mut self, event: ScopeEvent) {
        match event {
            ScopeEvent::Focus {
                scope,
                facts,
                at_ms,
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
                if self.new_thread_prior && facts.new_thread {
                    let row = self.row_mut(scope);
                    if row.open.is_none() {
                        row.open = Some(Claim {
                            value: false,
                            basis: Basis::MeasuredPrior,
                            at_ms,
                            fence_ms: at_ms,
                        });
                    }
                }
            }
            ScopeEvent::Explicit {
                open,
                mode,
                track,
                at_ms,
            } => {
                let Some((scope, _)) = self.focused else {
                    return;
                };
                let fence_ms = at_ms.saturating_add(self.settle_ms);
                let row = self.row_mut(scope);
                if let Some(t) = track {
                    row.track = t;
                }
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
            ScopeEvent::Track { track } => {
                if let Some((scope, _)) = self.focused {
                    self.row_mut(scope).track = track;
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
                        // 観測が来たので、変換モードの追跡は観測へ戻す(`ImeModel` の `InputModeObserved` と同じ)。
                        row.track.conv = None;
                    }
                }
            }
        }
    }

    /// フォーカス中のスコープの belief。主張なしは ON・かな(O2)。開で入力モードの主張が無ければかな(§4.6)。
    #[must_use]
    pub fn view(&self) -> ScopeView {
        let row = self.focused.map(|(k, _)| self.row(k)).unwrap_or_default();
        let (open, basis) = row
            .open
            .map_or((true, "z-default"), |c| (c.value, c.basis.label()));
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
            track: row.track,
        }
    }
}

/// 入力モードを Z の軸の値へ写す(O1)。`Unknown` は主張にしない。
#[must_use]
pub const fn mode_class_of(mode: InputModeState) -> Option<ModeClass> {
    match mode {
        InputModeState::ObservedEisu => Some(ModeClass::Eisu),
        InputModeState::ObservedKana => Some(ModeClass::KanaInputMethod),
        InputModeState::ObservedRomaji | InputModeState::AssumedRomaji { .. } => {
            Some(ModeClass::Kana)
        }
        InputModeState::Unknown => None,
    }
}

/// IMM の読み・conv のビットからの推測の源か(読めない窓では反証に使わない、ADR-282 §4.4 の信頼表)。
/// TSF・共有メモリ・字形は窓の種類に関わらず使う。
#[must_use]
pub const fn is_imm_family(source: ObservationSource) -> bool {
    !matches!(
        source,
        ObservationSource::Tsf | ObservationSource::MozcShm | ObservationSource::TaskbarGlyph
    )
}

/// 打鍵の予測を Z の事象へ写す。表のキー(`table_key`)は押した後の状態で明示操作として記録し(L1′。開閉の予測が無ければ
/// 打鍵前の Z の開)、それ以外は追跡だけを進める。予測の入力は Z 自身の読み出し(`view`)から作ること。
#[must_use]
pub fn scope_event_of_prediction(
    table_key: bool,
    prediction: &Prediction,
    view: &ScopeView,
    at_ms: u64,
) -> ScopeEvent {
    if !table_key {
        return ScopeEvent::Track {
            track: prediction.track,
        };
    }
    ScopeEvent::Explicit {
        open: Some(prediction.effect.open.unwrap_or(view.open)),
        mode: prediction.effect.mode.and_then(mode_class_of),
        track: Some(prediction.track),
        at_ms,
    }
}

/// 表のキーか(明示操作として記録するか)。
#[must_use]
pub const fn is_table_key(vk: u16) -> bool {
    TableKey::from_vk(vk).is_some()
}

/// 本番の `ImeEvent` を Z の事象へ写す(V1 の shadow。ADR-282 §4.3・§4.4)。
///
/// - 利用者の明示 ON/OFF(`UserImeSetIntent`、3 つの源とも)は明示操作。変換中の段階は捨てる(`ImeModel` と同じ)。
/// - 開閉の観測は、今のフォーカスの世代(`epoch`)のものだけ。キャッシュの復元・既定の推測は観測ではないので写さない(I7)。
/// - 入力モードの観測は、`Unknown` 以外。
/// - 打鍵の予測(`KeyEffectPredicted`)は写さない(Z は自分の読み出しを入力にした予測を別に受ける。
///   [`scope_event_of_prediction`])。窓切替も写さない(殻がスコープの鍵と窓の種類を添えて別に渡す)。
/// - それ以外(パニックリセット・キャッシュの復元・送信の結果・drift など)は Z の belief に入れない(D8・I7)。
///
/// 観測の時刻は事象を受けた時刻で代える(読み取りの開始時刻を `ImeEvent` が持たないため。V1 の近似)。
#[must_use]
pub fn scope_event_of(
    event: &ImeEvent,
    view: &ScopeView,
    epoch: FocusEpoch,
    at_ms: u64,
) -> Option<ScopeEvent> {
    match event {
        ImeEvent::UserImeSetIntent { target, .. } => Some(ScopeEvent::Explicit {
            open: Some(*target),
            mode: None,
            track: Some(KeyTrack {
                stage: Stage::None,
                ..view.track
            }),
            at_ms,
        }),
        ImeEvent::ObserverReported(o) => {
            let source = o.source();
            if matches!(
                source,
                ObservationSource::HwndCache | ObservationSource::HeuristicDefault
            ) || o.focus_epoch() != epoch
            {
                return None;
            }
            Some(ScopeEvent::Observed {
                open: Some(o.open()),
                mode: None,
                source,
                confidence: o.confidence(),
                imm_family: is_imm_family(source),
                read_started_ms: at_ms,
            })
        }
        ImeEvent::InputModeObserved {
            mode,
            source,
            confidence,
        } => mode_class_of(*mode).map(|m| ScopeEvent::Observed {
            open: None,
            mode: Some(m),
            source: *source,
            confidence: *confidence,
            imm_family: is_imm_family(*source),
            read_started_ms: at_ms,
        }),
        _ => None,
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
        // Low は主張なし・既定・実測の事前だけを覆す(既定は主張なしで表す)。明示操作・観測の主張は覆さない。
        ObservationConfidence::Low => c.basis == Basis::MeasuredPrior,
        ObservationConfidence::Medium | ObservationConfidence::High => {
            if read_started_ms < c.fence_ms {
                return false;
            }
            match c.basis {
                Basis::Explicit | Basis::Inherited | Basis::MeasuredPrior => true,
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
        new_thread: false,
    };
    const UNREADABLE: WindowFacts = WindowFacts {
        imm_trusted: false,
        inherit_open_on_entry: false,
        new_thread: false,
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
            track: None,
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
            new_thread: false,
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

    /// U2 の案: 新しいスレッドに初めて入場したら閉の実測の事前を置く。観測 1 つ(Low でも)で覆り、明示操作でも覆る。
    /// 開を引き継ぐ窓では、引き継いだ開が先に入る(O5 の CI 実測は新しいプロセスの WT で開を引き継いだ)。
    #[test]
    fn new_thread_prior_is_closed_and_yields_to_any_evidence() {
        let fresh = WindowFacts {
            new_thread: true,
            ..UNREADABLE
        };
        let mut z = ScopeBelief::with_new_thread_prior(170);
        focus(&mut z, 1, READABLE, 0);
        focus(&mut z, 2, fresh, 10);
        assert!(!z.view().open);
        assert_eq!(z.view().basis, "z-measured-prior");
        explicit(&mut z, true, 20);
        assert!(z.view().open);
        let fresh_readable = WindowFacts {
            new_thread: true,
            ..READABLE
        };
        focus(&mut z, 3, fresh_readable, 30);
        z.evolve(ScopeEvent::Observed {
            open: Some(true),
            mode: None,
            source: ObservationSource::FocusProbe,
            confidence: ObservationConfidence::Low,
            imm_family: true,
            read_started_ms: 40,
        });
        assert!(z.view().open, "Low の観測でも実測の事前は覆る");
        let fresh_wt = WindowFacts {
            new_thread: true,
            inherit_open_on_entry: true,
            ..UNREADABLE
        };
        focus(&mut z, 4, fresh_wt, 50);
        assert!(
            z.view().open,
            "直前のスコープの開を引き継ぐ窓では、引き継ぎが先"
        );
        assert_eq!(z.view().basis, "z-inherited");
    }

    /// 追跡はスコープの行に持ち、窓切替で捨てない(§4.5)。入力モードの観測で変換モードの追跡は観測へ戻る。
    #[test]
    fn key_track_is_kept_per_scope_and_cleared_by_mode_observation() {
        use crate::state::key_effect_predictor::Conv;
        let typing = KeyTrack {
            conv: Some(Conv::C10),
            stage: Stage::Typing,
        };
        let mut z = ScopeBelief::new(170);
        focus(&mut z, 1, READABLE, 0);
        z.evolve(ScopeEvent::Track { track: typing });
        focus(&mut z, 2, READABLE, 10);
        assert_eq!(z.view().track, KeyTrack::default(), "B は自分の追跡");
        focus(&mut z, 1, READABLE, 20);
        assert_eq!(z.view().track, typing, "A に戻れば A の追跡");
        z.evolve(ScopeEvent::Observed {
            open: None,
            mode: Some(ModeClass::Kana),
            source: ObservationSource::ObserverPoll,
            confidence: ObservationConfidence::Medium,
            imm_family: true,
            read_started_ms: 30,
        });
        assert_eq!(z.view().track.conv, None);
        assert_eq!(z.view().track.stage, Stage::Typing, "段階は残す");
    }

    /// 表のキーは押した後の状態で明示操作(開閉の予測が無ければ打鍵前の Z の開)、それ以外は追跡だけ。
    #[test]
    fn prediction_maps_table_keys_to_explicit_and_others_to_track() {
        use crate::state::key_effect_predictor::PredictedEffect;
        let mut z = ScopeBelief::new(170);
        focus(&mut z, 1, READABLE, 0);
        explicit(&mut z, false, 5);
        let p = Prediction {
            effect: PredictedEffect {
                open: None,
                mode: Some(InputModeState::ObservedEisu),
            },
            track: KeyTrack::default(),
        };
        let e = scope_event_of_prediction(true, &p, &z.view(), 10);
        assert_eq!(
            e,
            ScopeEvent::Explicit {
                open: Some(false),
                mode: Some(ModeClass::Eisu),
                track: Some(KeyTrack::default()),
                at_ms: 10,
            }
        );
        assert_eq!(
            scope_event_of_prediction(false, &p, &z.view(), 10),
            ScopeEvent::Track {
                track: KeyTrack::default()
            }
        );
    }

    /// `ImeEvent` の写像: 明示 ON/OFF は明示操作、別の世代・キャッシュ・既定の観測は写さない、予測・パニックは写さない。
    #[test]
    fn ime_event_mapping_keeps_only_user_ops_and_current_observations() {
        use crate::state::evidence::AnyObservation;
        use crate::state::ime_event::{HwndId, UserIntentSource};
        let view = ScopeBelief::new(170).view();
        let epoch = FocusEpoch::default();
        let user = ImeEvent::UserImeSetIntent {
            target: false,
            source: UserIntentSource::PhysicalImeKey,
        };
        assert!(matches!(
            scope_event_of(&user, &view, epoch, 1),
            Some(ScopeEvent::Explicit {
                open: Some(false),
                ..
            })
        ));
        let obs = |source| {
            ImeEvent::ObserverReported(AnyObservation::restored_from_journal(
                true,
                source,
                HwndId(1),
                ObservationConfidence::Medium,
                epoch,
            ))
        };
        assert!(matches!(
            scope_event_of(&obs(ObservationSource::ObserverPoll), &view, epoch, 2),
            Some(ScopeEvent::Observed {
                imm_family: true,
                ..
            })
        ));
        assert!(matches!(
            scope_event_of(&obs(ObservationSource::MozcShm), &view, epoch, 2),
            Some(ScopeEvent::Observed {
                imm_family: false,
                ..
            })
        ));
        assert_eq!(
            scope_event_of(&obs(ObservationSource::HwndCache), &view, epoch, 2),
            None
        );
        assert_eq!(
            scope_event_of(&obs(ObservationSource::HeuristicDefault), &view, epoch, 2),
            None
        );
        assert_eq!(
            scope_event_of(&ImeEvent::PanicReset { target: true }, &view, epoch, 3),
            None
        );
    }

    #[test]
    fn scope_key_separates_thread_keys_from_hwnd_fallback() {
        assert_eq!(scope_key(2, Some(3), 99), (2 << 32) | 3);
        assert_ne!(scope_key(0, None, 3), scope_key(0, Some(3), 0));
    }
}
