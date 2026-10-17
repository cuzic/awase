//! エピソード先頭の打鍵列から、日本語（NICOLA）入力かどうかを判定する純粋ロジック（ADR-263 PR 1）。
//!
//! 窓切替直後・長い無操作のあとの最初の数打鍵を、Engine の手前で保持して判定する。判定は次の 2 つだけで、
//! 打鍵の時刻は使わない（Down/Up の**順序**だけを見る）。
//!
//! 1. **親指と文字キーの重なり**: 親指の Down のときに文字キーが押されたまま、または文字キーの Down のときに
//!    親指が押されたまま。親指の単独タップ（重なりなし）は証拠にしない。無変換/変換・F13〜F24・Alt なりすましは
//!    英文の打鍵に現れないので重なりを証拠にできるが、Space を親指キーにした構成は英文のロールオーバーでも
//!    重なりが成立するので、呼び出し側（殻）がエピソードごと対象外にすること（ADR-263 D3）。
//! 2. **仮名 n-gram の対数尤度比**: 親指が出ていない間は、未シフト面でデコードした仮名列が日本語として自然かを、
//!    言語非依存（一様）の背景モデルと比べる。`LLR = Σ [ln P_ja(仮名_i | 仮名_i-1) − ln(1/N_keys)]`。
//!    英語専用のモデルは持たない（コーディング・他言語との併用のため）。
//!
//! 結果は次の 4 つ。**「日本語でない」と「決められない」は殻では同じ扱い**（保持した打鍵を今までと同じ流れに戻す）
//! なので、`en_threshold` は小さめ（英数側を早く流す）、`ja_threshold` は大きめ（日本語を慎重に）の非対称にする。
//!
//! このモジュールは OS・時刻・生の VK を持たない。入力は分類済みの `PhysicalPos` だけ（ADR-019）。

use rustc_hash::{FxHashMap, FxHashSet};

use crate::ngram::NgramModel;
use crate::scanmap::PhysicalPos;
use crate::yab::{YabFace, YabValue};

/// .yab の面の行数・列数の上限（`yab::MAX_ROWS`/`MAX_COLS` と同じ）
const GRID_ROWS: u8 = 4;
const GRID_COLS: u8 = 13;

/// 親指キーの左右
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThumbSide {
    Left,
    Right,
}

impl ThumbSide {
    const fn index(self) -> usize {
        match self {
            Self::Left => 0,
            Self::Right => 1,
        }
    }
}

/// 判定器への入力（分類済みの打鍵イベント）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectEvent {
    /// 文字キーの Down（自動リピートの再送は無視される）
    CharDown(PhysicalPos),
    CharUp(PhysicalPos),
    ThumbDown(ThumbSide),
    ThumbUp(ThumbSide),
}

/// 日本語と判定した根拠
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JapaneseEvidence {
    /// 親指と文字キーの重なり
    ThumbOverlap,
    /// 仮名 n-gram の対数尤度比が `ja_threshold` に達した
    Likelihood,
}

/// 判定結果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// まだ決まらない（保持を続ける）
    Pending,
    /// 日本語（NICOLA）入力
    Japanese(JapaneseEvidence),
    /// 対数尤度比が `-en_threshold` 以下（日本語として不自然）
    NotJapanese,
    /// `max_keys` 打鍵まで見ても決まらなかった
    Undecided,
}

impl Verdict {
    /// 確定したか（`Pending` 以外）
    #[must_use]
    pub const fn is_decided(self) -> bool {
        !matches!(self, Self::Pending)
    }
}

/// 判定のパラメータ
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DetectParams {
    /// この値以上で日本語と確定する（自然対数の尤度比）。大きめにして ON を慎重にする
    pub ja_threshold: f64,
    /// `-` この値以下で「日本語でない」とする。小さめにして英数側を早く流す
    pub en_threshold: f64,
    /// 判定に使う打鍵（Down）の最大数。超えたら `Undecided`
    pub max_keys: usize,
}

impl Default for DetectParams {
    /// 人工データのオフライン評価（2026-10-17、`ja_threshold = 4` で他言語の誤確定 0.2%、`en_threshold = 8` で
    /// 日本語の確定 91.9%・他言語の保持は平均 2.9 打鍵）。実ジャーナルで再校正する（ADR-263 PR 2a）。
    fn default() -> Self {
        Self {
            ja_threshold: 4.0,
            en_threshold: 8.0,
            max_keys: 7,
        }
    }
}

/// 未シフト面の位置→仮名と、仮名 2 連の条件付き確率（`data/ngram_hiragana` の生の表から作る）
#[derive(Debug)]
pub struct KanaModel {
    /// 未シフト面（`YabFace`）で仮名に読めるキー
    key_kana: FxHashMap<PhysicalPos, char>,
    /// 背景の一様分布の台（未シフト面で定義されているキーの数）
    key_count: f64,
    /// 2 連の相対頻度（`10^score`）
    freq: FxHashMap<(char, char), f64>,
    /// 左の文字ごとの頻度の和
    left: FxHashMap<char, f64>,
    /// 左の文字ごとの、表にある 2 連の数
    listed: FxHashMap<char, usize>,
    /// 表に現れる文字の集合
    vocab: FxHashSet<char>,
    /// 表に無い組の相対頻度（表の最小値の 1/10）
    floor: f64,
    /// 左の文字の頻度の総和
    total: f64,
}

impl KanaModel {
    /// 仮名 n-gram の生の表と、未シフト面（`normal`）から作る。
    ///
    /// 表が空でも作れる（その場合、仮名の尤度はすべて下限になり、日本語とは決まらない）。
    #[must_use]
    pub fn new(ngram: &NgramModel, normal: &YabFace) -> Self {
        let mut key_kana = FxHashMap::default();
        let mut defined = 0_u32;
        for row in 0..GRID_ROWS {
            for col in 0..GRID_COLS {
                let pos = PhysicalPos::new(row, col);
                match normal.get(&pos) {
                    None | Some(YabValue::None) => {}
                    Some(YabValue::Romaji {
                        kana: Some(kana), ..
                    }) => {
                        defined += 1;
                        key_kana.insert(pos, *kana);
                    }
                    Some(_) => defined += 1,
                }
            }
        }

        let mut freq = FxHashMap::default();
        let mut left: FxHashMap<char, f64> = FxHashMap::default();
        let mut listed: FxHashMap<char, usize> = FxHashMap::default();
        let mut vocab = FxHashSet::default();
        let mut min = f64::INFINITY;
        for ((a, b), score) in ngram.bigram_scores() {
            let f = 10_f64.powf(f64::from(score));
            freq.insert((a, b), f);
            *left.entry(a).or_insert(0.0) += f;
            *listed.entry(a).or_insert(0) += 1;
            vocab.insert(a);
            vocab.insert(b);
            min = min.min(f);
        }
        let total: f64 = left.values().sum();
        let floor = if min.is_finite() { min * 0.1 } else { 1e-12 };

        Self {
            key_kana,
            key_count: f64::from(defined.max(1)),
            freq,
            left,
            listed,
            vocab,
            floor,
            total,
        }
    }

    /// 1 打鍵ぶんの対数尤度比の増分 `ln P_ja(token | prev) − ln(1/N_keys)`。
    ///
    /// `prev` は直前の打鍵の仮名（直前が仮名に読めなかった・先頭なら `None`）。
    #[allow(clippy::cast_precision_loss)] // 語彙の数（高々数百）なので精度は問題にならない
    fn delta(&self, prev: Option<char>, pos: PhysicalPos) -> f64 {
        let background = -self.key_count.ln();
        let low = (self.floor / self.total.max(self.floor)).ln();
        let Some(&token) = self.key_kana.get(&pos) else {
            return low - background;
        };
        if !self.vocab.contains(&token) {
            return low - background;
        }
        let vocab_len = self.vocab.len() as f64;
        let ja = match prev {
            Some(a) if self.vocab.contains(&a) => {
                let listed = self.listed.get(&a).copied().unwrap_or(0) as f64;
                let denominator =
                    self.left.get(&a).copied().unwrap_or(0.0) + (vocab_len - listed) * self.floor;
                let numerator = self.freq.get(&(a, token)).copied().unwrap_or(self.floor);
                (numerator / denominator).ln()
            }
            _ => {
                let left = self.left.get(&token).copied().unwrap_or(0.0);
                ((left + self.floor) / (self.total + vocab_len * self.floor)).ln()
            }
        };
        ja - background
    }
}

/// 1 エピソードぶんの判定器。`observe` に打鍵イベントを順に渡す。
///
/// 決まったあと（`Pending` 以外）は結果が変わらず、以降のイベントは無視される。
#[derive(Debug)]
pub struct EpisodeDetector<'m> {
    model: &'m KanaModel,
    params: DetectParams,
    held_chars: FxHashSet<PhysicalPos>,
    held_thumbs: [bool; 2],
    presses: usize,
    prev: Option<char>,
    llr: f64,
    verdict: Verdict,
}

impl<'m> EpisodeDetector<'m> {
    #[must_use]
    pub fn new(model: &'m KanaModel, params: DetectParams) -> Self {
        Self {
            model,
            params,
            held_chars: FxHashSet::default(),
            held_thumbs: [false; 2],
            presses: 0,
            prev: None,
            llr: 0.0,
            verdict: Verdict::Pending,
        }
    }

    /// 現在の判定
    #[must_use]
    pub const fn verdict(&self) -> Verdict {
        self.verdict
    }

    /// これまでの対数尤度比（ログ用）
    #[must_use]
    pub const fn llr(&self) -> f64 {
        self.llr
    }

    /// 数えた文字キーの Down の数（自動リピートは数えない）
    #[must_use]
    pub const fn presses(&self) -> usize {
        self.presses
    }

    /// 打鍵イベントを 1 つ渡し、判定を返す。
    pub fn observe(&mut self, event: DetectEvent) -> Verdict {
        if self.verdict.is_decided() {
            return self.verdict;
        }
        match event {
            DetectEvent::ThumbDown(side) => {
                if !self.held_chars.is_empty() {
                    self.verdict = Verdict::Japanese(JapaneseEvidence::ThumbOverlap);
                } else {
                    self.held_thumbs[side.index()] = true;
                }
            }
            DetectEvent::ThumbUp(side) => self.held_thumbs[side.index()] = false,
            DetectEvent::CharUp(pos) => {
                self.held_chars.remove(&pos);
            }
            DetectEvent::CharDown(pos) => self.on_char_down(pos),
        }
        self.verdict
    }

    fn on_char_down(&mut self, pos: PhysicalPos) {
        // 押したままの自動リピート（Up より前の再送）は数えない
        if !self.held_chars.insert(pos) {
            return;
        }
        if self.held_thumbs.iter().any(|&down| down) {
            self.verdict = Verdict::Japanese(JapaneseEvidence::ThumbOverlap);
            return;
        }
        self.presses += 1;
        self.llr += self.model.delta(self.prev, pos);
        self.prev = self.model.key_kana.get(&pos).copied();

        self.verdict = if self.llr >= self.params.ja_threshold {
            Verdict::Japanese(JapaneseEvidence::Likelihood)
        } else if self.llr <= -self.params.en_threshold {
            Verdict::NotJapanese
        } else if self.presses >= self.params.max_keys {
            Verdict::Undecided
        } else {
            Verdict::Pending
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanmap::KeyboardModel;
    use crate::yab::YabLayout;
    use std::path::Path;

    fn pos(row: u8, col: u8) -> PhysicalPos {
        PhysicalPos::new(row, col)
    }

    /// 小さな合成モデル: か(1,1) た(1,2) こ(1,3) さ(1,4)。「かた」「たこ」「こさ」が自然、他は表に無い。
    fn tiny_model() -> KanaModel {
        let ngram =
            NgramModel::from_csv("2,かた,0.0\n2,たこ,0.0\n2,こさ,0.0\n2,かこ,-3.0\n", 0, 0, 0)
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
        // 仮名に読めないが定義されているキー（記号）
        face.insert(pos(0, 0), YabValue::Literal("1".to_string()));
        KanaModel::new(&ngram, &face)
    }

    fn params(ja: f64, en: f64, max_keys: usize) -> DetectParams {
        DetectParams {
            ja_threshold: ja,
            en_threshold: en,
            max_keys,
        }
    }

    #[test]
    fn thumb_down_while_char_held_is_overlap() {
        let model = tiny_model();
        let mut d = EpisodeDetector::new(&model, params(100.0, 100.0, 7));
        assert_eq!(
            d.observe(DetectEvent::CharDown(pos(1, 1))),
            Verdict::Pending
        );
        assert_eq!(
            d.observe(DetectEvent::ThumbDown(ThumbSide::Left)),
            Verdict::Japanese(JapaneseEvidence::ThumbOverlap)
        );
    }

    #[test]
    fn char_down_while_thumb_held_is_overlap() {
        let model = tiny_model();
        let mut d = EpisodeDetector::new(&model, params(100.0, 100.0, 7));
        assert_eq!(
            d.observe(DetectEvent::ThumbDown(ThumbSide::Right)),
            Verdict::Pending
        );
        assert_eq!(
            d.observe(DetectEvent::CharDown(pos(1, 2))),
            Verdict::Japanese(JapaneseEvidence::ThumbOverlap)
        );
    }

    #[test]
    fn thumb_tap_alone_is_not_evidence() {
        let model = tiny_model();
        let mut d = EpisodeDetector::new(&model, params(100.0, 100.0, 7));
        d.observe(DetectEvent::ThumbDown(ThumbSide::Left));
        d.observe(DetectEvent::ThumbUp(ThumbSide::Left));
        assert_eq!(
            d.observe(DetectEvent::CharDown(pos(1, 1))),
            Verdict::Pending
        );
    }

    #[test]
    fn char_released_before_thumb_is_not_overlap() {
        let model = tiny_model();
        let mut d = EpisodeDetector::new(&model, params(100.0, 100.0, 7));
        d.observe(DetectEvent::CharDown(pos(1, 1)));
        d.observe(DetectEvent::CharUp(pos(1, 1)));
        assert_eq!(
            d.observe(DetectEvent::ThumbDown(ThumbSide::Left)),
            Verdict::Pending
        );
    }

    #[test]
    fn auto_repeat_is_not_counted() {
        let model = tiny_model();
        let mut d = EpisodeDetector::new(&model, params(100.0, 100.0, 7));
        d.observe(DetectEvent::CharDown(pos(1, 1)));
        d.observe(DetectEvent::CharDown(pos(1, 1)));
        d.observe(DetectEvent::CharDown(pos(1, 1)));
        assert_eq!(d.presses(), 1);
    }

    #[test]
    fn natural_kana_sequence_reaches_japanese_by_likelihood() {
        let model = tiny_model();
        // か→た→こ→さ は表で自然。閾値を低くして、尤度だけで日本語に届くことを確かめる。
        let mut d = EpisodeDetector::new(&model, params(1.0, 100.0, 7));
        let mut last = Verdict::Pending;
        for col in 1..=4 {
            last = d.observe(DetectEvent::CharDown(pos(1, col)));
            d.observe(DetectEvent::CharUp(pos(1, col)));
        }
        assert_eq!(last, Verdict::Japanese(JapaneseEvidence::Likelihood));
        assert!(d.llr() >= 1.0);
    }

    #[test]
    fn unnatural_sequence_becomes_not_japanese() {
        let model = tiny_model();
        // 記号キー(0,0)は仮名に読めず、尤度が下限になる。en_threshold を低くして早く「日本語でない」になる。
        let mut d = EpisodeDetector::new(&model, params(100.0, 5.0, 7));
        let mut last = Verdict::Pending;
        for _ in 0..3 {
            last = d.observe(DetectEvent::CharDown(pos(0, 0)));
            d.observe(DetectEvent::CharUp(pos(0, 0)));
        }
        assert_eq!(last, Verdict::NotJapanese);
    }

    #[test]
    fn max_keys_without_a_decision_is_undecided() {
        let model = tiny_model();
        let mut d = EpisodeDetector::new(&model, params(1000.0, 1000.0, 3));
        let mut last = Verdict::Pending;
        for col in 1..=3 {
            last = d.observe(DetectEvent::CharDown(pos(1, col)));
            d.observe(DetectEvent::CharUp(pos(1, col)));
        }
        assert_eq!(last, Verdict::Undecided);
    }

    #[test]
    fn verdict_is_sticky_after_a_decision() {
        let model = tiny_model();
        let mut d = EpisodeDetector::new(&model, params(100.0, 100.0, 7));
        d.observe(DetectEvent::CharDown(pos(1, 1)));
        let decided = d.observe(DetectEvent::ThumbDown(ThumbSide::Left));
        assert_eq!(
            d.observe(DetectEvent::ThumbUp(ThumbSide::Left)),
            decided,
            "決まったあとはイベントを無視する"
        );
    }

    #[test]
    fn empty_ngram_never_decides_japanese_by_likelihood() {
        let ngram = NgramModel::new(0, 0, 0);
        let mut face = YabFace::new();
        face.insert(
            pos(1, 1),
            YabValue::Romaji {
                romaji: "ka".to_string(),
                kana: Some('か'),
            },
        );
        let model = KanaModel::new(&ngram, &face);
        let mut d = EpisodeDetector::new(&model, DetectParams::default());
        for _ in 0..3 {
            d.observe(DetectEvent::CharDown(pos(1, 1)));
            d.observe(DetectEvent::CharUp(pos(1, 1)));
        }
        assert!(!matches!(
            d.verdict(),
            Verdict::Japanese(JapaneseEvidence::Likelihood)
        ));
    }

    // ---- 実データ（data/ngram_hiragana.csv.gz と layout/nicola.yab）での回帰 ----

    fn real_model() -> KanaModel {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let ngram = NgramModel::from_file(&root.join("data/ngram_hiragana.csv.gz"), 0, 0, 0)
            .expect("ngram file");
        let yab = std::fs::read_to_string(root.join("layout/nicola.yab")).expect("yab");
        let layout = YabLayout::parse(&yab, KeyboardModel::Jis).expect("layout");
        KanaModel::new(&ngram, &layout.normal)
    }

    /// QWERTY の物理位置（yab の行・列。JIS）
    fn qwerty(c: char) -> PhysicalPos {
        const ROWS: [&str; 4] = ["1234567890-", "qwertyuiop@[", "asdfghjkl;:]", "zxcvbnm,./"];
        for (r, row) in ROWS.iter().enumerate() {
            if let Some(col) = row.find(c) {
                return pos(
                    u8::try_from(r).expect("row"),
                    u8::try_from(col).expect("col"),
                );
            }
        }
        panic!("QWERTY に無い文字: {c}");
    }

    fn run(model: &KanaModel, text: &str) -> (Verdict, usize) {
        let mut d = EpisodeDetector::new(model, DetectParams::default());
        for c in text.chars() {
            d.observe(DetectEvent::CharDown(qwerty(c)));
            d.observe(DetectEvent::CharUp(qwerty(c)));
            if d.verdict().is_decided() {
                break;
            }
        }
        (d.verdict(), d.presses())
    }

    #[test]
    fn english_and_code_are_not_decided_japanese_with_default_params() {
        let model = real_model();
        for text in [
            "thequickbrown",
            "gitstatus",
            "importos",
            "constx",
            "cargotest",
        ] {
            let (verdict, _) = run(&model, text);
            assert!(
                !matches!(verdict, Verdict::Japanese(_)),
                "{text} が日本語と判定された: {verdict:?}"
            );
        }
    }

    #[test]
    fn english_code_is_released_early_as_not_japanese() {
        let model = real_model();
        // 「thequickbrown」は 4 打鍵目までに尤度が大きく下がる（評価: -9.3, -11.5, -44.8）。7 打鍵を待たずに解放できる。
        let (verdict, presses) = run(&model, "thequickbrown");
        assert_eq!(verdict, Verdict::NotJapanese);
        assert!(presses <= 4, "保持が長すぎる: {presses} 打鍵");
    }
}
