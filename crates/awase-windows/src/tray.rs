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
    TrackPopupMenu, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, HMENU, ICONINFO, MF_CHECKED,
    MF_SEPARATOR, MF_STRING, SW_SHOWNORMAL, TPM_BOTTOMALIGN, TPM_LEFTALIGN, WM_CLOSE, WM_COMMAND,
    WM_DESTROY, WM_RBUTTONUP, WNDCLASSW, WS_OVERLAPPEDWINDOW,
};

use anyhow::{Context, Result};

use std::sync::atomic::{AtomicIsize, Ordering};

/// コンテキストメニューを表示する直前（`SetForegroundWindow` でトレイ自身に
/// フォーカスを奪う直前）に捕捉した、実際にフォーカスされていたウィンドウ。
///
/// `handle_tray_message` が右クリックのたびに更新し、`handle_wm_command`（メニュー
/// 選択後の `WM_COMMAND`）がこれを読んで IME コマンドの対象ウィンドウとして使う。
/// メニュー選択の時点で `GetGUIThreadInfo` / `GetForegroundWindow` を素朴に問い合わせると
/// トレイ自身やメニューの一時ウィンドウを掴んでしまい、ユーザーが実際に入力していた
/// アプリには何も効かない（2026-07-24 実機: 「IME ON・Engine OFF から何をしても
/// 復旧できない」として観測。トレイの「状態をリセット」がこの経路だった）。
static MENU_TARGET_HWND: AtomicIsize = AtomicIsize::new(0);

/// 直近の `handle_tray_message` 呼び出し時に捕捉したフォーカスウィンドウを返す。
/// 捕捉できていなければ `None`（呼び出し元は生の `GetForegroundWindow()` 等に
/// フォールバックすること）。
#[must_use]
pub(crate) fn menu_target_hwnd() -> Option<HWND> {
    let raw = MENU_TARGET_HWND.load(Ordering::Relaxed);
    (raw != 0).then_some(HWND(raw as *mut core::ffi::c_void))
}

/// トレイメニュー項目 ID
const IDM_SETTINGS: u16 = 50;
const IDM_RESTART_ADMIN: u16 = 51;
const IDM_CLEAR_IMM_CACHE: u16 = 52;
const IDM_AUTOSTART: u16 = 54;
const IDM_RESTART: u16 = 56;
const IDM_ABOUT: u16 = 57;
const IDM_BUG_REPORT: u16 = 58;
const IDM_UPDATE: u16 = 59;
const IDM_TOGGLE: u16 = 1001;
const IDM_EXIT: u16 = 1002;

/// 配列選択メニュー項目のベース ID
const IDM_LAYOUT_BASE: u16 = 100;

/// Caps Lock / IME 状態リセット メニュー項目 ID
const IDM_CAPSLOCK: u16 = 200;
// IDM_IME_HIRAGANA(201)〜IDM_IME_DIRECT(206) は 2026-08-17、ADR-094 で
// charset 軸（ひらがな/カタカナ×全角/半角の追跡）自体を撤去したのに伴い削除。
// IDM_INPUT_ROMAJI(207)/IDM_INPUT_KANA(208) は BUG-61 で撤去。実機確認の結果
// IMC write（ImmSetConversionStatus）が実モードに反映されず「押しても何も
// 変化しない」ことが確認されたため、tray 経由の手動テストハーネスとしては
// 不採用とし、Ctrl+Alt+R / Ctrl+Alt+K の直接ホットキー（フォーカス文脈が
// 確実に正しい）へ切り替えた。詳細は docs/known-bugs.md BUG-61 参照。
const IDM_RESET_STATE: u16 = 209;
const IDM_KANA_LOCK_HELP: u16 = 210;

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
    About,
    BugReport,
    OpenUpdatePage,
    /// 配列選択（インデックスは `IDM_LAYOUT_BASE` からのオフセット）
    SelectLayout(usize),
    CapsLock,
    ResetState,
    KanaLockHelp,
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

/// ウィンドウクラス名（設定 GUI や多重起動検出時の `FindWindowW` 検索用に一定の名前を使う）。
///
/// `crates/awase-settings/src/main.rs::send_reload_config_message()` がこの文字列を
/// **直書きで**参照している（awase-settings は awase-windows に依存しないため定数を
/// 共有できない）。ここを変更したら必ず向こうも合わせて変更すること。過去に
/// この2箇所の文字列が食い違い（"awase_tray_window" vs "awase_msg_window"）、
/// 設定 GUI の「適用」が awase.exe に一切通知されない（無言で失敗する）バグが
/// 長期間気づかれずに残っていた（2026-07-19 実機で発覚・修正）。
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
    /// エンジン有効状態（ツールチップ復元用）
    enabled: bool,
    /// OS かな入力ロック警告中かどうか
    kana_lock_warned: bool,
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
            set_tooltip(&mut nid, enabled, "", elevated, false);

            // トレイアイコンを追加
            // シェル未起動時（ログオン直後等）は失敗しても OK。
            // TaskbarCreated がブロードキャストされた時点で recreate() が呼ばれる。
            if Shell_NotifyIconW(NIM_ADD, &raw const nid).as_bool() {
                log::info!("System tray icon created (elevated={elevated})");
            } else {
                log::warn!("Shell_NotifyIcon NIM_ADD failed — shell not ready, will retry on TaskbarCreated");
            }

            Ok(Self {
                hwnd,
                nid,
                layout_names: Vec::new(),
                current_layout_name: String::new(),
                elevated,
                enabled,
                kana_lock_warned: false,
            })
        }
    }

    /// トレイアイコンのツールチップとアイコンを更新する
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        set_tooltip(
            &mut self.nid,
            enabled,
            &self.current_layout_name,
            self.elevated,
            self.kana_lock_warned,
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

    /// 現在アクティブな配列名を返す（トレイメニューでのチェックマーク表示用）
    #[must_use]
    pub fn current_layout_name(&self) -> &str {
        &self.current_layout_name
    }

    /// 現在の配列名を設定し、ツールチップを更新する
    pub fn set_layout_name(&mut self, name: &str) {
        self.current_layout_name = name.to_string();
        set_tooltip(
            &mut self.nid,
            self.enabled,
            &self.current_layout_name,
            self.elevated,
            self.kana_lock_warned,
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

    /// OS かな入力ロック警告状態をトレイ表示へ反映する。
    pub fn set_kana_lock_warned(&mut self, warned: bool) {
        if self.kana_lock_warned == warned {
            return;
        }
        self.kana_lock_warned = warned;
        set_tooltip(
            &mut self.nid,
            self.enabled,
            &self.current_layout_name,
            self.elevated,
            self.kana_lock_warned,
        );
        // SAFETY: `self.nid` は `new()` で正しく初期化された有効な `NOTIFYICONDATAW`。
        //         `self.hwnd` は生存中の有効なトレイウィンドウハンドル。
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &raw const self.nid);
        }
    }

    #[must_use]
    pub const fn kana_lock_warned(&self) -> bool {
        self.kana_lock_warned
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
fn set_tooltip(
    nid: &mut NOTIFYICONDATAW,
    enabled: bool,
    layout_name: &str,
    elevated: bool,
    kana_lock_warned: bool,
) {
    let admin_suffix = if elevated { " (管理者)" } else { "" };
    let tip = if kana_lock_warned {
        format!("awase - かな入力になっています{admin_suffix}")
    } else if layout_name.is_empty() {
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
    nid.szTip.fill(0);
    nid.szTip[..len].copy_from_slice(&tip_wide[..len]);
}

/// トレイアイコンイベントを処理する
///
/// `WM_APP` メッセージを受け取った時にメッセージループから呼ばれる。
/// 右クリックでコンテキストメニューを表示する。
pub fn handle_tray_message(
    hwnd: HWND,
    lparam: LPARAM,
    layout_names: &[String],
    current_layout_name: &str,
    elevated: bool,
    kana_lock_warned: bool,
    update_check_enabled: bool,
) {
    #[expect(clippy::cast_sign_loss)]
    let event = (lparam.0 & 0xFFFF) as u32;

    log::debug!(
        "Tray message: event=0x{event:04X} lparam=0x{:016X}",
        lparam.0
    );

    if event != WM_RBUTTONUP {
        return;
    }

    // メニュー表示のための `SetForegroundWindow(hwnd)`（下記）でトレイ自身に
    // フォーカスを奪う前に、実際にフォーカスされているウィンドウを捕捉しておく。
    // ここで捕捉し損ねると、メニュー選択後の IME コマンドがトレイ自身（または
    // メニューの一時ウィンドウ）を対象にしてしまう（`MENU_TARGET_HWND` の doc 参照）。
    // SAFETY: メッセージループスレッドから呼ばれるため Win32 スレッド要件を満たす。
    let captured = unsafe {
        crate::win32::get_gui_thread_info_with_timeout(std::time::Duration::from_millis(150))
    }
    .focused_hwnd;
    MENU_TARGET_HWND.store(captured.map_or(0, |h| h.0 as isize), Ordering::Relaxed);

    // 更新確認が無効なら update_check.json を読みもしない（右クリックのたびに無駄な
    // ファイルI/O + JSONパースが走るのを避ける。display() はどのみち enabled=false を
    // 見た瞬間に state を無視して Disabled を返すだけなので、読む意味が無い）。
    let should_spawn_update_check;
    let update_display = if update_check_enabled {
        let update_state = awase::update_state::load(&awase::update_state::default_path());
        let now = awase::update_state::now_unix();
        should_spawn_update_check = awase::update_state::should_attempt(&update_state, now);
        awase::update_state::display(&update_state, true, env!("CARGO_PKG_VERSION"), now)
    } else {
        should_spawn_update_check = false;
        awase::update_state::Display::Disabled
    };

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

        // 配列選択（現在アクティブなレイアウトにチェックマークを付ける）
        for (i, name) in layout_names.iter().enumerate() {
            let text = crate::win32::to_wide(name);
            let id = usize::from(IDM_LAYOUT_BASE) + i;
            let flags = if name == current_layout_name {
                MF_STRING | MF_CHECKED
            } else {
                MF_STRING
            };
            let _ = AppendMenuW(hmenu, flags, id, PCWSTR(text.as_ptr()));
        }
        if !layout_names.is_empty() {
            append_menu_sep(hmenu);
        }

        if kana_lock_warned {
            append_menu_item(
                hmenu,
                IDM_KANA_LOCK_HELP,
                "⚠ かな入力になっています（対処方法）",
            );
            append_menu_sep(hmenu);
        }

        // Caps Lock
        let caps_lock_on = crate::ime::is_caps_lock_on();
        append_menu_item_checked(hmenu, IDM_CAPSLOCK, "Caps Lock", caps_lock_on);

        // 「IME 状態」サブメニュー（ひらがな/全角カタカナ/全角英数/半角英数/
        // 半角カタカナ/直接入力の charset 選択）は 2026-08-17、ADR-094 で
        // charset 軸の追跡自体を撤去したのに伴い削除した。
        // 「JISかな / ローマ字」submenu は BUG-61 で撤去（IDM_INPUT_ROMAJI/KANA の
        // コメント参照）。

        append_menu_item(
            hmenu,
            IDM_RESET_STATE,
            "状態をリセット (Engine ON/Caps OFF/ひらがな)",
        );

        // 実験: cold warmup（F2送信/probe待機/捨て駒スキップ、per-VK confirm）は
        // 2026-07-18 に実機ソークを経て恒久化・撤去した。トレイの on/off トグルは
        // 不要になったため削除した（docs/known-bugs.md 参照）。

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
        append_menu_item(hmenu, IDM_ABOUT, "awase について");
        if let awase::update_state::Display::Available { version, .. } = update_display {
            append_menu_item(
                hmenu,
                IDM_UPDATE,
                &format!("新しいバージョン {version} があります..."),
            );
        }
        append_menu_item(hmenu, IDM_BUG_REPORT, "不具合を報告...");
        append_menu_item(hmenu, IDM_TOGGLE, "有効/無効切替");
        append_menu_item(hmenu, IDM_EXIT, "終了");

        // メニュー表示前にウィンドウをフォアグラウンドにする（メニューが閉じるために必要）
        let _ = SetForegroundWindow(hwnd);

        {
            let _modal_guard = crate::runtime::engine_window::ModalPumpGuard::enter();
            let _ = TrackPopupMenu(
                hmenu,
                TPM_LEFTALIGN | TPM_BOTTOMALIGN,
                point.x,
                point.y,
                Some(0),
                hwnd,
                None,
            );
        }

        let _ = DestroyMenu(hmenu);
    }

    // メニュー表示・選択が終わった後にspawnする。結果は今回のメニューには
    // 反映されない（ユーザー決定どおり、次回以降の右クリックに反映される）ので
    // 前倒しして得るものが無い一方、CreateProcessW の同期コストをメニュー表示の
    // 待ち時間から外せる（AVのオンアクセススキャンが重い環境で効く）。
    if should_spawn_update_check {
        crate::app::launch_settings_with_args(["--check-update".to_owned()]);
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
        IDM_ABOUT => Some(TrayCommand::About),
        IDM_BUG_REPORT => Some(TrayCommand::BugReport),
        IDM_UPDATE => Some(TrayCommand::OpenUpdatePage),
        IDM_CAPSLOCK => Some(TrayCommand::CapsLock),
        IDM_RESET_STATE => Some(TrayCommand::ResetState),
        IDM_KANA_LOCK_HELP => Some(TrayCommand::KanaLockHelp),
        c if (IDM_LAYOUT_BASE..IDM_CAPSLOCK).contains(&c) => {
            Some(TrayCommand::SelectLayout(usize::from(c - IDM_LAYOUT_BASE)))
        }
        _ => None,
    }
}

/// かな入力ロックの解除案内ダイアログを表示する。
pub fn show_kana_lock_help_dialog() {
    let text = "\
IMEが「かな入力」モードになっています。
この状態では、awaseが送るローマ字キーがJISかな配列として
解釈され、意図しない文字（例:「な」「とに」）が入力されます。

awaseからこのモードを元に戻すことはできません
（Windowsにプログラムから変更する手段が提供されていないため）。
お手数ですが、次のいずれかの操作でローマ字入力に戻してください。

【Microsoft IME】
 ・タスクバーの「あ」/「A」を右クリック →
   「ローマ字入力/かな入力」→「ローマ字入力」

【Google 日本語入力】
 ・タスクバーのアイコンを右クリック →「プロパティ」→
   「一般」タブ →「入力方式」を「ローマ字入力」に

いますぐWindowsのIME設定画面を開きますか？"
        .to_string();
    // check_and_warn（MS-IMEキー割り当て競合）と同一構造のダイアログなので
    // 共有ヘルパーに委譲する（msime_key_assignment.rs参照）。
    crate::msime_key_assignment::spawn_yes_open_ime_settings_dialog(
        "awase - かな入力モードの検知",
        text,
    );
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

/// awase のホームページ URL。
const HOMEPAGE_URL: &str = "https://awase.cc";

/// バージョン情報ダイアログを表示する。
///
/// 「はい」を選ぶとホームページまたは更新版のリリースページを既定のブラウザで開く。
/// ユーザー要望（2026-07-29: 「about awase」の追加とホームページへのリンク）に
/// 対応するもので、インストール済みファイル名からしかバージョンを確認できな
/// かった状態を解消する。
pub fn show_about_dialog(enabled: bool) {
    use windows::core::{w, PCWSTR};
    use windows::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, IDYES, MB_ICONINFORMATION, MB_SETFOREGROUND, MB_TOPMOST, MB_YESNO,
    };

    let state = awase::update_state::load(&awase::update_state::default_path());
    let now = awase::update_state::now_unix();
    let display = awase::update_state::display(&state, enabled, env!("CARGO_PKG_VERSION"), now);
    let (update_text, yes_url) = match display {
        awase::update_state::Display::Disabled => (
            "更新の自動確認は無効になっています".to_owned(),
            HOMEPAGE_URL.to_owned(),
        ),
        awase::update_state::Display::NeverSucceeded { last_attempt_ago } => {
            let text = last_attempt_ago.map_or_else(
                || "まだ最新版を確認できていません".to_owned(),
                |ago| {
                    format!(
                        "まだ最新版を確認できていません（最後の試行: {}前）",
                        approx_duration(ago)
                    )
                },
            );
            (text, HOMEPAGE_URL.to_owned())
        }
        awase::update_state::Display::NoUpdate { last_success_ago } => (
            format!(
                "更新は見つかりませんでした（最終確認: {}前）",
                approx_duration(last_success_ago)
            ),
            HOMEPAGE_URL.to_owned(),
        ),
        awase::update_state::Display::Available {
            version,
            last_success_ago,
        } => (
            format!(
                "新しいバージョン {version} があります（最終確認: {}前）",
                approx_duration(last_success_ago)
            ),
            awase::version::release_url(&version),
        ),
    };
    let text = format!(
        "awase バージョン {}\n\n{update_text}\n\n{HOMEPAGE_URL}\n\n\
         関連ページをブラウザで開きますか？",
        env!("CARGO_PKG_VERSION"),
    );
    let text_wide = crate::win32::to_wide(&text);

    // SAFETY: text_wide は NUL 終端済み UTF-16 で呼び出し中有効。タイトルは静的リテラル。
    let result = unsafe {
        MessageBoxW(
            None,
            PCWSTR(text_wide.as_ptr()),
            w!("awase について"),
            MB_YESNO | MB_ICONINFORMATION | MB_TOPMOST | MB_SETFOREGROUND,
        )
    };
    if result == IDYES {
        open_url(&yes_url);
    }
}

/// `HOMEPAGE_URL` を既定のブラウザで開く。
fn open_homepage() {
    open_url(HOMEPAGE_URL);
}

/// URLを既定のブラウザで開く。
fn open_url(url: &str) {
    use windows::core::{w, PCWSTR};
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let url_wide = crate::win32::to_wide(url);
    // SAFETY: url_wide は NUL 終端済み UTF-16 で呼び出し中有効。他はすべて静的リテラル。
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(url_wide.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    // ShellExecuteW returns HINSTANCE > 32 on success
    if result.0 as isize > 32 {
        log::info!("Opened URL: {url}");
    } else {
        log::warn!("Failed to open URL {url} (result={result:?})");
    }
}

pub fn open_update_page() {
    let state = awase::update_state::load(&awase::update_state::default_path());
    match state
        .last_seen_latest
        .as_deref()
        .and_then(awase::version::parse)
    {
        Some(version) => open_url(&awase::version::release_url(&version)),
        None => open_homepage(),
    }
}

fn approx_duration(seconds: u64) -> String {
    if seconds < 60 {
        return "約1分".to_owned();
    }
    if seconds < 60 * 60 {
        return format!("約{}分", seconds / 60);
    }
    if seconds < 24 * 60 * 60 {
        return format!("約{}時間", seconds / (60 * 60));
    }
    format!("約{}日", seconds / (24 * 60 * 60))
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
        // /code-review指摘（PR #127、5回目）: save_auto_start_configが
        // Vec<String>を返す実装だと「警告0件で成功」と「読み込み/保存自体が
        // 失敗」がどちらも空Vecになり区別できず、実際には保存に失敗していても
        // 成功バルーンが出てしまっていた。Option<Vec<String>>にし、
        // Noneを保存失敗として明示的に扱う。
        match save_auto_start_config(new_value) {
            Some(warnings) if warnings.is_empty() => {
                let _ = crate::with_app(|app| {
                    app.show_tray_balloon("awase", msg);
                });
            }
            Some(warnings) => {
                // /code-review指摘（PR #127、3回目）: save_auto_start_config
                // がvalidate()を経由するようになったことで、config.toml内の
                // 他の項目（例: 廃止済みconfirm_mode="speculative"、範囲外の
                // simultaneous_threshold_ms等）がauto_start切替のついでに
                // 無言でリセットされうる。以前はlog::warn!だけで、トレイ経由
                // の操作にはコンソールが無くユーザーには実質見えなかった。
                // 既存のバルーン通知機構を使い、警告をユーザーへ可視化する。
                let _ = crate::with_app(|app| {
                    app.show_tray_balloon(
                        "awase — 設定を修正しました",
                        &format!("{msg}\n{}", warnings.join("\n")),
                    );
                });
            }
            None => {
                let _ = crate::with_app(|app| {
                    app.show_tray_balloon(
                        "awase — 保存に失敗しました",
                        "自動起動の設定は変更されましたが、config.tomlへの保存に\
                         失敗しました。ログを確認してください。",
                    );
                });
            }
        }
    }
}

/// config.toml の `auto_start` 値を書き換えて保存する。`Some(warnings)`
/// なら保存に成功（`warnings`は検証で正規化・警告が発生した項目、無ければ
/// 空）、`None`なら読み込み/保存自体が失敗（呼び出し元は成功バルーンを
/// 出してはならない）。
fn save_auto_start_config(value: &str) -> Option<Vec<String>> {
    let Ok(config_path) = crate::app::find_config_path() else {
        log::warn!("Could not find config path to save auto_start");
        return None;
    };
    match awase::config::AppConfig::load(&config_path) {
        Ok(mut config) => {
            config.general.auto_start = value.to_string();
            // /code-review指摘（PR #127）: validate()を経由せず生のconfigを
            // そのまま保存すると、confirm_mode="speculative"のような廃止済み
            // 設定値が正規化されないまま再保存され、トレイのauto_start切替
            // だけを経由するユーザーはこの移行が永久に完了しない。他の保存経路
            // （設定画面のapply_confirmed）と同様、保存前に必ずvalidate()を通す。
            let (validated, warnings) = config.validate();
            for w in &warnings {
                log::warn!("Config validation warning while saving auto_start: {w}");
            }
            let config = awase::config::AppConfig::from(validated);
            if let Err(e) = config.save(&config_path) {
                log::error!("Failed to save auto_start config: {e}");
                return None;
            }
            Some(warnings)
        }
        Err(e) => {
            log::error!("Failed to load config for saving auto_start: {e}");
            None
        }
    }
}

/// トレイウィンドウプロシージャ
///
/// `WM_COMMAND`（`TrackPopupMenu` のメニュー選択確定）は、`TrackPopupMenu`
/// 自身が持つ内部モーダルループが、選択確定時に呼び出し元スレッドの
/// `WndProc` へ同期的に配送する（`GetMessageW` の戻り値としては一切
/// 観測されない）。`WM_TRAY_CALLBACK`（`WM_APP`、Shell からのトレイ通知）
/// も実機では同様に `tray_wnd_proc` にしか届かないことが確認できている
/// （下記 2026-07-27 実機ログ参照。正確な配送機構が sent message なのか
/// 別の経路なのかは未確定だが、少なくとも `GetMessageW` の戻り値経由では
/// 届いていない）。そのため `app::run_message_loop` 側の
/// `match msg.message { WM_APP => ..., WM_COMMAND => ... }` はどちらの
/// メッセージに対しても実際には到達しないコードであり、ここが実際の
/// 到達点になる。
///
/// （2026-07-27 実機: `4508231` で「メインループが先取りするので tray_wnd_proc
/// 側は到達不能」との判断のもとこの2ハンドラを削除したところ、右クリックで
/// コンテキストメニューが一度も表示されなくなった。上記の通り判断が逆で、
/// 実際にはこちらが唯一の到達点だった。ロジックの重複・陳腐化を避けるため
/// `message_handlers::handle_wm_app_tray` / `handle_wm_command` へ委譲する
/// 形で復元し、実機で右クリック→メニュー表示→各項目選択の動作を再確認した。
/// メインループ側の分岐は、配送機構の理解が今後の Windows バージョン等で
/// 崩れた場合のフェイルセーフとして削除せず残す。）
unsafe extern "system" fn tray_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_TRAY_CALLBACK => {
            // SAFETY: メニュー表示のための Win32 UI API 呼び出しは同一スレッド
            //         （トレイウィンドウの所有スレッド）から行われる。
            unsafe {
                crate::runtime::message_handlers::handle_wm_app_tray(hwnd, lparam);
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            // SAFETY: 同上。wparam はメニュー選択時に Win32 が設定した項目 ID。
            unsafe {
                crate::runtime::message_handlers::handle_wm_command(wparam);
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            // トレイウィンドウは常に非表示（WS_VISIBLE なし、ShowWindow も未呼び出し）で
            // フォーカスを持てないため、Alt+F4 の対象にはなり得ない。実際に WM_CLOSE が
            // 届くのは taskkill（/f なし）やタスクマネージャーの「タスクの終了」など、
            // 外部からの明示的な終了要求のみ（2026-07-22 実機ログで確認済み）。
            // これらを正常終了として受理する。
            log::info!("Tray window received WM_CLOSE — shutting down");
            PostQuitMessage(0);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
