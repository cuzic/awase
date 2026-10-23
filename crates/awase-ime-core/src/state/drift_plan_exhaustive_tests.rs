//! ME-3(`docs/tasks/mech-ecrs-2026-10-21/INDEX.md`): [`decide_drift_plan`] の**決定表の全列挙 golden**。
//!
//! 撤去・統合の PR の前後で「drift correction の計画が 1 つも変わらない」ことを、数秒の Linux テストで
//! 判定するための道具(`crates/awase-windows/src/state/sync_actuation_exhaustive.rs`〈#808〉と同じ方式)。
//! 入力の組を列挙し、出力の全文を `tests/golden/drift_plan_table.txt` に固定する。
//!
//! 列挙する軸(`DriftFacts` の全フィールド。値は判断の境界だけを取る):
//! - **前段(稼働条件 → ずれ → settle の順序)**: `engine_enabled` 2 × `japanese_ime` 2 ×
//!   `drift`(`NoDrift` 5 + `Ok` 4〈desired 2 × duration 2〉) × `settling` 2 = 72 通り。後段の材料は既定値に固定
//!   (前段で返るときは後段の材料を読まない、`DriftFacts` の doc)。
//! - **後段(試行の解決 → 授権 → 診断 → 方針ごとの打ち切り/収束/送信)**: 稼働・ずれあり・settle なしで、
//!   desired 2 × duration 2 × `active` 10 × `default_policy` 2 × `imm_cross`・`warrant_would_block`・
//!   `diag_already_notified`・`converged`・`fresh_evidence_after_giveup` の 5 ビットのうち、**殻が作る組だけ**の 792 通り。
//!
//! 殻(`runtime/ime_refresh.rs::ir_observe_drift_facts`)が作らない組は列挙しない(`plan_core` の (injected, toggled)=(T,T)
//! を除くのと同じ方針。契約外の組を golden に固定すると、到達できる挙動を変えない統合でも golden が落ち、偽の警告になる)。
//! 殻の約束(`produced_by_shell`): `warrant_would_block` は `imm_cross` のときだけ真になりうる。`converged` は解決した試行
//! (`resolve_actuation`)の方針が `Read` のときだけ、`fresh_evidence_after_giveup` は `Blind` で `gave_up_at` があるときだけ読む。
//!
//! 列挙しない(1 値に固定する)軸と理由:
//! - `DriftCorrection.observed`・`source`・`confidence`: `decide_drift_plan` は読まず、`DriftAct.drift` /
//!   `DeferToSettle.drift` に写すだけ(写しが入力と同じことは各行の `drift=in` で固定する)。observed は
//!   `!desired`(ずれがある状態の唯一の形)、source は `ObserverPoll`、confidence は `Medium`。
//! - `duration_ms`: 診断の閾値 `DRIFT_CORRECTION_BLIND_REARM_COOLDOWN_MS`(cd)の直前 `cd-1` と境界 `cd` だけ。
//! - `Blind.max_attempts`: 3 に固定し、`attempts` は境界の直前 `max-1` と境界 `max` だけ(`decide_action` は `>=`)。
//! - 時刻: `now` を基準に、試行の `sent_at` は `now-10s`、`gave_up_at` は再武装クールダウンの直前 `now-(cd-1)` と
//!   境界 `now-cd` だけ(`blind_rearm_cooldown_elapsed` は `>=`)。
//! - `Read.source`: `ObserverPoll` だけ(`decide_drift_plan` は方針の種類しか見ない)。
//! - 試行の世代(`origin.epoch`): 殻と同じく送信ごとに進む値(`attempts` と同じ)にする。再利用した試行の `origin` を作り直す
//!   変異(INITIAL に戻る)を検出するため、INITIAL 以外の世代を含める。
//!
//! 後段の 5 ビット(`imm_cross`・`warrant_would_block`・`diag_already_notified`・`converged`・
//! `fresh_evidence_after_giveup`)は、同じ結果になるビット列をまとめて 1 行に書く
//! (まとめ方は結果から機械的に決まり、情報は落とさない。その組の殻が作るビット列すべてが同じ結果なら `*`)。
//!
//! tuning の定数(`DRIFT_CORRECTION_BLIND_REARM_COOLDOWN_MS`)だけを変える PR では、見出しの `cd_ms` の 1 行だけが変わる
//! (行はラベル `cd`・`now-cd` で書く)。
//! enum に variant が増えると下の `match` がコンパイルエラーになる(列挙の取りこぼし防止)。
//!
//! 再生成: `UPDATE_GOLDEN=1 cargo test -p awase-ime-core --lib drift_plan::exhaustive`
//! (ローカルで走らせない運用では、CI の失敗ログの `----- BEGIN drift_plan_table -----` から
//! `----- END drift_plan_table -----` までの間を golden に写す。)

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::{
    decide_drift_plan, resolve_actuation, ActuationSnapshot, DriftAct, DriftFacts, DriftIdle,
    DriftPlan, DriftStep, GiveUpPark, SendPath,
};
use crate::state::drift_correction::{DriftCorrection, NoDrift};
use crate::state::event_origin::Generation;
use crate::state::ime_actuation::FeedbackPolicy;
use crate::state::ime_event::{ObservationConfidence, ObservationSource};
use crate::tuning::DRIFT_CORRECTION_BLIND_REARM_COOLDOWN_MS as CD;

const MAX: u32 = 3;

const BLIND: FeedbackPolicy = FeedbackPolicy::Blind { max_attempts: MAX };
const READ: FeedbackPolicy = FeedbackPolicy::Read {
    source: ObservationSource::ObserverPoll,
};

const NO_DRIFTS: [NoDrift; 5] = [
    NoDrift::NotExplicitIntent,
    NoDrift::NotDrifting,
    NoDrift::NoTrustedObservation,
    NoDrift::StaleObservation,
    NoDrift::ObservationMatchesDesired,
];

/// variant が増えたらコンパイルエラーにして、上の列挙表の更新を強いる。
const fn covered_no_drift(n: NoDrift) {
    match n {
        NoDrift::NotExplicitIntent
        | NoDrift::NotDrifting
        | NoDrift::NoTrustedObservation
        | NoDrift::StaleObservation
        | NoDrift::ObservationMatchesDesired => {}
    }
}

const fn covered_policy(p: FeedbackPolicy) {
    match p {
        FeedbackPolicy::Blind { .. } | FeedbackPolicy::Read { .. } => {}
    }
}

const fn covered_plan(p: &DriftPlan) {
    match p {
        DriftPlan::Idle(
            DriftIdle::EngineDisabled | DriftIdle::NotJapaneseIme | DriftIdle::NoDrift(_),
        )
        | DriftPlan::DeferToSettle { .. }
        | DriftPlan::Act(DriftAct {
            step:
                DriftStep::SkipWarrantWouldBlock
                | DriftStep::GiveUp(
                    GiveUpPark::FirstTime
                    | GiveUpPark::CooldownPending
                    | GiveUpPark::Rearm
                    | GiveUpPark::StillParked,
                )
                | DriftStep::Confirmed
                | DriftStep::Send(SendPath::ImmCross | SendPath::StrategyChain),
            ..
        }) => {}
    }
}

const fn tf(b: bool) -> &'static str {
    if b {
        "T"
    } else {
        "F"
    }
}

/// 時刻の基準。`now` から引いた時刻だけを使うので、`Instant` の下限を割らないよう 100 秒先に置く。
#[derive(Debug)]
struct Clock {
    now: Instant,
}

impl Clock {
    fn new() -> Self {
        Self {
            now: Instant::now() + Duration::from_secs(100),
        }
    }

    fn sent(&self) -> Instant {
        self.now - Duration::from_secs(10)
    }

    fn gave_up_before_cd(&self) -> Instant {
        self.now - Duration::from_millis(CD - 1)
    }

    fn gave_up_at_cd(&self) -> Instant {
        self.now - Duration::from_millis(CD)
    }

    fn label(&self, t: Instant) -> &'static str {
        if t == self.now {
            "now"
        } else if t == self.sent() {
            "now-10s"
        } else if t == self.gave_up_before_cd() {
            "now-(cd-1)"
        } else if t == self.gave_up_at_cd() {
            "now-cd"
        } else {
            "?"
        }
    }
}

fn fmt_policy(p: FeedbackPolicy) -> &'static str {
    covered_policy(p);
    match p {
        FeedbackPolicy::Blind { .. } => "Blind",
        FeedbackPolicy::Read { .. } => "Read",
    }
}

fn fmt_snapshot(c: &Clock, a: &ActuationSnapshot) -> String {
    let policy = match a.policy {
        FeedbackPolicy::Blind { max_attempts } => format!("Blind{{max={max_attempts}}}"),
        FeedbackPolicy::Read { source } => format!("Read{{{source:?}}}"),
    };
    let gave_up = a.gave_up_at.map_or("-", |t| c.label(t));
    // 出所(strategy)は方針から決まる。世代は値をそのまま書く(作り直しで INITIAL=0 に戻れば差分になる)。
    let source_ok = a.origin.source == a.policy.origin(Generation::INITIAL).source;
    format!(
        "(target={} {policy} attempts={} sent={} gave_up={gave_up} origin={}@{})",
        tf(a.target),
        a.attempts,
        c.label(a.sent_at),
        if source_ok { "policy" } else { "other" },
        a.origin.epoch.value()
    )
}

/// 試行の入力(`active`)の 10 通り。`desired` に対する `target` の一致/不一致で作る。
/// 世代は殻と同じく送信ごとに進む(`advance_epoch` が `attempts` と世代を同時に進める)ので `attempts` と同じ値にする。
fn actives(c: &Clock, desired: bool) -> Vec<(String, Option<ActuationSnapshot>)> {
    let snap =
        |policy: FeedbackPolicy, target: bool, attempts: u32, gave_up_at: Option<Instant>| {
            ActuationSnapshot {
                target,
                policy,
                attempts,
                sent_at: c.sent(),
                gave_up_at,
                origin: policy.origin(Generation::new(u64::from(attempts))),
            }
        };
    let mut out = vec![("none".to_string(), None)];
    for attempts in [MAX - 1, MAX] {
        for gave_up_at in [None, Some(c.gave_up_before_cd()), Some(c.gave_up_at_cd())] {
            let a = snap(BLIND, desired, attempts, gave_up_at);
            out.push((format!("same{}", fmt_snapshot(c, &a)), Some(a)));
        }
    }
    let read_same = snap(READ, desired, 1, None);
    out.push((
        format!("same{}", fmt_snapshot(c, &read_same)),
        Some(read_same),
    ));
    let blind_other = snap(BLIND, !desired, MAX, Some(c.gave_up_at_cd()));
    out.push((
        format!("other{}", fmt_snapshot(c, &blind_other)),
        Some(blind_other),
    ));
    let read_other = snap(READ, !desired, 1, None);
    out.push((
        format!("other{}", fmt_snapshot(c, &read_other)),
        Some(read_other),
    ));
    out
}

/// 殻(`ir_observe_drift_facts`)が作りうる後段の事実か。
fn produced_by_shell(f: &DriftFacts) -> bool {
    let Ok(d) = f.drift else {
        return true;
    };
    let (act, _) = resolve_actuation(f.active.as_ref(), d.desired, f.default_policy, f.now);
    let read = matches!(act.policy, FeedbackPolicy::Read { .. });
    let fresh_readable = !read && act.gave_up_at.is_some();
    (!f.warrant_would_block || f.imm_cross)
        && (!f.converged || read)
        && (!f.fresh_evidence_after_giveup || fresh_readable)
}

fn drift(desired: bool, duration_ms: u64) -> DriftCorrection {
    DriftCorrection {
        desired,
        observed: !desired,
        duration_ms,
        source: ObservationSource::ObserverPoll,
        confidence: ObservationConfidence::Medium,
    }
}

fn fmt_duration(ms: u64) -> &'static str {
    if ms == CD {
        "cd"
    } else {
        "cd-1"
    }
}

fn fmt_drift_input(d: Result<DriftCorrection, NoDrift>) -> String {
    match d {
        Ok(d) => format!(
            "Ok(desired={} dur={})",
            tf(d.desired),
            fmt_duration(d.duration_ms)
        ),
        Err(n) => {
            covered_no_drift(n);
            format!("Err({n:?})")
        }
    }
}

fn base_facts(c: &Clock) -> DriftFacts {
    DriftFacts {
        now: c.now,
        engine_enabled: true,
        japanese_ime: true,
        drift: Ok(drift(true, CD)),
        settling: false,
        active: None,
        default_policy: BLIND,
        imm_cross: false,
        warrant_would_block: false,
        diag_already_notified: false,
        converged: false,
        fresh_evidence_after_giveup: false,
    }
}

/// 計画を 1 行に書く。`drift=in` は計画の中のずれが入力のずれと同じ(写しただけ)であること。
fn fmt_plan(c: &Clock, f: &DriftFacts, plan: &DriftPlan) -> String {
    covered_plan(plan);
    let echoed = |d: &DriftCorrection| {
        if f.drift.as_ref().ok() == Some(d) {
            "in".to_string()
        } else {
            format!("{d:?}")
        }
    };
    let (kind, reason) = plan.label();
    let basis = plan
        .basis()
        .map_or_else(|| "-".to_string(), |b| format!("{b:?}"));
    match plan {
        DriftPlan::Idle(_) => format!("{kind}({reason}) basis={basis}"),
        DriftPlan::DeferToSettle { drift } => {
            format!("{kind}({reason}) basis={basis} drift={}", echoed(drift))
        }
        DriftPlan::Act(act) => format!(
            "{kind}({reason}) basis={basis} drift={} install={} notify={} act={}",
            echoed(&act.drift),
            tf(act.install_actuation),
            tf(act.notify_diagnostic),
            fmt_snapshot(c, &act.actuation)
        ),
    }
}

fn render() -> String {
    let c = Clock::new();
    let mut out = String::new();
    out.push_str(
        "# decide_drift_plan の決定表(全列挙) golden (ME-3)\n\
         #\n\
         # 生成元: crates/awase-ime-core/src/state/drift_plan_exhaustive_tests.rs\n\
         # このファイルは自動生成される。更新は UPDATE_GOLDEN=1 で再生成するか、CI の失敗ログの BEGIN/END の間を写すこと。\n\
         # 撤去・統合の PR でこのファイルが変わらなければ、decide_drift_plan の決定は列挙した全入力で不変。\n\
         # 後段は殻(ir_observe_drift_facts)が作る事実の組だけ(契約外の組は列挙しない)。\n\
         # cd = DRIFT_CORRECTION_BLIND_REARM_COOLDOWN_MS、max = Blind.max_attempts。\n\
         #\n",
    );
    let _ = writeln!(out, "cd_ms\t{CD}\tmax\t{MAX}");

    // ── 前段 ──
    out.push_str(
        "\n## 前段: engine_enabled × japanese_ime × drift × settling(後段の材料は既定値)\n\n",
    );
    let mut drifts: Vec<Result<DriftCorrection, NoDrift>> =
        NO_DRIFTS.iter().map(|&n| Err(n)).collect();
    for desired in [false, true] {
        for dur in [CD - 1, CD] {
            drifts.push(Ok(drift(desired, dur)));
        }
    }
    let mut pre_count = 0usize;
    for engine_enabled in [false, true] {
        for japanese_ime in [false, true] {
            for &d in &drifts {
                for settling in [false, true] {
                    let f = DriftFacts {
                        engine_enabled,
                        japanese_ime,
                        drift: d,
                        settling,
                        ..base_facts(&c)
                    };
                    pre_count += 1;
                    let _ = writeln!(
                        out,
                        "[engine={} jp={} drift={} settling={}] => {}",
                        tf(engine_enabled),
                        tf(japanese_ime),
                        fmt_drift_input(d),
                        tf(settling),
                        fmt_plan(&c, &f, &decide_drift_plan(&f))
                    );
                }
            }
        }
    }

    // ── 後段 ──
    out.push_str(
        "\n## 後段: desired × duration × active × default_policy ごとに、\n\
         ## ビット列 imm_cross,warrant_would_block,diag_already_notified,converged,fresh_evidence_after_giveup の結果\n\
         ## (殻が作る組だけ。* はその組の殻が作るビット列すべて)\n\n",
    );
    let mut post_count = 0usize;
    for desired in [false, true] {
        for dur in [CD - 1, CD] {
            for (active_label, active) in actives(&c, desired) {
                for default_policy in [BLIND, READ] {
                    let _ = writeln!(
                        out,
                        "[desired={} dur={} active={active_label} default={}]",
                        tf(desired),
                        fmt_duration(dur),
                        fmt_policy(default_policy)
                    );
                    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
                    let mut produced = 0usize;
                    for bits in 0u8..32 {
                        let bit = |i: u8| bits & (1 << (4 - i)) != 0;
                        let f = DriftFacts {
                            drift: Ok(drift(desired, dur)),
                            active,
                            default_policy,
                            imm_cross: bit(0),
                            warrant_would_block: bit(1),
                            diag_already_notified: bit(2),
                            converged: bit(3),
                            fresh_evidence_after_giveup: bit(4),
                            ..base_facts(&c)
                        };
                        if !produced_by_shell(&f) {
                            continue;
                        }
                        produced += 1;
                        post_count += 1;
                        let key = fmt_plan(&c, &f, &decide_drift_plan(&f));
                        let name: String = (0..5).map(|i| if bit(i) { '1' } else { '0' }).collect();
                        match groups.iter_mut().find(|(k, _)| *k == key) {
                            Some((_, names)) => names.push(name),
                            None => groups.push((key, vec![name])),
                        }
                    }
                    for (key, names) in groups {
                        let names = if names.len() == produced {
                            "*".to_string()
                        } else {
                            names.join(",")
                        };
                        let _ = writeln!(out, "  {{{names}}} => {key}");
                    }
                }
            }
        }
    }
    let _ = writeln!(out, "\n# 前段 {pre_count} 通り、後段 {post_count} 通り");
    out
}

fn golden_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join("drift_plan_table.txt")
}

#[test]
fn input_space_size_is_as_documented() {
    let c = Clock::new();
    assert_eq!(actives(&c, true).len(), 10);
    let text = render();
    assert!(
        text.contains("# 前段 72 通り、後段 792 通り"),
        "列挙の数が doc と違う"
    );
    assert!(
        !text.contains("=?"),
        "時刻のラベルが付かない Instant がある"
    );
}

#[test]
fn drift_plan_table_matches_golden() {
    let actual = render();
    let path = golden_path();
    if std::env::var("UPDATE_GOLDEN").as_deref() == Ok("1") {
        std::fs::create_dir_all(path.parent().expect("golden の親"))
            .expect("golden の親を作れない");
        std::fs::write(&path, &actual).expect("golden を書けない");
        eprintln!("golden を更新しました: {}", path.display());
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    if actual == expected {
        return;
    }
    let first_diff = actual
        .lines()
        .zip(expected.lines())
        .enumerate()
        .find(|(_, (a, e))| a != e)
        .map_or_else(
            || {
                format!(
                    "行数が違う(現状 {} 行 / golden {} 行)",
                    actual.lines().count(),
                    expected.lines().count()
                )
            },
            |(n, (a, e))| format!("{} 行目\n  現状:   {a}\n  golden: {e}", n + 1),
        );
    eprintln!("----- BEGIN drift_plan_table -----");
    eprint!("{actual}");
    eprintln!("----- END drift_plan_table -----");
    panic!(
        "decide_drift_plan の決定表が golden({}) と一致しません。最初の差: {first_diff}\n\
         撤去・統合で決定を変えないはずの PR なら、変えた箇所を見直すこと。意図した変更(tuning の定数の変更を含む)なら\n\
         `UPDATE_GOLDEN=1 cargo test -p awase-ime-core --lib drift_plan::exhaustive` で再生成するか、\n\
         上の BEGIN/END の間(現状の全文)を golden に写して、差分を PR で説明すること。",
        path.display()
    );
}
