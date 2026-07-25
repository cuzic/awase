//! 進行中の IME actuation 試行の実行時状態（ADR-080）。
//!
//! このモジュールは `Actuation` 構造体（`target`/`policy`/`attempts`/`sent_at` を持ち、
//! `Runtime` が非公開フィールド `active_actuation` として所有する actuation 試行そのもの）
//! を保持する。純データの `FeedbackPolicy`/`Resolution` は state 層
//! (`crate::state::ime_actuation`) 側にあり、こちらは生存期間を持つ実行時状態専用。
//!
//! 破棄・再構築の条件（ADR-080「状態の永続化先」節）:
//! 1. `desired_open` が前回の `Actuation.target` と異なる値に変わった（`actuation_for`）。
//! 2. `FocusChanged`（`runtime/ime_refresh.rs::ir_notify_focus_changed`）。
//! 3. `Resolution::Confirmed` 確定、または `Blind` が `GaveUp` した後に新しい観測
//!    （＝外部で状況が動いた証拠）を検知した（呼び出し元が `discard_actuation` を呼ぶ）。
//!    なお `GaveUp` は即座には破棄しない。`gave_up_at` を刻んで parked にし、以後の
//!    tick で新しい観測が来るのを待つ（ADR-080「有限 `Blind` からの復旧条件」）。

use super::Runtime;
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
}

impl Runtime {
    /// 目標値 `target` に対応する進行中の `Actuation` を返す。既存の
    /// `active_actuation` の `target` が異なる場合（破棄条件1）は破棄して
    /// 新規構築し、`attempts` を 0 にリセットする。同じ `target` なら
    /// 既存の試行を再利用する（ADR-080 不変条件4）。
    ///
    /// 再利用時、引数 `policy` は無視される（既存試行が構築時に持った
    /// `policy` がそのまま使われ続ける）。破棄条件は `target` の変化のみで
    /// `policy` の変化は対象外のため、同じ `target` に対して呼び出しごとに
    /// 異なる `policy` を渡しても反映されない。呼び出し元は同じ `target` の間
    /// 常に同じ `policy` を渡す前提で設計すること。
    pub(super) fn actuation_for(&mut self, target: bool, policy: FeedbackPolicy) -> &mut Actuation {
        let reuse = self
            .active_actuation
            .as_ref()
            .is_some_and(|a| a.target == target);
        if !reuse {
            self.active_actuation = Some(Actuation {
                target,
                policy,
                attempts: 0,
                sent_at: std::time::Instant::now(),
                gave_up_at: None,
            });
        }
        self.active_actuation
            .as_mut()
            .expect("active_actuation was set immediately above")
    }

    /// 進行中の actuation を破棄する。破棄条件2（FocusChanged）・3
    /// （`Resolution` 確定）で使う。次の observe tick で必要なら
    /// `actuation_for` が新規構築する。
    pub(super) fn discard_actuation(&mut self) {
        self.active_actuation = None;
    }
}
