//! ADR-280 D2: 入力言語の判定まわりの純粋な関数の全列挙（現状の挙動の固定。挙動は変えない）。
//!
//! - `HoldController::on_key`/`on_timer` が返す命令の列（`HoldCommand`）: 契機（窓切替/idle）×
//!   判定（Japanese/NotJapanese/Undecided）× 保持の可否。加えて、保持中の期限（同じ窓/別の窓）と、保持中の窓切替。
//! - `hiragana_write_skip_reason`/`eisu_write_skip_reason`（共通の `write_skip_reason`）: `WriteFacts` の 6 つの bool の
//!   2^6 = 64 通り。
//!
//! 期待値は実装の分岐を写さず、ADR-263 の doc と各関数の doc の仕様から、宣言的な表として別の形で書く。
//! 判定器（`EpisodeDetector`）は golden の外で、判定を引き起こす入力列だけを用意する。
//!
//! ## 旧 `HoldStep` から `HoldCommand` の列への対応（ADR-280 B。挙動は変えない）
//!
//! 期待値は、旧 `HoldStep` のどのフィールドが立つかと**同じ意味**の命令の列として、各ケースにリテラルで書く。
//!
//! | 旧 `HoldStep` | 新（命令の列） |
//! |---|---|
//! | `discarded = n`（n > 0） | `DiscardHeld(n)` |
//! | `japanese` | `WriteHiragana`（`Replay` の**前**） |
//! | `english` | `EnterHalfWidthAlnum`（`Replay` の**前**） |
//! | `outcomes`（古い順） | `Record(outcome)` を同じ順に（書き込みの後、`Replay` の前） |
//! | `release`（空でない） | `Replay(keys)` |
//! | `holding = false` | `DisarmDeadline`（同じ打鍵で始まって確定した場合を含む。保持しない打鍵・タイマーでも毎回） |
//! | `holding && started_holding` | `ArmDeadline` |
//! | `holding && !started_holding` | 何も出ない |
//! | `pass = Some(e)` | `Pass(e)` |
//! | `pass = None`（`on_key` のみ） | `Consume` |
//!
//! 並びは常に: DiscardHeld → Write/Enter → Record… → Replay → Arm/Disarm → Pass/Consume。

use awase::engine::fsm_types::ThumbSide;
use awase::lang_detect::{
    eisu_write_skip_reason, hiragana_write_skip_reason, DetectParams, HoldCommand, HoldController,
    JapaneseEvidence, KanaModel, KeyRole, ObservedKey, StopReason, ThumbOverlap, TrackerConfig,
    Trigger, Verdict, WriteFacts,
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

/// 比べやすい形にした命令（`Record` は (判定, 止まった理由) だけ。契機は `check` が別に見る）。
#[derive(Debug, Clone, PartialEq)]
enum Cmd {
    Discard(usize),
    WriteHiragana,
    EnterEisu,
    Record(Verdict, StopReason),
    Replay(Vec<u32>),
    Arm,
    Disarm,
    Pass(u32),
    Consume,
}

/// 実装の出力を `Cmd` に写す（期待値の側は、この写しを通さずリテラルで書く）。
fn normalize(cmds: &[HoldCommand<u32>]) -> Vec<Cmd> {
    cmds.iter()
        .map(|c| match c {
            HoldCommand::DiscardHeld(n) => Cmd::Discard(*n),
            HoldCommand::WriteHiragana => Cmd::WriteHiragana,
            HoldCommand::EnterHalfWidthAlnum => Cmd::EnterEisu,
            HoldCommand::Record(o) => Cmd::Record(o.verdict, o.stop),
            HoldCommand::Replay(keys) => Cmd::Replay(keys.clone()),
            HoldCommand::ArmDeadline => Cmd::Arm,
            HoldCommand::DisarmDeadline => Cmd::Disarm,
            HoldCommand::Pass(e) => Cmd::Pass(*e),
            HoldCommand::Consume => Cmd::Consume,
        })
        .collect()
}

fn check(got: &[HoldCommand<u32>], want: &[Cmd], trigger: Trigger, label: &str) {
    assert_eq!(normalize(got), want, "{label}");
    for c in got {
        if let HoldCommand::Record(o) = c {
            assert_eq!(o.trigger, trigger, "{label}: outcome の契機");
        }
    }
}

/// 契機を作る。窓切替: 最初の打鍵がそのまま窓切替（直前の窓が無い）。idle: 同じ窓で矢印キー（エピソードにならない）を
/// 打ったあと、idle 閾値以上あけて始める。`t0` は最初の材料の打鍵の時刻。流した打鍵は保持されず通る。
fn arm(c: &mut HoldController<u32>, m: &KanaModel, trigger: Trigger) -> u64 {
    match trigger {
        Trigger::WindowSwitch => 10,
        Trigger::Idle => {
            let s = c.on_key(m, key(KeyRole::Other, true, 0), 100, SCOPE, true);
            assert_eq!(
                normalize(&s),
                vec![Cmd::Disarm, Cmd::Pass(100)],
                "idle の準備の打鍵はエピソードにならず通る"
            );
            3010
        }
    }
}

const TRIGGERS: [Trigger; 2] = [Trigger::WindowSwitch, Trigger::Idle];

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

const OVERLAP: Verdict = Verdict::Japanese(JapaneseEvidence::ThumbOverlap);

/// 確定前の打鍵（Japanese の 1 打目）の期待値。保持するなら保持が始まる（`ArmDeadline`、`Consume`）。
/// 保持しないならそのまま通る（`DisarmDeadline`、`Pass`）。
fn expected_before_decision(may_hold: bool) -> Vec<Cmd> {
    if may_hold {
        vec![Cmd::Arm, Cmd::Consume]
    } else {
        vec![Cmd::Disarm, Cmd::Pass(1)]
    }
}

/// 確定の打鍵の期待値（仕様の表）。保持していたエピソードだけが書き込みの命令を出し、書き込み → 記録 → 解放の順。
/// 保持しないエピソードは各打鍵がそのまま通る（観測だけ）。
fn expected_final(want: Want, may_hold: bool) -> Vec<Cmd> {
    match (want, may_hold) {
        (Want::Japanese, true) => vec![
            Cmd::WriteHiragana,
            Cmd::Record(OVERLAP, StopReason::Decided),
            Cmd::Replay(vec![1, 2]),
            Cmd::Disarm,
            Cmd::Consume,
        ],
        (Want::Japanese, false) => vec![
            Cmd::Record(OVERLAP, StopReason::Decided),
            Cmd::Disarm,
            Cmd::Pass(2),
        ],
        (Want::NotJapanese, true) => vec![
            Cmd::EnterEisu,
            Cmd::Record(Verdict::NotJapanese, StopReason::Decided),
            Cmd::Replay(vec![1]),
            Cmd::Disarm,
            Cmd::Consume,
        ],
        (Want::NotJapanese, false) => vec![
            Cmd::Record(Verdict::NotJapanese, StopReason::Decided),
            Cmd::Disarm,
            Cmd::Pass(1),
        ],
        // 未決（上限）は書き込みの命令を出さない。
        (Want::Undecided, true) => vec![
            Cmd::Record(Verdict::Undecided, StopReason::MaxKeys),
            Cmd::Replay(vec![1]),
            Cmd::Disarm,
            Cmd::Consume,
        ],
        (Want::Undecided, false) => vec![
            Cmd::Record(Verdict::Undecided, StopReason::MaxKeys),
            Cmd::Disarm,
            Cmd::Pass(1),
        ],
    }
}

#[test]
fn hold_commands_for_every_trigger_verdict_and_may_hold() {
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
                let last = keys.len() - 1;
                for (i, k) in keys.iter().enumerate() {
                    let ev = u32::try_from(i).expect("i") + 1;
                    let got = c.on_key(&m, *k, ev, SCOPE, may_hold);
                    if i < last {
                        check(
                            &got,
                            &expected_before_decision(may_hold),
                            trigger,
                            &format!("{label}/step{i}"),
                        );
                    } else {
                        check(
                            &got,
                            &expected_final(want, may_hold),
                            trigger,
                            &format!("{label}/final"),
                        );
                    }
                }
                assert!(!c.is_holding(), "{label}: 確定後は保持しない");
                assert_eq!(c.held_len(), 0, "{label}: 確定後は保持列が空");
            }
        }
    }
    assert_eq!(cases, 12);
}

/// 書き込みの命令は、保持した打鍵の解放（`Replay`）より前に出る（書き込みの合成キーが先に届く）。記録はその間。
#[test]
fn write_command_comes_before_record_and_replay() {
    let m = model();
    for (want, write) in [
        (Want::Japanese, HoldCommand::WriteHiragana),
        (Want::NotJapanese, HoldCommand::EnterHalfWidthAlnum),
    ] {
        let mut c: HoldController<u32> = HoldController::new(scenario_cfg(want));
        let t0 = arm(&mut c, &m, Trigger::WindowSwitch);
        let mut last = Vec::new();
        for (i, k) in scenario_keys(want, t0).iter().enumerate() {
            last = c.on_key(&m, *k, u32::try_from(i).expect("i") + 1, SCOPE, true);
        }
        let index = |pred: fn(&HoldCommand<u32>) -> bool| last.iter().position(pred).expect("命令");
        let w = index(|c| {
            matches!(
                c,
                HoldCommand::WriteHiragana | HoldCommand::EnterHalfWidthAlnum
            )
        });
        let r = index(|c| matches!(c, HoldCommand::Record(_)));
        let p = index(|c| matches!(c, HoldCommand::Replay(_)));
        assert_eq!(last[w], write);
        assert!(w < r && r < p, "{want:?}: 書き込み < 記録 < 解放");
    }
}

/// 保持中の期限（`on_timer`）: 判定は出ず（Pending のまま閉じる）、書き込みの命令は出ない。
/// 同じ窓なら保持を解放（Timeout）、別の窓なら捨てる（WindowChanged）。保持していないなら、解放も破棄も無い。
/// どの場合も最後にタイマーを外す。
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
                check(&first, &expected_before_decision(may_hold), trigger, &label);
                let scope = if same_scope { SCOPE } else { OTHER_SCOPE };
                let got = c.on_timer(scope);
                let want = match (may_hold, same_scope) {
                    (true, true) => vec![
                        Cmd::Record(Verdict::Pending, StopReason::Timeout),
                        Cmd::Replay(vec![1]),
                        Cmd::Disarm,
                    ],
                    (true, false) => vec![
                        Cmd::Discard(1),
                        Cmd::Record(Verdict::Pending, StopReason::WindowChanged),
                        Cmd::Disarm,
                    ],
                    (false, true) => vec![
                        Cmd::Record(Verdict::Pending, StopReason::Timeout),
                        Cmd::Disarm,
                    ],
                    (false, false) => vec![
                        Cmd::Record(Verdict::Pending, StopReason::WindowChanged),
                        Cmd::Disarm,
                    ],
                };
                check(&got, &want, trigger, &label);
                assert!(
                    !c.is_holding() && c.held_len() == 0,
                    "{label}: 期限のあとは保持しない"
                );
            }
        }
    }
    assert_eq!(cases, 8);

    // エピソードが無いときのタイマーは、タイマーを外すだけ。
    let mut c: HoldController<u32> = HoldController::new(cfg(100.0, 100.0, 7));
    check(
        &c.on_timer(SCOPE),
        &[Cmd::Disarm],
        Trigger::WindowSwitch,
        "no episode",
    );
}

/// 保持中に別の窓の打鍵が来た: 保持していた打鍵は捨て（`DiscardHeld`）、止まった理由は WindowChanged。
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
            let got = c.on_key(
                &m,
                key(KeyRole::Char(pos(1, 2)), true, t0 + 10),
                2,
                OTHER_SCOPE,
                may_hold,
            );
            // 閉じた古いエピソードの契機は元のもの（Record の契機は古い側だけ）。新しいエピソードは窓切替。
            let want = if may_hold {
                vec![
                    Cmd::Discard(1),
                    Cmd::Record(Verdict::Pending, StopReason::WindowChanged),
                    Cmd::Arm,
                    Cmd::Consume,
                ]
            } else {
                vec![
                    Cmd::Record(Verdict::Pending, StopReason::WindowChanged),
                    Cmd::Disarm,
                    Cmd::Pass(2),
                ]
            };
            check(&got, &want, trigger, &label);
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
