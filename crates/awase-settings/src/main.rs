use eframe::egui;

/// 設定リロード用カスタムメッセージ ID（awase 本体側の `WM_APP + 10` と一致させる）
#[cfg(target_os = "windows")]
const WM_RELOAD_CONFIG: u32 = 0x8000 + 10; // WM_APP = 0x8000

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Basic,
    Keys,
    Keymap,
    ImeDetect,
    Preview,
    Advanced,
}

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
    // Keymap rule add-buffers
    new_keymap_app: String,
    new_keymap_from_ctrl: bool,
    new_keymap_from_shift: bool,
    new_keymap_from_alt: bool,
    new_keymap_from_main: String,
    new_keymap_to_main: String,
    // Keymap capture mode (None = not capturing)
    capturing: Option<CaptureTarget>,
    // Preview cache: (layout filename, parsed result)
    preview_cache: Option<(String, Result<awase::yab::YabLayout, String>)>,
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
            preview_cache: None,
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
            thumb_key_combo(ui, "left_thumb_key", &mut self.config.general.left_thumb_key);
        });
        ui.horizontal(|ui| {
            ui.label("  右親指:").on_hover_text(
                "右の親指シフトキーに使うキーです。\n通常は「変換」キーを使います。",
            );
            thumb_key_combo(ui, "right_thumb_key", &mut self.config.general.right_thumb_key);
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
                ui.horizontal(|ui| {
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
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.new_keymap_from_ctrl, "Ctrl");
                    ui.checkbox(&mut self.new_keymap_from_shift, "Shift");
                    ui.checkbox(&mut self.new_keymap_from_alt, "Alt");
                    main_key_combo(ui, "new_from_main", &mut self.new_keymap_from_main);
                    capture_button(ui, &mut capturing, CaptureTarget::NewFrom);
                });
                ui.end_row();

                ui.label("  to:")
                    .on_hover_text("再注入するキー。「（消費のみ）」を選ぶとキーを消費するだけ。");
                ui.horizontal(|ui| {
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

    fn tab_preview(&mut self, ui: &mut egui::Ui) {
        ui.heading("配列プレビュー");
        ui.label("選択中のレイアウトをキーボード風に表示します。\n通常面・左親指シフト面・右親指シフト面を確認できます。");
        ui.add_space(8.0);

        let layout_file = self.config.general.default_layout.clone();
        ui.horizontal(|ui| {
            ui.label(format!("レイアウト: {layout_file}"));
            if ui.button("再読み込み").clicked() {
                self.preview_cache = None;
            }
        });
        ui.add_space(8.0);

        // キャッシュをチェック、無ければロード
        let need_reload = self
            .preview_cache
            .as_ref()
            .map_or(true, |(cached_name, _)| cached_name != &layout_file);
        if need_reload {
            self.preview_cache = Some((
                layout_file.clone(),
                load_layout_for_preview(
                    &self.config.general.layouts_dir,
                    &layout_file,
                    &self.config.general.keyboard_model,
                ),
            ));
        }

        match self.preview_cache.as_ref().map(|(_, r)| r) {
            Some(Ok(layout)) => {
                ui.label(format!("名前: {}", layout.name));
                ui.add_space(12.0);
                ui.label("通常面");
                draw_face_grid(ui, &layout.normal, "normal");
                ui.add_space(12.0);
                ui.label("左親指シフト面");
                draw_face_grid(ui, &layout.left_thumb, "left");
                ui.add_space(12.0);
                ui.label("右親指シフト面");
                draw_face_grid(ui, &layout.right_thumb, "right");
            }
            Some(Err(e)) => {
                ui.colored_label(egui::Color32::RED, format!("読み込みエラー: {e}"));
            }
            None => {
                ui.label("読み込み中...");
            }
        }
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
                for (tab, label) in [
                    (Tab::Basic, "基本設定"),
                    (Tab::Keys, "キー設定"),
                    (Tab::Keymap, "ショートカット"),
                    (Tab::ImeDetect, "IME 検出"),
                    (Tab::Preview, "プレビュー"),
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
                Tab::Keymap => self.tab_keymap(ui),
                Tab::ImeDetect => self.tab_ime_detect(ui),
                Tab::Preview => self.tab_preview(ui),
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

/// 親指キー選択用候補一覧（表示名, config 内部表記）。
const THUMB_KEY_OPTIONS: &[(&str, &str)] = &[
    ("Space", "VK_SPACE"),
    ("変換", "VK_CONVERT"),
    ("無変換", "VK_NONCONVERT"),
    ("かな", "VK_KANA"),
    ("カタカナ", "VK_DBE_KATAKANA"),
    ("ひらがな", "VK_DBE_HIRAGANA"),
];

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
    let display = THUMB_KEY_OPTIONS
        .iter()
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
            for (label, internal) in THUMB_KEY_OPTIONS {
                if ui.selectable_label(current.as_str() == *internal, *label).clicked() {
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

// ── Preview helpers ──

/// JIS キーボードの各行のキー数（.yab の行/列と一致）
const JIS_ROW_KEYS: [usize; 4] = [13, 12, 11, 10];

/// 指定されたレイアウトファイルをパースしてプレビュー用に読み込む。
fn load_layout_for_preview(
    layouts_dir: &str,
    layout_file: &str,
    keyboard_model: &str,
) -> Result<awase::yab::YabLayout, String> {
    // layouts_dir が相対パスなら current_exe の隣を探す
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
    let path = dir.join(layout_file);
    let content = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let model = match keyboard_model {
        "us" => awase::scanmap::KeyboardModel::Us,
        _ => awase::scanmap::KeyboardModel::Jis,
    };
    awase::yab::YabLayout::parse(&content, model).map_err(|e| format!("{e}"))
}

/// YabFace をキーボード風のグリッドで描画する。
fn draw_face_grid(ui: &mut egui::Ui, face: &awase::yab::YabFace, id_suffix: &str) {
    let key_size = egui::vec2(32.0, 32.0);
    egui::Grid::new(format!("face_grid_{id_suffix}"))
        .spacing(egui::vec2(2.0, 2.0))
        .show(ui, |ui| {
            for (row_idx, &col_count) in JIS_ROW_KEYS.iter().enumerate() {
                // 行インデントで段差を表現
                let indent = (row_idx as f32) * 8.0;
                if indent > 0.0 {
                    ui.add_space(indent);
                }
                for col_idx in 0..col_count {
                    let pos = awase::scanmap::PhysicalPos::new(row_idx as u8, col_idx as u8);
                    let label = face.get(&pos).map(yab_value_display).unwrap_or_default();
                    let (rect, _response) = ui.allocate_exact_size(key_size, egui::Sense::hover());
                    ui.painter().rect_stroke(
                        rect,
                        4.0,
                        egui::Stroke::new(1.0, egui::Color32::GRAY),
                        egui::StrokeKind::Inside,
                    );
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        &label,
                        egui::FontId::proportional(16.0),
                        ui.visuals().text_color(),
                    );
                }
                ui.end_row();
            }
        });
}

/// `YabValue` を1〜2文字の表示文字列に変換する。
fn yab_value_display(v: &awase::yab::YabValue) -> String {
    use awase::yab::YabValue;
    match v {
        YabValue::Romaji { kana: Some(k), .. } => k.to_string(),
        YabValue::Romaji { romaji, kana: None } => romaji.clone(),
        YabValue::Literal(s) => s.clone(),
        YabValue::KeySequence(s) => s.clone(),
        YabValue::Special(_) => "◆".to_string(),
        YabValue::None => String::new(),
    }
}

// ── Utility functions ──

fn find_config_path() -> std::path::PathBuf {
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let p = dir.join("config.toml");
        if p.exists() {
            return p;
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

#[expect(clippy::missing_const_for_fn)]
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
