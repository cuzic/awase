//! macOS IME 検出 (TISCopyCurrentKeyboardInputSource)
//!
//! `kTISPropertyInputSourceID` から現在の入力ソース ID を取得し、
//! 日本語 IME のかな入力モードかどうかを判定する。
//!
//! 代表的な InputSourceID:
//! - `com.apple.inputmethod.Kotoeri.RomajiTyping.Japanese` — 日本語IM ひらがな
//! - `com.apple.inputmethod.Kotoeri.RomajiTyping.Roman` — 日本語IM 英字
//! - `com.google.inputmethod.Japanese.base` — Google 日本語入力 ひらがな
//! - `com.google.inputmethod.Japanese.Roman` — Google 日本語入力 英数
//! - `com.apple.keylayout.ABC` — 英語キーボードレイアウト（IME なし）

#[cfg(target_os = "macos")]
mod imp {
    // TIS (Text Input Sources) API の呼び出しに必要
    #![allow(unsafe_code)]

    use core_foundation::base::{CFRelease, TCFType};
    use core_foundation::string::{CFString, CFStringRef};
    use std::ffi::c_void;

    #[link(name = "Carbon", kind = "framework")]
    extern "C" {
        fn TISCopyCurrentKeyboardInputSource() -> *mut c_void;
        fn TISGetInputSourceProperty(source: *mut c_void, key: CFStringRef) -> *mut c_void;
        static kTISPropertyInputSourceID: CFStringRef;
    }

    /// 現在の入力ソース ID を取得する。
    fn current_input_source_id() -> Option<String> {
        unsafe {
            let source = TISCopyCurrentKeyboardInputSource();
            if source.is_null() {
                return None;
            }
            let id_ref = TISGetInputSourceProperty(source, kTISPropertyInputSourceID);
            let id = if id_ref.is_null() {
                None
            } else {
                // Get ルール: プロパティは input source が所有する
                Some(CFString::wrap_under_get_rule(id_ref.cast()).to_string())
            };
            // Copy ルール: input source は呼び出し側が解放する
            CFRelease(source.cast_const());
            id
        }
    }

    /// macOS IME 検出。
    #[derive(Debug)]
    pub struct ImeDetector;

    impl ImeDetector {
        #[must_use]
        pub fn new() -> Self {
            log::info!("IME detector: TISCopyCurrentKeyboardInputSource");
            Self
        }

        /// 現在の IME 状態を問い合わせる
        /// - Some(true): IME ON (ひらがな・カタカナモード等)
        /// - Some(false): IME OFF (英数モード・IME なしレイアウト)
        /// - None: 検出不可
        #[must_use]
        pub fn is_ime_on(&self) -> Option<bool> {
            let id = current_input_source_id()?;
            if id.contains("inputmethod") && id.contains("Japanese") {
                // "…Japanese" / "…Japanese.Katakana" は ON、
                // "…Roman" / "…FullWidthRoman"（英字モード）は OFF
                Some(!id.contains("Roman"))
            } else if id.contains("keylayout") {
                Some(false)
            } else {
                None
            }
        }

        /// 日本語 IME がアクティブかどうか（英数モードも含む）
        #[must_use]
        pub fn is_japanese_layout(&self) -> bool {
            current_input_source_id()
                .is_none_or(|id| id.contains("Japanese") || id.contains("Kotoeri"))
        }
    }

    impl Default for ImeDetector {
        fn default() -> Self {
            Self::new()
        }
    }
}

#[cfg(target_os = "macos")]
pub use imp::ImeDetector;

/// 非 macOS ビルド用スタブ。
#[cfg(not(target_os = "macos"))]
#[derive(Debug)]
pub struct ImeDetector;

#[cfg(not(target_os = "macos"))]
impl ImeDetector {
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    #[must_use]
    pub fn is_ime_on(&self) -> Option<bool> {
        None
    }

    #[must_use]
    pub fn is_japanese_layout(&self) -> bool {
        true
    }
}

#[cfg(not(target_os = "macos"))]
impl Default for ImeDetector {
    fn default() -> Self {
        Self::new()
    }
}
