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

struct FocusCacheEntry {
    kind: FocusKind,
    source: DetectionSource,
    timestamp: Instant,
    /// ウィンドウごとのエンジン ON/OFF 状態（None = 未設定、グローバルに従う）
    engine_enabled: Option<bool>,
}

/// フォーカス判定結果のキャッシュ
///
/// `(process_id, class_name)` をキーとして判定結果を保持する。
/// 同じコントロールへの再フォーカス時に UIA 非同期判定を省略できる。
/// ソース別の TTL と優先順位により、高優先エントリは低優先で上書きされない。
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
        // 既存エントリの engine_enabled を保持する
        let prev_engine = self.entries.get(&key).and_then(|e| e.engine_enabled);
        self.entries.insert(
            key,
            FocusCacheEntry {
                kind,
                source,
                timestamp: Instant::now(),
                engine_enabled: prev_engine,
            },
        );
        // エントリ数が上限を超えたら期限切れのみ削除
        if self.entries.len() > 1000 {
            self.entries
                .retain(|_, v| v.timestamp.elapsed().as_secs() < v.source.ttl_secs());
        }
    }

    /// ウィンドウごとのエンジン ON/OFF 状態を取得する。
    ///
    /// エントリが存在し有効期限内であれば `engine_enabled` を返す。
    pub fn get_engine_state(&self, process_id: u32, class_name: &str) -> Option<bool> {
        let key = (process_id, class_name.to_string());
        self.entries.get(&key).and_then(|entry| {
            if entry.timestamp.elapsed().as_secs() < entry.source.ttl_secs() {
                entry.engine_enabled
            } else {
                None
            }
        })
    }

    /// ウィンドウごとのエンジン ON/OFF 状態を記録する。
    ///
    /// 既存エントリがあれば `engine_enabled` を更新する。
    /// エントリがなければ `Automatic` / `Undetermined` で最小限のエントリを作成する。
    pub fn set_engine_state(&mut self, process_id: u32, class_name: String, enabled: bool) {
        let key = (process_id, class_name);
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.engine_enabled = Some(enabled);
        } else {
            self.entries.insert(
                key,
                FocusCacheEntry {
                    kind: FocusKind::Undetermined,
                    source: DetectionSource::Automatic,
                    timestamp: Instant::now(),
                    engine_enabled: Some(enabled),
                },
            );
        }
    }
}
