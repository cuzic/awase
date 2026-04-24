//! IME 状態の観測 — `detect_ime_state()` を呼び出して Preconditions を直接更新する。
//!
//! ## 更新ポリシー
//!
//! `ImeSnapshot` の 3 フィールドはすべて `Option<bool>` で 3 値意味論を持つ:
//! - `Some(v)` = 検出成功 → `preconditions` を更新する
//! - `None`    = 不明（タイムアウト等） → **前回キャッシュ値を維持する**
//!
//! `None` を「偽」として扱ってはならない。

use crate::Preconditions;

/// IME_CMODE_ROMAN ビット（0x0010）
const IME_CMODE_ROMAN: u32 = 0x0010;
/// IME_CMODE_NATIVE ビット（0x0001）
const IME_CMODE_NATIVE: u32 = 0x0001;

/// Win32 API を使って IME 状態を観測し、`Preconditions` を直接更新する。
///
/// ## フィールドごとの更新ルール
///
/// ### `is_japanese_ime`
/// `Some(v)` のときのみ更新。`None`（タイムアウト等）は前回値を維持。
///
/// ### `ime_on`
/// 優先順位順に評価:
/// 1. `is_japanese_ime == Some(false)`: 非日本語KB確定 → `ime_on = false`（推測不要）
/// 2. `ime_on == Some(on)`: IME 状態検出成功 → `on && preconditions.is_japanese_ime`
///    （`is_japanese_ime` が `None` の場合はキャッシュ値を使用）
/// 3. `ime_force_on_guard`: awase が SSOT → 変更しない
/// 4. それ以外（検出失敗）: miss_count をインクリメント、`ime_on` は維持
///
/// ### `is_romaji`
/// `Some(romaji)` のときのみ更新。
/// `None` かつ `conversion_mode != 0` の場合: ROMAN ビット遷移でモード切替を検出。
/// それ以外: 前回値を維持。
///
/// # Safety
/// Win32 API を呼び出す。メインスレッドから呼ぶこと。
pub unsafe fn observe(preconditions: &mut Preconditions) {
    // detect_ime_state は複数のブロッキング IMM32 API を連鎖呼び出しするため、
    // ワーカースレッドでタイムアウト付き実行する（メッセージループハング防止）。
    let snap = crate::ime::detect_ime_state_with_timeout(std::time::Duration::from_millis(300));

    // ── is_japanese_ime: 検出成功時のみ更新 ──────────────────────────────────────
    // None（タイムアウト等）は「非日本語」ではなく「不明」なので前回値を維持する。
    if let Some(is_jp) = snap.is_japanese_ime {
        preconditions.is_japanese_ime = is_jp;
    }

    // ── ime_on ────────────────────────────────────────────────────────────────────
    // is_japanese_ime の確定値を使用するため、上の更新後に評価する。
    let known_not_japanese = snap.is_japanese_ime == Some(false);

    if known_not_japanese {
        // 非日本語KB確定: IME アクティブ不可
        preconditions.ime_on = false;
        preconditions.ime_detect_miss_count = 0;
        preconditions.ime_force_on_guard = false;
    } else if let Some(on) = snap.ime_on {
        // IME 状態検出成功: キャッシュ済みの is_japanese_ime と組み合わせる。
        // （snap.is_japanese_ime が None でもキャッシュ値は直前のループで維持済み）
        preconditions.ime_on = on && preconditions.is_japanese_ime;
        preconditions.ime_detect_miss_count = 0;
        preconditions.ime_force_on_guard = false;
    } else if preconditions.ime_force_on_guard {
        // 検出失敗かつガード中: awase が SSOT なので ime_on を変更しない。
        // miss_count もインクリメントしない（ガードが解除されるまで force-ON は再発火しない）。
        log::debug!(
            "IME detection failed but force_on_guard active, preserving ime_on={}",
            preconditions.ime_on
        );
    } else {
        // 検出失敗: カウンタをインクリメント。
        // ime_on 自体は変更しない（検出成功時に即座に正しい値に復帰できるよう維持）。
        // 閾値到達時は refresh_ime_state_cache が IME を強制 ON にする。
        preconditions.ime_detect_miss_count =
            preconditions.ime_detect_miss_count.saturating_add(1);
        if preconditions.ime_detect_miss_count == crate::IME_DETECT_MISS_THRESHOLD {
            log::warn!(
                "IME detection failed {} consecutive times, will force IME ON",
                preconditions.ime_detect_miss_count
            );
        }
    }

    // ── is_romaji ─────────────────────────────────────────────────────────────────
    // ガード中は変更しない（awase が SSOT）
    if preconditions.ime_force_on_guard && snap.is_romaji.is_none() {
        // ガード中かつ検出失敗: is_romaji を維持
    } else if let Some(romaji) = snap.is_romaji {
        let prev = preconditions.is_romaji;
        preconditions.is_romaji = romaji;
        if prev != romaji {
            log::info!(
                "IME input method changed: {} → {}",
                if !prev { "kana" } else { "romaji" },
                if !romaji { "kana" } else { "romaji" },
            );
        }
    } else if let Some(conv_mode) = snap.conversion_mode {
        // direct check が失敗し is_romaji=None の場合:
        // conversion_mode の ROMAN ビット変化で実際のモード切替を検出する。
        // - ROMAN あり → ROMAN なし: かな入力に切り替わった
        // - ROMAN なし → ROMAN あり: ローマ字入力に切り替わった
        // - 変化なし: 前回値を維持（Zoom 等、ROMAN を報告しないアプリ対応）
        let curr_has_roman = conv_mode & IME_CMODE_ROMAN != 0;
        let curr_has_native = conv_mode & IME_CMODE_NATIVE != 0;

        if let Some(prev_conv) = preconditions.prev_conversion_mode {
            let prev_had_roman = prev_conv & IME_CMODE_ROMAN != 0;
            if prev_had_roman != curr_has_roman && curr_has_native {
                // ROMAN ビットが実際に変化した → モード切替を検出
                let new_romaji = curr_has_roman;
                if preconditions.is_romaji != new_romaji {
                    log::info!(
                        "IME input method changed (ROMAN bit transition): {} → {}",
                        if !preconditions.is_romaji { "kana" } else { "romaji" },
                        if !new_romaji { "kana" } else { "romaji" },
                    );
                    preconditions.is_romaji = new_romaji;
                }
            }
        }
        // 変化なし、または前回値なし: 前回値を維持
    }

    // conversion_mode を記録（次回比較用）
    if let Some(conv_mode) = snap.conversion_mode {
        preconditions.prev_conversion_mode = Some(conv_mode);
    }

    log::debug!(
        "IME snapshot: japanese={:?} ime_on={:?} romaji={:?} conv={:?} guard={}",
        snap.is_japanese_ime,
        snap.ime_on,
        snap.is_romaji,
        snap.conversion_mode.map(|v| format!("0x{v:08X}")),
        preconditions.ime_force_on_guard,
    );
}
