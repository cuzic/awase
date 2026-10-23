//! ADR-279 案 P の段階 P0(観測だけ)の殻: 明示 OFF の意図が残っている間も、直接観測の窓(`Imm32Unavailable` × GJI/
//! 同定済みの MS-IME 本体、TSF ネイティブを除く)で IMM の読みを専用のタイマーで取り、「`ime_on=Some(true)`(日本語入力)が
//! 書き込みの柵のあとも 3000ms 続いた」= 追随すべきだった、を journal(`JournalEntry::DriftFollowWould`)と awase.log
//! (`[drift-follow-would]`)に残す。判定は核 `state::drift_follow_would::step`。
//!
//! **観測専用**(挙動を変えない):
//! - IME へは何も書かない。belief・観測ストア・Engine・意図(`IntentStore`)・drift correction に何も入れない。
//! - 読みは refresh のパイプライン(`ir_execute`)を**通らない**。refresh を再開すると、`Imm32Unavailable` の窓では GJI の
//!   I/O 推論(`ir_stage_observe` の `Blacklist` 分岐の `write_observer_poll`)が `ObserverPoll(true)` を書き、drift correction
//!   (`evaluate_drift` は `ObserverPoll` を根拠にする)が明示 OFF に向けて IME を書き直す(設計メモ X1)。それを避けるため、
//!   `TIMER_IME_REFRESH` を使わず専用のタイマー(`TIMER_DRIFT_FOLLOW_WOULD`)で読みだけを取る。
//! - この殻が `platform_state` に触れるのは、明示意図の読み取り(`explicit_intent()`)・無効アプリの判定
//!   (`focus.app_disabled`)・記録(`journal.record(`)だけ(`tests/architecture_guard.rs` が固定)。
//!
//! 副作用として残るもの: IMM の読み(`read_ime_state_full_async`、ワーカーで 300ms 打ち切り)が約
//! `ime_poll_interval_ms`(既定 500ms)ごとに走る。明示意図が無いときの通常のポーリングと同じ読み・同じ間隔で、
//! 読みの所要 ms が `send_health` に 1 サンプル足される(`read_ime_state_full` の既存の挙動、ADR-258 段階 0 と同じ)。

use crate::state::drift_follow_would::{
    applies, classify_read, step, Event, ReadFacts, Run, RunClass, RunEnd,
};
use crate::state::probe_admission::FocusFence;

/// P0 の観測の状態。
#[derive(Default)]
pub(super) struct DriftFollowWould {
    /// 走っている開の連続。
    run: Option<Run<FocusFence>>,
}

impl super::Runtime {
    /// P0 の対象の窓か(`state::drift_follow_would::applies`)。
    fn drift_follow_would_applies(&self) -> bool {
        applies(
            self.platform_state.ime.explicit_intent(),
            self.direct_mode_key_watch_kind().is_some(),
            self.platform
                .current_app_profile()
                .is_effectively_tsf_native(self.platform.focus.class_name()),
            self.platform_state.focus.app_disabled,
        )
    }

    /// refresh が明示意図で止まるところ(`reschedule_ime_refresh`)と、専用のタイマーの満了から呼ぶ。対象の窓なら
    /// 専用のタイマーを張る(既に張ってあれば触らない)。対象外なら何もしない(連続は次の満了で捨てる)。
    pub(super) fn drift_follow_would_ensure_timer(&mut self) {
        if self
            .platform
            .timer
            .is_active(crate::TIMER_DRIFT_FOLLOW_WOULD)
            || !self.drift_follow_would_applies()
        {
            return;
        }
        self.platform.timer.set(
            crate::TIMER_DRIFT_FOLLOW_WOULD,
            std::time::Duration::from_millis(u64::from(
                self.platform_state.focus.ime_poll_interval_ms,
            )),
        );
    }

    /// 専用のタイマー(`TIMER_DRIFT_FOLLOW_WOULD`)の満了。ハンドラ冒頭で kill 済み。対象の窓なら、次のタイマーを先に
    /// 張ってから IMM を 1 回読む(読みの完了を待たずに張る: `with_app` に入れず読みを捨てた回でも、観測の連鎖が切れない)。
    pub(super) fn drift_follow_would_on_timer(&mut self) {
        if !self.drift_follow_would_applies() {
            // 対象外になった(意図が外れた・窓が変わった)。連続があれば終わりを記録して捨てる。
            if let Some(run) = self.drift_follow_would.run.take() {
                let end = RunEnd {
                    class: run.class,
                    elapsed_ms: run.elapsed_ms(),
                    crossed: run.crossed,
                    reason: if self.platform_state.ime.explicit_intent() == Some(false) {
                        crate::state::drift_follow_would::EndReason::FocusMoved
                    } else {
                        crate::state::drift_follow_would::EndReason::IntentGone
                    },
                };
                self.drift_follow_would_record_end(end, None);
            }
            return;
        }
        self.drift_follow_would_ensure_timer();
        let focus_at_start = self.focus_fence();
        let read_started_ms = crate::hook::current_tick_ms();
        win32_async::spawn_local(async move {
            let snap = crate::ime::read_ime_state_full_async().await;
            let _ = crate::with_app(|app| {
                app.drift_follow_would_on_read(focus_at_start, read_started_ms, &snap);
            });
        });
    }

    /// 読みの完了。核で判定し、節目だけを記録する。
    fn drift_follow_would_on_read(
        &mut self,
        focus_at_start: FocusFence,
        read_started_ms: u64,
        snap: &crate::ime::ImeSnapshot,
    ) {
        let read = classify_read(snap.ime_on, snap.conversion_mode);
        let facts = ReadFacts {
            explicit_off: self.drift_follow_would_applies(),
            focus_at_start,
            focus_now: self.focus_fence(),
            read,
            read_started_ms,
            last_operation_ms: crate::hook::last_operation_tick_ms(),
        };
        let (next, outcome) = step(
            self.drift_follow_would.run,
            &facts,
            crate::tuning::KEY_EFFECT_SETTLE_MS,
            crate::tuning::DRIFT_CORRECTION_BLIND_REARM_COOLDOWN_MS,
        );
        self.drift_follow_would.run = next;
        tracing::debug!(
            "[drift-follow-would] read={} conv={:?} event={:?}",
            read.label(),
            snap.conversion_mode.map(|v| format!("0x{v:X}")),
            outcome.event,
        );
        if let Some(end) = outcome.ended {
            self.drift_follow_would_record_end(end, snap.conversion_mode);
        }
        match outcome.event {
            Event::Started(class) => {
                self.drift_follow_would_record("start", class, 0, snap.conversion_mode);
            }
            Event::Crossed { class, elapsed_ms } => {
                let event = match class {
                    RunClass::Native => "would-follow",
                    RunClass::Alnum => "alnum-open",
                };
                self.drift_follow_would_record(event, class, elapsed_ms, snap.conversion_mode);
            }
            Event::Idle | Event::Held | Event::NotOpen | Event::Continuing { .. } => {}
        }
    }

    fn drift_follow_would_record(
        &mut self,
        event: &'static str,
        class: RunClass,
        elapsed_ms: u64,
        conv: Option<u32>,
    ) {
        let record = self.drift_follow_would_new_record(event, class, elapsed_ms, conv);
        tracing::info!(
            "[drift-follow-would] {event} class={} elapsed={elapsed_ms}ms conv={:?} ime={} \
             (観測だけ。awase は IME・belief・意図を変えない)",
            class.label(),
            conv.map(|v| format!("0x{v:X}")),
            record.ime,
        );
        self.platform_state
            .ime
            .journal
            .record(crate::journal::JournalEntry::DriftFollowWould { record });
    }

    fn drift_follow_would_record_end(&mut self, end: RunEnd, conv: Option<u32>) {
        let mut record = self.drift_follow_would_new_record("end", end.class, end.elapsed_ms, conv);
        record.crossed = Some(end.crossed);
        record.end_reason = Some(end.reason.label());
        tracing::info!(
            "[drift-follow-would] end class={} elapsed={}ms crossed={} reason={}",
            end.class.label(),
            end.elapsed_ms,
            end.crossed,
            end.reason.label(),
        );
        self.platform_state
            .ime
            .journal
            .record(crate::journal::JournalEntry::DriftFollowWould { record });
    }

    fn drift_follow_would_new_record(
        &self,
        event: &'static str,
        class: RunClass,
        elapsed_ms: u64,
        conv: Option<u32>,
    ) -> crate::journal::DriftFollowWouldRecord {
        let ime = match self.direct_mode_key_watch_kind() {
            Some(crate::state::ime_kind::ImeKindId::Gji) => "gji",
            Some(crate::state::ime_kind::ImeKindId::MsIme) => "ms-ime",
            None => "none",
        };
        crate::journal::DriftFollowWouldRecord {
            event,
            class: class.label(),
            elapsed_ms,
            crossed: None,
            end_reason: None,
            conv,
            ime,
            profile: super::lang_detect_shadow::profile_label(self.platform.current_app_profile()),
        }
    }
}
