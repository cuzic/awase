//! IME 状態の観測 — `detect_ime_state()` を呼び出して Preconditions を直接更新する。

use crate::Preconditions;

/// IME_CMODE_ROMAN ビット（0x0010）
const IME_CMODE_ROMAN: u32 = 0x0010;
/// IME_CMODE_NATIVE ビット（0x0001）
const IME_CMODE_NATIVE: u32 = 0x0001;

/// Win32 API を使って IME 状態を観測し、`Preconditions` を直接更新する。
///
/// `is_romaji` の判定:
/// - `detect_ime_state()` が `Some(...)` を返した場合はそのまま適用。
/// - `None` を返した場合（direct check 失敗かつ ROMAN ビットなし）は
///   `conversion_mode` の ROMAN ビット変化で実際のかな切替を検出する。
///   変化がなければ前回値を維持する（Zoom 等のアプリ対応）。
///
/// `ime_force_on_guard` が `true` の場合（awase が SSOT）:
/// - 検出成功時のみガードを解除して OS 側の SSOT に戻る。
/// - 検出失敗時は `ime_on` / `is_romaji` を変更しない。
///
/// # Safety
/// Win32 API を呼び出す。メインスレッドから呼ぶこと。
pub unsafe fn observe(preconditions: &mut Preconditions) {
    // detect_ime_state は複数のブロッキング IMM32 API を連鎖呼び出しするため、
    // ワーカースレッドでタイムアウト付き実行する（メッセージループハング防止）。
    let snap = crate::ime::detect_ime_state_with_timeout(std::time::Duration::from_millis(300));

    // is_japanese_ime: always update (LANGID is reliable)
    preconditions.is_japanese_ime = snap.is_japanese_ime;

    // ime_on: update if detected, fallback on repeated failure
    if let Some(on) = snap.ime_on {
        // 検出成功: OS が SSOT に戻る
        preconditions.ime_on = on && snap.is_japanese_ime;
        preconditions.ime_detect_miss_count = 0;
        preconditions.ime_force_on_guard = false;
    } else if !snap.is_japanese_ime {
        // 日本語 IME でない: 確実に OFF
        preconditions.ime_on = false;
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

    // is_romaji: update if detected, otherwise use conversion_mode transition
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
    } else if snap.conversion_mode != 0 {
        // direct check が失敗し is_romaji=None の場合:
        // conversion_mode の ROMAN ビット変化で実際のモード切替を検出する。
        // - ROMAN あり → ROMAN なし: かな入力に切り替わった
        // - ROMAN なし → ROMAN あり: ローマ字入力に切り替わった
        // - 変化なし: 前回値を維持（Zoom 等、ROMAN を報告しないアプリ対応）
        let prev_conv = preconditions.prev_conversion_mode;
        let prev_had_roman = prev_conv & IME_CMODE_ROMAN != 0;
        let curr_has_roman = snap.conversion_mode & IME_CMODE_ROMAN != 0;
        let curr_has_native = snap.conversion_mode & IME_CMODE_NATIVE != 0;

        if prev_conv != 0 && prev_had_roman != curr_has_roman && curr_has_native {
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
        // 変化なし: 前回値を維持
    }

    // conversion_mode を記録（次回比較用）
    if snap.conversion_mode != 0 {
        preconditions.prev_conversion_mode = snap.conversion_mode;
    }

    log::debug!(
        "IME snapshot: japanese={} ime_on={:?} romaji={:?} conv=0x{:08X} guard={}",
        snap.is_japanese_ime,
        snap.ime_on,
        snap.is_romaji,
        snap.conversion_mode,
        preconditions.ime_force_on_guard,
    );
}
