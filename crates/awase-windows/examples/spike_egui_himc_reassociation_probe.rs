//! [ADR-125](../../../docs/adr/125-egui-winit-dynamic-ime-association-focus-model-gap.md)
//! の未確定な点を実機で裏取りするための使い捨てスパイク。
//!
//! ## 検証したいこと
//!
//! ADR-125 は `winit`/`egui-winit` のソースコード読解（`~/.cargo/registry`
//! にキャッシュされたクレート本体の読解であり、実機での動作確認ではない）から、
//! 次の仮説を立てた:
//!
//! > `egui-winit` はフォーカス中のテキストウィジェットの有無に応じて毎フレーム
//! > `window.set_ime_allowed(bool)` を呼び、`winit` はこれを**同一 HWND に対する**
//! > `ImmAssociateContextEx(IACE_DEFAULT/IACE_CHILDREN)`（IME コンテキストの脱着）
//! > として実装している。つまり `awase-settings.exe` のようなウィンドウでは、
//! > **トップレベル HWND は変わらないまま**、ウィジェット単位のフォーカス移動
//! > だけで HIMC（IME コンテキストハンドル）が作り直されることがある。
//!
//! これが本当なら、`ImmGetContext(hwnd)` で読める HIMC の値は、
//! `GetForegroundWindow()` の戻り値（トップレベル HWND）が**変化していない**
//! 区間でも変化するはずである。このスパイクはそれを直接観測する。
//!
//! ## 実行方法（Windows 実機のみ）
//!
//! ```powershell
//! cargo run -p awase-windows --example spike_egui_himc_reassociation_probe --release
//! ```
//!
//! 起動すると非表示ウィンドウを1つ作り、100ms間隔で
//! `GetForegroundWindow()` + `ImmGetContext(そのhwnd)` をポーリングする。
//! **フォーカス中の HWND、または HIMC の値が変化した瞬間だけ**標準出力に
//! 1行ログを出す（変化なしはログしないので、長時間観察してもスクロールで
//! 埋もれない）。
//!
//! ### 手順
//!
//! 1. このプローブを起動したまま `awase-settings.exe`（設定画面、または
//!    `--bug-report` で起動した不具合報告画面）にフォーカスを移す。
//! 2. 症状欄・説明欄のテキストボックスをクリックし、そのまま連続して文字を
//!    入力する（IME ON でひらがなを入力する）。
//! 3. 次に、症状カテゴリの `ComboBox` を開く／チェックボックスをクリックする
//!    等、テキストボックス以外のウィジェットへフォーカスを移す。
//! 4. その後、再びテキストボックスをクリックして入力を続ける。
//! 5. 手順2〜4を繰り返しながらログを観察する。
//!
//! ### 見るべきポイント
//!
//! - `[FOCUS]` 行（`hwnd` が変化した行）は、Alt-Tab 等でトップレベル
//!   ウィンドウそのものが変わったときだけ出るはず。手順2〜4の間、
//!   `awase-settings.exe` の中でウィジェット間をクリックしているだけなら
//!   `[FOCUS]` 行は出ないはず（同じトップレベル HWND のまま）。
//! - **`[HIMC]` 行（`hwnd` は同じだが `himc` の値が変化した行）が、手順2〜4の
//!   ウィジェット間フォーカス移動のたびに出るかどうかが本スパイクの本題。**
//!   - 出る → ADR-125 の仮説（同一 HWND 内でウィジェット単位に HIMC が
//!     脱着される）が実機で確認できたことになる。手順2（連続入力中）だけの
//!     間は出ず、手順3（ウィジェット間移動）の瞬間にだけ出るか、それとも
//!     連続入力中にも出るか、の頻度差も記録しておくこと（ADR-125
//!     「未確定な点」1）。
//!   - 一切出ない → 少なくともこのスパイクの観測方法ではこの機構は
//!     確認できない。ADR-125 の仮説は棄却または別の説明を要する
//!     （「重さ」「かな混入」の原因は他所にある可能性が高くなる）。
//!
//! CapsLock を押すと `[CAPS]` 行が出る（`spike_kana_lock_probe.rs` と同じ、
//! 比較対象——プローブ自体が機能していることの裏取り用。本題とは無関係）。
//!
//! Ctrl+C またはウィンドウを閉じて終了する。

#![allow(unsafe_code)]

#[cfg(windows)]
mod probe {
    use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
        QueryFullProcessImageNameW,
    };
    use windows::Win32::UI::Input::Ime::{ImmGetContext, ImmReleaseContext};
    use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyState;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClassNameW, GetForegroundWindow,
        GetMessageW, GetWindowThreadProcessId, KillTimer, MSG, PostQuitMessage, RegisterClassW,
        SetTimer, TranslateMessage, WM_DESTROY, WM_TIMER, WNDCLASSW, WS_OVERLAPPEDWINDOW,
    };
    use windows::core::{PCWSTR, PWSTR};

    /// CapsLock。プローブ健全性の裏取り用（`spike_kana_lock_probe.rs` と同じ用途）。
    const VK_CAPITAL: i32 = 0x14;

    const WINDOW_CLASS_NAME: &str = "spike_egui_himc_reassociation_probe_window";
    const TIMER_ID: usize = 1;
    const POLL_INTERVAL_MS: u32 = 100;

    fn to_wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn now() -> String {
        // chrono 等を新規依存にしないため、std のみで簡易フォーマット。
        let dur = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let secs_of_day = dur.as_secs() % 86400;
        let h = secs_of_day / 3600;
        let m = (secs_of_day % 3600) / 60;
        let s = secs_of_day % 60;
        let ms = dur.subsec_millis();
        format!("{h:02}:{m:02}:{s:02}.{ms:03}")
    }

    fn class_name_of(hwnd: HWND) -> String {
        let mut buf = [0u16; 256];
        // SAFETY: hwnd はこの関数の呼出元（poll_and_log_on_change）で
        //         GetForegroundWindow が返した値をそのまま渡している。
        //         null の場合 GetClassNameW は 0 を返し、以降のガードで弾く。
        //         buf はスタック上の有効なバッファ。
        let len = unsafe { GetClassNameW(hwnd, &mut buf) };
        if len <= 0 {
            return "?".to_string();
        }
        String::from_utf16_lossy(&buf[..len.cast_unsigned() as usize])
    }

    /// プロセス実行ファイル名を取得する。`focus/classify.rs::get_process_name`
    /// と同じロジックだが、本体クレートに依存させたくないためこのスパイク内で
    /// 独立に持つ（`spike_kana_lock_probe.rs` の VK_KANA リテラルと同じ方針）。
    fn process_name_of(hwnd: HWND) -> String {
        let mut pid: u32 = 0;
        // SAFETY: hwnd はこのファイル内で GetForegroundWindow が返した値。
        //         pid はスタック上の有効な u32 変数へのポインタ。
        unsafe { GetWindowThreadProcessId(hwnd, Some(&raw mut pid)) };
        if pid == 0 {
            return "?".to_string();
        }
        // SAFETY: pid は直前に取得した有効なプロセス ID。
        //         PROCESS_QUERY_LIMITED_INFORMATION は最小権限。
        let Ok(handle) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) })
        else {
            return "?".to_string();
        };
        let mut buf = [0u16; 260];
        let mut len = buf.len() as u32;
        // SAFETY: handle は直前に取得した有効なプロセスハンドル。buf は
        //         スタック上の有効なバッファで、len に容量を渡している。
        let ok = unsafe {
            QueryFullProcessImageNameW(
                handle,
                PROCESS_NAME_WIN32,
                PWSTR(buf.as_mut_ptr()),
                &mut len,
            )
        };
        // SAFETY: handle は上で取得した有効なハンドルで、ここでのみ閉じる。
        let _ = unsafe { CloseHandle(handle) };
        if ok.is_err() {
            return "?".to_string();
        }
        let full_path = String::from_utf16_lossy(&buf[..len as usize]);
        full_path
            .rsplit('\\')
            .next()
            .unwrap_or(&full_path)
            .to_string()
    }

    /// `hwnd` に現在アタッチされている HIMC を、ポインタ値として読む
    /// （`0` = 脱着中/IME 無し）。`ImmGetContext`/`ImmReleaseContext` は
    /// 対にする（`imm.rs::ImmContextGuard` と同じ RAII パターンを、依存を
    /// 増やさずこのスパイク内で手動で行う）。
    fn himc_of(hwnd: HWND) -> usize {
        // SAFETY: hwnd はこのファイル内で GetForegroundWindow が返した値。
        //         ImmGetContext は他プロセスのウィンドウに対しても安全に
        //         呼べる（awase 本体が ime.rs で同じ手法をクロスプロセスで
        //         使っている、ADR-125 参照）。
        let himc = unsafe { ImmGetContext(hwnd) };
        let raw = himc.0 as usize;
        if raw != 0 {
            // SAFETY: himc は直前に ImmGetContext が返した値と対にして
            //         リリースする。
            let _ = unsafe { ImmReleaseContext(hwnd, himc) };
        }
        raw
    }

    thread_local! {
        static PREV_HWND: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
        static PREV_HIMC: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
        static PREV_CAPS_BIT: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
    }

    fn poll_and_log_on_change() {
        // SAFETY: GetForegroundWindow は引数なしで常に安全。
        let hwnd = unsafe { GetForegroundWindow() };
        let hwnd_raw = hwnd.0 as isize;
        let himc_raw = if hwnd.0.is_null() { 0 } else { himc_of(hwnd) };

        let prev_hwnd = PREV_HWND.get();
        let prev_himc = PREV_HIMC.get();

        if hwnd_raw != prev_hwnd {
            println!(
                "{} [FOCUS] hwnd 0x{:X} -> 0x{:X} class={} process={} himc=0x{:X}",
                now(),
                prev_hwnd,
                hwnd_raw,
                class_name_of(hwnd),
                process_name_of(hwnd),
                himc_raw,
            );
        } else if himc_raw != prev_himc {
            println!(
                "{} [HIMC] hwnd=0x{:X} class={} process={} himc 0x{:X} -> 0x{:X}  <-- 同一トップレベルHWNDのままHIMCが変化(ADR-125仮説の本題)",
                now(),
                hwnd_raw,
                class_name_of(hwnd),
                process_name_of(hwnd),
                prev_himc,
                himc_raw,
            );
        }
        PREV_HWND.set(hwnd_raw);
        PREV_HIMC.set(himc_raw);

        // SAFETY: GetKeyState はどのスレッドからも安全に呼べる。
        let caps_raw = unsafe { GetKeyState(VK_CAPITAL) };
        let caps_bit = (caps_raw & 1) != 0;
        if PREV_CAPS_BIT.get() != Some(caps_bit) {
            println!(
                "{} [CAPS] bit={} [比較対象・プローブ健全性の裏取り、本題とは無関係]",
                now(),
                if caps_bit { "on" } else { "off" }
            );
            PREV_CAPS_BIT.set(Some(caps_bit));
        }
    }

    extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        match msg {
            WM_TIMER => {
                poll_and_log_on_change();
                LRESULT(0)
            }
            WM_DESTROY => {
                // SAFETY: hwnd はこのウィンドウ自身、TIMER_ID は SetTimer で
                // 使ったものと同一。
                let _ = unsafe { KillTimer(Some(hwnd), TIMER_ID) };
                // SAFETY: メッセージループを持つスレッドから呼ぶ限り常に安全。
                unsafe { PostQuitMessage(0) };
                LRESULT(0)
            }
            // SAFETY: DefWindowProcW はどんな (hwnd, msg, wparam, lparam) の
            // 組でも安全に呼べる Win32 の既定ハンドラ。
            _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        }
    }

    pub(super) fn run() -> anyhow::Result<()> {
        let class_name_wide = to_wide(WINDOW_CLASS_NAME);
        // SAFETY: プロセス起動直後、他スレッドがまだ存在しない時点での呼び出し。
        let hinstance = unsafe { windows::Win32::System::LibraryLoader::GetModuleHandleW(None) }
            .unwrap_or_default();

        let wc = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinstance.into(),
            lpszClassName: PCWSTR(class_name_wide.as_ptr()),
            ..Default::default()
        };
        // SAFETY: wc はスタック上の有効な WNDCLASSW。呼び出しはプロセス起動
        // 直後の1回のみ。
        let atom = unsafe { RegisterClassW(&raw const wc) };
        anyhow::ensure!(atom != 0, "RegisterClassW failed");

        // 非表示ウィンドウで十分（ポーリングにはメッセージループさえ回って
        // いれば見た目は不要）。WS_OVERLAPPEDWINDOW のみで WS_VISIBLE は付けない。
        let title_wide = to_wide("spike_egui_himc_reassociation_probe (hidden)");
        // SAFETY: class_name_wide/title_wide/hinstance はいずれもこのスコープで
        // 有効。
        let hwnd = unsafe {
            CreateWindowExW(
                windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE::default(),
                PCWSTR(class_name_wide.as_ptr()),
                PCWSTR(title_wide.as_ptr()),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(hinstance.into()),
                None,
            )
        }
        .map_err(|e| anyhow::anyhow!("CreateWindowExW failed: {e}"))?;

        // SAFETY: hwnd は直前に作成した有効なウィンドウ。
        let timer_id = unsafe { SetTimer(Some(hwnd), TIMER_ID, POLL_INTERVAL_MS, None) };
        anyhow::ensure!(timer_id != 0, "SetTimer failed");

        println!("=== spike_egui_himc_reassociation_probe 起動 ===");
        println!(
            "{POLL_INTERVAL_MS}ms 間隔で GetForegroundWindow() + ImmGetContext(そのhwnd) をポーリングします。"
        );
        println!(
            "awase-settings.exe（設定画面/不具合報告画面）にフォーカスし、テキスト欄への連続入力と、"
        );
        println!(
            "テキスト欄⇔他ウィジェット（ComboBox/チェックボックス）間のフォーカス移動を繰り返してください。"
        );
        println!(
            "[HIMC] 行が hwnd 不変のまま出るかどうかが ADR-125 の本題です。詳細はこのファイル冒頭のコメント参照。"
        );
        println!("終了するにはこのコンソールを閉じるか Ctrl+C。");
        println!();

        // 起動直後の初期値も1回出しておく（変化検出だけだと「起動時点の状態」が
        // 分からないため）。
        poll_and_log_on_change();

        let mut msg = MSG::default();
        // SAFETY: msg はスタック上の有効な MSG バッファ。hwnd=None でこの
        // スレッドの全ウィンドウ分のメッセージを対象にする。GetMessageW は
        // WM_QUIT で false を返しループを抜ける。
        while unsafe { GetMessageW(&raw mut msg, None, 0, 0) }.as_bool() {
            let _ = unsafe { TranslateMessage(&raw const msg) };
            unsafe { DispatchMessageW(&raw const msg) };
        }

        Ok(())
    }
}

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    probe::run()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("このスパイクは Windows 専用です（cfg(windows) ガード）。");
}
