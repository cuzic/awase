//! ADR-245 追補 2 R2: MS-IME 本体の戻り待ちの復元(`FromResume`)で、戻った窓の最初の打鍵を保持し、R1 の観測が記録
//! されたら(または期限・上限・契機で)順序どおり再生する殻。
//!
//! 判断は `state/half_width_resume_hold.rs`(純粋な状態機械)にあり、ここは事実の収集(前面窓・時刻・打鍵の同一性)と
//! 実行(タイマー・`INPUT_DEFER.replay_later`・完了の札の post)だけ。**IME へは何も書かない・BS を送らない・belief を
//! 更新しない**(保持した打鍵を再生するだけ。再生された打鍵は通常どおり Engine を通る)。
//!
//! 解放の入口(すべて `resume_hold_release` か `resume_hold_on_key` の上限を経由し、前面窓の照合と
//! `TIMER_RESUME_HOLD` の kill は `resume_hold_finish` の 1 か所):
//! - 完了の札(`ResumeDoneToken`)の通知 → `WM_RESUME_HOLD_DONE`(`hold_id` が今の保持と一致するときだけ)
//! - 期限の一発タイマー `TIMER_RESUME_HOLD`
//! - 保持の上限(`resume_hold_on_key` の `OnKey::Release`)
//! - フォーカスの変化・`WM_PANIC_RESET`・設定の再読み込み

use std::time::Duration;

use awase::types::{KeyEventType, RawKeyEvent};

use crate::state::half_width_resume_hold::{KeyFacts, OnKey, ReleaseReason, Released};
use crate::state::probe_admission::ImmLikeTicket;
use crate::state::TickMs;
use crate::Runtime;

/// 復元タスクが持つ「完了の札」。R1 が記録した・記録できずに終わった、のどの経路でも 1 回だけ保持の解放を通知する。
///
/// 通常の通知は消費する `notify(self, reason)`。`Drop` は notify されずに破棄されたときの保険(`Dropped`)で、
/// `post_to_main_thread_with` だけを行い `with_app` を呼ばない・panic しない(executor の終了・panic の巻き戻し・
/// `with_app` の中でのタスクの破棄でも `Drop` は走る)。受け側は `WM_RESUME_HOLD_DONE` のハンドラ
/// (`with_app_or_repost_with`)で、`hold_id` が今の保持と一致するときだけ解放する。
#[derive(Debug)]
pub(crate) struct ResumeDoneToken {
    hold_id: u64,
    sent: bool,
}

impl ResumeDoneToken {
    pub(crate) const fn new(hold_id: u64) -> Self {
        Self {
            hold_id,
            sent: false,
        }
    }

    /// 理由つきで通知し、札を手放す。
    pub(crate) fn notify(mut self, reason: ReleaseReason) {
        self.post(reason);
    }

    fn post(&mut self, reason: ReleaseReason) {
        if self.sent {
            return;
        }
        self.sent = true;
        let hold_id = usize::try_from(self.hold_id).unwrap_or(usize::MAX);
        let _ = crate::win32::post_to_main_thread_with(
            crate::WM_RESUME_HOLD_DONE,
            hold_id,
            reason.code(),
        );
    }
}

impl Drop for ResumeDoneToken {
    fn drop(&mut self) {
        self.post(ReleaseReason::Dropped);
    }
}

/// 札が残っていれば、理由つきで通知して手放す(復元タスクの早期 return などで使う。複雑度を増やさないための 1 行ヘルパー)。
pub(crate) fn notify_done(slot: &mut Option<ResumeDoneToken>, reason: ReleaseReason) {
    if let Some(token) = slot.take() {
        token.notify(reason);
    }
}

/// 打鍵の物理キーとしての同一性(Down と Up で同じ値になること。`vk` では判定しない: `VK_DBE_*` は Down/Up で vk が変わる)。
/// scan が主。左右の Alt/Ctrl は raw の scan が同じで拡張ビットだけが違うが、`RawKeyEvent` には拡張ビットが無いので、
/// vk が左右を区別するキーだけ vk を畳み込む。
pub(super) fn key_facts(event: &RawKeyEvent) -> KeyFacts {
    let side_vk = if [
        crate::vk::VK_LMENU,
        crate::vk::VK_RMENU,
        crate::vk::VK_LCONTROL,
        crate::vk::VK_RCONTROL,
    ]
    .contains(&event.vk_code)
    {
        u64::from(event.vk_code.0)
    } else {
        0
    };
    KeyFacts {
        identity: (side_vk << 32) | u64::from(event.scan_code.0),
        is_down: matches!(event.event_type, KeyEventType::KeyDown),
        is_repeat: event.was_down,
    }
}

impl Runtime {
    /// 最初の打鍵を保持して始める(`kp_stage_half_width_return` の Resume から、IMC 経路かつ R1 を予約できるときだけ)。
    /// 始めたら `true`(呼び出し側は `Consumed` を返す)。既に保持中なら始めず `false`。
    ///
    /// 完了の札を作って `pending_resume_r1` に置く。`kp_restore_kana_from_half_width` が取り出して復元タスクへ渡す。
    pub(super) fn resume_hold_start(&mut self, event: &RawKeyEvent, ticket: ImmLikeTicket) -> bool {
        let scope = crate::win32::foreground_scope();
        let now = TickMs(crate::hook::current_tick_ms());
        let Some(hold_id) = self.resume_hold.start(scope, now, key_facts(event), *event) else {
            return false;
        };
        self.platform.timer.set(
            crate::TIMER_RESUME_HOLD,
            Duration::from_millis(crate::tuning::HALF_WIDTH_RESUME_HOLD_MAX_MS),
        );
        self.pending_resume_r1 = Some((ticket, ResumeDoneToken::new(hold_id)));
        tracing::info!(
            "[half-width-return] 最初の打鍵を保持 (R2) vk=0x{:02X} hold_id={hold_id}",
            event.vk_code.0
        );
        true
    }

    /// `deliver_key_event` の先頭用: origin が ime-off-rescue の再生でなければ、保持中の打鍵を保持に足す。
    /// ime-off-rescue の再生は保持の外で先に処理する(時刻が古く、origin の `skip_rescue_defer` を保つため。Opus r3 M-4')。
    pub(crate) fn resume_hold_on_delivery(
        &mut self,
        event: &RawKeyEvent,
        origin: super::message_handlers::KeyOrigin,
    ) -> bool {
        !matches!(
            origin,
            super::message_handlers::KeyOrigin::ImeOffRescueReplay
        ) && self.resume_hold_on_key(event)
    }

    /// 保持中なら打鍵を保持に足す。`true` = 保持に足した(呼び出し側は `Consumed`)。保持していなければ `false`。
    ///
    /// 取り込み口(`app/mod.rs::handle_hook_key_event`)と `deliver_key_event` の先頭の両方から呼ぶ
    /// (drain の途中で保持が始まっても、同じバッチの後続が追い越さないため。Opus r2 M-2)。
    pub(crate) fn resume_hold_on_key(&mut self, event: &RawKeyEvent) -> bool {
        if !self.resume_hold.is_holding() {
            return false;
        }
        let current = crate::win32::foreground_scope();
        let held_before = self.resume_hold.len();
        let started_at = self.resume_hold.started_at();
        match self.resume_hold.on_key(key_facts(event), *event, current) {
            OnKey::Pass(_) => false,
            OnKey::Held => true,
            OnKey::Release(released) => {
                self.resume_hold_finish(released, ReleaseReason::Cap, held_before + 1, started_at);
                true
            }
        }
    }

    /// 保持を解放する。`hold_id` が `Some` なら今の保持と一致するときだけ(古い札の通知を無視する)。
    pub(crate) fn resume_hold_release(&mut self, hold_id: Option<u64>, reason: ReleaseReason) {
        let current = crate::win32::foreground_scope();
        let held = self.resume_hold.len();
        let started_at = self.resume_hold.started_at();
        let released = self.resume_hold.release(hold_id, current);
        self.resume_hold_finish(released, reason, held, started_at);
    }

    /// 解放の出口(1 か所)。タイマーを kill し、再生する(窓が違えば捨てる)。
    fn resume_hold_finish(
        &mut self,
        released: Released<RawKeyEvent>,
        reason: ReleaseReason,
        held: usize,
        started_at: Option<TickMs>,
    ) {
        let hold_ms = started_at.map_or(0, |s| crate::hook::current_tick_ms().saturating_sub(s.0));
        match released {
            Released::Nothing => {}
            Released::Replay(events) => {
                self.platform.timer.kill(crate::TIMER_RESUME_HOLD);
                tracing::info!(
                    "[half-width-return] 保持の解放: reason={} held={held} hold_ms={hold_ms} → 再生",
                    reason.label()
                );
                crate::INPUT_DEFER.replay_later(events);
            }
            Released::Discard(events) => {
                self.platform.timer.kill(crate::TIMER_RESUME_HOLD);
                tracing::warn!(
                    "[half-width-return] 保持の解放: reason={} held={held} hold_ms={hold_ms} → 前面窓が保持の開始時と違うので {} 打鍵を捨てた",
                    reason.label(),
                    events.len()
                );
            }
        }
    }

    /// `TIMER_RESUME_HOLD`(一発タイマー)の満了。冒頭で自ら kill するのは呼び出し側(`message_handlers`)。
    pub(crate) fn resume_hold_on_timer(&mut self) {
        self.resume_hold_release(None, ReleaseReason::Deadline);
    }

    /// `WM_RESUME_HOLD_DONE`(完了の札の通知)。
    pub(crate) fn resume_hold_on_done(&mut self, hold_id: usize, reason_code: isize) {
        let reason = ReleaseReason::from_code(reason_code);
        self.resume_hold_release(Some(hold_id as u64), reason);
    }
}
