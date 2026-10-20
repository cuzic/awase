//! 単調増加のミリ秒時計(ADR-272 段階 A)。
//!
//! Windows は `GetTickCount64`(OS 起動からの ms)。非 Windows は、`tsf/warmup/` のテストが
//! `current_tick_ms().saturating_sub(50)` のように「少し前」の値を作るため、0 に張り付かない
//! よう**プロセス初回呼び出しの 1 時間後を起点にした**単調時計にする(`Instant` ベース)。
#![allow(unsafe_code)] // GetTickCount64 の呼び出し

/// 現在時刻をミリ秒で返す。
#[must_use]
pub fn current_tick_ms() -> u64 {
    #[cfg(windows)]
    {
        // SAFETY: GetTickCount64 はどのスレッドからも安全に呼び出せるスレッドセーフな Win32 API。
        //         引数なし・副作用なし・内部ロックにより安全性が保証される。
        unsafe { windows::Win32::System::SystemInformation::GetTickCount64() }
    }
    #[cfg(not(windows))]
    {
        use std::sync::OnceLock;
        use std::time::Instant;
        static START: OnceLock<Instant> = OnceLock::new();
        const OFFSET_MS: u64 = 3_600_000;
        let start = START.get_or_init(Instant::now);
        OFFSET_MS + u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX - OFFSET_MS)
    }
}
