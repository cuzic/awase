//! ADR-280: lang_detect の保持と解放(`HoldController` → `INPUT_DEFER.replay_later` → Engine)が、保持せずに Engine へ流した
//! 場合と同じ出力になるかを、Engine(`awase::engine`)に合成の打鍵列を与えて検査する(親指なしの連続した文字キー)。
//!
//! - (A) 実時間: 打鍵を元の時刻(`event.timestamp`、マイクロ秒)で順に渡す。Engine が出したタイマー(`TimerEffect::Set`)は
//!   実時間で動く(次の打鍵の時刻より前に満了すれば、その打鍵の前に `on_timeout`)。
//! - (B) 保持して再生: 同じ打鍵を保持して Engine に渡さず、解放の時刻 `W`(最後の打鍵 + `extra`)に、元の時刻のまま
//!   (`event.timestamp` は元のまま)**背中合わせに**渡す(`handle_wm_drain_output_queue` が `deliver_key_event` を続けて呼ぶ。
//!   その間にタイマーは満了しない)。再生中に Engine が出したタイマーは `W` からの時間で始まり、バッチの後に満了する。
//!
//! 殻(`crates/awase-windows`、`#[cfg(windows)]`)は Linux から呼べないので、殻が `event.timestamp` を書き換えずに
//! `deliver_key_event(app, *queued_event, KeyOrigin::DeferredReplay)` へ渡すこと(`message_handlers.rs`
//! `handle_wm_drain_output_queue`)と、タイマーが OS の実時間であることを、この模型で表している。
//! 出力は、Engine が出す文字(`KeyAction::Char`/`Romaji`)と、素通し(`PassThrough`)・再注入の印の列。

use awase::config::AppConfig;
use awase::engine::{
    Decision, Effect, Engine, InputContext, InputEffect, InputModeState, NicolaFsm,
    SpecialKeyCombos, TimerEffect,
};
use awase::scanmap::{KeyboardModel, PhysicalPos};
use awase::types::{
    ImeRelevance, KeyAction, KeyClassification, KeyEventType, RawKeyEvent, ScanCode, Timestamp,
    VkCode,
};
use awase::yab::YabLayout;
use std::collections::BTreeMap;

fn make_engine() -> Engine {
    static BASE: std::sync::OnceLock<(AppConfig, YabLayout)> = std::sync::OnceLock::new();
    let (config, layout) = BASE.get_or_init(|| {
        let config = AppConfig::load(std::path::Path::new("config.toml")).unwrap();
        let content = std::fs::read_to_string("layout/nicola.yab").expect("layout");
        let layout = YabLayout::parse(&content, KeyboardModel::Jis).expect("yab");
        (config, layout)
    });
    let fsm = NicolaFsm::new(
        layout.clone(),
        VkCode(0x1D),
        VkCode(0x1C),
        config.general.simultaneous_threshold_ms,
        config.general.confirm_mode,
        config.general.speculative_delay_ms,
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
    engine.set_prev_active(true);
    engine
}

fn ctx() -> InputContext {
    InputContext {
        ime_on: true,
        input_mode: InputModeState::ObservedRomaji,
        is_japanese_ime: true,
        composing: false,
        modifiers: awase::engine::ModifierState::default(),
        left_thumb_down: None,
        right_thumb_down: None,
    }
}

/// 文字キーの物理位置(未シフト面で仮名のあるキー。行 1〜3 の左 5 列ずつ)
const POSITIONS: [(u8, u8); 12] = [
    (1, 0),
    (1, 1),
    (1, 2),
    (1, 3),
    (1, 4),
    (2, 0),
    (2, 1),
    (2, 2),
    (2, 3),
    (2, 4),
    (3, 0),
    (3, 1),
];

#[derive(Clone, Copy, Debug)]
struct Ev {
    ts: Timestamp,
    key: usize,
    down: bool,
}

fn raw(ev: &Ev) -> RawKeyEvent {
    let (r, c) = POSITIONS[ev.key];
    RawKeyEvent {
        was_down: false,
        press_id: None,
        vk_code: VkCode(0x41 + u16::try_from(ev.key).unwrap()),
        scan_code: ScanCode(0x1E),
        event_type: if ev.down {
            KeyEventType::KeyDown
        } else {
            KeyEventType::KeyUp
        },
        extra_info: 0,
        timestamp: ev.ts,
        key_classification: KeyClassification::Char,
        physical_pos: Some(PhysicalPos::new(r, c)),
        ime_relevance: ImeRelevance::default(),
        modifier_key: None,
        modifier_snapshot: Default::default(),
        left_thumb_down_snapshot: None,
        right_thumb_down_snapshot: None,
        injected: false,
        foreign_ctrl: false,
        impersonated: false,
    }
}

/// タイマー表: 論理 ID → 満了の実時刻(マイクロ秒)
#[derive(Default)]
struct Timers(BTreeMap<usize, Timestamp>);

/// 決定の効果を、出力の印の列に足し、タイマーを表に反映する。`wall` はこの決定の実時刻。
fn apply(decision: &Decision, wall: Timestamp, timers: &mut Timers, out: &mut String) {
    let effects: &[Effect] = match decision {
        Decision::PassThrough => {
            out.push_str("[pt]");
            return;
        }
        Decision::PassThroughWith { effects } => {
            out.push_str("[pt]");
            effects
        }
        Decision::Consume { effects } => effects,
    };
    for e in effects {
        match e {
            Effect::Input(InputEffect::SendKeys(actions)) => {
                for a in actions {
                    match a {
                        KeyAction::Char(c) => out.push(*c),
                        KeyAction::Romaji(s) => out.push_str(s),
                        _ => out.push_str("[k]"),
                    }
                }
                out.push('|');
            }
            Effect::Input(InputEffect::ReinjectKey(_)) => out.push_str("[re]"),
            Effect::Timer(TimerEffect::Set { id, duration }) => {
                timers
                    .0
                    .insert(*id, wall + u64::try_from(duration.as_micros()).unwrap());
            }
            Effect::Timer(TimerEffect::Kill(id)) => {
                timers.0.remove(id);
            }
            _ => {}
        }
    }
}

/// `limit` より前(含む)に満了するタイマーを、満了の早い順に発火させる。
fn fire_until(engine: &mut Engine, timers: &mut Timers, limit: Timestamp, out: &mut String) {
    loop {
        let next = timers
            .0
            .iter()
            .filter(|(_, &dl)| dl <= limit)
            .min_by_key(|(_, &dl)| dl)
            .map(|(&id, &dl)| (id, dl));
        let Some((id, dl)) = next else { return };
        timers.0.remove(&id);
        let d = engine.on_timeout(id, &ctx());
        apply(&d, dl, timers, out);
    }
}

/// (A) 実時間で渡す。
fn run_realtime(events: &[Ev]) -> String {
    let mut engine = make_engine();
    let mut timers = Timers::default();
    let mut out = String::new();
    for ev in events {
        fire_until(&mut engine, &mut timers, ev.ts, &mut out);
        let d = engine.on_input(raw(ev), &ctx());
        apply(&d, ev.ts, &mut timers, &mut out);
    }
    fire_until(&mut engine, &mut timers, Timestamp::MAX, &mut out);
    out
}

/// (B) 保持して、`wall` に背中合わせで再生する。
fn run_replay(events: &[Ev], wall: Timestamp) -> String {
    let mut engine = make_engine();
    let mut timers = Timers::default();
    let mut out = String::new();
    for ev in events {
        let d = engine.on_input(raw(ev), &ctx());
        apply(&d, wall, &mut timers, &mut out);
    }
    fire_until(&mut engine, &mut timers, Timestamp::MAX, &mut out);
    out
}

/// 出力から、文字・ローマ字だけを取り出す(`[k]` などの印と区切りは落とす)。
fn text_only(out: &str) -> String {
    out.replace("[k]", "")
        .replace("[pt]", "<pt>")
        .replace('|', "")
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// `n` 打鍵の合成列。間隔(前の Down から)・押下の長さ(重なりうる)をばらす。
fn gen_case(rng: &mut Rng, n: usize) -> Vec<Ev> {
    let mut downs = Vec::new();
    let mut t: Timestamp = 1_000_000;
    let mut used: Vec<usize> = Vec::new();
    for i in 0..n {
        if i > 0 {
            // 前の Down からの間隔: 20〜450ms(閾値 100ms の前後を多めに)
            t += [
                20_000, 40_000, 60_000, 90_000, 110_000, 150_000, 250_000, 450_000,
            ][usize::try_from(rng.below(8)).unwrap()]
                + rng.below(9_000);
        }
        let mut key = usize::try_from(rng.below(12)).unwrap();
        while used.contains(&key) {
            key = (key + 1) % 12;
        }
        used.push(key);
        // 押下の長さ: 30〜200ms
        let hold = [30_000, 60_000, 90_000, 130_000, 200_000]
            [usize::try_from(rng.below(5)).unwrap()]
            + rng.below(9_000);
        downs.push((t, key, hold));
    }
    let mut evs = Vec::new();
    for (t, key, hold) in downs {
        evs.push(Ev {
            ts: t,
            key,
            down: true,
        });
        evs.push(Ev {
            ts: t + hold,
            key,
            down: false,
        });
    }
    evs.sort_by_key(|e| (e.ts, !e.down));
    evs
}

fn describe(events: &[Ev]) -> String {
    let t0 = events[0].ts;
    events
        .iter()
        .map(|e| {
            let (r, c) = POSITIONS[e.key];
            format!(
                "{}{}({r},{c})@{}ms",
                if e.down { "v" } else { "^" },
                e.key,
                (e.ts - t0) / 1000
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// 保持して再生した出力が、実時間のものと一致しない最小の列を探す。件数と反例を返す。
#[test]
fn replay_of_held_keys_matches_realtime_engine_output() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut report = String::new();
    let mut total = 0_usize;
    let mut mismatches = 0_usize;
    let mut text_mismatches = 0_usize;
    let mut shown = 0;
    // 解放の時刻: 最後の打鍵の後 0ms / 300ms / 2000ms
    for n in 1..=5 {
        for _ in 0..1500 {
            let events = gen_case(&mut rng, n);
            let last = events.last().unwrap().ts;
            for extra in [0_u64, 300_000, 2_000_000] {
                total += 1;
                let a = run_realtime(&events);
                let b = run_replay(&events, last + extra);
                if a != b {
                    mismatches += 1;
                }
                if text_only(&a) != text_only(&b) {
                    text_mismatches += 1;
                    if shown < 14 && (shown < 4 || n >= 3) {
                        shown += 1;
                        report.push_str(&format!(
                            "n={n} extra={}ms\n  {}\n  A(実時間) {a}\n  B(再生)   {b}\n",
                            extra / 1000,
                            describe(&events)
                        ));
                    }
                }
            }
        }
    }
    assert!(
        text_mismatches == 0,
        "再生が実時間と一致しない(文字の列): {text_mismatches}/{total}(印も含めると {mismatches})\n{report}"
    );
}
