#![allow(clippy::all, clippy::pedantic, clippy::nursery)]
//! ADR-208 L0: 明示キー押下の配送 `explicit_press_delivery_with` の全列挙テストと、現状の反例の golden。
//!
//! 状態空間（belief 2 × applied 5 × is_japanese 2 × profile 6 × kind 2 × current_focus 2 × 観測 3 × IntentStore 3 ×
//! candidate_was_seen 2 × chord 2 × win 2 = 34,560 状態）× キー 12 種 = 414,720 通りの押下を全列挙し
//! （実 IME の初期値 R∈{false,true} も掛けると 829,440 通り）、次の性質を検査する。
//!
//! - **P1 (INV-L1)**: Win 押下を除き `(physical==Allow) XOR write.is_some()`（配送の過不足なし）。
//! - **P2 (絶対キーの 1 回収束)**: 絶対指定キーは 1 押下で実 IME がキーの向きに一致する。
//! - **P3 (トグルの 2 回収束)**: トグルキーは 2 押下以内で実 IME の状態が変わる（固着しない）。
//! - **P4 (不動点なし)**: 最大 3 押下で同じ「どちらも届かない」を 2 回続けて繰り返さない。
//! - **P5 (BUG-113)**: 同一押下で shadow 書き込みと Engine SetOpen が両方来ても送信 1 回（押下 id が L2 で入るまで未達）。
//!
//! 現状はこれらが破れる。**`#[should_panic]` にせず、破れるケースを ADR-208 監査（`docs/tasks/adr208-liveness-audit-2026-10-01.md`）
//! の S-1〜S-4・L-x に対応するクラスごとの件数と代表例として golden（`tests/golden/explicit_press_counterexamples.txt`）に固定する。**
//! L1〜L3 で穴を直すと件数が減り、golden の更新（`UPDATE_GOLDEN=1`）がそのまま進捗になる。golden に載らない
//! 未分類の反例（`unclassified`）が出たらテストは失敗する（モデルか分類の更新漏れ）。
//!
//! 授権（`issue_open_warrant`）は合成した `IntentStore`/`ObservationStore` に対して**本物**を呼ぶ（`StoreJudge`）。
//!
//! 再生成: `UPDATE_GOLDEN=1 cargo test -p awase-windows --test explicit_press_exhaustive`

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

use awase_windows::state::app_ime_policy::AppImePolicy;
use awase_windows::state::evidence::AnyObservation;
use awase_windows::state::explicit_press::{
    explicit_press_delivery_with, ime_after_press, state_after_press, AppliedKnowledge, Delivery,
    ElisionReason, ExplicitKey, KeyMeaning, Physical, PressProfile, PressState, WarrantJudge,
    WarrantRequest,
};
use awase_windows::state::force_guard::ForceGuardSet;
use awase_windows::state::ime_event::{
    HwndId, ImePolicyProfile, ObservationConfidence, ObservationSource, UserIntentSource,
};
use awase_windows::state::intent_store::IntentStore;
use awase_windows::state::observation_store::ObservationStore;
use awase_windows::state::open_warrant::{issue_open_warrant, WarrantContext};
use awase_windows::state::TickMs;

const TARGET: HwndId = HwndId(0x1234);

/// 合成ストアに対して本物の `issue_open_warrant` を呼ぶ判定器（結果は引数の組でメモ化する）。
#[derive(Default)]
struct StoreJudge {
    memo: RefCell<HashMap<(bool, bool, bool, u8, bool, Option<bool>, Option<bool>), bool>>,
}

fn policy_index(p: ImePolicyProfile) -> u8 {
    match p {
        ImePolicyProfile::ImmCross => 0,
        ImePolicyProfile::Imm32Unavailable => 1,
        ImePolicyProfile::TsfNative => 2,
        ImePolicyProfile::Plain => 3,
        ImePolicyProfile::Unknown => 4,
    }
}

impl WarrantJudge for StoreJudge {
    fn warranted(&self, req: &WarrantRequest) -> bool {
        let key = (
            req.requested,
            req.target_known,
            req.is_japanese_ime,
            policy_index(req.policy_profile),
            req.desired_open,
            req.intent,
            req.actuating_obs,
        );
        if let Some(v) = self.memo.borrow().get(&key) {
            return *v;
        }
        let now = Instant::now();
        let mut intents = IntentStore::default();
        if let Some(open) = req.intent {
            intents.record(TARGET, open, UserIntentSource::PhysicalImeKey, TickMs(0));
        }
        let mut obs = ObservationStore::default();
        if let Some(open) = req.actuating_obs {
            obs.record_replayed(
                AnyObservation::restored_from_journal(
                    open,
                    ObservationSource::ImmGetOpenStatus,
                    TARGET,
                    ObservationConfidence::High,
                    0,
                ),
                now,
            );
        }
        let guards = ForceGuardSet::default();
        let policy = AppImePolicy::from_profile(req.policy_profile);
        let ctx = WarrantContext {
            intent_store: &intents,
            obs: &obs,
            guards: &guards,
            policy: &policy,
            desired_open: req.desired_open,
            is_japanese_ime: req.is_japanese_ime,
            now,
            now_ms: TickMs(0),
        };
        let target = if req.target_known {
            TARGET
        } else {
            HwndId::NULL
        };
        let v = issue_open_warrant(req.requested, target, &ctx).is_some();
        self.memo.borrow_mut().insert(key, v);
        v
    }
}

fn delivery(judge: &StoreJudge, s: &PressState, key: ExplicitKey) -> Delivery {
    explicit_press_delivery_with(s, key, judge)
}

// ── 分類 ─────────────────────────────────────────────────────────────────────

/// 反例のクラス（ADR-208 監査 §2/§3 の ID に対応）。表示順もこの順。
const CLASSES: &[(&str, &str)] = &[
    (
        "S1_L1_already_matched",
        "S-1/L-1: GjiDirect の already-matched（Engine 経由の絶対キー × 古い applied）で Consume して書かない",
    ),
    (
        "S2_L3_not_japanese_unwarranted",
        "S-2/L-3: is_japanese_ime=false で授権が下りない（Engine のコンボ・0x16/0x1A が全窓で効かない）",
    ),
    (
        "S4_L7_focus_none_unwarranted",
        "S-4/L-7: current_focus=None で授権が下りない（意図の記録が no-op で Step 1 が外れ、鮮度内の観測が無いか向きが逆）",
    ),
    (
        "L9_chord_filtered_unwarranted",
        "L-9: chord フィルタに落ちた Engine の OFF は意図を記録せず、観測と食い違うと授権が下りない",
    ),
    (
        "unwarranted_other",
        "授権が下りない（上記以外。分類の更新漏れの警告として件数を出す）",
    ),
    (
        "L4_not_promoted_suppressed",
        "L-4: is_japanese_ime=false の漢字(0x19)は shadow 昇格せず、ImmCross では物理も Suppress される",
    ),
    (
        "S3_L6_shadow_noop_suppressed",
        "S-3/L-6: shadow no-op（belief が既に向きと一致）は書かず、物理は Suppress される（ImmCross）",
    ),
    (
        "L5_input_relay_consumed",
        "L-5: InputRelay の窓では Engine のコンボを Consume するが awase は書かない（所有者決定3で素通しへ）",
    ),
    (
        "double_actuation",
        "二重 actuation: 物理キーが届き（Allow）、かつ awase も書く",
    ),
    ("unclassified", "未分類（あってはならない）"),
];

/// P1 を破る押下（Win 押下を除く）のクラス。破らなければ `None`。
fn p1_class(s: &PressState, key: ExplicitKey, d: &Delivery) -> Option<&'static str> {
    if s.win_held {
        return None;
    }
    let delivered = d.physical == Physical::Allow;
    let wrote = d.write.is_some();
    // INV-L1: ちょうど一方。両方（二重 actuation）か、どちらも無し（二重の空振り）が違反。
    if delivered != wrote {
        return None;
    }
    if delivered && wrote {
        return Some("double_actuation");
    }
    // どちらも届かない。理由で分類する。
    let eff_jp = state_after_press(s, key, d).is_japanese_ime;
    Some(match d.reason {
        ElisionReason::AlreadyMatched => "S1_L1_already_matched",
        ElisionReason::Unwarranted if !eff_jp => "S2_L3_not_japanese_unwarranted",
        ElisionReason::Unwarranted if !s.current_focus_known => "S4_L7_focus_none_unwarranted",
        ElisionReason::Unwarranted if key == ExplicitKey::EngineOff && s.ctrl_chord => {
            "L9_chord_filtered_unwarranted"
        }
        ElisionReason::Unwarranted => "unwarranted_other",
        ElisionReason::NotPromoted => "L4_not_promoted_suppressed",
        ElisionReason::ShadowNoop => "S3_L6_shadow_noop_suppressed",
        ElisionReason::InputRelayNotOwned => "L5_input_relay_consumed",
        ElisionReason::WinHeld | ElisionReason::Written => "unclassified",
    })
}

fn fmt_state(s: &PressState, key: ExplicitKey, real: Option<bool>) -> String {
    let applied = match s.applied {
        AppliedKnowledge::Unknown => "Unknown".to_string(),
        AppliedKnowledge::Optimistic(v) => format!("Opt({v})"),
        AppliedKnowledge::Confirmed(v) => format!("Conf({v})"),
    };
    let opt = |o: Option<bool>| o.map_or("None".to_string(), |v| format!("Some({v})"));
    let mut out = format!(
        "key={key:?} belief={} applied={applied} jp={} profile={:?} kind={:?} focus={} obs={} intent={} cand={} chord={} win={}",
        s.belief_open,
        s.is_japanese_ime,
        s.profile,
        s.ime_kind,
        if s.current_focus_known { "Some" } else { "None" },
        opt(s.actuating_obs),
        opt(s.intent),
        s.candidate_was_seen,
        s.ctrl_chord,
        s.win_held,
    );
    if let Some(r) = real {
        let _ = write!(out, " R={r}");
    }
    out
}

fn fmt_delivery(d: &Delivery) -> String {
    format!(
        "physical={:?} write={:?} reason={:?}",
        d.physical, d.write, d.reason
    )
}

// ── 集計 ─────────────────────────────────────────────────────────────────────

#[derive(Default)]
struct ClassStat {
    all: u64,
    plausible: u64,
    example: Option<String>,
    plausible_example: Option<String>,
}

#[derive(Default)]
struct PropStat {
    checked: u64,
    violations: u64,
    plausible_violations: u64,
    classes: BTreeMap<&'static str, ClassStat>,
}

impl PropStat {
    fn add(&mut self, class: &'static str, plausible: bool, example: impl FnOnce() -> String) {
        self.violations += 1;
        if plausible {
            self.plausible_violations += 1;
        }
        let c = self.classes.entry(class).or_default();
        c.all += 1;
        if plausible {
            c.plausible += 1;
        }
        if c.example.is_none() || (plausible && c.plausible_example.is_none()) {
            let e = example();
            if c.example.is_none() {
                c.example = Some(e.clone());
            }
            if plausible && c.plausible_example.is_none() {
                c.plausible_example = Some(e);
            }
        }
    }
}

struct Report {
    p1: PropStat,
    p2: PropStat,
    p3: PropStat,
    p4: PropStat,
    p5: PropStat,
    states: u64,
    plausible_states: u64,
}

/// 反例のクラスを持たない（P1 を満たす）押下で P2/P3/P4 が破れる場合のクラス。
fn fallback_class(s: &PressState, key: ExplicitKey, d: &Delivery) -> &'static str {
    p1_class(s, key, d).unwrap_or("unclassified")
}

fn analyze() -> Report {
    let judge = StoreJudge::default();
    let mut rep = Report {
        p1: PropStat::default(),
        p2: PropStat::default(),
        p3: PropStat::default(),
        p4: PropStat::default(),
        p5: PropStat::default(),
        states: 0,
        plausible_states: 0,
    };
    for s in PressState::all() {
        rep.states += 1;
        let plausible = s.is_plausible();
        if plausible {
            rep.plausible_states += 1;
        }
        for key in ExplicitKey::ALL {
            let d1 = delivery(&judge, &s, key);

            // P1
            rep.p1.checked += 1;
            if let Some(class) = p1_class(&s, key, &d1) {
                rep.p1.add(class, plausible, || {
                    format!("{} -> {}", fmt_state(&s, key, None), fmt_delivery(&d1))
                });
            }

            // P2（絶対キー、実 IME の初期値 R を掛ける）/ P3（トグル）
            for r0 in [false, true] {
                match key.meaning() {
                    KeyMeaning::Absolute(t) if !s.win_held => {
                        rep.p2.checked += 1;
                        let r1 = ime_after_press(r0, key, &d1);
                        if r1 != t {
                            rep.p2.add(fallback_class(&s, key, &d1), plausible, || {
                                format!(
                                    "{} -> {} (R: {r0} -> {r1}, 向き={t})",
                                    fmt_state(&s, key, Some(r0)),
                                    fmt_delivery(&d1)
                                )
                            });
                        }
                    }
                    KeyMeaning::Toggle if !s.win_held => {
                        rep.p3.checked += 1;
                        let r1 = ime_after_press(r0, key, &d1);
                        let s1 = state_after_press(&s, key, &d1);
                        let d2 = delivery(&judge, &s1, key);
                        let r2 = ime_after_press(r1, key, &d2);
                        if r1 == r0 && r2 == r0 {
                            let class = p1_class(&s, key, &d1)
                                .or_else(|| p1_class(&s1, key, &d2))
                                .unwrap_or("unclassified");
                            rep.p3.add(class, plausible, || {
                                format!(
                                    "{} -> 1回目 {} / 2回目 {} (R: {r0} -> {r1} -> {r2})",
                                    fmt_state(&s, key, Some(r0)),
                                    fmt_delivery(&d1),
                                    fmt_delivery(&d2)
                                )
                            });
                        }
                    }
                    _ => {}
                }
            }

            // P4: 最大 3 押下で同じ「どちらも届かない」を 2 回続けない（R は配送に影響しないので掛けない）。
            if !s.win_held {
                rep.p4.checked += 1;
                let mut cur = s;
                let mut prev: Option<&'static str> = None;
                let mut cur_d = d1;
                for press in 1..=3 {
                    let class = p1_class(&cur, key, &cur_d).filter(|c| *c != "double_actuation");
                    if let (Some(p), Some(c)) = (prev, class) {
                        if p == c {
                            rep.p4.add(c, plausible, || {
                                format!(
                                    "{} -> {press}回目も同じ: {}",
                                    fmt_state(&s, key, None),
                                    fmt_delivery(&cur_d)
                                )
                            });
                            break;
                        }
                    }
                    prev = class;
                    let next = state_after_press(&cur, key, &cur_d);
                    cur = next;
                    cur_d = delivery(&judge, &cur, key);
                }
            }
        }

        // P5（BUG-113、現状のモデル・推測）: 同一押下で shadow 書き込み → Engine の SetOpen が続く（sync キーが
        // `keys.ime_on/off` でもある構成）。Engine 側の executor は押下前の `applied_snapshot` を見る（完了の反映が
        // 押下の処理後のため）と仮定する。
        if !s.win_held {
            for (shadow_key, engine_key) in [
                (ExplicitKey::StaticOn, ExplicitKey::EngineOn),
                (ExplicitKey::StaticOff, ExplicitKey::EngineOff),
            ] {
                rep.p5.checked += 1;
                let d_shadow = delivery(&judge, &s, shadow_key);
                let after = state_after_press(&s, shadow_key, &d_shadow);
                let engine_state = PressState {
                    applied: s.applied,
                    ..after
                };
                let d_engine = delivery(&judge, &engine_state, engine_key);
                if d_shadow.write.is_some() && d_engine.write.is_some() {
                    rep.p5.add("bug113_double_send", plausible, || {
                        format!(
                            "{} -> shadow {} / engine {}",
                            fmt_state(&s, shadow_key, None),
                            fmt_delivery(&d_shadow),
                            fmt_delivery(&d_engine)
                        )
                    });
                }
            }
        }
    }
    rep
}

fn render(rep: &Report) -> String {
    let mut out = String::new();
    out.push_str(
        "# 明示キー押下の配送 現状の反例 golden (ADR-208 L0)\n\
         #\n\
         # 生成元: crates/awase-windows/tests/explicit_press_exhaustive.rs\n\
         # このファイルは自動生成される。更新は UPDATE_GOLDEN=1 で再生成すること。\n\
         #\n\
         # 状態空間(belief 2 × applied 5 × is_japanese 2 × profile 6 × kind 2 × current_focus 2 × 観測 3 ×\n\
         # IntentStore 3 × candidate_was_seen 2 × chord 2 × win 2) × キー 12 種を全列挙した、現状の本番判断の合成結果。\n\
         # L1〜L3 で穴を直すと該当クラスの件数が減る(この差分が進捗)。クラス名の ID は\n\
         # docs/tasks/adr208-liveness-audit-2026-10-01.md の §2(L-n) / §3(S-n) に対応する。\n\
         # 「起こりうる」= Blind プロファイル(Imm32Unavailable/TsfNative)で Actuating 観測が無い組み合わせ。\n\
         # P5 は「同一押下で shadow 書き込みの後に Engine の SetOpen が続くとき、executor は押下前の applied を見る」という\n\
         # 現状のモデル(推測)での件数。押下 id(L2)で 0 になるべきもの。\n\
         #\n",
    );
    let _ = writeln!(
        out,
        "states\t{}\tplausible\t{}\tkeys\t{}\n",
        rep.states,
        rep.plausible_states,
        ExplicitKey::ALL.len()
    );
    let props: [(&str, &str, &PropStat); 5] = [
        (
            "P1",
            "INV-L1: Win 押下を除き (physical==Allow) XOR write.is_some()",
            &rep.p1,
        ),
        (
            "P2",
            "絶対キーは 1 押下で実 IME がキーの向きに一致する（実 IME の初期値 R を掛ける）",
            &rep.p2,
        ),
        (
            "P3",
            "トグルは 2 押下以内で実 IME の状態が変わる（実 IME の初期値 R を掛ける）",
            &rep.p3,
        ),
        (
            "P4",
            "最大 3 押下で同じ「どちらも届かない」を 2 回続けて繰り返さない",
            &rep.p4,
        ),
        (
            "P5",
            "同一押下で shadow 書き込み + Engine SetOpen が来ても送信 1 回（現状モデル）",
            &rep.p5,
        ),
    ];
    for (id, desc, stat) in props {
        let _ = writeln!(out, "## {id}: {desc}");
        let _ = writeln!(
            out,
            "checked\t{}\tviolations\t{}\tplausible_violations\t{}",
            stat.checked, stat.violations, stat.plausible_violations
        );
        let mut classes: Vec<_> = stat.classes.iter().collect();
        classes.sort_by_key(|(name, _)| {
            CLASSES
                .iter()
                .position(|(n, _)| n == *name)
                .unwrap_or(usize::MAX)
        });
        for (name, c) in classes {
            let desc = CLASSES
                .iter()
                .find(|(n, _)| n == name)
                .map_or("(P5)", |(_, d)| *d);
            let _ = writeln!(
                out,
                "class\t{name}\tall\t{}\tplausible\t{}",
                c.all, c.plausible
            );
            let _ = writeln!(out, "  # {desc}");
            if let Some(e) = &c.example {
                let _ = writeln!(out, "  example: {e}");
            }
            if let Some(e) = &c.plausible_example {
                if Some(e) != c.example.as_ref() {
                    let _ = writeln!(out, "  plausible_example: {e}");
                }
            }
        }
        out.push('\n');
    }
    out
}

fn golden_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join("explicit_press_counterexamples.txt")
}

#[test]
fn exhaustive_properties_and_counterexample_golden() {
    let started = Instant::now();
    let rep = analyze();
    let elapsed = started.elapsed();
    eprintln!(
        "explicit_press: {} states × {} keys を {:?} で全列挙",
        rep.states,
        ExplicitKey::ALL.len(),
        elapsed
    );

    // 分類漏れは golden に載せず落とす（モデルか分類の更新漏れ）。
    for (name, stat) in [
        ("P1", &rep.p1),
        ("P2", &rep.p2),
        ("P3", &rep.p3),
        ("P4", &rep.p4),
    ] {
        assert!(
            !stat.classes.contains_key("unclassified"),
            "{name} に未分類の反例があります: {:?}",
            stat.classes
                .get("unclassified")
                .and_then(|c| c.example.as_ref())
        );
    }

    let actual = render(&rep);
    let path = golden_path();
    if std::env::var("UPDATE_GOLDEN").as_deref() == Ok("1") {
        std::fs::write(&path, &actual).expect("golden を書けない");
        eprintln!("golden を更新しました: {}", path.display());
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("golden {} を読めない: {e}", path.display()))
        .replace("\r\n", "\n");
    assert_eq!(
        actual, expected,
        "反例 golden が現状と一致しません。穴を直した/増やした場合は UPDATE_GOLDEN=1 で再生成して差分を確認すること"
    );
}

/// 実 IME の応答モデルが破綻していないことの前提: 反例の無い押下（P1 を満たす）だけを抜き出すと、
/// 絶対キーは常に 1 押下で向きに一致する（P2 の違反は P1 のどちらも届かないに帰着する）。
#[test]
fn p2_violations_reduce_to_p1_no_delivery() {
    let judge = StoreJudge::default();
    for s in PressState::all().filter(|s| !s.win_held) {
        for key in ExplicitKey::ALL {
            let KeyMeaning::Absolute(t) = key.meaning() else {
                continue;
            };
            let d = delivery(&judge, &s, key);
            let p1_ok = p1_class(&s, key, &d).is_none();
            for r0 in [false, true] {
                if p1_ok {
                    assert_eq!(
                        ime_after_press(r0, key, &d),
                        t,
                        "P1 を満たすのに絶対キーが向きに一致しない: {} -> {}",
                        fmt_state(&s, key, Some(r0)),
                        fmt_delivery(&d)
                    );
                }
            }
        }
    }
}

/// `Plain`/`Unknown` は ImmCross と同一内容でなければならない（INV-44、`caps` 表）。
#[test]
fn plain_and_unknown_profiles_deliver_like_imm_cross() {
    let judge = StoreJudge::default();
    for s in PressState::all().filter(|s| s.profile == PressProfile::ImmCross) {
        for key in ExplicitKey::ALL {
            let base = delivery(&judge, &s, key);
            for p in [PressProfile::Plain, PressProfile::Unknown] {
                let other = delivery(&judge, &PressState { profile: p, ..s }, key);
                assert_eq!(base, other, "{p:?} が ImmCross と違う: {key:?} {s:?}");
            }
        }
    }
}

/// Win 押下中は書き込まない（`UnsafeToToggle`、ADR-208 決定4(b) の例外）。物理の配送は Win の有無で変わらない。
#[test]
fn win_held_never_writes_and_does_not_change_physical_delivery() {
    let judge = StoreJudge::default();
    for s in PressState::all().filter(|s| s.win_held) {
        let free = PressState {
            win_held: false,
            ..s
        };
        for key in ExplicitKey::ALL {
            let d = delivery(&judge, &s, key);
            assert_eq!(d.write, None, "{key:?} {s:?}");
            let d_free = delivery(&judge, &free, key);
            assert_eq!(d.physical, d_free.physical, "{key:?} {s:?}");
        }
    }
}

/// 監査 §1 の分類表の「Phys」列との一致（`PhysicalKeyDisposition::plan_core` の合成が表どおりであること）。
/// どの内部状態でも変わらない配送だけを、全状態で固定する。
#[test]
fn physical_delivery_matches_the_audit_table() {
    let judge = StoreJudge::default();
    for s in PressState::all() {
        for key in ExplicitKey::ALL {
            let d = delivery(&judge, &s, key);
            // Engine のコンボ・単独タップは常に Consume。
            if matches!(key, ExplicitKey::EngineOn | ExplicitKey::EngineOff) {
                assert_eq!(d.physical, Physical::Consume, "{key:?} {s:?}");
                continue;
            }
            // InputRelay は常に Allow（issue #136）。
            if s.profile == PressProfile::InputRelay {
                assert_eq!(d.physical, Physical::Allow, "{key:?} {s:?}");
                continue;
            }
            // 物理のみのキー（英数/カタカナ/ひらがな、役割なしの無変換/変換）は常に Allow。
            if matches!(key, ExplicitKey::PhysOnlyMode | ExplicitKey::ThumbPlain) {
                assert_eq!(d.physical, Physical::Allow, "{key:?} {s:?}");
                continue;
            }
            let imm_cross = matches!(
                s.profile,
                PressProfile::ImmCross | PressProfile::Plain | PressProfile::Unknown
            );
            match key {
                // ImmCross: KANJI 系（shadow_action を持つキー）は Down を常に Suppress。
                ExplicitKey::StaticOn
                | ExplicitKey::StaticOff
                | ExplicitKey::HzToggle
                | ExplicitKey::Kanji
                | ExplicitKey::SyncOn
                | ExplicitKey::SyncOff
                | ExplicitKey::SyncToggle
                    if imm_cross =>
                {
                    assert_eq!(d.physical, Physical::Suppress, "{key:?} {s:?}");
                }
                // IU/TN: 0xF3/0xF4 の Down は shadow に関わらず常に Suppress。
                ExplicitKey::HzToggle => {
                    assert_eq!(d.physical, Physical::Suppress, "{key:?} {s:?}");
                }
                // IU/TN: 他のキーは awase が実際に書いた（shadow が belief を倒した）押下だけ Suppress。
                ExplicitKey::StaticOn
                | ExplicitKey::StaticOff
                | ExplicitKey::Kanji
                | ExplicitKey::SyncOn
                | ExplicitKey::SyncOff
                | ExplicitKey::SyncToggle => {
                    let expected = if d.shadow_toggled {
                        Physical::Suppress
                    } else {
                        Physical::Allow
                    };
                    assert_eq!(d.physical, expected, "{key:?} {s:?}");
                }
                // F13 役割: ImmCross でも IU/TN でも、書いた押下だけ Suppress。
                ExplicitKey::RoleFkeyToggle => {
                    let expected = if d.shadow_toggled {
                        Physical::Suppress
                    } else {
                        Physical::Allow
                    };
                    assert_eq!(d.physical, expected, "{key:?} {s:?}");
                }
                _ => unreachable!(),
            }
        }
    }
}

/// P5（BUG-113）: 同一押下で shadow 書き込みと Engine SetOpen が両方来ても送信 1 回。
/// 現状は押下 id が無く、executor が押下前の `applied` を見るため二重送信になる状態がある
/// （件数は golden の P5）。ADR-208 L2（押下 id）で通ること。
#[test]
#[ignore = "ADR-208 L2（押下 id: press_id・ActuationOrder.press・last_written_press）が入るまで未達。現状の件数は golden の P5"]
fn p5_same_press_sends_once() {
    let rep = analyze();
    assert_eq!(rep.p5.violations, 0, "{:?}", rep.p5.classes.keys());
}
