//! BUG-195 の再現プローブ: 「入力設定をアプリ ウィンドウごとに異なる値にする」
//! (`SPI_SETTHREADLOCALINPUTSETTINGS`)の ON/OFF で、あるスレッドで GJI を有効化したとき、
//! 別スレッド(awase の `gji-io-monitor` と同じく窓を持たない STA スレッド)の
//! `ITfInputProcessorProfileMgr::GetActiveProfile` が何を返すかを観測する。判定は付けない。
//!
//! 使い方: `tip_scope_probe --thread-local=on|off [--secs=6] [--winspace]`
#![allow(unsafe_code)]
#![allow(clippy::all, clippy::pedantic, clippy::nursery)]

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
fn main() {
    imp::run();
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use windows::core::{w, Interface as _, GUID};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::TextServices::{
        CLSID_TF_InputProcessorProfiles, ITfInputProcessorProfileMgr, ITfInputProcessorProfiles,
        GUID_TFCAT_TIP_KEYBOARD, TF_INPUTPROCESSORPROFILE, TF_PROFILETYPE_INPUTPROCESSOR,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DispatchMessageW, PeekMessageW, SetForegroundWindow,
        SystemParametersInfoW, TranslateMessage, MSG, PM_REMOVE, SPI_GETTHREADLOCALINPUTSETTINGS,
        SPI_SETTHREADLOCALINPUTSETTINGS, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, WINDOW_EX_STYLE,
        WS_OVERLAPPEDWINDOW, WS_VISIBLE,
    };

    fn say(s: &str) {
        println!("PROBE {s}");
    }

    fn ctx() -> Option<(ITfInputProcessorProfileMgr, ITfInputProcessorProfiles)> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok();
            let mgr: ITfInputProcessorProfileMgr =
                CoCreateInstance(&CLSID_TF_InputProcessorProfiles, None, CLSCTX_INPROC_SERVER)
                    .ok()?;
            let profiles: ITfInputProcessorProfiles = mgr.cast().ok()?;
            Some((mgr, profiles))
        }
    }

    fn describe(
        profiles: &ITfInputProcessorProfiles,
        p: &TF_INPUTPROCESSORPROFILE,
    ) -> String {
        unsafe {
            profiles
                .GetLanguageProfileDescription(&raw const p.clsid, p.langid, &raw const p.guidProfile)
                .map(|b| b.to_string())
                .unwrap_or_default()
        }
    }

    fn active(
        mgr: &ITfInputProcessorProfileMgr,
        profiles: &ITfInputProcessorProfiles,
    ) -> String {
        unsafe {
            let mut p = TF_INPUTPROCESSORPROFILE::default();
            match mgr.GetActiveProfile(&GUID_TFCAT_TIP_KEYBOARD, &raw mut p) {
                Ok(()) => format!(
                    "type={} clsid={:032X} desc={:?}",
                    p.dwProfileType,
                    p.clsid.to_u128(),
                    describe(profiles, &p)
                ),
                Err(e) => format!("GetActiveProfile failed: {e}"),
            }
        }
    }

    fn find_gji(
        mgr: &ITfInputProcessorProfileMgr,
        profiles: &ITfInputProcessorProfiles,
    ) -> Option<(GUID, GUID)> {
        unsafe {
            let e = mgr.EnumProfiles(0x0411).ok()?;
            let mut found = None;
            loop {
                let mut p = TF_INPUTPROCESSORPROFILE::default();
                let mut n = 0u32;
                if e.Next(std::slice::from_mut(&mut p), &raw mut n).is_err() || n == 0 {
                    break;
                }
                let d = describe(profiles, &p);
                say(&format!(
                    "enum type={} clsid={:032X} flags={:#x} desc={d:?}",
                    p.dwProfileType,
                    p.clsid.to_u128(),
                    p.dwFlags
                ));
                if p.dwProfileType == TF_PROFILETYPE_INPUTPROCESSOR && d.contains("Google") {
                    found = Some((p.clsid, p.guidProfile));
                }
            }
            found
        }
    }

    pub fn run() {
        let args: Vec<String> = std::env::args().collect();
        let on = args.iter().any(|a| a == "--thread-local=on");
        let secs: u64 = args
            .iter()
            .find_map(|a| a.strip_prefix("--secs="))
            .and_then(|s| s.parse().ok())
            .unwrap_or(6);
        unsafe {
            let mut cur = 0i32;
            let _ = SystemParametersInfoW(
                SPI_GETTHREADLOCALINPUTSETTINGS,
                0,
                Some((&raw mut cur).cast::<c_void>()),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            );
            say(&format!("thread_local_input_settings before={cur}"));
            let r = SystemParametersInfoW(
                SPI_SETTHREADLOCALINPUTSETTINGS,
                0,
                Some(usize::from(on).max(0) as *mut c_void),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(1),
            );
            let mut after = 0i32;
            let _ = SystemParametersInfoW(
                SPI_GETTHREADLOCALINPUTSETTINGS,
                0,
                Some((&raw mut after).cast::<c_void>()),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            );
            say(&format!("set(on={on}) -> {r:?}, readback={after}"));
        }

        let Some((mgr, profiles)) = ctx() else {
            say("COM init failed");
            return;
        };
        say(&format!("monitor(initial) {}", active(&mgr, &profiles)));
        let Some((clsid, pguid)) = find_gji(&mgr, &profiles) else {
            say("GJI not found in EnumProfiles(JA)");
            return;
        };

        let winspace = args.iter().any(|a| a == "--winspace");
        if args.iter().any(|a| a == "--reset-ms") {
            // 前の実行の Win+Space の影響を消すため、セッション全体で Microsoft IME に戻す。
            let r = unsafe {
                mgr.ActivateProfile(
                    TF_PROFILETYPE_INPUTPROCESSOR,
                    0x0411,
                    &GUID::from_u128(0x03B5835F_F03C_411B_9CE2_AA23E1171E36),
                    &GUID::from_u128(0xA76C93D9_5523_4E90_AAFA_4DB112F9AC76),
                    windows::Win32::UI::Input::KeyboardAndMouse::HKL(std::ptr::null_mut()),
                    0x1 | 0x2000_0000,
                )
            };
            say(&format!("reset to MS-IME (session) -> {r:?}"));
            std::thread::sleep(Duration::from_millis(1500));
            say(&format!("monitor(after reset) {}", active(&mgr, &profiles)));
        }
        let stop = Arc::new(AtomicBool::new(false));
        let stop_w = stop.clone();
        let w_thread = std::thread::spawn(move || unsafe {
            let winspace = winspace;
            let Some((wmgr, wprofiles)) = ctx() else {
                return;
            };
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("TIPSCOPE"),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                100,
                100,
                400,
                200,
                None,
                None,
                None,
                None,
            );
            if let Ok(h) = hwnd {
                let _ = SetForegroundWindow(h);
            }
            if winspace {
                // 実利用と同じ Win+Space を前面窓(このスレッド)へ送り、GJI になるまで最大4回切り替える。
                use windows::Win32::UI::Input::KeyboardAndMouse::{
                    GetKeyboardLayout, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
                    KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VIRTUAL_KEY,
                };
                let key = |vk: u16, up: bool| INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VIRTUAL_KEY(vk),
                            wScan: 0,
                            dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                };
                for i in 0..4 {
                    std::thread::sleep(Duration::from_millis(1500));
                    let seq = [key(0x5B, false), key(0x20, false), key(0x20, true), key(0x5B, true)];
                    let n = SendInput(&seq, std::mem::size_of::<INPUT>() as i32);
                    std::thread::sleep(Duration::from_millis(1200));
                    let a = active(&wmgr, &wprofiles);
                    say(&format!(
                        "winspace#{i} sent={n} hkl={:?} window-thread {a}",
                        GetKeyboardLayout(0).0
                    ));
                    if a.contains("Google") {
                        break;
                    }
                }
            } else {
                let r = wmgr.ActivateProfile(
                    TF_PROFILETYPE_INPUTPROCESSOR,
                    0x0411,
                    &clsid,
                    &pguid,
                    windows::Win32::UI::Input::KeyboardAndMouse::HKL(std::ptr::null_mut()),
                    0x1,
                );
                say(&format!("window-thread ActivateProfile(GJI) -> {r:?}"));
            }
            let mut last = Instant::now() - Duration::from_secs(10);
            while !stop_w.load(Ordering::Relaxed) {
                let mut msg = MSG::default();
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                if last.elapsed() >= Duration::from_secs(1) {
                    last = Instant::now();
                    say(&format!("window-thread {}", active(&wmgr, &wprofiles)));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        });

        let t0 = Instant::now();
        while t0.elapsed() < Duration::from_secs(secs) {
            std::thread::sleep(Duration::from_secs(1));
            say(&format!("monitor {}", active(&mgr, &profiles)));
        }
        stop.store(true, Ordering::Relaxed);
        let _ = w_thread.join();
    }
}
