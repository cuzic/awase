//! 起動シーケンス（Bootstrap）
//!
//! `run()` から呼ばれる起動専用の初期化ヘルパー群。
//! `reload_config()` 等から再利用される共有ヘルパーは `app/mod.rs` に残す。

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use anyhow::{Context, Result};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS,
};

use awase::config::ValidatedConfig;
use awase::engine::{Engine, NicolaFsm};
use awase::engine::SpecialKeyCombos;
use awase::types::{RawKeyEvent, VkCode};
use awase::yab::YabLayout;
use awase_windows::vk::vk_name_to_code;

use awase_windows::executor;
use awase_windows::focus;
use awase_windows::hook;
use awase_windows::hook::CallbackResult;
use awase_windows::ime;
use awase_windows::output::Output;
use awase_windows::platform;
use awase_windows::runtime;
use awase_windows::tray;
use awase_windows::tray::SystemTray;
use awase_windows::{
    LayoutEntry, Runtime, APP, ELEVATED, TIMER_HOOK_WATCHDOG,
};

use awase_windows::{MAIN_THREAD_ID, TIMER_IME_REFRESH, TIMER_POWER_RESUME};

use super::{
    HotKeyGuard, HOTKEY_ID_FOCUS_OVERRIDE, HOTKEY_ID_TOGGLE, RapidPressTracker,
    RAPID_IME_TIMESTAMPS, StartupDiagnostics, WM_DUPLICATE_INSTANCE,
    find_config_path, init_ime_sync_keys, init_ngram_validated, load_config, parse_key_combos,
    resolve_relative, run_message_loop,
};

/// ログ初期化
///
/// `#![windows_subsystem = "windows"]` でコンソールがないため、
/// ログを初期化する。
///
/// `debug_console=false`（通常起動）: 実行ファイルと同じディレクトリの `awase.log` に出力。
/// `debug_console=true`（`--debug` フラグ）: 親プロセスのコンソール（WezTerm/PowerShell）に
/// stderr で出力する。ログレベルを debug に上げ、リアルタイムに観察できる。
pub(super) fn init_logging(debug_console: bool) {
    let log_path = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("awase.log")))
        .unwrap_or_else(|| PathBuf::from("awase.log"));

    if debug_console {
        // #![windows_subsystem = "windows"] だとコンソールウィンドウがないため、
        // 親プロセス（WezTerm / PowerShell 等）のコンソールにアタッチして stderr を有効にする。
        // SAFETY: AttachConsole is a standard Win32 API; ATTACH_PARENT_PROCESS is the documented sentinel value.
        unsafe {
            use windows::Win32::System::Console::AttachConsole;
            const ATTACH_PARENT_PROCESS: u32 = 0xFFFF_FFFF;
            let _ = AttachConsole(ATTACH_PARENT_PROCESS);
        }
        let mut builder = env_logger::Builder::from_env(
            env_logger::Env::default().default_filter_or("debug"),
        );
        builder.format_timestamp_millis();
        builder.target(env_logger::Target::Stderr);
        builder.init();
        log::info!("--debug: ログをコンソール(stderr)に出力, レベル=debug");
    } else {
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
        // ファイルが開けない場合は stderr フォールバック

        builder.init();
        log::info!(
            "Keyboard Layout Emulator starting... (log → {})",
            log_path.display()
        );
    }
}

/// 自動起動の設定を処理する
///
/// `auto_start` の値に応じて Task Scheduler への登録/解除を行う。
/// "ask" の場合はダイアログで確認し、結果を config.toml に保存する。
pub(super) fn handle_auto_start(config: &mut awase::config::AppConfig) {
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
                if let Ok(config_path) = find_config_path() {
                    if let Err(e) = config.save(&config_path) {
                        log::error!("Failed to save auto_start setting: {e}");
                    }
                }
            }
        }
        "enabled" | "disabled" => {}
        other => {
            log::warn!("Unknown auto_start value: {other}, ignoring");
        }
    }
}

/// 検証済み設定で配列の読み込みとエンジン初期化を行い、構成要素を返す
pub(super) fn init_engine_validated(
    config: &ValidatedConfig,
    diag: &mut StartupDiagnostics,
) -> Result<(
    NicolaFsm,
    Vec<LayoutEntry>,
    Vec<String>,
    String,
    VkCode,
    VkCode,
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
    let layouts = scan_layouts(&layouts_dir, diag)?;
    let layout_names: Vec<String> = layouts.iter().map(|e| e.name.clone()).collect();
    log::info!("Available layouts: {layout_names:?}");

    let (layout, initial_layout_name) = select_default_layout(&layouts, config);
    log::info!(
        "Layout loaded: {} normal keys, {} left thumb keys, {} right thumb keys",
        layout.normal.len(),
        layout.left_thumb.len(),
        layout.right_thumb.len()
    );

    let engine = NicolaFsm::new(
        layout,
        left_thumb_vk,
        right_thumb_vk,
        config.general.simultaneous_threshold_ms,
        config.general.confirm_mode,
        config.general.speculative_delay_ms,
    );

    Ok((engine, layouts, layout_names, initial_layout_name, left_thumb_vk, right_thumb_vk))
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

/// キーボードレイアウトが日本語(106/109)かどうかを検証し、警告を出す
pub(super) fn check_keyboard_layout(diag: &mut StartupDiagnostics) {
    let (is_japanese, lang_id) = ime::keyboard_layout_info();
    log::info!("Keyboard layout: LANGID=0x{lang_id:04X}, Japanese={is_japanese}",);
    if !is_japanese {
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

/// システムトレイアイコンを作成する
pub(super) fn init_tray(
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
pub(super) fn install_hooks_and_hotkeys_validated(
    config: &ValidatedConfig,
) -> Result<(hook::HookGuard, Option<HotKeyGuard>, Option<HotKeyGuard>)> {
    let callback = Box::new(|event: RawKeyEvent| -> CallbackResult {
        // SAFETY: on_key_event_callback accesses APP via SingleThreadCell; hook callbacks run on the main thread.
        unsafe { super::on_key_event_callback(event) }
    });
    let guard = hook::install_hook(callback).context("Failed to install keyboard hook")?;

    let toggle_guard = config
        .general
        .engine_toggle_hotkey
        .as_ref()
        .and_then(|hotkey_str| register_toggle_hotkey(hotkey_str).map_err(|e| log::warn!("{e}")).ok());
    let app_override_guard = register_app_override_hotkey().map_err(|e| log::warn!("{e}")).ok();
    Ok((guard, toggle_guard, app_override_guard))
}

/// トグルホットキーを登録する
fn register_toggle_hotkey(hotkey_str: &str) -> Result<HotKeyGuard> {
    let (modifiers, vk) = awase_windows::vk::parse_hotkey(hotkey_str)
        .context(format!("Invalid toggle hotkey format: {hotkey_str}"))?;
    // SAFETY: RegisterHotKey with None HWND registers on the calling thread's message queue; VK and modifiers are valid values.
    unsafe {
        RegisterHotKey(
            None,
            HOTKEY_ID_TOGGLE,
            HOT_KEY_MODIFIERS(modifiers),
            u32::from(vk.0),
        )
        .context(format!("Failed to register toggle hotkey: {hotkey_str}"))?;
    }
    log::info!("Toggle hotkey registered: {hotkey_str}");
    Ok(HotKeyGuard(HOTKEY_ID_TOGGLE))
}

/// 手動アプリオーバーライドホットキー (Ctrl+Shift+F11) を登録する
fn register_app_override_hotkey() -> Result<HotKeyGuard> {
    const MOD_CONTROL: u32 = 0x0002;
    const MOD_SHIFT: u32 = 0x0004;
    const VK_F11: u32 = 0x7A;
    // SAFETY: RegisterHotKey with None HWND registers on the calling thread's message queue; VK and modifiers are valid values.
    unsafe {
        RegisterHotKey(
            None,
            HOTKEY_ID_FOCUS_OVERRIDE,
            HOT_KEY_MODIFIERS(MOD_CONTROL | MOD_SHIFT),
            VK_F11,
        )
        .context("Failed to register focus override hotkey: Ctrl+Shift+F11")?;
    }
    log::info!("Focus override hotkey registered: Ctrl+Shift+F11");
    Ok(HotKeyGuard(HOTKEY_ID_FOCUS_OVERRIDE))
}

/// `WTSRegisterSessionNotification` の RAII ガード。Drop 時に解除する。
pub(super) struct WtsGuard(pub(super) HWND);

impl Drop for WtsGuard {
    fn drop(&mut self) {
        // SAFETY: self.0 is the HWND passed to WTSRegisterSessionNotification; still valid at drop time.
        unsafe {
            let _ = super::WTSUnRegisterSessionNotification(self.0);
        }
        log::info!("WTS session notification unregistered");
    }
}

/// セッション変更通知（画面ロック/アンロック）を登録する
pub(super) fn register_session_notification() -> Result<WtsGuard> {
    // SAFETY: APP is a SingleThreadCell; called from the main thread at startup before the message loop.
    let tray_hwnd = unsafe {
        APP.get_ref()
            .context("APP not initialized")?
            .executor
            .platform
            .tray
            .hwnd()
    };
    let ok = unsafe {
        super::WTSRegisterSessionNotification(tray_hwnd, super::NOTIFY_FOR_THIS_SESSION).as_bool()
    };
    anyhow::ensure!(ok, "WTSRegisterSessionNotification failed");
    log::info!("WTS session notification registered");
    Ok(WtsGuard(tray_hwnd))
}

/// APP グローバルの初期化（PlatformState を含む）
pub(super) fn initialize_app(
    engine: Engine,
    tray: SystemTray,
    config: &ValidatedConfig,
    layouts: Vec<LayoutEntry>,
    sync_toggle_keys: Vec<VkCode>,
    sync_on_keys: Vec<VkCode>,
    sync_off_keys: Vec<VkCode>,
    left_thumb_vk: VkCode,
    right_thumb_vk: VkCode,
) {
    let mut ps = awase_windows::PlatformState::new();
    ps.focus_debounce_ms = config.general.focus_debounce_ms;
    ps.ime_poll_interval_ms = config.general.ime_poll_interval_ms;
    hook::set_thumb_vk_codes(&mut ps.hook_config, left_thumb_vk, right_thumb_vk);

    // SAFETY: APP.set() and RAPID_IME_TIMESTAMPS.set() are called once on the main thread before
    // the message loop starts; SingleThreadCell guarantees exclusive access.
    unsafe {
        APP.set(Runtime {
            engine,
            executor: executor::DecisionExecutor::new(
                platform::WindowsPlatform {
                    output: Output::new(config.general.output_mode),
                    tray,
                    focus: runtime::AppKindClassifier::new(config.app_overrides.clone()),
                    timer: awase_windows::timer::Win32Timer::new(),
                },
                config.general.hook_mode,
            ),
            layouts,
            sync_toggle_keys,
            sync_on_keys,
            sync_off_keys,
            platform_state: ps,
        });
        RAPID_IME_TIMESTAMPS.set(RapidPressTracker::new());
    }
}

/// 起動時に IME 状態キャッシュを初期化する（Unknown → 実際の値）。
pub(super) fn initialize_ime_cache() {
    // SAFETY: APP is a SingleThreadCell; called from the main thread at startup before the message loop.
    unsafe {
        if let Some(app) = APP.get_mut() {
            app.refresh_ime_state_cache();
        }
    }
}

/// クリーンアップ処理（フック解除は HookGuard の Drop で行われる）
pub(super) fn cleanup() {
    // SAFETY: APP is a SingleThreadCell; cleanup() is called from the main thread after the message loop exits.
    unsafe {
        APP.clear();
    }
    log::info!("Exited cleanly.");
}

/// `WINEVENT_OUTOFCONTEXT` (0x0000) — コールバックをメッセージループで実行
const WINEVENT_OUTOFCONTEXT: u32 = 0x0000;

/// `EVENT_OBJECT_FOCUS` (0x8005) — フォーカス変更イベント
const EVENT_OBJECT_FOCUS: u32 = 0x8005;

/// `SetWinEventHook` の RAII ガード。Drop 時に `UnhookWinEvent` を呼ぶ。
pub(super) struct WinEventHookGuard(pub(super) windows::Win32::UI::Accessibility::HWINEVENTHOOK);

impl Drop for WinEventHookGuard {
    fn drop(&mut self) {
        // SAFETY: self.0 is a valid HWINEVENTHOOK handle obtained from SetWinEventHook; drop is called once.
        unsafe {
            let _ = windows::Win32::UI::Accessibility::UnhookWinEvent(self.0);
        }
        log::info!("Focus event hook uninstalled");
    }
}

/// フォーカス変更イベントフックを登録する
pub(super) fn install_focus_hook() -> Result<WinEventHookGuard> {
    use windows::Win32::UI::Accessibility::SetWinEventHook;
    // SAFETY: SetWinEventHook with WINEVENT_OUTOFCONTEXT and a valid callback function pointer; 0 thread/process IDs means all processes.
    let hook = unsafe {
        SetWinEventHook(
            EVENT_OBJECT_FOCUS,
            EVENT_OBJECT_FOCUS,
            None,
            Some(win_event_proc),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        )
    };
    anyhow::ensure!(!hook.is_invalid(), "Failed to install focus event hook");
    log::info!("Focus event hook installed");
    Ok(WinEventHookGuard(hook))
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

    if hwnd == HWND::default() {
        return;
    }

    let Some(app) = APP.get_mut() else {
        return;
    };

    app.platform_state.focus_transition_pending = true;

    let debounce_ms = u64::from(app.platform_state.focus_debounce_ms);
    app.schedule_ime_refresh(debounce_ms);
}

/// Ctrl+C ハンドラを登録（Win32 SetConsoleCtrlHandler）
pub(super) fn install_ctrl_handler() -> Result<()> {
    unsafe extern "system" fn handler(_ctrl_type: u32) -> windows::core::BOOL {
        use windows::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_QUIT};
        use awase_windows::{QUIT_REQUESTED, MAIN_THREAD_ID};
        QUIT_REQUESTED.store(true, Ordering::SeqCst);
        let tid = MAIN_THREAD_ID.load(Ordering::SeqCst);
        if tid != 0 {
            let _ = PostThreadMessageW(tid, WM_QUIT, WPARAM(0), LPARAM(0));
        }
        windows::core::BOOL(1)
    }

    // SAFETY: handler is a valid extern "system" fn pointer; SetConsoleCtrlHandler is safe to call from the main thread.
    unsafe {
        windows::Win32::System::Console::SetConsoleCtrlHandler(Some(handler), true)?;
    }
    Ok(())
}

/// layouts_dir 内の *.yab を全てスキャンして配列一覧を構築する
fn scan_layouts(
    layouts_dir: &Path,
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

    layouts.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(layouts)
}

/// アプリケーション全体の起動シーケンスを実行する。
///
/// `app::run()` から呼ばれる唯一のエントリポイント。
pub(super) fn run_all() -> anyhow::Result<()> {
    let debug_console = std::env::args().any(|a| a == "--debug");
    init_logging(debug_console);

    // 多重起動防止: Named Mutex で既存インスタンスをチェック
    // SAFETY: CreateMutexW, FindWindowW, PostMessageW, CloseHandle are standard Win32 calls.
    unsafe {
        use windows::core::{w, PCWSTR};
        use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
        use windows::Win32::System::Threading::CreateMutexW;
        use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW};

        let mutex = CreateMutexW(None, false, w!("Global\\awase_keyboard_emulator"));
        match mutex {
            Ok(handle) => {
                if GetLastError() == ERROR_ALREADY_EXISTS {
                    log::error!("Another instance of awase is already running. Exiting.");
                    let class_wide: Vec<u16> = tray::WINDOW_CLASS_NAME
                        .encode_utf16()
                        .chain(std::iter::once(0))
                        .collect();
                    if let Ok(existing) = FindWindowW(PCWSTR(class_wide.as_ptr()), PCWSTR::null()) {
                        if !existing.is_invalid() {
                            let _ = PostMessageW(
                                Some(existing),
                                WM_DUPLICATE_INSTANCE,
                                WPARAM(0),
                                LPARAM(0),
                            );
                        }
                    }
                    let _ = windows::Win32::Foundation::CloseHandle(handle);
                    std::process::exit(1);
                }
                let _ = handle;
            }
            Err(e) => {
                log::warn!("Failed to create instance mutex: {e}");
            }
        }
    }

    let mut diag = StartupDiagnostics::new();

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
    let (fsm, layouts, layout_names, initial_layout_name, left_thumb_vk, right_thumb_vk) =
        init_engine_validated(&config, &mut diag)?;
    let engine_on_keys = parse_key_combos(&config.keys.engine_on, "Engine ON keys", &mut diag);
    let engine_off_keys = parse_key_combos(&config.keys.engine_off, "Engine OFF keys", &mut diag);
    let ime_control_on_keys = parse_key_combos(&config.keys.ime_on, "IME control ON keys", &mut diag);
    let ime_control_off_keys = parse_key_combos(&config.keys.ime_off, "IME control OFF keys", &mut diag);
    let (ime_sync_toggle, ime_sync_on, ime_sync_off) = init_ime_sync_keys(&config.keys.ime_detect, &mut diag);
    check_keyboard_layout(&mut diag);
    let system_tray = init_tray(&layout_names, &initial_layout_name, elevated)?;

    let sync_toggle_keys = ime_sync_toggle.clone();
    let sync_on_keys = ime_sync_on.clone();
    let sync_off_keys = ime_sync_off.clone();

    let mut engine = Engine::new(
        fsm,
        SpecialKeyCombos {
            engine_on: engine_on_keys,
            engine_off: engine_off_keys,
            ime_on: ime_control_on_keys,
            ime_off: ime_control_off_keys,
        },
    );
    engine.set_thumb_vks(left_thumb_vk, right_thumb_vk);

    initialize_app(
        engine,
        system_tray,
        &config,
        layouts,
        sync_toggle_keys,
        sync_on_keys,
        sync_off_keys,
        left_thumb_vk,
        right_thumb_vk,
    );

    init_ngram_validated(&config, &mut diag);
    let (hook_guard, _toggle_hotkey_guard, _app_override_hotkey_guard) =
        install_hooks_and_hotkeys_validated(&config)?;
    diag.report();

    log::info!("Hook installed. Running message loop...");
    MAIN_THREAD_ID.store(
        // SAFETY: GetCurrentThreadId always succeeds and has no preconditions.
        unsafe { windows::Win32::System::Threading::GetCurrentThreadId() },
        Ordering::SeqCst,
    );
    if let Err(e) = install_ctrl_handler() {
        log::warn!("{e}");
    }
    let _focus_hook_guard = install_focus_hook().map_err(|e| log::warn!("{e}")).ok();
    let _obs_hook_guards = awase_windows::tsf::observer::install_observation_hooks();

    // 統合 IME リフレッシュタイマー + ウォッチドッグタイマー
    // SAFETY: APP is a SingleThreadCell; this runs on the main thread before the message loop.
    unsafe {
        if let Some(app) = APP.get_mut() {
            app.schedule_ime_refresh(u64::from(app.platform_state.ime_poll_interval_ms));
            app.executor
                .platform
                .timer
                .set(TIMER_HOOK_WATCHDOG, std::time::Duration::from_secs(10));
        }
    }

    let uia_tx = awase_windows::focus::uia::spawn_uia_worker();
    awase_windows::tsf::observer::start_monitor_thread();
    // SAFETY: APP is a SingleThreadCell; this runs on the main thread before the message loop.
    unsafe {
        if let Some(app) = APP.get_mut() {
            app.executor.platform.focus.set_uia_sender(uia_tx);
        }
    }

    let _wts_guard = register_session_notification().map_err(|e| log::warn!("{e}")).ok();
    initialize_ime_cache();

    // Explorer 再起動時にトレイアイコンを復元するため TaskbarCreated メッセージを登録
    // SAFETY: RegisterWindowMessageW with a valid wide string literal.
    let taskbar_created_msg = unsafe {
        windows::Win32::UI::WindowsAndMessaging::RegisterWindowMessageW(
            windows::core::w!("TaskbarCreated"),
        )
    };

    run_message_loop(taskbar_created_msg);
    cleanup();
    drop(hook_guard);

    Ok(())
}
