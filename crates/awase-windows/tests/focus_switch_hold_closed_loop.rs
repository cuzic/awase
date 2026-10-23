#![allow(clippy::all, clippy::pedantic, clippy::nursery)]
//! PR #817(窓切替直後の短い保持、#809 の M1 の修正)の判断 `state/focus_switch_hold.rs::FocusSwitchHold` を、擬似 IME
//! (ADR-277 の遷移表 `support/pseudo_ime.rs::TransitionTable`)と仮想時計の閉ループに載せて網羅的に検証する。
//!
//! # 何を本物で、何を写しで回すか
//!
//! - **本物**: 保持の状態機械 `FocusSwitchHold`(#817 の純粋関数そのもの)と、擬似 IME の遷移表(実測の格子)。
//! - **写し(殻のモデル)**: `cfg(windows)` の配線(`runtime/focus_switch_hold.rs`・`deliver_key_event`・
//!   `INPUT_DEFER.replay_later` と drain・`spawn_ime_refresh` の世代・debounce 50ms のタイマー)を、本 PR の head
//!   (`e5f019e7`)のコードを読んで小さく写した `Sim`。**配線そのものは検証の対象外**(Linux では走らない)。
//!   写した性質: フックの打鍵は defer キューが空でなければキューへ積む(`app/mod.rs::handle_hook_key_event`)、
//!   drain はタイムスタンプ順(`take_all` の安定ソート)、refresh は debounce の後に spawn され、spawn 時点の世代と
//!   その時点の窓を読む、解放は refresh の完了・期限のタイマー・上限・期限超えの打鍵。
//! - **読み直しの時間**: CI の実測(#817 本文: `[focus-sync]` → `[notify-refresh]`、n=1,564、P50/P90/P95/P99/max =
//!   67/77/81/109/228ms)を**入力として与える**。全列挙はその分位点と 250ms 前後・超過・取りこぼしを並べ、ランダム
//!   ウォークは分位点の区分一様で近似した分布から引く(250ms 超・取りこぼしの小確率は仮定)。
//! - **IME とエンジンの効き**: エンジンの活性 = 「文脈の窓(最後に完了した refresh が読んだ窓)の belief が開」。
//!   素通しの IME キー(エンジン非活性の親指・親指でない変換・半角/全角)は、フォーカスの窓の真値を擬似 IME の表で
//!   動かし、予測(belief)は**文脈の窓**の表で動かす(古い文脈で処理されると、予測が別の窓に付く)。読める窓は
//!   refresh の完了で belief を真値に合わせ、読めない窓は合わせない。CI(#816・#817)で観測した「読めない窓 ×
//!   GJI で素通しの変換が GJI を閉じ、以後ローマ字のまま」をこの仮定で表す。
//!
//! # 不変条件(`Viol` の各列)
//!
//! loss(キーが落ちない)・dup(二重に届かない)・order(物理の順に届く。保持の外で届いた Down の Up が保持中の打鍵を
//! 追い越すのは #817 の仕様の例外として order_exempt に分けて数える)・modifier(文字が届いた時点の OS の Shift が
//! 押下時と同じ)・delay(素通しされるべき打鍵〈保持の外の Down の Up・保持中でない時の保持を始めない打鍵〉を遅らせない)・
//! bound(どの打鍵も 250ms を超えて遅れない)・cap(保持中の Down は 8 件まで)・stuck(終わりに保持が残らない)・
//! content(解放の中身が保持した打鍵と一致)・stale_release(refresh の完了での解放は新しい窓を読んだ後)・
//! stale_start(保持を始める打鍵〈文字・親指〉が古い窓の文脈で処理されない)・stale_other(それ以外の Down、参考)・
//! stale_regress(新しい世代の refresh の後に古い世代の refresh が完了して文脈が戻った間の処理。保持と無関係の既存の並びで参考)・
//! fix(読めない窓で belief と真値の開閉が食い違ったまま終わらない=ローマ字固着)・route(打った窓と届いた窓が同じ)。
//!
//! # ablation
//!
//! `Mirror` は `FocusSwitchHold` の写しで、防御を 1 つずつ外せる。既定(何も外さない)の `Mirror` が本物と同じ軌跡を
//! 出すことを全列挙とランダムウォークの全件で確かめてから(`mirror_matches_real_*`)、外した版で違反が増えるかを数える。
//!
//! 再生成: `UPDATE_GOLDEN=1 cargo test -p awase-windows --test focus_switch_hold_closed_loop`
//! 結果の読み方: `docs/tasks/focus-switch-hold-closed-loop-result-2026-10-23.md`。

mod support;

use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::Instant;

use awase_windows::state::focus_switch_hold::{
    starts_hold, FocusSwitchHold, HoldKey, OnKey, StartKind, FOCUS_SWITCH_HOLD_CAP, MOD_ALT,
    MOD_CTRL, MOD_SHIFT,
};
use awase_windows::state::half_width_resume_hold::KeyFacts;
use awase_windows::state::TickMs;
use awase_windows::tuning::FOCUS_SWITCH_HOLD_MAX_MS;
use support::pseudo_ime::{
    Grid, ImeInput, QuirkSwitches, Transition, TransitionTable, TrueStage, TrueState, CONV_HIRAGANA,
};

/// フォーカス変更の時刻(全列挙の基準)。
const F: u64 = 1000;
/// `focus_debounce_ms` の既定(`src/config.rs`)。refresh はフォーカス変更から 50ms 後に spawn される。
const DEBOUNCE: u64 = 50;
const MAX: u64 = FOCUS_SWITCH_HOLD_MAX_MS;

const RANK_FOCUS: u8 = 0;
const RANK_SPAWN: u8 = 1;
const RANK_KEY: u8 = 3;
const RANK_TIMER: u8 = 5;

const WIN_NAMES: [&str; 3] = ["A", "B", "C"];

// ---------------------------------------------------------------------------
// 打鍵
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Char,
    ThumbR,
    ThumbL,
    /// 親指として設定されていない変換キー(保持を始めない IME キー)。
    ConvKey,
    /// 半角/全角(保持を始めない IME キー)。
    HanZen,
    Shift,
    Ctrl,
    Alt,
    Tab,
}

impl Kind {
    const fn name(self) -> &'static str {
        match self {
            Self::Char => "char",
            Self::ThumbR => "thumbR",
            Self::ThumbL => "thumbL",
            Self::ConvKey => "conv",
            Self::HanZen => "hanzen",
            Self::Shift => "shift",
            Self::Ctrl => "ctrl",
            Self::Alt => "alt",
            Self::Tab => "tab",
        }
    }
    const fn is_modifier(self) -> bool {
        matches!(self, Self::Shift | Self::Ctrl | Self::Alt)
    }
}

#[derive(Clone, Copy, Debug)]
struct Ev {
    t: u64,
    kind: Kind,
    identity: u64,
    down: bool,
    repeat: bool,
    shift: bool,
    ctrl: bool,
    alt: bool,
    injected: bool,
}

impl Ev {
    const fn new(
        t: u64,
        kind: Kind,
        identity: u64,
        down: bool,
        repeat: bool,
        injected: bool,
    ) -> Self {
        Self {
            t,
            kind,
            identity,
            down,
            repeat,
            shift: false,
            ctrl: false,
            alt: false,
            injected,
        }
    }

    /// この打鍵が修飾キーなら、その印(`MOD_*`)。
    const fn modifier_bit(&self) -> u8 {
        match self.kind {
            Kind::Shift => MOD_SHIFT,
            Kind::Ctrl => MOD_CTRL,
            Kind::Alt => MOD_ALT,
            _ => 0,
        }
    }

    /// 殻の `runtime/focus_switch_hold.rs::hold_key` の写し(種類: 文字・親指/IME のモードキー〈親指でない変換・
    /// 半角/全角〉/その他。押下時の修飾と、この打鍵自体の修飾の印)。
    const fn hold_key(&self) -> HoldKey {
        let mods_at_press = (if self.shift { MOD_SHIFT } else { 0 })
            | (if self.ctrl { MOD_CTRL } else { 0 })
            | (if self.alt { MOD_ALT } else { 0 });
        HoldKey {
            facts: self.facts(),
            mods_at_press,
            modifier: self.modifier_bit(),
            injected: self.injected,
            kind: match self.kind {
                // 親指は変換・無変換(IME のモードキー)。殻は IME のモードキーを先に見る。
                Kind::ThumbR | Kind::ThumbL | Kind::ConvKey | Kind::HanZen => StartKind::ImeKey,
                Kind::Char => StartKind::CharOrThumb,
                _ => StartKind::Other,
            },
        }
    }

    /// 保持を始める打鍵か(本物の `starts_hold`)。
    const fn starts(&self) -> bool {
        starts_hold(self.hold_key())
    }

    const fn facts(&self) -> KeyFacts {
        KeyFacts {
            identity: self.identity,
            is_down: self.down,
            is_repeat: self.repeat,
        }
    }

    fn label(&self) -> String {
        let dir = if !self.down {
            "↑"
        } else if self.repeat {
            "⟳"
        } else {
            "↓"
        };
        let mut s = format!("{}{dir}", self.kind.name());
        if self.shift {
            s.push_str("+S");
        }
        if self.ctrl {
            s.push_str("+C");
        }
        if self.alt {
            s.push_str("+A");
        }
        if self.injected {
            s.push_str("(inj)");
        }
        s
    }
}

// ---------------------------------------------------------------------------
// シナリオ
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
struct WinInit {
    grid: Grid,
    open: bool,
    readable: bool,
}

impl WinInit {
    fn label(&self) -> String {
        format!(
            "{}{}{}",
            match self.grid {
                Grid::Atok => "GJI",
                Grid::GjiMsime => "GJI(MS)",
                Grid::MsimeNative => "MS-IME",
            },
            if self.open { "ON" } else { "OFF" },
            if self.readable {
                "/読める"
            } else {
                "/読めない"
            }
        )
    }
}

#[derive(Clone, Copy, Debug)]
enum Act {
    Focus {
        t: u64,
        win: usize,
        r: Option<u64>,
    },
    /// フォーカス変更より前に spawn され(その時点の窓と世代を読む)、`done` に完了する refresh。
    Stale {
        spawn: u64,
        done: u64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum RClass {
    /// 読み直しが実測の範囲(≤228ms)。
    Measured,
    /// 228ms < R ≤ 250ms(定数のマージン)。
    Margin,
    /// 250ms 超。
    Over,
    /// refresh の完了を取りこぼした。
    Lost,
}

impl RClass {
    const ALL: [Self; 4] = [Self::Measured, Self::Margin, Self::Over, Self::Lost];
    fn of(r: Option<u64>) -> Self {
        match r {
            None => Self::Lost,
            Some(r) if r <= 228 => Self::Measured,
            Some(r) if r <= 250 => Self::Margin,
            Some(_) => Self::Over,
        }
    }
    const fn name(self) -> &'static str {
        match self {
            Self::Measured => "R≤228",
            Self::Margin => "228<R≤250",
            Self::Over => "R>250",
            Self::Lost => "取りこぼし",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FirstKind {
    Char,
    ThumbR,
    ThumbL,
    ConvKey,
    HanZen,
    ShiftChar,
    CtrlChar,
    AltTab,
    UpOnly,
    Repeat,
    Injected,
}

const FIRST_KINDS: [FirstKind; 11] = [
    FirstKind::Char,
    FirstKind::ThumbR,
    FirstKind::ThumbL,
    FirstKind::ConvKey,
    FirstKind::HanZen,
    FirstKind::ShiftChar,
    FirstKind::CtrlChar,
    FirstKind::AltTab,
    FirstKind::UpOnly,
    FirstKind::Repeat,
    FirstKind::Injected,
];

impl FirstKind {
    const fn name(self) -> &'static str {
        match self {
            Self::Char => "文字",
            Self::ThumbR => "右親指(変換)",
            Self::ThumbL => "左親指(無変換)",
            Self::ConvKey => "親指でない変換",
            Self::HanZen => "半角/全角",
            Self::ShiftChar => "Shift+文字",
            Self::CtrlChar => "Ctrl+文字",
            Self::AltTab => "Alt+Tab の Tab",
            Self::UpOnly => "キーアップだけ",
            Self::Repeat => "リピート",
            Self::Injected => "外部注入の文字",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Follow {
    None,
    SecondChar,
    ThumbChord,
    Burst12,
    Typing6,
    FocusC,
    FocusBackA,
    StaleBefore,
    StaleAfter,
    InjBurst,
}

const FOLLOWS: [Follow; 10] = [
    Follow::None,
    Follow::SecondChar,
    Follow::ThumbChord,
    Follow::Burst12,
    Follow::Typing6,
    Follow::FocusC,
    Follow::FocusBackA,
    Follow::StaleBefore,
    Follow::StaleAfter,
    Follow::InjBurst,
];

impl Follow {
    const fn name(self) -> &'static str {
        match self {
            Self::None => "なし",
            Self::SecondChar => "+40ms に文字",
            Self::ThumbChord => "+10ms に右親指(同時打鍵)",
            Self::Burst12 => "6ms 間隔で文字 12 打",
            Self::Typing6 => "40ms 間隔で文字 6 打",
            Self::FocusC => "+20ms に窓 C へフォーカス",
            Self::FocusBackA => "+20ms に窓 A へ戻る",
            Self::StaleBefore => "変更前の refresh が打鍵の 5ms 前に完了",
            Self::StaleAfter => "変更前の refresh が打鍵の 5ms 後に完了",
            Self::InjBurst => "1ms 間隔で外部注入 20 打",
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Desc {
    E1 {
        kind: FirstKind,
        d: u64,
        r: Option<u64>,
        up: u64,
        follow: Follow,
        refresh_first: bool,
    },
    E2 {
        a: WinInit,
        b: WinInit,
        kind: FirstKind,
        d: u64,
        r: Option<u64>,
    },
    Walk {
        seed: u64,
    },
}

impl std::fmt::Display for Desc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let r_s = |r: Option<u64>| r.map_or_else(|| "取りこぼし".to_owned(), |r| format!("{r}ms"));
        match *self {
            Self::E1 {
                kind,
                d,
                r,
                up,
                follow,
                refresh_first,
            } => write!(
                f,
                "最初の打鍵={} 遅延={d}ms 読み直し={} 離し=+{up}ms 追加={} 同時刻={}",
                kind.name(),
                r_s(r),
                follow.name(),
                if refresh_first {
                    "refresh が先"
                } else {
                    "打鍵が先"
                }
            ),
            Self::E2 { a, b, kind, d, r } => write!(
                f,
                "A={} B={} 最初の打鍵={} 遅延={d}ms 読み直し={}",
                a.label(),
                b.label(),
                kind.name(),
                r_s(r)
            ),
            Self::Walk { seed } => write!(f, "ランダムウォーク seed={seed}"),
        }
    }
}

#[derive(Clone, Debug)]
struct Scenario {
    desc: Desc,
    wins: [WinInit; 3],
    evs: Vec<Ev>,
    acts: Vec<Act>,
    refresh_first: bool,
    rclass: RClass,
}

impl Scenario {
    fn size(&self) -> usize {
        self.evs.len() + self.acts.len()
    }
}

struct Builder {
    keys: Vec<Ev>,
    acts: Vec<Act>,
    next_ident: u64,
}

impl Builder {
    fn new() -> Self {
        Self {
            keys: Vec::new(),
            acts: Vec::new(),
            next_ident: 0x100,
        }
    }
    fn ident(&mut self) -> u64 {
        self.next_ident += 1;
        self.next_ident
    }
    fn press(&mut self, kind: Kind, down: u64, up: u64, injected: bool) {
        let id = self.ident();
        self.keys
            .push(Ev::new(down, kind, id, true, false, injected));
        self.keys
            .push(Ev::new(up, kind, id, false, false, injected));
    }
    fn ev(&mut self, ev: Ev) {
        self.keys.push(ev);
    }
    fn focus(&mut self, t: u64, win: usize, r: Option<u64>) {
        self.acts.push(Act::Focus { t, win, r });
    }
    fn stale(&mut self, spawn: u64, done: u64) {
        self.acts.push(Act::Stale { spawn, done });
    }

    /// 物理の順(時刻、同時刻は追加順)に並べ、押下時の修飾キーの状態(Shift/Ctrl/Alt)を付ける。
    fn build(
        mut self,
        desc: Desc,
        wins: [WinInit; 3],
        refresh_first: bool,
        rclass: RClass,
    ) -> Scenario {
        self.keys.sort_by_key(|e| e.t);
        let mut shift: Vec<u64> = Vec::new();
        let mut ctrl: Vec<u64> = Vec::new();
        let mut alt: Vec<u64> = Vec::new();
        for ev in &mut self.keys {
            let set = match ev.kind {
                Kind::Shift => Some(&mut shift),
                Kind::Ctrl => Some(&mut ctrl),
                Kind::Alt => Some(&mut alt),
                _ => None,
            };
            if let Some(set) = set {
                if ev.down {
                    if !set.contains(&ev.identity) {
                        set.push(ev.identity);
                    }
                } else {
                    set.retain(|&i| i != ev.identity);
                }
                continue;
            }
            ev.shift = !shift.is_empty();
            ev.ctrl = !ctrl.is_empty();
            ev.alt = !alt.is_empty();
        }
        Scenario {
            desc,
            wins,
            evs: self.keys,
            acts: self.acts,
            refresh_first,
            rclass,
        }
    }
}

// ---------------------------------------------------------------------------
// 保持の実装(本物と、防御を外せる写し)
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
enum Step {
    Pass,
    Started,
    Held,
    Release(Vec<usize>),
}

trait Hold {
    fn on_focus_event(&mut self, now: u64, seq: u64);
    fn on_key(&mut self, now: u64, key: HoldKey, item: usize) -> Step;
    fn on_refresh_settled(&mut self, seq: u64) -> Option<Vec<usize>>;
    fn on_deadline(&mut self) -> Option<Vec<usize>>;
}

/// #817 の本物。
struct Real(FocusSwitchHold<usize>);

impl Real {
    fn new() -> Self {
        Self(FocusSwitchHold::new(MAX))
    }
}

impl Hold for Real {
    fn on_focus_event(&mut self, now: u64, seq: u64) {
        self.0.on_focus_event(TickMs(now), seq);
    }
    fn on_key(&mut self, now: u64, key: HoldKey, item: usize) -> Step {
        match self.0.on_key(TickMs(now), key, item) {
            OnKey::Pass(_) => Step::Pass,
            OnKey::Started => Step::Started,
            OnKey::Held => Step::Held,
            OnKey::Release(v) => Step::Release(v),
        }
    }
    fn on_refresh_settled(&mut self, seq: u64) -> Option<Vec<usize>> {
        self.0.on_refresh_settled(seq)
    }
    fn on_deadline(&mut self) -> Option<Vec<usize>> {
        self.0.on_deadline()
    }
}

/// 外す防御。既定(全部 false)の `Mirror` は本物と同じ軌跡を出す(`mirror_matches_real_*` が確かめる)。
#[derive(Clone, Copy, Debug, Default)]
struct Ablate {
    /// 保持そのものが無い(#809 の後・#817 の前と同じ)。基準。
    no_hold: bool,
    /// 上限 8 Down を外す。
    no_cap: bool,
    /// 世代の照合を外す(どの refresh の完了でも解放する)。
    no_gen: bool,
    /// 保持を始める打鍵の絞り込み(文字・親指・修飾なし・注入でない)を外す: どの Down でも始める。
    start_any_down: bool,
    /// さらに KeyUp・リピートでも始める。
    start_any_event: bool,
    /// 保持対象の絞り込み(保持の外の Down の Up・リピートは通す)を外す: 保持中は全部足す。
    hold_outside_up: bool,
    /// 期限のタイマーを外す。
    no_deadline: bool,
    /// 待機の期限(フォーカスから 250ms)を外す。
    no_armed_expiry: bool,
    /// 保持中のフォーカス変更で世代を新しくしない。
    keep_seq_on_focus: bool,
    /// 期限を過ぎた打鍵での解放を外す(タイマーだけに頼る)。
    no_expiry_on_key: bool,
    /// IME のモードキー(親指でない変換・半角/全角)から保持を始めない(V2 の修正の前)。
    no_ime_key_start: bool,
    /// 保持中の打鍵が使った修飾キーの Up を保持に足さない(V1 の修正の前)。
    no_modifier_join: bool,
}

fn ablations() -> Vec<(&'static str, &'static str, Ablate)> {
    let d = Ablate::default();
    vec![
        (
            "no-hold",
            "保持そのもの(基準: #809 の後)",
            Ablate { no_hold: true, ..d },
        ),
        ("no-cap", "上限 8 Down", Ablate { no_cap: true, ..d }),
        ("no-generation", "世代の照合", Ablate { no_gen: true, ..d }),
        (
            "start-any-down",
            "保持を始める打鍵の絞り込み(種類・修飾・注入)",
            Ablate {
                start_any_down: true,
                ..d
            },
        ),
        (
            "start-any-event",
            "保持を始める打鍵の絞り込み(Down だけ)",
            Ablate {
                start_any_event: true,
                ..d
            },
        ),
        (
            "hold-outside-up",
            "保持対象の絞り込み(保持の外の Down の Up を通す)",
            Ablate {
                hold_outside_up: true,
                ..d
            },
        ),
        (
            "no-deadline-timer",
            "期限のタイマー",
            Ablate {
                no_deadline: true,
                ..d
            },
        ),
        (
            "no-armed-expiry",
            "待機の期限(フォーカスから 250ms)",
            Ablate {
                no_armed_expiry: true,
                ..d
            },
        ),
        (
            "keep-seq-on-focus",
            "保持中のフォーカス変更で世代を新しくする",
            Ablate {
                keep_seq_on_focus: true,
                ..d
            },
        ),
        (
            "no-expiry-on-key",
            "期限超えの打鍵での解放",
            Ablate {
                no_expiry_on_key: true,
                ..d
            },
        ),
        (
            "no-ime-key-start",
            "IME のモードキーから保持を始める(V2 の修正)",
            Ablate {
                no_ime_key_start: true,
                ..d
            },
        ),
        (
            "no-modifier-join",
            "保持中の打鍵が使った修飾キーの Up を足す(V1 の修正)",
            Ablate {
                no_modifier_join: true,
                ..d
            },
        ),
    ]
}

#[derive(Debug)]
enum MPhase {
    Idle,
    Armed {
        at: u64,
        seq: u64,
    },
    Holding {
        seq: u64,
        started: u64,
        items: Vec<(HoldKey, usize)>,
    },
}

/// `FocusSwitchHold` の写し(防御を外せる)。
struct Mirror {
    phase: MPhase,
    ab: Ablate,
}

impl Mirror {
    const fn new(ab: Ablate) -> Self {
        Self {
            phase: MPhase::Idle,
            ab,
        }
    }
}

fn items_of(items: Vec<(HoldKey, usize)>) -> Vec<usize> {
    items.into_iter().map(|(_, i)| i).collect()
}

impl Hold for Mirror {
    fn on_focus_event(&mut self, now: u64, seq: u64) {
        let ab = self.ab;
        if ab.no_hold {
            return;
        }
        if let MPhase::Holding { seq: held_seq, .. } = &mut self.phase {
            if !ab.keep_seq_on_focus {
                *held_seq = seq;
            }
        } else {
            self.phase = MPhase::Armed { at: now, seq };
        }
    }

    fn on_key(&mut self, now: u64, key: HoldKey, item: usize) -> Step {
        let facts = key.facts;
        let ab = self.ab;
        if ab.no_hold {
            return Step::Pass;
        }
        match std::mem::replace(&mut self.phase, MPhase::Idle) {
            MPhase::Idle => Step::Pass,
            MPhase::Armed { at, seq } => {
                if !ab.no_armed_expiry && now.saturating_sub(at) >= MAX {
                    return Step::Pass;
                }
                let start = if ab.start_any_event {
                    true
                } else if ab.start_any_down {
                    facts.is_down && !facts.is_repeat
                } else if ab.no_ime_key_start && key.kind == StartKind::ImeKey {
                    false
                } else {
                    starts_hold(key)
                };
                if start {
                    self.phase = MPhase::Holding {
                        seq,
                        started: now,
                        items: vec![(key, item)],
                    };
                    return Step::Started;
                }
                self.phase = MPhase::Armed { at, seq };
                Step::Pass
            }
            MPhase::Holding {
                seq,
                started,
                mut items,
            } => {
                let has_down = items
                    .iter()
                    .any(|(k, _)| k.facts.is_down && k.facts.identity == facts.identity);
                let modifier_used = !ab.no_modifier_join
                    && key.modifier != 0
                    && items
                        .iter()
                        .any(|(k, _)| k.mods_at_press & key.modifier != 0);
                if !ab.hold_outside_up
                    && (!facts.is_down || facts.is_repeat)
                    && !has_down
                    && !modifier_used
                {
                    self.phase = MPhase::Holding {
                        seq,
                        started,
                        items,
                    };
                    return Step::Pass;
                }
                items.push((key, item));
                let downs = items
                    .iter()
                    .filter(|(k, _)| k.facts.is_down && !k.facts.is_repeat)
                    .count();
                let expired = !ab.no_expiry_on_key && now.saturating_sub(started) >= MAX;
                if (!ab.no_cap && downs > FOCUS_SWITCH_HOLD_CAP) || expired {
                    return Step::Release(items_of(items));
                }
                self.phase = MPhase::Holding {
                    seq,
                    started,
                    items,
                };
                Step::Held
            }
        }
    }

    fn on_refresh_settled(&mut self, s: u64) -> Option<Vec<usize>> {
        let ab = self.ab;
        match std::mem::replace(&mut self.phase, MPhase::Idle) {
            MPhase::Idle => None,
            MPhase::Armed { at, seq } => {
                if !ab.no_gen && s < seq {
                    self.phase = MPhase::Armed { at, seq };
                }
                None
            }
            MPhase::Holding {
                seq,
                started,
                items,
            } => {
                if !ab.no_gen && s < seq {
                    self.phase = MPhase::Holding {
                        seq,
                        started,
                        items,
                    };
                    return None;
                }
                Some(items_of(items))
            }
        }
    }

    fn on_deadline(&mut self) -> Option<Vec<usize>> {
        if self.ab.no_deadline || !matches!(self.phase, MPhase::Holding { .. }) {
            return None;
        }
        match std::mem::replace(&mut self.phase, MPhase::Idle) {
            MPhase::Holding { items, .. } => Some(items_of(items)),
            other => {
                self.phase = other;
                None
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 殻のモデル(仮想時計)
// ---------------------------------------------------------------------------

struct Tables {
    atok: TransitionTable,
    gji_msime: TransitionTable,
    msime: TransitionTable,
}

impl Tables {
    fn load() -> Self {
        Self {
            atok: TransitionTable::load(Grid::Atok),
            gji_msime: TransitionTable::load(Grid::GjiMsime),
            msime: TransitionTable::load(Grid::MsimeNative),
        }
    }
    const fn get(&self, g: Grid) -> &TransitionTable {
        match g {
            Grid::Atok => &self.atok,
            Grid::GjiMsime => &self.gji_msime,
            Grid::MsimeNative => &self.msime,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reason {
    Refreshed,
    Deadline,
    Cap,
    Expiry,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Item {
    Key(usize),
    Focus { win: usize, r: Option<u64> },
    Spawn { token: Option<u64>, done_at: u64 },
    Done { seq: u64, read: usize },
}

#[derive(Clone, Copy, Debug)]
struct WinState {
    grid: Grid,
    readable: bool,
    truth: TrueState,
    belief: TrueState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Delivery {
    id: usize,
    at: u64,
    focus: usize,
    context: usize,
    /// 文脈が、新しい世代の refresh の後に完了した古い世代の refresh で戻されていた(保持と無関係の既存の並び)。
    regressed: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Trace {
    deliveries: Vec<Delivery>,
    /// (時刻, 理由, 件数, 新しい窓を読んだ後か)
    releases: Vec<(u64, Reason, usize, bool)>,
    /// (時刻, 世代, 読んだ窓)
    refreshes: Vec<(u64, u64, usize)>,
}

const N_INV: usize = 17;
const INV_NAMES: [&str; N_INV] = [
    "loss",
    "dup",
    "order",
    "order_exempt",
    "modifier",
    "delay",
    "bound",
    "cap",
    "stuck",
    "content",
    "stale_release",
    "stale_start",
    "stale_other",
    "stale_regress",
    "fix",
    "route",
    "unmeasured",
];
/// 違反ではなく参考に数える列(ablation の検出には使わない)。
const INFO_INV: [&str; 4] = ["order_exempt", "stale_other", "stale_regress", "unmeasured"];

fn inv_index(name: &str) -> usize {
    INV_NAMES
        .iter()
        .position(|n| *n == name)
        .expect("不変条件の名前")
}

type Viol = [u32; N_INV];

struct Outcome {
    trace: Trace,
    viol: Viol,
}

struct Sim<'a, H: Hold> {
    hold: H,
    tables: &'a Tables,
    evs: &'a [Ev],
    wins: Vec<WinState>,
    agenda: Vec<(u64, u8, u64, Item)>,
    next_no: u64,
    now: u64,
    focus: usize,
    focus_hist: Vec<(u64, usize)>,
    focus_seq: u64,
    last_focus_at: Option<u64>,
    settled_seq: u64,
    context: usize,
    max_applied_seq: u64,
    ctx_regressed: bool,
    debounce_token: u64,
    queue: Vec<usize>,
    drain_pending: bool,
    timer_at: Option<u64>,
    held: Vec<usize>,
    hold_started: Option<u64>,
    should_pass: Vec<bool>,
    delivered_count: Vec<u32>,
    delivered_at: Vec<Option<u64>>,
    os_shift: Vec<u64>,
    refresh_rank: u8,
    trace: Trace,
    v: Viol,
}

impl<'a, H: Hold> Sim<'a, H> {
    fn push(&mut self, t: u64, rank: u8, item: Item) {
        self.agenda.push((t, rank, self.next_no, item));
        self.next_no += 1;
    }

    /// 次に処理する事象。`None` の中身はタイマー。
    fn pop_next(&mut self) -> Option<(u64, Option<Item>)> {
        let mut best: Option<(u64, u8, u64, usize)> = None;
        for (i, &(t, r, n, _)) in self.agenda.iter().enumerate() {
            if best.map_or(true, |b| (t, r, n) < (b.0, b.1, b.2)) {
                best = Some((t, r, n, i));
            }
        }
        let timer_first = match (best, self.timer_at) {
            (_, None) => false,
            (None, Some(_)) => true,
            (Some(b), Some(tm)) => (tm, RANK_TIMER) < (b.0, b.1),
        };
        if timer_first {
            let t = self.timer_at.take().expect("timer");
            return Some((t, None));
        }
        let b = best?;
        let (t, _, _, item) = self.agenda.swap_remove(b.3);
        Some((t, Some(item)))
    }

    fn run(mut self) -> Outcome {
        let mut steps = 0u32;
        while let Some((t, item)) = self.pop_next() {
            steps += 1;
            assert!(steps < 100_000, "閉ループが終わらない");
            assert!(t >= self.now, "仮想時計が戻った");
            self.now = t;
            match item {
                None => self.on_timer(),
                Some(Item::Key(id)) => self.on_phys(id),
                Some(Item::Focus { win, r }) => self.on_focus(win, r),
                Some(Item::Spawn { token, done_at }) => self.on_spawn(token, done_at),
                Some(Item::Done { seq, read }) => self.on_done(seq, read),
            }
            while self.drain_pending {
                self.drain();
            }
        }
        self.finalize()
    }

    fn on_focus(&mut self, win: usize, r: Option<u64>) {
        self.focus = win;
        self.focus_hist.push((self.now, win));
        self.focus_seq += 1;
        self.last_focus_at = Some(self.now);
        self.hold.on_focus_event(self.now, self.focus_seq);
        // debounce: フォーカス変更のたびにタイマーを張り直す(先の spawn は取り消される)。
        self.debounce_token += 1;
        if let Some(r) = r {
            assert!(r >= DEBOUNCE, "読み直しは debounce より後に完了する");
            let tk = self.debounce_token;
            let done_at = self.now + r;
            self.push(
                self.now + DEBOUNCE,
                RANK_SPAWN,
                Item::Spawn {
                    token: Some(tk),
                    done_at,
                },
            );
        }
    }

    fn on_spawn(&mut self, token: Option<u64>, done_at: u64) {
        if let Some(tk) = token {
            if tk != self.debounce_token {
                return;
            }
        }
        let seq = self.focus_seq;
        let read = self.focus;
        let at = done_at.max(self.now);
        let rank = self.refresh_rank;
        self.push(at, rank, Item::Done { seq, read });
    }

    fn on_done(&mut self, seq: u64, read: usize) {
        self.context = read;
        if seq < self.max_applied_seq {
            self.ctx_regressed = true;
        } else {
            self.max_applied_seq = seq;
            self.ctx_regressed = false;
        }
        if self.wins[read].readable {
            self.wins[read].belief = self.wins[read].truth;
        }
        if seq >= self.focus_seq {
            self.settled_seq = seq;
        }
        self.trace.refreshes.push((self.now, seq, read));
        if let Some(list) = self.hold.on_refresh_settled(seq) {
            let exp = self.held.clone();
            self.finish(list, exp, Reason::Refreshed);
        }
    }

    fn on_timer(&mut self) {
        if let Some(list) = self.hold.on_deadline() {
            let exp = self.held.clone();
            self.finish(list, exp, Reason::Deadline);
        }
    }

    /// 保持の外で既に OS に届いた(遅れずに届いた)Down の Up・リピートか。
    fn down_delivered_outside(&self, id: usize) -> bool {
        let ev = self.evs[id];
        let mut j = id;
        while j > 0 {
            j -= 1;
            let e = self.evs[j];
            if e.identity == ev.identity && e.down && !e.repeat {
                return self.delivered_at[j] == Some(e.t);
            }
        }
        false
    }

    fn on_phys(&mut self, id: usize) {
        let ev = self.evs[id];
        let holding = !self.held.is_empty();
        let armed_open = self
            .last_focus_at
            .map_or(false, |f| self.now.saturating_sub(f) < MAX)
            && self.settled_seq < self.focus_seq;
        let up_like = !ev.down || ev.repeat;
        // V1: 保持中の打鍵が押下時に使った修飾キーの Up は、保持した打鍵を追い越さない(素通しされるべき打鍵ではない)。
        let evs = self.evs;
        let modifier_used = up_like
            && ev.modifier_bit() != 0
            && self
                .held
                .iter()
                .any(|&i| evs[i].hold_key().mods_at_press & ev.modifier_bit() != 0);
        let outside_up = up_like && self.down_delivered_outside(id) && !modifier_used;
        self.should_pass[id] =
            outside_up || (!holding && !(ev.starts() && !ev.repeat && armed_open));
        if self.queue.is_empty() {
            self.deliver(id);
        } else {
            // `handle_hook_key_event`: defer キューに未処理があればフックの打鍵もキューへ(順序を保つ)。
            self.queue.push(id);
        }
    }

    fn deliver(&mut self, id: usize) {
        let ev = self.evs[id];
        match self.hold.on_key(self.now, ev.hold_key(), id) {
            Step::Pass => self.process(id),
            Step::Started => {
                self.timer_at = Some(self.now + MAX);
                self.hold_started = Some(self.now);
                self.held.push(id);
            }
            Step::Held => {
                self.held.push(id);
                let evs = self.evs;
                let downs = self
                    .held
                    .iter()
                    .filter(|&&i| evs[i].down && !evs[i].repeat)
                    .count();
                if downs > FOCUS_SWITCH_HOLD_CAP {
                    self.v[inv_index("cap")] += 1;
                }
            }
            Step::Release(list) => {
                let mut exp = self.held.clone();
                exp.push(id);
                let expired = self
                    .hold_started
                    .map_or(false, |s| self.now.saturating_sub(s) >= MAX);
                let reason = if expired { Reason::Expiry } else { Reason::Cap };
                self.finish(list, exp, reason);
            }
        }
    }

    fn finish(&mut self, list: Vec<usize>, expected: Vec<usize>, reason: Reason) {
        if list != expected {
            self.v[inv_index("content")] += 1;
        }
        self.timer_at = None;
        let fresh = self.context == self.focus;
        if reason == Reason::Refreshed && !fresh {
            self.v[inv_index("stale_release")] += 1;
        }
        self.trace
            .releases
            .push((self.now, reason, list.len(), fresh));
        self.held.clear();
        self.hold_started = None;
        self.queue.extend(list);
        self.drain_pending = true;
    }

    fn drain(&mut self) {
        self.drain_pending = false;
        let mut q = std::mem::take(&mut self.queue);
        let evs = self.evs;
        q.sort_by_key(|&i| evs[i].t);
        for id in q {
            self.deliver(id);
        }
    }

    fn process(&mut self, id: usize) {
        let ev = self.evs[id];
        self.delivered_count[id] += 1;
        if self.delivered_at[id].is_none() {
            self.delivered_at[id] = Some(self.now);
        }
        self.trace.deliveries.push(Delivery {
            id,
            at: self.now,
            focus: self.focus,
            context: self.context,
            regressed: self.ctx_regressed,
        });
        if ev.kind == Kind::Shift {
            if ev.down {
                if !self.os_shift.contains(&ev.identity) {
                    self.os_shift.push(ev.identity);
                }
            } else {
                self.os_shift.retain(|&i| i != ev.identity);
            }
        }
        if !ev.down || ev.repeat {
            return;
        }
        match ev.kind {
            Kind::Char => {
                if ev.shift != !self.os_shift.is_empty() {
                    self.v[inv_index("modifier")] += 1;
                }
            }
            Kind::ThumbR | Kind::ThumbL => {
                // エンジンが活性(文脈の窓の belief が開)なら親指キーとして消費され OS に届かない。
                if !self.wins[self.context].belief.open {
                    self.ime_key(if ev.kind == Kind::ThumbR { 0x1C } else { 0x1D });
                }
            }
            Kind::ConvKey => self.ime_key(0x1C),
            Kind::HanZen => self.ime_key(0xF3),
            _ => {}
        }
    }

    /// 素通しの IME キー: フォーカスの窓の真値が動き、予測は文脈の窓の belief に付く。
    fn ime_key(&mut self, vk: u16) {
        let tables = self.tables;
        let f = self.focus;
        let c = self.context;
        let q = QuirkSwitches::default();
        match tables
            .get(self.wins[f].grid)
            .step(self.wins[f].truth, ImeInput::Press(vk), q)
        {
            Transition::Next { after, .. } => self.wins[f].truth = after,
            Transition::Unmeasured(_) => self.v[inv_index("unmeasured")] += 1,
        }
        match tables
            .get(self.wins[c].grid)
            .step(self.wins[c].belief, ImeInput::Press(vk), q)
        {
            Transition::Next { after, .. } => self.wins[c].belief = after,
            Transition::Unmeasured(_) => self.v[inv_index("unmeasured")] += 1,
        }
    }

    fn focus_at(&self, t: u64) -> usize {
        let mut w = 0;
        for &(ft, fw) in &self.focus_hist {
            if ft <= t {
                w = fw;
            }
        }
        w
    }

    fn finalize(mut self) -> Outcome {
        let evs = self.evs;
        for id in 0..evs.len() {
            match self.delivered_count[id] {
                0 => self.v[inv_index("loss")] += 1,
                1 => {}
                _ => self.v[inv_index("dup")] += 1,
            }
        }
        if !self.held.is_empty() {
            self.v[inv_index("stuck")] += 1;
        }
        let ds = std::mem::take(&mut self.trace.deliveries);
        for (x, a) in ds.iter().enumerate() {
            let ka = (evs[a.id].t, a.id);
            for b in &ds[x + 1..] {
                if (evs[b.id].t, b.id) < ka {
                    let ea = evs[a.id];
                    let exempt = (!ea.down || ea.repeat) && a.at == ea.t;
                    if exempt {
                        self.v[inv_index("order_exempt")] += 1;
                    } else {
                        self.v[inv_index("order")] += 1;
                    }
                }
            }
        }
        for d in &ds {
            let ev = evs[d.id];
            let delay = d.at - ev.t;
            if self.should_pass[d.id] && delay > 0 {
                self.v[inv_index("delay")] += 1;
            }
            if delay > MAX {
                self.v[inv_index("bound")] += 1;
            }
            if d.context != d.focus && ev.down && !ev.repeat {
                if d.regressed {
                    self.v[inv_index("stale_regress")] += 1;
                } else if ev.starts() {
                    self.v[inv_index("stale_start")] += 1;
                } else if !ev.kind.is_modifier() {
                    self.v[inv_index("stale_other")] += 1;
                }
            }
            if self.focus_at(ev.t) != d.focus {
                self.v[inv_index("route")] += 1;
            }
        }
        self.trace.deliveries = ds;
        for w in &self.wins {
            if !w.readable && w.belief.open != w.truth.open {
                self.v[inv_index("fix")] += 1;
            }
        }
        Outcome {
            trace: self.trace,
            viol: self.v,
        }
    }
}

fn true_state(open: bool) -> TrueState {
    TrueState {
        open,
        conv: CONV_HIRAGANA,
        stage: TrueStage::None,
    }
}

fn simulate<H: Hold>(sc: &Scenario, hold: H, tables: &Tables) -> Outcome {
    let n = sc.evs.len();
    let mut sim = Sim {
        hold,
        tables,
        evs: &sc.evs,
        wins: sc
            .wins
            .iter()
            .map(|w| WinState {
                grid: w.grid,
                readable: w.readable,
                truth: true_state(w.open),
                belief: true_state(w.open),
            })
            .collect(),
        agenda: Vec::new(),
        next_no: 0,
        now: 0,
        focus: 0,
        focus_hist: vec![(0, 0)],
        focus_seq: 0,
        last_focus_at: None,
        settled_seq: 0,
        context: 0,
        max_applied_seq: 0,
        ctx_regressed: false,
        debounce_token: 0,
        queue: Vec::new(),
        drain_pending: false,
        timer_at: None,
        held: Vec::new(),
        hold_started: None,
        should_pass: vec![false; n],
        delivered_count: vec![0; n],
        delivered_at: vec![None; n],
        os_shift: Vec::new(),
        refresh_rank: if sc.refresh_first { 2 } else { 4 },
        trace: Trace::default(),
        v: [0; N_INV],
    };
    for (id, ev) in sc.evs.iter().enumerate() {
        sim.push(ev.t, RANK_KEY, Item::Key(id));
    }
    for act in &sc.acts {
        match *act {
            Act::Focus { t, win, r } => sim.push(t, RANK_FOCUS, Item::Focus { win, r }),
            Act::Stale { spawn, done } => sim.push(
                spawn,
                RANK_SPAWN,
                Item::Spawn {
                    token: None,
                    done_at: done,
                },
            ),
        }
    }
    sim.run()
}

// ---------------------------------------------------------------------------
// 全列挙 E1(タイミング)・E2(IME の組)・ランダムウォーク
// ---------------------------------------------------------------------------

const E1_WINS: [WinInit; 3] = [
    WinInit {
        grid: Grid::Atok,
        open: false,
        readable: true,
    },
    WinInit {
        grid: Grid::Atok,
        open: true,
        readable: false,
    },
    WinInit {
        grid: Grid::MsimeNative,
        open: true,
        readable: true,
    },
];

/// 読み直しの完了(フォーカス変更から): CI 実測の分位点(P50 67・P90 77・P95 81・P99 109・max 228)と、その間・
/// 250ms 前後・超過・取りこぼし。
const R_AXIS: [Option<u64>; 15] = [
    Some(55),
    Some(60),
    Some(67),
    Some(77),
    Some(81),
    Some(109),
    Some(150),
    Some(200),
    Some(228),
    Some(245),
    Some(250),
    Some(255),
    Some(300),
    Some(400),
    None,
];

const UP_AXIS: [u64; 3] = [25, 70, 160];

fn d_axis(step: u64) -> Vec<u64> {
    let mut v: Vec<u64> = (0..=300).step_by(step as usize).collect();
    for x in [245, 249, 251, 255] {
        if !v.contains(&x) {
            v.push(x);
        }
    }
    v.sort_unstable();
    v
}

fn e1_scenario(
    kind: FirstKind,
    d: u64,
    r: Option<u64>,
    up: u64,
    follow: Follow,
    refresh_first: bool,
    wins: [WinInit; 3],
) -> Scenario {
    let mut b = Builder::new();
    b.focus(F, 1, r);
    let t0 = F + d;
    match kind {
        FirstKind::Char => b.press(Kind::Char, t0, t0 + up, false),
        FirstKind::ThumbR => b.press(Kind::ThumbR, t0, t0 + up, false),
        FirstKind::ThumbL => b.press(Kind::ThumbL, t0, t0 + up, false),
        FirstKind::ConvKey => b.press(Kind::ConvKey, t0, t0 + up, false),
        FirstKind::HanZen => b.press(Kind::HanZen, t0, t0 + up, false),
        FirstKind::ShiftChar | FirstKind::CtrlChar => {
            let m = if kind == FirstKind::ShiftChar {
                Kind::Shift
            } else {
                Kind::Ctrl
            };
            let s = b.ident();
            b.ev(Ev::new(t0 - 15, m, s, true, false, false));
            b.press(Kind::Char, t0, t0 + up - 10, false);
            b.ev(Ev::new(t0 + up, m, s, false, false, false));
        }
        FirstKind::AltTab => {
            let a = b.ident();
            b.ev(Ev::new(t0 - 30, Kind::Alt, a, true, false, false));
            b.press(Kind::Tab, t0, t0 + 15, false);
            b.ev(Ev::new(t0 + up, Kind::Alt, a, false, false, false));
            // Alt の解放で次の窓(C)へ切り替わる。
            b.focus(t0 + up + 1, 2, r);
        }
        FirstKind::UpOnly => {
            let x = b.ident();
            b.ev(Ev::new(F - 5, Kind::Char, x, true, false, false));
            b.ev(Ev::new(t0, Kind::Char, x, false, false, false));
        }
        FirstKind::Repeat => {
            let x = b.ident();
            b.ev(Ev::new(F - 300, Kind::Char, x, true, false, false));
            for k in 0..3 {
                b.ev(Ev::new(t0 + 33 * k, Kind::Char, x, true, true, false));
            }
            b.ev(Ev::new(t0 + 66 + up, Kind::Char, x, false, false, false));
        }
        FirstKind::Injected => b.press(Kind::Char, t0, t0 + up, true),
    }
    let clamp = |done: u64| r.map_or(done, |r| done.min(F + r - 1));
    match follow {
        Follow::None => {}
        Follow::SecondChar => b.press(Kind::Char, t0 + 40, t0 + 40 + up, false),
        Follow::ThumbChord => b.press(Kind::ThumbR, t0 + 10, t0 + 10 + up, false),
        Follow::Burst12 => {
            for i in 0..12 {
                b.press(Kind::Char, t0 + 5 + 6 * i, t0 + 8 + 6 * i, false);
            }
        }
        Follow::Typing6 => {
            for i in 1..=6 {
                b.press(Kind::Char, t0 + 40 * i, t0 + 40 * i + 20, false);
            }
        }
        Follow::FocusC => b.focus(t0 + 20, 2, r),
        Follow::FocusBackA => b.focus(t0 + 20, 0, r),
        // 古い refresh が新しい refresh より後に完了する並び(文脈が古い窓へ戻る)は保持と無関係の別の問題なので、
        // 新しい refresh の完了より前に収める(`clamp`)。
        Follow::StaleBefore => b.stale(F - 10, clamp((t0.saturating_sub(5)).max(F + 1))),
        Follow::StaleAfter => b.stale(F - 10, clamp(t0 + 5)),
        Follow::InjBurst => {
            for i in 0..20 {
                b.press(Kind::Char, t0 + 5 + i, t0 + 30 + i, true);
            }
        }
    }
    b.build(
        Desc::E1 {
            kind,
            d,
            r,
            up,
            follow,
            refresh_first,
        },
        wins,
        refresh_first,
        RClass::of(r),
    )
}

/// E1 の組を 1 つずつ作って `f(キー種の番号, 追加の事象の番号, 組)` に渡す(全部を同時にメモリに置かない)。
fn for_each_e1(d_step: u64, rs: &[Option<u64>], mut f: impl FnMut(usize, usize, Scenario)) {
    for (ki, kind) in FIRST_KINDS.into_iter().enumerate() {
        for d in d_axis(d_step) {
            for &r in rs {
                for up in UP_AXIS {
                    for (fi, follow) in FOLLOWS.into_iter().enumerate() {
                        for refresh_first in [true, false] {
                            f(
                                ki,
                                fi,
                                e1_scenario(kind, d, r, up, follow, refresh_first, E1_WINS),
                            );
                        }
                    }
                }
            }
        }
    }
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// 読み直しの時間を CI 実測の分位点の区分一様で引く(250ms 超・取りこぼしの確率は仮定)。
fn sample_r(rng: &mut Rng) -> Option<u64> {
    let u = rng.below(1000);
    Some(match u {
        0..=499 => 55 + rng.below(13),
        500..=899 => 67 + rng.below(11),
        900..=949 => 77 + rng.below(5),
        950..=989 => 81 + rng.below(29),
        990..=996 => 109 + rng.below(120),
        997..=998 => 229 + rng.below(172),
        _ => return None,
    })
}

const WALK_GRIDS: [Grid; 2] = [Grid::Atok, Grid::MsimeNative];

fn walk_scenario(seed: u64) -> Scenario {
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut wins = [E1_WINS[0]; 3];
    for w in &mut wins {
        *w = WinInit {
            grid: WALK_GRIDS[rng.below(2) as usize],
            open: rng.below(2) == 0,
            readable: rng.below(2) == 0,
        };
    }
    let mut b = Builder::new();
    let mut t = F;
    let mut focus = 0usize;
    let mut worst = Some(0u64);
    for _ in 0..30 {
        t += if rng.below(3) == 0 {
            rng.below(15)
        } else {
            rng.below(120)
        };
        let a = rng.below(100);
        if a < 15 {
            let mut w = rng.below(3) as usize;
            if w == focus {
                w = (w + 1) % 3;
            }
            focus = w;
            let r = sample_r(&mut rng);
            worst = match (worst, r) {
                (None, _) | (_, None) => None,
                (Some(x), Some(y)) => Some(x.max(y)),
            };
            b.focus(t, w, r);
        } else if a < 20 {
            b.stale(t, t + 20 + rng.below(280));
        } else {
            let up = t + 20 + rng.below(180);
            let k = rng.below(90);
            match k {
                0..=39 => b.press(Kind::Char, t, up, false),
                40..=49 => b.press(Kind::ThumbR, t, up, false),
                50..=57 => b.press(Kind::ThumbL, t, up, false),
                58..=65 | 66..=69 => {
                    let m = if k <= 65 { Kind::Shift } else { Kind::Ctrl };
                    let s = b.ident();
                    b.ev(Ev::new(t, m, s, true, false, false));
                    b.press(Kind::Char, t + 10, t + 10 + rng.below(60), false);
                    b.ev(Ev::new(up + 10, m, s, false, false, false));
                }
                70..=73 => b.press(Kind::ConvKey, t, up, false),
                74..=76 => b.press(Kind::HanZen, t, up, false),
                77..=81 => b.press(Kind::Char, t, up, true),
                82..=84 => {
                    let al = b.ident();
                    b.ev(Ev::new(t, Kind::Alt, al, true, false, false));
                    b.press(Kind::Tab, t + 10, t + 20, false);
                    b.ev(Ev::new(t + 40, Kind::Alt, al, false, false, false));
                    let mut w = rng.below(3) as usize;
                    if w == focus {
                        w = (w + 1) % 3;
                    }
                    focus = w;
                    let r = sample_r(&mut rng);
                    worst = match (worst, r) {
                        (None, _) | (_, None) => None,
                        (Some(x), Some(y)) => Some(x.max(y)),
                    };
                    b.focus(t + 41, w, r);
                }
                _ => {
                    let x = b.ident();
                    b.ev(Ev::new(t, Kind::Char, x, true, false, false));
                    for i in 1..=3 {
                        b.ev(Ev::new(t + 30 * i, Kind::Char, x, true, true, false));
                    }
                    b.ev(Ev::new(t + 120, Kind::Char, x, false, false, false));
                }
            }
        }
    }
    let rclass = match worst {
        Some(0) => RClass::Measured,
        other => RClass::of(other),
    };
    b.build(Desc::Walk { seed }, wins, rng.below(2) == 0, rclass)
}

const N_WALKS: u64 = 3000;

// ---------------------------------------------------------------------------
// 出力
// ---------------------------------------------------------------------------

fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join(name)
}

/// golden と比べる(`UPDATE_GOLDEN=1` なら書き換える)。食い違ったら現状の全文を出す(CI のログから写せるように)。
fn check_golden(name: &str, actual: &str) {
    let path = golden_path(name);
    if std::env::var("UPDATE_GOLDEN").as_deref() == Ok("1") {
        std::fs::write(&path, actual).expect("golden を書けない");
        eprintln!("golden を更新しました: {}", path.display());
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    assert!(
        actual == expected,
        "golden {} が現状と一致しない。意図した変化なら UPDATE_GOLDEN=1 で再生成して差分を確認すること。\n\
         ---- 現状の全文 ----\n{actual}---- ここまで ----",
        path.display()
    );
}

fn rel(t: u64) -> String {
    let x = t as i64 - F as i64;
    if x >= 0 {
        format!("+{x}")
    } else {
        format!("{x}")
    }
}

/// 再現手順: 事象の並びと、保持での届き方(`hold` が `None` なら本物、`Some` なら防御を外した写し)。
fn timeline(sc: &Scenario, tables: &Tables, hold: Option<Ablate>) -> String {
    let out = match hold {
        None => simulate(sc, Real::new(), tables),
        Some(ab) => simulate(sc, Mirror::new(ab), tables),
    };
    let mut s = String::new();
    let _ = writeln!(
        s,
        "    窓: A={} B={} C={}(開始時のフォーカスは A)",
        sc.wins[0].label(),
        sc.wins[1].label(),
        sc.wins[2].label()
    );
    let mut lines: Vec<(u64, u8, String)> = Vec::new();
    for (id, ev) in sc.evs.iter().enumerate() {
        lines.push((ev.t, 3, format!("打鍵 #{id} {}", ev.label())));
    }
    for act in &sc.acts {
        match *act {
            Act::Focus { t, win, r } => lines.push((
                t,
                0,
                format!(
                    "フォーカス → {}(読み直しの完了 {})",
                    WIN_NAMES[win],
                    r.map_or_else(|| "なし".to_owned(), |r| rel(t + r))
                ),
            )),
            Act::Stale { spawn, done } => {
                lines.push((spawn, 1, format!("refresh の spawn(完了 {})", rel(done))))
            }
        }
    }
    for &(at, seq, read) in &out.trace.refreshes {
        lines.push((
            at,
            2,
            format!("refresh の完了 seq={seq} 読んだ窓={}", WIN_NAMES[read]),
        ));
    }
    for &(at, reason, n, fresh) in &out.trace.releases {
        lines.push((
            at,
            4,
            format!(
                "保持の解放 {reason:?} {n} 件{}",
                if fresh { "" } else { "(古い窓の文脈)" }
            ),
        ));
    }
    for d in &out.trace.deliveries {
        lines.push((
            d.at,
            5,
            format!(
                "  処理 #{} {} 文脈={} フォーカス={}",
                d.id,
                sc.evs[d.id].label(),
                WIN_NAMES[d.context],
                WIN_NAMES[d.focus]
            ),
        ));
    }
    lines.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
    for (t, _, l) in lines {
        let _ = writeln!(s, "    {:>6}ms {l}", rel(t));
    }
    let viol: Vec<String> = INV_NAMES
        .iter()
        .zip(out.viol.iter())
        .filter(|(_, c)| **c > 0)
        .map(|(n, c)| format!("{n}={c}"))
        .collect();
    let _ = writeln!(s, "    違反: {}", viol.join(" "));
    s
}

fn is_hard(k: usize) -> bool {
    !INFO_INV.contains(&INV_NAMES[k])
}

/// 最短の再現の選び方: 読み直しの区分(実測の範囲を優先)→ 事象の数 → 列挙の順。
type BestKey = (usize, usize, usize);

/// 不変条件 × 読み直しの区分の集計(違反のあった組の数)と、不変条件ごとの最短の再現。組は流しながら足す。
struct Acc {
    n: usize,
    per_class: [usize; 4],
    counts: [[usize; 4]; N_INV],
    best: Vec<Option<(BestKey, Scenario)>>,
}

impl Acc {
    fn new() -> Self {
        Self {
            n: 0,
            per_class: [0; 4],
            counts: [[0; 4]; N_INV],
            best: vec![None; N_INV],
        }
    }

    fn add(&mut self, sc: &Scenario, v: &Viol) {
        let c = sc.rclass as usize;
        self.per_class[c] += 1;
        for k in 0..N_INV {
            if v[k] > 0 {
                self.counts[k][c] += 1;
                let key = (sc.rclass as usize, sc.size(), self.n);
                let better = self.best[k].as_ref().map_or(true, |(bk, _)| key < *bk);
                if better {
                    self.best[k] = Some((key, sc.clone()));
                }
            }
        }
        self.n += 1;
    }

    fn write(&self, title: &str, tables: &Tables, hold: Option<Ablate>, s: &mut String) {
        let _ = writeln!(s, "## {title}");
        let _ = writeln!(
            s,
            "組の数: {}(内訳 {})",
            self.n,
            RClass::ALL
                .iter()
                .map(|c| format!("{}={}", c.name(), self.per_class[*c as usize]))
                .collect::<Vec<_>>()
                .join(" ")
        );
        let _ = writeln!(
            s,
            "不変条件 | {} (違反のあった組の数)",
            RClass::ALL.map(RClass::name).join(" | ")
        );
        for k in 0..N_INV {
            let c = self.counts[k];
            let _ = writeln!(
                s,
                "{}{} | {} | {} | {} | {}",
                INV_NAMES[k],
                if is_hard(k) { "" } else { "(参考)" },
                c[0],
                c[1],
                c[2],
                c[3]
            );
        }
        let _ = writeln!(
            s,
            "### 最短の再現(不変条件ごと、読み直しの区分が実測の範囲〈R≤228〉の組を優先)"
        );
        for k in 0..N_INV {
            if INV_NAMES[k] == "unmeasured" {
                continue;
            }
            if let Some((_, sc)) = &self.best[k] {
                let _ = writeln!(s, "- {} — {}", INV_NAMES[k], sc.desc);
                s.push_str(&timeline(sc, tables, hold));
            }
        }
        s.push('\n');
    }
}

/// 種類(行)× 不変条件(列)の表。読み直しが実測の範囲(R≤228)の組で違反のあった組の数。
const TABLE_COLS: [&str; 8] = [
    "modifier",
    "order",
    "stale_start",
    "stale_other",
    "fix",
    "route",
    "delay",
    "bound",
];

fn add_row(rows: &mut [[usize; 8]], i: usize, sc: &Scenario, v: &Viol) {
    if sc.rclass != RClass::Measured {
        return;
    }
    for (j, col) in TABLE_COLS.iter().enumerate() {
        if v[inv_index(col)] > 0 {
            rows[i][j] += 1;
        }
    }
}

fn write_rows(s: &mut String, head: &str, names: &[&str], rows: &[[usize; 8]]) {
    let _ = writeln!(s, "{head} | {}", TABLE_COLS.join(" | "));
    for (name, row) in names.iter().zip(rows) {
        let cells: Vec<String> = row.iter().map(ToString::to_string).collect();
        let _ = writeln!(s, "{name} | {}", cells.join(" | "));
    }
    s.push('\n');
}

// ---------------------------------------------------------------------------
// テスト
// ---------------------------------------------------------------------------

/// E1: タイミングの全列挙。本物の結果を golden に固定し、写し(既定)が本物と同じ軌跡を出すことを全件で確かめる。
#[test]
fn e1_timing_exhaustive_golden() {
    let started = Instant::now();
    let tables = Tables::load();
    let mut acc = Acc::new();
    let mut by_kind = vec![[0usize; 8]; FIRST_KINDS.len()];
    let mut by_follow = vec![[0usize; 8]; FOLLOWS.len()];
    for_each_e1(5, &R_AXIS, |ki, fi, sc| {
        let real = simulate(&sc, Real::new(), &tables);
        let mirror = simulate(&sc, Mirror::new(Ablate::default()), &tables);
        assert!(
            real.trace == mirror.trace && real.viol == mirror.viol,
            "写し(Mirror)が本物と違う軌跡を出した: {}",
            sc.desc
        );
        acc.add(&sc, &real.viol);
        add_row(&mut by_kind, ki, &sc, &real.viol);
        add_row(&mut by_follow, fi, &sc, &real.viol);
    });
    let mut s = String::from(
        "# PR #817(窓切替直後の保持)の閉ループ検証 E1: タイミングの全列挙。\n\
         # 自動生成: UPDATE_GOLDEN=1 cargo test -p awase-windows --test focus_switch_hold_closed_loop\n\
         # 軸: 最初の打鍵 11 種 × 遅延 0〜300ms(5ms 刻み+249/251) × 読み直しの完了 15 点 × 離し 3 × 追加の事象 10 × 同時刻の順 2\n\
         # 窓: A=GJI(ATOK)OFF/読める(開始時のフォーカス) → B=GJI(ATOK)ON/読めない(Windows Terminal 相当) / C=MS-IME ON/読める\n\
         # 読み直しの時間は CI 実測の分位点を入力として与えている。cfg(windows) の配線は写し(Sim)で、対象外。\n\
         # 写し(Mirror、防御を外さない)は全組で本物と同じ軌跡を出した(違えばこのテストが落ちる)。\n\n",
    );
    acc.write("E1 本物(#817)", &tables, None, &mut s);
    let _ = writeln!(
        s,
        "## E1 最初の打鍵の種類ごと(読み直し R≤228 の組で違反のあった組の数)"
    );
    let kind_names: Vec<&str> = FIRST_KINDS.iter().map(|k| k.name()).collect();
    write_rows(&mut s, "種類", &kind_names, &by_kind);
    let _ = writeln!(
        s,
        "## E1 追加の事象ごと(読み直し R≤228 の組で違反のあった組の数)"
    );
    let follow_names: Vec<&str> = FOLLOWS.iter().map(|f| f.name()).collect();
    write_rows(&mut s, "追加", &follow_names, &by_follow);
    eprintln!("E1: {} 組、{:?}", acc.n, started.elapsed());
    check_golden("focus_switch_hold_e1.txt", &s);
}

/// E2: IME の組(GJI/MS-IME × ON/OFF × 読める/読めない の窓 A→B)の全列挙。本物と保持なし(#809 の後)を並べる。
#[test]
fn e2_ime_pairs_exhaustive_golden() {
    let started = Instant::now();
    let tables = Tables::load();
    let mut combos = Vec::new();
    for grid in WALK_GRIDS {
        for open in [true, false] {
            for readable in [true, false] {
                combos.push(WinInit {
                    grid,
                    open,
                    readable,
                });
            }
        }
    }
    let kinds = [
        FirstKind::Char,
        FirstKind::ThumbR,
        FirstKind::ThumbL,
        FirstKind::ConvKey,
        FirstKind::HanZen,
    ];
    let rs = [Some(67), Some(109), Some(228), Some(300)];
    let fix = inv_index("fix");
    let ss = inv_index("stale_start");
    let no_hold = Ablate {
        no_hold: true,
        ..Ablate::default()
    };
    let mut real_acc = Acc::new();
    let mut base_acc = Acc::new();
    // [A][B][種類] → [本物の fix, 保持なしの fix, 本物の stale_start, 保持なしの stale_start](R≤228 の組の数)
    let mut cells = vec![[0usize; 4]; combos.len() * combos.len() * kinds.len()];
    for (ai, &a) in combos.iter().enumerate() {
        for (bi, &bw) in combos.iter().enumerate() {
            for (ki, &kind) in kinds.iter().enumerate() {
                for d in d_axis(10) {
                    for r in rs {
                        let mut sc =
                            e1_scenario(kind, d, r, 70, Follow::None, true, [a, bw, E1_WINS[2]]);
                        sc.desc = Desc::E2 {
                            a,
                            b: bw,
                            kind,
                            d,
                            r,
                        };
                        let real = simulate(&sc, Real::new(), &tables).viol;
                        let base = simulate(&sc, Mirror::new(no_hold), &tables).viol;
                        real_acc.add(&sc, &real);
                        base_acc.add(&sc, &base);
                        if sc.rclass == RClass::Measured {
                            let cell = &mut cells[(ai * combos.len() + bi) * kinds.len() + ki];
                            cell[0] += usize::from(real[fix] > 0);
                            cell[1] += usize::from(base[fix] > 0);
                            cell[2] += usize::from(real[ss] > 0);
                            cell[3] += usize::from(base[ss] > 0);
                        }
                    }
                }
            }
        }
    }
    let mut s = String::from(
        "# PR #817 の閉ループ検証 E2: IME の組。自動生成: UPDATE_GOLDEN=1 cargo test -p awase-windows --test focus_switch_hold_closed_loop\n\
         # 軸: 窓 A 8 種 × 窓 B 8 種(GJI〈ATOK の格子〉/MS-IME 本体 × ON/OFF × 読める/読めない) × 最初の打鍵 5 種 × 遅延 0〜300ms(10ms 刻み+245/249/251/255) × 読み直し 67/109/228/300ms\n\
         # 「固着」(fix)= 読めない窓で belief と真値の開閉が食い違ったまま終わる(ローマ字固着・直接入力の固着)。\n\n",
    );
    real_acc.write("E2 本物(#817)", &tables, None, &mut s);
    base_acc.write(
        "E2 保持なし(#809 の後・#817 の前)",
        &tables,
        Some(no_hold),
        &mut s,
    );
    let _ = writeln!(
        s,
        "## E2 窓の組 × 打鍵の種類ごとの固着(fix)と古い文脈(stale_start)の組の数(R≤228)。本物 / 保持なし。どちらも 0 の行は省く"
    );
    for (ai, a) in combos.iter().enumerate() {
        for (bi, bw) in combos.iter().enumerate() {
            for (ki, kind) in kinds.iter().enumerate() {
                let c = cells[(ai * combos.len() + bi) * kinds.len() + ki];
                if c.iter().sum::<usize>() > 0 {
                    let _ = writeln!(
                        s,
                        "A={} B={} {}: fix {} / {}, stale_start {} / {}",
                        a.label(),
                        bw.label(),
                        kind.name(),
                        c[0],
                        c[1],
                        c[2],
                        c[3]
                    );
                }
            }
        }
    }
    eprintln!("E2: {} 組、{:?}", real_acc.n, started.elapsed());
    check_golden("focus_switch_hold_e2.txt", &s);
}

/// ランダムウォーク: 窓 3 つの設定・フォーカス変更・打鍵(修飾・注入・Alt+Tab・リピート)・古い refresh をランダムに並べ、
/// 読み直しの時間は CI 実測の分布から引く。写し(既定)が本物と同じ軌跡を出すことも確かめる。
#[test]
fn random_walk_golden() {
    let started = Instant::now();
    let tables = Tables::load();
    let mut acc = Acc::new();
    for seed in 0..N_WALKS {
        let sc = walk_scenario(seed);
        let real = simulate(&sc, Real::new(), &tables);
        let mirror = simulate(&sc, Mirror::new(Ablate::default()), &tables);
        assert!(
            real.trace == mirror.trace && real.viol == mirror.viol,
            "写し(Mirror)が本物と違う軌跡を出した: {}",
            sc.desc
        );
        acc.add(&sc, &real.viol);
    }
    let mut s = String::from(
        "# PR #817 の閉ループ検証: ランダムウォーク(3000 本 × 30 手、seed 0..3000)。\n\
         # 自動生成: UPDATE_GOLDEN=1 cargo test -p awase-windows --test focus_switch_hold_closed_loop\n\
         # 読み直しの時間: CI 実測の分位点(P50 67・P90 77・P95 81・P99 109・max 228)の区分一様。250ms 超 0.2%・取りこぼし 0.1% は仮定。\n\
         # 区分は、その walk の中で最も遅い読み直し。写し(Mirror)は全本で本物と同じ軌跡を出した。\n\n",
    );
    acc.write("ランダムウォーク 本物(#817)", &tables, None, &mut s);
    eprintln!("random walk: {} 本、{:?}", acc.n, started.elapsed());
    check_golden("focus_switch_hold_walk.txt", &s);
}

/// ablation: 防御を 1 つずつ外した写しで、全列挙(粗い刻み)とランダムウォークの違反が本物より増えるかを数える。
#[test]
fn ablation_detects_each_defense_golden() {
    let started = Instant::now();
    let tables = Tables::load();
    let rs = [Some(67), Some(109), Some(228), Some(255), Some(400), None];
    let abls = ablations();
    let mut inc = vec![[0usize; N_INV]; abls.len()];
    let mut dec = vec![[0usize; N_INV]; abls.len()];
    let mut first: Vec<Option<(BestKey, Scenario, Viol, Viol)>> = vec![None; abls.len()];
    let mut n = 0usize;
    let mut one = |sc: Scenario| {
        let real = simulate(&sc, Real::new(), &tables).viol;
        for (ai, (_, _, ab)) in abls.iter().enumerate() {
            let v = simulate(&sc, Mirror::new(*ab), &tables).viol;
            let mut any_hard = false;
            for k in 0..N_INV {
                if v[k] > real[k] {
                    inc[ai][k] += 1;
                    if is_hard(k) {
                        any_hard = true;
                    }
                } else if v[k] < real[k] {
                    dec[ai][k] += 1;
                }
            }
            if any_hard {
                let key = (sc.rclass as usize, sc.size(), n);
                let better = first[ai].as_ref().map_or(true, |(bk, ..)| key < *bk);
                if better {
                    first[ai] = Some((key, sc.clone(), real, v));
                }
            }
        }
        n += 1;
    };
    for_each_e1(20, &rs, |_, _, sc| one(sc));
    for seed in 0..N_WALKS {
        one(walk_scenario(seed));
    }
    let mut s = String::from(
        "# PR #817 の閉ループ検証: 防御の ablation。\n\
         # 自動生成: UPDATE_GOLDEN=1 cargo test -p awase-windows --test focus_switch_hold_closed_loop\n\
         # 事象の集合: E1 を粗くした全列挙(遅延 20ms 刻み+245/249/251/255・読み直し 67/109/228/255/400/取りこぼし)+ランダムウォーク 3000 本。\n\
         # 「増」= 外した版で本物より違反が増えた組の数、「減」= 減った組の数。検出 = 違反(参考の列を除く)が増えた組が 1 つ以上ある。\n\n",
    );
    let _ = writeln!(s, "組の数: {n}");
    let mut detected = 0;
    for (ai, (id, what, _)) in abls.iter().enumerate() {
        let fmt = |arr: &[usize; N_INV]| {
            let v: Vec<String> = (0..N_INV)
                .filter(|&k| arr[k] > 0)
                .map(|k| {
                    format!(
                        "{}{}={}",
                        INV_NAMES[k],
                        if is_hard(k) { "" } else { "(参考)" },
                        arr[k]
                    )
                })
                .collect();
            if v.is_empty() {
                "なし".to_owned()
            } else {
                v.join(" ")
            }
        };
        let is_det = first[ai].is_some();
        if is_det {
            detected += 1;
        }
        let _ = writeln!(
            s,
            "## {id}({what}を外す): {}",
            if is_det {
                "検出"
            } else {
                "検出できない"
            }
        );
        let _ = writeln!(s, "増: {}", fmt(&inc[ai]));
        let _ = writeln!(s, "減: {}", fmt(&dec[ai]));
        if let Some((_, sc, real, v)) = &first[ai] {
            let viol: Vec<String> = (0..N_INV)
                .filter(|&k| v[k] > real[k] && is_hard(k))
                .map(|k| format!("{}={}→{}", INV_NAMES[k], real[k], v[k]))
                .collect();
            let _ = writeln!(s, "最短の例: {} ({})", sc.desc, viol.join(" "));
        }
        s.push('\n');
    }
    let _ = writeln!(
        s,
        "検出率: {detected}/{}(基準の no-hold を含む)",
        abls.len()
    );
    eprintln!(
        "ablation: {n} 組 × {} 版、{:?}",
        abls.len(),
        started.elapsed()
    );
    check_golden("focus_switch_hold_ablation.txt", &s);
}
