#![allow(clippy::all, clippy::pedantic, clippy::nursery)]
//! ADR-199 T12/T17: Windows標準設定アプリの「キーとタッチのカスタマイズ」→「キーの割り当て」
//! （`SystemSettings_Language_JapaneseIME_IsKeyAssignmentEnabled_ToggleSwitch`ほか）を
//! `IUIAutomation`経由で読み書きするツール。
//!
//! # 背景
//!
//! `KeyAssignmentMuhenkan`/`KeyAssignmentHenkan`等のレジストリ値は、直接書き込んでも
//! 実際のMS-IME変換エンジンには反映されない（ADR-199 T12で実機確認済み）。実際に反映させる
//! には、設定アプリのUIを操作して値を選ぶ必要がある。T12はこれを実機での一回限りの手動操作
//! （コミットされたスクリプトなし）で確認したため、再検証のたびに同じ手作業を繰り返す羽目に
//! なっていた。このツールはその操作をコード化し、再実行可能にする。
//!
//! # 使い方（Windows実機のみ）
//!
//! ```powershell
//! # 現在のページ構造をダンプする（既定の挙動。レジストリ/UI状態の変更はしないが、
//! # ナビゲーションのため既存の設定ウィンドウを一度閉じる副作用がある、後述）
//! cargo run -p awase-windows --example msime_key_assignment_settings_probe --release
//!
//! # 「各キー/キーの組み合わせに好みの機能を割り当てる」マスタースイッチをON/OFFする
//! cargo run -p awase-windows --example msime_key_assignment_settings_probe --release -- --set-master=on
//! cargo run -p awase-windows --example msime_key_assignment_settings_probe --release -- --set-master=off
//!
//! # 無変換/変換キーの割り当てを変更する（ComboBoxの表示テキストと完全一致させること、例:
//! # "IME-オン/オフ" "ひらがな/カタカナ" "再変換" 等。表示言語がUI言語に依存する点に注意）
//! cargo run -p awase-windows --example msime_key_assignment_settings_probe --release -- --set-muhenkan="IME-オン/オフ"
//! cargo run -p awase-windows --example msime_key_assignment_settings_probe --release -- --set-henkan="IME-オン/オフ"
//! ```
//!
//! 複数のフラグを同時に指定できる。いずれの変更後も
//! `HKCU\Software\Microsoft\IME\15.0\IMEJP\MSIME`の`IsKeyAssignmentEnabled`/
//! `KeyAssignmentMuhenkan`/`KeyAssignmentHenkan`をログに出す。`--set-master`の値は
//! `on`/`off`のみ受け付ける（それ以外は起動時にエラー終了する）。
//!
//! # 副作用: 既存の設定ウィンドウを閉じる
//!
//! **ダンプ専用モード（既定の挙動）を含め、実行するたびに`SystemSettings.exe`を
//! `taskkill`で強制終了する。** 既存の設定ウィンドウが開いたまま残っていると、
//! `ms-settings:`URIの遷移が効かず既存インスタンスが単に前面化されるだけになることが
//! あるため（実機確認）。「読み取り専用」なのはレジストリ/UI状態を書き換えないという
//! 意味であり、ユーザーが別途開いていた設定ウィンドウ（未保存の入力があればそれも）は
//! この実行で失われる。
//!
//! # 実機での既知の注意点（ADR-199 opusレビュー・実機確認で判明）
//!
//! - 表示言語が英語のセッションではこのページ自体は開けるが、「キーの割り当て」セクションは
//!   MS-IME互換モード（「以前のバージョンのMicrosoft IMEを使う」）がONだと表示されない。
//! - 表示言語（UI Culture）は`Set-Culture`等をスクリプト内で呼んでも次回サインインまで
//!   反映されないため、CI（windows-latestランナー、英語UI）では確認できなかった。実機
//!   （日本語UIの通常セッション）でのみ確認できている。

#![allow(unsafe_code)]

#[cfg(windows)]
mod windows_probe {
    use std::time::{Duration, Instant};

    use windows::core::{w, BOOL, PCWSTR};
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::Accessibility::{
        CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationTreeWalker,
    };
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible, SW_SHOWNORMAL,
    };

    /// `HKCU\Software\Microsoft\IME\15.0\IMEJP\MSIME`のDWORD値を読む（`msime_key_assignment.rs`の
    /// `read_dword`と同じロジック。exampleはクレート内部のpub(crate)関数を参照できないため再実装）。
    fn read_msime_dword(value_name: PCWSTR) -> Option<u32> {
        use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
        let subkey = w!("Software\\Microsoft\\IME\\15.0\\IMEJP\\MSIME");
        let mut data: u32 = 0;
        let mut size = u32::try_from(size_of::<u32>()).unwrap_or(4);
        // SAFETY: HKEY_CURRENT_USER は擬似ハンドル。subkey/value_name はNUL終端済みUTF-16。
        //         data/size は呼び出し中有効なスタック上のバッファ。
        let result = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                subkey,
                value_name,
                RRF_RT_REG_DWORD,
                None,
                Some((&raw mut data).cast()),
                Some(&raw mut size),
            )
        };
        result.is_ok().then_some(data)
    }

    fn log_msime_registry_snapshot(label: &str) {
        let enabled = read_msime_dword(w!("IsKeyAssignmentEnabled"));
        let muhenkan = read_msime_dword(w!("KeyAssignmentMuhenkan"));
        let henkan = read_msime_dword(w!("KeyAssignmentHenkan"));
        log(&format!(
            "[registry:{label}] IsKeyAssignmentEnabled={enabled:?} KeyAssignmentMuhenkan={muhenkan:?} KeyAssignmentHenkan={henkan:?}"
        ));
    }

    fn now_ms() -> u128 {
        static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        START.get_or_init(Instant::now).elapsed().as_millis()
    }

    fn log(msg: &str) {
        use std::io::Write as _;
        let line = format!("[{:>8}ms] {msg}", now_ms());
        println!("{line}");
        let _ = std::io::stdout().flush();
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("msime_key_assignment_settings_probe.log")
        {
            use std::io::Write as _;
            let _ = writeln!(f, "{line}");
        }
    }

    fn process_name_of(hwnd: HWND) -> String {
        let mut pid: u32 = 0;
        // SAFETY: hwnd は EnumWindows のコールバックが渡した値。
        unsafe { GetWindowThreadProcessId(hwnd, Some(&raw mut pid)) };
        if pid == 0 {
            return "?".to_string();
        }
        // SAFETY: pid は直前に取得した有効な PID。
        let Ok(handle) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) })
        else {
            return "?".to_string();
        };
        let mut buf = [0u16; 260];
        let mut len = u32::try_from(buf.len()).unwrap_or(0);
        // SAFETY: handle は有効なプロセスハンドル、buf はスタック上有効。
        let ok = unsafe {
            QueryFullProcessImageNameW(
                handle,
                PROCESS_NAME_WIN32,
                windows::core::PWSTR(buf.as_mut_ptr()),
                &raw mut len,
            )
        };
        // SAFETY: handle はここでのみ閉じる。
        let _ = unsafe { windows::Win32::Foundation::CloseHandle(handle) };
        if ok.is_err() {
            return "?".to_string();
        }
        let full = String::from_utf16_lossy(&buf[..len as usize]);
        full.rsplit('\\').next().unwrap_or(&full).to_string()
    }

    fn title_of(hwnd: HWND) -> String {
        let mut buf = [0u16; 512];
        // SAFETY: hwnd は EnumWindows のコールバックが渡した値、buf はスタック上有効。
        let len = unsafe { GetWindowTextW(hwnd, &mut buf) };
        usize::try_from(len)
            .ok()
            .filter(|&n| n > 0)
            .map_or_else(String::new, |n| String::from_utf16_lossy(&buf[..n]))
    }

    struct WindowInfo {
        hwnd: HWND,
        title: String,
        process: String,
    }

    fn enumerate_top_level_windows() -> Vec<WindowInfo> {
        thread_local! {
            static COLLECTED: std::cell::RefCell<Vec<WindowInfo>> = const { std::cell::RefCell::new(Vec::new()) };
        }
        COLLECTED.with(|c| c.borrow_mut().clear());

        unsafe extern "system" fn callback(hwnd: HWND, _lparam: LPARAM) -> BOOL {
            // SAFETY: hwnd は EnumWindows が渡す値。
            if unsafe { IsWindowVisible(hwnd) }.as_bool() {
                let info = WindowInfo {
                    hwnd,
                    title: title_of(hwnd),
                    process: process_name_of(hwnd),
                };
                COLLECTED.with(|c| c.borrow_mut().push(info));
            }
            BOOL(1)
        }

        // SAFETY: callback は 'static 関数ポインタ、lparam は未使用。
        let _ = unsafe { EnumWindows(Some(callback), LPARAM(0)) };
        COLLECTED.with(std::cell::RefCell::take)
    }

    /// 設定アプリの「地域と言語」ページのウィンドウかどうかを厳密に判定する。
    ///
    /// 以前はプロセス名/タイトルの部分一致（`contains("settings")`等）で判定しており、
    /// ターミナルやUWPアプリ（例: タイトルに"region"を含むアプリ）を誤検出しうる穴が
    /// あった（PR #348レビュー指摘）。プロセス名は完全一致、`ApplicationFrameHost.exe`
    /// （UWPアプリの共通ホスト）の場合のみタイトルの完全一致で絞り込む。
    fn looks_like_settings_window(w: &WindowInfo) -> bool {
        let p = w.process.to_ascii_lowercase();
        if p == "systemsettings.exe" {
            return true;
        }
        if p == "applicationframehost.exe" {
            let t = w.title.trim();
            return t == "設定" || t == "Settings";
        }
        false
    }

    /// UIA要素ツリーを浅く辿ってログに残す（深さ・件数に上限、無限ループ防止）。
    unsafe fn dump_uia_tree(
        walker: &IUIAutomationTreeWalker,
        element: &IUIAutomationElement,
        depth: u32,
        max_depth: u32,
        budget: &mut u32,
    ) {
        if depth > max_depth || *budget == 0 {
            return;
        }
        let name = element
            .CurrentName()
            .map(|s| s.to_string())
            .unwrap_or_default();
        let automation_id = element
            .CurrentAutomationId()
            .map(|s| s.to_string())
            .unwrap_or_default();
        let control_type = element.CurrentControlType().map(|c| c.0).unwrap_or(-1);
        let class_name = element
            .CurrentClassName()
            .map(|s| s.to_string())
            .unwrap_or_default();
        let indent = "  ".repeat(depth as usize);
        // トグル/チェックボックスなら状態も出す(ADR-248: 互換性チェックボックスの実状態を読む)。
        // SAFETY: element は呼び出し元が渡した有効な COM オブジェクト。
        let toggle = unsafe { toggle_state_is_on(element) }
            .map(|on| format!(" toggle={}", if on { "On" } else { "Off" }))
            .unwrap_or_default();
        log(&format!(
            "{indent}[uia] name={name:?} automation_id={automation_id:?} control_type={control_type} class={class_name:?}{toggle}"
        ));
        *budget -= 1;

        // SAFETY: element/walker は呼び出し元が渡した有効な COM オブジェクト。
        let Ok(mut child) = (unsafe { walker.GetFirstChildElement(element) }) else {
            return;
        };
        loop {
            // SAFETY: child は直前に取得した有効な COM オブジェクト。
            unsafe { dump_uia_tree(walker, &child, depth + 1, max_depth, budget) };
            if *budget == 0 {
                return;
            }
            // SAFETY: walker/child は有効な COM オブジェクト。
            let Ok(next) = (unsafe { walker.GetNextSiblingElement(&child) }) else {
                return;
            };
            child = next;
        }
    }


    /// `automation_id`が`id`で、かつ名前が`name_needles`のどれかを(大文字小文字を無視して)含む最初の要素を探す
    /// (ADR-248: 「以前のバージョンの Microsoft IME を使う」トグルは `DialogToggle` が他と重なるので名前でも絞る)。
    unsafe fn find_by_id_and_name(
        walker: &IUIAutomationTreeWalker,
        element: &IUIAutomationElement,
        id: &str,
        name_needles: &[&str],
        depth: u32,
        max_depth: u32,
        budget: &mut u32,
    ) -> Option<IUIAutomationElement> {
        if depth > max_depth || *budget == 0 {
            return None;
        }
        *budget -= 1;
        let automation_id = element
            .CurrentAutomationId()
            .map(|s| s.to_string())
            .unwrap_or_default();
        let name = element
            .CurrentName()
            .map(|s| s.to_string().to_lowercase())
            .unwrap_or_default();
        if automation_id == id && name_needles.iter().any(|n| name.contains(&n.to_lowercase())) {
            return Some(element.clone());
        }
        // SAFETY: element/walker は呼び出し元が渡した有効な COM オブジェクト。
        let Ok(mut child) = (unsafe { walker.GetFirstChildElement(element) }) else {
            return None;
        };
        loop {
            // SAFETY: child は直前に取得した有効な COM オブジェクト。
            if let Some(found) = unsafe {
                find_by_id_and_name(walker, &child, id, name_needles, depth + 1, max_depth, budget)
            } {
                return Some(found);
            }
            if *budget == 0 {
                return None;
            }
            // SAFETY: walker/child は有効な COM オブジェクト。
            let Ok(next) = (unsafe { walker.GetNextSiblingElement(&child) }) else {
                return None;
            };
            child = next;
        }
    }

    /// `automation_id`に`needle`を含む最初の要素を深さ優先で探す（見つからなければ`None`）。
    unsafe fn find_by_automation_id(
        walker: &IUIAutomationTreeWalker,
        element: &IUIAutomationElement,
        needle: &str,
        depth: u32,
        max_depth: u32,
        budget: &mut u32,
    ) -> Option<IUIAutomationElement> {
        if depth > max_depth || *budget == 0 {
            return None;
        }
        *budget -= 1;
        let automation_id = element
            .CurrentAutomationId()
            .map(|s| s.to_string())
            .unwrap_or_default();
        if automation_id.contains(needle) {
            return Some(element.clone());
        }
        // SAFETY: element/walker は呼び出し元が渡した有効な COM オブジェクト。
        let Ok(mut child) = (unsafe { walker.GetFirstChildElement(element) }) else {
            return None;
        };
        loop {
            // SAFETY: child は直前に取得した有効な COM オブジェクト。
            if let Some(found) = unsafe {
                find_by_automation_id(walker, &child, needle, depth + 1, max_depth, budget)
            } {
                return Some(found);
            }
            if *budget == 0 {
                return None;
            }
            // SAFETY: walker/child は有効な COM オブジェクト。
            let Ok(next) = (unsafe { walker.GetNextSiblingElement(&child) }) else {
                return None;
            };
            child = next;
        }
    }

    /// `automation_id`ではなく`Name`が完全一致する最初の要素を探す（ComboBoxの項目等、AutomationIdが
    /// 空でNameだけが手がかりの場合に使う）。
    unsafe fn find_by_name(
        walker: &IUIAutomationTreeWalker,
        element: &IUIAutomationElement,
        name: &str,
        depth: u32,
        max_depth: u32,
        budget: &mut u32,
    ) -> Option<IUIAutomationElement> {
        if depth > max_depth || *budget == 0 {
            return None;
        }
        *budget -= 1;
        let current_name = element
            .CurrentName()
            .map(|s| s.to_string())
            .unwrap_or_default();
        if current_name == name {
            return Some(element.clone());
        }
        // SAFETY: element/walker は呼び出し元が渡した有効な COM オブジェクト。
        let Ok(mut child) = (unsafe { walker.GetFirstChildElement(element) }) else {
            return None;
        };
        loop {
            // SAFETY: child は直前に取得した有効な COM オブジェクト。
            if let Some(found) =
                unsafe { find_by_name(walker, &child, name, depth + 1, max_depth, budget) }
            {
                return Some(found);
            }
            if *budget == 0 {
                return None;
            }
            // SAFETY: walker/child は有効な COM オブジェクト。
            let Ok(next) = (unsafe { walker.GetNextSiblingElement(&child) }) else {
                return None;
            };
            child = next;
        }
    }

    /// `IUIAutomationInvokePattern`で要素をクリックしたのと同じ効果を起こす（ボタン等）。
    unsafe fn invoke_element(element: &IUIAutomationElement) -> windows::core::Result<()> {
        use windows::Win32::UI::Accessibility::{IUIAutomationInvokePattern, UIA_InvokePatternId};
        // SAFETY: element は呼び出し元が渡した有効な COM オブジェクト。
        let pattern: IUIAutomationInvokePattern =
            unsafe { element.GetCurrentPatternAs(UIA_InvokePatternId) }?;
        // SAFETY: pattern は直前に取得した有効な COM オブジェクト。
        unsafe { pattern.Invoke() }
    }

    /// `IUIAutomationExpandCollapsePattern`で要素（ComboBox等）を展開する。
    unsafe fn expand_element(element: &IUIAutomationElement) -> windows::core::Result<()> {
        use windows::Win32::UI::Accessibility::{
            IUIAutomationExpandCollapsePattern, UIA_ExpandCollapsePatternId,
        };
        // SAFETY: element は呼び出し元が渡した有効な COM オブジェクト。
        let pattern: IUIAutomationExpandCollapsePattern =
            unsafe { element.GetCurrentPatternAs(UIA_ExpandCollapsePatternId) }?;
        // SAFETY: pattern は直前に取得した有効な COM オブジェクト。
        unsafe { pattern.Expand() }
    }

    /// `IUIAutomationExpandCollapsePattern`で要素（ComboBox等）を折りたたむ。
    unsafe fn collapse_element(element: &IUIAutomationElement) -> windows::core::Result<()> {
        use windows::Win32::UI::Accessibility::{
            IUIAutomationExpandCollapsePattern, UIA_ExpandCollapsePatternId,
        };
        // SAFETY: element は呼び出し元が渡した有効な COM オブジェクト。
        let pattern: IUIAutomationExpandCollapsePattern =
            unsafe { element.GetCurrentPatternAs(UIA_ExpandCollapsePatternId) }?;
        // SAFETY: pattern は直前に取得した有効な COM オブジェクト。
        unsafe { pattern.Collapse() }
    }

    /// `IUIAutomationSelectionItemPattern`で要素（ComboBoxItem等）を選択する。
    unsafe fn select_element(element: &IUIAutomationElement) -> windows::core::Result<()> {
        use windows::Win32::UI::Accessibility::{
            IUIAutomationSelectionItemPattern, UIA_SelectionItemPatternId,
        };
        // SAFETY: element は呼び出し元が渡した有効な COM オブジェクト。
        let pattern: IUIAutomationSelectionItemPattern =
            unsafe { element.GetCurrentPatternAs(UIA_SelectionItemPatternId) }?;
        // SAFETY: pattern は直前に取得した有効な COM オブジェクト。
        unsafe { pattern.Select() }
    }

    /// `IUIAutomationTogglePattern`で要素（ToggleSwitch/CheckBox）の現在の状態を読む。
    unsafe fn toggle_state_is_on(element: &IUIAutomationElement) -> Option<bool> {
        use windows::Win32::UI::Accessibility::{
            IUIAutomationTogglePattern, ToggleState_On, UIA_TogglePatternId,
        };
        // SAFETY: element は呼び出し元が渡した有効な COM オブジェクト。
        let pattern: IUIAutomationTogglePattern =
            unsafe { element.GetCurrentPatternAs(UIA_TogglePatternId) }.ok()?;
        // SAFETY: pattern は直前に取得した有効な COM オブジェクト。
        let state = unsafe { pattern.CurrentToggleState() }.ok()?;
        Some(state == ToggleState_On)
    }

    /// `IUIAutomationTogglePattern`で要素をトグルする。
    unsafe fn toggle_element(element: &IUIAutomationElement) -> windows::core::Result<()> {
        use windows::Win32::UI::Accessibility::{IUIAutomationTogglePattern, UIA_TogglePatternId};
        // SAFETY: element は呼び出し元が渡した有効な COM オブジェクト。
        let pattern: IUIAutomationTogglePattern =
            unsafe { element.GetCurrentPatternAs(UIA_TogglePatternId) }?;
        // SAFETY: pattern は直前に取得した有効な COM オブジェクト。
        unsafe { pattern.Toggle() }
    }

    /// ComboBoxを展開し、`item_name`と完全一致する項目を選ぶ（WinUIのComboBoxは選択で自動的に閉じる）。
    unsafe fn select_combo_item(
        walker: &IUIAutomationTreeWalker,
        combo: &IUIAutomationElement,
        item_name: &str,
    ) -> Result<(), String> {
        // SAFETY: combo は呼び出し元が渡した有効な COM オブジェクト。
        unsafe { expand_element(combo) }.map_err(|e| format!("expand failed: {e:?}"))?;
        std::thread::sleep(Duration::from_millis(500));
        let mut budget: u32 = 100;
        // SAFETY: walker/combo は直前に取得した有効な COM オブジェクト。
        let Some(item) = (unsafe { find_by_name(walker, combo, item_name, 0, 4, &mut budget) })
        else {
            // 項目が見つからなかった場合、展開したままにするとUIが操作しづらい状態で
            // 残る（PR #348レビュー指摘）。折りたたんでから失敗を返す。
            // SAFETY: combo は呼び出し元が渡した有効な COM オブジェクト。
            let _ = unsafe { collapse_element(combo) };
            return Err(format!(
                "item {item_name:?} not found (budget left {budget})"
            ));
        };
        // SAFETY: item は直前に取得した有効な COM オブジェクト。
        if let Err(e) = unsafe { select_element(&item) } {
            // SAFETY: combo は呼び出し元が渡した有効な COM オブジェクト。
            let _ = unsafe { collapse_element(combo) };
            return Err(format!("select failed: {e:?}"));
        }
        std::thread::sleep(Duration::from_millis(500));
        Ok(())
    }

    pub(super) fn run() {
        let args: Vec<String> = std::env::args().collect();
        let set_master = args
            .iter()
            .find_map(|a| a.strip_prefix("--set-master=").map(str::to_owned));
        if let Some(v) = &set_master {
            if v != "on" && v != "off" {
                eprintln!("invalid --set-master value {v:?}, expected \"on\" or \"off\"");
                std::process::exit(2);
            }
        }
        let set_muhenkan = args
            .iter()
            .find_map(|a| a.strip_prefix("--set-muhenkan=").map(str::to_owned));
        let set_henkan = args
            .iter()
            .find_map(|a| a.strip_prefix("--set-henkan=").map(str::to_owned));
        let dump_only = set_master.is_none() && set_muhenkan.is_none() && set_henkan.is_none();

        let _ = std::fs::remove_file("msime_key_assignment_settings_probe.log");
        log("msime_key_assignment_settings_probe start");

        // SAFETY: プロセス起動直後の唯一の COM 初期化呼び出し。
        let com_init = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if com_init.is_err() {
            log(&format!("CoInitializeEx failed: {com_init:?}"));
        }

        log("opening ms-settings:regionlanguage-jpnime via ShellExecuteW");
        // 既存の設定ウィンドウが開いたまま残っていると、ms-settings: URIの遷移が効かず
        // 既存インスタンスが単に前面化されるだけになることがある（実機確認）。ダンプ専用
        // モードでもこの副作用は発生する（モジュール doc の「副作用」節に明記済み）。
        let _ = std::process::Command::new("taskkill")
            .args(["/f", "/im", "SystemSettings.exe"])
            .output();
        std::thread::sleep(Duration::from_millis(300));

        // SAFETY: 引数はすべて静的リテラルの NUL 終端 UTF-16。
        let exec_result = unsafe {
            ShellExecuteW(
                None,
                w!("open"),
                w!("ms-settings:regionlanguage-jpnime"),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        };
        log(&format!(
            "ShellExecuteW result={:?} (>32 means success)",
            exec_result.0 as isize
        ));

        let deadline = Instant::now() + Duration::from_secs(25);
        let mut candidates: Vec<WindowInfo> = Vec::new();
        while Instant::now() < deadline {
            let windows_found = enumerate_top_level_windows();
            let settings_like: Vec<WindowInfo> = windows_found
                .into_iter()
                .filter(looks_like_settings_window)
                .collect();
            if !settings_like.is_empty() {
                candidates = settings_like;
                break;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        if candidates.is_empty() {
            log("RESULT: no settings-like window appeared within timeout.");
            return;
        }
        log("waiting 3s for page content to settle...");
        std::thread::sleep(Duration::from_secs(3));

        // SAFETY: com_init が成功していることを前提に、同一スレッドで CoCreateInstance を呼ぶ。
        let automation: windows::core::Result<IUIAutomation> =
            unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) };
        let automation = match automation {
            Ok(a) => a,
            Err(e) => {
                log(&format!("CoCreateInstance(CUIAutomation) failed: {e:?}"));
                return;
            }
        };
        // SAFETY: automation は有効な COM オブジェクト。
        let walker = match unsafe { automation.ControlViewWalker() } {
            Ok(w) => w,
            Err(e) => {
                log(&format!("ControlViewWalker failed: {e:?}"));
                return;
            }
        };

        // SystemSettings.exe側(実コンテンツを持つCoreWindow)を優先する。無ければ最初の候補。
        let root_candidate = candidates
            .iter()
            .find(|c| c.process.eq_ignore_ascii_case("SystemSettings.exe"))
            .unwrap_or(&candidates[0]);

        // SAFETY: automation は直前に取得した有効な COM オブジェクト、hwnd は EnumWindows が返した
        //         有効なウィンドウハンドル。
        let root_element = match unsafe { automation.ElementFromHandle(root_candidate.hwnd) } {
            Ok(e) => e,
            Err(e) => {
                log(&format!("ElementFromHandle(root) failed: {e:?}"));
                return;
            }
        };

        // `--dump-root`: ボタンを押す前の最初のページ(Microsoft IME のトップ)を先にダンプする
        // (互換性チェックボックスの状態を読む。ADR-248)。
        if args.iter().any(|a| a == "--dump-root") {
            log("=== dumping root page (Microsoft IME) ===");
            let mut b: u32 = 1500;
            // SAFETY: walker/root_element は直前に取得した有効な COM オブジェクト。
            unsafe { dump_uia_tree(&walker, &root_element, 0, 14, &mut b) };
            log(&format!("RESULT: root dump done, remaining_budget={b}"));
        }
        // `--general-only`: 「General」サブページ(入力設定・互換性など)を開いてダンプして終わる
        // (互換性チェックボックス「以前のバージョンの Microsoft IME を使う」の状態を読む。ADR-248)。
        if args.iter().any(|a| a == "--general-only") {
            let mut b: u32 = 800;
            // SAFETY: walker/root_element は直前に取得した有効な COM オブジェクト。
            let general = unsafe {
                find_by_automation_id(
                    &walker,
                    &root_element,
                    "SystemSettings_Language_JapaneseIME_General_Link_JapaneseIMEGeneralLinkButton",
                    0,
                    14,
                    &mut b,
                )
            };
            let Some(general) = general else {
                log("RESULT: General button not found on root page.");
                return;
            };
            // SAFETY: general は直前に取得した有効な COM オブジェクト。
            if let Err(e) = unsafe { invoke_element(&general) } {
                log(&format!("RESULT: invoke_element(General) failed: {e:?}"));
                return;
            }
            std::thread::sleep(Duration::from_secs(3));
            // SAFETY: 同一ウィンドウ内の SPA 遷移。
            let page = match unsafe { automation.ElementFromHandle(root_candidate.hwnd) } {
                Ok(e) => e,
                Err(e) => {
                    log(&format!("ElementFromHandle(general) failed: {e:?}"));
                    return;
                }
            };
            log("=== dumping 'General' page ===");
            let mut b2: u32 = 1500;
            // SAFETY: walker/page は直前に取得した有効な COM オブジェクト。
            unsafe { dump_uia_tree(&walker, &page, 0, 14, &mut b2) };
            log(&format!("RESULT: general dump done, remaining_budget={b2}"));
            // `--set-compat=on|off`: 「以前のバージョンの Microsoft IME を使う」トグルを設定アプリ経由で切り替える(ADR-248)。
            if let Some(want) = args.iter().find_map(|a| a.strip_prefix("--set-compat=")) {
                let want_on = want == "on";
                let mut b3: u32 = 1500;
                // SAFETY: walker/page は直前に取得した有効な COM オブジェクト。
                let toggle = unsafe {
                    find_by_id_and_name(
                        &walker,
                        &page,
                        "DialogToggle",
                        &["以前のバージョン", "previous version"],
                        0,
                        14,
                        &mut b3,
                    )
                };
                let Some(toggle) = toggle else {
                    log("RESULT: --set-compat: compat toggle not found");
                    return;
                };
                // SAFETY: toggle は直前に取得した有効な COM オブジェクト。
                let was_on = unsafe { toggle_state_is_on(&toggle) };
                log(&format!("--set-compat={want}: current toggle state = {was_on:?}"));
                if was_on != Some(want_on) {
                    // SAFETY: toggle は直前に取得した有効な COM オブジェクト。
                    if let Err(e) = unsafe { toggle_element(&toggle) } {
                        log(&format!("RESULT: toggle_element(compat) failed: {e:?}"));
                    }
                    std::thread::sleep(Duration::from_secs(5));
                    // SAFETY: toggle は直前に取得した有効な COM オブジェクト。
                    let now = unsafe { toggle_state_is_on(&toggle) };
                    log(&format!("--set-compat={want}: toggle state after = {now:?}"));
                }
                log_msime_registry_snapshot("after-set-compat");
            }
            return;
        }
        // 「Key and touch customization」(キーとタッチのカスタマイズ)ボタンをAutomationIdで探す。
        // 表示言語が英語でもAutomationIdは言語非依存で安定している。
        let mut budget: u32 = 800;
        // SAFETY: walker/root_element は直前に取得した有効な COM オブジェクト。
        let key_and_touch = unsafe {
            find_by_automation_id(
                &walker,
                &root_element,
                "KeyAndTouchCustomization",
                0,
                10,
                &mut budget,
            )
        };
        let Some(key_and_touch) = key_and_touch else {
            log("RESULT: KeyAndTouchCustomization button not found on root page.");
            return;
        };
        // SAFETY: key_and_touch は直前に取得した有効な COM オブジェクト。
        if let Err(e) = unsafe { invoke_element(&key_and_touch) } {
            log(&format!(
                "RESULT: invoke_element(KeyAndTouchCustomization) failed: {e:?}"
            ));
            return;
        }
        std::thread::sleep(Duration::from_secs(2));

        // SAFETY: automation/root_candidate.hwnd は同一ウィンドウ内でSPA遷移しているはず。
        let page = match unsafe { automation.ElementFromHandle(root_candidate.hwnd) } {
            Ok(e) => e,
            Err(e) => {
                log(&format!("ElementFromHandle(page) failed: {e:?}"));
                return;
            }
        };

        if dump_only {
            log("=== dumping 'Key and touch customization' page ===");
            let mut budget: u32 = 800;
            // SAFETY: walker/page は直前に取得した有効な COM オブジェクト。
            unsafe { dump_uia_tree(&walker, &page, 0, 14, &mut budget) };
            log(&format!(
                "RESULT: uia tree dump done, remaining_budget={budget}"
            ));
            log_msime_registry_snapshot("dump");
            log("msime_key_assignment_settings_probe end (dump mode)");
            return;
        }

        let find_id = |needle: &str| -> Option<IUIAutomationElement> {
            let mut budget = 800;
            // SAFETY: walker/page は直前に取得した有効な COM オブジェクト。
            unsafe { find_by_automation_id(&walker, &page, needle, 0, 14, &mut budget) }
        };

        log_msime_registry_snapshot("before");

        if let Some(target) = set_master {
            let want_on = target == "on";
            if let Some(master_toggle) = find_id("IsKeyAssignmentEnabled_ToggleSwitch") {
                // SAFETY: master_toggle は直前に取得した有効な COM オブジェクト。
                let was_on = unsafe { toggle_state_is_on(&master_toggle) };
                match was_on {
                    None => {
                        log(
                            "RESULT: --set-master: could not read current toggle state, skipping toggle (unsafe to flip blind).",
                        );
                    }
                    Some(current) if current == want_on => {
                        log(&format!("--set-master={target}: already in desired state"));
                    }
                    Some(_) => {
                        log(&format!("--set-master={target}: toggling..."));
                        // SAFETY: master_toggle は直前に取得した有効な COM オブジェクト。
                        if let Err(e) = unsafe { toggle_element(&master_toggle) } {
                            log(&format!("RESULT: toggle_element(master) failed: {e:?}"));
                        }
                        std::thread::sleep(Duration::from_millis(500));
                        // SAFETY: master_toggle は直前に取得した有効な COM オブジェクト。
                        let now_on = unsafe { toggle_state_is_on(&master_toggle) };
                        if now_on == Some(want_on) {
                            log(&format!(
                                "--set-master={target}: verified new state = {now_on:?}"
                            ));
                        } else {
                            log(&format!(
                                "RESULT: --set-master={target}: verification failed, state after toggle = {now_on:?}"
                            ));
                        }
                    }
                }
            } else {
                log("RESULT: master toggle not found, skipping --set-master.");
            }
        }

        if let Some(name) = set_muhenkan {
            if let Some(combo) = find_id("KeyAssignment_Muhenkan_MSIME_ComboBox") {
                log(&format!("--set-muhenkan={name:?}: selecting..."));
                // SAFETY: walker/combo は直前に取得した有効な COM オブジェクト。
                if let Err(e) = unsafe { select_combo_item(&walker, &combo, &name) } {
                    log(&format!("RESULT: select_combo_item(muhenkan) failed: {e}"));
                }
            } else {
                log("RESULT: muhenkan combo not found, skipping --set-muhenkan.");
            }
        }

        if let Some(name) = set_henkan {
            if let Some(combo) = find_id("KeyAssignment_Henkan_MSIME_ComboBox") {
                log(&format!("--set-henkan={name:?}: selecting..."));
                // SAFETY: walker/combo は直前に取得した有効な COM オブジェクト。
                if let Err(e) = unsafe { select_combo_item(&walker, &combo, &name) } {
                    log(&format!("RESULT: select_combo_item(henkan) failed: {e}"));
                }
            } else {
                log("RESULT: henkan combo not found, skipping --set-henkan.");
            }
        }

        std::thread::sleep(Duration::from_millis(500));
        log_msime_registry_snapshot("after");
        log("msime_key_assignment_settings_probe end");
    }
}

#[cfg(windows)]
fn main() {
    windows_probe::run();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("this probe is windows-only");
}
