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
use crate::tsf::probe_fsm::DeferredVk;
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
    /// probe 進行中に届いた後続 VK の単一キュー。
    ///
    /// 個々の probe machine（`GjiWarmupCoro` 等）ではなく coordinator が直接所有する。
    /// これにより「最初の tick で握り潰される」「probe が上書きされて drop される」という
    /// 2種類のデータ消失を構造的に防ぐ（probe の生存期間に依存しない単一の書き込み先）。
    pending_deferred: RefCell<Vec<DeferredVk>>,
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
            pending_deferred: RefCell::new(Vec::new()),
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

    /// 現在の戦略が F2 (VK_DBE_HIRAGANA) cold-start probe を必要とするか。
    ///
    /// GJI 戦略（[`GjiFsm`]）なら `true`、MS-IME 戦略（[`MsImeStrategy`]）なら `false`。
    pub(crate) fn needs_f2_probe(&self) -> bool {
        self.tsf_warmup.borrow().needs_f2_probe()
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

    /// probe 進行中なら渡された VK 列を coordinator の deferred キューに追記し true を返す。
    ///
    /// キューは probe machine ではなく coordinator が所有するため、どの machine が
    /// `pending_tsf` に入っているか・何回 tick されたかに関係なく安全に蓄積できる。
    pub(crate) fn defer_vks_if_in_flight(&self, vks: &[(VkCode, bool)]) -> bool {
        if !self.has_pending_tsf() {
            return false;
        }
        self.pending_deferred
            .borrow_mut()
            .extend(vks.iter().map(|&(vk, needs_shift)| DeferredVk { vk, needs_shift }));
        true
    }

    /// deferred キューが空でないかを覗き見る（消費しない）。
    ///
    /// `decide_transmit_plan` の eager path 判定に使う。
    pub(crate) fn has_pending_deferred(&self) -> bool {
        !self.pending_deferred.borrow().is_empty()
    }

    /// deferred キューの中身を取り出してクリアする。
    ///
    /// 実際に romaji を送信する直前（`dispatch_probe_actions`）でのみ呼ぶこと。
    pub(crate) fn take_pending_deferred(&self) -> Vec<DeferredVk> {
        std::mem::take(&mut *self.pending_deferred.borrow_mut())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tsf::probe_fsm::TsfEnvSnapshot;

    /// `TickableFsm` の最小テストダブル。tick 回数を記録するだけで何も yield しない。
    struct StubMachine {
        ticks: u32,
    }

    impl TickableFsm for StubMachine {
        fn tick(&mut self, _env: &TsfEnvSnapshot) -> Vec<crate::tsf::probe_fsm::ProbeAction> {
            self.ticks += 1;
            vec![]
        }
        fn cold_seq_hint(&self) -> u32 {
            0
        }
    }

    #[test]
    fn defer_vks_if_in_flight_returns_false_without_pending_probe() {
        let coord = TsfWarmupCoordinator::new();
        assert!(!coord.defer_vks_if_in_flight(&[(VkCode(0x41), false)]));
        assert!(!coord.has_pending_deferred());
    }

    #[test]
    fn deferred_vks_survive_regardless_of_how_many_times_the_probe_was_ticked() {
        // 元バグの再現条件: 「probe インストール直後・最初の tick が一度も走っていない」
        // 状態で push された deferred VK が、tick 回数に関係なく消えないことを確認する。
        let coord = TsfWarmupCoordinator::new();
        coord.install_pending_tsf(Box::new(StubMachine { ticks: 0 }));

        // まだ一度も tick していない状態で defer する。
        assert!(coord.defer_vks_if_in_flight(&[(VkCode(0x4C), false), (VkCode(0x59), false)]));
        assert!(coord.has_pending_deferred());

        let drained = coord.take_pending_deferred();
        assert_eq!(drained.len(), 2, "tick 前に push した VK が失われてはいけない");
        assert_eq!(drained[0].vk, VkCode(0x4C));
        assert!(!coord.has_pending_deferred(), "take 後はキューが空になる");
    }

    #[test]
    fn deferred_vks_survive_probe_replacement() {
        // 元バグその2: pending_tsf が別 machine に置き換わっても deferred キューは
        // machine 側ではなく coordinator 側にあるため失われない。
        let coord = TsfWarmupCoordinator::new();
        coord.install_pending_tsf(Box::new(StubMachine { ticks: 0 }));
        assert!(coord.defer_vks_if_in_flight(&[(VkCode(0x41), false)]));

        // 別の probe が同じ pending_tsf スロットを上書きする（warn ログのみで許容される操作）。
        coord.install_pending_tsf(Box::new(StubMachine { ticks: 0 }));

        assert!(
            coord.has_pending_deferred(),
            "probe が上書きされても deferred キューは coordinator に残り続ける"
        );
        assert_eq!(coord.take_pending_deferred().len(), 1);
    }
}
