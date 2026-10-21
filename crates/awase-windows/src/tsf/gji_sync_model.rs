//! `GjiFsm` への同期の閉ループ検証(Linux だけで完結する。ECRS DD5-01、ADR-203 決定 4)。
//!
//! # 何を確かめるか
//!
//! `GjiFsm` が `OffCold` のまま belief だけが ON になると、最初の数文字が握りつぶされる(BUG-18・BUG-170)。
//! これを防ぐ同期の入口が複数ある:
//!
//! - **A1** フォーカス復帰時の `presync_applied_open_on`(`runtime/focus_tracking.rs`)
//! - **G1** IME 種別検出時の `sync_ime_kind_from_observation`(`runtime/message_handlers.rs`)
//! - **G2** 送信直前の level 突合 `needs_belief_sync_on`(`platform.rs::send_keys`)
//!
//! A1・G1 が G2 と二重かどうかを、実機 CI を使わずに**厳密に**決めた(ADR-276。Unicode 注入が無くなり、
//! 注入はすべて VK になったので、G2 は全送信で働く)。結果は「二重」で、A1・G1 は撤去した。`FULL` は撤去前の
//! 配線、`NO_A1_G1` が現在の配線である。このモデルは、撤去が害を増やさなかったことの記録として残す。
//!
//! # 方法(2 段)
//!
//! 1. **抽象の検証**: 害は「送信時に `GjiFsm` が `OffCold` か」だけで決まる。`OffCold` から出る遷移は `ImeOn`・`Reopen`
//!    だけ、`OffCold` へ入る遷移は `ImeOff` だけ、という 2 状態の抽象が実物の `GjiFsm` と一致することを、イベント列の
//!    全列挙(長さ 6 まで)で確かめる。
//! 2. **到達可能性の厳密な列挙**: 抽象の状態は 48 個ほどしかないので、列の長さを区切らず**到達可能な状態をすべて**
//!    列挙し、同期の入口の有無(変種)ごとに「害が起きる状態」の集合を比べる。
//!
//! 殻の配線(どのイベントがどの入口を呼ぶか)はこのモデルが写している。配線が変わったらここも直すこと
//! (`architecture_guard::gji_fsm_sync_entry_points_are_accounted_for` が入口の数を固定している)。

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use timed_fsm::TimedStateMachine;

use crate::state::injection_mode::InjectionMode;
use crate::tsf::gji_fsm::{GjiEvent, GjiFsm, GjiState, GjiTimer, PendingInput};

// ── 1. 抽象の検証 ───────────────────────────────────────────────────────────

const REAL_EVENT_KINDS: usize = 8;

fn run_real(fsm: &mut GjiFsm, kind: usize) {
    match kind {
        0 => {
            let _ = fsm.on_event(GjiEvent::ImeOn {
                injection_mode: InjectionMode::Vk,
                gji_idle_ms: 0,
            });
        }
        1 => {
            let _ = fsm.on_event(GjiEvent::Reopen {
                injection_mode: InjectionMode::Vk,
                gji_idle_ms: 0,
            });
        }
        2 => {
            let _ = fsm.on_event(GjiEvent::ImeOff);
        }
        3 => {
            let _ = fsm.on_event(GjiEvent::FocusChange {
                injection_mode: InjectionMode::Vk,
                gji_idle_ms: 0,
            });
        }
        4 => {
            let _ = fsm.on_event(GjiEvent::KeyInput(PendingInput::new("k")));
        }
        5 => {
            let _ = fsm.on_event(GjiEvent::CompositionReset { gji_idle_ms: 0 });
        }
        6 => {
            let _ = fsm.on_event(GjiEvent::NativeF2Consumed { gji_idle_ms: 0 });
        }
        _ => {
            let _ = fsm.on_timeout(GjiTimer::LongIdle);
        }
    }
}

fn is_off(fsm: &GjiFsm) -> bool {
    matches!(fsm.state(), GjiState::OffCold)
}

/// 抽象: `OffCold` から出るのは `ImeOn`(0)・`Reopen`(1) だけ、`OffCold` へ入るのは `ImeOff`(2) だけ。
fn abstract_off_after(off: bool, kind: usize) -> bool {
    match (off, kind) {
        (true, 0 | 1) => false,
        (false, 2) => true,
        (o, _) => o,
    }
}

/// 実物の `GjiFsm` の `OffCold` 性が、上の 2 状態の抽象に全イベント列(長さ 6 まで。`StartComposition`/`EndComposition` は `OffCold` の出入りに関わらないので除く)で従う。
#[test]
fn off_cold_abstraction_matches_the_real_fsm_for_every_event_sequence() {
    fn rec(seq: &mut Vec<usize>, depth: usize, checked: &mut u64) {
        // この列を実物へ再生し、各段で抽象と一致するか見る。
        let mut fsm = GjiFsm::new();
        let mut abs = true;
        for (i, &k) in seq.iter().enumerate() {
            run_real(&mut fsm, k);
            abs = abstract_off_after(abs, k);
            assert_eq!(
                is_off(&fsm),
                abs,
                "抽象とずれた: seq[..={i}]={:?}",
                &seq[..=i]
            );
        }
        *checked += 1;
        if depth == 0 {
            return;
        }
        for k in 0..REAL_EVENT_KINDS {
            seq.push(k);
            rec(seq, depth - 1, checked);
            seq.pop();
        }
    }
    let mut checked = 0u64;
    rec(&mut Vec::new(), 6, &mut checked);
    assert!(checked > 200_000, "列挙が空振りしている: {checked}");
}

// ── 2. 到達可能性の厳密な列挙 ───────────────────────────────────────────────

/// 殻から見た世界の状態(抽象)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct World {
    /// awase の belief(エンジンが ON と見ている)。
    belief: bool,
    /// 実 IME の開閉。
    real: bool,
    /// `applied`(awase が書いた/確認した開閉。`None` = 未知)。A1・G1 の条件に使う。
    applied_open: Option<bool>,
    /// IME 種別が GJI と検出済みか。
    kind_gji: bool,
    /// `GjiFsm` が `OffCold` か。
    off: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Ev {
    /// awase が IME へ ON を書いた(`on_ime_applied(open=true)` → `ImeOn`)。
    ActOn,
    /// awase が IME へ OFF を書いた(`on_ime_applied(false)` → `ImeOff`)。
    ActOff,
    /// 物理キーの予測で ON(`kp_reopen_gji_fsm` → `Reopen`)。
    PredOn,
    /// 物理キーの予測で OFF(`GjiFsm` へは通知されない)。
    PredOff,
    /// awase の知らないところで実 IME が閉じた。
    ExtOff,
    /// awase の知らないところで実 IME が開いた。
    ExtOn,
    /// 観測が追いつき belief が実 IME に揃う(`GjiFsm` へは通知されない)。
    ObsSync,
    /// フォーカス復帰で hwnd cache が belief を ON に戻す(BUG-18)。A1 の入口。
    FocusRestore,
    /// 普通のフォーカス変更(`GjiEvent::FocusChange`。`OffCold` では状態が変わらない)。
    Focus,
    /// IME 種別が GJI と検出された(`sync_ime_kind_from_observation`)。G1 の入口。
    KindDetect,
    /// 種別が GJI でなくなった(別の窓・未検出)。
    KindLost,
    /// エンジンがローマ字を IME へ送る(belief が ON のときだけ)。G2 の入口。
    Send,
}

const ALL_EVS: [Ev; 12] = [
    Ev::ActOn,
    Ev::ActOff,
    Ev::PredOn,
    Ev::PredOff,
    Ev::ExtOff,
    Ev::ExtOn,
    Ev::ObsSync,
    Ev::FocusRestore,
    Ev::Focus,
    Ev::KindDetect,
    Ev::KindLost,
    Ev::Send,
];

/// 同期の入口の有無。
#[derive(Debug, Clone, Copy)]
struct Variant {
    a1: bool,
    g1: bool,
    g2: bool,
}

impl Variant {
    const FULL: Self = Self {
        a1: true,
        g1: true,
        g2: true,
    };
    /// 現在の配線(ADR-276 で A1・G1 を撤去した。ablation a12 と同じ)。
    const NO_A1_G1: Self = Self {
        a1: false,
        g1: false,
        g2: true,
    };
}

/// `step` の結果。`harm` は `Send` で `GjiFsm` が `OffCold` のまま(=最初の文字が握りつぶされうる)。
struct Step {
    next: World,
    harm: bool,
}

fn step(w: World, ev: Ev, v: Variant) -> Option<Step> {
    let mut n = w;
    let mut harm = false;
    match ev {
        Ev::ActOn => {
            n.real = true;
            n.belief = true;
            n.applied_open = Some(true);
            n.off = false; // ImeOn
        }
        Ev::ActOff => {
            n.real = false;
            n.belief = false;
            n.applied_open = Some(false);
            n.off = true; // ImeOff
        }
        Ev::PredOn => {
            n.real = true;
            n.belief = true;
            n.off = false; // Reopen(Predict)
        }
        Ev::PredOff => {
            n.real = false;
            n.belief = false;
        }
        Ev::ExtOff => n.real = false,
        Ev::ExtOn => n.real = true,
        Ev::ObsSync => n.belief = w.real,
        Ev::FocusRestore => {
            n.belief = true;
            n.applied_open = Some(true); // presync_applied_open_on の record_confirmed(true)
            if v.a1 && w.kind_gji {
                n.off = false; // A1: gji_on_ime_on
            }
        }
        Ev::Focus => {}
        Ev::KindDetect => {
            n.kind_gji = true;
            if v.g1 && w.applied_open == Some(true) {
                n.off = false; // G1: gji_on_ime_on
            }
        }
        Ev::KindLost => n.kind_gji = false,
        Ev::Send => {
            if !w.belief {
                return None; // エンジンは belief が ON のときだけ送る
            }
            // G2: needs_belief_sync_on(send_has_romaji=true, strategy_is_gji_fsm=kind_gji,
            //                          gji_is_off_cold, probe_or_recovery_blocking=false)
            let g2_applies = v.g2 && w.kind_gji && w.off;
            if g2_applies {
                n.off = false; // OnImeOnBelief
            }
            harm = n.kind_gji && n.off;
        }
    }
    Some(Step { next: n, harm })
}

/// 初期状態: 起動直後(`GjiFsm` は `OffCold`、種別は未検出、applied は未知)。belief は ON/OFF 両方。
fn initial_worlds() -> Vec<World> {
    [false, true]
        .into_iter()
        .map(|b| World {
            belief: b,
            real: b,
            applied_open: None,
            kind_gji: false,
            off: true,
        })
        .collect()
}

/// 到達可能な状態をすべて列挙し、害が起きる (状態, 事象) と、そこへ至る最短の事象列を返す。
fn explore(v: Variant) -> (BTreeSet<World>, BTreeMap<World, Vec<Ev>>) {
    let mut seen: BTreeSet<World> = BTreeSet::new();
    let mut parent: BTreeMap<World, (World, Ev)> = BTreeMap::new();
    let mut queue: VecDeque<World> = VecDeque::new();
    for w in initial_worlds() {
        seen.insert(w);
        queue.push_back(w);
    }
    // 害の起きる「Send 直前の状態」→ そこへの最短の事象列(Send を含む)
    let mut harmful: BTreeMap<World, Vec<Ev>> = BTreeMap::new();
    let path_to = |parent: &BTreeMap<World, (World, Ev)>, mut w: World| -> Vec<Ev> {
        let mut path = Vec::new();
        while let Some(&(p, e)) = parent.get(&w) {
            path.push(e);
            w = p;
        }
        path.reverse();
        path
    };
    while let Some(w) = queue.pop_front() {
        for ev in ALL_EVS {
            let Some(s) = step(w, ev, v) else {
                continue;
            };
            if s.harm {
                harmful.entry(w).or_insert_with(|| {
                    let mut p = path_to(&parent, w);
                    p.push(ev);
                    p
                });
            }
            if seen.insert(s.next) {
                parent.insert(s.next, (w, ev));
                queue.push_back(s.next);
            }
        }
    }
    (seen, harmful)
}

fn fmt_path(path: &[Ev]) -> String {
    path.iter()
        .map(|e| format!("{e:?}"))
        .collect::<Vec<_>>()
        .join(" → ")
}

/// A1・G1 を外しても、害のある状態は 1 つも増えない(G2 が全部拾う)。ADR-276 の撤去の根拠。
#[test]
fn a1_g1_are_redundant_given_g2() {
    let (_, full) = explore(Variant::FULL);
    let (_, no_a1g1) = explore(Variant::NO_A1_G1);
    assert!(
        no_a1g1.is_empty(),
        "A1・G1 を外すと害が出る状態がある(G2 では拾えない): {:?}",
        no_a1g1.values().map(|p| fmt_path(p)).collect::<Vec<_>>()
    );
    assert!(full.is_empty(), "撤去前の配線も害ゼロのはず");
}

/// 状態空間が小さい(厳密な列挙が現実的)ことを固定する。増えたらモデルの見直し時。
#[test]
fn state_space_is_small() {
    let (seen, _) = explore(Variant::FULL);
    assert!(seen.len() <= 128, "到達可能な状態が多い: {}", seen.len());
}
