//! journal の `KeyInput` 列を HEAD の `Engine` に流す「入力の再生」の試作（journal 検討メモ B5、
//! BUG-105 の 1 件）。結果は `docs/tasks/journal-replay-rebuild-study-2026-10-06/b5-prototype-result.md`。
//!
//! - 記録側（`KeyEventSummary`）は変えない。分類は記録した `key_class` を使う（`hook::classify_key` は
//!   `#[cfg(windows)]` の `hook.rs` にあるので分類し直さない）。物理位置は `scanmap::scan_to_pos`、
//!   修飾キー種別は `VkCodeExt::classify_modifier` で導く（どちらも Linux で使える）。
//! - 記録に無い入力（InputContext の ime_on・input_mode・japanese・composing、`win` 修飾、拡張ビット、
//!   `ImeRelevance::sync_direction`）は呼び出し側が固定値で与える。
//! - タイマーは 2 通り: [`TimerOrder::Recorded`] は記録の `TimerFired` が並んだ位置（seq 順）で発火し、
//!   [`TimerOrder::VirtualClock`] は `timestamp_us` の仮想時計で期限が来たら発火する。
//!   どちらも最後の打鍵の後に残ったタイマーを期限順に発火する。
//!
//! Linux でも走る（`cargo nextest run --workspace --lib`）。

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use awase::config::AppConfig;
use awase::engine::{
    Decision, Effect, Engine, EngineCommand, InputContext, InputEffect, InputModeState, NicolaFsm,
    SpecialKeyCombos, TimerEffect, TIMER_PENDING,
};
use awase::ngram::NgramModel;
use awase::scanmap::KeyboardModel;
use awase::types::{
    ImeRelevance, KeyAction, KeyClassification, KeyEventType, ModifierState, RawKeyEvent, ScanCode,
    Timestamp, VkCode,
};
use awase::yab::YabLayout;
use serde::Deserialize;

use crate::scanmap::scan_to_pos;
use crate::state::alt_impersonation::resolve_thumb_key;
use crate::state::foreign_modifier::ForeignCtrlLatch;
use crate::tuning::FOREIGN_CTRL_TTL_MS;
use crate::vk::VkCodeExt;

// ── 再生の補助 ──────────────────────────────────────────────────────────────

/// `journal::KeyEventSummary` の JSON を読む側の写し（本体は `Serialize` だけで、`key_class` が `&'static str`）。
#[derive(Debug, Deserialize)]
struct RecordedKey {
    vk_code: u16,
    scan_code: u32,
    is_down: bool,
    injected: bool,
    timestamp_us: u64,
    key_class: String,
    alt: bool,
    ctrl: bool,
    shift: bool,
    /// 他アプリが注入した Ctrl の保持中に届いた注入打鍵か(ADR-249)。古い記録には無いので `false`。
    #[serde(default)]
    foreign_ctrl: bool,
}

/// 再生が読む `JournalEntry` の variant だけの写し。`decision`・`physical` などは読まない。
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum RecordedEntry {
    KeyInput {
        event: RecordedKey,
        state_before: String,
        state_after: String,
        repeat_count: u32,
        last_timestamp_us: u64,
    },
    TimerFired {
        timer_id: usize,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
struct RecordedEnvelope {
    seq: u64,
    entry: RecordedEntry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TimerOrder {
    Recorded,
    VirtualClock,
}

/// 1 回の `on_input`/`on_timeout`。`recorded` は記録の (state_before, state_after)。
#[derive(Debug)]
struct Step {
    what: String,
    state_before: String,
    state_after: String,
    recorded: Option<(String, String)>,
    output: String,
    /// エンジンの判定の種別(`PassThrough` / `PassThroughWith` / `Consume`)。
    decision: &'static str,
}

impl Step {
    fn diverges(&self) -> bool {
        self.recorded.as_ref().is_some_and(|(before, after)| {
            *before != self.state_before || *after != self.state_after
        })
    }
}

/// 本番の `app/bootstrap.rs`（Windows 専用）のうち、同時打鍵の判定に効く設定だけを写す:
/// 配列・親指キー・閾値・`confirm_mode`・`speculative_delay_ms`・タイミングマージン・n-gram。
/// `keys.*`（特殊キー）・親指の単独タップ設定・Space/Enter 親指は写していない。
fn build_engine(config: &AppConfig, layout_yab: &str, ngram_file: Option<&Path>) -> Engine {
    let general = &config.general;
    let layout = YabLayout::parse(layout_yab, general.keyboard_model).expect("layout");
    let (left_thumb, _) = resolve_thumb_key(&general.left_thumb_key).expect("left thumb");
    let (right_thumb, _) = resolve_thumb_key(&general.right_thumb_key).expect("right thumb");
    let fsm = NicolaFsm::new(
        layout,
        left_thumb,
        right_thumb,
        general.simultaneous_threshold_ms,
        general.confirm_mode,
        general.speculative_delay_ms,
    );
    let mut engine = Engine::new(
        fsm,
        SpecialKeyCombos {
            engine_on: vec![],
            engine_off: vec![],
            ime_on: vec![],
            ime_off: vec![],
            ime_toggle: vec![],
        },
    );
    engine.apply_general_config(general);
    engine.set_prev_active(true);
    if let Some(path) = ngram_file {
        let to_us = |ms: u32| u64::from(ms) * 1000;
        let model = NgramModel::from_file(
            path,
            to_us(general.ngram_adjustment_range_ms),
            to_us(general.ngram_min_threshold_ms),
            to_us(general.ngram_max_threshold_ms),
        )
        .expect("ngram");
        let _ = engine.on_command(EngineCommand::SetNgramModel(model), &ime_on_ctx());
    }
    engine
}

/// 記録に無い InputContext の 4 項目の固定値（IME ON・ローマ字・日本語 IME・未変換）。
fn ime_on_ctx() -> InputContext {
    InputContext {
        ime_on: true,
        input_mode: InputModeState::ObservedRomaji,
        is_japanese_ime: true,
        composing: false,
        modifiers: ModifierState::default(),
        left_thumb_down: None,
        right_thumb_down: None,
    }
}

fn decision_kind(decision: &Decision) -> &'static str {
    match decision {
        Decision::PassThrough => "PassThrough",
        Decision::PassThroughWith { .. } => "PassThroughWith",
        Decision::Consume { .. } => "Consume",
    }
}

struct Replay {
    engine: Engine,
    model: KeyboardModel,
    base_ctx: InputContext,
    /// 張っているタイマー（id → 期限、`timestamp_us` の系）。
    timers: BTreeMap<usize, Timestamp>,
    now: Timestamp,
    held: HashSet<u16>,
    /// 左右の親指の押下時刻（`KeyInput` の並びから導く）。
    thumbs: [Option<Timestamp>; 2],
    modifiers: ModifierState,
    steps: Vec<Step>,
}

impl Replay {
    fn run(journal_json: &str, engine: Engine, model: KeyboardModel, order: TimerOrder) -> Self {
        let mut envelopes: Vec<RecordedEnvelope> =
            serde_json::from_str(journal_json).expect("journal json");
        envelopes.sort_by_key(|e| e.seq);
        let mut replay = Self {
            engine,
            model,
            base_ctx: ime_on_ctx(),
            timers: BTreeMap::new(),
            now: 0,
            held: HashSet::new(),
            thumbs: [None, None],
            modifiers: ModifierState::default(),
            steps: Vec::new(),
        };
        for envelope in envelopes {
            match envelope.entry {
                RecordedEntry::KeyInput {
                    event,
                    state_before,
                    state_after,
                    repeat_count,
                    last_timestamp_us,
                } => {
                    // 畳み込まれたオートリピートは等間隔に広げる（間の時刻は記録に無い）。
                    let count = repeat_count.max(1);
                    for i in 0..count {
                        let ts = event.timestamp_us
                            + last_timestamp_us.saturating_sub(event.timestamp_us) * u64::from(i)
                                / u64::from((count - 1).max(1));
                        if order == TimerOrder::VirtualClock {
                            replay.fire_due(ts);
                        }
                        let recorded =
                            (i == 0).then(|| (state_before.clone(), state_after.clone()));
                        replay.key(&event, ts, recorded);
                    }
                }
                // HEAD が張っていないタイマーの記録は飛ばす。
                RecordedEntry::TimerFired { timer_id } if order == TimerOrder::Recorded => {
                    if let Some(deadline) = replay.timers.get(&timer_id).copied() {
                        replay.now = replay.now.max(deadline);
                        replay.fire(timer_id);
                    }
                }
                RecordedEntry::TimerFired { .. } | RecordedEntry::Other => {}
            }
        }
        replay.fire_due(Timestamp::MAX);
        replay
    }

    fn ctx(&self) -> InputContext {
        InputContext {
            modifiers: self.modifiers,
            left_thumb_down: self.thumbs[0],
            right_thumb_down: self.thumbs[1],
            ..self.base_ctx
        }
    }

    fn to_raw(&mut self, key: &RecordedKey, ts: Timestamp) -> RawKeyEvent {
        let vk = VkCode(key.vk_code);
        let class = match key.key_class.as_str() {
            "Char" => KeyClassification::Char,
            "LeftThumb" => KeyClassification::LeftThumb,
            "RightThumb" => KeyClassification::RightThumb,
            "Passthrough" => KeyClassification::Passthrough,
            other => panic!("unknown key_class {other}"),
        };
        let was_down = if key.is_down {
            !self.held.insert(key.vk_code)
        } else {
            self.held.remove(&key.vk_code)
        };
        let side = match class {
            KeyClassification::LeftThumb => Some(0),
            KeyClassification::RightThumb => Some(1),
            _ => None,
        };
        if let Some(side) = side {
            self.thumbs[side] = if key.is_down {
                self.thumbs[side].or(Some(ts))
            } else {
                None
            };
        }
        self.modifiers = ModifierState {
            ctrl: key.ctrl,
            alt: key.alt,
            shift: key.shift,
            win: false,
        };
        let scan_code = ScanCode(key.scan_code);
        RawKeyEvent {
            vk_code: vk,
            scan_code,
            event_type: if key.is_down {
                KeyEventType::KeyDown
            } else {
                KeyEventType::KeyUp
            },
            extra_info: 0,
            timestamp: ts,
            key_classification: class,
            physical_pos: if class == KeyClassification::Char {
                scan_to_pos(self.model, scan_code)
            } else {
                None
            },
            ime_relevance: ImeRelevance::default(),
            modifier_key: vk.classify_modifier(),
            modifier_snapshot: self.modifiers,
            left_thumb_down_snapshot: self.thumbs[0],
            right_thumb_down_snapshot: self.thumbs[1],
            injected: key.injected,
            was_down,
            press_id: None,
            foreign_ctrl: key.foreign_ctrl,
        }
    }

    fn key(&mut self, key: &RecordedKey, ts: Timestamp, recorded: Option<(String, String)>) {
        self.now = ts;
        let event = self.to_raw(key, ts);
        let state_before = self.engine.debug_state_label();
        let decision = self.engine.on_input(event, &self.ctx());
        let output = self.apply(&decision);
        self.steps.push(Step {
            what: format!(
                "key 0x{:02X} {}",
                key.vk_code,
                if key.is_down { "down" } else { "up" }
            ),
            state_before,
            state_after: self.engine.debug_state_label(),
            recorded,
            output,
            decision: decision_kind(&decision),
        });
    }

    fn fire(&mut self, timer_id: usize) {
        self.timers.remove(&timer_id);
        let state_before = self.engine.debug_state_label();
        let decision = self.engine.on_timeout(timer_id, &self.ctx());
        let output = self.apply(&decision);
        self.steps.push(Step {
            what: format!("timer {timer_id}"),
            state_before,
            state_after: self.engine.debug_state_label(),
            recorded: None,
            output,
            decision: decision_kind(&decision),
        });
    }

    /// 期限が `until` 以下のタイマーを期限順に発火する（発火中に張られたものも含む）。
    fn fire_due(&mut self, until: Timestamp) {
        while let Some((timer_id, deadline)) = self.next_timer() {
            if deadline > until {
                break;
            }
            self.now = deadline;
            self.fire(timer_id);
        }
    }

    fn next_timer(&self) -> Option<(usize, Timestamp)> {
        self.timers
            .iter()
            .min_by_key(|(_, deadline)| **deadline)
            .map(|(&timer_id, &deadline)| (timer_id, deadline))
    }

    /// タイマーの張り替えを反映し、送る文字（`Char`/`Romaji`）を連結して返す。
    fn apply(&mut self, decision: &Decision) -> String {
        let effects = match decision {
            Decision::PassThrough => return String::new(),
            Decision::PassThroughWith { effects } | Decision::Consume { effects } => effects,
        };
        let mut output = String::new();
        for effect in effects {
            match effect {
                Effect::Timer(TimerEffect::Set { id, duration }) => {
                    let duration_us = u64::try_from(duration.as_micros()).unwrap_or(u64::MAX);
                    self.timers
                        .insert(*id, self.now.saturating_add(duration_us));
                }
                Effect::Timer(TimerEffect::Kill(id)) => {
                    self.timers.remove(id);
                }
                Effect::Input(InputEffect::SendKeys(actions)) => {
                    for action in actions {
                        match action {
                            KeyAction::Char(ch) => output.push(*ch),
                            KeyAction::Romaji(romaji) => output.push_str(romaji),
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        output
    }

    fn output(&self) -> String {
        self.steps.iter().map(|s| s.output.as_str()).collect()
    }

    fn first_divergence(&self) -> Option<&Step> {
        self.steps.iter().find(|s| s.diverges())
    }

    fn step(&self, what: &str) -> &Step {
        self.steps
            .iter()
            .find(|s| s.what == what)
            .unwrap_or_else(|| panic!("no step {what}: {:#?}", self.steps))
    }
}

// ── BUG-105 ─────────────────────────────────────────────────────────────────

/// BUG-105 の既存テスト（`tests/scenarios.rs::scenario_3key_char1_released_tight_overlap_prefers_chord`、
/// `src/engine/tests.rs::test_three_key_char1_released_tight_d1_still_prefers_char1`）の入力列
/// （L↓・右親指↓ d1=11.7ms・L↑・A↓ d2=100.8ms）を `KeyInput` の列に戻したもの。時刻は報告の
/// `[engine-input]` の行（`BUG-105.md`）に合わせ、`state_*` は報告時（修正前）の記録を模している
/// （最後の A↓ の `state_after` が修正前の `Idle`）。
const BUG_105_FIXTURE: &str = "tests/journals/key_input/bug-105-tight-d1.json";

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn replay_bug_105(order: TimerOrder) -> Replay {
    let root = repo_root();
    let config = AppConfig::load(&root.join("config.toml")).expect("config.toml");
    let layout = std::fs::read_to_string(root.join("layout/nicola.yab")).expect("nicola.yab");
    let ngram_file = config.general.ngram_file.as_ref().map(|p| root.join(p));
    let journal =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(BUG_105_FIXTURE))
            .expect("fixture");
    let engine = build_engine(&config, &layout, ngram_file.as_deref());
    Replay::run(&journal, engine, config.general.keyboard_model, order)
}

const A_DOWN: &str = "key 0x41 down";

#[test]
fn bug_105_replay_in_recorded_timer_order_matches_engine_test() {
    let replay = replay_bug_105(TimerOrder::Recorded);

    // 既存テストと同じ: A↓ で L+右親指 が「ょ」(lyo) に確定し、A はタイムアウトで「う」(u)。
    assert_eq!(replay.output(), "lyou", "{:#?}", replay.steps);
    assert_eq!(replay.step(A_DOWN).output, "lyo", "{:#?}", replay.steps);

    // 記録（修正前）と最初に食い違うのは A↓ の後の状態: 修正前は L 単独+A+右親指 を出して Idle、
    // HEAD は L+右親指 を確定して A を保留する。
    let divergence = replay
        .first_divergence()
        .expect("HEAD must diverge from the pre-fix record");
    assert_eq!(divergence.what, A_DOWN, "{:#?}", replay.steps);
    assert_eq!(divergence.state_after, "PendingChar(vk=0x41)");
}

#[test]
fn bug_105_replay_on_virtual_clock_times_out_before_char2() {
    let replay = replay_bug_105(TimerOrder::VirtualClock);

    // 仮想時計では、右親指↓で張り直した TIMER_PENDING の期限（d1+100ms=111.7ms）が A↓（112.474ms）
    // より先に来るので、A↓ の前にタイムアウトで L+右親指 が確定し、3 鍵の仲裁を通らない。
    // 出力は同じ「lyou」になり、修正前のコードでも同じ結果になる（BUG-105 を検出できない）。
    // 実機では WM_TIMER の遅れで A↓ が先に処理された（報告の `Timer killed: logical=1`）。
    assert_eq!(replay.output(), "lyou", "{:#?}", replay.steps);
    let pending_timer = format!("timer {TIMER_PENDING}");
    let timer_pos = replay
        .steps
        .iter()
        .position(|s| s.what == pending_timer)
        .expect("timer fired");
    let a_pos = replay
        .steps
        .iter()
        .position(|s| s.what == A_DOWN)
        .expect("A down");
    assert!(timer_pos < a_pos, "{:#?}", replay.steps);
    assert_eq!(replay.step(A_DOWN).state_before, "Idle");
    assert_eq!(
        replay.first_divergence().map(|s| s.what.as_str()),
        Some(A_DOWN),
        "{:#?}",
        replay.steps
    );
}

// ── BUG-197(ADR-251): 他アプリが注入した Ctrl+V ────────────────────────────────

/// ADR-251 決定1 の最小形。報告 `01M4J72G985T0FFT6XPN0SWCQQ` の `KeyInput` 列から、各打鍵の
/// `{t_us, vk, scan, event_type, injected, alt, shift, ctrl}` だけを**記録のまま**残したもの
/// (記録時は `foreign_ctrl` 自体が無く、注入 V の `ctrl` は false)。
#[derive(Debug, Deserialize)]
struct ForeignCtrlFixture {
    source: String,
    keys: Vec<ForeignCtrlKey>,
}

#[derive(Debug, Deserialize)]
struct ForeignCtrlKey {
    t_us: u64,
    vk: u16,
    scan: u32,
    event_type: String,
    injected: bool,
    alt: bool,
    shift: bool,
    ctrl: bool,
}

/// 最小形を `Replay::run` が読む journal の形へ直す(wire が変わったとき直すのはここだけ)。
///
/// `through_latch` が true のときは、`hook.rs::hook_callback` が `modifier_snapshot` を作る順序
/// (注入の Down は `on_injected_down`、Up は `on_up`、その後に注入キーなら `ctrl_for_injected_key`)を
/// 記録の時刻順に `ForeignCtrlLatch` へ流し、注入打鍵の `ctrl`/`foreign_ctrl` を求める。
/// false のときは記録の `ctrl` のまま(ラッチを通さない = ADR-249 の修正前)。
/// hook.rs の配線(この順序で呼ぶこと自体)は再生しない。`architecture_guard` が固定している。
fn foreign_ctrl_journal(name: &str, through_latch: bool) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/journals/key_input")
        .join(name);
    let fixture: ForeignCtrlFixture =
        serde_json::from_str(&std::fs::read_to_string(path).expect("fixture")).expect("json");
    assert!(!fixture.source.is_empty());
    let latch = ForeignCtrlLatch::new();
    let ttl_us = FOREIGN_CTRL_TTL_MS * 1_000;
    let rows: Vec<serde_json::Value> = fixture
        .keys
        .iter()
        .enumerate()
        .map(|(i, k)| {
            let is_down = k.event_type == "down";
            let vk = VkCode(k.vk);
            let mut foreign_ctrl = false;
            if through_latch && k.injected {
                if is_down {
                    latch.on_injected_down(vk, k.t_us, ttl_us);
                } else {
                    latch.on_up(vk);
                }
                foreign_ctrl = !k.ctrl && latch.ctrl_for_injected_key(k.t_us, ttl_us);
            }
            serde_json::json!({
                "seq": i + 1,
                "entry": {
                    "type": "KeyInput",
                    "event": {
                        "vk_code": k.vk,
                        "scan_code": k.scan,
                        "is_down": is_down,
                        "injected": k.injected,
                        "timestamp_us": k.t_us,
                        "key_class": if k.vk == 0x56 { "Char" } else { "Passthrough" },
                        "alt": k.alt,
                        "ctrl": k.ctrl || foreign_ctrl,
                        "shift": k.shift,
                        "foreign_ctrl": foreign_ctrl,
                    },
                    "state_before": "",
                    "state_after": "",
                    "repeat_count": 1,
                    "last_timestamp_us": k.t_us,
                },
            })
        })
        .collect();
    serde_json::to_string(&rows).expect("journal")
}

fn replay_foreign_ctrl(name: &str, through_latch: bool) -> Replay {
    let root = repo_root();
    let config = AppConfig::load(&root.join("config.toml")).expect("config.toml");
    let layout = std::fs::read_to_string(root.join("layout/nicola.yab")).expect("nicola.yab");
    let engine = build_engine(&config, &layout, None);
    let journal = foreign_ctrl_journal(name, through_latch);
    Replay::run(
        &journal,
        engine,
        config.general.keyboard_model,
        TimerOrder::Recorded,
    )
}

const BUG_197_FIXTURES: [&str; 2] = [
    "bug-197-foreign-ctrl-paste-01.json",
    "bug-197-foreign-ctrl-paste-02.json",
];
const V_DOWN: &str = "key 0x56 down";
const V_UP: &str = "key 0x56 up";

/// 記録の列を `ForeignCtrlLatch`(TTL は本番の `FOREIGN_CTRL_TTL_MS`)に通すと、注入 V が `ctrl=true` で
/// エンジンに届き、V↓/V↑ とも PassThrough になる。ラッチの記録・期限・解除や TTL を壊すと落ちる。
#[test]
fn bug_197_foreign_ctrl_v_passes_through_on_reported_sequence() {
    for name in BUG_197_FIXTURES {
        let replay = replay_foreign_ctrl(name, true);
        let down = replay.step(V_DOWN);
        assert_eq!(down.decision, "PassThrough", "{name}: {:#?}", replay.steps);
        assert_eq!(down.state_after, "Idle", "{name}");
        // V↑ は Suppress されない(V↓ が OS に届いているので、↑ も届かないと V が固着する)。
        assert_eq!(replay.step(V_UP).decision, "PassThrough", "{name}");
        assert_eq!(replay.output(), "", "{name}");
    }
}

/// 対照: ラッチを通さず記録のまま(V は `ctrl=false`)流すと、V↓ は `PendingChar` に入り V↑ は Consume され、
/// 「ふ」(`fu`)が出る(修正前の挙動、BUG-197)。これが落ちない fixture は上のテストの意味を担保しない。
#[test]
fn bug_197_control_without_latch_enters_pending_char() {
    for name in BUG_197_FIXTURES {
        let replay = replay_foreign_ctrl(name, false);
        let down = replay.step(V_DOWN);
        assert_eq!(down.decision, "Consume", "{name}: {:#?}", replay.steps);
        assert_eq!(down.state_after, "PendingChar(vk=0x56)", "{name}");
        assert_eq!(replay.step(V_UP).decision, "Consume", "{name}");
        assert_eq!(replay.output(), "fu", "{name}: {:#?}", replay.steps);
    }
}
