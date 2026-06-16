mod bootstrap;

use std::path::{Path, PathBuf};

use anyhow::Result;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::UnregisterHotKey;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, MSG, WM_APP, WM_COMMAND, WM_HOTKEY, WM_INPUTLANGCHANGE,
    WM_POWERBROADCAST, WM_TIMER,
};

use awase::config::{AppConfig, ImeDetectConfig, ParsedKeyCombo, ValidatedConfig};
use awase::engine::SpecialKeyCombos;
use awase::ngram::NgramModel;
use awase::types::{RawKeyEvent, VkCode};

use crate::ime;
use crate::runtime::message_handlers;
use crate::vk::VkCodeExt;
use crate::{
    with_app, with_app_or_repost, with_app_or_repost_with, WM_DRAIN_OUTPUT_QUEUE, WM_DUMP_JOURNAL,
    WM_DUPLICATE_INSTANCE, WM_EXECUTE_EFFECTS, WM_FOCUS_KIND_UPDATE, WM_IME_KEY_DETECTED,
    WM_KEY_FROM_HOOK, WM_PANIC_RESET, WM_PROCESS_DEFERRED, WM_RELOAD_CONFIG,
};

// ── 定数 ──

/// 有効/無効切り替えホットキー ID
const HOTKEY_ID_TOGGLE: i32 = 1;

/// ジャーナルダンプトリガートラッカー（メインスレッド専用）
static DUMP_TRIGGER: crate::SingleThreadCell<crate::journal::DumpTriggerTracker> =
    crate::SingleThreadCell::new();

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
        let _ = with_app(|app| {
            app.show_tray_balloon(
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

use crate::panic_detect::{RapidPressTracker, RAPID_IME_TIMESTAMPS};

// ── エントリポイント ──

/// アプリケーションを起動する。
///
/// # Errors
/// 初期化に失敗した場合、またはメッセージループが正常に終了しなかった場合はエラーを返す。
pub fn run() -> Result<()> {
    bootstrap::run_all()
}

// ── 共有ヘルパー（bootstrap + reload_config から使用）──

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

/// 設定ファイルのパスを探索する
pub(crate) fn find_config_path() -> Result<PathBuf> {
    // `--flag` / `--flag value` 形式をスキップし、最初の非フラグ引数をパスとして扱う
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg.starts_with("--") {
            let _ = args.next(); // value をスキップ
            continue;
        }
        return Ok(PathBuf::from(arg));
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
    PathBuf::from(path)
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
            crate::vk::parse_key_combo(s).or_else(|| {
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
                VkCode::from_name(s).or_else(|| {
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

/// 検証済み設定で n-gram モデルのロード（オプション）
fn init_ngram_validated(config: &ValidatedConfig, diag: &mut StartupDiagnostics) {
    let Some(ref ngram_path) = config.general.ngram_file else {
        return;
    };
    let ngram_path = resolve_relative(ngram_path);
    let range_us = u64::from(config.general.ngram_adjustment_range_ms) * 1000;
    let min_us = u64::from(config.general.ngram_min_threshold_ms) * 1000;
    let max_us = u64::from(config.general.ngram_max_threshold_ms) * 1000;
    match NgramModel::from_file(&ngram_path, range_us, min_us, max_us) {
        Ok(model) => {
            log::info!("N-gram model loaded from {}", ngram_path.display());
            let _ = with_app(|app| app.set_ngram_model(model));
        }
        Err(e) => diag.warn(format!("n-gramモデル解析失敗: {e}")),
    }
}

/// `WM_INPUTLANGCHANGE` 時にキーボードレイアウトを検証する（message_handlers から呼ばれる）
pub(crate) fn check_keyboard_layout_on_change() {
    let (is_japanese, lang_id) = ime::keyboard_layout_info();
    if !is_japanese {
        if lang_id == crate::vk::LANGID_ENGLISH_US {
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
            app.show_tray_balloon(
                "awase",
                "日本語キーボードレイアウトが検出されません。親指シフトが正常に動作しない可能性があります。",
            );
        });
    }
}

// ── メッセージループ ──

#[allow(clippy::too_many_lines)]
fn run_message_loop(taskbar_created_msg: u32) {
    // フックスレッドへエンジンスレッド TID を公開（WM_KEY_FROM_HOOK の送信先）
    // SAFETY: GetCurrentThreadId は常に成功し副作用もない。
    crate::ENGINE_THREAD_ID.store(
        unsafe { GetCurrentThreadId() },
        std::sync::atomic::Ordering::Relaxed,
    );

    let mut msg = MSG::default();

    loop {
        // SAFETY: msg is a valid MSG on the stack; None HWND retrieves messages for the calling thread.
        let ret = unsafe { GetMessageW(&raw mut msg, None, 0, 0) };
        if ret.0 <= 0 {
            break;
        }

        match msg.message {
            WM_TIMER => {
                // SAFETY: WM_TIMER はメインスレッドのメッセージループからのみ呼ばれる。
                //         wParam のタイマー ID はアプリケーション定義の定数で一意。
                let _ = with_app(|app| unsafe {
                    message_handlers::handle_wm_timer(app, msg.wParam.0, &msg);
                });
            }
            WM_EXECUTE_EFFECTS => {
                // SAFETY: WM_EXECUTE_EFFECTS はメインスレッドのメッセージループからのみ配送される。
                let _ = with_app(|app| unsafe { message_handlers::handle_wm_execute_effects(app) });
            }
            WM_PANIC_RESET => {
                // 再入中に消えないよう repost する（blocking op 完了後に再実行）
                // SAFETY: WM_PANIC_RESET はメインスレッドのメッセージループからのみ配送される。
                with_app_or_repost(WM_PANIC_RESET, |app| unsafe {
                    message_handlers::handle_wm_panic_reset(app);
                });
            }
            WM_DUPLICATE_INSTANCE => {
                // SAFETY: WM_DUPLICATE_INSTANCE はメインスレッドのメッセージループからのみ配送される。
                let _ =
                    with_app(|app| unsafe { message_handlers::handle_wm_duplicate_instance(app) });
            }
            WM_IME_KEY_DETECTED => {
                // SAFETY: WM_IME_KEY_DETECTED はメインスレッドのメッセージループからのみ配送される。
                let _ =
                    with_app(|app| unsafe { message_handlers::handle_wm_ime_key_detected(app) });
            }
            WM_POWERBROADCAST => {
                let pbt = msg.wParam.0;
                // SAFETY: WM_POWERBROADCAST はメインスレッドのメッセージループからのみ配送される。
                //         pbt は OS が設定する PBT_* 定数で安全。
                let _ =
                    with_app(|app| unsafe { message_handlers::handle_wm_powerbroadcast(app, pbt) });
            }
            WM_WTSSESSION_CHANGE => {
                let session_event = msg.wParam.0 as u32;
                // SAFETY: WM_WTSSESSION_CHANGE はメインスレッドのメッセージループからのみ配送される。
                //         hwnd は WTSRegisterSessionNotification で登録した有効なウィンドウハンドル。
                let _ = with_app(|app| unsafe {
                    message_handlers::handle_wts_session_change(app, session_event);
                });
            }
            WM_INPUTLANGCHANGE => {
                // SAFETY: WM_INPUTLANGCHANGE はメインスレッドのメッセージループからのみ配送される。
                let _ = with_app(|app| unsafe { message_handlers::handle_wm_inputlangchange(app) });
            }
            WM_PROCESS_DEFERRED => {
                // SAFETY: WM_PROCESS_DEFERRED はメインスレッドのメッセージループからのみ配送される。
                let _ =
                    with_app(|app| unsafe { message_handlers::handle_wm_process_deferred(app) });
            }
            WM_FOCUS_KIND_UPDATE => {
                let (wparam, lparam) = (msg.wParam.0, msg.lParam.0);
                // SAFETY: WM_FOCUS_KIND_UPDATE はメインスレッドのメッセージループからのみ配送される。
                //         wparam/lparam の値はポスト元が正しく設定した FocusKind エンコード値。
                with_app_or_repost_with(WM_FOCUS_KIND_UPDATE, wparam, lparam, |app| unsafe {
                    message_handlers::handle_wm_focus_kind_update(app, wparam, lparam);
                });
            }
            WM_HOTKEY if msg.wParam.0 == HOTKEY_ID_TOGGLE as usize => {
                // SAFETY: WM_HOTKEY はメインスレッドのメッセージループからのみ配送される。
                //         wParam は RegisterHotKey で登録した HOTKEY_ID_TOGGLE と一致している。
                let _ = with_app(|app| unsafe { message_handlers::handle_wm_hotkey_toggle(app) });
            }
            WM_HOTKEY if msg.wParam.0 == HOTKEY_ID_FOCUS_OVERRIDE as usize => {
                // SAFETY: WM_HOTKEY はメインスレッドのメッセージループからのみ配送される。
                //         wParam は RegisterHotKey で登録した HOTKEY_ID_FOCUS_OVERRIDE と一致している。
                let _ = with_app(|app| unsafe {
                    message_handlers::handle_wm_hotkey_focus_override(app);
                });
            }
            WM_DUMP_JOURNAL => {
                let _ = with_app(|app| message_handlers::handle_wm_dump_journal(app));
            }
            WM_KEY_FROM_HOOK => {
                // フックスレッドから転送された物理キーイベント
                // SAFETY: lParam は Box::into_raw(Box::new(RawKeyEvent)) のポインタ。
                //         RawKeyEvent は Copy なので値をコピーして Box をドロップする。
                let event = unsafe { *Box::from_raw(msg.lParam.0 as *mut RawKeyEvent) };
                // パニックリセット検出: IME OFF→ON→OFF の交互シーケンスのみ発動
                if matches!(event.event_type, awase::types::KeyEventType::KeyDown) {
                    let mods = event.modifier_snapshot;
                    if let Some(is_on) = crate::panic_detect::get_panic_trigger_direction(
                        event.vk_code,
                        mods.ctrl,
                        mods.shift,
                        mods.alt,
                    ) {
                        crate::panic_detect::record_ime_keydown(
                            is_on,
                            crate::hook::current_tick_ms(),
                        );
                    }
                    // ジャーナルダンプトリガー: Alt+変換→Alt+無変換 を 2 回連続
                    let fired = DUMP_TRIGGER.try_with_mut(|t| t.push(event.vk_code.0, mods.alt));
                    if fired == Some(true) {
                        crate::win32::post_to_main_thread(WM_DUMP_JOURNAL);
                    }
                }
                if crate::OUTPUT_GATE.is_active() {
                    crate::INPUT_DEFER.defer_during_output(event);
                } else {
                    // 競合条件の修正: フックスレッドは OUTPUT_GATE active 中に WM_KEY_FROM_HOOK
                    // を POST する。メインスレッドが処理する頃には OUTPUT_GATE が false になっているが、
                    // WM_DRAIN_OUTPUT_QUEUE よりも WM_KEY_FROM_HOOK が先にキューに入っている場合、
                    // drain が Ctrl↑ を executor に追加する前に K↑/A↓ が直接 executor.queue に
                    // 入ってしまい、reinject 順序が Ctrl↑ < K↑/A↓ となって Ctrl+key 誤発火する。
                    // INPUT_DEFER に pending があれば drain と同じ順序で処理させるため defer する。
                    let has_pending_drain = crate::INPUT_DEFER
                        .pending_len_nonblocking()
                        .is_none_or(|n| n > 0);
                    if has_pending_drain {
                        crate::INPUT_DEFER.replay_later(std::iter::once(event));
                    } else {
                        let result =
                            with_app(|app| message_handlers::handle_wm_key_from_hook(app, event));
                        debug_assert!(result.is_some(), "with_app re-entry in WM_KEY_FROM_HOOK");
                    }
                }
            }
            WM_APP => {
                // SAFETY: WM_APP はシステムトレイ通知用に定義したメッセージ。
                //         msg.hwnd は有効なトレイ通知ウィンドウのハンドル。
                unsafe {
                    message_handlers::handle_wm_app_tray(msg.hwnd, msg.lParam);
                }
            }
            WM_RELOAD_CONFIG => {
                message_handlers::handle_wm_reload_config();
            }
            WM_COMMAND => {
                // SAFETY: WM_COMMAND はメインスレッドのメッセージループからのみ配送される。
                //         wParam はトレイメニューの定義済みコマンド ID。
                unsafe {
                    message_handlers::handle_wm_command(msg.wParam);
                }
            }
            WM_DRAIN_OUTPUT_QUEUE => {
                // SAFETY: WM_DRAIN_OUTPUT_QUEUE はメインスレッドのメッセージループからのみ配送される。
                //         OUTPUT_GATE が非アクティブになったタイミングで post される。
                unsafe {
                    message_handlers::handle_wm_drain_output_queue();
                }
            }
            m if m == taskbar_created_msg && taskbar_created_msg != 0 => {
                // SAFETY: TaskbarCreated はシェルが再起動した際にブロードキャストされる登録済みメッセージ。
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
pub(crate) fn launch_settings() {
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
pub(crate) fn reload_config() {
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

    let mut reload_diag = StartupDiagnostics::new();
    init_ngram_validated(&config, &mut reload_diag);
    reload_diag.report();

    let mut key_diag = StartupDiagnostics::new();
    let engine_on = parse_key_combos(&config.keys.engine_on, "Engine ON keys", &mut key_diag);
    let engine_off = parse_key_combos(&config.keys.engine_off, "Engine OFF keys", &mut key_diag);
    let ime_on = parse_key_combos(&config.keys.ime_on, "IME control ON keys", &mut key_diag);
    let ime_off = parse_key_combos(&config.keys.ime_off, "IME control OFF keys", &mut key_diag);
    let (toggle, on, off) = init_ime_sync_keys(&config.keys.ime_detect, &mut key_diag);
    let panic_trigger_combos: Vec<crate::panic_detect::PanicTriggerCombo> = ime_on
        .iter()
        .map(|k| crate::panic_detect::PanicTriggerCombo {
            vk: k.vk,
            ctrl: k.ctrl,
            shift: k.shift,
            alt: k.alt,
            is_on: true,
        })
        .chain(
            ime_off
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
    key_diag.report();

    let special_keys = SpecialKeyCombos {
        engine_on,
        engine_off,
        ime_on,
        ime_off,
    };
    let _ = with_app(|app| {
        app.apply_config_update(&config, special_keys, toggle, on, off);
    });
    log::info!("Config reloaded successfully");
}
