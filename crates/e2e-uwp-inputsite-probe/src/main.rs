//! issue #165（`Hook watchdog: no activity ... フックにイベントが届いていない疑い`）を、
//! 実アプリ（Chrome/Zoom/Windows検索ボックス等）無しで CI(windows-latest) 上でも
//! 再現できるかを切り分けるための使い捨てプローブ。
//!
//! ## 背景
//!
//! 不具合報告 `01M3JTVDPMW35MF81DGRKW10MQ`（`docs/bug-reports-triage.md` 参照、
//! GitHub issue #165 へコメント追記済み）で、他アプリ間のフォーカス往復の直後に
//! `Windows.UI.Input.InputSite.WindowClass`（UWP系入力面、explorer.exe内）へ
//! フォーカスが移り、その直後の約6.4秒間 awase の `WH_KEYBOARD_LL` フックに
//! キー入力が一切届かない（OS全体では直近に入力があったにもかかわらず）現象が
//! 観測された。「能動的」→「うどうてき」（先頭モーラ「の」欠落）という
//! `WrongCharacterOutput` の実害と初めて結びついた。
//!
//! ## 実装方針（1回転目からの方針転換）
//!
//! 当初は `windows` crate（0.62系）に含まれる Win32 API 経由で
//! `Windows.UI.Xaml.Hosting.DesktopWindowXamlSource` を素の COM 呼び出しで
//! ホストする案を試みたが、`windows` crate は Win32 名前空間専用で
//! `Windows.UI.Xaml.*`（レガシーUWP XAML）の生成バインディングを一切含んで
//! おらず（`cargo check` が `Win32_System_WinRT_Xaml` 等の存在しない feature で
//! 即座に失敗した）、コンパイル不能だった。手動でCOM ABI（GUID/vtable）を
//! 宣言する代替案はリスクが高い（GUIDを1つ間違えるとUB）ため採用せず、
//! 代わりに Microsoft 公式・windows-rs プロジェクト自身が提供する
//! **`windows-reactor`**（宣言的 WinUI3 ライブラリ、
//! <https://github.com/microsoft/windows-rs/tree/master/crates/libs/reactor>）
//! を使う。`TextBox` コンポーネントが実際に `Windows.UI.Input.InputSite.WindowClass`
//! を持つ入力面を作る（レガシーUWP XAMLとWinUI3は共通の `InputSite`
//! コンポーネントでテキスト入力をホストしているため、クラス名は一致する）。
//! `windows-reactor-setup`（build.rs専用）が Windows App SDK ランタイムを
//! 自己完結配置してくれるため、CIランナーへの事前インストールが要らない。
//!
//! 公開crate（crates.io の `windows-reactor 0.100`）は GitHub `master` より
//! API面が少なく、`ComponentContext::set_timeout`/`ComponentTimer`/
//! `WindowRef::request_activate` は未公開（`master` にはあるが降りてきていない）。
//! そのため本プローブは:
//!
//! - 繰り返しタイマーを `ComponentContext::spawn_background`（バックグラウンド
//!   スレッドで `sleep` してから `Tick` メッセージを返す、`update()` 側で
//!   毎回再発行）で自前再現する（`arm_tick` 参照）。
//! - ウィンドウ前景化を `context.window_title()` で設定したタイトルから
//!   `FindWindowW` で実HWNDを引き、素の Win32
//!   （`SetForegroundWindow`/`AttachThreadInput`）で行う
//!   （`force_foreground_by_title` 参照、`e2e_windows.rs::force_foreground` と同じ手法）。
//!
//! ## このプローブがやること
//!
//! `windows-reactor` で2つのウィンドウを開く:
//!
//! - `Distractor`: ただの `TextBlock` のみを持つ、無関係な別アプリ役のウィンドウ。
//! - `InputSurface`: `TextBox` を1つ持つ、実際に `Windows.UI.Input.InputSite.WindowClass`
//!   を持つウィンドウ。
//!
//! それぞれが自分の繰り返しタイマーで周期的に自分自身を前景化する
//! （2つのタイマーの位相を半周期ずらすことで、明示的な相互通信なしに
//! フォーカスが往復する）。`InputSurface` は前景化の直後に `TextBox` を
//! `request_focus()` してから `SendInput` でマーク付きキーを注入する
//! （不具合報告で見られた「無関係なアプリ間往復 → UWP系入力面へフォーカス
//! → 直後に入力」という順序を模す）。
//!
//! `Distractor` は自分の番が来るたびに、`docs/known-bugs/BUG-053.md` で実機
//! 確認済みの引き金（物理 Win キー押下→検索UI(`searchhost.exe`)オープン→
//! 直後に打鍵）も再現する（`trigger_search_ui_and_type`）。BUG-053 は
//! シェル側の別 `WH_KEYBOARD_LL` フックが `CallNextHookEx` を呼ばず KeyUp を
//! 消費し awase 側フックに届かないことを実機ログで確認済みで、issue #165 の
//! hook_starved と同族のメカニズムである可能性が高い。単純な自前ウィンドウの
//! フォーカス往復だけでは1回のCI実行(80回往復)で hook_starved を再現できな
//! かったため追加した。
//!
//! 成功/失敗の判定はこのプローブ自身では行わない。別プロセスとして起動中の
//! awase.exe（デバッグビルド、`AWASE_TEST_INJECTION=1` 環境変数、`RUST_LOG=debug`）
//! のログを`Hook watchdog: no activity for .*フックにイベントが届いていない疑い(issue #165)`
//! で grep することで行う（`crates/awase-windows/src/runtime/message_handlers.rs:574-598`
//! の診断専用ログ、`stale_ms>5000` かつ `os_idle_ms<5000` のときだけ出る）。
//!
//! **注意**: awase.exe は既定（`debug_console=false`）では stderr に何も出さず、
//! 実行ファイルと同じディレクトリの固定パス `awase.log` へ書く
//! （`crates/awase-windows/src/app/bootstrap.rs::init_logging`/`log_path`、
//! `tools/e2e/config_verify/run.py::start_awase`と同じ流儀）。`-RedirectStandardError`
//! でstderrを捕まえようとしても常に空になる（1回転目でこれを誤り、実機/CI
//! いずれの初回試行もこのため判定不能だった）。
//!
//! ## 実行方法（Windows実機、または CI の windows-latest ランナー）
//!
//! ```powershell
//! # 1. awase をデバッグビルドし、テスト注入を物理キー扱いする設定で起動
//! cargo build -p awase-windows --bin awase
//! $env:AWASE_TEST_INJECTION = "1"
//! Start-Process target\debug\awase.exe -WorkingDirectory target\debug
//! Start-Sleep -Seconds 2
//!
//! # 2. このプローブを実行（GJI 等の実 IME は不要 — hook_starved はキー配送層の
//! #    現象で IME 変換の正しさとは無関係）
//! cargo run -p e2e-uwp-inputsite-probe -- --iterations=80
//!
//! # 3. awase 側ログを確認（実行ファイルと同じディレクトリの固定パス）
//! Select-String -Path target\debug\awase.log -Pattern 'フックにイベントが届いていない疑い'
//! ```
//!
//! ## フラグ
//!
//! `--iterations=N`（既定80、フォーカス往復回数） /
//! `--dwell-ms=N`（既定120、フォーカス保持時間の目安。実報告では複数の短い
//! フォーカス往復ののちに問題の窓が出た）

#![allow(unsafe_code)]

#[cfg(windows)]
mod probe {
    use std::time::Duration;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
        KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, VIRTUAL_KEY,
    };
    use windows_reactor::*;

    /// テストドライバの目印（`hook.rs::TEST_INJECTION_MARKER` と同一、二重定義しない）。
    const MARKER: usize = awase_windows::hook::TEST_INJECTION_MARKER;

    /// 「N/O/U」等、NICOLA を経由しない単純なキー列
    /// （このプローブは文字化けの中身ではなくフック配送そのものを見るため、
    /// キー種別は問わない）。
    const PROBE_KEYS: &[(u16, u16)] = &[
        (0x4E, 0x31), // N
        (0x4F, 0x18), // O
        (0x55, 0x16), // U
    ];

    /// `docs/known-bugs/BUG-053.md` で実機確認済みの引き金: 物理 Win キー押下で
    /// 検索UI(`searchhost.exe`)が開く際、シェル側の別の `WH_KEYBOARD_LL` フックが
    /// `CallNextHookEx` を呼ばず KeyUp を消費し、awase 側のフックにイベントが
    /// 渡らないことがある。issue #165 の hook_starved(フックにイベントが届いて
    /// いない疑い)と同族のメカニズムである可能性が高いため、同じ引き金を
    /// このプローブでも再現する。
    const VK_LWIN: u16 = 0x5B;
    const VK_ESCAPE: u16 = 0x1B;

    fn arg_value(key: &str) -> Option<String> {
        std::env::args().find_map(|a| a.strip_prefix(key).map(str::to_string))
    }

    fn now() -> String {
        let dur = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let secs_of_day = dur.as_secs() % 86400;
        let (h, m, s) = (
            secs_of_day / 3600,
            (secs_of_day % 3600) / 60,
            secs_of_day % 60,
        );
        format!("{h:02}:{m:02}:{s:02}.{:03}", dur.subsec_millis())
    }

    fn log(line: &str) {
        println!("{} {line}", now());
    }

    /// キーを1つ、テストドライバの目印付きで注入する
    /// （`AWASE_TEST_INJECTION=1` の debug ビルド awase が物理キー扱いする）。
    fn send_marked_key(vk: u16, scan: u16) {
        send_marked_key_ex(vk, scan, false);
    }

    /// `send_marked_key` の拡張版。`extended=true` で `KEYEVENTF_EXTENDEDKEY`
    /// を立てる（Win キー等、拡張キーとして送る必要があるキー用）。
    fn send_marked_key_ex(vk: u16, scan: u16, extended: bool) {
        let ext_flag = if extended {
            KEYEVENTF_EXTENDEDKEY
        } else {
            KEYBD_EVENT_FLAGS::default()
        };
        let inputs = [
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VIRTUAL_KEY(vk),
                        wScan: scan,
                        dwFlags: ext_flag,
                        time: 0,
                        dwExtraInfo: MARKER,
                    },
                },
            },
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VIRTUAL_KEY(vk),
                        wScan: scan,
                        dwFlags: ext_flag | KEYEVENTF_KEYUP,
                        time: 0,
                        dwExtraInfo: MARKER,
                    },
                },
            },
        ];
        let size = i32::try_from(size_of::<INPUT>()).expect("INPUT size fits i32");
        // SAFETY: inputs はスタック上の有効な配列、size は正しい要素サイズ。
        let sent = unsafe { SendInput(&inputs, size) };
        if sent == 0 {
            log(&format!("SendInput failed for vk=0x{vk:02X}"));
        }
    }

    fn send_probe_sequence() {
        for &(vk, scan) in PROBE_KEYS {
            send_marked_key(vk, scan);
            std::thread::sleep(Duration::from_millis(15));
        }
    }

    /// BUG-053 の引き金（Win キー押下→検索UIオープン→直後に打鍵）を再現する。
    fn trigger_search_ui_and_type() {
        send_marked_key_ex(VK_LWIN, 0x5B, true);
        std::thread::sleep(Duration::from_millis(250));
        send_probe_sequence();
        send_marked_key(VK_ESCAPE, 0x01);
        std::thread::sleep(Duration::from_millis(150));
    }

    /// issue #165 の hook_starved を確実に発火させるための強制ブロック実験。
    ///
    /// フォーカス往復だけでは（BUG-053 の Win キー引き金を混ぜても）実際に
    /// awase の `WH_KEYBOARD_LL` フックへイベントが届かなくなる瞬間を1回の
    /// CI 実行で偶然引き当てられなかった。そこで、このプローブ自身が
    /// **もう1つの** `WH_KEYBOARD_LL` フックを（awase より後に）インストールし、
    /// 一定時間だけ `CallNextHookEx` を意図的に呼ばずに自分のマーク付きキーを
    /// 握りつぶす。フックは後から入れたものほど先に呼ばれる（LIFO）ため、
    /// この間 awase 側フックには一切イベントが渡らなくなる——「OS は
    /// `SendInput` を受理し続けている（`GetLastInputInfo` は更新され続ける）のに
    /// awase のフックだけ何も受け取らない」という issue #165 の観測条件そのものを
    /// 人為的に作り出す。真の自然発生条件（何が実際に awase のフックを詰まらせる
    /// のか）を特定するものではないが、「その状態になったら watchdog が正しく
    /// 検知し、実際に文字入力が壊れるか」という**下流の因果関係**を検証できる。
    ///
    /// **`--force-starvation-swallow-all`（opus round3 M1追記）**: 既定では
    /// このプローブ自身のマーク付きキーだけを握りつぶす（`blocking_hook::proc`
    /// の`info.dwExtraInfo == MARKER`判定）。issue #165 自己修復（`state::
    /// hook_watchdog`）のカナリアは別のマーカーを使うため、既定モードでは
    /// カナリアが常に素通りしてawase側に届いてしまい、**自己修復が一度も
    /// 発火しないまま「正常」に見える**。このフラグを付けると目印の有無を
    /// 問わず全イベントを握りつぶす「最悪ケース」になり、自己修復が実際に
    /// フックを再インストールして復帰することを検証できる（`state::
    /// hook_watchdog`モジュールdoc「既知の限界」も参照——注入イベントだけ
    /// 通す実装の原因フックには、この自己修復は効かない）。
    mod blocking_hook {
        use super::{log, send_marked_key, PROBE_KEYS};
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::time::{Duration, Instant};
        use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
        use windows::Win32::UI::WindowsAndMessaging::{
            CallNextHookEx, SetWindowsHookExW, UnhookWindowsHookEx, HHOOK, KBDLLHOOKSTRUCT,
            WH_KEYBOARD_LL,
        };

        static BLOCKING: AtomicBool = AtomicBool::new(false);
        /// opus round3 M1: `true` の間は目印の有無を問わず**全キーイベント**を
        /// 握りつぶす。issue #165 自己修復のカナリア（`awase_windows::hook::
        /// send_hook_watchdog_canary`）は自前の別マーカーを使うため、従来の
        /// 「`super::MARKER` のときだけ握りつぶす」実装だとカナリアは素通りして
        /// awase 側に届いてしまい、自己修復（フック再インストール）が一度も
        /// 発火しないまま「再現した」ことになってしまう。実際の issue #165 の
        /// 原因フックが注入イベントを区別せず握りつぶす実装かどうかは不明の
        /// ままだが（`docs/known-bugs/`にも確証は無い）、この「最悪ケース」
        /// （何も通さない）で自己修復が機能することを確認するのが目的。
        static SWALLOW_ALL: AtomicBool = AtomicBool::new(false);

        unsafe extern "system" fn proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
            if code >= 0 && BLOCKING.load(Ordering::SeqCst) {
                if SWALLOW_ALL.load(Ordering::SeqCst) {
                    // 意図的に CallNextHookEx を呼ばない = 目印の有無を問わず
                    // 後段(awase含む)へこのイベントは一切届かない。
                    return LRESULT(1);
                }
                // SAFETY: Win32 契約により code>=0 のとき lparam は有効な
                // KBDLLHOOKSTRUCT を指す。
                let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
                if info.dwExtraInfo == super::MARKER {
                    // 意図的に CallNextHookEx を呼ばない = 後段(awase含む)へ
                    // このイベントは一切届かない。
                    return LRESULT(1);
                }
            }
            // SAFETY: 標準的なフックチェーン継続呼び出し。
            unsafe { CallNextHookEx(None, code, wparam, lparam) }
        }

        pub(super) struct Guard(HHOOK);

        impl Guard {
            pub(super) fn install(swallow_all: bool) -> anyhow::Result<Self> {
                SWALLOW_ALL.store(swallow_all, Ordering::SeqCst);
                // SAFETY: proc は正しい HOOKPROC シグネチャ、hMod は同一プロセス内
                // フックのため None で良い（Win32 の標準的な用法）。
                let hhook = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(proc), None, 0) }
                    .map_err(|e| anyhow::anyhow!("SetWindowsHookExW(blocking) failed: {e}"))?;
                Ok(Self(hhook))
            }
        }

        impl Drop for Guard {
            fn drop(&mut self) {
                // SAFETY: self.0 は install() が返した有効なフックハンドル。
                let _ = unsafe { UnhookWindowsHookEx(self.0) };
            }
        }

        /// バックグラウンドスレッドで、少し待ってから `window_secs` 秒間
        /// ブロックを有効化し、その間もマーク付きキーを送り続ける
        /// （OS側の `GetLastInputInfo` を新鮮に保ちつつ、awase側には
        /// 一切届かせないため）。
        pub(super) fn spawn_forced_starvation_window(initial_delay: Duration, window_secs: u64) {
            std::thread::spawn(move || {
                std::thread::sleep(initial_delay);
                log(&format!(
                    "=== 強制hook_starved窓 開始 ({window_secs}秒、この間awase側フックには何も届かないはず) ==="
                ));
                BLOCKING.store(true, Ordering::SeqCst);
                let start = Instant::now();
                while start.elapsed() < Duration::from_secs(window_secs) {
                    send_marked_key(PROBE_KEYS[0].0, PROBE_KEYS[0].1);
                    std::thread::sleep(Duration::from_millis(400));
                }
                BLOCKING.store(false, Ordering::SeqCst);
                log("=== 強制hook_starved窓 終了 ===");
            });
        }
    }

    // `windows-reactor` 0.100(公開版)には `ComponentContext::set_timeout`/
    // `ComponentTimer`/`WindowRef::request_activate` が無い（GitHub `master` の
    // 開発中APIで、公開crateにはまだ降りてきていない）。そのため:
    //
    // - 繰り返しタイマーは `spawn_background`（バックグラウンドスレッドで
    //   `sleep` してから `Tick` を返す）を毎回再発行して自前で再現する。
    // - ウィンドウ前景化は Reactor 側にAPIが無いため、`context.window_title()`
    //   で設定したタイトルを `FindWindowW` で引いて実 HWND を取得し、
    //   素の Win32 (`SetForegroundWindow`/`AttachThreadInput`) で行う
    //   （`e2e_windows.rs::force_foreground` と同じ手法）。

    const DISTRACTOR_TITLE: &str = "uwp_inputsite_probe: distractor";
    const INPUT_SURFACE_TITLE: &str = "uwp_inputsite_probe: InputSite surface";

    fn to_wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// タイトルで自プロセスのトップレベルウィンドウを探し、前景化する。
    /// Reactor がウィンドウを作成しネイティブHWNDのタイトルへ反映するまで
    /// 数フレームかかりうるため、見つからない場合は静かに諦める（次のtickで
    /// 再試行される）。
    fn force_foreground_by_title(title: &str) {
        use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
        use windows::Win32::UI::WindowsAndMessaging::{
            BringWindowToTop, FindWindowW, GetForegroundWindow, GetWindowThreadProcessId,
            SetForegroundWindow,
        };
        let wide = to_wide(title);
        // SAFETY: wide はこの呼び出しの間有効なNUL終端バッファ。
        let Ok(hwnd) = (unsafe { FindWindowW(None, windows::core::PCWSTR(wide.as_ptr())) }) else {
            return;
        };
        // SAFETY: 各APIはハンドルの有効性を自身で検証する（無効なら false/エラー）。
        unsafe {
            let fg = GetForegroundWindow();
            let fg_thread = GetWindowThreadProcessId(fg, None);
            let current_thread = GetCurrentThreadId();
            let attached = fg_thread != current_thread
                && AttachThreadInput(current_thread, fg_thread, true).as_bool();
            let _ = SetForegroundWindow(hwnd);
            let _ = BringWindowToTop(hwnd);
            if attached {
                let _ = AttachThreadInput(current_thread, fg_thread, false);
            }
        }
    }

    #[derive(Clone, Copy, PartialEq)]
    struct ProbeConfig {
        iterations: u32,
        dwell_ms: u64,
    }

    #[derive(Clone, Copy)]
    enum Tick {
        Fire,
    }

    fn arm_tick<C: Component<Message = Tick>>(
        context: &ComponentContext<C>,
        delay: Duration,
    ) -> ComponentTask {
        context.spawn_background(move |_cancel| {
            std::thread::sleep(delay);
            Tick::Fire
        })
    }

    /// 無関係な「別アプリ」役。`TextBox` を持たず、`Windows.UI.Input.InputSite.WindowClass`
    /// とは無関係な素の WinUI ウィンドウ。
    struct Distractor {
        config: ProbeConfig,
        remaining: u32,
        _task: ComponentTask,
    }

    impl Component for Distractor {
        type Message = Tick;
        type Input = ProbeConfig;

        fn create(input: &ProbeConfig, context: &ComponentContext<Self>) -> Self {
            log(&format!(
                "uwp_inputsite_probe 起動: iterations={} dwell_ms={}",
                input.iterations, input.dwell_ms
            ));
            println!("READY-FOR-AWASE");
            // 副ウィンドウ（実際のInputSite面）を開く。Distractor自身がルート
            // ウィンドウ（`App::run_component`の対象）を兼ねる。
            let _ = context.open_window(View::component::<InputSurface>(*input));
            let task = arm_tick(context, Duration::from_millis(300));
            Self {
                config: *input,
                remaining: input.iterations,
                _task: task,
            }
        }

        fn update(&mut self, _message: Tick, context: &ComponentContext<Self>) {
            if self.remaining == 0 {
                let _ = context.window().request_close();
                return;
            }
            self.remaining -= 1;
            force_foreground_by_title(DISTRACTOR_TITLE);
            trigger_search_ui_and_type();
            send_marked_key(PROBE_KEYS[0].0, PROBE_KEYS[0].1);
            self._task = arm_tick(context, Duration::from_millis(self.config.dwell_ms));
        }

        fn view(&self, _input: &ProbeConfig, context: &mut ViewContext<Self>) -> View {
            context.window_title(DISTRACTOR_TITLE);
            context.window_visuals(WindowVisuals::new().client_size(360.0, 160.0));
            Border::new()
                .padding(16.0)
                .content(StackPanel::new().children((
                    "無関係な別アプリ役（Zoom/pushbullet_client 相当）",
                    format!("残り往復: {}", self.remaining),
                )))
        }
    }

    /// `Windows.UI.Input.InputSite.WindowClass` を実際に持つ入力面
    /// （`TextBox` を1つホストする）。不具合報告のUWP系入力面に相当。
    struct InputSurface {
        config: ProbeConfig,
        remaining: u32,
        _task: ComponentTask,
        text_box: ElementRef<TextBox>,
    }

    impl Component for InputSurface {
        type Message = Tick;
        type Input = ProbeConfig;

        fn create(input: &ProbeConfig, context: &ComponentContext<Self>) -> Self {
            // Distractor と半周期ずらして起動し、フォーカスが往復するようにする。
            let initial_delay = Duration::from_millis(300 + input.dwell_ms / 2);
            let task = arm_tick(context, initial_delay);
            Self {
                config: *input,
                remaining: input.iterations,
                _task: task,
                text_box: ElementRef::default(),
            }
        }

        fn update(&mut self, _message: Tick, context: &ComponentContext<Self>) {
            if self.remaining == 0 {
                let _ = context.window().request_close();
                return;
            }
            self.remaining -= 1;
            force_foreground_by_title(INPUT_SURFACE_TITLE);
            // 実報告は「フォーカス移動の直後」に入力していたため、focus 要求の
            // 直後、間を置かずに打鍵する。
            let _ = self.text_box.request_focus();
            send_probe_sequence();
            if self.remaining.is_multiple_of(10) {
                log(&format!(
                    "InputSurface iteration remaining={}",
                    self.remaining
                ));
            }
            self._task = arm_tick(context, Duration::from_millis(self.config.dwell_ms));
        }

        fn view(&self, _input: &ProbeConfig, context: &mut ViewContext<Self>) -> View {
            context.window_title(INPUT_SURFACE_TITLE);
            context.window_visuals(WindowVisuals::new().client_size(360.0, 160.0));
            Border::new().padding(16.0).content(
                StackPanel::new().children((
                    TextBox::new()
                        .placeholder_text("issue #165 probe target")
                        .element_ref(&self.text_box),
                    format!("残り往復: {}", self.remaining),
                )),
            )
        }
    }

    pub(super) fn run() -> anyhow::Result<()> {
        let iterations: u32 = arg_value("--iterations=")
            .and_then(|s| s.parse().ok())
            .unwrap_or(80);
        let dwell_ms: u64 = arg_value("--dwell-ms=")
            .and_then(|s| s.parse().ok())
            .unwrap_or(120);
        // opus round3 M1: 目印の有無を問わず全キーを握りつぶす「最悪ケース」
        // モード（`blocking_hook::SWALLOW_ALL`）。issue #165 自己修復の
        // カナリアもこのモードでは確実に握りつぶされるため、自己修復が
        // 実際に機能するかをこのプローブで検証できる（従来のマーカー限定
        // ブロックでは、カナリアが常に素通りして自己修復が一度も発火しない）。
        let force_starvation_swallow_all =
            std::env::args().any(|a| a == "--force-starvation-swallow-all");
        // 既定の強制窓は、swallow-allモードでは自己修復の検知遅延
        // （3秒周期tickの量子化5〜8秒 + カナリア確認200ms = 最大約8.2秒、
        // opus round1 m5で訂正）に十分なマージンを足した25秒にする。
        // 従来モード(自己修復を前提としない)は既定7秒のまま変えない。
        let force_starvation_secs: u64 = arg_value("--force-starvation-secs=")
            .and_then(|s| s.parse().ok())
            .unwrap_or(if force_starvation_swallow_all { 25 } else { 7 });
        let config = ProbeConfig {
            iterations,
            dwell_ms,
        };

        // issue #165 を確実に発火させる強制実験（詳細は `blocking_hook` モジュール
        // doc参照）。`--force-starvation-secs=0` で無効化できる。
        let _blocking_hook_guard = if force_starvation_secs > 0 {
            let guard = blocking_hook::Guard::install(force_starvation_swallow_all)?;
            blocking_hook::spawn_forced_starvation_window(
                Duration::from_secs(3),
                force_starvation_secs,
            );
            Some(guard)
        } else {
            None
        };

        // `Distractor` がルートウィンドウ（`App::run_component`の対象）を兼ねる。
        // 副ウィンドウ（実際のInputSite面、`InputSurface`）は
        // `Distractor::create` の中で `open_window` する。
        App::run_component::<Distractor>(config)?;
        log("=== 完了 ===");
        Ok(())
    }
}

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    probe::run()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("このプローブは Windows 専用です（cfg(windows) ガード）。");
}
