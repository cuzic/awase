/// マイクロ秒精度のタイムスタンプ（テスト容易性のため `Instant` を置換）
pub type Timestamp = u64;

/// スキャンコード（物理キー位置の識別子）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScanCode(pub u32);

impl From<u32> for ScanCode {
    fn from(v: u32) -> Self {
        Self(v)
    }
}
impl From<ScanCode> for u32 {
    fn from(v: ScanCode) -> Self {
        v.0
    }
}

/// 仮想キーコード（OS キーボードレイアウト依存の論理キー）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VkCode(pub u16);

impl From<u16> for VkCode {
    fn from(v: u16) -> Self {
        Self(v)
    }
}
impl From<VkCode> for u16 {
    fn from(v: VkCode) -> Self {
        v.0
    }
}

/// フックから受け取る生のキーイベント
#[derive(Debug, Clone, Copy)]
pub struct RawKeyEvent {
    pub vk_code: VkCode,
    pub scan_code: ScanCode,
    pub event_type: KeyEventType,
    pub extra_info: usize,
    pub timestamp: Timestamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyEventType {
    KeyDown,
    KeyUp,
    SysKeyDown,
    SysKeyUp,
}

/// 出力アクション
#[derive(Debug, Clone)]
pub enum KeyAction {
    /// 単一の仮想キーコードを押下
    Key(VkCode),
    /// 単一の仮想キーコードをリリース
    KeyUp(VkCode),
    /// Unicode 文字を直接出力（`SendInput` の `KEYEVENTF_UNICODE`）
    Char(char),
    /// 何もしない（キーを握りつぶす）
    Suppress,
    /// ローマ字文字列を VK コードのキーイベントとして送信（IME ローマ字入力モード用）
    Romaji(String),
}

/// コンテキスト無効化の理由（ログ・デバッグ用）
#[derive(Debug, Clone, Copy)]
pub enum ContextChange {
    /// IME がオフになった
    ImeOff,
    /// 入力言語が変更された（Win+Space 等）
    InputLanguageChanged,
    /// エンジンが無効化された（ホットキー等）
    EngineDisabled,
    /// レイアウトが差し替えられた
    LayoutSwapped,
    /// フォーカスが別のコントロールに移動した
    FocusChanged,
}

/// 外部プロセスの IME 状態をクロスプロセス API で正確に取得できるかの信頼度
///
/// `ImmGetDefaultIMEWnd` + `WM_IME_CONTROL / IMC_GETOPENSTATUS` は Win32 アプリでは
/// 正確に動作するが、WinUI 3 / XAML Islands 等の Modern UI では互換レイヤー経由のため
/// 実際の TSF IME 状態を反映しないことがある。
/// UIA `FrameworkId` と IMM コンテキスト有無から推定する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ImeReliability {
    /// クラシック Win32 / WinForms — クロスプロセス IME 検出を信頼できる
    Reliable = 0,
    /// Modern UI (DirectUI, XAML, WinUI 等) — クロスプロセス IME 検出が不正確な可能性
    Unreliable = 1,
    /// 未判定（UIA 非同期結果待ち）
    Unknown = 2,
}

impl ImeReliability {
    #[must_use]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Reliable,
            1 => Self::Unreliable,
            _ => Self::Unknown,
        }
    }

    pub fn load(atomic: &std::sync::atomic::AtomicU8) -> Self {
        Self::from_u8(atomic.load(std::sync::atomic::Ordering::Acquire))
    }

    pub fn store(self, atomic: &std::sync::atomic::AtomicU8) {
        atomic.store(self as u8, std::sync::atomic::Ordering::Release);
    }
}

/// IME 状態キャッシュ値（`IME_STATE_CACHE: AtomicU8` との相互変換用）
///
/// メッセージループで `refresh_ime_state_cache()` が書き込み、フックで読み取る。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ImeCacheState {
    /// IME OFF
    Off = 0,
    /// IME ON
    On = 1,
    /// 未判定（初期状態 or キャッシュ未更新）
    Unknown = 2,
}

impl ImeCacheState {
    #[must_use]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Off,
            1 => Self::On,
            _ => Self::Unknown,
        }
    }

    pub fn load(atomic: &std::sync::atomic::AtomicU8) -> Self {
        Self::from_u8(atomic.load(std::sync::atomic::Ordering::Acquire))
    }

    pub fn store(self, atomic: &std::sync::atomic::AtomicU8) {
        atomic.store(self as u8, std::sync::atomic::Ordering::Release);
    }

    /// `AtomicU8` に対してアトミック swap を行い、以前の値を返す
    #[must_use]
    pub fn swap(self, atomic: &std::sync::atomic::AtomicU8) -> Self {
        Self::from_u8(atomic.swap(self as u8, std::sync::atomic::Ordering::AcqRel))
    }

    /// Unknown の場合は shadow state にフォールバック
    #[must_use]
    pub const fn resolve_with_shadow(self, shadow_ime_on: bool) -> bool {
        match self {
            Self::On => true,
            Self::Off => false,
            Self::Unknown => shadow_ime_on,
        }
    }

    /// 表示用ラベル
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "OFF",
            Self::On => "ON",
            Self::Unknown => "Unknown",
        }
    }
}

impl From<bool> for ImeCacheState {
    fn from(ime_on: bool) -> Self {
        if ime_on {
            Self::On
        } else {
            Self::Off
        }
    }
}

/// フォーカス中コントロールの種別
///
/// テキスト入力を受け付けるかどうかを示す。
/// `AtomicU8` で共有するため `repr(u8)` を使用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FocusKind {
    /// テキスト入力コントロール（エンジン処理を許可）
    TextInput = 0,
    /// 非テキストコントロール（エンジンをバイパス）
    NonText = 1,
    /// 判定不能
    Undetermined = 2,
}

impl FocusKind {
    /// u8 から FocusKind に変換する。0→TextInput, 1→NonText, その他→Undetermined。
    /// AtomicU8 との相互変換に使用。
    #[must_use]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::TextInput,
            1 => Self::NonText,
            _ => Self::Undetermined,
        }
    }

    pub fn load(atomic: &std::sync::atomic::AtomicU8) -> Self {
        Self::from_u8(atomic.load(std::sync::atomic::Ordering::Acquire))
    }

    pub fn store(self, atomic: &std::sync::atomic::AtomicU8) {
        atomic.store(self as u8, std::sync::atomic::Ordering::Release);
    }
}
