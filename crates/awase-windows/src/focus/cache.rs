//! フォーカス判定結果のキャッシュ

use std::collections::HashMap;
use std::time::Instant;

use crate::focus::FocusKind;

/// 判定結果のソース（TTL と優先順位を決定する）
///
/// 旧 `UiaAsync = 1`（Phase 3 UIA 非同期判定の中間優先度）は 2026-07-06 の
/// 到達不能パス監査で撤去 — 唯一の生成元だった `handle_wm_focus_kind_update` が
/// BUG-12 でログのみ化されて以降、構築サイトゼロだった。UIA を再有効化する場合は
/// BUG-12 の記録どおり hwnd 粒度の別設計が必要で、その際に階層を再導入すること。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DetectionSource {
    /// Phase 1-2 同期判定（TTL: 5分、優先度: 低）
    Automatic = 0,
    /// ユーザー手動オーバーライド（TTL: 24時間、優先度: 最高）
    UserOverride = 2,
}

impl DetectionSource {
    /// ソースに応じた TTL（秒）
    #[must_use]
    pub const fn ttl_secs(self) -> u64 {
        match self {
            Self::Automatic => 300,      // 5分
            Self::UserOverride => 86400, // 24時間
        }
    }
}

#[derive(Debug)]
struct FocusCacheEntry {
    kind: FocusKind,
    source: DetectionSource,
    timestamp: Instant,
}

/// フォーカス判定結果のキャッシュ
///
/// `(process_id, class_name)` をキーとして判定結果を保持する。
/// 同じコントロールへの再フォーカス時に UIA 非同期判定を省略できる。
/// ソース別の TTL と優先順位により、高優先エントリは低優先で上書きされない。
#[derive(Debug)]
pub struct FocusCache {
    entries: HashMap<(u32, String), FocusCacheEntry>,
}

impl Default for FocusCache {
    fn default() -> Self {
        Self::new()
    }
}

impl FocusCache {
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// キャッシュを検索する。未登録または期限切れなら `None` を返す。
    #[must_use]
    pub fn get(&self, process_id: u32, class_name: &str) -> Option<FocusKind> {
        let key = (process_id, class_name.to_string());
        self.entries.get(&key).and_then(|entry| {
            (entry.timestamp.elapsed().as_secs() < entry.source.ttl_secs()).then_some(entry.kind)
        })
    }

    /// 判定結果をキャッシュに格納する。
    ///
    /// - `Undetermined` は格納しない。
    /// - 既存エントリより低優先のソースでは上書きしない（有効期限内の場合）。
    pub fn insert(
        &mut self,
        process_id: u32,
        class_name: String,
        kind: FocusKind,
        source: DetectionSource,
    ) {
        if kind == FocusKind::Undetermined {
            return;
        }
        let key = (process_id, class_name);
        // 既存エントリが高優先かつ有効期限内なら上書きしない
        if let Some(existing) = self.entries.get(&key) {
            if existing.source > source
                && existing.timestamp.elapsed().as_secs() < existing.source.ttl_secs()
            {
                return;
            }
        }
        self.entries.insert(
            key,
            FocusCacheEntry {
                kind,
                source,
                timestamp: Instant::now(),
            },
        );
        // エントリ数が上限を超えたら期限切れのみ削除
        if self.entries.len() > 1000 {
            self.entries
                .retain(|_, v| v.timestamp.elapsed().as_secs() < v.source.ttl_secs());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_returns_none_for_unregistered_key() {
        let cache = FocusCache::new();
        assert_eq!(cache.get(1, "SomeClass"), None);
    }

    #[test]
    fn insert_then_get_returns_the_cached_kind() {
        let mut cache = FocusCache::new();
        cache.insert(
            1,
            "SomeClass".to_string(),
            FocusKind::TextInput,
            DetectionSource::Automatic,
        );
        assert_eq!(cache.get(1, "SomeClass"), Some(FocusKind::TextInput));
    }

    /// `insert` は `Undetermined` を格納しない不変条件。`==`→`!=` の反転でこの
    /// ガードが壊れても検知できなかった。
    #[test]
    fn insert_does_not_store_undetermined() {
        let mut cache = FocusCache::new();
        cache.insert(
            1,
            "SomeClass".to_string(),
            FocusKind::Undetermined,
            DetectionSource::Automatic,
        );
        assert_eq!(cache.get(1, "SomeClass"), None);
    }

    /// BUG-11（UIA cache 毒で Edge 永久 NonText 化）の再発防止に直結する不変条件:
    /// 高優先度（`UserOverride`）で有効期限内のエントリは、低優先度（`Automatic`）
    /// では上書きされない。
    #[test]
    fn high_priority_entry_is_not_overwritten_by_low_priority_while_valid() {
        let mut cache = FocusCache::new();
        cache.insert(
            1,
            "SomeClass".to_string(),
            FocusKind::TextInput,
            DetectionSource::UserOverride,
        );
        cache.insert(
            1,
            "SomeClass".to_string(),
            FocusKind::NonText,
            DetectionSource::Automatic,
        );
        assert_eq!(
            cache.get(1, "SomeClass"),
            Some(FocusKind::TextInput),
            "UserOverride で格納した値が Automatic の上書きから保護されるべき"
        );
    }

    /// 同一優先度（`source` が既存と等しい、`>` を満たさない）は上書きされる
    /// （通常の再判定リフレッシュ）。優先度ガードが `>=` 等に壊れて過剰に
    /// 保護してしまう回帰の逆側を固定する。
    #[test]
    fn same_priority_entry_is_refreshed() {
        let mut cache = FocusCache::new();
        cache.insert(
            1,
            "SomeClass".to_string(),
            FocusKind::TextInput,
            DetectionSource::Automatic,
        );
        cache.insert(
            1,
            "SomeClass".to_string(),
            FocusKind::NonText,
            DetectionSource::Automatic,
        );
        assert_eq!(
            cache.get(1, "SomeClass"),
            Some(FocusKind::NonText),
            "同一優先度なら新しい判定で上書きされるべき"
        );
    }

    /// 低優先度で登録した後、より高優先度な判定が来れば上書きされる。
    #[test]
    fn low_priority_entry_is_overwritten_by_high_priority() {
        let mut cache = FocusCache::new();
        cache.insert(
            1,
            "SomeClass".to_string(),
            FocusKind::NonText,
            DetectionSource::Automatic,
        );
        cache.insert(
            1,
            "SomeClass".to_string(),
            FocusKind::TextInput,
            DetectionSource::UserOverride,
        );
        assert_eq!(cache.get(1, "SomeClass"), Some(FocusKind::TextInput));
    }
}
