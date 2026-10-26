//! `--mode=scope`(ADR-282 の測定: IME の開閉・変換モードの保持範囲。awase は起動しない):
//! 窓 A(別プロセス)で IME の状態を作って保持し、別プロセスの窓 B(新しいスレッド)で打鍵の結果を読んで、
//! B の状態が A の影響を受けるか(共有)、B 自身の独立した状態か(per-thread)を判定する材料を記録する。
//! 判定は tools/e2e/ime_scope/scope_probe.py(A と B のプロセスを順に起動する)。本文は読まない(固定の `ka` だけを打つ)。
//!
//! - `--scope-role=a --scope-state=hira|eisu|off --scope-hold=PATH`: IME を `hira`(OFF→ON)・`eisu`(ON のあと英数キー 0xF0)・
//!   `off` にそろえ、自分で `ka` を打って結果を `scope_a` に記録(自己検証: hira は `か`、eisu/off は `ka` のはず)。
//!   そのあと `[SCOPE] A-READY` を出し、PATH のファイルができるまで窓を保持する。
//! - `--scope-role=b --scope-act=none|ime-on`: 何もせず(または ON キーを 1 回押して)`ka` を打ち、Enter で確定して読む。
//!   打鍵の前に自プロセスの HIMC の開閉(`real_ime_open`、自前の窓のときだけ)も記録する。
//! 記録は `scope_a` / `scope_b` 行。

use serde_json::json;
use windows::Win32::Foundation::HWND;

use crate::{
    arg_value, clear_text, ime_on_key, press, read_text_maybe_settled, real_ime_open, rec, refocus,
    sleep_ms, turn_ime_on, utc_hms, VK_IME_OFF, VK_RETURN,
};

const VK_K: u32 = 0x4B;
const SCAN_K: u16 = 0x25;
const VK_A: u32 = 0x41;
const SCAN_A: u16 = 0x1E;
/// 英数キー(VK_DBE_ALPHANUMERIC)。JIS の英数キーのスキャンコード。
const VK_EISU: u32 = 0xF0;
const SCAN_EISU: u16 = 0x3A;

fn type_ka_and_commit(child: HWND) -> String {
    clear_text(child);
    sleep_ms(300);
    press(VK_K, SCAN_K, 40);
    sleep_ms(120);
    press(VK_A, SCAN_A, 40);
    sleep_ms(700);
    press(VK_RETURN, 0x1C, 50);
    sleep_ms(1200);
    let text = read_text_maybe_settled(child, true);
    clear_text(child);
    text
}

pub(crate) fn scope_scenario(child: HWND) {
    let role = arg_value("--scope-role=").unwrap_or_default();
    refocus();
    sleep_ms(800);
    if role == "a" {
        let state = arg_value("--scope-state=").unwrap_or_else(|| "hira".into());
        match state.as_str() {
            "off" => {
                press(VK_IME_OFF, 0x70, 50);
                sleep_ms(800);
            }
            "eisu" => {
                turn_ime_on(0);
                press(VK_EISU, SCAN_EISU, 50);
                sleep_ms(800);
            }
            _ => turn_ime_on(0),
        }
        let open = real_ime_open(child);
        let text = type_ka_and_commit(child);
        rec(
            &json!({"type":"scope_a","state":state,"utc":utc_hms(),"real_ime_open":open,"text":text}),
        );
        crate::log("[SCOPE] A-READY");
        let hold = arg_value("--scope-hold=").unwrap_or_default();
        for _ in 0..300 {
            if !hold.is_empty() && std::path::Path::new(&hold).exists() {
                break;
            }
            sleep_ms(500);
        }
        rec(&json!({"type":"scope_a_done","utc":utc_hms()}));
    } else {
        let act = arg_value("--scope-act=").unwrap_or_else(|| "none".into());
        let open_before = real_ime_open(child);
        if act == "ime-on" {
            press(ime_on_key(0), 0x70, 50);
            sleep_ms(800);
        }
        let open_after_act = real_ime_open(child);
        let text = type_ka_and_commit(child);
        rec(
            &json!({"type":"scope_b","act":act,"utc":utc_hms(),"open_before":open_before,
            "open_after_act":open_after_act,"text":text}),
        );
    }
}
