//! 起動シーケンス（Bootstrap）
//!
//! `run()` から呼ばれる起動専用の初期化ヘルパー群。
//! `reload_config()` 等から再利用される共有ヘルパーは `app/mod.rs` に残す。

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use anyhow::{Context, Result};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, HOT_KEY_MODIFIERS};

use crate::vk::VkCodeExt;
use crate::win32::HwndExt as _;
use awase::config::ValidatedConfig;
use awase::engine::SpecialKeyCombos;
use awase::engine::{Engine, NicolaFsm};
use awase::types::VkCode;
use awase::yab::YabLayout;

use crate::hook;
use crate::ime;
use crate::output::Output;
use crate::platform;
use crate::runtime::executor;
use crate::tray;
use crate::tray::SystemTray;
use crate::{with_app, with_app_ref, LayoutEntry, Runtime, ELEVATED, RUNTIME};

use crate::MAIN_THREAD_ID;

use super::{
    find_config_path, init_ime_sync_keys, init_ngram_validated, load_config, parse_key_combos,
    resolve_relative, run_message_loop, HotKeyGuard, RapidPressTracker, StartupDiagnostics,
    DUMP_TRIGGER, HOTKEY_ID_FOCUS_OVERRIDE, HOTKEY_ID_TOGGLE, RAPID_IME_TIMESTAMPS,
    WM_DUPLICATE_INSTANCE,
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
        let mut builder =
            env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("debug"));
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
    use crate::autostart;

    match config.general.auto_start.as_str() {
        "ask" => {
            if !autostart::is_registered() {
                if autostart::ask_user() {
                    let _ = autostart::register();
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
        "enabled" => {
            if !autostart::is_registered() {
                let _ = autostart::register();
            }
        }
        "disabled" => {}
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
    let left_thumb_vk = VkCode::from_name(&config.general.left_thumb_key).context(format!(
        "Unknown VK name: {}",
        config.general.left_thumb_key
    ))?;
    let right_thumb_vk = VkCode::from_name(&config.general.right_thumb_key).context(format!(
        "Unknown VK name: {}",
        config.general.right_thumb_key
    ))?;

    let layouts_dir = resolve_relative(&config.general.layouts_dir);
    let layouts = LayoutEntry::scan_all(&layouts_dir, diag)?;
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

    Ok((
        engine,
        layouts,
        layout_names,
        initial_layout_name,
        left_thumb_vk,
        right_thumb_vk,
    ))
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
    log::info!("Keyboard layout: LANGID=0x{lang_id:04X}, Japanese={is_japanese}");
    if !is_japanese {
        if lang_id == crate::vk::LANGID_ENGLISH_US {
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
    let guard = hook::install_hook().context("Failed to install keyboard hook")?;

    let toggle_guard = config
        .general
        .engine_toggle_hotkey
        .as_ref()
        .and_then(|hotkey_str| {
            register_toggle_hotkey(hotkey_str)
                .map_err(|e| log::warn!("{e}"))
                .ok()
        });
    let app_override_guard = register_app_override_hotkey()
        .map_err(|e| log::warn!("{e}"))
        .ok();
    Ok((guard, toggle_guard, app_override_guard))
}

/// トグルホットキーを登録する
fn register_toggle_hotkey(hotkey_str: &str) -> Result<HotKeyGuard> {
    let (modifiers, vk) = crate::vk::parse_hotkey(hotkey_str)
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
    use windows::Win32::UI::Input::KeyboardAndMouse::{MOD_CONTROL, MOD_SHIFT};
    // SAFETY: RegisterHotKey with None HWND registers on the calling thread's message queue; VK and modifiers are valid values.
    unsafe {
        RegisterHotKey(
            None,
            HOTKEY_ID_FOCUS_OVERRIDE,
            MOD_CONTROL | MOD_SHIFT,
            u32::from(crate::vk::VK_F11.0),
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
#[allow(clippy::redundant_closure_for_method_calls)]
pub(super) fn register_session_notification() -> Result<WtsGuard> {
    let tray_hwnd = with_app_ref(|app| app.tray_hwnd()).context("RUNTIME not initialized")?;
    let ok = unsafe {
        super::WTSRegisterSessionNotification(tray_hwnd, super::NOTIFY_FOR_THIS_SESSION).as_bool()
    };
    anyhow::ensure!(ok, "WTSRegisterSessionNotification failed");
    log::info!("WTS session notification registered");
    Ok(WtsGuard(tray_hwnd))
}

/// APP グローバルの初期化（PlatformState を含む）
#[allow(clippy::too_many_arguments)]
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
    all_keymaps: crate::keymap::KeymapTable,
) {
    let mut ps = crate::PlatformState::new();
    ps.focus_debounce_ms = config.general.focus_debounce_ms;
    ps.ime_poll_interval_ms = config.general.ime_poll_interval_ms;
    hook::set_thumb_vk_codes(left_thumb_vk, right_thumb_vk);

    let engine_on_ime_vk = config
        .keys
        .engine_on_ime_key
        .as_deref()
        .and_then(VkCode::from_name);
    let engine_off_ime_vk = config
        .keys
        .engine_off_ime_key
        .as_deref()
        .and_then(VkCode::from_name);

    let base_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."));

    // RUNTIME.set() / RAPID_IME_TIMESTAMPS.set() はメッセージループ開始前に一度だけ呼ばれる。
    // RefCell が排他借用中でないことは構造的に保証されている。
    RUNTIME.set(Runtime::new(
        engine,
        executor::DecisionExecutor::new(config.general.hook_mode),
        platform::WindowsPlatform {
            output: Output::new(config.general.output_mode),
            tray,
            timer: crate::timer::Win32Timer::new(),
            engine_on_ime_vk,
            engine_off_ime_vk,
            suppress_engine_state_key: false,
            focus: crate::focus::tracker::FocusTracker::new(
                crate::focus::cache::FocusCache::new(),
                crate::focus::classifier::ForceOverrides::new(config.app_overrides.clone()),
                crate::focus::classifier::ImmCapabilityStore::new(base_dir),
            ),
        },
        layouts,
        sync_toggle_keys,
        sync_on_keys,
        sync_off_keys,
        ps,
        all_keymaps,
    ));
    RAPID_IME_TIMESTAMPS.set(RapidPressTracker::new());
    DUMP_TRIGGER.set(crate::journal::DumpTriggerTracker::new());
}

/// 起動時に IME 状態キャッシュを初期化する（Unknown → 実際の値）。
pub(super) fn initialize_ime_cache() {
    let _ = with_app(Runtime::refresh_ime_state_cache);
}

/// クリーンアップ処理（フック解除は HookGuard の Drop で行われる）
pub(super) fn cleanup() {
    // cleanup() はメッセージループ終了後にメインスレッドから呼ばれる。
    RUNTIME.clear();
    log::info!("Exited cleanly.");
}

use windows::Win32::UI::WindowsAndMessaging::{EVENT_OBJECT_FOCUS, WINEVENT_OUTOFCONTEXT};

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
    use std::sync::atomic::{AtomicIsize, Ordering as AtomicOrdering};
    // 同一 HWND からの連続 EVENT_OBJECT_FOCUS は Chrome / UWP の子オブジェクト由来で
    // 数 ms 間隔で多発する。毎回 TsfGate を PendingWarmup に巻き戻すと、
    // ユーザーが押下した文字キーが held queue ごと破棄されて入力ロスする
    // （特に Chrome で文字入力不能になる症状）。
    // HWND が変わっていない場合は早期 return する。
    static LAST_FOCUS_HWND: AtomicIsize = AtomicIsize::new(0);

    if event != EVENT_OBJECT_FOCUS {
        return;
    }

    if hwnd.non_null().is_none() {
        return;
    }

    let hwnd_isize = hwnd.0 as isize;
    if LAST_FOCUS_HWND.swap(hwnd_isize, AtomicOrdering::Relaxed) == hwnd_isize {
        return;
    }

    let _ = with_app(|app| {
        // Step 5: focus_transition_pending: bool は InputBarrier::FocusTransition に置換。
        // 実際の barrier 設定は FocusChanged event 経由で行う (runtime/mod.rs)。
        // ここでは旧 pending=true 相当の動作を維持するため、すぐに FocusTransition を立てる。
        // (FocusChanged event の dispatch まで少しタイムラグがある場合に備えた safety net)
        let now = std::time::Instant::now();
        // HWND is a pointer value; cast to usize is valid
        #[allow(clippy::cast_sign_loss)]
        app.on_window_focus_event(crate::state::ime_event::HwndId(hwnd_isize as usize), now);
    });
}

/// Ctrl+C ハンドラを登録（Win32 SetConsoleCtrlHandler）
pub(super) fn install_ctrl_handler() -> Result<()> {
    unsafe extern "system" fn handler(_ctrl_type: u32) -> windows::core::BOOL {
        use crate::{MAIN_THREAD_ID, QUIT_REQUESTED};
        use windows::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_QUIT};
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

impl LayoutEntry {
    /// layouts_dir 内の *.yab を全てスキャンして配列一覧を構築する
    pub(super) fn scan_all(layouts_dir: &Path, diag: &mut StartupDiagnostics) -> Result<Vec<Self>> {
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
                    Ok(content) => {
                        match YabLayout::parse(&content, awase::scanmap::KeyboardModel::Jis) {
                            Ok(yab) => {
                                let yab = yab.resolve_kana();
                                log::info!("Discovered layout: {} ({})", yab.name, path.display());
                                layouts.push(Self {
                                    name: yab.name.clone(),
                                    layout: yab,
                                });
                            }
                            Err(e) => {
                                diag.warn(format!("レイアウト読込失敗: {}: {e}", path.display()));
                            }
                        }
                    }
                    Err(e) => {
                        diag.warn(format!("レイアウト読込失敗: {}: {e}", path.display()));
                    }
                }
            }
        }

        layouts.sort_by(|a, b| a.name.cmp(&b.name));

        Ok(layouts)
    }
}

/// アプリケーション全体の起動シーケンスを実行する。
///
/// `app::run()` から呼ばれる唯一のエントリポイント。
#[allow(clippy::too_many_lines)]
#[allow(clippy::items_after_statements)]
pub(super) fn run_all() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let debug_console = args.iter().any(|a| a == "--debug");
    init_logging(debug_console);

    // panic 発生時にファイル:行番号とメッセージをログに記録する。
    // デフォルトの panic handler は stderr に書くだけなので awase.log には残らない。
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info.location().map_or_else(
            || "unknown location".to_owned(),
            |l| format!("{}:{}:{}", l.file(), l.line(), l.column()),
        );
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| info.payload().downcast_ref::<String>().map(String::as_str))
            .unwrap_or("(non-string payload)");
        log::error!("[PANIC] {msg} @ {location}");
        prev_hook(info);
    }));

    // --exit-after <SECS>: デバッグ用タイムアウト自動終了
    let exit_after_secs: Option<u64> = args
        .windows(2)
        .find(|w| w[0] == "--exit-after")
        .and_then(|w| w[1].parse().ok());
    if let Some(secs) = exit_after_secs {
        log::info!("--exit-after {secs}s: {secs} 秒後に自動終了します");
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(secs));
            log::info!("--exit-after タイムアウト ({secs}s) → 終了");
            use crate::{MAIN_THREAD_ID, QUIT_REQUESTED};
            use std::sync::atomic::Ordering;
            use windows::Win32::Foundation::{LPARAM, WPARAM};
            use windows::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_QUIT};
            QUIT_REQUESTED.store(true, Ordering::SeqCst);
            let tid = MAIN_THREAD_ID.load(Ordering::SeqCst);
            if tid != 0 {
                // SAFETY: tid は起動時に格納した有効なスレッド ID。
                unsafe {
                    let _ = PostThreadMessageW(tid, WM_QUIT, WPARAM(0), LPARAM(0));
                }
            } else {
                std::process::exit(0);
            }
        });
    }

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
                    let class_wide = crate::win32::to_wide(tray::WINDOW_CLASS_NAME);
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
    let ime_control_on_keys =
        parse_key_combos(&config.keys.ime_on, "IME control ON keys", &mut diag);
    let ime_control_off_keys =
        parse_key_combos(&config.keys.ime_off, "IME control OFF keys", &mut diag);
    let (ime_sync_toggle, ime_sync_on, ime_sync_off) =
        init_ime_sync_keys(&config.keys.ime_detect, &mut diag);
    check_keyboard_layout(&mut diag);
    let system_tray = init_tray(&layout_names, &initial_layout_name, elevated)?;

    let sync_toggle_keys = ime_sync_toggle;
    let sync_on_keys = ime_sync_on;
    let sync_off_keys = ime_sync_off;

    let panic_trigger_combos: Vec<crate::panic_detect::PanicTriggerCombo> = ime_control_on_keys
        .iter()
        .map(|k| crate::panic_detect::PanicTriggerCombo {
            vk: k.vk,
            ctrl: k.ctrl,
            shift: k.shift,
            alt: k.alt,
            is_on: true,
        })
        .chain(
            ime_control_off_keys
                .iter()
                .map(|k| crate::panic_detect::PanicTriggerCombo {
                    vk: k.vk,
                    ctrl: k.ctrl,
                    shift: k.shift,
                    alt: k.alt,
                    is_on: false,
                }),
        )
        .collect();
    crate::panic_detect::set_panic_trigger_combos(panic_trigger_combos);

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

    if let Some(vk) = config
        .keys
        .engine_off_solo_triple
        .as_deref()
        .filter(|s| !s.is_empty())
        .and_then(|s: &str| {
            VkCode::from_name(s).or_else(|| {
                diag.warn(format!("Unknown key name for engine_off_solo_triple: {s}"));
                None
            })
        })
    {
        engine.set_engine_off_triple_vk(vk);
    }

    let compiled_keymaps = crate::keymap::KeymapTable::new(&config.keymaps);
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
        compiled_keymaps,
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
    let _obs_hook_guards = crate::tsf::observer::install_observation_hooks();

    // 統合 IME リフレッシュタイマー + ウォッチドッグタイマー
    let _ = with_app(|app| {
        app.reschedule_ime_refresh();
        app.start_hook_watchdog();
    });

    let (_uia_worker, uia_tx) = crate::focus::uia::spawn_uia_worker();
    let _gji_worker = crate::tsf::observer::start_monitor_thread();
    let _ = with_app(|app| app.set_uia_sender(uia_tx));

    let _wts_guard = register_session_notification()
        .map_err(|e| log::warn!("{e}"))
        .ok();
    initialize_ime_cache();

    // Explorer 再起動時にトレイアイコンを復元するため TaskbarCreated メッセージを登録
    // SAFETY: RegisterWindowMessageW with a valid wide string literal.
    let taskbar_created_msg = unsafe {
        windows::Win32::UI::WindowsAndMessaging::RegisterWindowMessageW(windows::core::w!(
            "TaskbarCreated"
        ))
    };

    run_message_loop(taskbar_created_msg);
    cleanup();
    drop(hook_guard);

    Ok(())
}
