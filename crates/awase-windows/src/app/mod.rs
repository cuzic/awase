#![allow(unsafe_code)] // Win32 API 呼び出しに unsafe が必須(lib.rsのクレート全体allowから個別移管、Task #9)
mod bootstrap;
pub(crate) use bootstrap::log_path as bug_report_log_path;
pub(crate) use bootstrap::{detect_conflicting_software, thumb_shift_faces_enabled_for};

use std::path::PathBuf;

use anyhow::{Context, Result};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::UnregisterHotKey;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, TranslateMessage, MSG, WM_APP, WM_COMMAND, WM_HOTKEY,
    WM_INPUTLANGCHANGE, WM_POWERBROADCAST, WM_TIMER,
};

use awase::config::{AppConfig, ImeDetectConfig, ParsedKeyCombo, ValidatedConfig};
use awase::engine::SpecialKeyCombos;
use awase::ngram::NgramModel;
use awase::types::VkCode;

use crate::ime;
use crate::runtime::message_handlers;
use crate::vk::VkCodeExt;
use crate::{
    with_app, with_app_or_repost, with_app_or_repost_with, WM_ASYNC_IME_APPLY_COMPLETE,
    WM_DRAIN_OUTPUT_QUEUE, WM_DUMP_JOURNAL, WM_DUPLICATE_INSTANCE, WM_ENGINE_QUIT_REQUEST,
    WM_EXECUTE_EFFECTS, WM_FOCUS_KIND_UPDATE, WM_GJI_CHARSET_FN_KEY_ACTIVATED,
    WM_GJI_REINIT_RETRY_COMPLETE, WM_IME_KIND_CHANGED, WM_KANA_LOCK_WARNING_CHANGED,
    WM_KEY_FROM_HOOK, WM_PANIC_RESET, WM_RELOAD_CONFIG,
};

// ── 定数 ──

/// 有効/無効切り替えホットキー ID
const HOTKEY_ID_TOGGLE: i32 = 1;

/// ジャーナルダンプトリガートラッカー（メインスレッド専用）
static DUMP_TRIGGER: crate::SingleThreadCell<crate::journal::DumpTriggerTracker> =
    crate::SingleThreadCell::new();

/// `RegisterWindowMessageW(TaskbarCreated)` で得た動的メッセージ ID。
///
/// 起動時に一度だけ `set_taskbar_created_msg` で設定する。`dispatch_engine_message`
/// が唯一の消費者であり、`run_message_loop` からの通常呼び出しと `engine_wnd_proc`
/// 経由のネストしたモーダルポンプ呼び出しの両方から同じ判定を通す（ADR-105が
/// 保証する「ネストポンプ中も配送される」の恩恵を Explorer 再起動時のトレイアイコン
/// 復元にも及ぼすため。Opus敵対的レビュー指摘、2026-08-26。旧実装は
/// `run_message_loop` 本体だけの手書き特別扱いで、ネストしたモーダルポンプ中の
/// `TaskbarCreated` を取りこぼしていた）。
static TASKBAR_CREATED_MSG: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

pub(crate) fn set_taskbar_created_msg(msg: u32) {
    TASKBAR_CREATED_MSG.store(msg, std::sync::atomic::Ordering::Relaxed);
}

/// 手動フォーカスオーバーライドホットキー ID (Ctrl+Shift+F11)
const HOTKEY_ID_FOCUS_OVERRIDE: i32 = 2;

/// `WM_WTSSESSION_CHANGE` — セッションの状態変更通知メッセージ
const WM_WTSSESSION_CHANGE: u32 = 0x02B1;

/// 現在のセッションのみ通知を受け取る
const NOTIFY_FOR_THIS_SESSION: u32 = 0;

#[link(name = "wtsapi32")]
unsafe extern "system" {
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
        "Default layout: {}, Threshold: {}ms",
        config.general.default_layout,
        config.general.simultaneous_threshold_ms,
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
    awase::paths::resolve_relative_to_exe(path)
}

/// 不具合報告用に `config.toml` と現在有効な `.yab` の生テキストを読み込む。
///
/// 両方ともベストエフォート。読めない場合は journal dump と同様に warn ログへ
/// 留め、呼び出し元には非致命的な `None` として返す。
pub(crate) fn read_bug_report_attachments(
    active_layout_name: &str,
) -> (Option<String>, Option<String>) {
    let config_toml = match find_config_path().and_then(|path| {
        std::fs::read_to_string(&path).with_context(|| format!("{} read failed", path.display()))
    }) {
        Ok(text) => Some(text),
        Err(e) => {
            log::warn!("[bug-report] config.toml read failed: {e}");
            None
        }
    };

    let layout_yab = config_toml.as_deref().and_then(|toml_text| {
        let parsed: AppConfig = match toml::from_str(toml_text) {
            Ok(parsed) => parsed,
            Err(e) => {
                log::warn!("[bug-report] config.toml parse failed: {e}");
                return None;
            }
        };
        // 実行時と同じ経路（`reload_config`）で validate() を通す。生の
        // `layouts_dir` をそのまま使うと `..` を含む値の正規化（`validate_layouts`）
        // が反映されず、実際に読まれている .yab と異なる場所を見に行く。
        let (validated, warnings) = parsed.validate();
        for w in &warnings {
            log::warn!("[bug-report] config.toml validation warning: {w}");
        }
        let layouts_dir = resolve_relative(&validated.general.layouts_dir);
        let yab_path = layouts_dir.join(format!("{active_layout_name}.yab"));
        match std::fs::read_to_string(&yab_path) {
            Ok(text) => Some(text),
            Err(e) => {
                log::warn!("[bug-report] {} read failed: {e}", yab_path.display());
                None
            }
        }
    });

    (config_toml, layout_yab)
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

/// IME control ON/OFF キーから panic トリガー用の `PanicTriggerCombo` 一覧を構築する
fn build_panic_trigger_combos(
    ime_on: &[ParsedKeyCombo],
    ime_off: &[ParsedKeyCombo],
) -> Vec<crate::panic_detect::PanicTriggerCombo> {
    ime_on
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
        .collect()
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

#[expect(clippy::too_many_lines)]
pub(crate) fn dispatch_engine_message(
    hwnd: HWND,
    message: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> bool {
    match message {
        WM_TIMER => {
            let msg = MSG {
                hwnd,
                message,
                wParam: wparam,
                lParam: lparam,
                ..Default::default()
            };
            let _ = with_app(|app| unsafe {
                message_handlers::handle_wm_timer(app, wparam.0, &msg);
            });
        }
        WM_EXECUTE_EFFECTS => {
            let _ = with_app(|app| unsafe { message_handlers::handle_wm_execute_effects(app) });
        }
        WM_ASYNC_IME_APPLY_COMPLETE => {
            let (wparam, lparam) = (wparam.0, lparam.0);
            with_app_or_repost_with(WM_ASYNC_IME_APPLY_COMPLETE, wparam, lparam, |app| {
                message_handlers::handle_wm_async_ime_apply_complete(app, wparam, lparam);
            });
        }
        WM_GJI_CHARSET_FN_KEY_ACTIVATED => {
            let _ = with_app(|app| {
                message_handlers::handle_wm_gji_charset_fn_key_activated(app, wparam.0);
            });
        }
        WM_GJI_REINIT_RETRY_COMPLETE => {
            let (wparam, lparam) = (wparam.0, lparam.0);
            with_app_or_repost_with(WM_GJI_REINIT_RETRY_COMPLETE, wparam, lparam, |app| {
                message_handlers::handle_wm_gji_reinit_retry_complete(app, wparam, lparam);
            });
        }
        WM_KANA_LOCK_WARNING_CHANGED => {
            with_app_or_repost(WM_KANA_LOCK_WARNING_CHANGED, |app| {
                message_handlers::handle_wm_kana_lock_warning_changed(app);
            });
        }
        WM_PANIC_RESET => {
            with_app_or_repost(WM_PANIC_RESET, |app| unsafe {
                message_handlers::handle_wm_panic_reset(app);
            });
        }
        WM_DUPLICATE_INSTANCE => {
            let _ = with_app(|app| unsafe { message_handlers::handle_wm_duplicate_instance(app) });
        }
        WM_IME_KIND_CHANGED => {
            let _ = with_app(|app| unsafe { message_handlers::handle_wm_ime_kind_changed(app) });
        }
        WM_POWERBROADCAST => {
            let _ = with_app(|app| unsafe {
                message_handlers::handle_wm_powerbroadcast(app, wparam.0);
            });
        }
        WM_WTSSESSION_CHANGE => {
            let session_event = wparam.0 as u32;
            let _ = with_app(|app| unsafe {
                message_handlers::handle_wts_session_change(app, session_event);
            });
        }
        WM_INPUTLANGCHANGE => {
            let _ = with_app(|app| unsafe { message_handlers::handle_wm_inputlangchange(app) });
        }
        WM_FOCUS_KIND_UPDATE => {
            let (wparam, lparam) = (wparam.0, lparam.0);
            with_app_or_repost_with(WM_FOCUS_KIND_UPDATE, wparam, lparam, |app| unsafe {
                message_handlers::handle_wm_focus_kind_update(app, wparam, lparam);
            });
        }
        WM_HOTKEY if wparam.0 == HOTKEY_ID_TOGGLE as usize => {
            let _ = with_app(|app| unsafe { message_handlers::handle_wm_hotkey_toggle(app) });
        }
        WM_HOTKEY if wparam.0 == HOTKEY_ID_FOCUS_OVERRIDE as usize => {
            let _ = with_app(|app| unsafe {
                message_handlers::handle_wm_hotkey_focus_override(app);
            });
        }
        WM_DUMP_JOURNAL => {
            let _ = with_app(message_handlers::handle_wm_dump_journal);
        }
        WM_KEY_FROM_HOOK => {
            crate::hook_channel::WAKE_PENDING.store(false, std::sync::atomic::Ordering::Release);
            let mut events = Vec::new();
            crate::hook_channel::HOOK_KEYS.consume_all(&mut |event| events.push(event));
            // dropped の読み取りと overflow ラッチ（指摘2-3）の解除を単一の
            // アトミック操作で行う（コードレビュー指摘1）。ring を consume し
            // 終えた直後に呼ぶことで、以後のフックコールバックはこの WM 到達
            // まで OS へパススルー固定していた分の resync が保証済みになる。
            let dropped = crate::hook_channel::HOOK_KEYS.take_dropped_and_clear_latch();
            if dropped > 0 {
                crate::runtime::engine_window::mark_needs_engine_resync();
                log::warn!("[hook-ring] dropped {dropped} key event(s)");
            }
            for event in events {
                handle_hook_key_event(event);
            }
        }
        WM_APP => unsafe {
            message_handlers::handle_wm_app_tray(hwnd, lparam);
        },
        WM_RELOAD_CONFIG => {
            message_handlers::handle_wm_reload_config();
        }
        WM_COMMAND => unsafe {
            message_handlers::handle_wm_command(wparam);
        },
        WM_DRAIN_OUTPUT_QUEUE => unsafe {
            message_handlers::handle_wm_drain_output_queue();
        },
        WM_ENGINE_QUIT_REQUEST => {
            crate::request_quit();
            if !crate::runtime::engine_window::is_in_modal_pump() {
                unsafe {
                    windows::Win32::UI::WindowsAndMessaging::PostQuitMessage(0);
                }
            }
        }
        msg if msg != 0
            && msg == TASKBAR_CREATED_MSG.load(std::sync::atomic::Ordering::Relaxed) =>
        {
            let _ = with_app(|app| unsafe { message_handlers::handle_taskbar_created(app) });
        }
        _ => return false,
    }
    true
}

fn handle_hook_key_event(event: awase::types::RawKeyEvent) {
    if matches!(event.event_type, awase::types::KeyEventType::KeyDown) {
        let mods = event.modifier_snapshot;
        if let Some(is_on) = crate::panic_detect::get_panic_trigger_direction(
            event.vk_code,
            mods.ctrl,
            mods.shift,
            mods.alt,
        ) {
            crate::panic_detect::record_ime_keydown(is_on, crate::hook::current_tick_ms());
        }
        let fired = DUMP_TRIGGER.try_with_mut(|t| t.push(event.vk_code.0, mods.alt));
        if fired == Some(true) {
            crate::win32::post_to_main_thread(WM_DUMP_JOURNAL);
        }
    }
    let defer_for_resync =
        crate::focus_resync::FOCUS_RESYNC.is_armed() && event.starts_focus_resync();
    if crate::OUTPUT_GATE.is_active() || defer_for_resync {
        if defer_for_resync {
            let generation = crate::focus_resync::FOCUS_RESYNC.consume_and_close();
            let _ = with_app(|app| {
                if app.kp_trigger_focus_resync(&event, generation) {
                    app.schedule_focus_resync_deadline();
                }
            });
        }
        crate::INPUT_DEFER.defer_during_output(event);
        return;
    }

    let has_pending_drain = crate::INPUT_DEFER
        .pending_len_nonblocking()
        .is_none_or(|n| n > 0);
    if has_pending_drain {
        crate::INPUT_DEFER.replay_later(std::iter::once(event));
        return;
    }
    if with_app(|app| message_handlers::handle_wm_key_from_hook(app, event)).is_none() {
        crate::INPUT_DEFER.replay_later(std::iter::once(event));
    }
}

fn run_message_loop() {
    // gji-io-monitor が TID 設定前に発行した初回 WM_IME_KIND_CHANGED は届かない
    // 可能性があるため、ループ開始時点の検出済み IME 種別で一度 pull 同期する
    // （BUG-09 の保険）。未検出（起動直後）なら MicrosoftIme 安全デフォルトになり、
    // 後の CLSID 検出変化が WM_IME_KIND_CHANGED で上書きする。
    // 副作用（戦略切替 + MS-IME 割当てチェック）は通常経路と同じ合流点に集約する。
    let _ = with_app(|app| {
        message_handlers::sync_ime_kind_from_observation(app, "startup pull sync");
    });

    let mut msg = MSG::default();

    loop {
        // SAFETY: msg is a valid MSG on the stack; None HWND retrieves messages for the calling thread.
        let ret = unsafe { GetMessageW(&raw mut msg, None, 0, 0) };
        if ret.0 <= 0 {
            break;
        }

        // `TaskbarCreated` を含む全ての内部メッセージは `dispatch_engine_message`
        // （唯一の集約テーブル、`TASKBAR_CREATED_MSG` 経由）が処理する。ここで
        // 特別扱いしないことで、ネストしたモーダルポンプ経由の `engine_wnd_proc`
        // からも同じ判定を通す。
        if dispatch_engine_message(msg.hwnd, msg.message, msg.wParam, msg.lParam) {
            continue;
        }
        unsafe {
            let _ = TranslateMessage(&raw const msg);
            DispatchMessageW(&raw const msg);
        }
    }
}

// ── アプリケーション機能 ──

/// 設定画面 (awase-settings) を起動する
pub(crate) fn launch_settings() {
    launch_settings_with_args(Vec::<String>::new());
}

pub(crate) fn launch_bug_report(
    journal_path: &std::path::Path,
    ime_kind: crate::bug_report::BugReportImeKind,
    diagnostics_path: Option<&std::path::Path>,
    app_log_path: Option<&std::path::Path>,
) {
    let mut args = vec![
        "--bug-report".to_owned(),
        "--journal".to_owned(),
        journal_path.to_string_lossy().into_owned(),
        "--ime-kind".to_owned(),
        ime_kind.as_str().to_owned(),
    ];
    if let Some(path) = diagnostics_path {
        args.push("--diagnostics".to_owned());
        args.push(path.to_string_lossy().into_owned());
    }
    // BUG-34 横展開: journal（構造化イベント）とは別に、実際の log::warn!/info!/
    // debug! 出力（awase.log）の末尾も添付できるようにする。journal には無い
    // send_health/degrade 系の警告ログを拾うため。
    if let Some(path) = app_log_path {
        args.push("--applog".to_owned());
        args.push(path.to_string_lossy().into_owned());
    }
    launch_settings_with_args(args);
}

fn launch_settings_with_args(args: impl IntoIterator<Item = String>) {
    let args: Vec<String> = args.into_iter().collect();
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
            let _ = std::process::Command::new(&path).args(&args).spawn();
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

    // ADR-116 決定2: 以前はここで3つの独立した StartupDiagnostics
    // （ngram用・keys用・layout用）を作り、それぞれ report() していたため、
    // 設定リロード1回でトレイバルーンが最大3回出ていた。1つに統合し、
    // report() は関数末尾で1回だけ呼ぶ。あわせて config.validate() の
    // 警告がこれまで log::warn! だけでユーザーに一切届いていなかった
    // 非対称（起動時は diag.warn 経由でトレイバルーンに出る）も解消する。
    let mut diag = StartupDiagnostics::new();

    let (config, config_warnings) = raw_config.validate();
    for w in config_warnings {
        diag.warn(w);
    }

    init_ngram_validated(&config, &mut diag);

    let engine_on = parse_key_combos(&config.keys.engine_on, "Engine ON keys", &mut diag);
    let engine_off = parse_key_combos(&config.keys.engine_off, "Engine OFF keys", &mut diag);
    let ime_on = parse_key_combos(&config.keys.ime_on, "IME control ON keys", &mut diag);
    let ime_off = parse_key_combos(&config.keys.ime_off, "IME control OFF keys", &mut diag);
    let ime_toggle = parse_key_combos(
        &config.keys.ime_toggle,
        "IME control Toggle keys",
        &mut diag,
    );
    let (toggle, on, off) = init_ime_sync_keys(&config.keys.ime_detect, &mut diag);
    let panic_trigger_combos = build_panic_trigger_combos(&ime_on, &ime_off);
    crate::panic_detect::set_panic_trigger_combos(panic_trigger_combos);

    crate::keymap::warn_on_engine_hotkey_collision(
        &config.keymaps,
        &engine_on,
        &engine_off,
        &ime_on,
        &ime_off,
        &ime_toggle,
        config.general.engine_toggle_hotkey.as_deref(),
    );
    let special_keys = SpecialKeyCombos {
        engine_on,
        engine_off,
        ime_on,
        ime_off,
        ime_toggle,
    };
    let _ = with_app(|app| {
        app.apply_config_update(&config, special_keys, toggle, on, off);
        // ADR-092 決定D Step4b前提条件3: MS-IME レジストリの Ctrl+Space/
        // Shift+Space トグル割当てを設定リロードのたびに再読みする
        // （stale化対策、apply_config_update が space_is_thumb_key を
        // 更新した直後に呼ぶ必要がある）。MS-IME/GJI は排他（決定A-2/A-3）
        // のため、現在確定している IME 種別が MS-IME の場合のみ読み直す
        // （`sync_ime_kind_from_observation` と同じガード）。
        let obs = crate::tsf::observer::tsf_obs();
        if obs.ime_kind_detected()
            && matches!(
                obs.active_ime_kind(),
                crate::tsf::observer::ActiveImeKind::MicrosoftIme
            )
        {
            message_handlers::sync_ime_toggle_auto_detect(app);
        }
    });

    let layouts_dir = resolve_relative(&config.general.layouts_dir);
    match crate::LayoutEntry::scan_all(
        &layouts_dir,
        &mut diag,
        config.general.keyboard_model,
        &config.keystroke_macro,
        config.general.keystroke_sequence,
    ) {
        Ok(layouts) => {
            let _ = with_app(|app| app.reload_layouts(layouts, &config.general.default_layout));
        }
        Err(e) => log::warn!("Failed to rescan layouts on config reload: {e}"),
    }

    diag.report();
    log::info!("Config reloaded successfully");
}
