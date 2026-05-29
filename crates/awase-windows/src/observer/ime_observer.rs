//! IME 状態の観測 — `read_ime_state_full()` を呼び出して観測スナップショットを返す。
//!
//! ## 設計方針
//!
//! observer は観測値を `ImeUpdate` として返すのみで、`Preconditions` を直接
//! 変更しない。状態への反映は `PlatformState::apply_ime_update()` に一元化。
//!
//! ## 更新ポリシー
//!
//! `ImeSnapshot` の 3 フィールドはすべて `Option<bool>` で 3 値意味論を持つ:
//! - `Some(v)` = 検出成功 → 呼び出し元が `Preconditions` を更新する
//! - `None`    = 不明（タイムアウト等） → **前回キャッシュ値を維持する**
//!
//! `None` を「偽」として扱ってはならない。

use awase::engine::InputModeState;
use crate::imm::{IME_CMODE_NATIVE, IME_CMODE_ROMAN};

/// Observer が返す単一観測 (値 + タイムスタンプ)。
#[derive(Debug, Clone, Copy)]
pub struct ImeObs {
    pub value: bool,
    pub ms: u64,
}

/// `classify_ime_snapshot()` が返す状態更新命令。
///
/// 副作用なし・純粋変換の結果を表す。
/// 呼び出し元（`PlatformState::apply_ime_update()`）が状態に反映する。
#[derive(Debug)]
pub struct ImeUpdate {
    /// 検出された is_japanese_ime（`Some` のときのみ更新すべき）
    pub is_japanese_ime: Option<bool>,
    /// `observer_poll` スロットに書くべき値（`Some` のときのみ書く）
    pub observer_poll: Option<ImeObs>,
    /// miss_count を 1 インクリメントすべきか
    pub increment_miss_count: bool,
    /// `force_on_broken_app_bootstrap` フラグをリセットすべきか（検出成功時）
    pub clear_force_on_broken_app_bootstrap: bool,
    /// `force_on_panic_reset` フラグと miss_count をリセットすべきか（検出成功時）
    pub clear_force_on_panic_reset: bool,
    /// `input_mode` に適用すべき新しい値（`Some` のときのみ更新すべき）
    pub new_input_mode: Option<InputModeState>,
    /// `prev_conversion_mode` に書くべき値（`Some` のときのみ更新すべき）
    pub new_prev_conversion_mode: Option<u32>,
}

/// observer_poll/miss_count/force_on_guard の更新方針
struct PollOutcome {
    observer_poll: Option<ImeObs>,
    increment_miss_count: bool,
    clear_force_on_broken_app_bootstrap: bool,
    clear_force_on_panic_reset: bool,
}

impl crate::ime::ImeSnapshot {
    fn classify_poll_outcome(
        &self,
        now_ms: u64,
        current_ime_on: bool,
        guard_active: bool,
    ) -> PollOutcome {
        let known_not_japanese = self.is_japanese_ime == Some(false);
        if known_not_japanese {
            PollOutcome {
                observer_poll: Some(ImeObs { value: false, ms: now_ms }),
                increment_miss_count: false,
                clear_force_on_broken_app_bootstrap: true,
                clear_force_on_panic_reset: true,
            }
        } else if let Some(on) = self.ime_on {
            PollOutcome {
                observer_poll: Some(ImeObs { value: on, ms: now_ms }),
                increment_miss_count: false,
                clear_force_on_broken_app_bootstrap: true,
                clear_force_on_panic_reset: true,
            }
        } else if self.is_tsf_native {
            log::debug!(
                "IME detection skipped (TSF-native window), preserving ime_on={current_ime_on}"
            );
            PollOutcome { observer_poll: None, increment_miss_count: false,
                clear_force_on_broken_app_bootstrap: false, clear_force_on_panic_reset: false }
        } else if guard_active {
            log::debug!(
                "IME detection failed but force_on_guard active, preserving ime_on={current_ime_on}"
            );
            PollOutcome { observer_poll: None, increment_miss_count: false,
                clear_force_on_broken_app_bootstrap: false, clear_force_on_panic_reset: false }
        } else {
            PollOutcome { observer_poll: None, increment_miss_count: true,
                clear_force_on_broken_app_bootstrap: false, clear_force_on_panic_reset: false }
        }
    }

    fn input_mode_from_romaji_flag(&self, current_input_mode: InputModeState) -> Option<InputModeState> {
        let romaji = self.is_romaji?;
        let prev = current_input_mode.is_romaji_capable();
        if prev != romaji {
            log::info!(
                "IME input method changed: {} → {}",
                if prev { "romaji" } else { "kana" },
                if romaji { "romaji" } else { "kana" },
            );
        }
        Some(if romaji { InputModeState::ObservedRomaji } else { InputModeState::ObservedKana })
    }

    fn input_mode_from_conversion(
        &self,
        current_prev_conversion_mode: Option<u32>,
        current_input_mode: InputModeState,
    ) -> Option<InputModeState> {
        let conv_mode = self.conversion_mode?;
        let curr_has_roman = conv_mode & IME_CMODE_ROMAN != 0;
        let curr_has_native = conv_mode & IME_CMODE_NATIVE != 0;
        let prev_conv = current_prev_conversion_mode?;
        let prev_had_roman = prev_conv & IME_CMODE_ROMAN != 0;
        if !(prev_had_roman != curr_has_roman && curr_has_native) {
            return None;
        }
        let new_romaji = curr_has_roman;
        let prev_romaji = current_input_mode.is_romaji_capable();
        if prev_romaji == new_romaji {
            return None;
        }
        log::info!(
            "IME input method changed (ROMAN bit transition): {} → {}",
            if prev_romaji { "romaji" } else { "kana" },
            if new_romaji { "romaji" } else { "kana" },
        );
        Some(if new_romaji { InputModeState::ObservedRomaji } else { InputModeState::ObservedKana })
    }
}

/// `ImeSnapshot` と現在の `Preconditions` の読み取り専用ビューから更新命令を計算する。
///
/// `Preconditions` への書き込みを一切行わない純粋関数。
/// `poll_and_classify_ime()` と `classify_fetched_snapshot()` の共通ロジックを集約。
#[must_use] 
pub fn classify_ime_snapshot(
    snap: &crate::ime::ImeSnapshot,
    now_ms: u64,
    // Preconditions の読み取り専用フィールド
    current_ime_on: bool,
    current_force_on_guard_active: bool,
    current_input_mode: InputModeState,
    current_prev_conversion_mode: Option<u32>,
) -> ImeUpdate {
    let guard_active = current_force_on_guard_active;
    let poll = snap.classify_poll_outcome(now_ms, current_ime_on, guard_active);

    let new_input_mode = if guard_active && snap.is_romaji.is_none() {
        None
    } else {
        snap.input_mode_from_romaji_flag(current_input_mode)
            .or_else(|| snap.input_mode_from_conversion(current_prev_conversion_mode, current_input_mode))
    };

    log::debug!(
        "IME snapshot: japanese={:?} ime_on={:?} romaji={:?} conv={:?} guard={}",
        snap.is_japanese_ime,
        snap.ime_on,
        snap.is_romaji,
        snap.conversion_mode.map(|v| format!("0x{v:08X}")),
        guard_active,
    );

    ImeUpdate {
        is_japanese_ime: snap.is_japanese_ime,
        observer_poll: poll.observer_poll,
        increment_miss_count: poll.increment_miss_count,
        clear_force_on_broken_app_bootstrap: poll.clear_force_on_broken_app_bootstrap,
        clear_force_on_panic_reset: poll.clear_force_on_panic_reset,
        new_input_mode,
        new_prev_conversion_mode: snap.conversion_mode,
    }
}

/// Win32 API を使って IME 状態を観測し、`ImeUpdate` を返す。
///
/// `Preconditions` を直接変更しない。呼び出し元が
/// `PlatformState::apply_ime_update()` で状態に反映すること。
///
/// # Safety
/// Win32 API を呼び出す。メインスレッドから呼ぶこと。
#[must_use] 
pub unsafe fn poll_and_classify_ime(
    current_ime_on: bool,
    current_force_on_guard_active: bool,
    current_input_mode: InputModeState,
    current_prev_conversion_mode: Option<u32>,
) -> ImeUpdate {
    // read_ime_state_full は複数のブロッキング IMM32 API を連鎖呼び出しするため、
    // ワーカースレッドでタイムアウト付き実行する（メッセージループハング防止）。
    let snap = crate::ime::read_ime_state_full_with_timeout(std::time::Duration::from_millis(300));
    let now_ms = crate::hook::current_tick_ms();
    classify_ime_snapshot(
        &snap,
        now_ms,
        current_ime_on,
        current_force_on_guard_active,
        current_input_mode,
        current_prev_conversion_mode,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ime::ImeSnapshot;
    use awase::engine::InputModeState;

    fn default_snap() -> ImeSnapshot {
        ImeSnapshot {
            is_japanese_ime: Some(true),
            ime_on: None,
            is_romaji: None,
            conversion_mode: None,
            is_tsf_native: false,
        }
    }

    /// ケース 1: 日本語 IME + IME ON → observer_poll に Some(true) が記録される
    #[test]
    fn classify_returns_observer_poll_true_for_japanese_ime_on() {
        let snap = ImeSnapshot {
            is_japanese_ime: Some(true),
            ime_on: Some(true),
            ..default_snap()
        };
        let update = classify_ime_snapshot(
            &snap,
            1000,
            false, // current_ime_on
            false, // current_force_on_guard_active
            InputModeState::Unknown,
            None,
        );
        assert!(update.observer_poll.is_some());
        assert_eq!(update.observer_poll.unwrap().value, true);
    }

    /// ケース 2: 日本語 IME + IME OFF → observer_poll に Some(false)
    #[test]
    fn classify_returns_observer_poll_false_for_japanese_ime_off() {
        let snap = ImeSnapshot {
            is_japanese_ime: Some(true),
            ime_on: Some(false),
            ..default_snap()
        };
        let update = classify_ime_snapshot(
            &snap,
            1000,
            true,  // current_ime_on
            false, // current_force_on_guard_active
            InputModeState::Unknown,
            None,
        );
        assert!(update.observer_poll.is_some());
        assert_eq!(update.observer_poll.unwrap().value, false);
    }

    /// ケース 3: 非日本語 IME → observer_poll に Some(false)（non-Japanese → IME 不活性）
    #[test]
    fn classify_returns_observer_poll_false_for_non_japanese_ime() {
        let snap = ImeSnapshot {
            is_japanese_ime: Some(false),
            ime_on: None,
            ..default_snap()
        };
        let update = classify_ime_snapshot(
            &snap,
            1000,
            true,  // current_ime_on
            false, // current_force_on_guard_active
            InputModeState::Unknown,
            None,
        );
        // known_not_japanese → (Some(false), false, true, true)
        assert!(update.observer_poll.is_some());
        assert_eq!(update.observer_poll.unwrap().value, false);
        assert!(!update.increment_miss_count);
        assert!(update.clear_force_on_broken_app_bootstrap);
        assert!(update.clear_force_on_panic_reset);
    }

    /// ケース 4: IME 検出失敗（is_japanese_ime: None, not tsf, no guard) → miss_count インクリメント
    #[test]
    fn classify_increments_miss_count_on_detection_failure() {
        let snap = ImeSnapshot {
            is_japanese_ime: None,
            ime_on: None,
            is_romaji: None,
            conversion_mode: None,
            is_tsf_native: false,
        };
        let update = classify_ime_snapshot(
            &snap,
            1000,
            false, // current_ime_on
            false, // current_force_on_guard_active（ガードなし）
            InputModeState::Unknown,
            None,
        );
        assert!(update.increment_miss_count);
        assert!(update.observer_poll.is_none());
    }

    /// ケース 5: force_on_guard アクティブ時 → observer_poll が None、miss_count も増えない
    #[test]
    fn classify_skips_observer_poll_when_force_on_guard_active() {
        let snap = ImeSnapshot {
            is_japanese_ime: Some(true),
            ime_on: None, // 検出失敗
            ..default_snap()
        };
        let update = classify_ime_snapshot(
            &snap,
            1000,
            true,  // current_ime_on
            true,  // current_force_on_guard_active = true
            InputModeState::Unknown,
            None,
        );
        assert!(update.observer_poll.is_none());
        assert!(!update.increment_miss_count);
        assert!(!update.clear_force_on_broken_app_bootstrap);
        assert!(!update.clear_force_on_panic_reset);
    }

    /// ケース 6: TSF ネイティブウィンドウ → observer_poll が None、miss_count も増えない
    #[test]
    fn classify_skips_observer_poll_for_tsf_native_window() {
        let snap = ImeSnapshot {
            is_japanese_ime: Some(true),
            ime_on: None, // TSF なので取得不能
            is_tsf_native: true,
            ..default_snap()
        };
        let update = classify_ime_snapshot(
            &snap,
            1000,
            true,  // current_ime_on
            false, // current_force_on_guard_active
            InputModeState::Unknown,
            None,
        );
        assert!(update.observer_poll.is_none());
        assert!(!update.increment_miss_count);
        assert!(!update.clear_force_on_broken_app_bootstrap);
        assert!(!update.clear_force_on_panic_reset);
    }
}

/// IME スナップショットを `ImeUpdate` に変換する（純粋 sync）。
///
/// `poll_and_classify_ime()` から blocking fetch 部分を分離したもの。async drain 後に with_app 内で呼ぶ。
/// `Preconditions` を直接変更しない。
#[must_use] 
pub fn classify_fetched_snapshot(
    snap: &crate::ime::ImeSnapshot,
    now_ms: u64,
    current_ime_on: bool,
    current_force_on_guard_active: bool,
    current_input_mode: InputModeState,
    current_prev_conversion_mode: Option<u32>,
) -> ImeUpdate {
    classify_ime_snapshot(
        snap,
        now_ms,
        current_ime_on,
        current_force_on_guard_active,
        current_input_mode,
        current_prev_conversion_mode,
    )
}
