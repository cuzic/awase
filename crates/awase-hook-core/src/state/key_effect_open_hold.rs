//! ADR-292 P: `KeyEffectPrediction`(モードキー直後、mozc-shm実測の裏取り前)が未照合の間、
//! Engineに入る前の物理キーを保持する状態機械(「第3のhold」、`focus_switch_hold`/
//! `resume_hold`と同じ形)。
//!
//! 背景([BUG-233](../../../../../docs/known-bugs/BUG-233.md)): モードキー予測だけで
//! `effective_open`をtrueに倒した直後、mozc-shmの実測が追いつく前にEngineがローマ字へ
//! 変換して送信すると、実際にはIMEが開いていない場合にリテラルとして出力されてしまう。
//! ローマ字はEngineがbelief=開を前提に変換した**結果**であり、予測が外れたと分かった後に
//! 元の物理キーへ戻す手段が無いため、変換**前**の物理キーを保持し、解放時のbeliefに従って
//! (`INPUT_DEFER.replay_later`で)Engineへ流し直す設計にする。
//!
//! 純粋: 時刻・前面窓のスコープは引数で受け、保持する打鍵 `T` は呼び出し側の型
//! (殻では `RawKeyEvent`)。解放の判断(開いた/閉で確定/期限/上限/読めなくなった等)は
//! 呼び出し側が行い、ここは「保持・解放・窓の照合」という形だけを持つ
//! (`decide_use`等の判断そのものはここに持たない——R6の行動選択・G3のbaseline計算は
//! 呼び出し側の責務)。

use crate::state::foreground_scope::ForegroundScope;
use crate::state::TickMs;

/// 保持の上限(打鍵数)。`FOCUS_SWITCH_HOLD_CAP`と同じ値(ADR-292 round3 S5)。
pub const KEY_EFFECT_OPEN_HOLD_CAP: usize = 8;

/// 解放の結果。保持の中身(`T`)を返す。
#[derive(Debug, PartialEq, Eq)]
pub enum Released<T> {
    /// 保持が無い、または `hold_id` が今の保持と違う(古い札の通知)。何もしない。
    Nothing,
    /// 前面窓が保持の開始時と同じ。順序どおり再生してよい
    /// (再生後にbeliefをどうするかは呼び出し側がどの理由で解放したかで決める)。
    Replay(Vec<T>),
    /// 前面窓が保持の開始時と違うので捨てる(別窓へ誤配送しない)。
    /// 黙って捨ててはならない(ADR-292 R6(ii)の訂正と同型、BUG-074型の無断消失を避ける)
    /// ——呼び出し側が利用者へ通知する責任を持つ。
    Discard(Vec<T>),
}

/// 保持中に打鍵が来たときの扱い。
#[derive(Debug, PartialEq, Eq)]
pub enum OnKey<T> {
    /// 保持が無い打鍵。呼び出し側が通常どおり処理する(打鍵を返す)。
    Pass(T),
    /// 保持に足した(呼び出し側は消費済みとして扱う)。
    Held,
    /// 上限に達した。この打鍵を列の末尾に含めて解放する(窓の照合は `release` と同じ)。
    Release(Released<T>),
}

#[derive(Debug)]
struct Active<T> {
    id: u64,
    scope: ForegroundScope,
    /// モードキー予測が立った時刻(`pred.at_ms`)。G3のbaseline計算(呼び出し側)の起点。
    pred_at_ms: TickMs,
    items: Vec<T>,
}

/// `KeyEffectPrediction`未照合中の物理キー保持。同時に1つだけ。
#[derive(Debug)]
pub struct KeyEffectOpenHold<T> {
    active: Option<Active<T>>,
    next_id: u64,
}

impl<T> Default for KeyEffectOpenHold<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> KeyEffectOpenHold<T> {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            active: None,
            next_id: 1,
        }
    }

    #[must_use]
    pub const fn is_holding(&self) -> bool {
        self.active.is_some()
    }

    #[must_use]
    pub fn hold_id(&self) -> Option<u64> {
        self.active.as_ref().map(|a| a.id)
    }

    /// 保持しているモードキー予測の時刻(`pred.at_ms`)。G3のbaseline計算に使う。
    #[must_use]
    pub fn pred_at_ms(&self) -> Option<TickMs> {
        self.active.as_ref().map(|a| a.pred_at_ms)
    }

    /// 保持中の件数(ログ用)。
    #[must_use]
    pub fn len(&self) -> usize {
        self.active.as_ref().map_or(0, |a| a.items.len())
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 保持を始める。既に保持中なら始めず `None`(呼び出し側は通常どおり処理する——
    /// ADR-292 round4 U2「予測ごとに1回」はこことは別軸で、呼び出し側が
    /// 「この予測はもう一度確認済みで保持対象ではない」と判断した場合はそもそも
    /// `start`を呼ばない)。始めたら新しい `hold_id` を返す。
    pub fn start(&mut self, scope: ForegroundScope, pred_at_ms: TickMs, item: T) -> Option<u64> {
        if self.active.is_some() {
            return None;
        }
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        self.active = Some(Active {
            id,
            scope,
            pred_at_ms,
            items: vec![item],
        });
        Some(id)
    }

    /// 保持中に打鍵が来た。保持が無ければ `Pass`。上限に達したら、この打鍵を列の末尾に
    /// 含めて解放する(追い越させない)。モードキーの再押下(ADR-292 round3 T2)も、
    /// 呼び出し側が区別せずここへ渡せば自然に保持される
    /// (`KeyEffectPredicted`の再発火・打刻は呼び出し側が再生の時点で行う)。
    pub fn on_key(&mut self, item: T, current: ForegroundScope) -> OnKey<T> {
        let Some(mut active) = self.active.take() else {
            return OnKey::Pass(item);
        };
        active.items.push(item);
        if active.items.len() > KEY_EFFECT_OPEN_HOLD_CAP {
            return OnKey::Release(Self::finish(active, current));
        }
        self.active = Some(active);
        OnKey::Held
    }

    /// 解放する。`hold_id` が `Some` なら今の保持と一致するときだけ(古い札の通知を無視する)。
    /// 前面窓が保持の開始時と違えば `Discard`。解放する理由(開いた/閉で確定/期限/読めなく
    /// なった等)は呼び出し側の責務——ここでは窓の照合と中身の取り出しだけを行う。
    pub fn release(&mut self, hold_id: Option<u64>, current: ForegroundScope) -> Released<T> {
        let Some(active) = self.active.take() else {
            return Released::Nothing;
        };
        if hold_id.is_some_and(|id| id != active.id) {
            self.active = Some(active);
            return Released::Nothing;
        }
        Self::finish(active, current)
    }

    fn finish(active: Active<T>, current: ForegroundScope) -> Released<T> {
        let items = active.items;
        if active.scope == current {
            Released::Replay(items)
        } else {
            Released::Discard(items)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(pid: u32, hwnd: isize) -> ForegroundScope {
        ForegroundScope { pid, hwnd }
    }

    fn started() -> (KeyEffectOpenHold<&'static str>, u64) {
        let mut h = KeyEffectOpenHold::new();
        let id = h
            .start(scope(1, 10), TickMs(100), "mode-key")
            .expect("start");
        (h, id)
    }

    /// 保持が無いとき、打鍵はそのまま返る(通常どおり処理する)。
    #[test]
    fn idle_passes_everything_through() {
        let mut h: KeyEffectOpenHold<&str> = KeyEffectOpenHold::new();
        assert_eq!(h.on_key("k", scope(1, 10)), OnKey::Pass("k"));
        assert!(!h.is_holding());
        assert_eq!(h.release(None, scope(1, 10)), Released::Nothing);
    }

    /// 保持中は同じ保持を二重に始めない。
    #[test]
    fn cannot_start_twice() {
        let (mut h, _) = started();
        assert_eq!(h.start(scope(1, 10), TickMs(101), "other"), None);
        assert_eq!(h.len(), 1);
    }

    /// 解放の列は保持した順序どおり。モードキーの再押下(T2)も区別せず保持される。
    #[test]
    fn release_preserves_order_including_a_re_pressed_mode_key() {
        let (mut h, id) = started();
        assert_eq!(h.on_key("k", scope(1, 10)), OnKey::Held);
        assert_eq!(h.on_key("re-mode-key", scope(1, 10)), OnKey::Held);
        assert_eq!(
            h.release(Some(id), scope(1, 10)),
            Released::Replay(vec!["mode-key", "k", "re-mode-key"])
        );
        assert!(!h.is_holding());
    }

    /// 上限に達した新しい打鍵は、契機の打鍵を列の末尾に含めて全て解放する(追い越させない)。
    #[test]
    fn cap_releases_trigger_key_at_the_end() {
        let (mut h, _) = started();
        for name in ["k2", "k3", "k4", "k5", "k6", "k7", "k8"] {
            assert_eq!(h.on_key(name, scope(1, 10)), OnKey::Held);
        }
        assert_eq!(h.len(), KEY_EFFECT_OPEN_HOLD_CAP);
        let out = h.on_key("k9", scope(1, 10));
        assert_eq!(
            out,
            OnKey::Release(Released::Replay(vec![
                "mode-key", "k2", "k3", "k4", "k5", "k6", "k7", "k8", "k9"
            ]))
        );
        assert!(!h.is_holding());
    }

    /// 前面窓が保持の開始時と違えば、理由を問わず保持の中の打鍵はすべて `Discard`
    /// (別窓へ誤配送しない。利用者への通知は呼び出し側の責務、ADR-292 R6(ii)参照)。
    #[test]
    fn scope_change_discards_everything_regardless_of_release_reason() {
        let (mut h, id) = started();
        h.on_key("k", scope(1, 10));
        assert_eq!(
            h.release(Some(id), scope(2, 20)),
            Released::Discard(vec!["mode-key", "k"])
        );
    }

    /// 上限での解放にも窓の照合が掛かる。
    #[test]
    fn cap_release_also_checks_scope() {
        let (mut h, _) = started();
        for i in 0..7 {
            h.on_key(Box::leak(format!("k{i}").into_boxed_str()), scope(1, 10));
        }
        assert!(matches!(
            h.on_key("k8", scope(9, 9)),
            OnKey::Release(Released::Discard(_))
        ));
    }

    /// 古い札(別の世代)の通知は、新しい保持を解放しない。
    #[test]
    fn stale_hold_id_does_not_release_a_new_hold() {
        let (mut h, old_id) = started();
        assert!(matches!(
            h.release(Some(old_id), scope(1, 10)),
            Released::Replay(_)
        ));
        let new_id = h
            .start(scope(1, 10), TickMs(200), "mode-key-2")
            .expect("start");
        assert_ne!(old_id, new_id);
        assert_eq!(h.release(Some(old_id), scope(1, 10)), Released::Nothing);
        assert!(h.is_holding());
        assert!(matches!(
            h.release(Some(new_id), scope(1, 10)),
            Released::Replay(_)
        ));
    }

    /// `pred_at_ms`は保持開始時のモードキー予測の時刻をそのまま返す(G3のbaseline計算に使う)。
    #[test]
    fn pred_at_ms_is_the_start_time() {
        let (h, _) = started();
        assert_eq!(h.pred_at_ms(), Some(TickMs(100)));
    }

    /// 世代は増え、0 にならない。
    #[test]
    fn hold_ids_are_monotonic_and_nonzero() {
        let mut h: KeyEffectOpenHold<u8> = KeyEffectOpenHold::new();
        let mut last = 0;
        for i in 0..5 {
            let id = h.start(scope(1, 1), TickMs(i), 0).expect("start");
            assert!(id > last);
            last = id;
            let _ = h.release(None, scope(1, 1));
        }
    }
}
