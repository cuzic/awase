// ブロッキング処理をワーカースレッドで実行し、async で結果を待つ。
// sleep_ms(5) でポーリングするのでメインスレッドのメッセージループをブロックしない。
#[allow(clippy::future_not_send)]
pub async fn offload<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    use std::sync::{Arc, Mutex};
    use std::sync::atomic::{AtomicBool, Ordering};

    let done = Arc::new(AtomicBool::new(false));
    let result: Arc<Mutex<Option<T>>> = Arc::new(Mutex::new(None));

    {
        let done = Arc::clone(&done);
        let result = Arc::clone(&result);
        std::thread::spawn(move || {
            *result.lock().unwrap() = Some(f());
            done.store(true, Ordering::Release);
        });
    }

    while !done.load(Ordering::Acquire) {
        super::sleep_ms(5).await;
    }

    let value = result.lock().unwrap().take().unwrap();
    value
}
