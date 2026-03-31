//! macOS IME 検出 (TISCopyCurrentKeyboardInputSource)
//!
//! 将来的に TISCopyCurrentKeyboardInputSource + kTISPropertyInputSourceID で
//! 現在の入力ソースを取得し、IME ON/OFF を判定する。

/// macOS IME 検出（スタブ実装）
#[derive(Debug)]
pub struct ImeDetector;

impl ImeDetector {
    pub fn new() -> Self {
        log::info!("IME detector: stub (TISCopyCurrentKeyboardInputSource not yet implemented)");
        Self
    }

    /// 現在の IME 状態を問い合わせる
    /// - Some(true): IME ON (ひらがなモード等)
    /// - Some(false): IME OFF (英数モード)
    /// - None: 検出不可
    pub fn is_ime_on(&self) -> Option<bool> {
        // TODO: TISCopyCurrentKeyboardInputSource で InputSourceID を取得
        // *.HiraganaInputMode → true
        // com.apple.keylayout.* → false
        None
    }

    /// 日本語キーボードレイアウトが有効かどうか
    pub fn is_japanese_layout(&self) -> bool {
        // TODO: TISCopyCurrentKeyboardLayoutInputSource で確認
        true // デフォルトは true（安全側）
    }
}

impl Default for ImeDetector {
    fn default() -> Self {
        Self::new()
    }
}
