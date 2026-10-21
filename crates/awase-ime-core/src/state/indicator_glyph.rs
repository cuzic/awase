//! ADR-261 段階 1(純粋部分): タスクバーの入力インジケーターの字形(画像)を前処理し、見本の最近傍で状態を引く
//! **純粋な判定**（Win32 を呼ばない。Linux でテストできる）。
//!
//! 画面の取り込み(GDI)と UI Automation での要素探しは殻の `observer/` が行い、ここへは**グレースケールの画素列**
//! ([`GlyphGray`])だけを渡す。ここは画素 → 特徴([`GlyphFeature`])の前処理と、特徴 → [`GlyphReading`] の分類だけを持つ。
//! **belief へは入れない**(段階 1 は journal に出すだけ。ADR-261 決定 6)。
//!
//! ## 前処理(ADR-261 決定 1・「類似度による判定の実機測定」)
//!
//! 背景(中央値)との差の絶対値(ライト・ダークの両方で字形が出る) → 枠線(ほぼ全幅の行・列)を除く → 共通の高さ(96)へ
//! 3 次補間で**先に拡大**(整数倍でない 125% でも以後の半径が固定できる) → 大津の 2 値化 → ①クロージング+穴埋め
//! (ライトの中抜きの輪郭をダークの塗りつぶしにそろえる)と、②Sobel のエッジ+膨張の**2 通り** → 小さな連結成分を除去 →
//! 外接矩形で 20×20 に正規化 → 距離変換。比較は 2 通りの対称チャンファー距離の平均+縦横比の差(×0.1)。
//!
//! ## 閾値は暫定
//!
//! [`MIN_DOTS`]・[`Thresholds::default`] は実機の実データ(ADR-261 D0-11)で決める**暫定値**で、段階 1 の shadow で
//! 読める窓の直接の読みと突き合わせて見直す。合成画像のテストが固定するのは、スケール・テーマ・枠線に対する不変性と、
//! 判定の**構造**(最近傍・距離比・`Miss` の理由)であって、数値の最適性ではない。
//!
//! ## 判断はここだけに置く
//!
//! 「表に近くない・距離比が足りない・一様な画像」のとき何もしない(`Miss`)ことは、観測側(殻)に `if` を書かず、
//! ここで決める(`.claude/rules/ime-belief-architecture.md` の Observe → classify)。

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::many_single_char_names,
    clippy::similar_names,
    clippy::too_many_lines,
    clippy::suboptimal_flops,
    clippy::missing_const_for_fn,
    clippy::must_use_candidate,
    clippy::float_cmp,
    clippy::needless_range_loop,
    clippy::manual_memcpy,
    clippy::cognitive_complexity
)]

use serde::Serialize;

/// 前処理で拡大する共通の高さ。
pub const NORMALIZED_HEIGHT: usize = 96;
/// 正規化後の特徴の一辺。
pub const FEATURE_SIZE: usize = 20;
const FEATURE_AREA: usize = FEATURE_SIZE * FEATURE_SIZE;
/// 前処理後(高さ 96)の前景の点の最小数。これに満たない画像は `Miss(EmptyGlyph)`(暫定。ADR-261 決定 1・D0-11)。
pub const MIN_DOTS: usize = 150;
/// 背景との差の最大値がこれ未満の画像は一様とみなして `Miss(EmptyGlyph)`(暫定)。
const MIN_CONTRAST: u8 = 16;
/// 枠線とみなす行・列の画素の差の下限と、その行・列に占める割合(暫定)。
const FRAME_PIXEL_DIFF: u8 = 24;
const FRAME_FRACTION_PERCENT: usize = 90;
/// 連結成分としてこれ未満の面積は除く(高さ 96 の画像で)。
const MIN_COMPONENT_AREA: usize = 24;
/// 縦横比の差を距離へ足す重み(ADR-261)。
const ASPECT_WEIGHT: f32 = 0.1;
/// 前景の無い特徴との距離(使わないはずの保険)。
const FAR: f32 = 1.0e6;

/// 取り込んだインジケーターのグレースケール画像(8 ビット、行優先)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlyphGray {
    width: usize,
    height: usize,
    pixels: Vec<u8>,
}

impl GlyphGray {
    /// 画素数が `width * height` と一致し、幅・高さがともに 8 以上のときだけ作る。
    pub fn new(width: usize, height: usize, pixels: Vec<u8>) -> Option<Self> {
        if width < 8 || height < 8 || pixels.len() != width.checked_mul(height)? {
            return None;
        }
        Some(Self {
            width,
            height,
            pixels,
        })
    }

    pub fn width(&self) -> usize {
        self.width
    }

    /// 背景(中央値)が暗いか(ダークテーマ)。環境指紋の一部(ADR-261 決定 3)。閾値は画素値の中間(128)。
    pub fn background_is_dark(&self) -> bool {
        median(&self.pixels) < 128
    }

    pub fn height(&self) -> usize {
        self.height
    }
}

/// 字形の分類のキー(ADR-261 決定 2)。閉は mode を落として 1 つにする。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub enum GlyphClass {
    Closed,
    Hiragana,
    FullKatakana,
    FullAlnum,
    HalfKatakana,
    HalfAlnum,
}

impl GlyphClass {
    /// [`Self::label`] の逆(保存データの読み込み用)。
    pub fn from_label(label: &str) -> Option<Self> {
        [
            Self::Closed,
            Self::Hiragana,
            Self::FullKatakana,
            Self::FullAlnum,
            Self::HalfKatakana,
            Self::HalfAlnum,
        ]
        .into_iter()
        .find(|c| c.label() == label)
    }

    /// journal・ログ用の短い名前。
    pub const fn label(self) -> &'static str {
        match self {
            Self::Closed => "closed",
            Self::Hiragana => "hiragana",
            Self::FullKatakana => "full-katakana",
            Self::FullAlnum => "full-alnum",
            Self::HalfKatakana => "half-katakana",
            Self::HalfAlnum => "half-alnum",
        }
    }
}

/// `Miss`(何もしない)の理由。journal に残す。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum GlyphMiss {
    /// 画像の大きさと画素数が合わない
    BadImage,
    /// 一様・黒・覆われた領域など、前処理後の前景が少なすぎる(空白を閉と取り違えない。ADR-261 決定 1)
    EmptyGlyph,
    /// 表に見本が無い
    EmptyTable,
    /// 最近傍の距離が上限を超える(表のどの見本にも近くない)
    TooFar,
    /// 最近傍と次点の距離比が下限に満たない
    Ambiguous,
}

impl GlyphMiss {
    /// journal・ログ用の短い名前。
    pub const fn label(self) -> &'static str {
        match self {
            Self::BadImage => "bad-image",
            Self::EmptyGlyph => "empty-glyph",
            Self::EmptyTable => "empty-table",
            Self::TooFar => "too-far",
            Self::Ambiguous => "ambiguous",
        }
    }
}

/// 前処理した特徴。2 通りの 20×20 の 2 値画像・その距離変換・縦横比・前景の点数を持つ(元の画素は保持しない)。
#[derive(Debug, Clone, PartialEq)]
pub struct GlyphFeature {
    masks: [[bool; FEATURE_AREA]; 2],
    dts: [[f32; FEATURE_AREA]; 2],
    aspect: f32,
    dots: usize,
}

impl GlyphFeature {
    /// 保存用に、2 通りの 20×20 の 2 値画像・縦横比・前景の点数を取り出す(距離変換は復元時に作り直す)。
    pub fn to_parts(&self) -> ([[bool; FEATURE_AREA]; 2], f32, usize) {
        (self.masks, self.aspect, self.dots)
    }

    /// [`Self::to_parts`] の逆。距離変換はここで作り直す。2 値画像のどちらかに前景が無ければ `None`(壊れた保存データ)。
    pub fn from_parts(masks: [[bool; FEATURE_AREA]; 2], aspect: f32, dots: usize) -> Option<Self> {
        if !aspect.is_finite() || masks.iter().any(|m| !m.iter().any(|&v| v)) {
            return None;
        }
        Some(Self {
            dts: [distance_transform(&masks[0]), distance_transform(&masks[1])],
            masks,
            aspect,
            dots,
        })
    }

    /// 高さ 96 の画像での前景の点数(穴埋め後の方式)。
    pub fn dots(&self) -> usize {
        self.dots
    }

    /// 外接矩形の縦横比(幅 / 高さ)。
    pub fn aspect(&self) -> f32 {
        self.aspect
    }

    /// 2 つの特徴の距離(2 通りの対称チャンファー距離の平均+縦横比の差 × [`ASPECT_WEIGHT`])。
    pub fn distance(&self, other: &Self) -> f32 {
        let mut sum = 0.0_f32;
        for v in 0..2 {
            sum += symmetric_chamfer(&self.masks[v], &self.dts[v], &other.masks[v], &other.dts[v]);
        }
        sum / 2.0 + ASPECT_WEIGHT * (self.aspect - other.aspect).abs()
    }
}

/// 画素 → 特徴。`Err` は `Miss` の理由。
///
/// # Errors
/// 一様な画像・前景が少なすぎる画像は `GlyphMiss::EmptyGlyph`。
pub fn preprocess(img: &GlyphGray) -> Result<GlyphFeature, GlyphMiss> {
    let (w, h) = (img.width, img.height);
    let bg = median(&img.pixels);
    let mut diff: Vec<u8> = img.pixels.iter().map(|&p| p.abs_diff(bg)).collect();
    remove_frame(&mut diff, w, h);
    if diff.iter().copied().max().unwrap_or(0) < MIN_CONTRAST {
        return Err(GlyphMiss::EmptyGlyph);
    }

    let dh = NORMALIZED_HEIGHT;
    let dw = ((w * dh + h / 2) / h).max(1);
    let big = resize_bicubic(&diff, w, h, dw, dh);

    // ① 2 値化 → クロージング → 穴埋め
    let thr = otsu(&big);
    let base: Vec<bool> = big.iter().map(|&v| v > thr).collect();
    let closed = erode(&dilate(&base, dw, dh, 2), dw, dh, 2);
    let filled = fill_holes(&closed, dw, dh);
    let a = remove_small_components(&filled, dw, dh, MIN_COMPONENT_AREA);

    // ② Sobel のエッジ → 2 値化 → 膨張
    let edge = sobel(&big, dw, dh);
    let thr_e = otsu(&edge);
    let edge_mask: Vec<bool> = edge.iter().map(|&v| v > thr_e).collect();
    let b = remove_small_components(&dilate(&edge_mask, dw, dh, 1), dw, dh, MIN_COMPONENT_AREA);

    let dots = a.iter().filter(|&&v| v).count();
    if dots < MIN_DOTS {
        return Err(GlyphMiss::EmptyGlyph);
    }
    let (x0, y0, x1, y1) = bbox(&a, dw, dh).ok_or(GlyphMiss::EmptyGlyph)?;
    if !b.iter().any(|&v| v) {
        return Err(GlyphMiss::EmptyGlyph);
    }
    let mask_a = normalize(&a, dw, (x0, y0, x1, y1));
    let mask_b = normalize(&b, dw, (x0, y0, x1, y1));
    let aspect = (x1 - x0 + 1) as f32 / (y1 - y0 + 1) as f32;
    Ok(GlyphFeature {
        dts: [distance_transform(&mask_a), distance_transform(&mask_b)],
        masks: [mask_a, mask_b],
        aspect,
        dots,
    })
}

/// 表の見本の出所(距離比の下限を変える。ADR-261 決定 2・3)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SampleOrigin {
    /// 同じ環境(DPI・テーマ)で受け身に集めた見本
    SameEnv,
    /// 共通の初期見本(100% のライト・ダーク)
    Common,
}

impl SampleOrigin {
    /// journal・ログ用の短い名前。
    pub const fn label(self) -> &'static str {
        match self {
            Self::SameEnv => "same-env",
            Self::Common => "common",
        }
    }
}

/// 表の見本 1 枚。
#[derive(Debug, Clone)]
pub struct GlyphSample {
    pub class: GlyphClass,
    pub feature: GlyphFeature,
    pub origin: SampleOrigin,
}

/// `GlyphTable::add` の結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddOutcome {
    Added,
    /// そのクラスの見本の数が上限に達している
    Full,
    /// 同じクラスのほぼ同じ見本が既にある
    Duplicate,
    /// 別のクラスの見本に近すぎる(そのクラスの組の判定が曖昧になる。ADR-261 決定 2 の衝突)
    Conflict,
}

/// 判定の閾値(暫定。ADR-261 D0-11 で実データから決める)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Thresholds {
    /// 最近傍の距離の上限
    pub max_distance: f32,
    /// 同じ環境の見本が最近傍のときの距離比(次点÷最近傍)の下限
    pub min_ratio_same_env: f32,
    /// 共通の初期見本が最近傍のときの距離比の下限
    pub min_ratio_common: f32,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            max_distance: 1.5,
            min_ratio_same_env: 1.5,
            min_ratio_common: 2.0,
        }
    }
}

/// クラスごとに複数枚の見本を持つ表。
#[derive(Debug, Clone)]
pub struct GlyphTable {
    samples: Vec<GlyphSample>,
    /// 別クラスの見本どうしの距離がこれ以下なら `Conflict`(暫定)。
    min_separation: f32,
    /// 同じクラスの見本どうしの距離がこれ以下なら `Duplicate`。
    duplicate_distance: f32,
    /// 1 クラスの見本の数の上限(暫定。ADR-261 決定 2「上限は D0 で決める」)。
    max_per_class: usize,
}

impl Default for GlyphTable {
    fn default() -> Self {
        Self {
            samples: Vec::new(),
            min_separation: 0.4,
            duplicate_distance: 0.05,
            max_per_class: 6,
        }
    }
}

impl GlyphTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// 衝突の判定距離を変えて作る(テスト・実データでの調整用)。
    pub fn with_min_separation(min_separation: f32) -> Self {
        Self {
            min_separation,
            ..Self::default()
        }
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// 同じ環境で集めた見本(出所が [`SampleOrigin::SameEnv`])。永続化の対象(内蔵の共通の見本は含まない)。
    pub fn same_env_samples(&self) -> Vec<&GlyphSample> {
        self.samples
            .iter()
            .filter(|s| s.origin == SampleOrigin::SameEnv)
            .collect()
    }

    /// 共通の初期見本から表を作る(衝突・重複・上限の判定はしない。見本は出所が確かで、近い 2 クラスも意図的に持つ)。
    /// 以後の `add`(同じ環境の見本)は通常の判定を受ける。
    pub fn from_common_samples(samples: Vec<GlyphSample>) -> Self {
        Self {
            samples,
            ..Self::default()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// 見本を足す。別クラスに近すぎれば `Conflict`、同じクラスにほぼ同じ見本があれば `Duplicate`。
    pub fn add(&mut self, sample: GlyphSample) -> AddOutcome {
        if self
            .samples
            .iter()
            .filter(|s| s.class == sample.class)
            .count()
            >= self.max_per_class
        {
            return AddOutcome::Full;
        }
        for s in &self.samples {
            let d = s.feature.distance(&sample.feature);
            if s.class == sample.class {
                if d <= self.duplicate_distance {
                    return AddOutcome::Duplicate;
                }
            } else if d <= self.min_separation {
                return AddOutcome::Conflict;
            }
        }
        self.samples.push(sample);
        AddOutcome::Added
    }
}

/// 採った読み。
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct GlyphHit {
    pub class: GlyphClass,
    /// 最近傍の距離
    pub distance: f32,
    /// 次点÷最近傍(次点が無ければ `f32::INFINITY`)
    pub ratio: f32,
    /// 最近傍の見本の出所
    pub origin: SampleOrigin,
}

/// 字形の読み。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GlyphReading {
    Hit(GlyphHit),
    Miss(GlyphMiss),
}

/// 特徴を表で分類する。最近傍の**クラス**を採るのは、距離が上限以内で、次点クラスとの距離比が下限以上のときだけ。
pub fn classify_glyph(
    table: &GlyphTable,
    thresholds: &Thresholds,
    feature: &GlyphFeature,
) -> GlyphReading {
    if table.is_empty() {
        return GlyphReading::Miss(GlyphMiss::EmptyTable);
    }
    // クラスごとの最短距離(と、その見本の出所)。
    let mut best: Vec<(GlyphClass, f32, SampleOrigin)> = Vec::new();
    for s in &table.samples {
        let d = s.feature.distance(feature);
        match best.iter_mut().find(|(c, _, _)| *c == s.class) {
            Some(entry) => {
                if d < entry.1 {
                    entry.1 = d;
                    entry.2 = s.origin;
                }
            }
            None => best.push((s.class, d, s.origin)),
        }
    }
    best.sort_by(|a, b| a.1.total_cmp(&b.1));
    let (class, distance, origin) = best[0];
    if distance > thresholds.max_distance {
        return GlyphReading::Miss(GlyphMiss::TooFar);
    }
    let ratio = match best.get(1) {
        None => f32::INFINITY,
        Some(&(_, second, _)) if distance <= f32::EPSILON => {
            if second <= f32::EPSILON {
                1.0
            } else {
                f32::INFINITY
            }
        }
        Some(&(_, second, _)) => second / distance,
    };
    let need = match origin {
        SampleOrigin::SameEnv => thresholds.min_ratio_same_env,
        SampleOrigin::Common => thresholds.min_ratio_common,
    };
    if ratio < need {
        return GlyphReading::Miss(GlyphMiss::Ambiguous);
    }
    GlyphReading::Hit(GlyphHit {
        class,
        distance,
        ratio,
        origin,
    })
}

/// 画素から分類まで(前処理に失敗したら、その `Miss`)。
pub fn classify_image(
    table: &GlyphTable,
    thresholds: &Thresholds,
    img: &GlyphGray,
) -> GlyphReading {
    match preprocess(img) {
        Ok(f) => classify_glyph(table, thresholds, &f),
        Err(m) => GlyphReading::Miss(m),
    }
}

// ───────────────────── belief の粗いラベルと、受け身の収集の判断(純粋) ─────────────────────

/// belief が持つ粗い入力モード(閉・かな系・英数系)。段階 1 の shadow は、この粗いラベルで見本の表を作って突き合わせる。
///
/// ADR-261 決定 2 の `GlyphClass` は 6 つだが、belief の入力モードはひらがなとカタカナ(かな系)、半角英数と全角英数(英数系)を
/// 区別しない。Engine が必要とするのは「開いていて、ローマ字入力か英数か」なので、まずこの 3 つで一致を見る。
/// 表のキーには [`GlyphClass`] の代表(閉=`Closed`、かな系=`Hiragana`、英数系=`HalfAlnum`)を使う([`Self::class`])。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum BeliefLabel {
    Closed,
    Kana,
    Eisu,
}

impl BeliefLabel {
    /// journal・ログ用の短い名前。
    pub const fn label(self) -> &'static str {
        match self {
            Self::Closed => "closed",
            Self::Kana => "kana",
            Self::Eisu => "eisu",
        }
    }

    /// 表のキーに使う代表の [`GlyphClass`]。
    pub const fn class(self) -> GlyphClass {
        match self {
            Self::Closed => GlyphClass::Closed,
            Self::Kana => GlyphClass::Hiragana,
            Self::Eisu => GlyphClass::HalfAlnum,
        }
    }

    /// [`GlyphClass`] が属する粗いラベル(閉・かな系=ひらがな/カタカナ/半角カナ・英数系=全角英数/半角英数)。
    pub const fn from_class(class: GlyphClass) -> Self {
        match class {
            GlyphClass::Closed => Self::Closed,
            GlyphClass::Hiragana | GlyphClass::FullKatakana | GlyphClass::HalfKatakana => {
                Self::Kana
            }
            GlyphClass::FullAlnum | GlyphClass::HalfAlnum => Self::Eisu,
        }
    }
}

/// belief の (開閉, 入力モード) から粗いラベルを作る。**確認された状態だけ**を採る: 閉は入力モードに依らず `Closed`、開いていて
/// `ObservedRomaji`/`ObservedKana` は `Kana`、`ObservedEisu` は `Eisu`。仮定(`AssumedRomaji`)・不明は `None`(ラベルにしない)。
pub const fn belief_label(
    open: bool,
    mode: awase::engine::mode_state::InputModeState,
) -> Option<BeliefLabel> {
    use awase::engine::mode_state::InputModeState;
    if !open {
        return Some(BeliefLabel::Closed);
    }
    match mode {
        InputModeState::ObservedRomaji | InputModeState::ObservedKana => Some(BeliefLabel::Kana),
        InputModeState::ObservedEisu => Some(BeliefLabel::Eisu),
        InputModeState::AssumedRomaji { .. } | InputModeState::Unknown => None,
    }
}

/// 同じ窓の 2 回の取り込み(待ちが違う)の特徴の距離がこれ以下なら「字形が落ち着いている」(暫定。ADR-261 決定 2 の安定化)。
pub const STABLE_DISTANCE: f32 = 0.05;

/// 受け身の収集で、取り込み 1 回の特徴と、そのときの belief のラベル。
#[derive(Debug, Clone, Copy)]
pub struct Capture<'a> {
    pub feature: &'a GlyphFeature,
    pub label: Option<BeliefLabel>,
}

/// 見本として表に足してよいか(足すなら、そのラベル)。**読める窓**(`readable`。belief が直接の読みに追随している窓)で、
/// 同じ窓の前回(待ちが短い方)の取り込みと今回で、ラベルが同じ(確認された状態)で、字形が落ち着いている(特徴の距離が
/// [`STABLE_DISTANCE`] 以下)ときだけ。前回が無い(最初の取り込み)なら足さない。
pub fn collect_label(
    prev: Option<Capture<'_>>,
    now: Capture<'_>,
    readable: bool,
) -> Option<BeliefLabel> {
    if !readable {
        return None;
    }
    let prev = prev?;
    let label = now.label?;
    if prev.label? != label {
        return None;
    }
    (prev.feature.distance(now.feature) <= STABLE_DISTANCE).then_some(label)
}

/// ラベル単位の分類で採った読み。
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct LabelHit {
    pub label: BeliefLabel,
    /// 最近傍の見本のクラス(journal の診断用。ラベルより細かい)
    pub class: GlyphClass,
    /// 最近傍の距離
    pub distance: f32,
    /// 次点のラベルとの距離比(次点が無ければ `f32::INFINITY`)
    pub ratio: f32,
    /// 最近傍の見本の出所
    pub origin: SampleOrigin,
}

/// ラベル単位の読み。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LabelReading {
    Hit(LabelHit),
    Miss(GlyphMiss),
}

/// 特徴を**ラベル(閉・かな・英数)単位**で分類する。最近傍と次点を、クラスではなくラベルで比べる: 同じラベルの 2 クラス
/// (全角英数と半角英数など)が近くても曖昧にならない(Engine が必要なのは、閉・かな系・英数系の区別だけ。ADR-261)。
/// 採るのは、最近傍の距離が上限以内で、次点のラベルとの距離比が下限(最近傍の見本の出所で決まる)以上のときだけ。
pub fn classify_label(
    table: &GlyphTable,
    thresholds: &Thresholds,
    feature: &GlyphFeature,
) -> LabelReading {
    if table.is_empty() {
        return LabelReading::Miss(GlyphMiss::EmptyTable);
    }
    let mut best: Vec<(BeliefLabel, GlyphClass, f32, SampleOrigin)> = Vec::new();
    for s in &table.samples {
        let d = s.feature.distance(feature);
        let label = BeliefLabel::from_class(s.class);
        match best.iter_mut().find(|e| e.0 == label) {
            Some(e) => {
                if d < e.2 {
                    *e = (label, s.class, d, s.origin);
                }
            }
            None => best.push((label, s.class, d, s.origin)),
        }
    }
    best.sort_by(|a, b| a.2.total_cmp(&b.2));
    let (label, class, distance, origin) = best[0];
    if distance > thresholds.max_distance {
        return LabelReading::Miss(GlyphMiss::TooFar);
    }
    let ratio = match best.get(1) {
        None => f32::INFINITY,
        Some(&(_, _, second, _)) if distance <= f32::EPSILON => {
            if second <= f32::EPSILON {
                1.0
            } else {
                f32::INFINITY
            }
        }
        Some(&(_, _, second, _)) => second / distance,
    };
    let need = match origin {
        SampleOrigin::SameEnv => thresholds.min_ratio_same_env,
        SampleOrigin::Common => thresholds.min_ratio_common,
    };
    if ratio < need {
        return LabelReading::Miss(GlyphMiss::Ambiguous);
    }
    LabelReading::Hit(LabelHit {
        label,
        class,
        distance,
        ratio,
        origin,
    })
}

/// 字形の読みを belief に報告してよいかの文脈（ADR-273 D4 の柵）。呼び出し側（殻）が持つ事実を値で渡す。
///
/// **3 つの事実は呼び出し側の自己申告**で、型が強制できるのは「柵の関数を通った」ことまで（`since_last_mode_key_ms: None` を
/// 渡せば素通しになる）。だから reducer にも同じ柵を二重に置いている（`ImeModel::last_mode_key_tick_ms`。予測の有無に依らず、
/// journal の再生でも決定的）。起点は「最後のモード系の入力」: 物理モードキーの予測・通過・明示意図・awase 自身の書き込み。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FenceCtx {
    /// 最後のモード系の打鍵（`KeyEffectPredicted` が付く打鍵）からの経過(ms)。打鍵が無ければ `None`
    pub since_last_mode_key_ms: Option<u64>,
    /// 取り込みを依頼した時点と報告する時点で、フォーカスの世代（`FocusGen`）が同じか
    pub focus_gen_unchanged: bool,
    /// 前面窓が、awase のフォーカス追跡が追っている窓と一致しているか（`lang_detect_focus_in_sync` と同じ型）
    pub foreground_in_sync: bool,
}

/// 柵を通らなかった理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum FenceMiss {
    /// 最後のモード系の打鍵から [`crate::tuning::GLYPH_KEY_FENCE_MS`] 経っていない（押す前の状態を映しているかもしれない）
    TooSoonAfterModeKey,
    /// 取り込みの間にフォーカスの世代が変わった（別の窓の字形）
    FocusMoved,
    /// 前面窓が追跡中の窓と一致していない（Alt+Tab の直後など。字形は前面を映す）
    ForegroundOutOfSync,
}

/// 柵を通った字形の読み。**belief へ報告できるのはこの型だけ**（`Observed::<TaskbarGlyph>::from_glyph` の witness）。
/// 構築子は [`fence_glyph`] だけで、`LabelHit` を直接渡して作ることはできない（フィールドが private）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FencedGlyph {
    hit: LabelHit,
}

impl FencedGlyph {
    /// 読みのラベル。
    #[must_use]
    pub const fn label(&self) -> BeliefLabel {
        self.hit.label
    }
}

/// 字形の読み（照合で `Hit` したもの）に、ADR-273 D4 の柵を掛ける。**押す前の状態を映した字形・別の窓の字形・前面と追跡がずれた
/// ときの字形は通さない**（B3: 遅れて届いた古い字形が、正しい予測を消して belief を押す前へ戻すのを防ぐ）。
///
/// # Errors
/// 柵を通らなかったとき、その理由。
pub fn fence_glyph(hit: LabelHit, ctx: FenceCtx) -> Result<FencedGlyph, FenceMiss> {
    if !ctx.focus_gen_unchanged {
        return Err(FenceMiss::FocusMoved);
    }
    if !ctx.foreground_in_sync {
        return Err(FenceMiss::ForegroundOutOfSync);
    }
    if ctx
        .since_last_mode_key_ms
        .is_some_and(|ms| ms < crate::tuning::GLYPH_KEY_FENCE_MS)
    {
        return Err(FenceMiss::TooSoonAfterModeKey);
    }
    Ok(FencedGlyph { hit })
}

/// 窓の種類についての事実（字形を belief に使ってよい窓かの判定の入力。ADR-273 D3・Q2・R1）。殻が集めて値で渡す。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GlyphScopeFacts {
    /// **実効の**プロファイルが `Standard`（学習で降格した後の値。降格した窓は偽。静的に `Standard` の窓だけが真）
    pub effective_standard: bool,
    /// 実測学習で `Standard` から降格された窓（NULL 型。D3 の述語 `is_demoted_by_learning`）
    pub demoted_by_learning: bool,
    /// 実質 TSF ネイティブの窓（`is_effectively_tsf_native`）
    pub tsf_native: bool,
    /// 入力中継ツールの窓
    pub input_relay: bool,
    /// mozc-readable の共有メモリのブロックがある窓（そちらが High で読める。R1）
    pub mozc_block: bool,
}

/// 字形を belief に使ってよい窓か（ADR-273 D3・Q2・R1）。
///
/// `level` が `unobserved` 未満なら常に偽。入力中継・実質 TSF ネイティブ・mozc の共有メモリがある窓は常に除外。
/// `unobserved` は実測学習で降格された窓（NULL 型）だけ、`all` はそれに加えて静的に `Standard` の窓
/// （**降格した窓を含む**。静的に `Imm32Unavailable` の窓は除外。降格は `Standard` からしか起きない）。
#[must_use]
pub fn glyph_window_in_scope(level: awase::config::EvidenceLevel, f: GlyphScopeFacts) -> bool {
    use awase::config::EvidenceLevel;
    if level < EvidenceLevel::Unobserved || f.input_relay || f.tsf_native || f.mozc_block {
        return false;
    }
    if level == EvidenceLevel::All {
        f.effective_standard || f.demoted_by_learning
    } else {
        f.demoted_by_learning
    }
}

/// journal 用: この窓が `unobserved`/`all` のどちらの段階から字形を belief に使う対象か（`none` = どちらでも対象外）。
#[must_use]
pub fn glyph_scope_label(f: GlyphScopeFacts) -> &'static str {
    use awase::config::EvidenceLevel;
    if glyph_window_in_scope(EvidenceLevel::Unobserved, f) {
        "unobserved"
    } else if glyph_window_in_scope(EvidenceLevel::All, f) {
        "all"
    } else {
        "none"
    }
}

/// 取り込み 1 回の照合の結果(journal に残す)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MatchResult {
    /// 照合で採ったラベル(`Hit` のとき)
    pub reading: Option<BeliefLabel>,
    /// 最近傍の見本のクラス(`Hit` のとき。診断用)
    pub class: Option<GlyphClass>,
    /// 最近傍の見本の出所(`Hit` のとき)
    pub origin: Option<SampleOrigin>,
    /// `Miss` の理由(`Hit` のときは `None`)
    pub miss: Option<GlyphMiss>,
    /// 最近傍の距離と距離比(`Hit` のとき)
    pub distance: Option<f32>,
    pub ratio: Option<f32>,
}

/// 表で分類し、journal 向けの結果にする。
pub fn match_feature(
    table: &GlyphTable,
    thresholds: &Thresholds,
    feature: &GlyphFeature,
) -> MatchResult {
    match classify_label(table, thresholds, feature) {
        LabelReading::Hit(h) => MatchResult {
            reading: Some(h.label),
            class: Some(h.class),
            origin: Some(h.origin),
            miss: None,
            distance: Some(h.distance),
            ratio: Some(h.ratio),
        },
        LabelReading::Miss(m) => MatchResult {
            reading: None,
            class: None,
            origin: None,
            miss: Some(m),
            distance: None,
            ratio: None,
        },
    }
}

// ───────────────────── インジケーター要素の選び方・プローブ結果(純粋) ─────────────────────

/// インジケーター要素の名前の接頭辞(表示言語ごと)。日本語 Windows の実機(dragonflyg4)では `トレイ入力インジケーター`、
/// CI(英語)では `Tray Input Indicator`。接頭辞が一致する要素が複数あるときの選び方は [`select_indicator`]。
pub const INDICATOR_NAME_PREFIXES: [&str; 2] = ["Tray Input Indicator", "トレイ入力インジケーター"];

/// 名前がインジケーターの接頭辞に当たるか。
pub fn name_is_indicator(name: &str) -> bool {
    INDICATOR_NAME_PREFIXES.iter().any(|p| name.starts_with(p))
}

/// UI Automation で見つけた、名前がインジケーターの接頭辞に当たる要素 1 つ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndicatorCandidate {
    pub name: String,
    /// `(left, top, right, bottom)`（物理ピクセル。呼び出し側が DPI 対応で取る）
    pub rect: (i32, i32, i32, i32),
}

/// [`select_indicator`] が選べなかった理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectMiss {
    /// 接頭辞に当たる要素が無い(タスクバーの自動非表示・インジケーター無し・UIA の木がツールチップ窓に隠れている等)
    NotFound,
    /// 接頭辞に当たる要素が複数あり、1 つに絞れない
    Ambiguous,
    /// 矩形が小さすぎる・空(画面外)
    BadRect,
}

/// 字形を読む対象の要素を選ぶ。
///
/// **実機の発見(ADR-261 決定 1)**: 日本語 Windows の実機では、名前が `トレイ入力インジケーター` で
/// 始まる要素が 2 つある(クラス・`AutomationId`・パターンは同じ)。モードの字形(「あ」)を描くのは**名前が 1 行**の方
/// (`… 右クリックで IME のオプションを表示します`)で、もう一方は IME 名・切替キーを**複数行**で持つ別のボタン
/// (IME 自身のアイコン)。表示言語に依らず、改行を含まない 1 つを選ぶ。接頭辞に当たる要素が 1 つだけならそれを選ぶ。
///
/// # Errors
/// 見つからない・絞れない・矩形が小さすぎるとき。
pub fn select_indicator(candidates: &[IndicatorCandidate]) -> Result<usize, SelectMiss> {
    let hits: Vec<usize> = candidates
        .iter()
        .enumerate()
        .filter(|(_, c)| name_is_indicator(&c.name))
        .map(|(i, _)| i)
        .collect();
    let chosen = match hits.as_slice() {
        [] => return Err(SelectMiss::NotFound),
        [one] => *one,
        many => {
            let single_line: Vec<usize> = many
                .iter()
                .copied()
                .filter(|&i| !candidates[i].name.contains('\n'))
                .collect();
            match single_line.as_slice() {
                [one] => *one,
                _ => return Err(SelectMiss::Ambiguous),
            }
        }
    };
    let (l, t, r, b) = candidates[chosen].rect;
    if r.saturating_sub(l) < 8 || b.saturating_sub(t) < 8 {
        return Err(SelectMiss::BadRect);
    }
    Ok(chosen)
}

/// 字形の取り込み 1 回の失敗の理由(journal・不具合報告に残す)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ProbeMiss {
    /// インジケーターの要素が見つからない
    NoIndicator,
    /// 接頭辞に当たる要素が複数で絞れない
    AmbiguousIndicator,
    /// 矩形が小さすぎる・空
    BadRect,
    /// 画面の取り込み(GDI)に失敗した
    CaptureFailed,
    /// 取り込んだ画像が一様・前景が少ない(空白を閉と取り違えない)
    EmptyGlyph,
}

impl ProbeMiss {
    /// journal・ログ用の短い名前。
    pub const fn label(self) -> &'static str {
        match self {
            Self::NoIndicator => "no-indicator",
            Self::AmbiguousIndicator => "ambiguous-indicator",
            Self::BadRect => "bad-rect",
            Self::CaptureFailed => "capture-failed",
            Self::EmptyGlyph => "empty-glyph",
        }
    }

    const fn code(self) -> u32 {
        match self {
            Self::NoIndicator => 1,
            Self::AmbiguousIndicator => 2,
            Self::BadRect => 3,
            Self::CaptureFailed => 4,
            Self::EmptyGlyph => 5,
        }
    }

    const fn from_code(code: u32) -> Option<Self> {
        match code {
            1 => Some(Self::NoIndicator),
            2 => Some(Self::AmbiguousIndicator),
            3 => Some(Self::BadRect),
            4 => Some(Self::CaptureFailed),
            5 => Some(Self::EmptyGlyph),
            _ => None,
        }
    }
}

impl From<SelectMiss> for ProbeMiss {
    fn from(m: SelectMiss) -> Self {
        match m {
            SelectMiss::NotFound => Self::NoIndicator,
            SelectMiss::Ambiguous => Self::AmbiguousIndicator,
            SelectMiss::BadRect => Self::BadRect,
        }
    }
}

/// 読み取りワーカーが返す、字形の取り込み 1 回の結果。**元の画素は持たない**(大きさ・前景の点数・所要だけ)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProbeOutcome {
    /// `Ok(())` = 前処理まで通った(段階 1 の前半は、表への照合までは行わない)。`Err` = `Miss` の理由
    pub result: Result<(), ProbeMiss>,
    /// 取り込んだ矩形の大きさ(物理ピクセル。DPI に比例する)。取り込めなかったときは 0
    pub width: u16,
    pub height: u16,
    /// 前処理後(高さ 96)の前景の点数。前処理まで進まなかったときは 0
    pub dots: u16,
    /// 取り込み〜前処理の所要(ミリ秒)
    pub read_ms: u16,
    /// 取り込んだ矩形の背景が暗いか(ダークテーマ。環境指紋の一部)。取り込めなかったときは `false`
    pub dark: bool,
}

impl ProbeOutcome {
    /// `WM_INDICATOR_GLYPH_READ` の `(wParam, lParam)` に符号化する(32 ビットでも収まる固定長)。
    /// `wParam` = 結果の符号(4 ビット)| 幅(10 ビット)<< 4 | 高さ(10 ビット)<< 14 | 背景が暗いか(1 ビット)<< 24。
    /// `lParam` = 点数(16)| 所要(16)<< 16。
    pub fn pack(self) -> (usize, isize) {
        let code = match self.result {
            Ok(()) => 0,
            Err(m) => m.code(),
        };
        let w = u32::from(self.width.min(1023));
        let h = u32::from(self.height.min(1023));
        let wparam = code | (w << 4) | (h << 14) | (u32::from(self.dark) << 24);
        let lparam = u32::from(self.dots) | (u32::from(self.read_ms) << 16);
        (wparam as usize, lparam as i32 as isize)
    }

    /// [`Self::pack`] の逆。符号が未知なら `None`（壊れたメッセージ）。
    pub fn unpack(wparam: usize, lparam: isize) -> Option<Self> {
        let wp = wparam as u32;
        let lp = lparam as i32 as u32;
        let code = wp & 0xF;
        let result = if code == 0 {
            Ok(())
        } else {
            Err(ProbeMiss::from_code(code)?)
        };
        Some(Self {
            result,
            width: ((wp >> 4) & 0x3FF) as u16,
            height: ((wp >> 14) & 0x3FF) as u16,
            dots: (lp & 0xFFFF) as u16,
            read_ms: (lp >> 16) as u16,
            dark: (wp >> 24) & 1 == 1,
        })
    }

    /// journal の記録にする。`profile`・`belief_open`・`delay_ms` は、読み取りを依頼した時点の文脈(呼び出し側が持つ)。
    pub fn to_record(
        self,
        profile: &'static str,
        belief_open: Option<bool>,
        delay_ms: u32,
    ) -> crate::journal::IndicatorGlyphRecord {
        crate::journal::IndicatorGlyphRecord {
            verdict: if self.result.is_ok() {
                "captured"
            } else {
                "miss"
            },
            miss_reason: self.result.err().map(ProbeMiss::label),
            width: u32::from(self.width),
            height: u32::from(self.height),
            dots: u32::from(self.dots),
            read_ms: u32::from(self.read_ms),
            delay_ms,
            profile,
            belief_open,
            ..Default::default()
        }
    }
}

/// 照合の結果を journal の記録に写す(`to_record` で作った記録に、照合・収集の欄を足す)。開閉の軸の一致(`agree_open`)は、
/// 記録に入っている belief の開閉(`belief_open`)と読みから決める。
pub fn apply_match(
    record: &mut crate::journal::IndicatorGlyphRecord,
    m: &MatchResult,
    belief: Option<BeliefLabel>,
    collected: bool,
    table_size: usize,
) {
    record.reading = m.reading.map(BeliefLabel::label);
    record.reading_class = m.class.map(GlyphClass::label);
    record.reading_origin = m.origin.map(SampleOrigin::label);
    record.match_miss = m.miss.map(GlyphMiss::label);
    record.distance_x100 = m.distance.map(x100);
    record.ratio_x100 = m.ratio.map(x100);
    record.belief_label = belief.map(BeliefLabel::label);
    record.agree = match (m.reading, belief) {
        (Some(r), Some(b)) => Some(r == b),
        _ => None,
    };
    // 開閉の軸だけの一致: 字形が「閉」なら belief も閉のはず、かな・英数なら開のはず。入力モードが不明でも数えられる。
    record.agree_open = match (m.reading, record.belief_open) {
        (Some(r), Some(open)) => Some((r != BeliefLabel::Closed) == open),
        _ => None,
    };
    record.collected = collected;
    record.table_size = u32::try_from(table_size).unwrap_or(u32::MAX);
}

/// 距離・距離比を 100 倍の整数にする(無限大・巨大な値は 9999 で頭打ち)。
fn x100(v: f32) -> u32 {
    if v.is_finite() {
        (v * 100.0).round().clamp(0.0, 9999.0) as u32
    } else {
        9999
    }
}

// ───────────────────────── 画像処理(すべて純粋) ─────────────────────────

fn median(pixels: &[u8]) -> u8 {
    let mut hist = [0usize; 256];
    for &p in pixels {
        hist[usize::from(p)] += 1;
    }
    let half = pixels.len() / 2;
    let mut acc = 0;
    for (v, &n) in hist.iter().enumerate() {
        acc += n;
        if acc > half {
            return v as u8;
        }
    }
    255
}

/// ほぼ全幅の行・全高の列(枠線)を 0 にする。
fn remove_frame(diff: &mut [u8], w: usize, h: usize) {
    let mut bad_rows = Vec::new();
    for y in 0..h {
        let n = diff[y * w..(y + 1) * w]
            .iter()
            .filter(|&&v| v >= FRAME_PIXEL_DIFF)
            .count();
        if n * 100 >= w * FRAME_FRACTION_PERCENT {
            bad_rows.push(y);
        }
    }
    let mut bad_cols = Vec::new();
    for x in 0..w {
        let n = (0..h)
            .filter(|&y| diff[y * w + x] >= FRAME_PIXEL_DIFF)
            .count();
        if n * 100 >= h * FRAME_FRACTION_PERCENT {
            bad_cols.push(x);
        }
    }
    for y in bad_rows {
        for x in 0..w {
            diff[y * w + x] = 0;
        }
    }
    for x in bad_cols {
        for y in 0..h {
            diff[y * w + x] = 0;
        }
    }
}

fn cubic_weight(t: f32) -> f32 {
    // Keys の 3 次畳み込み(a = -0.5)
    let a = -0.5_f32;
    let t = t.abs();
    if t <= 1.0 {
        (a + 2.0) * t * t * t - (a + 3.0) * t * t + 1.0
    } else if t < 2.0 {
        a * t * t * t - 5.0 * a * t * t + 8.0 * a * t - 4.0 * a
    } else {
        0.0
    }
}

fn resize_bicubic(src: &[u8], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<u8> {
    let mut out = vec![0u8; dw * dh];
    let sx_scale = sw as f32 / dw as f32;
    let sy_scale = sh as f32 / dh as f32;
    for y in 0..dh {
        let sy = (y as f32 + 0.5) * sy_scale - 0.5;
        let y0 = sy.floor() as isize;
        for x in 0..dw {
            let sx = (x as f32 + 0.5) * sx_scale - 0.5;
            let x0 = sx.floor() as isize;
            let mut acc = 0.0_f32;
            let mut wsum = 0.0_f32;
            for j in -1..=2_isize {
                let yy = (y0 + j).clamp(0, sh as isize - 1) as usize;
                let wy = cubic_weight(sy - (y0 + j) as f32);
                for i in -1..=2_isize {
                    let xx = (x0 + i).clamp(0, sw as isize - 1) as usize;
                    let wx = cubic_weight(sx - (x0 + i) as f32);
                    acc += f32::from(src[yy * sw + xx]) * wx * wy;
                    wsum += wx * wy;
                }
            }
            let v = if wsum.abs() > f32::EPSILON {
                acc / wsum
            } else {
                0.0
            };
            out[y * dw + x] = v.round().clamp(0.0, 255.0) as u8;
        }
    }
    out
}

/// 大津の 2 値化の閾値(`v > thr` を前景とする)。
fn otsu(img: &[u8]) -> u8 {
    let mut hist = [0usize; 256];
    for &p in img {
        hist[usize::from(p)] += 1;
    }
    let total = img.len() as f64;
    let sum_all: f64 = hist
        .iter()
        .enumerate()
        .map(|(i, &n)| i as f64 * n as f64)
        .sum();
    let (mut w_b, mut sum_b) = (0.0_f64, 0.0_f64);
    let (mut best_var, mut best_t) = (-1.0_f64, 0u8);
    for t in 0..256usize {
        w_b += hist[t] as f64;
        if w_b <= 0.0 {
            continue;
        }
        let w_f = total - w_b;
        if w_f <= 0.0 {
            break;
        }
        sum_b += t as f64 * hist[t] as f64;
        let m_b = sum_b / w_b;
        let m_f = (sum_all - sum_b) / w_f;
        let var = w_b * w_f * (m_b - m_f) * (m_b - m_f);
        if var > best_var {
            best_var = var;
            best_t = t as u8;
        }
    }
    best_t
}

fn dilate(mask: &[bool], w: usize, h: usize, r: usize) -> Vec<bool> {
    morph(mask, w, h, r, true)
}

fn erode(mask: &[bool], w: usize, h: usize, r: usize) -> Vec<bool> {
    morph(mask, w, h, r, false)
}

/// 正方形(半径 `r`)の窓での膨張(`any`)・収縮(`all`)。画像の外は膨張では偽、収縮では偽(端を削る)として扱う。
fn morph(mask: &[bool], w: usize, h: usize, r: usize, any: bool) -> Vec<bool> {
    // 横 → 縦の分離可能な処理。
    let pass = |src: &[bool], horizontal: bool| -> Vec<bool> {
        let mut out = vec![false; w * h];
        for y in 0..h {
            for x in 0..w {
                let mut found_true = false;
                let mut all_true = true;
                for d in 0..=(2 * r) {
                    let (xx, yy) = if horizontal {
                        (x as isize + d as isize - r as isize, y as isize)
                    } else {
                        (x as isize, y as isize + d as isize - r as isize)
                    };
                    let inside = xx >= 0 && yy >= 0 && (xx as usize) < w && (yy as usize) < h;
                    let v = inside && src[yy as usize * w + xx as usize];
                    found_true |= v;
                    all_true &= v;
                }
                out[y * w + x] = if any { found_true } else { all_true };
            }
        }
        out
    };
    let tmp = pass(mask, true);
    pass(&tmp, false)
}

/// 画像の縁から到達できない偽(穴)を真にする。
fn fill_holes(mask: &[bool], w: usize, h: usize) -> Vec<bool> {
    let mut outside = vec![false; w * h];
    let mut stack: Vec<usize> = Vec::new();
    let push = |x: usize, y: usize, outside: &mut Vec<bool>, stack: &mut Vec<usize>| {
        let i = y * w + x;
        if !mask[i] && !outside[i] {
            outside[i] = true;
            stack.push(i);
        }
    };
    for x in 0..w {
        push(x, 0, &mut outside, &mut stack);
        push(x, h - 1, &mut outside, &mut stack);
    }
    for y in 0..h {
        push(0, y, &mut outside, &mut stack);
        push(w - 1, y, &mut outside, &mut stack);
    }
    while let Some(i) = stack.pop() {
        let (x, y) = (i % w, i / w);
        if x > 0 {
            push(x - 1, y, &mut outside, &mut stack);
        }
        if x + 1 < w {
            push(x + 1, y, &mut outside, &mut stack);
        }
        if y > 0 {
            push(x, y - 1, &mut outside, &mut stack);
        }
        if y + 1 < h {
            push(x, y + 1, &mut outside, &mut stack);
        }
    }
    (0..w * h).map(|i| mask[i] || !outside[i]).collect()
}

fn sobel(img: &[u8], w: usize, h: usize) -> Vec<u8> {
    let at = |x: isize, y: isize| -> i32 {
        let xx = x.clamp(0, w as isize - 1) as usize;
        let yy = y.clamp(0, h as isize - 1) as usize;
        i32::from(img[yy * w + xx])
    };
    let mut out = vec![0u8; w * h];
    for y in 0..h as isize {
        for x in 0..w as isize {
            let gx = -at(x - 1, y - 1) - 2 * at(x - 1, y) - at(x - 1, y + 1)
                + at(x + 1, y - 1)
                + 2 * at(x + 1, y)
                + at(x + 1, y + 1);
            let gy = -at(x - 1, y - 1) - 2 * at(x, y - 1) - at(x + 1, y - 1)
                + at(x - 1, y + 1)
                + 2 * at(x, y + 1)
                + at(x + 1, y + 1);
            out[y as usize * w + x as usize] = ((gx.abs() + gy.abs()) / 4).clamp(0, 255) as u8;
        }
    }
    out
}

/// 8 連結で、面積が `min_area` 未満の連結成分を除く。
fn remove_small_components(mask: &[bool], w: usize, h: usize, min_area: usize) -> Vec<bool> {
    let mut seen = vec![false; w * h];
    let mut out = mask.to_vec();
    for start in 0..w * h {
        if !mask[start] || seen[start] {
            continue;
        }
        let mut comp = vec![start];
        seen[start] = true;
        let mut k = 0;
        while k < comp.len() {
            let i = comp[k];
            k += 1;
            let (x, y) = ((i % w) as isize, (i / w) as isize);
            for dy in -1..=1_isize {
                for dx in -1..=1_isize {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx < 0 || ny < 0 || nx >= w as isize || ny >= h as isize {
                        continue;
                    }
                    let j = ny as usize * w + nx as usize;
                    if mask[j] && !seen[j] {
                        seen[j] = true;
                        comp.push(j);
                    }
                }
            }
        }
        if comp.len() < min_area {
            for i in comp {
                out[i] = false;
            }
        }
    }
    out
}

/// 前景の外接矩形 `(x0, y0, x1, y1)`(端を含む)。
fn bbox(mask: &[bool], w: usize, h: usize) -> Option<(usize, usize, usize, usize)> {
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
    let mut any = false;
    for y in 0..h {
        for x in 0..w {
            if mask[y * w + x] {
                any = true;
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    any.then_some((x0, y0, x1, y1))
}

/// 外接矩形を 20×20 の 2 値画像にする(各セルの前景の割合が半分以上なら真)。
fn normalize(
    mask: &[bool],
    w: usize,
    (x0, y0, x1, y1): (usize, usize, usize, usize),
) -> [bool; FEATURE_AREA] {
    let bw = x1 - x0 + 1;
    let bh = y1 - y0 + 1;
    let mut out = [false; FEATURE_AREA];
    for j in 0..FEATURE_SIZE {
        let ys = y0 + j * bh / FEATURE_SIZE;
        let ye = (y0 + (j + 1) * bh / FEATURE_SIZE).max(ys + 1).min(y1 + 1);
        for i in 0..FEATURE_SIZE {
            let xs = x0 + i * bw / FEATURE_SIZE;
            let xe = (x0 + (i + 1) * bw / FEATURE_SIZE).max(xs + 1).min(x1 + 1);
            let (mut on, mut total) = (0usize, 0usize);
            for y in ys..ye {
                for x in xs..xe {
                    total += 1;
                    on += usize::from(mask[y * w + x]);
                }
            }
            out[j * FEATURE_SIZE + i] = total > 0 && on * 2 >= total;
        }
    }
    out
}

/// 各セルから最も近い前景セルまでの距離(3-4 チャンファー。単位はセル)。前景が無ければ [`FAR`]。
fn distance_transform(mask: &[bool; FEATURE_AREA]) -> [f32; FEATURE_AREA] {
    const INF: i32 = i32::MAX / 4;
    let n = FEATURE_SIZE;
    let mut d = [INF; FEATURE_AREA];
    for i in 0..FEATURE_AREA {
        if mask[i] {
            d[i] = 0;
        }
    }
    // 前向き
    for y in 0..n {
        for x in 0..n {
            let mut v = d[y * n + x];
            if x > 0 {
                v = v.min(d[y * n + x - 1] + 3);
            }
            if y > 0 {
                v = v.min(d[(y - 1) * n + x] + 3);
                if x > 0 {
                    v = v.min(d[(y - 1) * n + x - 1] + 4);
                }
                if x + 1 < n {
                    v = v.min(d[(y - 1) * n + x + 1] + 4);
                }
            }
            d[y * n + x] = v;
        }
    }
    // 後ろ向き
    for y in (0..n).rev() {
        for x in (0..n).rev() {
            let mut v = d[y * n + x];
            if x + 1 < n {
                v = v.min(d[y * n + x + 1] + 3);
            }
            if y + 1 < n {
                v = v.min(d[(y + 1) * n + x] + 3);
                if x + 1 < n {
                    v = v.min(d[(y + 1) * n + x + 1] + 4);
                }
                if x > 0 {
                    v = v.min(d[(y + 1) * n + x - 1] + 4);
                }
            }
            d[y * n + x] = v;
        }
    }
    let mut out = [FAR; FEATURE_AREA];
    if mask.iter().any(|&m| m) {
        for i in 0..FEATURE_AREA {
            out[i] = d[i] as f32 / 3.0;
        }
    }
    out
}

/// 対称チャンファー距離: `a` の前景から `b` への平均距離と、`b` の前景から `a` への平均距離の平均。
fn symmetric_chamfer(
    a: &[bool; FEATURE_AREA],
    dt_a: &[f32; FEATURE_AREA],
    b: &[bool; FEATURE_AREA],
    dt_b: &[f32; FEATURE_AREA],
) -> f32 {
    let mean_to = |from: &[bool; FEATURE_AREA], dt_to: &[f32; FEATURE_AREA]| -> f32 {
        let (mut sum, mut n) = (0.0_f32, 0usize);
        for i in 0..FEATURE_AREA {
            if from[i] {
                sum += dt_to[i];
                n += 1;
            }
        }
        if n == 0 {
            FAR
        } else {
            sum / n as f32
        }
    };
    f32::midpoint(mean_to(a, dt_b), mean_to(b, dt_a))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 9×9 の形(`#` が前景)。形どうしは互いに十分違う。
    const SHAPE_A: [&str; 9] = [
        "....#....",
        "...#.#...",
        "..#...#..",
        ".#.....#.",
        ".#######.",
        ".#.....#.",
        ".#.....#.",
        ".#.....#.",
        ".#.....#.",
    ];
    const SHAPE_BOX: [&str; 9] = [
        "#########",
        "#.......#",
        "#.......#",
        "#.......#",
        "#.......#",
        "#.......#",
        "#.......#",
        "#.......#",
        "#########",
    ];
    const SHAPE_CROSS: [&str; 9] = [
        "....#....",
        "....#....",
        "....#....",
        "....#....",
        "#########",
        "....#....",
        "....#....",
        "....#....",
        "....#....",
    ];
    const SHAPE_E: [&str; 9] = [
        "#########",
        "#........",
        "#........",
        "#........",
        "#######..",
        "#........",
        "#........",
        "#........",
        "#########",
    ];
    const SHAPE_SLASH: [&str; 9] = [
        "........#",
        ".......#.",
        "......#..",
        ".....#...",
        "....#....",
        "...#.....",
        "..#......",
        ".#.......",
        "#........",
    ];
    const SHAPES: [&[&str; 9]; 5] = [&SHAPE_A, &SHAPE_BOX, &SHAPE_CROSS, &SHAPE_E, &SHAPE_SLASH];
    const SIZES: [(usize, usize); 4] = [(32, 24), (40, 30), (48, 36), (64, 48)];

    /// 形を中央の 60% × 70% に描いた画像。`dark` は暗い背景に明るい字形(ダークテーマ)、`framed` は上下と左右に枠線を引く。
    fn render(shape: &[&str; 9], w: usize, h: usize, dark: bool, framed: bool) -> GlyphGray {
        let (fg, bg) = if dark { (230u8, 40u8) } else { (30u8, 225u8) };
        let mut px = vec![bg; w * h];
        for y in 0..h {
            for x in 0..w {
                let u = (x as f32 + 0.5) / w as f32;
                let v = (y as f32 + 0.5) / h as f32;
                if (0.2..0.8).contains(&u) && (0.15..0.85).contains(&v) {
                    let cx = (((u - 0.2) / 0.6) * 9.0) as usize;
                    let cy = (((v - 0.15) / 0.7) * 9.0) as usize;
                    if shape[cy.min(8)].as_bytes()[cx.min(8)] == b'#' {
                        px[y * w + x] = fg;
                    }
                }
            }
        }
        if framed {
            for x in 0..w {
                px[x] = fg;
                px[(h - 1) * w + x] = fg;
            }
            for y in 0..h {
                px[y * w] = fg;
                px[y * w + w - 1] = fg;
            }
        }
        GlyphGray::new(w, h, px).expect("寸法が合う")
    }

    fn feature(shape: &[&str; 9], w: usize, h: usize, dark: bool, framed: bool) -> GlyphFeature {
        preprocess(&render(shape, w, h, dark, framed)).expect("前景がある")
    }

    #[test]
    fn gray_requires_matching_length_and_minimum_size() {
        assert!(GlyphGray::new(32, 24, vec![0; 32 * 24]).is_some());
        assert!(GlyphGray::new(32, 24, vec![0; 10]).is_none());
        assert!(GlyphGray::new(4, 24, vec![0; 96]).is_none());
        assert!(GlyphGray::new(32, 4, vec![0; 128]).is_none());
    }

    #[test]
    fn uniform_or_nearly_uniform_image_is_empty() {
        let flat = GlyphGray::new(32, 24, vec![120; 32 * 24]).unwrap();
        assert_eq!(preprocess(&flat), Err(GlyphMiss::EmptyGlyph));
        // 背景とのわずかな差(ノイズ)だけの画像も空白。閉と取り違えない。
        let mut noisy = vec![120u8; 32 * 24];
        for (i, p) in noisy.iter_mut().enumerate() {
            *p = 120 + (i % 3) as u8;
        }
        let noisy = GlyphGray::new(32, 24, noisy).unwrap();
        assert_eq!(preprocess(&noisy), Err(GlyphMiss::EmptyGlyph));
    }

    #[test]
    fn frame_only_image_is_empty() {
        // 枠線だけ(字形なし)。枠線を除くと何も残らない。
        let mut px = vec![225u8; 32 * 24];
        for x in 0..32 {
            px[x] = 30;
            px[23 * 32 + x] = 30;
        }
        for y in 0..24 {
            px[y * 32] = 30;
            px[y * 32 + 31] = 30;
        }
        let img = GlyphGray::new(32, 24, px).unwrap();
        assert_eq!(preprocess(&img), Err(GlyphMiss::EmptyGlyph));
    }

    #[test]
    fn same_shape_is_closer_across_scales_themes_and_frames_than_different_shapes() {
        let mut feats: Vec<(usize, GlyphFeature)> = Vec::new();
        for (si, shape) in SHAPES.iter().enumerate() {
            for &(w, h) in &SIZES {
                for dark in [false, true] {
                    for framed in [false, true] {
                        feats.push((si, feature(shape, w, h, dark, framed)));
                    }
                }
            }
        }
        let (mut max_intra, mut min_inter) = (0.0_f32, f32::MAX);
        for (i, (si, a)) in feats.iter().enumerate() {
            for (sj, b) in feats.iter().skip(i + 1) {
                let d = a.distance(b);
                if si == sj {
                    max_intra = max_intra.max(d);
                } else {
                    min_inter = min_inter.min(d);
                }
            }
        }
        assert!(
            max_intra < min_inter,
            "同じ形の最大距離 {max_intra} が、別の形の最小距離 {min_inter} 未満であること"
        );
    }

    #[test]
    fn distance_is_symmetric_and_zero_for_the_same_feature() {
        let a = feature(&SHAPE_A, 40, 30, false, false);
        let b = feature(&SHAPE_BOX, 40, 30, false, false);
        assert!(a.distance(&a) < 1.0e-6);
        assert!((a.distance(&b) - b.distance(&a)).abs() < 1.0e-6);
        assert!(a.dots() >= MIN_DOTS);
        assert!(a.aspect() > 0.0);
    }

    fn table_with_one_sample_per_shape(origin: SampleOrigin) -> GlyphTable {
        let classes = [
            GlyphClass::Hiragana,
            GlyphClass::FullKatakana,
            GlyphClass::FullAlnum,
            GlyphClass::HalfKatakana,
            GlyphClass::Closed,
        ];
        let mut t = GlyphTable::new();
        for (shape, class) in SHAPES.iter().zip(classes) {
            let outcome = t.add(GlyphSample {
                class,
                feature: feature(shape, 32, 24, false, false),
                origin,
            });
            assert_eq!(outcome, AddOutcome::Added);
        }
        t
    }

    #[test]
    fn classify_picks_the_right_class_across_scales_and_themes() {
        let table = table_with_one_sample_per_shape(SampleOrigin::SameEnv);
        let expected = [
            GlyphClass::Hiragana,
            GlyphClass::FullKatakana,
            GlyphClass::FullAlnum,
            GlyphClass::HalfKatakana,
            GlyphClass::Closed,
        ];
        for (shape, want) in SHAPES.iter().zip(expected) {
            for &(w, h) in &SIZES {
                for dark in [false, true] {
                    let img = render(shape, w, h, dark, true);
                    match classify_image(&table, &Thresholds::default(), &img) {
                        GlyphReading::Hit(hit) => {
                            assert_eq!(hit.class, want, "{w}x{h} dark={dark}")
                        }
                        GlyphReading::Miss(m) => panic!("{w}x{h} dark={dark}: Miss({m:?})"),
                    }
                }
            }
        }
    }

    #[test]
    fn classify_misses_on_empty_table_and_empty_image() {
        let img = render(&SHAPE_A, 32, 24, false, false);
        assert_eq!(
            classify_image(&GlyphTable::new(), &Thresholds::default(), &img),
            GlyphReading::Miss(GlyphMiss::EmptyTable)
        );
        let table = table_with_one_sample_per_shape(SampleOrigin::SameEnv);
        let flat = GlyphGray::new(32, 24, vec![100; 32 * 24]).unwrap();
        assert_eq!(
            classify_image(&table, &Thresholds::default(), &flat),
            GlyphReading::Miss(GlyphMiss::EmptyGlyph)
        );
    }

    #[test]
    fn classify_misses_when_nothing_is_close_enough() {
        let table = table_with_one_sample_per_shape(SampleOrigin::SameEnv);
        let strict = Thresholds {
            max_distance: -1.0,
            ..Thresholds::default()
        };
        // 距離は 0 以上なので、負の上限ではどの見本にも近くない。
        let img = render(&SHAPE_A, 64, 48, true, false);
        assert_eq!(
            classify_image(&table, &strict, &img),
            GlyphReading::Miss(GlyphMiss::TooFar)
        );
    }

    #[test]
    fn classify_is_ambiguous_when_the_ratio_is_below_the_floor() {
        // 表の 2 クラス(A と BOX)のどちらとも同じではない形(E)を読むと、最近傍と次点の距離比は有限。
        // 距離比の下限を極端に高くすれば Ambiguous、上限距離を十分大きくして TooFar を避ける。
        let mut table = GlyphTable::new();
        for (class, shape) in [
            (GlyphClass::Closed, &SHAPE_A),
            (GlyphClass::FullAlnum, &SHAPE_BOX),
        ] {
            assert_eq!(
                table.add(GlyphSample {
                    class,
                    feature: feature(shape, 32, 24, false, false),
                    origin: SampleOrigin::SameEnv
                }),
                AddOutcome::Added
            );
        }
        let probe = feature(&SHAPE_E, 32, 24, false, false);
        let strict = Thresholds {
            max_distance: 1.0e6,
            min_ratio_same_env: 1.0e6,
            min_ratio_common: 1.0e6,
        };
        assert_eq!(
            classify_glyph(&table, &strict, &probe),
            GlyphReading::Miss(GlyphMiss::Ambiguous)
        );
        let lenient = Thresholds {
            max_distance: 1.0e6,
            min_ratio_same_env: 0.0,
            min_ratio_common: 0.0,
        };
        assert!(matches!(
            classify_glyph(&table, &lenient, &probe),
            GlyphReading::Hit(_)
        ));
    }

    #[test]
    fn common_samples_need_a_higher_ratio_than_same_env_samples() {
        // 同じ最近傍・次点でも、共通の初期見本が最近傍なら距離比の下限が高い。
        let near = GlyphSample {
            class: GlyphClass::Hiragana,
            feature: feature(&SHAPE_A, 32, 24, false, false),
            origin: SampleOrigin::Common,
        };
        let mut table = GlyphTable::new();
        assert_eq!(table.add(near), AddOutcome::Added);
        let other = GlyphSample {
            class: GlyphClass::FullAlnum,
            feature: feature(&SHAPE_BOX, 32, 24, false, false),
            origin: SampleOrigin::SameEnv,
        };
        assert_eq!(table.add(other), AddOutcome::Added);
        let mut shifted = SHAPE_A;
        shifted[8] = ".........";
        let probe = feature(&shifted, 64, 48, true, false);
        let lenient = Thresholds {
            min_ratio_common: 0.0,
            ..Thresholds::default()
        };
        let strict = Thresholds {
            min_ratio_common: 1.0e6,
            ..Thresholds::default()
        };
        assert!(matches!(
            classify_glyph(&table, &lenient, &probe),
            GlyphReading::Hit(_)
        ));
        assert_eq!(
            classify_glyph(&table, &strict, &probe),
            GlyphReading::Miss(GlyphMiss::Ambiguous)
        );
    }

    #[test]
    fn table_rejects_duplicates_and_conflicts() {
        let mut table = GlyphTable::new();
        let sample = |class, shape: &[&str; 9], w, h| GlyphSample {
            class,
            feature: feature(shape, w, h, false, false),
            origin: SampleOrigin::SameEnv,
        };
        assert_eq!(
            table.add(sample(GlyphClass::Hiragana, &SHAPE_A, 32, 24)),
            AddOutcome::Added
        );
        // 同じクラス・同じ形(同じ画像)は重複。
        assert_eq!(
            table.add(sample(GlyphClass::Hiragana, &SHAPE_A, 32, 24)),
            AddOutcome::Duplicate
        );
        // 別クラスに同じ形は衝突。
        assert_eq!(
            table.add(sample(GlyphClass::FullAlnum, &SHAPE_A, 32, 24)),
            AddOutcome::Conflict
        );
        assert_eq!(table.len(), 1);
    }

    fn cand(name: &str, rect: (i32, i32, i32, i32)) -> IndicatorCandidate {
        IndicatorCandidate {
            name: name.to_owned(),
            rect,
        }
    }

    #[test]
    fn select_indicator_picks_the_single_line_one_among_two_real_machine_candidates() {
        // 実機(日本語 Windows)の 2 要素: 「あ」のボタンは 1 行、IME 自身のアイコンは複数行。順序に依らない。
        let hint = cand(
            "トレイ入力インジケーター 右クリックで IME のオプションを表示します",
            (1726, 1256, 1758, 1280),
        );
        let info = cand("トレイ入力インジケーター 日本語\nGoogle 日本語入力\n\n入力方式を切り替えるには、Windows キー + Space キーを押します。", (1758, 1256, 1802, 1280));
        let other = cand("ネットワーク wr9500n-a", (1802, 1256, 1830, 1280));
        assert_eq!(
            select_indicator(&[hint.clone(), info.clone(), other.clone()]),
            Ok(0)
        );
        assert_eq!(select_indicator(&[other, info, hint]), Ok(2));
    }

    #[test]
    fn select_indicator_handles_english_and_single_candidates() {
        let en = cand(
            "Tray Input Indicator Right click to show IME options",
            (778, 720, 810, 768),
        );
        assert_eq!(select_indicator(&[en]), Ok(0));
    }

    #[test]
    fn select_indicator_misses_on_none_ambiguous_and_bad_rect() {
        let other = cand("ネットワーク", (0, 0, 32, 24));
        assert_eq!(select_indicator(&[other]), Err(SelectMiss::NotFound));
        assert_eq!(select_indicator(&[]), Err(SelectMiss::NotFound));
        // 同じく 1 行の要素が 2 つ → 絞れない。
        let a = cand("Tray Input Indicator A", (0, 0, 32, 24));
        let b = cand("Tray Input Indicator B", (32, 0, 64, 24));
        assert_eq!(select_indicator(&[a, b]), Err(SelectMiss::Ambiguous));
        // 矩形が空(画面外・自動非表示)。
        let tiny = cand("Tray Input Indicator", (0, 0, 0, 0));
        assert_eq!(select_indicator(&[tiny]), Err(SelectMiss::BadRect));
    }

    #[test]
    fn probe_outcome_round_trips_through_pack() {
        let cases = [
            ProbeOutcome {
                result: Ok(()),
                width: 32,
                height: 24,
                dots: 1234,
                read_ms: 57,
                dark: false,
            },
            ProbeOutcome {
                result: Err(ProbeMiss::EmptyGlyph),
                width: 64,
                height: 48,
                dots: 7,
                read_ms: 187,
                dark: false,
            },
            ProbeOutcome {
                result: Err(ProbeMiss::NoIndicator),
                width: 0,
                height: 0,
                dots: 0,
                read_ms: 3,
                dark: false,
            },
            ProbeOutcome {
                result: Err(ProbeMiss::CaptureFailed),
                width: 1023,
                height: 1023,
                dots: u16::MAX,
                read_ms: u16::MAX,
                dark: false,
            },
        ];
        for c in cases {
            let (w, l) = c.pack();
            assert_eq!(ProbeOutcome::unpack(w, l), Some(c));
        }
        // 背景が暗いことも往復する。
        let dark = ProbeOutcome {
            result: Ok(()),
            width: 32,
            height: 24,
            dots: 5,
            read_ms: 1,
            dark: true,
        };
        let (w, l) = dark.pack();
        assert_eq!(ProbeOutcome::unpack(w, l), Some(dark));
        // 未知の符号は壊れたメッセージ。
        assert_eq!(ProbeOutcome::unpack(0xF, 0), None);
    }

    #[test]
    fn probe_outcome_record_carries_context_without_pixels() {
        let o = ProbeOutcome {
            result: Err(ProbeMiss::EmptyGlyph),
            width: 32,
            height: 24,
            dots: 3,
            read_ms: 40,
            dark: false,
        };
        let r = o.to_record("imm32-unavailable", Some(true), 900);
        assert_eq!((r.verdict, r.miss_reason), ("miss", Some("empty-glyph")));
        assert_eq!(
            (r.width, r.height, r.dots, r.read_ms, r.delay_ms),
            (32, 24, 3, 40, 900)
        );
        assert_eq!(
            (r.profile, r.belief_open),
            ("imm32-unavailable", Some(true))
        );
        let ok = ProbeOutcome {
            result: Ok(()),
            width: 40,
            height: 30,
            dots: 900,
            read_ms: 55,
            dark: false,
        }
        .to_record("standard", None, 2500);
        assert_eq!((ok.verdict, ok.miss_reason), ("captured", None));
    }

    #[test]
    fn belief_label_uses_only_confirmed_states() {
        use awase::engine::mode_state::{AssumedReason, InputModeState as M};
        assert_eq!(belief_label(false, M::Unknown), Some(BeliefLabel::Closed));
        assert_eq!(
            belief_label(false, M::ObservedEisu),
            Some(BeliefLabel::Closed)
        );
        assert_eq!(
            belief_label(true, M::ObservedRomaji),
            Some(BeliefLabel::Kana)
        );
        assert_eq!(belief_label(true, M::ObservedKana), Some(BeliefLabel::Kana));
        assert_eq!(belief_label(true, M::ObservedEisu), Some(BeliefLabel::Eisu));
        assert_eq!(belief_label(true, M::Unknown), None);
        assert_eq!(
            belief_label(
                true,
                M::AssumedRomaji {
                    reason: AssumedReason::ImmBridgeBroken
                }
            ),
            None
        );
        // 代表のクラスとの往復。
        for l in [BeliefLabel::Closed, BeliefLabel::Kana, BeliefLabel::Eisu] {
            assert_eq!(BeliefLabel::from_class(l.class()), l);
        }
        assert_eq!(
            BeliefLabel::from_class(GlyphClass::FullKatakana),
            BeliefLabel::Kana
        );
        assert_eq!(
            BeliefLabel::from_class(GlyphClass::HalfKatakana),
            BeliefLabel::Kana
        );
        assert_eq!(
            BeliefLabel::from_class(GlyphClass::FullAlnum),
            BeliefLabel::Eisu
        );
    }

    #[test]
    fn collect_label_needs_a_readable_window_a_previous_capture_same_label_and_a_stable_glyph() {
        let a = feature(&SHAPE_A, 32, 24, false, false);
        let a_again = feature(&SHAPE_A, 32, 24, false, false);
        let other = feature(&SHAPE_BOX, 32, 24, false, false);
        let cap = |f, label| Capture { feature: f, label };
        let k = Some(BeliefLabel::Kana);
        // 条件がそろえば、そのラベルで集める。
        assert_eq!(collect_label(Some(cap(&a, k)), cap(&a_again, k), true), k);
        // 読めない窓・前回なし・ラベルなし・ラベルが違う・字形が違う(落ち着いていない)は集めない。
        assert_eq!(
            collect_label(Some(cap(&a, k)), cap(&a_again, k), false),
            None
        );
        assert_eq!(collect_label(None, cap(&a_again, k), true), None);
        assert_eq!(
            collect_label(Some(cap(&a, k)), cap(&a_again, None), true),
            None
        );
        assert_eq!(
            collect_label(Some(cap(&a, None)), cap(&a_again, k), true),
            None
        );
        assert_eq!(
            collect_label(
                Some(cap(&a, k)),
                cap(&a_again, Some(BeliefLabel::Eisu)),
                true
            ),
            None
        );
        assert_eq!(collect_label(Some(cap(&a, k)), cap(&other, k), true), None);
    }

    #[test]
    fn match_feature_reports_the_coarse_label_or_the_miss_reason() {
        let mut table = GlyphTable::new();
        let sample = |label: BeliefLabel, shape: &[&str; 9]| GlyphSample {
            class: label.class(),
            feature: feature(shape, 32, 24, false, false),
            origin: SampleOrigin::SameEnv,
        };
        assert_eq!(
            table.add(sample(BeliefLabel::Kana, &SHAPE_A)),
            AddOutcome::Added
        );
        assert_eq!(
            table.add(sample(BeliefLabel::Closed, &SHAPE_BOX)),
            AddOutcome::Added
        );
        let probe = feature(&SHAPE_A, 64, 48, true, true);
        let m = match_feature(&table, &Thresholds::default(), &probe);
        assert_eq!(m.reading, Some(BeliefLabel::Kana));
        assert!(m.miss.is_none() && m.distance.is_some() && m.ratio.is_some());
        let none = match_feature(&GlyphTable::new(), &Thresholds::default(), &probe);
        assert_eq!(
            (none.reading, none.miss),
            (None, Some(GlyphMiss::EmptyTable))
        );
    }

    #[test]
    fn apply_match_fills_the_record_without_touching_the_capture_fields() {
        let o = ProbeOutcome {
            result: Ok(()),
            width: 32,
            height: 24,
            dots: 900,
            read_ms: 50,
            dark: false,
        };
        let mut r = o.to_record("standard", Some(true), 900);
        let hit = MatchResult {
            reading: Some(BeliefLabel::Kana),
            class: Some(GlyphClass::FullKatakana),
            origin: Some(SampleOrigin::Common),
            miss: None,
            distance: Some(0.123),
            ratio: Some(f32::INFINITY),
        };
        apply_match(&mut r, &hit, Some(BeliefLabel::Kana), true, 3);
        assert_eq!(
            (r.reading, r.match_miss, r.agree),
            (Some("kana"), None, Some(true))
        );
        assert_eq!((r.distance_x100, r.ratio_x100), (Some(12), Some(9999)));
        assert_eq!(
            (r.belief_label, r.collected, r.table_size),
            (Some("kana"), true, 3)
        );
        assert_eq!((r.verdict, r.width, r.dots), ("captured", 32, 900));
        // 一致しない読み・照合の Miss・belief が無いとき。
        apply_match(&mut r, &hit, Some(BeliefLabel::Eisu), false, 3);
        assert_eq!(r.agree, Some(false));
        let miss = MatchResult {
            reading: None,
            class: None,
            origin: None,
            miss: Some(GlyphMiss::TooFar),
            distance: None,
            ratio: None,
        };
        apply_match(&mut r, &miss, Some(BeliefLabel::Kana), false, 0);
        assert_eq!(
            (r.reading, r.match_miss, r.agree, r.distance_x100),
            (None, Some("too-far"), None, None)
        );
        apply_match(&mut r, &hit, None, false, 1);
        assert_eq!(r.agree, None);
        // 開閉の軸: かなの読み × belief 開 = 一致、閉の読み × belief 開 = 不一致、belief 不明 = 数えない。
        assert_eq!(r.agree_open, Some(true));
        let closed = MatchResult {
            reading: Some(BeliefLabel::Closed),
            class: Some(GlyphClass::Closed),
            origin: Some(SampleOrigin::Common),
            miss: None,
            distance: Some(0.0),
            ratio: Some(1.0),
        };
        apply_match(&mut r, &closed, None, false, 1);
        assert_eq!(r.agree_open, Some(false));
        r.belief_open = Some(false);
        apply_match(&mut r, &closed, None, false, 1);
        assert_eq!(r.agree_open, Some(true));
        r.belief_open = None;
        apply_match(&mut r, &closed, None, false, 1);
        assert_eq!(r.agree_open, None);
    }

    #[test]
    fn table_caps_samples_per_class() {
        let mut table = GlyphTable::new();
        // 距離が離れた別の形を 7 つ同じクラスに入れようとすると、6 つで Full になる。
        let shapes = [&SHAPE_A, &SHAPE_BOX, &SHAPE_CROSS, &SHAPE_E, &SHAPE_SLASH];
        let mut added = 0;
        let mut full = false;
        for round in 0..2 {
            for shape in shapes {
                let mut s = *shape;
                // 2 周目は最後の行を落として別の見本にする。
                if round == 1 {
                    s[8] = ".........";
                }
                match table.add(GlyphSample {
                    class: GlyphClass::Hiragana,
                    feature: feature(&s, 32, 24, false, false),
                    origin: SampleOrigin::SameEnv,
                }) {
                    AddOutcome::Added => added += 1,
                    AddOutcome::Full => full = true,
                    AddOutcome::Duplicate | AddOutcome::Conflict => {}
                }
            }
        }
        assert!(added <= 6 && table.len() <= 6);
        assert!(
            full || added < 6,
            "上限に達したら Full、達しないなら重複で弾かれている"
        );
    }

    fn hit(label: BeliefLabel) -> LabelHit {
        LabelHit {
            label,
            class: label.class(),
            distance: 0.0,
            ratio: f32::INFINITY,
            origin: SampleOrigin::Common,
        }
    }

    /// ADR-273 D4: モード系の打鍵から柵（`GLYPH_KEY_FENCE_MS`）が経つまで、字形は報告できない。170ms（`KEY_EFFECT_SETTLE_MS`）では足りない。
    #[test]
    fn the_fence_blocks_glyphs_read_soon_after_a_mode_key_and_the_settle_time_is_not_enough() {
        let ctx = |ms| FenceCtx {
            since_last_mode_key_ms: ms,
            focus_gen_unchanged: true,
            foreground_in_sync: true,
        };
        let h = hit(BeliefLabel::Kana);
        // 打鍵が無い → 通る。
        assert!(fence_glyph(h, ctx(None)).is_ok());
        // 柵の手前 → 通らない。予測の settle（170ms）を過ぎていても通らない。
        let settle = crate::tuning::KEY_EFFECT_SETTLE_MS;
        assert_eq!(
            fence_glyph(h, ctx(Some(settle))),
            Err(FenceMiss::TooSoonAfterModeKey)
        );
        assert_eq!(
            fence_glyph(h, ctx(Some(crate::tuning::GLYPH_KEY_FENCE_MS - 1))),
            Err(FenceMiss::TooSoonAfterModeKey)
        );
        // 柵の境界ちょうどから通る。
        let ok = fence_glyph(h, ctx(Some(crate::tuning::GLYPH_KEY_FENCE_MS)));
        assert_eq!(ok.map(|f| f.label()), Ok(BeliefLabel::Kana));
        // 柵は ADR-261 の実測最大の遅れ(0.68 秒)より長い。
        assert!(crate::tuning::GLYPH_KEY_FENCE_MS > 680);
        assert!(crate::tuning::GLYPH_KEY_FENCE_MS > settle);
    }

    #[test]
    fn the_fence_blocks_a_moved_focus_and_an_out_of_sync_foreground() {
        let base = FenceCtx {
            since_last_mode_key_ms: None,
            focus_gen_unchanged: true,
            foreground_in_sync: true,
        };
        let h = hit(BeliefLabel::Closed);
        assert_eq!(
            fence_glyph(
                h,
                FenceCtx {
                    focus_gen_unchanged: false,
                    ..base
                }
            ),
            Err(FenceMiss::FocusMoved)
        );
        assert_eq!(
            fence_glyph(
                h,
                FenceCtx {
                    foreground_in_sync: false,
                    ..base
                }
            ),
            Err(FenceMiss::ForegroundOutOfSync)
        );
        assert!(fence_glyph(h, base).is_ok());
    }

    #[test]
    fn labels_are_stable() {
        assert_eq!(GlyphClass::Closed.label(), "closed");
        assert_eq!(GlyphClass::Hiragana.label(), "hiragana");
        assert_eq!(GlyphMiss::EmptyGlyph.label(), "empty-glyph");
        assert_eq!(GlyphMiss::Ambiguous.label(), "ambiguous");
    }

    fn scope(standard: bool, demoted: bool) -> GlyphScopeFacts {
        GlyphScopeFacts {
            effective_standard: standard,
            demoted_by_learning: demoted,
            tsf_native: false,
            input_relay: false,
            mozc_block: false,
        }
    }

    /// ADR-273 D3・Q2: 窓の述語の表。`all` は `unobserved` の対象（降格した窓）を含む（Opus E1）。
    #[test]
    fn glyph_window_scope_table() {
        use awase::config::EvidenceLevel::{All, Observe, Off, Unobserved};
        // (level, 実効 Standard, 降格, 期待)
        let table = [
            (Off, true, false, false),
            (Observe, false, true, false),
            (Unobserved, true, false, false), // 読める窓は unobserved では対象外
            (Unobserved, false, true, true),  // 降格した窓（実効は Imm32Unavailable）
            (Unobserved, false, false, false), // 静的な Imm32Unavailable
            (All, true, false, true),         // 静的 Standard
            (All, false, true, true),         // 降格した窓を含む
            (All, false, false, false),       // 静的 Imm32Unavailable は除外
        ];
        for (level, standard, demoted, want) in table {
            assert_eq!(
                glyph_window_in_scope(level, scope(standard, demoted)),
                want,
                "{level:?} standard={standard} demoted={demoted}"
            );
        }
    }

    /// 入力中継・TSF ネイティブ・mozc の共有メモリがある窓は、どの段階でも除外する（R1）。
    #[test]
    fn glyph_window_scope_excludes_relay_tsf_and_mozc() {
        use awase::config::EvidenceLevel::{All, Unobserved};
        for level in [Unobserved, All] {
            let base = scope(true, true);
            assert!(glyph_window_in_scope(level, base));
            for f in [
                GlyphScopeFacts {
                    input_relay: true,
                    ..base
                },
                GlyphScopeFacts {
                    tsf_native: true,
                    ..base
                },
                GlyphScopeFacts {
                    mozc_block: true,
                    ..base
                },
            ] {
                assert!(!glyph_window_in_scope(level, f), "{level:?} {f:?}");
            }
        }
        assert_eq!(glyph_scope_label(scope(false, true)), "unobserved");
        assert_eq!(glyph_scope_label(scope(true, false)), "all");
        assert_eq!(glyph_scope_label(scope(false, false)), "none");
    }
}
