//! 擬似 Mozc（cold の Mozc TIP と、その窓の文書）と、awase の確認（ADR-270/284/285）の流れ。
//!
//! 擬似 IME（`pseudo_ime.rs`）・擬似窓（`src/tsf/warmup/pseudo_window.rs`）と同じく、実機の観測から決めた挙動を
//! 決定的に模す。ここでは Mozc の共有メモリの値（`key_seq`・`phase`・`server_ready`）と、文書に何が出るかを持つ。
//!
//! 連携のための口（journal との突き合わせ用。段階 1）:
//! - [`Run::trace`]: 何を・いつ送り、何を読み、どう判断し、何を回収したかの時系列（[`TraceEvent`]）。
//!   擬似物の種類によらない中立の形にして、後で実機の記録（journal・ログ）から作る同じ形と比べられるようにする。
//! - [`PseudoMozc::shm_record`]: 実機と同じ `MozcShmRecord`（journal のエントリ）を、実機と同じ変換
//!   （`state::mozc_shm::to_record`）で作る。擬似物の読みが実機の journal と同じ解析器に載る。
//!
//! 限界（正直に）:
//! - [`run`] の流れは `awase-windows` の `MozcComposeCheckFsm::tick_inner` の写しで、FSM 本体（`cfg(windows)`）は
//!   Linux では走らない。判断関数は本物だが、つなぎの食い違いは検出しない。
//! - `MozcShmRecord` は開閉とモードだけを持つ。確認の結末と `key_seq` の前後は、`JournalEntry::LiteralDetect` の
//!   `facts.evidence.mozc_key_seq`（`{before, after}`）に残る（Mozc の確認が出した verdict のとき。回収・
//!   `notable` な確認のみ。健全な確認の繰り返しは畳まれる）。経過の時刻は journal の envelope と
//!   `evidence.epoch_send_ms` で復元する。擬似トレースとの突き合わせ（journal → シナリオ）は段階 2 の課題。

use awase_windows::focus::class_names::mozc_compose_check_applies;
use awase_windows::journal::MozcShmRecord;
use awase_windows::state::mozc_shm::{
    compose_literal_backs, judge_compose, judge_settled, should_check_compose, split_first_key,
    to_record, ComposeBlock, ComposeReading, ComposeVerdict, CompositionMode, Outcome,
    RecordContext, Route, SettleVerdict, ShmState, Trigger,
};
use awase_windows::tuning::MOZC_COMPOSE_CHECK_MS;

/// 確認の tick 間隔（実機の `TIMER_TSF_PROBE` は 10ms）。
pub const TICK_MS: u64 = 10;

/// cold の Mozc TIP と、その窓の文書の挙動。
#[derive(Debug, Clone)]
pub struct PseudoMozc {
    // ── 宣言（挙動の型） ──
    /// cold の最初の 1 キーを TIP が見ず、文書へそのまま届く（BUG-225/226 の実測）。一度だけ起きる。
    pub drop_first_key: bool,
    /// TIP が最初のキーを見てから入力中の文字（phase 1）ができるまで（ms）。実測 ~60ms 以内（ADR-270）。
    pub compose_delay_ms: u64,
    /// `false` にすると Mozc サーバーが応答できない（キーは握りつぶされ、文書にも出ない）。
    pub server_ready: bool,
    /// 2 打目以降を `key_seq` が数えるまでの遅れ（ms）。実測: 確認済み 21 回のうち 2 回で 2 打目が +30ms 遅れた
    /// （ADR-284、2026-10-21）。
    pub second_key_lag_ms: u64,
    // ── 状態 ──
    dropped: bool,
    /// `key_seq` の基準値（送信の前）。TIP が見たキーが数えられるたびに [`PseudoMozc::key_seq_at`] で足される。
    pub key_seq: u32,
    /// TIP が見たキーが `key_seq` に数えられる時刻。
    seq_ready: Vec<u64>,
    /// TIP が受け取ったローマ字（入力中の文字の元）。
    composing: String,
    composing_since: Option<u64>,
    /// 文書へリテラルで届いた文字。
    pub doc: String,
    /// 発行された Backspace の数（握りつぶされたキーの後に Backspace が飛んでいないことの検証用）。
    pub backspaces_issued: usize,
}

impl PseudoMozc {
    pub fn cold_dropping_first_key() -> Self {
        Self::new(true, true)
    }

    pub fn healthy() -> Self {
        Self::new(false, true)
    }

    pub fn new(drop_first_key: bool, server_ready: bool) -> Self {
        Self {
            drop_first_key,
            compose_delay_ms: 30,
            server_ready,
            second_key_lag_ms: 0,
            dropped: false,
            key_seq: 330,
            seq_ready: Vec::new(),
            composing: String::new(),
            composing_since: None,
            doc: String::new(),
            backspaces_issued: 0,
        }
    }

    /// VK でローマ字 1 文字を送る。
    pub fn send_key(&mut self, c: char, now: u64) {
        if !self.server_ready {
            // サーバー無応答: キーは TIP に取り込まれず、文書にも出ない（握りつぶし）。
            return;
        }
        if self.drop_first_key && !self.dropped {
            self.dropped = true;
            self.doc.push(c);
            return;
        }
        // 最初に見たキーはすぐ数えられ、2 打目以降は `second_key_lag_ms` 遅れる。
        let ready = if self.seq_ready.is_empty() {
            now
        } else {
            now + self.second_key_lag_ms
        };
        self.seq_ready.push(ready);
        self.composing.push(c);
        self.composing_since.get_or_insert(now);
    }

    pub fn send(&mut self, romaji: &str, now: u64) {
        for c in romaji.chars() {
            self.send_key(c, now);
        }
    }

    /// 入力中の文字の段階（0=なし、1=入力中）。
    fn phase(&self, now: u64) -> u32 {
        match self.composing_since {
            Some(t) if now >= t + self.compose_delay_ms => 1,
            _ => 0,
        }
    }

    /// `now` の時点で数えられている `key_seq`。
    pub fn key_seq_at(&self, now: u64) -> u32 {
        let ready = self.seq_ready.iter().filter(|t| **t <= now).count();
        self.key_seq + u32::try_from(ready).unwrap_or(u32::MAX)
    }

    /// 共有メモリの読み。
    pub fn read(&self, now: u64) -> ComposeReading {
        ComposeReading::Block(ComposeBlock {
            active: true,
            effective_open: true,
            effective_mode: 1, // ひらがな
            phase: self.phase(now),
            scope_forced_mode: 0,
            server_ready: self.server_ready,
            key_seq: self.key_seq_at(now),
            composition_key_seq: self.key_seq_at(now),
        })
    }

    /// 実機と同じ変換（`to_record`）で作る journal のエントリ。擬似物の読みを、実機の `MozcShmRecord` と同じ形で出す。
    pub fn shm_record(&self, trigger: Trigger, thread_id: u32, process_id: u32) -> MozcShmRecord {
        let state = ShmState {
            version: 5,
            tsf_open: true,
            tsf_mode: Some(CompositionMode::Hiragana),
            effective_open: true,
            effective_mode: Some(CompositionMode::Hiragana),
        };
        to_record(
            &Outcome::Accepted(state),
            RecordContext {
                trigger,
                route: Route::Local,
                read_us: 0,
                profile: "standard",
                thread_id,
                process_id,
                belief_open: Some(true),
                belief_mode: Some("Hiragana".to_string()),
            },
        )
    }

    pub fn escape(&mut self) {
        self.composing.clear();
        self.composing_since = None;
    }

    pub fn backspace(&mut self, n: usize) {
        self.backspaces_issued += n;
        for _ in 0..n {
            self.doc.pop();
        }
    }

    /// 画面に見える文字列（文書 + 入力中の文字）。
    pub fn text(&self) -> String {
        format!("{}{}", self.doc, kana(&self.composing))
    }
}

/// 入力中の文字の見え方。この試験に出る範囲だけの最小の表（か行 + 母音）。
fn kana(romaji: &str) -> String {
    const A: [&str; 5] = ["あ", "い", "う", "え", "お"];
    const K: [&str; 5] = ["か", "き", "く", "け", "こ"];
    let vowel = |c: char| "aiueo".find(c);
    let cs: Vec<char> = romaji.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < cs.len() {
        if let Some(v) = vowel(cs[i]) {
            out.push_str(A[v]);
            i += 1;
        } else if let (Some(v), 'k') = (cs.get(i + 1).and_then(|c| vowel(*c)), cs[i]) {
            out.push_str(K[v]);
            i += 2;
        } else {
            out.push(cs[i]);
            i += 1;
        }
    }
    out
}

/// awase の側の設定（修正の有無を切り替える）。
#[derive(Debug, Clone, Copy)]
pub struct Awase {
    /// 確認を張る窓の判定。`true`=ADR-285 の除外リスト（現行）、`false`=BUG-226 以前の許可リスト（WT・WezTerm のみ）。
    pub inverted_window_rule: bool,
    /// ADR-284 の `key_seq` の進みによる部分リテラルの検出（BUG-225 の修正）。
    pub settle: bool,
    /// ADR-285 の最初の 1 キーだけ送る per-VK 化。
    pub per_vk: bool,
}

impl Awase {
    /// 現行（develop）の設定。
    pub const CURRENT: Self = Self {
        inverted_window_rule: true,
        settle: true,
        per_vk: true,
    };
    /// BUG-225 の修正前（ADR-270 の確認のみ、許可リスト）。
    pub const BEFORE_BUG225: Self = Self {
        inverted_window_rule: false,
        settle: false,
        per_vk: false,
    };
    /// BUG-225 の修正後・BUG-226 の修正前（ADR-284 のみ、許可リスト）。
    pub const BEFORE_BUG226: Self = Self {
        inverted_window_rule: false,
        settle: true,
        per_vk: false,
    };
}

pub const WINDOWS_TERMINAL: &str = "CASCADIA_HOSTING_WINDOW_CLASS";
pub const LINE: &str = "Qt663QWindowIcon";
pub const CHROME: &str = "Chrome_WidgetWin_1";

/// BUG-226 以前の許可リスト（`is_mozc_compose_check_window` が Windows Terminal・WezTerm だけだった時代）。
fn legacy_applies(class: &str) -> bool {
    matches!(class, WINDOWS_TERMINAL | "org.wezfurlong.wezterm")
}

/// 確認の経過の 1 件（中立の形。実機の記録から作る同じ形と比べる）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceEvent {
    /// 確認を張らず、全体をそのまま送った。
    SentUnchecked { romaji: String },
    /// 確認を張って送った（per-VK なら最初の 1 キーだけ）。
    Sent { romaji: String },
    /// 入力中の文字ができた（`key_seq` の進み付き）。
    Confirmed { key_seq_delta: u32 },
    /// 進みが足りず、期限まで読み直す（ADR-284）。
    Settling { key_seq_delta: u32 },
    /// リテラルと判断して回収した。
    Recovered {
        backs: usize,
        escape: bool,
        resent: String,
    },
    /// 判断せずに終えた。
    Skipped,
    /// 残りを送った。
    SentRest { romaji: String },
}

/// 確認の 1 回分の結果。
#[derive(Debug, Clone)]
pub struct Run {
    /// 結末の画面の文字列。
    pub text: String,
    /// 時刻（ms、送信を 0 とする）と出来事の時系列。
    pub trace: Vec<(u64, TraceEvent)>,
}

impl Run {
    pub fn events(&self) -> Vec<&TraceEvent> {
        self.trace.iter().map(|(_, e)| e).collect()
    }
}

/// cold の最初の送信 `romaji` を `class` の窓へ送り、確認の結末までを通す。
pub fn run(cfg: Awase, class: &str, pm: &mut PseudoMozc, romaji: &str) -> Run {
    let mut trace: Vec<(u64, TraceEvent)> = Vec::new();
    let armed = if cfg.inverted_window_rule {
        mozc_compose_check_applies(true, class)
    } else {
        legacy_applies(class)
    };
    let t0 = 0;
    if !armed || !should_check_compose(pm.read(t0), romaji) {
        pm.send(romaji, t0);
        trace.push((
            t0,
            TraceEvent::SentUnchecked {
                romaji: romaji.to_string(),
            },
        ));
        return Run {
            text: pm.text(),
            trace,
        };
    }

    let before = pm.key_seq;
    let (sent, rest) = match (cfg.per_vk, split_first_key(romaji)) {
        (true, Some((first, rest))) => (first, rest),
        _ => (romaji, ""),
    };
    pm.send(sent, t0);
    trace.push((
        t0,
        TraceEvent::Sent {
            romaji: sent.to_string(),
        },
    ));

    let mut settling = false;
    let mut now = t0;
    // 期限 + 余裕まで tick する。
    while now <= t0 + MOZC_COMPOSE_CHECK_MS + 100 {
        now += TICK_MS;
        let elapsed = now - t0;
        let reading = pm.read(now);
        let ComposeReading::Block(b) = reading else {
            break;
        };
        let delta = b.key_seq.wrapping_sub(before);
        let sent_len = compose_literal_backs(sent);
        if settling {
            match judge_settled(sent_len, delta) {
                SettleVerdict::Complete => {
                    trace.push((
                        elapsed,
                        TraceEvent::Confirmed {
                            key_seq_delta: delta,
                        },
                    ));
                    break;
                }
                SettleVerdict::Skip => {
                    trace.push((elapsed, TraceEvent::Skipped));
                    break;
                }
                SettleVerdict::PartialLiteral { missing } => {
                    if elapsed >= MOZC_COMPOSE_CHECK_MS {
                        recover(pm, &mut trace, elapsed, missing, true, romaji);
                        return Run {
                            text: pm.text(),
                            trace,
                        };
                    }
                    continue;
                }
            }
        }
        match judge_compose(elapsed, MOZC_COMPOSE_CHECK_MS, reading) {
            ComposeVerdict::Pending => {}
            ComposeVerdict::Confirmed => {
                if cfg.settle
                    && matches!(
                        judge_settled(sent_len, delta),
                        SettleVerdict::PartialLiteral { .. }
                    )
                {
                    settling = true;
                    trace.push((
                        elapsed,
                        TraceEvent::Settling {
                            key_seq_delta: delta,
                        },
                    ));
                    continue;
                }
                trace.push((
                    elapsed,
                    TraceEvent::Confirmed {
                        key_seq_delta: delta,
                    },
                ));
                break;
            }
            ComposeVerdict::Literal => {
                recover(
                    pm,
                    &mut trace,
                    elapsed,
                    compose_literal_backs(sent),
                    false,
                    romaji,
                );
                return Run {
                    text: pm.text(),
                    trace,
                };
            }
            ComposeVerdict::Skip(_) => {
                trace.push((elapsed, TraceEvent::Skipped));
                break;
            }
        }
    }
    // 確認できた（または判断しない）: 残りを送る。
    if !rest.is_empty() {
        pm.send(rest, now);
        trace.push((
            now - t0,
            TraceEvent::SentRest {
                romaji: rest.to_string(),
            },
        ));
    }
    Run {
        text: pm.text(),
        trace,
    }
}

/// リテラルの回収: （Escape で入力中の文字を捨て、）Backspace で素通りした分を消し、全体を再送する。
fn recover(
    pm: &mut PseudoMozc,
    trace: &mut Vec<(u64, TraceEvent)>,
    at: u64,
    backs: usize,
    escape: bool,
    romaji: &str,
) {
    if escape {
        pm.escape();
    }
    pm.backspace(backs);
    pm.send(romaji, 10_000); // 全体を再送（cold は一度だけなので落ちない）
    trace.push((
        at,
        TraceEvent::Recovered {
            backs,
            escape,
            resent: romaji.to_string(),
        },
    ));
}
