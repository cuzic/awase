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
mod ime;
mod key_buffer;
mod output;
mod single_thread_cell;
mod tray;
mod win32;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use anyhow::{Context, Result};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetGUIThreadInfo, GetMessageW, KillTimer, PostMessageW, PostQuitMessage,
    SetTimer, GUITHREADINFO, MSG, WM_APP, WM_COMMAND, WM_HOTKEY, WM_INPUTLANGCHANGE, WM_TIMER,
};

use awase::config::{
    parse_key_combo, vk_name_to_code, AppConfig, ImeSyncConfig, ParsedKeyCombo, ValidatedConfig,
};
use awase::engine::{Engine, TIMER_PENDING, TIMER_SPECULATIVE};
use awase::ngram::NgramModel;
use awase::types::{ContextChange, FocusKind};
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
#[allow(dead_code)] // Undetermined バッファタイムアウトを PostMessageW で明示送信する将来拡張用
const WM_BUFFER_TIMEOUT: u32 = WM_APP + 13;

/// Undetermined + IME ON バッファリングのタイマー ID
pub(crate) const TIMER_UNDETERMINED_BUFFER: usize = 100;

/// IME の未確定状態を確認する（engine から呼び出し用）
#[allow(dead_code)] // engine 側から IME 未確定状態を確認する将来拡張用
pub fn check_ime_composing() -> bool {
    unsafe { IME.get_ref().is_some_and(ImeProvider::is_composing) }
}

pub(crate) static ENGINE: SingleThreadCell<Engine> = SingleThreadCell::new();
pub(crate) static OUTPUT: SingleThreadCell<Output> = SingleThreadCell::new();
pub(crate) static IME: SingleThreadCell<HybridProvider> = SingleThreadCell::new();
pub(crate) static TRAY: SingleThreadCell<SystemTray> = SingleThreadCell::new();

/// 利用可能な配列の一覧（名前, `YabLayout`, 左親指VK, 右親指VK）
static LAYOUTS: SingleThreadCell<Vec<(String, YabLayout, VkCode, VkCode)>> =
    SingleThreadCell::new();

/// キーイベントバッファ（IME ガード + 遅延キー + PassThrough 記憶）
pub(crate) static KEY_BUFFER: SingleThreadCell<key_buffer::KeyBuffer> = SingleThreadCell::new();

/// フォーカス検出のシングルスレッド状態を集約した構造体
pub(crate) static FOCUS: SingleThreadCell<focus::FocusDetector> = SingleThreadCell::new();

use crate::focus::cache::DetectionSource;

/// エンジン ON キー（変換キー等）— 起動時に設定から初期化（複数キー対応）
static ENGINE_ON_KEYS: SingleThreadCell<Vec<ParsedKeyCombo>> = SingleThreadCell::new();

/// エンジン OFF キー（Ctrl+無変換等）— 起動時に設定から初期化（複数キー対応）
static ENGINE_OFF_KEYS: SingleThreadCell<Vec<ParsedKeyCombo>> = SingleThreadCell::new();

/// Shadow IME state (UWP fallback when cross-process detection fails).
/// `true` = IME ON (safe default: engine processes keys).
static SHADOW_IME_ON: SingleThreadCell<bool> = SingleThreadCell::new();

/// IME sync toggle keys (VK codes, no modifiers)
static IME_SYNC_TOGGLE_KEYS: SingleThreadCell<Vec<u16>> = SingleThreadCell::new();

/// IME sync ON keys (VK codes, no modifiers)
static IME_SYNC_ON_KEYS: SingleThreadCell<Vec<u16>> = SingleThreadCell::new();

/// IME sync OFF keys (VK codes, no modifiers)
static IME_SYNC_OFF_KEYS: SingleThreadCell<Vec<u16>> = SingleThreadCell::new();

/// Ctrl+C 受信フラグ
static QUIT_REQUESTED: AtomicBool = AtomicBool::new(false);

/// フォーカス中コントロールの種別キャッシュ（Undetermined=2 で初期化）
pub(crate) static FOCUS_KIND: AtomicU8 = AtomicU8::new(2); // FocusKind::Undetermined

/// 起動時の警告を集約して報告する診断コレクター
struct StartupDiagnostics {
    warnings: Vec<String>,
}

impl StartupDiagnostics {
    fn new() -> Self {
        Self {
            warnings: Vec::new(),
        }
    }

    fn warn(&mut self, msg: impl Into<String>) {
        let msg = msg.into();
        log::warn!("startup: {}", msg);
        self.warnings.push(msg);
    }

    fn report(&self) {
        if self.warnings.is_empty() {
            return;
        }
        log::info!("{} startup warning(s):", self.warnings.len());
        for w in &self.warnings {
            log::info!("  - {}", w);
        }
        // Show tray balloon if tray is available
        unsafe {
            if let Some(tray) = TRAY.get_mut() {
                tray.show_balloon(
                    "awase",
                    &format!("{}件の警告があります", self.warnings.len()),
                );
            }
        }
    }
}

fn main() -> Result<()> {
    init_logging();
    let mut diag = StartupDiagnostics::new();
    let raw_config = load_config()?;
    let (config, config_warnings) = raw_config.validate();
    for w in &config_warnings {
        diag.warn(w);
    }
    let (layout_names, initial_layout_name) = init_engine_validated(&config, &mut diag)?;
    init_engine_toggle_keys(&config, &mut diag);
    init_ime_sync_keys(&config.ime_sync, &mut diag);
    init_ime(&mut diag);
    init_ngram_validated(&config, &mut diag);

    unsafe {
        OUTPUT.set(Output::new());
        KEY_BUFFER.set(key_buffer::KeyBuffer::new());
        FOCUS.set(focus::FocusDetector::new(config.focus_overrides.clone()));
    }

    init_tray(&layout_names, &initial_layout_name)?;
    install_hooks_and_hotkeys_validated(&config)?;
    diag.report();

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

/// 検証済み設定で配列の読み込みとエンジン初期化を行い、レイアウト名一覧とデフォルト名を返す
fn init_engine_validated(
    config: &ValidatedConfig,
    diag: &mut StartupDiagnostics,
) -> Result<(Vec<String>, String)> {
    let left_thumb_vk = VkCode(vk_name_to_code(&config.general.left_thumb_key).context(
        format!("Unknown VK name: {}", config.general.left_thumb_key),
    )?);
    let right_thumb_vk = VkCode(vk_name_to_code(&config.general.right_thumb_key).context(
        format!("Unknown VK name: {}", config.general.right_thumb_key),
    )?);

    let layouts_dir = resolve_relative(&config.general.layouts_dir);
    let layouts = scan_layouts(&layouts_dir, left_thumb_vk, right_thumb_vk, diag)?;
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
    config: &ValidatedConfig,
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

/// エンジン ON/OFF トグルキーの初期化（複数キー対応）
fn init_engine_toggle_keys(config: &ValidatedConfig, diag: &mut StartupDiagnostics) {
    let on_keys: Vec<ParsedKeyCombo> = config
        .general
        .engine_on_keys
        .iter()
        .filter_map(|s| {
            let result = parse_key_combo(s);
            if result.is_none() {
                diag.warn(format!("engine_on_keys のパースに失敗しました: {s}"));
            }
            result
        })
        .collect();
    log::info!(
        "Engine ON keys: {:?} ({} parsed)",
        config.general.engine_on_keys,
        on_keys.len()
    );
    unsafe {
        ENGINE_ON_KEYS.set(on_keys);
    }

    let off_keys: Vec<ParsedKeyCombo> = config
        .general
        .engine_off_keys
        .iter()
        .filter_map(|s| {
            let result = parse_key_combo(s);
            if result.is_none() {
                diag.warn(format!("engine_off_keys のパースに失敗しました: {s}"));
            }
            result
        })
        .collect();
    log::info!(
        "Engine OFF keys: {:?} ({} parsed)",
        config.general.engine_off_keys,
        off_keys.len()
    );
    unsafe {
        ENGINE_OFF_KEYS.set(off_keys);
    }
}

/// IME sync キーの初期化（shadow IME 状態追跡用）
fn init_ime_sync_keys(ime_sync: &ImeSyncConfig, diag: &mut StartupDiagnostics) {
    let mut parse_vk_list = |keys: &[String], label: &str| -> Vec<u16> {
        keys.iter()
            .filter_map(|s| {
                let code = vk_name_to_code(s);
                if code.is_none() {
                    diag.warn(format!("ime_sync.{label} のパースに失敗しました: {s}"));
                }
                code
            })
            .collect()
    };

    let toggle = parse_vk_list(&ime_sync.toggle_keys, "toggle_keys");
    let on = parse_vk_list(&ime_sync.on_keys, "on_keys");
    let off = parse_vk_list(&ime_sync.off_keys, "off_keys");

    log::info!(
        "IME sync keys: toggle={:?} on={:?} off={:?}",
        ime_sync.toggle_keys,
        ime_sync.on_keys,
        ime_sync.off_keys,
    );

    unsafe {
        IME_SYNC_TOGGLE_KEYS.set(toggle);
        IME_SYNC_ON_KEYS.set(on);
        IME_SYNC_OFF_KEYS.set(off);
        SHADOW_IME_ON.set(true); // safe default: engine ON
    }
}

/// IME プロバイダ初期化（TSF 優先、IMM32 フォールバック）
fn init_ime(diag: &mut StartupDiagnostics) {
    let ime_provider = HybridProvider::new();
    if !ime::is_japanese_input_language() {
        diag.warn("日本語キーボードが検出されませんでした");
    }
    log::info!(
        "IME provider initialized. Japanese keyboard: {}",
        ime::is_japanese_input_language()
    );
    unsafe {
        IME.set(ime_provider);
    }
}

/// 検証済み設定で n-gram モデルのロード（オプション）
fn init_ngram_validated(config: &ValidatedConfig, diag: &mut StartupDiagnostics) {
    let Some(ref ngram_path) = config.general.ngram_file else {
        return;
    };
    let ngram_path = resolve_relative(ngram_path);
    let content = match std::fs::read_to_string(&ngram_path) {
        Ok(c) => c,
        Err(e) => {
            diag.warn(format!(
                "n-gramファイル読込失敗: {}: {e}",
                ngram_path.display()
            ));
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
        Err(e) => diag.warn(format!("n-gramモデル解析失敗: {e}")),
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

/// 検証済み設定でフック登録とホットキー登録を行う
fn install_hooks_and_hotkeys_validated(config: &ValidatedConfig) -> Result<()> {
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
        ENGINE_ON_KEYS.clear();
        ENGINE_OFF_KEYS.clear();
        SHADOW_IME_ON.clear();
        IME_SYNC_TOGGLE_KEYS.clear();
        IME_SYNC_ON_KEYS.clear();
        IME_SYNC_OFF_KEYS.clear();
    }
    log::info!("Exited cleanly.");
}

/// Win32 タイマーランタイム
pub(crate) struct Win32TimerRuntime;

impl TimerRuntime for Win32TimerRuntime {
    type TimerId = usize;

    fn set_timer(&mut self, id: Self::TimerId, duration: std::time::Duration) {
        // Windows SetTimer API は u32 ミリ秒（最大 ~49日）。超過時は u32::MAX にキャップ。
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

/// キーコンボが修飾キー条件を含めてイベントに一致するか判定する。
///
/// 修飾キーの状態は `GetAsyncKeyState` で取得する（エンジン無効時は
/// エンジン内部の `ModifierState` が更新されないため OS 側で確認する必要がある）。
///
/// # Safety
/// `GetAsyncKeyState` は Win32 API。シングルスレッドフックコールバック内でのみ呼ぶこと。
unsafe fn matches_key_combo(combo: &ParsedKeyCombo, event: &RawKeyEvent) -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

    if event.vk_code.0 != combo.vk {
        return false;
    }

    let ctrl_held = GetAsyncKeyState(0x11) & (0x8000_u16 as i16) != 0;
    let shift_held = GetAsyncKeyState(0x10) & (0x8000_u16 as i16) != 0;
    let alt_held = GetAsyncKeyState(0x12) & (0x8000_u16 as i16) != 0;

    combo.ctrl == ctrl_held && combo.shift == shift_held && combo.alt == alt_held
}

/// エンジン ON/OFF トグルキーをチェックし、一致した場合は状態を変更して結果を返す。
///
/// # Safety
/// `ENGINE_ON_KEYS`, `ENGINE_OFF_KEYS`, `TRAY` はシングルスレッドからのみアクセスすること。
unsafe fn check_engine_toggle_keys(
    engine: &mut Engine,
    event: &RawKeyEvent,
) -> Option<CallbackResult> {
    // エンジン OFF → ON: engine_on_keys（デフォルト: VK_CONVERT）
    if !engine.is_enabled() {
        if let Some(keys) = ENGINE_ON_KEYS.get_ref() {
            if keys.iter().any(|k| matches_key_combo(k, event)) {
                let (enabled, flush_resp) = engine.set_enabled(true);
                let mut tr = Win32TimerRuntime;
                let mut ae = SendInputExecutor;
                dispatch(&flush_resp, &mut tr, &mut ae);
                log::info!("Engine ON (key combo)");
                if let Some(tray) = TRAY.get_mut() {
                    tray.set_enabled(enabled);
                }
                return Some(CallbackResult::Consumed);
            }
        }
    }

    // エンジン ON → OFF: engine_off_keys（デフォルト: Ctrl+VK_NONCONVERT）
    if engine.is_enabled() {
        if let Some(keys) = ENGINE_OFF_KEYS.get_ref() {
            if keys.iter().any(|k| matches_key_combo(k, event)) {
                let (enabled, flush_resp) = engine.set_enabled(false);
                let mut tr = Win32TimerRuntime;
                let mut ae = SendInputExecutor;
                dispatch(&flush_resp, &mut tr, &mut ae);
                log::info!("Engine OFF (key combo)");
                if let Some(tray) = TRAY.get_mut() {
                    tray.set_enabled(enabled);
                }
                return Some(CallbackResult::Consumed);
            }
        }
    }

    None
}

/// フックコールバック — キーイベント処理の中核
///
/// 処理フロー:
/// 1. Shadow IME 状態追跡（ime_sync キー）
/// 2. IME トグルガード（バッファリング）
/// 3. エンジン ON/OFF トグルキー
/// 4. IME 状態検出（クロスプロセス API or shadow）
/// 5. エンジン処理
unsafe fn on_key_event_callback(event: RawKeyEvent) -> CallbackResult {
    let Some(engine) = ENGINE.get_mut() else {
        return CallbackResult::PassThrough;
    };

    // ── Shadow IME state tracking (ime_sync keys) ──
    {
        let is_key_down = matches!(
            event.event_type,
            KeyEventType::KeyDown | KeyEventType::SysKeyDown
        );
        if is_key_down {
            let vk = event.vk_code.0;

            if let Some(on_keys) = IME_SYNC_ON_KEYS.get_ref() {
                if on_keys.contains(&vk) {
                    SHADOW_IME_ON.set(true);
                    log::debug!("Shadow IME ON (key 0x{vk:02X})");
                }
            }
            if let Some(off_keys) = IME_SYNC_OFF_KEYS.get_ref() {
                if off_keys.contains(&vk) {
                    SHADOW_IME_ON.set(false);
                    log::debug!("Shadow IME OFF (key 0x{vk:02X})");
                }
            }
            if let Some(toggle_keys) = IME_SYNC_TOGGLE_KEYS.get_ref() {
                if toggle_keys.contains(&vk) {
                    let current = SHADOW_IME_ON.get_ref().copied().unwrap_or(true);
                    let new_state = !current;
                    SHADOW_IME_ON.set(new_state);
                    log::debug!("Shadow IME toggle → {new_state} (key 0x{vk:02X})");
                }
            }
        }
    }

    // ── IME toggle guard: buffer keys after toggle to let IME state settle ──
    {
        let is_key_down = matches!(
            event.event_type,
            KeyEventType::KeyDown | KeyEventType::SysKeyDown
        );

        if is_key_down {
            // Check if current key IS a toggle/on/off key
            let is_toggle_key = IME_SYNC_TOGGLE_KEYS
                .get_ref()
                .is_some_and(|keys| keys.contains(&event.vk_code.0));
            let is_on_key = IME_SYNC_ON_KEYS
                .get_ref()
                .is_some_and(|keys| keys.contains(&event.vk_code.0));
            let is_off_key = IME_SYNC_OFF_KEYS
                .get_ref()
                .is_some_and(|keys| keys.contains(&event.vk_code.0));

            if is_toggle_key || is_on_key || is_off_key {
                // Set guard — next keys will be buffered
                if let Some(kb) = KEY_BUFFER.get_mut() {
                    kb.set_guard(true);
                }
                log::debug!("IME toggle guard ON (vk=0x{:02X})", event.vk_code.0);
                return CallbackResult::PassThrough; // let IME process the toggle
            }

            // While guard active, buffer character keys
            if let Some(kb) = KEY_BUFFER.get_mut() {
                if kb.is_guarded() {
                    kb.push_deferred(event);
                    let _ =
                        PostMessageW(HWND::default(), WM_PROCESS_DEFERRED, WPARAM(0), LPARAM(0));
                    return CallbackResult::Consumed;
                }
            }
        }

        // Guard clear on KeyUp of toggle key
        if !is_key_down {
            if let Some(kb) = KEY_BUFFER.get_mut() {
                if kb.is_guarded() {
                    let is_toggle_key = IME_SYNC_TOGGLE_KEYS
                        .get_ref()
                        .is_some_and(|keys| keys.contains(&event.vk_code.0));
                    let is_on_key = IME_SYNC_ON_KEYS
                        .get_ref()
                        .is_some_and(|keys| keys.contains(&event.vk_code.0));
                    let is_off_key = IME_SYNC_OFF_KEYS
                        .get_ref()
                        .is_some_and(|keys| keys.contains(&event.vk_code.0));
                    if is_toggle_key || is_on_key || is_off_key {
                        kb.set_guard(false);
                        // Process any buffered keys
                        let _ = PostMessageW(
                            HWND::default(),
                            WM_PROCESS_DEFERRED,
                            WPARAM(0),
                            LPARAM(0),
                        );
                    }
                }
            }
        }
    }

    // ── エンジン ON/OFF トグルキー ──
    {
        let is_key_down = matches!(
            event.event_type,
            KeyEventType::KeyDown | KeyEventType::SysKeyDown
        );
        if is_key_down {
            if let Some(result) = check_engine_toggle_keys(engine, &event) {
                return result;
            }
        }
    }

    // ── IME 状態検出（二層方式）──
    //
    // Layer 1: ImmGetDefaultIMEWnd + WM_IME_CONTROL（Win32 アプリ向けクロスプロセス検出）
    // Layer 2: Shadow IME state（UWP 等、Layer 1 失敗時のフォールバック）
    //
    // さらに HKL で日本語かどうかの基本チェックも行う。
    {
        use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyboardLayout;
        use windows::Win32::UI::WindowsAndMessaging::{
            GetGUIThreadInfo, GetWindowThreadProcessId, GUITHREADINFO,
        };

        let is_key_down = matches!(
            event.event_type,
            KeyEventType::KeyDown | KeyEventType::SysKeyDown
        );

        // Step 1: 対象スレッドの HKL を取得（日本語チェック）
        let mut gui_info = GUITHREADINFO {
            cbSize: size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        let thread_id = if GetGUIThreadInfo(0, &mut gui_info).is_ok() {
            let fg_hwnd = if gui_info.hwndFocus != HWND::default() {
                gui_info.hwndFocus
            } else {
                gui_info.hwndActive
            };
            let mut pid = 0u32;
            GetWindowThreadProcessId(fg_hwnd, Some(&mut pid))
        } else {
            0
        };

        let hkl = GetKeyboardLayout(thread_id);
        let lang_id = (hkl.0 as u32) & 0xFFFF;
        let is_japanese = lang_id == 0x0411;

        if !is_japanese {
            if is_key_down {
                log::trace!(
                    "IME: vk=0x{:02X} tid={thread_id} HKL=0x{lang_id:04X} → NonJapanese → passthrough",
                    event.vk_code.0,
                );
            }
            return CallbackResult::PassThrough;
        }

        // Step 2: Two-layer IME ON/OFF detection
        let ime_on = {
            // Layer 1: Direct API detection (Win32 apps)
            if let Some(open) = ime::detect_ime_open_cross_process() {
                if is_key_down {
                    log::trace!(
                        "IME: vk=0x{:02X} CrossProcess={open} → {}",
                        event.vk_code.0,
                        if open { "engine" } else { "passthrough" },
                    );
                }
                open
            } else {
                // Layer 2: Shadow state from key tracking (UWP fallback)
                let shadow = SHADOW_IME_ON.get_ref().copied().unwrap_or(true);
                if is_key_down {
                    log::trace!(
                        "IME: vk=0x{:02X} CrossProcess=None shadow={shadow} → {}",
                        event.vk_code.0,
                        if shadow { "engine" } else { "passthrough" },
                    );
                }
                shadow
            }
        };

        if !ime_on {
            return CallbackResult::PassThrough;
        }
    }

    // エンジン処理
    let response = engine.on_event(event);
    let mut timer_runtime = Win32TimerRuntime;
    let mut action_executor = SendInputExecutor;
    let consumed = dispatch(&response, &mut timer_runtime, &mut action_executor);

    if !response.actions.is_empty() {
        log::trace!(
            "Engine: vk=0x{:02X} consumed={consumed} actions={:?}",
            event.vk_code.0,
            response.actions,
        );
    }

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
            WM_TIMER if msg.wParam.0 == TIMER_UNDETERMINED_BUFFER => unsafe {
                key_buffer::handle_buffer_timeout();
            },
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
                // UIA 非同期判定完了 → メッセージから結果を取得
                let kind_u8 = msg.wParam.0 as u8;
                let result_hwnd = HWND(msg.lParam.0 as *mut _);
                let kind = FocusKind::from_u8(kind_u8);

                // 検証: UIA 結果の hwnd が現在のフォーカスと一致するか確認
                let mut info = GUITHREADINFO {
                    cbSize: size_of::<GUITHREADINFO>() as u32,
                    ..Default::default()
                };
                if GetGUIThreadInfo(0, &raw mut info).is_ok() && info.hwndFocus != result_hwnd {
                    log::debug!("UIA result for stale hwnd, ignoring");
                    // フォーカスが変わっているので適用しない
                } else {
                    // FOCUS_KIND を更新（メインスレッドのみが書き込む）
                    FocusKind::store(kind, &FOCUS_KIND);

                    // UIA 結果をキャッシュに反映
                    if let Some(f) = FOCUS.get_mut() {
                        if let Some((pid, cls)) = f.last_focus_info.as_ref() {
                            f.cache
                                .insert(*pid, cls.clone(), kind, DetectionSource::UiaAsync);
                        }
                    }
                    if kind == FocusKind::NonText {
                        invalidate_engine_context(ContextChange::FocusChanged);
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
    let raw_config = match load_config() {
        Ok(c) => c,
        Err(e) => {
            log::warn!("Failed to reload config: {e}");
            return;
        }
    };

    let (config, config_warnings) = raw_config.validate();
    for w in &config_warnings {
        log::warn!("config: {w}");
    }

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
    let mut reload_diag = StartupDiagnostics::new();
    init_ngram_validated(&config, &mut reload_diag);
    reload_diag.report();

    // エンジン ON/OFF キーの再読み込み
    {
        let mut reload_toggle_diag = StartupDiagnostics::new();
        // 既存の値をクリアしてから再設定
        ENGINE_ON_KEYS.clear();
        ENGINE_OFF_KEYS.clear();
        init_engine_toggle_keys(&config, &mut reload_toggle_diag);
        reload_toggle_diag.report();
    }

    // IME sync キーの再読み込み
    {
        let mut reload_sync_diag = StartupDiagnostics::new();
        IME_SYNC_TOGGLE_KEYS.clear();
        IME_SYNC_ON_KEYS.clear();
        IME_SYNC_OFF_KEYS.clear();
        init_ime_sync_keys(&config.ime_sync, &mut reload_sync_diag);
        reload_sync_diag.report();
    }

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
    diag: &mut StartupDiagnostics,
) -> Result<Vec<(String, YabLayout, VkCode, VkCode)>> {
    let mut layouts = Vec::new();

    if !layouts_dir.is_dir() {
        diag.warn(format!(
            "レイアウトディレクトリが見つかりません: {}",
            layouts_dir.display()
        ));
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
                        diag.warn(format!("レイアウト読込失敗: {}: {e}", path.display()));
                    }
                },
                Err(e) => {
                    diag.warn(format!("レイアウト読込失敗: {}: {e}", path.display()));
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

/// 設定ファイルのパスを探索する。
///
/// 検索順序:
/// 1. コマンドライン引数で指定されたパス
/// 2. 実行ファイルと同じディレクトリの `config.toml`
/// 3. カレントディレクトリの `config.toml`
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
