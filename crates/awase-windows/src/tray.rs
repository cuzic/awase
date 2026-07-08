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
    TrackPopupMenu, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, HMENU, ICONINFO, MF_CHECKED, MF_POPUP,
    MF_SEPARATOR, MF_STRING, SW_SHOWNORMAL, TPM_BOTTOMALIGN, TPM_LEFTALIGN, WM_CLOSE, WM_COMMAND,
    WM_DESTROY, WM_RBUTTONUP, WNDCLASSW, WS_OVERLAPPEDWINDOW,
};

use anyhow::{Context, Result};

/// トレイメニュー項目 ID
const IDM_SETTINGS: u16 = 50;
const IDM_RESTART_ADMIN: u16 = 51;
const IDM_CLEAR_IMM_CACHE: u16 = 52;
const IDM_AUTOSTART: u16 = 54;
const IDM_RESTART: u16 = 56;
const IDM_TOGGLE: u16 = 1001;
const IDM_EXIT: u16 = 1002;

/// 配列選択メニュー項目のベース ID
const IDM_LAYOUT_BASE: u16 = 100;

/// Caps Lock / IME 状態 / JISかな・ローマ字 メニュー項目 ID
const IDM_CAPSLOCK: u16 = 200;
const IDM_IME_HIRAGANA: u16 = 201;
const IDM_IME_FULL_KATAKANA: u16 = 202;
const IDM_IME_FULL_ALPHA: u16 = 203;
const IDM_IME_HALF_ALPHA: u16 = 204;
const IDM_IME_HALF_KATAKANA: u16 = 205;
const IDM_IME_DIRECT: u16 = 206;
const IDM_INPUT_ROMAJI: u16 = 207;
const IDM_INPUT_KANA: u16 = 208;
const IDM_RESET_STATE: u16 = 209;

/// トレイメニューから選択されたコマンド
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    Toggle,
    Exit,
    Settings,
    RestartAdmin,
    ClearImmCache,
    ToggleAutoStart,
    Restart,
    /// 配列選択（インデックスは `IDM_LAYOUT_BASE` からのオフセット）
    SelectLayout(usize),
    CapsLock,
    ImeHiragana,
    ImeFullKatakana,
    ImeFullAlpha,
    ImeHalfAlpha,
    ImeHalfKatakana,
    ImeDirect,
    InputRomaji,
    InputKana,
    ResetState,
}

/// 文字列メニュー項目を追加するヘルパー。
///
/// # Safety
/// `hmenu` は有効なポップアップメニューハンドルでなければならない。
unsafe fn append_menu_item(hmenu: HMENU, id: u16, label: &str) {
    let text = crate::win32::to_wide(label);
    let _ = unsafe { AppendMenuW(hmenu, MF_STRING, usize::from(id), PCWSTR(text.as_ptr())) };
}

/// セパレータを追加するヘルパー。
///
/// # Safety
/// `hmenu` は有効なポップアップメニューハンドルでなければならない。
unsafe fn append_menu_sep(hmenu: HMENU) {
    let _ = unsafe { AppendMenuW(hmenu, MF_SEPARATOR, 0, PCWSTR::null()) };
}

/// チェックマーク付き文字列メニュー項目を追加するヘルパー。
///
/// # Safety
/// `hmenu` は有効なポップアップメニューハンドルでなければならない。
unsafe fn append_menu_item_checked(hmenu: HMENU, id: u16, label: &str, checked: bool) {
    let text = crate::win32::to_wide(label);
    let flags = if checked {
        MF_STRING | MF_CHECKED
    } else {
        MF_STRING
    };
    let _ = unsafe { AppendMenuW(hmenu, flags, usize::from(id), PCWSTR(text.as_ptr())) };
}

/// トレイアイコン ID
const TRAY_ICON_ID: u32 = 1;

/// トレイアイコン用カスタムメッセージ
const WM_TRAY_CALLBACK: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP;

/// ウィンドウクラス名（設定 GUI や多重起動検出時の `FindWindowW` 検索用に一定の名前を使う）
pub const WINDOW_CLASS_NAME: &str = "awase_tray_window";

/// システムトレイアイコン管理
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

impl std::fmt::Debug for SystemTray {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SystemTray").finish_non_exhaustive()
    }
}

impl SystemTray {
    /// トレイアイコンを作成する
    ///
    /// # Errors
    ///
    /// ウィンドウクラスの登録、ウィンドウの作成、またはトレイアイコンの追加に失敗した場合
    pub fn new(enabled: bool, elevated: bool) -> Result<Self> {
        // SAFETY: `RegisterClassW`・`CreateWindowExW`・`Shell_NotifyIconW` はいずれも
        //         メインスレッドから呼ばれる Win32 UI API。`wc` や `nid` は直前に
        //         正しく初期化された有効な構造体ポインタを渡している。
        unsafe {
            // ウィンドウクラス名を UTF-16 に変換
            let class_name_wide = crate::win32::to_wide(WINDOW_CLASS_NAME);

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
                Some(wc.hInstance),
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
            // シェル未起動時（ログオン直後等）は失敗しても OK。
            // TaskbarCreated がブロードキャストされた時点で recreate() が呼ばれる。
            if !Shell_NotifyIconW(NIM_ADD, &raw const nid).as_bool() {
                log::warn!("Shell_NotifyIcon NIM_ADD failed — shell not ready, will retry on TaskbarCreated");
            } else {
                log::info!("System tray icon created (elevated={elevated})");
            }

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
                // SAFETY: `self.nid.hIcon` は `create_keyboard_icon` が返した有効な HICON。
                //         `is_invalid()` チェック済みのため NULL でないことが保証されている。
                unsafe {
                    let _ = DestroyIcon(self.nid.hIcon);
                }
            }
            self.nid.hIcon = icon;
        }
        // SAFETY: `self.nid` は `new()` で正しく初期化された有効な `NOTIFYICONDATAW`。
        //         `self.hwnd` は生存中の有効なトレイウィンドウハンドル。
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
        // SAFETY: `self.nid` は `new()` で正しく初期化された有効な `NOTIFYICONDATAW`。
        //         `self.hwnd` は生存中の有効なトレイウィンドウハンドル。
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
        // SAFETY: `self.nid` は `new()` で正しく初期化された有効な `NOTIFYICONDATAW`。
        //         Explorer 再起動後に再登録するため `NIM_ADD` を使用している。
        unsafe {
            let _ = Shell_NotifyIconW(NIM_ADD, &raw const self.nid);
        }
        log::info!("Tray icon re-registered after Explorer restart");
    }

    /// バルーン通知を表示する
    pub fn show_balloon(&mut self, title: &str, message: &str) {
        // szInfoTitle に UTF-16 タイトルをコピー
        let title_wide = crate::win32::to_wide(title);
        let title_len = title_wide.len().min(self.nid.szInfoTitle.len());
        self.nid.szInfoTitle[..title_len].copy_from_slice(&title_wide[..title_len]);

        // szInfo に UTF-16 メッセージをコピー
        let msg_wide = crate::win32::to_wide(message);
        let msg_len = msg_wide.len().min(self.nid.szInfo.len());
        self.nid.szInfo[..msg_len].copy_from_slice(&msg_wide[..msg_len]);

        // バルーン表示用フラグを設定
        self.nid.uFlags = NIF_INFO;
        self.nid.dwInfoFlags = NIIF_INFO;

        // SAFETY: `self.nid` は `new()` で正しく初期化された有効な `NOTIFYICONDATAW`。
        //         `NIF_INFO` フラグを設定し `NIM_MODIFY` でバルーン通知を送信する。
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &raw const self.nid);
        }

        // フラグを元に戻す（次回の NIM_MODIFY でバルーンが意図せず再表示されないように）
        self.nid.uFlags = NIF_ICON | NIF_TIP | NIF_MESSAGE;
    }
}

impl Drop for SystemTray {
    fn drop(&mut self) {
        // SAFETY: `self.nid` と `self.hwnd` は `new()` で作成された有効な構造体とハンドル。
        //         `Drop` は一度しか呼ばれず、`NIM_DELETE` でアイコン削除後にウィンドウを破棄する。
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
    pub(super) const TRANSPARENT: u32 = 0x00_00_00_00;
    /// ON 時のキーボード本体（青系）
    pub(super) const BODY_ON: u32 = 0xFF_D4_7B_2E;
    /// OFF 時のキーボード本体（グレー）
    pub(super) const BODY_OFF: u32 = 0xFF_80_80_80;
    /// ON 時のキートップ（明るいクリーム色）
    pub(super) const KEY_ON: u32 = 0xFF_FF_F0_E0;
    /// OFF 時のキートップ（薄いグレー）
    pub(super) const KEY_OFF: u32 = 0xFF_C0_C0_C0;
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
    // SAFETY: `CreateCompatibleDC`・`CreateDIBSection`・`CreateIconIndirect` は標準的な GDI 呼び出し。
    //         `bits` ポインタは `CreateDIBSection` が保証する有効なピクセルバッファを指す。
    //         `from_raw_parts_mut` の長さは `stride * stride`（16×16）で DIB バッファサイズと一致する。
    //         全 GDI オブジェクトは関数末尾で `DeleteDC`・`DeleteObject` により解放される。
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
        let color_bmp = CreateDIBSection(
            Some(dc),
            &raw const bmi,
            DIB_RGB_COLORS,
            &raw mut bits,
            None,
            0,
        )
        .ok()?;
        let mask_bmp = CreateDIBSection(
            Some(dc),
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
        let old = SelectObject(dc, color_bmp.into());

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
        let _ = DeleteObject(color_bmp.into());
        let _ = DeleteObject(mask_bmp.into());

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

    let tip_wide = crate::win32::to_wide(&tip);
    let len = tip_wide.len().min(nid.szTip.len());
    nid.szTip[..len].copy_from_slice(&tip_wide[..len]);
}

/// トレイアイコンイベントを処理する
///
/// `WM_APP` メッセージを受け取った時にメッセージループから呼ばれる。
/// 右クリックでコンテキストメニューを表示する。
pub fn handle_tray_message(hwnd: HWND, lparam: LPARAM, layout_names: &[String], elevated: bool) {
    #[expect(clippy::cast_sign_loss)]
    let event = (lparam.0 & 0xFFFF) as u32;

    log::debug!(
        "Tray message: event=0x{event:04X} lparam=0x{:016X}",
        lparam.0
    );

    if event != WM_RBUTTONUP {
        return;
    }

    // SAFETY: `hwnd` はシステムトレイ作成時に `CreateWindowExW` で得た有効なウィンドウハンドル。
    //         `GetCursorPos`・`CreatePopupMenu`・`AppendMenuW`・`TrackPopupMenu`・`DestroyMenu` は
    //         すべてメッセージループスレッドから呼ばれるため Win32 スレッド要件を満たす。
    unsafe {
        let mut point = windows::Win32::Foundation::POINT::default();
        let _ = GetCursorPos(&raw mut point);

        let hmenu = CreatePopupMenu().unwrap_or_default();
        if hmenu.is_invalid() {
            return;
        }

        // 配列選択
        for (i, name) in layout_names.iter().enumerate() {
            let text = crate::win32::to_wide(name);
            let id = usize::from(IDM_LAYOUT_BASE) + i;
            let _ = AppendMenuW(hmenu, MF_STRING, id, PCWSTR(text.as_ptr()));
        }
        if !layout_names.is_empty() {
            append_menu_sep(hmenu);
        }

        // Caps Lock / IME 状態 / JISかな・ローマ字
        let caps_lock_on = windows::Win32::UI::Input::KeyboardAndMouse::GetKeyState(0x14) & 1 != 0;
        let snap = crate::ime::read_ime_state_full();

        let ime_on = snap.ime_on.unwrap_or(false);
        let conv = snap.conversion_mode.unwrap_or(0);
        let is_romaji = snap.is_romaji.unwrap_or(true);

        let is_native = (conv & crate::imm::IME_CMODE_NATIVE) != 0;
        let is_katakana = (conv & crate::imm::IME_CMODE_KATAKANA) != 0;
        let is_fullshape = (conv & crate::imm::IME_CMODE_FULLSHAPE) != 0;

        let hiragana_checked = ime_on && is_native && !is_katakana && is_fullshape;
        let full_katakana_checked = ime_on && is_native && is_katakana && is_fullshape;
        let full_alpha_checked = ime_on && !is_native && is_fullshape;
        let half_alpha_checked = ime_on && !is_native && !is_fullshape;
        let half_katakana_checked = ime_on && is_native && is_katakana && !is_fullshape;
        let direct_checked = !ime_on;

        append_menu_item_checked(hmenu, IDM_CAPSLOCK, "Caps Lock", caps_lock_on);

        let h_ime_menu = CreatePopupMenu().unwrap_or_default();
        if !h_ime_menu.is_invalid() {
            append_menu_item_checked(h_ime_menu, IDM_IME_HIRAGANA, "ひらがな", hiragana_checked);
            append_menu_item_checked(
                h_ime_menu,
                IDM_IME_FULL_KATAKANA,
                "全角カタカナ",
                full_katakana_checked,
            );
            append_menu_item_checked(
                h_ime_menu,
                IDM_IME_FULL_ALPHA,
                "全角英数",
                full_alpha_checked,
            );
            append_menu_item_checked(
                h_ime_menu,
                IDM_IME_HALF_ALPHA,
                "半角英数",
                half_alpha_checked,
            );
            append_menu_item_checked(
                h_ime_menu,
                IDM_IME_HALF_KATAKANA,
                "半角カタカナ",
                half_katakana_checked,
            );
            append_menu_item_checked(h_ime_menu, IDM_IME_DIRECT, "直接入力", direct_checked);

            let ime_title_wide = crate::win32::to_wide("IME 状態");
            let _ = AppendMenuW(
                hmenu,
                MF_POPUP,
                h_ime_menu.0 as usize,
                PCWSTR(ime_title_wide.as_ptr()),
            );
        }

        let h_input_menu = CreatePopupMenu().unwrap_or_default();
        if !h_input_menu.is_invalid() {
            append_menu_item_checked(h_input_menu, IDM_INPUT_ROMAJI, "ローマ字入力", is_romaji);
            append_menu_item_checked(h_input_menu, IDM_INPUT_KANA, "かな入力", !is_romaji);

            let input_title_wide = crate::win32::to_wide("JISかな / ローマ字");
            let _ = AppendMenuW(
                hmenu,
                MF_POPUP,
                h_input_menu.0 as usize,
                PCWSTR(input_title_wide.as_ptr()),
            );
        }

        append_menu_item(
            hmenu,
            IDM_RESET_STATE,
            "状態をリセット (Caps OFF/ひらがな/ローマ字)",
        );

        append_menu_sep(hmenu);

        append_menu_item(hmenu, IDM_SETTINGS, "設定...");
        append_menu_item(hmenu, IDM_CLEAR_IMM_CACHE, "学習キャッシュをクリア");
        append_menu_item(hmenu, IDM_RESTART, "再起動");
        let autostart_registered = crate::autostart::is_registered();
        append_menu_item_checked(
            hmenu,
            IDM_AUTOSTART,
            "ログオン時に自動起動",
            autostart_registered,
        );
        if !elevated {
            append_menu_item(hmenu, IDM_RESTART_ADMIN, "管理者として再起動");
        }

        append_menu_sep(hmenu);
        append_menu_item(hmenu, IDM_TOGGLE, "有効/無効切替");
        append_menu_item(hmenu, IDM_EXIT, "終了");

        // メニュー表示前にウィンドウをフォアグラウンドにする（メニューが閉じるために必要）
        let _ = SetForegroundWindow(hwnd);

        let _ = TrackPopupMenu(
            hmenu,
            TPM_LEFTALIGN | TPM_BOTTOMALIGN,
            point.x,
            point.y,
            Some(0),
            hwnd,
            None,
        );

        let _ = DestroyMenu(hmenu);
    }
}

/// `WM_COMMAND` の `WPARAM` からトレイコマンドを解釈する。
#[must_use]
pub fn handle_tray_command(wparam: WPARAM) -> Option<TrayCommand> {
    let cmd = (wparam.0 & 0xFFFF) as u16;
    match cmd {
        IDM_TOGGLE => Some(TrayCommand::Toggle),
        IDM_EXIT => Some(TrayCommand::Exit),
        IDM_SETTINGS => Some(TrayCommand::Settings),
        IDM_RESTART_ADMIN => Some(TrayCommand::RestartAdmin),
        IDM_CLEAR_IMM_CACHE => Some(TrayCommand::ClearImmCache),
        IDM_AUTOSTART => Some(TrayCommand::ToggleAutoStart),
        IDM_RESTART => Some(TrayCommand::Restart),
        IDM_CAPSLOCK => Some(TrayCommand::CapsLock),
        IDM_IME_HIRAGANA => Some(TrayCommand::ImeHiragana),
        IDM_IME_FULL_KATAKANA => Some(TrayCommand::ImeFullKatakana),
        IDM_IME_FULL_ALPHA => Some(TrayCommand::ImeFullAlpha),
        IDM_IME_HALF_ALPHA => Some(TrayCommand::ImeHalfAlpha),
        IDM_IME_HALF_KATAKANA => Some(TrayCommand::ImeHalfKatakana),
        IDM_IME_DIRECT => Some(TrayCommand::ImeDirect),
        IDM_INPUT_ROMAJI => Some(TrayCommand::InputRomaji),
        IDM_INPUT_KANA => Some(TrayCommand::InputKana),
        IDM_RESET_STATE => Some(TrayCommand::ResetState),
        c if (IDM_LAYOUT_BASE..IDM_CAPSLOCK).contains(&c) => {
            Some(TrayCommand::SelectLayout(usize::from(c - IDM_LAYOUT_BASE)))
        }
        _ => None,
    }
}

/// 現在のプロセスが管理者権限で実行中かどうかを判定する。
///
/// `shell32.dll` の `IsUserAnAdmin` を使用する。
/// この API は非推奨だが、シンプルで `Win32_Security` feature を追加せずに使えるため採用。
#[must_use]
pub fn is_elevated() -> bool {
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn IsUserAnAdmin() -> i32;
    }
    // SAFETY: `IsUserAnAdmin` は shell32.dll にリンクされた有効な外部関数。
    //         引数なしで呼べる純粋なクエリ API であり副作用はない。
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

    let exe_wide = crate::win32::to_wide(&exe.to_string_lossy());
    let verb = crate::win32::to_wide("runas");

    // SAFETY: `exe_wide` と `verb` は直上で NUL 終端済みの有効な UTF-16 文字列。
    //         `PCWSTR` ポインタは `ShellExecuteW` 呼び出し中はスタック上に生存している。
    unsafe {
        let result = ShellExecuteW(
            None,
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

/// 通常権限で自身を再起動する。
///
/// 現在の実行ファイルを新しいプロセスとして spawn し、成功したら現在のプロセスを終了する。
pub fn restart_self() {
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            log::error!("Failed to get current exe path: {e}");
            return;
        }
    };
    match std::process::Command::new(&exe).spawn() {
        Ok(_) => {
            log::info!("Restarting self, exiting current process");
            std::process::exit(0);
        }
        Err(e) => {
            log::error!("Failed to restart self: {e}");
        }
    }
}

/// 設定画面 (awase-settings) を起動する。
/// 実行ファイルと同じディレクトリにある awase-settings.exe を探す。
/// 失敗時はバルーン通知でユーザーに知らせる。
fn launch_settings_gui() {
    let Ok(exe) = std::env::current_exe() else {
        show_settings_error("実行ファイルのパスを取得できません");
        return;
    };
    let Some(dir) = exe.parent() else {
        show_settings_error("実行ファイルのディレクトリを取得できません");
        return;
    };
    let path = dir.join("awase-settings.exe");
    if !path.exists() {
        let msg = format!("設定画面が見つかりません:\n{}", path.display());
        show_settings_error(&msg);
        return;
    }
    match std::process::Command::new(&path).spawn() {
        Ok(_) => log::info!("awase-settings launched: {}", path.display()),
        Err(e) => {
            let msg = format!("設定画面の起動に失敗しました:\n{e}");
            show_settings_error(&msg);
        }
    }
}

/// 設定画面起動エラーをログとバルーン通知で表示する。
fn show_settings_error(msg: &str) {
    log::error!("Settings launch: {msg}");
    let _ = crate::with_app(|app| app.show_tray_balloon("awase", msg));
}

/// 自動起動のトグル処理。
///
/// 現在の登録状態を確認し、登録 → 解除、解除 → 登録 を切り替える。
/// 結果を config.toml に保存し、バルーン通知で知らせる。
pub(crate) fn handle_autostart_toggle() {
    use crate::autostart;

    let is_registered = autostart::is_registered();
    let (success, new_value, msg) = if is_registered {
        (
            autostart::unregister(),
            "disabled",
            "自動起動を無効にしました",
        )
    } else {
        (autostart::register(), "enabled", "自動起動を有効にしました")
    };

    if success {
        save_auto_start_config(new_value);
        let _ = crate::with_app(|app| {
            app.show_tray_balloon("awase", msg);
        });
    }
}

/// config.toml の `auto_start` 値を書き換えて保存する。
fn save_auto_start_config(value: &str) {
    let Ok(config_path) = crate::app::find_config_path() else {
        log::warn!("Could not find config path to save auto_start");
        return;
    };
    match awase::config::AppConfig::load(&config_path) {
        Ok(mut config) => {
            config.general.auto_start = value.to_string();
            if let Err(e) = config.save(&config_path) {
                log::error!("Failed to save auto_start config: {e}");
            }
        }
        Err(e) => log::error!("Failed to load config for saving auto_start: {e}"),
    }
}

/// トレイウィンドウプロシージャ
///
/// Shell はトレイコールバックメッセージ（WM_TRAY_CALLBACK）をこのウィンドウに
/// 直接送信する。メッセージループの `match msg.message` には到達しないため、
/// ここで処理してメインスレッドの WM_APP / WM_COMMAND に転送する。
unsafe extern "system" fn tray_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_TRAY_CALLBACK => {
            // Shell からのトレイコールバック → メインのメッセージループに転送
            // PostMessage ではなく直接処理する（同じスレッドなので安全）
            let layout_names: Vec<String> =
                crate::with_app_ref(crate::Runtime::layout_names).unwrap_or_default();
            let elevated = crate::is_elevated();
            handle_tray_message(hwnd, lparam, &layout_names, elevated);
            LRESULT(0)
        }
        WM_COMMAND => {
            match handle_tray_command(wparam) {
                Some(TrayCommand::Exit) => PostQuitMessage(0),
                Some(TrayCommand::Toggle) => {
                    let _ = crate::with_app(super::runtime::Runtime::toggle_engine);
                }
                Some(TrayCommand::Settings) => launch_settings_gui(),
                Some(TrayCommand::ClearImmCache) => {
                    let _ = crate::with_app(|app| {
                        let count = app.clear_imm_learning();
                        log::info!("IMM capability cache cleared ({count} entries)");
                        app.show_tray_balloon(
                            "awase",
                            &format!("学習キャッシュをクリアしました（{count}件）"),
                        );
                    });
                }
                Some(TrayCommand::RestartAdmin) => restart_as_admin(),
                Some(TrayCommand::ToggleAutoStart) => handle_autostart_toggle(),
                Some(TrayCommand::Restart) => restart_self(),
                Some(TrayCommand::SelectLayout(index)) => {
                    let _ = crate::with_app(|app| app.switch_layout(index));
                }
                Some(TrayCommand::CapsLock) => unsafe {
                    crate::ime::toggle_caps_lock();
                },
                Some(TrayCommand::ImeHiragana) => unsafe {
                    let _ = crate::ime::set_ime_mode(
                        true,
                        crate::imm::IME_CMODE_NATIVE | crate::imm::IME_CMODE_FULLSHAPE,
                        crate::imm::IME_CMODE_KATAKANA,
                    );
                },
                Some(TrayCommand::ImeFullKatakana) => unsafe {
                    let _ = crate::ime::set_ime_mode(
                        true,
                        crate::imm::IME_CMODE_NATIVE
                            | crate::imm::IME_CMODE_KATAKANA
                            | crate::imm::IME_CMODE_FULLSHAPE,
                        0,
                    );
                },
                Some(TrayCommand::ImeFullAlpha) => unsafe {
                    let _ = crate::ime::set_ime_mode(
                        true,
                        crate::imm::IME_CMODE_FULLSHAPE,
                        crate::imm::IME_CMODE_NATIVE | crate::imm::IME_CMODE_KATAKANA,
                    );
                },
                Some(TrayCommand::ImeHalfAlpha) => unsafe {
                    let _ = crate::ime::set_ime_mode(
                        true,
                        0,
                        crate::imm::IME_CMODE_NATIVE
                            | crate::imm::IME_CMODE_KATAKANA
                            | crate::imm::IME_CMODE_FULLSHAPE,
                    );
                },
                Some(TrayCommand::ImeHalfKatakana) => unsafe {
                    let _ = crate::ime::set_ime_mode(
                        true,
                        crate::imm::IME_CMODE_NATIVE | crate::imm::IME_CMODE_KATAKANA,
                        crate::imm::IME_CMODE_FULLSHAPE,
                    );
                },
                Some(TrayCommand::ImeDirect) => unsafe {
                    let _ = crate::ime::set_ime_mode(false, 0, 0);
                },
                Some(TrayCommand::InputRomaji) => unsafe {
                    let _ = crate::ime::set_ime_romaji_mode_state(true);
                },
                Some(TrayCommand::InputKana) => unsafe {
                    let _ = crate::ime::set_ime_romaji_mode_state(false);
                },
                Some(TrayCommand::ResetState) => unsafe {
                    let caps_lock_on =
                        windows::Win32::UI::Input::KeyboardAndMouse::GetKeyState(0x14) & 1 != 0;
                    if caps_lock_on {
                        crate::ime::toggle_caps_lock();
                    }
                    let _ = crate::ime::set_ime_mode(
                        true,
                        crate::imm::IME_CMODE_NATIVE
                            | crate::imm::IME_CMODE_FULLSHAPE
                            | crate::imm::IME_CMODE_ROMAN,
                        crate::imm::IME_CMODE_KATAKANA,
                    );
                },
                None => {}
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            // DefWindowProcW は WM_CLOSE を DestroyWindow → WM_DESTROY → PostQuitMessage に
            // 変換してしまうため、意図しない Alt+F4 等によるシャットダウンを防ぐ。
            // トレイウィンドウは常に非表示であり、明示的な終了操作（トレイメニュー "終了"）
            // 以外では閉じるべきでない。
            log::warn!("Tray window received unexpected WM_CLOSE — ignoring to prevent accidental shutdown");
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
