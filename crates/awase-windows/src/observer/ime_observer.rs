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
/// # Safety
/// Win32 API を呼び出す。メインスレッドから呼ぶこと。
pub unsafe fn observe(preconditions: &mut Preconditions) {
    let snap = crate::ime::detect_ime_state();

    // is_japanese_ime: always update (LANGID is reliable)
    preconditions.is_japanese_ime = snap.is_japanese_ime;

    // ime_on: update if detected, keep shadow if not
    if let Some(on) = snap.ime_on {
        preconditions.ime_on = on && snap.is_japanese_ime;
    } else if !snap.is_japanese_ime {
        preconditions.ime_on = false;
    }

    // is_romaji: update if detected, otherwise use conversion_mode transition
    if let Some(romaji) = snap.is_romaji {
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
        "IME snapshot: japanese={} ime_on={:?} romaji={:?} conv=0x{:08X}",
        snap.is_japanese_ime,
        snap.ime_on,
        snap.is_romaji,
        snap.conversion_mode
    );
}
