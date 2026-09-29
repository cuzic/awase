use std::sync::Mutex;
use std::thread::JoinHandle;
use std::time::Duration;

/// タイムアウトで放棄されたワーカースレッドのリスト。
///
/// 次の `run_with_timeout` 呼び出し時に完了済みのものを刈り取る（GC）。
/// 永久にブロックする API を叩いたスレッドは `is_finished()` が false のままなので
/// GC できない。そのため上限を設け、満杯なら新規 spawn を拒否してリソース暴走を防ぐ。
///
/// 呼び出し元は用途ごとに別々の `static` インスタンスを持てる（[`run_with_timeout_in`]）。
/// 無関係な用途（例: IMM32/MSAA/UIAのフォーカス分類と、キーボードフックの
/// 再インストール待ち）が同じ8枠のプールを共有すると、一方の詰まりが他方の
/// 枠を奪い合う結合が生まれるため（issue #165自己修復のPR #349レビューで指摘）、
/// 気にする粒度で分けられるようにしてある。デフォルトの共有プール
/// （[`run_with_timeout`]が使う）は既存の全呼び出し元向けに残す。
pub struct LeakedThreadPool {
    threads: Mutex<Vec<JoinHandle<()>>>,
    max: usize,
}

impl std::fmt::Debug for LeakedThreadPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let len = self.threads.lock().map_or(0, |l| l.len());
        f.debug_struct("LeakedThreadPool")
            .field("len", &len)
            .field("max", &self.max)
            .finish()
    }
}

impl LeakedThreadPool {
    #[must_use]
    pub const fn new(max: usize) -> Self {
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
            tracing::debug!(
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
        tracing::warn!("Leaked worker thread (now {} in list)", leaked.len());
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
/// 次回の呼び出し時に完了していれば刈り取られる（GC）。全呼び出し元共有の
/// デフォルトプールを使う。無関係な用途との枠の奪い合いを避けたい場合は
/// [`run_with_timeout_in`] で専用の `LeakedThreadPool` を渡すこと。
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
    run_with_timeout_in(&LEAKED_THREADS, timeout, f)
}

/// [`run_with_timeout`]と同じだが、孤児スレッドの上限管理を呼び出し元指定の
/// `pool`で行う（デフォルトの共有プールを使わない）。
///
/// 用途の異なるブロッキング呼び出し（例: フォーカス分類のIMM32/MSAA/UIAと、
/// キーボードフック再インストールのjoin待ち）が同じ枠を奪い合わないよう、
/// 呼び出し元は`static`な専用`LeakedThreadPool`を用意して渡せる。
#[must_use]
pub fn run_with_timeout_in<T, F>(
    pool: &'static LeakedThreadPool,
    timeout: Duration,
    f: F,
) -> Option<T>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    pool.reap();

    if pool.is_full() {
        tracing::error!(
            "Leaked thread list is full ({}), refusing to spawn new worker. \
             A Win32 API is persistently blocking.",
            pool.max
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
            tracing::error!("run_with_timeout: worker thread ended without result");
            None
        }
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            tracing::warn!(
                "run_with_timeout: worker thread exceeded {}ms, leaked for later GC",
                timeout.as_millis()
            );
            pool.leak(handle);
            None
        }
    }
}
