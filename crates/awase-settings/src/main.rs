#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::path::{Path, PathBuf};

use eframe::egui;

use awase::kana_table::KanaTable;
use awase::scanmap::PhysicalPos;
use awase::types::{SpecialKey, VkCode};
use awase::yab::{FullwidthStrExt as _, YabFace, YabLayout, YabValue};
use awase_windows::vk::VkCodeExt as _;

mod bug_report;
mod scancode_map_admin;
mod startup_failure;
mod update_check;

/// 設定リロード用カスタムメッセージ ID（awase 本体側の `WM_APP + 10` と一致させる）
#[cfg(target_os = "windows")]
const WM_RELOAD_CONFIG: u32 = 0x8000 + 10; // WM_APP = 0x8000

/// awase のホームページ URL（`crates/awase-windows/src/tray.rs` の
/// `HOMEPAGE_URL` と同じ値。crate を跨ぐため定数を共有できず文字列直書き）。
const HOMEPAGE_URL: &str = "https://awase.cc";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Basic,
    Keys,
    Keymap,
    DisableApps,
    // サイドパネルから外しているため未構築（今後の課題として実装は保持）。
    // disable_apps 部分のみ `DisableApps` タブへ切り出し済み（2026-08-26、
    // BUG-90）。残る force_text/force_bypass/force_vk/force_tsf は
    // プロセス名+クラス名の両方が必要な、より高度な上書き設定のため
    // 引き続き非表示（config.toml の直接編集に委ねる）。
    #[allow(dead_code)]
    AppRules,
    Layout,
    Advanced,
}

/// 配列編集タブの6面。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Face {
    Normal,
    LeftThumb,
    RightThumb,
    Shift,
    LeftThumbShift,
    RightThumbShift,
}

const FACES: [(Face, &str); 6] = [
    (Face::Normal, "通常面"),
    (Face::LeftThumb, "左親指シフト"),
    (Face::RightThumb, "右親指シフト"),
    (Face::Shift, "小指シフト"),
    (Face::LeftThumbShift, "小指左親指シフト"),
    (Face::RightThumbShift, "小指右親指シフト"),
];

/// 配列編集タブのセル編集時の種別。
///
/// かつて awase-yab-editor という独立バイナリだったものを awase-settings に
/// 統合した（コードの再利用に価値はあるが、別バイナリに分ける価値は無いという
/// 判断。CI/配布物/インストーラで2バイナリを同期し続けるコストの方が大きかった）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ValueKind {
    /// ローマ字（複数文字、かな変換される）または記号・数字の打鍵（単発、
    /// IME がキーストロークとして処理する）。入力に応じて `apply_layout_edit` が
    /// `YabValue::Romaji` / `YabValue::KeySequence` のどちらを作るか自動判定する。
    /// JIS キーボード上に存在する文字（`char::is_ascii_graphic`）のみ許可する。
    Keystroke,
    Literal,
    Special,
    /// 仮想キーコード直接指定（やまぶきR互換の `V`+16進数）。
    /// `layout_edit_value` に16進文字列（`V`無し、例 `"1D"`）を保持する。
    Vk,
    None,
}

/// 打鍵欄の入力を検証する。JIS キーボード上のキーとして表現できない文字が
/// あれば、その文字を返す。
fn find_invalid_keystroke_char(input: &str) -> Option<char> {
    input.chars().find(|c| !c.is_ascii_graphic())
}

/// 打鍵欄の入力を正規化する。前後の空白を取り除き、全角英数記号
/// （IME 入力の癖で全角のまま打たれがち）を半角へ自動変換してから
/// 小文字化する。入力側が半角/全角を意識しなくて済むようにするため。
fn normalize_keystroke_input(input: &str) -> String {
    input.trim().to_halfwidth_str().to_lowercase()
}

const SPECIAL_KEYS: [(SpecialKey, &str); 14] = [
    (SpecialKey::Backspace, "Backspace"),
    (SpecialKey::Escape, "Escape"),
    (SpecialKey::Enter, "Enter"),
    (SpecialKey::Space, "Space"),
    (SpecialKey::Delete, "Delete"),
    (SpecialKey::Insert, "Insert"),
    (SpecialKey::Up, "Up"),
    (SpecialKey::Down, "Down"),
    (SpecialKey::Left, "Left"),
    (SpecialKey::Right, "Right"),
    (SpecialKey::Home, "Home"),
    (SpecialKey::End, "End"),
    (SpecialKey::PageUp, "PageUp"),
    (SpecialKey::PageDown, "PageDown"),
];

/// 配列編集タブのコピー履歴に保持する最大件数。
const CLIPBOARD_HISTORY_LEN: usize = 4;

/// キー入力キャプチャの対象。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaptureTarget {
    /// 既存ルールの from 全体（修飾+主キー）
    ExistingFrom(usize),
    /// 既存ルールの to 主キー
    ExistingTo(usize),
    /// 新規ルールの from 全体
    NewFrom,
    /// 新規ルールの to 主キー
    NewTo,
}

/// ログ初期化。
///
/// `#![windows_subsystem = "windows"]` によりコンソールが無いため、awase.exe
/// （`crates/awase-windows/src/app/bootstrap.rs::init_logging`）と同じ方式で
/// ログを初期化する: 通常起動は実行ファイル隣の `awase-settings.log` に出力し、
/// `--debug` フラグ指定時のみ親プロセスのコンソールへ stderr 出力する。
///
/// これが無いと GUI サブシステムでは panic してもコンソールに何も残らず
/// 「無言のまま強制終了」になる（2026-07-11 プレビュータブ egui::Grid panic の
/// 調査で発覚。当時 env_logger 自体が初期化されておらず log::warn! も no-op
/// だった）。
fn init_logging(debug_console: bool) {
    let log_path = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("awase-settings.log")))
        .unwrap_or_else(|| std::path::PathBuf::from("awase-settings.log"));

    if debug_console {
        attach_parent_console();
        let mut builder =
            env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("debug"));
        builder.format_timestamp_millis();
        builder.target(env_logger::Target::Stderr);
        builder.init();
        log::info!("--debug: ログをコンソール(stderr)に出力, レベル=debug");
        return;
    }

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
    log::info!("awase-settings starting... (log → {})", log_path.display());
}

/// ログを記録した直後に即 `flush` するチェックポイント。
///
/// 通常の `log::info!` はバッファされる場合があり、直後にハング/クラッシュ
/// すると出力が失われることがある（実機で「配列編集タブ関連のログが一切
/// 出ない」と報告された際、原因切り分けのために導入）。
fn log_checkpoint(msg: &str) {
    log::info!("[layout-tab] checkpoint: {msg}");
    log::logger().flush();
}

#[cfg(target_os = "windows")]
fn attach_parent_console() {
    use windows::Win32::System::Console::AttachConsole;
    const ATTACH_PARENT_PROCESS: u32 = 0xFFFF_FFFF;
    // SAFETY: AttachConsole is a standard Win32 API; ATTACH_PARENT_PROCESS is the documented sentinel value.
    unsafe {
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

#[cfg(not(target_os = "windows"))]
fn attach_parent_console() {}

/// panic 時にファイル:行番号とメッセージをログに記録する。
///
/// デフォルトの panic handler は stderr に書くだけなので、コンソールが無い
/// GUI サブシステムでは `awase-settings.log` に残らない。
fn install_panic_logging_hook() {
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
}

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let debug_console = args.iter().any(|a| a == "--debug");
    init_logging(debug_console);
    install_panic_logging_hook();

    if args.iter().any(|a| a == "--bug-report") {
        return bug_report::run(&parse_bug_report_args(&args));
    }

    if args.iter().any(|a| a == "--check-update") {
        update_check::run();
        std::process::exit(0);
    }

    // ADR-111決定4: 自己昇格フローの昇格側エントリポイント。GUIは起動せず
    // レジストリ操作のみ行い、終了コードで結果を返す（`--bug-report`と
    // 同型のヘッドレス分岐パターン）。
    if let Some(mode) = arg_value(&args, "--scancode-map") {
        let enable = match mode {
            "on" => true,
            "off" => false,
            _ => {
                log::error!("[scancode-map] 不正な --scancode-map 値: {mode}");
                std::process::exit(1);
            }
        };
        std::process::exit(scancode_map_admin::run_elevated_worker(enable));
    }

    let viewport = egui::ViewportBuilder::default()
        // 幅 760: サイドパネル(100) + 配列編集タブの最も幅を要する行（JIS 最上段
        // 13キー、ボタン min_size 40px + item_spacing 8px ≈ 616px）+ 余白/
        // スクロールバー分。580 だと draw_layout_keyboard_grid()
        // (crates/awase-settings/src/main.rs) のボタン幅が「⌫BS」等4文字
        // ラベルの視認性のため 34px→40px に広がった後もここが追随しておらず、
        // デフォルトサイズで開くと配列編集タブのキーボード図が右へはみ出す
        // ユーザー報告があった（2026-09-03。ウィンドウを広げれば表示は正常に
        // 戻る＝致命的ではないが既定値の追随漏れ）。
        .with_inner_size([760.0, 650.0])
        // ウィンドウを小さくしても全項目にスクロール + 下部固定ボタンで届くため、
        // 低解像度・高 DPI ディスプレイでも操作不能にならない下限だけ設ける。
        .with_min_inner_size([420.0, 320.0])
        .with_title("awase 設定");
    startup_failure::run_with_fallback("awase-settings", viewport, |cc| {
        Box::new(SettingsApp::new(cc)) as Box<dyn eframe::App>
    })
}

fn parse_bug_report_args(args: &[String]) -> bug_report::BugReportArgs {
    let journal_path = arg_value(args, "--journal").map(PathBuf::from);
    let diagnostics_path = arg_value(args, "--diagnostics").map(PathBuf::from);
    let app_log_path = arg_value(args, "--applog").map(PathBuf::from);
    let ime_kind = arg_value(args, "--ime-kind")
        .and_then(|s| s.parse().ok())
        .unwrap_or(awase_windows::bug_report::BugReportImeKind::Unknown);
    bug_report::BugReportArgs {
        journal_path,
        ime_kind,
        diagnostics_path,
        app_log_path,
    }
}

fn arg_value<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].as_str())
}

/// 各 bool は無関係な由来（keymap キャプチャの修飾キー3つ、配列編集タブの
/// dirty フラグ1つ、ADR-099 決定4の確認モーダル開閉フラグ1つ）を持つ独立
/// したフラグであり、bitflags 化や enum への統合は可読性を下げるだけ
/// なので許容する。
#[expect(clippy::struct_excessive_bools)]
struct SettingsApp {
    config: awase::config::AppConfig,
    config_path: std::path::PathBuf,
    /// 直近の `AppConfig::load` 結果の分類（ADR-099 決定4）。`Dangerous` の
    /// 間は `apply()` が無条件保存せず、確認・バックアップを必須にする。
    config_load_state: awase::config::ConfigLoadState,
    /// `config_load_state` が `Dangerous` のときに `apply()` が表示する
    /// 確認モーダルを開いているか。
    show_dangerous_save_confirm: bool,
    status: String,
    active_tab: Tab,
    available_layouts: Vec<String>,
    // Key list add-buffers (engine/IME control: modifiers + main key)
    new_engine_on: NewComboBuf,
    new_engine_off: NewComboBuf,
    new_ime_on: NewComboBuf,
    new_ime_off: NewComboBuf,
    new_ime_toggle: NewComboBuf,
    // Keymap rule add-buffers
    new_keymap_app: String,
    new_keymap_from_ctrl: bool,
    new_keymap_from_shift: bool,
    // Alt は GUI から選べない（ADR-114 決定5 — バックエンドが 'from' の Alt
    // 修飾を禁止・skip するため）。
    new_keymap_from_main: String,
    new_keymap_to_main: String,
    // Keymap capture mode (None = not capturing)
    capturing: Option<CaptureTarget>,
    // アプリ別タブ add-buffers: (process, class) × force_text/force_bypass/force_vk/force_tsf
    new_override_bufs: [(String, String); 4],
    // disable_apps add-buffer（プロセス名のみ、class 不要）
    new_disable_app: String,
    // post_bypass add-buffers
    new_pb_key: String,
    new_pb_process: String,
    new_pb_class: String,
    // ── 配列編集タブの状態（旧 awase-yab-editor バイナリを統合） ──
    layout: YabLayout,
    layout_file_path: Option<PathBuf>,
    layout_file_path_buf: String,
    layout_current_face: Face,
    layout_selected_pos: Option<PhysicalPos>,
    /// 「コピー」で選択中セルの生の値を先頭に積む履歴（最大
    /// `CLIPBOARD_HISTORY_LEN` 件、面をまたいでも保持する）。履歴の項目を
    /// クリックすると選択中セルへそのまま貼り付ける。テキスト欄を経由しない
    /// ため、ローマ字の かな 解決結果なども含めて正確に複製できる。
    layout_clipboard_history: Vec<YabValue>,
    layout_edit_kind: ValueKind,
    layout_edit_value: String,
    layout_edit_special_idx: usize,
    kana_table: KanaTable,
    layout_modified: bool,
    layout_status: String,
    /// 「配列編集」タブを一度でも開いたか（開いたときに一度だけ .yab を
    /// 読み込む。起動時の同期読み込みを避けるため）。
    layout_loaded: bool,
    /// rfd の非同期ファイルダイアログから戻ってきたパス（開く）
    layout_pending_open: Option<PathBuf>,
    /// rfd の非同期ファイルダイアログから戻ってきたパス（名前を付けて保存）
    layout_pending_save_as: Option<PathBuf>,
    /// コードレビュー指摘: `AppConfig::save()`（ADR-099 決定3、fsync+
    /// rename・失敗時50ms×最大4回リトライ）を `update()` から同期呼び出し
    /// すると、rename が失敗するケース（AV/OneDrive 等のロック）で最大
    /// 200ms 設定ウィンドウが再描画不能になる。バックアップ＋保存を
    /// バックグラウンドスレッドで実行し、この `Receiver` を毎フレーム
    /// ノンブロッキングでポーリングする。
    pending_save: Option<std::sync::mpsc::Receiver<PendingSaveResult>>,
    /// Caps(英数)⇔Ctrl 入れ替えプリセット（ADR-111）の現在の Scancode Map
    /// 状態キャッシュ。`None` はまだ一度も読んでいないことを示す（タブを
    /// 開いた時と操作直後にのみ読み直す、毎フレーム `RegGetValueW` しない）。
    scancode_map_status: Option<scancode_map_admin::ScancodeMapStatus>,
    /// 直近の有効化/無効化操作の結果メッセージ（GUI表示用）。
    scancode_map_last_message: Option<String>,
    /// 起動時設定診断（ADR-116）: `config.validate()` 警告 + `layouts_dir`
    /// 内の全 `.yab` の読込失敗/`yab::lint()` 警告。`recompute_diagnostics()`
    /// で計算し、`config_path_panel` に表示する。空なら表示しない。
    startup_diagnostics: Vec<String>,
}

/// バックグラウンドスレッドで実行する保存処理の結果。
enum PendingSaveResult {
    /// 保存前バックアップ（`config.toml.bak`）の作成に失敗し、安全のため
    /// 保存そのものを中止した。
    BackupFailed(String),
    /// `AppConfig::save()` が失敗した。
    SaveFailed(String),
    /// 保存に成功した。`warnings` は `validate()` が返した警告文。
    Saved { warnings: Vec<String> },
}

impl SettingsApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        setup_fonts(&cc.egui_ctx);
        let config_path = find_config_path();
        let (config, config_load_state) = match awase::config::AppConfig::load(&config_path) {
            Ok(cfg) => (cfg, awase::config::ConfigLoadState::Loaded),
            Err(e) => {
                let state = awase::config::classify_load_error(&e);
                log::warn!("Config load failed: {e} (classified as {state:?}), using defaults");
                (default_config(), state)
            }
        };
        let available_layouts = scan_layout_names(&config.general.layouts_dir);

        let mut app = Self {
            config,
            config_path,
            config_load_state,
            show_dangerous_save_confirm: false,
            status: String::new(),
            active_tab: Tab::Basic,
            available_layouts,
            new_engine_on: NewComboBuf::default(),
            new_engine_off: NewComboBuf::default(),
            new_ime_on: NewComboBuf::default(),
            new_ime_off: NewComboBuf::default(),
            new_ime_toggle: NewComboBuf::default(),
            new_keymap_app: String::new(),
            new_keymap_from_ctrl: false,
            new_keymap_from_shift: false,
            new_keymap_from_main: String::new(),
            new_keymap_to_main: String::new(),
            capturing: None,
            new_override_bufs: Default::default(),
            new_disable_app: String::new(),
            new_pb_key: String::new(),
            new_pb_process: String::new(),
            new_pb_class: String::new(),
            // 配列編集タブの状態は「配列編集」タブを開くまで読み込まない
            // （ensure_layout_loaded 参照）。起動時に毎回 .yab を同期的に
            // 読み込むと、ウィンドウ生成〜最初の描画までの間が延び、実機で
            // 「黒い画面が出てから編集画面が出る」形で体感された。
            layout: empty_yab_layout(),
            layout_file_path: None,
            layout_file_path_buf: String::new(),
            layout_current_face: Face::Normal,
            layout_selected_pos: None,
            layout_clipboard_history: Vec::new(),
            layout_edit_kind: ValueKind::None,
            layout_edit_value: String::new(),
            layout_edit_special_idx: 0,
            kana_table: KanaTable::build(),
            layout_modified: false,
            layout_status: String::new(),
            layout_loaded: false,
            layout_pending_open: None,
            layout_pending_save_as: None,
            pending_save: None,
            scancode_map_status: None,
            scancode_map_last_message: None,
            startup_diagnostics: Vec::new(),
        };
        app.recompute_diagnostics();
        app
    }

    /// `apply()` の実処理。`config_load_state` が `Dangerous`（読み込み失敗で
    /// 初期値を表示中）のときは呼び出し元（`apply()`）で確認を挟んでから
    /// これを呼ぶこと（ADR-099 決定4）。
    /// 保存前バックアップ＋`AppConfig::save()`（ADR-099 決定3、fsync+
    /// rename・失敗時最大200msリトライ）をバックグラウンドスレッドへ
    /// 委譲して起動する。呼び出し元スレッド（egui の UI スレッド）は
    /// 一切ブロックしない。完了は `poll_pending_save()` が毎フレーム
    /// ノンブロッキングで確認する。
    ///
    /// コードレビュー指摘: 以前はこの関数自体が `AppConfig::save()` を
    /// 同期呼び出ししており、rename が失敗するケース（AV/OneDrive 等の
    /// ロック）で設定ウィンドウ全体が最大200ms再描画不能になっていた。
    /// `crates/awase-windows/src/tray.rs::save_auto_start_config`
    /// （こちらはエンジンスレッド上で動くため ADR-099 round2 SF-1 で
    /// 別途トレードオフとして許容済み）とは異なり、こちらは最も高頻度に
    /// 呼ばれる呼び出し元（「適用」ボタン）であり、非同期化の効果が大きい。
    fn apply_confirmed(&mut self) {
        if self.pending_save.is_some() {
            // 前回の保存がまだ完了していなければ多重に起動しない。
            // コードレビュー指摘: ボタン自体は無効化していないため、
            // 連打すると何も起きていないように見えてしまう。ステータスで
            // 「保存中」を明示し、無反応に見えないようにする。
            self.status = "保存中です。少々お待ちください…".to_string();
            return;
        }

        // /code-review指摘（PR #127、2回目）: self.configはこの直後に
        // AppConfig::from(validated)で上書きされるため、事前の
        // `self.config.clone()`はvalidate()に渡した瞬間に捨てられる
        // 無駄なdeep clone（keymaps/app_overridesのVecまで含む）だった。
        // mem::takeで元の値をmoveし、cloneを避ける。
        let (validated, warnings) = std::mem::take(&mut self.config).validate();
        if !warnings.is_empty() {
            self.status = format!("警告: {}", warnings.join("; "));
        }
        // /code-review指摘: 以前はvalidate()の戻り値（confirm_mode="speculative"
        // → two_phase正規化等を含む）を警告文の生成にしか使わず、保存対象は
        // 未検証のself.configのcloneだった。設定画面がconfirm_modeを一切
        // 表示しなくなった今、この正規化はユーザーが手で直す手段が無いため、
        // 保存されずに警告だけが「適用」を押すたび永遠に再表示され続けていた。
        // validated側をself.configにも反映し、保存対象にする。
        self.config = awase::config::AppConfig::from(validated);
        // /code-review指摘（PR #133）: self.config はここで既に正規化済みの
        // 新しい値に置き換わっている。診断リストの再計算をバックグラウンド
        // 保存の結果（Saved分岐）だけに任せると、保存が BackupFailed/
        // SaveFailed で失敗した場合に self.config は変わっているのに
        // 診断リストだけ古いままになる。保存の成否によらず、config が
        // 変わった時点で再計算する。
        self.recompute_diagnostics();
        let clone = self.config.clone();

        let config_path = self.config_path.clone();
        let is_dangerous = matches!(
            self.config_load_state,
            awase::config::ConfigLoadState::Dangerous(_)
        );

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            // 保存前バックアップ: config_load_state が Dangerous のとき、既存
            // ファイルを一度だけ config.toml.bak へ退避する（round1 指摘 M4:
            // 無条件だと2回目の適用で「壊れた元ファイルのバックアップ」自体を
            // 上書きしてしまう）。
            //
            // コードレビュー指摘 C2: バックアップに失敗しても以前は
            // `log::warn!` するだけで保存を続行しており、Dangerous を招いた
            // 原因（PermissionDenied・共有違反など）はバックアップの
            // 読み取り＝コピーも失敗させやすいため、最もバックアップが必要な
            // 場面でこそ原本が無防備に上書きされ得た。バックアップ対象の
            // 既存ファイルがあるのにコピーに失敗した場合は、保存そのものを
            // 中止しユーザーに見える形でエラーを出す。
            if is_dangerous {
                let bak_path = config_path.with_extension("toml.bak");
                if config_path.exists() && !bak_path.exists() {
                    if let Err(e) = std::fs::copy(&config_path, &bak_path) {
                        let _ = tx.send(PendingSaveResult::BackupFailed(format!(
                            "既存の {} をバックアップできませんでした（{e}）。\
                             安全のため保存を中止しました。",
                            config_path.display()
                        )));
                        return;
                    }
                    log::info!("Backed up unreadable config to {}", bak_path.display());
                }
            }

            match clone.save(&config_path) {
                Ok(()) => {
                    let _ = tx.send(PendingSaveResult::Saved { warnings });
                }
                Err(e) => {
                    let _ = tx.send(PendingSaveResult::SaveFailed(e.to_string()));
                }
            }
        });
        self.pending_save = Some(rx);
    }

    /// `apply_confirmed()` がバックグラウンドスレッドへ委譲した保存処理の
    /// 完了を毎フレームノンブロッキングで確認し、完了していれば
    /// `status`/`config_load_state` を更新する。
    fn poll_pending_save(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.pending_save else {
            return;
        };
        match rx.try_recv() {
            Ok(PendingSaveResult::Saved { warnings }) => {
                if warnings.is_empty() {
                    self.status = "設定を保存しました".to_string();
                }
                self.config_load_state = awase::config::ConfigLoadState::Loaded;
                // apply_confirmed() は保存開始前にも recompute_diagnostics()
                // を呼ぶが、その時点で config_load_state がまだ Dangerous
                // だった場合はガードに掛かり何もしない。Dangerous から
                // 復帰する保存が成功したこの分岐でも改めて呼び直す必要がある。
                self.recompute_diagnostics();
                send_reload_config_message();
                self.pending_save = None;
            }
            Ok(PendingSaveResult::BackupFailed(msg)) => {
                self.status = format!("保存失敗: {msg}");
                self.pending_save = None;
            }
            Ok(PendingSaveResult::SaveFailed(e)) => {
                self.status = format!("保存失敗: {e}");
                self.pending_save = None;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                // まだ完了していない。ポーリングを継続するため再描画を要求する。
                ctx.request_repaint();
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.pending_save = None;
            }
        }
    }

    /// 「適用」ボタンのハンドラ。`config_load_state` が `Dangerous`
    /// （読み込み失敗により現在初期値を表示中）なら、無条件保存の代わりに
    /// 確認モーダルを開く（`show_dangerous_save_confirm`、実処理は
    /// `apply_confirmed()`）。それ以外は通常通り即保存する。
    fn apply(&mut self) {
        if matches!(
            self.config_load_state,
            awase::config::ConfigLoadState::Dangerous(_)
        ) {
            self.show_dangerous_save_confirm = true;
        } else {
            self.apply_confirmed();
        }
    }

    fn cancel(&mut self) {
        // コードレビュー指摘: 確認モーダル（`show_dangerous_save_confirm_modal`）は
        // `egui::Modal` のような背景ブロッキングを持たないため、モーダル表示中でも
        // 下部パネルの「キャンセル」ボタンが押せてしまう。ここで cancel() が
        // 呼ばれた場合はモーダルの前提（Dangerous 状態）が解消されるため、
        // 開いたままのモーダルが状態と矛盾しないよう必ず閉じる。
        self.show_dangerous_save_confirm = false;
        match awase::config::AppConfig::load(&self.config_path) {
            Ok(cfg) => {
                self.available_layouts = scan_layout_names(&cfg.general.layouts_dir);
                self.config = cfg;
                self.config_load_state = awase::config::ConfigLoadState::Loaded;
                self.status = "変更を破棄しました".to_string();
            }
            Err(e) => {
                self.config_load_state = awase::config::classify_load_error(&e);
                self.status = format!("読み込み失敗: {e}");
            }
        }
        self.recompute_diagnostics();
    }

    /// 起動時設定診断（ADR-116）を再計算する。`config.validate()` の警告と、
    /// `layouts_dir` 内の全 `.yab` の読込失敗/`yab::lint()` 警告を集める。
    ///
    /// `config_load_state` が `Dangerous`（`config.toml` の読込自体に失敗し
    /// `default_config()` を表示中）のときは何もしない——その既定設定を
    /// 診断しても無意味な結果が、本当に重要な「読込失敗」という赤字警告
    /// （`config_path_panel`）の隣に並ぶだけになる。
    ///
    /// `config.validate()` は `self.config.clone()` に対して呼び、
    /// `self.config` 自体は変更しない（`apply_confirmed` の `mem::take` +
    /// 書き戻しパターンはここでは使わない）。書き戻すと `validate_layouts`
    /// の正規化（`..` を含むパスを `"layout"` に書き換える）が確定して
    /// しまい、ユーザーがまだ「適用」を押していないのに画面表示上の
    /// `layouts_dir` が黙って変わる（`recompute_diagnostics` は保存や
    /// キャンセルとは無関係な複数箇所から呼ばれるため、この副作用は
    /// 許容できない）。
    fn recompute_diagnostics(&mut self) {
        if matches!(
            self.config_load_state,
            awase::config::ConfigLoadState::Dangerous(_)
        ) {
            self.startup_diagnostics.clear();
            return;
        }

        let layouts_dir = resolve_layouts_dir(&self.config.general.layouts_dir);
        let mut diagnostics = scan_yab_files_for_diagnostics(&layouts_dir);

        // ここは apply_confirmed の mem::take + 書き戻しパターンを**あえて
        // 使わない**（Opus敵対的レビュー指摘、r3→r4）。書き戻すと
        // `validate_layouts` の正規化（".." 含みパスを "layout" へ書き換え）
        // が `self.config` に確定してしまい、ユーザーがまだ「適用」を押して
        // いないのに画面表示上の `layouts_dir` が黙って変わる（例:
        // 設定画面を開いただけで "../foo" が "layout" に見える）。
        // `apply_confirmed` は正規化を保存対象にする必要があるため書き戻しが
        // 正しいが、ここは警告文を覗くだけなので clone で十分——
        // `AppConfig` は小さく、この関数はユーザー操作のたび（毎フレームでは
        // ない）にしか呼ばれない。
        let (_, warnings) = self.config.clone().validate();
        diagnostics.extend(warnings);

        self.startup_diagnostics = diagnostics;
    }

    /// 現在編集している config.toml の実パスを常時表示する上部パネル。
    ///
    /// awase.exe と awase-settings.exe はそれぞれ独立に config.toml のパスを
    /// 解決する（find_config_path()、コマンドライン引数 → 実行ファイル隣 →
    /// ワークスペースルート、の優先順位で決まる）ため、起動方法や配置次第では
    /// 2つの実行ファイルが異なる config.toml を読み書きしてしまい得る
    /// （設定画面で保存しても awase.exe に反映されない、という実機バグとして
    /// 2026-07-19 に確認済み）。この表示により、少なくとも「今どのファイルを
    /// 編集しているか」を一目で確認・比較できるようにする。
    ///
    /// あわせて `config_load_state` が `Dangerous`（読み込み失敗）の間は、
    /// 現在表示中の内容が実ファイルではなく初期値であることを常時警告する
    /// （ADR-099 決定4、`update()` の行数を抑えるため別関数に分離）。
    fn config_path_panel(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("config_path_panel").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("設定ファイル:");
                let display_path = self
                    .config_path
                    .canonicalize()
                    .unwrap_or_else(|_| self.config_path.clone());
                ui.monospace(display_path.display().to_string())
                    .on_hover_text(
                        "awase.exe が実際に読み込む config.toml と同じパスか確認してください。\n\
                         起動方法（コマンドライン引数の有無・カレントディレクトリ）によっては\n\
                         別のファイルを指している場合があります。",
                    );
                if ui.small_button("フォルダを開く").clicked()
                    && let Some(dir) = display_path.parent()
                {
                    let _ = std::process::Command::new("explorer")
                        .arg("/select,")
                        .arg(&display_path)
                        .spawn()
                        .or_else(|_| std::process::Command::new("explorer").arg(dir).spawn());
                }
                // バージョン表示要望（2026-07-29）: これまでインストール済みファイル名
                // でしかバージョンを確認できなかったため、常時見える位置に出す。
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.hyperlink_to("ホームページ", HOMEPAGE_URL);
                    ui.label(format!("awase v{}", env!("CARGO_PKG_VERSION")));
                });
            });
            if let awase::config::ConfigLoadState::Dangerous(reason) = &self.config_load_state {
                ui.add_space(4.0);
                let bak_path = self.config_path.with_extension("toml.bak");
                let bak_note = if bak_path.exists() {
                    format!("（バックアップ: {}）", bak_path.display())
                } else {
                    String::new()
                };
                ui.colored_label(
                    egui::Color32::from_rgb(200, 60, 60),
                    format!(
                        "⚠ 設定ファイルの読み込みに失敗したため、現在表示中の内容は初期値です。\
                         このまま「適用」すると既存の設定を失う可能性があります{bak_note}。\
                         （原因: {reason}）"
                    ),
                );
            }
            if !self.startup_diagnostics.is_empty() {
                ui.add_space(4.0);
                egui::CollapsingHeader::new(format!(
                    "⚠ 設定の診断結果（{}件）",
                    self.startup_diagnostics.len()
                ))
                .default_open(false)
                .show(ui, |ui| {
                    // /code-review指摘: 警告件数が多いと TopBottomPanel が
                    // 際限なく伸びて中央の設定UIを押し出しかねないため、
                    // 高さを固定してスクロール可能にする。
                    egui::ScrollArea::vertical()
                        .max_height(150.0)
                        .show(ui, |ui| {
                            for msg in &self.startup_diagnostics {
                                ui.label(msg);
                            }
                        });
                });
            }
            ui.add_space(4.0);
        });
    }

    /// `config_load_state` が `Dangerous` の状態で「適用」を押したときの
    /// 確認モーダル（ADR-099 決定4）。既存コードの `rfd::AsyncFileDialog` は
    /// UI スレッドから直接ネイティブダイアログを呼ばず `thread::spawn` +
    /// `pollster::block_on` で迂回する設計になっているため、その方針に
    /// 合わせてネイティブダイアログ（`rfd::MessageDialog`）は使わず、
    /// egui 内製の `egui::Window`（最前面固定でモーダル相当）で実装する。
    fn show_dangerous_save_confirm_modal(&mut self, ctx: &egui::Context) {
        if !self.show_dangerous_save_confirm {
            return;
        }
        let mut open = true;
        let mut confirmed = false;
        let mut cancelled = false;
        egui::Window::new("確認")
            .id(egui::Id::new("dangerous_save_confirm"))
            .order(egui::Order::Foreground)
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(
                    "設定ファイルの読み込みに失敗したため、現在表示中の内容は初期値です。\n\
                     このまま保存すると既存の設定を失う可能性があります。続行しますか？",
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("続行").clicked() {
                        confirmed = true;
                    }
                    if ui.button("キャンセル").clicked() {
                        cancelled = true;
                    }
                });
            });
        if confirmed {
            self.show_dangerous_save_confirm = false;
            self.apply_confirmed();
        } else if cancelled || !open {
            self.show_dangerous_save_confirm = false;
        }
    }

    // ── 配列編集タブ（旧 awase-yab-editor）──

    const fn layout_face_mut(&mut self, face: Face) -> &mut YabFace {
        match face {
            Face::Normal => &mut self.layout.normal,
            Face::LeftThumb => &mut self.layout.left_thumb,
            Face::RightThumb => &mut self.layout.right_thumb,
            Face::Shift => &mut self.layout.shift,
            Face::LeftThumbShift => &mut self.layout.left_thumb_shift,
            Face::RightThumbShift => &mut self.layout.right_thumb_shift,
        }
    }

    const fn layout_face(&self, face: Face) -> &YabFace {
        match face {
            Face::Normal => &self.layout.normal,
            Face::LeftThumb => &self.layout.left_thumb,
            Face::RightThumb => &self.layout.right_thumb,
            Face::Shift => &self.layout.shift,
            Face::LeftThumbShift => &self.layout.left_thumb_shift,
            Face::RightThumbShift => &self.layout.right_thumb_shift,
        }
    }

    fn select_layout_cell(&mut self, pos: PhysicalPos) {
        self.layout_selected_pos = Some(pos);
        let value = self
            .layout_face(self.layout_current_face)
            .get(&pos)
            .cloned();
        match value {
            Some(YabValue::Romaji { romaji, .. }) => {
                self.layout_edit_kind = ValueKind::Keystroke;
                self.layout_edit_value = romaji;
            }
            Some(YabValue::Literal(s)) => {
                self.layout_edit_kind = ValueKind::Literal;
                self.layout_edit_value = s;
            }
            Some(YabValue::KeySequence(s)) => {
                self.layout_edit_kind = ValueKind::Keystroke;
                self.layout_edit_value = s;
            }
            Some(YabValue::Special(sk)) => {
                self.layout_edit_kind = ValueKind::Special;
                self.layout_edit_special_idx =
                    SPECIAL_KEYS.iter().position(|(k, _)| *k == sk).unwrap_or(0);
                self.layout_edit_value.clear();
            }
            Some(YabValue::Vk(vk)) => {
                self.layout_edit_kind = ValueKind::Vk;
                self.layout_edit_value = format!("{:X}", vk.0);
            }
            // ADR-115: 打鍵列(CtrlChord/InlineSequence/MacroRef)のGUI編集は
            // 非対象。生テキストをLiteralとして表示するに留める——保存すると
            // 打鍵列としての意味を失う既知の限界（決定9(a)）。Sequenceは
            // resolve_keystroke_syntax後のプレビュー専用コピーにしか
            // 現れないためNone扱いでよい。
            Some(
                v @ (YabValue::CtrlChord { .. }
                | YabValue::InlineSequence { .. }
                | YabValue::MacroRef(_)),
            ) => {
                self.layout_edit_kind = ValueKind::Literal;
                self.layout_edit_value = v.serialize();
            }
            Some(YabValue::Sequence(_) | YabValue::None) | None => {
                self.layout_edit_kind = ValueKind::None;
                self.layout_edit_value.clear();
            }
        }
    }

    /// 選択中セルの生の値を履歴の先頭に積む。同じ値が履歴に既にあれば
    /// 重複させず先頭へ移動する。`CLIPBOARD_HISTORY_LEN` 件を超えた古い
    /// 項目は捨てる。
    fn copy_layout_cell(&mut self) {
        let Some(pos) = self.layout_selected_pos else {
            return;
        };
        let value = self
            .layout_face(self.layout_current_face)
            .get(&pos)
            .cloned()
            .unwrap_or(YabValue::None);
        self.layout_status = format!("履歴にコピーしました: {}", cell_tooltip(Some(&value), pos));
        self.layout_clipboard_history.retain(|v| v != &value);
        self.layout_clipboard_history.insert(0, value);
        self.layout_clipboard_history
            .truncate(CLIPBOARD_HISTORY_LEN);
    }

    /// 履歴の項目を選択中セルへそのまま書き込む（面をまたいでも可）。
    /// テキスト欄（打鍵/リテラル入力）を経由しないため、ローマ字の かな
    /// 解決結果を含めてコピー元と完全に同じ値になる。
    fn paste_layout_cell(&mut self, value: YabValue) {
        let Some(pos) = self.layout_selected_pos else {
            return;
        };
        self.layout_status = format!("貼り付けました: {}", cell_tooltip(Some(&value), pos));
        self.layout_face_mut(self.layout_current_face)
            .insert(pos, value);
        self.layout_modified = true;
        // 編集パネルの表示も貼り付け後の値に合わせて更新する。
        self.select_layout_cell(pos);
    }

    fn apply_layout_edit(&mut self) {
        let Some(pos) = self.layout_selected_pos else {
            return;
        };
        let value = match self.layout_edit_kind {
            ValueKind::Keystroke => {
                let input = normalize_keystroke_input(&self.layout_edit_value);
                if input.is_empty() {
                    YabValue::None
                } else if let Some(bad) = find_invalid_keystroke_char(&input) {
                    self.layout_status =
                        format!("「{bad}」は JIS キーボード上のキーとして入力できません");
                    return;
                } else if input.chars().all(|c| c.is_ascii_alphabetic()) {
                    let kana = self.kana_table.kana_for_romaji(&input);
                    YabValue::Romaji {
                        romaji: input,
                        kana,
                    }
                } else {
                    YabValue::KeySequence(input)
                }
            }
            ValueKind::Literal => {
                let s = self.layout_edit_value.clone();
                if s.is_empty() {
                    YabValue::None
                } else {
                    YabValue::Literal(s)
                }
            }
            ValueKind::Special => YabValue::Special(SPECIAL_KEYS[self.layout_edit_special_idx].0),
            ValueKind::Vk => {
                let hex = self.layout_edit_value.trim().trim_start_matches('V');
                if hex.is_empty() {
                    YabValue::None
                } else if let Ok(code) = u16::from_str_radix(hex, 16) {
                    YabValue::Vk(VkCode(code))
                } else {
                    self.layout_status = format!("「{hex}」は16進数として解釈できません");
                    return;
                }
            }
            ValueKind::None => YabValue::None,
        };
        self.layout_face_mut(self.layout_current_face)
            .insert(pos, value);
        self.layout_modified = true;
        self.layout_status = "変更あり".to_string();
    }

    fn layout_do_save(&mut self) {
        let path = self.layout_file_path.clone();
        match path {
            Some(p) => self.layout_write_to_path(&p),
            None => self.layout_do_save_as_dialog(),
        }
    }

    fn layout_write_to_path(&mut self, path: &Path) {
        let text = self.layout.serialize(self.config.general.keyboard_model);
        match std::fs::write(path, &text) {
            Ok(()) => {
                self.layout_file_path = Some(path.to_path_buf());
                self.layout_file_path_buf = path.display().to_string();
                self.layout_modified = false;
                // 書き込み成功後にのみ lint する（失敗時に計算を無駄にしない）。
                let lint_warnings = awase::yab::lint(&text);
                self.layout_status = append_lint_warnings(
                    format!("{} に保存しました", path.display()),
                    &lint_warnings,
                );
                // ADR-116: 配列編集タブでの保存直後に上部パネルの診断リストが
                // 古いまま（クォート崩れを直したのに警告が残る等）にならない
                // よう再計算する。
                self.recompute_diagnostics();
            }
            Err(e) => self.layout_status = format!("保存失敗: {e}"),
        }
    }

    fn layout_do_open_dialog(&mut self) {
        let task = rfd::AsyncFileDialog::new()
            .set_title("配列ファイルを開く")
            .add_filter("YAB 配列ファイル", &["yab"])
            .add_filter("すべてのファイル", &["*"])
            .pick_file();
        let result = std::thread::spawn(move || {
            let handle = pollster::block_on(task);
            handle.map(|h| PathBuf::from(h.path()))
        });
        if let Ok(maybe_path) = result.join() {
            self.layout_pending_open = maybe_path;
        }
    }

    fn layout_do_save_as_dialog(&mut self) {
        let task = rfd::AsyncFileDialog::new()
            .set_title("名前を付けて保存")
            .add_filter("YAB 配列ファイル", &["yab"])
            .add_filter("すべてのファイル", &["*"])
            .save_file();
        let result = std::thread::spawn(move || {
            let handle = pollster::block_on(task);
            handle.map(|h| PathBuf::from(h.path()))
        });
        if let Ok(Some(path)) = result.join() {
            self.layout_pending_save_as = Some(path);
        }
    }

    fn layout_load_from_path(&mut self, path: &Path) {
        match load_yab_layout(path, self.config.general.keyboard_model) {
            Ok((ly, lint_warnings)) => {
                self.layout = ly;
                self.layout_file_path_buf = path.display().to_string();
                self.layout_file_path = Some(path.to_path_buf());
                self.layout_modified = false;
                self.layout_selected_pos = None;
                self.layout_status = append_lint_warnings(
                    format!("{} を読み込みました", path.display()),
                    &lint_warnings,
                );
            }
            Err(e) => self.layout_status = format!("読み込み失敗: {e}"),
        }
    }

    fn layout_do_open_from_text_box(&mut self) {
        let path = PathBuf::from(&self.layout_file_path_buf);
        self.layout_load_from_path(&path);
    }

    fn layout_do_reload(&mut self) {
        let Some(path) = self.layout_file_path.clone() else {
            self.layout_status = "ファイルパスが未設定です".to_string();
            return;
        };
        match load_yab_layout(&path, self.config.general.keyboard_model) {
            Ok((ly, lint_warnings)) => {
                self.layout = ly;
                self.layout_modified = false;
                self.layout_selected_pos = None;
                self.layout_status = append_lint_warnings(
                    format!("{} を再読み込みしました", path.display()),
                    &lint_warnings,
                );
            }
            Err(e) => self.layout_status = format!("再読み込み失敗: {e}"),
        }
    }

    /// 「配列編集」タブを開いたときに一度だけ設定中のレイアウトファイルを
    /// 読み込む。`SettingsApp::new` で毎回同期読み込みすると、アプリ起動〜
    /// 最初の描画までの間が延びるため遅延させている。
    fn ensure_layout_loaded(&mut self) {
        if self.layout_loaded {
            return;
        }
        self.layout_loaded = true;
        log_checkpoint("ensure_layout_loaded 開始");
        let start = std::time::Instant::now();
        let path = resolve_layouts_dir(&self.config.general.layouts_dir)
            .join(&self.config.general.default_layout);
        log_checkpoint(&format!(
            "resolve_layouts_dir 完了: {}ms (path={})",
            start.elapsed().as_millis(),
            path.display()
        ));
        self.layout_load_from_path(&path);
        log_checkpoint(&format!(
            "layout_load_from_path 完了: 合計 {}ms",
            start.elapsed().as_millis()
        ));
    }

    fn drain_layout_pending_async(&mut self) {
        if let Some(path) = self.layout_pending_open.take() {
            self.layout_load_from_path(&path);
        }
        if let Some(path) = self.layout_pending_save_as.take() {
            self.layout_write_to_path(&path);
        }
    }

    /// 配列編集タブ表示中のみキーボードショートカットを解釈する
    /// （他タブでの入力中に Ctrl+S 等を奪わないため）。
    fn handle_layout_shortcuts(&mut self, ctx: &egui::Context) {
        if self.active_tab != Tab::Layout {
            return;
        }
        if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::S)) {
            self.layout_do_save();
        }
        if ctx.input(|i| i.modifiers.ctrl && !i.modifiers.shift && i.key_pressed(egui::Key::O)) {
            self.layout_do_open_dialog();
        }
        if ctx.input(|i| i.modifiers.ctrl && i.modifiers.shift && i.key_pressed(egui::Key::S)) {
            self.layout_do_save_as_dialog();
        }
        if ctx.input(|i| i.key_pressed(egui::Key::F5)) {
            self.layout_do_reload();
        }
    }

    /// キャプチャモード中に押されたキーを処理する。
    fn process_keymap_capture(&mut self, ctx: &egui::Context) {
        let Some(target) = self.capturing else { return };
        let captured: Option<CapturedKey> = ctx.input(|i| {
            for ev in &i.events {
                if let egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } = ev
                {
                    // 修飾キーなしの Esc はキャンセル扱い（Ctrl+Esc 等は通常のキーとして捕捉）
                    if *key == egui::Key::Escape && modifiers.is_none() {
                        return Some(CapturedKey::Cancel);
                    }
                    if let Some(internal) = egui_key_to_internal(*key) {
                        return Some(CapturedKey::Key {
                            internal: internal.to_string(),
                            ctrl: modifiers.ctrl,
                            shift: modifiers.shift,
                            alt: modifiers.alt,
                        });
                    }
                }
            }
            None
        });

        let Some(captured) = captured else { return };
        match captured {
            CapturedKey::Cancel => {
                self.capturing = None;
            }
            CapturedKey::Key {
                internal,
                ctrl,
                shift,
                alt,
            } => {
                // Alt はキャプチャで押されていても [[keymap]] の from には反映しない
                // （ADR-114 決定5 — バックエンドが 'from' の Alt 修飾を禁止・skip する
                // ため、GUI 側でも作れないようにして対称性を保つ）。
                let _ = alt;
                match target {
                    CaptureTarget::ExistingFrom(i) => {
                        if let Some(rule) = self.config.keymaps.get_mut(i) {
                            rule.from = format_combo(ctrl, shift, false, &internal);
                        }
                    }
                    CaptureTarget::ExistingTo(i) => {
                        if let Some(rule) = self.config.keymaps.get_mut(i) {
                            rule.to = Some(internal);
                        }
                    }
                    CaptureTarget::NewFrom => {
                        self.new_keymap_from_ctrl = ctrl;
                        self.new_keymap_from_shift = shift;
                        self.new_keymap_from_main = internal;
                    }
                    CaptureTarget::NewTo => {
                        self.new_keymap_to_main = internal;
                    }
                }
                self.capturing = None;
            }
        }
    }
}

/// `process_keymap_capture` の内部結果型。
enum CapturedKey {
    Cancel,
    Key {
        internal: String,
        ctrl: bool,
        shift: bool,
        alt: bool,
    },
}

// ── Tab methods ──

impl SettingsApp {
    #[expect(clippy::too_many_lines)]
    fn tab_basic(&mut self, ui: &mut egui::Ui) {
        ui.heading("全般設定");
        ui.add_space(4.0);

        let threshold_hover = "同時打鍵と判定する時間の幅です。この値を大きくするほど判定が甘く\n(親指シフトが入りやすく)なりますが、遅延が増えます。\n100ms が NICOLA 規格の標準値です。";
        ui.horizontal(|ui| {
            ui.label("同時打鍵閾値:").on_hover_text(threshold_hover);
            ui.add(
                egui::Slider::new(&mut self.config.general.simultaneous_threshold_ms, 10..=500)
                    .suffix(" ms"),
            )
            .on_hover_text(threshold_hover);
        });
        ui.label("出力方式: アプリごとに最適な注入方式を自動選択します（設定不要）")
            .on_hover_text(
                "フォアグラウンドのアプリ種別を判別し、VK送信・TSF送信等の\n最適な文字注入方式を自動的に切り替えます。手動設定は不要です。",
            );
        let mut auto_start_checked = self.config.general.auto_start == "enabled";
        if ui.checkbox(&mut auto_start_checked, "自動起動").on_hover_text("ONにすると: Windows ログオン時に自動的に awase を起動します。\nタスクスケジューラに登録されます。").changed() {
            self.config.general.auto_start = if auto_start_checked {
                "enabled"
            } else {
                "disabled"
            }
            .to_string();
        }
        ui.checkbox(&mut self.config.general.update_check, "更新を確認")
            .on_hover_text(
                "ONにすると: トレイを右クリックした際に awase.cc のサーバーへ最新バージョンを\n\
                 問い合わせ、新しい版があればメニューに表示します。バージョン・OS・端末固有の情報は\n\
                 送信しません（接続元IPアドレスはCloudflareの標準アクセスログに記録されます）。\n\
                 OFFにすると一切通信しません。"
            );
        let keyboard_model_hover = "物理キーボードの配列です。\n\
                 JIS: 無変換/変換キーが物理的に存在する日本語キーボード。\n\
                 US: ANSI 104キー配列。無変換/変換キーが無いため、\n\
                 親指キーとホットキーを別途 US 向けに変更する必要があります。\n\
                 切り替えると: 親指キー・ホットキーの既定値が自動的にJIS/US向けへ\n\
                 入れ替わります（下の説明参照）。";
        ui.horizontal(|ui| {
            ui.label("キーボード配列:")
                .on_hover_text(keyboard_model_hover);
            let prev_keyboard_model = self.config.general.keyboard_model;
            egui::ComboBox::from_id_salt("keyboard_model")
                .selected_text(keyboard_model_label(self.config.general.keyboard_model))
                .show_ui(ui, |ui| {
                    use awase::scanmap::KeyboardModel;
                    ui.selectable_value(
                        &mut self.config.general.keyboard_model,
                        KeyboardModel::Jis,
                        "JIS (日本語109キー)",
                    );
                    ui.selectable_value(
                        &mut self.config.general.keyboard_model,
                        KeyboardModel::Us,
                        "US (ANSI 104キー)",
                    );
                })
                .response
                .on_hover_text(keyboard_model_hover);
            // US → JIS への切替時、Space/Left Alt/Right Alt・独自ホットキー等
            // US 向けに変更していた設定が JIS では意味が変わる/使えないまま残る
            // （Space は単独タップの意味が変わり、Alt はなりすまし設定自体が
            // 無意味になる）のを防ぐため、親指キーとホットキー一式を
            // KeysConfig::default()/GeneralConfig::default() と同じ JIS 既定値へ
            // 強制的に戻す。ユーザーが手動で戻す手間を省く。
            //
            // `engine_off_solo_repeat` も既定値（VK_INSERT）へ強制的に揃える。
            // 既定値自体は JIS/US どちらの物理キーボードにも存在し配列非依存だが、
            // 「触らない」実装では config.toml に旧デフォルトの VK_NONCONVERT を
            // 明示保存済みの既存ユーザーが US へ切り替えたときにその値が残ってしまい、
            // US には無変換キーが無いためこの緊急停止機能が無反応のまま気付かれない
            // 回帰があった（2026-08-25 レビューで発見）。他のキー設定と同様、配列
            // 切替のたびに既定値へ揃えることでこの穴を塞ぐ。
            if prev_keyboard_model != awase::scanmap::KeyboardModel::Jis
                && self.config.general.keyboard_model == awase::scanmap::KeyboardModel::Jis
            {
                self.config.general.left_thumb_key = "無変換".to_string();
                self.config.general.right_thumb_key = "変換".to_string();
                self.config.keys.engine_on = vec!["Ctrl+Shift+変換".to_string()];
                self.config.keys.engine_off = vec!["Ctrl+Shift+無変換".to_string()];
                self.config.keys.ime_on = vec!["Ctrl+変換".to_string()];
                self.config.keys.ime_off = vec!["Ctrl+無変換".to_string()];
                self.config.keys.ime_toggle = vec!["VK_KANJI".to_string()];
                self.config.keys.engine_off_solo_repeat = Some("VK_INSERT".to_string());
            }
            // JIS → US への切替時、エンジンON/OFF・IME ON/OFF の既定値
            // （Ctrl+Shift+変換 等）は US に無変換/変換キーが物理的に存在しないため
            // 動作しない。動かない既定値を黙って残すより、未設定にして
            // 「キー設定」タブで明示的に選んでもらう方が誠実（他アプリのショートカット
            // と衝突しない US 向け「正解」の組み合わせを勝手に決め打ちできないため）。
            //
            // `engine_off_solo_repeat` だけは他と違い None にせず VK_INSERT へ
            // 揃える: 既定値の VK_INSERT は US キーボードにも物理的に存在し
            // そのまま動作するため、他のフィールドのように「動かない既定値」では
            // ない。無変換/変換を明示指定していた既存ユーザーの値をここで
            // VK_INSERT へ上書きするのは意図的（US では無変換/変換は動かないため、
            // 黙って残すより動く既定値へ揃える方が誠実）。
            if prev_keyboard_model != awase::scanmap::KeyboardModel::Us
                && self.config.general.keyboard_model == awase::scanmap::KeyboardModel::Us
            {
                self.config.keys.engine_on.clear();
                self.config.keys.engine_off.clear();
                self.config.keys.ime_on.clear();
                self.config.keys.ime_off.clear();
                self.config.keys.ime_toggle.clear();
                self.config.keys.engine_off_solo_repeat = Some("VK_INSERT".to_string());
            }
        });
        if self.config.general.keyboard_model == awase::scanmap::KeyboardModel::Us {
            ui.label(
                "  US 配列では既定の親指キー(無変換/変換)とホットキーが使えません。\n\
                 下の「レイアウト」で nicola_us.yab を選び、キー設定タブで\n\
                 親指キーを変更してください（Ctrl/Win は OS 予約修飾キーのため\n\
                 使用不可・同時打鍵検出自体が機能しません。プログラマブルキーボードで\n\
                 F13-F20 等へ物理リマップするか、Space を検討してください。\n\
                 キー設定タブの候補にある「Left Alt」「Right Alt」を選ぶと、\n\
                 親指シフト入力中のみ Alt キーを親指キーとして使えます）。\n\
                 \n\
                 親指シフト入力／ローマ字入力（上級者向け設定タブ）・IME ON/OFF（キー設定タブ）の\n\
                 ホットキーも未設定になっています（無変換/変換前提の既定値は US では\n\
                 動かないため）。動作する物理キーの組み合わせを設定してください。\n\
                 単独5連打で無効化（既定 Insert キー）は無変換/変換に依存しないため\n\
                 US でもそのまま動作します。",
            );
        }
        let layout_hover = "使用する配列定義ファイルを選びます。\n選ぶと: 「配列」タブの内容がこのファイルに切り替わります。\nlayout フォルダ内の .yab ファイルが表示されます。";
        ui.horizontal(|ui| {
            ui.label("レイアウト:").on_hover_text(layout_hover);
            let current = self
                .config
                .general
                .default_layout
                .trim_end_matches(".yab")
                .to_string();
            egui::ComboBox::from_id_salt("layout")
                .selected_text(&current)
                .show_ui(ui, |ui| {
                    for name in &self.available_layouts {
                        if ui.selectable_label(current == *name, name).clicked() {
                            self.config.general.default_layout = format!("{name}.yab");
                        }
                    }
                })
                .response
                .on_hover_text(layout_hover);
            if ui
                .button("再スキャン")
                .on_hover_text(
                    "押すと: layout フォルダを再読み込みし、上の選択肢一覧を更新します。",
                )
                .clicked()
            {
                self.available_layouts = scan_layout_names(&self.config.general.layouts_dir);
                self.recompute_diagnostics();
            }
        });
    }

    #[expect(clippy::too_many_lines)]
    fn tab_keys(&mut self, ui: &mut egui::Ui) {
        ui.heading("キー設定");
        ui.add_space(4.0);

        // Thumb keys
        ui.label("親指キー");
        let left_thumb_hover = "左の親指シフトキーに使うキーです。通常は「無変換」キーを使います。\n\
                 「Left Alt」を選ぶと、物理 Left Alt キーを親指シフト入力中に限り\n\
                 左親指キーとして使います（ローマ字入力中は通常の Alt として機能し、\n\
                 Alt+Tab 等を損ないません。PowerToys 等の OS レベルキーリマップと\n\
                 同様の効果を awase 単体で実現する機能です）。";
        ui.horizontal(|ui| {
            ui.label("  左親指:").on_hover_text(left_thumb_hover);
            thumb_key_combo(
                ui,
                "left_thumb_key",
                &mut self.config.general.left_thumb_key,
                left_thumb_hover,
            );
        });
        let right_thumb_hover = "右の親指シフトキーに使うキーです。通常は「変換」キーを使います。\n\
                 「Right Alt」を選ぶと、物理 Right Alt キーを親指シフト入力中に限り\n\
                 右親指キーとして使います（詳細は左親指のヒントを参照）。";
        ui.horizontal(|ui| {
            ui.label("  右親指:").on_hover_text(right_thumb_hover);
            thumb_key_combo(
                ui,
                "right_thumb_key",
                &mut self.config.general.right_thumb_key,
                right_thumb_hover,
            );
        });
        if self.config.general.left_thumb_key == "VK_SPACE"
            || self.config.general.right_thumb_key == "VK_SPACE"
        {
            ui.indent("space_thumb_options", |ui| {
                ui.checkbox(
                    &mut self.config.general.space_thumb_ignore_composing_guard,
                    "変換候補ウィンドウ表示中でも Space を送出する",
                )
                .on_hover_text(
                    "OFF にすると、無変換/変換キーと同様に変換候補ウィンドウ表示中は\n\
                     Space 単独タップを抑制します（IME の変換候補送り機能が使えなくなります）。\n\
                     通常は ON のままにしてください。",
                );
                ui.checkbox(
                    &mut self.config.general.space_thumb_shift_literal,
                    "Shift+Space は常に半角スペースとして送出する",
                )
                .on_hover_text(
                    "ON の場合、Shift を押しながら Space 親指キーを押すと、\n\
                     同時打鍵判定を待たず即座に半角スペースを入力します。",
                );
            });
        }
        if self.config.general.left_thumb_key == "VK_RETURN"
            || self.config.general.right_thumb_key == "VK_RETURN"
        {
            ui.indent("enter_thumb_options", |ui| {
                ui.checkbox(
                    &mut self.config.general.enter_thumb_ignore_composing_guard,
                    "変換候補ウィンドウ表示中でも Enter を送出する",
                )
                .on_hover_text(
                    "OFF にすると、無変換/変換キーと同様に変換候補ウィンドウ表示中は\n\
                     Enter 単独タップを抑制します（IME の変換確定機能が使えなくなります）。\n\
                     通常は ON のままにしてください。",
                );
                ui.checkbox(
                    &mut self.config.general.enter_thumb_shift_literal,
                    "Shift+Enter は常にソフト改行として送出する",
                )
                .on_hover_text(
                    "ON の場合、Shift を押しながら Enter 親指キーを押すと、\n\
                     同時打鍵判定を待たず即座に Shift+Enter を入力します。",
                );
            });
        }
        if is_muhenkan_thumb_key(&self.config.general.left_thumb_key)
            || is_muhenkan_thumb_key(&self.config.general.right_thumb_key)
        {
            ui.indent("muhenkan_thumb_options", |ui| {
                solo_tap_suppress_combo(
                    ui,
                    "無変換",
                    "MS-IME は無変換キー単独打鍵に既定で「かな切替」（IME オン相当）を\n\
                     割り当てているため、送出すると awase の管理外で IME モードが\n\
                     切り替わることがあります（2026-08-07 実機で確認）。",
                    &mut self.config.general.muhenkan_solo_tap_always_suppress,
                    &mut self.config.general.muhenkan_solo_tap_ignore_composing_guard,
                );
            });
        }
        // 2026-08-17 ユーザー判断: GJI 専用Fnキー変換（ADR-091 §D3.2）の
        // 手動設定 UI（専用Fnキードロップダウン・config1.dbへの書き込み
        // ボタン）は設定画面から撤去した。2026-09-02、実験的機能のまま
        // 撤去し忘れて出荷されていたGJI検出時のconfig1.db自動判定・
        // 起動時ポップアップ同意フロー（`gji_charset_autodetect.rs`の
        // 専用Fnキー部分・`gji_charset_popup.rs`/`gji_charset_write.rs`）も
        // 全撤去した（実機でGJIのキー設定が実際にはカスタムなのに
        // 「カスタム以外」と誤診断されるなど、ユーザーの混乱を招いていた）。
        // `muhenkan_solo_tap_dedicated_fn_key`（config.toml による手動設定）
        // の内部配線は残っており、上級者は引き続き手動で有効化できるが、
        // 現状これを設定画面から有効化する経路は無い。無変換キー単独タップ
        // のすぐ下に変換キー単独タップが並ぶよう、この節を撤去した分だけ
        // 表示順も詰まる。
        if is_henkan_thumb_key(&self.config.general.left_thumb_key)
            || is_henkan_thumb_key(&self.config.general.right_thumb_key)
        {
            ui.indent("henkan_thumb_options", |ui| {
                solo_tap_suppress_combo(
                    ui,
                    "変換",
                    "MS-IME は変換キー単独打鍵に既定で「再変換」を割り当てており、\n\
                     設定次第では IME オン相当の割当ても可能なため、送出すると\n\
                     awase の管理外で IME モードが切り替わることがあります。",
                    &mut self.config.general.henkan_solo_tap_always_suppress,
                    &mut self.config.general.henkan_solo_tap_ignore_composing_guard,
                );
            });
        }
        ui.add_space(8.0);

        // 2026-08-15 ユーザー判断: 「awase → IME ON/OFFキー」は単に
        // 「IME ON/OFFキー」へ改称。「IME → awase ON/OFFキー」（旧
        // `tab_ime_detect`）は既に GUI から撤去済みで対比する相手が無くなった
        // ため「awase → 」を残す意味が無い。「親指シフト入力／ローマ字入力」
        // （旧称「親指シフト ON/OFF」→「awase 有効/無効」）は2026-09-02に
        // 「上級者向け設定」タブへ移動済みのため、このタブでは本項目のみを扱う。
        ui.label("IME ON/OFFキー");
        combo_key_list_ui(
            ui,
            "IME ON",
            "ime_on",
            &mut self.config.keys.ime_on,
            &mut self.new_ime_on,
            "IME を ON にするキーの組み合わせです。\nIME がオフの状態からオンに切り替えます。",
        );
        combo_key_list_ui(
            ui,
            "IME OFF",
            "ime_off",
            &mut self.config.keys.ime_off,
            &mut self.new_ime_off,
            "IME を OFF にするキーの組み合わせです。\nIME がオンの状態からオフに切り替えます。",
        );
        combo_key_list_ui(
            ui,
            "IME ON/OFF トグル",
            "ime_toggle",
            &mut self.config.keys.ime_toggle,
            &mut self.new_ime_toggle,
            "IME の ON/OFF をトグルするキーの組み合わせです。\n現在の状態に応じて ON⇔OFF が切り替わります。",
        );
    }

    #[expect(clippy::too_many_lines)]
    fn tab_keymap(&mut self, ui: &mut egui::Ui) {
        ui.heading("ショートカット再割当");
        ui.label(
            "アプリ別にキー入力を別キーへ置き換えます。\n\
             例: Ctrl+I を F7 に再割当（vim 系で Tab と区別したい場合等）。\n\
             ※ 記号キーの表示は JIS 配列基準です（US 配列では別の文字に対応）。\n\
             ※ to 側で修飾キー付きの送信は現状未対応です。\n\
             ※ ⌨ ボタンを押した後にキーを押すと自動で設定されます（Esc で取消）。\n\
             ※ キャプチャは JIS 配列前提。`:` `@` `^` `_` や IME キーはドロップダウンから設定してください。",
        );
        ui.add_space(8.0);

        // local copy of capturing to avoid borrow-conflict with self.config.keymaps below
        let mut capturing = self.capturing;

        // Existing rules table
        ui.label("登録済みルール");
        if self.config.keymaps.is_empty() {
            ui.label("  （ルールはまだ登録されていません）");
        } else {
            let mut rm = None;
            for (i, rule) in self.config.keymaps.iter_mut().enumerate() {
                // horizontal_wrapped: ウィンドウ幅が狭いときは行内で折り返す（リフロー）。
                // 収まる幅では従来どおり1行表示。
                ui.horizontal_wrapped(|ui| {
                    // App field
                    let mut app_buf = rule.app.clone().unwrap_or_default();
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut app_buf)
                                .desired_width(120.0)
                                .hint_text("全アプリ"),
                        )
                        .on_hover_text("対象プロセス名（例: vim.exe）。空欄で全アプリ対象。")
                        .changed()
                    {
                        rule.app = if app_buf.is_empty() {
                            None
                        } else {
                            Some(app_buf)
                        };
                    }

                    // from: modifiers + main key + capture button
                    // Alt 修飾は GUI から選べない（ADR-114 決定5 — バックエンドが
                    // 'from' の Alt 修飾を禁止・skip するため、対称性を保つ）。
                    // 既存 config.toml に Alt 付きルールが手書きされていた場合、
                    // その alt 値はここでは変更されず素通しされる（バックエンドが
                    // 別途警告して skip する）。
                    let (mut ctrl, mut shift, alt, mut main) = parse_combo_str(&rule.from);
                    let mut changed = false;
                    changed |= ui.checkbox(&mut ctrl, "Ctrl").changed();
                    changed |= ui.checkbox(&mut shift, "Shift").changed();
                    if main_key_combo(
                        ui,
                        &format!("from_main_{i}"),
                        &mut main,
                        "変換元のキーです。左の Ctrl/Shift/Alt と組み合わせて判定します。",
                    ) {
                        changed = true;
                    }
                    let from_target = CaptureTarget::ExistingFrom(i);
                    capture_button(ui, &mut capturing, from_target);
                    if changed {
                        rule.from = format_combo(ctrl, shift, alt, &main);
                    }

                    ui.label("→");

                    // to: main key only + capture button
                    let mut to_main = rule.to.clone().unwrap_or_default();
                    if main_key_combo_optional(
                        ui,
                        &format!("to_main_{i}"),
                        &mut to_main,
                        "再注入するキー。「（消費のみ）」を選ぶとキーを消費するだけになります。",
                    ) {
                        rule.to = if to_main.is_empty() {
                            None
                        } else {
                            Some(to_main)
                        };
                    }
                    let to_target = CaptureTarget::ExistingTo(i);
                    capture_button(ui, &mut capturing, to_target);

                    if ui
                        .small_button("x")
                        .on_hover_text("押すと: このルールを削除します。")
                        .clicked()
                    {
                        rm = Some(i);
                    }
                });
            }
            if let Some(i) = rm {
                self.config.keymaps.remove(i);
            }
        }
        ui.add_space(12.0);

        // New rule form
        ui.label("新規追加");
        egui::Grid::new("keymap_new_grid")
            .num_columns(2)
            .show(ui, |ui| {
                ui.label("  アプリ:")
                    .on_hover_text("対象プロセス名（例: vim.exe）。空欄で全アプリ対象。");
                ui.add(
                    egui::TextEdit::singleline(&mut self.new_keymap_app)
                        .desired_width(180.0)
                        .hint_text("vim.exe など（空欄=全アプリ）"),
                )
                .on_hover_text("対象プロセス名（例: vim.exe）。空欄で全アプリ対象。");
                ui.end_row();

                let from_hover =
                    "変換元のキーです。左の Ctrl/Shift と組み合わせて判定します（Alt 修飾は使用できません）。";
                ui.label("  from:").on_hover_text(from_hover);
                ui.horizontal_wrapped(|ui| {
                    ui.checkbox(&mut self.new_keymap_from_ctrl, "Ctrl");
                    ui.checkbox(&mut self.new_keymap_from_shift, "Shift");
                    main_key_combo(
                        ui,
                        "new_from_main",
                        &mut self.new_keymap_from_main,
                        from_hover,
                    );
                    capture_button(ui, &mut capturing, CaptureTarget::NewFrom);
                })
                .response
                .on_hover_text(from_hover);
                ui.end_row();

                let to_hover =
                    "再注入するキー。「（消費のみ）」を選ぶとキーを消費するだけになります。";
                ui.label("  to:").on_hover_text(to_hover);
                ui.horizontal_wrapped(|ui| {
                    main_key_combo_optional(
                        ui,
                        "new_to_main",
                        &mut self.new_keymap_to_main,
                        to_hover,
                    );
                    capture_button(ui, &mut capturing, CaptureTarget::NewTo);
                })
                .response
                .on_hover_text(to_hover);
                ui.end_row();
            });
        self.capturing = capturing;
        if ui
            .button("+追加")
            .on_hover_text("押すと: 上で組み立てたルールを一覧に追加します。")
            .clicked()
            && !self.new_keymap_from_main.is_empty()
        {
            let from = format_combo(
                self.new_keymap_from_ctrl,
                self.new_keymap_from_shift,
                false, // Alt は使用できない（ADR-114 決定5）
                &self.new_keymap_from_main,
            );
            self.config.keymaps.push(awase::config::KeymapRule {
                app: if self.new_keymap_app.is_empty() {
                    None
                } else {
                    Some(self.new_keymap_app.clone())
                },
                from,
                to: if self.new_keymap_to_main.is_empty() {
                    None
                } else {
                    Some(self.new_keymap_to_main.clone())
                },
            });
            self.new_keymap_app.clear();
            self.new_keymap_from_ctrl = false;
            self.new_keymap_from_shift = false;
            self.new_keymap_from_main.clear();
            self.new_keymap_to_main.clear();
        }

        ui.add_space(16.0);
        self.scancode_map_section(ui);
    }

    /// Caps(英数)⇔Ctrl 入れ替えプリセット（ADR-111決定7）。Scancode Map
    /// （レジストリ、要昇格・要再起動）方式のみを提供する——ADR-110の
    /// フックベース`key_remap`機構はJIS英数キー位置で日本語IMEと衝突する
    /// 構造的リスクが判明したため撤回済み（`docs/adr/111-...md`参照）。
    fn scancode_map_section(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.heading("Caps(英数)⇔Ctrl 入れ替え");
        ui.label(
            "CapsLock（JISキーボードでは英数キー）と左Ctrlの役割を入れ替えます。\n\
             Windows のレジストリ（Scancode Map）を書き換えるため、管理者権限の\n\
             確認が1回表示されます。変更の反映には再起動が必要です（サインアウトでは\n\
             反映されないことがあります）。\n\
             この設定はこのPCの全ユーザーに影響します。リモートデスクトップ接続の\n\
             セッション内では動作しません。",
        );
        ui.add_space(4.0);

        if self.scancode_map_status.is_none() {
            self.scancode_map_status = Some(scancode_map_admin::read_status());
        }

        match &self.scancode_map_status {
            Some(scancode_map_admin::ScancodeMapStatus::Active { extra_entries }) => {
                if *extra_entries == 0 {
                    ui.colored_label(egui::Color32::from_rgb(0, 140, 0), "✓ 有効");
                } else {
                    ui.colored_label(
                        egui::Color32::from_rgb(0, 140, 0),
                        format!("✓ 有効（他に awase と無関係な設定が {extra_entries} 件あります）"),
                    );
                }
            }
            Some(scancode_map_admin::ScancodeMapStatus::Inactive { extra_entries }) => {
                if *extra_entries == 0 {
                    ui.label("未設定");
                } else {
                    ui.label(format!(
                        "未設定（awase と無関係な Scancode Map 設定が {extra_entries} 件あります）"
                    ));
                }
            }
            Some(scancode_map_admin::ScancodeMapStatus::ReadError(e)) => {
                ui.colored_label(egui::Color32::RED, format!("読み取りエラー: {e}"));
            }
            None => unreachable!("直前に read_status() で埋めている"),
        }

        ui.horizontal(|ui| {
            let is_active = matches!(
                self.scancode_map_status,
                Some(scancode_map_admin::ScancodeMapStatus::Active { .. })
            );
            if ui.button("有効にする").clicked() && !is_active {
                self.apply_scancode_map_change(true);
            }
            if ui.button("無効にする").clicked() && is_active {
                self.apply_scancode_map_change(false);
            }
        });

        if let Some(msg) = &self.scancode_map_last_message {
            ui.label(msg);
        }
    }

    /// 有効化/無効化ボタン押下時の処理。自己昇格フロー
    /// （`scancode_map_admin::request_elevated_change`）を起動して完了を
    /// 待ち、結果に応じてメッセージを表示し、状態キャッシュを読み直す
    /// （ADR-111決定4・決定7）。
    fn apply_scancode_map_change(&mut self, enable: bool) {
        use scancode_map_admin::ElevationOutcome;
        let outcome = scancode_map_admin::request_elevated_change(enable);
        self.scancode_map_last_message = Some(match outcome {
            ElevationOutcome::Success => {
                if enable {
                    "有効にしました。反映するには再起動してください。".to_string()
                } else {
                    "無効にしました。反映するには再起動してください。".to_string()
                }
            }
            ElevationOutcome::Failed => "処理に失敗しました。".to_string(),
            ElevationOutcome::Cancelled => {
                "キャンセルされました（管理者権限が必要です）。".to_string()
            }
            ElevationOutcome::LaunchError(e) => format!("起動できませんでした: {e}"),
        });
        // 決定7: 操作直後にのみ再読み込みする（毎フレーム読まない）。
        self.scancode_map_status = Some(scancode_map_admin::read_status());
    }

    /// 「アプリ無効化」タブ（`disable_apps`）。プロセス名のみで完結する単純な
    /// 設定のため、`tab_app_rules`（force_text/force_bypass/force_vk/force_tsf、
    /// プロセス名+クラス名の両方が必要でGUI化を見送っている）とは切り離して
    /// 常時表示する（2026-08-26、BUG-90: PowerToys Mouse Without Borders 使用中
    /// に物理「英数」キーが効かない不具合の回避策としてユーザーが自分で
    /// `disable_apps` に中継ウィンドウのプロセス名を追加できるようにする）。
    fn tab_disable_apps(&mut self, ui: &mut egui::Ui) {
        ui.heading("アプリを無効化");
        ui.label(
            "指定したアプリにフォーカスがある間、awase を丸ごと無効化します（force_bypass より強く、\n\
             フックレベルで生キーがそのままアプリへ届きます。DirectInput 等を使うゲームにも通用します）。\n\
             プロセス名のみで指定し、クラス名は不要です。大文字小文字・.exe の有無は区別しません。\n\
             無効化中は IME 制御（ON/OFF 自動切替・warmup 等）も完全に停止します。",
        );
        ui.add_space(4.0);
        process_list_ui(
            ui,
            "disable_apps",
            "awase を無効化するアプリ (disable_apps)",
            "フォーカス中このプロセスでは、awase のキー変換・IME 制御を一切行いません。\n既定でリモートデスクトップ接続（mstsc.exe）が登録されています\n（接続元での Ctrl キー押しっぱなし不具合対策）。",
            &mut self.config.app_overrides.disable_apps,
            &mut self.new_disable_app,
        );
    }

    #[expect(clippy::too_many_lines)]
    fn tab_app_rules(&mut self, ui: &mut egui::Ui) {
        ui.heading("アプリ別オーバーライド");
        ui.label(
            "特定アプリでの awase の挙動を上書きします。\n\
             プロセス名・クラス名は両方必須で、完全一致（大文字小文字は区別しない）です。\n\
             クラス名はログの [focus-sync] 行などで確認できます。",
        );
        ui.add_space(8.0);

        let [buf_text, buf_bypass, buf_vk, buf_tsf] = &mut self.new_override_bufs;
        override_list_ui(
            ui,
            "ov_text",
            "テキスト入力扱いを強制 (force_text)",
            "フォーカス分類を強制的に TextInput にします。\nNICOLA 変換が効かないアプリで有効にします。",
            &mut self.config.app_overrides.force_text,
            buf_text,
        );
        override_list_ui(
            ui,
            "ov_bypass",
            "素通しを強制 (force_bypass)",
            "フォーカス分類を強制的に NonText にし、全キーを変換せず OS に通します。\nゲーム等、awase を効かせたくないアプリで有効にします。",
            &mut self.config.app_overrides.force_bypass,
            buf_bypass,
        );
        override_list_ui(
            ui,
            "ov_vk",
            "VK 注入を強制 (force_vk)",
            "文字出力を VK Batched 方式（IME に composition させる）に強制します。",
            &mut self.config.app_overrides.force_vk,
            buf_vk,
        );
        override_list_ui(
            ui,
            "ov_tsf",
            "TSF 注入を強制 (force_tsf)",
            "文字出力を TSF Sequential 方式に強制します。\nWezTerm 等の TSF ネイティブアプリで使用します。",
            &mut self.config.app_overrides.force_tsf,
            buf_tsf,
        );

        ui.separator();
        ui.heading("プレフィックスキー素通し (post_bypass)");
        ui.label(
            "Ctrl+キー（tmux prefix 等）が素通しされた直後の次の1キーを\n\
             NICOLA 変換せずそのまま通します。\n\
             プロセス名・クラス名は部分一致で、空欄はすべてにマッチします。",
        );
        ui.add_space(4.0);
        let mut rm = None;
        for (i, rule) in self.config.post_bypass.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(format!(
                    "    {} / process={} / class={}",
                    rule.key,
                    if rule.process.is_empty() {
                        "(すべて)"
                    } else {
                        &rule.process
                    },
                    if rule.class.is_empty() {
                        "(すべて)"
                    } else {
                        &rule.class
                    },
                ));
                if ui
                    .small_button("x")
                    .on_hover_text("押すと: この行を削除します。")
                    .clicked()
                {
                    rm = Some(i);
                }
            });
        }
        if let Some(i) = rm {
            self.config.post_bypass.remove(i);
        }
        let pb_key_hover = "Ctrl+このキーが素通しされた直後の次の1キーを NICOLA 変換せず\nそのまま通します（tmux の Ctrl+B 等の prefix キー用）。";
        ui.horizontal(|ui| {
            ui.label("Ctrl+").on_hover_text(pb_key_hover);
            main_key_combo(ui, "new_pb_key", &mut self.new_pb_key, pb_key_hover);
            ui.add(
                egui::TextEdit::singleline(&mut self.new_pb_process)
                    .desired_width(120.0)
                    .hint_text("プロセス名 (部分一致)"),
            )
            .on_hover_text("対象プロセス名（部分一致）。空欄はすべてのプロセスにマッチします。");
            ui.add(
                egui::TextEdit::singleline(&mut self.new_pb_class)
                    .desired_width(120.0)
                    .hint_text("クラス名 (部分一致)"),
            )
            .on_hover_text("対象ウィンドウのクラス名（部分一致）。空欄はすべてにマッチします。");
            if ui
                .button("+追加")
                .on_hover_text("押すと: 上で組み立てたルールを一覧に追加します。")
                .clicked()
                && !self.new_pb_key.is_empty()
            {
                // ランタイムの parse は "Ctrl+<キー>" 形式（Ctrl 必須）を要求する
                self.config.post_bypass.push(awase::config::PostBypassRule {
                    key: format_combo(true, false, false, &std::mem::take(&mut self.new_pb_key)),
                    process: std::mem::take(&mut self.new_pb_process),
                    class: std::mem::take(&mut self.new_pb_class),
                });
            }
        });
    }

    #[expect(clippy::too_many_lines)]
    fn tab_layout(&mut self, ui: &mut egui::Ui) {
        // 実機で「編集タブを開くと黒い画面になり、[layout-tab] ログが一切
        // 出ない」と報告された。ログが出ないのは、途中でハング/クラッシュして
        // バッファが flush される前に消えている可能性があるため、各チェック
        // ポイントで即 flush するログに切り替える（log_checkpoint 参照）。
        let first_open = !self.layout_loaded;
        let frame_start = std::time::Instant::now();

        if first_open {
            log_checkpoint("tab_layout 開始（ensure_layout_loaded 呼び出し前）");
        }
        self.ensure_layout_loaded();
        self.drain_layout_pending_async();
        if first_open {
            log_checkpoint("ensure_layout_loaded 完了、ウィジェット構築開始");
        }

        ui.heading("配列編集");
        ui.add_space(4.0);

        // ツールバー
        ui.horizontal(|ui| {
            if ui
                .button("開く")
                .on_hover_text(
                    "押すと: ファイル選択ダイアログを開いて別の .yab ファイルを読み込みます。",
                )
                .clicked()
            {
                self.layout_do_open_dialog();
            }
            if ui
                .button("保存")
                .on_hover_text("押すと: 現在編集中の内容を今開いているファイルへ上書き保存します。")
                .clicked()
            {
                self.layout_do_save();
            }
            if ui
                .button("名前を付けて保存")
                .on_hover_text("押すと: 保存先を選ぶダイアログを開き、別名/別の場所へ保存します。")
                .clicked()
            {
                self.layout_do_save_as_dialog();
            }
            if ui
                .button("再読み込み")
                .on_hover_text(
                    "押すと: ディスク上のファイルを読み直します（未保存の編集は失われます）。",
                )
                .clicked()
            {
                self.layout_do_reload();
            }
        });
        ui.horizontal(|ui| {
            ui.label("パス:").on_hover_text(
                "開いている .yab ファイルのパスです。書き換えて Enter を押すと\nそのパスのファイルを開きます。",
            );
            let resp = ui
                .add(
                    egui::TextEdit::singleline(&mut self.layout_file_path_buf)
                        .desired_width(300.0),
                )
                .on_hover_text("Enter を押すと: このパスのファイルを開きます。");
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                self.layout_do_open_from_text_box();
            }
        });
        ui.horizontal(|ui| {
            let fname = self
                .layout_file_path
                .as_ref()
                .and_then(|p| p.file_name())
                .map_or("-", |n| n.to_str().unwrap_or("-"));
            ui.label(fname);
            ui.separator();
            ui.label(if self.layout_modified {
                egui::RichText::new("変更あり").color(egui::Color32::from_rgb(200, 80, 0))
            } else {
                egui::RichText::new("保存済み").color(egui::Color32::from_rgb(0, 140, 0))
            });
            ui.separator();
            ui.label(keyboard_model_label(self.config.general.keyboard_model))
                .on_hover_text(
                    "配列のキーボード配列（JIS/US）は「全般設定」タブの設定に従います。",
                );
            if !self.layout_status.is_empty() {
                ui.separator();
                ui.label(&self.layout_status);
            }
        });
        ui.add_space(8.0);

        // 面タブ
        //
        // LeftThumbShift/RightThumbShift（小指左親指シフト/小指右親指シフト、
        // ADR-097）はUIタブから一時的に隠す。実機確認で、この面に何も
        // 割り当てていない状態だと親指+小指+文字キーの同時打鍵でアルファベットが
        // そのまま出力されてしまうことが分かり、既定でリリースできる完成度に
        // 達していないと判断した。Face enum・YabLayout フィールド・.yab の
        // パース/シリアライズ・エンジンの面解決ロジックは維持する
        // （.yab を直接編集すればこれまで通り機能する。UI から選べなくする
        // だけ）。ADR-097 の既定配列（やまぶきR互換の割り当て）が確定したら
        // このフィルタを外す。
        ui.horizontal(|ui| {
            for (face, label) in FACES
                .iter()
                .filter(|(f, _)| !matches!(f, Face::LeftThumbShift | Face::RightThumbShift))
            {
                let is_active = self.layout_current_face == *face;
                let btn_text = if is_active {
                    egui::RichText::new(*label).strong()
                } else {
                    egui::RichText::new(*label)
                };
                if ui
                    .selectable_label(is_active, btn_text)
                    .on_hover_text("クリックすると: このシフト面のキー配列に切り替えて表示します。")
                    .clicked()
                {
                    self.layout_current_face = *face;
                    self.layout_selected_pos = None;
                }
            }
        });
        ui.separator();

        // 凡例
        ui.horizontal(|ui| {
            ui.label("凡例:");
            color_legend(ui, egui::Color32::from_rgb(255, 255, 255), "打鍵(ローマ字)");
            color_legend(ui, egui::Color32::from_rgb(210, 230, 255), "リテラル");
            color_legend(ui, egui::Color32::from_rgb(210, 255, 220), "特殊キー");
            color_legend(
                ui,
                egui::Color32::from_rgb(200, 235, 255),
                "打鍵(記号/数字)",
            );
            color_legend(ui, egui::Color32::from_rgb(220, 220, 220), "なし");
        });
        ui.add_space(8.0);

        if first_open {
            log_checkpoint("ツールバー/タブ/凡例 描画完了、グリッド描画開始");
        }
        // ScrollArea::vertical()（縦のみ）ではなく ::both() を使う。理由:
        // 縦専用スクロール領域は横方向を単に available 幅へ clip するだけで
        // 自前の横スクロールバーを持たず、しかも egui 0.31.1 の ScrollArea は
        // 常に content_max_size を自身の可視サイズに収める（scroll_area.rs の
        // `if true {}` 分岐、`content_max_size[d] = f32::INFINITY` はデッド
        // コード）ため、外側の egui::ScrollArea::both()（tab_layout の呼び出し元、
        // Main content の横スクロール = 最終フォールバック、7207413c 参照）も
        // キーボード図の実際の幅を検知できず、横スクロールバーが出ないまま
        // 右端が切れていた（ウィンドウを既定幅より手動で縮めると発生。#156
        // code review 1周目の指摘）。
        //
        // かといって ScrollArea を丸ごと外すと、ツールバー/保存・開くボタン/
        // 面タブ/凡例（このスコープの外、上で描画済み）まで grid+編集パネルと
        // 一緒に縦スクロールしてしまい、編集パネルが伸びた状態で下へスクロール
        // すると保存ボタンや面タブが画面外に流れて操作しづらくなる（#156
        // code review 2周目の指摘）。そこでこの内側スコープだけ独立した
        // スクロール領域として残しつつ、::both() にして自前の横スクロール
        // バーを持たせることで、上のツールバー類は固定したまま
        // grid+編集パネル側の横オーバーフローにも到達できるようにする。
        egui::ScrollArea::both().show(ui, |ui| {
            self.draw_layout_keyboard_grid(ui);
            if first_open {
                log_checkpoint("グリッド描画完了、編集パネル描画開始");
            }
            ui.add_space(8.0);
            ui.separator();
            self.draw_layout_edit_panel(ui);
        });

        if first_open {
            log_checkpoint(&format!(
                "初回描画完了（読み込み+全ウィジェット構築）: 合計 {}ms",
                frame_start.elapsed().as_millis()
            ));
        }
    }

    fn draw_layout_keyboard_grid(&mut self, ui: &mut egui::Ui) {
        let row_sizes = self.config.general.keyboard_model.row_sizes();
        let mut clicked_pos = None;

        // Row indents to simulate staggered keyboard layout
        let indents: [f32; 4] = [0.0, 14.0, 28.0, 42.0];

        for (row, &cols) in row_sizes.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.add_space(indents[row]);
                for col in 0..cols {
                    #[expect(clippy::cast_possible_truncation)]
                    let pos = PhysicalPos::new(row as u8, col as u8);
                    let value = self.layout_face(self.layout_current_face).get(&pos);
                    let is_selected = self.layout_selected_pos == Some(pos);

                    let display = cell_display(value);
                    let bg_color = cell_color(value);
                    let stroke = if is_selected {
                        egui::Stroke::new(2.5_f32, egui::Color32::from_rgb(30, 100, 220))
                    } else {
                        egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(160, 160, 160))
                    };

                    let tip = cell_tooltip(value, pos);
                    let btn =
                        egui::Button::new(egui::RichText::new(display).monospace().size(14.0))
                            .fill(bg_color)
                            .stroke(stroke)
                            .min_size(egui::vec2(40.0, 34.0));

                    if ui.add(btn).on_hover_text(tip).clicked() {
                        clicked_pos = Some(pos);
                    }
                }
            });
        }

        if let Some(pos) = clicked_pos {
            self.select_layout_cell(pos);
        }
    }

    #[expect(clippy::too_many_lines)]
    fn draw_layout_edit_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("編集パネル");
        let Some(pos) = self.layout_selected_pos else {
            ui.label(
                egui::RichText::new("キーボードグリッドのセルをクリックして選択してください")
                    .italics()
                    .color(egui::Color32::GRAY),
            );
            return;
        };

        // Position display
        let pos_label = format!(
            "位置: 行 {} 列 {}  (row={}, col={})",
            pos.row + 1,
            pos.col + 1,
            pos.row,
            pos.col
        );
        ui.label(egui::RichText::new(pos_label).strong());
        ui.add_space(4.0);

        // コピー履歴: 「コピー」を押すたびに選択中セルの値が履歴の先頭に
        // 積まれる。履歴の項目をクリックすると選択中セルへ直接貼り付ける。
        ui.horizontal(|ui| {
            if ui
                .button("コピー")
                .on_hover_text("選択中のセルの値を履歴に追加します。")
                .clicked()
            {
                self.copy_layout_cell();
            }
            ui.label("履歴（クリックで貼り付け）:");
            if self.layout_clipboard_history.is_empty() {
                ui.label(
                    egui::RichText::new("(空)")
                        .small()
                        .color(egui::Color32::GRAY),
                );
            }
        });
        if !self.layout_clipboard_history.is_empty() {
            // 狭いウィンドウ幅でもボタン列が画面外に切れないよう折り返す
            // （他タブと同様のリフロー対応）。
            ui.horizontal_wrapped(|ui| {
                for value in self.layout_clipboard_history.clone() {
                    let label = cell_display(Some(&value));
                    let tip = value_description(Some(&value));
                    if ui.button(label).on_hover_text(tip).clicked() {
                        self.paste_layout_cell(value);
                    }
                }
            });
        }
        ui.add_space(4.0);

        // Type selector (radio buttons)
        ui.horizontal(|ui| {
            ui.label("種別:");
            ui.radio_value(&mut self.layout_edit_kind, ValueKind::Keystroke, "打鍵")
                .on_hover_text(
                    "ローマ字（複数文字、例: 「si」「tsu」）またはキーボード上の\n\
                     記号・数字（例: 「!」「1」）を、実際のキー押下として送信し、\n\
                     IME に処理させます。\n\
                     \n\
                     アルファベットのみならローマ字入力として（かな変換テーブルを\n\
                     引いてかな文字を出力）、それ以外（記号・数字）はキーシーケンス\n\
                     として扱われ、結果は今の IME の変換モードに依存します。\n\
                     \n\
                     JIS キーボード上に存在する文字（半角の英数字・記号）のみ\n\
                     入力できます。全角で入力しても自動で半角に変換されるので、\n\
                     半角/全角を意識する必要はありません。",
                );
            ui.radio_value(&mut self.layout_edit_kind, ValueKind::Literal, "リテラル")
                .on_hover_text(
                    "指定した文字列を Unicode 文字としてそのまま直接送信します\n\
                     （IME を一切経由しません）。IME や変換モードに関係なく必ず\n\
                     その文字が出ます。「ー」「…」のような固定記号に向いています。",
                );
            ui.radio_value(&mut self.layout_edit_kind, ValueKind::Special, "特殊キー")
                .on_hover_text(
                    "Backspace / Escape / Enter / Space / Delete / Insert / \n\
                     矢印 / Home / End / PageUp / PageDown を送信します。",
                );
            ui.radio_value(&mut self.layout_edit_kind, ValueKind::Vk, "VKコード")
                .on_hover_text(
                    "仮想キーコードを16進数で直接指定します（やまぶきR互換の\n\
                     「V」+16進数指定）。特殊キーに無いキーを送りたい場合に使います。",
                );
            ui.radio_value(&mut self.layout_edit_kind, ValueKind::None, "なし")
                .on_hover_text("このキーへの割り当てを解除します（パススルー）。");
        });
        ui.add_space(4.0);

        // Value input
        match self.layout_edit_kind {
            ValueKind::Keystroke => {
                ui.horizontal(|ui| {
                    ui.label("打鍵:").on_hover_text(
                        "ローマ字（例: ka, si, tsu）または半角記号/数字（例: !, 1）を入力します。",
                    );
                    let resp = ui
                        .add(
                            egui::TextEdit::singleline(&mut self.layout_edit_value)
                                .desired_width(120.0)
                                .hint_text("例: ka, si, tsu, !, 1"),
                        )
                        .on_hover_text(
                            "ローマ字（例: ka, si, tsu）または半角記号/数字（例: !, 1）を入力します。",
                        );
                    if resp.changed() {
                        self.layout_edit_value = normalize_keystroke_input(&self.layout_edit_value);
                    }
                });
                let trimmed = self.layout_edit_value.trim();
                if let Some(bad) = find_invalid_keystroke_char(trimmed) {
                    ui.colored_label(
                        egui::Color32::RED,
                        format!("「{bad}」は JIS キーボード上のキーとして入力できません"),
                    );
                } else if !trimmed.is_empty() && trimmed.chars().all(|c| c.is_ascii_alphabetic()) {
                    let preview: String = self
                        .kana_table
                        .kana_for_romaji(trimmed)
                        .map_or_else(|| "（未対応）".to_string(), |c| c.to_string());
                    ui.horizontal(|ui| {
                        ui.label("かな変換:");
                        ui.label(
                            egui::RichText::new(&preview)
                                .size(22.0)
                                .strong()
                                .color(egui::Color32::from_rgb(0, 80, 160)),
                        );
                    });
                } else if !trimmed.is_empty() {
                    ui.label(
                        egui::RichText::new("※ 記号/数字のキーシーケンスとして IME に処理させます")
                            .small()
                            .color(egui::Color32::GRAY),
                    );
                }
            }
            ValueKind::Literal => {
                ui.horizontal(|ui| {
                    ui.label("文字:")
                        .on_hover_text("このキーを押したときに直接送信する文字列です。");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.layout_edit_value)
                            .desired_width(120.0)
                            .hint_text("例: ー、…"),
                    )
                    .on_hover_text("このキーを押したときに直接送信する文字列です。");
                });
                ui.label(
                    egui::RichText::new("※ Unicode 文字をそのまま送信します")
                        .small()
                        .color(egui::Color32::GRAY),
                );
            }
            ValueKind::Special => {
                let special_hover = "このキーを押したときに送信する特殊キーを選びます。";
                ui.horizontal(|ui| {
                    ui.label("特殊キー:").on_hover_text(special_hover);
                    egui::ComboBox::from_id_salt("special_key")
                        .selected_text(SPECIAL_KEYS[self.layout_edit_special_idx].1)
                        .show_ui(ui, |ui| {
                            for (i, (_, name)) in SPECIAL_KEYS.iter().enumerate() {
                                ui.selectable_value(&mut self.layout_edit_special_idx, i, *name);
                            }
                        })
                        .response
                        .on_hover_text(special_hover);
                });
            }
            ValueKind::Vk => {
                ui.horizontal(|ui| {
                    ui.label("VKコード (16進):")
                        .on_hover_text("送信する仮想キーコードを16進数で指定します（例: 1D）。");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.layout_edit_value)
                            .desired_width(80.0)
                            .hint_text("例: 1D"),
                    )
                    .on_hover_text("送信する仮想キーコードを16進数で指定します（例: 1D）。");
                });
                if !self.layout_edit_value.trim().is_empty()
                    && u16::from_str_radix(
                        self.layout_edit_value.trim().trim_start_matches('V'),
                        16,
                    )
                    .is_err()
                {
                    ui.colored_label(egui::Color32::RED, "16進数として解釈できません");
                }
            }
            ValueKind::None => {
                ui.label(
                    egui::RichText::new("このキーへの割り当てを解除します")
                        .color(egui::Color32::GRAY),
                );
            }
        }

        let can_apply = (self.layout_edit_kind != ValueKind::Keystroke
            || find_invalid_keystroke_char(self.layout_edit_value.trim()).is_none())
            && (self.layout_edit_kind != ValueKind::Vk
                || self.layout_edit_value.trim().is_empty()
                || u16::from_str_radix(self.layout_edit_value.trim().trim_start_matches('V'), 16)
                    .is_ok());

        ui.add_space(6.0);
        if ui
            .add_enabled(
                can_apply,
                egui::Button::new(egui::RichText::new("適用").strong()),
            )
            .on_hover_text(
                "押すと: 上で入力した内容を選択中のセルへ反映します（ファイルへの\n保存は別途「保存」ボタンで行います）。",
            )
            .clicked()
        {
            self.apply_layout_edit();
        }
    }

    #[expect(clippy::too_many_lines)]
    fn tab_advanced(&mut self, ui: &mut egui::Ui) {
        ui.heading("上級者向け設定");
        ui.add_space(4.0);

        // Engine on/off
        // 2026-09-02 ユーザー判断: 旧称「エンジン ON/OFF」→「親指シフト
        // ON/OFF」は、隣接する「IME ON/OFF」（Windows側のIME状態）と紛らわしい
        // という指摘を受け「awase 有効/無効」へ改称。かつ日常的に触る設定
        // ではないため「キー設定」タブから本タブへ移動した。
        // 2026-09-03 ユーザー判断: 「awase 有効/無効」もエンジンという実装用語が
        // 残り分かりにくいという指摘を受け、状態そのものを名付ける「親指シフト
        // 入力／ローマ字入力」へ改称。ON/OFF表記をやめたことで「IME ON/OFF」との
        // 混同も避けられる。
        let awase_enable_hover = "ローマ字入力にすると、キー入力の変換（親指シフト同時打鍵判定・\n\
             ローマ字→かな変換）を一切行わず、すべてのキーをそのまま素通しします。\n\
             Windows 側の IME の ON/OFF 状態には影響しません（別項目の「IME ON/OFF」参照）。";
        ui.label("親指シフト入力／ローマ字入力")
            .on_hover_text(awase_enable_hover);
        combo_key_list_ui(
            ui,
            "親指シフト入力にする",
            "eng_on",
            &mut self.config.keys.engine_on,
            &mut self.new_engine_on,
            "親指シフト入力に切り替えるキーの組み合わせです。\n複数登録できます。",
        );
        combo_key_list_ui(
            ui,
            "ローマ字入力にする",
            "eng_off",
            &mut self.config.keys.engine_off,
            &mut self.new_engine_off,
            "ローマ字入力に切り替えるキーの組み合わせです。\n複数登録できます。",
        );
        let solo_repeat_hover = "指定キーを単独で素早く5回連続押下するとローマ字入力に切り替えます。\nCtrl スタック等で通常のキー操作が効かなくなった際の緊急脱出用です。";
        ui.horizontal(|ui| {
            ui.label("  単独5連打でローマ字入力にする:")
                .on_hover_text(solo_repeat_hover);
            solo_repeat_combo(
                ui,
                &mut self.config.keys.engine_off_solo_repeat,
                solo_repeat_hover,
            );
        });
        ui.add_space(8.0);

        // Toggle hotkey
        ui.label("親指シフト入力／ローマ字入力 切替")
            .on_hover_text(awase_enable_hover);
        let engine_toggle_hover =
            "親指シフト入力とローマ字入力をトグルするホットキーです。\nシステム全体で有効です。";
        ui.horizontal(|ui| {
            ui.label("  切替:").on_hover_text(engine_toggle_hover);
            hotkey_combo_ui(
                ui,
                "engine_toggle_hotkey",
                &mut self.config.general.engine_toggle_hotkey,
                engine_toggle_hover,
            );
        });
        ui.add_space(8.0);

        // confirm_mode / speculative_delay_ms は設定画面から完全に非表示にした
        // （2026-08-30、ユーザー判断: 「wait 単一表示というか設定UIから見えなく
        // したらいい」）。ConfirmMode のバリアント・`dispatch_confirm_mode` の
        // 分岐ロジックは残してあり、`config.toml` に `confirm_mode = "two_phase"`
        // 等と手書きすれば引き続き使える純粋な toml 裏設定になった。

        let slider_with_tip = |ui: &mut egui::Ui,
                               label: &str,
                               tip: &str,
                               suffix: &str,
                               val: &mut u32,
                               range: std::ops::RangeInclusive<u32>| {
            ui.horizontal(|ui| {
                ui.label(label).on_hover_text(tip);
                ui.add(egui::Slider::new(val, range).suffix(suffix))
                    .on_hover_text(tip);
            });
        };
        slider_with_tip(
            ui,
            "3キー分岐マージン:",
            "char1→親指→char2 と3キーが来た場合の仲裁マージンです。\n\
             2つの間隔の差がこの割合を超えるとタイミングだけで決定し、\n\
             それ以外は n-gram で判定します（下の n-gram 設定を参照）。",
            " %",
            &mut self.config.general.timing_margin_percent,
            0..=100,
        );
        slider_with_tip(
            ui,
            "重なり不足マージン:",
            "文字キー+親指キーの物理的な重なり時間がこの割合未満だと\n\
             「重なり不足」とみなし、n-gram でタイブレークします\n\
             （n-gram が無効なら単独打鍵扱いになります）。",
            " %",
            &mut self.config.general.min_overlap_margin_percent,
            0..=100,
        );
        ui.add_space(8.0);

        ui.checkbox(
            &mut self.config.general.swallow_alt_kana_input_method_switch,
            "Alt+かな による IME 入力方式切替（ローマ字⇔JIS かな）を無効化する",
        )
        .on_hover_text(
            "ON(既定)の場合、物理 Alt を押しながら「かな」キーを押しても、\n\
             MS-IME の「ローマ字入力 ⇔ JIS かな直接入力」切替ショートカットが\n\
             発動しないようにブロックします。\n\
             JIS かな直接入力に切り替わると、awase が送出するローマ字綴りの\n\
             キー列が誤読され、一度切り替わると awase 側からは元に戻せません\n\
             （Windows にこの入力方式を外部から切り替える公式 API が無いため）。\n\
             JIS かな直接入力を意図的に使いたい場合（= awase をローマ字入力に\n\
             して使う場合など）のみ OFF にしてください。",
        );
        ui.add_space(4.0);
        half_width_alnum_toggle_checkbox(ui, &mut self.config.general.half_width_alnum_toggle);
        ui.add_space(8.0);
        // n-gram はタイブレーク（3キー分岐・重なり不足判定・2キーしきい値の
        // 動的調整）に confirm_mode を問わず常に使われる（ngram_file が
        // ロードできていれば）。かつては「confirm_mode が n-gram 予測の
        // ときだけ使う」という誤った前提でグレーアウトしていたが、
        // 実際にはタイブレーク経路は wait を含む全モードで有効なため
        // 2026-08-30 に外した。
        let ngram_file_hover = "n-gram 統計データファイルのパスです。\n.csv.gz または .toml 形式に対応しています。\n同時打鍵のタイブレーク（3キー分岐・重なり不足判定）に\nconfirm_mode を問わず常に使われます。";
        ui.horizontal(|ui| {
            ui.label("n-gram ファイル:").on_hover_text(ngram_file_hover);
            let mut buf = self.config.general.ngram_file.clone().unwrap_or_default();
            if ui
                .text_edit_singleline(&mut buf)
                .on_hover_text(ngram_file_hover)
                .changed()
            {
                self.config.general.ngram_file = if buf.is_empty() { None } else { Some(buf) };
            }
        });
        slider_with_tip(
            ui,
            "n-gram 調整幅:",
            "n-gram による同時打鍵しきい値調整の幅です。\n大きいほど予測の影響が強くなります。",
            " ms",
            &mut self.config.general.ngram_adjustment_range_ms,
            0..=100,
        );
        slider_with_tip(
            ui,
            "n-gram 最小閾値:",
            "n-gram で調整される同時打鍵しきい値の下限です。\nこれより短い閾値にはなりません。",
            " ms",
            &mut self.config.general.ngram_min_threshold_ms,
            10..=200,
        );
        slider_with_tip(
            ui,
            "n-gram 最大閾値:",
            "n-gram で調整される同時打鍵しきい値の上限です。\nこれより長い閾値にはなりません。",
            " ms",
            &mut self.config.general.ngram_max_threshold_ms,
            50..=500,
        );
        ui.add_space(8.0);
        slider_with_tip(
            ui,
            "フォーカスデバウンス:",
            "フォーカス切り替え時のデバウンス時間です。\nAlt+Tab などでフォーカスが連続変更される際の誤検知を防ぎます。",
            " ms",
            &mut self.config.general.focus_debounce_ms,
            0..=200,
        );
        slider_with_tip(
            ui,
            "IME ポーリング間隔:",
            "IME 状態のポーリング間隔です。\nマウスで言語バーを操作した場合などの検出用です。\n小さいほどレスポンスが良くなりますが、CPU 負荷が増えます。",
            " ms",
            &mut self.config.general.ime_poll_interval_ms,
            100..=5000,
        );
        ui.horizontal(|ui| {
            let layouts_dir_hover = "配列定義ファイル (.yab) を格納するフォルダです。";
            ui.label("レイアウトディレクトリ:")
                .on_hover_text(layouts_dir_hover);
            ui.text_edit_singleline(&mut self.config.general.layouts_dir)
                .on_hover_text(layouts_dir_hover);
        });
    }
}

// ── eframe::App ──

impl eframe::App for SettingsApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_pending_save(ctx);
        self.handle_layout_shortcuts(ctx);

        // 複数ディスプレイ対応: DPI スケールの異なるモニタへ移動すると、
        // WM_DPICHANGED 後のウィンドウサイズが移動先モニタに収まらず
        // 下部が画面外に出て操作不能になることがある。現在のモニタサイズを
        // 超えていたら収まるサイズへ自動クランプする（収まった後は発火しない）。
        let clamp = ctx.input(|i| {
            let vp = i.viewport();
            match (vp.monitor_size, vp.inner_rect) {
                (Some(monitor), Some(inner)) if monitor.x > 0.0 && monitor.y > 0.0 => {
                    let max = monitor * 0.95; // タイトルバー・タスクバー分の余白
                    let size = inner.size();
                    (size.x > max.x || size.y > max.y).then(|| size.min(max))
                }
                _ => None,
            }
        });
        if let Some(new_size) = clamp {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(new_size));
        }

        // Keymap capture: drain key events while capturing
        if self.capturing.is_some() {
            self.process_keymap_capture(ctx);
            ctx.request_repaint();
        }

        self.config_path_panel(ctx);
        self.show_dangerous_save_confirm_modal(ctx);

        // Side panel for tab selection
        egui::SidePanel::left("tab_panel")
            .resizable(false)
            .default_width(100.0)
            .show(ctx, |ui| {
                ui.add_space(8.0);
                // 「アプリ別」(AppRules) は高度な機能（force_text/force_bypass/
                // force_vk/force_tsf、プロセス名+クラス名の両方が必要）のため
                // GUI 化を見送り、config.toml の直接編集に委ねている。
                // tab_app_rules の実装自体は残してある。disable_apps 部分だけは
                // プロセス名のみで完結する単純な設定のため、2026-08-26（BUG-90）
                // に「アプリ無効化」タブとして切り出して表示するようにした。
                //
                // 「配列編集」(Layout) は 2026-07-06 に「配列プレビューの実装が
                // まだ固まっていない」として一旦非表示にしていたが、layouts_dir の
                // パス解決バグ修正を経て再表示した。その後、独立バイナリだった
                // awase-yab-editor を統合し、プレビューではなく実際に編集できる
                // タブにした（バイナリを分ける価値は無いという判断）。
                // タブ順序は使用頻度順（2026-08-26 見直し）: 全般設定・キー設定・
                // 配列編集・上級者向け設定を先に置き、日常的に触らない「アプリ無効化」
                // 「ショートカット」を末尾にまとめる。
                for (tab, label) in [
                    (Tab::Basic, "全般設定"),
                    (Tab::Keys, "キー設定"),
                    (Tab::Layout, "配列編集"),
                    (Tab::Advanced, "上級者向け設定"),
                    (Tab::DisableApps, "アプリ無効化"),
                    (Tab::Keymap, "ショートカット"),
                ] {
                    if ui.selectable_label(self.active_tab == tab, label).clicked() {
                        self.active_tab = tab;
                    }
                }
            });

        // 適用/キャンセルは常時表示の下部パネルに置く。
        // スクロール領域の下に直置きすると、ウィンドウが縦に伸び切った状態や
        // 画面外にはみ出した状態でボタンに到達できなくなるため
        // （複数ディスプレイの DPI 遷移で実発生）。
        egui::TopBottomPanel::bottom("action_panel").show(ctx, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("適用").clicked() {
                    self.apply();
                }
                if ui.button("キャンセル").clicked() {
                    self.cancel();
                }
            });
            // ステータス（バリデーション警告等）はボタン行とは別の行に出す。
            // ボタンと同じ ui.horizontal() 内に置くと、長い警告文が折り返さず
            // 右側に切れて見えなくなるため（複数警告を "; " 連結すると
            // 数百文字になり得る）。折り返しを明示指定し、ウィンドウ幅いっぱいまで
            // 使って複数行に自然に折り返させる。
            if !self.status.is_empty() {
                ui.add_space(4.0);
                ui.add(egui::Label::new(&self.status).wrap());
            }
            ui.add_space(6.0);
        });

        // Main content（残り領域全体を縦横スクロール可能に）。
        // 横スクロールも有効にすることで、ウィンドウ幅が狭くても keymap 行や
        // プレビューのキーボード図の右端に到達できる（どんなサイズでも全項目操作可能）。
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::both()
                .auto_shrink([false; 2])
                .show(ui, |ui| match self.active_tab {
                    Tab::Basic => self.tab_basic(ui),
                    Tab::Keys => self.tab_keys(ui),
                    Tab::Keymap => self.tab_keymap(ui),
                    Tab::DisableApps => self.tab_disable_apps(ui),
                    Tab::AppRules => self.tab_app_rules(ui),
                    Tab::Layout => self.tab_layout(ui),
                    Tab::Advanced => self.tab_advanced(ui),
                });
        });
    }
}

// ── Reusable UI helpers ──

/// アプリ別オーバーライド1カテゴリ分のリスト UI（完全一致・両フィールド必須）。
fn override_list_ui(
    ui: &mut egui::Ui,
    id: &str,
    label: &str,
    tooltip: &str,
    entries: &mut Vec<awase::config::AppOverrideEntry>,
    buf: &mut (String, String),
) {
    ui.label(label).on_hover_text(tooltip);
    let mut rm = None;
    for (i, e) in entries.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.label(format!("    {} / {}", e.process, e.class));
            if ui
                .small_button("x")
                .on_hover_text("押すと: この行を削除します。")
                .clicked()
            {
                rm = Some(i);
            }
        })
        .response
        .on_hover_text(tooltip);
    }
    if let Some(i) = rm {
        entries.remove(i);
    }
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut buf.0)
                .desired_width(150.0)
                .hint_text("プロセス名 (例: msedge.exe)")
                .id(egui::Id::new(format!("{id}_proc"))),
        )
        .on_hover_text("対象プロセスの実行ファイル名です。完全一致で判定します。");
        ui.add(
            egui::TextEdit::singleline(&mut buf.1)
                .desired_width(200.0)
                .hint_text("クラス名 (完全一致)")
                .id(egui::Id::new(format!("{id}_class"))),
        )
        .on_hover_text(
            "対象ウィンドウのクラス名です。完全一致で判定します。\nログの [focus-sync] 行などで確認できます。",
        );
        if ui
            .button("+追加")
            .on_hover_text("押すと: 入力したプロセス名・クラス名の組み合わせを一覧に追加します。")
            .clicked()
            && !buf.0.is_empty()
            && !buf.1.is_empty()
        {
            entries.push(awase::config::AppOverrideEntry {
                process: std::mem::take(&mut buf.0),
                class: std::mem::take(&mut buf.1),
            });
        }
    });
    ui.add_space(8.0);
}

/// `disable_apps`（プロセス名のみでアプリ全体を無効化するリスト）編集 UI。
///
/// `override_list_ui` はプロセス名+クラス名の2フィールド固定で密結合なため、
/// プロセス名のみの入力欄を別関数として新設した（トレイト/クロージャ導入は
/// 40行の関数には過剰）。
fn process_list_ui(
    ui: &mut egui::Ui,
    id: &str,
    label: &str,
    tooltip: &str,
    entries: &mut Vec<String>,
    buf: &mut String,
) {
    ui.label(label).on_hover_text(tooltip);
    let mut rm = None;
    for (i, e) in entries.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.label(format!("    {e}"));
            if ui
                .small_button("x")
                .on_hover_text("押すと: この行を削除します。")
                .clicked()
            {
                rm = Some(i);
            }
        })
        .response
        .on_hover_text(tooltip);
    }
    if let Some(i) = rm {
        entries.remove(i);
    }
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(buf)
                .desired_width(200.0)
                .hint_text("プロセス名 (例: mstsc.exe)")
                .id(egui::Id::new(format!("{id}_proc"))),
        )
        .on_hover_text(
            "対象プロセスの実行ファイル名です。大文字小文字は区別せず、.exe の有無どちらでも一致します。",
        );
        if ui
            .button("+追加")
            .on_hover_text("押すと: 入力したプロセス名を一覧に追加します。")
            .clicked()
            && !buf.is_empty()
        {
            entries.push(std::mem::take(buf));
        }
    });
    ui.add_space(8.0);
}

/// `engine_off_solo_repeat`（単独5連打でエンジン OFF にするキー）の選択 UI。
///
/// `SOLO_REPEAT_EXTRA_OPTIONS`（Insert 等、親指キーではない候補）を
/// `THUMB_KEY_OPTIONS`（親指キーとして設定した VK と同じものを使う場合）の
/// 前に並べる——既定値は Insert のため一覧の先頭に来た方が見つけやすい。
fn solo_repeat_combo(ui: &mut egui::Ui, current: &mut Option<String>, tooltip: &str) {
    let all_options = SOLO_REPEAT_EXTRA_OPTIONS
        .iter()
        .chain(THUMB_KEY_OPTIONS.iter());
    let display = current.as_deref().map_or_else(
        || "（無効）".to_string(),
        |v| {
            SOLO_REPEAT_EXTRA_OPTIONS
                .iter()
                .chain(THUMB_KEY_OPTIONS.iter())
                .find(|(_, internal)| *internal == v)
                .map_or_else(|| v.to_string(), |(d, _)| (*d).to_string())
        },
    );
    egui::ComboBox::from_id_salt("engine_off_solo_repeat")
        .selected_text(display)
        .width(110.0)
        .show_ui(ui, |ui| {
            if ui.selectable_label(current.is_none(), "（無効）").clicked() {
                *current = None;
            }
            for (label, internal) in all_options {
                if ui
                    .selectable_label(current.as_deref() == Some(*internal), *label)
                    .clicked()
                {
                    *current = Some((*internal).to_string());
                }
            }
        })
        .response
        .on_hover_text(tooltip);
}

/// 無変換/変換キー単独タップの抑制方針。実体は `*_solo_tap_always_suppress`/
/// `*_solo_tap_ignore_composing_guard` の2boolだが、GUI上は「常に無視する」/
/// 「常に送出する」の2択コンボボックスとして見せる。当初は変換候補ウィンドウ
/// 表示中かどうかで挙動を変える中間状態も設けていたが、その判定（composing、
/// UIA/MSAAのフォーカス監視に依存）自体がこのリポジトリでは何度も裏切ってきた
/// 実績があり（例: BUG-11 の UIA キャッシュ汚染）、信頼できない判定を条件にした
/// 中間状態を持たせても挙動が読めないだけと判断し2026-08-15に2択へ簡略化した
/// （ユーザー判断）。config.tomlのスキーマ・エンジン側（`nicola_fsm.rs`等）は
/// 従来通り2bool独立のまま変更しない——「常に送出する」選択時は
/// `ignore_composing_guard`を`true`に固定することで、常にcomposing判定を
/// 無視した一貫した挙動にする。
#[derive(Clone, Copy, PartialEq, Eq)]
enum SoloTapSuppressMode {
    AlwaysSuppress,
    PassThrough,
}

impl SoloTapSuppressMode {
    const ALL: [Self; 2] = [Self::AlwaysSuppress, Self::PassThrough];

    fn from_bools(always_suppress: bool) -> Self {
        if always_suppress {
            Self::AlwaysSuppress
        } else {
            Self::PassThrough
        }
    }

    fn apply(self, always_suppress: &mut bool, ignore_composing_guard: &mut bool) {
        match self {
            Self::AlwaysSuppress => *always_suppress = true,
            Self::PassThrough => {
                *always_suppress = false;
                *ignore_composing_guard = true;
            }
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::AlwaysSuppress => "常に無視する（既定）",
            Self::PassThrough => "常に送出する（パススルー）",
        }
    }

    fn hover_text(self, key_label: &str, default_hijack_risk: &str) -> String {
        match self {
            Self::AlwaysSuppress => format!(
                "{key_label}キーの単独タップを常に完全に無視します\n\
                 （OS へ一切送出しません）。\n\
                 {default_hijack_risk}\n\
                 {key_label}キー本来の機能を Windows 全般で使いたい場合のみ\n\
                 「常に送出する」にしてください。"
            ),
            Self::PassThrough => format!(
                "{key_label}キーの単独タップを常に{key_label}キー本来の機能として\n\
                 OS へ送出します。変換候補ウィンドウの表示有無では挙動を\n\
                 変えません（この判定自体がフォーカス監視に依存し必ずしも\n\
                 信頼できないため、中間の挙動は設けていません）。\n\
                 {default_hijack_risk}"
            ),
        }
    }
}

/// 「左Shift単独タップで半角英数トグルを有効にする」チェックボックス。
/// `Off`/`All`の二択として操作する（`MsImeOnly`はGUIからは選べない中間値、
/// チェックボックスに触れなければ既存の`MsImeOnly`設定は変更されない）。
fn half_width_alnum_toggle_checkbox(
    ui: &mut egui::Ui,
    policy: &mut awase::config::HalfWidthAlnumTogglePolicy,
) {
    let mut enabled = *policy == awase::config::HalfWidthAlnumTogglePolicy::All;
    if ui
        .checkbox(
            &mut enabled,
            "左Shift単独タップで半角英数トグルを有効にする",
        )
        .on_hover_text(
            "ONにすると: 左Shiftキーを他のキーを介さずに単独でタップすると、\n\
             IMEをONにしたまま半角英数入力に切り替わります（もう一度タップ、\n\
             または右Shiftタップで解除）。MS-IME・Google 日本語入力の\n\
             両方で有効になります（実機ソーク中の機能、BUG-25 参照）。\n\
             OFFにすると: この機能全体を無効化します。",
        )
        .changed()
    {
        *policy = if enabled {
            awase::config::HalfWidthAlnumTogglePolicy::All
        } else {
            awase::config::HalfWidthAlnumTogglePolicy::Off
        };
    }
}

/// 無変換/変換キー単独タップの抑制方針コンボボックス。`key_label`は
/// 表示用（例:「無変換」「変換」）、`default_hijack_risk`はそのキーの
/// 単独タップを素通しした際にMS-IME既定割当てへ横取りされるリスクの説明
/// （既存の各チェックボックスの`on_hover_text`から引き継いだ、キーごとに
/// 異なる根拠）。
fn solo_tap_suppress_combo(
    ui: &mut egui::Ui,
    key_label: &str,
    default_hijack_risk: &str,
    always_suppress: &mut bool,
    ignore_composing_guard: &mut bool,
) {
    let mut mode = SoloTapSuppressMode::from_bools(*always_suppress);
    ui.horizontal(|ui| {
        ui.label(format!("{key_label}キー単独タップ:"));
        egui::ComboBox::from_id_salt(format!("{key_label}_solo_tap_suppress"))
            .selected_text(mode.label())
            .width(300.0)
            .show_ui(ui, |ui| {
                for option in SoloTapSuppressMode::ALL {
                    if ui
                        .selectable_label(mode == option, option.label())
                        .on_hover_text(option.hover_text(key_label, default_hijack_risk))
                        .clicked()
                    {
                        mode = option;
                    }
                }
            })
            .response
            .on_hover_text(mode.hover_text(key_label, default_hijack_risk));
    });
    mode.apply(always_suppress, ignore_composing_guard);
}

/// `combo_key_list_ui` の「新規追加」行が保持する一時入力状態。
#[derive(Default)]
struct NewComboBuf {
    ctrl: bool,
    shift: bool,
    alt: bool,
    main: String,
}

/// エンジン制御・IME制御キー用のキーリスト UI。
///
/// 自由記述テキストの代わりに、Ctrl/Shift/Alt の修飾チェックボックスと
/// `THUMB_KEY_OPTIONS`（変換/無変換/かな/F13-F20 等の安全な候補のみ）から選ぶ
/// メインキーのドロップダウンで組み立てる。既存エントリもその場で編集できる。
/// `parse_combo_str`/`format_combo`（keymap タブと共通）で文字列化するため、
/// バックエンドのパース（`vk::parse_key_combo`）・config 形式は変更不要。
fn combo_key_list_ui(
    ui: &mut egui::Ui,
    label: &str,
    id: &str,
    keys: &mut Vec<String>,
    new_entry: &mut NewComboBuf,
    tooltip: &str,
) {
    ui.label(format!("  {label}:")).on_hover_text(tooltip);
    let mut rm = None;
    for (i, key) in keys.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            let (mut ctrl, mut shift, mut alt, mut main) = parse_combo_str(key);
            let mut changed = false;
            changed |= ui.checkbox(&mut ctrl, "Ctrl").changed();
            changed |= ui.checkbox(&mut shift, "Shift").changed();
            changed |= ui.checkbox(&mut alt, "Alt").changed();
            if engine_key_combo(ui, &format!("{id}_{i}"), &mut main, tooltip) {
                changed = true;
            }
            if changed {
                *key = format_combo(ctrl, shift, alt, &main);
            }
            if ui
                .small_button("x")
                .on_hover_text("押すと: この組み合わせを削除します。")
                .clicked()
            {
                rm = Some(i);
            }
        })
        .response
        .on_hover_text(tooltip);
    }
    if let Some(i) = rm {
        keys.remove(i);
    }
    ui.horizontal(|ui| {
        ui.checkbox(&mut new_entry.ctrl, "Ctrl");
        ui.checkbox(&mut new_entry.shift, "Shift");
        ui.checkbox(&mut new_entry.alt, "Alt");
        engine_key_combo(ui, &format!("{id}_new"), &mut new_entry.main, tooltip);
        if ui
            .button("+追加")
            .on_hover_text("押すと: 上で組み立てたキーの組み合わせを一覧に追加します。")
            .clicked()
            && !new_entry.main.is_empty()
        {
            keys.push(format_combo(
                new_entry.ctrl,
                new_entry.shift,
                new_entry.alt,
                &new_entry.main,
            ));
            *new_entry = NewComboBuf::default();
        }
    });
}

/// エンジン制御・IME制御用の main key ドロップダウン（`THUMB_KEY_OPTIONS` +
/// `IME_MODE_KEY_OPTIONS`。Alt impersonation 候補は含めない。必須選択・空欄なし）。
/// 変更時は true を返す。
fn engine_key_combo(ui: &mut egui::Ui, id: &str, current: &mut String, tooltip: &str) -> bool {
    let options = THUMB_KEY_OPTIONS.iter().chain(IME_MODE_KEY_OPTIONS);
    let display = options
        .clone()
        .find(|(_, internal)| *internal == current.as_str())
        .map_or(current.as_str(), |(d, _)| *d)
        .to_string();
    let mut changed = false;
    egui::ComboBox::from_id_salt(id)
        .selected_text(if current.is_empty() {
            "（未選択）"
        } else {
            &display
        })
        .width(110.0)
        .show_ui(ui, |ui| {
            for (label, internal) in options {
                if ui.selectable_label(current == internal, *label).clicked() {
                    *current = (*internal).to_string();
                    changed = true;
                }
            }
        })
        .response
        .on_hover_text(tooltip);
    changed
}

/// 親指キー選択用候補一覧（表示名, config 内部表記）。
///
/// F13-F20: 物理キーとしては存在しない拡張ファンクションキー。プログラマブル
/// キーボード（QMK/ZMK 等）で親指位置のキーに割り当てて使う想定。US 配列で
/// 無変換/変換キーが無い場合の代替はこちらの範囲を推奨する。
///
/// F21-F24 は意図的に含めていない。awase 内部（ADR-091 の GJI 専用Fnキー
/// 自動検出等）で使う予約範囲のため、ユーザー側の親指キー/ホットキー選択に
/// 割り当てさせない（2026-08-15 ユーザー判断）。
/// 意図的に含めていないもの: VK_LCONTROL/VK_RCONTROL（Ctrl）・VK_LWIN/VK_RWIN（Win）。
/// これらは `ModifierState::is_os_modifier_held` の対象で、`bypass_reason` が
/// そのキーの KeyDown を即座に `OsModifierHeld` として素通しするため、親指キーに
/// 割り当てても `PendingThumb` に一切入らず同時打鍵検出そのものが機能しない
/// （`engine/tests.rs` の
/// `test_ctrl_alt_win_thumb_key_never_enters_pending_due_to_os_modifier_bypass` で
/// 確認済み。手動 remap の思いつきではなく実測済みの制約）。手動で config.toml に
/// 書けばパースは通るが動作しないため、GUI の候補としては提示しない。
/// Alt (VK_LMENU/VK_RMENU) は本来同じ制約を受けるが、`ALT_IMPERSONATION_OPTIONS`
/// （左親指/右親指の候補にのみ追加で表示、`thumb_key_combo` 参照）経由でなら
/// エンジン ON 時限定のなりすまし機構（`hook.rs::resolve_thumb_key`）が
/// この制約を回避するため使用可能。単独5連打エンジンOFF（`solo_repeat_combo`）
/// 等、`THUMB_KEY_OPTIONS` を共有する他の用途には Alt を出さないよう分離してある。
const THUMB_KEY_OPTIONS: &[(&str, &str)] = &[
    ("Space", "VK_SPACE"),
    ("Enter", "VK_RETURN"),
    ("変換", "VK_CONVERT"),
    ("無変換", "VK_NONCONVERT"),
    ("かな", "VK_KANA"),
    ("カタカナ", "VK_DBE_KATAKANA"),
    ("ひらがな", "VK_DBE_HIRAGANA"),
    ("F13", "VK_F13"),
    ("F14", "VK_F14"),
    ("F15", "VK_F15"),
    ("F16", "VK_F16"),
    ("F17", "VK_F17"),
    ("F18", "VK_F18"),
    ("F19", "VK_F19"),
    ("F20", "VK_F20"),
];

/// `left_thumb_key`/`right_thumb_key` が無変換キーを指しているか。
///
/// `THUMB_KEY_OPTIONS` のドロップダウンで選択すると内部表記 `"VK_NONCONVERT"`
/// が書き込まれるが、`config.rs` のデフォルト値は漢字表記 `"無変換"` のまま
/// （表記が統一されていない）。無変換キー単独タップ設定の表示条件が漢字表記
/// だけを見ていたため、ドロップダウンで選び直すと表示が消える不具合があった
/// （GitHub issue #99、report `01M10SA5K7J4HZ3C5R1BF6K2QK`）。
///
/// 独自の別名リストを持たず、実際のキー入力解決に使われる
/// `VkCodeExt::from_name`（`vk.rs`、`"VK_MUHENKAN"`/`"Nonconvert"` 等の
/// 別名も含む）に解決させて比較する。文字列比較の一覧を二重管理すると、
/// vk.rs 側に別名が追加されたときに表示条件だけ追従し忘れて同じ不具合が
/// 再発するため（/code-review 指摘）。
fn is_muhenkan_thumb_key(key: &str) -> bool {
    VkCode::from_name(key) == Some(awase_windows::vk::VK_NONCONVERT)
}

/// `left_thumb_key`/`right_thumb_key` が変換キーを指しているか。
/// [`is_muhenkan_thumb_key`] の変換キー版。
fn is_henkan_thumb_key(key: &str) -> bool {
    VkCode::from_name(key) == Some(awase_windows::vk::VK_CONVERT)
}

/// 左親指/右親指キーの候補にのみ追加する、Alt なりすまし用エントリ。
///
/// 内部表記 `"Left Alt"`/`"Right Alt"` は VK 名ではなく、`hook.rs::resolve_thumb_key`
/// が特別に解釈する指示文字列。物理 Left/Right Alt キーをエンジン ON 時に限り
/// 親指キー（無変換/変換相当）として扱う（`config.rs` の `GeneralConfig::keyboard_model`
/// doc・`THUMB_KEY_OPTIONS` doc 参照）。`solo_repeat_combo` 等、`THUMB_KEY_OPTIONS` を
/// 共有する他の用途には出さないため、意図的に別の定数に分離してある。
const ALT_IMPERSONATION_OPTIONS: &[(&str, &str)] =
    &[("Left Alt", "Left Alt"), ("Right Alt", "Right Alt")];

/// `solo_repeat_combo`（単独5連打エンジンOFF）にのみ追加する、親指キーでは
/// ない候補。`engine_off_solo_repeat` は `NicolaFsm::handle_bypass` が
/// `KeyClass::Passthrough` 経路で独立にカウントするため、`left_thumb_key`/
/// `right_thumb_key` と無関係な VK でも動作する（`config.rs` の
/// `KeysConfig::engine_off_solo_repeat` doc 参照）。既定値の Insert は
/// JIS/US どちらの物理キーボードにも存在し、通常のタイピングで連打される
/// ことがなく、他の既定キー割当てとも重複しないため選んだ
/// （2026-08-25、無変換キーが `left_thumb_key` と `keys.ime_on`/`ime_off` に
/// 二重に割り当てられていると Phase 1 ホットキー層が先に消費してこの機能が
/// 無反応になる実例が確認されたため、既定値を無変換から変更した）。
const SOLO_REPEAT_EXTRA_OPTIONS: &[(&str, &str)] = &[("Insert", "VK_INSERT")];

/// エンジン制御・IME制御・IME検出用のドロップダウンにのみ追加する、IME モード
/// 切替キー。`VK_DBE_ALPHANUMERIC`（英数）は `VkCode::from_name`（vk.rs）で
/// 解決可能で `config.toml` に手書きすれば従来から機能していたが、
/// `THUMB_KEY_OPTIONS` に候補が無く GUI 上選べなかった
/// （2026-08-03 ユーザー報告「エンジンOFFの条件で英数キーが選択出来ない」）。
/// `VK_KANJI`（漢字）は「IME ON/OFF トグル」（`keys.ime_toggle`）の既定値
/// （2026-08-16 ユーザー要望）として選べるようにするため追加。
///
/// `THUMB_KEY_OPTIONS` には**混ぜない**: `thumb_key_combo`/`solo_repeat_combo`
/// （親指キー・単独連打候補）は同時打鍵の相手や単独タップ判定に使われるため、
/// IME モード専用キーをそこに混入させると意図しない組み合わせが選択可能に
/// なってしまう。`ALT_IMPERSONATION_OPTIONS` と同じ
/// 「用途ごとに候補リストを分離する」既存パターンに倣う。
const IME_MODE_KEY_OPTIONS: &[(&str, &str)] =
    &[("英数", "VK_DBE_ALPHANUMERIC"), ("漢字", "VK_KANJI")];

#[cfg(test)]
mod ime_mode_key_options_tests {
    use super::{IME_MODE_KEY_OPTIONS, THUMB_KEY_OPTIONS};

    /// 「英数」がエンジン制御/IME検出のドロップダウン候補に出るようにする
    /// （2026-08-03 ユーザー報告の回帰防止）。
    #[test]
    fn ime_mode_key_options_contains_eisu() {
        assert!(
            IME_MODE_KEY_OPTIONS
                .iter()
                .any(|(label, internal)| *label == "英数" && *internal == "VK_DBE_ALPHANUMERIC")
        );
    }

    /// 「漢字」が IME ON/OFF トグル欄のドロップダウン候補に出るようにする
    /// （`keys.ime_toggle` の既定値 `VK_KANJI` が選択可能である必要がある）。
    #[test]
    fn ime_mode_key_options_contains_kanji() {
        assert!(
            IME_MODE_KEY_OPTIONS
                .iter()
                .any(|(label, internal)| *label == "漢字" && *internal == "VK_KANJI")
        );
    }

    /// IME モード専用キーは親指キー候補（`THUMB_KEY_OPTIONS`）には混ぜない
    /// （`IME_MODE_KEY_OPTIONS` の doc コメント参照）。
    #[test]
    fn ime_mode_key_options_do_not_leak_into_thumb_key_options() {
        for (_, internal) in IME_MODE_KEY_OPTIONS {
            assert!(
                !THUMB_KEY_OPTIONS.iter().any(|(_, t)| t == internal),
                "{internal} が THUMB_KEY_OPTIONS に漏れている"
            );
        }
    }
}

#[cfg(test)]
mod thumb_key_display_condition_tests {
    use super::{is_henkan_thumb_key, is_muhenkan_thumb_key};

    /// `config.rs` の初期デフォルト値（漢字表記）でも、`THUMB_KEY_OPTIONS`
    /// ドロップダウン選択後の内部表記でも、無変換キー単独タップ設定の
    /// 表示条件が一致すること（GitHub issue #99、report
    /// `01M10SA5K7J4HZ3C5R1BF6K2QK` の回帰防止）。
    #[test]
    fn is_muhenkan_thumb_key_matches_both_representations() {
        assert!(is_muhenkan_thumb_key("無変換"));
        assert!(is_muhenkan_thumb_key("VK_NONCONVERT"));
        assert!(!is_muhenkan_thumb_key("変換"));
        assert!(!is_muhenkan_thumb_key("VK_CONVERT"));
    }

    /// 変換キー版。上記と対称。
    #[test]
    fn is_henkan_thumb_key_matches_both_representations() {
        assert!(is_henkan_thumb_key("変換"));
        assert!(is_henkan_thumb_key("VK_CONVERT"));
        assert!(!is_henkan_thumb_key("無変換"));
        assert!(!is_henkan_thumb_key("VK_NONCONVERT"));
    }
}

/// keymap タブで使用する主キー一覧（表示名, parse_key_combo に渡す内部表記）。
///
/// 記号キーの表示ラベルは JIS 配列基準（VK_OEM_PLUS=「;」, VK_OEM_3=「@」 等）。
/// US 配列では同じ VK が別の文字に対応するため、ツールチップで補足する。
const KEYMAP_MAIN_KEYS: &[(&str, &str)] = &[
    // アルファベット
    ("A", "VK_A"),
    ("B", "VK_B"),
    ("C", "VK_C"),
    ("D", "VK_D"),
    ("E", "VK_E"),
    ("F", "VK_F"),
    ("G", "VK_G"),
    ("H", "VK_H"),
    ("I", "VK_I"),
    ("J", "VK_J"),
    ("K", "VK_K"),
    ("L", "VK_L"),
    ("M", "VK_M"),
    ("N", "VK_N"),
    ("O", "VK_O"),
    ("P", "VK_P"),
    ("Q", "VK_Q"),
    ("R", "VK_R"),
    ("S", "VK_S"),
    ("T", "VK_T"),
    ("U", "VK_U"),
    ("V", "VK_V"),
    ("W", "VK_W"),
    ("X", "VK_X"),
    ("Y", "VK_Y"),
    ("Z", "VK_Z"),
    // 数字
    ("0", "VK_0"),
    ("1", "VK_1"),
    ("2", "VK_2"),
    ("3", "VK_3"),
    ("4", "VK_4"),
    ("5", "VK_5"),
    ("6", "VK_6"),
    ("7", "VK_7"),
    ("8", "VK_8"),
    ("9", "VK_9"),
    // 記号キー（JIS 配列）
    (";", "VK_OEM_PLUS"),
    (":", "VK_OEM_1"),
    (",", "VK_OEM_COMMA"),
    ("-", "VK_OEM_MINUS"),
    (".", "VK_OEM_PERIOD"),
    ("/", "VK_OEM_2"),
    ("@", "VK_OEM_3"),
    ("[", "VK_OEM_4"),
    ("¥", "VK_OEM_5"),
    ("]", "VK_OEM_6"),
    ("^", "VK_OEM_7"),
    ("_", "VK_OEM_102"),
    // ファンクションキー
    ("F1", "VK_F1"),
    ("F2", "VK_F2"),
    ("F3", "VK_F3"),
    ("F4", "VK_F4"),
    ("F5", "VK_F5"),
    ("F6", "VK_F6"),
    ("F7", "VK_F7"),
    ("F8", "VK_F8"),
    ("F9", "VK_F9"),
    ("F10", "VK_F10"),
    ("F11", "VK_F11"),
    ("F12", "VK_F12"),
    // F13-F20: 物理キーとしては存在しない拡張ファンクションキー
    // （`THUMB_KEY_OPTIONS` の doc コメント参照）。プログラマブルキーボードで
    // 割り当てて使う想定。egui のキーイベントには対応する `Key` 変種が無く
    // ショートカット再割当タブの⌨キャプチャでは検出できないため、この
    // ドロップダウンが唯一の設定手段（2026-08-15 ユーザー要望）。
    // F21-F24 は意図的に含めていない。awase 内部（ADR-091 の GJI 専用Fnキー
    // 自動検出等）の予約範囲のため（2026-08-15 ユーザー判断、
    // `THUMB_KEY_OPTIONS` doc 参照）。
    ("F13", "VK_F13"),
    ("F14", "VK_F14"),
    ("F15", "VK_F15"),
    ("F16", "VK_F16"),
    ("F17", "VK_F17"),
    ("F18", "VK_F18"),
    ("F19", "VK_F19"),
    ("F20", "VK_F20"),
    // 制御キー
    ("Space", "VK_SPACE"),
    ("Enter", "VK_RETURN"),
    ("Tab", "VK_TAB"),
    ("Esc", "VK_ESCAPE"),
    ("Backspace", "VK_BACK"),
    ("Delete", "VK_DELETE"),
    ("Insert", "VK_INSERT"),
    ("Home", "VK_HOME"),
    ("End", "VK_END"),
    ("PgUp", "VK_PRIOR"),
    ("PgDn", "VK_NEXT"),
    ("PrintScreen", "VK_SNAPSHOT"),
    // IME 関連
    ("変換", "変換"),
    ("無変換", "無変換"),
    ("かな", "かな"),
    ("漢字", "漢字"),
    ("IMEオン", "VK_IME_ON"),
    ("IMEオフ", "VK_IME_OFF"),
    ("英数", "VK_DBE_ALPHANUMERIC"),
    ("カタカナ", "VK_DBE_KATAKANA"),
    ("ひらがな", "VK_DBE_HIRAGANA"),
    ("半角", "VK_DBE_SBCSCHAR"),
    ("全角", "VK_DBE_DBCSCHAR"),
];

const fn keyboard_model_label(model: awase::scanmap::KeyboardModel) -> &'static str {
    use awase::scanmap::KeyboardModel;
    match model {
        KeyboardModel::Jis => "JIS (日本語109キー)",
        KeyboardModel::Us => "US (ANSI 104キー)",
    }
}

/// 内部表記（"VK_I", "変換" 等）を表示名（"I", "変換"）に変換する。
fn key_display_name(internal: &str) -> &str {
    KEYMAP_MAIN_KEYS
        .iter()
        .find(|(_, v)| *v == internal)
        .map_or(internal, |(d, _)| *d)
}

/// keymap rule の `from` 文字列を (Ctrl, Shift, Alt, main_internal) に分解する。
/// パース失敗時は (false, false, false, "") を返す。
fn parse_combo_str(s: &str) -> (bool, bool, bool, String) {
    let parts: Vec<&str> = s.split('+').map(str::trim).collect();
    if parts.is_empty() {
        return (false, false, false, String::new());
    }
    let (mut ctrl, mut shift, mut alt) = (false, false, false);
    let mod_count = parts.len().saturating_sub(1);
    for &part in &parts[..mod_count] {
        match part {
            "Ctrl" | "Control" => ctrl = true,
            "Shift" => shift = true,
            "Alt" => alt = true,
            _ => {}
        }
    }
    let main = (*parts.last().unwrap_or(&"")).to_string();
    (ctrl, shift, alt, main)
}

/// 修飾キーと main key から keymap rule 用文字列を組み立てる。
fn format_combo(ctrl: bool, shift: bool, alt: bool, main: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if ctrl {
        parts.push("Ctrl");
    }
    if shift {
        parts.push("Shift");
    }
    if alt {
        parts.push("Alt");
    }
    parts.push(main);
    parts.join("+")
}

/// 親指キー選択ドロップダウン。変更時は true を返す。
fn thumb_key_combo(ui: &mut egui::Ui, id: &str, current: &mut String, tooltip: &str) -> bool {
    let options = THUMB_KEY_OPTIONS.iter().chain(ALT_IMPERSONATION_OPTIONS);
    let display = options
        .clone()
        .find(|(_, v)| *v == current.as_str())
        .map_or(current.as_str(), |(d, _)| *d)
        .to_string();
    let mut changed = false;
    egui::ComboBox::from_id_salt(id)
        .selected_text(if current.is_empty() {
            "（未選択）"
        } else {
            &display
        })
        .width(110.0)
        .show_ui(ui, |ui| {
            for (label, internal) in options {
                if ui
                    .selectable_label(current.as_str() == *internal, *label)
                    .clicked()
                {
                    *current = (*internal).to_string();
                    changed = true;
                }
            }
        })
        .response
        .on_hover_text(tooltip);
    changed
}

/// main key ドロップダウン（必須選択版）。変更時は true を返す。
fn main_key_combo(ui: &mut egui::Ui, id: &str, current: &mut String, tooltip: &str) -> bool {
    let display = key_display_name(current).to_string();
    let mut changed = false;
    egui::ComboBox::from_id_salt(id)
        .selected_text(if current.is_empty() {
            "（未選択）"
        } else {
            &display
        })
        .width(110.0)
        .show_ui(ui, |ui| {
            for (label, internal) in KEYMAP_MAIN_KEYS {
                if ui.selectable_label(current == internal, *label).clicked() {
                    *current = (*internal).to_string();
                    changed = true;
                }
            }
        })
        .response
        .on_hover_text(tooltip);
    changed
}

/// グローバルトグルホットキー（`GeneralConfig::engine_toggle_hotkey`、単一値）用の
/// Ctrl/Shift/Alt チェックボックス + メインキードロップダウン。
///
/// `vk::parse_hotkey` は `vk::parse_key_combo` と同じ修飾キー解析を使うため、
/// `parse_combo_str`/`format_combo`（keymap タブと共通）でそのまま組み立てられる。
/// メインキー候補は IME 系ではなく英数字/F1-F12/OEM記号中心の `KEYMAP_MAIN_KEYS`
/// （デフォルト値 `"Ctrl+Shift+F12"` と同系統）を使う。メインキー未選択の状態は
/// ホットキー無効（`None`）として扱う。
fn hotkey_combo_ui(ui: &mut egui::Ui, id: &str, current: &mut Option<String>, tooltip: &str) {
    let (mut ctrl, mut shift, mut alt, mut main) =
        parse_combo_str(current.as_deref().unwrap_or(""));
    let mut changed = false;
    changed |= ui.checkbox(&mut ctrl, "Ctrl").changed();
    changed |= ui.checkbox(&mut shift, "Shift").changed();
    changed |= ui.checkbox(&mut alt, "Alt").changed();
    if main_key_combo(ui, id, &mut main, tooltip) {
        changed = true;
    }
    if ui
        .small_button("解除")
        .on_hover_text("押すと: このホットキーの設定を解除します（未設定に戻します）。")
        .clicked()
    {
        *current = None;
        return;
    }
    if changed {
        *current = if main.is_empty() {
            None
        } else {
            Some(format_combo(ctrl, shift, alt, &main))
        };
    }
}

/// キー入力キャプチャボタン。クリックでこの target をキャプチャ対象に設定し、
/// 既にこの target がキャプチャ中なら「待機中」ラベルを表示する。
fn capture_button(ui: &mut egui::Ui, capturing: &mut Option<CaptureTarget>, target: CaptureTarget) {
    let is_active = *capturing == Some(target);
    let label = if is_active { "⌨ 待機…" } else { "⌨" };
    if ui
        .selectable_label(is_active, label)
        .on_hover_text("クリック後にキーを押すと自動入力されます (Esc で取消)")
        .clicked()
    {
        *capturing = if is_active { None } else { Some(target) };
    }
}

/// egui のキー名を内部 VK 名に変換する。マップ対象外は None。
///
/// OEM 記号は JIS 配列前提でマッピング:
/// `;` (Semicolon) → VK_OEM_PLUS、`¥` (Backslash) → VK_OEM_5 など。
/// US 配列では VK 対応が異なるため、捕捉結果が期待と違う場合は
/// ドロップダウンから直接選択すること。
///
/// PrintScreen と IME 系キー（変換/無変換/漢字/かな/英数 等）は
/// egui に対応する Key 変種が無いため、引き続きドロップダウン専用。
fn egui_key_to_internal(key: egui::Key) -> Option<&'static str> {
    use egui::Key;
    Some(match key {
        Key::A => "VK_A",
        Key::B => "VK_B",
        Key::C => "VK_C",
        Key::D => "VK_D",
        Key::E => "VK_E",
        Key::F => "VK_F",
        Key::G => "VK_G",
        Key::H => "VK_H",
        Key::I => "VK_I",
        Key::J => "VK_J",
        Key::K => "VK_K",
        Key::L => "VK_L",
        Key::M => "VK_M",
        Key::N => "VK_N",
        Key::O => "VK_O",
        Key::P => "VK_P",
        Key::Q => "VK_Q",
        Key::R => "VK_R",
        Key::S => "VK_S",
        Key::T => "VK_T",
        Key::U => "VK_U",
        Key::V => "VK_V",
        Key::W => "VK_W",
        Key::X => "VK_X",
        Key::Y => "VK_Y",
        Key::Z => "VK_Z",
        Key::Num0 => "VK_0",
        Key::Num1 => "VK_1",
        Key::Num2 => "VK_2",
        Key::Num3 => "VK_3",
        Key::Num4 => "VK_4",
        Key::Num5 => "VK_5",
        Key::Num6 => "VK_6",
        Key::Num7 => "VK_7",
        Key::Num8 => "VK_8",
        Key::Num9 => "VK_9",
        Key::F1 => "VK_F1",
        Key::F2 => "VK_F2",
        Key::F3 => "VK_F3",
        Key::F4 => "VK_F4",
        Key::F5 => "VK_F5",
        Key::F6 => "VK_F6",
        Key::F7 => "VK_F7",
        Key::F8 => "VK_F8",
        Key::F9 => "VK_F9",
        Key::F10 => "VK_F10",
        Key::F11 => "VK_F11",
        Key::F12 => "VK_F12",
        Key::Space => "VK_SPACE",
        Key::Enter => "VK_RETURN",
        Key::Tab => "VK_TAB",
        // Escape はキャンセル扱いのため、修飾キー付きの場合のみ捕捉される
        Key::Escape => "VK_ESCAPE",
        Key::Backspace => "VK_BACK",
        Key::Delete => "VK_DELETE",
        Key::Insert => "VK_INSERT",
        Key::Home => "VK_HOME",
        Key::End => "VK_END",
        Key::PageUp => "VK_PRIOR",
        Key::PageDown => "VK_NEXT",
        // OEM 記号キー（JIS 配列前提）
        Key::Comma => "VK_OEM_COMMA",
        Key::Period => "VK_OEM_PERIOD",
        Key::Slash => "VK_OEM_2",
        Key::Minus => "VK_OEM_MINUS",
        Key::Semicolon => "VK_OEM_PLUS",
        Key::OpenBracket => "VK_OEM_4",
        Key::CloseBracket => "VK_OEM_6",
        Key::Backslash => "VK_OEM_5",
        _ => return None,
    })
}

/// main key ドロップダウン（オプショナル版＝「消費のみ」選択肢付き）。変更時は true を返す。
fn main_key_combo_optional(
    ui: &mut egui::Ui,
    id: &str,
    current: &mut String,
    tooltip: &str,
) -> bool {
    let display = key_display_name(current).to_string();
    let mut changed = false;
    egui::ComboBox::from_id_salt(id)
        .selected_text(if current.is_empty() {
            "（消費のみ）"
        } else {
            &display
        })
        .width(110.0)
        .show_ui(ui, |ui| {
            if ui
                .selectable_label(current.is_empty(), "（消費のみ）")
                .clicked()
                && !current.is_empty()
            {
                current.clear();
                changed = true;
            }
            for (label, internal) in KEYMAP_MAIN_KEYS {
                if ui.selectable_label(current == internal, *label).clicked() {
                    *current = (*internal).to_string();
                    changed = true;
                }
            }
        })
        .response
        .on_hover_text(tooltip);
    changed
}

// ── 配列編集タブ ヘルパー（旧 awase-yab-editor）──

fn color_legend(ui: &mut egui::Ui, color: egui::Color32, label: &str) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 2.0, color);
    ui.painter().rect_stroke(
        rect,
        2.0,
        egui::Stroke::new(1.0_f32, egui::Color32::GRAY),
        egui::StrokeKind::Middle,
    );
    ui.label(label);
}

fn cell_display(value: Option<&YabValue>) -> String {
    match value {
        Some(YabValue::Romaji { kana: Some(ch), .. }) => ch.to_string(),
        Some(YabValue::Romaji { romaji, .. }) if romaji.len() <= 3 => romaji.clone(),
        Some(YabValue::Romaji { romaji, .. }) => format!("{}.", &romaji[..2]),
        Some(YabValue::Literal(s) | YabValue::KeySequence(s)) if s.chars().count() <= 2 => {
            s.clone()
        }
        Some(YabValue::Literal(s) | YabValue::KeySequence(s)) => {
            s.chars().take(2).collect::<String>() + "."
        }
        // ⌫ (U+232B) 単体はフォントによっては潰れて視認しづらいため、
        // 左矢印 + "BS" で「後ろを消す」方向を明示する（ESC/DEL とテキスト量を揃えた）。
        Some(YabValue::Special(SpecialKey::Backspace)) => "\u{2190}BS".to_string(), // ←BS
        Some(YabValue::Special(SpecialKey::Enter)) => "\u{23ce}".to_string(),       // ⏎
        Some(YabValue::Special(SpecialKey::Escape)) => "ESC".to_string(),
        Some(YabValue::Special(SpecialKey::Space)) => "\u{2423}".to_string(), // ␣
        Some(YabValue::Special(SpecialKey::Delete)) => "DEL".to_string(),
        Some(YabValue::Special(SpecialKey::Insert)) => "INS".to_string(),
        Some(YabValue::Special(SpecialKey::Up)) => "\u{2191}".to_string(), // ↑
        Some(YabValue::Special(SpecialKey::Down)) => "\u{2193}".to_string(), // ↓
        Some(YabValue::Special(SpecialKey::Left)) => "\u{2190}".to_string(), // ←
        Some(YabValue::Special(SpecialKey::Right)) => "\u{2192}".to_string(), // →
        Some(YabValue::Special(SpecialKey::Home)) => "HOME".to_string(),
        Some(YabValue::Special(SpecialKey::End)) => "END".to_string(),
        Some(YabValue::Special(SpecialKey::PageUp)) => "PgUp".to_string(),
        Some(YabValue::Special(SpecialKey::PageDown)) => "PgDn".to_string(),
        Some(YabValue::Vk(vk)) => format!("V{:X}", vk.0),
        // ADR-115決定9(a): CtrlChordはVK、MacroRef/InlineSequenceはrawを
        // そのまま表示する。Sequenceは(resolve_keystroke_syntax後の
        // プレビュー専用コピーにしか現れないため)Noneと同じ表示にする。
        Some(YabValue::CtrlChord { vk, .. }) => format!("Ctrl+{:X}", vk.0),
        Some(YabValue::MacroRef(name)) => format!("@{name}"),
        Some(YabValue::InlineSequence { raw, .. }) => raw.clone(),
        Some(YabValue::Sequence(_) | YabValue::None) | None => "\u{2014}".to_string(), // —
    }
}

const fn cell_color(value: Option<&YabValue>) -> egui::Color32 {
    match value {
        Some(YabValue::Romaji { .. }) => egui::Color32::from_rgb(255, 255, 255),
        Some(YabValue::Literal(_)) => egui::Color32::from_rgb(210, 230, 255),
        Some(YabValue::Special(_)) => egui::Color32::from_rgb(210, 255, 220),
        Some(YabValue::KeySequence(_)) => egui::Color32::from_rgb(200, 235, 255),
        Some(YabValue::Vk(_)) => egui::Color32::from_rgb(255, 225, 200),
        // ADR-115: 打鍵列系は薄紫で統一し、既存カラーパレットと区別する。
        Some(
            YabValue::CtrlChord { .. } | YabValue::InlineSequence { .. } | YabValue::MacroRef(_),
        ) => egui::Color32::from_rgb(230, 210, 255),
        Some(YabValue::Sequence(_) | YabValue::None) | None => {
            egui::Color32::from_rgb(220, 220, 220)
        }
    }
}

/// セルの位置に依存しない、値そのものの説明文字列。
fn value_description(value: Option<&YabValue>) -> String {
    match value {
        Some(YabValue::Romaji { romaji, kana }) => {
            let kana_str = kana.map_or_else(
                || "なし".to_string(),
                |c| {
                    let mut s = String::new();
                    s.push(c);
                    s
                },
            );
            format!("ローマ字: {romaji}  かな: {kana_str}")
        }
        Some(YabValue::Literal(s)) => format!("リテラル: {s}"),
        Some(YabValue::KeySequence(s)) => format!("キーシーケンス: {s}"),
        Some(YabValue::Special(sk)) => format!("特殊キー: {sk:?}"),
        Some(YabValue::Vk(vk)) => format!("仮想キーコード: 0x{:X}", vk.0),
        Some(YabValue::CtrlChord { vk, .. }) => format!("Ctrl+VK: 0x{:X}", vk.0),
        Some(YabValue::InlineSequence { raw, .. }) => format!("打鍵列: {raw}"),
        Some(YabValue::MacroRef(name)) => format!("打鍵列マクロ参照: @{name}"),
        Some(YabValue::Sequence(_) | YabValue::None) | None => "割り当てなし".to_string(),
    }
}

fn cell_tooltip(value: Option<&YabValue>, pos: PhysicalPos) -> String {
    format!("({}, {})  {}", pos.row, pos.col, value_description(value))
}

/// `.yab` ファイルを読み込みパースする。パースには影響しない軽量チェック
/// （`awase::yab::lint`）の結果も併せて返す——タイプミス（例: クォートの
/// 片方だけ書き忘れ）はパース自体には失敗せず無警告でリテラルとして
/// 受理されてしまうため（report `01M13EACMQ7D2VETW75N0BTZ9C`）、読み込み時に
/// ステータス表示で気づけるようにする。
fn load_yab_layout(
    path: &Path,
    model: awase::scanmap::KeyboardModel,
) -> Result<(YabLayout, Vec<String>), String> {
    let content = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    // NOTE: ここは意図的に `resolve_keystroke_syntax` を通さない。この関数が返す
    // `YabLayout` は GUI 編集の作業コピーであると同時に `layout_write_to_path` が
    // そのまま `serialize()` して .yab へ書き戻す対象そのもの。一度 resolve すると
    // `CtrlChord`/`InlineSequence`/`MacroRef` が `Literal`/`Sequence` に置き換わり、
    // 元のセル生テキストが失われた状態で保存されてしまう（`Sequence` は
    // `serialize()` が `無` を返す設計のため、キルスイッチ on で保存すると打鍵列
    // セルが丸ごと消える。off でも `raw` の一部だけが残る形で壊れる）。実装直後の
    // Opus実装後レビューでこの resolve 呼び出しを一度追加したところ、まさにこの
    // 破壊的な保存回帰が3系統のレビュー観点から独立に指摘されたため差し戻した
    // （経緯は `docs/known-bugs.md` FEATURE-115 参照）。GUI プレビューがキルスイッチ
    // off 時の実際の送信内容と食い違う点（新構文セルがそのまま表示される）は
    // 非破壊的な既知の制約として残す。
    let layout = YabLayout::parse(&content, model)
        .map(YabLayout::resolve_kana)
        .map_err(|e| format!("パース失敗: {e}"))?;
    // パース成功後にのみ lint する（失敗時に計算を無駄にしない）。
    let lint_warnings = awase::yab::lint(&content);
    Ok((layout, lint_warnings))
}

/// `load_yab_layout` の警告を読み込み/保存メッセージに付記する。
fn append_lint_warnings(status: String, lint_warnings: &[String]) -> String {
    if lint_warnings.is_empty() {
        status
    } else {
        format!("{status}（警告: {}）", lint_warnings.join("; "))
    }
}

fn empty_yab_layout() -> YabLayout {
    YabLayout {
        name: "untitled".to_string(),
        normal: YabFace::new(),
        left_thumb: YabFace::new(),
        right_thumb: YabFace::new(),
        shift: YabFace::new(),
        left_thumb_shift: YabFace::new(),
        right_thumb_shift: YabFace::new(),
    }
}

// ── Utility functions ──

/// config.toml のパスを解決する。
///
/// `crates/awase-windows/src/app/mod.rs::find_config_path()` と**同じ優先順位**
/// （コマンドライン引数 → `resolve_relative_to_exe`）で解決すること。ここが
/// ズレていると、`awase.exe` を明示パス引数付きショートカット等で起動している
/// 場合に、awase-settings.exe が別の（自動解決された）config.toml を編集して
/// しまい、「設定画面で保存しても awase.exe に反映されない」という実機バグの
/// 原因になる（2026-07-19 に実際に発生し確認済み）。
fn find_config_path() -> std::path::PathBuf {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg.starts_with("--") {
            let _ = args.next(); // value をスキップ
            continue;
        }
        return std::path::PathBuf::from(arg);
    }
    awase::paths::resolve_relative_to_exe("config.toml")
}

/// `layouts_dir` を解決する。実行ファイル隣・`cargo run` 時のワークスペース
/// ルートのどちらでも動くよう、`awase::paths` の共通ロジックに委ねる（かつては
/// ここに exe 隣のみを見る独自ロジックがあり、`target` 配下から起動した際に
/// ワークスペースルート直下の `layout/` を見つけられなかった）。
fn resolve_layouts_dir(layouts_dir: &str) -> std::path::PathBuf {
    awase::paths::resolve_relative_to_exe(layouts_dir)
}

/// `dir` 内の全 `.yab` を読込失敗（UTF-8デコード失敗含む）と
/// `yab::lint()` 警告について診断する（ADR-116 決定3）。
///
/// `awase-windows` 側の `LayoutEntry::scan_all` と同じ「read + lint のみ、
/// `YabLayout::parse` は呼ばない」原則を踏む。パースまで行うと、同梱の
/// JIS用レイアウトが `keyboard_model = "us"` 環境で列数上限超過により
/// 恒久的に警告される（US配列ユーザーには対処しようがない誤警告になる）。
/// `scan_layout_names`（ファイル名一覧のみ返す既存関数）を拡張せず専用の
/// 小さいループにしているのは、共有を狙って抽象化すると
/// `LayoutEntry::scan_all` とほぼ同じロジックの再発明になるため
/// （ADR-116 r1のレビューで指摘された過剰設計と同型）。
fn scan_yab_files_for_diagnostics(dir: &std::path::Path) -> Vec<String> {
    let mut diagnostics = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            // /code-review指摘: 以前は無言で空リストを返していた。
            // `layouts_dir` の打ち間違いは最も起きやすい設定ミスの1つで、
            // awase.exe 側（LayoutEntry::scan_all）は同じ状況で
            // 「レイアウトディレクトリが見つかりません」と警告している
            // （bootstrap.rs:710-716）のに、直しに行く設定画面だけが沈黙する
            // のはADR-116の出発点だったBUG-104と同型の無言フォールバック。
            diagnostics.push(format!(
                "レイアウトディレクトリが見つかりません: {}: {e}",
                dir.display()
            ));
            return diagnostics;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "yab") {
            continue;
        }
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                // 1セルごとに1件積むと、崩れたファイル1つで診断リストが
                // 数十行に膨れ上がる（1ファイルにつき1件へ集約、
                // scan_all 側の同種修正と対称）。
                let lint_warnings = awase::yab::lint(&content);
                if !lint_warnings.is_empty() {
                    diagnostics.push(format!("{}: {}", path.display(), lint_warnings.join(" / ")));
                }
            }
            Err(e) => {
                diagnostics.push(format!("レイアウト読込失敗: {}: {e}", path.display()));
            }
        }
    }
    // /code-review指摘: read_dir の返す順はファイルシステム依存で
    // 実行のたび変わりうる。scan_layout_names（同じ layouts_dir を走査する
    // 既存関数）も名前でソートしているため、これに揃える。
    diagnostics.sort();
    diagnostics
}

#[cfg(test)]
mod scan_yab_files_for_diagnostics_tests {
    use super::scan_yab_files_for_diagnostics;

    fn unique_temp_dir(label: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "awase_test_scan_yab_diag_{label}_{}_{id}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn missing_directory_reports_a_warning_not_silence() {
        // /code-review指摘: 以前は read_dir 失敗時に無言で空 Vec を返しており、
        // layouts_dir の打ち間違いという最も起きやすい設定ミスに対して
        // 設定画面だけが沈黙していた。
        let dir = std::env::temp_dir().join("awase_test_scan_yab_diag_does_not_exist_ever");
        let _ = std::fs::remove_dir_all(&dir);
        let diagnostics = scan_yab_files_for_diagnostics(&dir);
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].contains("レイアウトディレクトリが見つかりません"));
    }

    #[test]
    fn empty_directory_reports_nothing() {
        let dir = unique_temp_dir("empty");
        assert!(scan_yab_files_for_diagnostics(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unreadable_file_reports_one_warning() {
        // 非UTF-8バイト列 = read_to_string が失敗するケース（BUG-104の
        // トリガーそのもの）。
        let dir = unique_temp_dir("unreadable");
        std::fs::write(dir.join("broken.yab"), [0xFF, 0xFE, 0x00, 0x81]).unwrap();
        let diagnostics = scan_yab_files_for_diagnostics(&dir);
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].contains("レイアウト読込失敗"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn quote_corrupted_cells_collapse_into_one_entry_per_file() {
        // BUG-95のクォート崩れ再現。複数セルが崩れていても診断リストの
        // 1エントリに集約されること（/code-review指摘: 1セルごとに1件だと
        // 崩れたファイル1つで診断リストが数十行に膨れ上がる）。
        let dir = unique_temp_dir("lint");
        std::fs::write(
            dir.join("broken.yab"),
            "[ローマ字シフト無し]\n'あ,'い,ｕ,ｅ,ｏ\n",
        )
        .unwrap();
        let diagnostics = scan_yab_files_for_diagnostics(&dir);
        assert_eq!(
            diagnostics.len(),
            1,
            "expected one aggregated entry: {diagnostics:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clean_file_reports_nothing() {
        let dir = unique_temp_dir("clean");
        std::fs::write(
            dir.join("ok.yab"),
            "[ローマ字シフト無し]\nｋａ,ｔａ,ｋｏ,ｓａ,ｒａ\n",
        )
        .unwrap();
        assert!(scan_yab_files_for_diagnostics(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn non_yab_files_are_ignored() {
        let dir = unique_temp_dir("ignore");
        std::fs::write(dir.join("readme.txt"), "not a layout").unwrap();
        assert!(scan_yab_files_for_diagnostics(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

fn scan_layout_names(layouts_dir: &str) -> Vec<String> {
    let dir = resolve_layouts_dir(layouts_dir);
    let mut names = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(stem) = path
                .extension()
                .filter(|ext| *ext == "yab")
                .and_then(|_| path.file_stem())
            {
                names.push(stem.to_string_lossy().to_string());
            }
        }
    }
    names.sort();
    names
}

fn default_config() -> awase::config::AppConfig {
    toml::from_str("[general]").unwrap()
}

fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    for path in &[
        "C:\\Windows\\Fonts\\meiryo.ttc",
        "C:\\Windows\\Fonts\\msgothic.ttc",
        "C:\\Windows\\Fonts\\YuGothR.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc",
    ] {
        if let Ok(font_data) = std::fs::read(path) {
            fonts.font_data.insert(
                "japanese".into(),
                egui::FontData::from_owned(font_data).into(),
            );
            fonts
                .families
                .get_mut(&egui::FontFamily::Proportional)
                .unwrap()
                .insert(0, "japanese".into());
            fonts
                .families
                .get_mut(&egui::FontFamily::Monospace)
                .unwrap()
                .insert(0, "japanese".into());
            break;
        }
    }
    ctx.set_fonts(fonts);
}

fn send_reload_config_message() {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW};
        use windows::core::w;
        unsafe {
            // クラス名は crates/awase-windows/src/tray.rs の
            // `WINDOW_CLASS_NAME` = "awase_tray_window" と必ず一致させること。
            // 以前ここは "awase_msg_window" という存在しないクラス名を探しており、
            // FindWindowW が常に失敗 → 適用ボタンを押しても awase.exe に一切
            // 通知が届かない（無言で失敗する）というバグがあった
            // （2026-07-19 実機で確認・修正）。awase-settings は awase-windows を
            // 依存に持たないため定数を共有できず、文字列直書きで揃えている。
            let hwnd = FindWindowW(w!("awase_tray_window"), None);
            if let Ok(hwnd) = hwnd {
                let msg = windows::Win32::Foundation::WPARAM(0);
                let lparam = windows::Win32::Foundation::LPARAM(0);
                let _ = PostMessageW(hwnd, WM_RELOAD_CONFIG, msg, lparam);
            } else {
                log::warn!(
                    "設定リロード通知の送信先ウィンドウ (awase_tray_window) が見つかりません。\
                     awase.exe が起動していないか、権限レベルが異なる可能性があります。"
                );
            }
        }
    }
}

#[cfg(test)]
mod layout_tab_repro {
    use super::{
        CLIPBOARD_HISTORY_LEN, Face, KanaTable, NewComboBuf, PhysicalPos, SPECIAL_KEYS,
        SettingsApp, Tab, ValueKind, YabValue, empty_yab_layout, find_config_path, load_yab_layout,
        resolve_layouts_dir,
    };

    fn test_settings_app(config: awase::config::AppConfig) -> SettingsApp {
        let layout_path =
            resolve_layouts_dir(&config.general.layouts_dir).join(&config.general.default_layout);
        let layout = load_yab_layout(&layout_path, config.general.keyboard_model)
            .map(|(ly, _lint_warnings)| ly)
            .unwrap_or_else(|_| empty_yab_layout());
        SettingsApp {
            config,
            config_path: std::path::PathBuf::from("config.toml"),
            config_load_state: awase::config::ConfigLoadState::Loaded,
            show_dangerous_save_confirm: false,
            status: String::new(),
            active_tab: Tab::Layout,
            available_layouts: Vec::new(),
            new_engine_on: NewComboBuf::default(),
            new_engine_off: NewComboBuf::default(),
            new_ime_on: NewComboBuf::default(),
            new_ime_off: NewComboBuf::default(),
            new_ime_toggle: NewComboBuf::default(),
            new_keymap_app: String::new(),
            new_keymap_from_ctrl: false,
            new_keymap_from_shift: false,
            new_keymap_from_main: String::new(),
            new_keymap_to_main: String::new(),
            capturing: None,
            new_override_bufs: <[(String, String); 4]>::default(),
            new_disable_app: String::new(),
            new_pb_key: String::new(),
            new_pb_process: String::new(),
            new_pb_class: String::new(),
            layout_file_path_buf: layout_path.display().to_string(),
            layout_file_path: Some(layout_path),
            layout,
            layout_current_face: Face::Normal,
            layout_selected_pos: None,
            layout_clipboard_history: Vec::new(),
            layout_edit_kind: ValueKind::None,
            layout_edit_value: String::new(),
            layout_edit_special_idx: 0,
            kana_table: KanaTable::build(),
            layout_modified: false,
            layout_status: String::new(),
            layout_loaded: true,
            layout_pending_open: None,
            layout_pending_save_as: None,
            pending_save: None,
            scancode_map_status: None,
            scancode_map_last_message: None,
            startup_diagnostics: Vec::new(),
        }
    }

    /// `SettingsApp::tab_layout` を丸ごと（ツールバー行のボタン・凡例・グリッド・
    /// セル選択後の編集パネル込みで）GPU/ウィンドウ無しで実行し、実機で
    /// 「プレビュー押したら無言のまま強制終了」した現象と同じコードパスを再現
    /// する（egui::Grid の panic 修正が有効であることの回帰テスト）。
    #[test]
    fn full_tab_layout_render_with_real_config_does_not_panic() {
        let config_path = find_config_path();
        let config = awase::config::AppConfig::load(&config_path).unwrap_or_else(|e| {
            panic!(
                "テスト前提: {} の読み込みに失敗した: {e}",
                config_path.display()
            )
        });
        assert_eq!(
            config.general.layouts_dir, "layout",
            "テストがリポジトリ実物の config.toml を読めていない可能性"
        );

        let mut app = test_settings_app(config);
        assert!(
            app.layout_file_path.is_some(),
            "実際の layout/nicola.yab のロードに失敗した"
        );

        let ctx = eframe::egui::Context::default();
        let _ = ctx.run(eframe::egui::RawInput::default(), |ctx| {
            eframe::egui::CentralPanel::default().show(ctx, |ui| {
                app.tab_layout(ui);
            });
        });

        // セルを選択した状態（編集パネル描画）も再現する。
        app.select_layout_cell(PhysicalPos::new(0, 0));
        let _ = ctx.run(eframe::egui::RawInput::default(), |ctx| {
            eframe::egui::CentralPanel::default().show(ctx, |ui| {
                app.tab_layout(ui);
            });
        });
    }

    /// `tab_keys`（無変換/変換の条件付き indent ブロックと「awase → IME
    /// ON/OFFキー」を含む、折りたたみ「上級者向け設定」小見出しは2026-08-15に
    /// 撤去済み——2026-08-26に`Tab::Advanced`タブへ付けた同名ラベルとは無関係の、
    /// `tab_keys`内の別機能）
    /// がパニックしないことを固定する
    /// （`full_tab_layout_render_with_real_config_does_not_panic` と同じ
    /// パターン）。無変換/変換を親指キーへ割り当て、`tab_keys` 内の条件付き
    /// indent ブロック（無変換/変換オプション）も一通り描画させる。
    #[test]
    fn full_tab_keys_render_does_not_panic() {
        let config_path = find_config_path();
        let mut config = awase::config::AppConfig::load(&config_path).unwrap_or_else(|e| {
            panic!(
                "テスト前提: {} の読み込みに失敗した: {e}",
                config_path.display()
            )
        });
        config.general.left_thumb_key = "無変換".to_string();
        config.general.right_thumb_key = "変換".to_string();

        let mut app = test_settings_app(config);

        let ctx = eframe::egui::Context::default();
        let _ = ctx.run(eframe::egui::RawInput::default(), |ctx| {
            eframe::egui::CentralPanel::default().show(ctx, |ui| {
                app.tab_keys(ui);
            });
        });
    }

    /// `tab_basic`/`tab_keymap`/`tab_disable_apps`/`tab_app_rules`/
    /// `tab_advanced` がパニックしないことを固定する（2026-08-15、ホバー
    /// ヒント拡充で全タブに手を入れたため追加。`tab_disable_apps` は
    /// 2026-08-26 BUG-90 で `tab_app_rules` から切り出した際に追加。
    /// `full_tab_layout_render_with_real_config_does_not_panic` と同じ
    /// パターン）。`tab_keymap`は既存ルールが無いと空一覧の分岐しか通らない
    /// ため、ダミーの `KeymapRule` を1件足して非空分岐（`main_key_combo`/
    /// `main_key_combo_optional` を含む行）も描画させる。
    #[test]
    fn remaining_tabs_render_does_not_panic() {
        let config_path = find_config_path();
        let mut config = awase::config::AppConfig::load(&config_path).unwrap_or_else(|e| {
            panic!(
                "テスト前提: {} の読み込みに失敗した: {e}",
                config_path.display()
            )
        });
        config.keymaps.push(awase::config::KeymapRule {
            app: Some("vim.exe".to_string()),
            from: "Ctrl+I".to_string(),
            to: Some("VK_F7".to_string()),
        });
        config.post_bypass.push(awase::config::PostBypassRule {
            key: "Ctrl+B".to_string(),
            process: String::new(),
            class: String::new(),
        });

        let mut app = test_settings_app(config);

        let ctx = eframe::egui::Context::default();
        let _ = ctx.run(eframe::egui::RawInput::default(), |ctx| {
            eframe::egui::CentralPanel::default().show(ctx, |ui| {
                app.tab_basic(ui);
            });
        });
        let _ = ctx.run(eframe::egui::RawInput::default(), |ctx| {
            eframe::egui::CentralPanel::default().show(ctx, |ui| {
                app.tab_keymap(ui);
            });
        });
        let _ = ctx.run(eframe::egui::RawInput::default(), |ctx| {
            eframe::egui::CentralPanel::default().show(ctx, |ui| {
                app.tab_disable_apps(ui);
            });
        });
        let _ = ctx.run(eframe::egui::RawInput::default(), |ctx| {
            eframe::egui::CentralPanel::default().show(ctx, |ui| {
                app.tab_app_rules(ui);
            });
        });
        let _ = ctx.run(eframe::egui::RawInput::default(), |ctx| {
            eframe::egui::CentralPanel::default().show(ctx, |ui| {
                app.tab_advanced(ui);
            });
        });
    }

    #[test]
    fn apply_layout_edit_rejects_non_jis_keystroke() {
        let config: awase::config::AppConfig = toml::from_str("[general]").unwrap();
        let mut app = test_settings_app(config);
        app.layout_selected_pos = Some(PhysicalPos::new(0, 0));
        app.layout_edit_kind = ValueKind::Keystroke;
        app.layout_edit_value = "あ".to_string();
        app.apply_layout_edit();
        assert!(
            !app.layout_modified,
            "JIS キーボードに存在しない文字が適用されてしまった"
        );
    }

    #[test]
    fn apply_layout_edit_classifies_alphabetic_as_romaji_and_symbol_as_key_sequence() {
        let config: awase::config::AppConfig = toml::from_str("[general]").unwrap();
        let mut app = test_settings_app(config);

        app.layout_selected_pos = Some(PhysicalPos::new(0, 0));
        app.layout_edit_kind = ValueKind::Keystroke;
        app.layout_edit_value = "ka".to_string();
        app.apply_layout_edit();
        assert!(matches!(
            app.layout_face(Face::Normal).get(&PhysicalPos::new(0, 0)),
            Some(YabValue::Romaji { .. })
        ));

        app.layout_selected_pos = Some(PhysicalPos::new(0, 1));
        app.layout_edit_kind = ValueKind::Keystroke;
        app.layout_edit_value = "!".to_string();
        app.apply_layout_edit();
        assert!(matches!(
            app.layout_face(Face::Normal).get(&PhysicalPos::new(0, 1)),
            Some(YabValue::KeySequence(_))
        ));
    }

    #[test]
    fn apply_layout_edit_normalizes_fullwidth_keystroke_input() {
        // IME で入力すると全角のまま打ちがちなので、入力側が半角/全角を
        // 意識しなくていいように自動変換されることを確認する。
        let config: awase::config::AppConfig = toml::from_str("[general]").unwrap();
        let mut app = test_settings_app(config);

        // 全角ローマ字 "ｋａ" → 半角化されて "ka" → Romaji として分類される。
        app.layout_selected_pos = Some(PhysicalPos::new(0, 0));
        app.layout_edit_kind = ValueKind::Keystroke;
        app.layout_edit_value = "\u{FF4B}\u{FF41}".to_string();
        app.apply_layout_edit();
        assert!(matches!(
            app.layout_face(Face::Normal).get(&PhysicalPos::new(0, 0)),
            Some(YabValue::Romaji { romaji, .. }) if romaji == "ka"
        ));

        // 全角記号 "！" → 半角化されて "!" → KeySequence として分類され、
        // JIS キーボード外の文字として拒否されない。
        app.layout_selected_pos = Some(PhysicalPos::new(0, 1));
        app.layout_edit_kind = ValueKind::Keystroke;
        app.layout_edit_value = "\u{FF01}".to_string();
        app.apply_layout_edit();
        assert!(matches!(
            app.layout_face(Face::Normal).get(&PhysicalPos::new(0, 1)),
            Some(YabValue::KeySequence(s)) if s == "!"
        ));
    }

    /// セルを編集 → 実際に `layout_write_to_path` で .yab ファイルへ保存 →
    /// 別途パースし直して値が正しく往復することを確認する。`apply_layout_edit`
    /// が正しく分類していても、`YabLayout::serialize` / `YabValue::parse` 側の
    /// 実装と噛み合っていなければファイルには正しく書き出せないため、
    /// メモリ上の分類テストとは別に実ファイル I/O を通す。
    #[test]
    fn edited_cells_round_trip_through_actual_yab_file() {
        let config: awase::config::AppConfig = toml::from_str("[general]").unwrap();
        let mut app = test_settings_app(config);

        let edits = [
            (PhysicalPos::new(0, 0), ValueKind::Keystroke, "ka"),
            // 全角のまま入力しても正規化されて保存されることも兼ねて確認する。
            (PhysicalPos::new(0, 1), ValueKind::Keystroke, "\u{FF01}"), // ！→ !
            (PhysicalPos::new(0, 2), ValueKind::Literal, "\u{30fc}"),   // ー
            (PhysicalPos::new(0, 3), ValueKind::Special, ""),
        ];
        for (pos, kind, value) in edits {
            app.layout_selected_pos = Some(pos);
            app.layout_edit_kind = kind;
            app.layout_edit_value = value.to_string();
            if kind == ValueKind::Special {
                app.layout_edit_special_idx = 0; // Backspace
            }
            app.apply_layout_edit();
        }

        let tmp_dir = std::env::temp_dir();
        let path = tmp_dir.join(format!(
            "awase_settings_roundtrip_test_{}.yab",
            std::process::id()
        ));
        app.layout_write_to_path(&path);
        assert!(!app.layout_modified, "保存後も変更ありのままになっている");

        let content = std::fs::read_to_string(&path).expect("保存したファイルを読み戻せない");
        let _ = std::fs::remove_file(&path);
        let reparsed = awase::yab::YabLayout::parse(&content, app.config.general.keyboard_model)
            .expect("保存した .yab の再パースに失敗した")
            .resolve_kana();

        assert!(
            matches!(
                reparsed.normal.get(&PhysicalPos::new(0, 0)),
                Some(YabValue::Romaji { romaji, .. }) if romaji == "ka"
            ),
            "ローマ字が正しく往復しなかった: {:?}",
            reparsed.normal.get(&PhysicalPos::new(0, 0))
        );
        assert!(
            matches!(
                reparsed.normal.get(&PhysicalPos::new(0, 1)),
                Some(YabValue::KeySequence(s)) if s == "!"
            ),
            "全角記号が正規化された上でキーシーケンスとして往復しなかった: {:?}",
            reparsed.normal.get(&PhysicalPos::new(0, 1))
        );
        assert!(
            matches!(
                reparsed.normal.get(&PhysicalPos::new(0, 2)),
                Some(YabValue::Literal(s)) if s == "\u{30fc}"
            ),
            "リテラルが正しく往復しなかった: {:?}",
            reparsed.normal.get(&PhysicalPos::new(0, 2))
        );
        assert!(
            matches!(
                reparsed.normal.get(&PhysicalPos::new(0, 3)),
                Some(YabValue::Special(awase::types::SpecialKey::Backspace))
            ),
            "特殊キーが正しく往復しなかった: {:?}",
            reparsed.normal.get(&PhysicalPos::new(0, 3))
        );
    }

    #[test]
    fn copy_then_paste_duplicates_cell_exactly_including_kana() {
        let config: awase::config::AppConfig = toml::from_str("[general]").unwrap();
        let mut app = test_settings_app(config);

        // コピー元セルにローマ字を設定する（かな解決も含めて複製できるかを見る）。
        app.layout_selected_pos = Some(PhysicalPos::new(0, 0));
        app.layout_edit_kind = ValueKind::Keystroke;
        app.layout_edit_value = "ka".to_string();
        app.apply_layout_edit();
        let original = app
            .layout_face(Face::Normal)
            .get(&PhysicalPos::new(0, 0))
            .cloned();

        app.layout_selected_pos = Some(PhysicalPos::new(0, 0));
        app.copy_layout_cell();
        assert_eq!(app.layout_clipboard_history.first().cloned(), original);

        // 貼り付けは面をまたいでも動く。
        app.layout_current_face = Face::LeftThumb;
        app.layout_selected_pos = Some(PhysicalPos::new(1, 2));
        let clipped = app.layout_clipboard_history[0].clone();
        app.paste_layout_cell(clipped);

        let pasted = app
            .layout_face(Face::LeftThumb)
            .get(&PhysicalPos::new(1, 2))
            .cloned();
        assert_eq!(
            original, pasted,
            "貼り付け後の値がコピー元と完全に一致しない（かな解決結果含む）"
        );
        assert!(app.layout_modified);

        // 編集パネルの表示も貼り付け後の値に更新されている。
        assert_eq!(app.layout_edit_kind, ValueKind::Keystroke);
        assert_eq!(app.layout_edit_value, "ka");
    }

    #[test]
    fn copy_history_holds_multiple_independent_entries_most_recent_first() {
        let config: awase::config::AppConfig = toml::from_str("[general]").unwrap();
        let mut app = test_settings_app(config);

        // CLIPBOARD_HISTORY_LEN 件、別々の値をコピーする。
        for i in 0..CLIPBOARD_HISTORY_LEN {
            #[expect(clippy::cast_possible_truncation)]
            let pos = PhysicalPos::new(0, i as u8);
            app.layout_selected_pos = Some(pos);
            app.layout_edit_kind = ValueKind::Special;
            // Special キーの種類を毎回変えて区別できるようにする。
            app.layout_edit_special_idx = i % SPECIAL_KEYS.len();
            app.apply_layout_edit();
            app.copy_layout_cell();
        }

        // 履歴は最大件数ぶん、最後にコピーしたものが先頭に来る（全件の並びを検証）。
        assert_eq!(app.layout_clipboard_history.len(), CLIPBOARD_HISTORY_LEN);
        for (history_idx, entry) in app.layout_clipboard_history.iter().enumerate() {
            // i=CLIPBOARD_HISTORY_LEN-1 が最後にコピーされたので history[0] に来る
            // → history_idx と i は逆順で対応する。
            let expected_i = CLIPBOARD_HISTORY_LEN - 1 - history_idx;
            let expected = SPECIAL_KEYS[expected_i % SPECIAL_KEYS.len()].0;
            assert!(
                matches!(entry, YabValue::Special(sk) if *sk == expected),
                "history[{history_idx}] が想定と異なる: {entry:?}"
            );
        }

        // 履歴の2番目の項目を貼り付けると、その値だけが使われる。
        let second = app.layout_clipboard_history[1].clone();
        app.layout_selected_pos = Some(PhysicalPos::new(3, 0));
        app.paste_layout_cell(second.clone());
        assert_eq!(
            app.layout_face(Face::Normal)
                .get(&PhysicalPos::new(3, 0))
                .cloned(),
            Some(second)
        );
    }

    #[test]
    fn copying_same_value_again_moves_it_to_front_without_duplicating() {
        let config: awase::config::AppConfig = toml::from_str("[general]").unwrap();
        let mut app = test_settings_app(config);

        app.layout_selected_pos = Some(PhysicalPos::new(0, 0));
        app.layout_edit_kind = ValueKind::Special;
        app.layout_edit_special_idx = 0; // Backspace
        app.apply_layout_edit();
        app.copy_layout_cell();

        app.layout_selected_pos = Some(PhysicalPos::new(0, 1));
        app.layout_edit_kind = ValueKind::Special;
        app.layout_edit_special_idx = 1; // Escape
        app.apply_layout_edit();
        app.copy_layout_cell();

        // Backspace を再度コピーすると、重複せず先頭に移動するだけ。
        app.layout_selected_pos = Some(PhysicalPos::new(0, 0));
        app.copy_layout_cell();

        assert_eq!(app.layout_clipboard_history.len(), 2);
        assert!(matches!(
            &app.layout_clipboard_history[0],
            YabValue::Special(awase::types::SpecialKey::Backspace)
        ));
        // Escape は追い出されず2番目に残っている。
        assert!(matches!(
            &app.layout_clipboard_history[1],
            YabValue::Special(awase::types::SpecialKey::Escape)
        ));
    }

    #[test]
    fn copying_beyond_history_capacity_drops_the_oldest_entry() {
        let config: awase::config::AppConfig = toml::from_str("[general]").unwrap();
        let mut app = test_settings_app(config);

        // CLIPBOARD_HISTORY_LEN を超える数、別々の値をコピーする
        // （SPECIAL_KEYS は5種類あり CLIPBOARD_HISTORY_LEN(4) より多いので
        // すべて別の値になる）。
        let extra = 2;
        for i in 0..(CLIPBOARD_HISTORY_LEN + extra) {
            #[expect(clippy::cast_possible_truncation)]
            let pos = PhysicalPos::new(0, i as u8);
            app.layout_selected_pos = Some(pos);
            app.layout_edit_kind = ValueKind::Special;
            app.layout_edit_special_idx = i % SPECIAL_KEYS.len();
            app.apply_layout_edit();
            app.copy_layout_cell();
        }

        assert_eq!(app.layout_clipboard_history.len(), CLIPBOARD_HISTORY_LEN);
        // 最古（i=0, i=1）は追い出され、最新 CLIPBOARD_HISTORY_LEN 件だけが残る。
        let oldest_surviving_i = extra; // i=0..extra は捨てられた
        for (history_idx, entry) in app.layout_clipboard_history.iter().enumerate() {
            let expected_i = CLIPBOARD_HISTORY_LEN + extra - 1 - history_idx;
            assert!(
                expected_i >= oldest_surviving_i,
                "捨てられたはずの古い項目が残っている: history[{history_idx}]"
            );
            let expected = SPECIAL_KEYS[expected_i % SPECIAL_KEYS.len()].0;
            assert!(
                matches!(entry, YabValue::Special(sk) if *sk == expected),
                "history[{history_idx}] が想定と異なる: {entry:?}"
            );
        }
    }

    #[test]
    fn copy_and_paste_are_noop_without_a_selected_cell() {
        let config: awase::config::AppConfig = toml::from_str("[general]").unwrap();
        let mut app = test_settings_app(config);
        // YabLayout は PartialEq を実装していないため、シリアライズした
        // テキストの一致で「変更されていない」ことを確認する。
        let model = app.config.general.keyboard_model;
        let serialized_before = app.layout.serialize(model);

        app.layout_selected_pos = None;
        app.copy_layout_cell();
        assert!(
            app.layout_clipboard_history.is_empty(),
            "選択セルが無いのに履歴へコピーされてしまった"
        );

        app.paste_layout_cell(YabValue::Literal("x".to_string()));
        assert_eq!(
            app.layout.serialize(model),
            serialized_before,
            "選択セルが無いのにレイアウトが変更されてしまった"
        );
        assert!(
            !app.layout_modified,
            "選択セルが無いのに modified フラグが立ってしまった"
        );
    }

    // ── ADR-099 決定4b/4c: config_load_state の遷移・保存前バックアップ ──
    // GUI 起動なしのロジック単体テスト。apply()/apply_confirmed() は
    // send_reload_config_message() を呼ぶが、#[cfg(target_os = "windows")]
    // ガード済みで Linux では no-op のため直接呼んでよい。

    use awase::config::ConfigLoadState;

    /// round2 指摘 P3: `{:p}` によるスタックアドレス由来の一時パスは
    /// `--test-threads=1` 実行時に全テストが同一パスを共有しうる。
    /// テストごとに一意になるよう、プロセス内で単調増加するカウンタを
    /// 使う。
    fn unique_test_id() -> usize {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        COUNTER.fetch_add(1, Ordering::Relaxed)
    }

    fn dangerous_app() -> (SettingsApp, std::path::PathBuf) {
        let config: awase::config::AppConfig = toml::from_str("[general]").unwrap();
        let mut app = test_settings_app(config);
        let config_path = std::env::temp_dir().join(format!(
            "awase_test_dangerous_save_{}_{}.toml",
            std::process::id(),
            unique_test_id()
        ));
        app.config_path = config_path.clone();
        app.config_load_state = ConfigLoadState::Dangerous("broken toml".to_string());
        (app, config_path)
    }

    /// `apply_confirmed()` はバックグラウンドスレッドへ保存処理を委譲する
    /// ため（コードレビュー指摘: UI スレッドを最大200msブロックしていた
    /// 同期呼び出しを解消）、テストからは完了をポーリングして待つ。
    fn wait_for_pending_save(app: &mut SettingsApp) {
        let ctx = eframe::egui::Context::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while app.pending_save.is_some() {
            assert!(
                std::time::Instant::now() < deadline,
                "pending_save did not complete within 5s"
            );
            app.poll_pending_save(&ctx);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    /// `apply()` は `Dangerous` の間、無条件保存せず確認モーダルを開くだけで
    /// ファイルへは書き込まない（round1 M4: 静かなデフォルト永続化を防ぐ）。
    #[test]
    fn apply_does_not_save_immediately_when_dangerous() {
        let (mut app, config_path) = dangerous_app();
        let _ = std::fs::remove_file(&config_path);

        app.apply();

        assert!(
            !config_path.exists(),
            "Dangerous 状態での apply() は確認前に保存してはならない"
        );
        assert!(app.show_dangerous_save_confirm);
        assert_eq!(
            app.config_load_state,
            ConfigLoadState::Dangerous("broken toml".to_string())
        );
    }

    /// 確認後（`apply_confirmed()`）に保存すると `config_load_state` が
    /// `Loaded` へ遷移する（round1 M4: フラグの寿命）。
    #[test]
    fn apply_confirmed_transitions_to_loaded_after_successful_save() {
        let (mut app, config_path) = dangerous_app();
        let _ = std::fs::remove_file(&config_path);

        app.apply_confirmed();
        wait_for_pending_save(&mut app);
        let _ = std::fs::remove_file(&config_path);
        let bak_path = config_path.with_extension("toml.bak");
        let _ = std::fs::remove_file(&bak_path);

        assert_eq!(app.config_load_state, ConfigLoadState::Loaded);
    }

    /// Dangerous 状態での保存前に、既存ファイルが一度だけ `.bak` へ退避される
    /// こと。2回目の Dangerous な保存では既存の `.bak` を上書きしない
    /// （round1 M4 / round2 SF-3: 壊れた元ファイルのバックアップを保護する）。
    #[test]
    fn apply_confirmed_backs_up_existing_file_only_once() {
        let (mut app, config_path) = dangerous_app();
        std::fs::write(&config_path, "original broken content").unwrap();
        let bak_path = config_path.with_extension("toml.bak");
        let _ = std::fs::remove_file(&bak_path);

        app.apply_confirmed();
        wait_for_pending_save(&mut app);
        let bak_after_first = std::fs::read_to_string(&bak_path).unwrap();
        assert_eq!(bak_after_first, "original broken content");

        // 2回目: config_load_state を再び Dangerous にして保存しても、
        // 既に存在する .bak は上書きされない。
        app.config_load_state = ConfigLoadState::Dangerous("still broken".to_string());
        app.apply_confirmed();
        wait_for_pending_save(&mut app);
        let bak_after_second = std::fs::read_to_string(&bak_path).unwrap();

        let _ = std::fs::remove_file(&config_path);
        let _ = std::fs::remove_file(&bak_path);

        assert_eq!(
            bak_after_second, "original broken content",
            ".bak は最初の異常発生時点のものを保持し続けるべき"
        );
    }

    /// /code-review指摘（PR #127）: apply_confirmed()はvalidate()の戻り値
    /// （confirm_mode="speculative"→two_phase正規化を含む）を警告文の表示
    /// にしか使わず、保存対象は未検証のself.configのcloneのままだった。
    /// 設定画面がconfirm_modeを一切表示しなくなったため、この正規化を
    /// ユーザーが手で直す手段が無く、警告が「適用」を押すたび永遠に
    /// 再表示され続けるバグになっていた。正規化後の値がファイルへ保存され、
    /// かつ self.config にも反映されることを確認する。
    #[test]
    fn apply_confirmed_persists_normalized_confirm_mode_not_raw_config() {
        let config: awase::config::AppConfig = toml::from_str(
            r#"
[general]
confirm_mode = "speculative"
speculative_delay_ms = 30
"#,
        )
        .unwrap();
        assert_eq!(
            config.general.confirm_mode,
            awase::config::ConfirmMode::Speculative,
            "sanity: toml側の記述が期待通りspeculativeとしてパースされているか"
        );
        let mut app = test_settings_app(config);
        let config_path = std::env::temp_dir().join(format!(
            "awase_test_normalize_persist_{}_{}.toml",
            std::process::id(),
            unique_test_id()
        ));
        app.config_path = config_path.clone();

        app.apply_confirmed();
        wait_for_pending_save(&mut app);

        let saved = std::fs::read_to_string(&config_path).unwrap();
        let _ = std::fs::remove_file(&config_path);

        assert!(
            saved.contains(r#"confirm_mode = "two_phase""#),
            "保存されたファイルはconfirm_mode正規化後(two_phase)であるべき: {saved}"
        );
        assert!(
            !saved.contains(r#"confirm_mode = "speculative""#),
            "保存されたファイルに廃止済みのconfirm_mode=\"speculative\"が\
             残ってはいけない: {saved}"
        );
        assert_eq!(
            app.config.general.confirm_mode,
            awase::config::ConfirmMode::TwoPhase,
            "self.configも正規化後の値へ更新されるべき（次回のApplyで同じ警告が\
             永遠に再表示されるのを防ぐため）"
        );
        assert_eq!(
            app.status.contains("speculative"),
            true,
            "1回目のApplyでは廃止警告が表示されるべき: {}",
            app.status
        );
    }

    /// コードレビュー指摘 C2 の回帰テスト: バックアップへのコピーが失敗した
    /// 場合、保存を中止し元ファイルを上書きしない（安全側に倒す）。
    /// `config_path` を読み取り不可（`chmod 0o000`）にすることで
    /// `fs::copy` の読み取り側を確実に失敗させる。これは実際に C2 が
    /// 問題にした状況（`Dangerous` を招いた `PermissionDenied` が、
    /// バックアップ用の読み取りも同様に失敗させる）そのものの再現でも
    /// ある。root 実行では `chmod 0o000` でも読めてしまうため unix限定
    /// テストとし、root では意味を持たない前提を明示する。
    #[cfg(unix)]
    #[test]
    fn apply_confirmed_aborts_save_when_backup_fails() {
        use std::os::unix::fs::PermissionsExt;

        let (mut app, config_path) = dangerous_app();
        std::fs::write(&config_path, "original broken content").unwrap();
        std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o000)).unwrap();
        let bak_path = config_path.with_extension("toml.bak");
        let _ = std::fs::remove_file(&bak_path);

        app.apply_confirmed();
        // バックグラウンドスレッドが 0o000 のまま `fs::copy` を試みて
        // 失敗することを確認したいので、権限を戻すのは完了待ちの後にする。
        wait_for_pending_save(&mut app);

        // 検証のため読み取り権限を戻してから内容を確認する。
        std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let content_after = std::fs::read_to_string(&config_path).unwrap();
        let bak_exists = bak_path.exists();

        let _ = std::fs::remove_file(&config_path);
        let _ = std::fs::remove_file(&bak_path);

        assert_eq!(
            content_after, "original broken content",
            "バックアップに失敗した場合、元ファイルを上書きしてはならない"
        );
        assert!(
            !bak_exists,
            "失敗したバックアップが中途半端に残ってはならない"
        );
        assert!(app.status.contains("保存失敗"));
        assert_eq!(
            app.config_load_state,
            ConfigLoadState::Dangerous("broken toml".to_string()),
            "バックアップ失敗時は Dangerous のまま維持されるべき"
        );
    }

    /// round2 指摘 P4 の回帰テスト: `Loaded`/`NotFound` のときは `.bak` を
    /// 作らない（バックアップは `Dangerous` 限定）。
    #[test]
    fn apply_does_not_create_backup_when_not_dangerous() {
        let (mut app, config_path) = dangerous_app();
        std::fs::write(&config_path, "some content").unwrap();
        let bak_path = config_path.with_extension("toml.bak");
        let _ = std::fs::remove_file(&bak_path);
        app.config_load_state = ConfigLoadState::Loaded;

        app.apply();
        wait_for_pending_save(&mut app);

        let bak_exists = bak_path.exists();
        let _ = std::fs::remove_file(&config_path);
        let _ = std::fs::remove_file(&bak_path);

        assert!(
            !bak_exists,
            "Loaded 状態での保存で .bak が作られてはならない"
        );
    }

    /// `Loaded` 状態（通常の保存）では、確認モーダルを経由せず保存を開始する
    /// （実際の書き込みはバックグラウンドスレッドで非同期に完了する）。
    #[test]
    fn apply_saves_immediately_when_not_dangerous() {
        let (mut app, config_path) = dangerous_app();
        let _ = std::fs::remove_file(&config_path);
        app.config_load_state = ConfigLoadState::Loaded;

        app.apply();
        wait_for_pending_save(&mut app);

        assert!(
            config_path.exists(),
            "Loaded 状態では保存が完了しているべき"
        );
        assert!(!app.show_dangerous_save_confirm);
        let _ = std::fs::remove_file(&config_path);
    }

    /// コードレビュー指摘の回帰テスト: `apply_confirmed()` は保存の完了を
    /// 待たずに即座に返る（UI スレッドをブロックしない）。宛先を既存
    /// ディレクトリにして `rename` を確実に失敗させ、内部の50ms×最大4回
    /// リトライ（最大200ms）が `apply_confirmed()` の呼び出し自体を
    /// ブロックしていないことを、呼び出しの所要時間で確認する。
    #[test]
    fn apply_confirmed_returns_without_blocking_on_slow_save() {
        let (mut app, _config_path) = dangerous_app();
        app.config_load_state = ConfigLoadState::Loaded;
        // 宛先をディレクトリにして rename を確実に失敗させ、内部リトライ
        // ループ（最大200ms）を必ず踏ませる。
        let dir_as_dest = std::env::temp_dir().join(format!(
            "awase_test_apply_confirmed_nonblocking_{}_{}",
            std::process::id(),
            unique_test_id()
        ));
        std::fs::create_dir(&dir_as_dest).unwrap();
        app.config_path = dir_as_dest.clone();

        let start = std::time::Instant::now();
        app.apply_confirmed();
        let elapsed = start.elapsed();

        wait_for_pending_save(&mut app);
        let _ = std::fs::remove_dir_all(&dir_as_dest);

        assert!(
            elapsed < std::time::Duration::from_millis(50),
            "apply_confirmed() should return immediately (spawn only), took {elapsed:?}"
        );
        assert!(
            app.status.contains("保存失敗"),
            "the background save must still surface its failure once polled: {}",
            app.status
        );
    }

    /// コードレビュー指摘の回帰テスト: 保存が進行中（`pending_save` が
    /// `Some`）の間に `apply_confirmed()` を再度呼んでも、ボタン自体は
    /// 無効化していないため連打しうる。多重起動はしない（既存仕様）が、
    /// 単に無言で無視するのではなく「保存中」であることをステータスで
    /// 示すべき（コードレビュー指摘: 以前は無反応に見えるだけだった）。
    #[test]
    fn apply_confirmed_shows_status_when_save_already_in_progress() {
        let (mut app, _config_path) = dangerous_app();
        app.config_load_state = ConfigLoadState::Loaded;
        // 宛先をディレクトリにして保存を必ず失敗させ、内部リトライ
        // ループ（最大200ms）の間 `pending_save` が `Some` であり続ける
        // ことを保証する（通常の保存はごく短時間で完了しうるため、
        // 2回目の apply_confirmed() 呼び出しが確実に「進行中」の状態を
        // 観測できるよう、意図的に遅延させる）。
        let dir_as_dest = std::env::temp_dir().join(format!(
            "awase_test_apply_confirmed_double_click_{}_{}",
            std::process::id(),
            unique_test_id()
        ));
        std::fs::create_dir(&dir_as_dest).unwrap();
        app.config_path = dir_as_dest.clone();

        app.apply_confirmed();
        assert!(
            app.pending_save.is_some(),
            "save should still be in flight immediately after apply_confirmed()"
        );

        app.apply_confirmed();
        // `poll_pending_save()` を呼ぶ前（＝1回目の保存の完了通知で上書き
        // される前）に、2回目のクリックが残したステータスを確認する。
        assert!(
            app.status.contains("保存中"),
            "re-clicking while a save is in flight should say so, not silently no-op: {}",
            app.status
        );

        wait_for_pending_save(&mut app);
        let _ = std::fs::remove_dir_all(&dir_as_dest);
    }

    /// `cancel()` で再読み込みに成功したら `config_load_state` が `Loaded`
    /// へ戻る（round2 SF-4）。
    #[test]
    fn cancel_transitions_to_loaded_after_successful_reload() {
        let (mut app, config_path) = dangerous_app();
        std::fs::write(&config_path, "[general]\n").unwrap();

        app.cancel();
        let _ = std::fs::remove_file(&config_path);

        assert_eq!(app.config_load_state, ConfigLoadState::Loaded);
    }

    /// コードレビュー指摘: 確認モーダル表示中でも背景の「キャンセル」ボタンは
    /// 操作可能（モーダルは `egui::Modal` のようなブロッキングオーバーレイを
    /// 持たない）。この経路で `cancel()` が呼ばれた場合、`config_load_state`
    /// が正常化したのに `show_dangerous_save_confirm` だけが残留し、
    /// 解消済みの状態に対して古い警告モーダルが表示され続けてはならない。
    #[test]
    fn cancel_closes_dangerous_save_confirm_modal() {
        let (mut app, config_path) = dangerous_app();
        std::fs::write(&config_path, "[general]\n").unwrap();
        app.show_dangerous_save_confirm = true;

        app.cancel();
        let _ = std::fs::remove_file(&config_path);

        assert!(
            !app.show_dangerous_save_confirm,
            "cancel() はモーダルの前提(Dangerous)を解消するので、開いたままの確認モーダルも閉じるべき"
        );
    }
}
