//! issue #137（Teams/WebView2 で awase の送信 VK が JIS かな配列として誤解釈
//! される）の対策設計における Phase 0 検証タスク V1 用の使い捨てスパイク。
//!
//! ## 検証したいこと
//!
//! `GetKeyState(VK_KANA) & 1`（`VK_KANA` = `0x15`）が、IME の「ローマ字入力
//! ⇔かな入力」の実際の反転に追従するかどうかは**未確認**。CapsLock/NumLock/
//! ScrollLock は Windows が公式にトグルキーとして管理しており
//! `crates/awase-windows/src/ime.rs:1753` の `is_caps_lock_on()` が同じ
//! `GetKeyState(vk) & 1` パターンで実際に機能しているが、`VK_KANA` が同じ
//! トグル管理に乗っているかは日本語キーボードドライバの実装依存で、この
//! リポジトリ内に確認済みの前例がない。
//!
//! 比較対象として CapsLock も同時にログするのは、「そもそもこのプローブの
//! 読み取り機構自体が動いているか」をユーザーが目視で切り分けられるように
//! するため（CapsLock が正しく追従するのに VK_KANA だけ動かない、なら
//! プローブの不具合ではなく VK_KANA 固有の結果と判断できる）。
//!
//! ## 実行方法（Windows 実機のみ）
//!
//! ```powershell
//! cargo run -p awase-windows --example spike_kana_lock_probe --release
//! ```
//!
//! 起動すると非表示ウィンドウを1つ作り、150ms間隔で `GetKeyState` をポーリング
//! する。**値が変化した瞬間だけ**標準出力に1行ログを出す（変化なしはログしない
//! ので、長時間観察してもスクロールで埋もれない）。
//!
//! ### 手順
//!
//! 1. このプローブを起動したまま Teams（または再現環境）にフォーカスを移す。
//! 2. 言語バー、または `Alt + カタカナひらがなローマ字` キーで
//!    「ローマ字入力」⇔「かな入力」を手動で切り替える。
//! 3. 切り替えるたびに `KANA` の行が出るかを見る。
//!    - 出る（`bit` が `off`⇔`on` を追従する） → 候補C（V1）は成立。
//!      Phase 1 以降（`observer/kana_lock.rs` の実装）に進める。
//!    - 出ない（切り替えても `KANA` 行が一切出ない、または `bit` が
//!      常に同じ値のまま） → 候補C（V1）は不成立。設計の V2（フックスレッド
//!      からの読み取り）→ 失敗なら候補A（レジストリ読み）に倒す。
//!
//! CapsLock を押して `CAPS` の行が正しく出るかも合わせて確認しておくと、
//! 「プローブ自体は機能している」という前提の裏取りになる。
//!
//! Ctrl+C またはウィンドウを閉じて終了する。

#![allow(unsafe_code)]

#[cfg(windows)]
mod probe {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyState;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, KillTimer, PostQuitMessage,
        RegisterClassW, SetTimer, TranslateMessage, MSG, WM_DESTROY, WM_TIMER, WNDCLASSW,
        WS_OVERLAPPEDWINDOW,
    };

    /// `crates/awase-windows/src/vk.rs` の `VK_KANA` と同じ値。この使い捨て
    /// スパイクは本体クレートに依存させたくないのでリテラルで持つ。
    const VK_KANA: i32 = 0x15;
    /// CapsLock。`ime.rs:1753` の `is_caps_lock_on()` と同じ値、比較対象。
    const VK_CAPITAL: i32 = 0x14;

    const WINDOW_CLASS_NAME: &str = "spike_kana_lock_probe_window";
    const TIMER_ID: usize = 1;
    const POLL_INTERVAL_MS: u32 = 150;

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

    fn foreground_class_name() -> String {
        use windows::Win32::UI::WindowsAndMessaging::{GetClassNameW, GetForegroundWindow};
        // SAFETY: GetForegroundWindow は引数なしで常に安全。戻り値が null
        // でも以降のガードで弾く。
        let hwnd = unsafe { GetForegroundWindow() };
        if hwnd.0.is_null() {
            return "?".to_string();
        }
        let mut buf = [0u16; 256];
        // SAFETY: buf はスタック上の有効なバッファで長さも渡している。
        let len = unsafe { GetClassNameW(hwnd, &mut buf) };
        if len <= 0 {
            return "?".to_string();
        }
        String::from_utf16_lossy(&buf[..len.cast_unsigned() as usize])
    }

    // 直前値との比較用。`static mut` を経由した共有参照は edition 2024 で
    // 既定拒否になる `static_mut_refs` の典型パターンなので、`Cell` を
    // thread_local に包んで参照を取らずに get/set する形にする（このスパイクは
    // シングルスレッド・シングルウィンドウ前提だが、健全性のコストを払う理由は
    // 無い）。
    thread_local! {
        static PREV_KANA_BIT: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
        static PREV_CAPS_BIT: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
    }

    fn poll_and_log_on_change() {
        // SAFETY: GetKeyState はどのスレッドからも安全に呼べる（引数は仮想
        // キーコードのみ、副作用なし）。メッセージループを持つこのウィンドウの
        // スレッドから呼んでいるので、production 側（Runtime のメッセージ
        // ループスレッド）と同じ条件を再現している。
        let kana_raw = unsafe { GetKeyState(VK_KANA) };
        let kana_bit = (kana_raw & 1) != 0;
        // SAFETY: 同上。
        let caps_raw = unsafe { GetKeyState(VK_CAPITAL) };
        let caps_bit = (caps_raw & 1) != 0;

        if PREV_KANA_BIT.get() != Some(kana_bit) {
            let prev = PREV_KANA_BIT
                .get()
                .map_or("?", |b| if b { "on" } else { "off" });
            println!(
                "{} KANA raw=0x{:04X} bit={} (prev={}) fg_class={}",
                now(),
                kana_raw.cast_unsigned(),
                if kana_bit { "on" } else { "off" },
                prev,
                foreground_class_name()
            );
            PREV_KANA_BIT.set(Some(kana_bit));
        }
        if PREV_CAPS_BIT.get() != Some(caps_bit) {
            let prev = PREV_CAPS_BIT
                .get()
                .map_or("?", |b| if b { "on" } else { "off" });
            println!(
                "{} CAPS raw=0x{:04X} bit={} (prev={}) [比較対象・プローブ健全性の裏取り]",
                now(),
                caps_raw.cast_unsigned(),
                if caps_bit { "on" } else { "off" },
                prev
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

        // 非表示ウィンドウで十分（GetKeyState はメッセージループさえ回って
        // いれば見た目は不要）。WS_OVERLAPPEDWINDOW のみで WS_VISIBLE は付けない。
        let title_wide = to_wide("spike_kana_lock_probe (hidden)");
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

        println!("=== spike_kana_lock_probe 起動 ===");
        println!("{POLL_INTERVAL_MS}ms 間隔で GetKeyState(VK_KANA)/GetKeyState(VK_CAPITAL) をポーリングします。");
        println!("値が変化した瞬間だけログします。Teams 等にフォーカスして言語バーからかな入力⇔ローマ字入力を切り替えてください。");
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
