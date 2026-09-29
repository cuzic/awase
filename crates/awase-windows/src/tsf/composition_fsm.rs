//! TSF composition の warmup タイミングを管理する FSM。
//!
//! executor に散在していた `pending_warmup_on_keyup: bool` のミニ FSM を
//! 状態として昇格させ、confirm キー（Space/Enter/Esc）・物理 F2・Ctrl↑ 等の
//! passthrough イベントから「いつ eager warmup を送るか」を決定する。
//!
//! ## 設計
//!
//! - 副作用なし。遷移ごとに [`CompositionAction`] を返し、dispatcher（`WindowsPlatform`）が
//!   `EmitWarmup` / `LatchWarmup` / `MarkCold` / `GjiCompositionReset` / `GjiNativeF2Consumed` を実行する。
//! - warm 判定そのものは GjiFsm が SSOT であり、この FSM は重複させない。ここが
//!   所有するのは「confirm キー KeyDown 後、KeyUp まで warmup を保留する」という
//!   executor 固有の遷移である。warm/tsf の現況は呼び出し元がイベントに載せて渡す。
//! - confirm キー KeyDown は WezTerm 等で F2 と Enter が競合する（F2 で新規
//!   composition 開始 → 即 Enter 確定）ため、warm+TSF では KeyUp まで warmup を遅らせる。
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

/// warmup を発火させる理由（診断用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WarmupReason {
    /// cold 状態の Ctrl↑（GJI recovery 再計測）
    CtrlUp,
}

/// composition 状態。
#[derive(Debug)]
pub(crate) enum CompositionState {
    /// 初期状態 / IME OFF 時
    Idle,
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
    /// Ctrl KeyUp（cold 状態で eager warmup リセット）
    CtrlUp { warm: bool },
    /// 物理 F2 (VK_DBE_HIRAGANA) KeyDown。`warm` は現況（`tsf_mode=false` 側でのみ参照）。
    NativeF2Down { tsf_mode: bool, warm: bool },
}

/// composition FSM が出力するアクション（dispatcher が副作用を実行する）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompositionAction {
    /// warmup を送信する。
    EmitWarmup { reason: WarmupReason },
    /// composition を cold にマークする。
    MarkCold { reason: ColdReason },
    /// `VK_IME_ON` は送らず、`eager_warmup_sent_ms`（focus probe grace の基準点）だけを新しい物理 F2 の時刻に
    /// 更新する（BUG-173: 物理 F2 は素通しなので代わりの warmup 送信は不要。ただし F2 が TSF 初期化を再トリガー
    /// するため、`mark_composition_cold(NativeF2Consumed)` が 0 に戻した基準点は保つ。BUG-06 の派生形の回避）。
    LatchWarmup,
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
}

impl CompositionFsm {
    pub(crate) const fn new() -> Self {
        Self {
            state: CompositionState::Idle,
        }
    }

    /// 現在状態の診断ラベル（dispatcher の debug ログ用）。
    pub(crate) fn state_label(&self) -> String {
        match &self.state {
            CompositionState::Idle => "Idle".to_owned(),
            CompositionState::Cold { reason } => format!("Cold({reason:?})"),
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
                tracing::trace!("[composition-fsm] ImeOn(tsf={tsf_mode}) → Cold");
                self.state = CompositionState::Cold {
                    reason: ColdReason::SetOpenTrue,
                };
                Response::consume()
            }
            CompositionEvent::ImeOff => {
                self.state = CompositionState::Idle;
                Response::consume()
            }

            // ── FocusChange ────────────────────────────────────────────────
            CompositionEvent::FocusChange { tsf_mode } => {
                tracing::trace!("[composition-fsm] FocusChange(tsf={tsf_mode}) → Cold");
                self.state = CompositionState::Cold {
                    reason: ColdReason::FocusChange,
                };
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
            CompositionEvent::NativeF2Down { tsf_mode, warm } => {
                if tsf_mode {
                    // 物理 F2 は素通し（BUG-173、`PhysicalKeyDisposition::plan`）。awase は代わりの
                    // warmup を送らない（送ると F2 と VK_IME_ON の SendInput 2連送になり、ADR-149/BUG-113 の
                    // 「@」の必要条件を作る）。cold 化と GjiFsm への通知、warmup 基準点の latch だけ行う。
                    // GjiNativeF2Consumed を使うことで GjiFsm が Medium/Long cold 状態を維持できる。
                    // GjiCompositionReset を使うと handle_composition_reset が Short に降格してしまい、
                    // Long cold の forces_prepend_f2/is_long_cold が失われる（Bug 1 の原因）。
                    self.state = CompositionState::Cold {
                        reason: ColdReason::NativeF2Consumed,
                    };
                    Response::emit(vec![
                        CompositionAction::MarkCold {
                            reason: ColdReason::NativeF2Consumed,
                        },
                        CompositionAction::GjiNativeF2Consumed,
                        CompositionAction::LatchWarmup,
                    ])
                } else if warm {
                    // 2026-07-19 (BUG-31): warm な状態で「TSF を経由しない F2 系キー」が
                    // 届いても、実際には何も冷えていない。確定キーの warm 時と同じ
                    // 理由（2026-07-11 修正、上記コメント参照）で、warm を確定キー以外の
                    // イベントで cold 化する根拠も無い。連続 typing 中に無関係な物理 IME
                    // キー（VK_DBE_HIRAGANA、自己注入ではない = 外部/OS 由来）が届くと、
                    // それだけで直後 1.8 秒後の全く無関係なタイピングまで cold-start
                    // 経路（F2/probe 待機省略 → per-VK confirm）に落とされ、GJI 候補
                    // ウィンドウ可視性のレース（BUG-29/BUG-30）に巻き込まれて文字が
                    // 消失する実機不具合を確認した（docs/known-bugs.md BUG-31）。
                    // 過去の F2NonTsf cold-mark が実際に有効だった事例
                    // （`3c275a7`/`79134f5`/`b5946bb`）はいずれも GJI long-idle
                    // （既に GjiFsm が OnCold(Long/Medium)）由来であり、warm 中に F2 系
                    // イベントが来て何かを温め直す必要があった実測事例は無い。よって
                    // warm 中は何もしない。
                    Response::consume()
                } else {
                    // 非 TSF・非 warm: cold mark のみ（Chrome/Win32 向け）。
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

    #[test]
    fn native_f2_in_tsf_marks_cold_and_latches_without_sending() {
        let mut fsm = CompositionFsm::new();
        let r = fsm.on_event(CompositionEvent::NativeF2Down {
            tsf_mode: true,
            warm: false,
        });
        assert!(
            !r.actions
                .iter()
                .any(|a| matches!(a, CompositionAction::EmitWarmup { .. })),
            "物理 F2 は素通しなので代わりの VK_IME_ON warmup は送らない（BUG-173）"
        );
        assert!(r.actions.contains(&CompositionAction::LatchWarmup));
        assert!(r.actions.iter().any(|a| matches!(
            a,
            CompositionAction::MarkCold {
                reason: ColdReason::NativeF2Consumed
            }
        )));
        assert!(r.actions.contains(&CompositionAction::GjiNativeF2Consumed));
    }

    #[test]
    fn native_f2_non_tsf_marks_cold_without_latch() {
        let mut fsm = CompositionFsm::new();
        let r = fsm.on_event(CompositionEvent::NativeF2Down {
            tsf_mode: false,
            warm: false,
        });
        assert!(!r.actions.contains(&CompositionAction::LatchWarmup));
        assert!(r.actions.iter().any(|a| matches!(
            a,
            CompositionAction::MarkCold {
                reason: ColdReason::F2NonTsf
            }
        )));
    }

    // 2026-07-19 (BUG-31): warm 中に「TSF を経由しない F2 系キー」が届いても、
    // 実際には何も冷えていない（連続 typing 中の無関係な物理 IME キーの副作用で
    // GJI 候補ウィンドウ可視性レースに巻き込まれ文字が消失する不具合の根治）。
    // warm_confirm_keydown_does_not_mark_cold_or_reset_gji と同じ理由で、
    // NativeF2Down(tsf_mode=false, warm=true) も MarkCold/GjiCompositionReset を
    // 一切発行しないべきである。
    #[test]
    fn native_f2_non_tsf_while_warm_is_noop() {
        let mut fsm = CompositionFsm::new();
        let r = fsm.on_event(CompositionEvent::NativeF2Down {
            tsf_mode: false,
            warm: true,
        });
        assert!(
            r.actions.is_empty(),
            "warm 中の非 TSF F2 は cold 化・GJI reset とも不要 (actions={:?})",
            r.actions
        );
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
