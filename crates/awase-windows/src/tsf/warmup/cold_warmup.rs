//! TSF cold-start ウォームアップシーケンス。
//!
//! [`ColdWarmupSequence`] は F2 送信等の即時処理を行い [`WarmupStarted`] を返す。
//! 残りの GJI 静止待ちは TIMER_TSF_PROBE + `TsfReadinessProbe::check_now` で行う。
//!
//! ## パス分岐
//!
//! ```text
//! run_start()
//!   └─ preamble()             : 診断ログ・IMM32 ローマ字モード設定・cold_seq インクリメント
//!      ├─[DIAG_COLD_SKIP_F2 || DIAG_COLD_SKIP_PROBE_WAIT]
//!      │    └─ run_experimental_start(): トグルに応じて F2 送信・probe 待機を個別にスキップ
//!      ├─ run_eager_start()  : eager warmup パス (eager_warmup_sent_ms != 0)
//!      │    ├─ FreshF2        (remaining == 0 && !requires_settle) → F2 送信 + probe
//!      │    ├─ ReWarmup       (remaining == 0 &&  requires_settle) → F2 再送 + RE_WARMUP_MS 待機
//!      │    └─ ProbeWithSettle(remaining > 0)                      → probe (NAMECHANGE 確認付き)
//!      └─ run_non_eager_start(): F2×2 送信 + WarmupStarted (GjiProbe)
//! ```
//!
//! `DIAG_COLD_SKIP_F2`/`DIAG_COLD_SKIP_PROBE_WAIT`（`tuning.rs`、`AtomicBool`、
//! トレイメニューの「実験: cold warmup」から実行中に on/off できる）のどちらかが
//! `true` の間は `run_eager_start`/`run_non_eager_start` に到達しない
//! （比較・切り戻し用に残してある、実験中）。

use std::sync::atomic::Ordering::Relaxed;

use crate::output::Output;
use crate::tuning::LONG_IDLE_MS;

use crate::tsf::send::send_vk_dbe_hiragana_pair;

/// warmup VK ペア（F2↓F2↑, ひらがな）を1回送信し、送信後の時刻を返す。
///
/// カタカナ/英数 charset への追従（F1/F0 系キー選択）は BUG-19 のロックイン事故を
/// 受けて撤去した。`DIAG_FORCE_HIRAGANA_CHARSET` 下では `charset` 引数は常に
/// Hiragana のため、常に F2 のみを送る（`docs/known-bugs.md` BUG-19 参照）。
fn send_charset_warmup_pair(_charset: awase::engine::Charset) -> u64 {
    send_vk_dbe_hiragana_pair()
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
    /// `ConvModeAuthority::AwaseOwned` のとき `true`。
    ///
    /// `false`（UserOwned/Unknown）のとき VK_DBE_HIRAGANA 送信と
    /// `ImmSetConversionStatus` をスキップする。
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

        // eager/non-eager のパス分岐は WarmupKind（FreshF2/ReWarmup/ProbeWithSettle）の
        // 予算選択に直結する（切り分けログ強化、2026-07-09）。RUST_LOG=debug で確認する運用。
        log::debug!(
            "[h1-warmup] cold={} path={} eager_ms={eager_ms} now_ms={now_ms} elapsed={}ms",
            ctx.cold_seq,
            if use_eager { "eager" } else { "non-eager" },
            crate::output::fmt_ms(eager_elapsed),
        );

        let skip_f2 = crate::tuning::DIAG_COLD_SKIP_F2.load(Relaxed);
        let skip_wait = crate::tuning::DIAG_COLD_SKIP_PROBE_WAIT.load(Relaxed);
        if skip_f2 || skip_wait {
            return Self::run_experimental_start(&ctx, skip_f2, skip_wait);
        }

        if use_eager {
            Self::run_eager_start(&ctx, eager_ms, eager_elapsed)
        } else {
            Self::run_non_eager_start(&ctx)
        }
    }

    /// `DIAG_COLD_SKIP_F2`/`DIAG_COLD_SKIP_PROBE_WAIT` 用: トレイメニューで選んだ
    /// 組み合わせに応じて、予防的な F2 warmup 送信・`TsfReadinessProbe` の待機を
    /// 独立にスキップする。
    ///
    /// `skip_f2=true` の間、F2 は送らない（romaji の VK だけを per-VK confirm
    /// ループへ渡す）。`skip_wait=true` の間、`WarmupKind`（FreshF2/ReWarmup/
    /// ProbeWithSettle）による `eager_settle_ms`/`probe_min_ms` の使い分けを
    /// 行わず、`min_ms=0`/`total_max_ms=0` の probe を返す（`gji_coro_body` の
    /// Phase 1 は次 tick で即座に解放される）。両方 `true` のとき、cold で GJI が
    /// hiragana composition を受け付けなければ1文字目が `SuspectedLiteral` に
    /// なり、`emit_recovery_actions` の `StartSacrificialWarmup` 経路（TSF mode +
    /// consecutive==0）が再確立を担う。
    fn run_experimental_start(
        ctx: &WarmupContext,
        skip_f2: bool,
        skip_wait: bool,
    ) -> WarmupStarted {
        log::debug!(
            "[h1-warmup] cold={} DIAG_COLD_SKIP: skip_f2={skip_f2} skip_wait={skip_wait} \
             (charset={} conv_mutation={})",
            ctx.cold_seq,
            ctx.charset,
            ctx.conv_mutation_allowed,
        );
        let sent_f2 = !skip_f2 && ctx.conv_mutation_allowed;
        let warmup_sent_ms = if sent_f2 {
            send_charset_warmup_pair(ctx.charset)
        } else {
            crate::hook::current_tick_ms()
        };
        let (min_ms, total_max_ms) = if skip_wait {
            (0, 0)
        } else {
            (ctx.probe_min_ms, ctx.eager_settle_ms)
        };
        WarmupStarted {
            probe: crate::tsf::probe::TsfReadinessProbe::new(warmup_sent_ms, ctx.cold_seq, min_ms),
            total_max_ms,
            needs_settle_check: false,
            cold_reason: ctx.cold_reason,
            fresh_f2_at_probe_start: sent_f2,
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
        // conv_mode から ImmSetConversionStatus の目標値を取得する。カタカナ/英数への
        // 明示的復元（KATAKANA/FULLSHAPE ビット等）は BUG-19 のロックイン事故を受けて
        // 撤去した。常に None（ROMAN ビット確保のみ）を書き戻す（`docs/known-bugs.md`
        // BUG-19 参照）。
        let conv_target: Option<u32> = None;
        let conv_mutation_allowed = self.output.conv_mutation_allowed.get();
        let should_write_conv_target = conv_mutation_allowed;
        win32_async::spawn_local(async move {
            let conv_pre = crate::ime::get_ime_conversion_mode_raw_timeout_async(50).await;
            log::debug!(
                "[cold-diag] pre-send conv={} NATIVE={} ROMAN={} KATAKANA={} target={:?} write={should_write_conv_target}",
                conv_pre.map_or_else(|| "none".to_string(), |v| format!("0x{v:08X}")),
                conv_pre.is_some_and(|v| crate::imm::cmode_has(v, crate::imm::IME_CMODE_NATIVE)),
                conv_pre.is_some_and(|v| crate::imm::cmode_has(v, crate::imm::IME_CMODE_ROMAN)),
                conv_pre.is_some_and(|v| crate::imm::cmode_has(v, crate::imm::IME_CMODE_KATAKANA)),
                conv_target.map(|v| format!("0x{v:08X}")),
            );
            if should_write_conv_target {
                let _ = crate::ime::set_ime_romaji_mode_with_target_async(conv_target).await;
            }
        });

        let cold_seq = self.output.composition.increment_cold_start_count();

        // SAFETY: Win32 GetForegroundWindow + GetClassName; returns empty string on failure.
        let win_class = unsafe { crate::ime::get_foreground_window_class() };
        // 実フォアグラウンドクラスと focus トラッカーの認識（AppKind changed ログ）の
        // 食い違いを事後診断するために必要（切り分けログ強化、2026-07-09）。
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
        // 実際に選ばれた予算（eager_settle_ms/probe_min_ms）と long_idle 判定を
        // 事後ログから直接確認できるようにする（切り分けログ強化、2026-07-09）。
        log::debug!(
            "[h1-warmup] cold={cold_seq} eager_settle_ms={eager_settle_ms}ms probe_min_ms={probe_min_ms}ms \
             reason={:?} long_idle={long_idle} idle_at_cold={}ms",
            cold_reason,
            self.output.composition.idle_ms_at_last_cold()
        );

        let charset = self.output.conv_mode.effective_charset();

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
