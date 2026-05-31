use std::sync::Mutex;
use std::thread::JoinHandle;
use std::time::Duration;

/// タイムアウトで放棄されたワーカースレッドのリスト。
///
/// 次の `run_with_timeout` 呼び出し時に完了済みのものを刈り取る（GC）。
/// 永久にブロックする API を叩いたスレッドは `is_finished()` が false のままなので
/// GC できない。そのため上限を設け、満杯なら新規 spawn を拒否してリソース暴走を防ぐ。
struct LeakedThreadPool {
    threads: Mutex<Vec<JoinHandle<()>>>,
    max: usize,
}

impl LeakedThreadPool {
    const fn new(max: usize) -> Self {
        Self {
            threads: Mutex::new(Vec::new()),
            max,
        }
    }

    fn reap(&self) {
        let Ok(mut leaked) = self.threads.lock() else {
            return;
        };
        let before = leaked.len();
        leaked.retain(|h| !h.is_finished());
        let reaped = before - leaked.len();
        if reaped > 0 {
            log::debug!(
                "Reaped {reaped} finished leaked worker threads ({} remaining)",
                leaked.len()
            );
        }
    }

    fn leak(&self, handle: JoinHandle<()>) {
        let Ok(mut leaked) = self.threads.lock() else {
            return;
        };
        leaked.push(handle);
        log::warn!("Leaked worker thread (now {} in list)", leaked.len());
    }

    fn is_full(&self) -> bool {
        self.threads
            .lock()
            .is_ok_and(|leaked| leaked.len() >= self.max)
    }
}

static LEAKED_THREADS: LeakedThreadPool = LeakedThreadPool::new(8);

/// タイムアウト付きで任意の処理をワーカースレッドで実行する。
///
/// ブロッキング Win32 API（IMM32, MSAA, UIA 等）を安全に呼び出すために使用する。
/// タイムアウトした場合は `None` を返し、ワーカースレッドは孤児スレッドリストに追加され、
/// 次回の呼び出し時に完了していれば刈り取られる（GC）。
///
/// # Type parameters
/// - `T`: 戻り値の型。`Send + 'static` である必要がある。
///
/// # 制約
/// クロージャ内では COM/IMM32/GDI 等のスレッド親和性のある API を呼び出せない。
/// `GetForegroundWindow`, `GetGUIThreadInfo`, `SendMessageTimeoutW` 等の
/// 読み取り系 API は一般的にワーカースレッドから呼んでも安全。
#[must_use]
pub fn run_with_timeout<T, F>(timeout: Duration, f: F) -> Option<T>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    LEAKED_THREADS.reap();

    if LEAKED_THREADS.is_full() {
        log::error!(
            "Leaked thread list is full ({}), refusing to spawn new worker. \
             A Win32 API is persistently blocking.",
            LEAKED_THREADS.max
        );
        return None;
    }

    let (tx, rx) = std::sync::mpsc::sync_channel::<T>(1);
    let handle: JoinHandle<()> = std::thread::spawn(move || {
        let result = f();
        let _ = tx.send(result);
    });

    match rx.recv_timeout(timeout) {
        Ok(result) => {
            let _ = handle.join();
            Some(result)
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            let _ = handle.join();
            log::error!("run_with_timeout: worker thread ended without result");
            None
        }
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            log::warn!(
                "run_with_timeout: worker thread exceeded {}ms, leaked for later GC",
                timeout.as_millis()
            );
            LEAKED_THREADS.leak(handle);
            None
        }
    }
}
