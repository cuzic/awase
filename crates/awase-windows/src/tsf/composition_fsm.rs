//! TSF composition の warmup タイミングを管理する FSM。
//!
//! executor に散在していた `pending_warmup_on_keyup: bool` のミニ FSM を
//! 状態として昇格させ、confirm キー（Space/Enter/Esc）・物理 F2・Ctrl↑ 等の
//! passthrough イベントから「いつ eager warmup を送るか」を決定する。
//!
//! ## 設計
//!
//! - 副作用なし。遷移ごとに [`CompositionAction`] を返し、dispatcher（`WindowsPlatform`）が
//!   `EmitWarmup` / `MarkCold` / `ConsumeF2` / `GjiCompositionReset` / `GjiNativeF2Consumed` を実行する。
//! - warm 判定そのものは GjiFsm が SSOT であり、この FSM は重複させない。ここが
//!   所有するのは「confirm キー KeyDown 後、KeyUp まで warmup を保留する」という
//!   executor 固有の遷移である。warm/tsf の現況は呼び出し元がイベントに載せて渡す。
//! - confirm キー KeyDown は WezTerm 等で F2 と Enter が競合する（F2 で新規
//!   composition 開始 → 即 Enter 確定）ため、warm+TSF では KeyUp まで warmup を遅らせる。
//! - `epoch` はフォーカス変更を跨いだ stale な `PendingWarmupOnKeyUp` を弾く内部カウンタ。
//!   FSM が自前で保持・更新するためイベントには載せない。
//! - タイマーは不要なので `TimerId = std::convert::Infallible`。
//!
//! ## GjiFsm との warm/cold の違い
//!
//! `CompositionFsm` と `GjiFsm` はどちらも warm/cold の概念を持つが、意味が異なる。
//!
//! - **CompositionFsm**: 「最後の warmup シーケンスを送った」という**タイミング制御**の状態。
//!   confirm キーや F2 の KeyDown/Up タイミングに応じて warmup の送信を遅延・即時化する。
//!
//! - **GjiFsm**: 「GJI が実際に readiness を確認済みか」という**事実推測**の状態。
//!   probe（TsfReadinessProbe）による観測結果で更新される。
//!
//! 両者は独立して管理されており、統合は意図的にしていない。
//! dispatcher（`platform.rs`）が両方に対して個別にイベントを送る。

use std::convert::Infallible;

use timed_fsm::{Response, TimedStateMachine};

use crate::output::ColdReason;
use crate::tsf::gji_fsm::FocusEpoch;
use awase::types::VkCode;

/// warmup を発火させる理由（診断用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WarmupReason {
    /// cold 状態の Ctrl↑（GJI recovery 再計測）
    CtrlUp,
    /// warm+TSF confirm キーの KeyUp（KeyDown で保留した warmup を送信）
    ConfirmKeyUp,
    /// TSF mode の物理 F2 を consume した代替 warmup
    NativeF2,
    /// cold / 非 TSF confirm キー KeyDown 直後の即時 warmup
    ConfirmKeyDown,
}

/// composition 状態。
#[derive(Debug)]
pub(crate) enum CompositionState {
    /// 初期状態 / IME OFF 時
    Idle,
    /// TSF warm（通常入力中）
    Warm { tsf_mode: bool },
    /// 確定キー(Space/Enter/Esc)KeyDown後、KeyUpでwarmupを送るまで待機
    PendingWarmupOnKeyUp {
        confirm_vk: VkCode,
        tsf_mode: bool,
        epoch: FocusEpoch,
    },
    /// TSF cold（次の入力でwarmupが必要）
    Cold { reason: ColdReason },
}

/// composition FSM へのイベント。
#[derive(Debug)]
pub(crate) enum CompositionEvent {
    /// IME ON / TSF mode 開始
    ImeOn { tsf_mode: bool },
    /// IME OFF
    ImeOff,
    /// フォーカス変更
    FocusChange { tsf_mode: bool },
    /// 確定キー(Space/Enter/Esc) KeyDown。`warm`/`tsf_mode` は現況。
    ConfirmKeyDown {
        vk: VkCode,
        tsf_mode: bool,
        warm: bool,
    },
    /// 確定キー KeyUp
    ConfirmKeyUp { vk: VkCode },
    /// Ctrl KeyUp（cold 状態で eager warmup リセット）
    CtrlUp { warm: bool },
    /// 物理 F2 (VK_DBE_HIRAGANA) KeyDown
    NativeF2Down { tsf_mode: bool },
}

/// composition FSM が出力するアクション（dispatcher が副作用を実行する）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompositionAction {
    /// warmup を送信する。
    EmitWarmup { reason: WarmupReason },
    /// composition を cold にマークする。
    MarkCold { reason: ColdReason },
    /// F2 を consume する（TSF mode で物理 F2 を swallow する）。
    ConsumeF2,
    /// GJI composition reset を通知する。
    GjiCompositionReset,
    /// TSF mode での物理 F2 消費を GjiFsm に通知する（NativeF2Down(tsf_mode=true) 専用）。
    ///
    /// `GjiCompositionReset` の代わりに使用することで、GjiFsm が Medium/Long cold 中に
    /// `OnCold(Long/Medium)` 状態を維持できる（`handle_composition_reset` による Short 降格を回避する）。
    GjiNativeF2Consumed,
}

/// composition warmup タイミング FSM。
pub(crate) struct CompositionFsm {
    state: CompositionState,
    /// フォーカスを跨いだ stale な `PendingWarmupOnKeyUp` を弾く単調カウンタ。
    epoch: FocusEpoch,
}

impl CompositionFsm {
    pub(crate) const fn new() -> Self {
        Self {
            state: CompositionState::Idle,
            epoch: FocusEpoch::ZERO,
        }
    }

    /// 現在状態の診断ラベル（dispatcher の debug ログ用）。
    pub(crate) fn state_label(&self) -> String {
        match &self.state {
            CompositionState::Idle => "Idle".to_owned(),
            CompositionState::Warm { tsf_mode } => format!("Warm(tsf={tsf_mode})"),
            CompositionState::PendingWarmupOnKeyUp {
                confirm_vk,
                tsf_mode,
                epoch,
            } => format!(
                "PendingWarmupOnKeyUp(vk={:#04x}, tsf={tsf_mode}, {epoch:?})",
                confirm_vk.0
            ),
            CompositionState::Cold { reason } => format!("Cold({reason:?})"),
        }
    }

    /// `PendingWarmupOnKeyUp` で待機中の confirm VK を返す（デバッグ / 照合用）。
    pub(crate) const fn pending_warmup_vk(&self) -> Option<VkCode> {
        match self.state {
            CompositionState::PendingWarmupOnKeyUp { confirm_vk, .. } => Some(confirm_vk),
            _ => None,
        }
    }
}

impl Default for CompositionFsm {
    fn default() -> Self {
        Self::new()
    }
}

impl TimedStateMachine for CompositionFsm {
    type Event = CompositionEvent;
    type Action = CompositionAction;
    type TimerId = Infallible;

    fn on_event(&mut self, event: CompositionEvent) -> Response<CompositionAction, Infallible> {
        match event {
            // ── IME ON / OFF ───────────────────────────────────────────────
            CompositionEvent::ImeOn { tsf_mode } => {
                // IME ON 直後は cold（次の入力で warmup が必要）。
                log::trace!("[composition-fsm] ImeOn(tsf={tsf_mode}) → Cold");
                self.state = CompositionState::Cold {
                    reason: ColdReason::SetOpenTrue,
                };
                Response::consume()
            }
            CompositionEvent::ImeOff => {
                self.epoch = self.epoch.next();
                self.state = CompositionState::Idle;
                Response::consume()
            }

            // ── FocusChange ────────────────────────────────────────────────
            CompositionEvent::FocusChange { tsf_mode } => {
                log::trace!("[composition-fsm] FocusChange(tsf={tsf_mode}) → Cold (epoch++)");
                self.epoch = self.epoch.next();
                self.state = CompositionState::Cold {
                    reason: ColdReason::FocusChange,
                };
                Response::consume()
            }

            // ── ConfirmKeyDown ─────────────────────────────────────────────
            CompositionEvent::ConfirmKeyDown { vk, tsf_mode, warm } => {
                if warm {
                    // warm（TSF/Chrome 共通）: KeyUp まで warmup を遅延する（F2 と Enter の競合回避）。
                    // 2026-07 まで Chrome (tsf_mode=false) はこの分岐を通らず、warm でも
                    // 即 cold mark + reset していた（a3425bf でフラグ統合した際に
                    // WezTerm 専用ルールを is_tsf_mode() ガードなしで引き継いだ副作用。
                    // Chrome 固有の根拠は無く、cold-start warmup が確定キーのたびに
                    // 過剰発火していた）。warm な GJI/TSF を確定キーだけで cold 化する理由は
                    // tsf_mode に関係なく無いため、判定を warm 単独に統一した。
                    self.state = CompositionState::PendingWarmupOnKeyUp {
                        confirm_vk: vk,
                        tsf_mode,
                        epoch: self.epoch,
                    };
                    Response::emit(vec![
                        CompositionAction::MarkCold {
                            reason: ColdReason::PassthroughConfirmKey,
                        },
                        CompositionAction::GjiCompositionReset,
                    ])
                } else {
                    // cold: 即 cold mark + warmup。
                    self.state = CompositionState::Cold {
                        reason: ColdReason::PassthroughConfirmKey,
                    };
                    Response::emit(vec![
                        CompositionAction::MarkCold {
                            reason: ColdReason::PassthroughConfirmKey,
                        },
                        CompositionAction::GjiCompositionReset,
                        CompositionAction::EmitWarmup {
                            reason: WarmupReason::ConfirmKeyDown,
                        },
                    ])
                }
            }

            // ── ConfirmKeyUp ───────────────────────────────────────────────
            CompositionEvent::ConfirmKeyUp { vk } => {
                if let CompositionState::PendingWarmupOnKeyUp {
                    confirm_vk,
                    tsf_mode,
                    epoch,
                } = self.state
                {
                    if confirm_vk == vk && epoch == self.epoch {
                        self.state = CompositionState::Warm { tsf_mode };
                        return Response::emit_one(CompositionAction::EmitWarmup {
                            reason: WarmupReason::ConfirmKeyUp,
                        });
                    }
                }
                Response::consume()
            }

            // ── CtrlUp ─────────────────────────────────────────────────────
            CompositionEvent::CtrlUp { warm } => {
                if warm {
                    Response::consume()
                } else {
                    // cold 状態の Ctrl↑: GJI recovery のために warmup を再送する。
                    Response::emit_one(CompositionAction::EmitWarmup {
                        reason: WarmupReason::CtrlUp,
                    })
                }
            }

            // ── NativeF2Down ───────────────────────────────────────────────
            CompositionEvent::NativeF2Down { tsf_mode } => {
                if tsf_mode {
                    // 物理 F2 を consume し、代替の warmup F2 で一本化する（double-F2 防止）。
                    // GjiNativeF2Consumed を使うことで GjiFsm が Medium/Long cold 状態を維持できる。
                    // GjiCompositionReset を使うと handle_composition_reset が Short に降格してしまい、
                    // Long cold の forces_prepend_f2/is_long_cold が失われる（Bug 1 の原因）。
                    self.state = CompositionState::Cold {
                        reason: ColdReason::NativeF2Consumed,
                    };
                    Response::emit(vec![
                        CompositionAction::ConsumeF2,
                        CompositionAction::MarkCold {
                            reason: ColdReason::NativeF2Consumed,
                        },
                        CompositionAction::GjiNativeF2Consumed,
                        CompositionAction::EmitWarmup {
                            reason: WarmupReason::NativeF2,
                        },
                    ])
                } else {
                    // 非 TSF: cold mark のみ（Chrome/Win32 向け）。
                    self.state = CompositionState::Cold {
                        reason: ColdReason::F2NonTsf,
                    };
                    Response::emit(vec![
                        CompositionAction::MarkCold {
                            reason: ColdReason::F2NonTsf,
                        },
                        CompositionAction::GjiCompositionReset,
                    ])
                }
            }
        }
    }

    fn on_timeout(&mut self, timer_id: Infallible) -> Response<CompositionAction, Infallible> {
        // TimerId = Infallible なので到達不能。
        match timer_id {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENTER: VkCode = VkCode(0x0D);

    fn warm_tsf_confirm_down(vk: VkCode) -> CompositionEvent {
        CompositionEvent::ConfirmKeyDown {
            vk,
            tsf_mode: true,
            warm: true,
        }
    }

    #[test]
    fn warm_tsf_confirm_keydown_defers_warmup_to_keyup() {
        let mut fsm = CompositionFsm::new();
        let r = fsm.on_event(warm_tsf_confirm_down(ENTER));
        // KeyDown では warmup を出さず、cold mark + gji reset のみ。
        assert!(
            !r.actions
                .iter()
                .any(|a| matches!(a, CompositionAction::EmitWarmup { .. })),
            "warm+TSF の KeyDown では warmup を遅延する"
        );
        assert_eq!(fsm.pending_warmup_vk(), Some(ENTER));

        let r = fsm.on_event(CompositionEvent::ConfirmKeyUp { vk: ENTER });
        assert_eq!(
            r.actions,
            vec![CompositionAction::EmitWarmup {
                reason: WarmupReason::ConfirmKeyUp
            }],
            "KeyUp で保留 warmup を送信する"
        );
        assert!(fsm.state_label().starts_with("Warm"));
    }

    #[test]
    fn warm_chrome_confirm_keydown_defers_warmup_to_keyup() {
        // 2026-07: 以前は tsf_mode=false (Chrome) だと warm でも即 cold mark + warmup
        // していた（a3425bf でフラグ統合した際に WezTerm 専用ルールを is_tsf_mode()
        // ガードなしで引き継いだ副作用）。warm な GJI/TSF を確定キーだけで即時再送する
        // 理由は tsf_mode に関係なく無いため、TSF と同じ KeyUp 遅延に統一した。
        let mut fsm = CompositionFsm::new();
        let r = fsm.on_event(CompositionEvent::ConfirmKeyDown {
            vk: ENTER,
            tsf_mode: false,
            warm: true,
        });
        assert!(
            !r.actions
                .iter()
                .any(|a| matches!(a, CompositionAction::EmitWarmup { .. })),
            "warm+Chrome の KeyDown でも warmup を遅延する"
        );
        assert_eq!(fsm.pending_warmup_vk(), Some(ENTER));

        let r = fsm.on_event(CompositionEvent::ConfirmKeyUp { vk: ENTER });
        assert_eq!(
            r.actions,
            vec![CompositionAction::EmitWarmup {
                reason: WarmupReason::ConfirmKeyUp
            }],
            "KeyUp で保留 warmup を送信する"
        );
        assert!(fsm.state_label().starts_with("Warm"));
    }

    #[test]
    fn cold_confirm_keydown_emits_warmup_immediately() {
        let mut fsm = CompositionFsm::new();
        let r = fsm.on_event(CompositionEvent::ConfirmKeyDown {
            vk: ENTER,
            tsf_mode: true,
            warm: false,
        });
        assert!(
            r.actions.iter().any(|a| matches!(
                a,
                CompositionAction::EmitWarmup {
                    reason: WarmupReason::ConfirmKeyDown
                }
            )),
            "cold では KeyDown で即 warmup"
        );
        assert_eq!(fsm.pending_warmup_vk(), None);
    }

    #[test]
    fn focus_change_invalidates_pending_warmup_keyup() {
        let mut fsm = CompositionFsm::new();
        fsm.on_event(warm_tsf_confirm_down(ENTER));
        // フォーカス変更で epoch が進み、保留 warmup は stale になる。
        fsm.on_event(CompositionEvent::FocusChange { tsf_mode: true });
        let r = fsm.on_event(CompositionEvent::ConfirmKeyUp { vk: ENTER });
        assert!(
            r.actions.is_empty(),
            "focus change を跨いだ KeyUp は warmup しない"
        );
    }

    #[test]
    fn mismatched_vk_keyup_does_not_emit() {
        let mut fsm = CompositionFsm::new();
        fsm.on_event(warm_tsf_confirm_down(ENTER));
        let r = fsm.on_event(CompositionEvent::ConfirmKeyUp { vk: VkCode(0x20) });
        assert!(r.actions.is_empty(), "別 VK の KeyUp は warmup しない");
    }

    #[test]
    fn native_f2_in_tsf_consumes_and_warms() {
        let mut fsm = CompositionFsm::new();
        let r = fsm.on_event(CompositionEvent::NativeF2Down { tsf_mode: true });
        assert!(r.actions.contains(&CompositionAction::ConsumeF2));
        assert!(r.actions.iter().any(|a| matches!(
            a,
            CompositionAction::EmitWarmup {
                reason: WarmupReason::NativeF2
            }
        )));
    }

    #[test]
    fn native_f2_non_tsf_marks_cold_without_consume() {
        let mut fsm = CompositionFsm::new();
        let r = fsm.on_event(CompositionEvent::NativeF2Down { tsf_mode: false });
        assert!(!r.actions.contains(&CompositionAction::ConsumeF2));
        assert!(r.actions.iter().any(|a| matches!(
            a,
            CompositionAction::MarkCold {
                reason: ColdReason::F2NonTsf
            }
        )));
    }

    #[test]
    fn ctrl_up_while_cold_emits_warmup() {
        let mut fsm = CompositionFsm::new();
        let r = fsm.on_event(CompositionEvent::CtrlUp { warm: false });
        assert_eq!(
            r.actions,
            vec![CompositionAction::EmitWarmup {
                reason: WarmupReason::CtrlUp
            }]
        );
    }

    #[test]
    fn ctrl_up_while_warm_is_noop() {
        let mut fsm = CompositionFsm::new();
        let r = fsm.on_event(CompositionEvent::CtrlUp { warm: true });
        assert!(r.actions.is_empty());
    }
}
