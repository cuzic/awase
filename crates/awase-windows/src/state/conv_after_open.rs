//! `ConvAfterOpen` の ungated 表現（ADR-163 TH1a）。
//!
//! `crate::ime::ConvAfterOpen` は `#[cfg(windows)]` の下にあり Linux から
//! 参照できない。将来の actuation 決定入力/出力を Linux でテストできるように
//! するため、state 層に同じ意味を持つ値を ungated で置く。両者の変換は
//! Windows 境界（`ime.rs` の `From<ConvAfterOpenId> for ConvAfterOpen`）1 箇所だけに置く。

/// `open` 成功後に conv-mode も書くかどうかの指定。
///
/// `Write(None)` は ROMAN ビット確保のみ（既存 conv に `IME_CMODE_ROMAN` を追加）、
/// `Write(Some(v))` は `v` をそのまま設定する。`crate::ime::ConvAfterOpen` と同じ
/// 意味だが、こちらは state 層の ungated なミラーである。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ConvAfterOpenId {
    /// conv は書かない。
    Skip,
    /// `open` が成功したら続けて書く。
    Write(Option<u32>),
}

impl ConvAfterOpenId {
    /// 全 variant shape。`Write(Some(_))` は代表値で網羅性を固定する。
    pub const ALL: [Self; 3] = [Self::Skip, Self::Write(None), Self::Write(Some(0))];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ime_kind_style_all_covers_every_variant_shape() {
        for conv in ConvAfterOpenId::ALL {
            // match の網羅性で「ALL に載せ忘れた variant」を検出する。
            match conv {
                ConvAfterOpenId::Skip
                | ConvAfterOpenId::Write(None)
                | ConvAfterOpenId::Write(Some(_)) => {}
            }
        }
        assert_eq!(ConvAfterOpenId::ALL.len(), 3);
    }
}
