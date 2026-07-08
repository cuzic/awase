//! keycode → 文字マッピング調査（`UCKeyTranslate`）。
//!
//! 目的: awase のエンジンは `Romaji(String)` を **仮想キーコード列** として
//! `CGEventCreateKeyboardEvent` で送る。しかし「ある仮想キーコードがどの文字を
//! 生むか」は **OS で現在選択中のキーボードレイアウト**（JIS ローマ字 / US ABC /
//! Dvorak …）に依存し、固定ではない。macOS で `Romaji("ka")` を実装する前に、
//! keycode ↔ 文字の関係を実測する土台がこのモジュール。判断材料を集めるのが役割で、
//! 設計判断そのものはここでは下さない。
//!
//! ## いま分かっていること（Linux 上の静的知識のみ）
//! - Carbon の `UCKeyTranslate` は、TIS の現在ソースが持つ直列化 `UCKeyboardLayout`
//!   （`kTISPropertyUnicodeKeyLayoutData`）を使って `(keycode, modifiers)` を
//!   Unicode 文字へ解決する **純粋なソフトウェアクエリ**。物理キー押下も TCC 権限も不要。
//! - よって macos-latest の GitHub Actions ランナー（人間不在・権限なし）でも実行でき、
//!   その環境の「現在レイアウト」に対する keycode→文字表を機械的に取得できる。
//! - 逆に「ある論理文字を出す keycode」を得たいなら、全 keycode を forward 変換して
//!   **逆引き表（char→keycode）** を startup で構築するのが素直（下記推奨 B）。
//!
//! ## CI（macos-latest）が教えてくれること
//! - ランナー既定レイアウト（おそらく "U.S." / ABC, ASCII 系）での keycode→文字表。
//!   例えば keycode 0x00 が `a`、Shift+0x00 が `A` を返すはず、という前提を実測で確認できる。
//! - `unicode_key_layout_data()` が実際に非 NULL を返すか（= 'uchr' を持つソースか）。
//!
//! ## まだ実機（Task #19）が要ること
//! - JIS / Dvorak / かな入力など **非既定レイアウト** での挙動。GH Actions ランナーは
//!   ストックの入力ソースしか持たない可能性が高く、`TISSelectInputSource` で JIS を
//!   選べる保証がない（インストール済みでなければ選択不可）。網羅は実機タスク。
//! - IME（ことえり等）選択時の挙動: IME ソースは 'uchr' を持たず
//!   `unicode_key_layout_data()` が None になりうる。その場合の Romaji 送出戦略は別問題。
//!
//! ## 暫定推奨（証拠が指す方向。確定ではない）
//! - **選択肢 B（現在レイアウトを見て論理文字→keycode を逆解決）に傾く。** 理由:
//!   選択肢 A（常に QWERTY 位置を仮定）は Dvorak/JIS 実キーボードでユーザが別レイアウトを
//!   使うと崩れる。`UCKeyTranslate` の forward 変換で startup 時に char→keycode 表を作り、
//!   `kTISNotifySelectedKeyboardInputSourceChanged` でレイアウト変更時に作り直せば、
//!   ユーザのレイアウトに依らず意図した romaji 文字を出せる。
//! - ただし確証は無い: 一部の文字は modifier / dead key を要し、かな入力や IME ON 時は
//!   'uchr' 逆引きが成立しない。B を確定する前に実機（#19）で JIS/Dvorak/IME を要検証。
//! - このモジュールは、その逆引き表を作る／検証するための **forward プリミティブ** を提供する。

// Carbon の UCKeyTranslate FFI 呼び出しに unsafe が必須。
#![allow(unsafe_code)]

#[cfg(target_os = "macos")]
pub use imp::run;

#[cfg(not(target_os = "macos"))]
pub use imp::run;

/// 調査対象のサンプル keycode（ANSI 物理配列）。`run()` と test で共有する。
/// `Romaji("ka")` を意識して K と A を含める。
const SAMPLE_KEYCODES: &[(u16, &str)] = &[
    (0x00, "kVK_ANSI_A"),
    (0x01, "kVK_ANSI_S"),
    (0x02, "kVK_ANSI_D"),
    (0x28, "kVK_ANSI_K"),
    (0x2D, "kVK_ANSI_N"),
    (0x12, "kVK_ANSI_1"),
];

/// `UCKeyTranslate` の `modifierKeyState` に渡す Shift ビット。
/// Event-record の `shiftKey`(0x0200) を右 8bit した値（= 0x02）。修飾なしは 0。
const MODIFIER_SHIFT: u32 = 0x02;

#[cfg(target_os = "macos")]
mod imp {
    use core::ffi::{c_ulong, c_void};

    use crate::tis_sys::{copy_current_input_source, InputSourceHandle};

    use super::{MODIFIER_SHIFT, SAMPLE_KEYCODES};

    /// `UniCharCount`（Carbon）は `unsigned long`。Darwin LP64 では 64bit。
    type UniCharCount = c_ulong;

    /// `kUCKeyActionDown`: キー押下時に生成される文字を問い合わせる。
    const UC_KEY_ACTION_DOWN: u16 = 0;
    /// `kUCKeyTranslateNoDeadKeysMask`(= 1<<0): dead key 状態を持ち越さず、
    /// 各 keycode を独立に解決する（本調査はステートレスに見たい）。
    const UC_KEY_TRANSLATE_NO_DEAD_KEYS_MASK: u32 = 1;

    #[link(name = "Carbon", kind = "framework")]
    #[allow(non_snake_case)]
    extern "C" {
        /// 直列化 `UCKeyboardLayout`（第1引数, 借用ポインタ）を使って
        /// `(virtualKeyCode, modifierKeyState)` を Unicode 文字列へ変換する。
        /// 戻り値は `OSStatus`（0 = 成功）。`unicodeString` は呼び出し側バッファ。
        fn UCKeyTranslate(
            key_layout_ptr: *const c_void,
            virtual_key_code: u16,
            key_action: u16,
            modifier_key_state: u32,
            keyboard_type: u32,
            key_translate_options: u32,
            dead_key_state: *mut u32,
            max_string_length: UniCharCount,
            actual_string_length: *mut UniCharCount,
            unicode_string: *mut u16,
        ) -> i32;

        /// 現在の物理キーボード種別（`UCKeyTranslate` の `keyboardType` 引数用）。
        fn LMGetKbdType() -> u8;
    }

    /// 現在選択中の入力ソースの下で `virtual_keycode`（+ `modifier_key_state`）が
    /// 生成する文字を解決する。純粋なソフトウェアクエリで権限も物理入力も不要。
    ///
    /// `modifier_key_state` は event-record 修飾ビットを右 8bit した値
    /// （[`MODIFIER_SHIFT`] = Shift、0 = 修飾なし）。
    /// 対象ソースが 'uchr' を持たない（IME 等）場合や変換失敗時は `None`。
    #[must_use]
    pub fn resolve_keycode(
        handle: &InputSourceHandle,
        virtual_keycode: u16,
        modifier_key_state: u32,
    ) -> Option<String> {
        let layout = handle.unicode_key_layout_data()?;
        // layout（Vec<u8>）はこの関数の終わりまで生存するため、その先頭ポインタは
        // UCKeyTranslate 呼び出し中ずっと有効。UCKeyTranslate は読み取りのみ。
        let layout_ptr = layout.as_ptr().cast::<c_void>();

        // SAFETY: 引数なしの Carbon 関数。現在のキーボード種別を返す。
        let keyboard_type = u32::from(unsafe { LMGetKbdType() });

        let mut dead_key_state: u32 = 0;
        let mut buf = [0u16; 8];
        let mut actual_len: UniCharCount = 0;
        let max_len = buf.len() as UniCharCount;

        // SAFETY: layout_ptr は len>0 の直列化 UCKeyboardLayout を指す借用ポインタ。
        // dead_key_state / actual_len / buf は呼び出し側所有の可変域で、buf は max_len 要素。
        let status = unsafe {
            UCKeyTranslate(
                layout_ptr,
                virtual_keycode,
                UC_KEY_ACTION_DOWN,
                modifier_key_state,
                keyboard_type,
                UC_KEY_TRANSLATE_NO_DEAD_KEYS_MASK,
                &raw mut dead_key_state,
                max_len,
                &raw mut actual_len,
                buf.as_mut_ptr(),
            )
        };

        if status != 0 {
            return None;
        }
        let n = usize::try_from(actual_len).unwrap_or(0).min(buf.len());
        if n == 0 {
            return None;
        }
        String::from_utf16(&buf[..n]).ok()
    }

    /// `layout-probe` サブコマンド本体。現在の入力ソースを表示し、サンプル keycode を
    /// 修飾なし / Shift 付きで解決してログに出す。macos-latest の CI で実行されると、
    /// そのランナーの既定レイアウトに対する keycode→文字表が得られる。
    pub fn run() {
        let Some(source) = copy_current_input_source() else {
            log::warn!("layout-probe: no current keyboard input source (unexpected on a Mac)");
            return;
        };
        log::info!(
            "layout-probe: current input source id={:?} name={:?} ascii_capable={:?}",
            source.source_id(),
            source.localized_name(),
            source.is_ascii_capable(),
        );

        if source.unicode_key_layout_data().is_none() {
            log::warn!(
                "layout-probe: current source has no UnicodeKeyLayoutData ('uchr'); \
                 likely an IME/keyboard-layout-less source — keycode→char is undefined here"
            );
            return;
        }

        for &(keycode, name) in SAMPLE_KEYCODES {
            let plain = resolve_keycode(&source, keycode, 0);
            let shifted = resolve_keycode(&source, keycode, MODIFIER_SHIFT);
            log::info!(
                "layout-probe: {name} (0x{keycode:02X}) -> plain={plain:?} shift={shifted:?}"
            );
        }
    }

    #[cfg(test)]
    mod tests {
        use super::{resolve_keycode, run};
        use crate::tis_sys::copy_current_input_source;

        /// forward 解決のスモークテスト。macos-latest ランナー（既定 ASCII 系レイアウト）で
        /// 実行される前提。特定文字（US 固有の 'a' 等）を hard-code すると JIS/Dvorak 環境で
        /// 脆くなるため、**レイアウト非依存な不変条件のみ** を検証する:
        /// - 現在ソースが 'uchr' を持つなら、英字物理キーは何らかの非空 1 文字を生む。
        /// - Shift 版も何らかの非空文字を生む。
        ///
        /// 実際の文字は情報としてログに出す（CI ログで US ABC の実測が読める）。
        #[test]
        fn current_layout_resolves_alpha_keys() {
            let Some(source) = copy_current_input_source() else {
                // ヘッドレス環境で現在ソースが無い場合はスキップ（CI を赤化させない）。
                eprintln!("skip: no current keyboard input source");
                return;
            };
            if source.unicode_key_layout_data().is_none() {
                eprintln!("skip: current source has no UnicodeKeyLayoutData ('uchr')");
                return;
            }

            // 英字物理キー（A/S/D/K/N）は、いかなる Latin 系レイアウトでも非空文字を生む。
            for &keycode in &[0x00u16, 0x01, 0x02, 0x28, 0x2D] {
                let plain = resolve_keycode(&source, keycode, 0);
                eprintln!("keycode 0x{keycode:02X} plain -> {plain:?}");
                let s = plain.expect("alpha keycode should resolve to a character");
                assert!(!s.is_empty(), "resolved string must be non-empty");
            }
        }

        /// `run()` がどの環境でも panic せず完走することだけを保証する
        /// （現在ソース無し・'uchr' 無しでも早期 return する）。
        #[test]
        fn run_does_not_panic() {
            run();
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    //! 非 macOS ホスト向けスタブ。Carbon / `UCKeyTranslate` は存在しないため、
    //! 解決は常に `None`、`run()` は未対応を告知する。
    use crate::tis_sys::InputSourceHandle;

    // Platform-parity stub: the only non-macOS caller path is `run()`, which
    // short-circuits before translating, so this has no caller off macOS.
    #[allow(dead_code)]
    #[must_use]
    pub const fn resolve_keycode(
        _handle: &InputSourceHandle,
        _virtual_keycode: u16,
        _modifier_key_state: u32,
    ) -> Option<String> {
        None
    }

    pub fn run() {
        log::info!(
            "layout-probe: keyboard-layout translation requires macOS (Carbon UCKeyTranslate); \
             unavailable on this host"
        );
    }
}
