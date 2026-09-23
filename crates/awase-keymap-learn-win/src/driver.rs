#![allow(unsafe_code)]

use std::cell::Cell;
use std::mem::size_of;
use std::thread;
use std::time::{Duration, Instant};

use awase_keymap_learn::anomaly::ResetLevel;
use awase_keymap_learn::exec::ImeDriver;
use awase_keymap_learn::model::{Disposition, Outcome, Status};
use awase_keymap_learn::sim::PressReport;
use awase_windows::state::key_effect_predictor::Conv;
use windows::core::{w, Interface, Result as WinResult};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::Ime::{
    ImmGetCompositionStringW, ImmGetContext, ImmGetConversionStatus, ImmGetOpenStatus,
    ImmReleaseContext, IME_COMPOSITION_STRING, IME_CONVERSION_MODE, IME_SENTENCE_MODE,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, SetFocus, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    KEYEVENTF_KEYUP, VIRTUAL_KEY,
};
use windows::Win32::UI::TextServices::{
    CLSID_TF_ThreadMgr, ITfCompartmentMgr, ITfThreadMgr,
    GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION, GUID_COMPARTMENT_KEYBOARD_OPENCLOSE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, PeekMessageW, RegisterClassW,
    SetForegroundWindow, SetWindowTextW, ShowWindow, TranslateMessage, MSG, PM_REMOVE, SW_SHOW,
    WINDOW_STYLE, WNDCLASSW, WS_BORDER, WS_CHILD, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};

const GCS_COMPSTR: u32 = 0x0008;
const SETUP_GAP_MS: u64 = 20;
const QUIET_MS: u64 = 40;
const SETTLE_TIMEOUT_MS: u64 = 150;
const FOCUS_DEBOUNCE_FALLBACK_MS: u64 = 100;
const FOCUS_MARGIN_MS: u64 = 25;

extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

#[derive(Debug)]
struct Observation {
    status: Status,
    text: String,
}

/// 専用EDIT窓、TSF thread manager、IMM観測と生SendInputを同一スレッドに保持する。
#[derive(Debug)]
pub struct RealImeDriver {
    started: Instant,
    window: HWND,
    edit: HWND,
    thread_mgr: ITfThreadMgr,
    thread_compartments: ITfCompartmentMgr,
    keys: Vec<u32>,
    initial: Status,
    /// `observe_imm()`がIME観測を復号できず`self.initial`へフォールバックした回数
    /// (ADR-195が前提とする「誤りに強い分類」が`awase-keymap-learn`に未実装のため、
    /// この駆動部だけでは異常として`Executor`に伝える経路が無い。せめて可視化する
    /// ——レビュー指摘対応)。`&self`のメソッドから増分するため`Cell`。
    decode_errors: Cell<u32>,
}

impl RealImeDriver {
    pub fn new(keys: Vec<u32>) -> WinResult<Self> {
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()? };
        let thread_mgr: ITfThreadMgr =
            unsafe { CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER)? };
        unsafe { thread_mgr.Activate()? };
        let thread_compartments = thread_mgr.cast::<ITfCompartmentMgr>()?;
        let (window, edit) = create_window()?;
        unsafe {
            let _ = SetForegroundWindow(window);
            let _ = SetFocus(Some(edit));
            let _ = ShowWindow(window, SW_SHOW);
        }

        // TODO(ADR-195段階0): config.tomlからgeneral.focus_debounce_msを読み、
        // その実値+マージンへ置き換える。読み取り統合前は保守的な100msを使う。
        pump_for(Duration::from_millis(
            FOCUS_DEBOUNCE_FALLBACK_MS + FOCUS_MARGIN_MS,
        ));
        unsafe { SetWindowTextW(edit, w!(""))? };
        pump_for(Duration::from_millis(QUIET_MS));

        let mut driver = Self {
            started: Instant::now(),
            window,
            edit,
            thread_mgr,
            thread_compartments,
            keys,
            initial: Status {
                open: false,
                mode: 0x09,
                composing: false,
            },
            decode_errors: Cell::new(0),
        };
        driver.initial = driver.observe_imm()?.status;
        Ok(driver)
    }

    pub const fn initial_status(&self) -> Status {
        self.initial
    }

    /// `observe_imm()`が復号に失敗し`self.initial`へフォールバックした回数。
    /// 0でなければ学習表に信頼できない観測が混じっている可能性がある
    /// (呼び出し元は最終サマリで表示することを推奨)。
    pub fn decode_error_count(&self) -> u32 {
        self.decode_errors.get()
    }

    fn note_decode_error(&self, reason: &str) {
        self.decode_errors.set(self.decode_errors.get() + 1);
        eprintln!(
            "[awase-keymap-learn-win] observe_imm失敗({reason})、self.initialへフォールバック \
             — この観測は信頼できない可能性がある(ADR-195: 誤りに強い分類は未実装)"
        );
    }

    fn observe_imm(&self) -> WinResult<Observation> {
        unsafe {
            let himc = ImmGetContext(self.edit);
            if himc.is_invalid() {
                self.note_decode_error("ImmGetContextが無効なハンドルを返した");
                return Err(windows::core::Error::from_thread());
            }
            let open = ImmGetOpenStatus(himc).as_bool();
            let mut raw = IME_CONVERSION_MODE::default();
            let mut sentence = IME_SENTENCE_MODE::default();
            let conv_ok =
                ImmGetConversionStatus(himc, Some(&raw mut raw), Some(&raw mut sentence)).as_bool();
            let comp_len =
                ImmGetCompositionStringW(himc, IME_COMPOSITION_STRING(GCS_COMPSTR), None, 0);
            let _ = ImmReleaseContext(self.edit, himc);
            if !conv_ok {
                self.note_decode_error("ImmGetConversionStatusが失敗した");
                return Err(windows::core::Error::from_thread());
            }
            let mode = match normalized_mode(raw.0) {
                Ok(mode) => mode,
                Err(err) => {
                    self.note_decode_error(&format!("未知の変換モード値 0x{:04X}", raw.0));
                    return Err(err);
                }
            };
            Ok(Observation {
                status: Status {
                    open,
                    mode,
                    composing: comp_len > 0,
                },
                text: window_text(self.edit),
            })
        }
    }

    fn observe_tsf(&self) -> Option<Status> {
        let open = read_compartment(
            &self.thread_compartments,
            &GUID_COMPARTMENT_KEYBOARD_OPENCLOSE,
        )? != 0;
        let raw = read_compartment(
            &self.thread_compartments,
            &GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION,
        )?;
        Some(Status {
            open,
            mode: normalized_mode(u32::try_from(raw).ok()?).ok()?,
            composing: self.observe_imm().ok()?.status.composing,
        })
    }

    fn inject(&self, key: usize) -> bool {
        self.keys.get(key).is_some_and(|vk| send_key_press(*vk))
    }

    fn settle(&self) -> Observation {
        let deadline = Instant::now() + Duration::from_millis(SETTLE_TIMEOUT_MS);
        let mut last = self.observe_imm().unwrap_or_else(|_| Observation {
            status: self.initial,
            text: String::new(),
        });
        let mut quiet_since = Instant::now();
        while Instant::now() < deadline {
            pump_for(Duration::from_millis(5));
            if let Ok(now) = self.observe_imm() {
                if now.status != last.status || now.text != last.text {
                    last = now;
                    quiet_since = Instant::now();
                } else if quiet_since.elapsed() >= Duration::from_millis(QUIET_MS) {
                    break;
                }
            }
        }
        last
    }

    fn clear_edit(&self) {
        let _ = unsafe { SetWindowTextW(self.edit, w!("")) };
        pump_for(Duration::from_millis(QUIET_MS));
    }
}

impl Drop for RealImeDriver {
    fn drop(&mut self) {
        let _ = unsafe { self.thread_mgr.Deactivate() };
        // `self.edit`は`self.window`の子窓なので、親を破棄すれば一緒に破棄される。
        let _ = unsafe { DestroyWindow(self.window) };
        unsafe { CoUninitialize() };
    }
}

impl ImeDriver for RealImeDriver {
    fn press(&mut self, key: usize) -> PressReport {
        let before = self.observe_imm().unwrap_or_else(|_| Observation {
            status: self.initial,
            text: String::new(),
        });
        let delivered = self.inject(key);
        let after = self.settle();
        let disp = disposition(&before, &after);
        let seen_b = self.observe_tsf().unwrap_or(after.status);
        PressReport {
            delivered,
            cost_ms: 0.0,
            seen: Outcome {
                status: after.status,
                disp,
            },
            seen_b,
        }
    }

    fn press_setup(&mut self, key: usize) {
        let _ = self.inject(key);
        pump_for(Duration::from_millis(SETUP_GAP_MS));
    }

    fn read_primary(&mut self) -> Status {
        self.observe_imm().map_or(self.initial, |o| o.status)
    }

    fn read_secondary(&mut self) -> Status {
        self.observe_tsf().unwrap_or(self.initial)
    }

    fn reread_status(&mut self) -> Status {
        self.observe_imm().map_or(self.initial, |o| o.status)
    }

    fn settle_setup(&mut self) -> Status {
        self.settle().status
    }

    fn reset(&mut self, level: ResetLevel) -> bool {
        self.clear_edit();
        let esc = self.keys.iter().position(|vk| *vk == 0x1B);
        if let Some(key) = esc {
            let _ = self.inject(key);
            let _ = self.inject(key);
        }
        if level >= ResetLevel::Mode {
            for vk in [0x16, 0xF2] {
                let _ = send_key_press(vk);
                pump_for(Duration::from_millis(SETUP_GAP_MS));
            }
        }
        if level == ResetLevel::Hard {
            let _ = unsafe { SetForegroundWindow(self.window) };
            let _ = unsafe { SetFocus(Some(self.edit)) };
        }
        self.settle().status == self.initial
    }

    fn elapsed_ms(&self) -> f64 {
        self.started.elapsed().as_secs_f64() * 1000.0
    }

    fn machine_initial_status(&self) -> Status {
        self.initial
    }
}

fn normalized_mode(raw: u32) -> WinResult<u8> {
    Conv::from_raw(raw).map_or_else(
        || {
            Err(windows::core::Error::new(
                windows::core::HRESULT(0x8000_4005u32.cast_signed()),
                "unsupported conversion mode",
            ))
        },
        |conv| {
            Ok(match conv {
                Conv::C10 => 0x00,
                Conv::C19 => 0x09,
                Conv::C1B => 0x0B,
            })
        },
    )
}

fn disposition(before: &Observation, after: &Observation) -> Disposition {
    if !before.status.composing {
        Disposition::None
    } else if after.status.composing {
        Disposition::Kept
    } else if before.text == after.text {
        Disposition::Discarded
    } else {
        Disposition::Committed
    }
}

fn read_compartment(manager: &ITfCompartmentMgr, guid: &windows::core::GUID) -> Option<i32> {
    unsafe {
        let compartment = manager.GetCompartment(guid).ok()?;
        i32::try_from(&compartment.GetValue().ok()?).ok()
    }
}

fn scan_for(vk: u32) -> u16 {
    match vk {
        0x1D => 0x7B,
        0x1C => 0x79,
        0xF2 | 0x15 | 0xF1 | 0xF5 | 0xF6 => 0x70,
        0xF3 | 0xF4 | 0x19 => 0x29,
        0xF0 => 0x3A,
        0x41 => 0x1E,
        0x0D => 0x1C,
        0x20 => 0x39,
        0x08 => 0x0E,
        0x1B => 0x01,
        _ => 0,
    }
}

fn send_key_press(vk: u32) -> bool {
    let make = |up| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk as u16),
                wScan: scan_for(vk),
                dwFlags: if up {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_EVENT_FLAGS(0)
                },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let cb_size = i32::try_from(size_of::<INPUT>()).expect("size_of::<INPUT>() fits in i32");
    unsafe { SendInput(&[make(false), make(true)], cb_size) == 2 }
}

fn pump_for(duration: Duration) {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&raw mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&raw const msg);
                DispatchMessageW(&raw const msg);
            }
        }
        thread::sleep(Duration::from_millis(1));
    }
}

fn window_text(hwnd: HWND) -> String {
    let mut buffer = [0u16; 512];
    let len = unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowTextW(hwnd, &mut buffer) };
    String::from_utf16_lossy(&buffer[..usize::try_from(len).unwrap_or(0)])
}

fn create_window() -> WinResult<(HWND, HWND)> {
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = w!("AwaseKeymapLearnWindow");
        let window_class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            lpszClassName: class,
            ..Default::default()
        };
        RegisterClassW(&raw const window_class);
        let window = CreateWindowExW(
            windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE::default(),
            class,
            w!("awase keymap learn"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            100,
            100,
            640,
            120,
            None,
            None,
            Some(instance.into()),
            None,
        )?;
        let edit = CreateWindowExW(
            windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE::default(),
            w!("EDIT"),
            w!(""),
            WINDOW_STYLE((WS_CHILD | WS_VISIBLE).0 | WS_BORDER.0),
            10,
            10,
            600,
            28,
            Some(window),
            None,
            Some(instance.into()),
            None,
        )?;
        Ok((window, edit))
    }
}
