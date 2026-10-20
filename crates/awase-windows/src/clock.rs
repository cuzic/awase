//! 単調増加のミリ秒時計(ADR-272 段階 A)。
//!
//! Windows は `GetTickCount64`(OS 起動からの ms)。非 Windows は、`tsf/warmup/` のテストが
//! `current_tick_ms().saturating_sub(50)` のように「少し前」の値を作るため、0 に張り付かない
//! よう**プロセス初回呼び出しの 1 時間後を起点にした**単調時計にする(`Instant` ベース)。
//!
//! テストでは [`freeze`] で**仮想時計**に切り替えられる(ADR-272 段階 B0)。実時間の `sleep` で
//! 待つテストは、`GetTickCount64` の分解能(~15.6ms)や CI の負荷でフレークした(BUG-130/131)。
//! 仮想時計は [`advance`] を呼ぶまで進まず、スレッドローカルなので他のテストに影響しない。
#![allow(unsafe_code)] // GetTickCount64 の呼び出し

#[cfg(test)]
thread_local! {
    /// `Some` の間、`current_tick_ms` は実時間ではなくこの値を返す。
    static VIRTUAL_MS: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
fn virtual_ms() -> Option<u64> {
    VIRTUAL_MS.with(std::cell::Cell::get)
}

/// 現在時刻をミリ秒で返す。
#[must_use]
pub fn current_tick_ms() -> u64 {
    #[cfg(test)]
    if let Some(ms) = virtual_ms() {
        return ms;
    }
    real_tick_ms()
}

fn real_tick_ms() -> u64 {
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

/// [`freeze`] が返す。drop で実時間の時計に戻す。
#[cfg(test)]
#[must_use = "drop すると仮想時計が解除される。`let _clock = freeze();` の形で持つこと"]
pub(crate) struct FrozenClock(());

#[cfg(test)]
impl Drop for FrozenClock {
    fn drop(&mut self) {
        VIRTUAL_MS.with(|c| c.set(None));
    }
}

/// このスレッドの時計を、今の実時間の値で止めた仮想時計に切り替える(テスト専用)。
///
/// 以後 [`advance`] を呼ぶまで `current_tick_ms()` は同じ値を返す。戻り値を drop すると実時間に戻る。
#[cfg(test)]
pub(crate) fn freeze() -> FrozenClock {
    assert!(
        virtual_ms().is_none(),
        "clock::freeze: すでに仮想時計になっている(二重に freeze しない)"
    );
    VIRTUAL_MS.with(|c| c.set(Some(real_tick_ms())));
    FrozenClock(())
}

/// 仮想時計を `ms` 進める(テスト専用)。[`freeze`] していなければ panic する。
#[cfg(test)]
pub(crate) fn advance(ms: u64) {
    VIRTUAL_MS.with(|c| {
        let now = c.get().expect("clock::advance: freeze() していない");
        c.set(Some(now + ms));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frozen_clock_stands_still_until_advanced_and_restores_on_drop() {
        let before_real = current_tick_ms();
        {
            let _clock = freeze();
            let t0 = current_tick_ms();
            assert!(t0 >= before_real);
            std::thread::sleep(std::time::Duration::from_millis(20));
            assert_eq!(current_tick_ms(), t0, "advance するまで進まない");
            advance(15);
            assert_eq!(current_tick_ms(), t0 + 15);
        }
        // 解除後は実時間(単調増加)に戻る。
        assert!(current_tick_ms() >= before_real);
        assert!(virtual_ms().is_none());
    }

    #[test]
    #[should_panic(expected = "freeze() していない")]
    fn advance_without_freeze_panics() {
        advance(1);
    }
}
