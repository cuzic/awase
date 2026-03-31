use std::cell::UnsafeCell;

/// シングルスレッド専用の内部可変性コンテナ。
///
/// # Safety
///
/// `get_mut()` は `&self` から `&mut T` を返す。これは通常 Rust のエイリアシング規則に
/// 違反するが、Windows のメッセージループがシングルスレッドであることを前提に安全性を保証する。
/// マルチスレッドアクセスが必要な場合は `Mutex<T>` 等を使用すること。
///
/// `Sync` を実装しているのは、static 変数として使用するため。
/// 実際のアクセスはメインスレッド（メッセージループ + フックコールバック）からのみ行われる。
pub struct SingleThreadCell<T>(UnsafeCell<Option<T>>);
// Safety: 実際のアクセスはメインスレッドからのみ行われる（上記ドキュメント参照）。
unsafe impl<T> Sync for SingleThreadCell<T> {}

impl<T> SingleThreadCell<T> {
    pub const fn new() -> Self {
        Self(UnsafeCell::new(None))
    }

    /// Safety: シングルスレッドからのみ呼び出すこと
    pub unsafe fn set(&self, val: T) {
        *self.0.get() = Some(val);
    }

    /// Safety: シングルスレッドからのみ呼び出すこと
    pub unsafe fn get_mut(&self) -> Option<&mut T> {
        (*self.0.get()).as_mut()
    }

    /// Safety: シングルスレッドからのみ呼び出すこと
    pub unsafe fn get_ref(&self) -> Option<&T> {
        (*self.0.get()).as_ref()
    }

    /// Safety: シングルスレッドからのみ呼び出すこと
    pub unsafe fn clear(&self) {
        *self.0.get() = None;
    }
}
