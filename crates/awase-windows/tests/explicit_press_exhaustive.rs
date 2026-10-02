#![allow(clippy::all, clippy::pedantic, clippy::nursery)]
//! ADR-208 L0: 明示キー押下の配送 `explicit_press_delivery_with` の全列挙テストと、現状の反例の golden。
//!
//! 状態空間（belief 2 × applied 5 × is_japanese 2 × profile 7 × (kind, TIP 同定) 3 × current_focus 2 × 観測 3 × IntentStore 3 ×
//! candidate_was_seen 2 × chord 2 × win 2 × was_down 2 = 120,960 状態）× キー 12 種 = 1,451,520 通りの押下を全列挙し
//! （実 IME の初期値 R∈{false,true} も掛けると約 166 万通り）、次の性質を検査する。
//!
//! - **P1 (INV-L1)**: 対象押下（非リピート・Win 押下中を除く）の `Delivery` が、配送か書き込みの**ちょうど一方**
//!   （`Delivery::resolve` が `Ok`）で、配送側なら前提 A1 の表のキー（`ExplicitKey::a1_holds`）。
//! - **P2 (絶対キーの 1 回収束)**: 絶対指定キーは 1 押下で実 IME がキーの向きに一致する。
//! - **P3 (トグルの 2 回収束)**: トグルキーは 2 押下以内で実 IME の状態が変わる（固着しない）。
//! - **P4 (不動点なし)**: 最大 3 押下で同じ「どちらも届かない」を 2 回続けて繰り返さない。
//! - **P5 (BUG-113)**: 同一押下で shadow 経路と Engine SetOpen が両方来ても、**同じ向きの二重送信は無い**（押下 ID の予約、
//!   ADR-208 L1）。向きが逆なら Engine の明示コンボが上書きして最終の向きは Engine（書き込みは 2 回。衝突の優先順位）。
//! - **P6**: 自動リピートの Down は対象外（`press=None`）。押下の書き込みの直後のリピートは、GjiDirect の `applied` の
//!   already-matched 省略に任せて VK を追い送りしない（ImmCross/MsImeDirect に省略は無く、従来どおり）。
//!
//! **本番の判断は ADR-208 L1 の `DeliveryMode::PressId`**（押下 ID の予約と applied の未知化）。破れるケースは
//! **`#[should_panic]` にせず**、ADR-208 監査（`docs/tasks/adr208-liveness-audit-2026-10-01.md`）の S-1〜S-4・L-x に対応する
//! クラスごとの件数と代表例として golden（`tests/golden/explicit_press_counterexamples.txt`）に固定する。
//! L1 で S-1 は TsfNative×GJI（BUG-124 の実機 A/B〈ADR-208 L3'〉まで段階制御）を除いて 0 になった（`P1-PreL1` が L1 前の件数）。L2/L3 で穴を直すと件数が減り、golden の更新（`UPDATE_GOLDEN=1`）が
//! そのまま進捗になる。golden に載らない未分類の反例（`unclassified`）が出たらテストは失敗する（モデルか分類の更新漏れ）。
//!
//! 授権（`issue_open_warrant`）は合成した `IntentStore`/`ObservationStore` に対して**本物**を呼ぶ（`StoreJudge`）。
//!
//! 遷移（書いた後の applied）は実物の `ImeModel`（`confirm_applied`・`reduce`）を通す。押下の予約は本番と同じ純粋な
//! `PressLedger`、applied の未知化は本番と同じ `explicit_press_shadow_on` を呼ぶ。D4 の固定点（`DeliveryMode::PressIdFixedPoint`）
//! での P1 も参考として golden に載せる（L3 で本番がこの形になる）。
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
    dual_route_writes_ledger_only, dual_route_writes_with, explicit_press_delivery_after,
    explicit_press_delivery_with, ime_after_press, reservation_after_route, state_after_press,
    AppliedKnowledge, Delivery, DeliveryMode, ElisionReason, ExplicitKey, KeyMeaning, Physical,
    PressProfile, PressState, Resolution, Violation, WarrantJudge, WarrantRequest,
};
use awase_windows::state::force_guard::ForceGuardSet;
use awase_windows::state::ime_event::{
    HwndId, ImePolicyProfile, ObservationConfidence, ObservationSource, UserIntentSource,
};
use awase_windows::state::ime_kind::ImeKindId;
use awase_windows::state::intent_store::IntentStore;
use awase_windows::state::observation_store::ObservationStore;
use awase_windows::state::open_warrant::{issue_open_warrant, issue_press_warrant, WarrantContext};
use awase_windows::state::TickMs;

const TARGET: HwndId = HwndId(0x1234);

/// 合成ストアに対して本物の `issue_open_warrant` を呼ぶ判定器（結果は引数の組でメモ化する）。
#[derive(Default)]
struct StoreJudge {
    memo: RefCell<HashMap<(bool, bool, bool, u8, bool, Option<bool>, Option<bool>, bool), bool>>,
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
            req.explicit_press,
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
        // 本番は `ActuationOrder::issue`（両方を評価）→ `with_press(Some)` で押下の授権へ差し替える（ADR-208 L2）。
        let v = if req.explicit_press {
            issue_press_warrant(req.requested, target, &ctx)
        } else {
            issue_open_warrant(req.requested, target, &ctx)
        }
        .is_some();
        self.memo.borrow_mut().insert(key, v);
        v
    }
}

/// 本番の判断（ADR-208 L1〜L3a: 押下 ID の予約・applied の未知化・授権・D4 の固定点）。
const PROD: DeliveryMode = DeliveryMode::PressIdFixedPoint;

fn delivery(judge: &StoreJudge, s: &PressState, key: ExplicitKey) -> Delivery {
    explicit_press_delivery_with(s, key, judge, PROD)
}

/// L1 まで（ADR-208 L2 前）の判断。`P1-PreL2` で L2 の効果を件数で比べるためだけに使う。
fn delivery_pre_l2(judge: &StoreJudge, s: &PressState, key: ExplicitKey) -> Delivery {
    explicit_press_delivery_with(s, key, judge, DeliveryMode::PressIdBeforeL2)
}

/// L1 前（ADR-208 L0 の現状）の判断。`P1-PreL1` で件数を比べるためだけに使う。
fn delivery_pre_l1(judge: &StoreJudge, s: &PressState, key: ExplicitKey) -> Delivery {
    explicit_press_delivery_with(s, key, judge, DeliveryMode::Legacy)
}

// ── 分類 ─────────────────────────────────────────────────────────────────────

/// 反例のクラス（ADR-208 の S-1〜S-4 と、監査・Opus レビューで分かった既知の分類）。表示順もこの順。
const CLASSES: &[(&str, &str)] = &[
    (
        "S1_already_matched",
        "S-1: GjiDirect の already-matched（Engine 経由の絶対キー × 古い applied）で Consume して書かない。L1 で解消（TsfNative×GJI だけ、BUG-124 の実機 A/B〈ADR-208 L3'〉まで既知の制限として残る）",
    ),
    (
        "S2_not_japanese",
        "S-2: is_japanese_ime=false。授権が下りない（Engine のコンボ・0x16/0x1A）／漢字(0x19)・F13 が昇格せず握る・配送だけになる。L2 で解消（押下の授権と非リピートの昇格が is_japanese_ime を問わない）",
    ),
    (
        "S3_shadow_noop_suppressed",
        "S-3: shadow no-op（belief が既に向きと一致）は書かず、物理は Suppress される（ImmCross）",
    ),
    (
        "S4_focus_none_unwarranted",
        "S-4: current_focus=None で授権が下りない（意図の記録が no-op で Step 1 が外れ、鮮度内の観測が無いか向きが逆）。L2 で解消（押下の授権は押下の意図そのもの、ExplicitPress）",
    ),
    (
        "L9_chord_filtered_unwarranted",
        "L-9（監査 §2、S-4 と同根: 意図が記録されない）: chord フィルタに落ちた Engine の OFF は意図を記録せず、観測と食い違うと授権が下りない",
    ),
    (
        "L5_input_relay_consumed",
        "L-5（所有者決定3、L4 段で素通しへ）: InputRelay の窓では Engine のコンボを Consume するが awase は書かない",
    ),
    (
        "A1_noop_pass_through",
        "前提 A1（Opus round1 B-2）: 前提 A1 が成り立たないキー（任意の sync キー等）の no-op を Allow で配送する（IME が処理する保証が無い）",
    ),
    (
        "double_actuation",
        "二重 actuation: 物理キーが届き（Allow）、かつ awase も書く",
    ),
    ("unclassified", "未分類（あってはならない）"),
];

/// 0x19 が IME の開閉キーではない状態（TIP 未同定かつ `is_japanese_ime=false`、ADR-208 L2 M-1）。受動（`shadow_action=None`）で
/// 物理は素通しなので、awase が決めることは無く P1〜P4 の対象外（英語 IME・IME 無しの窓・US 配列の Alt+` を飲み込まない）。
fn kanji_is_not_an_ime_key(s: &PressState, key: ExplicitKey) -> bool {
    key == ExplicitKey::Kanji && !s.is_japanese_ime && !s.ime_identified
}

/// 対象押下の P1 を破るクラス。破らなければ `None`。対象外（リピート・Win 押下・物理のみで意図を持たないキー）も `None`。
fn p1_class(s: &PressState, key: ExplicitKey, d: &Delivery) -> Option<&'static str> {
    if s.win_held || s.was_down || !key.is_target_press_key() || kanji_is_not_an_ime_key(s, key) {
        return None;
    }
    let eff_jp = state_after_press(s, key, d).is_japanese_ime;
    match d.resolve() {
        Ok(Resolution::Write { .. }) => None,
        // InputRelay は素通しが設計（中継先の IME が処理する。所有者決定3）なので A1 を問わない。
        Ok(Resolution::PassThrough) if key.a1_holds() || s.profile == PressProfile::InputRelay => {
            None
        }
        // no-op（belief が既に向きと一致）を A1 の無いキーで配送するのは `is_japanese_ime` を問わない別の穴（L2 で、
        // `is_japanese_ime` が偽の no-op を S-2 と誤分類していたのを直した）。
        Ok(Resolution::PassThrough) => Some(if d.reason == ElisionReason::ShadowNoop {
            "A1_noop_pass_through"
        } else if !eff_jp {
            "S2_not_japanese"
        } else {
            "unclassified"
        }),
        Err(Violation::Both { .. }) => Some("double_actuation"),
        Err(Violation::Neither(reason)) => Some(match reason {
            ElisionReason::AlreadyMatched => "S1_already_matched",
            ElisionReason::Unwarranted if !eff_jp => "S2_not_japanese",
            ElisionReason::Unwarranted if !s.current_focus_known => "S4_focus_none_unwarranted",
            ElisionReason::Unwarranted if key == ExplicitKey::EngineOff && s.ctrl_chord => {
                "L9_chord_filtered_unwarranted"
            }
            ElisionReason::NotPromoted if !eff_jp => "S2_not_japanese",
            ElisionReason::ShadowNoop => "S3_shadow_noop_suppressed",
            ElisionReason::InputRelayNotOwned => "L5_input_relay_consumed",
            _ => "unclassified",
        }),
    }
}

fn fmt_state(s: &PressState, key: ExplicitKey, real: Option<bool>) -> String {
    let applied = match s.applied {
        AppliedKnowledge::Unknown => "Unknown".to_string(),
        AppliedKnowledge::Optimistic(v) => format!("Opt({v})"),
        AppliedKnowledge::Confirmed(v) => format!("Conf({v})"),
    };
    let opt = |o: Option<bool>| o.map_or("None".to_string(), |v| format!("Some({v})"));
    let mut out = format!(
        "key={key:?} belief={} applied={applied} jp={} profile={:?} kind={:?} focus={} obs={} intent={} cand={} chord={} win={} repeat={}",
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
        s.was_down,
    );
    if let Some(r) = real {
        let _ = write!(out, " R={r}");
    }
    out
}

/// 代表例の「小ささ」: 基準状態（belief=false・applied=Unknown・jp=true・ImmCross・GJI・focus=Some・観測/意図なし・
/// 他は false）からずれているフィールドの数。小さいほど最小の代表例。
fn complexity(s: &PressState) -> u32 {
    u32::from(s.belief_open)
        + u32::from(s.applied != AppliedKnowledge::Unknown)
        + u32::from(!s.is_japanese_ime)
        + u32::from(s.profile != PressProfile::ImmCross)
        + u32::from(s.ime_kind != ImeKindId::Gji)
        + u32::from(!s.ime_identified)
        + u32::from(!s.current_focus_known)
        + u32::from(s.actuating_obs.is_some())
        + u32::from(s.intent.is_some())
        + u32::from(s.candidate_was_seen)
        + u32::from(s.ctrl_chord)
        + u32::from(s.win_held)
        + u32::from(s.was_down)
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
    /// (複雑さ, 例)。複雑さが最小のもの（同点は列挙順で最初）。
    example: Option<(u32, String)>,
    plausible_example: Option<(u32, String)>,
}

#[derive(Default)]
struct PropStat {
    checked: u64,
    violations: u64,
    plausible_violations: u64,
    classes: BTreeMap<&'static str, ClassStat>,
}

impl PropStat {
    fn add(&mut self, class: &'static str, s: &PressState, example: impl FnOnce() -> String) {
        let plausible = s.is_plausible();
        let score = complexity(s);
        self.violations += 1;
        if plausible {
            self.plausible_violations += 1;
        }
        let c = self.classes.entry(class).or_default();
        c.all += 1;
        if plausible {
            c.plausible += 1;
        }
        let better = |cur: &Option<(u32, String)>| cur.as_ref().is_none_or(|(sc, _)| score < *sc);
        let (b_all, b_pl) = (
            better(&c.example),
            plausible && better(&c.plausible_example),
        );
        if b_all || b_pl {
            let e = example();
            if b_all {
                c.example = Some((score, e.clone()));
            }
            if b_pl {
                c.plausible_example = Some((score, e));
            }
        }
    }
}

struct Report {
    p1: PropStat,
    /// 参考: L1 前（押下 ID なし）の P1。S-1 が L1 で 0 になったことを件数で残す。
    p1_pre_l1: PropStat,
    /// P1 の S-1 のプロファイル別件数（TSF 系だけであることを golden に固定する）。
    s1_by_profile: BTreeMap<String, u64>,
    /// 参考: L2 前（L1 まで）の P1。S-2・S-4 が L2 で 0 になったことを件数で残す。
    p1_pre_l2: PropStat,
    p1_fixed_point: PropStat,
    p2: PropStat,
    p3: PropStat,
    p4: PropStat,
    p5: PropStat,
    p6: PropStat,
    /// P5 の防御線: 予約（`PressLedger`）だけの評価。
    p5_ledger: PropStat,
    /// P5 の参考: 同一押下で向きが逆の衝突で、Engine が shadow を上書きして 2 回書いた（Engine の向きに収束した）件数。
    p5_engine_overrides: u64,
    /// P5 の参考: 向きが逆の衝突だが Engine 自身の書き込みが省略され（授権・Win キー等）、shadow の向きが残った件数
    /// （P1 のクラス〈S-2 等〉で扱う別の穴。L2 以降で減る）。
    p5_engine_write_elided: u64,
    states: u64,
    plausible_states: u64,
}

fn analyze() -> Report {
    let judge = StoreJudge::default();
    let mut rep = Report {
        p1: PropStat::default(),
        p1_pre_l1: PropStat::default(),
        s1_by_profile: BTreeMap::new(),
        p1_pre_l2: PropStat::default(),
        p1_fixed_point: PropStat::default(),
        p2: PropStat::default(),
        p3: PropStat::default(),
        p4: PropStat::default(),
        p5: PropStat::default(),
        p6: PropStat::default(),
        p5_ledger: PropStat::default(),
        p5_engine_overrides: 0,
        p5_engine_write_elided: 0,
        states: 0,
        plausible_states: 0,
    };
    for s in PressState::all() {
        rep.states += 1;
        if s.is_plausible() {
            rep.plausible_states += 1;
        }
        for key in ExplicitKey::ALL {
            let d1 = delivery(&judge, &s, key);

            // P1（現状）と、D4 固定点適用後の P1（参考）
            rep.p1.checked += 1;
            if let Some(class) = p1_class(&s, key, &d1) {
                if class == "S1_already_matched" {
                    *rep.s1_by_profile
                        .entry(format!("{:?}/{:?}", s.profile, s.ime_kind))
                        .or_default() += 1;
                }
                rep.p1.add(class, &s, || {
                    format!("{} -> {}", fmt_state(&s, key, None), fmt_delivery(&d1))
                });
            }
            let dpre = delivery_pre_l1(&judge, &s, key);
            rep.p1_pre_l1.checked += 1;
            if let Some(class) = p1_class(&s, key, &dpre) {
                rep.p1_pre_l1.add(class, &s, || {
                    format!("{} -> {}", fmt_state(&s, key, None), fmt_delivery(&dpre))
                });
            }
            let dpre2 = delivery_pre_l2(&judge, &s, key);
            rep.p1_pre_l2.checked += 1;
            if let Some(class) = p1_class(&s, key, &dpre2) {
                rep.p1_pre_l2.add(class, &s, || {
                    format!("{} -> {}", fmt_state(&s, key, None), fmt_delivery(&dpre2))
                });
            }
            let dfp = explicit_press_delivery_with(&s, key, &judge, DeliveryMode::PressId);
            rep.p1_fixed_point.checked += 1;
            if let Some(class) = p1_class(&s, key, &dfp) {
                rep.p1_fixed_point.add(class, &s, || {
                    format!("{} -> {}", fmt_state(&s, key, None), fmt_delivery(&dfp))
                });
            }

            // P6: 押下の書き込みの直後の自動リピート（`press=None`）。GjiDirect は `applied` の already-matched 省略で VK を
            // 追い送りしない（L1: 押下の書き込みだけが applied を未知にする。リピートは従来の省略）。トグルキーのリピートは
            // belief を反転し続ける既存の挙動（ADR-199 決定18(ii) は F13 だけを除外）、ImmCross/MsImeDirect に省略は無い（従来どおり）。
            if !s.was_down && !s.win_held && key.is_target_press_key() && d1.write.is_some() {
                let s1 = PressState {
                    was_down: true,
                    ..state_after_press(&s, key, &d1)
                };
                let d2 = delivery(&judge, &s1, key);
                rep.p6.checked += 1;
                if d2.write.is_some() {
                    let class = if key.meaning() == KeyMeaning::Toggle && key.is_shadow_path() {
                        "repeat_toggles_belief"
                    } else if s.chain_head_is_gji_direct() {
                        "repeat_rewrites_gji_direct"
                    } else {
                        "repeat_rewrites_no_applied_elision"
                    };
                    rep.p6.add(class, &s, || {
                        format!(
                            "{} -> 押下 {} / 直後のリピート {}",
                            fmt_state(&s, key, None),
                            fmt_delivery(&d1),
                            fmt_delivery(&d2)
                        )
                    });
                }
            }

            // 収束・不動点の検査は対象押下（非リピート・Win なし）だけ。
            if s.was_down
                || s.win_held
                || !key.is_target_press_key()
                || kanji_is_not_an_ime_key(&s, key)
            {
                continue;
            }
            let class_of =
                |st: &PressState, d: &Delivery| p1_class(st, key, d).unwrap_or("unclassified");

            // P2（絶対キー、実 IME の初期値 R を掛ける）/ P3（トグル）
            for r0 in [false, true] {
                match key.meaning() {
                    KeyMeaning::Absolute(t) => {
                        rep.p2.checked += 1;
                        let r1 = ime_after_press(r0, key, s.profile, &d1);
                        if r1 != t {
                            rep.p2.add(class_of(&s, &d1), &s, || {
                                format!(
                                    "{} -> {} (R: {r0} -> {r1}, 向き={t})",
                                    fmt_state(&s, key, Some(r0)),
                                    fmt_delivery(&d1)
                                )
                            });
                        }
                    }
                    KeyMeaning::Toggle => {
                        rep.p3.checked += 1;
                        let r1 = ime_after_press(r0, key, s.profile, &d1);
                        let s1 = state_after_press(&s, key, &d1);
                        let d2 = delivery(&judge, &s1, key);
                        let r2 = ime_after_press(r1, key, s.profile, &d2);
                        if r1 == r0 && r2 == r0 {
                            let class = p1_class(&s, key, &d1)
                                .or_else(|| p1_class(&s1, key, &d2))
                                .unwrap_or("unclassified");
                            rep.p3.add(class, &s, || {
                                format!(
                                    "{} -> 1回目 {} / 2回目 {} (R: {r0} -> {r1} -> {r2})",
                                    fmt_state(&s, key, Some(r0)),
                                    fmt_delivery(&d1),
                                    fmt_delivery(&d2)
                                )
                            });
                        }
                    }
                    KeyMeaning::NoIntent => {}
                }
            }

            // P4: 最大 3 押下で同じ違反を 2 回続けない（R は配送に影響しないので掛けない）。
            rep.p4.checked += 1;
            let mut cur = s;
            let mut prev: Option<&'static str> = None;
            let mut cur_d = d1;
            for press in 1..=3 {
                let class = p1_class(&cur, key, &cur_d).filter(|c| *c != "double_actuation");
                if let (Some(p), Some(c)) = (prev, class) {
                    if p == c {
                        rep.p4.add(c, &s, || {
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
                cur = state_after_press(&cur, key, &cur_d);
                cur_d = delivery(&judge, &cur, key);
            }

            // P5（BUG-113）: 同一押下で shadow 経路と Engine の SetOpen が両方来る構成。同じ向きの二重送信は無い。
            // 逆向きなら Engine の明示コンボが上書きする（最終の向きは Engine）。
            if key.is_shadow_path() && key.meaning() != KeyMeaning::NoIntent {
                for (engine_key, engine_open) in [
                    (ExplicitKey::EngineOn, true),
                    (ExplicitKey::EngineOff, false),
                ] {
                    // 本番: Engine が同じ打鍵の SetOpen を出すキーは、静的な事前問い合わせで shadow が書かない（M-4）。
                    rep.p5.checked += 1;
                    let resolved = dual_route_writes_with(&s, key, engine_key, &judge, PROD);
                    if let [Some(a), b] = resolved {
                        rep.p5
                            .add("shadow_writes_although_engine_owns_the_key", &s, || {
                                format!(
                                    "{} + {engine_key:?} -> shadow write={a} / engine write={b:?}",
                                    fmt_state(&s, key, None)
                                )
                            });
                    }
                    // 防御線: 事前問い合わせが効かなかった場合に、予約だけで同じ向きの二重送信を防げるか。
                    rep.p5_ledger.checked += 1;
                    let w = dual_route_writes_ledger_only(&s, key, engine_key, &judge, PROD);
                    match w {
                        [Some(a), Some(b)] if a == b => {
                            rep.p5_ledger
                                .add("bug113_double_send_same_direction", &s, || {
                                    format!(
                                        "{} + {engine_key:?} -> shadow write={a} / engine write={b}",
                                        fmt_state(&s, key, None)
                                    )
                                });
                        }
                        [Some(_), Some(_)] => rep.p5_engine_overrides += 1,
                        [Some(a), None] if a != engine_open => rep.p5_engine_write_elided += 1,
                        _ => {}
                    }
                }
            }
        }
    }
    rep
}

fn render(rep: &Report) -> String {
    let mut out = String::new();
    out.push_str(
        "# 明示キー押下の配送 現状の反例 golden (ADR-208 L3a 時点)\n\
         #\n\
         # 生成元: crates/awase-windows/tests/explicit_press_exhaustive.rs\n\
         # このファイルは自動生成される。更新は UPDATE_GOLDEN=1 で再生成すること。\n\
         #\n\
         # 状態空間(belief 2 × applied 5 × is_japanese 2 × profile 7 × (kind, TIP 同定) 3 × current_focus 2 × 観測 3 ×\n\
         # IntentStore 3 × candidate_was_seen 2 × chord 2 × win 2 × was_down 2) × キー 12 種を全列挙した、現状(ADR-208 L3a)の本番判断の合成結果。\n\
         # 反例は「分類 × 件数 + 各分類の最小の代表例(基準状態からのずれが最小)」で固定する(S-2 だけで状態空間の約半分が\n\
         # 反例なので行は列挙しない)。分類に当てはまらない反例(unclassified)が出たらテストが失敗する。\n\
         # L1 で S-1 は TsfNative×GJI(BUG-124 の実機 A/B〈ADR-208 L3'〉まで段階制御)を除いて 0 になった(P1-PreL1 が L1 前の件数)。L2 で S-2(is_japanese_ime=false)と S-4(current_focus=None)は 0 になった(P1-PreL2 が L2 前の件数)。L3 以降で穴を直すと該当クラスの件数が 0 に向かう(この差分が進捗)。\n\
         # 「起こりうる」= Blind プロファイル(Imm32Unavailable/TsfNative)で Actuating 観測が無い組み合わせ。\n\
         # 対象押下 = 非リピート・Win 押下なし・意図を持つキー。P1 の合格は Delivery が配送か書き込みのちょうど一方\n\
         # (Delivery::resolve が Ok)で、配送側なら前提 A1 のキー(0x16/0x1A・0xF0/F2・学習済み 0xF3/0xF4)。\n\
         # P5 は「同一押下で shadow 書き込みの後に Engine の SetOpen が続くとき、executor は押下前の applied を見る」という\n\
         # モデル(推測)。押下 ID の予約(L1)で同じ向きの二重送信は 0。逆向きは Engine が上書きする(件数は engine_overrides)。\n\
         # P6 は「押下の書き込みの直後の自動リピート」を 2 押下で見る。\n\
         #\n",
    );
    let _ = writeln!(
        out,
        "states\t{}\tplausible\t{}\tkeys\t{}\n",
        rep.states,
        rep.plausible_states,
        ExplicitKey::ALL.len()
    );
    let props: [(&str, &str, &PropStat); 10] = [
        (
            "P1",
            "INV-L1: 対象押下の Delivery が配送か書き込みのちょうど一方で、配送側なら A1 のキー",
            &rep.p1,
        ),
        (
            "P1-PreL1",
            "(参考) L1 前(押下 ID なし、ADR-208 L0 の現状)の P1。S-1 が L1 で解消した差分（TsfNative×GJI を除く）を残す",
            &rep.p1_pre_l1,
        ),
        (
            "P1-PreL2",
            "(参考) L2 前(L1 まで、押下の授権が is_japanese_ime・current_focus を問う)の P1。S-2・S-4 が L2 で解消した差分を残す",
            &rep.p1_pre_l2,
        ),
        (
            "P1-PreL3a",
            "(参考) L3a 前(L2 まで、D4 の固定点なし)の P1。S-3(IC の shadow no-op が書かず Suppress される)が L3a で解消した差分を残す",
            &rep.p1_fixed_point,
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
            "最大 3 押下で同じ違反を 2 回続けて繰り返さない",
            &rep.p4,
        ),
        (
            "P5",
            "同一押下で shadow 経路と Engine SetOpen が重なるキー（keys.ime_* 等）では、Engine への静的な事前問い合わせで shadow は書かない（書き込みは Engine の 1 回）",
            &rep.p5,
        ),
        (
            "P5-Ledger",
            "(防御線) 事前問い合わせが効かなかったとき、予約だけで同じ向きの二重送信を防げる（逆向きは Engine が上書きして 2 回。同期で何も送らなかったときは予約を解く）",
            &rep.p5_ledger,
        ),
        (
            "P6",
            "押下の書き込みの直後の自動リピート（press=None）で、GjiDirect は VK を追い送りしない（applied の省略）",
            &rep.p6,
        ),
    ];
    for (id, desc, stat) in props {
        let _ = writeln!(out, "## {id}: {desc}");
        if id == "P1" {
            for (k, n) in &rep.s1_by_profile {
                let _ = writeln!(out, "info\ts1_profile/kind\t{k}\t{n}");
            }
        }
        if id == "P5-Ledger" {
            let _ = writeln!(
                out,
                "info\tengine_overrides\t{}\t(向きが逆で Engine が shadow を上書きして 2 回書き、最終の向きが Engine)",
                rep.p5_engine_overrides
            );
            let _ = writeln!(
                out,
                "info\tengine_write_elided\t{}\t(向きが逆だが Engine 自身の書き込みが省略され shadow の向きが残る。授権・Win キー等の別の穴)",
                rep.p5_engine_write_elided
            );
        }
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
                .map_or("(P5/P6)", |(_, d)| *d);
            let _ = writeln!(
                out,
                "class\t{name}\tall\t{}\tplausible\t{}",
                c.all, c.plausible
            );
            let _ = writeln!(out, "  # {desc}");
            if let Some((_, e)) = &c.example {
                let _ = writeln!(out, "  minimal_example: {e}");
            }
            if let Some((_, e)) = &c.plausible_example {
                if Some(e) != c.example.as_ref().map(|(_, e)| e) {
                    let _ = writeln!(out, "  minimal_plausible_example: {e}");
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
        ("P1-PreL1", &rep.p1_pre_l1),
        ("P1-PreL2", &rep.p1_pre_l2),
        ("P1-PreL3a", &rep.p1_fixed_point),
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
                .map(|(_, e)| e)
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
    for s in PressState::all().filter(|s| !s.win_held && !s.was_down) {
        for key in ExplicitKey::ALL {
            let KeyMeaning::Absolute(t) = key.meaning() else {
                continue;
            };
            let d = delivery(&judge, &s, key);
            let p1_ok = p1_class(&s, key, &d).is_none();
            for r0 in [false, true] {
                if p1_ok {
                    assert_eq!(
                        ime_after_press(r0, key, s.profile, &d),
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
            // 未同定かつ `is_japanese_ime=false` の 0x19 は受動（ADR-208 L2 M-1）。IME の開閉キーではないので常に素通し。
            if kanji_is_not_an_ime_key(&s, key) {
                assert_eq!(d.physical, Physical::Allow, "{key:?} {s:?}");
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
                    // 自動リピートの Down は昇格しないが、ラッチ由来の `shadow_action` で Suppress される。
                    let expected = if d.shadow_toggled || s.was_down {
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

/// P5（BUG-113）: 同一押下で shadow 経路と Engine SetOpen が重なるキーでは、書き込みは Engine の 1 回だけ（M-4: shadow は
/// 書く前に静的に抑止される）。防御線（予約）だけでも同じ向きの二重送信は無い。向きが逆の衝突は Engine が上書きする。
#[test]
fn p5_same_press_sends_once() {
    let rep = analyze();
    assert_eq!(rep.p5.violations, 0, "{:?}", rep.p5.classes.keys());
    assert_eq!(
        rep.p5_ledger.violations,
        0,
        "{:?}",
        rep.p5_ledger.classes.keys()
    );
    // 防御線の衝突の優先順位が実際に働いている（空振りしていない）。
    assert!(
        rep.p5_engine_overrides > 0,
        "向きが逆の衝突が 1 件も無い=モデルが衝突を作れていない"
    );
}

/// M-1: 書かなかった重複（`AlreadyWrittenThisPress`）の完了は applied を動かさない（`AlreadyMatched` だと実 IME に書いていない
/// 押下が applied=Confirmed になる）。完了は実物の `ImeModel` の遷移（`reduce`）を通して検査する。全状態 × 先行の予約の向きで固定。
#[test]
fn duplicate_completion_never_confirms_applied() {
    let judge = StoreJudge::default();
    let mut checked = 0u64;
    for s in PressState::all().filter(|s| !s.was_down && !s.win_held) {
        for key in [ExplicitKey::EngineOn, ExplicitKey::EngineOff] {
            for claimed in [false, true] {
                let d = explicit_press_delivery_after(&s, key, &judge, PROD, Some(claimed));
                if d.reason == ElisionReason::AlreadyWrittenThisPress {
                    checked += 1;
                    assert_eq!(
                        state_after_press(&s, key, &d).applied,
                        s.applied,
                        "書かなかった重複の完了が applied を動かした: {}",
                        fmt_state(&s, key, None)
                    );
                }
            }
        }
    }
    assert!(checked > 0);
}

/// M-2: 同期の書き込みが何も送らなかったなら予約を解き、同じ押下の Engine 経路が改めて書く判断に進む（「絶対指定は 1 回」）。
/// 非同期（ImmCross 先頭）は解けないので Engine 経路は省かれる（次の押下で直る。ADR-208 の例外）。
#[test]
fn sync_route_that_sent_nothing_releases_the_reservation_but_async_does_not() {
    let judge = StoreJudge::default();
    // Win キー押下中（`UnsafeToToggle`。L2 で押下の授権は常に下りるので、同期で何も送らない代表は Win 押下）の
    // shadow 経路: 何も送らない。
    let base = PressState {
        belief_open: false,
        applied: AppliedKnowledge::Unknown,
        is_japanese_ime: false,
        profile: PressProfile::ImmUnavailable,
        ime_kind: ImeKindId::Gji,
        ime_identified: true,
        current_focus_known: false,
        actuating_obs: None,
        intent: None,
        candidate_was_seen: false,
        ctrl_chord: false,
        win_held: true,
        was_down: false,
    };
    for (profile, expect_released) in [
        (PressProfile::ImmUnavailable, true),
        (PressProfile::ImmUnavailableTsfClass, true),
        (PressProfile::TsfNative, true),
        (PressProfile::ImmCross, false),
    ] {
        let s = PressState { profile, ..base };
        let d_shadow = delivery(&judge, &s, ExplicitKey::StaticOn);
        assert_eq!(d_shadow.reason, ElisionReason::WinHeld, "{profile:?}");
        assert_eq!(d_shadow.reserved, Some(true), "{profile:?}: 予約はした");
        let claimed = reservation_after_route(&s, &d_shadow);
        assert_eq!(claimed.is_none(), expect_released, "{profile:?}");
        let d_engine =
            explicit_press_delivery_after(&s, ExplicitKey::EngineOn, &judge, PROD, claimed);
        if expect_released {
            assert_ne!(
                d_engine.reason,
                ElisionReason::AlreadyWrittenThisPress,
                "{profile:?}: 解いたので Engine は改めて書く判断に進む"
            );
        } else {
            assert_eq!(
                d_engine.reason,
                ElisionReason::AlreadyWrittenThisPress,
                "{profile:?}: 非同期は予約を解けない"
            );
        }
    }
}

/// M-3: Windows Terminal 等（`Imm32Unavailable` に分類されるが実質 TSF）も、TsfNative と同じく L3' まで Engine 経路を
/// 未知化しない（S-1 が残るのは実質 TSF の窓だけで、他は 0）。
#[test]
fn effectively_tsf_native_class_is_staged_like_tsf_native() {
    let judge = StoreJudge::default();
    let mut found = 0u64;
    for s in PressState::all()
        .filter(|s| s.profile == PressProfile::ImmUnavailableTsfClass && !s.was_down && !s.win_held)
    {
        let twin = PressState {
            profile: PressProfile::TsfNative,
            ..s
        };
        for key in [ExplicitKey::EngineOn, ExplicitKey::EngineOff] {
            let a = delivery(&judge, &s, key);
            let b = delivery(&judge, &twin, key);
            // Engine 経路の未知化は TsfNative と同じ判断（物理・書き込み・理由が一致）。
            assert_eq!(
                (a.write, a.reason),
                (b.write, b.reason),
                "{}",
                fmt_state(&s, key, None)
            );
            found += u64::from(a.reason == ElisionReason::AlreadyMatched);
        }
    }
    assert!(
        found > 0,
        "実質 TSF のクラスで S-1 の段階制御が効いていない"
    );
}

/// L1 前（押下 ID なし）は同一押下の二重送信が実際に起きる（P5 が意味のあるテストであることの対照）。
#[test]
fn p5_pre_l1_double_sends_exist() {
    let judge = StoreJudge::default();
    let mut doubles = 0u64;
    for s in PressState::all().filter(|s| !s.was_down && !s.win_held) {
        for key in [
            ExplicitKey::StaticOn,
            ExplicitKey::StaticOff,
            ExplicitKey::SyncOn,
            ExplicitKey::SyncOff,
        ] {
            for engine_key in [ExplicitKey::EngineOn, ExplicitKey::EngineOff] {
                if let [Some(_), Some(_)] =
                    dual_route_writes_with(&s, key, engine_key, &judge, DeliveryMode::Legacy)
                {
                    doubles += 1;
                }
            }
        }
    }
    assert!(
        doubles > 0,
        "L1 前に二重送信が無いなら P5 は検査になっていない"
    );
}

/// S-1（GjiDirect の already-matched。Engine 経由の絶対キー × 古い applied）は L1 で解消する。ただし TsfNative の窓は
/// BUG-124 型の「@」の実機 A/B（ADR-208 L3'）が済むまで Engine 経路の未知化を止めている
/// （`engine_press_unknowns_applied`）ので、残る S-1 は TsfNative × GJI の Engine 経路だけ。L1 前は全プロファイルで非ゼロ。
#[test]
fn s1_already_matched_is_resolved_by_l1_except_tsf_native() {
    let judge = StoreJudge::default();
    let mut residual_tsf_native = 0u64;
    for s in PressState::all().filter(|s| !s.was_down && !s.win_held) {
        for key in [ExplicitKey::EngineOn, ExplicitKey::EngineOff] {
            let d = delivery(&judge, &s, key);
            if p1_class(&s, key, &d) == Some("S1_already_matched") {
                assert_eq!(
                    (s.profile.is_effectively_tsf_native(), s.ime_kind),
                    (true, ImeKindId::Gji),
                    "実質 TsfNative×GJI 以外に S-1 が残っています: {}",
                    fmt_state(&s, key, None)
                );
                residual_tsf_native += 1;
            }
        }
    }
    assert!(
        residual_tsf_native > 0,
        "TsfNative の段階制御が効いていない（L3' 前は S-1 が残る）"
    );
    // L2 前の S-1 は `is_japanese_ime=true` の状態にしか現れなかった（偽は S-2 の Unwarranted が先に出た）ので、L1 が
    // S-1 を減らしたかは `is_japanese_ime=true` の状態どうしで比べる（L2 で S-2 から S-1 へ移ってきた分を含めない）。
    let mut pre = 0u64;
    let mut now = 0u64;
    for s in PressState::all().filter(|s| !s.was_down && !s.win_held && s.is_japanese_ime) {
        for key in [ExplicitKey::EngineOn, ExplicitKey::EngineOff] {
            if p1_class(&s, key, &delivery_pre_l1(&judge, &s, key)) == Some("S1_already_matched") {
                pre += 1;
            }
            if p1_class(&s, key, &delivery(&judge, &s, key)) == Some("S1_already_matched") {
                now += 1;
            }
        }
    }
    assert!(now < pre, "L1 が S-1 を減らしていない: pre={pre} now={now}");
}

/// P6: 押下の書き込みの直後の自動リピートで、GjiDirect は VK を追い送りしない。
#[test]
fn p6_repeat_never_rewrites_gji_direct() {
    let rep = analyze();
    assert!(
        !rep.p6.classes.contains_key("repeat_rewrites_gji_direct"),
        "{:?}",
        rep.p6
            .classes
            .get("repeat_rewrites_gji_direct")
            .and_then(|c| c.example.as_ref())
    );
    assert!(rep.p6.checked > 0);
}

/// L1 が L0 から変えるのは（L2 前の判断どうしの比較）、押下 ID を持つ押下（非リピート）の Engine 経路の already-matched 省略（S-1。TsfNative を除く）だけ。
/// shadow 経路は押下 ID あり（非リピート）なら L0 と同じ（従来から無条件に降格していた）、Engine 経路はリピート（`press=None`）
/// なら L0 と同じ（`applied` の省略のまま）。
#[test]
fn l1_changes_only_the_press_engine_already_matched_elision() {
    let judge = StoreJudge::default();
    for s in PressState::all() {
        for key in ExplicitKey::ALL {
            let pre = delivery_pre_l1(&judge, &s, key);
            let l1 = delivery_pre_l2(&judge, &s, key);
            // `reserved`（予約の記録）は L1 で増えた帳簿で、書く/書かないの判断ではない。
            let l1 = Delivery {
                reserved: None,
                ..l1
            };
            if key.is_shadow_path() {
                if !s.was_down {
                    assert_eq!(
                        pre,
                        l1,
                        "{} 非リピートの shadow 経路は L0 と同じ",
                        fmt_state(&s, key, None)
                    );
                }
                // リピートの shadow 経路は `applied` の省略（従来の無条件降格をやめる）ので、書いたかが変わりうる。
            } else if s.was_down
                || pre.reason != ElisionReason::AlreadyMatched
                || s.profile.is_effectively_tsf_native()
            {
                assert_eq!(
                    (pre.physical, pre.write, pre.reason, pre.belief_after),
                    (l1.physical, l1.write, l1.reason, l1.belief_after),
                    "{} Engine 経路は S-1（押下の already-matched。TsfNative は L3' まで対象外）以外は L0 と同じ",
                    fmt_state(&s, key, None)
                );
            } else {
                // S-1: 押下の書き込みは already-matched で省かれず、授権・Win キーの判定へ進む。
                assert_ne!(
                    l1.reason,
                    ElisionReason::AlreadyMatched,
                    "{}",
                    fmt_state(&s, key, None)
                );
            }
        }
    }
}

/// 同一押下の 2 経路の評価順（shadow → Engine）で、Engine が先に予約した逆向きを shadow が上書きしない
/// （現状の順序ではありえないが、`PressLedger` の優先順位が本番の呼び出し側の前提）。
#[test]
fn shadow_never_overrides_an_earlier_engine_reservation() {
    let judge = StoreJudge::default();
    let base = PressState {
        belief_open: false,
        applied: AppliedKnowledge::Unknown,
        is_japanese_ime: true,
        profile: PressProfile::ImmCross,
        ime_kind: ImeKindId::Gji,
        ime_identified: true,
        current_focus_known: true,
        actuating_obs: None,
        intent: None,
        candidate_was_seen: false,
        ctrl_chord: false,
        win_held: false,
        was_down: false,
    };
    // Engine が OFF（false）を予約済みの押下に、shadow の ON（0x16）が来ても書かない。
    let d = explicit_press_delivery_after(&base, ExplicitKey::StaticOn, &judge, PROD, Some(false));
    assert_eq!(d.write, None);
    assert_eq!(d.reason, ElisionReason::AlreadyWrittenThisPress);
    // 同じ向きでも書かない。リピート（press なし）は予約を見ない。
    let d = explicit_press_delivery_after(&base, ExplicitKey::StaticOn, &judge, PROD, Some(true));
    assert_eq!(d.write, None);
    let rep = PressState {
        was_down: true,
        ..base
    };
    let d = explicit_press_delivery_after(&rep, ExplicitKey::StaticOn, &judge, PROD, Some(true));
    assert_ne!(d.reason, ElisionReason::AlreadyWrittenThisPress);
}

/// L2（ADR-208 決定2 D2・D3）が L1 から変えるのは、押下 ID を持つ押下（非リピート）の授権（`is_japanese_ime`・`current_focus` を
/// 問わない）と、shadow 昇格の `is_japanese_ime` 条件の撤去（0x19・0xF3/F4・F13）だけ。リピート（`press=None`）は L1 と同じ。
/// 押下の order は（真の安全弁を除き、全列挙のモデルには無い）`Unwarranted` で止まらない。
#[test]
fn l2_changes_only_the_press_warrant_and_the_shadow_promotion() {
    let judge = StoreJudge::default();
    let mut changed = 0u64;
    for s in PressState::all() {
        for key in ExplicitKey::ALL {
            let l1 = delivery_pre_l2(&judge, &s, key);
            let l2 = explicit_press_delivery_with(&s, key, &judge, DeliveryMode::PressId);
            if s.was_down {
                // 変わるのは未同定かつ `is_japanese_ime=false` の 0x19 の物理（受動 → 素通し、M-1）だけ。
                if kanji_is_not_an_ime_key(&s, key) {
                    assert_eq!(l2.physical, Physical::Allow);
                    assert_eq!((l1.write, l1.reason), (l2.write, l2.reason));
                } else {
                    assert_eq!(l1, l2, "{} リピートは L1 と同じ", fmt_state(&s, key, None));
                }
                continue;
            }
            assert_ne!(
                l2.reason,
                ElisionReason::Unwarranted,
                "{} 押下の order は授権される（D2・D3）",
                fmt_state(&s, key, None)
            );
            if l1 != l2 {
                changed += 1;
                // 変わるのは「授権されなかった」か「昇格しなかった」押下だけ。
                assert!(
                    matches!(
                        l1.reason,
                        ElisionReason::Unwarranted | ElisionReason::NotPromoted
                    ),
                    "{} -> L1 {:?} / L2 {:?}",
                    fmt_state(&s, key, None),
                    l1,
                    l2
                );
            }
        }
    }
    assert!(changed > 0);
}

/// S-2（`is_japanese_ime=false`）と S-4（`current_focus=None`）は L2 で 0 件になる（残る反例は別クラス）。
#[test]
fn s2_and_s4_are_resolved_by_l2() {
    let rep = analyze();
    for class in ["S2_not_japanese", "S4_focus_none_unwarranted"] {
        let pre = rep.p1_pre_l2.classes.get(class).map_or(0, |c| c.all);
        let now = rep.p1.classes.get(class).map_or(0, |c| c.all);
        assert!(pre > 0, "{class}: L2 前の件数が 0（モデルの更新漏れ）");
        assert_eq!(
            now,
            0,
            "{class} が L2 で解消していない: {:?}",
            rep.p1.classes.get(class).and_then(|c| c.example.as_ref())
        );
    }
}

/// 0x19 は TIP 未同定かつ `is_japanese_ime=false` のときだけ受動（昇格せず、物理は素通し。ADR-208 L2 M-1、所有者決定）。
/// 同定済みなら `is_japanese_ime` の誤判定でも昇格して書く。
#[test]
fn kanji_is_promoted_when_identified_and_passed_through_when_unidentified() {
    let judge = StoreJudge::default();
    let mut passive = 0u64;
    for s in PressState::all().filter(|s| !s.was_down && !s.win_held) {
        let d = delivery(&judge, &s, ExplicitKey::Kanji);
        let promoted = d.reason != ElisionReason::NotPromoted;
        assert_eq!(
            promoted,
            s.is_japanese_ime || s.ime_identified,
            "{}",
            fmt_state(&s, ExplicitKey::Kanji, None)
        );
        if kanji_is_not_an_ime_key(&s, ExplicitKey::Kanji) {
            // 飲み込まない（二重の空振りにならない）: 物理は Down/Up とも素通し（Allow）で awase は何も書かない。
            assert_eq!(
                (d.physical, d.write),
                (Physical::Allow, None),
                "{}",
                fmt_state(&s, ExplicitKey::Kanji, None)
            );
            passive += 1;
        }
    }
    assert!(passive > 0);
}

/// L3a（ADR-208 決定2 D4）が L2 から変えるのは、shadow の no-op（belief が既に向きと一致）で `plan(shadow_toggled=false)` が
/// Suppress の非リピートの押下が書くようになる（S-3）ことだけ。物理の配送は変わらない（後段の `plan(true)` も Suppress）。
/// Allow の窓・リピート・TsfNative（BUG-124 の実機 A/B〈L3'〉まで段階制御）・Engine 経路は L2 と同じ。
#[test]
fn l3a_changes_only_the_suppressed_shadow_noop_write() {
    let judge = StoreJudge::default();
    let mut written = 0u64;
    let (mut changed, mut tsf_kept, mut repeat_kept, mut allow_kept) = (0u64, 0u64, 0u64, 0u64);
    for s in PressState::all() {
        for key in ExplicitKey::ALL {
            let l2 = explicit_press_delivery_with(&s, key, &judge, DeliveryMode::PressId);
            let l3a = delivery(&judge, &s, key);
            assert_eq!(
                l2.physical,
                l3a.physical,
                "{} 物理の配送は変わらない",
                fmt_state(&s, key, None)
            );
            let noop_suppressed =
                l2.reason == ElisionReason::ShadowNoop && l2.physical == Physical::Suppress;
            if noop_suppressed && !s.was_down && !s.profile.is_effectively_tsf_native() {
                changed += 1;
                assert_ne!(
                    l3a.reason,
                    ElisionReason::ShadowNoop,
                    "{} Suppress される no-op は書く判断に回る",
                    fmt_state(&s, key, None)
                );
                assert!(l3a.shadow_toggled);
                // M-1/m-2: 書いた向きはキーの意味（belief の向きではない）。書かない場合（授権・gate）も逆向きは書かない。
                assert!(
                    l3a.write.is_none() || l3a.write == l3a.target,
                    "{} 書いた向きがキーの意味と一致しない: {l3a:?}",
                    fmt_state(&s, key, None)
                );
                written += u64::from(l3a.write.is_some());
            } else {
                assert_eq!(l2, l3a, "{}", fmt_state(&s, key, None));
                if noop_suppressed {
                    if s.was_down {
                        repeat_kept += 1;
                    } else {
                        tsf_kept += 1;
                    }
                }
                if l2.reason == ElisionReason::ShadowNoop && l2.physical == Physical::Allow {
                    allow_kept += 1;
                }
            }
        }
    }
    assert!(
        // tsf_kept は現状 0（TsfNative で Suppress される no-op はモデルに無い: Suppress されるのはトグルキーで、トグルは必ず belief を倒す）。
        // TsfNative の除外は `shadow_noop_write_target` の単体テストが固定する（L3' で定数を反転するときの足場）。
        changed > 0 && written > 0 && repeat_kept > 0 && allow_kept > 0,
        "{changed} {tsf_kept} {repeat_kept} {allow_kept}"
    );
}
