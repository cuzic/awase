// Win32 API (フック, SendInput, SetTimer 等) の使用に unsafe が必須
#![allow(unsafe_code)]
// Win32 API の型キャスト (usize → i32 等) は OS の ABI 制約により不可避
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    // SingleThreadCell は &self → &mut T を返すが、シングルスレッド保証下で安全
    clippy::mut_from_ref,
    // コールバック型定義が複雑になるのは Win32 API の設計上避けられない
    clippy::type_complexity
)]

mod hook;
mod ime;
mod output;
mod single_thread_cell;
mod tray;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc;

use anyhow::{Context, Result};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS,
};
use windows::core::{Interface, VARIANT};
use windows::Win32::UI::Accessibility::{
    AccessibleObjectFromWindow, CUIAutomation, IAccessible, IUIAutomation,
    IUIAutomationElement, SetWinEventHook, HWINEVENTHOOK,
    UIA_ButtonControlTypeId, UIA_ComboBoxControlTypeId, UIA_DocumentControlTypeId,
    UIA_EditControlTypeId, UIA_HyperlinkControlTypeId, UIA_ImageControlTypeId,
    UIA_ListItemControlTypeId, UIA_MenuBarControlTypeId, UIA_MenuControlTypeId,
    UIA_MenuItemControlTypeId, UIA_ProgressBarControlTypeId, UIA_ScrollBarControlTypeId,
    UIA_SeparatorControlTypeId, UIA_SliderControlTypeId, UIA_StatusBarControlTypeId,
    UIA_TabControlTypeId, UIA_TabItemControlTypeId, UIA_TextControlTypeId,
    UIA_TitleBarControlTypeId, UIA_ToolBarControlTypeId, UIA_TreeItemControlTypeId,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Input::Ime::{ImmGetContext, ImmReleaseContext};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetClassNameW, GetMessageW, GetWindowLongW, GWL_EXSTYLE, GWL_STYLE,
    KillTimer, PostMessageW, PostQuitMessage, SetTimer, MSG, WM_APP, WM_COMMAND, WM_HOTKEY,
    WM_INPUTLANGCHANGE, WM_TIMER,
};

use awase::config::{vk_name_to_code, AppConfig};
use awase::engine::{ContextInvalidation, Engine, EngineBypassState, TIMER_PENDING, TIMER_SPECULATIVE};
use awase::ngram::NgramModel;
use awase::types::{KeyEventType, RawKeyEvent};
use awase::yab::YabLayout;
use timed_fsm::{dispatch, ActionExecutor, TimedStateMachine, TimerRuntime};

use crate::hook::CallbackResult;
use crate::ime::{HybridProvider, ImeProvider};
use crate::output::Output;
use crate::single_thread_cell::SingleThreadCell;
use crate::tray::SystemTray;

/// 有効/無効切り替えホットキー ID
const HOTKEY_ID_TOGGLE: i32 = 1;

/// 設定リロード用カスタムメッセージ（設定 GUI から `PostMessageW` で送信される）
const WM_RELOAD_CONFIG: u32 = WM_APP + 10;

/// IME 制御キー後の遅延キー再処理用カスタムメッセージ
const WM_PROCESS_DEFERRED: u32 = WM_APP + 11;

/// UIA 非同期判定完了通知用カスタムメッセージ
const WM_UIA_BYPASS_UPDATE: u32 = WM_APP + 12;

/// IME の未確定状態を確認する（engine から呼び出し用）
#[allow(dead_code)]
pub fn check_ime_composing() -> bool {
    unsafe { IME.get_ref().map_or(false, |ime| ime.is_composing()) }
}

static ENGINE: SingleThreadCell<Engine> = SingleThreadCell::new();
static OUTPUT: SingleThreadCell<Output> = SingleThreadCell::new();
static IME: SingleThreadCell<HybridProvider> = SingleThreadCell::new();
static TRAY: SingleThreadCell<SystemTray> = SingleThreadCell::new();

/// 利用可能な配列の一覧（名前, `YabLayout`, 左親指VK, 右親指VK）
static LAYOUTS: SingleThreadCell<Vec<(String, YabLayout, u16, u16)>> = SingleThreadCell::new();

/// IME 制御キー直後のガードフラグ（true: 後続キーを遅延処理する）
static IME_GUARD: SingleThreadCell<bool> = SingleThreadCell::new();

/// ガード中に遅延されたキーイベントのバッファ
static DEFERRED_KEYS: SingleThreadCell<Vec<RawKeyEvent>> = SingleThreadCell::new();

/// UIA 非同期判定リクエスト送信チャネル（メインスレッドからのみアクセス）
static UIA_SENDER: SingleThreadCell<mpsc::Sender<SendableHwnd>> = SingleThreadCell::new();

/// `HWND` を `Send` 可能にするラッパー
///
/// `HWND` は `*mut c_void` を含むため `Send` を実装していないが、
/// ウィンドウハンドルの値自体はスレッド間で安全に受け渡せる。
/// UIA ワーカースレッドへの HWND 送信専用。
#[derive(Clone, Copy)]
struct SendableHwnd(HWND);
// Safety: HWND の値（ポインタ値）はスレッド間で安全に共有できる。
// ウィンドウハンドルはプロセス内でグローバルに有効であり、
// 別スレッドから参照しても問題ない。
unsafe impl Send for SendableHwnd {}

/// Ctrl+C 受信フラグ
static QUIT_REQUESTED: AtomicBool = AtomicBool::new(false);

/// フォーカス中コントロールのバイパス判定キャッシュ（Unknown=2 で安全側初期化）
static BYPASS_STATE: AtomicU8 = AtomicU8::new(EngineBypassState::Unknown as u8);

/// `WINEVENT_OUTOFCONTEXT` (0x0000) — コールバックをメッセージループで実行
const WINEVENT_OUTOFCONTEXT: u32 = 0x0000;

/// `EVENT_OBJECT_FOCUS` (0x8005) — フォーカス変更イベント
const EVENT_OBJECT_FOCUS: u32 = 0x8005;

/// `WS_EX_NOIME` (0x0040_0000) — IME 入力を受け付けないウィンドウスタイル
const WS_EX_NOIME: i32 = 0x0040_0000;

/// `ES_READONLY` (0x0800) — 読み取り専用 Edit コントロール
const ES_READONLY: i32 = 0x0800;

fn main() -> Result<()> {
    init_logging();
    let config = load_config()?;
    let (layout_names, initial_layout_name) = init_engine(&config)?;
    init_ime();
    init_ngram(&config);

    unsafe {
        OUTPUT.set(Output::new());
        IME_GUARD.set(false);
        DEFERRED_KEYS.set(Vec::new());
    }

    init_tray(&layout_names, &initial_layout_name)?;
    install_hooks_and_hotkeys(&config)?;

    log::info!("Hook installed. Running message loop...");
    log::info!("Press Ctrl+C to exit.");
    install_ctrl_handler();
    install_focus_hook();

    // Phase 3: UIA 非同期判定ワーカースレッドを起動
    let uia_tx = spawn_uia_worker();
    unsafe {
        UIA_SENDER.set(uia_tx);
    }

    run_message_loop();
    cleanup();

    Ok(())
}

/// ログ初期化
fn init_logging() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .init();
    log::info!("Keyboard Layout Emulator starting...");
}

/// 設定ファイルを読み込む
fn load_config() -> Result<AppConfig> {
    let config_path = find_config_path()?;
    log::info!("Loading config from: {}", config_path.display());
    let config = AppConfig::load(&config_path)?;
    log::info!(
        "Default layout: {}, Threshold: {}ms",
        config.general.default_layout,
        config.general.simultaneous_threshold_ms
    );
    Ok(config)
}

/// 配列の読み込みとエンジン初期化を行い、レイアウト名一覧とデフォルト名を返す
fn init_engine(config: &AppConfig) -> Result<(Vec<String>, String)> {
    let left_thumb_vk = vk_name_to_code(&config.general.left_thumb_key).context(format!(
        "Unknown VK name: {}",
        config.general.left_thumb_key
    ))?;
    let right_thumb_vk = vk_name_to_code(&config.general.right_thumb_key).context(format!(
        "Unknown VK name: {}",
        config.general.right_thumb_key
    ))?;

    let layouts_dir = resolve_relative(&config.general.layouts_dir);
    let layouts = scan_layouts(&layouts_dir, left_thumb_vk, right_thumb_vk)?;
    let layout_names: Vec<String> = layouts.iter().map(|(name, ..)| name.clone()).collect();
    log::info!("Available layouts: {layout_names:?}");

    let (layout, initial_layout_name) = select_default_layout(&layouts, config);
    log::info!(
        "Layout loaded: {} normal keys, {} left thumb keys, {} right thumb keys",
        layout.normal.len(),
        layout.left_thumb.len(),
        layout.right_thumb.len()
    );

    unsafe {
        ENGINE.set(Engine::new(
            layout,
            left_thumb_vk,
            right_thumb_vk,
            config.general.simultaneous_threshold_ms,
            config.general.confirm_mode,
            config.general.speculative_delay_ms,
        ));
        LAYOUTS.set(layouts);
    }

    Ok((layout_names, initial_layout_name))
}

/// デフォルトレイアウトを選択し、YabLayout とレイアウト名を返す
fn select_default_layout(
    layouts: &[(String, YabLayout, u16, u16)],
    config: &AppConfig,
) -> (YabLayout, String) {
    let default_name = config.general.default_layout.trim_end_matches(".yab");
    let index = layouts
        .iter()
        .position(|(name, ..)| name == default_name)
        .unwrap_or(0);
    let (ref name, ref layout, _, _) = layouts[index];
    let copied = YabLayout {
        name: layout.name.clone(),
        normal: layout.normal.clone(),
        left_thumb: layout.left_thumb.clone(),
        right_thumb: layout.right_thumb.clone(),
        shift: layout.shift.clone(),
    };
    (copied, name.clone())
}

/// IME プロバイダ初期化（TSF 優先、IMM32 フォールバック）
fn init_ime() {
    let ime_provider = HybridProvider::new();
    log::info!(
        "IME provider initialized. Japanese keyboard: {}",
        ime::is_japanese_input_language()
    );
    unsafe {
        IME.set(ime_provider);
    }
}

/// n-gram モデルのロード（オプション）
fn init_ngram(config: &AppConfig) {
    let Some(ref ngram_path) = config.general.ngram_file else {
        return;
    };
    let ngram_path = resolve_relative(ngram_path);
    let content = match std::fs::read_to_string(&ngram_path) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("Failed to read n-gram file {}: {e}", ngram_path.display());
            return;
        }
    };
    let base_us = u64::from(config.general.simultaneous_threshold_ms) * 1000;
    let range_us = u64::from(config.general.ngram_adjustment_range_ms) * 1000;
    let min_us = u64::from(config.general.ngram_min_threshold_ms) * 1000;
    let max_us = u64::from(config.general.ngram_max_threshold_ms) * 1000;
    match NgramModel::from_toml(&content, base_us, range_us, min_us, max_us) {
        Ok(model) => {
            log::info!("N-gram model loaded from {}", ngram_path.display());
            unsafe {
                if let Some(engine) = ENGINE.get_mut() {
                    engine.set_ngram_model(model);
                }
            }
        }
        Err(e) => log::warn!("Failed to parse n-gram model: {e}"),
    }
}

/// システムトレイアイコンを作成する
fn init_tray(layout_names: &[String], initial_layout_name: &str) -> Result<()> {
    let mut system_tray = SystemTray::new(true).context("Failed to create system tray icon")?;
    system_tray.set_layout_names(layout_names.to_vec());
    system_tray.set_layout_name(initial_layout_name);
    unsafe {
        TRAY.set(system_tray);
    }
    Ok(())
}

/// フック登録とホットキー登録を行う
fn install_hooks_and_hotkeys(config: &AppConfig) -> Result<()> {
    let callback = Box::new(|event: RawKeyEvent| -> CallbackResult {
        unsafe { on_key_event_callback(event) }
    });
    hook::install_hook(callback).context("Failed to install keyboard hook")?;

    if let Some(ref hotkey_str) = config.general.toggle_hotkey {
        register_toggle_hotkey(hotkey_str);
    }
    Ok(())
}

/// トグルホットキーを登録する
fn register_toggle_hotkey(hotkey_str: &str) {
    if let Some((modifiers, vk)) = awase::config::parse_hotkey(hotkey_str) {
        unsafe {
            let result = RegisterHotKey(
                HWND::default(),
                HOTKEY_ID_TOGGLE,
                HOT_KEY_MODIFIERS(modifiers),
                u32::from(vk),
            );
            if result.is_ok() {
                log::info!("Toggle hotkey registered: {hotkey_str}");
            } else {
                log::warn!("Failed to register toggle hotkey: {hotkey_str}");
            }
        }
    } else {
        log::warn!("Invalid toggle hotkey format: {hotkey_str}");
    }
}

/// クリーンアップ処理
fn cleanup() {
    hook::uninstall_hook();
    unsafe {
        let _ = UnregisterHotKey(HWND::default(), HOTKEY_ID_TOGGLE);
    }
    unsafe {
        if let Some(tray) = TRAY.get_mut() {
            tray.destroy();
        }
        TRAY.clear();
    }
    unsafe {
        ENGINE.clear();
        OUTPUT.clear();
        IME.clear();
        LAYOUTS.clear();
    }
    log::info!("Exited cleanly.");
}

/// Win32 タイマーランタイム
struct Win32TimerRuntime;

impl TimerRuntime for Win32TimerRuntime {
    type TimerId = usize;

    fn set_timer(&mut self, id: Self::TimerId, duration: std::time::Duration) {
        let ms = u32::try_from(duration.as_millis()).unwrap_or(u32::MAX);
        unsafe {
            let _ = SetTimer(HWND::default(), id, ms, None);
        }
    }

    fn kill_timer(&mut self, id: Self::TimerId) {
        unsafe {
            let _ = KillTimer(HWND::default(), id);
        }
    }
}

/// `SendInput` アクション実行器
struct SendInputExecutor;

impl ActionExecutor for SendInputExecutor {
    type Action = awase::types::KeyAction;

    fn execute(&mut self, actions: &[Self::Action]) {
        unsafe {
            if let Some(output) = OUTPUT.get_ref() {
                output.send_keys(actions);
            }
        }
    }
}

/// フックコールバックからの Engine 呼び出し
unsafe fn on_key_event_callback(event: RawKeyEvent) -> CallbackResult {
    let is_key_down = matches!(
        event.event_type,
        KeyEventType::KeyDown | KeyEventType::SysKeyDown
    );

    // ── IME ガード: IME 制御キー直後の後続キーを遅延処理する ──
    // IME 制御キー（半角/全角等）が OS に渡された直後は、IME 状態がまだ反映されて
    // いない可能性がある。後続キーをメッセージループに回すことで、IME 状態が
    // 確実に更新された後に処理する。
    if let Some(guard) = IME_GUARD.get_mut() {
        if *guard {
            // IME 制御キーの KeyUp でガード解除
            if !is_key_down && Engine::is_ime_control_vk(event.vk_code) {
                *guard = false;
                return CallbackResult::PassThrough;
            }
            // KeyDown はバッファに保存してメッセージループに回す
            if is_key_down {
                if let Some(buf) = DEFERRED_KEYS.get_mut() {
                    buf.push(event);
                    let _ = PostMessageW(HWND::default(), WM_PROCESS_DEFERRED, WPARAM(0), LPARAM(0));
                }
                return CallbackResult::Consumed;
            }
            // ガード中の KeyUp（IME制御キー以外）はパススルー
            return CallbackResult::PassThrough;
        }
    }

    // ── IME 制御キーの検出: ガードを有効にしてパススルー ──
    if is_key_down && Engine::is_ime_control_vk(event.vk_code) {
        // エンジンの保留をフラッシュ（engine 側の handle_bypass で実行される）
        if let Some(engine) = ENGINE.get_mut() {
            let response = engine.on_event(event);
            let mut timer_runtime = Win32TimerRuntime;
            let mut action_executor = SendInputExecutor;
            dispatch(&response, &mut timer_runtime, &mut action_executor);
            // engine が passthrough を返す → ガードを有効にして OS に渡す
            if !response.consumed {
                if let Some(guard) = IME_GUARD.get_mut() {
                    *guard = true;
                }
            }
        }
        // consumed=false の場合は OS にそのまま渡す（CallbackResult::PassThrough）
        // consumed=true の場合はエンジンが処理済み（通常ありえないがsafety）
        return CallbackResult::PassThrough;
    }

    // ── 通常のキー処理 ──
    let Some(engine) = ENGINE.get_mut() else {
        return CallbackResult::PassThrough;
    };

    // Bypass判定: 非テキストコントロール（Button, Static 等）ではエンジンをバイパス。
    // Unknown（判定不能）は AllowEngine と同じ扱い（deny リスト方式）。
    // Phase 2/3 (MSAA/UIA) で Unknown の解像度が上がったら allow リスト方式に切替可能。
    let bypass = BYPASS_STATE.load(Ordering::Acquire);
    if bypass == EngineBypassState::ShouldBypass as u8 {
        return CallbackResult::PassThrough;
    }

    // IME OFF / 英数モード → PassThrough（エンジンバイパス）
    if let Some(ime_provider) = IME.get_ref() {
        if !ime_provider.is_active() || !ime_provider.get_mode().is_kana_input() {
            return CallbackResult::PassThrough;
        }
    }

    let response = engine.on_event(event);
    let mut timer_runtime = Win32TimerRuntime;
    let mut action_executor = SendInputExecutor;
    let consumed = dispatch(&response, &mut timer_runtime, &mut action_executor);

    if consumed {
        CallbackResult::Consumed
    } else {
        CallbackResult::PassThrough
    }
}

/// フォーカス変更イベントフックを登録する
///
/// `WINEVENT_OUTOFCONTEXT` を使用するため、コールバックはメッセージループ上で実行される。
/// これにより `determine_bypass_state` が非同期（キーイベントとは別タイミング）で呼ばれる。
fn install_focus_hook() {
    unsafe {
        let hook = SetWinEventHook(
            EVENT_OBJECT_FOCUS,
            EVENT_OBJECT_FOCUS,
            None,
            Some(win_event_proc),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        );
        if hook.is_invalid() {
            log::warn!("Failed to install focus event hook");
        } else {
            log::info!("Focus event hook installed");
        }
    }
}

/// フォーカス変更イベントのコールバック
///
/// `WINEVENT_OUTOFCONTEXT` により、メッセージループのコンテキストで呼ばれる。
/// フォーカスが移動するたびにバイパス判定を更新し、キャッシュに書き込む。
unsafe extern "system" fn win_event_proc(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    _id_object: i32,
    _id_child: i32,
    _event_thread: u32,
    _event_time: u32,
) {
    if event != EVENT_OBJECT_FOCUS {
        return;
    }

    // Step 1: 評価中は安全側（Unknown → bypass）に設定
    BYPASS_STATE.store(EngineBypassState::Unknown as u8, Ordering::Release);

    // Step 2: バイパス状態を判定
    let state = determine_bypass_state(hwnd);

    // Step 3: キャッシュを更新
    BYPASS_STATE.store(state as u8, Ordering::Release);

    // Step 4: ShouldBypass ならエンジンの保留状態をフラッシュ
    if state == EngineBypassState::ShouldBypass {
        invalidate_engine_context(ContextInvalidation::FocusChanged);
    }

    // Step 5: Phase 1-2 で判定不能なら UIA 非同期判定をリクエスト
    if state == EngineBypassState::Unknown {
        if let Some(tx) = UIA_SENDER.get_ref() {
            let _ = tx.send(SendableHwnd(hwnd));
        }
    }

    log::debug!("Focus changed: hwnd={:?} → {:?}", hwnd, state);
}

/// フォーカス中のウィンドウがテキスト入力を受け付けるかを判定する
///
/// deny-first（バイパスを優先）、allow は確信がある場合のみ。
/// 判定不能なら `Unknown`（安全側＝バイパス）を返す。
unsafe fn determine_bypass_state(hwnd: HWND) -> EngineBypassState {
    if hwnd == HWND::default() {
        return EngineBypassState::ShouldBypass;
    }

    // 1. ImmGetContext == NULL → IME 入力不可
    let himc = ImmGetContext(hwnd);
    if himc.is_invalid() {
        return EngineBypassState::ShouldBypass;
    }
    let _ = ImmReleaseContext(hwnd, himc);

    // 2. WS_EX_NOIME ウィンドウスタイル
    let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE);
    if ex_style & WS_EX_NOIME != 0 {
        return EngineBypassState::ShouldBypass;
    }

    // 3. クラス名による判定
    let mut class_buf = [0u16; 256];
    let len = GetClassNameW(hwnd, &mut class_buf);
    if len > 0 {
        let class_name = String::from_utf16_lossy(&class_buf[..len as usize]);

        // 既知のテキスト入力コントロール
        if matches!(
            class_name.as_str(),
            "Edit"
                | "RichEdit"
                | "RichEdit20A"
                | "RichEdit20W"
                | "RICHEDIT50W"
                | "Scintilla"
                | "ConsoleWindowClass"
        ) {
            // Edit コントロールの読み取り専用チェック
            if class_name == "Edit" {
                let style = GetWindowLongW(hwnd, GWL_STYLE);
                if style & ES_READONLY != 0 {
                    return EngineBypassState::ShouldBypass;
                }
            }
            return EngineBypassState::AllowEngine;
        }

        // 既知の非テキストコントロール
        if matches!(
            class_name.as_str(),
            "Button"
                | "Static"
                | "SysListView32"
                | "SysTreeView32"
                | "SysHeader32"
                | "ToolbarWindow32"
                | "msctls_statusbar32"
                | "SysTabControl32"
                | "msctls_trackbar32"
                | "msctls_progress32"
        ) {
            return EngineBypassState::ShouldBypass;
        }
    }

    // 4. MSAA (IAccessible) role による判定
    {
        /// `OBJID_CLIENT` — クライアント領域のアクセシブルオブジェクト
        const OBJID_CLIENT: i32 = -4;

        let mut acc: *mut std::ffi::c_void = std::ptr::null_mut();
        let ok = AccessibleObjectFromWindow(
            hwnd,
            OBJID_CLIENT as u32,
            &IAccessible::IID,
            &mut acc,
        );
        if ok.is_ok() && !acc.is_null() {
            let accessible: IAccessible = IAccessible::from_raw(acc);
            let child_self = VARIANT::from(0i32); // CHILDID_SELF
            if let Ok(role) = accessible.get_accRole(&child_self) {
                let role_id = role.as_raw().Anonymous.Anonymous.Anonymous.lVal as u32;

                // テキスト入力ロール
                const ROLE_SYSTEM_TEXT: u32 = 42; // 0x2A — editable text
                const ROLE_SYSTEM_DOCUMENT: u32 = 15; // 0x0F — document window

                if matches!(role_id, ROLE_SYSTEM_TEXT | ROLE_SYSTEM_DOCUMENT) {
                    log::debug!("MSAA: role={} → AllowEngine", role_id);
                    return EngineBypassState::AllowEngine;
                }

                // 非テキストロール
                const ROLE_SYSTEM_TITLEBAR: u32 = 1;
                const ROLE_SYSTEM_MENUBAR: u32 = 2;
                const ROLE_SYSTEM_SCROLLBAR: u32 = 3;
                const ROLE_SYSTEM_MENUPOPUP: u32 = 11;
                const ROLE_SYSTEM_MENUITEM: u32 = 12;
                const ROLE_SYSTEM_TOOLBAR: u32 = 22;
                const ROLE_SYSTEM_STATUSBAR: u32 = 23;
                const ROLE_SYSTEM_LIST: u32 = 33;
                const ROLE_SYSTEM_LISTITEM: u32 = 34;
                const ROLE_SYSTEM_OUTLINE: u32 = 35; // tree view
                const ROLE_SYSTEM_OUTLINEITEM: u32 = 36;
                const ROLE_SYSTEM_PAGETAB: u32 = 37;
                const ROLE_SYSTEM_INDICATOR: u32 = 39;
                const ROLE_SYSTEM_GRAPHIC: u32 = 40;
                const ROLE_SYSTEM_STATICTEXT: u32 = 41;
                const ROLE_SYSTEM_PUSHBUTTON: u32 = 43;
                const ROLE_SYSTEM_PROGRESSBAR: u32 = 48;
                const ROLE_SYSTEM_SLIDER: u32 = 51;

                if matches!(
                    role_id,
                    ROLE_SYSTEM_TITLEBAR
                        | ROLE_SYSTEM_MENUBAR
                        | ROLE_SYSTEM_SCROLLBAR
                        | ROLE_SYSTEM_MENUPOPUP
                        | ROLE_SYSTEM_MENUITEM
                        | ROLE_SYSTEM_TOOLBAR
                        | ROLE_SYSTEM_STATUSBAR
                        | ROLE_SYSTEM_LIST
                        | ROLE_SYSTEM_LISTITEM
                        | ROLE_SYSTEM_OUTLINE
                        | ROLE_SYSTEM_OUTLINEITEM
                        | ROLE_SYSTEM_PAGETAB
                        | ROLE_SYSTEM_INDICATOR
                        | ROLE_SYSTEM_GRAPHIC
                        | ROLE_SYSTEM_STATICTEXT
                        | ROLE_SYSTEM_PUSHBUTTON
                        | ROLE_SYSTEM_PROGRESSBAR
                        | ROLE_SYSTEM_SLIDER
                ) {
                    log::debug!("MSAA: role={} → ShouldBypass", role_id);
                    return EngineBypassState::ShouldBypass;
                }

                log::debug!(
                    "MSAA: role={} → Unknown (not in allow/deny list)",
                    role_id
                );
            }
        }
    }

    // 5. 判定不能 → Unknown（安全側: バイパス）
    EngineBypassState::Unknown
}

/// UIA 非同期判定ワーカースレッドを起動する
///
/// 専用スレッドで COM を初期化し、`IUIAutomation` インスタンスを保持する。
/// チャネル経由で HWND を受け取り、`GetFocusedElement` でコントロール種別を判定して
/// `BYPASS_STATE` を更新する。Phase 1-2 で `Unknown` だったコントロールの解像度を上げる。
fn spawn_uia_worker() -> mpsc::Sender<SendableHwnd> {
    let (tx, rx) = mpsc::channel::<SendableHwnd>();
    std::thread::spawn(move || {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }

        let automation: Option<IUIAutomation> = unsafe {
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()
        };

        let Some(automation) = automation else {
            log::warn!("UIA: Failed to create IUIAutomation, Phase 3 disabled");
            return;
        };

        log::info!("UIA worker thread started");

        while let Ok(SendableHwnd(hwnd)) = rx.recv() {
            let state = unsafe { uia_determine_bypass(&automation, hwnd) };
            if state != EngineBypassState::Unknown {
                BYPASS_STATE.store(state as u8, Ordering::Release);
                log::debug!("UIA async: hwnd={:?} → {:?}", hwnd, state);

                // ShouldBypass の場合はメインスレッドにエンジンフラッシュを依頼
                if state == EngineBypassState::ShouldBypass {
                    unsafe {
                        let _ = PostMessageW(
                            HWND::default(),
                            WM_UIA_BYPASS_UPDATE,
                            WPARAM(0),
                            LPARAM(0),
                        );
                    }
                }
            }
        }
    });
    tx
}

/// UIA を使用してフォーカス中コントロールのバイパス状態を判定する
///
/// `GetFocusedElement` で実際の UI 要素を取得し、`CurrentControlType` で
/// テキスト入力系か非テキスト系かを判別する。Chrome/WPF/UWP など
/// Win32 クラス名では判定できないコントロールに有効。
///
/// Safety: COM が初期化済みのスレッドから呼び出すこと
#[allow(unused_variables)] // hwnd はデバッグ用に保持
unsafe fn uia_determine_bypass(automation: &IUIAutomation, hwnd: HWND) -> EngineBypassState {
    let element: IUIAutomationElement = match automation.GetFocusedElement() {
        Ok(el) => el,
        Err(e) => {
            log::trace!("UIA: GetFocusedElement failed: {:?}", e);
            return EngineBypassState::Unknown;
        }
    };

    let control_type = match element.CurrentControlType() {
        Ok(ct) => ct,
        Err(_) => return EngineBypassState::Unknown,
    };

    // テキスト入力コントロール → AllowEngine
    if control_type == UIA_EditControlTypeId || control_type == UIA_DocumentControlTypeId {
        return EngineBypassState::AllowEngine;
    }

    // ComboBox: 多くの場合テキスト入力を受け付ける
    if control_type == UIA_ComboBoxControlTypeId {
        return EngineBypassState::AllowEngine;
    }

    // 非テキストコントロール → ShouldBypass
    if control_type == UIA_ButtonControlTypeId
        || control_type == UIA_MenuItemControlTypeId
        || control_type == UIA_TreeItemControlTypeId
        || control_type == UIA_ListItemControlTypeId
        || control_type == UIA_TabControlTypeId
        || control_type == UIA_TabItemControlTypeId
        || control_type == UIA_ToolBarControlTypeId
        || control_type == UIA_StatusBarControlTypeId
        || control_type == UIA_ProgressBarControlTypeId
        || control_type == UIA_SliderControlTypeId
        || control_type == UIA_ScrollBarControlTypeId
        || control_type == UIA_HyperlinkControlTypeId
        || control_type == UIA_ImageControlTypeId
        || control_type == UIA_MenuBarControlTypeId
        || control_type == UIA_MenuControlTypeId
        || control_type == UIA_TitleBarControlTypeId
        || control_type == UIA_SeparatorControlTypeId
        || control_type == UIA_TextControlTypeId
    {
        return EngineBypassState::ShouldBypass;
    }

    // 未知のコントロール種別
    EngineBypassState::Unknown
}

/// メッセージループ
fn run_message_loop() {
    let mut msg = MSG::default();

    loop {
        let ret = unsafe { GetMessageW(&raw mut msg, HWND::default(), 0, 0) };
        if ret.0 <= 0 {
            break; // WM_QUIT or エラー
        }

        match msg.message {
            WM_TIMER if msg.wParam.0 == TIMER_PENDING || msg.wParam.0 == TIMER_SPECULATIVE => {
                let timer_id = msg.wParam.0;
                unsafe {
                    // IME が非活性なら on_timeout せず flush（コンテキスト喪失）
                    let ime_active = IME
                        .get_ref()
                        .map_or(true, |ime| ime.is_active() && ime.get_mode().is_kana_input());
                    if !ime_active {
                        invalidate_engine_context(ContextInvalidation::ImeOff);
                    } else if let Some(engine) = ENGINE.get_mut() {
                        let response = engine.on_timeout(timer_id);
                        let mut timer_runtime = Win32TimerRuntime;
                        let mut action_executor = SendInputExecutor;
                        dispatch(&response, &mut timer_runtime, &mut action_executor);
                    }
                }
            }
            WM_INPUTLANGCHANGE => unsafe {
                // 入力言語が変更された（Win+Space 等）→ 保留をフラッシュ + ガード ON
                // 言語切替直後は IME 状態が未反映の可能性があるため、
                // 後続キーをメッセージループに回して確実に更新後に処理する。
                log::info!("Input language changed, flushing pending state and enabling guard");
                invalidate_engine_context(ContextInvalidation::InputLanguageChanged);
                if let Some(guard) = IME_GUARD.get_mut() {
                    *guard = true;
                }
            },
            WM_PROCESS_DEFERRED => unsafe {
                // IME 制御キー後の遅延キーを再処理する。
                // この時点で IME 状態は確実に更新済み。
                process_deferred_keys();
            },
            WM_UIA_BYPASS_UPDATE => unsafe {
                // UIA 非同期判定が ShouldBypass を返した → エンジンの保留をフラッシュ
                let bypass = BYPASS_STATE.load(Ordering::Acquire);
                if bypass == EngineBypassState::ShouldBypass as u8 {
                    invalidate_engine_context(ContextInvalidation::FocusChanged);
                }
            },
            WM_HOTKEY if msg.wParam.0 == HOTKEY_ID_TOGGLE as usize => unsafe {
                toggle_engine();
            },
            WM_APP => unsafe {
                let layout_names: Vec<String> = LAYOUTS
                    .get_ref()
                    .map(|layouts| layouts.iter().map(|(name, ..)| name.clone()).collect())
                    .unwrap_or_default();
                tray::handle_tray_message(msg.hwnd, msg.lParam, &layout_names);
            },
            WM_RELOAD_CONFIG => unsafe {
                log::info!("Config reload requested via WM_RELOAD_CONFIG");
                reload_config();
            },
            WM_COMMAND => unsafe {
                if let Some(cmd) = tray::handle_tray_command(msg.wParam) {
                    if cmd == tray::cmd_settings() {
                        launch_settings();
                    } else if cmd == tray::cmd_toggle() {
                        toggle_engine();
                    } else if cmd == tray::cmd_exit() {
                        PostQuitMessage(0);
                    } else if cmd >= tray::cmd_layout_base() {
                        let index = usize::from(cmd - tray::cmd_layout_base());
                        switch_layout(index);
                    }
                }
            },
            _ => unsafe {
                DispatchMessageW(&raw const msg);
            },
        }
    }
}

/// IME 制御キー後に遅延されたキーを再処理する。
///
/// メッセージループから呼ばれるため、この時点で IME 制御キーは OS/IME に
/// 渡し済みで、IME 状態は最新に更新されている。
///
/// Safety: シングルスレッドからのみ呼び出すこと
unsafe fn process_deferred_keys() {
    // ガード解除
    if let Some(guard) = IME_GUARD.get_mut() {
        *guard = false;
    }

    // バッファからキーを取り出す
    let keys = DEFERRED_KEYS
        .get_mut()
        .map(std::mem::take)
        .unwrap_or_default();

    if keys.is_empty() {
        return;
    }

    log::debug!("Processing {} deferred key(s) after IME control", keys.len());

    for event in keys {
        // IME 状態を再チェック（最新の状態で判定）
        let ime_active = IME
            .get_ref()
            .map_or(false, |ime| ime.is_active() && ime.get_mode().is_kana_input());

        if ime_active {
            // IME ON → エンジンで処理
            if let Some(engine) = ENGINE.get_mut() {
                let response = engine.on_event(event);
                let mut timer_runtime = Win32TimerRuntime;
                let mut action_executor = SendInputExecutor;
                dispatch(&response, &mut timer_runtime, &mut action_executor);
            }
        } else {
            // IME OFF → キーをそのまま再注入（INJECTED_MARKER 付き）
            reinject_key(&event);
        }
    }
}

/// キーイベントを SendInput で再注入する（IME OFF 時の遅延キー用）
///
/// INJECTED_MARKER 付きなのでフックに再捕捉されない。
unsafe fn reinject_key(event: &RawKeyEvent) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
        KEYEVENTF_SCANCODE, VIRTUAL_KEY,
    };
    use crate::output::INJECTED_MARKER;

    let is_keyup = matches!(
        event.event_type,
        KeyEventType::KeyUp | KeyEventType::SysKeyUp
    );

    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(event.vk_code),
                wScan: event.scan_code as u16,
                dwFlags: if is_keyup {
                    KEYEVENTF_KEYUP | KEYEVENTF_SCANCODE
                } else {
                    KEYEVENTF_SCANCODE
                },
                time: 0,
                dwExtraInfo: INJECTED_MARKER,
            },
        },
    };
    SendInput(&[input], size_of::<INPUT>() as i32);
}

/// エンジンの有効/無効を切り替え、トレイアイコンを更新する
///
/// Safety: シングルスレッドからのみ呼び出すこと
unsafe fn toggle_engine() {
    if let Some(engine) = ENGINE.get_mut() {
        let (enabled, flush_resp) = engine.toggle_enabled();
        let mut timer_runtime = Win32TimerRuntime;
        let mut action_executor = SendInputExecutor;
        dispatch(&flush_resp, &mut timer_runtime, &mut action_executor);
        log::info!("Engine toggled: {}", if enabled { "ON" } else { "OFF" });
        if let Some(tray) = TRAY.get_mut() {
            tray.set_enabled(enabled);
        }
    }
}

/// 外部コンテキスト喪失時にエンジンの保留状態を安全にフラッシュする。
///
/// IMEオフ、入力言語変更など、エンジンの前提が崩れた場合に呼ぶ。
/// 全てのコンテキスト無効化経路はこの関数を通すこと。
unsafe fn invalidate_engine_context(reason: ContextInvalidation) {
    if let Some(engine) = ENGINE.get_mut() {
        let response = engine.flush_pending(reason);
        let mut timer_runtime = Win32TimerRuntime;
        let mut action_executor = SendInputExecutor;
        dispatch(&response, &mut timer_runtime, &mut action_executor);
    }
}

/// 設定画面 (awase-settings) を起動する
fn launch_settings() {
    let names = if cfg!(windows) {
        vec!["awase-settings.exe"]
    } else {
        vec!["awase-settings"]
    };
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for name in &names {
                let path = dir.join(name);
                if path.exists() {
                    let _ = std::process::Command::new(&path).spawn();
                    return;
                }
            }
        }
    }
    log::warn!("awase-settings not found");
}

/// 設定ファイルを再読み込みし、エンジンのパラメータを更新する
///
/// Safety: シングルスレッドからのみ呼び出すこと
unsafe fn reload_config() {
    let config = match load_config() {
        Ok(c) => c,
        Err(e) => {
            log::warn!("Failed to reload config: {e}");
            return;
        }
    };

    if let Some(engine) = ENGINE.get_mut() {
        engine.set_threshold_ms(config.general.simultaneous_threshold_ms);
        engine.set_confirm_mode(
            config.general.confirm_mode,
            config.general.speculative_delay_ms,
        );
        log::info!(
            "Engine parameters updated: threshold={}ms, confirm_mode={:?}, speculative_delay={}ms",
            config.general.simultaneous_threshold_ms,
            config.general.confirm_mode,
            config.general.speculative_delay_ms,
        );
    }

    // n-gram モデルの再読み込み
    init_ngram(&config);

    log::info!("Config reloaded successfully");
}

/// layouts_dir 内の *.yab を全てスキャンして配列一覧を構築する
fn scan_layouts(
    layouts_dir: &Path,
    left_thumb_vk: u16,
    right_thumb_vk: u16,
) -> Result<Vec<(String, YabLayout, u16, u16)>> {
    let mut layouts = Vec::new();

    if !layouts_dir.is_dir() {
        log::warn!("Layouts directory not found: {}", layouts_dir.display());
        return Ok(layouts);
    }

    let entries = std::fs::read_dir(layouts_dir).with_context(|| {
        format!(
            "Failed to read layouts directory: {}",
            layouts_dir.display()
        )
    })?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "yab") {
            match std::fs::read_to_string(&path) {
                Ok(content) => match YabLayout::parse(&content) {
                    Ok(yab) => {
                        let yab = yab.resolve_kana();
                        log::info!("Discovered layout: {} ({})", yab.name, path.display());
                        layouts.push((yab.name.clone(), yab, left_thumb_vk, right_thumb_vk));
                    }
                    Err(e) => {
                        log::warn!("Failed to parse layout {}: {e}", path.display());
                    }
                },
                Err(e) => {
                    log::warn!("Failed to read layout file {}: {e}", path.display());
                }
            }
        }
    }

    // 名前順にソートして安定した順序にする
    layouts.sort_by(|(a, ..), (b, ..)| a.cmp(b));

    Ok(layouts)
}

/// 相対パスを実行ファイルのディレクトリ基準で解決する
fn resolve_relative(path: &str) -> PathBuf {
    if Path::new(path).is_absolute() {
        return PathBuf::from(path);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let resolved = dir.join(path);
            if resolved.exists() {
                return resolved;
            }
        }
    }
    // フォールバック: カレントディレクトリからの相対パス
    PathBuf::from(path)
}

/// 配列を動的に切り替える
///
/// Safety: シングルスレッドからのみ呼び出すこと
unsafe fn switch_layout(index: usize) {
    let Some(layouts) = LAYOUTS.get_ref() else {
        return;
    };

    let Some((name, layout_template, _l_vk, _r_vk)) = layouts.get(index) else {
        log::warn!("Layout index {index} out of range");
        return;
    };

    // YabLayout の各面を clone して新しいレイアウトを構築する
    let new_layout = YabLayout {
        name: layout_template.name.clone(),
        normal: layout_template.normal.clone(),
        left_thumb: layout_template.left_thumb.clone(),
        right_thumb: layout_template.right_thumb.clone(),
        shift: layout_template.shift.clone(),
    };

    if let Some(engine) = ENGINE.get_mut() {
        let response = engine.swap_layout(new_layout);
        let mut timer_runtime = Win32TimerRuntime;
        let mut action_executor = SendInputExecutor;
        dispatch(&response, &mut timer_runtime, &mut action_executor);
    }

    if let Some(tray) = TRAY.get_mut() {
        tray.set_layout_name(name);
    }

    log::info!("Switched layout to: {name}");
}

/// 設定ファイルのパスを探す
fn find_config_path() -> Result<PathBuf> {
    // 1. コマンドライン引数
    if let Some(path) = std::env::args().nth(1) {
        return Ok(PathBuf::from(path));
    }

    // 2. 実行ファイルディレクトリ → カレントディレクトリの順で config.toml を探す
    let resolved = resolve_relative("config.toml");
    if resolved.exists() {
        return Ok(resolved);
    }

    anyhow::bail!(
        "Config file not found. Place config.toml next to the executable, \
         or specify path as command line argument."
    )
}

/// Ctrl+C ハンドラを登録（Win32 SetConsoleCtrlHandler）
fn install_ctrl_handler() {
    unsafe extern "system" fn handler(_ctrl_type: u32) -> windows::Win32::Foundation::BOOL {
        QUIT_REQUESTED.store(true, Ordering::SeqCst);
        PostQuitMessage(0);
        windows::Win32::Foundation::BOOL(1) // TRUE = handled
    }

    unsafe {
        windows::Win32::System::Console::SetConsoleCtrlHandler(Some(handler), true).unwrap_or_else(
            |e| {
                log::warn!("Failed to set console ctrl handler: {e}");
            },
        );
    }
}
