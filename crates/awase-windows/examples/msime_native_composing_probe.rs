#![allow(clippy::all, clippy::pedantic, clippy::nursery)]
//! ADR-199 T17: MS-IME本体で無変換/変換キーの割り当てが「IME-オン/オフ」（トグル、値2）の
//! とき、**composing中（未確定の変換文字列がある状態）**に無変換キーを押すと開閉トグルとして
//! 動くか、それとも変換や無視など別の挙動になるかを実機で直接測定するツール。
//!
//! ADR-199 T17 Phase 4（無変換/変換=2のときawaseが単独タップでIME開閉する配線、
//! `state/key_effect_predictor.rs::KeyEffectKeymap::msime_native_key_role`）の設計を
//! 左右する前提確認として2026-09-28に実施し、composing中はIME-オン/オフの発火自体が起きず、
//! 無変換キー本来の既定動作（かな⇔カタカナ変換）が優先されることを確認した
//! （`docs/adr/199-derive-key-roles-from-user-ime-keymap.md`決定16本文参照）。Windows/MS-IME
//! のアップデートで挙動が変わっていないかの再確認や、他のキー・他の割り当て値での追加検証に
//! 再利用できるよう、恒久的なツールとしてコミットする。
//!
//! 前提: `crates/awase-windows/examples/msime_key_assignment_settings_probe.rs -- --set-master=on`
//! を先に実行し、「キーの割り当て」マスタースイッチをONにしておくこと（無変換/変換の
//! ComboBoxを「IME-オン/オフ」にする場合は同ツールの`--set-muhenkan=`/`--set-henkan=`も使う）。
//!
//! 仕組みは `gji_composition_probe.rs`(既存, ADR-091検証用)と同じ: 自前のEDITウィンドウを
//! 作り、SendInputで物理キー相当を注入し、ImmGetOpenStatus/ImmGetConversionStatus/
//! ImmGetCompositionStringWで実際の状態を直接読む。MS-IME本体のTSFプロファイルへ切り替える
//! (`ime_key_matrix_spike.rs`の`--msime`と同じCLSID/プロファイルGUID)。この切替は
//! `TF_IPPMF_FORSESSION`（ログオンセッション全体・他の実行中アプリにも影響し、プロセス
//! 終了後も残存する。`TF_IPPMF_FORPROCESS`＝このプロセスだけへスコープを絞る方が本来安全
//! だが、2026-09-28にdragonflyg4実機で検証したところSendInputで注入した物理キーがその
//! スコープのプロファイルへ実効的に届かず、`open_status`が常に変化しない＝ツールが機能
//! しなくなる退行を確認したため`FORSESSION`のままにしている。詳細は下記
//! `TF_IPPMF_FORSESSION`定数のコメント参照）で行う。終了時（正常終了・早期return・panicの
//! いずれでも`ProfileRestoreGuard`のDropで）元のプロファイルへ戻す。元のプロファイルが
//! 読めない場合は切り替え自体を行わず中止する（`FORSESSION`のまま残存するリスクを避けるため）。
//! **この保証はCtrl+C・コンソールを閉じる・`taskkill`等でプロセスが終了する場合には及ばない**
//! （Rustの既定ではCtrl+Cはスタック巻き戻し無しでプロセスを終了させるため、Dropは走らない）。
//! `FORSESSION`はログオンセッション全体に効くため、実行中に中断するとデスクトップ全体が
//! MS-IME本体のままになりうる（PR #348再レビュー指摘）。この状態に陥った場合は、
//! `Win+Space`（または`Ctrl+Shift`等、通常のIME切替キー）で手動で元のIMEへ戻すこと。
//!
//! # 既知の未確認事項
//!
//! `ActivateProfile`に渡す`TF_IPPMF_ENABLEPROFILE`フラグは、対象プロファイルが「入力方式の
//! 一覧」から無効化・削除されている場合にそれを再度有効化しうる（PR #348レビュー指摘、
//! 未確認）。MS-IME本体を一覧から意図的に外しているユーザーがこのツールを実行すると、
//! 実行後にMS-IME本体が一覧へ戻ってしまう可能性がある。
//!
//! 実行方法(Windows実機のみ): `cargo run -p awase-windows --example msime_native_composing_probe --release`

#![allow(unsafe_code)]

#[cfg(windows)]
mod windows_probe {
    use std::time::{Duration, Instant};

    use serde::Serialize;
    use windows::core::{Result as WinResult, GUID, PCWSTR};
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows::Win32::UI::Input::Ime::{
        ImmGetCompositionStringW, ImmGetContext, ImmGetConversionStatus, ImmGetOpenStatus,
        ImmGetDefaultIMEWnd, ImmReleaseContext, ImmSetOpenStatus, IME_COMPOSITION_STRING, IME_CONVERSION_MODE, IME_SENTENCE_MODE,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, SetFocus, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
        KEYEVENTF_KEYUP, VIRTUAL_KEY,
    };
    use windows::Win32::UI::TextServices::{
        CLSID_TF_InputProcessorProfiles, ITfInputProcessorProfileMgr, GUID_TFCAT_TIP_KEYBOARD,
        TF_INPUTPROCESSORPROFILE,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetForegroundWindow,
        GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, PeekMessageW,
        RegisterClassW, SendMessageW, SetForegroundWindow, SetWindowTextW, ShowWindow, TranslateMessage,
        CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, MSG, PM_REMOVE, SW_SHOW, WM_DESTROY, WNDCLASSW,
        WS_OVERLAPPEDWINDOW, WS_VISIBLE, WM_IME_CONTROL,
    };

    const WINDOW_CLASS_NAME: &str = "msime_native_composing_probe_window";
    const ES_MULTILINE: u32 = 0x0004;
    const ES_AUTOVSCROLL: u32 = 0x0040;
    const WS_CHILD: u32 = 0x4000_0000;
    const WS_BORDER: u32 = 0x0080_0000;

    const VK_NONCONVERT: u16 = 0x1D;
    const VK_IME_ON: u16 = 0x16;
    const VK_IME_OFF: u16 = 0x1A;

    const TF_PROFILETYPE_INPUTPROCESSOR: u32 = 1;
    const TF_IPPMF_ENABLEPROFILE: u32 = 0x1;
    // PR #348レビューでTF_IPPMF_FORSESSION（ログオンセッション全体、他のアプリ全部に影響し
    // プロセス終了後も残る）からTF_IPPMF_FORPROCESS（このプロセスだけ、プロセス終了で自動的に
    // 元へ戻るはず）へ一度変更したが、2026-09-28にdragonflyg4実機で検証したところ、
    // FORPROCESSスコープではSendInputで注入した物理キー（VK_NONCONVERT等）がそのプロファイルへ
    // 実効的に届かず、direct_input_closedのようなT12で確立済みの最も単純なベースライン
    // シナリオですらopen_statusが0→1に変化しなくなる退行を確認した（ActivateProfile自体は
    // Ok(())を返しており、復元は正常に動作した——プロファイル切替APIの成功と、実際の
    // キー入力ルーティングへの反映は別物だった）。ツールの本来の目的（実際のIME開閉挙動の
    // 観測）が果たせなくなるため、FORSESSIONへ戻す。元のプロファイルへの復元は
    // ProfileRestoreGuardのDropが正常終了・早期return・panicのどの経路でも保証する
    // （復元処理自体の正しさはPR #348レビューの指摘#1/#2で修正済み、このコミットで
    // 引き続き維持）。
    const TF_IPPMF_FORSESSION: u32 = 0x2000_0000;
    // MS-IME本体のCLSID/プロファイルGUID(ime_key_matrix_spike.rsの--msimeと同一)。
    const MSIME_CLSID: u128 = 0x03B5835F_F03C_411B_9CE2_AA23E1171E36;
    const MSIME_PROFILE: u128 = 0xA76C93D9_5523_4E90_AAFA_4DB112F9AC76;

    #[derive(Debug, Clone, Serialize)]
    struct ConvState {
        open_status: Option<bool>,
        conversion_mode: Option<u32>,
        sentence_mode: Option<u32>,
        comp_str: Option<String>,
    }

    #[derive(Debug, Clone, Serialize)]
    struct ScenarioResult {
        scenario: String,
        edit_text: String,
        before: ConvState,
        after: ConvState,
    }

    extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if msg == WM_DESTROY {
            return LRESULT(0);
        }
        // SAFETY: DefWindowProcW はどんな組でも安全に呼べる既定ハンドラ。
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    use awase_windows::win32::to_wide;

    /// stdout に加えてログファイルへも書く（他のプローブ`config_verify_probe.rs`/
    /// `msime_key_assignment_settings_probe.rs`と同様。stdoutのみでは、プローブが
    /// クラッシュ/Ctrl+Cで中断された場合に出力が失われる、opusコードレビュー指摘）。
    fn log(msg: &str) {
        use std::io::Write as _;
        println!("{msg}");
        let _ = std::io::stdout().flush();
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("msime_native_composing_probe.log")
        {
            let _ = writeln!(f, "{msg}");
        }
    }

    /// # Safety
    /// メインスレッドから一度だけ呼ぶこと。
    unsafe fn create_probe_windows() -> anyhow::Result<(HWND, HWND)> {
        let class_name_wide = to_wide(WINDOW_CLASS_NAME);
        let hinstance = windows::Win32::System::LibraryLoader::GetModuleHandleW(None)
            .unwrap_or_default()
            .into();
        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinstance,
            lpszClassName: PCWSTR(class_name_wide.as_ptr()),
            ..Default::default()
        };
        let atom = unsafe { RegisterClassW(&raw const wc) };
        anyhow::ensure!(atom != 0, "RegisterClassW failed for parent window");

        let title = to_wide("MS-IME Native Composing Probe");
        let parent = unsafe {
            CreateWindowExW(
                windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE::default(),
                PCWSTR(class_name_wide.as_ptr()),
                PCWSTR(title.as_ptr()),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                600,
                300,
                None,
                None,
                Some(hinstance),
                None,
            )
        }
        .map_err(|e| anyhow::anyhow!("CreateWindowExW (parent) failed: {e}"))?;

        let edit_class = to_wide("EDIT");
        let edit_style = windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE(
            WS_CHILD | WS_VISIBLE.0 | WS_BORDER | ES_MULTILINE | ES_AUTOVSCROLL,
        );
        let edit = unsafe {
            CreateWindowExW(
                windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE::default(),
                PCWSTR(edit_class.as_ptr()),
                PCWSTR::null(),
                edit_style,
                0,
                0,
                580,
                260,
                Some(parent),
                None,
                Some(hinstance),
                None,
            )
        }
        .map_err(|e| anyhow::anyhow!("CreateWindowExW (edit) failed: {e}"))?;
        Ok((parent, edit))
    }

    /// # Safety
    /// `hwnd` は有効なウィンドウハンドルであること。
    unsafe fn force_foreground(hwnd: HWND) {
        let fg = unsafe { GetForegroundWindow() };
        let mut fg_thread_pid = 0u32;
        let fg_thread_id = unsafe { GetWindowThreadProcessId(fg, Some(&raw mut fg_thread_pid)) };
        let my_thread_id = unsafe { GetCurrentThreadId() };
        let attached = if fg_thread_id != 0 && fg_thread_id != my_thread_id {
            unsafe { AttachThreadInput(my_thread_id, fg_thread_id, true) }.as_bool()
        } else {
            false
        };
        let _ = unsafe { ShowWindow(hwnd, SW_SHOW) };
        let _ = unsafe { SetForegroundWindow(hwnd) };
        if attached {
            let _ = unsafe { AttachThreadInput(my_thread_id, fg_thread_id, false) };
        }
    }

    /// # Safety
    /// SendInputはシステムの入力キューに入り、その時点の前面ウィンドウに届く（プロセス単位では
    /// なくOS単位の副作用）。`force_foreground`でこのプローブの`parent`ウィンドウを前面にした
    /// 直後の短い時間だけ呼ぶことを前提にしている。ユーザーが操作中に別ウィンドウへフォーカスを
    /// 奪われると、Esc/a/i/k/無変換の注入がそちらに届く。テスト目的でのみ呼ぶこと。
    unsafe fn send_vk(vk: u16, keyup: bool) {
        let flags = if keyup {
            KEYEVENTF_KEYUP
        } else {
            KEYBD_EVENT_FLAGS(0)
        };
        let input = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(vk),
                    wScan: 0,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        let size = i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32");
        unsafe { SendInput(&[input], size) };
    }

    unsafe fn send_vk_tap(vk: u16) {
        unsafe {
            send_vk(vk, false);
            send_vk(vk, true);
        }
    }

    fn send_ascii_tap(c: char) {
        let upper = c.to_ascii_uppercase();
        let vk = match upper {
            'A'..='Z' | '0'..='9' => u16::from(upper as u8),
            _ => return,
        };
        unsafe { send_vk_tap(vk) };
    }

    fn pump_messages(duration: Duration) {
        let deadline = Instant::now() + duration;
        while Instant::now() < deadline {
            let mut msg = MSG::default();
            // SAFETY: msg はスタック上の有効な MSG バッファ。
            while unsafe { PeekMessageW(&raw mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
                let _ = unsafe { TranslateMessage(&raw const msg) };
                unsafe { DispatchMessageW(&raw const msg) };
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn get_edit_text(hwnd: HWND) -> String {
        // SAFETY: hwnd は create_probe_windows で作成した有効な EDIT コントロール。
        let len = unsafe { GetWindowTextLengthW(hwnd) };
        if len <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; usize::try_from(len).unwrap_or(0) + 1];
        let written = unsafe { GetWindowTextW(hwnd, &mut buf) };
        let written = usize::try_from(written).unwrap_or(0);
        String::from_utf16_lossy(&buf[..written])
    }

    fn clear_edit_text(hwnd: HWND) {
        let empty = to_wide("");
        // SAFETY: hwnd は有効な EDIT コントロール。empty は NUL 終端済み。
        let _ = unsafe { SetWindowTextW(hwnd, PCWSTR(empty.as_ptr())) };
    }

    /// # Safety
    /// `himc` は `ImmGetContext` で得た有効な HIMC であること。
    unsafe fn read_comp_str(himc: windows::Win32::UI::Input::Ime::HIMC) -> Option<String> {
        const GCS_COMPSTR: u32 = 0x0008;
        let byte_len =
            unsafe { ImmGetCompositionStringW(himc, IME_COMPOSITION_STRING(GCS_COMPSTR), None, 0) };
        if byte_len < 0 {
            return None;
        }
        let byte_len = usize::try_from(byte_len).unwrap_or(0);
        if byte_len == 0 {
            return Some(String::new());
        }
        let mut buf = vec![0u16; byte_len.div_ceil(2)];
        let written = unsafe {
            ImmGetCompositionStringW(
                himc,
                IME_COMPOSITION_STRING(GCS_COMPSTR),
                Some(buf.as_mut_ptr().cast()),
                u32::try_from(buf.len() * 2).unwrap_or(0),
            )
        };
        if written <= 0 {
            return None;
        }
        let char_count = usize::try_from(written).unwrap_or(0) / 2;
        Some(String::from_utf16_lossy(&buf[..char_count]))
    }

    fn read_conv_state(hwnd: HWND) -> ConvState {
        // SAFETY: hwnd は create_probe_windows で作成した有効なウィンドウ。
        let himc = unsafe { ImmGetContext(hwnd) };
        if himc.is_invalid() {
            return ConvState {
                open_status: None,
                conversion_mode: None,
                sentence_mode: None,
                comp_str: None,
            };
        }
        // SAFETY: himc は有効。
        let open_status = Some(unsafe { ImmGetOpenStatus(himc) }.as_bool());
        let mut conv = IME_CONVERSION_MODE::default();
        let mut sent = IME_SENTENCE_MODE::default();
        // SAFETY: himc は有効。書き込み先は両方 null でない。
        let ok = unsafe { ImmGetConversionStatus(himc, Some(&raw mut conv), Some(&raw mut sent)) };
        let (conversion_mode, sentence_mode) = if ok.as_bool() {
            (Some(conv.0), Some(sent.0))
        } else {
            (None, None)
        };
        // SAFETY: himc は有効。
        let comp_str = unsafe { read_comp_str(himc) };
        // SAFETY: hwnd/himc は対応する有効なペア。
        let _ = unsafe { ImmReleaseContext(hwnd, himc) };
        ConvState {
            open_status,
            conversion_mode,
            sentence_mode,
            comp_str,
        }
    }

    const VK_ESCAPE: u16 = 0x1B;

    fn run_scenario(name: &str, edit: HWND, setup: impl FnOnce(HWND)) -> ScenarioResult {
        // 前のシナリオの未確定composition(あれば)をEscでキャンセルしてから始める。
        // clear_edit_text(SetWindowTextW)だけではTSF側のcomposition overlayは
        // リセットされず、次のシナリオへ状態が混入する(1回目の実行で確認)。
        // SAFETY: テスト目的での注入。
        unsafe { send_vk_tap(VK_ESCAPE) };
        pump_messages(Duration::from_millis(200));
        clear_edit_text(edit);
        pump_messages(Duration::from_millis(200));
        setup(edit);
        pump_messages(Duration::from_millis(300));
        let before = read_conv_state(edit);
        // SAFETY: テスト目的での無変換キー注入。
        unsafe { send_vk_tap(VK_NONCONVERT) };
        pump_messages(Duration::from_millis(300));
        let after = read_conv_state(edit);
        ScenarioResult {
            scenario: name.to_string(),
            edit_text: get_edit_text(edit),
            before,
            after,
        }
    }


    /// `--off-methods`(ADR-221 D1、BUG-184 / issue #138): 未確定文字(「か」)がある状態で、IME を閉じる手段ごとに
    /// 未確定文字・EDIT の本文・開閉がどうなるかを測る。1 行 1 試行の `OFFM {json}` を出す。
    /// 手段は awase が実際に使う機構に対応する: `vk1a`(MsImeDirect の VK_IME_OFF)、`vk1a_x2`(Edge で見られた 2 回送信)、
    /// `f0`(素の英数キー)、`imm_setopen0`(同一プロセスの ImmSetOpenStatus(FALSE)、ImmCross 相当)、
    /// `wm_imc0`(他プロセス向けの WM_IME_CONTROL IMC_SETOPENSTATUS 0、ImmCross の実機構)、
    /// `vk1a_then_imc0`(`fix/msime-off-composition-imc` の試作: VK_IME_OFF の後に IMC を補う)、`none`(対照)。
    fn run_off_methods(edit: HWND, trials: usize) {
        const IMC_SETOPENSTATUS: usize = 0x0006;
        let wm_imc0 = |edit: HWND| {
            // SAFETY: edit は有効な EDIT コントロール。デフォルト IME ウィンドウへ WM_IME_CONTROL を送る。
            let ime_wnd = unsafe { ImmGetDefaultIMEWnd(edit) };
            if ime_wnd.0.is_null() {
                return;
            }
            let _ = unsafe {
                SendMessageW(ime_wnd, WM_IME_CONTROL, Some(WPARAM(IMC_SETOPENSTATUS)), Some(LPARAM(0)))
            };
        };
        let methods: [&str; 7] = [
            "none",
            "vk1a",
            "vk1a_x2",
            "f0",
            "imm_setopen0",
            "wm_imc0",
            "vk1a_then_imc0",
        ];
        for method in methods {
            for n in 0..trials {
                // 前の試行の未確定文字を片づけて、IME を開いた状態から始める。
                // SAFETY: テスト目的での注入。
                unsafe { send_vk_tap(VK_ESCAPE) };
                pump_messages(Duration::from_millis(200));
                clear_edit_text(edit);
                unsafe { send_vk_tap(VK_IME_ON) };
                pump_messages(Duration::from_millis(300));
                send_ascii_tap('k');
                pump_messages(Duration::from_millis(120));
                send_ascii_tap('a');
                pump_messages(Duration::from_millis(300));
                let before = read_conv_state(edit);
                match method {
                    "vk1a" => unsafe { send_vk_tap(VK_IME_OFF) },
                    "vk1a_x2" => {
                        unsafe { send_vk_tap(VK_IME_OFF) };
                        pump_messages(Duration::from_millis(150));
                        unsafe { send_vk_tap(VK_IME_OFF) };
                    }
                    "f0" => unsafe { send_vk_tap(0xF0) },
                    "imm_setopen0" => {
                        // SAFETY: edit は有効。himc は ImmGetContext で得て直後に解放する。
                        let himc = unsafe { ImmGetContext(edit) };
                        if !himc.is_invalid() {
                            let _ = unsafe { ImmSetOpenStatus(himc, false) };
                            let _ = unsafe { ImmReleaseContext(edit, himc) };
                        }
                    }
                    "wm_imc0" => wm_imc0(edit),
                    "vk1a_then_imc0" => {
                        unsafe { send_vk_tap(VK_IME_OFF) };
                        pump_messages(Duration::from_millis(100));
                        wm_imc0(edit);
                    }
                    _ => {}
                }
                pump_messages(Duration::from_millis(500));
                let after = read_conv_state(edit);
                let text_mid = get_edit_text(edit);
                // 残った未確定文字があれば Enter で確定させ、最終的に本文に何が残るかを見る。
                unsafe { send_vk_tap(0x0D) };
                pump_messages(Duration::from_millis(400));
                let text_final = get_edit_text(edit);
                let rec = serde_json::json!({
                    "method": method, "n": n,
                    "before": before, "after": after,
                    "text_mid": text_mid, "text_final": text_final,
                });
                log(&format!("OFFM {rec}"));
            }
        }
        log("[off-methods] done");
    }

    /// 現在アクティブなキーボードTIPプロファイルを取得する。
    ///
    /// # Safety
    /// TSF が初期化済み(CoInitializeEx済み)であること。
    unsafe fn get_active_profile(
        mgr: &ITfInputProcessorProfileMgr,
    ) -> WinResult<TF_INPUTPROCESSORPROFILE> {
        let mut p = TF_INPUTPROCESSORPROFILE::default();
        // SAFETY: p はスタック上の有効なバッファ。
        unsafe { mgr.GetActiveProfile(&GUID_TFCAT_TIP_KEYBOARD, &raw mut p) }?;
        Ok(p)
    }

    /// レビュー指摘(高)反映: 復元時に`langid`/`dwProfileType`/`hkl`を`0x0411`/固定値で決め打ちに
    /// せず、`get_active_profile`が読んだ値をそのまま渡す。元がキーボードレイアウト型の
    /// プロファイル（`clsid`/`guidProfile`が`GUID_NULL`）だった場合、これを
    /// `TF_PROFILETYPE_INPUTPROCESSOR`・固定`langid`で活性化しようとすると失敗し、
    /// MS-IME本体のまま戻らなくなる。
    ///
    /// # Safety
    /// TSF が初期化済みであること。`hkl`は呼び出し元が渡した有効な値（`HKL(null)`でもよい）。
    unsafe fn activate_profile(
        mgr: &ITfInputProcessorProfileMgr,
        profile_type: u32,
        langid: u16,
        clsid: GUID,
        profile: GUID,
        hkl: windows::Win32::UI::Input::KeyboardAndMouse::HKL,
        flags: u32,
    ) -> WinResult<()> {
        // SAFETY: mgr は有効な COM オブジェクト、hkl は呼び出し元が渡した有効な値。
        let r = unsafe { mgr.ActivateProfile(profile_type, langid, &clsid, &profile, hkl, flags) };
        log(&format!(
            "[tsf] ActivateProfile(type={profile_type} langid=0x{langid:04X} clsid={clsid:?} profile={profile:?} flags=0x{flags:X}) -> {r:?}"
        ));
        std::thread::sleep(Duration::from_millis(1500));
        r
    }

    /// [`activate_profile`]で元のプロファイルへ戻すことを保証するRAIIガード。
    /// レビュー指摘(中〜高)反映: 復元処理を`run()`の正常終了パスの末尾だけに置くと、
    /// `create_probe_windows()?`・JSON書き出し・ファイル書き込み等の早期returnや、
    /// 万一のpanicで復元がスキップされる。Dropに置くことでどの経路でも復元を試みる。
    struct ProfileRestoreGuard<'a> {
        mgr: &'a ITfInputProcessorProfileMgr,
        original: TF_INPUTPROCESSORPROFILE,
    }

    impl Drop for ProfileRestoreGuard<'_> {
        fn drop(&mut self) {
            log("[tsf] restoring original profile (guard)...");
            let p = self.original;
            // SAFETY: mgr は run() が保持している有効な COM オブジェクト。hkl は
            //         get_active_profile が返した値をそのまま渡す。
            let restore_result = unsafe {
                activate_profile(
                    self.mgr,
                    p.dwProfileType,
                    p.langid,
                    p.clsid,
                    p.guidProfile,
                    p.hkl,
                    TF_IPPMF_ENABLEPROFILE | TF_IPPMF_FORSESSION,
                )
            };
            // PR #348再レビュー指摘: 復元の失敗を`let _ =`で握り潰すと、デスクトップ全体が
            // MS-IME本体のまま残っていることにユーザーが気づけない。目立つログを出す。
            if let Err(e) = restore_result {
                log(&format!(
                    "RESULT: restore FAILED ({e:?}) — このセッションはMS-IME本体のままの可能性があります。Win+Space（または通常のIME切替キー）で手動で元のIMEへ切り替えてください。"
                ));
            }
        }
    }

    pub(super) fn run() -> anyhow::Result<()> {
        // SAFETY: プロセス起動直後の唯一の COM 初期化呼び出し。
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
            .ok()
            .map_err(|e| anyhow::anyhow!("CoInitializeEx failed: {e}"))?;

        let mgr: ITfInputProcessorProfileMgr =
            // SAFETY: COM 初期化済み。
            unsafe { CoCreateInstance(&CLSID_TF_InputProcessorProfiles, None, CLSCTX_INPROC_SERVER) }?;
        // SAFETY: mgr は直前に取得した有効な COM オブジェクト。
        let Ok(original_profile) = (unsafe { get_active_profile(&mgr) }) else {
            // レビュー指摘(高)反映: 元のプロファイルが読めないまま切り替えると、復元先が
            // 分からず「MS-IME本体のまま戻せない」事故になりうる。読めないときは切り替え
            // 自体を行わずに中止する。
            anyhow::bail!(
                "could not read the original active TIP profile; aborting before switching to MS-IME to avoid leaving the session stuck on it"
            );
        };
        log(&format!(
            "[tsf] original active profile: {original_profile:?}"
        ));
        // `_restore_guard`のDropが、以降のどの終了経路（正常終了・`?`による早期return・panic）
        // でも元のプロファイルへ戻すことを保証する（レビュー指摘、中〜高）。
        let _restore_guard = ProfileRestoreGuard {
            mgr: &mgr,
            original: original_profile,
        };

        log("[tsf] switching to MS-IME native profile (session-wide, TF_IPPMF_FORSESSION)...");
        // SAFETY: mgr は有効な COM オブジェクト、hkl はテスト用の値。この切替はログオン
        //         セッション全体に効く（TF_IPPMF_FORSESSION、上記モジュールdoc参照）。
        unsafe {
            activate_profile(
                &mgr,
                TF_PROFILETYPE_INPUTPROCESSOR,
                0x0411,
                GUID::from_u128(MSIME_CLSID),
                GUID::from_u128(MSIME_PROFILE),
                windows::Win32::UI::Input::KeyboardAndMouse::HKL(std::ptr::null_mut()),
                TF_IPPMF_ENABLEPROFILE | TF_IPPMF_FORSESSION,
            )
        }?;

        // SAFETY: メインスレッドから一度だけウィンドウを作成する。
        let (parent, edit) = unsafe { create_probe_windows() }?;
        // SAFETY: parent は直前に作成した有効なウィンドウ。
        unsafe { force_foreground(parent) };
        pump_messages(Duration::from_millis(300));
        let _ = unsafe { SetFocus(Some(edit)) };
        pump_messages(Duration::from_millis(300));

        if std::env::args().any(|a| a == "--off-methods") {
            let trials = std::env::args()
                .find_map(|a| a.strip_prefix("--trials=").and_then(|v| v.parse().ok()))
                .unwrap_or(5);
            run_off_methods(edit, trials);
            return Ok(());
        }

        let mut results = Vec::new();

        // (a) 直接入力(閉じている)状態 → 無変換 (T12既知: 開く、0→1)
        results.push(run_scenario("direct_input_closed", edit, |e| {
            // SAFETY: テスト目的での注入。
            unsafe { send_vk_tap(VK_IME_OFF) };
            let _ = e;
        }));

        // (b) 開いていて入力なし(アイドル) → 無変換 (T12既知: 閉じる、1→0)
        results.push(run_scenario("open_idle", edit, |e| {
            // SAFETY: テスト目的での注入。
            unsafe { send_vk_tap(VK_IME_ON) };
            let _ = e;
        }));

        // (c) 開いていて入力中(「あい」未確定) → 無変換 ★M1の核心、実機未確認だった論点
        results.push(run_scenario("open_composing_ai", edit, |_e| {
            // SAFETY: テスト目的での注入。
            unsafe { send_vk_tap(VK_IME_ON) };
            pump_messages(Duration::from_millis(200));
            send_ascii_tap('a');
            pump_messages(Duration::from_millis(150));
            send_ascii_tap('i');
        }));

        // (d) 開いていて入力中、まだ子音だけ(「k」未確定、かな1文字にすら達していない)
        results.push(run_scenario("open_composing_k_only", edit, |_e| {
            // SAFETY: テスト目的での注入。
            unsafe { send_vk_tap(VK_IME_ON) };
            pump_messages(Duration::from_millis(200));
            send_ascii_tap('k');
        }));

        for r in &results {
            log(&format!(
                "[result] scenario={} edit_text={:?} before(open={:?} comp={:?}) after(open={:?} comp={:?})",
                r.scenario,
                r.edit_text,
                r.before.open_status,
                r.before.comp_str,
                r.after.open_status,
                r.after.comp_str,
            ));
        }
        let json = serde_json::to_string_pretty(&results)?;
        std::fs::write("msime_native_composing_probe_result.json", &json)?;
        log("[done] wrote msime_native_composing_probe_result.json");

        // 元のプロファイルへの復元は `_restore_guard` の Drop（関数の終わりで発火）が行う。
        Ok(())
    }
}

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    windows_probe::run()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("this probe is windows-only");
}
