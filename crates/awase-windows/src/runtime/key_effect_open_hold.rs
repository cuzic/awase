//! ADR-292 P(BUG-233): モードキー予測(`KeyEffectPredicted` の開)が mozc-shm の実測で未照合の間、Engine に入る前の
//! 物理キーを保持し、確認・反証してから順序どおり再生する殻(「第3のhold」)。
//!
//! 判断は `state/key_effect_open_hold.rs`(純粋な状態機械と解放判定の表、Linux で単体テスト)にあり、ここは事実の収集
//! (前面窓・時刻・打鍵の同一性・予測・mozc-shm の読み)と実行(タイマー・`INPUT_DEFER.replay_later`・ログ)だけ。
//! **IME へは何も書かない**。belief への入口は既存の `report_mozc_shm` だけ(`mozc_shm_read_for_hold` 経由、ADR-292 S6)。
//!
//! 武装の入口は `kp_predict_key_effect`(予測が開に倒れた直後、`key_effect_open_hold_arm`)。保持は武装の後に来る次の
//! 非リピートの KeyDown から始まる(予測を立てた打鍵自身は通常どおり処理済み)。
//!
//! 解放の入口(すべて `key_effect_open_hold_release` を経由し、`TIMER_KEY_EFFECT_OPEN_HOLD` の kill と再生・破棄は
//! `key_effect_open_hold_finish` の 1 か所):
//! - 短周期の読み `TIMER_KEY_EFFECT_OPEN_HOLD`(`tuning::KEY_EFFECT_OPEN_HOLD_POLL_MS`)の判定(`judge`): 開と確認・閉と確定
//!   (R6(iv))・読めない(U1)・予測が他の経路で照合済み・期限(`tuning::KEY_EFFECT_OPEN_HOLD_MAX_MS`)
//! - 保持の上限(`key_effect_open_hold_on_delivery` の `OnKey::Release`)
//! - フォーカスの変化(`ir_notify_focus_changed`。窓が違えば捨てて warn を残す、ADR-292 T3)・`WM_PANIC_RESET`・設定の再読み込み
//!
//! **再生したモードキーの扱い(ADR-292 round3 T2・round4 U3)**: 保持中に押し直されたモードキーも区別せず保持する。再生は
//! `INPUT_DEFER.replay_later` → `handle_wm_drain_output_queue` → `deliver_key_event` → `kp_run_inner` の通常の経路を通り、
//! 再生した `RawKeyEvent` は元の `injected == false` を保つので、`kp_stage_key_effect_track` の `is_physical_key_down`
//! (`KeyDown && !injected`)を満たし、`kp_predict_key_effect` → `apply_key_effect_prediction` が**再生の時点**で改めて
//! 走る(`KeyEffectPredicted` の時刻 = 再生時刻、`last_mode_key_tick_ms` も再生時刻で進む。取り込み時点の打刻にはならない)。
//! 同じ経路で `kp_predict_key_effect` の末尾から `key_effect_open_hold_arm` も呼ばれ、新しい予測(新しい `pred.at_ms`)に
//! ついて改めて武装する(U2 の「予測ごとに1回」は予測の時刻で識別するので、押し直しは新しい対象になる)。
//! `focus_switch_hold`・`resume_hold` が再生するモードキーも同じ経路を通る(どちらも `replay_later` で、`injected` を
//! 書き換えない)。
//!
//! **既存の 2 つの hold との順序**: `hold_on_delivery` は窓切替直後の保持 → 戻り待ちの保持 → この保持の順に見る。
//! 前の 2 つは窓・フォーカスに紐づく保持で、その間に届いた打鍵は新しい窓の文脈で扱うべきなので先に取る。この保持は
//! それらが手放した(再生した)打鍵が Engine に入る直前に掛かる(再生された打鍵も `deliver_key_event` を通る)。

use std::time::Duration;

use awase::types::RawKeyEvent;

use crate::state::key_effect_open_hold::{
    hold_baseline_ms, judge, ArmFacts, OnKey, ReleaseReason, Released, Verdict,
};
use crate::state::TickMs;
use crate::Runtime;

impl Runtime {
    /// 未照合の予測の「開」の時刻(`pred.at_ms`)。開でない・予測が無い(照合済み)なら `None`。
    fn key_effect_open_pending_at(&self) -> Option<TickMs> {
        self.platform_state
            .ime
            .model()
            .key_effect()
            .filter(|p| p.open == Some(true))
            .map(|p| TickMs(p.at_ms))
    }

    /// 予測が開に倒れた直後に呼ぶ(`kp_predict_key_effect`)。始める条件(ADR-292 T4: GJI の証拠が無い・前面窓のスレッドの
    /// 直前の mozc-shm の読みが受け入れ・この予測でまだ保持していない)が揃えば武装する。判断は純粋側の `arm`。
    pub(super) fn key_effect_open_hold_arm(&mut self) {
        let facts = ArmFacts {
            gji_evidence_absent: crate::tsf::observer::gji_evidence_absent(),
            shm_accepted_for_focus: self.mozc_shm_accepted_for_focus(),
            predicted_open_at: self.key_effect_open_pending_at(),
        };
        if self.key_effect_open_hold.arm(
            facts,
            crate::win32::foreground_scope(),
            TickMs(crate::hook::current_tick_ms()),
        ) {
            tracing::info!(
                "[key-effect-open-hold] 武装: 予測の開が mozc-shm で未照合 pred_at={:?}",
                facts.predicted_open_at
            );
        }
    }

    /// `deliver_key_event` の先頭(`hold_on_delivery`)と `kp_run_inner` の Phase A の直後から呼ぶ。`true` = 保持した
    /// (呼び出し側は `Consumed`)。ime-off-rescue の再生は対象外(他の 2 つの hold と同じ。origin の `skip_rescue_defer` を保つ)。
    pub(crate) fn key_effect_open_hold_on_delivery(
        &mut self,
        event: &RawKeyEvent,
        origin: super::message_handlers::KeyOrigin,
    ) -> bool {
        !matches!(
            origin,
            super::message_handlers::KeyOrigin::ImeOffRescueReplay
        ) && self.key_effect_open_hold_on_key(event)
    }

    /// 保持中・武装中なら打鍵を純粋側へ渡す。`true` = 保持に足した・保持を始めた(呼び出し側は `Consumed`)。
    /// ime-off-rescue の再生を対象外にするのは呼び出し側(`key_effect_open_hold_on_delivery` は origin で、
    /// `kp_run_inner` は `skip_rescue_defer` で見る)。
    pub(super) fn key_effect_open_hold_on_key(&mut self, event: &RawKeyEvent) -> bool {
        if !self.key_effect_open_hold.is_holding() && !self.key_effect_open_hold.is_armed() {
            return false;
        }
        let current = crate::win32::foreground_scope();
        let pending = self.key_effect_open_pending_at();
        let now = TickMs(crate::hook::current_tick_ms());
        let held_before = self.key_effect_open_hold.len();
        let started_at = self.key_effect_open_hold.started_at();
        let facts = super::resume_hold::key_facts(event);
        match self
            .key_effect_open_hold
            .on_key(facts, *event, current, pending, now)
        {
            OnKey::Pass(_) => false,
            OnKey::Started(hold_id) => {
                // 保持中は短周期の読みだけにし、通常の 500ms の読みのタイマーを止める(同じ読みを二重に判定しない)。
                self.mozc_shm_suspend_timer(true);
                self.platform.timer.set(
                    crate::TIMER_KEY_EFFECT_OPEN_HOLD,
                    Duration::from_millis(crate::tuning::KEY_EFFECT_OPEN_HOLD_POLL_MS),
                );
                tracing::info!(
                    "[key-effect-open-hold] 予測の開が未照合なので打鍵を保持 vk=0x{:02X} hold_id={hold_id} pred_at={:?}",
                    event.vk_code.0,
                    pending
                );
                true
            }
            OnKey::Held => true,
            OnKey::Release(released) => {
                self.key_effect_open_hold_finish(
                    released,
                    ReleaseReason::Cap,
                    held_before + 1,
                    started_at,
                );
                true
            }
        }
    }

    /// `TIMER_KEY_EFFECT_OPEN_HOLD`(一発タイマー)の満了。冒頭で自ら kill するのは呼び出し側(`message_handlers`)。
    ///
    /// mozc-shm を読み、保持専用の非ローリングの起点(予測の時刻と S1 の送信打刻の大きい方、ADR-292 round3 T1)で
    /// 既存の `report_mozc_shm`(→`decide_use`)に通す。belief の更新(閉の確定なら追随)はその中で通常どおり行われる(S6)。
    /// 結論は純粋側の `judge`: 続けるならタイマーを張り直す。
    pub(crate) fn key_effect_open_hold_on_timer(&mut self) {
        let Some(pred_at) = self.key_effect_open_hold.pred_at_ms() else {
            return;
        };
        let actuation =
            crate::hook::imm_cross_write_tick_ms().max(crate::hook::ime_actuation_send_tick_ms());
        let baseline = hold_baseline_ms(pred_at, actuation);
        let read = if self.platform_state.focus.app_disabled {
            None
        } else {
            self.mozc_shm_read_for_hold(baseline)
        };
        let prediction_pending = self.key_effect_open_pending_at() == Some(pred_at);
        let now = crate::hook::current_tick_ms();
        let held_ms = self
            .key_effect_open_hold
            .started_at()
            .map_or(0, |s| now.saturating_sub(s.0));
        match judge(
            read,
            prediction_pending,
            held_ms,
            crate::tuning::KEY_EFFECT_OPEN_HOLD_MAX_MS,
        ) {
            Verdict::Keep => {
                self.platform.timer.set(
                    crate::TIMER_KEY_EFFECT_OPEN_HOLD,
                    Duration::from_millis(crate::tuning::KEY_EFFECT_OPEN_HOLD_POLL_MS),
                );
            }
            Verdict::Release(reason) => {
                if reason == ReleaseReason::ObservedClosed {
                    // R6(iv): belief は `report_mozc_shm` の追随で閉に直っている。再生の前に Engine へ反映する
                    // (再生した打鍵は閉の belief で素通しとして処理される)。
                    self.notify_engine_refresh();
                }
                self.key_effect_open_hold_release(None, reason);
            }
        }
    }

    /// 保持を解放する(フォーカスの変化・panic・設定の再読み込み・判定)。`hold_id` が `Some` なら今の保持と一致する
    /// ときだけ。フォーカスの変化・panic・設定の再読み込みでは武装も解く。
    pub(crate) fn key_effect_open_hold_release(
        &mut self,
        hold_id: Option<u64>,
        reason: ReleaseReason,
    ) {
        if matches!(
            reason,
            ReleaseReason::FocusChanged | ReleaseReason::PanicReset | ReleaseReason::ConfigReload
        ) {
            self.key_effect_open_hold.disarm();
        }
        let current = crate::win32::foreground_scope();
        let held = self.key_effect_open_hold.len();
        let started_at = self.key_effect_open_hold.started_at();
        let released = self.key_effect_open_hold.release(hold_id, current);
        self.key_effect_open_hold_finish(released, reason, held, started_at);
    }

    /// 解放の出口(1 か所)。タイマーを kill し、再生する(窓が違えば捨てて warn を残す)。
    fn key_effect_open_hold_finish(
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
                self.platform.timer.kill(crate::TIMER_KEY_EFFECT_OPEN_HOLD);
                self.mozc_shm_suspend_timer(false);
                if reason.needs_notice() {
                    // ADR-292 R6(iv): 利用者に知らせる。既存の通知手段のうち、毎回出ると邪魔になるバルーン(round3 T6)は
                    // 使わず、今はログだけ(窓クラスごとの学習・初回だけの通知は未決定)。
                    tracing::warn!(
                        "[key-effect-open-hold] モードキーの予測(開)を mozc-shm で確認できなかった: reason={} held={held} hold_ms={hold_ms} → belief={} で再生",
                        reason.label(),
                        self.platform_state.ime.effective_open()
                    );
                } else {
                    tracing::info!(
                        "[key-effect-open-hold] 保持の解放: reason={} held={held} hold_ms={hold_ms} → 再生",
                        reason.label()
                    );
                }
                crate::INPUT_DEFER.replay_later(events);
            }
            Released::Discard(events) => {
                self.platform.timer.kill(crate::TIMER_KEY_EFFECT_OPEN_HOLD);
                self.mozc_shm_suspend_timer(false);
                // ADR-292 T3: 黙って捨てない(別窓へ誤配送しないために捨てるが、ログに残す)。
                tracing::warn!(
                    "[key-effect-open-hold] 保持の解放: reason={} held={held} hold_ms={hold_ms} → 前面窓が保持の開始時と違うので {} 打鍵を捨てた",
                    reason.label(),
                    events.len()
                );
            }
        }
    }
}
