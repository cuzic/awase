//! 低レベルのキーイベント送出プリミティブ。
//!
//! synthetic identity（[`crate::synthetic::SyntheticEventOrigin`]）で tagged した
//! CGEvent を組み立て、`CGEventPost` で送出する。各イベントには
//! `kCGEventSourceUserData` として origin の cookie を埋め込み、後段の tap 側が
//! `is_self_event` で自己生成物を識別できるようにする。
//!
//! [`release_all_best_effort`] は tap 無効化などでの断絶復旧用で、
//! [`crate::synthetic::SyntheticPressedKeys`] に残った未解放キーへ best-effort で
//! tagged key-up を送る（個別失敗はログのみでループは止めない）。
//!
//! 対話的な出力実験（Unicode 文字列の `CGEventKeyboardSetUnicodeString` 直接送出、
//! `output-test` CLI サブコマンド、手動テスト文字列セット）もこのモジュールに含む
//! （[`run_output_test`] / [`OutputTestMode`] / [`MANUAL_TEST_STRINGS`] / [`to_utf16`]）。
//! ここは **送出側のみ** を担い、対象アプリごとの受理挙動差の確認は実機ゲート
//! （タスク #19）に委ねる。

use crate::synthetic::{SyntheticEventOrigin, SyntheticPressedKeys};

/// `keycode` の key-down を origin cookie 付きで送出する。
///
/// # Errors
/// CGEvent の生成に失敗した場合、またはサポート外プラットフォームで呼ばれた場合に返す。
pub fn post_key_down(keycode: u16, origin: &SyntheticEventOrigin) -> anyhow::Result<()> {
    post_key_event(keycode, true, origin)
}

/// `keycode` の key-up を origin cookie 付きで送出する。
///
/// # Errors
/// CGEvent の生成に失敗した場合、またはサポート外プラットフォームで呼ばれた場合に返す。
pub fn post_key_up(keycode: u16, origin: &SyntheticEventOrigin) -> anyhow::Result<()> {
    post_key_event(keycode, false, origin)
}

/// `keycode` の key-down を送出し、直後に key-up を送出する。
///
/// # Errors
/// down / up いずれかの送出に失敗した場合に返す（down 失敗時は up を送らない）。
pub fn post_key_pair(keycode: u16, origin: &SyntheticEventOrigin) -> anyhow::Result<()> {
    post_key_down(keycode, origin)?;
    post_key_up(keycode, origin)
}

/// 未解放キーを drain し、それぞれへ tagged key-up を best-effort で送る。
///
/// 個別の送出失敗は握りつぶしてログに残すのみで、残りのキーの解放は続行する
/// （断絶復旧の途中で 1 キーの失敗により他キーが stuck するのを避けるため）。
pub fn release_all_best_effort(pressed: &mut SyntheticPressedKeys, origin: &SyntheticEventOrigin) {
    for keycode in pressed.take_all_for_release() {
        if let Err(err) = post_key_up(keycode, origin) {
            log::warn!("release_all_best_effort: key-up for keycode {keycode} failed: {err}");
        }
    }
}

#[cfg(target_os = "macos")]
fn post_key_event(
    keycode: u16,
    key_down: bool,
    origin: &SyntheticEventOrigin,
) -> anyhow::Result<()> {
    use objc2_core_graphics::{CGEvent, CGEventField, CGEventTapLocation};

    let event = CGEvent::new_keyboard_event(None, keycode, key_down).ok_or_else(|| {
        anyhow::anyhow!(
            "CGEventCreateKeyboardEvent returned null (keycode={keycode}, down={key_down})"
        )
    })?;
    CGEvent::set_integer_value_field(
        Some(&event),
        CGEventField::EventSourceUserData,
        origin.cookie(),
    );
    CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&event));
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn post_key_event(
    _keycode: u16,
    _key_down: bool,
    _origin: &SyntheticEventOrigin,
) -> anyhow::Result<()> {
    Err(anyhow::anyhow!("unsupported platform"))
}

// ---------------------------------------------------------------------------
// 対話的な出力実験（`output-test` サブコマンド、タスク #10）
// ---------------------------------------------------------------------------

/// 手動確認用のテスト文字列セット。UTF-16 エンコードの境界ケースを網羅する:
/// BMP 内（`あ` / 事前合成済み `é`）、結合文字列（`e` + U+0301）、BMP 外でサロゲート
/// ペアになる絵文字（`👍` = U+1F44D）と CJK 拡張漢字（`𠮷` = U+20BB7）。実機での
/// 受理挙動差（タスク #19）を試すときの入力候補。
#[allow(dead_code)] // #19 実機検証・将来の網羅送信サブコマンド向けの参照セット
pub const MANUAL_TEST_STRINGS: &[&str] = &[
    "あ",               // BMP、1 code unit
    "\u{00e9}",         // é（事前合成 U+00E9）、1 code unit
    "e\u{0301}",        // é（e + 結合アクセント U+0301）、2 code unit
    "👍",               // U+1F44D、サロゲートペアで 2 code unit
    "𠮷",               // U+20BB7、サロゲートペアで 2 code unit
    "aあ👍𠮷e\u{0301}", // 混在
];

/// `output-test <mode>` のモード。
///
/// 文字列表現（`passthrough` / `suppress` / `substitute:<key>` / `unicode:<str>`）は
/// [`FromStr`](std::str::FromStr) で相互変換でき、main.rs 側の結線を簡潔にする。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputTestMode {
    /// 実イベントを素通しする（送出側は何もしない）。
    Passthrough,
    /// 実イベントを握り潰す（送出側は何もしない）。
    Suppress,
    /// 実キーを代替キーコードに差し替える。
    Substitute(u16),
    /// 実キーの代わりに Unicode 文字列を直接送出する。
    Unicode(String),
}

impl std::str::FromStr for OutputTestMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "passthrough" => Ok(Self::Passthrough),
            "suppress" => Ok(Self::Suppress),
            _ => {
                if let Some(key) = s.strip_prefix("substitute:") {
                    let keycode = key
                        .parse::<u16>()
                        .map_err(|_| format!("invalid keycode in `substitute:<key>`: {key:?}"))?;
                    Ok(Self::Substitute(keycode))
                } else if let Some(text) = s.strip_prefix("unicode:") {
                    Ok(Self::Unicode(text.to_owned()))
                } else {
                    Err(format!(
                        "unknown output-test mode: {s:?} \
                         (expected passthrough | suppress | substitute:<key> | unicode:<str>)"
                    ))
                }
            }
        }
    }
}

/// 文字列を UTF-16 code unit 列へエンコードする。
///
/// `char as u16` では BMP 外（絵文字・CJK 拡張等）が壊れるため使わない。
/// `encode_utf16` は BMP 外文字をサロゲートペア（2 code unit）に展開する。
/// `CGEventKeyboardSetUnicodeString` はこの UTF-16 列をそのまま受け取る。
#[must_use]
pub fn to_utf16(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

/// `output-test <mode>` の本体。output.rs が担う **送出側** の実験を行う。
///
/// - `Unicode(s)`: [`CGEventKeyboardSetUnicodeString`][set] で `s` を直接送出する（実送出）。
/// - `Substitute(k)`: 代替キー `k` の down/up を 1 組送出する（tap で実キーを差し替える
///   ときに送る出力の送出側デモ）。
/// - `Passthrough` / `Suppress`: 「実キーストリームをどう扱うか」という **tap callback 側の
///   変換** であり、送出側からは新規送出しない（passthrough=素通し、suppress=握り潰し）。
///   実際の横取り挙動は tap のイベントループ結線と実機ゲート #19 で確認する。
///
/// **対象アプリ差の注意**: TextEdit / Safari 通常欄・パスワード欄 / Terminal / Electron /
/// VS Code / ネイティブ AppKit / Secure Input 中での受理挙動差は、このクレートからは
/// 観測できない（ここは送出側のみ）。実アプリ差の確認はタスク #19。
///
/// [set]: https://developer.apple.com/documentation/coregraphics/1456028-cgeventkeyboardsetunicodestring
///
/// # Errors
///
/// CGEvent 生成・送出に失敗した場合、または非 macOS で呼ばれた場合に返す。
#[cfg(target_os = "macos")]
pub fn run_output_test(mode: OutputTestMode) -> anyhow::Result<()> {
    let origin = SyntheticEventOrigin::new();
    match mode {
        OutputTestMode::Unicode(text) => {
            log::info!(
                "output-test unicode: sending {text:?} ({} UTF-16 code units)",
                to_utf16(&text).len()
            );
            post_unicode_string(&text, &origin)
        }
        OutputTestMode::Substitute(keycode) => {
            log::info!("output-test substitute: emitting substitute keycode {keycode}");
            post_key_pair(keycode, &origin)
        }
        OutputTestMode::Passthrough => {
            log::info!(
                "output-test passthrough: no synthetic output (events flow unchanged); \
                 interception is the tap loop's job (#19)"
            );
            Ok(())
        }
        OutputTestMode::Suppress => {
            log::info!(
                "output-test suppress: emit nothing (real key suppression happens in the \
                 tap callback; #19)"
            );
            Ok(())
        }
    }
}

/// [`run_output_test`] の非 macOS スタブ。
///
/// # Errors
///
/// 非 macOS では常に `Err`（unsupported platform）を返す。
// `mode` を値で取るのは macOS 版と同一シグネチャに揃えるため（macOS 版は
// `Unicode(String)` を move で消費する）。非 macOS スタブでは使わないだけなので許可。
#[cfg(not(target_os = "macos"))]
#[allow(clippy::needless_pass_by_value)]
pub fn run_output_test(mode: OutputTestMode) -> anyhow::Result<()> {
    let _ = mode;
    Err(anyhow::anyhow!("unsupported platform"))
}

/// `CGEventKeyboardSetUnicodeString` で UTF-16 文字列を載せた keyboard event を送出する。
#[cfg(target_os = "macos")]
fn post_unicode_string(text: &str, origin: &SyntheticEventOrigin) -> anyhow::Result<()> {
    use objc2_core_graphics::{CGEvent, CGEventField, CGEventTapLocation};

    let utf16 = to_utf16(text);
    // UniCharCount は objc2-core-graphics 内部で `c_ulong` への pub(crate) 別名。
    // 透過的なので c_ulong をそのまま渡す。
    let length = core::ffi::c_ulong::try_from(utf16.len())
        .map_err(|_| anyhow::anyhow!("unicode string too long ({} code units)", utf16.len()))?;
    let event = CGEvent::new_keyboard_event(None, 0, true).ok_or_else(|| {
        anyhow::anyhow!("CGEventCreateKeyboardEvent returned null for unicode payload")
    })?;
    CGEvent::set_integer_value_field(
        Some(&event),
        CGEventField::EventSourceUserData,
        origin.cookie(),
    );
    // SAFETY: utf16 は呼び出し中有効な UTF-16 スライスで、length はその要素数。
    // keyboard_set_unicode_string はポインタと長さを読むのみ（所有権移動なし）。
    #[allow(unsafe_code)]
    unsafe {
        CGEvent::keyboard_set_unicode_string(Some(&event), length, utf16.as_ptr());
    }
    CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&event));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{to_utf16, OutputTestMode, MANUAL_TEST_STRINGS};

    #[test]
    fn utf16_bmp_char_is_one_unit() {
        assert_eq!(to_utf16("あ").len(), 1);
        assert_eq!(to_utf16("\u{00e9}").len(), 1); // 事前合成 é
    }

    #[test]
    fn utf16_astral_chars_are_surrogate_pairs() {
        assert_eq!(to_utf16("👍").len(), 2); // U+1F44D
        assert_eq!(to_utf16("𠮷").len(), 2); // U+20BB7
    }

    #[test]
    fn utf16_combining_sequence_keeps_both_units() {
        // e + 結合アクセントは 2 code unit（合成されない）。
        assert_eq!(to_utf16("e\u{0301}").len(), 2);
    }

    #[test]
    fn utf16_roundtrips_back_to_the_same_string() {
        for s in MANUAL_TEST_STRINGS {
            let units = to_utf16(s);
            let back = String::from_utf16(&units).expect("valid UTF-16");
            assert_eq!(&back, s);
        }
    }

    #[test]
    fn parse_simple_modes() {
        assert_eq!(
            "passthrough".parse::<OutputTestMode>(),
            Ok(OutputTestMode::Passthrough)
        );
        assert_eq!(
            "suppress".parse::<OutputTestMode>(),
            Ok(OutputTestMode::Suppress)
        );
    }

    #[test]
    fn parse_substitute_and_unicode() {
        assert_eq!(
            "substitute:40".parse::<OutputTestMode>(),
            Ok(OutputTestMode::Substitute(40))
        );
        assert_eq!(
            "unicode:あé👍".parse::<OutputTestMode>(),
            Ok(OutputTestMode::Unicode("あé👍".to_owned()))
        );
        // `unicode:` 以降はコロンを含めてそのまま本文になる。
        assert_eq!(
            "unicode:a:b".parse::<OutputTestMode>(),
            Ok(OutputTestMode::Unicode("a:b".to_owned()))
        );
    }

    #[test]
    fn parse_rejects_garbage_and_bad_keycode() {
        assert!("nope".parse::<OutputTestMode>().is_err());
        assert!("substitute:notanumber".parse::<OutputTestMode>().is_err());
        assert!("substitute:99999".parse::<OutputTestMode>().is_err()); // u16 溢れ
    }
}
