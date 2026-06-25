use awase::config::OutputMode;
use awase::kana_table::KanaTable;
use awase::types::{KeyAction, VkCode};
use std::collections::HashMap;
use std::time::Duration;

pub use crate::tsf::output::ColdReason;
pub use crate::tsf::output::{INJECTED_MARKER, TSF_MARKER};

pub mod sender;
pub(crate) mod types;
pub(crate) use sender::OutputSession;
pub(crate) use types::InjectionMode;

pub(crate) mod probe_io;
mod resolve;
mod vk_send;
use resolve::{ascii_to_vk, build_symbol_to_vk, special_key_to_vk};

/// 公開ヘルパー: ASCII → VK 変換（`platform.rs` の dispatcher 用）。
pub(crate) use resolve::ascii_to_vk as resolve_ascii_to_vk;
/// 公開ヘルパー: TSF 送信パイプライン（`platform.rs` の dispatcher 用）。
pub(crate) use vk_send::{TsfSendPipeline, VkMarker};

pub(crate) use crate::tsf::probe_fsm::DeferredVk;

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

/// SendInput によるキー注入を行うモジュール
pub struct Output {
    mode: OutputMode,
    /// ローマ字↔かな双方向テーブル（Unicode モード・Chrome VK モード両用）
    kana_table: KanaTable,
    /// Chrome VK モード用: 記号→VK コードマッピング
    symbol_to_vk: HashMap<char, (VkCode, bool)>,
    /// TSF composition context の warm/cold 状態管理。
    ///
    /// warm/cold epoch、last_send_ms、eager_warmup_sent_ms 等を集約する。
    /// 詳細は [`crate::tsf::probe::CompositionState`] を参照。
    pub composition: crate::tsf::probe::CompositionState,
    /// TIMER_TSF_PROBE で処理中の保留 TSF/VK probe ステートマシン。
    ///
    /// `send_romaji_as_tsf` / `send_romaji_batched` が cold start 時に設定し、
    /// `WindowsPlatform::advance_tsf_probe` がタイマーごとに 1 ステップ進める。
    /// 内部 `_guard` により OUTPUT_GATE.active が保留期間中維持される。
    pub(crate) pending_tsf: std::cell::RefCell<Option<Box<dyn crate::tsf::tickable_fsm::TickableFsm>>>,
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
    /// GJI の warm/cold 状態機械。
    ///
    /// フォーカス変更・IME ON/OFF・WarmupComplete を受け取り、LongIdle タイマーを管理する。
    /// Phase 2: CompositionState と並走（FSM が安定したら CompositionState を撤去）。
    pub(crate) gji_fsm: std::cell::RefCell<Box<dyn crate::tsf::warmup_strategy::ImeWarmupStrategy>>,
    /// 現在実行中の GJI probe の ID（`GjiAction::StartProbe` 受信時にセット）。
    ///
    /// `dispatch_probe_actions` が `WarmupResult` を生成した際の照合に使う。
    pub(crate) current_gji_probe_id: std::cell::Cell<Option<crate::tsf::gji_fsm::ProbeId>>,
    /// GJI probe 中に OUTPUT_GATE を活性化するガード。
    ///
    /// `send_romaji_as_tsf` の cold パスで `gji_begin_probe_guard()` を呼び、
    /// `step_probe` 完了時 / `CancelProbe` 時 / `SwitchMachine` 時に `gji_end_probe_guard()` で解放する。
    /// Chrome / LiteralDetect probe は `TsfProbeMachine._guard` で独立管理する。
    gji_probe_guard: std::cell::RefCell<Option<crate::tsf::probe_bridge::OutputActiveGuard>>,
    /// `dispatch_probe_actions` → `GjiFsm::WarmupComplete` の橋渡しバッファ。
    ///
    /// `ProbeIo::store_gji_warmup_result` がセットし、`step_probe` 完了後に取り出す。
    pending_gji_warmup: std::cell::Cell<Option<crate::tsf::gji_fsm::WarmupResult>>,
    /// `ProbeIo::mark_cold_raw_tsf` → `GjiFsm::CompositionReset` の橋渡しフラグ。
    ///
    /// `mark_cold_raw_tsf` は `&self` しか取れないため直接 `dispatch_gji_response` を呼べない。
    /// このフラグで pending を表現し、`step_probe` 完了後に Platform が拾う。
    pub(crate) pending_gji_composition_reset: std::cell::Cell<bool>,
    /// `send_romaji_as_tsf` / `send_romaji_batched` の `GjiFsm::KeyInput` Response バッファ。
    ///
    /// これらのメソッドは `&self` しか取れないためタイマー操作を直接行えない。
    /// Platform の `send_keys` が `drain_pending_gji_key_responses` で全件取り出して
    /// `dispatch_gji_response` に渡し、LongIdle タイマーリセット等を実行する。
    ///
    /// Vec にするのは、1回の send_keys で複数文字（例: NICOLA 同時打鍵で す+る）を
    /// 送る際に各文字の KeyInput Response を全て保存するため。Option だと後の文字が
    /// 前の文字の StartProbe Response を上書きしてしまい、gji_store_probe_id が
    /// 呼ばれなくなる。
    pub(crate) pending_gji_key_responses: std::cell::RefCell<
        Vec<
            timed_fsm::Response<crate::tsf::gji_fsm::GjiAction, crate::tsf::gji_fsm::GjiTimer>,
        >,
    >,
    /// IME 入力モード belief（Off / Hiragana / Katakana / Unknown）。
    ///
    /// F21/F22 送信時に即時 belief 更新。`IMC_GETCONVERSIONMODE` async ポーリングで確認。
    /// `TsfEnvSnapshot.ime_mode` / `ime_mode_confirmed` を通じて各 TickableFsm に公開する。
    /// `ChromeGjiReinitFsm` が F22→F21 後の Hiragana 確認待機に使用する。
    pub(crate) ime_mode_fsm: std::cell::RefCell<crate::tsf::ime_mode_fsm::ImeModeFsm>,
    /// `gji_on_focus_change` の `spawn_local` IMC ポーリングを世代管理する。
    ///
    /// フォーカス変更のたびにインクリメントし、`spawn_local` クロージャが取得時の世代を
    /// キャプチャする。コールバック到達時に現在値と一致しない（= その後に別のフォーカス変更
    /// が来た）場合は stale として破棄し、古いポーリング結果で ImeModeFsm を汚染しない。
    pub(crate) ime_mode_focus_gen: std::cell::Cell<u32>,
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
impl Output {
    pub fn new(mode: OutputMode) -> Self {
        Self {
            mode,
            kana_table: KanaTable::build(),
            symbol_to_vk: build_symbol_to_vk(),
            composition: crate::tsf::probe::CompositionState::new(),
            pending_tsf: std::cell::RefCell::new(None),
            tsf_gate: crate::tsf::TsfGate::new(),
            injection_mode: InjectionMode::Unicode,
            gji_fsm: std::cell::RefCell::new(Box::new(crate::tsf::gji_fsm::GjiFsm::new())),
            current_gji_probe_id: std::cell::Cell::new(None),
            gji_probe_guard: std::cell::RefCell::new(None),
            pending_gji_warmup: std::cell::Cell::new(None),
            pending_gji_composition_reset: std::cell::Cell::new(false),
            pending_gji_key_responses: std::cell::RefCell::new(Vec::new()),
            ime_mode_fsm: std::cell::RefCell::new(crate::tsf::ime_mode_fsm::ImeModeFsm::new()),
            ime_mode_focus_gen: std::cell::Cell::new(0),
        }
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
self.gji_fsm.borrow_mut().on_gji_event(event)
    }

    /// `OnComposing` 状態の現在 epoch を返す。`EndComposition` イベント送信に使う。
    /// `OnComposing` 以外の状態では `None`。
    pub(crate) fn gji_current_composition_epoch(
        &self,
    ) -> Option<crate::tsf::gji_fsm::FocusEpoch> {
self.gji_fsm.borrow().gji_current_composition_epoch()
    }

    // ── ImeModeFsm ヘルパー ─────────────────────────────────────────────────────

    /// `IMC_GETCONVERSIONMODE` の結果を `ImeModeFsm` に反映する。
    ///
    /// `spawn_local` 内の async ポーリングタスクから `with_app(|runtime| runtime.platform.output.update_ime_mode_from_imc(conv))` で呼ぶ。
    pub(crate) fn update_ime_mode_from_imc(&self, mode: Option<u32>) {
        self.ime_mode_fsm.borrow_mut().on_conversion_mode_read(mode);
    }

    /// フォーカス変更時に呼ぶ。F21/F22 直後の副作用 FocusChange かを判定して適切にリセット。
    ///
    /// 世代カウンタ `ime_mode_focus_gen` をインクリメントすることで、
    /// 以前の `spawn_local` IMC ポーリングが古いフォーカスの結果を書き込まないよう保護する。
    pub(crate) fn on_ime_mode_focus_changed(&self) {
        let now_ms = crate::hook::current_tick_ms();
        self.ime_mode_fsm.borrow_mut().on_focus_changed(now_ms);
        self.ime_mode_focus_gen
            .set(self.ime_mode_focus_gen.get().wrapping_add(1));
    }

    /// F21/F22 送信時に `ImeModeFsm` の belief を即時更新する。
    ///
    /// `send_chrome_gji_reinit_and_poll` は個別に `on_f22_sent()`/`on_f21_sent()` を呼ぶため、
    /// このメソッドは通常 IME ON/OFF（`send_engine_state_ime_key` 経由）用。
    pub(crate) fn on_ime_mode_vk_sent(&self, vk: VkCode) {
        let mut fsm = self.ime_mode_fsm.borrow_mut();
        if vk == crate::vk::VK_F21 {
            fsm.on_f21_sent();
        } else if vk == crate::vk::VK_F22 {
            fsm.on_f22_sent();
        }
    }

    /// GjiFsm に LongIdle タイムアウトを送り、Response を返す。
    pub(crate) fn gji_on_long_idle(
        &self,
    ) -> timed_fsm::Response<crate::tsf::gji_fsm::GjiAction, crate::tsf::gji_fsm::GjiTimer> {
self.gji_fsm.borrow_mut().on_gji_long_idle()
    }

    /// `GjiAction::StartProbe` を受信したとき probe_id を記録する。
    pub(crate) fn gji_store_probe_id(&self, id: crate::tsf::gji_fsm::ProbeId) {
        self.current_gji_probe_id.set(Some(id));
    }

    /// `GjiAction::StartProbe` の ncwait_budget_ms / forces_prepend_f2 / is_long_cold を記録する。
    ///
    /// `send_romaji_as_tsf` が `GjiWarmupFsm::new` を生成する際に参照する。
    /// GjiFsm の `Authorized` 状態から `ProbeParams` を読み出す。
    ///
    /// `Authorized` でない場合は `ProbeParams::default()` を返す。
    pub(crate) fn gji_current_probe_params(&self) -> crate::tsf::gji_fsm::ProbeParams {
        self.gji_fsm
            .borrow()
            .current_probe_params()
            .unwrap_or_default()
    }

    /// 現在の GJI probe_id を返す（確認用、消費しない）。
    pub(crate) fn gji_current_probe_id(&self) -> Option<crate::tsf::gji_fsm::ProbeId> {
        self.current_gji_probe_id.get()
    }

    /// GJI probe の OUTPUT_GATE ガードを開始する。
    ///
    /// `send_romaji_as_tsf` の cold パスで `TsfProbeMachine::new_gji` を呼ぶ直前に使う。
    pub(crate) fn gji_begin_probe_guard(&self) {
        *self.gji_probe_guard.borrow_mut() = Some(crate::tsf::probe_bridge::OutputActiveGuard::begin());
    }

    /// GJI probe の OUTPUT_GATE ガードを解放する。
    ///
    /// `step_probe` 完了時 / `CancelProbe` 時に呼ぶ。
    pub(crate) fn gji_end_probe_guard(&self) {
        *self.gji_probe_guard.borrow_mut() = None;
    }

    /// `pending_gji_key_responses` を全件取り出す。
    ///
    /// Platform の `send_keys` が呼び出し、タイマー操作（LongIdle リセット等）を実行する。
    /// Vec で返すのは、1回の send_keys で複数文字を送る場合に全 Response を保存するため。
    pub(crate) fn drain_pending_gji_key_responses(
        &self,
    ) -> Vec<
        timed_fsm::Response<crate::tsf::gji_fsm::GjiAction, crate::tsf::gji_fsm::GjiTimer>,
    > {
        std::mem::take(&mut *self.pending_gji_key_responses.borrow_mut())
    }

    /// `pending_gji_warmup` を取り出す（1回限り）。
    pub(crate) fn gji_take_warmup_result(&self) -> Option<crate::tsf::gji_fsm::WarmupResult> {
        self.pending_gji_warmup.take()
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

    /// 現在の composition_warm フラグを返す（GjiFsm が SSOT）。
    #[must_use]
    pub fn is_composition_warm(&self) -> bool {
self.gji_fsm.borrow().is_warm()
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
    pub fn tsf_readiness(&self, applied_ime_on: Option<bool>) -> awase::tsf::TsfReadiness {
        awase::tsf::TsfReadiness {
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
        if !self.tsf_readiness(applied_ime_on).can_warmup() {
            return;
        }
        // OBJ_NAMECHANGE 連番をリセット（warmup 後のイベント順序追跡用）
        crate::tsf::observer::reset_namechange_seq();
        // VK_DBE_HIRAGANA (F2) を送信: VK_IME_ON (0x16) は IME ON 状態をセットするだけで
        // TSF composition context の初期化をトリガーしない。WezTerm は物理 F2 受信時に
        // TSF composition を初期化するため、同等の VK_DBE_HIRAGANA を送る必要がある。
        let ms = crate::tsf::send::send_vk_dbe_hiragana_pair();
        self.composition.set_eager_warmup_sent_ms(ms);
        log::debug!("[tsf-eager-warmup] VK_DBE_HIRAGANA 送信, eager_warmup_sent_ms={ms}ms");
    }

    /// `send_keys` 完了時刻を記録する内部ヘルパー。
    fn mark_send(&self) {
        self.composition.update_last_send_ms();
    }

    /// 出力モードを変更する
    pub const fn set_mode(&mut self, mode: OutputMode) {
        self.mode = mode;
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
                    self.send_key(special_key_to_vk(*sk), false);
                }
                KeyAction::Key(vk) => {
                    log::debug!("  → Key({vk:#06X})");
                    self.send_key(*vk, false);
                }
                KeyAction::KeyUp(vk) => {
                    log::debug!("  → KeyUp({vk:#06X})");
                    self.send_key(*vk, true);
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
            prepend_f2_warmup: !warm || session_expired,
        }
    }

    /// probe 進行中なら romaji を VK 列に変換して deferred_vks に追記し true を返す。
    /// probe がなければ何もせず false を返す。
    pub(super) fn defer_if_probe_in_flight(&self, romaji: &str) -> bool {
        self.pending_tsf
            .borrow_mut()
            .as_mut()
            .is_some_and(|machine| {
                let vks: Vec<DeferredVk> = romaji
                    .chars()
                    .filter_map(ascii_to_vk)
                    .map(|(vk, needs_shift)| DeferredVk { vk, needs_shift })
                    .collect();
                log::debug!(
                    "[tsf] probe in flight → deferred {} VK(s) for {:?}",
                    vks.len(),
                    romaji
                );
                for DeferredVk { vk, needs_shift } in vks {
                    machine.push_deferred(vk, needs_shift);
                }
                true
            })
    }

    /// probe 進行中なら単一 VK を deferred_vks に追記し true を返す。
    /// probe がなければ何もせず false を返す。
    pub(super) fn defer_vk_if_probe_in_flight(&self, vk: VkCode, needs_shift: bool) -> bool {
        self.pending_tsf
            .borrow_mut()
            .as_mut()
            .is_some_and(|machine| {
                machine.push_deferred(vk, needs_shift);
                true
            })
    }

    /// long-cold 後の GJI 再初期化: F22（IME OFF）→ F21（IME ON）を SendInput で注入する。
    ///
    /// Chrome の `send_chrome_gji_reinit_and_poll` と同じ F22→F21 シーケンスだが、
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
        use probe_io::ProbeIo as _;
        let env = {
            let ime_fsm = self.ime_mode_fsm.borrow();
            crate::tsf::probe_fsm::TsfEnvSnapshot {
                is_tsf_mode: self.is_tsf_mode(),
                gji_active: self.gji_monitor_healthy(),
                // SAFETY: GetForegroundWindow + ImmGetContext + ImmGetCompositionStringW。
                //         step_probe は TIMER_TSF_PROBE ハンドラ（メインスレッド）から呼ばれる。
                foreground_comp_char: unsafe { crate::ime::get_foreground_comp_str_char() },
                gji_candidate_visible: crate::tsf::observer::gji_candidate_visible_now(),
                ime_mode: ime_fsm.state(),
                ime_mode_confirmed: ime_fsm.is_confirmed(),
            }
        };

        // ── Chrome / LiteralDetect / GjiWarmup probe パス（machine は pending_tsf に格納）──
        let machine = self.pending_tsf.borrow_mut().take();
        let Some(mut machine) = machine else {
            return StepProbeResult {
                timer_cmd: TimerCommand::Kill { id: crate::TIMER_TSF_PROBE },
                gji_response: None,
                needs_gji_composition_reset: false,
            };
        };
        log::debug!("[tsf-probe-tick] cold={} t={}ms", machine.cold_seq_hint(), tick_t);
        let actions = machine.tick(&env);
        let dispatch = probe_io::dispatch_probe_actions(machine.as_mut(), actions, self);
        match dispatch {
            probe_io::DispatchResult::Done => {
                self.on_tsf_probe_ready();
                self.gji_end_probe_guard();
                let gji_response = self.gji_take_warmup_result().and_then(|result| {
                    let probe_id = self.current_gji_probe_id.take()?;
                    Some(self.gji_on_event(crate::tsf::gji_fsm::GjiEvent::WarmupComplete {
                        probe_id,
                        result,
                    }))
                });
                let needs_gji_composition_reset = self.pending_gji_composition_reset.take();
                StepProbeResult {
                    timer_cmd: TimerCommand::Kill { id: crate::TIMER_TSF_PROBE },
                    gji_response,
                    needs_gji_composition_reset,
                }
            }
            probe_io::DispatchResult::Continue => {
                let needs_gji_composition_reset = self.pending_gji_composition_reset.take();
                *self.pending_tsf.borrow_mut() = Some(machine);
                StepProbeResult {
                    timer_cmd: TimerCommand::Continue {
                        id: crate::TIMER_TSF_PROBE,
                        delay: Duration::from_millis(10),
                    },
                    gji_response: None,
                    needs_gji_composition_reset,
                }
            }
            probe_io::DispatchResult::SwitchMachine(new_machine) => {
                // GjiWarmupFsm → LiteralDetectFsm 切り替え。
                // LiteralDetectFsm が内部ガードを保持するため gji_probe_guard を解放する。
                self.gji_end_probe_guard();
                let needs_gji_composition_reset = self.pending_gji_composition_reset.take();
                *self.pending_tsf.borrow_mut() = Some(new_machine);
                StepProbeResult {
                    timer_cmd: TimerCommand::Continue {
                        id: crate::TIMER_TSF_PROBE,
                        delay: Duration::from_millis(10),
                    },
                    gji_response: None,
                    needs_gji_composition_reset,
                }
            }
        }
    }

    /// probe を `pending_tsf` にセットする。既存 probe があれば上書きして warn を出す。
    ///
    /// 直接 `*self.pending_tsf.borrow_mut() = Some(...)` するのではなくこのメソッドを使うことで、
    /// 暗黙のキャンセルをログに残し、バグ調査を容易にする。
    pub(super) fn install_pending_tsf(&self, machine: Box<dyn crate::tsf::tickable_fsm::TickableFsm>) {
        let mut slot = self.pending_tsf.borrow_mut();
        if slot.is_some() {
            log::warn!(
                "[tsf-probe] overwriting in-flight probe with new probe cold={}",
                machine.cold_seq_hint()
            );
        }
        *slot = Some(machine);
    }

    /// Chrome/LiteralDetect/GjiWarmup probe が実行中なら継続タイマー命令を返す。
    ///
    /// `send_keys` 完了後の補完に使う。
    pub(crate) fn pending_tsf_timer(&self) -> Option<TimerCommand> {
        self.pending_tsf.borrow().is_some().then_some(TimerCommand::Continue {
            id: crate::TIMER_TSF_PROBE,
            delay: Duration::from_millis(10),
        })
    }

    /// sacr-warmup probe に StartComposition が観測されたことを通知する。
    ///
    /// `platform.rs::drain_pending_composition_events` が StartComposition を処理した際に呼ぶ。
    /// VK_A+BS atomic batch で SHOW+HIDE が最初の tick より前に完了したケースを検出するため、
    /// `SacrificialWarmupFsm::composition_was_seen` フラグをセットする。
    pub(crate) fn notify_probe_start_composition(&self) {
        if let Some(machine) = self.pending_tsf.borrow_mut().as_mut() {
            machine.notify_start_composition();
        }
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
    /// `RAW_TSF_LITERAL` グローバルに backs と romaji を書き込む。
    ///
    /// `RawTsfLiteralRecovery` 処理で `consecutive == 0` のときのみ呼ぶ。
    /// `flush_raw_tsf_literal_backspaces` と `flush_raw_tsf_literal_romaji` の read 側と
    /// ここの write 側を `Output` に集約し、dispatcher が直接グローバルを触らないようにする。
    #[allow(clippy::unused_self)]
    pub(crate) fn record_raw_tsf_literal(&self, backs: usize, romaji: String) {
        use std::sync::atomic::Ordering::Relaxed;
        crate::RAW_TSF_LITERAL.backs.store(backs, Relaxed);
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
        assert_eq!(ColdReason::SessionExpired.eager_settle_ms(false), 500);
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
        assert_eq!(ColdReason::SessionExpired.eager_settle_ms(true), 500);
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
        assert_eq!(ColdReason::SessionExpired.probe_min_ms(false), 200);
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
        assert!(!ColdReason::SessionExpired.is_confirm_key());
        assert!(!ColdReason::RawTsfLiteralRecovery.is_confirm_key());
        assert!(!ColdReason::SetOpenFalse.is_confirm_key());
    }

    #[test]
    fn cold_reason_requires_settle() {
        assert!(ColdReason::FocusChange.requires_settle());
        assert!(ColdReason::NativeF2Consumed.requires_settle());
        assert!(ColdReason::SetOpenTrue.requires_settle());
        assert!(!ColdReason::PassthroughConfirmKey.requires_settle());
        assert!(!ColdReason::SessionExpired.requires_settle());
        assert!(!ColdReason::RawTsfLiteralRecovery.requires_settle());
        assert!(!ColdReason::SetOpenFalse.requires_settle());
    }

    // ── Output 状態管理テスト ───────────────────────────────────────────────────

    fn make_output() -> Output {
        Output::new(OutputMode::Unicode)
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
        o.mark_composition_cold(ColdReason::SessionExpired);
        assert_eq!(o.composition.last_cold_reason(), ColdReason::SessionExpired);
        o.mark_composition_cold(ColdReason::RawTsfLiteralRecovery);
        assert_eq!(
            o.composition.last_cold_reason(),
            ColdReason::RawTsfLiteralRecovery
        );
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
