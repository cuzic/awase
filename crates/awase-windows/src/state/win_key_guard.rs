//! Win キー「押されたまま」判定の純粋関数（`hook.rs` から分離）。
//!
//! `hook.rs::win_key_held()` が保持する `PHYSICAL_KEY_STATE`/`PHYSICAL_KEY_DOWN_AT_MS`
//! は Win32 API 依存だが、「保持時間から stale かどうかを判定する」ロジック自体は
//! Win32 を呼ばない純粋関数のため、`alt_impersonation.rs` 移設と同じ理由でここに
//! 分離する — Linux の `cargo test -p awase-windows --lib` から常時実行できることが
//! 再発防止の本体になる（2026-08-06 実機: Win キー押下時に KeyUp が失われ
//! `PHYSICAL_KEY_STATE` が恒久的にスタックした不具合の対策）。

/// 保持時間から「まだ本当に押されているとみなせるか」を判定する。
///
/// `held_ms` が `stale_after_ms` 以上続いている場合は stale
/// （KeyUp 消失によるスタック）とみなし `false` を返す。
#[must_use]
pub(crate) const fn is_held_fresh(held_ms: Option<u64>, stale_after_ms: u64) -> bool {
    matches!(held_ms, Some(ms) if ms < stale_after_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_is_not_held() {
        assert!(!is_held_fresh(None, 2_000));
    }

    #[test]
    fn fresh_hold_is_held() {
        assert!(is_held_fresh(Some(0), 2_000));
        assert!(is_held_fresh(Some(1_999), 2_000));
    }

    #[test]
    fn stale_hold_is_not_held() {
        assert!(!is_held_fresh(Some(2_000), 2_000));
        assert!(!is_held_fresh(Some(10_000), 2_000));
    }
}
