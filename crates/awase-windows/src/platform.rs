#![allow(unsafe_code)]
// Win32 API 呼び出しに unsafe が必須(lib.rsのクレート全体allowから個別移管、Task #9)
//! Windows 実装の `PlatformRuntime`。
//!
//! `Output`, `SystemTray`, フォーカス検出フィールド群, `Win32Timer` を束ね、
//! `PlatformRuntime` トレイトを実装する。

use std::time::Duration;

use awase::platform::{PlatformRuntime, TsfComposition};
use awase::types::{KeyAction, RawKeyEvent};

use crate::focus::class_names::AppImeProfile;
use crate::focus::classifier::{ImmCapability, InjectionHint};
use crate::focus::tracker::FocusTracker;
use crate::output::Output;
use crate::timer::Win32Timer;
use crate::tray::SystemTray;

use crate::state::event_origin::Generation;
use crate::state::ConvModeAuthority;

/// Windows 固有のプラットフォーム実装
pub struct WindowsPlatform {
    pub output: Output,
    pub tray: SystemTray,
    pub timer: Win32Timer,
    /// Engine ON 時に送信する IME モード切り替え VK コード（None で無効）
    pub engine_on_ime_vk: Option<awase::types::VkCode>,
    /// Engine OFF 時に送信する IME モード切り替え VK コード（None で無効）
    pub engine_off_ime_vk: Option<awase::types::VkCode>,
    /// ポーリング/フォーカス変更起因の EngineStateChanged で engine_state_ime_key を
    /// 送らないためのガード。IME 状態変化 → VK 送信 → IME 状態変化の無限ループを防ぐ。
    pub suppress_engine_state_key: bool,
    /// フォーカス追跡の全状態（ウィンドウ情報・判定キャッシュ・IME キャッシュ等）。
    pub(crate) focus: FocusTracker,
    /// confirm キーの warmup タイミングを管理する FSM。
    ///
    /// executor の `pending_warmup_on_keyup: bool` ミニ FSM を状態に昇格させたもの。
    /// warm 判定そのものは GjiFsm が SSOT であり、この FSM は「confirm キー KeyDown 後、
    /// KeyUp まで warmup を保留する」遷移を所有する。
    pub(crate) composition_fsm: crate::tsf::composition_fsm::CompositionFsm,
    stamper: crate::journal::JournalStamper,
    pending_journal_entries: Vec<crate::journal::JournalEnvelope>,
    active_tsf_probe_started_ms: Option<(u64, u64)>,
    probe_tick_index: u32,
    suppressed_probe_ticks: u32,
    suppressed_literal_confirms: u16,
    pending_literal_vk: Option<PendingLiteralVk>,
}

#[derive(Debug, Clone, Copy)]
struct PendingLiteralVk {
    cold_seq: u64,
    vk: u16,
    idx: u16,
    last_idx: u16,
    target: crate::tsf::literal_facts::DetectTarget,
    sent_at_ms: u64,
}

impl std::fmt::Debug for WindowsPlatform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowsPlatform").finish_non_exhaustive()
    }
}

/// [`WindowsPlatform::suppress_engine_state_key`] を `true` にし、Drop で `false` に戻す RAII ガード。
///
/// パニック時も含めてフラグが必ずリセットされることを保証する。
/// [`WindowsPlatform::suppress_engine_state_key_guard`] 経由で取得する。
pub(crate) struct SuppressEngineStateKeyGuard(*mut bool);

impl SuppressEngineStateKeyGuard {
    fn new(platform: &mut WindowsPlatform) -> Self {
        let ptr = std::ptr::addr_of_mut!(platform.suppress_engine_state_key);
        // SAFETY: ptr は platform の有効なフィールドを指し、
        //         このガードはシングルスレッドのメインループ内でのみ使用される。
        unsafe {
            *ptr = true;
        }
        Self(ptr)
    }
}

impl Drop for SuppressEngineStateKeyGuard {
    fn drop(&mut self) {
        // SAFETY: ポインタはシングルスレッドのメインループ内でのみ使用される。
        //         WindowsPlatform は APP (SingleThreadCell) が保持しており、
        //         with_app の外側では Drop しないことが保証されている。
        unsafe {
            *self.0 = false;
        }
    }
}

impl WindowsPlatform {
    // ── コンストラクタ ────────────────────────────────────────────────────────

    /// `WindowsPlatform` を構築する。
    ///
    /// conv mode 権限の初期値は `Output::conv_mutation_allowed`（`false`）が保持する。
    /// 初期化後の権限変更は `set_conv_mode_authority()` 経由で行うこと。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        output: Output,
        tray: SystemTray,
        timer: Win32Timer,
        engine_on_ime_vk: Option<awase::types::VkCode>,
        engine_off_ime_vk: Option<awase::types::VkCode>,
        suppress_engine_state_key: bool,
        focus: FocusTracker,
        composition_fsm: crate::tsf::composition_fsm::CompositionFsm,
        stamper: crate::journal::JournalStamper,
    ) -> Self {
        Self {
            output,
            tray,
            timer,
            engine_on_ime_vk,
            engine_off_ime_vk,
            suppress_engine_state_key,
            focus,
            composition_fsm,
            stamper,
            pending_journal_entries: Vec::new(),
            active_tsf_probe_started_ms: None,
            probe_tick_index: 0,
            suppressed_probe_ticks: 0,
            suppressed_literal_confirms: 0,
            pending_literal_vk: None,
        }
    }

    pub(crate) fn drain_journal_entries(&mut self) -> Vec<crate::journal::JournalEnvelope> {
        std::mem::take(&mut self.pending_journal_entries)
    }

    pub(crate) fn gji_state_label(&self) -> String {
        self.output.gji_state_label()
    }

    fn push_journal_entry(&mut self, entry: crate::journal::JournalEntry) {
        if self.pending_journal_entries.len() >= 4096 {
            self.pending_journal_entries.remove(0);
        }
        self.pending_journal_entries.push(self.stamper.stamp(entry));
    }

    fn note_gji_transition(&mut self, trigger: impl Into<String>, state_before: String) {
        let state_after = self.gji_state_label();
        self.push_journal_entry(crate::journal::JournalEntry::GjiFsmTransition {
            trigger: trigger.into(),
            state_before,
            state_after,
        });
    }

    /// `GjiAction::StartProbe` ハンドラから呼ぶ。ADR-123: `pending_deferred`
    /// （probe 実行中に届いた別モーラの VK 退避キュー）が非ゼロのまま
    /// この probe が開始しようとしているかを journal に記録する
    /// （issue #148 の根本原因である「追い越し」の直接シグナル）。
    /// `dispatch_gji_response` 本体の cognitive complexity を抑えるため
    /// 別関数に切り出している。
    fn note_tsf_probe_started_from_gji_action(&mut self, probe_id: crate::tsf::gji_fsm::ProbeId) {
        let pending_deferred_len = self.output.pending_deferred_len();
        if pending_deferred_len > 0 {
            // この probe が pending_deferred をまだ flush されていない状態で
            // 追い越して開始しようとしている(issue #148 の根本原因そのもの)。
            tracing::warn!(
                "[gji-fsm] StartProbe probe_id={probe_id:?} が pending_deferred \
                 {pending_deferred_len} VK(s) をflush前に追い越して開始 (ADR-123)"
            );
        }
        self.push_journal_entry(crate::journal::JournalEntry::TsfProbeStarted {
            source: "GjiAction::StartProbe".to_owned(),
            // ADR-123 round 2 (architect) 指摘の修正: 以前はここに probe_id
            // をそのまま入れていたが、cold_seq とは別の採番空間であり
            // 読み違いの原因になっていた。
            cold_seq: self.output.composition.cold_start_count().value(),
            probe_id: Some(u64::from(probe_id.0)),
            gji_state: self.gji_state_label(),
            consecutive_at_start: self.output.composition.consecutive_count(),
            pending_deferred_len,
        });
    }

    fn note_tsf_probe_completed(
        &mut self,
        outcome: impl Into<String>,
        cold_seq: Option<u64>,
        probe_id: Option<u64>,
    ) {
        let now = crate::hook::current_tick_ms();
        let elapsed_ms = self
            .active_tsf_probe_started_ms
            .take()
            .map_or(0, |(started_ms, _)| now.saturating_sub(started_ms));
        self.push_journal_entry(crate::journal::JournalEntry::TsfProbeCompleted {
            outcome: outcome.into(),
            cold_seq,
            probe_id,
            elapsed_ms,
            tick_count: self.probe_tick_index,
            gji_state: self.gji_state_label(),
        });
    }

    fn reset_probe_tick_counters(&mut self) {
        self.flush_pending_literal_vk_as_aborted();
        self.probe_tick_index = 0;
        self.suppressed_probe_ticks = 0;
        self.suppressed_literal_confirms = 0;
    }

    fn note_literal_detect_record(
        &mut self,
        record: crate::tsf::literal_facts::LiteralDetectRecord,
        since_vk_sent_ms: u64,
    ) {
        if crate::journal_policy::literal_detect_is_notable(&record) {
            let suppressed_confirms = self.suppressed_literal_confirms;
            self.suppressed_literal_confirms = 0;
            self.push_journal_entry(crate::journal::JournalEntry::LiteralDetect {
                record,
                suppressed_confirms,
                since_vk_sent_ms,
            });
        } else {
            self.suppressed_literal_confirms = self.suppressed_literal_confirms.saturating_add(1);
        }
    }

    fn flush_pending_literal_vk_as_aborted(&mut self) {
        let Some(pending) = self.pending_literal_vk.take() else {
            return;
        };
        let now = crate::hook::current_tick_ms();
        let record = crate::tsf::literal_facts::LiteralDetectRecord {
            cold_seq: Generation::new(pending.cold_seq),
            facts: crate::tsf::literal_facts::LiteralDetectFacts {
                verdict: crate::tsf::literal_facts::LiteralVerdict::AbortedNoVerdict,
                route: crate::tsf::literal_facts::DetectRoute::ProbeEnd,
                path: crate::tsf::literal_facts::DetectPath::PerVk,
                target: pending.target,
                vk: Some(pending.vk),
                idx: pending.idx,
                last_idx: pending.last_idx,
                evidence: crate::tsf::literal_facts::DetectEvidence::default(),
            },
            consecutive_before: self.output.composition.consecutive_count(),
            gave_up: false,
            backs: 0,
            escape_composition: false,
            session_marked: false,
            romaji: None,
        };
        self.note_literal_detect_record(record, now.saturating_sub(pending.sent_at_ms));
    }

    fn consume_literal_detect_trace(
        &mut self,
        trace: crate::tsf::literal_facts::LiteralDetectTrace,
        terminal_timer: bool,
    ) {
        for item in trace.0 {
            match item {
                crate::tsf::literal_facts::LiteralDetectTraceItem::VkSent {
                    cold_seq,
                    vk,
                    idx,
                    last_idx,
                    target,
                } => {
                    self.pending_literal_vk = Some(PendingLiteralVk {
                        cold_seq,
                        vk,
                        idx,
                        last_idx,
                        target,
                        sent_at_ms: crate::hook::current_tick_ms(),
                    });
                }
                crate::tsf::literal_facts::LiteralDetectTraceItem::Verdict(record) => {
                    let since_vk_sent_ms = self.pending_literal_vk.take().map_or(0, |pending| {
                        crate::hook::current_tick_ms().saturating_sub(pending.sent_at_ms)
                    });
                    self.note_literal_detect_record(record, since_vk_sent_ms);
                }
            }
        }
        if terminal_timer {
            self.flush_pending_literal_vk_as_aborted();
        }
    }

    // ── Output 委譲メソッド ──────────────────────────────────────────────────

    /// `warmup_ime_on` を指定して eager warmup を送信する（ADR-098 決定1-b）。
    ///
    /// 唯一の呼び出し元（`ime_refresh.rs` の FocusChange 処理）は
    /// `warmup_ime_on()`（BUG-110ゲート適用済み）を渡すため `origin=WarmupOrigin::Gated` 固定。
    pub(crate) fn send_eager_warmup(&self, warmup_ime_on: awase::platform::WarmupImeOn) {
        self.output
            .send_eager_tsf_warmup(warmup_ime_on, crate::output::WarmupOrigin::Gated);
    }

    /// conv mode 制御権限を更新する (H-3-e)。
    ///
    /// エンジンが有効になったとき `AwaseOwned`、無効になったとき `UserOwned` を渡す。
    /// 現在の呼び出し元は `AwaseOwned` / `UserOwned` のみを渡す。
    ///
    /// conv mutation 可否の唯一の実体は `Output::conv_mutation_allowed`（Cell<bool>）で、
    /// `send_eager_tsf_warmup` / probe warmup がこのフラグを self-gate に使う。
    /// `AwaseOwned` かどうかだけが即時ゲートに効くため、ここでその bool を push する。
    pub(crate) fn set_conv_mode_authority(&self, authority: ConvModeAuthority) {
        self.output
            .set_conv_mutation_allowed(authority.allows_conv_mutation());
    }

    /// フォーカス変更時の FocusChange cold マークを Output に通知する（ime_refresh 用）。
    pub(crate) fn mark_composition_cold_focus_change(&self) {
        self.output
            .mark_composition_cold(crate::output::ColdReason::FocusChange);
    }

    /// GJI 候補ウィンドウが現在表示中かどうか（Ctrl bypass 判定用）。
    ///
    /// `GjiFsm` の状態遷移は WM_* 処理を経由するため数百 ms の遅延がある。
    /// ここでは observer が AtomicBool で即時更新する値を直接読む。
    // Platform の他メソッドと対称な API 配置のため、状態はグローバル observer から
    // 読むが `&self` を維持する（呼び出し側の `self.method()` 記法との一貫性）。
    #[allow(clippy::unused_self)]
    pub(crate) fn is_composition_warm_in_tsf(&self) -> bool {
        crate::tsf::observer::gji_candidate_visible_now()
    }

    /// composition キャンセル後の内部状態更新（Ctrl+key パススルー・
    /// `[[keymap]]` の `target_vk` 送信、いずれも IME ショートカット横取り
    /// 防止のためのキャンセル）。
    ///
    /// IMM32 の `cancel_ime_composition()` を呼んだ直後に続けて呼ぶこと。
    /// `reason` は journal・診断用に呼び出し元の文脈を正しく記録するため
    /// 呼び出し元が指定する（実装レビュー m-2、`CtrlKeyBypass` 固定だと
    /// `[[keymap]]` 起因のキャンセルも「Ctrl bypass が原因」と誤誘導する）。
    pub(crate) fn on_composition_cancel(&mut self, reason: crate::output::ColdReason) {
        self.output.mark_composition_cold(reason);
        self.gji_on_composition_reset();
    }

    /// フォーカス変更時に injection_mode を更新する（runtime 用）。
    pub(crate) const fn update_injection_mode(
        &mut self,
        mode: crate::output::types::InjectionMode,
    ) {
        self.output.update_injection_mode(mode);
    }

    /// フォーカス変更を Output に通知し、warm epoch をリセットする（runtime 用）。
    pub(crate) fn notify_focus_changed(&self) {
        self.output.on_focus_changed();
    }

    /// TSF モード確定時に TsfGate を Probing に遷移させ、保留キーを返す（runtime 用）。
    pub(crate) fn confirm_tsf(&mut self) -> Vec<RawKeyEvent> {
        self.output.confirm_tsf()
    }

    /// 非 TSF モード確定時に TsfGate を Bypass に遷移させ、保留キーを返す（runtime 用）。
    pub(crate) fn bypass_tsf(&mut self) -> Vec<RawKeyEvent> {
        self.output.bypass_tsf()
    }

    /// フォーカス変更時に TsfGate を PendingWarmup に遷移させる（bootstrap 用）。
    pub(crate) fn on_focus_change_tsf(&mut self) {
        self.output.on_focus_change_tsf();
    }

    /// TIMER_TSF_GATE タイムアウト時に TsfGate を Bypass にフォールバックし、保留キーを返す。
    pub(crate) fn on_tsf_warmup_timeout(&mut self) -> Vec<RawKeyEvent> {
        self.output.on_tsf_warmup_timeout()
    }

    /// キーを TsfGate で処理する。`true` = 保留（呼び出し元は Consumed を返すこと）。
    pub(crate) fn try_hold_key(&mut self, event: RawKeyEvent) -> bool {
        self.output.try_hold_key(event)
    }

    /// `suppress_engine_state_key = true` のスコープを RAII で管理する。
    ///
    /// 返されたガードが Drop されると `false` に戻る。パニック時も保証。
    pub(crate) fn suppress_engine_state_key_guard(&mut self) -> SuppressEngineStateKeyGuard {
        SuppressEngineStateKeyGuard::new(self)
    }

    /// eager warmup F2 を送信した時刻 (ms) を返す。0 = 未送信。
    pub(crate) const fn eager_warmup_sent_ms(&self) -> u64 {
        self.output.eager_warmup_sent_ms()
    }

    /// `send_keys()` が開始した TSF/GJI probe がまだ完了していないか。
    pub(crate) fn has_pending_tsf_work(&self) -> bool {
        self.output.has_pending_tsf_work()
    }

    /// 出力モードを切り替える（設定変更時）。
    /// pending_tsf をインストールし、TIMER_TSF_PROBE を起動する（vk_send async パス用）。
    pub(crate) fn install_pending_tsf_and_set_timer(
        &mut self,
        machine: Box<dyn crate::tsf::warmup::tickable_fsm::TickableFsm>,
    ) {
        let cold_seq = machine.cold_seq_hint().value();
        self.active_tsf_probe_started_ms = Some((crate::hook::current_tick_ms(), cold_seq));
        self.reset_probe_tick_counters();
        self.push_journal_entry(crate::journal::JournalEntry::TsfProbeStarted {
            source: "install_pending_tsf_and_set_timer".to_owned(),
            cold_seq,
            probe_id: None,
            gji_state: self.gji_state_label(),
            consecutive_at_start: self.output.composition.consecutive_count(),
            pending_deferred_len: self.output.pending_deferred_len(),
        });
        self.output.install_pending_tsf(machine);
        if let Some(cmd) = self.output.pending_tsf_timer() {
            self.apply_timer_command(cmd);
        }
    }

    // ── TIMER_TSF_PROBE / raw TSF literal ─────────────────────────────────

    /// TIMER_TSF_PROBE ハンドラ。`Output::step_probe` に委譲し、タイマー命令と GJI FSM 応答を処理する。
    pub fn advance_tsf_probe(&mut self) {
        // tick() より前に drain する: VK_A+BS atomic batch で SHOW+HIDE が最初の tick 前に
        // 完了した場合、composition_was_seen フラグは tick() が参照する前にセットされる必要がある。
        // drain を tick() の後に置くと、最初の tick で composition_was_seen=false になり
        // Phase 1 即再送に落ちて IPC race が再発する。
        self.drain_pending_composition_events();
        let state_before_step = self.gji_state_label();
        let result = self.output.step_probe();
        let state_after_step = self.gji_state_label();
        self.probe_tick_index = self.probe_tick_index.saturating_add(1);
        let terminal_timer = matches!(
            result.timer_cmd,
            crate::output::TimerCommand::Kill {
                id: crate::TIMER_TSF_PROBE
            }
        );
        self.consume_literal_detect_trace(result.literal_detect, terminal_timer);
        let notable =
            crate::journal_policy::probe_tick_is_notable(crate::journal_policy::ProbeTickFacts {
                state_changed: state_before_step != state_after_step,
                needs_composition_reset: result.needs_gji_composition_reset,
                has_gji_response: result.gji_response.is_some(),
                learned_tsf: result.learned_tsf,
                completed: result.completed_cold_seq.is_some(),
                terminal_timer,
                is_first_tick: self.probe_tick_index == 1,
            });
        if notable {
            let suppressed = self.suppressed_probe_ticks;
            self.suppressed_probe_ticks = 0;
            self.push_journal_entry(crate::journal::JournalEntry::GjiFsmTransition {
                trigger: format!(
                    "TsfProbeTick(#{}, skipped={suppressed})",
                    self.probe_tick_index
                ),
                state_before: state_before_step,
                state_after: state_after_step,
            });
        } else {
            self.suppressed_probe_ticks = self.suppressed_probe_ticks.saturating_add(1);
        }
        if result.needs_gji_composition_reset {
            self.gji_on_composition_reset();
        }
        // step_probe 内（SacrificialResend 等）で発生したイベントを追加で drain する。
        self.drain_pending_composition_events();
        if let Some(gji_resp) = result.gji_response {
            self.dispatch_gji_response(&gji_resp);
        }
        if result.learned_tsf {
            // UnicodeLiteralObserverFsm が GJI write なしと判断 → フォーカス中クラスを Tsf に昇格。
            let class_name = self.focus.class_name().to_string();
            tracing::info!("[injection-mode] {class_name:?} → Tsf 事後昇格（GJI write 未観測）");
            self.focus.learn_injection_mode_tsf(class_name);
            // 現セッション（現在のフォーカスウィンドウ）にも即時 Tsf モードを適用する。
            self.output
                .update_injection_mode(crate::output::InjectionMode::Tsf);
            // 次の文字送信が cold-start TSF probe を正しく踏むよう composition を cold にリセット。
            self.output
                .mark_composition_cold(crate::output::ColdReason::FocusChange);
        }
        if result.completed_cold_seq.is_some() || result.learned_tsf {
            let outcome = if result.learned_tsf {
                "LearnedTsf"
            } else {
                "Done"
            };
            self.note_tsf_probe_completed(outcome, result.completed_cold_seq, None);
        }
        self.apply_timer_command(result.timer_cmd);
    }

    // ── GjiFsm ディスパッチャ ────────────────────────────────────────────────

    /// `GjiFsm::on_event` / `on_timeout` の結果を処理し、タイマー操作とアクションを実行する。
    pub(crate) fn dispatch_gji_response(
        &mut self,
        response: &timed_fsm::Response<
            crate::tsf::gji_fsm::GjiAction,
            crate::tsf::gji_fsm::GjiTimer,
        >,
    ) {
        use crate::tsf::gji_fsm::{GjiAction, GjiTimer};
        use timed_fsm::TimerCommand;
        for cmd in &response.timers {
            match cmd {
                TimerCommand::Set {
                    id: GjiTimer::LongIdle,
                    duration,
                } => {
                    tracing::debug!(
                        "[gji-fsm] LongIdle timer set duration={}ms",
                        duration.as_millis()
                    );
                    self.timer.set(crate::TIMER_GJI_LONG_IDLE, *duration);
                }
                TimerCommand::Kill {
                    id: GjiTimer::LongIdle,
                } => {
                    self.timer.kill(crate::TIMER_GJI_LONG_IDLE);
                }
            }
        }
        for action in &response.actions {
            match action {
                GjiAction::StartProbe { probe_id, params } => {
                    tracing::debug!(
                        "[gji-fsm] StartProbe probe_id={probe_id:?} forces_f2={} long={}",
                        params.forces_prepend_f2,
                        params.is_long_cold
                    );
                    self.output.gji_store_probe_id(*probe_id);
                    let now_ms = crate::hook::current_tick_ms();
                    self.active_tsf_probe_started_ms = Some((now_ms, u64::from(probe_id.0)));
                    self.reset_probe_tick_counters();
                    self.note_tsf_probe_started_from_gji_action(*probe_id);
                    // Unicode injection mode では KEYEVENTF_UNICODE が GJI TSF context を迂回するため
                    // GjiWarmupFsm も ChromeProbe も作成されず GjiFsm が OnCold(Authorized) に留まり続ける。
                    // 即 WarmupComplete を dispatch して OnWarm に遷移させる。
                    // long-cold（≥10s idle）の場合:
                    //   deferred chars あり → VK_IME_ON poke + UnicodeColdWarmupFsm (GJI 起動待ち後に chars 送信)
                    //   deferred chars なし → 従来通り VK_IME_OFF→VK_IME_ON reinit
                    if self.output.injection_mode == crate::output::InjectionMode::Unicode {
                        use crate::tsf::gji_fsm::GjiEvent;
                        if params.is_long_cold {
                            let deferred = self.output.take_unicode_cold_deferred();
                            if deferred.is_empty() {
                                tracing::debug!(
                                    "[gji-fsm] Unicode long-cold StartProbe: VK_IME_OFF→VK_IME_ON reinit (chars なし)"
                                );
                                self.output.send_f22_f21_reinit();
                            } else {
                                // probe_id (GjiFsm 側の probe 相関 ID) をそのまま cold_seq の
                                // ログ相関値として転用する既存の挙動を維持する（値そのものは
                                // 変えず、型だけ Generation に揃える）。
                                self.start_unicode_cold_warmup(
                                    Generation::new(u64::from(probe_id.0)),
                                    deferred,
                                );
                            }
                        }
                        let state_before = self.gji_state_label();
                        let warmup_resp = self.output.gji_on_event(GjiEvent::WarmupComplete {
                            probe_id: *probe_id,
                        });
                        self.note_gji_transition("WarmupComplete(unicode)", state_before);
                        // ADR-123 `/code-review` 指摘: このハンドラ内で直前に push した
                        // TsfProbeStarted と Start/Complete を probe_id で突合できるよう、
                        // cold_seq には（別の採番空間である）probe_id を入れない。
                        self.note_tsf_probe_completed(
                            "UnicodeImmediate",
                            None,
                            Some(u64::from(probe_id.0)),
                        );
                        self.dispatch_gji_response(&warmup_resp);
                    }
                }
                GjiAction::CancelProbe { probe_id } => {
                    if self.output.gji_current_probe_id() == Some(*probe_id) {
                        tracing::debug!("[gji-fsm] CancelProbe probe_id={probe_id:?}");
                        // pending_tsf / OUTPUT_GATE ガード / probe_id を一括キャンセルする。
                        self.output.cancel_probe();
                        self.timer.kill(crate::TIMER_TSF_PROBE);
                        // ADR-123 `/code-review` 指摘: cancel 時点の
                        // `cold_start_count()` は StartProbe 時点と一致する保証がない
                        // （別の cold-mark を挟んでいる可能性がある）ため推測せず、
                        // probe_id のみを突合キーとして記録する。
                        self.note_tsf_probe_completed(
                            "Canceled",
                            None,
                            Some(u64::from(probe_id.0)),
                        );
                    }
                }
                // 実際の送信は Output が担うため FSM の SendInput/SendInputDirect は無視する。
                GjiAction::SendInput { .. } | GjiAction::SendInputDirect(..) => {}
                // ADR-103 決定5-a: pending（romaji の shadow）を破棄したことを明示的な
                // 行為として記録する。副作用は無い（実データの破棄は GjiFsm 内部の
                // 状態上書きで既に完了しており、ここはログ・診断専用）。
                GjiAction::DiscardPending { count, reason } => {
                    tracing::debug!("[gji-fsm] DiscardPending count={count} reason={reason:?}");
                }
            }
        }
    }

    // ── CompositionFsm ディスパッチャ ─────────────────────────────────────────

    /// `CompositionFsm` の `Response` を処理し、warmup 送信・cold mark・GJI reset を実行する。
    ///
    /// `warmup_ime_on` は `EmitWarmup` の送信先 IME 状態（ADR-098 決定1-b）。
    /// 戻り値は F2 を consume すべきか（`ConsumeF2` アクションの有無）で、TSF mode
    /// で物理 F2 を swallow する判断に使う。
    fn dispatch_composition_response(
        &mut self,
        response: &timed_fsm::Response<
            crate::tsf::composition_fsm::CompositionAction,
            std::convert::Infallible,
        >,
        warmup_ime_on: awase::platform::WarmupImeOn,
        origin: crate::output::WarmupOrigin,
    ) -> bool {
        use crate::tsf::composition_fsm::CompositionAction;
        let mut consume_f2 = false;
        for action in &response.actions {
            match *action {
                CompositionAction::EmitWarmup { reason } => {
                    tracing::debug!("[composition-fsm] EmitWarmup ({reason:?})");
                    // conv mutation の可否は Output::send_eager_tsf_warmup が
                    // `conv_mutation_allowed` で self-gate する（non-AwaseOwned なら内部で skip）。
                    self.output.send_eager_tsf_warmup(warmup_ime_on, origin);
                }
                CompositionAction::MarkCold { reason } => {
                    self.output.mark_composition_cold(reason);
                }
                CompositionAction::GjiCompositionReset => {
                    self.gji_on_composition_reset();
                }
                CompositionAction::GjiNativeF2Consumed => {
                    self.gji_on_native_f2_consumed();
                }
                CompositionAction::ConsumeF2 => {
                    consume_f2 = true;
                }
            }
        }
        consume_f2
    }

    /// `CompositionFsm` にイベントを feed し、`Response` を dispatch する。
    /// 戻り値は F2 を consume すべきか（`ConsumeF2` の有無）。
    fn feed_composition_event(
        &mut self,
        event: crate::tsf::composition_fsm::CompositionEvent,
        warmup_ime_on: awase::platform::WarmupImeOn,
        origin: crate::output::WarmupOrigin,
    ) -> bool {
        use timed_fsm::TimedStateMachine;
        let response = self.composition_fsm.on_event(event);
        let consume_f2 = self.dispatch_composition_response(&response, warmup_ime_on, origin);
        tracing::trace!(
            "[composition-fsm] state={}",
            self.composition_fsm.state_label()
        );
        consume_f2
    }

    /// confirm キー KeyUp を `CompositionFsm` に通知し、保留 warmup があれば送信する。
    ///
    /// 唯一の呼び出し元（executor の `try_pending_warmup_on_keyup`）は
    /// `resolve_warmup_ime_on` 経由（ゲート適用済み）を渡すため `origin=WarmupOrigin::Gated` 固定。
    pub(crate) fn composition_confirm_key_up(
        &mut self,
        vk: awase::types::VkCode,
        warmup_ime_on: awase::platform::WarmupImeOn,
    ) {
        self.feed_composition_event(
            crate::tsf::composition_fsm::CompositionEvent::ConfirmKeyUp { vk },
            warmup_ime_on,
            crate::output::WarmupOrigin::Gated,
        );
    }

    /// Ctrl↑ を `CompositionFsm` に通知し、cold 状態なら warmup を再送する。
    ///
    /// 唯一の呼び出し元（executor の `handle_ctrl_up_recovery`）は
    /// `resolve_warmup_ime_on` 経由（ゲート適用済み）を渡すため `origin=WarmupOrigin::Gated` 固定。
    pub(crate) fn composition_ctrl_up(&mut self, warmup_ime_on: awase::platform::WarmupImeOn) {
        let warm = self.output.is_composition_warm();
        self.feed_composition_event(
            crate::tsf::composition_fsm::CompositionEvent::CtrlUp { warm },
            warmup_ime_on,
            crate::output::WarmupOrigin::Gated,
        );
    }

    /// 物理 F2 (VK_DBE_HIRAGANA) KeyDown を `CompositionFsm` に通知する。
    /// 戻り値 `true` なら物理 F2 を consume すべき（TSF mode、`ConsumeF2` action）。
    ///
    /// 唯一の呼び出し元（`key_pipeline.rs` の物理 F2 down 処理）は
    /// `warmup_ime_on()` 経由（ゲート適用済み）を渡すため `origin=WarmupOrigin::Gated` 固定。
    pub(crate) fn composition_native_f2_down(
        &mut self,
        warmup_ime_on: awase::platform::WarmupImeOn,
    ) -> bool {
        let tsf_mode = self.output.is_tsf_mode();
        let warm = self.output.is_composition_warm();
        self.feed_composition_event(
            crate::tsf::composition_fsm::CompositionEvent::NativeF2Down { tsf_mode, warm },
            warmup_ime_on,
            crate::output::WarmupOrigin::Gated,
        )
    }

    // ── GjiFsm イベント通知 ──────────────────────────────────────────────────

    /// フォーカス変更を GjiFsm に通知する（`ir_post_focus_change_snapshot` から呼ぶ）。
    pub(crate) fn gji_on_focus_change(
        &mut self,
        injection_mode: crate::output::types::InjectionMode,
    ) {
        // CompositionFsm の epoch を進めて、フォーカスを跨いだ保留 warmup を無効化する。
        let tsf_mode = matches!(injection_mode, crate::output::types::InjectionMode::Tsf);
        // `FocusChange` arm は `EmitWarmup` を一切出さない（composition_fsm.rs
        // の当該 match アーム参照）ため、この値は don't-care。`off()` で明示する。
        self.feed_composition_event(
            crate::tsf::composition_fsm::CompositionEvent::FocusChange { tsf_mode },
            awase::platform::WarmupImeOn::off(),
            crate::output::WarmupOrigin::Off,
        );
        let gji_idle_ms = crate::tsf::observer::gji_idle_ms();
        let state_before = self.gji_state_label();
        let resp = self
            .output
            .gji_on_event(crate::tsf::gji_fsm::GjiEvent::FocusChange {
                injection_mode,
                gji_idle_ms,
            });
        self.note_gji_transition(
            format!("FocusChange(gji_idle_ms={gji_idle_ms})"),
            state_before,
        );
        self.dispatch_gji_response(&resp);
        // ImeModeFsm: フォーカス変更で Unknown に戻す（次の IMC 確認待ち）。
        // on_ime_mode_focus_changed が ime_mode_focus_gen をインクリメントするため、
        // spawn_local の前に gen を取得して closure にキャプチャする。
        self.output.on_ime_mode_focus_changed();
        let ime_mode_gen = self.output.ime_mode_focus_gen.get();
        // FocusChange 直後に IMC を 1 回ポーリングして初期状態を Unknown → 実値に更新する。
        // sacr-warmup 開始前から Off/Hiragana が判明するため cold 判定の精度が上がる。
        // with_app 再入を避けるため spawn_local でメインループに戻してから実行する。
        //
        // BUG-59: この読み取りは「TSF compose sink が実際に準備完了しているか」を
        // 確認したものではなく、単なる参考値（cold 判定を早めるためのヒント）。
        // `update_ime_mode_from_imc`（`confirmed=true` を立てる）を使うと、
        // `Output::ms_ime_gate_defer`（BUG-13 の confirm-then-transmit ゲート）が
        // 「安全に送信してよい」と誤認し、フォーカス変更直後の未準備な状態へ
        // romaji を即送信して先頭文字がリテラル化した。`confirmed` を立てない
        // `update_ime_mode_hint_from_imc` を使うこと。
        //
        // ADR-140 Step1b（`/code-review max`指摘）: `kp_stage_idle_conv_check_inner`
        // と全く同型のクロスプロセス conv 読み取りでありながらフェンス対象外
        // だった。hint専用（confirmed を立てない）ため severity は低いが、
        // GJI actuation と交錯した値で cold 判定のヒントが歪むこと自体は
        // 避けられる（`crate::probe_actuation_fence` module doc 参照）。
        let probe_actuation_fence_at_spawn = crate::probe_actuation_fence::current();
        win32_async::spawn_local(async move {
            let outcome = crate::ime::get_ime_conversion_mode_fenced_async(
                50,
                probe_actuation_fence_at_spawn,
            )
            .await;
            let conv = match outcome {
                crate::probe_actuation_fence::FencedProbeOutcome::Abandoned => {
                    tracing::debug!(
                        "[ime-mode] FocusProbe: GJI actuation との交錯を検知 → hint更新をabandon"
                    );
                    return;
                }
                crate::probe_actuation_fence::FencedProbeOutcome::Read(conv) => conv,
            };
            let _ = crate::with_app(|runtime| {
                let current_gen = runtime.platform.output.ime_mode_focus_gen.get();
                if current_gen == ime_mode_gen {
                    runtime.platform.output.update_ime_mode_hint_from_imc(conv);
                } else {
                    tracing::debug!(
                        "[ime-mode] FocusProbe: stale gen={ime_mode_gen} current={current_gen} → skip"
                    );
                }
            });
        });
    }

    /// IME ON を GjiFsm に通知する（`on_ime_applied(open=true)` から呼ぶ）。
    pub(crate) fn gji_on_ime_on(&mut self, injection_mode: crate::output::types::InjectionMode) {
        let gji_idle_ms = crate::tsf::observer::gji_idle_ms();
        let state_before = self.gji_state_label();
        let resp = self
            .output
            .gji_on_event(crate::tsf::gji_fsm::GjiEvent::ImeOn {
                injection_mode,
                gji_idle_ms,
            });
        self.note_gji_transition(format!("ImeOn(gji_idle_ms={gji_idle_ms})"), state_before);
        self.dispatch_gji_response(&resp);
    }

    fn dispatch_gji_event(
        &mut self,
        trigger: impl Into<String>,
        event: crate::tsf::gji_fsm::GjiEvent,
    ) {
        let state_before = self.gji_state_label();
        let resp = self.output.gji_on_event(event);
        self.note_gji_transition(trigger, state_before);
        self.dispatch_gji_response(&resp);
    }

    /// IME OFF を GjiFsm に通知する（`on_ime_applied(open=false)` から呼ぶ）。
    pub(crate) fn gji_on_ime_off(&mut self) {
        self.dispatch_gji_event("ImeOff", crate::tsf::gji_fsm::GjiEvent::ImeOff);
    }

    /// TIMER_GJI_LONG_IDLE ハンドラ。LongIdle タイムアウトを GjiFsm に通知する。
    pub(crate) fn gji_on_timer_long_idle(&mut self) {
        let state_before = self.gji_state_label();
        let resp = self.output.gji_on_long_idle();
        self.note_gji_transition("LongIdleTimeout", state_before);
        self.dispatch_gji_response(&resp);
    }

    /// IME ON/OFF やフォーカス変化なしに composition context が無効化されたことを GjiFsm に通知する。
    ///
    /// `on_passthrough_key` の PassthroughKey / F2NonTsf や
    /// `mark_cold_raw_tsf`（`step_probe` 経由）から呼ぶ。
    pub(crate) fn gji_on_composition_reset(&mut self) {
        // `gji_on_focus_change` と同じパターン: 実測 idle を観測して渡す。
        // GjiFsm 側の `handle_composition_reset` がこれを `ColdKind::classify` に
        // かけ、genuinely warm（Short）なら cold へ倒さず `OnWarm` を維持する
        // （BUG-33 追補3: 弱い代理指標のみで無条件に cold 化していた回帰の修正）。
        let gji_idle_ms = crate::tsf::observer::gji_idle_ms();
        self.dispatch_gji_event(
            format!("CompositionReset(gji_idle_ms={gji_idle_ms})"),
            crate::tsf::gji_fsm::GjiEvent::CompositionReset { gji_idle_ms },
        );
    }

    /// TSF mode で物理 F2 が消費されたことを GjiFsm に通知する（`on_reinject_key` の NativeF2Consumed パス）。
    ///
    /// Medium/Long cold 中は probe が継続（saw_native_f2=true）。Short cold / OnWarm / OnComposing は
    /// CompositionReset 相当として処理される（GjiFsm 側で分岐、`gji_idle_ms` による
    /// 再検証込み）。
    pub(crate) fn gji_on_native_f2_consumed(&mut self) {
        let gji_idle_ms = crate::tsf::observer::gji_idle_ms();
        self.dispatch_gji_event(
            format!("NativeF2Consumed(gji_idle_ms={gji_idle_ms})"),
            crate::tsf::gji_fsm::GjiEvent::NativeF2Consumed { gji_idle_ms },
        );
    }

    /// GJI candidate SHOW → GjiFsm::StartComposition を dispatch する。
    ///
    /// `observation_event_proc` が `pending_start_composition` を set した後、
    /// `advance_tsf_probe` / `send_keys` で `take_pending_start_composition()` が true を返したときに呼ぶ。
    pub(crate) fn gji_on_start_composition(&mut self) {
        tracing::debug!("[gji-fsm] StartComposition (candidate SHOW)");
        self.dispatch_gji_event(
            "StartComposition(candidate SHOW)",
            crate::tsf::gji_fsm::GjiEvent::StartComposition,
        );
    }

    /// GJI candidate HIDE → GjiFsm::EndComposition を dispatch する。
    ///
    /// `observation_event_proc` が `pending_end_composition` を set した後、
    /// `advance_tsf_probe` / `send_keys` で `take_pending_end_composition()` が true を返したときに呼ぶ。
    /// `OnComposing` 以外の状態では epoch が取れないためスキップする（GjiFsm 側でも無視される）。
    pub(crate) fn gji_on_end_composition(&mut self) {
        if let Some(epoch) = self.output.gji_current_composition_epoch() {
            tracing::debug!("[gji-fsm] EndComposition (candidate HIDE) epoch={epoch:?}");
            self.dispatch_gji_event(
                format!("EndComposition(candidate HIDE, epoch={epoch:?})"),
                crate::tsf::gji_fsm::GjiEvent::EndComposition { epoch },
            );
            // BUG-24 追補: 候補ウィンドウ HIDE = IME セッションの終了。次のセッションの
            // 最初の1文字は改めて literal-detect の確認を受けるようリセットする。
            crate::tsf::observer::reset_literal_session_confirmed();
        }
    }

    /// candidate SHOW/HIDE → StartComposition/EndComposition の pending フラグを drain する。
    ///
    /// `advance_tsf_probe` と `send_keys` の末尾で呼ぶ。
    /// StartComposition を先に dispatch してから EndComposition を dispatch する順序を保つ。
    fn drain_pending_composition_events(&mut self) {
        if crate::tsf::observer::take_pending_start_composition() {
            self.gji_on_start_composition();
        }
        if crate::tsf::observer::take_pending_end_composition() {
            self.gji_on_end_composition();
        }
    }

    /// WM_DRAIN_OUTPUT_QUEUE ハンドラ用: raw TSF literal 回収 + probe タイマーをセット。
    ///
    /// `output.flush_raw_tsf_literal_recovery()` は内部で `send_romaji_as_tsf` /
    /// `send_romaji_batched` を呼ぶため、`send_keys()` と同様に `GjiFsm::KeyInput` の
    /// `Response`（`pending_gji_key_responses`）や `composition_reset` フラグが
    /// 発生しうる。`platform.send_keys` を経由しないため、`drain_output_post_send_effects`
    /// で同じ後処理を補完する（BUG-28: これを怠ると `pending_gji_key_responses` が
    /// 次の実 `send_keys()` 呼び出しまで滞留し、溜まった分がまとめて stale な
    /// `StartProbe` として burst 発火する。docs/known-bugs.md 参照）。
    pub fn flush_raw_tsf_literal_recovery(&mut self) {
        let outcome = self.output.flush_raw_tsf_literal_recovery();
        let outcome = match outcome {
            crate::output::RawRecoveryOutcome::DiscardedStale {
                backs,
                romaji_present,
                deferred_vk_count,
            } => crate::journal::DeferredRecoveryOutcomeSummary::DiscardedStale {
                backs,
                romaji_present,
                deferred_vk_count,
            },
            crate::output::RawRecoveryOutcome::SkippedWhilePolling => {
                crate::journal::DeferredRecoveryOutcomeSummary::SkippedWhilePolling
            }
            crate::output::RawRecoveryOutcome::Flushed { vk_count } => {
                crate::journal::DeferredRecoveryOutcomeSummary::Flushed { vk_count }
            }
        };
        let facts = match outcome {
            crate::journal::DeferredRecoveryOutcomeSummary::DiscardedStale { .. } => {
                crate::journal_policy::DeferredRecoveryFlushFacts::DiscardedStale
            }
            crate::journal::DeferredRecoveryOutcomeSummary::SkippedWhilePolling => {
                crate::journal_policy::DeferredRecoveryFlushFacts::SkippedWhilePolling
            }
            crate::journal::DeferredRecoveryOutcomeSummary::Flushed { vk_count } => {
                crate::journal_policy::DeferredRecoveryFlushFacts::Flushed { vk_count }
            }
        };
        if crate::journal_policy::deferred_recovery_flush_is_notable(facts) {
            self.push_journal_entry(crate::journal::JournalEntry::DeferredRecoveryFlush {
                trigger: "raw_recovery",
                outcome,
            });
        }
        self.drain_output_post_send_effects();
    }

    /// `WM_GJI_REINIT_RETRY_COMPLETE` ハンドラから呼ぶ。ADR-101 決定4が要求する
    /// 順序（`Confirmed` の場合）を、この関数の呼び出し順そのものとして固定する:
    /// 1. `resend_gji_reinit_retry_romaji`（retry送信）
    /// 2. `drain_output_post_send_effects`（送信後処理）
    /// 3. `flush_deferred_vks_after_gji_reinit_completion`（deferred flush）
    /// 4. `push_journal_entry(GjiReinitRetryCompleted)`（ADR-123: 診断ログ、
    ///    flush/discard 件数の確定後・guard drop 前に記録する）
    /// 5. `drop(completion.guard)`（関数末尾、`match` の外）
    ///
    /// `completion.guard` は成功/timeout/staleいずれの分岐でも関数末尾で1回だけ
    /// dropする。Win32/`Platform`依存のためLinux上でこの呼び出し順自体をユニット
    /// テストすることはできない（本関数の実装＝この doc コメントの記述が
    /// SSOT。順序を変える場合はここも更新すること）。
    pub(crate) fn complete_gji_reinit_retry(
        &mut self,
        token: u32,
        status: crate::output::GjiReinitPollStatus,
    ) {
        let Some(completion) = self.output.take_gji_reinit_completion(token) else {
            tracing::warn!(
                "[chrome-reinit-retry] completion ignored: token={token} status={status:?}"
            );
            return;
        };
        let current_focus_gen = self.output.current_ime_mode_focus_gen();
        let focus_matches = current_focus_gen == completion.focus_gen;
        tracing::debug!(
            "[chrome-reinit-retry] completion: token={} status={:?} cold={} \
             origin_focus_gen={} current_focus_gen={} retry={}",
            token,
            status,
            completion.cold_seq.value(),
            completion.focus_gen,
            current_focus_gen,
            completion.retry_romaji.is_some(),
        );

        let retry_romaji_present = completion.retry_romaji.is_some();
        let (deferred_flushed, deferred_discarded) = if status
            == crate::output::GjiReinitPollStatus::Confirmed
            && focus_matches
        {
            if let Some(romaji) = completion.retry_romaji {
                self.output
                    .mark_gji_reinit_retry_attempted(completion.focus_gen, romaji.clone());
                self.output.resend_gji_reinit_retry_romaji(&romaji);
                self.drain_output_post_send_effects();
            }
            let flushed = self.output.flush_deferred_vks_after_gji_reinit_completion();
            if flushed > 0 {
                self.drain_output_post_send_effects();
            }
            (flushed, 0)
        } else if focus_matches && status == crate::output::GjiReinitPollStatus::Timeout {
            let flushed = self.output.flush_deferred_vks_after_gji_reinit_completion();
            if flushed > 0 {
                self.drain_output_post_send_effects();
            }
            (flushed, 0)
        } else {
            let discarded = self
                .output
                .discard_pending_deferred_after_stale_gji_reinit();
            tracing::warn!(
                "[chrome-reinit-retry] stale completion: discard_deferred={discarded} token={token} status={status:?}",
            );
            (0, discarded)
        };
        // ADR-123: reinit retry の完了を journal（構造化・容量優先度あり）に
        // 残す。従来は tracing::debug!/tracing::warn! の自由文字列のみで、issue #148
        // の調査時に journal では確認できず app_log_excerpt を直接読む必要が
        // あった。
        self.push_journal_entry(crate::journal::JournalEntry::GjiReinitRetryCompleted {
            token,
            status: format!("{status:?}"),
            cold_seq: completion.cold_seq.value(),
            origin_focus_gen: completion.focus_gen,
            current_focus_gen,
            focus_matches,
            retry_romaji_present,
            deferred_flushed,
            deferred_discarded,
        });
        drop(completion.guard);
    }

    /// `output.send_keys()` / `output.flush_raw_tsf_literal_recovery()` の直後に共通で
    /// 必要な後処理をまとめる（BUG-28）。
    ///
    /// `GjiFsm::KeyInput` の `Response` は `push_key_response` で
    /// `pending_gji_key_responses` に一旦バッファされ、ここで初めて dispatch・ログ出力
    /// （`"[gji-fsm] StartProbe probe_id=..."` 等）される。この関数を呼ばずに
    /// `output.send_keys()`/`output.flush_raw_tsf_literal_recovery()` だけ呼ぶと、
    /// バッファされた `Response` が次にこの関数が呼ばれるまで滞留し続ける。
    fn drain_output_post_send_effects(&mut self) {
        // ADR-128: drain-before-send（`output/vk_send.rs`）が実際に flush した
        // 件数を journal 化する。`JournalStamper::stamp` は push 時に
        // seq/elapsed_ms を採番するため、ここ（全送信直後、`drain_journal_entries`
        // より前）で変換しないと「flush が resend より前に発火した」ことを
        // journal 上で示せず、`GjiReinitRetryCompleted` 等の後続entryより
        // 後ろの seq になってしまう（round: 実装後コードレビュー指摘）。
        //
        // `deferred_recovery_flush_is_notable` の判定はこの呼び出し元
        // （`vk_count` は既に `n > 0` でガード済み）では常に true になるが、
        // 意図的に外していない——ADR-123 変更D が確立した「notability は
        // journal_policy.rs の1箇所だけで判定する」という単一判定点を
        // 崩すと、将来 policy 側の閾値を変えても drain_before_send 側だけ
        // 追随し忘れる退行を招く（/code-review 指摘・検討のうえ据え置き）。
        let vk_count = self.output.take_pending_drain_before_send_flush();
        if vk_count > 0 {
            let facts = crate::journal_policy::DeferredRecoveryFlushFacts::Flushed { vk_count };
            if crate::journal_policy::deferred_recovery_flush_is_notable(facts) {
                self.push_journal_entry(crate::journal::JournalEntry::DeferredRecoveryFlush {
                    trigger: "drain_before_send",
                    outcome: crate::journal::DeferredRecoveryOutcomeSummary::Flushed { vk_count },
                });
            }
        }
        // KeyInput shadow routing: LongIdle タイマーリセット等を処理する。
        // Vec で取り出すのは、1回の送信で複数文字を送る際に全 Response（StartProbe 含む）を
        // 保存するため。Option だと後の文字が前の StartProbe Response を上書きしてしまう。
        for resp in self.output.drain_pending_gji_key_responses() {
            self.dispatch_gji_response(&resp);
        }
        // SymbolVkSent 等の CompositionReset フラグを drain する。
        if self.output.take_composition_reset() {
            self.gji_on_composition_reset();
        }
        // candidate SHOW/HIDE (observation_event_proc) → StartComposition/EndComposition
        self.drain_pending_composition_events();
        // cold-start 時に pending_tsf が設定された場合は 10ms タイマーを起動してプローブを進める。
        if let Some(cmd) = self.output.pending_tsf_timer() {
            self.apply_timer_command(cmd);
        }
    }

    /// `TimerCommand` を受け取り、Win32 タイマー操作を実行する。
    pub(crate) fn apply_timer_command(&mut self, cmd: crate::output::TimerCommand) {
        match cmd {
            crate::output::TimerCommand::Continue { id, delay } => self.timer.set(id, delay),
            crate::output::TimerCommand::Kill { id } => self.timer.kill(id),
        }
    }

    // ── Unicode cold-start warmup ヘルパー ────────────────────────────────

    /// Unicode long-cold warm-up: 飛行中 FSM があれば `deferred` を追記、なければ新規 FSM を生成する。
    ///
    /// `send_keys()` と `dispatch_gji_response()` の両方から呼ぶ共通起点。
    /// 飛行中 FSM への追記に成功した場合は VK_IME_ON / VK_A+BS を再送しない。
    fn start_unicode_cold_warmup(&mut self, cold_seq: Generation, deferred: Vec<char>) {
        if self.output.try_push_unicode_chars_to_pending(&deferred) {
            tracing::debug!(
                "[unicode-cold-warmup] {} chars を飛行中 FSM に追記 (新規 FSM/VK_A+BS 送信スキップ)",
                deferred.len()
            );
            return;
        }
        let baseline = crate::tsf::observer::gji_write_bytes();
        self.output.send_unicode_cold_warmup_keys(cold_seq);
        tracing::info!(
            "[unicode-cold-warmup] cold={cold_seq} long-cold Unicode warm-up: \
             VK_IME_ON+VK_A+BS → {} chars defer",
            deferred.len(),
            cold_seq = cold_seq.value(),
        );
        let fsm = crate::tsf::warmup::unicode_cold_warmup_fsm::UnicodeColdWarmupFsm::new(
            cold_seq, deferred, baseline,
        );
        self.install_pending_tsf_and_set_timer(Box::new(fsm));
    }

    /// `output` の Unicode cold deferred chars を取り出し、warm-up FSM を起動する。
    ///
    /// `send_keys()` の Unicode cold-start パスで `output.send_keys()` の直後に呼ぶ。
    /// deferred が空なら何もしない。
    fn flush_unicode_cold_deferred_chars(&mut self) {
        let deferred = self.output.take_unicode_cold_deferred();
        if deferred.is_empty() {
            return;
        }
        let cold_seq = self.output.composition.cold_start_count();
        self.start_unicode_cold_warmup(cold_seq, deferred);
    }
}

impl PlatformRuntime for WindowsPlatform {
    // ── キー出力 ──

    fn send_keys(&mut self, actions: &[KeyAction]) {
        // Unicode モード + 未学習クラスなら、Romaji 送信後に GJI write 観測をリクエストする（事後昇格）。
        if self.output.injection_mode == crate::output::InjectionMode::Unicode
            && !self
                .focus
                .has_learned_injection_mode_tsf(self.focus.class_name())
        {
            self.output.request_unicode_observation();
        }
        // Unicode cold-start warmup: GjiFsm が long cold のとき chars を defer する。
        //
        // Unicode モードでは send_romaji_as_unicode() が GjiFsm::KeyInput を発行しないため
        // GjiFsm が StartProbe を emit することがない。そのため dispatch_gji_response() を
        // 経由せず、ここで直接 FSM をインストールする。
        //
        // defer は Char/Romaji のみが対象（`send_unicode_char` 経由）。CtrlChord/Key/
        // KeyUp/SpecialKey は injector を直接叩き defer をバイパスし、送信ループ内で
        // 即座に実行される。defer された Char/Romaji はループ完了後（send_keys 呼び出し
        // 全体が終わった後）にまとめて flush されるため、バッチ内で「Char/Romaji の後に
        // 非defer対象が続く」形（例: ADR-115 打鍵列 `'（'+CV4D+'）'+CV4D+左` の
        // Char→CtrlChord→Char→CtrlChord→Special）だと、後続の非defer対象がまだ
        // バッファに残っている Char より先に実行され、実行順序が入れ替わる
        // （Opus実装後レビュー M1 で発見）。
        //
        // 逆に「非defer対象が先、Char/Romaji が後」の形（例: retract_and_replace が
        // 出す `[Backspace, Char]`、ADR-115 以前から存在する）は安全——Backspace は
        // 即座に実行され、Char は後で flush されても元の順序どおり
        // Backspace→Char のまま変わらない。この安全な既存パターンまで一律で defer を
        // 諦めると、GJI long-cold 時の warmup 保護（BUG-02 系文字化けの再発防止）が
        // ADR-115 と無関係な既存経路にまで及んでしまう
        // （初回のM1修正が過剰に広かった、との実装後レビュー2件目で発見・訂正）。
        // 「Char/Romaji の後に非defer対象が続かない」ことだけを判定する。
        //
        // 走査自体は Unicode モードのときだけ行う（`&&` の短絡評価で非Unicodeモードでは
        // スキップする、効率面の実装後レビュー指摘）。
        let needs_unicode_cold_warmup = self.output.injection_mode
            == crate::output::InjectionMode::Unicode
            && {
                let mut seen_deferrable = false;
                actions.iter().all(|a| {
                    if matches!(a, KeyAction::Char(_) | KeyAction::Romaji(_)) {
                        seen_deferrable = true;
                        true
                    } else {
                        !seen_deferrable
                    }
                })
            }
            && self.output.gji_is_next_key_long_cold();
        if needs_unicode_cold_warmup {
            self.output.set_unicode_cold_defer(true);
        }
        self.output.send_keys(actions);
        if needs_unicode_cold_warmup {
            self.output.set_unicode_cold_defer(false);
            self.flush_unicode_cold_deferred_chars();
        }
        self.drain_output_post_send_effects();
    }

    fn reinject_key(&mut self, event: &RawKeyEvent) {
        use crate::RawKeyEventExt as _;
        unsafe { event.reinject() };
    }

    // ── タイマー ──

    fn set_timer(&mut self, id: usize, duration: Duration) {
        self.timer.set(id, duration);
    }

    fn kill_timer(&mut self, id: usize) {
        self.timer.kill(id);
    }

    // ── IME ──

    fn set_ime_open(&mut self, open: bool) -> bool {
        // IMM32 API で直接 open/close できないアプリ（Imm32Unavailable / TSF-native）では
        // get_gui_thread_info + send_ime_control が ~200ms タイムアウトしてブロックする。
        // 早期 return して IMM32 経由のクロスプロセス呼び出しをスキップする。
        if !self.current_app_profile().can_use_imm32_cross_process() {
            return false;
        }
        // `set_ime_open_cross_process` は SendMessageTimeoutW を含むため、メインスレッドで
        // 同期実行すると `with_app` 再入トリガーになる。ワーカースレッドに offload する
        // async ラッパーを spawn_local で fire-and-forget する。
        // 戻り値の semantics は「dispatch 成功」(= profile 互換) に変更。実際の SendMessage
        // 結果は呼び出し側に届かない（旧 API の sync bool に依存していた診断ログは廃止）。
        win32_async::spawn_local(async move {
            let _ = crate::ime::set_ime_open_cross_process_async(open).await;
        });
        true
    }

    fn post_ime_refresh(&mut self) {
        // SetOpen 後の IME 状態反映に数十ms かかるため、即時ではなく
        // 統合タイマー経由で短い遅延後にリフレッシュする。
        // guard が active なら後続キーはバッファされるので安全。
        self.timer
            .set(crate::TIMER_IME_REFRESH, Duration::from_millis(20));
    }

    // ── Engine 状態変化時 IME モードキー送信 ──

    fn send_engine_state_ime_key(&self, enabled: bool, applied: Option<bool>) {
        if self.suppress_engine_state_key {
            // ポーリング/フォーカス変化起因の遷移では VK を送らない。
            // 送ると IME 状態が変わり → 次のポーリングでエンジンが逆転 → 無限ループになる。
            tracing::debug!(
                "[engine-state-key] suppressed (polling/focus-triggered, enabled={enabled})"
            );
            return;
        }
        // apply_ime_open（VK_KANJI or IMM クロスプロセス）が既に IME 状態を確定させている場合、
        // 追加の mode key 送信は不要かつ有害。MS-IME は IME 閉時に VK_DBE_SBCSCHAR を受け取ると
        // 半角英数モードで再オープンする挙動があり、Engine OFF / 実 IME ON の乖離を引き起こす。
        //
        // mode key 送信の本来の用途は「Engine 状態は変わったが IME open/close は変わらない」
        // ケース（例: user_enabled トグルで IME はそのまま）に限定する。
        let last_applied = applied.unwrap_or(false);
        if last_applied == enabled {
            tracing::debug!(
                "[engine-state-key] skipped (apply_ime_open aligned ime={enabled}, profile={:?})",
                self.current_app_profile()
            );
            return;
        }
        // VK_KANJI トグルで IME を制御するアプリ（Imm32Unavailable: Chrome/Edge）では
        // apply_ime_open が既に VK_KANJI を送信済み。VK_DBE_SBCSCHAR/DBCSCHAR を追加送信すると:
        //   OFF 時: VK_KANJI でクローズ直後に VK_DBE_SBCSCHAR が IME を再オープンする恐れがある。
        //   ON 時: VK_KANJI で開いた後に VK_DBE_DBCSCHAR を送ると全角カタカナモードになりかねない。
        let profile = self.current_app_profile();
        if profile.uses_kanji_toggle() {
            tracing::debug!("[engine-state-key] skipped (profile={profile:?}, VK_KANJI済み)");
            return;
        }
        let vk = if enabled {
            self.engine_on_ime_vk
        } else {
            self.engine_off_ime_vk
        };
        if let Some(vk) = vk {
            // Win キー押下中スキップ時は on_ime_mode_vk_sent も呼ばない
            // （送っていないキーで ime_mode_fsm の belief を動かさない）。
            if unsafe { crate::ime::send_ime_mode_key(vk) } {
                self.output.on_ime_mode_vk_sent(vk);
            }
        }
    }

    // ── トレイ ──

    fn update_tray(&mut self, enabled: bool) {
        self.tray.set_enabled(enabled);
    }

    fn show_balloon(&mut self, title: &str, message: &str) {
        self.tray.show_balloon(title, message);
    }

    fn set_tray_layout_name(&mut self, name: &str) {
        self.tray.set_layout_name(name);
    }
}

/// ADR-089 §2.4（INV-42）: `GjiFsm` 同期義務の実行口。
///
/// ungated 側（`state/gji_direct_mechanism.rs`）は `GjiFsm` に依存できないため
/// （ADR-065）、同期義務は `GjiFsmSync` という値で受け取り、実 FSM への写像を
/// ここだけが行う。1 回の同期は `output.gji_on_event(..)` が返す
/// `Response<GjiAction, GjiTimer>` を `dispatch_gji_response` へ流すところまでを
/// 含むため、`&mut GjiFsm` ではなく `&mut WindowsPlatform` が要る（§1.3(f)）。
impl crate::state::gji_direct_mechanism::GjiSyncSink for WindowsPlatform {
    fn sync_gji(&mut self, sync: crate::state::gji_direct_mechanism::GjiFsmSync) {
        use crate::state::gji_direct_mechanism::GjiFsmSync;
        match sync {
            GjiFsmSync::OnImeOn => {
                // settle 時点の値を読む（receipt 生成時に積むと、actuation 中に
                // mode が変わった場合に古い値で同期する。ADR-089 §2.4 細目2）。
                let mode = self.output.injection_mode;
                self.gji_on_ime_on(mode);
            }
            GjiFsmSync::OnImeOff => self.gji_on_ime_off(),
        }
    }
}

impl TsfComposition for WindowsPlatform {
    fn composition_output(&self) -> Option<&dyn awase::platform::CompositionOutput> {
        Some(&self.output)
    }

    fn output_in_flight_ms(&self) -> u64 {
        self.output.ms_since_last_send()
    }

    fn is_composition_warm(&self) -> bool {
        self.output.is_composition_warm()
    }

    fn is_tsf_mode(&self) -> bool {
        self.output.is_tsf_mode()
    }

    fn on_ime_applied(&mut self, open: bool, outcome: awase::platform::ImeOpenOutcome) {
        use awase::platform::ImeOpenOutcome;
        // ADR-089 §2.4（INV-42/43）: `GjiFsm` 同期義務を `ActuationReceipt` として
        // 明示的に運ぶ。同期の要否を決める式は
        // `state/gji_direct_mechanism.rs::legacy_gji_sync_obligation` ただ 1 箇所で
        // あり（`outcome != UnsafeToToggle`）、ここに条件を書き足してはならない
        // ——profile 軸でも K 軸でもゲートしないことが INV-42 の本体である。
        //
        // receipt は **この呼び出しフレームのローカル値**として持ち、同じフレームで
        // settle する。`WindowsPlatform` のフィールドに持たせると
        // `receipt.settle(&mut self)` が `&mut self` からの二重可変借用になる
        // （ADR-089 §2.4 細目3）。
        let mut receipt = crate::state::gji_direct_mechanism::ActuationReceipt::new(open, outcome);
        // UnsafeToToggle: 送信しなかったので何もしない（executor 側で早期リターン済みだが念のため）
        if matches!(
            outcome,
            ImeOpenOutcome::UnsafeToToggle | ImeOpenOutcome::NotOwned
        ) {
            // 同期義務は無い（`legacy_gji_sync_obligation` が `None`）が、
            // settle 済みにしないと `Drop` の `debug_assert` が発火する。
            receipt.settle(self);
            return;
        }
        let effective = match outcome {
            ImeOpenOutcome::Applied
            | ImeOpenOutcome::FallbackSent
            | ImeOpenOutcome::AlreadyMatched => open,
            ImeOpenOutcome::Failed => !open,
            ImeOpenOutcome::UnsafeToToggle | ImeOpenOutcome::NotOwned => unreachable!(),
        };
        // IME 状態が変化したので GJI 候補ウィンドウの「見た」フラグをリセットする。
        // これをリセットしないと次の composition 検出で desync と誤判定される。
        crate::tsf::observer::reset_candidate_was_seen();
        // ImeModeFsm belief 更新（BUG-13）: 実際に適用が走った場合のみ unconfirmed 化する。
        // MsImeDirect は VK_IME_ON/OFF を送らず on_ime_mode_vk_sent を経由しないため、
        // ここが唯一の invalidate 点。これにより IME ON 遷移直後の送信が
        // ms_ime_gate_defer で IMC 確認を待つようになる。
        // AlreadyMatched は状態不変（確認済み belief を降格させない）、Failed は
        // 実状態が不明のため belief を汚さない。
        if matches!(
            outcome,
            ImeOpenOutcome::Applied | ImeOpenOutcome::FallbackSent
        ) {
            self.output
                .ime_mode_fsm
                .borrow_mut()
                .on_set_open_applied(open);
            if open {
                // 新しい IME ON 試行 → give-up latch を解除して再確認の機会を与える。
                self.output.ms_ime_gate_give_up.set(false);
                // pass-5 レビュー指摘（should-fix）: SetOpen(true) が適用された
                // ということは IME が OFF→ON へサイクルしたということであり、
                // `shift-conv-guard` の hold が前提としていた「conv=0x0000 を
                // 書いた直後」という状態はもはや成立しない（IME が一度 OFF に
                // なった時点で conv の意味が失われている）。`on_ime_mode_focus_changed`
                // と対称に、confirm-gate の override と所有権世代を併せて
                // クリア・無効化し、最大 `SHIFT_CONV_GUARD_ENTRY_SUSPEND_CAP_MS`
                // 分の猶予が無関係な後続の送信に残留しないようにする。
                self.output.confirm_gate_deadline_override_ms.set(0);
                self.output.bump_shift_conv_guard_gen();
            }
        }
        // CompositionFsm の状態を IME ON/OFF に追従させる（保留 warmup の epoch 整合用）。
        let tsf_mode = self.output.is_tsf_mode();
        let comp_event = if open {
            crate::tsf::composition_fsm::CompositionEvent::ImeOn { tsf_mode }
        } else {
            crate::tsf::composition_fsm::CompositionEvent::ImeOff
        };
        // BUG-110/ADR-132 Phase 2 敵対的コードレビュー指摘: この `warmup_ime_on` は
        // `from_actuated`（実 actuation 直後の確定値）由来であり、`resolve_warmup_ime_on`
        // が課す `off_drift_active` ゲートを通らない——force-ON
        // （`apply_force_on_for_imm_broken`）が `SetOpen(true)` を適用した直後にも
        // ここを通るため、drift correction が OFF 方向へ送り続けている最中でも
        // 随伴 warmup（`VK_IME_ON`）が飛びうる。INV-B1'（`send_eager_tsf_warmup` が
        // `VK_IME_ON` を送信する瞬間 OFF 方向 drift は検出されていない）は
        // **この経路には及ばない**、既知の限界（ADR-132「Phase 2」節参照）。
        // `origin=WarmupOrigin::Actuated` を付け、次回実機報告でゲート対象（Gated）の
        // warmup と区別できるようにする。
        let warmup_ime_on = awase::platform::WarmupImeOn::from_actuated(effective);
        self.feed_composition_event(
            comp_event,
            warmup_ime_on,
            crate::output::WarmupOrigin::Actuated,
        );
        if open {
            tracing::debug!("[composition] ImeEffect::SetOpen(true) → marking cold");
            self.output
                .mark_composition_cold(crate::output::ColdReason::SetOpenTrue);
            // `injection_mode` は receipt にも settle の引数にも積まない。
            // `sync_gji` の実装内で settle 時点の値を読む（ADR-089 §2.4 細目2）。
            receipt.settle(self);
            self.output
                .send_eager_tsf_warmup(warmup_ime_on, crate::output::WarmupOrigin::Actuated);
        } else {
            tracing::debug!("[composition] ImeEffect::SetOpen(false) → marking cold (prevent warm+TSF Enter leak)");
            self.output
                .mark_composition_cold(crate::output::ColdReason::SetOpenFalse);
            receipt.settle(self);
        }
    }

    fn on_passthrough_key(
        &mut self,
        vk: awase::types::VkCode,
        is_keydown: bool,
        warmup_ime_on: awase::platform::WarmupImeOn,
    ) -> bool {
        use crate::tsf::composition_fsm::CompositionEvent;
        use crate::vk::VkCodeExt as _;

        // confirm キー KeyDown を CompositionFsm に委譲する。
        // FSM が cold mark / GJI reset / warmup 送信 を action として返し dispatcher が実行する。
        // warm+TSF では warmup を KeyUp まで遅延し PendingWarmupOnKeyUp に入るので、
        // その有無を deferral 戻り値とする。
        // （物理 F2 は composition_native_f2_down を直接呼ぶ別経路で処理する。）
        if is_keydown && vk.is_composition_confirm_key() {
            let tsf_mode = self.output.is_tsf_mode();
            let warm = self.output.is_composition_warm();
            // 呼び出し元（executor の `handle_confirm_key_passthrough`）は
            // `resolve_warmup_ime_on` 経由（ゲート適用済み）を渡す。
            self.feed_composition_event(
                CompositionEvent::ConfirmKeyDown { vk, tsf_mode, warm },
                warmup_ime_on,
                crate::output::WarmupOrigin::Gated,
            );
            return self.composition_fsm.pending_warmup_vk() == Some(vk);
        }
        false
    }

    /// 呼び出し元（executor の `handle_reinject`）は `resolve_warmup_ime_on` 経由
    /// （ゲート適用済み）を渡すため、以下2箇所の `send_eager_tsf_warmup` は
    /// いずれも `origin=WarmupOrigin::Gated` 固定。
    fn on_reinject_key(
        &mut self,
        vk: awase::types::VkCode,
        is_keydown: bool,
        warmup_ime_on: awase::platform::WarmupImeOn,
    ) {
        use crate::vk::VkCodeExt as _;

        if vk == crate::vk::VK_DBE_HIRAGANA && is_keydown && self.output.is_tsf_mode() {
            tracing::debug!(
                "[reinject-tsf] vk=0xf2 KeyDown TSF mode → marking cold (NativeF2Consumed)",
            );
            self.output
                .mark_composition_cold(crate::output::ColdReason::NativeF2Consumed);
            self.gji_on_native_f2_consumed();
            // conv mutation の可否は send_eager_tsf_warmup が conv_mutation_allowed で self-gate する。
            self.output
                .send_eager_tsf_warmup(warmup_ime_on, crate::output::WarmupOrigin::Gated);
            return;
        }

        if is_keydown && vk.is_composition_confirm_key() {
            // 2026-07-11: この confirm キーは on_passthrough_key で既に一度処理済みの
            // 同じ物理キーイベントが reinject/defer キューを経由して再度届いたもの。
            // warm であれば（composition_fsm.rs の ConfirmKeyDown と同じ理由で）
            // cold 化・GJI reset とも不要 — 何もしないと BUG-24 系の false positive
            // （不要な BS）の温床になっていた連続 typing 中の余分な cold 化を防げる。
            if self.output.is_composition_warm() {
                tracing::trace!(
                    "[composition] reinject KeyDown vk={vk:#04x} warm → cold化スキップ"
                );
                return;
            }
            tracing::debug!(
                "[composition] reinject KeyDown vk={vk:#04x} → marking cold + eager warmup",
            );
            self.output
                .mark_composition_cold(crate::output::ColdReason::ReinjectConfirmKey);
            self.gji_on_composition_reset();
            // conv mutation の可否は send_eager_tsf_warmup が conv_mutation_allowed で self-gate する。
            self.output
                .send_eager_tsf_warmup(warmup_ime_on, crate::output::WarmupOrigin::Gated);
        }
    }
}

impl WindowsPlatform {
    /// `apply_ime_open` 用の `ImeControlView` を構築する。
    ///
    /// `applied` には呼び出し元が持つ `ImeModel.applied_pair()` の戻り値を渡す。
    /// `None`（未適用・`AppliedImeState::Unknown`）は `ControlLog.shadow_on`
    /// の `None`（未知）へそのまま伝播する——`Some(false)`（確認済み OFF）
    /// と潰して混同してはならない（BUG-113 Blocker、docs/known-bugs.md 参照）。
    #[tracing::instrument(level = "debug", skip_all, fields(?applied))]
    pub(crate) fn build_ime_control_view(
        &self,
        applied: Option<(bool, u64)>,
    ) -> crate::state::ImeControlView<'_> {
        let class_name = if self.focus.is_focused() {
            self.focus.class_name()
        } else {
            ""
        };
        let shadow_on = applied.map(|(open, _applied_at_ms)| open);
        crate::state::ImeControlView {
            focus: crate::state::FocusFacts {
                class_name,
                profile: self.current_app_profile(),
                // ADR-089 §6 Phase C item 12: 同期 ROMAN 補完（ADR-086 INV-14）の
                // `ActuationTarget` 照合基準。`executor.rs::dispatch_ime_set_open` の
                // async 経路が `ActuationTarget::capture(focus_gen)` に渡すのと同じ値。
                focus_gen: self.output.ime_mode_focus_gen.get(),
            },
            observed: crate::state::ObservedState::from_snapshot(crate::tsf::observer::tsf_obs()),
            control: crate::state::ControlLog { shadow_on },
            belief_input_mode: awase::engine::InputModeState::Unknown,
        }
    }

    /// 事前構築済みの `ImeControlView` と `OpenBelief` を受け取る中核実装。
    ///
    /// `tsf_obs()` の重複呼び出しを避けるため view は呼び出し元が一度だけ構築して渡す。
    /// 戦略選択と実行は [`crate::ime_controller::ImeController`] が唯一の SSOT として担う。
    /// `belief` は診断ログ用（`effective_open` / `confident`）に受け取る。
    // 兄弟メソッド apply_ime_open_with_belief から `self.` 記法で呼ばれるため、
    // また PlatformRuntime 委譲メソッド群との一貫した API 配置のため `&self` を維持する。
    #[allow(clippy::unused_self)]
    pub(crate) fn apply_ime_open_with_view(
        &self,
        order: crate::state::actuation_chain::ActuationOrder,
        view: &crate::state::ImeControlView<'_>,
        belief: crate::output::OpenBelief,
    ) -> awase::platform::ImeOpenOutcome {
        let open = order.open();
        let outcome = crate::ime_controller::ImeController::apply(order, view);
        tracing::debug!(
            "[apply-ime] open={open} eff={} conf={} → outcome={outcome:?}",
            belief.effective_open,
            belief.confident
        );
        outcome
    }

    /// `applied` から view を構築して [`Self::apply_ime_open_with_view`] に委譲する。
    ///
    /// 呼び出し元が view を持たない場合（refresh / probe 完了後等）のラッパー。
    pub(crate) fn apply_ime_open_with_belief(
        &self,
        order: crate::state::actuation_chain::ActuationOrder,
        applied: Option<(bool, u64)>,
        belief: crate::output::OpenBelief,
    ) -> awase::platform::ImeOpenOutcome {
        let view = self.build_ime_control_view(applied);
        self.apply_ime_open_with_view(order, &view, belief)
    }

    /// `set_ime_open`（トレイトメソッド）の `ActuationOrder` 版
    /// （ADR-090 §2.A 設計案 3、§6 ステップ 5 item 20）。
    ///
    /// # なぜトレイトメソッドではなくこちらを使うのか
    ///
    /// `PlatformRuntime::set_ime_open(&mut self, open: bool) -> bool`
    /// （`src/platform.rs` の**トレイト定義**）には引数を足せない。実 actuation
    /// 入口 8 つのうち 2 つ（`ime_refresh.rs` の focus change 強制 OFF と
    /// drift correction の ImmCross 分岐）がそのトレイトメソッドを通っていたため、
    /// `WindowsPlatform` の inherent メソッドとして `ActuationOrder` を受ける
    /// 版を足し、そちらへ移した。**トレイトメソッド側は呼び出し元ゼロの
    /// 死んだ入口になる**（`ime_open_actuation_entry_points_are_accounted_for`
    /// が `.set_ime_open(` の本番呼び出し 0 件を固定する）。
    ///
    /// A-1 は shadow モードなので、授権が下りていなくても書き込みは止めない。
    pub(crate) fn set_ime_open_ordered(
        &mut self,
        order: crate::state::actuation_chain::ActuationOrder,
    ) -> bool {
        crate::ime_controller::log_shadow_warrant("set_ime_open", &order);
        let open = order.open();
        // `order` は**値で**受け取り、ここで消費する。1 つの `ActuationOrder`
        // = 高々 1 回の write という `Actuation` のアフィン性（ADR-089 INV-41）を、
        // チェーンを通らないこの経路でも保つため——参照で受けると同じ order で
        // 2 回書けてしまう。
        drop(order);
        PlatformRuntime::set_ime_open(self, open)
    }

    // ── タイマー問い合わせ ──

    /// エンジンの親指シフト FSM タイマー（PENDING / SPECULATIVE）が活性かどうかを返す。
    ///
    /// タイピング中はフォーカス分類をスキップするためのガード判定に使用する。
    /// タイマー ID の詳細を focus 層に露出しないためのカプセル化。
    #[must_use]
    pub fn is_engine_processing(&self) -> bool {
        use awase::engine::{TIMER_PENDING, TIMER_SPECULATIVE};
        self.timer.is_active(TIMER_PENDING) || self.timer.is_active(TIMER_SPECULATIVE)
    }

    // ── フォーカス委譲メソッド ──

    /// フォーカス中アプリの IME 制御プロファイルを返す。
    #[must_use]
    pub const fn current_app_profile(&self) -> AppImeProfile {
        self.focus.current_profile()
    }

    /// 現在のフォーカス先に対する注入ヒントを返す。
    #[must_use]
    pub fn injection_hint(&self) -> InjectionHint {
        self.focus.injection_hint()
    }

    /// 指定した pid/class に対する injection_hint を返す（フォーカス変更直後の stale 回避用）。
    #[must_use]
    pub(crate) fn injection_hint_for(&self, pid: u32, class_name: &str) -> InjectionHint {
        self.focus.injection_hint_for(pid, class_name)
    }

    /// フォーカス情報と `AppImeProfile` キャッシュをアトミックに更新する。
    pub fn update_focus_info(&mut self, process_id: u32, class_name: String, hwnd: usize) {
        self.focus.update(process_id, class_name, hwnd);
    }

    /// 同一フォーカスプローブ内で取得済みの process_name を再利用して更新する。
    pub fn update_focus_info_with_process_name(
        &mut self,
        process_id: u32,
        class_name: String,
        hwnd: usize,
        process_name: Option<String>,
    ) {
        self.focus
            .update_with_process_name(process_id, class_name, hwnd, process_name);
    }

    /// IMM 能力キャッシュに学習結果を追加し、ファイルに永続化する。
    pub fn learn_imm_capability(
        &mut self,
        process_name: String,
        class_name: String,
        cap: ImmCapability,
    ) {
        self.focus
            .learn_imm_capability(process_name, class_name, cap);
    }

    /// `ImmGetDefaultIMEWnd`=NULL の観測を記録する（BUG-56: 閾値回連続で初めて確定）。
    pub fn record_imm_null_probe(&mut self, process_name: String, class_name: String) {
        self.focus.record_imm_null_probe(process_name, class_name);
    }

    /// 非 NULL 観測を得たら「疑い」カウントをクリアする（BUG-56）。
    pub fn clear_imm_pending_unavailable(&mut self, process_name: &str, class_name: &str) {
        self.focus
            .clear_imm_pending_unavailable(process_name, class_name);
    }

    /// UIA ワーカーへの送信チャネルを設定する。
    pub fn set_uia_sender(
        &mut self,
        sender: std::sync::mpsc::Sender<crate::focus::uia::SendableHwnd>,
    ) {
        self.focus.set_uia_sender(sender);
    }
}
