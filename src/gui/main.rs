use eframe::egui;

/// 設定リロード用カスタムメッセージ ID（awase 本体側の `WM_APP + 10` と一致させる）
#[cfg(target_os = "windows")]
const WM_RELOAD_CONFIG: u32 = 0x8000 + 10; // WM_APP = 0x8000

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([500.0, 650.0])
            .with_title("awase 設定"),
        ..Default::default()
    };

    eframe::run_native(
        "awase-settings",
        options,
        Box::new(|cc| Ok(Box::new(SettingsApp::new(cc)))),
    )
}

struct SettingsApp {
    config: awase::config::AppConfig,
    config_path: std::path::PathBuf,
    status_message: String,
    available_layouts: Vec<String>,
    preview_engine: Option<awase::engine::Engine>,
    preview_output: String,
    preview_state: String,
    /// 新規 force_text エントリ入力バッファ
    new_force_text_process: String,
    new_force_text_class: String,
    /// 新規 force_bypass エントリ入力バッファ
    new_force_bypass_process: String,
    new_force_bypass_class: String,
}

impl SettingsApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        setup_fonts(&cc.egui_ctx);

        let config_path = find_config_path();
        let config =
            awase::config::AppConfig::load(&config_path).unwrap_or_else(|_| default_config());

        let available_layouts = scan_layout_names(&config.general.layouts_dir);

        let mut app = Self {
            config,
            config_path,
            status_message: String::new(),
            available_layouts,
            preview_engine: None,
            preview_output: String::new(),
            preview_state: String::new(),
            new_force_text_process: String::new(),
            new_force_text_class: String::new(),
            new_force_bypass_process: String::new(),
            new_force_bypass_class: String::new(),
        };
        app.rebuild_preview_engine();
        app
    }
}

fn find_config_path() -> std::path::PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let p = dir.join("config.toml");
            if p.exists() {
                return p;
            }
        }
    }
    std::path::PathBuf::from("config.toml")
}

fn scan_layout_names(layouts_dir: &str) -> Vec<String> {
    let dir = if std::path::Path::new(layouts_dir).is_absolute() {
        std::path::PathBuf::from(layouts_dir)
    } else if let Ok(exe) = std::env::current_exe() {
        exe.parent().map_or_else(
            || std::path::PathBuf::from(layouts_dir),
            |d| d.join(layouts_dir),
        )
    } else {
        std::path::PathBuf::from(layouts_dir)
    };

    let mut names = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "yab") {
                if let Some(stem) = path.file_stem() {
                    names.push(stem.to_string_lossy().to_string());
                }
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

    // Try to load OS Japanese font
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

impl eframe::App for SettingsApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("awase 設定");
            ui.separator();

            // ── 基本設定 ──
            egui::CollapsingHeader::new("基本設定")
                .default_open(true)
                .show(ui, |ui| {
                    self.basic_settings_ui(ui);
                });

            // ── 配列 ──
            egui::CollapsingHeader::new("配列")
                .default_open(true)
                .show(ui, |ui| {
                    self.layout_settings_ui(ui);
                });

            // ── n-gram（上級設定） ──
            egui::CollapsingHeader::new("n-gram（上級設定）")
                .default_open(false)
                .show(ui, |ui| {
                    self.ngram_settings_ui(ui);
                });

            // ── フォーカスオーバーライド ──
            egui::CollapsingHeader::new("フォーカスオーバーライド")
                .default_open(false)
                .show(ui, |ui| {
                    self.focus_overrides_ui(ui);
                });

            // ── プレビュー ──
            egui::CollapsingHeader::new("プレビュー")
                .default_open(true)
                .show(ui, |ui| {
                    self.preview_ui(ui);
                });

            ui.separator();

            // Apply/Cancel buttons
            ui.horizontal(|ui| {
                if ui.button("適用").clicked() {
                    match self.config.save(&self.config_path) {
                        Ok(()) => {
                            self.status_message = "設定を保存しました".into();
                            send_reload_config_message();
                        }
                        Err(e) => self.status_message = format!("保存エラー: {e}"),
                    }
                    self.rebuild_preview_engine();
                }
                if ui.button("キャンセル").clicked() {
                    std::process::exit(0);
                }
                if !self.status_message.is_empty() {
                    ui.label(&self.status_message);
                }
            });
        });
    }
}

impl SettingsApp {
    fn layout_settings_ui(&mut self, ui: &mut egui::Ui) {
        // Default layout dropdown
        ui.horizontal(|ui| {
            ui.label("デフォルト配列:");
            let current = self
                .config
                .general
                .default_layout
                .trim_end_matches(".yab")
                .to_string();
            egui::ComboBox::from_id_salt("default_layout")
                .selected_text(&current)
                .show_ui(ui, |ui| {
                    for name in &self.available_layouts {
                        let is_selected = current == *name;
                        if ui.selectable_label(is_selected, name).clicked() {
                            self.config.general.default_layout = format!("{name}.yab");
                        }
                    }
                });
        });

        // Layouts directory
        ui.horizontal(|ui| {
            ui.label("配列フォルダ:");
            ui.text_edit_singleline(&mut self.config.general.layouts_dir);
            if ui.button("再スキャン").clicked() {
                self.available_layouts = scan_layout_names(&self.config.general.layouts_dir);
            }
        });

        // Layout info
        ui.label(format!(
            "  検出された配列: {} 件",
            self.available_layouts.len()
        ));
    }

    fn basic_settings_ui(&mut self, ui: &mut egui::Ui) {
        // Confirm mode dropdown
        ui.horizontal(|ui| {
            ui.label("確定モード:");
            egui::ComboBox::from_id_salt("confirm_mode")
                .selected_text(confirm_mode_label(self.config.general.confirm_mode))
                .show_ui(ui, |ui| {
                    use awase::config::ConfirmMode;
                    for (mode, label) in [
                        (ConfirmMode::Wait, "待機 (wait)"),
                        (ConfirmMode::Speculative, "先行確定 (speculative)"),
                        (ConfirmMode::TwoPhase, "二段タイマー (two_phase)"),
                        (
                            ConfirmMode::AdaptiveTiming,
                            "適応タイミング (adaptive_timing)",
                        ),
                        (
                            ConfirmMode::NgramPredictive,
                            "n-gram 予測 (ngram_predictive)",
                        ),
                    ] {
                        ui.selectable_value(&mut self.config.general.confirm_mode, mode, label);
                    }
                });
        });

        // Confirm mode description
        ui.label(confirm_mode_description(self.config.general.confirm_mode));
        ui.add_space(8.0);

        // Threshold slider
        ui.horizontal(|ui| {
            ui.label("同時打鍵閾値:");
            ui.add(
                egui::Slider::new(&mut self.config.general.simultaneous_threshold_ms, 30..=200)
                    .suffix(" ms"),
            );
        });

        // Speculative delay slider (used by TwoPhase, AdaptiveTiming, etc.)
        ui.horizontal(|ui| {
            ui.label("投機遅延:");
            ui.add(
                egui::Slider::new(&mut self.config.general.speculative_delay_ms, 10..=100)
                    .suffix(" ms"),
            );
        });

        // Hotkey
        ui.horizontal(|ui| {
            ui.label("切替ホットキー:");
            let hotkey = self
                .config
                .general
                .toggle_hotkey
                .get_or_insert_with(String::new);
            ui.text_edit_singleline(hotkey);
        });
    }
}

impl SettingsApp {
    fn ngram_settings_ui(&mut self, ui: &mut egui::Ui) {
        // Enable/disable checkbox
        let mut enabled = self.config.general.ngram_file.is_some();
        if ui
            .checkbox(&mut enabled, "n-gram 適応閾値を有効にする")
            .changed()
        {
            if enabled {
                self.config.general.ngram_file = Some("data/ngram_hiragana.toml".into());
            } else {
                self.config.general.ngram_file = None;
            }
        }

        // Grayed out if disabled
        ui.add_enabled_ui(enabled, |ui| {
            // Corpus file path
            if let Some(ref mut path) = self.config.general.ngram_file {
                ui.horizontal(|ui| {
                    ui.label("コーパスファイル:");
                    ui.text_edit_singleline(path);
                });
            }

            // Adjustment range
            ui.horizontal(|ui| {
                ui.label("調整幅:");
                ui.add(
                    egui::Slider::new(&mut self.config.general.ngram_adjustment_range_ms, 5..=50)
                        .suffix(" ms"),
                );
            });

            // Min/Max thresholds
            ui.horizontal(|ui| {
                ui.label("閾値下限:");
                ui.add(
                    egui::Slider::new(&mut self.config.general.ngram_min_threshold_ms, 10..=100)
                        .suffix(" ms"),
                );
            });
            ui.horizontal(|ui| {
                ui.label("閾値上限:");
                ui.add(
                    egui::Slider::new(&mut self.config.general.ngram_max_threshold_ms, 50..=200)
                        .suffix(" ms"),
                );
            });
        });
    }
}

/// 配列フォルダのパスを解決する（相対パスなら実行ファイルの親ディレクトリを基準にする）。
fn resolve_layouts_dir(layouts_dir: &str) -> std::path::PathBuf {
    if std::path::Path::new(layouts_dir).is_absolute() {
        std::path::PathBuf::from(layouts_dir)
    } else if let Ok(exe) = std::env::current_exe() {
        exe.parent().map_or_else(
            || std::path::PathBuf::from(layouts_dir),
            |d| d.join(layouts_dir),
        )
    } else {
        std::path::PathBuf::from(layouts_dir)
    }
}

impl SettingsApp {
    fn rebuild_preview_engine(&mut self) {
        let layouts_dir = resolve_layouts_dir(&self.config.general.layouts_dir);
        let layout_file = layouts_dir.join(&self.config.general.default_layout);

        let Ok(content) = std::fs::read_to_string(&layout_file) else {
            self.preview_engine = None;
            return;
        };
        let Ok(layout) = awase::yab::YabLayout::parse(&content) else {
            self.preview_engine = None;
            return;
        };
        let layout = layout.resolve_kana();

        let left_vk = awase::types::VkCode(
            awase::config::vk_name_to_code(&self.config.general.left_thumb_key).unwrap_or(0x1D),
        );
        let right_vk = awase::types::VkCode(
            awase::config::vk_name_to_code(&self.config.general.right_thumb_key).unwrap_or(0x1C),
        );

        self.preview_engine = Some(awase::engine::Engine::new(
            layout,
            left_vk,
            right_vk,
            self.config.general.simultaneous_threshold_ms,
            self.config.general.confirm_mode,
            self.config.general.speculative_delay_ms,
        ));
        self.preview_output.clear();
    }

    fn preview_ui(&mut self, ui: &mut egui::Ui) {
        ui.label("キーボードで入力してみてください（実際の IME には送信されません）");
        ui.label("※ タイムアウトと同時打鍵のシミュレーションは簡易版です");

        // Capture keyboard input from egui
        let events = ui.input(|i| i.events.clone());
        for event in &events {
            if let egui::Event::Key {
                key, pressed: true, ..
            } = event
            {
                self.handle_preview_key(*key);
            }
        }

        // Output display
        ui.horizontal(|ui| {
            ui.label("出力:");
            ui.monospace(&self.preview_output);
        });

        // State display
        if self.preview_engine.is_some() {
            ui.label(format!(
                "状態: {} | 確定モード: {:?}",
                self.preview_state, self.config.general.confirm_mode
            ));
        } else {
            ui.label("⚠ 配列ファイルを読み込めませんでした。配列設定を確認してください。");
        }

        if ui.button("クリア").clicked() {
            self.preview_output.clear();
            self.rebuild_preview_engine();
        }
    }

    #[allow(clippy::too_many_lines)]
    fn handle_preview_key(&mut self, key: egui::Key) {
        use timed_fsm::TimedStateMachine;

        let Some(ref mut engine) = self.preview_engine else {
            return;
        };

        // Map egui key to VK code (simplified A-Z mapping)
        let vk: u16 = match key {
            egui::Key::A => 0x41,
            egui::Key::B => 0x42,
            egui::Key::C => 0x43,
            egui::Key::D => 0x44,
            egui::Key::E => 0x45,
            egui::Key::F => 0x46,
            egui::Key::G => 0x47,
            egui::Key::H => 0x48,
            egui::Key::I => 0x49,
            egui::Key::J => 0x4A,
            egui::Key::K => 0x4B,
            egui::Key::L => 0x4C,
            egui::Key::M => 0x4D,
            egui::Key::N => 0x4E,
            egui::Key::O => 0x4F,
            egui::Key::P => 0x50,
            egui::Key::Q => 0x51,
            egui::Key::R => 0x52,
            egui::Key::S => 0x53,
            egui::Key::T => 0x54,
            egui::Key::U => 0x55,
            egui::Key::V => 0x56,
            egui::Key::W => 0x57,
            egui::Key::X => 0x58,
            egui::Key::Y => 0x59,
            egui::Key::Z => 0x5A,
            _ => return,
        };

        // Create a simulated RawKeyEvent
        let scan = awase::yab::vk_to_pos(vk)
            .and_then(awase::scanmap::pos_to_scan)
            .unwrap_or(0);
        let event = awase::types::RawKeyEvent {
            vk_code: awase::types::VkCode(vk),
            scan_code: awase::types::ScanCode(scan),
            event_type: awase::types::KeyEventType::KeyDown,
            extra_info: 0,
            timestamp: u64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_micros(),
            )
            .unwrap_or(u64::MAX),
        };

        let response = engine.on_event(event);

        // Collect output from actions
        for action in &response.actions {
            match action {
                awase::types::KeyAction::Romaji(s) => self.preview_output.push_str(s),
                awase::types::KeyAction::Char(ch) => self.preview_output.push(*ch),
                awase::types::KeyAction::Key(0x08) => {
                    // Backspace
                    self.preview_output.pop();
                }
                _ => {}
            }
        }

        self.preview_state = if response.consumed {
            "消費".to_string()
        } else {
            "通過".to_string()
        };
    }
}

impl SettingsApp {
    fn focus_overrides_ui(&mut self, ui: &mut egui::Ui) {
        ui.label("特定のプロセス/ウィンドウクラスに対する動作を強制指定します。");
        ui.add_space(4.0);

        // ── force_text（常にテキスト入力として扱う） ──
        ui.label("常にテキスト入力として扱う (force_text):");
        let mut remove_text_idx = None;
        for (i, entry) in self.config.focus_overrides.force_text.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(format!("  {} / {}", entry.process, entry.class));
                if ui.small_button("削除").clicked() {
                    remove_text_idx = Some(i);
                }
            });
        }
        if let Some(idx) = remove_text_idx {
            self.config.focus_overrides.force_text.remove(idx);
        }
        ui.horizontal(|ui| {
            ui.label("  プロセス:");
            ui.add(egui::TextEdit::singleline(&mut self.new_force_text_process).desired_width(120.0));
            ui.label("クラス:");
            ui.add(egui::TextEdit::singleline(&mut self.new_force_text_class).desired_width(120.0));
            if ui.button("追加").clicked()
                && !self.new_force_text_process.is_empty()
                && !self.new_force_text_class.is_empty()
            {
                self.config
                    .focus_overrides
                    .force_text
                    .push(awase::config::FocusOverrideEntry {
                        process: self.new_force_text_process.drain(..).collect(),
                        class: self.new_force_text_class.drain(..).collect(),
                    });
            }
        });

        ui.add_space(8.0);

        // ── force_bypass（常に非テキストとしてバイパス） ──
        ui.label("常にバイパスする (force_bypass):");
        let mut remove_bypass_idx = None;
        for (i, entry) in self.config.focus_overrides.force_bypass.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(format!("  {} / {}", entry.process, entry.class));
                if ui.small_button("削除").clicked() {
                    remove_bypass_idx = Some(i);
                }
            });
        }
        if let Some(idx) = remove_bypass_idx {
            self.config.focus_overrides.force_bypass.remove(idx);
        }
        ui.horizontal(|ui| {
            ui.label("  プロセス:");
            ui.add(egui::TextEdit::singleline(&mut self.new_force_bypass_process).desired_width(120.0));
            ui.label("クラス:");
            ui.add(egui::TextEdit::singleline(&mut self.new_force_bypass_class).desired_width(120.0));
            if ui.button("追加").clicked()
                && !self.new_force_bypass_process.is_empty()
                && !self.new_force_bypass_class.is_empty()
            {
                self.config
                    .focus_overrides
                    .force_bypass
                    .push(awase::config::FocusOverrideEntry {
                        process: self.new_force_bypass_process.drain(..).collect(),
                        class: self.new_force_bypass_class.drain(..).collect(),
                    });
            }
        });
    }
}

/// 設定保存後に awase 本体プロセスへ `WM_RELOAD_CONFIG` を送信する。
///
/// Windows では `FindWindowW` で awase のメッセージウィンドウを探し、
/// `PostMessageW` でカスタムメッセージを送信する。
/// Windows 以外では何もしない（GUI プレビュー開発用）。
#[allow(clippy::missing_const_for_fn)]
fn send_reload_config_message() {
    #[cfg(target_os = "windows")]
    {
        use windows::core::w;
        use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW};

        unsafe {
            let hwnd = FindWindowW(w!("awase_msg_window"), None);
            if let Ok(hwnd) = hwnd {
                let msg = windows::Win32::Foundation::WPARAM(0);
                let lparam = windows::Win32::Foundation::LPARAM(0);
                let _ = PostMessageW(hwnd, WM_RELOAD_CONFIG, msg, lparam);
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        // Non-Windows: no-op (for development/testing)
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

const fn confirm_mode_description(mode: awase::config::ConfirmMode) -> &'static str {
    use awase::config::ConfirmMode;
    match mode {
        ConfirmMode::Wait => "  タイムアウトまで出力を保留します。安定性重視。",
        ConfirmMode::Speculative => "  即座に出力し、同時打鍵時に BackSpace で差し替えます。",
        ConfirmMode::TwoPhase => "  短い待機後に投機出力。遅延とちらつきのバランスが最適。",
        ConfirmMode::AdaptiveTiming => "  連続入力中は待機、途切れたら投機出力に切り替えます。",
        ConfirmMode::NgramPredictive => "  n-gram 頻度で投機/待機を自動判定します。",
    }
}
