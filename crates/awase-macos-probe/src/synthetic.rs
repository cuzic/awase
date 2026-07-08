//! 合成イベントの自己識別（[`SyntheticEventOrigin`]）と、送出済みで key-up 未送信の
//! キー追跡（[`SyntheticPressedKeys`]）を **別々の型** として扱う。
//!
//! この 2 責務を 1 つの集合で兼ねると実バグを生む（awase が合成 A-down を送出した
//! 直後にユーザーが物理 A を押すと、その物理イベントも self-synthetic と誤判定される）。
//! 設計根拠は project_macos_probe_interfaces.md の「synthetic identity: 2つの責務を分離」を参照。
//!
//! 実際に CGEvent を組み立てて送出する低レベルプリミティブは、この型を利用する側の
//! [`crate::output`] に置く（責務分割は teammate タスク #5 の指定に従う）。

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};

/// 受信イベントが自プロセスの生成物かどうかの判定「専用」の識別子。
///
/// `cookie` は固定の magic 値ではなく **プロセス起動ごとのランダム 64bit** にする。
/// Probe と本体の同時起動・複数インスタンス・他ツールとの偶然の衝突を避けるためで、
/// セキュリティ境界ではなく識別の衝突回避が目的。`std` に RNG は無く、`rand` crate を
/// 足したくないので、`RandomState`（OS 由来のシードで毎プロセス初期化される Hasher）で
/// プロセス ID と現在時刻を混ぜて擬似乱数を得る。
#[derive(Debug)]
pub struct SyntheticEventOrigin {
    cookie: i64,
}

impl SyntheticEventOrigin {
    /// このプロセス固有のランダムな cookie で新しい origin を作る。
    #[must_use]
    pub fn new() -> Self {
        Self {
            cookie: random_cookie(),
        }
    }

    /// 受信イベントの `kCGEventSourceUserData` 値が自分の cookie と完全一致するときだけ真。
    #[must_use]
    pub const fn is_self_event(&self, event_user_data: i64) -> bool {
        event_user_data == self.cookie
    }

    /// 送出イベントに埋め込むための cookie 値。
    #[must_use]
    pub const fn cookie(&self) -> i64 {
        self.cookie
    }
}

impl Default for SyntheticEventOrigin {
    fn default() -> Self {
        Self::new()
    }
}

/// 自分が down を送出し、まだ up を送っていないキーの追跡「専用」。
///
/// tap 無効化などで断絶したときに、押しっぱなしで残る stuck key を防ぐため、
/// [`Self::take_all_for_release`] で未解放キーを取り出して best-effort key-up を送る
/// （実送出は [`crate::output::release_all_best_effort`]）。
#[derive(Debug, Default)]
pub struct SyntheticPressedKeys {
    pressed: Vec<u16>,
}

impl SyntheticPressedKeys {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            pressed: Vec::new(),
        }
    }

    /// `keycode` を「down 済み」として記録する。同じ keycode の重複 `mark_down` は
    /// 冪等（二重登録しない）。物理配列上 1 つのキーが同時に 2 度 down になることは
    /// なく、二重登録すると [`Self::take_all_for_release`] が余分な key-up を送るため。
    ///
    /// Phase M0 の tap.rs はまだ observe-only（自ら合成キーを送出しない）ため、
    /// 呼び出し元が無い。M1+ で tap 内から `output::post_key_down` を呼ぶ経路が
    /// できた時に、その直後でこれを呼んで追跡する。
    #[allow(dead_code)]
    pub fn mark_down(&mut self, keycode: u16) {
        if !self.pressed.contains(&keycode) {
            self.pressed.push(keycode);
        }
    }

    /// `keycode` の「down 済み」記録を消す。未登録キーの `mark_up` は無視する。
    /// [`Self::mark_down`] と対。呼び出し元が無い理由も同様（M1+ で配線）。
    #[allow(dead_code)]
    pub fn mark_up(&mut self, keycode: u16) {
        self.pressed.retain(|&k| k != keycode);
    }

    /// 未解放キーを drain して返し、内部記録を空にする。断絶からの復旧時に、
    /// 返った keycode 群へ tagged key-up を送るために使う。二度目の呼び出しは空を返す。
    #[must_use]
    pub fn take_all_for_release(&mut self) -> Vec<u16> {
        std::mem::take(&mut self.pressed)
    }
}

/// OS 由来シードの `RandomState` から擬似乱数 64bit を得て `i64` にする。
fn random_cookie() -> i64 {
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u32(std::process::id());
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    hasher.write_u128(nanos);
    i64::from_ne_bytes(hasher.finish().to_ne_bytes())
}

#[cfg(test)]
mod tests {
    use super::{SyntheticEventOrigin, SyntheticPressedKeys};

    #[test]
    fn mark_down_then_mark_up_removes() {
        let mut keys = SyntheticPressedKeys::new();
        keys.mark_down(1);
        keys.mark_down(2);
        keys.mark_down(3);
        keys.mark_up(2);
        assert_eq!(keys.take_all_for_release(), vec![1, 3]);
    }

    #[test]
    fn duplicate_mark_down_is_idempotent() {
        let mut keys = SyntheticPressedKeys::new();
        keys.mark_down(7);
        keys.mark_down(7);
        keys.mark_down(7);
        assert_eq!(keys.take_all_for_release(), vec![7]);
    }

    #[test]
    fn mark_up_of_unknown_key_is_noop() {
        let mut keys = SyntheticPressedKeys::new();
        keys.mark_down(5);
        keys.mark_up(99);
        assert_eq!(keys.take_all_for_release(), vec![5]);
    }

    #[test]
    fn take_all_drains_and_second_call_is_empty() {
        let mut keys = SyntheticPressedKeys::new();
        keys.mark_down(10);
        keys.mark_down(11);
        assert_eq!(keys.take_all_for_release(), vec![10, 11]);
        assert!(keys.take_all_for_release().is_empty());
    }

    #[test]
    fn is_self_event_matches_only_exact_cookie() {
        let origin = SyntheticEventOrigin::new();
        let cookie = origin.cookie();
        assert!(origin.is_self_event(cookie));
        assert!(!origin.is_self_event(cookie.wrapping_add(1)));
        assert!(!origin.is_self_event(0));
    }

    #[test]
    fn two_origins_have_distinct_cookies() {
        // 64bit random の偶然衝突は事実上起きない（ベストエフォートな確率的テスト）。
        let a = SyntheticEventOrigin::new();
        let b = SyntheticEventOrigin::new();
        assert_ne!(a.cookie(), b.cookie());
    }
}
