//! `config1.db` の置き場所（製品ごと）。純粋（パスの部品を返すだけで、ファイルシステムには触れない）。
//!
//! Mozc 系の IME は、ユーザーのプロファイルの `AppData\LocalLow` の下に `config1.db` を置く
//! （Mozc の `SystemUtil::GetUserProfileDirectory`。Windows では `WinUtil::GetLocalAppDataPath` = LocalLow）。
//! 製品名のフォルダーは `kProductNameInEnglish`、Google 日本語入力の配布版だけ、その前に会社名のフォルダーが挟まる。
//!
//! | 製品 | `LocalLow` からの相対パス |
//! | --- | --- |
//! | Google 日本語入力 | `Google\Google Japanese Input\config1.db` |
//! | Mozc（公式の OSS 版） | `Mozc\config1.db` |
//! | mozc-readable（Mozc の fork） | `Mozc Readable\config1.db`（製品名を `Mozc Readable` に変えてあるため。別のユーザープロファイルになる） |
//!
//! 3 つとも、protobuf の形式は同じ（GJI の実装ベースが Mozc。[`crate::wire`] の注意を参照）。

/// `config1.db` を持つ製品。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigProduct {
    /// Google 日本語入力（配布版）
    GoogleJapaneseInput,
    /// Mozc（公式の OSS 版）
    Mozc,
    /// mozc-readable（Mozc の fork。製品名は `Mozc Readable`）
    MozcReadable,
}

impl ConfigProduct {
    /// `AppData\LocalLow` の下の、`config1.db` までの相対パス（フォルダー名とファイル名を順に）。
    #[must_use]
    pub const fn relative_path(self) -> &'static [&'static str] {
        match self {
            Self::GoogleJapaneseInput => &["Google", "Google Japanese Input", "config1.db"],
            Self::Mozc => &["Mozc", "config1.db"],
            Self::MozcReadable => &["Mozc Readable", "config1.db"],
        }
    }

    /// 診断ログ用のラベル。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::GoogleJapaneseInput => "gji",
            Self::Mozc => "mozc",
            Self::MozcReadable => "mozc-readable",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ConfigProduct;

    #[test]
    fn relative_paths_match_each_products_profile_directory() {
        assert_eq!(
            ConfigProduct::GoogleJapaneseInput.relative_path(),
            ["Google", "Google Japanese Input", "config1.db"]
        );
        assert_eq!(ConfigProduct::Mozc.relative_path(), ["Mozc", "config1.db"]);
        assert_eq!(
            ConfigProduct::MozcReadable.relative_path(),
            ["Mozc Readable", "config1.db"]
        );
    }

    #[test]
    fn every_path_ends_with_the_config_file_and_has_no_empty_part() {
        for p in [
            ConfigProduct::GoogleJapaneseInput,
            ConfigProduct::Mozc,
            ConfigProduct::MozcReadable,
        ] {
            let parts = p.relative_path();
            assert_eq!(parts.last().copied(), Some("config1.db"));
            assert!(parts.iter().all(|s| !s.is_empty()));
        }
    }

    #[test]
    fn products_do_not_share_a_directory() {
        // mozc-readable は公式の Mozc と別のプロファイル（辞書や設定は引き継がれない）
        assert_ne!(
            ConfigProduct::Mozc.relative_path(),
            ConfigProduct::MozcReadable.relative_path()
        );
    }

    #[test]
    fn labels_are_distinct() {
        let labels = [
            ConfigProduct::GoogleJapaneseInput.label(),
            ConfigProduct::Mozc.label(),
            ConfigProduct::MozcReadable.label(),
        ];
        assert_eq!(labels, ["gji", "mozc", "mozc-readable"]);
    }
}
