//! ADR-261 決定 2・3: 字形の見本の**永続化の形式**と**環境指紋**（純粋。ファイル I/O は殻が行う）。
//!
//! 学習表（`cache.toml` のキー効果表）とは**別ファイル**（`glyph_samples.json`）にする（学習表の版を上げると、学習済みの
//! 利用者全員のキー効果予測が内蔵表へ戻るため）。保存するのは、**同じ環境で受け身に集めた見本**だけ（内蔵の共通の見本は
//! 含まない）。見本の特徴は 2 通りの 20×20 の 2 値画像（16 進）・縦横比・前景の点数で、**元の画素は保存しない**。
//!
//! ## 環境指紋
//!
//! 表を作ったときと違う環境では使わない（ADR-261 決定 3）。[`EnvFingerprint`] = 矩形の大きさ（DPI に比例）・背景の明暗
//! （ライト/ダーク）・IME の種類・取り込み方式の版（[`CAPTURE_VERSION`]。前処理を変えたら上げる。全部の表が無効になる）。
//! OS のビルド番号は含めていない（取得経路が未整備。Windows のアップデートで字形が変わった場合は、実行時の
//! 継続的な突き合わせによる失効〔次の段階〕で捉える）。
//!
//! 壊れた・版の違うファイルは**空として読む**（見本は集め直せる。ファイルを壊さずに捨てるだけ）。

use serde::{Deserialize, Serialize};

use super::indicator_glyph::{GlyphClass, GlyphFeature, GlyphSample, GlyphTable, SampleOrigin};

/// 取り込み方式の版。前処理（`preprocess`）・矩形の取り方を変えたら上げる（全部の保存済みの表が無効になる）。
pub const CAPTURE_VERSION: u32 = 1;
/// ファイル形式の版。
pub const FILE_VERSION: u32 = 1;
/// 1 つの環境が持つ見本の数の上限（壊れた・巨大なファイルを読まない保険）。
const MAX_SAMPLES_PER_ENV: usize = 64;
/// 保存する環境の数の上限（古いものから捨てる）。
const MAX_ENVS: usize = 16;
const FEATURE_AREA: usize = 400;

/// 環境指紋。表を作ったときと違う環境では、その表を使わない。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EnvFingerprint {
    /// 取り込んだ矩形の大きさ（物理ピクセル。DPI に比例する: 100%=32×24、125%=40×30、150%=48×36、200%=64×48）
    pub width: u16,
    pub height: u16,
    /// 背景が暗いか（ダークテーマ）
    pub dark: bool,
    /// IME の種類（`gji` / `ms-ime` / 不明は空）
    pub ime: String,
    /// 取り込み方式の版（[`CAPTURE_VERSION`]）
    pub capture_version: u32,
}

impl EnvFingerprint {
    /// 現在の取り込み方式の版で作る。
    #[must_use]
    pub fn new(width: u16, height: u16, dark: bool, ime: &str) -> Self {
        Self {
            width,
            height,
            dark,
            ime: ime.to_owned(),
            capture_version: CAPTURE_VERSION,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredSample {
    /// [`GlyphClass::label`]
    class: String,
    /// 2 通りの 20×20 の 2 値画像（行優先、各 400 ビット → 100 バイト → 16 進 200 文字）
    masks: [String; 2],
    /// 縦横比 × 1000（整数）
    aspect_milli: u32,
    dots: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredEnv {
    fingerprint: EnvFingerprint,
    samples: Vec<StoredSample>,
}

/// 保存ファイル全体。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GlyphSampleFile {
    version: u32,
    envs: Vec<StoredEnv>,
}

fn mask_to_hex(mask: &[bool; FEATURE_AREA]) -> String {
    let mut out = String::with_capacity(FEATURE_AREA / 2);
    for chunk in mask.chunks(4) {
        let nibble = chunk
            .iter()
            .enumerate()
            .fold(0u8, |acc, (i, &b)| acc | (u8::from(b) << i));
        out.push(char::from_digit(u32::from(nibble), 16).unwrap_or('0'));
    }
    out
}

fn mask_from_hex(hex: &str) -> Option<[bool; FEATURE_AREA]> {
    if hex.len() != FEATURE_AREA / 4 {
        return None;
    }
    let mut mask = [false; FEATURE_AREA];
    for (i, c) in hex.chars().enumerate() {
        let nibble = c.to_digit(16)?;
        for bit in 0..4 {
            mask[i * 4 + bit] = (nibble >> bit) & 1 == 1;
        }
    }
    Some(mask)
}

// 縦横比は 0 以上に丸めてから整数にする（符号は失われない）。
#[allow(clippy::cast_sign_loss)]
fn to_stored(sample: &GlyphSample) -> StoredSample {
    let (masks, aspect, dots) = sample.feature.to_parts();
    StoredSample {
        class: sample.class.label().to_owned(),
        masks: [mask_to_hex(&masks[0]), mask_to_hex(&masks[1])],
        aspect_milli: (aspect * 1000.0).round().clamp(0.0, 1.0e7) as u32,
        dots: u32::try_from(dots).unwrap_or(u32::MAX),
    }
}

// 縦横比 × 1000 は 1.0e7 以下（`to_stored` で丸める）なので、`f32` への変換で精度は失われない。
#[allow(clippy::cast_precision_loss)]
fn from_stored(s: &StoredSample) -> Option<GlyphSample> {
    let class = GlyphClass::from_label(&s.class)?;
    let masks = [mask_from_hex(&s.masks[0])?, mask_from_hex(&s.masks[1])?];
    let feature = GlyphFeature::from_parts(
        masks,
        s.aspect_milli as f32 / 1000.0,
        usize::try_from(s.dots).ok()?,
    )?;
    Some(GlyphSample {
        class,
        feature,
        origin: SampleOrigin::SameEnv,
    })
}

impl GlyphSampleFile {
    /// JSON から読む。**壊れている・版が違うときは空**（見本は集め直せる）。
    #[must_use]
    pub fn from_json(json: &str) -> Self {
        match serde_json::from_str::<Self>(json) {
            Ok(f) if f.version == FILE_VERSION => f,
            _ => Self::default(),
        }
    }

    /// JSON にする。
    #[must_use]
    pub fn to_json(&self) -> String {
        let mut f = self.clone();
        f.version = FILE_VERSION;
        serde_json::to_string(&f).unwrap_or_default()
    }

    /// この環境の保存済みの見本（読めない行は捨てる。取り込み方式の版が違う環境は含めない）。
    #[must_use]
    pub fn samples_for(&self, fingerprint: &EnvFingerprint) -> Vec<GlyphSample> {
        if fingerprint.capture_version != CAPTURE_VERSION {
            return Vec::new();
        }
        self.envs
            .iter()
            .find(|e| &e.fingerprint == fingerprint)
            .map(|e| {
                e.samples
                    .iter()
                    .take(MAX_SAMPLES_PER_ENV)
                    .filter_map(from_stored)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// この環境の見本を、表の同じ環境の見本で置き換える（なければ足す）。環境の数が上限を超えたら古いものから捨てる。
    pub fn set_env(&mut self, fingerprint: &EnvFingerprint, table: &GlyphTable) {
        let samples: Vec<StoredSample> = table
            .same_env_samples()
            .into_iter()
            .take(MAX_SAMPLES_PER_ENV)
            .map(to_stored)
            .collect();
        self.envs.retain(|e| &e.fingerprint != fingerprint);
        if !samples.is_empty() {
            self.envs.push(StoredEnv {
                fingerprint: fingerprint.clone(),
                samples,
            });
        }
        let excess = self.envs.len().saturating_sub(MAX_ENVS);
        self.envs.drain(..excess);
    }

    /// 環境の数（テスト・診断用）。
    #[must_use]
    pub fn env_count(&self) -> usize {
        self.envs.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::indicator_glyph::{preprocess, GlyphGray};

    /// 合成画像から特徴を作る（`indicator_glyph` のテストと同じ要領の簡易版: 中央の縦棒と横棒）。
    fn feature(thick: usize) -> GlyphFeature {
        let (w, h) = (32usize, 24usize);
        let mut px = vec![225u8; w * h];
        for y in 4..20 {
            for x in 16 - thick..16 + thick {
                px[y * w + x] = 30;
            }
        }
        for y in 10..10 + thick * 2 {
            for x in 8..24 {
                px[y * w + x] = 30;
            }
        }
        preprocess(&GlyphGray::new(w, h, px).expect("寸法が合う")).expect("前景がある")
    }

    fn fp(dark: bool) -> EnvFingerprint {
        EnvFingerprint::new(32, 24, dark, "gji")
    }

    #[test]
    fn mask_hex_round_trips_and_rejects_bad_input() {
        let mut m = [false; FEATURE_AREA];
        for i in (0..FEATURE_AREA).step_by(3) {
            m[i] = true;
        }
        let hex = mask_to_hex(&m);
        assert_eq!(hex.len(), 100);
        assert_eq!(mask_from_hex(&hex), Some(m));
        assert_eq!(mask_from_hex("abc"), None);
        assert_eq!(mask_from_hex(&"z".repeat(100)), None);
    }

    #[test]
    fn samples_survive_a_json_round_trip_with_the_same_distances() {
        let f = feature(2);
        let mut table = GlyphTable::from_common_samples(Vec::new());
        // 同じ環境の見本として足す（出所 SameEnv）。
        let _ = table.add(GlyphSample {
            class: GlyphClass::Hiragana,
            feature: f.clone(),
            origin: SampleOrigin::SameEnv,
        });
        let mut file = GlyphSampleFile::default();
        file.set_env(&fp(false), &table);
        let json = file.to_json();
        let back = GlyphSampleFile::from_json(&json);
        let samples = back.samples_for(&fp(false));
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].class, GlyphClass::Hiragana);
        assert_eq!(samples[0].origin, SampleOrigin::SameEnv);
        // 特徴は距離 0（2 値画像・縦横比・点数が保たれる）。
        assert!(samples[0].feature.distance(&f) < 1.0e-3);
        assert_eq!(samples[0].feature.dots(), f.dots());
        // 画素は保存しない。
        assert!(!json.contains("pixels"));
    }

    #[test]
    fn only_same_env_samples_are_stored_and_other_environments_are_separate() {
        let mut table = GlyphTable::from_common_samples(vec![GlyphSample {
            class: GlyphClass::Closed,
            feature: feature(1),
            origin: SampleOrigin::Common,
        }]);
        let _ = table.add(GlyphSample {
            class: GlyphClass::Hiragana,
            feature: feature(3),
            origin: SampleOrigin::SameEnv,
        });
        let mut file = GlyphSampleFile::default();
        file.set_env(&fp(false), &table);
        // 共通の見本は保存しない（同じ環境の 1 つだけ）。
        assert_eq!(file.samples_for(&fp(false)).len(), 1);
        // 違う環境（ダーク・別の大きさ・別の IME・別の取り込み方式）の見本は返さない。
        assert!(file.samples_for(&fp(true)).is_empty());
        assert!(file
            .samples_for(&EnvFingerprint::new(40, 30, false, "gji"))
            .is_empty());
        assert!(file
            .samples_for(&EnvFingerprint::new(32, 24, false, "ms-ime"))
            .is_empty());
        let mut other = fp(false);
        other.capture_version = CAPTURE_VERSION + 1;
        assert!(file.samples_for(&other).is_empty());
    }

    #[test]
    fn broken_or_other_version_files_are_read_as_empty() {
        assert_eq!(GlyphSampleFile::from_json("").env_count(), 0);
        assert_eq!(GlyphSampleFile::from_json("{not json").env_count(), 0);
        assert_eq!(
            GlyphSampleFile::from_json(r#"{"version":999,"envs":[]}"#).env_count(),
            0
        );
        // 読めない見本の行（未知のクラス・壊れた 16 進）は捨て、他の行は残す。
        let mut table = GlyphTable::from_common_samples(Vec::new());
        let _ = table.add(GlyphSample {
            class: GlyphClass::Hiragana,
            feature: feature(2),
            origin: SampleOrigin::SameEnv,
        });
        let mut file = GlyphSampleFile::default();
        file.set_env(&fp(false), &table);
        let json = file.to_json().replace("hiragana", "no-such-class");
        assert!(GlyphSampleFile::from_json(&json)
            .samples_for(&fp(false))
            .is_empty());
    }

    #[test]
    fn set_env_replaces_and_caps_the_number_of_environments() {
        let mut table = GlyphTable::from_common_samples(Vec::new());
        let _ = table.add(GlyphSample {
            class: GlyphClass::Hiragana,
            feature: feature(2),
            origin: SampleOrigin::SameEnv,
        });
        let mut file = GlyphSampleFile::default();
        for w in 0..(MAX_ENVS as u16 + 4) {
            file.set_env(&EnvFingerprint::new(32 + w, 24, false, "gji"), &table);
        }
        assert_eq!(file.env_count(), MAX_ENVS);
        // 古い環境（最初の 4 つ）は捨てられ、新しいものは残る。
        assert!(file
            .samples_for(&EnvFingerprint::new(32, 24, false, "gji"))
            .is_empty());
        assert_eq!(
            file.samples_for(&EnvFingerprint::new(
                32 + MAX_ENVS as u16 + 3,
                24,
                false,
                "gji"
            ))
            .len(),
            1
        );
        // 同じ環境の再保存は置き換え（増えない）。
        let before = file.env_count();
        file.set_env(
            &EnvFingerprint::new(32 + MAX_ENVS as u16 + 3, 24, false, "gji"),
            &table,
        );
        assert_eq!(file.env_count(), before);
        // 見本が無い環境で保存すると、その環境の行は消える。
        file.set_env(
            &EnvFingerprint::new(32 + MAX_ENVS as u16 + 3, 24, false, "gji"),
            &GlyphTable::from_common_samples(Vec::new()),
        );
        assert_eq!(file.env_count(), before - 1);
    }
}
