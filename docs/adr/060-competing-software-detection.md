# ADR-060: 競合ソフトウェア起動時チェック

## ステータス

採用済み（2026-06-25 実装、commit 9875c2c）

## コンテキスト

やまぶき・やまぶきR・紅皿などの親指シフト対応 IME／エミュレータは、
awase と同様にキーボードフックを使って打鍵を横取りする。
これらが同時に動いている場合、同一キーイベントが両アプリに渡り
「キー入力が二重になる」「片仮名が出力される」など動作が壊れる。

ユーザーから見ると原因が分かりにくい症状であり、サポート負荷も高い。
起動時にこの状況を検知してユーザーに知らせる仕組みが必要だった。

## 決定

起動シーケンス（`run_all()`）の設定読み込み完了直後に
`check_conflicting_software(&mut diag)` を呼び出し、
Win32 ToolHelp API でプロセス一覧を走査して競合ソフトを検出する。

### 検出対象

```rust
const CONFLICTS: &[ConflictEntry] = &[
    ConflictEntry { exe: "yamabuki.exe",  display: "やまぶき" },
    ConflictEntry { exe: "yamabukiR.exe", display: "やまぶきR" },
    ConflictEntry { exe: "benizara.exe",  display: "紅皿" },
];
```

### プロセス列挙の実装

`CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)` でシステム全プロセスの
スナップショットを取り、`Process32FirstW` / `Process32NextW` でループして
`szExeFile`（UTF-16）を大文字小文字無視で比較する。

```rust
let exe_name = String::from_utf16_lossy(&entry.szExeFile[..end]);
for conflict in CONFLICTS {
    if exe_name.eq_ignore_ascii_case(conflict.exe) {
        results.push(conflict.display);
        break;
    }
}
```

スナップショット取得失敗時は警告なしで早期 `return`（権限不足等の環境への配慮）。

### 警告の配信

競合を発見するたびに `diag.warn(...)` を呼ぶ。
`StartupDiagnostics` は警告を蓄積し、`diag.report()` 時点でシステムトレイの
バルーン通知として一括表示する（"N 件の警告があります"）。
ログ（`awase.log`）にも `warn!` レベルで記録される。

**強制終了は行わない。** 判断はユーザーに委ねる。

## なぜこの設計か / 検討した代替案

| 案 | 却下理由 |
|---|---|
| 競合プロセスを強制 kill する | ユーザーの同意なく他プロセスを終了するのは不適切 |
| サービス起動時のみチェック | awase は通常アプリとして動くためサービス前提は不自然 |
| レジストリの「Run」キーを調べる | 登録されていても起動中とは限らず、未登録でも起動できる |
| ウィンドウクラス名で検索 | 各ソフトのクラス名は非公開で変わりやすい |

ToolHelp スナップショットはカーネルが一時点のコピーを返す API であり、
列挙中に対象プロセスが終了してもクラッシュしない安全な設計になっている。
また管理者権限なしで全ユーザーのプロセス名を取得できる。

## 結果

- 競合ソフトを同時起動した状態で awase を起動すると、
  トレイバルーン「N 件の警告があります」と awase.log への記録で通知される
- 強制終了を伴わないため、意図して併用テストをしているユーザーの作業を妨げない
- `StartupDiagnostics` の共通パスに乗るため、将来の競合ソフト追加は
  `CONFLICTS` 配列への 1 エントリ追記だけで済む

## 関連 ADR

- ADR-043: アプリ配信プロファイル（起動時チェック群の位置付け）
- ADR-052: トレイパニックリセット（バルーン通知を用いる別の起動時警告）
