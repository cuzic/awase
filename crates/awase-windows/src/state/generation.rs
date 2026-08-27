//! `ApplyGeneration` 専用アロケータ（ADR-106 決定1）。
//!
//! 旧 `allocate_event_generation` は `ImeEventLog.next_seq()`（診断用リング
//! バッファの通し番号、`&self` で読むだけ）をそのまま流用しており、一意性は
//! 呼び出し元が必ず `dispatch_event` して `next_seq` を実際に進める、という
//! 型で守られない契約にのみ依存していた（`generation = 0` が bootstrap 経路で
//! 実際に払い出されうる、ADR-106 原因A参照）。
//!
//! `GenerationAllocator` は `&mut self` の専用アロケータとして、
//! - 読むだけで進まないことを型で不可能にする
//! - `NonZeroU64` により `0` を「generation なし」の番兵として使うのを
//!   型として正しくする（`Option<ApplyGeneration>` は無損失で `0 = None` に
//!   エンコードできる）
//! - `next_seq`（診断ログ）から独立するため、記録有無と generation の一意性が
//!   無関係になる

use std::num::NonZeroU64;

/// OS への IME apply 要求ごとに払い出される世代 ID。stale な async/sync 完了の
/// 照合に使う。`NonZeroU64` のため `0` を安全に「generation なし」の番兵として
/// 使える（`to_wire`/`from_wire` 参照）。
///
/// `serde::Serialize` は `ImeEvent`（journal 書き出し用）が要求する。書き出し
/// 専用のため `Deserialize` は導出しない（`ime_event.rs` と同じ方針）。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, serde::Serialize)]
pub struct ApplyGeneration(NonZeroU64);

impl ApplyGeneration {
    /// テスト・診断用に生の `u64` から構築する。`0` は `None` を返す。
    #[must_use]
    pub const fn new(value: u64) -> Option<Self> {
        match NonZeroU64::new(value) {
            Some(n) => Some(Self(n)),
            None => None,
        }
    }

    /// 生の `u64` 値を返す（ログ・wire エンコード用）。
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0.get()
    }

    /// `Option<ApplyGeneration>` を `u64` へ無損失エンコードする（`None` → `0`）。
    /// `ApplyGeneration` は `NonZeroU64` なので `0` は正当な generation 値と
    /// 衝突しない。
    #[must_use]
    pub const fn to_wire(value: Option<Self>) -> u64 {
        match value {
            Some(g) => g.get(),
            None => 0,
        }
    }

    /// [`to_wire`] の逆変換。`0` は `None` にデコードする。
    #[must_use]
    pub const fn from_wire(value: u64) -> Option<Self> {
        Self::new(value)
    }
}

impl std::fmt::Display for ApplyGeneration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.get())
    }
}

/// [`ApplyGeneration`] の唯一のアロケータ。`&mut self` の `allocate()` だけが
/// 次の値を払い出せる——読み取り専用の `&self` メソッドは存在しない。
#[derive(Debug)]
pub struct GenerationAllocator {
    next: NonZeroU64,
}

impl GenerationAllocator {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            next: NonZeroU64::MIN,
        }
    }

    /// 次の generation を払い出し、内部カウンタを進める。
    ///
    /// `u64::MAX` に達した場合は `NonZeroU64::MIN`（=1）に折り返す。
    /// 実運用でこの折り返しに到達することは無い（1ms間隔で払い出しても
    /// 5億年以上かかる）。
    pub fn allocate(&mut self) -> ApplyGeneration {
        let g = self.next;
        self.next = self.next.checked_add(1).unwrap_or(NonZeroU64::MIN);
        ApplyGeneration(g)
    }
}

impl Default for GenerationAllocator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocate_starts_at_one_and_increments() {
        let mut alloc = GenerationAllocator::new();
        assert_eq!(alloc.allocate().get(), 1);
        assert_eq!(alloc.allocate().get(), 2);
        assert_eq!(alloc.allocate().get(), 3);
    }

    #[test]
    fn allocate_never_yields_zero() {
        let mut alloc = GenerationAllocator::new();
        for _ in 0..1000 {
            assert_ne!(alloc.allocate().get(), 0);
        }
    }

    #[test]
    fn allocate_wraps_from_u64_max_to_one() {
        let mut alloc = GenerationAllocator {
            next: NonZeroU64::new(u64::MAX).unwrap(),
        };
        assert_eq!(alloc.allocate().get(), u64::MAX);
        assert_eq!(
            alloc.allocate().get(),
            1,
            "u64::MAX の次は 1 に折り返す（0 には絶対にならない）"
        );
    }

    #[test]
    fn new_rejects_zero() {
        assert_eq!(ApplyGeneration::new(0), None);
        assert!(ApplyGeneration::new(1).is_some());
    }

    #[test]
    fn wire_roundtrip_is_lossless_for_none_and_all_nonzero_samples() {
        assert_eq!(ApplyGeneration::to_wire(None), 0);
        assert_eq!(ApplyGeneration::from_wire(0), None);

        for raw in [1u64, 2, 9, 10, 42, 1_000_000, u64::MAX] {
            let g = ApplyGeneration::new(raw).unwrap();
            let wire = ApplyGeneration::to_wire(Some(g));
            assert_eq!(wire, raw, "to_wire は 0 を経由せず raw 値をそのまま運ぶ");
            assert_eq!(
                ApplyGeneration::from_wire(wire),
                Some(g),
                "from_wire(to_wire(x)) == x"
            );
        }
    }

    #[test]
    fn display_matches_get() {
        let g = ApplyGeneration::new(42).unwrap();
        assert_eq!(format!("{g}"), "42");
    }
}
