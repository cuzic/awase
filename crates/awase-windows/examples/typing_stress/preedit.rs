//! `--mode=preedit`(再現用の部品: 未確定文字の独立読み取り): 最終テキスト(確定後の本文)とは別に、
//! 「今、未確定文字(composition)があるか・その内容」を読む手段の可否を、入力先×IME ごとに測る。
//!
//! BUG-184(MS-IME+TSF で英数 OFF すると未確定が消える)・BUG-171/168(GJI の ESC・reinit で未確定が消える)の
//! 判定は、最終テキストだけでは「消えた」のか「元から無かった」のか区別できない。これは区別するための読み取り部品。
//!
//! 1 試行 = ON にそろえる →(before)→ かな 1 打 →(composing)→ もう 1 打 →(composing2)→ 終端キー →(after)。
//! 各 phase で次の手段を同時に読み、`[TS-JSON]` に `preedit` 行として記録する(判定は check_preedit.py):
//!  - `uia`   : フォーカス要素の UIA `TextEditPattern::GetActiveComposition`。別プロセスの入力先(実 Chrome 等)でも読める。
//!              `composing:<文字列>` / `none` / `nopattern`(パターン非対応)/ `err:<段階>`。Chrome は composition 無しのとき
//!              U+FFFC か空を返す(BUG-185 の調査 n≈70)ので、空・U+FFFC・空白だけは `none` に正規化する。
//!  - `imm`   : 自プロセスの窓(`--form=edit|multi|rich|tsf`)の `ImmGetCompositionStringW(GCS_COMPSTR)`。別プロセスは `nohimc`。
//!  - `value` : フォーカス要素の UIA ValuePattern(本文)。composition が本文に含まれて見えるかの参考。
//!  - `text`  : 入力先の読み戻し(`WM_GETTEXT` / UIA)。composition 中に本文へ出ていないことの確認にも使う。
//! 追加フラグ: `--preedit-end=enter|esc|none`(終端キー。既定 enter=確定 / esc=取り消し / none=何も押さない)、`--trials=N`(既定 6)。
//! TSF の `ITfContext` を直接読む手段は、`ITfThreadMgr` がスレッド単位で、この worker スレッドのフォーカス文書が空になるため
//! 未実装(UI スレッド側で実行する経路が要る)。

use serde_json::json;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationTextEditPattern, IUIAutomationValuePattern,
    UIA_TextEditPatternId, UIA_ValuePatternId,
};
use windows::Win32::UI::Input::Ime::{
    ImmGetCompositionStringW, ImmGetContext, ImmReleaseContext, GCS_COMPSTR,
};

use crate::{
    arg_value, clear_text, focus_ok, press, read_text_maybe_settled, rec, refocus, sleep_ms,
    turn_ime_on, utc_hms, Cell, VK_RETURN,
};

const VK_ESCAPE: u32 = 0x1B;

/// composition 無しの Chrome は U+FFFC(OBJECT REPLACEMENT)か空を返す。それらと空白だけなら無しとみなす。
fn normalize(s: &str) -> String {
    let t: String = s.chars().filter(|c| *c != '\u{FFFC}' && !c.is_whitespace()).collect();
    if t.is_empty() {
        "none".into()
    } else {
        format!("composing:{s}")
    }
}

/// フォーカス要素の UIA TextEditPattern::GetActiveComposition。
fn uia_composition() -> String {
    // SAFETY: UIA の COM 呼び出しのみ。
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let Ok(a) =
            CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
        else {
            return "err:create".into();
        };
        let Ok(el) = a.GetFocusedElement() else {
            return "err:focus".into();
        };
        let Ok(pat) = el.GetCurrentPatternAs::<IUIAutomationTextEditPattern>(UIA_TextEditPatternId)
        else {
            return "nopattern".into();
        };
        match pat.GetActiveComposition() {
            Ok(r) => normalize(&r.GetText(-1).map(|b| b.to_string()).unwrap_or_default()),
            Err(_) => "none".into(), // null 範囲(composition 無し)
        }
    }
}

/// フォーカス要素の UIA ValuePattern の値(composition が本文に含まれて見えるかの参考)。
fn uia_value() -> String {
    // SAFETY: UIA プロパティの読み取りのみ。
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let Ok(a) =
            CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
        else {
            return "err:create".into();
        };
        let Ok(el) = a.GetFocusedElement() else {
            return "err:focus".into();
        };
        match el.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId) {
            Ok(p) => p.CurrentValue().map(|b| b.to_string()).unwrap_or_default(),
            Err(_) => "nopattern".into(),
        }
    }
}

/// 自プロセスの窓の ImmGetCompositionStringW(GCS_COMPSTR)。
fn imm_composition(child: HWND) -> String {
    // SAFETY: 自プロセスの入力欄の HWND に対する IMM 呼び出し。取得した HIMC は必ず解放する。
    unsafe {
        let himc = ImmGetContext(child);
        if himc.is_invalid() {
            return "nohimc".into();
        }
        let bytes = ImmGetCompositionStringW(himc, GCS_COMPSTR, None, 0);
        let out = if bytes <= 0 {
            "none".to_owned()
        } else {
            let mut buf = vec![0u16; usize::try_from(bytes).unwrap_or(0) / 2 + 1];
            let n = ImmGetCompositionStringW(
                himc,
                GCS_COMPSTR,
                Some(buf.as_mut_ptr().cast()),
                u32::try_from(bytes).unwrap_or(0),
            );
            let len = usize::try_from(n).unwrap_or(0) / 2;
            format!("composing:{}", String::from_utf16_lossy(&buf[..len.min(buf.len())]))
        };
        let _ = ImmReleaseContext(child, himc);
        out
    }
}

fn read_all(child: HWND, n: usize, phase: &str, settle: bool) {
    let t = std::time::Instant::now();
    let uia = uia_composition();
    let uia_ms = u64::try_from(t.elapsed().as_millis()).unwrap_or(u64::MAX);
    let imm = imm_composition(child);
    let value = uia_value();
    let text = read_text_maybe_settled(child, settle);
    rec(&json!({"type":"preedit","n":n,"phase":phase,"utc":utc_hms(),
        "uia":uia,"uia_ms":uia_ms,"imm":imm,"value":value,"text":text}));
}

pub(crate) fn preedit_scenario(child: HWND, cells: &[Vec<Cell>; 3]) {
    let trials: usize = arg_value("--trials=").and_then(|v| v.parse().ok()).unwrap_or(6);
    let end = arg_value("--preedit-end=").unwrap_or_else(|| "enter".into());
    if !matches!(end.as_str(), "enter" | "esc" | "none") {
        rec(&json!({"type":"abort","reason":format!("--preedit-end={end} は enter|esc|none")}));
        return;
    }
    let settle = crate::has_flag("--settle-read");
    let Some(probe) = cells[0].iter().find(|c| c.romaji == "ka").cloned() else {
        rec(&json!({"type":"abort","reason":"preedit の打鍵に使う ka セルが無い"}));
        return;
    };
    rec(&json!({"type":"preedit_config","end":end,"expect":probe.kana.to_string(),"trials":trials}));
    for n in 0..trials {
        if !focus_ok() {
            refocus();
        }
        if !focus_ok() {
            rec(&json!({"type":"abort","reason":format!("preedit 試行前にフォーカスが外れた n={n}")}));
            return;
        }
        turn_ime_on(0);
        clear_text(child);
        sleep_ms(300);
        read_all(child, n, "before", settle);
        press(probe.vk, probe.scan, 60);
        sleep_ms(600);
        read_all(child, n, "composing", settle);
        press(probe.vk, probe.scan, 60);
        sleep_ms(500);
        read_all(child, n, "composing2", settle);
        match end.as_str() {
            "enter" => press(VK_RETURN, 0x1C, 50),
            "esc" => press(VK_ESCAPE, 0x01, 50),
            _ => {}
        }
        sleep_ms(700);
        read_all(child, n, "after", settle);
        // 次の試行に未確定を持ち越さない(none のときだけ。Enter で確定、本文は clear_text が消す)。
        if end == "none" {
            press(VK_RETURN, 0x1C, 50);
            sleep_ms(300);
        }
        clear_text(child);
    }
}
