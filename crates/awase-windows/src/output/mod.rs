use crate::vk::ascii_to_vk;
use awase::types::{KeyAction, VkCode};
use std::time::Duration;

pub use crate::tsf::output::ColdReason;
pub use crate::tsf::output::{INJECTED_MARKER, TSF_MARKER};

pub mod sender;
pub(crate) mod types;
pub(crate) use sender::OutputSession;
pub(crate) use types::InjectionMode;

pub(crate) mod ime_apply_planner;
mod key_injector;
pub(crate) mod probe_io;
mod resolve;
mod tsf_warmup_coord;
mod vk_send;
/// IME open 状態の観測値を適用時ビリーフへ純粋還元する data-model。
pub(crate) use ime_apply_planner::{reduce_open_belief, OpenBelief, OpenBeliefInputs};
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
    /// `kp_stage_idle_conv_check` が `update_from_conv` で更新し、
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
    /// JISかな化からのローマ字入力復元（BUG-08 Apply(3)）を最後に送った時刻
    /// （`GetTickCount64` 由来）。steady-state 検出のレート制限に使う。
    pub(crate) last_roman_restore_ms: std::cell::Cell<u64>,
    /// Output → Runtime の遅延リクエストを蓄積するアウトボックス。
    ///
    /// キー注入中に `with_app` 経由で Runtime を直接呼ぶと再入するため、
    /// `RuntimeRequest` を積んでキー処理境界で Runtime が `take_pending_requests` で drain する。
    /// H-4-b: vk_send.rs Chrome cold パスが `StartTsfProbe` を積み、
    /// drain_runtime_requests が TIMER_TSF_PROBE を起動する。
    pub(crate) runtime_outbox: std::cell::RefCell<crate::runtime::outbox::RuntimeOutbox>,
}

impl std::fmt::Debug for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Output").finish_non_exhaustive()
    }
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
}

/// `ensure_tsf_warm` の戻り値。warmup フローの結果を表す。
pub(crate) struct WarmupOutcome {
    /// F2 ウォームアップバッチが前置きされたか
    pub prepend_f2_warmup: bool,
    /// eager warmup パス（既存の F2 経由）を通ったか（Unicode 送信判定に使用）
    pub used_eager_path: bool,
    /// cold start シーケンス番号（ログ相関用）
    pub cold_seq: u32,
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
            observe_unicode_literal: std::sync::atomic::AtomicBool::new(false),
            conv_mutation_allowed: std::cell::Cell::new(false),
            last_roman_restore_ms: std::cell::Cell::new(0),
            runtime_outbox: std::cell::RefCell::new(crate::runtime::outbox::RuntimeOutbox::new()),
        }
    }

    /// Output が蓄積した `RuntimeRequest` を全件取り出す。
    ///
    /// Runtime がキー処理境界（`WM_EXECUTE_EFFECTS` / `WM_DRAIN_OUTPUT_QUEUE` 末尾）で呼び、
    /// 各リクエストを実行する。H-4-b で push 側が配線されるまでは常に空を返す。
    pub(crate) fn take_pending_requests(&self) -> Vec<crate::runtime::outbox::RuntimeRequest> {
        self.runtime_outbox.borrow_mut().drain()
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

    /// GjiFsm が long-cold（≥10s idle）な次の KeyInput か判定する（send_keys の defer 判定用）。
    pub(crate) fn gji_is_next_key_long_cold(&self) -> bool {
        self.warmup_coord.is_next_key_long_cold()
    }

    /// `send_unicode_char()` の遅延モードを ON/OFF する。
    ///
    /// ON 中は `send_unicode_char()` が実送信せず `unicode_cold_deferred` に蓄積する。
    /// `Platform::send_keys` が `output.send_keys()` の前後でセット／クリアする。
    pub(crate) fn set_unicode_cold_defer(&self, defer: bool) {
        self.injector.set_unicode_cold_defer(defer);
    }

    /// 蓄積した Unicode deferred 文字を取り出してバッファをクリアする。
    ///
    /// `Platform::dispatch_gji_response` が `StartProbe { is_long_cold }` 処理時に呼ぶ。
    pub(crate) fn take_unicode_cold_deferred(&self) -> Vec<char> {
        self.injector.take_unicode_cold_deferred()
    }

    /// 飛行中の `UnicodeColdWarmupFsm` に chars を追記する。
    ///
    /// 成功（FSM が存在して追記できた）なら `true`、なければ `false`。
    pub(crate) fn try_push_unicode_chars_to_pending(&self, chars: &[char]) -> bool {
        self.warmup_coord.try_push_unicode_chars_to_pending(chars)
    }

    /// Unicode cold-start 用の GJI ウォームアップキーを送信する。
    ///
    /// 1. VK_IME_ON (0x16) を `IME_KANJI_MARKER` 付きで送信してひらがなモードへ切替。
    /// 2. VK_A + BS を `INJECTED_MARKER` 付きで同一バッチ送信（犠牲キー）。
    ///    VK_A が GJI の hiragana composition を起動して `gji_write_bytes` を増やし、
    ///    BS が即キャンセルするため文字フラッシュは発生しない。
    pub(crate) fn send_unicode_cold_warmup_keys(&self, cold_seq: u32) {
        use crate::tsf::output::{make_key_input_ex, IME_KANJI_MARKER, INJECTED_MARKER};
        use crate::vk::{VK_BACK, VK_IME_ON};
        use awase::types::VkCode;
        const VK_A: VkCode = VkCode(0x41);

        let ime_on_inputs = [
            make_key_input_ex(VK_IME_ON, false, IME_KANJI_MARKER),
            make_key_input_ex(VK_IME_ON, true, IME_KANJI_MARKER),
        ];
        log::debug!("[unicode-cold-warmup] cold={cold_seq} VK_IME_ON 送信 (ひらがなモード切替)");
        let _ = crate::win32::send_input_safe(&ime_on_inputs);
        self.ime_mode_fsm.borrow_mut().on_f21_sent();

        let sacr_inputs = [
            make_key_input_ex(VK_A, false, INJECTED_MARKER),
            make_key_input_ex(VK_A, true, INJECTED_MARKER),
            make_key_input_ex(VK_BACK, false, INJECTED_MARKER),
            make_key_input_ex(VK_BACK, true, INJECTED_MARKER),
        ];
        log::debug!(
            "[unicode-cold-warmup] cold={cold_seq} VK_A+BS 犠牲キー送信 (gji_write_bytes 上昇待ち)"
        );
        let _ = crate::win32::send_input_safe(&sacr_inputs);
    }

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
    }

    // `start_ms_ime_ready_poll`（BUG-13 の IMC 確認ポーリング）は spawn_local 内で
    // with_app を使うため、layer-boundaries B-1 の ALLOW 対象である `probe_io.rs` にある。

    /// VK_IME_OFF → VK_IME_ON の連続送信を ImeModeFsm に通知する。
    ///
    /// `send_sacrificial_ime_off_on` / `send_chrome_gji_reinit_and_poll` で使う。
    pub(crate) fn on_f22_f21_sent(&self) {
        let mut fsm = self.ime_mode_fsm.borrow_mut();
        fsm.on_f22_sent();
        fsm.on_f21_sent();
    }

    /// VK_IME_ON/OFF 送信時に `ImeModeFsm` の belief を即時更新する。
    ///
    /// 通常 IME ON/OFF（`send_engine_state_ime_key` 経由）用。
    pub(crate) fn on_ime_mode_vk_sent(&self, vk: VkCode) {
        let mut fsm = self.ime_mode_fsm.borrow_mut();
        if vk == crate::vk::VK_IME_ON {
            fsm.on_f21_sent();
        } else if vk == crate::vk::VK_IME_OFF {
            fsm.on_f22_sent();
        }
    }

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

    /// `GjiAction::StartProbe` の ncwait_budget_ms / forces_prepend_f2 / is_long_cold を記録する。
    ///
    /// `send_romaji_as_tsf` が `GjiWarmupCoro::new` を生成する際に参照する。
    /// GjiFsm の `Authorized` 状態から `ProbeParams` を読み出す。
    ///
    /// `Authorized` でない場合は `ProbeParams::default()` を返す。
    pub(crate) fn gji_current_probe_params(&self) -> crate::tsf::gji_fsm::ProbeParams {
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

    /// `pending_gji_warmup` を取り出す（1回限り）。
    pub(crate) fn gji_take_warmup_result(&self) -> Option<crate::tsf::gji_fsm::WarmupResult> {
        self.warmup_coord.take_warmup_result()
    }

    /// eager warmup F2 を送信した時刻（ms）を返す。0 = 未送信。
    /// WinEvent 観察コールバックが warmup からの経過時間をログするために使う。
    #[must_use]
    pub const fn eager_warmup_sent_ms(&self) -> u64 {
        self.composition.eager_warmup_sent_ms()
    }

    /// 最後の `send_keys` 完了からの経過時間（ms）。
    /// 一度も送信していない場合は `u64::MAX` を返す（= 永久に in-flight でない）。
    #[must_use]
    pub fn ms_since_last_send(&self) -> u64 {
        self.composition.ms_since_last_send()
    }

    /// IME composition context をコールド状態にマークする。
    ///
    /// 次の VK / TSF composition 送信時に VK_DBE_HIRAGANA ウォームアップを
    /// 先行送信させる。Space/Enter/Escape passthrough・エンジン toggle 等のタイミングで呼ぶ。
    /// フォーカス変更は `on_focus_changed()` を使うこと（epoch も更新される）。
    ///
    /// # NativeF2Consumed でも eager_warmup_sent_ms をリセットする理由
    ///
    /// 物理 F2 が押された = WezTerm に新しい F2 が届く = TSF 初期化が再トリガーされる。
    /// FocusChange のタイムスタンプを保持すると「古い F2 からの経過時間」を elapsed として
    /// 計算してしまい、sleep がスキップされる（"hoんらい" 化け: BUG-06 の派生形）。
    ///
    /// 例: FocusChange warmup(T=0) → 物理F2(T=2265ms) → ほ送信(T=2562ms)
    ///   旧: elapsed=2562ms→即送信、新F2からは297ms→TSF未初期化→"ho"リテラル
    ///   新: elapsed=297ms→sleep203ms→新F2から500ms待機→TSF初期化済み→"ほ" ✓
    ///
    /// 直後に send_eager_tsf_warmup() が新しいタイムスタンプをセットする。
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
    /// 現在の warmup 戦略が F2 (VK_DBE_HIRAGANA) を自前送信するか（= GJI 戦略か）。
    ///
    /// `PhysicalKeyDisposition::plan` の F2 Suppress 判断に使う。false（MsImeStrategy）
    /// のとき物理 F2 を Suppress すると、代替送信が無いため IME ON にならない（BUG-10）。
    pub(crate) fn f2_warmup_owned(&self) -> bool {
        self.warmup_coord.needs_f2_probe()
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
        // フォーカス変更後 1500ms 以内の HanKata→ZenKata ダウングレードを抑制するため
        // タイムスタンプを記録する（TsfNative IMM/TSF 乖離対策）。
        #[cfg(windows)]
        self.conv_mode
            .on_focus_changed(crate::state::TickMs(crate::hook::current_tick_ms()));
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
            log::debug!(
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

    /// 現在の TSF 準備状態を多次元スナップショットとして返す。
    ///
    /// `applied_ime_on`: 呼び出し元が知っている「最後に apply された IME 開閉状態」。
    /// executor は `applied_snapshot` から渡す。不明な場合は `None`（`false` として扱われる）。
    /// 条件判定には返り値のメソッド（`can_warmup()` 等）を使う。
    #[must_use]
    pub fn tsf_readiness(&self, applied_ime_on: Option<bool>) -> crate::tsf::TsfReadiness {
        crate::tsf::TsfReadiness {
            gate: self.tsf_gate.state(),
            ime_on: applied_ime_on.unwrap_or(false),
            is_tsf_mode: self.is_tsf_mode(),
        }
    }

    /// TSF composition context の事前ウォームアップ F2 を送信する。
    ///
    /// 以下のタイミングで呼ぶ:
    /// - FocusChange 直後: WezTerm に TSF 初期化の先行時間を与える
    /// - NativeF2Consumed 直後: 物理 F2 の代替として送信（二重 F2 防止）
    /// - PassthroughConfirmKey / ReinjectConfirmKey 直後: Enter/Escape 後の次打鍵を warmup
    ///
    /// `applied_ime_on`: 呼び出し元が知っている IME 開閉状態。`None` で latch にフォールバック。
    /// 値が false（IME OFF）または TSF モード以外では何もしない。
    ///
    /// `eager_warmup_sent_ms` を現在時刻で更新する。NativeF2Consumed 等の前に
    /// `mark_composition_cold` が呼ばれて 0 にリセットされるため二重更新は発生しない。
    pub fn send_eager_tsf_warmup(&self, applied_ime_on: Option<bool>) {
        if !self.conv_mutation_allowed.get() {
            log::trace!("[tsf-eager-warmup] non-AwaseOwned → warmup スキップ");
            return;
        }
        if !self.warmup_coord.needs_f2_probe() {
            log::trace!("[tsf-eager-warmup] non-GJI strategy → warmup スキップ");
            return;
        }
        if !self.tsf_readiness(applied_ime_on).can_warmup() {
            return;
        }
        // OBJ_NAMECHANGE 連番をリセット（warmup 後のイベント順序追跡用）
        crate::tsf::observer::reset_namechange_seq();
        // charset に応じた warmup VK を選択する（送るとモードが変わってしまうため charset-aware が必須）:
        //   Hiragana  → F2 (VK_DBE_HIRAGANA)
        //   ZenKata   → F1 (VK_DBE_KATAKANA)
        //   HanKata   → F1+F3 (VK_DBE_KATAKANA + VK_DBE_SBCSCHAR)
        //   ZenAlpha  → F0+F4 (VK_DBE_ALPHANUMERIC + VK_DBE_DBCSCHAR)
        //   HanAlpha  → F0 (VK_DBE_ALPHANUMERIC)
        //
        // conv_mode は idle-check / focus-probe でしか更新されないため、ユーザーが直前に
        // モードを切替えた直後にキーを押すと stale な Hiragana で F2 を送ってしまう。
        // apply_focus_probe と同様に直前の IMM 読み取りで conv_mode を最新化する。
        // SAFETY: get_ime_conversion_mode_raw_timeout は apply_focus_probe (with_app 内) でも
        //         安全に呼ばれており、ここも同じ with_app コンテキストで安全。
        if let Some(fresh_conv) = unsafe { crate::ime::get_ime_conversion_mode_raw_timeout(5) } {
            self.conv_mode.update_from_conv(
                fresh_conv,
                crate::state::TickMs(crate::hook::current_tick_ms()),
            );
        }
        let charset = self.conv_mode.effective_charset();
        // カタカナ/英数系 warmup キー (F1/F0 系) は実際に GJI の charset を書き換える
        // 副作用を持つ。この関数は EmitWarmup (ConfirmKeyDown/Up・CtrlUp・SetOpenTrue 等)
        // のたびに呼ばれるため、同じ確定 mode に対して無条件に送り続けると、一発の
        // 誤読が belief に確定しただけでも real IME へ warmup のたびに再アサートされ
        // 続けて自己増幅するループになる (BUG-19)。`cold_warmup.rs::preamble` /
        // `probe_io.rs::send_sacrificial_ime_off_on` と同じ `needs_conv_restore_write()`
        // で「同じ mode への warmup 送信は1回だけ」に制限する。BUG-19 の根本原因分析
        // 自体がこの関数を名指ししていたが、従来の対策(ADR-078 Phase 1a)は上記2箇所
        // にしか配線されておらず、本関数は無防備なまま残っていた（実機ログで
        // 2026-07-11 に確認、`docs/known-bugs.md` BUG-19 追補4）。
        // Hiragana (F2) は ROMAN ビット確保のみで冪等なため対象外（既存の
        // `conv_target.is_none()` 除外と同じ理由）。
        if !matches!(charset, crate::state::Charset::Hiragana)
            && !self.conv_mode.needs_conv_restore_write()
        {
            log::trace!(
                "[tsf-eager-warmup] {charset} は確定済み → 反復 warmup 送信をスキップ (BUG-19 追補4)"
            );
            return;
        }
        let ms = match charset {
            crate::state::Charset::ZenkakuKatakana | crate::state::Charset::HankakuKatakana => {
                self.conv_mode.mark_conv_restore_written();
                let ms = crate::tsf::send::send_vk_dbe_katakana_warmup(charset);
                log::debug!(
                    "[tsf-eager-warmup] {charset} warmup 送信, eager_warmup_sent_ms={ms}ms"
                );
                // HanKata warmup (F1+F3) 後は IMM conv が ZenKata (0x0B) を返すことがある。
                // TsfNative では F3 が IMM FULLSHAPE ビットを変更しないため。conv_mode 汚染を抑制する。
                if charset == crate::state::Charset::HankakuKatakana {
                    self.conv_mode.on_hankata_warmup_sent(crate::state::TickMs(
                        crate::hook::current_tick_ms(),
                    ));
                }
                ms
            }
            crate::state::Charset::ZenkakuAlpha | crate::state::Charset::HankakuAlpha => {
                self.conv_mode.mark_conv_restore_written();
                let ms = crate::tsf::send::send_vk_dbe_alpha_warmup(charset);
                log::debug!(
                    "[tsf-eager-warmup] {charset} warmup 送信, eager_warmup_sent_ms={ms}ms"
                );
                ms
            }
            crate::state::Charset::Hiragana => {
                let ms = crate::tsf::send::send_vk_dbe_hiragana_pair();
                log::debug!("[tsf-eager-warmup] VK_DBE_HIRAGANA 送信, eager_warmup_sent_ms={ms}ms");
                ms
            }
        };
        self.composition.set_eager_warmup_sent_ms(ms);
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
        log::debug!(
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
                    log::debug!("  → SpecialKey({sk:?}) vk=0x{:02X}", special_key_to_vk(*sk));
                    self.injector.send_key(special_key_to_vk(*sk), false);
                }
                KeyAction::Key(vk) => {
                    log::debug!("  → Key({vk:#06X})");
                    self.injector.send_key(*vk, false);
                }
                KeyAction::KeyUp(vk) => {
                    log::debug!("  → KeyUp({vk:#06X})");
                    self.injector.send_key(*vk, true);
                }
                KeyAction::Char(ch) => {
                    log::debug!("  → Char('{ch}') via {}", sender.mode_label());
                    sender.send_char(*ch);
                }
                KeyAction::Suppress => {
                    log::debug!("  → Suppress");
                }
                KeyAction::Romaji(s) => {
                    log::debug!("  → Romaji(\"{s}\") via {}", sender.mode_label());
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
                            log::debug!(
                                "[unicode-obs] cold={cold_seq} Unicode Romaji 送信後に GJI write 観測開始 \
                                (baseline={baseline})"
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
                    log::debug!("  → KeySequence(\"{s}\") via {}", sender.mode_label());
                    sender.send_key_sequence(s);
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
        let warm = self.is_composition_warm();
        let elapsed = self.ms_since_last_send();
        let session_expired =
            warm && elapsed < u64::MAX && elapsed > crate::tuning::COMPOSITION_TIMEOUT_MS;
        WarmthContext {
            warm,
            elapsed,
            session_expired,
            prepend_f2_warmup: (!warm || session_expired) && self.warmup_coord.needs_f2_probe(),
        }
    }

    /// probe 進行中なら romaji を VK 列に変換して deferred_vks に追記し true を返す。
    /// probe がなければ何もせず false を返す。
    pub(super) fn defer_if_probe_in_flight(&self, romaji: &str) -> bool {
        if !self.warmup_coord.has_pending_tsf() {
            return false;
        }
        let vks: Vec<(VkCode, bool)> = romaji.chars().filter_map(ascii_to_vk).collect();
        log::debug!(
            "[tsf] probe in flight → deferred {} VK(s) for {:?}",
            vks.len(),
            romaji
        );
        self.warmup_coord.defer_vks_if_in_flight(&vks)
    }

    /// probe 進行中なら単一 VK を deferred_vks に追記し true を返す。
    /// probe がなければ何もせず false を返す。
    pub(super) fn defer_vk_if_probe_in_flight(&self, vk: VkCode, needs_shift: bool) -> bool {
        self.warmup_coord
            .defer_vks_if_in_flight(&[(vk, needs_shift)])
    }

    /// long-cold 後の GJI 再初期化: VK_IME_OFF→VK_IME_ON を SendInput で注入する。
    ///
    /// Chrome の `send_chrome_gji_reinit_and_poll` と同じ VK_IME_OFF→VK_IME_ON シーケンスだが、
    /// WT（Unicode mode）向けに async IMC ポーリングは行わない。
    pub(crate) fn send_f22_f21_reinit(&self) {
        use probe_io::ProbeIo as _;
        self.send_chrome_gji_reinit_and_poll(0);
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
                // SAFETY: GetForegroundWindow + ImmGetContext + ImmGetCompositionStringW。
                //         step_probe は TIMER_TSF_PROBE ハンドラ（メインスレッド）から呼ばれる。
                foreground_comp_char: unsafe { crate::ime::get_foreground_comp_str_char() },
                gji_candidate_visible: crate::tsf::observer::gji_candidate_visible_now(),
                ime_mode: ime_fsm.state(),
                ime_mode_confirmed: ime_fsm.is_confirmed(),
                deferred_pending: self.warmup_coord.has_pending_deferred(),
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
            };
        };
        log::debug!(
            "[tsf-probe-tick] cold={} t={}ms",
            machine.cold_seq_hint(),
            tick_t
        );
        let actions = machine.tick(&env);
        let dispatch = probe_io::dispatch_probe_actions(machine.as_mut(), actions, self);
        match dispatch {
            probe_io::DispatchResult::Done => {
                self.on_tsf_probe_ready();
                self.gji_end_probe_guard();
                let gji_response = self.gji_take_warmup_result().and_then(|result| {
                    let probe_id = self.warmup_coord.take_probe_id()?;
                    Some(
                        self.gji_on_event(crate::tsf::gji_fsm::GjiEvent::WarmupComplete {
                            probe_id,
                            result,
                        }),
                    )
                });
                let needs_gji_composition_reset = self.warmup_coord.take_composition_reset();
                StepProbeResult {
                    timer_cmd: TimerCommand::Kill {
                        id: crate::TIMER_TSF_PROBE,
                    },
                    gji_response,
                    needs_gji_composition_reset,
                    learned_tsf: false,
                }
            }
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
                }
            }
            probe_io::DispatchResult::SwitchMachine(new_machine) => {
                // SacrificialWarmupFsm への切り替え（GjiWarmupCoro long_cold パス / Chrome）。
                // 新 machine が内部ガードを保持するため gji_probe_guard を解放する。
                self.gji_end_probe_guard();
                let needs_gji_composition_reset = self.warmup_coord.take_composition_reset();
                self.warmup_coord.restore_pending_tsf(new_machine);
                StepProbeResult {
                    timer_cmd: TimerCommand::Continue {
                        id: crate::TIMER_TSF_PROBE,
                        delay: Duration::from_millis(10),
                    },
                    gji_response: None,
                    needs_gji_composition_reset,
                    learned_tsf: false,
                }
            }
            probe_io::DispatchResult::LearnedTsf => {
                // UnicodeLiteralObserverFsm が GJI write なしと判断した。
                // advance_tsf_probe がフォーカス中クラスを Tsf に昇格する。
                let needs_gji_composition_reset = self.warmup_coord.take_composition_reset();
                StepProbeResult {
                    timer_cmd: TimerCommand::Kill {
                        id: crate::TIMER_TSF_PROBE,
                    },
                    gji_response: None,
                    needs_gji_composition_reset,
                    learned_tsf: true,
                }
            }
        }
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

    /// sacr-warmup probe に StartComposition が観測されたことを通知する。
    ///
    /// `platform.rs::drain_pending_composition_events` が StartComposition を処理した際に呼ぶ。
    /// VK_A+BS atomic batch で SHOW+HIDE が最初の tick より前に完了したケースを検出するため、
    /// `SacrificialWarmupFsm::composition_was_seen` フラグをセットする。
    pub(crate) fn notify_probe_start_composition(&self) {
        self.warmup_coord.notify_probe_start_composition();
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
        log::debug!("[raw-tsf-literal] re-sending raw TSF literal romaji={romaji:?}");
        // Bypass (Chrome) では send_romaji_as_tsf が GJI probe (TransmitTarget::Tsf) を
        // 起動するが、Chrome は gate=Bypass のため dispatch_probe_actions でスキップされる。
        // Chrome バッチパス (TransmitTarget::Chrome) を使うことで正しく再送できる。
        if self.tsf_gate.state() == crate::tsf::TsfGateState::Bypass {
            self.send_romaji_batched(&romaji);
        } else {
            self.send_romaji_as_tsf(&romaji);
        }
    }

    /// raw TSF literal 回収を一括実行: backspace 送信 → romaji 再送。
    ///
    /// WM_DRAIN_OUTPUT_QUEUE ハンドラから呼ぶ。drain keys より前に実行すること。
    pub fn flush_raw_tsf_literal_recovery(&self) {
        flush_raw_tsf_literal_backspaces();
        self.flush_raw_tsf_literal_romaji();
    }
}

pub use crate::tsf::output::flush_raw_tsf_literal_backspaces;

#[cfg(test)]
mod tests {
    use super::*;

    // ── ColdReason impl メソッドテスト ────────────────────────────────────────

    #[test]
    fn cold_reason_eager_settle_ms_short_idle() {
        assert_eq!(ColdReason::FocusChange.eager_settle_ms(false), 1500);
        assert_eq!(ColdReason::NativeF2Consumed.eager_settle_ms(false), 1500);
        assert_eq!(ColdReason::SetOpenTrue.eager_settle_ms(false), 1500);
        assert_eq!(
            ColdReason::PassthroughConfirmKey.eager_settle_ms(false),
            500
        );
        assert_eq!(ColdReason::ReinjectConfirmKey.eager_settle_ms(false), 500);
        assert_eq!(ColdReason::SymbolVkSent.eager_settle_ms(false), 500);
        assert_eq!(ColdReason::F2NonTsf.eager_settle_ms(false), 500);
        assert_eq!(
            ColdReason::RawTsfLiteralRecovery.eager_settle_ms(false),
            500
        );
        assert_eq!(ColdReason::SetOpenFalse.eager_settle_ms(false), 500);
    }

    #[test]
    fn cold_reason_eager_settle_ms_long_idle() {
        // FocusChange 系: long_idle で 1500→2000ms
        assert_eq!(ColdReason::FocusChange.eager_settle_ms(true), 2000);
        assert_eq!(ColdReason::NativeF2Consumed.eager_settle_ms(true), 2000);
        assert_eq!(ColdReason::SetOpenTrue.eager_settle_ms(true), 2000);
        // ConfirmKey 系: long_idle で 500→1500ms
        assert_eq!(
            ColdReason::PassthroughConfirmKey.eager_settle_ms(true),
            1500
        );
        assert_eq!(ColdReason::ReinjectConfirmKey.eager_settle_ms(true), 1500);
        // 他は不変
        assert_eq!(ColdReason::SymbolVkSent.eager_settle_ms(true), 500);
        assert_eq!(ColdReason::SetOpenFalse.eager_settle_ms(true), 500);
    }

    #[test]
    fn cold_reason_probe_min_ms() {
        assert_eq!(ColdReason::FocusChange.probe_min_ms(false), 100);
        assert_eq!(ColdReason::NativeF2Consumed.probe_min_ms(false), 100);
        assert_eq!(ColdReason::SetOpenTrue.probe_min_ms(false), 100);
        assert_eq!(ColdReason::FocusChange.probe_min_ms(true), 300);
        assert_eq!(ColdReason::NativeF2Consumed.probe_min_ms(true), 300);
        assert_eq!(ColdReason::SetOpenTrue.probe_min_ms(true), 300);
        assert_eq!(ColdReason::PassthroughConfirmKey.probe_min_ms(false), 50);
        assert_eq!(ColdReason::ReinjectConfirmKey.probe_min_ms(false), 50);
        assert_eq!(ColdReason::PassthroughConfirmKey.probe_min_ms(true), 300);
        assert_eq!(ColdReason::SymbolVkSent.probe_min_ms(false), 30);
        assert_eq!(ColdReason::F2NonTsf.probe_min_ms(false), 100);
        assert_eq!(ColdReason::RawTsfLiteralRecovery.probe_min_ms(false), 100);
        assert_eq!(ColdReason::SetOpenFalse.probe_min_ms(false), 100);
    }

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
