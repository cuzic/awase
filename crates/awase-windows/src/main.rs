// コンソールウィンドウを非表示にする（タスクトレイで操作する）
#![windows_subsystem = "windows"]
// Win32 API の型キャスト (usize → i32 等) は OS の ABI 制約により不可避
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::mut_from_ref,
    clippy::type_complexity
)]

// 非 Windows ではスタブのみ
#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
fn main() {
    if let Err(e) = awase_windows::run() {
        eprintln!("Error: {e:#}");
        std::process::exit(1);
    }
}
