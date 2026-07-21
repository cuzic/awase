// コンソールウィンドウを非表示にする（タスクトレイで操作する）
#![windows_subsystem = "windows"]
// Win32 API (MessageBoxW 等) の使用に unsafe が必須。lib.rs と同じ方針。
#![allow(unsafe_code)]
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
        let detail = format!("{e:#}");
        eprintln!("Error: {detail}");
        show_startup_error(&detail);
        std::process::exit(1);
    }
}

/// 起動失敗の原因をメッセージボックスで表示する。
///
/// `#![windows_subsystem = "windows"]` によりコンソールが無く、`eprintln!` は
/// 誰の目にも触れない。ダブルクリックしても何も起きないように見えるだけの
/// 事故を防ぐため、エラー内容とよくある原因への対処法をダイアログで示す。
#[cfg(windows)]
fn show_startup_error(detail: &str) {
    use windows::core::{w, PCWSTR};
    use windows::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, MB_ICONERROR, MB_OK, MB_SETFOREGROUND, MB_TOPMOST,
    };

    let hint = startup_error_hint(detail);
    let text = format!("awase の起動に失敗しました。\n\n{detail}\n\n{hint}");
    let text_wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();

    // SAFETY: text_wide は NUL 終端済み UTF-16 で呼び出し中有効。タイトルは静的リテラル。
    unsafe {
        let _ = MessageBoxW(
            None,
            PCWSTR(text_wide.as_ptr()),
            w!("awase - 起動エラー"),
            MB_OK | MB_ICONERROR | MB_TOPMOST | MB_SETFOREGROUND,
        );
    }
}

/// エラー内容から、よくある原因に対する具体的な対処法を返す。
///
/// 文字列は `bootstrap.rs` の各 `.context(...)` 呼び出しに合わせて分類している。
/// メッセージの文言を変更した場合はこちらも合わせて見直すこと。
#[cfg(windows)]
fn startup_error_hint(detail: &str) -> &'static str {
    if detail.contains("Failed to parse") && detail.contains(".toml") {
        "→ config.toml の TOML 構文に誤りがあります。上記メッセージの行番号・列番号と \
         「^」の位置を確認し、クォートや括弧の閉じ忘れ、カンマの過不足を修正してください。\n\
         同梱の config.sample.toml と見比べると原因を特定しやすくなります。"
    } else if detail.contains("Config file not found") {
        "→ config.toml が見つかりません。awase.exe と同じフォルダに config.toml を \
         置いてください（同梱の config.sample.toml をコピーして使えます）。"
    } else if detail.contains("Unknown VK name") {
        "→ config.toml のキー名の指定に誤りがあります。上記メッセージが示す設定項目を、\
         有効なキー名（例: VK_MUHENKAN, VK_CONVERT 等）に修正してください。"
    } else if detail.contains("Failed to install keyboard hook") {
        "→ セキュリティソフト（ウイルス対策ソフト）がキーボード監視機能をブロックしている \
         可能性があります。awase.exe をセキュリティソフトの除外リストに追加してから \
         再度起動してみてください。"
    } else if detail.contains("Failed to create system tray icon") {
        "→ タスクトレイアイコンの作成に失敗しました。awase.exe がタスクマネージャー上で \
         既に起動していないか確認し、PC を再起動してから再度お試しください。"
    } else {
        "→ 詳しいログは awase.exe と同じフォルダの awase.log に記録されています。\n\
         解決しない場合は awase.log の内容を添えて GitHub Issue でご報告ください。"
    }
}
