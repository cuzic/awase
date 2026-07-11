//! ProbeIo トレイト — Win32 副作用を抽象化し `dispatch_probe_actions` をテスト可能にする。
//!
//! `Output` が本番実装。`#[cfg(test)]` ブロック内の `FakeProbeIo` がテスト実装。
//! `dispatch_probe_actions` は `ProbeIo` を受け取り、Win32 呼び出しを直接行わない。

use crate::output::{KeyInjector, Output, VkMarker, VkSequence, WarmupOutcome};
use crate::state::key_sequence_policy::{self, SacrificialWarmupKey};
use crate::tsf::observer::NamechangeBaseline;
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
    /// deferred VKs を送信する。
    fn send_deferred_vks(&self, vks: &[DeferredVk], marker: VkMarker);
    /// `TsfWarmupCoordinator` の deferred キューを取り出してクリアする。
    ///
    /// probe machine が何回 tick されたか・途中で置き換わったかに関係なく、
    /// 実際に romaji を送信する直前でこれを呼んで得た値を `send_deferred_vks` に渡すこと。
    fn take_pending_deferred_vks(&self) -> Vec<DeferredVk>;
    /// fresh F2 (`VK_DBE_HIRAGANA`) を送信し、`(namechange_baseline, sent_ms)` を返す。
    ///
    /// ベースラインは SendInput **前**に取得すること（送信中の NAMECHANGE を見逃さないため）。
    fn send_fresh_f2(&self) -> (NamechangeBaseline, u64);
    /// gji_long_idle 時に追加 F2 を送信して F2×2 連続とする。
    ///
    /// F2 単発では GJI I/O が発生しないが、F2×2 連続では GJI が起動して I/O を出す
    /// （cold=1244 実測: 31ms 以内）。`send_fresh_f2` の直後に呼ぶこと。
    fn send_extra_f2(&self);
    /// 連続 raw TSF literal 回数を返す。
    fn consecutive_count(&self) -> u32;
    /// warm 状態を維持したまま連続カウントをインクリメントする（TSF mode 回収パス用）。
    fn increment_consecutive_count(&self);
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
    /// VK_IME_OFF→VK_IME_ON を IME_KANJI_MARKER 付きで送信する（vim 安全な一次プローブ）。
    ///
    /// `StartSacrificialWarmup` ハンドラが呼ぶ。Off→On 遷移が GJI WriteTransferCount を増加させ、
    /// `ImeOffOnWarmupFsm` が write_bytes 上昇を検出してから実ローマ字を再送する。
    fn send_sacrificial_ime_off_on(&self, cold_seq: u32);
    /// 犠牲キー（VK_A + BS）を同一 SendInput バッチで送信する（Chrome 用）。
    ///
    /// VK_A と BS を一括送信することで Chrome が次フレームを描画する前に
    /// 'あ'/'a' が形成→即消去される。ユーザーには文字が表示されない。
    /// `SacrificialResend` Chrome 側では追加の BS 送信を行わない。
    fn send_sacrificial_vk_a_with_bs(&self, cold_seq: u32);
    /// BS×1 を送信する（犠牲キーの削除用）。
    ///
    /// `SacrificialResend` ハンドラが呼ぶ（TSF/WezTerm target のみ）。
    fn send_sacrificial_bs_one(&self, cold_seq: u32);
    /// BS×n を送信する（partial literal 回収前の terminal cleanup 用）。
    ///
    /// `RawTsfLiteralRecovery` → sacr warmup 切り替え時に VK_A 送信前に呼ぶ。
    fn send_literal_recovery_bs(&self, backs: usize, cold_seq: u32);
    /// `VK_ESCAPE` で現在の composition を破棄してから BS×n を送信する。
    ///
    /// partial literal（candidate 表示中に一部だけ literal 化）の回収専用。ESC は
    /// composition の文字数に関わらず 1 打鍵で確実に全消去できるため、
    /// 「composition が何文字だったか」を推測する必要がなくなる（backs は残る
    /// literal プレフィックス分のみでよい）。
    fn send_literal_recovery_esc_bs(&self, backs: usize, cold_seq: u32);
    /// Chrome sacr-warmup cold タイムアウト後に GJI を強制リセットし、IMC ポーリングを開始する。
    ///
    /// VK_A+BS でも Chrome の GJI が初期化されなかった場合（80s 以上の超長時間 idle 等）に、
    /// VK_IME_OFF→VK_IME_ON を SendInput でキューイングして再初期化を試みる。
    ///
    /// さらに `ImeModeFsm` の belief を Off → Hiragana に更新し、
    /// async `IMC_GETCONVERSIONMODE` ポーリングを `spawn_local` で開始する。
    /// ポーリング結果は `with_app(|runtime| runtime.platform.output.update_ime_mode_from_imc(conv))` で反映される。
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
        // BUG-19 のロックイン事故を受けて撤去した。`DIAG_FORCE_HIRAGANA_CHARSET` により
        // `ConvModeMgr::effective_charset()` は既に常に Hiragana を返しており、この
        // 追従パスは実質的に到達不能だった（`docs/known-bugs.md` BUG-19 参照）。
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

    fn send_deferred_vks(&self, vks: &[DeferredVk], marker: VkMarker) {
        let pairs: Vec<(VkCode, bool)> = vks.iter().map(|d| (d.vk, d.needs_shift)).collect();
        Self::send_deferred_probe_vks_from(&pairs, marker);
    }

    fn take_pending_deferred_vks(&self) -> Vec<DeferredVk> {
        self.warmup_coord.take_pending_deferred()
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

    fn consecutive_count(&self) -> u32 {
        self.composition.consecutive_count()
    }

    fn increment_consecutive_count(&self) {
        self.composition.increment_consecutive_count();
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

    fn send_sacrificial_ime_off_on(&self, cold_seq: u32) {
        use crate::tsf::output::{make_key_input_ex, IME_KANJI_MARKER};
        use crate::vk::{VK_IME_OFF, VK_IME_ON};
        // VK_IME_ON 後に GJI が正しい conv を復元できるよう、直前に ImmSetConversionStatus を
        // 再スケジュールする。cold warmup 開始時の imm-romaji が stale な conv_mode を参照して
        // いた場合（例: cold=3 で ZenKata が誤設定された状態）の補正として機能する。
        // GJI は cross-process で VK_IME_ON を非同期処理するため、spawn_local が完了する
        // ~10ms 以内に ImmSetConversionStatus が確定し、GJI が conv を読む前に間に合う。
        //
        // `cold_warmup.rs::preamble()` と同じ理由で、同じ belief (mode) に対する復元書き込みは
        // 1回だけに制限する — サクリファイシャル warmup がフォーカス往復のたびに繰り返し
        // 発生すると、誤った belief を毎回 real IME へ再アサートする自己増幅経路になる
        // (BUG-19, ADR-078 Phase 1a、実機検証待ち)。
        if self.conv_mutation_allowed.get() && self.conv_mode.needs_conv_restore_write() {
            if let Some(conv_target) = self
                .conv_mode
                .get()
                .and_then(awase::engine::ConvMode::imm_conv_target)
            {
                self.conv_mode.mark_conv_restore_written();
                win32_async::spawn_local(async move {
                    let _ =
                        crate::ime::set_ime_romaji_mode_with_target_async(Some(conv_target)).await;
                    log::debug!(
                        "[sacr-warmup] cold={cold_seq} imm-romaji 直前補正: \
                         target=0x{conv_target:08X}"
                    );
                });
            }
        }
        let inputs = [
            make_key_input_ex(VK_IME_OFF, false, IME_KANJI_MARKER),
            make_key_input_ex(VK_IME_OFF, true, IME_KANJI_MARKER),
            make_key_input_ex(VK_IME_ON, false, IME_KANJI_MARKER),
            make_key_input_ex(VK_IME_ON, true, IME_KANJI_MARKER),
        ];
        log::debug!("[sacr-warmup] cold={cold_seq} VK_IME_OFF→ON 送信（vim 安全プローブ）");
        self.on_f22_f21_sent();
        let _ = crate::win32::send_input_safe(&inputs);
    }

    fn send_sacrificial_vk_a_with_bs(&self, cold_seq: u32) {
        use crate::tsf::output::make_key_input_ex;
        use crate::tsf::output::INJECTED_MARKER;
        use crate::vk::VK_BACK;
        use awase::types::VkCode;
        const VK_A: VkCode = VkCode(0x41);
        // VK_A + BS を一括 SendInput することで Chrome が次フレームを描画する前に
        // 'あ'/'a' → BS と処理され、ユーザーには文字フラッシュが見えない。
        let inputs = [
            make_key_input_ex(VK_A, false, INJECTED_MARKER),
            make_key_input_ex(VK_A, true, INJECTED_MARKER),
            make_key_input_ex(VK_BACK, false, INJECTED_MARKER),
            make_key_input_ex(VK_BACK, true, INJECTED_MARKER),
        ];
        log::debug!(
            "[sacr-warmup] cold={cold_seq} VK_A+BS 同時送信（Chrome 用：文字フラッシュ防止）"
        );
        let _ = crate::win32::send_input_safe(&inputs);
    }

    fn send_sacrificial_bs_one(&self, cold_seq: u32) {
        use crate::tsf::output::make_key_input_ex;
        use crate::tsf::output::INJECTED_MARKER;
        use crate::vk::VK_BACK;
        let inputs = [
            make_key_input_ex(VK_BACK, false, INJECTED_MARKER),
            make_key_input_ex(VK_BACK, true, INJECTED_MARKER),
        ];
        log::debug!("[sacr-warmup] cold={cold_seq} BS×1 送信（犠牲キー削除）");
        let _ = crate::win32::send_input_safe(&inputs);
    }

    fn send_literal_recovery_bs(&self, backs: usize, cold_seq: u32) {
        use crate::tsf::output::make_key_input_ex;
        use crate::tsf::output::INJECTED_MARKER;
        use crate::vk::VK_BACK;
        use windows::Win32::UI::Input::KeyboardAndMouse::INPUT;
        if backs == 0 {
            return;
        }
        let inputs: Vec<INPUT> = (0..backs)
            .flat_map(|_| {
                [
                    make_key_input_ex(VK_BACK, false, INJECTED_MARKER),
                    make_key_input_ex(VK_BACK, true, INJECTED_MARKER),
                ]
            })
            .collect();
        log::debug!("[raw-tsf-literal] cold={cold_seq} partial literal cleanup BS×{backs}");
        let _ = crate::win32::send_input_safe(&inputs);
    }

    fn send_literal_recovery_esc_bs(&self, backs: usize, cold_seq: u32) {
        use crate::tsf::output::make_key_input_ex;
        use crate::tsf::output::INJECTED_MARKER;
        use crate::vk::{VK_BACK, VK_ESCAPE};
        use windows::Win32::UI::Input::KeyboardAndMouse::INPUT;
        let mut inputs: Vec<INPUT> = vec![
            make_key_input_ex(VK_ESCAPE, false, INJECTED_MARKER),
            make_key_input_ex(VK_ESCAPE, true, INJECTED_MARKER),
        ];
        inputs.extend((0..backs).flat_map(|_| {
            [
                make_key_input_ex(VK_BACK, false, INJECTED_MARKER),
                make_key_input_ex(VK_BACK, true, INJECTED_MARKER),
            ]
        }));
        log::debug!(
            "[raw-tsf-literal] cold={cold_seq} partial literal cleanup: VK_ESCAPE (composition破棄) + BS×{backs}"
        );
        let _ = crate::win32::send_input_safe(&inputs);
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
    /// 発行機構は `send_chrome_gji_reinit_and_poll` の IMC ポーリングと同型（VK 送信なし）。
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
/// `ProbeAction::Transmit` の TSF/Chrome 両アームと `StartSacrificialWarmup` で
/// 同一の10行ブロックが繰り返されるため、共通関数として抽出する。
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
            gji_resumed: obs.gji_resumed,
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
    SwitchMachine(Box<dyn crate::tsf::warmup::tickable_fsm::TickableFsm>),
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
            } => {
                if io.gate_is_bypass() {
                    log::debug!(
                        "[do-transmit] cold={cold_seq} gate=Bypass, skipping per-VK TSF injection"
                    );
                    return DispatchResult::Done;
                }
                // ベースラインは SendInput **前**に取得する（送信中の SHOW/I-O 変化を見逃さないため）。
                let detector = crate::tsf::probe::LiteralDetector::new();
                io.send_single_tsf_vk(vk, needs_shift);
                let deadline_ms = crate::hook::current_tick_ms() + timeout_ms;
                if is_last {
                    io.send_deferred_vks(&io.take_pending_deferred_vks(), VkMarker::Tsf);
                    // GjiFsm bridge: romaji 全体の送信完了に相当するタイミングで warmup 結果を保存する。
                    store_gji_warmup_if_probing(io, observations, &plan);
                }
                machine.apply_vk_sent(detector, deadline_ms);
            }

            ProbeAction::SendRecoveryBs {
                cold_seq,
                backs,
                escape_composition,
            } => {
                // GjiWarmupCoro（inline LiteralDetect）が TSF mode + consecutive==0 で
                // partial literal / SuspectedLiteral を検出したときに emit する。
                // StartSacrificialWarmup の直前に backs 個の BS で terminal cleanup を行う。
                // escape_composition=true（partial literal）: ESC で composition を確実に
                // 破棄してから残る literal プレフィックスのみ BS する。
                if escape_composition {
                    io.send_literal_recovery_esc_bs(backs, cold_seq);
                } else {
                    io.send_literal_recovery_bs(backs, cold_seq);
                }
            }

            ProbeAction::StartSacrificialWarmup(config) => {
                // GjiWarmupCoro が long_cold + TSF mode のときに emit する（直接）か、
                // inline LiteralDetect が partial literal / SuspectedLiteral を検出して
                // sacr warmup 経由で回収する場合（from_literal_recovery=true）に emit される。
                //
                // VK_A+BS の代わりに VK_IME_OFF→VK_IME_ON を送信する（vim 安全プローブ）。
                // Off→On 状態遷移が GJI WriteTransferCount を増加させ（実測 +46B / ~30ms）、
                // ImeOffOnWarmupFsm が write_bytes 上昇を検出してから実ローマ字を再送する。
                // Chrome は常に gate=Bypass 運用のため gate チェック対象外（policy に集約）。
                // TSF/WezTerm の場合のみ bypass 状態でスキップする。
                if key_sequence_policy::warmup_respects_bypass_gate(config.target)
                    && io.gate_is_bypass()
                {
                    log::debug!(
                        "[sacr-warmup] cold={} StartSacrificialWarmup: gate=Bypass, skipping",
                        config.cold_seq
                    );
                    return DispatchResult::Done;
                }
                if config
                    .romaji
                    .chars()
                    .find_map(crate::output::resolve_ascii_to_vk)
                    .is_none()
                {
                    return DispatchResult::Done;
                }
                // literal recovery パスでは consecutive をインクリメントしてループを防ぐ。
                if config.from_literal_recovery {
                    io.increment_consecutive_count();
                }
                // GjiFsm bridge: 送信時点で warmup 結果を記録する。
                store_gji_warmup_if_probing(io, config.observations, &config.plan);
                // target に応じて戦略を切り替える。
                //
                // Chrome: VK_A+BS（元の方式）。
                //   VK_IME_OFF が Chrome TSF context を壊すため使用不可（過去検証済み）。
                //   Chrome 内で vim が動くケースは稀であり VK_A の vim 問題は許容。
                //
                // TSF（WezTerm 等）: VK_IME_OFF→ON + write_bytes 検出（vim 安全プローブ）。
                //   vim は VK_IME_OFF/ON を無視するため cold 時にアプリへ届いても誤動作しない。
                //
                // DIAG_CHROME_SACRIFICIAL_KEY_IME_OFFON: Chrome にも ImeOffThenOn を試す診断。
                // `key_sequence_policy::sacrificial_warmup_key`（実機確定済みテーブル）自体は
                // 変更せず、ここで戻り値だけ上書きする。tuning.rs 参照。
                let sacrificial_key = if config.target == TransmitTarget::Chrome
                    && crate::tuning::DIAG_CHROME_SACRIFICIAL_KEY_IME_OFFON
                {
                    log::info!(
                        "[sacr-warmup-diag] cold={} Chrome: VkAThenBackspace → ImeOffThenOn に差し替え \
                         (DIAG_CHROME_SACRIFICIAL_KEY_IME_OFFON=true)",
                        config.cold_seq
                    );
                    SacrificialWarmupKey::ImeOffThenOn
                } else {
                    key_sequence_policy::sacrificial_warmup_key(config.target)
                };
                match sacrificial_key {
                    SacrificialWarmupKey::VkAThenBackspace => {
                        let write_bytes_before_vk_a = crate::tsf::observer::gji_write_bytes();
                        io.send_sacrificial_vk_a_with_bs(config.cold_seq);
                        let detector = crate::tsf::probe::LiteralDetector::new_gji_resumed_with_pre_send_baseline(write_bytes_before_vk_a);
                        let deadline_ms = crate::hook::current_tick_ms() + config.literal_detect_ms;
                        log::debug!(
                            "[sacr-warmup] cold={} VK_A+BS 送信 → SacrificialWarmupCoro 開始 \
                            (romaji={:?} write_bytes_baseline={})",
                            config.cold_seq,
                            config.romaji,
                            write_bytes_before_vk_a,
                        );
                        let sacr_coro =
                            crate::tsf::warmup::sacr_warmup_coro::SacrificialWarmupCoro::new(
                                config.cold_seq,
                                config.romaji,
                                detector,
                                deadline_ms,
                                config.target,
                            );
                        return DispatchResult::SwitchMachine(Box::new(sacr_coro));
                    }
                    SacrificialWarmupKey::ImeOffThenOn => {
                        let write_bytes_baseline = crate::tsf::observer::gji_write_bytes();
                        io.send_sacrificial_ime_off_on(config.cold_seq);
                        log::debug!(
                            "[sacr-warmup] cold={} VK_IME_OFF→ON 送信 → ImeOffOnWarmupFsm 開始 \
                            (romaji={:?} write_bytes_baseline={})",
                            config.cold_seq,
                            config.romaji,
                            write_bytes_baseline,
                        );
                        let fsm = crate::tsf::warmup::ime_offon_warmup_fsm::ImeOffOnWarmupFsm::new(
                            config.cold_seq,
                            config.romaji,
                            config.target,
                            write_bytes_baseline,
                        );
                        return DispatchResult::SwitchMachine(Box::new(fsm));
                    }
                }
            }

            ProbeAction::SendChromeGjiReinit { cold_seq } => {
                // SacrificialWarmupCoro が Chrome cold タイムアウト後に emit する。
                // VK_IME_OFF→VK_IME_ON 送信 + ImeModeFsm belief 更新 + async IMC ポーリング開始。
                // FSM 切り替えは不要（SacrificialWarmupCoro がそのまま IME 確認を待機する）。
                io.send_chrome_gji_reinit_and_poll(cold_seq);
            }

            ProbeAction::SacrificialResend(resend) => {
                // SacrificialWarmupCoro から emit される（composition 確認後）。
                // BS×1（犠牲 VK_A 削除）→ 実ローマ字送信 → deferred_vks 送信。
                // target に応じて Chrome/TSF パスを切り替える。
                let cold_seq = resend.cold_seq;
                let chars: VkSequence = resend
                    .romaji
                    .chars()
                    .filter_map(crate::output::resolve_ascii_to_vk)
                    .collect();
                // Chrome は常に gate=Bypass 運用のため gate チェック対象外（policy に集約）。
                if chars.is_empty()
                    || (key_sequence_policy::warmup_respects_bypass_gate(resend.target)
                        && io.gate_is_bypass())
                {
                    // ゲートが閉じている or 実ローマ字なし: BS も送らず即終了
                    log::debug!(
                        "[sacr-warmup] cold={cold_seq} SacrificialResend: skip (bypass or empty)"
                    );
                } else {
                    // BS×1: 犠牲 VK_A の結果を削除。
                    // skip_cleanup_bs=true（ImeOffOnWarmupFsm）は VK_A を送っていないので BS 不要。
                    // Chrome は VK_A+BS を atomic batch で送信済みのため cleanup BS 不要（policy に集約）。
                    if !resend.skip_cleanup_bs
                        && key_sequence_policy::target_needs_sacrificial_cleanup_bs(resend.target)
                    {
                        io.send_sacrificial_bs_one(cold_seq);
                    }
                    match resend.target {
                        TransmitTarget::Chrome => {
                            // Chrome パス: INJECTED_MARKER バッチ送信。
                            // Chrome cold case は SendChromeGjiReinit 後に SacrificialWarmupCoro が
                            // IME 確認し SacrificialResend(confirmed_warm=false) を emit する。
                            // VK_IME_OFF→VK_IME_ON は SendChromeGjiReinit で送信済み。
                            log::debug!(
                                "[sacr-warmup] cold={cold_seq} 実ローマ字 {:?} を Chrome パスで再送 \
                                (confirmed_warm={})",
                                resend.romaji, resend.confirmed_warm,
                            );
                            io.transmit_chrome(&resend.romaji, &chars);
                            io.send_deferred_vks(
                                &io.take_pending_deferred_vks(),
                                VkMarker::Injected,
                            );
                        }
                        TransmitTarget::Tsf => {
                            // confirmed_warm=true: warm 維持のまま VK run 送信（F2 prepend なし）。
                            //   WezTerm の 344ms composition context タイマーをリセットしない。
                            // confirmed_warm=false: VK_A timeout → TSF cold 確定 → F2 で warmup。
                            //   partial literal 回収後の sacr warmup や long-cold timeout で使われる。
                            //   timeout 期間（≥300ms）中に WezTerm の 344ms timer が既に発火している
                            //   可能性があるため、F2 を明示的に送って TSF context を再初期化する。
                            let prepend_f2 = !resend.confirmed_warm;
                            let outcome = WarmupOutcome {
                                prepend_f2_warmup: prepend_f2,
                                used_eager_path: false,
                                cold_seq,
                            };
                            log::debug!(
                                "[sacr-warmup] cold={cold_seq} 実ローマ字 {:?} を {} パスで再送",
                                resend.romaji,
                                if prepend_f2 { "F2+warm" } else { "warm" },
                            );
                            io.transmit_tsf(&resend.romaji, &chars, &outcome);
                            io.send_deferred_vks(&io.take_pending_deferred_vks(), VkMarker::Tsf);
                        }
                    }
                }
                // SacrificialWarmupCoro は Done を後続 action として emit しているため
                // ここでは machine 状態を更新せず Continue を返す（queue が Done を処理する）。
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
                // TSF mode + consecutive==0 の場合は GjiWarmupCoro が SendRecoveryBs +
                // StartSacrificialWarmup を直接 emit するため、このハンドラには到達しない。
                // ここに来るのは非 TSF パス（Chrome 等）か give-up（consecutive>0）のみ。
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
        deferred_vks_called: Cell<bool>,
        send_fresh_f2_called: Cell<bool>,
        send_extra_f2_called: Cell<bool>,
        set_raw_literal_called: Cell<bool>,
        mark_cold_raw_tsf_called: Cell<bool>,
        increment_consecutive_called: Cell<bool>,
        send_literal_recovery_bs_called: Cell<bool>,
        send_literal_recovery_esc_bs_called: Cell<bool>,
        send_sacrificial_ime_off_on_called: Cell<bool>,
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
                deferred_vks_called: Cell::new(false),
                send_fresh_f2_called: Cell::new(false),
                send_extra_f2_called: Cell::new(false),
                set_raw_literal_called: Cell::new(false),
                mark_cold_raw_tsf_called: Cell::new(false),
                increment_consecutive_called: Cell::new(false),
                send_literal_recovery_bs_called: Cell::new(false),
                send_literal_recovery_esc_bs_called: Cell::new(false),
                send_sacrificial_ime_off_on_called: Cell::new(false),
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
        fn send_deferred_vks(&self, _vks: &[DeferredVk], _marker: VkMarker) {
            self.deferred_vks_called.set(true);
        }
        fn take_pending_deferred_vks(&self) -> Vec<DeferredVk> {
            vec![]
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

        fn send_sacrificial_ime_off_on(&self, _cold_seq: u32) {
            self.send_sacrificial_ime_off_on_called.set(true);
        }

        fn send_sacrificial_vk_a_with_bs(&self, _cold_seq: u32) {}

        fn send_sacrificial_bs_one(&self, _cold_seq: u32) {}

        fn send_literal_recovery_bs(&self, _backs: usize, _cold_seq: u32) {
            self.send_literal_recovery_bs_called.set(true);
        }

        fn send_literal_recovery_esc_bs(&self, _backs: usize, _cold_seq: u32) {
            self.send_literal_recovery_esc_bs_called.set(true);
        }

        fn send_chrome_gji_reinit_and_poll(&self, _cold_seq: u32) {}

        fn send_unicode_char_direct(&self, _ch: char) {}
    }

    fn make_chrome_machine() -> crate::tsf::warmup::probe_fsm::TsfProbeCoro {
        make_chrome_machine_with_cold(true)
    }

    fn make_chrome_machine_with_cold(
        is_long_cold: bool,
    ) -> crate::tsf::warmup::probe_fsm::TsfProbeCoro {
        let guard = OutputActiveGuard::noop_for_test();
        let probe = crate::tsf::probe::TsfReadinessProbe::new(0, 0, 0);
        crate::tsf::warmup::probe_fsm::TsfProbeCoro::new_chrome(
            "ka",
            0,
            probe,
            0,
            guard,
            is_long_cold,
        )
    }

    fn make_gji_machine() -> crate::tsf::warmup::gji_warmup_coro::GjiWarmupCoro {
        make_gji_machine_with_cold(crate::tuning::SETTLE_TIMEOUT_MS, false)
    }

    fn make_gji_machine_with_cold(
        ncwait_budget_ms: u64,
        forces_prepend_f2: bool,
    ) -> crate::tsf::warmup::gji_warmup_coro::GjiWarmupCoro {
        let is_long_cold = ncwait_budget_ms == crate::tuning::GJI_LONG_IDLE_PROBE_TOTAL_MS;
        let probe = crate::tsf::probe::TsfReadinessProbe::new(0, 0, 0);
        crate::tsf::warmup::gji_warmup_coro::GjiWarmupCoro::new(
            "ka",
            0,
            probe,
            0,
            false,
            ColdReason::FocusChange,
            false,
            false,
            ncwait_budget_ms,
            forces_prepend_f2,
            is_long_cold,
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
                gji_resumed: false,
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
                gji_resumed: false,
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
                gji_resumed: false,
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
                gji_resumed: false,
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
                gji_resumed: false,
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
                gji_resumed: false,
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
        let io = FakeProbeIo::default();
        let mut machine =
            make_gji_machine_with_cold(crate::tuning::GJI_LONG_IDLE_PROBE_TOTAL_MS, true);
        let actions = vec![ProbeAction::SendFreshF2 {
            cold_seq: 0,
            probe_settled: false,
        }];
        // forces_prepend_f2=true (Long cold) のとき:
        // - send_fresh_f2 と send_extra_f2 が呼ばれる（F2×2 連続）
        // - GjiWarmupCoro が GJI I/O 応答を待つため Done を即返さない
        let result = dispatch_probe_actions(&mut machine, actions, &io);
        assert!(
            !result.is_done(),
            "forces_prepend_f2: GJI I/O 応答を待つため Done を即返さないべき"
        );
        assert!(
            io.send_fresh_f2_called.get(),
            "send_fresh_f2 が呼ばれるべき"
        );
        assert!(
            io.send_extra_f2_called.get(),
            "forces_prepend_f2: 追加 F2 で F2×2 連続にするべき"
        );
        assert!(
            !io.transmit_tsf_called.get(),
            "TransmitTsf は即実行されないべき"
        );
    }

    #[test]
    fn send_fresh_f2_with_medium_cold_sends_extra_f2_and_waits_namechange() {
        // 再現テスト: ColdKind::Medium（7s〜10s idle）
        // cold=7 "このろぐ → kおのろぐ" バグ: GJI が fresh F2 から 325ms 後に起動するため
        // SETTLE_TIMEOUT_MS (300ms) では間に合わず "kお" になっていた。
        // gji_long_idle_probe=true + MEDIUM_IDLE_PROBE_TOTAL_MS (550ms) で GJI I/O を待てること。
        // forces_prepend_f2=true だが Long ではないので追加 F2 なし（F2×1 のみ）。
        // ※ Medium の forces_prepend_f2=true は「F2×2 を強制」ではなく「gji_long_idle_probe=true」の意味。
        let io = FakeProbeIo::default();
        // Medium cold: forces_prepend_f2=true (gji_long_idle_probe 有効), budget=MEDIUM_IDLE_PROBE_TOTAL_MS
        let mut machine =
            make_gji_machine_with_cold(crate::tuning::MEDIUM_IDLE_PROBE_TOTAL_MS, true);
        let actions = vec![ProbeAction::SendFreshF2 {
            cold_seq: 0,
            probe_settled: false,
        }];
        let result = dispatch_probe_actions(&mut machine, actions, &io);
        assert!(
            !result.is_done(),
            "medium idle: GJI I/O 応答を待つため Done を即返さないべき"
        );
        assert!(
            io.send_fresh_f2_called.get(),
            "send_fresh_f2 が呼ばれるべき"
        );
        assert!(
            io.send_extra_f2_called.get(),
            "medium idle (forces_prepend_f2=true): F2×2 を送るべき"
        );
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
            observations: ProbeObservations {
                nc_fired: false,
                gji_resumed: false,
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
                gji_resumed: false,
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
                gji_resumed: false,
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
                needs_literal: true, // should_prepend_f2 && gji_active && (!gji_long_idle || is_tsf_mode) && !gji_resumed
                literal_detect_ms: crate::tuning::RAW_TSF_LITERAL_DETECT_MS_LONG_IDLE,
            },
            observations: ProbeObservations {
                nc_fired: false,
                gji_resumed: false,
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
                gji_resumed: false,
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
            observations: ProbeObservations {
                nc_fired: true,
                gji_resumed: true,
            },
            romaji: "to".to_string(),
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
            observations: ProbeObservations {
                nc_fired: false,
                gji_resumed: false,
            },
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
    fn literal_recovery_send_recovery_bs_then_sacr_warmup_switches_fsm() {
        // LiteralDetectFsm が TSF mode + consecutive==0 で partial literal を検出した場合:
        // [SendRecoveryBs, StartSacrificialWarmup(from_literal_recovery=true), Done] を emit する。
        // dispatch_probe_actions がこの列を処理し:
        //   1. send_literal_recovery_bs (BS×backs で terminal cleanup)
        //   2. increment_consecutive_count (ループ防止)
        //   3. send_sacrificial_ime_off_on (VK_IME_OFF→ON 送信)
        //   4. SacrificialWarmupCoro に SwitchMachine
        let io = FakeProbeIo {
            consecutive: 0,
            ..Default::default()
        };
        let mut machine = make_gji_machine();
        let actions = vec![
            ProbeAction::SendRecoveryBs {
                cold_seq: 0,
                backs: 2,
                escape_composition: false,
            },
            ProbeAction::StartSacrificialWarmup(
                crate::tsf::warmup::probe_fsm::LiteralDetectConfig {
                    cold_seq: 0,
                    romaji: "ko".to_string(),
                    plan: TransmitPlan {
                        should_prepend_f2: false,
                        used_eager_path: false,
                        needs_literal: true,
                        literal_detect_ms: crate::tuning::RAW_TSF_LITERAL_DETECT_MS,
                    },
                    observations: ProbeObservations {
                        nc_fired: false,
                        gji_resumed: false,
                    },
                    literal_detect_ms: crate::tuning::RAW_TSF_LITERAL_DETECT_MS,
                    target: TransmitTarget::Tsf,
                    from_literal_recovery: true,
                },
            ),
            ProbeAction::Done,
        ];
        let result = dispatch_probe_actions(&mut machine, actions, &io);
        assert!(
            matches!(result, DispatchResult::SwitchMachine(_)),
            "from_literal_recovery: SacrificialWarmupCoro に SwitchMachine するべき"
        );
        assert!(
            io.send_literal_recovery_bs_called.get(),
            "SendRecoveryBs: send_literal_recovery_bs を呼ぶべき"
        );
        assert!(
            io.increment_consecutive_called.get(),
            "from_literal_recovery=true: increment_consecutive_count を呼ぶべき"
        );
        assert!(
            io.send_sacrificial_ime_off_on_called.get(),
            "StartSacrificialWarmup(TSF): send_sacrificial_ime_off_on を呼ぶべき"
        );
        assert!(
            !io.set_raw_literal_called.get(),
            "sacr warmup パス: set_raw_literal は呼ばない"
        );
        assert!(
            !io.mark_cold_raw_tsf_called.get(),
            "sacr warmup パス: mark_cold_raw_tsf は呼ばない"
        );
    }

    // SendRecoveryBs{escape_composition: true}（partial literal 回収）は
    // send_literal_recovery_esc_bs を呼び、無印の send_literal_recovery_bs は呼ばない。
    #[test]
    fn send_recovery_bs_with_escape_composition_calls_esc_bs_variant() {
        let io = FakeProbeIo {
            consecutive: 0,
            ..Default::default()
        };
        let mut machine = make_gji_machine();
        let actions = vec![
            ProbeAction::SendRecoveryBs {
                cold_seq: 0,
                backs: 1,
                escape_composition: true,
            },
            ProbeAction::StartSacrificialWarmup(
                crate::tsf::warmup::probe_fsm::LiteralDetectConfig {
                    cold_seq: 0,
                    romaji: "ko".to_string(),
                    plan: TransmitPlan {
                        should_prepend_f2: false,
                        used_eager_path: false,
                        needs_literal: true,
                        literal_detect_ms: crate::tuning::RAW_TSF_LITERAL_DETECT_MS,
                    },
                    observations: ProbeObservations {
                        nc_fired: false,
                        gji_resumed: false,
                    },
                    literal_detect_ms: crate::tuning::RAW_TSF_LITERAL_DETECT_MS,
                    target: TransmitTarget::Tsf,
                    from_literal_recovery: true,
                },
            ),
            ProbeAction::Done,
        ];
        dispatch_probe_actions(&mut machine, actions, &io);
        assert!(
            io.send_literal_recovery_esc_bs_called.get(),
            "escape_composition=true: send_literal_recovery_esc_bs を呼ぶべき"
        );
        assert!(
            !io.send_literal_recovery_bs_called.get(),
            "escape_composition=true: 無印の send_literal_recovery_bs は呼ばないべき"
        );
    }

    #[test]
    fn start_sacrificial_warmup_without_literal_recovery_does_not_increment_consecutive() {
        // GjiWarmupCoro の通常 cold-start パス（from_literal_recovery=false）では
        // increment_consecutive_count を呼ばない。
        let io = FakeProbeIo {
            consecutive: 0,
            ..Default::default()
        };
        let mut machine = make_gji_machine();
        let actions = vec![ProbeAction::StartSacrificialWarmup(
            crate::tsf::warmup::probe_fsm::LiteralDetectConfig {
                cold_seq: 0,
                romaji: "ko".to_string(),
                plan: TransmitPlan {
                    should_prepend_f2: false,
                    used_eager_path: false,
                    needs_literal: true,
                    literal_detect_ms: crate::tuning::RAW_TSF_LITERAL_DETECT_MS,
                },
                observations: ProbeObservations {
                    nc_fired: true,
                    gji_resumed: false,
                },
                literal_detect_ms: crate::tuning::RAW_TSF_LITERAL_DETECT_MS,
                target: TransmitTarget::Tsf,
                from_literal_recovery: false,
            },
        )];
        dispatch_probe_actions(&mut machine, actions, &io);
        assert!(
            !io.increment_consecutive_called.get(),
            "from_literal_recovery=false: increment_consecutive_count を呼ばないべき"
        );
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
