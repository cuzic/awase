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
    DetectEvent, DetectParams, EpisodeDetector, KanaModel, KanaSide, LatinBackground, ThumbOverlap,
    Verdict,
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

/// 従来の構成（2 連 + 一様分布。`KanaModel::new`）。ADR-280 の前の既定。
fn real_model_for(layout_file: &str) -> KanaModel {
    build_model(layout_file, false)
}

/// 新しい既定の構成（3 連の補間 + ラテン文字の 2 連。実機が使うのと同じ `with_models`）。
fn extended_model() -> KanaModel {
    build_model("layout/nicola.yab", true)
}

fn build_model(layout_file: &str, extended: bool) -> KanaModel {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ngram = NgramModel::from_file(&root.join("data/ngram_hiragana.csv.gz"), 0, 0, 0)
        .expect("ngram file");
    let yab = std::fs::read_to_string(root.join(layout_file)).expect("yab");
    // 本番(bootstrap)と同じく、読み込み直後に仮名を解決する。
    let layout = YabLayout::parse(&yab, KeyboardModel::Jis)
        .expect("layout")
        .resolve_kana();
    if extended {
        KanaModel::with_models(
            &ngram,
            &layout.normal,
            KanaSide::TrigramInterp,
            Some(LatinBackground::embedded()),
        )
    } else {
        KanaModel::new(&ngram, &layout.normal)
    }
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

/// 従来の構成の閾値（`ja_threshold = 4`。ADR-280 の前の既定）。従来の構成のテストはこれで固定する。
fn with_en_threshold(en_threshold: f64) -> DetectParams {
    DetectParams {
        ja_threshold: 4.0,
        en_threshold,
        max_keys: 7,
    }
}

/// 新しい構成の閾値（`ja_threshold` は新しい既定の 14）。
fn ext_en_threshold(en_threshold: f64) -> DetectParams {
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

fn japanese_rate(model: &KanaModel, params: DetectParams, label: &str) -> f64 {
    let all = rows(label);
    assert!(!all.is_empty(), "{label} の行が無い");
    let hits = all
        .iter()
        .filter(|keys| matches!(judge(model, params, keys).0, Verdict::Japanese(_)))
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
    let params = with_en_threshold(8.0);
    let en = japanese_rate(&model, params, "en");
    let code = japanese_rate(&model, params, "code");
    let rand = japanese_rate(&model, params, "rand");
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

// ---- 新しい構成(ADR-280: 仮名側 tri-interp、背景 ラテン文字の 2 連) ----
//
// 期待値は `tools/lang-detect-eval/eval_fixture_extended.py`（同じ式の Python 再現）の実測に余裕を持たせたもの。
// **フィクスチャの ja は仮名の 2 連の分布から作られていて、3 連・ラテン文字の評価には偏る**（ja が楽観的）。実際の文での
// 誤り率は `tools/lang-detect-eval/eval_licensed_background.py`（評価データ 1400 文）で測る（日本語→英数の誤判定:
// en_threshold 8 で 6.8%、13 で 0.71%）。このテストは退行の検出が目的で、絶対的な誤り率ではない。

#[test]
fn extended_japanese_is_rarely_misjudged_as_not_japanese() {
    // Python 再現の実測(フィクスチャ seed=280, ja_threshold=14): en_threshold 8 → 1.28%、13 → 0.09%、20 → 0.03%。
    let model = extended_model();
    let ja = rows("ja");
    let misjudged = |th: f64| {
        let n = ja
            .iter()
            .filter(|k| {
                matches!(
                    judge(&model, ext_en_threshold(th), k).0,
                    Verdict::NotJapanese
                )
            })
            .count();
        rate(n, ja.len())
    };
    let (at8, at13, at20) = (misjudged(8.0), misjudged(13.0), misjudged(20.0));
    assert!(at8 <= 0.03, "en_threshold=8 の誤判定率: {at8}");
    assert!(at13 <= 0.01, "en_threshold=13 の誤判定率: {at13}");
    assert!(
        at20 <= at13,
        "閾値を上げて誤判定が増えた: 13 → {at13}, 20 → {at20}"
    );
}

#[test]
fn extended_releases_english_earlier_than_the_legacy_model() {
    // Python 再現の実測(en/code, 新): 8 → 99.6/86.8%(平均 2.8/2.9 打鍵)、13 → 98.0/79.2%(平均 3.8/3.8)。
    // 従来(en_threshold 14 = 旧 ime_on_eisu の既定): 83.8/69.2%。新しい既定の入りの閾値 13 で、従来より英文が約 14 ポイント多く流れる。
    let ext = extended_model();
    let legacy = real_model();
    for (label, th, min_rate) in [
        ("en", 8.0, 0.95),
        ("en", 13.0, 0.93),
        ("code", 8.0, 0.80),
        ("code", 13.0, 0.72),
    ] {
        let (released, _) = not_japanese_stats(&ext, ext_en_threshold(th), label);
        assert!(
            released >= min_rate,
            "{label} en_threshold={th}: 流れる率 {released} < {min_rate}"
        );
    }
    let (ext13, _) = not_japanese_stats(&ext, ext_en_threshold(13.0), "en");
    let (legacy14, _) = not_japanese_stats(&legacy, with_en_threshold(14.0), "en");
    assert!(
        ext13 >= legacy14 + 0.08,
        "新(13) {ext13} が従来(14) {legacy14} より十分多くない"
    );
    let (_, mean_en) = not_japanese_stats(&ext, ext_en_threshold(8.0), "en");
    assert!(
        mean_en <= 3.2,
        "英文が解放されるまでの平均打鍵数: {mean_en}"
    );
}

#[test]
fn extended_english_and_code_are_not_misjudged_as_japanese() {
    // Python 再現の実測(新、ja_threshold=14): en 0%、code 0%、rand 0.60%(従来と同じ)。
    let model = extended_model();
    let params = ext_en_threshold(13.0);
    let en = japanese_rate(&model, params, "en");
    let code = japanese_rate(&model, params, "code");
    let rand = japanese_rate(&model, params, "rand");
    assert!(en <= 0.005, "en: {en}");
    assert!(code <= 0.005, "code: {code}");
    assert!(rand <= 0.02, "rand: {rand}");
}

#[test]
fn extended_a_symbol_alone_is_still_neutral() {
    let model = extended_model();
    for c in NEUTRAL_KEYS.chars() {
        let mut d =
            EpisodeDetector::new(DetectParams::default(), ThumbOverlap::Enabled, [false; 2]);
        d.observe(&model, DetectEvent::CharDown(qwerty(c)));
        assert_eq!(d.verdict(), Verdict::Pending, "{c:?} の 1 打");
        assert!(d.llr().abs() < f64::EPSILON, "{c:?}: {}", d.llr());
    }
}

/// 実機で「英数と判定されない」と報告された語(windows・hello・google)を含む事例。新しい既定の入りの閾値 13 で、
/// 何打目で NotJapanese に確定するか(Python 再現: tools/lang-detect-eval/eval_fixture_extended.py の [4])。
#[test]
fn extended_reported_english_words_are_decided_at_the_default_eisu_threshold() {
    let model = extended_model();
    let params = ext_en_threshold(13.0);
    for (word, presses) in [
        ("windows", 5),
        ("window", 5),
        ("hello", 5),
        ("google", 4),
        ("function", 3),
        ("chrome", 4),
        ("build", 2),
        ("search", 5),
        ("github", 2),
        ("the", 3),
        ("excel", 3),
        ("zoom", 4),
        ("slack", 4),
    ] {
        assert_eq!(
            judge(&model, params, word),
            (Verdict::NotJapanese, presses),
            "{word}"
        );
    }
    // setup は 5 打（-12.72）で閾値 13 に届かない。`ime_on` 側の閾値 8 なら 4 打目に確定する。
    let (verdict, n) = judge(&model, params, "setup");
    assert_ne!(verdict, Verdict::NotJapanese);
    assert_eq!(n, 5);
    assert_eq!(
        judge(&model, ext_en_threshold(8.0), "setup"),
        (Verdict::NotJapanese, 4)
    );
}

/// フィクスチャの生成分布とは無関係に手で選んだ、このリポジトリの文書・コードの英単語・識別子（Python 側の
/// `INDEPENDENT_WORDS` と同じ並び）。従来の構成（en_threshold 14）は 54 語中 44 語、新しい構成（13）は 53 語を英数と確定する。
const INDEPENDENT_WORDS: &str = "keyboard remapper simultaneous keystrokes windows hook belief focus state executor \
    runtime pipeline observer output injector coordinator tracking classifier journal replay golden scenario architecture \
    guard layer boundary constants measurement evidence threshold episode detector tracker controller background latin \
    bigram trigram kana layout engine config settings session worktree develop release branch commit workflow cargo \
    clippy format target";

#[test]
fn extended_decides_almost_all_independent_words_without_false_japanese() {
    let words: Vec<&str> = INDEPENDENT_WORDS.split_whitespace().collect();
    assert_eq!(words.len(), 54);
    let ext = extended_model();
    let legacy = real_model();
    let decided = |model: &KanaModel, params: DetectParams| {
        words
            .iter()
            .filter(|w| judge(model, params, w).0 == Verdict::NotJapanese)
            .count()
    };
    let false_japanese = words
        .iter()
        .filter(|w| {
            matches!(
                judge(&ext, ext_en_threshold(13.0), w).0,
                Verdict::Japanese(_)
            )
        })
        .count();
    assert_eq!(false_japanese, 0);
    let ext13 = decided(&ext, ext_en_threshold(13.0));
    let legacy14 = decided(&legacy, with_en_threshold(14.0));
    assert!(ext13 >= 52, "新(13): {ext13}/54");
    assert!(
        ext13 >= legacy14 + 6,
        "新 {ext13} と従来 {legacy14} の差が小さい"
    );
}

#[test]
fn extended_eisu_threshold_boundary_sequence_flips_between_13_and_15() {
    // CI の実機検証 sc-langdetect-eisu13/15(e2e-ime.yml、check_langdetect.py)で打つ固定の合成列。新しい構成の背景はラテン文字の
    // 2 連で、配列の定義済みキー数 N に依らない(従来の一様分布は N が効いた)ので、nicola.yab と CI の nicola_keytop.yab で同じ値になる。
    //   `wkxd`(境界 B): llr = -1.45 → -0.55 → -10.47 → -14.05。閾値 13 で 4 打目に NotJapanese、15 では決まらない。
    //   `build`(英数 E): llr = -6.16 → -16.82。閾値 13 でも 15 でも 2 打目に NotJapanese。
    for file in ["layout/nicola.yab", "layout/nicola_keytop.yab"] {
        let model = build_model(file, true);
        assert_eq!(
            judge(&model, ext_en_threshold(13.0), "wkxd"),
            (Verdict::NotJapanese, 4),
            "wkxd @13 {file}"
        );
        let (v15, n15) = judge(&model, ext_en_threshold(15.0), "wkxd");
        assert_ne!(v15, Verdict::NotJapanese, "wkxd @15 {file}");
        assert_eq!(n15, 4, "{file}");

        let mut d = EpisodeDetector::new(ext_en_threshold(15.0), ThumbOverlap::Enabled, [false; 2]);
        for c in "wkxd".chars() {
            d.observe(&model, DetectEvent::CharDown(qwerty(c)));
            d.observe(&model, DetectEvent::CharUp(qwerty(c)));
        }
        assert!(
            (d.llr() + 14.05).abs() < 0.06,
            "wkxd の最終 llr {file}: {}",
            d.llr()
        );

        for th in [13.0, 15.0] {
            assert_eq!(
                judge(&model, ext_en_threshold(th), "build"),
                (Verdict::NotJapanese, 2),
                "build @{th} {file}"
            );
        }
    }
}
