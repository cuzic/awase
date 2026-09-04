#![allow(unsafe_code)]
// Win32 API 呼び出しに unsafe が必須(lib.rsのクレート全体allowから個別移管、Task #9)
//! 修飾キー（Ctrl / Shift / Alt）の解放・復元パターン（`HeldModifiers`）。
//!
//! `ime.rs` の VK_KANJI/VK_IME_ON/VK_IME_OFF 送信（`IME_KANJI_MARKER` 固定）と
//! `[[keymap]]` のターゲットキー送信（`send_keymap_target`、`INJECTED_MARKER`）が
//! 同じ構造体・メソッドを共有する（ADR-114 決定3）。
//!
//! **「共通化」は構造体とフィールドの切り出しに留め、どの修飾を解放するかは
//! 呼び出し側が明示的に指定する** — デフォルトで全解放するヘルパーは作らない。
//! `ime.rs` の3呼び出し元は Alt の扱いがそれぞれ異なる（全解放 / 常に解放しない）
//! ため、共通ヘルパーで一律に決め打つと壊れる。

use crate::tsf::output::make_key_input_ex;
use crate::vk::{
    VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_MENU, VK_RCONTROL, VK_RMENU, VK_RSHIFT,
    VK_SHIFT,
};
use awase::types::VkCode;
use windows::Win32::UI::Input::KeyboardAndMouse::INPUT;

/// 修飾キー（Ctrl / Shift / Alt）の押下状態スナップショット。
///
/// `SendInput` で修飾なしキーを届ける際の解放・復元シーケンス構築に使う。
#[derive(Clone, Copy)]
pub(crate) struct HeldModifiers {
    pub(crate) ctrl: bool,
    pub(crate) shift: bool,
    pub(crate) alt: bool,
}

impl HeldModifiers {
    /// 物理キー状態 (`PHYSICAL_KEY_STATE`) で修飾キーの押下状態を読み取る。
    ///
    /// `GetAsyncKeyState` は直前に注入した synthetic KeyUp の影響を受けて汚染される場合があるため、
    /// SendInput 非影響の物理キー状態で読み取ることで CTRL MISMATCH を防ぐ。
    pub(crate) fn read() -> Self {
        Self {
            ctrl: crate::hook::is_physical_key_down(VK_LCONTROL)
                || crate::hook::is_physical_key_down(VK_RCONTROL),
            shift: crate::hook::is_physical_key_down(VK_LSHIFT)
                || crate::hook::is_physical_key_down(VK_RSHIFT),
            alt: crate::hook::is_physical_key_down(VK_LMENU)
                || crate::hook::is_physical_key_down(VK_RMENU),
        }
    }

    /// 押下中の修飾キーを解放する `INPUT` イベントを追加する。
    ///
    /// `marker` は `dwExtraInfo` に付けるマーカー（`IME_KANJI_MARKER`/
    /// `INJECTED_MARKER` 等、呼び出し元が明示する）。
    pub(crate) fn push_release(self, inputs: &mut Vec<INPUT>, marker: usize) {
        if self.ctrl {
            inputs.push(make_key_input_ex(VK_CONTROL, true, marker));
        }
        if self.shift {
            inputs.push(make_key_input_ex(VK_SHIFT, true, marker));
        }
        if self.alt {
            inputs.push(make_key_input_ex(VK_MENU, true, marker));
        }
    }

    /// 物理的にまだ押下中の修飾キーを復元する `INPUT` イベントを追加し、復元した状態を返す。
    ///
    /// # Safety
    /// Win32 API を呼び出す。
    pub(crate) unsafe fn push_restore(self, inputs: &mut Vec<INPUT>, marker: usize) -> Self {
        // GetAsyncKeyState は直前に注入した synthetic Ctrl↑ の影響を受けるため、
        // SendInput 非影響の物理キー状態 (PHYSICAL_KEY_STATE) で判定する。
        // これにより Ctrl+W 等のショートカットを押したまま IME キーが注入された場合でも
        // Ctrl が正しく復元され、Chrome へ Ctrl+W が届く。
        let still = Self {
            ctrl: self.ctrl
                && (crate::hook::is_physical_key_down(VK_LCONTROL)
                    || crate::hook::is_physical_key_down(VK_RCONTROL)),
            shift: self.shift
                && (crate::hook::is_physical_key_down(VK_LSHIFT)
                    || crate::hook::is_physical_key_down(VK_RSHIFT)),
            alt: self.alt
                && (crate::hook::is_physical_key_down(VK_LMENU)
                    || crate::hook::is_physical_key_down(VK_RMENU)),
        };
        if still.ctrl {
            inputs.push(make_key_input_ex(VK_CONTROL, false, marker));
        }
        if still.shift {
            inputs.push(make_key_input_ex(VK_SHIFT, false, marker));
        }
        if still.alt {
            inputs.push(make_key_input_ex(VK_MENU, false, marker));
        }
        still
    }
}

/// `[[keymap]]` の `to` ターゲット送信（ADR-114 決定3、ADR-130 決定3・4）。
///
/// `release_ctrl`/`release_shift` は `find_match` がマッチした `from` の
/// `combo.ctrl`/`combo.shift` と一致する（マッチが成立した時点で
/// `combo.ctrl == mods.ctrl` かつ `combo.shift == mods.shift` が保証されるため、
/// 呼び出し元は `event.modifier_snapshot` の値をそのまま渡せる）。Alt は
/// ADR-114 決定5 で `from`/`to` 双方から禁止済みのため扱わない。
///
/// `target_vks` の各ステップを独立した Down/Up ペアとして、列全体を同一
/// `SendInput` バッチで送信する（描画前に完結させ、中間状態を外部に見せない。
/// Chrome cold-start 検出の VK_A+BS アトミックバッチと同じ原則）。
/// マーカーは `INJECTED_MARKER`（`ime.rs` の3箇所が使う
/// `IME_KANJI_MARKER` とは意図的に区別する — こちらは IME 漢字キー送信ではない）。
///
/// # Safety
/// Win32 API を呼び出す。メインスレッドから呼ぶこと。
pub(crate) unsafe fn send_keymap_target(
    release_ctrl: bool,
    release_shift: bool,
    target_vks: &[VkCode],
) {
    use crate::tsf::output::INJECTED_MARKER;

    let held = HeldModifiers {
        ctrl: release_ctrl,
        shift: release_shift,
        alt: false,
    };
    // 最大要素: release(ctrl+shift 最大2) + target_vks down/up + restore(最大2)。
    let mut inputs: Vec<INPUT> = Vec::with_capacity(2 + target_vks.len() * 2 + 2);
    held.push_release(&mut inputs, INJECTED_MARKER);
    for (vk, keyup) in crate::keymap::keymap_target_tap_pairs(target_vks) {
        inputs.push(make_key_input_ex(vk, keyup, INJECTED_MARKER));
    }
    // SAFETY: 呼び出し元の doc の通り、メインスレッドから呼ばれる前提。
    let _still = unsafe { held.push_restore(&mut inputs, INJECTED_MARKER) };

    let sent = crate::win32::send_input_safe(&inputs);
    if sent as usize != inputs.len() {
        log::warn!(
            "[keymap] SendInput(target_vk=0x{:02X}) sent {sent}/{} events",
            target_vks.first().map_or(0, |vk| vk.0),
            inputs.len()
        );
    }
}
