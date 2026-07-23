//! judgement 層 — 観測データから TSF 状態を推測する。
//!
//! - `TsfReadinessProbe`: GJI I/O 静止を待って「composition が受け付け可能か」を判定
//! - `CompositionState`: warm/cold epoch 管理（フォーカス変更で自動無効化）
//! - `LiteralDetector`: 文字送信後に GJI 候補ウィンドウ変化を監視して
//!   「composition が成功したか / raw literal が出力されたか」を判定

use std::sync::atomic::Ordering;

use crate::tsf::observer::{Baseline, TSF_OBS};
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
///   min_ms 経過済みなので即解放
///   （WezTerm 等 F2 に応じて GJI I/O を出さないアプリでは常にこのパス）
/// - `now >= max_deadline` → タイムアウト（フォールバック、通常は到達しない）
///
/// ## `min_ms` の目安（ColdReason 別）
///
/// | 状況 | min_ms |
/// |---|---|
/// | FocusChange / SetOpenTrue / NativeF2Consumed (long_idle) | 300ms |
/// | FocusChange / SetOpenTrue / NativeF2Consumed (short_idle) | 100ms |
/// | SessionExpired | 200ms |
/// | PassthroughConfirmKey / ReinjectConfirmKey | 50ms |
/// | SymbolVkSent | 30ms |
/// | その他 | 100ms |
#[derive(Debug)]
pub struct TsfReadinessProbe {
    /// VK_IME_ON を送信した時刻 (GetTickCount64 ms)。
    pub warmup_sent_ms: u64,
    /// ログ相関用 cold-start シーケンス番号。
    pub cold_seq: u32,
    /// VK_IME_ON 送信から最低この ms が経過するまで I/O 観測を信頼しない。
    pub min_ms: u64,
    /// GJI 静止を最初に検出した時刻（POST_IDLE_MARGIN 用）。0 = 未検出。
    settled_at_ms: std::cell::Cell<u64>,
}

/// [`TsfReadinessProbe::check_now`] が `true` を返したときに合わせて返す観測スナップショット。
#[derive(Debug)]
pub struct GjiProbeOutcome {
    /// `warmup_sent_ms` からの経過時間（ms）
    pub elapsed_ms: u64,
    /// warmup 後に GJI I/O が発生していたか（`gji_last_io_ms >= warmup_sent_ms`）
    pub settled: bool,
    /// GJI モニターが健全か
    pub monitor_healthy: bool,
    /// プローブ完了時点での GJI 無通信時間（`now - gji_last_io_ms`、ms）
    pub gji_idle_ms: u64,
}

impl TsfReadinessProbe {
    #[must_use]
    pub const fn new(warmup_sent_ms: u64, cold_seq: u32, min_ms: u64) -> Self {
        Self {
            warmup_sent_ms,
            cold_seq,
            min_ms,
            settled_at_ms: std::cell::Cell::new(0),
        }
    }

    /// タイマーポーリング用判定。完了時に settle 情報も返す。
    ///
    /// `None` = まだ待機中、`Some(outcome)` = 送信可能。
    /// TIMER_TSF_PROBE ハンドラから 10ms ごとに呼ぶ。
    pub fn check_outcome(&self, total_max_ms: u64) -> Option<GjiProbeOutcome> {
        if !self.check_now(total_max_ms) {
            return None;
        }
        let now = crate::hook::current_tick_ms();
        let monitor_healthy = TSF_OBS.gji_monitor_ok.load(Ordering::Acquire);
        let gji_last_io = TSF_OBS.gji_last_io_ms.load(Ordering::Relaxed);
        let gji_idle_ms = now.saturating_sub(gji_last_io);
        Some(GjiProbeOutcome {
            elapsed_ms: now.saturating_sub(self.warmup_sent_ms),
            settled: gji_last_io >= self.warmup_sent_ms,
            monitor_healthy,
            gji_idle_ms,
        })
    }

    /// タイマーポーリング用ノンブロッキング判定。
    ///
    /// `true` = 送信可能（GJI 静止 or タイムアウト）、`false` = まだ待機中。
    /// TIMER_TSF_PROBE ハンドラから 10ms ごとに呼ぶ。
    ///
    /// 2026-07-19 時点、本番の producer（`cold_warmup.rs::run_start`・
    /// `vk_send.rs` の Chrome cold パス）はいずれも `min_ms=0`・`total_max_ms=0`
    /// を渡すため、`min_deadline`/`max_deadline` は共に `warmup_sent_ms` と等しくなり、
    /// このメソッドは実質「常に最初の呼び出しで `true` を返す」（`GJI_IDLE_MS`/
    /// `settled_at_ms`/`POST_IDLE_MARGIN_MS` の待機ロジックは通らない）。
    /// これは値が静的に固定されている（`ProbeObservations.gji_resumed` や
    /// `DIAG_DISABLE_PROACTIVE_TSF_WARMUP` のような）到達不能ではなく、両呼び出し元が
    /// たまたま実行時に 0 を渡しているだけの状態（コンパイラは保証しない）。
    /// 本メソッド自体は任意の `min_ms`/`total_max_ms` に対して汎用的に正しく動作する
    /// タイミング primitive であり、cold-start 待機時間の調整は本リポジトリで
    /// 何度も出し入れされてきた領域（`tuning-constants.md` の釣り上げ履歴参照）。
    /// 「今 0 が渡っているから」という理由だけでこの分岐自体を削除しないこと。
    pub fn check_now(&self, total_max_ms: u64) -> bool {
        let now = crate::hook::current_tick_ms();
        let max_deadline = self.warmup_sent_ms.saturating_add(total_max_ms);
        let min_deadline = self.warmup_sent_ms.saturating_add(self.min_ms);

        if !TSF_OBS.gji_monitor_ok.load(Ordering::Acquire) {
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
                let since_settled = now.saturating_sub(settled_at);
                return since_settled >= POST_IDLE_MARGIN_MS;
            }
            self.settled_at_ms.set(0); // GJI が再びアクティブになった
        }
        // warmup 後に GJI I/O が来ていない = GJI は既に正常状態 → min_ms 経過済みで即解放
        true
    }
}

// ── WarmEpoch ──

/// warmup epoch・送信タイミング・cold-start 回数を管理するサブ構造体。
///
/// フォーカス epoch とウォーム epoch の組み合わせにより、
/// フォーカス変更後の古いウォーム状態を自動無効化する。
#[derive(Debug)]
pub struct WarmEpoch {
    /// 最後の `send_keys` 完了時刻（ms）
    last_send_ms: std::cell::Cell<u64>,
    /// Cold-start 発生回数カウンタ
    cold_start_count: std::cell::Cell<u32>,
    /// NativeF2Consumed 時に即送信した eager warmup F2 の送信時刻（ms）。0 = 未送信
    eager_warmup_sent_ms: std::cell::Cell<u64>,
    /// KEYEVENTF_UNICODE で文字を送信した時刻（ms）。0 = 未送信 / リセット済み。
    ///
    /// GJI probe が settled=false で完了した後 unicode fallback を使った際にセットされる。
    /// `gji_last_io_ms() > last_unicode_transmit_ms` になれば GJI が応答したとみなし、
    /// 次の warm パスは通常 VK sequential に戻る（`PendingGjiConfirm` 状態の SSOT）。
    last_unicode_transmit_ms: std::cell::Cell<u64>,
}

impl WarmEpoch {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            last_send_ms: std::cell::Cell::new(0),
            cold_start_count: std::cell::Cell::new(0),
            eager_warmup_sent_ms: std::cell::Cell::new(0),
            last_unicode_transmit_ms: std::cell::Cell::new(0),
        }
    }

    /// コールド状態にマークする（eager_warmup_sent_ms / last_unicode_transmit_ms をリセット）。
    pub fn mark_cold(&self) {
        self.eager_warmup_sent_ms.set(0);
        self.last_unicode_transmit_ms.set(0);
    }

    /// フォーカス変更時に eager_warmup_sent_ms / last_unicode_transmit_ms をリセットする。
    pub fn on_focus_changed(&self) {
        self.eager_warmup_sent_ms.set(0);
        self.last_unicode_transmit_ms.set(0);
    }

    /// unicode 送信時刻を返す。0 = 未送信。
    #[must_use]
    pub const fn last_unicode_transmit_ms(&self) -> u64 {
        self.last_unicode_transmit_ms.get()
    }

    /// unicode 送信時刻をセットする。
    pub fn set_last_unicode_transmit_ms(&self, ms: u64) {
        self.last_unicode_transmit_ms.set(ms);
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
    pub const fn eager_warmup_sent_ms(&self) -> u64 {
        self.eager_warmup_sent_ms.get()
    }

    /// eager warmup F2 の送信時刻をセットする。
    pub fn set_eager_warmup_sent_ms(&self, ms: u64) {
        self.eager_warmup_sent_ms.set(ms);
    }

    /// cold-start 発生回数を返す。
    #[must_use]
    pub const fn cold_start_count(&self) -> u32 {
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
    #[must_use]
    pub const fn new() -> Self {
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
    pub const fn idle_ms_at_last_cold(&self) -> u64 {
        self.idle_ms_at_last_cold.get()
    }

    /// 最後に cold にマークされた理由を返す。
    #[must_use]
    pub const fn last_cold_reason(&self) -> crate::output::ColdReason {
        self.last_cold_reason.get()
    }

    /// `RawTsfLiteralRecovery` が連続で発火した回数を返す。
    #[must_use]
    pub const fn consecutive_count(&self) -> u32 {
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
/// 内部を責務別サブ構造体に分割している:
/// - `warm_epoch`: warmup epoch・送信タイミング・cold-start 回数
/// - `cold_ctx`: cold の理由・idle 時間・連続 recovery 回数
#[derive(Debug)]
pub struct CompositionState {
    /// warmup epoch・送信タイミング・cold-start 回数
    pub warm_epoch: WarmEpoch,
    /// cold の理由・idle 時間・連続 recovery 回数
    pub cold_ctx: ColdContext,
}

impl Default for CompositionState {
    fn default() -> Self {
        Self::new()
    }
}

impl CompositionState {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            warm_epoch: WarmEpoch::new(),
            cold_ctx: ColdContext::new(),
        }
    }

    /// IME composition context をコールド状態にマークする。
    pub fn mark_composition_cold(&self, reason: crate::output::ColdReason) {
        let idle_ms = self.ms_since_last_send();
        if reason == crate::output::ColdReason::RawTsfLiteralRecovery {
            let n = self.cold_ctx.increment_consecutive_count();
            log::debug!("[composition] marked cold reason={reason:?} idle={idle_ms}ms consecutive={n} → next VK/TSF output will send VK_DBE_HIRAGANA warmup");
        } else {
            // consecutive_count はフォーカス変更と SetOpenTrue（engine activation）でリセット。
            // SetOpenTrue = engine が新たに IME ON を決定した瞬間。前回セッションのリテラル履歴は
            // 新しい IME ON セッションには無関係なため、リセットして recovery の機会を再付与する。
            // これにより「0xF2→literal→BS再送(count=1)→0xF0→0xF2→literal→give up→stuck」を防ぐ。
            // PassthroughConfirmKey / ReinjectConfirmKey / SymbolVkSent 等の通常タイピング操作では
            // リセットしないことで「GJI 非対応ウィンドウでスペースを押すたびに BS が発動する」
            // false positive ループを防ぐ。
            if matches!(
                reason,
                crate::output::ColdReason::FocusChange | crate::output::ColdReason::SetOpenTrue
            ) {
                self.cold_ctx.reset_consecutive_count();
            }
            log::debug!("[composition] marked cold reason={reason:?} idle={idle_ms}ms → next VK/TSF output will send VK_DBE_HIRAGANA warmup");
        }
        self.warm_epoch.mark_cold();
        self.cold_ctx.record_cold(reason, idle_ms);
        // PassthroughConfirmKey/ReinjectConfirmKey は物理 Space/Enter/Escape が
        // 実際に OS の TSF composition context へ届くタイミングであり、それ自体が
        // 「活動」である。send_keys() を経由しないローマ字非出力の確定キー連打では
        // last_send_ms が更新されず、idle_ms が壁時計時間で単調増加し続けて通常の
        // タイピングを long_idle と誤判定してしまう（2026-07-10 診断）。確定キー系
        // の reason に限り last_send_ms をここで touch し、次回の idle_ms 計算を
        // このイベント基準にリセットする。
        if reason.is_confirm_key() {
            self.warm_epoch.update_last_send_ms();
        }
    }

    /// フォーカスウィンドウが変わったことを通知する。
    pub fn on_focus_changed(&self) {
        let idle_ms = self.ms_since_last_send();
        self.warm_epoch.on_focus_changed();
        // FocusChange で last_cold_reason を更新し、F2NonTsf などの前回理由が
        // フォーカス遷移後も残り続けて誤判定される不具合を防ぐ。
        self.cold_ctx
            .record_cold(crate::output::ColdReason::FocusChange, idle_ms);
        self.cold_ctx.reset_consecutive_count();
        log::debug!("[composition] focus changed → marked cold");
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
    pub const fn eager_warmup_sent_ms(&self) -> u64 {
        self.warm_epoch.eager_warmup_sent_ms()
    }

    /// eager warmup F2 の送信時刻をセットする。
    pub fn set_eager_warmup_sent_ms(&self, ms: u64) {
        self.warm_epoch.set_eager_warmup_sent_ms(ms);
    }

    /// 最後に cold になった時点での idle 時間（ms）を返す。
    #[must_use]
    pub const fn idle_ms_at_last_cold(&self) -> u64 {
        self.cold_ctx.idle_ms_at_last_cold()
    }

    /// cold-start 発生回数を返す。
    #[must_use]
    pub const fn cold_start_count(&self) -> u32 {
        self.warm_epoch.cold_start_count()
    }

    /// cold-start 発生回数をインクリメントして新値を返す。
    pub fn increment_cold_start_count(&self) -> u32 {
        self.warm_epoch.increment_cold_start_count()
    }

    /// 最後に cold にマークされた理由を返す。
    #[must_use]
    pub const fn last_cold_reason(&self) -> crate::output::ColdReason {
        self.cold_ctx.last_cold_reason()
    }

    /// `RawTsfLiteralRecovery` が連続で発火した回数を返す。
    #[must_use]
    pub const fn consecutive_count(&self) -> u32 {
        self.cold_ctx.consecutive_count()
    }

    /// `RawTsfLiteralRecovery` 連続カウントをリセットする。
    ///
    /// `DetectionResult::CompositionConfirmed`（非 partial）を確認した dispatcher が
    /// 呼ぶ（BUG-27 追補4）。フォーカス変更・`SetOpenTrue` 以外に、本物の confirm
    /// でも「連続失敗」の連続性が切れるため、ここでもリセットが必要。
    pub fn reset_consecutive_count(&self) {
        self.cold_ctx.reset_consecutive_count();
    }

    /// unicode 送信時刻を返す。0 = 未送信。
    #[must_use]
    pub const fn last_unicode_transmit_ms(&self) -> u64 {
        self.warm_epoch.last_unicode_transmit_ms()
    }

    /// unicode 送信時刻をセットする（`PendingGjiConfirm` 状態の開始）。
    pub fn set_last_unicode_transmit_ms(&self, ms: u64) {
        self.warm_epoch.set_last_unicode_transmit_ms(ms);
    }
}

// ── LiteralDetector ──

/// `send_romaji_as_tsf` が文字を送信した直後に生成し、
/// GJI 候補ウィンドウの変化を監視して composition が成功したか判定する検出器。
///
/// ## 確認シグナル（TSF/Chrome 共通、BUG-30 で統一）
///
/// `gji_write_bytes()` が [`COMPOSITION_BYTES_THRESHOLD`] バイト以上増加した、
/// **または** `gji_candidate_show`（候補ウィンドウ SHOW イベント）が発火した、
/// のいずれかで `CompositionConfirmed` とする（OR 条件、BUG-27 追補5）。
///
/// 以前は target（Chrome/TSF）ごとに別ロジックだった（Chrome は write-bytes 閾値、
/// TSF は `gji_last_io_ms` の「変化の有無」を閾値なしで判定）。閾値なし判定は
/// 「F2 等のモード切り替えキーでも `gji_last_io_ms` が変化する」誤検知の懸念が
/// あり、かつ Chrome 側は cold literal（未 compose）でも WriteTransferCount が
/// 実測 +300B ほど動くことが分かっている（[`COMPOSITION_BYTES_THRESHOLD`] の
/// 根拠参照）ため、TSF-native composition でも同様の非ゼロ I/O が literal 時に
/// 出ないという保証はなかった。閾値なし判定より閾値ありの方が安全側（confirm が
/// 遅れる方向にしか倒れない）と判断し、target 分岐を撤去して統一した
/// （`docs/known-bugs.md` BUG-30 追補1）。
///
/// **実測未了**: [`COMPOSITION_BYTES_THRESHOLD`]（350B）は Chrome の SendInput
/// 経路で実測された値であり、TSF-native composition（WezTerm 等）で同じ桁の
/// I/O が出るかは未検証。実機で乖離が確認された場合は target 別の定数に
/// 分離すること。
#[derive(Debug)]
pub struct LiteralDetector {
    /// 送信前の GJI 候補ウィンドウ SHOW ベースライン
    gji_show_baseline: Baseline,
    /// 送信前の WriteTransferCount 累積値
    write_bytes_baseline: u64,
    /// 候補ウィンドウ可視性による backspace veto（BUG-30）を適用してよいか。
    ///
    /// per-VK 単体確認（`TransmitSingleVk`）では前モーラ由来の誤 veto の恐れが
    /// あるため `false` を渡すこと。単語単位のバッチ確認では `true`。
    veto_eligible: bool,
    /// この detector が対応する送信（VK 送信 or romaji バッチ送信）の epoch。
    ///
    /// 構築時点（VK/バッチ送信の直前）の `current_tick_ms()`。ADR-079 の
    /// epoch fencing で「confirm の根拠になった GJI I/O が本当にこの送信より
    /// 後に起きたものか」を判定するために使う。[`Self::check_now`] と
    /// [`Self::visible_fencing_verdict`] の両方が参照する。
    epoch_send_ms: u64,
    /// SHOW イベントのみで confirmed 判定されたが `gji_last_write_ms()` が
    /// まだ `epoch_send_ms` に追いついていない状態を最初に観測した時刻。
    /// `None` = 未観測。ADR-079 fencing の一時的な hold 状態。
    show_stale_hold_since_ms: std::cell::Cell<Option<u64>>,
}

/// raw-TSF-literal 検出結果。
#[derive(Debug)]
pub enum DetectionResult {
    /// composition 成功（IME が文字を受け付けた）
    CompositionConfirmed,
    /// raw TSF literal 疑い（IME をバイパスして ASCII が出力された）
    SuspectedLiteral,
    /// confirm の根拠（候補 SHOW / write-bytes 増加）が、この送信より前に
    /// 起きた GJI I/O に由来する（= 別の、既に見捨てた世代の残存証拠を
    /// 誤って現世代の confirm として使い回そうとしている）と判明した
    /// （ADR-079 epoch fencing）。
    StaleConfirm,
}

impl LiteralDetector {
    /// SHOW イベントは write-bytes ポーリング（`GJI_SAMPLE_INTERVAL_MS`）より
    /// 早く届きうるため、fencing 判定前に最大2ポーリング分だけ
    /// `gji_last_write_ms()` の追いつきを待つ猶予。
    ///
    /// UX タイミング調整の定数ではなく、既存のポーリング周期
    /// `GJI_SAMPLE_INTERVAL_MS`（構造的に既に確定している値）の2倍という
    /// 数学的なマージンに過ぎないため、`tuning-constants.md` の実測義務の
    /// 対象外とする（ADR-079 実装計画、Opus レビュー欠陥2対処）。
    ///
    /// `pub(crate)`: `probe_fsm.rs` の「候補ウィンドウ既に可視」ショートカット
    /// （[`Self::visible_fencing_verdict`]）のテストが猶予切れを模擬するために
    /// この値を必要とするため。
    pub(crate) const EPOCH_FENCE_GRACE_MS: u64 = crate::tuning::GJI_SAMPLE_INTERVAL_MS as u64 * 2;

    /// 現在の観測値からベースラインを取得して `LiteralDetector` を生成する。
    ///
    /// ローマ字送信直前に呼ぶこと。`veto_eligible` は
    /// [`veto_eligible`](Self::veto_eligible) を参照。
    #[must_use]
    pub fn new(veto_eligible: bool) -> Self {
        Self {
            gji_show_baseline: TSF_OBS.gji_candidate_show.baseline(),
            write_bytes_baseline: crate::tsf::observer::gji_write_bytes(),
            veto_eligible,
            epoch_send_ms: crate::hook::current_tick_ms(),
            show_stale_hold_since_ms: std::cell::Cell::new(None),
        }
    }

    /// VK 送信前に取得済みの WriteTransferCount ベースラインを使う `LiteralDetector` を生成する。
    ///
    /// `SacrificialWarmup` の Chrome パス・per-VK confirm で使用する。VK 送信後に
    /// [`Self::new`] を呼ぶと、タイミングによっては当該 VK の write がベースラインに
    /// 吸収されて検出できない。VK 送信直前に取得したベースラインを引数で渡すことで
    /// この race を解消する。
    ///
    /// ## 閾値の根拠（Chrome 実測、TSF は未検証。型ドキュメント参照）
    ///
    /// 実機ログ（5サンプル）より:
    /// - cold Chrome（リテラル 'a'）: VK_A 後 w_KB ≈ +0.3KB（+300 バイト）
    /// - warm Chrome（コンポジション 'あ'）: VK_A 後 w_KB ≈ +0.4KB（+400 バイト）
    ///
    /// [`COMPOSITION_BYTES_THRESHOLD`] = 350 バイトで cold/warm を分離できる。
    #[must_use]
    pub fn new_with_pre_send_baseline(write_bytes_before_vk: u64, veto_eligible: bool) -> Self {
        Self {
            gji_show_baseline: TSF_OBS.gji_candidate_show.baseline(),
            write_bytes_baseline: write_bytes_before_vk,
            veto_eligible,
            epoch_send_ms: crate::hook::current_tick_ms(),
            show_stale_hold_since_ms: std::cell::Cell::new(None),
        }
    }

    /// cold（リテラル 'a': +300B）と warm（コンポジション 'あ': +400B）を
    /// 区別するための WriteTransferCount 増加閾値。
    ///
    /// 実機ログ 5 サンプル（Chrome）に基づく。cold/warm の中間値 350 バイトを閾値とする。
    const COMPOSITION_BYTES_THRESHOLD: u64 = 350;

    /// タイマーポーリング用ノンブロッキング判定。
    ///
    /// `Some` = 判定確定、`None` = まだ待機中。
    /// TIMER_TSF_PROBE ハンドラから 10ms ごとに呼ぶ。
    ///
    /// ## ADR-079 epoch fencing
    ///
    /// confirm の根拠（write-bytes 閾値超過 / 候補 SHOW）が本当にこの
    /// detector の送信（`epoch_send_ms`）より後に起きた GJI I/O に由来するかを
    /// `gji_last_write_ms()` と突き合わせる。前の世代の backspace で見捨てた
    /// 送信の遅延した副作用を、現世代の confirm として誤って採用してしまう
    /// （実機トレースで確認済み、ADR-079 コンテキスト節）ことを防ぐ。
    ///
    /// - `write_confirmed`: write-bytes の増加は `gji_last_write_ms` の
    ///   更新と同一ポーリングサンプルで自己整合するため、fencing は即時判定
    ///   でよい。
    /// - `show_confirmed`（write 未達）: `EVENT_OBJECT_SHOW` は write-bytes の
    ///   ポーリング（`GJI_SAMPLE_INTERVAL_MS`）より早く届きうるため、
    ///   即座に stale と断定せず [`Self::EPOCH_FENCE_GRACE_MS`] の間だけ
    ///   `gji_last_write_ms()` が追いつくのを待ってから再判定する。
    /// - `gji_last_write_ms() == 0`（GJI I/O monitor が未アタッチ等で一度も
    ///   観測していない）場合は fencing を適用せず、常に従来通りの confirm
    ///   判定にフォールバックする。
    #[must_use]
    pub fn check_now(&self, deadline_ms: u64) -> Option<DetectionResult> {
        let now = crate::hook::current_tick_ms();

        // COMPOSITION_BYTES_THRESHOLD (350B) は「VK_A→'あ' のように1VKで
        // 完結する1文字」の実測（5サンプル）に基づく値であり、per-VK confirm
        // （`TransmitSingleVk`、romaji を1VKずつ送る）が子音単体（例: "si"の"s"、
        // "ta"の"t"）を送った直後に問い合わせるケースは検証対象外だった。子音単体は
        // romaji バッファがまだ未確定のため、完結した1文字と同じ量の
        // WriteTransferCount 増加が起きるとは限らない。実機（Microsoft
        // Teams/TeamsWebView）で「候補ウィンドウは正しく表示されている
        // （[gji-obs] candidate SHOW ログで確認済み）のに 350B閾値に届かず
        // SuspectedLiteral と誤判定され、正しく入力できていた文字が backspace で
        // 消える」regression を確認した（docs/known-bugs.md BUG-27 追補5）。
        // SHOW イベントが増えていれば、WriteTransferCount 閾値に未達でも
        // confirmed とする（OR 条件）。
        //
        // 既知の限界: 直前の VK 送信で候補ウィンドウが既に表示中だった場合、
        // SHOW カウンタは「新規表示」でのみ増分するため、続く VK では SHOW が
        // 増えないケースがあり得る。その場合は write-bytes 閾値に委ねる
        // （OR 条件なので write-bytes 側が拾えば確認できる）。両方とも拾えない
        // 残存ケースは `LiteralDetectCore::veto_decision`（BUG-30）が
        // 候補ウィンドウ可視性ベースの veto で補う。
        let write_confirmed = crate::tsf::observer::gji_write_bytes()
            > self
                .write_bytes_baseline
                .saturating_add(Self::COMPOSITION_BYTES_THRESHOLD);
        let show_confirmed = TSF_OBS
            .gji_candidate_show
            .has_changed(self.gji_show_baseline);

        let last_write_ms = crate::tsf::observer::gji_last_write_ms();
        // monitor 未観測なら fencing 自体を無効化する（Opus レビュー指摘3対処）。
        let fencing_active = last_write_ms != 0;
        let evidence_is_fresh = !fencing_active || last_write_ms >= self.epoch_send_ms;

        if write_confirmed {
            return Some(if evidence_is_fresh {
                DetectionResult::CompositionConfirmed
            } else {
                DetectionResult::StaleConfirm
            });
        }

        if show_confirmed {
            return self.grace_hold_verdict(now, deadline_ms, evidence_is_fresh);
        }

        if now >= deadline_ms {
            Some(DetectionResult::SuspectedLiteral)
        } else {
            None
        }
    }

    /// SHOW-only な confirm 根拠に対する ADR-079 猶予判定の共通実装。
    ///
    /// `evidence_is_fresh` が `true` なら即座に `CompositionConfirmed`。
    /// `false` なら、`gji_last_write_ms()` が追いつくかもしれない
    /// [`Self::EPOCH_FENCE_GRACE_MS`] の間だけ即断せず `None`（＝呼び出し側は
    /// 次 tick で再判定すること）を返し、猶予切れ後に `StaleConfirm` で確定する。
    /// `deadline_ms` を過ぎている場合は猶予中でも即 `StaleConfirm` にする
    /// （猶予は「少し待てば追いつくかも」の窓であり、deadline 到達後まで
    /// 引き延ばす理由はない）。
    ///
    /// [`Self::check_now`] の SHOW-only 分岐と
    /// [`Self::visible_fencing_verdict`]（`probe_fsm.rs` の「候補ウィンドウ
    /// 既に可視」ショートカット）の両方から使う。両者は 1 detector
    /// インスタンスにつきどちらか一方の経路しか通らない
    /// （`await_vk_detection` が `gji_candidate_visible_now()` で分岐する）ため、
    /// `show_stale_hold_since_ms` の hold 状態が競合することはない。
    fn grace_hold_verdict(
        &self,
        now: u64,
        deadline_ms: u64,
        evidence_is_fresh: bool,
    ) -> Option<DetectionResult> {
        if evidence_is_fresh {
            self.show_stale_hold_since_ms.set(None);
            return Some(DetectionResult::CompositionConfirmed);
        }
        let hold_since = self.show_stale_hold_since_ms.get().unwrap_or_else(|| {
            self.show_stale_hold_since_ms.set(Some(now));
            now
        });
        if now.saturating_sub(hold_since) < Self::EPOCH_FENCE_GRACE_MS {
            return (now >= deadline_ms).then_some(DetectionResult::StaleConfirm);
        }
        Some(DetectionResult::StaleConfirm)
    }

    /// `probe_fsm.rs::await_vk_detection` の「候補ウィンドウが送信直後から
    /// 既に可視」ショートカット専用の epoch fencing 判定（ADR-079）。
    ///
    /// このケースでは [`Self::check_now`] の確定シグナル（write-bytes 閾値・
    /// SHOW エッジ）が構造的に機能しない（SHOW は既に可視＝再発火せず、
    /// per-VK 単体の子音では write-bytes 閾値に届かないことがある。
    /// `probe_fsm.rs` の `await_vk_detection` 冒頭コメント参照）ため、
    /// `check_now` に委譲できない。代わりに `gji_last_write_ms()` と
    /// `epoch_send_ms` の比較だけを根拠にし、[`Self::grace_hold_verdict`] と
    /// 同じ猶予ロジックを共有する。
    ///
    /// 呼び出し側は `Some` が返るまで tick ごとに呼び直すこと（`None` は
    /// 「まだ猶予中、次 tick で再確認」を意味する）。猶予無しで即断すると、
    /// 高速タイピングで候補ウィンドウが開きっぱなしの通常ケースでも
    /// `gji_last_write_ms` のポーリングサンプルが追いつく前に false positive
    /// の `StaleConfirm` を返してしまい、正しく合成できていた文字が
    /// backspace で失われる（実機で確認済み）。
    #[must_use]
    pub(crate) fn visible_fencing_verdict(&self, deadline_ms: u64) -> Option<DetectionResult> {
        let now = crate::hook::current_tick_ms();
        let last_write_ms = crate::tsf::observer::gji_last_write_ms();
        let fencing_active = last_write_ms != 0;
        let evidence_is_fresh = !fencing_active || last_write_ms >= self.epoch_send_ms;
        self.grace_hold_verdict(now, deadline_ms, evidence_is_fresh)
    }

    /// 候補ウィンドウ可視性による backspace veto（BUG-30）を適用してよいかどうか。
    ///
    /// 構築時に渡された `veto_eligible` をそのまま返す。per-VK 単体確認では
    /// 前モーラ由来の誤 veto の恐れがあるため `false` で構築すること。
    #[must_use]
    pub(crate) fn veto_eligible(&self) -> bool {
        self.veto_eligible
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

    /// `check_now` が `true` を返すまでポーリングする（旧 `wait_until_ready`、
    /// 本番未使用のため 2026-07-19 に撤去。`check_now` 自体のタイミング挙動を
    /// 検証するテスト専用の待機ヘルパーとしてここに残す）。
    fn poll_until_ready(probe: &TsfReadinessProbe, total_max_ms: u64) {
        const POLL_MS: u64 = 10;
        loop {
            if probe.check_now(total_max_ms) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(POLL_MS));
        }
    }

    /// GJI モニター不可のとき、total_max_ms ぶん待機して返る（フォールバックパス）
    #[test]
    fn probe_fallback_waits_total_max_ms() {
        let _g = TEST_LOCK.lock().unwrap();
        TSF_OBS.gji_monitor_ok.store(false, SeqCst);

        let start = Instant::now();
        let now_ms = crate::hook::current_tick_ms();
        let probe = TsfReadinessProbe::new(now_ms, 0, 0);
        poll_until_ready(&probe, 100);

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
        poll_until_ready(&probe, 1_000);

        let elapsed = start.elapsed().as_millis();
        // 即 settled（margin = POST_IDLE_MARGIN_MS = 30ms 以内）
        assert!(
            elapsed < 150,
            "should settle quickly (idle>80ms), got {elapsed}ms"
        );
    }

    /// phase1: min_ms が経過するまで probe は I/O 観測を信頼しない
    #[test]
    fn probe_phase1_min_wait_respected() {
        let _g = TEST_LOCK.lock().unwrap();
        let now_ms = crate::hook::current_tick_ms();

        // GJI は settled 状態だが min_ms=80 のため phase1 で 80ms 待機する
        TSF_OBS.gji_monitor_ok.store(true, SeqCst);
        TSF_OBS
            .gji_last_io_ms
            .store(now_ms.saturating_sub(200), SeqCst); // 200ms 前に I/O（warmup 前）

        let start = Instant::now();
        let probe = TsfReadinessProbe::new(now_ms, 0, 80); // min_ms=80
        poll_until_ready(&probe, 300);

        let elapsed = start.elapsed().as_millis();
        // min_ms=80 の phase1 wait 後に即解放（I/O なし → 正常状態）
        // → 60ms 以上 200ms 以内に完了するはず
        assert!(elapsed >= 60, "phase1 min_wait not respected: {elapsed}ms");
        assert!(
            elapsed < 200,
            "should release at ~80ms, not wait full 300ms: {elapsed}ms"
        );
    }

    /// warmup 後に GJI I/O が発生しない場合は min_ms 経過後に即解放（WezTerm 等の正常ケース）
    #[test]
    fn probe_phase2_ready_immediately_when_no_io_after_warmup() {
        let _g = TEST_LOCK.lock().unwrap();
        let now_ms = crate::hook::current_tick_ms();

        // GJI I/O は warmup より前 → warmup 後に I/O なし → 既に正常状態 → min_ms 経過で即解放
        TSF_OBS.gji_monitor_ok.store(true, SeqCst);
        TSF_OBS
            .gji_last_io_ms
            .store(now_ms.saturating_sub(5_000), SeqCst);

        let start = Instant::now();
        let probe = TsfReadinessProbe::new(now_ms, 0, 0); // min_ms=0
        poll_until_ready(&probe, 1_000);

        let elapsed = start.elapsed().as_millis();
        // min_ms=0 なので即解放（1000ms タイムアウトを待たない）
        assert!(elapsed < 100, "should release immediately, got {elapsed}ms");
    }

    /// 確定キー(Space/Enter/Escape)連打時、ローマ字出力(send_keys)を伴わなくても
    /// `mark_composition_cold(PassthroughConfirmKey/ReinjectConfirmKey)` のたびに
    /// last_send_ms がリセットされ、idle_ms が壁時計時間で累積し続けないことを確認する。
    ///
    /// 2026-07-10 診断: この reset が無いと、通常タイピング中の確定キー連打
    /// （Tab移動やフォーム入力等、ローマ字出力を伴わない Space/Enter）で idle_ms が
    /// 単調増加し続け、`cold_warmup.rs` の long_idle 判定が誤って true になり
    /// 不要に settle 予算(1500/2000ms)を拡張してしまう（回帰防止）。
    #[test]
    fn confirm_key_cold_mark_resets_idle_instead_of_accumulating() {
        let _g = TEST_LOCK.lock().unwrap();
        let state = CompositionState::new();

        // 実際のローマ字送信 (send_keys 相当) を模擬した baseline。
        state.update_last_send_ms();
        std::thread::sleep(std::time::Duration::from_millis(60));

        state.mark_composition_cold(crate::output::ColdReason::PassthroughConfirmKey);
        let first_idle = state.cold_ctx.idle_ms_at_last_cold();
        assert!(
            (40..150).contains(&first_idle),
            "1回目の idle は直前 sleep の ~60ms を反映するはず: {first_idle}ms"
        );

        std::thread::sleep(std::time::Duration::from_millis(60));

        state.mark_composition_cold(crate::output::ColdReason::PassthroughConfirmKey);
        let second_idle = state.cold_ctx.idle_ms_at_last_cold();
        assert!(
            (40..150).contains(&second_idle),
            "確定キー連打のたびに last_send_ms がリセットされ idle は ~60ms のはず\
             （リセットされないと累積して ~120ms になり long_idle 誤判定を招く）: {second_idle}ms"
        );
    }

    // ── BUG-27 追補5: Chrome per-VK confirm の write-bytes 閾値が候補ウィンドウ
    // SHOW を見ていなかった問題の回帰テスト ────────────────────────────────

    /// write-bytes が閾値未満でも、候補ウィンドウ SHOW が観測されていれば
    /// `CompositionConfirmed` を返すことを確認する。
    ///
    /// `COMPOSITION_BYTES_THRESHOLD`（350B）は「VK_A→'あ' のように1VKで完結する
    /// 1文字」の実測に基づく値で、Chrome per-VK confirm が子音単体
    /// （例: "si"の"s"）を送った直後に問い合わせるケースは対象外だった。
    /// 実機（Microsoft Teams/TeamsWebView）で「候補ウィンドウは正しく表示されて
    /// いるのに350B閾値に届かず SuspectedLiteral と誤判定される」regression を
    /// 確認した（docs/known-bugs.md BUG-27 追補5）。
    #[test]
    fn check_now_confirms_via_candidate_show_when_write_bytes_below_threshold() {
        let _g = TEST_LOCK.lock().unwrap();
        TSF_OBS.gji_write_bytes.store(1_000, SeqCst);

        let detector = LiteralDetector::new_with_pre_send_baseline(1_000, true);

        // write_bytes は閾値未満のまま（子音単体で完結した1文字分の書き込みが
        // 起きないケースを模擬）。
        TSF_OBS.gji_write_bytes.store(1_100, SeqCst); // +100B < 350B 閾値

        // しかし候補ウィンドウの SHOW イベントは観測されている。
        TSF_OBS.gji_candidate_show.notify();

        let now_ms = crate::hook::current_tick_ms();
        let result = detector.check_now(now_ms + 10_000); // まだ deadline 未到達
        assert!(
            matches!(result, Some(DetectionResult::CompositionConfirmed)),
            "write-bytes 閾値未達でも candidate SHOW があれば CompositionConfirmed の \
             はず: {result:?}"
        );
    }

    /// write-bytes 閾値超過・SHOW どちらも観測されなければ、従来通り deadline 経過で
    /// `SuspectedLiteral` を返すことを確認する（本物の literal 化検出は壊さない）。
    #[test]
    fn check_now_still_detects_suspected_literal_when_neither_signal_fires() {
        let _g = TEST_LOCK.lock().unwrap();
        TSF_OBS.gji_write_bytes.store(2_000, SeqCst);
        let baseline_show = TSF_OBS.gji_candidate_show.baseline();
        TSF_OBS.gji_last_write_ms.store(0, SeqCst);

        let detector = LiteralDetector::new_with_pre_send_baseline(2_000, true);

        // write_bytes・candidate_show とも変化なし（本物の literal 化を模擬）。
        assert!(!TSF_OBS.gji_candidate_show.has_changed(baseline_show));

        let now_ms = crate::hook::current_tick_ms();
        let result = detector.check_now(now_ms); // deadline は既に到達済み
        assert!(
            matches!(result, Some(DetectionResult::SuspectedLiteral)),
            "write-bytes・SHOW とも変化なしで deadline 到達なら SuspectedLiteral の \
             はず: {result:?}"
        );
    }

    // ── ADR-079: epoch fencing の回帰テスト ─────────────────────────────────

    /// write-bytes 閾値超過の confirm 根拠が epoch 送信時刻より後の GJI I/O に
    /// 裏付けられていれば、従来通り即座に `CompositionConfirmed` を返す。
    #[test]
    fn check_now_confirms_when_write_evidence_is_fresh() {
        let _g = TEST_LOCK.lock().unwrap();
        TSF_OBS.gji_write_bytes.store(4_000, SeqCst);
        let detector = LiteralDetector::new_with_pre_send_baseline(4_000, true);
        let now_ms = crate::hook::current_tick_ms();
        TSF_OBS.gji_last_write_ms.store(now_ms, SeqCst); // epoch_send_ms 以降
        TSF_OBS.gji_write_bytes.store(4_400, SeqCst); // 閾値(350B)超過

        let result = detector.check_now(now_ms + 10_000);
        assert!(
            matches!(result, Some(DetectionResult::CompositionConfirmed)),
            "fresh な write 根拠は即座に CompositionConfirmed のはず: {result:?}"
        );
        TSF_OBS.gji_last_write_ms.store(0, SeqCst);
    }

    /// write-bytes 閾値超過の confirm 根拠が epoch 送信時刻より前の GJI I/O
    /// （前世代の見捨てた送信の残存効果）にしか裏付けられていなければ、
    /// deadline を待たず即座に `StaleConfirm` を返す（ADR-079 コンテキスト節の
    /// 実機トレースが示す誤帰属パターンの再発防止）。
    #[test]
    fn check_now_returns_stale_confirm_when_write_evidence_predates_epoch() {
        let _g = TEST_LOCK.lock().unwrap();
        let stale_write_ms = crate::hook::current_tick_ms();
        TSF_OBS.gji_last_write_ms.store(stale_write_ms, SeqCst);
        std::thread::sleep(std::time::Duration::from_millis(5));

        TSF_OBS.gji_write_bytes.store(5_000, SeqCst);
        let detector = LiteralDetector::new_with_pre_send_baseline(5_000, true); // epoch > stale_write_ms
        TSF_OBS.gji_write_bytes.store(5_400, SeqCst); // 閾値超過だが根拠は stale

        let now_ms = crate::hook::current_tick_ms();
        let result = detector.check_now(now_ms + 10_000); // deadline 未到達
        assert!(
            matches!(result, Some(DetectionResult::StaleConfirm)),
            "epoch より前の write 根拠は StaleConfirm のはず: {result:?}"
        );
        TSF_OBS.gji_last_write_ms.store(0, SeqCst);
    }

    /// SHOW のみによる confirm 根拠が epoch より前の write にしか裏付けられて
    /// いない場合、即断せず `EPOCH_FENCE_GRACE_MS` の間は保留（`None`）する
    /// （SHOW が write-bytes ポーリングより早く届く benign なレースを誤って
    /// stale 扱いしないため、Opus レビュー欠陥2対処）。猶予中に
    /// `gji_last_write_ms` が追いつけば `CompositionConfirmed` に確定する。
    #[test]
    fn check_now_holds_show_only_confirm_then_resolves_fresh_within_grace() {
        let _g = TEST_LOCK.lock().unwrap();
        TSF_OBS.gji_write_bytes.store(6_000, SeqCst);
        let epoch_ms = crate::hook::current_tick_ms();
        TSF_OBS
            .gji_last_write_ms
            .store(epoch_ms.saturating_sub(50), SeqCst); // stale

        let detector = LiteralDetector::new_with_pre_send_baseline(6_000, true);
        TSF_OBS.gji_candidate_show.notify(); // write_bytes は閾値未満のまま

        let far_deadline = crate::hook::current_tick_ms() + 10_000;
        let held = detector.check_now(far_deadline);
        assert!(
            held.is_none(),
            "SHOW根拠が stale なら即断せず猶予中は None のはず: {held:?}"
        );

        // 猶予中に gji_last_write_ms が追いつく。
        TSF_OBS
            .gji_last_write_ms
            .store(crate::hook::current_tick_ms(), SeqCst);
        let resolved = detector.check_now(far_deadline);
        assert!(
            matches!(resolved, Some(DetectionResult::CompositionConfirmed)),
            "猶予中に write が追いつけば CompositionConfirmed に確定するはず: {resolved:?}"
        );
        TSF_OBS.gji_last_write_ms.store(0, SeqCst);
    }

    /// SHOW のみによる confirm 根拠が猶予期間を過ぎても epoch に追いつかなければ、
    /// `StaleConfirm` として確定する。
    #[test]
    fn check_now_show_only_confirm_becomes_stale_after_grace_expires() {
        let _g = TEST_LOCK.lock().unwrap();
        TSF_OBS.gji_write_bytes.store(7_000, SeqCst);
        let epoch_ms = crate::hook::current_tick_ms();
        TSF_OBS
            .gji_last_write_ms
            .store(epoch_ms.saturating_sub(50), SeqCst); // stale のまま追いつかない

        let detector = LiteralDetector::new_with_pre_send_baseline(7_000, true);
        TSF_OBS.gji_candidate_show.notify();

        let far_deadline = crate::hook::current_tick_ms() + 10_000;
        let held = detector.check_now(far_deadline);
        assert!(held.is_none(), "猶予開始直後はまだ None のはず: {held:?}");

        std::thread::sleep(std::time::Duration::from_millis(
            LiteralDetector::EPOCH_FENCE_GRACE_MS + 10,
        ));
        let result = detector.check_now(far_deadline);
        assert!(
            matches!(result, Some(DetectionResult::StaleConfirm)),
            "猶予切れでも write が追いつかなければ StaleConfirm のはず: {result:?}"
        );
        TSF_OBS.gji_last_write_ms.store(0, SeqCst);
    }

    /// `gji_last_write_ms()` が一度も観測されていない（GJI I/O monitor 未アタッチ、
    /// MS-IME 使用時等）場合は fencing 自体を無効化し、従来通りの confirm 判定に
    /// フォールバックする（Opus レビュー指摘3対処: fencing が false-negative の
    /// 温床にならないようにする）。
    #[test]
    fn check_now_fencing_inactive_when_last_write_ms_unobserved() {
        let _g = TEST_LOCK.lock().unwrap();
        TSF_OBS.gji_last_write_ms.store(0, SeqCst);
        TSF_OBS.gji_write_bytes.store(8_000, SeqCst);

        let detector = LiteralDetector::new_with_pre_send_baseline(8_000, true);
        TSF_OBS.gji_candidate_show.notify(); // write_bytes は閾値未満のまま

        let far_deadline = crate::hook::current_tick_ms() + 10_000;
        let result = detector.check_now(far_deadline);
        assert!(
            matches!(result, Some(DetectionResult::CompositionConfirmed)),
            "gji_last_write_ms 未観測 (0) なら fencing を適用せず即 CompositionConfirmed \
             のはず: {result:?}"
        );
    }
}
