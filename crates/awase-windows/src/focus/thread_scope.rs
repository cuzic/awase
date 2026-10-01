//! フォーカス先スレッドが「awase の起動後に作られたか」の判定（ADR-212 P2）。
//!
//! IME の開閉はスレッド単位で保持され、測定済みの設定・IME では新しいスレッド/
//! プロセスの窓が「閉」で始まる
//! （ADR-191 gji-state-scope-spec §2、CI 実測。実 Chrome でも awase 起動後に起動した
//! Chrome の初期状態は `ka`＝閉、`sc-p2-initial-chrome-*`）。純粋な Imm32Unavailable
//! 窓では IME の開閉を読めないので、測定済み条件かつ awase 起動後に作られた
//! スレッドに限り、読めなくても「閉」と決められる。
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

/// 「新スレッドは閉」を適用するための測定済み条件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClosedAssumption {
    Apply,
    SpiUnavailable,
    ThreadLocalInputSettingsEnabled,
    UnsupportedIme,
}

impl ClosedAssumption {
    #[must_use]
    pub const fn applied(self) -> bool {
        matches!(self, Self::Apply)
    }

    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Self::Apply => "measured-shared-input-settings-and-ime",
            Self::SpiUnavailable => "spi-unavailable",
            Self::ThreadLocalInputSettingsEnabled => "thread-local-input-settings-enabled",
            Self::UnsupportedIme => "unsupported-or-unidentified-ime",
        }
    }
}

/// スレッドの由来と測定済み条件から「閉」を仮定するかとログ理由を決める。
#[must_use]
pub const fn should_assume_closed(
    scope: Option<ThreadScope>,
    assumption: ClosedAssumption,
) -> (bool, &'static str) {
    if matches!(scope, Some(ThreadScope::NewSinceStart)) {
        (assumption.applied(), assumption.reason())
    } else {
        (false, "scope-not-new")
    }
}

/// CI で測定した「入力設定を共有」かつ GJI/MS-IME 本体だけに規則を限定する。
#[must_use]
pub const fn closed_assumption(
    thread_local_input_settings: Option<bool>,
    ime_kind: Option<crate::state::ime_kind::ImeKindId>,
) -> ClosedAssumption {
    match thread_local_input_settings {
        None => ClosedAssumption::SpiUnavailable,
        Some(true) => ClosedAssumption::ThreadLocalInputSettingsEnabled,
        Some(false) if ime_kind.is_some() => ClosedAssumption::Apply,
        Some(false) => ClosedAssumption::UnsupportedIme,
    }
}

/// 純粋判定。`created_after_awase_ms` はスレッド作成時刻 − awase 起動時刻（ms、負=起動前）。
#[must_use]
pub const fn classify_thread_scope(
    created_after_awase_ms: Option<i64>,
    first_seen: bool,
    pid_matches: bool,
) -> ThreadScope {
    if !pid_matches {
        return ThreadScope::Unknown;
    }
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
            classify_thread_scope(Some(10_200), true, true),
            ThreadScope::NewSinceStart
        );
    }

    #[test]
    fn thread_created_before_start_is_pre_existing() {
        assert_eq!(
            classify_thread_scope(Some(-187_332), true, true),
            ThreadScope::PreExisting
        );
        assert_eq!(
            classify_thread_scope(Some(0), true, true),
            ThreadScope::PreExisting
        );
    }

    #[test]
    fn already_seen_thread_is_never_new() {
        assert_eq!(
            classify_thread_scope(Some(10_200), false, true),
            ThreadScope::SeenBefore
        );
    }

    #[test]
    fn unreadable_creation_time_is_unknown() {
        assert_eq!(
            classify_thread_scope(None, true, true),
            ThreadScope::Unknown
        );
    }

    #[test]
    fn mismatched_pid_is_unknown_even_for_a_new_thread() {
        assert_eq!(
            classify_thread_scope(Some(10_200), true, false),
            ThreadScope::Unknown
        );
    }

    #[test]
    fn closed_assumption_is_limited_to_measured_settings_and_imes() {
        use crate::state::ime_kind::ImeKindId;

        assert_eq!(
            closed_assumption(Some(false), Some(ImeKindId::Gji)),
            ClosedAssumption::Apply
        );
        assert_eq!(
            closed_assumption(Some(false), Some(ImeKindId::MsIme)),
            ClosedAssumption::Apply
        );
        assert_eq!(
            closed_assumption(Some(true), Some(ImeKindId::Gji)),
            ClosedAssumption::ThreadLocalInputSettingsEnabled
        );
        assert_eq!(
            closed_assumption(None, Some(ImeKindId::Gji)),
            ClosedAssumption::SpiUnavailable
        );
        assert_eq!(
            closed_assumption(Some(false), None),
            ClosedAssumption::UnsupportedIme
        );
    }

    #[test]
    fn should_assume_closed_fixes_application_and_reason() {
        assert_eq!(
            should_assume_closed(Some(ThreadScope::NewSinceStart), ClosedAssumption::Apply),
            (true, "measured-shared-input-settings-and-ime")
        );
        assert_eq!(
            should_assume_closed(
                Some(ThreadScope::NewSinceStart),
                ClosedAssumption::UnsupportedIme
            ),
            (false, "unsupported-or-unidentified-ime")
        );
        assert_eq!(
            should_assume_closed(
                Some(ThreadScope::NewSinceStart),
                ClosedAssumption::SpiUnavailable
            ),
            (false, "spi-unavailable")
        );
        assert_eq!(
            should_assume_closed(
                Some(ThreadScope::NewSinceStart),
                ClosedAssumption::ThreadLocalInputSettingsEnabled
            ),
            (false, "thread-local-input-settings-enabled")
        );
        assert_eq!(
            should_assume_closed(Some(ThreadScope::SeenBefore), ClosedAssumption::Apply),
            (false, "scope-not-new")
        );
        assert_eq!(
            should_assume_closed(None, ClosedAssumption::Apply),
            (false, "scope-not-new")
        );
    }
}

#[cfg(any(windows, test))]
use std::collections::{HashSet, VecDeque};

#[cfg(any(windows, test))]
const MAX_SEEN_THREADS: usize = 4096;

/// TID は再利用されるため、作成時刻を含めてスレッドの同一性を表す。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg(any(windows, test))]
struct ThreadIdentity {
    tid: u32,
    created_at: u64,
}

/// 初回フォーカス判定用の有界 LRU 履歴。最も長く参照されていない要素から追い出す。
#[cfg(any(windows, test))]
pub(crate) struct SeenThreads {
    order: VecDeque<ThreadIdentity>,
    entries: HashSet<ThreadIdentity>,
    capacity: usize,
}

#[cfg(any(windows, test))]
impl SeenThreads {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            order: VecDeque::with_capacity(capacity),
            entries: HashSet::with_capacity(capacity),
            capacity,
        }
    }

    fn record(&mut self, identity: ThreadIdentity) -> bool {
        if self.entries.contains(&identity) {
            if let Some(position) = self.order.iter().position(|entry| *entry == identity) {
                self.order.remove(position);
            }
            self.order.push_back(identity);
            return false;
        }
        if self.entries.len() >= self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
        self.order.push_back(identity);
        self.entries.insert(identity);
        true
    }
}

#[cfg(any(windows, test))]
impl Default for SeenThreads {
    fn default() -> Self {
        Self::new(MAX_SEEN_THREADS)
    }
}

#[cfg(test)]
mod seen_tests {
    use super::*;

    #[test]
    fn reused_tid_with_a_new_creation_time_is_new() {
        let mut seen = SeenThreads::new(4);
        assert!(seen.record(ThreadIdentity {
            tid: 7,
            created_at: 100
        }));
        assert!(!seen.record(ThreadIdentity {
            tid: 7,
            created_at: 100
        }));
        assert!(seen.record(ThreadIdentity {
            tid: 7,
            created_at: 200
        }));
    }

    #[test]
    fn history_is_bounded_and_evicts_least_recently_used() {
        let mut seen = SeenThreads::new(2);
        let first = ThreadIdentity {
            tid: 1,
            created_at: 10,
        };
        assert!(seen.record(first));
        assert!(seen.record(ThreadIdentity {
            tid: 2,
            created_at: 20
        }));
        assert!(!seen.record(first));
        assert!(seen.record(ThreadIdentity {
            tid: 3,
            created_at: 30
        }));
        assert_eq!(seen.entries.len(), 2);
        assert!(!seen.record(first));
        assert!(seen.record(ThreadIdentity {
            tid: 2,
            created_at: 20
        }));
    }
}

#[cfg(windows)]
pub(crate) use win::{probe_focus_thread, read_thread_local_input_settings};

#[cfg(windows)]
#[allow(unsafe_code)]
mod win {
    use windows::Win32::Foundation::{CloseHandle, FILETIME, HWND};
    use windows::Win32::System::Threading::{
        GetCurrentProcess, GetProcessTimes, GetThreadTimes, OpenThread,
        THREAD_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetGUIThreadInfo, GetWindowThreadProcessId, SystemParametersInfoW, GUITHREADINFO,
        SPI_GETTHREADLOCALINPUTSETTINGS, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    };

    use super::{classify_thread_scope, SeenThreads, ThreadIdentity, ThreadScope};

    /// フォーカス先スレッドの観測結果。
    #[derive(Debug, Clone, Copy)]
    pub(crate) struct FocusThreadProbe {
        pub pid: u32,
        pub tid: u32,
        pub created_after_awase_ms: Option<i64>,
        pub scope: ThreadScope,
    }

    fn filetime(f: FILETIME) -> u64 {
        (u64::from(f.dwHighDateTime) << 32) | u64::from(f.dwLowDateTime)
    }

    /// 分類済み HWND のフォーカス先スレッドを観測し、そのスレッドを「見た」ことを記録する。
    #[must_use]
    pub(crate) fn probe_focus_thread(
        classified_hwnd: HWND,
        classified_pid: u32,
        seen_threads: &mut SeenThreads,
    ) -> Option<FocusThreadProbe> {
        // SAFETY: 読み取り専用の Win32 呼び出し。out 引数はスタック上のローカルで、
        // OpenThread のハンドルは必ず CloseHandle する。
        unsafe {
            let mut classified_window_pid = 0_u32;
            let window_tid =
                GetWindowThreadProcessId(classified_hwnd, Some(&raw mut classified_window_pid));
            if window_tid == 0 {
                return None;
            }
            let mut gti = GUITHREADINFO {
                cbSize: u32::try_from(size_of::<GUITHREADINFO>()).unwrap_or(0),
                ..Default::default()
            };
            let target = if GetGUIThreadInfo(window_tid, &raw mut gti).is_ok()
                && !gti.hwndFocus.0.is_null()
            {
                gti.hwndFocus
            } else {
                classified_hwnd
            };
            let mut pid = 0_u32;
            let tid = GetWindowThreadProcessId(target, Some(&raw mut pid));
            if tid == 0 {
                return None;
            }
            let (mut created, mut exited, mut kernel, mut user) = (
                FILETIME::default(),
                FILETIME::default(),
                FILETIME::default(),
                FILETIME::default(),
            );
            let awase_start = if GetProcessTimes(
                GetCurrentProcess(),
                &raw mut created,
                &raw mut exited,
                &raw mut kernel,
                &raw mut user,
            )
            .is_ok()
            {
                Some(filetime(created))
            } else {
                None
            };
            let mut created_after_awase_ms = None;
            let mut thread_created_at = None;
            if let (Some(start), Ok(h)) = (
                awase_start,
                OpenThread(THREAD_QUERY_LIMITED_INFORMATION, false, tid),
            ) {
                if GetThreadTimes(
                    h,
                    &raw mut created,
                    &raw mut exited,
                    &raw mut kernel,
                    &raw mut user,
                )
                .is_ok()
                {
                    thread_created_at = Some(filetime(created));
                    // 100ns 単位の差を ms にする（負=起動前）。
                    let delta = i128::from(filetime(created)) - i128::from(start);
                    created_after_awase_ms = i64::try_from(delta / 10_000).ok();
                }
                let _ = CloseHandle(h);
            }
            let first_seen = thread_created_at
                .is_none_or(|created_at| seen_threads.record(ThreadIdentity { tid, created_at }));
            let pid_matches = classified_window_pid == classified_pid
                && pid == classified_pid
                && tid == window_tid;
            Some(FocusThreadProbe {
                pid,
                tid,
                created_after_awase_ms,
                scope: classify_thread_scope(created_after_awase_ms, first_seen, pid_matches),
            })
        }
    }

    /// Windows の「入力設定をアプリ ウィンドウごとに異なる値にする」設定を読む。
    /// `None` は API 失敗であり、測定済み条件外として扱う。
    #[must_use]
    pub(crate) fn read_thread_local_input_settings() -> Option<bool> {
        let mut enabled = 0_i32;
        // SAFETY: SPI_GETTHREADLOCALINPUTSETTINGS は BOOL の out 引数へ書き込む。
        unsafe {
            SystemParametersInfoW(
                SPI_GETTHREADLOCALINPUTSETTINGS,
                0,
                Some(std::ptr::from_mut(&mut enabled).cast()),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS::default(),
            )
            .ok()
            .map(|()| enabled != 0)
        }
    }
}
