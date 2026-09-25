//! 検証専用（develop へはマージしない）。ADR-191 A/B-1 アプローチ3:
//! `ir_post_focus_change_snapshot` の `focus_change_enforce_off` を、実HWND・実IMMで
//! プロセス内から駆動して観測する。`AB1_RUN=1` のときだけ動く。
#![allow(unsafe_code, clippy::all, clippy::pedantic, clippy::nursery)]

use std::io::Write as _;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use awase::config::{AppOverrides, ConfirmMode};
use awase::engine::{Engine, NicolaFsm, SpecialKeyCombos};
use awase::types::VkCode;
use awase::yab::{YabFace, YabLayout};
use windows::core::PCWSTR;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::Ime::{
    ImmGetContext, ImmGetOpenStatus, ImmReleaseContext, ImmSetOpenStatus,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, CreateWindowExW, DispatchMessageW, GetForegroundWindow, PeekMessageW,
    SetForegroundWindow, ShowWindow, TranslateMessage, MSG, PM_REMOVE, SW_SHOW,
    WINDOW_EX_STYLE, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};

use super::Runtime;

#[derive(Clone)]
struct Buf(Arc<Mutex<Vec<u8>>>);
impl std::io::Write for Buf {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn pump_for(d: Duration) {
    let end = Instant::now() + d;
    while Instant::now() < end {
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

unsafe fn make_edit_window() -> HWND {
    let class: Vec<u16> = "EDIT\0".encode_utf16().collect();
    let h = CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        PCWSTR(class.as_ptr()),
        PCWSTR::null(),
        WS_OVERLAPPEDWINDOW | WS_VISIBLE,
        0,
        0,
        400,
        200,
        None,
        None,
        None,
        None,
    )
    .expect("CreateWindowExW(EDIT)");
    let _ = ShowWindow(h, SW_SHOW);
    h
}

unsafe fn foreground(h: HWND) {
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
    let fg = GetForegroundWindow();
    let fg_thread = GetWindowThreadProcessId(fg, None);
    let cur = GetCurrentThreadId();
    let attached = fg_thread != cur && AttachThreadInput(cur, fg_thread, true).as_bool();
    let _ = ShowWindow(h, SW_SHOW);
    let _ = SetForegroundWindow(h);
    let _ = BringWindowToTop(h);
    if attached {
        let _ = AttachThreadInput(cur, fg_thread, false);
    }
    pump_for(Duration::from_millis(150));
}

unsafe fn set_open(h: HWND, open: bool) -> bool {
    let himc = ImmGetContext(h);
    if himc.is_invalid() {
        return false;
    }
    let r = ImmSetOpenStatus(himc, open).as_bool();
    let _ = ImmReleaseContext(h, himc);
    r
}

unsafe fn get_open(h: HWND) -> Option<bool> {
    let himc = ImmGetContext(h);
    if himc.is_invalid() {
        return None;
    }
    let r = ImmGetOpenStatus(himc).as_bool();
    let _ = ImmReleaseContext(h, himc);
    Some(r)
}

fn build_runtime(dir: &std::path::Path) -> Runtime {
    let mut normal = YabFace::new();
    let _ = &mut normal;
    let layout = YabLayout {
        name: String::from("ab1"),
        normal,
        left_thumb: YabFace::new(),
        right_thumb: YabFace::new(),
        shift: YabFace::new(),
        left_thumb_shift: YabFace::new(),
        right_thumb_shift: YabFace::new(),
    };
    let (l, r) = (VkCode(0x1D), VkCode(0x1C));
    let fsm = NicolaFsm::new(layout, l, r, 100, ConfirmMode::Wait, 30);
    let engine = Engine::new(
        fsm,
        SpecialKeyCombos {
            engine_on: Vec::new(),
            engine_off: Vec::new(),
            ime_on: Vec::new(),
            ime_off: Vec::new(),
            ime_toggle: Vec::new(),
        },
    );
    let mut ps = crate::PlatformState::new();
    ps.focus.focus_debounce_ms = 0;
    let stamper = ps.ime.journal.stamper();
    let tray = crate::tray::SystemTray::new(true, false).expect("SystemTray::new");
    let platform = crate::platform::WindowsPlatform::new(
        crate::output::Output::new(),
        tray,
        crate::timer::Win32Timer::new(),
        None,
        None,
        false,
        crate::focus::tracker::FocusTracker::new(
            crate::focus::cache::FocusCache::new(),
            crate::focus::classifier::ForceOverrides::new(AppOverrides::default()),
            crate::focus::classifier::ImmCapabilityStore::new(dir.to_path_buf()),
            crate::focus::classifier::InjectionModeStore::new(dir.to_path_buf()),
        ),
        crate::tsf::composition_fsm::CompositionFsm::new(),
        stamper,
    );
    Runtime::new(
        engine,
        super::executor::DecisionExecutor::new(),
        platform,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        ps,
        crate::keymap::KeymapTable::new(&[], l, r),
        Vec::new(),
    )
}

#[test]
fn ab1_focus_enforce_off_in_process() {
    if std::env::var("AB1_RUN").is_err() {
        return;
    }
    let runs: usize = std::env::var("AB1_RUNS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8);

    let log = Buf(Arc::new(Mutex::new(Vec::new())));
    let log_w = log.clone();
    let sub = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .with_writer(move || log_w.clone())
        .finish();
    let _g = tracing::subscriber::set_default(sub);

    // 窓クラス "Edit" を ImmCross として扱わせる（テスト exe 名で works を事前登録）。
    let dir = std::env::temp_dir().join("ab1-approach3");
    let _ = std::fs::create_dir_all(&dir);
    let exe = std::env::current_exe()
        .unwrap()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_lowercase();
    let _ = std::fs::write(
        dir.join("cache.toml"),
        format!("[imm_capability.\"{exe}\"]\nEdit = \"works\"\n"),
    );

    let mut rt = build_runtime(&dir);
    let (w1, w2) = unsafe { (make_edit_window(), make_edit_window()) };
    println!("AB1|setup|w1={w1:?}|w2={w2:?}|exe={exe}");

    for i in 1..=runs {
        unsafe {
            let _ = set_open(w1, true);
            let _ = set_open(w2, true);
            foreground(w1);
        }
        rt.run_ime_refresh();
        let tick = crate::hook::current_tick_ms();
        rt.platform_state.ime.record_confirmed(false, tick);
        log.0.lock().unwrap().clear();

        unsafe { foreground(w2) };
        let t0 = Instant::now();
        rt.run_ime_refresh();
        let mut obs = Vec::new();
        for ms in [0u64, 100, 400, 1500] {
            let target = Duration::from_millis(ms);
            if t0.elapsed() < target {
                pump_for(target - t0.elapsed());
            }
            obs.push(unsafe { get_open(w2) });
        }
        let text = String::from_utf8_lossy(&log.0.lock().unwrap()).to_string();
        let fired = text.contains("focus_change_enforce_off")
            || text.contains("enforce IME OFF on new window");
        let sent = text
            .lines()
            .find_map(|l| l.split("sent=").nth(1).map(|s| s.split_whitespace().next().unwrap_or("?").to_string()))
            .unwrap_or_else(|| "-".into());
        println!(
            "AB1|run={i}|fired={fired}|sent={sent}|w2_open(0/100/400/1500ms)={obs:?}|fg_is_w2={}",
            unsafe { GetForegroundWindow() } == w2
        );
        let _ = std::io::stdout().flush();
        if i == 1 {
            println!("AB1|firstlog|{}", text.lines().take(60).collect::<Vec<_>>().join("\nAB1|firstlog|"));
        }
    }
}
