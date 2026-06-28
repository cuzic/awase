# ADR-059: 自動起動: schtasks → HKCU\Run レジストリへの移行

## ステータス

採用済み（2026-06-25 実装、commit 0d09d80 + 584897a）

## コンテキスト

v1.4.x 以前の自動起動は `schtasks.exe` 経由で Task Scheduler にタスクを登録していた。

```rust
// 旧実装（抜粋）
Command::new("schtasks")
    .args([
        "/create", "/tn", TASK_NAME, "/tr", exe_path,
        "/sc", "onlogon", "/rl", "limited", "/delay", "0000:30", "/f",
    ])
    .creation_flags(CREATE_NO_WINDOW)
    .output();
```

この方式には 3 つの問題があった。

1. **起動遅延が固定 30 秒**  
   デスクトップシェルが完全に初期化される前にトレイ登録を試みると失敗するため、
   `/delay 0000:30` で 30 秒後に起動させていた。

2. **schtasks 呼び出しのオーバーヘッド**  
   schtasks はコンソールアプリなので GUI から呼ぶとウィンドウが一瞬表示される。
   `CREATE_NO_WINDOW` で抑制していたが、外部プロセス生成自体が重い。
   また GPO 制限や Task Scheduler サービスの状態に依存するため、
   環境によっては登録・削除が失敗した。

3. **ZIP 配布との相性の悪さ**  
   インストーラなし（ZIP 展開）で配布する際に、
   タスクスケジューラのエントリが残骸として残りやすく管理が煩雑だった。

## 決定

### HKCU\Run レジストリキーへの移行

`HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run` に
REG_SZ エントリを書き込む方式に変更した。Win32 Registry API を直接呼ぶため
外部プロセス生成は不要で、ユーザー権限のみで動作する。

```rust
// crates/awase-windows/src/autostart.rs
const RUN_SUBKEY: windows::core::PCWSTR =
    windows::core::w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const VALUE_NAME: windows::core::PCWSTR = windows::core::w!("awase");

pub fn register() -> bool {
    let exe_wide: Vec<u16> = exe_str.encode_utf16().chain(std::iter::once(0)).collect();
    let byte_len = u32::try_from(exe_wide.len() * 2).unwrap_or(u32::MAX);
    unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER, RUN_SUBKEY, VALUE_NAME,
            REG_SZ.0, Some(exe_wide.as_ptr().cast()), byte_len,
        )
    }.is_ok()
}
```

### 起動遅延の撤廃: TaskbarCreated メッセージで代替

シェル未起動時に `Shell_NotifyIcon NIM_ADD` が失敗しても、
`tray.rs` では従来エラーとして扱っていたのを **警告に格下げ** した。
シェルが起動した後に届く `TaskbarCreated` メッセージで既存の `recreate()` が
呼ばれ、トレイアイコンが正常に登録される。これにより 30 秒遅延が不要になった。

### 旧タスクの自動削除移行コード

v1.4.x 以前のユーザーが更新した場合、Task Scheduler に旧タスクが残る。
`migrate_from_schtasks()` を起動時（`handle_auto_start()` の先頭）で毎回呼び、
旧タスクを静かに削除する。

```rust
// crates/awase-windows/src/app/bootstrap.rs
pub(super) fn handle_auto_start(config: &mut awase::config::AppConfig) {
    // 旧バージョン（schtasks 方式）からの移行: 古いタスクが残っていれば削除する
    autostart::migrate_from_schtasks();
    // ...
}
```

```rust
// crates/awase-windows/src/autostart.rs
pub fn migrate_from_schtasks() {
    let output = Command::new("schtasks")
        .args(["/delete", "/tn", TASK_NAME, "/f"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    // タスクが存在しない場合（移行済み・新規インストール）は何もしない
}
```

`auto_start = "disabled"` のユーザーにも確実に移行が適用されるよう、
設定値の分岐より前に呼ぶ設計とした。

## なぜこの設計か / 検討した代替案

| 案 | 評価 |
|---|---|
| HKCU\Run レジストリ（採用） | ユーザー権限のみ・即時起動・外部プロセス不要 |
| schtasks 継続 | 30 秒遅延・GPO 依存・ZIP 配布困難 |
| COM `ITaskService` | schtasks と同等の問題、実装が複雑 |
| スタートアップフォルダへのショートカット | Shell API が必要、パス管理が煩雑 |

HKCU\Run はすべての Windows ユーザーアプリが採用する標準的な手法であり、
管理者権限不要・即時起動・Win32 API 直接操作という点で最も適切と判断した。

## 結果

- 自動起動の登録・削除が Win32 Registry API 直接呼び出しになり、
  外部プロセス生成ゼロで完結する
- 30 秒の起動遅延が撤廃され、ログオン直後に awase が起動するようになった
- GPO や Task Scheduler サービスの状態に依存しなくなった
- 旧 schtasks タスクは初回起動時に自動削除されるため、
  ユーザーが手動でクリーンアップする必要がない

## 関連 ADR

- ADR-052: トレイパニックリセット（`TaskbarCreated` による recreate の設計）
