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
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, KillTimer, PostQuitMessage, SetTimer, MSG, WM_APP, WM_COMMAND,
    WM_HOTKEY, WM_TIMER,
};

use awase::config::{vk_name_to_code, AppConfig};
use awase::engine::{Engine, TIMER_PENDING};
use awase::ngram::NgramModel;
use awase::types::RawKeyEvent;
use awase::yab::YabLayout;
use timed_fsm::{dispatch, ActionExecutor, TimedStateMachine, TimerRuntime};

use crate::hook::CallbackResult;
use crate::ime::{HybridProvider, ImeProvider};
use crate::output::Output;
use crate::single_thread_cell::SingleThreadCell;
use crate::tray::SystemTray;

/// 有効/無効切り替えホットキー ID
const HOTKEY_ID_TOGGLE: i32 = 1;

static ENGINE: SingleThreadCell<Engine> = SingleThreadCell::new();
static OUTPUT: SingleThreadCell<Output> = SingleThreadCell::new();
static IME: SingleThreadCell<HybridProvider> = SingleThreadCell::new();
static TRAY: SingleThreadCell<SystemTray> = SingleThreadCell::new();

/// 利用可能な配列の一覧（名前, `YabLayout`, 左親指VK, 右親指VK）
static LAYOUTS: SingleThreadCell<Vec<(String, YabLayout, u16, u16)>> = SingleThreadCell::new();

/// Ctrl+C 受信フラグ
static QUIT_REQUESTED: AtomicBool = AtomicBool::new(false);

fn main() -> Result<()> {
    init_logging();
    let config = load_config()?;
    let (layout_names, initial_layout_name) = init_engine(&config)?;
    init_ime();
    init_ngram(&config);

    unsafe {
        OUTPUT.set(Output::new());
    }

    init_tray(&layout_names, &initial_layout_name)?;
    install_hooks_and_hotkeys(&config)?;

    log::info!("Hook installed. Running message loop...");
    log::info!("Press Ctrl+C to exit.");
    install_ctrl_handler();

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
    let Some(engine) = ENGINE.get_mut() else {
        return CallbackResult::PassThrough;
    };

    // 出力方式:
    // - IME OFF / 英数モード → PassThrough（エンジンバイパス）
    // - かな入力モード → Engine が KeyAction::Char を出力、output が KEYEVENTF_UNICODE で送信
    if let Some(ime_provider) = IME.get_ref() {
        let mode = ime_provider.get_mode();
        if !ime_provider.is_active() {
            return CallbackResult::PassThrough;
        }
        log::trace!("IME mode: {:?}", mode);
        if !mode.is_kana_input() {
            log::warn!(
                "IME is active but not in kana input mode ({:?}), passing through",
                mode
            );
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
fn run_message_loop() {
    let mut msg = MSG::default();

    loop {
        let ret = unsafe { GetMessageW(&raw mut msg, HWND::default(), 0, 0) };
        if ret.0 <= 0 {
            break; // WM_QUIT or エラー
        }

        match msg.message {
            WM_TIMER if msg.wParam.0 == TIMER_PENDING => {
                // タイムアウト：保留キーを単独打鍵として確定
                unsafe {
                    if let Some(engine) = ENGINE.get_mut() {
                        let response = engine.on_timeout(TIMER_PENDING);
                        let mut timer_runtime = Win32TimerRuntime;
                        let mut action_executor = SendInputExecutor;
                        dispatch(&response, &mut timer_runtime, &mut action_executor);
                    }
                }
            }
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
            WM_COMMAND => unsafe {
                if let Some(cmd) = tray::handle_tray_command(msg.wParam) {
                    if cmd == tray::cmd_toggle() {
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
        let enabled = engine.toggle_enabled();
        log::info!("Engine toggled: {}", if enabled { "ON" } else { "OFF" });
        if let Some(tray) = TRAY.get_mut() {
            tray.set_enabled(enabled);
        }
    }
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
