//! TSF cold-start ウォームアップシーケンス。
//!
//! `Output::execute_cold_warmup()` の処理を `ColdWarmupSequence` に委譲する。
//! 内部分解は次フェーズで行う — このファイルは骨格のみ。

use std::mem::size_of;
use std::sync::atomic::Ordering::Relaxed;

use windows::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT};

use crate::tsf::output::make_tsf_key_input;

/// `u64::MAX` は「未送信」を意味するセンチネル値。ログ表示用に "∞" に変換する。
fn fmt_ms(ms: u64) -> String {
    if ms == u64::MAX { "∞".to_owned() } else { ms.to_string() }
}

/// TSF cold-start ウォームアップシーケンスの実行コンテキスト。
///
/// `Output::execute_cold_warmup()` の処理を保持する。
/// `run()` を呼ぶと cold-start シーケンス番号を返す。
pub(crate) struct ColdWarmupSequence<'a> {
    output: &'a crate::output::Output,
    session_expired: bool,
    elapsed_ms: u64,
}

impl<'a> ColdWarmupSequence<'a> {
    pub(crate) fn new(
        output: &'a crate::output::Output,
        session_expired: bool,
        elapsed_ms: u64,
    ) -> Self {
        Self { output, session_expired, elapsed_ms }
    }

    /// ウォームアップシーケンスを実行し、cold-start シーケンス番号を返す。
    ///
    /// # Safety
    /// 内部で `SendInput` を呼ぶ。メッセージループスレッドから呼ぶこと。
    pub(crate) unsafe fn run(self) -> u32 {
        let session_expired = self.session_expired;
        let elapsed_ms = self.elapsed_ms;

        const VK_DBE_HIRAGANA: u16 = 0xF2;
        // cold 発生前の idle 時間が長い場合（ナビゲーション等）、GJI が TSF セッションを
        // リセットしている可能性があり、再初期化に FocusChange 相当の時間が必要。
        // 閾値は 10s: 2-9s 程度の「考える・少し読む」では GJI セッションが生存しており
        // I/O が発火せず probe が 1500ms タイムアウトしてしまうため、低すぎる閾値は NG。
        // 10s 以上の長期 idle（矢印キーナビゲーション等）では GJI セッションリセットが確実。
        const LONG_IDLE_MS: u64 = 10_000;

        if session_expired {
            log::debug!("[tsf-warmup] session expired ({elapsed_ms}ms) → F2-only先行バッチ (案A)");
        } else {
            log::debug!("[tsf-warmup] cold → F2-only先行バッチ (案A)");
        }
        // H4/H5 判定: pre-send で ROMAN=true なら IMM32 は正しいが TSF が無視している。
        // SAFETY: IMM32 API; uses the foreground thread's IME context, valid during message loop.
        let conv_pre = unsafe { crate::ime::get_ime_conversion_mode_raw() };
        log::debug!(
            "[cold-diag] pre-send conv={} NATIVE={} ROMAN={} KATAKANA={}",
            conv_pre.map_or_else(|| "none".to_string(), |v| format!("0x{v:08X}")),
            conv_pre.map_or(false, |v| v & 0x0001 != 0),
            conv_pre.map_or(false, |v| v & 0x0010 != 0),
            conv_pre.map_or(false, |v| v & 0x0002 != 0),
        );
        // SAFETY: IMM32 API; sets conversion mode on the foreground window's IME context.
        // IMM32 経由で同期的にローマ字モードへ切り替え。
        unsafe { let _ = crate::ime::set_ime_romaji_mode(); }

        let cold_n = self.output.composition.increment_cold_start_count();

        // SAFETY: Win32 GetForegroundWindow + GetClassName; returns empty string on failure.
        let win_class = unsafe { crate::ime::get_foreground_window_class() };
        log::debug!("[h1-window] cold={cold_n} class={win_class}");

        let long_idle = self.output.composition.idle_ms_at_last_cold() > LONG_IDLE_MS;
        // ColdReason に応じてウォームアップ待機時間を決定:
        //   FocusChange / SetOpenTrue / NativeF2Consumed:
        //     awase が物理キーを消費して VK_DBE_HIRAGANA を代わりに送るため、
        //     GJI から見ると FocusChange 相当の TSF 再初期化が発生しうる。
        //     実測で候補窓出現まで 1031ms かかることがあるため 1500ms を上限とする。
        //   PassthroughConfirmKey / ReinjectConfirmKey + long_idle:
        //     Enter/Space/Escape 後でも長期 idle 後は GJI セッションがリセットされ、
        //     500ms のバジェットでは不足する（kおのじしょう バグ）。1500ms に拡張する。
        //   その他（Enter/Space/記号等）: composition 再突入のみ → 500ms
        let cold_reason = self.output.composition.last_cold_reason();
        if cold_reason.is_confirm_key() && long_idle {
            log::debug!(
                "[h1-warmup] cold={cold_n} PassthroughConfirmKey/ReinjectConfirmKey + long idle \
                 ({}ms) → eager_settle_ms=1500ms",
                self.output.composition.idle_ms_at_last_cold()
            );
        }
        let eager_settle_ms: u64 = cold_reason.eager_settle_ms(long_idle);
        // ColdReason に応じた probe 最小待機時間（warmup_sent_ms 起点）:
        //   VK_DBE_HIRAGANA がキューに入ってから GJI が最初の I/O を開始するまでの
        //   実測下限。この時間内は GJI I/O 監視結果を信頼しない。
        let probe_min_ms: u64 = cold_reason.probe_min_ms(long_idle);
        log::debug!(
            "[h1-warmup] cold={cold_n} eager_settle_ms={eager_settle_ms}ms probe_min_ms={probe_min_ms}ms \
             reason={:?} long_idle={long_idle} idle_at_cold={}ms",
            self.output.composition.last_cold_reason(),
            self.output.composition.idle_ms_at_last_cold()
        );

        // session_expired: 2秒以上放置後は TSF composition context がリセット済みの可能性大。
        // 古い eager_warmup_sent_ms を使って「elapsed >= 500ms → スリープなし」にすると、
        // TSF が cold なまま 'd' 等が literal になる（dえーた バグ）。
        // fresh な VK_DBE_HIRAGANA を送信して eager_warmup_sent_ms を更新し、500ms 待機を強制する。
        if session_expired {
            log::debug!("[h1-warmup] cold={cold_n} session expired → fresh VK_DBE_HIRAGANA 送信 (500ms待機を強制)");
            self.output.send_eager_tsf_warmup();
        }

        let eager_ms = self.output.composition.eager_warmup_sent_ms();
        let now_ms = crate::hook::current_tick_ms();
        let eager_elapsed =
            if eager_ms != 0 { now_ms.saturating_sub(eager_ms) } else { u64::MAX };
        let use_eager = eager_ms != 0;

        // どのパスを通るかを明示的にログ（根本原因判別用）
        log::debug!(
            "[h1-warmup] cold={cold_n} path={} eager_ms={eager_ms} now_ms={now_ms} elapsed={}ms",
            if use_eager { "eager" } else { "non-eager" },
            fmt_ms(eager_elapsed),
        );

        if use_eager {
            let remaining = eager_settle_ms.saturating_sub(eager_elapsed);
            if remaining == 0 {
                // eager_settle_ms 以上経過しているが、GJI は WM_SETFOCUS の遅延処理
                // (メッセージキュー滞留 500-900ms) で TSF context を再初期化することがある。
                // FocusChange / SetOpenTrue / NativeF2Consumed の場合はこの再初期化レースが
                // 発生しやすいため、新規 VK_DBE_HIRAGANA を送って再び 500ms 待機する。
                // PassthroughConfirmKey 等の composition-only reset では不要。
                let needs_re_warmup = cold_reason.requires_settle();
                if needs_re_warmup {
                    log::debug!(
                        "[h1-warmup] cold={cold_n} eager: {eager_elapsed}ms 経過 → 再warmup (GJI 再初期化レース対策)",
                    );
                    // SAFETY: SendInput via send_vk_dbe_hiragana_pair; called from message-loop thread.
                    let re_warmup_sent_ms = unsafe { crate::tsf::send::send_vk_dbe_hiragana_pair() };
                    const RE_WARMUP_MS: u64 = 500;
                    crate::tsf::probe::TsfReadinessProbe::new(
                        re_warmup_sent_ms, cold_n, probe_min_ms,
                    )
                    .wait_until_ready(RE_WARMUP_MS);
                    let actual_wait = crate::hook::current_tick_ms().saturating_sub(re_warmup_sent_ms);
                    log::debug!(
                        "[h1-warmup] cold={cold_n} 再warmup probe完了={actual_wait}ms",
                    );
                } else {
                    // eager F2 から時間が経過（elapsed >= eager_settle_ms）しており、
                    // gji_idle が大きい状態で即送信すると raw-tsf-literal false positive が発生する。
                    // （GJI candidate SHOW が 300ms を超えるため raw-tsf-literal がタイムアウト）
                    // → fresh F2 を送って TsfReadinessProbe で composition context を
                    //   再確認してから送信する。追加 ~140ms だが false positive を防げる。
                    let last_io = crate::tsf::observer::OBS_GJI_LAST_IO_MS.load(Relaxed);
                    let gji_idle = crate::hook::current_tick_ms().saturating_sub(last_io);
                    log::debug!(
                        "[h1-warmup] cold={cold_n} eager: {eager_elapsed}ms 経過 (gji_idle={gji_idle}ms) \
                         → fresh F2 + probe (raw-tsf-literal false positive 防止)",
                    );
                    // SAFETY: SendInput via send_vk_dbe_hiragana_pair; called from message-loop thread.
                    let fresh_f2_ms = unsafe { crate::tsf::send::send_vk_dbe_hiragana_pair() };
                    crate::tsf::probe::TsfReadinessProbe::new(
                        fresh_f2_ms, cold_n, probe_min_ms,
                    )
                    .wait_until_ready(eager_settle_ms);
                    let actual_wait = crate::hook::current_tick_ms().saturating_sub(fresh_f2_ms);
                    log::debug!(
                        "[h1-warmup] cold={cold_n} eager→fresh probe完了={actual_wait}ms",
                    );
                }
            } else {
                log::debug!(
                    "[h1-warmup] cold={cold_n} eager: elapsed={eager_elapsed}ms → probe (budget={eager_settle_ms}ms from warmup)",
                );
                // total_max_ms は warmup_sent_ms 起点の合計予算（remaining ではない）。
                // probe 内で max_deadline = eager_ms + eager_settle_ms が計算される。
                crate::tsf::probe::TsfReadinessProbe::new(
                    eager_ms, cold_n, probe_min_ms,
                )
                .wait_until_ready(eager_settle_ms);
                let total_elapsed = crate::hook::current_tick_ms().saturating_sub(eager_ms);
                log::debug!(
                    "[h1-warmup] cold={cold_n} probe完了 warmup経過={total_elapsed}ms",
                );

                // probe が GJI 活動なしでタイムアウトした場合、または NativeF2Consumed /
                // SetOpenTrue の cold start では、GJI idle だけでは WezTerm の TSF
                // composition context が ready かを保証できない。
                //
                // 理由: probe は std::thread::sleep でブロックするため、その間に
                // WezTerm が発行した OBJ_NAMECHANGE WinEvent がキューに溜まるが
                // 処理されない。probe 完了後に即ローマ字送信すると、WezTerm の TSF が
                // まだ活性化処理中の場合、先頭の 1 文字（例: 't'）が PTY に素通りし、
                // 次の文字（例: 'o'）が IME に捕捉されて "tお" になる。
                //
                // 修正: fresh F2 を送り wait_for_tsf_cold_settle でメッセージをポンプする。
                // probe sleep 中に溜まった pending NAMECHANGE を即処理するため、
                // NativeF2Consumed/SetOpenTrue では追加遅延はほぼ 0ms で済む。
                let gji_last = crate::tsf::observer::OBS_GJI_LAST_IO_MS.load(Relaxed);
                let probe_settled = gji_last >= eager_ms;
                let gji_monitor_ok = crate::tsf::observer::OBS_GJI_MONITOR_OK.load(Relaxed);

                let is_ime_init_cold = cold_reason.requires_settle();

                if (!probe_settled || is_ime_init_cold) && gji_monitor_ok {
                    // GJI probe timeout (no activity) または IME ON 初期化 cold start:
                    // TSF context が stale / 未確定の可能性あり → fresh F2 + settle。
                    //
                    // wait_for_tsf_cold_settle で OBJ_NAMECHANGE を reactive に待つ（上限 300ms）。
                    const SETTLE_TIMEOUT_MS: u32 = 300;
                    let nc_baseline = crate::OBS_FOCUS_NAMECHANGE_SEQ.load(Relaxed);
                    let settle_reason = if !probe_settled {
                        "probe timeout (no GJI activity)"
                    } else {
                        "NativeF2Consumed/SetOpenTrue (GJI settled だが NAMECHANGE 未処理の可能性)"
                    };
                    log::debug!(
                        "[h1-warmup] cold={cold_n} {settle_reason} \
                        → fresh F2 + tsf_cold_settle (up to {SETTLE_TIMEOUT_MS}ms, nc_seq={nc_baseline})",
                    );
                    // SAFETY: SendInput via send_vk_dbe_hiragana_pair; called from message-loop thread.
                    let fresh_f2_ms = unsafe { crate::tsf::send::send_vk_dbe_hiragana_pair() };
                    let settled = crate::output::wait_for_tsf_cold_settle(nc_baseline, SETTLE_TIMEOUT_MS);

                    // OBJ_NAMECHANGE 確認かつ GJI 活動なし（probe_settled=false）の場合、
                    // OBJ_NAMECHANGE は「WezTerm が F2 を処理した」シグナルだが
                    // GJI composition session 初期化の完了を意味しない。
                    // 直後にローマ字を送ると GJI がまだ初期化中で literal になる（raw TSF literal バグ）。
                    // → fresh F2 タイムスタンプ起点で GJI I/O 静止を待つ二次プローブを実施。
                    if settled && !probe_settled {
                        const GJI_POST_NAMECHANGE_MS: u64 = 300;
                        log::debug!(
                            "[h1-warmup] cold={cold_n} OBJ_NAMECHANGE後 GJI 二次プローブ (max {GJI_POST_NAMECHANGE_MS}ms)",
                        );
                        crate::tsf::probe::TsfReadinessProbe::new(
                            fresh_f2_ms, cold_n, 0,
                        )
                        .wait_until_ready(GJI_POST_NAMECHANGE_MS);
                        log::debug!("[h1-warmup] cold={cold_n} GJI 二次プローブ完了");
                    }
                }
            }
        } else {
            // 投機的プローブ: VK_DBE_HIRAGANA (F2相当) を2連送する。
            // 1回目 (warmup): TSF composition context 初期化をトリガー（VK_IME_ON では不足）
            // 2回目 (probe):  WezTerm が 1回目を処理済みであることを FIFO で保証
            // VK_DBE_HIRAGANA はひらがなモードへの切替のため、既にひらがななら実質冪等。
            log::debug!("[h1-warmup] cold={cold_n} non-eager: VK_DBE_HIRAGANA warmup+probe 送信");
            let ime_on_probe = [
                make_tsf_key_input(VK_DBE_HIRAGANA, false),
                make_tsf_key_input(VK_DBE_HIRAGANA, true),
                make_tsf_key_input(VK_DBE_HIRAGANA, false),
                make_tsf_key_input(VK_DBE_HIRAGANA, true),
            ];
            let t_pre = crate::hook::current_tick_ms();
            // SAFETY: ime_on_probe is a valid array of INPUT structs for the duration of the call.
            unsafe {
                SendInput(
                    &ime_on_probe,
                    i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32"),
                );
            }
            let elapsed = crate::hook::current_tick_ms().saturating_sub(t_pre);
            log::debug!("[h1-warmup] cold={cold_n} non-eager probe 完了 ({elapsed}ms)");
            // VK_DBE_HIRAGANA 単独では SendInput 完了後でも TSF 初期化に時間がかかる（実測: 40ms では不足）。
            // GJI I/O モニターが利用可能なら静止検出、なければ固定 sleep。
            let probe_sent_ms = crate::hook::current_tick_ms();
            crate::tsf::probe::TsfReadinessProbe::new(
                probe_sent_ms, cold_n, probe_min_ms,
            )
            .wait_until_ready(eager_settle_ms);
            log::debug!("[h1-warmup] cold={cold_n} non-eager probe完了");
        }

        cold_n
    }
}
