//! ADR-280 D2: 入力言語の判定まわりの純粋な関数の全列挙（現状の挙動の固定。挙動は変えない）。
//!
//! - `HoldController::on_key`/`on_timer` の `HoldStep`: 契機（窓切替/idle）× 判定（Japanese/NotJapanese/Undecided）×
//!   保持の可否。加えて、保持中の期限（同じ窓/別の窓）と、保持中の窓切替。
//! - `hiragana_write_skip_reason`/`eisu_write_skip_reason`（共通の `write_skip_reason`）: `WriteFacts` の 6 つの bool の
//!   2^6 = 64 通り。
//!
//! 期待値は実装の分岐を写さず、ADR-263 の doc と各関数の doc の仕様から、宣言的な表として別の形で書く。
//! 判定器（`EpisodeDetector`）は golden の外で、判定を引き起こす入力列だけを用意する。

use awase::engine::fsm_types::ThumbSide;
use awase::lang_detect::{
    eisu_write_skip_reason, hiragana_write_skip_reason, DetectParams, HoldController, HoldStep,
    KanaModel, KeyRole, ObservedKey, StopReason, ThumbOverlap, TrackerConfig, Trigger, Verdict,
    WriteFacts,
};
use awase::ngram::NgramModel;
use awase::scanmap::PhysicalPos;
use awase::yab::{YabFace, YabValue};

fn pos(row: u8, col: u8) -> PhysicalPos {
    PhysicalPos::new(row, col)
}

/// か(1,1) た(1,2) こ(1,3) さ(1,4)。「かた」「たこ」「こさ」が自然。さ は行頭の尤度が低い。
fn model() -> KanaModel {
    let ngram = NgramModel::from_csv("2,かた,0.0\n2,たこ,0.0\n2,こさ,0.0\n2,かこ,-3.0\n", 0, 0, 0)
        .expect("csv");
    let mut face = YabFace::new();
    for (col, (romaji, kana)) in [("ka", 'か'), ("ta", 'た'), ("ko", 'こ'), ("sa", 'さ')]
        .into_iter()
        .enumerate()
    {
        face.insert(
            pos(1, u8::try_from(col + 1).expect("col")),
            YabValue::Romaji {
                romaji: romaji.to_string(),
                kana: Some(kana),
            },
        );
    }
    KanaModel::new(&ngram, &face)
}

fn key(role: KeyRole, down: bool, t: u64) -> ObservedKey {
    ObservedKey {
        role,
        down,
        excluded: false,
        timestamp_ms: t,
    }
}

fn cfg(ja: f64, en: f64, max_keys: usize) -> TrackerConfig {
    TrackerConfig {
        params: DetectParams {
            ja_threshold: ja,
            en_threshold: en,
            max_keys,
        },
        thumb_overlap: ThumbOverlap::Enabled,
        idle_threshold_ms: 3000,
        max_ms: 2500,
    }
}

const SCOPE: u64 = 1;
const OTHER_SCOPE: u64 = 2;

/// 契機を作る。窓切替: 最初の打鍵がそのまま窓切替（直前の窓が無い）。idle: 同じ窓で矢印キー（エピソードにならない）を
/// 打ったあと、idle 閾値以上あけて始める。`t0` は最初の材料の打鍵の時刻。流した打鍵は保持されず通る（`pass`）。
fn arm(c: &mut HoldController<u32>, m: &KanaModel, trigger: Trigger) -> u64 {
    match trigger {
        Trigger::WindowSwitch => 10,
        Trigger::Idle => {
            let s = c.on_key(m, key(KeyRole::Other, true, 0), 100, SCOPE, true);
            assert_eq!(
                s.pass,
                Some(100),
                "idle の準備の打鍵はエピソードにならず通る"
            );
            assert!(!s.holding && s.release.is_empty() && s.outcomes.is_empty());
            3010
        }
    }
}

const TRIGGERS: [Trigger; 2] = [Trigger::WindowSwitch, Trigger::Idle];

/// `HoldStep` の期待値（`outcomes` は (契機, 判定, 止まった理由) の列）。
#[derive(Debug, Default)]
struct Exp {
    pass: Option<u32>,
    release: Vec<u32>,
    discarded: usize,
    outcomes: Vec<(Verdict, StopReason)>,
    japanese: bool,
    english: bool,
    holding: bool,
    started_holding: bool,
}

fn check(step: &HoldStep<u32>, exp: &Exp, trigger: Trigger, label: &str) {
    assert_eq!(step.pass, exp.pass, "{label}: pass");
    assert_eq!(step.release, exp.release, "{label}: release");
    assert_eq!(step.discarded, exp.discarded, "{label}: discarded");
    assert_eq!(step.japanese, exp.japanese, "{label}: japanese");
    assert_eq!(step.english, exp.english, "{label}: english");
    assert_eq!(step.holding, exp.holding, "{label}: holding");
    assert_eq!(
        step.started_holding, exp.started_holding,
        "{label}: started_holding"
    );
    let got: Vec<_> = step.outcomes.iter().map(|o| (o.verdict, o.stop)).collect();
    assert_eq!(got, exp.outcomes, "{label}: outcomes");
    for o in &step.outcomes {
        assert_eq!(o.trigger, trigger, "{label}: outcome の契機");
    }
}

#[derive(Debug, Clone, Copy)]
enum Want {
    Japanese,
    NotJapanese,
    Undecided,
}

/// 判定ごとの設定。
fn scenario_cfg(want: Want) -> TrackerConfig {
    match want {
        Want::Japanese => cfg(100.0, 100.0, 7),
        // さ は行頭の尤度が低く、en_threshold = 5 で 1 打目に確定
        Want::NotJapanese => cfg(100.0, 5.0, 7),
        // max_keys = 1 で、1 打目に決まらないまま上限
        Want::Undecided => cfg(100.0, 100.0, 1),
    }
}

/// その判定に達する打鍵列（最後の打鍵で確定する）。
fn scenario_keys(want: Want, t0: u64) -> Vec<ObservedKey> {
    match want {
        // か を押したまま左親指 = 重なり（2 打目で確定）
        Want::Japanese => vec![
            key(KeyRole::Char(pos(1, 1)), true, t0),
            key(KeyRole::Thumb(ThumbSide::Left), true, t0 + 5),
        ],
        Want::NotJapanese => vec![key(KeyRole::Char(pos(1, 4)), true, t0)],
        Want::Undecided => vec![key(KeyRole::Char(pos(1, 1)), true, t0)],
    }
}

/// 仕様の表: 判定 × 保持の可否 → 最終打鍵の期待値。保持していたエピソードだけが `japanese`/`english` を立て、
/// 保持していたぶんを（保持した順に）解放する。保持しないエピソードは各打鍵がそのまま通る（観測だけ）。
/// `n` は列の打鍵数。打鍵 `i` のイベントは `i + 1`。
fn expected_final(want: Want, may_hold: bool, n: u32) -> Exp {
    let (verdict, stop) = match want {
        Want::Japanese => (
            Verdict::Japanese(awase::lang_detect::JapaneseEvidence::ThumbOverlap),
            StopReason::Decided,
        ),
        Want::NotJapanese => (Verdict::NotJapanese, StopReason::Decided),
        Want::Undecided => (Verdict::Undecided, StopReason::MaxKeys),
    };
    if may_hold {
        Exp {
            release: (1..=n).collect(),
            outcomes: vec![(verdict, stop)],
            japanese: matches!(want, Want::Japanese),
            english: matches!(want, Want::NotJapanese),
            ..Exp::default()
        }
    } else {
        Exp {
            pass: Some(n),
            outcomes: vec![(verdict, stop)],
            ..Exp::default()
        }
    }
}

#[test]
fn hold_step_for_every_trigger_verdict_and_may_hold() {
    let m = model();
    let mut cases = 0;
    for trigger in TRIGGERS {
        for want in [Want::Japanese, Want::NotJapanese, Want::Undecided] {
            for may_hold in [false, true] {
                cases += 1;
                let label = format!("{trigger:?}/{want:?}/may_hold={may_hold}");
                let mut c: HoldController<u32> = HoldController::new(scenario_cfg(want));
                let t0 = arm(&mut c, &m, trigger);
                let keys = scenario_keys(want, t0);
                let n = u32::try_from(keys.len()).expect("n");
                for (i, k) in keys.iter().enumerate() {
                    let ev = u32::try_from(i).expect("i") + 1;
                    let step = c.on_key(&m, *k, ev, SCOPE, may_hold);
                    if u32::try_from(i).expect("i") + 1 < n {
                        // 確定前の打鍵: 保持するなら保持（開始の打鍵だけ started_holding）、しないなら通す。
                        let exp = if may_hold {
                            Exp {
                                holding: true,
                                started_holding: i == 0,
                                ..Exp::default()
                            }
                        } else {
                            Exp {
                                pass: Some(ev),
                                ..Exp::default()
                            }
                        };
                        check(&step, &exp, trigger, &format!("{label}/step{i}"));
                    } else {
                        // 1 打で確定する列では、確定の打鍵が開始の打鍵でもある。
                        let mut exp = expected_final(want, may_hold, n);
                        exp.started_holding = may_hold && n == 1;
                        check(&step, &exp, trigger, &format!("{label}/final"));
                    }
                }
                assert!(!c.is_holding(), "{label}: 確定後は保持しない");
                assert_eq!(c.held_len(), 0, "{label}: 確定後は保持列が空");
            }
        }
    }
    assert_eq!(cases, 12);
}

/// 保持中の期限（`on_timer`）: 判定は出ず（Pending のまま閉じる）、`japanese`/`english` は立たない。
/// 同じ窓なら保持を解放（Timeout）、別の窓なら捨てる（WindowChanged）。保持していないなら、どちらも空。
#[test]
fn on_timer_for_every_trigger_may_hold_and_scope() {
    let m = model();
    let mut cases = 0;
    for trigger in TRIGGERS {
        for may_hold in [false, true] {
            for same_scope in [true, false] {
                cases += 1;
                let label = format!("{trigger:?}/may_hold={may_hold}/same_scope={same_scope}");
                let mut c: HoldController<u32> = HoldController::new(cfg(100.0, 100.0, 7));
                let t0 = arm(&mut c, &m, trigger);
                // 決まらない 1 打（か のみ）。
                let first = c.on_key(
                    &m,
                    key(KeyRole::Char(pos(1, 1)), true, t0),
                    1,
                    SCOPE,
                    may_hold,
                );
                assert_eq!(first.holding, may_hold, "{label}: 開始の打鍵");
                let scope = if same_scope { SCOPE } else { OTHER_SCOPE };
                let step = c.on_timer(scope);
                let stop = if same_scope {
                    StopReason::Timeout
                } else {
                    StopReason::WindowChanged
                };
                let exp = Exp {
                    release: if may_hold && same_scope {
                        vec![1]
                    } else {
                        vec![]
                    },
                    discarded: usize::from(may_hold && !same_scope),
                    outcomes: vec![(Verdict::Pending, stop)],
                    ..Exp::default()
                };
                check(&step, &exp, trigger, &label);
                assert!(
                    !c.is_holding() && c.held_len() == 0,
                    "{label}: 期限のあとは保持しない"
                );
            }
        }
    }
    assert_eq!(cases, 8);

    // エピソードが無いときのタイマーは何もしない。
    let mut c: HoldController<u32> = HoldController::new(cfg(100.0, 100.0, 7));
    check(
        &c.on_timer(SCOPE),
        &Exp::default(),
        Trigger::WindowSwitch,
        "no episode",
    );
}

/// 保持中に別の窓の打鍵が来た: 保持していた打鍵は捨て（`discarded`）、止まった理由は WindowChanged。
/// その打鍵は新しい窓の窓切替の契機で新しいエピソードを始める（保持するかは `may_hold`）。
#[test]
fn key_in_another_window_during_an_episode_discards_held_keys() {
    let m = model();
    let mut cases = 0;
    for trigger in TRIGGERS {
        for may_hold in [false, true] {
            cases += 1;
            let label = format!("{trigger:?}/may_hold={may_hold}");
            let mut c: HoldController<u32> = HoldController::new(cfg(100.0, 100.0, 7));
            let t0 = arm(&mut c, &m, trigger);
            c.on_key(
                &m,
                key(KeyRole::Char(pos(1, 1)), true, t0),
                1,
                SCOPE,
                may_hold,
            );
            let step = c.on_key(
                &m,
                key(KeyRole::Char(pos(1, 2)), true, t0 + 10),
                2,
                OTHER_SCOPE,
                may_hold,
            );
            // 閉じた古いエピソードの契機は元のもの、新しいエピソードは窓切替。outcomes の契機は古い側だけ。
            let exp = Exp {
                pass: if may_hold { None } else { Some(2) },
                discarded: usize::from(may_hold),
                outcomes: vec![(Verdict::Pending, StopReason::WindowChanged)],
                holding: may_hold,
                started_holding: may_hold,
                ..Exp::default()
            };
            check(&step, &exp, trigger, &label);
            assert_eq!(
                c.held_len(),
                usize::from(may_hold),
                "{label}: 新しいエピソードの保持列"
            );
        }
    }
    assert_eq!(cases, 4);
}

// ---- write_skip_reason ----

/// 優先順位の表（上から順に最初に当たった理由）。`explicit` は逆向きの明示操作の理由の名前（関数ごとに違う）。
fn priority_table(explicit: &'static str) -> [(fn(&WriteFacts) -> bool, &'static str); 5] {
    [
        (|f| !f.belief_open, "skipped-belief-off"),
        // 観測の無い窓だけが対象。対象でも NonText は外す。
        (
            |f| !f.unobserved_window || f.non_text,
            "skipped-observed-window",
        ),
        (|f| !f.focus_in_sync, "skipped-focus-lag"),
        (|f| f.explicit_opposite_intent, explicit),
        (|f| f.modifier_held, "skipped-modifier-held"),
    ]
}

fn expected_reason(f: &WriteFacts, explicit: &'static str) -> Option<&'static str> {
    priority_table(explicit)
        .iter()
        .find(|(hit, _)| hit(f))
        .map(|(_, reason)| *reason)
}

fn facts_from_bits(bits: u32) -> WriteFacts {
    let bit = |i: u32| bits & (1 << i) != 0;
    WriteFacts {
        belief_open: bit(0),
        unobserved_window: bit(1),
        non_text: bit(2),
        focus_in_sync: bit(3),
        explicit_opposite_intent: bit(4),
        modifier_held: bit(5),
    }
}

#[test]
fn write_skip_reasons_match_the_priority_table_for_all_64_fact_sets() {
    let mut cases = 0;
    let mut sendable = 0;
    for bits in 0..64_u32 {
        cases += 1;
        let f = facts_from_bits(bits);
        assert_eq!(
            hiragana_write_skip_reason(f),
            expected_reason(&f, "skipped-explicit-off"),
            "hiragana: {f:?}"
        );
        assert_eq!(
            eisu_write_skip_reason(f),
            expected_reason(&f, "skipped-explicit-on"),
            "eisu: {f:?}"
        );
        if hiragana_write_skip_reason(f).is_none() {
            sendable += 1;
        }
    }
    assert_eq!(cases, 64);
    // 送ってよい組は 1 つだけ（対象の窓・NonText でない・追いついている・逆向きの意図なし・修飾なし・belief ON）。
    assert_eq!(sendable, 1);
}

/// `WriteFacts` のフィールドが増えたら、列挙（上の `facts_from_bits`）を直す必要がある。構造体の全フィールドを
/// 分解して名前で受けることで、フィールドの追加をコンパイルエラーにする。
#[test]
fn write_facts_has_exactly_the_six_enumerated_fields() {
    let WriteFacts {
        belief_open: _,
        unobserved_window: _,
        non_text: _,
        focus_in_sync: _,
        explicit_opposite_intent: _,
        modifier_held: _,
    } = facts_from_bits(0);
}
