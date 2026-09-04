//! [BUG-112](../../../docs/known-bugs.md) の恒久修正（グレースピリオド定数の
//! 導入）に必要な実測値を取るための使い捨てスパイク。
//!
//! ## 検証したいこと
//!
//! `docs/known-bugs.md` BUG-112 は、`awase-settings.exe --bug-report` の
//! ウィンドウが生成された直後、`ImmGetDefaultIMEWnd(hwnd)` が一時的に
//! `NULL` を返すことがあり（実 Win32 レベルの競合状態と推定）、
//! `ImmCapabilityStore::UNAVAILABLE_CONFIRM_THRESHOLD = 2`（500ms 周期の
//! ポーリングで2回連続 NULL）がこの一時的な状態を恒久的な `Unavailable` と
//! 誤確定してしまう、という仮説を立てた（2回の手動再現実験で「works に
//! なる回」「unavailable になる回」の両方を観測済み——再現性なく揺れる
//! ことは確認済みだが、NULL が持続する実時間は未計測）。
//!
//! `.claude/rules/tuning-constants.md` は新規タイミング定数の導入に実測値を
//! 要求するため、本スパイクは「ウィンドウ生成から `ImmGetDefaultIMEWnd` が
//! 安定して non-NULL を返すようになるまでの実時間」の分布を、
//! `awase-settings.exe --bug-report` を繰り返し起動して機械的に計測する。
//!
//! ## 実行方法（Windows 実機のみ、`awase.exe` は起動していなくてよい——
//! 本スパイクは `imm.rs::get_ime_wnd` と同一の Win32 呼び出しを直接行う
//! ため、`awase.exe` の学習ロジックそのものは経由しない独立測定）
//!
//! ```powershell
//! cargo run -p awase-windows --example spike_bug112_ime_wnd_race_probe --release -- <awase-settings.exeへのパス> [試行回数]
//! ```
//!
//! 引数省略時は `target/release/awase-settings.exe`（カレントディレクトリ
//! 基準）、試行回数省略時は 10 回。
//!
//! ## 出力
//!
//! 試行ごとに1行、`run=N window_found_ms=A first_nonnull_ms=B null_duration_ms=C`
//! （`A`=プロセス起動からウィンドウ検出までのms、`B`=プロセス起動から初回
//! non-NULL 観測までのms、`C`=ウィンドウ検出から初回non-NULL観測までの
//! ms——猶予期間定数の直接の根拠となる値）を標準出力に出す。3000ms 以内に
//! non-NULL にならなかった試行は `null_duration_ms=TIMEOUT` と出す。
//! 全試行後、`C` の分布（min/median/max/timeout件数）を要約する。

#![allow(unsafe_code)]

#[cfg(windows)]
mod probe {
    use awase_windows::win32::HwndExt as _;
    use std::process::{Child, Command};
    use std::time::{Duration, Instant};
    use windows::core::BOOL;
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::System::Threading::GetCurrentProcessId;
    use windows::Win32::UI::Input::Ime::ImmGetDefaultIMEWnd;
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowThreadProcessId, IsWindowVisible,
    };

    const POLL_INTERVAL: Duration = Duration::from_millis(2);
    const WINDOW_FIND_TIMEOUT: Duration = Duration::from_secs(3);
    const IME_WND_TIMEOUT: Duration = Duration::from_secs(3);

    struct FindByPidCtx {
        target_pid: u32,
        found: Option<HWND>,
    }

    extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: lparam はこの関数の唯一の呼び出し元 (find_window_for_pid) が
        //         スタック上の FindByPidCtx へのポインタとして渡している。
        //         EnumWindows のコールバック規約どおり、列挙が終わるまで
        //         そのスタックフレームは生存している。
        let ctx = unsafe { &mut *(lparam.0 as *mut FindByPidCtx) };
        let mut pid: u32 = 0;
        // SAFETY: hwnd は EnumWindows がこのプロセスに渡す有効なウィンドウ
        //         ハンドル。pid はスタック上の有効な u32 へのポインタ。
        unsafe { GetWindowThreadProcessId(hwnd, Some(&raw mut pid)) };
        if pid == ctx.target_pid {
            // SAFETY: hwnd は EnumWindows が渡した有効なハンドル。
            let visible = unsafe { IsWindowVisible(hwnd) }.as_bool();
            if visible {
                ctx.found = Some(hwnd);
                return BOOL(0); // 列挙を打ち切る
            }
        }
        BOOL(1) // 継続
    }

    fn find_window_for_pid(pid: u32, timeout: Duration) -> Option<(HWND, Duration)> {
        let start = Instant::now();
        loop {
            let mut ctx = FindByPidCtx {
                target_pid: pid,
                found: None,
            };
            // SAFETY: enum_proc はこのファイル内で定義した正しい ABI
            //         (extern "system") のコールバック。ctx はこのスコープの
            //         スタック上に生存しており、EnumWindows は同期的に
            //         コールバックを呼び終えてから返るため参照は有効。
            let _ = unsafe { EnumWindows(Some(enum_proc), LPARAM(&raw mut ctx as isize)) };
            if let Some(hwnd) = ctx.found {
                return Some((hwnd, start.elapsed()));
            }
            if start.elapsed() > timeout {
                return None;
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    }

    /// `imm.rs::get_ime_wnd` と同一の呼び出し（本スパイクは awase 本体の
    /// 学習ロジックを経由しない独立測定のため、あえて同じ1行を再実装する）。
    fn get_ime_wnd(hwnd: HWND) -> Option<HWND> {
        // SAFETY: hwnd は呼出元が EnumWindows で確認済みの有効なウィンドウ
        //         ハンドル。ImmGetDefaultIMEWnd は副作用のない問い合わせ。
        unsafe { ImmGetDefaultIMEWnd(hwnd) }.non_null()
    }

    struct RunResult {
        window_found: Duration,
        first_nonnull: Option<Duration>,
        null_duration: Option<Duration>,
    }

    fn run_once(exe_path: &str) -> anyhow::Result<RunResult> {
        let mut child: Child = Command::new(exe_path)
            .arg("--bug-report")
            .spawn()
            .map_err(|e| anyhow::anyhow!("spawn {exe_path} failed: {e}"))?;
        let pid = child.id();
        let process_start = Instant::now();

        let found = find_window_for_pid(pid, WINDOW_FIND_TIMEOUT);
        let Some((hwnd, window_found_elapsed)) = found else {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("window not found within {WINDOW_FIND_TIMEOUT:?}");
        };
        let window_found_at = Instant::now();

        let mut first_nonnull = None;
        let mut null_duration = None;
        loop {
            if get_ime_wnd(hwnd).is_some() {
                first_nonnull = Some(process_start.elapsed());
                null_duration = Some(window_found_at.elapsed());
                break;
            }
            if window_found_at.elapsed() > IME_WND_TIMEOUT {
                break;
            }
            std::thread::sleep(POLL_INTERVAL);
        }

        let _ = child.kill();
        let _ = child.wait();

        Ok(RunResult {
            window_found: window_found_elapsed,
            first_nonnull,
            null_duration,
        })
    }

    // `main` の `anyhow::Result<()>` イディオムに合わせるため、実際には常に
    // `Ok` でも戻り値型を維持する。
    #[allow(clippy::unnecessary_wraps)]
    pub(super) fn run() -> anyhow::Result<()> {
        let args: Vec<String> = std::env::args().collect();
        let exe_path = args
            .get(1)
            .cloned()
            .unwrap_or_else(|| "target/release/awase-settings.exe".to_string());
        let iterations: usize = args
            .get(2)
            .and_then(|s| s.parse().ok())
            .unwrap_or(10);

        // SAFETY: 引数なし、常に安全。自プロセスの pid をログに残すだけ
        //         （EnumWindows が自分自身のウィンドウを誤って拾わないことの
        //         裏取り用、本スパイクは非表示ウィンドウを作らないため実際には
        //         起きないが記録として出す）。
        let self_pid = unsafe { GetCurrentProcessId() };
        println!("# spike_bug112_ime_wnd_race_probe self_pid={self_pid} exe_path={exe_path} iterations={iterations}");

        let mut durations: Vec<u128> = Vec::new();
        let mut timeouts = 0usize;

        for run in 1..=iterations {
            match run_once(&exe_path) {
                Ok(r) => {
                    let null_str = r.null_duration.map_or_else(
                        || {
                            timeouts += 1;
                            "TIMEOUT".to_string()
                        },
                        |d| d.as_millis().to_string(),
                    );
                    let first_nonnull_str = r
                        .first_nonnull
                        .map_or_else(|| "TIMEOUT".to_string(), |d| d.as_millis().to_string());
                    println!(
                        "run={run} window_found_ms={} first_nonnull_ms={first_nonnull_str} null_duration_ms={null_str}",
                        r.window_found.as_millis(),
                    );
                    if let Some(d) = r.null_duration {
                        durations.push(d.as_millis());
                    }
                }
                Err(e) => {
                    println!("run={run} ERROR={e}");
                }
            }
            std::thread::sleep(Duration::from_millis(300));
        }

        durations.sort_unstable();
        println!("# --- summary ---");
        println!(
            "# resolved={} timeout={} total={}",
            durations.len(),
            timeouts,
            iterations
        );
        if !durations.is_empty() {
            let min = durations[0];
            let max = durations[durations.len() - 1];
            let median = durations[durations.len() / 2];
            println!("# null_duration_ms: min={min} median={median} max={max}");
        }

        Ok(())
    }
}

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    probe::run()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("このスパイクは Windows 専用です。");
}
