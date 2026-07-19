//! ProbeIo トレイト — Win32 副作用を抽象化し `dispatch_probe_actions` をテスト可能にする。
//!
//! `Output` が本番実装。`#[cfg(test)]` ブロック内の `FakeProbeIo` がテスト実装。
//! `dispatch_probe_actions` は `ProbeIo` を受け取り、Win32 呼び出しを直接行わない。

use crate::output::{KeyInjector, Output, VkMarker, VkSequence, WarmupOutcome};
use crate::tsf::output::ColdReason;
use crate::tsf::warmup::probe_fsm::DeferredVk;
use crate::tsf::TsfGateState;
use awase::types::VkCode;
use win32_async;

/// `dispatch_probe_actions` が要求する Win32 / 状態ミューテーション操作の抽象。
///
/// - `Output` が本番実装（Win32 SendInput・グローバル原子値の操作）
/// - `FakeProbeIo` がテスト実装（状態変化をフラグで記録し、返値を制御）
pub(crate) trait ProbeIo {
    /// TSF ゲートが `Bypass` 状態かどうかを返す。
    fn gate_is_bypass(&self) -> bool;
    /// TSF 送信パイプラインを実行し、backspace 相当数を返す。
    fn transmit_tsf(
        &self,
        romaji: &str,
        chars: &[(VkCode, bool)],
        outcome: &WarmupOutcome,
    ) -> usize;
    /// Chrome バッチ送信を実行する。
    fn transmit_chrome(&self, romaji: &str, chars: &[(VkCode, bool)]);
    /// IME セッション最初の1文字の per-VK confirm ループ専用（BUG-24 追補）: 1 VK の
    /// DOWN+UP を単独の SendInput で送信する。`transmit_tsf` と異なり F2 prepend /
    /// unicode kana 分岐を一切行わない。
    fn send_single_tsf_vk(&self, vk: VkCode, needs_shift: bool);
    /// Chrome per-VK confirm 専用: 1 VK の
    /// DOWN+UP を `VkMarker::Injected` で単独送信する。`send_single_tsf_vk` の Chrome版。
    fn send_single_chrome_vk(&self, vk: VkCode, needs_shift: bool);
    /// deferred VKs を送信する。
    fn send_deferred_vks(&self, vks: &[DeferredVk], marker: VkMarker);
    /// `TsfWarmupCoordinator` の deferred キューを取り出してクリアする。
    ///
    /// probe machine が何回 tick されたか・途中で置き換わったかに関係なく、
    /// 実際に romaji を送信する直前でこれを呼んで得た値を `send_deferred_vks` に渡すこと。
    fn take_pending_deferred_vks(&self) -> Vec<DeferredVk>;
    /// 連続 raw TSF literal 回数を返す。
    fn consecutive_count(&self) -> u32;
    /// 連続カウントをリセットする（`DetectionResult::CompositionConfirmed` 確認時、BUG-27 追補4）。
    fn reset_consecutive_count(&self);
    /// `RAW_TSF_LITERAL` グローバルを設定する（`consecutive == 0` のときのみ呼ばれる）。
    ///
    /// `escape_composition`: partial literal（candidate 表示中に一部だけ literal 化）回収時に
    /// `true`。`flush_raw_tsf_literal_backspaces` がバックスペース前に `VK_ESCAPE` を送る。
    fn set_raw_literal(&self, backs: usize, romaji: String, escape_composition: bool);
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
    /// Unicode injection mode の long-cold GJI 再初期化: VK_IME_OFF→VK_IME_ON を
    /// SendInput でキューイングし、`ImeModeFsm` の belief 更新 + async
    /// `IMC_GETCONVERSIONMODE` ポーリングを開始する。
    ///
    /// `Output::send_f22_f21_reinit`（`gji_fsm::GjiAction::StartProbe` の
    /// Unicode-mode long-cold ハンドリング）が直接呼ぶ。
    fn send_chrome_gji_reinit_and_poll(&self, cold_seq: u32);
    /// Unicode char を直接送信する（defer モードを無視して即送信）。
    ///
    /// `FlushDeferredUnicodeChars` ハンドラが deferred chars を送信するために使う。
    /// `Output::send_unicode_char()` とは異なり defer フラグをチェックしない。
    fn send_unicode_char_direct(&self, ch: char);
}

impl ProbeIo for Output {
    fn gate_is_bypass(&self) -> bool {
        self.tsf_gate.state() == TsfGateState::Bypass
    }

    fn transmit_tsf(
        &self,
        romaji: &str,
        chars: &[(VkCode, bool)],
        outcome: &WarmupOutcome,
    ) -> usize {
        // カタカナ/英数 charset への追従送信（VK_DBE_KATAKANA 等の leading warmup）は
        // BUG-19 のロックイン事故を受けて撤去した（`docs/known-bugs.md` BUG-19 参照）。
        let result = crate::output::TsfSendPipeline::transmit(romaji, chars, outcome);
        // unicode パスを使った場合（used_eager_path=true かつ kana が存在する）は
        // PendingGjiConfirm 状態に入る: GJI が I/O 応答するまで次の warm キーも unicode で送る。
        if outcome.used_eager_path && crate::tsf::output::kana_for_romaji_static(romaji).is_some() {
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

    fn send_single_tsf_vk(&self, vk: VkCode, needs_shift: bool) {
        KeyInjector::send_vk_pair(vk, needs_shift, VkMarker::Tsf);
    }

    fn send_single_chrome_vk(&self, vk: VkCode, needs_shift: bool) {
        KeyInjector::send_vk_pair(vk, needs_shift, VkMarker::Injected);
    }

    fn send_deferred_vks(&self, vks: &[DeferredVk], marker: VkMarker) {
        let pairs: Vec<(VkCode, bool)> = vks.iter().map(|d| (d.vk, d.needs_shift)).collect();
        Self::send_deferred_probe_vks_from(&pairs, marker);
    }

    fn take_pending_deferred_vks(&self) -> Vec<DeferredVk> {
        self.warmup_coord.take_pending_deferred()
    }

    fn consecutive_count(&self) -> u32 {
        self.composition.consecutive_count()
    }

    fn reset_consecutive_count(&self) {
        self.composition.reset_consecutive_count();
    }

    fn set_raw_literal(&self, backs: usize, romaji: String, escape_composition: bool) {
        self.record_raw_tsf_literal(backs, romaji, escape_composition);
    }

    fn mark_cold_raw_tsf(&self) {
        self.mark_composition_cold(ColdReason::RawTsfLiteralRecovery);
        self.warmup_coord.mark_composition_reset();
    }

    fn store_gji_warmup_result(&self, result: crate::tsf::gji_fsm::WarmupResult) {
        self.warmup_coord.store_warmup_result(result);
    }

    fn current_gji_probe_id(&self) -> Option<crate::tsf::gji_fsm::ProbeId> {
        self.warmup_coord.current_probe_id()
    }

    fn send_chrome_gji_reinit_and_poll(&self, cold_seq: u32) {
        use crate::tsf::output::{make_key_input_ex, IME_KANJI_MARKER};
        use crate::vk::{VK_IME_OFF, VK_IME_ON};
        // 1. VK_IME_OFF → VK_IME_ON を SendInput でキューイングし GJI を OFF/ON リセット。
        let inputs = [
            make_key_input_ex(VK_IME_OFF, false, IME_KANJI_MARKER),
            make_key_input_ex(VK_IME_OFF, true, IME_KANJI_MARKER),
            make_key_input_ex(VK_IME_ON, false, IME_KANJI_MARKER),
            make_key_input_ex(VK_IME_ON, true, IME_KANJI_MARKER),
        ];
        // write_bytes ベースラインを SendInput 前に取得する。
        // VK_IME_OFF→ON が GJI の WriteTransferCount を上昇させるかを観測する実験ログ。
        let write_bytes_before = crate::tsf::observer::gji_write_bytes();
        log::debug!(
            "[chrome-reinit] cold={cold_seq} VK_IME_OFF→VK_IME_ON 強制リセット送信 + IMC ポーリング開始 \
             (write_bytes_baseline={write_bytes_before})"
        );
        let _ = crate::win32::send_input_safe(&inputs);

        // 2. ImeModeFsm belief を即時更新: VK_IME_OFF → Off, VK_IME_ON → Hiragana。
        self.on_f22_f21_sent();

        // 3. async IMC ポーリング開始（CHROME_GJI_REINIT_CONFIRM_MS の間、10ms ごとに発行）。
        //    with_app 再入を避けるため spawn_local で defer する。
        let max_retries = crate::tuning::CHROME_GJI_REINIT_CONFIRM_MS
            / crate::tuning::CHROME_GJI_REINIT_POLL_INTERVAL_MS;
        win32_async::spawn_local(async move {
            let mut first_write_tick: Option<u32> = None;
            for i in 0..max_retries {
                win32_async::sleep_ms(crate::tuning::CHROME_GJI_REINIT_POLL_INTERVAL_MS as u32)
                    .await;
                let write_bytes_now = crate::tsf::observer::gji_write_bytes();
                let write_delta = write_bytes_now.saturating_sub(write_bytes_before);
                if write_delta > 0 && first_write_tick.is_none() {
                    first_write_tick = Some(i as u32 + 1);
                    log::info!(
                        "[chrome-reinit] cold={cold_seq} GJI write_bytes 上昇検出: \
                         tick=#{i} delta=+{write_delta}B (+{:.1}KB)",
                        write_delta as f64 / 1024.0,
                    );
                }
                let conv = crate::ime::get_ime_conversion_mode_raw_timeout_async(15).await;
                log::debug!(
                    "[chrome-reinit] cold={cold_seq} IMC poll #{i}: conv={} NATIVE={} \
                     write_delta=+{write_delta}B",
                    fmt_conv(conv),
                    conv.is_some_and(|v| crate::imm::cmode_has(v, crate::imm::IME_CMODE_NATIVE)),
                );
                let confirmed = crate::with_app(|runtime| {
                    runtime.platform.output.update_ime_mode_from_imc(conv);
                    // Hiragana 確認済みならポーリング終了
                    let fsm = runtime.platform.output.ime_mode_fsm.borrow();
                    fsm.state().is_hiragana() && fsm.is_confirmed()
                });
                if confirmed.unwrap_or(false) {
                    log::debug!("[chrome-reinit] cold={cold_seq} Hiragana 確認 → ポーリング終了");
                    break;
                }
            }
            log::info!(
                "[chrome-reinit] cold={cold_seq} ポーリング完了: \
                 total_write_delta=+{}B first_write_tick={:?}",
                crate::tsf::observer::gji_write_bytes().saturating_sub(write_bytes_before),
                first_write_tick,
            );
        });
    }

    fn send_unicode_char_direct(&self, ch: char) {
        // FSM tick 時は unicode_cold_defer=false のため、通常の send_unicode_char で直接送信できる。
        self.send_unicode_char(ch);
    }
}

/// `Option<u32>` の IMC conversion mode 値をログ用文字列にフォーマットする。
fn fmt_conv(conv: Option<u32>) -> String {
    conv.map_or_else(|| "none".to_owned(), |v| format!("0x{v:08X}"))
}

/// [`Output::start_ms_ime_ready_poll`] の `with_app` クロージャ戻り値。
#[derive(Clone, Copy, PartialEq, Eq)]
enum MsImePollStatus {
    /// NATIVE 確認済み → ポーリング終了。
    Ready,
    /// 未確認 → 継続。
    Pending,
    /// フォーカス世代不一致 / with_app 失敗 → 黙って終了。
    Stale,
}

impl Output {
    /// MS-IME confirm-then-transmit ゲート（BUG-13）の IMC 確認ポーリングを開始する。
    ///
    /// `MS_IME_READY_POLL_INTERVAL_MS` 間隔で `IMC_GETCONVERSIONMODE` を読み、
    /// `ImeModeFsm` に反映する。NATIVE 確認（Hiragana/Katakana confirmed）で終了。
    /// `deadline_ms` までに一度も確認できなければ `ms_ime_gate_give_up` を立てて
    /// 以後のゲート発動をフォーカス変更 / 次の `SetOpen(true)` まで抑止する。
    ///
    /// `ime_mode_focus_gen` の世代照合により、ポーリング中にフォーカスが変わった場合は
    /// stale 結果で `ImeModeFsm` / latch を汚染せず黙って終了する。
    /// 待機側は `MsImeReadyCoro`（`pending_tsf`）が env 経由で確認を観測する。
    pub(crate) fn start_ms_ime_ready_poll(&self, cold_seq: u32, deadline_ms: u64) {
        let gen = self.ime_mode_focus_gen.get();
        win32_async::spawn_local(async move {
            loop {
                let conv = crate::ime::get_ime_conversion_mode_raw_timeout_async(10).await;
                let status = crate::with_app(|runtime| {
                    let out = &runtime.platform.output;
                    if out.ime_mode_focus_gen.get() != gen {
                        return MsImePollStatus::Stale;
                    }
                    out.update_ime_mode_from_imc(conv);
                    if out.ime_mode_fsm.borrow().is_native_ready() {
                        MsImePollStatus::Ready
                    } else {
                        MsImePollStatus::Pending
                    }
                })
                .unwrap_or(MsImePollStatus::Stale);

                match status {
                    MsImePollStatus::Ready => {
                        log::debug!(
                            "[msime-ready] cold={cold_seq} IMC ポーリング: NATIVE 確認 → 終了"
                        );
                        return;
                    }
                    MsImePollStatus::Stale => return,
                    MsImePollStatus::Pending => {}
                }

                if crate::hook::current_tick_ms() >= deadline_ms {
                    let _ = crate::with_app(|runtime| {
                        let out = &runtime.platform.output;
                        if out.ime_mode_focus_gen.get() == gen {
                            out.ms_ime_gate_give_up.set(true);
                            log::warn!(
                                "[msime-ready] cold={cold_seq} IMC 未確認のまま期限切れ → \
                                 give-up latch 設定（フォーカス変更 / 次の IME ON まで gate 停止）"
                            );
                        }
                    });
                    return;
                }
                win32_async::sleep_ms(crate::tuning::MS_IME_READY_POLL_INTERVAL_MS as u32).await;
            }
        });
    }
}

/// GJI probe が飛行中なら `WarmupResult` を記録する。
///
/// `ProbeAction::Transmit`/`TransmitSingleVk` の TSF/Chrome 両アームで同一の
/// 10行ブロックが繰り返されるため、共通関数として抽出する。
fn store_gji_warmup_if_probing(
    io: &impl ProbeIo,
    obs: crate::tsf::warmup::probe_fsm::ProbeObservations,
    plan: &crate::tsf::warmup::probe_fsm::TransmitPlan,
) {
    if io.current_gji_probe_id().is_some() {
        use crate::tsf::gji_fsm::WarmupResult;
        io.store_gji_warmup_result(WarmupResult {
            path: classify_warmup_path(obs, plan),
            prepend_f2_warmup: plan.should_prepend_f2,
            nc_fired: obs.nc_fired,
        });
    }
}

/// probe dispatcher の汎用実装。
/// `ProbeObservations` と `TransmitPlan` から `WarmupPath` を分類する純粋関数。
/// Tsf/Chrome の両 Transmit アームで共用する。
fn classify_warmup_path(
    obs: crate::tsf::warmup::probe_fsm::ProbeObservations,
    plan: &crate::tsf::warmup::probe_fsm::TransmitPlan,
) -> crate::tsf::gji_fsm::WarmupPath {
    use crate::tsf::gji_fsm::WarmupPath;
    if obs.nc_fired {
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
    /// Unicode 送信後に GJI write が観測されなかった → フォーカス中クラスを Tsf に昇格する。
    ///
    /// `advance_tsf_probe` が `focus.learn_injection_mode_tsf()` を呼ぶ。
    LearnedTsf,
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
#[expect(clippy::cognitive_complexity)]
pub(crate) fn dispatch_probe_actions<M, I>(
    machine: &mut M,
    initial_actions: Vec<crate::tsf::warmup::probe_fsm::ProbeAction>,
    io: &I,
) -> DispatchResult
where
    M: crate::tsf::warmup::tickable_fsm::TickableFsm + ?Sized,
    I: ProbeIo,
{
    use crate::tsf::warmup::probe_fsm::{ProbeAction, TransmitTarget};
    use std::collections::VecDeque;

    let mut queue: VecDeque<ProbeAction> = initial_actions.into();

    while let Some(action) = queue.pop_front() {
        match action {
            ProbeAction::Done => return DispatchResult::Done,

            ProbeAction::Transmit {
                cold_seq,
                plan,
                observations,
                romaji,
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
                            let gji_idle = crate::tsf::observer::gji_idle_ms();
                            let romaji_owned: String = romaji.clone();
                            let chars_len = chars.len();
                            win32_async::spawn_local(async move {
                                let conv =
                                    crate::ime::get_ime_conversion_mode_raw_timeout_async(10).await;
                                log::debug!(
                                    "[h1-send] cold={cold_seq} romaji={romaji_owned:?} chars={chars_len} \
                                     gji_idle={gji_idle}ms conv={} ROMAN={} NATIVE={}",
                                    fmt_conv(conv),
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
                        io.send_deferred_vks(&io.take_pending_deferred_vks(), VkMarker::Tsf);
                        // GjiFsm bridge: 送信完了時の warmup 結果を一時バッファに保存する。
                        // step_probe が probe 完了を確認した後に取り出して WarmupComplete に変換する。
                        store_gji_warmup_if_probing(io, observations, &plan);
                        if machine.apply_transmit_done(
                            romaji,
                            ze_bs_count,
                            detector,
                            plan.literal_detect_ms,
                            expected_kana,
                        ) {
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
                        io.send_deferred_vks(&io.take_pending_deferred_vks(), VkMarker::Injected);
                        // GjiFsm bridge: Chrome 経由でも同様に warmup 結果を保存する。
                        store_gji_warmup_if_probing(io, observations, &plan);
                        if machine.apply_transmit_done(
                            romaji,
                            ze_bs_count,
                            detector,
                            plan.literal_detect_ms,
                            None,
                        ) {
                            return DispatchResult::Done;
                        }
                    }
                }
            }

            ProbeAction::TransmitSingleVk {
                cold_seq,
                vk,
                needs_shift,
                timeout_ms,
                is_last,
                observations,
                plan,
                target,
            } => {
                // `gate_is_bypass()` は TSF composition context の readiness ゲートで
                // Chrome には適用されない（Chrome は常に gate=Bypass 運用）。
                // Tsf 向けのときだけ確認する。
                if target == TransmitTarget::Tsf && io.gate_is_bypass() {
                    log::debug!(
                        "[do-transmit] cold={cold_seq} gate=Bypass, skipping per-VK TSF injection"
                    );
                    return DispatchResult::Done;
                }
                // ベースラインは SendInput **前**に取得する（送信中の SHOW/I-O 変化を見逃さないため）。
                // Chrome は write-bytes 閾値ベース（HIMC 不使用、`new_gji_resumed_with_pre_send_baseline`）、
                // TSF は候補ウィンドウ SHOW ベース（`new()`）を使う。
                let detector = match target {
                    TransmitTarget::Tsf => crate::tsf::probe::LiteralDetector::new(),
                    TransmitTarget::Chrome => {
                        crate::tsf::probe::LiteralDetector::new_gji_resumed_with_pre_send_baseline(
                            crate::tsf::observer::gji_write_bytes(),
                        )
                    }
                };
                let marker = match target {
                    TransmitTarget::Tsf => {
                        io.send_single_tsf_vk(vk, needs_shift);
                        VkMarker::Tsf
                    }
                    TransmitTarget::Chrome => {
                        io.send_single_chrome_vk(vk, needs_shift);
                        VkMarker::Injected
                    }
                };
                let deadline_ms = crate::hook::current_tick_ms() + timeout_ms;
                if is_last {
                    io.send_deferred_vks(&io.take_pending_deferred_vks(), marker);
                    // GjiFsm bridge: romaji 全体の送信完了に相当するタイミングで warmup 結果を保存する。
                    store_gji_warmup_if_probing(io, observations, &plan);
                }
                machine.apply_vk_sent(detector, deadline_ms);
            }

            ProbeAction::UpgradeToTsf => {
                // UnicodeLiteralObserverFsm が GJI write なしと判断した。
                // Done は後続 action として queue に入っているので、ここでは LearnedTsf を返す。
                return DispatchResult::LearnedTsf;
            }

            ProbeAction::FlushDeferredUnicodeChars(chars) => {
                // UnicodeColdWarmupFsm が GJI wake-up 確認後に emit する。
                // deferred chars を直接送信する（Done が続いて FSM 完了）。
                log::debug!(
                    "[unicode-cold-warmup] FlushDeferredUnicodeChars: {} chars 送信",
                    chars.len()
                );
                for ch in &chars {
                    io.send_unicode_char_direct(*ch);
                }
            }

            ProbeAction::RawTsfLiteralRecovery {
                cold_seq,
                backs,
                romaji,
                escape_composition,
            } => {
                // emit_recovery_actions は常にこのアクションを emit する（捨て駒キー
                // には倒れない、2026-07-16 撤去）。consecutive==0 なら backspace + romaji
                // 再送を scheduled し、次の cold パス（per-VK confirm）へ自然に委ねる。
                let consecutive = io.consecutive_count();
                if consecutive == 0 {
                    log::warn!(
                        "[raw-tsf-literal] cold={cold_seq} raw TSF literal suspected \
                        → backspace ×{backs} + re-send {romaji:?} scheduled \
                        + mark cold"
                    );
                    io.set_raw_literal(backs, romaji, escape_composition);
                } else {
                    log::warn!(
                        "[raw-tsf-literal] cold={cold_seq} consecutive raw-tsf-literal \
                        (count={}) → giving up, backs={backs} cleanup only (no re-send)",
                        consecutive + 1,
                    );
                    // 諦めても partial literal 由来の 'k'(literal) + composition が
                    // terminal に残ると "kおの" 等の文字化けになる。
                    // romaji 再送はせず BS のみ送って terminal をクリーンにする。
                    // escape_composition はそのまま引き継ぎ、composition が残っていれば
                    // ESC で確実に破棄する。
                    io.set_raw_literal(backs, String::new(), escape_composition);
                }
                io.mark_cold_raw_tsf();
            }

            ProbeAction::CompositionConfirmed {
                mark_literal_session,
            } => {
                // BUG-27 追補4: consecutive_count は「連続失敗」の抑止用カウンタ。
                // 本物の CompositionConfirmed が挟まれば連続ではなくなるため、
                // 必ずリセットする（従来は FocusChange/SetOpenTrue でしかリセット
                // されず、セッション中に一度でも literal 化すると以後ずっと
                // give-up=backspace-onlyに固定される regression があった）。
                io.reset_consecutive_count();
                if mark_literal_session {
                    crate::tsf::observer::mark_literal_session_confirmed();
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
    use crate::tsf::warmup::probe_fsm::{
        ProbeAction, ProbeObservations, TransmitPlan, TransmitTarget,
    };
    use std::cell::Cell;

    /// テスト用フェイク ProbeIo。Win32 副作用を no-op にし、呼び出しをフラグで記録する。
    struct FakeProbeIo {
        bypass: bool,
        tsf_transmit_result: usize,
        consecutive: u32,
        transmit_tsf_called: Cell<bool>,
        transmit_chrome_called: Cell<bool>,
        send_single_tsf_vk_call_count: Cell<u32>,
        send_single_chrome_vk_call_count: Cell<u32>,
        deferred_vks_called: Cell<bool>,
        set_raw_literal_called: Cell<bool>,
        mark_cold_raw_tsf_called: Cell<bool>,
        reset_consecutive_called: Cell<bool>,
        /// transmit_tsf に渡された WarmupOutcome.used_eager_path を記録する。
        last_used_eager_path: Cell<bool>,
        /// transmit_tsf に渡された WarmupOutcome.prepend_f2_warmup を記録する。
        last_used_prepend_f2: Cell<bool>,
    }

    impl Default for FakeProbeIo {
        fn default() -> Self {
            Self {
                bypass: false,
                tsf_transmit_result: 1,
                consecutive: 0,
                transmit_tsf_called: Cell::new(false),
                transmit_chrome_called: Cell::new(false),
                send_single_tsf_vk_call_count: Cell::new(0),
                send_single_chrome_vk_call_count: Cell::new(0),
                deferred_vks_called: Cell::new(false),
                set_raw_literal_called: Cell::new(false),
                mark_cold_raw_tsf_called: Cell::new(false),
                reset_consecutive_called: Cell::new(false),
                last_used_eager_path: Cell::new(false),
                last_used_prepend_f2: Cell::new(false),
            }
        }
    }

    impl ProbeIo for FakeProbeIo {
        fn gate_is_bypass(&self) -> bool {
            self.bypass
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
        fn send_single_tsf_vk(&self, _vk: VkCode, _needs_shift: bool) {
            self.send_single_tsf_vk_call_count
                .set(self.send_single_tsf_vk_call_count.get() + 1);
        }
        fn send_single_chrome_vk(&self, _vk: VkCode, _needs_shift: bool) {
            self.send_single_chrome_vk_call_count
                .set(self.send_single_chrome_vk_call_count.get() + 1);
        }
        fn send_deferred_vks(&self, _vks: &[DeferredVk], _marker: VkMarker) {
            self.deferred_vks_called.set(true);
        }
        fn take_pending_deferred_vks(&self) -> Vec<DeferredVk> {
            vec![]
        }
        fn consecutive_count(&self) -> u32 {
            self.consecutive
        }
        fn reset_consecutive_count(&self) {
            self.reset_consecutive_called.set(true);
        }
        fn set_raw_literal(&self, _backs: usize, _romaji: String, _escape_composition: bool) {
            self.set_raw_literal_called.set(true);
        }
        fn mark_cold_raw_tsf(&self) {
            self.mark_cold_raw_tsf_called.set(true);
        }

        fn store_gji_warmup_result(&self, _result: crate::tsf::gji_fsm::WarmupResult) {}

        fn current_gji_probe_id(&self) -> Option<crate::tsf::gji_fsm::ProbeId> {
            None
        }

        fn send_chrome_gji_reinit_and_poll(&self, _cold_seq: u32) {}

        fn send_unicode_char_direct(&self, _ch: char) {}
    }

    fn make_chrome_machine() -> crate::tsf::warmup::probe_fsm::TsfProbeCoro {
        let guard = OutputActiveGuard::noop_for_test();
        let probe = crate::tsf::probe::TsfReadinessProbe::new(0, 0, 0);
        crate::tsf::warmup::probe_fsm::TsfProbeCoro::new_chrome("ka", 0, probe, 0, guard)
    }

    fn make_gji_machine() -> crate::tsf::warmup::gji_warmup_coro::GjiWarmupCoro {
        let probe = crate::tsf::probe::TsfReadinessProbe::new(0, 0, 0);
        crate::tsf::warmup::gji_warmup_coro::GjiWarmupCoro::new(
            "ka",
            0,
            probe,
            0,
            ColdReason::FocusChange,
            false,
            false,
            false,
            false,
            false,
            0,
        )
    }

    #[test]
    fn done_action_returns_true_without_side_effects() {
        let io = FakeProbeIo::default();
        let mut machine = make_chrome_machine();
        let result = dispatch_probe_actions(&mut machine, vec![ProbeAction::Done], &io);
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
            observations: ProbeObservations {
                nc_fired: true,
            },
            romaji: "ka".to_string(),
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
            observations: ProbeObservations {
                nc_fired: true,
            },
            romaji: "ka".to_string(),
            target: TransmitTarget::Chrome,
        }];
        let result = dispatch_probe_actions(&mut machine, actions, &io);
        assert!(
            !result.is_done(),
            "should not be Done — LiteralDetect phase pending"
        );
        assert!(io.transmit_chrome_called.get());
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
            observations: ProbeObservations {
                nc_fired: true,
            },
            romaji: "ka".to_string(),
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
            observations: ProbeObservations {
                nc_fired: true,
            },
            romaji: "ka".to_string(),
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
            observations: ProbeObservations {
                nc_fired: true,
            },
            romaji: "ka".to_string(),
            target: TransmitTarget::Tsf,
        }];
        let result = dispatch_probe_actions(&mut machine, actions, &io);
        assert!(result.is_done());
        assert!(io.transmit_tsf_called.get());
        assert!(!io.transmit_chrome_called.get());
    }

    #[test]
    fn tsf_transmit_uses_eager_path_when_nc_not_fired() {
        // nc_fired=false のとき、decide_transmit_plan が確定した used_eager_path=true が
        // WarmupOutcome.used_eager_path=true として transmit_tsf に渡ることを確認する。
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
            observations: ProbeObservations {
                nc_fired: false,
            },
            romaji: "ki".to_string(),
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
                escape_composition: false,
            },
            ProbeAction::Done,
        ];
        let result = dispatch_probe_actions(&mut machine, actions, &io);
        assert!(result.is_done());
        assert!(io.set_raw_literal_called.get());
        assert!(io.mark_cold_raw_tsf_called.get());
    }

    // NOTE: `raw_tsf_literal_recovery_skips_set_literal_when_consecutive`（consecutive>0 で
    // set_raw_literal を呼ばない、という旧設計を検証していたテスト）は 2026-07-10 に削除した。
    // 2026-05-25 (9aa7e29) 時点の「諦めたら set_raw_literal を呼ばずスキップする」設計を
    // テストしていたが、2026-06-18 (84e6942, BUG-13 修正) で「諦めても set_raw_literal は
    // 呼び、romaji を空にして BS のみ送る（terminal に 'k'(literal)+composition が残ると
    // 文字化けするため）」という設計に意図的に変更された。この変更時に古いテストが
    // 削除されず、単一の生産コード経路に対して直下の
    // `raw_tsf_literal_recovery_tsf_mode_consecutive_gives_up_with_cold_mark`（set_raw_literal を
    // 呼ぶことを期待）と正反対の期待値を持つ矛盾したテストペアが残っていた。
    // 現在の意図（84e6942）と一致する後者のみを残す。

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
            observations: ProbeObservations {
                nc_fired: false,
            },
            romaji: "ka".to_string(),
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
            observations: ProbeObservations {
                nc_fired: false,
            },
            romaji: "i".to_string(),
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
            observations: ProbeObservations {
                nc_fired: false,
            },
            romaji: "i".to_string(),
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
                used_eager_path: false, // is_tsf_mode → VK path
                needs_literal: true, // should_prepend_f2 && gji_active && (!gji_long_idle || is_tsf_mode)
                literal_detect_ms: crate::tuning::RAW_TSF_LITERAL_DETECT_MS_LONG_IDLE,
            },
            observations: ProbeObservations {
                nc_fired: false,
            },
            romaji: "ko".to_string(),
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
            observations: ProbeObservations {
                nc_fired: false,
            },
            romaji: "ko".to_string(),
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

    // 旧 gji_resumed_skips_literal_detect_to_prevent_false_positive テストは
    // ProbeObservations.gji_resumed 撤去（BUG-24 追補9）に伴い削除した。この signal は
    // 本番では常に false だったため（唯一の producer である gji_warmup_coro.rs の
    // 'initial ループが2分岐とも gji_resumed=false を返していた）、実際には一度も
    // 発火していなかった。当時の実機報告（WezTerm long_idle(120s) 後の 'と' 部分
    // リテラル誤判定、2026-06-20）は per-VK confirm（BUG-24 本体）が別の仕組みで
    // 解決済み。詳細は docs/known-bugs.md BUG-24 追補9参照。

    #[test]
    fn long_idle_tsf_mode_keeps_literal_detect() {
        // gji_long_idle + tsf_mode: GJI 応答未確認 → LiteralDetect 有効。
        // VK がリテラル化した場合の回収パスが必要。
        let io = FakeProbeIo::default();
        let mut machine = make_gji_machine();
        let actions = vec![ProbeAction::Transmit {
            cold_seq: 0,
            plan: TransmitPlan {
                should_prepend_f2: true,
                used_eager_path: false, // is_tsf_mode → VK path
                needs_literal: true,    // LiteralDetect 有効
                literal_detect_ms: crate::tuning::RAW_TSF_LITERAL_DETECT_MS_LONG_IDLE,
            },
            observations: ProbeObservations { nc_fired: false },
            romaji: "to".to_string(),
            target: TransmitTarget::Tsf,
        }];
        let result = dispatch_probe_actions(&mut machine, actions, &io);
        assert!(
            !result.is_done(),
            "plan.needs_literal=true → LiteralDetect フェーズへ移行"
        );
        assert!(io.transmit_tsf_called.get());
    }

    #[test]
    fn raw_tsf_literal_recovery_tsf_mode_consecutive_gives_up_with_cold_mark() {
        // TSF mode でも consecutive > 0 のときは諦める。
        // ただし terminal に 'k'(literal) + composition が残らないよう BS のみ送る (romaji 再送なし)。
        let io = FakeProbeIo {
            consecutive: 1, // already attempted once
            ..Default::default()
        };
        let mut machine = make_gji_machine();
        let actions = vec![
            ProbeAction::RawTsfLiteralRecovery {
                cold_seq: 0,
                backs: 2,
                romaji: "ko".to_string(),
                escape_composition: false,
            },
            ProbeAction::Done,
        ];
        let result = dispatch_probe_actions(&mut machine, actions, &io);
        assert!(result.is_done());
        assert!(
            io.set_raw_literal_called.get(),
            "consecutive > 0: BS cleanup のため set_raw_literal を呼ぶべき (romaji は空で再送なし)"
        );
        assert!(
            io.mark_cold_raw_tsf_called.get(),
            "consecutive > 0: mark_cold_raw_tsf で cold に戻すべき"
        );
    }
}
