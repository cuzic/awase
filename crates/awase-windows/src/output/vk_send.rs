#![allow(unsafe_code)] // Win32 API 呼び出しに unsafe が必須(lib.rsのクレート全体allowから個別移管、Task #9)
use super::key_injector::{KeyInjector, VkMarker};
use super::resolve::{ascii_to_vk, CharResolution};
use super::{fmt_ms, WarmthContext, WarmupOutcome};
use super::{Output, VkSequence};
use crate::state::event_origin::Generation;
use crate::tsf::output::kana_for_romaji_static;
use crate::tsf::output::ColdReason;
use crate::tsf::output::TSF_MARKER;
use crate::tsf::probe_bridge::OutputActiveGuard;
use crate::tsf::warmup::probe_fsm::{DeferredOrigin, TransmitTarget};
use awase::types::VkCode;
use windows::Win32::UI::Input::KeyboardAndMouse::INPUT;

/// ADR-123 変更A+C 決定4-2: `send_romaji_batched`/`send_romaji_as_tsf` の
/// 新設 `defer_if_probe_in_flight` gate を適用するかどうか。
/// 既存の TSF gate（`gate_is_bypass()`/`TsfGate::Bypass`、composition context
/// の bypass 状態）とは無関係の別概念のため命名を分けている。`Exempt` は
/// `send_romaji_batched_bypass_gate`/`send_romaji_as_tsf_bypass_gate`
/// （raw recovery 回収再送・ADR-101 決定3 retry 専用）だけが使う。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeferGate {
    Enforced,
    Exempt,
}

impl DeferGate {
    /// `Enforced`（通常のユーザー入力）は `UserInput`、`Exempt`（raw recovery
    /// 回収再送・ADR-101 決定3 retry）は `RecoveryResend`。万一 `has_pending_tsf()`
    /// によって defer された場合でも、discard 経路の origin 別内訳
    /// （ADR-123 変更B）が正しく分類できるようにする。
    const fn deferred_origin(self) -> DeferredOrigin {
        match self {
            Self::Enforced => DeferredOrigin::UserInput,
            Self::Exempt => DeferredOrigin::RecoveryResend,
        }
    }
}

impl Output {
    /// `gate` に応じて `defer_if_probe_in_flight`（全条件）または
    /// `defer_if_probe_in_flight_recovery_exempt`（`raw_recovery_owns_deferred()`
    /// を見ない版）のどちらを使うかを切り替える。`has_pending_tsf()` による
    /// defer は `Exempt` でも従来どおり適用される——除外されるのは
    /// `raw_recovery_owns_deferred()` の項だけ（理由は
    /// `send_romaji_batched_bypass_gate` の doc コメント参照）。
    fn defer_respecting_gate(&self, romaji: &str, gate: DeferGate) -> bool {
        let origin = gate.deferred_origin();
        match gate {
            DeferGate::Enforced => self.defer_if_probe_in_flight(romaji, origin),
            DeferGate::Exempt => self.defer_if_probe_in_flight_recovery_exempt(romaji, origin),
        }
    }

    /// ADR-123 変更A+C 決定4-3（drain-before-send）: `has_pending_tsf()`も
    /// `raw_recovery_owns_deferred()`も偽で、`pending_deferred`が非空
    /// 「なだけ」の場合、新しいモーラを追加でキューに積むのではなく、
    /// 先にキューをflushしてから通常送信に進む。**`assess_warmth()`より
    /// 前に呼ぶこと**——後に置くとwarm/cold判定がflush前の状態で下され、
    /// 直後のprobeが汚染された`last_send_ms`/write_deltaを自分の証拠として
    /// 読みうる（round3指摘）。
    ///
    /// ADR-128: ADR-123 決定4-3 の「新しいモーラ」は通常のユーザー入力
    /// （`DeferGate::Enforced`）だけを指す。raw recovery 回収再送・GJI reinit
    /// retry（`DeferGate::Exempt`）でここを通すと、再送自身より前に sibling
    /// mora を drain して順序反転と per-VK confirm 証拠汚染を起こすため、
    /// `ms_ime_gate_defer`/`defer_respecting_gate` と同じく呼び出し元の `gate`
    /// をそのまま引き継いで判定する。
    ///
    /// **`raw_recovery_owns_deferred()`は`gate`に関わらず常にチェックする
    /// （4-2の`defer_respecting_gate`とは非対称、Opus敵対的レビュー
    /// round4-3指摘）。** defer方向（自分の送信を止めるか）では
    /// 「無関係な別recoveryか自分自身か」の区別が自己defer回避に必要
    /// だったが、drain方向（他人が所有するキューを解放してよいか）では
    /// この区別は無意味かつ危険——`raw_recovery_owns_deferred()`が
    /// trueである以上、それが自分の状態由来であることはない
    /// （`flush_raw_tsf_literal_romaji`到達時点で自分の`RAW_TSF_LITERAL`は
    /// 既にswap/take済み・`pending_gji_reinit`は非give-up経路では未予約、
    /// `resend_gji_reinit_retry_romaji`到達時点では自分の`pending_gji_reinit`
    /// は`take_gji_reinit_completion`で既にtake済み）。つまりtrueならほぼ
    /// 確実に無関係な別recoveryが所有中であり、`finish_probe_stage`が
    /// 守るINV-Fと同じ理由でここも手を出してはならない。
    fn drain_pending_deferred_before_send_if_queue_only(&self, gate: DeferGate) {
        if gate != DeferGate::Enforced {
            return;
        }
        if self.is_probe_or_recovery_blocking(true) || self.pending_deferred_len() == 0 {
            return;
        }
        let n = self.flush_pending_deferred_vks();
        if n > 0 {
            log::debug!(
                "[pending-deferred] drain-before-send: 新規モーラの前に取り残し {n} VK(s) をflush"
            );
            // `JournalEntry` への変換は platform.rs 側で行う（output/tsf は
            // `crate::journal` を直接参照しない、`Output::
            // pending_drain_before_send_flush` の doc 参照）。
            self.pending_drain_before_send_flush.set(n);
        }
    }
}

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
        // warm パス:
        //   used_eager_path=true → unicode TSF（B↓A↓B↑A↑ VK の「b」チラつき回避）
        //   used_eager_path=false → VK run
        //   TSF-native (WezTerm): send_romaji_as_tsf_warm が false を設定するため常に VK run →
        //     GJI コンポジション経由で候補ウィンドウが表示される。
        let unicode_kana: Option<char> = if outcome.used_eager_path {
            kana_for_romaji_static(romaji)
        } else {
            None
        };

        let t_send = crate::hook::current_tick_ms();
        log::debug!(
            "[tsf-transmit] cold={} romaji={:?} → {} t={}ms (eager={})",
            outcome.cold_seq.value(),
            romaji,
            if unicode_kana.is_some() {
                "unicode"
            } else {
                "vk-run"
            },
            t_send,
            outcome.used_eager_path,
        );

        unicode_kana.map_or_else(
            || {
                Output::send_vk_runs(chars, outcome.cold_seq);
                chars.len()
            },
            |kana| {
                log::debug!(
                    "[h1-run] cold={} unicode TSF: {romaji:?} → '{}' (U+{:04X})",
                    outcome.cold_seq.value(),
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
    /// cold 時は GJI プローブを開始し（ノンブロッキング）、TIMER_TSF_PROBE が
    /// `ChromeProbe` フェーズを進めてローマ字を送信する。
    pub(super) fn send_romaji_batched(&self, romaji: &str) {
        self.send_romaji_batched_gated(romaji, DeferGate::Enforced);
    }

    /// ADR-123 変更A+C 決定4-2: `send_romaji_dispatching_on_gate` 経由の
    /// raw recovery 回収再送・ADR-101 決定3 retry 専用。新設した
    /// `defer_if_probe_in_flight` の `raw_recovery_owns_deferred()` 判定を
    /// この2経路にだけ適用してはならない——`raw_recovery_owns_deferred()` は
    /// 「今まさに自分が処理している recovery か」と「無関係などこか別の
    /// recovery が in-flight か」を区別できないグローバル述語であり、
    /// `flush_raw_tsf_literal_romaji` は自身の `pending_gji_reinit`
    /// Scheduled→Polling 遷移より**前**に呼ばれる（`flush_raw_tsf_literal_recovery`
    /// の呼び出し順序）ため、無関係な別 give-up 由来の `pending_gji_reinit` が
    /// 既に `Polling` 中だと、この再送自身が誤って `pending_deferred` に
    /// 積まれてしまう（probe を経由しない bare VK flush に化けて BUG-38/
    /// ADR-103 が塞いだ literal 化リスクが戻る、Opus敵対的レビュー round4指摘）。
    /// `has_pending_tsf()`（無関係な別 probe が実際に走っているか）による
    /// defer は PR4 以前から存在する挙動であり、この2経路にも従来どおり
    /// 適用する（`defer_if_probe_in_flight_recovery_exempt` 参照）。
    pub(super) fn send_romaji_batched_bypass_gate(&self, romaji: &str) {
        self.send_romaji_batched_gated(romaji, DeferGate::Exempt);
    }

    fn send_romaji_batched_gated(&self, romaji: &str, gate: DeferGate) {
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

        self.drain_pending_deferred_before_send_if_queue_only(gate);

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

        // ADR-123 変更A+C 決定4-4: この defer 判定は元々 `prepend_f2_warmup`
        // （cold）分岐の中でしか呼ばれておらず、warm 判定されたモーラは
        // `send_romaji_batch_immediate` へ直行して gate を一切通らなかった
        // （`assess_warmth()` の warm/cold は composition の温度状態であり、
        // `has_pending_tsf()`/`raw_recovery_owns_deferred()` とは独立した
        // 別軸——warm でも無関係な別 probe/recovery が in-flight でありうる）。
        // `assess_warmth()` より後（cold/warm 分岐の外）に置くことで両方を
        // カバーする。
        if self.defer_respecting_gate(romaji, gate) {
            return;
        }

        if prepend_f2_warmup {
            if session_expired {
                log::debug!("[vk-warmup] session expired ({elapsed}ms) → F2-only先行バッチ (案A)");
            } else {
                log::debug!("[vk-warmup] cold → F2-only先行バッチ (案A)");
            }

            let cold_seq = self.composition.increment_cold_start_count();
            let win_class = unsafe { crate::ime::get_foreground_window_class() };
            log::debug!(
                "[h1-window] cold={cold_seq} class={win_class}",
                cold_seq = cold_seq.value(),
            );

            // 予防的な programmatic F2 送信・TsfReadinessProbe の事前待機は 2026-07-18 に
            // 撤去した（実機ソーク数日で無破損確認、docs/known-bugs.md 参照）。per-VK
            // confirm（tsf_probe_coro_body の Phase 2c）が送信後の confirm/recovery を
            // 担うため、送信前に GJI の準備を待つ予防は二重の保険だった。
            let f2_sent_ms = crate::hook::current_tick_ms();
            let (probe_min_ms, probe_max_ms) = (0, 0);
            log::debug!(
                "[h1-probe] cold={cold_seq} idle_at_cold={}ms F2/probe待機省略 → per-VK confirm へ",
                self.composition.idle_ms_at_last_cold(),
                cold_seq = cold_seq.value(),
            );

            // SendMessageTimeoutW 系の同期呼び出しを with_app の外で実行するため、
            // async タスクへオフロードする（旧 set_ime_romaji_mode/send_f2_via_sendmessage
            // はいずれも削除済み、当時の設計意図の記録として残す）。
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
            self.install_pending_tsf(Box::new(
                crate::tsf::warmup::chrome_probe::ChromeProbe::new(
                    romaji,
                    cold_seq,
                    probe,
                    probe_max_ms,
                    guard,
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
                // 予防的な programmatic F2 送信は 2026-07-18 に撤去した（上記コメント参照）。
                // romaji は per-VK confirm（ChromeProbe/tsf_probe_coro_body）がそのまま送る。
            });

            return;
        }

        // MS-IME confirm-then-transmit ゲート（BUG-13 の Vk 注入モードへの拡張）:
        // warm GJI もここに到達しうるが、ゲート冒頭の needs_f2_probe() ガードで即
        // false になり no-op（GJI 戦略は上の prepend_f2_warmup 分岐が cold-start を
        // 担う）。実際にゲートが発動するのは MsImeStrategy（needs_f2_probe()=false）
        // のときのみ。IME ON 遷移直後（OS 準備に実測 ~130-300ms、WT×Tsf モードでの
        // 計測値）でも即送信すると先頭 VK がリテラル化する（「を」→「wお」、BUG-13）。
        // 従来 Tsf モードにしか配線していなかったが、IMC_GETCONVERSIONMODE の観測
        // 自体は injection mode に依存しないため Vk モードにも展開する。
        if self.ms_ime_gate_defer(romaji, TransmitTarget::Chrome, gate) {
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
    pub(super) fn send_vk_runs(chars: &[(VkCode, bool)], cold_seq: Generation) {
        KeyInjector::send_vk_runs(chars, cold_seq);
    }

    pub(super) fn send_romaji_as_tsf(&self, romaji: &str) {
        self.send_romaji_as_tsf_gated(romaji, DeferGate::Enforced);
    }

    /// `send_romaji_batched_bypass_gate` と対になる TSF 側。理由は同関数の
    /// doc コメント参照。
    pub(super) fn send_romaji_as_tsf_bypass_gate(&self, romaji: &str) {
        self.send_romaji_as_tsf_gated(romaji, DeferGate::Exempt);
    }

    fn send_romaji_as_tsf_gated(&self, romaji: &str, gate: DeferGate) {
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

        self.drain_pending_deferred_before_send_if_queue_only(gate);

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

        // ADR-123 変更A+C 決定4-4: `send_romaji_batched_gated` と同じ理由で
        // `assess_warmth()` の cold/warm 分岐の外に置く（warm 判定でも
        // gate を通す）。
        if self.defer_respecting_gate(romaji, gate) {
            return;
        }

        if prepend_f2_warmup {
            // ノンブロッキング warmup を開始して pending_tsf に保留
            let started = crate::tsf::warmup::cold_warmup::ColdWarmupSequence::new(self)
                .run_start(session_expired, elapsed);
            let cold_seq = started.probe.cold_seq;
            self.gji_begin_probe_guard();
            let probe_params = self.gji_current_probe_params().unwrap_or_else(|| {
                log::warn!(
                    "[tsf-send] cold パスだが GjiFsm に Authorized probe が無い（state={})",
                    self.gji_state_label()
                );
                crate::tsf::gji_fsm::ColdKind::Short.probe_params()
            });
            let coro = Box::new(crate::tsf::warmup::gji_warmup_coro::GjiWarmupCoro::new(
                romaji,
                cold_seq,
                started.probe,
                started.total_max_ms,
                started.cold_reason,
                used_eager_path,
                probe_params.forces_prepend_f2,
                probe_params.is_long_cold,
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
        if self.ms_ime_gate_defer(romaji, TransmitTarget::Tsf, gate) {
            return;
        }

        // warm パス: 即座に送信
        self.send_romaji_as_tsf_warm(romaji, &chars, used_eager_path);
    }

    /// MS-IME confirm-then-transmit ゲート（BUG-13）。defer した場合 `true` を返す。
    ///
    /// 発動条件: MS-IME 戦略（GJI probe 非対象）+ `ImeModeFsm` が NATIVE 未確認 +
    /// give-up latch なし。発動時は romaji を `MsImeReadyCoro` に預けて IMC 確認
    /// ポーリングを開始する。probe 進行中の後続キーは順序維持のため無条件で
    /// deferred キューに積む。
    ///
    /// `target` は呼び出し元の injection mode に対応する送信先
    /// （`send_romaji_as_tsf` → `Tsf`、`send_romaji_batched` → `Chrome`）。
    /// `IMC_GETCONVERSIONMODE` の観測自体は injection mode に依存しないため、
    /// 呼び出し元を切り替えるだけで両モードに同じゲートを展開できる
    /// （`InjectionMode::Unicode` は IME composition を経由しないため元々このゲートを
    /// 呼ばず、対象外のままでよい）。
    ///
    /// GJI の raw TSF literal 再送経路（`Output::flush_raw_tsf_literal_romaji` から
    /// `send_romaji_batched`/`send_romaji_batched_bypass_gate` 経由で呼ばれる）
    /// もこの関数を通るが、GJI 由来のため冒頭の `needs_f2_probe()` ガードで即
    /// false になり no-op（GJI には別途 F2 probe / LiteralDetect の保護がある）。
    /// この no-op は偶然ではなく、raw literal の記録自体が常に `gji_active`
    /// （`gji_is_active_ime()` / `env.gji_active`）でゲートされているため
    /// （`send_romaji_as_tsf_warm` の `LiteralDetectFsm` 設置、`probe_fsm.rs` の
    /// `enter_transmit_chrome`）に成り立つ。記録から `WM_DRAIN_OUTPUT_QUEUE` 経由の
    /// flush までの間に `active_ime_kind` が GJI→MS-IME に切り替わるレースは、TIP 検出が
    /// 2 秒間隔ポーリング + 2 tick 連続一致デバウンス（`gji_monitor.rs`）であるのに対し
    /// flush は同一メッセージループ内で完結するため、実質的に起こり得ない。
    ///
    /// **`gate` を呼び出し元からそのまま引き継ぐこと（2026-09-03 code review
    /// 指摘で修正）**: 以前は内部で無条件に `defer_if_probe_in_flight`
    /// （常に `Enforced` 相当）を直接呼んでおり、`send_romaji_batched_bypass_gate`/
    /// `send_romaji_as_tsf_bypass_gate`（raw recovery回収再送・ADR-101決定3
    /// retry専用）経由でここに到達した場合でも`raw_recovery_owns_deferred()`
    /// を評価してしまっていた。上記の「GJI由来なら`needs_f2_probe()`で
    /// no-op」という前提が成り立つ間は無害だが、MS-IME戦略の下でこの2経路に
    /// 到達する変更が将来入ると、4-2で塞いだのと同じ自己defer退行が
    /// この呼び出し口で再燃する（`gate`引数自体が無かったため、型では
    /// 防げていなかった）。`defer_respecting_gate`経由にすることで
    /// `gate`が`Exempt`の場合は`raw_recovery_owns_deferred()`を見ない
    /// （`has_pending_tsf()`のみ、PR4以前からの挙動）よう統一した。
    fn ms_ime_gate_defer(&self, romaji: &str, target: TransmitTarget, gate: DeferGate) -> bool {
        // GJI 戦略時は F2 probe 機構（prepend_f2_warmup 分岐）が cold-start を担う。
        if self.warmup_coord.needs_f2_probe() {
            return false;
        }
        // 既に probe/coro 進行中 → 確認状態に関わらず defer（送信順序の維持）。
        if self.defer_respecting_gate(romaji, gate) {
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
                "[msime-ready] cold={cold_seq} target={target:?} IME mode 未確認 \
                 (state={:?} confirmed={}) → {romaji:?} を defer して IMC 確認待ち",
                fsm.state(),
                fsm.is_confirmed(),
                cold_seq = cold_seq.value(),
            );
        }
        let cold_seq = self.composition.cold_start_count();
        let deadline_ms = crate::hook::current_tick_ms() + crate::tuning::MS_IME_READY_CONFIRM_MS;
        self.start_ms_ime_ready_poll(cold_seq, deadline_ms);
        let coro = Box::new(crate::tsf::warmup::ms_ime_ready_coro::MsImeReadyCoro::new(
            romaji,
            cold_seq,
            deadline_ms,
            target,
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
                "[tsf-warm-start] cold={cold_seq} PendingGjiConfirm: GJI 未応答 → romaji={romaji:?} を unicode で強制送信",
                cold_seq = cold_seq.value(),
            );
            true
        } else {
            used_eager_path
        };

        log::debug!(
            "[tsf-warm-start] cold={cold_seq} romaji={romaji:?} t={t_warm}ms",
            cold_seq = cold_seq.value(),
        );
        let outcome = WarmupOutcome {
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
                    cold_seq = cold_seq.value(),
                );
            });
        }

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
            self.install_pending_tsf(Box::new(
                crate::tsf::warmup::literal_detect_fsm::LiteralDetectFsm::new(
                    cold_seq,
                    romaji.to_owned(),
                    crate::tsf::warmup::probe_fsm::ProbeObservations {
                        nc_fired: false,
                        ..Default::default()
                    },
                    ze_bs_count,
                    crate::tuning::RAW_TSF_LITERAL_DETECT_MS,
                    self.composition.consecutive_count(),
                ),
            ));
        } else {
            // ze_bs_count は Probing+GJI 健全パスでのみ使う。
            // 他パスでは warm マーク済みで LiteralDetect 不要。
            let _ = ze_bs_count;
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
                // ASCII 1 文字で表現できる記号（。→'.'、、→','、ー→'-'、Shift 付きの
                // ！→'!'/？→'?'/～→'~' 等も含め build_symbol_to_vk の全記号）は
                // romaji 送信経路（send_romaji_as_tsf）へ合流させる。これにより
                // assess_warmth() によるcold-startウォームアップ判定・probe設置・
                // MS-IME confirmゲートを記号送信でも通す（2026-08-03/2026-08-05
                // ユーザー報告 BUG-47: cold な TSF へ記号 VK を無条件送信すると
                // 変換されず半角のまま出力される。docs/known-bugs.md BUG-47 参照）。
                // warm パスは既存の send_vk_pair(vk, needs_shift, VkMarker::Tsf) と
                // バイト列が同一になるよう vk_pair_to_ascii を ascii_to_vk の厳密な
                // 逆写像として設計してある（vk.rs のラウンドトリップテスト参照）。
                if let Some(ascii) = crate::vk::vk_pair_to_ascii(vk, needs_shift) {
                    log::debug!(
                        "    send_char_as_tsf: '{ch}' → VK 0x{vk:02X} shift={needs_shift} \
                         → romaji \"{ascii}\" 経由 (cold-start保護あり)"
                    );
                    let mut buf = [0u8; 4];
                    self.send_romaji_as_tsf(ascii.encode_utf8(&mut buf));
                    return;
                }
                log::debug!("    send_char_as_tsf: '{ch}' → VK 0x{vk:02X} shift={needs_shift}");
                // probe 進行中は VK を後回しにして romaji との送信順序を保証する
                // （このフォールバック自体が現状理論上到達しない。理由は下の
                // send_vk_pair 直後のコメント参照）。
                if self.defer_vk_if_probe_in_flight(vk, needs_shift, DeferredOrigin::UserInput) {
                    log::debug!("    send_char_as_tsf: VK 0x{vk:02X} deferred (probe in flight)");
                    return;
                }
                Self::send_vk_pair(vk, needs_shift, VkMarker::Tsf);
                // 2026-08-05 修正で vk_pair_to_ascii が build_symbol_to_vk の全記号
                // （Shift 付きを含む）をカバーしたため、このフォールバックは
                // resolve_char が build_symbol_to_vk 以外から VK を返すようになった
                // 場合の保険としてのみ残る（現状は理論上到達しない）。到達した場合の
                // 安全側の挙動として常に cold マークする。
                self.mark_composition_cold(ColdReason::SymbolVkSent);
                self.warmup_coord.mark_composition_reset();
                self.send_eager_tsf_warmup(awase::platform::WarmupImeOn::off(), "off");
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
                // ASCII 1 文字で表現できる記号（Shift 付きを含む build_symbol_to_vk
                // の全記号）は romaji 送信経路（send_romaji_batched）へ合流させる。
                // send_char_as_tsf 側の同種修正と対称（コメント参照）。
                // 2026-08-03/2026-08-05 ユーザー報告 BUG-47。
                if let Some(ascii) = crate::vk::vk_pair_to_ascii(vk, needs_shift) {
                    log::debug!(
                        "    send_char_as_vk: '{ch}' → VK 0x{vk:02X} shift={needs_shift} \
                         → romaji \"{ascii}\" 経由 (cold-start保護あり)"
                    );
                    let mut buf = [0u8; 4];
                    self.send_romaji_batched(ascii.encode_utf8(&mut buf));
                    return;
                }
                log::debug!("    send_char_as_vk: '{ch}' → VK 0x{vk:02X} shift={needs_shift}");
                // probe 進行中は VK を後回しにして romaji との送信順序を保証する
                // （このフォールバック自体が現状理論上到達しない。理由は下の
                // send_vk_pair 直後のコメント参照）。
                if self.defer_vk_if_probe_in_flight(vk, needs_shift, DeferredOrigin::UserInput) {
                    log::debug!("    send_char_as_vk: VK 0x{vk:02X} deferred (probe in flight)");
                    return;
                }
                // scan code 付き（VkMarker::InjectedWithScan）、send_romaji_batch_immediate
                // と同じ恒久仕様。詳細はそちらのコメント参照。2026-08-05 修正で
                // vk_pair_to_ascii が build_symbol_to_vk の全記号をカバーしたため、
                // このフォールバックは resolve_char が build_symbol_to_vk 以外から
                // VK を返すようになった場合の保険としてのみ残る（現状は理論上到達しない）。
                Self::send_vk_pair(vk, needs_shift, VkMarker::InjectedWithScan);
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── drain_pending_deferred_before_send_if_queue_only テスト（ADR-123
    // 変更A+C 決定4-3）────────────────────────────────────────────────

    #[test]
    fn drain_pending_deferred_before_send_if_queue_only_preserves_queue_while_blocking() {
        // blocking 中（has_pending_tsf()=true）は「queue-only」ではないため
        // drain してはいけない——キューはそのまま残る。実送信(SendInput)を
        // 発火させないケースのみをここで検証する。
        //
        // 既知のテストギャップ（Opus敵対的レビュー round4-3指摘）:
        // drain が実際に発火する正のケース（queue-only → flush →
        // pending_deferred_len()==0）はここではカバーしていない。
        // `flush_pending_deferred_vks` は実際に `SendInput` を発火する経路
        // （`key_injector.rs`）に委譲しており、この crate には元々
        // `SendInput` を伴う送信経路を直接叩くユニットテストが1件も無い
        // （`grep -n "fn.*test" key_injector.rs` で確認済み）。同じ理由で
        // このケースも e2e/golden シナリオでカバーする前提とする。
        let o = Output::new();
        o.install_pending_tsf(Box::new(
            crate::tsf::warmup::chrome_probe::ChromeProbe::new(
                "x",
                Generation::INITIAL,
                crate::tsf::probe::TsfReadinessProbe::new(0, Generation::INITIAL, 0),
                0,
                OutputActiveGuard::begin(),
            ),
        ));
        o.warmup_coord
            .push_deferred_vks(&[(VkCode(0x41), false)], DeferredOrigin::UserInput);
        assert_eq!(o.pending_deferred_len(), 1);

        o.drain_pending_deferred_before_send_if_queue_only(DeferGate::Enforced);

        assert_eq!(
            o.pending_deferred_len(),
            1,
            "blocking 中は drain せずキューを保持すべき"
        );
    }

    #[test]
    fn drain_pending_deferred_before_send_if_queue_only_is_noop_when_queue_empty() {
        // blocking もしておらずキューも空 = 何もしない（drainを試みない）。
        let o = Output::new();
        assert_eq!(o.pending_deferred_len(), 0);

        o.drain_pending_deferred_before_send_if_queue_only(DeferGate::Enforced);

        assert_eq!(o.pending_deferred_len(), 0);
    }

    #[test]
    fn drain_pending_deferred_before_send_if_queue_only_preserves_queue_when_gate_exempt() {
        // ADR-128 / BUG-109: BUG-109 で queue に滞留していたのはユーザー入力
        // モーラ（`DeferredOrigin::UserInput`、「ま」「え」）である。`gate` は
        // 送信者側（recovery resend か通常ユーザー入力か）の属性で、queue内
        // アイテムの `origin` とは別軸（ADR-123 変更B）——ここを
        // `RecoveryResend` にすると「recovery resend 起源の VK は drain
        // されない」という別の主張に読めてしまうため、`UserInput` を使い
        // 「recovery resend の送信者はユーザーモーラを drain してはならない」
        // という本来の主張のまま検証する（コードレビュー指摘）。
        let o = Output::new();
        o.warmup_coord
            .push_deferred_vks(&[(VkCode(0x41), false)], DeferredOrigin::UserInput);
        assert_eq!(o.pending_deferred_len(), 1);

        o.drain_pending_deferred_before_send_if_queue_only(DeferGate::Exempt);

        assert_eq!(
            o.pending_deferred_len(),
            1,
            "gate=Exempt の recovery resend は drain せずキューを保持すべき"
        );
        assert_eq!(
            o.take_pending_drain_before_send_flush(),
            0,
            "drain しなかった以上、journal 化すべき flush 事実も無いはず"
        );
    }
}
