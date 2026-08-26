//! ADR-106 決定3(`FocusHwndUpdated` による `current_focus_hwnd` 追従修正)を
//! 実機で検証するための、使い捨て2ウィンドウ IMM32 プローブ。
//!
//! 同一プロセス内に独立したトップレベルウィンドウを2つ作り、それぞれに
//! `EDIT` コントロール(組み込みクラス)を1つ埋め込む。TSF/UWP 要素は一切
//! 使わない素の `WNDCLASSW` 登録 + `CreateWindowExW` のみで構成しており、
//! これが IMM32 経路であることの担保になる(`crates/awase-windows/examples/
//! gji_composition_probe.rs` と同じパターン)。
//!
//! Windows Terminal (TsfNative) は ADR-106 決定2により観測を記録しない
//! アプリで検証にならず、Windows 11 のメモ帳も現在は IMM32 ベースでは
//! なくなっているため、確実に IMM32 を使うテスト専用アプリとしてこれを
//! 作った。ユーザーが Alt+Tab やクリックで2ウィンドウ間を手動で切り替え、
//! `state/observation_store.rs` の `derive_any()` 内 `is_identity_ok` が
//! `current_focus_hwnd` に正しく追従するかを awase のデバッグログと
//! 突き合わせて確認する。
//!
//! # 実行方法(Windows 実機のみ)
//!
//! ```powershell
//! cargo run -p awase-windows --example two_imm32_windows_probe --release
//! ```
//!
//! 起動直後、標準出力に2つのウィンドウそれぞれの HWND(16進)とタイトルを
//! 表示する。これを awase のデバッグログに出る `HWND(0x...)` の値と突き合わせる。
//! 両方のウィンドウを閉じるとプロセスが終了する。

#![allow(unsafe_code)]

#[cfg(windows)]
mod windows_probe {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, PostQuitMessage,
        RegisterClassW, SetWindowLongPtrW, ShowWindow, TranslateMessage, CS_HREDRAW, CS_VREDRAW,
        GWLP_USERDATA, MSG, SW_SHOW, WM_DESTROY, WNDCLASSW, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
    };

    const WINDOW_CLASS_NAME: &str = "two_imm32_windows_probe_window";

    /// `EDIT` コントロール(組み込みクラス)の class-specific style。
    /// `gji_composition_probe.rs` と同じ raw 値(`windows` crate は `ES_*` を
    /// 独立定数として持たない)。
    const ES_MULTILINE: u32 = 0x0004;
    const ES_AUTOVSCROLL: u32 = 0x0040;
    const WS_CHILD: u32 = 0x4000_0000;
    const WS_BORDER: u32 = 0x0080_0000;

    /// 生存中のトップレベルウィンドウ数。0 になったらプロセスを終了する。
    static ALIVE_COUNT: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

    extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if msg == WM_DESTROY {
            let remaining =
                ALIVE_COUNT.fetch_sub(1, std::sync::atomic::Ordering::SeqCst) - 1;
            if remaining <= 0 {
                // SAFETY: メッセージループを持つスレッドから呼ぶ限り常に安全。
                unsafe { PostQuitMessage(0) };
            }
            return LRESULT(0);
        }
        // SAFETY: DefWindowProcW はどんな (hwnd, msg, wparam, lparam) の組でも
        // 安全に呼べる Win32 の既定ハンドラ。
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    fn to_wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// # Safety
    /// メインスレッドから、`class_name_wide`/`hinstance` が有効な間に呼ぶこと。
    unsafe fn create_probe_window(
        class_name_wide: &[u16],
        hinstance: windows::Win32::Foundation::HMODULE,
        title: &str,
        x: i32,
        y: i32,
    ) -> anyhow::Result<(HWND, HWND)> {
        let title_wide = to_wide(title);
        let parent = unsafe {
            CreateWindowExW(
                windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE::default(),
                PCWSTR(class_name_wide.as_ptr()),
                PCWSTR(title_wide.as_ptr()),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                x,
                y,
                480,
                260,
                None,
                None,
                Some(hinstance.into()),
                None,
            )
        }
        .map_err(|e| anyhow::anyhow!("CreateWindowExW (parent) failed: {e}"))?;

        let edit_class = to_wide("EDIT");
        let edit_style = windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE(
            WS_CHILD | WS_VISIBLE.0 | WS_BORDER | ES_MULTILINE | ES_AUTOVSCROLL,
        );
        let edit = unsafe {
            CreateWindowExW(
                windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE::default(),
                PCWSTR(edit_class.as_ptr()),
                PCWSTR::null(),
                edit_style,
                0,
                0,
                460,
                220,
                Some(parent),
                None,
                Some(hinstance.into()),
                None,
            )
        }
        .map_err(|e| anyhow::anyhow!("CreateWindowExW (edit) failed: {e}"))?;

        // SAFETY: parent は直前に作成した有効なウィンドウ。GWLP_USERDATA は
        // このプローブでは未使用のため上書きしても安全(将来の拡張余地として
        // 明示的に 0 で初期化しておく)。
        unsafe { SetWindowLongPtrW(parent, GWLP_USERDATA, 0) };

        let _ = unsafe { ShowWindow(parent, SW_SHOW) };

        ALIVE_COUNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        Ok((parent, edit))
    }

    pub(super) fn run() -> anyhow::Result<()> {
        let class_name_wide = to_wide(WINDOW_CLASS_NAME);
        // SAFETY: プロセス起動直後、他スレッドがまだ存在しない時点での呼び出し。
        let hinstance = unsafe { windows::Win32::System::LibraryLoader::GetModuleHandleW(None) }
            .unwrap_or_default();

        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinstance.into(),
            lpszClassName: PCWSTR(class_name_wide.as_ptr()),
            ..Default::default()
        };
        // SAFETY: wc はスタック上の有効な WNDCLASSW。呼び出しはプロセス起動直後の
        // 1回のみ。
        let atom = unsafe { RegisterClassW(&raw const wc) };
        anyhow::ensure!(atom != 0, "RegisterClassW failed");

        // SAFETY: create_probe_window はメインスレッドから、直前に登録した
        // ウィンドウクラス名を使って呼ぶ。重ならない位置(A=左上, B=Aの右隣)に置く。
        let (hwnd_a, _edit_a) =
            unsafe { create_probe_window(&class_name_wide, hinstance, "IMM32 Probe A", 100, 100) }?;
        let (hwnd_b, _edit_b) = unsafe {
            create_probe_window(&class_name_wide, hinstance, "IMM32 Probe B", 620, 100)
        }?;

        println!("IMM32 Probe A: hwnd={hwnd_a:?} title=\"IMM32 Probe A\"");
        println!("IMM32 Probe B: hwnd={hwnd_b:?} title=\"IMM32 Probe B\"");
        println!(
            "hex: A=0x{:X} B=0x{:X}",
            hwnd_a.0 as usize, hwnd_b.0 as usize
        );
        println!("両方のウィンドウを閉じるとこのプロセスは終了します。");

        let mut msg = MSG::default();
        // SAFETY: msg はスタック上の有効な MSG バッファ。hwnd=None でこのスレッドの
        // 全ウィンドウ分のメッセージを対象にする。GetMessageW は WM_QUIT で false を
        // 返しループを抜ける。
        while unsafe { GetMessageW(&raw mut msg, None, 0, 0) }.as_bool() {
            let _ = unsafe { TranslateMessage(&raw const msg) };
            unsafe { DispatchMessageW(&raw const msg) };
        }

        Ok(())
    }
}

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    windows_probe::run()
}

#[cfg(not(windows))]
fn main() {}
