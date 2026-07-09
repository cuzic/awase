use super::key_injector::{make_key_input, KeyInjector, VkMarker};
use super::resolve::{ascii_to_vk, CharResolution};
use super::{fmt_ms, WarmthContext, WarmupOutcome};
use super::{Output, VkSequence};
use crate::tsf::output::kana_for_romaji_static;
use crate::tsf::output::ColdReason;
use crate::tsf::output::TSF_MARKER;
use crate::tsf::probe_bridge::OutputActiveGuard;
use crate::vk::{VK_DBE_HIRAGANA, VK_OEM_MINUS};
use awase::types::VkCode;
use windows::Win32::UI::Input::KeyboardAndMouse::INPUT;

/// TSF 送信パイプライン（transmit フェーズのみ）。
///
/// - `transmit`: VK または Unicode kana で romaji を WezTerm に送信
///
/// warm パス（`send_romaji_as_tsf` の non-cold ブランチ）と
/// `do_transmit_tsf`（タイマー FSM からの遅延送信）が使用する。
pub(crate) struct TsfSendPipeline;

impl TsfSendPipeline {
    /// VK run または Unicode kana を送信し、バックスペース数を返す。
    pub(crate) fn transmit(
        romaji: &str,
        chars: &[(VkCode, bool)],
        outcome: &WarmupOutcome,
    ) -> usize {
        // cold パスかつ eager warmup あり → unicode TSF（既存）
        // cold パスかつ eager なし → VK のまま（F2 ウォームアップ未完のため）
        // warm パス（prepend_f2_warmup=false）:
        //   used_eager_path=true → unicode TSF（B↓A↓B↑A↑ VK の「b」チラつき回避）
        //   used_eager_path=false → VK run
        //   TSF-native (WezTerm): send_romaji_as_tsf_warm が false を設定するため常に VK run →
        //     GJI コンポジション経由で候補ウィンドウが表示される。
        let unicode_kana: Option<char> = if outcome.prepend_f2_warmup {
            if outcome.used_eager_path {
                kana_for_romaji_static(romaji)
            } else {
                None
            }
        } else if outcome.used_eager_path {
            kana_for_romaji_static(romaji)
        } else {
            None
        };

        let t_send = crate::hook::current_tick_ms();
        log::debug!(
            "[tsf-transmit] cold={} romaji={:?} → {} t={}ms (prepend_f2={} eager={})",
            outcome.cold_seq,
            romaji,
            if unicode_kana.is_some() {
                "unicode"
            } else if outcome.prepend_f2_warmup {
                "vk-run+f2"
            } else {
                "vk-run"
            },
            t_send,
            outcome.prepend_f2_warmup,
            outcome.used_eager_path,
        );

        unicode_kana.map_or_else(
            || {
                if outcome.prepend_f2_warmup {
                    // VK run cold path: F2 を K+O と同一 SendInput バッチに含める。
                    // F2↓ が K↓ の直前に WezTerm へ届くため、GJI の composition context が
                    // K を受け取る前に確実に初期化される（partial literal 防止）。
                    // 例: NameChangeWait nc_fired=false 後に ko を送る際、
                    //   F2+K+O バッチ → F2↓ で GJI 初期化 → K↓ でコンポジション開始 → こ
                    Output::send_vk_runs_with_leading_f2(chars, outcome.cold_seq);
                } else {
                    Output::send_vk_runs(chars, outcome.cold_seq);
                }
                chars.len()
            },
            |kana| {
                log::debug!(
                    "[h1-run] cold={} unicode TSF: {romaji:?} → '{}' (U+{:04X})",
                    outcome.cold_seq,
                    kana,
                    kana as u32,
                );
                let mut inputs = Vec::with_capacity(4);
                Output::push_unicode_char_inputs(&mut inputs, kana, TSF_MARKER);
                let _ = crate::win32::send_input_safe(&inputs);
                1
            },
        )
    }
}

impl Output {
    /// Unicode 文字を直接送信する（`KEYEVENTF_UNICODE`）
    ///
    /// `unicode_cold_defer` フラグが立っている場合は実送信せず `unicode_cold_deferred` に蓄積する。
    /// 実際の送信処理は `KeyInjector::send_unicode_char` に委譲する。
    pub(super) fn send_unicode_char(&self, ch: char) {
        self.injector.send_unicode_char(ch);
    }

    /// Batched モード: 全文字を1回の SendInput にまとめて送信（重畳押し順）
    ///
    /// cold 時は F2 を先行送信してから GJI プローブを開始し（ノンブロッキング）、
    /// TIMER_TSF_PROBE が `ChromeProbe` フェーズを進めてローマ字を送信する。
    // cold/warm・GJI probe 有無で分岐が本質的に多い送信パス。分割は挙動変更リスクが
    // 高いため、複雑度警告のみ抑制する。
    #[expect(clippy::cognitive_complexity)]
    pub(super) fn send_romaji_batched(&self, romaji: &str) {
        let chars: VkSequence = romaji.chars().filter_map(ascii_to_vk).collect();
        if chars.is_empty() {
            return;
        }

        // KeyInput shadow routing: FSM state track のためだけ（actual send は既存ロジックが担う）。
        {
            let resp = self.gji_on_event(crate::tsf::gji_fsm::GjiEvent::KeyInput(
                crate::tsf::gji_fsm::PendingInput::new(romaji),
            ));
            self.warmup_coord.push_key_response(resp);
        }

        {
            let ime_suffix = if self.warmup_coord.needs_f2_probe() {
                let now_ms = crate::hook::current_tick_ms();
                let last_write_ms = crate::tsf::observer::gji_last_write_ms();
                let ago = if last_write_ms == 0 {
                    "never".to_string()
                } else {
                    format!("{}ms ago", now_ms.saturating_sub(last_write_ms))
                };
                format!("ime=GJI last_write={ago}")
            } else {
                "ime=MsIme".to_string()
            };
            log::info!("[key-output] KeyInput(batched): romaji={romaji:?} {ime_suffix}");
        }

        let WarmthContext {
            warm,
            elapsed,
            session_expired,
            prepend_f2_warmup,
        } = self.assess_warmth();
        log::debug!(
            "[vk-send] romaji={romaji:?} warm={warm} elapsed={}ms session_expired={session_expired} prepend_f2_warmup={prepend_f2_warmup}",
            fmt_ms(elapsed)
        );

        if prepend_f2_warmup {
            if self.defer_if_probe_in_flight(romaji) {
                return;
            }

            if session_expired {
                log::debug!("[vk-warmup] session expired ({elapsed}ms) → F2-only先行バッチ (案A)");
            } else {
                log::debug!("[vk-warmup] cold → F2-only先行バッチ (案A)");
            }

            let cold_seq = self.composition.increment_cold_start_count();
            let win_class = unsafe { crate::ime::get_foreground_window_class() };
            log::debug!("[h1-window] cold={cold_seq} class={win_class}");

            // F2NonTsf: 物理 F2 がすでに Chrome の composition context を初期化済み。
            // プログラム的な F2 送信（SendMessageTimeout + SendInput）をスキップし、
            // 物理 F2 の時刻を probe 基準点として使うことで Chrome が 3 回 F2 を受け取る
            // バグ（「かんりのつごう → kaんりのつごう」）を防ぐ。
            // ただし以下の場合は F2NonTsf を無効化して programmatic F2 を再送する:
            // - F2 から F2_STALE_MS 以上経過した場合: context が失効している可能性
            // - long_idle=true の場合: 物理 F2 単体では Chrome の composition context
            //   を再初期化できない（実測: ~518ms 必要なのに probe が 499ms で発火して
            //   最初のキーが literal になる）。programmatic F2 (SendMessageTimeout +
            //   SendInput) を必ず送って確実に初期化する。
            // - f2_gji_long_idle=true の場合: GJI が長期 idle のとき物理 F2 だけでは
            //   TSF context が起動せず GJI I/O が来ない。probe の "GJI I/O なし →
            //   min_ms 後に即解放" パスが早期発火して最初のキーがリテラルになる
            //   （例: たんなる → tあんなる）。long_idle 同様 programmatic F2 を送る。
            let cold_reason = self.composition.last_cold_reason();
            let cold_marked_ms = self.composition.cold_marked_ms();
            let f2_stale = cold_reason == ColdReason::F2NonTsf
                && cold_marked_ms != 0
                && crate::hook::current_tick_ms().saturating_sub(cold_marked_ms)
                    > crate::tuning::F2_STALE_MS;
            // ノンブロッキング Chrome プローブを開始。
            // 長期 idle 後の cold start では GJI が reinit に要する時間が長いため
            // min/max を延長する（120ms では GJI が settle する前に timeout して literal
            // 出力される回帰を抑制）。
            let long_idle =
                self.composition.idle_ms_at_last_cold() > crate::tuning::CHROME_LONG_IDLE_MS;
            let skip_f2_send = cold_reason == ColdReason::F2NonTsf && !f2_stale && !long_idle;
            // 物理 F2 (skip_f2_send=true) かつ GJI が長期 idle の場合:
            // GJI I/O が来ないのは正常状態だからではなく Chrome TSF context が未起動のため。
            // skip_f2_send を false に上書きして programmatic F2 を強制送信する。
            let f2_gji_long_idle = skip_f2_send && {
                let gji_last_io = crate::tsf::observer::gji_last_io_ms();
                crate::hook::current_tick_ms().saturating_sub(gji_last_io)
                    > crate::tuning::CHROME_LONG_IDLE_MS
            };
            let skip_f2_send = skip_f2_send && !f2_gji_long_idle;
            let f2_sent_ms = if skip_f2_send && cold_marked_ms != 0 {
                cold_marked_ms
            } else {
                crate::hook::current_tick_ms()
            };
            let (probe_min_ms, probe_max_ms) = if long_idle || f2_gji_long_idle {
                (
                    crate::tuning::CHROME_PROBE_LONG_IDLE_MIN_MS,
                    crate::tuning::CHROME_PROBE_LONG_IDLE_MAX_MS,
                )
            } else {
                (
                    crate::tuning::CHROME_PROBE_MIN_MS,
                    crate::tuning::CHROME_PROBE_MAX_MS,
                )
            };
            if f2_stale {
                let elapsed = crate::hook::current_tick_ms().saturating_sub(cold_marked_ms);
                log::debug!(
                    "[h1-probe] cold={cold_seq} F2NonTsf stale ({elapsed}ms > F2_STALE_MS={}) \
                     → programmatic F2 を再送",
                    crate::tuning::F2_STALE_MS
                );
            }
            log::debug!(
                "[h1-probe] cold={cold_seq} long_idle={long_idle} f2_gji_long_idle={f2_gji_long_idle} idle_at_cold={}ms min={probe_min_ms}ms max={probe_max_ms}ms skip_f2={skip_f2_send} f2_stale={f2_stale}",
                self.composition.idle_ms_at_last_cold(),
            );

            // SendMessageTimeoutW 系の同期呼び出し (set_ime_romaji_mode + send_f2_via_sendmessage)
            // を with_app の外で実行するため、async タスクへオフロードする。
            // OutputActiveGuard を先に取得しておくことで、await 中に走るフックコールバックが
            // キーを INPUT_DEFER に退避し、cold start シーケンスと race しないようにする。
            //
            // H-4-b: ChromeProbe を spawn_local より前に同期生成してインストールする。
            // これにより async クロージャ内の with_app() → Runtime 逆参照が不要になり、
            // Runtime → Platform → Output → グローバル → Runtime の循環依存を断つ。
            // guard は ChromeProbe に move され、probe 完了まで OUTPUT_GATE を保持する。
            // WindowsPlatform::send_keys が pending_tsf_timer() で TIMER_TSF_PROBE を起動する
            // （sync パスと同一経路）。RuntimeRequest::StartTsfProbe は belt-and-suspenders として積む。
            let guard = OutputActiveGuard::begin();
            let probe =
                crate::tsf::probe::TsfReadinessProbe::new(f2_sent_ms, cold_seq, probe_min_ms);
            // is_long_cold: GJI が本当に CHROME_LONG_IDLE_MS を超えて寝ていたか。
            // 確定キーや IME OFF→ON 再有効化直後の一瞬だけの cold (Short/Medium) では
            // false になり、ChromeProbe は VK_A probe + Chrome reinit のフルコースを
            // 省略して軽量パス（inline LiteralDetect のみ）を使う（過剰な cold-start
            // 発火の抑制、docs/known-bugs.md BUG-21）。
            //
            // 注: この判定は「awase 自身が最後に送信してからの経過時間」
            // （`idle_ms_at_last_cold` = `ms_since_last_send()`）という自己参照タイマーに
            // 基づく。実際に GJI プロセスが動いているかどうかの直接観測
            // （`gji_last_io_ms` / `gji_idle_ms()`, `tsf/gji_monitor.rs` の
            // `GetProcessIoCounters` 実IO監視）とは独立している。
            let self_timer_is_long_cold = long_idle || f2_gji_long_idle;
            let real_gji_idle_ms = crate::tsf::observer::gji_idle_ms();
            let is_long_cold =
                self_timer_is_long_cold && !crate::tuning::DIAG_SKIP_PROACTIVE_SACRIFICIAL_WARMUP;
            if self_timer_is_long_cold {
                log::info!(
                    "[h1-probe-diag] cold={cold_seq} self_timer_is_long_cold=true \
                     real_gji_idle_ms={real_gji_idle_ms} \
                     DIAG_SKIP_PROACTIVE_SACRIFICIAL_WARMUP={} → is_long_cold={is_long_cold}",
                    crate::tuning::DIAG_SKIP_PROACTIVE_SACRIFICIAL_WARMUP,
                );
            }
            self.install_pending_tsf(Box::new(
                crate::tsf::warmup::chrome_probe::ChromeProbe::new(
                    romaji,
                    cold_seq,
                    probe,
                    probe_max_ms,
                    guard,
                    is_long_cold,
                ),
            ));
            self.runtime_outbox
                .borrow_mut()
                .push(crate::runtime::outbox::RuntimeRequest::StartTsfProbe);

            win32_async::spawn_local(async move {
                // 診断: pre-send IME conversion mode（旧 [cold-diag] log）
                let conv_pre = crate::ime::get_ime_conversion_mode_raw_timeout_async(50).await;
                log::debug!(
                    "[cold-diag] pre-send conv={} NATIVE={} ROMAN={} KATAKANA={}",
                    conv_pre.map_or_else(|| "none".to_string(), |v| format!("0x{v:08X}")),
                    conv_pre
                        .is_some_and(|v| crate::imm::cmode_has(v, crate::imm::IME_CMODE_NATIVE)),
                    conv_pre.is_some_and(|v| crate::imm::cmode_has(v, crate::imm::IME_CMODE_ROMAN)),
                    conv_pre
                        .is_some_and(|v| crate::imm::cmode_has(v, crate::imm::IME_CMODE_KATAKANA)),
                );

                if skip_f2_send {
                    // 物理 F2 が Chrome の composition context を既に初期化済みのため
                    // プログラム的な F2 送信をスキップする。probe は cold_marked_ms 基準で待機。
                    log::debug!("[h1-run] cold={cold_seq} F2NonTsf: skip programmatic F2 (physical F2 at f2_sent_ms={f2_sent_ms})");
                } else {
                    // IMC_SETCONVERSIONMODE を ROMAN に揃えてから SendInput でローマ字を送ることで
                    // カナ出力化けを防ぐ。await でワーカースレッドに完全委譲しているので順序は保たれる。
                    let _ = crate::ime::set_ime_romaji_mode_async().await;

                    log::debug!("[h1-run] cold={cold_seq} F2 via SendMessageTimeout");
                    let f2_ok = crate::ime::send_f2_via_sendmessage_async().await;
                    log::debug!("[h1-run] cold={cold_seq} F2 SendMessageTimeout delivered={f2_ok}");

                    // SendMessageTimeout はウィンドウの wndproc に直接届くが TSF のキーストローク
                    // マネージャーを経由しないため、Chrome の composition context が初期化されない。
                    // SendInput 経由でも F2 を送り TSF に composition context を初期化させる。
                    // INJECTED_MARKER 付きなので awase 自身のフックは即座に素通しする（mark_cold 不要）。
                    let f2_via_sendinput = [
                        make_key_input(VK_DBE_HIRAGANA, false),
                        make_key_input(VK_DBE_HIRAGANA, true),
                    ];
                    let _ = crate::win32::send_input_safe(&f2_via_sendinput);
                    log::debug!(
                        "[h1-run] cold={cold_seq} F2 via SendInput (TSF composition context init)"
                    );
                }
            });

            return;
        }

        // warm パス: 即座にバッチ送信
        Self::send_romaji_batch_immediate(romaji, &chars);
    }

    /// ローマ字を即座にバッチ送信する（重畳順・VK ラン分割）。
    /// `KeyInjector::send_romaji_batch_immediate` に委譲する。
    pub(crate) fn send_romaji_batch_immediate(romaji: &str, chars: &[(VkCode, bool)]) {
        KeyInjector::send_romaji_batch_immediate(romaji, chars);
    }

    /// Unicode モード: ローマ字→ひらがなに変換して Unicode 文字として直接送信
    ///
    /// IME を経由せず、ひらがなを直接テキストフィールドに挿入する。
    /// 変換テーブルにないローマ字は PerKey モードでフォールバック送信する。
    /// 実際の変換・送信処理は `KeyInjector::send_romaji_as_unicode` に委譲する。
    pub(super) fn send_romaji_as_unicode(&self, romaji: &str) {
        self.injector.send_romaji_as_unicode(romaji);
    }

    /// VK run 分割送信: 同一 VK 連続境界でバッチを分割して IME のオートリピート誤検出を回避する。
    /// `KeyInjector::send_vk_runs` に委譲する。
    pub(super) fn send_vk_runs(chars: &[(VkCode, bool)], cold_seq: u32) {
        KeyInjector::send_vk_runs(chars, cold_seq);
    }

    /// VK run 分割送信（F2 leading）: F2 を先頭に付加して送信する。
    /// `KeyInjector::send_vk_runs_with_leading_f2` に委譲する。
    pub(super) fn send_vk_runs_with_leading_f2(chars: &[(VkCode, bool)], cold_seq: u32) {
        KeyInjector::send_vk_runs_with_leading_f2(chars, cold_seq);
    }

    /// VK run 分割送信（カタカナ warmup 選択）: hint に応じた先頭ウォームアップ VK を使う。
    /// `KeyInjector::send_vk_runs_with_leading_warmup` に委譲する。
    pub(super) fn send_vk_runs_with_leading_warmup(
        chars: &[(VkCode, bool)],
        cold_seq: u32,
        charset: crate::state::Charset,
    ) {
        KeyInjector::send_vk_runs_with_leading_warmup(chars, cold_seq, charset);
    }

    pub(super) fn send_romaji_as_tsf(&self, romaji: &str) {
        let chars: VkSequence = romaji.chars().filter_map(ascii_to_vk).collect();
        if chars.is_empty() {
            return;
        }

        // KeyInput shadow routing: FSM state track のためだけ（actual send は既存ロジックが担う）。
        // Response のタイマー操作（LongIdle リセット）は Platform の send_keys が dispatch する。
        {
            let resp = self.gji_on_event(crate::tsf::gji_fsm::GjiEvent::KeyInput(
                crate::tsf::gji_fsm::PendingInput::new(romaji),
            ));
            self.warmup_coord.push_key_response(resp);
        }

        {
            let ime_suffix = if self.warmup_coord.needs_f2_probe() {
                let now_ms = crate::hook::current_tick_ms();
                let last_write_ms = crate::tsf::observer::gji_last_write_ms();
                let ago = if last_write_ms == 0 {
                    "never".to_string()
                } else {
                    format!("{}ms ago", now_ms.saturating_sub(last_write_ms))
                };
                format!("ime=GJI last_write={ago}")
            } else {
                "ime=MsIme".to_string()
            };
            log::info!("[key-output] KeyInput(tsf): romaji={romaji:?} {ime_suffix}");
        }

        let WarmthContext {
            warm,
            elapsed,
            session_expired,
            prepend_f2_warmup,
        } = self.assess_warmth();
        // 常に VK path で開始する（unicode は GJI コンポジションをバイパスして "nお" race を
        // 起こすため）。true になる生きた経路は send_romaji_as_tsf_warm 内の
        // PendingGjiConfirm オーバーライドのみ。
        // 旧条件 `!is_tsf_mode() && eager_warmup_sent_ms() != 0` は恒偽だった
        // （eager の書き手は can_warmup() = ime_on && is_tsf_mode ガード内のみで、
        // 非 TSF epoch では常に 0。2026-07-06 到達不能パス監査 B1）。
        let used_eager_path = false;

        log::debug!(
            "[tsf-send] warm={warm} elapsed={}ms session_expired={session_expired} prepend_f2_warmup={prepend_f2_warmup}",
            fmt_ms(elapsed)
        );

        if prepend_f2_warmup {
            if self.defer_if_probe_in_flight(romaji) {
                return;
            }

            // ノンブロッキング warmup を開始して pending_tsf に保留
            let started = crate::tsf::warmup::cold_warmup::ColdWarmupSequence::new(self)
                .run_start(session_expired, elapsed);
            let cold_seq = started.probe.cold_seq;
            self.gji_begin_probe_guard();
            let probe_params = self.gji_current_probe_params();
            let coro = Box::new(crate::tsf::warmup::gji_warmup_coro::GjiWarmupCoro::new(
                romaji,
                cold_seq,
                started.probe,
                started.total_max_ms,
                started.needs_settle_check,
                started.cold_reason,
                prepend_f2_warmup,
                used_eager_path,
                probe_params.ncwait_budget_ms,
                probe_params.forces_prepend_f2,
                probe_params.is_long_cold,
                started.fresh_f2_at_probe_start,
                self.composition.consecutive_count(),
            ));
            self.install_pending_tsf(coro);
            // WindowsPlatform::send_keys が TIMER_TSF_PROBE をセットする
            return;
        }

        // MS-IME confirm-then-transmit ゲート（BUG-13）:
        // MsImeStrategy は needs_f2_probe()=false のため上の GJI probe 分岐に入らず、
        // IME ON 遷移直後（OS 準備に実測 ~130-300ms）でも即送信して先頭 VK がリテラル化
        // していた（「を」→「wお」）。ImeModeFsm の NATIVE 確認が取れるまで defer する。
        if self.ms_ime_gate_defer(romaji) {
            return;
        }

        // warm パス: 即座に送信
        self.send_romaji_as_tsf_warm(romaji, &chars, used_eager_path);
    }

    /// MS-IME confirm-then-transmit ゲート（BUG-13）。defer した場合 `true` を返す。
    ///
    /// 発動条件: MS-IME 戦略（GJI probe 非対象）+ TSF mode + `ImeModeFsm` が
    /// NATIVE 未確認 + give-up latch なし。発動時は romaji を `MsImeReadyCoro` に
    /// 預けて IMC 確認ポーリングを開始する。probe 進行中の後続キーは順序維持のため
    /// 無条件で deferred キューに積む。
    fn ms_ime_gate_defer(&self, romaji: &str) -> bool {
        // GJI 戦略時は F2 probe 機構（prepend_f2_warmup 分岐）が cold-start を担う。
        if self.warmup_coord.needs_f2_probe() {
            return false;
        }
        if !self.is_tsf_mode() {
            return false;
        }
        // 既に probe/coro 進行中 → 確認状態に関わらず defer（送信順序の維持）。
        if self.defer_if_probe_in_flight(romaji) {
            return true;
        }
        if self.ms_ime_gate_give_up.get() {
            return false;
        }
        {
            let fsm = self.ime_mode_fsm.borrow();
            if fsm.is_native_ready() {
                return false;
            }
            let cold_seq = self.composition.cold_start_count();
            log::info!(
                "[msime-ready] cold={cold_seq} IME mode 未確認 (state={:?} confirmed={}) → \
                 {romaji:?} を defer して IMC 確認待ち",
                fsm.state(),
                fsm.is_confirmed(),
            );
        }
        let cold_seq = self.composition.cold_start_count();
        let deadline_ms = crate::hook::current_tick_ms() + crate::tuning::MS_IME_READY_CONFIRM_MS;
        self.start_ms_ime_ready_poll(cold_seq, deadline_ms);
        let coro = Box::new(crate::tsf::warmup::ms_ime_ready_coro::MsImeReadyCoro::new(
            romaji,
            cold_seq,
            deadline_ms,
        ));
        self.install_pending_tsf(coro);
        // WindowsPlatform::send_keys が pending_tsf_timer() で TIMER_TSF_PROBE を起動する
        true
    }

    fn send_romaji_as_tsf_warm(&self, romaji: &str, chars: &VkSequence, used_eager_path: bool) {
        let t_warm = crate::hook::current_tick_ms();
        let cold_seq = self.composition.cold_start_count();

        // PendingGjiConfirm: unicode 送信後 GJI がまだ I/O 応答していない状態。
        // この間は VK sequential を送っても GJI composition が準備できておらず先頭 VK が
        // リテラル化する（例: こ(unicode)+れ(VK) → こrえ）。
        // GJI が応答するまで次のキーも unicode で送ることで race を回避する。
        let in_post_unicode_pending = {
            let last_unicode_ms = self.composition.last_unicode_transmit_ms();
            last_unicode_ms != 0 && crate::tsf::observer::gji_last_io_ms() <= last_unicode_ms
        };
        let used_eager_path = if in_post_unicode_pending {
            log::debug!(
                "[tsf-warm-start] cold={cold_seq} PendingGjiConfirm: GJI 未応答 → romaji={romaji:?} を unicode で強制送信"
            );
            true
        } else {
            used_eager_path
        };

        log::debug!("[tsf-warm-start] cold={cold_seq} romaji={romaji:?} t={t_warm}ms");
        let outcome = WarmupOutcome {
            prepend_f2_warmup: false,
            used_eager_path,
            cold_seq,
        };

        {
            // 診断ログ: IMC_GETCONVERSIONMODE は SendMessageTimeoutW を呼ぶため、
            // with_app 再入を避けるため async タスクへオフロードする (Step 3)。
            // ログ出力タイミングが数 ms 遅れるが診断用途のため許容。
            let last_io = crate::tsf::observer::gji_last_io_ms();
            let gji_idle = crate::hook::current_tick_ms().saturating_sub(last_io);
            let romaji_owned: String = romaji.to_string();
            let chars_len = chars.len();
            win32_async::spawn_local(async move {
                let conv = crate::ime::get_ime_conversion_mode_raw_timeout_async(10).await;
                log::debug!(
                    "[h1-send] cold={cold_seq} romaji={romaji_owned:?} chars={chars_len} gji_idle={gji_idle}ms \
                     conv={} ROMAN={} NATIVE={}",
                    conv.map_or_else(|| "none".to_string(), |v| format!("0x{v:08X}")),
                    conv.is_some_and(|v| crate::imm::cmode_has(v, crate::imm::IME_CMODE_ROMAN)),
                    conv.is_some_and(|v| crate::imm::cmode_has(v, crate::imm::IME_CMODE_NATIVE)),
                );
            });
        }

        let detector = crate::tsf::probe::LiteralDetector::new();
        let ze_bs_count = TsfSendPipeline::transmit(romaji, chars, &outcome);

        // cold-start probe 機構を持つ IME（GJI 等）が LONG_IDLE_MS 以上静止している場合は
        // LiteralDetector が常にタイムアウト → SuspectedLiteral の false positive になる。
        // 長期静止時は composition が TSF で正常に処理されたと見なして LiteralDetect をスキップ。
        let probe_long_idle = crate::hook::current_tick_ms()
            .saturating_sub(crate::tsf::observer::gji_last_io_ms())
            >= crate::tuning::LONG_IDLE_MS;
        if self.tsf_gate.state() == crate::tsf::TsfGateState::Probing
            && crate::tsf::observer::gji_is_active_ime()
            && !probe_long_idle
            && !self.is_tsf_mode()
        {
            // detector と guard は LiteralDetectFsm::new が内部生成するため渡さない。
            // ze_bs_count は実際の値を渡す。
            let _ = (detector,);
            self.install_pending_tsf(Box::new(
                crate::tsf::warmup::literal_detect_fsm::LiteralDetectFsm::new(
                    cold_seq,
                    romaji.to_owned(),
                    crate::tsf::warmup::probe_fsm::ProbeObservations {
                        nc_fired: false,
                        gji_resumed: false,
                    },
                    ze_bs_count,
                    crate::tuning::RAW_TSF_LITERAL_DETECT_MS,
                    self.composition.consecutive_count(),
                ),
            ));
        } else {
            // detector と ze_bs_count は Probing+GJI 健全パスでのみ使う。
            // 他パスでは warm マーク済みで LiteralDetect 不要。
            let _ = (detector, ze_bs_count);
        }
    }

    /// 文字を TSF Sequential VK キーストロークとして送信する（WezTerm TSF モード用）
    ///
    /// かな文字はローマ字に逆変換してから `send_romaji_as_tsf` で送信する。
    /// 記号は symbol_to_vk テーブルで直接 VK コードに変換する。
    /// マッチしない場合は Unicode 直接出力にフォールバックする。
    pub(super) fn send_char_as_tsf(&self, ch: char) {
        match self.injector.resolve_char(ch) {
            CharResolution::Romaji(romaji) => {
                log::debug!("    send_char_as_tsf: '{ch}' → romaji \"{romaji}\"");
                self.send_romaji_as_tsf(romaji);
            }
            CharResolution::Vk(vk, needs_shift) => {
                log::debug!("    send_char_as_tsf: '{ch}' → VK 0x{vk:02X} shift={needs_shift}");
                // probe 進行中は VK を後回しにして romaji との送信順序を保証する。
                // 例: ば(probe中) + ー(VK0xBD) の場合、先に ba VKs を送ってから ー を送る。
                // probe なしで直接送ると「F2 → ー → ba」→「ーば」の順序逆転が起きる。
                if self.defer_vk_if_probe_in_flight(vk, needs_shift) {
                    log::debug!("    send_char_as_tsf: VK 0x{vk:02X} deferred (probe in flight)");
                    return;
                }
                Self::send_vk_pair(vk, needs_shift, VkMarker::Tsf);
                // VK_OEM_MINUS (0xBD, no-shift) = '-' は GJI ローマ字モードで「ー」として
                // composition に取り込まれる（composition context はリセットされない）。
                // これらは warm 状態を維持し、次の romaji を warmup sleep なしで即送信する。
                // その他の記号（句読点など）は composition を commit する可能性があるため cold にマーク。
                let keeps_composition = vk == VK_OEM_MINUS && !needs_shift;
                if keeps_composition {
                    log::debug!("    send_char_as_tsf: VK 0x{vk:02X} は composition 継続 (ー系) → warm 維持");
                } else {
                    self.mark_composition_cold(ColdReason::SymbolVkSent);
                    self.warmup_coord.mark_composition_reset();
                    self.send_eager_tsf_warmup(None);
                }
            }
            CharResolution::Unicode(ch) => {
                log::debug!(
                    "    send_char_as_tsf: '{ch}' (U+{:04X}) → fallback Unicode",
                    ch as u32
                );
                self.send_unicode_char(ch);
            }
        }
    }

    /// 文字を VK キーストロークとして送信する（Chrome モード用）
    ///
    /// かな文字はローマ字に逆変換してからキーストロークとして送信する。
    /// ASCII 記号は対応する VK コードで直接送信する。
    /// いずれにもマッチしない場合は Unicode 直接出力にフォールバックする。
    /// 文字を Chrome モード用に送信する。
    ///
    /// 1. かな → ローマ字 VK（IME 経由で変換）
    /// 2. 記号 → マッピングテーブルの VK コード（IME が全角変換）
    /// 3. フォールバック → Unicode 直接出力
    pub(super) fn send_char_as_vk(&self, ch: char) {
        match self.injector.resolve_char(ch) {
            CharResolution::Romaji(romaji) => {
                log::debug!("    send_char_as_vk: '{ch}' → romaji \"{romaji}\"");
                // Batched (1回の SendInput) を使うことで、後続キー（Enter reinject 等）との
                // 競合を防ぐ。per_key では K↓K↑ と A↓A↑ が別 SendInput になり、
                // 間に Enter が割り込むと "kあ" のような出力破壊が起きる。
                self.send_romaji_batched(romaji);
            }
            CharResolution::Vk(vk, needs_shift) => {
                log::debug!("    send_char_as_vk: '{ch}' → VK 0x{vk:02X} shift={needs_shift}");
                // probe 進行中は VK を後回しにして romaji との送信順序を保証する。
                // 例: ば(ChromeProbe中) + ー(VK0xBD) の場合、先に ba VKs を送ってから ー を送る。
                if self.defer_vk_if_probe_in_flight(vk, needs_shift) {
                    log::debug!("    send_char_as_vk: VK 0x{vk:02X} deferred (probe in flight)");
                    return;
                }
                Self::send_vk_pair(vk, needs_shift, VkMarker::Injected);
            }
            CharResolution::Unicode(ch) => {
                log::debug!(
                    "    send_char_as_vk: '{ch}' (U+{:04X}) → fallback Unicode",
                    ch as u32
                );
                self.send_unicode_char(ch);
            }
        }
    }

    /// probe 完了後に deferred_vks を romaji の直後に送出する。
    /// `KeyInjector::send_deferred_probe_vks_from` に委譲する。
    pub(crate) fn send_deferred_probe_vks_from(vks: &[(VkCode, bool)], marker: VkMarker) {
        KeyInjector::send_deferred_probe_vks_from(vks, marker);
    }

    /// VK の DOWN+UP ペアを（オプション shift 付きで）1回の SendInput で送信する。
    /// `KeyInjector::send_vk_pair` に委譲する。
    fn send_vk_pair(vk: VkCode, needs_shift: bool, marker: VkMarker) {
        KeyInjector::send_vk_pair(vk, needs_shift, marker);
    }

    /// `ch` を UTF-16 エンコードし、down/up ペアを `inputs` に追加する。
    /// `KeyInjector::push_unicode_char_inputs` に委譲する。
    fn push_unicode_char_inputs(inputs: &mut Vec<INPUT>, ch: char, marker: usize) {
        KeyInjector::push_unicode_char_inputs(inputs, ch, marker);
    }
}
