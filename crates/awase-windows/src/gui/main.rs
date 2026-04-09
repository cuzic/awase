use eframe::egui;

/// 設定リロード用カスタムメッセージ ID（awase 本体側の `WM_APP + 10` と一致させる）
#[cfg(target_os = "windows")]
const WM_RELOAD_CONFIG: u32 = 0x8000 + 10; // WM_APP = 0x8000

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Basic,
    Keys,
    ImeDetect,
    Focus,
    Advanced,
}

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
    // Focus override add-buffers
    new_force_text_process: String,
    new_force_text_class: String,
    new_force_bypass_process: String,
    new_force_bypass_class: String,
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
            new_force_text_process: String::new(),
            new_force_text_class: String::new(),
            new_force_bypass_process: String::new(),
            new_force_bypass_class: String::new(),
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
}

// ── Tab methods ──

impl SettingsApp {
    fn tab_basic(&mut self, ui: &mut egui::Ui) {
        ui.heading("基本設定");
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            ui.label("キーボードモデル:").on_hover_text("キーボードの種類を選びます。\n日本語キーボード(JIS)か、英語キーボード(US)かを指定します。");
            egui::ComboBox::from_id_salt("kb_model")
                .selected_text(&self.config.general.keyboard_model)
                .show_ui(ui, |ui| {
                    for m in ["jis", "us"] {
                        ui.selectable_value(
                            &mut self.config.general.keyboard_model,
                            m.to_string(),
                            m,
                        );
                    }
                });
        });
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
                        .on_hover_text("統計データで次の文字を予測し、判定を最適化します。\nn-gram ファイルの指定が推奨です。");
                });
        });
        ui.label(confirm_mode_tooltip(self.config.general.confirm_mode));
        let spec_enabled = matches!(
            self.config.general.confirm_mode,
            awase::config::ConfirmMode::TwoPhase | awase::config::ConfirmMode::AdaptiveTiming
        );
        ui.add_enabled_ui(spec_enabled, |ui| {
            ui.horizontal(|ui| {
                ui.label("投機出力待機:").on_hover_text("投機出力までの待機時間です。\n短いほど応答が速くなりますが、誤判定が増えます。\nTwoPhase/AdaptiveTiming モードで使用されます。");
                ui.add(
                    egui::Slider::new(&mut self.config.general.speculative_delay_ms, 0..=100)
                        .suffix(" ms"),
                );
            });
        });
        ui.horizontal(|ui| {
            ui.label("出力モード:").on_hover_text("文字の出力方式を選びます。\nアプリとの相性に応じて切り替えてください。");
            egui::ComboBox::from_id_salt("output_mode")
                .selected_text(output_mode_label(self.config.general.output_mode))
                .show_ui(ui, |ui| {
                    use awase::config::OutputMode;
                    ui.selectable_value(&mut self.config.general.output_mode, OutputMode::Unicode, "Unicode")
                        .on_hover_text("ひらがなを直接出力します。\nIME の未確定文字列にはなりません。\n最も互換性が高い方式です。");
                    ui.selectable_value(&mut self.config.general.output_mode, OutputMode::PerKey, "PerKey")
                        .on_hover_text("ローマ字をキーイベントとして送信します。\nIME の未確定文字列として入力されます。");
                    ui.selectable_value(&mut self.config.general.output_mode, OutputMode::Batched, "Batched")
                        .on_hover_text("ローマ字をまとめて送信します。\n高速ですが、一部のアプリと相性が悪い場合があります。");
                });
        });
        ui.horizontal(|ui| {
            ui.label("フックモード:").on_hover_text("キーボードフックの動作方式です。\n通常は Relay を推奨します。");
            ui.radio_value(
                &mut self.config.general.hook_mode,
                awase::config::HookMode::Filter,
                "Filter",
            )
            .on_hover_text("変換しないキーはそのまま OS に通します。\n低遅延ですが、まれにキー順序の問題が起きます。");
            ui.radio_value(
                &mut self.config.general.hook_mode,
                awase::config::HookMode::Relay,
                "Relay",
            )
            .on_hover_text("全キーを一旦取り込んで再送信します。\nキーの順序が保証され、最も安定します。（推奨）");
        });
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
                "左の親指シフトキーに使うキーです。\n通常は「無変換」キーを使います。",
            );
            ui.text_edit_singleline(&mut self.config.general.left_thumb_key);
        });
        ui.horizontal(|ui| {
            ui.label("  右親指:").on_hover_text(
                "右の親指シフトキーに使うキーです。\n通常は「変換」キーを使います。",
            );
            ui.text_edit_singleline(&mut self.config.general.right_thumb_key);
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

    fn tab_focus(&mut self, ui: &mut egui::Ui) {
        ui.heading("フォーカス制御");
        ui.label("特定のアプリケーションで、エンジンの動作を強制的に切り替えます。\nプロセス名とクラス名の組み合わせで指定します。\nクラス名はログ出力で確認できます（RUST_LOG=debug で起動）。");
        ui.add_space(8.0);

        // force_text
        ui.label("テキスト入力として強制:").on_hover_text("これらのアプリでは、常にテキスト入力として扱います。\n自動判定で誤ってバイパスされるアプリを指定してください。");
        focus_table_ui(
            ui,
            "ft",
            &mut self.config.focus_overrides.force_text,
            &mut self.new_force_text_process,
            &mut self.new_force_text_class,
        );
        ui.add_space(12.0);

        // force_bypass
        ui.label("バイパスとして強制（エンジン無効）:").on_hover_text("これらのアプリでは、常にエンジンを無効にします。\n日本語入力が不要なアプリを指定してください。");
        focus_table_ui(
            ui,
            "fb",
            &mut self.config.focus_overrides.force_bypass,
            &mut self.new_force_bypass_process,
            &mut self.new_force_bypass_class,
        );

        ui.add_space(8.0);
        ui.label("プロセス名・クラス名はログで確認できます（RUST_LOG=debug で起動）。");
    }

    fn tab_advanced(&mut self, ui: &mut egui::Ui) {
        ui.heading("詳細設定");
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label("n-gram ファイル:").on_hover_text("n-gram 統計データファイルのパスです。\n.csv.gz または .toml 形式に対応しています。\nngram_predictive モードで使用されます。");
            let mut buf = self.config.general.ngram_file.clone().unwrap_or_default();
            if ui.text_edit_singleline(&mut buf).changed() {
                self.config.general.ngram_file = if buf.is_empty() { None } else { Some(buf) };
            }
        });
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
        // Side panel for tab selection
        egui::SidePanel::left("tab_panel")
            .resizable(false)
            .default_width(100.0)
            .show(ctx, |ui| {
                ui.add_space(8.0);
                for (tab, label) in [
                    (Tab::Basic, "基本設定"),
                    (Tab::Keys, "キー設定"),
                    (Tab::ImeDetect, "IME 検出"),
                    (Tab::Advanced, "詳細設定"),
                ] {
                    if ui.selectable_label(self.active_tab == tab, label).clicked() {
                        self.active_tab = tab;
                    }
                }
            });

        // Main content
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| match self.active_tab {
                Tab::Basic => self.tab_basic(ui),
                Tab::Keys => self.tab_keys(ui),
                Tab::ImeDetect => self.tab_ime_detect(ui),
                Tab::Focus => self.tab_focus(ui),
                Tab::Advanced => self.tab_advanced(ui),
            });

            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("適用").clicked() {
                    self.apply();
                }
                if ui.button("キャンセル").clicked() {
                    self.cancel();
                }
                if !self.status.is_empty() {
                    ui.label(&self.status);
                }
            });
        });
    }
}

// ── Reusable UI helpers ──

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

fn focus_table_ui(
    ui: &mut egui::Ui,
    id: &str,
    entries: &mut Vec<awase::config::FocusOverrideEntry>,
    np: &mut String,
    nc: &mut String,
) {
    ui.horizontal(|ui| {
        ui.label("  プロセス名");
        ui.add_space(60.0);
        ui.label("クラス名");
    });
    let mut rm = None;
    for (i, e) in entries.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.label(format!("  {}", e.process));
            ui.add_space(40.0);
            ui.label(&e.class);
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
            egui::TextEdit::singleline(np)
                .desired_width(120.0)
                .id(egui::Id::new(format!("{id}_p"))),
        );
        ui.add(
            egui::TextEdit::singleline(nc)
                .desired_width(120.0)
                .id(egui::Id::new(format!("{id}_c"))),
        );
        if ui.button("+追加").clicked() && !np.is_empty() && !nc.is_empty() {
            entries.push(awase::config::FocusOverrideEntry {
                process: std::mem::take(np),
                class: std::mem::take(nc),
            });
        }
    });
}

// ── Utility functions ──

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
        ConfirmMode::NgramPredictive => "  n-gram 統計で投機/待機を動的判断。最も賢い。",
    }
}

const fn output_mode_label(mode: awase::config::OutputMode) -> &'static str {
    use awase::config::OutputMode;
    match mode {
        OutputMode::Unicode => "Unicode",
        OutputMode::PerKey => "PerKey",
        OutputMode::Batched => "Batched",
    }
}
