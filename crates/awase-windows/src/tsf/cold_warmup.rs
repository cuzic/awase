//! TSF cold-start ウォームアップシーケンス。
//!
//! [`ColdWarmupSequence`] は `Output::execute_cold_warmup` から切り出したロジックを
//! Preamble → Eager / Non-eager の 3 段に分解した構造体。
//!
//! ## パス分岐
//!
//! ```text
//! run()
//!   └─ preamble()           : 診断ログ・IMM32 ローマ字モード設定・cold_n インクリメント
//!      ├─ session-expired guard (run_non_eager / run_eager への振り分け前)
//!      ├─ run_eager()        : eager warmup パス (eager_warmup_sent_ms != 0)
//!      │    ├─ remaining == 0 かつ requires_settle → eager_re_warmup()
//!      │    ├─ remaining == 0 かつ !requires_settle → eager_fresh_f2_then_probe()
//!      │    └─ remaining > 0                        → eager_probe_with_settle()
//!      │         └─ run_secondary_gji_probe()  (GJI 活動なし or IME init cold の場合)
//!      └─ run_non_eager()   : 非 eager パス (eager_warmup_sent_ms == 0)
//! ```

use std::mem::size_of;
use std::sync::atomic::Ordering::Relaxed;

use windows::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT};

use crate::output::Output;
use crate::tsf::output::make_tsf_key_input;

/// VK_DBE_HIRAGANA 仮想キーコード (F2 相当)
const VK_DBE_HIRAGANA: u16 = 0xF2;

/// cold 発生前のアイドル時間がこれ以上なら「長期 idle」と判定する (ms)。
///
/// 2-9s 程度の「考える・少し読む」では GJI セッションが生存しているため、
/// 低すぎる閾値は NG（GJI I/O が発火せず probe が 1500ms でタイムアウトしてしまう）。
/// 10s 以上の長期 idle（矢印キーナビゲーション等）では GJI セッションリセットが確実。
const LONG_IDLE_MS: u64 = 10_000;

/// `preamble()` が計算した warmup パラメータをまとめるコンテキスト。
///
/// `run_eager` / `run_non_eager` 等の各サブメソッドに渡すことで引数を一本化する。
struct WarmupContext {
    /// cold-start シーケンス番号（ログ相関用）
    cold_n: u32,
    /// cold 発生前のアイドルが LONG_IDLE_MS を超えていたか
    long_idle: bool,
    /// VK_DBE_HIRAGANA 送信後の eager settle 最大待機時間 (ms)
    eager_settle_ms: u64,
    /// VK_DBE_HIRAGANA 送信後の GJI I/O 観測を開始するまでの最小待機時間 (ms)
    probe_min_ms: u64,
    /// cold になった理由（ログ用）
    cold_reason: crate::output::ColdReason,
}

/// TSF cold-start ウォームアップシーケンスを管理する構造体。
///
/// `Output::execute_cold_warmup` のロジックを複数のプライベートメソッドに分解し、
/// 可読性・テスト性を高める。
///
/// `run()` を呼ぶとウォームアップを実行し cold-start シーケンス番号を返す。
pub struct ColdWarmupSequence<'a> {
    output: &'a Output,
}

impl<'a> ColdWarmupSequence<'a> {
    /// 新しいシーケンスを生成する。
    pub fn new(output: &'a Output) -> Self {
        Self { output }
    }

    /// ウォームアップシーケンスを実行し、cold-start シーケンス番号を返す。
    ///
    /// 内部で以下の順序で処理する:
    /// 1. `preamble()` — 診断ログ・IMM32 設定・`cold_n` インクリメント
    /// 2. session_expired ガード (必要なら fresh F2 を送信)
    /// 3. `run_eager()` または `run_non_eager()` に振り分け
    pub fn run(&self, session_expired: bool, elapsed_ms: u64) -> u32 {
        let ctx = self.preamble(session_expired, elapsed_ms);

        // session_expired: 2秒以上放置後は TSF composition context がリセット済みの可能性大。
        // 古い eager_warmup_sent_ms を使って「elapsed >= 500ms → スリープなし」にすると、
        // TSF が cold なまま 'd' 等が literal になる（dえーた バグ）。
        // fresh な VK_DBE_HIRAGANA を送信して eager_warmup_sent_ms を更新し、500ms 待機を強制する。
        if session_expired {
            log::debug!(
                "[h1-warmup] cold={} session expired → fresh VK_DBE_HIRAGANA 送信 (500ms待機を強制)",
                ctx.cold_n
            );
            self.output.send_eager_tsf_warmup();
        }

        let eager_ms = self.output.composition.eager_warmup_sent_ms();
        let now_ms = crate::hook::current_tick_ms();
        let eager_elapsed =
            if eager_ms != 0 { now_ms.saturating_sub(eager_ms) } else { u64::MAX };
        let use_eager = eager_ms != 0;

        // どのパスを通るかを明示的にログ（根本原因判別用）
        log::debug!(
            "[h1-warmup] cold={} path={} eager_ms={eager_ms} now_ms={now_ms} elapsed={}ms",
            ctx.cold_n,
            if use_eager { "eager" } else { "non-eager" },
            crate::output::fmt_ms(eager_elapsed),
        );

        if use_eager {
            self.run_eager(&ctx, use_eager, eager_ms, eager_elapsed);
        } else {
            self.run_non_eager(&ctx);
        }

        ctx.cold_n
    }

    /// 準備フェーズ: 診断ログ出力・IMM32 設定・`cold_n` インクリメントを行い
    /// [`WarmupContext`] を返す。
    fn preamble(&self, session_expired: bool, elapsed_ms: u64) -> WarmupContext {
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
        let cold_reason = self.output.composition.last_cold_reason();

        // ColdReason に応じてウォームアップ待機時間を決定:
        //   FocusChange / SetOpenTrue / NativeF2Consumed:
        //     awase が物理キーを消費して VK_DBE_HIRAGANA を代わりに送るため、
        //     GJI から見ると FocusChange 相当の TSF 再初期化が発生しうる。
        //     実測で候補窓出現まで 1031ms かかることがあるため 1500ms を上限とする。
        //   PassthroughConfirmKey / ReinjectConfirmKey + long_idle:
        //     Enter/Space/Escape 後でも長期 idle 後は GJI セッションがリセットされ、
        //     500ms のバジェットでは不足する（kおのじしょう バグ）。1500ms に拡張する。
        //   その他（Enter/Space/記号等）: composition 再突入のみ → 500ms
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
            cold_reason,
            self.output.composition.idle_ms_at_last_cold()
        );

        WarmupContext { cold_n, long_idle, eager_settle_ms, probe_min_ms, cold_reason }
    }

    /// 非 eager パス: eager warmup なし（eager_warmup_sent_ms == 0）の場合の処理。
    ///
    /// VK_DBE_HIRAGANA を 2 連送し（warmup + probe）、GJI I/O 静止を待つ。
    fn run_non_eager(&self, ctx: &WarmupContext) {
        // 投機的プローブ: VK_DBE_HIRAGANA (F2相当) を2連送する。
        // 1回目 (warmup): TSF composition context 初期化をトリガー（VK_IME_ON では不足）
        // 2回目 (probe):  WezTerm が 1回目を処理済みであることを FIFO で保証
        // VK_DBE_HIRAGANA はひらがなモードへの切替のため、既にひらがななら実質冪等。
        log::debug!(
            "[h1-warmup] cold={} non-eager: VK_DBE_HIRAGANA warmup+probe 送信",
            ctx.cold_n
        );
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
        log::debug!("[h1-warmup] cold={} non-eager probe 完了 ({elapsed}ms)", ctx.cold_n);
        // VK_DBE_HIRAGANA 単独では SendInput 完了後でも TSF 初期化に時間がかかる（実測: 40ms では不足）。
        // GJI I/O モニターが利用可能なら静止検出、なければ固定 sleep。
        let probe_sent_ms = crate::hook::current_tick_ms();
        crate::tsf::probe::TsfReadinessProbe::new(probe_sent_ms, ctx.cold_n, ctx.probe_min_ms)
            .wait_until_ready(ctx.eager_settle_ms);
        log::debug!("[h1-warmup] cold={} non-eager probe完了", ctx.cold_n);
    }

    /// eager パス: remaining == 0 かつ `requires_settle()` のとき。
    ///
    /// GJI 再初期化レース対策として fresh F2 を再送し、500ms 待機する。
    fn eager_re_warmup(&self, ctx: &WarmupContext, probe_min_ms: u64) {
        log::debug!(
            "[h1-warmup] cold={} eager: {}ms 経過 → 再warmup (GJI 再初期化レース対策)",
            ctx.cold_n,
            // eager_elapsed はこのパスでは常に eager_settle_ms 以上
            ctx.eager_settle_ms,
        );
        let refresh_inputs = [
            make_tsf_key_input(VK_DBE_HIRAGANA, false),
            make_tsf_key_input(VK_DBE_HIRAGANA, true),
        ];
        // SAFETY: refresh_inputs is a valid array of INPUT structs for the duration of the call.
        unsafe {
            SendInput(
                &refresh_inputs,
                i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32"),
            );
        }
        const RE_WARMUP_MS: u64 = 500;
        let re_warmup_sent_ms = crate::hook::current_tick_ms();
        crate::tsf::probe::TsfReadinessProbe::new(re_warmup_sent_ms, ctx.cold_n, probe_min_ms)
            .wait_until_ready(RE_WARMUP_MS);
        let actual_wait = crate::hook::current_tick_ms().saturating_sub(re_warmup_sent_ms);
        log::debug!("[h1-warmup] cold={} 再warmup probe完了={actual_wait}ms", ctx.cold_n);
    }

    /// eager パス: remaining == 0 かつ `!requires_settle()` のとき。
    ///
    /// raw-tsf-literal false positive 防止のため fresh F2 + probe を実行する。
    fn eager_fresh_f2_then_probe(&self, ctx: &WarmupContext, probe_min_ms: u64) {
        let last_io = crate::tsf::observer::OBS_GJI_LAST_IO_MS.load(Relaxed);
        let gji_idle = crate::hook::current_tick_ms().saturating_sub(last_io);
        log::debug!(
            "[h1-warmup] cold={} eager: {}ms 経過 (gji_idle={gji_idle}ms) \
             → fresh F2 + probe (raw-tsf-literal false positive 防止)",
            ctx.cold_n,
            ctx.eager_settle_ms,
        );
        let refresh_inputs = [
            make_tsf_key_input(VK_DBE_HIRAGANA, false),
            make_tsf_key_input(VK_DBE_HIRAGANA, true),
        ];
        let fresh_f2_ms = crate::hook::current_tick_ms();
        // SAFETY: refresh_inputs is a valid array of INPUT structs for the duration of the call.
        unsafe {
            SendInput(
                &refresh_inputs,
                i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32"),
            );
        }
        crate::tsf::probe::TsfReadinessProbe::new(fresh_f2_ms, ctx.cold_n, probe_min_ms)
            .wait_until_ready(ctx.eager_settle_ms);
        let actual_wait = crate::hook::current_tick_ms().saturating_sub(fresh_f2_ms);
        log::debug!("[h1-warmup] cold={} eager→fresh probe完了={actual_wait}ms", ctx.cold_n);
    }

    /// eager パス: remaining > 0 のとき（まだバジェット内）。
    ///
    /// eager_warmup_sent_ms 起点で GJI I/O 静止プローブを実行する。
    /// probe 完了後に必要なら二次プローブ ([`run_secondary_gji_probe`]) を実施する。
    fn eager_probe_with_settle(&self, ctx: &WarmupContext, eager_ms: u64, eager_elapsed: u64) {
        log::debug!(
            "[h1-warmup] cold={} eager: elapsed={eager_elapsed}ms → probe (budget={}ms from warmup)",
            ctx.cold_n,
            ctx.eager_settle_ms,
        );
        // total_max_ms は warmup_sent_ms 起点の合計予算（remaining ではない）。
        // probe 内で max_deadline = eager_ms + eager_settle_ms が計算される。
        crate::tsf::probe::TsfReadinessProbe::new(eager_ms, ctx.cold_n, ctx.probe_min_ms)
            .wait_until_ready(ctx.eager_settle_ms);
        let total_elapsed = crate::hook::current_tick_ms().saturating_sub(eager_ms);
        log::debug!(
            "[h1-warmup] cold={} probe完了 warmup経過={total_elapsed}ms",
            ctx.cold_n
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
        let is_ime_init_cold = ctx.cold_reason.requires_settle();

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
                "[h1-warmup] cold={} {settle_reason} \
                → fresh F2 + tsf_cold_settle (up to {SETTLE_TIMEOUT_MS}ms, nc_seq={nc_baseline})",
                ctx.cold_n,
            );
            let refresh_inputs = [
                make_tsf_key_input(VK_DBE_HIRAGANA, false),
                make_tsf_key_input(VK_DBE_HIRAGANA, true),
            ];
            let fresh_f2_ms = crate::hook::current_tick_ms();
            // SAFETY: refresh_inputs is a valid array of INPUT structs for the duration of the call.
            unsafe {
                SendInput(
                    &refresh_inputs,
                    i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32"),
                );
            }
            let settled = wait_for_tsf_cold_settle(nc_baseline, SETTLE_TIMEOUT_MS);

            // OBJ_NAMECHANGE 確認かつ GJI 活動なし（probe_settled=false）の場合に
            // 二次プローブを実施する。
            if settled && !probe_settled {
                self.run_secondary_gji_probe(ctx, fresh_f2_ms);
            }
        }
    }

    /// GJI 二次プローブ: OBJ_NAMECHANGE 後に GJI I/O 静止を待つ。
    ///
    /// OBJ_NAMECHANGE は「WezTerm が F2 を処理した」シグナルだが、
    /// GJI composition session 初期化の完了を意味しない。
    /// 直後にローマ字を送ると GJI がまだ初期化中で literal になる（raw TSF literal バグ）。
    /// → fresh F2 タイムスタンプ起点で GJI I/O 静止を待つ二次プローブを実施。
    fn run_secondary_gji_probe(&self, ctx: &WarmupContext, fresh_f2_ms: u64) {
        const GJI_POST_NAMECHANGE_MS: u64 = 300;
        log::debug!(
            "[h1-warmup] cold={} OBJ_NAMECHANGE後 GJI 二次プローブ (max {GJI_POST_NAMECHANGE_MS}ms)",
            ctx.cold_n,
        );
        crate::tsf::probe::TsfReadinessProbe::new(fresh_f2_ms, ctx.cold_n, 0)
            .wait_until_ready(GJI_POST_NAMECHANGE_MS);
        log::debug!("[h1-warmup] cold={} GJI 二次プローブ完了", ctx.cold_n);
    }

    /// eager パスのディスパッチャ。
    ///
    /// `eager_elapsed` と `eager_settle_ms` の比較により以下の 3 サブメソッドに振り分ける:
    /// - `remaining == 0` かつ `requires_settle()` → [`eager_re_warmup`]
    /// - `remaining == 0` かつ `!requires_settle()` → [`eager_fresh_f2_then_probe`]
    /// - `remaining > 0` → [`eager_probe_with_settle`]
    fn run_eager(&self, ctx: &WarmupContext, _use_eager: bool, eager_ms: u64, eager_elapsed: u64) {
        let remaining = ctx.eager_settle_ms.saturating_sub(eager_elapsed);
        if remaining == 0 {
            // eager_settle_ms 以上経過しているが、GJI は WM_SETFOCUS の遅延処理
            // (メッセージキュー滞留 500-900ms) で TSF context を再初期化することがある。
            // FocusChange / SetOpenTrue / NativeF2Consumed の場合はこの再初期化レースが
            // 発生しやすいため、新規 VK_DBE_HIRAGANA を送って再び 500ms 待機する。
            // PassthroughConfirmKey 等の composition-only reset では不要。
            let needs_re_warmup = ctx.cold_reason.requires_settle();
            if needs_re_warmup {
                self.eager_re_warmup(ctx, ctx.probe_min_ms);
            } else {
                self.eager_fresh_f2_then_probe(ctx, ctx.probe_min_ms);
            }
        } else {
            self.eager_probe_with_settle(ctx, eager_ms, eager_elapsed);
        }
    }
}

/// TSF cold-start 後の composition context 初期化完了を reactive に待つ。
///
/// fresh F2 送信直後に呼ぶ。OBJ_NAMECHANGE WinEvent か タイムアウトまで待機する。
///
/// WezTerm は TSF composition ウィンドウ名をひらがなモード切替時に更新する (~125ms)。
/// このイベントで早期終了する。発火しない場合は timeout_ms まで待つ。
///
/// # Re-entrancy safety
/// OUTPUT_ACTIVE=true（send_keys スコープ）でメッセージループを動かしながら OBJ_NAMECHANGE を待つ。
/// `MsgWaitForMultipleObjects` を廃止し、`win32_async::block_on` + `sleep_ms` で実装。
///
/// Returns `true` = OBJ_NAMECHANGE 検出、`false` = タイムアウト
fn wait_for_tsf_cold_settle(nc_baseline: u32, timeout_ms: u32) -> bool {
    let settled = win32_async::block_on(settle_async(nc_baseline, timeout_ms));
    // drain は OutputActiveGuard::drop が行うため、ここでは呼ばない。

    let nc_fired = crate::OBS_FOCUS_NAMECHANGE_SEQ.load(Relaxed) != nc_baseline;
    log::debug!(
        "[tsf-settle] → {} (nc_fired={nc_fired})",
        if settled { "OBJ_NAMECHANGE" } else { "timeout" },
    );
    settled
}

/// OBJ_NAMECHANGE または タイムアウト まで非ブロッキングで待つ。
/// block_on の内部ループがメッセージをポンプするため WinEvent コールバックが発火し、
/// `OBS_FOCUS_NAMECHANGE_SEQ` が更新される。
async fn settle_async(nc_baseline: u32, timeout_ms: u32) -> bool {
    const POLL_MS: u32 = 5;

    let deadline_ms = crate::hook::current_tick_ms() + u64::from(timeout_ms);

    loop {
        if crate::OBS_FOCUS_NAMECHANGE_SEQ.load(Relaxed) != nc_baseline {
            return true;
        }
        let now = crate::hook::current_tick_ms();
        if now >= deadline_ms {
            return false;
        }
        let remaining = u32::try_from(deadline_ms.saturating_sub(now)).unwrap_or(u32::MAX);
        win32_async::sleep_ms(remaining.min(POLL_MS)).await;
    }
}

#[cfg(test)]
#[cfg(windows)]
mod tests {
    use super::*;

    // ── settle_async テスト ──────────────────────────────────────────────────────

    /// テスト間でグローバル OBS_FOCUS_NAMECHANGE_SEQ が競合しないようシリアライズ
    static SETTLE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// baseline がすでに変化していれば即 true を返す
    #[test]
    fn settle_returns_true_when_already_changed() {
        let _g = SETTLE_TEST_LOCK.lock().unwrap();
        // seq をインクリメントして baseline とずらす
        let baseline = crate::OBS_FOCUS_NAMECHANGE_SEQ
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let result = win32_async::block_on(settle_async(baseline, 500));
        assert!(result, "seq already != baseline → should return true");
    }

    /// NAMECHANGE が来ない場合は timeout_ms 後に false を返す
    #[test]
    fn settle_times_out_when_no_namechange() {
        let _g = SETTLE_TEST_LOCK.lock().unwrap();
        let current = crate::OBS_FOCUS_NAMECHANGE_SEQ
            .load(std::sync::atomic::Ordering::SeqCst);

        let start = std::time::Instant::now();
        let result = win32_async::block_on(settle_async(current, 100));
        let elapsed = start.elapsed().as_millis();

        assert!(!result, "no NAMECHANGE → should timeout with false");
        assert!(elapsed >= 60, "timed out too early: {elapsed}ms");
        assert!(elapsed < 500, "timed out too late: {elapsed}ms");
    }

    /// 待機中に別タスクから seq が変化したら true を返す
    #[test]
    fn settle_returns_true_on_namechange_during_wait() {
        let _g = SETTLE_TEST_LOCK.lock().unwrap();
        let baseline = crate::OBS_FOCUS_NAMECHANGE_SEQ
            .load(std::sync::atomic::Ordering::SeqCst);

        let result = win32_async::block_on(async {
            // 30ms 後に seq を変化させる spawn_local タスク
            win32_async::spawn_local(async {
                win32_async::sleep_ms(30).await;
                crate::OBS_FOCUS_NAMECHANGE_SEQ
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                // settle_async は 5ms ポーリングなので次のポーリングで検出
            });

            settle_async(baseline, 500).await
        });

        assert!(result, "NAMECHANGE fired during wait → should return true");
    }
}
