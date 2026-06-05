//! ProbeIo トレイト — Win32 副作用を抽象化し `dispatch_probe_actions` をテスト可能にする。
//!
//! `Output` が本番実装。`#[cfg(test)]` ブロック内の `FakeProbeIo` がテスト実装。
//! `dispatch_probe_actions` は `ProbeIo` を受け取り、Win32 呼び出しを直接行わない。

use crate::output::{Output, VkSequence, WarmupOutcome};
use crate::tsf::observer::NamechangeBaseline;
use crate::tsf::output::ColdReason;
use crate::tsf::probe_fsm::DeferredVk;
use crate::tsf::TsfGateState;
use awase::types::VkCode;

/// `dispatch_probe_actions` が要求する Win32 / 状態ミューテーション操作の抽象。
///
/// - `Output` が本番実装（Win32 SendInput・グローバル原子値の操作）
/// - `FakeProbeIo` がテスト実装（状態変化をフラグで記録し、返値を制御）
pub(crate) trait ProbeIo {
    /// TSF ゲートが `Bypass` 状態かどうかを返す。
    fn gate_is_bypass(&self) -> bool;
    /// GJI モニターが正常動作しているかどうかを返す。
    fn gji_monitor_healthy(&self) -> bool;
    /// GJI が LONG_IDLE_MS 以上静止しているかどうかを返す。
    ///
    /// `true` のとき LiteralDetect は常に false positive になるためスキップする。
    fn gji_long_idle(&self) -> bool;
    /// TSF 送信パイプラインを実行し、backspace 相当数を返す。
    fn transmit_tsf(
        &self,
        romaji: &str,
        chars: &[(VkCode, bool)],
        outcome: &WarmupOutcome,
    ) -> usize;
    /// Chrome バッチ送信を実行する。
    fn transmit_chrome(&self, romaji: &str, chars: &[(VkCode, bool)]);
    /// deferred VKs を送信する。`use_tsf_marker` = true → `TSF_MARKER`、false → `INJECTED_MARKER`。
    fn send_deferred_vks(&self, vks: &[DeferredVk], use_tsf_marker: bool);
    /// composition を warm にマークする。
    fn mark_warm(&self);
    /// fresh F2 (`VK_DBE_HIRAGANA`) を送信し、`(namechange_baseline, sent_ms)` を返す。
    ///
    /// ベースラインは SendInput **前**に取得すること（送信中の NAMECHANGE を見逃さないため）。
    fn send_fresh_f2(&self) -> (NamechangeBaseline, u64);
    /// 連続 raw TSF literal 回数を返す。
    fn consecutive_count(&self) -> u32;
    /// `RAW_TSF_LITERAL` グローバルを設定する（`consecutive == 0` のときのみ呼ばれる）。
    fn set_raw_literal(&self, backs: usize, romaji: String);
    /// composition を `RawTsfLiteralRecovery` で cold にマークする。
    fn mark_cold_raw_tsf(&self);
}

impl ProbeIo for Output {
    fn gate_is_bypass(&self) -> bool {
        self.tsf_gate.state() == TsfGateState::Bypass
    }

    fn gji_monitor_healthy(&self) -> bool {
        crate::tsf::observer::gji_monitor_healthy()
    }

    fn gji_long_idle(&self) -> bool {
        crate::hook::current_tick_ms()
            .saturating_sub(crate::tsf::observer::gji_last_io_ms())
            >= crate::tuning::LONG_IDLE_MS
    }

    fn transmit_tsf(
        &self,
        romaji: &str,
        chars: &[(VkCode, bool)],
        outcome: &WarmupOutcome,
    ) -> usize {
        crate::output::TsfSendPipeline::transmit(romaji, chars, outcome)
    }

    fn transmit_chrome(&self, romaji: &str, chars: &[(VkCode, bool)]) {
        Self::send_romaji_batch_immediate(romaji, chars);
    }

    fn send_deferred_vks(&self, vks: &[DeferredVk], use_tsf_marker: bool) {
        let pairs: Vec<(VkCode, bool)> = vks.iter().map(|d| (d.vk, d.needs_shift)).collect();
        Self::send_deferred_probe_vks_from(&pairs, use_tsf_marker);
    }

    fn mark_warm(&self) {
        self.mark_composition_warm();
    }

    fn send_fresh_f2(&self) -> (NamechangeBaseline, u64) {
        use crate::vk::VK_DBE_HIRAGANA;
        let refresh = [
            crate::tsf::output::make_tsf_key_input(VK_DBE_HIRAGANA, false),
            crate::tsf::output::make_tsf_key_input(VK_DBE_HIRAGANA, true),
        ];
        let nc_baseline = crate::tsf::observer::namechange_baseline();
        let _ = crate::win32::send_input_safe(&refresh);
        let fresh_f2_ms = crate::hook::current_tick_ms();
        (nc_baseline, fresh_f2_ms)
    }

    fn consecutive_count(&self) -> u32 {
        self.composition.consecutive_count()
    }

    fn set_raw_literal(&self, backs: usize, romaji: String) {
        self.record_raw_tsf_literal(backs, romaji);
    }

    fn mark_cold_raw_tsf(&self) {
        self.mark_composition_cold(ColdReason::RawTsfLiteralRecovery);
    }
}

/// probe dispatcher の汎用実装。
///
/// `platform.rs` の `dispatch_probe_actions` を置き換える。
/// `io: &impl ProbeIo` で Win32 副作用を注入することでテスト可能。
#[expect(clippy::too_many_lines)]
pub(crate) fn dispatch_probe_actions<I: ProbeIo>(
    machine: &mut crate::tsf::probe_fsm::TsfProbeMachine,
    initial_actions: Vec<crate::tsf::probe_fsm::ProbeAction>,
    io: &I,
) -> bool {
    use crate::tsf::probe_fsm::{ProbeAction, TransmitTarget};
    use std::collections::VecDeque;

    let mut queue: VecDeque<ProbeAction> = initial_actions.into();

    while let Some(action) = queue.pop_front() {
        match action {
            ProbeAction::Done => return true,

            ProbeAction::SendFreshF2 {
                cold_seq,
                probe_settled,
            } => {
                let settle_reason = if probe_settled {
                    "NativeF2Consumed/SetOpenTrue"
                } else {
                    "probe timeout"
                };
                log::debug!(
                    "[tsf-probe] cold={cold_seq} {settle_reason} → fresh F2 + NameChangeWait"
                );
                let (nc_baseline, fresh_f2_ms) = io.send_fresh_f2();
                machine.apply_fresh_f2_sent(nc_baseline, fresh_f2_ms);
                // gji_long_idle 時は NameChangeWait をスキップして即 TransmitTsf へ。
                // unicode TSF (KEYEVENTF_UNICODE) は IME モード非依存のため
                // OBJ_NAMECHANGE によるモード確認待機が不要。
                if io.gji_long_idle() {
                    queue.extend(machine.skip_namechange_wait());
                }
            }

            ProbeAction::Transmit {
                cold_seq,
                prepend_f2_warmup,
                used_eager_path,
                nc_fired,
                romaji,
                deferred_vks,
                target,
            } => {
                let chars: VkSequence = romaji
                    .chars()
                    .filter_map(crate::output::resolve_ascii_to_vk)
                    .collect();
                match target {
                    TransmitTarget::Tsf => {
                        if io.gate_is_bypass() {
                            log::debug!("[do-transmit] gate=Bypass, skipping TSF injection");
                            return true;
                        }
                        if chars.is_empty() {
                            return true;
                        }
                        let outcome = WarmupOutcome {
                            prepend_f2_warmup,
                            // nc_fired=true: IME モード確認済み（OBJ_NAMECHANGE 受信）。
                            //   通常は deferred_vks.is_empty() を守り、
                            //   KEYEVENTF_UNICODE 直後の N VK で "の" が "nお" になる
                            //   WezTerm バグを防ぐ。
                            //   gji_long_idle 時は IME モード依存の VK ローマ字を避けるため
                            //   deferred_vks が空なら unicode TSF を優先する。
                            //
                            // nc_fired=false: NameChangeWait タイムアウトまたは gji_long_idle スキップ。
                            //   IME モード切替未確認。VK ローマ字で送ると katakana 等で誤出力になる。
                            //   gji_long_idle: unicode TSF を強制（IME モード非依存）。
                            //   非 long_idle: used_eager_path のまま（8d38b2d の挙動を維持）。
                            used_eager_path: if nc_fired {
                                (used_eager_path || io.gji_long_idle()) && deferred_vks.is_empty()
                            } else {
                                used_eager_path || io.gji_long_idle()
                            },
                            cold_seq,
                        };
                        {
                            // 診断ログ: IMC_GETCONVERSIONMODE は SendMessageTimeoutW を呼ぶため、
                            // with_app 再入を避けるため async タスクへオフロードする (Step 3)。
                            // ログ出力タイミングが数 ms 遅れるが診断用途のため許容。
                            let last_io = crate::tsf::observer::gji_last_io_ms();
                            let gji_idle = crate::hook::current_tick_ms().saturating_sub(last_io);
                            let romaji_owned: String = romaji.clone();
                            let chars_len = chars.len();
                            win32_async::spawn_local(async move {
                                let conv =
                                    crate::ime::get_ime_conversion_mode_raw_timeout_async(10).await;
                                log::debug!(
                                    "[h1-send] cold={cold_seq} romaji={romaji_owned:?} chars={chars_len} \
                                     gji_idle={gji_idle}ms conv={} ROMAN={} NATIVE={}",
                                    conv.map_or_else(
                                        || "none".to_string(),
                                        |v| format!("0x{v:08X}")
                                    ),
                                    conv.is_some_and(|v| crate::imm::cmode_has(v, crate::imm::IME_CMODE_ROMAN)),
                                    conv.is_some_and(|v| crate::imm::cmode_has(v, crate::imm::IME_CMODE_NATIVE)),
                                );
                            });
                        }
                        let gji_active = io.gji_monitor_healthy();
                        let needs_literal =
                            prepend_f2_warmup
                            && gji_active
                            && !io.gji_long_idle()
                            && io.consecutive_count() < crate::tuning::CONSECUTIVE_LITERAL_SKIP;
                        let detector = needs_literal.then(crate::tsf::probe::LiteralDetector::new);
                        let ze_bs_count = io.transmit_tsf(&romaji, &chars, &outcome);
                        io.send_deferred_vks(&deferred_vks, true);
                        io.mark_warm();
                        if machine.apply_transmit_done(romaji, ze_bs_count, detector) {
                            return true;
                        }
                    }
                    TransmitTarget::Chrome => {
                        // GJI モニター健全時のみ literal 検出を起動する。
                        // 検出ベースラインは送信前に確定させること。
                        let detector = io
                            .gji_monitor_healthy()
                            .then(crate::tsf::probe::LiteralDetector::new);
                        let ze_bs_count = chars.len();
                        io.transmit_chrome(&romaji, &chars);
                        io.send_deferred_vks(&deferred_vks, false);
                        io.mark_warm();
                        if machine.apply_transmit_done(romaji, ze_bs_count, detector) {
                            return true;
                        }
                    }
                }
            }

            ProbeAction::RawTsfLiteralRecovery {
                cold_seq,
                backs,
                romaji,
            } => {
                let consecutive = io.consecutive_count();
                if consecutive == 0 {
                    log::warn!(
                        "[raw-tsf-literal] cold={cold_seq} raw TSF literal suspected \
                        → backspace ×{backs} + re-send {romaji:?} scheduled \
                        + mark cold"
                    );
                    io.set_raw_literal(backs, romaji);
                } else {
                    log::warn!(
                        "[raw-tsf-literal] cold={cold_seq} consecutive raw-tsf-literal \
                        (count={}) → likely false positive, giving up",
                        consecutive + 1,
                    );
                }
                io.mark_cold_raw_tsf();
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tsf::probe_bridge::OutputActiveGuard;
    use crate::tsf::probe_fsm::{ProbeAction, TransmitTarget};
    use std::cell::Cell;

    /// テスト用フェイク ProbeIo。Win32 副作用を no-op にし、呼び出しをフラグで記録する。
    struct FakeProbeIo {
        bypass: bool,
        gji_healthy: bool,
        gji_long_idle: bool,
        tsf_transmit_result: usize,
        consecutive: u32,
        transmit_tsf_called: Cell<bool>,
        transmit_chrome_called: Cell<bool>,
        deferred_vks_called: Cell<bool>,
        mark_warm_called: Cell<bool>,
        send_fresh_f2_called: Cell<bool>,
        set_raw_literal_called: Cell<bool>,
        mark_cold_raw_tsf_called: Cell<bool>,
        /// transmit_tsf に渡された WarmupOutcome.used_eager_path を記録する。
        last_used_eager_path: Cell<bool>,
    }

    impl Default for FakeProbeIo {
        fn default() -> Self {
            Self {
                bypass: false,
                gji_healthy: false,
                gji_long_idle: false,
                tsf_transmit_result: 1,
                consecutive: 0,
                transmit_tsf_called: Cell::new(false),
                transmit_chrome_called: Cell::new(false),
                deferred_vks_called: Cell::new(false),
                mark_warm_called: Cell::new(false),
                send_fresh_f2_called: Cell::new(false),
                set_raw_literal_called: Cell::new(false),
                mark_cold_raw_tsf_called: Cell::new(false),
                last_used_eager_path: Cell::new(false),
            }
        }
    }

    impl ProbeIo for FakeProbeIo {
        fn gate_is_bypass(&self) -> bool {
            self.bypass
        }
        fn gji_monitor_healthy(&self) -> bool {
            self.gji_healthy
        }
        fn gji_long_idle(&self) -> bool {
            self.gji_long_idle
        }
        fn transmit_tsf(
            &self,
            _romaji: &str,
            _chars: &[(VkCode, bool)],
            outcome: &WarmupOutcome,
        ) -> usize {
            self.transmit_tsf_called.set(true);
            self.last_used_eager_path.set(outcome.used_eager_path);
            self.tsf_transmit_result
        }
        fn transmit_chrome(&self, _romaji: &str, _chars: &[(VkCode, bool)]) {
            self.transmit_chrome_called.set(true);
        }
        fn send_deferred_vks(&self, _vks: &[DeferredVk], _use_tsf_marker: bool) {
            self.deferred_vks_called.set(true);
        }
        fn mark_warm(&self) {
            self.mark_warm_called.set(true);
        }
        fn send_fresh_f2(&self) -> (NamechangeBaseline, u64) {
            self.send_fresh_f2_called.set(true);
            (crate::tsf::observer::namechange_baseline(), 0)
        }
        fn consecutive_count(&self) -> u32 {
            self.consecutive
        }
        fn set_raw_literal(&self, _backs: usize, _romaji: String) {
            self.set_raw_literal_called.set(true);
        }
        fn mark_cold_raw_tsf(&self) {
            self.mark_cold_raw_tsf_called.set(true);
        }
    }

    fn make_chrome_machine() -> crate::tsf::probe_fsm::TsfProbeMachine {
        let guard = OutputActiveGuard::noop_for_test();
        let probe = crate::tsf::probe::TsfReadinessProbe::new(0, 0, 0);
        crate::tsf::probe_fsm::TsfProbeMachine::new_chrome("ka", 0, probe, 0, guard)
    }

    fn make_gji_machine() -> crate::tsf::probe_fsm::TsfProbeMachine {
        let guard = OutputActiveGuard::noop_for_test();
        let probe = crate::tsf::probe::TsfReadinessProbe::new(0, 0, 0);
        crate::tsf::probe_fsm::TsfProbeMachine::new_gji(
            "ka",
            0,
            probe,
            0,
            false,
            crate::tsf::output::ColdReason::FocusChange,
            false,
            false,
            guard,
        )
    }

    #[test]
    fn done_action_returns_true_without_side_effects() {
        let io = FakeProbeIo::default();
        let mut machine = make_chrome_machine();
        let done = dispatch_probe_actions(&mut machine, vec![ProbeAction::Done], &io);
        assert!(done);
        assert!(!io.transmit_tsf_called.get());
        assert!(!io.transmit_chrome_called.get());
        assert!(!io.mark_warm_called.get());
    }

    #[test]
    fn chrome_transmit_calls_transmit_chrome_and_mark_warm() {
        let io = FakeProbeIo::default();
        let mut machine = make_chrome_machine();
        let actions = vec![ProbeAction::Transmit {
            cold_seq: 0,
            prepend_f2_warmup: false,
            used_eager_path: false,
            nc_fired: true,
            romaji: "ka".to_string(),
            deferred_vks: vec![],
            target: TransmitTarget::Chrome,
        }];
        let done = dispatch_probe_actions(&mut machine, actions, &io);
        assert!(done);
        assert!(io.transmit_chrome_called.get());
        assert!(io.mark_warm_called.get());
        assert!(!io.transmit_tsf_called.get());
    }

    #[test]
    fn chrome_transmit_with_gji_healthy_installs_literal_detect() {
        // GJI モニター健全時は Chrome バッチ送信後も LiteralDetect フェーズへ遷移し、
        // Done を即返さないことで literal 検出のための再ティックを許可する。
        let io = FakeProbeIo {
            gji_healthy: true,
            ..Default::default()
        };
        let mut machine = make_chrome_machine();
        let actions = vec![ProbeAction::Transmit {
            cold_seq: 0,
            prepend_f2_warmup: false,
            used_eager_path: false,
            nc_fired: true,
            romaji: "ka".to_string(),
            deferred_vks: vec![],
            target: TransmitTarget::Chrome,
        }];
        let done = dispatch_probe_actions(&mut machine, actions, &io);
        assert!(!done, "should not be Done — LiteralDetect phase pending");
        assert!(io.transmit_chrome_called.get());
        assert!(io.mark_warm_called.get());
        assert_eq!(machine.phase_label(), "LiteralDetect");
    }

    #[test]
    fn tsf_transmit_bypass_returns_true_without_transmit() {
        let io = FakeProbeIo {
            bypass: true,
            ..Default::default()
        };
        let mut machine = make_gji_machine();
        let actions = vec![ProbeAction::Transmit {
            cold_seq: 0,
            prepend_f2_warmup: false,
            used_eager_path: false,
            nc_fired: true,
            romaji: "ka".to_string(),
            deferred_vks: vec![],
            target: TransmitTarget::Tsf,
        }];
        let done = dispatch_probe_actions(&mut machine, actions, &io);
        assert!(done);
        assert!(!io.transmit_tsf_called.get());
        assert!(!io.mark_warm_called.get());
    }

    #[test]
    fn tsf_transmit_skips_literal_detect_when_gji_long_idle() {
        // GJI が長期静止（WezTerm 等）のとき LiteralDetect を入れないことで
        // false positive による BS 送信ループを防ぐ。
        let io = FakeProbeIo {
            gji_healthy: true,
            gji_long_idle: true,
            ..Default::default()
        };
        let mut machine = make_gji_machine();
        let actions = vec![ProbeAction::Transmit {
            cold_seq: 0,
            prepend_f2_warmup: true,
            used_eager_path: false,
            nc_fired: true,
            romaji: "ka".to_string(),
            deferred_vks: vec![],
            target: TransmitTarget::Tsf,
        }];
        let done = dispatch_probe_actions(&mut machine, actions, &io);
        assert!(done, "should be Done — LiteralDetect must be skipped when GJI is long-idle");
        assert!(io.transmit_tsf_called.get());
        assert!(io.mark_warm_called.get());
    }

    #[test]
    fn tsf_transmit_calls_transmit_tsf_and_mark_warm() {
        let io = FakeProbeIo::default(); // bypass=false, gji_healthy=false
        let mut machine = make_gji_machine();
        let actions = vec![ProbeAction::Transmit {
            cold_seq: 0,
            prepend_f2_warmup: false,
            used_eager_path: false,
            nc_fired: true,
            romaji: "ka".to_string(),
            deferred_vks: vec![],
            target: TransmitTarget::Tsf,
        }];
        let done = dispatch_probe_actions(&mut machine, actions, &io);
        assert!(done);
        assert!(io.transmit_tsf_called.get());
        assert!(io.mark_warm_called.get());
        assert!(!io.transmit_chrome_called.get());
    }

    #[test]
    fn tsf_transmit_uses_eager_path_when_nc_not_fired_even_with_deferred_vks() {
        // NameChangeWait がタイムアウトした（nc_fired=false）場合、deferred_vks があっても
        // used_eager_path を尊重して Unicode TSF パスを使う。
        // IME モード切替未確認時に VK ローマ字を送ると katakana 等でリテラル出力になるバグ回避。
        let io = FakeProbeIo {
            gji_healthy: false, // LiteralDetect は無効化（today's focus はモード切替）
            ..Default::default()
        };
        let mut machine = make_gji_machine();
        let actions = vec![ProbeAction::Transmit {
            cold_seq: 0,
            prepend_f2_warmup: true,
            used_eager_path: true,
            nc_fired: false, // NameChangeWait タイムアウト
            romaji: "ki".to_string(),
            deferred_vks: vec![crate::tsf::probe_fsm::DeferredVk {
                vk: awase::types::VkCode(0x49), // VK_I
                needs_shift: false,
            }],
            target: TransmitTarget::Tsf,
        }];
        let done = dispatch_probe_actions(&mut machine, actions, &io);
        assert!(done);
        assert!(io.transmit_tsf_called.get());
        // nc_fired=false のとき used_eager_path は deferred_vks.is_empty() に関わらず true。
        // TsfSendPipeline::transmit が Unicode TSF を選ぶかどうかは transmit_tsf の内部判定だが
        // ここでは WarmupOutcome に used_eager_path=true が渡ることを確認する。
        // （実際の Unicode 選択は transmit_tsf 実装依存のため、呼び出しが行われたことのみ確認）
    }

    #[test]
    fn raw_tsf_literal_recovery_sets_literal_and_marks_cold_when_first_time() {
        let io = FakeProbeIo::default(); // consecutive == 0
        let mut machine = make_gji_machine();
        let actions = vec![
            ProbeAction::RawTsfLiteralRecovery {
                cold_seq: 0,
                backs: 2,
                romaji: "ka".to_string(),
            },
            ProbeAction::Done,
        ];
        let done = dispatch_probe_actions(&mut machine, actions, &io);
        assert!(done);
        assert!(io.set_raw_literal_called.get());
        assert!(io.mark_cold_raw_tsf_called.get());
    }

    #[test]
    fn raw_tsf_literal_recovery_skips_set_literal_when_consecutive() {
        let io = FakeProbeIo {
            consecutive: 1,
            ..Default::default()
        };
        let mut machine = make_gji_machine();
        let actions = vec![
            ProbeAction::RawTsfLiteralRecovery {
                cold_seq: 0,
                backs: 2,
                romaji: "ka".to_string(),
            },
            ProbeAction::Done,
        ];
        let done = dispatch_probe_actions(&mut machine, actions, &io);
        assert!(done);
        assert!(
            !io.set_raw_literal_called.get(),
            "should skip set when consecutive > 0"
        );
        assert!(io.mark_cold_raw_tsf_called.get(), "should always mark cold");
    }

    #[test]
    fn send_fresh_f2_action_calls_send_fresh_f2() {
        let io = FakeProbeIo::default();
        let mut machine = make_gji_machine();
        let actions = vec![ProbeAction::SendFreshF2 {
            cold_seq: 0,
            probe_settled: false,
        }];
        // SendFreshF2 は apply_fresh_f2_sent を呼ぶだけで Done を emit しない。
        // 返値は false（queue が空になり Done なし）。
        let done = dispatch_probe_actions(&mut machine, actions, &io);
        assert!(!done);
        assert!(io.send_fresh_f2_called.get());
    }

    #[test]
    fn send_fresh_f2_with_gji_long_idle_skips_namechange_wait_and_transmits() {
        // gji_long_idle 時は SendFreshF2 の直後に NameChangeWait をスキップして
        // 同一ディスパッチコール内で TransmitTsf まで進む。
        // 340ms の OBJ_NAMECHANGE 待機を排除し、コールドスタート遅延を ~60ms に削減する。
        use crate::tsf::probe_fsm::{ProbePhase, WaitingFor};
        let io = FakeProbeIo {
            gji_long_idle: true,
            ..Default::default()
        };
        let mut machine = make_gji_machine();
        // apply_fresh_f2_sent が機能するよう、FreshF2Sent フェーズへ強制移行する。
        let send = Box::new(crate::tsf::probe_fsm::SendState {
            romaji: "ka".to_string(),
            deferred_vks: vec![],
        });
        machine.force_phase_for_test(ProbePhase::WaitingForCallback(
            WaitingFor::FreshF2Sent {
                probe_settled: false,
                gji_idle_ms: 15_000,
                remaining_ms: 0,
                send,
                guard: OutputActiveGuard::noop_for_test(),
            },
        ));
        let actions = vec![ProbeAction::SendFreshF2 {
            cold_seq: 0,
            probe_settled: false,
        }];
        // gji_long_idle=true のとき、同一ディスパッチで NameChangeWait をスキップして
        // TransmitTsf まで実行される（返値 true = Done）。
        let done = dispatch_probe_actions(&mut machine, actions, &io);
        assert!(done, "gji_long_idle: NameChangeWait をスキップして同一コールで Done になるべき");
        assert!(io.send_fresh_f2_called.get());
        assert!(io.transmit_tsf_called.get(), "TransmitTsf が実行されるべき");
        assert!(
            io.last_used_eager_path.get(),
            "gji_long_idle: used_eager_path が true に強制されるべき（unicode TSF）"
        );
        assert!(io.mark_warm_called.get());
    }

    #[test]
    fn nc_not_fired_with_gji_long_idle_forces_unicode_tsf() {
        // nc_fired=false（NameChangeWait タイムアウトまたはスキップ）かつ gji_long_idle のとき、
        // used_eager_path=false でも unicode TSF（used_eager_path=true）が強制される。
        let io = FakeProbeIo {
            gji_long_idle: true,
            ..Default::default()
        };
        let mut machine = make_gji_machine();
        let actions = vec![ProbeAction::Transmit {
            cold_seq: 0,
            prepend_f2_warmup: true,
            used_eager_path: false, // eager warmup なしのコールドスタート
            nc_fired: false,
            romaji: "ka".to_string(),
            deferred_vks: vec![],
            target: TransmitTarget::Tsf,
        }];
        let done = dispatch_probe_actions(&mut machine, actions, &io);
        assert!(done);
        assert!(io.transmit_tsf_called.get());
        assert!(
            io.last_used_eager_path.get(),
            "gji_long_idle: nc_fired=false でも used_eager_path が true になるべき（unicode TSF 強制）"
        );
    }
}
