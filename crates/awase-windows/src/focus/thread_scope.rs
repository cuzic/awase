//! フォーカス先スレッドが「awase の起動後に作られたか」の判定（ADR-212 P2）。
//!
//! IME の開閉はスレッド単位で保持され、新しいスレッド/プロセスの窓は必ず「閉」で始まる
//! （ADR-191 gji-state-scope-spec §2、CI 実測。実 Chrome でも awase 起動後に起動した
//! Chrome の初期状態は `ka`＝閉、`sc-p2-initial-chrome-*`）。TSF-native では IME の開閉を
//! 読めないので、awase 起動後に作られたスレッドに限り、読めなくても「閉」と決められる。
//! 起動前から存在するスレッドはユーザーが開けた可能性があり、この規則では決められない。

/// フォーカス先スレッドの由来。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadScope {
    /// awase 起動後に作られ、フォーカスで初めて見たスレッド。IME は閉で始まる。
    NewSinceStart,
    /// 起動前から存在したスレッド。開閉は不明。
    PreExisting,
    /// 既に見たスレッド（この規則の対象外。belief は既存の経路に任せる）。
    SeenBefore,
    /// スレッド作成時刻を取得できなかった。
    Unknown,
}

/// 純粋判定。`created_after_awase_ms` はスレッド作成時刻 − awase 起動時刻（ms、負=起動前）。
#[must_use]
pub const fn classify_thread_scope(
    created_after_awase_ms: Option<i64>,
    first_seen: bool,
) -> ThreadScope {
    if !first_seen {
        return ThreadScope::SeenBefore;
    }
    match created_after_awase_ms {
        Some(ms) if ms > 0 => ThreadScope::NewSinceStart,
        Some(_) => ThreadScope::PreExisting,
        None => ThreadScope::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_thread_after_start_is_new() {
        assert_eq!(
            classify_thread_scope(Some(10_200), true),
            ThreadScope::NewSinceStart
        );
    }

    #[test]
    fn thread_created_before_start_is_pre_existing() {
        assert_eq!(
            classify_thread_scope(Some(-187_332), true),
            ThreadScope::PreExisting
        );
        assert_eq!(
            classify_thread_scope(Some(0), true),
            ThreadScope::PreExisting
        );
    }

    #[test]
    fn already_seen_thread_is_never_new() {
        assert_eq!(
            classify_thread_scope(Some(10_200), false),
            ThreadScope::SeenBefore
        );
    }

    #[test]
    fn unreadable_creation_time_is_unknown() {
        assert_eq!(classify_thread_scope(None, true), ThreadScope::Unknown);
    }
}

#[cfg(windows)]
pub use win::{probe_focus_thread, FocusThreadProbe};

#[cfg(windows)]
#[allow(unsafe_code)]
mod win {
    use std::cell::RefCell;
    use std::collections::HashSet;

    use windows::Win32::Foundation::{CloseHandle, FILETIME};
    use windows::Win32::System::Threading::{
        GetCurrentProcess, GetProcessTimes, GetThreadTimes, OpenThread,
        THREAD_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId, GUITHREADINFO,
    };

    use super::{classify_thread_scope, ThreadScope};

    thread_local! {
        static SEEN_TIDS: RefCell<HashSet<u32>> = RefCell::new(HashSet::new());
    }

    /// フォーカス先スレッドの観測結果。
    #[derive(Debug, Clone, Copy)]
    pub struct FocusThreadProbe {
        pub pid: u32,
        pub tid: u32,
        pub created_after_awase_ms: Option<i64>,
        pub scope: ThreadScope,
    }

    fn filetime(f: FILETIME) -> u64 {
        (u64::from(f.dwHighDateTime) << 32) | u64::from(f.dwLowDateTime)
    }

    /// 現在のフォーカス先スレッドを観測し、そのスレッドを「見た」ことを記録する
    /// （同じ tid への 2 回目以降は `SeenBefore`）。メインスレッドから呼ぶこと。
    #[must_use]
    pub fn probe_focus_thread() -> Option<FocusThreadProbe> {
        // SAFETY: 読み取り専用の Win32 呼び出し。out 引数はスタック上のローカルで、
        // OpenThread のハンドルは必ず CloseHandle する。
        unsafe {
            let fg = GetForegroundWindow();
            let fg_tid = GetWindowThreadProcessId(fg, None);
            if fg_tid == 0 {
                return None;
            }
            let mut gti = GUITHREADINFO {
                cbSize: u32::try_from(size_of::<GUITHREADINFO>()).unwrap_or(0),
                ..Default::default()
            };
            let mut target = fg;
            if GetGUIThreadInfo(fg_tid, &raw mut gti).is_ok() && !gti.hwndFocus.0.is_null() {
                target = gti.hwndFocus;
            }
            let mut pid = 0_u32;
            let tid = GetWindowThreadProcessId(target, Some(&raw mut pid));
            if tid == 0 {
                return None;
            }
            let (mut c, mut e, mut k, mut u) = (
                FILETIME::default(),
                FILETIME::default(),
                FILETIME::default(),
                FILETIME::default(),
            );
            let awase_start = if GetProcessTimes(
                GetCurrentProcess(),
                &raw mut c,
                &raw mut e,
                &raw mut k,
                &raw mut u,
            )
            .is_ok()
            {
                Some(filetime(c))
            } else {
                None
            };
            let mut created_after_awase_ms = None;
            if let (Some(start), Ok(h)) = (
                awase_start,
                OpenThread(THREAD_QUERY_LIMITED_INFORMATION, false, tid),
            ) {
                if GetThreadTimes(h, &raw mut c, &raw mut e, &raw mut k, &raw mut u).is_ok() {
                    // 100ns 単位の差を ms にする（負=起動前）。
                    let d = i128::from(filetime(c)) - i128::from(start);
                    created_after_awase_ms = i64::try_from(d / 10_000).ok();
                }
                let _ = CloseHandle(h);
            }
            let first_seen = SEEN_TIDS.with(|s| s.borrow_mut().insert(tid));
            Some(FocusThreadProbe {
                pid,
                tid,
                created_after_awase_ms,
                scope: classify_thread_scope(created_after_awase_ms, first_seen),
            })
        }
    }
}
