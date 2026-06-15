//! パニックリセット連打検出。
//!
//! `with_app` 再入中（フック再帰）でも確実に動作するよう、
//! ライブラリ層に配置してフックコールバックから直接呼べるようにする。

use crate::SingleThreadCell;

/// IME 関連キー押下の履歴（循環バッファ）。
///
/// フックコールバックはメインスレッドで実行されるため `SingleThreadCell` で十分。
/// bootstrap で `.set(RapidPressTracker::new())` を呼ぶこと。
pub static RAPID_IME_TIMESTAMPS: SingleThreadCell<RapidPressTracker> = SingleThreadCell::new();

/// `ime_on` / `ime_off` 特殊キーに登録されたショートカットの combo リスト。
///
/// VK 単体ではなく combo（vk + ctrl/shift/alt）で保持するのは、`変換` / `無変換`
/// 等の親指キーが NICOLA タイピング用としても使われる場合に、modifier 無しの
/// 単打を誤って panic シーケンスとして数えてしまうのを防ぐため。
pub static PANIC_TRIGGER_COMBOS: SingleThreadCell<Vec<PanicTriggerCombo>> = SingleThreadCell::new();

/// パニックリセット判定用のキー combo（`ParsedKeyCombo` のミラー）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PanicTriggerCombo {
    pub vk: awase::types::VkCode,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    /// true = IME ON ショートカット、false = IME OFF ショートカット
    pub is_on: bool,
}

/// シーケンス検出用の軽量トラッカー。
///
/// 「IME OFF → IME ON → IME OFF」の交互シーケンスが `WINDOW_MS` 以内に
/// 完結したときだけ `true` を返す。同一キーの連打では発動しない。
#[derive(Debug)]
pub struct RapidPressTracker {
    /// 直近3エントリ: (is_on, timestamp_ms)
    buf: [(bool, u64); 3],
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
    /// シーケンス全体の時間窓（ms）。意図的な操作には十分、偶発的連打では収まりにくい。
    const WINDOW_MS: u64 = 2000;

    #[must_use]
    pub const fn new() -> Self {
        Self {
            buf: [(false, 0); Self::THRESHOLD],
            cursor: 0,
            count: 0,
        }
    }

    /// キー押下を記録し、OFF→ON→OFF シーケンスが検出されたら `true` を返す。
    pub fn push(&mut self, is_on: bool, now_ms: u64) -> bool {
        self.buf[self.cursor] = (is_on, now_ms);
        self.cursor = (self.cursor + 1) % Self::THRESHOLD;
        if self.count < Self::THRESHOLD {
            self.count += 1;
        }
        if self.count < Self::THRESHOLD {
            return false;
        }
        // cursor は「次に書き込むスロット」= 最古エントリのインデックス
        let oldest = self.buf[self.cursor];
        let middle = self.buf[(self.cursor + 1) % Self::THRESHOLD];
        let newest = self.buf[(self.cursor + 2) % Self::THRESHOLD];
        // OFF → ON → OFF のシーケンスかつ全体が WINDOW_MS 以内
        let is_off_on_off = !oldest.0 && middle.0 && !newest.0;
        let within_window = now_ms.saturating_sub(oldest.1) < Self::WINDOW_MS;
        is_off_on_off && within_window
    }

    /// バッファをクリアする（発動後のリセット用）
    pub const fn clear(&mut self) {
        self.buf = [(false, 0); Self::THRESHOLD];
        self.cursor = 0;
        self.count = 0;
    }
}

/// IME ショートカット KeyDown をシーケンスカウンタに記録する。
///
/// `with_app` 再入中でも安全に呼べる。OFF→ON→OFF シーケンスが完結したら
/// `WM_PANIC_RESET` を post する。
pub fn record_ime_keydown(is_on: bool, now_ms: u64) {
    RAPID_IME_TIMESTAMPS.try_with_mut(|tracker| {
        if tracker.push(is_on, now_ms) {
            tracker.clear();
            log::warn!("Rapid IME key press detected — requesting panic reset");
            crate::win32::post_to_main_thread(crate::WM_PANIC_RESET);
        }
    });
}

/// `vk` + modifier が IME ON/OFF ショートカットに一致する場合、`is_on` 値を返す。
///
/// 一致しなければ `None`。bootstrap 前は常に `None`。
#[must_use]
pub fn get_panic_trigger_direction(
    vk: awase::types::VkCode,
    ctrl: bool,
    shift: bool,
    alt: bool,
) -> Option<bool> {
    PANIC_TRIGGER_COMBOS
        .with(|combos| {
            combos
                .iter()
                .find(|c| c.vk == vk && c.ctrl == ctrl && c.shift == shift && c.alt == alt)
                .map(|c| c.is_on)
        })
        .flatten()
}

/// `ime_on` / `ime_off` 特殊キーの combo リストを更新する。
///
/// bootstrap 完了時と config reload 時に呼ぶ。重複は呼び出し側で除去すること。
pub fn set_panic_trigger_combos(combos: Vec<PanicTriggerCombo>) {
    PANIC_TRIGGER_COMBOS.set(combos);
}
