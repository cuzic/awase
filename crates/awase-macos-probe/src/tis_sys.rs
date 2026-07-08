//! Carbon framework の TIS 関数群に対する自前 unsafe extern "C" FFI バインディング
//! （objc2 系クレートには TIS の bindings が無いため）。所有権の注意点は
//! project_macos_probe_interfaces.md の「Rust依存クレート」節末尾を参照。
//!
//! 所有権の規律（このモジュールの正しさの要）:
//! - `TISCopy...` / `TISCreate...` の戻り値は **呼び出し側所有**（Drop で CFRelease）。
//! - `TISGetInputSourceProperty` の戻り値は **借用**（対象 input source が所有。release しない）。
//! - `kTISProperty*` / `kTISCategory*` は **グローバル定数**（release しない）。
//!
//! 生ポインタは公開しない。`unsafe` FFI をこのモジュールに閉じ込め、上位には
//! `InputSourceHandle` と安全な getter だけを見せる。

// Carbon framework の TIS 関数呼び出し・CF 所有権操作に unsafe が必須。
#![allow(unsafe_code)]

// select_input_source: 入力ソース切替 CLI が無いためまだ未使用（#[allow(dead_code)]
// を関数定義側に付けても re-export の unused_imports は別扱いのためここにも要る）。
#[allow(unused_imports)]
#[cfg(target_os = "macos")]
pub use imp::{
    copy_current_input_source, create_input_source_list,
    enabled_input_sources_changed_notification_name, select_input_source,
    selected_input_source_changed_notification_name, InputSourceHandle,
};

#[allow(unused_imports)]
#[cfg(not(target_os = "macos"))]
pub use imp::{
    copy_current_input_source, create_input_source_list,
    enabled_input_sources_changed_notification_name, select_input_source,
    selected_input_source_changed_notification_name, InputSourceHandle,
};

#[cfg(target_os = "macos")]
mod imp {
    use core::ffi::c_void;
    use core::ptr::NonNull;

    use objc2_core_foundation::{CFArray, CFBoolean, CFData, CFRetained, CFString, CFType};

    /// 所有 `CFTypeRef`（`TISInputSourceRef` 等）を表す生ポインタ型。
    type CFTypeRef = *mut c_void;
    /// 借用／定数の `CFTypeRef`（property key 等）を表す生ポインタ型。
    type ConstCFTypeRef = *const c_void;

    #[link(name = "Carbon", kind = "framework")]
    // dead_code: この extern ブロックは Carbon TIS API 全体の宣言を1箇所にまとめる。
    // 個々の関数/定数を使う安全ラッパーが揃うまで、未使用の宣言があっても構わない
    // （例: TISSelectInputSource は select_input_source() 経由、まだ CLI から未配線）。
    #[allow(non_upper_case_globals, non_snake_case, dead_code)]
    extern "C" {
        /// 現在選択中のキーボード入力ソースを **所有付き**（+1）で返す。
        fn TISCopyCurrentKeyboardInputSource() -> CFTypeRef;
        /// 条件に一致する入力ソース一覧を **所有付き**（+1）の CFArray で返す。
        /// 要素（各 `TISInputSourceRef`）は配列が所有する借用。
        fn TISCreateInputSourceList(
            properties: ConstCFTypeRef,
            include_all_installed: u8,
        ) -> CFTypeRef;
        /// 入力ソースの property を **借用**で返す（release してはならない）。
        fn TISGetInputSourceProperty(
            input_source: CFTypeRef,
            property_key: ConstCFTypeRef,
        ) -> CFTypeRef;
        /// 入力ソースを選択する。戻り値は `OSStatus`（0 = 成功）。
        fn TISSelectInputSource(input_source: CFTypeRef) -> i32;

        static kTISPropertyInputSourceID: ConstCFTypeRef;
        static kTISPropertyLocalizedName: ConstCFTypeRef;
        static kTISPropertyInputSourceLanguages: ConstCFTypeRef;
        static kTISPropertyInputSourceIsASCIICapable: ConstCFTypeRef;
        static kTISPropertyInputSourceCategory: ConstCFTypeRef;
        static kTISCategoryKeyboardInputSource: ConstCFTypeRef;
        static kTISPropertyInputSourceIsEnabled: ConstCFTypeRef;
        static kTISPropertyInputSourceIsSelectCapable: ConstCFTypeRef;
        static kTISPropertyBundleID: ConstCFTypeRef;
        static kTISPropertyInputModeID: ConstCFTypeRef;
        /// `CFDataRef` に直列化された `UCKeyboardLayout`（`UCKeyTranslate` 用）。
        /// キーボード（'uchr'）ソースにのみ存在し、IME ソースでは NULL のことがある。
        static kTISPropertyUnicodeKeyLayoutData: ConstCFTypeRef;
        /// 入力ソースの種別（例: "TISTypeKeyboardInputMode" / "TISTypeKeyboardLayout"）。
        static kTISPropertyInputSourceType: ConstCFTypeRef;
        /// 現在選択中キーボード入力ソース変更の distributed notification 名。
        static kTISNotifySelectedKeyboardInputSourceChanged: ConstCFTypeRef;
        /// 有効な入力ソース一覧変更の distributed notification 名。
        static kTISNotifyEnabledKeyboardInputSourcesChanged: ConstCFTypeRef;
    }

    /// 1 つの `TISInputSourceRef` を所有し、Drop 時に CFRelease する安全ハンドル。
    pub struct InputSourceHandle {
        inner: CFRetained<CFType>,
    }

    impl core::fmt::Debug for InputSourceHandle {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.debug_struct("InputSourceHandle")
                .field("source_id", &self.source_id())
                .finish()
        }
    }

    /// 所有付き（+1）の生 `CFTypeRef` を `InputSourceHandle` に格納する。
    /// `NULL` の場合は `None`。CFRelease は `CFRetained` の Drop に委ねる。
    fn adopt_owned(ptr: CFTypeRef) -> Option<InputSourceHandle> {
        let nn = NonNull::new(ptr.cast::<CFType>())?;
        // SAFETY: ptr は TISCopy.../TISCreate... が返した所有付き(+1)の CFTypeRef。
        // from_raw はこの +1 を消費して所有権を移すため二重解放にならない。
        let inner = unsafe { CFRetained::from_raw(nn) };
        Some(InputSourceHandle { inner })
    }

    /// 借用の生 `CFTypeRef` を +1 retain して所有ハンドルにする（配列要素用）。
    fn retain_borrowed(ptr: ConstCFTypeRef) -> Option<InputSourceHandle> {
        let nn = NonNull::new(ptr.cast::<CFType>().cast_mut())?;
        // SAFETY: nn は配列が所有する生存中の TISInputSourceRef を指す。retain で +1 し、
        // 以後この所有権は CFRetained の Drop が CFRelease で解放する。
        let inner = unsafe { CFRetained::retain(nn) };
        Some(InputSourceHandle { inner })
    }

    /// 現在選択中のキーボード入力ソースを取得する。
    #[must_use]
    pub fn copy_current_input_source() -> Option<InputSourceHandle> {
        // SAFETY: 引数なしの Carbon 関数呼び出し。戻り値は所有付き CFTypeRef。
        let ptr = unsafe { TISCopyCurrentKeyboardInputSource() };
        adopt_owned(ptr)
    }

    /// 入力ソース一覧を列挙する。`include_all_installed` が真なら未選択のものも含む。
    #[must_use]
    pub fn create_input_source_list(include_all_installed: bool) -> Vec<InputSourceHandle> {
        // SAFETY: properties=NULL は「フィルタなし全件」を意味する妥当な引数。
        // 戻り値は所有付きの CFArrayRef。
        let arr_ptr =
            unsafe { TISCreateInputSourceList(core::ptr::null(), u8::from(include_all_installed)) };
        let Some(arr_owned) = adopt_owned(arr_ptr) else {
            return Vec::new();
        };
        let Some(arr) = arr_owned.inner.downcast_ref::<CFArray>() else {
            return Vec::new();
        };

        let count = arr.count();
        let mut out = Vec::with_capacity(usize::try_from(count).unwrap_or(0));
        for i in 0..count {
            // SAFETY: i は 0..count の範囲内。value_at_index は借用要素を返す。
            let elem = unsafe { arr.value_at_index(i) };
            if let Some(handle) = retain_borrowed(elem) {
                out.push(handle);
            }
        }
        out
    }

    /// 入力ソースを選択する。失敗時は `OSStatus` を `Err` で返す。
    ///
    /// 現時点では入力ソースを切り替える CLI サブコマンドが無いため未使用（列挙・観測
    /// のみが Phase M0 のスコープ）。将来の切替コマンド追加時に配線する。
    ///
    /// # Errors
    /// `TISSelectInputSource` が 0 以外の `OSStatus` を返した場合。
    #[allow(dead_code)]
    pub fn select_input_source(handle: &InputSourceHandle) -> Result<(), i32> {
        // SAFETY: handle.raw() は生存中の所有 TISInputSourceRef。
        let status = unsafe { TISSelectInputSource(handle.raw()) };
        if status == 0 {
            Ok(())
        } else {
            Err(status)
        }
    }

    /// グローバル定数の `CFStringRef`（借用）を Rust `String` に複製する。
    fn const_cfstring_to_string(ptr: ConstCFTypeRef) -> Option<String> {
        let nn = NonNull::new(ptr.cast::<CFType>().cast_mut())?;
        // SAFETY: ptr は Carbon が提供するグローバル定数の CFStringRef。常に生存し、
        // 借用として文字列を読み取るだけ（release しない）。
        let cf = unsafe { nn.as_ref() };
        let s = cf.downcast_ref::<CFString>()?;
        Some(s.to_string())
    }

    /// 「現在選択中のキーボード入力ソース変更」distributed notification 名。
    #[must_use]
    pub fn selected_input_source_changed_notification_name() -> Option<String> {
        // SAFETY: extern static は Carbon が提供する CFStringRef グローバル定数。
        const_cfstring_to_string(unsafe { kTISNotifySelectedKeyboardInputSourceChanged })
    }

    /// 「有効な入力ソース一覧変更」distributed notification 名。
    #[must_use]
    pub fn enabled_input_sources_changed_notification_name() -> Option<String> {
        // SAFETY: extern static は Carbon が提供する CFStringRef グローバル定数。
        const_cfstring_to_string(unsafe { kTISNotifyEnabledKeyboardInputSourcesChanged })
    }

    impl InputSourceHandle {
        /// 所有する `TISInputSourceRef` を生ポインタとして取り出す（呼び出しに渡すだけ）。
        fn raw(&self) -> CFTypeRef {
            CFRetained::as_ptr(&self.inner).as_ptr().cast::<c_void>()
        }

        /// property を **借用**の `&CFType` として取得する（release しない）。
        /// 戻り値の生存期間は `&self`（対象 input source）に束縛される。
        fn property(&self, key: ConstCFTypeRef) -> Option<&CFType> {
            // SAFETY: raw() は生存中の所有 TISInputSourceRef、key はグローバルな
            // CFStringRef property key。戻り値は input source が所有する借用値。
            let val = unsafe { TISGetInputSourceProperty(self.raw(), key) };
            let nn = NonNull::new(val.cast::<CFType>())?;
            // SAFETY: 借用値は input source（= self）が生きている間有効。release しない。
            Some(unsafe { nn.as_ref() })
        }

        fn property_string(&self, key: ConstCFTypeRef) -> Option<String> {
            let s = self.property(key)?.downcast_ref::<CFString>()?;
            Some(s.to_string())
        }

        fn property_bool(&self, key: ConstCFTypeRef) -> Option<bool> {
            let b = self.property(key)?.downcast_ref::<CFBoolean>()?;
            Some(b.value())
        }

        /// 入力ソース ID（例: `com.apple.inputmethod.Kotoeri.Japanese`）。
        #[must_use]
        pub fn source_id(&self) -> Option<String> {
            // SAFETY: extern static は Carbon が提供する CFStringRef グローバル定数。
            self.property_string(unsafe { kTISPropertyInputSourceID })
        }

        /// ローカライズされた表示名。
        #[must_use]
        pub fn localized_name(&self) -> Option<String> {
            // SAFETY: extern static は Carbon が提供する CFStringRef グローバル定数。
            self.property_string(unsafe { kTISPropertyLocalizedName })
        }

        /// 対応言語コード一覧（BCP-47）。
        #[must_use]
        pub fn languages(&self) -> Vec<String> {
            // SAFETY: extern static は Carbon が提供する CFStringRef グローバル定数。
            let key = unsafe { kTISPropertyInputSourceLanguages };
            let Some(val) = self.property(key) else {
                return Vec::new();
            };
            let Some(arr) = val.downcast_ref::<CFArray>() else {
                return Vec::new();
            };
            let count = arr.count();
            let mut out = Vec::with_capacity(usize::try_from(count).unwrap_or(0));
            for i in 0..count {
                // SAFETY: i は 0..count の範囲内。要素は借用の CFStringRef。
                let elem = unsafe { arr.value_at_index(i) };
                if let Some(nn) = NonNull::new(elem.cast::<CFType>().cast_mut()) {
                    // SAFETY: 要素は arr（= self が所有）が生きている間有効な借用値。
                    let cf = unsafe { nn.as_ref() };
                    if let Some(s) = cf.downcast_ref::<CFString>() {
                        out.push(s.to_string());
                    }
                }
            }
            out
        }

        /// ASCII 入力が可能か。
        #[must_use]
        pub fn is_ascii_capable(&self) -> Option<bool> {
            // SAFETY: extern static は Carbon が提供する CFStringRef グローバル定数。
            self.property_bool(unsafe { kTISPropertyInputSourceIsASCIICapable })
        }

        /// カテゴリが `kTISCategoryKeyboardInputSource` か。
        #[must_use]
        pub fn is_keyboard_category(&self) -> bool {
            // SAFETY: extern static は Carbon が提供する CFStringRef グローバル定数。
            let key = unsafe { kTISPropertyInputSourceCategory };
            let Some(cat) = self.property(key) else {
                return false;
            };
            // SAFETY: 比較対象もグローバル定数の CFStringRef。release しない。
            let value_const = unsafe { kTISCategoryKeyboardInputSource };
            let Some(nn) = NonNull::new(value_const.cast::<CFType>().cast_mut()) else {
                return false;
            };
            // SAFETY: グローバル定数は常に生存。借用として比較にのみ使う。
            let expected = unsafe { nn.as_ref() };
            cat == expected
        }

        /// 有効化されているか。
        #[must_use]
        pub fn is_enabled(&self) -> Option<bool> {
            // SAFETY: extern static は Carbon が提供する CFStringRef グローバル定数。
            self.property_bool(unsafe { kTISPropertyInputSourceIsEnabled })
        }

        /// プログラムから選択可能か。
        #[must_use]
        pub fn is_select_capable(&self) -> Option<bool> {
            // SAFETY: extern static は Carbon が提供する CFStringRef グローバル定数。
            self.property_bool(unsafe { kTISPropertyInputSourceIsSelectCapable })
        }

        /// 提供元アプリの bundle identifier。
        #[must_use]
        pub fn bundle_id(&self) -> Option<String> {
            // SAFETY: extern static は Carbon が提供する CFStringRef グローバル定数。
            self.property_string(unsafe { kTISPropertyBundleID })
        }

        /// input mode ID（例: `com.apple.inputmethod.Japanese`）。
        #[must_use]
        pub fn input_mode_id(&self) -> Option<String> {
            // SAFETY: extern static は Carbon が提供する CFStringRef グローバル定数。
            self.property_string(unsafe { kTISPropertyInputModeID })
        }

        /// 入力ソースのカテゴリ文字列（例: `TISCategoryKeyboardInputSource`）。
        #[must_use]
        pub fn category(&self) -> Option<String> {
            // SAFETY: extern static は Carbon が提供する CFStringRef グローバル定数。
            self.property_string(unsafe { kTISPropertyInputSourceCategory })
        }

        /// 入力ソースの種別文字列（例: `TISTypeKeyboardInputMode`）。
        #[must_use]
        pub fn source_type(&self) -> Option<String> {
            // SAFETY: extern static は Carbon が提供する CFStringRef グローバル定数。
            self.property_string(unsafe { kTISPropertyInputSourceType })
        }

        /// 直列化された `UCKeyboardLayout`（`UCKeyTranslate` に渡すバイト列）を
        /// **所有 `Vec<u8>` にコピー**して返す。生ポインタを外へ出さない本モジュールの
        /// 規律を保つため、借用の `CFData` からバイトを複製する。'uchr' を持たない
        /// 入力ソース（一部の IME 等）では `None`。
        #[must_use]
        pub fn unicode_key_layout_data(&self) -> Option<Vec<u8>> {
            // SAFETY: extern static は Carbon が提供する CFStringRef グローバル定数。
            let val = self.property(unsafe { kTISPropertyUnicodeKeyLayoutData })?;
            let data = val.downcast_ref::<CFData>()?;
            let len = data.len();
            if len == 0 {
                return None;
            }
            let ptr = data.byte_ptr();
            if ptr.is_null() {
                return None;
            }
            // SAFETY: ptr は data（= self が生存中に所有する借用 CFData）の先頭を指し、
            // len バイト有効。この関数内で即座にコピーし、借用を跨いで保持しない。
            let bytes = unsafe { core::slice::from_raw_parts(ptr, len) };
            Some(bytes.to_vec())
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    //! 非 macOS ホスト（この開発機の Linux 等）向けフォールバック。Carbon は存在しない
    //! ため、全 API は空の結果を返す。これによりクレートはどのホストでもコンパイルできる。
    #![allow(clippy::unused_self)]

    /// 非 macOS では中身を持たないプレースホルダ。
    #[derive(Debug)]
    pub struct InputSourceHandle {
        _private: (),
    }

    impl InputSourceHandle {
        #[must_use]
        pub const fn source_id(&self) -> Option<String> {
            None
        }
        #[must_use]
        pub const fn localized_name(&self) -> Option<String> {
            None
        }
        #[must_use]
        pub const fn languages(&self) -> Vec<String> {
            Vec::new()
        }
        #[must_use]
        pub const fn is_ascii_capable(&self) -> Option<bool> {
            None
        }
        #[must_use]
        pub const fn is_keyboard_category(&self) -> bool {
            false
        }
        #[must_use]
        pub const fn is_enabled(&self) -> Option<bool> {
            None
        }
        #[must_use]
        pub const fn is_select_capable(&self) -> Option<bool> {
            None
        }
        #[must_use]
        pub const fn bundle_id(&self) -> Option<String> {
            None
        }
        #[must_use]
        pub const fn input_mode_id(&self) -> Option<String> {
            None
        }
        #[must_use]
        pub const fn category(&self) -> Option<String> {
            None
        }
        #[must_use]
        pub const fn source_type(&self) -> Option<String> {
            None
        }
        #[must_use]
        pub const fn unicode_key_layout_data(&self) -> Option<Vec<u8>> {
            None
        }
    }

    #[must_use]
    pub const fn copy_current_input_source() -> Option<InputSourceHandle> {
        None
    }

    #[must_use]
    pub const fn create_input_source_list(_include_all_installed: bool) -> Vec<InputSourceHandle> {
        Vec::new()
    }

    /// # Errors
    /// 非 macOS では常に `Err(-1)`（未対応）。
    pub const fn select_input_source(_handle: &InputSourceHandle) -> Result<(), i32> {
        Err(-1)
    }

    #[must_use]
    pub const fn selected_input_source_changed_notification_name() -> Option<String> {
        None
    }

    #[must_use]
    pub const fn enabled_input_sources_changed_notification_name() -> Option<String> {
        None
    }
}
