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

mod focus;
mod hook;
mod key_buffer;
mod ime;
mod output;
mod single_thread_cell;
mod tray;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use anyhow::{Context, Result};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetGUIThreadInfo, GetMessageW,
    GUITHREADINFO, KillTimer, PostMessageW,
    PostQuitMessage, SetTimer, MSG, WM_APP, WM_COMMAND, WM_HOTKEY, WM_INPUTLANGCHANGE, WM_TIMER,
};

use awase::config::{vk_name_to_code, AppConfig};
use awase::engine::{Engine, TIMER_PENDING, TIMER_SPECULATIVE};
use awase::types::{ContextChange, FocusKind};
use awase::vk;
use awase::ngram::NgramModel;
use awase::types::{KeyEventType, RawKeyEvent, VkCode};
use awase::yab::YabLayout;
use timed_fsm::{dispatch, ActionExecutor, TimedStateMachine, TimerRuntime};

use crate::hook::CallbackResult;
use crate::ime::{HybridProvider, ImeProvider};
use crate::output::Output;
use crate::single_thread_cell::SingleThreadCell;
use crate::tray::SystemTray;

/// 有効/無効切り替えホットキー ID
const HOTKEY_ID_TOGGLE: i32 = 1;

/// 手動フォーカスオーバーライドホットキー ID (Ctrl+Shift+F11)
const HOTKEY_ID_FOCUS_OVERRIDE: i32 = 2;

/// 設定リロード用カスタムメッセージ（設定 GUI から `PostMessageW` で送信される）
const WM_RELOAD_CONFIG: u32 = WM_APP + 10;

/// IME 制御キー後の遅延キー再処理用カスタムメッセージ
const WM_PROCESS_DEFERRED: u32 = WM_APP + 11;

/// UIA 非同期判定完了通知用カスタムメッセージ
pub(crate) const WM_FOCUS_KIND_UPDATE: u32 = WM_APP + 12;

/// Undetermined + IME ON バッファリングのタイムアウト用カスタムメッセージ
/// （将来的に `PostMessageW` で明示送信する場合に使用）
#[allow(dead_code)]
const WM_BUFFER_TIMEOUT: u32 = WM_APP + 13;

/// Undetermined + IME ON バッファリングのタイマー ID
pub(crate) const TIMER_UNDETERMINED_BUFFER: usize = 100;

/// IME の未確定状態を確認する（engine から呼び出し用）
#[allow(dead_code)]
pub fn check_ime_composing() -> bool {
    unsafe { IME.get_ref().is_some_and(ImeProvider::is_composing) }
}

pub(crate) static ENGINE: SingleThreadCell<Engine> = SingleThreadCell::new();
pub(crate) static OUTPUT: SingleThreadCell<Output> = SingleThreadCell::new();
pub(crate) static IME: SingleThreadCell<HybridProvider> = SingleThreadCell::new();
static TRAY: SingleThreadCell<SystemTray> = SingleThreadCell::new();

/// 利用可能な配列の一覧（名前, `YabLayout`, 左親指VK, 右親指VK）
static LAYOUTS: SingleThreadCell<Vec<(String, YabLayout, VkCode, VkCode)>> = SingleThreadCell::new();

/// キーイベントバッファ（IME ガード + 遅延キー + PassThrough 記憶）
pub(crate) static KEY_BUFFER: SingleThreadCell<key_buffer::KeyBuffer> = SingleThreadCell::new();

/// フォーカス検出のシングルスレッド状態を集約した構造体
pub(crate) static FOCUS: SingleThreadCell<focus::FocusDetector> = SingleThreadCell::new();

use crate::focus::cache::DetectionSource;

/// Ctrl+C 受信フラグ
static QUIT_REQUESTED: AtomicBool = AtomicBool::new(false);

/// フォーカス中コントロールの種別キャッシュ（Undetermined=2 で初期化）
pub(crate) static FOCUS_KIND: AtomicU8 = AtomicU8::new(2); // FocusKind::Undetermined


fn main() -> Result<()> {
    init_logging();
    let config = load_config()?;
    let (layout_names, initial_layout_name) = init_engine(&config)?;
    init_ime();
    init_ngram(&config);

    unsafe {
        OUTPUT.set(Output::new());
        KEY_BUFFER.set(key_buffer::KeyBuffer::new());
        FOCUS.set(focus::FocusDetector::new(config.focus_overrides.clone()));
    }

    init_tray(&layout_names, &initial_layout_name)?;
    install_hooks_and_hotkeys(&config)?;

    log::info!("Hook installed. Running message loop...");
    log::info!("Press Ctrl+C to exit.");
    install_ctrl_handler();
    focus::install_focus_hook();

    // Phase 3: UIA 非同期判定ワーカースレッドを起動
    let uia_tx = focus::uia::spawn_uia_worker();
    unsafe {
        if let Some(f) = FOCUS.get_mut() {
            f.set_uia_sender(uia_tx);
        }
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
    let left_thumb_vk = VkCode(vk_name_to_code(&config.general.left_thumb_key).context(format!(
        "Unknown VK name: {}",
        config.general.left_thumb_key
    ))?);
    let right_thumb_vk = VkCode(vk_name_to_code(&config.general.right_thumb_key).context(format!(
        "Unknown VK name: {}",
        config.general.right_thumb_key
    ))?);

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
    layouts: &[(String, YabLayout, VkCode, VkCode)],
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
    register_focus_override_hotkey();
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

/// 手動フォーカスオーバーライドホットキー (Ctrl+Shift+F11) を登録する
fn register_focus_override_hotkey() {
    const MOD_CONTROL: u32 = 0x0002;
    const MOD_SHIFT: u32 = 0x0004;
    const VK_F11: u32 = 0x7A;
    unsafe {
        let result = RegisterHotKey(
            HWND::default(),
            HOTKEY_ID_FOCUS_OVERRIDE,
            HOT_KEY_MODIFIERS(MOD_CONTROL | MOD_SHIFT),
            VK_F11,
        );
        if result.is_ok() {
            log::info!("Focus override hotkey registered: Ctrl+Shift+F11");
        } else {
            log::warn!("Failed to register focus override hotkey: Ctrl+Shift+F11");
        }
    }
}


/// クリーンアップ処理
fn cleanup() {
    hook::uninstall_hook();
    unsafe {
        let _ = UnregisterHotKey(HWND::default(), HOTKEY_ID_TOGGLE);
        let _ = UnregisterHotKey(HWND::default(), HOTKEY_ID_FOCUS_OVERRIDE);
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
        FOCUS.clear();
        KEY_BUFFER.clear();
    }
    log::info!("Exited cleanly.");
}

/// Win32 タイマーランタイム
pub(crate) struct Win32TimerRuntime;

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
pub(crate) struct SendInputExecutor;

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



/// on_key_event_callback Step 4 の入力コンテキスト判定結果
enum InputContext {
    /// テキスト入力確定 → エンジン処理
    TextInput,
    /// 非テキスト確定 → パススルー
    NonText,
    /// 判定不能 + IME ON → バッファリング
    UndeterminedImeOn,
    /// 判定不能 + IME OFF → パススルー + 記憶
    UndeterminedImeOff,
}

/// フォーカス種別と IME 状態から入力コンテキストを判定する
///
/// # Safety
/// `IME` / `FOCUS_KIND` はシングルスレッドからのみアクセスすること。
unsafe fn resolve_input_context() -> InputContext {
    match FocusKind::load(&FOCUS_KIND) {
        FocusKind::TextInput => InputContext::TextInput,
        FocusKind::NonText => InputContext::NonText,
        FocusKind::Undetermined => {
            let ime_on = IME
                .get_ref()
                .is_some_and(|ime| ime.is_active() && ime.get_mode().is_kana_input());
            if ime_on {
                InputContext::UndeterminedImeOn
            } else {
                InputContext::UndeterminedImeOff
            }
        }
    }
}

/// フックコールバックからの Engine 呼び出し
#[allow(clippy::too_many_lines)] // event dispatch hub with multiple steps
unsafe fn on_key_event_callback(event: RawKeyEvent) -> CallbackResult {
    let is_key_down = matches!(
        event.event_type,
        KeyEventType::KeyDown | KeyEventType::SysKeyDown
    );

    // ── Step 0: パターン観察（すべてのキーイベントに対して、バイパスチェック前に実行） ──
    focus::pattern::observe_key_pattern(&event);

    // ── Step 1: IME/親指キー検出による即時 TextInput 昇格 ──
    // IME 制御キーまたは親指キー（変換/無変換）が押された場合、
    // ユーザーがテキスト入力コンテキストにいると判断して昇格する。
    if is_key_down && vk::is_ime_context(event.vk_code) {
        let current = FocusKind::load(&FOCUS_KIND);
        if current != FocusKind::TextInput {
            focus::pattern::promote_to_text_input(
                DetectionSource::ImeKeyInferred,
                &format!("IME/thumb key 0x{:02X}", event.vk_code.0),
            );
            // Undetermined バッファリング中ならバッファを処理
            if let Some(kb) = KEY_BUFFER.get_mut() {
                if kb.undetermined_buffering {
                    kb.undetermined_buffering = false;
                    let _ = KillTimer(HWND::default(), TIMER_UNDETERMINED_BUFFER);
                    // バッファされたキーをエンジンで処理
                    let keys = kb.drain_deferred();
                    for buffered in keys {
                        if let Some(engine) = ENGINE.get_mut() {
                            let response = engine.on_event(buffered);
                            let mut timer_runtime = Win32TimerRuntime;
                            let mut action_executor = SendInputExecutor;
                            dispatch(&response, &mut timer_runtime, &mut action_executor);
                        }
                    }
                }
            }
            // PassThrough 済みキーがあれば取り消して再処理
            key_buffer::retract_passthrough_memory();
        }
    }

    // ── Step 2: IME ガード: IME 制御キー直後の後続キーを遅延処理する ──
    // IME 制御キー（半角/全角等）が OS に渡された直後は、IME 状態がまだ反映されて
    // いない可能性がある。後続キーをメッセージループに回すことで、IME 状態が
    // 確実に更新された後に処理する。
    if let Some(kb) = KEY_BUFFER.get_mut() {
        if kb.is_guarded() {
            // IME 制御キーの KeyUp でガード解除
            if !is_key_down && vk::is_ime_control(event.vk_code) {
                kb.set_guard(false);
                return CallbackResult::PassThrough;
            }
            // KeyDown はバッファに保存してメッセージループに回す
            if is_key_down {
                kb.push_deferred(event);
                let _ = PostMessageW(HWND::default(), WM_PROCESS_DEFERRED, WPARAM(0), LPARAM(0));
                return CallbackResult::Consumed;
            }
            // ガード中の KeyUp（IME制御キー以外）はパススルー
            return CallbackResult::PassThrough;
        }
    }

    // ── Step 3: IME 制御キーの検出: ガードを有効にしてパススルー ──
    if is_key_down && vk::is_ime_control(event.vk_code) {
        // エンジンの保留をフラッシュ（engine 側の handle_bypass で実行される）
        if let Some(engine) = ENGINE.get_mut() {
            let response = engine.on_event(event);
            let mut timer_runtime = Win32TimerRuntime;
            let mut action_executor = SendInputExecutor;
            dispatch(&response, &mut timer_runtime, &mut action_executor);
            // engine が passthrough を返す → ガードを有効にして OS に渡す
            if !response.consumed {
                if let Some(kb) = KEY_BUFFER.get_mut() {
                    kb.set_guard(true);
                }
            }
        }
        // consumed=false の場合は OS にそのまま渡す（CallbackResult::PassThrough）
        // consumed=true の場合はエンジンが処理済み（通常ありえないがsafety）
        return CallbackResult::PassThrough;
    }

    // ── Step 4: フォーカス判定によるハイブリッド戦略 ──
    let Some(engine) = ENGINE.get_mut() else {
        return CallbackResult::PassThrough;
    };

    match resolve_input_context() {
        InputContext::NonText => {
            // 非テキストコントロール → 常にパススルー
            return CallbackResult::PassThrough;
        }
        InputContext::TextInput => {
            // テキスト入力 → 既存のエンジン処理（下の Step 5 に進む）
        }
        InputContext::UndeterminedImeOn => {
            // IME ON + Undetermined → 文字キーならバッファリング
            if is_key_down {
                let is_char = vk::is_modifier_free_char(event.vk_code, focus::pattern::is_os_modifier_held());
                if is_char {
                    if let Some(kb) = KEY_BUFFER.get_mut() {
                        kb.push_deferred(event);
                    }
                    key_buffer::start_buffer_timeout_if_needed();
                    return CallbackResult::Consumed;
                }
            }
            return CallbackResult::PassThrough;
        }
        InputContext::UndeterminedImeOff => {
            // IME OFF + Undetermined → 文字キーなら PassThrough + 記憶
            if is_key_down {
                let is_char = vk::is_modifier_free_char(event.vk_code, focus::pattern::is_os_modifier_held());
                if is_char {
                    if let Some(kb) = KEY_BUFFER.get_mut() {
                        kb.push_passthrough(event);
                    }
                    return CallbackResult::PassThrough;
                }
            }
            return CallbackResult::PassThrough;
        }
    }

    // ── Step 5: エンジン処理（TextInput 確定時のみ到達） ──
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








/// メッセージループ
#[allow(clippy::too_many_lines)] // message loop dispatch with many match arms
fn run_message_loop() {
    let mut msg = MSG::default();

    loop {
        let ret = unsafe { GetMessageW(&raw mut msg, HWND::default(), 0, 0) };
        if ret.0 <= 0 {
            break; // WM_QUIT or エラー
        }

        match msg.message {
            WM_TIMER if msg.wParam.0 == TIMER_UNDETERMINED_BUFFER => {
                unsafe {
                    key_buffer::handle_buffer_timeout();
                }
            }
            WM_TIMER if msg.wParam.0 == TIMER_PENDING || msg.wParam.0 == TIMER_SPECULATIVE => {
                let timer_id = msg.wParam.0;
                unsafe {
                    // IME が非活性なら on_timeout せず flush（コンテキスト喪失）
                    let ime_active = IME
                        .get_ref()
                        .is_none_or(|ime| ime.is_active() && ime.get_mode().is_kana_input());
                    if !ime_active {
                        invalidate_engine_context(ContextChange::ImeOff);
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
                invalidate_engine_context(ContextChange::InputLanguageChanged);
                if let Some(kb) = KEY_BUFFER.get_mut() {
                    kb.set_guard(true);
                }
            },
            WM_PROCESS_DEFERRED => unsafe {
                // IME 制御キー後の遅延キーを再処理する。
                // この時点で IME 状態は確実に更新済み。
                key_buffer::process_deferred_keys();
            },
            WM_FOCUS_KIND_UPDATE => unsafe {
                // UIA 非同期判定完了 → キャッシュ更新 + IME 状態復帰
                let kind = FocusKind::load(&FOCUS_KIND);
                // UIA 結果をキャッシュに反映
                if let Some(f) = FOCUS.get_mut() {
                    if let Some((pid, cls)) = f.last_focus_info.as_ref() {
                        f.cache.insert(*pid, cls.clone(), kind, DetectionSource::UiaAsync);
                    }
                }
                if kind == FocusKind::NonText {
                    invalidate_engine_context(ContextChange::FocusChanged);
                }
                // UIA が TextInput を返した場合、IME OFF されていたら ON に復帰
                // （非ブラウザ系で自動 IME OFF された後に UIA が TextInput を返したケース）
                if kind == FocusKind::TextInput {
                    if let Some(f) = FOCUS.get_ref() {
                        if let Some((_, cls)) = f.last_focus_info.as_ref() {
                            if !vk::is_browser_or_electron_class(cls) {
                                // 非ブラウザ系で UIA が TextInput → IME ON に復帰
                                let mut info = GUITHREADINFO {
                                    cbSize: size_of::<GUITHREADINFO>() as u32,
                                    ..Default::default()
                                };
                                if GetGUIThreadInfo(0, &raw mut info).is_ok()
                                    && info.hwndFocus != HWND::default()
                                {
                                    focus::set_ime_on(info.hwndFocus);
                                }
                            }
                        }
                    }
                }
            },
            WM_HOTKEY if msg.wParam.0 == HOTKEY_ID_TOGGLE as usize => unsafe {
                toggle_engine();
            },
            WM_HOTKEY if msg.wParam.0 == HOTKEY_ID_FOCUS_OVERRIDE as usize => unsafe {
                focus::toggle_focus_override();
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
pub(crate) unsafe fn invalidate_engine_context(reason: ContextChange) {
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

    // フォーカスオーバーライド再読み込み + キャッシュクリア
    if let Some(f) = FOCUS.get_mut() {
        f.overrides = config.focus_overrides;
        f.cache = focus::cache::FocusCache::new();
    }
    log::info!("Focus overrides reloaded");

    log::info!("Config reloaded successfully");
}

/// layouts_dir 内の *.yab を全てスキャンして配列一覧を構築する
fn scan_layouts(
    layouts_dir: &Path,
    left_thumb_vk: VkCode,
    right_thumb_vk: VkCode,
) -> Result<Vec<(String, YabLayout, VkCode, VkCode)>> {
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
