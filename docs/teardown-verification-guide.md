# 機構撤去の動作確認ガイド（CI・実機・純粋テストの使い分け）

[ADR-191](adr/191-ime-is-source-of-truth-observe-not-write.md)（IMEを状態の正とし、awaseは書き込まず観測に追随する）の
撤去作業で、「この機構を消しても壊れないか」を確かめる手段の地図。撤去そのものは別ブランチで進める前提で、
本書は**確認の仕組みと、その調査結果**だけを扱う（撤去の設計判断は ADR-191 決定5・6）。

書いた時点（2026-09-21）の `develop`（116eebdc）の実装に基づく。`develop` に無い手段（別セッションの
`ci/e2e-fastnotify` の検証結果）を引いた箇所には「（未マージ）」と書いた。

## 1. 撤去の確認が証明すべきこと

撤去は「追加」と違い、壊れたときに**何が壊れたのかが見えにくい**（書き込みが消えるだけで、エラーは出ない）。

**合否の基準は「撤去前と同じ挙動」ではない。** ADR-191 は、旧挙動（awase の書き込みで決まる部分）を**誤りとして測定**しており
（awase 経由の一段予測 84.5% に対し、IME単体は仕様どおり 98.5%）、そこは変わるのが正しい。基準は、設計から導いた
**不変条件と期待結果**に置く。本書は確認の手段を扱い、期待結果そのものは
[ime-passive-model-expected-results.md](ime-passive-model-expected-results.md) にある（不変条件 INV-1〜7、旧挙動から変わるべき点、
撤去候補ごとの撤去後の期待）。**諦めるキー**（IME 内部状態に依存し、設定の取得・学習の表で「決定できない」と判定されたもの）も、そこの §4 にまとめてある。

`e2e-ime.yml` の `expect` 列（`pass`/`fail`/`observe`）は、ADR-186 の撤去実験（E1〜E7b）で使った**旧設計向けの語彙**である。
`fail`（消すと壊れる）は「必要性の固定」だった。受動モデルでは機構を消すこと自体が目的なので、`fail` は
「消してはいけない例外」（ADR-189 のトグルなど）にだけ残り、それ以外は期待結果の不変条件で判定する。

**重要な限界**: CI の実機E2E（e2e-ime）は GJI/MS-IME × Win32 の `Edit`（ImmCross）で測る。TsfNative
（Chrome・Windows Terminal など）にしか効かない経路は、e2e-ime では**検証できない**。
ADR-186 E6（idle-conv-check を無効化しても3/3 ALL PASS）は、「壊れなかった」のではなく
「Win32 では使われない経路なので測れなかった」だった。撤去が `pass` になっても、対象がTsfNative専用の
機構なら、それは何も証明していない。§6 の表の「空白」はこの型の穴である。

## 2. 確認手段の層

下の層ほど安く速く、上の層ほど実環境に近い。撤去1件ごとに**最も安い層で確認できるものは、そこで確認する**。

| 層 | 手段 | 何を見るか | 実行場所 |
|---|---|---|---|
| L0 | コンパイルとガード | `cargo check --target x86_64-pc-windows-msvc`、`tests/architecture_guard.rs`（呼び出し箇所の件数ガード）、`tests/layer_boundary_guard.rs`、dylint、`adr-evidence-consistency` | Linux |
| L1 | 純粋判定の単体テスト | `state/` の `classify_*`・`check_drift_correction` 等。`tests/golden_scenarios.rs`。**「諦めない」キーは、状態×キー×プリセットの全セルの網羅テストで 100% を要求できる**（[expected-results §4.6](ime-passive-model-expected-results.md)） | Linux |
| L2 | ジャーナル・リプレイ | 実機ログを固めた入力列を純粋関数へ流し、遷移を回帰させる（`tests/journal_replay.rs`、`tests/drift_correction_replay.rs`、[journal-replay-guide.md](journal-replay-guide.md)）。**入力中などを打鍵履歴から追う領域（`KeyTrack`）は、未見データのリプレイで一段予測の正答率が閾値以上であることを見る**（100% は要求しない） | Linux |
| L3 | CI実機E2E（`e2e-ime.yml`） | GitHub-hosted の Windows で GJI/MS-IME を入れ、キーを注入して実IMEとEngineの一致を見る | GitHub Actions |
| L4 | 実機（dragonflyg4） | 本番と同じ環境。clipwire で別 worktree をビルド・起動して測る | Windows 実機 |
| L5 | TSFプローブ | TsfNative 相当の入力先で composition・リテラル漏れを見る（`examples/richedit_tsf_probe.rs`、`chrome_probe.rs`） | 実機（CI化は未着手、ADR-193） |

Windows専用のテスト（`ime_key_sequence_golden` など `#![cfg(windows)]`）は Linux では**存在しないのと同じ**で、
エラーも出ない。`cargo check --target x86_64-pc-windows-msvc -p awase-windows --tests --lib` で型は確かめられ、
実行は `windows-build` CI に任せる（[CLAUDE.md](../CLAUDE.md) 参照）。

## 3. 撤去1件あたりの手順

1. **撤去が何に効くか分類する**: Win32(ImmCross) にも効く機構か、TsfNative専用か。後者は L3 で測れない（§1）。
2. **既存の確認が守っているか調べる**: §6 の表と、`grep` で対象関数を呼ぶテストを探す。
   守りが無いなら、**撤去より先に確認手段を用意する**か、確認できないことを記録して進める（§7）。
3. **L0〜L2 を通す**: 件数ガードは、消した分だけ**下がる**はず（下がらない・上がるなら撤去になっていない）。
4. **L3 を回す**: 撤去ブランチを CI に載せ、不変条件の判定（`consistency`・`toggle` など）が通ることと、残す機構（例外）を固定する `fail` の構成が
   変わらないことを確かめる（§4）。期待は [ime-passive-model-expected-results.md](ime-passive-model-expected-results.md) から引く。
5. **L4 で1回だけ実機確認**: L3 で測れなかった範囲（TsfNative、実アプリ、フォーカス遷移）に限る。
6. **記録する**: 撤去コミット本文に、確認した層と結果を書く。`revert` する場合は
   [experiment-logging](../.claude/rules/experiment-logging.md) に従い、アプリ・IME・再現手順を残す。
   TsfNative に効く撤去は [fix-requires-evidence](../.claude/rules/fix-requires-evidence.md) の
   「回帰テストか known-bugs」も要る。

## 4. CI実機E2E（L3）の使い方

### 起動

- `develop` の `e2e-ime.yml` は、**remote の `ci/e2e-ime` または `ci/e2e-scenarios` へ push したときだけ**動く
  （`workflow_dispatch` は既定ブランチにファイルが無いと使えない）。`ci/e2e-scenarios` は名前が `sc-` で始まる操作シナリオだけを回す。
  例: `git push origin HEAD:ci/e2e-ime`（強制pushになりうるので、他セッションが同じ remote ブランチを使っていないか確認する）。
- 1構成 × 3回 × 数十構成が並列に走る。`build`（構成ごと）→ `e2e`（構成×3回）→ `summary`（期待との照合）。
- 結果は `summary` の表（`OK` / `NG(...)` / `観測` / `判定不能`）と、`result-<構成>-<回>` の artifact
  （`ime_key_matrix_spike.log`・`awase.log`・`awase-filtered.log`）。

### 構成の足し方

`e2e-ime.yml` の `plan` ジョブの `cfg(...)` に1行足す。主な引数:

| 引数 | 意味 |
|---|---|
| `expect` | `pass`（全回PASS）/ `fail`（撤去や設定で壊れる）/ `observe`（表に出すだけ） |
| `check` | `expect`（ATOK前提の期待表 `check.py`）/ `consistency`（実IMEのかな=Engine ON、英数=OFF に追随するか）/ `toggle` / `resync` / `vkprobe` |
| `args` | スパイクの引数（`--walk`、`--cold`、`--hz`、`--resync`、`--seq=F2,F0,...` 等） |
| `mutator` | `tools/e2e/ime_key_matrix/ablations/` のスクリプト。コードを機械的に消して別バイナリを作る |
| `ime` / `keymap` | `gji`/`msime`、GJIのプリセット（1=ATOK、2=MS-IME） |
| `awase` | `false` で awase を起動しない（**IME単体の対照実験**） |
| `general` | `config.toml` の `[general]` に足す行（`dbe_mode_key_policy = "passthrough"` 等） |

**撤去ブランチ自体を測るなら `mutator` は要らない**（ビルドされるのがそのブランチのコード）。
`mutator` は「まだ撤去していないコードを、確認のためだけに消す」実験用で、`develop` に対して
「この機構は必要か」を先に測るときに使う（`a7-no-follow.sh` が例）。
ミューテーターが差分を作らなかったら、ワークフローが失敗する（コードが動いて撤去箇所が消えた合図）。

### 読み方

- **`awase=false` の構成が対照**。「実IMEがそうなるのはawaseのせいか、IME本体・CI環境の癖か」を切り分ける。
  MS-IME本体の調査（BUG-152）では、対照で本体の挙動が正常と分かり、原因がawase側と絞れた。
- **`INVALID`（rc=3）は失敗ではなく無効**。人の入力の混入などで測定が成り立たなかった回。有効回が0なら「判定不能」。
- `expect=pass` の構成が1回でもFAILなら `NG`。`expect=fail` で全PASSなら「撤去しても壊れない」＝**その機構は不要かもしれない**
  （ただし §1 の限界に注意）。

### 主な既存構成と、守っているもの

| 構成 | 守っているもの |
|---|---|
| `baseline` / `baseline-henkan` | ATOKプリセットでの無変換/変換・ひらがな等の追随（ADR-186/187） |
| `a1`〜`a7` | 個々の機構の撤去（E1/E2/E5/E7b が**必須**と確定、E4/E6 は不変） |
| `atok-passthrough(-cold)` / `msime` | 素通し・フォーカス直後の追随（BUG-151 の再現条件を含む） |
| `atok-hz` / `msime-hz` / `sc-hz-*` | 半角/全角（ADR-189、トグル） |
| `atok-resync*` | Ctrl+無変換/変換での再同期（`observe`） |
| `sc-dbe-*` / `sc-kanji-*` / `sc-shift-*` | DBEキー・漢字・Shift単独打鍵と `dbe_mode_key_policy` |
| `*-noawase` / `probe-vk-*` | 対照実験・各VKの実効果の調査 |

### 落とし穴

- **GJI のキーマッププリセットは `config1.db` を書いて選ぶ**（`session_keymap` フィールド）。書いたあとは GJI のプロセスと
  `ctfmon` を再起動しないと読み直されない（ワークフローが実施）。
- ビルド成果物のキャッシュキーに**ワークフロー自体が含まれない**ため、ワークフローだけを直すと古い dist が使われることがある。
- `ci.yml` を `workflow_dispatch` で起動すると、手動専用の mutants（数十分〜90分）も走る。通常のゲートだけ見たいなら PR 経由にする。
- `gh run view --json jobs` はジョブ30件で打ち切られる。多い実行は
  `gh api --paginate repos/<owner>/<repo>/actions/runs/<id>/jobs?per_page=100`。
- 速度: 実測で、押下後の観測待ち（+400/+1500ms）を縮める `--fast` は安全側だが効果は約7%。**手順間の待ちも縮める `--speed=K` は
  結果が変わる**（セル表が23〜25/30セルで不一致、決定性 100%→96.7〜98.3%）ので、撤去の確認には使わない
  （別セッションの検証、`ci/e2e-fastnotify`、未マージ）。

## 5. 実機（L4）の使い方

- 本番のcheckoutには未コミットの `config.toml`（本番設定）があるので**触らない**。検証は別 worktree
  （`git worktree add --detach`）＋別 target で行い、終わったら awase を元に戻す（`awase-reboot`）。
- 実行は clipwire（`clipwire-exec`）。ハーネス側のスクリプトは `tools/e2e/ime_key_matrix/`（`run.sh`・`run_loop.sh`・`check*.py`）と
  `device/adr190-{build,run}.ps1`。
- **実行中は実機のキーボード・マウスに触らない**。人の入力が混ざると失敗が偽の再現になる（`check_multi.py` が INVALID にする）。
- **1回ずつ別々の exec から起動する**。PowerShell の `foreach` で連続起動すると2回目以降は前面化に失敗し、全 INVALID になる。
- clipwire のスクリプトはバックスラッシュを書かず（`C:/Users/...`）、`C:\` 直下には書けない（ログが黙って作られない）。
- awase を止めて IME 単体を測る（`e2e-awase-stop`）／awase 有りで測る（`e2e-awase-start`、debug ログと
  `AWASE_TEST_INJECTION=1` で再起動）。注入は `dwExtraInfo = 0x5350494B` の目印付きで、**デバッグビルドでのみ**物理キー扱いになる。
- **API が返す状態を信じず、実際に打って確かめる**（特に TsfNative）。IMEは自分の状態を偽ることがある。

詳細は [tools/e2e/ime_key_matrix/README.md](../tools/e2e/ime_key_matrix/README.md)。

## 6. 撤去候補ごとの確認カバレッジ

ADR-191 決定5の P1/P2 候補について、**現在ある確認**と**空白**を並べた（2026-09-21、コードで確認）。

| 撤去候補 | L1/L2 | L3（e2e-ime） | 空白 |
|---|---|---|---|
| `shadow_effect` / `shadow_action` の決め打ち（ひらがな・無変換/変換・半角全角） | golden、`src/engine/tests.rs` | `baseline*`、`atok-*`、`msime*`、`sc-*`、`*-hz`（ATOK・MS-IMEプリセットのGJI、MS-IME本体） | TsfNative でのEngine追随（ADR-189のトグルが担う。CIで測れない） |
| `dbe_mode_key_policy` / `transport.rs::plan` のDBE分岐 | `plan` の単体テスト（`runtime/` は `cfg(windows)` なので Windows CI のみ。Linuxでは存在しない） | `sc-dbe-*`（suppress/passthrough） | — |
| 単独タップ代行（`*_delegate_to_open_axis`） | `nicola_fsm` のテスト | `atok-optin`、`e7-toggle-false` | 撤去済みブランチでの再測定 |
| follow（ADR-187）／モードキー後の20ms再読み取り | 無し（`ir_decide_read_strategy` は `runtime/` の `cfg(windows)` で、専用の単体テストは見つからなかった） | `a5`、`a7`、`atok-resync*` | BUG-151（cold・SkipTyping）は約4%（約26回中1回）の低頻度で、3回では拾いにくい |
| eisu reset | `eisu_recovery` のテスト | `a4`（**pass**） | TsfNative専用の3経路は測れない（ADR-186 E4） |
| idle-conv-check | — | `a6`（**pass**） | **Win32では使われない**。TsfNative限定のため実質空白 |
| **フォーカス変更時の強制OFF** | **無し** | **無し** | **フォーカス切替のシナリオが無い**（§7.1） |
| **`ir_apply_drift_correction`** | 判定側（`check_drift_correction`）と BUG-43 のリプレイのみ | 無し（`--walk` は明示意図の回復を測れない） | 書き込み側の回復（TsfNative、BUG-20/BUG-19） |
| warmup（TSF cold-start） | `warmup_gate_focus_scope` 等 | 無し | 既存の例外として**撤去対象外**（ADR-191 決定1） |

## 7. 調査結果

### 7.1 フォーカス変更時の強制OFF（`runtime/ime_refresh.rs`、`focus_change_enforce_off`）

- **何をするか**: フォーカスが新しいウィンドウへ移ったとき、awase の belief が OFF なら、そのウィンドウの IME へ IMM32 経由で OFF を書く
  （非TsfNativeのみ）。belief を IME へ押し込む能動モデルで、ADR-191 決定1に反する。
- **効く範囲は ImmCross のアプリだけ**: 書き込みに使う `PlatformRuntime::set_ime_open_ordered`（`platform.rs`。トレイトの `set_ime_open` から ADR-090 §2.A で移した版）は IMM32 専用で、それ以外では no-op になる。
  発火したかは `[composition] FocusChange: set_ime_open(false) sent=…` のログで分かる（`sent=false` は授権が下りず書いていない）。
- **判定に使う値が観測ではない**: 同じ関数の直前で、非TsfNativeなら `record_confirmed(effective_open)`（前の窓の belief）を
  「確認済み」として書く。強制OFFの条件 `applied_ime_on` は、この値で決まる。新しい窓の実IMEは読んでいない。
- **撤去したときの懸念**: belief=OFF のまま、新しい窓の実IMEがONだと、「Engine OFF なのに IME ON」のずれが、観測が belief を上書きするまで残る。
  上書きまでの時間は**コードだけでは確認できなかった**（未検証）。
- **裏付けの不在**: 専用のテスト・known-bugs は無い。ADR-090 のインベントリ表の1行（入口の1つ）として載るだけ。
  導入の意図は `git log -S` で辿れる範囲（リファクタ）までしか遡れなかった。BUG-025 の記述は「トグルON中のフォーカス変更時の安全策」で別機構。
- **判断**: 撤去は試作できる（約10行）が、**確認手段が無いので、撤去前に確認手段（§8-1）を用意する**。

### 7.2 `ir_apply_drift_correction`（TsfNative救済の最後の1本、BUG-20）

- **判定側は守られている**: `check_drift_correction` の単体テスト（明示意図と conv 推論の衝突、意図なしでの発火抑止など）と
  `tests/drift_correction_replay.rs`（BUG-43 の実機ログ16回連続送信を有界化することの固定）。
- **書く側は純粋テストで守れない**: TsfNative では `apply_ime_open_with_belief` が VK を実送信する部分は、実機か CI の E2E でしか測れない。
  ADR-191 決定5の「`--walk` では測れない」はこの部分を指す。撤去前に、**明示意図の回復シナリオ**（ユーザーが OFF にした直後に
  IME が ON のまま固定される、2026-07-08 の実機症状）を用意する必要がある。
- **TsfNative を決定的に測る入力先が使える**（ADR-193）: `RICHEDIT50W` を `Chrome_RenderWidgetHostHWND` の名前でスーパークラス化すると、
  awase は `app_kind=TsfNative → mode=Vk` として扱い、確定文字を `WM_GETTEXT` で厳密に読める（実機・GJIで成功）。
  ただし CI（`e2e-ime.yml`）への配線は未着手（GJI 有効化の `--activate-gji` 相当が要る）。

### 7.3 一般的な発見

- CI の GJI 系構成は Win32 の `Edit` が入力先なので、**「TsfNative 専用の機構を消しても全PASS」は証拠にならない**（§1）。
- `expect=fail` の撤去実験（E1/E2/E5/E7b）は、機構が必須と**CIで機械的に固定**している。撤去ブランチがこれらの機構に触れるなら、
  期待が `fail` から変わる（=撤去で壊れなくなる）ことが、撤去の妥当性の根拠になる。**期待の変更は撤去コミットとセットで行う**。
- MS-IME 本体だけ、CI 環境の 150ms タイムアウトで ImmCross が失敗して非冪等な経路へ落ちる問題があった（BUG-152/ADR-190、修正済み）。
  CI と実機で結果が食い違うときは、まず `awase=false` の対照を疑う。

## 8. 空白を埋める提案（未着手）

撤去の実作業とは別に、確認手段を先に整える候補。いずれも**追加**なので、撤去が頭打ちになる前に増やしすぎないこと（ADR-191 決定5）。

1. **フォーカス切替シナリオ**（§7.1の空白）: `ime_key_matrix_spike` に、ウィンドウを2つ持ち、片方を IME ON にしてから
   もう片方（belief OFF のまま）へフォーカスを移し、**移動後の実IME状態とEngineの一致**を +100/+400/+1500ms で記録するモードを足す。
   構成は `consistency` 判定の流用で足りる見込み。撤去の前後で「不一致が続く時間」を比べる。
2. **TsfNative 相当（ADR-193 の RichEdit スーパークラス）の CI 配線**: §7.2 の空白を埋める。GJI の有効化と、
   `awase=true/false` の対照構成が要る。BUG-002 型は実機で再現しなかったので、対象は drift correction と warmup に絞る。
3. **明示意図の回復シナリオ**: 「Ctrl+無変換で OFF にした直後に IME が ON へ戻る」状況を作る（`--resync` の流用を検討）。
   ATOK/GJI で作れるかは未確認。作れなければ、drift correction は撤去せず残す判断の根拠になる。
4. **低頻度の失敗の拾い方**: BUG-151 のような約4%の失敗は、3回では見逃す。`run_loop.sh` の高速版（1回約23秒）で回数を増やすか、
   決定的な再現条件（`--cold` で先頭のひらがなを除く）を構成に固定する。

## 9. 出典

- ADR-186（撤去実験E1〜E7b、CI化）、ADR-187、ADR-189、ADR-190/BUG-152、ADR-191（方針・決定5）、ADR-193（RichEdit スーパークラス）
- `.github/workflows/e2e-ime.yml`、`tools/e2e/ime_key_matrix/README.md`
- [journal-replay-guide.md](journal-replay-guide.md)、[smoke-testing-guide.md](smoke-testing-guide.md)（アプリ別の手動スモーク）
- BUG-020、BUG-025、BUG-043、BUG-151、BUG-152（`docs/known-bugs/`）
