fn main() {
    // 非Windowsホスト(`cargo check --workspace`等)ではこの関数自体を参照しない。
    // `windows_reactor_setup::as_self_contained()` は `CARGO_CFG_TARGET_OS` が
    // "windows" でないと panic する仕様なので、実行時分岐ではなくコンパイル時に
    // 呼び出しごと消す。
    #[cfg(windows)]
    windows_reactor_setup::as_self_contained();
}
