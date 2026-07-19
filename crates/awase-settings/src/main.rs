#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::path::{Path, PathBuf};

use eframe::egui;

use awase::kana_table::KanaTable;
use awase::scanmap::PhysicalPos;
use awase::types::SpecialKey;
use awase::yab::{FullwidthStrExt as _, YabFace, YabLayout, YabValue};

/// 設定リロード用カスタムメッセージ ID（awase 本体側の `WM_APP + 10` と一致させる）
#[cfg(target_os = "windows")]
const WM_RELOAD_CONFIG: u32 = 0x8000 + 10; // WM_APP = 0x8000

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Basic,
    Keys,
    Keymap,
    // サイドパネルから外しているため未構築（今後の課題として実装は保持）。
    #[allow(dead_code)]
    AppRules,
    ImeDetect,
    Layout,
    Advanced,
}

/// 配列編集タブの4面。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Face {
    Normal,
    LeftThumb,
    RightThumb,
    Shift,
}

const FACES: [(Face, &str); 4] = [
    (Face::Normal, "通常面"),
    (Face::LeftThumb, "左親指シフト"),
    (Face::RightThumb, "右親指シフト"),
    (Face::Shift, "小指シフト"),
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

const SPECIAL_KEYS: [(SpecialKey, &str); 5] = [
    (SpecialKey::Backspace, "Backspace"),
    (SpecialKey::Escape, "Escape"),
    (SpecialKey::Enter, "Enter"),
    (SpecialKey::Space, "Space"),
    (SpecialKey::Delete, "Delete"),
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

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            // 幅 580: サイドパネル(100) + プレビューのキーボード図(13キー×34px+段差
            // インデント ≈ 464) + 余白。最も幅を要するタブがデフォルトで横スクロール
            // なしに収まる値（従来の 500 ではプレビュー右端が切れていた）。
            .with_inner_size([580.0, 650.0])
            // ウィンドウを小さくしても全項目にスクロール + 下部固定ボタンで届くため、
            // 低解像度・高 DPI ディスプレイでも操作不能にならない下限だけ設ける。
            .with_min_inner_size([420.0, 320.0])
            .with_title("awase 設定"),
        ..Default::default()
    };
    eframe::run_native(
        "awase-settings",
        options,
        Box::new(|cc| Ok(Box::new(SettingsApp::new(cc)))),
    )
}

/// 各 bool は無関係な由来（keymap キャプチャの修飾キー3つ、配列編集タブの
/// dirty フラグ1つ）を持つ独立したフラグであり、bitflags 化や enum への統合は
/// 可読性を下げるだけなので許容する。
#[expect(clippy::struct_excessive_bools)]
struct SettingsApp {
    config: awase::config::AppConfig,
    config_path: std::path::PathBuf,
    status: String,
    active_tab: Tab,
    available_layouts: Vec<String>,
    // Key list add-buffers
    new_engine_on_key: String,
    new_engine_off_key: String,
    new_ime_on_key: String,
    new_ime_off_key: String,
    new_ime_toggle_key: String,
    new_ime_detect_on_key: String,
    new_ime_detect_off_key: String,
    // Keymap rule add-buffers
    new_keymap_app: String,
    new_keymap_from_ctrl: bool,
    new_keymap_from_shift: bool,
    new_keymap_from_alt: bool,
    new_keymap_from_main: String,
    new_keymap_to_main: String,
    // Keymap capture mode (None = not capturing)
    capturing: Option<CaptureTarget>,
    // アプリ別タブ add-buffers: (process, class) × force_text/force_bypass/force_vk/force_tsf
    new_override_bufs: [(String, String); 4],
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
}

impl SettingsApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        setup_fonts(&cc.egui_ctx);
        let config_path = find_config_path();
        let config = match awase::config::AppConfig::load(&config_path) {
            Ok(cfg) => cfg,
            Err(e) => {
                log::warn!("Config load failed: {e}, using defaults");
                default_config()
            }
        };
        let available_layouts = scan_layout_names(&config.general.layouts_dir);

        Self {
            config,
            config_path,
            status: String::new(),
            active_tab: Tab::Basic,
            available_layouts,
            new_engine_on_key: String::new(),
            new_engine_off_key: String::new(),
            new_ime_on_key: String::new(),
            new_ime_off_key: String::new(),
            new_ime_toggle_key: String::new(),
            new_ime_detect_on_key: String::new(),
            new_ime_detect_off_key: String::new(),
            new_keymap_app: String::new(),
            new_keymap_from_ctrl: false,
            new_keymap_from_shift: false,
            new_keymap_from_alt: false,
            new_keymap_from_main: String::new(),
            new_keymap_to_main: String::new(),
            capturing: None,
            new_override_bufs: Default::default(),
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
        }
    }

    fn apply(&mut self) {
        let clone = self.config.clone();
        let (_validated, warnings) = clone.validate();
        if !warnings.is_empty() {
            self.status = format!("警告: {}", warnings.join("; "));
        }
        match self.config.save(&self.config_path) {
            Ok(()) => {
                if warnings.is_empty() {
                    self.status = "設定を保存しました".to_string();
                }
                send_reload_config_message();
            }
            Err(e) => self.status = format!("保存失敗: {e}"),
        }
    }

    fn cancel(&mut self) {
        match awase::config::AppConfig::load(&self.config_path) {
            Ok(cfg) => {
                self.available_layouts = scan_layout_names(&cfg.general.layouts_dir);
                self.config = cfg;
                self.status = "変更を破棄しました".to_string();
            }
            Err(e) => self.status = format!("読み込み失敗: {e}"),
        }
    }

    // ── 配列編集タブ（旧 awase-yab-editor）──

    const fn layout_face_mut(&mut self, face: Face) -> &mut YabFace {
        match face {
            Face::Normal => &mut self.layout.normal,
            Face::LeftThumb => &mut self.layout.left_thumb,
            Face::RightThumb => &mut self.layout.right_thumb,
            Face::Shift => &mut self.layout.shift,
        }
    }

    const fn layout_face(&self, face: Face) -> &YabFace {
        match face {
            Face::Normal => &self.layout.normal,
            Face::LeftThumb => &self.layout.left_thumb,
            Face::RightThumb => &self.layout.right_thumb,
            Face::Shift => &self.layout.shift,
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
            Some(YabValue::None) | None => {
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
                self.layout_status = format!("{} に保存しました", path.display());
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
            Ok(ly) => {
                self.layout = ly;
                self.layout_file_path_buf = path.display().to_string();
                self.layout_file_path = Some(path.to_path_buf());
                self.layout_modified = false;
                self.layout_selected_pos = None;
                self.layout_status = format!("{} を読み込みました", path.display());
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
            Ok(ly) => {
                self.layout = ly;
                self.layout_modified = false;
                self.layout_selected_pos = None;
                self.layout_status = format!("{} を再読み込みしました", path.display());
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
                match target {
                    CaptureTarget::ExistingFrom(i) => {
                        if let Some(rule) = self.config.keymaps.get_mut(i) {
                            rule.from = format_combo(ctrl, shift, alt, &internal);
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
                        self.new_keymap_from_alt = alt;
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
    fn tab_basic(&mut self, ui: &mut egui::Ui) {
        ui.heading("基本設定");
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            ui.label("同時打鍵閾値:").on_hover_text("同時打鍵と判定する時間の幅です。\n大きいほど判定が甘く(親指シフトが入りやすく)なりますが、遅延が増えます。\n100ms が NICOLA 規格の標準値です。");
            ui.add(
                egui::Slider::new(&mut self.config.general.simultaneous_threshold_ms, 10..=500)
                    .suffix(" ms"),
            );
        });
        ui.horizontal(|ui| {
            ui.label("確定モード:").on_hover_text("文字の確定方法を選びます。\nモードごとに速度と正確さのバランスが異なります。");
            egui::ComboBox::from_id_salt("confirm_mode")
                .selected_text(confirm_mode_label(self.config.general.confirm_mode))
                .show_ui(ui, |ui| {
                    use awase::config::ConfirmMode;
                    ui.selectable_value(&mut self.config.general.confirm_mode, ConfirmMode::Wait, "待機 (wait)")
                        .on_hover_text("タイムアウトまで出力を保留します。\n最も正確ですが、入力に少し遅延を感じます。");
                    ui.selectable_value(&mut self.config.general.confirm_mode, ConfirmMode::Speculative, "先行確定 (speculative)")
                        .on_hover_text("即座に出力し、親指シフトと判定されたら差し替えます。\n高速ですが、まれに画面がちらつきます。");
                    ui.selectable_value(&mut self.config.general.confirm_mode, ConfirmMode::TwoPhase, "二段タイマー (two_phase)")
                        .on_hover_text("短い待機の後に投機出力します。\nwait と speculative の中間的な動作です。");
                    ui.selectable_value(&mut self.config.general.confirm_mode, ConfirmMode::AdaptiveTiming, "適応タイミング (adaptive_timing)")
                        .on_hover_text("連続入力中は待機、途切れたら投機出力します。\nタイピング速度に自動適応します。");
                    ui.selectable_value(&mut self.config.general.confirm_mode, ConfirmMode::NgramPredictive, "n-gram 予測 (ngram_predictive)")
                        .on_hover_text("統計データで次の文字を予測し、判定を最適化します。\nn-gram ファイル未指定時は二段タイマーとして動作します。");
                });
        });
        ui.label(confirm_mode_tooltip(self.config.general.confirm_mode));
        let spec_enabled = matches!(
            self.config.general.confirm_mode,
            awase::config::ConfirmMode::TwoPhase
                | awase::config::ConfirmMode::AdaptiveTiming
                | awase::config::ConfirmMode::NgramPredictive
        );
        ui.add_enabled_ui(spec_enabled, |ui| {
            ui.horizontal(|ui| {
                ui.label("投機出力待機:").on_hover_text("投機出力までの待機時間です。\n短いほど応答が速くなりますが、誤判定が増えます。\nTwoPhase/AdaptiveTiming と、NgramPredictive のフォールバック動作で使用されます。");
                ui.add(
                    egui::Slider::new(&mut self.config.general.speculative_delay_ms, 0..=100)
                        .suffix(" ms"),
                );
            });
        });
        ui.label("出力方式: アプリごとに最適な注入方式を自動選択します（設定不要）");
        let mut auto_start_checked = self.config.general.auto_start == "enabled";
        if ui.checkbox(&mut auto_start_checked, "自動起動").on_hover_text("Windows ログオン時に自動的に awase を起動します。\nタスクスケジューラに登録されます。").changed() {
            self.config.general.auto_start = if auto_start_checked {
                "enabled"
            } else {
                "disabled"
            }
            .to_string();
        }
        ui.horizontal(|ui| {
            ui.label("キーボード配列:").on_hover_text(
                "物理キーボードの配列です。\n\
                 JIS: 無変換/変換キーが物理的に存在する日本語キーボード。\n\
                 US: ANSI 104キー配列。無変換/変換キーが無いため、\n\
                 親指キーとホットキーを別途 US 向けに変更する必要があります。",
            );
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
                });
            // US → JIS への切替時、Space/Left Alt/Right Alt 等 US 向けに変更していた
            // 親指キーが JIS では使えない（Space は単独タップの意味が変わり、Alt は
            // なりすまし設定自体が無意味になる）まま残ってしまうのを防ぐため、
            // 既定値（無変換/変換）へ強制的に戻す。ユーザーが手動で戻す手間を省く。
            if prev_keyboard_model != awase::scanmap::KeyboardModel::Jis
                && self.config.general.keyboard_model == awase::scanmap::KeyboardModel::Jis
            {
                self.config.general.left_thumb_key = "無変換".to_string();
                self.config.general.right_thumb_key = "変換".to_string();
            }
            // JIS → US への切替時、エンジンON/OFF・IME ON/OFF・単独5連打OFF の既定値
            // （Ctrl+Shift+変換 等）は US に無変換/変換キーが物理的に存在しないため
            // 動作しない。動かない既定値を黙って残すより、未設定にして
            // 「キー設定」タブで明示的に選んでもらう方が誠実（他アプリのショートカット
            // と衝突しない US 向け「正解」の組み合わせを勝手に決め打ちできないため）。
            if prev_keyboard_model != awase::scanmap::KeyboardModel::Us
                && self.config.general.keyboard_model == awase::scanmap::KeyboardModel::Us
            {
                self.config.keys.engine_on.clear();
                self.config.keys.engine_off.clear();
                self.config.keys.ime_on.clear();
                self.config.keys.ime_off.clear();
                self.config.keys.engine_off_solo_triple = None;
            }
        });
        if self.config.general.keyboard_model == awase::scanmap::KeyboardModel::Us {
            ui.label(
                "  US 配列では既定の親指キー(無変換/変換)とホットキーが使えません。\n\
                 下の「レイアウト」で nicola_us.yab を選び、キー設定タブで\n\
                 親指キーを変更してください（Ctrl/Win は OS 予約修飾キーのため\n\
                 使用不可・同時打鍵検出自体が機能しません。プログラマブルキーボードで\n\
                 F13-F24 等へ物理リマップするか、Space を検討してください。\n\
                 キー設定タブの候補にある「Left Alt」「Right Alt」を選ぶと、\n\
                 エンジン ON 時のみ Alt キーを親指キーとして使えます）。\n\
                 \n\
                 エンジン ON/OFF・IME ON/OFF・単独5連打OFF のホットキーも未設定に\n\
                 なっています（無変換/変換前提の既定値は US では動かないため）。\n\
                 「キー設定」タブで、動作する物理キーの組み合わせを設定してください。\n\
                 単独5連打OFF は、親指キーとして設定した物理キー自体を指定してください\n\
                 （それ以外のキーを指定しても発火しません）。",
            );
        }
        ui.horizontal(|ui| {
            ui.label("レイアウト:").on_hover_text("使用する配列定義ファイルを選びます。\nlayout フォルダ内の .yab ファイルが表示されます。");
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
                });
            if ui.button("再スキャン").clicked() {
                self.available_layouts = scan_layout_names(&self.config.general.layouts_dir);
            }
        });
    }

    fn tab_keys(&mut self, ui: &mut egui::Ui) {
        ui.heading("キー設定");
        ui.add_space(4.0);

        // Thumb keys
        ui.label("親指キー");
        ui.horizontal(|ui| {
            ui.label("  左親指:").on_hover_text(
                "左の親指シフトキーに使うキーです。通常は「無変換」キーを使います。\n\
                 「Left Alt」を選ぶと、物理 Left Alt キーをエンジン ON 時に限り\n\
                 左親指キーとして使います（OFF 時は通常の Alt として機能し、\n\
                 Alt+Tab 等を損ないません。PowerToys 等の OS レベルキーリマップと\n\
                 同様の効果を awase 単体で実現する機能です）。",
            );
            thumb_key_combo(
                ui,
                "left_thumb_key",
                &mut self.config.general.left_thumb_key,
            );
        });
        ui.horizontal(|ui| {
            ui.label("  右親指:").on_hover_text(
                "右の親指シフトキーに使うキーです。通常は「変換」キーを使います。\n\
                 「Right Alt」を選ぶと、物理 Right Alt キーをエンジン ON 時に限り\n\
                 右親指キーとして使います（詳細は左親指のヒントを参照）。",
            );
            thumb_key_combo(
                ui,
                "right_thumb_key",
                &mut self.config.general.right_thumb_key,
            );
        });
        ui.add_space(8.0);

        // Engine on/off
        ui.label("エンジン制御");
        key_list_ui(
            ui,
            "エンジン ON",
            "eng_on",
            &mut self.config.keys.engine_on,
            &mut self.new_engine_on_key,
            "エンジンを ON にするキーの組み合わせです。\n複数登録できます。",
        );
        key_list_ui(
            ui,
            "エンジン OFF",
            "eng_off",
            &mut self.config.keys.engine_off,
            &mut self.new_engine_off_key,
            "エンジンを OFF にするキーの組み合わせです。\n複数登録できます。",
        );
        ui.horizontal(|ui| {
            ui.label("  単独5連打で OFF:").on_hover_text(
                "指定キーを単独で素早く5回連続押下するとエンジンを OFF にします。\nCtrl スタック等で通常のキー操作が効かなくなった際の緊急脱出用です。",
            );
            solo_triple_combo(ui, &mut self.config.keys.engine_off_solo_triple);
        });
        ui.add_space(8.0);

        // IME on/off
        ui.label("IME 制御");
        key_list_ui(
            ui,
            "IME ON",
            "ime_on",
            &mut self.config.keys.ime_on,
            &mut self.new_ime_on_key,
            "IME を ON にするキーの組み合わせです。\nIME がオフの状態からオンに切り替えます。",
        );
        key_list_ui(
            ui,
            "IME OFF",
            "ime_off",
            &mut self.config.keys.ime_off,
            &mut self.new_ime_off_key,
            "IME を OFF にするキーの組み合わせです。\nIME がオンの状態からオフに切り替えます。",
        );
        ui.add_space(8.0);

        // Toggle hotkey
        ui.label("トグルホットキー");
        ui.horizontal(|ui| {
            ui.label("  エンジン切替:").on_hover_text(
                "エンジンの ON/OFF をトグルするホットキーです。\nシステム全体で有効です。",
            );
            let hotkey = self
                .config
                .general
                .engine_toggle_hotkey
                .get_or_insert_with(String::new);
            ui.text_edit_singleline(hotkey);
        });
    }

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
                        .changed()
                    {
                        rule.app = if app_buf.is_empty() {
                            None
                        } else {
                            Some(app_buf)
                        };
                    }

                    // from: modifiers + main key + capture button
                    let (mut ctrl, mut shift, mut alt, mut main) = parse_combo_str(&rule.from);
                    let mut changed = false;
                    changed |= ui.checkbox(&mut ctrl, "Ctrl").changed();
                    changed |= ui.checkbox(&mut shift, "Shift").changed();
                    changed |= ui.checkbox(&mut alt, "Alt").changed();
                    if main_key_combo(ui, &format!("from_main_{i}"), &mut main) {
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
                    if main_key_combo_optional(ui, &format!("to_main_{i}"), &mut to_main) {
                        rule.to = if to_main.is_empty() {
                            None
                        } else {
                            Some(to_main)
                        };
                    }
                    let to_target = CaptureTarget::ExistingTo(i);
                    capture_button(ui, &mut capturing, to_target);

                    if ui.small_button("x").clicked() {
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
                );
                ui.end_row();

                ui.label("  from:");
                ui.horizontal_wrapped(|ui| {
                    ui.checkbox(&mut self.new_keymap_from_ctrl, "Ctrl");
                    ui.checkbox(&mut self.new_keymap_from_shift, "Shift");
                    ui.checkbox(&mut self.new_keymap_from_alt, "Alt");
                    main_key_combo(ui, "new_from_main", &mut self.new_keymap_from_main);
                    capture_button(ui, &mut capturing, CaptureTarget::NewFrom);
                });
                ui.end_row();

                ui.label("  to:")
                    .on_hover_text("再注入するキー。「（消費のみ）」を選ぶとキーを消費するだけ。");
                ui.horizontal_wrapped(|ui| {
                    main_key_combo_optional(ui, "new_to_main", &mut self.new_keymap_to_main);
                    capture_button(ui, &mut capturing, CaptureTarget::NewTo);
                });
                ui.end_row();
            });
        self.capturing = capturing;
        if ui.button("+追加").clicked() && !self.new_keymap_from_main.is_empty() {
            let from = format_combo(
                self.new_keymap_from_ctrl,
                self.new_keymap_from_shift,
                self.new_keymap_from_alt,
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
            self.new_keymap_from_alt = false;
            self.new_keymap_from_main.clear();
            self.new_keymap_to_main.clear();
        }
    }

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
                if ui.small_button("x").clicked() {
                    rm = Some(i);
                }
            });
        }
        if let Some(i) = rm {
            self.config.post_bypass.remove(i);
        }
        ui.horizontal(|ui| {
            ui.label("Ctrl+");
            main_key_combo(ui, "new_pb_key", &mut self.new_pb_key);
            ui.add(
                egui::TextEdit::singleline(&mut self.new_pb_process)
                    .desired_width(120.0)
                    .hint_text("プロセス名 (部分一致)"),
            );
            ui.add(
                egui::TextEdit::singleline(&mut self.new_pb_class)
                    .desired_width(120.0)
                    .hint_text("クラス名 (部分一致)"),
            );
            if ui.button("+追加").clicked() && !self.new_pb_key.is_empty() {
                // ランタイムの parse は "Ctrl+<キー>" 形式（Ctrl 必須）を要求する
                self.config.post_bypass.push(awase::config::PostBypassRule {
                    key: format_combo(true, false, false, &std::mem::take(&mut self.new_pb_key)),
                    process: std::mem::take(&mut self.new_pb_process),
                    class: std::mem::take(&mut self.new_pb_class),
                });
            }
        });
    }

    fn tab_ime_detect(&mut self, ui: &mut egui::Ui) {
        ui.heading("IME 検出");
        ui.label("IME の ON/OFF 切り替えを検出するためのキー設定です。\n通常はデフォルトのままで問題ありません。\n半角/全角キーなど、IME を切り替えるキーを登録します。");
        ui.add_space(8.0);

        key_list_ui(
            ui,
            "トグルキー（ON/OFF 切替）",
            "ime_det_toggle",
            &mut self.config.keys.ime_detect.toggle,
            &mut self.new_ime_toggle_key,
            "IME の ON/OFF をトグルするキーです。\n押すたびに ON/OFF が切り替わります。\n例: 半角/全角キー",
        );
        key_list_ui(
            ui,
            "ON キー（IME を ON にする）",
            "ime_det_on",
            &mut self.config.keys.ime_detect.on,
            &mut self.new_ime_detect_on_key,
            "IME を ON にするキーです。\n押すと必ず ON になります。",
        );
        key_list_ui(
            ui,
            "OFF キー（IME を OFF にする）",
            "ime_det_off",
            &mut self.config.keys.ime_detect.off,
            &mut self.new_ime_detect_off_key,
            "IME を OFF にするキーです。\n押すと必ず OFF になります。",
        );

        ui.add_space(8.0);
        if ui.button("デフォルトに戻す").clicked() {
            self.config.keys.ime_detect = awase::config::ImeDetectConfig::default();
        }
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
            if ui.button("開く").clicked() {
                self.layout_do_open_dialog();
            }
            if ui.button("保存").clicked() {
                self.layout_do_save();
            }
            if ui.button("名前を付けて保存").clicked() {
                self.layout_do_save_as_dialog();
            }
            if ui.button("再読み込み").clicked() {
                self.layout_do_reload();
            }
        });
        ui.horizontal(|ui| {
            ui.label("パス:");
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.layout_file_path_buf).desired_width(300.0),
            );
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
                    "配列のキーボード配列（JIS/US）は「基本設定」タブの設定に従います。",
                );
            if !self.layout_status.is_empty() {
                ui.separator();
                ui.label(&self.layout_status);
            }
        });
        ui.add_space(8.0);

        // 面タブ
        ui.horizontal(|ui| {
            for (face, label) in &FACES {
                let is_active = self.layout_current_face == *face;
                let btn_text = if is_active {
                    egui::RichText::new(*label).strong()
                } else {
                    egui::RichText::new(*label)
                };
                if ui.selectable_label(is_active, btn_text).clicked() {
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
        egui::ScrollArea::vertical().show(ui, |ui| {
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
                        egui::Stroke::new(2.5, egui::Color32::from_rgb(30, 100, 220))
                    } else {
                        egui::Stroke::new(1.0, egui::Color32::from_rgb(160, 160, 160))
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
                .on_hover_text("Backspace / Escape / Enter / Space / Delete を送信します。");
            ui.radio_value(&mut self.layout_edit_kind, ValueKind::None, "なし")
                .on_hover_text("このキーへの割り当てを解除します（パススルー）。");
        });
        ui.add_space(4.0);

        // Value input
        match self.layout_edit_kind {
            ValueKind::Keystroke => {
                ui.horizontal(|ui| {
                    ui.label("打鍵:");
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut self.layout_edit_value)
                            .desired_width(120.0)
                            .hint_text("例: ka, si, tsu, !, 1"),
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
                    ui.label("文字:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.layout_edit_value)
                            .desired_width(120.0)
                            .hint_text("例: ー、…"),
                    );
                });
                ui.label(
                    egui::RichText::new("※ Unicode 文字をそのまま送信します")
                        .small()
                        .color(egui::Color32::GRAY),
                );
            }
            ValueKind::Special => {
                ui.horizontal(|ui| {
                    ui.label("特殊キー:");
                    egui::ComboBox::from_id_salt("special_key")
                        .selected_text(SPECIAL_KEYS[self.layout_edit_special_idx].1)
                        .show_ui(ui, |ui| {
                            for (i, (_, name)) in SPECIAL_KEYS.iter().enumerate() {
                                ui.selectable_value(&mut self.layout_edit_special_idx, i, *name);
                            }
                        });
                });
            }
            ValueKind::None => {
                ui.label(
                    egui::RichText::new("このキーへの割り当てを解除します")
                        .color(egui::Color32::GRAY),
                );
            }
        }

        let can_apply = self.layout_edit_kind != ValueKind::Keystroke
            || find_invalid_keystroke_char(self.layout_edit_value.trim()).is_none();

        ui.add_space(6.0);
        if ui
            .add_enabled(
                can_apply,
                egui::Button::new(egui::RichText::new("適用").strong()),
            )
            .clicked()
        {
            self.apply_layout_edit();
        }
    }

    fn tab_advanced(&mut self, ui: &mut egui::Ui) {
        ui.heading("詳細設定");
        ui.add_space(4.0);
        let slider_with_tip = |ui: &mut egui::Ui,
                               label: &str,
                               tip: &str,
                               val: &mut u32,
                               range: std::ops::RangeInclusive<u32>| {
            ui.horizontal(|ui| {
                ui.label(label).on_hover_text(tip);
                ui.add(egui::Slider::new(val, range).suffix(" ms"));
            });
        };
        let ngram_enabled = matches!(
            self.config.general.confirm_mode,
            awase::config::ConfirmMode::NgramPredictive
        );
        if !ngram_enabled {
            ui.label(
                "n-gram 設定は確定モードが「n-gram 予測」のときのみ使用されます（基本設定タブ）",
            );
        }
        ui.add_enabled_ui(ngram_enabled, |ui| {
        ui.horizontal(|ui| {
            ui.label("n-gram ファイル:").on_hover_text("n-gram 統計データファイルのパスです。\n.csv.gz または .toml 形式に対応しています。\nngram_predictive モードで使用されます。");
            let mut buf = self.config.general.ngram_file.clone().unwrap_or_default();
            if ui.text_edit_singleline(&mut buf).changed() {
                self.config.general.ngram_file = if buf.is_empty() { None } else { Some(buf) };
            }
        });
        slider_with_tip(
            ui,
            "n-gram 調整幅:",
            "n-gram 予測による閾値調整の幅です。\n大きいほど予測の影響が強くなります。",
            &mut self.config.general.ngram_adjustment_range_ms,
            0..=100,
        );
        slider_with_tip(
            ui,
            "n-gram 最小閾値:",
            "n-gram 予測で調整される閾値の下限です。\nこれより短い閾値にはなりません。",
            &mut self.config.general.ngram_min_threshold_ms,
            10..=200,
        );
        slider_with_tip(
            ui,
            "n-gram 最大閾値:",
            "n-gram 予測で調整される閾値の上限です。\nこれより長い閾値にはなりません。",
            &mut self.config.general.ngram_max_threshold_ms,
            50..=500,
        );
        });
        ui.add_space(8.0);
        slider_with_tip(
            ui,
            "フォーカスデバウンス:",
            "フォーカス切り替え時のデバウンス時間です。\nAlt+Tab などでフォーカスが連続変更される際の誤検知を防ぎます。",
            &mut self.config.general.focus_debounce_ms,
            0..=200,
        );
        slider_with_tip(
            ui,
            "IME ポーリング間隔:",
            "IME 状態のポーリング間隔です。\nマウスで言語バーを操作した場合などの検出用です。\n小さいほどレスポンスが良くなりますが、CPU 負荷が増えます。",
            &mut self.config.general.ime_poll_interval_ms,
            100..=5000,
        );
        ui.horizontal(|ui| {
            ui.label("レイアウトディレクトリ:")
                .on_hover_text("配列定義ファイル (.yab) を格納するフォルダです。");
            ui.text_edit_singleline(&mut self.config.general.layouts_dir);
        });
    }
}

// ── eframe::App ──

impl eframe::App for SettingsApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
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

        // Side panel for tab selection
        egui::SidePanel::left("tab_panel")
            .resizable(false)
            .default_width(100.0)
            .show(ctx, |ui| {
                ui.add_space(8.0);
                // 「アプリ別」(AppRules) は高度な機能のため GUI 化を見送り、
                // config.toml の直接編集（app_overrides / post_bypass）に委ねている。
                // tab_app_rules の実装自体は残してある。
                //
                // 「配列編集」(Layout) は 2026-07-06 に「配列プレビューの実装が
                // まだ固まっていない」として一旦非表示にしていたが、layouts_dir の
                // パス解決バグ修正を経て再表示した。その後、独立バイナリだった
                // awase-yab-editor を統合し、プレビューではなく実際に編集できる
                // タブにした（バイナリを分ける価値は無いという判断）。
                for (tab, label) in [
                    (Tab::Basic, "基本設定"),
                    (Tab::Keys, "キー設定"),
                    (Tab::Keymap, "ショートカット"),
                    (Tab::ImeDetect, "IME 検出"),
                    (Tab::Layout, "配列編集"),
                    (Tab::Advanced, "詳細設定"),
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
                    Tab::AppRules => self.tab_app_rules(ui),
                    Tab::ImeDetect => self.tab_ime_detect(ui),
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
            if ui.small_button("x").clicked() {
                rm = Some(i);
            }
        });
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
        );
        ui.add(
            egui::TextEdit::singleline(&mut buf.1)
                .desired_width(200.0)
                .hint_text("クラス名 (完全一致)")
                .id(egui::Id::new(format!("{id}_class"))),
        );
        if ui.button("+追加").clicked() && !buf.0.is_empty() && !buf.1.is_empty() {
            entries.push(awase::config::AppOverrideEntry {
                process: std::mem::take(&mut buf.0),
                class: std::mem::take(&mut buf.1),
            });
        }
    });
    ui.add_space(8.0);
}

/// `engine_off_solo_triple`（単独5連打でエンジン OFF にするキー）の選択 UI。
fn solo_triple_combo(ui: &mut egui::Ui, current: &mut Option<String>) {
    let display = current.as_deref().map_or_else(
        || "（無効）".to_string(),
        |v| {
            THUMB_KEY_OPTIONS
                .iter()
                .find(|(_, internal)| *internal == v)
                .map_or_else(|| v.to_string(), |(d, _)| (*d).to_string())
        },
    );
    egui::ComboBox::from_id_salt("engine_off_solo_triple")
        .selected_text(display)
        .width(110.0)
        .show_ui(ui, |ui| {
            if ui.selectable_label(current.is_none(), "（無効）").clicked() {
                *current = None;
            }
            for (label, internal) in THUMB_KEY_OPTIONS {
                if ui
                    .selectable_label(current.as_deref() == Some(*internal), *label)
                    .clicked()
                {
                    *current = Some((*internal).to_string());
                }
            }
        });
}

fn key_list_ui(
    ui: &mut egui::Ui,
    label: &str,
    id: &str,
    keys: &mut Vec<String>,
    buf: &mut String,
    tooltip: &str,
) {
    ui.label(format!("  {label}:")).on_hover_text(tooltip);
    let mut rm = None;
    for (i, key) in keys.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.label(format!("    {key}"));
            if ui.small_button("x").clicked() {
                rm = Some(i);
            }
        });
    }
    if let Some(i) = rm {
        keys.remove(i);
    }
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(buf)
                .desired_width(180.0)
                .id(egui::Id::new(id)),
        );
        if ui.button("+追加").clicked() && !buf.is_empty() {
            keys.push(buf.clone());
            buf.clear();
        }
    });
}

/// 親指キー選択用候補一覧（表示名, config 内部表記）。
///
/// F13-F24: 物理キーとしては存在しない拡張ファンクションキー。プログラマブル
/// キーボード（QMK/ZMK 等）で親指位置のキーに割り当てて使う想定。US 配列で
/// 無変換/変換キーが無い場合の代替はこちらの範囲を推奨する。
///
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
/// この制約を回避するため使用可能。単独5連打エンジンOFF（`solo_triple_combo`）
/// 等、`THUMB_KEY_OPTIONS` を共有する他の用途には Alt を出さないよう分離してある。
const THUMB_KEY_OPTIONS: &[(&str, &str)] = &[
    ("Space", "VK_SPACE"),
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
    ("F21", "VK_F21"),
    ("F22", "VK_F22"),
    ("F23", "VK_F23"),
    ("F24", "VK_F24"),
];

/// 左親指/右親指キーの候補にのみ追加する、Alt なりすまし用エントリ。
///
/// 内部表記 `"Left Alt"`/`"Right Alt"` は VK 名ではなく、`hook.rs::resolve_thumb_key`
/// が特別に解釈する指示文字列。物理 Left/Right Alt キーをエンジン ON 時に限り
/// 親指キー（無変換/変換相当）として扱う（`config.rs` の `GeneralConfig::keyboard_model`
/// doc・`THUMB_KEY_OPTIONS` doc 参照）。`solo_triple_combo` 等、`THUMB_KEY_OPTIONS` を
/// 共有する他の用途には出さないため、意図的に別の定数に分離してある。
const ALT_IMPERSONATION_OPTIONS: &[(&str, &str)] = &[("Left Alt", "Left Alt"), ("Right Alt", "Right Alt")];

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

const fn confirm_mode_label(mode: awase::config::ConfirmMode) -> &'static str {
    use awase::config::ConfirmMode;
    match mode {
        ConfirmMode::Wait => "待機 (wait)",
        ConfirmMode::Speculative => "先行確定 (speculative)",
        ConfirmMode::TwoPhase => "二段タイマー (two_phase)",
        ConfirmMode::AdaptiveTiming => "適応タイミング (adaptive_timing)",
        ConfirmMode::NgramPredictive => "n-gram 予測 (ngram_predictive)",
    }
}

const fn confirm_mode_tooltip(mode: awase::config::ConfirmMode) -> &'static str {
    use awase::config::ConfirmMode;
    match mode {
        ConfirmMode::Wait => "  タイムアウトまで出力を保留。最も正確だが遅延あり。",
        ConfirmMode::Speculative => "  即座に出力し、同時打鍵時に差し替え。高速だが一瞬ちらつく。",
        ConfirmMode::TwoPhase => "  短い待機後に投機出力。wait と speculative の中間。",
        ConfirmMode::AdaptiveTiming => {
            "  連続打鍵中は wait、途切れたら投機。タイピング速度に適応。"
        }
        ConfirmMode::NgramPredictive => {
            "  n-gram 統計で投機/待機を動的判断。モデル未指定時は二段タイマー動作。"
        }
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
fn thumb_key_combo(ui: &mut egui::Ui, id: &str, current: &mut String) -> bool {
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
        });
    changed
}

/// main key ドロップダウン（必須選択版）。変更時は true を返す。
fn main_key_combo(ui: &mut egui::Ui, id: &str, current: &mut String) -> bool {
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
        });
    changed
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
fn main_key_combo_optional(ui: &mut egui::Ui, id: &str, current: &mut String) -> bool {
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
        });
    changed
}

// ── 配列編集タブ ヘルパー（旧 awase-yab-editor）──

fn color_legend(ui: &mut egui::Ui, color: egui::Color32, label: &str) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 2.0, color);
    ui.painter().rect_stroke(
        rect,
        2.0,
        egui::Stroke::new(1.0, egui::Color32::GRAY),
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
        Some(YabValue::None) | None => "\u{2014}".to_string(), // —
    }
}

const fn cell_color(value: Option<&YabValue>) -> egui::Color32 {
    match value {
        Some(YabValue::Romaji { .. }) => egui::Color32::from_rgb(255, 255, 255),
        Some(YabValue::Literal(_)) => egui::Color32::from_rgb(210, 230, 255),
        Some(YabValue::Special(_)) => egui::Color32::from_rgb(210, 255, 220),
        Some(YabValue::KeySequence(_)) => egui::Color32::from_rgb(200, 235, 255),
        Some(YabValue::None) | None => egui::Color32::from_rgb(220, 220, 220),
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
        Some(YabValue::None) | None => "割り当てなし".to_string(),
    }
}

fn cell_tooltip(value: Option<&YabValue>, pos: PhysicalPos) -> String {
    format!("({}, {})  {}", pos.row, pos.col, value_description(value))
}

fn load_yab_layout(path: &Path, model: awase::scanmap::KeyboardModel) -> Result<YabLayout, String> {
    let content = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    YabLayout::parse(&content, model)
        .map(YabLayout::resolve_kana)
        .map_err(|e| format!("パース失敗: {e}"))
}

fn empty_yab_layout() -> YabLayout {
    YabLayout {
        name: "untitled".to_string(),
        normal: YabFace::new(),
        left_thumb: YabFace::new(),
        right_thumb: YabFace::new(),
        shift: YabFace::new(),
    }
}

// ── Utility functions ──

fn find_config_path() -> std::path::PathBuf {
    awase::paths::resolve_relative_to_exe("config.toml")
}

/// `layouts_dir` を解決する。実行ファイル隣・`cargo run` 時のワークスペース
/// ルートのどちらでも動くよう、`awase::paths` の共通ロジックに委ねる（かつては
/// ここに exe 隣のみを見る独自ロジックがあり、`target` 配下から起動した際に
/// ワークスペースルート直下の `layout/` を見つけられなかった）。
fn resolve_layouts_dir(layouts_dir: &str) -> std::path::PathBuf {
    awase::paths::resolve_relative_to_exe(layouts_dir)
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

#[expect(clippy::missing_const_for_fn)]
fn send_reload_config_message() {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW};
        use windows::core::w;
        unsafe {
            let hwnd = FindWindowW(w!("awase_msg_window"), None);
            if let Ok(hwnd) = hwnd {
                let msg = windows::Win32::Foundation::WPARAM(0);
                let lparam = windows::Win32::Foundation::LPARAM(0);
                let _ = PostMessageW(hwnd, WM_RELOAD_CONFIG, msg, lparam);
            }
        }
    }
}

#[cfg(test)]
mod layout_tab_repro {
    use super::{
        CLIPBOARD_HISTORY_LEN, Face, KanaTable, PhysicalPos, SPECIAL_KEYS, SettingsApp, Tab,
        ValueKind, YabValue, empty_yab_layout, find_config_path, load_yab_layout,
        resolve_layouts_dir,
    };

    fn test_settings_app(config: awase::config::AppConfig) -> SettingsApp {
        let layout_path =
            resolve_layouts_dir(&config.general.layouts_dir).join(&config.general.default_layout);
        let layout = load_yab_layout(&layout_path, config.general.keyboard_model)
            .unwrap_or_else(|_| empty_yab_layout());
        SettingsApp {
            config,
            config_path: std::path::PathBuf::from("config.toml"),
            status: String::new(),
            active_tab: Tab::Layout,
            available_layouts: Vec::new(),
            new_engine_on_key: String::new(),
            new_engine_off_key: String::new(),
            new_ime_on_key: String::new(),
            new_ime_off_key: String::new(),
            new_ime_toggle_key: String::new(),
            new_ime_detect_on_key: String::new(),
            new_ime_detect_off_key: String::new(),
            new_keymap_app: String::new(),
            new_keymap_from_ctrl: false,
            new_keymap_from_shift: false,
            new_keymap_from_alt: false,
            new_keymap_from_main: String::new(),
            new_keymap_to_main: String::new(),
            capturing: None,
            new_override_bufs: <[(String, String); 4]>::default(),
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
}
