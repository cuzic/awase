use crate::state::deferred_gate_plan;
use crate::state::event_origin::Generation;
use crate::state::half_width_alnum::HalfWidthAlnumAction;
use crate::state::warm_send_plan;
use crate::tsf::warmup::probe_fsm::DeferredOrigin;
use crate::vk::ascii_to_vk;
use awase::types::{KeyAction, VkCode};
use std::time::Duration;

pub use crate::tsf::output::ColdReason;
pub use crate::tsf::output::{INJECTED_MARKER, TSF_MARKER};

pub mod sender;
pub(crate) mod types;
pub(crate) use sender::OutputSession;
pub(crate) use types::InjectionMode;

pub(crate) mod conv_actuation;
pub(crate) mod held_modifiers;
mod key_injector;
pub(crate) mod probe_io;
mod resolve;
mod tsf_warmup_coord;
mod vk_send;
use resolve::special_key_to_vk;
pub(crate) use tsf_warmup_coord::TsfWarmupCoordinator;

/// 公開ヘルパー: ASCII → VK 変換（`platform.rs` の dispatcher 用）。
pub(crate) use crate::vk::ascii_to_vk as resolve_ascii_to_vk;
/// SendInput / Unicode / VK 送信コンポーネント。
pub(crate) use key_injector::{KeyInjector, VkMarker};
/// 公開ヘルパー: TSF 送信パイプライン（`platform.rs` の dispatcher 用）。
pub(crate) use vk_send::TsfSendPipeline;

/// VK コード＋シフトフラグのペアを要素とする VK シーケンス型。
pub(crate) type VkSequence = Vec<(VkCode, bool)>;

/// `WindowsPlatform` へのタイマー操作指示。`Output::step_probe` / `pending_tsf_timer` が返す。
///
/// タイマーの set/kill 判断は `Output` 側で完結し、`WindowsPlatform` は受け取ったコマンドを
/// 実行するだけになる。これにより `Output` が Win32 タイマー ID を知る必要がなくなる。
#[derive(Debug, Clone, Copy)]
pub(crate) enum TimerCommand {
    /// 指定タイマーを継続（未セットなら新規セット、既セットなら再セット）。
    Continue { id: usize, delay: Duration },
    /// 指定タイマーを kill する。
    Kill { id: usize },
}

/// `u64::MAX` は「未送信」を意味するセンチネル値。ログ表示用に "∞" に変換する。
#[must_use]
pub(crate) fn fmt_ms(ms: u64) -> String {
    if ms == u64::MAX {
        "∞".to_owned()
    } else {
        ms.to_string()
    }
}

/// SendInput によるキー注入を行うモジュール。
///
/// キー注入の低レベル操作は [`KeyInjector`] に委譲する Facade。
pub struct Output {
    /// SendInput / Unicode / VK 送信コンポーネント。
    ///
    /// `kana_table`・`symbol_to_vk`・`unicode_cold_defer` 等を内包し、
    /// 低レベルのキー注入操作を一括して管理する。
    pub(crate) injector: KeyInjector,
    /// TSF composition context の warm/cold 状態管理。
    ///
    /// warm/cold epoch、last_send_ms、eager_warmup_sent_ms 等を集約する。
    /// 詳細は [`crate::tsf::probe::CompositionState`] を参照。
    pub composition: crate::tsf::probe::CompositionState,
    /// GJI ウォームアップ / TSF プローブ調停コンポーネント。
    ///
    /// warmup 戦略・保留 TSF プローブ FSM・probe_id・OUTPUT_GATE ガード・
    /// GJI FSM 橋渡しバッファ群を集約する。詳細は [`TsfWarmupCoordinator`] を参照。
    /// `output` モジュール外からは `Output` の公開メソッド経由でのみ操作する。
    warmup_coord: TsfWarmupCoordinator,
    /// フォーカス変更直後の TSF モード確定前にキーを一時保留するゲート。
    ///
    /// PendingWarmup 状態中のみキーを保留し、run_with_prefetched 完了後に
    /// Probing または Bypass に遷移して保留キーを再処理する。
    pub(crate) tsf_gate: crate::tsf::TsfGate,
    /// フォーカス変更時に Runtime から push される注入モード。
    ///
    /// フォーカスが確定するたびに `update_injection_mode()` で更新される。
    /// `with_app_ref` によるグローバル読み取りを排除し、output 層を self-contained にする。
    pub(crate) injection_mode: InjectionMode,
    /// IME 変換モード管理コンポーネント。
    ///
    /// `kp_stage_idle_conv_check` が `observe()` で更新し、
    /// `cold_warmup` と `transmit_tsf` が warmup VK と `ImmSetConversionStatus` 目標値の選択に使う。
    pub(crate) conv_mode: crate::state::ConvModeMgr,
    /// IME 入力モード belief（Off / Hiragana / Katakana / Unknown）。
    ///
    /// VK_IME_ON/OFF 送信時に即時 belief 更新。`IMC_GETCONVERSIONMODE` async ポーリングで確認。
    /// `TsfEnvSnapshot.ime_mode` / `ime_mode_confirmed` を通じて各 TickableFsm に公開する。
    /// `ChromeGjiReinitFsm` が VK_IME_OFF→VK_IME_ON 後の Hiragana 確認待機に使用する。
    pub(crate) ime_mode_fsm: std::cell::RefCell<crate::tsf::ime_mode_fsm::ImeModeFsm>,
    /// `gji_on_focus_change` の `spawn_local` IMC ポーリングを世代管理する。
    ///
    /// フォーカス変更のたびにインクリメントし、`spawn_local` クロージャが取得時の世代を
    /// キャプチャする。コールバック到達時に現在値と一致しない（= その後に別のフォーカス変更
    /// が来た）場合は stale として破棄し、古いポーリング結果で ImeModeFsm を汚染しない。
    pub(crate) ime_mode_focus_gen: std::cell::Cell<u32>,
    /// MS-IME confirm-then-transmit ゲート（BUG-13）の give-up latch。
    ///
    /// `start_ms_ime_ready_poll` が「期限まで IMC が一度も確認できなかった」ときに立てる。
    /// IMC が読めないアプリで毎キーストロークが probe 化（+`MS_IME_READY_CONFIRM_MS` の
    /// 遅延）するのを防ぐ。フォーカス変更と `SetOpen(true)` 適用でクリアされ、
    /// 再確認の機会が与えられる。
    pub(crate) ms_ime_gate_give_up: std::cell::Cell<bool>,
    /// MS-IME confirm-then-transmit ゲート（BUG-13）の期限を、`shift-conv-guard`
    /// の hold 中だけ上書きするための値。`0` = 上書きなし（通常どおり defer 時点で
    /// 計算した固定 `deadline_ms` を使う）。
    ///
    /// `runtime/key_pipeline.rs` の `kp_shift_conv_guard_key_down` が
    /// `platform_state.gate.shift_conv_guard_pending` を立てるのと同時に
    /// `current_tick_ms() + SHIFT_CONV_GUARD_ENTRY_SUSPEND_CAP_MS`（有限キャップ、
    /// 真の無期限ではない）をセットして期限を延長する（awase 自身が
    /// conv=0x0000 を書いた直後だと分かっている間、IMC 未確認を理由に
    /// 強制送信・give-up latch してはならない — BUG-49 追補2）。
    /// `kp_shift_conv_guard_key_up`（`pending` 消費時点、チョード確定でも
    /// 単独タップ確定でも共通）が `current_tick_ms() + SHIFT_CONV_GUARD_RELEASE_CONFIRM_MS`
    /// （hold 終了時点を起点とするフレッシュな猶予）へ差し替え、続く
    /// `kp_restore_kana_from_half_width` のリトライループが `shift_conv_guard_gen`
    /// が自分の起動時点と一致する限り毎試行ごとに同じ幅で押し出し続ける。
    ///
    /// 消費側（`MsImeReadyCoro`/`start_ms_ime_ready_poll`）は
    /// `deadline_ms.max(この値)` を実効期限として使う。`0` のときは
    /// `deadline_ms`（送信試行時点起点、BUG-13 の元々の cold-start 保護）が
    /// そのまま効く。`shift-conv-guard` と無関係な確認待ちには一切影響しない。
    pub(crate) confirm_gate_deadline_override_ms: std::cell::Cell<u64>,
    /// `confirm_gate_deadline_override_ms` の所有権世代（ADR-084 BUG-49 追補2、
    /// Opus pass-5 レビュー指摘）。
    ///
    /// `kp_shift_conv_guard_key_down` の MS-IME entry 分岐が新しい hold を
    /// 開始するたびインクリメントする。`kp_restore_kana_from_half_width` は
    /// 起動時点でこの値を `owner_gen` として捕獲し、`spawn_local` リトライ
    /// ループの各試行で現在値と一致するかを確認してから
    /// `confirm_gate_deadline_override_ms` を書く。
    ///
    /// これが無いと、hold #1 の解放直後に hold #2 が始まった場合（実測: 通常の
    /// 連続 Shift タップ間隔で発生しうる）、hold #1 の detached restore task が
    /// NATIVE 確認後に override を `0` へクリアする書き込みが hold #2 の
    /// 有効な override を消してしまい、hold #2 で BUG-49 が release 側として
    /// 再発する（pass-5 レビューで発見）。世代不一致のときはループを即座に
    /// 中断し、IMC write すら行わない（フォーカスが既に別の対象へ移っている
    /// 可能性があるため、無関係な書き込みもしない）。
    pub(crate) shift_conv_guard_gen: std::cell::Cell<u32>,
    /// Unicode 送信後に GJI write 観測を行うフラグ。
    ///
    /// Platform::send_keys が Unicode モード + 未学習クラスのときにセットし、
    /// send_keys 内の `KeyAction::Romaji` 処理で `UnicodeLiteralObserverFsm` をインストールする。
    /// フラグは最初の Romaji 送信時に消費される（swap false）。
    observe_unicode_literal: std::sync::atomic::AtomicBool,
    /// `ConvModeAuthority::AwaseOwned` のときだけ true。
    ///
    /// `send_eager_tsf_warmup` / `ImmSetConversionStatus` 等の conv mutation を一括ガードする。
    /// `Platform::set_conv_mode_authority` が `allows_conv_mutation()` の結果を push する。
    pub(crate) conv_mutation_allowed: std::cell::Cell<bool>,
    /// Output → Runtime の遅延リクエストを蓄積するアウトボックス。
    ///
    /// キー注入中に `with_app` 経由で Runtime を直接呼ぶと再入するため、
    /// `RuntimeRequest` を積んでキー処理境界で Runtime が `take_pending_requests` で drain する。
    /// H-4-b: vk_send.rs Chrome cold パスが `StartTsfProbe` を積み、
    /// drain_runtime_requests が TIMER_TSF_PROBE を起動する。
    pub(crate) runtime_outbox: std::cell::RefCell<crate::runtime::outbox::RuntimeOutbox>,
    /// ADR-128: drain-before-send が実際に flush した件数を Platform へ渡す
    /// ための一時バッファ。0 は「未 flush」を表す（`vk_count` は 0 の場合
    /// push されないため曖昧さは無い、`suppressed_literal_confirms` と
    /// 同じ 0 デフォルトのアキュムレータパターン）。呼び出しグラフ上、
    /// 1回の `drain_output_post_send_effects` の間に2回以上 push される
    /// ことは無い（`send_keys` バッチ内の2文字目以降は
    /// `is_probe_or_recovery_blocking(true)` が true になるため drain 自体が
    /// 起きない）ため `Vec` ではなく `Cell` で十分（/code-review 指摘）。
    ///
    /// `output`/`tsf` の本番コードは `crate::journal` を直接参照してはならない
    /// （`JournalEntry` への変換は platform.rs に一元化する、
    /// `tests/architecture_guard.rs::
    /// output_and_tsf_production_code_do_not_reference_journal_directly`）。
    /// そのため `drain_pending_deferred_before_send_if_queue_only` は
    /// `JournalEntry` そのものではなく生の `vk_count` だけをここに積み、
    /// `WindowsPlatform::drain_output_post_send_effects`（全送信直後に呼ばれる、
    /// `push_journal_entry` の seq/elapsed_ms が「flush 時刻」に近い値になる
    /// 唯一の場所）が `JournalEntry::DeferredRecoveryFlush { trigger:
    /// "drain_before_send", .. }` へ変換する（`tsf::literal_facts::
    /// LiteralDetectRecord` を platform.rs 側で `JournalEntry::LiteralDetect`
    /// に包むのと型付けは同じパターンだが、変換タイミングは「発生直後」に
    /// 揃えている点が異なる——`drain_journal_entries` まで遅延させると、
    /// この計装の目的である「drain が resend より前に発火したことを示す」
    /// こと自体が journal 上で逆順になる、コードレビュー指摘）。
    pending_drain_before_send_flush: std::cell::Cell<usize>,
}

impl std::fmt::Debug for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Output").finish_non_exhaustive()
    }
}

/// `Output::flush_raw_tsf_literal_recovery` の結果（ADR-123）。
///
/// 呼び出し元（`platform.rs`）が journal
/// （`JournalEntry::DeferredRecoveryFlush`）へ記録するための情報。従来は
/// `tracing::debug!`/`tracing::warn!` の自由文字列でしか残らず、`pending_deferred`
/// が実際に flush/discard されたかが journal から追えなかった
/// （issue #148 の調査で `app_log_excerpt` を直接読まないと確認できなかった）。
#[derive(Debug, Clone, Copy)]
pub(crate) enum RawRecoveryOutcome {
    /// `pending_deferred` を実際に flush した（0 件なら「取り残しなし」）。
    Flushed { vk_count: usize },
}

/// `assess_warmth` の戻り値。composition の温度状態をまとめる。
pub(super) struct WarmthContext {
    pub warm: bool,
    pub elapsed: u64,
    pub session_expired: bool,
    pub prepend_f2_warmup: bool,
}

/// `Output::step_probe` の戻り値。タイマー命令と GjiFsm レスポンスを束ねる。
pub(crate) struct StepProbeResult {
    pub timer_cmd: TimerCommand,
    /// probe 完了時に GjiFsm から返ってきた Response（`WarmupComplete` イベント由来）。
    /// `None` = probe 進行中 or warmup result がなかった（probe_id 不一致等）。
    pub gji_response:
        Option<timed_fsm::Response<crate::tsf::gji_fsm::GjiAction, crate::tsf::gji_fsm::GjiTimer>>,
    /// `ProbeIo::mark_cold_raw_tsf` が呼ばれたとき true になる。
    /// `advance_tsf_probe` が `gji_on_composition_reset` を呼ぶために使う。
    pub needs_gji_composition_reset: bool,
    /// `UnicodeLiteralObserverFsm` が GJI write なしと判断したとき true になる。
    /// `advance_tsf_probe` がフォーカス中クラスを Tsf に昇格する。
    pub learned_tsf: bool,
    pub completed_cold_seq: Option<u64>,
    pub literal_detect: crate::tsf::literal_facts::LiteralDetectTrace,
}

/// `ensure_tsf_warm` の戻り値。warmup フローの結果を表す。
pub(crate) struct WarmupOutcome {
    /// eager warmup パス（既存の F2 経由）を通ったか（Unicode 送信判定に使用）
    pub used_eager_path: bool,
    /// cold start シーケンス番号（ログ相関用）
    pub cold_seq: Generation,
}

/// 状態管理・キー送信・TSF プローブ FSM を含む主実装ブロック。
///
/// - 状態アクセサ（warmth、composition、injection_mode、TsfGate）
/// - キー送信（`send_keys`、`send_romaji_*`、`send_char_*`、`send_unicode_char`）
/// - ノンブロッキング TSF/Chrome プローブ FSM（`advance_tsf_probe` とその内部メソッド群）
impl Default for Output {
    fn default() -> Self {
        Self::new()
    }
}

impl Output {
    #[must_use]
    pub fn new() -> Self {
        Self {
            injector: KeyInjector::new(),
            composition: crate::tsf::probe::CompositionState::new(),
            warmup_coord: TsfWarmupCoordinator::new(),
            tsf_gate: crate::tsf::TsfGate::new(),
            injection_mode: InjectionMode::Unicode,
            conv_mode: crate::state::ConvModeMgr::default(),
            ime_mode_fsm: std::cell::RefCell::new(crate::tsf::ime_mode_fsm::ImeModeFsm::new()),
            ime_mode_focus_gen: std::cell::Cell::new(0),
            ms_ime_gate_give_up: std::cell::Cell::new(false),
            confirm_gate_deadline_override_ms: std::cell::Cell::new(0),
            shift_conv_guard_gen: std::cell::Cell::new(0),
            observe_unicode_literal: std::sync::atomic::AtomicBool::new(false),
            conv_mutation_allowed: std::cell::Cell::new(false),
            runtime_outbox: std::cell::RefCell::new(crate::runtime::outbox::RuntimeOutbox::new()),
            pending_drain_before_send_flush: std::cell::Cell::new(0),
        }
    }

    /// Output が蓄積した `RuntimeRequest` を全件取り出す。
    ///
    /// Runtime がキー処理境界（`WM_EXECUTE_EFFECTS` / `WM_DRAIN_OUTPUT_QUEUE` 末尾）で呼び、
    /// 各リクエストを実行する。H-4-b で push 側が配線されるまでは常に空を返す。
    pub(crate) fn take_pending_requests(&self) -> Vec<crate::runtime::outbox::RuntimeRequest> {
        self.runtime_outbox.borrow_mut().drain()
    }

    /// ADR-128: drain-before-send が実際に flush した `vk_count` を Platform
    /// へ引き渡す（`JournalEntry` への変換は呼び出し元が行う——`output`/
    /// `tsf` は `crate::journal` を直接参照しない、上記フィールド doc 参照）。
    /// 0 は「今回は flush しなかった」を表す。
    pub(crate) fn take_pending_drain_before_send_flush(&self) -> usize {
        self.pending_drain_before_send_flush.replace(0)
    }

    /// conv mutation（`send_eager_tsf_warmup`・`ImmSetConversionStatus` 等）の許可フラグを更新する。
    ///
    /// `Platform::set_conv_mode_authority` が `ConvModeAuthority::allows_conv_mutation()` の結果を push する。
    pub(crate) fn set_conv_mutation_allowed(&self, allowed: bool) {
        self.conv_mutation_allowed.set(allowed);
    }

    /// 次の Unicode モード Romaji 送信後に GJI write 観測を行うようリクエストする。
    ///
    /// `Platform::send_keys` が Unicode モード + 未学習クラスのときに呼ぶ。
    pub(crate) fn request_unicode_observation(&self) {
        self.observe_unicode_literal
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    // ── Unicode cold-start warmup ────────────────────────────────────────────

    /// フォーカス変更時に Runtime から呼ばれ、注入モードを更新する。
    pub(crate) const fn update_injection_mode(&mut self, mode: InjectionMode) {
        self.injection_mode = mode;
    }

    // ── GjiFsm ヘルパー ─────────────────────────────────────────────────────

    /// GjiFsm にイベントを送り、Response を返す（`WindowsPlatform::dispatch_gji_response` に渡す）。
    pub(crate) fn gji_on_event(
        &self,
        event: crate::tsf::gji_fsm::GjiEvent,
    ) -> timed_fsm::Response<crate::tsf::gji_fsm::GjiAction, crate::tsf::gji_fsm::GjiTimer> {
        self.warmup_coord.gji_on_event(event)
    }

    pub(crate) fn gji_state_label(&self) -> String {
        self.warmup_coord.gji_state_label()
    }

    /// `OnComposing` 状態の現在 epoch を返す。`EndComposition` イベント送信に使う。
    /// `OnComposing` 以外の状態では `None`。
    pub(crate) fn gji_current_composition_epoch(&self) -> Option<crate::tsf::gji_fsm::FocusEpoch> {
        self.warmup_coord.gji_current_composition_epoch()
    }

    // ── ImeModeFsm ヘルパー ─────────────────────────────────────────────────────

    /// `IMC_GETCONVERSIONMODE` の結果を `ImeModeFsm` に反映する。
    ///
    /// `spawn_local` 内の async ポーリングタスクから `with_app(|runtime| runtime.platform.output.update_ime_mode_from_imc(conv))` で呼ぶ。
    pub(crate) fn update_ime_mode_from_imc(&self, mode: Option<u32>) {
        self.ime_mode_fsm.borrow_mut().on_conversion_mode_read(mode);
    }

    /// `IMC_GETCONVERSIONMODE` の結果を `ImeModeFsm` へ「参考値」として反映する（BUG-59）。
    ///
    /// `update_ime_mode_from_imc` と異なり `confirmed` を立てない
    /// （`ImeModeFsm::on_conversion_mode_hint` 参照）。FocusChange 直後の
    /// cold 判定用ポーリングなど、「安全に送信してよい」という確認ではない
    /// 呼び出し元から使うこと。
    pub(crate) fn update_ime_mode_hint_from_imc(&self, mode: Option<u32>) {
        self.ime_mode_fsm.borrow_mut().on_conversion_mode_hint(mode);
    }

    /// フォーカス変更時に呼ぶ。VK_IME_ON/OFF 直後の副作用 FocusChange かを判定して適切にリセット。
    ///
    /// 世代カウンタ `ime_mode_focus_gen` をインクリメントすることで、
    /// 以前の `spawn_local` IMC ポーリングが古いフォーカスの結果を書き込まないよう保護する。
    pub(crate) fn on_ime_mode_focus_changed(&self) {
        let now_ms = crate::hook::current_tick_ms();
        self.ime_mode_fsm.borrow_mut().on_focus_changed(now_ms);
        self.ime_mode_focus_gen
            .set(self.ime_mode_focus_gen.get().wrapping_add(1));
        // 新しいフォーカス先では IMC が読める可能性があるため give-up latch を解除する。
        self.ms_ime_gate_give_up.set(false);
        // ADR-084（BUG-49 追補2、Opus レビュー指摘2）: フォーカス変更は
        // shift-conv-guard の hold が想定する「同一ウィンドウ内で完結する」
        // 前提が崩れたことを意味する。Shift の KeyUp がフックに届かないまま
        // 別ウィンドウ/ロック画面等へ遷移した場合の取りこぼしに備え、
        // confirm-gate の override も併せて解除する。
        self.confirm_gate_deadline_override_ms.set(0);
        // pass-5 レビュー指摘: このクリアだけでは、まだ走行中の
        // `kp_restore_kana_from_half_width` リトライループが次の試行で
        // override を再設定してしまい実効性が無い。`shift_conv_guard_gen` も
        // 併せてインクリメントし、そのループの `owner_gen` を無効化することで
        // クリアを恒久化する（旧フォーカス向けの conv write 自体も止まる）。
        self.bump_shift_conv_guard_gen();
    }

    // ── shift-conv-guard confirm-gate override（ADR-084 BUG-49 追補2）───────────

    /// `shift_conv_guard_gen` を新しい値に進め、直前までの世代を「所有権を
    /// 失った」ものとする。以下の 4 箇所で呼ぶ:
    /// 1. 新しい hold の開始（`kp_shift_conv_guard_key_down` の MS-IME entry 分岐）。
    /// 2. 同関数の早期 return 分岐（かな入力コンテキスト前提が崩れた場合）。
    /// 3. フォーカス変更（`on_ime_mode_focus_changed`）。
    /// 4. `SetOpen(true)` 適用（`platform.rs`）。
    pub(crate) fn bump_shift_conv_guard_gen(&self) -> u32 {
        let next = self.shift_conv_guard_gen.get().wrapping_add(1);
        self.shift_conv_guard_gen.set(next);
        next
    }

    /// `owner_gen` が現在の `shift_conv_guard_gen` と一致する場合のみ
    /// `confirm_gate_deadline_override_ms` を `until_ms` に書き込む。
    ///
    /// 一致しない（`owner_gen` を捕獲した後に別の hold が始まった／フォーカスが
    /// 変わった）場合は何もせず `false` を返す。`kp_restore_kana_from_half_width`
    /// の detached retry task が、自分より新しい hold の override を誤って
    /// 延長・上書きしないためのガード（pass-5 レビュー指摘、blocking）。
    pub(crate) fn extend_confirm_gate_override(&self, owner_gen: u32, until_ms: u64) -> bool {
        if self.shift_conv_guard_gen.get() != owner_gen {
            return false;
        }
        self.confirm_gate_deadline_override_ms.set(until_ms);
        true
    }

    /// `owner_gen` が現在の `shift_conv_guard_gen` と一致する場合のみ
    /// `confirm_gate_deadline_override_ms` を `0`（上書きなし）に戻す。
    ///
    /// 一致しない場合は何もしない — 既に次の hold が override を所有して
    /// いる可能性があり、それを誤ってクリアしてはならない（pass-5 レビュー
    /// 指摘、blocking。この不一致無視こそが本ガードの主目的）。
    pub(crate) fn clear_confirm_gate_override(&self, owner_gen: u32) {
        if self.shift_conv_guard_gen.get() == owner_gen {
            self.confirm_gate_deadline_override_ms.set(0);
        }
    }

    // `start_ms_ime_ready_poll`（BUG-13 の IMC 確認ポーリング）は spawn_local 内で
    // with_app を使うため、layer-boundaries B-1 の ALLOW 対象である `probe_io.rs` にある。

    /// GjiFsm に LongIdle タイムアウトを送り、Response を返す。
    pub(crate) fn gji_on_long_idle(
        &self,
    ) -> timed_fsm::Response<crate::tsf::gji_fsm::GjiAction, crate::tsf::gji_fsm::GjiTimer> {
        self.warmup_coord.gji_on_long_idle()
    }

    /// `GjiAction::StartProbe` を受信したとき probe_id を記録する。
    pub(crate) fn gji_store_probe_id(&self, id: crate::tsf::gji_fsm::ProbeId) {
        self.warmup_coord.store_probe_id(id);
    }

    /// `GjiAction::StartProbe` の forces_prepend_f2 / is_long_cold を記録する。
    ///
    /// `send_romaji_as_tsf` が `GjiWarmupCoro::new` を生成する際に参照する。
    /// GjiFsm の `Authorized` 状態から `ProbeParams` を読み出す。
    ///
    /// `Authorized` でない場合は `None` を返す。
    pub(crate) fn gji_current_probe_params(&self) -> Option<crate::tsf::gji_fsm::ProbeParams> {
        self.warmup_coord.current_probe_params()
    }

    /// 現在の GJI probe_id を返す（確認用、消費しない）。
    pub(crate) fn gji_current_probe_id(&self) -> Option<crate::tsf::gji_fsm::ProbeId> {
        self.warmup_coord.current_probe_id()
    }

    /// GJI probe の OUTPUT_GATE ガードを開始する。
    ///
    /// `send_romaji_as_tsf` の cold パスで `GjiWarmupCoro::new` を呼ぶ直前に使う。
    pub(crate) fn gji_begin_probe_guard(&self) {
        self.warmup_coord.begin_probe_guard();
    }

    /// GJI probe の OUTPUT_GATE ガードを解放する。
    ///
    /// `step_probe` 完了時 / `CancelProbe` 時に呼ぶ。
    pub(crate) fn gji_end_probe_guard(&self) {
        self.warmup_coord.end_probe_guard();
    }

    /// `pending_gji_key_responses` を全件取り出す。
    ///
    /// Platform の `send_keys` が呼び出し、タイマー操作（LongIdle リセット等）を実行する。
    /// Vec で返すのは、1回の send_keys で複数文字を送る場合に全 Response を保存するため。
    pub(crate) fn drain_pending_gji_key_responses(
        &self,
    ) -> Vec<timed_fsm::Response<crate::tsf::gji_fsm::GjiAction, crate::tsf::gji_fsm::GjiTimer>>
    {
        self.warmup_coord.drain_key_responses()
    }

    /// 最後の `send_keys` 完了からの経過時間（ms）。
    /// 一度も送信していない場合は `u64::MAX` を返す（= 永久に in-flight でない）。
    #[must_use]
    pub fn ms_since_last_send(&self) -> u64 {
        self.composition.ms_since_last_send()
    }

    /// IME composition context をコールド状態にマークする。
    ///
    /// 次の VK / TSF composition 送信時に VK_IME_ON ウォームアップを
    /// 先行送信させる。Enter/Space/Escape の reinject・エンジン toggle 等のタイミングで呼ぶ。
    /// フォーカス変更は `on_focus_changed()` を使うこと（epoch も更新される）。
    ///
    /// # NativeF2Consumed でも eager_warmup_sent_ms をリセットする理由
    ///
    /// 物理 F2 が押された = 新しい F2 が届き TSF 初期化が再トリガーされる。FocusChange 時の
    /// タイムスタンプを保持すると「古い F2 からの経過時間」を elapsed として計算してしまう
    /// （"hoんらい" 化け: BUG-06 の派生形）。BUG-173 以降は物理 F2 の代わりの warmup を送らないので、
    /// 基準点は 0（未送信）のまま次の送信まで残る。
    ///
    pub fn mark_composition_cold(&self, reason: ColdReason) {
        self.composition.mark_composition_cold(reason);
    }

    /// 現在の composition_warm フラグを返す（`tsf_warmup` 戦略が SSOT）。
    #[must_use]
    pub fn is_composition_warm(&self) -> bool {
        self.warmup_coord.is_warm()
    }

    /// 検出した IME 種別に応じてウォームアップ戦略を切り替える。
    ///
    /// - MS-IME → `MsImeStrategy`（常に warm、probe なし）
    /// - GJI → `GjiFsm`（cold probe 機構あり、起動時と同じ）
    ///
    /// 現在の warmup 戦略が GJI 戦略（cold probe を持つ）か。false（MsImeStrategy）なら eager warmup は送らない。
    /// 物理 F2 の Suppress 判断には使わない（BUG-173: 物理 F2 は常に Allow）。
    pub(crate) fn f2_warmup_owned(&self) -> bool {
        self.warmup_coord.needs_f2_probe()
    }

    /// ADR-203 (i): `GjiFsm` が `OffCold` か。
    pub(crate) fn gji_is_off_cold(&self) -> bool {
        self.warmup_coord.is_off_cold()
    }

    /// ADR-203 (i): probe または raw recovery/reinit が実行中か（`is_probe_or_recovery_blocking(true)`
    /// と同一条件）。実行中に `ImeOn` を出すと probe_id の相関が崩れるため level 突合は行わない。
    pub(crate) fn probe_or_recovery_in_flight(&self) -> bool {
        self.is_probe_or_recovery_blocking(true)
    }

    /// `WM_IME_KIND_CHANGED` がメインスレッドで受信されたときに呼ぶこと。
    pub(crate) fn set_active_ime_kind(&self, kind: crate::tsf::observer::ActiveImeKind) {
        self.warmup_coord.set_active_ime_kind(kind);
    }

    /// フォーカスウィンドウが変わったことを通知する。
    ///
    /// `focus_epoch` をインクリメントし、前ウィンドウのウォーム状態を自動無効化する。
    /// 従来の `mark_composition_cold()` 呼び出しの代わりに使う（明示的なコールド化も同時に行う）。
    pub fn on_focus_changed(&self) {
        self.composition.on_focus_changed();
        // deferred_vks は TsfProbeData に内包されているため、
        // pending_tsf が Some の場合は probe と一緒にドロップされる。
    }

    // ── TsfGate ラッパー ──────────────────────────────────────────────────

    /// フォーカス変更時に `tsf_gate` を `PendingWarmup` に遷移させる。
    ///
    /// 呼び出し後に `TIMER_TSF_GATE` を `WARMUP_TIMEOUT_MS` ms でセットすること。
    ///
    /// Chrome/Edge は複数の focus イベントを連続発生させる（タブ・アドレスバー・コンテンツ等）。
    /// すでに `PendingWarmup` 中なら `on_focus_change()` を呼ばず held バッファを保持する。
    /// 呼び出し元がタイマーをリセットするため warmup 期間は延長されるが、
    /// Ctrl+T 等のショートカットが複数回のフォーカスイベントで消去されることを防ぐ。
    pub fn on_focus_change_tsf(&mut self) {
        if self.tsf_gate.state() == crate::tsf::TsfGateState::PendingWarmup {
            tracing::debug!(
                "[tsf-gate] focus change while PendingWarmup — held バッファを保持して再初期化スキップ (Chrome等の連続フォーカスイベント対策)"
            );
            return;
        }
        self.tsf_gate.on_focus_change();
    }

    /// TSF モード確定時に `tsf_gate` を `Probing` に遷移させ、保留キーを返す。
    ///
    /// 呼び出し後に `TIMER_TSF_GATE` を kill すること。
    #[must_use]
    pub(crate) fn confirm_tsf(&mut self) -> Vec<awase::types::RawKeyEvent> {
        self.tsf_gate.on_tsf_confirmed()
    }

    /// 非 TSF モード確定時に `tsf_gate` を `Bypass` に遷移させ、保留キーを返す。
    ///
    /// 呼び出し後に `TIMER_TSF_GATE` を kill すること。
    #[must_use]
    pub(crate) fn bypass_tsf(&mut self) -> Vec<awase::types::RawKeyEvent> {
        self.tsf_gate.on_bypass()
    }

    /// `TIMER_TSF_GATE` タイムアウト時に呼ぶ。`Bypass` にフォールバックし、保留キーを返す。
    #[must_use]
    pub fn on_tsf_warmup_timeout(&mut self) -> Vec<awase::types::RawKeyEvent> {
        self.tsf_gate.on_warmup_timeout()
    }

    /// キーを `tsf_gate` で処理する。`true` = 保留（呼び出し元は Consumed を返すこと）。
    pub fn try_hold_key(&mut self, event: awase::types::RawKeyEvent) -> bool {
        self.tsf_gate.try_hold(event)
    }

    /// TSF プローブ完了時に `tsf_gate` を `Probing` → `Ready` に遷移させる。
    pub(crate) fn on_tsf_probe_ready(&mut self) {
        self.tsf_gate.on_ready();
    }

    /// 現在のフォーカス先が TSF 注入モードかどうかを返す。
    ///
    /// TSF モード（WezTerm 等）では物理 F2 の扱いが特殊なため、
    /// executor がこのメソッドで判定してキー処理を切り替える。
    #[must_use]
    pub fn is_tsf_mode(&self) -> bool {
        self.injection_mode == InjectionMode::Tsf
    }

    /// BUG-25 GJI 用の「IME-ON 半角英数」entry/exit トグルを送信する。
    ///
    /// 呼び出し元が `effective_open()` を評価して `ime_open` に渡す。Output は
    /// `ImeModel` への参照を持たないため、ここで belief を自前評価しない。
    /// `prepend_synthetic_shift_up` は呼び出し元にそのまま委譲する（entry は
    /// 常に物理左Shiftタップ起点なので true 固定、exit は
    /// `kp_restore_kana_from_half_width` の引数をそのまま伝播する）。
    /// Task 0 未完了のため settle 待ち・連続発火クールダウンはまだ実装しない。
    /// 戻り値 `false`（未送信）の場合、呼び出し元は belief を進めてはならない
    /// （INV-D）。exit 側は呼び出し元でラッチを戻す必要がある点に注意。
    #[allow(unsafe_code)]
    // SendInput ヘルパー呼び出しに必要。ゲート判定はこの関数内で完結する。
    #[must_use]
    pub fn send_gji_half_width_alnum_toggle(
        &self,
        action: HalfWidthAlnumAction,
        ime_open: bool,
        prepend_synthetic_shift_up: bool,
    ) -> bool {
        let vk = match action {
            HalfWidthAlnumAction::None => return false,
            HalfWidthAlnumAction::Enter => crate::vk::VK_DBE_ALPHANUMERIC,
            HalfWidthAlnumAction::Exit => crate::vk::VK_DBE_HIRAGANA,
        };
        if crate::hook::ime_mode_key_injection_blocked_by_modifier() {
            tracing::info!(
                "[shift-conv-guard] GJI 半角英数トグル {action:?} をスキップ \
                 (Win/Alt 押下中)"
            );
            return false;
        }
        if !ime_open {
            tracing::info!(
                "[shift-conv-guard] GJI 半角英数トグル {action:?} をスキップ \
                 (effective_open=false)"
            );
            return false;
        }
        if matches!(action, HalfWidthAlnumAction::Enter) {
            // ADR-107 決定5は当初「Composition/候補表示中は発火せずラッチも
            // しない」だったが、決定2の正しい注入方式（IME_KANJI_MARKER付き
            // SendInput）で実機検証した結果（known-bugs.md BUG-25追補5・
            // ユーザー確認2026-08-27）非破壊・成功が再現したため、決定5を
            // 緩和しComposition中も発火させる。preedit破壊の兆候が実機で
            // 出た場合はここにガードを復活させること。
            tracing::debug!(
                "[shift-conv-guard] GJI 半角英数 entry \
                 (composition_active={} candidate_visible={})",
                crate::tsf::observer::ime_composition_active_now(),
                crate::tsf::observer::gji_candidate_visible_now()
            );
        }
        // SAFETY: `send_ime_mode_key_with_shift_release_prefix` は SendInput のみを行う。
        unsafe {
            crate::ime::send_ime_mode_key_with_shift_release_prefix(vk, prepend_synthetic_shift_up)
        }
    }

    /// `send_keys` 完了時刻を記録する内部ヘルパー。
    fn mark_send(&self) {
        self.composition.update_last_send_ms();
    }

    /// VK/TSF 出力後に「最終キー活動時刻」を同期更新する。
    ///
    /// SendInput 後の hook 通知はメッセージループで非同期処理されるため、
    /// 直後に IME ポーリングが走ると `last_hook_activity_ms` が更新前のまま
    /// アイドル判定を通過してしまう。送信直後に同期更新することで
    /// アイドルタイマーが正しくリセットされる。
    ///
    /// `with_app` は `execute_one` からの再入 UB を避けるため使用不可。
    /// グローバル atomic に書き込み、読み取り側で `last_hook_activity_ms` と max を取る。
    fn mark_vk_output() {
        crate::tsf::probe_bridge::OUTPUT_GATE.mark_vk_output(crate::hook::current_tick_ms());
    }

    /// アクション列を順に実行する
    ///
    /// 注入モードは `resolve_injection_mode()` で決定:
    /// - Unicode: Win32/UWP デフォルト。Unicode 直接注入で IME をバイパス。
    /// - Vk: Chrome/Edge/Electron。Batched VK で IME composition。
    /// - Tsf: WezTerm 等。Sequential VK で TSF/IME に composition させる。
    // 注入モード(Unicode/Vk/Tsf)ごとの分岐が本質的に多いディスパッチャ。分割は挙動変更
    // リスクが高いため、複雑度警告のみ抑制する。
    #[expect(clippy::cognitive_complexity)]
    pub fn send_keys(&self, actions: &[KeyAction]) {
        // モード解決 + OutputActiveGuard 取得をセッションオブジェクトに委譲
        let session = OutputSession::begin(self);

        // mark_send() より前に elapsed を読む。mark_send() は last_send_ms を上書きするため、
        // 内部の send_romaji_as_tsf 等での ms_since_last_send() は常に ~0ms を返す。
        // 真の「前回送信からの経過時間」はここで記録する。
        let prev_elapsed_ms = self.ms_since_last_send();
        tracing::debug!(
            "send_keys: mode={:?} actions={actions:?} prev_elapsed={}ms",
            session.mode,
            fmt_ms(prev_elapsed_ms)
        );

        // NOTE: ImeDiagnosticSnapshot::capture("send_keys_pre") をここに置いてはいけない。
        // capture() は内部で GetGUIThreadInfo(100ms) + SendMessageTimeoutW(50ms×2) を
        // 呼ぶため、send_keys の中でメッセージポンプが走り Space 等の WH_KEYBOARD_LL
        // コールバックが SendInput より前に発火して "境界dえ" 等の race を起こす。

        // output in-flight guard の基準点を SendInput より前に設定する。
        self.mark_send();

        let sender = session.sender();
        for action in actions {
            match action {
                KeyAction::SpecialKey(sk) => {
                    tracing::debug!("  → SpecialKey({sk:?}) vk=0x{:02X}", special_key_to_vk(*sk));
                    self.injector.send_key(special_key_to_vk(*sk), false);
                }
                KeyAction::Key(vk) => {
                    tracing::debug!("  → Key({vk:#06X})");
                    self.injector.send_key(*vk, false);
                }
                KeyAction::KeyUp(vk) => {
                    tracing::debug!("  → KeyUp({vk:#06X})");
                    self.injector.send_key(*vk, true);
                }
                KeyAction::Char(ch) => {
                    tracing::debug!("  → Char('{ch}') via {}", sender.mode_label());
                    sender.send_char(*ch);
                }
                KeyAction::Suppress => {
                    tracing::debug!("  → Suppress");
                }
                KeyAction::Romaji(s) => {
                    tracing::debug!("  → Romaji(\"{s}\") via {}", sender.mode_label());
                    sender.send_romaji(s);
                    // Unicode モードで未学習クラスの場合、GJI write を観測して事後昇格を判断する。
                    // observe_unicode_literal フラグは Platform が request_unicode_observation() でセット。
                    // 最初の Romaji 送信時に 1 回だけ消費する（複数文字を 1 回の send_keys で送る場合も 1 度のみ）。
                    if self
                        .observe_unicode_literal
                        .swap(false, std::sync::atomic::Ordering::Relaxed)
                        && self.injection_mode == InjectionMode::Unicode
                        && !self.warmup_coord.has_pending_tsf()
                    {
                        use crate::tsf::ime_mode_fsm::ImeModeState;
                        let ime_state = self.ime_mode_fsm.borrow().state();
                        if matches!(ime_state, ImeModeState::Hiragana | ImeModeState::Katakana) {
                            let baseline = crate::tsf::observer::gji_write_bytes();
                            let cold_seq = self.composition.cold_start_count();
                            tracing::debug!(
                                "[unicode-obs] cold={cold_seq} Unicode Romaji 送信後に GJI write 観測開始 \
                                (baseline={baseline})",
                                cold_seq = cold_seq.value(),
                            );
                            self.install_pending_tsf(Box::new(
                                crate::tsf::warmup::unicode_literal_observer::UnicodeLiteralObserverFsm::new(
                                    baseline, cold_seq,
                                ),
                            ));
                        }
                    }
                }
                KeyAction::KeySequence(s) => {
                    tracing::debug!("  → KeySequence(\"{s}\") via {}", sender.mode_label());
                    sender.send_key_sequence(s);
                }
                KeyAction::CtrlChord(vk) => {
                    tracing::debug!("  → CtrlChord(Ctrl+{vk:#06X})");
                    self.injector.send_ctrl_chord(*vk);
                }
                KeyAction::Sequence(items) => {
                    // flatten_actions（ADR-115 決定5）により、ここへ到達する
                    // 前に Sequence は decide()/build_response()/flush_pending
                    // の出口で全て平坦化されているはずだが、防御的に
                    // 同じループ内でその場展開する（再帰呼び出しにすると
                    // OutputSession/mark_send を二重に開いてしまうため、
                    // 新しい send_keys() 呼び出しは行わない）。
                    tracing::error!(
                        "[output] 未平坦化の Sequence が send_keys に到達した \
                         — flatten_actions の呼び出し漏れ: {items:?}"
                    );
                    for it in items {
                        match it {
                            KeyAction::SpecialKey(sk) => {
                                self.injector.send_key(special_key_to_vk(*sk), false);
                            }
                            KeyAction::Char(ch) => sender.send_char(*ch),
                            KeyAction::KeySequence(s) => sender.send_key_sequence(s),
                            KeyAction::CtrlChord(vk) => self.injector.send_ctrl_chord(*vk),
                            KeyAction::Suppress | KeyAction::Sequence(_) => {}
                            KeyAction::Key(vk) => self.injector.send_key(*vk, false),
                            KeyAction::KeyUp(vk) => self.injector.send_key(*vk, true),
                            KeyAction::Romaji(s) => sender.send_romaji(s),
                        }
                    }
                }
            }
        }

        // VK/TSF モードで出力した場合、直後の IME ポーリングをガードするため
        // タイムスタンプを記録する（母音落ち「て→tえ」防止）。
        if session.is_vk_mode() {
            Self::mark_vk_output();
        }

        // executor が「output in-flight」判定に使う送信時刻を記録する。
        self.mark_send();
        // session ここで Drop → OutputActiveGuard::drop() → OUTPUT_GATE.active=false + drain
    }

    /// composition の温度状態を評価する。
    #[must_use]
    pub(super) fn assess_warmth(&self) -> WarmthContext {
        // 判断は `warm_send_plan::plan_warmth`（FCIS F6b）。読むのは副作用の無い Cell/RefCell だけ。
        let warm = self.is_composition_warm();
        let elapsed = self.ms_since_last_send();
        let plan = warm_send_plan::plan_warmth(warm_send_plan::WarmthFacts {
            warm,
            elapsed_ms: elapsed,
            needs_f2_probe: self.warmup_coord.needs_f2_probe(),
            composition_timeout_ms: crate::tuning::COMPOSITION_TIMEOUT_MS,
        });
        tracing::trace!("[warmth] reason={:?}", plan.reason);
        WarmthContext {
            warm,
            elapsed,
            session_expired: plan.session_expired,
            prepend_f2_warmup: plan.prepend_f2_warmup,
        }
    }

    /// probe 進行中、または give-up 由来の raw recovery/reinit retry が
    /// romaji/backspace を予約中なら、romaji を VK 列に変換して deferred_vks に
    /// 追記し true を返す。どちらでもなければ何もせず false を返す。
    ///
    /// ADR-123 変更A: 旧来は `has_pending_tsf()`（TSF probe FSM の在/不在）だけを
    /// 見ていたため、give-up からの reinit-retry が `Scheduled`/`Polling` の間に
    /// 届いた別モーラは「probe は in-flight ではない」と誤判定され、それ自身の
    /// 独立した probe を開始して追い越すことがあった（BUG-74 追補3、
    /// `report_id: 01M1KEGZ081YHJ1T2NC765SYYH`）。`raw_recovery_owns_deferred()`
    /// も条件に加えることでこの追い越しを防ぐ。
    pub(super) fn defer_if_probe_in_flight(&self, romaji: &str, origin: DeferredOrigin) -> bool {
        self.defer_if_probe_or_recovery_in_flight(romaji, origin, true)
    }

    /// `defer_if_probe_in_flight` から `raw_recovery_owns_deferred()` の条件だけを
    /// 除いた版。`send_romaji_batched_bypass_gate`/`send_romaji_as_tsf_bypass_gate`
    /// （raw recovery 回収再送・ADR-101 決定3 retry 専用）が使う。
    ///
    /// `raw_recovery_owns_deferred()` は「今まさに自分が処理している recovery か」
    /// と「無関係などこか別の recovery が in-flight か」を区別できないグローバル
    /// 述語であり、これらの経路自身の送信にそのまま適用すると自己 defer を
    /// 起こす（詳細は `send_romaji_batched_bypass_gate` の doc コメント参照）。
    /// `has_pending_tsf()`（無関係な別 probe が実際に走っているかどうか）は
    /// この2経路にも従来どおり適用してよい——こちらは自己参照を起こさない
    /// 独立した観測であり、この gate 自体は PR4 以前から存在していた
    /// （挙動を後退させない）。
    pub(super) fn defer_if_probe_in_flight_recovery_exempt(
        &self,
        romaji: &str,
        origin: DeferredOrigin,
    ) -> bool {
        self.defer_if_probe_or_recovery_in_flight(romaji, origin, false)
    }

    /// probe/recovery が進行中か（`probe_or_recovery_block_reason` の bool 版。ADR-203 の参照用）。
    pub(super) fn is_probe_or_recovery_blocking(&self, check_raw_recovery: bool) -> bool {
        self.probe_or_recovery_block_reason(check_raw_recovery)
            .is_some()
    }

    /// 判断は `deferred_gate_plan::plan_blocking`（FCIS F6）。`raw_recovery_owns_deferred()` は元の短絡評価と同じく
    /// 必要なときだけ読む。`vk_send.rs::drain_pending_deferred_before_send_if_queue_only`（ADR-123 決定4-3）は、
    /// `pending_deferred` が「queue-only」（誰も blocking していないのに非空）かを判定するためにこれを使う。
    /// `raw_recovery_owns_deferred()` の呼び出し箇所を `output/mod.rs` 内に閉じておくため（INV-F 系の集約方針、
    /// `tests/architecture_guard.rs::raw_recovery_owns_deferred_call_sites_are_accounted_for` 参照）、
    /// `vk_send.rs` 側から直接呼ばずこの accessor 経由にする。
    pub(super) fn probe_or_recovery_block_reason(
        &self,
        check_raw_recovery: bool,
    ) -> Option<deferred_gate_plan::BlockReason> {
        let has_pending_tsf = self.warmup_coord.has_pending_tsf();
        let raw_recovery_owns =
            deferred_gate_plan::needs_raw_recovery_read(check_raw_recovery, has_pending_tsf)
                && self.raw_recovery_owns_deferred();
        deferred_gate_plan::plan_blocking(deferred_gate_plan::BlockingFacts {
            has_pending_tsf,
            check_raw_recovery,
            raw_recovery_owns,
        })
    }

    fn defer_if_probe_or_recovery_in_flight(
        &self,
        romaji: &str,
        origin: DeferredOrigin,
        check_raw_recovery: bool,
    ) -> bool {
        let vks: Vec<(VkCode, bool)> = romaji.chars().filter_map(ascii_to_vk).collect();
        self.defer_vks_if_probe_or_recovery_in_flight(&vks, origin, check_raw_recovery, romaji)
    }

    /// `defer_if_probe_or_recovery_in_flight` の実体。romaji 文字列からの
    /// VK 変換を終えた後の共通コアで、`defer_vk_if_probe_in_flight`
    /// （単一 VK 版）とも共有する（2026-09-03 code review指摘で統合——
    /// 単一VK版が旧来 `warmup_coord.defer_vks_if_in_flight` を直接呼び、
    /// `raw_recovery_owns_deferred()`も件数上限もどちらも経由しない
    /// 状態だった。BUG-47により現状は到達不能だが「理論上到達不能」という
    /// 主張自体が過去に誤りだったことがある経路のため、片方だけ保護する
    /// 非対称を解消した）。
    ///
    /// `log_desc` はログ表示専用（romaji文字列、または単一VKの説明）。
    fn defer_vks_if_probe_or_recovery_in_flight(
        &self,
        vks: &[(VkCode, bool)],
        origin: DeferredOrigin,
        check_raw_recovery: bool,
        log_desc: &str,
    ) -> bool {
        let blocking = self.probe_or_recovery_block_reason(check_raw_recovery);
        // 件数上限は進行中のときだけ読む（元の読む回数と同じ）。
        let would_exceed_cap =
            blocking.is_some() && self.warmup_coord.would_exceed_deferred_cap(vks.len());
        match deferred_gate_plan::plan_defer(blocking, would_exceed_cap) {
            deferred_gate_plan::DeferPlan::NotBlocked => return false,
            // ADR-123 変更A+C 決定4-3: 件数上限を超える場合は defer を諦め、
            // 通常送信経路（今日の挙動と同じ、probe保護なしの可能性あり）へ
            // degrade する。「最も古いエントリから強制flush」は probe の
            // per-VK confirm 中に生VKを割り込ませることになり危険なため採らない
            // （`TsfWarmupCoordinator::would_exceed_deferred_cap` の doc コメント
            // 参照）。この呼び出しが持つ VK 数（`vks.len()`）を渡して判定する
            // ——1件だけを見て許可すると、複数VKからなるromajiの一括pushで
            // 上限を超えうる（2026-09-03 code review指摘で修正）。
            deferred_gate_plan::DeferPlan::DegradeCapExceeded(reason) => {
                tracing::error!(
                    "[pending-deferred] count limit exceeded, degrading to immediate send: \
                     input={log_desc:?} origin={origin:?} vk_count={} reason={reason:?}",
                    vks.len()
                );
                return false;
            }
            deferred_gate_plan::DeferPlan::Defer(_) => {}
        }
        tracing::debug!(
            "[tsf] probe/recovery in flight → deferred {} VK(s) for {:?}",
            vks.len(),
            log_desc
        );
        self.warmup_coord.push_deferred_vks(vks, origin);
        true
    }

    /// probe/recovery 進行中なら単一 VK を deferred_vks に追記し true を返す。
    /// 進行中でなければ何もせず false を返す。
    ///
    /// 呼び出し元 (`vk_send.rs` の `send_char_as_tsf`/`send_char_as_vk`) は
    /// `CharResolution::Vk` の生 VK フォールバック経路にあり、2026-08-05 の
    /// BUG-47 追補修正で `vk_pair_to_ascii` が `build_symbol_to_vk` の全記号を
    /// カバーするようになったため、現状この2箇所は理論上到達しない
    /// （`docs/known-bugs.md` BUG-47 参照）。
    ///
    /// **`defer_if_probe_or_recovery_in_flight`（romaji版）と同じ共通コア
    /// (`defer_vks_if_probe_or_recovery_in_flight`) を使う（2026-09-03
    /// code review指摘で修正）**: 以前は`warmup_coord.defer_vks_if_in_flight`
    /// を直接呼んでおり、`raw_recovery_owns_deferred()`もPR4の件数上限
    /// （`would_exceed_deferred_cap`）もどちらも経由しなかった（旧来の
    /// has_pending_tsf()のみのgate）。BUG-47のとおり現状は理論上到達
    /// 不能だが、「理論上到達不能」という主張自体がBUG-47の履歴が示す
    /// とおり過去に誤りだったことがあるため、romaji版と同じ保護に揃えた。
    /// この呼び出し元は常に通常のユーザー入力（raw recovery回収再送・
    /// ADR-101 retryのbypass経路ではない）のため`check_raw_recovery=true`
    /// （`defer_if_probe_in_flight`と同じ、`Exempt`版は使わない）。
    pub(super) fn defer_vk_if_probe_in_flight(
        &self,
        vk: VkCode,
        needs_shift: bool,
        origin: DeferredOrigin,
    ) -> bool {
        self.defer_vks_if_probe_or_recovery_in_flight(
            &[(vk, needs_shift)],
            origin,
            true,
            &format!("{vk:?}"),
        )
    }

    /// TIMER_TSF_PROBE ハンドラから呼ぶ。probe を 1 ステップ進め、結果を返す。
    ///
    /// `WindowsPlatform::advance_tsf_probe` は `timer_cmd` を `apply_timer_command` に渡し、
    /// `gji_response` を `dispatch_gji_response` に渡す。
    /// pending_tsf の有無とタイマー kill/set の判断はここで完結する。
    pub(crate) fn step_probe(&mut self) -> StepProbeResult {
        let tick_t = crate::hook::current_tick_ms();
        let env = {
            let ime_fsm = self.ime_mode_fsm.borrow();
            crate::tsf::warmup::probe_fsm::TsfEnvSnapshot {
                is_tsf_mode: self.is_tsf_mode(),
                gji_active: crate::tsf::observer::gji_is_active_ime(),
                ime_mode: ime_fsm.state(),
                ime_mode_confirmed: ime_fsm.is_confirmed(),
                confirm_gate_deadline_override_ms: self.confirm_gate_deadline_override_ms.get(),
                deferred_pending: self.warmup_coord.has_pending_deferred(),
                gji_candidate_visible_now: crate::tsf::observer::gji_candidate_visible_now(),
                literal_session_confirmed_gen:
                    crate::tsf::observer::literal_session_confirmed_gen_snapshot(),
            }
        };

        // ── Chrome / LiteralDetect / GjiWarmup probe パス（machine は pending_tsf に格納）──
        let machine = self.warmup_coord.take_pending_tsf();
        let Some(mut machine) = machine else {
            return StepProbeResult {
                timer_cmd: TimerCommand::Kill {
                    id: crate::TIMER_TSF_PROBE,
                },
                gji_response: None,
                needs_gji_composition_reset: false,
                learned_tsf: false,
                completed_cold_seq: None,
                literal_detect: crate::tsf::literal_facts::LiteralDetectTrace::default(),
            };
        };
        let cold_seq = machine.cold_seq_hint().value();
        tracing::debug!(
            "[tsf-probe-tick] cold={} t={}ms",
            machine.cold_seq_hint().value(),
            tick_t
        );
        let actions = machine.tick(env);
        let mut literal_detect = crate::tsf::literal_facts::LiteralDetectTrace::default();
        let dispatch =
            probe_io::dispatch_probe_actions(machine.as_mut(), actions, self, &mut literal_detect);
        match dispatch {
            probe_io::DispatchResult::Continue => {
                let needs_gji_composition_reset = self.warmup_coord.take_composition_reset();
                self.warmup_coord.restore_pending_tsf(machine);
                StepProbeResult {
                    timer_cmd: TimerCommand::Continue {
                        id: crate::TIMER_TSF_PROBE,
                        delay: Duration::from_millis(10),
                    },
                    gji_response: None,
                    needs_gji_composition_reset,
                    learned_tsf: false,
                    completed_cold_seq: None,
                    literal_detect,
                }
            }
            probe_io::DispatchResult::Ended(end) => {
                // `machine` はここで drop される（restore しない）＝段の終わり。
                let learned_tsf = end.reason == crate::tsf::gji_fsm::StageEndReason::UpgradedToTsf;
                drop(machine);
                let needs_gji_composition_reset = self.warmup_coord.take_composition_reset();
                let gji_response = self.finish_probe_stage(end);
                StepProbeResult {
                    timer_cmd: TimerCommand::Kill {
                        id: crate::TIMER_TSF_PROBE,
                    },
                    gji_response,
                    needs_gji_composition_reset,
                    learned_tsf,
                    completed_cold_seq: Some(cold_seq),
                    literal_detect,
                }
            }
        }
    }

    /// deferred VK の解放権が raw literal 回収 / GJI reinit retry 側にあるか（INV-F）。
    ///
    /// `flush_raw_tsf_literal_recovery` は末尾で必ず
    /// `flush_stale_deferred_vks_after_recovery` を通り、`WM_DRAIN_OUTPUT_QUEUE`
    /// ハンドラから無条件に呼ばれる。BUG-38 の順序（backspace / romaji 再送 /
    /// reinit がすべて実送信されたあとでなければ deferred を出してはいけない）は
    /// この経路が守る。段末（`finish_probe_stage`）はこの間 deferred に触れない。
    // reinit の pending を見ていた頃の名残で `self` を使わなくなった（ADR-212 P3）。呼び出し箇所の件数を固定する
    // `architecture_guard` があるので、メソッドの形は変えない。
    #[expect(clippy::unused_self)]
    fn raw_recovery_owns_deferred(&self) -> bool {
        use std::sync::atomic::Ordering::Relaxed;
        crate::RAW_TSF_LITERAL.backs.load(Relaxed) != 0
            || !crate::RAW_TSF_LITERAL
                .romaji
                .lock()
                .expect("RAW_TSF_LITERAL.romaji mutex poisoned")
                .is_empty()
    }

    /// probe 段が終わったときに必ず1回だけ通る後始末（ADR-103 決定4-e）。
    ///
    /// 呼び出し元は `step_probe` の `Ended` アームただ1つ（machine が drop される
    /// 唯一の点）。`cancel_probe` は別途、段を畳んで捨てる形で同じ資源を後始末する。
    fn finish_probe_stage(
        &mut self,
        end: probe_io::StageEnd,
    ) -> Option<timed_fsm::Response<crate::tsf::gji_fsm::GjiAction, crate::tsf::gji_fsm::GjiTimer>>
    {
        // (a) deferred VK の解放。所有権が raw literal 回収側にある間は触らない（INV-F）。
        if self.raw_recovery_owns_deferred() {
            tracing::debug!(
                "[stage-end] {:?}: deferred の解放は raw recovery 側に委ねる",
                end.reason
            );
        } else {
            let n = self.flush_pending_deferred_vks();
            if n > 0 {
                tracing::debug!("[stage-end] {:?}: deferred {n} VK(s) を flush", end.reason);
            }
        }
        // (c) TsfGate / OUTPUT_GATE ガード。deferred を送り切ってからゲートを開ける。
        self.on_tsf_probe_ready();
        self.gji_end_probe_guard();
        // (b) GjiFsm への通知。
        let rec = self.warmup_coord.take_stage_record();
        let probe_id = self.warmup_coord.take_probe_id()?;
        Some(self.gji_on_event(if rec.injected && !rec.recovered {
            crate::tsf::gji_fsm::GjiEvent::WarmupComplete { probe_id }
        } else {
            crate::tsf::gji_fsm::GjiEvent::WarmupAborted {
                probe_id,
                reason: end.reason,
            }
        }))
    }

    /// probe を `warmup_coord` にインストールする。既存 probe があれば上書きして warn を出す。
    ///
    /// [`TsfWarmupCoordinator::install_pending_tsf`] への Facade。暗黙のキャンセルを
    /// ログに残し、バグ調査を容易にする。
    pub(super) fn install_pending_tsf(
        &self,
        machine: Box<dyn crate::tsf::warmup::tickable_fsm::TickableFsm>,
    ) {
        self.warmup_coord.install_pending_tsf(machine);
    }

    /// Chrome/LiteralDetect/GjiWarmup probe が実行中なら継続タイマー命令を返す。
    ///
    /// `send_keys` 完了後の補完に使う。
    pub(crate) fn pending_tsf_timer(&self) -> Option<TimerCommand> {
        self.warmup_coord.pending_tsf_timer()
    }

    /// `send_keys()` が開始した TSF/GJI probe がまだ完了していないか。
    pub(crate) fn has_pending_tsf_work(&self) -> bool {
        self.warmup_coord.has_pending_tsf()
    }

    /// `pending_deferred`（probe 実行中に届いた別モーラの VK 退避キュー）の
    /// 現在の長さ（ADR-123、診断用）。
    pub(crate) fn pending_deferred_len(&self) -> usize {
        self.warmup_coord.pending_deferred_len()
    }

    /// GJI probe をキャンセルし、OUTPUT_GATE ガードを解放する。
    ///
    /// `GjiAction::CancelProbe` ハンドラが呼ぶ。内部で以下を一括実行する:
    /// 1. `pending_tsf` をクリア
    /// 2. OUTPUT_GATE ガードを解放
    /// 3. `current_gji_probe_id` をクリア
    ///
    /// 呼び出し元は続けて `TIMER_TSF_PROBE` を kill すること（タイマー操作は platform の責務）。
    pub(crate) fn cancel_probe(&self) {
        self.warmup_coord.clear_pending_tsf();
        self.gji_end_probe_guard();
        let _ = self.warmup_coord.take_probe_id();
        let _ = self.warmup_coord.take_stage_record();
        // ADR-103 決定4-f: cancel_probe が発火するのは ImeOff / FocusChange /
        // handle_composition_reset の3経路だけで、これは GjiFsm の pending
        // （同じ打鍵の romaji 影）を破棄する経路と完全に同じ集合である。片方だけ
        // 残すと shadow と実体がずれ、残った VK は「誰にも所有されないまま、
        // はるか後の無関係な回収でまとめて送られる」——BUG-27 の順序反転になる。
        let discarded = self.warmup_coord.take_pending_deferred();
        if !discarded.is_empty() {
            tracing::warn!(
                "[stage-cancel] deferred {n} VK(s) を破棄（宛先窓が変わった / エンジン停止）",
                n = discarded.len()
            );
        }
    }

    /// `warmup_coord` の composition reset フラグを取り出す。
    ///
    /// `SymbolVkSent` 等の VK 記号送信直後に `send_char_as_tsf` が立てたフラグを
    /// `platform.rs::send_keys` が drain して `gji_on_composition_reset` を呼ぶために使う。
    pub(crate) fn take_composition_reset(&self) -> bool {
        self.warmup_coord.take_composition_reset()
    }
}

impl awase::platform::CompositionOutput for Output {
    fn send_romaji(&self, romaji: &str) {
        match self.injection_mode {
            InjectionMode::Vk => self.send_romaji_batched(romaji),
            InjectionMode::Tsf => self.send_romaji_as_tsf(romaji),
            InjectionMode::Unicode => self.send_romaji_as_unicode(romaji),
        }
    }

    fn send_kana_char(&self, ch: char) {
        self.send_char_as_tsf(ch);
    }

    fn is_composition_warm(&self) -> bool {
        self.is_composition_warm()
    }

    fn mark_cold(&self, reason: awase::platform::PlatformColdReason) {
        use awase::platform::PlatformColdReason;
        let cold_reason = match reason {
            PlatformColdReason::FocusChange => ColdReason::FocusChange,
            PlatformColdReason::ConfirmKey => ColdReason::PassthroughConfirmKey,
            PlatformColdReason::ImeToggle => ColdReason::SetOpenTrue,
        };
        self.mark_composition_cold(cold_reason);
    }

    fn on_focus_changed(&self) {
        self.on_focus_changed();
    }
}

/// raw TSF literal 検出・回収メソッド群。
///
/// WM_DRAIN_OUTPUT_QUEUE ハンドラから呼び出す。
/// backspace 送信 → romaji 再送の順序を保証するため、drain keys より前に実行すること。
impl Output {
    /// `RAW_TSF_LITERAL` グローバルに backs / romaji / escape_composition を書き込む。
    ///
    /// `RawTsfLiteralRecovery` 処理で `consecutive == 0` のときのみ呼ぶ。
    /// `flush_raw_tsf_literal_backspaces` と `flush_raw_tsf_literal_romaji` の read 側と
    /// ここの write 側を `Output` に集約し、dispatcher が直接グローバルを触らないようにする。
    ///
    /// `escape_composition`: partial literal（candidate 表示中に一部だけ literal 化）回収時に
    /// `true`。バックスペース前に `VK_ESCAPE` を送って composition を確実に破棄する。
    #[expect(clippy::unused_self)]
    pub(crate) fn record_raw_tsf_literal(
        &self,
        backs: usize,
        romaji: String,
        escape_composition: bool,
    ) {
        use std::sync::atomic::Ordering::Relaxed;
        crate::RAW_TSF_LITERAL.backs.store(backs, Relaxed);
        crate::RAW_TSF_LITERAL
            .escape_composition
            .store(escape_composition, Relaxed);
        *crate::RAW_TSF_LITERAL
            .romaji
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = romaji;
    }

    /// WM_DRAIN_OUTPUT_QUEUE ハンドラから呼ぶ。`flush_raw_tsf_literal_backspaces` の後に呼ぶこと。
    ///
    /// `RAW_TSF_LITERAL.romaji` に退避されたローマ字を読み取り、`send_romaji_as_tsf` で再送する。
    /// cold 状態（RawTsfLiteralRecovery）で呼ばれるため warmup probe が走り正しく compose される。
    /// drain キーの前に呼ぶことで「backspace → raw TSF literal char → drain keys」の順を保証する。
    pub fn flush_raw_tsf_literal_romaji(&self) {
        let romaji = {
            let mut guard = crate::RAW_TSF_LITERAL
                .romaji
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            std::mem::take(&mut *guard)
        };
        if romaji.is_empty() {
            return;
        }
        tracing::debug!("[raw-tsf-literal] re-sending raw TSF literal romaji={romaji:?}");
        self.send_romaji_dispatching_on_gate(&romaji);
    }

    /// TSF gate の状態に応じて `romaji` を通常送信経路へ振り分ける。
    ///
    /// Bypass (Chrome) では `send_romaji_as_tsf` が GJI probe (`TransmitTarget::Tsf`) を
    /// 起動するが、Chrome は gate=Bypass のため `dispatch_probe_actions` でスキップされる。
    /// Chrome バッチパス (`TransmitTarget::Chrome`) を使うことで正しく送信できる。
    /// `flush_raw_tsf_literal_romaji`（consecutive==0 の通常リカバリ）と
    /// `resend_gji_reinit_retry_romaji`（give-up 後、reinit confirmed 後の retry、
    /// ADR-101）が共有する — コードレビュー指摘: 以前は同じ分岐が2箇所に
    /// 手書きで重複していた。
    fn send_romaji_dispatching_on_gate(&self, romaji: &str) {
        use probe_io::ProbeIo as _;
        // ADR-123 変更A+C 決定4-2: 新設した defer gate をこの2経路（raw recovery
        // 回収再送・ADR-101 決定3 retry）には適用しない。理由は
        // `send_romaji_batched_bypass_gate` の doc コメント参照。
        if self.gate_is_bypass() {
            self.send_romaji_batched_bypass_gate(romaji);
        } else {
            self.send_romaji_as_tsf_bypass_gate(romaji);
        }
    }

    /// raw TSF literal 回収を一括実行: backspace 送信 → romaji 再送 → (あれば) GJI reinit
    /// → 取り残された deferred VK flush。
    ///
    /// WM_DRAIN_OUTPUT_QUEUE ハンドラから呼ぶ。drain keys より前に実行すること。
    ///
    /// BUG-36: `pending_gji_reinit_cold_seq` の消化をここに置くのは、backspace の
    /// 実送信（`flush_raw_tsf_literal_backspaces`）より reinit（`VK_IME_OFF`→
    /// `VK_IME_ON`）が先に外へ出るのを防ぐため。`VK_IME_OFF` は未確定の preedit を
    /// commit してしまうため、reinit が先行すると commit 済みの literal 文字を
    /// backspace で確実に消せなくなる（`pending_gji_reinit_cold_seq` のフィールド
    /// doc・`docs/known-bugs.md` BUG-36 参照）。
    ///
    /// BUG-38: `flush_stale_deferred_vks_after_recovery` を最後に置くのは、
    /// backspace/romaji再送/reinit がすべて実際に SendInput された後でなければ
    /// 取り残された deferred VK を送出してはいけないため（先に送ると backspace が
    /// deferred 側の文字を巻き込んで消してしまう、`docs/known-bugs.md` BUG-38 参照）。
    pub(crate) fn flush_raw_tsf_literal_recovery(&self) -> RawRecoveryOutcome {
        flush_raw_tsf_literal_backspaces();
        self.flush_raw_tsf_literal_romaji();
        let vk_count = self.flush_stale_deferred_vks_after_recovery();
        RawRecoveryOutcome::Flushed { vk_count }
    }

    /// give-up（romaji 再送なし）で `RawTsfLiteralRecovery` が終わった場合に、
    /// `pending_deferred` に取り残された VK を送出する。
    ///
    /// `dispatch_probe_actions` の `ProbeAction::RawTsfLiteralRecovery` ハンドラは
    /// `record_raw_tsf_literal` で backspace/romaji を static に退避するだけで、
    /// `TransmitTsf`/`TransmitChrome`/`TransmitSingleVk` の各ハンドラと違って
    /// `pending_deferred` を一切 flush しない（docs/known-bugs.md 参照）。
    /// 何もしないと、probe 実行中に届いた別の打鍵の VK がこのキューに取り残されたまま
    /// 消費されず、後続の全く別の打鍵が先に probe を通過して出力順が入れ替わる
    /// （例: "とうろく" と連続入力して "と" が消え "うろ" が "ろう" に逆転する）。
    ///
    /// `flush_raw_tsf_literal_romaji` が romaji を再送した場合（consecutive==0 の
    /// 通常リカバリ）は、その再送自身が新しい probe を張るため
    /// `warmup_coord.has_pending_tsf()` が true になり、ここでは何もしない。
    /// その新しい probe の `TransmitTsf` 等のハンドラが、確認完了後に
    /// 正しい順序（再送した romaji → deferred VK）で自然に flush する。
    /// give up（romaji 再送なし）の場合のみ、ここで直接 flush する。
    ///
    /// 既知の残課題: この flush は cold-mark 直後（GJI がまだ確実に温まっていない
    /// 状態）に raw VK を probe なしで送るため、deferred 側が literal 化する
    /// リスクは理論上残る（escape_composition=true の場合は composition が
    /// ESC で丸ごと破棄された直後でもある）。probe を経由した re-entry は
    /// ADR-079 Stage2（未実装）のスコープであり、本 fix は「取り残されたまま
    /// 順序が入れ替わる」実害の解消に限定する。
    fn flush_stale_deferred_vks_after_recovery(&self) -> usize {
        let len = self.flush_pending_deferred_vks();
        if len > 0 {
            tracing::debug!(
                "[raw-tsf-literal] give-up 後に取り残されていた deferred {len} VK(s) を flush"
            );
        }
        len
    }

    /// `warmup_coord.pending_deferred`（probe実行中に届いた後続キーの退避キュー）
    /// を条件付きで取り出し、TSF gate状態に応じた marker で送信する共通コア。
    ///
    /// `flush_stale_deferred_vks_after_recovery`（raw recovery直後、`Polling`中は
    /// 呼び出し元が事前ガードする）と `flush_deferred_vks_after_gji_reinit_completion`
    /// （retry completion後）が共有する — コードレビュー指摘: 以前は
    /// take/marker選択/送信の並びが2箇所に手書きで重複していた。事前ガード・
    /// ログ文言は呼び出し元ごとに異なるためここには含めない。
    fn flush_pending_deferred_vks(&self) -> usize {
        use probe_io::ProbeIo as _;
        let Some(vks) = self.warmup_coord.take_pending_deferred_if_probe_idle() else {
            return 0;
        };
        let len = vks.len();
        // order_violation はイテレータを直接受けるため、違反が無い共通ケース
        // では中間 Vec を確保しない（2026-09-03 code review指摘で修正）。
        // ログ表示用の Vec は違反を検出した場合にのみ組み立てる。
        if let Some(violation) =
            crate::journal_policy::order_violation(vks.iter().map(|vk| vk.order_token))
        {
            // ADR-123 変更A+C（gate拡張・drain-before-send）がdevelopマージ
            // 済みのため、warn!からerror!へ昇格した。変更A+C未実装の間は
            // この順序違反が実際に頻発する既知の状態だったため、その間は
            // 意図的にwarn!に留めていた（変更E新設時のログレベル判断、
            // ADR-123変更E参照）。
            let order_tokens: Vec<u64> = vks.iter().map(|vk| vk.order_token).collect();
            tracing::error!(
                "[pending-deferred] order violation: index={} previous={} current={} tokens={:?}",
                violation.index,
                violation.previous,
                violation.current,
                order_tokens
            );
        }
        let marker = if self.gate_is_bypass() {
            VkMarker::InjectedWithScan
        } else {
            VkMarker::Tsf
        };
        self.send_deferred_vks(&vks, marker);
        len
    }
}

pub use crate::tsf::output::flush_raw_tsf_literal_backspaces;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tsf::probe_bridge::OutputActiveGuard;

    // ── ColdReason impl メソッドテスト ────────────────────────────────────────

    #[test]
    fn cold_reason_is_confirm_key() {
        assert!(ColdReason::PassthroughConfirmKey.is_confirm_key());
        assert!(ColdReason::ReinjectConfirmKey.is_confirm_key());
        assert!(!ColdReason::FocusChange.is_confirm_key());
        assert!(!ColdReason::RawTsfLiteralRecovery.is_confirm_key());
        assert!(!ColdReason::SetOpenFalse.is_confirm_key());
    }

    #[test]
    fn cold_reason_requires_settle() {
        assert!(ColdReason::FocusChange.requires_settle());
        assert!(ColdReason::NativeF2Consumed.requires_settle());
        assert!(ColdReason::SetOpenTrue.requires_settle());
        assert!(!ColdReason::PassthroughConfirmKey.requires_settle());
        assert!(!ColdReason::RawTsfLiteralRecovery.requires_settle());
        assert!(!ColdReason::SetOpenFalse.requires_settle());
    }

    // ── Output 状態管理テスト ───────────────────────────────────────────────────

    fn make_output() -> Output {
        Output::new()
    }

    #[test]
    fn output_starts_cold() {
        let o = make_output();
        assert!(!o.is_composition_warm(), "Output should start cold");
    }

    #[test]
    fn output_consecutive_count_increments_on_raw_tsf_literal_recovery() {
        let o = make_output();
        assert_eq!(o.composition.consecutive_count(), 0);
        o.mark_composition_cold(ColdReason::RawTsfLiteralRecovery);
        assert_eq!(o.composition.consecutive_count(), 1);
        o.mark_composition_cold(ColdReason::RawTsfLiteralRecovery);
        assert_eq!(o.composition.consecutive_count(), 2);
    }

    #[test]
    fn output_consecutive_count_resets_on_other_cold_reason() {
        let o = make_output();
        o.mark_composition_cold(ColdReason::RawTsfLiteralRecovery);
        o.mark_composition_cold(ColdReason::RawTsfLiteralRecovery);
        assert_eq!(o.composition.consecutive_count(), 2);
        o.mark_composition_cold(ColdReason::FocusChange);
        assert_eq!(
            o.composition.consecutive_count(),
            0,
            "non-recovery cold should reset count"
        );
    }

    #[test]
    fn output_consecutive_count_resets_on_focus_change() {
        let o = make_output();
        o.mark_composition_cold(ColdReason::RawTsfLiteralRecovery);
        assert_eq!(o.composition.consecutive_count(), 1);
        o.on_focus_changed();
        assert_eq!(
            o.composition.consecutive_count(),
            0,
            "focus change should reset consecutive count"
        );
    }

    #[test]
    fn output_last_cold_reason_tracks_latest() {
        let o = make_output();
        o.mark_composition_cold(ColdReason::SymbolVkSent);
        assert_eq!(o.composition.last_cold_reason(), ColdReason::SymbolVkSent);
        o.mark_composition_cold(ColdReason::RawTsfLiteralRecovery);
        assert_eq!(
            o.composition.last_cold_reason(),
            ColdReason::RawTsfLiteralRecovery
        );
    }

    // コードレビュー指摘(simplify角度): 以前ここにあった
    // `completion_confirmed_orders_retry_post_send_effects_deferred_then_guard_drop`
    // は、ハードコードした `Vec` リテラルが自分自身と等しいことだけを検証する
    // トートロジーで、`Platform::complete_gji_reinit_retry` を一切実行しない
    // ため、実装の呼び出し順を変えても壊れなかった（削除済み）。
    // 呼び出し順の規約は `Platform::complete_gji_reinit_retry` の doc コメント
    // （SSOT）に移した。この関数はWin32/`Platform`依存のためLinux上でのユニット
    // テストが非現実的（既存の `tsf`/`platform` 系コードと同じ制約）。

    // ── ConvModeAuthority 不変条件テスト ─────────────────────────────────────────

    #[test]
    fn conv_mutation_allowed_starts_false() {
        // Output 初期状態は UserOwned（Unknown）相当 → conv mutation 禁止
        let o = make_output();
        assert!(!o.conv_mutation_allowed.get());
    }

    #[test]
    fn set_conv_mutation_allowed_roundtrip() {
        let o = make_output();
        o.set_conv_mutation_allowed(true);
        assert!(o.conv_mutation_allowed.get());
        o.set_conv_mutation_allowed(false);
        assert!(!o.conv_mutation_allowed.get());
    }

    #[test]
    fn conv_policy_user_managed_forbids_mutation() {
        use crate::state::ConvModeAuthority;
        assert!(!ConvModeAuthority::UserOwned.allows_conv_mutation());
    }

    #[test]
    fn conv_policy_awase_locked_allows_mutation() {
        use crate::state::ConvModeAuthority;
        assert!(ConvModeAuthority::AwaseOwned.allows_conv_mutation());
    }

    #[test]
    fn conv_policy_default_is_user_managed() {
        use crate::state::ConvModeAuthority;
        assert_eq!(ConvModeAuthority::default(), ConvModeAuthority::Unknown);
    }

    // ── RAW_TSF_LITERAL グローバル構造体テスト ──────────────────────────────────

    #[test]
    fn raw_tsf_literal_backs_roundtrip() {
        use std::sync::atomic::Ordering::Relaxed;
        crate::RAW_TSF_LITERAL.backs.store(3, Relaxed);
        let n = crate::RAW_TSF_LITERAL.backs.swap(0, Relaxed);
        assert_eq!(n, 3);
        assert_eq!(crate::RAW_TSF_LITERAL.backs.load(Relaxed), 0);
    }

    #[test]
    fn raw_tsf_literal_romaji_roundtrip() {
        {
            let mut guard = crate::RAW_TSF_LITERAL.romaji.lock().unwrap();
            *guard = "konnichiwa".to_string();
        }
        let taken = {
            let mut guard = crate::RAW_TSF_LITERAL.romaji.lock().unwrap();
            std::mem::take(&mut *guard)
        };
        assert_eq!(taken, "konnichiwa");
        let now_empty = crate::RAW_TSF_LITERAL.romaji.lock().unwrap().clone();
        assert!(now_empty.is_empty());
    }

    // ── discard_raw_recovery_if_focus_stale テスト（ADR-101/BUG-74 コードレビュー
    // 指摘: backspace 送信より前に focus 世代を照合する）──────────────────────────

    // ── defer_if_probe_in_flight_recovery_exempt テスト（ADR-123 変更A+C
    // 決定4-2、Opus敵対的レビュー round4指摘: raw recovery 自身の再送が
    // 無関係な別 give-up の pending_gji_reinit(Polling) を見て自己 defer
    // してしまう退行）──────────────────────────────────────────────────

    #[test]
    fn defer_vk_if_probe_in_flight_keeps_deferring_past_the_old_cap_of_32() {
        // BUG-165: 旧上限 32 は 2ms 間隔の高速打鍵で cold probe 中に超過し、
        // 超過分が通常送信へ degrade して消えた。通常の打鍵量では上限に達しない。
        let o = make_output();
        o.install_pending_tsf(Box::new(
            crate::tsf::warmup::chrome_probe::ChromeProbe::new(
                "x",
                Generation::INITIAL,
                crate::tsf::probe::TsfReadinessProbe::new(0, Generation::INITIAL, 0),
                0,
                OutputActiveGuard::begin(),
            ),
        ));
        for i in 0..500 {
            assert!(
                o.defer_vk_if_probe_in_flight(VkCode(0x41), false, DeferredOrigin::UserInput),
                "{i} 件目で defer が諦められた"
            );
        }
        assert_eq!(o.pending_deferred_len(), 500);
    }

    #[test]
    fn defer_vk_if_probe_in_flight_degrades_instead_of_pushing_past_the_cap() {
        // 2026-09-03 code review指摘の回帰テスト: 単一VK版が件数上限
        // (would_exceed_deferred_cap)を経由せず無条件pushしていた退行の
        // 固定。romaji版の同名テストと対をなす。
        let o = make_output();
        o.install_pending_tsf(Box::new(
            crate::tsf::warmup::chrome_probe::ChromeProbe::new(
                "x",
                Generation::INITIAL,
                crate::tsf::probe::TsfReadinessProbe::new(0, Generation::INITIAL, 0),
                0,
                OutputActiveGuard::begin(),
            ),
        ));
        for _ in 0..TsfWarmupCoordinator::DEFERRED_QUEUE_CAP {
            assert!(o.defer_vk_if_probe_in_flight(VkCode(0x41), false, DeferredOrigin::UserInput));
        }
        assert_eq!(
            o.pending_deferred_len(),
            TsfWarmupCoordinator::DEFERRED_QUEUE_CAP
        );

        let deferred =
            o.defer_vk_if_probe_in_flight(VkCode(0x41), false, DeferredOrigin::UserInput);

        assert!(!deferred, "上限到達後は defer せず false を返すべき");
        assert_eq!(
            o.pending_deferred_len(),
            TsfWarmupCoordinator::DEFERRED_QUEUE_CAP,
            "上限到達後にキューが増えてはいけない"
        );
    }

    #[test]
    fn defer_if_probe_in_flight_recovery_exempt_still_defers_when_probe_in_flight() {
        // has_pending_tsf()=true（無関係な別 probe が実際に走っている）は
        // recovery_exempt 版でも従来どおり defer する——除外されるのは
        // raw_recovery_owns_deferred() の項だけ。
        let o = make_output();
        o.install_pending_tsf(Box::new(
            crate::tsf::warmup::chrome_probe::ChromeProbe::new(
                "x",
                Generation::INITIAL,
                crate::tsf::probe::TsfReadinessProbe::new(0, Generation::INITIAL, 0),
                0,
                OutputActiveGuard::begin(),
            ),
        ));
        assert!(o.warmup_coord.has_pending_tsf());

        let deferred =
            o.defer_if_probe_in_flight_recovery_exempt("a", DeferredOrigin::RecoveryResend);

        assert!(
            deferred,
            "has_pending_tsf()=true による defer は recovery_exempt でも維持すべき"
        );
    }

    // ── is_probe_or_recovery_blocking テスト（ADR-123 変更A+C 決定4-3、
    // drain-before-send の判定ロジック）─────────────────────────────────

    #[test]
    fn defer_if_probe_in_flight_degrades_instead_of_pushing_past_the_cap() {
        // 件数上限に達した状態で新たな defer 要求が来た場合、push せず
        // false を返して「今日と同じ挙動へ degrade」すべき（強制flushは
        // しない、`TsfWarmupCoordinator::is_deferred_queue_full` の doc
        // コメント参照）。
        let o = make_output();
        o.install_pending_tsf(Box::new(
            crate::tsf::warmup::chrome_probe::ChromeProbe::new(
                "x",
                Generation::INITIAL,
                crate::tsf::probe::TsfReadinessProbe::new(0, Generation::INITIAL, 0),
                0,
                OutputActiveGuard::begin(),
            ),
        ));
        for _ in 0..TsfWarmupCoordinator::DEFERRED_QUEUE_CAP {
            assert!(o.defer_if_probe_in_flight("a", DeferredOrigin::UserInput));
        }
        assert_eq!(
            o.pending_deferred_len(),
            TsfWarmupCoordinator::DEFERRED_QUEUE_CAP
        );

        let deferred = o.defer_if_probe_in_flight("a", DeferredOrigin::UserInput);

        assert!(!deferred, "上限到達後は defer せず false を返すべき");
        assert_eq!(
            o.pending_deferred_len(),
            TsfWarmupCoordinator::DEFERRED_QUEUE_CAP,
            "上限到達後にキューが増えてはいけない（強制flushもしない）"
        );
    }

    // ── shift-conv-guard confirm-gate override 所有権テスト（ADR-084 BUG-49 追補2、pass-5）──

    #[test]
    fn extend_confirm_gate_override_writes_when_gen_matches() {
        let o = make_output();
        let owner_gen = o.bump_shift_conv_guard_gen();
        assert!(o.extend_confirm_gate_override(owner_gen, 12345));
        assert_eq!(o.confirm_gate_deadline_override_ms.get(), 12345);
    }

    #[test]
    fn extend_confirm_gate_override_is_a_noop_when_gen_is_stale() {
        let o = make_output();
        let owner_gen = o.bump_shift_conv_guard_gen();
        o.confirm_gate_deadline_override_ms.set(999);
        // 別の hold が始まった（gen が進んだ）ことをシミュレートする。
        o.bump_shift_conv_guard_gen();
        assert!(!o.extend_confirm_gate_override(owner_gen, 12345));
        assert_eq!(
            o.confirm_gate_deadline_override_ms.get(),
            999,
            "stale な owner_gen からの延長は新しい hold の override を \
             上書きしてはならない"
        );
    }

    #[test]
    fn clear_confirm_gate_override_resets_when_gen_matches() {
        let o = make_output();
        let owner_gen = o.bump_shift_conv_guard_gen();
        o.confirm_gate_deadline_override_ms.set(12345);
        o.clear_confirm_gate_override(owner_gen);
        assert_eq!(o.confirm_gate_deadline_override_ms.get(), 0);
    }

    #[test]
    fn clear_confirm_gate_override_is_a_noop_when_gen_is_stale() {
        let o = make_output();
        let owner_gen = o.bump_shift_conv_guard_gen();
        // owner_gen 捕獲後に新しい hold が始まり、その override を書き込む
        // （実際のシーケンス: 旧タスクが gen を捕獲 → 新 hold が bump + 延長）。
        let new_owner_gen = o.bump_shift_conv_guard_gen();
        assert!(o.extend_confirm_gate_override(new_owner_gen, 67890));
        // 旧タスク（stale な owner_gen）がクリアしようとしても、新 hold の
        // override を壊してはならない — pass-5 レビューが検出した blocking
        // 欠陥そのものの再発防止テスト。
        o.clear_confirm_gate_override(owner_gen);
        assert_eq!(
            o.confirm_gate_deadline_override_ms.get(),
            67890,
            "stale な owner_gen からのクリアが新しい hold の override を \
             消してしまうと、その hold は BUG-49 の release 側で無防備になる"
        );
    }

    #[test]
    fn bump_shift_conv_guard_gen_returns_the_new_value() {
        let o = make_output();
        let g1 = o.bump_shift_conv_guard_gen();
        let g2 = o.bump_shift_conv_guard_gen();
        assert_ne!(g1, g2);
        assert_eq!(o.shift_conv_guard_gen.get(), g2);
    }

    // ── 既存テスト ─────────────────────────────────────────────────────────────

    #[test]
    fn test_ascii_to_vk_lowercase() {
        assert_eq!(ascii_to_vk('a'), Some((VkCode(0x41), false)));
        assert_eq!(ascii_to_vk('z'), Some((VkCode(0x5A), false)));
    }

    #[test]
    fn test_ascii_to_vk_uppercase() {
        assert_eq!(ascii_to_vk('A'), Some((VkCode(0x41), true)));
    }

    #[test]
    fn test_ascii_to_vk_digits() {
        assert_eq!(ascii_to_vk('0'), Some((VkCode(0x30), false)));
        assert_eq!(ascii_to_vk('9'), Some((VkCode(0x39), false)));
    }

    #[test]
    fn test_ascii_to_vk_unknown() {
        assert_eq!(ascii_to_vk('\u{3042}'), None); // 'あ'
    }
}
