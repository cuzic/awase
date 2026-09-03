# ADR-124: タスクトレイからの更新確認

## ステータス

採用・実装中（2026-09-03）。Phase 2 の Cloudflare Worker は別プロセスで実装、Rust 側は
本ADRに従い `awase.exe` が直接通信しない形で実装する。

## コンテキスト

awase はトレイ常駐のキーボードフックプロセスを持つ。ここへ定期的な外部通信を足すと、
未署名バイナリが「低レベルフック + ネットワーク」を併せ持つことになり、AV/EDR の
ヒューリスティック上も、プライバシー説明上も不利になる。一方、不具合報告は古い版から
届きやすく、ユーザーが更新に気付ける導線は必要だった。

## 決定

### 1. Worker 側 API 設計

`GET /v1/latest-release` を `services/report-worker` に追加する。レスポンスは
`schema_version`、`latest_version`、`checked_at`、`stale` のみとし、
`download_url`、`tag`、`released_at`、`release_url` は返さない。クライアントは
検証済み SemVer から GitHub リリースページ URL をローカル生成する。

Worker は GitHub の latest release API を `User-Agent: awase-update-check-worker (+https://awase.cc)`、
`Accept: application/vnd.github+json`、`X-GitHub-Api-Version: 2022-11-28` 付きで呼ぶ。
キャッシュは新規KV namespaceを作らず、既存 `RATE_LIMIT_KV` に `latest-release:` prefix で
相乗りする。fresh は即返し、stale は `ctx.waitUntil()` で再検証、empty は同期取得する。

### 2. 通信主体とトリガー

`awase.exe` はネットワークに一切触れない。更新確認は `awase-settings.exe --check-update` が
WinHTTPで実行する。トリガーはトレイ右クリックのみで、自動ポーリングや新規 `WM_TIMER` は
追加しない。右クリック時に `update_check.json` を読み、試行可能なら設定アプリを
fire-and-forgetで起動する。

### 3. 状態と表示

永続状態は exe 隣接の `update_check.json` に置く。保存するのは
`last_attempted_at`、`last_success_at`、`last_seen_latest` と `schema_version` のみ。
表示状態は保存せず、`display()` が `Disabled`、`NeverSucceeded`、`NoUpdate`、`Available`
を毎回導出する。ファイルが壊れている、読めない、schema不一致の場合は default 扱いにする。

### 4. バージョン比較

`src/version.rs` に awase 用 SemVer パーサと `Ord` を置く。タグ接頭辞 `v` を受け付け、
build metadata は比較から除外し、prerelease は SemVer 規則で比較する。URLに使う文字列は
`SemVer` の `Display` 出力だけに限定する。

### 5. UI 詳細

トレイメニューは `Available` のときだけ更新項目を表示する。Aboutダイアログは毎回
`update_check.json` を読み直し、4つの表示状態に応じて文面を出し分ける。「はい」は
通常はホームページ、更新ありの場合は `RELEASE_TAG_URL_PREFIX + SemVer` のリリースページを開く。

### 6. プライバシーとオプトアウト

設定に `general.update_check` を追加し、既定値は `true` とする。OFFの場合、右クリックしても
`--check-update` は起動せず通信しない。送るHTTPリクエストには awase のバージョン、OS、
端末固有情報を含めない。ただし接続元IPアドレスは Cloudflare の標準アクセスログに残る。
機械横断のオプトアウトは持たず、`config.toml` は per-user の設定として扱う。

### 7. 失敗時とエッジケース

GitHub、Worker、DNS、TLS、プロキシ、WinHTTPが失敗しても入力処理には影響させない。
成功済みの状態があれば古い結果を表示し続け、空なら「未確認」として扱う。HTTP前に
`last_attempted_at` を保存し、失敗時は `last_success_at` と `last_seen_latest` を保持する。
短時間の多重起動は `should_attempt()` と `Global\awase_update_check` Mutex で抑制する。

### 8. テスト計画

コアは Linux で `version` と `update_state` の純粋関数をテストする。Worker は fresh/stale/empty、
`waitUntil`、GitHubヘッダ、HEAD/405、不要フィールド除外を vitest で固定する。Windows実機では
`awase-settings.exe --check-update`、二重起動、ログ、`update_check.json`、トレイ右クリック、
About表示、`awase.exe` に `winhttp.dll` がロードされないことを確認する。

### 9. リリースフローへの影響

リリース後は `curl -s https://report.awase.cc/v1/latest-release` で新バージョンが反映されて
いるか確認する。Workerのsoft TTLにより最大1時間遅れる。即時反映が必要な場合はKVの
`latest-release:v1` を削除して再取得させる。

## 既知の限界

1. ユーザーがトレイを一度も右クリックしなければ、更新確認は一度も走らない。
2. 右クリック時の表示は1回前のチェック結果であり、その回のspawn結果は次回以降に反映される。
3. 恒久ブロック環境では、15分ゲートを超えた右クリックごとに `awase-settings.exe` が起動されうる。
4. `awase-settings.exe` の `CreateProcess` 自体の失敗は警告が出る。子プロセスが起動後即座に
   落とされる場合のみ警告が出ない。
5. アンインストール後に `update_check.json` が残る。
6. 機械横断のオプトアウト手段が無い。企業展開ではユーザーごとの設定配置かネットワーク遮断で扱う。
7. Scoop更新では `update_check.json` が消え、未チェックに戻る。
8. Workerのリリースキャッシュは用途名と一致しない `RATE_LIMIT_KV` に相乗りする。
9. `--check-update` の所要時間に厳密な上限は無い。WPAD探索は WinHTTP timeout に含まれない。
10. インストール経路を検出せず、ScoopユーザーにもGitHubリリースページを見せる。
11. リリースページURL形式をクライアントが決め打ちする。タグ規則変更時はCIで検出する。
12. `checked_at` と `stale` は診断用で、クライアントUIは読まない。
13. 失敗回数や失敗理由は永続化しない。詳細は `awase-settings.log` を見る。
14. 直近の試行が失敗したかをUIでは区別しない。要望が出た時点で `Display` 拡張を検討する。
15. `update_check.json` を書けない環境（例: 非昇格で `C:\Program Files\awase` 等の書き込み
    不可なディレクトリに展開したZIP）では、`last_attempted_at` が前に進まないため右クリック
    のたびに `awase-settings.exe` が起動され続ける（HTTPも毎回飛ぶ。Mutexで同時実行数だけは
    1に制限される）。MSIはper-userインストールなので該当しない。
16. `awase-settings.exe --check-update` はexe隣接の`config.toml`のみを読む。`awase.exe`を
    位置引数でconfigパスを指定して起動している場合（`find_config_path`参照）、その設定と
    子プロセスが実際に読む設定が食い違いうる。
17. 右クリックからの `awase-settings.exe` 起動（`CreateProcessW`）自体の所要時間は評価して
    いない。未署名exeへのAVリアルタイムスキャンが重い環境では、ゲートが開いた直後の右クリック
    でコンテキストメニュー表示が体感できる程度に遅れることがある（キー入力への影響はない —
    `WH_KEYBOARD_LL`は専用スレッドでatomicキャッシュのみを見るため）。

## Verification

- `cargo check --target x86_64-pc-windows-msvc -p awase -p awase-windows -p awase-settings`
- `cargo test -p awase --lib`
- `pnpm -C services/report-worker typecheck && pnpm -C services/report-worker test`
- `cargo fmt -- --check`
- `cargo clippy --target x86_64-pc-windows-msvc -p awase -- -A clippy::cargo_common_metadata -D warnings`
- `cargo clippy --target x86_64-pc-windows-msvc -p awase-windows -- -D warnings`

Windows実機確認はCIでは代替できないため、PR後に別途実施する。

## レビュー指摘の処理状況

rev.4まではWorker APIとSemVer比較が中心だった。M-1でWorker→GitHubのUser-Agent必須化、
M-2でWorker stale-while-revalidate と `ExecutionContext` を採用した。m-4〜m-7では新規KVを
やめ、未使用レスポンスフィールドを削除し、`Allow: GET, HEAD` に直した。

rev.5〜rev.11では常駐プロセス内の定期ポーリング、バックオフ、in-flight guard、
永続状態復元を検討したが、レビューで「フックプロセス + ネットワーク」の構造リスク、
panic/early return時のguard漏れ、状態復元漏れ、恒久ブロック時のリトライ過多が繰り返し
指摘された。

rev.12で通信主体を `awase-settings.exe` へ移し、自動ポーリングを廃止してトレイ右クリックを
唯一のトリガーにした。これにより `awase.exe` のネットワーク依存、ワーカースレッド、
新規タイマー、複雑なバックオフ状態を削除した。rev.13では子プロセス起動失敗時の既知の限界、
`schema_version` の書き手責務、Aboutで状態を読み直す方針を明記した。
