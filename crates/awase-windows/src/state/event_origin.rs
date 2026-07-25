//! 出所 (`EventSource`) ・世代 (`Generation`) を横断的に表現する最小型 (ADR-082 第一歩)。
//!
//! `docs/adr/082-journal-structured-replay-and-event-origin.md` が指摘する通り、
//! 「これは誰が起こしたイベントか」「これはどの世代の要求に対する応答か」という
//! 2つの概念は、これまで対象ごとに個別実装されてきた:
//!
//! - 出所: `RawKeyEvent::injected: bool`（`src/types.rs`、BUG-14）は物理/注入の2値のみ、
//!   `InputModeApplyStrategy`（`state/ime_event.rs`）は awase 自身の能動的訂正の理由付け。
//!   両者は別の型で、共通の「出所」として横断的に扱えない。
//! - 世代: `WarmEpoch`（ADR-069）、`cold_seq`（BUG-39）、`Actuation.attempts`（ADR-080）が
//!   それぞれ独立に「単調増加するカウンタで stale な応答を弾く」仕組みを再実装してきた。
//!
//! `EventOrigin { source: EventSource, epoch: Generation }` はこの2概念を1箇所に
//! 統合するための型。
//!
//! # スコープ（重要）
//!
//! ADR-082「第一歩」1. の時点では、この実装は型を定義するのみで既存コードへの
//! 配線を一切行わなかった（`RawKeyEvent::injected` / `InputModeApplyStrategy` /
//! `WarmEpoch` / `cold_seq` はいずれも今も無変更）。
//!
//! # Phase 0.5 での配線（2026-07-25）
//!
//! ADR-082 Phase 0.5 で `EventOrigin` を最初の1経路に配線した:
//! `runtime::ime_actuation::Actuation` が `origin: EventOrigin` を持ち、drift
//! correction の actuation 試行ごとに `source = SelfActuated{strategy}`・`epoch`
//! を積む（`runtime/ime_refresh.rs::ir_apply_drift_correction`）。試行1回分は
//! `journal.rs::JournalEntry::ImeActuation` として構造化記録される。`strategy` の
//! 導出と `EventOrigin` の構築は `state::ime_actuation::actuation_origin()`（純粋
//! 関数、Linux でユニットテスト可能）に集約している。`WarmEpoch` / `cold_seq` /
//! `RawKeyEvent::injected` を `EventOrigin` へ寄せるのは引き続き将来スコープ。

// ── Generation ───────────────────────────────────────────────────────────────

/// 単調増加する世代カウンタ。
///
/// `WarmEpoch`（`tsf/probe.rs`、ADR-069）・`cold_seq`（BUG-39）・
/// `Actuation.attempts`（`runtime/ime_actuation.rs`、ADR-080）が個別に実装してきた
/// 「これは何回目 / どの世代の要求か」を型として統合するための横断型。
///
/// `0` から開始し、`next()` で単調増加のみを許可する（巻き戻し不可）。
/// `u64` を包むだけの newtype であり、蓄積・比較以外のロジックは持たない。
///
/// `serde` 導出は ADR-082 Phase 0.5: `journal.rs::JournalEntry::ImeActuation` の
/// `origin.epoch` を journal に記録し（`Serialize`）、`DriftCorrectionFixture` の
/// tick が世代を保持してリプレイで往復できる（`Deserialize`）ようにするため。
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Default,
    serde::Serialize,
    serde::Deserialize,
)]
pub struct Generation(u64);

impl Generation {
    /// 初期世代（`0`）。
    pub const INITIAL: Self = Self(0);

    /// 生の `u64` から `Generation` を作る。
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// 内部の `u64` 値を取り出す。ログ出力・シリアライズ用途。
    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }

    /// 次の世代を返す（`self` 自体は変更しない、`Copy` なので
    /// 呼び出し元が明示的に代入する: `gen = gen.next();`）。
    ///
    /// `u64::MAX` からのオーバーフローは `wrapping_add` で折り返す。ADR-069 が
    /// `focus_epoch: u32` の wraparound リスクを教訓に `WarmEpoch` を再設計した
    /// 経緯があるため、`u64` の範囲では実運用上到達不可能な桁数であっても
    /// panic ではなく折り返しを選び、`next()` 自体を無条件に安全な操作にする。
    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }

    /// `self` が `other` より新しい世代（`self > other`）かを返す。
    /// stale な非同期応答を弾く判定（`FocusEpoch` 照合等）の置き換え候補。
    #[must_use]
    pub const fn is_newer_than(self, other: Self) -> bool {
        self.0 > other.0
    }
}

// ── EventSource ──────────────────────────────────────────────────────────────

/// イベントの出所。
///
/// `RawKeyEvent::injected: bool`（2値、BUG-14: 外部注入をユーザー意図に昇格させない
/// ためのフラグ）と `InputModeApplyStrategy`（awase 自身の能動的訂正の理由付け、
/// `.claude/rules/ime-belief-architecture.md` 「禁止パターン2」参照）が個別に
/// 表現してきた「これは誰が起こしたイベントか」を1つの型に統合する。
///
/// `reason` / `strategy` はコンパイル時定数の識別子文字列（ログ・デバッグ用途）。
/// 将来的に既存コードへ配線する際は、`InputModeApplyStrategy` のような専用 enum
/// への置き換えを検討すること（この最小実装では、まだどの呼び出し元にも配線
/// しないため、既存の専用 enum を re-export するのではなく汎用の文字列に留める）。
///
/// `serde::Serialize` のみ導出する（`Deserialize` はしない）。`reason`/`strategy` が
/// `&'static str` のため、任意入力から借用を復元する `Deserialize` は型として表現
/// できない。journal は書き出し専用（`Serialize`）でありこれで足りる。リプレイ側
/// （`DriftCorrectionFixture`）は世代（`Generation`、Ser/De 両対応）だけを保存し、
/// `SelfActuated` の `strategy` は `actuation_strategy()`（`state::ime_actuation`）で
/// `policy` から一意に再構築するため、`EventSource` 自体の `Deserialize` は不要。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum EventSource {
    /// 実機ユーザーの物理キー操作（`LLKHF_INJECTED` が立っていない）。
    Physical,
    /// 他プロセス由来の `SendInput` 注入（MS-IME/CTF・タッチキーボード・他ツール等）。
    /// `RawKeyEvent::injected == true` に対応する。ユーザー意図には昇格させない
    /// （BUG-14）。
    Injected { reason: &'static str },
    /// awase 自身が能動的に発行した actuation・訂正。
    /// `InputModeApplyStrategy` の各 variant（`ImmBrokenCorrection` 等）や
    /// `FeedbackPolicy::Blind` による drift correction 送信が該当する。
    SelfActuated { strategy: &'static str },
}

impl EventSource {
    /// 物理操作でも awase 自身の能動的操作でもない、外部注入かどうか。
    #[must_use]
    pub const fn is_injected(self) -> bool {
        matches!(self, Self::Injected { .. })
    }

    /// ユーザーの物理操作かどうか。
    #[must_use]
    pub const fn is_physical(self) -> bool {
        matches!(self, Self::Physical)
    }
}

// ── EventOrigin ──────────────────────────────────────────────────────────────

/// 出所と世代の組。ADR-082 が「journal に記録される全ての『非同期に届く確認・
/// 観測・完了通知』系エントリは `EventOrigin` を必須フィールドとして持つ」と
/// 決定した、その必須フィールドの型そのもの。
///
/// ADR-082 Phase 0.5 で `journal.rs::JournalEntry::ImeActuation` の `origin`
/// フィールドとして配線された（journal への書き出し用に `Serialize` を導出）。
/// `EventSource` が `&'static str` を持つため `Deserialize` は導出しない
/// （`EventSource` のコメント参照）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct EventOrigin {
    pub source: EventSource,
    pub epoch: Generation,
}

impl EventOrigin {
    #[must_use]
    pub const fn new(source: EventSource, epoch: Generation) -> Self {
        Self { source, epoch }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Generation ──────────────────────────────────────────────────────────

    #[test]
    fn generation_starts_at_zero() {
        assert_eq!(Generation::INITIAL.value(), 0);
        assert_eq!(Generation::default().value(), 0);
    }

    #[test]
    fn generation_next_increments_by_one() {
        let g0 = Generation::INITIAL;
        let g1 = g0.next();
        let g2 = g1.next();
        assert_eq!(g0.value(), 0);
        assert_eq!(g1.value(), 1);
        assert_eq!(g2.value(), 2);
    }

    #[test]
    fn generation_next_does_not_mutate_self() {
        // Copy 型なので `next()` は self を変更しない。呼び出し元が明示的に
        // 再代入する規約であることをテストで固定する。
        let g0 = Generation::INITIAL;
        let _ = g0.next();
        assert_eq!(g0.value(), 0, "next() は self を変更しないはず (Copy)");
    }

    #[test]
    fn generation_wraps_on_overflow_instead_of_panicking() {
        let max = Generation::new(u64::MAX);
        assert_eq!(max.next().value(), 0);
    }

    #[test]
    fn generation_is_newer_than() {
        let g0 = Generation::INITIAL;
        let g1 = g0.next();
        assert!(g1.is_newer_than(g0));
        assert!(!g0.is_newer_than(g1));
        assert!(!g0.is_newer_than(g0), "同じ世代は newer ではない");
    }

    #[test]
    fn generation_ordering_matches_value() {
        let g0 = Generation::new(0);
        let g5 = Generation::new(5);
        let g10 = Generation::new(10);
        assert!(g0 < g5);
        assert!(g5 < g10);
        assert_eq!(g5, Generation::new(5));
    }

    // ── EventSource ─────────────────────────────────────────────────────────

    #[test]
    fn physical_is_not_injected() {
        assert!(EventSource::Physical.is_physical());
        assert!(!EventSource::Physical.is_injected());
    }

    #[test]
    fn injected_is_injected_and_not_physical() {
        let src = EventSource::Injected {
            reason: "ms_ime_ctf",
        };
        assert!(src.is_injected());
        assert!(!src.is_physical());
    }

    #[test]
    fn self_actuated_is_neither_physical_nor_injected() {
        let src = EventSource::SelfActuated {
            strategy: "drift_correction_blind",
        };
        assert!(!src.is_physical());
        assert!(!src.is_injected());
    }

    #[test]
    fn event_source_equality_considers_payload() {
        let a = EventSource::Injected { reason: "a" };
        let b = EventSource::Injected { reason: "b" };
        assert_ne!(a, b, "reason が異なれば別イベントとして区別できる");
        assert_eq!(a, EventSource::Injected { reason: "a" });
    }

    // ── EventOrigin ─────────────────────────────────────────────────────────

    #[test]
    fn event_origin_pairs_source_and_epoch() {
        let origin = EventOrigin::new(EventSource::Physical, Generation::new(3));
        assert_eq!(origin.source, EventSource::Physical);
        assert_eq!(origin.epoch, Generation::new(3));
    }

    #[test]
    fn event_origin_is_copy() {
        let origin = EventOrigin::new(EventSource::Physical, Generation::INITIAL);
        let copy = origin;
        // Copy であることの確認: 両方とも引き続き使える。
        assert_eq!(origin, copy);
    }
}
