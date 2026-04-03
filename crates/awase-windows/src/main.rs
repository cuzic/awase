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

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use anyhow::{Context, Result};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetGUIThreadInfo, GetMessageW, KillTimer, PostQuitMessage,
    RegisterWindowMessageW, SetTimer, GUITHREADINFO, MSG, WM_APP, WM_COMMAND, WM_HOTKEY,
    WM_INPUTLANGCHANGE, WM_POWERBROADCAST, WM_TIMER,
};

use awase::config::{AppConfig, ImeDetectConfig, ParsedKeyCombo, ValidatedConfig};
use awase::engine::{Engine, InputContext, NicolaFsm, TIMER_PENDING, TIMER_SPECULATIVE};
use awase::engine::{ImeSyncKeys, SpecialKeyCombos};
use awase::ngram::NgramModel;
use awase::types::{ContextChange, FocusKind};
use awase::types::{RawKeyEvent, VkCode};
use awase::yab::YabLayout;
use awase_windows::vk::{parse_key_combo, vk_name_to_code};

use awase_windows::executor;
use awase_windows::focus;
use awase_windows::hook;
use awase_windows::hook::CallbackResult;
use awase_windows::ime;
use awase_windows::ime::HybridProvider;
use awase_windows::observer;
use awase_windows::output::Output;
use awase_windows::platform;
use awase_windows::runtime;
use awase_windows::tray;
use awase_windows::tray::SystemTray;
use awase_windows::{
    LayoutEntry, Runtime, APP, ELEVATED, FOCUS_DEBOUNCE_MS, FOCUS_KIND, IME_POLL_INTERVAL_MS,
    MAIN_THREAD_ID, QUIT_REQUESTED, TIMER_FOCUS_DEBOUNCE,
    TIMER_HOOK_WATCHDOG, TIMER_IME_POLL, WM_EXECUTE_EFFECTS, WM_FOCUS_KIND_UPDATE,
    WM_IME_KEY_DETECTED, WM_PANIC_RESET, WM_PROCESS_DEFERRED, WM_RELOAD_CONFIG,
};

/// 有効/無効切り替えホットキー ID
const HOTKEY_ID_TOGGLE: i32 = 1;

/// 手動フォーカスオーバーライドホットキー ID (Ctrl+Shift+F11)
const HOTKEY_ID_FOCUS_OVERRIDE: i32 = 2;

use awase_windows::focus::cache::DetectionSource;

// ── セッション変更通知（WTS）定数 ──

/// `WM_WTSSESSION_CHANGE` — セッションの状態変更通知メッセージ
const WM_WTSSESSION_CHANGE: u32 = 0x02B1;
/// セッションがロックされた
const WTS_SESSION_LOCK: u32 = 7;
/// セッションがアンロックされた
const WTS_SESSION_UNLOCK: u32 = 8;
/// 現在のセッションのみ通知を受け取る
const NOTIFY_FOR_THIS_SESSION: u32 = 0;

#[link(name = "wtsapi32")]
extern "system" {
    fn WTSRegisterSessionNotification(hwnd: HWND, flags: u32) -> windows::Win32::Foundation::BOOL;
    fn WTSUnRegisterSessionNotification(hwnd: HWND) -> windows::Win32::Foundation::BOOL;
}

/// 起動時の警告を集約して報告する診断コレクター
struct StartupDiagnostics {
    warnings: Vec<String>,
}

impl StartupDiagnostics {
    const fn new() -> Self {
        Self {
            warnings: Vec::new(),
        }
    }

    fn warn(&mut self, msg: impl Into<String>) {
        let msg = msg.into();
        log::warn!("startup: {msg}");
        self.warnings.push(msg);
    }

    fn report(&self) {
        if self.warnings.is_empty() {
            return;
        }
        log::info!("{} startup warning(s):", self.warnings.len());
        for w in &self.warnings {
            log::info!("  - {w}");
        }
        // Show tray balloon if tray is available
        unsafe {
            if let Some(app) = APP.get_mut() {
                app.executor.platform.tray.show_balloon(
                    "awase",
                    &format!("{}件の警告があります", self.warnings.len()),
                );
            }
        }
    }
}

fn main() -> Result<()> {
    init_logging();

    // 多重起動防止: Named Mutex で既存インスタンスをチェック
    unsafe {
        use windows::core::w;
        use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
        use windows::Win32::System::Threading::CreateMutexW;

        let mutex = CreateMutexW(None, false, w!("Global\\awase_keyboard_emulator"));
        match mutex {
            Ok(handle) => {
                if GetLastError() == ERROR_ALREADY_EXISTS {
                    log::error!("Another instance of awase is already running. Exiting.");
                    let _ = windows::Win32::Foundation::CloseHandle(handle);
                    std::process::exit(1);
                }
                // ハンドルをプロセス終了まで保持する（ドロップさせない）
                std::mem::forget(handle);
            }
            Err(e) => {
                log::warn!("Failed to create instance mutex: {e}");
                // Mutex 作成失敗は起動を妨げない
            }
        }
    }

    let mut diag = StartupDiagnostics::new();

    // 管理者権限チェック
    let elevated = tray::is_elevated();
    ELEVATED.store(elevated, Ordering::Relaxed);
    if elevated {
        log::info!("Running with administrator privileges");
    } else {
        log::warn!(
            "Running without administrator privileges — \
             keyboard hook will not work in elevated windows (e.g. Task Manager)"
        );
    }

    let mut raw_config = load_config()?;
    handle_auto_start(&mut raw_config);
    let (config, config_warnings) = raw_config.validate();
    for w in &config_warnings {
        diag.warn(w);
    }
    let (fsm, tracker, layouts, layout_names, initial_layout_name) =
        init_engine_validated(&config, &mut diag)?;
    let engine_on_keys = parse_key_combos(&config.keys.engine_on, "Engine ON keys", &mut diag);
    let engine_off_keys = parse_key_combos(&config.keys.engine_off, "Engine OFF keys", &mut diag);
    let ime_control_on_keys =
        parse_key_combos(&config.keys.ime_on, "IME control ON keys", &mut diag);
    let ime_control_off_keys =
        parse_key_combos(&config.keys.ime_off, "IME control OFF keys", &mut diag);
    let (ime_sync_toggle, ime_sync_on, ime_sync_off) =
        init_ime_sync_keys(&config.keys.ime_detect, &mut diag);
    let ime = init_ime(&mut diag);

    let system_tray = init_tray(&layout_names, &initial_layout_name, elevated)?;

    // Clone sync keys for Runtime's event enrichment
    let sync_toggle_keys = ime_sync_toggle.clone();
    let sync_on_keys = ime_sync_on.clone();
    let sync_off_keys = ime_sync_off.clone();

    let engine = Engine::new(
        fsm,
        tracker,
        ImeSyncKeys {
            toggle: ime_sync_toggle,
            on: ime_sync_on,
            off: ime_sync_off,
        },
        SpecialKeyCombos {
            engine_on: engine_on_keys,
            engine_off: engine_off_keys,
            ime_on: ime_control_on_keys,
            ime_off: ime_control_off_keys,
        },
    );

    initialize_app(
        engine,
        system_tray,
        &config,
        ime,
        layouts,
        sync_toggle_keys,
        sync_on_keys,
        sync_off_keys,
    );
    store_timing_config(&config);

    init_ngram_validated(&config, &mut diag);
    let (hook_guard, _toggle_hotkey_guard, _focus_override_hotkey_guard) =
        install_hooks_and_hotkeys_validated(&config)?;
    diag.report();

    log::info!("Hook installed. Running message loop...");
    MAIN_THREAD_ID.store(
        unsafe { windows::Win32::System::Threading::GetCurrentThreadId() },
        Ordering::SeqCst,
    );
    install_ctrl_handler();
    let _focus_hook_guard = install_focus_hook();

    // IME 状態ポーリングタイマー + ウォッチドッグタイマー
    // Win32Timer 経由で設定（OS ID マッピングは内部で管理）
    unsafe {
        if let Some(app) = APP.get_mut() {
            app.executor.platform.timer.set(
                TIMER_IME_POLL,
                std::time::Duration::from_millis(u64::from(
                    IME_POLL_INTERVAL_MS.load(Ordering::Relaxed),
                )),
            );
            app.executor
                .platform
                .timer
                .set(TIMER_HOOK_WATCHDOG, std::time::Duration::from_secs(10));
        }
    }

    // Phase 3: UIA 非同期判定ワーカースレッドを起動
    let uia_tx = focus::uia::spawn_uia_worker();
    unsafe {
        if let Some(app) = APP.get_mut() {
            app.executor.platform.focus.set_uia_sender(uia_tx);
        }
    }

    let _wts_guard = register_session_notification();
    initialize_ime_cache();

    // Explorer 再起動時にトレイアイコンを復元するため TaskbarCreated メッセージを登録
    let taskbar_created_msg =
        unsafe { RegisterWindowMessageW(windows::core::w!("TaskbarCreated")) };

    run_message_loop(taskbar_created_msg);
    cleanup();
    drop(hook_guard);

    Ok(())
}

/// ログ初期化
///
/// `#![windows_subsystem = "windows"]` でコンソールがないため、
/// ログはファイル（実行ファイルと同じディレクトリの `awase.log`）に出力する。
fn init_logging() {
    let log_path = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("awase.log")))
        .unwrap_or_else(|| std::path::PathBuf::from("awase.log"));

    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path);

    let mut builder =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"));
    builder.format_timestamp_millis();

    if let Ok(file) = log_file {
        builder.target(env_logger::Target::Pipe(Box::new(file)));
    }
    // ファイルが開けない場合は stderr フォールバック（コンソール起動時用）

    builder.init();
    log::info!(
        "Keyboard Layout Emulator starting... (log → {})",
        log_path.display()
    );
}

/// 設定ファイルを読み込む
fn load_config() -> Result<AppConfig> {
    let config_path = find_config_path()?;
    log::info!("Loading config from: {}", config_path.display());
    let config = AppConfig::load(&config_path)?;
    log::info!(
        "Default layout: {}, Threshold: {}ms, Output: {:?}, Hook: {:?}",
        config.general.default_layout,
        config.general.simultaneous_threshold_ms,
        config.general.output_mode,
        config.general.hook_mode,
    );
    Ok(config)
}

/// 自動起動の設定を処理する
///
/// `auto_start` の値に応じて Task Scheduler への登録/解除を行う。
/// "ask" の場合はダイアログで確認し、結果を config.toml に保存する。
fn handle_auto_start(config: &mut AppConfig) {
    use awase_windows::autostart;

    match config.general.auto_start.as_str() {
        "ask" => {
            if !autostart::is_registered() {
                if autostart::ask_user() {
                    autostart::register();
                    config.general.auto_start = "enabled".to_string();
                } else {
                    config.general.auto_start = "disabled".to_string();
                }
                // 設定を保存して次回以降は確認しない
                if let Ok(config_path) = find_config_path() {
                    if let Err(e) = config.save(&config_path) {
                        log::error!("Failed to save auto_start setting: {e}");
                    }
                }
            }
        }
        "enabled" => {
            if !autostart::is_registered() {
                autostart::register();
            }
        }
        "disabled" => {
            if autostart::is_registered() {
                autostart::unregister();
            }
        }
        other => {
            log::warn!("Unknown auto_start value: {other}, ignoring");
        }
    }
}

/// 検証済み設定で配列の読み込みとエンジン初期化を行い、構成要素を返す
fn init_engine_validated(
    config: &ValidatedConfig,
    diag: &mut StartupDiagnostics,
) -> Result<(
    NicolaFsm,
    awase::engine::input_tracker::InputTracker,
    Vec<LayoutEntry>,
    Vec<String>,
    String,
)> {
    let left_thumb_vk = vk_name_to_code(&config.general.left_thumb_key).context(format!(
        "Unknown VK name: {}",
        config.general.left_thumb_key
    ))?;
    let right_thumb_vk = vk_name_to_code(&config.general.right_thumb_key).context(format!(
        "Unknown VK name: {}",
        config.general.right_thumb_key
    ))?;

    let layouts_dir = resolve_relative(&config.general.layouts_dir);
    let layouts = scan_layouts(&layouts_dir, left_thumb_vk, right_thumb_vk, diag)?;
    let layout_names: Vec<String> = layouts.iter().map(|e| e.name.clone()).collect();
    log::info!("Available layouts: {layout_names:?}");

    let (layout, initial_layout_name) = select_default_layout(&layouts, config);
    log::info!(
        "Layout loaded: {} normal keys, {} left thumb keys, {} right thumb keys",
        layout.normal.len(),
        layout.left_thumb.len(),
        layout.right_thumb.len()
    );

    let tracker = awase::engine::input_tracker::InputTracker::new();
    // Set thumb VK codes for hook classification
    hook::set_thumb_vk_codes(left_thumb_vk, right_thumb_vk);
    let engine = NicolaFsm::new(
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
fn select_default_layout(layouts: &[LayoutEntry], config: &ValidatedConfig) -> (YabLayout, String) {
    let default_name = config.general.default_layout.trim_end_matches(".yab");
    let index = layouts
        .iter()
        .position(|e| e.name == default_name)
        .unwrap_or(0);
    let entry = &layouts[index];
    (entry.layout.clone(), entry.name.clone())
}

/// キーコンボ文字列のリストをパースし、失敗時は診断に警告を出す
fn parse_key_combos(
    keys: &[String],
    label: &str,
    diag: &mut StartupDiagnostics,
) -> Vec<ParsedKeyCombo> {
    let parsed: Vec<ParsedKeyCombo> = keys
        .iter()
        .filter_map(|s| {
            parse_key_combo(s).or_else(|| {
                diag.warn(format!("{label} のパースに失敗しました: {s}"));
                None
            })
        })
        .collect();
    log::info!("{label}: {keys:?} ({} parsed)", parsed.len());
    parsed
}

/// IME sync キーの初期化（shadow IME 状態追跡用）
fn init_ime_sync_keys(
    ime_detect: &ImeDetectConfig,
    diag: &mut StartupDiagnostics,
) -> (Vec<VkCode>, Vec<VkCode>, Vec<VkCode>) {
    let mut parse_vk_list = |keys: &[String], label: &str| -> Vec<VkCode> {
        keys.iter()
            .filter_map(|s| {
                vk_name_to_code(s).or_else(|| {
                    diag.warn(format!(
                        "keys.ime_detect.{label} のパースに失敗しました: {s}"
                    ));
                    None
                })
            })
            .collect()
    };

    let toggle = parse_vk_list(&ime_detect.toggle, "toggle");
    let on = parse_vk_list(&ime_detect.on, "on");
    let off = parse_vk_list(&ime_detect.off, "off");

    log::info!(
        "IME detect keys: toggle={:?} on={:?} off={:?}",
        ime_detect.toggle,
        ime_detect.on,
        ime_detect.off,
    );

    (toggle, on, off)
}

/// IME プロバイダ初期化（TSF 優先、IMM32 フォールバック）
fn init_ime(diag: &mut StartupDiagnostics) -> HybridProvider {
    let ime_provider = HybridProvider::new();
    check_keyboard_layout(diag);
    ime_provider
}

/// キーボードレイアウトが日本語(106/109)かどうかを検証し、警告を出す
fn check_keyboard_layout(diag: &mut StartupDiagnostics) {
    let (is_japanese, lang_id) = ime::keyboard_layout_info();
    log::info!("Keyboard layout: LANGID=0x{lang_id:04X}, Japanese={is_japanese}",);
    if !is_japanese {
        // 英語キーボード (0x0409) の場合、より具体的なメッセージを出す
        if lang_id == 0x0409 {
            diag.warn(
                "英語キーボード(101/102)が検出されました。\
                 親指シフトには日本語キーボードレイアウト(106/109)が必要です。\
                 設定 → 時刻と言語 → 言語と地域 → 日本語 → キーボードレイアウト で\
                 「日本語キーボード(106/109キー)」に変更してください。\
                 ※ Windows Update 後にレイアウトが英語に戻る場合があります。",
            );
        } else {
            diag.warn(format!(
                "日本語キーボード(106/109)が検出されませんでした(LANGID=0x{lang_id:04X})。\
                 親指シフトには日本語キーボードレイアウトが必要です。\
                 設定 → 時刻と言語 → 言語と地域 → 日本語 → キーボードレイアウト で変更できます。"
            ));
        }
    }
}

/// `WM_INPUTLANGCHANGE` 時にキーボードレイアウトを検証する
fn check_keyboard_layout_on_change() {
    let (is_japanese, lang_id) = ime::keyboard_layout_info();
    if !is_japanese {
        if lang_id == 0x0409 {
            log::warn!(
                "Input language changed to English keyboard (101/102). \
                 Thumb-shift requires Japanese keyboard layout (106/109). \
                 LANGID=0x{lang_id:04X}",
            );
        } else {
            log::warn!(
                "Input language changed to non-Japanese layout (LANGID=0x{lang_id:04X}). \
                 Thumb-shift requires Japanese keyboard layout (106/109).",
            );
        }
        // バルーン通知で警告
        unsafe {
            if let Some(app) = APP.get_mut() {
                app.executor.platform.tray.show_balloon(
                    "awase",
                    "日本語キーボードレイアウトが検出されません。親指シフトが正常に動作しない可能性があります。",
                );
            }
        }
    }
}

/// 検証済み設定で n-gram モデルのロード（オプション）
fn init_ngram_validated(config: &ValidatedConfig, diag: &mut StartupDiagnostics) {
    let Some(ref ngram_path) = config.general.ngram_file else {
        return;
    };
    let ngram_path = resolve_relative(ngram_path);
    let base_us = u64::from(config.general.simultaneous_threshold_ms) * 1000;
    let range_us = u64::from(config.general.ngram_adjustment_range_ms) * 1000;
    let min_us = u64::from(config.general.ngram_min_threshold_ms) * 1000;
    let max_us = u64::from(config.general.ngram_max_threshold_ms) * 1000;
    match NgramModel::from_file(&ngram_path, base_us, range_us, min_us, max_us) {
        Ok(model) => {
            log::info!("N-gram model loaded from {}", ngram_path.display());
            unsafe {
                if let Some(app) = APP.get_mut() {
                    app.engine
                        .on_command(awase::engine::EngineCommand::SetNgramModel(model), &crate::runtime::build_input_context());
                }
            }
        }
        Err(e) => diag.warn(format!("n-gramモデル解析失敗: {e}")),
    }
}

/// システムトレイアイコンを作成する
fn init_tray(
    layout_names: &[String],
    initial_layout_name: &str,
    elevated: bool,
) -> Result<SystemTray> {
    let mut system_tray =
        SystemTray::new(true, elevated).context("Failed to create system tray icon")?;
    system_tray.set_layout_names(layout_names.to_vec());
    system_tray.set_layout_name(initial_layout_name);
    Ok(system_tray)
}

/// 検証済み設定でフック登録とホットキー登録を行う
fn install_hooks_and_hotkeys_validated(
    config: &ValidatedConfig,
) -> Result<(hook::HookGuard, Option<HotKeyGuard>, Option<HotKeyGuard>)> {
    let callback = Box::new(|event: RawKeyEvent| -> CallbackResult {
        unsafe { on_key_event_callback(event) }
    });
    let guard = hook::install_hook(callback).context("Failed to install keyboard hook")?;

    let toggle_guard = config
        .general
        .engine_toggle_hotkey
        .as_ref()
        .and_then(|hotkey_str| register_toggle_hotkey(hotkey_str));
    let focus_override_guard = register_focus_override_hotkey();
    Ok((guard, toggle_guard, focus_override_guard))
}

/// `WTSRegisterSessionNotification` の RAII ガード。Drop 時に解除する。
struct WtsGuard(HWND);

impl Drop for WtsGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = WTSUnRegisterSessionNotification(self.0);
        }
        log::info!("WTS session notification unregistered");
    }
}

// タイマーは Win32Timer が管理するため RAII ガード不要

/// `RegisterHotKey` の RAII ガード。Drop 時に `UnregisterHotKey` を呼ぶ。
struct HotKeyGuard(i32);

impl Drop for HotKeyGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = UnregisterHotKey(HWND::default(), self.0);
        }
        log::info!("Hotkey {} unregistered", self.0);
    }
}

/// トグルホットキーを登録する
fn register_toggle_hotkey(hotkey_str: &str) -> Option<HotKeyGuard> {
    if let Some((modifiers, vk)) = awase_windows::vk::parse_hotkey(hotkey_str) {
        unsafe {
            let result = RegisterHotKey(
                HWND::default(),
                HOTKEY_ID_TOGGLE,
                HOT_KEY_MODIFIERS(modifiers),
                u32::from(vk.0),
            );
            if result.is_ok() {
                log::info!("Toggle hotkey registered: {hotkey_str}");
                Some(HotKeyGuard(HOTKEY_ID_TOGGLE))
            } else {
                log::warn!("Failed to register toggle hotkey: {hotkey_str}");
                None
            }
        }
    } else {
        log::warn!("Invalid toggle hotkey format: {hotkey_str}");
        None
    }
}

/// 手動フォーカスオーバーライドホットキー (Ctrl+Shift+F11) を登録する
fn register_focus_override_hotkey() -> Option<HotKeyGuard> {
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
            Some(HotKeyGuard(HOTKEY_ID_FOCUS_OVERRIDE))
        } else {
            log::warn!("Failed to register focus override hotkey: Ctrl+Shift+F11");
            None
        }
    }
}

/// セッション変更通知（画面ロック/アンロック）を登録する
fn register_session_notification() -> Option<WtsGuard> {
    unsafe {
        APP.get_ref().and_then(|app| {
            let tray_hwnd = app.executor.platform.tray.hwnd();
            if WTSRegisterSessionNotification(tray_hwnd, NOTIFY_FOR_THIS_SESSION).as_bool() {
                log::info!("WTS session notification registered");
                Some(WtsGuard(tray_hwnd))
            } else {
                log::warn!("Failed to register WTS session notification");
                None
            }
        })
    }
}

/// タイミング設定を Atomic に書き込み（コールバックから参照）
/// APP グローバルの初期化
fn initialize_app(
    engine: Engine,
    tray: SystemTray,
    config: &ValidatedConfig,
    ime: HybridProvider,
    layouts: Vec<LayoutEntry>,
    sync_toggle_keys: Vec<VkCode>,
    sync_on_keys: Vec<VkCode>,
    sync_off_keys: Vec<VkCode>,
) {
    unsafe {
        APP.set(Runtime {
            engine,
            executor: executor::DecisionExecutor::new(
                platform::WindowsPlatform {
                    output: Output::new(config.general.output_mode),
                    tray,
                    focus: runtime::FocusDetector::new(config.focus_overrides.clone()),
                    timer: awase_windows::timer::Win32Timer::new(),
                },
                config.general.hook_mode,
            ),
            ime,
            layouts,
            sync_toggle_keys,
            sync_on_keys,
            sync_off_keys,
        });
        RAPID_IME_TIMESTAMPS.set(RapidPressTracker::new());
    }
}

fn store_timing_config(config: &ValidatedConfig) {
    FOCUS_DEBOUNCE_MS.store(config.general.focus_debounce_ms, Ordering::Relaxed);
    IME_POLL_INTERVAL_MS.store(config.general.ime_poll_interval_ms, Ordering::Relaxed);
}

/// 起動時に IME 状態キャッシュを初期化する（Unknown → 実際の値）。
fn initialize_ime_cache() {
    unsafe {
        if let Some(app) = APP.get_mut() {
            app.refresh_ime_state_cache();
        }
    }
}

/// クリーンアップ処理（フック解除は HookGuard の Drop で行われる）
fn cleanup() {
    unsafe {
        APP.clear();
    }
    log::info!("Exited cleanly.");
}

// ── パニックリセット: IME 関連キー連打検出 ──

/// IME 関連キー押下のタイムスタンプ（循環バッファ）。
/// フックコールバックはメインスレッドで実行されるため `SingleThreadCell` で十分。
static RAPID_IME_TIMESTAMPS: awase_windows::SingleThreadCell<RapidPressTracker> =
    awase_windows::SingleThreadCell::new();

/// 連打検出用の軽量トラッカー
struct RapidPressTracker {
    /// 直近のタイムスタンプ（最大 `THRESHOLD` 個保持）
    buf: [u64; 3],
    /// 次の書き込み位置
    cursor: usize,
    /// 有効なエントリ数
    count: usize,
}

impl RapidPressTracker {
    /// 検出閾値: この回数以上の IME キー押下で発動
    const THRESHOLD: usize = 3;
    /// 検出ウィンドウ（ミリ秒）
    const WINDOW_MS: u64 = 1000;

    const fn new() -> Self {
        Self {
            buf: [0; Self::THRESHOLD],
            cursor: 0,
            count: 0,
        }
    }

    /// タイムスタンプを記録し、連打が検出されたら `true` を返す。
    fn push(&mut self, now_ms: u64) -> bool {
        self.buf[self.cursor] = now_ms;
        self.cursor = (self.cursor + 1) % Self::THRESHOLD;
        if self.count < Self::THRESHOLD {
            self.count += 1;
        }

        if self.count < Self::THRESHOLD {
            return false;
        }

        // 全エントリが WINDOW_MS 以内に収まっているか
        let oldest = *self.buf.iter().min().unwrap_or(&0);
        now_ms.saturating_sub(oldest) < Self::WINDOW_MS
    }

    /// バッファをクリアする（発動後のリセット用）
    fn clear(&mut self) {
        self.buf = [0; Self::THRESHOLD];
        self.cursor = 0;
        self.count = 0;
    }
}

/// フックコールバック — unsafe は `APP.get_mut()` のみ。
///
/// # Safety
/// `APP.get_mut()` はシングルスレッド保証下でのみ安全。
/// フックコールバックはメインスレッドで呼ばれるため OK。
unsafe fn on_key_event_callback(event: RawKeyEvent) -> CallbackResult {
    let Some(app) = APP.get_mut() else {
        return CallbackResult::PassThrough;
    };
    on_key_event_impl(app, event)
}

/// フックコールバックの本体。
///
/// Engine.on_input() で consume/passthrough を判断し即座に返す（1-5ms）。
/// Effect の実行（SendInput, IME 操作等）はキューに入れ、メッセージループに委譲する。
/// これにより OS の 300ms タイムアウトでフックが解除されることを根本的に防ぐ。
fn on_key_event_impl(app: &mut Runtime, event: RawKeyEvent) -> CallbackResult {
    // Enrich IME relevance with sync key info from config
    let mut event = event;
    app.enrich_ime_relevance(&mut event);

    // ── Shadow IME toggle: フックコールバックで即座に PRECOND_IME_ON を更新 ──
    // IME トグルキーのキーダウン時に shadow 値を反映する。
    // Observer のポーリングで実際の OS 状態に収束するが、
    // ポーリング間隔中は shadow 値で Engine が正しく動作する。
    if matches!(event.event_type, awase::types::KeyEventType::KeyDown) {
        if let Some(action) = event.ime_relevance.shadow_action.or(event.ime_relevance.sync_direction) {
            let current = awase_windows::PRECOND_IME_ON.load(Ordering::Acquire);
            let new_val = match action {
                awase::types::ShadowImeAction::Toggle => !current,
                awase::types::ShadowImeAction::TurnOn => true,
                awase::types::ShadowImeAction::TurnOff => false,
            };
            if new_val != current {
                awase_windows::PRECOND_IME_ON.store(new_val, Ordering::Release);
                log::debug!(
                    "Shadow IME toggle: {} → {} (vk=0x{:02X})",
                    if current { "ON" } else { "OFF" },
                    if new_val { "ON" } else { "OFF" },
                    event.vk_code.0,
                );
            }
        }
    }

    // ── パニックリセット: IME 関連キーの連打を検出 ──
    if event.ime_relevance.may_change_ime
        && matches!(event.event_type, awase::types::KeyEventType::KeyDown)
    {
        let now = hook::current_tick_ms();
        unsafe {
            if let Some(tracker) = RAPID_IME_TIMESTAMPS.get_mut() {
                if tracker.push(now) {
                    tracker.clear();
                    log::warn!("Rapid IME key press detected — requesting panic reset");
                    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
                    use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
                    let _ = PostMessageW(HWND::default(), WM_PANIC_RESET, WPARAM(0), LPARAM(0));
                }
            }
        }
    }

    let ctx = crate::runtime::build_input_context();

    // Engine の判断: consume/passthrough を決定（1-5ms、OS API 呼び出しなし）
    let decision = app.engine.on_input(event, &ctx);

    // consume/passthrough を即座に返し、Effects はキューに入れる（OS API 呼び出しなし）
    let hook_result = app.executor.execute_from_hook(decision, &event);

    // キューに Effects があれば、メッセージループに実行を委譲する
    if hook_result.has_pending {
        unsafe {
            use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
            use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
            let _ = PostMessageW(HWND::default(), WM_EXECUTE_EFFECTS, WPARAM(0), LPARAM(0));
        }
    }

    hook_result.callback
}

/// メッセージループ
#[allow(clippy::too_many_lines)] // message loop dispatch with many match arms
fn run_message_loop(taskbar_created_msg: u32) {
    let mut msg = MSG::default();

    loop {
        let ret = unsafe { GetMessageW(&raw mut msg, HWND::default(), 0, 0) };
        if ret.0 <= 0 {
            break; // WM_QUIT or エラー
        }

        match msg.message {
            WM_TIMER => unsafe {
                let Some(app) = APP.get_mut() else { continue };
                let logical_id = app.executor.platform.timer.resolve(msg.wParam.0);
                match logical_id {
                    Some(id) if id == TIMER_IME_POLL => {
                        app.refresh_ime_state_cache();
                    }
                    Some(id) if id == TIMER_HOOK_WATCHDOG => {
                        use std::sync::atomic::AtomicU64;
                        // Ping 方式: 合成キーイベントを送り、フックが受信するか確認する。
                        // Phase 1: 前回の ping 後にフックが応答したか確認
                        static PING_SENT_AT: AtomicU64 = AtomicU64::new(0);
                        let ping_sent = PING_SENT_AT.load(Ordering::Relaxed);
                        let last_activity = hook::last_hook_activity_ms();

                        if ping_sent > 0 && last_activity < ping_sent {
                            // ping を送ったのにフックが応答していない → フック消失
                            let stale_ms = hook::current_tick_ms() - last_activity;
                            log::error!(
                                "Hook watchdog: ping not received (last activity {stale_ms}ms ago) — reinstalling"
                            );
                            if hook::reinstall_hook() {
                                app.executor.platform.tray.show_balloon(
                                    "awase",
                                    "キーボードフックを自動復旧しました",
                                );
                            } else {
                                app.executor.platform.tray.show_balloon(
                                    "awase",
                                    "フック復旧に失敗しました。再起動してください。",
                                );
                            }
                        }

                        // Phase 2: 次回チェック用に ping を送信
                        PING_SENT_AT.store(hook::current_tick_ms(), Ordering::Relaxed);
                        hook::send_ping();
                    }
                    Some(id) if id == TIMER_FOCUS_DEBOUNCE => {
                        app.executor.platform.timer.kill(TIMER_FOCUS_DEBOUNCE);
                        app.refresh_ime_state_cache();
                    }
                    Some(timer_id) => {
                        // Engine タイマー (TIMER_PENDING, TIMER_SPECULATIVE)
                        log::debug!("WM_TIMER fired: logical_id={timer_id}");
                        let ctx = crate::runtime::build_input_context();
                        let decision = app.engine.on_timeout(timer_id, &ctx);
                        app.execute_decision(decision);
                    }
                    None => {
                        // 未知のタイマー → 無視
                    }
                }
            },
            WM_EXECUTE_EFFECTS => unsafe {
                if let Some(app) = APP.get_mut() {
                    app.executor.drain_deferred();
                }
            },
            WM_PANIC_RESET => unsafe {
                if let Some(app) = APP.get_mut() {
                    app.panic_reset();
                }
            },
            WM_IME_KEY_DETECTED => unsafe {
                if let Some(app) = APP.get_mut() {
                    app.process_deferred_keys();
                    let os_mods = observer::focus_observer::read_os_modifiers();
                    let decision = app
                        .engine
                        .on_command(awase::engine::EngineCommand::SyncModifiers(os_mods), &crate::runtime::build_input_context());
                    app.executor.execute_from_loop(decision);
                }
            },
            WM_POWERBROADCAST => unsafe {
                // スリープ復帰時に全状態をリフレッシュする。
                // PBT_APMRESUMEAUTOMATIC (0x12) / PBT_APMRESUMESUSPEND (0x07)
                let pbt = msg.wParam.0;
                if pbt == 0x12 || pbt == 0x07 {
                    log::info!("Power resume detected (PBT=0x{pbt:02X}), reinstalling hook and refreshing state");
                    // スリープ中にフックが解除されている可能性があるため即座に再インストール
                    hook::reinstall_hook();
                    if let Some(app) = APP.get_mut() {
                        app.invalidate_engine_context(ContextChange::InputLanguageChanged);
                        app.refresh_ime_state_cache();
                    }
                    FocusKind::Undetermined.store(&FOCUS_KIND);
                }
            },
            WM_WTSSESSION_CHANGE => unsafe {
                let session_event = msg.wParam.0 as u32;
                match session_event {
                    WTS_SESSION_LOCK => {
                        log::info!("Session locked, flushing engine state");
                        if let Some(app) = APP.get_mut() {
                            app.invalidate_engine_context(ContextChange::FocusChanged);
                        }
                    }
                    WTS_SESSION_UNLOCK => {
                        log::info!("Session unlocked, reinstalling hook and refreshing state");
                        // ロック中にフックが解除されている可能性があるため即座に再インストール
                        hook::reinstall_hook();
                        if let Some(app) = APP.get_mut() {
                            app.invalidate_engine_context(ContextChange::InputLanguageChanged);
                            app.refresh_ime_state_cache();
                        }
                        FocusKind::Undetermined.store(&FOCUS_KIND);
                    }
                    _ => {}
                }
            },
            WM_INPUTLANGCHANGE => unsafe {
                // 入力言語が変更された（Win+Space 等）→ 保留をフラッシュ + ガード ON
                // 言語切替直後は IME 状態が未反映の可能性があるため、
                // 後続キーをメッセージループに回して確実に更新後に処理する。
                log::info!("Input language changed, flushing pending state and enabling guard");
                if let Some(app) = APP.get_mut() {
                    app.invalidate_engine_context(ContextChange::InputLanguageChanged);
                    app.engine
                        .on_command(awase::engine::EngineCommand::SetGuard(true), &crate::runtime::build_input_context());
                    app.refresh_ime_state_cache();
                }
                // レイアウトが日本語でなくなった場合に警告
                check_keyboard_layout_on_change();
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
                // wParam: 下位 8 bit = FocusKind, 次の 8 bit = AppKind (0xFF = なし)
                let kind_u8 = msg.wParam.0 as u8;
                let app_kind_u8 = (msg.wParam.0 >> 8) as u8;
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
                    // AppKind を更新（UIA 結果が有効な場合のみ）
                    if app_kind_u8 != 0xFF {
                        let app_kind = awase::types::AppKind::from_u8(app_kind_u8);
                        app_kind.store(&awase_windows::APP_KIND);
                        log::debug!("UIA AppKind update: {app_kind:?}");
                    }

                    // FOCUS_KIND を更新（Undetermined の場合はスキップ）
                    if kind != FocusKind::Undetermined {
                        FocusKind::store(kind, &FOCUS_KIND);

                        // UIA 結果をキャッシュに反映
                        if let Some(app) = APP.get_mut() {
                            if let Some((pid, cls)) =
                                app.executor.platform.focus.last_focus_info.as_ref()
                            {
                                app.executor.platform.focus.cache.insert(
                                    *pid,
                                    cls.clone(),
                                    kind,
                                    DetectionSource::UiaAsync,
                                );
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
                    .map(|app| app.layouts.iter().map(|e| e.name.clone()).collect())
                    .unwrap_or_default();
                tray::handle_tray_message(
                    msg.hwnd,
                    msg.lParam,
                    &layout_names,
                    ELEVATED.load(Ordering::Relaxed),
                );
            },
            WM_RELOAD_CONFIG => {
                log::info!("Config reload requested via WM_RELOAD_CONFIG");
                reload_config();
            }
            WM_COMMAND => unsafe {
                if let Some(cmd) = tray::handle_tray_command(msg.wParam) {
                    if cmd == tray::cmd_settings() {
                        launch_settings();
                    } else if cmd == tray::cmd_restart_admin() {
                        tray::restart_as_admin();
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
            m if m == taskbar_created_msg && taskbar_created_msg != 0 => unsafe {
                log::info!("Explorer restarted, re-registering tray icon");
                if let Some(app) = APP.get_mut() {
                    app.executor.platform.tray.recreate();
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
    let Ok(exe) = std::env::current_exe() else {
        log::warn!("awase-settings not found");
        return;
    };
    let Some(dir) = exe.parent() else {
        log::warn!("awase-settings not found");
        return;
    };
    for name in &names {
        let path = dir.join(name);
        if path.exists() {
            let _ = std::process::Command::new(&path).spawn();
            return;
        }
    }
    log::warn!("awase-settings not found");
}

/// 設定ファイルを再読み込みし、エンジンのパラメータを更新する
///
/// Safety: `APP.get_mut()` はシングルスレッドからのみ呼び出すこと
fn reload_config() {
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

    // Safety: メインスレッドのメッセージループ上でのみ呼ばれる
    unsafe {
        if let Some(app) = APP.get_mut() {
            app.engine
                .on_command(awase::engine::EngineCommand::UpdateFsmParams {
                    threshold_ms: config.general.simultaneous_threshold_ms,
                    confirm_mode: config.general.confirm_mode,
                    speculative_delay_ms: config.general.speculative_delay_ms,
                }, &crate::runtime::build_input_context());
            app.executor
                .platform
                .output
                .set_mode(config.general.output_mode);
            log::info!(
                "Engine parameters updated: threshold={}ms, confirm_mode={:?}, speculative_delay={}ms, output_mode={:?}",
                config.general.simultaneous_threshold_ms,
                config.general.confirm_mode,
                config.general.speculative_delay_ms,
                config.general.output_mode,
            );
        }
    }

    // n-gram モデルの再読み込み
    let mut reload_diag = StartupDiagnostics::new();
    init_ngram_validated(&config, &mut reload_diag);
    reload_diag.report();

    // キーコンボの再読み込み（エンジン切替 + IME 制御）
    {
        let mut key_diag = StartupDiagnostics::new();
        let engine_on = parse_key_combos(&config.keys.engine_on, "Engine ON keys", &mut key_diag);
        let engine_off =
            parse_key_combos(&config.keys.engine_off, "Engine OFF keys", &mut key_diag);
        let ime_on = parse_key_combos(&config.keys.ime_on, "IME control ON keys", &mut key_diag);
        let ime_off = parse_key_combos(&config.keys.ime_off, "IME control OFF keys", &mut key_diag);
        let (toggle, on, off) = init_ime_sync_keys(&config.keys.ime_detect, &mut key_diag);
        unsafe {
            if let Some(app) = APP.get_mut() {
                // Update Runtime's sync keys for event enrichment
                app.sync_toggle_keys = toggle.clone();
                app.sync_on_keys = on.clone();
                app.sync_off_keys = off.clone();

                app.engine
                    .on_command(awase::engine::EngineCommand::ReloadKeys {
                        special: SpecialKeyCombos {
                            engine_on,
                            engine_off,
                            ime_on,
                            ime_off,
                        },
                        sync: ImeSyncKeys { toggle, on, off },
                    }, &crate::runtime::build_input_context());
            }
        }
        key_diag.report();
    }

    // フォーカスオーバーライド再読み込み + キャッシュクリア
    unsafe {
        if let Some(app) = APP.get_mut() {
            app.executor.platform.focus.overrides = config.focus_overrides;
            app.executor.platform.focus.cache = focus::cache::FocusCache::new();
        }
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
) -> Result<Vec<LayoutEntry>> {
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
                Ok(content) => match YabLayout::parse(&content, awase::scanmap::KeyboardModel::Jis)
                {
                    Ok(yab) => {
                        let yab = yab.resolve_kana();
                        log::info!("Discovered layout: {} ({})", yab.name, path.display());
                        layouts.push(LayoutEntry {
                            name: yab.name.clone(),
                            layout: yab,
                            left_thumb_vk,
                            right_thumb_vk,
                        });
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
    layouts.sort_by(|a, b| a.name.cmp(&b.name));

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
/// `SetWinEventHook` の RAII ガード。Drop 時に `UnhookWinEvent` を呼ぶ。
struct WinEventHookGuard(windows::Win32::UI::Accessibility::HWINEVENTHOOK);

impl Drop for WinEventHookGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::UI::Accessibility::UnhookWinEvent(self.0);
        }
        log::info!("Focus event hook uninstalled");
    }
}

fn install_focus_hook() -> Option<WinEventHookGuard> {
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
            None
        } else {
            log::info!("Focus event hook installed");
            Some(WinEventHookGuard(hook))
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

    // Observer: OS 観測 → FocusObservation
    let obs = observer::focus_observer::observe(
        hwnd,
        process_id,
        &class_name,
        &app.executor.platform.focus,
    );

    // Engine: 判断 → Decision
    let decision = app
        .engine
        .on_command(awase::engine::EngineCommand::FocusChanged(obs), &crate::runtime::build_input_context());

    // Runtime: 副作用実行
    app.execute_decision(decision);
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
