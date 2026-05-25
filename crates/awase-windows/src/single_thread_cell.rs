use std::cell::{RefCell, RefMut};

/// シングルスレッド専用の内部可変性コンテナ。
///
/// `RefCell<Option<T>>` を `static` に置けるようにするラッパー。
/// `RefCell` は `Sync` でないため `unsafe impl Sync` が必要だが、
/// 実際のアクセスはメインスレッド（Windows メッセージループ）からのみ行われる。
///
/// `UnsafeCell` ベースの旧実装と異なり、`RefCell` の実行時借用チェックにより
/// 二重借用（再入）は UB ではなく安全な `None` 返却として扱われる。
pub struct SingleThreadCell<T>(RefCell<Option<T>>);

impl<T> std::fmt::Debug for SingleThreadCell<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("SingleThreadCell").finish()
    }
}

// Safety: 実際のアクセスはメインスレッドからのみ行われる（上記ドキュメント参照）。
unsafe impl<T> Sync for SingleThreadCell<T> {}

impl<T> Default for SingleThreadCell<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> SingleThreadCell<T> {
    pub const fn new() -> Self {
        Self(RefCell::new(None))
    }

    /// 値を設定する。メッセージループ開始前の初期化専用。
    ///
    /// # Panics
    ///
    /// 既に借用中の場合（= メッセージループ中に誤って呼ばれた場合）。
    pub fn set(&self, val: T) {
        *self.0.borrow_mut() = Some(val);
    }

    /// 値を破棄する。シャットダウン時専用。
    ///
    /// # Panics
    ///
    /// 既に借用中の場合。
    pub fn clear(&self) {
        *self.0.borrow_mut() = None;
    }

    /// 値が存在する場合に `f` を共有参照で呼び出す。
    ///
    /// 排他借用中の場合は `None` を返す（UB なし）。
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> Option<R> {
        self.0.try_borrow().ok().and_then(|g| g.as_ref().map(f))
    }

    /// 値が存在する場合に `f` を可変参照で呼び出す。
    ///
    /// 既に借用中の場合は `None` を返す（UB なし）。
    pub fn try_with_mut<R>(&self, f: impl FnOnce(&mut T) -> R) -> Option<R> {
        self.0.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f))
    }

    /// 値への可変借用を試みる。
    ///
    /// 返却された guard が生きている間は再借用不可。
    /// 既に借用中の場合は `None` を返す（UB なし）。
    pub fn try_borrow_mut(&self) -> Option<RefMut<'_, Option<T>>> {
        self.0.try_borrow_mut().ok()
    }

    /// 現在排他借用中かどうかを返す。
    pub fn is_borrowed_mut(&self) -> bool {
        self.0.try_borrow_mut().is_err()
    }
}
