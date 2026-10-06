//! 進行中の IME actuation 試行の実行時状態（ADR-080）。
//!
//! このモジュールは `Actuation` 構造体（`target`/`policy`/`attempts`/`sent_at` を持ち、
//! `Runtime` が非公開フィールド `active_actuation` として所有する actuation 試行そのもの）
//! を保持する。純データの `FeedbackPolicy`/`Resolution` は state 層
//! (`crate::state::ime_actuation`) 側にあり、こちらは生存期間を持つ実行時状態専用。
//!
//! 破棄・再構築の条件（ADR-080「状態の永続化先」節）:
//! 1. `desired_open` が前回の `Actuation.target` と異なる値に変わった（`resolve_actuation`、`state/drift_plan.rs`）。
//! 2. `FocusChanged`（`runtime/ime_refresh.rs::ir_notify_focus_changed`）。
//! 3. `Resolution::Confirmed` 確定、または `Blind` が `GaveUp` した後に新しい観測
//!    （＝外部で状況が動いた証拠）を検知した（呼び出し元が `discard_actuation` を呼ぶ）。
//!    なお `GaveUp` は即座には破棄しない。`gave_up_at` を刻んで parked にし、以後の
//!    tick で新しい観測が来るのを待つ（ADR-080「有限 `Blind` からの復旧条件」）。

use super::Runtime;
use crate::state::drift_plan::ActuationSnapshot;
use crate::state::event_origin::EventOrigin;
use crate::state::ime_actuation::FeedbackPolicy;

/// 進行中の actuation 試行そのもの（`Copy` ではない、生存期間を持つ状態）。
///
/// observe tick（~20ms）ごとに使い回し、破棄条件（モジュール doc 参照）に
/// 該当したときのみ破棄・再構築する。tick ごとに無条件で作り直すと `max_attempts`
/// が実質無効化されるため禁止（ADR-080 不変条件4）。
pub(super) struct Actuation {
    pub(super) target: bool,
    pub(super) policy: FeedbackPolicy,
    pub(super) attempts: u32,
    /// この試行が最初に actuate した時刻。drift 判定が参照してよい観測の
    /// 下限（タイムスタンプ・フェンシング）としても使う。
    pub(super) sent_at: std::time::Instant,
    /// `Blind` がこの試行で最初に `max_attempts` 到達（`GiveUp`）した時刻。
    /// `None` の間はまだ諦めていない。`Some(t)` になった後、`t` 以降に新しい
    /// 観測が record されたら（値は問わない＝外部で状況が動いた証拠）試行を
    /// 破棄してやり直す（ADR-080「有限 `Blind` からの復旧条件」／task #15）。
    /// 破棄・再構築のたびに `None` に戻る。
    pub(super) gave_up_at: Option<std::time::Instant>,
    /// この actuation 試行の出所・世代（ADR-082 Phase 0.5）。
    /// `source` は常に `SelfActuated{strategy}`（`policy` から導出）。`epoch` は
    /// `attempts` と歩調を合わせて単調増加し（`advance_epoch`）、journal の
    /// `JournalEntry::ImeActuation` に「何回目・どの世代の訂正か」として記録される。
    /// 新規構築（target 変化）のたびに `Generation::INITIAL` から振り直す。
    pub(super) origin: EventOrigin,
}

impl Actuation {
    /// 純粋な判断（`state/drift_plan.rs`）へ渡す読み取り用の写し。
    pub(super) const fn snapshot(&self) -> ActuationSnapshot {
        ActuationSnapshot {
            target: self.target,
            policy: self.policy,
            attempts: self.attempts,
            sent_at: self.sent_at,
            gave_up_at: self.gave_up_at,
            origin: self.origin,
        }
    }

    /// 実送信して `attempts` を1つ進めるのに合わせ、`origin.epoch` も次の世代へ
    /// 進める。両者を1メソッドで動かし、`attempts` と `epoch` の歩調がずれない
    /// ことを構造的に保証する（呼び出し元が別々に更新して片方を忘れる事故を防ぐ）。
    pub(super) fn advance_epoch(&mut self) {
        self.attempts += 1;
        self.origin.epoch = self.origin.epoch.next();
    }
}

impl Runtime {
    /// `resolve_actuation`（`state/drift_plan.rs`）が新規と決めた試行を据える。
    ///
    /// 目標値が変わった（破棄条件1）ときは、前の試行を置き換えて `attempts` を 0 に戻す。
    /// 同じ `target` の試行は再利用され（ADR-080 不変条件4。この関数は呼ばれない）、再利用時は
    /// 方針も引き継がれる——呼び出し元は同じ `target` の間、常に同じ方針を渡す前提で設計すること。
    /// 判断（再利用か新規か）は純粋関数側、ここは据えるだけ。
    pub(super) fn install_actuation(&mut self, snapshot: &ActuationSnapshot) {
        self.active_actuation = Some(Actuation {
            target: snapshot.target,
            policy: snapshot.policy,
            attempts: snapshot.attempts,
            sent_at: snapshot.sent_at,
            gave_up_at: snapshot.gave_up_at,
            origin: snapshot.origin,
        });
    }

    /// 進行中の actuation を破棄する。破棄条件2（FocusChanged）・3
    /// （`Resolution` 確定）で使う。次の observe tick で必要なら
    /// `resolve_actuation` が新規と決め、`install_actuation` が据える。
    ///
    /// `force_open_pending`（ADR-086 Phase 3）はここでは触らない。
    /// フォーカス変更時の武装/解除は `ir_post_focus_change_snapshot` に
    /// 一元化されており、本関数の呼び出し元には `Resolution` 確定（drift
    /// correction の収束、フォーカス変更を伴わない）も含まれるため、ここで
    /// 一緒にクリアすると武装済みの force-ON を無関係なイベントで
    /// 取りこぼすことになる。
    pub(super) fn discard_actuation(&mut self) {
        self.active_actuation = None;
    }
}
