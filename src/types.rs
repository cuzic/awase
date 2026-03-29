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
    pub vk_code: u16,
    pub scan_code: u32,
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
