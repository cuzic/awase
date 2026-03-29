//! Windows API の安全ラッパー

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT};
use windows::Win32::UI::Input::Ime::{ImmGetContext, ImmReleaseContext, ImmSetOpenStatus, HIMC};

/// `SendInput` の安全ラッパー（`size_of` キャストを安全に処理）
pub fn send_input_safe(inputs: &[INPUT]) -> u32 {
    let size = i32::try_from(size_of::<INPUT>())
        .expect("INPUT size fits in i32");
    unsafe { SendInput(inputs, size) }
}

/// `HWND` が有効（非 null）かを判定
#[allow(dead_code)] // 将来のリファクタリングで使用予定
pub const fn hwnd_is_valid(hwnd: HWND) -> bool {
    !hwnd.0.is_null()
}

/// IME コンテキストの RAII ラッパー
///
/// Drop 時に自動で `ImmReleaseContext` を呼ぶ。
pub struct ImeContext {
    hwnd: HWND,
    himc: HIMC,
}

impl ImeContext {
    /// 指定ウィンドウの IME コンテキストを取得する。無効な場合は `None`。
    ///
    /// # Safety
    ///
    /// `hwnd` は有効なウィンドウハンドルであること。
    pub unsafe fn open(hwnd: HWND) -> Option<Self> {
        let himc = ImmGetContext(hwnd);
        if himc.is_invalid() {
            None
        } else {
            Some(Self { hwnd, himc })
        }
    }

    /// IME のオープン状態を設定する
    ///
    /// # Safety
    ///
    /// 呼び出し元は IME コンテキストが有効なスレッドから呼ぶこと。
    pub unsafe fn set_open_status(&self, open: bool) {
        let _ = ImmSetOpenStatus(self.himc, open);
    }

    /// 内部の `HIMC` を取得（`ImmGetConversionStatus` 等で必要）
    #[allow(dead_code)] // 将来 ImmGetConversionStatus 等で使用予定
    pub const fn himc(&self) -> HIMC {
        self.himc
    }
}

impl Drop for ImeContext {
    fn drop(&mut self) {
        unsafe {
            let _ = ImmReleaseContext(self.hwnd, self.himc);
        }
    }
}

impl std::fmt::Debug for ImeContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImeContext")
            .field("hwnd", &self.hwnd)
            .finish()
    }
}
