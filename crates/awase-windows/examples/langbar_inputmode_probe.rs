//! 別プロセスの IME 入力モードを、言語バーの項目の文字列(`ITfLangBarItemButton::GetText`/`GetTooltipString`)で読めるかの測定プローブ
//! (ADR-261 D0-0′。使い捨て、観測のみ、awase 非依存)。
//!
//! 既存の `spike_langbar_input_mode --watch-langbar` は `ITfThreadMgr` 経由で **awase 自身のスレッド**の言語バー項目を見るだけで、
//! 他プロセスの状態は測れない。ここでは `ITfLangBarMgr::GetThreadLangBarItemMgr(スレッド ID)` で**前面/フォーカスのスレッド**の
//! 項目の管理を取り、全項目(GUID・説明・テキスト・ツールチップ)を `--interval` ms ごとに読んで、変化だけを時刻つきで記録する。
//! 並行して `ime_key_matrix_spike`(`--auto --seq=...`)が IME の状態を変えるので、ログの時刻で実 IME(spike の KEY 行)と突き合わせる。
//!
//! 使い方: `langbar_inputmode_probe [--log=<path>] [--duration=<秒>] [--interval=<ms>]`
//! ログ行: `<UTC> LB-ERR …`(取得失敗)/ `LB tid=<> kind=fg|focus items=[…]`(変化)/ `LB-TICK n=<>`。

#[cfg(windows)]
mod app {
    use std::io::Write as _;
    use std::time::{Duration, Instant};

    use windows::core::Interface;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::TextServices::{
        CLSID_TF_LangBarMgr, CLSID_TF_ThreadMgr, ITfLangBarItem, ITfLangBarItemButton,
        ITfLangBarItemMgr, ITfLangBarMgr, ITfThreadMgr, TF_LANGBARITEMINFO,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId, GUITHREADINFO,
    };

    fn arg(prefix: &str) -> Option<String> {
        std::env::args().find_map(|a| a.strip_prefix(prefix).map(str::to_owned))
    }

    fn utc_stamp() -> String {
        let d = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let secs = d.as_secs() % 86_400;
        format!(
            "{:02}:{:02}:{:02}.{:03}Z",
            secs / 3600,
            (secs / 60) % 60,
            secs % 60,
            d.subsec_millis()
        )
    }

    struct Log(std::fs::File);
    impl Log {
        fn line(&mut self, s: &str) {
            let _ = writeln!(self.0, "[{}] {s}", utc_stamp());
            let _ = self.0.flush();
        }
    }

    fn guid_str(g: &windows::core::GUID) -> String {
        format!("{g:?}")
    }

    /// 項目 1 つを「GUID|説明|テキスト|ツールチップ」の文字列にする。
    fn describe(item: &ITfLangBarItem) -> String {
        let mut info = TF_LANGBARITEMINFO::default();
        // SAFETY: item は有効な COM 参照、info はローカル。
        let ok = unsafe { item.GetInfo(&raw mut info) }.is_ok();
        let desc = if ok {
            let n = info.szDescription.iter().position(|&c| c == 0).unwrap_or(32);
            String::from_utf16_lossy(&info.szDescription[..n])
        } else {
            "?".to_owned()
        };
        let guid = if ok { guid_str(&info.guidItem) } else { "?".to_owned() };
        let (text, tip) = match item.cast::<ITfLangBarItemButton>() {
            Ok(b) => (
                // SAFETY: b は有効な COM 参照。
                unsafe { b.GetText() }.map(|s| s.to_string()).unwrap_or_else(|e| format!("ERR{:x}", e.code().0)),
                unsafe { b.GetTooltipString() }.map(|s| s.to_string()).unwrap_or_else(|e| format!("ERR{:x}", e.code().0)),
            ),
            Err(_) => ("(not button)".to_owned(), String::new()),
        };
        format!("{guid}|{desc}|{text}|{tip}")
    }

    fn items_of(mgr: &ITfLangBarMgr, tid: u32) -> Result<String, String> {
        let mut item_mgr: Option<ITfLangBarItemMgr> = None;
        let mut got_tid = 0u32;
        // SAFETY: mgr は有効な COM 参照、出力先はローカル。
        unsafe { mgr.GetThreadLangBarItemMgr(tid, &raw mut item_mgr, &raw mut got_tid) }
            .map_err(|e| format!("GetThreadLangBarItemMgr hr={:x}", e.code().0))?;
        let item_mgr = item_mgr.ok_or_else(|| "item_mgr=None".to_owned())?;
        // SAFETY: item_mgr は有効な COM 参照。
        let en = unsafe { item_mgr.EnumItems() }.map_err(|e| format!("EnumItems hr={:x}", e.code().0))?;
        let mut out = Vec::new();
        loop {
            let mut buf: [Option<ITfLangBarItem>; 1] = [None];
            let mut fetched = 0u32;
            // SAFETY: en は有効な COM 参照、出力先はローカル。
            let hr = unsafe { en.Next(&mut buf, &raw mut fetched) };
            if hr.is_err() || fetched == 0 {
                break;
            }
            if let Some(it) = buf[0].take() {
                out.push(describe(&it));
            }
        }
        Ok(format!("got_tid={got_tid} items=[{}]", out.join(" ; ")))
    }

    /// `GetThreadMarshalInterface`(別プロセス向けのマーシャリング経路)を試す。結果は「成功/HRESULT」だけ。
    fn marshal_try(mgr: &ITfLangBarMgr, tid: u32) -> String {
        let mut out = Vec::new();
        for dw_type in [0u32, 1, 2] {
            // SAFETY: mgr は有効な COM 参照。
            let r = unsafe { mgr.GetThreadMarshalInterface(tid, dw_type, &ITfLangBarItemMgr::IID) };
            out.push(match r {
                Ok(_) => format!("type{dw_type}=OK"),
                Err(e) => format!("type{dw_type}=hr{:x}", e.code().0),
            });
        }
        out.join(",")
    }

    pub fn run() -> anyhow::Result<()> {
        let path = arg("--log=").unwrap_or_else(|| "langbar_inputmode_probe.log".to_owned());
        let duration: u64 = arg("--duration=").and_then(|v| v.parse().ok()).unwrap_or(60);
        let interval: u64 = arg("--interval=").and_then(|v| v.parse().ok()).unwrap_or(100);
        let mut log = Log(std::fs::OpenOptions::new().create(true).append(true).open(&path)?);
        log.line(&format!("start duration={duration}s interval={interval}ms"));

        // SAFETY: プロセス起動直後の単発初期化。
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.ok()?;
        // SAFETY: 標準的な COM オブジェクト生成。
        let mgr: ITfLangBarMgr = unsafe { CoCreateInstance(&CLSID_TF_LangBarMgr, None, CLSCTX_INPROC_SERVER) }?;
        log.line("ITfLangBarMgr: OK");
        // 正の対照: 自スレッドで ITfThreadMgr を Activate してから、自スレッドの項目が読めるか。
        // SAFETY: 標準的な COM オブジェクト生成。
        let tm: Option<ITfThreadMgr> = unsafe { CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER) }.ok();
        if let Some(tm) = tm.as_ref() {
            // SAFETY: tm は有効な COM 参照。
            let r = unsafe { tm.Activate() };
            log.line(&format!("LB-CTRL ThreadMgr.Activate={:?}", r.as_ref().map(|_| "OK").map_err(|e| e.code().0)));
        }
        // SAFETY: 読み取りのみ。
        let self_tid = unsafe { windows::Win32::System::Threading::GetCurrentThreadId() };
        match items_of(&mgr, self_tid) {
            Ok(v) => log.line(&format!("LB-CTRL self tid={self_tid} {v}")),
            Err(e) => log.line(&format!("LB-CTRL self tid={self_tid} ERR {e}")),
        }
        log.line(&format!("LB-CTRL self marshal {}", marshal_try(&mgr, self_tid)));

        let end = Instant::now() + Duration::from_secs(duration);
        let mut prev: std::collections::HashMap<&'static str, String> = std::collections::HashMap::new();
        let mut tick = 0u64;
        while Instant::now() < end {
            // SAFETY: Win32 の読み取り API。
            let fg = unsafe { GetForegroundWindow() };
            let fg_tid = if fg.0.is_null() { 0 } else { unsafe { GetWindowThreadProcessId(fg, None) } };
            let mut gti = GUITHREADINFO {
                cbSize: u32::try_from(std::mem::size_of::<GUITHREADINFO>()).unwrap_or(0),
                ..Default::default()
            };
            let focus_tid = if fg_tid != 0 && unsafe { GetGUIThreadInfo(fg_tid, &raw mut gti) }.is_ok() && !gti.hwndFocus.0.is_null() {
                unsafe { GetWindowThreadProcessId(gti.hwndFocus, None) }
            } else {
                0
            };
            for (kind, tid) in [("fg", fg_tid), ("focus", focus_tid)] {
                if tid == 0 {
                    continue;
                }
                let cur = match items_of(&mgr, tid) {
                    Ok(s) => format!("tid={tid} {s}"),
                    Err(e) => format!("tid={tid} LB-ERR {e}"),
                };
                if prev.get(kind) != Some(&cur) {
                    log.line(&format!("LB kind={kind} {cur} marshal[{}]", marshal_try(&mgr, tid)));
                    prev.insert(kind, cur);
                }
            }
            if tick % 50 == 0 {
                log.line(&format!("LB-TICK n={tick} fg_tid={fg_tid} focus_tid={focus_tid}"));
            }
            tick += 1;
            std::thread::sleep(Duration::from_millis(interval));
        }
        log.line("done");
        Ok(())
    }
}

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    app::run()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Windows 専用");
}
