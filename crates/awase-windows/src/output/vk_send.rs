#![allow(unsafe_code)] // Win32 API 呼び出しに unsafe が必須(lib.rsのクレート全体allowから個別移管、Task #9)
use super::key_injector::{KeyInjector, VkMarker};
use super::resolve::{ascii_to_vk, CharResolution};
use super::{fmt_ms, WarmthContext};
use super::{Output, VkSequence};
use crate::state::deferred_gate_plan;
use crate::tsf::probe_bridge::OutputActiveGuard;
use crate::tsf::warmup::probe_fsm::DeferredOrigin;
use awase::types::VkCode;

/// ADR-123 変更A+C 決定4-2: `send_romaji_batched` の
/// 新設 `defer_if_probe_in_flight` gate を適用するかどうか。
/// （撤去済みの TsfGate とは無関係の別概念。）`Exempt` は
/// `send_romaji_batched_bypass_gate`
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
        // 判断は `deferred_gate_plan::plan_drain_before_send`（FCIS F6）。値は元の短絡評価と同じ順・同じ条件でだけ読む。
        let gate_enforced = gate == DeferGate::Enforced;
        let blocking = if gate_enforced {
            self.probe_or_recovery_block_reason(true)
        } else {
            None
        };
        let queue_len = if gate_enforced && blocking.is_none() {
            self.pending_deferred_len()
        } else {
            0
        };
        if !matches!(
            deferred_gate_plan::plan_drain_before_send(gate_enforced, blocking, queue_len),
            deferred_gate_plan::DrainBeforeSendPlan::Flush
        ) {
            return;
        }
        let n = self.flush_pending_deferred_vks();
        if n > 0 {
            tracing::debug!(
                "[pending-deferred] drain-before-send: 新規モーラの前に取り残し {n} VK(s) をflush"
            );
            // `JournalEntry` への変換は platform.rs 側で行う（output/tsf は
            // `crate::journal` を直接参照しない、`Output::
            // pending_drain_before_send_flush` の doc 参照）。
            self.pending_drain_before_send_flush.set(n);
        }
    }
}

impl Output {
    /// 表に無い文字を `KEYEVENTF_UNICODE` で直接送信する（`CharResolution::Unicode` のフォールバック）。
    ///
    /// 実際の送信処理は `KeyInjector::send_unicode_char` に委譲する。
    pub(super) fn send_unicode_char(&self, ch: char) {
        self.injector.send_unicode_char(ch);
    }

    /// Batched モード: 全文字を1回の SendInput にまとめて送信（重畳押し順）
    ///
    /// cold 時は GJI プローブを開始し（ノンブロッキング）、TIMER_TSF_PROBE が
    /// Chrome probe フェーズを進めてローマ字を送信する。
    pub(super) fn send_romaji_batched(&self, romaji: &str) {
        self.send_romaji_batched_gated(romaji, DeferGate::Enforced);
    }

    /// ADR-123 変更A+C 決定4-2: `send_romaji_recovery` 経由の
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

    #[tracing::instrument(level = "debug", skip_all, fields(romaji = romaji))]
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
            tracing::info!("[key-output] KeyInput(batched): romaji={romaji:?} {ime_suffix}");
        }

        self.drain_pending_deferred_before_send_if_queue_only(gate);

        let WarmthContext {
            warm,
            elapsed,
            prepend_f2_warmup,
        } = self.assess_warmth();
        tracing::debug!(
            "[vk-send] romaji={romaji:?} warm={warm} elapsed={}ms prepend_f2_warmup={prepend_f2_warmup}",
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
            tracing::debug!("[vk-warmup] cold → F2-only先行バッチ (案A)");

            let cold_seq = self.composition.increment_cold_start_count();
            let win_class = unsafe { crate::ime::get_foreground_window_class() };
            tracing::debug!(
                "[h1-window] cold={cold_seq} class={win_class}",
                cold_seq = cold_seq.value(),
            );

            // 予防的な programmatic F2 送信・TsfReadinessProbe の事前待機は 2026-07-18 に
            // 撤去した（実機ソーク数日で無破損確認、docs/known-bugs.md 参照）。per-VK
            // confirm（tsf_probe_coro_body の Phase 2c）が送信後の confirm/recovery を
            // 担うため、送信前に GJI の準備を待つ予防は二重の保険だった。
            let f2_sent_ms = crate::hook::current_tick_ms();
            tracing::debug!(
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
            // H-4-b: Chrome 用 TsfProbeCoro を spawn_local より前に同期生成してインストールする。
            // これにより async クロージャ内の with_app() → Runtime 逆参照が不要になり、
            // Runtime → Platform → Output → グローバル → Runtime の循環依存を断つ。
            // guard は TsfProbeCoro に move され、probe 完了まで OUTPUT_GATE を保持する。
            // TIMER_TSF_PROBE は WindowsPlatform::drain_output_post_send_effects が
            // pending_tsf_timer() で起動する（send_keys・raw recovery の再送の両方の後に呼ばれる。
            // sync パスと同一経路）。
            let guard = OutputActiveGuard::begin();
            let probe = crate::tsf::probe::TsfReadinessProbe::new(f2_sent_ms, cold_seq);
            self.install_pending_tsf(Box::new(
                crate::tsf::warmup::probe_fsm::TsfProbeCoro::new_chrome(
                    romaji, cold_seq, probe, guard,
                ),
            ));

            win32_async::spawn_local(async move {
                // 診断: pre-send IME conversion mode（旧 [cold-diag] log）
                let conv_pre = crate::ime::get_ime_conversion_mode_raw_timeout_async(50).await;
                tracing::debug!(
                    "[cold-diag] pre-send conv={} NATIVE={} ROMAN={} KATAKANA={}",
                    conv_pre.map_or_else(|| "none".to_string(), |v| format!("0x{v:08X}")),
                    conv_pre
                        .is_some_and(|v| crate::imm::cmode_has(v, crate::imm::IME_CMODE_NATIVE)),
                    conv_pre.is_some_and(|v| crate::imm::cmode_has(v, crate::imm::IME_CMODE_ROMAN)),
                    conv_pre
                        .is_some_and(|v| crate::imm::cmode_has(v, crate::imm::IME_CMODE_KATAKANA)),
                );
                // 予防的な programmatic F2 送信は 2026-07-18 に撤去した（上記コメント参照）。
                // romaji は per-VK confirm（tsf_probe_coro_body）がそのまま送る。
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
        if self.ms_ime_gate_defer(romaji, gate) {
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

    /// MS-IME confirm-then-transmit ゲート（BUG-13）。defer した場合 `true` を返す。
    ///
    /// 発動条件: MS-IME 戦略（GJI probe 非対象）+ `ImeModeFsm` が NATIVE 未確認 +
    /// give-up latch なし。発動時は romaji を `MsImeReadyCoro` に預けて IMC 確認
    /// ポーリングを開始する。probe 進行中の後続キーは順序維持のため無条件で
    /// deferred キューに積む。
    ///
    /// 呼び出し元は `send_romaji_batched` だけ（Tsf 注入経路は 2026-10-22 に撤去した）。
    ///
    /// GJI の raw TSF literal 再送経路（`Output::flush_raw_tsf_literal_romaji` から
    /// `send_romaji_batched`/`send_romaji_batched_bypass_gate` 経由で呼ばれる）
    /// もこの関数を通るが、GJI 由来のため冒頭の `needs_f2_probe()` ガードで即
    /// false になり no-op（GJI には別途 F2 probe / LiteralDetect の保護がある）。
    /// この no-op は偶然ではなく、raw literal の記録自体が常に `gji_active`
    /// （`gji_is_active_ime()` / `env.gji_active`）でゲートされているため
    /// （`probe_fsm.rs` の `enter_transmit_chrome`。warm パスの `LiteralDetectFsm` 設置は ECRS MB-02 で撤去）に成り立つ。記録から `WM_DRAIN_OUTPUT_QUEUE` 経由の
    /// flush までの間に `active_ime_kind` が GJI→MS-IME に切り替わるレースは、TIP 検出が
    /// 2 秒間隔ポーリング + 2 tick 連続一致デバウンス（`gji_monitor.rs`）であるのに対し
    /// flush は同一メッセージループ内で完結するため、実質的に起こり得ない。
    ///
    /// **`gate` を呼び出し元からそのまま引き継ぐこと（2026-09-03 code review
    /// 指摘で修正）**: 以前は内部で無条件に `defer_if_probe_in_flight`
    /// （常に `Enforced` 相当）を直接呼んでおり、`send_romaji_batched_bypass_gate`/
    /// （raw recovery回収再送・ADR-101決定3
    /// retry専用）経由でここに到達した場合でも`raw_recovery_owns_deferred()`
    /// を評価してしまっていた。上記の「GJI由来なら`needs_f2_probe()`で
    /// no-op」という前提が成り立つ間は無害だが、MS-IME戦略の下でこの2経路に
    /// 到達する変更が将来入ると、4-2で塞いだのと同じ自己defer退行が
    /// この呼び出し口で再燃する（`gate`引数自体が無かったため、型では
    /// 防げていなかった）。`defer_respecting_gate`経由にすることで
    /// `gate`が`Exempt`の場合は`raw_recovery_owns_deferred()`を見ない
    /// （`has_pending_tsf()`のみ、PR4以前からの挙動）よう統一した。
    fn ms_ime_gate_defer(&self, romaji: &str, gate: DeferGate) -> bool {
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
            tracing::info!(
                "[msime-ready] cold={cold_seq} IME mode 未確認 \
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
        ));
        self.install_pending_tsf(coro);
        // WindowsPlatform::send_keys が pending_tsf_timer() で TIMER_TSF_PROBE を起動する
        true
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
                tracing::debug!("    send_char_as_vk: '{ch}' → romaji \"{romaji}\"");
                // Batched (1回の SendInput) を使うことで、後続キー（Enter reinject 等）との
                // 競合を防ぐ。per_key では K↓K↑ と A↓A↑ が別 SendInput になり、
                // 間に Enter が割り込むと "kあ" のような出力破壊が起きる。
                self.send_romaji_batched(romaji);
            }
            CharResolution::Vk(vk, needs_shift) => {
                // ASCII 1 文字で表現できる記号（Shift 付きを含む build_symbol_to_vk
                // の全記号）は romaji 送信経路（send_romaji_batched）へ合流させる。
                // 2026-08-03/2026-08-05 ユーザー報告 BUG-47。
                if let Some(ascii) = crate::vk::vk_pair_to_ascii(vk, needs_shift) {
                    tracing::debug!(
                        "    send_char_as_vk: '{ch}' → VK 0x{vk:02X} shift={needs_shift} \
                         → romaji \"{ascii}\" 経由 (cold-start保護あり)"
                    );
                    let mut buf = [0u8; 4];
                    self.send_romaji_batched(ascii.encode_utf8(&mut buf));
                    return;
                }
                tracing::debug!("    send_char_as_vk: '{ch}' → VK 0x{vk:02X} shift={needs_shift}");
                // probe 進行中は VK を後回しにして romaji との送信順序を保証する
                // （このフォールバック自体が現状理論上到達しない。理由は下の
                // send_vk_pair 直後のコメント参照）。
                if self.defer_vk_if_probe_in_flight(vk, needs_shift, DeferredOrigin::UserInput) {
                    tracing::debug!(
                        "    send_char_as_vk: VK 0x{vk:02X} deferred (probe in flight)"
                    );
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
                tracing::debug!(
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::event_origin::Generation;

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
            crate::tsf::warmup::probe_fsm::TsfProbeCoro::new_chrome(
                "x",
                Generation::INITIAL,
                crate::tsf::probe::TsfReadinessProbe::new(0, Generation::INITIAL),
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
