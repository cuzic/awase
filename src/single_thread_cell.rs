use std::cell::UnsafeCell;

/// シングルスレッド専用のグローバルセル
/// Safety: このプログラムはシングルスレッドで動作し、フックコールバックと
/// メッセージループは同一スレッドで順次実行される。
pub struct SingleThreadCell<T>(UnsafeCell<Option<T>>);
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
