//! [ADR-125](../../../docs/adr/125-egui-winit-dynamic-ime-association-focus-model-gap.md)
//! の「次のアクション」1（実運用コードパスでの確認）を実機で行うための
//! 使い捨てスパイク。
//!
//! ## 背景・前回のスパイクとの違い
//!
//! 前回の `spike_egui_himc_reassociation_probe.rs` は `ImmGetContext(hwnd)`
//! で HIMC を直接読み、`awase-settings.exe` にフォーカス中は終始 0（NULL）で
//! 変化しないことを確認した。しかし **これは awase 本体が実際に使っている
//! クロスプロセス IME 制御の方式ではない**——awase
//! （`crates/awase-windows/src/imm.rs::get_ime_wnd`/`send_ime_control`）は
//! `ImmGetContext` による HIMC の直接読み取りではなく、
//! `ImmGetDefaultIMEWnd(hwnd)` で対象スレッドの IME ウィンドウを取得し、
//! そこへ `WM_IME_CONTROL`（`IMC_GETOPENSTATUS`/`IMC_GETCONVERSIONMODE`/
//! `IMC_SETOPENSTATUS`）を `SendMessageTimeoutW` で送るという、
//! **別のメカニズム**を使っている。
//!
//! つまり前回のスパイクの「HIMC が終始 0」という結果は、**awase 本体の
//! 実際の制御経路が機能しているかどうかについては何も証明していない**
//! （ADR-125 の「解釈(a)/(b)」参照）。本スパイクは awase 本体と全く同じ
//! Win32 呼び出し列（`get_ime_wnd`/`send_ime_control` と同一ロジックを、
//! 本体クレートに依存させず直接再実装したもの——`imm.rs` の関数は
//! `pub(crate)` のため example から直接呼べない）を使い、この分岐を
//! 直接確定させる。
//!
//! ## 検証したいこと
//!
//! 1. `ImmGetDefaultIMEWnd(hwnd)` は `awase-settings.exe` に対して有効な
//!    IME ウィンドウハンドルを返すか（`None` なら、この時点で awase の
//!    クロスプロセス制御は最初の一歩から成立しない）。
//! 2. `SendMessageTimeoutW(ime_wnd, WM_IME_CONTROL, IMC_GETOPENSTATUS, ...)`
//!    は成功する（タイムアウトしない）か、また何 ms かかるか
//!    （awase 本体は 20/50/150ms のタイムアウトを使う——本スパイクでは
//!    150ms を使い、それでも頻繁にタイムアウトするなら「重い」症状の
//!    直接の説明になりうる）。
//! 3. 読めた `IMC_GETOPENSTATUS` の値が、実際に見えている IME の ON/OFF
//!    と一致するか（ユーザーが手動で IME を ON/OFF 切り替えながら確認）。
//!
//! ## 実行方法（Windows 実機のみ）
//!
//! ```powershell
//! cargo run -p awase-windows --example spike_egui_ime_control_probe --release
//! ```
//!
//! 100ms間隔でポーリングする。**フォアグラウンドウィンドウが
//! `awase-settings.exe` の間は毎tickログする**（前回スパイクと違い、
//! この区間は「変化なし」自体が重要な情報なので間引かない）。それ以外の
//! ウィンドウにフォーカスがある間は、フォーカス変更時のみ1行ログする。
//!
//! ### 手順
//!
//! 1. このプローブを起動したまま不具合報告画面
//!    （`awase-settings.exe --bug-report`）にフォーカスする。
//! 2. 説明欄をクリックし、実際に IME で日本語入力を試す（ひらがなが
//!    出るか、変換されるかを目視で確認しながら）。
//! 3. 可能なら、言語バーまたは物理キーで IME を明示的に OFF → ON と
//!    切り替える動作も試す。
//! 4. ログの `open=` 列（`Some(true)`/`Some(false)`/`None`）が、目視した
//!    実際の IME 状態と一致するか、`ime_wnd=` が毎回同じ値か、
//!    `elapsed_ms=` が異常に大きい（タイムアウト近く）行が無いかを見る。
//!
//! ### 見るべきポイント
//!
//! - `ime_wnd=NULL` が続く → 解釈(a)寄り（この時点で awase の制御経路が
//!   そもそも成立していない）。
//! - `ime_wnd` は非NULLだが `open=None`（`SendMessageTimeoutW` 失敗/
//!   タイムアウト）が頻発 → 解釈(a)寄り（ウィンドウは見つかるが応答しない）。
//! - `open=Some(..)` が返り、かつ実際の IME 状態と一致する → 解釈(b)寄り
//!   （awase 本体の制御経路自体は機能しており、前回スパイクの
//!   `ImmGetContext` 直読みが単に awase の実装と異なる方式だったために
//!   無意味な結果を返していただけ）。
//!
//! CapsLock を押すと `[CAPS]` 行が出る（プローブ健全性の裏取り、本題とは無関係）。
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
    use windows::Win32::UI::Input::Ime::ImmGetDefaultIMEWnd;
    use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyState;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClassNameW, GetForegroundWindow,
        GetMessageW, GetWindowThreadProcessId, KillTimer, MSG, PostQuitMessage, RegisterClassW,
        SMTO_ABORTIFHUNG, SendMessageTimeoutW, SetTimer, TranslateMessage, WM_DESTROY, WM_TIMER,
        WNDCLASSW, WS_OVERLAPPEDWINDOW,
    };
    use windows::core::{PCWSTR, PWSTR};

    /// `crates/awase-windows/src/imm.rs` と同じ値。`pub(crate)` のため
    /// example から直接参照できず、依存も増やしたくないのでリテラルで持つ
    /// （`spike_kana_lock_probe.rs` の VK_KANA と同じ方針）。
    const WM_IME_CONTROL: u32 = 0x0283;
    const IMC_GETOPENSTATUS: usize = 0x0005;

    /// CapsLock。プローブ健全性の裏取り用。
    const VK_CAPITAL: i32 = 0x14;

    const TARGET_PROCESS: &str = "awase-settings.exe";

    const WINDOW_CLASS_NAME: &str = "spike_egui_ime_control_probe_window";
    const TIMER_ID: usize = 1;
    const POLL_INTERVAL_MS: u32 = 100;
    /// awase 本体が `IMC_GETOPENSTATUS` に実際に使っている値
    /// （`ime.rs:545`）に合わせる。
    const SEND_IME_CONTROL_TIMEOUT_MS: u32 = 50;

    fn to_wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn now() -> String {
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
        // SAFETY: hwnd はこのファイル内で GetForegroundWindow が返した値。
        //         buf はスタック上の有効なバッファ。
        let len = unsafe { GetClassNameW(hwnd, &mut buf) };
        if len <= 0 {
            return "?".to_string();
        }
        String::from_utf16_lossy(&buf[..len.cast_unsigned() as usize])
    }

    /// `focus/classify.rs::get_process_name` と同じロジックの独立実装
    /// （`spike_egui_himc_reassociation_probe.rs` と同じ関数を再掲——依存を
    /// 増やさない方針のためコピーする）。
    fn process_name_of(hwnd: HWND) -> String {
        let mut pid: u32 = 0;
        // SAFETY: hwnd は GetForegroundWindow が返した値。pid はスタック上の
        //         有効な u32 変数へのポインタ。
        unsafe { GetWindowThreadProcessId(hwnd, Some(&raw mut pid)) };
        if pid == 0 {
            return "?".to_string();
        }
        // SAFETY: pid は直前に取得した有効なプロセス ID。
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

    /// `crates/awase-windows/src/imm.rs::get_ime_wnd` と同じロジック。
    fn get_ime_wnd(hwnd: HWND) -> Option<HWND> {
        // SAFETY: hwnd はこのファイル内で GetForegroundWindow が返した値。
        //         ImmGetDefaultIMEWnd は hwnd に対応する IME ウィンドウを
        //         返すだけで副作用なし、クロスプロセスでも安全に呼べる
        //         （awase 本体が同じ呼び出しをクロスプロセスで使っている）。
        let wnd = unsafe { ImmGetDefaultIMEWnd(hwnd) };
        (!wnd.0.is_null()).then_some(wnd)
    }

    /// `crates/awase-windows/src/imm.rs::send_ime_control` と同じロジック
    /// （SendHealth 計装等の本体固有の副作用は除く、Win32 呼び出し部分のみ）。
    /// 戻り値: `(結果, 呼び出しにかかった時間ms)`。
    fn send_ime_control(ime_wnd: HWND, cmd: usize, timeout_ms: u32) -> (Option<usize>, u128) {
        let mut result = 0usize;
        let start = std::time::Instant::now();
        // SAFETY: ime_wnd は get_ime_wnd が返した有効な IME ウィンドウ
        //         ハンドル。SMTO_ABORTIFHUNG によりハングしたスレッドで
        //         無期限にブロックしない。result はスタック上の有効な
        //         usize でポインタ渡しが安全。
        let ok = unsafe {
            SendMessageTimeoutW(
                ime_wnd,
                WM_IME_CONTROL,
                WPARAM(cmd),
                LPARAM(0),
                SMTO_ABORTIFHUNG,
                timeout_ms,
                Some(&raw mut result),
            )
        };
        let elapsed_ms = start.elapsed().as_millis();
        (if ok.0 != 0 { Some(result) } else { None }, elapsed_ms)
    }

    thread_local! {
        static PREV_HWND: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
        static PREV_CAPS_BIT: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
    }

    fn poll_and_log() {
        // SAFETY: GetForegroundWindow は引数なしで常に安全。
        let hwnd = unsafe { GetForegroundWindow() };
        let hwnd_raw = hwnd.0 as isize;
        let prev_hwnd = PREV_HWND.get();
        let focus_changed = hwnd_raw != prev_hwnd;
        PREV_HWND.set(hwnd_raw);

        let process = if hwnd.0.is_null() {
            "?".to_string()
        } else {
            process_name_of(hwnd)
        };
        let is_target = process.eq_ignore_ascii_case(TARGET_PROCESS);

        if focus_changed {
            println!(
                "{} [FOCUS] hwnd 0x{:X} -> 0x{:X} class={} process={}",
                now(),
                prev_hwnd,
                hwnd_raw,
                if hwnd.0.is_null() {
                    "?".to_string()
                } else {
                    class_name_of(hwnd)
                },
                process,
            );
        }

        // ターゲット（awase-settings.exe）にフォーカス中は毎tickログする
        // （前回スパイクと違い「変化なし」自体が重要な情報のため間引かない）。
        if is_target {
            match get_ime_wnd(hwnd) {
                None => {
                    println!(
                        "{} [IME-CTRL] ime_wnd=NULL (ImmGetDefaultIMEWnd failed)",
                        now()
                    );
                }
                Some(ime_wnd) => {
                    let (open, elapsed_ms) =
                        send_ime_control(ime_wnd, IMC_GETOPENSTATUS, SEND_IME_CONTROL_TIMEOUT_MS);
                    println!(
                        "{} [IME-CTRL] ime_wnd=0x{:X} open={:?} elapsed_ms={}",
                        now(),
                        ime_wnd.0 as isize,
                        open.map(|v| v != 0),
                        elapsed_ms,
                    );
                }
            }
        }

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
                poll_and_log();
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

        let title_wide = to_wide("spike_egui_ime_control_probe (hidden)");
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

        println!("=== spike_egui_ime_control_probe 起動 ===");
        println!(
            "{POLL_INTERVAL_MS}ms 間隔でポーリングします。{TARGET_PROCESS} にフォーカス中は毎tick、"
        );
        println!(
            "ImmGetDefaultIMEWnd(hwnd) + WM_IME_CONTROL(IMC_GETOPENSTATUS) の結果をログします。"
        );
        println!(
            "詳細・見るべきポイントはこのファイル冒頭のコメント参照。終了するにはこのコンソールを閉じるか Ctrl+C。"
        );
        println!();

        poll_and_log();

        let mut msg = MSG::default();
        // SAFETY: msg はスタック上の有効な MSG バッファ。hwnd=None でこの
        // スレッドの全ウィンドウ分のメッセージを対象にする。
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
