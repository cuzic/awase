use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};

use awase_windows::bug_report::{
    BugReportDiagnostics, BugReportImeKind, BugReportInput, LOG_EXCERPT_MAX_BYTES, MAX_BODY_BYTES,
    RETENTION_HINT, SymptomCategory, build_payload_json_fitting, unix_seconds_to_rfc3339,
};
use eframe::egui;

#[derive(Debug, Clone)]
pub(crate) struct BugReportArgs {
    pub(crate) journal_path: Option<PathBuf>,
    pub(crate) ime_kind: BugReportImeKind,
    pub(crate) diagnostics_path: Option<PathBuf>,
    /// 実際の `log::` 出力（`awase.log`）のパス。journal（構造化イベント）とは
    /// 別系統の添付（BUG-34 横展開）。
    pub(crate) app_log_path: Option<PathBuf>,
}

#[derive(Debug)]
pub(crate) struct BugReportApp {
    symptom_category: Option<SymptomCategory>,
    description: String,
    attach_log: bool,
    attach_state_snapshot: bool,
    attach_config: bool,
    attach_layout: bool,
    journal_json: Option<String>,
    journal_status: String,
    app_log: Option<String>,
    app_log_status: String,
    ime_kind: BugReportImeKind,
    diagnostics: BugReportDiagnostics,
    os_version: String,
    reported_at: String,
    preview_json: String,
    last_generated_preview: String,
    /// journal/awase.log の添付を自動的に切り詰めた場合の通知文。`status`
    /// （送信中/送信結果/エラー用）とは別フィールドにしている —
    /// 同じフィールドで扱うと、デバウンスによるプレビュー再生成が
    /// 送信成功時の report_id や送信失敗時の保存先パスのような、
    /// まだユーザーが読んでいない重要な情報を上書きしてしまう。
    log_shrink_notice: Option<String>,
    /// 説明欄・添付チェックボックスの変更があった時刻。プレビュー再生成
    /// （JSON 全体を最大 ~500KB 再シリアライズする、決して軽くない処理）を
    /// キー入力のたびに同期実行すると、egui の即時モード再描画と相まって
    /// テキスト入力に体感できる遅延が出る（実機報告）。変更を即座には
    /// 反映せず、最後の変更から `PREVIEW_DEBOUNCE` 経過してから 1 回だけ
    /// 生成する（デバウンス）。
    pending_preview_refresh: Option<std::time::Instant>,
    status: String,
    pending: Option<Receiver<SendOutcome>>,
    /// CJK フォント読み込み（`setup_fonts`）を初回フレームで一度だけ行った
    /// か。`run()` のウィンドウ生成クロージャ内で同期的に読み込むと、
    /// トレイ（バックグラウンドプロセス）から起動されたウィンドウが前面へ
    /// 表示される前にフォントパース（数MBのCJK .ttc）で数百ms 遅延し、
    /// Windows の「新規ウィンドウへのフォアグラウンド許可」の猶予時間を
    /// 逃して背面のまま開くことがあった（BUG-72 対応時の副作用、
    /// 実機で「一瞬表示されてすぐ消える」と報告）。ウィンドウ生成自体は
    /// 即座に行い、フォント読み込みは最初の `update()` へ遅延させる。
    fonts_initialized: bool,
}

#[derive(Debug)]
enum SendOutcome {
    Success {
        report_id: String,
    },
    Failure {
        message: String,
        saved_payload: Result<PathBuf, String>,
    },
}

impl BugReportApp {
    pub(crate) fn new(args: &BugReportArgs) -> Self {
        let (journal_json, journal_status) = match args.journal_path.as_ref() {
            Some(path) => match std::fs::read_to_string(path) {
                Ok(json) => (Some(json), format!("添付ログ: {}", path.display())),
                Err(e) => (
                    None,
                    format!("添付ログを読めませんでした: {} ({e})", path.display()),
                ),
            },
            None => (None, "添付ログ: なし".to_owned()),
        };
        let (app_log, app_log_status) = match args.app_log_path.as_ref() {
            Some(path) => match std::fs::read_to_string(path) {
                Ok(text) => (
                    Some(text),
                    format!("添付ログ(awase.log): {}", path.display()),
                ),
                Err(e) => (
                    None,
                    format!(
                        "添付ログ(awase.log)を読めませんでした: {} ({e})",
                        path.display()
                    ),
                ),
            },
            None => (None, "添付ログ(awase.log): なし".to_owned()),
        };
        let reported_at = current_reported_at();
        let os_version = detect_os_version();
        let diagnostics = load_diagnostics(args.diagnostics_path.as_ref());
        let mut app = Self {
            symptom_category: None,
            description: String::new(),
            attach_log: true,
            attach_state_snapshot: true,
            attach_config: true,
            attach_layout: true,
            journal_json,
            journal_status,
            app_log,
            app_log_status,
            ime_kind: args.ime_kind,
            diagnostics,
            os_version,
            reported_at,
            preview_json: String::new(),
            last_generated_preview: String::new(),
            log_shrink_notice: None,
            pending_preview_refresh: None,
            status: "症状カテゴリを選択して、送信前の内容を確認してください。".to_owned(),
            pending: None,
            fonts_initialized: false,
        };
        app.refresh_preview_if_unedited();
        app
    }

    /// 添付チェックボックス4つとその下のステータスラベルを描画する。
    /// `update` の行数を抑えるための抽出（clippy::too_many_lines）。
    /// 戻り値: いずれかのチェックボックスが変化したか。
    fn draw_attachment_checkboxes(&mut self, ui: &mut egui::Ui) -> bool {
        let attach_log_changed = ui
            .checkbox(
                &mut self.attach_log,
                "ログを添付する（journal + awase.log）",
            )
            .changed();
        let attach_state_snapshot_changed = ui
            .checkbox(
                &mut self.attach_state_snapshot,
                "内部状態スナップショットを添付する",
            )
            .changed();
        let attach_config_changed = ui
            .checkbox(
                &mut self.attach_config,
                "設定ファイル(config.toml)を添付する",
            )
            .changed();
        let attach_layout_changed = ui
            .checkbox(&mut self.attach_layout, "配列ファイル(.yab)を添付する")
            .changed();
        ui.label(&self.journal_status);
        ui.label(&self.app_log_status);
        attach_log_changed
            || attach_state_snapshot_changed
            || attach_config_changed
            || attach_layout_changed
    }

    /// 生成済みのプレビュー JSON を反映する。デバウンス完了時と「プレビュー
    /// を再生成」ボタンの両方から使う共通処理（片方だけ更新すると、
    /// 縮小通知の表示漏れや `pending_preview_refresh` の消し忘れのような
    /// 差異が生まれるため一本化している）。
    fn apply_generated_preview(&mut self, json: String, shrunk: bool) {
        self.preview_json.clone_from(&json);
        self.last_generated_preview = json;
        self.log_shrink_notice = shrunk.then(|| {
            "送信内容が上限を超えていたため、添付ログ(journal/awase.log)を自動的に切り詰めました。"
                .to_owned()
        });
    }

    fn refresh_preview_if_unedited(&mut self) {
        if self.preview_json != self.last_generated_preview {
            return;
        }
        match self.generated_preview() {
            Ok((json, shrunk)) => self.apply_generated_preview(json, shrunk),
            Err(e) => {
                let json = format!("{{\n  \"error\": \"{}\"\n}}", escape_json_string(&e));
                self.preview_json.clone_from(&json);
                self.last_generated_preview = json;
            }
        }
    }

    /// プレビュー JSON を生成する。`MAX_BODY_BYTES` を超える場合は
    /// journal/awase.log の添付上限（既定 `LOG_EXCERPT_MAX_BYTES`）を
    /// 自動的に縮小して収まるまで再構築する（戻り値の bool が縮小の有無）。
    fn generated_preview(&self) -> Result<(String, bool), String> {
        let symptom_category = self
            .symptom_category
            .ok_or_else(|| "症状カテゴリを選択してください".to_owned())?;
        let (json, used_budget) = build_payload_json_fitting(
            &BugReportInput {
                app_version: env!("CARGO_PKG_VERSION"),
                os_version: &self.os_version,
                ime_kind: self.ime_kind,
                ime_product_name: self.diagnostics.ime_product_name.as_deref(),
                keyboard_model: &self.diagnostics.keyboard_model,
                windows_keyboard_layout: &self.diagnostics.windows_keyboard_layout,
                competing_software: self.diagnostics.competing_software.clone(),
                symptom_category,
                description: &self.description,
                attach_log: self.attach_log,
                journal_json: self.journal_json.as_deref(),
                app_log: self.app_log.as_deref(),
                state_snapshot: self.diagnostics.state_snapshot.clone(),
                attach_state_snapshot: self.attach_state_snapshot,
                config_toml: self.diagnostics.config_toml.as_deref(),
                attach_config: self.attach_config,
                layout_yab: self.diagnostics.layout_yab.as_deref(),
                attach_layout: self.attach_layout,
                reported_at: &self.reported_at,
            },
            MAX_BODY_BYTES,
        )
        .map_err(|e| e.to_string())?;
        // used_budget < LOG_EXCERPT_MAX_BYTES だけでは「縮小を試みた」ことしか
        // 分からず、縮小してもなお上限を超えているケース（journal/app_log
        // 以外のフィールドが支配的）を「自動的に切り詰めました」という
        // 成功通知として誤って表示してしまう。実際に上限内に収まった場合に
        // 限り shrunk=true とする。
        let shrunk = used_budget < LOG_EXCERPT_MAX_BYTES && json.len() <= MAX_BODY_BYTES;
        Ok((json, shrunk))
    }

    fn poll_send_result(&mut self) {
        let Some(rx) = self.pending.take() else {
            return;
        };
        match rx.try_recv() {
            Ok(SendOutcome::Success { report_id }) => {
                self.status = format!("送信しました。report_id: {report_id}");
            }
            Ok(SendOutcome::Failure {
                message,
                saved_payload,
            }) => {
                self.status = match saved_payload {
                    Ok(saved_path) => format!(
                        "送信できませんでした: {message}\n送信内容を {} に保存しました。後で手動で送ってください。",
                        saved_path.display()
                    ),
                    Err(save_error) => format!(
                        "送信できませんでした: {message}\n送信内容のローカル保存にも失敗しました: {save_error}"
                    ),
                };
            }
            Err(mpsc::TryRecvError::Empty) => {
                self.pending = Some(rx);
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                "送信処理が予期せず終了しました。".clone_into(&mut self.status);
            }
        }
    }

    fn start_send(&mut self) {
        if self.pending.is_some() {
            return;
        }
        let Some(symptom_category) = self.symptom_category else {
            "症状カテゴリを選択してください。".clone_into(&mut self.status);
            return;
        };
        if symptom_category == SymptomCategory::Other && self.description.trim().is_empty() {
            "その他の場合は説明を入力してください。".clone_into(&mut self.status);
            return;
        }
        // デバウンス中（直近の変更からまだ PREVIEW_DEBOUNCE 経過していない）に
        // 送信ボタンを押した場合、プレビューが最新の入力内容を反映していない
        // ことがあるため、送信直前に確定させる。
        if self.pending_preview_refresh.is_some() {
            self.refresh_preview_if_unedited();
            self.pending_preview_refresh = None;
        }
        let body = self.preview_json.clone();
        if body.len() > MAX_BODY_BYTES {
            self.status = format!(
                "送信内容が大きすぎます({}KB > {}KB上限)。ログの自動切り詰めを試みても収まりませんでした。プレビューを直接編集するか、設定ファイル・配列ファイルの添付を外してください。",
                body.len() / 1024,
                MAX_BODY_BYTES / 1024,
            );
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        "送信中です...".clone_into(&mut self.status);
        std::thread::spawn(move || {
            let outcome = match send_report(&body) {
                Ok(report_id) => SendOutcome::Success { report_id },
                Err(message) => SendOutcome::Failure {
                    message,
                    saved_payload: save_failed_payload(&body),
                },
            };
            let _ = tx.send(outcome);
        });
    }
}

fn load_diagnostics(path: Option<&PathBuf>) -> BugReportDiagnostics {
    let Some(path) = path else {
        return BugReportDiagnostics::default();
    };
    std::fs::read_to_string(path)
        .ok()
        .and_then(|json| serde_json::from_str::<BugReportDiagnostics>(&json).ok())
        .unwrap_or_default()
}

/// 説明欄・添付チェックボックスの変更後、プレビュー再生成を実際に行うまで
/// 待つ時間。キー入力のたびに同期実行しない理由は `pending_preview_refresh`
/// フィールドのコメント参照。
const PREVIEW_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(300);

impl BugReportApp {
    /// 送信/プレビュー再生成ボタンとステータス行。`egui::TopBottomPanel::bottom`
    /// で画面下部に固定表示する（`update` の行数を抑えるための抽出、
    /// clippy::too_many_lines 対策も兼ねる）。ボタンが通常フローの末尾に
    /// あると、症状カテゴリ・説明欄・チェックボックス・プレビューを積み上げた
    /// 縦方向の合計高さがウィンドウの初期サイズを超えたとき、
    /// `CentralPanel` はスクロールしないためボタンごと可視領域外に押し
    /// 出されてしまう（ウィンドウを広げるまで送信ボタンの存在に気付けない、
    /// という実機報告があった）。固定位置に置くことで、ウィンドウサイズに
    /// 関わらず常に見える。
    fn draw_bottom_actions(&mut self, ui: &mut egui::Ui, pending: bool) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let can_send = self.symptom_category.is_some()
                && (self.symptom_category != Some(SymptomCategory::Other)
                    || !self.description.trim().is_empty());
            if ui
                .add_enabled(!pending && can_send, egui::Button::new("送信"))
                .clicked()
            {
                self.start_send();
            }
            if ui.button("プレビューを再生成").clicked() {
                // デバウンス中の保留があれば、これから同期的に再生成する
                // ので不要（残しておくと ~300ms 後にもう一度、同じ重い
                // 再構築が無駄に走る）。
                self.pending_preview_refresh = None;
                if let Ok((json, shrunk)) = self.generated_preview() {
                    self.apply_generated_preview(json, shrunk);
                }
            }
        });
        ui.label(&self.status);
        if let Some(notice) = &self.log_shrink_notice {
            ui.colored_label(egui::Color32::from_rgb(180, 120, 0), notice);
        }
        ui.add_space(4.0);
    }

    /// 見出し・症状カテゴリ・説明欄・添付チェックボックス・プレビュー
    /// エリア。呼び出し側の `ScrollArea` の中に描画される。送信先ホスト名
    /// / エンドポイントURLは表示しない（ユーザー向けに有用な情報ではなく、
    /// 内部実装の詳細を不必要に露出するだけのため）。
    fn draw_form(&mut self, ui: &mut egui::Ui) {
        ui.heading("不具合を報告");
        ui.add_space(6.0);
        ui.label(format!("保存期間: {RETENTION_HINT}"));
        ui.label("保持期間は ADR-095 の未決定事項の暫定値として、調査に必要な期間と削除の見通しを両立する90日を表示しています。");
        ui.add_space(8.0);

        ui.label("症状カテゴリ");
        let previous_category = self.symptom_category;
        egui::ComboBox::from_id_salt("symptom_category")
            .selected_text(
                self.symptom_category
                    .map_or("選択してください", SymptomCategory::label),
            )
            .show_ui(ui, |ui| {
                for category in SymptomCategory::ALL {
                    ui.selectable_value(
                        &mut self.symptom_category,
                        Some(category),
                        category.label(),
                    );
                }
            });
        let category_changed = self.symptom_category != previous_category;

        ui.add_space(8.0);
        ui.label("説明（任意）");
        let desc_changed = ui
            .add(
                egui::TextEdit::multiline(&mut self.description)
                    .desired_rows(5)
                    .lock_focus(true),
            )
            .changed();

        let attachments_changed = self.draw_attachment_checkboxes(ui);

        if category_changed || desc_changed || attachments_changed {
            // 重いプレビュー再生成（ペイロード全体の再シリアライズ、
            // 最大 ~500KB）を毎フレーム同期実行するとテキスト入力に
            // 遅延が出るため、ここでは即座に実行せずデバウンスする
            // （実際の再生成は `update` 末尾で経過時間を見て行う）。
            self.pending_preview_refresh = Some(std::time::Instant::now());
        }

        ui.separator();
        ui.label("送信前プレビュー（この内容を編集してから送信できます。折りたたまれず全文をスクロールして確認できます）");
        egui::ScrollArea::vertical()
            .id_salt("bug_report_preview_scroll")
            .max_height(320.0)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut self.preview_json)
                        .desired_rows(18)
                        .desired_width(f32::INFINITY)
                        .code_editor()
                        .lock_focus(true),
                );
            });
    }
}

impl eframe::App for BugReportApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.fonts_initialized {
            // ウィンドウ自体は `run()` が既に生成・表示済み。ここで初めて
            // CJK フォントを読み込むのは、ウィンドウ生成クロージャの中で
            // 同期的に読み込むとウィンドウの初回表示が数百ms遅れ、
            // Windows がトレイ（バックグラウンドプロセス）由来の新規
            // ウィンドウへ与えるフォアグラウンド表示の猶予を逃してしまう
            // ため（詳細は `fonts_initialized` フィールドのコメント参照）。
            crate::setup_fonts(ctx);
            self.fonts_initialized = true;
            ctx.request_repaint();
        }

        self.poll_send_result();

        // このフレームで最新のプレビュー/通知を描画できるよう、デバウンス
        // 完了判定はパネル描画より前に行う（描画後に行うと、更新結果は
        // 次の repaint まで画面に反映されない — egui は即時モードGUIで、
        // 明示的な repaint 要求か新規入力がない限り再描画しないため）。
        if let Some(since) = self.pending_preview_refresh {
            let elapsed = since.elapsed();
            if elapsed >= PREVIEW_DEBOUNCE {
                self.refresh_preview_if_unedited();
                self.pending_preview_refresh = None;
            } else {
                ctx.request_repaint_after(
                    PREVIEW_DEBOUNCE
                        .checked_sub(elapsed)
                        .unwrap_or(std::time::Duration::ZERO),
                );
            }
        }

        let pending = self.pending.is_some();

        egui::TopBottomPanel::bottom("bug_report_actions")
            .show(ctx, |ui| self.draw_bottom_actions(ui, pending));

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("bug_report_main_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| self.draw_form(ui));
        });

        if pending {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
}

pub(crate) fn run(args: &BugReportArgs) -> eframe::Result<()> {
    let args = args.clone();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([720.0, 760.0])
            .with_min_inner_size([520.0, 420.0])
            .with_title("awase 不具合報告"),
        ..Default::default()
    };
    // `awase-settings` の通常起動（`SettingsApp::new`）は `setup_fonts` で
    // CJK フォントを読み込むが、`--bug-report` 起動はこの `run_native`
    // 呼び出しが独立した別ウィンドウであり同じ呼び出しを経由しないため、
    // 元々は日本語グリフが一切無い egui 既定フォントのままになっていた
    // （「症状カテゴリ」等のラベルや JSON プレビュー中の日本語がトーフ表示
    // ＝文字化けに見える、BUG-72）。
    //
    // コードレビュー指摘（BUG-72 対応の副作用）: フォント読み込みをこの
    // ウィンドウ生成クロージャ内で同期的に行うと、CJK .ttc（数MB）の
    // パースでウィンドウの初回表示が数百ms遅れ、トレイ（バックグラウンド
    // プロセス）から起動されたウィンドウに Windows が与える「新規ウィンドウ
    // へのフォアグラウンド許可」の猶予時間を逃し、ウィンドウが背面のまま
    // 開いて「一瞬表示されてすぐ消えたように見える」実機報告があった
    // （BUG-73）。フォント読み込みは `BugReportApp::update()` の初回フレーム
    // へ遅延させ（`fonts_initialized` フィールド参照）、ウィンドウ生成
    // 自体はここで即座に行う。
    eframe::run_native(
        "awase-bug-report",
        options,
        Box::new(move |_cc| Ok(Box::new(BugReportApp::new(&args)))),
    )
}

fn current_reported_at() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    unix_seconds_to_rfc3339(secs)
}

fn detect_os_version() -> String {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "ver"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Windows".to_owned())
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::consts::OS.to_owned()
    }
}

fn save_failed_payload(body: &str) -> Result<PathBuf, String> {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let path = std::env::temp_dir().join(format!("awase_bug_report_failed_{secs}.json"));
    match std::fs::write(&path, body) {
        Ok(()) => Ok(path),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

fn escape_json_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(target_os = "windows")]
fn send_report(body: &str) -> Result<String, String> {
    winhttp_send_report(body)
}

#[cfg(not(target_os = "windows"))]
fn send_report(_body: &str) -> Result<String, String> {
    Err("WinHTTP 送信は Windows でのみ利用できます".to_owned())
}

#[cfg(target_os = "windows")]
fn winhttp_send_report(body: &str) -> Result<String, String> {
    use windows::Win32::Networking::WinHttp::{
        WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE, WINHTTP_QUERY_FLAG_NUMBER,
        WINHTTP_QUERY_STATUS_CODE, WinHttpCloseHandle, WinHttpConnect, WinHttpOpen,
        WinHttpOpenRequest, WinHttpQueryHeaders, WinHttpReceiveResponse, WinHttpSendRequest,
        WinHttpSetTimeouts,
    };
    use windows::core::{PCWSTR, w};

    struct Handle(*mut core::ffi::c_void);
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
    impl Handle {
        fn new(handle: *mut core::ffi::c_void, label: &str) -> Result<Self, String> {
            if handle.is_null() {
                Err(format!("{label} に失敗しました"))
            } else {
                Ok(Self(handle))
            }
        }
    }

    let bytes = body.as_bytes();
    let body_len = u32::try_from(bytes.len()).map_err(|_| "送信内容が大きすぎます".to_owned())?;
    // NUL終端を含めない: windows crate の WinHttpSendRequest バインディングは
    // `Option<&[u16]>` の `slice.len()` をそのまま `dwHeadersLength`（文字数）
    // として WinHTTP API に渡す（ポインタ渡しではなく明示的な長さ渡し）。
    // NUL終端(0)を含めて collect すると長さが実際の文字数より1多く報告され、
    // 余分な NUL 文字がヘッダー文字列の一部として解釈され、実機で
    // `WinHttpSendRequest: パラメーターが間違っています (0x80070057)` に
    // なることを確認した。
    let headers: Vec<u16> = "Content-Type: application/json\r\n"
        .encode_utf16()
        .collect();

    unsafe {
        let session = Handle::new(
            WinHttpOpen(
                w!("awase-bug-report/1.0"),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                PCWSTR::null(),
                PCWSTR::null(),
                0,
            ),
            "WinHttpOpen",
        )?;
        WinHttpSetTimeouts(session.0, 15_000, 15_000, 30_000, 30_000)
            .map_err(|e| format!("WinHttpSetTimeouts: {e}"))?;
        let connect = Handle::new(
            WinHttpConnect(session.0, w!("report.awase.cc"), 443, 0),
            "WinHttpConnect",
        )?;
        let request = Handle::new(
            WinHttpOpenRequest(
                connect.0,
                w!("POST"),
                w!("/v1/reports"),
                PCWSTR::null(),
                PCWSTR::null(),
                std::ptr::null(),
                WINHTTP_FLAG_SECURE,
            ),
            "WinHttpOpenRequest",
        )?;
        WinHttpSendRequest(
            request.0,
            Some(&headers),
            Some(bytes.as_ptr().cast()),
            body_len,
            body_len,
            0,
        )
        .map_err(|e| format!("WinHttpSendRequest: {e}"))?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut())
            .map_err(|e| format!("WinHttpReceiveResponse: {e}"))?;

        let mut status_code = 0_u32;
        let mut status_len = u32::try_from(std::mem::size_of::<u32>()).unwrap_or(4);
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some((&raw mut status_code).cast()),
            &raw mut status_len,
            std::ptr::null_mut(),
        )
        .map_err(|e| format!("WinHttpQueryHeaders: {e}"))?;

        let response = read_response_body(request.0)?;
        if status_code != 201 {
            return Err(format!("HTTP {status_code}: {response}"));
        }
        parse_report_id(&response)
            .ok_or_else(|| "成功レスポンスに report_id がありません".to_owned())
    }
}

#[cfg(target_os = "windows")]
unsafe fn read_response_body(request: *mut core::ffi::c_void) -> Result<String, String> {
    use windows::Win32::Networking::WinHttp::{WinHttpQueryDataAvailable, WinHttpReadData};

    let mut out = Vec::new();
    loop {
        let mut available = 0_u32;
        unsafe {
            WinHttpQueryDataAvailable(request, &raw mut available)
                .map_err(|e| format!("WinHttpQueryDataAvailable: {e}"))?;
        }
        if available == 0 {
            break;
        }
        let mut buf = vec![0_u8; usize::try_from(available).unwrap_or(0)];
        let mut read = 0_u32;
        unsafe {
            WinHttpReadData(request, buf.as_mut_ptr().cast(), available, &raw mut read)
                .map_err(|e| format!("WinHttpReadData: {e}"))?;
        }
        buf.truncate(usize::try_from(read).unwrap_or(0));
        out.extend_from_slice(&buf);
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}

#[cfg(target_os = "windows")]
fn parse_report_id(response: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(response).ok()?;
    value
        .get("report_id")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod font_guard_tests {
    use super::BugReportApp;

    fn read_own_source() -> String {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        std::fs::read_to_string(std::path::Path::new(manifest_dir).join("src/bug_report.rs"))
            .expect("failed to read src/bug_report.rs")
            .replace("\r\n", "\n")
    }

    /// 回帰テスト(BUG-72): `--bug-report` ウィンドウは `SettingsApp::new()`
    /// を経由しない独立した `eframe::run_native` 呼び出しのため、CJK
    /// フォントを読み込む `setup_fonts()` を明示的に呼ばない限り日本語
    /// グリフが一切無い egui 既定フォントのままになり、「症状カテゴリ」等の
    /// ラベルや JSON プレビュー中の日本語がトーフ表示（文字化けに見える）
    /// になる。通常のユニットテストでは egui のヘッドレス描画を要し検証
    /// しづらいため、`architecture_guard.rs`/`wix_installer_guard.rs` に
    /// 倣いソースファイルの文字列走査で機械的に固定する。
    ///
    /// 回帰テスト(BUG-73): `setup_fonts` を `run_native` のウィンドウ生成
    /// クロージャ内で同期的に呼ぶと、CJK .ttc（数MB）のパースでウィンドウの
    /// 初回表示が数百ms遅れ、トレイ（バックグラウンドプロセス）から起動
    /// されたウィンドウに Windows が与える「新規ウィンドウへの
    /// フォアグラウンド許可」の猶予時間を逃し、ウィンドウが背面のまま開く
    /// （実機で「一瞬表示されてすぐ消えたように見える」と報告）。BUG-72の
    /// 修正時にこの副作用を作り込んだため、「`run_native` のクロージャは
    /// `setup_fonts` を呼ばない（ウィンドウ生成を遅延させない）」ことも
    /// 同時に固定する。
    #[test]
    fn setup_fonts_is_deferred_to_first_update_not_run_native_closure() {
        let src = read_own_source();

        let run_native_pos = src
            .find("eframe::run_native(")
            .expect("bug_report.rs must call eframe::run_native");
        let closure_end = src[run_native_pos..]
            .find("\n    )")
            .map(|i| run_native_pos + i)
            .expect("could not find end of run_native(...) call");
        let closure_body = &src[run_native_pos..closure_end];
        assert!(
            !closure_body.contains("setup_fonts"),
            "run_native()'s window-creation closure must NOT call setup_fonts synchronously \
             (BUG-73: delays the window's first show past Windows' foreground-grant window \
             for background-process-spawned windows); closure body was:\n{closure_body}"
        );

        let update_pos = src
            .find("impl eframe::App for BugReportApp")
            .and_then(|p| src[p..].find("fn update(").map(|i| p + i))
            .expect("BugReportApp must implement eframe::App::update");
        let update_body = &src[update_pos..(update_pos + 800).min(src.len())];
        assert!(
            update_body.contains("setup_fonts(ctx)"),
            "BugReportApp::update() must call setup_fonts(ctx) on its first frame \
             (gated by fonts_initialized) or Japanese text renders as tofu boxes; \
             update() head was:\n{update_body}"
        );
    }

    /// 回帰テスト: `fonts_initialized` は `BugReportApp::new()` の時点では
    /// 常に `false`（フォント読み込みが `update()` の初回フレームへ遅延
    /// されていることの直接確認）。
    #[test]
    fn new_app_has_fonts_not_yet_initialized() {
        let args = super::BugReportArgs {
            journal_path: None,
            ime_kind: awase_windows::bug_report::BugReportImeKind::Unknown,
            diagnostics_path: None,
            app_log_path: None,
        };
        let app = BugReportApp::new(&args);
        assert!(!app.fonts_initialized);
    }
}
