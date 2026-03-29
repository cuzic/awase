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
    Key(u16),
    /// 単一の仮想キーコードをリリース
    KeyUp(u16),
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
    pub fn from_u8(v: u8) -> Self {
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
