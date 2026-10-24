//! `--mode=langdetect`(ADR-280 C0 の配線確認): `[auto_detect] eisu_threshold` が実際の awase.exe の判定器へ届くかを、
//! 固定の合成列を打って awase.log の `[lang-detect]` 記録で確かめる。判定は check_langdetect.py(本文は読まない)。
//!
//! 1 試行 = IME を ON にそろえる → 無操作を `--ld-idle=MS`(既定 3800。`idle_ms` 既定 3000 を超える)置く → 列を打つ → `--ld-wait=MS`
//! (既定 4000。`max_ms` 既定 2500 を超える)待つ。列は次の 3 種類(親指なしの未シフト面、Space を含まない):
//!  - `B`(境界): `ayre`。llr = 1.5741 → -4.6360 → -8.5266 → -14.9978。閾値 14 なら 4 打目で NotJapanese、16 なら決まらない。
//!  - `E`(英数): `build`。llr = -6.4998 → -15.6293 → -19.4536。閾値 14 でも 16 でも NotJapanese。
//!  - `J`(日本語): `k` を押している間に左親指(無変換)を重ねる(文字先行の親指との重なり)。どの閾値でも Japanese。
//! 記録は `langdetect` 行(試行の開始・終了の UTC 時刻。check_langdetect.py が awase.log を時刻で切り出す)。

use serde_json::json;
use windows::Win32::Foundation::HWND;

use crate::{
    arg_value, clear_text, focus_ok, press, rec, refocus, send_key, sleep_ms, turn_ime_on, utc_hms,
    VK_RETURN,
};

const VK_MUHENKAN: u32 = 0x1D;
const SCAN_MUHENKAN: u16 = 0x7B;

fn scan_of(vk: u32) -> u16 {
    awase_vkmap::vk_to_pos(awase::types::VkCode(u16::try_from(vk).unwrap_or(0)))
        .and_then(|p| awase_windows::scanmap::pos_to_scan(awase::scanmap::KeyboardModel::Jis, p))
        .and_then(|s| u16::try_from(s.0).ok())
        .unwrap_or(0)
}

fn type_letters(word: &str, interval_ms: u64) {
    for ch in word.bytes() {
        let vk = u32::from(ch.to_ascii_uppercase());
        press(vk, scan_of(vk), 20);
        sleep_ms(interval_ms);
    }
}

pub(crate) fn langdetect_scenario(child: HWND) {
    let trials: usize = arg_value("--trials=")
        .and_then(|v| v.parse().ok())
        .unwrap_or(2);
    let idle: u64 = arg_value("--ld-idle=")
        .and_then(|v| v.parse().ok())
        .unwrap_or(3800);
    let wait: u64 = arg_value("--ld-wait=")
        .and_then(|v| v.parse().ok())
        .unwrap_or(4000);
    let iv: u64 = arg_value("--interval=")
        .and_then(|v| v.parse().ok())
        .unwrap_or(40);
    rec(
        &json!({"type":"langdetect_config","trials":trials,"idle_ms":idle,"wait_ms":wait,"interval_ms":iv}),
    );
    for n in 0..trials {
        for (kind, keys) in [("B", "ayre"), ("E", "build"), ("J", "k")] {
            if !focus_ok() {
                refocus();
            }
            if !focus_ok() {
                rec(
                    &json!({"type":"abort","reason":format!("langdetect 試行前にフォーカスが外れた n={n} kind={kind}")}),
                );
                return;
            }
            turn_ime_on(0);
            clear_text(child);
            sleep_ms(idle);
            let t0 = utc_hms();
            if kind == "J" {
                // 文字を先に押し、押している間に左親指を重ねる(NICOLA の文字先行の同時打鍵)。親指を先に押すと、
                // 親指の Down が無操作の長さをリセットして、文字がエピソードの開始(idle)にならない(初回 CI: 記録 0 件)。
                let vk = u32::from(keys.as_bytes()[0].to_ascii_uppercase());
                send_key(vk, scan_of(vk), true);
                sleep_ms(30);
                send_key(VK_MUHENKAN, SCAN_MUHENKAN, true);
                sleep_ms(60);
                send_key(vk, scan_of(vk), false);
                sleep_ms(30);
                send_key(VK_MUHENKAN, SCAN_MUHENKAN, false);
            } else {
                type_letters(keys, iv);
            }
            sleep_ms(wait);
            let t1 = utc_hms();
            // 次の試行へ持ち越さないよう、入力中の文字列を確定して消す(本文は記録しない)。
            press(VK_RETURN, 0x1C, 50);
            sleep_ms(300);
            clear_text(child);
            rec(&json!({"type":"langdetect","n":n,"kind":kind,"keys":keys,"start":t0,"end":t1}));
        }
    }
}
