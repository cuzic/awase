//! OS のかな入力ロック観測に対する通知ヒステリシス。

/// OS から読んだかな入力ロック状態。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KanaLockReading {
    Off,
    On,
    Unknown,
}

/// 観測後に Apply 層が行うべき通知操作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarnAction {
    None,
    Warn,
    ClearWarned,
}

/// 現在伸びている連続観測の向き。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KanaLockStreak {
    None,
    Off(u8),
    On(u8),
}

/// On 連続で Warn に達する閾値。
pub const KANA_LOCK_WARN_STREAK: u8 = 3;

/// Off 連続で ClearWarned に達する閾値。
pub const KANA_LOCK_CLEAR_STREAK: u8 = 2;

/// かな入力ロック検知のヒステリシス。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KanaLockHysteresis {
    streak: u8,
    streak_on: bool,
    warned: bool,
}

impl Default for KanaLockHysteresis {
    fn default() -> Self {
        Self::new()
    }
}

impl KanaLockHysteresis {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            streak: 0,
            streak_on: false,
            warned: false,
        }
    }

    #[must_use]
    pub const fn warned(&self) -> bool {
        self.warned
    }

    #[must_use]
    pub const fn streak(&self) -> KanaLockStreak {
        if self.streak == 0 {
            KanaLockStreak::None
        } else if self.streak_on {
            KanaLockStreak::On(self.streak)
        } else {
            KanaLockStreak::Off(self.streak)
        }
    }

    /// 観測を1つ反映する。
    ///
    /// `Unknown` は連続観測だけを切り、警告済み状態は変更しない。
    pub const fn observe(&mut self, reading: KanaLockReading) -> WarnAction {
        match reading {
            KanaLockReading::Unknown => {
                self.streak = 0;
                WarnAction::None
            }
            KanaLockReading::On => {
                self.advance_streak(true);
                if self.streak >= KANA_LOCK_WARN_STREAK && !self.warned {
                    self.warned = true;
                    WarnAction::Warn
                } else {
                    WarnAction::None
                }
            }
            KanaLockReading::Off => {
                self.advance_streak(false);
                if self.streak >= KANA_LOCK_CLEAR_STREAK && self.warned {
                    self.warned = false;
                    WarnAction::ClearWarned
                } else {
                    WarnAction::None
                }
            }
        }
    }

    const fn advance_streak(&mut self, on: bool) {
        if self.streak == 0 || self.streak_on != on {
            self.streak = 1;
            self.streak_on = on;
        } else {
            self.streak = self.streak.saturating_add(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observe_all(input: &[KanaLockReading]) -> Vec<WarnAction> {
        let mut h = KanaLockHysteresis::new();
        input.iter().map(|&r| h.observe(r)).collect()
    }

    #[test]
    fn alternating_on_off_never_warns_before_threshold() {
        let mut h = KanaLockHysteresis::new();
        for _ in 0..16 {
            assert_eq!(h.observe(KanaLockReading::On), WarnAction::None);
            assert_eq!(h.observe(KanaLockReading::Off), WarnAction::None);
        }
        assert!(!h.warned());
    }

    #[test]
    fn warn_fires_once_on_nth_consecutive_on() {
        assert_eq!(
            observe_all(&[
                KanaLockReading::On,
                KanaLockReading::On,
                KanaLockReading::On,
                KanaLockReading::On,
                KanaLockReading::On,
            ]),
            vec![
                WarnAction::None,
                WarnAction::None,
                WarnAction::Warn,
                WarnAction::None,
                WarnAction::None,
            ]
        );
    }

    #[test]
    fn clear_fires_once_on_mth_consecutive_off_after_warn() {
        assert_eq!(
            observe_all(&[
                KanaLockReading::On,
                KanaLockReading::On,
                KanaLockReading::On,
                KanaLockReading::Off,
                KanaLockReading::Off,
                KanaLockReading::Off,
            ]),
            vec![
                WarnAction::None,
                WarnAction::None,
                WarnAction::Warn,
                WarnAction::None,
                WarnAction::ClearWarned,
                WarnAction::None,
            ]
        );
    }

    #[test]
    fn unknown_resets_streak_but_preserves_warned() {
        let mut h = KanaLockHysteresis::new();
        assert_eq!(h.observe(KanaLockReading::On), WarnAction::None);
        assert_eq!(h.observe(KanaLockReading::On), WarnAction::None);
        assert_eq!(h.observe(KanaLockReading::Unknown), WarnAction::None);
        assert_eq!(h.streak(), KanaLockStreak::None);
        assert_eq!(h.observe(KanaLockReading::On), WarnAction::None);
        assert_eq!(h.observe(KanaLockReading::On), WarnAction::None);
        assert_eq!(h.observe(KanaLockReading::On), WarnAction::Warn);
        assert!(h.warned());
        assert_eq!(h.observe(KanaLockReading::Unknown), WarnAction::None);
        assert!(h.warned());
        assert_eq!(h.observe(KanaLockReading::Off), WarnAction::None);
        assert!(h.warned());
    }

    #[test]
    fn warns_again_after_clear() {
        assert_eq!(
            observe_all(&[
                KanaLockReading::On,
                KanaLockReading::On,
                KanaLockReading::On,
                KanaLockReading::Off,
                KanaLockReading::Off,
                KanaLockReading::On,
                KanaLockReading::On,
                KanaLockReading::On,
            ]),
            vec![
                WarnAction::None,
                WarnAction::None,
                WarnAction::Warn,
                WarnAction::None,
                WarnAction::ClearWarned,
                WarnAction::None,
                WarnAction::None,
                WarnAction::Warn,
            ]
        );
    }

    #[test]
    fn exhaustive_short_sequences_match_reference_model() {
        fn reference(input: &[KanaLockReading]) -> Vec<WarnAction> {
            let mut streak = 0_u8;
            let mut streak_on = false;
            let mut warned = false;
            let mut out = Vec::new();
            for &reading in input {
                let action = match reading {
                    KanaLockReading::Unknown => {
                        streak = 0;
                        WarnAction::None
                    }
                    KanaLockReading::On => {
                        if streak == 0 || !streak_on {
                            streak = 1;
                            streak_on = true;
                        } else {
                            streak += 1;
                        }
                        if streak >= KANA_LOCK_WARN_STREAK && !warned {
                            warned = true;
                            WarnAction::Warn
                        } else {
                            WarnAction::None
                        }
                    }
                    KanaLockReading::Off => {
                        if streak == 0 || streak_on {
                            streak = 1;
                            streak_on = false;
                        } else {
                            streak += 1;
                        }
                        if streak >= KANA_LOCK_CLEAR_STREAK && warned {
                            warned = false;
                            WarnAction::ClearWarned
                        } else {
                            WarnAction::None
                        }
                    }
                };
                out.push(action);
            }
            out
        }

        let alphabet = [
            KanaLockReading::Off,
            KanaLockReading::On,
            KanaLockReading::Unknown,
        ];
        for len in 0..=8 {
            let mut total = 1_usize;
            for _ in 0..len {
                total *= alphabet.len();
            }
            for mut n in 0..total {
                let mut seq = Vec::new();
                for _ in 0..len {
                    seq.push(alphabet[n % alphabet.len()]);
                    n /= alphabet.len();
                }
                assert_eq!(observe_all(&seq), reference(&seq), "seq={seq:?}");
            }
        }
    }
}
