// Win32 API (Shell_NotifyIconW, CreateWindowExW 等) の使用に unsafe が必須
#![allow(unsafe_code)]

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, SelectObject, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
};
use windows::Win32::UI::Shell::{
    ShellExecuteW, Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD,
    NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon,
    DestroyMenu, DestroyWindow, GetCursorPos, PostQuitMessage, RegisterClassW, SetForegroundWindow,
    TrackPopupMenu, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, ICONINFO, MF_STRING, SW_SHOWNORMAL,
    TPM_BOTTOMALIGN, TPM_LEFTALIGN, WM_DESTROY, WM_RBUTTONUP, WNDCLASSW, WS_OVERLAPPEDWINDOW,
};

use anyhow::{Context, Result};

/// トレイメニュー項目 ID
const IDM_SETTINGS: u16 = 50;
const IDM_RESTART_ADMIN: u16 = 51;
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
#[allow(missing_debug_implementations)]
pub struct SystemTray {
    hwnd: HWND,
    nid: NOTIFYICONDATAW,
    /// 利用可能な配列名の一覧（メニュー表示用）
    layout_names: Vec<String>,
    /// 現在アクティブな配列名
    current_layout_name: String,
    /// 管理者権限で実行中かどうか
    elevated: bool,
}

impl SystemTray {
    /// トレイアイコンを作成する
    ///
    /// # Errors
    ///
    /// ウィンドウクラスの登録、ウィンドウの作成、またはトレイアイコンの追加に失敗した場合
    pub fn new(enabled: bool, elevated: bool) -> Result<Self> {
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

            // キーボードシルエットアイコンを生成
            let icon = create_keyboard_icon(enabled).unwrap_or_default();

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
            set_tooltip(&mut nid, enabled, "", elevated);

            // トレイアイコンを追加
            Shell_NotifyIconW(NIM_ADD, &raw const nid)
                .ok()
                .context("Failed to add tray icon")?;

            log::info!("System tray icon created (elevated={elevated})");

            Ok(Self {
                hwnd,
                nid,
                layout_names: Vec::new(),
                current_layout_name: String::new(),
                elevated,
            })
        }
    }

    /// トレイアイコンのツールチップとアイコンを更新する
    pub fn set_enabled(&mut self, enabled: bool) {
        set_tooltip(
            &mut self.nid,
            enabled,
            &self.current_layout_name,
            self.elevated,
        );
        if let Some(icon) = create_keyboard_icon(enabled) {
            // 古いアイコンを破棄してから差し替え
            if !self.nid.hIcon.is_invalid() {
                unsafe {
                    let _ = DestroyIcon(self.nid.hIcon);
                }
            }
            self.nid.hIcon = icon;
        }
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
        set_tooltip(
            &mut self.nid,
            true,
            &self.current_layout_name,
            self.elevated,
        );
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &raw const self.nid);
        }
    }

    /// トレイウィンドウの HWND を返す
    #[must_use]
    pub const fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// Explorer 再起動時にトレイアイコンを再登録する
    pub fn recreate(&self) {
        unsafe {
            let _ = Shell_NotifyIconW(NIM_ADD, &raw const self.nid);
        }
        log::info!("Tray icon re-registered after Explorer restart");
    }

    /// バルーン通知を表示する
    pub fn show_balloon(&mut self, title: &str, message: &str) {
        // szInfoTitle に UTF-16 タイトルをコピー
        let title_wide: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
        let title_len = title_wide.len().min(self.nid.szInfoTitle.len());
        self.nid.szInfoTitle[..title_len].copy_from_slice(&title_wide[..title_len]);

        // szInfo に UTF-16 メッセージをコピー
        let msg_wide: Vec<u16> = message.encode_utf16().chain(std::iter::once(0)).collect();
        let msg_len = msg_wide.len().min(self.nid.szInfo.len());
        self.nid.szInfo[..msg_len].copy_from_slice(&msg_wide[..msg_len]);

        // バルーン表示用フラグを設定
        self.nid.uFlags = NIF_INFO;
        self.nid.dwInfoFlags = NIIF_INFO;

        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &raw const self.nid);
        }

        // フラグを元に戻す（次回の NIM_MODIFY でバルーンが意図せず再表示されないように）
        self.nid.uFlags = NIF_ICON | NIF_TIP | NIF_MESSAGE;
    }
}

impl Drop for SystemTray {
    fn drop(&mut self) {
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &raw const self.nid);
            let _ = DestroyWindow(self.hwnd);
        }
        log::info!("System tray icon destroyed");
    }
}

// ── トレイアイコン描画定義 ──

/// アイコンサイズ（ピクセル）
const ICON_SIZE: i32 = 16;

/// BGRA カラー定義
mod icon_color {
    /// 透明（背景）
    pub const TRANSPARENT: u32 = 0x00_00_00_00;
    /// ON 時のキーボード本体（青系）
    pub const BODY_ON: u32 = 0xFF_D4_7B_2E;
    /// OFF 時のキーボード本体（グレー）
    pub const BODY_OFF: u32 = 0xFF_80_80_80;
    /// ON 時のキートップ（明るいクリーム色）
    pub const KEY_ON: u32 = 0xFF_FF_F0_E0;
    /// OFF 時のキートップ（薄いグレー）
    pub const KEY_OFF: u32 = 0xFF_C0_C0_C0;
}

/// キーボード本体の描画範囲
const BODY_Y: std::ops::Range<usize> = 3..13;
const BODY_X: std::ops::Range<usize> = 1..15;

/// キー配列の定義: (y座標, [(x開始, x終了), ...])
const KEY_ROWS: &[(usize, &[(usize, usize)])] = &[
    (5, &[(3, 4), (5, 6), (7, 8), (9, 10), (11, 12)]), // 上段: 5キー
    (7, &[(3, 4), (5, 6), (7, 8), (9, 10), (11, 12)]), // 中段: 5キー
    (9, &[(4, 5), (6, 7), (8, 9), (10, 11)]),          // 下段: 4キー
];

/// スペースバーの描画範囲
const SPACEBAR_Y: usize = 11;
const SPACEBAR_X: std::ops::Range<usize> = 5..11;

/// 16x16 のキーボードシルエットアイコンを GDI で生成する。
///
/// ON: 青系のキーボード、OFF: グレーのキーボード。
fn create_keyboard_icon(enabled: bool) -> Option<windows::Win32::UI::WindowsAndMessaging::HICON> {
    unsafe {
        // DIB セクション（32bit ARGB）を作成
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: u32::try_from(size_of::<BITMAPINFOHEADER>()).unwrap_or(0),
                biWidth: ICON_SIZE,
                biHeight: -ICON_SIZE, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let dc = CreateCompatibleDC(None);
        let mut bits = std::ptr::null_mut();
        let color_bmp =
            CreateDIBSection(dc, &raw const bmi, DIB_RGB_COLORS, &raw mut bits, None, 0).ok()?;
        let mask_bmp = CreateDIBSection(
            dc,
            &raw const bmi,
            DIB_RGB_COLORS,
            std::ptr::null_mut(),
            None,
            0,
        )
        .ok()?;

        let stride = ICON_SIZE as usize;
        let pixels = std::slice::from_raw_parts_mut(bits.cast::<u32>(), stride * stride);

        let body_color = if enabled {
            icon_color::BODY_ON
        } else {
            icon_color::BODY_OFF
        };
        let key_color = if enabled {
            icon_color::KEY_ON
        } else {
            icon_color::KEY_OFF
        };

        // 背景クリア
        pixels.fill(icon_color::TRANSPARENT);

        // キーボード本体
        for y in BODY_Y {
            for x in BODY_X.clone() {
                pixels[y * stride + x] = body_color;
            }
        }

        // 四隅を透明にして角丸風に
        let top = BODY_Y.start;
        let bottom = BODY_Y.end - 1;
        let left = BODY_X.start;
        let right = BODY_X.end;
        pixels[top * stride + left] = icon_color::TRANSPARENT;
        pixels[top * stride + right] = icon_color::TRANSPARENT;
        pixels[bottom * stride + left] = icon_color::TRANSPARENT;
        pixels[bottom * stride + right] = icon_color::TRANSPARENT;

        // キー配列
        for &(y, keys) in KEY_ROWS {
            for &(x_start, x_end) in keys {
                for x in x_start..=x_end {
                    pixels[y * stride + x] = key_color;
                }
            }
        }

        // スペースバー
        for x in SPACEBAR_X {
            pixels[SPACEBAR_Y * stride + x] = key_color;
        }

        // マスクビットマップ（全不透明 — alpha チャネルで制御）
        let old = SelectObject(dc, color_bmp);

        let icon_info = ICONINFO {
            fIcon: true.into(),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask_bmp,
            hbmColor: color_bmp,
        };
        let icon = CreateIconIndirect(&raw const icon_info).ok();

        SelectObject(dc, old);
        let _ = DeleteDC(dc);
        let _ = DeleteObject(color_bmp);
        let _ = DeleteObject(mask_bmp);

        icon
    }
}

/// ツールチップ文字列を `NOTIFYICONDATAW` に設定する
fn set_tooltip(nid: &mut NOTIFYICONDATAW, enabled: bool, layout_name: &str, elevated: bool) {
    let admin_suffix = if elevated { " (管理者)" } else { "" };
    let tip = if layout_name.is_empty() {
        if enabled {
            format!("NICOLA: ON{admin_suffix}")
        } else {
            format!("NICOLA: OFF{admin_suffix}")
        }
    } else if enabled {
        format!("NICOLA: ON ({layout_name}){admin_suffix}")
    } else {
        format!("NICOLA: OFF ({layout_name}){admin_suffix}")
    };

    let tip_wide: Vec<u16> = tip.encode_utf16().chain(std::iter::once(0)).collect();
    let len = tip_wide.len().min(nid.szTip.len());
    nid.szTip[..len].copy_from_slice(&tip_wide[..len]);
}

/// トレイアイコンイベントを処理する
///
/// `WM_APP` メッセージを受け取った時にメッセージループから呼ばれる。
/// 右クリックでコンテキストメニューを表示する。
pub fn handle_tray_message(hwnd: HWND, lparam: LPARAM, layout_names: &[String], elevated: bool) {
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

        // 管理者として再起動（未昇格時のみ表示）
        if !elevated {
            let admin_text: Vec<u16> = "管理者として再起動"
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let _ = AppendMenuW(
                hmenu,
                MF_STRING,
                usize::from(IDM_RESTART_ADMIN),
                PCWSTR(admin_text.as_ptr()),
            );
        }

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
        IDM_SETTINGS | IDM_RESTART_ADMIN | IDM_TOGGLE | IDM_EXIT => Some(cmd),
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

/// 管理者再起動メニューコマンド ID のアクセサ
pub const fn cmd_restart_admin() -> u16 {
    IDM_RESTART_ADMIN
}

/// 現在のプロセスが管理者権限で実行中かどうかを判定する。
///
/// `shell32.dll` の `IsUserAnAdmin` を使用する。
/// この API は非推奨だが、シンプルで `Win32_Security` feature を追加せずに使えるため採用。
pub fn is_elevated() -> bool {
    #[link(name = "shell32")]
    extern "system" {
        fn IsUserAnAdmin() -> i32;
    }
    unsafe { IsUserAnAdmin() != 0 }
}

/// 管理者権限で自身を再起動する。
///
/// `ShellExecuteW` の "runas" verb で UAC ダイアログを表示し、
/// 成功したら現在のプロセスを終了する。
pub fn restart_as_admin() {
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            log::error!("Failed to get current exe path: {e}");
            return;
        }
    };

    let exe_wide: Vec<u16> = exe
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let verb: Vec<u16> = "runas".encode_utf16().chain(std::iter::once(0)).collect();

    unsafe {
        let result = ShellExecuteW(
            HWND::default(),
            PCWSTR(verb.as_ptr()),
            PCWSTR(exe_wide.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
        // ShellExecuteW returns HINSTANCE > 32 on success
        if result.0 as isize > 32 {
            log::info!("Restarting as admin, exiting current process");
            std::process::exit(0);
        } else {
            log::warn!("Failed to restart as admin (user may have cancelled UAC)");
        }
    }
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
