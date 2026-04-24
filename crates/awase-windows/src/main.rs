// コンソールウィンドウを非表示にする（タスクトレイで操作する）
#![windows_subsystem = "windows"]
// Win32 API (フック, SendInput, SetTimer 等) の使用に unsafe が必須
#![allow(unsafe_code)]
// Win32 API の型キャスト (usize → i32 等) は OS の ABI 制約により不可避
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    // SingleThreadCell は &self → &mut T を返すが、シングルスレッド保証下で安全
    clippy::mut_from_ref,
    // コールバック型定義が複雑になるのは Win32 API の設計上避けられない
    clippy::type_complexity
)]

// 非 Windows ではスタブのみ
#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
mod app;

#[cfg(windows)]
fn main() {
    if let Err(e) = app::run() {
        eprintln!("Error: {e:#}");
        std::process::exit(1);
    }
}
