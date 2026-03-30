// コンソールウィンドウを非表示にする（タスクトレイで操作する）
#![windows_subsystem = "windows"]
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

mod app_state;
mod focus;
mod hook;
mod ime;
mod output;
mod single_thread_cell;
mod tray;
mod win32;

pub(crate) use app_state::{AppAction, AppState};

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
use timed_fsm::{dispatch, ActionExecutor, TimerRuntime};

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

/// フックで IME 制御キーを検出した際の即時キャッシュ更新要求
const WM_IME_KEY_DETECTED: u32 = WM_APP + 14;

/// フォーカス遷移デバウンス完了通知

/// Undetermined + IME ON バッファリングのタイマー ID
pub(crate) const TIMER_UNDETERMINED_BUFFER: usize = 100;

/// フォーカス遷移デバウンスタイマー ID
const TIMER_FOCUS_DEBOUNCE: usize = 103;

/// フォーカス遷移デバウンス時間（ミリ秒）
const FOCUS_DEBOUNCE_MS: u32 = 50;

/// IME の未確定状態を確認する（engine から呼び出し用）
#[allow(dead_code)] // engine 側から IME 未確定状態を確認する将来拡張用
pub fn check_ime_composing() -> bool {
    unsafe {
        APP.get_ref()
            .is_some_and(|app| app.ime.is_composing())
    }
}


/// キーイベントを SendInput で再注入する（IME OFF 時の遅延キー用）
///
/// INJECTED_MARKER 付きなのでフックに再捕捉されない。
///
/// # Safety
/// Win32 API (`send_input_safe`) を呼び出す。メインスレッドから呼ぶこと。
pub(crate) unsafe fn reinject_key(event: &RawKeyEvent) {
    use crate::output::INJECTED_MARKER;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE,
        VIRTUAL_KEY,
    };

    let is_keyup = matches!(
        event.event_type,
        KeyEventType::KeyUp | KeyEventType::SysKeyUp
    );

    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(event.vk_code.0),
                wScan: event.scan_code.0 as u16,
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
    win32::send_input_safe(&[input]);
}

pub(crate) static APP: SingleThreadCell<AppState> = SingleThreadCell::new();

use crate::focus::cache::DetectionSource;

/// Ctrl+C 受信フラグ
static QUIT_REQUESTED: AtomicBool = AtomicBool::new(false);

/// メインスレッド ID（Ctrl+C ハンドラから WM_QUIT を送るため）
static MAIN_THREAD_ID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// フォーカス中コントロールの種別キャッシュ（Undetermined=2 で初期化）
pub(crate) static FOCUS_KIND: AtomicU8 = AtomicU8::new(2); // FocusKind::Undetermined
pub(crate) static IME_RELIABILITY: AtomicU8 = AtomicU8::new(2); // ImeReliability::Unknown

/// キャッシュされた IME ON/OFF 状態。メッセージループで更新、フックで読み取り。
/// 0=OFF, 1=ON, 2=Unknown（初期状態）
pub(crate) static IME_STATE_CACHE: AtomicU8 = AtomicU8::new(2);

/// IME 状態ポーリング用タイマー ID（安全ネット: マウスで言語バー操作した場合等）
pub(crate) const TIMER_IME_POLL: usize = 101;

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
            if let Some(app) = APP.get_mut() {
                app.tray.show_balloon(
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
    let (engine, tracker, layouts, layout_names, initial_layout_name) =
        init_engine_validated(&config, &mut diag)?;
    let (engine_on_keys, engine_off_keys) = init_engine_toggle_keys(&config, &mut diag);
    let (ime_sync_toggle_keys, ime_sync_on_keys, ime_sync_off_keys) =
        init_ime_sync_keys(&config.ime_sync, &mut diag);
    let ime = init_ime(&mut diag);

    let system_tray = init_tray(&layout_names, &initial_layout_name)?;

    unsafe {
        APP.set(AppState {
            engine,
            tracker,
            output: Output::new(config.general.output_mode),
            ime,
            tray: system_tray,
            layouts,
            key_buffer: app_state::KeyBuffer::new(),
            focus: app_state::FocusDetector::new(config.focus_overrides.clone()),
            engine_on_keys,
            engine_off_keys,
            shadow_ime_on: true, // safe default: engine ON
            ime_sync_toggle_keys,
            ime_sync_on_keys,
            ime_sync_off_keys,
        });
    }

    init_ngram_validated(&config, &mut diag);
    install_hooks_and_hotkeys_validated(&config)?;
    diag.report();

    log::info!("Hook installed. Running message loop...");
    MAIN_THREAD_ID.store(
        unsafe { windows::Win32::System::Threading::GetCurrentThreadId() },
        Ordering::SeqCst,
    );
    install_ctrl_handler();
    install_focus_hook();

    // IME 状態ポーリングタイマー（安全ネット: マウスで言語バー操作等に対応）
    unsafe {
        let _ = SetTimer(HWND::default(), TIMER_IME_POLL, 500, None);
    }

    // Phase 3: UIA 非同期判定ワーカースレッドを起動
    let uia_tx = focus::uia::spawn_uia_worker();
    unsafe {
        if let Some(app) = APP.get_mut() {
            app.focus.set_uia_sender(uia_tx);
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
        "Default layout: {}, Threshold: {}ms, Output: {:?}",
        config.general.default_layout,
        config.general.simultaneous_threshold_ms,
        config.general.output_mode
    );
    Ok(config)
}

/// 検証済み設定で配列の読み込みとエンジン初期化を行い、構成要素を返す
fn init_engine_validated(
    config: &ValidatedConfig,
    diag: &mut StartupDiagnostics,
) -> Result<(
    Engine,
    awase::engine::input_tracker::InputTracker,
    Vec<(String, YabLayout, VkCode, VkCode)>,
    Vec<String>,
    String,
)> {
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

    let tracker = awase::engine::input_tracker::InputTracker::new(left_thumb_vk, right_thumb_vk);
    let engine = Engine::new(
        layout,
        left_thumb_vk,
        right_thumb_vk,
        config.general.simultaneous_threshold_ms,
        config.general.confirm_mode,
        config.general.speculative_delay_ms,
    );

    Ok((engine, tracker, layouts, layout_names, initial_layout_name))
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
fn init_engine_toggle_keys(
    config: &ValidatedConfig,
    diag: &mut StartupDiagnostics,
) -> (Vec<ParsedKeyCombo>, Vec<ParsedKeyCombo>) {
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

    (on_keys, off_keys)
}

/// IME sync キーの初期化（shadow IME 状態追跡用）
fn init_ime_sync_keys(
    ime_sync: &ImeSyncConfig,
    diag: &mut StartupDiagnostics,
) -> (Vec<u16>, Vec<u16>, Vec<u16>) {
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

    (toggle, on, off)
}

/// IME プロバイダ初期化（TSF 優先、IMM32 フォールバック）
fn init_ime(diag: &mut StartupDiagnostics) -> HybridProvider {
    let ime_provider = HybridProvider::new();
    if !ime::is_japanese_input_language() {
        diag.warn("日本語キーボードが検出されませんでした");
    }
    log::info!(
        "IME provider initialized. Japanese keyboard: {}",
        ime::is_japanese_input_language()
    );
    ime_provider
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
                if let Some(app) = APP.get_mut() {
                    app.engine.set_ngram_model(model);
                }
            }
        }
        Err(e) => diag.warn(format!("n-gramモデル解析失敗: {e}")),
    }
}

/// システムトレイアイコンを作成する
fn init_tray(layout_names: &[String], initial_layout_name: &str) -> Result<SystemTray> {
    let mut system_tray = SystemTray::new(true).context("Failed to create system tray icon")?;
    system_tray.set_layout_names(layout_names.to_vec());
    system_tray.set_layout_name(initial_layout_name);
    Ok(system_tray)
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
        let _ = KillTimer(HWND::default(), TIMER_IME_POLL);
        let _ = UnregisterHotKey(HWND::default(), HOTKEY_ID_TOGGLE);
        let _ = UnregisterHotKey(HWND::default(), HOTKEY_ID_FOCUS_OVERRIDE);
    }
    unsafe {
        if let Some(app) = APP.get_mut() {
            app.tray.destroy();
        }
        APP.clear();
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
            if let Some(app) = APP.get_mut() {
                app.output.send_keys(actions);
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
pub(crate) unsafe fn matches_key_combo(combo: &ParsedKeyCombo, event: &RawKeyEvent) -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

    if event.vk_code.0 != combo.vk {
        return false;
    }

    let ctrl_held = GetAsyncKeyState(0x11) & (0x8000_u16 as i16) != 0;
    let shift_held = GetAsyncKeyState(0x10) & (0x8000_u16 as i16) != 0;
    let alt_held = GetAsyncKeyState(0x12) & (0x8000_u16 as i16) != 0;

    combo.ctrl == ctrl_held && combo.shift == shift_held && combo.alt == alt_held
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
    let Some(app) = APP.get_mut() else {
        return CallbackResult::PassThrough;
    };

    // ── 入力レイヤー: 物理キー状態追跡 ──
    // IME チェックやエンジン無効等で処理レイヤーがスキップされても、
    // 修飾キー/親指キーの押下・解放を漏らさないために最初に呼ぶ。
    let phys = app.tracker.process(&event);

    // ── Shadow IME state tracking (ime_sync keys) ──
    {
        let is_key_down = matches!(
            event.event_type,
            KeyEventType::KeyDown | KeyEventType::SysKeyDown
        );
        if is_key_down {
            let vk = event.vk_code.0;

            if app.ime_sync_on_keys.contains(&vk) {
                app.shadow_ime_on = true;
                log::debug!("Shadow IME ON (key 0x{vk:02X})");
            }
            if app.ime_sync_off_keys.contains(&vk) {
                app.shadow_ime_on = false;
                log::debug!("Shadow IME OFF (key 0x{vk:02X})");
            }
            if app.ime_sync_toggle_keys.contains(&vk) {
                app.shadow_ime_on = !app.shadow_ime_on;
                log::debug!("Shadow IME toggle → {} (key 0x{vk:02X})", app.shadow_ime_on);
            }
        }
    }

    // ── IME 制御キー検出 → shadow 更新 + キャッシュ更新要求 ──
    {
        let vk = event.vk_code.0;
        let is_key_down = matches!(
            event.event_type,
            KeyEventType::KeyDown | KeyEventType::SysKeyDown
        );
        if is_key_down {
            // 日本語キーボード固有の IME ON/OFF キーで shadow を追跡する。
            // CrossProcess 検出が不正確なアプリ（Modern UI 等）では shadow が
            // 唯一の IME 状態ソースになるため、ここでの追跡が重要。
            match vk {
                // IME ON: 半角/全角（activate）、VK_IME_ON
                0xF2 | 0x16 => {
                    app.shadow_ime_on = true;
                    log::trace!("Shadow IME ON (vk=0x{vk:02X})");
                }
                // IME OFF: 半角/全角（deactivate）、VK_IME_OFF
                0xF3 | 0xF4 | 0x1A => {
                    app.shadow_ime_on = false;
                    log::trace!("Shadow IME OFF (vk=0x{vk:02X})");
                }
                // VK_KANJI (半角/全角トグル)
                0x19 => {
                    app.shadow_ime_on = !app.shadow_ime_on;
                    log::trace!("Shadow IME toggle → {} (vk=0x{vk:02X})", app.shadow_ime_on);
                }
                _ => {}
            }

            // IME 状態を変える可能性のあるキー → キャッシュ更新を要求
            let may_change_ime = awase::vk::is_ime_control(event.vk_code)
                || matches!(vk, 0xF0..=0xF5);
            if may_change_ime {
                let _ =
                    PostMessageW(HWND::default(), WM_IME_KEY_DETECTED, WPARAM(0), LPARAM(0));
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
            let is_toggle_key = app.ime_sync_toggle_keys.contains(&event.vk_code.0);
            let is_on_key = app.ime_sync_on_keys.contains(&event.vk_code.0);
            let is_off_key = app.ime_sync_off_keys.contains(&event.vk_code.0);

            if is_toggle_key || is_on_key || is_off_key {
                // Set guard — next keys will be buffered
                app.key_buffer.set_guard(true);
                log::debug!("IME toggle guard ON (vk=0x{:02X})", event.vk_code.0);
                return CallbackResult::PassThrough; // let IME process the toggle
            }

            // While IME guard active, buffer keys
            if app.key_buffer.is_guarded() {
                app.key_buffer.push_deferred(event, phys);
                let _ =
                    PostMessageW(HWND::default(), WM_PROCESS_DEFERRED, WPARAM(0), LPARAM(0));
                return CallbackResult::Consumed;
            }
        }

        // Guard clear on KeyUp of toggle key
        if !is_key_down {
            if app.key_buffer.is_guarded() {
                let is_toggle_key = app.ime_sync_toggle_keys.contains(&event.vk_code.0);
                let is_on_key = app.ime_sync_on_keys.contains(&event.vk_code.0);
                let is_off_key = app.ime_sync_off_keys.contains(&event.vk_code.0);
                if is_toggle_key || is_on_key || is_off_key {
                    app.key_buffer.set_guard(false);
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

    // ── エンジン ON/OFF トグルキー ──
    {
        let is_key_down = matches!(
            event.event_type,
            KeyEventType::KeyDown | KeyEventType::SysKeyDown
        );
        if is_key_down {
            if let Some(result) = app.check_engine_toggle_keys(&event) {
                return result;
            }
        }
    }

    // ── IME 状態判定（キャッシュ読み取りのみ — ノンブロッキング）──
    //
    // 実際の IME 検出（CrossProcess + ImeReliability + shadow fallback）は
    // メッセージループ上の refresh_ime_state_cache() で行い、結果を
    // IME_STATE_CACHE に書き込む。フックではキャッシュを読むだけ。
    {
        let cached = IME_STATE_CACHE.load(Ordering::Acquire);
        let ime_on = match cached {
            0 => false,
            1 => true,
            _ => {
                // Unknown（初期状態 or キャッシュ未更新）→ shadow fallback
                app.shadow_ime_on
            }
        };

        if !ime_on {
            return CallbackResult::PassThrough;
        }
    }

    // エンジン処理
    let response = app.engine.on_event(event, &phys);
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
                if let Some(app) = APP.get_mut() {
                    app.handle_buffer_timeout();
                }
            },
            WM_TIMER if msg.wParam.0 == TIMER_IME_POLL => unsafe {
                if let Some(app) = APP.get_ref() {
                    app.refresh_ime_state_cache();
                }
            },
            WM_IME_KEY_DETECTED => unsafe {
                // フックで IME 制御キーが検出された → 即座にキャッシュ更新
                if let Some(app) = APP.get_ref() {
                    app.refresh_ime_state_cache();
                }
            },
            WM_TIMER if msg.wParam.0 == TIMER_FOCUS_DEBOUNCE => unsafe {
                // フォーカス遷移デバウンス完了 → IME キャッシュ更新
                // キーはバッファせず、デバウンス中は前のキャッシュ値で処理される。
                let _ = KillTimer(HWND::default(), TIMER_FOCUS_DEBOUNCE);
                if let Some(app) = APP.get_ref() {
                    app.refresh_ime_state_cache();
                }
            },
            WM_TIMER if msg.wParam.0 == TIMER_PENDING || msg.wParam.0 == TIMER_SPECULATIVE => {
                let timer_id = msg.wParam.0;
                unsafe {
                    if let Some(app) = APP.get_mut() {
                        // IME が非活性な��� on_timeout せず flush（コンテキスト喪失）
                        let ime_active =
                            app.ime.is_active() && app.ime.get_mode().is_kana_input();
                        if !ime_active {
                            let response =
                                app.engine.flush_pending(ContextChange::ImeOff);
                            let mut timer_runtime = Win32TimerRuntime;
                            let mut action_executor = SendInputExecutor;
                            dispatch(&response, &mut timer_runtime, &mut action_executor);
                        } else {
                            let phys = app.tracker.snapshot();
                            let response = app.engine.on_timeout(timer_id, &phys);
                            let mut timer_runtime = Win32TimerRuntime;
                            let mut action_executor = SendInputExecutor;
                            dispatch(&response, &mut timer_runtime, &mut action_executor);
                        }
                    }
                }
            }
            WM_INPUTLANGCHANGE => unsafe {
                // 入力言語が変更された（Win+Space 等）→ 保留をフラッシュ + ガード ON
                // 言語切替直後は IME 状態が未反映の可能性があるため、
                // 後続キーをメッセージループに回して確実に更新後に処理する。
                log::info!("Input language changed, flushing pending state and enabling guard");
                if let Some(app) = APP.get_mut() {
                    app.invalidate_engine_context(ContextChange::InputLanguageChanged);
                    app.key_buffer.set_guard(true);
                    app.refresh_ime_state_cache();
                }
            },
            WM_PROCESS_DEFERRED => unsafe {
                // IME 制御キー後の遅延キーを再処理する。
                // この時点で IME 状態は確実に更新済み。
                if let Some(app) = APP.get_mut() {
                    app.process_deferred_keys();
                }
            },
            WM_FOCUS_KIND_UPDATE => unsafe {
                // UIA 非同期判定完了 → メッセージから結果を取得
                // wParam: 下位 8 bit = FocusKind, 次の 8 bit = ImeReliability
                let kind_u8 = msg.wParam.0 as u8;
                let reliability_u8 = (msg.wParam.0 >> 8) as u8;
                let result_hwnd = HWND(msg.lParam.0 as *mut _);
                let kind = FocusKind::from_u8(kind_u8);
                let reliability = awase::types::ImeReliability::from_u8(reliability_u8);

                // 検証: UIA 結果の hwnd が現在のフォーカスと一致するか確認
                let mut info = GUITHREADINFO {
                    cbSize: size_of::<GUITHREADINFO>() as u32,
                    ..Default::default()
                };
                if GetGUIThreadInfo(0, &raw mut info).is_ok() && info.hwndFocus != result_hwnd {
                    log::debug!("UIA result for stale hwnd, ignoring");
                    // フォーカスが変わっているので適用しない
                } else {
                    // ImeReliability を更新（常に適用）
                    reliability.store(&IME_RELIABILITY);

                    // FOCUS_KIND を更新（Undetermined の場合はスキップ）
                    if kind != FocusKind::Undetermined {
                        FocusKind::store(kind, &FOCUS_KIND);

                        // UIA 結果をキャッシュに反映
                        if let Some(app) = APP.get_mut() {
                            if let Some((pid, cls)) = app.focus.last_focus_info.as_ref() {
                                app.focus.cache
                                    .insert(*pid, cls.clone(), kind, DetectionSource::UiaAsync);
                            }
                        }
                        if kind == FocusKind::NonText {
                            if let Some(app) = APP.get_mut() {
                                app.invalidate_engine_context(ContextChange::FocusChanged);
                            }
                        }
                    }
                }
            },
            WM_HOTKEY if msg.wParam.0 == HOTKEY_ID_TOGGLE as usize => unsafe {
                if let Some(app) = APP.get_mut() {
                    app.toggle_engine();
                }
            },
            WM_HOTKEY if msg.wParam.0 == HOTKEY_ID_FOCUS_OVERRIDE as usize => unsafe {
                if let Some(app) = APP.get_mut() {
                    app.toggle_focus_override();
                }
            },
            WM_APP => unsafe {
                let layout_names: Vec<String> = APP
                    .get_ref()
                    .map(|app| app.layouts.iter().map(|(name, ..)| name.clone()).collect())
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
                        if let Some(app) = APP.get_mut() {
                            app.toggle_engine();
                        }
                    } else if cmd == tray::cmd_exit() {
                        PostQuitMessage(0);
                    } else if cmd >= tray::cmd_layout_base() {
                        let index = usize::from(cmd - tray::cmd_layout_base());
                        if let Some(app) = APP.get_mut() {
                            app.switch_layout(index);
                        }
                    }
                }
            },
            _ => unsafe {
                DispatchMessageW(&raw const msg);
            },
        }
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

    if let Some(app) = APP.get_mut() {
        app.engine.set_threshold_ms(config.general.simultaneous_threshold_ms);
        app.engine.set_confirm_mode(
            config.general.confirm_mode,
            config.general.speculative_delay_ms,
        );
        app.output.set_mode(config.general.output_mode);
        log::info!(
            "Engine parameters updated: threshold={}ms, confirm_mode={:?}, speculative_delay={}ms, output_mode={:?}",
            config.general.simultaneous_threshold_ms,
            config.general.confirm_mode,
            config.general.speculative_delay_ms,
            config.general.output_mode,
        );
    }

    // n-gram モデルの再読み込み
    let mut reload_diag = StartupDiagnostics::new();
    init_ngram_validated(&config, &mut reload_diag);
    reload_diag.report();

    // エンジン ON/OFF キーの再読み込み
    {
        let mut reload_toggle_diag = StartupDiagnostics::new();
        let (on_keys, off_keys) = init_engine_toggle_keys(&config, &mut reload_toggle_diag);
        if let Some(app) = APP.get_mut() {
            app.engine_on_keys = on_keys;
            app.engine_off_keys = off_keys;
        }
        reload_toggle_diag.report();
    }

    // IME sync キーの再読み込み
    {
        let mut reload_sync_diag = StartupDiagnostics::new();
        let (toggle, on, off) = init_ime_sync_keys(&config.ime_sync, &mut reload_sync_diag);
        if let Some(app) = APP.get_mut() {
            app.ime_sync_toggle_keys = toggle;
            app.ime_sync_on_keys = on;
            app.ime_sync_off_keys = off;
        }
        reload_sync_diag.report();
    }

    // フォーカスオーバーライド再読み込み + キャッシュクリア
    if let Some(app) = APP.get_mut() {
        app.focus.overrides = config.focus_overrides;
        app.focus.cache = focus::cache::FocusCache::new();
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

/// `WINEVENT_OUTOFCONTEXT` (0x0000) — コールバックをメッセージループで実行
const WINEVENT_OUTOFCONTEXT: u32 = 0x0000;

/// `EVENT_OBJECT_FOCUS` (0x8005) — フォーカス変更イベント
const EVENT_OBJECT_FOCUS: u32 = 0x8005;

/// フォーカス変更イベントフックを登録する
fn install_focus_hook() {
    use windows::Win32::UI::Accessibility::SetWinEventHook;
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

/// フォーカス変更イベントのコールバック（メッセージループ上で実行される）
unsafe extern "system" fn win_event_proc(
    _hook: windows::Win32::UI::Accessibility::HWINEVENTHOOK,
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

    let process_id = focus::classify::get_window_process_id(hwnd);
    let class_name = focus::classify::get_class_name_string(hwnd);

    let Some(app) = APP.get_mut() else {
        return;
    };
    let actions = app.on_focus_changed(hwnd, process_id, &class_name);

    for action in actions {
        match action {
            AppAction::InvalidateEngineContext(reason) => {
                app.invalidate_engine_context(reason);
            }
            AppAction::RefreshImeStateCache => {
                // 即座に更新せず、デバウンスタイマーをセット（リセット）。
                // フォーカス遷移中は中間ウィンドウの IME 状態が不正確なため、
                // 最終ウィンドウに落ち着いてから更新する。
                // その間のキーはフォーカスガードでバッファされる。
                let _ = SetTimer(
                    HWND::default(),
                    TIMER_FOCUS_DEBOUNCE,
                    FOCUS_DEBOUNCE_MS,
                    None,
                );
            }
            _ => {}
        }
    }
}

/// Ctrl+C ハンドラを登録（Win32 SetConsoleCtrlHandler）
fn install_ctrl_handler() {
    unsafe extern "system" fn handler(_ctrl_type: u32) -> windows::Win32::Foundation::BOOL {
        use windows::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_QUIT};
        QUIT_REQUESTED.store(true, Ordering::SeqCst);
        // PostQuitMessage は呼び出しスレッドのキューに WM_QUIT をポストするが、
        // SetConsoleCtrlHandler のコールバックは別スレッドで呼ばれるため、
        // メインスレッドには届かない。PostThreadMessageW でメインスレッドに送る。
        let tid = MAIN_THREAD_ID.load(Ordering::SeqCst);
        if tid != 0 {
            let _ = PostThreadMessageW(tid, WM_QUIT, WPARAM(0), LPARAM(0));
        }
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
