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
    /// `true` のとき `RawTsfLiteralRecovery` では `mark_cold_raw_tsf` を呼ばず warm を維持する
    /// ことで、`flush_raw_tsf_literal_romaji` の再送が F2 warmup なしの直接 VK 送信を通る
    /// （WezTerm の 344ms composition context タイマーをリセットしないため）。
    fn is_tsf_mode(&self) -> bool;
    /// 連続 raw TSF literal 回数を返す。
    fn consecutive_count(&self) -> u32;
    /// warm 状態を維持したまま連続カウントをインクリメントする（TSF mode 回収パス用）。
    fn increment_consecutive_count(&self);
    /// `RAW_TSF_LITERAL` グローバルを設定する（`consecutive == 0` のときのみ呼ばれる）。
    fn set_raw_literal(&self, backs: usize, romaji: String);
    /// composition を `RawTsfLiteralRecovery` で cold にマークする。
    fn mark_cold_raw_tsf(&self);
    /// `ProbeAction::Transmit` 完了時に `WarmupResult` を一時バッファに保存する。
    ///
    /// `Output::step_probe` が probe 完了を確認した後に取り出し、`GjiFsm::WarmupComplete` に変換する。
    fn store_gji_warmup_result(&self, result: crate::tsf::gji_fsm::WarmupResult);
    /// 現在実行中の GJI probe_id を返す（GjiFsm へ通知済みの ID）。
    ///
    /// `None` の場合は GjiFsm 未接続なので `store_gji_warmup_result` 呼び出しをスキップできる。
    fn current_gji_probe_id(&self) -> Option<crate::tsf::gji_fsm::ProbeId>;
    /// 犠牲キー（VK_A）を TSF パイプライン経由で送信する。
    ///
    /// `StartSacrificialWarmup` ハンドラが呼ぶ。F2 prepend なし・VK path で送信する。
    fn send_sacrificial_vk_a(&self, cold_seq: u32);
    /// BS×1 を送信する（犠牲キーの削除用）。
    ///
    /// `SacrificialResend` ハンドラが呼ぶ。
    fn send_sacrificial_bs_one(&self, cold_seq: u32);
}

impl ProbeIo for Output {
    fn gate_is_bypass(&self) -> bool {
        self.tsf_gate.state() == TsfGateState::Bypass
    }

    fn gji_monitor_healthy(&self) -> bool {
        crate::tsf::observer::gji_monitor_healthy()
    }

    fn transmit_tsf(
        &self,
        romaji: &str,
        chars: &[(VkCode, bool)],
        outcome: &WarmupOutcome,
    ) -> usize {
        let result = crate::output::TsfSendPipeline::transmit(romaji, chars, outcome);
        // unicode パスを使った場合（used_eager_path=true かつ kana が存在する）は
        // PendingGjiConfirm 状態に入る: GJI が I/O 応答するまで次の warm キーも unicode で送る。
        if outcome.used_eager_path
            && crate::tsf::output::kana_for_romaji_static(romaji).is_some()
        {
            let now = crate::hook::current_tick_ms();
            self.composition.set_last_unicode_transmit_ms(now);
            log::debug!(
                "[post-unicode] PendingGjiConfirm 開始: last_unicode_transmit_ms={now} romaji={romaji:?}"
            );
        }
        result
    }

    fn transmit_chrome(&self, romaji: &str, chars: &[(VkCode, bool)]) {
        Self::send_romaji_batch_immediate(romaji, chars);
    }

    fn send_deferred_vks(&self, vks: &[DeferredVk], marker: VkMarker) {
        let pairs: Vec<(VkCode, bool)> = vks.iter().map(|d| (d.vk, d.needs_shift)).collect();
        Self::send_deferred_probe_vks_from(&pairs, marker);
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

    fn increment_consecutive_count(&self) {
        self.composition.increment_consecutive_count();
    }

    fn set_raw_literal(&self, backs: usize, romaji: String) {
        self.record_raw_tsf_literal(backs, romaji);
    }

    fn mark_cold_raw_tsf(&self) {
        self.mark_composition_cold(ColdReason::RawTsfLiteralRecovery);
        self.pending_gji_composition_reset.set(true);
    }

    fn store_gji_warmup_result(&self, result: crate::tsf::gji_fsm::WarmupResult) {
        self.pending_gji_warmup.set(Some(result));
    }

    fn current_gji_probe_id(&self) -> Option<crate::tsf::gji_fsm::ProbeId> {
        self.current_gji_probe_id.get()
    }

    fn send_sacrificial_vk_a(&self, cold_seq: u32) {
        use awase::types::VkCode;
        use crate::tsf::output::make_key_input_ex;
        use crate::tsf::output::INJECTED_MARKER;
        use std::mem::size_of;
        use windows::Win32::UI::Input::KeyboardAndMouse::SendInput;
        use windows::Win32::UI::Input::KeyboardAndMouse::INPUT;
        const VK_A: VkCode = VkCode(0x41);
        let inputs = [
            make_key_input_ex(VK_A, false, INJECTED_MARKER),
            make_key_input_ex(VK_A, true, INJECTED_MARKER),
        ];
        log::debug!("[sacr-warmup] cold={cold_seq} VK_A 送信（犠牲キー）");
        unsafe {
            SendInput(
                &inputs,
                i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32"),
            );
        }
    }

    fn send_sacrificial_bs_one(&self, cold_seq: u32) {
        use crate::tsf::output::make_key_input_ex;
        use crate::tsf::output::INJECTED_MARKER;
        use crate::vk::VK_BACK;
        use std::mem::size_of;
        use windows::Win32::UI::Input::KeyboardAndMouse::SendInput;
        use windows::Win32::UI::Input::KeyboardAndMouse::INPUT;
        let inputs = [
            make_key_input_ex(VK_BACK, false, INJECTED_MARKER),
            make_key_input_ex(VK_BACK, true, INJECTED_MARKER),
        ];
        log::debug!("[sacr-warmup] cold={cold_seq} BS×1 送信（犠牲キー削除）");
        unsafe {
            SendInput(
                &inputs,
                i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32"),
            );
        }
    }
}

/// probe dispatcher の汎用実装。
/// `ProbeObservations` と `TransmitPlan` から `WarmupPath` を分類する純粋関数。
/// Tsf/Chrome の両 Transmit アームで共用する。
fn classify_warmup_path(
    obs: &crate::tsf::probe_fsm::ProbeObservations,
    plan: &crate::tsf::probe_fsm::TransmitPlan,
) -> crate::tsf::gji_fsm::WarmupPath {
    use crate::tsf::gji_fsm::WarmupPath;
    if obs.gji_resumed {
        WarmupPath::GjiResumed
    } else if obs.nc_fired {
        WarmupPath::NameChangeConfirmed
    } else if plan.used_eager_path {
        WarmupPath::EagerLiteralDetected
    } else {
        WarmupPath::TimedOutFallback
    }
}

/// `dispatch_probe_actions` の結果。
pub(crate) enum DispatchResult {
    /// probe 完了（タイマー停止）。
    Done,
    /// probe 継続（次回 tick を待つ）。
    Continue,
    /// 別の FSM に切り替える（`LiteralDetectFsm` 等）。
    SwitchMachine(Box<dyn crate::tsf::tickable_fsm::TickableFsm>),
}

impl DispatchResult {
    #[cfg(test)]
    pub(crate) fn is_done(&self) -> bool {
        matches!(self, Self::Done)
    }
}

///
/// `platform.rs` の `dispatch_probe_actions` を置き換える。
/// `io: &impl ProbeIo` で Win32 副作用を注入することでテスト可能。
#[expect(clippy::too_many_lines)]
#[allow(clippy::cognitive_complexity)]
pub(crate) fn dispatch_probe_actions<M, I>(
    machine: &mut M,
    initial_actions: Vec<crate::tsf::probe_fsm::ProbeAction>,
    io: &I,
) -> DispatchResult
where
    M: crate::tsf::tickable_fsm::TickableFsm + ?Sized,
    I: ProbeIo,
{
    use crate::tsf::probe_fsm::{ProbeAction, TransmitTarget};
    use std::collections::VecDeque;

    let mut queue: VecDeque<ProbeAction> = initial_actions.into();

    while let Some(action) = queue.pop_front() {
        match action {
            ProbeAction::Done => return DispatchResult::Done,

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
                if machine.forces_prepend_f2_for_extra_f2() {
                    // Medium/Long cold: F2 単発では GJI が I/O を出さない。F2×2 連続で GJI を起動させる。
                    // NameChangeWait 内の gji_long_idle_probe が GJI I/O 応答を監視し、
                    // GJI_IDLE_MS 静止確認後に VK path へ移行する。
                    log::debug!("[tsf-probe] cold={cold_seq} forces_prepend_f2: 追加 F2 送信 (F2×2 連続で GJI 起動)");
                    io.send_extra_f2();
                }
                machine.apply_fresh_f2_sent(nc_baseline, fresh_f2_ms);
            }

            ProbeAction::Transmit {
                cold_seq,
                plan,
                observations,
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
                            return DispatchResult::Done;
                        }
                        if chars.is_empty() {
                            return DispatchResult::Done;
                        }
                        // plan は FSM の enter_transmit_tsf が confirm 時点の env で確定済み。
                        // dispatcher は再導出せずそのまま使う。
                        let outcome = WarmupOutcome {
                            prepend_f2_warmup: plan.should_prepend_f2,
                            used_eager_path: plan.used_eager_path,
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
                        let detector = plan
                            .needs_literal
                            .then(crate::tsf::probe::LiteralDetector::new);
                        // TSF cold path の部分リテラル検出: SHOW 発火時に IMM32 composition と突き合わせる。
                        // K がリテラル化して O だけが compose された場合（"ko"→'k'+'お'）を
                        // expected_kana='こ' vs actual='お' の不一致で検出する。
                        let expected_kana = crate::tsf::output::kana_for_romaji_static(&romaji);
                        let ze_bs_count = io.transmit_tsf(&romaji, &chars, &outcome);
                        io.send_deferred_vks(&deferred_vks, VkMarker::Tsf);
                        // GjiFsm bridge: 送信完了時の warmup 結果を一時バッファに保存する。
                        // step_probe が probe 完了を確認した後に取り出して WarmupComplete に変換する。
                        if io.current_gji_probe_id().is_some() {
                            use crate::tsf::gji_fsm::WarmupResult;
                            io.store_gji_warmup_result(WarmupResult {
                                path: classify_warmup_path(&observations, &plan),
                                prepend_f2_warmup: plan.should_prepend_f2,
                                nc_fired: observations.nc_fired,
                                gji_resumed: observations.gji_resumed,
                            });
                        }
                        if machine.apply_transmit_done(romaji, ze_bs_count, detector, plan.literal_detect_ms, expected_kana) {
                            return DispatchResult::Done;
                        }
                    }
                    TransmitTarget::Chrome => {
                        // plan.needs_literal は enter_transmit_chrome が env.gji_active で確定済み。
                        // 検出ベースラインは送信前に確定させること。
                        // Chrome 経由では GJI が VK を処理すると辞書 I/O が発生し gji_last_io_ms が
                        // 更新される。gji_candidate_show はシンプルなかな（「や」など）では発火しないため
                        // new_gji_resumed() を使って I/O 変化を確認シグナルとする。
                        // これにより「ya→や」等で false SuspectedLiteral が発生し BS×2 + 再送が
                        // ループするバグを防ぐ。
                        let detector = plan
                            .needs_literal
                            .then(crate::tsf::probe::LiteralDetector::new_gji_resumed);
                        let ze_bs_count = chars.len();
                        io.transmit_chrome(&romaji, &chars);
                        io.send_deferred_vks(&deferred_vks, VkMarker::Injected);
                        // GjiFsm bridge: Chrome 経由でも同様に warmup 結果を保存する。
                        if io.current_gji_probe_id().is_some() {
                            use crate::tsf::gji_fsm::WarmupResult;
                            io.store_gji_warmup_result(WarmupResult {
                                path: classify_warmup_path(&observations, &plan),
                                prepend_f2_warmup: plan.should_prepend_f2,
                                nc_fired: observations.nc_fired,
                                gji_resumed: observations.gji_resumed,
                            });
                        }
                        if machine.apply_transmit_done(romaji, ze_bs_count, detector, plan.literal_detect_ms, None) {
                            return DispatchResult::Done;
                        }
                    }
                }
            }

            ProbeAction::StartLiteralDetect(config) => {
                // GjiWarmupFsm から emit される（plan.needs_literal=true の場合）。
                // TSF 送信を実行してから LiteralDetectFsm に切り替える。
                let chars: VkSequence = config.romaji
                    .chars()
                    .filter_map(crate::output::resolve_ascii_to_vk)
                    .collect();
                if io.gate_is_bypass() {
                    log::debug!(
                        "[tsf-probe] cold={} StartLiteralDetect: gate=Bypass, skipping",
                        config.cold_seq
                    );
                    return DispatchResult::Done;
                }
                if chars.is_empty() {
                    return DispatchResult::Done;
                }
                let outcome = WarmupOutcome {
                    prepend_f2_warmup: config.plan.should_prepend_f2,
                    used_eager_path: config.plan.used_eager_path,
                    cold_seq: config.cold_seq,
                };
                if io.current_gji_probe_id().is_some() {
                    use crate::tsf::gji_fsm::WarmupResult;
                    io.store_gji_warmup_result(WarmupResult {
                        path: classify_warmup_path(&config.observations, &config.plan),
                        prepend_f2_warmup: config.plan.should_prepend_f2,
                        nc_fired: config.observations.nc_fired,
                        gji_resumed: config.observations.gji_resumed,
                    });
                }
                let ze_bs_count = io.transmit_tsf(&config.romaji, &chars, &outcome);
                io.send_deferred_vks(&config.deferred_vks, VkMarker::Tsf);
                log::debug!(
                    "[tsf-probe] cold={} StartLiteralDetect → LiteralDetectFsm (ze_bs={})",
                    config.cold_seq, ze_bs_count
                );
                let literal_fsm = crate::tsf::literal_detect_fsm::LiteralDetectFsm::new(
                    config.cold_seq,
                    config.romaji,
                    config.deferred_vks,
                    config.plan,
                    config.observations,
                    ze_bs_count,
                    config.literal_detect_ms,
                );
                return DispatchResult::SwitchMachine(Box::new(literal_fsm));
            }

            ProbeAction::StartSacrificialWarmup(config) => {
                // GjiWarmupFsm から emit される（plan.needs_literal=true の場合）。
                // 実ローマ字を即送信する代わりに VK_A（犠牲キー）を送信し、
                // composition 確認後に BS×1 + 実ローマ字を再送する。
                // これにより実ローマ字が readline バッファにリテラル状態で残らない
                // （Engine ON / IME OFF 状態を構造的に排除する）。
                let real_chars: VkSequence = config.romaji
                    .chars()
                    .filter_map(crate::output::resolve_ascii_to_vk)
                    .collect();
                // Chrome は常に gate=Bypass のため Chrome target の場合はゲートチェックをスキップする。
                // TSF/WezTerm の場合のみ bypass 状態でスキップする。
                if config.target != TransmitTarget::Chrome && io.gate_is_bypass() {
                    log::debug!(
                        "[sacr-warmup] cold={} StartSacrificialWarmup: gate=Bypass, skipping",
                        config.cold_seq
                    );
                    return DispatchResult::Done;
                }
                if real_chars.is_empty() {
                    return DispatchResult::Done;
                }
                // GjiFsm bridge: 犠牲キー送信時点で warmup 結果を記録する。
                if io.current_gji_probe_id().is_some() {
                    use crate::tsf::gji_fsm::WarmupResult;
                    io.store_gji_warmup_result(WarmupResult {
                        path: classify_warmup_path(&config.observations, &config.plan),
                        prepend_f2_warmup: config.plan.should_prepend_f2,
                        nc_fired: config.observations.nc_fired,
                        gji_resumed: config.observations.gji_resumed,
                    });
                }
                // VK_A（犠牲キー）を送信。TSF warm なら 'あ' formation、cold なら 'a' リテラル。
                io.send_sacrificial_vk_a(config.cold_seq);
                log::debug!(
                    "[sacr-warmup] cold={} VK_A 送信完了 → SacrificialWarmupFsm 開始 \
                    (romaji={:?} literal_detect={}ms)",
                    config.cold_seq, config.romaji, config.literal_detect_ms,
                );
                let sacr_fsm = crate::tsf::sacr_warmup_fsm::SacrificialWarmupFsm::new(
                    config.cold_seq,
                    config.romaji,
                    config.deferred_vks,
                    config.literal_detect_ms,
                    config.target,
                );
                return DispatchResult::SwitchMachine(Box::new(sacr_fsm));
            }

            ProbeAction::SacrificialResend(resend) => {
                // SacrificialWarmupFsm から emit される（composition 確認後）。
                // BS×1（犠牲 VK_A 削除）→ 実ローマ字送信 → deferred_vks 送信。
                // target に応じて Chrome/TSF パスを切り替える。
                let cold_seq = resend.cold_seq;
                let chars: VkSequence = resend.romaji
                    .chars()
                    .filter_map(crate::output::resolve_ascii_to_vk)
                    .collect();
                // Chrome は常に gate=Bypass のため Chrome target の場合はゲートチェックをスキップする。
                if chars.is_empty() || (resend.target != TransmitTarget::Chrome && io.gate_is_bypass()) {
                    // ゲートが閉じている or 実ローマ字なし: BS も送らず即終了
                    log::debug!("[sacr-warmup] cold={cold_seq} SacrificialResend: skip (bypass or empty)");
                } else {
                    // BS×1: 犠牲 VK_A（'a' または 'あ' composition unit）を削除
                    io.send_sacrificial_bs_one(cold_seq);
                    match resend.target {
                        TransmitTarget::Chrome => {
                            // Chrome パス: INJECTED_MARKER バッチ送信
                            log::debug!(
                                "[sacr-warmup] cold={cold_seq} 実ローマ字 {:?} を Chrome パスで再送",
                                resend.romaji
                            );
                            io.transmit_chrome(&resend.romaji, &chars);
                            io.send_deferred_vks(&resend.deferred_vks, VkMarker::Injected);
                        }
                        TransmitTarget::Tsf => {
                            // TSF パス: warm 維持のまま VK run 送信（F2 prepend なし）
                            // WezTerm の 344ms composition context タイマーをリセットしない。
                            let outcome = WarmupOutcome {
                                prepend_f2_warmup: false,
                                used_eager_path: false,
                                cold_seq,
                            };
                            log::debug!(
                                "[sacr-warmup] cold={cold_seq} 実ローマ字 {:?} を warm パスで再送",
                                resend.romaji
                            );
                            io.transmit_tsf(&resend.romaji, &chars, &outcome);
                            io.send_deferred_vks(&resend.deferred_vks, VkMarker::Tsf);
                        }
                    }
                }
                // SacrificialWarmupFsm は Done を後続 action として emit しているため
                // ここでは machine 状態を更新せず Continue を返す（queue が Done を処理する）。
            }

            ProbeAction::RawTsfLiteralRecovery {
                cold_seq,
                backs,
                romaji,
            } => {
                let consecutive = io.consecutive_count();
                if consecutive == 0 {
                    if io.is_tsf_mode() {
                        // TSF mode (WezTerm): warm を維持して再送。
                        // mark_cold_raw_tsf を呼ぶと flush_raw_tsf_literal_romaji が
                        // cold 経路で F2 warmup を再送し WezTerm の 344ms タイマーをリセットする。
                        // warm のまま即 VK 再送することでタイマーリセットを回避する。
                        log::warn!(
                            "[raw-tsf-literal] cold={cold_seq} TSF mode literal suspected \
                            → backspace ×{backs} + re-send {romaji:?} scheduled (warm maintained)"
                        );
                        io.set_raw_literal(backs, romaji);
                        io.increment_consecutive_count();
                    } else {
                        log::warn!(
                            "[raw-tsf-literal] cold={cold_seq} raw TSF literal suspected \
                            → backspace ×{backs} + re-send {romaji:?} scheduled \
                            + mark cold"
                        );
                        io.set_raw_literal(backs, romaji);
                        io.mark_cold_raw_tsf();
                    }
                } else {
                    log::warn!(
                        "[raw-tsf-literal] cold={cold_seq} consecutive raw-tsf-literal \
                        (count={}) → likely false positive, giving up",
                        consecutive + 1,
                    );
                    io.mark_cold_raw_tsf();
                }
            }
        }
    }
    DispatchResult::Continue
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tsf::probe_bridge::OutputActiveGuard;
    use crate::tsf::probe_fsm::{ProbeAction, ProbeObservations, TransmitPlan, TransmitTarget};
    use std::cell::Cell;

    /// テスト用フェイク ProbeIo。Win32 副作用を no-op にし、呼び出しをフラグで記録する。
    struct FakeProbeIo {
        bypass: bool,
        gji_healthy: bool,
        tsf_mode: bool,
        tsf_transmit_result: usize,
        consecutive: u32,
        transmit_tsf_called: Cell<bool>,
        transmit_chrome_called: Cell<bool>,
        deferred_vks_called: Cell<bool>,
        send_fresh_f2_called: Cell<bool>,
        send_extra_f2_called: Cell<bool>,
        set_raw_literal_called: Cell<bool>,
        mark_cold_raw_tsf_called: Cell<bool>,
        increment_consecutive_called: Cell<bool>,
        /// transmit_tsf に渡された WarmupOutcome.used_eager_path を記録する。
        last_used_eager_path: Cell<bool>,
        /// transmit_tsf に渡された WarmupOutcome.prepend_f2_warmup を記録する。
        last_used_prepend_f2: Cell<bool>,
    }

    impl Default for FakeProbeIo {
        fn default() -> Self {
            Self {
                bypass: false,
                gji_healthy: false,
                tsf_mode: false,
                tsf_transmit_result: 1,
                consecutive: 0,
                transmit_tsf_called: Cell::new(false),
                transmit_chrome_called: Cell::new(false),
                deferred_vks_called: Cell::new(false),
                send_fresh_f2_called: Cell::new(false),
                send_extra_f2_called: Cell::new(false),
                set_raw_literal_called: Cell::new(false),
                mark_cold_raw_tsf_called: Cell::new(false),
                increment_consecutive_called: Cell::new(false),
                last_used_eager_path: Cell::new(false),
                last_used_prepend_f2: Cell::new(false),
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
            self.last_used_prepend_f2.set(outcome.prepend_f2_warmup);
            self.tsf_transmit_result
        }
        fn transmit_chrome(&self, _romaji: &str, _chars: &[(VkCode, bool)]) {
            self.transmit_chrome_called.set(true);
        }
        fn send_deferred_vks(&self, _vks: &[DeferredVk], _marker: VkMarker) {
            self.deferred_vks_called.set(true);
        }
        fn send_fresh_f2(&self) -> (NamechangeBaseline, u64) {
            self.send_fresh_f2_called.set(true);
            (crate::tsf::observer::namechange_baseline(), 0)
        }
        fn send_extra_f2(&self) {
            self.send_extra_f2_called.set(true);
        }
        fn consecutive_count(&self) -> u32 {
            self.consecutive
        }
        fn increment_consecutive_count(&self) {
            self.increment_consecutive_called.set(true);
        }
        fn set_raw_literal(&self, _backs: usize, _romaji: String) {
            self.set_raw_literal_called.set(true);
        }
        fn mark_cold_raw_tsf(&self) {
            self.mark_cold_raw_tsf_called.set(true);
        }

        fn store_gji_warmup_result(&self, _result: crate::tsf::gji_fsm::WarmupResult) {}

        fn current_gji_probe_id(&self) -> Option<crate::tsf::gji_fsm::ProbeId> {
            None
        }

        fn send_sacrificial_vk_a(&self, _cold_seq: u32) {}

        fn send_sacrificial_bs_one(&self, _cold_seq: u32) {}
    }

    fn make_chrome_machine() -> crate::tsf::probe_fsm::TsfProbeMachine {
        let guard = OutputActiveGuard::noop_for_test();
        let probe = crate::tsf::probe::TsfReadinessProbe::new(0, 0, 0);
        crate::tsf::probe_fsm::TsfProbeMachine::new_chrome("ka", 0, probe, 0, guard)
    }

    fn make_gji_machine() -> crate::tsf::probe_fsm::TsfProbeMachine {
        make_gji_machine_with_cold(crate::tuning::SETTLE_TIMEOUT_MS, false)
    }

    fn make_gji_machine_with_cold(ncwait_budget_ms: u64, forces_prepend_f2: bool) -> crate::tsf::probe_fsm::TsfProbeMachine {
        let is_long_cold = ncwait_budget_ms == crate::tuning::GJI_LONG_IDLE_PROBE_TOTAL_MS;
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
            ncwait_budget_ms,
            forces_prepend_f2,
            is_long_cold,
            false,
        )
    }

    #[test]
    fn done_action_returns_true_without_side_effects() {
        let io = FakeProbeIo::default();
        let mut machine = make_chrome_machine();
        let result = dispatch_probe_actions(&mut machine,vec![ProbeAction::Done], &io);
        assert!(result.is_done());
        assert!(!io.transmit_tsf_called.get());
        assert!(!io.transmit_chrome_called.get());
    }

    #[test]
    fn chrome_transmit_calls_transmit_chrome_and_mark_warm() {
        let io = FakeProbeIo::default();
        let mut machine = make_chrome_machine();
        let actions = vec![ProbeAction::Transmit {
            cold_seq: 0,
            plan: TransmitPlan {
                should_prepend_f2: false,
                used_eager_path: false,
                needs_literal: false,
                literal_detect_ms: crate::tuning::RAW_TSF_LITERAL_DETECT_MS,
            },
            observations: ProbeObservations { nc_fired: true, gji_resumed: false },
            romaji: "ka".to_string(),
            deferred_vks: vec![],
            target: TransmitTarget::Chrome,
        }];
        let result = dispatch_probe_actions(&mut machine, actions, &io);
        assert!(result.is_done());
        assert!(io.transmit_chrome_called.get());
        assert!(!io.transmit_tsf_called.get());
    }

    #[test]
    fn chrome_transmit_with_gji_healthy_installs_literal_detect() {
        // plan.needs_literal=true のとき Chrome バッチ送信後も LiteralDetect フェーズへ遷移し、
        // Done を即返さないことで literal 検出のための再ティックを許可する。
        let io = FakeProbeIo::default();
        let mut machine = make_chrome_machine();
        let actions = vec![ProbeAction::Transmit {
            cold_seq: 0,
            plan: TransmitPlan {
                should_prepend_f2: false,
                used_eager_path: false,
                needs_literal: true, // enter_transmit_chrome が gji_active=true のとき設定
                literal_detect_ms: crate::tuning::RAW_TSF_LITERAL_DETECT_MS,
            },
            observations: ProbeObservations { nc_fired: true, gji_resumed: false },
            romaji: "ka".to_string(),
            deferred_vks: vec![],
            target: TransmitTarget::Chrome,
        }];
        let result = dispatch_probe_actions(&mut machine, actions, &io);
        assert!(!result.is_done(), "should not be Done — LiteralDetect phase pending");
        assert!(io.transmit_chrome_called.get());
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
            plan: TransmitPlan {
                should_prepend_f2: false,
                used_eager_path: false,
                needs_literal: false,
                literal_detect_ms: crate::tuning::RAW_TSF_LITERAL_DETECT_MS,
            },
            observations: ProbeObservations { nc_fired: true, gji_resumed: false },
            romaji: "ka".to_string(),
            deferred_vks: vec![],
            target: TransmitTarget::Tsf,
        }];
        let result = dispatch_probe_actions(&mut machine, actions, &io);
        assert!(result.is_done());
        assert!(!io.transmit_tsf_called.get());
    }

    #[test]
    fn tsf_transmit_skips_literal_detect_when_gji_long_idle() {
        // plan.needs_literal=false のとき (gji_long_idle で decide_transmit_plan が設定)
        // LiteralDetect を入れない → Done を即返す。
        let io = FakeProbeIo::default();
        let mut machine = make_gji_machine();
        let actions = vec![ProbeAction::Transmit {
            cold_seq: 0,
            plan: TransmitPlan {
                should_prepend_f2: true,
                used_eager_path: true, // nc_fired=true + gji_long_idle=true
                needs_literal: false,  // gji_long_idle + !is_tsf_mode → false
                literal_detect_ms: crate::tuning::RAW_TSF_LITERAL_DETECT_MS,
            },
            observations: ProbeObservations { nc_fired: true, gji_resumed: false },
            romaji: "ka".to_string(),
            deferred_vks: vec![],
            target: TransmitTarget::Tsf,
        }];
        let result = dispatch_probe_actions(&mut machine, actions, &io);
        assert!(
            result.is_done(),
            "should be Done — LiteralDetect must be skipped when GJI is long-idle"
        );
        assert!(io.transmit_tsf_called.get());
    }

    #[test]
    fn tsf_transmit_calls_transmit_tsf_and_mark_warm() {
        let io = FakeProbeIo::default();
        let mut machine = make_gji_machine();
        let actions = vec![ProbeAction::Transmit {
            cold_seq: 0,
            plan: TransmitPlan {
                should_prepend_f2: false,
                used_eager_path: false,
                needs_literal: false,
                literal_detect_ms: crate::tuning::RAW_TSF_LITERAL_DETECT_MS,
            },
            observations: ProbeObservations { nc_fired: true, gji_resumed: false },
            romaji: "ka".to_string(),
            deferred_vks: vec![],
            target: TransmitTarget::Tsf,
        }];
        let result = dispatch_probe_actions(&mut machine, actions, &io);
        assert!(result.is_done());
        assert!(io.transmit_tsf_called.get());
        assert!(!io.transmit_chrome_called.get());
    }

    #[test]
    fn tsf_transmit_uses_eager_path_when_nc_not_fired_even_with_deferred_vks() {
        // nc_fired=false のとき、decide_transmit_plan は deferred_vks に関わらず
        // used_eager_path=true（nc_fired=true branch の deferred_vks チェックは通らない）。
        // WarmupOutcome.used_eager_path=true が transmit_tsf に渡ることを確認する。
        let io = FakeProbeIo::default();
        let mut machine = make_gji_machine();
        let actions = vec![ProbeAction::Transmit {
            cold_seq: 0,
            plan: TransmitPlan {
                should_prepend_f2: true,
                used_eager_path: true, // nc_fired=false + non-tsf → initial_used_eager || gji_long_idle
                needs_literal: false,
                literal_detect_ms: crate::tuning::RAW_TSF_LITERAL_DETECT_MS,
            },
            observations: ProbeObservations { nc_fired: false, gji_resumed: false },
            romaji: "ki".to_string(),
            deferred_vks: vec![crate::tsf::probe_fsm::DeferredVk {
                vk: awase::types::VkCode(0x49), // VK_I
                needs_shift: false,
            }],
            target: TransmitTarget::Tsf,
        }];
        let result = dispatch_probe_actions(&mut machine, actions, &io);
        assert!(result.is_done());
        assert!(io.transmit_tsf_called.get());
        assert!(
            io.last_used_eager_path.get(),
            "plan.used_eager_path=true は WarmupOutcome に反映されるべき"
        );
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
        let result = dispatch_probe_actions(&mut machine, actions, &io);
        assert!(result.is_done());
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
        let result = dispatch_probe_actions(&mut machine, actions, &io);
        assert!(result.is_done());
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
        let result = dispatch_probe_actions(&mut machine, actions, &io);
        assert!(!result.is_done());
        assert!(io.send_fresh_f2_called.get());
    }

    #[test]
    fn send_fresh_f2_with_gji_long_idle_sends_extra_f2_and_waits_namechange() {
        // forces_prepend_f2=true (Long cold) 時は SendFreshF2 の直後に追加 F2 を送信して F2×2 連続とする。
        // NameChangeWait はスキップせず GJI I/O 応答を gji_long_idle_probe モードで監視する。
        use crate::tsf::probe_fsm::{ProbePhase, WaitingFor};
        let io = FakeProbeIo::default();
        let mut machine = make_gji_machine_with_cold(crate::tuning::GJI_LONG_IDLE_PROBE_TOTAL_MS, true);
        // apply_fresh_f2_sent が機能するよう、FreshF2Sent フェーズへ強制移行する。
        machine.force_phase_for_test(ProbePhase::WaitingForCallback(WaitingFor::FreshF2Sent {
            probe_settled: false,
            budget_ms: crate::tuning::GJI_LONG_IDLE_PROBE_TOTAL_MS,
            send: crate::tsf::probe_fsm::SendState::default(),
        }));
        let actions = vec![ProbeAction::SendFreshF2 {
            cold_seq: 0,
            probe_settled: false,
        }];
        // forces_prepend_f2=true (Long cold) のとき:
        // - send_fresh_f2 と send_extra_f2 が呼ばれる（F2×2 連続）
        // - NameChangeWait フェーズへ移行し GJI I/O 応答を待つ（Done を即返さない）
        let result = dispatch_probe_actions(&mut machine, actions, &io);
        assert!(
            !result.is_done(),
            "forces_prepend_f2: NameChangeWait で GJI I/O 応答を待つため Done を即返さないべき"
        );
        assert!(io.send_fresh_f2_called.get(), "send_fresh_f2 が呼ばれるべき");
        assert!(
            io.send_extra_f2_called.get(),
            "forces_prepend_f2: 追加 F2 で F2×2 連続にするべき"
        );
        assert_eq!(machine.phase_label(), "NameChangeWait", "NameChangeWait フェーズで待機するべき");
        assert!(!io.transmit_tsf_called.get(), "TransmitTsf は即実行されないべき");
    }

    #[test]
    fn send_fresh_f2_with_medium_cold_sends_extra_f2_and_waits_namechange() {
        // 再現テスト: ColdKind::Medium（7s〜10s idle）
        // cold=7 "このろぐ → kおのろぐ" バグ: GJI が fresh F2 から 325ms 後に起動するため
        // SETTLE_TIMEOUT_MS (300ms) では間に合わず "kお" になっていた。
        // gji_long_idle_probe=true + MEDIUM_IDLE_PROBE_TOTAL_MS (550ms) で GJI I/O を待てること。
        // forces_prepend_f2=true だが Long ではないので追加 F2 なし（F2×1 のみ）。
        // ※ Medium の forces_prepend_f2=true は「F2×2 を強制」ではなく「gji_long_idle_probe=true」の意味。
        use crate::tsf::probe_fsm::{ProbePhase, WaitingFor};
        let io = FakeProbeIo::default();
        // Medium cold: forces_prepend_f2=true (gji_long_idle_probe 有効), budget=MEDIUM_IDLE_PROBE_TOTAL_MS
        let mut machine = make_gji_machine_with_cold(crate::tuning::MEDIUM_IDLE_PROBE_TOTAL_MS, true);
        machine.force_phase_for_test(ProbePhase::WaitingForCallback(WaitingFor::FreshF2Sent {
            probe_settled: false,
            budget_ms: crate::tuning::MEDIUM_IDLE_PROBE_TOTAL_MS,
            send: crate::tsf::probe_fsm::SendState::default(),
        }));
        let actions = vec![ProbeAction::SendFreshF2 {
            cold_seq: 0,
            probe_settled: false,
        }];
        let result = dispatch_probe_actions(&mut machine, actions, &io);
        assert!(!result.is_done(), "medium idle: NameChangeWait で GJI I/O 応答を待つため Done を即返さないべき");
        assert!(io.send_fresh_f2_called.get(), "send_fresh_f2 が呼ばれるべき");
        assert!(
            io.send_extra_f2_called.get(),
            "medium idle (forces_prepend_f2=true): F2×2 を送るべき"
        );
        assert_eq!(machine.phase_label(), "NameChangeWait", "NameChangeWait フェーズで GJI I/O を監視するべき");
    }

    #[test]
    fn nc_not_fired_with_gji_long_idle_forces_unicode_tsf() {
        // nc_fired=false（NameChangeWait タイムアウトまたはスキップ）かつ gji_long_idle のとき、
        // 非 TSF mode では used_eager_path=false でも unicode TSF（used_eager_path=true）が強制される。
        let io = FakeProbeIo::default();
        let mut machine = make_gji_machine();
        let actions = vec![ProbeAction::Transmit {
            cold_seq: 0,
            plan: TransmitPlan {
                should_prepend_f2: true,
                // nc_fired=false + non-tsf → initial_used_eager || gji_long_idle = false || true = true
                used_eager_path: true,
                needs_literal: false,
                literal_detect_ms: crate::tuning::RAW_TSF_LITERAL_DETECT_MS,
            },
            observations: ProbeObservations { nc_fired: false, gji_resumed: false },
            romaji: "ka".to_string(),
            deferred_vks: vec![],
            target: TransmitTarget::Tsf,
        }];
        let result = dispatch_probe_actions(&mut machine, actions, &io);
        assert!(result.is_done());
        assert!(io.transmit_tsf_called.get());
        assert!(
            io.last_used_eager_path.get(),
            "plan.used_eager_path=true は WarmupOutcome に反映されるべき"
        );
    }

    #[test]
    fn tsf_mode_nc_not_fired_gji_active_uses_vk_path() {
        // decide_transmit_plan: nc_fired=false + is_tsf_mode=true → used_eager_path=false (VK path)。
        // KEYEVENTF_UNICODE は GJI コンポジションをバイパスして候補ウィンドウが出ないため TSF mode では使わない。
        // prepend_f2_warmup + nc_fired=false + !is_tsf_mode → should_prepend_f2=false。
        let io = FakeProbeIo::default();
        let mut machine = make_gji_machine();
        let actions = vec![ProbeAction::Transmit {
            cold_seq: 0,
            plan: TransmitPlan {
                // nc_fired=false + is_tsf_mode=true + !gji_long_idle → should_prepend_f2=false
                should_prepend_f2: false,
                used_eager_path: false, // is_tsf_mode=true → VK path
                needs_literal: false,
                literal_detect_ms: crate::tuning::RAW_TSF_LITERAL_DETECT_MS,
            },
            observations: ProbeObservations { nc_fired: false, gji_resumed: false },
            romaji: "i".to_string(),
            deferred_vks: vec![],
            target: TransmitTarget::Tsf,
        }];
        let result = dispatch_probe_actions(&mut machine, actions, &io);
        assert!(result.is_done());
        assert!(io.transmit_tsf_called.get());
        assert!(
            !io.last_used_eager_path.get(),
            "plan.used_eager_path=false は WarmupOutcome に反映されるべき"
        );
    }

    #[test]
    fn tsf_mode_nc_not_fired_gji_long_idle_uses_vk_path() {
        // decide_transmit_plan: nc_fired=false + is_tsf_mode=true → used_eager_path=false (VK path)。
        // gji_long_idle=true でも TSF mode では KEYEVENTF_UNICODE による "nお" race を避けるため VK path。
        // gji_active=false (default) → needs_literal=false → done=true。
        let io = FakeProbeIo::default();
        let mut machine = make_gji_machine();
        let actions = vec![ProbeAction::Transmit {
            cold_seq: 0,
            plan: TransmitPlan {
                // nc_fired=false + is_tsf_mode=true + gji_long_idle=true → should_prepend_f2=true
                should_prepend_f2: true,
                used_eager_path: false, // is_tsf_mode=true → VK path
                needs_literal: false,   // gji_active=false → false
                literal_detect_ms: crate::tuning::RAW_TSF_LITERAL_DETECT_MS_LONG_IDLE,
            },
            observations: ProbeObservations { nc_fired: false, gji_resumed: false },
            romaji: "i".to_string(),
            deferred_vks: vec![],
            target: TransmitTarget::Tsf,
        }];
        let result = dispatch_probe_actions(&mut machine, actions, &io);
        assert!(result.is_done());
        assert!(io.transmit_tsf_called.get());
        assert!(
            !io.last_used_eager_path.get(),
            "plan.used_eager_path=false (VK path) は WarmupOutcome に反映されるべき"
        );
    }

    #[test]
    fn tsf_mode_nc_not_fired_gji_long_idle_gji_healthy_enables_literal_detect() {
        // decide_transmit_plan: nc_fired=false + is_tsf_mode=true + gji_active=true + gji_long_idle=true
        // → should_prepend_f2=true, used_eager_path=false (VK), needs_literal=true (TSF mode override)。
        // VK path でリテラル化した場合に BS 再送で回収できるよう LiteralDetect を有効化する。
        let io = FakeProbeIo::default();
        let mut machine = make_gji_machine();
        let actions = vec![ProbeAction::Transmit {
            cold_seq: 0,
            plan: TransmitPlan {
                should_prepend_f2: true,
                used_eager_path: false,  // is_tsf_mode → VK path
                needs_literal: true,     // should_prepend_f2 && gji_active && (!gji_long_idle || is_tsf_mode) && !gji_resumed
                literal_detect_ms: crate::tuning::RAW_TSF_LITERAL_DETECT_MS_LONG_IDLE,
            },
            observations: ProbeObservations { nc_fired: false, gji_resumed: false },
            romaji: "ko".to_string(),
            deferred_vks: vec![],
            target: TransmitTarget::Tsf,
        }];
        let result = dispatch_probe_actions(&mut machine, actions, &io);
        assert!(
            !result.is_done(),
            "plan.needs_literal=true → LiteralDetect phase: Done を即返さないべき"
        );
        assert!(io.transmit_tsf_called.get());
        assert!(
            !io.last_used_eager_path.get(),
            "plan.used_eager_path=false (VK path) は WarmupOutcome に反映されるべき"
        );
        assert_eq!(machine.phase_label(), "LiteralDetect");
    }

    #[test]
    fn tsf_mode_cold_start_nc_not_fired_not_long_idle_skips_f2_and_literal_detect() {
        // nc_fired=false + is_tsf_mode=true + !gji_long_idle:
        // decide_transmit_plan: should_prepend_f2 = prepend_f2_warmup && (nc_fired || !is_tsf_mode || gji_long_idle)
        //   = true && (false || false || false) = false → F2 をバッチに含めない。
        // SendFreshF2 が ~300ms 前に fresh F2 を送信済み → 再び含めると TSF reinit race (Bug 1)。
        // should_prepend_f2=false → needs_literal=false → done=true。
        let io = FakeProbeIo::default();
        let mut machine = make_gji_machine();
        let actions = vec![ProbeAction::Transmit {
            cold_seq: 0,
            plan: TransmitPlan {
                should_prepend_f2: false, // nc_fired=false + is_tsf_mode + !gji_long_idle → false
                used_eager_path: false,
                needs_literal: false,
                literal_detect_ms: crate::tuning::RAW_TSF_LITERAL_DETECT_MS,
            },
            observations: ProbeObservations { nc_fired: false, gji_resumed: false },
            romaji: "ko".to_string(),
            deferred_vks: vec![],
            target: TransmitTarget::Tsf,
        }];
        let result = dispatch_probe_actions(&mut machine, actions, &io);
        assert!(
            result.is_done(),
            "plan.should_prepend_f2=false + plan.needs_literal=false → Done を即返す"
        );
        assert!(io.transmit_tsf_called.get());
        assert!(
            !io.last_used_prepend_f2.get(),
            "plan.should_prepend_f2=false は WarmupOutcome.prepend_f2_warmup に反映されるべき"
        );
    }

    #[test]
    fn gji_resumed_skips_literal_detect_to_prevent_false_positive() {
        // gji_resumed=true: F2×2 送信後に GJI I/O 応答を確認済み → VK composition は成功する。
        // long_idle 後の候補ウィンドウ表示に 500ms 超かかるため LiteralDetect タイムアウトが
        // false positive (BS 誤送信) になる。gji_resumed=true では LiteralDetect をスキップする。
        //
        // 実機ログ: WezTerm long_idle(120s) + NativeF2Consumed → cold=72 で 'と' が正常 compose されたが
        // SuspectedLiteral 誤判定 → BS×2 で 'と' 削除 → 後続打鍵 'つ' の composition context 破壊
        // → IME-OFF Engine-ON 状態になった（2026-06-20 報告）。
        let io = FakeProbeIo::default();
        let mut machine = make_gji_machine();
        let actions = vec![ProbeAction::Transmit {
            cold_seq: 0,
            plan: TransmitPlan {
                should_prepend_f2: true,
                used_eager_path: false, // gji_resumed=true → false
                needs_literal: false,   // !gji_resumed=false → false
                literal_detect_ms: crate::tuning::RAW_TSF_LITERAL_DETECT_MS_LONG_IDLE,
            },
            observations: ProbeObservations { nc_fired: true, gji_resumed: true },
            romaji: "to".to_string(),
            deferred_vks: vec![],
            target: TransmitTarget::Tsf,
        }];
        let result = dispatch_probe_actions(&mut machine, actions, &io);
        assert!(
            result.is_done(),
            "plan.needs_literal=false → Done を即返す（false positive BS 防止）"
        );
        assert!(io.transmit_tsf_called.get());
    }

    #[test]
    fn gji_not_resumed_long_idle_tsf_mode_keeps_literal_detect() {
        // gji_resumed=false + gji_long_idle + tsf_mode: GJI 応答未確認 → LiteralDetect 有効。
        // VK がリテラル化した場合の回収パスが必要。
        let io = FakeProbeIo::default();
        let mut machine = make_gji_machine();
        let actions = vec![ProbeAction::Transmit {
            cold_seq: 0,
            plan: TransmitPlan {
                should_prepend_f2: true,
                used_eager_path: false, // is_tsf_mode → VK path
                needs_literal: true,    // gji_resumed=false → LiteralDetect 有効
                literal_detect_ms: crate::tuning::RAW_TSF_LITERAL_DETECT_MS_LONG_IDLE,
            },
            observations: ProbeObservations { nc_fired: false, gji_resumed: false },
            romaji: "to".to_string(),
            deferred_vks: vec![],
            target: TransmitTarget::Tsf,
        }];
        let result = dispatch_probe_actions(&mut machine, actions, &io);
        assert!(
            !result.is_done(),
            "plan.needs_literal=true → LiteralDetect フェーズへ移行"
        );
        assert!(io.transmit_tsf_called.get());
        assert_eq!(machine.phase_label(), "LiteralDetect");
    }

    #[test]
    fn raw_tsf_literal_recovery_tsf_mode_keeps_warm_and_increments_consecutive() {
        // TSF mode の RawTsfLiteralRecovery: warm を維持 + consecutive インクリメント。
        // mark_cold_raw_tsf を呼ぶと flush_raw_tsf_literal_romaji が cold 経路で F2 warmup を
        // 再送し WezTerm の 344ms タイマーをリセットしてしまう。
        let io = FakeProbeIo {
            tsf_mode: true,
            consecutive: 0,
            ..Default::default()
        };
        let mut machine = make_gji_machine();
        let actions = vec![
            ProbeAction::RawTsfLiteralRecovery {
                cold_seq: 0,
                backs: 2,
                romaji: "ko".to_string(),
            },
            ProbeAction::Done,
        ];
        let result = dispatch_probe_actions(&mut machine, actions, &io);
        assert!(result.is_done());
        assert!(
            io.set_raw_literal_called.get(),
            "TSF mode: set_raw_literal は呼ぶべき"
        );
        assert!(
            !io.mark_cold_raw_tsf_called.get(),
            "TSF mode: mark_cold_raw_tsf を呼ばず warm を維持するべき"
        );
        assert!(
            io.increment_consecutive_called.get(),
            "TSF mode: ループ防止のため increment_consecutive_count を呼ぶべき"
        );
    }

    #[test]
    fn raw_tsf_literal_recovery_tsf_mode_consecutive_gives_up_with_cold_mark() {
        // TSF mode でも consecutive > 0 のときは諦めて mark_cold_raw_tsf を呼ぶ。
        // 連続 false positive ループを防ぐ。
        let io = FakeProbeIo {
            tsf_mode: true,
            consecutive: 1, // already attempted once
            ..Default::default()
        };
        let mut machine = make_gji_machine();
        let actions = vec![
            ProbeAction::RawTsfLiteralRecovery {
                cold_seq: 0,
                backs: 2,
                romaji: "ko".to_string(),
            },
            ProbeAction::Done,
        ];
        let result = dispatch_probe_actions(&mut machine, actions, &io);
        assert!(result.is_done());
        assert!(
            !io.set_raw_literal_called.get(),
            "consecutive > 0: set_raw_literal を呼ばないべき"
        );
        assert!(
            io.mark_cold_raw_tsf_called.get(),
            "consecutive > 0: mark_cold_raw_tsf で cold に戻すべき"
        );
    }
}
