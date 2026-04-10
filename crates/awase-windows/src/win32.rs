//! Windows API の安全ラッパー

use std::os::windows::io::AsRawHandle;
use std::sync::Mutex;
use std::thread::JoinHandle;
use std::time::Duration;

use windows::Win32::Foundation::{HANDLE, HWND};
use windows::Win32::System::Threading::TerminateThread;
use windows::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId, GUITHREADINFO,
};

/// タイムアウトで放棄されたワーカースレッドのリスト。
///
/// 次の `run_with_timeout` 呼び出し時に完了済みのものを刈り取る（GC）。
/// 永久にブロックする API を叩いたスレッドは `is_finished()` が false のままなので
/// GC できない。そのため上限を設け、満杯なら新規 spawn を拒否してリソース暴走を防ぐ。
static LEAKED_THREADS: Mutex<Vec<JoinHandle<()>>> = Mutex::new(Vec::new());

/// 孤児スレッドの最大許容数。これを超えると `run_with_timeout` は spawn せず即 `None` を返す。
const LEAKED_THREAD_MAX: usize = 8;

/// 孤児スレッドリストから完了済みのものを刈り取る。
/// `run_with_timeout` の冒頭で呼ばれる。
///
/// `is_finished() == true` の `JoinHandle` を `retain` で削除すると drop され、
/// 既に終了している OS スレッドのリソースが回収される。
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

/// 孤児スレッドリストに追加する。
fn leak_thread(handle: JoinHandle<()>) {
    let Ok(mut leaked) = LEAKED_THREADS.lock() else {
        return;
    };
    leaked.push(handle);
    log::warn!(
        "Leaked worker thread (now {} in list)",
        leaked.len()
    );
}

/// 孤児スレッドリストが満杯のとき、最も古い（インデックス 0）スレッドを
/// `TerminateThread` で強制終了してスロットを空ける。
///
/// ラウンドロビン的に古いスレッドを捨てることで、新しいリクエストを受け付けられるようにする。
///
/// # 警告
/// `TerminateThread` はスレッドのデストラクタを実行せず、ロックを解放しない危険な API。
/// しかし対象はすでにハングしているスレッドなので、失うものは少ない。
/// メッセージループを守る方が優先。
fn terminate_oldest_leaked_if_full() -> bool {
    let Ok(mut leaked) = LEAKED_THREADS.lock() else {
        return false;
    };
    if leaked.len() < LEAKED_THREAD_MAX {
        return true; // 空きがあるので何もしない
    }
    // 最も古いスレッドを強制終了
    let oldest = leaked.remove(0);
    let raw_handle = oldest.as_raw_handle();
    unsafe {
        let handle = HANDLE(raw_handle.cast());
        match TerminateThread(handle, 1) {
            Ok(()) => {
                log::warn!(
                    "Forcibly terminated oldest leaked worker thread (round-robin, {} remaining)",
                    leaked.len()
                );
                // JoinHandle を drop → OS スレッドリソースが回収される
                drop(oldest);
                true
            }
            Err(e) => {
                log::error!("TerminateThread failed: {e}, putting handle back");
                leaked.insert(0, oldest);
                false
            }
        }
    }
}

/// タイムアウト付きで任意の処理をワーカースレッドで実行する。
///
/// ブロッキング Win32 API（IMM32, MSAA, UIA 等）を安全に呼び出すために使用する。
/// タイムアウトした場合は `None` を返し、ワーカースレッドは孤児スレッドリストに追加され、
/// 次回の `run_with_timeout` 呼び出し時に完了していれば刈り取られる（GC）。
///
/// # Type parameters
/// - `T`: 戻り値の型。`Send + 'static` である必要がある。
///
/// # 制約
/// クロージャ内では COM/IMM32/GDI 等のスレッド親和性のある API を呼び出せない。
/// ただし `GetForegroundWindow`, `GetGUIThreadInfo`, `SendMessageTimeoutW` 等の
/// 読み取り系 API は一般的にワーカースレッドから呼んでも安全。
pub fn run_with_timeout<T, F>(timeout: Duration, f: F) -> Option<T>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    // 前回以前にリークしたスレッドのうち完了済みのものを刈り取る
    reap_leaked_threads();

    // 満杯なら最古のスレッドを TerminateThread で強制終了してスロットを空ける（ラウンドロビン）。
    // 万一終了に失敗した場合は新規 spawn を諦めて None を返す。
    if !terminate_oldest_leaked_if_full() {
        log::error!(
            "Leaked thread list is full and TerminateThread failed, refusing to spawn new worker."
        );
        return None;
    }

    // 結果はチャンネルで受け取る（JoinHandle の型を () に揃えるため）
    let (tx, rx) = std::sync::mpsc::sync_channel::<T>(1);
    let handle: JoinHandle<()> = std::thread::spawn(move || {
        let result = f();
        // 受信側がタイムアウトで drop されている可能性があるので送信失敗は無視
        let _ = tx.send(result);
    });

    let start = std::time::Instant::now();
    loop {
        // 結果が届いたか確認
        match rx.try_recv() {
            Ok(result) => {
                // スレッドは間もなく終了するはず。join で回収する。
                let _ = handle.join();
                return Some(result);
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                // ワーカーがパニック等で送信せずに終了
                let _ = handle.join();
                log::error!("run_with_timeout: worker thread ended without result");
                return None;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }

        if start.elapsed() >= timeout {
            log::warn!(
                "run_with_timeout: worker thread exceeded {}ms, leaked for later GC",
                timeout.as_millis()
            );
            leak_thread(handle);
            return None;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// `SendInput` の安全ラッパー（`size_of` キャストを安全に処理）
pub fn send_input_safe(inputs: &[INPUT]) -> u32 {
    let size = i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32");
    unsafe { SendInput(inputs, size) }
}

/// `GetGUIThreadInfo` の結果
#[derive(Debug, Clone, Copy)]
pub struct GuiThreadResult {
    /// フォーカスを持つウィンドウ（フォールバック時は `GetForegroundWindow` の結果）
    pub focused_hwnd: HWND,
    /// ウィンドウが属するスレッド ID（0 = 取得失敗）
    pub thread_id: u32,
}

/// `GetGUIThreadInfo(0, ...)` のラッパー — ブロッキングが一定時間を超えたら
/// フォールバックとして `GetForegroundWindow()` を返す。
///
/// `GetGUIThreadInfo` はフォアグラウンドウィンドウの GUI スレッドにメッセージを送るため、
/// 対象スレッドがハングしていると無期限にブロックする。
/// ワーカースレッドで実行し、タイムアウトした場合は非ブロッキングな
/// `GetForegroundWindow` にフォールバックする。
///
/// # Safety
/// Win32 API を呼び出す。
pub unsafe fn get_gui_thread_info_with_timeout(timeout: Duration) -> GuiThreadResult {
    // HWND はポインタだが、スレッド間で安全に送信可能
    // （Win32 ウィンドウハンドルはプロセス内で有効なグローバルリソース）
    struct SendableResult(HWND, u32);
    unsafe impl Send for SendableResult {}

    let handle = std::thread::spawn(|| {
        let mut info = GUITHREADINFO {
            cbSize: u32::try_from(size_of::<GUITHREADINFO>()).unwrap(),
            ..Default::default()
        };
        unsafe {
            if GetGUIThreadInfo(0, &raw mut info).is_ok() {
                let hwnd = if info.hwndFocus.0.is_null() {
                    info.hwndActive
                } else {
                    info.hwndFocus
                };
                let mut pid = 0u32;
                let tid = GetWindowThreadProcessId(hwnd, Some(&raw mut pid));
                SendableResult(hwnd, tid)
            } else {
                let hwnd = GetForegroundWindow();
                SendableResult(hwnd, 0)
            }
        }
    });

    // タイムアウト付き join: park_timeout で待機
    let start = std::time::Instant::now();
    loop {
        if handle.is_finished() {
            match handle.join() {
                Ok(SendableResult(hwnd, tid)) => {
                    return GuiThreadResult {
                        focused_hwnd: hwnd,
                        thread_id: tid,
                    };
                }
                Err(_) => {
                    // ワーカースレッドがパニックした場合はフォールバック
                    log::error!("GetGUIThreadInfo worker thread panicked");
                    break;
                }
            }
        }
        if start.elapsed() >= timeout {
            log::warn!(
                "GetGUIThreadInfo timed out after {}ms, falling back to GetForegroundWindow",
                timeout.as_millis()
            );
            // ワーカースレッドは放置（OS がスレッド終了時に回収）
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }

    // フォールバック: GetForegroundWindow は非ブロッキング
    let hwnd = unsafe { GetForegroundWindow() };
    GuiThreadResult {
        focused_hwnd: hwnd,
        thread_id: 0,
    }
}
