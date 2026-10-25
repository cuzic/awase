//! ADR-280 D2 の続き: 打鍵列のレベルで lang_detect を網羅するテスト（有限モデル検査と参照モデルとの差分）。
//!
//! 設計:
//! - **字母**（小さな合成モデル）: 文字キー 4 種（`A`=か・`B`=た は仮名に読め、`A`→`B` は尤度が積まれて日本語側へ、`C`=さ は
//!   行頭の尤度が極端に低く英数側へ振れる、`N`=記号は仮名に読めず中立）の Down/Up、左右の親指の Down/Up、除外キー
//!   （IME キー等）、修飾キー自身、判定の材料でない打鍵（矢印等）、時刻の進み（期限を超えるが idle には足りない 2500ms、
//!   idle を超える 3000ms）、窓の切替、期限のタイマーの発火。
//! - **全列挙**: 字母全部で長さ ≤ 4、小さい字母で長さ 5〜6、固定 seed のランダムで長い列。`may_hold` は true/false の両方。
//!   列の長さを短い順に回すので、最初の失敗が最小の再現列になる。
//! - **不変条件**（[`Inv`]。参照モデルとは別に、命令の列だけから検査する）: 保存則（各打鍵はちょうど 1 つの運命）・
//!   命令の順序・タイマー・保持しないときの観測のみ・保持中の窓切替。
//! - **参照モデル**（[`RefHold`]。ADR-263 D1〜D3 と `lang_detect.rs` の doc の仕様から素朴に書き直したもの。実装の
//!   `EpisodeTracker`/`EpisodeDetector`/`HoldController` は使わない）と、命令の列・閉じたエピソードの記録が全ステップで一致する。
//!   仮名の尤度は `KanaModel` の doc の式（`ln P_ja(token | prev) − ln(1/N_keys)`、表に無い組は表の最小値の 1/10）から別に計算する。

use std::collections::{BTreeSet, HashMap};
use std::sync::OnceLock;

use super::{
    DetectEvent, DetectParams, EpisodeDetector, EpisodeTracker, HoldCommand, HoldController,
    JapaneseEvidence, KanaModel, KeyRole, ObservedKey, StopReason, ThumbOverlap, TrackerConfig,
    Trigger, Verdict,
};
use crate::engine::fsm_types::ThumbSide;
use crate::ngram::NgramModel;
use crate::scanmap::PhysicalPos;
use crate::yab::{YabFace, YabValue};

// ---- 合成モデル ----

type P = (u8, u8);
/// か
const KA: P = (1, 1);
/// た
const TA: P = (1, 2);
/// こ（字母には使わないが、定義されたキーの数に入れる）
const KO: P = (1, 3);
/// さ
const SA: P = (1, 4);
/// 記号（仮名に読めない）
const SYM: P = (0, 0);

const MAX_MS: u64 = 2500;
const IDLE_MS: u64 = 3000;
const JA: f64 = 2.0;
const EN: f64 = 5.0;
const MAX_KEYS: usize = 4;

const BIGRAMS: [(char, char, f64); 4] = [
    ('か', 'た', 0.0),
    ('た', 'こ', 0.0),
    ('こ', 'さ', 0.0),
    ('か', 'こ', -3.0),
];

fn pp(p: P) -> PhysicalPos {
    PhysicalPos::new(p.0, p.1)
}

fn kana_model() -> &'static KanaModel {
    static MODEL: OnceLock<KanaModel> = OnceLock::new();
    MODEL.get_or_init(|| {
        let csv: String = BIGRAMS
            .iter()
            .map(|(a, b, s)| format!("2,{a}{b},{s:?}\n"))
            .collect();
        let ngram = NgramModel::from_csv(&csv, 0, 0, 0).expect("csv");
        let mut face = YabFace::new();
        for (p, romaji, kana) in [
            (KA, "ka", 'か'),
            (TA, "ta", 'た'),
            (KO, "ko", 'こ'),
            (SA, "sa", 'さ'),
        ] {
            face.insert(
                pp(p),
                YabValue::Romaji {
                    romaji: romaji.to_string(),
                    kana: Some(kana),
                },
            );
        }
        face.insert(pp(SYM), YabValue::Literal("1".to_string()));
        KanaModel::new(&ngram, &face)
    })
}

fn tracker_cfg() -> TrackerConfig {
    TrackerConfig {
        params: DetectParams {
            ja_threshold: JA,
            en_threshold: EN,
            max_keys: MAX_KEYS,
        },
        thumb_overlap: ThumbOverlap::Enabled,
        idle_threshold_ms: IDLE_MS,
        max_ms: MAX_MS,
    }
}

// ---- 仮名の尤度（KanaModel の doc の式から別に計算する）----

struct RefKana {
    freq: HashMap<(char, char), f64>,
    left: HashMap<char, f64>,
    listed: HashMap<char, f64>,
    vocab_len: f64,
    floor: f64,
    total: f64,
    key_count: f64,
    kana_of: HashMap<P, char>,
}

fn ref_kana() -> &'static RefKana {
    static REF: OnceLock<RefKana> = OnceLock::new();
    REF.get_or_init(|| {
        let mut freq = HashMap::new();
        let mut left: HashMap<char, f64> = HashMap::new();
        let mut listed: HashMap<char, f64> = HashMap::new();
        let mut vocab = BTreeSet::new();
        let mut min = f64::INFINITY;
        for (a, b, score) in BIGRAMS {
            let f = 10_f64.powf(score);
            freq.insert((a, b), f);
            *left.entry(a).or_insert(0.0) += f;
            *listed.entry(a).or_insert(0.0) += 1.0;
            vocab.insert(a);
            vocab.insert(b);
            min = min.min(f);
        }
        let total = left.values().sum();
        RefKana {
            freq,
            left,
            listed,
            vocab_len: vocab.len() as f64,
            floor: min * 0.1,
            total,
            // 定義されたキー: か・た・こ・さ・記号
            key_count: 5.0,
            kana_of: HashMap::from([(KA, 'か'), (TA, 'た'), (KO, 'こ'), (SA, 'さ')]),
        }
    })
}

impl RefKana {
    /// 1 打鍵ぶんの対数尤度比の増分。仮名に読めないキーは 0（中立）。
    fn delta(&self, prev: Option<char>, p: P) -> f64 {
        let Some(&token) = self.kana_of.get(&p) else {
            return 0.0;
        };
        let background = -self.key_count.ln();
        let ja = match prev {
            Some(a) => {
                let denominator = (self.vocab_len - self.listed.get(&a).copied().unwrap_or(0.0))
                    * self.floor
                    + self.left.get(&a).copied().unwrap_or(0.0);
                let numerator = self.freq.get(&(a, token)).copied().unwrap_or(self.floor);
                (numerator / denominator).ln()
            }
            None => {
                let left = self.left.get(&token).copied().unwrap_or(0.0);
                ((left + self.floor) / (self.vocab_len * self.floor + self.total)).ln()
            }
        };
        ja - background
    }
}

// ---- 比べやすい形の命令 ----

/// 閉じたエピソードの記録（`llr` は 1e-6 に丸める）。
#[derive(Debug, Clone, PartialEq)]
struct Rec {
    trigger: Trigger,
    verdict: Verdict,
    stop: StopReason,
    presses: usize,
    idle_ms: u64,
    duration_ms: u64,
    llr_micro: i64,
}

#[allow(clippy::cast_possible_truncation)] // 検査用の丸め
fn micro(llr: f64) -> i64 {
    (llr * 1e6).round() as i64
}

#[derive(Debug, Clone, PartialEq)]
enum Cmd {
    Discard(usize),
    Write,
    Enter,
    Record(Rec),
    Replay(Vec<u32>),
    Arm,
    Disarm,
    Pass(u32),
    Consume,
}

fn rec_of(o: &super::EpisodeOutcome) -> Rec {
    Rec {
        trigger: o.trigger,
        verdict: o.verdict,
        stop: o.stop,
        presses: o.presses,
        idle_ms: o.idle_ms,
        duration_ms: o.duration_ms,
        llr_micro: micro(o.llr),
    }
}

fn normalize(cmds: &[HoldCommand<u32>]) -> Vec<Cmd> {
    cmds.iter()
        .map(|c| match c {
            HoldCommand::DiscardHeld(n) => Cmd::Discard(*n),
            HoldCommand::WriteHiragana => Cmd::Write,
            HoldCommand::EnterHalfWidthAlnum => Cmd::Enter,
            HoldCommand::Record(o) => Cmd::Record(rec_of(o)),
            HoldCommand::Replay(keys) => Cmd::Replay(keys.clone()),
            HoldCommand::ArmDeadline => Cmd::Arm,
            HoldCommand::DisarmDeadline => Cmd::Disarm,
            HoldCommand::Pass(e) => Cmd::Pass(*e),
            HoldCommand::Consume => Cmd::Consume,
        })
        .collect()
}

// ---- 参照モデル（仕様から素朴に）----

/// 参照の判定器（ADR-263 D3）。
struct RefDet {
    thumbs: [bool; 2],
    chars: BTreeSet<P>,
    presses: usize,
    llr: f64,
    prev: Option<char>,
    verdict: Verdict,
}

impl RefDet {
    fn new() -> Self {
        Self {
            thumbs: [false; 2],
            chars: BTreeSet::new(),
            presses: 0,
            llr: 0.0,
            prev: None,
            verdict: Verdict::Pending,
        }
    }

    fn char_down(&mut self, p: P) {
        if !self.chars.insert(p) {
            return; // 自動リピートは数えない
        }
        if self.thumbs.iter().any(|&t| t) {
            self.verdict = Verdict::Japanese(JapaneseEvidence::ThumbOverlap);
            return;
        }
        self.presses += 1;
        let k = ref_kana();
        self.llr += k.delta(self.prev, p);
        self.prev = k.kana_of.get(&p).copied();
        self.verdict = if self.llr >= JA {
            Verdict::Japanese(JapaneseEvidence::Likelihood)
        } else if self.llr <= -EN {
            Verdict::NotJapanese
        } else if self.presses >= MAX_KEYS {
            Verdict::Undecided
        } else {
            Verdict::Pending
        };
    }

    fn thumb_down(&mut self, side: usize) {
        if self.chars.is_empty() {
            self.thumbs[side] = true;
        } else {
            self.verdict = Verdict::Japanese(JapaneseEvidence::ThumbOverlap);
        }
    }
}

struct RefEp {
    trigger: Trigger,
    idle: u64,
    scope: u64,
    start: u64,
    det: RefDet,
}

impl RefEp {
    fn rec(&self, stop: StopReason, now: u64) -> Rec {
        Rec {
            trigger: self.trigger,
            verdict: self.det.verdict,
            stop,
            presses: self.det.presses,
            idle_ms: self.idle,
            duration_ms: now.saturating_sub(self.start),
            llr_micro: micro(self.det.llr),
        }
    }
}

struct RefStep {
    closed_prev: Option<Rec>,
    closed_now: Option<Rec>,
    in_episode: bool,
    started: bool,
}

/// 参照の追跡器（ADR-263 D1: 窓切替・長い無操作の最初の Down でエピソードを始める）。
struct RefTracker {
    last_scope: Option<u64>,
    last_key: Option<u64>,
    armed: Option<(Trigger, u64)>,
    active: Option<RefEp>,
    started_total: usize,
}

impl RefTracker {
    fn new() -> Self {
        Self {
            last_scope: None,
            last_key: None,
            armed: None,
            active: None,
            started_total: 0,
        }
    }

    fn close(&mut self, stop: StopReason, now: u64) -> Rec {
        let ep = self.active.take().expect("active");
        ep.rec(stop, now)
    }

    fn step(&mut self, key: ObservedKey, scope: u64) -> RefStep {
        let now = key.timestamp_ms;
        // 1. 前のエピソードが、窓の変化・期限で閉じる
        let mut closed_prev = None;
        if let Some((other_window, expired)) = self
            .active
            .as_ref()
            .map(|ep| (ep.scope != scope, now.saturating_sub(ep.start) >= MAX_MS))
        {
            if other_window {
                closed_prev = Some(self.close(StopReason::WindowChanged, now));
            } else if expired {
                closed_prev = Some(self.close(StopReason::Timeout, now));
            }
        }
        // 2. 窓切替・無操作を武装する（エピソードが無いときだけ。窓切替が優先）
        let idle = self.last_key.map_or(0, |l| now.saturating_sub(l));
        if self.active.is_none() {
            if self.last_scope != Some(scope) {
                self.armed = Some((Trigger::WindowSwitch, idle));
            } else if self.armed.is_none() && idle >= IDLE_MS {
                self.armed = Some((Trigger::Idle, idle));
            }
        }
        self.last_scope = Some(scope);
        self.last_key = Some(now);
        // 3. 武装後の最初の Down（修飾キー自身を除く）でエピソードを始める。材料でない打鍵は見送る
        let mut started = false;
        if key.down && self.active.is_none() && key.role != KeyRole::Modifier {
            if let Some((trigger, idle_ms)) = self.armed.take() {
                if !key.excluded && matches!(key.role, KeyRole::Char(_) | KeyRole::Thumb(_)) {
                    self.active = Some(RefEp {
                        trigger,
                        idle: idle_ms,
                        scope,
                        start: now,
                        det: RefDet::new(),
                    });
                    self.started_total += 1;
                    started = true;
                }
            }
        }
        // 4. 判定器へ
        let in_episode = self.active.is_some();
        let mut closed_now = None;
        if in_episode {
            if key.down && key.excluded {
                closed_now = Some(self.close(StopReason::ExcludedKey, now));
            } else {
                match key.role {
                    KeyRole::Modifier => {}
                    KeyRole::Other => {
                        if key.down {
                            closed_now = Some(self.close(StopReason::NonMaterialKey, now));
                        }
                    }
                    KeyRole::Char(p) => {
                        let ep = self.active.as_mut().expect("active");
                        let q = (p.row, p.col);
                        if ep.det.verdict == Verdict::Pending {
                            if key.down {
                                ep.det.char_down(q);
                            } else {
                                ep.det.chars.remove(&q);
                            }
                        }
                        closed_now = self.close_if_decided(now);
                    }
                    KeyRole::Thumb(side) => {
                        let ep = self.active.as_mut().expect("active");
                        let s = usize::from(side == ThumbSide::Right);
                        if ep.det.verdict == Verdict::Pending {
                            if key.down {
                                ep.det.thumb_down(s);
                            } else {
                                ep.det.thumbs[s] = false;
                            }
                        }
                        closed_now = self.close_if_decided(now);
                    }
                }
            }
        }
        RefStep {
            closed_prev,
            closed_now,
            in_episode,
            started,
        }
    }

    fn close_if_decided(&mut self, now: u64) -> Option<Rec> {
        let verdict = self.active.as_ref()?.det.verdict;
        match verdict {
            Verdict::Pending => None,
            Verdict::Undecided => Some(self.close(StopReason::MaxKeys, now)),
            Verdict::Japanese(_) | Verdict::NotJapanese => {
                Some(self.close(StopReason::Decided, now))
            }
        }
    }
}

/// 参照の保持層（ADR-263 D2: エピソードの全打鍵を保持し、判定が出たら順序どおりに解放。窓が変わっていたら捨てる）。
struct RefHold {
    tr: RefTracker,
    held: Vec<u32>,
    holding: bool,
}

impl RefHold {
    fn new() -> Self {
        Self {
            tr: RefTracker::new(),
            held: Vec::new(),
            holding: false,
        }
    }

    fn assemble(
        discard: usize,
        write: Option<Cmd>,
        recs: Vec<Rec>,
        release: Vec<u32>,
        timer: Option<Cmd>,
        fate: Option<Cmd>,
    ) -> Vec<Cmd> {
        let mut out = Vec::new();
        if discard > 0 {
            out.push(Cmd::Discard(discard));
        }
        out.extend(write);
        out.extend(recs.into_iter().map(Cmd::Record));
        if !release.is_empty() {
            out.push(Cmd::Replay(release));
        }
        out.extend(timer);
        out.extend(fate);
        out
    }

    fn on_key(
        &mut self,
        key: ObservedKey,
        id: u32,
        scope: u64,
        may_hold: bool,
    ) -> (Vec<Cmd>, RefStep) {
        let st = self.tr.step(key, scope);
        let mut discard = 0;
        let mut release = Vec::new();
        let mut recs = Vec::new();
        let mut write = None;
        let mut pass = false;
        let mut started_holding = false;
        if let Some(prev) = &st.closed_prev {
            if prev.stop == StopReason::WindowChanged {
                discard = self.held.len();
                self.held.clear();
            } else {
                release.append(&mut self.held);
            }
            self.holding = false;
            recs.push(prev.clone());
        }
        if st.in_episode {
            if st.started {
                self.holding = may_hold;
                started_holding = may_hold;
            }
            if self.holding {
                self.held.push(id);
            } else {
                pass = true;
            }
            if let Some(now) = &st.closed_now {
                if self.holding {
                    match now.verdict {
                        Verdict::Japanese(_) => write = Some(Cmd::Write),
                        Verdict::NotJapanese => write = Some(Cmd::Enter),
                        _ => {}
                    }
                }
                release.append(&mut self.held);
                self.holding = false;
                recs.push(now.clone());
            }
        } else {
            pass = true;
        }
        // 保持が続いているだけのステップは、タイマーに触れない。
        let timer = if !self.holding {
            Some(Cmd::Disarm)
        } else if started_holding {
            Some(Cmd::Arm)
        } else {
            None
        };
        let cmds = Self::assemble(
            discard,
            write,
            recs,
            release,
            timer,
            Some(if pass { Cmd::Pass(id) } else { Cmd::Consume }),
        );
        (cmds, st)
    }

    fn on_timer(&mut self, scope: u64) -> Vec<Cmd> {
        self.holding = false;
        let Some(ep) = self.tr.active.as_ref() else {
            return vec![Cmd::Disarm];
        };
        let deadline = ep.start + MAX_MS;
        let same = ep.scope == scope;
        let rec = self.tr.close(
            if same {
                StopReason::Timeout
            } else {
                StopReason::WindowChanged
            },
            deadline,
        );
        let (mut discard, mut release) = (0, Vec::new());
        if same {
            release.append(&mut self.held);
        } else {
            discard = self.held.len();
            self.held.clear();
        }
        Self::assemble(discard, None, vec![rec], release, Some(Cmd::Disarm), None)
    }
}

// ---- 不変条件（参照モデルとは別に、命令の列だけから検査する）----

#[derive(Debug, Clone, Copy, PartialEq)]
enum Fate {
    Passed,
    Replayed,
    Discarded,
}

#[derive(Default)]
struct Inv {
    /// 保持列（入力順の id）
    held: Vec<u32>,
    fates: HashMap<u32, Fate>,
    /// 殻のタイマーが張られているか（`ArmDeadline`/`DisarmDeadline` から模す）
    shell_armed: bool,
    records: usize,
}

fn rank(c: &Cmd) -> u8 {
    match c {
        Cmd::Discard(_) => 0,
        Cmd::Write | Cmd::Enter => 1,
        Cmd::Record(_) => 2,
        Cmd::Replay(_) => 3,
        Cmd::Arm | Cmd::Disarm => 4,
        Cmd::Pass(_) | Cmd::Consume => 5,
    }
}

impl Inv {
    /// `id` が `Some` なら打鍵のステップ、`None` ならタイマーのステップ。
    fn check(
        &mut self,
        cmds: &[Cmd],
        id: Option<u32>,
        may_hold: bool,
        ctl_held: usize,
        ctl_holding: bool,
    ) -> Result<(), String> {
        // 命令の順序
        if !cmds.windows(2).all(|w| rank(&w[0]) <= rank(&w[1])) {
            return Err("命令の順序が 書き込み→Record→Replay→タイマー の並びでない".into());
        }
        let count = |f: fn(&Cmd) -> bool| cmds.iter().filter(|c| f(c)).count();
        if count(|c| matches!(c, Cmd::Discard(_))) > 1
            || count(|c| matches!(c, Cmd::Write | Cmd::Enter)) > 1
            || count(|c| matches!(c, Cmd::Replay(_))) > 1
        {
            return Err("Discard/書き込み/Replay は 1 ステップに高々 1 つ".into());
        }
        let records: Vec<&Rec> = cmds
            .iter()
            .filter_map(|c| {
                if let Cmd::Record(r) = c {
                    Some(r)
                } else {
                    None
                }
            })
            .collect();
        let max_records = if id.is_some() { 2 } else { 1 };
        if records.len() > max_records {
            return Err("Record が多すぎる".into());
        }
        self.records += records.len();
        if cmds.iter().any(|c| matches!(c, Cmd::Write | Cmd::Enter)) {
            // 書き込みは、保持していたエピソードが判定で確定したステップの最後の Record に伴う
            let last = records.last().ok_or("書き込みに Record が無い")?;
            let want_hiragana = cmds.contains(&Cmd::Write);
            let ok = last.stop == StopReason::Decided
                && if want_hiragana {
                    matches!(last.verdict, Verdict::Japanese(_))
                } else {
                    last.verdict == Verdict::NotJapanese
                };
            if !ok {
                return Err(format!("書き込みと最後の Record が整合しない: {last:?}"));
            }
        }
        let timers = count(|c| matches!(c, Cmd::Arm | Cmd::Disarm));
        if timers > 1 || (id.is_none() && !cmds.contains(&Cmd::Disarm)) {
            return Err("タイマーの命令が不正（タイマーのステップは DisarmDeadline のみ）".into());
        }
        let key_fates = count(|c| matches!(c, Cmd::Pass(_) | Cmd::Consume));
        match id {
            Some(_) if key_fates != 1 => {
                return Err("打鍵の行き先が Pass/Consume のちょうど 1 つでない".into())
            }
            None if key_fates != 0 => return Err("タイマーで Pass/Consume が出た".into()),
            _ => {}
        }
        // 保持しない（観測のみ）
        if !may_hold {
            let forbidden = cmds.iter().any(|c| {
                matches!(
                    c,
                    Cmd::Discard(_)
                        | Cmd::Write
                        | Cmd::Enter
                        | Cmd::Replay(_)
                        | Cmd::Arm
                        | Cmd::Consume
                )
            });
            if forbidden {
                return Err(
                    "may_hold = false なのに保持・解放・書き込み・タイマー起動が出た".into(),
                );
            }
        }
        // 保存則・保持列
        let mut current_consumed = false;
        for c in cmds {
            match c {
                Cmd::Discard(n) => {
                    if *n != self.held.len() {
                        return Err(format!(
                            "Discard({n}) が保持列 {:?} の全部でない",
                            self.held
                        ));
                    }
                    for h in self.held.drain(..) {
                        if self.fates.insert(h, Fate::Discarded).is_some() {
                            return Err(format!("打鍵 {h} の運命が 2 回"));
                        }
                    }
                }
                Cmd::Pass(x) => {
                    if Some(*x) != id {
                        return Err(format!("Pass({x}) が今の打鍵でない"));
                    }
                    if self.fates.insert(*x, Fate::Passed).is_some() {
                        return Err(format!("打鍵 {x} の運命が 2 回"));
                    }
                }
                Cmd::Consume => current_consumed = true,
                Cmd::Replay(list) => {
                    // 保持列の全部（入力順）に続けて、今の打鍵（契機。保持に残ることもあるので、あれば最後に 1 つ）。
                    let consumed = cmds.contains(&Cmd::Consume);
                    if list.len() < self.held.len() || list[..self.held.len()] != self.held[..] {
                        return Err(format!(
                            "Replay {list:?} が保持列 {:?} の全部を先頭に持たない",
                            self.held
                        ));
                    }
                    let rest = &list[self.held.len()..];
                    if !(rest.is_empty() || (rest.len() == 1 && Some(rest[0]) == id && consumed)) {
                        return Err(format!("Replay {list:?} の末尾が不正"));
                    }
                    if !list.windows(2).all(|w| w[0] < w[1]) {
                        return Err(format!("Replay {list:?} が入力順でない"));
                    }
                    for x in list {
                        if self.fates.insert(*x, Fate::Replayed).is_some() {
                            return Err(format!("打鍵 {x} の運命が 2 回"));
                        }
                    }
                    self.held.clear();
                }
                Cmd::Arm => self.shell_armed = true,
                Cmd::Disarm => self.shell_armed = false,
                _ => {}
            }
        }
        // 今の打鍵が Consume で、Replay にも入らなかった → 保持列に残る
        if let Some(cur) = id {
            if current_consumed && !self.fates.contains_key(&cur) {
                self.held.push(cur);
            }
        }
        // タイマーの整合: 張られているのは保持中だけ
        let holding = !self.held.is_empty();
        if self.shell_armed != holding || ctl_holding != holding || ctl_held != self.held.len() {
            return Err(format!(
                "タイマー/保持の不整合: armed={} holding(ctl)={ctl_holding} held={:?} held_len(ctl)={ctl_held}",
                self.shell_armed, self.held
            ));
        }
        Ok(())
    }

    fn all_resolved(&self, last_id: u32, episodes_started: usize) -> Result<(), String> {
        if !self.held.is_empty() {
            return Err("最後のタイマーのあとも保持列が残った".into());
        }
        for i in 1..=last_id {
            if !self.fates.contains_key(&i) {
                return Err(format!("打鍵 {i} がどの運命にも対応しない（欠落）"));
            }
        }
        if self.records != episodes_started {
            return Err(format!(
                "Record の数 {} がエピソードの数 {episodes_started} と違う",
                self.records
            ));
        }
        Ok(())
    }
}

// ---- 字母と列の実行 ----

#[derive(Debug, Clone, Copy)]
enum Ev {
    Key {
        role: KeyRole,
        down: bool,
        excluded: bool,
    },
    Gap(u64),
    Switch,
    Timer,
}

const fn ch(p: P, down: bool) -> Ev {
    Ev::Key {
        role: KeyRole::Char(PhysicalPos { row: p.0, col: p.1 }),
        down,
        excluded: false,
    }
}

const fn th(side: ThumbSide, down: bool) -> Ev {
    Ev::Key {
        role: KeyRole::Thumb(side),
        down,
        excluded: false,
    }
}

const OTHER_DOWN: Ev = Ev::Key {
    role: KeyRole::Other,
    down: true,
    excluded: false,
};
const EXCLUDED_DOWN: Ev = Ev::Key {
    role: KeyRole::Char(PhysicalPos {
        row: KA.0,
        col: KA.1,
    }),
    down: true,
    excluded: true,
};
const MOD_DOWN: Ev = Ev::Key {
    role: KeyRole::Modifier,
    down: true,
    excluded: false,
};

/// 字母全部（19 種）。
fn full_alphabet() -> Vec<Ev> {
    vec![
        ch(KA, true),
        ch(KA, false),
        ch(TA, true),
        ch(TA, false),
        ch(SA, true),
        ch(SA, false),
        ch(SYM, true),
        ch(SYM, false),
        th(ThumbSide::Left, true),
        th(ThumbSide::Left, false),
        th(ThumbSide::Right, true),
        th(ThumbSide::Right, false),
        OTHER_DOWN,
        EXCLUDED_DOWN,
        MOD_DOWN,
        Ev::Gap(MAX_MS),
        Ev::Gap(IDLE_MS),
        Ev::Switch,
        Ev::Timer,
    ]
}

/// 小さい字母（11 種）。長い列の全列挙用。
fn core_alphabet() -> Vec<Ev> {
    vec![
        ch(KA, true),
        ch(KA, false),
        ch(TA, true),
        ch(SA, true),
        th(ThumbSide::Left, true),
        th(ThumbSide::Left, false),
        OTHER_DOWN,
        Ev::Gap(MAX_MS),
        Ev::Gap(IDLE_MS),
        Ev::Switch,
        Ev::Timer,
    ]
}

/// さらに小さい字母（7 種）。長さ 6 の全列挙用。
fn tiny_alphabet() -> Vec<Ev> {
    vec![
        ch(KA, true),
        ch(TA, true),
        ch(SA, true),
        th(ThumbSide::Left, true),
        Ev::Gap(MAX_MS),
        Ev::Gap(IDLE_MS),
        Ev::Switch,
    ]
}

fn run(seq: &[Ev], may_hold: bool) -> Result<(), String> {
    let m = kana_model();
    let mut imp: HoldController<u32> = HoldController::new(tracker_cfg());
    let mut trk = EpisodeTracker::new(tracker_cfg());
    let mut rf = RefHold::new();
    let mut inv = Inv::default();
    let (mut now, mut scope, mut id) = (0_u64, 1_u64, 0_u32);
    let fail = |i: usize, what: &str| format!("step {i} ({:?}): {what}", seq.get(i));
    for (i, ev) in seq.iter().enumerate() {
        match *ev {
            Ev::Gap(g) => now += g,
            Ev::Switch => scope = if scope == 1 { 2 } else { 1 },
            Ev::Key {
                role,
                down,
                excluded,
            } => {
                now += 10;
                id += 1;
                let key = ObservedKey {
                    role,
                    down,
                    excluded,
                    timestamp_ms: now,
                };
                let got = normalize(&imp.on_key(m, key, id, scope, may_hold));
                let (want, rks) = rf.on_key(key, id, scope, may_hold);
                // 追跡器の KeyStep（closed_prev / closed_now / in_episode）
                let ks = trk.on_key_step(m, key, scope);
                let to_rec = |o: &Option<super::EpisodeOutcome>| o.as_ref().map(rec_of);
                if to_rec(&ks.closed_prev) != rks.closed_prev
                    || to_rec(&ks.closed_now) != rks.closed_now
                    || ks.in_episode != rks.in_episode
                {
                    return Err(fail(
                        i,
                        &format!(
                            "KeyStep が参照と違う: {ks:?} 参照 {:?}/{:?}/{}",
                            rks.closed_prev, rks.closed_now, rks.in_episode
                        ),
                    ));
                }
                if got != want {
                    return Err(fail(
                        i,
                        &format!("命令の列が参照と違う\n  実装 {got:?}\n  参照 {want:?}"),
                    ));
                }
                inv.check(&got, Some(id), may_hold, imp.held_len(), imp.is_holding())
                    .map_err(|e| fail(i, &e))?;
            }
            Ev::Timer => {
                let got = normalize(&imp.on_timer(scope));
                if let Some(s) = trk.active_scope() {
                    let deadline = trk.active_deadline_ms().unwrap_or(0);
                    let stop = if s == scope {
                        StopReason::Timeout
                    } else {
                        StopReason::WindowChanged
                    };
                    trk.force_close(stop, deadline);
                }
                let want = rf.on_timer(scope);
                if got != want {
                    return Err(fail(
                        i,
                        &format!("タイマーの命令の列が参照と違う\n  実装 {got:?}\n  参照 {want:?}"),
                    ));
                }
                inv.check(&got, None, may_hold, imp.held_len(), imp.is_holding())
                    .map_err(|e| fail(i, &e))?;
            }
        }
    }
    // 最後に期限のタイマーを発火して、保持を全部解決させる。
    let got = normalize(&imp.on_timer(scope));
    let want = rf.on_timer(scope);
    if got != want {
        return Err(format!("最後のタイマー: 実装 {got:?} 参照 {want:?}"));
    }
    inv.check(&got, None, may_hold, imp.held_len(), imp.is_holding())
        .map_err(|e| format!("最後のタイマー: {e}"))?;
    inv.all_resolved(id, rf.tr.started_total)
        .map_err(|e| format!("終了時: {e}"))
}

/// 字母 `alphabet` の長さ `len` の列を全部、`may_hold` の両方で実行する。件数を返す。
fn enumerate(alphabet: &[Ev], len: usize) -> usize {
    let n = alphabet.len();
    let mut idx = vec![0_usize; len];
    let mut count = 0;
    loop {
        let seq: Vec<Ev> = idx.iter().map(|&k| alphabet[k]).collect();
        for may_hold in [false, true] {
            if let Err(e) = run(&seq, may_hold) {
                panic!("may_hold={may_hold} 列 {seq:?}\n{e}");
            }
            count += 1;
        }
        // 桁上げ
        let mut pos = len;
        loop {
            if pos == 0 {
                return count;
            }
            pos -= 1;
            idx[pos] += 1;
            if idx[pos] < n {
                break;
            }
            idx[pos] = 0;
        }
    }
}

#[test]
fn full_alphabet_all_sequences_up_to_length_4() {
    let a = full_alphabet();
    assert_eq!(a.len(), 19);
    let total: usize = (1..=4).map(|len| enumerate(&a, len)).sum();
    // 2 × (19 + 19^2 + 19^3 + 19^4)
    assert_eq!(total, 2 * (19 + 361 + 6859 + 130_321));
}

#[test]
fn core_alphabet_all_sequences_of_length_5() {
    let a = core_alphabet();
    assert_eq!(a.len(), 11);
    assert_eq!(enumerate(&a, 5), 2 * 161_051);
}

#[test]
fn tiny_alphabet_all_sequences_of_length_6() {
    let a = tiny_alphabet();
    assert_eq!(a.len(), 7);
    assert_eq!(enumerate(&a, 6), 2 * 117_649);
}

/// 固定 seed の疑似乱数で、長い列（長さ 14）を全字母から。
#[test]
fn random_long_sequences_with_a_fixed_seed() {
    let a = full_alphabet();
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    let mut next = move || {
        // xorshift64*
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        state.wrapping_mul(0x2545_F491_4F6C_DD1D)
    };
    let mut runs = 0;
    for _ in 0..20_000 {
        let seq: Vec<Ev> = (0..14)
            .map(|_| {
                let r = usize::try_from(next() >> 33).expect("usize");
                a[r % a.len()]
            })
            .collect();
        for may_hold in [false, true] {
            if let Err(e) = run(&seq, may_hold) {
                panic!("may_hold={may_hold} 列 {seq:?}\n{e}");
            }
            runs += 1;
        }
    }
    assert_eq!(runs, 40_000);
}

// ---- EpisodeDetector 単体 ----

#[derive(Clone, Copy, Debug)]
struct DetCfg {
    params: DetectParams,
    overlap: ThumbOverlap,
    thumbs_held: [bool; 2],
}

fn det_alphabet() -> Vec<DetectEvent> {
    let mut v = Vec::new();
    for p in [KA, TA, SA, SYM] {
        v.push(DetectEvent::CharDown(pp(p)));
    }
    for p in [KA, TA, SA, SYM] {
        v.push(DetectEvent::CharUp(pp(p)));
    }
    for s in [ThumbSide::Left, ThumbSide::Right] {
        v.push(DetectEvent::ThumbDown(s));
    }
    for s in [ThumbSide::Left, ThumbSide::Right] {
        v.push(DetectEvent::ThumbUp(s));
    }
    v
}

fn side_index(s: ThumbSide) -> usize {
    usize::from(s == ThumbSide::Right)
}

/// 1 列を判定器に流して、各ステップで不変条件を調べる。
fn check_detector(
    model: &KanaModel,
    usable: bool,
    cfg: DetCfg,
    seq: &[DetectEvent],
) -> Result<(), String> {
    let mut d = EpisodeDetector::new(cfg.params, cfg.overlap, cfg.thumbs_held);
    // 独立に持つ押下状態
    let mut chars: BTreeSet<P> = BTreeSet::new();
    let mut thumbs = cfg.thumbs_held;
    let mut presses = 0_usize;
    let mut llr = 0.0_f64;
    let mut prev: Option<char> = None;
    let k = ref_kana();
    for (i, ev) in seq.iter().enumerate() {
        let before = (d.verdict(), d.llr(), d.presses());
        let v = d.observe(model, *ev);
        let err = |m: &str| {
            format!(
                "step {i} {ev:?}: {m} (verdict {v:?} llr {} presses {})",
                d.llr(),
                d.presses()
            )
        };
        if before.0.is_decided() {
            // 吸収的: 決まったあとは何も変わらない
            if v != before.0 || d.llr().to_bits() != before.1.to_bits() || d.presses() != before.2 {
                return Err(err("決まったあとに変わった"));
            }
            continue;
        }
        let enabled = cfg.overlap == ThumbOverlap::Enabled;
        let mut overlap = false;
        let mut counted = false;
        match *ev {
            DetectEvent::CharDown(p) => {
                let q = (p.row, p.col);
                if chars.insert(q) {
                    if enabled && thumbs.iter().any(|&t| t) {
                        overlap = true;
                    } else {
                        counted = true;
                        presses += 1;
                        if usable {
                            llr += k.delta(prev, q);
                            prev = k.kana_of.get(&q).copied();
                        }
                    }
                }
            }
            DetectEvent::CharUp(p) => {
                chars.remove(&(p.row, p.col));
            }
            DetectEvent::ThumbDown(s) if enabled => {
                if chars.is_empty() {
                    thumbs[side_index(s)] = true;
                } else {
                    overlap = true;
                }
            }
            DetectEvent::ThumbUp(s) if enabled => thumbs[side_index(s)] = false,
            DetectEvent::ThumbDown(_) | DetectEvent::ThumbUp(_) => {}
        }
        if d.presses() != presses {
            return Err(err("押下数が違う（自動リピート・重なりの数え方）"));
        }
        if (d.llr() - llr).abs() > 1e-6 {
            return Err(err(&format!("llr が参照 {llr} と違う")));
        }
        let trace_sum: f64 = d.llr_trace().iter().map(|&x| f64::from(x)).sum();
        if usable {
            if d.llr_trace().len() != presses || (trace_sum - d.llr()).abs() > 1e-3 {
                return Err(err("llr が trace の和でない"));
            }
        } else if !d.llr_trace().is_empty() || d.llr() != 0.0 {
            return Err(err("表が使えないのに尤度を積んだ"));
        }
        let (ja, en, max) = (
            cfg.params.ja_threshold,
            cfg.params.en_threshold,
            cfg.params.max_keys,
        );
        let expected = if overlap {
            Verdict::Japanese(JapaneseEvidence::ThumbOverlap)
        } else if counted && usable && d.llr() >= ja {
            Verdict::Japanese(JapaneseEvidence::Likelihood)
        } else if counted && usable && d.llr() <= -en {
            Verdict::NotJapanese
        } else if counted && presses >= max {
            Verdict::Undecided
        } else {
            Verdict::Pending
        };
        if v != expected {
            return Err(err(&format!("判定が期待 {expected:?} と違う")));
        }
        // 判定の意味（独立な言い換え）
        match v {
            Verdict::NotJapanese if !(d.llr() <= -en) => {
                return Err(err("NotJapanese なのに llr > -en"))
            }
            Verdict::Japanese(JapaneseEvidence::Likelihood) if !(d.llr() >= ja) => {
                return Err(err("Likelihood なのに llr < ja"))
            }
            Verdict::Undecided if presses != max => {
                return Err(err("Undecided が max_keys 以外で出た"))
            }
            Verdict::Japanese(JapaneseEvidence::ThumbOverlap) if !enabled => {
                return Err(err("重なりを無効にしたのに重なりで確定した"))
            }
            _ => {}
        }
    }
    Ok(())
}

fn enumerate_detector(model: &KanaModel, usable: bool, cfg: DetCfg, len: usize) -> usize {
    let alphabet = det_alphabet();
    let n = alphabet.len();
    let mut idx = vec![0_usize; len];
    let mut count = 0;
    loop {
        let seq: Vec<DetectEvent> = idx.iter().map(|&k| alphabet[k]).collect();
        if let Err(e) = check_detector(model, usable, cfg, &seq) {
            panic!("cfg {cfg:?} 列 {seq:?}\n{e}");
        }
        count += 1;
        let mut pos = len;
        loop {
            if pos == 0 {
                return count;
            }
            pos -= 1;
            idx[pos] += 1;
            if idx[pos] < n {
                break;
            }
            idx[pos] = 0;
        }
    }
}

fn det_cfg(overlap: ThumbOverlap, thumbs_held: [bool; 2]) -> DetCfg {
    DetCfg {
        params: DetectParams {
            ja_threshold: JA,
            en_threshold: EN,
            max_keys: MAX_KEYS,
        },
        overlap,
        thumbs_held,
    }
}

#[test]
fn detector_all_sequences_of_length_5_with_overlap() {
    let m = kana_model();
    let n = enumerate_detector(m, true, det_cfg(ThumbOverlap::Enabled, [false; 2]), 5);
    assert_eq!(n, 12_usize.pow(5));
}

#[test]
fn detector_all_sequences_up_to_length_4_in_every_configuration() {
    let m = kana_model();
    let mut total = 0;
    for overlap in [ThumbOverlap::Enabled, ThumbOverlap::Disabled] {
        for thumbs_held in [[false; 2], [true, false], [false, true]] {
            for len in 1..=4 {
                total += enumerate_detector(m, true, det_cfg(overlap, thumbs_held), len);
            }
        }
    }
    assert_eq!(total, 6 * (12 + 144 + 1728 + 20_736));
}

#[test]
fn detector_with_an_unusable_table_decides_only_by_overlap() {
    // 表が空: 尤度を積まず、日本語は重なりだけ、max_keys で未決。
    let ngram = NgramModel::new(0, 0, 0);
    let mut face = YabFace::new();
    for (p, romaji, kana) in [(KA, "ka", 'か'), (TA, "ta", 'た'), (SA, "sa", 'さ')] {
        face.insert(
            pp(p),
            YabValue::Romaji {
                romaji: romaji.to_string(),
                kana: Some(kana),
            },
        );
    }
    let model = KanaModel::new(&ngram, &face);
    assert!(!model.usable());
    let mut total = 0;
    for len in 1..=4 {
        total += enumerate_detector(
            &model,
            false,
            det_cfg(ThumbOverlap::Enabled, [false; 2]),
            len,
        );
    }
    assert_eq!(total, 12 + 144 + 1728 + 20_736);
}
