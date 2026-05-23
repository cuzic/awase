//! IME 状態の観測値コレクション（Phase 2: 観測と判断の分離）。
//!
//! ## 設計方針
//!
//! 各更新ソースは `ImeObservations` の専用フィールドに観測値を記録するだけで、
//! 「どの値を採用するか」の判断は行わない。採用ロジックはすべて `resolve_and_clear()`
//! に集約されている。
//!
//! ## 優先度（高→低）
//!
//! 1. `sync_key` — config 由来の同期キー（ユーザー設定）
//! 2. `physical_key` — 物理 IME キー（ユーザーの直接操作）
//! 3. `set_open_request` — `ImeEffect::SetOpen`（Engine の判断）
//! 4. `focus_probe` — フォーカス変更直後の高速プローブ（`user_enabled=true` の場合 false は採用しない）
//! 5. `observer_poll` — IME observer ポーリング（バックグラウンド観測）
//!
//! ## 一撃性（one-shot）
//!
//! 優先度 1〜3（sync_key / physical_key / set_open_request）は `resolve_and_clear()` で
//! 採用後にクリアされる（`Option::take`）。これにより高優先度の意図が一度だけ適用され、
//! 次の呼び出しからは低優先度の観測値（observer_poll 等）が効くようになる。
//!
//! 優先度 4〜5（focus_probe / observer_poll）はクリアされず、新しい観測値で上書きされる。

use crate::ShadowSource;

/// 単一の IME 状態観測値（値 + タイムスタンプ）
#[derive(Debug, Clone, Copy)]
pub struct ImeObs {
    pub value: bool,
    pub ms: u64,
}

/// IME 状態の観測値コレクション
#[derive(Debug, Default)]
pub struct ImeObservations {
    /// config 由来の同期キー（最優先・一撃）
    pub(crate) sync_key: Option<ImeObs>,
    /// 物理 IME キー押下（一撃）
    pub(crate) physical_key: Option<ImeObs>,
    /// `ImeEffect::SetOpen` による強制設定（一撃）
    pub(crate) set_open_request: Option<ImeObs>,
    /// フォーカス変更直後の高速プローブ（永続）
    pub(crate) focus_probe: Option<ImeObs>,
    /// IME observer ポーリング（永続）
    pub(crate) observer_poll: Option<ImeObs>,
}

impl ImeObservations {
    /// 優先度・フィルタリングルールを適用して `ime_on` の最終値を決定する。
    ///
    /// 高優先度（sync_key / physical_key / set_open_request）は採用後にクリアされる。
    ///
    /// # パラメータ
    /// - `current`: 現在の `ime_on` 値（観測がない場合のフォールバック）
    /// - `user_enabled`: Engine が有効か（`focus_probe=false` の抑制に使用）
    /// - `is_japanese_ime`: 日本語 IME か（probe/poll の `false` フィルタ）
    ///
    /// # 戻り値
    /// `Some((value, source))` = `preconditions.ime_on` を更新すべき値とソース。
    /// `None` = 適用可能な観測なし（`ime_on` を変更しない）。
    pub fn resolve_and_clear(
        &mut self,
        current: bool,
        user_enabled: bool,
        is_japanese_ime: bool,
    ) -> Option<(bool, ShadowSource)> {
        // Priority 1: sync_key（一撃: 採用後クリア）
        if let Some(obs) = self.sync_key.take() {
            return Some((obs.value, ShadowSource::SyncKey));
        }

        // Priority 2: physical_key（一撃: 採用後クリア）
        if let Some(obs) = self.physical_key.take() {
            return Some((obs.value, ShadowSource::PhysicalImeKey));
        }

        // Priority 3: set_open_request（一撃: 採用後クリア）
        if let Some(obs) = self.set_open_request.take() {
            return Some((obs.value, ShadowSource::SetOpenRequest));
        }

        // Priority 4/5: focus_probe と observer_poll はタイムスタンプで比較し、
        // より新しい方を採用する（「最後に書いた方が勝つ」現行動作を維持）。
        //
        // focus_probe: user_enabled=true のとき false は採用しない（一時オーバーレイ対策）。
        let fp_candidate = self.focus_probe.as_ref().and_then(|o| {
            let effective = o.value && is_japanese_ime;
            if effective || !user_enabled {
                Some((o.ms, effective, ShadowSource::FocusProbe))
            } else {
                None // user_enabled=true かつ false → スキップ
            }
        });
        let op_candidate = self.observer_poll.as_ref().map(|o| {
            (o.ms, o.value && is_japanese_ime, ShadowSource::ObserverPoll)
        });

        let winner = match (fp_candidate, op_candidate) {
            (Some(fp), Some(op)) => Some(if fp.0 >= op.0 { (fp.1, fp.2) } else { (op.1, op.2) }),
            (Some(fp), None) => Some((fp.1, fp.2)),
            (None, Some(op)) => Some((op.1, op.2)),
            (None, None) => None,
        };

        let _ = current;
        winner
    }

    /// フォーカス変更時にウィンドウ固有の観測値をクリアする。
    ///
    /// `focus_probe` と `observer_poll` はウィンドウ固有なのでクリアする。
    /// `physical_key` / `sync_key` / `set_open_request` は一撃なので
    /// 通常はすでにクリア済みだが、残っていればここでクリアする。
    pub fn clear_on_focus_change(&mut self) {
        self.physical_key = None;
        self.sync_key = None;
        self.set_open_request = None;
        self.focus_probe = None;
        self.observer_poll = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(value: bool, ms: u64) -> ImeObs {
        ImeObs { value, ms }
    }

    // 1. 観測値が何もなければ None を返す
    #[test]
    fn test_no_observations_returns_none() {
        let mut o = ImeObservations::default();
        assert!(o.resolve_and_clear(false, true, true).is_none());
    }

    // 2. 優先度: sync_key > physical_key > set_open_request
    #[test]
    fn test_priority_order_high() {
        let mut o = ImeObservations {
            sync_key: Some(obs(true, 10)),
            physical_key: Some(obs(false, 20)),
            set_open_request: Some(obs(false, 30)),
            ..Default::default()
        };
        let (val, src) = o.resolve_and_clear(false, true, true).unwrap();
        assert_eq!(val, true);
        assert_eq!(src, ShadowSource::SyncKey);
    }

    // 3. sync_key は一撃（採用後クリア）; 二度目は physical_key が出る
    #[test]
    fn test_sync_key_one_shot() {
        let mut o = ImeObservations {
            sync_key: Some(obs(true, 10)),
            physical_key: Some(obs(false, 20)),
            ..Default::default()
        };
        let (_, src1) = o.resolve_and_clear(false, true, true).unwrap();
        assert_eq!(src1, ShadowSource::SyncKey);
        // sync_key は消費済み → 次は physical_key
        let (val2, src2) = o.resolve_and_clear(false, true, true).unwrap();
        assert_eq!(val2, false);
        assert_eq!(src2, ShadowSource::PhysicalImeKey);
        // physical_key も消費済み → None
        assert!(o.resolve_and_clear(false, true, true).is_none());
    }

    // 4. focus_probe=false は user_enabled=true のとき採用されない
    #[test]
    fn test_focus_probe_false_suppressed_when_user_enabled() {
        let mut o = ImeObservations {
            focus_probe: Some(obs(false, 100)),
            ..Default::default()
        };
        // user_enabled=true → false は採用しない
        assert!(o.resolve_and_clear(false, true, true).is_none());
    }

    // 5. focus_probe=false は user_enabled=false のとき採用される
    #[test]
    fn test_focus_probe_false_accepted_when_user_disabled() {
        let mut o = ImeObservations {
            focus_probe: Some(obs(false, 100)),
            ..Default::default()
        };
        let (val, src) = o.resolve_and_clear(false, false, true).unwrap();
        assert_eq!(val, false);
        assert_eq!(src, ShadowSource::FocusProbe);
    }

    // 6. observer_poll=true でも is_japanese_ime=false なら effective=false
    #[test]
    fn test_observer_poll_filtered_by_japanese_ime() {
        let mut o = ImeObservations {
            observer_poll: Some(obs(true, 100)),
            ..Default::default()
        };
        let (val, src) = o.resolve_and_clear(false, true, false).unwrap();
        assert_eq!(val, false); // true && false → false
        assert_eq!(src, ShadowSource::ObserverPoll);
    }

    // 7. タイムスタンプ: 新しい方が勝つ（focus_probe > observer_poll）
    #[test]
    fn test_newer_focus_probe_beats_observer_poll() {
        let mut o = ImeObservations {
            focus_probe: Some(obs(true, 200)), // 新しい
            observer_poll: Some(obs(false, 100)),
            ..Default::default()
        };
        let (val, src) = o.resolve_and_clear(false, true, true).unwrap();
        assert_eq!(val, true);
        assert_eq!(src, ShadowSource::FocusProbe);
    }

    // 8. clear_on_focus_change はすべてのフィールドをクリアする
    #[test]
    fn test_clear_on_focus_change() {
        let mut o = ImeObservations {
            sync_key: Some(obs(true, 1)),
            physical_key: Some(obs(true, 2)),
            set_open_request: Some(obs(true, 3)),
            focus_probe: Some(obs(true, 4)),
            observer_poll: Some(obs(true, 5)),
        };
        o.clear_on_focus_change();
        assert!(o.resolve_and_clear(false, true, true).is_none());
    }
}
