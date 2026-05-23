//! TSF cold-start ウォームアップシーケンス。
//!
//! [`ColdWarmupSequence`] は F2 送信等の即時処理を行い [`WarmupStarted`] を返す。
//! 残りの GJI 静止待ちは TIMER_TSF_PROBE + `TsfReadinessProbe::check_now` で行う。
//!
//! ## パス分岐
//!
//! ```text
//! run_start()
//!   └─ preamble()           : 診断ログ・IMM32 ローマ字モード設定・cold_seq インクリメント
//!      ├─ run_eager_start()  : eager warmup パス (eager_warmup_sent_ms != 0)
//!      │    ├─ FreshF2        (remaining == 0 && !requires_settle) → F2 送信 + probe
//!      │    ├─ ProbeWithSettle(remaining == 0 &&  requires_settle) → F2 再送 + 500ms 待機
//!      │    └─ ReWarmup       (remaining > 0)                      → probe (NAMECHANGE 確認付き)
//!      └─ run_non_eager_start(): F2×2 送信 + WarmupStarted (GjiProbe)
//! ```

use std::mem::size_of;
use std::sync::atomic::Ordering::Relaxed;

use windows::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT};

use crate::output::Output;
use crate::tsf::output::make_tsf_key_input;
use crate::tuning::LONG_IDLE_MS;

use super::send::send_vk_dbe_hiragana_pair;

/// VK_DBE_HIRAGANA 仮想キーコード (F2 相当)
const VK_DBE_HIRAGANA: u16 = 0xF2;

/// eager パスの 3 分岐を表す enum。
///
/// `run_eager_start()` 内で `WarmupKind::from_context()` により生成し、
/// `match` で各パスの処理を統一する。
enum WarmupKind {
    /// `remaining == 0 && !requires_settle`: 通常の fresh F2 → probe
    FreshF2,
    /// `remaining == 0 &&  requires_settle`: F2 再送 + 500ms 待機（settle なし）
    ProbeWithSettle,
    /// `remaining > 0`: eager 起点でそのまま probe（NAMECHANGE 確認付き）
    ReWarmup { _remaining_ms: u64 },
}

impl WarmupKind {
    const fn from_context(remaining: u64, requires_settle: bool) -> Self {
        if remaining > 0 {
            Self::ReWarmup { _remaining_ms: remaining }
        } else if requires_settle {
            Self::ProbeWithSettle
        } else {
            Self::FreshF2
        }
    }
}
/// `preamble()` が計算した warmup パラメータをまとめるコンテキスト。
///
/// `run_eager` / `run_non_eager` 等の各サブメソッドに渡すことで引数を一本化する。
struct WarmupContext {
    /// cold-start シーケンス番号（ログ相関用）
    cold_seq: u32,
    /// VK_DBE_HIRAGANA 送信後の eager settle 最大待機時間 (ms)
    eager_settle_ms: u64,
    /// VK_DBE_HIRAGANA 送信後の GJI I/O 観測を開始するまでの最小待機時間 (ms)
    probe_min_ms: u64,
    /// cold になった理由（ログ用）
    cold_reason: crate::output::ColdReason,
}

/// `ColdWarmupSequence::run_start` の戻り値。
///
/// 即座に実行できる部分（F2 送信等）は完了済み。
/// 残りの待機はタイマー（TIMER_TSF_PROBE）で `TsfReadinessProbe::check_now` を
/// ポーリングすることで行う。
pub struct WarmupStarted {
    /// GJI 静止プローブ
    pub probe: crate::tsf::probe::TsfReadinessProbe,
    /// probe の最大待機時間 (ms, warmup_sent_ms 起点)
    pub total_max_ms: u64,
    /// プローブ完了後に NAMECHANGE 確認フェーズが必要かどうか
    /// (`eager_probe_with_settle` パスのみ `true`)
    pub needs_settle_check: bool,
    /// cold になった理由（NAMECHANGE フェーズの判断に使用）
    pub cold_reason: crate::output::ColdReason,
}

/// TSF cold-start ウォームアップシーケンスを管理する構造体。
///
/// `Output::execute_cold_warmup` のロジックを複数のプライベートメソッドに分解し、
/// 可読性・テスト性を高める。
///
/// `run_start()` を呼ぶと即座に実行できる部分（F2 送信等）を行い [`WarmupStarted`] を返す。
/// `run()` は旧来のブロッキング API（テスト互換用）。
pub struct ColdWarmupSequence<'a> {
    output: &'a Output,
}

impl<'a> ColdWarmupSequence<'a> {
    /// 新しいシーケンスを生成する。
    pub const fn new(output: &'a Output) -> Self {
        Self { output }
    }

    /// ノンブロッキング版ウォームアップ開始。
    ///
    /// 即座に実行できる部分（F2 送信、IMM32 設定等）を行い [`WarmupStarted`] を返す。
    /// 残りの GJI 静止待ちは TIMER_TSF_PROBE + `TsfReadinessProbe::check_now` で行う。
    pub fn run_start(&self, session_expired: bool, elapsed_ms: u64) -> WarmupStarted {
        let ctx = self.preamble(session_expired, elapsed_ms);

        if session_expired {
            log::debug!(
                "[h1-warmup] cold={} session expired → fresh VK_DBE_HIRAGANA 送信 (500ms待機を強制)",
                ctx.cold_seq
            );
            self.output.send_eager_tsf_warmup();
        }

        let eager_ms = self.output.composition.eager_warmup_sent_ms();
        let now_ms = crate::hook::current_tick_ms();
        let eager_elapsed =
            if eager_ms != 0 { now_ms.saturating_sub(eager_ms) } else { u64::MAX };
        let use_eager = eager_ms != 0;

        log::debug!(
            "[h1-warmup] cold={} path={} eager_ms={eager_ms} now_ms={now_ms} elapsed={}ms",
            ctx.cold_seq,
            if use_eager { "eager" } else { "non-eager" },
            crate::output::fmt_ms(eager_elapsed),
        );

        if use_eager {
            Self::run_eager_start(&ctx, eager_ms, eager_elapsed)
        } else {
            Self::run_non_eager_start(&ctx)
        }
    }

    /// 準備フェーズ: 診断ログ出力・IMM32 設定・`cold_seq` インクリメントを行い
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
            conv_pre.is_some_and(|v| v & 0x0001 != 0),
            conv_pre.is_some_and(|v| v & 0x0010 != 0),
            conv_pre.is_some_and(|v| v & 0x0002 != 0),
        );
        // SAFETY: IMM32 API; sets conversion mode on the foreground window's IME context.
        // IMM32 経由で同期的にローマ字モードへ切り替え。
        unsafe { let _ = crate::ime::set_ime_romaji_mode(); }

        let cold_seq = self.output.composition.increment_cold_start_count();

        // SAFETY: Win32 GetForegroundWindow + GetClassName; returns empty string on failure.
        let win_class = unsafe { crate::ime::get_foreground_window_class() };
        log::debug!("[h1-window] cold={cold_seq} class={win_class}");

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
                "[h1-warmup] cold={cold_seq} PassthroughConfirmKey/ReinjectConfirmKey + long idle \
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
            "[h1-warmup] cold={cold_seq} eager_settle_ms={eager_settle_ms}ms probe_min_ms={probe_min_ms}ms \
             reason={:?} long_idle={long_idle} idle_at_cold={}ms",
            cold_reason,
            self.output.composition.idle_ms_at_last_cold()
        );

        WarmupContext { cold_seq, eager_settle_ms, probe_min_ms, cold_reason }
    }

    /// non-eager: F2×2 を送信して WarmupStarted を返す。
    fn run_non_eager_start(ctx: &WarmupContext) -> WarmupStarted {
        log::debug!(
            "[h1-warmup] cold={} non-eager: VK_DBE_HIRAGANA warmup+probe 送信",
            ctx.cold_seq
        );
        let ime_on_probe = [
            make_tsf_key_input(VK_DBE_HIRAGANA, false),
            make_tsf_key_input(VK_DBE_HIRAGANA, true),
            make_tsf_key_input(VK_DBE_HIRAGANA, false),
            make_tsf_key_input(VK_DBE_HIRAGANA, true),
        ];
        // SAFETY: ime_on_probe is a valid array of INPUT structs.
        unsafe {
            SendInput(
                &ime_on_probe,
                i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32"),
            );
        }
        let probe_sent_ms = crate::hook::current_tick_ms();
        WarmupStarted {
            probe: crate::tsf::probe::TsfReadinessProbe::new(
                probe_sent_ms,
                ctx.cold_seq,
                ctx.probe_min_ms,
            ),
            total_max_ms: ctx.eager_settle_ms,
            needs_settle_check: false,
            cold_reason: ctx.cold_reason,
        }
    }

    /// eager ノンブロッキック開始: パスを判定して F2 を送信し WarmupStarted を返す。
    fn run_eager_start(ctx: &WarmupContext, eager_ms: u64, eager_elapsed: u64) -> WarmupStarted {
        let remaining = ctx.eager_settle_ms.saturating_sub(eager_elapsed);
        let requires_settle = ctx.cold_reason.requires_settle();
        let kind = WarmupKind::from_context(remaining, requires_settle);

        match kind {
            WarmupKind::FreshF2 => {
                // eager_fresh_f2_then_probe: fresh F2 + probe
                let last_io = crate::tsf::observer::TSF_OBS.gji_last_io_ms.load(Relaxed);
                let gji_idle = crate::hook::current_tick_ms().saturating_sub(last_io);
                log::debug!(
                    "[h1-warmup] cold={} eager: {}ms 経過 (gji_idle={gji_idle}ms) → fresh F2 start",
                    ctx.cold_seq, ctx.eager_settle_ms,
                );
                // SAFETY: SendInput をメッセージループスレッドから呼ぶ。
                let fresh_f2_ms = unsafe { send_vk_dbe_hiragana_pair() };
                WarmupStarted {
                    probe: crate::tsf::probe::TsfReadinessProbe::new(
                        fresh_f2_ms,
                        ctx.cold_seq,
                        ctx.probe_min_ms,
                    ),
                    total_max_ms: ctx.eager_settle_ms,
                    needs_settle_check: false,
                    cold_reason: ctx.cold_reason,
                }
            }
            WarmupKind::ProbeWithSettle => {
                // eager_re_warmup: fresh F2 を送信して 500ms 待機
                log::debug!(
                    "[h1-warmup] cold={} eager: {}ms 経過 → 再warmup start",
                    ctx.cold_seq, ctx.eager_settle_ms,
                );
                // SAFETY: SendInput をメッセージループスレッドから呼ぶ。
                let re_warmup_ms = unsafe { send_vk_dbe_hiragana_pair() };
                WarmupStarted {
                    probe: crate::tsf::probe::TsfReadinessProbe::new(
                        re_warmup_ms,
                        ctx.cold_seq,
                        ctx.probe_min_ms,
                    ),
                    total_max_ms: crate::tuning::RE_WARMUP_MS,
                    needs_settle_check: false,
                    cold_reason: ctx.cold_reason,
                }
            }
            WarmupKind::ReWarmup { .. } => {
                // eager_probe_with_settle: eager_ms 起点のプローブ（NAMECHANGE チェックが必要）
                log::debug!(
                    "[h1-warmup] cold={} eager: elapsed={}ms → probe start (budget={}ms from warmup)",
                    ctx.cold_seq, eager_elapsed, ctx.eager_settle_ms,
                );
                WarmupStarted {
                    probe: crate::tsf::probe::TsfReadinessProbe::new(
                        eager_ms,
                        ctx.cold_seq,
                        ctx.probe_min_ms,
                    ),
                    total_max_ms: ctx.eager_settle_ms,
                    needs_settle_check: true,
                    cold_reason: ctx.cold_reason,
                }
            }
        }
    }
}

