//! B-1 実機検証用: EDIT サブクラス経由で `WM_IME_NOTIFY` が観測できるかを測る。
//! 段階1: 自己注入のみ（外部書き込みは 0 のはず。猶予窓 150ms の妥当性の実測）。
//! 段階2: 別プロセス（`tools/e2e/b1-notify-probe.ps1`）が外部から IME を操作し、
//! IME 通知経由の外部書き込みが検出されるか。
#[cfg(windows)]
fn main() {
    use awase_keymap_learn::exec::ImeDriver;
    use awase_keymap_learn_win::RealImeDriver;
    use std::time::{Duration, Instant};

    const KEYS: [u32; 14] = [
        0x1D, 0x1C, 0xF2, 0xF1, 0xF0, 0xF3, 0x19, 0x16, 0x1A, 0x1B, 0x0D, 0x20, 0x08, 0x41,
    ];
    let mut driver = match RealImeDriver::new(KEYS.to_vec()) {
        Ok(d) => d,
        Err(e) => {
            println!("INIT_FAILED {e}");
            std::process::exit(2);
        }
    };
    let baseline = driver.diag_notify_external_count();
    println!("BASELINE notify_external={baseline}");
    for key in [2usize, 5, 2, 5, 2, 5] {
        let report = driver.press(key);
        println!(
            "PRESS key_idx={key} delivered={} notify_external={} notify_since_mark={}",
            report.delivered,
            driver.diag_notify_external_count(),
            driver.diag_notify_since_mark()
        );
    }
    let phase1 = driver.diag_notify_external_count();
    println!("PHASE1 notify_external={}", phase1 - baseline);
    driver.diag_pump(Duration::from_millis(500));
    println!("WAIT_EXTERNAL");
    let deadline = Instant::now() + Duration::from_mins(1);
    let mut last = phase1;
    while Instant::now() < deadline {
        driver.diag_pump(Duration::from_millis(100));
        let now = driver.diag_notify_external_count();
        if now != last {
            println!("EXTERNAL_NOTIFY notify_external={now}");
            last = now;
        }
        if std::path::Path::new("notify-probe.stop").exists() {
            break;
        }
    }
    println!(
        "PHASE2 notify_external={}",
        driver.diag_notify_external_count() - phase1
    );
}

#[cfg(not(windows))]
fn main() {}
