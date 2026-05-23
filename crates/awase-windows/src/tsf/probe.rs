//! judgement 層 — 観測データから TSF 状態を推測する。
//!
//! - `TsfReadinessProbe`: GJI I/O 静止を待って「composition が受け付け可能か」を判定
//! - `CompositionState`: warm/cold epoch 管理（フォーカス変更で自動無効化）
//! - `LiteralDetector`: 文字送信後に GJI 候補ウィンドウ変化を監視して
//!   「composition が成功したか / raw literal が出力されたか」を判定

use std::sync::atomic::Ordering;

use crate::tsf::observer::TSF_OBS;
use crate::tuning::{GJI_IDLE_MS, POST_IDLE_MARGIN_MS};

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
    /// GJI 静止を最初に検出した時刻（POST_IDLE_MARGIN 用）。0 = 未検出。
    settled_at_ms: std::cell::Cell<u64>,
}

impl TsfReadinessProbe {
    pub const fn new(warmup_sent_ms: u64, cold_n: u32, min_ms: u64) -> Self {
        Self {
            warmup_sent_ms,
            cold_n,
            min_ms,
            settled_at_ms: std::cell::Cell::new(0),
        }
    }

    /// タイマーポーリング用ノンブロッキング判定。
    ///
    /// `true` = 送信可能（GJI 静止 or タイムアウト）、`false` = まだ待機中。
    /// TIMER_TSF_PROBE ハンドラから 10ms ごとに呼ぶ。
    pub fn check_now(&self, total_max_ms: u64) -> bool {
        let now = crate::hook::current_tick_ms();
        let max_deadline = self.warmup_sent_ms.saturating_add(total_max_ms);
        let min_deadline = self.warmup_sent_ms.saturating_add(self.min_ms);

        if !TSF_OBS.gji_monitor_ok.load(Ordering::Relaxed) {
            return now >= max_deadline;
        }
        if now < min_deadline {
            return false;
        }
        if now >= max_deadline {
            return true;
        }
        let gji_io = TSF_OBS.gji_last_io_ms.load(Ordering::Relaxed);
        let found_io_after_warmup = gji_io >= self.warmup_sent_ms;
        if found_io_after_warmup {
            let gji_idle = now.saturating_sub(gji_io);
            if gji_idle >= GJI_IDLE_MS {
                let settled_at = self.settled_at_ms.get();
                if settled_at == 0 {
                    self.settled_at_ms.set(now);
                    return false;
                }
                let margin = max_deadline.saturating_sub(now).min(POST_IDLE_MARGIN_MS);
                let since_settled = now.saturating_sub(settled_at);
                return since_settled >= margin;
            } else {
                self.settled_at_ms.set(0); // GJI が再びアクティブになった
            }
        }
        false
    }

    /// GJI が settled になるまでポーリング待機する。
    ///
    /// `block_on` ではなく `std::thread::sleep` を使うため、ネストされたメッセージループを
    /// 起動しない。`with_app` 内からの呼び出しでも WinEvent 再入が発生しない。
    ///
    /// 主にテストコードおよびフォールバックパスで使用する。
    /// 本番の TSF プローブは TIMER_TSF_PROBE + `check_now` を使うこと。
    pub fn wait_until_ready(&self, total_max_ms: u64) {
        const POLL_MS: u64 = 10;
        let cold_n = self.cold_n;
        let call_ms = crate::hook::current_tick_ms();
        loop {
            if self.check_now(total_max_ms) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(POLL_MS));
        }
        let total = crate::hook::current_tick_ms().saturating_sub(call_ms);
        log::debug!("[tsf-probe] cold={cold_n} wait_until_ready done, waited {total}ms");
    }
}

// ── WarmEpoch ──

/// warmup epoch・送信タイミング・cold-start 回数を管理するサブ構造体。
///
/// フォーカス epoch とウォーム epoch の組み合わせにより、
/// フォーカス変更後の古いウォーム状態を自動無効化する。
#[derive(Debug)]
pub struct WarmEpoch {
    /// ウォーム状態の epoch（0 = cold）
    composition_warm_epoch: std::cell::Cell<u32>,
    /// フォーカスウィンドウの epoch（変更のたびにインクリメント）
    focus_epoch: std::cell::Cell<u32>,
    /// 最後の `send_keys` 完了時刻（ms）
    last_send_ms: std::cell::Cell<u64>,
    /// Cold-start 発生回数カウンタ
    cold_start_count: std::cell::Cell<u32>,
    /// NativeF2Consumed 時に即送信した eager warmup F2 の送信時刻（ms）。0 = 未送信
    eager_warmup_sent_ms: std::cell::Cell<u64>,
}

impl WarmEpoch {
    pub fn new() -> Self {
        Self {
            composition_warm_epoch: std::cell::Cell::new(0),
            focus_epoch: std::cell::Cell::new(1),
            last_send_ms: std::cell::Cell::new(0),
            cold_start_count: std::cell::Cell::new(0),
            eager_warmup_sent_ms: std::cell::Cell::new(0),
        }
    }

    /// `composition_warm_epoch` のみ 0 にリセットする（`eager_warmup_sent_ms` は保持）。
    pub fn suppress_warm_epoch(&self) {
        self.composition_warm_epoch.set(0);
        log::debug!("[composition] warm epoch suppressed (eager_warmup_sent_ms preserved)");
    }

    /// ウォーム状態にマークする。
    pub fn mark_warm(&self) {
        let epoch = self.focus_epoch.get();
        self.composition_warm_epoch.set(epoch);
    }

    /// コールド状態にマークする（epoch と eager_warmup_sent_ms をリセット）。
    pub fn mark_cold(&self) {
        self.composition_warm_epoch.set(0);
        self.eager_warmup_sent_ms.set(0);
    }

    /// 現在 warm かどうかを返す。
    ///
    /// `focus_epoch` が変化していれば前ウィンドウのウォーム状態は自動無効化される。
    pub fn is_warm(&self) -> bool {
        let epoch = self.focus_epoch.get();
        self.composition_warm_epoch.get() == epoch && epoch != 0
    }

    /// フォーカス変更時に epoch をインクリメントし、warm/eager 状態をリセットする。
    ///
    /// 戻り値: 新しい focus_epoch
    pub fn on_focus_changed(&self) -> u32 {
        let new_epoch = self.focus_epoch.get().wrapping_add(1).max(1);
        self.focus_epoch.set(new_epoch);
        self.composition_warm_epoch.set(0);
        self.eager_warmup_sent_ms.set(0);
        new_epoch
    }

    /// 最後の `send_keys` 完了からの経過時間（ms）。
    /// 一度も送信していない場合は `u64::MAX` を返す。
    #[must_use]
    pub fn ms_since_last_send(&self) -> u64 {
        let last = self.last_send_ms.get();
        if last == 0 {
            return u64::MAX;
        }
        crate::hook::current_tick_ms().saturating_sub(last)
    }

    /// `last_send_ms` を現在時刻に更新する。
    pub fn update_last_send_ms(&self) {
        let ms = crate::hook::current_tick_ms();
        log::debug!("[mark-send] last_send_ms={ms}");
        self.last_send_ms.set(ms);
    }

    /// eager warmup F2 を送信した時刻（ms）を返す。0 = 未送信。
    #[must_use]
    pub fn eager_warmup_sent_ms(&self) -> u64 {
        self.eager_warmup_sent_ms.get()
    }

    /// eager warmup F2 の送信時刻をセットする。
    pub fn set_eager_warmup_sent_ms(&self, ms: u64) {
        self.eager_warmup_sent_ms.set(ms);
    }

    /// cold-start 発生回数を返す。
    #[must_use]
    pub fn cold_start_count(&self) -> u32 {
        self.cold_start_count.get()
    }

    /// cold-start 発生回数をインクリメントして新値を返す。
    pub fn increment_cold_start_count(&self) -> u32 {
        let n = self.cold_start_count.get() + 1;
        self.cold_start_count.set(n);
        n
    }
}

impl Default for WarmEpoch {
    fn default() -> Self {
        Self::new()
    }
}

// ── ColdContext ──

/// cold になった理由・idle 時間・連続 recovery 回数を保持するサブ構造体。
#[derive(Debug)]
pub struct ColdContext {
    /// 最後に cold にマークされた理由
    last_cold_reason: std::cell::Cell<crate::output::ColdReason>,
    /// 最後に cold になった時点での idle 時間（ms）
    idle_ms_at_last_cold: std::cell::Cell<u64>,
    /// `RawTsfLiteralRecovery` が連続で発火した回数
    raw_tsf_literal_consecutive_count: std::cell::Cell<u32>,
}

impl ColdContext {
    pub fn new() -> Self {
        Self {
            last_cold_reason: std::cell::Cell::new(crate::output::ColdReason::FocusChange),
            idle_ms_at_last_cold: std::cell::Cell::new(0),
            raw_tsf_literal_consecutive_count: std::cell::Cell::new(0),
        }
    }

    /// cold にマークされた理由と idle 時間を記録する。
    pub fn record_cold(&self, reason: crate::output::ColdReason, idle_ms: u64) {
        self.last_cold_reason.set(reason);
        self.idle_ms_at_last_cold.set(idle_ms);
    }

    /// `RawTsfLiteralRecovery` 連続カウントをインクリメントして新値を返す。
    pub fn increment_consecutive_count(&self) -> u32 {
        let n = self.raw_tsf_literal_consecutive_count.get() + 1;
        self.raw_tsf_literal_consecutive_count.set(n);
        n
    }

    /// `RawTsfLiteralRecovery` 連続カウントをリセットする。
    pub fn reset_consecutive_count(&self) {
        self.raw_tsf_literal_consecutive_count.set(0);
    }

    /// 最後に cold になった時点での idle 時間（ms）を返す。
    #[must_use]
    pub fn idle_ms_at_last_cold(&self) -> u64 {
        self.idle_ms_at_last_cold.get()
    }

    /// `idle_ms_at_last_cold` を更新する。
    pub fn set_idle_ms_at_last_cold(&self, ms: u64) {
        self.idle_ms_at_last_cold.set(ms);
    }

    /// 最後に cold にマークされた理由を返す。
    #[must_use]
    pub fn last_cold_reason(&self) -> crate::output::ColdReason {
        self.last_cold_reason.get()
    }

    /// `RawTsfLiteralRecovery` が連続で発火した回数を返す。
    #[must_use]
    pub fn consecutive_count(&self) -> u32 {
        self.raw_tsf_literal_consecutive_count.get()
    }
}

impl Default for ColdContext {
    fn default() -> Self {
        Self::new()
    }
}

// ── CompositionState ──

/// TSF composition context の warm/cold 状態を管理する。
///
/// `Output` 構造体がこれをフィールドとして保持する。
/// 内部を 3 つの責務別サブ構造体に分割している:
/// - `warm_epoch`: warmup epoch・送信タイミング・cold-start 回数
/// - `cold_ctx`: cold の理由・idle 時間・連続 recovery 回数
/// - `latch`: `apply_ime_open` の直前結果ラッチ（[`crate::tsf::last_apply::ImeApplyLatch`]）
#[derive(Debug)]
pub struct CompositionState {
    /// warmup epoch・送信タイミング・cold-start 回数
    pub warm_epoch: WarmEpoch,
    /// cold の理由・idle 時間・連続 recovery 回数
    pub cold_ctx: ColdContext,
    /// `apply_ime_open` の直前結果ラッチ（KanjiToggleStrategy の shadow_on 用）
    pub latch: crate::tsf::last_apply::ImeApplyLatch,
}

impl CompositionState {
    pub fn new() -> Self {
        Self {
            warm_epoch: WarmEpoch::new(),
            cold_ctx: ColdContext::new(),
            latch: crate::tsf::last_apply::ImeApplyLatch::new(),
        }
    }

    /// `composition_warm_epoch` のみ 0 にリセットする（`eager_warmup_sent_ms` は保持）。
    ///
    /// フォーカス遷移直後の最初のキーで呼ぶ。eager warmup タイムスタンプを消さないことで
    /// non-eager 1500ms パスへの意図しない劣化を防ぐ。
    pub fn suppress_warm_epoch(&self) {
        self.warm_epoch.suppress_warm_epoch();
    }

    /// IME composition context をコールド状態にマークする。
    pub fn mark_composition_cold(&self, reason: crate::output::ColdReason) {
        let idle_ms = self.ms_since_last_send();
        if reason == crate::output::ColdReason::RawTsfLiteralRecovery {
            let n = self.cold_ctx.increment_consecutive_count();
            log::debug!("[composition] marked cold reason={reason:?} idle={idle_ms}ms consecutive={n} → next VK/TSF output will send VK_DBE_HIRAGANA warmup");
        } else {
            self.cold_ctx.reset_consecutive_count();
            log::debug!("[composition] marked cold reason={reason:?} idle={idle_ms}ms → next VK/TSF output will send VK_DBE_HIRAGANA warmup");
        }
        self.warm_epoch.mark_cold();
        self.cold_ctx.record_cold(reason, idle_ms);
    }

    /// IME composition context をウォーム状態にマークする。
    pub fn mark_composition_warm(&self) {
        let epoch = self.warm_epoch.focus_epoch.get();
        log::debug!("[composition] marked warm (epoch={epoch}) → next VK/TSF output will NOT send VK_DBE_HIRAGANA warmup");
        self.warm_epoch.mark_warm();
        self.cold_ctx.reset_consecutive_count();
    }

    /// 現在の composition_warm フラグを返す。
    ///
    /// `focus_epoch` が変化していれば前ウィンドウのウォーム状態は自動無効化される。
    pub fn is_composition_warm(&self) -> bool {
        self.warm_epoch.is_warm()
    }

    /// フォーカスウィンドウが変わったことを通知する。
    pub fn on_focus_changed(&self) {
        let idle_ms = self.ms_since_last_send();
        let new_epoch = self.warm_epoch.on_focus_changed();
        self.cold_ctx.set_idle_ms_at_last_cold(idle_ms);
        self.cold_ctx.reset_consecutive_count();
        self.latch.invalidate();
        log::debug!("[composition] focus changed → epoch={new_epoch}, marked cold");
    }

    /// `apply_ime_open` 完了後にラッチを更新する。
    ///
    /// `KanjiToggleStrategy` が次の `apply_ime_open` で shadow_on を読むために使う。
    pub fn set_ime_apply_latch(&self, open: bool) {
        self.latch.set(open);
    }

    /// 最後の `send_keys` 完了からの経過時間（ms）。
    /// 一度も送信していない場合は `u64::MAX` を返す（= 永久に in-flight でない）。
    #[must_use]
    pub fn ms_since_last_send(&self) -> u64 {
        self.warm_epoch.ms_since_last_send()
    }

    /// `last_send_ms` を現在時刻に更新する。
    pub fn update_last_send_ms(&self) {
        self.warm_epoch.update_last_send_ms();
    }

    /// eager warmup F2 を送信した時刻（ms）を返す。0 = 未送信。
    #[must_use]
    pub fn eager_warmup_sent_ms(&self) -> u64 {
        self.warm_epoch.eager_warmup_sent_ms()
    }

    /// eager warmup F2 の送信時刻をセットする。
    pub fn set_eager_warmup_sent_ms(&self, ms: u64) {
        self.warm_epoch.set_eager_warmup_sent_ms(ms);
    }

    /// 最後に cold になった時点での idle 時間（ms）を返す。
    #[must_use]
    pub fn idle_ms_at_last_cold(&self) -> u64 {
        self.cold_ctx.idle_ms_at_last_cold()
    }

    /// cold-start 発生回数を返す。
    #[must_use]
    pub fn cold_start_count(&self) -> u32 {
        self.warm_epoch.cold_start_count()
    }

    /// cold-start 発生回数をインクリメントして新値を返す。
    pub fn increment_cold_start_count(&self) -> u32 {
        self.warm_epoch.increment_cold_start_count()
    }

    /// `apply_ime_open` が最後に設定した値を返す。
    /// フォーカス変更直後など未設定の場合は `false` を返す。
    #[must_use]
    pub fn shadow_ime_on(&self) -> bool {
        self.latch.get_or(false)
    }

    /// 最後に cold にマークされた理由を返す。
    #[must_use]
    pub fn last_cold_reason(&self) -> crate::output::ColdReason {
        self.cold_ctx.last_cold_reason()
    }

    /// `RawTsfLiteralRecovery` が連続で発火した回数を返す。
    #[must_use]
    pub fn consecutive_count(&self) -> u32 {
        self.cold_ctx.consecutive_count()
    }
}

// ── LiteralDetector ──

/// raw-TSF-literal IO 検出のセッション ID。
///
/// `raw_tsf_literal_io_or_timeout_async` の polling ループが、新しい検出セッションの
/// 開始を検知して即座に抜けるために使う。SHOW 版は `race_with_timeout` を使うため不要。
static RAW_TSF_LITERAL_IO_SESSION: std::sync::atomic::AtomicU32 =
    std::sync::atomic::AtomicU32::new(0);

/// `send_romaji_as_tsf` が文字を送信した直後に生成し、
/// GJI 候補ウィンドウの変化を監視して composition が成功したか判定する検出器。
#[derive(Debug)]
pub struct LiteralDetector {
    /// 送信前の GJI 候補ウィンドウ SHOW シーケンス番号
    gji_show_baseline: u32,
    /// 送信前の GJI I/O タイムスタンプ
    io_baseline: u64,
    /// 送信前に候補ウィンドウが表示中だったか
    was_candidate_visible: bool,
}

/// raw-TSF-literal 検出結果。
#[derive(Debug)]
pub enum DetectionResult {
    /// composition 成功（IME が文字を受け付けた）
    CompositionConfirmed,
    /// raw TSF literal 疑い（IME をバイパスして ASCII が出力された）
    SuspectedLiteral,
}

impl LiteralDetector {
    /// 現在の観測値からベースラインを取得して `LiteralDetector` を生成する。
    ///
    /// ローマ字送信直前に呼ぶこと。
    pub fn new() -> Self {
        use std::sync::atomic::Ordering::Relaxed;
        Self {
            gji_show_baseline: TSF_OBS.gji_candidate_show_seq.load(Relaxed),
            io_baseline: TSF_OBS.gji_last_io_ms.load(Relaxed),
            was_candidate_visible: TSF_OBS.gji_candidate_visible.load(Relaxed),
        }
    }

    /// タイマーポーリング用ノンブロッキング判定。
    ///
    /// `Some` = 判定確定、`None` = まだ待機中。
    /// TIMER_TSF_PROBE ハンドラから 10ms ごとに呼ぶ。
    pub fn check_now(&self, deadline_ms: u64) -> Option<DetectionResult> {
        use std::sync::atomic::Ordering::Relaxed;
        let now = crate::hook::current_tick_ms();
        let confirmed = if self.was_candidate_visible {
            TSF_OBS.gji_last_io_ms.load(Relaxed) != self.io_baseline
        } else {
            TSF_OBS.gji_candidate_show_seq.load(Relaxed) != self.gji_show_baseline
        };
        if confirmed {
            Some(DetectionResult::CompositionConfirmed)
        } else if now >= deadline_ms {
            Some(DetectionResult::SuspectedLiteral)
        } else {
            None
        }
    }

    /// ローマ字送信後に呼び、composition 成功 / raw literal 疑いを判定する。
    ///
    /// - 候補ウィンドウが非表示だった場合: SHOW イベントで composition を確認
    /// - 候補ウィンドウが表示済みだった場合: GJI I/O 変化で composition を確認
    pub fn detect(&self, timeout_ms: u64) -> DetectionResult {
        let timeout_u32 = u32::try_from(timeout_ms).unwrap_or(u32::MAX);
        let confirmed = if self.was_candidate_visible {
            wait_for_raw_tsf_literal_io(self.io_baseline, timeout_u32)
        } else {
            wait_for_raw_tsf_literal_show(self.gji_show_baseline, timeout_u32)
        };
        if confirmed {
            DetectionResult::CompositionConfirmed
        } else {
            DetectionResult::SuspectedLiteral
        }
    }
}

/// cold start 後のローマ字送信で GJI candidate window が表示されるのを event-driven に待つ。
///
/// - `show_baseline`: 送信直前の `TSF_OBS.gji_candidate_show_seq` の値
/// - `timeout_ms`: タイムアウト (ms)
/// - 戻り値: `true` = SHOW 検出（composition 成功）、`false` = timeout（raw TSF literal 疑い）
pub(crate) fn wait_for_raw_tsf_literal_show(show_baseline: u32, timeout_ms: u32) -> bool {
    win32_async::block_on(raw_tsf_literal_show_or_timeout_async(show_baseline, timeout_ms))
}

async fn raw_tsf_literal_show_or_timeout_async(show_baseline: u32, timeout_ms: u32) -> bool {
    use std::sync::atomic::Ordering::Relaxed;

    // TSF_OBS.composition_probe_seq は observer が GJI SHOW イベントを受け取るたびにインクリメントし
    // notify_all() を呼ぶ。race_with_timeout でタイムアウトと競走させることで、
    // orphan タイムアウトタスクや session ガードが不要になる。
    let probe_baseline = crate::tsf::observer::TSF_OBS.composition_probe_seq.load(Relaxed);
    let got_event = win32_async::race_with_timeout(
        timeout_ms,
        win32_async::AtomicWatcher::new(&crate::tsf::observer::TSF_OBS.composition_probe_seq, probe_baseline),
    )
    .await;

    // イベントが来た場合のみ SHOW シーケンスが進んでいるか確認する
    got_event.is_some() && TSF_OBS.gji_candidate_show_seq.load(Relaxed) != show_baseline
}

/// GJI candidate window がすでに表示中の場合の raw TSF literal 検出。
/// SHOW イベントは来ないため GJI I/O 変化（TSF_OBS.gji_last_io_ms）でポーリングする。
///
/// - `io_baseline`: 送信直前の `TSF_OBS.gji_last_io_ms` の値
/// - `timeout_ms`: タイムアウト (ms)
/// - 戻り値: `true` = I/O 変化検出（composition 成功）、`false` = timeout（raw TSF literal 疑い）
pub(crate) fn wait_for_raw_tsf_literal_io(io_baseline: u64, timeout_ms: u32) -> bool {
    win32_async::block_on(raw_tsf_literal_io_or_timeout_async(io_baseline, timeout_ms))
}

/// [`wait_for_raw_tsf_literal_io`] の非同期実装。`TSF_OBS.gji_last_io_ms` をポーリングする。
///
/// GJI I/O モニタースレッドは 10ms 間隔でサンプリングするため、
/// ポーリング間隔は 15ms に設定し、I/O 変化を確実に捕捉する。
async fn raw_tsf_literal_io_or_timeout_async(io_baseline: u64, timeout_ms: u32) -> bool {
    use std::sync::atomic::Ordering::Relaxed;
    const POLL_MS: u32 = 15;

    let session = RAW_TSF_LITERAL_IO_SESSION.fetch_add(1, Relaxed) + 1;
    let deadline = crate::hook::current_tick_ms() + u64::from(timeout_ms);

    loop {
        if RAW_TSF_LITERAL_IO_SESSION.load(Relaxed) != session {
            return false;
        }
        let io_now = TSF_OBS.gji_last_io_ms.load(Relaxed);
        if io_now != io_baseline {
            return true;
        }
        let now = crate::hook::current_tick_ms();
        if now >= deadline {
            return false;
        }
        let remaining = u32::try_from(deadline.saturating_sub(now)).unwrap_or(u32::MAX);
        win32_async::sleep_ms(remaining.min(POLL_MS)).await;
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
        TSF_OBS.gji_monitor_ok.store(false, SeqCst);

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

        TSF_OBS.gji_monitor_ok.store(true, SeqCst);
        TSF_OBS.gji_last_io_ms.store(io_ms, SeqCst);

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
        TSF_OBS.gji_monitor_ok.store(true, SeqCst);
        TSF_OBS.gji_last_io_ms.store(now_ms.saturating_sub(200), SeqCst); // 200ms 前に I/O（warmup 前）

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
        TSF_OBS.gji_monitor_ok.store(true, SeqCst);
        TSF_OBS.gji_last_io_ms.store(now_ms.saturating_sub(5_000), SeqCst);

        let start = Instant::now();
        let probe = TsfReadinessProbe::new(now_ms, 0, 0); // min_ms=0
        probe.wait_until_ready(120);

        let elapsed = start.elapsed().as_millis();
        assert!(elapsed >= 80, "should timeout at ~120ms, got {elapsed}ms");
        assert!(elapsed < 500, "exceeded max by too much: {elapsed}ms");
    }
}
