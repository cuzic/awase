use std::collections::HashMap;
use std::path::PathBuf;

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
}

impl std::fmt::Debug for YabEditor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("YabEditor")
            .field("model", &self.model)
            .field("current_face", &self.current_face)
            .field("selected_pos", &self.selected_pos)
            .field("modified", &self.modified)
            .finish()
    }
}

impl YabEditor {
    fn new(cc: &eframe::CreationContext<'_>, file_path: Option<PathBuf>) -> Self {
        setup_fonts(&cc.egui_ctx);
        let model = KeyboardModel::Jis;
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
        }
    }

    fn face_mut(&mut self, face: Face) -> &mut YabFace {
        match face {
            Face::Normal => &mut self.layout.normal,
            Face::LeftThumb => &mut self.layout.left_thumb,
            Face::RightThumb => &mut self.layout.right_thumb,
            Face::Shift => &mut self.layout.shift,
        }
    }

    fn face(&self, face: Face) -> &YabFace {
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
            ValueKind::Special => YabValue::Special(SPECIAL_KEYS[self.edit_special_idx].0),
            ValueKind::None => YabValue::None,
        };
        self.face_mut(self.current_face).insert(pos, value);
        self.modified = true;
        self.status = "変更あり".to_string();
    }

    fn do_save(&mut self) {
        let Some(ref path) = self.file_path else {
            self.status = "ファイルパスが未設定です".to_string();
            return;
        };
        let text = self.layout.serialize(self.model);
        match std::fs::write(path, &text) {
            Ok(()) => {
                self.modified = false;
                self.status = format!("{} に保存しました", path.display());
            }
            Err(e) => self.status = format!("保存失敗: {e}"),
        }
    }

    fn do_open(&mut self) {
        let path = PathBuf::from(&self.file_path_buf);
        match load_layout(&path, self.model) {
            Ok(ly) => {
                self.layout = ly;
                self.file_path = Some(path.clone());
                self.modified = false;
                self.selected_pos = None;
                self.status = format!("{} を読み込みました", path.display());
            }
            Err(e) => self.status = format!("読み込み失敗: {e}"),
        }
    }

    fn do_reload(&mut self) {
        let Some(ref path) = self.file_path.clone() else {
            self.status = "ファイルパスが未設定です".to_string();
            return;
        };
        match load_layout(path, self.model) {
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
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::S)) {
            self.do_save();
        }
        if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::O)) {
            self.do_open();
        }
        if ctx.input(|i| i.key_pressed(egui::Key::F5)) {
            self.do_reload();
        }

        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("開く").clicked() {
                    self.do_open();
                }
                if ui.button("保存").clicked() {
                    self.do_save();
                }
                if ui.button("再読み込み").clicked() {
                    self.do_reload();
                }
                ui.separator();
                ui.label("パス:");
                let resp = ui
                    .add(egui::TextEdit::singleline(&mut self.file_path_buf).desired_width(250.0));
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    self.do_open();
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
                    "変更あり"
                } else {
                    "保存済み"
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

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                for (face, label) in &FACES {
                    if ui
                        .selectable_label(self.current_face == *face, *label)
                        .clicked()
                    {
                        self.current_face = *face;
                        self.selected_pos = None;
                    }
                }
            });
            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| {
                self.draw_keyboard_grid(ui);
                ui.separator();
                self.draw_edit_panel(ui);
            });
        });
    }
}

impl YabEditor {
    fn draw_keyboard_grid(&mut self, ui: &mut egui::Ui) {
        let row_sizes = self.model.row_sizes();
        let mut clicked_pos = None;
        for (row, &cols) in row_sizes.iter().enumerate() {
            ui.horizontal(|ui| {
                let indent = [0.0, 10.0, 20.0, 30.0][row];
                ui.add_space(indent);
                for col in 0..cols {
                    let pos = PhysicalPos::new(row as u8, col as u8);
                    let value = self.face(self.current_face).get(&pos);
                    let is_selected = self.selected_pos == Some(pos);
                    let btn = egui::Button::new(
                        egui::RichText::new(cell_display(value))
                            .monospace()
                            .size(13.0),
                    )
                    .fill(cell_color(value))
                    .stroke(if is_selected {
                        egui::Stroke::new(2.0, egui::Color32::from_rgb(0, 100, 200))
                    } else {
                        egui::Stroke::new(1.0, egui::Color32::GRAY)
                    })
                    .min_size(egui::vec2(38.0, 32.0));
                    if ui.add(btn).clicked() {
                        clicked_pos = Some(pos);
                    }
                }
            });
        }
        if let Some(pos) = clicked_pos {
            self.select_cell(pos);
        }
    }

    fn draw_edit_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("編集パネル");
        let Some(pos) = self.selected_pos else {
            ui.label("セルを選択してください");
            return;
        };
        ui.label(format!("位置: ({}, {})", pos.row, pos.col));
        ui.horizontal(|ui| {
            ui.label("種別:");
            ui.radio_value(&mut self.edit_kind, ValueKind::Romaji, "ローマ字");
            ui.radio_value(&mut self.edit_kind, ValueKind::Literal, "リテラル");
            ui.radio_value(&mut self.edit_kind, ValueKind::Special, "特殊キー");
            ui.radio_value(&mut self.edit_kind, ValueKind::None, "なし");
        });
        match self.edit_kind {
            ValueKind::Romaji => {
                ui.horizontal(|ui| {
                    ui.label("値:");
                    ui.text_edit_singleline(&mut self.edit_value);
                });
                let preview = self
                    .romaji_table
                    .get(self.edit_value.trim())
                    .map_or_else(|| "-".to_string(), |ch| ch.to_string());
                ui.horizontal(|ui| {
                    ui.label("かな:");
                    ui.label(egui::RichText::new(&preview).size(18.0).strong());
                });
            }
            ValueKind::Literal => {
                ui.horizontal(|ui| {
                    ui.label("値:");
                    ui.text_edit_singleline(&mut self.edit_value);
                });
            }
            ValueKind::Special => {
                ui.horizontal(|ui| {
                    ui.label("値:");
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
                ui.label("割り当てなし");
            }
        }
        if ui.button("適用").clicked() {
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
        Some(YabValue::Literal(s)) if s.chars().count() <= 2 => s.clone(),
        Some(YabValue::Literal(s)) => s.chars().take(2).collect::<String>() + ".",
        Some(YabValue::Special(SpecialKey::Backspace)) => "\u{232b}".to_string(),
        Some(YabValue::Special(SpecialKey::Enter)) => "\u{23ce}".to_string(),
        Some(YabValue::Special(SpecialKey::Escape)) => "ESC".to_string(),
        Some(YabValue::Special(SpecialKey::Space)) => "\u{2423}".to_string(),
        Some(YabValue::Special(SpecialKey::Delete)) => "DEL".to_string(),
        Some(YabValue::None) | None => "\u{2014}".to_string(),
    }
}

fn cell_color(value: Option<&YabValue>) -> egui::Color32 {
    match value {
        Some(YabValue::Romaji { .. }) => egui::Color32::from_rgb(255, 255, 255),
        Some(YabValue::Literal(_)) => egui::Color32::from_rgb(210, 230, 255),
        Some(YabValue::Special(_)) => egui::Color32::from_rgb(210, 255, 220),
        Some(YabValue::None) | None => egui::Color32::from_rgb(220, 220, 220),
    }
}

const fn model_label(model: KeyboardModel) -> &'static str {
    match model {
        KeyboardModel::Jis => "JIS",
        KeyboardModel::Us => "US",
    }
}

fn load_layout(path: &std::path::Path, model: KeyboardModel) -> Result<YabLayout, String> {
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
            .with_inner_size([700.0, 600.0])
            .with_title(&title),
        ..Default::default()
    };
    eframe::run_native(
        "awase-yab-editor",
        options,
        Box::new(move |cc| Ok(Box::new(YabEditor::new(cc, file_path)))),
    )
}
