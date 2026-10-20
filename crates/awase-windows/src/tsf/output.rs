#![allow(unsafe_code)]
// Win32 API 呼び出しに unsafe が必須(lib.rsのクレート全体allowから個別移管、Task #9)
//! action 層 — judgement 結果を元に SendInput を組み立て実行する。
//!
//! - `ColdReason`: cold になった理由（タイミングパラメータを決定する）
//! - `INJECTED_MARKER`, `TSF_MARKER`: SendInput の dwExtraInfo マーカー
//! - TSF 専用ヘルパー関数: `make_tsf_key_input`, `make_key_input_ex`

use windows::Win32::UI::Input::KeyboardAndMouse::{
    MapVirtualKeyW, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    MAPVK_VK_TO_VSC, VIRTUAL_KEY,
};

/// 自己注入マーカー（"KEYM" = 0x4B45_594D）
pub const INJECTED_MARKER: usize = 0x4B45_594D;

/// TSF 向け注入マーカー（"KEYF" = 0x4B45_5946）
///
/// hook では INJECTED_MARKER と同様に再処理をスキップするが、
/// dwExtraInfo の値が異なることで TSF Sequential モードの識別に使う。
pub const TSF_MARKER: usize = 0x4B45_5946;

/// IME KANJI トグル注入マーカー（"KEYJ" = 0x4B45_594A）
///
/// Chrome 等の TSF ネイティブアプリ向け IME OFF フォールバックで使用。
/// hook では再処理・shadow toggle を一切行わずそのままパススルーする。
pub const IME_KANJI_MARKER: usize = 0x4B45_594A;

pub use super::cold_reason::ColdReason;

/// scan code 付き INPUT 構造体を作成する共通ヘルパー。
///
/// `wVk` を保持したまま `MapVirtualKeyW` で算出した `wScan` も設定する。
/// `KEYEVENTF_SCANCODE` は付加しない（付加すると WezTerm が LLKHF_SCANCODE フラグ付き
/// キーとして検出し IME をバイパスしてしまうため、`99f56a2`→`2d4d85c` の2段階の実機
/// トライで判明した組み合わせ）。`extra_info` は呼び出し元のマーカーをそのまま使う。
pub(crate) fn make_scan_key_input(
    vk: awase::types::VkCode,
    is_keyup: bool,
    extra_info: usize,
) -> INPUT {
    let scan = unsafe { MapVirtualKeyW(u32::from(vk.0), MAPVK_VK_TO_VSC) as u16 };
    let flags = if is_keyup {
        KEYEVENTF_KEYUP
    } else {
        KEYBD_EVENT_FLAGS(0)
    };
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk.0),
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: extra_info,
            },
        },
    }
}

/// TSF モード用 INPUT 構造体を作成するヘルパー（TSF_MARKER 付き）。
///
/// [`make_scan_key_input`] を参照。
pub(crate) fn make_tsf_key_input(vk: awase::types::VkCode, is_keyup: bool) -> INPUT {
    make_scan_key_input(vk, is_keyup, TSF_MARKER)
}

/// INPUT 構造体を作成するヘルパー（dwExtraInfo 指定版）
pub(crate) const fn make_key_input_ex(
    vk: awase::types::VkCode,
    is_keyup: bool,
    extra_info: usize,
) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk.0),
                wScan: 0,
                dwFlags: if is_keyup {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_EVENT_FLAGS(0)
                },
                time: 0,
                dwExtraInfo: extra_info,
            },
        },
    }
}

/// ローマ字文字列に対応するひらがな文字を返す静的ルックアップ。
/// LazyLock で初回のみテーブルを構築し、以降は参照を返す。
pub(crate) fn kana_for_romaji_static(romaji: &str) -> Option<char> {
    use std::sync::LazyLock;
    static TABLE: LazyLock<awase::kana_table::KanaTable> =
        LazyLock::new(awase::kana_table::KanaTable::build);
    TABLE.kana_for_romaji(romaji)
}

/// WM_DRAIN_OUTPUT_QUEUE ハンドラから呼ぶ。
///
/// `RAW_TSF_LITERAL.backs` に退避されたバックスペース数を読み取り、SendInput で送信する。
/// `RAW_TSF_LITERAL.escape_composition` が立っていれば、バックスペースの前に
/// `VK_ESCAPE` を送って現在の composition を（何文字分かに関わらず）確実に破棄する
/// （partial literal 回収: candidate 表示中に一部だけ literal 化したケース）。
/// drain キーの SendInput より先に呼ぶことで WezTerm への到着順を保証する。
///
/// # Panics
/// `INPUT` のサイズが `i32` に収まらない場合（実際には起こらない）。
#[tracing::instrument(level = "debug")]
pub fn flush_raw_tsf_literal_backspaces() {
    use crate::vk::{VK_BACK, VK_ESCAPE};
    use std::sync::atomic::Ordering::Relaxed;
    let n = crate::RAW_TSF_LITERAL.backs.swap(0, Relaxed);
    let escape_composition = crate::RAW_TSF_LITERAL
        .escape_composition
        .swap(false, Relaxed);
    if n == 0 && !escape_composition {
        return;
    }
    let mut inputs = Vec::with_capacity(2 * (n + usize::from(escape_composition)));
    if escape_composition {
        inputs.push(make_key_input_ex(VK_ESCAPE, false, INJECTED_MARKER));
        inputs.push(make_key_input_ex(VK_ESCAPE, true, INJECTED_MARKER));
    }
    inputs.extend((0..n).flat_map(|_| {
        [
            make_key_input_ex(VK_BACK, false, INJECTED_MARKER),
            make_key_input_ex(VK_BACK, true, INJECTED_MARKER),
        ]
    }));
    tracing::debug!("[raw-tsf-literal] flush escape={escape_composition} backspace ×{n}");
    let _ = crate::win32::send_input_safe(&inputs);
}
