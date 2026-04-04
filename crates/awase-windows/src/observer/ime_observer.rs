//! IME 状態の観測 — `detect_ime_state()` を呼び出して Preconditions を直接更新する。

use crate::Preconditions;

/// Win32 API を使って IME 状態を観測し、`Preconditions` を直接更新する。
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

    // is_romaji: update if detected, keep current if not
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
    }

    log::debug!(
        "IME snapshot: japanese={} ime_on={:?} romaji={:?} conv=0x{:08X}",
        snap.is_japanese_ime,
        snap.ime_on,
        snap.is_romaji,
        snap.conversion_mode
    );
}
