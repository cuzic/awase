//! IME 状態の観測 — `detect_ime_state()` を呼び出して観測スナップショットを返す。
//!
//! ## 設計方針
//!
//! observer は観測値を `ImeObserverOutput` として返すのみで、`Preconditions` を直接
//! 変更しない。状態への反映は `PlatformState::apply_ime_observer_output()` に一元化。
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
use crate::ime_observations::ImeObs;

/// `observe()` / `compute_observer_output()` が返す観測結果。
///
/// `Preconditions` を直接変更せず、このスナップショットを返す。
/// 呼び出し元（`PlatformState::apply_ime_observer_output()`）が状態に反映する。
#[derive(Debug)]
pub struct ImeObserverOutput {
    /// 検出された is_japanese_ime（`Some` のときのみ更新すべき）
    pub is_japanese_ime: Option<bool>,
    /// `observer_poll` スロットに書くべき値（`Some` のときのみ書く）
    pub observer_poll: Option<ImeObs>,
    /// miss_count を 1 インクリメントすべきか
    pub increment_miss_count: bool,
    /// `ime_force_on_guard` を `Inactive` にリセットすべきか
    pub clear_force_on_guard: bool,
    /// `input_mode` に適用すべき新しい値（`Some` のときのみ更新すべき）
    pub new_input_mode: Option<InputModeState>,
    /// `prev_conversion_mode` に書くべき値（`Some` のときのみ更新すべき）
    pub new_prev_conversion_mode: Option<u32>,
    /// ログ用: TSF ネイティブウィンドウだったか
    pub is_tsf_native_skip: bool,
    /// ログ用: force_on_guard によりスキップされたか
    pub force_on_guard_skip: bool,
    /// ログ用: IME snap の raw フィールド（デバッグ用）
    pub snap_is_japanese_ime: Option<bool>,
    pub snap_ime_on: Option<bool>,
    pub snap_is_romaji: Option<bool>,
    pub snap_conversion_mode: Option<u32>,
    pub guard_was_active: bool,
}

/// `ImeSnapshot` と現在の `Preconditions` の読み取り専用ビューから観測結果を計算する。
///
/// `Preconditions` への書き込みを行わない純粋関数。
/// `observe()` と `apply_snapshot()` の共通ロジックを集約。
pub fn compute_observer_output(
    snap: &crate::ime::ImeSnapshot,
    now_ms: u64,
    // Preconditions の読み取り専用フィールド
    current_ime_on: bool,
    current_ime_force_on_guard: crate::ImeForceOnGuard,
    current_input_mode: InputModeState,
    current_prev_conversion_mode: Option<u32>,
) -> ImeObserverOutput {
    let known_not_japanese = snap.is_japanese_ime == Some(false);
    let guard_active = current_ime_force_on_guard.is_active();

    // ── observer_poll / miss_count / force_on_guard ──────────────────────────────
    let (observer_poll, increment_miss_count, clear_force_on_guard, is_tsf_native_skip, force_on_guard_skip) =
        if known_not_japanese {
            // 非日本語KB確定: IME アクティブ不可
            (Some(ImeObs { value: false, ms: now_ms }), false, true, false, false)
        } else if let Some(on) = snap.ime_on {
            // IME 状態検出成功
            (Some(ImeObs { value: on, ms: now_ms }), false, true, false, false)
        } else if snap.is_tsf_native {
            // TSF ネイティブウィンドウ: 検出不能だが miss_count を増やさない
            log::debug!(
                "IME detection skipped (TSF-native window), preserving ime_on={}",
                current_ime_on
            );
            (None, false, false, true, false)
        } else if guard_active {
            // 検出失敗かつガード中
            log::debug!(
                "IME detection failed but force_on_guard active, preserving ime_on={}",
                current_ime_on
            );
            (None, false, false, false, true)
        } else {
            // 検出失敗: miss_count をインクリメント
            (None, true, false, false, false)
        };

    // ── is_romaji / input_mode ────────────────────────────────────────────────────
    let new_input_mode = if guard_active && snap.is_romaji.is_none() {
        // ガード中かつ検出失敗: input_mode を維持
        None
    } else if let Some(romaji) = snap.is_romaji {
        let prev = current_input_mode.is_romaji_capable();
        let new_mode = if romaji {
            InputModeState::ObservedRomaji
        } else {
            InputModeState::ObservedKana
        };
        if prev != romaji {
            log::info!(
                "IME input method changed: {} → {}",
                if !prev { "kana" } else { "romaji" },
                if !romaji { "kana" } else { "romaji" },
            );
        }
        Some(new_mode)
    } else if let Some(conv_mode) = snap.conversion_mode {
        let curr_has_roman = conv_mode & IME_CMODE_ROMAN != 0;
        let curr_has_native = conv_mode & IME_CMODE_NATIVE != 0;

        if let Some(prev_conv) = current_prev_conversion_mode {
            let prev_had_roman = prev_conv & IME_CMODE_ROMAN != 0;
            if prev_had_roman != curr_has_roman && curr_has_native {
                let new_romaji = curr_has_roman;
                let prev_romaji = current_input_mode.is_romaji_capable();
                if prev_romaji != new_romaji {
                    log::info!(
                        "IME input method changed (ROMAN bit transition): {} → {}",
                        if !prev_romaji { "kana" } else { "romaji" },
                        if !new_romaji { "kana" } else { "romaji" },
                    );
                    Some(if new_romaji {
                        InputModeState::ObservedRomaji
                    } else {
                        InputModeState::ObservedKana
                    })
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    // conversion_mode を次回比較用に記録
    let new_prev_conversion_mode = snap.conversion_mode;

    ImeObserverOutput {
        is_japanese_ime: snap.is_japanese_ime,
        observer_poll,
        increment_miss_count,
        clear_force_on_guard,
        new_input_mode,
        new_prev_conversion_mode,
        is_tsf_native_skip,
        force_on_guard_skip,
        snap_is_japanese_ime: snap.is_japanese_ime,
        snap_ime_on: snap.ime_on,
        snap_is_romaji: snap.is_romaji,
        snap_conversion_mode: snap.conversion_mode,
        guard_was_active: guard_active,
    }
}

/// Win32 API を使って IME 状態を観測し、`ImeObserverOutput` を返す。
///
/// `Preconditions` を直接変更しない。呼び出し元が
/// `PlatformState::apply_ime_observer_output()` で状態に反映すること。
///
/// # Safety
/// Win32 API を呼び出す。メインスレッドから呼ぶこと。
pub unsafe fn observe(
    current_ime_on: bool,
    current_ime_force_on_guard: crate::ImeForceOnGuard,
    current_input_mode: InputModeState,
    current_prev_conversion_mode: Option<u32>,
) -> ImeObserverOutput {
    // detect_ime_state は複数のブロッキング IMM32 API を連鎖呼び出しするため、
    // ワーカースレッドでタイムアウト付き実行する（メッセージループハング防止）。
    let snap = crate::ime::detect_ime_state_with_timeout(std::time::Duration::from_millis(300));
    let now_ms = crate::hook::current_tick_ms();
    compute_observer_output(
        &snap,
        now_ms,
        current_ime_on,
        current_ime_force_on_guard,
        current_input_mode,
        current_prev_conversion_mode,
    )
}

/// IME スナップショットを `ImeObserverOutput` に変換する（純粋 sync）。
///
/// `observe()` から blocking fetch 部分を分離したもの。async drain 後に with_app 内で呼ぶ。
/// `Preconditions` を直接変更しない。
pub fn apply_snapshot(
    snap: &crate::ime::ImeSnapshot,
    now_ms: u64,
    current_ime_on: bool,
    current_ime_force_on_guard: crate::ImeForceOnGuard,
    current_input_mode: InputModeState,
    current_prev_conversion_mode: Option<u32>,
) -> ImeObserverOutput {
    compute_observer_output(
        snap,
        now_ms,
        current_ime_on,
        current_ime_force_on_guard,
        current_input_mode,
        current_prev_conversion_mode,
    )
}
