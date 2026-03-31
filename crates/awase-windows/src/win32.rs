//! Windows API の安全ラッパー

use windows::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT};

/// `SendInput` の安全ラッパー（`size_of` キャストを安全に処理）
pub fn send_input_safe(inputs: &[INPUT]) -> u32 {
    let size = i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32");
    unsafe { SendInput(inputs, size) }
}
