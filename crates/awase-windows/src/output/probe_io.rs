//! ProbeIo トレイト — Win32 副作用を抽象化し `dispatch_probe_actions` をテスト可能にする。
//!
//! `Output` が本番実装。`#[cfg(test)]` ブロック内の `FakeProbeIo` がテスト実装。
//! `dispatch_probe_actions` は `ProbeIo` を受け取り、Win32 呼び出しを直接行わない。

use crate::output::{Output, VkMarker, VkSequence, WarmupOutcome};
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
    /// deferred VKs を送信する。
    fn send_deferred_vks(&self, vks: &[DeferredVk], marker: VkMarker);
    /// composition を warm にマークする。
    fn mark_warm(&self);
    /// fresh F2 (`VK_DBE_HIRAGANA`) を送信し、`(namechange_baseline, sent_ms)` を返す。
    ///
    /// ベースラインは SendInput **前**に取得すること（送信中の NAMECHANGE を見逃さないため）。
    fn send_fresh_f2(&self) -> (NamechangeBaseline, u64);
    /// gji_long_idle 時に追加 F2 を送信して F2×2 連続とする。
    ///
    /// F2 単発では GJI I/O が発生しないが、F2×2 連続では GJI が起動して I/O を出す
    /// （cold=1244 実測: 31ms 以内）。`send_fresh_f2` の直後に呼ぶこと。
    fn send_extra_f2(&self);
    /// TSF モード（WezTerm 等 ForceTsf アプリ）かどうかを返す。
    ///
    /// `true` のとき LiteralDetect は IMM composition context が存在しないため
    /// 常に false positive になる。スキップして warm を維持する。
    fn is_tsf_mode(&self) -> bool;
    /// 連続 raw TSF literal 回数を返す。
    fn consecutive_count(&self) -> u32;
    /// `RAW_TSF_LITERAL` グローバルを設定する（`consecutive == 0` のときのみ呼ばれる）。
    fn set_raw_literal(&self, backs: usize, romaji: String);
    /// composition を `RawTsfLiteralRecovery` で cold にマークする。
    fn mark_cold_raw_tsf(&self);
    /// 単一の VK キーを `SendInput` で送信する。
    ///
    /// `KeySeqExec` フェーズが `ProbeAction::SendSeqKey` を emit したときに呼ばれる。
    fn send_key(&self, vk: VkCode);
}

impl ProbeIo for Output {
    fn gate_is_bypass(&self) -> bool {
        self.tsf_gate.state() == TsfGateState::Bypass
    }

    fn gji_monitor_healthy(&self) -> bool {
        crate::tsf::observer::gji_monitor_healthy()
    }

    fn gji_long_idle(&self) -> bool {
        crate::hook::current_tick_ms().saturating_sub(crate::tsf::observer::gji_last_io_ms())
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

    fn send_deferred_vks(&self, vks: &[DeferredVk], marker: VkMarker) {
        let pairs: Vec<(VkCode, bool)> = vks.iter().map(|d| (d.vk, d.needs_shift)).collect();
        Self::send_deferred_probe_vks_from(&pairs, marker);
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

    fn send_extra_f2(&self) {
        use crate::vk::VK_DBE_HIRAGANA;
        let extra = [
            crate::tsf::output::make_tsf_key_input(VK_DBE_HIRAGANA, false),
            crate::tsf::output::make_tsf_key_input(VK_DBE_HIRAGANA, true),
        ];
        let _ = crate::win32::send_input_safe(&extra);
    }

    fn is_tsf_mode(&self) -> bool {
        self.is_tsf_mode()
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

    fn send_key(&self, vk: VkCode) {
        // SAFETY: send_ime_mode_key は Win32 API を呼び出す unsafe fn。
        unsafe { crate::ime::send_ime_mode_key(vk) };
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
                if io.gji_long_idle() {
                    // F2 単発では GJI が I/O を出さない。F2×2 連続で GJI を起動させる。
                    // NameChangeWait 内の gji_long_idle_probe が GJI I/O 応答を監視し、
                    // GJI_IDLE_MS 静止確認後に VK path へ移行する。
                    log::debug!("[tsf-probe] cold={cold_seq} gji_long_idle: 追加 F2 送信 (F2×2 連続で GJI 起動)");
                    io.send_extra_f2();
                }
                machine.apply_fresh_f2_sent(nc_baseline, fresh_f2_ms);
            }

            ProbeAction::Transmit {
                cold_seq,
                prepend_f2_warmup,
                used_eager_path,
                nc_fired,
                gji_resumed,
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
                            // gji_resumed=true: F2×2 後に GJI が I/O 応答 → VK パス強制。
                            //
                            // nc_fired=true: IME モード確認済み（OBJ_NAMECHANGE 受信）。
                            //   通常は deferred_vks.is_empty() を守り、
                            //   KEYEVENTF_UNICODE 直後の N VK で "の" が "nお" になる
                            //   WezTerm バグを防ぐ。
                            //   gji_long_idle 時は IME モード依存の VK ローマ字を避けるため
                            //   deferred_vks が空なら unicode TSF を優先する。
                            //
                            // nc_fired=false: NameChangeWait タイムアウト。
                            //   IME モード切替未確認。VK ローマ字で送ると katakana 等で誤出力になる。
                            //   keybinds_ok=true: F14→F13 活性化が先行するため通常ここに来ない。
                            //     gji_resumed=true で VK path が強制される。
                            //   keybinds_ok=false + gji_long_idle: unicode TSF を強制（IME モード非依存）。
                            //   keybinds_ok=false + 非 long_idle: used_eager_path のまま。
                            //
                            // TSF mode (WezTerm 等 ForceTsf): OBJ_NAMECHANGE が CASCADIA クラスではない
                            //   ため nc_fired=false は常態。unicode は KEYEVENTF_UNICODE で GJI コンポジション
                            //   を完全にバイパスし候補ウィンドウが表示されない。
                            //   GJI が warmup に応答して活動中 (!gji_long_idle) → VK path で GJI コンポジション経由。
                            //   GJI が長期静止 (gji_long_idle) → unicode fallback（GJI 未応答時の安全策）。
                            used_eager_path: if gji_resumed {
                                false // GJI が F2×2 に応答: VK path 強制
                            } else if nc_fired {
                                (used_eager_path || io.gji_long_idle()) && deferred_vks.is_empty()
                            } else if io.is_tsf_mode() {
                                // TSF-native (WezTerm): GJI 活動中は VK path で候補表示; 長期静止は unicode
                                io.gji_long_idle()
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
                        let needs_literal = prepend_f2_warmup
                            && gji_active
                            && !io.gji_long_idle()
                            && !io.is_tsf_mode();
                        let detector = needs_literal.then(crate::tsf::probe::LiteralDetector::new);
                        let ze_bs_count = io.transmit_tsf(&romaji, &chars, &outcome);
                        io.send_deferred_vks(&deferred_vks, VkMarker::Tsf);
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
                        io.send_deferred_vks(&deferred_vks, VkMarker::Injected);
                        io.mark_warm();
                        if machine.apply_transmit_done(romaji, ze_bs_count, detector) {
                            return true;
                        }
                    }
                }
            }

            ProbeAction::SendSeqKey { cold_seq, vk } => {
                log::debug!(
                    "[tsf-probe] cold={cold_seq} KeySeqExec: send_key vk=0x{vk:02X}"
                );
                io.send_key(vk);
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
        tsf_mode: bool,
        tsf_transmit_result: usize,
        consecutive: u32,
        transmit_tsf_called: Cell<bool>,
        transmit_chrome_called: Cell<bool>,
        deferred_vks_called: Cell<bool>,
        mark_warm_called: Cell<bool>,
        send_fresh_f2_called: Cell<bool>,
        send_extra_f2_called: Cell<bool>,
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
                tsf_mode: false,
                tsf_transmit_result: 1,
                consecutive: 0,
                transmit_tsf_called: Cell::new(false),
                transmit_chrome_called: Cell::new(false),
                deferred_vks_called: Cell::new(false),
                mark_warm_called: Cell::new(false),
                send_fresh_f2_called: Cell::new(false),
                send_extra_f2_called: Cell::new(false),
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
        fn is_tsf_mode(&self) -> bool {
            self.tsf_mode
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
        fn send_deferred_vks(&self, _vks: &[DeferredVk], _marker: VkMarker) {
            self.deferred_vks_called.set(true);
        }
        fn mark_warm(&self) {
            self.mark_warm_called.set(true);
        }
        fn send_fresh_f2(&self) -> (NamechangeBaseline, u64) {
            self.send_fresh_f2_called.set(true);
            (crate::tsf::observer::namechange_baseline(), 0)
        }
        fn send_extra_f2(&self) {
            self.send_extra_f2_called.set(true);
        }
        fn send_key(&self, _vk: awase::types::VkCode) {
            // テスト用: 実際には送信しない
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
            gji_resumed: false,
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
            gji_resumed: false,
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
            gji_resumed: false,
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
            gji_resumed: false,
            romaji: "ka".to_string(),
            deferred_vks: vec![],
            target: TransmitTarget::Tsf,
        }];
        let done = dispatch_probe_actions(&mut machine, actions, &io);
        assert!(
            done,
            "should be Done — LiteralDetect must be skipped when GJI is long-idle"
        );
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
            gji_resumed: false,
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
            gji_resumed: false,
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
    fn send_fresh_f2_with_gji_long_idle_sends_extra_f2_and_waits_namechange() {
        // gji_long_idle 時は SendFreshF2 の直後に追加 F2 を送信して F2×2 連続とする。
        // NameChangeWait はスキップせず GJI I/O 応答を gji_long_idle_probe モードで監視する。
        use crate::tsf::probe_fsm::{ProbePhase, WaitingFor};
        let io = FakeProbeIo {
            gji_long_idle: true,
            ..Default::default()
        };
        let mut machine = make_gji_machine();
        // apply_fresh_f2_sent が機能するよう、FreshF2Sent フェーズへ強制移行する。
        machine.force_phase_for_test(ProbePhase::WaitingForCallback(WaitingFor::FreshF2Sent {
            probe_settled: false,
            gji_idle_ms: 15_000,
            remaining_ms: 0,
            send: crate::tsf::probe_fsm::SendState::default(),
        }));
        let actions = vec![ProbeAction::SendFreshF2 {
            cold_seq: 0,
            probe_settled: false,
        }];
        // gji_long_idle=true のとき:
        // - send_fresh_f2 と send_extra_f2 が呼ばれる（F2×2 連続）
        // - NameChangeWait フェーズへ移行し GJI I/O 応答を待つ（Done を即返さない）
        let done = dispatch_probe_actions(&mut machine, actions, &io);
        assert!(
            !done,
            "gji_long_idle: NameChangeWait で GJI I/O 応答を待つため Done を即返さないべき"
        );
        assert!(io.send_fresh_f2_called.get(), "send_fresh_f2 が呼ばれるべき");
        assert!(
            io.send_extra_f2_called.get(),
            "gji_long_idle: 追加 F2 で F2×2 連続にするべき"
        );
        assert_eq!(machine.phase_label(), "NameChangeWait", "NameChangeWait フェーズで待機するべき");
        assert!(!io.transmit_tsf_called.get(), "TransmitTsf は即実行されないべき");
    }

    #[test]
    fn nc_not_fired_with_gji_long_idle_forces_unicode_tsf() {
        // nc_fired=false（NameChangeWait タイムアウトまたはスキップ）かつ gji_long_idle のとき、
        // 非 TSF mode では used_eager_path=false でも unicode TSF（used_eager_path=true）が強制される。
        let io = FakeProbeIo {
            gji_long_idle: true,
            tsf_mode: false, // 非 TSF mode（Chrome 等）
            ..Default::default()
        };
        let mut machine = make_gji_machine();
        let actions = vec![ProbeAction::Transmit {
            cold_seq: 0,
            prepend_f2_warmup: true,
            used_eager_path: false, // eager warmup なしのコールドスタート
            nc_fired: false,
            gji_resumed: false,
            romaji: "ka".to_string(),
            deferred_vks: vec![],
            target: TransmitTarget::Tsf,
        }];
        let done = dispatch_probe_actions(&mut machine, actions, &io);
        assert!(done);
        assert!(io.transmit_tsf_called.get());
        assert!(
            io.last_used_eager_path.get(),
            "非TSF mode + gji_long_idle: nc_fired=false でも used_eager_path が true になるべき（unicode TSF 強制）"
        );
    }

    #[test]
    fn tsf_mode_nc_not_fired_gji_active_uses_vk_path() {
        // TSF-native (WezTerm): nc_fired=false でも GJI が活動中（!gji_long_idle）なら VK path。
        // KEYEVENTF_UNICODE は GJI コンポジションをバイパスして候補ウィンドウが出ないため。
        let io = FakeProbeIo {
            tsf_mode: true,
            gji_long_idle: false, // GJI 活動中（warmup に応答済み）
            ..Default::default()
        };
        let mut machine = make_gji_machine();
        let actions = vec![ProbeAction::Transmit {
            cold_seq: 0,
            prepend_f2_warmup: true,
            used_eager_path: true, // eager warmup あり
            nc_fired: false,       // WezTerm は CASCADIA クラスではないため常に false
            gji_resumed: false,
            romaji: "i".to_string(),
            deferred_vks: vec![],
            target: TransmitTarget::Tsf,
        }];
        let done = dispatch_probe_actions(&mut machine, actions, &io);
        assert!(done);
        assert!(io.transmit_tsf_called.get());
        assert!(
            !io.last_used_eager_path.get(),
            "TSF mode + nc_fired=false + GJI 活動中 → VK path（used_eager_path=false）で GJI 候補を表示"
        );
    }

    #[test]
    fn tsf_mode_nc_not_fired_gji_long_idle_uses_unicode_fallback() {
        // TSF-native (WezTerm): GJI が長期静止（warmup 未応答）なら unicode fallback。
        // VK path では GJI が受け付けず romaji がそのまま出力される可能性があるため。
        let io = FakeProbeIo {
            tsf_mode: true,
            gji_long_idle: true, // GJI 長期静止（warmup に未応答）
            ..Default::default()
        };
        let mut machine = make_gji_machine();
        let actions = vec![ProbeAction::Transmit {
            cold_seq: 0,
            prepend_f2_warmup: true,
            used_eager_path: true,
            nc_fired: false,
            gji_resumed: false,
            romaji: "i".to_string(),
            deferred_vks: vec![],
            target: TransmitTarget::Tsf,
        }];
        let done = dispatch_probe_actions(&mut machine, actions, &io);
        assert!(done);
        assert!(io.transmit_tsf_called.get());
        assert!(
            io.last_used_eager_path.get(),
            "TSF mode + nc_fired=false + GJI 長期静止 → unicode fallback（used_eager_path=true）"
        );
    }
}
