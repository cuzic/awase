//! TSF composition context readiness の観測システム。
//!
//! ## 設計
//!
//! `ImeObservations` が「IME ON/OFF」を複数ソースから観測するのと同様に、
//! `TsfObservations` は「TSF composition context が ready か」を複数ソースで観測する。
//!
//! ## 観測ソース
//!
//! 1. `GjiMonitor` — GetProcessIoCounters ベースの I/O 静止検出
//!    - GJI Converter プロセスの累積 I/O カウンタを 10ms ごとに監視
//!    - 静止（80ms 変化なし）で初期化完了と推定
//!
//! 2. `SessionIpcMonitor` — GJI ディレクトリ内 .ipc ファイルの atime/mtime 監視
//!    - `%LocalAppDataLow%\Google\Google Japanese Input\*.ipc`
//!      (session.ipc, renderer.*.ipc など)
//!    - TSF が GJI に session request を送ると atime が更新される直接シグナル
//!    - GetFileTime(LastAccessTime + LastWriteTime) を 10ms ごとにポーリング
//!    - GJI ディレクトリは SHGetKnownFolderPath(FOLDERID_LocalAppDataLow) で特定
//!
//! 3. 時間ベース（フォールバック）
//!    - 両モニターが利用不可の場合の固定 sleep
//!
//! ## 使い方
//!
//! ```text
//! // 起動時
//! tsf_observations::start_monitor_thread();
//!
//! // send_romaji_as_tsf 内
//! let probe = TsfReadinessProbe::new(warmup_sent_ms, cold_reason);
//! probe.wait_until_ready(timeout_ms); // GJI 静止を待つ or フォールバック
//! // → romaji 送信
//! ```

use std::sync::atomic::Ordering;

// GjiMonitor・start_monitor_thread および OBS_GJI_* グローバルは tsf/observer.rs に移動。
// 後方互換のため re-export する。
pub use crate::tsf::observer::{OBS_GJI_LAST_IO_MS, OBS_GJI_MONITOR_OK, start_monitor_thread};

// ── TsfReadinessProbe ──

/// TSF composition readiness を観測し「ready になるまで待つ」プローブ。
///
/// `send_romaji_as_tsf` の固定 sleep を置き換える。
///
/// ## 判断ロジック（2フェーズ）
///
/// ### Phase 1 — 必須最小待機 (`min_ms` from `warmup_sent_ms`)
///
/// VK_IME_ON 送信直後は GJI の I/O がまだ始まっていない可能性がある。
/// `min_ms` 経過するまでは GJI I/O の観測値を信頼せず待機する。
///
/// ### Phase 2 — GJI I/O 静止監視
///
/// - `last_io >= warmup_ms`（warmup 後に GJI I/O が発生した）かつ
///   80ms 静止 → settled → `POST_IDLE_MARGIN_MS` 待機後に送信
/// - `last_io < warmup_ms`（warmup 後に I/O なし）→ GJI は既に正常状態と推定、
///   max_deadline 到達まで待機継続（万が一 I/O が来れば settled 検出に切り替え）
/// - `now >= max_deadline` → タイムアウト（フォールバック）
///
/// ## `min_ms` の目安（ColdReason 別）
///
/// | 状況 | min_ms |
/// |---|---|
/// | FocusChange / SetOpenTrue / NativeF2Consumed | 300ms |
/// | SessionExpired | 200ms |
/// | PassthroughConfirmKey / ReinjectConfirmKey | 50ms |
/// | SymbolVkSent | 30ms |
/// | その他 | 100ms |
#[derive(Debug)]
pub struct TsfReadinessProbe {
    /// VK_IME_ON を送信した時刻 (GetTickCount64 ms)。
    pub warmup_sent_ms: u64,
    /// ログ相関用 cold-start シーケンス番号。
    pub cold_n: u32,
    /// VK_IME_ON 送信から最低この ms が経過するまで I/O 観測を信頼しない。
    pub min_ms: u64,
}

impl TsfReadinessProbe {
    pub const fn new(warmup_sent_ms: u64, cold_n: u32, min_ms: u64) -> Self {
        Self { warmup_sent_ms, cold_n, min_ms }
    }

    /// GJI が settled になるまで待機する。
    ///
    /// - `total_max_ms`: `warmup_sent_ms` からの最大許容待機時間（タイムアウト）。
    ///   呼び出し時点での残り時間ではなく、VK_IME_ON 送信からの合計予算。
    ///
    /// 内部で `win32_async::block_on` を呼び、メッセージループを動かしながら待機する。
    /// `std::thread::sleep` を使わないため、待機中も WinEvent（OBJ_NAMECHANGE 等）が処理される。
    pub fn wait_until_ready(&self, total_max_ms: u64) {
        use std::sync::atomic::Ordering::Relaxed;
        let _guard = crate::ProbeGuard;
        // ネストしたメッセージループ中にキーフックが再入しないようガード
        crate::PROBE_ACTIVE.store(true, Relaxed);
        win32_async::block_on(self.wait_until_ready_async(total_max_ms));
        // drain はここでは呼ばない。呼び出し元（send_romaji_batched / send_romaji_as_tsf）が
        // バッチ送信・mark_composition_warm 完了後に post_drain_probe_queue を呼ぶ。
        // ここで drain すると block_on のネストされたメッセージループ中に再配送が走り、
        // 後続キー（ん等）が composition cold のまま send_romaji_as_tsf → 再プローブ → 二重入力を起こす。
    }

    /// [`wait_until_ready`] の非同期実装。`sleep_ms` を使って待機し、
    /// メッセージループをブロックしない。
    async fn wait_until_ready_async(&self, total_max_ms: u64) {
        /// warmup 後の GJI I/O がこの ms 静止したら settled
        const GJI_IDLE_MS: u64 = 80;
        /// settled 確認後の追加余裕 (ms)
        const POST_IDLE_MARGIN_MS: u64 = 30;
        /// ポーリング間隔 (ms)
        const POLL_MS: u32 = 10;

        let cold_n = self.cold_n;
        let warmup_ms = self.warmup_sent_ms;
        let call_ms = crate::hook::current_tick_ms();
        let min_deadline = warmup_ms.saturating_add(self.min_ms);
        let max_deadline = warmup_ms.saturating_add(total_max_ms);

        if !OBS_GJI_MONITOR_OK.load(Ordering::Relaxed) {
            // GJI プロセス監視不可: max_deadline まで非ブロッキング sleep
            let remaining = max_deadline.saturating_sub(crate::hook::current_tick_ms());
            log::debug!(
                "[tsf-probe] cold={cold_n} fallback fixed sleep {remaining}ms (GJI monitor unavailable)"
            );
            if remaining > 0 {
                win32_async::sleep_ms(u32::try_from(remaining).unwrap_or(u32::MAX)).await;
            }
            let total = crate::hook::current_tick_ms().saturating_sub(call_ms);
            log::debug!("[tsf-probe] cold={cold_n} done (fallback), waited {total}ms");
            return;
        }

        // Phase 1: min_deadline まで無条件待機（I/O 観測は信頼しない）
        let phase1_wait = min_deadline.saturating_sub(crate::hook::current_tick_ms());
        if phase1_wait > 0 {
            log::debug!("[tsf-probe] cold={cold_n} phase1 min wait {phase1_wait}ms");
            win32_async::sleep_ms(u32::try_from(phase1_wait).unwrap_or(u32::MAX)).await;
        }

        // Phase 2: GJI I/O 静止監視
        let p2_start = crate::hook::current_tick_ms();
        let gji_io_at_p2 = OBS_GJI_LAST_IO_MS.load(Ordering::Relaxed);
        let io_after_warmup_at_start = gji_io_at_p2 >= warmup_ms;
        log::debug!(
            "[tsf-probe] cold={cold_n} phase2 polling \
             (max_remaining={}ms, gji_io_idle={}ms, io_after_warmup={io_after_warmup_at_start})",
            max_deadline.saturating_sub(p2_start),
            p2_start.saturating_sub(gji_io_at_p2),
        );

        let mut found_io_after_warmup = io_after_warmup_at_start;

        loop {
            let now = crate::hook::current_tick_ms();
            let gji_io = OBS_GJI_LAST_IO_MS.load(Ordering::Relaxed);

            if gji_io >= warmup_ms {
                found_io_after_warmup = true;
            }

            if now >= max_deadline {
                log::debug!(
                    "[tsf-probe] cold={cold_n} timeout \
                     (warmup+{}ms, gji_io_idle={}ms, io_after_warmup={found_io_after_warmup})",
                    now.saturating_sub(warmup_ms),
                    now.saturating_sub(gji_io),
                );
                break;
            }

            if found_io_after_warmup {
                let gji_idle = now.saturating_sub(gji_io);
                if gji_idle >= GJI_IDLE_MS {
                    let elapsed_from_warmup = now.saturating_sub(warmup_ms);
                    let margin = max_deadline.saturating_sub(now).min(POST_IDLE_MARGIN_MS);
                    log::debug!(
                        "[tsf-probe] cold={cold_n} GJI settled \
                         (idle={gji_idle}ms) at warmup+{elapsed_from_warmup}ms, +{margin}ms margin"
                    );
                    if margin > 0 {
                        #[allow(clippy::cast_possible_truncation)]
                        win32_async::sleep_ms(margin as u32).await;
                    }
                    break;
                }
            }

            win32_async::sleep_ms(POLL_MS).await;
        }

        let total = crate::hook::current_tick_ms().saturating_sub(call_ms);
        log::debug!("[tsf-probe] cold={cold_n} done, waited {total}ms");
    }
}

#[cfg(test)]
#[cfg(windows)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering::SeqCst;
    use std::time::Instant;

    /// テスト間でグローバルな観測 atomic が競合しないようにシリアライズするロック
    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// GJI モニター不可のとき、total_max_ms ぶん待機して返る（フォールバックパス）
    #[test]
    fn probe_fallback_waits_total_max_ms() {
        let _g = TEST_LOCK.lock().unwrap();
        OBS_GJI_MONITOR_OK.store(false, SeqCst);

        let start = Instant::now();
        let now_ms = crate::hook::current_tick_ms();
        let probe = TsfReadinessProbe::new(now_ms, 0, 0);
        probe.wait_until_ready(100);

        let elapsed = start.elapsed().as_millis();
        // フォールバック: warmup_ms=now, remaining=100ms → sleep_ms(100)
        assert!(elapsed >= 60, "fallback too short: {elapsed}ms");
        assert!(elapsed < 400, "fallback too long: {elapsed}ms");
    }

    /// GJI モニター有効・warmup 後にすでに 80ms+ 静止していれば即 settled
    #[test]
    fn probe_phase2_detects_already_settled() {
        let _g = TEST_LOCK.lock().unwrap();
        let now_ms = crate::hook::current_tick_ms();

        // warmup 200ms 前、GJI 最終 I/O は warmup の 50ms 後（= 150ms 前）
        // → idle = 150ms > 80ms → settled 検出済み
        let warmup_ms = now_ms.saturating_sub(200);
        let io_ms = warmup_ms + 50;

        OBS_GJI_MONITOR_OK.store(true, SeqCst);
        OBS_GJI_LAST_IO_MS.store(io_ms, SeqCst);

        let start = Instant::now();
        let probe = TsfReadinessProbe::new(warmup_ms, 0, 0); // min_ms=0
        probe.wait_until_ready(1_000);

        let elapsed = start.elapsed().as_millis();
        // 即 settled（margin = POST_IDLE_MARGIN_MS = 30ms 以内）
        assert!(elapsed < 150, "should settle quickly (idle>80ms), got {elapsed}ms");
    }

    /// phase1: min_ms が経過するまで probe は I/O 観測を信頼しない
    #[test]
    fn probe_phase1_min_wait_respected() {
        let _g = TEST_LOCK.lock().unwrap();
        let now_ms = crate::hook::current_tick_ms();

        // GJI は settled 状態だが min_ms=80 のため phase1 で 80ms 待機する
        OBS_GJI_MONITOR_OK.store(true, SeqCst);
        OBS_GJI_LAST_IO_MS.store(now_ms.saturating_sub(200), SeqCst); // 200ms 前に I/O（warmup 前）

        let start = Instant::now();
        let probe = TsfReadinessProbe::new(now_ms, 0, 80); // min_ms=80
        probe.wait_until_ready(300);

        let elapsed = start.elapsed().as_millis();
        // min_ms=80 の phase1 wait + phase2 timeout(no io after warmup)=300ms
        // → 最低 60ms 以上はかかる
        assert!(elapsed >= 60, "phase1 min_wait not respected: {elapsed}ms");
    }

    /// warmup 後に GJI I/O が発生しない場合 max_deadline でタイムアウト
    #[test]
    fn probe_phase2_times_out_when_no_io_after_warmup() {
        let _g = TEST_LOCK.lock().unwrap();
        let now_ms = crate::hook::current_tick_ms();

        // GJI I/O は warmup より前 → warmup 後に I/O なし → タイムアウト
        OBS_GJI_MONITOR_OK.store(true, SeqCst);
        OBS_GJI_LAST_IO_MS.store(now_ms.saturating_sub(5_000), SeqCst);

        let start = Instant::now();
        let probe = TsfReadinessProbe::new(now_ms, 0, 0); // min_ms=0
        probe.wait_until_ready(120);

        let elapsed = start.elapsed().as_millis();
        assert!(elapsed >= 80, "should timeout at ~120ms, got {elapsed}ms");
        assert!(elapsed < 500, "exceeded max by too much: {elapsed}ms");
    }
}
