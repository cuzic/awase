//! 入力言語の判定器(`awase::lang_detect`)の誤り率を、生成済みフィクスチャで測って固定する(ADR-280 D1)。
//!
//! フィクスチャ `tests/fixtures/lang_detect_eval.tsv` は `tools/lang-detect-eval-fixture/gen_fixture.py` が固定 seed で
//! 生成する(手順は同ディレクトリの README)。1 行 = 1 エピソードの親指なしの打鍵列で、`ラベル<TAB>QWERTY の文字列`。
//!
//! - `ja`: `data/ngram_hiragana.csv.gz` の分布から作った仮名列を、文中のあちこちから打ち始めたときの、
//!   親指なしで続く未シフト面の区間(空の行は、最初の打鍵から親指が要る = 判定器に着く前に日本語と決まる開始位置)。
//!   区間が尽きた = 親指が出た = 日本語確定とみなすので、`NotJapanese` に達した行だけが誤判定。
//! - `en` / `code` / `rand`: 英単語・コード断片・一様なキー列(合成)。
//!
//! 合格値は、同じ式を Python で再現した実測値(下の各アサートに併記)に余裕を持たせたもの。**ja は判定器自身の
//! 分布から作った列なので、実際の文(#819 の評価: `en_threshold` 8 で 7.0%、14 で 1.0%)より誤りが少なく楽観的**。
//! この値は絶対的な誤り率ではなく、判定器・仮名表・配列を変えたときの退行の検出に使う。

use std::path::Path;

use awase::lang_detect::{
    DetectEvent, DetectParams, EpisodeDetector, KanaModel, ThumbOverlap, Verdict,
};
use awase::ngram::NgramModel;
use awase::scanmap::{KeyboardModel, PhysicalPos};
use awase::yab::YabLayout;

const FIXTURE: &str = include_str!("fixtures/lang_detect_eval.tsv");

/// QWERTY の物理位置(yab の行・列。JIS)。gen_fixture.py の `GRID` と同じ。
const GRID: [&str; 4] = [
    "1234567890-^\\",
    "qwertyuiop@[",
    "asdfghjkl;:]",
    "zxcvbnm,./",
];

/// 未シフト面で仮名に読めないキー(数字行・`。` `、` `[` `:`〈後〉 `]`〈逃〉 `．` `／`)
const NEUTRAL_KEYS: &str = "1234567890-^\\qp@[:]z/";

fn real_model() -> KanaModel {
    real_model_for("layout/nicola.yab")
}

fn real_model_for(layout_file: &str) -> KanaModel {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ngram = NgramModel::from_file(&root.join("data/ngram_hiragana.csv.gz"), 0, 0, 0)
        .expect("ngram file");
    let yab = std::fs::read_to_string(root.join(layout_file)).expect("yab");
    // 本番(bootstrap)と同じく、読み込み直後に仮名を解決する。
    let layout = YabLayout::parse(&yab, KeyboardModel::Jis)
        .expect("layout")
        .resolve_kana();
    KanaModel::new(&ngram, &layout.normal)
}

fn qwerty(c: char) -> PhysicalPos {
    for (r, row) in GRID.iter().enumerate() {
        if let Some(col) = row.find(c) {
            return PhysicalPos::new(
                u8::try_from(r).expect("row"),
                u8::try_from(col).expect("col"),
            );
        }
    }
    panic!("GRID に無い文字: {c:?}");
}

/// 親指なしの打鍵列を判定器にかける。決まらないまま列が尽きたら `Pending` のまま返る。
fn judge(model: &KanaModel, params: DetectParams, keys: &str) -> (Verdict, usize) {
    let mut d = EpisodeDetector::new(params, ThumbOverlap::Enabled, [false; 2]);
    for c in keys.chars() {
        d.observe(model, DetectEvent::CharDown(qwerty(c)));
        d.observe(model, DetectEvent::CharUp(qwerty(c)));
        if d.verdict().is_decided() {
            break;
        }
    }
    (d.verdict(), d.presses())
}

fn with_en_threshold(en_threshold: f64) -> DetectParams {
    DetectParams {
        en_threshold,
        ..DetectParams::default()
    }
}

fn rows(label: &str) -> Vec<&'static str> {
    FIXTURE
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .filter(|(l, _)| *l == label)
        .map(|(_, keys)| keys)
        .collect()
}

fn rate(count: usize, total: usize) -> f64 {
    f64::from(u32::try_from(count).expect("count"))
        / f64::from(u32::try_from(total).expect("total"))
}

/// `NotJapanese` に達した行の割合と、達した行の平均打鍵数
fn not_japanese_stats(model: &KanaModel, params: DetectParams, label: &str) -> (f64, f64) {
    let all = rows(label);
    assert!(!all.is_empty(), "{label} の行が無い");
    let presses: Vec<usize> = all
        .iter()
        .filter_map(|keys| match judge(model, params, keys) {
            (Verdict::NotJapanese, n) => Some(n),
            _ => None,
        })
        .collect();
    let mean = if presses.is_empty() {
        0.0
    } else {
        f64::from(u32::try_from(presses.iter().sum::<usize>()).expect("sum"))
            / f64::from(u32::try_from(presses.len()).expect("len"))
    };
    (rate(presses.len(), all.len()), mean)
}

fn japanese_rate(model: &KanaModel, label: &str) -> f64 {
    let all = rows(label);
    assert!(!all.is_empty(), "{label} の行が無い");
    let hits = all
        .iter()
        .filter(|keys| {
            matches!(
                judge(model, DetectParams::default(), keys).0,
                Verdict::Japanese(_)
            )
        })
        .count();
    rate(hits, all.len())
}

#[test]
fn fixture_has_every_class_with_expected_sizes() {
    // 生成の取り違え(ラベルの typo・空ファイル)を先に検出する。件数は gen_fixture.py の N_* と合わせる。
    assert_eq!(rows("ja").len(), 3200);
    assert_eq!(rows("en").len(), 1000);
    assert_eq!(rows("code").len(), 500);
    assert_eq!(rows("rand").len(), 500);
    // ja は空の行(最初から親指が要る開始位置)と、記号で始まる行の両方を含む。
    let ja = rows("ja");
    assert!(ja.iter().any(|k| k.is_empty()));
    assert!(ja.iter().any(|k| k.starts_with(['@', 'q'])));
}

#[test]
fn japanese_is_rarely_misjudged_as_not_japanese() {
    // 日本語(親指なしの区間)を NotJapanese と誤判定する率。`en_threshold` ごとに別の指標。
    // Python 再現の実測(フィクスチャ seed=280): 8 → 0.69%、14 → 0.03%、20 → 0%。
    // en_threshold = 8 は `ime_on` 側の既定、14 は `ime_on_eisu` の入りの実効の閾値(`max(eisu_threshold, en_threshold)`)。
    let model = real_model();
    let ja = rows("ja");
    let misjudged = |th: f64| {
        let n = ja
            .iter()
            .filter(|k| {
                matches!(
                    judge(&model, with_en_threshold(th), k).0,
                    Verdict::NotJapanese
                )
            })
            .count();
        rate(n, ja.len())
    };
    let at8 = misjudged(8.0);
    let at14 = misjudged(14.0);
    assert!(at8 <= 0.03, "en_threshold=8 の誤判定率: {at8}");
    assert!(at14 <= 0.01, "en_threshold=14 の誤判定率: {at14}");
    assert!(
        at14 <= at8,
        "閾値を上げて誤判定が増えた: 8 → {at8}, 14 → {at14}"
    );
}

#[test]
fn english_side_is_released_early() {
    // 英数側が max_keys(7)以内に NotJapanese に達する率(早く流れる率)と、達するまでの平均打鍵数。
    // Python 再現の実測 en/code/rand: 8 → 97.5/87.0/64.8%(平均 3.3/3.3/4.0 打鍵)、14 → 85.3/69.4/34.4%(平均 4.7/4.6/4.9)。
    // #819 の評価(英文 8 → 96.8%、14 → 85.3%)と近い。余裕は約 5〜13 ポイント。
    let model = real_model();
    for (label, th, min_rate) in [
        ("en", 8.0, 0.90),
        ("en", 14.0, 0.78),
        ("code", 8.0, 0.80),
        ("code", 14.0, 0.62),
        ("rand", 8.0, 0.52),
        ("rand", 14.0, 0.28),
    ] {
        let (released, _) = not_japanese_stats(&model, with_en_threshold(th), label);
        assert!(
            released >= min_rate,
            "{label} en_threshold={th}: 流れる率 {released} < {min_rate}"
        );
    }
    let (_, mean_en) = not_japanese_stats(&model, with_en_threshold(8.0), "en");
    assert!(
        mean_en <= 4.0,
        "英文が解放されるまでの平均打鍵数: {mean_en}"
    );
}

#[test]
fn english_and_code_are_not_misjudged_as_japanese() {
    // 英文・コード・一様なキー列を Japanese(尤度)と誤判定する率。#819 で記号が中立になった後の再測定。
    // Python 再現の実測: en 0%、code 0%、rand 0.6%。
    let model = real_model();
    let en = japanese_rate(&model, "en");
    let code = japanese_rate(&model, "code");
    let rand = japanese_rate(&model, "rand");
    assert!(en <= 0.005, "en: {en}");
    assert!(code <= 0.005, "code: {code}");
    assert!(rand <= 0.02, "rand: {rand}");
}

#[test]
fn a_symbol_alone_is_neutral_and_never_decides_not_japanese() {
    // 仮名に読めないキー(記号・数字・未定義)は尤度比に 0 を足す。1 打で NotJapanese になる事故(#819 の `¥` 1 打)の再発防止。
    let model = real_model();
    for c in NEUTRAL_KEYS.chars() {
        let mut d =
            EpisodeDetector::new(DetectParams::default(), ThumbOverlap::Enabled, [false; 2]);
        d.observe(&model, DetectEvent::CharDown(qwerty(c)));
        assert_eq!(d.verdict(), Verdict::Pending, "{c:?} の 1 打");
        assert!(
            d.llr().abs() < f64::EPSILON,
            "{c:?} の増分が 0 でない: {}",
            d.llr()
        );
    }
    // 記号だけが max_keys まで続いても、決まるのは Undecided で NotJapanese にはならない。
    let run: String = NEUTRAL_KEYS.chars().cycle().take(7).collect();
    let (verdict, presses) = judge(&model, DetectParams::default(), &run);
    assert_eq!(verdict, Verdict::Undecided, "{run:?}");
    assert_eq!(presses, 7);
}

#[test]
fn eisu_threshold_boundary_sequence_flips_between_14_and_16() {
    // ADR-280 C0 の配線確認(CI の実機検証 sc-langdetect-*)で打つ固定の合成列。Python 再現の推移
    // (配列ごとに定義済みキー数 N が違い、1 打ごとに `+ln N` が乗る。nicola.yab は N=44、CI の既定 nicola_keytop.yab は N=47):
    //   `ayre`(境界 B): N=44 で llr = 1.5741 → -4.6360 → -8.5266 → -14.9978、N=47 で -14.7340。閾値 14 で 4 打目に NotJapanese、16 では決まらない。
    //   `build`(英数 E): N=44 で llr = -6.4998 → -15.6293 → -19.4536。閾値 14 で 2 打目、16 でも 3 打目に NotJapanese。
    // Rust の判定が Python の再現と一致することの確認を兼ねる(`docs/tasks/adr280-c0-wiring-ci-result-2026-10-24.md`)。
    for (file, ayre_llr) in [
        ("layout/nicola.yab", -14.9978),
        ("layout/nicola_keytop.yab", -14.7340),
    ] {
        let model = real_model_for(file);
        let (v14, n14) = judge(&model, with_en_threshold(14.0), "ayre");
        assert_eq!((v14, n14), (Verdict::NotJapanese, 4), "ayre @14 {file}");
        let (v16, n16) = judge(&model, with_en_threshold(16.0), "ayre");
        assert_ne!(v16, Verdict::NotJapanese, "ayre @16 {file}");
        assert_eq!(n16, 4, "{file}");

        let mut d =
            EpisodeDetector::new(with_en_threshold(16.0), ThumbOverlap::Enabled, [false; 2]);
        for c in "ayre".chars() {
            d.observe(&model, DetectEvent::CharDown(qwerty(c)));
            d.observe(&model, DetectEvent::CharUp(qwerty(c)));
        }
        assert!(
            (d.llr() - ayre_llr).abs() < 0.01,
            "ayre の最終 llr {file}: {}",
            d.llr()
        );

        let (e14, en14) = judge(&model, with_en_threshold(14.0), "build");
        assert_eq!((e14, en14), (Verdict::NotJapanese, 2), "build @14 {file}");
        let (e16, en16) = judge(&model, with_en_threshold(16.0), "build");
        assert_eq!((e16, en16), (Verdict::NotJapanese, 3), "build @16 {file}");
    }
}
