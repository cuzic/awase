use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

/// N-gram model for adaptive threshold adjustment.
///
/// Adjusts the simultaneous keystroke detection threshold based on
/// the likelihood of character sequences. Common sequences get a
/// more lenient threshold, rare ones get tighter.
#[derive(Debug)]
pub struct NgramModel {
    /// 2-gram frequency: (prev_char, current_char) -> log-probability score
    bigram: HashMap<(char, char), f32>,
    /// 3-gram frequency: (2-back, 1-back, current) -> log-probability score
    trigram: HashMap<(char, char, char), f32>,
    /// Base threshold in microseconds
    base_threshold_us: u64,
    /// Adjustment range in microseconds (+/-)
    adjustment_range_us: u64,
    /// Minimum threshold in microseconds (clamp lower bound)
    min_threshold_us: u64,
    /// Maximum threshold in microseconds (clamp upper bound)
    max_threshold_us: u64,
}

impl NgramModel {
    /// Create a new empty `NgramModel` with the given base threshold and adjustment range.
    #[must_use]
    pub fn new(
        base_threshold_us: u64,
        adjustment_range_us: u64,
        min_threshold_us: u64,
        max_threshold_us: u64,
    ) -> Self {
        Self {
            bigram: HashMap::new(),
            trigram: HashMap::new(),
            base_threshold_us,
            adjustment_range_us,
            min_threshold_us,
            max_threshold_us,
        }
    }

    /// Load from a file path (TOML, CSV, or gzip-compressed CSV).
    ///
    /// Format is auto-detected:
    /// - `.gz` extension triggers gzip decompression first
    /// - Content starting with `[` is parsed as TOML
    /// - Otherwise parsed as CSV (`type,chars,score`)
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read, decompressed, or parsed.
    pub fn from_file(
        path: &Path,
        base_threshold_us: u64,
        adjustment_range_us: u64,
        min_threshold_us: u64,
        max_threshold_us: u64,
    ) -> anyhow::Result<Self> {
        let content = if path.extension().is_some_and(|e| e == "gz") {
            let file = std::fs::File::open(path)?;
            let mut decoder = flate2::read::GzDecoder::new(file);
            let mut s = String::new();
            decoder.read_to_string(&mut s)?;
            s
        } else {
            std::fs::read_to_string(path)?
        };

        if content.trim_start().starts_with('[') {
            Self::from_toml(
                &content,
                base_threshold_us,
                adjustment_range_us,
                min_threshold_us,
                max_threshold_us,
            )
        } else {
            Self::from_csv(
                &content,
                base_threshold_us,
                adjustment_range_us,
                min_threshold_us,
                max_threshold_us,
            )
        }
    }

    /// Load from a CSV string.
    ///
    /// Expected format:
    /// ```csv
    /// # comment lines start with #
    /// 2,かき,-1.234
    /// 3,かきく,-3.456
    /// ```
    ///
    /// First column: `2` for bigram, `3` for trigram.
    /// Second column: character sequence.
    /// Third column: float score.
    ///
    /// # Errors
    ///
    /// Returns an error if a line has an incorrect format or mismatched
    /// character count for the given n-gram type.
    pub fn from_csv(
        csv: &str,
        base_threshold_us: u64,
        adjustment_range_us: u64,
        min_threshold_us: u64,
        max_threshold_us: u64,
    ) -> anyhow::Result<Self> {
        let mut bigram = HashMap::new();
        let mut trigram = HashMap::new();

        for line in csv.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let parts: Vec<&str> = line.splitn(3, ',').collect();
            if parts.len() != 3 {
                anyhow::bail!("invalid CSV line: {line}");
            }
            let n: u8 = parts[0].parse()?;
            let chars: Vec<char> = parts[1].chars().collect();
            let score: f32 = parts[2].parse()?;
            match n {
                2 if chars.len() == 2 => {
                    bigram.insert((chars[0], chars[1]), score);
                }
                3 if chars.len() == 3 => {
                    trigram.insert((chars[0], chars[1], chars[2]), score);
                }
                _ => anyhow::bail!(
                    "invalid n-gram: type={n} with {} chars in line: {line}",
                    chars.len()
                ),
            }
        }

        Ok(Self {
            bigram,
            trigram,
            base_threshold_us,
            adjustment_range_us,
            min_threshold_us,
            max_threshold_us,
        })
    }

    /// Load from a TOML string.
    ///
    /// Expected format:
    /// ```toml
    /// [bigram]
    /// "ある" = 1.5
    ///
    /// [trigram]
    /// "ありが" = 2.5
    /// ```
    ///
    /// Keys are multi-char strings; they are split into individual chars
    /// for the tuple keys (2 chars for bigram, 3 chars for trigram).
    ///
    /// # Errors
    ///
    /// Returns an error if the TOML cannot be parsed or if any key has
    /// an incorrect number of characters.
    pub fn from_toml(
        toml_str: &str,
        base_threshold_us: u64,
        adjustment_range_us: u64,
        min_threshold_us: u64,
        max_threshold_us: u64,
    ) -> anyhow::Result<Self> {
        let table: toml::Value = toml_str.parse()?;

        let mut bigram = HashMap::new();
        if let Some(bi_table) = table.get("bigram").and_then(toml::Value::as_table) {
            for (key, value) in bi_table {
                let chars: Vec<char> = key.chars().collect();
                if chars.len() != 2 {
                    anyhow::bail!(
                        "bigram key must be exactly 2 characters, got {} for {:?}",
                        chars.len(),
                        key
                    );
                }
                #[allow(clippy::cast_precision_loss)]
                let score = value
                    .as_float()
                    .or_else(|| value.as_integer().map(|i| i as f64))
                    .ok_or_else(|| anyhow::anyhow!("bigram value for {key:?} is not a number"))?;
                #[allow(clippy::cast_possible_truncation)]
                bigram.insert((chars[0], chars[1]), score as f32);
            }
        }

        let mut trigram = HashMap::new();
        if let Some(tri_table) = table.get("trigram").and_then(toml::Value::as_table) {
            for (key, value) in tri_table {
                let chars: Vec<char> = key.chars().collect();
                if chars.len() != 3 {
                    anyhow::bail!(
                        "trigram key must be exactly 3 characters, got {} for {:?}",
                        chars.len(),
                        key
                    );
                }
                #[allow(clippy::cast_precision_loss)]
                let score = value
                    .as_float()
                    .or_else(|| value.as_integer().map(|i| i as f64))
                    .ok_or_else(|| anyhow::anyhow!("trigram value for {key:?} is not a number"))?;
                #[allow(clippy::cast_possible_truncation)]
                trigram.insert((chars[0], chars[1], chars[2]), score as f32);
            }
        }

        Ok(Self {
            bigram,
            trigram,
            base_threshold_us,
            adjustment_range_us,
            min_threshold_us,
            max_threshold_us,
        })
    }

    /// Calculate adjusted threshold based on recent output and candidate character.
    ///
    /// Uses the n-gram frequency score to adjust the base threshold:
    /// - Positive score (common bigram/trigram) -> more lenient threshold
    /// - Negative score (rare bigram/trigram) -> tighter threshold
    /// - Zero (unknown combination) -> base threshold unchanged
    ///
    /// Result is clamped to [`min_threshold_us`, `max_threshold_us`].
    #[must_use]
    pub fn adjusted_threshold(&self, recent: &[char], candidate: char) -> u64 {
        let score = self.frequency_score(recent, candidate);
        #[allow(clippy::cast_precision_loss)]
        let base = self.base_threshold_us as f64;
        #[allow(clippy::cast_precision_loss)]
        let range = self.adjustment_range_us as f64;
        // tanh maps to [-1, 1], then scale by range
        let adjustment = f64::from(score).tanh() * range;
        #[allow(clippy::cast_precision_loss)]
        let min = self.min_threshold_us as f64;
        #[allow(clippy::cast_precision_loss)]
        let max = self.max_threshold_us as f64;
        // Clamped to [min, max] which are both non-negative, so the cast is safe.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        {
            (base + adjustment).clamp(min, max) as u64
        }
    }

    /// Compute the frequency score for a candidate character given recent output.
    ///
    /// Tries 3-gram first (if enough history), then falls back to 2-gram.
    /// Returns 0.0 for unknown combinations (neutral).
    #[must_use]
    pub fn frequency_score(&self, recent: &[char], candidate: char) -> f32 {
        // Try 3-gram first
        if recent.len() >= 2 {
            if let Some(&score) = self.trigram.get(&(
                recent[recent.len() - 2],
                recent[recent.len() - 1],
                candidate,
            )) {
                return score;
            }
        }
        // Fall back to 2-gram
        if let Some(&prev) = recent.last() {
            if let Some(&score) = self.bigram.get(&(prev, candidate)) {
                return score;
            }
        }
        0.0 // unknown combination = neutral
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_model() -> NgramModel {
        let mut model = NgramModel::new(100_000, 20_000, 30_000, 120_000);
        model.bigram.insert(('あ', 'る'), 1.5);
        model.bigram.insert(('し', 'た'), 1.8);
        model.bigram.insert(('を', 'ぱ'), -1.5);
        model.bigram.insert(('ぬ', 'ゔ'), -2.0);
        model.trigram.insert(('あ', 'り', 'が'), 2.5);
        model.trigram.insert(('し', 'て', 'い'), 2.2);
        model
    }

    #[test]
    fn frequency_score_bigram() {
        let model = sample_model();
        let recent = vec!['あ'];
        assert!((model.frequency_score(&recent, 'る') - 1.5).abs() < f32::EPSILON);
    }

    #[test]
    fn frequency_score_trigram_takes_priority() {
        let model = sample_model();
        // With 2 chars of history, trigram should be tried first
        let recent = vec!['あ', 'り'];
        assert!((model.frequency_score(&recent, 'が') - 2.5).abs() < f32::EPSILON);
    }

    #[test]
    fn frequency_score_falls_back_to_bigram() {
        let model = sample_model();
        // Trigram ('x', 'し', 'た') doesn't exist, but bigram ('し', 'た') does
        let recent = vec!['x', 'し'];
        assert!((model.frequency_score(&recent, 'た') - 1.8).abs() < f32::EPSILON);
    }

    #[test]
    fn frequency_score_unknown_returns_zero() {
        let model = sample_model();
        let recent = vec!['x'];
        assert!((model.frequency_score(&recent, 'y')).abs() < f32::EPSILON);
    }

    #[test]
    fn frequency_score_empty_history_returns_zero() {
        let model = sample_model();
        let recent: Vec<char> = vec![];
        assert!((model.frequency_score(&recent, 'あ')).abs() < f32::EPSILON);
    }

    #[test]
    fn adjusted_threshold_neutral_returns_base() {
        let model = NgramModel::new(100_000, 20_000, 30_000, 120_000);
        // Unknown combination -> score=0 -> tanh(0)=0 -> no adjustment
        let threshold = model.adjusted_threshold(&['x'], 'y');
        assert_eq!(threshold, 100_000);
    }

    #[test]
    fn adjusted_threshold_high_frequency_increases() {
        let model = sample_model();
        // bigram ('あ', 'る') = 1.5, tanh(1.5) ~ 0.905
        let threshold = model.adjusted_threshold(&['あ'], 'る');
        assert!(threshold > 100_000, "high-freq should increase threshold");
        // Should be approximately 100_000 + 0.905 * 20_000 = ~118_100
        assert!(threshold > 115_000);
        assert!(threshold < 120_001);
    }

    #[test]
    fn adjusted_threshold_low_frequency_decreases() {
        let model = sample_model();
        // bigram ('ぬ', 'ゔ') = -2.0, tanh(-2.0) ~ -0.964
        let threshold = model.adjusted_threshold(&['ぬ'], 'ゔ');
        assert!(threshold < 100_000, "low-freq should decrease threshold");
        // Should be approximately 100_000 - 0.964 * 20_000 = ~80_720
        assert!(threshold < 85_000);
        assert!(threshold > 30_000);
    }

    #[test]
    fn adjusted_threshold_clamps_to_range() {
        // Very small base: should clamp to 30_000
        let mut model = NgramModel::new(25_000, 5_000, 30_000, 120_000);
        model.bigram.insert(('a', 'b'), -3.0);
        let threshold = model.adjusted_threshold(&['a'], 'b');
        assert_eq!(threshold, 30_000);

        // Very large base: should clamp to 120_000
        let mut model = NgramModel::new(130_000, 5_000, 30_000, 120_000);
        model.bigram.insert(('a', 'b'), 3.0);
        let threshold = model.adjusted_threshold(&['a'], 'b');
        assert_eq!(threshold, 120_000);
    }

    #[test]
    fn from_toml_parses_correctly() {
        let toml_str = r#"
[bigram]
"ある" = 1.5
"した" = 1.8
"をぱ" = -1.5

[trigram]
"ありが" = 2.5
"ですか" = 2.0
"#;
        let model = NgramModel::from_toml(toml_str, 100_000, 20_000, 30_000, 120_000).unwrap();
        assert!((model.frequency_score(&['あ'], 'る') - 1.5).abs() < f32::EPSILON);
        assert!((model.frequency_score(&['し'], 'た') - 1.8).abs() < f32::EPSILON);
        assert!((model.frequency_score(&['を'], 'ぱ') - (-1.5)).abs() < f32::EPSILON);
        assert!((model.frequency_score(&['あ', 'り'], 'が') - 2.5).abs() < f32::EPSILON);
        assert!((model.frequency_score(&['で', 'す'], 'か') - 2.0).abs() < f32::EPSILON);
    }

    #[test]
    fn from_toml_rejects_wrong_length_bigram() {
        let toml_str = r#"
[bigram]
"abc" = 1.0
"#;
        let result = NgramModel::from_toml(toml_str, 100_000, 20_000, 30_000, 120_000);
        assert!(result.is_err());
    }

    #[test]
    fn from_toml_rejects_wrong_length_trigram() {
        let toml_str = r#"
[trigram]
"ab" = 1.0
"#;
        let result = NgramModel::from_toml(toml_str, 100_000, 20_000, 30_000, 120_000);
        assert!(result.is_err());
    }

    #[test]
    fn from_toml_empty_sections_ok() {
        let toml_str = "";
        let model = NgramModel::from_toml(toml_str, 100_000, 20_000, 30_000, 120_000).unwrap();
        assert!((model.frequency_score(&['a'], 'b')).abs() < f32::EPSILON);
    }

    #[test]
    fn from_toml_integer_values_work() {
        let toml_str = r#"
[bigram]
"あい" = 2
"#;
        let model = NgramModel::from_toml(toml_str, 100_000, 20_000, 30_000, 120_000).unwrap();
        assert!((model.frequency_score(&['あ'], 'い') - 2.0).abs() < f32::EPSILON);
    }

    #[test]
    fn from_csv_parses_correctly() {
        let csv = "\
# comment line
2,ある,1.5
2,した,1.8
2,をぱ,-1.5

3,ありが,2.5
3,ですか,2.0
";
        let model = NgramModel::from_csv(csv, 100_000, 20_000, 30_000, 120_000).unwrap();
        assert!((model.frequency_score(&['あ'], 'る') - 1.5).abs() < f32::EPSILON);
        assert!((model.frequency_score(&['し'], 'た') - 1.8).abs() < f32::EPSILON);
        assert!((model.frequency_score(&['を'], 'ぱ') - (-1.5)).abs() < f32::EPSILON);
        assert!((model.frequency_score(&['あ', 'り'], 'が') - 2.5).abs() < f32::EPSILON);
        assert!((model.frequency_score(&['で', 'す'], 'か') - 2.0).abs() < f32::EPSILON);
    }

    #[test]
    fn from_csv_empty_and_comments_ok() {
        let csv = "# only comments\n\n# another comment\n";
        let model = NgramModel::from_csv(csv, 100_000, 20_000, 30_000, 120_000).unwrap();
        assert!((model.frequency_score(&['a'], 'b')).abs() < f32::EPSILON);
    }

    #[test]
    fn from_csv_rejects_wrong_char_count() {
        let csv = "2,abc,1.0\n";
        let result = NgramModel::from_csv(csv, 100_000, 20_000, 30_000, 120_000);
        assert!(result.is_err());
    }

    #[test]
    fn from_csv_rejects_invalid_line() {
        let csv = "bad line\n";
        let result = NgramModel::from_csv(csv, 100_000, 20_000, 30_000, 120_000);
        assert!(result.is_err());
    }

    #[test]
    fn from_file_detects_csv_format() {
        let dir = std::env::temp_dir().join("ngram_test_csv");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.csv");
        std::fs::write(&path, "2,ある,1.5\n3,ありが,2.5\n").unwrap();
        let model = NgramModel::from_file(&path, 100_000, 20_000, 30_000, 120_000).unwrap();
        assert!((model.frequency_score(&['あ'], 'る') - 1.5).abs() < f32::EPSILON);
        assert!((model.frequency_score(&['あ', 'り'], 'が') - 2.5).abs() < f32::EPSILON);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn from_file_detects_toml_format() {
        let dir = std::env::temp_dir().join("ngram_test_toml");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.toml");
        std::fs::write(
            &path,
            "[bigram]\n\"ある\" = 1.5\n\n[trigram]\n\"ありが\" = 2.5\n",
        )
        .unwrap();
        let model = NgramModel::from_file(&path, 100_000, 20_000, 30_000, 120_000).unwrap();
        assert!((model.frequency_score(&['あ'], 'る') - 1.5).abs() < f32::EPSILON);
        assert!((model.frequency_score(&['あ', 'り'], 'が') - 2.5).abs() < f32::EPSILON);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn from_file_reads_gzip() {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use std::io::Write;

        let dir = std::env::temp_dir().join("ngram_test_gz");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.csv.gz");

        let csv_data = "2,ある,1.5\n3,ありが,2.5\n";
        let file = std::fs::File::create(&path).unwrap();
        let mut encoder = GzEncoder::new(file, Compression::default());
        encoder.write_all(csv_data.as_bytes()).unwrap();
        encoder.finish().unwrap();

        let model = NgramModel::from_file(&path, 100_000, 20_000, 30_000, 120_000).unwrap();
        assert!((model.frequency_score(&['あ'], 'る') - 1.5).abs() < f32::EPSILON);
        assert!((model.frequency_score(&['あ', 'り'], 'が') - 2.5).abs() < f32::EPSILON);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
