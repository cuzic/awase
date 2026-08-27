use crate::state::event_origin::Generation;
use crate::tsf::probe_bridge::OutputActiveGuard;
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

/// give-up 由来の GJI reinit 予約結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScheduleGjiReinitResult {
    /// reinit を予約した。`WM_DRAIN_OUTPUT_QUEUE` で raw cleanup 後に開始する。
    Scheduled,
    /// 既に retry 付き reinit poll が進行中のため、新しい give-up は抑止した。
    SuppressedExistingPoll {
        existing_cold_seq: Generation,
        poll_token: u32,
        age_ms: u64,
    },
    /// 直前の give-up が予約した reinit がまだ `WM_DRAIN_OUTPUT_QUEUE` で
    /// 実送信されていない（`Scheduled` のまま、guard もまだ無い）段階で、
    /// 新しい give-up が来た。コードレビュー指摘: この段階を無条件上書き
    /// すると、先行 give-up の romaji と `RAW_TSF_LITERAL`（単一グローバル
    /// スロット）の backspace 数が後勝ちで消え、retry も cleanup も一切
    /// 行われないまま文字が失われる — ADR-101 が直そうとしている症状その
    /// ものが再演する。`Polling`（実送信済み・guard保持中）と同様、上書き
    /// せず新しい give-up 側を抑止する。
    SuppressedExistingScheduled { existing_cold_seq: Generation },
}

/// raw literal cleanup 後に pending reinit を開始した結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GjiReinitStartResult {
    None,
    SkippedRateLimited,
    StartedNoRetry,
    StartedRetryPolling { poll_token: u32 },
    AbortedFocusStale,
    AlreadyPolling,
}

impl GjiReinitStartResult {
    const fn should_flush_stale_deferred_after_raw_recovery(self) -> bool {
        !matches!(
            self,
            Self::StartedRetryPolling { .. } | Self::AlreadyPolling
        )
    }
}

/// async IMC poll の完了状態。`WM_GJI_REINIT_RETRY_COMPLETE` の lParam にも使う。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GjiReinitPollStatus {
    Confirmed = 0,
    Timeout = 1,
    Stale = 2,
}

impl GjiReinitPollStatus {
    pub(crate) const fn encode(self) -> isize {
        self as isize
    }

    pub(crate) const fn decode(value: isize) -> Option<Self> {
        match value {
            0 => Some(Self::Confirmed),
            1 => Some(Self::Timeout),
            2 => Some(Self::Stale),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub(crate) struct PendingGjiReinitCompletion {
    pub cold_seq: Generation,
    pub focus_gen: u32,
    pub retry_romaji: Option<String>,
    pub guard: OutputActiveGuard,
}

#[derive(Debug)]
enum PendingGjiReinitPhase {
    Scheduled {
        /// give-up 検出時点で確保した retry 対象 romaji（`None` = give-up 由来
        /// ではない、または tombstone により retry 権を消費済み）。
        ///
        /// コードレビュー指摘(simplify角度): 以前は `PendingGjiReinitRetry {
        /// romaji, attempted }` という専用構造体で保持していたが、
        /// `attempted` を観測できる経路が実際には存在しなかった
        /// （`take_gji_reinit_completion` は `pending_gji_reinit` ごと
        /// `take()` して消費するため、同じ `retry` に2回アクセスすることが
        /// 構造的にない）。`Option<String>` へ単純化した。
        retry: Option<String>,
    },
    Polling {
        retry: Option<String>,
        guard: OutputActiveGuard,
        poll_token: u32,
        started_ms: u64,
    },
}

#[derive(Debug)]
struct PendingGjiReinit {
    cold_seq: Generation,
    focus_gen: u32,
    phase: PendingGjiReinitPhase,
}

#[derive(Debug)]
struct GjiReinitRetryTombstone {
    focus_gen: u32,
    romaji: String,
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
    /// `send_chrome_gji_reinit_and_poll` を最後に送った時刻（`GetTickCount64` 由来）。
    ///
    /// BUG-33: per-VK confirm の give-up（`RawTsfLiteralRecovery` 連続失敗）から
    /// この reinit を呼ぶ経路を追加したため、短時間に連続 give-up した場合に
    /// `VK_IME_OFF→VK_IME_ON` の SendInput バーストが多重発火しないようレート制限する。
    /// `CHROME_GJI_REINIT_CONFIRM_MS` のポーリング窓が終わる前の再発火を抑止する。
    pub(crate) last_gji_reinit_ms: std::cell::Cell<u64>,
    /// `RawTsfLiteralRecovery` give-up 分岐から予約された Chrome GJI reinit。
    ///
    /// BUG-36: give-up 分岐は `set_raw_literal` で backspace を予約すると同時に
    /// reinit（`VK_IME_OFF`→`VK_IME_ON`）を要求するが、backspace の実送信は
    /// `WM_DRAIN_OUTPUT_QUEUE` まで遅延される。reinit を同期的に即送信すると、
    /// `VK_IME_OFF` が未確定の preedit を commit してしまい、その後に届く backspace が
    /// commit 済み文字を確実に消せないレース（backspace より reinit が先に外へ出る）
    /// が起きる。そのため reinit 本体はここに予約だけして、
    /// `flush_raw_tsf_literal_recovery`（backspace 送信の直後）で実行する。
    pending_gji_reinit: std::cell::RefCell<Option<PendingGjiReinit>>,
    /// retry 付き reinit poll completion を識別する単調増加 token。
    next_gji_reinit_retry_token: std::cell::Cell<u32>,
    /// 同一 give-up romaji を最大1回だけ retry するための tombstone。
    gji_reinit_retry_tombstone: std::cell::RefCell<Option<GjiReinitRetryTombstone>>,
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
            last_gji_reinit_ms: std::cell::Cell::new(0),
            pending_gji_reinit: std::cell::RefCell::new(None),
            next_gji_reinit_retry_token: std::cell::Cell::new(1),
            gji_reinit_retry_tombstone: std::cell::RefCell::new(None),
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

    pub(crate) fn current_ime_mode_focus_gen(&self) -> u32 {
        self.ime_mode_focus_gen.get()
    }

    pub(crate) fn schedule_pending_gji_reinit(
        &self,
        cold_seq: Generation,
        focus_gen: u32,
        retry_romaji: Option<String>,
        consecutive_before: u32,
    ) -> ScheduleGjiReinitResult {
        let mut pending = self.pending_gji_reinit.borrow_mut();
        if let Some(existing) = pending.as_ref() {
            match existing.phase {
                PendingGjiReinitPhase::Polling {
                    poll_token,
                    started_ms,
                    ..
                } => {
                    let age_ms = crate::hook::current_tick_ms().saturating_sub(started_ms);
                    log::warn!(
                        "[chrome-reinit-retry] suppress give-up while poll in flight: \
                         new_cold={} existing_cold={} token={} age_ms={} consecutive_before={}",
                        cold_seq.value(),
                        existing.cold_seq.value(),
                        poll_token,
                        age_ms,
                        consecutive_before,
                    );
                    return ScheduleGjiReinitResult::SuppressedExistingPoll {
                        existing_cold_seq: existing.cold_seq,
                        poll_token,
                        age_ms,
                    };
                }
                PendingGjiReinitPhase::Scheduled { .. } => {
                    // コードレビュー指摘: ここを無条件上書きすると、まだ実送信前の
                    // 先行 give-up の romaji と RAW_TSF_LITERAL の backspace 数が
                    // 後勝ちで失われ、retry も cleanup も一切行われないまま文字が
                    // 消える（ADR-101 が直そうとしている症状そのものの再演）。
                    // Polling と同様、上書きせず新しい give-up 側を抑止する。
                    log::warn!(
                        "[chrome-reinit-retry] suppress give-up while earlier reinit still \
                         scheduled (not yet flushed): new_cold={} existing_cold={} \
                         consecutive_before={}",
                        cold_seq.value(),
                        existing.cold_seq.value(),
                        consecutive_before,
                    );
                    return ScheduleGjiReinitResult::SuppressedExistingScheduled {
                        existing_cold_seq: existing.cold_seq,
                    };
                }
            }
        }
        let retry = retry_romaji.and_then(|romaji| {
            let duplicate = self
                .gji_reinit_retry_tombstone
                .borrow()
                .as_ref()
                .is_some_and(|t| t.focus_gen == focus_gen && t.romaji == romaji);
            if duplicate {
                log::warn!(
                    "[chrome-reinit-retry] suppress duplicate retry reservation: \
                     cold={} focus_gen={} romaji={:?}",
                    cold_seq.value(),
                    focus_gen,
                    romaji,
                );
                None
            } else {
                Some(romaji)
            }
        });
        *pending = Some(PendingGjiReinit {
            cold_seq,
            focus_gen,
            phase: PendingGjiReinitPhase::Scheduled { retry },
        });
        ScheduleGjiReinitResult::Scheduled
    }

    pub(crate) fn has_polling_gji_reinit_retry(&self) -> bool {
        self.pending_gji_reinit
            .borrow()
            .as_ref()
            .is_some_and(|pending| {
                matches!(
                    pending.phase,
                    PendingGjiReinitPhase::Polling { retry: Some(_), .. }
                )
            })
    }

    fn next_gji_reinit_retry_token(&self) -> u32 {
        let token = self.next_gji_reinit_retry_token.get();
        self.next_gji_reinit_retry_token
            .set(token.wrapping_add(1).max(1));
        token
    }

    pub(crate) fn start_pending_gji_reinit_after_raw_cleanup(&self) -> GjiReinitStartResult {
        let pending = self.pending_gji_reinit.borrow_mut().take();
        let Some(pending) = pending else {
            return GjiReinitStartResult::None;
        };
        let PendingGjiReinitPhase::Scheduled { retry } = pending.phase else {
            *self.pending_gji_reinit.borrow_mut() = Some(pending);
            return GjiReinitStartResult::AlreadyPolling;
        };
        let current_focus_gen = self.current_ime_mode_focus_gen();
        if current_focus_gen != pending.focus_gen {
            log::warn!(
                "[chrome-reinit-retry] abort scheduled reinit before send: cold={} \
                 origin_focus_gen={} current_focus_gen={}",
                pending.cold_seq.value(),
                pending.focus_gen,
                current_focus_gen,
            );
            return GjiReinitStartResult::AbortedFocusStale;
        }
        let has_retry = retry.is_some();
        let poll_token = has_retry.then(|| self.next_gji_reinit_retry_token());
        let guard = has_retry.then(OutputActiveGuard::begin);
        let started = {
            use probe_io::ProbeIo as _;
            self.send_chrome_gji_reinit_and_poll(pending.cold_seq, pending.focus_gen, poll_token)
        };
        if !started {
            drop(guard);
            return GjiReinitStartResult::SkippedRateLimited;
        }
        if let (Some(guard), Some(poll_token)) = (guard, poll_token) {
            *self.pending_gji_reinit.borrow_mut() = Some(PendingGjiReinit {
                cold_seq: pending.cold_seq,
                focus_gen: pending.focus_gen,
                phase: PendingGjiReinitPhase::Polling {
                    retry,
                    guard,
                    poll_token,
                    started_ms: crate::hook::current_tick_ms(),
                },
            });
            GjiReinitStartResult::StartedRetryPolling { poll_token }
        } else {
            GjiReinitStartResult::StartedNoRetry
        }
    }

    pub(crate) fn take_gji_reinit_completion(
        &self,
        poll_token: u32,
    ) -> Option<PendingGjiReinitCompletion> {
        let mut pending_slot = self.pending_gji_reinit.borrow_mut();
        let pending = pending_slot.take()?;
        let PendingGjiReinitPhase::Polling {
            retry,
            guard,
            poll_token: existing_token,
            started_ms,
            ..
        } = pending.phase
        else {
            *pending_slot = Some(pending);
            return None;
        };
        if existing_token != poll_token {
            log::warn!(
                "[chrome-reinit-retry] stale completion token={} expected={} cold={}",
                poll_token,
                existing_token,
                pending.cold_seq.value(),
            );
            *pending_slot = Some(PendingGjiReinit {
                cold_seq: pending.cold_seq,
                focus_gen: pending.focus_gen,
                phase: PendingGjiReinitPhase::Polling {
                    retry,
                    guard,
                    poll_token: existing_token,
                    started_ms,
                },
            });
            return None;
        }
        // `pending_slot.take()` でこの `retry` を所有する `pending` ごと
        // 消費済みなので、そのまま Completion へ渡してよい（同じ retry に
        // 2回アクセスする経路は無い。以前の `attempted` フラグはこの不変条件
        // を守るためだけの死んだガードだった）。
        Some(PendingGjiReinitCompletion {
            cold_seq: pending.cold_seq,
            focus_gen: pending.focus_gen,
            retry_romaji: retry,
            guard,
        })
    }

    pub(crate) fn mark_gji_reinit_retry_attempted(&self, focus_gen: u32, romaji: String) {
        *self.gji_reinit_retry_tombstone.borrow_mut() =
            Some(GjiReinitRetryTombstone { focus_gen, romaji });
    }

    pub(crate) fn clear_gji_reinit_retry_tombstone(&self) {
        self.gji_reinit_retry_tombstone.borrow_mut().take();
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
    pub(crate) fn send_unicode_cold_warmup_keys(&self, cold_seq: Generation) {
        use crate::tsf::output::{make_key_input_ex, IME_KANJI_MARKER, INJECTED_MARKER};
        use crate::vk::{VK_A, VK_BACK, VK_IME_ON};

        let ime_on_inputs = [
            make_key_input_ex(VK_IME_ON, false, IME_KANJI_MARKER),
            make_key_input_ex(VK_IME_ON, true, IME_KANJI_MARKER),
        ];
        log::debug!(
            "[unicode-cold-warmup] cold={cold_seq} VK_IME_ON 送信 (ひらがなモード切替)",
            cold_seq = cold_seq.value(),
        );
        let _ = crate::win32::send_input_safe(&ime_on_inputs);
        self.ime_mode_fsm.borrow_mut().on_f21_sent();

        let sacr_inputs = [
            make_key_input_ex(VK_A, false, INJECTED_MARKER),
            make_key_input_ex(VK_A, true, INJECTED_MARKER),
            make_key_input_ex(VK_BACK, false, INJECTED_MARKER),
            make_key_input_ex(VK_BACK, true, INJECTED_MARKER),
        ];
        log::debug!(
            "[unicode-cold-warmup] cold={cold_seq} VK_A+BS 犠牲キー送信 (gji_write_bytes 上昇待ち)",
            cold_seq = cold_seq.value(),
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

    /// VK_IME_OFF → VK_IME_ON の連続送信を ImeModeFsm に通知する。
    ///
    /// `send_chrome_gji_reinit_and_poll` で使う。
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
    /// 次の VK / TSF composition 送信時に VK_IME_ON ウォームアップを
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
        if matches!(reason, ColdReason::FocusChange | ColdReason::SetOpenTrue) {
            self.clear_gji_reinit_retry_tombstone();
        }
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
    /// 現在の warmup 戦略が物理 F2 の代替として VK_IME_ON を自前送信するか（= GJI 戦略か）。
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
        self.clear_gji_reinit_retry_tombstone();
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
    /// `warmup_ime_on`: warmup を送ってよいかの判定に使う IME 開状態（ADR-098
    /// 決定1-b）。`WarmupImeOn` の構築経路は `applied` が既知ならそれを、
    /// `Unknown` のときだけ belief にフォールバックする——呼び出し側が
    /// `unwrap_or(false)` を書く必要はもう無い。
    #[must_use]
    pub fn tsf_readiness(
        &self,
        warmup_ime_on: awase::platform::WarmupImeOn,
    ) -> crate::tsf::TsfReadiness {
        crate::tsf::TsfReadiness {
            gate: self.tsf_gate.state(),
            ime_on: warmup_ime_on.is_on(),
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
    /// `warmup_ime_on`: 呼び出し元が知っている IME 開閉状態（ADR-098 決定1-b、
    /// `WarmupImeOn` 参照）。`is_on()==false` または TSF モード以外では何もしない。
    ///
    /// 実際に送信できた場合のみ `eager_warmup_sent_ms` を現在時刻で更新する。Win キー
    /// 押下中で送信がスキップされた場合は更新しない（BUG-32: スキップを送信成功扱いに
    /// すると、GJI に IME-ON 信号が一度も届かないまま belief だけ ON 確定する）。
    /// 送信できた場合、NativeF2Consumed 等の前に `mark_composition_cold` が呼ばれて
    /// 0 にリセットされるため二重更新は発生しない。
    pub fn send_eager_tsf_warmup(&self, warmup_ime_on: awase::platform::WarmupImeOn) {
        if !self.conv_mutation_allowed.get() {
            log::trace!("[tsf-eager-warmup] non-AwaseOwned → warmup スキップ");
            return;
        }
        if !self.warmup_coord.needs_f2_probe() {
            log::trace!("[tsf-eager-warmup] non-GJI strategy → warmup スキップ");
            return;
        }
        if !self.tsf_readiness(warmup_ime_on).can_warmup() {
            return;
        }
        // OBJ_NAMECHANGE 連番をリセット（warmup 後のイベント順序追跡用）
        crate::tsf::observer::reset_namechange_seq();
        // カタカナ/英数系 charset への追従 warmup（F1/F0 系）は BUG-19 のロックイン
        // 事故を受けて撤去した（`docs/known-bugs.md` BUG-19 参照）。常に VK_IME_ON
        // のみを送る（open 軸のみの冪等キーなため反復送信も無害。2026-08-22、
        // ADR-100 決定2により VK_DBE_HIRAGANA から変更——後者は「開く」と「ひらがなに
        // 強制する」を束ねており BUG-50 デッドロックの前提だった）。
        match crate::tsf::send::send_eager_warmup_vk_pair() {
            Some(ms) => {
                log::debug!("[tsf-eager-warmup] VK_IME_ON 送信, eager_warmup_sent_ms={ms}ms");
                self.composition.set_eager_warmup_sent_ms(ms);
            }
            None => {
                log::debug!(
                    "[tsf-eager-warmup] スキップ (Win key held) → eager_warmup_sent_ms は \
                     更新しない (BUG-32)"
                );
            }
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
    ///
    /// 呼び出し元 (`vk_send.rs` の `send_char_as_tsf`/`send_char_as_vk`) は
    /// `CharResolution::Vk` の生 VK フォールバック経路にあり、2026-08-05 の
    /// BUG-47 追補修正で `vk_pair_to_ascii` が `build_symbol_to_vk` の全記号を
    /// カバーするようになったため、現状この2箇所は理論上到達しない
    /// （`docs/known-bugs.md` BUG-47 参照）。
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
        let focus_gen = self.current_ime_mode_focus_gen();
        let _ = self.send_chrome_gji_reinit_and_poll(Generation::INITIAL, focus_gen, None);
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
        log::debug!(
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
    fn raw_recovery_owns_deferred(&self) -> bool {
        use std::sync::atomic::Ordering::Relaxed;
        crate::RAW_TSF_LITERAL.backs.load(Relaxed) != 0
            || !crate::RAW_TSF_LITERAL
                .romaji
                .lock()
                .expect("RAW_TSF_LITERAL.romaji mutex poisoned")
                .is_empty()
            || self.pending_gji_reinit.borrow().is_some()
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
            log::debug!(
                "[stage-end] {:?}: deferred の解放は raw recovery 側に委ねる",
                end.reason
            );
        } else {
            let n = self.flush_pending_deferred_vks();
            if n > 0 {
                log::debug!("[stage-end] {:?}: deferred {n} VK(s) を flush", end.reason);
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
            log::warn!(
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
        log::debug!("[raw-tsf-literal] re-sending raw TSF literal romaji={romaji:?}");
        self.send_romaji_dispatching_on_gate(&romaji);
    }

    /// give-up 後、reinit の IMC poll が Hiragana 復帰を確認できた場合に限り、
    /// 保存しておいた romaji を一度だけ通常送信経路へ戻す（ADR-101）。
    pub(crate) fn resend_gji_reinit_retry_romaji(&self, romaji: &str) {
        log::warn!("[chrome-reinit-retry] retry romaji via normal path: {romaji:?}");
        self.send_romaji_dispatching_on_gate(romaji);
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
        if self.gate_is_bypass() {
            self.send_romaji_batched(romaji);
        } else {
            self.send_romaji_as_tsf(romaji);
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
    pub fn flush_raw_tsf_literal_recovery(&self) {
        if self.discard_raw_recovery_if_focus_stale() {
            return;
        }
        flush_raw_tsf_literal_backspaces();
        self.flush_raw_tsf_literal_romaji();
        let start_result = self.start_pending_gji_reinit_after_raw_cleanup();
        if !start_result.should_flush_stale_deferred_after_raw_recovery() {
            log::debug!(
                "[raw-tsf-literal] skip stale deferred flush while GJI reinit retry is polling: \
                 result={start_result:?}"
            );
            return;
        }
        self.flush_stale_deferred_vks_after_recovery();
    }

    /// give-up 検出時点の focus 世代と、実際に `WM_DRAIN_OUTPUT_QUEUE` が処理される
    /// 時点の focus 世代を、backspace/romaji を送信する**前**に照合する。
    ///
    /// 対象は `pending_gji_reinit.phase == Scheduled`（直前の give-up が予約した、
    /// まだ実送信していない reinit）のみ。`Polling`（無関係な別の give-up 由来で
    /// 既にポーリング中）はここでは触らない — `start_pending_gji_reinit_after_raw_cleanup`
    /// 側の `AlreadyPolling` 分岐が扱う、stale focus とは無関係な cleanup である。
    ///
    /// ADR-101 決定3・BUG-74 コードレビュー指摘: 旧実装は
    /// `flush_raw_tsf_literal_backspaces()` を先に実行してから
    /// `start_pending_gji_reinit_after_raw_cleanup()` 内で focus 世代を照合していたため、
    /// give-up 検出後に focus が別ウィンドウへ移った場合、**backspace が新ウィンドウへ
    /// 送られてから**ようやく stale 判定されていた。これは ADR-100 が最初から懸念していた
    /// 「別ウィンドウへの誤送信」を、判定タイミングの違いで再導入していた。
    /// backspace/romaji 送信そのものより前に照合することで、この経路を塞ぐ。
    fn discard_raw_recovery_if_focus_stale(&self) -> bool {
        let stale_origin = {
            let pending = self.pending_gji_reinit.borrow();
            pending.as_ref().and_then(|p| {
                matches!(p.phase, PendingGjiReinitPhase::Scheduled { .. })
                    .then_some((p.cold_seq, p.focus_gen))
            })
        };
        let Some((cold_seq, origin_focus_gen)) = stale_origin else {
            return false;
        };
        let current_focus_gen = self.current_ime_mode_focus_gen();
        if current_focus_gen == origin_focus_gen {
            return false;
        }
        self.pending_gji_reinit.borrow_mut().take();
        let (backs, romaji) = crate::RAW_TSF_LITERAL.take_pending();
        crate::RAW_TSF_LITERAL
            .escape_composition
            .store(false, std::sync::atomic::Ordering::Relaxed);
        let discarded_deferred = self.discard_pending_deferred_after_stale_gji_reinit();
        log::warn!(
            "[raw-tsf-literal] discard raw recovery: focus changed since give-up detection \
             cold={} origin_focus_gen={origin_focus_gen} current_focus_gen={current_focus_gen} \
             backs={backs} romaji_present={} discarded_deferred={discarded_deferred}",
            cold_seq.value(),
            !romaji.is_empty(),
        );
        true
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
    fn flush_stale_deferred_vks_after_recovery(&self) {
        if self.has_polling_gji_reinit_retry() {
            log::debug!(
                "[raw-tsf-literal] stale deferred flush postponed: GJI reinit retry polling"
            );
            return;
        }
        let len = self.flush_pending_deferred_vks();
        if len > 0 {
            log::debug!(
                "[raw-tsf-literal] give-up 後に取り残されていた deferred {len} VK(s) を flush"
            );
        }
    }

    pub(crate) fn flush_deferred_vks_after_gji_reinit_completion(&self) -> usize {
        let len = self.flush_pending_deferred_vks();
        if len > 0 {
            log::debug!("[chrome-reinit-retry] completion後に deferred {len} VK(s) を flush");
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
        let marker = if self.gate_is_bypass() {
            VkMarker::InjectedWithScan
        } else {
            VkMarker::Tsf
        };
        self.send_deferred_vks(&vks, marker);
        len
    }

    pub(crate) fn discard_pending_deferred_after_stale_gji_reinit(&self) -> usize {
        let vks = self.warmup_coord.take_pending_deferred();
        let len = vks.len();
        if len > 0 {
            log::warn!("[chrome-reinit-retry] discard deferred {len} VK(s) after stale completion");
        }
        len
    }
}

pub use crate::tsf::output::flush_raw_tsf_literal_backspaces;

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn started_retry_polling_skips_raw_recovery_stale_deferred_flush() {
        assert!(
            !GjiReinitStartResult::StartedRetryPolling { poll_token: 1 }
                .should_flush_stale_deferred_after_raw_recovery(),
            "retry poll confirmed待ち中は pending_deferred が retry を追い越さないよう \
             raw recovery末尾の stale deferred flush を抑止する"
        );
        assert!(
            GjiReinitStartResult::StartedNoRetry.should_flush_stale_deferred_after_raw_recovery()
        );
        assert!(GjiReinitStartResult::SkippedRateLimited
            .should_flush_stale_deferred_after_raw_recovery());
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

    #[test]
    fn discard_raw_recovery_if_focus_stale_clears_state_when_focus_mismatched() {
        let o = make_output();
        o.ime_mode_focus_gen.set(2);
        *o.pending_gji_reinit.borrow_mut() = Some(PendingGjiReinit {
            cold_seq: Generation::INITIAL,
            focus_gen: 1,
            phase: PendingGjiReinitPhase::Scheduled { retry: None },
        });
        crate::RAW_TSF_LITERAL.set_pending(2, "ko".to_owned());

        let discarded = o.discard_raw_recovery_if_focus_stale();

        assert!(
            discarded,
            "origin_focus_gen(1) != current(2) なら discard すべき"
        );
        assert!(
            o.pending_gji_reinit.borrow().is_none(),
            "discard 後は pending_gji_reinit を残さない"
        );
        let (backs, romaji) = crate::RAW_TSF_LITERAL.take_pending();
        assert_eq!(
            (backs, romaji.as_str()),
            (0, ""),
            "discard が RAW_TSF_LITERAL を先に消費しているべき（後続の flush_raw_tsf_literal_backspaces \
             が誤って新フォーカスへ送らないように）"
        );
    }

    #[test]
    fn discard_raw_recovery_if_focus_stale_leaves_state_when_focus_matches() {
        let o = make_output();
        o.ime_mode_focus_gen.set(1);
        *o.pending_gji_reinit.borrow_mut() = Some(PendingGjiReinit {
            cold_seq: Generation::INITIAL,
            focus_gen: 1,
            phase: PendingGjiReinitPhase::Scheduled { retry: None },
        });
        crate::RAW_TSF_LITERAL.set_pending(2, "ko".to_owned());

        let discarded = o.discard_raw_recovery_if_focus_stale();

        assert!(!discarded, "focus 世代が一致するなら discard しない");
        assert!(
            o.pending_gji_reinit.borrow().is_some(),
            "focus 一致時は pending_gji_reinit をそのまま残す"
        );
        let (backs, romaji) = crate::RAW_TSF_LITERAL.take_pending();
        assert_eq!(
            (backs, romaji.as_str()),
            (2, "ko"),
            "focus 一致時は RAW_TSF_LITERAL を消費せず後続の実送信に委ねる"
        );
    }

    #[test]
    fn discard_raw_recovery_if_focus_stale_ignores_polling_phase() {
        // Polling は別の give-up 由来で既にポーリング中の reinit。ここで stale
        // 判定してしまうと、無関係な直近の cleanup まで巻き込んで discard して
        // しまう（AlreadyPolling は start_pending_gji_reinit_after_raw_cleanup 側の
        // 責務）。
        let o = make_output();
        o.ime_mode_focus_gen.set(2);
        *o.pending_gji_reinit.borrow_mut() = Some(PendingGjiReinit {
            cold_seq: Generation::INITIAL,
            focus_gen: 1,
            phase: PendingGjiReinitPhase::Polling {
                retry: None,
                guard: OutputActiveGuard::begin(),
                poll_token: 7,
                started_ms: 0,
            },
        });
        crate::RAW_TSF_LITERAL.set_pending(2, "ko".to_owned());

        let discarded = o.discard_raw_recovery_if_focus_stale();

        assert!(!discarded, "Polling 中の pending は対象外");
        assert!(o.pending_gji_reinit.borrow().is_some());
        let (backs, romaji) = crate::RAW_TSF_LITERAL.take_pending();
        assert_eq!((backs, romaji.as_str()), (2, "ko"));
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
