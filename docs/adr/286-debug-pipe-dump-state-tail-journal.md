---
id: ADR-286
title: |-
  実行中の awase の内部状態を、既存インスタンスへのメッセージ要求とファイル出力で読み取り専用に観測する(dump-state・tail-journal)
summary: |-
  再現が稀な不具合(BUG-219・225・226 など)は、再現した瞬間の状態を後から取り戻せない。いまの観測は awase.log と、トレイ操作や不具合報告のときに書き出す journal ファイルだけで、再現中の実機から状態を問い合わせる口が無い。
  決定(案): (1) 既存インスタンスの隠しウィンドウ(多重起動通知と同じ `FindWindowW`)へ `WM_COPYDATA` で要求を送る。新しい thread・pipe・ポートは作らない。(2) 要求は 2 つだけ: `dump-state`(belief・観測・focus・フック生存などの小さな JSON)と `tail-journal`(seq の続きから最大 N 件)。(3) 処理は既存のメッセージループ上(`WM_DUMP_JOURNAL` と同じ規律)で行い、結果は awase.log と同じ場所の固定名ファイルへ書く。クライアントは `SendMessageTimeoutW` の復帰を待ってから読む。(4) 出す値はキーコード・状態・クラス名・プロセス名まで。打鍵した文字・ウィンドウタイトル・本文は出さない。(5) 書き込み系・購読・REPL は範囲外。継続的な購読が要るようになったら named pipe を段階 2 の候補にする。
status: |-
  起草(2026-10-31、named pipe 案から書き直し)。所有者の決定 4 件を反映済み(下記)。未実装。
related_adr:
  - "ADR-082"
  - "ADR-095"
  - "ADR-169"
  - "ADR-222"
  - "ADR-225"
  - "ADR-250"
---

# ADR-286: 実行中の awase の内部状態を、メッセージ要求とファイル出力で読み取り専用に観測する

## 背景

- 再現が稀な不具合では、再現した直後に調べたくても状態が残らない。BUG-226(LINE の Mozc で cold の最初の `ko` が `kおちら`)は、所有者が再現しないまま様子見になっている(2026-10-28 時点)。BUG-219(打鍵中に belief が約 1 秒 OFF)は、IMM の偽の「閉」の出所が未解明のまま。どちらも「再発したら時刻を聞いて awase.log を見る」運用になっている。
- いまの観測の口は 3 つで、どれも「書き出す操作をしたときの 1 回きりのスナップショット」である。
  - `awase.log`(`app/logging.rs`、20MB で 1 世代ローテーション)。
  - journal の ring(`awase-ime-core/src/journal.rs`、4 レーン: State・Timing・Actuation・KeyInput)。トレイの操作 `WM_DUMP_JOURNAL` で `%TEMP%/awase_journal_<tick>.json` に書く(`journal_dump.rs`)。
  - 不具合報告(ADR-095・ADR-222)。ring の中身を gzip して送る。
- 診断画面用に `Runtime::diagnostic_snapshot()`(`RuntimeDiagnosticSnapshot`)が既にあるが、フォーカス pid・クラス・shadow の ON/OFF・最後のフック活動時刻・プロファイルの 8 項目だけで、belief と観測の食い違いを見るには足りない。
- 実機の調査はいま `clipwire exec` で PowerShell を回して行う。常駐の awase に「いまの状態」を聞く手段が無いので、ログに出ていない値は取れない。
- **既存の足場**: 多重起動の通知は、`tray::WINDOW_CLASS_NAME` の隠しウィンドウを `FindWindowW` で探して `WM_DUPLICATE_INSTANCE` を送る形で既に動いている(`app/bootstrap.rs`)。外から常駐インスタンスへ要求を送る道は、新しく作らなくても手元にある。
- ADR-225・226(journal・CI 実機・閉ループの連携)は、実害を測らずに広げた結果、大半を見送った。本 ADR は同じ轍を避けるため、**最小の 2 要求だけ**を決め、それ以外は実際に困った事例が出てから別 ADR にする。
- 前提: awase は通常、管理者権限なしで動く(`tray::is_elevated()`。所有者に確認、2026-10-31)。

## 決定(案)

### D1. 口は「隠しウィンドウへの `WM_COPYDATA`」

- クライアントは `FindWindowW(WINDOW_CLASS_NAME)` で既存インスタンスを探し、`SendMessageTimeoutW(.., WM_COPYDATA, .., SMTO_ABORTIFHUNG, 3000ms)` で要求を送る。
- `COPYDATASTRUCT.dwData` が要求の種類(`1 = dump-state`、`2 = tail-journal`)、`lpData` が JSON のパラメータ(`tail-journal` の `since_seq`・`limit`・`lanes`)。
- 受け側は隠しウィンドウのウィンドウプロシージャで `WM_COPYDATA` を処理する。**ハンドラが結果をファイルへ書き終えてから返る**ので、クライアントは `SendMessageTimeoutW` の復帰後にファイルを読めばよく、ポーリングは要らない。タイムアウトは送信側が持つので、メインスレッドが詰まっていても呼び出し側は戻れる。
- 受け側は常時有効にする(所有者決定。状態を変えない読み取り専用で、フックのコールバックの外)。既定 OFF の設定は置かない。そのかわり D4 の出さないものを守る。
- 新しい thread・ACL・ポートは持たない。同じ現在ユーザーのセッション内の別プロセスからだけ届く(UIPI により、より低い整合性レベルのプロセスからは届かない)。管理者権限で awase を動かす場合は、非管理者のクライアントからの `WM_COPYDATA` が UIPI で落ちる。最初は入れない(所有者決定 3)。必要が出たときの対応は `ChangeWindowMessageFilterEx` で当該メッセージを許可するか、管理者権限のクライアントから送る。通常運用(非管理者)では問題にならない。
- 新しい実行ファイルは足さない。クライアントは PowerShell(`user32` の P/Invoke)と、`clipwire` の target 定義にする。`awase.exe debug <cmd>` の副コマンドは必要が出てから別 ADR。

### D2. 要求と出力(JSON、schema 1)

```
dump-state      → <awase.log と同じ場所>/awase_debug_state.json
tail-journal    → <同>/awase_debug_journal_tail.json
```

```
{"schema":1,"uptime_ms":..,"focus":{..},"belief":{..},"observation":{..},"hook":{..},"profile":{..}}

{"schema":1,"next_seq":12550,"evicted":{"state":0,..},"oldest_elapsed_ms":{..},"entries":[{seq,elapsed_ms,entry},..]}
```

- 出力先はサーバ側が決める固定名(クライアントがパスを指定しない)。上書きで 1 世代だけ持つ。直前の結果を読む前に別の要求が来ると上書きされるので、クライアントは 1 つずつ送る。
- `dump-state` の中身は、新設する**純粋な構造体 `StateDump`**(`awase-ime-core`、`Serialize`)とする。`RuntimeDiagnosticSnapshot` を広げず、診断画面用と分ける。項目の候補:
  - belief(open・input_mode・scope。採用元の証拠の優先度までは出さない、所有者決定 2)と applied(`Option<bool>`、未知を `false` に潰さない)。
  - 観測の最新値(IMM・TSF・MSAA・Mozc 共有メモリ・字形)と、それぞれの時刻・世代。
  - focus(hwnd・pid・プロセス名・クラス名・`AppKind`・プロファイル・warm/cold・idle 秒)。
  - フック(最後の活動時刻、watchdog の状態、コールバック所要時間の最大値)。
  - 進行中の保留(`pending_deferred`・probe・recovery の有無と件数)。
- `tail-journal` は `since_seq` の続きから `limit` 件(既定 200、上限 2,000)を、seq 昇順で出す。`since_seq` が ring から追い出されていれば、`evicted` と `oldest_elapsed_ms` で欠落を知らせ、残っている最古から出す(黙って欠落させない)。
- 項目名と値は journal の `Serialize` の形をそのまま使い、新しい表現を作らない。リプレイ(`journal_replay.rs`)や不具合報告の読み方がそのまま使える。

### D3. スレッドの扱い

- 単一スレッドの前提をそのまま守る。処理は `WM_COPYDATA` のハンドラ内(メインスレッド)で行い、`app.platform_state...` の借用規律は既存の `WM_DUMP_JOURNAL` と同じ。ロックも共有状態も増やさない。
- キーボードフックのコールバックの中では何もしない。
- メインスレッドで書くので、要求 1 件の処理時間だけ他のメッセージが遅れる。`tail-journal` の `limit` 上限と、ハンドラ内の所要時間のログ(`[debug-req] kind=.. entries=.. bytes=.. ms=..`)で見える化する。上限値の根拠は、実装 PR の CI 実測で定める(下記「検証の計画」)。

### D4. 出さないもの(プライバシー)

- 出す: VK・scan code・修飾・状態名・判定名・hwnd・pid・プロセス名・クラス名・時刻。journal の `KeyEventSummary`(`vk_code`・`scan_code`・`is_down`・`injected`・`key_class`・修飾・`foreign_ctrl`・`impersonated`)が既にその範囲であることを確認した(`journal.rs`)。
- 出さない: 入力された文字、IME の変換候補・composition 文字列、ウィンドウタイトル、ドキュメントの本文。2026-10-18 の事故(測定中に利用者の Notepad の本文を記録した)を再発させない。
- `StateDump` の各フィールドは文字列を取らず、列挙型か数値にする。文字列を足すのは「プロセス名・クラス名」だけにし、それ以外は ADR の追記を要する。`tests/architecture_guard.rs` に、デバッグ要求のモジュールが `GetWindowText`・`SendInput`・クリップボードを呼ばないことを走査するガードを足す。
- 出力ファイルは現在のユーザーの領域(awase.log と同じ場所)にだけ置く。`%TEMP%` の共有位置には置かない。

### D5. 範囲外(別 ADR)

- 書き込み系: キー注入、tuning 定数の上書き、設定の変更。
- 購読(`follow`)・イベントの push。**継続的な購読や往復が要るようになったら、named pipe を段階 2 の候補にする**(「検討した代替」の A)。
- REPL・スクリプト言語の埋め込み。
- `awase.exe debug` 副コマンドと、Settings 画面への表示。
- ライブとリプレイの判断の差分検査。

## 検討した代替

- **A. named pipe(`\\.\pipe\awase-debug-<SID>`、専用 worker thread が I/O)**: 要求と応答が同期で、あとから購読へ広げやすい。一方で、単一スレッドの前提への例外(専用 thread)、ACL・同時接続の管理、既定 OFF の設定と再読み込みでの開閉が増える。「再現中の実機に外から 1 回聞く」だけなら、B の方が小さい。→ 段階 2 の候補として残す(本 ADR の初稿はこれを採っていたが、既存の足場を見落としていたので書き直した)。
- **B(本 ADR)**: `WM_COPYDATA` + ファイル。
- **C. 既存の `WM_DUMP_JOURNAL` に `StateDump` を足すだけ**: 最小の変更で、不具合報告やトレイのダンプに状態が付く。外から要求できないので、B の D2 の内容を journal ダンプにも載せる形で併用する(`DumpTriggered` の前に `StateDump` を 1 件積む案は未決事項 4)。
- **D. awase.log へ状態を定期的に出す**: ログが肥大し(ADR-139 の 747MB)、何を出すかを事前に決めねばならない。→ 不採用。
- **E. ローカルの TCP ポート**: 他ユーザー・他プロセスからの到達を絞れない。→ 不採用。
- **F. 共有メモリ(Mozc の `version 5` のような)**: 読む側が構造体のレイアウトに依存し、版管理が要る。`tail-journal` の seq 続き読みに向かない。→ 不採用。
- **G. 組み込み言語の REPL**: 本 ADR では扱わない。読み取り専用の観測が先で、書き込みは実害が出てから。

## 検証の計画(受け入れ条件)

実装の PR で次を示す。数値は**未測定**で、ここでは決めない(`tuning-constants.md`: 実測してから定める)。

1. Linux 単体テスト(`awase-ime-core`): `StateDump` の `Serialize` の形(schema 1)を golden にする。`tail-journal` の seq 続き読み・追い出し済み `since_seq`・`limit` 上限・レーン絞りを表で固定する。
2. `architecture_guard`: D4 の禁止呼び出しと、フックのコールバックから要求ハンドラへ到達しないこと。
3. Windows CI(`windows-build` と実機 e2e のスモーク): 起動して `WM_COPYDATA` を送り、`dump-state` が schema 1 の JSON をファイルへ書く。`tail-journal` を `since_seq=0` と続きで 2 回送り、seq が連続する。
4. メインスレッドで要求 1 件の処理にかかる時間を、`dump-state` と `tail-journal`(limit 200・2,000)で CI に測らせてログに残し、`limit` の上限の根拠にする。打鍵中(1 分 475 件の速さ、ADR-222 の実測)にフックの所要時間の最大値が前後で変わらないこと。

## 所有者の決定(2026-10-31)

1. 受け側は**常時有効**(読み取り専用・同一ユーザーのみ・D4 の範囲)。既定 OFF の設定は置かない。
2. `dump-state` の belief は**採用した値と時刻・世代のみ**(open・input_mode・scope と、各観測の最新値・時刻・世代)。採用元の証拠の優先度までは出さない。`ime_model.rs` に足す読み取り専用の view はこの範囲に留める。必要が出てから広げる。
3. 管理者権限で動かす利用者向けの `ChangeWindowMessageFilterEx` は**必要が出てから**。
4. 不具合報告・トレイの journal ダンプへの `StateDump` の付加は**別 PR**。報告の schema とサイズ(ADR-222)への影響を測ってから決める。本 ADR の範囲は外から聞く口だけ。

## 未決事項

- 複雑性予算(`complexity-budget.md`)は未発効だが、`WM_COPYDATA` の入口を 1 つ足す。撤去する対の候補があるか(実装 PR で確認)。

## 関連

- ADR-082(journal の構造化)・ADR-169(KeyInput の畳み込み)・ADR-222(ring の全ダンプ)・ADR-250(境界で取る journal)。
- ADR-225・226(journal・CI 実機の連携を広げすぎた先例)。
