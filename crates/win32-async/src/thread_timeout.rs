use std::sync::Mutex;
use std::thread::JoinHandle;
use std::time::Duration;

/// タイムアウトで放棄されたワーカースレッドのリスト。
///
/// 次の `run_with_timeout` 呼び出し時に完了済みのものを刈り取る（GC）。
/// 永久にブロックする API を叩いたスレッドは `is_finished()` が false のままなので
/// GC できない。そのため上限を設け、満杯なら新規 spawn を拒否してリソース暴走を防ぐ。
static LEAKED_THREADS: Mutex<Vec<JoinHandle<()>>> = Mutex::new(Vec::new());

/// 孤児スレッドの最大許容数。これを超えると `run_with_timeout` は spawn せず即 `None` を返す。
const LEAKED_THREAD_MAX: usize = 8;

fn reap_leaked_threads() {
    let Ok(mut leaked) = LEAKED_THREADS.lock() else {
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

fn leak_thread(handle: JoinHandle<()>) {
    let Ok(mut leaked) = LEAKED_THREADS.lock() else {
        return;
    };
    leaked.push(handle);
    log::warn!("Leaked worker thread (now {} in list)", leaked.len());
}

fn is_leaked_list_full() -> bool {
    LEAKED_THREADS
        .lock()
        .map_or(false, |leaked| leaked.len() >= LEAKED_THREAD_MAX)
}

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
    reap_leaked_threads();

    if is_leaked_list_full() {
        log::error!(
            "Leaked thread list is full ({LEAKED_THREAD_MAX}), refusing to spawn new worker. \
             A Win32 API is persistently blocking."
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
            return Some(result);
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            let _ = handle.join();
            log::error!("run_with_timeout: worker thread ended without result");
            return None;
        }
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            log::warn!(
                "run_with_timeout: worker thread exceeded {}ms, leaked for later GC",
                timeout.as_millis()
            );
            leak_thread(handle);
            return None;
        }
    }
}
