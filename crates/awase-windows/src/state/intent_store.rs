//! IntentStore — 対象（`HwndId`）ごとの明示 IME 意図を保持する（ADR-087 §2.3 P15 /
//! §4 INV-24 / §5 Phase 1'）。
//!
//! ## 位置づけ
//!
//! 既存の `ImeModel.last_intent`（単一グローバルな `Option<RecordedIntent>`、
//! `ime_model.rs`）とは**別**のデータ構造。`last_intent` は `FocusChanged` の
//! たびに無条件でクリアされ、`effective_open()`（engine の内部挙動決定）が
//! 参照する。`IntentStore` は複数の対象を同時に保持でき、
//! `issue_open_warrant()`（`open_warrant.rs`、actuation の根拠）の Step 1 が
//! 参照する。
//!
//! この二重化は ADR-087 §7 round3 S4 で「意図的な一時状態」として記録されている
//! ——実際の runtime への配線（`last_intent` との統合）は ADR Phase 3 のスコープで
//! あり、本モジュール単体では行わない。
//!
//! ## 対象一致・TTL の設計（ADR-087 §4 INV-24 (a)(b)）
//!
//! - **対象一致は2段判定**: 同一 `HwndId` のみ一致、それ以外は不一致。
//!   round2 で提案した「同一プロセス + 同一 tsf-native 性」という3段目
//!   （tier②）は round3 で削除された（前提が誤りだった上に、共有ホスト
//!   プロセスで無関係なアプリの意図を混同する実害があった）。
//! - **TTL は ON/OFF で非対称だが、両方とも有界**: `IntentStore` の
//!   既定推測（`open_warrant.rs` の Step 4）は観測ゼロのとき ON 方向にのみ
//!   バイアスを持つ。そのため ON 意図の失効は Step 4 と同じ結論になり
//!   実害が薄いが、OFF 意図の失効は Step 4 が正反対の結論を出す。ON 意図には
//!   `tuning::EXPLICIT_ON_INTENT_TTL_MS`、OFF 意図には意図的により長い
//!   `tuning::EXPLICIT_OFF_INTENT_TTL_MS` を課す——**round3 時点では
//!   「OFF は無期限」としていたが、round4 の Opus レビューで「対象ごとに
//!   永続する本ストアで無期限は、フォーカス単位で有界だった旧
//!   `last_intent` と違い、drift correction が永久に再同期できない固着
//!   （HWND 再利用の混入も含む）を作る」と指摘され、有界に訂正した
//!   （§7 round4 M-A、`focus/hwnd_cache.rs::HwndImeCache` の
//!   `HWND_CACHE_MAX_AGE_MS` と同じ設計判断）。
//! - **期限切れエントリの掃除**: `record()` のたびに期限切れエントリを
//!   まとめて削除する（`HwndImeCache::save()` と同じパターン）。これにより
//!   `remove()`/`clear()` が Phase 3 で一度も呼ばれなくても、ストアが
//!   無限に肥大化したり、HWND 再利用で無関係な新規ウィンドウへ古い意図が
//!   適用されたりしない。

use super::ime_event::{HwndId, UserIntentSource};
use super::TickMs;
use std::collections::HashMap;

/// 対象（`HwndId`）ごとに記録された明示 IME 意図。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordedTargetIntent {
    pub target: HwndId,
    pub open: bool,
    pub source: UserIntentSource,
    pub recorded_at_ms: TickMs,
}

/// [`IntentStore::resolve_effective_open`] の判定内訳。
///
/// `intent` が `Some` かつ `value != shadow_open` のときだけ「実際に
/// 上書きした」ことになる（`ImeStateHub::effective_open()` の INFO ログは
/// この遷移でのみ出す）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntentOverride {
    /// 最終的な belief 値。
    pub value: bool,
    /// 上書きに使ったエントリ。`None` なら shadow をそのまま採用した。
    pub intent: Option<RecordedTargetIntent>,
}

/// [`IntentStore::invalidate_for_cache_restore`] の判定結果（BUG-51 追補 v3 修正2a）。
///
/// 呼び出し元（`ImeStateHub::apply_hwnd_cache_restore`）は `Kept` のときだけ
/// INFO ログを出す。エントリの削除自体はこのメソッドの中で完結する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheRestoreVerdict {
    /// 有効な明示意図が無かった（未記録または TTL 超過）。掃除だけ行った。
    NoIntent,
    /// キャッシュのほうが新しい（同時刻を含む）ため明示意図を無効化した。
    Invalidated { intent_recorded_at_ms: u64 },
    /// 明示意図のほうが新しいためエントリを保持した（キャッシュに負けさせない）。
    Kept { intent_recorded_at_ms: u64 },
}

/// 対象ごとの明示意図を保持するストア。
#[derive(Debug, Default, Clone)]
pub struct IntentStore {
    entries: HashMap<HwndId, RecordedTargetIntent>,
}

/// エントリの TTL（`entry.open` によって ON/OFF いずれの定数を使うか決まる）。
const fn ttl_for(open: bool) -> u64 {
    if open {
        crate::tuning::EXPLICIT_ON_INTENT_TTL_MS
    } else {
        crate::tuning::EXPLICIT_OFF_INTENT_TTL_MS
    }
}

impl RecordedTargetIntent {
    /// `now` の時点でこのエントリが期限切れか。
    #[must_use]
    fn is_expired(&self, now: TickMs) -> bool {
        let elapsed = now.saturating_sub(self.recorded_at_ms.0);
        elapsed > ttl_for(self.open)
    }
}

impl IntentStore {
    /// 対象への明示意図を記録する。同一対象の既存エントリは無条件で置換する
    /// （append-only にはしない——ADR-087 §7 round3 シナリオ9「同一対象では
    /// 最新 intent が旧 intent を置換する」を満たす）。
    ///
    /// 呼び出しのたびに、他の対象の期限切れエントリもまとめて掃除する
    /// （`HwndImeCache::save()` と同じパターン、§7 round4 M-A）。
    pub fn record(&mut self, target: HwndId, open: bool, source: UserIntentSource, now: TickMs) {
        self.entries.retain(|_, e| !e.is_expired(now));
        self.entries.insert(
            target,
            RecordedTargetIntent {
                target,
                open,
                source,
                recorded_at_ms: now,
            },
        );
    }

    /// 対象の有効な意図を返す。TTL 超過なら `None`（ON/OFF で異なる TTL、
    /// `ttl_for()` 参照）。
    ///
    /// 対象一致は呼び出し元が `target: HwndId` を正しく渡すことで担保する
    /// （2段判定: 同一 `HwndId` のみ一致、それ以外は呼び出し元がそもそも
    /// 別のキーで `lookup` する）。
    #[must_use]
    pub fn lookup(&self, target: HwndId, now: TickMs) -> Option<&RecordedTargetIntent> {
        let entry = self.entries.get(&target)?;
        if entry.is_expired(now) {
            return None;
        }
        Some(entry)
    }

    /// belief（`ImeModel::effective_open()` の値）に明示意図の上書きを重ねる
    /// **判定本体**（BUG-51 追補 v3）。`ImeStateHub::effective_open()` は
    /// ログの重複排除を除きこのメソッドの結果をそのまま返す。
    ///
    /// `focus` が `None`（フォーカス未確定）か、その対象に有効なエントリが
    /// 無ければ `shadow_open` をそのまま返す。
    ///
    /// # なぜ判定を `platform_state.rs` から切り出すのか
    ///
    /// `state/platform_state.rs` は `#[cfg(windows)]` であり、その中の
    /// `mod tests`（`cfg(test)`）は Linux の `cargo test -p awase-windows` では
    /// **1 件もコンパイルされない**（BUG-51 追補 v3 の回帰テスト群がまさに
    /// それで、Windows クロスチェックでの型検査しか受けていなかった）。
    /// 判定本体をこの ungated モジュールへ置くことで、
    /// `tests/intent_store_effective_open.rs` が Linux CI で
    /// 「壊れた `ConvOpenInference` 1 件だけでは `effective_open()` が反転
    /// しない」ことを実際に走らせて固定できる。
    #[must_use]
    pub fn resolve_effective_open(
        &self,
        focus: Option<HwndId>,
        shadow_open: bool,
        now: TickMs,
    ) -> IntentOverride {
        focus.and_then(|target| self.lookup(target, now)).map_or(
            IntentOverride {
                value: shadow_open,
                intent: None,
            },
            |intent| IntentOverride {
                value: intent.open,
                intent: Some(*intent),
            },
        )
    }

    /// `HwndCacheRestored`（`ImeStateHub::apply_hwnd_cache_restore`）が、対象の
    /// 明示意図を無効化してよいかを判定し、必要なら実際に削除する**判定本体**
    /// （BUG-51 追補 v3 修正2a）。
    ///
    /// 無条件 `remove()` は「フォーカス滞在 100ms 未満（`MIN_FOCUS_DURATION_MS`）
    /// だと退場時の cache 保存自体がスキップされる」ケース（BUG-57 型のフォーカス
    /// 奪取）で、たった今の新しい明示意図より古いキャッシュを勝たせてしまう
    /// （pre-mortem #2）。記録時刻を比較し、キャッシュのほうが新しい（**同時刻を
    /// 含む** = 「最新の決定が勝つ」）場合のみ無効化する。
    ///
    /// `resolve_effective_open()` と同じ理由でこの ungated モジュールに置く——
    /// `state/platform_state.rs` は `#[cfg(windows)]` で、そこに書いた
    /// `mod tests` は Linux CI では 1 件もコンパイルされない（追補4）。
    pub fn invalidate_for_cache_restore(
        &mut self,
        target: HwndId,
        cache_recorded_ms: u64,
        now: TickMs,
    ) -> CacheRestoreVerdict {
        let intent_recorded_at_ms = self.lookup(target, now).map(|i| i.recorded_at_ms.0);
        match intent_recorded_at_ms {
            // 意図が無い/期限切れ → remove は無害な掃除（冪等）。
            None => {
                self.remove(target);
                CacheRestoreVerdict::NoIntent
            }
            Some(intent_ms) if cache_recorded_ms >= intent_ms => {
                self.remove(target);
                CacheRestoreVerdict::Invalidated {
                    intent_recorded_at_ms: intent_ms,
                }
            }
            Some(intent_ms) => CacheRestoreVerdict::Kept {
                intent_recorded_at_ms: intent_ms,
            },
        }
    }

    /// 対象のエントリを削除する。
    pub fn remove(&mut self, target: HwndId) {
        self.entries.remove(&target);
    }

    /// 全エントリを削除する。
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TARGET_A: HwndId = HwndId(0x1000);
    const TARGET_B: HwndId = HwndId(0x2000);

    #[test]
    fn lookup_returns_none_for_unknown_target() {
        let store = IntentStore::default();
        assert!(store.lookup(TARGET_A, TickMs(1_000)).is_none());
    }

    #[test]
    fn record_then_lookup_same_target_returns_intent() {
        let mut store = IntentStore::default();
        store.record(
            TARGET_A,
            false,
            UserIntentSource::PhysicalImeKey,
            TickMs(1_000),
        );
        let found = store.lookup(TARGET_A, TickMs(1_500)).unwrap();
        assert!(!found.open);
        assert_eq!(found.source, UserIntentSource::PhysicalImeKey);
    }

    #[test]
    fn different_targets_do_not_leak_into_each_other() {
        // ADR-087 §7 round3 M2/M3: 共有ホストプロセスで無関係な別アプリの
        // 意図を混同しないことの直接の pinned test。
        let mut store = IntentStore::default();
        store.record(TARGET_A, false, UserIntentSource::PhysicalImeKey, TickMs(0));
        store.record(TARGET_B, true, UserIntentSource::PhysicalImeKey, TickMs(0));
        assert!(!store.lookup(TARGET_A, TickMs(0)).unwrap().open);
        assert!(store.lookup(TARGET_B, TickMs(0)).unwrap().open);
    }

    #[test]
    fn recording_same_target_again_replaces_old_intent() {
        let mut store = IntentStore::default();
        store.record(TARGET_A, false, UserIntentSource::PhysicalImeKey, TickMs(0));
        store.record(
            TARGET_A,
            true,
            UserIntentSource::PhysicalImeKey,
            TickMs(100),
        );
        let found = store.lookup(TARGET_A, TickMs(100)).unwrap();
        assert!(found.open, "後から record した ON が古い OFF を置換する");
    }

    #[test]
    fn on_intent_expires_after_ttl() {
        let mut store = IntentStore::default();
        store.record(TARGET_A, true, UserIntentSource::PhysicalImeKey, TickMs(0));
        let ttl = crate::tuning::EXPLICIT_ON_INTENT_TTL_MS;
        assert!(
            store.lookup(TARGET_A, TickMs(ttl)).is_some(),
            "TTL ちょうどはまだ有効"
        );
        assert!(
            store.lookup(TARGET_A, TickMs(ttl + 1)).is_none(),
            "TTL 超過後は ON 意図が失効する"
        );
    }

    #[test]
    fn off_intent_outlives_on_intent_but_still_expires() {
        // ADR-087 §4 INV-24(a)（round4 訂正版）: OFF 意図は ON より長く保持
        // されるが、無期限ではない（§7 round4 M-A、HwndImeCache と同型の
        // 有界設計）。
        let mut store = IntentStore::default();
        store.record(TARGET_A, false, UserIntentSource::PhysicalImeKey, TickMs(0));
        let on_ttl = crate::tuning::EXPLICIT_ON_INTENT_TTL_MS;
        let off_ttl = crate::tuning::EXPLICIT_OFF_INTENT_TTL_MS;
        assert!(off_ttl > on_ttl, "OFF の TTL は ON より長い");
        assert!(
            store.lookup(TARGET_A, TickMs(on_ttl + 1)).is_some(),
            "ON の TTL を超えても OFF 意図はまだ有効"
        );
        assert!(
            store.lookup(TARGET_A, TickMs(off_ttl)).is_some(),
            "OFF の TTL ちょうどはまだ有効"
        );
        assert!(
            store.lookup(TARGET_A, TickMs(off_ttl + 1)).is_none(),
            "OFF の TTL を超えると失効する（無期限ではない）"
        );
    }

    #[test]
    fn record_sweeps_expired_entries_of_other_targets() {
        // §7 round4 M-A: record() のたびに他対象の期限切れエントリも掃除する
        // （HwndImeCache::save() と同じパターン）。
        let mut store = IntentStore::default();
        let on_ttl = crate::tuning::EXPLICIT_ON_INTENT_TTL_MS;
        store.record(TARGET_A, true, UserIntentSource::PhysicalImeKey, TickMs(0));
        assert_eq!(store.entries.len(), 1);
        // TARGET_A の ON 意図が失効した後に TARGET_B を record すると、
        // TARGET_A のエントリは掃除される。
        store.record(
            TARGET_B,
            true,
            UserIntentSource::PhysicalImeKey,
            TickMs(on_ttl + 1),
        );
        assert_eq!(
            store.entries.len(),
            1,
            "期限切れの TARGET_A エントリが record() 時に掃除される"
        );
        assert!(store.lookup(TARGET_A, TickMs(on_ttl + 1)).is_none());
        assert!(store.lookup(TARGET_B, TickMs(on_ttl + 1)).is_some());
    }

    #[test]
    fn remove_clears_single_target() {
        let mut store = IntentStore::default();
        store.record(TARGET_A, false, UserIntentSource::PhysicalImeKey, TickMs(0));
        store.record(TARGET_B, false, UserIntentSource::PhysicalImeKey, TickMs(0));
        store.remove(TARGET_A);
        assert!(store.lookup(TARGET_A, TickMs(0)).is_none());
        assert!(store.lookup(TARGET_B, TickMs(0)).is_some());
    }

    // ── invalidate_for_cache_restore（修正2a、追補4 で Linux 実行可能化）─────

    #[test]
    fn cache_restore_keeps_intent_newer_than_cache() {
        // BUG-57 型: フォーカス滞在 100ms 未満だと退場時の cache 保存が
        // スキップされ、たった今の明示意図より古いキャッシュが復元される。
        let mut store = IntentStore::default();
        store.record(
            TARGET_A,
            false,
            UserIntentSource::PhysicalImeKey,
            TickMs(500),
        );
        let verdict = store.invalidate_for_cache_restore(TARGET_A, 100, TickMs(600));
        assert_eq!(
            verdict,
            CacheRestoreVerdict::Kept {
                intent_recorded_at_ms: 500
            }
        );
        assert!(
            store.lookup(TARGET_A, TickMs(600)).is_some(),
            "キャッシュより新しい明示意図はエントリごと残る"
        );
    }

    #[test]
    fn cache_restore_invalidates_intent_older_than_cache() {
        let mut store = IntentStore::default();
        store.record(
            TARGET_A,
            false,
            UserIntentSource::PhysicalImeKey,
            TickMs(100),
        );
        let verdict = store.invalidate_for_cache_restore(TARGET_A, 500, TickMs(600));
        assert_eq!(
            verdict,
            CacheRestoreVerdict::Invalidated {
                intent_recorded_at_ms: 100
            }
        );
        assert!(store.lookup(TARGET_A, TickMs(600)).is_none());
    }

    #[test]
    fn cache_restore_invalidates_intent_recorded_at_the_same_ms() {
        // 同時刻はキャッシュ側の勝ち（v1 と同じ「最新の決定が勝つ」の境界）。
        let mut store = IntentStore::default();
        store.record(
            TARGET_A,
            false,
            UserIntentSource::PhysicalImeKey,
            TickMs(300),
        );
        assert_eq!(
            store.invalidate_for_cache_restore(TARGET_A, 300, TickMs(300)),
            CacheRestoreVerdict::Invalidated {
                intent_recorded_at_ms: 300
            }
        );
        assert!(store.lookup(TARGET_A, TickMs(300)).is_none());
    }

    #[test]
    fn cache_restore_sweeps_expired_intent_as_no_intent() {
        let mut store = IntentStore::default();
        store.record(TARGET_A, false, UserIntentSource::PhysicalImeKey, TickMs(0));
        let off_ttl = crate::tuning::EXPLICIT_OFF_INTENT_TTL_MS;
        // 期限切れなら「意図より新しいキャッシュ」判定に入らず掃除だけ行う。
        assert_eq!(
            store.invalidate_for_cache_restore(TARGET_A, 0, TickMs(off_ttl + 1)),
            CacheRestoreVerdict::NoIntent
        );
        assert!(store.lookup(TARGET_A, TickMs(off_ttl + 1)).is_none());
    }

    #[test]
    fn cache_restore_of_one_target_does_not_touch_another() {
        let mut store = IntentStore::default();
        store.record(TARGET_A, false, UserIntentSource::PhysicalImeKey, TickMs(0));
        store.record(TARGET_B, false, UserIntentSource::PhysicalImeKey, TickMs(0));
        let _ = store.invalidate_for_cache_restore(TARGET_A, 100, TickMs(100));
        assert!(store.lookup(TARGET_A, TickMs(100)).is_none());
        assert!(
            store.lookup(TARGET_B, TickMs(100)).is_some(),
            "別対象のエントリは cache restore で消えない"
        );
    }

    #[test]
    fn clear_removes_all_targets() {
        let mut store = IntentStore::default();
        store.record(TARGET_A, false, UserIntentSource::PhysicalImeKey, TickMs(0));
        store.record(TARGET_B, true, UserIntentSource::PhysicalImeKey, TickMs(0));
        store.clear();
        assert!(store.lookup(TARGET_A, TickMs(0)).is_none());
        assert!(store.lookup(TARGET_B, TickMs(0)).is_none());
    }
}
