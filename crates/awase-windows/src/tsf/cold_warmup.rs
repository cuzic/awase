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
//!      │    ├─ ReWarmup       (remaining == 0 &&  requires_settle) → F2 再送 + RE_WARMUP_MS 待機
//!      │    └─ ProbeWithSettle(remaining > 0)                      → probe (NAMECHANGE 確認付き)
//!      └─ run_non_eager_start(): F2×2 送信 + WarmupStarted (GjiProbe)
//! ```

use std::sync::atomic::Ordering::Relaxed;

use crate::output::Output;
use crate::tuning::LONG_IDLE_MS;

use super::send::{send_vk_dbe_alpha_warmup, send_vk_dbe_hiragana_pair, send_vk_dbe_katakana_warmup};

/// charset に応じた warmup VK ペアを 1 回送信し、送信後の時刻を返す。
///
/// - `HankakuKatakana` → F1↓F1↑F3↓F3↑ (半角カタカナ)
/// - `ZenkakuKatakana` → F1↓F1↑ (全角カタカナ)
/// - `Hiragana` → F2↓F2↑ (ひらがな)
/// - `ZenkakuAlpha` → F0↓F0↑F4↓F4↑ (全角英数)
/// - `HankakuAlpha` → F0↓F0↑ (半角英数)
fn send_charset_warmup_pair(charset: awase::engine::Charset) -> u64 {
    use awase::engine::Charset;
    match charset {
        Charset::ZenkakuKatakana | Charset::HankakuKatakana => send_vk_dbe_katakana_warmup(charset),
        Charset::ZenkakuAlpha | Charset::HankakuAlpha => send_vk_dbe_alpha_warmup(charset),
        Charset::Hiragana => send_vk_dbe_hiragana_pair(),
    }
}

/// eager パスの 3 分岐を表す enum。
///
/// `run_eager_start()` 内で `WarmupKind::from_context()` により生成し、
/// `match` で各パスの処理を統一する。
enum WarmupKind {
    /// `remaining == 0 && !requires_settle`: 通常の fresh F2 → probe
    FreshF2,
    /// `remaining == 0 &&  requires_settle`: F2 再送 + RE_WARMUP_MS 待機（settle なし）
    ReWarmup,
    /// `remaining > 0`: eager 起点でそのまま probe（NAMECHANGE 確認付き）
    ProbeWithSettle,
}

impl WarmupKind {
    const fn from_context(remaining: u64, requires_settle: bool) -> Self {
        if remaining > 0 {
            Self::ProbeWithSettle
        } else if requires_settle {
            Self::ReWarmup
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
    /// `ConvModePolicy::AwaseLocked` のとき `true`。
    ///
    /// `false` (UserManaged) のとき VK_DBE_HIRAGANA 送信と `ImmSetConversionStatus` をスキップする。
    conv_mutation_allowed: bool,
    /// 現在の入力文字セット（warmup VK の選択に使用）。
    ///
    /// HanKata→F1+F3、ZenKata→F1、Hiragana→F2、ZenAlpha→F0+F4、HanAlpha→F0。
    charset: awase::engine::Charset,
}

/// `ColdWarmupSequence::run_start` の戻り値。
///
/// 即座に実行できる部分（F2 送信等）は完了済み。
/// 残りの待機はタイマー（TIMER_TSF_PROBE）で `TsfReadinessProbe::check_now` を
/// ポーリングすることで行う。
pub(crate) struct WarmupStarted {
    /// GJI 静止プローブ
    pub probe: crate::tsf::probe::TsfReadinessProbe,
    /// probe の最大待機時間 (ms, warmup_sent_ms 起点)
    pub total_max_ms: u64,
    /// プローブ完了後に NAMECHANGE 確認フェーズが必要かどうか
    /// (`eager_probe_with_settle` パスのみ `true`)
    pub needs_settle_check: bool,
    /// cold になった理由（NAMECHANGE フェーズの判断に使用）
    pub cold_reason: crate::output::ColdReason,
    /// プローブ開始前に VK_DBE_HIRAGANA pair が送信済みかどうか。
    ///
    /// ReWarmup / FreshF2 / non-eager パスで `true`。
    /// `TransmitTsf` 時にバッチへの F2 重複送信を抑制するために使用する。
    /// （バッチに F2 を追加すると WezTerm が TSF reinit を起こし先頭 VK がリテラル化する）
    pub fresh_f2_at_probe_start: bool,
}

/// TSF cold-start ウォームアップシーケンスを管理する構造体。
///
/// `Output::execute_cold_warmup` のロジックを複数のプライベートメソッドに分解し、
/// 可読性・テスト性を高める。
///
/// `run_start()` を呼ぶと即座に実行できる部分（F2 送信等）を行い [`WarmupStarted`] を返す。
/// `run()` は旧来のブロッキング API（テスト互換用）。
pub(crate) struct ColdWarmupSequence<'a> {
    output: &'a Output,
}

impl<'a> ColdWarmupSequence<'a> {
    /// 新しいシーケンスを生成する。
    pub(crate) const fn new(output: &'a Output) -> Self {
        Self { output }
    }

    /// ノンブロッキング版ウォームアップ開始。
    ///
    /// 即座に実行できる部分（F2 送信、IMM32 設定等）を行い [`WarmupStarted`] を返す。
    /// 残りの GJI 静止待ちは TIMER_TSF_PROBE + `TsfReadinessProbe::check_now` で行う。
    pub(crate) fn run_start(&self, session_expired: bool, elapsed_ms: u64) -> WarmupStarted {
        let ctx = self.preamble(session_expired, elapsed_ms);

        if session_expired {
            log::debug!(
                "[h1-warmup] cold={} session expired → fresh VK_DBE_HIRAGANA 送信 (500ms待機を強制)",
                ctx.cold_seq
            );
            self.output.send_eager_tsf_warmup(None);
        }

        let eager_ms = self.output.composition.eager_warmup_sent_ms();
        let now_ms = crate::hook::current_tick_ms();
        let eager_elapsed = if eager_ms != 0 {
            now_ms.saturating_sub(eager_ms)
        } else {
            u64::MAX
        };
        let use_eager = eager_ms != 0;

        log::debug!(
            "[h1-warmup] cold={} path={} eager_ms={eager_ms} now_ms={now_ms} elapsed={}ms",
            ctx.cold_seq,
            if use_eager { "eager" } else { "non-eager" },
            crate::output::fmt_ms(eager_elapsed),
        );

        let started = if use_eager {
            Self::run_eager_start(&ctx, eager_ms, eager_elapsed)
        } else {
            Self::run_non_eager_start(&ctx)
        };

        // FreshF2 / ReWarmup / non-eager パスで HanKata warmup (F1+F3) を送信した場合、
        // IMM が ZenKata (0x0B) を返すことがあるため conv_mode 汚染を抑制する。
        // fresh_f2_at_probe_start=true かつ HanKata かつ conv_mutation_allowed のとき実際に送信済み。
        if started.fresh_f2_at_probe_start
            && ctx.conv_mutation_allowed
            && ctx.charset == awase::engine::Charset::HankakuKatakana
        {
            self.output.conv_mode.on_hankata_warmup_sent();
        }

        started
    }

    /// 準備フェーズ: 診断ログ出力・IMM32 設定・`cold_seq` インクリメントを行い
    /// [`WarmupContext`] を返す。
    fn preamble(&self, session_expired: bool, elapsed_ms: u64) -> WarmupContext {
        if session_expired {
            log::debug!("[tsf-warmup] session expired ({elapsed_ms}ms) → F2-only先行バッチ (案A)");
        } else {
            log::debug!("[tsf-warmup] cold → F2-only先行バッチ (案A)");
        }

        // 診断ログ (get_ime_conversion_mode_raw) と IMM32 ローマ字モード設定
        // (set_ime_romaji_mode) は SendMessageTimeoutW を呼ぶため、メインスレッドで
        // 同期実行すると `with_app` 再入の原因になる。ワーカースレッドに offload する
        // async ラッパーを spawn_local で起動して退避する。
        //
        // 順序保証: 本パスの直後の SendInput は F2 (VK_DBE_HIRAGANA) のみで、
        // romaji 文字は含まない。実 romaji 送信は TIMER_TSF_PROBE 完了後の probe
        // ハンドリング経由で行われる (probe_min_ms ≥ 50ms 待機)。
        // worker-thread SendMessageTimeoutW は通常 10ms 以内に完了するため、
        // ROMAN ビット設定は実 romaji 送信より先に完了する。
        // conv_mode から ImmSetConversionStatus の目標値を取得する。
        // カタカナ系は KATAKANA/FULLSHAPE ビットを明示的に復元する必要があるため Some を返す。
        let conv_target = self.output.conv_mode.get().and_then(|m| m.imm_conv_target());
        let conv_mutation_allowed = self.output.conv_mutation_allowed.get();
        win32_async::spawn_local(async move {
            let conv_pre = crate::ime::get_ime_conversion_mode_raw_timeout_async(50).await;
            log::debug!(
                "[cold-diag] pre-send conv={} NATIVE={} ROMAN={} KATAKANA={} target={:?}",
                conv_pre.map_or_else(|| "none".to_string(), |v| format!("0x{v:08X}")),
                conv_pre.is_some_and(|v| crate::imm::cmode_has(v, crate::imm::IME_CMODE_NATIVE)),
                conv_pre.is_some_and(|v| crate::imm::cmode_has(v, crate::imm::IME_CMODE_ROMAN)),
                conv_pre.is_some_and(|v| crate::imm::cmode_has(v, crate::imm::IME_CMODE_KATAKANA)),
                conv_target.map(|v| format!("0x{v:08X}")),
            );
            if conv_mutation_allowed {
                let _ = crate::ime::set_ime_romaji_mode_with_target_async(conv_target).await;
            }
        });

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
        //     長期 idle 後は WezTerm 等で TSF 初期化がさらに遅延するため 2000ms に拡張する。
        //   PassthroughConfirmKey / ReinjectConfirmKey + long_idle:
        //     Enter/Space/Escape 後でも長期 idle 後は GJI セッションがリセットされ、
        //     500ms のバジェットでは不足する（kおのじしょう バグ）。1500ms に拡張する。
        //   その他（Enter/Space/記号等）: composition 再突入のみ → 500ms
        if cold_reason.requires_settle() && long_idle {
            log::debug!(
                "[h1-warmup] cold={cold_seq} {:?} + long idle \
                 ({}ms) → eager_settle_ms=2000ms",
                cold_reason,
                self.output.composition.idle_ms_at_last_cold()
            );
        } else if cold_reason.is_confirm_key() && long_idle {
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

        let charset = self
            .output
            .conv_mode
            .get()
            .map(|m| m.charset)
            .unwrap_or(awase::engine::Charset::Hiragana);

        WarmupContext {
            cold_seq,
            eager_settle_ms,
            probe_min_ms,
            cold_reason,
            conv_mutation_allowed,
            charset,
        }
    }

    /// non-eager: charset に応じた warmup VK×2 を送信して WarmupStarted を返す。
    fn run_non_eager_start(ctx: &WarmupContext) -> WarmupStarted {
        log::debug!(
            "[h1-warmup] cold={} non-eager: {} warmup+probe 送信 (conv_mutation={})",
            ctx.cold_seq,
            ctx.charset,
            ctx.conv_mutation_allowed,
        );
        let probe_sent_ms = if ctx.conv_mutation_allowed {
            // warmup VK ペアを 2 回送信（GJI I/O を確実に起動する）。
            // HanKata: F1+F3 × 2、ZenKata: F1 × 2、Hiragana: F2 × 2。
            send_charset_warmup_pair(ctx.charset);
            send_charset_warmup_pair(ctx.charset)
        } else {
            crate::hook::current_tick_ms()
        };
        WarmupStarted {
            probe: crate::tsf::probe::TsfReadinessProbe::new(
                probe_sent_ms,
                ctx.cold_seq,
                ctx.probe_min_ms,
            ),
            total_max_ms: ctx.eager_settle_ms,
            needs_settle_check: false,
            cold_reason: ctx.cold_reason,
            fresh_f2_at_probe_start: true,
        }
    }

    /// eager ノンブロッキック開始: パスを判定して F2 を送信し WarmupStarted を返す。
    fn run_eager_start(ctx: &WarmupContext, eager_ms: u64, eager_elapsed: u64) -> WarmupStarted {
        let remaining = ctx.eager_settle_ms.saturating_sub(eager_elapsed);
        let requires_settle = ctx.cold_reason.requires_settle();
        let kind = WarmupKind::from_context(remaining, requires_settle);

        match kind {
            WarmupKind::FreshF2 => {
                // eager_fresh_f2_then_probe: fresh warmup + probe
                let last_io = crate::tsf::observer::TSF_OBS.gji_last_io_ms.load(Relaxed);
                let gji_idle = crate::hook::current_tick_ms().saturating_sub(last_io);
                log::debug!(
                    "[h1-warmup] cold={} eager: {}ms 経過 (gji_idle={gji_idle}ms) → fresh {} start (conv_mutation={})",
                    ctx.cold_seq,
                    ctx.eager_settle_ms,
                    ctx.charset,
                    ctx.conv_mutation_allowed,
                );
                let fresh_warmup_ms = if ctx.conv_mutation_allowed {
                    send_charset_warmup_pair(ctx.charset)
                } else {
                    crate::hook::current_tick_ms()
                };
                WarmupStarted {
                    probe: crate::tsf::probe::TsfReadinessProbe::new(
                        fresh_warmup_ms,
                        ctx.cold_seq,
                        ctx.probe_min_ms,
                    ),
                    total_max_ms: ctx.eager_settle_ms,
                    needs_settle_check: false,
                    cold_reason: ctx.cold_reason,
                    fresh_f2_at_probe_start: ctx.conv_mutation_allowed,
                }
            }
            WarmupKind::ReWarmup => {
                // eager_re_warmup: fresh warmup を送信して RE_WARMUP_MS 待機
                log::debug!(
                    "[h1-warmup] cold={} eager: {}ms 経過 → 再warmup ({}) start (conv_mutation={})",
                    ctx.cold_seq,
                    ctx.eager_settle_ms,
                    ctx.charset,
                    ctx.conv_mutation_allowed,
                );
                let re_warmup_ms = if ctx.conv_mutation_allowed {
                    send_charset_warmup_pair(ctx.charset)
                } else {
                    crate::hook::current_tick_ms()
                };
                WarmupStarted {
                    probe: crate::tsf::probe::TsfReadinessProbe::new(
                        re_warmup_ms,
                        ctx.cold_seq,
                        ctx.probe_min_ms,
                    ),
                    total_max_ms: crate::tuning::RE_WARMUP_MS,
                    needs_settle_check: false,
                    cold_reason: ctx.cold_reason,
                    fresh_f2_at_probe_start: ctx.conv_mutation_allowed,
                }
            }
            WarmupKind::ProbeWithSettle => {
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
                    fresh_f2_at_probe_start: false,
                }
            }
        }
    }
}
