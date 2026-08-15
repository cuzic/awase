//! OS レベルの低レベルキーボードフックによる、egui のキーイベントでは
//! 検出できないキー（無変換/変換/かな/カタカナ/ひらがな/英数、F13-F24）の
//! キャプチャ機構。
//!
//! # なぜ egui の Key イベントでは不十分か
//!
//! `main.rs::egui_key_to_internal`（ショートカット再割当タブのキャプチャで
//! 使用）は winit/egui の `Key` enum を経由するが、IME 専用の仮想キー
//! （無変換/変換/かな/カタカナ/ひらがな/英数）や F13-F24 に対応する `Key`
//! 変種が存在せず検出できない（`egui_key_to_internal` の doc コメント参照）。
//! これらはまさに「親指シフト ON/OFF」「awase → IME ON/OFFキー」の既定値
//! （例: `Ctrl+変換`）で使われるキーそのものであり、キャプチャ機能から
//! 除外すると最も需要のあるユースケースを取りこぼす
//! （2026-08-15 ユーザー判断: OS レベルのキーフックで解決する）。
//!
//! # 設計
//!
//! `crates/awase-windows/src/hook.rs::install_hook` と同じ「専用スレッド +
//! `WH_KEYBOARD_LL` + 軽量メッセージポンプ + Drop で `WM_QUIT` 送信して
//! 解除」パターンを踏襲するが、あちらの本番フックとは以下の点で異なる
//! （awase.exe 常駐フックの複雑さ——自己注入判定・BUG-08/BUG-14 対策・
//! stuck modifier 対策等——は awase-settings の一時的なキャプチャには
//! 一切不要なため持ち込まない）:
//!
//! - **観測専用**。`CallNextHookEx` へ必ずそのまま通し、キーを一切
//!   握りつぶさない（`ncode`/イベント種別を問わず、他アプリ・awase.exe
//!   本体への配送を妨げない）。
//! - 対象は固定の安全な VK 集合のみ（[`ALLOWED_VK`]、
//!   `THUMB_KEY_OPTIONS`/`IME_MODE_KEY_OPTIONS` の main key 候補と一致）。
//!   それ以外の VK は無視する（読み捨てるだけで、他の処理には一切影響しない）。
//! - **自プロセスのウィンドウが前面（フォアグラウンド）にある間だけ**
//!   採用する（`GetForegroundWindow`/`GetWindowThreadProcessId` で自
//!   プロセスIDと比較）。他アプリ操作中や Alt-Tab で離れている間の
//!   押下を誤ってキャプチャしないための安全策。
//! - 修飾キー判定は `GetAsyncKeyState`（`VK_CONTROL`/`VK_SHIFT`/`VK_MENU`、
//!   左右区別なし）。`format_combo` が左右非区別の "Ctrl"/"Shift"/"Alt"
//!   のみを扱うため、これで十分。
//! - Esc によるキャンセルはこのモジュールでは扱わない。呼び出し側
//!   （`main.rs::process_combo_capture`）が既存の `process_keymap_capture`
//!   と同じく egui 側の Esc 検出（ウィンドウフォーカス前提）に任せる。

#[cfg(windows)]
mod windows_impl {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU32, Ordering};

    use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VK_CONTROL, VK_MENU, VK_SHIFT,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, DispatchMessageW, GetForegroundWindow, GetMessageW,
        GetWindowThreadProcessId, KBDLLHOOKSTRUCT, MSG, PostThreadMessageW, SetWindowsHookExW,
        UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_KEYDOWN, WM_QUIT, WM_SYSKEYDOWN,
    };

    /// 対象として認識する VK（`THUMB_KEY_OPTIONS`/`IME_MODE_KEY_OPTIONS` の
    /// main key 候補と一致させること。新しい候補を追加したらここにも追加する）。
    const ALLOWED_VK: &[(u32, &str)] = &[
        (0x20, "VK_SPACE"),
        (0x0D, "VK_RETURN"),
        (0x1C, "VK_CONVERT"),
        (0x1D, "VK_NONCONVERT"),
        (0x15, "VK_KANA"),
        (0xF1, "VK_DBE_KATAKANA"),
        (0xF2, "VK_DBE_HIRAGANA"),
        (0xF0, "VK_DBE_ALPHANUMERIC"),
        (0x7C, "VK_F13"),
        (0x7D, "VK_F14"),
        (0x7E, "VK_F15"),
        (0x7F, "VK_F16"),
        (0x80, "VK_F17"),
        (0x81, "VK_F18"),
        (0x82, "VK_F19"),
        (0x83, "VK_F20"),
        (0x84, "VK_F21"),
        (0x85, "VK_F22"),
        (0x86, "VK_F23"),
        (0x87, "VK_F24"),
    ];

    fn vk_to_internal(vk: u32) -> Option<&'static str> {
        ALLOWED_VK
            .iter()
            .find(|(code, _)| *code == vk)
            .map(|(_, name)| *name)
    }

    /// 押下時点で修飾キーが押されているか（左右区別なし）。
    fn key_held(vk: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY) -> bool {
        // SAFETY: GetAsyncKeyState は任意のスレッド・任意のタイミングで
        // 呼んでよい単純な状態照会 API（ドキュメント上の制約なし）。
        let state = unsafe { GetAsyncKeyState(i32::from(vk.0)) };
        (state.cast_unsigned() & 0x8000) != 0
    }

    /// キャプチャ結果（cancel は呼び出し側が egui の Esc 検出で扱うため、
    /// このモジュールが返すのは「対象キーを検出した」場合のみ）。
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct CapturedKey {
        pub ctrl: bool,
        pub shift: bool,
        pub alt: bool,
        pub internal: &'static str,
    }

    static CAPTURED: Mutex<Option<CapturedKey>> = Mutex::new(None);

    /// `start()` がフックスレッドから TID を受け取るためのハンドシェイク用
    /// スロット（`hook.rs::hook_tid_*` と同じパターン）。
    /// 0 = 待機中、`u32::MAX` = `SetWindowsHookExW` 失敗、それ以外 = TID。
    static HOOK_TID_SLOT: AtomicU32 = AtomicU32::new(0);

    /// 自プロセスの PID。フックコールバックがフォアグラウンドウィンドウの
    /// 所有プロセスと比較するために使う（他アプリ操作中の誤キャプチャ防止）。
    fn current_process_id() -> u32 {
        std::process::id()
    }

    /// フォアグラウンドウィンドウが自プロセスのものかどうか。
    fn is_own_window_foreground() -> bool {
        // SAFETY: 引数を取らない単純な状態照会 API。
        let hwnd = unsafe { GetForegroundWindow() };
        if hwnd.0.is_null() {
            return false;
        }
        let mut pid = 0u32;
        // SAFETY: pid は有効なスタック上の u32、hwnd は上で取得した値。
        unsafe {
            GetWindowThreadProcessId(hwnd, Some(&raw mut pid));
        }
        pid == current_process_id()
    }

    unsafe extern "system" fn hook_callback(ncode: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if ncode < 0 {
            // SAFETY: ncode < 0 の場合、フック契約によりそのまま次へ渡す
            // 以外の処理をしてはならない。hhk は無視される（MSDN）ため None。
            return unsafe { CallNextHookEx(None, ncode, wparam, lparam) };
        }
        // wparam はフック契約上メッセージ ID（WM_KEYDOWN 等の小さな定数）を
        // 保持するとドキュメントされており、u32 へのキャストで情報を失わない。
        #[expect(clippy::cast_possible_truncation)]
        let is_keydown = matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
        if is_keydown {
            // SAFETY: lparam はフック契約により有効な KBDLLHOOKSTRUCT を指す。
            let kb = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
            if let Some(internal) = vk_to_internal(kb.vkCode)
                && is_own_window_foreground()
            {
                let captured = CapturedKey {
                    ctrl: key_held(VK_CONTROL),
                    shift: key_held(VK_SHIFT),
                    alt: key_held(VK_MENU),
                    internal,
                };
                if let Ok(mut slot) = CAPTURED.lock() {
                    *slot = Some(captured);
                }
            }
        }
        // 観測専用: 対象キーであっても握りつぶさず必ず通す。
        // SAFETY: hhk は無視される（MSDN）ため None。
        unsafe { CallNextHookEx(None, ncode, wparam, lparam) }
    }

    /// キャプチャセッションの RAII ガード。ドロップ時にフックスレッドへ
    /// `WM_QUIT` を送信し、スレッド終了（`UnhookWindowsHookEx` 込み）を
    /// 待機する（`hook.rs::HookGuard` と同じパターン）。
    pub struct CaptureGuard {
        hook_thread_id: u32,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl Drop for CaptureGuard {
        fn drop(&mut self) {
            // SAFETY: hook_thread_id はフックスレッドの有効な TID。
            unsafe {
                let _ = PostThreadMessageW(self.hook_thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
            }
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    /// キャプチャを開始する。失敗時（スレッド起動失敗・`SetWindowsHookExW`
    /// 失敗）は `None`。
    #[must_use]
    pub fn start() -> Option<CaptureGuard> {
        HOOK_TID_SLOT.store(0, Ordering::SeqCst);
        {
            let mut slot = CAPTURED.lock().ok()?;
            *slot = None;
        }

        let thread = std::thread::Builder::new()
            .name("awase-settings-key-capture".into())
            .spawn(|| {
                // SAFETY: hook_callback は正しいシグネチャの extern "system" fn。
                let hook_result =
                    unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_callback), None, 0) };
                match hook_result {
                    Ok(hook) => {
                        let tid =
                            unsafe { windows::Win32::System::Threading::GetCurrentThreadId() };
                        HOOK_TID_SLOT.store(tid, Ordering::Release);

                        let mut msg = MSG::default();
                        loop {
                            // SAFETY: msg は有効なスタック上の MSG。
                            let ret = unsafe { GetMessageW(&raw mut msg, None, 0, 0) };
                            if ret.0 <= 0 {
                                break;
                            }
                            // SAFETY: msg は GetMessageW が充填した有効な値。
                            unsafe {
                                DispatchMessageW(&raw const msg);
                            }
                        }
                        // SAFETY: hook はこのスレッドが SetWindowsHookExW で得た値。
                        unsafe {
                            let _ = UnhookWindowsHookEx(hook);
                        }
                    }
                    Err(e) => {
                        log::error!("[key-capture] SetWindowsHookExW failed: {e}");
                        HOOK_TID_SLOT.store(u32::MAX, Ordering::Release);
                    }
                }
            })
            .ok()?;

        let hook_tid = loop {
            let t = HOOK_TID_SLOT.load(Ordering::Acquire);
            if t != 0 {
                break t;
            }
            std::hint::spin_loop();
        };

        if hook_tid == u32::MAX {
            let _ = thread.join();
            return None;
        }

        Some(CaptureGuard {
            hook_thread_id: hook_tid,
            thread: Some(thread),
        })
    }

    /// キャプチャされたキー（あれば）を取り出す（1ショット）。
    #[must_use]
    pub fn take_captured() -> Option<CapturedKey> {
        CAPTURED.lock().ok()?.take()
    }

    #[cfg(test)]
    mod tests {
        use super::ALLOWED_VK;
        use std::collections::BTreeSet;

        /// `ALLOWED_VK` は `combo_key_list_ui` の main key ドロップダウン
        /// （`THUMB_KEY_OPTIONS ∪ IME_MODE_KEY_OPTIONS`）と過不足なく一致
        /// させること。新しい候補をどちらかに追加してもう片方を更新し忘れると、
        /// - `ALLOWED_VK` に無い候補: ドロップダウンでは選べるのにキャプチャ
        ///   ボタンでは検出できない（片手落ちの機能）。
        /// - `ALLOWED_VK` にだけある候補: ドロップダウンに存在しない内部表記を
        ///   キャプチャが書き込んでしまい、表示名が解決できず「(未選択)」
        ///   のような表示崩れを招きうる。
        ///   のどちらかを静かに生む。この回帰を機械的に検知する。
        #[test]
        fn allowed_vk_matches_combo_key_list_ui_main_key_candidates() {
            let allowed: BTreeSet<&str> = ALLOWED_VK.iter().map(|(_, name)| *name).collect();
            let dropdown: BTreeSet<&str> = crate::THUMB_KEY_OPTIONS
                .iter()
                .chain(crate::IME_MODE_KEY_OPTIONS)
                .map(|(_, internal)| *internal)
                .collect();
            assert_eq!(
                allowed, dropdown,
                "ALLOWED_VK と THUMB_KEY_OPTIONS∪IME_MODE_KEY_OPTIONS の内部表記が\n\
                 一致しません。候補を追加/削除したら両方を更新してください。"
            );
        }

        /// `ALLOWED_VK` に重複エントリが無いこと（コピペミス防止）。
        #[test]
        fn allowed_vk_has_no_duplicate_vk_codes() {
            let mut codes: Vec<u32> = ALLOWED_VK.iter().map(|(code, _)| *code).collect();
            let before = codes.len();
            codes.sort_unstable();
            codes.dedup();
            assert_eq!(
                codes.len(),
                before,
                "ALLOWED_VK に重複した VK コードがあります"
            );
        }
    }
}

#[cfg(windows)]
pub use windows_impl::{CaptureGuard, CapturedKey, start, take_captured};

#[cfg(not(windows))]
mod stub {
    /// 非 Windows 環境向けスタブ（`awase-settings` は Linux でもビルド・
    /// テストされるため）。常に無効。
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct CapturedKey {
        pub ctrl: bool,
        pub shift: bool,
        pub alt: bool,
        pub internal: &'static str,
    }

    #[derive(Debug)]
    pub struct CaptureGuard;

    #[must_use]
    pub fn start() -> Option<CaptureGuard> {
        None
    }

    #[must_use]
    pub fn take_captured() -> Option<CapturedKey> {
        None
    }
}

#[cfg(not(windows))]
pub use stub::{CaptureGuard, CapturedKey, start, take_captured};
