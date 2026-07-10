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

/// IME composition context がコールド状態になった理由（診断ログ用）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColdReason {
    /// フォーカス変更
    #[default]
    FocusChange,
    /// `ImeEffect::SetOpen(true)` 実行後
    SetOpenTrue,
    /// `ImeEffect::SetOpen(false)` 実行後（IME OFF → composition context 無効化）
    SetOpenFalse,
    /// 物理 F2 (VK_DBE_HIRAGANA) をフックで Consume（TSF モード）
    NativeF2Consumed,
    /// Space/Enter/Escape のパススルー
    PassthroughConfirmKey,
    /// Space/Enter/Escape の reinject
    ReinjectConfirmKey,
    /// 記号 VK 送信後（TSF context リセット可能性あり）
    SymbolVkSent,
    /// F2 non-TSF mode passthrough
    F2NonTsf,
    // 旧 SessionExpired は 2026-07-06 の到達不能パス監査で撤去 — production の
    // 構築サイトゼロ。session-expiry は `assess_warmth` の bool（session_expired）
    // 経由で F2 prepend バッチを駆動する別パスで実装されており、この ColdReason
    // variant で mark_composition_cold する経路は一度も配線されなかった。
    /// raw TSF literal 検出後のリカバリ（バックスペース後に再 cold 扱い）
    RawTsfLiteralRecovery,
    /// Ctrl+key パススルー時の composition キャンセル（IME ショートカット横取り防止）
    CtrlKeyBypass,
}

impl ColdReason {
    /// warmup 後の eager settle 待機時間 (ms)。long_idle=true のとき延長。
    #[must_use]
    pub const fn eager_settle_ms(self, long_idle: bool) -> u64 {
        match self {
            Self::FocusChange | Self::SetOpenTrue | Self::NativeF2Consumed => {
                if long_idle {
                    2000
                } else {
                    1500
                }
            }
            Self::PassthroughConfirmKey | Self::ReinjectConfirmKey => {
                if long_idle {
                    1500
                } else {
                    500
                }
            }
            Self::SymbolVkSent
            | Self::F2NonTsf
            | Self::RawTsfLiteralRecovery
            | Self::CtrlKeyBypass
            | Self::SetOpenFalse => 500,
        }
    }

    /// VK_DBE_HIRAGANA 送信後の最小待機時間 (ms)（GJI I/O 観測開始前の固定待機）
    #[must_use]
    pub const fn probe_min_ms(self, long_idle: bool) -> u64 {
        match self {
            Self::FocusChange | Self::SetOpenTrue | Self::NativeF2Consumed => {
                if long_idle {
                    300
                } else {
                    // short_idle: LiteralDetect が安全網になるため短縮可
                    100
                }
            }
            Self::PassthroughConfirmKey | Self::ReinjectConfirmKey => {
                if long_idle {
                    300
                } else {
                    50
                }
            }
            Self::SymbolVkSent => 30,
            Self::F2NonTsf | Self::RawTsfLiteralRecovery | Self::SetOpenFalse => 100,
            Self::CtrlKeyBypass => 50,
        }
    }

    /// confirmation キー（composition 確定/キャンセル）かどうか
    #[must_use]
    pub const fn is_confirm_key(self) -> bool {
        matches!(self, Self::PassthroughConfirmKey | Self::ReinjectConfirmKey)
    }

    /// fresh F2 再送 + settle が必要かどうか（IME 初期化系 cold reason）
    #[must_use]
    pub const fn requires_settle(self) -> bool {
        matches!(
            self,
            Self::FocusChange | Self::NativeF2Consumed | Self::SetOpenTrue
        )
    }
}

/// TSF モード用 INPUT 構造体を作成するヘルパー（TSF_MARKER 付き）
///
/// `wVk` を保持したまま `MapVirtualKeyW` で算出した `wScan` も設定する。
/// `KEYEVENTF_SCANCODE` は付加しない（付加すると WezTerm が LLKHF_SCANCODE フラグ付き
/// キーとして検出し IME をバイパスしてしまうため）。
pub(crate) fn make_tsf_key_input(vk: awase::types::VkCode, is_keyup: bool) -> INPUT {
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
                dwExtraInfo: TSF_MARKER,
            },
        },
    }
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
pub fn flush_raw_tsf_literal_backspaces() {
    use crate::vk::{VK_BACK, VK_ESCAPE};
    use std::sync::atomic::Ordering::Relaxed;
    let n = crate::RAW_TSF_LITERAL.backs.swap(0, Relaxed);
    let escape_composition = crate::RAW_TSF_LITERAL.escape_composition.swap(false, Relaxed);
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
    log::debug!("[raw-tsf-literal] flush escape={escape_composition} backspace ×{n}");
    let _ = crate::win32::send_input_safe(&inputs);
}
