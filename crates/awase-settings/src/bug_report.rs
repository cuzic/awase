use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};

use awase_windows::bug_report::{
    BugReportDiagnostics, BugReportImeKind, BugReportInput, ENDPOINT_URL, REPORT_HOST,
    RETENTION_HINT, SymptomCategory, build_payload_json, unix_seconds_to_rfc3339,
};
use eframe::egui;

#[derive(Debug, Clone)]
pub(crate) struct BugReportArgs {
    pub(crate) journal_path: Option<PathBuf>,
    pub(crate) ime_kind: BugReportImeKind,
    pub(crate) diagnostics_path: Option<PathBuf>,
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
    ime_kind: BugReportImeKind,
    diagnostics: BugReportDiagnostics,
    os_version: String,
    reported_at: String,
    preview_json: String,
    last_generated_preview: String,
    status: String,
    pending: Option<Receiver<SendOutcome>>,
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
            ime_kind: args.ime_kind,
            diagnostics,
            os_version,
            reported_at,
            preview_json: String::new(),
            last_generated_preview: String::new(),
            status: "症状カテゴリを選択して、送信前の内容を確認してください。".to_owned(),
            pending: None,
        };
        app.refresh_preview_if_unedited();
        app
    }

    fn refresh_preview_if_unedited(&mut self) {
        if self.preview_json != self.last_generated_preview {
            return;
        }
        match self.generated_preview() {
            Ok(json) => {
                self.preview_json.clone_from(&json);
                self.last_generated_preview = json;
            }
            Err(e) => {
                let json = format!("{{\n  \"error\": \"{}\"\n}}", escape_json_string(&e));
                self.preview_json.clone_from(&json);
                self.last_generated_preview = json;
            }
        }
    }

    fn generated_preview(&self) -> Result<String, String> {
        let symptom_category = self
            .symptom_category
            .ok_or_else(|| "症状カテゴリを選択してください".to_owned())?;
        build_payload_json(&BugReportInput {
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
            state_snapshot: self.diagnostics.state_snapshot.clone(),
            attach_state_snapshot: self.attach_state_snapshot,
            config_toml: self.diagnostics.config_toml.as_deref(),
            attach_config: self.attach_config,
            layout_yab: self.diagnostics.layout_yab.as_deref(),
            attach_layout: self.attach_layout,
            reported_at: &self.reported_at,
        })
        .map_err(|e| e.to_string())
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
        let body = self.preview_json.clone();
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

impl eframe::App for BugReportApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_send_result();

        let pending = self.pending.is_some();
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("不具合を報告");
            ui.add_space(6.0);
            ui.label(format!("送信先: {REPORT_HOST}"));
            ui.label(format!("エンドポイント: {ENDPOINT_URL}"));
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

            let attach_log_changed = ui
                .checkbox(&mut self.attach_log, "journal ログを添付する")
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

            if category_changed
                || desc_changed
                || attach_log_changed
                || attach_state_snapshot_changed
                || attach_config_changed
                || attach_layout_changed
            {
                self.refresh_preview_if_unedited();
            }

            ui.separator();
            ui.label("送信前プレビュー（この内容を編集してから送信できます）");
            ui.add(
                egui::TextEdit::multiline(&mut self.preview_json)
                    .desired_rows(18)
                    .code_editor()
                    .lock_focus(true),
            );

            ui.separator();
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
                if ui.button("プレビューを再生成").clicked()
                    && let Ok(json) = self.generated_preview()
                {
                    self.preview_json.clone_from(&json);
                    self.last_generated_preview = json;
                }
            });
            ui.label(&self.status);
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
    let headers: Vec<u16> = "Content-Type: application/json\r\n"
        .encode_utf16()
        .chain(std::iter::once(0))
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
