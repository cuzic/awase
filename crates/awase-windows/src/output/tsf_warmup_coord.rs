//! `Output` から抽出した GJI ウォームアップ / TSF プローブ調停コンポーネント。
//!
//! GJI warmup 戦略・保留 TSF プローブ FSM・probe_id・OUTPUT_GATE ガード・
//! GJI FSM への橋渡しバッファ群を一括して管理する。`Output` はこの構造体への
//! Facade として残り、各操作をメソッド委譲する。
//!
//! `step_probe` のように `Output` 全体（`ime_mode_fsm`・`tsf_gate` 等）へのアクセスが
//! 必要な処理は `Output` 側に残し、ここではプローブ状態の take/set を提供する。

use super::TimerCommand;
use awase::types::VkCode;
use std::cell::{Cell, RefCell};
use std::time::Duration;

use crate::tsf::gji_fsm::{
    FocusEpoch, GjiAction, GjiEvent, GjiFsm, ProbeId, ProbeParams, WarmupResult,
};
use crate::tsf::gji_fsm::GjiTimer;
use crate::tsf::probe_bridge::OutputActiveGuard;
use crate::tsf::tickable_fsm::TickableFsm;
use crate::tsf::warmup_strategy::ImeWarmupStrategy;

type GjiResponse = timed_fsm::Response<GjiAction, GjiTimer>;

/// GJI ウォームアップ / TSF プローブ状態の集約。
///
/// フィールドは `Output`（親モジュール `output` とその子モジュール）から直接借用できるよう
/// `pub(super)` にしているが、`platform.rs` 等モジュール外からはメソッド経由でのみ操作する。
pub(crate) struct TsfWarmupCoordinator {
    /// IME の warm/cold ウォームアップ戦略（GJI: `GjiFsm`、MS-IME: `MsImeStrategy`）。
    pub(super) tsf_warmup: RefCell<Box<dyn ImeWarmupStrategy>>,
    /// TIMER_TSF_PROBE で処理中の保留 TSF/VK probe ステートマシン。
    pub(super) pending_tsf: RefCell<Option<Box<dyn TickableFsm>>>,
    /// 現在実行中の GJI probe の ID（`GjiAction::StartProbe` 受信時にセット）。
    pub(super) current_gji_probe_id: Cell<Option<ProbeId>>,
    /// GJI probe 中に OUTPUT_GATE を活性化するガード。
    gji_probe_guard: RefCell<Option<OutputActiveGuard>>,
    /// `dispatch_probe_actions` → `GjiFsm::WarmupComplete` の橋渡しバッファ。
    pending_gji_warmup: Cell<Option<WarmupResult>>,
    /// `ProbeIo::mark_cold_raw_tsf` → `GjiFsm::CompositionReset` の橋渡しフラグ。
    pub(super) pending_gji_composition_reset: Cell<bool>,
    /// `send_romaji_as_tsf` / `send_romaji_batched` の `GjiFsm::KeyInput` Response バッファ。
    pending_gji_key_responses: RefCell<Vec<GjiResponse>>,
}

impl TsfWarmupCoordinator {
    pub(crate) fn new() -> Self {
        Self {
            tsf_warmup: RefCell::new(Box::new(GjiFsm::new())),
            pending_tsf: RefCell::new(None),
            current_gji_probe_id: Cell::new(None),
            gji_probe_guard: RefCell::new(None),
            pending_gji_warmup: Cell::new(None),
            pending_gji_composition_reset: Cell::new(false),
            pending_gji_key_responses: RefCell::new(Vec::new()),
        }
    }

    // ── ウォームアップ戦略 ────────────────────────────────────────────────────

    /// 現在の composition_warm フラグを返す（`tsf_warmup` 戦略が SSOT）。
    pub(crate) fn is_warm(&self) -> bool {
        self.tsf_warmup.borrow().is_warm()
    }

    /// GjiFsm が long-cold（≥10s idle）な次の KeyInput か判定する。
    pub(crate) fn is_next_key_long_cold(&self) -> bool {
        self.tsf_warmup.borrow().is_next_key_long_cold()
    }

    /// 検出した IME 種別に応じてウォームアップ戦略を切り替える。
    pub(crate) fn set_active_ime_kind(&self, kind: crate::tsf::observer::ActiveImeKind) {
        use crate::tsf::observer::ActiveImeKind;
        match kind {
            ActiveImeKind::MicrosoftIme => {
                log::info!("[output] Switching warmup strategy → MsImeStrategy (MS-IME detected)");
                *self.tsf_warmup.borrow_mut() = Box::new(crate::tsf::warmup_strategy::MsImeStrategy);
            }
            ActiveImeKind::GoogleJapaneseInput => {
                log::info!("[output] Switching warmup strategy → GjiFsm (GJI detected)");
                *self.tsf_warmup.borrow_mut() = Box::new(GjiFsm::new());
            }
        }
    }

    /// GjiFsm にイベントを送り、Response を返す。
    pub(crate) fn gji_on_event(&self, event: GjiEvent) -> GjiResponse {
        self.tsf_warmup.borrow_mut().on_gji_event(event)
    }

    /// GjiFsm に LongIdle タイムアウトを送り、Response を返す。
    pub(crate) fn gji_on_long_idle(&self) -> GjiResponse {
        self.tsf_warmup.borrow_mut().on_gji_long_idle()
    }

    /// `OnComposing` 状態の現在 epoch を返す。それ以外の状態では `None`。
    pub(crate) fn gji_current_composition_epoch(&self) -> Option<FocusEpoch> {
        self.tsf_warmup.borrow().gji_current_composition_epoch()
    }

    /// `Authorized` 状態の `ProbeParams` を返す。それ以外なら `default()`。
    pub(crate) fn current_probe_params(&self) -> ProbeParams {
        self.tsf_warmup
            .borrow()
            .current_probe_params()
            .unwrap_or_default()
    }

    // ── probe_id ──────────────────────────────────────────────────────────

    /// `GjiAction::StartProbe` を受信したとき probe_id を記録する。
    pub(crate) fn store_probe_id(&self, id: ProbeId) {
        self.current_gji_probe_id.set(Some(id));
    }

    /// 現在の GJI probe_id を返す（確認用、消費しない）。
    pub(crate) fn current_probe_id(&self) -> Option<ProbeId> {
        self.current_gji_probe_id.get()
    }

    /// 現在の GJI probe_id を取り出してクリアする。
    pub(crate) fn take_probe_id(&self) -> Option<ProbeId> {
        self.current_gji_probe_id.take()
    }

    // ── OUTPUT_GATE ガード ─────────────────────────────────────────────────

    /// GJI probe の OUTPUT_GATE ガードを開始する。
    pub(crate) fn begin_probe_guard(&self) {
        *self.gji_probe_guard.borrow_mut() = Some(OutputActiveGuard::begin());
    }

    /// GJI probe の OUTPUT_GATE ガードを解放する。
    pub(crate) fn end_probe_guard(&self) {
        *self.gji_probe_guard.borrow_mut() = None;
    }

    // ── warmup result 橋渡し ───────────────────────────────────────────────

    /// `pending_gji_warmup` をセットする（`ProbeIo::store_gji_warmup_result` 用）。
    pub(crate) fn store_warmup_result(&self, result: WarmupResult) {
        self.pending_gji_warmup.set(Some(result));
    }

    /// `pending_gji_warmup` を取り出す（1回限り）。
    pub(crate) fn take_warmup_result(&self) -> Option<WarmupResult> {
        self.pending_gji_warmup.take()
    }

    // ── composition reset 橋渡し ───────────────────────────────────────────

    /// `pending_gji_composition_reset` をセットする（`ProbeIo::mark_cold_raw_tsf` 用）。
    pub(crate) fn mark_composition_reset(&self) {
        self.pending_gji_composition_reset.set(true);
    }

    /// `pending_gji_composition_reset` を取り出してクリアする。
    pub(crate) fn take_composition_reset(&self) -> bool {
        self.pending_gji_composition_reset.take()
    }

    // ── KeyInput Response バッファ ─────────────────────────────────────────

    /// `GjiFsm::KeyInput` Response を蓄積する。
    pub(crate) fn push_key_response(&self, resp: GjiResponse) {
        self.pending_gji_key_responses.borrow_mut().push(resp);
    }

    /// 蓄積した KeyInput Response を全件取り出す。
    pub(crate) fn drain_key_responses(&self) -> Vec<GjiResponse> {
        std::mem::take(&mut *self.pending_gji_key_responses.borrow_mut())
    }

    // ── 保留 TSF プローブ FSM ───────────────────────────────────────────────

    /// probe を `pending_tsf` にセットする。既存 probe があれば上書きして warn を出す。
    pub(crate) fn install_pending_tsf(&self, machine: Box<dyn TickableFsm>) {
        let mut slot = self.pending_tsf.borrow_mut();
        if slot.is_some() {
            log::warn!(
                "[tsf-probe] overwriting in-flight probe with new probe cold={}",
                machine.cold_seq_hint()
            );
        }
        *slot = Some(machine);
    }

    /// `pending_tsf` を取り出す（`step_probe` の1ステップ処理用）。
    pub(crate) fn take_pending_tsf(&self) -> Option<Box<dyn TickableFsm>> {
        self.pending_tsf.borrow_mut().take()
    }

    /// `pending_tsf` に machine を戻す（`step_probe` の Continue 用）。
    pub(crate) fn restore_pending_tsf(&self, machine: Box<dyn TickableFsm>) {
        *self.pending_tsf.borrow_mut() = Some(machine);
    }

    /// `pending_tsf` をクリアする（`CancelProbe` 用）。
    pub(crate) fn clear_pending_tsf(&self) {
        *self.pending_tsf.borrow_mut() = None;
    }

    /// probe が実行中かどうかを返す。
    pub(crate) fn has_pending_tsf(&self) -> bool {
        self.pending_tsf.borrow().is_some()
    }

    /// Chrome/LiteralDetect/GjiWarmup probe が実行中なら継続タイマー命令を返す。
    pub(crate) fn pending_tsf_timer(&self) -> Option<TimerCommand> {
        self.has_pending_tsf().then_some(TimerCommand::Continue {
            id: crate::TIMER_TSF_PROBE,
            delay: Duration::from_millis(10),
        })
    }

    /// sacr-warmup probe に StartComposition が観測されたことを通知する。
    pub(crate) fn notify_probe_start_composition(&self) {
        if let Some(machine) = self.pending_tsf.borrow_mut().as_mut() {
            machine.notify_start_composition();
        }
    }

    /// probe 進行中なら渡された VK 列を deferred_vks に追記し true を返す。
    pub(crate) fn defer_vks_if_in_flight(&self, vks: &[(VkCode, bool)]) -> bool {
        self.pending_tsf
            .borrow_mut()
            .as_mut()
            .is_some_and(|machine| {
                for &(vk, needs_shift) in vks {
                    machine.push_deferred(vk, needs_shift);
                }
                true
            })
    }
}
