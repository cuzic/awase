//! IMM32 (Input Method Manager) 低レベルユーティリティ。
//!
//! IME 制御定数・RAII コンテキストガード・クロスプロセスクエリヘルパーを一元管理する。
//! `ime.rs` / `ime_diagnostic.rs` / `observer/ime_observer.rs` に分散していた重複を集約。

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Input::Ime::{HIMC, ImmGetContext, ImmGetDefaultIMEWnd, ImmReleaseContext};
use windows::Win32::UI::WindowsAndMessaging::{SendMessageTimeoutW, SMTO_ABORTIFHUNG};

// ─── IME 制御メッセージ・定数 ────────────────────────────────────

pub(crate) const WM_IME_CONTROL: u32 = 0x0283;
pub(crate) const IMC_GETOPENSTATUS: usize = 0x0005;
pub(crate) const IMC_SETOPENSTATUS: usize = 0x0006;
pub(crate) const IMC_GETCONVERSIONMODE: usize = 0x0001;
pub(crate) const IMC_SETCONVERSIONMODE: usize = 0x0002;

/// ローマ字入力モードフラグ（0x0010）
pub(crate) const IME_CMODE_ROMAN: u32 = 0x0010;
/// 日本語ネイティブ入力モードフラグ（0x0001）
pub(crate) const IME_CMODE_NATIVE: u32 = 0x0001;
/// カタカナ入力モードフラグ（0x0002）
pub(crate) const IME_CMODE_KATAKANA: u32 = 0x0002;

/// IME 変換モード生値が指定フラグを含むかどうかを返す（診断ログ等で使う）。
#[must_use]
pub(crate) const fn cmode_has(mode: u32, flag: u32) -> bool {
    mode & flag != 0
}

// ─── RAII コンテキストガード ─────────────────────────────────────

/// `ImmGetContext` / `ImmReleaseContext` の RAII ガード。
///
/// `new()` で取得し、`Drop` で自動リリースする。
/// `himc.is_invalid()` の場合は `None` を返す。
pub(crate) struct ImmContextGuard {
    hwnd: HWND,
    himc: HIMC,
}

impl ImmContextGuard {
    /// # Safety
    /// `hwnd` は有効なウィンドウハンドルでなければならない。
    pub(crate) unsafe fn new(hwnd: HWND) -> Option<Self> {
        // SAFETY: hwnd は呼出元でチェック済みの有効なウィンドウハンドル。
        //         ImmReleaseContext は Drop で必ず呼ばれる RAII パターン。
        let himc = unsafe { ImmGetContext(hwnd) };
        if himc.is_invalid() { None } else { Some(Self { hwnd, himc }) }
    }

    pub(crate) fn himc(&self) -> HIMC {
        self.himc
    }
}

impl Drop for ImmContextGuard {
    fn drop(&mut self) {
        // SAFETY: self.hwnd と self.himc は new() で ImmGetContext が返した有効なペア。
        //         ImmReleaseContext は ImmGetContext と必ず対になる RAII パターン。
        unsafe { let _ = ImmReleaseContext(self.hwnd, self.himc); }
    }
}

// ─── IME ウィンドウヘルパー ───────────────────────────────────────

/// `ImmGetDefaultIMEWnd` の null チェック付きラッパー。
///
/// IMM ブリッジが存在する場合は `Some(ime_hwnd)` を返す。
///
/// # Safety
/// Win32 API を呼び出す。
pub(crate) unsafe fn get_ime_wnd(hwnd: HWND) -> Option<HWND> {
    // SAFETY: hwnd は呼出元でチェック済みの有効なウィンドウハンドル。
    //         ImmGetDefaultIMEWnd は hwnd に対応する IME ウィンドウを返すだけで副作用なし。
    crate::win32::non_null_hwnd(unsafe { ImmGetDefaultIMEWnd(hwnd) })
}

// ─── クロスプロセス IME コントロール ─────────────────────────────

/// `WM_IME_CONTROL` を IME ウィンドウに送信し、結果を返す。
///
/// タイムアウトまたはエラー時は `None` を返す。
///
/// # Safety
/// Win32 API を呼び出す。
pub(crate) unsafe fn send_ime_control(
    ime_wnd: HWND,
    cmd: usize,
    lparam: isize,
    timeout_ms: u32,
) -> Option<usize> {
    let mut result = 0usize;
    // SAFETY: ime_wnd は呼出元が ImmGetDefaultIMEWnd で取得した有効な IME ウィンドウハンドル。
    //         SMTO_ABORTIFHUNG によりハングしたスレッドで無期限にブロックしない。
    //         result はスタック上の有効な usize でポインタ渡しが安全。
    let ok = unsafe {
        SendMessageTimeoutW(
            ime_wnd,
            WM_IME_CONTROL,
            WPARAM(cmd),
            LPARAM(lparam),
            SMTO_ABORTIFHUNG,
            timeout_ms,
            Some(&raw mut result),
        )
    };
    (ok.0 != 0).then_some(result)
}
