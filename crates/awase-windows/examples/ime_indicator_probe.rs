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

    use windows::core::{w, Interface, BOOL, PCWSTR};
    use windows::Win32::Foundation::{HWND, LPARAM, RECT};
    use windows::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC,
        GetDIBits, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
        HGDIOBJ, SRCCOPY,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Accessibility::{
        CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationLegacyIAccessiblePattern,
        TreeScope_Descendants, UIA_LegacyIAccessiblePatternId,
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

    /// 入力インジケーター要素の、名前以外の情報(HelpText・ItemStatus・LegacyIAccessible・矩形)。
    unsafe fn indicator_props(el: &IUIAutomationElement) -> (String, RECT) {
        // SAFETY: el は有効な COM オブジェクト。
        unsafe {
            let help = el
                .CurrentHelpText()
                .map(|b| b.to_string())
                .unwrap_or_default();
            let status = el
                .CurrentItemStatus()
                .map(|b| b.to_string())
                .unwrap_or_default();
            let rect = el.CurrentBoundingRectangle().unwrap_or_default();
            let legacy = el
                .GetCurrentPattern(UIA_LegacyIAccessiblePatternId)
                .ok()
                .and_then(|u| u.cast::<IUIAutomationLegacyIAccessiblePattern>().ok())
                .map(|p| {
                    format!(
                        "name={:?} value={:?} desc={:?} state={:#x} help={:?}",
                        p.CurrentName().map(|b| b.to_string()).unwrap_or_default(),
                        p.CurrentValue().map(|b| b.to_string()).unwrap_or_default(),
                        p.CurrentDescription()
                            .map(|b| b.to_string())
                            .unwrap_or_default(),
                        p.CurrentState().unwrap_or(0),
                        p.CurrentHelp().map(|b| b.to_string()).unwrap_or_default()
                    )
                })
                .unwrap_or_else(|| "(LegacyIAccessible なし)".to_owned());
            (
                format!(
                    "help={help:?} status={status:?} rect=({},{},{},{}) legacy[{legacy}]",
                    rect.left, rect.top, rect.right, rect.bottom
                ),
                rect,
            )
        }
    }

    /// 画面の矩形を取り込み、輝度を平均で 2 値化した ASCII アートと FNV ハッシュを返す(インジケーターの「あ」/「A」の字形を見る)。
    unsafe fn capture_art(r: RECT) -> Option<(u64, String)> {
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        if w <= 0 || h <= 0 || w > 200 || h > 200 {
            return None;
        }
        // SAFETY: GDI の呼び出し。取得したハンドルは必ず解放する。
        unsafe {
            let dc = GetDC(None);
            let mem = CreateCompatibleDC(Some(dc));
            let bmp = CreateCompatibleBitmap(dc, w, h);
            let old = SelectObject(mem, HGDIOBJ(bmp.0));
            let ok = BitBlt(mem, 0, 0, w, h, Some(dc), r.left, r.top, SRCCOPY).is_ok();
            let mut bi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: u32::try_from(std::mem::size_of::<BITMAPINFOHEADER>()).unwrap_or(40),
                    biWidth: w,
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut buf = vec![0u8; usize::try_from(w * h * 4).unwrap_or(0)];
            let lines = GetDIBits(
                mem,
                bmp,
                0,
                u32::try_from(h).unwrap_or(0),
                Some(buf.as_mut_ptr().cast()),
                &mut bi,
                DIB_RGB_COLORS,
            );
            SelectObject(mem, old);
            let _ = DeleteObject(HGDIOBJ(bmp.0));
            let _ = DeleteDC(mem);
            ReleaseDC(None, dc);
            if !ok || lines == 0 {
                return None;
            }
            let luma: Vec<u32> = buf
                .chunks_exact(4)
                .map(|p| (u32::from(p[0]) * 29 + u32::from(p[1]) * 150 + u32::from(p[2]) * 77) >> 8)
                .collect();
            let mean = luma.iter().sum::<u32>() / u32::try_from(luma.len()).unwrap_or(1).max(1);
            // 背景より明るい字形も暗い字形も拾う(明るい背景に暗い字形だと、片側だけでは全部「空白」になる、MS-IME 本体で発覚)。
            let bits: Vec<bool> = luma.iter().map(|&l| l.abs_diff(mean) > 20).collect();
            let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
            for b in &bits {
                hash ^= u64::from(*b);
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
            let wu = usize::try_from(w).unwrap_or(1);
            let art = bits
                .chunks(wu)
                .map(|row| {
                    row.iter()
                        .map(|&b| if b { '#' } else { '.' })
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("/");
            Some((hash, art))
        }
    }

    /// 画面全体を 32bit の BMP として保存する(`--shots=<間隔秒>`。インジケーターやツールバーが実際に描かれているかを画像で見る用)。
    unsafe fn save_screen_bmp(path: &str) {
        use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};
        // SAFETY: GDI の呼び出し。取得したハンドルは必ず解放する。
        unsafe {
            let (w, h) = (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
            if w <= 0 || h <= 0 {
                return;
            }
            let dc = GetDC(None);
            let mem = CreateCompatibleDC(Some(dc));
            let bmp = CreateCompatibleBitmap(dc, w, h);
            let old = SelectObject(mem, HGDIOBJ(bmp.0));
            let ok = BitBlt(mem, 0, 0, w, h, Some(dc), 0, 0, SRCCOPY).is_ok();
            let mut bi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: u32::try_from(std::mem::size_of::<BITMAPINFOHEADER>()).unwrap_or(40),
                    biWidth: w,
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut buf = vec![0u8; usize::try_from(w * h * 4).unwrap_or(0)];
            let lines = GetDIBits(
                mem,
                bmp,
                0,
                u32::try_from(h).unwrap_or(0),
                Some(buf.as_mut_ptr().cast()),
                &mut bi,
                DIB_RGB_COLORS,
            );
            SelectObject(mem, old);
            let _ = DeleteObject(HGDIOBJ(bmp.0));
            let _ = DeleteDC(mem);
            ReleaseDC(None, dc);
            if !ok || lines == 0 {
                return;
            }
            let mut out = Vec::with_capacity(54 + buf.len());
            out.extend_from_slice(b"BM");
            out.extend_from_slice(&u32::try_from(54 + buf.len()).unwrap_or(0).to_le_bytes());
            out.extend_from_slice(&[0, 0, 0, 0]);
            out.extend_from_slice(&54u32.to_le_bytes());
            out.extend_from_slice(&40u32.to_le_bytes());
            out.extend_from_slice(&w.to_le_bytes());
            out.extend_from_slice(&(-h).to_le_bytes());
            out.extend_from_slice(&1u16.to_le_bytes());
            out.extend_from_slice(&32u16.to_le_bytes());
            out.extend_from_slice(&[0u8; 24]);
            out.extend_from_slice(&buf);
            let _ = std::fs::write(path, out);
        }
    }

    /// 前回と同じ集合から入力インジケーターの要素を探す。
    unsafe fn find_indicator(auto: &IUIAutomation, root: HWND) -> Option<IUIAutomationElement> {
        // SAFETY: auto・root は有効。
        unsafe {
            let el = auto.ElementFromHandle(root).ok()?;
            let cond = auto.CreateTrueCondition().ok()?;
            let arr = el.FindAll(TreeScope_Descendants, &cond).ok()?;
            for i in 0..arr.Length().unwrap_or(0) {
                if let Ok(c) = arr.GetElement(i) {
                    if c.CurrentName()
                        .map(|b| b.to_string())
                        .unwrap_or_default()
                        .starts_with("Tray Input Indicator")
                    {
                        return Some(c);
                    }
                }
            }
        }
        None
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
        let mut prev_props = String::new();
        let mut prev_hash = 0u64;
        // SAFETY: プロセスの DPI 認識を有効にして、UIA の矩形と画面座標をそろえる。
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::SetProcessDPIAware();
        }
        let t0 = Instant::now();
        let mut tick = 0u64;
        let shots_every: Option<u64> = arg("--shots=").and_then(|v| v.parse().ok());
        let mut shot_t = std::time::Instant::now().checked_sub(Duration::from_secs(3600)).unwrap_or_else(std::time::Instant::now);
        let mut shots_n = 0u32;
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
            // 入力インジケーターの名前以外の情報と、画面上の字形(「あ」/「A」)の変化。
            let tray = find(w!("Shell_TrayWnd"));
            if !tray.0.is_null() {
                // SAFETY: auto・tray は有効。
                if let Some(ind) = unsafe { find_indicator(&auto, tray) } {
                    // SAFETY: ind は有効。
                    let (props, rect) = unsafe { indicator_props(&ind) };
                    if props != prev_props {
                        log.line(&format!("IND-PROPS {props}"));
                        prev_props = props;
                    }
                    // SAFETY: GDI の呼び出し(ハンドルは関数内で解放)。
                    if let Some((hash, art)) = unsafe { capture_art(rect) } {
                        if hash != prev_hash {
                            log.line(&format!("IND-PIX hash={hash:016x} art={art}"));
                            prev_hash = hash;
                        }
                    }
                }
            }
            if let Some(every) = shots_every {
                if shot_t.elapsed() >= Duration::from_secs(every) && shots_n < 12 {
                    shot_t = std::time::Instant::now();
                    // SAFETY: GDI の呼び出し(ハンドルは関数内で解放)。
                    unsafe { save_screen_bmp(&format!("indshot-{shots_n:02}.bmp")) };
                    log.line(&format!("IND-SHOT n={shots_n}"));
                    shots_n += 1;
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
