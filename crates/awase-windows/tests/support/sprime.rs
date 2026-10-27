//! ADR-282 候補 S′ の写し(V0 の比較専用。本番のコードは変えない)。
//!
//! S′ は「今の構造(グローバルな belief 1 つ + 観測の導出 `derive_any`)」に、所有者の決定 O3・O4・O6・O7 を満たすための
//! 部品を足したもの(ADR-282 §0.3 の 4):
//! - (a) 明示意図の記録の分離: belief 用の記録は寿命なし(授権用の `IntentStore` の TTL は belief に効かせない)。
//! - (b) フォールバックの後退の停止: 観測の鮮度(3 秒)が切れても、最後に受け入れた値を保つ。
//! - (c) 向きつきの共有の表: 窓に入るとき、開を引き継ぐと測れた向き(O5)だけ直前の窓の開を持ち越し、それ以外(O6)は
//!   その窓の前回の値(窓ごとの記録)か、無ければ既定(O2 の ON、または U2 の案の閉)にする。
//! - (d) L1′: 表のセルで決まるモードキーは押した後の状態で記録する(呼び出し元が `ScopeEvent::Explicit` に入れる)。
//! - (e) Step 4c の入力の差し替え: V0 のモデルは授権に入力しないので、ここでは写さない(本番で配線する段階で測る)。
//!
//! Z(`scope_belief.rs`)との違いとして残るのは、今の構造の観測の受け入れ方をそのまま使う点:
//! - 信頼表が無い(読めない窓でも IMM 系・conv の推測の観測を受け入れる)。
//! - 観測の導出は今の `derive_any` と同じ(鮮度 3 秒の間は High 単独が勝つ。無ければ Medium 以上の無競合多数決)。
//! (c) の窓ごとの記録を持つので、窓をまたぐ belief の持ち方は Z のスコープ表と同じ形になる(ADR-282 §14.3)。

use awase::engine::{AssumedReason, InputModeState};
use awase_windows::state::ime_event::{ObservationConfidence, ObservationSource};
use awase_windows::state::key_effect_predictor::KeyTrack;
use awase_windows::state::scope_belief::{ModeClass, ScopeEvent, ScopeView, WindowFacts};

/// 観測の鮮度(本番の `OBSERVATION_FRESH_WINDOW_MS`)。
const FRESH_MS: u64 = awase_windows::tuning::OBSERVATION_FRESH_WINDOW_MS;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Basis {
    Explicit,
    Observed(ObservationConfidence),
    Inherited,
    MeasuredPrior,
}

impl Basis {
    const fn label(self) -> &'static str {
        match self {
            Self::Explicit => "s-explicit",
            Self::Observed(ObservationConfidence::High) => "s-derive-high",
            Self::Observed(ObservationConfidence::Medium) => "s-derive-medium",
            Self::Observed(ObservationConfidence::Low) => "s-observed-low",
            Self::Inherited => "s-inherited",
            Self::MeasuredPrior => "s-measured-prior",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Claim<T> {
    value: T,
    basis: Basis,
    at_ms: u64,
    fence_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Snapshot {
    open: Option<Claim<bool>>,
    mode: Option<Claim<ModeClass>>,
    /// 追跡(窓ごとの記録に一緒に持つ)。
    track: KeyTrack,
}

/// 今の観測プールの写し(ソースごとに最新 1 件、窓切替で全消し)。
#[derive(Debug, Clone, Copy)]
struct Obs {
    source: ObservationSource,
    confidence: ObservationConfidence,
    open: bool,
    at_ms: u64,
}

/// 候補 S′。
#[derive(Debug, Clone)]
pub struct GlobalLatch {
    current: Snapshot,
    /// (c) 窓ごとの記録(窓から出たときの値)。
    by_scope: Vec<(u64, Snapshot)>,
    focused: Option<(u64, WindowFacts)>,
    pool: Vec<Obs>,
    settle_ms: u64,
    new_thread_prior: bool,
}

impl GlobalLatch {
    #[must_use]
    pub const fn new(settle_ms: u64, new_thread_prior: bool) -> Self {
        Self {
            current: Snapshot {
                open: None,
                mode: None,
                track: KeyTrack {
                    conv: None,
                    stage: awase_windows::state::key_effect_predictor::Stage::None,
                },
            },
            by_scope: Vec::new(),
            focused: None,
            pool: Vec::new(),
            settle_ms,
            new_thread_prior,
        }
    }

    fn saved(&self, scope: u64) -> Option<Snapshot> {
        self.by_scope
            .iter()
            .find(|(k, _)| *k == scope)
            .map(|(_, s)| *s)
    }

    fn save(&mut self, scope: u64, snap: Snapshot) {
        if let Some(i) = self.by_scope.iter().position(|(k, _)| *k == scope) {
            self.by_scope[i].1 = snap;
        } else {
            self.by_scope.push((scope, snap));
        }
    }

    pub fn evolve(&mut self, e: ScopeEvent) {
        match e {
            ScopeEvent::Focus {
                scope,
                facts,
                at_ms,
            } => {
                let prev = self.focused.map(|(k, _)| k);
                self.focused = Some((scope, facts));
                if prev == Some(scope) {
                    return;
                }
                let outgoing = self.current;
                if let Some(p) = prev {
                    self.save(p, outgoing);
                }
                // 今の構造と同じく、窓切替で観測プールは全部捨てる(ADR-106)。
                self.pool.clear();
                let mut next = self.saved(scope).unwrap_or_default();
                // (c) 開を引き継ぐと測れた向き(O5)だけ、直前の窓の開を持ち越す(開だけ。入力モードは持ち越さない)。
                if facts.inherit_open_on_entry {
                    if let Some(src) = outgoing.open.filter(|c| c.value) {
                        if next.open.is_none_or(|c| c.at_ms < src.at_ms) {
                            next.open = Some(Claim {
                                value: true,
                                basis: Basis::Inherited,
                                at_ms: src.at_ms,
                                fence_ms: src.fence_ms,
                            });
                        }
                    }
                }
                if self.new_thread_prior && facts.new_thread && next.open.is_none() {
                    next.open = Some(Claim {
                        value: false,
                        basis: Basis::MeasuredPrior,
                        at_ms,
                        fence_ms: at_ms,
                    });
                }
                self.current = next;
            }
            ScopeEvent::Explicit {
                open,
                mode,
                track,
                at_ms,
            } => {
                let fence_ms = at_ms + self.settle_ms;
                if let Some(t) = track {
                    self.current.track = t;
                }
                if let Some(v) = open {
                    self.current.open = Some(Claim {
                        value: v,
                        basis: Basis::Explicit,
                        at_ms,
                        fence_ms,
                    });
                }
                if let Some(m) = mode {
                    self.current.mode = Some(Claim {
                        value: m,
                        basis: Basis::Explicit,
                        at_ms,
                        fence_ms,
                    });
                }
            }
            ScopeEvent::Track { track } => self.current.track = track,
            // 字形(ADR-283)は S′ の比較の対象外。
            ScopeEvent::Glyph { .. } | ScopeEvent::StateChange { .. } => {}
            ScopeEvent::Observed {
                open,
                mode,
                source,
                confidence,
                imm_family: _,
                read_started_ms,
            } => {
                // 信頼表は無い(今の構造のまま)。開閉は今の `derive_any` と同じ導出で決める。
                if let Some(v) = open {
                    self.pool.retain(|o| o.source != source);
                    self.pool.push(Obs {
                        source,
                        confidence,
                        open: v,
                        at_ms: read_started_ms,
                    });
                    self.accept_derived(read_started_ms);
                }
                if let Some(m) = mode {
                    if confidence >= ObservationConfidence::Medium
                        && self
                            .current
                            .mode
                            .is_none_or(|c| read_started_ms >= c.fence_ms)
                    {
                        self.current.mode = Some(Claim {
                            value: m,
                            basis: Basis::Observed(confidence),
                            at_ms: read_started_ms,
                            fence_ms: read_started_ms,
                        });
                        self.current.track.conv = None;
                    }
                }
            }
        }
    }

    /// 今の `derive_any` の写し: 新鮮な High が 1 つあればそれ、無ければ Medium 以上の無競合多数決(1 ソースでも可)。
    /// 導いた値を、明示操作の柵の後なら主張にする(b: 鮮度が切れても主張は消さない)。Low だけのときは主張なし・事前だけを覆す。
    fn accept_derived(&mut self, now_ms: u64) {
        let fresh: Vec<Obs> = self
            .pool
            .iter()
            .copied()
            .filter(|o| now_ms.saturating_sub(o.at_ms) <= FRESH_MS)
            .collect();
        let high = fresh
            .iter()
            .filter(|o| o.confidence == ObservationConfidence::High)
            .max_by_key(|o| o.at_ms);
        let derived = if let Some(h) = high {
            Some((h.open, ObservationConfidence::High, h.at_ms))
        } else {
            let med: Vec<&Obs> = fresh
                .iter()
                .filter(|o| o.confidence >= ObservationConfidence::Medium)
                .collect();
            let t = med.iter().filter(|o| o.open).count();
            let f = med.len() - t;
            match (t, f) {
                (t, 0) if t > 0 => Some((
                    true,
                    ObservationConfidence::Medium,
                    med.iter().map(|o| o.at_ms).max().unwrap_or(now_ms),
                )),
                (0, f) if f > 0 => Some((
                    false,
                    ObservationConfidence::Medium,
                    med.iter().map(|o| o.at_ms).max().unwrap_or(now_ms),
                )),
                _ => None,
            }
        };
        let fence_ok = |c: &Claim<bool>, at: u64| at >= c.fence_ms;
        if let Some((v, conf, at)) = derived {
            if self.current.open.is_none_or(|c| fence_ok(&c, at)) {
                self.current.open = Some(Claim {
                    value: v,
                    basis: Basis::Observed(conf),
                    at_ms: at,
                    fence_ms: at,
                });
            }
            return;
        }
        // Low だけ: 主張なし・実測の事前だけを覆す。
        if let Some(low) = fresh
            .iter()
            .filter(|o| o.confidence == ObservationConfidence::Low)
            .max_by_key(|o| o.at_ms)
        {
            let replace = self
                .current
                .open
                .is_none_or(|c| c.basis == Basis::MeasuredPrior);
            if replace {
                self.current.open = Some(Claim {
                    value: low.open,
                    basis: Basis::Observed(ObservationConfidence::Low),
                    at_ms: low.at_ms,
                    fence_ms: low.at_ms,
                });
            }
        }
    }

    #[must_use]
    pub fn view(&self) -> ScopeView {
        let (open, basis) = self
            .current
            .open
            .map_or((true, "s-default"), |c| (c.value, c.basis.label()));
        let input_mode = match self.current.mode.map(|c| (c.value, c.basis)) {
            Some((ModeClass::Eisu, _)) => InputModeState::ObservedEisu,
            Some((ModeClass::KanaInputMethod, _)) => InputModeState::ObservedKana,
            Some((ModeClass::Kana, Basis::Observed(_))) => InputModeState::ObservedRomaji,
            Some((ModeClass::Kana, _)) | None => InputModeState::AssumedRomaji {
                reason: AssumedReason::KeyEffectPrediction,
            },
        };
        ScopeView {
            open,
            input_mode,
            basis,
            track: self.current.track,
        }
    }
}
