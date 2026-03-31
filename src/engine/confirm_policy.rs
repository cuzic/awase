//! 確定モード（ConfirmPolicy）の実装。
//! NicolaFsm の idle_* メソッド群。
//!
//! Idle 状態で文字キーまたは親指キーが到着したとき、
//! `ConfirmMode` に応じて保留・投機出力・即時確定を選択する。

use crate::config::ConfirmMode;

use super::fsm_types::{
    ClassifiedEvent, Face, ParseAction, PendingKey, PendingThumbData, TimerIntent,
};
use super::nicola_fsm::{record_output, NicolaFsm, CONTINUOUS_KEYSTROKE_THRESHOLD_US};

impl NicolaFsm {
    /// 確定モードに応じた保留処理へディスパッチ
    pub(crate) fn dispatch_confirm_mode(&mut self, ev: &ClassifiedEvent) -> ParseAction {
        match self.confirm_mode {
            ConfirmMode::Wait => self.idle_wait(ev),
            ConfirmMode::Speculative => self.idle_speculative(ev),
            ConfirmMode::TwoPhase => self.idle_two_phase(ev),
            ConfirmMode::AdaptiveTiming => {
                let is_continuous = self
                    .last_key_gap_us
                    .is_some_and(|gap| gap < CONTINUOUS_KEYSTROKE_THRESHOLD_US);
                if is_continuous {
                    self.idle_wait(ev)
                } else {
                    self.idle_two_phase(ev)
                }
            }
            ConfirmMode::NgramPredictive => self.idle_ngram(ev),
        }
    }

    /// Idle + Wait モード: 新規キーを保留状態に遷移させタイマーを起動する
    pub(crate) const fn idle_wait(&mut self, ev: &ClassifiedEvent) -> ParseAction {
        if ev.key_class.is_thumb() {
            self.enter_pending_thumb(PendingThumbData {
                scan_code: ev.scan_code,
                vk_code: ev.vk_code,
                is_left: ev.key_class.is_left_thumb(),
                timestamp: ev.timestamp,
            });
        } else {
            self.enter_pending_char(PendingKey {
                scan_code: ev.scan_code,
                vk_code: ev.vk_code,
                timestamp: ev.timestamp,
            });
        }
        ParseAction::Shift {
            timer: TimerIntent::Pending,
        }
    }

    /// Idle + Speculative モード: 文字キーは即時出力して SpeculativeChar へ遷移
    pub(crate) fn idle_speculative(&mut self, ev: &ClassifiedEvent) -> ParseAction {
        if ev.key_class.is_thumb() {
            // Thumb key → same as Wait mode (pending thumb)
            return self.idle_wait(ev);
        }

        // Character key → immediately output normal face, enter SpeculativeChar
        let face = Face::Normal;
        if let Some((action, kana)) =
            self.lookup_face(ev.scan_code, ev.vk_code, self.get_face(face))
        {
            self.enter_speculative_char(PendingKey {
                scan_code: ev.scan_code,
                vk_code: ev.vk_code,
                timestamp: ev.timestamp,
            });
            // Output immediately + set timer for the threshold window
            ParseAction::Reduce {
                actions: vec![action.clone()],
                record: record_output(ev.scan_code, &action, kana),
                timer: TimerIntent::Pending,
            }
        } else {
            ParseAction::PassThrough {
                timer: TimerIntent::Keep,
            }
        }
    }

    /// Idle + TwoPhase モード: Phase 1 は短い待機、Phase 2 は投機出力
    ///
    /// 親指キーは Wait モードと同じ扱い。
    /// 文字キーは短い待機（speculative_delay_us）の後、投機出力に遷移する。
    pub(crate) const fn idle_two_phase(&mut self, ev: &ClassifiedEvent) -> ParseAction {
        if ev.key_class.is_thumb() {
            // Thumb keys use Wait mode (same as Speculative)
            return self.idle_wait(ev);
        }

        // Phase 1: Short wait (speculative_delay_us)
        // Same as Wait mode but with shorter timer
        self.enter_pending_char(PendingKey {
            scan_code: ev.scan_code,
            vk_code: ev.vk_code,
            timestamp: ev.timestamp,
        });

        // Use TIMER_SPECULATIVE with the short delay
        ParseAction::Shift {
            timer: TimerIntent::SpeculativeWait,
        }
    }

    /// Idle + NgramPredictive モード: n-gram スコアで投機/待機を動的切替
    ///
    /// 親指キーは Wait モードと同じ扱い。
    /// 文字キーは通常面と親指面の n-gram スコアを比較し、
    /// 通常面が明らかに有利なら Speculative、そうでなければ Wait。
    pub(crate) fn idle_ngram(&mut self, ev: &ClassifiedEvent) -> ParseAction {
        if ev.key_class.is_thumb() {
            return self.idle_wait(ev);
        }

        // If no n-gram model, fall back to TwoPhase
        if self.ngram_model.is_none() {
            return self.idle_two_phase(ev);
        }

        // Get candidate kana for each face
        let normal_kana = self
            .lookup_face(ev.scan_code, ev.vk_code, self.get_face(Face::Normal))
            .and_then(|(_, kana)| kana);
        let left_kana = self
            .lookup_face(ev.scan_code, ev.vk_code, self.get_face(Face::LeftThumb))
            .and_then(|(_, kana)| kana);
        let right_kana = self
            .lookup_face(ev.scan_code, ev.vk_code, self.get_face(Face::RightThumb))
            .and_then(|(_, kana)| kana);

        // Decision: if normal is clearly more likely, output speculatively
        let judge = self.timing_judge();
        if judge.should_speculate(normal_kana, left_kana, right_kana) {
            self.idle_speculative(ev)
        } else {
            self.idle_wait(ev)
        }
    }
}
