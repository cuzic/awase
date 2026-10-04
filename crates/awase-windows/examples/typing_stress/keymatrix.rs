//! `--mode=keymatrix`(ADR-208 L3b): 「ずれの作り方 × キー」行列の1セルずつを試行する。
//!
//! 目的: 明示的な IME キーを押したとき、awase の内部状態(belief / `applied`)が何であっても、絶対キーは1押下・トグルは
//! 2押下で実 IME がキーの意味に一致するか(固着ゼロ、ADR-208 INV-L2)を、実打鍵と実 IME の読み取りで測る。
//! 判定は `tools/e2e/ime_key_matrix/check_keymatrix.py`。このファイルは注入と記録だけを行う。
//!
//! 引数: `--km-cells=<key>=<kind>:<gap>,...`(例: `ctrl+1c=on:close,1d=tog:open,ctrl+1d=off:fresh`)
//! - `<key>` は VK の16進(`1c`=変換、`1d`=無変換、`16`=VK_IME_ON、`1a`=VK_IME_OFF、`f3`=半角/全角 …)。`ctrl+` を前置すると
//!   マーカー付き注入の Ctrl↓→キー↓↑→Ctrl↑(debug awase は物理 Ctrl 扱い)。
//! - `<kind>` は `on`(ON 絶対) / `off`(OFF 絶対) / `tog`(トグル)。
//! - `<gap>` は ずれの作り方: `sync`(awase に書かせた状態で、ずれなし) / `close`(awase に ON を書かせた後、実 IME を外から閉じる。S-1) /
//!   `open`(awase に OFF を書かせた後、実 IME を外から開く) / `fresh`(awase を再起動し、belief・applied が未知の状態で押す。ADR-208 決定4 の E2)。
//!   `close` は `off` と、`open` は `on` と組み合わせない(押す前から実 IME が意味と一致していて測定にならない)。
//!
//! 追加フラグ: `--km-n=N`(1セルあたりの試行数。既定10) / `--km-wait=MS`(ずれを作ってから押すまで。既定1000) /
//! `--km-max-press=N`(一致しないとき最大何回押すか。既定3) / `--km-fresh-settle=MS`(awase 再起動後の落ち着き。既定8000)。
//!
//! 記録: `km_config`(1回) と、試行ごとの `km_trial`(`cell`/`key`/`kind`/`gap`/`n`/`r0`/`target`/`pre_ok`/`pre_api`/`presses[]`/`typed`)。
//! `presses[]` は押下ごとに +500ms と +2000ms の `ImmGetOpenStatus`(`api500`/`api2000`)。打鍵(かな単打)は最後に1回だけ(`typed`)で、
//! 押下の間には打たない(awase の literal 回収などが状態を直す交絡を避ける)。

// 親モジュール(main.rs)の非公開の注入・IME 読み取りヘルパーをそのまま使う。
#[allow(clippy::wildcard_imports)]
use super::*;

use std::os::windows::process::CommandExt as _;
use std::process::{Command, Stdio};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    On,
    Off,
    Tog,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Gap {
    Sync,
    Close,
    Open,
    Fresh,
}

#[derive(Clone)]
struct KmCell {
    label: String,
    key: String,
    vk: u32,
    scan: u16,
    ctrl: bool,
    kind: Kind,
    gap: Gap,
}

impl KmCell {
    /// ずれを作った直後に実 IME がとるべき状態(押す前の状態)。
    fn r0(&self) -> bool {
        match self.gap {
            Gap::Close => false,
            Gap::Open => true,
            Gap::Sync | Gap::Fresh => !matches!(self.kind, Kind::On),
        }
    }

    /// キーの意味に一致した状態(トグルは押す前の状態の反転)。
    fn target(&self) -> bool {
        match self.kind {
            Kind::On => true,
            Kind::Off => false,
            Kind::Tog => !self.r0(),
        }
    }

    fn kind_name(&self) -> &'static str {
        match self.kind {
            Kind::On => "on",
            Kind::Off => "off",
            Kind::Tog => "tog",
        }
    }

    fn gap_name(&self) -> &'static str {
        match self.gap {
            Gap::Sync => "sync",
            Gap::Close => "close",
            Gap::Open => "open",
            Gap::Fresh => "fresh",
        }
    }
}

/// キーごとの scan。無変換/変換/半角全角は物理位置(scancode)で分類されるので対応する scan を使う(BUG-131/132)。
fn scan_of(vk: u32) -> u16 {
    match vk {
        VK_MUHENKAN => SCAN_MUHENKAN,
        VK_HENKAN => SCAN_HENKAN,
        0xF3 | 0xF4 | 0x19 => 0x29,
        0x7C => 0x64,
        _ => 0x70,
    }
}

fn parse_cell(spec: &str) -> Result<KmCell, String> {
    let (key, rest) = spec
        .split_once('=')
        .ok_or_else(|| format!("セル指定に '=' が無い: {spec}"))?;
    let (kind, gap) = rest
        .split_once(':')
        .ok_or_else(|| format!("セル指定に ':' が無い: {spec}"))?;
    let ctrl = key.starts_with("ctrl+");
    let hex = key.strip_prefix("ctrl+").unwrap_or(key);
    let vk = u32::from_str_radix(hex.trim_start_matches("0x"), 16)
        .map_err(|e| format!("キーが16進でない: {key}: {e}"))?;
    let kind = match kind {
        "on" => Kind::On,
        "off" => Kind::Off,
        "tog" => Kind::Tog,
        other => return Err(format!("kind が不正: {other}")),
    };
    let gap = match gap {
        "sync" => Gap::Sync,
        "close" => Gap::Close,
        "open" => Gap::Open,
        "fresh" => Gap::Fresh,
        other => return Err(format!("gap が不正: {other}")),
    };
    if (gap == Gap::Close && kind == Kind::Off) || (gap == Gap::Open && kind == Kind::On) {
        return Err(format!(
            "意味と一致した状態から始まる組み合わせは測定にならない: {spec}"
        ));
    }
    Ok(KmCell {
        label: spec.to_string(),
        key: key.to_string(),
        vk,
        scan: scan_of(vk),
        ctrl,
        kind,
        gap,
    })
}

/// マーカー付き注入の Ctrl↓→キー↓↑→Ctrl↑(`press_ctrl_muhenkan` と同じ順序・間隔)。
fn press_chord(vk: u32, scan: u16) {
    send_key(VK_LCONTROL, SCAN_LCONTROL, true);
    sleep_ms(40);
    send_key(vk, scan, true);
    sleep_ms(60);
    send_key(vk, scan, false);
    sleep_ms(30);
    send_key(VK_LCONTROL, SCAN_LCONTROL, false);
}

/// `--km-commit=<名前>` の確定系キー(BUG-185 方針C)。(vk, scan, 修飾 vk)。
fn commit_key(name: &str) -> Option<(u32, u16, Option<(u32, u16)>)> {
    Some(match name {
        "enter" => (0x0D, 0x1C, None),
        "ctrlm" => (0x4D, 0x32, Some((VK_LCONTROL, SCAN_LCONTROL))),
        "ctrlj" => (0x4A, 0x24, Some((VK_LCONTROL, SCAN_LCONTROL))),
        "ctrlenter" => (0x0D, 0x1C, Some((VK_LCONTROL, SCAN_LCONTROL))),
        "shiftenter" => (0x0D, 0x1C, Some((0xA0, 0x2A))),
        "tab" => (0x09, 0x0F, None),
        "f6" => (0x75, 0x40, None),
        "f7" => (0x76, 0x41, None),
        "f8" => (0x77, 0x42, None),
        "f9" => (0x78, 0x43, None),
        "f10" => (0x79, 0x44, None),
        _ => return None,
    })
}

fn send_commit_key(name: &str) {
    let Some((vk, scan, m)) = commit_key(name) else {
        return;
    };
    if let Some((mv, ms)) = m {
        send_key(mv, ms, true);
        sleep_ms(40);
        send_key(vk, scan, true);
        sleep_ms(60);
        send_key(vk, scan, false);
        sleep_ms(30);
        send_key(mv, ms, false);
    } else {
        press(vk, scan, 50);
    }
}

fn send_cell_key(c: &KmCell) {
    if c.ctrl {
        press_chord(c.vk, c.scan);
    } else {
        press(c.vk, c.scan, 60);
    }
}

/// 既定 IME ウィンドウへ `IMC_SETOPENSTATUS` を送る(awase を経由しない外部要因の再現)。`open=false` が閉じる、`true` が開く。
fn force_set_real_ime(child: HWND, open: bool) -> Option<isize> {
    const WM_IME_CONTROL: u32 = 0x0283;
    const IMC_SETOPENSTATUS: usize = 0x0006;
    // SAFETY: 自プロセスの入力欄に対応する既定 IME ウィンドウへ同期 SendMessage するだけ。
    unsafe {
        let ime_wnd = windows::Win32::UI::Input::Ime::ImmGetDefaultIMEWnd(child);
        if ime_wnd.0.is_null() {
            return None;
        }
        Some(
            SendMessageW(
                ime_wnd,
                WM_IME_CONTROL,
                Some(WPARAM(IMC_SETOPENSTATUS)),
                Some(LPARAM(isize::from(open))),
            )
            .0,
        )
    }
}

/// awase のキー(明示意図)で実 IME を `on` にそろえる。API が一致したかを返す。
/// 手順は `--mode=drift-on` と同じ(先頭は VK_IME_OFF、ON は VK_IME_ON から。効かなければ次の候補)。
fn set_state_via_keys(child: HWND, on: bool) -> bool {
    for k in 0..3 {
        press(VK_IME_OFF, 0x70, 50);
        if on {
            sleep_ms(600);
            let on_key = if k == 0 { VK_IME_ON } else { ime_on_key(k) };
            press(on_key, 0x70, 50);
        }
        sleep_ms(1500);
        if real_ime_open(child) == Some(on) {
            return true;
        }
    }
    false
}

fn awase_exe() -> Option<std::path::PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("awase.exe")))
}

fn kill_awase() {
    let _ = Command::new("taskkill")
        .args(["/F", "/IM", "awase.exe"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .status();
}

fn start_awase() -> bool {
    let Some(exe) = awase_exe() else {
        return false;
    };
    let Some(dir) = exe.parent() else {
        return false;
    };
    // RUST_LOG / AWASE_TEST_INJECTION は CI のステップが typing_stress に渡した環境変数を引き継ぐ。
    Command::new(&exe)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .is_ok()
}

/// `fresh`: awase を落として実 IME を `on` にそろえ、awase を起動し直して(belief・applied が未知の状態で)落ち着くのを待つ。
/// 戻り値は (そろえられたか, awase を起動できたか, awase.log が伸びたか)。
fn restart_awase_fresh(child: HWND, on: bool, settle_ms: u64) -> (bool, bool, bool) {
    let before = std::fs::metadata("awase.log").map_or(0, |m| m.len());
    kill_awase();
    sleep_ms(1500);
    // awase が居ない間は、IME が自分で VK_IME_ON/OFF を処理する(D1 の startup モードと同じ前提)。
    let mut aligned = false;
    for _ in 0..3 {
        press(VK_IME_OFF, 0x70, 50);
        if on {
            sleep_ms(600);
            press(VK_IME_ON, 0x70, 50);
        }
        sleep_ms(1200);
        if real_ime_open(child) == Some(on) {
            aligned = true;
            break;
        }
    }
    let started = start_awase();
    let mut grew = false;
    for _ in 0..60 {
        if std::fs::metadata("awase.log").map_or(0, |m| m.len()) > before {
            grew = true;
            break;
        }
        sleep_ms(250);
    }
    sleep_ms(settle_ms);
    refocus();
    sleep_ms(500);
    (aligned, started, grew)
}

fn int_arg(key: &str, default: u64) -> u64 {
    arg_value(key)
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// `--mode=keymatrix`。`ime_ready` の後に呼ぶ。
pub(crate) fn keymatrix_scenario(child: HWND, cells: &[Vec<Cell>; 3]) {
    let Some(spec) = arg_value("--km-cells=") else {
        rec(&json!({"type":"abort","reason":"keymatrix: --km-cells= が無い"}));
        return;
    };
    let mut km_cells = Vec::new();
    for s in spec.split(',').filter(|s| !s.is_empty()) {
        match parse_cell(s) {
            Ok(c) => km_cells.push(c),
            Err(e) => {
                rec(&json!({"type":"abort","reason":format!("keymatrix: {e}")}));
                return;
            }
        }
    }
    let n = int_arg("--km-n=", 10);
    let wait_ms = int_arg("--km-wait=", 1000);
    let max_press = int_arg("--km-max-press=", 3);
    let fresh_settle = int_arg("--km-fresh-settle=", 8000);
    // `--km-comp`(MS-IME×実 Chrome の OFF 切り分け): 押す前にかな単打を1回打って未確定の composition を残す。
    let comp = std::env::args().any(|a| a == "--km-comp");
    let commit = arg_value("--km-commit=");
    let Some(probe) = cells[0]
        .iter()
        .find(|c| c.romaji == "ka")
        .cloned()
        .or_else(|| cells[0].first().cloned())
    else {
        rec(&json!({"type":"abort","reason":"keymatrix の打鍵確認に使う単打セルが無い"}));
        return;
    };
    rec(
        &json!({"type":"km_config","cells":km_cells.iter().map(|c| c.label.clone()).collect::<Vec<_>>(),
        "n":n,"wait_ms":wait_ms,"max_press":max_press,"fresh_settle_ms":fresh_settle,"evidence":"api","comp":comp}),
    );
    for cell in &km_cells {
        for i in 0..n {
            if !focus_ok() {
                refocus();
            }
            if !focus_ok() {
                rec(
                    &json!({"type":"abort","reason":format!("keymatrix 試行前にフォーカスが外れた cell={} n={i}",cell.label)}),
                );
                return;
            }
            let utc0 = utc_hms();
            let r0 = cell.r0();
            let target = cell.target();
            let mut fresh_info = serde_json::Value::Null;
            let pre_ok = match cell.gap {
                Gap::Sync => set_state_via_keys(child, r0),
                Gap::Close => {
                    let ok = set_state_via_keys(child, true);
                    let _ = force_set_real_ime(child, false);
                    sleep_ms(50);
                    ok
                }
                Gap::Open => {
                    let ok = set_state_via_keys(child, false);
                    let _ = force_set_real_ime(child, true);
                    sleep_ms(50);
                    ok
                }
                Gap::Fresh => {
                    let (aligned, started, grew) = restart_awase_fresh(child, r0, fresh_settle);
                    fresh_info = json!({"aligned":aligned,"started":started,"log_grew":grew});
                    aligned && started
                }
            };
            sleep_ms(wait_ms);
            let pre_api = real_ime_open(child);
            let mut presses = Vec::new();
            if comp && pre_ok && pre_api == Some(r0) {
                press(probe.vk, probe.scan, 60);
                sleep_ms(500);
            }
            // `--km-commit=<名前>`: 確定系キー(composition があれば確定して本文に残るか、無ければ副作用が無いか)。直後の本文を記録。
            let mut text_commit = serde_json::Value::Null;
            if let Some(ck) = &commit {
                if pre_ok && pre_api == Some(r0) {
                    send_commit_key(ck);
                    sleep_ms(500);
                    text_commit = json!(read_text(child));
                }
            }
            if pre_ok && pre_api == Some(r0) {
                for p in 1..=max_press {
                    let utc = utc_hms();
                    send_cell_key(cell);
                    sleep_ms(500);
                    let api500 = real_ime_open(child);
                    sleep_ms(1500);
                    let api2000 = real_ime_open(child);
                    presses.push(json!({"i":p,"utc":utc,"api500":api500,"api2000":api2000}));
                    if api2000.or(api500) == Some(target) {
                        break;
                    }
                }
            }
            let focus_lost = !focus_ok();
            let press_utc = utc_hms();
            // 押下(OFF 等)の直後の本文(確定された文字が残っているか。空は取り消し)。
            let text_post = read_text(child);
            clear_text(child);
            sleep_ms(200);
            press(probe.vk, probe.scan, 60);
            sleep_ms(700);
            press(VK_RETURN, 0x1C, 50);
            sleep_ms(700);
            let text = read_text(child);
            rec(
                &json!({"type":"km_trial","cell":cell.label,"key":cell.key,"kind":cell.kind_name(),
                "gap":cell.gap_name(),"n":i,"r0":r0,"target":target,"utc":utc0,"pre_ok":pre_ok,
                "pre_api":pre_api,"commit":commit,"text_commit":text_commit,"text_post":text_post,"presses":presses,"focus_lost":focus_lost,"fresh":fresh_info,
                "typed":{"press_utc":press_utc,"text":text,"expect":probe.kana.to_string(),
                    "ok":text.trim() == probe.kana.to_string()}}),
            );
            clear_text(child);
        }
    }
}
