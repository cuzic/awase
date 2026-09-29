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

/// eager warmup（`VK_IME_ON` の合成注入）を、修飾キー押下中に送ってよいかの判定。
///
/// Win/Alt/Ctrl/Shift の**いずれか1つでも**押下中なら注入をブロックする。
/// 押下中に合成 `VK_IME_ON` を送ると、アプリ（GJI の TSF・Windows Terminal）には
/// 「修飾キー+`VK_IME_ON`」として届く。実機 A/B（BUG-175）で、Ctrl+Shift 押下中の
/// 注入で「@」が出て、注入を止めると出なくなることを確認した。
///
/// `hook::ime_mode_key_injection_blocked_by_modifier`（Win/Alt のみ）とは別にしてあるのは、
/// あちらの呼び出し元が Ctrl+無変換 等の IME OFF ショートカット（Ctrl を押したまま
/// IME 制御を注入する）を実現する必要があり、Ctrl をブロック対象に含められないため。
/// warmup は「念のため」の予防措置で、送らなくても cold-start は per-VK confirm が担う。
#[must_use]
pub(crate) const fn eager_warmup_blocked(win: bool, alt: bool, ctrl: bool, shift: bool) -> bool {
    win || alt || ctrl || shift
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eager_warmup_passes_only_when_no_modifier_is_held() {
        assert!(!eager_warmup_blocked(false, false, false, false));
    }

    #[test]
    fn eager_warmup_is_blocked_by_each_modifier_alone() {
        assert!(eager_warmup_blocked(true, false, false, false), "Win");
        assert!(eager_warmup_blocked(false, true, false, false), "Alt");
        assert!(eager_warmup_blocked(false, false, true, false), "Ctrl");
        assert!(eager_warmup_blocked(false, false, false, true), "Shift");
    }

    /// BUG-175: 報告 01M3NJYRQ5ZBYTKV55FV06KETP の Ctrl+Shift（Alt/Win なし）。
    #[test]
    fn eager_warmup_is_blocked_by_ctrl_shift_chord() {
        assert!(eager_warmup_blocked(false, false, true, true));
    }

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
