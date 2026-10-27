#![allow(clippy::all, clippy::pedantic, clippy::nursery)]
//! 「IME の真の状態」と「awase の Engine の活性」が食い違わないかを、擬似 IME・擬似窓の上で調べる基準の測定
//! （オラクル）。所有者の方針の定式化: **IME ON かつ ひらがなのときは、必ず awase（NICOLA の Engine）が ON**。
//!
//! 実装を変えずに、いまの実装がどんな事象の列でこの不変条件を破るかを数え、最小の列に仕分けて golden
//! （`tests/golden/belief_engine_invariant_violations.txt`）に固定する。違反が減ったら golden を更新する
//! （ゼロベースの設計がこのテストを違反ゼロで通すことが目標）。違反が増えたら落ちる。
//! 背景・結果の読み方は `docs/tasks/belief-engine-invariant-baseline-2026-10-26.md`。
//!
//! # 不変条件
//!
//! 真の状態 = 擬似 IME の `TrueState`（いまフォーカスのあるスレッドの IME。窓モデルは下記）。
//! Engine の活性 = ハーネスの `StepRecord::engine_active`（本物の `Engine::compute_active` に、本物の `ImeStateHub::effective_open_at`・
//! `input_mode`・`is_japanese_ime` から組んだ文脈を渡した値。`user_enabled` はハーネスでは常に真）。
//!
//! - **I1（必須）**: 真が「開 かつ ひらがな（conv が NATIVE ビットと ROMAN ビットを持つ。ひらがな 0x19・カタカナ 0x1B 等）」
//!   なら、Engine は Active でなければならない。
//! - **I2（参考。逆向き）**: 真が「閉」または「開×半角英数（NATIVE ビットなし）」なら、Engine は Inactive でなければならない
//!   （誤って ON だと生の英字が NICOLA の文字になる）。
//!
//! 検査は各事象の直後（`settle` の後）の状態に対して行う。起動直後（`start`）は検査しない。
//!
//! # 時間の許容（T）
//!
//! T = 0: 事象の直後に食い違っていれば違反。T = 2000ms: 最初に食い違った事象の後、**事象が何も来ないまま 2000ms 経っても**
//! 食い違いが残る（`Harness::advance_ms(2000)` の後も違反のまま）ものだけを数える（実機で観測された「次の観測が来るまで」の許容）。
//! 時間の経過そのもの（観測の鮮度切れ、`OBSERVATION_FRESH_WINDOW_MS`=3000ms）は字母の `Advance`（3500ms）で入る。
//!
//! # 窓モデル
//!
//! `shared`: 全ての窓が同じスレッドで、IME の状態は窓をまたいで共有される（実機の事故〈BUG-223〉の記述に合う。窓切替の後も
//! IME は ON のまま）。`per-thread`: 窓ごとに別スレッドで、初めて行く窓の IME は閉×ひらがな（ADR-191 §3.2、ハーネスの既定）。
//!
//! 再生成: `UPDATE_GOLDEN=1 cargo test -p awase-windows --test belief_engine_invariant`

mod support;

use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::Instant;

use awase_windows::focus::class_names::AppImeProfile;
use awase_windows::state::ime_event::HwndId;
use support::harness::{Harness, Setup, Source, StepRecord, Window, ALT_NAMES};
use support::pseudo_ime::{grid_key_name, Grid, TrueStage, TrueState, CONV_HIRAGANA};

// ---------------------------------------------------------------------------
// 規模（CI の時間の予算に合わせて調整する。表の本体は golden）
// ---------------------------------------------------------------------------

/// 全字母の全列挙の長さ。
const FULL_DEPTH: usize = 3;
/// 小さい字母（実機の事故の列に要る事象だけ）の全列挙の長さ。
const TINY_DEPTH: usize = 4;
/// I2（参考）の全列挙の長さ（核の字母）。
const I2_DEPTH: usize = 3;
/// I6(時間の経過だけで belief が変わる)の列の長さ(時間の予算。列ごとに 2 回時間を進める)。
const I6_DEPTH: usize = 2;
/// ランダム探索の本数（設定ごと）と長さ。
const RANDOM_COUNT: usize = 300;
const RANDOM_MIN_LEN: usize = 8;
const RANDOM_MAX_LEN: usize = 12;
/// ランダム探索で最小化して調べる違反の数（設定ごと。決定的に先頭から）。
const RANDOM_MINIMIZE: usize = 20;
/// golden に載せる例の数（分類ごと）。
const EXAMPLES_PER_CLASS: usize = 2;
/// golden に最小列を全部載せる上限（設定ごと）。超えたら先頭だけ。
const LIST_LIMIT: usize = 60;
/// 1 設定あたりの時間の上限（秒）。超えたら、どの段で何本まで進んだかを添えて panic する（規模を CI で調整するための安全弁）。
const BUDGET_SECS: u64 = 240;

fn check_budget(cfg: Config, started: Instant, phase: &str, progress: usize) {
    assert!(
        started.elapsed().as_secs() < BUDGET_SECS,
        "[budget] {} が {BUDGET_SECS} 秒を超えた: 段={phase} 進んだ数={progress} 経過={:?}",
        cfg.name(),
        started.elapsed()
    );
}

// ---------------------------------------------------------------------------
// 窓
// ---------------------------------------------------------------------------

const W_EXPLORER: usize = 0;
const W_XAML: usize = 1;
const W_WEZTERM: usize = 2;
const W_CHROME: usize = 3;
#[allow(dead_code)]
const W_RELAY: usize = 4;
const WINDOW_NAMES: [&str; 5] = ["explorer", "xaml", "wezterm", "chrome", "relay"];

/// 窓の表。`shared` なら全部同じスレッド（IME の状態を共有）。pid は ADR-277 の表に合わせ、explorer と xaml だけが同じプロセス。
fn windows(shared: bool) -> [Window; 5] {
    let t = |n: u32| if shared { 1 } else { n };
    [
        Window {
            name: "explorer",
            hwnd: HwndId(0x1001),
            pid: 10,
            thread: t(1),
            class: "CabinetWClass",
            process: "explorer.exe",
            input_relay: false,
            learned_unavailable: false,
        },
        Window {
            name: "xaml",
            hwnd: HwndId(0x1002),
            pid: 10,
            thread: t(2),
            class: "XamlExplorerHostIslandWindow",
            process: "explorer.exe",
            input_relay: false,
            learned_unavailable: false,
        },
        Window {
            name: "wezterm",
            hwnd: HwndId(0x2001),
            pid: 20,
            thread: t(3),
            class: "org.wezfurlong.wezterm",
            process: "wezterm-gui.exe",
            input_relay: false,
            learned_unavailable: false,
        },
        Window {
            name: "chrome",
            hwnd: HwndId(0x5001),
            pid: 50,
            thread: t(6),
            class: "Chrome_WidgetWin_1",
            process: "chrome.exe",
            input_relay: false,
            learned_unavailable: false,
        },
        Window {
            name: "relay",
            hwnd: HwndId(0x4001),
            pid: 40,
            thread: t(5),
            class: "RelayMainWindow",
            process: "relay.exe",
            input_relay: true,
            learned_unavailable: false,
        },
    ]
}

fn profile_name(w: &Window) -> &'static str {
    match w.live_profile() {
        AppImeProfile::Standard => "std",
        AppImeProfile::Imm32Unavailable => "imm32-unavailable",
        AppImeProfile::TsfNative => "tsf-native",
        AppImeProfile::InputRelay => "relay",
    }
}

// ---------------------------------------------------------------------------
// 事象の字母
// ---------------------------------------------------------------------------

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Obs {
    /// 読める窓の ObserverPoll（Medium）で真の開閉を読む。
    PollTruth,
    /// 読める窓の ImmCrossProbe（High）で真の開閉を読む。
    ImmTruth,
    /// FocusProbe（Low、BeliefOnly）で真の開閉を読む（どの窓でも）。
    ProbeTruth,
    /// 読めない窓の `ConvInference`（Medium）が「開」と**推測する**（GJI × TsfNative は IME を閉じても conv の NATIVE が残るので、
    /// 閉のときは嘘になる）。
    ConvInferOpen,
    /// 読める窓の ObserverPoll が真と逆の開閉を返す（古い読み・誤読）。
    PollLie,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Ev {
    /// 物理キー（awase が消費せず IME へ通した、修飾なしの KeyDown）。
    Key(u16),
    /// 利用者の明示の IME ON/OFF（`user_set_open`。`ExplicitIntent` を記録して書く）。
    UserSet(bool),
    /// 窓切替（窓の表の添字）。
    Focus(usize),
    Obs(Obs),
    /// 外から IME の開閉が変わる（言語バー・他アプリ。awase は見ていない）。
    External(bool),
    /// 3500ms 経過（観測の鮮度切れ 3000ms を越える）。
    Advance,
    /// いまの窓が実測学習で降格される。
    Demote,
}

const VK_EISU: u16 = 0xF0;
const VK_HIRAGANA: u16 = 0xF2;
const VK_IME_OFF: u16 = 0x1A;
const VK_IME_ON: u16 = 0x16;
const VK_HANZEN: u16 = 0xF3;
#[allow(dead_code)]
const VK_CHAR: u16 = 0x41;
/// 列の前の準備のステップ数（`start` と、最初の観測）。検査は列の事象の後だけ。
const SETTLE_STEPS: usize = 2;
const ADVANCE_MS: u64 = 3500;
const PERSIST_MS: u64 = 2000;

impl Ev {
    fn name(self) -> String {
        match self {
            Self::Key(vk) => format!("key:{}", grid_key_name(vk).unwrap_or("char")),
            Self::UserSet(b) => format!("user-{}", if b { "on" } else { "off" }),
            Self::Focus(i) => format!("focus:{}", WINDOW_NAMES[i]),
            Self::Obs(o) => match o {
                Obs::PollTruth => "obs:poll".into(),
                Obs::ImmTruth => "obs:imm-cross".into(),
                Obs::ProbeTruth => "obs:focus-probe".into(),
                Obs::ConvInferOpen => "obs:conv-infer-open(lie-if-closed)".into(),
                Obs::PollLie => "obs:poll-LIE".into(),
            },
            Self::External(b) => format!("external-{}", if b { "on" } else { "off" }),
            Self::Advance => format!("advance:{ADVANCE_MS}ms"),
            Self::Demote => "demote-by-learning".into(),
        }
    }

    /// 分類に使う、事象の種類（引数を落とした名前）。
    fn kind(self) -> &'static str {
        match self {
            Self::Key(VK_HIRAGANA) => "key:hiragana(F2)",
            Self::Key(VK_EISU) => "key:eisu(F0)",
            Self::Key(VK_IME_OFF) => "key:ime-off",
            Self::Key(VK_IME_ON) => "key:ime-on",
            Self::Key(VK_HANZEN) => "key:hankaku-zenkaku",
            Self::Key(_) => "key:char",
            Self::UserSet(true) => "user-on",
            Self::UserSet(false) => "user-off",
            Self::Focus(_) => "focus",
            Self::Obs(Obs::PollTruth) => "obs:poll",
            Self::Obs(Obs::ImmTruth) => "obs:imm-cross",
            Self::Obs(Obs::ProbeTruth) => "obs:focus-probe",
            Self::Obs(Obs::ConvInferOpen) => "obs:conv-infer-open",
            Self::Obs(Obs::PollLie) => "obs:poll-LIE",
            Self::External(true) => "external-on",
            Self::External(false) => "external-off",
            Self::Advance => "advance",
            Self::Demote => "demote",
        }
    }

    /// 嘘・誤読を流す事象か（偽陽性の疑いの仕分け用）。`ConvInferOpen` は閉のときだけ嘘だが、保守的に嘘の側に数える。
    fn is_lie(self) -> bool {
        matches!(self, Self::Obs(Obs::PollLie | Obs::ConvInferOpen))
    }
}

fn seq_name(seq: &[Ev]) -> String {
    seq.iter().map(|e| e.name()).collect::<Vec<_>>().join(" > ")
}

/// 全字母（19 種）。文字キー・明示 ON・ImmCross の観測（Poll と同じ性質）は時間の予算のため外した。
fn full_alphabet() -> Vec<Ev> {
    let mut v = vec![
        Ev::Key(VK_EISU),
        Ev::Key(VK_HIRAGANA),
        Ev::Key(VK_IME_OFF),
        Ev::Key(VK_IME_ON),
        Ev::Key(VK_HANZEN),
        Ev::UserSet(false),
    ];
    v.extend((0..5).map(Ev::Focus));
    v.extend([
        Ev::Obs(Obs::PollTruth),
        Ev::Obs(Obs::ProbeTruth),
        Ev::Obs(Obs::ConvInferOpen),
        Ev::Obs(Obs::PollLie),
        Ev::External(false),
        Ev::External(true),
        Ev::Advance,
        Ev::Demote,
    ]);
    v
}

/// 核の字母（12 種。実機の事故の列に要る事象: OFF キー・F0・F2・明示 OFF・窓切替〈読める窓・TSF・読めない窓〉・観測・時間）。
fn core_alphabet() -> Vec<Ev> {
    vec![
        Ev::Key(VK_EISU),
        Ev::Key(VK_HIRAGANA),
        Ev::Key(VK_IME_OFF),
        Ev::UserSet(false),
        Ev::Focus(W_EXPLORER),
        Ev::Focus(W_XAML),
        Ev::Focus(W_WEZTERM),
        Ev::Focus(W_CHROME),
        Ev::Obs(Obs::PollTruth),
        Ev::Obs(Obs::ProbeTruth),
        Ev::Obs(Obs::ConvInferOpen),
        Ev::Advance,
    ]
}

/// 小さい字母（7 種。実機の事故の列〈OFF キー・F2・明示 OFF・窓切替・時間〉だけ）。長さ 4 まで全列挙する。
fn tiny_alphabet() -> Vec<Ev> {
    vec![
        Ev::Key(VK_HIRAGANA),
        Ev::Key(VK_IME_OFF),
        Ev::UserSet(false),
        Ev::Focus(W_XAML),
        Ev::Focus(W_WEZTERM),
        Ev::Focus(W_CHROME),
        Ev::Advance,
    ]
}

/// 事象を流す。意味の無い事象（いまの窓へのフォーカス・読めない窓の IMM 観測・実測の無いキー・変化しない外部操作・降格しない窓の降格）は
/// 何もせず `false`。副作用は `true` を返すときだけ。
fn apply(h: &mut Harness, wins: &[Window; 5], ev: Ev) -> bool {
    let w = h.window().expect("窓モデル");
    match ev {
        Ev::Key(vk) => {
            if !h.ime.knows(vk) {
                return false;
            }
            h.key(vk);
        }
        Ev::UserSet(open) => {
            h.user_set_open(open);
        }
        Ev::Focus(i) => {
            if wins[i].hwnd == w.hwnd {
                return false;
            }
            h.focus_to(wins[i]);
        }
        Ev::Obs(o) => match o {
            Obs::PollTruth | Obs::ImmTruth | Obs::PollLie => {
                if !w.readable() {
                    return false;
                }
                let truth = h.ime.read_state().open;
                match o {
                    Obs::PollTruth => h.observe_value(Source::Poll, truth),
                    Obs::ImmTruth => h.observe_value(Source::ImmCross, truth),
                    _ => h.observe_value(Source::Poll, !truth),
                };
            }
            Obs::ProbeTruth => {
                h.observe(Source::FocusProbe);
            }
            Obs::ConvInferOpen => {
                if w.readable() {
                    return false;
                }
                h.observe_value(Source::ConvInference, true);
            }
        },
        Ev::External(open) => {
            if h.ime.state().open == open {
                return false;
            }
            h.external_set_open(open);
        }
        Ev::Advance => {
            h.advance_ms(ADVANCE_MS);
        }
        Ev::Demote => {
            if w.learned_unavailable || w.live_profile() != AppImeProfile::Standard {
                return false;
            }
            h.demote_by_learning();
        }
    }
    true
}

// ---------------------------------------------------------------------------
// 不変条件
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Inv {
    I1,
    I2,
    /// ADR-282 V0: 同じ不変条件を候補 Z(`ScopeBelief`)の Engine の活性で見たもの。
    /// ADR-282 V0: 同じ不変条件を比較用のモデル(`ALT_NAMES` の添字: Z・Z+U2・S′・S′+U2・配線Z+U2)の Engine の活性で見たもの。
    I1Alt(usize),
    I2Alt(usize),
}

impl Inv {
    const fn name(self) -> &'static str {
        match self {
            Self::I1 => "I1",
            Self::I2 => "I2",
            Self::I1Alt(0) => "I1(Z)",
            Self::I1Alt(1) => "I1(Z+U2)",
            Self::I1Alt(2) => "I1(S′)",
            Self::I1Alt(3) => "I1(S′+U2)",
            Self::I1Alt(_) => "I1(配線Z+U2)",
            Self::I2Alt(0) => "I2(Z)",
            Self::I2Alt(1) => "I2(Z+U2)",
            Self::I2Alt(2) => "I2(S′)",
            Self::I2Alt(3) => "I2(S′+U2)",
            Self::I2Alt(_) => "I2(配線Z+U2)",
        }
    }
}

/// 真が「開 かつ ひらがな（NATIVE と ROMAN）」か。
fn truth_is_hiragana_on(t: TrueState) -> bool {
    t.open && t.conv & 0x01 != 0 && t.conv & 0x10 != 0
}

/// 真が「閉」または「開×半角英数（NATIVE なし）」か。戻り値は分類名。
fn truth_is_off_or_alnum(t: TrueState) -> Option<&'static str> {
    if !t.open {
        Some("closed")
    } else if !t.is_native() {
        Some("alnum")
    } else {
        None
    }
}

/// 状態 `s`（起動直後ではない）が `inv` に違反するなら、分類名。
fn violation(inv: Inv, s: &StepRecord) -> Option<String> {
    match inv {
        Inv::I1 => {
            if truth_is_hiragana_on(s.truth) && !s.engine_active {
                Some(if s.effective_open {
                    format!("belief-on-but-engine-off/mode={:?}", s.input_mode)
                } else {
                    format!("belief-off/{}", s.basis)
                })
            } else {
                None
            }
        }
        Inv::I2 => {
            let kind = truth_is_off_or_alnum(s.truth)?;
            s.engine_active
                .then(|| format!("truth-{kind}-but-engine-on/{}", s.basis))
        }
        Inv::I1Alt(k) => {
            let a = s.alts.get(k)?;
            (truth_is_hiragana_on(s.truth) && !a.engine_active).then(|| {
                if a.open {
                    "alt-belief-on-but-engine-off".to_string()
                } else {
                    format!("alt-belief-off/{}", a.basis)
                }
            })
        }
        Inv::I2Alt(k) => {
            let kind = truth_is_off_or_alnum(s.truth)?;
            let a = s.alts.get(k)?;
            a.engine_active
                .then(|| format!("truth-{kind}-but-engine-on/{}", a.basis))
        }
    }
}

// ---------------------------------------------------------------------------
// 実行
// ---------------------------------------------------------------------------

/// 起動時の IME の真値（閉×ひらがな。起動時の belief＝閉と一致する）。
const INITIAL: TrueState = TrueState {
    open: false,
    conv: CONV_HIRAGANA,
    stage: TrueStage::None,
};

#[derive(Debug, Clone, Copy)]
struct Config {
    grid: Grid,
    shared: bool,
    /// ADR-282 V0: 窓の種類で保持単位を混ぜた窓モデル(`shared` は偽で窓ごとに別スレッド)。`Some(true)` なら tsf-native の窓
    /// (wezterm)が入場時に直前のスレッドの「開」を引き継ぐ(O5: MS-IME 本体の Win32 EDIT → Windows Terminal の向き、CI 6/6 の写し。
    /// 測ったのは Windows Terminal で、ここでは同じ tsf-native の wezterm の窓に当てはめる近似)。`Some(false)` は引き継がない
    /// (O6 の既定〈独立〉。GJI × tsf-native は未確定なので両方で回す)。`None` は従来の shared / per-thread。
    tsf_inherit: Option<bool>,
}

impl Config {
    const fn plain(grid: Grid, shared: bool) -> Self {
        Self {
            grid,
            shared,
            tsf_inherit: None,
        }
    }

    const fn mixed(grid: Grid, tsf_inherit: bool) -> Self {
        Self {
            grid,
            shared: false,
            tsf_inherit: Some(tsf_inherit),
        }
    }

    fn name(self) -> String {
        match self.tsf_inherit {
            Some(i) => format!(
                "{} mixed(tsf-native={})",
                self.grid.name(),
                if i { "inherit-open" } else { "independent" }
            ),
            None => format!(
                "{} {}",
                self.grid.name(),
                if self.shared { "shared" } else { "per-thread" }
            ),
        }
    }

    fn start(self) -> (Harness, [Window; 5]) {
        let wins = windows(self.shared);
        let mut h =
            Harness::start(Setup::in_window(INITIAL, wins[W_EXPLORER]).with_grid(self.grid));
        if self.tsf_inherit == Some(true) {
            h.inherit_open_on_entry(wins[W_WEZTERM].thread);
        }
        // ADR-282 V0: 候補 Z を並走させる(本物の belief・Engine には効かせない。違反は I1(Z)・I2(Z) で別に数える)。
        h.with_scope_belief();
        // 起動直後の belief は placeholder（`desired_open=true`）で、真（閉）と食い違っている（BUG-163 の領分。起動の事象は
        // ここでは測らない）。最初の観測で揃えた「動いている awase」の状態から列を始める。
        h.observe(Source::ImmCross);
        (h, wins)
    }
}

/// 列を最初から流す。意味の無い事象を含めば `None`。
fn replay(cfg: Config, seq: &[Ev]) -> Option<(Harness, [Window; 5])> {
    let (mut h, wins) = cfg.start();
    for &e in seq {
        if !apply(&mut h, &wins, e) {
            return None;
        }
    }
    Some((h, wins))
}

/// 窓の種類（violation の起きた時点でフォーカスのある窓）。
fn window_label(h: &Harness) -> String {
    let w = h.window().expect("窓モデル");
    format!("{}({})", w.name, profile_name(&w))
}

#[derive(Debug, Clone)]
struct Raw {
    seq: Vec<Ev>,
    class: String,
    window: String,
    /// 事象が来ないまま 2000ms 経っても違反のままか。
    persists: bool,
    /// 2000ms 経つ間に、読める窓なら周期の ObserverPoll（真の開閉）が 1 回届いたとしても違反のままか（読めない窓は届かない）。
    persists_poll: bool,
    /// 列の性質（嘘の観測を含む／awase に見えない外部変更を含む／どちらも含まない）。
    tag: &'static str,
}

/// 列の性質。外部変更（awase は見ていない）は観測が無ければ原理的に直せないので、awase 自身の操作・窓切替・正しい観測だけで
/// 起きる違反（`TAG_CLEAN`）と分ける。嘘の観測を含む違反は、字母の仮定（嘘が実機で起きるか）の偽陽性の疑いを持つ。
const TAG_LIE: &str = "嘘の観測を含む";
const TAG_EXTERNAL: &str = "外部変更を含む";
const TAG_CLEAN: &str = "awase の操作・窓切替・正しい観測だけ";

fn tag_of(seq: &[Ev]) -> &'static str {
    if seq.iter().any(|e| e.is_lie()) {
        TAG_LIE
    } else if seq.iter().any(|e| matches!(e, Ev::External(_))) {
        TAG_EXTERNAL
    } else {
        TAG_CLEAN
    }
}

/// 違反した直後の `h` を 2000ms 進め、(事象なしで残るか, 読める窓なら周期の poll が 1 回届いても残るか) を返す。
fn persistence(h: &mut Harness, inv: Inv) -> (bool, bool) {
    h.advance_ms(PERSIST_MS);
    let idle = violation(inv, h.steps.last().expect("step")).is_some();
    let w = h.window().expect("窓モデル");
    if !w.readable() {
        return (idle, idle);
    }
    let truth = h.ime.read_state().open;
    h.observe_value(Source::Poll, truth);
    let poll = violation(inv, h.steps.last().expect("step")).is_some();
    (idle, poll)
}

struct Exhaustive {
    checked: usize,
    raws: Vec<Raw>,
}

/// 幅優先に、`alphabet` の意味のある列を長さ `depth` まで全部流し、`inv` の違反を**最初の違反で打ち切って**集める
/// （違反した列は延ばさない＝集めた列は「最後の事象で初めて違反する列」。短い順・字母の順で決定的）。
fn exhaustive(
    cfg: Config,
    inv: Inv,
    alphabet: &[Ev],
    depth: usize,
    started: Instant,
) -> Exhaustive {
    let mut frontier: Vec<Vec<Ev>> = vec![Vec::new()];
    let mut raws = Vec::new();
    let mut checked = 0;
    for _ in 0..depth {
        let mut next = Vec::new();
        for seq in &frontier {
            for &e in alphabet {
                let mut cand = seq.clone();
                cand.push(e);
                let Some((mut h, _)) = replay(cfg, &cand) else {
                    continue;
                };
                checked += 1;
                if checked % 200 == 0 {
                    check_budget(
                        cfg,
                        started,
                        &format!("exhaustive {} depth<={depth}", inv.name()),
                        checked,
                    );
                }
                let last = h.steps.last().expect("step").clone();
                if let Some(class) = violation(inv, &last) {
                    let window = window_label(&h);
                    let (persists, persists_poll) = persistence(&mut h, inv);
                    raws.push(Raw {
                        tag: tag_of(&cand),
                        seq: cand,
                        class,
                        window,
                        persists,
                        persists_poll,
                    });
                } else {
                    next.push(cand);
                }
            }
        }
        frontier = next;
    }
    Exhaustive { checked, raws }
}

/// ADR-282 I6(寿命で後退しない)の基準の測定。列を流した後、事象を何も足さずに時間だけを進め、belief(`effective_open`)が
/// 変わる列を数える。今の構造では `IntentStore` の TTL(ON 10 秒・OFF 30 秒)と観測の鮮度(3 秒)・寿命で変わりうる。
/// 戻り値: (調べた列の数, 「進めた時間・前の根拠(値) → 後の根拠(値)」ごとの (列の数, 最初の列))。
const I6_ADVANCES_MS: [u64; 2] = [11_000, 20_000];

fn i6_sweep(
    cfg: Config,
    alphabet: &[Ev],
    depth: usize,
    started: Instant,
) -> (usize, BTreeMap<String, (usize, String)>) {
    let mut frontier: Vec<Vec<Ev>> = vec![Vec::new()];
    let mut checked = 0;
    let mut by_change: BTreeMap<String, (usize, String)> = BTreeMap::new();
    for _ in 0..depth {
        let mut next = Vec::new();
        for seq in &frontier {
            for &e in alphabet {
                let mut cand = seq.clone();
                cand.push(e);
                let Some((mut h, _)) = replay(cfg, &cand) else {
                    continue;
                };
                checked += 1;
                if checked % 200 == 0 {
                    check_budget(cfg, started, "i6 sweep", checked);
                }
                let mut elapsed = 0;
                let mut alt_changed = [false; ALT_NAMES.len()];
                for ms in I6_ADVANCES_MS {
                    let before = h.steps.last().expect("step").clone();
                    h.advance_ms(ms);
                    elapsed += ms;
                    let after = h.steps.last().expect("step").clone();
                    for (k, (b, a)) in before.alts.iter().zip(after.alts.iter()).enumerate() {
                        if !alt_changed[k] && b.open != a.open {
                            alt_changed[k] = true;
                            let key = format!(
                                "({}) +{elapsed}ms: {}({}) → {}({})",
                                ALT_NAMES[k], b.basis, b.open, a.basis, a.open
                            );
                            let entry =
                                by_change.entry(key).or_insert_with(|| (0, seq_name(&cand)));
                            entry.0 += 1;
                        }
                    }
                    if before.effective_open != after.effective_open {
                        let key = format!(
                            "+{elapsed}ms: {}({}) → {}({})",
                            before.basis, before.effective_open, after.basis, after.effective_open
                        );
                        let entry = by_change.entry(key).or_insert_with(|| (0, seq_name(&cand)));
                        entry.0 += 1;
                        break;
                    }
                }
                next.push(cand);
            }
        }
        frontier = next;
    }
    (checked, by_change)
}

/// I3・I4 の掃き出しの列の長さ。
const SCOPE_DEPTH: usize = 3;

struct ScopeSweep {
    checked: usize,
    /// (モデル名, (I3 の違反した列の数, 最初の列), (I4 の違反した列の数, 最初の列))。旧 + `ALT_NAMES`。
    rows: Vec<(
        &'static str,
        (usize, Option<String>),
        (usize, Option<String>),
    )>,
}

/// ADR-282 I3・I4 の掃き出し。列を流しながら、オラクルの側でスレッドごとに次を追う(モデルには依存しない):
/// - I3: 利用者の明示操作(`user-on/off`)をしたスレッドへ、そのスレッドで他の事象が無いまま戻ったとき、belief がその値か。
/// - I4: スレッドを離れて戻ったとき(そのスレッドで事象が無い。窓の事象はフォーカスのあるスレッドでしか起きない)、belief が
///   離れたときの値のままか。開を引き継ぐスレッド(O5 の写し)は I3・I4 とも除く。
/// 旧(`effective_open`)と比較用のモデルを同じ列で調べる。
fn scope_sweep(cfg: Config, alphabet: &[Ev], depth: usize, started: Instant) -> ScopeSweep {
    let n_models = 1 + ALT_NAMES.len();
    let mut i3 = vec![(0usize, None::<String>); n_models];
    let mut i4 = vec![(0usize, None::<String>); n_models];
    let mut frontier: Vec<Vec<Ev>> = vec![Vec::new()];
    let mut checked = 0;
    for _ in 0..depth {
        let mut next = Vec::new();
        for seq in &frontier {
            for &e in alphabet {
                let mut cand = seq.clone();
                cand.push(e);
                let (mut h, wins) = cfg.start();
                let inherit_thread =
                    (cfg.tsf_inherit == Some(true)).then(|| wins[W_WEZTERM].thread);
                // スレッドごとの (I3 の記録, I4 の離れたときの値〈モデルごと〉)
                let mut rec: Vec<(u32, Option<bool>, Option<Vec<bool>>)> = Vec::new();
                let mut ok = true;
                let mut viol3 = vec![false; n_models];
                let mut viol4 = vec![false; n_models];
                let opens = |h: &Harness| -> Vec<bool> {
                    let last = h.steps.last().expect("step");
                    std::iter::once(last.effective_open)
                        .chain(last.alts.iter().map(|a| a.open))
                        .collect()
                };
                for &ev in &cand {
                    let cur = h.window().expect("窓モデル").thread;
                    if let Ev::Focus(i) = ev {
                        let to = wins[i].thread;
                        if to != cur {
                            let snap = opens(&h);
                            match rec.iter_mut().find(|r| r.0 == cur) {
                                Some(r) => r.2 = Some(snap),
                                None => rec.push((cur, None, Some(snap))),
                            }
                        }
                    }
                    if !apply(&mut h, &wins, ev) {
                        ok = false;
                        break;
                    }
                    let now = h.window().expect("窓モデル").thread;
                    match ev {
                        Ev::Focus(_) => {
                            if now != cur && Some(now) != inherit_thread {
                                let got = opens(&h);
                                if let Some(r) = rec.iter().find(|r| r.0 == now) {
                                    if let Some(v) = r.1 {
                                        for (m, g) in got.iter().enumerate() {
                                            viol3[m] |= *g != v;
                                        }
                                    }
                                    if let Some(snap) = &r.2 {
                                        for (m, (g, s0)) in got.iter().zip(snap.iter()).enumerate()
                                        {
                                            viol4[m] |= g != s0;
                                        }
                                    }
                                }
                            }
                        }
                        Ev::UserSet(v) => match rec.iter_mut().find(|r| r.0 == now) {
                            Some(r) => r.1 = Some(v),
                            None => rec.push((now, Some(v), None)),
                        },
                        _ => {
                            // そのスレッドで他の事象があったら、I3 の記録は無効にする。
                            if let Some(r) = rec.iter_mut().find(|r| r.0 == now) {
                                r.1 = None;
                            }
                        }
                    }
                }
                if !ok {
                    continue;
                }
                checked += 1;
                if checked % 200 == 0 {
                    check_budget(cfg, started, "scope sweep", checked);
                }
                for m in 0..n_models {
                    if viol3[m] {
                        i3[m].0 += 1;
                        i3[m].1.get_or_insert_with(|| seq_name(&cand));
                    }
                    if viol4[m] {
                        i4[m].0 += 1;
                        i4[m].1.get_or_insert_with(|| seq_name(&cand));
                    }
                }
                next.push(cand);
            }
        }
        frontier = next;
    }
    let names: Vec<&'static str> = std::iter::once("旧")
        .chain(ALT_NAMES.iter().copied())
        .collect();
    ScopeSweep {
        checked,
        rows: names
            .into_iter()
            .zip(i3.into_iter().zip(i4))
            .map(|(n, (a, b))| (n, a, b))
            .collect(),
    }
}

/// `raws`（`exhaustive` の結果）のうち、**事象を 1 つ削っても違反が残らない**（最小の）列。
/// 削った列が違反するのは、その列のどれかの接頭辞が `raws` にあるとき（`exhaustive` は全ての意味のある短い列を調べている）。
fn irreducible(raws: &[Raw]) -> Vec<&Raw> {
    let set: HashSet<&[Ev]> = raws.iter().map(|r| r.seq.as_slice()).collect();
    raws.iter()
        .filter(|r| {
            (0..r.seq.len()).all(|i| {
                let mut s = r.seq.clone();
                s.remove(i);
                (1..=s.len()).all(|n| !set.contains(&s[..n]))
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// ランダム探索
// ---------------------------------------------------------------------------

struct SplitMix(u64);

impl SplitMix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// 最初に違反する接頭辞へ切り、事象を 1 つずつ削って違反が残る限り縮める（再生で確かめる）。
fn minimize(cfg: Config, inv: Inv, seq: &[Ev]) -> Vec<Ev> {
    fn first_violation(cfg: Config, inv: Inv, seq: &[Ev]) -> Option<usize> {
        let (mut h, wins) = cfg.start();
        for (i, &e) in seq.iter().enumerate() {
            if !apply(&mut h, &wins, e) {
                return None;
            }
            if violation(inv, h.steps.last().expect("step")).is_some() {
                return Some(i + 1);
            }
        }
        None
    }
    let mut cur = seq.to_vec();
    if let Some(n) = first_violation(cfg, inv, &cur) {
        cur.truncate(n);
    }
    'outer: loop {
        for i in 0..cur.len() {
            let mut cand = cur.clone();
            cand.remove(i);
            if let Some(n) = first_violation(cfg, inv, &cand) {
                cand.truncate(n);
                cur = cand;
                continue 'outer;
            }
        }
        return cur;
    }
}

struct RandomResult {
    total: usize,
    violated: usize,
    by_class: BTreeMap<String, usize>,
    /// 最小化した違反（先頭 `RANDOM_MINIMIZE` 本）の最小列と分類。
    minimized: Vec<(Vec<Ev>, String)>,
}

fn random_search(cfg: Config, inv: Inv, seed: u64, started: Instant) -> RandomResult {
    let alphabet = full_alphabet();
    let mut rng = SplitMix(seed);
    let mut res = RandomResult {
        total: 0,
        violated: 0,
        by_class: BTreeMap::new(),
        minimized: Vec::new(),
    };
    for _ in 0..RANDOM_COUNT {
        let len = RANDOM_MIN_LEN + rng.below(RANDOM_MAX_LEN - RANDOM_MIN_LEN + 1);
        let (mut h, wins) = cfg.start();
        let mut seq: Vec<Ev> = Vec::new();
        res.total += 1;
        check_budget(cfg, started, "random", res.total);
        for _ in 0..len {
            // 意味の無い事象は引き直す（最大 30 回。副作用は流したときだけ）。
            let mut applied = None;
            for _ in 0..30 {
                let e = alphabet[rng.below(alphabet.len())];
                if apply(&mut h, &wins, e) {
                    applied = Some(e);
                    break;
                }
            }
            let Some(e) = applied else { break };
            seq.push(e);
            if let Some(class) = violation(inv, h.steps.last().expect("step")) {
                res.violated += 1;
                *res.by_class.entry(class.clone()).or_default() += 1;
                if res.minimized.len() < RANDOM_MINIMIZE {
                    res.minimized.push((minimize(cfg, inv, &seq), class));
                }
                break;
            }
        }
    }
    res
}

// ---------------------------------------------------------------------------
// 報告
// ---------------------------------------------------------------------------

fn count_by<'a>(
    it: impl Iterator<Item = &'a Raw>,
    key: impl Fn(&Raw) -> String,
) -> BTreeMap<String, (usize, usize, usize)> {
    let mut m: BTreeMap<String, (usize, usize, usize)> = BTreeMap::new();
    for r in it {
        let e = m.entry(key(r)).or_default();
        e.0 += 1;
        e.1 += usize::from(r.persists);
        e.2 += usize::from(r.persists_poll);
    }
    m
}

fn write_table(out: &mut String, title: &str, m: &BTreeMap<String, (usize, usize, usize)>) {
    let _ = writeln!(
        out,
        "  {title}（T=0 の件数 / 2000ms 事象なしで残る / 2000ms 内に poll が 1 回届いても残る）:"
    );
    for (k, (n, p, q)) in m {
        let _ = writeln!(out, "    {n:>6} / {p:>6} / {q:>6}  {k}");
    }
}

/// 全列挙 1 回分の報告。
fn render_exhaustive(
    out: &mut String,
    label: &str,
    cfg: Config,
    inv: Inv,
    alphabet: &[Ev],
    depth: usize,
    r: &Exhaustive,
) {
    let min = irreducible(&r.raws);
    let persists = r.raws.iter().filter(|x| x.persists).count();
    let persists_poll = r.raws.iter().filter(|x| x.persists_poll).count();
    let clean = min.iter().filter(|x| x.tag == TAG_CLEAN).count();
    let lie_free = min
        .iter()
        .filter(|x| !x.seq.iter().any(|e| e.is_lie()))
        .count();
    let _ = writeln!(
        out,
        "\n## {} {} {label}（字母 {} 種・長さ {depth} まで・検査した列 {}）",
        cfg.name(),
        inv.name(),
        alphabet.len(),
        r.checked
    );
    let _ = writeln!(
        out,
        "  初めて違反する列: {} 本（T=0）、うち 2000ms 事象なしで残る: {persists} 本（T=2000）、読める窓で poll が 1 回届いても残る: {persists_poll} 本",
        r.raws.len()
    );
    let _ = writeln!(
        out,
        "  事象を 1 つ削ると違反しない最小の列: {} 本（うち嘘の観測を含まない: {lie_free} 本、awase の操作・窓切替・正しい観測だけ: {clean} 本）",
        min.len()
    );
    write_table(
        out,
        "原因の分類（初めて違反する列）",
        &count_by(r.raws.iter(), |x| x.class.clone()),
    );
    write_table(
        out,
        "列の性質（初めて違反する列）",
        &count_by(r.raws.iter(), |x| x.tag.into()),
    );
    write_table(
        out,
        "違反した時点の窓（初めて違反する列）",
        &count_by(r.raws.iter(), |x| x.window.clone()),
    );
    write_table(
        out,
        "最後の事象（初めて違反する列）",
        &count_by(r.raws.iter(), |x| x.seq.last().expect("非空").kind().into()),
    );
    // 分類ごとの最小列の例（最短・字母順の先頭）。
    let mut by_class: BTreeMap<&str, Vec<&Raw>> = BTreeMap::new();
    for x in &min {
        by_class.entry(x.class.as_str()).or_default().push(x);
    }
    let _ = writeln!(
        out,
        "  最小列の例（分類ごと、最大 {EXAMPLES_PER_CLASS} 本）:"
    );
    for (class, xs) in &by_class {
        let _ = writeln!(out, "    [{class}] 最小列 {} 本", xs.len());
        for x in xs.iter().take(EXAMPLES_PER_CLASS) {
            let _ = writeln!(
                out,
                "      {} 手 {}（窓={}、性質={}、事象なしで残る={}、poll が届いても残る={}）",
                x.seq.len(),
                seq_name(&x.seq),
                x.window,
                x.tag,
                x.persists,
                x.persists_poll
            );
        }
    }
    let _ = writeln!(out, "  最小列の一覧（先頭 {LIST_LIMIT} 本。長さ→字母順）:");
    for x in min.iter().take(LIST_LIMIT) {
        let _ = writeln!(out, "    [{}] {}", x.tag, seq_name(&x.seq));
    }
    if min.len() > LIST_LIMIT {
        let _ = writeln!(out, "    ...（残り {} 本は省略）", min.len() - LIST_LIMIT);
    }
}

/// 実機の事故（BUG-223）の列。必ず I1 の違反として出ること。
/// ADR-282 §1.1.1 の型の列(窓 A で明示 OFF → 窓 B で F2 → A に戻る。`IntentStore` の OFF の行は 30 秒の内側)。
fn intent_store_override_sequences() -> Vec<(&'static str, Vec<Ev>)> {
    vec![
        (
            "読める窓 A で明示 OFF → tsf-native の B で F2 → A に戻る",
            vec![
                Ev::UserSet(false),
                Ev::Focus(W_WEZTERM),
                Ev::Key(VK_HIRAGANA),
                Ev::Focus(W_EXPLORER),
            ],
        ),
        (
            "imm32-unavailable の A で明示 OFF → tsf-native の B で F2 → A に戻る",
            vec![
                Ev::Focus(W_CHROME),
                Ev::UserSet(false),
                Ev::Focus(W_WEZTERM),
                Ev::Key(VK_HIRAGANA),
                Ev::Focus(W_CHROME),
            ],
        ),
    ]
}

fn bug223_sequences() -> Vec<(&'static str, Vec<Ev>)> {
    vec![
        (
            "OFF キー → F2 → 窓切替（tsf-native）",
            vec![
                Ev::Key(VK_IME_OFF),
                Ev::Key(VK_HIRAGANA),
                Ev::Focus(W_WEZTERM),
            ],
        ),
        (
            "明示 OFF → F2 → 窓切替（tsf-native）",
            vec![
                Ev::UserSet(false),
                Ev::Key(VK_HIRAGANA),
                Ev::Focus(W_WEZTERM),
            ],
        ),
        (
            "tsf-native で OFF キー → F2 → imm32-unavailable へ",
            vec![
                Ev::Focus(W_WEZTERM),
                Ev::Key(VK_IME_OFF),
                Ev::Key(VK_HIRAGANA),
                Ev::Focus(W_CHROME),
            ],
        ),
        (
            "imm32-unavailable で 明示 OFF → F2 → 同じプロセスの tsf へ",
            vec![
                Ev::Focus(W_CHROME),
                Ev::UserSet(false),
                Ev::Key(VK_HIRAGANA),
                Ev::Focus(W_WEZTERM),
            ],
        ),
    ]
}

fn run_config(cfg: Config) -> String {
    let t0 = Instant::now();
    let lap = |what: &str| eprintln!("[timing] {} {what}: {:?}", cfg.name(), t0.elapsed());
    let mut out = String::new();
    let full = full_alphabet();
    let core = core_alphabet();
    let tiny = tiny_alphabet();
    // 窓ごとに別スレッドの窓モデルは、全字母ではなく核の字母（時間の予算）。
    let (main_label, main_alphabet) = if cfg.shared {
        ("全字母", &full)
    } else {
        ("核の字母", &core)
    };
    let r_full = exhaustive(cfg, Inv::I1, main_alphabet, FULL_DEPTH, t0);
    render_exhaustive(
        &mut out,
        main_label,
        cfg,
        Inv::I1,
        main_alphabet,
        FULL_DEPTH,
        &r_full,
    );
    lap("I1 full");
    // 小さい字母の長さ 4（実機の事故の長さ）は、事故の格子（GJI の MS-IME プリセット、窓をまたいで IME 共有）だけ（時間の予算）。
    let r_tiny = (cfg.grid == Grid::GjiMsime && cfg.shared)
        .then(|| exhaustive(cfg, Inv::I1, &tiny, TINY_DEPTH, t0));
    if let Some(r) = &r_tiny {
        render_exhaustive(&mut out, "小さい字母", cfg, Inv::I1, &tiny, TINY_DEPTH, r);
    }
    lap("I1 tiny");
    let r = exhaustive(cfg, Inv::I2, &core, I2_DEPTH, t0);
    render_exhaustive(&mut out, "核の字母", cfg, Inv::I2, &core, I2_DEPTH, &r);

    lap("I2 core");
    // ADR-282 I6: 時間の経過だけで belief が変わる列(核の字母・長さ 2。O3 を S′ にも課す所有者の決定の基準の測定)。
    let (i6_checked, i6) = i6_sweep(cfg, &core, I6_DEPTH, t0);
    let total: usize = i6.values().map(|(n, _)| n).sum();
    let _ = writeln!(
        out,
        "\n## {} I6 時間の経過だけで belief が変わる列（核の字母・長さ {I6_DEPTH} まで・検査した列 {i6_checked}。列の後に +11000ms、さらに +20000ms）",
        cfg.name()
    );
    let _ = writeln!(out, "  変わった列: {total} 本");
    for (k, (n, example)) in &i6 {
        let _ = writeln!(out, "    {n:>6}  {k}(例: {example})");
    }
    lap("I6");
    // 実機の事故の列。
    let _ = writeln!(out, "\n## {} 実機の事故（BUG-223）の列", cfg.name());
    for (name, seq) in bug223_sequences() {
        let verdict = match replay(cfg, &seq) {
            None => "（この窓モデルでは再生できない）".to_string(),
            Some((mut h, _)) => {
                let steps: Vec<_> = h.steps.iter().skip(SETTLE_STEPS).cloned().collect();
                match steps.iter().find_map(|s| violation(Inv::I1, s)) {
                    Some(class) => {
                        let (idle, poll) = persistence(&mut h, Inv::I1);
                        format!(
                            "I1 違反 [{class}]（事象なしで残る={idle}、poll が届いても残る={poll}）"
                        )
                    }
                    None => "違反なし".to_string(),
                }
            }
        };
        let _ = writeln!(out, "  {name}: {} → {verdict}", seq_name(&seq));
    }
    // ADR-282 §1.1.1: 実機の再現(「r;にちは」)の型。窓 A の明示 OFF が `IntentStore` の行(TTL 30 秒)に残り、別の窓 B で ON に
    // した後に A へ戻ると、その行が `resolve_open_at` の結果を上書きする。A・B が開を共有する窓モデルでだけ I1 の違反になる見込み。
    let _ = writeln!(
        out,
        "\n## {} 実機の再現(ADR-282 §1.1.1、IntentStore の行の上書き)の列",
        cfg.name()
    );
    for (name, seq) in intent_store_override_sequences() {
        let verdict = match replay(cfg, &seq) {
            None => "(この窓モデルでは再生できない)".to_string(),
            Some((h, _)) => {
                let steps: Vec<_> = h.steps.iter().skip(SETTLE_STEPS).cloned().collect();
                let old = match steps.iter().find_map(|s| violation(Inv::I1, s)) {
                    Some(class) => format!("I1 違反 [{class}]"),
                    None => "違反なし".to_string(),
                };
                let mut parts = vec![format!("旧: {old}")];
                for k in 0..ALT_NAMES.len() {
                    let v = match steps.iter().find_map(|s| violation(Inv::I1Alt(k), s)) {
                        Some(class) => format!("I1 違反 [{class}]"),
                        None => "違反なし".to_string(),
                    };
                    parts.push(format!("{}: {v}", ALT_NAMES[k]));
                }
                parts.join(" / ")
            }
        };
        let _ = writeln!(out, "  {name}: {} → {verdict}", seq_name(&seq));
    }

    // ランダム探索（I1）。
    let seed = 0x00B0_0AC1_E000
        + (cfg.grid as u64) * 16
        + u64::from(cfg.shared)
        + cfg.tsf_inherit.map_or(0, |i| 2 + u64::from(i));
    let rr = random_search(cfg, Inv::I1, seed, t0);
    let _ = writeln!(
        out,
        "\n## {} I1 ランダム探索（固定 seed・全字母・長さ {RANDOM_MIN_LEN}〜{RANDOM_MAX_LEN}・{} 本）",
        cfg.name(),
        rr.total
    );
    let _ = writeln!(out, "  違反した列: {} 本", rr.violated);
    for (k, n) in &rr.by_class {
        let _ = writeln!(out, "    {n:>6}  {k}");
    }
    lap("random");
    let known: HashSet<Vec<Ev>> = r_tiny
        .iter()
        .flat_map(|r| r.raws.iter())
        .chain(r_full.raws.iter())
        .map(|r| r.seq.clone())
        .collect();
    let mut news: Vec<String> = Vec::new();
    let mut seen_new: HashSet<Vec<Ev>> = HashSet::new();
    for (seq, class) in &rr.minimized {
        if !known.contains(seq) && seen_new.insert(seq.clone()) {
            news.push(format!("{} 手 [{class}] {}", seq.len(), seq_name(seq)));
        }
    }
    news.sort();
    let _ = writeln!(
        out,
        "  最小化した {} 本のうち、全列挙の最小列に無い（長さが全列挙の範囲外か、別の最小列）: {} 本",
        rr.minimized.len(),
        news.len()
    );
    for n in news.iter().take(10) {
        let _ = writeln!(out, "    {n}");
    }
    out
}

/// ADR-282 V0: 比較用のモデル(Z・Z+U2・S′・S′+U2・配線Z+U2)の I1・I2 と I3・I4 の掃き出し(`belief_engine_alt_comparison`)。
/// 旧の測定(`run_config`)とは別のテストにして、時間の予算を分ける。
fn run_alt_config(cfg: Config) -> String {
    let t0 = Instant::now();
    let lap = |what: &str| eprintln!("[timing] {} {what}: {:?}", cfg.name(), t0.elapsed());
    let mut out = String::new();
    let full = full_alphabet();
    let core = core_alphabet();
    let (main_label, main_alphabet) = if cfg.shared {
        ("全字母", &full)
    } else {
        ("核の字母", &core)
    };
    // ADR-282 V0: 比較用のモデル(Z・Z+U2・S′・S′+U2・配線Z+U2)の I1・I2(同じ字母・同じ長さ。旧と並べて比べる)。U2 の案(新しい
    // スレッドは閉)は、窓をまたいで同じスレッドの shared では変わらないので、窓ごとに別スレッドの設定だけ回す。
    for k in 0..ALT_NAMES.len() {
        let u2 = ALT_NAMES[k].ends_with("+U2");
        if u2 && cfg.shared {
            continue;
        }
        let r1 = exhaustive(cfg, Inv::I1Alt(k), main_alphabet, FULL_DEPTH, t0);
        render_exhaustive(
            &mut out,
            main_label,
            cfg,
            Inv::I1Alt(k),
            main_alphabet,
            FULL_DEPTH,
            &r1,
        );
        let r2 = exhaustive(cfg, Inv::I2Alt(k), &core, I2_DEPTH, t0);
        render_exhaustive(
            &mut out,
            "核の字母",
            cfg,
            Inv::I2Alt(k),
            &core,
            I2_DEPTH,
            &r2,
        );
        lap(ALT_NAMES[k]);
    }
    // ADR-282 V0: I3(最後の明示操作が勝つ)・I4(スコープの独立)の掃き出し(核の字母・長さ 3)。
    let sc = scope_sweep(cfg, &core, SCOPE_DEPTH, t0);
    let _ = writeln!(
        out,
        "\n## {} I3・I4 の掃き出し（核の字母・長さ {SCOPE_DEPTH} まで・検査した列 {}。違反した列の数）",
        cfg.name(),
        sc.checked
    );
    let _ = writeln!(
        out,
        "  I3 = 利用者の明示操作(user-on/off)をしたスレッドへ、そのスレッドで他の事象が無いまま戻ったとき、belief がその値か。"
    );
    let _ = writeln!(
        out,
        "  I4 = スレッドを離れて戻ったとき(そのスレッドで事象が無いとき)、belief が離れたときの値のままか。開を引き継ぐスレッドは除く。"
    );
    for (name, (i3, i3_ex), (i4, i4_ex)) in &sc.rows {
        let _ = writeln!(
            out,
            "  {name:<8} I3 {i3:>4} 本{}  I4 {i4:>4} 本{}",
            i3_ex
                .as_ref()
                .map(|e| format!("(例: {e})"))
                .unwrap_or_default(),
            i4_ex
                .as_ref()
                .map(|e| format!("(例: {e})"))
                .unwrap_or_default()
        );
    }
    lap("I3/I4");
    out
}

fn alt_golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join("belief_engine_alt_comparison.txt")
}

fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join("belief_engine_invariant_violations.txt")
}

#[test]
fn belief_engine_invariant_baseline() {
    let started = Instant::now();
    // IME の状態を共有する窓モデルは全格子、窓ごとに別スレッドの窓モデルは F2 が ON にする GJI の MS-IME プリセットだけ（時間の予算）。
    let mut configs: Vec<Config> = Grid::ALL
        .into_iter()
        .map(|grid| Config::plain(grid, true))
        .collect();
    configs.push(Config::plain(Grid::GjiMsime, false));
    // ADR-282 V0: 窓の種類で保持単位を混ぜた窓モデル(測った事実: 読める窓・Chromium は独立、MS-IME 本体 × WT は入場時に開を
    // 引き継ぐ〈O5〉、GJI × tsf-native は未確定なので両方)。核の字母(時間の予算)。
    configs.push(Config::mixed(Grid::MsimeNative, true));
    configs.push(Config::mixed(Grid::GjiMsime, false));
    configs.push(Config::mixed(Grid::GjiMsime, true));
    let results: Vec<String> = std::thread::scope(|s| {
        let handles: Vec<_> = configs
            .iter()
            .map(|&cfg| {
                std::thread::Builder::new()
                    .stack_size(16 * 1024 * 1024)
                    .spawn_scoped(s, move || run_config(cfg))
                    .expect("spawn")
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("設定ごとの測定"))
            .collect()
    });
    let mut out = String::from(
        "# 「IME の真の状態」と「Engine の活性」の食い違い（基準の測定）。不変条件 I1/I2 の定義・字母・読み方は\n\
         # tests/belief_engine_invariant.rs の冒頭と docs/tasks/belief-engine-invariant-baseline-2026-10-26.md。\n\
         # 違反が減ったら golden を更新する（ゼロベースの設計の目標は I1 の違反ゼロ）。違反が増えたら落ちる。\n\
         # 自動生成: UPDATE_GOLDEN=1 cargo test -p awase-windows --test belief_engine_invariant\n",
    );
    for r in &results {
        out.push_str(r);
    }

    eprintln!("belief_engine_invariant: {:?}", started.elapsed());
    let path = golden_path();
    if std::env::var("UPDATE_GOLDEN").as_deref() == Ok("1") {
        std::fs::write(&path, &out).expect("golden を書けない");
        eprintln!("golden を更新しました: {}", path.display());
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    assert!(
        out == expected,
        "golden {} が現状と一致しない。意図した変化なら UPDATE_GOLDEN=1 で再生成して差分を確認すること。\n\
         ---- 現状の全文 ----\n{out}---- ここまで ----",
        path.display()
    );
}

/// 実機の事故の列は、IME の状態が窓をまたいで共有される窓モデル（`shared`）では必ず I1 の違反として出る（所有者の要件）。
#[test]
fn bug223_sequences_are_i1_violations_when_ime_state_is_shared() {
    // F2 が閉から ON にする格子（GJI の MS-IME プリセットと MS-IME 本体。ATOK プリセットの F2 は閉のまま変わらない）。
    for grid in [Grid::GjiMsime, Grid::MsimeNative] {
        let cfg = Config::plain(grid, true);
        for (name, seq) in bug223_sequences() {
            let (h, _) = replay(cfg, &seq).expect("shared では再生できる");
            assert!(
                h.steps
                    .iter()
                    .skip(SETTLE_STEPS)
                    .any(|s| violation(Inv::I1, s).is_some()),
                "{}: BUG-223 の列「{name}」が I1 の違反として出ない\n{}",
                cfg.name(),
                h.trace()
            );
        }
    }
}

/// ADR-282 V0: 旧と比較用のモデル(Z・Z+U2・S′・S′+U2・配線Z+U2)を同じ事象列で並べる(I1・I2・I3・I4)。数値は golden
/// `tests/golden/belief_engine_alt_comparison.txt`。旧だけの測定は `belief_engine_invariant_baseline`。
#[test]
fn belief_engine_alt_comparison() {
    let started = Instant::now();
    let configs = [
        Config::plain(Grid::Atok, true),
        Config::plain(Grid::GjiMsime, true),
        Config::plain(Grid::GjiMsime, false),
        Config::mixed(Grid::MsimeNative, true),
        Config::mixed(Grid::GjiMsime, false),
        Config::mixed(Grid::GjiMsime, true),
    ];
    let results: Vec<String> = std::thread::scope(|s| {
        let handles: Vec<_> = configs
            .iter()
            .map(|&cfg| {
                std::thread::Builder::new()
                    .stack_size(16 * 1024 * 1024)
                    .spawn_scoped(s, move || run_alt_config(cfg))
                    .expect("spawn")
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("設定ごとの測定"))
            .collect()
    });
    let mut out = String::from(
        "# ADR-282 V0: 旧(今の本物の belief)と比較用のモデル(Z・Z+U2・S′・S′+U2・配線Z+U2)を同じ事象列で並べる。\n\
         # Z は crates/awase-ime-core/src/state/scope_belief.rs、S′ は tests/support/sprime.rs。配線Z+U2 は本物の hub の\n\
         # shadow(ImeStateHub::scope_shadow_*、V1)を殻と同じ口で動かしたもの。旧の数値は\n\
         # belief_engine_invariant_violations.txt。読み方は docs/adr/282-per-scope-latched-ime-belief-zero-base.md §14。\n\
         # 自動生成: UPDATE_GOLDEN=1 cargo test -p awase-windows --test belief_engine_invariant\n",
    );
    for r in &results {
        out.push_str(r);
    }
    eprintln!("belief_engine_alt_comparison: {:?}", started.elapsed());
    let path = alt_golden_path();
    if std::env::var("UPDATE_GOLDEN").as_deref() == Ok("1") {
        std::fs::write(&path, &out).expect("golden を書けない");
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    assert!(
        out == expected,
        "golden {} が現状と一致しない。意図した変化なら UPDATE_GOLDEN=1 で再生成して差分を確認すること。\n\
         ---- 現状の全文 ----\n{out}---- ここまで ----",
        path.display()
    );
}
