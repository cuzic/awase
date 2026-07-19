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
        }
    }

    // ── Output 委譲メソッド ──────────────────────────────────────────────────

    /// `applied_ime_on` を指定して eager warmup を送信する。
    pub(crate) fn send_eager_warmup(&self, applied_ime_on: Option<bool>) {
        self.output.send_eager_tsf_warmup(applied_ime_on);
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

    /// Ctrl+key パススルー時の composition キャンセル内部状態更新。
    ///
    /// IMM32 の `cancel_ime_composition()` を呼んだ直後に続けて呼ぶこと。
    pub(crate) fn on_ctrl_bypass_composition_cancel(&mut self) {
        self.output
            .mark_composition_cold(crate::output::ColdReason::CtrlKeyBypass);
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
        let result = self.output.step_probe();
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
            log::info!("[injection-mode] {class_name:?} → Tsf 事後昇格（GJI write 未観測）");
            self.focus.learn_injection_mode_tsf(class_name);
            // 現セッション（現在のフォーカスウィンドウ）にも即時 Tsf モードを適用する。
            self.output
                .update_injection_mode(crate::output::InjectionMode::Tsf);
            // 次の文字送信が cold-start TSF probe を正しく踏むよう composition を cold にリセット。
            self.output
                .mark_composition_cold(crate::output::ColdReason::FocusChange);
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
                    log::debug!(
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
                GjiAction::StartProbe {
                    probe_id,
                    budget_ms,
                    params,
                } => {
                    log::debug!(
                        "[gji-fsm] StartProbe probe_id={probe_id:?} budget={budget_ms}ms \
                         forces_f2={} long={}",
                        params.forces_prepend_f2,
                        params.is_long_cold
                    );
                    self.output.gji_store_probe_id(*probe_id);
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
                                log::debug!(
                                    "[gji-fsm] Unicode long-cold StartProbe: VK_IME_OFF→VK_IME_ON reinit (chars なし)"
                                );
                                self.output.send_f22_f21_reinit();
                            } else {
                                self.start_unicode_cold_warmup(probe_id.0, deferred);
                            }
                        }
                        let warmup_resp = self.output.gji_on_event(GjiEvent::WarmupComplete {
                            probe_id: *probe_id,
                        });
                        self.dispatch_gji_response(&warmup_resp);
                    }
                }
                GjiAction::CancelProbe { probe_id } => {
                    if self.output.gji_current_probe_id() == Some(*probe_id) {
                        log::debug!("[gji-fsm] CancelProbe probe_id={probe_id:?}");
                        // pending_tsf / OUTPUT_GATE ガード / probe_id を一括キャンセルする。
                        self.output.cancel_probe();
                        self.timer.kill(crate::TIMER_TSF_PROBE);
                    }
                }
                // 実際の送信は Output が担うため FSM の SendInput/SendInputDirect は無視する。
                GjiAction::SendInput { .. } | GjiAction::SendInputDirect(..) => {}
            }
        }
    }

    // ── CompositionFsm ディスパッチャ ─────────────────────────────────────────

    /// `CompositionFsm` の `Response` を処理し、warmup 送信・cold mark・GJI reset を実行する。
    ///
    /// `applied_ime_on` は `EmitWarmup` の送信先 IME 状態。戻り値は F2 を consume すべきか
    /// （`ConsumeF2` アクションの有無）で、TSF mode で物理 F2 を swallow する判断に使う。
    fn dispatch_composition_response(
        &mut self,
        response: &timed_fsm::Response<
            crate::tsf::composition_fsm::CompositionAction,
            std::convert::Infallible,
        >,
        applied_ime_on: Option<bool>,
    ) -> bool {
        use crate::tsf::composition_fsm::CompositionAction;
        let mut consume_f2 = false;
        for action in &response.actions {
            match *action {
                CompositionAction::EmitWarmup { reason } => {
                    log::debug!("[composition-fsm] EmitWarmup ({reason:?})");
                    // conv mutation の可否は Output::send_eager_tsf_warmup が
                    // `conv_mutation_allowed` で self-gate する（non-AwaseOwned なら内部で skip）。
                    self.output.send_eager_tsf_warmup(applied_ime_on);
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
        applied_ime_on: Option<bool>,
    ) -> bool {
        use timed_fsm::TimedStateMachine;
        let response = self.composition_fsm.on_event(event);
        let consume_f2 = self.dispatch_composition_response(&response, applied_ime_on);
        log::trace!(
            "[composition-fsm] state={}",
            self.composition_fsm.state_label()
        );
        consume_f2
    }

    /// confirm キー KeyUp を `CompositionFsm` に通知し、保留 warmup があれば送信する。
    pub(crate) fn composition_confirm_key_up(
        &mut self,
        vk: awase::types::VkCode,
        applied_ime_on: Option<bool>,
    ) {
        self.feed_composition_event(
            crate::tsf::composition_fsm::CompositionEvent::ConfirmKeyUp { vk },
            applied_ime_on,
        );
    }

    /// Ctrl↑ を `CompositionFsm` に通知し、cold 状態なら warmup を再送する。
    pub(crate) fn composition_ctrl_up(&mut self, applied_ime_on: Option<bool>) {
        let warm = self.output.is_composition_warm();
        self.feed_composition_event(
            crate::tsf::composition_fsm::CompositionEvent::CtrlUp { warm },
            applied_ime_on,
        );
    }

    /// 物理 F2 (VK_DBE_HIRAGANA) KeyDown を `CompositionFsm` に通知する。
    /// 戻り値 `true` なら物理 F2 を consume すべき（TSF mode、`ConsumeF2` action）。
    pub(crate) fn composition_native_f2_down(&mut self, applied_ime_on: Option<bool>) -> bool {
        let tsf_mode = self.output.is_tsf_mode();
        self.feed_composition_event(
            crate::tsf::composition_fsm::CompositionEvent::NativeF2Down { tsf_mode },
            applied_ime_on,
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
        self.feed_composition_event(
            crate::tsf::composition_fsm::CompositionEvent::FocusChange { tsf_mode },
            None,
        );
        let gji_idle_ms = crate::tsf::observer::gji_idle_ms();
        let resp = self
            .output
            .gji_on_event(crate::tsf::gji_fsm::GjiEvent::FocusChange {
                injection_mode,
                gji_idle_ms,
            });
        self.dispatch_gji_response(&resp);
        // ImeModeFsm: フォーカス変更で Unknown に戻す（次の IMC 確認待ち）。
        // on_ime_mode_focus_changed が ime_mode_focus_gen をインクリメントするため、
        // spawn_local の前に gen を取得して closure にキャプチャする。
        self.output.on_ime_mode_focus_changed();
        let ime_mode_gen = self.output.ime_mode_focus_gen.get();
        // FocusChange 直後に IMC を 1 回ポーリングして初期状態を Unknown → 実値に更新する。
        // sacr-warmup 開始前から Off/Hiragana が判明するため cold 判定の精度が上がる。
        // with_app 再入を避けるため spawn_local でメインループに戻してから実行する。
        win32_async::spawn_local(async move {
            let conv = crate::ime::get_ime_conversion_mode_raw_timeout_async(50).await;
            let _ = crate::with_app(|runtime| {
                let current_gen = runtime.platform.output.ime_mode_focus_gen.get();
                if current_gen == ime_mode_gen {
                    runtime.platform.output.update_ime_mode_from_imc(conv);
                } else {
                    log::debug!(
                        "[ime-mode] FocusProbe: stale gen={ime_mode_gen} current={current_gen} → skip"
                    );
                }
            });
        });
    }

    /// IME ON を GjiFsm に通知する（`on_ime_applied(open=true)` から呼ぶ）。
    pub(crate) fn gji_on_ime_on(&mut self, injection_mode: crate::output::types::InjectionMode) {
        let gji_idle_ms = crate::tsf::observer::gji_idle_ms();
        let resp = self
            .output
            .gji_on_event(crate::tsf::gji_fsm::GjiEvent::ImeOn {
                injection_mode,
                gji_idle_ms,
            });
        self.dispatch_gji_response(&resp);
    }

    fn dispatch_gji_event(&mut self, event: crate::tsf::gji_fsm::GjiEvent) {
        let resp = self.output.gji_on_event(event);
        self.dispatch_gji_response(&resp);
    }

    /// IME OFF を GjiFsm に通知する（`on_ime_applied(open=false)` から呼ぶ）。
    pub(crate) fn gji_on_ime_off(&mut self) {
        self.dispatch_gji_event(crate::tsf::gji_fsm::GjiEvent::ImeOff);
    }

    /// TIMER_GJI_LONG_IDLE ハンドラ。LongIdle タイムアウトを GjiFsm に通知する。
    pub(crate) fn gji_on_timer_long_idle(&mut self) {
        let resp = self.output.gji_on_long_idle();
        self.dispatch_gji_response(&resp);
    }

    /// IME ON/OFF やフォーカス変化なしに composition context が無効化されたことを GjiFsm に通知する。
    ///
    /// `on_passthrough_key` の PassthroughKey / F2NonTsf や
    /// `mark_cold_raw_tsf`（`step_probe` 経由）から呼ぶ。
    pub(crate) fn gji_on_composition_reset(&mut self) {
        self.dispatch_gji_event(crate::tsf::gji_fsm::GjiEvent::CompositionReset);
    }

    /// TSF mode で物理 F2 が消費されたことを GjiFsm に通知する（`on_reinject_key` の NativeF2Consumed パス）。
    ///
    /// Medium/Long cold 中は probe が継続（saw_native_f2=true）。Short cold / OnWarm / OnComposing は
    /// CompositionReset 相当として処理される（GjiFsm 側で分岐）。
    pub(crate) fn gji_on_native_f2_consumed(&mut self) {
        self.dispatch_gji_event(crate::tsf::gji_fsm::GjiEvent::NativeF2Consumed);
    }

    /// GJI candidate SHOW → GjiFsm::StartComposition を dispatch する。
    ///
    /// `observation_event_proc` が `pending_start_composition` を set した後、
    /// `advance_tsf_probe` / `send_keys` で `take_pending_start_composition()` が true を返したときに呼ぶ。
    pub(crate) fn gji_on_start_composition(&mut self) {
        log::debug!("[gji-fsm] StartComposition (candidate SHOW)");
        self.dispatch_gji_event(crate::tsf::gji_fsm::GjiEvent::StartComposition);
    }

    /// GJI candidate HIDE → GjiFsm::EndComposition を dispatch する。
    ///
    /// `observation_event_proc` が `pending_end_composition` を set した後、
    /// `advance_tsf_probe` / `send_keys` で `take_pending_end_composition()` が true を返したときに呼ぶ。
    /// `OnComposing` 以外の状態では epoch が取れないためスキップする（GjiFsm 側でも無視される）。
    pub(crate) fn gji_on_end_composition(&mut self) {
        if let Some(epoch) = self.output.gji_current_composition_epoch() {
            log::debug!("[gji-fsm] EndComposition (candidate HIDE) epoch={epoch:?}");
            self.dispatch_gji_event(crate::tsf::gji_fsm::GjiEvent::EndComposition { epoch });
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
        self.output.flush_raw_tsf_literal_recovery();
        self.drain_output_post_send_effects();
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
    fn start_unicode_cold_warmup(&mut self, cold_seq: u32, deferred: Vec<char>) {
        if self.output.try_push_unicode_chars_to_pending(&deferred) {
            log::debug!(
                "[unicode-cold-warmup] {} chars を飛行中 FSM に追記 (新規 FSM/VK_A+BS 送信スキップ)",
                deferred.len()
            );
            return;
        }
        let baseline = crate::tsf::observer::gji_write_bytes();
        self.output.send_unicode_cold_warmup_keys(cold_seq);
        log::info!(
            "[unicode-cold-warmup] cold={cold_seq} long-cold Unicode warm-up: \
             VK_IME_ON+VK_A+BS → {} chars defer",
            deferred.len()
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
        let needs_unicode_cold_warmup = self.output.injection_mode
            == crate::output::InjectionMode::Unicode
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

    fn apply_ime_open(&mut self, open: bool) -> awase::platform::ImeOpenOutcome {
        self.apply_ime_open_with_applied(open, None)
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
            log::debug!(
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
            log::debug!(
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
            log::debug!("[engine-state-key] skipped (profile={profile:?}, VK_KANJI済み)");
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
        // UnsafeToToggle: 送信しなかったので何もしない（executor 側で早期リターン済みだが念のため）
        if outcome == ImeOpenOutcome::UnsafeToToggle {
            return;
        }
        let effective = match outcome {
            ImeOpenOutcome::Applied
            | ImeOpenOutcome::FallbackSent
            | ImeOpenOutcome::AlreadyMatched => open,
            ImeOpenOutcome::Failed => !open,
            ImeOpenOutcome::UnsafeToToggle => unreachable!(),
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
            }
        }
        // CompositionFsm の状態を IME ON/OFF に追従させる（保留 warmup の epoch 整合用）。
        let tsf_mode = self.output.is_tsf_mode();
        let comp_event = if open {
            crate::tsf::composition_fsm::CompositionEvent::ImeOn { tsf_mode }
        } else {
            crate::tsf::composition_fsm::CompositionEvent::ImeOff
        };
        self.feed_composition_event(comp_event, Some(effective));
        if open {
            log::debug!("[composition] ImeEffect::SetOpen(true) → marking cold");
            self.output
                .mark_composition_cold(crate::output::ColdReason::SetOpenTrue);
            let mode = self.output.injection_mode;
            self.gji_on_ime_on(mode);
            self.output.send_eager_tsf_warmup(Some(effective));
        } else {
            log::debug!("[composition] ImeEffect::SetOpen(false) → marking cold (prevent warm+TSF Enter leak)");
            self.output
                .mark_composition_cold(crate::output::ColdReason::SetOpenFalse);
            self.gji_on_ime_off();
        }
    }

    fn on_passthrough_key(
        &mut self,
        vk: awase::types::VkCode,
        is_keydown: bool,
        applied_ime_on: Option<bool>,
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
            self.feed_composition_event(
                CompositionEvent::ConfirmKeyDown { vk, tsf_mode, warm },
                applied_ime_on,
            );
            return self.composition_fsm.pending_warmup_vk() == Some(vk);
        }
        false
    }

    fn on_reinject_key(
        &mut self,
        vk: awase::types::VkCode,
        is_keydown: bool,
        applied_ime_on: Option<bool>,
    ) {
        use crate::vk::VkCodeExt as _;
        let applied = applied_ime_on;

        if vk == crate::vk::VK_DBE_HIRAGANA && is_keydown && self.output.is_tsf_mode() {
            log::debug!(
                "[reinject-tsf] vk=0xf2 KeyDown TSF mode → marking cold (NativeF2Consumed)",
            );
            self.output
                .mark_composition_cold(crate::output::ColdReason::NativeF2Consumed);
            self.gji_on_native_f2_consumed();
            // conv mutation の可否は send_eager_tsf_warmup が conv_mutation_allowed で self-gate する。
            self.output.send_eager_tsf_warmup(applied);
            return;
        }

        if is_keydown && vk.is_composition_confirm_key() {
            // 2026-07-11: この confirm キーは on_passthrough_key で既に一度処理済みの
            // 同じ物理キーイベントが reinject/defer キューを経由して再度届いたもの。
            // warm であれば（composition_fsm.rs の ConfirmKeyDown と同じ理由で）
            // cold 化・GJI reset とも不要 — 何もしないと BUG-24 系の false positive
            // （不要な BS）の温床になっていた連続 typing 中の余分な cold 化を防げる。
            if self.output.is_composition_warm() {
                log::trace!("[composition] reinject KeyDown vk={vk:#04x} warm → cold化スキップ",);
                return;
            }
            log::debug!(
                "[composition] reinject KeyDown vk={vk:#04x} → marking cold + eager warmup",
            );
            self.output
                .mark_composition_cold(crate::output::ColdReason::ReinjectConfirmKey);
            self.gji_on_composition_reset();
            // conv mutation の可否は send_eager_tsf_warmup が conv_mutation_allowed で self-gate する。
            self.output.send_eager_tsf_warmup(applied);
        }
    }
}

impl WindowsPlatform {
    /// `apply_ime_open` 用の `ImeControlView` を構築する。
    ///
    /// `applied` には呼び出し元が持つ `ImeModel.applied_pair()` の戻り値を渡す。
    /// `None` を渡した場合は `(false, 0)`（未適用）として扱う。
    pub(crate) fn build_ime_control_view(
        &self,
        applied: Option<(bool, u64)>,
    ) -> crate::state::ImeControlView<'_> {
        let class_name = if self.focus.is_focused() {
            self.focus.class_name()
        } else {
            ""
        };
        let (shadow_on, _applied_at_ms) = applied.unwrap_or((false, 0));
        crate::state::ImeControlView {
            focus: crate::state::FocusFacts {
                class_name,
                profile: self.current_app_profile(),
            },
            observed: crate::state::ObservedState::from_snapshot(crate::tsf::observer::tsf_obs()),
            control: crate::state::ControlLog { shadow_on },
            belief_input_mode: awase::engine::InputModeState::Unknown,
        }
    }

    /// 事前構築済みの `ImeControlView` と `OpenBelief` を受け取る中核実装。
    ///
    /// `tsf_obs()` の重複呼び出しを避けるため view は呼び出し元が一度だけ構築して渡す。
    /// 戦略選択と実行は [`crate::ime_controller::CONTROLLER`] が唯一の SSOT として担う。
    /// `belief` は診断ログ用（`effective_open` / `confident`）に受け取る。
    // 兄弟メソッド apply_ime_open_with_belief から `self.` 記法で呼ばれるため、
    // また PlatformRuntime 委譲メソッド群との一貫した API 配置のため `&self` を維持する。
    #[allow(clippy::unused_self)]
    pub(crate) fn apply_ime_open_with_view(
        &self,
        open: bool,
        view: &crate::state::ImeControlView<'_>,
        belief: crate::output::OpenBelief,
    ) -> awase::platform::ImeOpenOutcome {
        let outcome = crate::ime_controller::CONTROLLER.apply(open, view);
        log::debug!(
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
        open: bool,
        applied: Option<(bool, u64)>,
        belief: crate::output::OpenBelief,
    ) -> awase::platform::ImeOpenOutcome {
        let view = self.build_ime_control_view(applied);
        self.apply_ime_open_with_view(open, &view, belief)
    }

    /// shadow のみから自明なビリーフを作る後方互換ラッパー。
    ///
    /// ImmCross 非経路かつ EngineIntent 外の呼び出しに使う。
    pub(crate) fn apply_ime_open_with_applied(
        &self,
        open: bool,
        applied: Option<(bool, u64)>,
    ) -> awase::platform::ImeOpenOutcome {
        let shadow_on = applied.is_some_and(|(s, _)| s);
        self.apply_ime_open_with_belief(
            open,
            applied,
            crate::output::OpenBelief::from_shadow(shadow_on),
        )
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
    pub fn update_focus_info(&mut self, process_id: u32, class_name: String) {
        self.focus.update(process_id, class_name);
    }

    /// IMM 能力キャッシュに学習結果を追加し、ファイルに永続化する。
    pub fn learn_imm_capability(&mut self, class_name: String, cap: ImmCapability) {
        self.focus.learn_imm_capability(class_name, cap);
    }

    /// UIA ワーカーへの送信チャネルを設定する。
    pub fn set_uia_sender(
        &mut self,
        sender: std::sync::mpsc::Sender<crate::focus::uia::SendableHwnd>,
    ) {
        self.focus.set_uia_sender(sender);
    }
}
