#![allow(unsafe_code)]

use std::cell::Cell;
use std::mem::size_of;
use std::thread;
use std::time::{Duration, Instant};

use awase::config::AppConfig;
use awase::paths::resolve_relative_to_exe;
use awase_keymap_learn::anomaly::ResetLevel;
use awase_keymap_learn::exec::ImeDriver;
use awase_keymap_learn::external_write::{
    is_measurement_suspicious, trial_contaminated, SessionMonitor,
};
use awase_keymap_learn::model::{Disposition, Outcome, Status};
use awase_keymap_learn::sim::PressReport;
use awase_windows::state::key_effect_predictor::Conv;

use crate::hook_monitor::{HookMonitor, SELF_MARKER};
use crate::ime_notify::ImeNotifyMonitor;
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
    GetFocus, SendInput, SetFocus, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
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
/// ADR-196決定1b項目3: フォーカス移行・デバウンス待ち直後に、注入を一切しない
/// 期間を置き、その間に外部からの書き込みが観測されないことをセッション開始
/// 条件にする（quiet window）。暫定値——`.claude/rules/tuning-constants.md`の
/// 実測義務に従い、実機プロトタイプでの計測後に更新すること。
const QUIET_WINDOW_MS: u64 = 200;
/// ADR-196決定1b項目5: セッション中に外部からの書き込みで試行が無効化された
/// 回数の上限。超えたらセッション全体を失敗として終了する。暫定値、実測で
/// 更新する。
const SESSION_INVALIDATION_LIMIT: u32 = 3;
/// 自分の注入によって`WM_IME_NOTIFY`が届くと期待してよい猶予（`settle()`の
/// `SETTLE_TIMEOUT_MS`と揃える）。
const NOTIFY_EXPECT_WINDOW_MS: u64 = SETTLE_TIMEOUT_MS;

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
    /// ADR-196決定1b: 学習窓への「自分以外からの書き込み」を直接観測する基盤。
    hook_monitor: HookMonitor,
    notify_monitor: ImeNotifyMonitor,
    /// 決定1b項目5（セッション中の監視）: 外部からの書き込みで試行が無効化
    /// された回数を数え、上限超過でセッション全体を失敗にする。`&self`の
    /// メソッドから更新するため`Cell`。
    session_monitor: Cell<SessionMonitor>,
    /// 直近に観測した「外部からの書き込み」の累計件数のスナップショット
    /// （測定と測定の間で差分を取るための基準点）。
    external_baseline: Cell<u32>,
    /// [ADR195-T7](../../../../docs/tasks/adr195-t7-safety-measures.md)項目2:
    /// 直近に観測したユーザー物理入力（`LLKHF_INJECTED`無し）の累計件数の
    /// スナップショット（`external_baseline`と同じ基準点方式）。
    physical_baseline: Cell<u32>,
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

        // ADR-196決定1b: 学習窓への外部からの書き込みを直接観測する基盤を、
        // フォーカス移行より前に立ち上げる（以降の待ちすべてを観測できるように）。
        let hook_monitor = HookMonitor::install()?;
        let notify_monitor = ImeNotifyMonitor::new();

        pump_for(
            Duration::from_millis(focus_debounce_wait_ms()),
            &notify_monitor,
        );
        unsafe { SetWindowTextW(edit, w!(""))? };
        pump_for(Duration::from_millis(QUIET_MS), &notify_monitor);

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
            hook_monitor,
            notify_monitor,
            session_monitor: Cell::new(SessionMonitor::new(SESSION_INVALIDATION_LIMIT)),
            external_baseline: Cell::new(0),
            physical_baseline: Cell::new(0),
        };

        // 決定1b項目3・ADR195-T7項目2: 静かな観測窓（quiet window）——ここまでの
        // 待ちの後、注入を一切しない期間T msを置き、その間に外部からの書き込み・
        // ユーザーの物理入力・学習窓からのフォーカス喪失のいずれも観測されない
        // ことをセッション開始条件にする。ここで失敗すれば`driver`はこのまま
        // スコープを抜けてDropされ、窓・TSF・フックが片付く。
        let external_before = driver.external_total();
        let physical_before = driver.physical_total();
        driver.pump(Duration::from_millis(QUIET_WINDOW_MS));
        if trial_contaminated(
            driver.external_total() != external_before,
            driver.physical_total() != physical_before,
            !driver.focus_intact(),
        ) {
            return Err(windows::core::Error::new(
                windows::core::HRESULT(0x8000_4004u32.cast_signed()),
                "quiet window中に外部からの書き込み・物理入力・フォーカス喪失のいずれかを検出した(A'が崩れている疑い)",
            ));
        }

        driver.initial = driver.observe_imm()?.status;
        driver.external_baseline.set(driver.external_total());
        driver.physical_baseline.set(driver.physical_total());
        Ok(driver)
    }

    /// メッセージを回しながら待つ（`self.notify_monitor`に観測させる）。
    fn pump(&self, duration: Duration) {
        pump_for(duration, &self.notify_monitor);
    }

    /// 現在の「外部からの書き込み」累計件数（フック経由＋IME通知経由）。
    fn external_total(&self) -> u32 {
        self.hook_monitor.external_event_count() + self.notify_monitor.external_count()
    }

    /// [ADR195-T7](../../../../docs/tasks/adr195-t7-safety-measures.md)項目2:
    /// 現在のユーザー物理入力（`LLKHF_INJECTED`無し）の累計件数。
    fn physical_total(&self) -> u32 {
        self.hook_monitor.physical_event_count()
    }

    /// [ADR195-T7](../../../../docs/tasks/adr195-t7-safety-measures.md)項目2:
    /// フォーカスが学習窓の専用EDITコントロールに留まっているか。学習プロセスの
    /// 窓を作成したのと同じスレッドから呼ぶ前提（`AttachThreadInput`無しで
    /// `GetFocus`が有効）。
    fn focus_intact(&self) -> bool {
        (unsafe { GetFocus() }) == self.edit
    }

    /// 決定1b項目5（セッション中の監視）・ADR195-T7項目2: 前回チェック以降に
    /// 外部からの書き込み・ユーザーの物理入力が観測されたか、フォーカスが
    /// 学習窓から外れていたら、直前の試行を無効化としてセッション監視へ記録
    /// する。セッション全体を失敗にすべきなら`true`を返す。
    fn check_session_interference(&self) -> bool {
        let current_external = self.external_total();
        let external_baseline = self.external_baseline.replace(current_external);
        let current_physical = self.physical_total();
        let physical_baseline = self.physical_baseline.replace(current_physical);
        let contaminated = trial_contaminated(
            current_external != external_baseline,
            current_physical != physical_baseline,
            !self.focus_intact(),
        );
        if !contaminated {
            return false;
        }
        let mut monitor = self.session_monitor.get();
        let fail = monitor.record_invalidated_trial();
        self.session_monitor.set(monitor);
        fail
    }

    /// 決定1b項目4・項目2（生存確認）: フックとIME通知経路の両方が生きているか。
    /// `status_changed`は直近の自己注入で実際に開閉・変換モードが変わったかを
    /// 渡す（変わっていなければ通知が無くても判定できない）。
    #[must_use]
    pub fn observation_alive(&self, status_changed: bool) -> bool {
        self.hook_monitor.liveness().is_alive()
            && self
                .notify_monitor
                .is_alive_given_status_changed(status_changed)
    }

    /// 決定1b項目6（残余リスクの緩和）: 直近の自己注入1件に対して、
    /// 開閉・変換モードの通知が2回以上届いていたら、その試行を無効とみなす
    /// べきかを返す（向きの逆転の判定は未実装——`WM_IME_NOTIFY`はメッセージの
    /// 種別しか運ばないため、件数のみで判定する）。
    #[must_use]
    pub fn measurement_suspicious(&self) -> bool {
        is_measurement_suspicious(self.notify_monitor.notify_count_since_mark(), false)
    }

    /// これまでにセッション監視が記録した無効化件数。呼び出し側が上限超過を
    /// 検知したらセッションを失敗として終了し、表を書き出さない。
    #[must_use]
    pub fn session_invalidated_trials(&self) -> u32 {
        self.session_monitor.get().invalidated_trials()
    }

    /// フック・IME通知の生存確認用に、この後の自己注入で状態が変わったら
    /// 通知が届くはずだと申告する。
    fn mark_self_injection(&mut self, count: u32) {
        for _ in 0..count {
            self.hook_monitor.mark_self_injection_sent();
        }
        self.notify_monitor
            .mark_expected_notify(Duration::from_millis(NOTIFY_EXPECT_WINDOW_MS));
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

    /// キーを1件注入する。決定1b項目1・4: 送信直前に自分の注入として記録する
    /// （フック生存確認・IME通知の期待猶予の起点）。
    fn inject(&mut self, key: usize) -> bool {
        self.mark_self_injection(1);
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
            self.pump(Duration::from_millis(5));
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
        self.pump(Duration::from_millis(QUIET_MS));
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
        // 決定1b項目5（セッション中の監視）・ADR195-T7項目2: この測定の間に
        // 外部からの書き込み・ユーザーの物理入力・フォーカス喪失のいずれかが
        // 観測されていたら記録する。呼び出し側（`main.rs`/将来の`Executor`
        // 統合、ADR196-T2）は`session_invalidated_trials()`を見て、上限超過なら
        // セッションを失敗として終了すること。
        let _session_should_fail = self.check_session_interference();
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
        self.pump(Duration::from_millis(SETUP_GAP_MS));
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
                self.mark_self_injection(1);
                let _ = send_key_press(vk);
                self.pump(Duration::from_millis(SETUP_GAP_MS));
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
                // ADR-196決定1b項目1: 自分の注入だとADR196-T1の分類器が判定
                // できるよう、専用の目印を付ける（`0`のままだと「目印の無い
                // 注入」＝外部からの書き込みとして誤分類される）。
                dwExtraInfo: SELF_MARKER,
            },
        },
    };
    let cb_size = i32::try_from(size_of::<INPUT>()).expect("size_of::<INPUT>() fits in i32");
    unsafe { SendInput(&[make(false), make(true)], cb_size) == 2 }
}

/// ADR-195段階1 Major-1対応: 専用窓へフォーカスを移した後、最初の注入まで
/// `config.general.focus_debounce_ms`(既定50ms)+`FOCUS_MARGIN_MS`だけ待つ。
/// awase.exeの`config.toml`をexe隣・ワークスペースルートから探して読む
/// (`resolve_relative_to_exe`、`find_config_path`と同じ解決順)。読めない・
/// パースできない場合は保守的な既定値`FOCUS_DEBOUNCE_FALLBACK_MS`を使う
/// (config読み取り統合前の暫定値、ADR-195段階0参照)。
fn focus_debounce_wait_ms() -> u64 {
    let path = resolve_relative_to_exe("config.toml");
    let configured = AppConfig::load(&path)
        .ok()
        .map(|config| u64::from(config.general.focus_debounce_ms));
    configured.unwrap_or(FOCUS_DEBOUNCE_FALLBACK_MS) + FOCUS_MARGIN_MS
}

/// メッセージを回しながら待つ（ADR-196決定1b項目4: フックが黙って外れるのを
/// 防ぐため、待ちの間もメッセージポンプを回し続ける）。`notify_monitor`に
/// `WM_IME_NOTIFY`を観測させる（決定1b項目2）。
fn pump_for(duration: Duration, notify_monitor: &ImeNotifyMonitor) {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&raw mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                notify_monitor.observe_message(&msg);
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
