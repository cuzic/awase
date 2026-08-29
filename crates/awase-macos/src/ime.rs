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

    use core_foundation::array::{CFArrayGetCount, CFArrayGetValueAtIndex, CFArrayRef};
    use core_foundation::base::{CFRelease, TCFType};
    use core_foundation::dictionary::CFDictionaryRef;
    use core_foundation::string::{CFString, CFStringRef};
    use std::ffi::c_void;
    use std::ptr;

    #[link(name = "Carbon", kind = "framework")]
    extern "C" {
        fn TISCopyCurrentKeyboardInputSource() -> *mut c_void;
        fn TISGetInputSourceProperty(source: *mut c_void, key: CFStringRef) -> *mut c_void;
        fn TISCreateInputSourceList(
            properties: CFDictionaryRef,
            include_all_installed: bool,
        ) -> CFArrayRef;
        fn TISSelectInputSource(source: *mut c_void) -> i32;
        static kTISPropertyInputSourceID: CFStringRef;
    }

    /// input source ポインタから InputSourceID を取り出す（Get ルール）。
    unsafe fn input_source_id(source: *mut c_void) -> Option<String> {
        let id_ref = TISGetInputSourceProperty(source, kTISPropertyInputSourceID);
        if id_ref.is_null() {
            None
        } else {
            Some(CFString::wrap_under_get_rule(id_ref.cast()).to_string())
        }
    }

    /// 現在の入力ソース ID を取得する。
    fn current_input_source_id() -> Option<String> {
        unsafe {
            let source = TISCopyCurrentKeyboardInputSource();
            if source.is_null() {
                return None;
            }
            let id = input_source_id(source);
            // Copy ルール: input source は呼び出し側が解放する
            CFRelease(source.cast_const());
            id
        }
    }

    /// ID がひらがな系の日本語 IME モードかどうか（ON 側の選択対象）。
    fn is_japanese_kana_mode(id: &str) -> bool {
        id.contains("inputmethod")
            && id.contains("Japanese")
            && !id.contains("Roman")
            && !id.contains("Katakana")
    }

    /// ID が英数側の選択対象かどうか。
    ///
    /// 日本語 IM の英字モード（`…Roman`）を優先し、無ければ ABC 等の
    /// keylayout へフォールバックする（2 段階で呼び分ける）。
    fn is_japanese_roman_mode(id: &str) -> bool {
        id.contains("inputmethod") && id.contains("Japanese") && id.contains("Roman")
            && !id.contains("FullWidth")
    }

    /// 有効な入力ソースから述語に合う最初のものを選択する。
    fn select_input_source_matching(pred: impl Fn(&str) -> bool) -> bool {
        unsafe {
            let list = TISCreateInputSourceList(ptr::null(), false);
            if list.is_null() {
                return false;
            }
            let mut selected = false;
            let count = CFArrayGetCount(list.cast());
            for i in 0..count {
                let source = CFArrayGetValueAtIndex(list.cast(), i).cast_mut();
                if source.is_null() {
                    continue;
                }
                if input_source_id(source).is_some_and(|id| pred(&id)) {
                    selected = TISSelectInputSource(source) == 0;
                    if selected {
                        break;
                    }
                }
            }
            CFRelease(list.cast());
            selected
        }
    }

    /// macOS IME 検出。
    ///
    /// 最後に観測した日本語 IME（かなモード）の入力ソース ID を記憶し、
    /// `set_ime_on` での復元先として使う。単一 thread（main の
    /// CFRunLoop）でのみ使う前提で `RefCell` を持つ。
    #[derive(Debug)]
    pub struct ImeDetector {
        last_japanese_id: std::cell::RefCell<Option<String>>,
    }

    impl ImeDetector {
        #[must_use]
        pub fn new() -> Self {
            log::info!("IME detector: TISCopyCurrentKeyboardInputSource");
            let detector = Self {
                last_japanese_id: std::cell::RefCell::new(None),
            };
            // 起動時点の入力ソースを観測しておく（最初の打鍵前に
            // set_ime_on(true) が呼ばれても復元先が分かるように）
            let _ = detector.is_ime_on();
            detector
        }

        /// 現在の IME 状態を問い合わせる
        /// - Some(true): IME ON (ひらがな・カタカナモード等)
        /// - Some(false): IME OFF (英数モード・IME なしレイアウト)
        /// - None: 検出不可
        #[must_use]
        pub fn is_ime_on(&self) -> Option<bool> {
            let id = current_input_source_id()?;
            if is_japanese_kana_mode(&id) {
                // ユーザーが実際に使っている日本語 IME を記憶する
                // （ATOK / Google 日本語入力 / 日本語IM の区別を保つため。
                // 述語ベースの選択だとリスト先頭の OS 標準 IME に化ける）
                let mut last = self.last_japanese_id.borrow_mut();
                if last.as_deref() != Some(&id) {
                    log::debug!("IME observed: {id}");
                    *last = Some(id.clone());
                }
            }
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

        /// IME の ON/OFF を強制する（`ImeEffect::SetOpen` の実装）。
        ///
        /// - ON: 最後に使っていた日本語 IME のかなモードを復元。未観測なら
        ///   述語マッチにフォールバック
        /// - OFF: 同じ IM ファミリの英字モード（`….Roman` / `…Eiji`）を優先し、
        ///   無ければ任意の日本語 IM 英字モード → keylayout（ABC 等）
        ///
        /// 対象の入力ソースが見つからない/選択に失敗した場合は false。
        pub fn set_ime_on(&self, open: bool) -> bool {
            let last = self.last_japanese_id.borrow().clone();
            if open {
                if let Some(ref id) = last {
                    if select_input_source_matching(|c| c == id) {
                        return true;
                    }
                    log::warn!("IME restore failed for {id}, falling back to predicate match");
                }
                select_input_source_matching(is_japanese_kana_mode)
            } else {
                // 同じ IM ファミリ（ID の末尾セグメントを除いた prefix）の
                // 英字モードを優先する。例:
                //   com.justsystems.inputmethod.atok34.Japanese → …atok34.Roman
                //   com.google.inputmethod.Japanese.base       → ….Japanese.Roman
                if let Some(prefix) = last.as_deref().and_then(|id| id.rsplit_once('.')) {
                    let prefix = prefix.0;
                    if select_input_source_matching(|c| {
                        c.starts_with(prefix)
                            && (c.contains("Roman") || c.contains("Eiji"))
                            && !c.contains("FullWidth")
                    }) {
                        return true;
                    }
                }
                select_input_source_matching(is_japanese_roman_mode)
                    || select_input_source_matching(|id| id.contains("keylayout"))
            }
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

    pub fn set_ime_on(&self, _open: bool) -> bool {
        false
    }
}

#[cfg(not(target_os = "macos"))]
impl Default for ImeDetector {
    fn default() -> Self {
        Self::new()
    }
}
