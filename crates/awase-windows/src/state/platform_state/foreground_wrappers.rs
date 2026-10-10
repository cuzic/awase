//! `ImeStateHub` の「フォアグラウンドを読んで `_in_scope` 版へ渡す」薄い層（FCIS P2 の殻だったもの）。
//!
//! フォアグラウンド取得は `ImeStateHub` に注入された関数（`with_clock` の `foreground` 引数。
//! `HubClock` と同じ形の注入）で行うので、このファイルは OS に触れない（ADR-229 段階 B で、
//! crate を分けても inherent impl のまま核に置けるように、殻の `win32::foreground_scope()` 直読みをやめた）。
//! 各メソッドは `self.foreground_scope()` を1回読み、1つの `_in_scope` に委譲するだけ
//! （`tests/architecture_guard.rs` の `shell_methods_only_read_scope_once_and_delegate` が形を固定する）。
//!
//! メソッド名とシグネチャは分割前と同じ（`runtime/`・`app/` の呼び出し元は変えない）。
//! 記録系（`record_*`）の呼び出し元の固定ガード（`RECORDERS`）は、このファイルも走査する。

use super::{ApplyGeneration, ImeApplyAcceptance, ImeStateHub};
use crate::state::TickMs;

impl ImeStateHub {
    /// 無変換/変換の生キーを通過させたら呼ぶ（ADR-187）。現在のフォアグラウンドに対する一回マークを立てる。
    pub(crate) fn arm_mode_key_pass_mark(&mut self, now_ms: u64, readable: bool) {
        self.arm_mode_key_pass_mark_in_scope(now_ms, readable, self.foreground_scope());
    }

    /// 立てた時点で読める窓だった通過マークが、窓の終了を待っているとき、その残り時間(ms)。
    /// 通過の途中で窓が読めなくなった（降格した）場合に、窓の終了時に`expire_mode_key_pass_mark`を呼ぶための
    /// 起床時刻に使う（読めない窓の`reschedule_ime_refresh`は通過マークが有効な間は何も予約しないため）。
    pub(crate) fn mode_key_pass_expiry_wait_ms(&mut self, now_ms: u64) -> Option<u64> {
        self.mode_key_pass_expiry_wait_ms_in_scope(now_ms, self.foreground_scope())
    }

    /// 通過マークの窓が切れるまでの残り時間(ms)。マークが無い/フォアグラウンドが変わった/窓が切れていれば`None`。
    /// 観測が失敗した通過の後、読み直しを窓の終了時の1回に絞るために使う（BUG-158）。
    pub(crate) fn mode_key_pass_window_remaining_ms(&mut self, now_ms: u64) -> Option<u64> {
        self.mode_key_pass_window_remaining_ms_in_scope(now_ms, self.foreground_scope())
    }

    /// 通過マークが有効か（消費しない）。フォアグラウンドが変わっていれば`peek`が失効させる。
    /// typing-idleガードのバイパス判定用（`ir_decide_read_strategy`）。
    pub(crate) fn mode_key_pass_mark_live(&mut self, now_ms: u64) -> bool {
        self.mode_key_pass_mark_live_in_scope(now_ms, self.foreground_scope())
    }

    /// 通過マークの窓が切れても、観測が一度も成功しなかった（`invalidated`のまま）ときに、古い明示意図を捨てる。
    ///
    /// 通過マークは「ユーザーの物理モードキーが通った。結果は分からないので、古い意図を根拠にしない」
    /// という事実そのものである。意図の破棄を観測の成功だけに頼ると、読み取りが失敗し続ける環境
    /// （MS-IME本体の`ime_on=None`）で意図が残り、`reschedule_ime_refresh`の早期returnでポーリングが止まったまま
    /// 次のモードキーまで固まる（BUG-151 原因③の再発、BUG-158）。窓の終了で必ず捨て、ポーリングを再開させる。
    /// 既に観測の成功で捨てた（`invalidated`）/窓の間は何もしない。
    pub(crate) fn expire_mode_key_pass_mark(&mut self, now_ms: u64, tick_ms: TickMs) -> bool {
        self.expire_mode_key_pass_mark_in_scope(now_ms, tick_ms, self.foreground_scope())
    }

    /// 外部注入の IME キーを見たら呼ぶ（読めない窓のみ）。現在のフォアグラウンドに対する監視窓を開く／延ばす。
    pub(crate) fn arm_external_change_watch(&mut self, now_ms: u64) {
        self.arm_external_change_watch_in_scope(now_ms, self.foreground_scope());
    }

    /// 監視窓の残り時間(ms)。無い・切れた・フォアグラウンドが変わったなら`None`（`reschedule_ime_refresh`の読み直し予約用）。
    pub(crate) fn external_change_watch_remaining_ms(&mut self, now_ms: u64) -> Option<u64> {
        self.external_change_watch_remaining_ms_in_scope(now_ms, self.foreground_scope())
    }

    /// prefetch 済みの開閉の読み（`read`）を監視窓に照合し、窓の中で基準値から変わっていれば実状態へ追随する。
    /// 本体は核の `follow_external_change_in_scope`（`platform_state.rs`）を参照。
    pub(crate) fn follow_external_change(
        &mut self,
        read: Option<bool>,
        now_ms: u64,
        tick_ms: TickMs,
        accepted: crate::state::probe_admission::AcceptedObservation,
    ) -> Option<bool> {
        self.follow_external_change_in_scope(
            read,
            now_ms,
            tick_ms,
            accepted,
            self.foreground_scope(),
        )
    }

    /// 物理のモードキー通過／FSM 再送出を見たら呼ぶ（GJI／同定済み MS-IME 本体 × `Imm32Unavailable`、ADR-188・ADR-244）。現在のフォアグラウンドに
    /// 対する直接観測の窓を開く／延ばす。
    pub(crate) fn arm_direct_external_change_watch(&mut self, now_ms: u64) {
        self.arm_direct_external_change_watch_in_scope(now_ms, self.foreground_scope());
    }

    /// 直接観測の窓の中の prefetch 済みの読みを belief と照合し、食い違う軸へ追随する（ADR-188）。
    pub(crate) fn follow_direct_read(
        &mut self,
        read_open: Option<bool>,
        read_conv: Option<u32>,
        now_ms: u64,
        tick_ms: TickMs,
        accepted: crate::state::probe_admission::AcceptedObservation,
        kind: crate::state::ime_kind::ImeKindId,
    ) -> Option<crate::state::external_change_watch::DirectFollow> {
        self.follow_direct_read_in_scope(
            read_open,
            read_conv,
            now_ms,
            tick_ms,
            accepted,
            kind,
            self.foreground_scope(),
        )
    }

    /// 通過マークの窓が**切れた後**の最初の成功観測で、`desired_open`を観測へ揃える（BUG-158追補2）。
    /// 本体は核の `align_after_expired_mode_key_pass_in_scope` を参照。
    pub(crate) fn align_after_expired_mode_key_pass(
        &mut self,
        now_ms: u64,
        tick_ms: TickMs,
    ) -> bool {
        self.align_after_expired_mode_key_pass_in_scope(now_ms, tick_ms, self.foreground_scope())
    }

    /// 通過マークが有効なら、古い明示意図を捨てる。本体は核の
    /// `invalidate_intents_if_mode_key_pass_live_in_scope` を参照。
    pub(crate) fn invalidate_intents_if_mode_key_pass_live(
        &mut self,
        now_ms: u64,
        tick_ms: TickMs,
    ) -> bool {
        self.invalidate_intents_if_mode_key_pass_live_in_scope(
            now_ms,
            tick_ms,
            self.foreground_scope(),
        )
    }

    /// 非同期送信済み・未確認の actuation を記録する（`applied = Optimistic`）。
    /// 本体と INV-A97-1 の注記は核の `record_optimistic_in_scope` を参照。
    pub(crate) fn record_optimistic(&mut self, open: bool) {
        self.record_optimistic_in_scope(open, self.foreground_scope());
    }

    /// 完了が確認された actuation を記録する（`applied = Confirmed`）。
    /// 本体は核の `record_confirmed_in_scope` を参照。
    pub(crate) fn record_confirmed(&mut self, open: bool, at_ms: u64) {
        self.record_confirmed_in_scope(open, at_ms, self.foreground_scope());
    }

    /// IME apply 完了を記録する（D: generation 照合 dispatch）。
    /// 本体は核の `record_ime_apply_result_in_scope` を参照。
    pub(crate) fn record_ime_apply_result(
        &mut self,
        open: bool,
        outcome: awase::platform::ImeOpenOutcome,
        generation: Option<ApplyGeneration>,
        ts: u64,
    ) -> ImeApplyAcceptance {
        self.record_ime_apply_result_in_scope(
            open,
            outcome,
            generation,
            ts,
            self.foreground_scope(),
        )
    }
}
