#![allow(clippy::all, clippy::pedantic, clippy::nursery)]
//! ADR-277 段階 1: 擬似 IME の遷移表（`support/pseudo_ime.rs::TransitionTable`）を使う全列挙。
//!
//! 3 本の検査。どれも Linux で走り、実機 CI を待たずに答えが出る。
//!
//! 1. **到達性**（`reachability_golden`）: 実測のある状態から、表のキー 13 種・文字キー・awase の書き込み（開/閉）で
//!    到達しうる状態と、到達しうるのに実測の無いセル（穴）を全列挙し、golden に固定する。穴は「閉ループのシナリオが
//!    panic する所」「ランダム探索が黙って飛ばす所」「格子の測定を足すべき所」の一覧。
//! 2. **予測の全列挙**（`prediction_exhaustive_golden`）: 各キーの直前に観測して belief を真値に合わせた上で、
//!    到達しうる抽象状態（真値・`key_track`・入力モード）を幅優先で全部たどり、予測器の予測が真値と
//!    食い違う点を、そこへ至る**最短の**キー列と**機序の仕分け**（`TRIAGE`）つきで golden に固定する。抽象状態は
//!    ハーネスの実物を毎回最初から再生して作るので、写しは増えない（ハブの他の隠れ状態は抽象状態に入れていないので、
//!    履歴に依るずれはランダム探索の側に残す）。観測は打鍵から予測の柵（`KEY_EFFECT_SETTLE_MS`）の外で行う
//!    （ランダム探索の手順〈打鍵の 50ms 後に観測〉は柵の内側で、観測が捨てられて前のずれが次の打鍵へ持ち越され、
//!    1 つの原因が何種類にも数えられていた。ADR-277 の仕分けの節）。
//! 3. **防御の ablation**（`defense_ablation_finds_shortest_violation`）: 防御の目録（`support/defenses.rs`）のうち
//!    ハーネスで外せるものについて、外さなければ違反が無く、外すと違反が出る最短の事象列を出す。
//!
//! 4. **窓モデル**（ADR-277 段階 2、`window_model_exhaustive`）: 窓の切り替え（同じプロセスの中のクラスの変化・別の
//!    プロセス・学習による降格）・打鍵・IME の外部操作・観測を長さ 4 まで全部並べ、各ステップの後に (a) 核の
//!    `app_policy` が殻の live の profile に追随する、(b) drift の再送の上限が窓の種類（Read/Blind）に応じて効く、を
//!    検査する（DD3-01・#779 の回帰）。外すと破れる最短の列は 3. の golden に載る（`app-profile-follow`）。
//!    進行中の試行の持ち越し（不変条件ではなく疑い）の最短の列は `pseudo_ime_window_model.txt` に固定する。
//!
//! 再生成: `UPDATE_GOLDEN=1 cargo test -p awase-windows --test pseudo_ime_exhaustive`

mod support;

use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::Instant;

use awase::engine::{AssumedReason, InputModeState};
use awase_windows::focus::class_names::AppImeProfile;
use awase_windows::state::ime_event::HwndId;
use awase_windows::state::key_effect_predictor::{Conv, Stage};
use support::defenses::{Ablation, DEFENSES};
use support::harness::{Defenses, Harness, PredictionRecord, Setup, Source, Window};
use support::invariants::{
    p1_no_warranted_write_without_intent, p3_startup_aligns_desired_without_drift,
    prediction_mismatch, w_app_policy_follows_live_profile,
    w_carried_actuation_matches_live_window, w_drift_cap_follows_live_window,
};
use support::pseudo_ime::{
    state_name, Grid, ImeInput, QuirkSwitches, TransitionTable, TrueStage, TrueState, CONV_HIRAGANA,
};

/// 表にある 13 種のキー（`grid_key_name`。0xF4 は 0xF3 と同じ列なので除く）と文字キー 1 つ。
const KEYS: [u16; 14] = [
    0x08, 0xF0, 0x0D, 0x1B, 0xF3, 0x1C, 0xF2, 0x1A, 0x16, 0x19, 0xF1, 0x1D, 0x20, 0x41,
];

fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join(name)
}

/// golden と比べる（`UPDATE_GOLDEN=1` なら書き換える）。食い違ったら現状の全文を出す（CI のログから写せるように）。
fn check_golden(name: &str, actual: &str) {
    let path = golden_path(name);
    if std::env::var("UPDATE_GOLDEN").as_deref() == Ok("1") {
        std::fs::write(&path, actual).expect("golden を書けない");
        eprintln!("golden を更新しました: {}", path.display());
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    assert!(
        actual == expected,
        "golden {} が現状と一致しない。意図した変化なら UPDATE_GOLDEN=1 で再生成して差分を確認すること。\n\
         ---- 現状の全文 ----\n{actual}---- ここまで ----",
        path.display()
    );
}

// ---------------------------------------------------------------------------
// 1. 到達性
// ---------------------------------------------------------------------------

fn render_reachability() -> String {
    let mut inputs: Vec<ImeInput> = KEYS.iter().map(|&vk| ImeInput::Press(vk)).collect();
    inputs.push(ImeInput::Write(true));
    inputs.push(ImeInput::Write(false));
    let mut out = String::from(
        "# 擬似 IME の到達性（ADR-277）。自動生成: UPDATE_GOLDEN=1 cargo test -p awase-windows --test pseudo_ime_exhaustive\n",
    );
    for grid in Grid::ALL {
        let table = TransitionTable::load(grid);
        let r = table.reachable(table.measured_states(), &inputs, QuirkSwitches::default());
        let holes: usize = r.holes.values().map(|k| k.len()).sum();
        let _ = writeln!(
            out,
            "[{}] reachable={} holes={holes}",
            grid.name(),
            r.states.len()
        );
        for (state, keys) in &r.holes {
            let keys: Vec<&str> = keys.iter().copied().collect();
            let _ = writeln!(out, "hole {state}: {}", keys.join(" "));
        }
        let reached: std::collections::BTreeSet<String> =
            r.states.iter().map(|s| state_name(*s)).collect();
        for unreached in table.measured_state_names().difference(&reached) {
            let _ = writeln!(out, "unreached {unreached}");
        }
    }
    out
}

#[test]
fn reachability_golden() {
    check_golden("pseudo_ime_reachability.txt", &render_reachability());
}

// ---------------------------------------------------------------------------
// 2. 予測の全列挙
// ---------------------------------------------------------------------------

/// 打鍵から次の観測までの間隔（ms）。予測の柵（`KEY_EFFECT_SETTLE_MS`、ADR-191 決定3）より長くし、観測が必ず
/// belief に入るようにする。柵の内側（ランダム探索の 50ms）だと、予測の付いた打鍵の直後の観測は捨てられ
/// （`ImeModel::reconcile_key_effect_open`/`reconcile_key_effect_mode`）、ずれが次の打鍵へ持ち越される。持ち越しは本番でも
/// 起きる（柵は設計どおり）が、原因は前の打鍵のずれと同じなので、全列挙では原因ごとに 1 回だけ数える。
const OBSERVE_AFTER_KEY_MS: u64 = awase_windows::tuning::KEY_EFFECT_SETTLE_MS + 30;

/// 開始状態から `path` を再生したハーネス。各キーの前に、柵の外で観測して belief を真値に合わせる。
fn replay(grid: Grid, composing_visible: bool, start: TrueState, path: &[u16]) -> Harness {
    let mut setup = Setup::imm_cross(start).with_grid(grid);
    setup.composing_visible = composing_visible;
    let mut h = Harness::start(setup);
    h.advance_ms(100).observe(Source::ImmCross).advance_ms(200);
    for &vk in path {
        h.advance_ms(OBSERVE_AFTER_KEY_MS)
            .observe(Source::ImmCross)
            .advance_ms(50)
            .key(vk);
    }
    h
}

/// 抽象状態（重複の判定に使う）: 真値・予測器の追跡・入力モード。
fn node_key(h: &Harness) -> String {
    format!(
        "{}|{:?}|{:?}",
        state_name(h.ime.state()),
        h.model().key_track(),
        h.model().input_mode()
    )
}

// ---- 仕分け（ADR-277 の「仕分けの結果」の節） ----------------------------------

/// 食い違いの分類。各機序を (1) 実 IME の測定(格子)、(2) 予測器の現行の動作、(3) 本来あるべき動作(利用者にとって
/// 正しい挙動) の 3 層で見て決める(不一致 = 予測器のバグ、とは決めつけない)。
#[allow(dead_code)] // 現状の TRIAGE に件の無い分類（A・D・E）も、分類の定義として残す。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Category {
    /// ハーネスの観測・擬似 IME の写し方の問題（直せるものはテスト側で直す）。
    A,
    /// 予測器の既知の制限（BUG・ADR に記録がある）。
    B,
    /// 製品のバグの疑い（予測器の規則が実測とずれ、本来あるべき動作は実測の側）。根拠に「本来」を 1 行添える。
    C,
    /// 到達しても実害の無いもの。
    D,
    /// 予測器が意図して実 IME と違う予測をしている、または実 IME の動作の方が利用者にとって望ましくない（格子を既知のくせとして扱う）。
    E,
    /// 所有者の判断待ち（本来あるべき動作がどちらか決められない。根拠は質問の形）。
    P,
}

/// 閉じた打鍵の予測ずれが、閉の間（入力モードを観測しない）を経て、開く打鍵まで持ち越されたときのラベルの頭。
/// 分類は起点の機序（`←` の後）に従う。
const CARRY_PREFIX: &str = "閉の間の持ち越し←";

/// 機序のラベル → (分類, 根拠)。**ここに無いラベルが出たらテストが落ちる**（新しい種類の食い違いは仕分けてから載せる）。
/// ラベルは `mechanism` が作る。根拠は golden の冒頭に 1 行ずつ出る。
const TRIAGE: &[(&str, Category, &str)] = &[
    (
        "閉→開で入力モードを予測しない（起動時の初期値）",
        Category::B,
        "BUG-204: 閉の間は入力モードを観測せず（is_eisu_evidence が ime_on=false で None）、閉のセルは after_conv=None で mode を予測しないので、起動時の AssumedRomaji{Startup} が開くまで残る",
    ),
    (
        "閉→開で入力モードを予測しない（閉じる前・閉の間のモードの持ち越し）",
        Category::B,
        "BUG-204 と同じ機序: 閉じる打鍵の予測は合っていたが、実 IME は閉じる打鍵・閉の間の打鍵・開く打鍵で変換モードを変えることがあり（格子の after_conv）、belief のモードは開くまで観測されない",
    ),
    (
        "段階のずれ 起点=0x08（押す前=Typing・予測なし）［入力中を観測できる窓］",
        Category::C,
        "BUG-211 は修正済み(入力中の BS の後は段階を「不明」にして次のモードキーを予測しない。観測できる窓は composing で Typing に戻る)。ここに残る分は、最短の開始がすべて on-c13-none(半角カタカナ 0x13)で、予測器が 0x13 に入ったことを知らない別原因(BUG-212 の残り)が、起点の BS に帰属して数えられている。BUG-212 の追跡の印の変更で解消する想定",
    ),
    (
        "段階のずれ 起点=0x08（押す前=Typing・予測なし）［入力中を観測できない窓］",
        Category::P,
        "BUG-211 は修正済み(見えない窓では、入力中の BS の後を「不明」にして予測しない。所有者の判断 2026-10-22)。ここに残る分は、最短の開始がすべて on-c13-none(半角カタカナ 0x13)で、予測器が 0x13 に入ったことを知らない別原因(BUG-212 の残り)が、起点の BS に帰属して数えられている",
    ),
    (
        "段階のずれ 起点=0x1B（押す前=Typing・予測あり）",
        Category::B,
        "BUG-162 の残る限界: MS-IME 本体の入力中の Esc は予測なし（候補窓依存）で段階を None に捨てるが、格子は保持（入力中のまま）。入力中を観測できる窓は composing で直り、観測できない窓だけに残る（key_effect_predictor.rs の msime_native_henkan_none_and_typing_esc_are_not_predicted の注記）",
    ),
    (
        "変換モード 0x13 を予測器が表せない",
        Category::C,
        "BUG-212: Conv::from_raw(0x13)=None で入力モードから C19 とみなし、GJI の MS-IME プリセットの 0x13 の英数キーを「半角英数へ」と予測する。格子は 0x13|英数→0x19（ひらがな）。0x13 は無変換 2 回で到達する（on-c1B-none|muhenkan=ON/0x13）。本来: 予測しない（ADR-191 は表せない conv を「予測なし」とする）か実 IME どおりひらがな。半角英数と予測すると Engine が止まり NICOLA が効かない",
    ),
];

/// 打鍵前の belief の入力モードが「かな入力系」か（`invariants.rs::mode_is_native` と同じ）。
const fn mode_is_native(mode: InputModeState) -> bool {
    !matches!(mode, InputModeState::ObservedEisu)
}

fn as_stage(s: TrueStage) -> Stage {
    match s {
        TrueStage::None => Stage::None,
        TrueStage::Typing => Stage::Typing,
        TrueStage::ConvSpace => Stage::ConvSpace,
        TrueStage::ConvHenkan => Stage::ConvHenkan,
        TrueStage::ConvMuhenkan => Stage::ConvMuhenkan,
    }
}

/// 真値の段階（閉なら `None`）。
fn truth_stage(s: TrueState) -> Stage {
    if s.open {
        as_stage(s.stage)
    } else {
        Stage::None
    }
}

/// 予測器が表を引く段階（`predict_in_table` の `stage` の決め方。仕分けのための写し）。
fn predictor_stage(r: &PredictionRecord) -> Stage {
    if !r.input_open {
        return Stage::None;
    }
    match r.input_track.stage {
        Stage::None | Stage::Indeterminate if r.input_composing => Stage::Typing,
        s => s,
    }
}

/// 予測器が表を引く変換モード（`predict_in_table` の `conv` の決め方。仕分けのための写し）。
fn predictor_conv(r: &PredictionRecord) -> Conv {
    r.input_track
        .conv
        .or_else(|| r.input_conv_raw.and_then(Conv::from_raw))
        .unwrap_or(if matches!(r.input_mode, InputModeState::ObservedEisu) {
            Conv::C10
        } else {
            Conv::C19
        })
}

/// 食い違い（`records[i]`）の機序のラベル。予測器の入力（belief・段階・変換モード）のどれが真値とずれていたかを、
/// 上から順に調べる。段階のずれは、ずれが始まった打鍵（起点）まで遡ってラベルにする。
fn mechanism(records: &[PredictionRecord], i: usize, composing_visible: bool) -> String {
    let r = &records[i];
    let before = r.truth_before;
    let predicted_open = r
        .prediction
        .and_then(|p| p.effect.open)
        .unwrap_or(r.input_open);
    if !before.open && r.truth_after.open && predicted_open {
        if r.input_mode
            == (InputModeState::AssumedRomaji {
                reason: AssumedReason::Startup,
            })
        {
            return "閉→開で入力モードを予測しない（起動時の初期値）".to_owned();
        }
        // 閉じた打鍵（閉の間の直前で、押す前が開だった打鍵）の予測が外れていたら、その機序を起点とする。
        if let Some(c) = (0..i).rev().find(|&k| records[k].truth_before.open) {
            if prediction_mismatch(&records[c]).is_some() {
                return format!("{CARRY_PREFIX}{}", mechanism(records, c, composing_visible));
            }
        }
        return "閉→開で入力モードを予測しない（閉じる前・閉の間のモードの持ち越し）".to_owned();
    }
    if r.input_open != before.open
        || (before.open && mode_is_native(r.input_mode) != before.is_native())
    {
        return "打鍵前の belief が真値と違う（観測が belief に入っていない）".to_owned();
    }
    if predictor_stage(r) != truth_stage(before) {
        let mut j = i;
        while j > 0 && predictor_stage(&records[j - 1]) != truth_stage(records[j - 1].truth_before)
        {
            j -= 1;
        }
        return match j.checked_sub(1).map(|k| &records[k]) {
            Some(o) => format!(
                "段階のずれ 起点=0x{:02X}（押す前={:?}・予測{}）{}",
                o.vk,
                o.truth_before.stage,
                if o.prediction.is_some() {
                    "あり"
                } else {
                    "なし"
                },
                // 予測なしの起点は、入力中を観測できるか（`composing`）で本来の動作が変わる（BUG-211）。
                match (o.prediction.is_none(), composing_visible) {
                    (false, _) => "",
                    (true, true) => "［入力中を観測できる窓］",
                    (true, false) => "［入力中を観測できない窓］",
                }
            ),
            None => "段階のずれ 起点=開始時".to_owned(),
        };
    }
    if before.open {
        match Conv::from_raw(before.conv) {
            None => return format!("変換モード 0x{:02X} を予測器が表せない", before.conv),
            Some(c) if c != predictor_conv(r) => {
                return format!(
                    "変換モードのずれ（予測器={:?}・真値=0x{:02X}）",
                    predictor_conv(r),
                    before.conv
                )
            }
            Some(_) => {}
        }
    }
    if r.prediction.is_none() {
        "予測なし（表にセルが無い）".to_owned()
    } else {
        "表と格子が食い違う（入力は真値どおり）".to_owned()
    }
}

/// ラベルの分類（`TRIAGE` に無ければ `None`=未仕分け）。
fn category_of(label: &str) -> Option<(Category, &'static str)> {
    let root = label.strip_prefix(CARRY_PREFIX).unwrap_or(label);
    TRIAGE
        .iter()
        .find(|(l, _, _)| *l == root)
        .map(|&(_, c, why)| (c, why))
}

/// golden に書く分類の札（`TRIAGE` に無ければ「未仕分け」）。
fn tag(label: &str) -> String {
    category_of(label).map_or_else(|| "未仕分け".to_owned(), |(c, _)| format!("{c:?}"))
}

/// 食い違い 1 件。golden には `class`(格子・窓・押す前の真値・キー・機序)ごとに件数と最短の例だけを載せる。
struct Mismatch {
    class: String,
    label: String,
    path_len: usize,
    example: String,
}

/// 1 つの構成（格子 × 入力中を観測できるか）の幅優先。戻り値は (抽象状態の数, 食い違い)。
fn explore(grid: Grid, composing_visible: bool) -> (usize, Vec<Mismatch>) {
    /// 抽象状態の数の上限（爆発したら抽象化を見直す合図）。
    const MAX_NODES: usize = 5_000;
    let table = TransitionTable::load(grid);
    let starts: Vec<TrueState> = table
        .measured_states()
        .into_iter()
        .filter(|s| s.stage == TrueStage::None)
        .collect();
    let mut seen: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<(TrueState, Vec<u16>)> = VecDeque::new();
    for &s in &starts {
        let h = replay(grid, composing_visible, s, &[]);
        if seen.insert(node_key(&h)) {
            queue.push_back((s, Vec::new()));
        }
    }
    let mut lines = Vec::new();
    while let Some((start, path)) = queue.pop_front() {
        let base = replay(grid, composing_visible, start, &path);
        for vk in KEYS {
            if !base.ime.knows(vk) {
                continue; // 実測の無いセル（到達性の golden に載る）
            }
            let mut next_path = path.clone();
            next_path.push(vk);
            let h = replay(grid, composing_visible, start, &next_path);
            let last = h.predictions.len() - 1;
            let r = &h.predictions[last];
            if let Some(m) = prediction_mismatch(r) {
                let seq: Vec<String> = next_path.iter().map(|k| format!("{k:02X}")).collect();
                let label = mechanism(&h.predictions, last, composing_visible);
                lines.push(Mismatch {
                    class: format!(
                        "{} 観測可能={composing_visible} 押す前={} キー=0x{vk:02X} 機序={label}",
                        grid.name(),
                        state_name(r.truth_before),
                    ),
                    label,
                    path_len: next_path.len(),
                    example: format!(
                        "    最短 開始={} mode={:?} 列=[{}]\n    {m}",
                        state_name(start),
                        r.input_mode,
                        seq.join(" ")
                    ),
                });
            }
            if seen.insert(node_key(&h)) {
                assert!(
                    seen.len() <= MAX_NODES,
                    "抽象状態が {MAX_NODES} を超えた（{} 観測可能={composing_visible}）。抽象化を見直すこと",
                    grid.name()
                );
                queue.push_back((start, next_path));
            }
        }
    }
    (seen.len(), lines)
}

#[test]
fn prediction_exhaustive_golden() {
    let started = Instant::now();
    let mut by_config: BTreeMap<String, (usize, Vec<Mismatch>)> = BTreeMap::new();
    for grid in Grid::ALL {
        for composing_visible in [true, false] {
            by_config.insert(
                format!("{} 観測可能={composing_visible}", grid.name()),
                explore(grid, composing_visible),
            );
        }
    }
    // 仕分けの集計: ラベル → (種類, 件数)。
    let mut by_label: BTreeMap<&str, (BTreeSet<&str>, usize)> = BTreeMap::new();
    for (_, mismatches) in by_config.values() {
        for m in mismatches {
            let e = by_label.entry(m.label.as_str()).or_default();
            e.0.insert(m.class.as_str());
            e.1 += 1;
        }
    }
    let mut out = String::from(
        "# 予測器と擬似 IME の真値の食い違い（各キーの直前に柵の外で観測して belief を合わせた上で、到達しうる抽象状態を全列挙。ADR-277）。\n\
         # 自動生成: UPDATE_GOLDEN=1 cargo test -p awase-windows --test pseudo_ime_exhaustive\n\
         # 仕分け（A=ハーネス・B=既知の制限・C=製品のバグの疑い・D=実害なし・E=予測器が意図して違う/実 IME の方が望ましくない・\n\
         # P=所有者の判断待ち。ラベルと根拠は TRIAGE。「閉の間の持ち越し←X」は X の分類に従う）:\n",
    );
    let mut by_category: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for (label, (classes, n)) in &by_label {
        let e = by_category.entry(tag(label)).or_default();
        e.0 += classes.len();
        e.1 += n;
    }
    for (cat, (classes, n)) in &by_category {
        let _ = writeln!(out, "# [{cat}] 種類={classes} 件={n}");
    }
    for (label, (classes, n)) in &by_label {
        let why = category_of(label).map_or("（TRIAGE に無い）", |(_, why)| why);
        let _ = writeln!(
            out,
            "# [{}] {label}: 種類={} 件={n}\n#     根拠: {why}",
            tag(label),
            classes.len()
        );
    }
    for (config, (nodes, mismatches)) in &by_config {
        // class → (件数, 最短の例〈列の長さ, 例の文字列〉の最小, ラベル)
        let mut classes: BTreeMap<&str, (usize, (usize, &str), &str)> = BTreeMap::new();
        for m in mismatches {
            let e = classes.entry(m.class.as_str()).or_insert((
                0,
                (m.path_len, m.example.as_str()),
                m.label.as_str(),
            ));
            e.0 += 1;
            e.1 = e.1.min((m.path_len, m.example.as_str()));
        }
        let _ = writeln!(
            out,
            "[{config}] 抽象状態={nodes} 食い違い={} 種類={}",
            mismatches.len(),
            classes.len()
        );
        for (class, (n, (_, example), label)) in &classes {
            let _ = writeln!(out, "{n:3} 件 [{}] {class}\n{example}", tag(label));
        }
    }
    eprintln!("prediction_exhaustive: {:?}", started.elapsed());
    check_golden("pseudo_ime_prediction_mismatches.txt", &out);
    let unclassified: Vec<&&str> = by_label
        .keys()
        .filter(|l| category_of(l).is_none())
        .collect();
    assert!(
        unclassified.is_empty(),
        "仕分けていない機序がある（TRIAGE に分類と根拠を足すこと）: {unclassified:?}"
    );
}

// ---------------------------------------------------------------------------
// 3. 防御の ablation（最短の違反列）
// ---------------------------------------------------------------------------

/// ablation の探索に使う事象。
#[derive(Debug, Clone, Copy)]
enum Event {
    Advance(u64),
    Observe(Source),
}

const EVENTS: [Event; 3] = [
    Event::Advance(500),
    Event::Observe(Source::ImmCross),
    Event::Observe(Source::Poll),
];

/// 探索の深さ（事象の数）の上限。
const MAX_DEPTH: usize = 3;

fn run_events(start: TrueState, defenses: Defenses, events: &[Event]) -> Harness {
    let mut h = Harness::start(Setup::imm_cross(start).with_defenses(defenses));
    for e in events {
        match *e {
            Event::Advance(ms) => {
                h.advance_ms(ms);
            }
            Event::Observe(src) => {
                h.observe(src);
            }
        }
    }
    h
}

type Check = fn(&Harness) -> Result<(), String>;

/// 最短の違反列 1 件（golden の 1 行の後半）。
struct Violation {
    len: usize,
    /// `開始=… 列=[…]`。
    desc: String,
}

/// 目録の不変条件に合う探索で、最短の違反列を返す（窓モデルの不変条件 `w_*` は窓の事象で、それ以外は段階 1 の事象で）。
fn shortest_violation_for(invariant: &str, defenses: Defenses) -> Option<Violation> {
    let classic = |check: Check| {
        shortest_violation(defenses, check).map(|(start, seq)| Violation {
            len: seq.len(),
            desc: format!("開始={} 列={seq:?}", state_name(start)),
        })
    };
    let window = |check: Check| {
        shortest_window_violation(defenses, check)
            .violation
            .map(|(start, seq)| Violation {
                len: seq.len(),
                desc: format!("開始={}@{start} 列={seq:?}", state_name(W_INITIAL)),
            })
    };
    match invariant {
        "p1_no_warranted_write_without_intent" => classic(p1_no_warranted_write_without_intent),
        "p3_startup_aligns_desired_without_drift" => {
            classic(p3_startup_aligns_desired_without_drift)
        }
        "w_drift_cap_follows_live_window" => window(w_drift_cap_follows_live_window),
        other => panic!("目録の不変条件 {other} がこのテストに無い"),
    }
}

fn switch_off(name: &str) -> Defenses {
    let mut d = Defenses::ALL_ON;
    match name {
        "placeholder_align" => d.placeholder_align = false,
        "app_profile_follow" => d.app_profile_follow = false,
        other => panic!("目録のスイッチ {other} が Defenses に無い"),
    }
    d
}

/// 短い順（深さ 0 から `MAX_DEPTH` まで、各深さは辞書順）に事象列を全列挙し、最初の違反を返す。
fn shortest_violation(
    defenses: Defenses,
    check: fn(&Harness) -> Result<(), String>,
) -> Option<(TrueState, Vec<Event>)> {
    let starts = [
        TrueState {
            open: false,
            conv: CONV_HIRAGANA,
            stage: TrueStage::None,
        },
        TrueState {
            open: true,
            conv: CONV_HIRAGANA,
            stage: TrueStage::None,
        },
    ];
    let mut frontier: Vec<Vec<Event>> = vec![Vec::new()];
    for _depth in 0..=MAX_DEPTH {
        for seq in &frontier {
            for &start in &starts {
                if check(&run_events(start, defenses, seq)).is_err() {
                    return Some((start, seq.clone()));
                }
            }
        }
        frontier = frontier
            .iter()
            .flat_map(|seq| {
                EVENTS.iter().map(move |&e| {
                    let mut s = seq.clone();
                    s.push(e);
                    s
                })
            })
            .collect();
    }
    None
}

#[test]
fn defense_ablation_finds_shortest_violation() {
    let mut out = String::from(
        "# 防御を外すと破れる最短の事象列（ADR-277、support/defenses.rs の HarnessSwitch）。\n\
         # 自動生成: UPDATE_GOLDEN=1 cargo test -p awase-windows --test pseudo_ime_exhaustive\n",
    );
    for d in DEFENSES {
        let Ablation::HarnessSwitch(switch) = d.ablation else {
            continue;
        };
        if let Some(v) = shortest_violation_for(d.invariant, Defenses::ALL_ON) {
            panic!(
                "{}: 防御が有効なのに {} が破れた（{}）",
                d.id, d.invariant, v.desc
            );
        }
        let Some(v) = shortest_violation_for(d.invariant, switch_off(switch)) else {
            panic!(
                "{}: 外しても探索の深さまでに {} が破れない。防御が要らないか、事象の集合が足りない",
                d.id, d.invariant
            );
        };
        let _ = writeln!(
            out,
            "{}: 外すと {} 手で {} が破れる {}",
            d.id, v.len, d.invariant, v.desc
        );
    }
    check_golden("defense_ablation.txt", &out);
}

// ---------------------------------------------------------------------------
// 4. 窓モデル（ADR-277 段階 2）
// ---------------------------------------------------------------------------

/// 窓の表。Explorer 本体と XAML ホストは同じプロセスでクラスが違う（DD3-01 の場面）。メモ帳は学習で降格しうる
/// `Standard`、wezterm は `TsfNative`、relay は `input_relay_apps` に載った `InputRelay`。
const WINDOWS: [Window; 5] = [
    Window {
        name: "explorer",
        hwnd: HwndId(0x1001),
        pid: 10,
        thread: 1,
        class: "CabinetWClass",
        process: "explorer.exe",
        input_relay: false,
        learned_unavailable: false,
    },
    Window {
        name: "xaml",
        hwnd: HwndId(0x1002),
        pid: 10,
        thread: 2,
        class: "XamlExplorerHostIslandWindow",
        process: "explorer.exe",
        input_relay: false,
        learned_unavailable: false,
    },
    Window {
        name: "wezterm",
        hwnd: HwndId(0x2001),
        pid: 20,
        thread: 3,
        class: "org.wezfurlong.wezterm",
        process: "wezterm-gui.exe",
        input_relay: false,
        learned_unavailable: false,
    },
    Window {
        name: "notepad",
        hwnd: HwndId(0x3001),
        pid: 30,
        thread: 4,
        class: "Notepad",
        process: "notepad.exe",
        input_relay: false,
        learned_unavailable: false,
    },
    Window {
        name: "relay",
        hwnd: HwndId(0x4001),
        pid: 40,
        thread: 5,
        class: "RelayMainWindow",
        process: "relay.exe",
        input_relay: true,
        learned_unavailable: false,
    },
];

fn window_named(name: &str) -> Window {
    *WINDOWS
        .iter()
        .find(|w| w.name == name)
        .unwrap_or_else(|| panic!("窓 {name} が表に無い"))
}

/// 窓モデルの事象。
#[derive(Debug, Clone, Copy)]
enum WEvent {
    /// その窓へフォーカスを移す（同じプロセスなら `FocusHwndUpdated`+追随、違えば `FocusChanged`）。
    FocusTo(&'static str),
    /// いまの窓が学習で降格される。
    Demote,
    /// 文字キー 1 回。
    Key,
    /// 外から IME の開閉を反転する（awase は見ていない）。
    ExternalToggle,
    /// 読める窓で IME の実状態を観測する（ImmCross）。
    Observe,
}

const W_EVENTS: [WEvent; 9] = [
    WEvent::FocusTo("explorer"),
    WEvent::FocusTo("xaml"),
    WEvent::FocusTo("wezterm"),
    WEvent::FocusTo("notepad"),
    WEvent::FocusTo("relay"),
    WEvent::Demote,
    WEvent::Key,
    WEvent::ExternalToggle,
    WEvent::Observe,
];

/// 窓モデルの探索の深さ（事象の数）の上限。
const W_MAX_DEPTH: usize = 4;

/// 起動する窓（読める窓と読めない窓）。
const W_STARTS: [&str; 2] = ["explorer", "xaml"];

/// 起動時の IME の真値（閉×ひらがな）。
const W_INITIAL: TrueState = TrueState {
    open: false,
    conv: CONV_HIRAGANA,
    stage: TrueStage::None,
};

/// 事象 1 つを流す。意味の無い事象（いまの窓へのフォーカス・読めない窓の観測・降格しない窓の降格）は流さず `false`。
fn apply_wevent(h: &mut Harness, e: WEvent) -> bool {
    let w = h.window().expect("窓モデル");
    match e {
        WEvent::FocusTo(name) => {
            let to = window_named(name);
            if to.hwnd == w.hwnd {
                return false;
            }
            h.focus_to(to);
        }
        WEvent::Demote => {
            if w.learned_unavailable || w.live_profile() != AppImeProfile::Standard {
                return false;
            }
            h.demote_by_learning();
        }
        WEvent::Key => {
            h.key(0x41);
        }
        WEvent::ExternalToggle => {
            let open = h.ime.state().open;
            h.external_set_open(!open);
        }
        WEvent::Observe => {
            if !w.readable() {
                return false;
            }
            h.observe(Source::ImmCross);
        }
    }
    true
}

/// 事象列を最初から流す。意味の無い事象を含む列は `None`。
fn run_window_events(start: &str, defenses: Defenses, events: &[WEvent]) -> Option<Harness> {
    let mut h =
        Harness::start(Setup::in_window(W_INITIAL, window_named(start)).with_defenses(defenses));
    for &e in events {
        if !apply_wevent(&mut h, e) {
            return None;
        }
    }
    Some(h)
}

/// 窓モデルの探索の結果。
struct WSearch {
    /// 最初に見つかった違反（開始の窓・事象列）。
    violation: Option<(&'static str, Vec<WEvent>)>,
    /// 検査した（意味のある）事象列の数。
    checked: usize,
}

/// 短い順（深さ 0 から `W_MAX_DEPTH` まで、各深さは開始の窓 → 事象の表の順）に、意味のある事象列を全部流して `check`
/// する。不変条件は全ステップを見るので、列の途中の状態も検査される。意味の無い列は伸ばさない。
fn shortest_window_violation(defenses: Defenses, check: Check) -> WSearch {
    let mut frontiers: Vec<Vec<Vec<WEvent>>> = W_STARTS.iter().map(|_| vec![Vec::new()]).collect();
    let mut checked = 0;
    for depth in 0..=W_MAX_DEPTH {
        for (&start, frontier) in W_STARTS.iter().zip(frontiers.iter_mut()) {
            let mut alive = Vec::new();
            for seq in frontier.drain(..) {
                let Some(h) = run_window_events(start, defenses, &seq) else {
                    continue;
                };
                checked += 1;
                if check(&h).is_err() {
                    return WSearch {
                        violation: Some((start, seq)),
                        checked,
                    };
                }
                alive.push(seq);
            }
            if depth < W_MAX_DEPTH {
                *frontier = alive
                    .iter()
                    .flat_map(|seq| {
                        W_EVENTS.iter().map(move |&e| {
                            let mut s = seq.clone();
                            s.push(e);
                            s
                        })
                    })
                    .collect();
            }
        }
    }
    WSearch {
        violation: None,
        checked,
    }
}

fn window_invariants(h: &Harness) -> Result<(), String> {
    w_app_policy_follows_live_profile(h)?;
    w_drift_cap_follows_live_window(h)
}

#[test]
fn window_model_exhaustive() {
    let started = Instant::now();
    let all = shortest_window_violation(Defenses::ALL_ON, window_invariants);
    if let Some((start, seq)) = &all.violation {
        let h = run_window_events(start, Defenses::ALL_ON, seq).expect("再生できる");
        panic!(
            "窓モデルの不変条件が破れた（開始={start} 列={seq:?}）: {}\n経過:\n{}",
            window_invariants(&h).unwrap_err(),
            h.trace()
        );
    }
    eprintln!(
        "窓モデル: {} 本の事象列（深さ {W_MAX_DEPTH} まで）で (a)(b) の違反なし、{:?}",
        all.checked,
        started.elapsed()
    );
    assert!(all.checked > 1000, "列が少なすぎる: {}", all.checked);

    // 疑いの記録: 同じプロセスの中で窓の種類が変わる前に据えた試行の方針の持ち越し（DD3-01 の残り）。
    let mut out = String::from(
        "# 窓モデルの疑い（ADR-277 段階 2）。不変条件ではなく、現状の最短の列を固定する。\n\
         # 自動生成: UPDATE_GOLDEN=1 cargo test -p awase-windows --test pseudo_ime_exhaustive\n",
    );
    let carried =
        shortest_window_violation(Defenses::ALL_ON, w_carried_actuation_matches_live_window);
    match &carried.violation {
        Some((start, seq)) => {
            let h = run_window_events(start, Defenses::ALL_ON, seq).expect("再生できる");
            let _ = writeln!(
                out,
                "carried-actuation-policy: {} 手 開始={}@{start} 列={seq:?}\n  {}",
                seq.len(),
                state_name(W_INITIAL),
                w_carried_actuation_matches_live_window(&h).unwrap_err()
            );
        }
        None => {
            let _ = writeln!(out, "carried-actuation-policy: 深さ {W_MAX_DEPTH} まで無し");
        }
    }
    check_golden("pseudo_ime_window_model.txt", &out);
}
