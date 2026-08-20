use crate::focus::{AppKind, FocusKind};
use awase::engine::InputModeState;

use super::belief::ImeBelief;
use super::evidence::{self, IntentWitness, Observed};
use super::force_guard::{ForceGuard, ForceOnReason};
use super::hook_state::SyncKeyGate;
use super::ime_event::{
    ChordKind, HwndId, ImeEvent, ImeEventEnvelope, ImePolicyProfile, InputModeApplyResult,
    InputModeApplyStrategy, ObservationConfidence, ObservationSource, UserIntentSource,
};
use super::ime_event_log::ImeEventLog;
use super::ime_model::{AppliedImeState, ImeModel};
use super::input_barrier::InputBarrier;
use super::TickMs;
use crate::journal::{JournalEntry, UnifiedJournal};

// ────────────────────────────────────────────────────────────────────────────
// ImeStateHub
// ────────────────────────────────────────────────────────────────────────────

/// IME 観測・判断を担う凝集ユニット。
///
/// `PlatformState` から IME 関連フィールドを切り出すことで、
/// 「観測」「フォーカス状態」「フック設定」の混在を解消する。
///
/// - `belief`        : input_mode / is_japanese_ime / prev_conversion_mode（IME ON/OFF 自体は shadow_model が SSOT）
/// - `shadow_model`  : IME ON/OFF と force_guards / observe_miss_monitor を持つ SSOT
#[derive(Debug)]
pub(crate) struct ImeStateHub {
    /// input_mode・is_japanese_ime・prev_conversion_mode を保持する。
    pub(crate) belief: ImeBelief,
    /// IME 状態変更 event のリングバッファ (Step 0)。
    pub(crate) event_log: ImeEventLog,
    /// 統合ジャーナル: エンジン + IME 両イベントを記録する。
    pub(crate) journal: UnifiedJournal,

    /// Shadow IME モデル (Step 1)。Phase 3a で recovery 統合済。
    /// IME ON/OFF (desired_open / applied_open) と force_guards / observe_miss_monitor を持つ SSOT。
    shadow_model: ImeModel,

    /// ユーザーが明示的に IME OFF にした最終時刻 (tick_ms)。
    ///
    /// `FocusChanged` でクリアされない永続フィールド。複数の rapid focus 変化が連続する
    /// 場合（仮想デスクトップ切替等）でも、最初のフォーカス変化後に `last_intent` が
    /// クリアされても guard が機能し続けるようにする。
    ///
    /// - SyncKey / PhysicalImeKey / Command による `target=false` で更新。
    /// - SyncKey / PhysicalImeKey / Command による `target=true` でリセット。
    /// - FocusChanged / Recovery / HwndCache ではリセットしない。
    ///
    /// BUG-48 修正（PR #44）により `Command` ソースは `handle_engine_set_open`
    /// （`SetOpenOrigin::ExplicitUserAction`）経由でのみ発行されるようになり、
    /// エンジン内部の対称 echo（`ActivationSync` → `handle_engine_activation_sync`、
    /// こちらは `write_set_open_request` を呼ばない）とは完全に分離された。
    /// つまり `Command` は「Ctrl+無変換 等デフォルトキーバインドでの明示 IME OFF/ON」を
    /// 表す実ユーザー操作専用ソースであり、SyncKey/PhysicalImeKey と同じ扱いにできる。
    last_user_explicit_off_ms: u64,

    /// エンジンが明示的 IME ON/OFF を適用した最終時刻 (tick_ms)。0 = 未操作。
    ///
    /// `handle_engine_set_open` が実際に apply を実行したときに更新される。
    /// idle-conv-check が明示的 IME 操作直後に belief を上書きしないよう
    /// `EXPLICIT_IME_SUPPRESS_MS` の間スキップするために参照する。
    last_explicit_ime_action_ms: u64,

    /// 対象 (`HwndId`) ごとの明示意図ストア（ADR-087 §5 Phase 1' 配線、
    /// BUG-51 追補 v3）。
    ///
    /// `record_explicit_intent`（本物のユーザー操作と確定できる3箇所からのみ
    /// 呼ばれる）が書き込み、`effective_open()` が `FocusChanged` をまたいだ
    /// 明示意図の優先読み取りに使う。`issue_open_warrant()` への配線は
    /// まだ無く、Phase 3 本体のスコープ。
    intent_store: super::intent_store::IntentStore,

    /// `effective_open()` の IntentStore 分岐が `shadow_model` と異なる値を
    /// 返している（＝実際に override している）間 `true`。遷移時のみ INFO
    /// ログを出すための dedup 用（BUG-51 追補 v3）。`&self` の `effective_open()`
    /// から更新するため `Cell`——`ImeStateHub` は単一 UI スレッドが所有する
    /// （`with_app` パターン）ため `!Sync` でも問題ない。
    intent_override_logged: std::cell::Cell<bool>,
}

/// [`ImeStateHub::capture_poll_state`] で取得する IME ポーリング入力スナップショット。
///
/// `poll_and_classify_ime` / `classify_fetched_snapshot` の 4 引数をひとつにまとめることで
/// `ir_poll_and_learn` 内の同一フィールド二重読み取りを解消する。
#[derive(Clone, Copy)]
pub(crate) struct ImePollState {
    pub(crate) ime_on: bool,
    pub(crate) force_guard: bool,
    pub(crate) input_mode: InputModeState,
    pub(crate) prev_conv: Option<u32>,
}

impl ImeStateHub {
    /// デフォルト値で初期化する。
    pub(crate) fn new() -> Self {
        Self {
            belief: ImeBelief::default(),
            event_log: ImeEventLog::default(),
            journal: UnifiedJournal::default(),
            shadow_model: ImeModel::default(),
            last_user_explicit_off_ms: 0,
            last_explicit_ime_action_ms: 0,
            intent_store: super::intent_store::IntentStore::default(),
            intent_override_logged: std::cell::Cell::new(false),
        }
    }
}

impl ImeStateHub {
    /// Event を log に記録し、shadow_model にも reduce する (Step 1)。
    ///
    /// `event_log.record()` だけを呼ぶより、こちらを使うと record + reduce が
    /// 同一 envelope で進む。write_* メソッドはこちらを使う。
    ///
    /// `tick_ms`: 呼び出し元が取得した現在時刻（`GetTickCount64` 由来）。
    /// state/ 層が `hook::current_tick_ms()` を直接呼ばないよう注入する。
    pub(crate) fn dispatch_event(&mut self, event: ImeEvent, tick_ms: TickMs) {
        // ユーザー明示の IME OFF/ON を永続タイムスタンプに反映する。
        // FocusChanged で last_intent がクリアされても guard が機能し続けるよう、
        // ImeStateHub 側で独自に保持する。
        if let ImeEvent::UserImeSetIntent { target, source } = &event {
            if matches!(
                source,
                UserIntentSource::SyncKey
                    | UserIntentSource::PhysicalImeKey
                    | UserIntentSource::Command
            ) {
                if *target {
                    self.last_user_explicit_off_ms = 0;
                } else {
                    self.last_user_explicit_off_ms = tick_ms.0;
                }
                // IntentStore への record() はここでは行わない（BUG-51 追補 v3 で移設）。
                // Command ソースは conv 由来の内部同期（EngineSync::DirectInput →
                // handle_engine_set_open → write_set_open_request）でも dispatch される
                // ため、このイベントだけでは「本物のユーザー操作」と区別できない。
                // 記録は実ユーザー操作と確定できる呼び出し元
                // （record_explicit_intent の doc 参照）が行う。
            }
        }
        let event_for_journal = event.clone();
        let event_for_reduce = event.clone();
        let time = self.event_log.record(event, tick_ms);
        let envelope = ImeEventEnvelope {
            time,
            event: event_for_reduce,
        };
        self.shadow_model.reduce(&envelope);
        self.journal.record(JournalEntry::ImeEvent {
            event: event_for_journal,
        });
    }

    /// shadow_model から派生した最新の explicit intent。
    ///
    /// (Step 2B 以降の SSOT。Priority 4-5 observer による上書きを block する根拠。)
    pub(crate) fn explicit_intent(&self) -> Option<bool> {
        self.shadow_model.last_intent.as_ref().map(|i| i.target)
    }

    /// applied_open / applied_at_ms を更新する（apply 完了時の SSOT 更新）。
    ///
    /// ImeModel アクセス可能なサイトで `set_ime_apply_latch` の代わりに呼ぶ。
    /// executor 内部 (PlatformState 非アクセス) は ImeApplySucceeded event 経由で更新される。
    ///
    /// `tick_ms`: 呼び出し元が取得した現在時刻（`GetTickCount64` 由来）。
    pub(crate) fn mirror_applied_open(&mut self, value: bool, tick_ms: TickMs) {
        self.mirror_applied_open_with_ts(value, tick_ms.0);
    }

    /// `applied` を指定タイムスタンプで更新する。
    ///
    /// `ts = 0` → `Optimistic`（ImmCross async 送信直後など、楽観的未確認）
    /// `ts > 0` → `Confirmed`（実 apply 完了後）
    pub(crate) fn mirror_applied_open_with_ts(&mut self, value: bool, ts: u64) {
        use crate::state::ime_model::AppliedImeState;
        self.shadow_model.applied = if ts == 0 {
            AppliedImeState::Optimistic(value)
        } else {
            AppliedImeState::Confirmed {
                open: value,
                at_ms: ts,
            }
        };
        // 同じ apply が完了した扱いなので pending も clear
        if let Some(p) = &self.shadow_model.pending {
            if p.target == value {
                self.shadow_model.pending = None;
            }
        }
    }

    // ── Chord barrier ──

    pub(crate) const fn is_ctrl_ime_chord_active(&self) -> bool {
        self.shadow_model.is_ctrl_ime_chord_active()
    }

    pub(crate) fn active_chord_kind(&self) -> Option<ChordKind> {
        self.shadow_model.active_chord_kind()
    }

    /// Engine が SetOpen を要求したときの chord-aware 処理を一元化するメソッド。
    ///
    /// chord active + IME OFF の組み合わせは「chord transaction 中の二次要求」として
    /// フィルタする（write_set_open_request と ImeApplyRequested の両方をスキップ）。
    /// パイプラインがコード状態を直接参照しなくて済むよう、判断をここに集約する。
    ///
    /// `tick_ms`: 呼び出し元が取得した現在時刻（`GetTickCount64` 由来）。
    ///
    /// 戻り値: apply 要求が実行されたか（ログ用）
    ///
    /// `focus_transition_was_pending`: この event の処理開始時点（`kp_stage_focus_probe`
    /// が barrier を consume する前）で FocusTransition barrier が settle 期間内だったか。
    /// 呼び出し元はこの値を event 処理の先頭でスナップショットして渡すこと
    /// （本関数の呼び出し時点で `is_focus_transition_settling` を評価しても、既に
    /// consume 済みで false になっているため無意味）。
    pub(crate) fn handle_engine_set_open(
        &mut self,
        target: bool,
        ctrl_held: bool,
        focus_transition_was_pending: bool,
        generation: u64,
        tick_ms: TickMs,
    ) -> bool {
        if self.is_ctrl_ime_chord_active() && !target {
            // chord transaction 中の二次 IME OFF 要求: フィルタ。
            // ChordEnded（Ctrl KeyUp）が barrier を解除するため、ここでは何もしない。
            //
            // 診断ログ (2026-08-05): 従来ここは完全無音だったため、実機ログだけでは
            // 「明示 OFF がこのフィルタでサイレント無効化された」ケースを他の原因と
            // 区別できなかった。挙動は変更しない。
            log::info!(
                "[chord-filter] SetOpen(false) request filtered: ctrl_ime_chord が既に active \
                 (last_intent/desired_open は更新されない)"
            );
            return false;
        }
        if focus_transition_was_pending {
            // belief 保護の最終防衛線（P3-1: 3→2 集約）。
            //
            // 一次フィルタは decision からの SetOpen effect 除去
            // （`runtime::executor::strip_ime_set_open_if_settling`。キーボード経路 =
            // key_pipeline::kp_run_inner と非キーボード経路 = execute_from_loop の両方から呼ぶ）。
            // ここは意図が異なり（decision 除去 ≠ belief 汚染防止）、万一その一次フィルタを
            // すり抜けた SetOpen 要求が belief（desired_open 等）を書き換えるのを防ぐ二重化。
            //
            // フォーカス遷移直後（settle_until 未経過）は、Alt+Tab 等の高速な多重フォーカス遷移で
            // 中間ウィンドウ（Alt+Tab スイッチャー等）の未確定 belief に基づき Engine が SetOpen を
            // 発行し得る（2026-07-05 実機ログで確認）。barrier consume 時に kick される非同期
            // focus probe が観測を更新すれば、次の入力イベントで正しい SetOpen が再発行され自己修復する。
            //
            // 2026-08-05: 実機再発報告の切り分けのため debug → info に格上げ（頻度は低い）。
            log::info!(
                "[focus-settle] SetOpen({target}) request filtered at belief last line of defense \
                 (focus transition barrier still settling at event start)"
            );
            return false;
        }
        self.write_set_open_request(target, tick_ms);
        self.on_set_open_requested();
        self.dispatch_event(
            ImeEvent::ImeApplyRequested {
                target,
                generation,
                ctrl_held,
            },
            tick_ms,
        );
        self.last_explicit_ime_action_ms = tick_ms.0;
        true
    }

    /// `awase::engine::decision::SetOpenOrigin::ActivationSync` 由来の `SetOpen` を処理する。
    ///
    /// `handle_engine_set_open` との違いは唯一つ: `ImeEvent::UserImeSetIntent`（`last_intent`
    /// を設定する）の代わりに `ImeEvent::EngineActivationSync`（`last_intent` を設定しない）を
    /// dispatch する点。この SetOpen は Engine の active/inactive 遷移が対称性のために
    /// 自動発行した echo であり、ユーザーが今このキーで ON/OFF を明示的に選んだわけではない
    /// （`ctx.ime_on` が観測駆動で変化しただけでも Active/Inactive は遷移しうる）。
    /// `last_intent` を設定すると、以後の drift correction がこの echo を「ユーザーの本物の
    /// 意図」として扱ってしまい、ユーザーが明示的に IME を OFF にした直後でも Engine が
    /// 勝手に ON へ戻る再発を引き起こす（2026-08-04、`docs/known-bugs.md` 参照）。
    ///
    /// chord/focus-transition-settling のフィルタ条件は `handle_engine_set_open` と同一
    /// （どちらも「これから OS へ実 apply する SetOpen 要求」という点は変わらないため）。
    ///
    /// `last_explicit_ime_action_ms` は `handle_engine_set_open` と同様に更新する。この
    /// フィールドの実際の役割は「ユーザーが明示操作したか」ではなく「awase 自身が
    /// 能動的に IME へ書き込んだか」（`note_explicit_ime_action` の doc 参照）であり、
    /// この関数も実際に OS へ SetOpen を適用する以上、idle-conv-check が遷移途中の
    /// conv 値を汚染された観測として拾わないよう抑制窓を効かせる必要がある
    /// （Opus レビュー 2026-08-04 で指摘: 更新しないと `get_ime_conversion_mode_raw_timeout_async`
    /// が BUG-34 級にブロックしている間に本関数の SetOpen 適用が挟まった場合、
    /// idle-conv-check のガード (b)（値一致比較）が素通りし、遷移途中の conv が
    /// そのまま belief に入りうる）。
    pub(crate) fn handle_engine_activation_sync(
        &mut self,
        target: bool,
        ctrl_held: bool,
        focus_transition_was_pending: bool,
        generation: u64,
        tick_ms: TickMs,
    ) -> bool {
        if self.is_ctrl_ime_chord_active() && !target {
            // 診断ログ: handle_engine_set_open 側と同じ理由で info に格上げ。
            log::info!(
                "[chord-filter] ActivationSync SetOpen(false) request filtered: \
                 ctrl_ime_chord が既に active"
            );
            return false;
        }
        if focus_transition_was_pending {
            // 2026-08-05: 実機再発報告の切り分けのため debug → info に格上げ。
            log::info!(
                "[focus-settle] ActivationSync SetOpen({target}) request filtered at belief \
                 last line of defense (focus transition barrier still settling at event start)"
            );
            return false;
        }
        self.dispatch_event(ImeEvent::EngineActivationSync { target }, tick_ms);
        self.on_set_open_requested();
        self.dispatch_event(
            ImeEvent::ImeApplyRequested {
                target,
                generation,
                ctrl_held,
            },
            tick_ms,
        );
        self.last_explicit_ime_action_ms = tick_ms.0;
        true
    }

    /// Ctrl 系 KeyUp で chord barrier を解除する。
    ///
    /// パイプラインが chord 状態を直接参照しなくて済むよう、
    /// is_ctrl_ime_chord_active / active_chord_kind の参照をここに集約する。
    /// 呼び出し元は `crate::vk::is_ctrl_variant` チェック後に呼ぶこと。
    ///
    /// `tick_ms`: 呼び出し元が取得した現在時刻（`GetTickCount64` 由来）。
    pub(crate) fn on_ctrl_key_up(&mut self, vk: awase::types::VkCode, tick_ms: TickMs) {
        if !self.is_ctrl_ime_chord_active() {
            return;
        }
        let kind = self
            .active_chord_kind()
            .unwrap_or(ChordKind::CtrlMuhenkanImeOff);
        self.dispatch_event(ImeEvent::ChordEnded { kind }, tick_ms);
        log::debug!("[ctrl-bypass] chord barrier cleared (Ctrl KeyUp vk=0x{vk:02X})");
    }

    // ── Input barrier ──

    /// フォーカス遷移 barrier が pending なら消費して true を返す。
    pub(crate) fn consume_focus_barrier(&mut self) -> bool {
        if self.shadow_model.is_focus_transition_pending() {
            self.shadow_model.input_barrier = None;
            true
        } else {
            false
        }
    }

    /// input_barrier を無条件クリアする（panic reset・フォーカス変更確定等）。
    pub(crate) const fn clear_input_barrier(&mut self) {
        self.shadow_model.input_barrier = None;
    }

    /// FocusTransition barrier が未設定なら設定する。
    pub(crate) fn try_set_focus_transition_barrier(
        &mut self,
        to_hwnd: HwndId,
        started_at: std::time::Instant,
    ) {
        if self.shadow_model.input_barrier.is_none() {
            let settle = self.shadow_model.app_policy.focus_settle_ms;
            self.shadow_model.input_barrier = Some(InputBarrier::FocusTransition {
                to_hwnd,
                started_seq: self.event_log.next_seq(),
                started_at,
                settle_until: started_at + std::time::Duration::from_millis(settle),
            });
        }
    }

    // ── Explicit intent timing ──

    /// 直近の明示的 IME 操作からの経過 ms。
    ///
    /// 未操作の場合は `u64::MAX` を返す。
    /// `EXPLICIT_IME_SUPPRESS_MS` との比較で idle-conv-check を抑制するために使う。
    ///
    /// `now_ms`: 呼び出し元が取得した現在時刻（`GetTickCount64` 由来）。
    /// idle-conv-check 抑止用に「明示的 IME 操作」時刻を記録する。
    ///
    /// `handle_engine_set_open` 以外の能動的 IME 書き込み（Shift 解放時の conv 復元等）
    /// から呼ぶ。`EXPLICIT_IME_SUPPRESS_MS` の間 idle-conv-check がスキップされる。
    pub(crate) fn note_explicit_ime_action(&mut self, tick_ms: TickMs) {
        self.last_explicit_ime_action_ms = tick_ms.0;
    }

    pub(crate) fn explicit_ime_action_age_ms(&self, now_ms: TickMs) -> u64 {
        if self.last_explicit_ime_action_ms == 0 {
            return u64::MAX;
        }
        now_ms.saturating_sub(self.last_explicit_ime_action_ms)
    }

    /// `last_explicit_ime_action_ms` の生値（0 = 未操作）。
    ///
    /// idle-conv-check の spawn 時スナップショットと apply 時の値を突き合わせ、
    /// 「spawn〜apply の間に新しい明示的 IME 操作が起きたか」を経過時間ではなく
    /// 値の一致で判定するために使う。`explicit_ime_action_age_ms` の閾値判定
    /// （`EXPLICIT_IME_SUPPRESS_MS`）は、`get_ime_conversion_mode_raw_timeout_async`
    /// が BUG-34（`SendMessageTimeoutW(SMTO_ABORTIFHUNG)` が指定タイムアウトを無視して
    /// 数秒ブロックしうる）で長時間ブロックした場合、spawn 直後に明示操作が起きても
    /// apply 時点では age が閾値を超えてしまい素通りする穴がある。値の一致比較なら
    /// 遅延の長さに関わらず「spawn 後に何か明示操作があった」事実だけで棄却できる。
    pub(crate) fn last_explicit_ime_action_ms_raw(&self) -> u64 {
        self.last_explicit_ime_action_ms
    }

    /// フォーカス変化をまたいで持続するユーザー明示 IME OFF タイムスタンプ。
    ///
    /// `last_explicit_off_ms()` は `FocusChanged` で `last_intent` がクリアされると 0 に
    /// 戻るため、複数の rapid focus 変化（仮想デスクトップ切替等）では 2 回目以降の
    /// guard が機能しない。このメソッドは SyncKey / PhysicalImeKey / Command による明示 OFF
    /// のみを追跡し、FocusChanged でリセットしない。
    pub(crate) fn persistent_explicit_off_ms(&self) -> u64 {
        self.last_user_explicit_off_ms
    }

    /// `ImeModel::effective_open()`（Engine の `ctx.ime_on` に直結する belief）に、
    /// `IntentStore`（ADR-087 §5 Phase 1'、BUG-51 追補で配線）による上書きを重ねる。
    ///
    /// `ImeModel.last_intent` は `FocusChanged` で無条件にクリアされる
    /// （`ime_model.rs` の `has_user_explicit_intent()` 参照）。同一プロセス内の
    /// 別ウィンドウへの一瞬のフォーカス奪取（BUG-57 の Pushbullet 通知等）や、
    /// スリープ復帰直後のフォーカス再構築を挟むと、直前に押した明示 IME OFF/ON
    /// （Ctrl+無変換 等）の意図が `last_intent` から消え、`effective_open()` が
    /// 観測プールの `derive_open_filtered()`/`most_recent_trusted()` にフォールバック
    /// する。TsfNative（MS-IME 等、conv ビットからの間接推論しか観測源が無い
    /// プロファイル）ではこのフォールバックが `ConvOpenInference`
    /// （`NativeToggleShadowOff`（旧 `KatakanaShadowOff` を統合済み）、conv=NATIVE を「open」と誤読する
    /// BUG-55 由来の壊れた観測）1 件だけで確定してしまい、`desired_open` が正しく
    /// false のままでも `effective_open()` が true に反転する。`Engine::compute_state`
    /// はこれを直接 `ctx.ime_on` として使うため、実 IME は正しく OFF なのに Engine
    /// だけが ON へ再活性化する（2026-08-11 実機再発、`docs/known-bugs.md` BUG-51 追補、
    /// Opus 独立レビュー済み）。
    ///
    /// `IntentStore` は `HwndId` 単位で最後の明示意図を保持し、`FocusChanged` では
    /// クリアされない（ON/OFF 非対称 TTL、`intent_store.rs` 参照）。**同一対象への
    /// フォーカスが戻った場合に限り**、`last_intent` 消失後もこのエントリを
    /// `desired_open` の代わりに使うことで、上記の壊れた観測へのフォールバックを
    /// 回避する（ADR-087 §5 Phase 1' item8 が要求する配線）。`current_focus` は
    /// `FocusChanged`（PID 変化時のみ発火）でしか更新されないため、実効粒度は
    /// 「同一ウィンドウ」ではなく実質「最後に PID が変わった時点の対象」＝
    /// per-process に近い点に注意（pre-mortem #1 角度1/3）。
    ///
    /// **記録対象は本物のユーザー操作 3 箇所に限定される**（`record_explicit_intent`
    /// の doc 参照、BUG-51 追補 v3）。conv 由来の内部同期（`EngineSync::
    /// SetOpen(RomajiRecovered)`/`DirectInput`）はここに記録されない——v1 では
    /// これらも `UserImeSetIntent{Command}` 経由で記録され、壊れた conv 読み1件が
    /// `FocusChanged` を生き延びる偽の明示意図になるという、この override 自体が
    /// 生む新しい退行があった（pre-mortem #1 角度2）。
    ///
    /// `PanicReset` は同じ対象の `IntentStore` エントリを無条件に無効化する
    /// （`apply_panic_reset`、安全弁は時系列比較の余地なく常に最新の決定）。
    /// `HwndCacheRestored` はキャッシュの記録時刻が意図の記録時刻以上の場合のみ
    /// 無効化する（`apply_hwnd_cache_restore`、フォーカス滞在が短くキャッシュ
    /// 保存自体がスキップされた場合に新しい意図をより古いキャッシュへ明け渡さない
    /// ため、pre-mortem #2）。`reset_stale_ime_on_for_imm_broken`（BUG-16 系
    /// safety-net）は有効な `IntentStore` エントリがある間、`Low` confidence の
    /// 安全デフォルトをそもそも書かずに温存する（同、逆転防止）。
    ///
    /// 判定本体は `IntentStore::resolve_effective_open()`（`state/intent_store.rs`、
    /// `#[cfg(windows)]` の**外**）にあり、本メソッドはそこに INFO ログの重複排除を
    /// 被せるだけ。**このモジュールは `#[cfg(windows)]` なので、ここに書いた
    /// `mod tests`（`cfg(test)`）は Linux の `cargo test -p awase-windows` では
    /// 1 件も走らない**——Linux CI で毎回走る回帰は
    /// `tests/intent_store_effective_open.rs` にある。
    ///
    /// # 時刻の出どころ（追補4、2026-08-13 windows-build 失敗の原因）
    ///
    /// `IntentStore` の TTL 判定に使う「現在時刻」は
    /// `crate::hook::current_tick_ms()`（`GetTickCount64`、= OS 起動からの経過 ms）
    /// である。本番では `record_explicit_intent()` に渡る `tick_ms` も同じ
    /// `current_tick_ms()` 由来（`runtime/key_pipeline.rs` の 3 箇所すべて）なので
    /// 整合している。一方、`mod tests` が `TickMs(100)` のような**合成 tick** で
    /// エントリを記録してから引数なしの本メソッドを呼ぶと、実機では
    /// `GetTickCount64()` が数分〜数日を返すため `EXPLICIT_OFF_INTENT_TTL_MS`
    /// (30 秒) を必ず超え、**IntentStore 上書きが一度も発火しない**。合成 tick を
    /// 使うテストは必ず [`Self::effective_open_at`] を呼ぶこと。
    pub(crate) fn effective_open(&self) -> bool {
        self.effective_open_at(TickMs(crate::hook::current_tick_ms()))
    }

    /// [`Self::effective_open`] の判定本体。`now_ms` を明示的に受け取る版。
    ///
    /// 本番の呼び出し口は引数なしの [`Self::effective_open`] 一択（壁時計を読む）。
    /// 合成 tick でイベントを流すテストは、同じ時間軸を渡すためにこちらを使う。
    ///
    /// この形（時刻を注入する）が `state/mod.rs` の `TickMs` doc が定めた
    /// 「state/ 層は `hook::current_tick_ms()` を直接呼ばず、runtime 層から
    /// タイムスタンプを注入する」原則に沿う。`effective_open()` が壁時計を
    /// 読んでいるのは、その 29 箇所ある runtime 側呼び出し元をまだ書き換えて
    /// いないため（追補4 の残タスク、`docs/known-bugs.md` BUG-51 追補4 参照）。
    pub(crate) fn effective_open_at(&self, now_ms: TickMs) -> bool {
        let shadow = self.shadow_model.effective_open();
        let decision = self.intent_store.resolve_effective_open(
            self.shadow_model.current_focus(),
            shadow,
            now_ms,
        );
        match decision.intent {
            Some(intent) if decision.value != shadow => {
                if !self.intent_override_logged.get() {
                    log::info!(
                        "[intent-store] effective_open override 開始: hwnd={:?} \
                         intent.open={} (source={:?}, age={}ms) shadow_model={shadow}",
                        intent.target,
                        intent.open,
                        intent.source,
                        now_ms.0.saturating_sub(intent.recorded_at_ms.0),
                    );
                    self.intent_override_logged.set(true);
                }
            }
            Some(_) => {
                if self.intent_override_logged.get() {
                    log::info!("[intent-store] effective_open override 終了 (shadow が一致)");
                    self.intent_override_logged.set(false);
                }
            }
            None => {
                if self.intent_override_logged.get() {
                    log::info!(
                        "[intent-store] effective_open override 終了 \
                         (intent 消失/期限切れ/フォーカス変更)"
                    );
                    self.intent_override_logged.set(false);
                }
            }
        }
        decision.value
    }

    /// フォーカス切替直後の settle 期間内（`settle_until` 未経過）かどうか。
    pub(crate) fn is_focus_transition_settling(&self, now: std::time::Instant) -> bool {
        self.shadow_model.is_focus_transition_settling(now)
    }

    pub(crate) fn detect_miss_count(&self) -> u32 {
        self.shadow_model
            .observe_miss_monitor
            .consecutive_miss_count
    }

    pub(crate) fn is_force_on_guard_active(&self) -> bool {
        self.shadow_model.force_guards.requires_on()
    }

    /// awase が IME をこうしたい状態を返す（BugReport 診断用）。
    pub(crate) fn desired_open(&self) -> bool {
        self.shadow_model.desired_open()
    }

    /// 現在の入力モードを返す（SSOT = `shadow_model.input_mode`）。
    ///
    /// H-3-d 以降、`belief.input_mode` は private 化されたため、
    /// 呼び出し元はすべてこのメソッドを使うこと。
    pub(crate) fn input_mode(&self) -> InputModeState {
        self.shadow_model.input_mode()
    }

    /// 最後に actuator が成功させた IME 開閉状態の確信度を返す（BugReport 診断用）。
    pub(crate) fn applied_state(&self) -> AppliedImeState {
        self.shadow_model.applied_state()
    }

    /// `poll_and_classify_ime` / `classify_fetched_snapshot` に渡す 4 フィールドを一括取得する。
    ///
    /// `ir_poll_and_learn` で同じ 4 フィールドを 2 回読んでいた重複を解消する。
    pub(crate) fn capture_poll_state(&self) -> ImePollState {
        ImePollState {
            ime_on: self.effective_open(),
            force_guard: self.is_force_on_guard_active(),
            input_mode: self.input_mode(),
            prev_conv: self.belief.prev_conversion_mode(),
        }
    }

    /// `belief.is_japanese_ime() && effective_open()` の複合述語。
    ///
    /// `apply_force_on_for_imm_broken` / `try_force_on_bootstrap` で重複していたガード条件。
    /// `engine.is_user_enabled()` と組み合わせて IME force-ON の前提条件として使う。
    ///
    /// **belief 由来の暫定ゲート（ADR-087 §5 Phase 3 item15 で
    /// `issue_open_warrant()` に置換予定、まだ未配線）。** `effective_open()` は
    /// belief（間違っていても低リスク）であり、actuation の根拠に直接使うべき
    /// ではない——これはまさに本関数が持つ構造であり、BUG-63 の原因パターンが
    /// 実 actuation ゲートとして今も本番で使われている状態を示す。呼び出し元は
    /// 3箇所（`runtime/mod.rs` の `apply_force_on_for_imm_broken` /
    /// `consume_force_open_pending` / `try_force_on_bootstrap`）。
    pub(crate) fn is_eligible_for_ime_force_on(&self) -> bool {
        self.belief.is_japanese_ime() && self.effective_open()
    }

    /// 現在のアプリの focus settle 期間（ms、`AppImePolicy` 由来）。
    ///
    /// settle 中にスキップした force-ON の再試行スケジュールに使う。
    pub(crate) fn focus_settle_ms(&self) -> u64 {
        self.shadow_model.app_policy.focus_settle_ms
    }

    /// 現在のアプリの feedback（収束確認）方針（`AppImePolicy` 由来、ADR-080）。
    ///
    /// `ir_apply_drift_correction` が `Actuation` を構築する際に使う。
    pub(crate) fn default_feedback(&self) -> super::ime_actuation::FeedbackPolicy {
        self.shadow_model.app_policy.default_feedback
    }

    /// 次のイベント generation 番号を払い出す。
    ///
    /// 呼び出し元で `self.platform_state.ime.event_log.next_seq()` を直接書かずに
    /// このメソッドを使うこと（3 段チェーンの解消）。
    pub(crate) fn allocate_event_generation(&self) -> u64 {
        self.event_log.next_seq()
    }

    /// IMM-broken アプリで IME-ON が確認されたとき、`input_mode` を補正すべき値を返す。
    ///
    /// `ImeBelief::correction_for_imm_broken` と同じロジックを `shadow_model.input_mode`
    /// に対して適用する（H-3-d で `belief.input_mode` が private 化されたため移譲）。
    pub(crate) fn correction_for_imm_broken(&self) -> Option<InputModeState> {
        use awase::engine::AssumedReason;
        let mode = self.shadow_model.input_mode();
        if mode.is_romaji_capable() || matches!(mode, InputModeState::ObservedEisu) {
            return None;
        }
        Some(InputModeState::AssumedRomaji {
            reason: AssumedReason::ImmBridgeBroken,
        })
    }

    /// `ImeModel` への読み取り専用アクセス。
    ///
    /// 書き込みはすべて `dispatch_event()` 経由とすること。
    pub(crate) fn model(&self) -> &ImeModel {
        &self.shadow_model
    }

    // ── warrant（ADR-087 / ADR-090 §2.A）──────────────────────────────────

    /// `issue_open_warrant()` が要求する状態一式を組み立てる**唯一の場所**
    /// （ADR-090 INV-48）。
    ///
    /// # なぜ 1 箇所に絞るのか
    ///
    /// `WarrantContext` の 8 材料のうち先頭 5 つ（`intent_store` / `obs` /
    /// `guards` / `policy` / `desired_open`）はすべて `ImeStateHub` 配下に
    /// あり、`intent_store` は**private フィールド**である。実 actuation 入口は
    /// 外部 8 経路あるので（ADR-090 §2.A.2(3)）、各入口がリテラルで
    /// `WarrantContext { .. }` を組み立てると `intent_store` の private を
    /// 崩すか、8 箇所に同じ組み立てが散る（ADR-087 §7 round4 N-A が
    /// `WarrantContext` を導入して避けたかったもの）。本メソッド 1 本だけが
    /// 読む形にすることで、private を維持したまま読み手を集約する。
    /// `tests/architecture_guard.rs::warrant_context_is_built_in_one_place` が
    /// 本番コードに `WarrantContext {` のリテラル構築が無いことを固定する。
    ///
    /// `now` / `now_ms` は呼び出し元が注入する（ADR-087 INV-23:
    /// `issue_open_warrant` は時刻を内部で取らない純粋関数。加えて `state/` 層は
    /// `hook::current_tick_ms()` を直接呼ばない規約）。
    pub(crate) fn warrant_context(
        &self,
        now: std::time::Instant,
        now_ms: TickMs,
    ) -> super::open_warrant::WarrantContext<'_> {
        super::open_warrant::WarrantContext {
            intent_store: &self.intent_store,
            obs: &self.shadow_model.observations,
            guards: &self.shadow_model.force_guards,
            policy: &self.shadow_model.app_policy,
            desired_open: self.shadow_model.desired_open(),
            is_japanese_ime: self.belief.is_japanese_ime(),
            now,
            now_ms,
        }
    }

    /// 実 actuation の 1 件を起案する（ADR-090 §2.A 設計案 1、INV-47）。
    ///
    /// 実 actuation 入口（外部 8 経路）はすべてこれを通る。
    /// `target` は `ImeModel::current_focus()`——`None`（フォーカス不明）の
    /// ときは `HwndId::NULL` を渡す。Step 1（`IntentStore::lookup`）が必ず
    /// 外れるだけで他の Step の判定は変わらない（ADR-090 A-R4）。
    ///
    /// **A-1（shadow）の時点では、返り値の `would_have_blocked` は
    /// ログ・journal にしか効かない。** 書き込みを止めるのは A-2。
    pub(crate) fn issue_actuation_order(
        &self,
        open: bool,
        origin: super::event_origin::EventOrigin,
        now: std::time::Instant,
        now_ms: TickMs,
    ) -> super::actuation_chain::ActuationOrder {
        let target = self.shadow_model.current_focus().unwrap_or(HwndId::NULL);
        let ctx = self.warrant_context(now, now_ms);
        super::actuation_chain::ActuationOrder::issue(open, target, &ctx, origin)
    }

    // ── Desired state / drift correction ──

    /// desired ≠ observed ドリフトが補正閾値を超えているか判定し、超えていれば補正情報を返す。
    ///
    /// 戻り値: `Some((desired, observed, duration_ms))` — 補正が必要な場合
    /// `explicit_intent`: `PlatformState::explicit_intent()` の値をそのまま渡す。
    pub(crate) fn check_drift_correction(
        &self,
        now: std::time::Instant,
        explicit_intent: Option<bool>,
    ) -> Option<(bool, bool, u64)> {
        let desired = self.shadow_model.desired_open();

        let dur = self.shadow_model.observations.drift_duration(now)?;
        // last_intent は UserImeSetIntent / UserImeToggleIntent のみが設定する。
        // PanicReset / HwndCacheRestored は設定しないため、is_some() で十分。
        // SyncKey / PhysicalImeKey / Command は全て閾値 0 (即時補正) の対象。
        let is_strong_intent = self.shadow_model.last_intent.is_some();
        let threshold = if explicit_intent == Some(desired) && is_strong_intent {
            0
        } else {
            u128::from(crate::tuning::DRIFT_CORRECTION_THRESHOLD_MS)
        };
        if dur.as_millis() < threshold {
            return None;
        }

        let max_age =
            std::time::Duration::from_millis(crate::tuning::DRIFT_CORRECTION_OBS_MAX_AGE_MS);
        let trusted = self.shadow_model.observations.most_recent_trusted(now)?;
        if trusted.age(now) > max_age {
            return None;
        }
        // ConvOpenInference（conv ビットからの間接推測、KatakanaShadowOff/
        // NativeToggleShadowOff 由来）は、明示的なユーザー意図が一度も無い間は単独で
        // drift correction を発火させない。desired_open のデフォルト値（起動直後等、
        // last_intent が一度も設定されていない状態）を conv 由来の推論だけで
        // actuate すると、ユーザーが望んでもいない ON/OFF の押し付けになりかねない。
        // 明示意図がある場合（BUG-19 再発の本来のシナリオ: ユーザーが OFF にした
        // 直後に conv がまだ native/katakana を示す）はこの gate を素通りし、
        // 既存の `desired`（ユーザーの意図した値）が正しく再適用される。
        if trusted.source == ObservationSource::ConvOpenInference && explicit_intent.is_none() {
            return None;
        }
        if trusted.open == desired {
            return None;
        }

        Some((desired, trusted.open, dur.as_millis() as u64))
    }

    /// IME apply 完了を記録する（C: mirror + D: generation 照合 dispatch）。
    ///
    /// `generation` がある場合は pending transition と一致する完了だけを受理する。
    /// 古い async 完了をここで弾くことで、GJI/Composition 側にも stale な
    /// `SetOpen(false)` 完了を伝播させない。
    ///
    /// 戻り値は、この完了を現在の IME apply として受理したかどうか。
    pub(crate) fn record_ime_apply_result(
        &mut self,
        open: bool,
        outcome: awase::platform::ImeOpenOutcome,
        generation: Option<u64>,
        ts: u64,
    ) -> bool {
        use awase::platform::ImeOpenOutcome;
        if let Some(generation) = generation {
            let pending = self.shadow_model.pending_generation();
            if pending != Some(generation) {
                log::debug!(
                    "[ime-apply] stale completion ignored: target={open} outcome={outcome:?} \
                     generation={generation} pending={pending:?}"
                );
                return false;
            }
        }

        let effective = match outcome {
            ImeOpenOutcome::Applied
            | ImeOpenOutcome::FallbackSent
            | ImeOpenOutcome::AlreadyMatched => open,
            ImeOpenOutcome::Failed => !open,
            ImeOpenOutcome::UnsafeToToggle => unreachable!(),
        };
        self.mirror_applied_open_with_ts(effective, ts);

        if let Some(generation) = generation {
            let event = ImeEvent::from_apply_outcome(open, outcome, generation);
            self.dispatch_event(event, TickMs(ts));
        }
        true
    }
}

// ── IME 操作ロジック ─────────────────────────────────────────────────────────
//
// PlatformState から委譲されるメソッド群。shadow_model / belief / event_log への
// 書き込みはすべてここに集約し、PlatformState からは直接 shadow_model を触らない。

impl ImeStateHub {
    /// `BrokenAppBootstrap` force-on ガードを追加する。
    pub(crate) fn set_force_on_broken_app_bootstrap(&mut self) {
        self.shadow_model.force_guards.add(ForceGuard {
            reason: ForceOnReason::BrokenAppBootstrap,
            expires_at: None,
            generation: self.event_log.next_seq(),
        });
    }

    /// observe_miss_monitor をリセットし、すべての force-on ガードを解除する。
    ///
    /// ユーザー操作（IME トグル・SetOpen 等）で「意図した状態」が確定したときに呼ぶ。
    pub(crate) fn reset_detect_state(&mut self) {
        self.shadow_model.observe_miss_monitor.record_success();
        self.shadow_model.force_guards.clear();
    }

    /// IME トグルが実際に適用されたことを記録する。
    pub(crate) fn on_ime_toggled(&mut self) {
        self.reset_detect_state();
    }

    /// Engine の SetOpen リクエスト直後に呼ぶ。
    pub(crate) fn on_set_open_requested(&mut self) {
        self.reset_detect_state();
    }

    /// panic_reset 向け全面リセット。
    ///
    /// belief・shadow_model を初期化し `PanicReset` force guard を立てる。
    ///
    /// `tick_ms`: 呼び出し元が取得した現在時刻（`GetTickCount64` 由来）。
    pub(crate) fn apply_panic_reset(&mut self, tick_ms: TickMs) {
        self.dispatch_event(
            ImeEvent::InputModeApplied {
                mode: InputModeState::ObservedRomaji,
                strategy: InputModeApplyStrategy::PanicReset,
                result: InputModeApplyResult::Applied,
                at: tick_ms,
            },
            tick_ms,
        );
        self.belief.is_japanese_ime = true;
        self.belief.prev_conversion_mode = None;
        self.shadow_model.observe_miss_monitor.record_success();
        self.shadow_model.force_guards.clear();
        self.shadow_model.force_guards.add(ForceGuard {
            reason: ForceOnReason::PanicReset,
            expires_at: None,
            generation: self.event_log.next_seq(),
        });
        // PanicReset は desired_open=true に戻すが last_intent を設定しない。
        // ForceGuard::PanicReset が IME ON を保証する。
        self.dispatch_event(ImeEvent::PanicReset { target: true }, tick_ms);
        // IntentStore（BUG-51 追補配線）: この対象に古い明示意図（例: 直前の明示
        // IME OFF）が残っていると、`effective_open()` の IntentStore 優先ロジックが
        // 全面リセットより古い意図を優先してしまう。「同一対象では最新の決定が
        // 古い意図を置換する」という IntentStore 自身の設計を守るため、無効化する。
        if let Some(hwnd) = self.shadow_model.current_focus() {
            self.intent_store.remove(hwnd);
        }
        // panic reset はフォーカスエポックを変えない（同じフォーカスコンテキスト内のリセット）。
        let cur_epoch = self.shadow_model.observations.current_focus_epoch;
        self.shadow_model
            .observations
            .clear_on_focus_change(cur_epoch);
    }

    /// `ImeUpdate` を belief / shadow_model に反映する。
    ///
    /// `observer::ime_observer::poll_and_classify_ime()` の結果を受け取り、
    /// 状態への書き込みをここに集約する。判断ロジックを持たない純粋適用関数。
    ///
    /// `tick_ms`: 呼び出し元が取得した現在時刻（`GetTickCount64` 由来）。
    pub(crate) fn apply_ime_update(
        &mut self,
        update: &crate::observer::ime_observer::ImeUpdate,
        tick_ms: TickMs,
        accepted: crate::state::probe_admission::AcceptedObservation,
    ) {
        if let Some(is_jp) = update.is_japanese_ime {
            self.belief.is_japanese_ime = is_jp;
        }
        if let Some(obs) = update.observer_poll {
            self.dispatch_event(
                ImeEvent::ObserverReported(
                    Observed::<evidence::ObserverPoll>::from_poll(
                        &accepted,
                        obs.value,
                        HwndId::NULL,
                    )
                    .into(),
                ),
                tick_ms,
            );
        }
        if update.increment_miss_count {
            self.shadow_model
                .observe_miss_monitor
                .record_miss(std::time::Instant::now());
            let miss = self
                .shadow_model
                .observe_miss_monitor
                .consecutive_miss_count;
            if miss == crate::IME_DETECT_MISS_THRESHOLD {
                log::warn!("IME detection failed {miss} consecutive times, will force IME ON");
            }
        }
        if update.clear_force_on_broken_app_bootstrap {
            self.shadow_model
                .force_guards
                .remove(ForceOnReason::BrokenAppBootstrap);
        }
        if update.clear_force_on_panic_reset {
            self.shadow_model
                .force_guards
                .remove(ForceOnReason::PanicReset);
            self.shadow_model.observe_miss_monitor.record_success();
        }
        if let Some(mode) = update.new_input_mode {
            self.dispatch_event(
                ImeEvent::InputModeObserved {
                    mode,
                    source: ObservationSource::ObserverPoll,
                    confidence: ObservationConfidence::Medium,
                    at: tick_ms,
                },
                tick_ms,
            );
        }
        if let Some(conv) = update.new_prev_conversion_mode {
            self.belief.prev_conversion_mode = Some(conv);
        }
    }

    /// `hwnd_cache` の復元結果を belief / shadow_model に反映する。
    ///
    /// `tick_ms`: 呼び出し元が取得した現在時刻（`GetTickCount64` 由来）。
    pub(crate) fn apply_hwnd_cache_restore(
        &mut self,
        snapshot: Option<crate::focus::hwnd_cache::HwndImeSnapshot>,
        tick_ms: TickMs,
    ) {
        if let Some(snap) = snapshot {
            // HwndCacheRestored は desired_open を回復するが last_intent を設定しない。
            // キャッシュ復元はユーザーの能動的操作ではなく、後続の実観測で上書き可能。
            self.dispatch_event(
                ImeEvent::HwndCacheRestored {
                    target: snap.ime_on,
                },
                tick_ms,
            );
            // IntentStore（BUG-51 追補 v3）: 無条件 remove() は「フォーカス滞在
            // 100ms 未満（MIN_FOCUS_DURATION_MS）だと退場時の cache 保存自体が
            // スキップされる」ケース（BUG-57 型のフォーカス奪取）で、たった今の
            // 新しい明示意図より古いキャッシュを勝たせてしまう（pre-mortem #2）。
            // 記録時刻を比較し、キャッシュのほうが新しい（意図と同時刻を含む）場合
            // のみ無効化する。意図の方が新しい場合はエントリを残し、
            // `effective_open()` が IntentStore を優先することで新しい意図を守る。
            //
            // 判定本体は `IntentStore::invalidate_for_cache_restore()`（ungated、
            // Linux CI で走る）にある。ここはログだけ（追補4）。
            if let Some(hwnd) = self.shadow_model.current_focus() {
                if let crate::state::intent_store::CacheRestoreVerdict::Kept {
                    intent_recorded_at_ms,
                } =
                    self.intent_store
                        .invalidate_for_cache_restore(hwnd, snap.recorded_ms, tick_ms)
                {
                    log::info!(
                        "[intent-store] cache restore より新しい明示意図を保持 \
                         (cache recorded_ms={} < intent recorded_at_ms={intent_recorded_at_ms})",
                        snap.recorded_ms,
                    );
                }
            }
            // キャッシュされた input_mode が ObservedEisu の場合、生の観測と同じ強さで
            // engine activation を塞がせない（cache_restore_eisu_guard 参照）。
            // 2026-07-09 MS Edge で実発生: Uwp⇔TsfNative フォーカス往復のたびに
            // 131 秒前の ObservedEisu キャッシュが復元され、eisu guard に阻まれて
            // engine が inactive のまま固着し続けた。
            let mode = crate::state::eisu_recovery::cache_restore_eisu_guard(snap.input_mode);
            self.dispatch_event(
                ImeEvent::InputModeApplied {
                    mode,
                    strategy: InputModeApplyStrategy::CacheRestore,
                    result: InputModeApplyResult::Applied,
                    at: tick_ms,
                },
                tick_ms,
            );
        }
    }

    /// Imm32Unavailable (Chrome/Teams 等) 入場時に stale な `desired_open=false` を IME ON へ寄せ直す。
    ///
    /// TsfNative と同様だが、Imm32Unavailable では awase が IME 状態を制御できないため
    /// キャッシュが carry-over で汚染されやすい。キャッシュ値が「ユーザー明示の OFF」に
    /// 由来しない場合にのみ呼ぶこと（呼び出し側が stale 判定を行う）。
    ///
    /// `reset_to_off_for_tsf_native_cache_miss` と同様、これも「観測が何もない」ことを
    /// 根拠にした安全デフォルトの推測にすぎないため `UserImeSetIntent` は使わず
    /// `ObserverReported`（`HeuristicDefault`, Low confidence）として記録する。
    /// `desired_open` は書き換えない。
    ///
    /// `tick_ms`: 呼び出し元が取得した現在時刻（`GetTickCount64` 由来）。
    pub(crate) fn reset_stale_ime_on_for_imm_broken(
        &mut self,
        profile: ImePolicyProfile,
        tick_ms: TickMs,
    ) {
        if !self.belief.is_japanese_ime() || self.shadow_model.effective_open() {
            return;
        }
        if let Some(intent) = self.shadow_model.last_intent.as_ref() {
            log::debug!(
                "Imm32Unavailable entry: preserving ime_on=false (intent source={:?})",
                intent.source
            );
            return;
        }
        // IntentStore（BUG-51 追補 v3）: last_intent は FocusChanged でクリアされるが、
        // IntentStore の有効エントリは同一対象への明示意図がまだ生きていることを
        // 意味する。last_intent と同じ扱いで safety-net（HeuristicDefault ON）を
        // 書かずに温存する。エントリを消して heuristic を通すのは「観測ゼロの推測が
        // 明示意図に勝つ」逆転になるため行わない（pre-mortem #2）。
        if let Some(hwnd) = self.shadow_model.current_focus() {
            if let Some(intent) = self.intent_store.lookup(hwnd, tick_ms) {
                log::debug!(
                    "Imm32Unavailable entry: preserving stored intent open={} (source={:?})",
                    intent.open,
                    intent.source
                );
                return;
            }
        }
        log::info!(
            "Imm32Unavailable entry without trusted cache: 安全デフォルト ON を Low confidence \
             observation として記録 (no explicit intent, Japanese layout, IME state \
             uncontrollable in Imm32Unavailable)"
        );
        let focus_epoch = self.shadow_model.observations.current_focus_epoch;
        self.dispatch_event(
            ImeEvent::ObserverReported(
                Observed::<evidence::HeuristicDefault>::at_startup(
                    profile,
                    true,
                    HwndId::NULL,
                    focus_epoch,
                )
                .into(),
            ),
            tick_ms,
        );
    }

    pub(crate) fn set_is_japanese_ime(&mut self, value: bool) {
        self.belief.is_japanese_ime = value;
    }

    pub(crate) fn set_prev_conversion_mode(&mut self, value: Option<u32>) {
        self.belief.prev_conversion_mode = value;
    }

    // ── イベント dispatch ヘルパ ──

    pub(crate) fn write_observer_poll(
        &mut self,
        value: bool,
        tick_ms: TickMs,
        accepted: crate::state::probe_admission::AcceptedObservation,
    ) {
        self.dispatch_event(
            ImeEvent::ObserverReported(
                Observed::<evidence::ObserverPoll>::from_poll(&accepted, value, HwndId::NULL)
                    .into(),
            ),
            tick_ms,
        );
    }

    /// 設定された同期キー由来の意図。`IntentWitness::from_sync_key` を通った
    /// 「注入されていない実キーイベント」がないと呼べない（ADR-089 §2.2、
    /// BUG-14 の型化）。source は witness が運ぶ。
    ///
    /// witness があるということは「注入されていない実キーイベントが存在した」
    /// ということなので、そのまま `record_explicit_intent` の前提
    /// （本物のユーザー操作）も満たす（BUG-51 追補 v3、ADR-089 §2.2 と同型）。
    pub(crate) fn write_sync_key(&mut self, witness: IntentWitness, value: bool, tick_ms: TickMs) {
        let source = witness.source();
        self.dispatch_event(
            ImeEvent::UserImeSetIntent {
                target: value,
                source,
            },
            tick_ms,
        );
        self.record_explicit_intent(value, source, tick_ms);
    }

    /// 実ユーザー操作と確定した明示 IME 意図を IntentStore に記録する
    /// (ADR-087 §5 Phase 1' 配線、BUG-51 追補 v3)。
    ///
    /// `dispatch_event` の `UserImeSetIntent` 分岐で record しないのは、
    /// `Command` ソースが conv 由来の内部同期（`EngineSync::DirectInput`）でも
    /// dispatch されるため。呼び出してよいのは以下の3箇所のみ:
    /// - `write_sync_key` / `write_physical_key`（物理 IME キーの shadow toggle。
    ///   `IntentWitness` が「注入されていない実キーイベント」を型で要求する）
    /// - `kp_stage_post_decision` の `SetOpenOrigin::ExplicitUserAction` 分岐
    ///   （IME ON/OFF コンボ、`applied=true` のときのみ）
    ///
    /// # どのガードが何を固定しているか（2026-08-13 訂正）
    ///
    /// v3 のこの doc は当初「3箇所のみ（`tests/architecture_guard.rs` で出現数を
    /// 固定）」と書いていたが、実際に固定されていたのは
    /// `intent_store_record_call_sites_are_limited_to_explicit_user_actions`
    /// による `self.intent_store.record(`（本ファイル内、1箇所＝本メソッド内）
    /// だけで、**`record_explicit_intent` 自身の呼び出し元の数は固定されて
    /// いなかった**（3箇所目のある `runtime/key_pipeline.rs` はそのガードの
    /// 走査対象ですらなかった）。BUG-51 追補を develop へ統合した際のレビューで
    /// 発覚し、`record_explicit_intent_call_sites_are_limited_to_real_user_actions`
    /// （`src/` 全走査でファイルごとの出現数を固定）を新設して穴を埋めた。
    /// 現在は 2 本のガードが二段で効く:
    /// - 「`IntentStore` へ record できるのは本メソッドだけ」＝ 前者
    /// - 「本メソッドを呼べるのは上記3箇所だけ」＝ 後者
    pub(crate) fn record_explicit_intent(
        &mut self,
        target: bool,
        source: UserIntentSource,
        tick_ms: TickMs,
    ) {
        if let Some(hwnd) = self.shadow_model.current_focus() {
            self.intent_store.record(hwnd, target, source, tick_ms);
        }
    }

    /// 物理 IME キー由来の意図。`IntentWitness::from_physical` を通った
    /// 「注入されていない実キーイベント」がないと呼べない。
    ///
    /// `write_sync_key` と同様、witness の存在がそのまま
    /// `record_explicit_intent` の前提を満たす（BUG-51 追補 v3）。
    pub(crate) fn write_physical_key(
        &mut self,
        witness: IntentWitness,
        value: bool,
        tick_ms: TickMs,
    ) {
        let source = witness.source();
        self.dispatch_event(
            ImeEvent::UserImeSetIntent {
                target: value,
                source,
            },
            tick_ms,
        );
        self.record_explicit_intent(value, source, tick_ms);
    }

    pub(crate) fn write_set_open_request(&mut self, value: bool, tick_ms: TickMs) {
        self.dispatch_event(
            ImeEvent::UserImeSetIntent {
                target: value,
                source: UserIntentSource::Command,
            },
            tick_ms,
        );
    }

    pub(crate) fn write_focus_probe(
        &mut self,
        value: bool,
        tick_ms: TickMs,
        accepted: crate::state::probe_admission::AcceptedObservation,
    ) {
        // confidence は `Observed<FocusProbe>` 側で Low 固定
        // （top-level hwnd の IMC を読むため Qt/GJI 等では child hwnd と異なる
        // 場合がある。High confidence の ImmCrossProbe が後から上書きする）。
        self.dispatch_event(
            ImeEvent::ObserverReported(
                Observed::<evidence::FocusProbe>::from_probe(&accepted, value, HwndId::NULL).into(),
            ),
            tick_ms,
        );
    }

    /// ImmCross 非同期プローブ結果を記録する（High confidence）。
    ///
    /// `read_ime_state_full_async` が child hwnd の IMM32 状態を読んだ後に呼ぶ。
    /// High confidence のため `derive_any()` で即採用される。
    /// `accepted` は `ImmLikeTicket::admit()` が返した `AcceptedObservation`（epoch 照合済み）。
    pub(crate) fn write_imm_cross_probe(
        &mut self,
        value: bool,
        tick_ms: TickMs,
        accepted: crate::state::probe_admission::AcceptedObservation,
    ) {
        self.dispatch_event(
            ImeEvent::ObserverReported(
                Observed::<evidence::ImmCrossProbe>::from_cross_probe(
                    &accepted,
                    value,
                    HwndId::NULL,
                )
                .into(),
            ),
            tick_ms,
        );
    }

    /// idle-conv-check の conv ビット推論から得た IME open 状態を観測として記録する
    /// (`NativeToggleShadowOff`（旧 `KatakanaShadowOff` を統合済み）、`conv_classify::EngineSync::
    /// ReportOpenInference` 経由)。
    ///
    /// `desired_open` を直接書き換えない — `ObserverReported` として `observations`
    /// に記録するだけにとどめ、実際に補正が必要かどうかの判断は既存の drift
    /// correction 経路 (`check_drift_correction`) に委ねる。かつては
    /// `handle_engine_set_open(true)` を直接呼び `UserImeSetIntent{Command}` を偽装して
    /// `desired_open` を上書きしていたため、ユーザーの明示 OFF 直後でも engine が
    /// 勝手に ON へ戻る再発バグを起こした（2026-07-08, BUG-19 再発）。
    ///
    /// conv 由来の open 推論は間接観測（`ImmGetConversionStatus` の conv ビットから
    /// 「native/katakana ならおそらく open」と推測しているだけで、`ImmGetOpenStatus`
    /// を直接呼んでいるわけではない）のため confidence は `Medium` を上限とする
    /// (`GjiIoInference` と同じ「間接観測」区分)。
    ///
    /// `tick_ms`: 呼び出し元が取得した現在時刻。
    pub(crate) fn report_conv_open_inference(
        &mut self,
        open: bool,
        reason: crate::state::conv_classify::ConvSyncReason,
        tick_ms: TickMs,
    ) {
        log::debug!("[conv-open-inference] reason={reason:?} open={open}");
        let focus_epoch = self.shadow_model.observations.current_focus_epoch;
        self.dispatch_event(
            ImeEvent::ObserverReported(
                Observed::<evidence::ConvOpenInference>::from_conv(
                    reason,
                    open,
                    HwndId::NULL,
                    focus_epoch,
                )
                .into(),
            ),
            tick_ms,
        );
    }
}

#[cfg(test)]
impl ImeStateHub {
    pub(crate) fn set_desired_open_for_test(&mut self, value: bool) {
        self.shadow_model.set_desired_open_for_test(value);
    }

    pub(crate) fn clear_last_intent_for_test(&mut self) {
        self.shadow_model.last_intent = None;
    }

    /// 現在呼び出し元がないが診断用アクセサとして残す。
    #[allow(dead_code)]
    pub(crate) fn last_intent_source(&self) -> Option<UserIntentSource> {
        self.shadow_model.last_intent.as_ref().map(|i| i.source)
    }
}

// ────────────────────────────────────────────────────────────────────────────
// FocusStore
// ────────────────────────────────────────────────────────────────────────────

/// フォーカスメタデータを集約する sub-struct。
///
/// `PlatformState` の Facade から内部委譲される。親を参照しない。
#[derive(Debug)]
pub(crate) struct FocusStore {
    pub app_kind: AppKind,
    pub focus_kind: FocusKind,
    /// 最後にフォアグラウンドプロセスが変わった時刻（ms, GetTickCount 系）。
    /// IME 診断ログで「フォーカス変更からの経過時間」を表示するために使う。
    pub last_focus_change_ms: u64,
    /// journal 専用: 最後に FocusTransition を記録した時刻（ms, GetTickCount 系）。
    ///
    /// プロセス変更以外の window / app_kind / focus_kind 変化も含む。既存の
    /// `last_focus_change_ms` はキャッシュ保存判定の意味を持つため流用しない。
    pub last_focus_transition_ms: u64,
    pub focus_debounce_ms: u32,
    pub ime_poll_interval_ms: u32,
    /// フォーカスプロセス変更のエポック番号。
    ///
    /// `on_focus_process_changed` のたびに `wrapping_add(1)` でインクリメントされる。
    /// probe の spawn 時にキャプチャし、完了時に照合することで「spawn 後にフォーカスが
    /// 変わったか」を時間ベースの競合なしに正確に判定できる（→ probe_admission モジュール）。
    pub focus_epoch: u64,
}

impl FocusStore {
    pub(crate) fn new() -> Self {
        Self {
            app_kind: AppKind::Win32,
            focus_kind: FocusKind::Undetermined,
            last_focus_change_ms: 0,
            last_focus_transition_ms: 0,
            focus_debounce_ms: 50,
            ime_poll_interval_ms: 500,
            focus_epoch: 0,
        }
    }
}

impl Default for FocusStore {
    fn default() -> Self {
        Self::new()
    }
}

// ────────────────────────────────────────────────────────────────────────────
// GateStore
// ────────────────────────────────────────────────────────────────────────────

/// フックゲート・バイパス関連状態を集約する sub-struct。
///
/// `PlatformState` の Facade から内部委譲される。親を参照しない。
#[derive(Debug)]
pub(crate) struct GateStore {
    pub last_hook_activity_ms: u64,
    /// Ctrl+key bypass 直後フラグ。
    ///
    /// Ctrl+非修飾キーが PassThrough として素通りした後、次の非修飾 non-Ctrl キー 1 つを
    /// NICOLA エンジンをスキップして直接 passthrough させる。
    /// tmux prefix (Ctrl+J) → コマンドキー (n/p) のように、
    /// prefix 直後のコマンドキーが NICOLA に横取りされる問題を防ぐ。
    pub post_bypass_passthrough: bool,
    /// IME 同期キー直後のキー保留バッファ（旧 `ime_gate`）。
    pub sync_key_gate: SyncKeyGate,
    /// 今回の左Shift downが単独タップ候補か（`kp_stage_shift_conv_guard`）。
    ///
    /// 左Shift KeyDownでtrueにセットし、Shift保持中に`VK_LSHIFT`/`VK_RSHIFT`以外の
    /// 非注入物理KeyDownが来たらfalseに倒す（チョード判定）。左Shift KeyUp時に
    /// これがtrueのままなら「本物の単独タップ」として半角英数トグルの対象にする。
    pub left_shift_tap_candidate: bool,
    /// 今回のShift downに対応する復元処理が必要か（`kp_stage_shift_conv_guard`）。
    ///
    /// Shift KeyDownで awase が conv=0x00000000（IME-ON 半角英数）へ切り替えたとき
    /// true。Shift KeyUpで`std::mem::take`し、trueならKeyUp側の復元/トグル判定を
    /// 走らせる。**`half_width_alnum_toggle_active`とは独立**（トグルON中の
    /// Shift downでも必ずtrueにする——立てないとKeyUp側でトグルOFF/右Shift緊急解除が
    /// 発火しなくなる、2026-07-11 codexレビューで発覚）。
    pub shift_conv_guard_pending: bool,
    /// 左Shift単独タップによる「IME-ON半角英数」持続トグルが有効か。
    ///
    /// `shift_conv_guard_pending`と違い、Shift keyup後も左Shiftの次の単独タップ
    /// （または右Shiftタップ/フォーカス変更による緊急解除）まで true であり続ける。
    /// true の間、`platform_state.ime.input_mode()`はObservedEisuへ誘導され
    /// Engineが`Inactive(NotRomajiInput)`で素通りになる（IMEはbelief上ONのまま）。
    /// idle-conv-check / ime_refresh の OS poll を凍結する（`shift_conv_guard_pending`
    /// と同じ理由: conv=0x0000は awase自身の意図的な状態のため）。
    pub half_width_alnum_toggle_active: bool,
    /// `kp_stage_idle_conv_check` の conv 読み取り（offload 済み、`SendMessageTimeoutW`
    /// ベース）が in-flight かどうか。
    ///
    /// GJI が本当にハングしている間に断続的なタイピングが続くと、idle ゲートを
    /// 通過するたびに新しい offload 呼び出しが積み上がりワーカースレッドが増え続ける。
    /// 1 件 in-flight の間は新規 spawn をスキップし、完了時（epoch 棄却時も含む）に
    /// `with_app` 内で必ず false へ戻す。
    pub idle_conv_check_in_flight: bool,
}

impl GateStore {
    pub(crate) fn new() -> Self {
        Self {
            last_hook_activity_ms: 0,
            post_bypass_passthrough: false,
            sync_key_gate: SyncKeyGate::new(),
            left_shift_tap_candidate: false,
            shift_conv_guard_pending: false,
            half_width_alnum_toggle_active: false,
            idle_conv_check_in_flight: false,
        }
    }
}

impl Default for GateStore {
    fn default() -> Self {
        Self::new()
    }
}

// ────────────────────────────────────────────────────────────────────────────
// KeymapStore
// ────────────────────────────────────────────────────────────────────────────

/// アクティブなキーマップルールを保持する sub-struct。
///
/// `PlatformState` の Facade から内部委譲される。親を参照しない。
#[derive(Debug, Default)]
pub(crate) struct KeymapStore {
    /// 現在のフォーカスアプリに適用されるキーマップルール
    pub active_keymaps: crate::keymap::KeymapTable,
}

// ────────────────────────────────────────────────────────────────────────────
// PlatformState
// ────────────────────────────────────────────────────────────────────────────

/// Platform 層の全状態を集約する Facade 構造体。
///
/// 各ドメインの状態は sub-struct（`FocusStore` / `GateStore` / `KeymapStore`）に委譲する。
/// `ImeStateHub` は IME 観測・判断を担う凝集ユニットとして引き続き `ime` フィールドで保持する。
///
/// シングルスレッド（メインスレッド＋フックコールバック）からのみアクセスされる。
/// `APP: SingleThreadCell<Runtime>` 経由で保持される。
#[derive(Debug)]
pub struct PlatformState {
    /// IME 観測・判断・belief 書き戻しを担う凝集ユニット（ImeStore 相当）。
    pub(crate) ime: ImeStateHub,
    /// フォーカスメタデータ（AppKind / FocusKind / タイムスタンプ / デバウンス設定）。
    pub(crate) focus: FocusStore,
    /// フックゲート・バイパス関連状態（アクティビティタイムスタンプ / post-bypass / sync_key_gate）。
    pub(crate) gate: GateStore,
    /// キーマップルール（フォーカスアプリ別アクティブルール）。
    pub(crate) keymap: KeymapStore,
}

impl PlatformState {
    /// デフォルト値で初期化する
    #[must_use]
    pub fn new() -> Self {
        Self {
            ime: ImeStateHub::new(),
            focus: FocusStore::new(),
            gate: GateStore::new(),
            keymap: KeymapStore::default(),
        }
    }
}

impl Default for PlatformState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// shadow_model を直接設定するヘルパ:
    /// `set_intent=Some(source)` なら UserImeSetIntent を dispatch し last_intent を設定する。
    /// `set_intent=None` なら desired_open のみ直接書き換え、last_intent は空のままにする
    /// (focus 変更後の carry-over シナリオを模擬)。
    fn ps_with_shadow(
        desired_open: bool,
        set_intent: Option<UserIntentSource>,
        is_japanese: bool,
    ) -> PlatformState {
        let mut ps = PlatformState::new();
        ps.ime.belief.is_japanese_ime = is_japanese;
        if let Some(source) = set_intent {
            ps.ime.dispatch_event(
                ImeEvent::UserImeSetIntent {
                    target: desired_open,
                    source,
                },
                TickMs(0),
            );
        } else {
            ps.ime.set_desired_open_for_test(desired_open);
            ps.ime.clear_last_intent_for_test();
        }
        ps
    }

    // reset_stale_ime_on_for_imm_broken も同様に desired_open を書き換えない。
    #[test]
    fn imm_broken_reset_does_not_touch_desired_open() {
        let mut ps = ps_with_shadow(false, None, true);
        ps.ime
            .reset_stale_ime_on_for_imm_broken(ImePolicyProfile::Imm32Unavailable, TickMs(0));
        assert!(
            !ps.ime.model().desired_open(),
            "desired_open はユーザーの真の意図のまま変更されない"
        );
        assert!(
            ps.ime.effective_open(),
            "実効値は Low confidence observation 経由で true になる"
        );
    }

    // ── handle_engine_set_open: focus_transition_was_pending フィルタ ──
    //
    // 2026-07-05: Alt+Tab 中の中間ウィンドウ（Alt+Tab スイッチャー等）への一瞬の
    // フォーカスで Engine が SetOpen を発行し、それが最終的な着地先ウィンドウとは
    // 無関係な SendInput として実行され、belief と実IME状態が乖離するバグの修正。

    // focus_transition_was_pending=true の場合、SetOpen 要求はフィルタされ
    // desired_open/last_explicit_ime_action_ms は変化しない。
    #[test]
    fn handle_engine_set_open_filters_when_focus_transition_was_pending() {
        let mut ps = ps_with_shadow(false, Some(UserIntentSource::SyncKey), true);
        let applied = ps
            .ime
            .handle_engine_set_open(true, false, true, 1, TickMs(0));
        assert!(!applied, "focus transition pending 中は適用されない");
        assert!(
            !ps.ime.model().desired_open(),
            "フィルタされた SetOpen は desired_open を書き換えない"
        );
    }

    // focus_transition_was_pending=false なら通常通り適用される（回帰防止）。
    #[test]
    fn handle_engine_set_open_applies_when_focus_transition_not_pending() {
        let mut ps = ps_with_shadow(false, Some(UserIntentSource::SyncKey), true);
        let applied = ps
            .ime
            .handle_engine_set_open(true, false, false, 1, TickMs(0));
        assert!(
            applied,
            "focus transition が pending でなければ通常通り適用される"
        );
        assert!(ps.ime.model().desired_open());
    }

    // 既存の CtrlImeChord フィルタが、focus_transition フィルタ追加後も
    // 引き続き機能することを確認する回帰テスト。
    #[test]
    fn handle_engine_set_open_ctrl_chord_filter_still_works() {
        let mut ps = ps_with_shadow(true, Some(UserIntentSource::SyncKey), true);
        // 1 回目: IME OFF 要求 + Ctrl 押下中 → chord transaction 開始。
        let first = ps
            .ime
            .handle_engine_set_open(false, true, false, 1, TickMs(0));
        assert!(first, "chord を開始する最初の要求は適用される");
        assert!(ps.ime.is_ctrl_ime_chord_active());
        // 2 回目: chord transaction 中の二次 IME OFF 要求 → フィルタされる。
        let second = ps
            .ime
            .handle_engine_set_open(false, true, false, 2, TickMs(0));
        assert!(
            !second,
            "chord transaction 中の二次 IME OFF 要求はフィルタされる"
        );
    }

    // ── persistent_explicit_off_ms: Command ソースも SyncKey/PhysicalImeKey と
    //    同じく永続タイムスタンプを更新すること（2026-08-04 実機ログ調査）。
    //
    // Ctrl+無変換（デフォルトキーバインド）による明示 IME OFF は
    // `SpecialKeyMatch::ImeOff` → `handle_engine_set_open` → `write_set_open_request`
    // → `UserIntentSource::Command` を経由するが、`dispatch_event` の永続タイムスタンプ
    // 更新が SyncKey/PhysicalImeKey のみを対象にしていたため Command が漏れていた。
    // その結果、明示 OFF の数秒後に UWP 系中間ウィンドウ（Imm32Unavailable、
    // 例: ForegroundStaging）へフォーカスが渡ると `focus_tracking.rs` の
    // `EXPLICIT_OFF_CACHE_SUPPRESS_MS`（10秒）抑制ガードが効かず（`persistent_explicit_off_ms()`
    // が常に 0 のため `last_off_ms > 0` が false）、`reset_stale_ime_on_for_imm_broken`
    // が「明示的意図なし」と誤判定して belief を Low confidence で ON に戻し、
    // Engine が「IME OFF のはずなのに勝手に ON へ戻る」症状を起こしていた
    // （BUG-48 の「未解明: 最初に ctx.ime_on が観測駆動で true に振れる具体的トリガー」
    // に対応する原因の一つ）。BUG-48 修正（PR #44）により Command ソースは
    // `handle_engine_set_open`（`SetOpenOrigin::ExplicitUserAction`）経由でのみ
    // 発行されるようになり、エンジン内部の対称 echo と分離済みなので、
    // SyncKey/PhysicalImeKey と同列に永続タイムスタンプへ含めてよい。
    #[test]
    fn command_source_updates_persistent_explicit_off_ms() {
        let mut ps = PlatformState::new();
        ps.ime.dispatch_event(
            ImeEvent::UserImeSetIntent {
                target: false,
                source: UserIntentSource::Command,
            },
            TickMs(12_345),
        );
        assert_eq!(
            ps.ime.persistent_explicit_off_ms(),
            12_345,
            "Command ソースの明示 OFF も永続タイムスタンプを更新すること"
        );

        ps.ime.dispatch_event(
            ImeEvent::UserImeSetIntent {
                target: true,
                source: UserIntentSource::Command,
            },
            TickMs(20_000),
        );
        assert_eq!(
            ps.ime.persistent_explicit_off_ms(),
            0,
            "Command ソースの明示 ON はタイムスタンプをリセットすること"
        );
    }

    // Ctrl+無変換 のデフォルトキーバインドが実際にたどる呼び出し経路
    // （`handle_engine_set_open` → `write_set_open_request` → `Command`）を
    // 直接エンドツーエンドで確認する回帰テスト。
    #[test]
    fn handle_engine_set_open_updates_persistent_explicit_off_ms() {
        let mut ps = ps_with_shadow(true, Some(UserIntentSource::SyncKey), true);
        let applied = ps
            .ime
            .handle_engine_set_open(false, false, false, 1, TickMs(9_999));
        assert!(applied);
        assert_eq!(
            ps.ime.persistent_explicit_off_ms(),
            9_999,
            "デフォルトキーバインド経由の明示 IME OFF が \
             Imm32Unavailable cache-miss ガードから漏れないこと"
        );
    }

    // ── handle_engine_activation_sync（BUG-48）: handle_engine_set_open と同じ
    //    filter を独立に実装しているため、乖離を検知できるよう同型のテストを鏡写しで
    //    用意する（Opus レビュー 2026-08-04 で「コピペされた filter に対応テストが
    //    無く、2つの実装が乖離しても気づけない」と指摘された）。

    #[test]
    fn handle_engine_activation_sync_filters_when_focus_transition_was_pending() {
        let mut ps = ps_with_shadow(false, Some(UserIntentSource::SyncKey), true);
        let applied = ps
            .ime
            .handle_engine_activation_sync(true, false, true, 1, TickMs(0));
        assert!(!applied, "focus transition pending 中は適用されない");
        assert!(
            !ps.ime.model().desired_open(),
            "フィルタされた ActivationSync は desired_open を書き換えない \
             (そもそも desired_open は書き換えない設計だが、フィルタされた場合も \
             念のため確認する)"
        );
    }

    #[test]
    fn handle_engine_activation_sync_applies_when_focus_transition_not_pending() {
        let mut ps = ps_with_shadow(false, None, true);
        let applied = ps
            .ime
            .handle_engine_activation_sync(true, false, false, 1, TickMs(0));
        assert!(
            applied,
            "focus transition が pending でなければ通常通り適用される"
        );
    }

    #[test]
    fn handle_engine_activation_sync_ctrl_chord_filter_still_works() {
        let mut ps = ps_with_shadow(false, None, true);
        // 1 回目: ActivationSync による IME OFF 要求 + Ctrl 押下中 → chord transaction 開始。
        let first = ps
            .ime
            .handle_engine_activation_sync(false, true, false, 1, TickMs(0));
        assert!(first, "chord を開始する最初の要求は適用される");
        assert!(ps.ime.is_ctrl_ime_chord_active());
        // 2 回目: chord transaction 中の二次 IME OFF 要求 → フィルタされる。
        let second = ps
            .ime
            .handle_engine_activation_sync(false, true, false, 2, TickMs(0));
        assert!(
            !second,
            "chord transaction 中の二次 IME OFF 要求はフィルタされる"
        );
    }

    // handle_engine_set_open との核心的な違い: last_intent が既にある間は
    // desired_open を一切書き換えない（BUG-48 修正の中心的な不変条件）。
    #[test]
    fn handle_engine_activation_sync_never_sets_last_intent_or_desired_open() {
        let mut ps = ps_with_shadow(false, Some(UserIntentSource::PhysicalImeKey), true);
        let applied = ps
            .ime
            .handle_engine_activation_sync(true, false, false, 1, TickMs(0));
        assert!(applied);
        assert_eq!(
            ps.ime.model().last_intent.as_ref().map(|i| i.target),
            Some(false),
            "ActivationSync はユーザーの明示的な OFF 意図 (last_intent) を上書きしない"
        );
        assert!(
            !ps.ime.model().desired_open(),
            "ActivationSync は desired_open も一切書き換えない"
        );
        assert!(
            !ps.ime.effective_open(),
            "explicit intent が残っているため effective_open() は false のまま"
        );
    }

    // 修正1a 回帰（BUG-51 追補 v3）: ActivationSync 経由（conv 由来の RomajiRecovered
    // 相当）は last_intent/desired_open だけでなく IntentStore にも記録されない
    // こと。v1 のままだと DirectInput/RomajiRecovered が UserImeSetIntent{Command}
    // を dispatch し IntentStore に「壊れた conv 読み由来の偽の明示意図」が
    // FocusChanged を生き延びて残ってしまっていた（pre-mortem #1 角度2）。
    #[test]
    fn handle_engine_activation_sync_does_not_record_intent_store_entry() {
        let mut ps = PlatformState::new();
        dispatch_focus_changed(&mut ps, TARGET_HWND, 1, 0);
        let applied = ps
            .ime
            .handle_engine_activation_sync(true, false, false, 1, TickMs(0));
        assert!(applied);
        // IntentStore にエントリが無いことを直接確認する:
        // conv 観測が effective_open() を反転させても、IntentStore 側からの
        // 上書きは発生しない（= 生の shadow_model の値がそのまま反映される）。
        dispatch_conv_open_inference(&mut ps, true, 100);
        assert_eq!(
            ps.ime.effective_open_at(TickMs(100)),
            ps.ime.model().effective_open(),
            "ActivationSync は IntentStore に記録しないため、hub 版と生の \
             ImeModel 版の effective_open() は一致し続ける（IntentStore 由来の \
             上書きが存在しないことの証拠）"
        );
    }

    // ── report_conv_open_inference / check_drift_correction (BUG-19 再発対策) ──
    //
    // 2026-07-08 実機再発: ユーザーが IME OFF (last_intent=Some(false)) にした
    // 約1.6秒後、conv ビットが native/katakana を示したことを理由に
    // KatakanaShadowOff が UserImeSetIntent{Command} を偽装して desired_open を
    // true に書き換え、engine が勝手に ON へ戻った。修正後は ObserverReported
    // (ConvOpenInference) として記録するだけにとどめ、既存の drift correction が
    // 正しい方向（desired=false の再送）で解決することを、実時間 sleep を使わず
    // （drift.started_at / 観測の at を直接バックデートして）確認する。

    use super::super::observation_store::ImeDrift;
    use crate::state::conv_classify::ConvSyncReason;

    #[test]
    fn report_conv_open_inference_does_not_touch_desired_open_or_last_intent() {
        let mut ps = ps_with_shadow(false, Some(UserIntentSource::PhysicalImeKey), true);
        ps.ime
            .report_conv_open_inference(true, ConvSyncReason::NativeToggleShadowOff, TickMs(0));
        assert!(
            !ps.ime.model().desired_open(),
            "conv 由来の open 推論は desired_open を書き換えない"
        );
        assert_eq!(
            ps.ime.explicit_intent(),
            Some(false),
            "last_intent (explicit_intent) も変更されない — ObserverReported は意図を偽装しない"
        );
    }

    // BUG-19 再発の実ログ相当: last_intent=Some(false) (explicit_intent==desired) なので
    // threshold=0 となり、conv の一発観測直後でも正しい方向 (false の再送) が返る。
    #[test]
    fn check_drift_correction_fires_immediately_when_explicit_off_intent_conflicts_with_conv_inference(
    ) {
        let mut ps = ps_with_shadow(false, Some(UserIntentSource::PhysicalImeKey), true);
        ps.ime
            .report_conv_open_inference(true, ConvSyncReason::NativeToggleShadowOff, TickMs(0));
        let now = std::time::Instant::now();
        let explicit_intent = ps.ime.explicit_intent();
        match ps.ime.check_drift_correction(now, explicit_intent) {
            Some((desired, observed, _dur_ms)) => {
                assert!(!desired, "desired は false のまま保持されている");
                assert!(observed, "conv 推論が observed=true として記録されている");
            }
            None => panic!(
                "explicit intent が desired と一致する場合は即時 (threshold=0) で \
                 補正が返るべき"
            ),
        }
    }

    // 明示意図が一度も無い（起動直後等）状態では、conv 推論単独で drift correction
    // を発火させない — desired_open のデフォルト値をユーザーの意図なしに actuate
    // してしまうのを防ぐ。
    #[test]
    fn check_drift_correction_ignores_conv_inference_alone_without_explicit_intent() {
        let mut ps = ps_with_shadow(false, None, true);
        ps.ime
            .report_conv_open_inference(true, ConvSyncReason::NativeToggleShadowOff, TickMs(0));
        // 明示意図が無いので threshold=DRIFT_CORRECTION_THRESHOLD_MS。実時間 sleep を
        // 避けるため drift.started_at を直接バックデートして閾値超過を模す。
        ps.ime.shadow_model.observations.drift = Some(ImeDrift {
            started_at: std::time::Instant::now()
                - std::time::Duration::from_millis(
                    crate::tuning::DRIFT_CORRECTION_THRESHOLD_MS + 50,
                ),
        });
        let now = std::time::Instant::now();
        let explicit_intent = ps.ime.explicit_intent();
        assert_eq!(explicit_intent, None);
        assert_eq!(
            ps.ime.check_drift_correction(now, explicit_intent),
            None,
            "明示意図なしでは ConvOpenInference 単独で補正を発火させない"
        );
    }

    #[test]
    fn check_drift_correction_none_when_conv_inference_matches_desired() {
        let mut ps = ps_with_shadow(true, Some(UserIntentSource::PhysicalImeKey), true);
        ps.ime
            .report_conv_open_inference(true, ConvSyncReason::NativeToggleShadowOff, TickMs(0));
        let now = std::time::Instant::now();
        let explicit_intent = ps.ime.explicit_intent();
        assert_eq!(
            ps.ime.check_drift_correction(now, explicit_intent),
            None,
            "desired と observed が一致していれば補正不要"
        );
    }

    // GJI 候補ポップアップの観測が古くなった場合 (DRIFT_CORRECTION_OBS_MAX_AGE_MS 超過)
    // は、明示意図があっても採用しない（BUG-20 の max_age ガードが ConvOpenInference
    // にも同じく効くことの確認）。
    #[test]
    fn check_drift_correction_ignores_stale_conv_inference_beyond_max_age() {
        let mut ps = ps_with_shadow(false, Some(UserIntentSource::PhysicalImeKey), true);
        ps.ime
            .report_conv_open_inference(true, ConvSyncReason::NativeToggleShadowOff, TickMs(0));
        let stale_at = std::time::Instant::now()
            - std::time::Duration::from_millis(
                crate::tuning::DRIFT_CORRECTION_OBS_MAX_AGE_MS + 200,
            );
        ps.ime
            .shadow_model
            .observations
            .per_source
            .conv_open_inference
            .as_mut()
            .unwrap()
            .at = stale_at;
        let now = std::time::Instant::now();
        let explicit_intent = ps.ime.explicit_intent();
        assert_eq!(
            ps.ime.check_drift_correction(now, explicit_intent),
            None,
            "max_age を超えた観測は無視される"
        );
    }

    // ── IntentStore 配線（ADR-087 §5 Phase 1' item8、BUG-51 追補 v3、2026-08-11） ──
    //
    // 実機再発: Ctrl+無変換 で明示 IME OFF を送った直後、同一ウィンドウへの
    // フォーカス再構築（`FocusChanged`、スリープ復帰直後の同一アプリ再フォーカス等）
    // で `last_intent` がクリアされ、直後の `ConvOpenInference`（TsfNative の壊れた
    // conv 観測、BUG-55）1 件だけで `effective_open()` が true に反転し、実 IME は
    // OFF のままなのに `Engine::compute_state` が `ctx.ime_on=true` を受け取って
    // 再活性化する（「IME OFF, Engine ON」）。IntentStore は `HwndId` 単位で
    // `FocusChanged` をまたいで明示意図を保持するため、この反転を防ぐ。
    //
    // v3（pre-mortem #1/#2 反映）: IntentStore への record() は
    // `record_explicit_intent`（本物のユーザー操作と確定できる3箇所のみ）が行う。
    // 生の `dispatch_event(UserImeSetIntent)` だけでは記録されない
    // （conv 由来の内部同期がこの経路を偽装できないようにするため）。

    use super::super::ime_event::ImePolicyProfile;

    const TARGET_HWND: HwndId = HwndId(0x1234);

    fn dispatch_focus_changed(ps: &mut PlatformState, to: HwndId, focus_epoch: u64, tick_ms: u64) {
        ps.ime.dispatch_event(
            ImeEvent::FocusChanged {
                from: None,
                to,
                profile: ImePolicyProfile::TsfNative,
                focus_epoch,
            },
            TickMs(tick_ms),
        );
    }

    /// 壊れた conv 由来 open 推論（`NativeToggleShadowOff`、BUG-55）を 1 件だけ
    /// 流し込む。ADR-089 Phase A 以降、この観測を構築できるのは
    /// `report_conv_open_inference()`（`Observed<evidence::ConvOpenInference>` の
    /// witness 構築子を通す唯一の経路）だけなので、本番と同じ口を使う。
    fn dispatch_conv_open_inference(ps: &mut PlatformState, open: bool, tick_ms: u64) {
        ps.ime.report_conv_open_inference(
            open,
            ConvSyncReason::NativeToggleShadowOff,
            TickMs(tick_ms),
        );
    }

    /// `IntentWitness`（ADR-089 §2.2）を作るための「注入されていない実キー
    /// イベント」。`write_sync_key` / `write_physical_key` は witness 無しには
    /// 呼べないため、テストからもこの経路を通す。
    fn physical_ime_key_event() -> awase::types::RawKeyEvent {
        use awase::types::{
            ImeRelevance, KeyClassification, KeyEventType, ModifierState, ScanCode,
            ShadowImeAction, VkCode,
        };
        awase::types::RawKeyEvent {
            vk_code: VkCode(0xF2),
            scan_code: ScanCode(0),
            event_type: KeyEventType::KeyDown,
            extra_info: 0,
            timestamp: 0,
            key_classification: KeyClassification::Passthrough,
            physical_pos: None,
            ime_relevance: ImeRelevance {
                may_change_ime: true,
                shadow_action: Some(ShadowImeAction::TurnOff),
                is_sync_key: true,
                sync_direction: Some(ShadowImeAction::TurnOff),
                is_ime_control: false,
            },
            modifier_key: None,
            modifier_snapshot: ModifierState::default(),
            injected: false,
        }
    }

    fn sync_key_witness() -> IntentWitness {
        IntentWitness::from_sync_key(&physical_ime_key_event())
            .expect("注入されていない sync キーは必ず witness になる")
    }

    fn physical_key_witness() -> IntentWitness {
        IntentWitness::from_physical(&physical_ime_key_event())
            .expect("注入されていない物理 IME キーは必ず witness になる")
    }

    /// `write_sync_key`/`kp_stage_post_decision` の `ExplicitUserAction` 分岐が
    /// 実機で行う「belief 書き込み + IntentStore 記録」の組を1関数にまとめた
    /// テストダブル。
    ///
    /// **注意（追補4）**: これで記録した `IntentStore` エントリを読むときは、
    /// 必ず `ps.ime.effective_open_at(TickMs(..))` を使い、ここで渡した合成 tick と
    /// 同じ時間軸で評価すること。引数なしの `effective_open()` は
    /// `GetTickCount64()`（実機では数分〜数日）を読むため、合成 tick で記録した
    /// エントリは常に TTL 超過となり、上書きが一度も発火しないまま「テストは
    /// 通っている」状態になる（実際に 2026-08-13 の windows-build 失敗を招いた）。
    fn dispatch_and_record_explicit_intent(ps: &mut PlatformState, target: bool, tick_ms: u64) {
        ps.ime.dispatch_event(
            ImeEvent::UserImeSetIntent {
                target,
                source: UserIntentSource::Command,
            },
            TickMs(tick_ms),
        );
        ps.ime
            .record_explicit_intent(target, UserIntentSource::Command, TickMs(tick_ms));
    }

    /// 中核の回帰テスト: 明示 OFF → 同一対象への FocusChanged（last_intent 消失）→
    /// 壊れた ConvOpenInference 観測、という実機再現手順で、生の
    /// `ImeModel::effective_open()` は true に反転してしまうが（退行の証拠として
    /// 明示的にアサートする）、`PlatformState::effective_open()`（IntentStore 込み）
    /// は false を維持することを確認する。
    #[test]
    fn effective_open_survives_focus_change_via_intent_store() {
        let mut ps = PlatformState::new();
        dispatch_focus_changed(&mut ps, TARGET_HWND, 1, 0);
        dispatch_and_record_explicit_intent(&mut ps, false, 100);
        assert!(
            !ps.ime.effective_open_at(TickMs(100)),
            "明示 OFF 直後は false"
        );

        // 同一対象への FocusChanged（例: スリープ復帰直後の同一アプリ再フォーカス）
        // が last_intent をクリアする。
        dispatch_focus_changed(&mut ps, TARGET_HWND, 2, 200);
        assert!(
            ps.ime.explicit_intent().is_none(),
            "FocusChanged は last_intent を無条件にクリアする"
        );

        // 壊れた conv 観測（NativeToggleShadowOff 由来）が届く。
        dispatch_conv_open_inference(&mut ps, true, 300);

        assert!(
            ps.ime.model().effective_open(),
            "退行の証拠: IntentStore 抜きの生の ImeModel::effective_open() は \
             ConvOpenInference 1 件だけで true に反転する（BUG-63 と同型の機構）"
        );
        assert!(
            !ps.ime.effective_open_at(TickMs(300)),
            "IntentStore 込みの PlatformState::effective_open() は同一対象なら \
             明示 OFF 意図を維持し、Engine の ctx.ime_on が誤って true に反転しない"
        );
    }

    /// 対象が違えば IntentStore は効かない（ADR-087 INV-24(b) の2段判定、BUG-26 非退行）。
    /// 別ウィンドウへの本物のフォーカス変更では、そのウィンドウ自身の観測に従うべき。
    #[test]
    fn effective_open_intent_store_does_not_leak_to_different_target() {
        let mut ps = PlatformState::new();
        dispatch_focus_changed(&mut ps, TARGET_HWND, 1, 0);
        dispatch_and_record_explicit_intent(&mut ps, false, 100);

        let other_hwnd = HwndId(0x5678);
        dispatch_focus_changed(&mut ps, other_hwnd, 2, 200);
        dispatch_conv_open_inference(&mut ps, true, 300);

        assert!(
            ps.ime.effective_open_at(TickMs(300)),
            "別ウィンドウへの本物のフォーカス変更では、IntentStore は別対象の \
             エントリを漏らさず、その対象の観測（true）に従う"
        );
    }

    /// OFF 意図の TTL 超過後は IntentStore もフォールバックする（無期限固着はしない）。
    #[test]
    fn effective_open_intent_store_entry_expires_after_ttl() {
        let mut ps = PlatformState::new();
        dispatch_focus_changed(&mut ps, TARGET_HWND, 1, 0);
        dispatch_and_record_explicit_intent(&mut ps, false, 0);
        dispatch_focus_changed(&mut ps, TARGET_HWND, 2, 0);
        dispatch_conv_open_inference(&mut ps, true, 0);

        let off_ttl = crate::tuning::EXPLICIT_OFF_INTENT_TTL_MS;
        // まだ TTL 内: IntentStore が効いて false を維持。
        assert!(!ps.ime.effective_open_at(TickMs(off_ttl)));

        // TTL 超過後に再度観測を読む（IntentStore.record は行われていないので
        // エントリ自体は動かない、現在時刻だけ進める）。
        dispatch_conv_open_inference(&mut ps, true, off_ttl + 1);
        assert!(
            ps.ime.effective_open_at(TickMs(off_ttl + 1)),
            "OFF 意図が TTL を超えたら IntentStore は無期限固着せず、\
             観測ベースの effective_open() にフォールバックする"
        );
    }

    /// PanicReset は同一対象の古い IntentStore エントリより優先される
    /// （安全弁が古い明示意図に負けてはならない、時系列比較の余地なく常に最新の決定）。
    #[test]
    fn effective_open_panic_reset_overrides_stale_intent_store_entry() {
        let mut ps = PlatformState::new();
        dispatch_focus_changed(&mut ps, TARGET_HWND, 1, 0);
        dispatch_and_record_explicit_intent(&mut ps, false, 0);
        assert!(!ps.ime.effective_open_at(TickMs(0)));

        ps.ime.apply_panic_reset(TickMs(100));

        assert!(
            ps.ime.effective_open_at(TickMs(100)),
            "PanicReset は desired_open=true に戻し、IntentStore の古い OFF \
             エントリを無効化するため、effective_open() は true になる"
        );
    }

    /// 修正1b 回帰: 生の `dispatch_event(UserImeSetIntent)` だけでは IntentStore に
    /// 記録されない（`record_explicit_intent` を経由しない限り）。v1 のままだと
    /// `EngineSync::DirectInput`（conv 由来、`handle_engine_set_open` 経由で
    /// `UserImeSetIntent{Command}` を dispatch する）が壊れた conv 読み1件を
    /// FocusChanged を生き延びる偽の明示意図として永続化してしまっていた
    /// （pre-mortem #1 角度2）。
    #[test]
    fn dispatch_event_alone_does_not_record_intent_store_entry() {
        let mut ps = PlatformState::new();
        dispatch_focus_changed(&mut ps, TARGET_HWND, 1, 0);
        ps.ime.dispatch_event(
            ImeEvent::UserImeSetIntent {
                target: false,
                source: UserIntentSource::Command,
            },
            TickMs(100),
        );
        assert!(
            !ps.ime.model().desired_open(),
            "belief (desired_open) はこれまでどおり書かれる"
        );

        // 同一対象への FocusChanged が last_intent をクリアする。
        dispatch_focus_changed(&mut ps, TARGET_HWND, 2, 200);
        dispatch_conv_open_inference(&mut ps, true, 300);

        assert!(
            ps.ime.effective_open_at(TickMs(300)),
            "record_explicit_intent を経由しない dispatch_event だけでは \
             IntentStore に何も残らないため、FocusChanged 後は通常どおり \
             観測（conv, true）にフォールバックする"
        );
    }

    /// 修正1b 正常系: `write_sync_key`/`write_physical_key` は実ユーザー操作として
    /// IntentStore に記録し、FocusChanged 後も維持される。
    #[test]
    fn write_sync_key_records_intent_store_entry_surviving_focus_change() {
        let mut ps = PlatformState::new();
        dispatch_focus_changed(&mut ps, TARGET_HWND, 1, 0);
        ps.ime
            .write_sync_key(sync_key_witness(), false, TickMs(100));
        dispatch_focus_changed(&mut ps, TARGET_HWND, 2, 200);
        dispatch_conv_open_inference(&mut ps, true, 300);
        assert!(
            !ps.ime.effective_open_at(TickMs(300)),
            "write_sync_key の明示 OFF は IntentStore に記録され、\
             FocusChanged をまたいで維持される"
        );
    }

    #[test]
    fn write_physical_key_records_intent_store_entry_surviving_focus_change() {
        let mut ps = PlatformState::new();
        dispatch_focus_changed(&mut ps, TARGET_HWND, 1, 0);
        ps.ime
            .write_physical_key(physical_key_witness(), false, TickMs(100));
        dispatch_focus_changed(&mut ps, TARGET_HWND, 2, 200);
        dispatch_conv_open_inference(&mut ps, true, 300);
        assert!(
            !ps.ime.effective_open_at(TickMs(300)),
            "write_physical_key の明示 OFF は IntentStore に記録され、\
             FocusChanged をまたいで維持される"
        );
    }

    /// 修正2a (i): キャッシュより新しい明示意図は `apply_hwnd_cache_restore` で
    /// 消えない（BUG-57 型: フォーカス滞在 100ms 未満だと退場時の cache 保存が
    /// スキップされ、古いキャッシュが残ったまま復帰することがある）。
    #[test]
    fn apply_hwnd_cache_restore_keeps_intent_newer_than_cache() {
        let mut ps = PlatformState::new();
        dispatch_focus_changed(&mut ps, TARGET_HWND, 1, 0);
        ps.ime
            .write_sync_key(sync_key_witness(), false, TickMs(500));
        ps.ime.apply_hwnd_cache_restore(
            Some(crate::focus::hwnd_cache::HwndImeSnapshot {
                ime_on: true,
                input_mode: InputModeState::ObservedRomaji,
                recorded_ms: 100,
                from_explicit_off_intent: false,
            }),
            TickMs(600),
        );
        assert!(
            !ps.ime.effective_open_at(TickMs(600)),
            "キャッシュ(recorded_ms=100)より新しい明示意図(recorded_at_ms=500)は \
             cache restore で消えず、effective_open() は意図側(false)を返す"
        );
    }

    /// 修正2a (ii): キャッシュの方が新しい（または同時刻）場合は、意図を除去して
    /// キャッシュ復元を優先する（v1 と同じ「最新の決定が勝つ」原則）。
    #[test]
    fn apply_hwnd_cache_restore_discards_intent_older_than_cache() {
        let mut ps = PlatformState::new();
        dispatch_focus_changed(&mut ps, TARGET_HWND, 1, 0);
        ps.ime
            .write_sync_key(sync_key_witness(), false, TickMs(100));
        ps.ime.apply_hwnd_cache_restore(
            Some(crate::focus::hwnd_cache::HwndImeSnapshot {
                ime_on: true,
                input_mode: InputModeState::ObservedRomaji,
                recorded_ms: 500,
                from_explicit_off_intent: false,
            }),
            TickMs(600),
        );
        assert!(
            ps.ime.effective_open_at(TickMs(600)),
            "キャッシュ(recorded_ms=500)より古い意図(recorded_at_ms=100)は \
             cache restore で無効化され、effective_open() はキャッシュ値(true)を返す"
        );
    }

    /// 修正2b: 有効な IntentStore エントリがある間、`reset_stale_ime_on_for_imm_broken`
    /// （BUG-16 系 safety-net）は `HeuristicDefault` を書かずに温存する。
    /// 「観測ゼロの推測が明示意図に勝つ」逆転を避ける（pre-mortem #2）。
    #[test]
    fn reset_stale_ime_on_for_imm_broken_preserves_valid_intent_store_entry() {
        let mut ps = PlatformState::new();
        ps.ime.belief.is_japanese_ime = true;
        dispatch_focus_changed(&mut ps, TARGET_HWND, 1, 0);
        ps.ime
            .write_sync_key(sync_key_witness(), false, TickMs(100));
        // 同一対象への FocusChanged が last_intent と observations をクリアする
        // （safety-net の第一ガードが素通りする状態を作る）。
        dispatch_focus_changed(&mut ps, TARGET_HWND, 2, 200);
        assert!(!ps.ime.effective_open_at(TickMs(200)));

        ps.ime
            .reset_stale_ime_on_for_imm_broken(ImePolicyProfile::Imm32Unavailable, TickMs(300));

        assert!(
            !ps.ime.effective_open_at(TickMs(300)),
            "IntentStore に有効な OFF エントリがある間は HeuristicDefault(ON) が \
             書かれず、effective_open() は false のまま"
        );
    }
}
