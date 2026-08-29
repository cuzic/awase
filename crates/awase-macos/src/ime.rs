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

    /// ID が日本語 IM のいずれかのモードかどうか。
    ///
    /// ATOK の英字モード ID は "Japanese" を含まない可能性があるため、
    /// 既知ベンダー（justsystems/atok）も判定に含める。
    fn is_japanese_im(id: &str) -> bool {
        id.contains("inputmethod")
            && (id.contains("Japanese") || id.contains("Kotoeri") || id.contains("atok"))
    }

    /// ID がひらがな系の日本語 IME モードかどうか（ON 側の選択対象）。
    fn is_japanese_kana_mode(id: &str) -> bool {
        is_japanese_im(id)
            && !id.contains("Roman")
            && !id.contains("Katakana")
            && !id.contains("Eiji")
    }

    /// ID の末尾セグメントを除いた IM ファミリ prefix を返す。
    ///
    /// かな/英字などのモード違いで同じ値になる。例:
    ///   com.justsystems.inputmethod.atok34.Japanese → …atok34
    ///   com.justsystems.inputmethod.atok34.Roman    → …atok34
    ///   com.google.inputmethod.Japanese.base        → ….Japanese
    fn im_family_prefix(id: &str) -> Option<&str> {
        id.rsplit_once('.').map(|(prefix, _)| prefix)
    }

    /// 候補 ID が同じ IM ファミリに属するか（セグメント境界厳密）。
    ///
    /// 素の `starts_with` だと `…Japanese` に `…JapaneseEvil.…` のような
    /// 兄弟 ID まで一致してしまう（2026-08-29 セキュリティレビュー第3回指摘3）。
    fn in_family(candidate: &str, prefix: &str) -> bool {
        candidate == prefix
            || candidate
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.starts_with('.'))
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

    /// IME 切替キー送出後、TIS 観測が追いつくまで期待値を優先する猶予時間。
    ///
    /// 入力ソースの切替は非同期で数十〜数百 ms かかるため、英数/かな 直後の
    /// 打鍵は観測ベースだと旧状態で判定されて素通り/誤変換する
    /// （英字モード→かな→即 k で「き」でなく k が出る問題）。
    const EXPECTATION_GRACE: std::time::Duration = std::time::Duration::from_millis(500);

    /// macOS IME 検出。
    ///
    /// - 最後に観測した日本語 IME（かなモード）の入力ソース ID を記憶し、
    ///   `set_ime_on` での復元先として使う。
    /// - IME 切替キーの直後は「まもなく切り替わる」期待値を観測より優先する
    ///   （`expect_ime_on` / `EXPECTATION_GRACE`）。
    ///
    /// 単一 thread（main の CFRunLoop）でのみ使う前提で `RefCell` を持つ。
    #[derive(Debug)]
    pub struct ImeDetector {
        /// 最後に観測した日本語 IME かなモードの ID（ON 復元先の第一候補）
        last_japanese_id: std::cell::RefCell<Option<String>>,
        /// 最後に観測した日本語 IM のファミリ prefix（英数モードでも更新。
        /// 英数モードで起動して ON する場合の復元先の確定に使う —
        /// 2026-08-29 セキュリティレビュー第2回指摘）
        last_japanese_prefix: std::cell::RefCell<Option<String>>,
        /// IME 切替キー送出後の期待状態（観測が追いつくか猶予超過で解除）
        pending: std::cell::RefCell<Option<(bool, std::time::Instant)>>,
    }

    impl ImeDetector {
        #[must_use]
        pub fn new() -> Self {
            log::info!("IME detector: TISCopyCurrentKeyboardInputSource");
            let detector = Self {
                last_japanese_id: std::cell::RefCell::new(None),
                last_japanese_prefix: std::cell::RefCell::new(None),
                pending: std::cell::RefCell::new(None),
            };
            // 起動時点の入力ソースを観測しておく（最初の打鍵前に
            // set_ime_on(true) が呼ばれても復元先が分かるように）
            let _ = detector.is_ime_on();
            detector
        }

        /// IME 切替キー（英数/かな）が OS に届いた直後に呼び、期待状態を立てる。
        pub fn expect_ime_on(&self, open: bool) {
            *self.pending.borrow_mut() = Some((open, std::time::Instant::now()));
        }

        /// IME 切替がまだ観測で確認できていない（切替中）かどうか。
        ///
        /// この間に注入したキーストロークは旧入力ソースで解釈されて
        /// リテラルの "wo" 等になるため、呼び出し側は送出を遅延させる。
        #[must_use]
        pub fn is_switch_pending(&self) -> bool {
            // 観測を取り込んで pending の解除判定を進めてから判定する
            let _ = self.is_ime_on();
            self.pending.borrow().is_some()
        }

        /// 現在の IME 状態を問い合わせる
        /// - Some(true): IME ON (ひらがな・カタカナモード等)
        /// - Some(false): IME OFF (英数モード・IME なしレイアウト)
        /// - None: 検出不可
        #[must_use]
        pub fn is_ime_on(&self) -> Option<bool> {
            let observed = self.observe_ime_on();

            // 期待値の適用: 観測が一致したら解除、猶予内の不一致は期待値優先
            let mut pending = self.pending.borrow_mut();
            if let Some((expected, at)) = *pending {
                if observed == Some(expected) || at.elapsed() > EXPECTATION_GRACE {
                    *pending = None;
                } else {
                    return Some(expected);
                }
            }
            observed
        }

        /// TIS 観測のみで IME 状態を判定する（期待値を適用しない）。
        fn observe_ime_on(&self) -> Option<bool> {
            let id = current_input_source_id()?;
            if is_japanese_im(&id) {
                // ユーザーが実際に使っている日本語 IM を記憶する
                // （ATOK / Google 日本語入力 / 日本語IM の区別を保つため。
                // 述語ベースの選択だとリスト先頭の OS 標準 IME に化ける）。
                // ファミリ prefix は英数モードでも更新する — 英数モードで
                // 起動すると kana ID が未観測のままになるため。
                if let Some(prefix) = im_family_prefix(&id) {
                    *self.last_japanese_prefix.borrow_mut() = Some(prefix.to_string());
                }
                if is_japanese_kana_mode(&id) {
                    let mut last = self.last_japanese_id.borrow_mut();
                    if last.as_deref() != Some(&id) {
                        log::debug!("IME observed: {id}");
                        *last = Some(id.clone());
                    }
                }
                // "…Japanese" / "…Japanese.Katakana" は ON、
                // "…Roman" / "…FullWidthRoman" / "…HalfWidthEiji"（英字モード）は OFF
                Some(!(id.contains("Roman") || id.contains("Eiji")))
            } else if id.contains("keylayout") {
                Some(false)
            } else {
                None
            }
        }

        /// 日本語 IME がアクティブかどうか（英数モードも含む）
        ///
        /// ON 期待（IME 切替キー直後）の間は true を返す — `compute_state` は
        /// `is_japanese_ime` を `ime_on` より先に評価するため、ABC keylayout
        /// からの切替中にここが false だと期待値ブリッジが無効化される。
        #[must_use]
        pub fn is_japanese_layout(&self) -> bool {
            if let Some((true, at)) = *self.pending.borrow() {
                if at.elapsed() <= EXPECTATION_GRACE {
                    return true;
                }
            }
            current_input_source_id().is_none_or(|id| is_japanese_im(&id))
        }

        /// IME の ON/OFF を強制する（`ImeEffect::SetOpen` の実装）。
        ///
        /// - ON: 最後に使っていた日本語 IME のかなモードを復元。かなモード
        ///   未観測ならファミリ prefix 一致に限って選択する。ファミリも不明なら
        ///   失敗させる — 複数 IME 環境でリスト先頭の別 IME に切り替えるのは
        ///   ユーザー意図に反し、入力内容を扱うコンポーネントの選択としても
        ///   危険なため（2026-08-29 セキュリティレビュー第2回指摘）
        /// - OFF: 同じ IM ファミリの英字モード（`….Roman` / `…Eiji`）を優先し、
        ///   無ければ keylayout（ABC 等）
        ///
        /// 対象の入力ソースが見つからない/選択に失敗した場合は false。
        pub fn set_ime_on(&self, open: bool) -> bool {
            let selected = self.select_for(open);
            if selected {
                // TISSelectInputSource も反映まで observation lag があるため
                // 期待を立てる（失敗時に立てると偽の ON/OFF 判定が猶予いっぱい
                // 残るので、成功時のみ）
                self.expect_ime_on(open);
            }
            selected
        }

        fn select_for(&self, open: bool) -> bool {
            let last = self.last_japanese_id.borrow().clone();
            let prefix = self.last_japanese_prefix.borrow().clone();
            if open {
                if let Some(ref id) = last {
                    if select_input_source_matching(|c| c == id) {
                        return true;
                    }
                    log::warn!("IME restore failed for {id}, trying family prefix");
                }
                if let Some(ref prefix) = prefix {
                    if select_input_source_matching(|c| {
                        in_family(c, prefix) && is_japanese_kana_mode(c)
                    }) {
                        return true;
                    }
                }
                log::warn!(
                    "IME set_open(true): no Japanese IME observed yet; refusing to pick \
                     an arbitrary input source"
                );
                false
            } else {
                if let Some(ref prefix) = prefix {
                    // 例: …atok34.Japanese → …atok34.Roman / …atok34.…Eiji
                    if select_input_source_matching(|c| {
                        in_family(c, prefix)
                            && (c.contains("Roman") || c.contains("Eiji"))
                            && !c.contains("FullWidth")
                    }) {
                        return true;
                    }
                }
                select_input_source_matching(|id| id.contains("keylayout"))
            }
        }
    }

    impl Default for ImeDetector {
        fn default() -> Self {
            Self::new()
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn kana_mode_matches_hiragana_ids_of_known_imes() {
            assert!(is_japanese_kana_mode(
                "com.apple.inputmethod.Kotoeri.RomajiTyping.Japanese"
            ));
            assert!(is_japanese_kana_mode("com.google.inputmethod.Japanese.base"));
            assert!(is_japanese_kana_mode(
                "com.justsystems.inputmethod.atok34.Japanese"
            ));
        }

        #[test]
        fn kana_mode_rejects_roman_katakana_and_keylayouts() {
            assert!(!is_japanese_kana_mode(
                "com.apple.inputmethod.Kotoeri.RomajiTyping.Roman"
            ));
            assert!(!is_japanese_kana_mode(
                "com.google.inputmethod.Japanese.Roman"
            ));
            assert!(!is_japanese_kana_mode(
                "com.google.inputmethod.Japanese.Katakana"
            ));
            assert!(!is_japanese_kana_mode(
                "com.justsystems.inputmethod.atok34.Japanese.HalfWidthEiji"
            ));
            assert!(!is_japanese_kana_mode("com.apple.keylayout.ABC"));
        }

        #[test]
        fn japanese_im_covers_atok_roman_mode_without_japanese_substring() {
            assert!(is_japanese_im("com.justsystems.inputmethod.atok34.Roman"));
            assert!(!is_japanese_im("com.apple.keylayout.ABC"));
        }

        #[test]
        fn in_family_requires_segment_boundary() {
            let prefix = "com.google.inputmethod.Japanese";
            assert!(in_family("com.google.inputmethod.Japanese", prefix));
            assert!(in_family("com.google.inputmethod.Japanese.Roman", prefix));
            // 兄弟 ID（prefix + 追加文字）はセグメント境界で拒否する
            assert!(!in_family("com.google.inputmethod.JapaneseEvil.base", prefix));
        }

        #[test]
        fn family_prefix_is_mode_invariant() {
            assert_eq!(
                im_family_prefix("com.justsystems.inputmethod.atok34.Japanese"),
                im_family_prefix("com.justsystems.inputmethod.atok34.Roman"),
            );
            assert_eq!(
                im_family_prefix("com.google.inputmethod.Japanese.base"),
                Some("com.google.inputmethod.Japanese"),
            );
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

    pub fn expect_ime_on(&self, _open: bool) {}

    #[must_use]
    pub fn is_switch_pending(&self) -> bool {
        false
    }
}

#[cfg(not(target_os = "macos"))]
impl Default for ImeDetector {
    fn default() -> Self {
        Self::new()
    }
}
