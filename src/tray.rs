// Win32 API (Shell_NotifyIconW, CreateWindowExW 等) の使用に unsafe が必須
#![allow(unsafe_code)]

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    GetCursorPos, LoadIconW, PostQuitMessage, RegisterClassW, SetForegroundWindow, TrackPopupMenu,
    CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, IDI_APPLICATION, MF_STRING, TPM_BOTTOMALIGN,
    TPM_LEFTALIGN, WM_DESTROY, WM_RBUTTONUP, WNDCLASSW, WS_OVERLAPPEDWINDOW,
};

use anyhow::{Context, Result};

/// トレイメニュー項目 ID
const IDM_SETTINGS: u16 = 50;
const IDM_TOGGLE: u16 = 1001;
const IDM_EXIT: u16 = 1002;

/// 配列選択メニュー項目のベース ID
const IDM_LAYOUT_BASE: u16 = 100;

/// トレイアイコン ID
const TRAY_ICON_ID: u32 = 1;

/// トレイアイコン用カスタムメッセージ
const WM_TRAY_CALLBACK: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP;

/// ウィンドウクラス名（設定 GUI からの `FindWindowW` 検索用に一定の名前を使う）
const WINDOW_CLASS_NAME: &str = "awase_tray_window";

/// システムトレイアイコン管理
pub struct SystemTray {
    hwnd: HWND,
    nid: NOTIFYICONDATAW,
    /// 利用可能な配列名の一覧（メニュー表示用）
    layout_names: Vec<String>,
    /// 現在アクティブな配列名
    current_layout_name: String,
}

impl SystemTray {
    /// トレイアイコンを作成する
    ///
    /// # Errors
    ///
    /// ウィンドウクラスの登録、ウィンドウの作成、またはトレイアイコンの追加に失敗した場合
    pub fn new(enabled: bool) -> Result<Self> {
        unsafe {
            // ウィンドウクラス名を UTF-16 に変換
            let class_name_wide: Vec<u16> = WINDOW_CLASS_NAME
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();

            let wc = WNDCLASSW {
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(tray_wnd_proc),
                hInstance: windows::Win32::System::LibraryLoader::GetModuleHandleW(None)
                    .unwrap_or_default()
                    .into(),
                lpszClassName: PCWSTR(class_name_wide.as_ptr()),
                ..Default::default()
            };

            let atom = RegisterClassW(&raw const wc);
            if atom == 0 {
                anyhow::bail!("Failed to register tray window class");
            }

            let hwnd = CreateWindowExW(
                windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE::default(),
                PCWSTR(class_name_wide.as_ptr()),
                PCWSTR::null(),
                WS_OVERLAPPEDWINDOW,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                None,
                None,
                wc.hInstance,
                None,
            )
            .context("Failed to create tray window")?;

            // デフォルトアイコンを読み込む
            let icon = LoadIconW(None, IDI_APPLICATION).unwrap_or_default();

            // NOTIFYICONDATAW を構築
            let mut nid = NOTIFYICONDATAW {
                cbSize: u32::try_from(size_of::<NOTIFYICONDATAW>()).unwrap_or(0),
                hWnd: hwnd,
                uID: TRAY_ICON_ID,
                uFlags: NIF_ICON | NIF_TIP | NIF_MESSAGE,
                uCallbackMessage: WM_TRAY_CALLBACK,
                hIcon: icon,
                ..Default::default()
            };

            // ツールチップ設定
            set_tooltip(&mut nid, enabled, "");

            // トレイアイコンを追加
            Shell_NotifyIconW(NIM_ADD, &raw const nid)
                .ok()
                .context("Failed to add tray icon")?;

            log::info!("System tray icon created");

            Ok(Self {
                hwnd,
                nid,
                layout_names: Vec::new(),
                current_layout_name: String::new(),
            })
        }
    }

    /// トレイアイコンのツールチップを更新する
    pub fn set_enabled(&mut self, enabled: bool) {
        set_tooltip(&mut self.nid, enabled, &self.current_layout_name);
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &raw const self.nid);
        }
    }

    /// 利用可能な配列名の一覧を設定する
    pub fn set_layout_names(&mut self, names: Vec<String>) {
        self.layout_names = names;
    }

    /// 現在の配列名を設定し、ツールチップを更新する
    pub fn set_layout_name(&mut self, name: &str) {
        self.current_layout_name = name.to_string();
        set_tooltip(&mut self.nid, true, &self.current_layout_name);
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &raw const self.nid);
        }
    }

    /// トレイアイコンを削除し、ウィンドウを破棄する
    pub fn destroy(&mut self) {
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &raw const self.nid);
            let _ = DestroyWindow(self.hwnd);
        }
        log::info!("System tray icon destroyed");
    }
}

/// ツールチップ文字列を `NOTIFYICONDATAW` に設定する
fn set_tooltip(nid: &mut NOTIFYICONDATAW, enabled: bool, layout_name: &str) {
    let tip = if layout_name.is_empty() {
        if enabled {
            "NICOLA: ON".to_string()
        } else {
            "NICOLA: OFF".to_string()
        }
    } else if enabled {
        format!("NICOLA: ON ({layout_name})")
    } else {
        format!("NICOLA: OFF ({layout_name})")
    };

    let tip_wide: Vec<u16> = tip.encode_utf16().chain(std::iter::once(0)).collect();
    let len = tip_wide.len().min(nid.szTip.len());
    nid.szTip[..len].copy_from_slice(&tip_wide[..len]);
}

/// トレイアイコンイベントを処理する
///
/// `WM_APP` メッセージを受け取った時にメッセージループから呼ばれる。
/// 右クリックでコンテキストメニューを表示する。
pub fn handle_tray_message(hwnd: HWND, lparam: LPARAM, layout_names: &[String]) {
    #[allow(clippy::cast_sign_loss)]
    let event = (lparam.0 & 0xFFFF) as u32;

    if event != WM_RBUTTONUP {
        return;
    }

    unsafe {
        let mut point = windows::Win32::Foundation::POINT::default();
        let _ = GetCursorPos(&raw mut point);

        let hmenu = CreatePopupMenu().unwrap_or_default();
        if hmenu.is_invalid() {
            return;
        }

        // 配列選択メニュー項目を追加
        for (i, name) in layout_names.iter().enumerate() {
            let text: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
            let id = usize::from(IDM_LAYOUT_BASE) + i;
            let _ = AppendMenuW(hmenu, MF_STRING, id, PCWSTR(text.as_ptr()));
        }

        // 配列が複数ある場合はセパレータを追加
        if !layout_names.is_empty() {
            let _ = AppendMenuW(
                hmenu,
                windows::Win32::UI::WindowsAndMessaging::MF_SEPARATOR,
                0,
                PCWSTR::null(),
            );
        }

        // 設定メニュー項目を追加
        let settings_text: Vec<u16> = "設定...".encode_utf16().chain(std::iter::once(0)).collect();
        let _ = AppendMenuW(
            hmenu,
            MF_STRING,
            usize::from(IDM_SETTINGS),
            PCWSTR(settings_text.as_ptr()),
        );

        // セパレータ
        let _ = AppendMenuW(
            hmenu,
            windows::Win32::UI::WindowsAndMessaging::MF_SEPARATOR,
            0,
            PCWSTR::null(),
        );

        // メニュー項目を追加
        let toggle_text: Vec<u16> = "有効/無効切替"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let exit_text: Vec<u16> = "終了".encode_utf16().chain(std::iter::once(0)).collect();

        let _ = AppendMenuW(
            hmenu,
            MF_STRING,
            usize::from(IDM_TOGGLE),
            PCWSTR(toggle_text.as_ptr()),
        );
        let _ = AppendMenuW(
            hmenu,
            MF_STRING,
            usize::from(IDM_EXIT),
            PCWSTR(exit_text.as_ptr()),
        );

        // メニュー表示前にウィンドウをフォアグラウンドにする（メニューが閉じるために必要）
        let _ = SetForegroundWindow(hwnd);

        let _ = TrackPopupMenu(
            hmenu,
            TPM_LEFTALIGN | TPM_BOTTOMALIGN,
            point.x,
            point.y,
            0,
            hwnd,
            None,
        );

        let _ = DestroyMenu(hmenu);
    }
}

/// トレイウィンドウの `WM_COMMAND` を処理する
///
/// # Returns
///
/// メニュー項目の ID を返す。
pub const fn handle_tray_command(wparam: WPARAM) -> Option<u16> {
    let cmd = (wparam.0 & 0xFFFF) as u16;
    match cmd {
        IDM_SETTINGS | IDM_TOGGLE | IDM_EXIT => Some(cmd),
        // 配列選択メニュー項目 (IDM_LAYOUT_BASE 以上の範囲)
        _ if cmd >= IDM_LAYOUT_BASE && cmd < IDM_TOGGLE => Some(cmd),
        _ => None,
    }
}

/// メニューコマンド ID のアクセサ
pub const fn cmd_toggle() -> u16 {
    IDM_TOGGLE
}

/// メニューコマンド ID のアクセサ
pub const fn cmd_exit() -> u16 {
    IDM_EXIT
}

/// 配列選択メニュー項目のベース ID のアクセサ
pub const fn cmd_layout_base() -> u16 {
    IDM_LAYOUT_BASE
}

/// 設定メニューコマンド ID のアクセサ
pub const fn cmd_settings() -> u16 {
    IDM_SETTINGS
}

/// トレイウィンドウプロシージャ
unsafe extern "system" fn tray_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
