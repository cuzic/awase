//! VK_IME_OFF→VK_IME_ON ウォームアップ FSM。
//!
//! [`ImeOffOnWarmupFsm`] は VK_A+BS の代わりに VK_IME_OFF→VK_IME_ON を送信し、
//! GJI の WriteTransferCount 上昇を検出してから実ローマ字を再送する。
//!
//! ## 名前が Coro でなく Fsm である理由
//!
//! 実体は「10ms ごとに elapsed_ms を加算し gji_write_bytes 上昇 or タイムアウトを待つ」
//! 単一の線形カウンタであり、複数フェーズを跨ぐ制御フロー（`StepCoro` が解決する問題）を
//! 持たない。そのため coroutine 化せず、手書き tick カウンタ FSM のまま保持する
//! （ADR-053 の StepCoro 化基準: 分岐が線形シーケンスなら StepCoro、単純カウンタなら FSM）。
//!
//! ## vim 互換性
//!
//! VK_A は vim normal mode で append に割り当てられているため cold 時にアプリへ届くと誤動作する。
//! VK_IME_OFF/ON はアプリ定義のバインドを持たない IME 制御キーのため安全に送信できる。
//!
//! ## 検出方法
//!
//! VK_IME_OFF→VK_IME_ON の Off→On 状態遷移が GJI に WriteTransferCount を増加させる（+46B 実測）。
//! `gji_write_bytes()` ポーリングで上昇を検出したら `confirmed_warm=true` で再送。
//! `TIMEOUT_MS` 内に上昇しない場合は `confirmed_warm=false`（F2 prepend フォールバック）で再送。

use crate::tsf::probe_bridge::OutputActiveGuard;
use crate::tsf::warmup::probe_fsm::{
    ProbeAction, SacrificialResend, TransmitTarget, TsfEnvSnapshot,
};
use crate::tsf::warmup::tickable_fsm::TickableFsm;

/// VK_IME_OFF→ON 送信後に GJI write が観測されるまで待つミリ秒数。
///
/// 実測: VK_IME_OFF→ON 送信から +46B 検出まで ~30ms。余裕を持って 200ms。
const TIMEOUT_MS: u64 = 200;

pub(crate) struct ImeOffOnWarmupFsm {
    cold_seq: u32,
    romaji: String,
    target: TransmitTarget,
    baseline_bytes: u64,
    elapsed_ms: u64,
    _guard: OutputActiveGuard,
}

impl ImeOffOnWarmupFsm {
    pub(crate) fn new(
        cold_seq: u32,
        romaji: String,
        target: TransmitTarget,
        baseline_bytes: u64,
    ) -> Self {
        log::debug!(
            "[ime-offon-warmup] cold={cold_seq} 開始: romaji={romaji:?} \
             target={target:?} baseline_bytes={baseline_bytes}"
        );
        Self {
            cold_seq,
            romaji,
            target,
            baseline_bytes,
            elapsed_ms: 0,
            _guard: OutputActiveGuard::begin(),
        }
    }

    fn tick_inner(&mut self) -> Vec<ProbeAction> {
        self.elapsed_ms += 10;
        let current = crate::tsf::observer::gji_write_bytes();
        let gji_wrote = current > self.baseline_bytes;
        let timed_out = self.elapsed_ms >= TIMEOUT_MS;

        if !gji_wrote && !timed_out {
            return vec![];
        }

        let delta = current.saturating_sub(self.baseline_bytes);
        log::debug!(
            "[ime-offon-warmup] cold={} gji_wrote={gji_wrote} timed_out={timed_out} \
             elapsed={}ms delta=+{delta}B",
            self.cold_seq,
            self.elapsed_ms,
        );

        vec![
            ProbeAction::SacrificialResend(SacrificialResend {
                cold_seq: self.cold_seq,
                romaji: std::mem::take(&mut self.romaji),
                target: self.target,
                confirmed_warm: gji_wrote,
                skip_cleanup_bs: true,
            }),
            ProbeAction::Done,
        ]
    }
}

impl TickableFsm for ImeOffOnWarmupFsm {
    fn tick(&mut self, _env: &TsfEnvSnapshot) -> Vec<ProbeAction> {
        self.tick_inner()
    }

    fn cold_seq_hint(&self) -> u32 {
        self.cold_seq
    }
}
