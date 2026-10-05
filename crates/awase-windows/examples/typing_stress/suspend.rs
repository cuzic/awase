//! 「相手を遅くする」摂動: 指定プロセス(GJI の変換サーバなど)を、打鍵の途中の指定時刻から指定 ms だけ
//! 一時停止する(`NtSuspendProcess` → `NtResumeProcess`)。`perturb.rs` の `--pause-*` は自分が「待つ」だけで、
//! IME や対象アプリの応答が遅れる状況(BUG-171 の途中の語の StaleConfirm、BUG-075、BUG-147 の時間窓)は作れなかった。
//!
//! | フラグ                          | 効果                                                                   |
//! |---------------------------------|------------------------------------------------------------------------|
//! | `--suspend-proc=A,B`            | 一時停止する実行ファイル名(大文字小文字を区別しない部分一致。カンマ区切り) |
//! | `--suspend-at-ms=T`             | 各試行の打鍵開始から T ms 後に停止する(既定 300)                        |
//! | `--suspend-ms=D`                | D ms 後に再開する(既定 500、上限 10000)                                  |
//!
//! 各試行の打鍵開始の直前にスレッドを起こし、`type:"suspend"` のレコードに「実際に停止できたプロセス(名前・PID・
//! NTSTATUS)」と停止・再開の実時刻(`utc`)を残す。`matched` が空・status が非 0 なら停止は効いていない
//! (権限不足・プロセス名違い)ので、その run の結果を「遅延の下でも壊れなかった」と読まないこと。
//! 起動時に `type:"suspend_candidates"` として、IME 関連らしいプロセス名の一覧も残す(名前の確認用)。

use std::time::{Duration, Instant};

use serde_json::json;
use windows::core::{s, w};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_SUSPEND_RESUME};

use crate::{arg_value, rec, utc_hms};

const MAX_SUSPEND_MS: u64 = 10_000;

type NtProcFn = unsafe extern "system" fn(HANDLE) -> i32;

pub(crate) struct Suspend {
    names: Vec<String>,
    at_ms: u64,
    dur_ms: u64,
}

fn num(key: &str, default: u64) -> u64 {
    arg_value(key).and_then(|v| v.parse().ok()).unwrap_or(default)
}

/// (名前, PID) の一覧。
fn processes() -> Vec<(String, u32)> {
    let mut out = Vec::new();
    // SAFETY: TH32CS_SNAPPROCESS と PID=0(全プロセス)は有効な引数。ハンドルは下で必ず閉じる。
    let Ok(snap) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else {
        return out;
    };
    let mut e = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    // SAFETY: snap は有効なスナップショット、e は dwSize 設定済み。
    if unsafe { Process32FirstW(snap, &raw mut e) }.is_ok() {
        loop {
            let end = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
            out.push((String::from_utf16_lossy(&e.szExeFile[..end]), e.th32ProcessID));
            // SAFETY: 同上。
            if unsafe { Process32NextW(snap, &raw mut e) }.is_err() {
                break;
            }
        }
    }
    // SAFETY: snap は直上で作った有効なハンドルで、二度閉じない。
    let _ = unsafe { CloseHandle(snap) };
    out
}

fn nt_fns() -> Option<(NtProcFn, NtProcFn)> {
    // SAFETY: ntdll はすべてのプロセスに読み込み済み。Nt(Suspend|Resume)Process は (HANDLE)->NTSTATUS の関数。
    unsafe {
        let m = GetModuleHandleW(w!("ntdll.dll")).ok()?;
        let s = GetProcAddress(m, s!("NtSuspendProcess"))?;
        let r = GetProcAddress(m, s!("NtResumeProcess"))?;
        Some((std::mem::transmute(s), std::mem::transmute(r)))
    }
}

impl Suspend {
    pub(crate) fn from_args() -> Option<Self> {
        let names: Vec<String> = arg_value("--suspend-proc=")?
            .split(',')
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        if names.is_empty() {
            return None;
        }
        Some(Self {
            names,
            at_ms: num("--suspend-at-ms=", 300),
            dur_ms: num("--suspend-ms=", 500).min(MAX_SUSPEND_MS),
        })
    }

    pub(crate) fn describe(&self) -> serde_json::Value {
        json!({"proc":self.names,"at_ms":self.at_ms,"dur_ms":self.dur_ms})
    }

    /// 停止できるプロセス名の見当をつけるための一覧(IME・入力・Chrome らしい名前だけ)。
    pub(crate) fn log_candidates(&self) {
        let want = ["google", "ime", "ctf", "mozc", "chrome", "text", "input", "msedge"];
        let list: Vec<String> = processes()
            .into_iter()
            .filter(|(n, _)| {
                let l = n.to_ascii_lowercase();
                want.iter().any(|w| l.contains(w))
            })
            .map(|(n, p)| format!("{n}({p})"))
            .collect();
        rec(&json!({"type":"suspend_candidates","candidates":list,"want":self.names}));
    }

    /// 試行の打鍵開始の直前に呼ぶ。返したハンドルを打鍵の後で `join` する(再開まで待つ)。
    pub(crate) fn arm(&self, kind: &str, n: usize) -> std::thread::JoinHandle<()> {
        let names = self.names.clone();
        let (at_ms, dur_ms) = (self.at_ms, self.dur_ms);
        let kind = kind.to_string();
        std::thread::spawn(move || {
            let armed = Instant::now();
            std::thread::sleep(Duration::from_millis(at_ms));
            let Some((suspend, resume)) = nt_fns() else {
                rec(&json!({"type":"suspend","kind":kind,"n":n,"error":"ntdll の Nt(Suspend|Resume)Process が見つからない"}));
                return;
            };
            let mut held: Vec<(String, u32, HANDLE, i32)> = Vec::new();
            for (name, pid) in processes() {
                let l = name.to_ascii_lowercase();
                if !names.iter().any(|w| l.contains(w.as_str())) {
                    continue;
                }
                // SAFETY: pid は直前のスナップショット由来。失敗は matched に status を残して続行する。
                match unsafe { OpenProcess(PROCESS_SUSPEND_RESUME, false, pid) } {
                    Ok(h) => {
                        // SAFETY: h は PROCESS_SUSPEND_RESUME 付きの有効なハンドル。
                        let st = unsafe { suspend(h) };
                        held.push((name, pid, h, st));
                    }
                    Err(e) => rec(&json!({"type":"suspend_open_failed","name":name,"pid":pid,"error":e.to_string()})),
                }
            }
            let t_susp = Instant::now();
            let susp_utc = utc_hms();
            std::thread::sleep(Duration::from_millis(dur_ms));
            let mut matched = Vec::new();
            for (name, pid, h, st) in held {
                // SAFETY: h は上で開いた有効なハンドル。停止に成功していなくても Resume は無害(カウントが 0 なら何もしない)。
                let rst = unsafe { resume(h) };
                // SAFETY: h は一度だけ閉じる。
                let _ = unsafe { CloseHandle(h) };
                matched.push(json!({"name":name,"pid":pid,"suspend_status":st,"resume_status":rst}));
            }
            rec(&json!({"type":"suspend","kind":kind,"n":n,"matched":matched,
                "at_ms_req":at_ms,"dur_ms_req":dur_ms,
                "at_ms_actual":u64::try_from(t_susp.duration_since(armed).as_millis()).unwrap_or(u64::MAX),
                "dur_ms_actual":u64::try_from(t_susp.elapsed().as_millis()).unwrap_or(u64::MAX),
                "suspended_utc":susp_utc,"resumed_utc":utc_hms()}));
        })
    }
}
