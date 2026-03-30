use windows::core::{Interface, GUID};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER};
use windows::Win32::UI::Input::Ime::{
    ImmGetCompositionStringW, ImmGetContext, ImmGetConversionStatus, ImmGetOpenStatus,
    ImmReleaseContext, GCS_COMPSTR, IME_CMODE_FULLSHAPE, IME_CMODE_KATAKANA, IME_CMODE_NATIVE,
    IME_CONVERSION_MODE, IME_SENTENCE_MODE,
};
use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyboardLayout;
use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
use windows::Win32::UI::TextServices::{
    CLSID_TF_ThreadMgr, ITfCompartment, ITfCompartmentMgr, ITfThreadMgr,
    GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION, GUID_COMPARTMENT_KEYBOARD_OPENCLOSE,
};

pub use awase::platform::ImeMode;

/// IME 状態検知のトレイト
pub trait ImeProvider {
    /// 現在の IME モードを取得する
    fn get_mode(&self) -> ImeMode;

    /// IME が有効（日本語入力可能な状態）かどうか
    fn is_active(&self) -> bool {
        let mode = self.get_mode();
        !matches!(mode, ImeMode::Off | ImeMode::Alphanumeric)
    }

    /// IME が未確定文字列を持っているか（変換中か）
    fn is_composing(&self) -> bool;
}

/// conversion モードビットマスクから `ImeMode` を判定する
const fn conversion_to_ime_mode(open: bool, conversion: u32) -> ImeMode {
    if !open {
        return ImeMode::Off;
    }

    if conversion & IME_CMODE_NATIVE.0 == 0 {
        return ImeMode::Alphanumeric;
    }

    if conversion & IME_CMODE_KATAKANA.0 != 0 {
        if conversion & IME_CMODE_FULLSHAPE.0 != 0 {
            ImeMode::Katakana
        } else {
            ImeMode::HalfKatakana
        }
    } else {
        ImeMode::Hiragana
    }
}

// ─── TSF (Text Services Framework) ───────────────────────────

/// TSF ベースの IME 状態検知
pub struct TsfProvider {
    thread_mgr: ITfThreadMgr,
}

impl TsfProvider {
    /// TSF を初期化する。失敗した場合は `None` を返す。
    pub fn try_new() -> Option<Self> {
        unsafe {
            // COM 初期化（既に初期化済みでも問題ない）
            let _ = CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);

            let thread_mgr: ITfThreadMgr =
                CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER).ok()?;

            log::info!("TSF provider initialized successfully");
            Some(Self { thread_mgr })
        }
    }

    /// Compartment の値を読み取る
    fn get_compartment_value(&self, guid: &GUID) -> Option<u32> {
        unsafe {
            let mgr: ITfCompartmentMgr = self.thread_mgr.cast().ok()?;
            let compartment: ITfCompartment = mgr.GetCompartment(guid).ok()?;
            let variant = compartment.GetValue().ok()?;
            // VARIANT から i32 を取り出し u32 にキャスト
            let raw = variant.as_raw();
            Some(raw.Anonymous.Anonymous.Anonymous.lVal.cast_unsigned())
        }
    }
}

impl ImeProvider for TsfProvider {
    fn get_mode(&self) -> ImeMode {
        let open = self
            .get_compartment_value(&GUID_COMPARTMENT_KEYBOARD_OPENCLOSE)
            .unwrap_or(0);
        let conversion = self
            .get_compartment_value(&GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION)
            .unwrap_or(0);

        let mode = conversion_to_ime_mode(open != 0, conversion);
        log::trace!("TSF: open={open} conversion=0x{conversion:08X} → {mode:?}");
        mode
    }

    fn is_composing(&self) -> bool {
        // TSF composition detection is complex (requires ITfContextComposition).
        // Fall back to false for now — HybridProvider will use ImmProvider as fallback.
        false
    }
}

// ─── IMM32 (Input Method Manager) ────────────────────────────

/// IMM32 ベースの IME 状態検知
pub struct ImmProvider;

impl ImmProvider {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl ImeProvider for ImmProvider {
    fn get_mode(&self) -> ImeMode {
        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd == HWND::default() {
                log::trace!("IMM: GetForegroundWindow returned NULL");
                return ImeMode::Off;
            }

            let himc = ImmGetContext(hwnd);
            if himc.is_invalid() {
                log::trace!("IMM: ImmGetContext({hwnd:?}) returned invalid");
                return ImeMode::Off;
            }

            let mut conversion = IME_CONVERSION_MODE::default();
            let mut sentence = IME_SENTENCE_MODE::default();
            let ok =
                ImmGetConversionStatus(himc, Some(&raw mut conversion), Some(&raw mut sentence));
            let _ = ImmReleaseContext(hwnd, himc);

            if !ok.as_bool() {
                log::trace!("IMM: ImmGetConversionStatus failed for hwnd={hwnd:?}");
                return ImeMode::Off;
            }

            let native = conversion.0 & IME_CMODE_NATIVE.0 != 0;
            let mode = conversion_to_ime_mode(native, conversion.0);
            log::trace!(
                "IMM: hwnd={hwnd:?} conversion=0x{:08X} native={native} → {mode:?}",
                conversion.0,
            );
            mode
        }
    }

    fn is_composing(&self) -> bool {
        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd == HWND::default() {
                return false;
            }
            let himc = ImmGetContext(hwnd);
            if himc.is_invalid() {
                return false;
            }
            let len = ImmGetCompositionStringW(himc, GCS_COMPSTR, None, 0);
            let _ = ImmReleaseContext(hwnd, himc);
            len > 0
        }
    }
}

// ─── 複合プロバイダ（TSF 優先、IMM32 フォールバック）────────

/// TSF を優先し、失敗時に IMM32 にフォールバックするプロバイダ
pub struct HybridProvider {
    tsf: Option<TsfProvider>,
    imm: ImmProvider,
}

impl HybridProvider {
    /// TSF の初期化を試み、成否に関わらず IMM32 もフォールバックとして保持する
    pub fn new() -> Self {
        let tsf = TsfProvider::try_new();
        if tsf.is_none() {
            log::info!("TSF initialization failed, using IMM32 only");
        }
        Self {
            tsf,
            imm: ImmProvider::new(),
        }
    }
}

impl ImeProvider for HybridProvider {
    fn get_mode(&self) -> ImeMode {
        // All methods for comparison logging
        let tsf_mode = self.tsf.as_ref().map(ImeProvider::get_mode);
        let imm_mode = self.imm.get_mode();

        // Keyboard layout (HKL) as additional signal
        let hkl = unsafe { GetKeyboardLayout(0) };
        let lang_id = hkl.0 as u32 & 0xFFFF;
        let is_japanese_hkl = lang_id == 0x0411;

        // ImmGetOpenStatus as yet another signal
        let imm_open = unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd != HWND::default() {
                let himc = ImmGetContext(hwnd);
                if !himc.is_invalid() {
                    let open = ImmGetOpenStatus(himc);
                    let _ = ImmReleaseContext(hwnd, himc);
                    Some(open.as_bool())
                } else {
                    None
                }
            } else {
                None
            }
        };

        log::trace!(
            "HybridIME: TSF={tsf_mode:?} IMM={imm_mode:?} ImmOpenStatus={imm_open:?} HKL=0x{lang_id:04X} japanese={is_japanese_hkl}",
        );

        // Decision: TSF first, then IMM fallback
        let result = if let Some(tsf) = tsf_mode {
            if tsf != ImeMode::Off {
                tsf
            } else {
                // TSF says Off — check IMM as fallback
                imm_mode
            }
        } else {
            imm_mode
        };

        // Additional fallback: if both say Off but ImmOpenStatus is true,
        // the IME is likely active but in a state we can't detect well.
        // Log this discrepancy for debugging.
        if result == ImeMode::Off && imm_open == Some(true) {
            log::debug!(
                "HybridIME: TSF/IMM say Off but ImmOpenStatus=true — possible detection gap"
            );
        }

        log::trace!("HybridIME: final result={result:?}");
        result
    }

    fn is_composing(&self) -> bool {
        let result = self.imm.is_composing();
        log::trace!("HybridIME: is_composing={result}");
        result
    }
}

/// 現在のキーボードレイアウトが日本語かどうかを判定する
#[must_use]
pub fn is_japanese_input_language() -> bool {
    unsafe {
        let hkl = GetKeyboardLayout(0);
        // 下位 16 bit が言語 ID。日本語は 0x0411
        let lang_id = hkl.0 as u32 & 0xFFFF;
        lang_id == 0x0411
    }
}
