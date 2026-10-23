//! PR #809 の M1: 窓切替直後、refresh が新しい窓を読み終える前に届いた打鍵を保持し、refresh の完了で再生する殻。
//!
//! 判断は `state/focus_switch_hold.rs`(純粋な状態機械)にあり、ここは事実の収集(時刻・打鍵の同一性・世代)と
//! 実行(タイマー・`INPUT_DEFER.replay_later`)だけ。**IME へは何も書かない・belief を更新しない**(保持した打鍵を
//! 再生するだけ。再生された打鍵は `deliver_key_event` から通常どおり Engine を通り、その時点の新しい窓の状態で処理される)。
//!
//! 待機の入口は `on_window_focus_event`(フォーカス変更の検知)。解放の入口(すべて `focus_switch_hold_finish` を
//! 経由し、`TIMER_FOCUS_SWITCH_HOLD` の kill と再生はその 1 か所):
//! - refresh の完了(`run_ime_refresh`・`run_ime_refresh_with_prefetched` の後。spawn 時の世代で照合)
//! - 期限の一発タイマー `TIMER_FOCUS_SWITCH_HOLD`(`tuning::FOCUS_SWITCH_HOLD_MAX_MS`)
//! - 上限・期限超えの打鍵(`focus_switch_hold_on_delivery` の `OnKey::Release`)
//! - `WM_PANIC_RESET`・設定の再読み込み

use std::time::Duration;

use awase::types::{KeyClassification, ModifierKey, RawKeyEvent};

use crate::state::focus_switch_hold::{
    HoldKey, OnKey, StartKind, MOD_ALT, MOD_CTRL, MOD_SHIFT, MOD_WIN,
};
use crate::state::TickMs;
use crate::Runtime;

/// `RawKeyEvent` から保持の判断に使う事実を作る(分類だけ。判断は `state::focus_switch_hold::starts_hold` と状態機械)。
///
/// 種類: IME のモードキー(`vk::is_ime_mode_key_for_ime`。親指に設定した変換・無変換を含む)か、文字・親指
/// (`KeyClassification::{Char, LeftThumb, RightThumb}`)か、その他。修飾は押下時の `modifier_snapshot` と、この打鍵自体の `modifier_key`。
fn hold_key(event: &RawKeyEvent) -> HoldKey {
    let m = event.modifier_snapshot;
    let mods_at_press = (if m.shift { MOD_SHIFT } else { 0 })
        | (if m.ctrl { MOD_CTRL } else { 0 })
        | (if m.alt { MOD_ALT } else { 0 })
        | (if m.win { MOD_WIN } else { 0 });
    let modifier = match event.modifier_key {
        Some(ModifierKey::Shift) => MOD_SHIFT,
        Some(ModifierKey::Ctrl) => MOD_CTRL,
        Some(ModifierKey::Alt) => MOD_ALT,
        Some(ModifierKey::Meta) => MOD_WIN,
        None => 0,
    };
    // IME のモードキーを先に見る: 親指に設定した変換・無変換を Ctrl/Alt つきで押したときも IME の操作として保持を始める
    // (文字・親指の判定だと修飾つきは始めず、古い窓の文脈で素通しになる。閉ループ検証の E1 の「+10ms に右親指」)。
    let kind = if crate::vk::is_ime_mode_key_for_ime(event.vk_code) {
        StartKind::ImeKey
    } else if matches!(
        event.key_classification,
        KeyClassification::Char | KeyClassification::LeftThumb | KeyClassification::RightThumb
    ) {
        StartKind::CharOrThumb
    } else {
        StartKind::Other
    };
    HoldKey {
        facts: super::resume_hold::key_facts(event),
        mods_at_press,
        modifier,
        injected: event.injected,
        kind,
    }
}

impl Runtime {
    /// フォーカス変更を見た(`on_window_focus_event` から)。世代を進めて待機に入る。
    pub(super) fn focus_switch_hold_on_focus_event(&mut self) {
        self.focus_event_seq = self.focus_event_seq.wrapping_add(1);
        let now = TickMs(crate::hook::current_tick_ms());
        self.focus_switch_hold
            .on_focus_event(now, self.focus_event_seq);
    }

    /// `deliver_key_event` の先頭の保持の判定。`true` = どちらかが保持した(呼び出し側は `Consumed`)。
    ///
    /// 窓切替直後の保持を先に見る: 窓切替の後に届いた打鍵を、前の窓で始まった戻り待ちの保持(ADR-245 追補 2 R2。
    /// フォーカス変化で解放され、窓が違えば中身を捨てる)に足さない。
    pub(crate) fn hold_on_delivery(
        &mut self,
        event: &RawKeyEvent,
        origin: super::message_handlers::KeyOrigin,
    ) -> bool {
        self.focus_switch_hold_on_delivery(event, origin)
            || self.resume_hold_on_delivery(event, origin)
    }

    /// 窓切替直後の保持の判定。`true` = 保持した(呼び出し側は `Consumed`)。
    ///
    /// ime-off-rescue の再生は対象外(救済窓が保留していた古い打鍵で、origin の `skip_rescue_defer` を保つため。
    /// `resume_hold_on_delivery` と同じ)。
    fn focus_switch_hold_on_delivery(
        &mut self,
        event: &RawKeyEvent,
        origin: super::message_handlers::KeyOrigin,
    ) -> bool {
        if matches!(
            origin,
            super::message_handlers::KeyOrigin::ImeOffRescueReplay
        ) {
            return false;
        }
        let now = TickMs(crate::hook::current_tick_ms());
        let started_at = self.focus_switch_hold.started_at();
        let held_before = self.focus_switch_hold.len();
        match self.focus_switch_hold.on_key(now, hold_key(event), *event) {
            OnKey::Pass(_) => false,
            OnKey::Started => {
                self.platform.timer.set(
                    crate::TIMER_FOCUS_SWITCH_HOLD,
                    Duration::from_millis(crate::tuning::FOCUS_SWITCH_HOLD_MAX_MS),
                );
                tracing::info!(
                    "[focus-switch-hold] 窓切替直後の打鍵を保持 vk=0x{:02X} seq={}",
                    event.vk_code.0,
                    self.focus_event_seq
                );
                true
            }
            OnKey::Held => true,
            OnKey::Release(events) => {
                self.focus_switch_hold_finish(events, "Cap", held_before + 1, started_at);
                true
            }
        }
    }

    /// refresh が終わった。`seq` は refresh を spawn した時点の世代。
    pub(super) fn focus_switch_hold_on_refresh_settled(&mut self, seq: u64) {
        let started_at = self.focus_switch_hold.started_at();
        let held = self.focus_switch_hold.len();
        if let Some(events) = self.focus_switch_hold.on_refresh_settled(seq) {
            self.focus_switch_hold_finish(events, "Refreshed", held, started_at);
        }
    }

    /// `TIMER_FOCUS_SWITCH_HOLD`(一発タイマー)の満了。冒頭で自ら kill するのは呼び出し側(`message_handlers`)。
    pub(crate) fn focus_switch_hold_on_timer(&mut self) {
        let started_at = self.focus_switch_hold.started_at();
        let held = self.focus_switch_hold.len();
        if let Some(events) = self.focus_switch_hold.on_deadline() {
            self.focus_switch_hold_finish(events, "Deadline", held, started_at);
        }
    }

    /// 強制の解放(`WM_PANIC_RESET`・設定の再読み込み)。保持した打鍵は失わず再生する。
    pub(crate) fn focus_switch_hold_release_all(&mut self, reason: &'static str) {
        let started_at = self.focus_switch_hold.started_at();
        let held = self.focus_switch_hold.len();
        if let Some(events) = self.focus_switch_hold.release_all() {
            self.focus_switch_hold_finish(events, reason, held, started_at);
        }
    }

    /// 解放の出口(1 か所)。タイマーを kill し、順序どおり再生する。
    fn focus_switch_hold_finish(
        &mut self,
        events: Vec<RawKeyEvent>,
        reason: &'static str,
        held: usize,
        started_at: Option<TickMs>,
    ) {
        self.platform.timer.kill(crate::TIMER_FOCUS_SWITCH_HOLD);
        let hold_ms = started_at.map_or(0, |s| crate::hook::current_tick_ms().saturating_sub(s.0));
        tracing::info!(
            "[focus-switch-hold] 保持の解放: reason={reason} held={held} hold_ms={hold_ms} → 再生"
        );
        crate::INPUT_DEFER.replay_later(events);
    }
}
