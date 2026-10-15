//! タスクバーの IME 入力インジケーター(「あ」/「A」)を UI Automation で読めるかの測定プローブ(観測のみ、awase 非依存)。
//!
//! 背景(ADR-260 の議論): 読み(IMM)が効かない窓(一太郎・実機の Chrome 等)でも、タスクバーのインジケーターは
//! フォーカス中のスレッドの IME の状態を表示している(ユーザーが「タスクバーは OFF なのに」と報告している情報)。
//! 外部プロセスから UIA で読めれば、観測の無い窓の入力モード(かな/それ以外)を観測できる可能性がある。ADR-223 F 案・ADR-258 Q5 に
//! 「入力インジケーターの UIA は未測定」と残っている。
//!
//! 動作: 起動時にトップレベルウィンドウの一覧(クラス名・タイトル)と、`Shell_TrayWnd`/`Shell_SecondaryTrayWnd` 配下の UIA 要素を
//! すべてダンプし、以後 `--interval` ms ごとに同じ要素の集合を取り直して、変化(追加・削除)を時刻つきで記録する。
//! 並行して `ime_key_matrix_spike`(`--auto --seq=...`)が IME の状態を変えるので、ログの時刻で実 IME(spike の KEY 行)と突き合わせる。
//!
//! 使い方: `ime_indicator_probe [--log=<path>] [--duration=<秒>] [--interval=<ms>]`
//! ログ行: `<UTC> IND-ROOTS …`(起動時の窓の一覧)/ `IND-DUMP …`(起動時の全要素)/ `IND + …`・`IND - …`(変化)/ `IND-TICK n=…`(周期の件数)/ `=== 完了 ===`。

#[cfg(windows)]
mod app {
    use std::collections::BTreeSet;
    use std::io::Write as _;
    use std::time::{Duration, Instant};

    use windows::core::{w, BOOL, PCWSTR};
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Accessibility::{
        CUIAutomation, IUIAutomation, IUIAutomationElement, TreeScope_Descendants,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, FindWindowW, GetClassNameW, GetWindowTextW, IsWindowVisible,
    };

    fn utc_stamp() -> String {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let secs = t.as_secs() % 86_400;
        format!(
            "{:02}:{:02}:{:02}.{:03}Z",
            secs / 3600,
            (secs / 60) % 60,
            secs % 60,
            t.subsec_millis()
        )
    }

    struct Log(std::fs::File);
    impl Log {
        fn line(&mut self, s: &str) {
            let l = format!("[{}] {s}", utc_stamp());
            println!("{l}");
            let _ = writeln!(self.0, "{l}");
        }
    }

    fn arg(prefix: &str) -> Option<String> {
        std::env::args().find_map(|a| a.strip_prefix(prefix).map(str::to_owned))
    }

    unsafe extern "system" fn enum_cb(h: HWND, lp: LPARAM) -> BOOL {
        // SAFETY: lp は呼び出し元が渡した `Vec<String>` の可変参照。
        let v = unsafe { &mut *(lp.0 as *mut Vec<String>) };
        let mut cls = [0u16; 128];
        let mut title = [0u16; 128];
        // SAFETY: バッファは有効。
        let (cn, tn) = unsafe { (GetClassNameW(h, &mut cls), GetWindowTextW(h, &mut title)) };
        let cn = usize::try_from(cn).unwrap_or(0);
        let tn = usize::try_from(tn).unwrap_or(0);
        // SAFETY: h は列挙された有効なハンドル。
        let vis = unsafe { IsWindowVisible(h).as_bool() };
        v.push(format!(
            "hwnd={:#x} visible={vis} class={:?} title={:?}",
            h.0 as usize,
            String::from_utf16_lossy(&cls[..cn]),
            String::from_utf16_lossy(&title[..tn])
        ));
        BOOL(1)
    }

    /// 1 要素を `ctrl=<型> class=<クラス> id=<AutomationId> name=<名前>` の文字列にする。
    unsafe fn describe(el: &IUIAutomationElement) -> String {
        // SAFETY: el は有効な COM オブジェクト。
        unsafe {
            let name = el.CurrentName().map(|b| b.to_string()).unwrap_or_default();
            let class = el
                .CurrentClassName()
                .map(|b| b.to_string())
                .unwrap_or_default();
            let id = el
                .CurrentAutomationId()
                .map(|b| b.to_string())
                .unwrap_or_default();
            let ct = el.CurrentControlType().map(|c| c.0).unwrap_or(0);
            format!("ctrl={ct} class={class:?} id={id:?} name={name:?}")
        }
    }

    unsafe fn collect(auto: &IUIAutomation, root: HWND) -> BTreeSet<String> {
        let mut set = BTreeSet::new();
        // SAFETY: auto は有効な IUIAutomation。root は FindWindow で得たハンドル。
        unsafe {
            let Ok(el) = auto.ElementFromHandle(root) else {
                return set;
            };
            set.insert(format!("ROOT {}", describe(&el)));
            let Ok(cond) = auto.CreateTrueCondition() else {
                return set;
            };
            let Ok(arr) = el.FindAll(TreeScope_Descendants, &cond) else {
                return set;
            };
            let n = arr.Length().unwrap_or(0);
            for i in 0..n {
                if let Ok(c) = arr.GetElement(i) {
                    set.insert(describe(&c));
                }
            }
        }
        set
    }

    pub(crate) fn main() {
        let path = arg("--log=").unwrap_or_else(|| "ime_indicator_probe.log".to_owned());
        let duration: u64 = arg("--duration=")
            .and_then(|v| v.parse().ok())
            .unwrap_or(120);
        let interval: u64 = arg("--interval=")
            .and_then(|v| v.parse().ok())
            .unwrap_or(100);
        let Ok(file) = std::fs::File::create(&path) else {
            eprintln!("ログを作れない: {path}");
            return;
        };
        let mut log = Log(file);
        log.line(&format!("start duration={duration}s interval={interval}ms"));
        // SAFETY: 単一スレッドで COM を初期化してから UIA を作る。
        let auto: IUIAutomation = unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            match CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) {
                Ok(a) => a,
                Err(e) => {
                    log.line(&format!("ABORT CUIAutomation を作れない: {e}"));
                    return;
                }
            }
        };
        // トップレベルウィンドウの一覧(タスクバー関連のクラスが存在するかの確認)。
        let mut wins: Vec<String> = Vec::new();
        // SAFETY: コールバックは同期的に呼ばれ、wins はその間有効。
        unsafe {
            let _ = EnumWindows(Some(enum_cb), LPARAM(std::ptr::addr_of_mut!(wins) as isize));
        }
        for w in &wins {
            log.line(&format!("IND-ROOTS {w}"));
        }
        let classes: [(&str, PCWSTR); 2] = [
            ("Shell_TrayWnd", w!("Shell_TrayWnd")),
            ("Shell_SecondaryTrayWnd", w!("Shell_SecondaryTrayWnd")),
        ];
        let find = |p: PCWSTR| -> HWND {
            // SAFETY: 定数のクラス名で検索するだけ。
            unsafe { FindWindowW(p, PCWSTR::null()) }.unwrap_or_default()
        };
        let mut prev: BTreeSet<String> = BTreeSet::new();
        let t0 = Instant::now();
        let mut tick = 0u64;
        while t0.elapsed() < Duration::from_secs(duration) {
            let mut now = BTreeSet::new();
            for (name, p) in &classes {
                let h = find(*p);
                if h.0.is_null() {
                    if tick == 0 {
                        log.line(&format!("IND-ROOT-MISSING {name}"));
                    }
                    continue;
                }
                // SAFETY: auto・h は有効。
                for l in unsafe { collect(&auto, h) } {
                    now.insert(format!("{name}: {l}"));
                }
            }
            if tick == 0 {
                for l in &now {
                    log.line(&format!("IND-DUMP {l}"));
                }
            } else {
                for l in now.difference(&prev) {
                    log.line(&format!("IND + {l}"));
                }
                for l in prev.difference(&now) {
                    log.line(&format!("IND - {l}"));
                }
            }
            if tick % 50 == 0 {
                log.line(&format!("IND-TICK tick={tick} elements={}", now.len()));
            }
            prev = now;
            tick += 1;
            std::thread::sleep(Duration::from_millis(interval));
        }
        log.line("=== 完了 ===");
    }
}

#[cfg(windows)]
fn main() {
    app::main();
}

#[cfg(not(windows))]
fn main() {}
