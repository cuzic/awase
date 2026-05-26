//! パニックリセット連打検出。
//!
//! `with_app` 再入中（フック再帰）でも確実に動作するよう、
//! ライブラリ層に配置してフックコールバックから直接呼べるようにする。

use crate::SingleThreadCell;

/// IME 関連キー押下のタイムスタンプ（循環バッファ）。
///
/// フックコールバックはメインスレッドで実行されるため `SingleThreadCell` で十分。
/// bootstrap で `.set(RapidPressTracker::new())` を呼ぶこと。
pub static RAPID_IME_TIMESTAMPS: SingleThreadCell<RapidPressTracker> = SingleThreadCell::new();

/// `ime_on` / `ime_off` 特殊キーに登録されたショートカットの combo リスト。
///
/// `may_change_ime` が拾わない VK_CONVERT(0x1C) / VK_NONCONVERT(0x1D) を含む
/// ユーザ設定のショートカット連打もパニック連打として扱うためのカスタムトリガー。
/// bootstrap および config reload で `set_panic_trigger_combos` を呼んで更新する。
///
/// VK 単体ではなく combo（vk + ctrl/shift/alt）で保持するのは、`変換` / `無変換`
/// 等の親指キーが NICOLA タイピング用としても使われる場合に、modifier 無しの
/// 単打を誤って panic 連打として数えてしまうのを防ぐため。
pub static PANIC_TRIGGER_COMBOS: SingleThreadCell<Vec<PanicTriggerCombo>> = SingleThreadCell::new();

/// パニック連打判定用のキー combo（`ParsedKeyCombo` のミラー）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PanicTriggerCombo {
    pub vk: u16,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

/// 連打検出用の軽量トラッカー
#[derive(Debug)]
pub struct RapidPressTracker {
    buf: [u64; 3],
    cursor: usize,
    count: usize,
}

impl Default for RapidPressTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl RapidPressTracker {
    const THRESHOLD: usize = 3;
    const WINDOW_MS: u64 = 1000;

    #[must_use] 
    pub const fn new() -> Self {
        Self {
            buf: [0; Self::THRESHOLD],
            cursor: 0,
            count: 0,
        }
    }

    /// タイムスタンプを記録し、連打が検出されたら `true` を返す。
    pub fn push(&mut self, now_ms: u64) -> bool {
        self.buf[self.cursor] = now_ms;
        self.cursor = (self.cursor + 1) % Self::THRESHOLD;
        if self.count < Self::THRESHOLD {
            self.count += 1;
        }
        if self.count < Self::THRESHOLD {
            return false;
        }
        let oldest = *self.buf.iter().min().unwrap_or(&0);
        now_ms.saturating_sub(oldest) < Self::WINDOW_MS
    }

    /// バッファをクリアする（発動後のリセット用）
    pub const fn clear(&mut self) {
        self.buf = [0; Self::THRESHOLD];
        self.cursor = 0;
        self.count = 0;
    }
}

/// IME 関連キー KeyDown をパニック連打カウンタに記録する。
///
/// `with_app` 再入中でも安全に呼べる。連打閾値に達したら `WM_PANIC_RESET` を post する。
pub fn record_ime_keydown(now_ms: u64) {
    // with_app 再入中でも安全に呼べる（try_with_mut は RefCell の借用を試み、
    // 失敗した場合は何もせず返る）。
    RAPID_IME_TIMESTAMPS.try_with_mut(|tracker| {
        if tracker.push(now_ms) {
            tracker.clear();
            log::warn!("Rapid IME key press detected — requesting panic reset");
            crate::win32::post_to_main_thread(crate::WM_PANIC_RESET);
        }
    });
}

/// `vk` + 現在の modifier 状態が、ユーザ設定の IME 制御ショートカット
/// （ime_on / ime_off）の combo と一致するかを判定する。bootstrap 前は常に false。
#[must_use]
pub fn is_panic_trigger(vk: u16, ctrl: bool, shift: bool, alt: bool) -> bool {
    PANIC_TRIGGER_COMBOS
        .with(|combos| {
            combos.iter().any(|c| {
                c.vk == vk && c.ctrl == ctrl && c.shift == shift && c.alt == alt
            })
        })
        .unwrap_or(false)
}

/// `ime_on` / `ime_off` 特殊キーの combo リストを更新する。
///
/// bootstrap 完了時と config reload 時に呼ぶ。重複は呼び出し側で除去すること。
pub fn set_panic_trigger_combos(combos: Vec<PanicTriggerCombo>) {
    PANIC_TRIGGER_COMBOS.set(combos);
}
