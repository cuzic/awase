//! フォーカス変更時に belief（`effective_open`）を `applied` へ写してよいかの判断（純粋）。
//!
//! 何も apply していないのに `applied` を `Confirmed` として書く写し（mirror）は、読めない窓では偽の確定になる
//! （ADR-098 が TsfNative で止めた型。ADR-262 段階 A が、実測学習で降格された窓にも広げた）。
//! GJI の already-matched は `applied` が向きと一致すると後続の ON の書き込みを省くので、偽の `Confirmed(true)` は
//! #623（belief ON・実 IME OFF）を直せなくする。
//!
//! 写しを行う殻の 3 箇所は、すべてこの関数を通す（`tests/architecture_guard.rs` が固定）:
//! `runtime/focus_tracking.rs` の先同期（`presync_applied_open_on` の呼び出し）、
//! `runtime/ime_refresh.rs::ir_align_placeholder_desired`、`runtime/ime_refresh.rs::ir_post_focus_change_snapshot`。

/// フォーカス変更で `applied` へ写してよいなら `true`。
///
/// - `tsf_native`: 実効的に TSF ネイティブの窓。`applied` は Unknown のまま維持する（ADR-098 決定 1-a）。
/// - `demoted_by_learning`: 窓内の IMM が読めないと実測学習され、`Standard` から `Imm32Unavailable` に降格された窓
///   （一太郎等。静的リスト由来の `Imm32Unavailable`〔Chrome/Edge/UWP〕は含まない）。ADR-262 段階 A。
#[must_use]
pub const fn may_mirror_applied_on_focus(tsf_native: bool, demoted_by_learning: bool) -> bool {
    !tsf_native && !demoted_by_learning
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirrors_only_for_ordinary_windows() {
        assert!(may_mirror_applied_on_focus(false, false));
    }

    #[test]
    fn does_not_mirror_for_tsf_native() {
        assert!(!may_mirror_applied_on_focus(true, false));
    }

    #[test]
    fn does_not_mirror_for_windows_demoted_by_learning() {
        assert!(!may_mirror_applied_on_focus(false, true));
    }

    #[test]
    fn does_not_mirror_when_both() {
        assert!(!may_mirror_applied_on_focus(true, true));
    }
}
