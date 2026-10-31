//! 擬似 Mozc による BUG-225 / BUG-226 の再現（Linux で走る）。
//!
//! 目的: 「実機で起きたバグを、修正前の設定で再現し、修正後の設定で直ることを示す」型を作る。
//! 擬似物の精度は「過去のバグを修正前に再現できるか」で測る（この 2 件が最初の検証）。
//!
//! 構成:
//! - [`PseudoMozc`]: cold の Mozc TIP が共有メモリに見せる値（`key_seq`・`phase`・`server_ready`）と、
//!   アプリの文書に何が出るかを決定的に模す。cold で「最初の 1 キーを TIP が見ない」挙動（BUG-225/226 の実測）を
//!   `drop_first_key` で宣言する。
//! - [`run`]: awase の確認（ADR-270/284/285）の流れを、**本物の純粋な判断関数**
//!   （`judge_compose`・`judge_settled`・`split_first_key`・`compose_literal_backs`・`should_check_compose`・
//!   `mozc_compose_check_applies`）で組んだもの。
//!
//! 限界（正直に）: `run` の流れ自体は `awase-windows` の `MozcComposeCheckFsm::tick_inner` の写しで、FSM 本体
//! （`cfg(windows)`）は Linux では走らない。判断関数は本物だが、つなぎ（settle への遷移・回収の順序）は写し。
//! つなぎが食い違えば Windows 側の単体テストが先に壊れるが、写しの食い違い自体は検出しない。
//! 後続でつなぎの判断も `awase-ime-core` へ移して写しをなくす(試験は殻の tests/ に置く: core-layers の配線検査)。
//!
//! 時間は仮想（ms の整数）。実測の出典: BUG-225（sent 2 chars, key_seq 330→331）、BUG-226（LINE、cold=184）。

use awase_windows::focus::class_names::mozc_compose_check_applies;
use awase_windows::state::mozc_shm::{
    compose_literal_backs, judge_compose, judge_settled, should_check_compose, split_first_key,
    ComposeBlock, ComposeReading, ComposeVerdict, SettleVerdict,
};
use awase_windows::tuning::MOZC_COMPOSE_CHECK_MS;

/// 確認の tick 間隔（実機の `TIMER_TSF_PROBE` は 10ms）。
const TICK_MS: u64 = 10;

/// cold の Mozc TIP と、その窓の文書の挙動。
#[derive(Debug, Clone)]
struct PseudoMozc {
    // ── 宣言（挙動の型） ──
    /// cold の最初の 1 キーを TIP が見ず、文書へそのまま届く（BUG-225/226 の実測）。一度だけ起きる。
    drop_first_key: bool,
    /// TIP が最初のキーを見てから入力中の文字（phase 1）ができるまで（ms）。実測 ~60ms 以内（ADR-270）。
    compose_delay_ms: u64,
    /// `false` にすると Mozc サーバーが応答できない（キーは握りつぶされ、文書にも出ない）。
    server_ready: bool,
    // ── 状態 ──
    dropped: bool,
    key_seq: u32,
    /// TIP が受け取ったローマ字（入力中の文字の元）。
    composing: String,
    composing_since: Option<u64>,
    /// 文書へリテラルで届いた文字。
    doc: String,
    /// 発行された Backspace の数（握りつぶされたキーの後に Backspace が飛んでいないことの検証用）。
    backspaces_issued: usize,
}

impl PseudoMozc {
    fn cold_dropping_first_key() -> Self {
        Self::new(true, true)
    }

    fn healthy() -> Self {
        Self::new(false, true)
    }

    fn new(drop_first_key: bool, server_ready: bool) -> Self {
        Self {
            drop_first_key,
            compose_delay_ms: 30,
            server_ready,
            dropped: false,
            key_seq: 330,
            composing: String::new(),
            composing_since: None,
            doc: String::new(),
            backspaces_issued: 0,
        }
    }

    /// VK でローマ字 1 文字を送る。
    fn send_key(&mut self, c: char, now: u64) {
        if !self.server_ready {
            // サーバー無応答: キーは TIP に取り込まれず、文書にも出ない（握りつぶし）。
            return;
        }
        if self.drop_first_key && !self.dropped {
            self.dropped = true;
            self.doc.push(c);
            return;
        }
        self.key_seq += 1;
        self.composing.push(c);
        self.composing_since.get_or_insert(now);
    }

    fn send(&mut self, romaji: &str, now: u64) {
        for c in romaji.chars() {
            self.send_key(c, now);
        }
    }

    /// 共有メモリの読み。
    fn read(&self, now: u64) -> ComposeReading {
        let phase = match self.composing_since {
            Some(t) if now >= t + self.compose_delay_ms => 1,
            _ => 0,
        };
        ComposeReading::Block(ComposeBlock {
            active: true,
            effective_open: true,
            effective_mode: 1, // ひらがな
            phase,
            scope_forced_mode: 0,
            server_ready: self.server_ready,
            key_seq: self.key_seq,
            composition_key_seq: self.key_seq,
        })
    }

    fn escape(&mut self) {
        self.composing.clear();
        self.composing_since = None;
    }

    fn backspace(&mut self, n: usize) {
        self.backspaces_issued += n;
        for _ in 0..n {
            self.doc.pop();
        }
    }

    /// 画面に見える文字列（文書 + 入力中の文字）。
    fn text(&self) -> String {
        format!("{}{}", self.doc, kana(&self.composing))
    }
}

/// 入力中の文字の見え方。この試験に出る範囲だけの最小の表（か行 + 母音）。
fn kana(romaji: &str) -> String {
    let mut out = String::new();
    let cs: Vec<char> = romaji.chars().collect();
    let mut i = 0;
    while i < cs.len() {
        let vowel = |c: char| "aiueo".find(c);
        const A: [&str; 5] = ["あ", "い", "う", "え", "お"];
        const K: [&str; 5] = ["か", "き", "く", "け", "こ"];
        if let Some(v) = vowel(cs[i]) {
            out.push_str(A[v]);
            i += 1;
        } else if cs[i] == 'k' && i + 1 < cs.len() && vowel(cs[i + 1]).is_some() {
            out.push_str(K[vowel(cs[i + 1]).unwrap()]);
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
struct Awase {
    /// 確認を張る窓の判定。`true`=ADR-285 の除外リスト（現行）、`false`=BUG-226 以前の許可リスト（WT・WezTerm のみ）。
    inverted_window_rule: bool,
    /// ADR-284 の `key_seq` の進みによる部分リテラルの検出（BUG-225 の修正）。
    settle: bool,
    /// ADR-285 の最初の 1 キーだけ送る per-VK 化。
    per_vk: bool,
}

impl Awase {
    /// 現行（develop）の設定。
    const CURRENT: Self = Self {
        inverted_window_rule: true,
        settle: true,
        per_vk: true,
    };
    /// BUG-225 の修正前（ADR-270 の確認のみ、許可リスト）。
    const BEFORE_BUG225: Self = Self {
        inverted_window_rule: false,
        settle: false,
        per_vk: false,
    };
    /// BUG-225 の修正後・BUG-226 の修正前（ADR-284 のみ、許可リスト）。
    const BEFORE_BUG226: Self = Self {
        inverted_window_rule: false,
        settle: true,
        per_vk: false,
    };
}

const WINDOWS_TERMINAL: &str = "CASCADIA_HOSTING_WINDOW_CLASS";
const LINE: &str = "Qt663QWindowIcon";
const CHROME: &str = "Chrome_WidgetWin_1";

/// BUG-226 以前の許可リスト（`is_mozc_compose_check_window` が Windows Terminal・WezTerm だけだった時代）。
fn legacy_applies(class: &str) -> bool {
    matches!(class, WINDOWS_TERMINAL | "org.wezfurlong.wezterm")
}

/// cold の最初の送信 `romaji` を `class` の窓へ送り、確認の結末までの画面の文字列を返す。
fn run(cfg: Awase, class: &str, pm: &mut PseudoMozc, romaji: &str) -> String {
    let armed = if cfg.inverted_window_rule {
        mozc_compose_check_applies(true, class)
    } else {
        legacy_applies(class)
    };
    let t0 = 0;
    if !armed || !should_check_compose(pm.read(t0), romaji) {
        pm.send(romaji, t0);
        return pm.text();
    }

    let before = pm.key_seq;
    let (sent, rest) = match (cfg.per_vk, split_first_key(romaji)) {
        (true, Some((first, rest))) => (first, rest),
        _ => (romaji, ""),
    };
    pm.send(sent, t0);

    let recover = |pm: &mut PseudoMozc, backs: usize, escape: bool| {
        if escape {
            pm.escape();
        }
        pm.backspace(backs);
        pm.send(romaji, 10_000); // 全体を再送（cold は一度だけなので落ちない）
    };

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
                SettleVerdict::Complete | SettleVerdict::Skip => break,
                SettleVerdict::PartialLiteral { missing } => {
                    if elapsed >= MOZC_COMPOSE_CHECK_MS {
                        recover(pm, missing, true);
                        return pm.text();
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
                    continue;
                }
                break;
            }
            ComposeVerdict::Literal => {
                recover(pm, compose_literal_backs(sent), false);
                return pm.text();
            }
            ComposeVerdict::Skip(_) => break,
        }
    }
    // 確認できた（または判断しない）: 残りを送る。
    pm.send(rest, now);
    pm.text()
}

// ───────────────────────── シナリオ ─────────────────────────

/// 対照: cold でも最初のキーを落とさない Mozc なら、どの設定でも `こ`。
#[test]
fn healthy_cold_types_correctly_in_every_config() {
    for cfg in [Awase::BEFORE_BUG225, Awase::BEFORE_BUG226, Awase::CURRENT] {
        let mut pm = PseudoMozc::healthy();
        assert_eq!(run(cfg, WINDOWS_TERMINAL, &mut pm, "ko"), "こ", "{cfg:?}");
        assert_eq!(pm.backspaces_issued, 0, "正常なときは何も消さない: {cfg:?}");
    }
}

/// BUG-225: 修正前は `k` が素通りして `kお` になる。
#[test]
fn bug225_reproduces_before_the_fix() {
    let mut pm = PseudoMozc::cold_dropping_first_key();
    assert_eq!(
        run(Awase::BEFORE_BUG225, WINDOWS_TERMINAL, &mut pm, "ko"),
        "kお"
    );
}

/// BUG-225: ADR-284（`key_seq` の進みによる回収）で直る。per-VK 化は無くても直る。
#[test]
fn bug225_is_fixed_by_adr284_settle() {
    let mut pm = PseudoMozc::cold_dropping_first_key();
    assert_eq!(
        run(Awase::BEFORE_BUG226, WINDOWS_TERMINAL, &mut pm, "ko"),
        "こ"
    );
    assert_eq!(pm.backspaces_issued, 1, "素通りした k の 1 文字だけ消す");
}

/// BUG-226: LINE では、ADR-284 を含む設定でも許可リストの外なので確認が張られず `kお` のまま。
#[test]
fn bug226_reproduces_in_line_with_the_allow_list() {
    let mut pm = PseudoMozc::cold_dropping_first_key();
    assert_eq!(run(Awase::BEFORE_BUG226, LINE, &mut pm, "ko"), "kお");
}

/// BUG-226: ADR-285（除外リスト + per-VK）で LINE も直る。
#[test]
fn bug226_is_fixed_by_adr285_in_line() {
    let mut pm = PseudoMozc::cold_dropping_first_key();
    assert_eq!(run(Awase::CURRENT, LINE, &mut pm, "ko"), "こ");
    assert_eq!(pm.backspaces_issued, 1);
}

/// ADR-285 の per-VK 化は、ADR-284 の settle が無くても単独で直す（素通りは最初の 1 キーだけと決まるため）。
#[test]
fn per_vk_alone_fixes_the_first_key_literal() {
    let cfg = Awase {
        settle: false,
        ..Awase::CURRENT
    };
    let mut pm = PseudoMozc::cold_dropping_first_key();
    assert_eq!(run(cfg, LINE, &mut pm, "ko"), "こ");
    assert_eq!(pm.backspaces_issued, 1);
}

/// Chrome 系は確認を張らない（誤判定の Backspace が正しい文字を消すおそれがあるため、実測まで外す）。
#[test]
fn chrome_is_left_alone_so_the_leak_is_not_recovered() {
    let mut pm = PseudoMozc::cold_dropping_first_key();
    assert_eq!(run(Awase::CURRENT, CHROME, &mut pm, "ko"), "kお");
    assert_eq!(pm.backspaces_issued, 0);
}

/// 安全側: Mozc サーバーが応答できないときはキーが握りつぶされている可能性があるので、何も消さない。
#[test]
fn swallowed_keys_never_cause_a_backspace() {
    let mut pm = PseudoMozc::new(false, false);
    let _ = run(Awase::CURRENT, LINE, &mut pm, "ko");
    assert_eq!(pm.backspaces_issued, 0);
}
