//! 宣言表で定義する入力先(`--form=ext --ext=<名前>`)。
//!
//! 入力先の種類(Java Swing/AWT・LibreOffice・OpenOffice・wxWidgets・Qt …)ごとに Rust の列挙子と
//! `InputTarget` 実装を足す代わりに、`tools/e2e/input_forms/forms.toml` へ 1 項目書けば入力先が増える。
//! この実装が持つのは「起動する・窓を探す・読み戻す・閉じる」の汎用部分だけで、入力先固有の知識は表に置く。
//!
//! 読み戻しは 3 種類(表の `read`):
//! - `dump`: 入力先(または補助プロセス `helper_*`)が、確定済みの本文を UTF-8 のファイル(`{dump}`)へ書き出し続ける。
//!   UI Automation で読めない入力先(Java は Java Access Bridge が要る、Office は UNO で読む)向け。
//! - `uia`: 窓内の最初の Edit を UI Automation で読む(Qt・wxWidgets・WinForms・WPF など)。
//! - `clipboard`: 入力欄で Ctrl+A → Ctrl+C して、クリップボードを PowerShell で読む(Scintilla・Flutter など、上の 2 つで読めない入力先)。
//!
//! 表の文字列中の置換: `{repo}`=`--ext-repo=`、`{dump}`=書き出しファイル、`{profile}`/`{profile_url}`=この run 専用の
//! 一時プロファイル(Office の `-env:UserInstallation=` 用)。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::Ordering;
use std::sync::Mutex;

use serde::Deserialize;
use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{mouse_event, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowRect, GetWindowThreadProcessId, PostMessageW, SetCursorPos,
    WM_CLOSE,
};

use super::{fatal, find_window, foreground_is_top, kill_tree, raise_top, title_of, InputTarget};
use crate::uia;
use crate::{class_of, hwnd_of, log, sleep_ms, CHILD, TOP};

#[derive(Deserialize)]
struct Manifest {
    forms: BTreeMap<String, Spec>,
}

#[derive(Deserialize, Default)]
struct WindowSpec {
    /// 窓タイトルに含まれる文字列。
    title: Option<String>,
    /// 窓クラス名(完全一致)。
    class: Option<String>,
}

#[derive(Deserialize)]
struct Spec {
    /// 起動する実行ファイルの候補(最初に存在するもの。区切りの無い名前は PATH 解決に任せる)。
    exe: Vec<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    window: WindowSpec,
    /// 起動した pid に限らず窓を探す(Office のように起動 exe が別プロセスへ引き継ぐ入力先)。
    #[serde(default)]
    any_pid: bool,
    /// `dump`(既定)/ `uia` / `clipboard`。
    #[serde(default)]
    read: String,
    /// `dump` を書く補助プロセス(例: Office 同梱の python で UNO から本文を読む)。
    #[serde(default)]
    helper_exe: Vec<String>,
    #[serde(default)]
    helper_args: Vec<String>,
    /// 一時プロファイルへ最初に書く設定(`office` = 初回起動のダイアログを出さない)。
    #[serde(default)]
    profile_seed: String,
    /// 起動直後に出て前面を奪うダイアログの窓クラス(例: LibreOffice の "Welcome" は `SALSUBFRAME`)。同じ pid の可視窓を閉じる。
    #[serde(default)]
    dismiss_class: Option<String>,
    /// 窓の中央をクリックして入力欄へフォーカスを入れる(WinUI 3 など、前面化だけでは入力欄にフォーカスが入らない入力先)。
    #[serde(default)]
    click: bool,
    /// 窓が出てから入力を始めるまでの待ち(ms)。
    #[serde(default)]
    settle_ms: u64,
}

struct Ext {
    spawned: u32,
    win_pid: u32,
    helper: Mutex<Option<Child>>,
    dump: PathBuf,
    profile: PathBuf,
    mode: ReadMode,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ReadMode {
    Dump,
    Uia,
    Clipboard,
}

const OFFICE_SEED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<oor:items xmlns:oor="http://openoffice.org/2001/registry" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
<item oor:path="/org.openoffice.Office.Common/Misc"><prop oor:name="ShowTipOfTheDay" oor:op="fuse"><value>false</value></prop></item>
<item oor:path="/org.openoffice.Office.Common/Misc"><prop oor:name="FirstRun" oor:op="fuse"><value>false</value></prop></item>
<item oor:path="/org.openoffice.Office.Common/Misc"><prop oor:name="UseOpenCL" oor:op="fuse"><value>false</value></prop></item>
<item oor:path="/org.openoffice.Office.Common/VCL"><prop oor:name="UseOpenGL" oor:op="fuse"><value>false</value></prop></item>
</oor:items>
"#;

fn pick(cands: &[String]) -> Option<String> {
    cands
        .iter()
        .find(|p| !p.contains(['/', '\\']) || Path::new(p).exists())
        .cloned()
}

fn pid_of(h: HWND) -> u32 {
    let mut pid = 0u32;
    // SAFETY: 窓の所有 pid の取得のみ。
    unsafe { GetWindowThreadProcessId(h, Some(&raw mut pid)) };
    pid
}

pub(super) fn launch() -> Box<dyn InputTarget> {
    let name = crate::arg_value("--ext=")
        .unwrap_or_else(|| fatal("--form=ext には --ext=<入力先の名前> が要る"));
    let manifest_path = crate::arg_value("--ext-manifest=")
        .unwrap_or_else(|| "tools/e2e/input_forms/forms.toml".into());
    let repo = crate::arg_value("--ext-repo=").unwrap_or_default();
    let text = std::fs::read_to_string(&manifest_path)
        .unwrap_or_else(|e| fatal(&format!("入力先の表を読めない: {manifest_path} {e}")));
    let mut manifest: Manifest = toml::from_str(&text)
        .unwrap_or_else(|e| fatal(&format!("入力先の表の構文エラー: {manifest_path} {e}")));
    let spec = manifest.forms.remove(&name).unwrap_or_else(|| {
        let known: Vec<_> = manifest.forms.keys().cloned().collect();
        fatal(&format!("表に無い入力先: {name}(表: {known:?})"))
    });

    let tmp = std::env::temp_dir();
    let id = std::process::id();
    let dump = tmp.join(format!("ts-ext-{id}.txt"));
    let profile = tmp.join(format!("ts-ext-profile-{id}"));
    let _ = std::fs::remove_file(&dump);
    let _ = std::fs::create_dir_all(profile.join("user"));
    if spec.profile_seed == "office" {
        let _ = std::fs::write(
            profile.join("user").join("registrymodifications.xcu"),
            OFFICE_SEED,
        );
    }
    let profile_url = format!("file:///{}", profile.to_string_lossy().replace('\\', "/"));
    let subst = |s: &String| {
        s.replace("{repo}", &repo)
            .replace("{dump}", &dump.to_string_lossy())
            .replace("{profile_url}", &profile_url)
            .replace("{profile}", &profile.to_string_lossy())
    };

    let exe_cands: Vec<String> = spec.exe.iter().map(subst).collect();
    let exe = pick(&exe_cands)
        .unwrap_or_else(|| fatal(&format!("{name}: 実行ファイルが見つからない {exe_cands:?}")));
    let args: Vec<String> = spec.args.iter().map(subst).collect();
    let spawned = match Command::new(&exe).args(&args).spawn() {
        Ok(c) => c.id(),
        Err(e) => fatal(&format!("{name}: 起動に失敗: {exe} {args:?} {e}")),
    };
    log(&format!("[init] ext={name} 起動 pid={spawned} exe={exe}"));

    let want_title = spec.window.title.clone();
    let want_class = spec.window.class.clone();
    let accept = move |h: HWND| {
        want_title
            .as_ref()
            .is_none_or(|t| title_of(h).contains(t.as_str()))
            && want_class.as_ref().is_none_or(|c| class_of(h) == *c)
    };
    let search_pid = if spec.any_pid { 0 } else { spawned };
    let Some(top) = find_window(search_pid, &accept, 180) else {
        kill_tree(spawned);
        fatal(&format!(
            "{name}: 窓が見つからない(title={:?} class={:?})",
            spec.window.title, spec.window.class
        ));
    };
    let win_pid = pid_of(top);
    log(&format!(
        "[init] ext={name} 窓 class={} title={:?} pid={win_pid}",
        class_of(top),
        title_of(top)
    ));
    TOP.store(top.0 as isize, Ordering::SeqCst);
    if let Some(cls) = spec.dismiss_class.as_deref() {
        dismiss_dialogs(win_pid, cls);
        raise_top(500);
    }

    if spec.click {
        click_center(top);
    }

    let helper_cands: Vec<String> = spec.helper_exe.iter().map(subst).collect();
    let helper = pick(&helper_cands).map(|hexe| {
        let hargs: Vec<String> = spec.helper_args.iter().map(subst).collect();
        let mut cmd = Command::new(&hexe);
        cmd.args(&hargs);
        // 補助プロセスの標準出力/エラーは {dump}.helper.log へ(起動に失敗した理由を残す)。
        let hlog = format!("{}.helper.log", dump.display());
        if let Ok(f) = std::fs::File::create(&hlog) {
            if let Ok(f2) = f.try_clone() {
                cmd.stdout(f2);
            }
            cmd.stderr(f);
        }
        if let Some(dir) = Path::new(&hexe).parent().filter(|d| d.exists()) {
            cmd.current_dir(dir);
        }
        match cmd.spawn() {
            Ok(c) => {
                log(&format!("[init] ext={name} 補助 pid={} {hexe}", c.id()));
                c
            }
            Err(e) => fatal(&format!("{name}: 補助プロセスの起動に失敗: {hexe} {e}")),
        }
    });

    let mode = match spec.read.as_str() {
        "uia" => ReadMode::Uia,
        "clipboard" => ReadMode::Clipboard,
        _ => ReadMode::Dump,
    };
    if mode == ReadMode::Dump {
        // 書き出しファイルが現れるまで待つ(Office は補助プロセスが UNO へ接続できて初めて出る)。
        let ok = (0..180).any(|_| {
            sleep_ms(500);
            dump.exists()
        });
        if !ok {
            log_helper_err(&dump);
            kill_tree(win_pid);
            kill_tree(spawned);
            fatal(&format!(
                "{name}: 読み戻しファイルが現れない: {}",
                dump.display()
            ));
        }
    }
    sleep_ms(spec.settle_ms.max(800));
    CHILD.store(top.0 as isize, Ordering::SeqCst);
    Box::new(Ext {
        spawned,
        win_pid,
        helper: Mutex::new(helper),
        dump,
        profile,
        mode,
    })
}

/// 起動直後のダイアログ(クラス `cls`、`pid` の可視窓)を、現れなくなるまで閉じる(最大 8 秒待つ)。
fn dismiss_dialogs(pid: u32, cls: &str) {
    let mut closed = 0;
    for _ in 0..16 {
        let want = cls.to_string();
        match find_window(pid, &move |h| class_of(h) == want, 1) {
            Some(h) => {
                log(&format!(
                    "[init] ダイアログを閉じる class={cls} title={:?}",
                    title_of(h)
                ));
                // SAFETY: 他プロセスの窓への WM_CLOSE の送信のみ。
                unsafe {
                    let _ = PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
                closed += 1;
                sleep_ms(500);
            }
            None if closed > 0 => break,
            None => sleep_ms(0),
        }
    }
}

/// 入力欄の全文をコピーして、クリップボードを読む。選択が残るので、続けて打つ前に `clear` を挟む前提(打鍵の試行間は clear が入る)。
fn read_clipboard() -> String {
    // コンソール窓を出さない(出すと前面を奪い、入力先のフォーカスが外れる)。
    let ps = |script: &str| {
        use std::os::windows::process::CommandExt;
        Command::new("powershell")
            .args(["-NoProfile", "-Sta", "-Command", script])
            .creation_flags(0x0800_0000)
            .output()
    };
    let _ = ps("Set-Clipboard -Value ''");
    crate::send_key(0x11, 0x1D, true);
    sleep_ms(20);
    crate::press(0x41, 0x1E, 30);
    sleep_ms(60);
    crate::press(0x43, 0x2E, 30);
    crate::send_key(0x11, 0x1D, false);
    sleep_ms(250);
    match ps("[Console]::OutputEncoding=[Text.Encoding]::UTF8; Get-Clipboard -Raw") {
        Ok(o) => {
            let t = String::from_utf8_lossy(&o.stdout).into_owned();
            t.trim_end_matches(['\r', '\n']).to_string()
        }
        Err(e) => {
            log(&format!("[ext] クリップボードを読めない: {e}"));
            uia::NOT_FOUND.to_string()
        }
    }
}

fn click_center(top: HWND) {
    let mut r = RECT::default();
    // SAFETY: 窓の矩形取得とカーソル移動・クリック注入のみ。
    unsafe {
        if GetWindowRect(top, &mut r).is_err() {
            return;
        }
        raise_top(300);
        let (x, y) = ((r.left + r.right) / 2, (r.top + r.bottom) / 2);
        let _ = SetCursorPos(x, y);
        sleep_ms(100);
        mouse_event(MOUSEEVENTF_LEFTDOWN, 0, 0, 0, 0);
        sleep_ms(50);
        mouse_event(MOUSEEVENTF_LEFTUP, 0, 0, 0, 0);
    }
    log("[init] 窓の中央をクリックしてフォーカスを入れた");
    sleep_ms(500);
}

fn log_helper_err(dump: &Path) {
    for ext in [".err", ".helper.log"] {
        let path = PathBuf::from(format!("{}{ext}", dump.display()));
        if let Ok(s) = std::fs::read_to_string(&path) {
            for l in s.lines().take(40) {
                log(&format!("[ext-helper{ext}] {l}"));
            }
        }
    }
}

impl InputTarget for Ext {
    fn read(&self) -> String {
        match self.mode {
            ReadMode::Uia => {
                let edits = uia::wait_edits(hwnd_of(&TOP), |e| (!e.is_empty()).then_some(e))
                    .unwrap_or_default();
                edits
                    .first()
                    .map_or_else(|| uia::NOT_FOUND.to_string(), uia::read_value)
            }
            ReadMode::Clipboard => read_clipboard(),
            ReadMode::Dump => std::fs::read(&self.dump).map_or_else(
                |_| uia::NOT_FOUND.to_string(),
                |b| String::from_utf8_lossy(&b).into_owned(),
            ),
        }
    }
    fn clear(&self) {
        // Ctrl+A が効かない入力先(Flutter: Backspace が 1 文字ずつしか消えなかった)があるので、空になったか読んで、
        // 残っていたら 2 回目以降は残りの文字数ぶん Backspace を打つ(最大 4 回)。読み損ね(<uia-...>)のときは 1 回で済ませる。
        let mut remain = 0usize;
        for attempt in 0..4 {
            raise_top(200);
            if attempt == 0 {
                uia::clear_focused();
            } else {
                for _ in 0..remain + 3 {
                    crate::press(0x08, 0x0E, 8);
                }
                sleep_ms(150);
            }
            sleep_ms(150);
            let now = self.read();
            if now.is_empty() || now.starts_with("<uia-") {
                return;
            }
            remain = now.chars().count();
            log(&format!(
                "[ext] clear が効かなかった(残り {remain} 文字)、やり直す attempt={}",
                attempt + 1
            ));
        }
    }
    fn refocus(&self) {
        raise_top(400);
    }
    fn focus_ok(&self) -> bool {
        let ok = foreground_is_top();
        if !ok {
            // SAFETY: 前面窓の取得のみ。
            let fg = unsafe { GetForegroundWindow() };
            log(&format!(
                "[ext] 前面窓が入力先ではない: 前面 class={} title={:?} pid={} / 入力先 class={} title={:?} pid={}",
                class_of(fg),
                title_of(fg),
                pid_of(fg),
                class_of(hwnd_of(&TOP)),
                title_of(hwnd_of(&TOP)),
                self.win_pid
            ));
        }
        ok
    }
    fn shutdown(&self) {
        log_helper_err(&self.dump);
        if let Some(mut h) = self.helper.lock().ok().and_then(|mut g| g.take()) {
            let _ = h.kill();
        }
        kill_tree(self.win_pid);
        kill_tree(self.spawned);
        for ext in ["", ".err", ".tmp", ".helper.log"] {
            let _ = std::fs::remove_file(format!("{}{ext}", self.dump.display()));
        }
        for _ in 0..5 {
            if std::fs::remove_dir_all(&self.profile).is_ok() || !self.profile.exists() {
                break;
            }
            sleep_ms(300);
        }
    }
}
