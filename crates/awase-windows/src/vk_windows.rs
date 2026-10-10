//! `vk` モジュールの windows crate 依存の殻（`#[cfg(windows)]`。ADR-229 段階 B）。
//!
//! 核へ移す `vk.rs` は windows crate に依存しない。windows crate の定数との突き合わせ
//! （コンパイル時 assert）と、`MOD_*` を使う `parse_hotkey` はここに置く。

use windows::Win32::UI::Input::KeyboardAndMouse as km;

use crate::vk::{VK_JUNJA, VK_LWIN, VK_NONAME, VK_RWIN};

/// `vk_table!` の各行を、`windows` crate の同名の `VIRTUAL_KEY` 定数とコンパイル時に突き合わせる。
/// Microsoft 自身のメタデータがオラクルになるので、表の値の打ち間違い
/// （`VK_LSHIFT`/`VK_RSHIFT` の入れ替えなど）は `cargo check --target x86_64-pc-windows-msvc` で検出される。
macro_rules! assert_matches_windows_crate {
    ($( $(#[doc = $doc:expr])* $id:ident = $vk:literal $(, [$($alias:literal),* $(,)?])? ; )*) => {
        $(
            const _: () = assert!(
                km::$id.0 == $vk,
                concat!("VK 値が windows crate の定数と違う: ", stringify!($id))
            );
        )*
    };
}

crate::vk_table!(assert_matches_windows_crate);

// 表外の4定数も、表と同じく windows crate の定数と突き合わせる。
const _: () = {
    assert!(km::VK_JUNJA.0 == VK_JUNJA.0);
    assert!(km::VK_LWIN.0 == VK_LWIN.0);
    assert!(km::VK_RWIN.0 == VK_RWIN.0);
    assert!(km::VK_NONAME.0 == VK_NONAME.0);
};

/// ホットキー文字列をパースして修飾キーフラグと仮想キーコードに変換する。
///
/// `windows::Win32::UI::Input::KeyboardAndMouse::{MOD_ALT, MOD_CONTROL, MOD_SHIFT}` に
/// 依存するため殻（このファイル）に置く。`vk` モジュールは windows crate に依存しない
/// （ADR-082「決定1実施記録」の次の一歩、ADR-229 段階 B で `vk` を核へ移すための分離）。
/// 解釈は [`parse_key_combo`] と同じ(BUG-167: 手書きの `F12` と GUI の `VK_F12` の両表記、
/// `変換` などの日本語名も `from_name` が受理する)。
#[must_use]
pub fn parse_hotkey(s: &str) -> Option<(u32, awase::types::VkCode)> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{MOD_ALT, MOD_CONTROL, MOD_SHIFT};

    let k = crate::vk::parse_key_combo(s)?;
    let mut modifiers = 0u32;
    if k.ctrl {
        modifiers |= MOD_CONTROL.0;
    }
    if k.shift {
        modifiers |= MOD_SHIFT.0;
    }
    if k.alt {
        modifiers |= MOD_ALT.0;
    }
    Some((modifiers, k.vk))
}

/// [`crate::vk::is_role_candidate`] の解決結果を 1 度だけ作る版（全打鍵で通る本番の入口）。
#[must_use]
pub fn is_role_candidate_cached(vk: awase::types::VkCode) -> bool {
    static CANDIDATES: std::sync::OnceLock<Vec<awase::types::VkCode>> = std::sync::OnceLock::new();
    CANDIDATES
        .get_or_init(crate::vk::role_candidates)
        .contains(&vk)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `parse_hotkey`（Windows 専用。Linux では走らず windows-build CI で走る）が
    /// 両表記・日本語名・大文字小文字で同じ修飾キー・VK を返すこと。
    #[test]
    fn parse_hotkey_accepts_gui_and_handwritten_spellings() {
        let handwritten = parse_hotkey("Ctrl+Shift+F12");
        let gui = parse_hotkey("Ctrl+Shift+VK_F12");
        assert!(handwritten.is_some());
        assert_eq!(handwritten, gui);
        assert_eq!(parse_hotkey("ctrl+shift+vk_f12"), gui);
        let (_, vk) = parse_hotkey("Ctrl+Shift+変換").unwrap();
        assert_eq!(vk, crate::vk::VK_CONVERT);
        assert!(parse_hotkey("Ctrl+").is_none());
        assert!(parse_hotkey("Bogus+F12").is_none());
    }
}
