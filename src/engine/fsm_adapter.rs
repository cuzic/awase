//! NicolaFsm と Decision/Effect 層の橋渡し。
//! timed-fsm の Response を Effect/Decision に変換する。

use timed_fsm::{Response, TimerCommand};

use crate::config::ConfirmMode;
use crate::ngram::NgramModel;
use crate::types::{ContextChange, KeyAction, RawKeyEvent};
use crate::yab::YabLayout;

use super::decision::{Decision, Effect, InputEffect, TimerEffect};
use super::input_tracker::PhysicalKeyState;
use super::nicola_fsm::NicolaFsm;

/// NicolaFsm と Decision/Effect 層の橋渡し。
/// timed-fsm の Response を Effect/Decision に変換する。
#[allow(missing_debug_implementations)]
pub struct FsmAdapter {
    fsm: NicolaFsm,
}

impl FsmAdapter {
    /// 新しい `FsmAdapter` を作成する。
    #[must_use]
    pub const fn new(fsm: NicolaFsm) -> Self {
        Self { fsm }
    }

    /// キーイベントを処理し、Decision を返す。
    pub fn on_event(&mut self, event: RawKeyEvent, phys: &PhysicalKeyState) -> Decision {
        let resp = self.fsm.on_event(event, phys);
        Self::response_to_decision(&resp)
    }

    /// タイマー満了時の処理。
    pub fn on_timeout(&mut self, timer_id: usize, phys: &PhysicalKeyState) -> Decision {
        let resp = self.fsm.on_timeout(timer_id, phys);
        Self::response_to_decision(&resp)
    }

    /// 保留中のキーをフラッシュし、Decision を返す。
    pub fn flush(&mut self, reason: ContextChange) -> Decision {
        let resp = self.fsm.flush_pending(reason);
        Self::response_to_decision(&resp)
    }

    /// フラッシュして Effect リストのみを返す（他の Effect と結合する用途）。
    pub fn flush_to_effects(&mut self, reason: ContextChange) -> Vec<Effect> {
        let resp = self.fsm.flush_pending(reason);
        Self::response_to_effects(&resp)
    }

    /// エンジンの有効/無効をトグルする。
    pub fn toggle_enabled(&mut self) -> (bool, Decision) {
        let (enabled, resp) = self.fsm.toggle_enabled();
        (enabled, Self::response_to_decision(&resp))
    }

    /// エンジンの有効/無効を明示的に設定する。
    pub fn set_enabled(&mut self, enabled: bool) -> (bool, Decision) {
        let (actual, resp) = self.fsm.set_enabled(enabled);
        (actual, Self::response_to_decision(&resp))
    }

    /// 配列を動的に差し替える。
    pub fn swap_layout(&mut self, layout: YabLayout) -> Decision {
        let resp = self.fsm.swap_layout(layout);
        Self::response_to_decision(&resp)
    }

    /// 同時打鍵判定の閾値を更新する（ミリ秒指定）。
    pub fn set_threshold_ms(&mut self, ms: u32) {
        self.fsm.set_threshold_ms(ms);
    }

    /// 確定モードと投機出力の待機時間を更新する。
    pub fn set_confirm_mode(&mut self, mode: ConfirmMode, delay_ms: u32) {
        self.fsm.set_confirm_mode(mode, delay_ms);
    }

    /// n-gram モデルを設定する。
    pub fn set_ngram_model(&mut self, model: NgramModel) {
        self.fsm.set_ngram_model(model);
    }

    /// エンジンが有効かどうかを返す。
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.fsm.is_enabled()
    }

    // ── 内部メソッド ──

    /// timed-fsm Response → Effect リストに変換（consumed フラグは呼び出し側で判定）
    fn response_to_effects(resp: &Response<KeyAction, usize>) -> Vec<Effect> {
        let mut effects = Vec::new();
        for cmd in &resp.timers {
            match cmd {
                TimerCommand::Set { id, duration } => {
                    effects.push(Effect::Timer(TimerEffect::Set {
                        id: *id,
                        duration: *duration,
                    }));
                }
                TimerCommand::Kill { id } => {
                    effects.push(Effect::Timer(TimerEffect::Kill(*id)));
                }
            }
        }
        if !resp.actions.is_empty() {
            effects.push(Effect::Input(InputEffect::SendKeys(resp.actions.clone())));
        }
        effects
    }

    /// timed-fsm Response → Decision に変換
    fn response_to_decision(resp: &Response<KeyAction, usize>) -> Decision {
        let effects = Self::response_to_effects(resp);
        if resp.consumed {
            Decision::consumed_with(effects)
        } else if effects.is_empty() {
            Decision::pass_through()
        } else {
            Decision::pass_through_with(effects)
        }
    }
}
