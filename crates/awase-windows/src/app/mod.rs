mod bootstrap;
mod key_pipeline;
mod message_handlers;

use std::path::{Path, PathBuf};

use anyhow::Result;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::UnregisterHotKey;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, MSG, WM_APP, WM_COMMAND, WM_HOTKEY,
    WM_INPUTLANGCHANGE, WM_POWERBROADCAST, WM_TIMER,
};

use awase::config::{AppConfig, ImeDetectConfig, ParsedKeyCombo, ValidatedConfig};
use awase::engine::SpecialKeyCombos;
use awase::ngram::NgramModel;
use awase::types::{RawKeyEvent, VkCode};

use awase_windows::hook::CallbackResult;
use awase_windows::ime;
use awase_windows::runtime;
use awase_windows::{
    Runtime, WM_DRAIN_OUTPUT_QUEUE,
    WM_EXECUTE_EFFECTS, WM_FOCUS_KIND_UPDATE,
    WM_DUPLICATE_INSTANCE, WM_IME_KEY_DETECTED, WM_PANIC_RESET, WM_PROCESS_DEFERRED,
    WM_RELOAD_CONFIG, with_app, with_app_or_repost, with_app_or_repost_with,
};

// ── 定数 ──

/// 有効/無効切り替えホットキー ID
const HOTKEY_ID_TOGGLE: i32 = 1;

/// 手動フォーカスオーバーライドホットキー ID (Ctrl+Shift+F11)
const HOTKEY_ID_FOCUS_OVERRIDE: i32 = 2;

/// `WM_WTSSESSION_CHANGE` — セッションの状態変更通知メッセージ
const WM_WTSSESSION_CHANGE: u32 = 0x02B1;

/// 現在のセッションのみ通知を受け取る
const NOTIFY_FOR_THIS_SESSION: u32 = 0;

#[link(name = "wtsapi32")]
extern "system" {
    fn WTSRegisterSessionNotification(hwnd: HWND, flags: u32) -> windows::core::BOOL;
    fn WTSUnRegisterSessionNotification(hwnd: HWND) -> windows::core::BOOL;
}

// ── 共有型 ──

/// 起動時の警告を集約して報告する診断コレクター
struct StartupDiagnostics {
    warnings: Vec<String>,
}

impl StartupDiagnostics {
    const fn new() -> Self {
        Self { warnings: Vec::new() }
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
        let _ = with_app(|app| {
            app.executor.platform.tray.show_balloon(
                "awase",
                &format!("{}件の警告があります", self.warnings.len()),
            );
        });
    }
}

/// `RegisterHotKey` の RAII ガード。Drop 時に `UnregisterHotKey` を呼ぶ。
struct HotKeyGuard(i32);

impl Drop for HotKeyGuard {
    fn drop(&mut self) {
        // SAFETY: self.0 is the hotkey ID registered with RegisterHotKey; None hwnd targets this thread.
        unsafe {
            let _ = UnregisterHotKey(None, self.0);
        }
        log::info!("Hotkey {} unregistered", self.0);
    }
}

use awase_windows::panic_detect::{RapidPressTracker, RAPID_IME_TIMESTAMPS};

// ── エントリポイント ──

/// アプリケーションを起動する。
pub fn run() -> Result<()> {
    bootstrap::run_all()
}

// ── 共有ヘルパー（bootstrap + reload_config から使用）──

/// 設定ファイルを読み込む
pub(self) fn load_config() -> Result<AppConfig> {
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

/// 設定ファイルのパスを探索する
pub(self) fn find_config_path() -> Result<PathBuf> {
    if let Some(path) = std::env::args().nth(1) {
        return Ok(PathBuf::from(path));
    }
    let resolved = resolve_relative("config.toml");
    if resolved.exists() {
        return Ok(resolved);
    }
    anyhow::bail!(
        "Config file not found. Place config.toml next to the executable, \
         or specify path as command line argument."
    )
}

/// 相対パスを実行ファイルのディレクトリ基準で解決する
pub(self) fn resolve_relative(path: &str) -> PathBuf {
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
    PathBuf::from(path)
}

/// キーコンボ文字列のリストをパースし、失敗時は診断に警告を出す
pub(self) fn parse_key_combos(
    keys: &[String],
    label: &str,
    diag: &mut StartupDiagnostics,
) -> Vec<ParsedKeyCombo> {
    let parsed: Vec<ParsedKeyCombo> = keys
        .iter()
        .filter_map(|s| {
            awase_windows::vk::parse_key_combo(s).or_else(|| {
                diag.warn(format!("{label} のパースに失敗しました: {s}"));
                None
            })
        })
        .collect();
    log::info!("{label}: {keys:?} ({} parsed)", parsed.len());
    parsed
}

/// IME sync キーの初期化（shadow IME 状態追跡用）
pub(self) fn init_ime_sync_keys(
    ime_detect: &ImeDetectConfig,
    diag: &mut StartupDiagnostics,
) -> (Vec<VkCode>, Vec<VkCode>, Vec<VkCode>) {
    let mut parse_vk_list = |keys: &[String], label: &str| -> Vec<VkCode> {
        keys.iter()
            .filter_map(|s| {
                awase_windows::vk::vk_name_to_code(s).or_else(|| {
                    diag.warn(format!("keys.ime_detect.{label} のパースに失敗しました: {s}"));
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
        ime_detect.toggle, ime_detect.on, ime_detect.off,
    );
    (toggle, on, off)
}

/// 検証済み設定で n-gram モデルのロード（オプション）
pub(self) fn init_ngram_validated(config: &ValidatedConfig, diag: &mut StartupDiagnostics) {
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
            let _ = with_app(|app| {
                let modifiers = unsafe { awase_windows::observer::focus_observer::read_os_modifiers() };
                app.engine.on_command(
                    awase::engine::EngineCommand::SetNgramModel(model),
                    &runtime::build_input_context(
                        app.platform_state.preconditions(),
                        &modifiers,
                    ),
                );
            });
        }
        Err(e) => diag.warn(format!("n-gramモデル解析失敗: {e}")),
    }
}

/// `WM_INPUTLANGCHANGE` 時にキーボードレイアウトを検証する（message_handlers から呼ばれる）
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
        let _ = with_app(|app| {
            app.executor.platform.tray.show_balloon(
                "awase",
                "日本語キーボードレイアウトが検出されません。親指シフトが正常に動作しない可能性があります。",
            );
        });
    }
}

// ── フックコールバック ──

/// フックコールバック。
///
/// # Safety
/// Win32 キーボードフックコールバックはメインスレッドで呼ばれる。
unsafe fn on_key_event_callback(event: RawKeyEvent) -> CallbackResult {
    if awase_windows::OUTPUT_GATE.is_active() {
        log::debug!("[output-active] queuing vk=0x{:02X} {:?}", event.vk_code.0, event.event_type);
        awase_windows::INPUT_DEFER.defer_during_output(event);
        return CallbackResult::Consumed;
    }
    // OUTPUT_GATE.active=false だがキューに未 drain エントリがある場合、
    // 対応 KeyDown が drain 待ちの間に KeyUp が届いた（drain race）。
    // drain で synthetic KeyUp が注入されるが、念のためログを残す。
    if matches!(event.event_type, awase::types::KeyEventType::KeyUp) {
        if let Some(len) = awase_windows::INPUT_DEFER.pending_len_nonblocking() {
            if len > 0 {
                log::debug!(
                    "[drain-race] vk=0x{:02X} KeyUp arrived while drain queue has {len} item(s) (OUTPUT_GATE.active=false)",
                    event.vk_code.0
                );
            }
        }
    }
    with_app(|app| on_key_event_impl(app, event)).unwrap_or_else(|| {
        // with_app 再入中（set_ime_open_cross_process の SendMessageTimeoutW が
        // メッセージポンプを起動し、その間にハードウェアキーが届いた場合）。
        // PassThrough にすると NICOLA 文字キー（F=け、U=ち など）が MS-IME に
        // ラテン文字として届き文字化けする。
        // pending キューに退避して WM_DRAIN_OUTPUT_QUEUE で NICOLA に再配送する。
        log::debug!(
            "[with-app-reentry] queuing vk=0x{:02X} {:?} (IN_WITH_APP, defer to drain)",
            event.vk_code.0, event.event_type,
        );
        awase_windows::INPUT_DEFER.defer_during_with_app(event);
        CallbackResult::Consumed
    })
}

/// フックコールバックの本体。`KeyEventPipeline` に処理を委譲する。
fn on_key_event_impl(app: &mut Runtime, event: RawKeyEvent) -> CallbackResult {
    key_pipeline::KeyEventPipeline { app }.run(event)
}

// ── メッセージループ ──

pub(self) fn run_message_loop(taskbar_created_msg: u32) {
    let mut msg = MSG::default();

    loop {
        // SAFETY: msg is a valid MSG on the stack; None HWND retrieves messages for the calling thread.
        let ret = unsafe { GetMessageW(&raw mut msg, None, 0, 0) };
        if ret.0 <= 0 {
            break;
        }

        match msg.message {
            WM_TIMER => {
                let _ = with_app(|app| unsafe {
                    let logical_id = app.executor.platform.timer.resolve(msg.wParam.0);
                    message_handlers::handle_wm_timer(app, logical_id, msg.wParam.0, &msg);
                });
            }
            WM_EXECUTE_EFFECTS => {
                let _ = with_app(|app| unsafe { message_handlers::handle_wm_execute_effects(app) });
            }
            WM_PANIC_RESET => {
                // 再入中に消えないよう repost する（blocking op 完了後に再実行）
                with_app_or_repost(WM_PANIC_RESET, |app| unsafe { message_handlers::handle_wm_panic_reset(app) });
            }
            WM_DUPLICATE_INSTANCE => {
                let _ = with_app(|app| unsafe { message_handlers::handle_wm_duplicate_instance(app) });
            }
            WM_IME_KEY_DETECTED => {
                let _ = with_app(|app| unsafe { message_handlers::handle_wm_ime_key_detected(app) });
            }
            WM_POWERBROADCAST => {
                let pbt = msg.wParam.0;
                let _ = with_app(|app| unsafe { message_handlers::handle_wm_powerbroadcast(app, pbt) });
            }
            WM_WTSSESSION_CHANGE => {
                let session_event = msg.wParam.0 as u32;
                let _ = with_app(|app| unsafe { message_handlers::handle_wts_session_change(app, session_event) });
            }
            WM_INPUTLANGCHANGE => {
                let _ = with_app(|app| unsafe { message_handlers::handle_wm_inputlangchange(app) });
            }
            WM_PROCESS_DEFERRED => {
                let _ = with_app(|app| unsafe { message_handlers::handle_wm_process_deferred(app) });
            }
            WM_FOCUS_KIND_UPDATE => {
                let (wparam, lparam) = (msg.wParam.0, msg.lParam.0);
                with_app_or_repost_with(WM_FOCUS_KIND_UPDATE, wparam, lparam, |app| unsafe {
                    message_handlers::handle_wm_focus_kind_update(app, wparam, lparam);
                });
            }
            WM_HOTKEY if msg.wParam.0 == HOTKEY_ID_TOGGLE as usize => {
                let _ = with_app(|app| unsafe { message_handlers::handle_wm_hotkey_toggle(app) });
            }
            WM_HOTKEY if msg.wParam.0 == HOTKEY_ID_FOCUS_OVERRIDE as usize => {
                let _ = with_app(|app| unsafe { message_handlers::handle_wm_hotkey_focus_override(app) });
            }
            WM_APP => unsafe {
                message_handlers::handle_wm_app_tray(msg.hwnd, msg.lParam);
            },
            WM_RELOAD_CONFIG => {
                message_handlers::handle_wm_reload_config();
            }
            WM_COMMAND => unsafe {
                message_handlers::handle_wm_command(msg.wParam);
            },
            WM_DRAIN_OUTPUT_QUEUE => unsafe {
                message_handlers::handle_wm_drain_output_queue();
            },
            m if m == taskbar_created_msg && taskbar_created_msg != 0 => {
                let _ = with_app(|app| unsafe { message_handlers::handle_taskbar_created(app) });
            }
            _ => unsafe {
                // SAFETY: msg was filled by GetMessageW and is valid for the calling thread.
                DispatchMessageW(&raw const msg);
            },
        }
    }
}

// ── アプリケーション機能 ──

/// 設定画面 (awase-settings) を起動する
pub fn launch_settings() {
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

    let _ = with_app(|app| {
        let modifiers = unsafe { awase_windows::observer::focus_observer::read_os_modifiers() };
        app.engine.on_command(
            awase::engine::EngineCommand::UpdateFsmParams {
                threshold_ms: config.general.simultaneous_threshold_ms,
                confirm_mode: config.general.confirm_mode,
                speculative_delay_ms: config.general.speculative_delay_ms,
            },
            &runtime::build_input_context(
                app.platform_state.preconditions(),
                &modifiers,
            ),
        );
        app.executor.platform.output.set_mode(config.general.output_mode);
        log::info!(
            "Engine parameters updated: threshold={}ms, confirm_mode={:?}, speculative_delay={}ms, output_mode={:?}",
            config.general.simultaneous_threshold_ms,
            config.general.confirm_mode,
            config.general.speculative_delay_ms,
            config.general.output_mode,
        );
    });

    let mut reload_diag = StartupDiagnostics::new();
    init_ngram_validated(&config, &mut reload_diag);
    reload_diag.report();

    {
        let mut key_diag = StartupDiagnostics::new();
        let engine_on = parse_key_combos(&config.keys.engine_on, "Engine ON keys", &mut key_diag);
        let engine_off = parse_key_combos(&config.keys.engine_off, "Engine OFF keys", &mut key_diag);
        let ime_on = parse_key_combos(&config.keys.ime_on, "IME control ON keys", &mut key_diag);
        let ime_off = parse_key_combos(&config.keys.ime_off, "IME control OFF keys", &mut key_diag);
        let (toggle, on, off) = init_ime_sync_keys(&config.keys.ime_detect, &mut key_diag);
        let _ = with_app(|app| {
            app.sync_toggle_keys = toggle.clone();
            app.sync_on_keys = on.clone();
            app.sync_off_keys = off.clone();
            let modifiers = unsafe { awase_windows::observer::focus_observer::read_os_modifiers() };
            app.engine.on_command(
                awase::engine::EngineCommand::ReloadKeys {
                    special: SpecialKeyCombos {
                        engine_on,
                        engine_off,
                        ime_on,
                        ime_off,
                    },
                },
                &runtime::build_input_context(
                    app.platform_state.preconditions(),
                    &modifiers,
                ),
            );
        });
        key_diag.report();
    }

    let _ = with_app(|app| {
        app.executor.platform.focus.overrides = awase_windows::focus::classifier::ForceOverrides::new(config.app_overrides);
        app.executor.platform.focus.cache = awase_windows::focus::cache::FocusCache::new();
    });
    log::info!("App overrides reloaded");
    log::info!("Config reloaded successfully");
}
