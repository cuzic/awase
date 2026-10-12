//! `--mode=lateorder`(ADR-255 段階3b: 遅いルール `[[keymap]] ime = "off"` の高速打鍵・押しっぱなしの検証):
//! IME OFF にして、無変換(`--lateorder-key=1D`、既定)を Space にする遅いルールを、速い連続打鍵と自動リピートで試す。
//! 対照は `--lateorder-key=20`(物理 Space を直接注入。ルールなし・awase なしでも同じ期待になる)。
//!
//! 1 試行 = IME OFF → 本文を消す → 次の2種類を順に:
//!  - `burst`: 「foo」+ key +「bar」を `--reps=N`(既定 5)回、キー間隔 `--interval=MS`(既定 20)で連続して打つ。
//!    期待は `foo bar` を N 回並べた文字列(Space の位置が先行・後続の文字と入れ替わらない=R7-M1)。
//!  - `hold`: key を押したまま KeyDown を 6 回(33ms 間隔、自動リピート相当)送ってから離し、続けて `x` を打つ。
//!    期待は ` x`(Space が 1 個だけ。押されっぱなしなら `x` の後ろに空白が続く、リピートが漏れれば空白が複数)。
//!    対照(key=20)は OS の自動リピートの扱いに依存するので、この種別の期待は遅いルールのときだけ意味を持つ。
//! 追加(`--lateorder-comp`、既定オフ。composition の有無と遅いルールの発動条件 = ADR-255 決定2-6 の実機確認用):
//!  - `comp_off`: IME ON → `a` を 1 打(未確定文字が出る)→ 物理 OFF(`VK_IME_OFF`)→ key を 1 打。未確定は OFF で確定し、
//!    その後の key が遅いルールで Space になるはず。期待は「(確定文字) + ` `」(確定文字の中身は問わず、末尾が Space で先頭側が空でない)。
//!    未確定の印(composing フラグ)が確定後も残ると Space が出ず、確定文字だけになる。
//!  - `comp_on`: IME ON → `a` を 1 打(未確定文字)→ key を 1 打(belief は ON なので遅いルールは当たらない)→ Enter。
//!    期待は Space が増えないこと(IME の ON 側の扱いのまま)。対照として本文を記録する。
//! 記録は `lateorder` 行(判定は check_lateorder.py。`comp_off`/`comp_on` は `--lateorder-comp` のときだけ出る行で、目視で読む)。

use serde_json::json;
use windows::Win32::Foundation::HWND;

use crate::{
    arg_value, clear_text, focus_ok, press, read_text_maybe_settled, rec, refocus, send_key,
    sleep_ms, turn_ime_on, utc_hms, VK_RETURN,
};

const VK_IME_OFF: u32 = 0x1A;
/// (VK, scan)。スキャンコードは JIS 配列の物理位置。
const KEY_F: (u32, u16) = (0x46, 0x21);
const KEY_O: (u32, u16) = (0x4F, 0x18);
const KEY_B: (u32, u16) = (0x42, 0x30);
const KEY_A: (u32, u16) = (0x41, 0x1E);
const KEY_R: (u32, u16) = (0x52, 0x13);
const KEY_X: (u32, u16) = (0x58, 0x2D);

fn scan_of(vk: u32) -> u16 {
    match vk {
        0x1D => 0x7B, // 無変換
        0x1C => 0x79, // 変換
        0x20 => 0x39, // Space
        _ => 0,
    }
}

pub(crate) fn lateorder_scenario(child: HWND) {
    let trials: usize = arg_value("--trials=")
        .and_then(|v| v.parse().ok())
        .unwrap_or(4);
    let reps: usize = arg_value("--reps=")
        .and_then(|v| v.parse().ok())
        .unwrap_or(5);
    let iv: u64 = arg_value("--interval=")
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);
    let key = arg_value("--lateorder-key=")
        .and_then(|v| u32::from_str_radix(v.trim_start_matches("0x"), 16).ok())
        .unwrap_or(0x1D);
    let kscan = scan_of(key);
    let settle = crate::has_flag("--settle-read");
    let comp = crate::has_flag("--lateorder-comp");
    rec(
        &json!({"type":"lateorder_config","comp":comp,"key":format!("{key:#x}"),"reps":reps,"interval_ms":iv,"trials":trials}),
    );
    for n in 0..trials {
        if !focus_ok() {
            refocus();
        }
        if !focus_ok() {
            rec(
                &json!({"type":"abort","reason":format!("lateorder 試行前にフォーカスが外れた n={n}")}),
            );
            return;
        }
        // --- burst ---
        press(VK_IME_OFF, 0x70, 50);
        sleep_ms(800);
        clear_text(child);
        sleep_ms(300);
        let t0 = utc_hms();
        for _ in 0..reps {
            for (vk, scan) in [KEY_F, KEY_O, KEY_O, (key, kscan), KEY_B, KEY_A, KEY_R] {
                press(vk, scan, 12);
                sleep_ms(iv);
            }
            sleep_ms(iv * 3);
        }
        sleep_ms(1500);
        let text = read_text_maybe_settled(child, settle);
        rec(
            &json!({"type":"lateorder","n":n,"kind":"burst","utc":t0,"interval_ms":iv,
            "expect":"foo bar".repeat(reps),"text":text}),
        );
        // --- hold(自動リピート) ---
        press(VK_IME_OFF, 0x70, 50);
        sleep_ms(800);
        clear_text(child);
        sleep_ms(300);
        let t0 = utc_hms();
        for _ in 0..6 {
            send_key(key, kscan, true);
            sleep_ms(33);
        }
        send_key(key, kscan, false);
        sleep_ms(500);
        press(KEY_X.0, KEY_X.1, 12);
        sleep_ms(1500);
        let text = read_text_maybe_settled(child, settle);
        rec(&json!({"type":"lateorder","n":n,"kind":"hold","utc":t0,"expect":" x","text":text}));
        if comp {
            // --- comp_off: 未確定 → 物理 OFF(確定)→ key。Space が出るはず ---
            turn_ime_on(0);
            clear_text(child);
            sleep_ms(300);
            let t0 = utc_hms();
            press(KEY_A.0, KEY_A.1, 12);
            sleep_ms(400);
            press(VK_IME_OFF, 0x70, 50);
            sleep_ms(1000);
            press(key, kscan, 12);
            sleep_ms(1500);
            let text = read_text_maybe_settled(child, settle);
            rec(&json!({"type":"lateorder","n":n,"kind":"comp_off","utc":t0,
                "expect":"(確定文字)+空白","text":text}));
            // --- comp_on: 未確定 → key(belief は ON、遅いルールは当たらない)→ Enter ---
            turn_ime_on(0);
            clear_text(child);
            sleep_ms(300);
            let t0 = utc_hms();
            press(KEY_A.0, KEY_A.1, 12);
            sleep_ms(400);
            press(key, kscan, 12);
            sleep_ms(600);
            press(VK_RETURN, 0x1C, 50);
            sleep_ms(1500);
            let text = read_text_maybe_settled(child, settle);
            rec(&json!({"type":"lateorder","n":n,"kind":"comp_on","utc":t0,
                "expect":"(Space は増えない)","text":text}));
        }
    }
}
