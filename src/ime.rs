use windows::core::{Interface, GUID};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER};
use windows::Win32::UI::Input::Ime::{
    ImmGetContext, ImmGetConversionStatus, ImmReleaseContext, IME_CMODE_FULLSHAPE,
    IME_CMODE_KATAKANA, IME_CMODE_NATIVE, IME_CONVERSION_MODE, IME_SENTENCE_MODE,
};
use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyboardLayout;
use windows::Win32::UI::TextServices::{
    CLSID_TF_ThreadMgr, ITfCompartment, ITfCompartmentMgr, ITfThreadMgr,
    GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION, GUID_COMPARTMENT_KEYBOARD_OPENCLOSE,
};
use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

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

        conversion_to_ime_mode(open != 0, conversion)
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
                return ImeMode::Off;
            }

            let himc = ImmGetContext(hwnd);
            if himc.is_invalid() {
                return ImeMode::Off;
            }

            let mut conversion = IME_CONVERSION_MODE::default();
            let mut sentence = IME_SENTENCE_MODE::default();
            let ok =
                ImmGetConversionStatus(himc, Some(&raw mut conversion), Some(&raw mut sentence));
            let _ = ImmReleaseContext(hwnd, himc);

            if !ok.as_bool() {
                return ImeMode::Off;
            }

            conversion_to_ime_mode(conversion.0 & IME_CMODE_NATIVE.0 != 0, conversion.0)
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
        // TSF が利用可能ならまず TSF で取得を試みる
        if let Some(ref tsf) = self.tsf {
            let mode = tsf.get_mode();
            // TSF が有効な結果を返したらそれを使う
            if mode != ImeMode::Off {
                return mode;
            }
            // TSF が Off を返した場合、IMM32 でも確認する
            // （一部アプリで TSF が正しく動作しないケースへの対応）
        }

        self.imm.get_mode()
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
