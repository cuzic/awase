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
use super::ime_model::{AppliedImeState, ImeApplyAcceptance, ImeModel};
use super::input_barrier::InputBarrier;
use super::mode_key_pass::ModeKeyPassLatch;
use super::scoped_latch::ScopedOneShot;
use super::{ApplyGeneration, TickMs};
use crate::journal::{JournalEntry, UnifiedJournal};

// OS（フォアグラウンド）を読む殻。核の `_in_scope` 版を呼ぶ1行だけを置く（FCIS P2）。
#[cfg(windows)]
mod shell;

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
pub struct ImeStateHub {
    /// input_mode・is_japanese_ime・prev_conversion_mode を保持する。
    pub(crate) belief: ImeBelief,
    /// IME 状態変更 event の `seq` 採番器（ADR-232 D2: 旧リングバッファは撤去）。
    pub(crate) event_log: ImeEventLog,
    /// 時刻の供給元（実機は実時計、閉ループ・テストは仮想時計。`state/hub_clock.rs`）。
    clock: super::hub_clock::HubClock,
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
    /// 経由でのみ発行されるようになり、エンジン内部の対称 echo（旧 `ActivationSync`。
    /// ADR-213 P2c で撤去済み）とは完全に分離された。
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

    /// 無変換/変換の生キーを IME 側へ通過させた直後だけ有効な一回マーク。
    ///
    /// ADR-187 follow 方式: 生キー配送の結果は awase には分からないため、短時間だけ
    /// typing-idle ガードを迂回して観測し、観測成功後に古い明示意図を捨てる。
    /// 寿命判断そのものは `state/mode_key_pass.rs::ModeKeyPassLatch`（Win32非依存）に委譲する
    /// （design-patterns-review.md 提案3）。ここは副作用（`intent_store`/`dispatch_event`）を
    /// 適用する側に回る。
    mode_key_pass_mark: ModeKeyPassLatch<crate::state::foreground_scope::ForegroundScope>,

    /// 外部注入の IME キー直後だけ開く短い監視窓（ADR-205、BUG-172）。読めない窓（`Imm32Unavailable`）で、
    /// 窓の中の prefetch 済みの開閉の読みが基準値から変わったときだけ実状態へ追随する。寿命・基準値の判断は
    /// `state/external_change_watch.rs`（Win32非依存）に委譲し、ここは副作用の適用側。
    external_change_watch: super::external_change_watch::ExternalChangeWatch<
        crate::state::foreground_scope::ForegroundScope,
    >,

    /// 最後に外部変化へ追随した時刻（ms）。追随の直後に、閉じる前の GJI I/O 推測が `ObserverPoll(true)` で
    /// 追随結果を上書きしないための柵（`observe_gji_after_focus` の第1引数）に使う（ADR-205 round3 m1）。
    last_external_change_ms: u64,

    /// `effective_open()` の IntentStore 分岐が `shadow_model` と異なる値を
    /// 返している（＝実際に override している）間 `true`。遷移時のみ INFO
    /// ログを出すための dedup 用（BUG-51 追補 v3）。`&self` の `effective_open()`
    /// から更新するため `Cell`——`ImeStateHub` は単一 UI スレッドが所有する
    /// （`with_app` パターン）ため `!Sync` でも問題ない。
    intent_override_logged: std::cell::Cell<bool>,

    /// [`ImeStateHub::resolve_warmup_ime_on`] の `off_drift_active` ゲートが
    /// `ApplyGeneration` 専用アロケータ（ADR-106 決定1）。`event_log.next_seq()`
    /// から独立しており、診断ログの記録有無と generation の一意性が無関係になる。
    generation_alloc: super::GenerationAllocator,

    /// 「この押下で既に書いた」の予約（`last_written_press`、ADR-208 決定2 D1）。belief ではない
    /// （`ImeModel` の外。`ImeEvent` を介さず、order の発行時点で `claim_press_write` が更新する）。
    press_ledger: super::press_ledger::PressLedger,
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

/// [`ImeStateHub::check_drift_correction`] の戻り値。定義は ungated な
/// `state/drift_correction.rs` へ移した（Linux ホストのテストから判定本体を呼ぶため）。
pub(crate) use super::drift_correction::DriftCorrection;

impl ImeStateHub {
    /// 時計を注入して初期化する。`hook` に依存しないので、テスト・閉ループは仮想時計
    /// （`HubClock::manual`）を渡せる。
    ///
    /// 閉ループのハーネス（`tests/support/harness.rs`）からも呼ぶ。本番の呼び出し元は crate 内だけ。
    #[must_use]
    pub fn with_clock(clock: super::hub_clock::HubClock) -> Self {
        Self {
            belief: ImeBelief::default(),
            event_log: ImeEventLog::default(),
            clock,
            journal: UnifiedJournal::default(),
            shadow_model: ImeModel::default(),
            last_user_explicit_off_ms: 0,
            last_explicit_ime_action_ms: 0,
            intent_store: super::intent_store::IntentStore::default(),
            mode_key_pass_mark: ModeKeyPassLatch::new(),
            external_change_watch: super::external_change_watch::ExternalChangeWatch::new(),
            last_external_change_ms: 0,
            intent_override_logged: std::cell::Cell::new(false),
            generation_alloc: super::GenerationAllocator::new(),
            press_ledger: super::press_ledger::PressLedger::default(),
        }
    }
}

impl ImeStateHub {
    /// Event に seq を採番し、shadow_model に reduce して journal に記録する (Step 1)。
    ///
    /// `event_log.record_at()` だけを呼ぶより、こちらを使うと採番 + reduce が
    /// 同一 envelope で進む。write_* メソッドはこちらを使う。
    ///
    /// `tick_ms`: 呼び出し元が取得した現在時刻（`GetTickCount64` 由来）。
    /// state/ 層が `hook::current_tick_ms()` を直接呼ばないよう注入する。
    ///
    /// 閉ループのハーネス（`tests/support/harness.rs`）からも呼ぶ。本番の呼び出し元は crate 内だけ。
    pub fn dispatch_event(&mut self, event: ImeEvent, tick_ms: TickMs) {
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
                // Command ソースは conv 由来の内部同期（EngineSync::DirectInput〈ADR-185で撤去済み〉 →
                // handle_engine_set_open → write_set_open_request）でも dispatch される
                // ため、このイベントだけでは「本物のユーザー操作」と区別できない。
                // 記録は実ユーザー操作と確定できる呼び出し元
                // （record_explicit_intent の doc 参照）が行う。
            }
        }
        let time = self
            .event_log
            .record_at(&event, tick_ms, self.clock.now_instant());
        let envelope = ImeEventEnvelope { time, event };
        self.shadow_model.reduce(&envelope);
        self.journal.record(JournalEntry::ImeEvent {
            event: envelope.event,
            event_seq: time.seq,
            tick_ms: time.tick_ms,
        });
    }

    /// 明示キー押下 `press` の向き `open` の書き込みを**予約**する（order の発行直前に呼ぶ。ADR-208 決定2 D1）。
    ///
    /// ImmCross の書き込みは async で完了が WM 経由で後から届くので、完了時でなく発行時に予約する
    /// （同じ打鍵の Engine の `SetOpen` が先に評価されて二重に送るのを防ぐ）。`UnsafeToToggle`/`Failed` で書けなくても
    /// 予約は解かない（同一押下内の再試行はしない。次の押下で直る）。判定は純粋な [`PressLedger::claim`]。
    /// 戻り値の `writes()` が `false`（同じ押下で既に書いた）なら、呼び出し側は order を発行せず書かない。
    /// 押下 ID の無い order（`press=None`）は記録に触れず `Unpressed`（従来どおり `applied` の省略に任せる）。
    ///
    /// 衝突（同じ押下で向きが違う経路）は journal（`PressWriteClaim`、派生の debug 行 `press write claim`）に残す。優先順位は Engine の明示コンボ > shadow
    /// （`state/press_ledger.rs` のモジュール doc）。
    pub(crate) fn claim_press_write(
        &mut self,
        press: Option<awase::types::PressId>,
        open: bool,
        source: super::press_ledger::PressSource,
    ) -> super::press_ledger::PressClaim {
        let claim = self.press_ledger.claim(press, open, source);
        if let Some(press) = press {
            self.journal.record(JournalEntry::PressWriteClaim {
                press: press.get(),
                open,
                source: source.label(),
                verdict: claim.label(),
            });
        }
        claim
    }

    /// 同期の書き込みが何も送らなかったとき（`press_ledger::outcome_sent_nothing`）、同一押下の予約を解く
    /// （次の経路〈同じ押下の Engine 等〉が改めて書ける。ADR-208 L1 / PR #419 Opus M-2）。async は完了が後から届くので解かない。
    pub(crate) fn release_press_write(&mut self, press: Option<awase::types::PressId>, open: bool) {
        if self.press_ledger.release(press, open) {
            if let Some(press) = press {
                self.journal.record(JournalEntry::PressWriteClaim {
                    press: press.get(),
                    open,
                    source: "release",
                    verdict: "released",
                });
            }
        }
    }

    /// shadow_model から派生した最新の explicit intent。
    ///
    /// (Step 2B 以降の SSOT。Priority 4-5 observer による上書きを block する根拠。)
    pub(crate) fn explicit_intent(&self) -> Option<bool> {
        self.shadow_model.last_intent.as_ref().map(|i| i.target)
    }

    /// 物理モードキーの打鍵時点で、表から予測した効果をbeliefへ反映する（ADR-191 決定3）。
    ///
    /// `ImeEvent::KeyEffectPredicted`の**唯一のdispatch元**。awaseはIMEへ書かない（生キーはそのまま通る）。
    /// 後から来る観測（settle後）が照合し、食い違えば観測が勝つ（`ImeModel::reduce`のfence）。
    ///
    /// 閉ループのハーネス（`tests/support/harness.rs`）からも呼ぶ。本番の呼び出し元は crate 内だけ。
    pub fn apply_key_effect_prediction(
        &mut self,
        prediction: crate::state::key_effect_predictor::Prediction,
        tick_ms: TickMs,
    ) {
        // 開閉・入力モードも追跡状態も変わらない打鍵は何もしない。
        if prediction.effect.is_noop() && prediction.track == self.shadow_model.key_track() {
            return;
        }
        self.dispatch_event(
            ImeEvent::KeyEffectPredicted {
                open: prediction.effect.open,
                mode: prediction.effect.mode,
                track: prediction.track,
            },
            tick_ms,
        );
        // 開閉の予測は、この対象に残る古い明示意図（例: 起動直後の明示IME OFF）を置き換える。
        // `IntentStore`は`effective_open()`で`shadow_model`より優先されるので、消さないと予測が効かない
        // （読めないアプリでは観測が来ず、意図のTTL〈約30秒〉が切れるまで開閉の予測が無視される）。
        // 「同一対象では最新の決定が古い意図を置換する」という`IntentStore`自身の設計と、通過マークの
        // 観測（`consume_mode_key_pass_mark`）が同じ対象の意図を消す扱いに揃える。
        if prediction.effect.open.is_some() {
            if let Some(hwnd) = self.shadow_model.current_focus() {
                self.intent_store.remove(hwnd);
            }
        }
    }

    // ── 通過マーク（ADR-187）: 寿命判断は `ModeKeyPassLatch`（`state/mode_key_pass.rs`）に委譲する ──
    //
    // ここに残るのは、latch が返す判断・`PassEffect` を実際に適用する副作用（`intent_store.remove`・
    // `dispatch_event(ModeKeyPassedThrough)`）だけ（design-patterns-review.md 提案3）。
    // 各メソッドのシグネチャは委譲前と変えていない（呼び出し元・テストの変更を避けるため）。

    /// 無変換/変換の生キーを通過させたら呼ぶ（ADR-187）。現在のフォアグラウンドに対する一回マークを立てる。
    fn arm_mode_key_pass_mark_in_scope(
        &mut self,
        now_ms: u64,
        readable: bool,
        scope: crate::state::foreground_scope::ForegroundScope,
    ) {
        self.mode_key_pass_mark.arm(scope, now_ms, readable);
    }

    /// 立てた時点で読める窓だった通過マークが、窓の終了を待っているとき、その残り時間(ms)。
    /// 通過の途中で窓が読めなくなった（降格した）場合に、窓の終了時に`expire_mode_key_pass_mark`を呼ぶための
    /// 起床時刻に使う（読めない窓の`reschedule_ime_refresh`は通過マークが有効な間は何も予約しないため）。
    fn mode_key_pass_expiry_wait_ms_in_scope(
        &mut self,
        now_ms: u64,
        scope: crate::state::foreground_scope::ForegroundScope,
    ) -> Option<u64> {
        self.mode_key_pass_mark.expiry_wait_ms(
            now_ms,
            scope,
            crate::tuning::MODE_KEY_PASS_MARK_WINDOW_MS,
        )
    }

    /// awaseが実際にIMEへ書いた（`applied`を更新した）ことを、有効な通過マークへ記録する（BUG-158追補2）。
    fn note_awase_write_for_mode_key_pass_in_scope(
        &mut self,
        scope: crate::state::foreground_scope::ForegroundScope,
    ) {
        self.mode_key_pass_mark.note_awase_write(scope);
    }

    fn mode_key_pass_mark_live_in_scope(
        &mut self,
        now_ms: u64,
        scope: crate::state::foreground_scope::ForegroundScope,
    ) -> bool {
        self.mode_key_pass_mark
            .live(now_ms, scope, crate::tuning::MODE_KEY_PASS_MARK_WINDOW_MS)
    }

    /// 通過マークの窓が切れるまでの残り時間(ms)。マークが無い/フォアグラウンドが変わった/窓が切れていれば`None`。
    /// 観測が失敗した通過の後、読み直しを窓の終了時の1回に絞るために使う（BUG-158）。
    fn mode_key_pass_window_remaining_ms_in_scope(
        &mut self,
        now_ms: u64,
        scope: crate::state::foreground_scope::ForegroundScope,
    ) -> Option<u64> {
        self.mode_key_pass_mark.window_remaining_ms(
            now_ms,
            scope,
            crate::tuning::MODE_KEY_PASS_MARK_WINDOW_MS,
        )
    }

    fn invalidate_intents_if_mode_key_pass_live_in_scope(
        &mut self,
        now_ms: u64,
        tick_ms: TickMs,
        scope: crate::state::foreground_scope::ForegroundScope,
    ) -> bool {
        self.drop_intents_for_mode_key_pass_in_scope(now_ms, tick_ms, scope, false)
    }

    fn expire_mode_key_pass_mark_in_scope(
        &mut self,
        now_ms: u64,
        tick_ms: TickMs,
        scope: crate::state::foreground_scope::ForegroundScope,
    ) -> bool {
        self.drop_intents_for_mode_key_pass_in_scope(now_ms, tick_ms, scope, true)
    }

    /// 判断は `ModeKeyPassLatch::drop_decision`（Win32非依存）。ここは`PassEffect`の適用のみ。
    fn drop_intents_for_mode_key_pass_in_scope(
        &mut self,
        now_ms: u64,
        tick_ms: TickMs,
        scope: crate::state::foreground_scope::ForegroundScope,
        on_expiry: bool,
    ) -> bool {
        let Some(effect) = self.mode_key_pass_mark.drop_decision(
            now_ms,
            scope,
            on_expiry,
            self.shadow_model.last_intent.is_some(),
            crate::tuning::MODE_KEY_PASS_MARK_WINDOW_MS,
        ) else {
            return false;
        };
        if effect.remove_intent {
            if let Some(hwnd) = self.shadow_model.current_focus() {
                self.intent_store.remove(hwnd);
            }
        }
        if effect.pass_through {
            // 窓の終了時の破棄（`on_expiry`）は観測を得ていない: 意図だけ捨て、desired は書かない（A-N1）。
            self.pass_through_observed(tick_ms, !on_expiry, false);
        }
        true
    }

    // ── 外部変化の監視窓（ADR-205、BUG-172）──

    /// 外部注入の IME キーを見たら呼ぶ（読めない窓のみ）。現在のフォアグラウンドに対する監視窓を開く／延ばす。
    ///
    /// 閉ループのハーネス（`tests/support/harness.rs`）からも呼ぶ。本番の呼び出し元は crate 内だけ。
    pub fn arm_external_change_watch_in_scope(
        &mut self,
        now_ms: u64,
        scope: crate::state::foreground_scope::ForegroundScope,
    ) {
        self.external_change_watch
            .arm(scope, now_ms, crate::tuning::MODE_KEY_PASS_MARK_WINDOW_MS);
    }

    /// 監視窓の基準値(ログ用。ADR-227 の give-up 契機で、追随が起きなかった理由を区別する)。
    pub(crate) fn external_change_baseline(&self) -> Option<bool> {
        self.external_change_watch.baseline()
    }

    /// 監視窓の残り時間(ms)。無い・切れた・フォアグラウンドが変わったなら`None`（`reschedule_ime_refresh`の読み直し予約用）。
    fn external_change_watch_remaining_ms_in_scope(
        &mut self,
        now_ms: u64,
        scope: crate::state::foreground_scope::ForegroundScope,
    ) -> Option<u64> {
        self.external_change_watch.remaining_ms(
            scope,
            now_ms,
            crate::tuning::MODE_KEY_PASS_MARK_WINDOW_MS,
        )
    }

    /// 最後に外部変化へ追随した時刻（ms）。0 は未追随。
    pub(crate) const fn last_external_change_ms(&self) -> u64 {
        self.last_external_change_ms
    }

    /// prefetch 済みの開閉の読み（`read`）を監視窓に照合し、窓の中で基準値から変わっていれば実状態へ追随する。
    ///
    /// 追随 = `ObserverPoll(v)` を記録 → 対象の明示意図（`IntentStore`）を削除 → `ModeKeyPassedThrough{align_desired:true}`
    /// （`last_intent` を捨て、`desired_open` を観測へ揃え、食い違う `applied` を未確認へ落とす）。awase は IME を書かない。
    /// 開く・閉じるの両方向を同じ規則で追随する（呼び出し側が GJI × Imm32Unavailable に限る）。戻り値は追随した値。
    /// どのフォーカスでも直近の読みは記録する（基準値の初期値になる）。
    ///
    /// 閉ループのハーネス（`tests/support/harness.rs`）からも呼ぶ。本番の呼び出し元は crate 内だけ。
    pub fn follow_external_change_in_scope(
        &mut self,
        read: Option<bool>,
        now_ms: u64,
        tick_ms: TickMs,
        accepted: crate::state::probe_admission::AcceptedObservation,
        scope: crate::state::foreground_scope::ForegroundScope,
    ) -> Option<bool> {
        let verdict = self.external_change_watch.observe(
            scope,
            now_ms,
            crate::tuning::MODE_KEY_PASS_MARK_WINDOW_MS,
            read,
        );
        self.external_change_watch.record_read(scope, read);
        let super::external_change_watch::ChangeVerdict::Changed(v) = verdict else {
            return None;
        };
        self.write_observer_poll(v, tick_ms, accepted);
        if let Some(hwnd) = self.shadow_model.current_focus() {
            self.intent_store.remove(hwnd);
        }
        self.pass_through_observed(tick_ms, true, true);
        self.last_external_change_ms = now_ms;
        Some(v)
    }

    /// `ModeKeyPassedThrough` のdispatch元（ADR-187の「1箇所に限定」）。reducerは`last_intent`を捨て、
    /// `desired_open`を観測から導ける開閉へ揃える（BUG-157）。窓の間の揃えと、窓が切れた後の最初の成功観測での
    /// 揃え（BUG-158追補2）の両方がここを通る。
    fn pass_through_observed(
        &mut self,
        tick_ms: TickMs,
        align_desired: bool,
        demote_applied: bool,
    ) {
        self.dispatch_event(
            ImeEvent::ModeKeyPassedThrough {
                align_desired,
                demote_applied,
            },
            tick_ms,
        );
    }

    /// `desired_open` が起動時の初期値のまま（BUG-163）か。`true` の間、`desired_open` は awase の意図ではない。
    #[must_use]
    pub(crate) fn desired_is_placeholder(&self) -> bool {
        self.shadow_model.desired_is_placeholder()
    }

    /// 起動時の初期値のままの `desired_open` を、最初の成功観測へ**1回だけ**揃える（BUG-163、代案A）。
    ///
    /// 初期値 `true` は「観測が無いときの既定」で、awase が IME にそうしたい意図ではない。揃えないと、IME を閉じて起動したとき
    /// 最初の観測「閉」が初期値 `true` と比べられ、明示意図が無いのに drift 補正が発火する（`ir_apply_drift_correction`）。
    /// 揃える条件: 初期値のまま（`desired_is_placeholder`）、明示意図が無い（`last_intent`）、観測から導ける開閉
    /// （`derive_any`）がある。揃えたら（`ModeKeyPassedThrough { align_desired: true }` の reducer 経路、BUG-157 と同じ）
    /// 揃えた後の `desired_open` を返す。揃えなかったら `None`（読めない窓では観測が来るまで触れない）。
    pub(crate) fn align_placeholder_desired(
        &mut self,
        now: std::time::Instant,
        tick_ms: TickMs,
    ) -> Option<bool> {
        if !self.shadow_model.desired_is_placeholder() || self.shadow_model.last_intent.is_some() {
            return None;
        }
        self.shadow_model.observations.derive_any(now)?;
        self.pass_through_observed(tick_ms, true, false);
        Some(self.shadow_model.desired_open())
    }

    /// 通過マークの窓が**切れた後**の最初の成功観測で、`desired_open`を観測へ揃える（BUG-158追補2）。
    /// 窓の間の観測が全て時間切れだった通過は、揃える機会が無いまま`observed ≠ desired`が続くため。
    /// 通過につき1回だけ。通過より後にawaseが書いた/新しい明示意図があるときは揃えない。
    /// 観測が成功したときに呼ぶ。揃えたら`true`。
    fn align_after_expired_mode_key_pass_in_scope(
        &mut self,
        now_ms: u64,
        tick_ms: TickMs,
        scope: crate::state::foreground_scope::ForegroundScope,
    ) -> bool {
        if !self.mode_key_pass_mark.align_after_expired(
            now_ms,
            scope,
            self.shadow_model.last_intent.is_some(),
            crate::tuning::MODE_KEY_PASS_MARK_WINDOW_MS,
        ) {
            return false;
        }
        self.pass_through_observed(tick_ms, true, false);
        true
    }

    /// 非同期送信済み・未確認の actuation を記録する（`applied = Optimistic`）。
    ///
    /// ADR-098 決定6-a: 旧 `mirror_applied_open_with_ts(value, 0)` に相当する。
    /// `ts==0` というマジック値ではなく、呼び出し元がどちらの構築子を呼ぶかで
    /// `Optimistic`/`Confirmed` を選ばせることで、「時刻のつもりで渡した値が
    /// 副作用として Confirmed を意味してしまう」という取り違え（BUG-69 F2 の
    /// 発生機構）を型の形から消す。
    ///
    /// # INV-A97-1（決定0）と既知の例外
    ///
    /// 原則: `applied` は実際に OS actuation を試みた経路からのみ書く。
    /// ただし `record_confirmed` の呼び出し元5箇所のうち3箇所
    /// （`ir_post_focus_change_snapshot` の非TsfNative分岐、
    /// `focus_tracking.rs` の hard pre-sync、`process_deferred_keys`〈dead
    /// code〉）は actuation を伴わない belief ミラーであり、この不変条件の
    /// 対象外として ADR-098 決定5 で明示的に許容している（Standard/GJI
    /// プロファイルは `read_ime_state_full` で実状態を確認できるため、
    /// ミラーが誤りでも次の観測で自己修正される——TsfNative のような
    /// 観測不能プロファイルでのみ有害だったのが BUG-69 の本質）。新しい
    /// `record_confirmed`/`record_optimistic` 呼び出しを追加する際は、
    /// この5箇所のどれとも異なる新規パターンなら actuation 由来かどうかを
    /// 必ず確認すること。
    fn record_optimistic_in_scope(
        &mut self,
        open: bool,
        scope: crate::state::foreground_scope::ForegroundScope,
    ) {
        self.note_awase_write_for_mode_key_pass_in_scope(scope);
        self.shadow_model.applied = AppliedImeState::Optimistic(open);
        self.clear_pending_if_matches(open);
    }

    /// 完了が確認された actuation を記録する（`applied = Confirmed`）。
    ///
    /// ADR-098 決定6-a: 旧 `mirror_applied_open_with_ts(value, ts)`（`ts>0`）に相当。
    /// `at_ms`: 呼び出し元が取得した現在時刻（`GetTickCount64` 由来、非ゼロ）。
    /// INV-A97-1 の既知の例外は `record_optimistic` の doc を参照。
    fn record_confirmed_in_scope(
        &mut self,
        open: bool,
        at_ms: u64,
        scope: crate::state::foreground_scope::ForegroundScope,
    ) {
        self.note_awase_write_for_mode_key_pass_in_scope(scope);
        self.shadow_model.confirm_applied(open, at_ms);
    }

    /// 同じ apply が完了した扱いになったので pending も clear する。
    fn clear_pending_if_matches(&mut self, value: bool) {
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
    /// ADR-213 P2d-2: settle 中の SetOpen を belief 側でも落としていた
    /// `focus_transition_was_pending` フィルタは、executor 側の strip と対で撤去した
    /// （明示操作は settle 中も belief を書き、実書き込みも行う）。
    pub(crate) fn handle_engine_set_open(
        &mut self,
        target: bool,
        ctrl_held: bool,
        generation: ApplyGeneration,
        tick_ms: TickMs,
    ) -> bool {
        if super::explicit_press::engine_set_open_filtered_by_chord(
            self.is_ctrl_ime_chord_active(),
            target,
        ) {
            // chord transaction 中の二次 IME OFF 要求: フィルタ。
            // ChordEnded（Ctrl KeyUp）が barrier を解除するため、ここでは何もしない。
            //
            // 診断ログ (2026-08-05): 従来ここは完全無音だったため、実機ログだけでは
            // 「明示 OFF がこのフィルタでサイレント無効化された」ケースを他の原因と
            // 区別できなかった。挙動は変更しない。
            tracing::info!(
                "[chord-filter] SetOpen(false) request filtered: ctrl_ime_chord が既に active \
                 (last_intent/desired_open は更新されない)"
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
        tracing::debug!("[ctrl-bypass] chord barrier cleared (Ctrl KeyUp vk=0x{vk:02X})");
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
    /// 被せるだけ。このモジュールは ungated（FCIS P4）なので、ここに書いた
    /// `mod tests` は Linux でも走る。判定本体だけの回帰は
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
        self.effective_open_at(TickMs(self.clock.now_tick()))
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
    ///
    /// `shadow_model` の根拠判定（観測の鮮度）に使う `Instant` は `self.clock` から取る
    /// （旧実装は `shadow_model.effective_open()` が壁時計の `Instant::now()` を読んでいたため、
    /// 仮想時計では `now_ms` と時間軸が食い違った。`state/hub_clock.rs`）。
    /// 閉ループのハーネス（`tests/support/harness.rs`）からも呼ぶ。本番の呼び出し元は crate 内だけ。
    #[must_use]
    pub fn effective_open_at(&self, now_ms: TickMs) -> bool {
        let shadow = self
            .shadow_model
            .effective_open_at(self.clock.now_instant());
        let decision = self.intent_store.resolve_effective_open(
            self.shadow_model.current_focus(),
            shadow,
            now_ms,
        );
        match decision.intent {
            Some(intent) if decision.value != shadow => {
                if !self.intent_override_logged.get() {
                    tracing::info!(
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
                    tracing::info!("[intent-store] effective_open override 終了 (shadow が一致)");
                    self.intent_override_logged.set(false);
                }
            }
            None => {
                if self.intent_override_logged.get() {
                    tracing::info!(
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

    /// 次の `ApplyGeneration` を払い出す（ADR-106 決定1）。
    ///
    /// 専用アロケータ（`&mut self`）を使うため「読むだけで進まない」ことが
    /// 型で不可能——旧実装（`event_log.next_seq()` を読むだけ）は `&self` の
    /// ため呼び出し元が別途 `dispatch_event` しないと一意性が壊れる、型で
    /// 守られない契約に依存していた。
    pub(crate) fn allocate_event_generation(&mut self) -> ApplyGeneration {
        self.generation_alloc.allocate()
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
    ///
    /// 閉ループのハーネス（`tests/support/harness.rs`）からも呼ぶ。本番の呼び出し元は crate 内だけ。
    #[must_use]
    pub fn model(&self) -> &ImeModel {
        &self.shadow_model
    }

    /// 時計（`Copy`）。`HubClock::now_instant`・`now_tick` で現在時刻を読む。
    /// 閉ループのハーネス（`tests/support/harness.rs`）からも呼ぶ。本番の呼び出し元は crate 内だけ。
    #[must_use]
    pub const fn clock(&self) -> super::hub_clock::HubClock {
        self.clock
    }

    /// 仮想時計（`HubClock::Manual`）を進める。実時計では何もしない。
    /// 閉ループのハーネス（`tests/support/harness.rs`）からも呼ぶ。本番の呼び出し元は crate 内だけ。
    pub fn advance_clock_ms(&mut self, ms: u64) {
        self.clock.advance_ms(ms);
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
    ///
    /// 閉ループのハーネス（`tests/support/harness.rs`）からも呼ぶ。本番の呼び出し元は crate 内だけ。
    #[must_use]
    pub fn warrant_context(
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

    /// desired ≠ observed ドリフトが補正閾値を超えているか判定し、超えていれば補正情報を、
    /// 超えていなければ理由（`NoDrift`、E1）を返す。
    ///
    /// `explicit_intent`: [`Self::explicit_intent`] の値をそのまま渡す。
    ///
    /// `ConvOpenInference` は根拠にしない（BUG-173 追補3。`state/drift_correction.rs` 参照）。
    /// `resolve_warmup_ime_on` が同じ述語を `matches!(.., Some(DriftCorrection { desired: false, observed: true, .. }))`
    /// として使う（ADR-132/INV-B1'）。
    pub(crate) fn evaluate_drift(
        &self,
        now: std::time::Instant,
        explicit_intent: Option<bool>,
    ) -> Result<DriftCorrection, super::drift_correction::NoDrift> {
        // 判定本体は ungated な `state/drift_correction.rs`（Linux の
        // `tests/closed_loop_scenarios.rs` から呼べるように移した。ロジックは不変）。
        super::drift_correction::evaluate_drift(&self.shadow_model, now, explicit_intent)
    }

    /// [`Self::evaluate_drift`] から理由を捨てたもの。本番の呼び出し元は `evaluate_drift` に移ったので、
    /// これは既存の単体テスト（`check_drift_correction_*`）の窓口として残している。
    #[cfg(test)]
    pub(crate) fn check_drift_correction(
        &self,
        now: std::time::Instant,
        explicit_intent: Option<bool>,
    ) -> Option<DriftCorrection> {
        self.evaluate_drift(now, explicit_intent).ok()
    }

    /// IME apply 完了を記録する（D: generation 照合 dispatch）。
    ///
    /// ADR-108: generation 付き完了の `applied` 書き込みは `ImeModel::reduce()` に
    /// 集約する。戻り値は composition/warmup 副作用を駆動してよいかの判定であり、
    /// `applied` 更新の可否とは分離する。
    ///
    /// generation を持たない既存5経路は現状維持。target 一致で pending を解放し、
    /// `record_confirmed` で `applied` を書く。
    fn record_ime_apply_result_in_scope(
        &mut self,
        open: bool,
        outcome: awase::platform::ImeOpenOutcome,
        generation: Option<ApplyGeneration>,
        ts: u64,
        scope: crate::state::foreground_scope::ForegroundScope,
    ) -> ImeApplyAcceptance {
        let Some(generation) = generation else {
            let Some(effective) = super::ime_model::apply_result_effective_open(open, outcome)
            else {
                return ImeApplyAcceptance::NotSent;
            };
            // `ts` は常に `current_tick_ms()`（非ゼロ）由来——`on_ime_apply_complete`
            // の唯一の呼び出し元（`runtime/mod.rs`）がそうしている。よって
            // 常に `record_confirmed_in_scope`（ADR-098 決定6-a）。
            self.record_confirmed_in_scope(effective, ts, scope);
            return ImeApplyAcceptance::Accepted;
        };

        let acceptance = self
            .shadow_model
            .classify_apply_completion(open, outcome, generation);
        if matches!(acceptance, ImeApplyAcceptance::Stale) {
            tracing::debug!(
                "[ime-apply] stale completion ignored for side effects: target={open} \
                 outcome={outcome:?} generation={generation} pending={:?}",
                self.shadow_model.pending_generation()
            );
        }
        let event = ImeEvent::from_apply_outcome(open, outcome, generation);
        self.dispatch_event(event, TickMs(ts));
        acceptance
    }
}

// ── IME 操作ロジック ─────────────────────────────────────────────────────────
//
// PlatformState から委譲されるメソッド群。shadow_model / belief / event_log への
// 書き込みはすべてここに集約し、PlatformState からは直接 shadow_model を触らない。

impl ImeStateHub {
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

    /// conv 観測由来の engine ON 同期（`EngineSync::SetOpen`）が陽性証拠を得たとき、
    /// `PanicReset` ガードだけを解除する（他 reason のガードは残す）。
    ///
    /// 旧 `handle_conv_engine_on_sync` が `on_set_open_requested` 経由で全ガードを
    /// 消していたうちの、PanicReset 解除だけを引き継ぐ（ADR-213 P2d-1）。
    pub(crate) fn release_panic_reset_guard_on_positive_evidence(&mut self) {
        self.shadow_model
            .force_guards
            .remove(ForceOnReason::PanicReset);
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
        // panic reset はフォーカスエポック/hwnd を変えない（同じフォーカスコンテキスト
        // 内のリセット）。
        let cur_fence = self.shadow_model.observations.current_fence();
        self.shadow_model
            .observations
            .clear_on_focus_change(cur_fence);
    }

    /// `ImeUpdate` を belief / shadow_model に反映する。
    ///
    /// `observer::ime_observer::poll_and_classify_ime()` の結果を受け取り、
    /// 状態への書き込みをここに集約する。判断ロジックを持たない純粋適用関数。
    ///
    /// `tick_ms`: 呼び出し元が取得した現在時刻（`GetTickCount64` 由来）。
    pub(crate) fn apply_ime_update(
        &mut self,
        update: &crate::state::ime_update::ImeUpdate,
        tick_ms: TickMs,
        accepted: crate::state::probe_admission::AcceptedObservation,
    ) {
        if let Some(is_jp) = update.is_japanese_ime {
            self.belief.is_japanese_ime = is_jp;
        }
        if let Some(obs) = update.observer_poll {
            self.dispatch_event(
                ImeEvent::ObserverReported(
                    Observed::<evidence::ObserverPoll>::from_poll(&accepted, obs.value).into(),
                ),
                tick_ms,
            );
        }
        if update.increment_miss_count {
            self.shadow_model
                .observe_miss_monitor
                .record_miss(self.clock.now_instant());
            let miss = self
                .shadow_model
                .observe_miss_monitor
                .consecutive_miss_count;
            if miss == crate::IME_DETECT_MISS_THRESHOLD {
                tracing::warn!(
                    "IME detection failed {miss} consecutive times (force-ON was removed; recording only)"
                );
            }
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
                    tracing::info!(
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
            tracing::debug!(
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
                tracing::debug!(
                    "Imm32Unavailable entry: preserving stored intent open={} (source={:?})",
                    intent.open,
                    intent.source
                );
                return;
            }
        }
        tracing::info!(
            "Imm32Unavailable entry without trusted cache: 安全デフォルト ON を Low confidence \
             observation として記録 (no explicit intent, Japanese layout, IME state \
             uncontrollable in Imm32Unavailable)"
        );
        let focus_epoch = self.shadow_model.observations.current_fence().epoch;
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

    /// awase 起動後に作られたスレッドの IME は「閉」で始まる（`focus/thread_scope.rs`）。
    /// 純粋な Imm32Unavailable では開閉を読めないため、この規則を根拠に「閉」を Low confidence の
    /// `HeuristicDefault` として記録する（`reset_stale_ime_on_for_imm_broken` の ON 版と対）。
    /// ユーザー意図は偽装せず、`desired_open` も書き換えない。明示操作・より強い観測が
    /// 後から届けばそちらが優先される。
    pub(crate) fn assume_closed_for_new_thread(
        &mut self,
        profile: ImePolicyProfile,
        tick_ms: TickMs,
    ) {
        if !self.belief.is_japanese_ime() {
            return;
        }
        tracing::info!(
            "new-thread entry: IME は閉で始まる（awase 起動後に作られたスレッド）→ \
             安全デフォルト OFF を Low confidence observation として記録"
        );
        let focus_epoch = self.shadow_model.observations.current_fence().epoch;
        self.dispatch_event(
            ImeEvent::ObserverReported(
                Observed::<evidence::HeuristicDefault>::at_startup(
                    profile,
                    false,
                    HwndId::NULL,
                    focus_epoch,
                )
                .into(),
            ),
            tick_ms,
        );
    }

    /// `belief.is_japanese_ime` を書く（`warrant_context` の材料）。
    ///
    /// 閉ループのハーネス（`tests/support/harness.rs`）からも呼ぶ。本番の呼び出し元は crate 内だけ。
    pub fn set_is_japanese_ime(&mut self, value: bool) {
        self.belief.is_japanese_ime = value;
    }

    /// ADR-223 段階 1: 打鍵の取り込み時に読んだ入力言語で `is_japanese_ime` を更新する。
    /// 不明(`None`)・同じ値なら何もしない。値が変わったら `true` を返す(呼び出し側が読み直しを 1 回だけ予約する)。
    pub(crate) fn observe_layout_language(&mut self, read: Option<bool>) -> bool {
        match read {
            Some(japanese) if japanese != self.belief.is_japanese_ime => {
                self.belief.is_japanese_ime = japanese;
                true
            }
            _ => false,
        }
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
                Observed::<evidence::ObserverPoll>::from_poll(&accepted, value).into(),
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
    /// `Command` ソースが conv 由来の内部同期（`EngineSync::DirectInput`（ADR-185で撤去済み））でも
    /// dispatch されるため。呼び出してよいのは以下の3箇所のみ:
    /// - `write_sync_key` / `write_physical_key`（物理 IME キーの shadow toggle。
    ///   `IntentWitness` が「注入されていない実キーイベント」を型で要求する）
    /// - `kp_stage_post_decision` の `SetOpenOrigin::ExplicitUserAction` 分岐
    ///   （IME ON/OFF コンボ、`applied=true` のときのみ）
    ///
    /// 例外: 閉ループのハーネス（`tests/support/harness.rs`）は `pub` なこのメソッドを直接呼ぶ
    /// （自前で作ったハブにだけ。本番のハブには crate の外から届かない。
    /// `tests/architecture_guard.rs::production_hub_is_unreachable_from_outside_the_crate`）。
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
    ///
    /// 閉ループのハーネス（`tests/support/harness.rs`）からも呼ぶ。本番の呼び出し元は crate 内だけ。
    pub fn record_explicit_intent(
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

    /// `value` は [`super::observation_store::FocusProbeOpenStatus::Read`] からしか
    /// 得られない型（ADR-106 決定2）。belief 由来の `bool`（`effective_open()` 等）を
    /// 観測として書き込むコードは型検査で落ちる。
    pub(crate) fn write_focus_probe(
        &mut self,
        value: super::observation_store::ObservedOpenValue,
        tick_ms: TickMs,
        accepted: crate::state::probe_admission::AcceptedObservation,
    ) {
        // confidence は `Observed<FocusProbe>` 側で Low 固定
        // （`hwndFocus`——フォーカス中コントロールであり、真の top-level ウィンドウ
        // ではない（PR 109 コードレビュー指摘1 Step1、BUG-91）——の IMC を読むため
        // Qt/GJI 等では child hwnd と異なる場合がある。High confidence の
        // ImmCrossProbe が後から上書きする）。
        //
        // hwnd は `from_probe` 内部で `accepted.hwnd()`（ticket が spawn 時に捕まえ、
        // admission で現在値と照合済みの `hwndFocus`）を使う（ADR-106 決定3）。
        // 以前は `HwndId::NULL` 固定だったため、`ObservationStore::derive_filtered`
        // が hwnd も照合するようになると全ての FocusProbe 観測が常に棄却されて
        // しまっていた。
        self.dispatch_event(
            ImeEvent::ObserverReported(
                Observed::<evidence::FocusProbe>::from_probe(&accepted, value.get()).into(),
            ),
            tick_ms,
        );
    }

    /// ImmCross 非同期プローブ結果を記録する（High confidence）。
    ///
    /// `read_ime_state_full_async` が child hwnd の IMM32 状態を読んだ後に呼ぶ。
    /// High confidence のため `derive_any()` で即採用される。
    /// `accepted` は `ImmLikeTicket::admit()` が返した `AcceptedObservation`
    /// （epoch/hwnd 照合済み、ADR-106 決定3）。hwnd は `from_cross_probe` 内部で
    /// `accepted.hwnd()`（`hwndFocus`。真の top-level ウィンドウではない——
    /// PR 109 コードレビュー指摘1 Step1、BUG-91）を使う——`write_focus_probe` と
    /// 同じ理由（`HwndId::NULL` 固定だと `derive_filtered` の hwnd 照合で常に
    /// 棄却されてしまう）。
    pub(crate) fn write_imm_cross_probe(
        &mut self,
        value: bool,
        tick_ms: TickMs,
        accepted: crate::state::probe_admission::AcceptedObservation,
    ) {
        self.dispatch_event(
            ImeEvent::ObserverReported(
                Observed::<evidence::ImmCrossProbe>::from_cross_probe(&accepted, value).into(),
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
        tracing::debug!("[conv-open-inference] reason={reason:?} open={open}");
        let focus_epoch = self.shadow_model.observations.current_fence().epoch;
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
    /// 現在フォーカス中のプロセスが `config.app_overrides.disable_apps`
    /// にマッチし、awase が丸ごと無効化されているか（BUG-78 対策）。
    /// `runtime/focus_tracking.rs` がフォーカス変更のたびに更新する。
    pub app_disabled: bool,
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
            app_disabled: false,
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
    /// Ctrl+key bypass 直後 latch。
    ///
    /// Ctrl+非修飾キーが PassThrough として素通りした後、次の非修飾 non-Ctrl キー 1 つを
    /// 同じ前景スコープ内でだけ NICOLA エンジンをスキップして直接 passthrough させる。
    /// tmux prefix (Ctrl+J) → コマンドキー (n/p) のように、
    /// prefix 直後のコマンドキーが NICOLA に横取りされる問題を防ぐ。
    pub post_bypass: ScopedOneShot<crate::state::foreground_scope::ForegroundScope, PostBypassArm>,
    /// IME 同期キー直後のキー保留バッファ（旧 `ime_gate`）。
    pub sync_key_gate: SyncKeyGate,
    /// 左右Shift単独タップによる「IME-ON 半角英数」持続トグルの全状態
    /// （旧 `left_shift_tap_candidate`/`right_shift_tap_candidate`/
    /// `shift_conv_guard_pending`/`half_width_alnum_toggle_active` の4
    /// フィールドと、旧 `Runtime::half_width_alnum_toggle_policy` を統合）。
    ///
    /// `HalfWidthAlnumState` のフィールドは private。読み書きは
    /// `state/half_width_alnum.rs` のメソッド経由に限定する
    /// （`tests/architecture_guard.rs` が生フィールド名の本番出現数を
    /// 0 に固定する）。
    pub half_width_alnum: crate::state::half_width_alnum::HalfWidthAlnumState,
    /// `kp_stage_idle_conv_check` の conv 読み取り（offload 済み、`SendMessageTimeoutW`
    /// ベース）が in-flight かどうか。spawn 時の `hook::current_tick_ms()` を持つ
    /// （BUG-34 横展開レビュー指摘: 単なる bool だと、完了時に `with_app` が
    /// 再入で `None` を返した場合にフラグが永久に立ちっぱなしになり、以後
    /// idle-conv-check がプロセスの寿命いっぱい発火しなくなる。単なる bool 化
    /// 解除だけでなく、経過時間で自動的に「放棄された」とみなして再武装できる
    /// ようにするため `Option<u64>`（spawn 時刻）にする）。
    ///
    /// GJI が本当にハングしている間に断続的なタイピングが続くと、idle ゲートを
    /// 通過するたびに新しい offload 呼び出しが積み上がりワーカースレッドが増え続ける。
    /// 1 件 in-flight の間は新規 spawn をスキップし、完了時（epoch 棄却時も含む）に
    /// `with_app` 内で必ず `None` へ戻す。それに加えて、spawn からの経過時間が
    /// [`IDLE_CONV_CHECK_IN_FLIGHT_STALE_MS`] を超えていれば in-flight とはみなさず
    /// 新規 spawn を許可する（完了取りこぼし時の自己回復）。
    pub idle_conv_check_in_flight_since_ms: Option<u64>,
    /// BUG-173追補: `shadow_action` を持つ IME 系キーの最初の KeyDown の配送結果（scan_code, Suppress したか）。
    /// KeyUp を Down に揃えるためのラッチ（`key_effect_runtime::keyup_follows_keydown`）。
    pub shadow_key_down_disposition: Vec<(awase::types::ScanCode, bool)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PostBypassArm {
    /// ログ・診断専用。判定には使わない。
    pub armed_focus_epoch: u64,
}

/// [`GateStore::idle_conv_check_in_flight_since_ms`] の自己回復しきい値。
///
/// BUG-34 の実測（WezTerm, ~5741ms、docs/known-bugs.md）が示す
/// `HungAppTimeout` の既定値（~5000ms）+ マージンで、正当な in-flight 読み取り
/// （offload 先のワーカースレッドが実際にハング境界までブロックしている場合）を
/// 誤って「放棄された」と判定しないようにする。
pub(crate) const IDLE_CONV_CHECK_IN_FLIGHT_STALE_MS: u64 = 8_000;

impl GateStore {
    pub(crate) fn new() -> Self {
        Self {
            last_hook_activity_ms: 0,
            post_bypass: ScopedOneShot::new(),
            sync_key_gate: SyncKeyGate::new(),
            half_width_alnum: crate::state::half_width_alnum::HalfWidthAlnumState::default(),
            idle_conv_check_in_flight_since_ms: None,
            shadow_key_down_disposition: Vec::new(),
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
    /// `[[keymap]]` の KeyUp 回収・自動リピート抑制用 latch（ADR-114 決定4）
    pub keymap_latch: crate::state::keymap_latch::KeymapLatch,
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

#[cfg(test)]
impl PlatformState {
    /// 時計を注入して初期化するテスト用の構築口（`new()` は実時計 `hook::current_tick_ms` を読む）。
    pub(crate) fn for_test(clock: super::hub_clock::HubClock) -> Self {
        Self {
            ime: ImeStateHub::with_clock(clock),
            focus: FocusStore::new(),
            gate: GateStore::new(),
            keymap: KeymapStore::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 仮想時計の基準 tick。実機（`GetTickCount64` = 起動からの経過）は通常 `IntentStore` の
    /// どの TTL よりも十分大きいので、最大の TTL の 2 倍を取って同じ位置関係にする
    /// （導出元は `tuning` の定数で、数値の直書きはしない）。
    const BASE_TICK: u64 = {
        let (on, off) = (
            crate::tuning::EXPLICIT_ON_INTENT_TTL_MS,
            crate::tuning::EXPLICIT_OFF_INTENT_TTL_MS,
        );
        (if on > off { on } else { off }) * 2
    };

    /// テスト共通の `PlatformState`: 仮想時計（進めない限り `BASE_TICK` で止まる）。
    fn ps_for_test() -> PlatformState {
        PlatformState::for_test(crate::state::hub_clock::HubClock::manual(BASE_TICK))
    }

    /// shadow_model を直接設定するヘルパ:
    /// `set_intent=Some(source)` なら UserImeSetIntent を dispatch し last_intent を設定する。
    /// `set_intent=None` なら desired_open のみ直接書き換え、last_intent は空のままにする
    /// (focus 変更後の carry-over シナリオを模擬)。
    fn ps_with_shadow(
        desired_open: bool,
        set_intent: Option<UserIntentSource>,
        is_japanese: bool,
    ) -> PlatformState {
        let mut ps = ps_for_test();
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

    /// `HubClock` が `Instant` の供給元になっている: 手動時計を進めた量だけ、`dispatch_event` が
    /// 付ける `EventTime::monotonic` が進む（壁時計を読んでいれば実測の数 µs しか進まない）。
    ///
    /// `monotonic` は journal に載らないので、reduce の結果に現れる Event で見る:
    /// `FocusChanged` の reducer は `input_barrier` の `started_at` に `envelope.time.monotonic` を入れる。
    #[test]
    fn manual_hub_clock_drives_event_monotonic() {
        let mut ps = ps_for_test();
        ps.ime.clock = crate::state::hub_clock::HubClock::manual(10_000);
        let started_at = |ps: &PlatformState| match ps.ime.model().input_barrier {
            Some(InputBarrier::FocusTransition { started_at, .. }) => started_at,
            ref other => panic!("FocusTransition の barrier が立っていない: {other:?}"),
        };
        let tick = ps.ime.clock.now_tick();
        dispatch_focus_changed(&mut ps, HwndId(1), 1, tick);
        let older = started_at(&ps);
        ps.ime.clock.advance_ms(500);
        let tick = ps.ime.clock.now_tick();
        dispatch_focus_changed(&mut ps, HwndId(2), 2, tick);
        let newer = started_at(&ps);
        assert_eq!(newer - older, std::time::Duration::from_millis(500));
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

    #[test]
    fn new_thread_assumption_makes_effective_open_false_without_changing_desired() {
        let mut ps = ps_with_shadow(true, None, true);

        ps.ime
            .assume_closed_for_new_thread(ImePolicyProfile::Imm32Unavailable, TickMs(100));

        assert!(!ps.ime.effective_open_at(TickMs(100)));
        assert!(
            ps.ime.model().desired_open(),
            "HeuristicDefault OFF は desired_open を書き換えない"
        );
        assert_eq!(
            ps.ime.explicit_intent(),
            None,
            "HeuristicDefault OFF は last_intent を作らない"
        );
    }

    #[test]
    fn new_thread_assumption_yields_to_last_intent() {
        let mut ps = ps_with_shadow(true, Some(UserIntentSource::Command), true);

        ps.ime
            .assume_closed_for_new_thread(ImePolicyProfile::Imm32Unavailable, TickMs(100));

        assert!(ps.ime.effective_open_at(TickMs(100)));
        assert_eq!(ps.ime.explicit_intent(), Some(true));
    }

    #[test]
    fn new_thread_assumption_yields_to_intent_store() {
        let mut ps = ps_for_test();
        ps.ime.belief.is_japanese_ime = true;
        dispatch_focus_changed(&mut ps, TARGET_HWND, 1, 0);
        dispatch_and_record_explicit_intent(&mut ps, true, 100);
        // last_intent と観測を消し、IntentStore だけを優先根拠として残す。
        dispatch_focus_changed(&mut ps, TARGET_HWND, 2, 200);
        assert_eq!(ps.ime.explicit_intent(), None);

        ps.ime
            .assume_closed_for_new_thread(ImePolicyProfile::Imm32Unavailable, TickMs(300));

        assert!(ps.ime.effective_open_at(TickMs(300)));
    }

    #[test]
    fn new_thread_assumption_does_nothing_for_non_japanese_ime() {
        let mut ps = ps_with_shadow(true, None, false);

        ps.ime
            .assume_closed_for_new_thread(ImePolicyProfile::Imm32Unavailable, TickMs(100));

        assert!(ps.ime.effective_open_at(TickMs(100)));
        assert!(ps.ime.model().desired_open());
        assert_eq!(ps.ime.explicit_intent(), None);
        assert!(
            ps.ime
                .shadow_model
                .observations
                .per_source
                .heuristic_default
                .is_none(),
            "日本語 IME でなければ HeuristicDefault を記録しない"
        );
    }

    // ── handle_engine_set_open: settle 中の明示操作は落とさない（ADR-213 P2d-2）──
    //
    // 2026-07-05 に「Alt+Tab 中間窓で Engine の自動遷移が未確定 belief から書く」対策として
    // 入れた focus_transition_was_pending フィルタは、自動遷移（ActivationSync）の撤去（P2c）後は
    // settle 中の明示操作（Ctrl+変換等）を黙って捨てるだけになり、実測（Chrome×GJI・MS-IME が
    // settle 約20ms後の書き込みを受け付ける）で撤去した。settle 中でも belief を書いて適用する。
    #[test]
    fn handle_engine_set_open_applies_even_while_focus_transition_settling() {
        let mut ps = ps_with_shadow(false, Some(UserIntentSource::SyncKey), true);
        // 将来に開始する barrier: settle_until が必ず now より先になり、settling が確実に true。
        let started_at = std::time::Instant::now() + std::time::Duration::from_secs(10);
        ps.ime
            .try_set_focus_transition_barrier(HwndId::NULL, started_at);
        assert!(
            ps.ime
                .is_focus_transition_settling(std::time::Instant::now()),
            "前提: settle 中"
        );
        let applied =
            ps.ime
                .handle_engine_set_open(true, false, ApplyGeneration::new(1).unwrap(), TickMs(0));
        assert!(applied, "settle 中の明示操作 SetOpen も適用される");
        assert!(
            ps.ime.model().desired_open(),
            "settle 中でも desired_open を書く（belief と実書き込みの非対称を作らない）"
        );
    }

    // settle 外でも通常通り適用される。
    #[test]
    fn handle_engine_set_open_applies_when_focus_transition_not_pending() {
        let mut ps = ps_with_shadow(false, Some(UserIntentSource::SyncKey), true);
        let applied =
            ps.ime
                .handle_engine_set_open(true, false, ApplyGeneration::new(1).unwrap(), TickMs(0));
        assert!(applied);
        assert!(ps.ime.model().desired_open());
    }

    // ── BUG-34 横展開 D-prep: record_ime_apply_result の UnsafeToToggle 処理 ────
    //
    // 以前は呼び出し元 (`runtime/mod.rs::on_ime_apply_complete`) が
    // `UnsafeToToggle` をここへ到達する前に早期 return しており、generation 付き
    // で立てた pending が一度も解放されず、以後の別 generation の完了が全て
    // stale 判定され続ける固着になっていた（round-2 premortem で発見）。

    /// `UnsafeToToggle` は pending を解放するが、`applied` はミラーリングしない
    /// (実際には何も送っていないため、どちらの状態か分からない)。
    #[test]
    fn unsafe_to_toggle_releases_pending_without_mirroring_applied() {
        let mut ps = ps_for_test();
        ps.ime.dispatch_event(
            ImeEvent::ImeApplyRequested {
                target: true,
                generation: ApplyGeneration::new(5).unwrap(),
                ctrl_held: false,
            },
            TickMs(0),
        );
        assert_eq!(
            ps.ime.model().pending_generation(),
            Some(ApplyGeneration::new(5).unwrap())
        );

        let accepted = ps.ime.record_ime_apply_result_in_scope(
            true,
            awase::platform::ImeOpenOutcome::UnsafeToToggle,
            Some(ApplyGeneration::new(5).unwrap()),
            100,
            test_foreground_scope(),
        );

        assert_eq!(
            accepted,
            ImeApplyAcceptance::NotSent,
            "UnsafeToToggle は composition 更新(on_ime_applied)を誘発しない"
        );
        assert!(
            ps.ime.model().pending_generation().is_none(),
            "UnsafeToToggle でも pending は解放される — 解放しないと以後の別 \
             generation の完了が全て stale 判定され続ける固着になる"
        );
        assert!(
            ps.ime.model().applied.applied_open().is_none(),
            "何を実際に適用したか不明なため applied はミラーリングしない"
        );
    }

    #[test]
    fn not_owned_releases_pending_without_mirroring_applied() {
        let mut ps = ps_for_test();
        ps.ime.dispatch_event(
            ImeEvent::ImeApplyRequested {
                target: true,
                generation: ApplyGeneration::new(5).unwrap(),
                ctrl_held: false,
            },
            TickMs(0),
        );

        let accepted = ps.ime.record_ime_apply_result_in_scope(
            true,
            awase::platform::ImeOpenOutcome::NotOwned,
            Some(ApplyGeneration::new(5).unwrap()),
            100,
            test_foreground_scope(),
        );

        assert_eq!(accepted, ImeApplyAcceptance::NotSent);
        assert!(ps.ime.model().pending_generation().is_none());
        assert!(
            ps.ime.model().applied.applied_open().is_none(),
            "InputRelay では送っていないため applied はミラーリングしない"
        );
    }

    /// generation が一致しない UnsafeToToggle 完了は、他の outcome と同様
    /// stale として無視され pending に触れない。
    #[test]
    fn unsafe_to_toggle_with_stale_generation_does_not_touch_pending() {
        let mut ps = ps_for_test();
        ps.ime.dispatch_event(
            ImeEvent::ImeApplyRequested {
                target: true,
                generation: ApplyGeneration::new(5).unwrap(),
                ctrl_held: false,
            },
            TickMs(0),
        );

        let accepted = ps.ime.record_ime_apply_result_in_scope(
            true,
            awase::platform::ImeOpenOutcome::UnsafeToToggle,
            Some(ApplyGeneration::new(4).unwrap()),
            100,
            test_foreground_scope(),
        );

        assert_eq!(accepted, ImeApplyAcceptance::NotSent);
        assert_eq!(
            ps.ime.model().pending_generation(),
            Some(ApplyGeneration::new(5).unwrap()),
            "generation 不一致の UnsafeToToggle は stale として無視され、現在の \
             pending を消費しない"
        );
    }

    // 既存の CtrlImeChord フィルタが、settle フィルタの有無によらず
    // 引き続き機能することを確認する回帰テスト。
    #[test]
    fn handle_engine_set_open_ctrl_chord_filter_still_works() {
        let mut ps = ps_with_shadow(true, Some(UserIntentSource::SyncKey), true);
        // 1 回目: IME OFF 要求 + Ctrl 押下中 → chord transaction 開始。
        let first =
            ps.ime
                .handle_engine_set_open(false, true, ApplyGeneration::new(1).unwrap(), TickMs(0));
        assert!(first, "chord を開始する最初の要求は適用される");
        assert!(ps.ime.is_ctrl_ime_chord_active());
        // 2 回目: chord transaction 中の二次 IME OFF 要求 → フィルタされる。
        let second =
            ps.ime
                .handle_engine_set_open(false, true, ApplyGeneration::new(2).unwrap(), TickMs(0));
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
        let mut ps = ps_for_test();
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
        let applied = ps.ime.handle_engine_set_open(
            false,
            false,
            ApplyGeneration::new(1).unwrap(),
            TickMs(9_999),
        );
        assert!(applied);
        assert_eq!(
            ps.ime.persistent_explicit_off_ms(),
            9_999,
            "デフォルトキーバインド経由の明示 IME OFF が \
             Imm32Unavailable cache-miss ガードから漏れないこと"
        );
    }

    // ── release_panic_reset_guard_on_positive_evidence（ADR-213 P2d-1）: ──
    // conv 観測由来の engine ON 同期は PanicReset ガードだけを外し、
    // last_intent/desired_open/IntentStore は書かない。

    #[test]
    fn release_panic_guard_never_sets_last_intent_or_desired_open() {
        let mut ps = ps_with_shadow(false, Some(UserIntentSource::PhysicalImeKey), true);
        ps.ime.release_panic_reset_guard_on_positive_evidence();
        assert_eq!(
            ps.ime.model().last_intent.as_ref().map(|i| i.target),
            Some(false),
            "PanicReset ガード解除はユーザーの明示的な OFF 意図 (last_intent) を上書きしない"
        );
        assert!(
            !ps.ime.model().desired_open(),
            "PanicReset ガード解除は desired_open を書き換えない"
        );
        assert!(
            !ps.ime.effective_open(),
            "explicit intent が残っているため effective_open() は false のまま"
        );
    }

    #[test]
    fn release_panic_guard_removes_only_panic_reset_reason() {
        let mut ps = ps_for_test();
        let guard = |reason| ForceGuard {
            reason,
            expires_at: None,
            generation: 0,
        };
        ps.ime
            .shadow_model
            .force_guards
            .add(guard(ForceOnReason::PanicReset));
        ps.ime
            .shadow_model
            .force_guards
            .add(guard(ForceOnReason::ProfilePolicy));
        ps.ime.release_panic_reset_guard_on_positive_evidence();
        assert_eq!(
            ps.ime.shadow_model.force_guards.active_reason(),
            Some(ForceOnReason::ProfilePolicy),
            "PanicReset だけが外れ、ProfilePolicy ガードは残る"
        );
    }

    #[test]
    fn release_panic_guard_does_not_record_intent_store_entry() {
        let mut ps = ps_for_test();
        dispatch_focus_changed(&mut ps, TARGET_HWND, 1, 0);
        ps.ime.release_panic_reset_guard_on_positive_evidence();
        dispatch_conv_open_inference(&mut ps, true, 100);
        assert_eq!(
            ps.ime.effective_open_at(TickMs(100)),
            ps.ime.model().effective_open(),
            "IntentStore に記録しないため、hub 版と生の ImeModel 版の effective_open() は一致し続ける"
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

    // BUG-173 追補3（D4）: conv 由来の open 推論は、明示意図（ユーザーの IME OFF）と食い違っても drift correction を
    // 発火させない（旧: BUG-19 再発対策として threshold=0 で即時に false を再送していた）。GJI×TsfNative では IME を
    // 閉じても conv の NATIVE が残り、この推測は `VK_IME_OFF` を何度送っても収束しなかった。
    #[test]
    fn check_drift_correction_ignores_conv_inference_even_when_explicit_off_intent_conflicts() {
        let mut ps = ps_with_shadow(false, Some(UserIntentSource::PhysicalImeKey), true);
        ps.ime
            .report_conv_open_inference(true, ConvSyncReason::NativeToggleShadowOff, TickMs(0));
        let now = std::time::Instant::now();
        let explicit_intent = ps.ime.explicit_intent();
        assert_eq!(
            ps.ime.check_drift_correction(now, explicit_intent),
            None,
            "conv 推論だけを根拠にした drift は、明示意図があっても補正を発火させない"
        );
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
                .checked_sub(std::time::Duration::from_millis(
                    crate::tuning::DRIFT_CORRECTION_THRESHOLD_MS + 50,
                ))
                .expect("test instant can be backdated"),
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

    // BUG-110 追補7（issue #189）: `HeuristicDefault`（観測ゼロの安全デフォルト）も
    // （当時の `ConvOpenInference` と全く同じ理由で）、明示意図が無い間は単独で drift
    // correction を発火させない。拡張前は、Word 等で明示 OFF → Chrome へ
    // フォーカス移動 → `reset_stale_ime_on_for_imm_broken` が `HeuristicDefault(true)`
    // を記録、という経路で `check_drift_correction` が
    // `Some(desired:false, observed:true)` を返し、（撤去済みの）`apply_force_on_for_imm_broken`
    // （`effective_open()` 経由で同じ `HeuristicDefault` を信頼して ON を送っていた）と
    // 反対方向に競合し、短時間の ON/OFF 往復を起こしていた。
    #[test]
    fn check_drift_correction_ignores_heuristic_default_alone_without_explicit_intent() {
        let mut ps = ps_for_test();
        ps.ime.belief.is_japanese_ime = true;
        // Word 相当のウィンドウで明示 OFF。
        dispatch_focus_changed(&mut ps, TARGET_HWND, 1, 0);
        ps.ime
            .write_sync_key(sync_key_witness(), false, TickMs(100));
        // Chrome 相当の別ウィンドウへフォーカス移動
        // （last_intent クリア、対象 hwnd 向けの IntentStore エントリも無い）。
        // 注（opus-adversarial-consult S3）: `dispatch_focus_changed` ヘルパは
        // `ImePolicyProfile::TsfNative` 固定で、下の
        // `reset_stale_ime_on_for_imm_broken` には別途 `Imm32Unavailable` を
        // 渡している——実際の Chrome 入場（`AppKind: TsfNative` かつ
        // Imm32Unavailable 扱い）を厳密に再現してはいないが、
        // `check_drift_correction` は `app_policy` を読まないため本テストの
        // 検証内容には影響しない。
        let other_hwnd = HwndId(0x5678);
        dispatch_focus_changed(&mut ps, other_hwnd, 2, 200);
        assert!(
            !ps.ime.effective_open_at(TickMs(200)),
            "生の desired_open() フォールバックにより false のまま"
        );

        ps.ime
            .reset_stale_ime_on_for_imm_broken(ImePolicyProfile::Imm32Unavailable, TickMs(300));

        // opus-adversarial-consult S1: `reset_stale_ime_on_for_imm_broken` には
        // 4つの早期 return があり、将来そのいずれかが誤って成立すると
        // `HeuristicDefault` が一切記録されなくなる。その場合
        // `most_recent_trusted()` が `None` を返し、`check_drift_correction` は
        // 新ガード（本テストが検証したい箇所）より手前の別の分岐で `None` に
        // なってしまい、テストは「間違った理由で」緑のままになる。観測が
        // 実際に記録されたことを積極的にアサートしてこれを防ぐ。
        let recorded = ps
            .ime
            .shadow_model
            .observations
            .per_source
            .heuristic_default
            .as_ref()
            .expect("reset_stale_ime_on_for_imm_broken が HeuristicDefault を記録しているはず");
        assert!(
            recorded.open,
            "HeuristicDefault の安全デフォルトは常に true"
        );
        assert!(
            !ps.ime.shadow_model.desired_open(),
            "desired_open は Word での明示 OFF のまま false（observed との食い違いが本題）"
        );

        // 明示意図なしでは閾値が DRIFT_CORRECTION_THRESHOLD_MS になる
        // （ConvOpenInference のテストと同様、実 sleep を避けるためバックデートする）。
        ps.ime.shadow_model.observations.drift = Some(ImeDrift {
            started_at: std::time::Instant::now()
                .checked_sub(std::time::Duration::from_millis(
                    crate::tuning::DRIFT_CORRECTION_THRESHOLD_MS + 50,
                ))
                .expect("test instant can be backdated"),
        });
        let now = std::time::Instant::now();
        let explicit_intent = ps.ime.explicit_intent();
        assert_eq!(explicit_intent, None);
        assert_eq!(
            ps.ime.check_drift_correction(now, explicit_intent),
            None,
            "明示意図なしでは HeuristicDefault 単独で補正を発火させない（issue #189）"
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

    /// 強い（High）open 観測を1件流し込む（IMM 直接読み取り相当）。テスト専用の再生口
    /// （`AnyObservation::restored_from_journal`）を使う。
    fn write_open_observation_high(ps: &mut PlatformState, open: bool, tick_ms: u64) {
        ps.ime.dispatch_event(
            ImeEvent::ObserverReported(evidence::AnyObservation::restored_from_journal(
                open,
                ObservationSource::ImmGetOpenStatus,
                TARGET_HWND,
                ObservationConfidence::High,
                1,
            )),
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
            was_down: false,
            press_id: None,
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
                is_ime_mode_key: false,
                layout_japanese: None,
            },
            modifier_key: None,
            modifier_snapshot: ModifierState::default(),
            left_thumb_down_snapshot: None,
            right_thumb_down_snapshot: None,
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

    impl PlatformState {
        /// テスト専用: `align_after_expired_mode_key_pass` の scope 指定版。
        fn align_after_expired_pass_for_test(
            &mut self,
            now_ms: u64,
            scope: crate::state::foreground_scope::ForegroundScope,
        ) -> bool {
            self.ime
                .align_after_expired_mode_key_pass_in_scope(now_ms, TickMs(now_ms), scope)
        }
    }

    fn arm_mode_key_pass_mark_for_test(
        ps: &mut PlatformState,
        scope: crate::state::foreground_scope::ForegroundScope,
        now_ms: u64,
    ) {
        ps.ime.mode_key_pass_mark.arm(scope, now_ms, true);
    }

    fn test_foreground_scope() -> crate::state::foreground_scope::ForegroundScope {
        crate::state::foreground_scope::ForegroundScope {
            pid: 42,
            hwnd: 0x1234,
        }
    }

    fn follow_fence() -> crate::state::probe_admission::AcceptedObservation {
        crate::state::probe_admission::AcceptedObservation::for_sync(
            crate::state::probe_admission::FocusFence {
                epoch: 1,
                hwnd: TARGET_HWND,
            },
        )
    }

    /// ADR-205 D2/D6: IntentStore に ON の意図がある状態で、監視窓の中の 1→0 を観測すると、意図を捨て desired を
    /// 実状態へ揃え、`effective_open()` が false になる。追随時刻も記録する（GJI I/O 推測の柵に使う）。
    #[test]
    fn follow_external_change_closes_belief_even_with_explicit_on_intent() {
        let mut ps = ps_for_test();
        dispatch_focus_changed(&mut ps, TARGET_HWND, 1, 0);
        dispatch_and_record_explicit_intent(&mut ps, true, 100);
        assert!(ps.ime.effective_open_at(TickMs(110)), "明示 ON 直後は true");
        // awase 自身の直近の書き込みの記録は ON（追随後の実状態 OFF と食い違う → 未確認へ落ちる、D6）。
        ps.ime
            .record_confirmed_in_scope(true, 90, test_foreground_scope());
        assert!(ps.ime.model().applied_state().applied_open().is_some());
        // arm 前の直近の読み（基準値になる）。窓が無いので追随しない。
        assert_eq!(
            ps.ime.follow_external_change_in_scope(
                Some(true),
                900,
                TickMs(900),
                follow_fence(),
                test_foreground_scope()
            ),
            None
        );
        ps.ime
            .arm_external_change_watch_in_scope(1000, test_foreground_scope());
        assert_eq!(
            ps.ime.follow_external_change_in_scope(
                Some(false),
                1032,
                TickMs(1032),
                follow_fence(),
                test_foreground_scope()
            ),
            Some(false)
        );
        assert!(
            !ps.ime.effective_open_at(TickMs(1040)),
            "IntentStore の ON の意図が残ると belief が ON のまま（ADR-205 R3-1）"
        );
        assert!(ps.ime.explicit_intent().is_none());
        assert_eq!(ps.ime.last_external_change_ms(), 1032);
        assert_eq!(
            ps.ime.model().applied_state().applied_open(),
            None,
            "追随経路だけが食い違う applied を未確認へ落とす（demote_applied=true、GjiDirect の already-matched を防ぐ）"
        );
    }

    /// 監視窓の外（arm していない・窓が切れた後）の読みの変化では追随しない。
    #[test]
    fn follow_external_change_ignores_reads_outside_the_window() {
        let mut ps = ps_for_test();
        dispatch_focus_changed(&mut ps, TARGET_HWND, 1, 0);
        dispatch_and_record_explicit_intent(&mut ps, true, 100);
        let _ = ps.ime.follow_external_change_in_scope(
            Some(true),
            900,
            TickMs(900),
            follow_fence(),
            test_foreground_scope(),
        );
        // arm していない
        assert_eq!(
            ps.ime.follow_external_change_in_scope(
                Some(false),
                1032,
                TickMs(1032),
                follow_fence(),
                test_foreground_scope()
            ),
            None
        );
        // 窓が切れた後（arm 前の直近の読みを 1 にしてから arm し、窓内の最初の読みも 1 = 変化なし）
        let _ = ps.ime.follow_external_change_in_scope(
            Some(true),
            1990,
            TickMs(1990),
            follow_fence(),
            test_foreground_scope(),
        );
        ps.ime
            .arm_external_change_watch_in_scope(2000, test_foreground_scope());
        let _ = ps.ime.follow_external_change_in_scope(
            Some(true),
            2010,
            TickMs(2010),
            follow_fence(),
            test_foreground_scope(),
        );
        assert_eq!(
            ps.ime.follow_external_change_in_scope(
                Some(false),
                2000 + crate::tuning::MODE_KEY_PASS_MARK_WINDOW_MS + 1,
                TickMs(2400),
                follow_fence(),
                test_foreground_scope()
            ),
            None
        );
        assert!(ps.ime.effective_open_at(TickMs(2410)), "追随していない");
        assert_eq!(ps.ime.last_external_change_ms(), 0);
    }

    /// 開く方向（0→1）も同じ規則で追随する（適用窓の GJI 限定は呼び出し側の `external_change_watch_applies`）。
    #[test]
    fn follow_external_change_opens_belief_on_zero_to_one() {
        let mut ps = ps_for_test();
        dispatch_focus_changed(&mut ps, TARGET_HWND, 1, 0);
        dispatch_and_record_explicit_intent(&mut ps, false, 100);
        let _ = ps.ime.follow_external_change_in_scope(
            Some(false),
            900,
            TickMs(900),
            follow_fence(),
            test_foreground_scope(),
        );
        ps.ime
            .arm_external_change_watch_in_scope(1000, test_foreground_scope());
        assert_eq!(
            ps.ime.follow_external_change_in_scope(
                Some(true),
                1040,
                TickMs(1040),
                follow_fence(),
                test_foreground_scope()
            ),
            Some(true)
        );
        assert!(ps.ime.effective_open_at(TickMs(1050)));
    }

    /// 中核の回帰テスト: 明示 OFF → 同一対象への FocusChanged（last_intent 消失）→
    /// 壊れた ConvOpenInference 観測、という実機再現手順で、生の
    /// `ImeModel::effective_open()` は true に反転してしまうが（退行の証拠として
    /// 明示的にアサートする）、`PlatformState::effective_open()`（IntentStore 込み）
    /// は false を維持することを確認する。
    #[test]
    fn effective_open_survives_focus_change_via_intent_store() {
        let mut ps = ps_for_test();
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

    /// 読めないアプリ（観測が来ない）で、起動直後の明示OFF意図が開閉の予測を無視させ続けない
    /// （CI blind: `intent-store` の上書きが約30秒続き、予測でopenにしてもEngineが動かなかった）。
    #[test]
    fn key_effect_open_prediction_replaces_stale_explicit_off_intent() {
        use crate::state::key_effect_predictor::{KeyTrack, PredictedEffect, Prediction, Stage};
        let mut ps = ps_for_test();
        dispatch_focus_changed(&mut ps, TARGET_HWND, 1, 0);
        dispatch_and_record_explicit_intent(&mut ps, false, 100);
        assert!(!ps.ime.effective_open_at(TickMs(110)), "明示OFF直後はfalse");

        let open = Prediction {
            effect: PredictedEffect {
                open: Some(true),
                mode: None,
            },
            track: KeyTrack {
                conv: None,
                stage: Stage::None,
            },
        };
        ps.ime.apply_key_effect_prediction(open, TickMs(120));
        assert!(
            ps.ime.effective_open_at(TickMs(130)),
            "開閉の予測は、同じ対象の古い明示OFF意図（IntentStore）を置き換えてEngineへ効く"
        );
    }

    /// 開閉を変えない予測（変換モードだけ等）は、明示意図を消さない。
    #[test]
    fn key_effect_prediction_without_open_keeps_explicit_intent() {
        use crate::state::key_effect_predictor::{KeyTrack, PredictedEffect, Prediction, Stage};
        let mut ps = ps_for_test();
        dispatch_focus_changed(&mut ps, TARGET_HWND, 1, 0);
        dispatch_and_record_explicit_intent(&mut ps, false, 100);
        let no_open = Prediction {
            effect: PredictedEffect {
                open: None,
                mode: None,
            },
            track: KeyTrack {
                conv: None,
                stage: Stage::Typing,
            },
        };
        ps.ime.apply_key_effect_prediction(no_open, TickMs(120));
        assert!(
            !ps.ime.effective_open_at(TickMs(130)),
            "開閉を予測しない打鍵では、明示OFF意図は残る"
        );
    }

    #[test]
    fn mode_key_pass_invalidation_without_mark_keeps_intents() {
        let mut ps = ps_for_test();
        dispatch_focus_changed(&mut ps, TARGET_HWND, 1, 0);
        dispatch_and_record_explicit_intent(&mut ps, false, 100);
        dispatch_conv_open_inference(&mut ps, true, 120);

        assert!(
            !ps.ime.invalidate_intents_if_mode_key_pass_live_in_scope(
                130,
                TickMs(130),
                test_foreground_scope(),
            ),
            "通過マークがなければ何もしない"
        );
        assert_eq!(ps.ime.explicit_intent(), Some(false));
        assert!(
            !ps.ime.effective_open_at(TickMs(130)),
            "IntentStore の OFF 意図も残る"
        );
    }

    #[test]
    fn mode_key_pass_invalidation_drops_intents_and_follows_observation_within_window() {
        let mut ps = ps_for_test();
        let scope = test_foreground_scope();
        dispatch_focus_changed(&mut ps, TARGET_HWND, 1, 0);
        dispatch_and_record_explicit_intent(&mut ps, false, 100);
        dispatch_conv_open_inference(&mut ps, true, 120);
        assert!(
            !ps.ime.effective_open_at(TickMs(120)),
            "破棄前は IntentStore が観測 true より優先される"
        );

        arm_mode_key_pass_mark_for_test(&mut ps, scope, 125);
        assert!(
            ps.ime
                .invalidate_intents_if_mode_key_pass_live_in_scope(140, TickMs(140), scope),
            "live な通過マークは観測成功後に一回だけ消費される"
        );
        assert_eq!(ps.ime.explicit_intent(), None);
        assert!(
            ps.ime.effective_open_at(TickMs(140)),
            "古い意図を捨てた後は観測 true に従う"
        );
        assert!(
            ps.ime
                .invalidate_intents_if_mode_key_pass_live_in_scope(141, TickMs(141), scope),
            "窓の間はマークを消費せず、観測のたびに再読み取りを続ける(最初の観測が古い状態を読んでも取りこぼさない)"
        );
        // 通過より後に記録された意図は、2回目以降の観測で捨てない(意図の破棄は通過ごとに1回)。
        dispatch_and_record_explicit_intent(&mut ps, false, 150);
        assert!(
            ps.ime
                .invalidate_intents_if_mode_key_pass_live_in_scope(160, TickMs(160), scope),
            "まだ有効"
        );
        assert_eq!(
            ps.ime.explicit_intent(),
            Some(false),
            "通過より後に記録された明示意図は残る"
        );
        assert!(
            !ps.ime.invalidate_intents_if_mode_key_pass_live_in_scope(
                125 + crate::tuning::MODE_KEY_PASS_MARK_WINDOW_MS,
                TickMs(500),
                scope,
            ),
            "窓が切れたら止まる"
        );
    }

    /// BUG-157 の回帰テスト: 起動直後のVK_IME_OFF（desired=false）の後、ユーザーのひらがなキーで
    /// 実IMEが開いた。通過マークの観測がこれを確認したら、`desired_open`は開へ揃い、drift correction は
    /// ユーザーの操作を閉じ直さない（修正前は desired=false のまま「観測 true ≠ desired false」で発火した）。
    #[test]
    fn mode_key_pass_observation_aligns_desired_so_drift_correction_does_not_revert_user_key() {
        let mut ps = ps_for_test();
        let scope = test_foreground_scope();
        dispatch_focus_changed(&mut ps, TARGET_HWND, 1, 0);
        // 起動直後の明示OFF（スパイク/ユーザー）。desired=false、意図あり。
        dispatch_and_record_explicit_intent(&mut ps, false, 100);
        // ユーザーの物理ひらがな（通過）→ 実IMEが開き、強い観測（High、ImmCross読み取り）が届く。
        arm_mode_key_pass_mark_for_test(&mut ps, scope, 125);
        write_open_observation_high(&mut ps, true, 130);
        assert!(ps
            .ime
            .invalidate_intents_if_mode_key_pass_live_in_scope(140, TickMs(140), scope));
        assert!(
            ps.ime.shadow_model.desired_open(),
            "通過したモードキーの結果（開）を desired として採る"
        );
        // 乖離の継続時間が閾値を超えていても（drift.started_at をバックデートして模す）、
        // 観測 == desired なので drift correction は発火しない（揃える前は desired=false ≠ 観測 true で発火した）。
        ps.ime.shadow_model.observations.drift = Some(ImeDrift {
            started_at: std::time::Instant::now()
                .checked_sub(std::time::Duration::from_millis(
                    crate::tuning::DRIFT_CORRECTION_THRESHOLD_MS + 50,
                ))
                .expect("test instant can be backdated"),
        });
        let now = std::time::Instant::now();
        assert!(
            ps.ime
                .check_drift_correction(now, ps.ime.explicit_intent())
                .is_none(),
            "揃った後は、観測 == desired なので drift correction は発火しない"
        );
    }

    /// 観測が無い窓（読めない窓）では、通過マークがあっても `desired_open` を書かない。
    #[test]
    fn mode_key_pass_without_observation_keeps_desired() {
        let mut ps = ps_for_test();
        let scope = test_foreground_scope();
        dispatch_focus_changed(&mut ps, TARGET_HWND, 1, 0);
        dispatch_and_record_explicit_intent(&mut ps, false, 100);
        arm_mode_key_pass_mark_for_test(&mut ps, scope, 125);
        assert!(ps
            .ime
            .invalidate_intents_if_mode_key_pass_live_in_scope(140, TickMs(140), scope));
        assert!(
            !ps.ime.shadow_model.desired_open(),
            "観測が無ければ desired は書かない（awaseが最後に書こうとした意図のまま）"
        );
    }

    /// awase が書いた意図（通過マーク無し）が実IMEに届かなかった場合は、従来どおり drift correction が
    /// 訂正する（BUG-157 の修正が、この必要な訂正を止めない）。
    #[test]
    fn drift_correction_still_fires_for_awase_write_without_mode_key_pass() {
        let mut ps = ps_for_test();
        dispatch_focus_changed(&mut ps, TARGET_HWND, 1, 0);
        dispatch_and_record_explicit_intent(&mut ps, true, 100);
        write_open_observation_high(&mut ps, false, 130);
        let now = std::time::Instant::now();
        let drift = ps.ime.check_drift_correction(now, ps.ime.explicit_intent());
        assert!(
            matches!(drift, Some(DriftCorrection { desired: true, observed: false, .. })),
            "通過マークが無ければ desired（awaseの意図）と観測の乖離は従来どおり補正される: {drift:?}"
        );
    }

    /// BUG-158: 通過マークの窓が切れても観測が一度も成功しなかったとき（読み取りが失敗し続ける環境）、
    /// 古い明示意図を捨てる（捨てないと `reschedule_ime_refresh` の早期returnでポーリングが止まったままになる）。
    #[test]
    fn mode_key_pass_expiry_drops_intents_when_no_observation_succeeded() {
        let mut ps = ps_for_test();
        let scope = test_foreground_scope();
        dispatch_focus_changed(&mut ps, TARGET_HWND, 1, 0);
        dispatch_and_record_explicit_intent(&mut ps, false, 100);
        arm_mode_key_pass_mark_for_test(&mut ps, scope, 125);
        let window = crate::tuning::MODE_KEY_PASS_MARK_WINDOW_MS;
        // 窓の間は捨てない（観測の成功を待つ）。
        assert!(!ps
            .ime
            .drop_intents_for_mode_key_pass_in_scope(140, TickMs(140), scope, true));
        assert_eq!(ps.ime.explicit_intent(), Some(false));
        // 窓が切れたら、観測が成功していなくても捨てる（一度だけ）。
        assert!(ps.ime.drop_intents_for_mode_key_pass_in_scope(
            125 + window,
            TickMs(125 + window),
            scope,
            true
        ));
        assert_eq!(
            ps.ime.explicit_intent(),
            None,
            "意図が残らないのでポーリングが再開する"
        );
        assert!(!ps.ime.drop_intents_for_mode_key_pass_in_scope(
            126 + window,
            TickMs(126 + window),
            scope,
            true
        ));
    }

    /// 観測の成功で既に捨てた通過マークは、窓の終了で再度捨てない（通過より後の明示意図を守る）。
    #[test]
    fn mode_key_pass_expiry_does_nothing_after_successful_invalidation() {
        let mut ps = ps_for_test();
        let scope = test_foreground_scope();
        dispatch_focus_changed(&mut ps, TARGET_HWND, 1, 0);
        dispatch_and_record_explicit_intent(&mut ps, false, 100);
        arm_mode_key_pass_mark_for_test(&mut ps, scope, 125);
        assert!(ps
            .ime
            .invalidate_intents_if_mode_key_pass_live_in_scope(140, TickMs(140), scope));
        dispatch_and_record_explicit_intent(&mut ps, true, 150);
        let window = crate::tuning::MODE_KEY_PASS_MARK_WINDOW_MS;
        assert!(!ps.ime.drop_intents_for_mode_key_pass_in_scope(
            125 + window,
            TickMs(125 + window),
            scope,
            true
        ));
        assert_eq!(
            ps.ime.explicit_intent(),
            Some(true),
            "通過より後の意図は残る"
        );
    }

    /// BUG-158追補2: 通過→窓の間の観測は全て時間切れ（観測なし）→窓切れ→最初の成功観測で `desired_open` を揃える。
    /// 揃えた後は通常の drift correction に戻る（2回目は揃えない）。
    #[test]
    fn align_after_expired_pass_aligns_once_on_first_successful_observation() {
        let mut ps = ps_for_test();
        let scope = test_foreground_scope();
        let window = crate::tuning::MODE_KEY_PASS_MARK_WINDOW_MS;
        dispatch_focus_changed(&mut ps, TARGET_HWND, 1, 0);
        dispatch_and_record_explicit_intent(&mut ps, false, 100);
        arm_mode_key_pass_mark_for_test(&mut ps, scope, 125);
        // 窓の間は観測が無い（全て時間切れ）。窓が切れて意図だけ捨てる（BUG-158）。
        assert!(ps.ime.drop_intents_for_mode_key_pass_in_scope(
            125 + window,
            TickMs(125 + window),
            scope,
            true
        ));
        assert!(
            !ps.ime.shadow_model.desired_open(),
            "観測が無いので desired は古いまま"
        );
        // 窓が切れた後の最初の成功観測（実IMEは開）。
        write_open_observation_high(&mut ps, true, 500);
        assert!(ps.align_after_expired_pass_for_test(600, scope));
        assert!(
            ps.ime.shadow_model.desired_open(),
            "最初の成功観測で desired を揃える"
        );
        // 通常の drift correction へ戻る: 2回目は揃えない。
        write_open_observation_high(&mut ps, false, 900);
        assert!(
            !ps.align_after_expired_pass_for_test(1000, scope),
            "通過につき1回だけ"
        );
        assert!(
            ps.ime.shadow_model.desired_open(),
            "2回目の観測では desired を動かさない"
        );
    }

    /// 通過より後に awase 自身が書いた（`record_optimistic`）場合は揃えない（実IMEを信用せず drift correction が訂正する）。
    #[test]
    fn align_after_expired_pass_skips_when_awase_wrote_after_pass() {
        let mut ps = ps_for_test();
        let scope = test_foreground_scope();
        let window = crate::tuning::MODE_KEY_PASS_MARK_WINDOW_MS;
        dispatch_focus_changed(&mut ps, TARGET_HWND, 1, 0);
        dispatch_and_record_explicit_intent(&mut ps, false, 100);
        arm_mode_key_pass_mark_for_test(&mut ps, scope, 125);
        ps.ime.note_awase_write_for_mode_key_pass_in_scope(scope);
        assert!(ps.ime.drop_intents_for_mode_key_pass_in_scope(
            125 + window,
            TickMs(125 + window),
            scope,
            true
        ));
        write_open_observation_high(&mut ps, true, 500);
        assert!(
            !ps.align_after_expired_pass_for_test(600, scope),
            "awase が書いた後は揃えない"
        );
    }

    /// 対象が違えば IntentStore は効かない（ADR-087 INV-24(b) の2段判定、BUG-26 非退行）。
    /// 別ウィンドウへの本物のフォーカス変更では、そのウィンドウ自身の観測に従うべき。
    #[test]
    fn effective_open_intent_store_does_not_leak_to_different_target() {
        let mut ps = ps_for_test();
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

    /// BUG-148/ADR-186 の回帰テスト: 起動時に既に前面にあるアプリでは、最初のプロセス
    /// 切替（`FocusChanged`）が来なくても `current_focus` が設定され、明示意図が
    /// `IntentStore` に記録される。
    ///
    /// 退行の証拠として「初期フォーカス未設定のままだと `record_explicit_intent` が
    /// 空振りし、壊れた観測1件で effective_open が true に反転する」ことも固定する
    /// （CI の E2E で委譲 SetOpen が全て Unwarranted になった機序）。
    #[test]
    fn initial_focus_hwnd_lets_explicit_intent_be_recorded_before_first_focus_change() {
        // `UserImeSetIntent` はモデルの `last_intent` を書くため、`effective_open` は IntentStore に記録されなくても
        // 直後は明示意図に固定される。IntentStore への記録の有無は、`FocusChanged`（`last_intent` をクリアする）の
        // 後に観測が入ったときの `effective_open` で区別する（`effective_open_survives_focus_change_via_intent_store` と同じ観点）。

        // 初期フォーカス未設定（BUG-148 の状態）: 意図が IntentStore に記録されず、FocusChanged で意図が消えると観測に従う。
        let mut ps = ps_for_test();
        assert_eq!(ps.ime.model().current_focus(), None);
        dispatch_and_record_explicit_intent(&mut ps, false, 100);
        dispatch_focus_changed(&mut ps, TARGET_HWND, 1, 200);
        dispatch_conv_open_inference(&mut ps, true, 300);
        assert!(
            ps.ime.effective_open_at(TickMs(300)),
            "退行の証拠: current_focus=None のときは record_explicit_intent が空振りし、\
             明示 OFF 意図が IntentStore に残らない"
        );

        // 起動時の初期フォーカスを確立した状態: 同じ操作で意図が IntentStore に保持される。
        let mut ps = ps_for_test();
        ps.ime.dispatch_event(
            ImeEvent::InitialFocusScopeEstablished {
                to: TARGET_HWND,
                profile: ImePolicyProfile::TsfNative,
                focus_epoch: 1,
            },
            TickMs(0),
        );
        assert_eq!(ps.ime.model().current_focus(), Some(TARGET_HWND));
        dispatch_and_record_explicit_intent(&mut ps, false, 100);
        dispatch_focus_changed(&mut ps, TARGET_HWND, 1, 200);
        dispatch_conv_open_inference(&mut ps, true, 300);
        assert!(
            !ps.ime.effective_open_at(TickMs(300)),
            "初期フォーカス確立後は明示 OFF 意図が IntentStore に記録され、\
             open_warrant Step 1 の根拠になる"
        );
    }

    /// OFF 意図の TTL 超過後は IntentStore もフォールバックする（無期限固着はしない）。
    #[test]
    fn effective_open_intent_store_entry_expires_after_ttl() {
        let mut ps = ps_for_test();
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
        let mut ps = ps_for_test();
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
    /// `EngineSync::DirectInput`（ADR-185で撤去済み）（conv 由来、`handle_engine_set_open` 経由で
    /// `UserImeSetIntent{Command}` を dispatch する）が壊れた conv 読み1件を
    /// FocusChanged を生き延びる偽の明示意図として永続化してしまっていた
    /// （pre-mortem #1 角度2）。
    #[test]
    fn dispatch_event_alone_does_not_record_intent_store_entry() {
        let mut ps = ps_for_test();
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
        let mut ps = ps_for_test();
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
        let mut ps = ps_for_test();
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
        let mut ps = ps_for_test();
        dispatch_focus_changed(&mut ps, TARGET_HWND, 1, 0);
        ps.ime
            .write_sync_key(sync_key_witness(), false, TickMs(500));
        ps.ime.apply_hwnd_cache_restore(
            Some(crate::focus::hwnd_cache::HwndImeSnapshot {
                ime_on: true,
                input_mode: InputModeState::ObservedRomaji,
                recorded_ms: 100,
                from_explicit_off_intent: false,
                hwnd: 0,
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
        let mut ps = ps_for_test();
        dispatch_focus_changed(&mut ps, TARGET_HWND, 1, 0);
        ps.ime
            .write_sync_key(sync_key_witness(), false, TickMs(100));
        ps.ime.apply_hwnd_cache_restore(
            Some(crate::focus::hwnd_cache::HwndImeSnapshot {
                ime_on: true,
                input_mode: InputModeState::ObservedRomaji,
                recorded_ms: 500,
                from_explicit_off_intent: false,
                hwnd: 0,
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
        let mut ps = ps_for_test();
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

    // ── ADR-158 TF1: ObservationSource の journal 記録経路 ─────────────────
    //
    // ADR-159段階0の当初計画は「JournalEntryに新しいバリアントを1〜2個追加する」
    // だったが、着手時に確認したところ`ImeEvent::InputModeObserved`が既に
    // `source: ObservationSource`をフィールドとして持ち、`dispatch_event`が
    // 無条件で全ImeEventを`JournalEntry::ImeEvent`として記録している（単一の
    // 合流点、上記`journal.record`呼び出し参照）ため、11バリアントすべてが
    // 新しい機構なしで既にjournal化されていると判明した。このテストはその
    // 事実を固定する回帰テストであり、将来`dispatch_event`の記録経路が
    // 分岐・迂回された場合に検出する。

    /// `InputModeObserved`を`dispatch_event`した場合、`ObservationSource`の値が
    /// 欠落・置換されずにそのままjournalへ記録されることを確認する
    /// （11バリアントのうち代表的な3つで検証、新規JournalEntryバリアントは不要）。
    #[test]
    fn dispatch_event_journals_observation_source_without_new_journal_entry_variant() {
        for source in [
            ObservationSource::Tsf,
            ObservationSource::GjiIoInference,
            ObservationSource::HeuristicDefault,
        ] {
            let mut ps = ps_for_test();
            ps.ime.dispatch_event(
                ImeEvent::InputModeObserved {
                    mode: InputModeState::ObservedKana,
                    source,
                    confidence: ObservationConfidence::Medium,
                },
                TickMs(0),
            );
            let json = ps
                .ime
                .journal
                .to_json()
                .expect("journal to_json should succeed for a single recorded entry");
            assert!(
                json.contains("InputModeObserved"),
                "source={source:?}: journalにInputModeObservedエントリが記録されていない: {json}"
            );
            assert!(
                json.contains(&format!("{source:?}")),
                "source={source:?}: journalにObservationSourceの値が記録されていない: {json}"
            );
        }
    }

    /// W-c: `dispatch_event` が `record_at` の `EventTime`（`seq` と、呼び出し側が渡した `tick_ms`）を
    /// journal の `ImeEvent` にそのまま載せる配線の検査（組み立てた値を記録するだけの
    /// `journal::tests` では配線が見えない）。
    #[test]
    fn dispatch_event_journals_event_time_seq_and_tick_ms() {
        let mut ps = ps_for_test();
        // event_log だけを先に 3 つ進める（journal には載らない）。`ImeEventLog` と
        // `UnifiedJournal` の seq はどちらも 0 始まりなので、揃ったままだと
        // 「journal 自身の seq を event_seq に載せる」取り違えを検出できない。
        let now = ps.ime.clock.now_instant();
        for _ in 0..3 {
            ps.ime
                .event_log
                .record_at(&ImeEvent::PanicReset { target: true }, TickMs(1), now);
        }
        let seq0 = ps.ime.event_log.next_seq();
        assert_eq!(seq0, 3);
        ps.ime
            .dispatch_event(ImeEvent::PanicReset { target: true }, TickMs(111));
        ps.ime
            .dispatch_event(ImeEvent::PanicReset { target: false }, TickMs(222));

        let rows: Vec<serde_json::Value> =
            serde_json::from_str(&ps.ime.journal.to_json().unwrap()).unwrap();
        let ime_events: Vec<&serde_json::Value> = rows
            .iter()
            .map(|r| &r["entry"])
            .filter(|e| e["type"].as_str() == Some("ImeEvent"))
            .collect();
        assert_eq!(
            ime_events.len(),
            2,
            "ImeEvent の記録が 2 件でない: {rows:?}"
        );
        assert_eq!(ime_events[0]["event_seq"].as_u64(), Some(seq0));
        assert_eq!(ime_events[0]["tick_ms"].as_u64(), Some(111));
        assert_eq!(ime_events[1]["event_seq"].as_u64(), Some(seq0 + 1));
        assert_eq!(ime_events[1]["tick_ms"].as_u64(), Some(222));
        // dispatch 1 回につき record_at がちょうど 1 回（配線ではなく採番回数の確認）。
        assert_eq!(ps.ime.event_log.next_seq(), seq0 + 2);
    }
}
