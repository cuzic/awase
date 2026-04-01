use std::collections::HashMap;
use std::path::{Path, PathBuf};

use eframe::egui;

use awase::kana_table::build_romaji_to_kana;
use awase::scanmap::{KeyboardModel, PhysicalPos};
use awase::types::SpecialKey;
use awase::yab::{YabFace, YabLayout, YabValue};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ValueKind {
    Romaji,
    Literal,
    KeySequence,
    Special,
    None,
}

const SPECIAL_KEYS: [(SpecialKey, &str); 5] = [
    (SpecialKey::Backspace, "Backspace"),
    (SpecialKey::Escape, "Escape"),
    (SpecialKey::Enter, "Enter"),
    (SpecialKey::Space, "Space"),
    (SpecialKey::Delete, "Delete"),
];

// ── Editor state ──

struct YabEditor {
    layout: YabLayout,
    model: KeyboardModel,
    current_face: Face,
    selected_pos: Option<PhysicalPos>,
    edit_kind: ValueKind,
    edit_value: String,
    edit_special_idx: usize,
    romaji_table: HashMap<String, char>,
    file_path: Option<PathBuf>,
    file_path_buf: String,
    modified: bool,
    status: String,
    /// Pending file path from async rfd dialog (open)
    pending_open: Option<PathBuf>,
    /// Pending file path from async rfd dialog (save-as)
    pending_save_as: Option<PathBuf>,
}

impl std::fmt::Debug for YabEditor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("YabEditor")
            .field("model", &self.model)
            .field("current_face", &self.current_face)
            .field("selected_pos", &self.selected_pos)
            .field("modified", &self.modified)
            .finish_non_exhaustive()
    }
}

impl YabEditor {
    #[allow(clippy::needless_pass_by_value)]
    fn new(cc: &eframe::CreationContext<'_>, file_path: Option<PathBuf>) -> Self {
        setup_fonts(&cc.egui_ctx);
        let model = KeyboardModel::Jis;
        #[allow(clippy::option_if_let_else)]
        let (layout, status, fp) = if let Some(ref path) = file_path {
            match load_layout(path, model) {
                Ok(ly) => (
                    ly,
                    format!("{} を読み込みました", path.display()),
                    file_path.clone(),
                ),
                Err(e) => (empty_layout(), format!("読み込み失敗: {e}"), None),
            }
        } else {
            (empty_layout(), "新規レイアウト".to_string(), None)
        };
        let path_str = fp
            .as_ref()
            .map_or_else(String::new, |p| p.display().to_string());
        Self {
            layout,
            model,
            current_face: Face::Normal,
            selected_pos: None,
            edit_kind: ValueKind::None,
            edit_value: String::new(),
            edit_special_idx: 0,
            romaji_table: build_romaji_to_kana(),
            file_path: fp,
            file_path_buf: path_str,
            modified: false,
            status,
            pending_open: None,
            pending_save_as: None,
        }
    }

    const fn face_mut(&mut self, face: Face) -> &mut YabFace {
        match face {
            Face::Normal => &mut self.layout.normal,
            Face::LeftThumb => &mut self.layout.left_thumb,
            Face::RightThumb => &mut self.layout.right_thumb,
            Face::Shift => &mut self.layout.shift,
        }
    }

    const fn face(&self, face: Face) -> &YabFace {
        match face {
            Face::Normal => &self.layout.normal,
            Face::LeftThumb => &self.layout.left_thumb,
            Face::RightThumb => &self.layout.right_thumb,
            Face::Shift => &self.layout.shift,
        }
    }

    fn select_cell(&mut self, pos: PhysicalPos) {
        self.selected_pos = Some(pos);
        let value = self.face(self.current_face).get(&pos).cloned();
        match value {
            Some(YabValue::Romaji { romaji, .. }) => {
                self.edit_kind = ValueKind::Romaji;
                self.edit_value = romaji;
            }
            Some(YabValue::Literal(s)) => {
                self.edit_kind = ValueKind::Literal;
                self.edit_value = s;
            }
            Some(YabValue::KeySequence(s)) => {
                self.edit_kind = ValueKind::KeySequence;
                self.edit_value = s;
            }
            Some(YabValue::Special(sk)) => {
                self.edit_kind = ValueKind::Special;
                self.edit_special_idx =
                    SPECIAL_KEYS.iter().position(|(k, _)| *k == sk).unwrap_or(0);
                self.edit_value.clear();
            }
            Some(YabValue::None) | None => {
                self.edit_kind = ValueKind::None;
                self.edit_value.clear();
            }
        }
    }

    fn apply_edit(&mut self) {
        let Some(pos) = self.selected_pos else { return };
        let value = match self.edit_kind {
            ValueKind::Romaji => {
                let romaji = self.edit_value.trim().to_string();
                if romaji.is_empty() {
                    YabValue::None
                } else {
                    let kana = self.romaji_table.get(&romaji).copied();
                    YabValue::Romaji { romaji, kana }
                }
            }
            ValueKind::Literal => {
                let s = self.edit_value.clone();
                if s.is_empty() {
                    YabValue::None
                } else {
                    YabValue::Literal(s)
                }
            }
            ValueKind::KeySequence => {
                let s = self.edit_value.clone();
                if s.is_empty() {
                    YabValue::None
                } else {
                    YabValue::KeySequence(s)
                }
            }
            ValueKind::Special => YabValue::Special(SPECIAL_KEYS[self.edit_special_idx].0),
            ValueKind::None => YabValue::None,
        };
        self.face_mut(self.current_face).insert(pos, value);
        self.modified = true;
        self.status = "変更あり".to_string();
    }

    fn do_save(&mut self) {
        let path = self.file_path.clone();
        match path {
            Some(p) => self.write_to_path(&p),
            None => self.do_save_as_dialog(),
        }
    }

    fn write_to_path(&mut self, path: &Path) {
        let text = self.layout.serialize(self.model);
        match std::fs::write(path, &text) {
            Ok(()) => {
                self.file_path = Some(path.to_path_buf());
                self.file_path_buf = path.display().to_string();
                self.modified = false;
                self.status = format!("{} に保存しました", path.display());
            }
            Err(e) => self.status = format!("保存失敗: {e}"),
        }
    }

    fn do_open_dialog(&mut self) {
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
            self.pending_open = maybe_path;
        }
    }

    fn do_save_as_dialog(&mut self) {
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
            self.pending_save_as = Some(path);
        }
    }

    fn load_from_path(&mut self, path: &Path) {
        match load_layout(path, self.model) {
            Ok(ly) => {
                self.layout = ly;
                self.file_path_buf = path.display().to_string();
                self.file_path = Some(path.to_path_buf());
                self.modified = false;
                self.selected_pos = None;
                self.status = format!("{} を読み込みました", path.display());
            }
            Err(e) => self.status = format!("読み込み失敗: {e}"),
        }
    }

    fn do_open_from_text_box(&mut self) {
        let path = PathBuf::from(&self.file_path_buf);
        self.load_from_path(&path);
    }

    fn do_reload(&mut self) {
        let Some(path) = self.file_path.clone() else {
            self.status = "ファイルパスが未設定です".to_string();
            return;
        };
        match load_layout(&path, self.model) {
            Ok(ly) => {
                self.layout = ly;
                self.modified = false;
                self.selected_pos = None;
                self.status = format!("{} を再読み込みしました", path.display());
            }
            Err(e) => self.status = format!("再読み込み失敗: {e}"),
        }
    }
}

// ── eframe::App ──

impl eframe::App for YabEditor {
    #[allow(clippy::too_many_lines)]
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Process pending async file dialog results
        if let Some(path) = self.pending_open.take() {
            self.load_from_path(&path);
        }
        if let Some(path) = self.pending_save_as.take() {
            self.write_to_path(&path);
        }

        // Keyboard shortcuts
        if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::S)) {
            self.do_save();
        }
        if ctx.input(|i| {
            i.modifiers.ctrl && !i.modifiers.shift && i.key_pressed(egui::Key::O)
        }) {
            self.do_open_dialog();
        }
        if ctx.input(|i| {
            i.modifiers.ctrl && i.modifiers.shift && i.key_pressed(egui::Key::S)
        }) {
            self.do_save_as_dialog();
        }
        if ctx.input(|i| i.key_pressed(egui::Key::F5)) {
            self.do_reload();
        }

        // ── ツールバー ──
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("開く").clicked() {
                    self.do_open_dialog();
                }
                if ui.button("保存").clicked() {
                    self.do_save();
                }
                if ui.button("名前を付けて保存").clicked() {
                    self.do_save_as_dialog();
                }
                if ui.button("再読み込み").clicked() {
                    self.do_reload();
                }
                ui.separator();
                ui.label("パス:");
                let resp = ui
                    .add(egui::TextEdit::singleline(&mut self.file_path_buf).desired_width(250.0));
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    self.do_open_from_text_box();
                }
                ui.separator();
                ui.label("モデル:");
                egui::ComboBox::from_id_salt("kb_model")
                    .selected_text(model_label(self.model))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.model, KeyboardModel::Jis, "JIS");
                        ui.selectable_value(&mut self.model, KeyboardModel::Us, "US");
                    });
            });
        });

        // ── ステータスバー ──
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let fname = self
                    .file_path
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .map_or("-", |n| n.to_str().unwrap_or("-"));
                ui.label(fname);
                ui.separator();
                ui.label(if self.modified {
                    egui::RichText::new("変更あり").color(egui::Color32::from_rgb(200, 80, 0))
                } else {
                    egui::RichText::new("保存済み").color(egui::Color32::from_rgb(0, 140, 0))
                });
                ui.separator();
                let rs = self.model.row_sizes();
                ui.label(format!(
                    "{} ({}-{}-{}-{})",
                    model_label(self.model),
                    rs[0],
                    rs[1],
                    rs[2],
                    rs[3]
                ));
                if !self.status.is_empty() {
                    ui.separator();
                    ui.label(&self.status);
                }
            });
        });

        // ── 中央パネル ──
        egui::CentralPanel::default().show(ctx, |ui| {
            // 面タブ
            ui.horizontal(|ui| {
                for (face, label) in &FACES {
                    let is_active = self.current_face == *face;
                    let btn_text = if is_active {
                        egui::RichText::new(*label).strong()
                    } else {
                        egui::RichText::new(*label)
                    };
                    if ui.selectable_label(is_active, btn_text).clicked() {
                        self.current_face = *face;
                        self.selected_pos = None;
                    }
                }
            });
            ui.separator();

            // 凡例
            ui.horizontal(|ui| {
                ui.label("凡例:");
                color_legend(ui, egui::Color32::from_rgb(255, 255, 255), "ローマ字");
                color_legend(ui, egui::Color32::from_rgb(210, 230, 255), "リテラル");
                color_legend(ui, egui::Color32::from_rgb(210, 255, 220), "特殊キー");
                color_legend(ui, egui::Color32::from_rgb(200, 235, 255), "キーシーケンス");
                color_legend(ui, egui::Color32::from_rgb(220, 220, 220), "なし");
            });
            ui.separator();

            egui::ScrollArea::vertical().show(ui, |ui| {
                self.draw_keyboard_grid(ui);
                ui.add_space(8.0);
                ui.separator();
                self.draw_edit_panel(ui);
            });
        });
    }
}

// ── 凡例ヘルパー ──

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

impl YabEditor {
    fn draw_keyboard_grid(&mut self, ui: &mut egui::Ui) {
        let row_sizes = self.model.row_sizes();
        let mut clicked_pos = None;

        // Row indents to simulate staggered keyboard layout
        let indents: [f32; 4] = [0.0, 14.0, 28.0, 42.0];

        for (row, &cols) in row_sizes.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.add_space(indents[row]);
                for col in 0..cols {
                    #[allow(clippy::cast_possible_truncation)]
                    let pos = PhysicalPos::new(row as u8, col as u8);
                    let value = self.face(self.current_face).get(&pos);
                    let is_selected = self.selected_pos == Some(pos);

                    let display = cell_display(value);
                    let bg_color = cell_color(value);
                    let stroke = if is_selected {
                        egui::Stroke::new(2.5, egui::Color32::from_rgb(30, 100, 220))
                    } else {
                        egui::Stroke::new(1.0, egui::Color32::from_rgb(160, 160, 160))
                    };

                    let tip = cell_tooltip(value, pos);
                    let btn = egui::Button::new(
                        egui::RichText::new(display).monospace().size(14.0),
                    )
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
            self.select_cell(pos);
        }
    }

    #[allow(clippy::too_many_lines)]
    fn draw_edit_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("編集パネル");
        let Some(pos) = self.selected_pos else {
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

        // Type selector (radio buttons)
        ui.horizontal(|ui| {
            ui.label("種別:");
            ui.radio_value(&mut self.edit_kind, ValueKind::Romaji, "ローマ字");
            ui.radio_value(&mut self.edit_kind, ValueKind::Literal, "リテラル");
            ui.radio_value(
                &mut self.edit_kind,
                ValueKind::KeySequence,
                "キーシーケンス",
            );
            ui.radio_value(&mut self.edit_kind, ValueKind::Special, "特殊キー");
            ui.radio_value(&mut self.edit_kind, ValueKind::None, "なし");
        });
        ui.add_space(4.0);

        // Value input
        match self.edit_kind {
            ValueKind::Romaji => {
                ui.horizontal(|ui| {
                    ui.label("ローマ字:");
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut self.edit_value)
                            .desired_width(120.0)
                            .hint_text("例: ka, si, tsu"),
                    );
                    if resp.changed() {
                        // Normalize: strip spaces, lowercase
                        self.edit_value = self.edit_value.trim().to_lowercase();
                    }
                });
                let preview: String = self
                    .romaji_table
                    .get(self.edit_value.trim())
                    .map_or_else(|| "（未対応）".to_string(), |c| {
                        let mut s = String::new();
                        s.push(*c);
                        s
                    });
                ui.horizontal(|ui| {
                    ui.label("かな変換:");
                    ui.label(
                        egui::RichText::new(&preview)
                            .size(22.0)
                            .strong()
                            .color(egui::Color32::from_rgb(0, 80, 160)),
                    );
                });
            }
            ValueKind::Literal => {
                ui.horizontal(|ui| {
                    ui.label("文字:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.edit_value)
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
            ValueKind::KeySequence => {
                ui.horizontal(|ui| {
                    ui.label("シーケンス:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.edit_value)
                            .desired_width(120.0)
                            .hint_text("例: ．"),
                    );
                });
                ui.label(
                    egui::RichText::new("※ IME がキーストロークとして処理します")
                        .small()
                        .color(egui::Color32::GRAY),
                );
            }
            ValueKind::Special => {
                ui.horizontal(|ui| {
                    ui.label("特殊キー:");
                    egui::ComboBox::from_id_salt("special_key")
                        .selected_text(SPECIAL_KEYS[self.edit_special_idx].1)
                        .show_ui(ui, |ui| {
                            for (i, (_, name)) in SPECIAL_KEYS.iter().enumerate() {
                                ui.selectable_value(&mut self.edit_special_idx, i, *name);
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

        ui.add_space(6.0);
        if ui
            .add(egui::Button::new(egui::RichText::new("適用").strong()))
            .clicked()
        {
            self.apply_edit();
        }
    }
}

// ── Helpers ──

fn cell_display(value: Option<&YabValue>) -> String {
    match value {
        Some(YabValue::Romaji { kana: Some(ch), .. }) => ch.to_string(),
        Some(YabValue::Romaji { romaji, .. }) if romaji.len() <= 3 => romaji.clone(),
        Some(YabValue::Romaji { romaji, .. }) => format!("{}.", &romaji[..2]),
        Some(YabValue::Literal(s) | YabValue::KeySequence(s))
            if s.chars().count() <= 2 =>
        {
            s.clone()
        }
        Some(YabValue::Literal(s) | YabValue::KeySequence(s)) => {
            s.chars().take(2).collect::<String>() + "."
        }
        Some(YabValue::Special(SpecialKey::Backspace)) => "\u{232b}".to_string(), // ⌫
        Some(YabValue::Special(SpecialKey::Enter)) => "\u{23ce}".to_string(),     // ⏎
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

fn cell_tooltip(value: Option<&YabValue>, pos: PhysicalPos) -> String {
    let kind_str = match value {
        Some(YabValue::Romaji { romaji, kana }) => {
            let kana_str = kana.map_or_else(|| "なし".to_string(), |c| {
                let mut s = String::new();
                s.push(c);
                s
            });
            format!("ローマ字: {romaji}  かな: {kana_str}")
        }
        Some(YabValue::Literal(s)) => format!("リテラル: {s}"),
        Some(YabValue::KeySequence(s)) => format!("キーシーケンス: {s}"),
        Some(YabValue::Special(sk)) => format!("特殊キー: {sk:?}"),
        Some(YabValue::None) | None => "割り当てなし".to_string(),
    };
    format!("({}, {})  {}", pos.row, pos.col, kind_str)
}

const fn model_label(model: KeyboardModel) -> &'static str {
    match model {
        KeyboardModel::Jis => "JIS",
        KeyboardModel::Us => "US",
    }
}

fn load_layout(path: &Path, model: KeyboardModel) -> Result<YabLayout, String> {
    let content = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    YabLayout::parse(&content, model)
        .map(YabLayout::resolve_kana)
        .map_err(|e| format!("パース失敗: {e}"))
}

fn empty_layout() -> YabLayout {
    YabLayout {
        name: "untitled".to_string(),
        normal: YabFace::new(),
        left_thumb: YabFace::new(),
        right_thumb: YabFace::new(),
        shift: YabFace::new(),
    }
}

fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    for path in &[
        "C:\\Windows\\Fonts\\meiryo.ttc",
        "C:\\Windows\\Fonts\\msgothic.ttc",
        "C:\\Windows\\Fonts\\YuGothR.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc",
    ] {
        if let Ok(font_data) = std::fs::read(path) {
            fonts.font_data.insert(
                "japanese".into(),
                egui::FontData::from_owned(font_data).into(),
            );
            if let Some(fam) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                fam.insert(0, "japanese".into());
            }
            if let Some(fam) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
                fam.insert(0, "japanese".into());
            }
            break;
        }
    }
    ctx.set_fonts(fonts);
}

fn main() -> eframe::Result<()> {
    env_logger::init();
    let file_path = std::env::args().nth(1).map(PathBuf::from);
    let title = file_path.as_ref().and_then(|p| p.file_name()).map_or_else(
        || "awase 配列エディタ".to_string(),
        |n| format!("awase 配列エディタ - {}", n.to_string_lossy()),
    );
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([780.0, 680.0])
            .with_title(&title),
        ..Default::default()
    };
    eframe::run_native(
        "awase-yab-editor",
        options,
        Box::new(move |cc| Ok(Box::new(YabEditor::new(cc, file_path)))),
    )
}
