#![allow(unsafe_code)]
//! 学習窓への「自分以外からの注入」を直接観測する専用`WH_KEYBOARD_LL`フック
//! （ADR-196決定1b項目1・項目4）。
//!
//! awase本体（`crates/awase-windows/src/hook.rs`）と同じ「専用スレッドにフックを
//! 登録し、そのスレッドの軽量メッセージポンプで維持する」パターンを踏襲する
//! （フックはフックを登録したスレッドがメッセージを回し続けないと黙って外れる、
//! round4 M-B）。

use std::sync::atomic::{AtomicU32, Ordering};
use std::thread::JoinHandle;

use awase_keymap_learn::external_write::{classify_injection, InjectionOrigin, LivenessCounter};
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, PostThreadMessageW, SetWindowsHookExW,
    UnhookWindowsHookEx, KBDLLHOOKSTRUCT, MSG, WH_KEYBOARD_LL, WM_QUIT,
};

/// 学習プロセス自身の`SendInput`に付ける目印。awase本体の`INJECTED_MARKER`
/// （"KEYM"）・`TSF_MARKER`（"KEYF"）・`IME_KANJI_MARKER`（"KEYJ"）のいずれとも
/// 異なる値にする（"LRNM"、学習プロセス専用）。
pub const SELF_MARKER: usize = 0x4C52_4E4D;

const LLKHF_INJECTED: u32 = 0x10;

static SELF_OBSERVED: AtomicU32 = AtomicU32::new(0);
static EXTERNAL_COUNT: AtomicU32 = AtomicU32::new(0);
static PHYSICAL_COUNT: AtomicU32 = AtomicU32::new(0);

/// フックスレッドの起動状態。0=待機中、`u32::MAX`=`SetWindowsHookExW`失敗、
/// それ以外=フックスレッドのTID（`awase-windows/src/hook.rs`と同型のパターン）。
static HOOK_TID: AtomicU32 = AtomicU32::new(0);

unsafe extern "system" fn hook_callback(ncode: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if ncode < 0 {
        return unsafe { CallNextHookEx(None, ncode, wparam, lparam) };
    }
    let kb = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
    let is_injected = (kb.flags.0 & LLKHF_INJECTED) != 0;
    match classify_injection(is_injected, kb.dwExtraInfo, SELF_MARKER) {
        InjectionOrigin::SelfInjected => {
            SELF_OBSERVED.fetch_add(1, Ordering::SeqCst);
        }
        InjectionOrigin::External => {
            EXTERNAL_COUNT.fetch_add(1, Ordering::SeqCst);
        }
        InjectionOrigin::Physical => {
            PHYSICAL_COUNT.fetch_add(1, Ordering::SeqCst);
        }
    }
    // 自分の注入・外部からの注入・物理入力のいずれも、観測するだけで消費しない
    // （学習窓の入力そのものは妨げない）。
    unsafe { CallNextHookEx(None, ncode, wparam, lparam) }
}

/// フックスレッドが起動しきるまでスピン待機する。
fn wait_for_hook_thread() -> windows::core::Result<u32> {
    loop {
        let tid = HOOK_TID.load(Ordering::SeqCst);
        if tid == u32::MAX {
            return Err(windows::core::Error::from_thread());
        }
        if tid != 0 {
            return Ok(tid);
        }
        std::hint::spin_loop();
    }
}

/// 専用スレッドで`WH_KEYBOARD_LL`フックを保持するガード。ドロップ時に
/// フックスレッドを終了させる。
#[derive(Debug)]
pub struct HookMonitor {
    hook_thread_id: u32,
    thread: Option<JoinHandle<()>>,
    /// このモニタが生きている間に学習プロセス自身が送った注入の数
    /// （呼び出し側が`mark_self_injection_sent`で加算する。観測側
    /// `SELF_OBSERVED`はグローバルなのでプロセス内で複数インスタンスを
    /// 作らないこと——`install`は1回だけ呼ぶ前提）。
    sent: u32,
}

impl HookMonitor {
    /// フックを専用スレッドへ登録する。
    ///
    /// # Errors
    /// スレッドのスポーン失敗、または`SetWindowsHookExW`が失敗した場合。
    pub fn install() -> windows::core::Result<Self> {
        HOOK_TID.store(0, Ordering::SeqCst);
        SELF_OBSERVED.store(0, Ordering::SeqCst);
        EXTERNAL_COUNT.store(0, Ordering::SeqCst);
        PHYSICAL_COUNT.store(0, Ordering::SeqCst);

        let thread = std::thread::Builder::new()
            .name("awase-keymap-learn-hook".into())
            .spawn(|| {
                let hook_result =
                    unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_callback), None, 0) };
                let Ok(hook) = hook_result else {
                    HOOK_TID.store(u32::MAX, Ordering::SeqCst);
                    return;
                };
                let tid = unsafe { windows::Win32::System::Threading::GetCurrentThreadId() };
                HOOK_TID.store(tid, Ordering::SeqCst);

                let mut msg = MSG::default();
                loop {
                    let ret = unsafe { GetMessageW(&raw mut msg, None, 0, 0) };
                    if ret.0 <= 0 {
                        break;
                    }
                    unsafe {
                        DispatchMessageW(&raw const msg);
                    }
                }
                let _ = unsafe { UnhookWindowsHookEx(hook) };
            })
            .map_err(|_| windows::core::Error::from_thread())?;

        let hook_thread_id = match wait_for_hook_thread() {
            Ok(tid) => tid,
            Err(err) => {
                let _ = thread.join();
                return Err(err);
            }
        };

        Ok(Self {
            hook_thread_id,
            thread: Some(thread),
            sent: 0,
        })
    }

    /// 学習プロセスが自分の注入を1件送ったことを記録する
    /// （`SendInput`呼び出しの直前に呼ぶこと）。
    pub const fn mark_self_injection_sent(&mut self) {
        self.sent += 1;
    }

    /// フックの生存確認（決定1b項目4）: 送った数だけ自分のフックで観測できて
    /// いるかを返す。観測漏れが1件でもあればフックが外れた（または
    /// タイムアウトした）とみなす。
    #[must_use]
    pub fn liveness(&self) -> LivenessCounter {
        let mut counter = LivenessCounter::new();
        for _ in 0..self.sent {
            counter.mark_sent();
        }
        let observed = SELF_OBSERVED.load(Ordering::SeqCst).min(self.sent);
        for _ in 0..observed {
            counter.mark_observed();
        }
        counter
    }

    /// これまでに観測した「外部からの書き込み」（注入されているが自分の目印が
    /// 無いキーイベント）の累計件数。
    #[must_use]
    pub fn external_event_count(&self) -> u32 {
        EXTERNAL_COUNT.load(Ordering::SeqCst)
    }

    /// これまでに観測した物理入力（`LLKHF_INJECTED`が無い）の累計件数
    /// （[ADR195-T7](../../../../docs/tasks/adr195-t7-safety-measures.md)の
    /// ユーザー入力混入検出が使う）。
    #[must_use]
    pub fn physical_event_count(&self) -> u32 {
        PHYSICAL_COUNT.load(Ordering::SeqCst)
    }
}

impl Drop for HookMonitor {
    fn drop(&mut self) {
        unsafe {
            let _ = PostThreadMessageW(self.hook_thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// テスト補助: `HHOOK`を直接構築しない（`windows`クレートの`HHOOK`は
/// `isize`のnewtypeで、テストからは触らない——このモジュールの純粋なロジックは
/// `awase_keymap_learn::external_write`側でLinux上でも検証済み）。
#[cfg(test)]
mod tests {
    // このモジュール自体はWin32 APIへの依存が強くLinux上でのユニットテストは
    // 組めない。分類・生存確認の判定ロジックそのものは
    // `awase-keymap-learn::external_write`側でテスト済み（本モジュールは
    // そのロジックにOSイベントを結線するだけ）。
    #[test]
    fn self_marker_differs_from_awase_own_markers() {
        // awase本体のtsf/output.rsが定義する3つの目印のいずれとも重複しないこと
        // を固定する（値がハードコードの重複だと分類が壊れるため）。
        const AWASE_INJECTED_MARKER: usize = 0x4B45_594D;
        const AWASE_TSF_MARKER: usize = 0x4B45_5946;
        const AWASE_IME_KANJI_MARKER: usize = 0x4B45_594A;
        assert_ne!(super::SELF_MARKER, AWASE_INJECTED_MARKER);
        assert_ne!(super::SELF_MARKER, AWASE_TSF_MARKER);
        assert_ne!(super::SELF_MARKER, AWASE_IME_KANJI_MARKER);
    }
}
