//! フォーカス判定結果のキャッシュ

use std::collections::HashMap;
use std::time::Instant;

use awase::types::FocusKind;

/// 判定結果のソース（TTL と優先順位を決定する）
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DetectionSource {
    /// Phase 1-2 同期判定（TTL: 5分、優先度: 最低）
    Automatic = 0,
    /// Phase 3 UIA 非同期判定（TTL: 5分）
    UiaAsync = 1,
    /// ユーザー手動オーバーライド（TTL: 24時間、優先度: 最高）
    UserOverride = 2,
}

impl DetectionSource {
    /// ソースに応じた TTL（秒）
    pub const fn ttl_secs(self) -> u64 {
        match self {
            Self::Automatic | Self::UiaAsync => 300, // 5分
            Self::UserOverride => 86400,             // 24時間
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

impl FocusCache {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// キャッシュを検索する。未登録または期限切れなら `None` を返す。
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
