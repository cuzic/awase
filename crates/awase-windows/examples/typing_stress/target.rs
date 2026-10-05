//! 入力先(`--form=`)の抽象。ハーネス本体(注入・記録・シナリオ)は入力先の種類を知らず、
//! [`InputTarget`] 越しに「読む・空にする・前面へ戻す・フォーカスを確認する・終了する」だけを使う。
//!
//! 新しい入力先を足すときは、(1) `InputTarget` を実装し、(2) [`launch`] の `match` に 1 行足し、
//! (3) `Form`(main.rs)に名前を足す。シナリオ側・CI の構成表側は触らなくてよい。
//!
//! | `--form=`                    | 実体                                           | 読み戻し        | 自プロセス HIMC |
//! |------------------------------|------------------------------------------------|-----------------|-----------------|
//! | `edit`/`multi`/`rich`/`tsf`  | 自前で作る Win32 窓(`create_own_window`)       | `WM_GETTEXT`    | あり            |
//! | `chromebar`/`chromepage`     | 本物の Chrome(専用プロファイル)                | UI Automation   | なし            |
//! | `bugreport`                  | 本物の `awase-settings --bug-report`           | UI Automation   | なし            |
//! | `qt`                         | `line.exe`(Qt QLineEdit、LINE の再現用)        | UI Automation   | なし            |
//!
//! HIMC が無い入力先では `real_ime_open` が `None` になるため、drift 系モード(実 IME の開閉を直接
//! 観測/操作する)は使えない。

use std::path::PathBuf;
use std::sync::atomic::Ordering;

use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Accessibility::IUIAutomationElement;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
    PostMessageW, WM_CLOSE,
};

use crate::uia;
use crate::{class_of, hwnd_of, log, press, raise_foreign, send_key, sleep_ms, Form, CHILD, TOP};

pub(crate) trait InputTarget: Send + Sync {
    /// 入力欄の現在の内容。読めなかったときは `<uia-...>` 形式のセンチネル文字列を返す。
    fn read(&self) -> String;
    /// 入力欄を空にする(フォーカスが入力欄にある前提でよい。無ければ実装側で合わせる)。
    fn clear(&self);
    /// 入力先を前面化し、入力欄にフォーカスを戻す。ワーカースレッドから呼ぶ。
    fn refocus(&self);
    /// 前面窓が入力先で、フォーカスも入力欄にあるか。
    fn focus_ok(&self) -> bool;
    /// 入力先を閉じる(次の構成・試行に持ち越さない)。
    fn shutdown(&self);
    /// 自プロセスの窓で、`ImmGetContext` が使えるか。
    fn owns_himc(&self) -> bool {
        false
    }
}

/// `--form=` に対応する入力先を起動(または作成)する。`TOP`/`CHILD` もここで設定する。
pub(crate) fn launch(form: Form) -> Box<dyn InputTarget> {
    match form {
        Form::Edit | Form::Multi | Form::Rich | Form::Tsf => {
            crate::create_own_window(form);
            Box::new(OwnWindow)
        }
        Form::ChromeBar => Box::new(Chrome::launch(false)),
        Form::ChromePage => Box::new(Chrome::launch(true)),
        Form::BugReport => Box::new(BugReport::launch()),
        Form::Qt => Box::new(QtLine::launch()),
    }
}

// ---------------------------------------------------------------- 自前の Win32 窓

struct OwnWindow;

impl InputTarget for OwnWindow {
    fn read(&self) -> String {
        crate::own_read_text(hwnd_of(&CHILD))
    }
    fn clear(&self) {
        crate::own_clear_text(hwnd_of(&CHILD));
    }
    fn refocus(&self) {
        crate::own_refocus();
    }
    fn focus_ok(&self) -> bool {
        crate::own_focus_ok()
    }
    fn shutdown(&self) {
        post_close();
    }
    fn owns_himc(&self) -> bool {
        true
    }
}

fn post_close() {
    // SAFETY: 自プロセスの最上位窓への PostMessage のみ。
    unsafe {
        let _ = PostMessageW(Some(hwnd_of(&TOP)), WM_CLOSE, WPARAM(0), LPARAM(0));
    }
}

// ---------------------------------------------------------------- 別プロセスの窓の探索・終了

struct Search<'a> {
    pid: u32,
    accept: &'a dyn Fn(HWND) -> bool,
    found: Option<HWND>,
}

unsafe extern "system" fn enum_cb(hwnd: HWND, lp: LPARAM) -> BOOL {
    // SAFETY: `lp` は `find_window` が渡す、呼び出し中だけ生きている `Search` へのポインタ。
    unsafe {
        let s = &mut *(lp.0 as *mut Search);
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&raw mut pid));
        if IsWindowVisible(hwnd).as_bool() && (s.pid == 0 || pid == s.pid) && (s.accept)(hwnd) {
            s.found = Some(hwnd);
            return false.into();
        }
        true.into()
    }
}

/// `pid` のプロセス(0 なら全プロセス)が持つ可視窓のうち `accept` を満たす最初のものを、
/// 最大 `tries` × 500ms 待って探す。
fn find_window(pid: u32, accept: &dyn Fn(HWND) -> bool, tries: usize) -> Option<HWND> {
    for _ in 0..tries {
        sleep_ms(500);
        let mut s = Search {
            pid,
            accept,
            found: None,
        };
        // SAFETY: `s` は EnumWindows の同期呼び出しの間だけ参照される。
        unsafe {
            let _ = EnumWindows(Some(enum_cb), LPARAM(&raw mut s as isize));
        }
        if s.found.is_some() {
            return s.found;
        }
    }
    None
}

fn title_of(h: HWND) -> String {
    let mut buf = [0u16; 256];
    // SAFETY: 窓タイトルの読み取りのみ。
    let n = unsafe { GetWindowTextW(h, &mut buf) };
    String::from_utf16_lossy(&buf[..usize::try_from(n).unwrap_or(0)])
}

/// プロセスツリーごと止める。イメージ名では止めない(開発機で使うとユーザー自身の Chrome を巻き込むため)。
fn kill_tree(pid: u32) {
    let _ = std::process::Command::new("taskkill")
        .args(["/F", "/T", "/PID", &pid.to_string()])
        .output();
}

fn fatal(msg: &str) -> ! {
    log(&format!("[FATAL] {msg}"));
    std::process::exit(2);
}

/// 別プロセスの入力先を前面へ戻す共通部分(フォーカス確認はプロセスごとに違うので呼び出し側で)。
fn raise_top(settle_ms: u64) {
    raise_foreign(hwnd_of(&TOP));
    sleep_ms(settle_ms);
}

fn foreground_is_top() -> bool {
    // SAFETY: 前面窓の取得のみ。
    unsafe { GetForegroundWindow() == hwnd_of(&TOP) }
}

// ---------------------------------------------------------------- 本物の Chrome

/// アドレスバー(`page=false`、Alt+D でフォーカス)またはページ内 textarea(`page=true`、autofocus)。
/// 専用プロファイルで起動する。`--chrome-path=` で実行ファイルを指定できる。
struct Chrome {
    pid: u32,
    page: bool,
    profile: PathBuf,
    page_file: PathBuf,
}

/// ページ内 textarea のアクセシビリティ名(アドレスバーの Edit と区別するため)。
const PAGE_INPUT_NAME: &str = "stress-input";

impl Chrome {
    fn launch(page: bool) -> Self {
        let tmp = std::env::temp_dir();
        let profile = tmp.join(format!("ts-chrome-profile-{}", std::process::id()));
        let html_path = tmp.join(format!("ts-chrome-page-{}.html", std::process::id()));
        let _ = std::fs::write(
            &html_path,
            format!(
                "<!doctype html><meta charset=utf-8><title>ts</title>\n\
                 <textarea id=t aria-label=\"{PAGE_INPUT_NAME}\" autofocus rows=8 cols=80></textarea>\n"
            ),
        );
        let url = if page {
            format!("file:///{}", html_path.to_string_lossy().replace('\\', "/"))
        } else {
            "about:blank".to_string()
        };
        let exe = crate::arg_value("--chrome-path=").unwrap_or_else(|| {
            [
                r"C:\Program Files\Google\Chrome\Application\chrome.exe",
                r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
            ]
            .into_iter()
            .find(|p| std::path::Path::new(p).exists())
            .unwrap_or("chrome.exe")
            .to_string()
        });
        let pid = match std::process::Command::new(&exe)
            .arg(format!("--user-data-dir={}", profile.display()))
            .args([
                "--no-first-run",
                "--no-default-browser-check",
                "--disable-extensions",
                "--force-renderer-accessibility",
                "--new-window",
            ])
            .arg(&url)
            .spawn()
        {
            Ok(c) => c.id(),
            Err(e) => fatal(&format!("Chrome の起動に失敗: {exe} {e}")),
        };
        log(&format!("[init] Chrome 起動 pid={pid} url={url}"));
        let is_chrome_top = |h: HWND| class_of(h) == "Chrome_WidgetWin_1";
        // 起動した pid の窓だけを待つ。pid を問わない探索は、開発機でユーザー自身の Chrome を
        // 操作してしまうので行わない(専用プロファイルなので、起動した pid が窓を持つ)。
        let top = find_window(pid, &is_chrome_top, 120).unwrap_or_else(|| {
            kill_tree(pid);
            fatal("Chrome の窓が見つからない(起動した pid の Chrome_WidgetWin_1 なし)")
        });
        TOP.store(top.0 as isize, Ordering::SeqCst);
        // 初回描画とアクセシビリティツリーの構築を待つ余裕。
        sleep_ms(4000);
        CHILD.store(top.0 as isize, Ordering::SeqCst);
        Self {
            pid,
            page,
            profile,
            page_file: html_path,
        }
    }

    fn focus_omnibox() {
        send_key(0x12, 0x38, true);
        sleep_ms(30);
        press(0x44, 0x20, 30);
        send_key(0x12, 0x38, false);
        sleep_ms(200);
    }
}

impl InputTarget for Chrome {
    fn read(&self) -> String {
        // 目的の Edit(ページ=名前が一致、アドレスバー=一致しない)が値つきで現れるまで待つ。
        // omnibox の Edit は常にあるので、「Edit が 1 つでもあれば良い」とするとレンダラのツリーが
        // 作り直し中の瞬間に、消失ではない読み損ねを消失として数えてしまう。
        uia::wait_edits(hwnd_of(&TOP), |edits| {
            edits.iter().find_map(|el| {
                let is_page_input = uia::name_of(el) == PAGE_INPUT_NAME;
                if is_page_input == self.page {
                    uia::try_read_value(el)
                } else {
                    None
                }
            })
        })
        .unwrap_or_else(|| uia::NOT_FOUND.into())
    }
    fn clear(&self) {
        if !self.page {
            Self::focus_omnibox();
        }
        uia::clear_focused();
    }
    fn refocus(&self) {
        raise_top(400);
        if !self.page {
            Self::focus_omnibox();
        }
    }
    fn focus_ok(&self) -> bool {
        // 別プロセスの内部フォーカスは GetGUIThreadInfo で確定できないので、前面窓だけを見る。
        foreground_is_top()
    }
    fn shutdown(&self) {
        kill_tree(self.pid);
        let _ = std::fs::remove_file(&self.page_file);
        // 子プロセスがプロファイルを解放しきるまで少し待って消す(開発機の %TEMP% に溜めない)。
        for _ in 0..5 {
            if std::fs::remove_dir_all(&self.profile).is_ok() || !self.profile.exists() {
                break;
            }
            sleep_ms(300);
        }
    }
}

// ---------------------------------------------------------------- 本物の awase-settings --bug-report

/// 不具合報告フォーム(ADR-095)の「説明」欄。自プロセスの exe と同じディレクトリの
/// `awase-settings.exe` を起動する(CI の `dist/` はビルド成果物をフラットに置く)。
struct BugReport {
    pid: u32,
}

impl BugReport {
    fn launch() -> Self {
        let exe = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("awase-settings.exe")))
            .unwrap_or_else(|| PathBuf::from("awase-settings.exe"));
        let pid = match std::process::Command::new(&exe).arg("--bug-report").spawn() {
            Ok(c) => c.id(),
            Err(e) => fatal(&format!(
                "awase-settings.exe の起動に失敗: {} {e}",
                exe.display()
            )),
        };
        log(&format!(
            "[init] awase-settings --bug-report 起動 pid={pid} exe={}",
            exe.display()
        ));
        // bug_report.rs::run() の with_title("awase 不具合報告") と一致させる。
        let top =
            find_window(pid, &|h| title_of(h).contains("不具合報告"), 60).unwrap_or_else(|| {
                kill_tree(pid);
                fatal("不具合報告窓が見つからない(タイトル「不具合報告」を含む可視窓なし)")
            });
        TOP.store(top.0 as isize, Ordering::SeqCst);
        // 初回フレーム(CJK フォント読み込み)の完了を待つ余裕。
        sleep_ms(1500);
        CHILD.store(top.0 as isize, Ordering::SeqCst);
        Self { pid }
    }

    /// 「説明」欄の Edit。egui は `.labelled_by` 未使用で Name が空のことがあるため、描画順
    /// (説明欄が先、JSON プレビューが後)に頼って走査順の先頭を選ぶ。順序の仮定が崩れたときに
    /// ログから気付けるよう、`log_all` で見つかった全 Edit の名前と位置を残す。
    fn description(log_all: bool) -> Option<IUIAutomationElement> {
        let mut edits =
            uia::wait_edits(hwnd_of(&TOP), |e| (!e.is_empty()).then_some(e)).unwrap_or_default();
        if log_all {
            for (i, el) in edits.iter().enumerate() {
                // SAFETY: UIA プロパティの読み取りのみ。
                let rect = unsafe { el.CurrentBoundingRectangle().ok() };
                log(&format!(
                    "[bugreport] edit#{i} name={:?} rect={rect:?}",
                    uia::name_of(el)
                ));
            }
        }
        if edits.is_empty() {
            log("[bugreport] 説明欄の Edit が見つからない");
            return None;
        }
        Some(edits.remove(0))
    }

    fn focus_description() {
        if let Some(el) = Self::description(true) {
            // SAFETY: UIA 要素へのフォーカス設定のみ。
            if let Err(e) = unsafe { el.SetFocus() } {
                log(&format!("[bugreport] SetFocus 失敗: {e}"));
            }
            sleep_ms(150);
        }
    }
}

impl InputTarget for BugReport {
    fn read(&self) -> String {
        Self::description(false)
            .map_or_else(|| uia::NOT_FOUND.to_string(), |el| uia::read_value(&el))
    }
    fn clear(&self) {
        Self::focus_description();
        uia::clear_focused();
    }
    fn refocus(&self) {
        raise_top(300);
        Self::focus_description();
    }
    fn focus_ok(&self) -> bool {
        foreground_is_top()
    }
    fn shutdown(&self) {
        post_close();
        sleep_ms(500);
        // WM_CLOSE で閉じ損ねた場合の保険(次の構成・試行を巻き込まないため)。
        kill_tree(self.pid);
    }
}

// ---------------------------------------------------------------- Qt の QLineEdit(line.exe)

/// LINE(Qt)の症状の再現用。自プロセスの exe と同じディレクトリの `line.exe`
/// (tools/e2e/qt_line_input をプロセス名だけ LINE に合わせたもの)を起動し、唯一の Edit を読む。
struct QtLine {
    pid: u32,
}

impl QtLine {
    fn launch() -> Self {
        let exe = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("line.exe")))
            .unwrap_or_else(|| PathBuf::from("line.exe"));
        let pid = match std::process::Command::new(&exe).spawn() {
            Ok(c) => c.id(),
            Err(e) => fatal(&format!("line.exe の起動に失敗: {} {e}", exe.display())),
        };
        log(&format!(
            "[init] line.exe 起動 pid={pid} exe={}",
            exe.display()
        ));
        let top =
            find_window(pid, &|h| title_of(h).contains("qt-line-input"), 60).unwrap_or_else(|| {
                kill_tree(pid);
                fatal("Qt 窓が見つからない(タイトル qt-line-input の可視窓なし)")
            });
        log(&format!("[init] qt top class={}", class_of(top)));
        TOP.store(top.0 as isize, Ordering::SeqCst);
        sleep_ms(800);
        CHILD.store(top.0 as isize, Ordering::SeqCst);
        Self { pid }
    }

    fn edit() -> Option<IUIAutomationElement> {
        let mut edits =
            uia::wait_edits(hwnd_of(&TOP), |e| (!e.is_empty()).then_some(e)).unwrap_or_default();
        if edits.is_empty() {
            log("[qt] Edit が見つからない");
            return None;
        }
        Some(edits.remove(0))
    }

    fn focus_edit() {
        if let Some(el) = Self::edit() {
            // SAFETY: UIA 要素へのフォーカス設定のみ。
            if let Err(e) = unsafe { el.SetFocus() } {
                log(&format!("[qt] SetFocus 失敗: {e}"));
            }
            sleep_ms(150);
        }
    }
}

impl InputTarget for QtLine {
    fn read(&self) -> String {
        Self::edit().map_or_else(|| uia::NOT_FOUND.to_string(), |el| uia::read_value(&el))
    }
    fn clear(&self) {
        Self::focus_edit();
        uia::clear_focused();
    }
    fn refocus(&self) {
        raise_top(300);
        Self::focus_edit();
    }
    fn focus_ok(&self) -> bool {
        foreground_is_top()
    }
    fn shutdown(&self) {
        post_close();
        sleep_ms(500);
        kill_tree(self.pid);
    }
}
