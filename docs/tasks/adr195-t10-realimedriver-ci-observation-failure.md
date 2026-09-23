# ADR-195 T10: `RealImeDriver`がGitHub Actions windows-latestで実IMEを観測できない原因を究明する

状態: 未着手（2026-09-23起票）。[ADR195-T1](adr195-t1-independent-learning-process.md)
（独立学習プロセス本体）・[ADR195-T9](../adr/195-keymap-learn-productization.md)
（学習結果の出力結合、PR #258、`feat/adr195-t9-learning-output-binding`）の実機検証で
発見された不具合。着手時は `.claude/rules/worktree-per-session.md` に従い専用
worktree/branch を切ること。

## 背景

T-rebase/T0/T1/T2/T3/T4/T5/T6/T8/T9を統合したブランチ（`ci/adr195-integration-verify`、
コミット`2a06626b`時点）を、GitHub Actions windows-latestランナー上で実際に動かして
検証した（`.github/workflows/adr195-integration-verify.yml`、run
[35840828329](https://github.com/cuzic/awase/actions/runs/35840828329)）。

ビルド・書き出しパイプライン自体（T9のスコープ、B1対応）は正常に動作することを確認できた
——`awase-keymap-learn-win.exe --strategy=s0`は353秒(約5分53秒)実行後に`result
status=success`で自然終了し、`<config dir>/keymap-learn-table.json`もパース可能な
JSON(`schema_version=1`)として書き出された。

しかし、**学習の核心である実機観測が一度も成功しなかった**:

```
result status=success strategy=S0 現状(毎回リセット) elapsed_ms=353031 presses=0 cells=0 total=168 decode_errors=0 persisted_cells=0 verify_accuracy=0.000 verify_confidence=0.000
```

`presses=0`・`cells=0`——353秒・全168セルに対して`s0()`
(`crates/awase-keymap-learn/src/strategy.rs`)が試行を重ねたにもかかわらず、
**測定が1件も記録されなかった**。

## `s0()`の該当ロジック（無限ループではない、上限付き）

```rust
fn s0<D: ImeDriver>(exec: &mut Executor<D>, g: &Graph, req: &Req) {
    for (node, key) in edge_cells(g) {
        let s = g.status_of_node(node);
        let mut attempts = 0;
        while (exec.table.count(s, key) as u32) < req.k && attempts < req.k + 3 {
            if over(exec, req) {
                return;
            }
            attempts += 1;
            exec.reset();
            for kind in g.path(g.initial_node, node) {
                if let EdgeKind::Press { key, .. } = kind {
                    exec.press_setup(key);
                }
            }
            let st = exec.settle_setup();
            if st != s {
                exec.note_sync_loss();
                continue;
            }
            exec.set_recording(true);
            let _ = exec.press(key);
        }
    }
}
```

`presses=0`ということは、**全168セル×最大5回(`req.k + 3`、既定`k=2`)の試行すべてで
`settle_setup()`後の状態が目標状態`s`と一致しなかった**(`st != s`が常に真、
`note_sync_loss()`だけが積み上がり、`exec.press(key)`にまで到達しなかった)ことを意味する。
これは学習アルゴリズム側の欠陥ではなく、`RealImeDriver`
(`crates/awase-keymap-learn-win/src/driver.rs`)が実際のGJI/IMEの状態変化を
このCI環境で正しく観測できていないことを強く示唆する。

## A'実測（自己操作0件）の結果は判定保留

学習プロセス実行中に検出されたactuation関連ログは1件のみ:

```
2026-09-23T09:16:56.528403Z DEBUG awase_windows::ime_controller: [warrant-shadow] chain=set_ime_open open=false origin=EventOrigin { source: SelfActuated { strategy: "focus_change_enforce_off" }, epoch: Generation(0) } warranted
```

このタイムスタンプ(09:16:56.528)は、ワークフロー側が学習プロセスの自然終了を検出した
タイムスタンプ(09:16:58.293、3秒間隔のポーリングループのため最大3秒のラグを含む)と
ほぼ同時刻であり、**学習プロセスの専用窓が閉じてフォーカスが移動した直後の、awaseの
正常な追随動作（学習セッション終了後の挙動）である可能性が高い**。「学習中の
A'違反」と断定はできない——今回のワークフローは「学習開始〜自然終了検出」の全区間を
一括でスキャンしており、終了直後の数秒を「学習中」と区別できていない、ワークフロー側の
粒度の粗さが原因の可能性がある。

## 究明すべきこと

1. **`RealImeDriver`の窓活性化・フォーカス確保ロジックの調査**
   (`crates/awase-keymap-learn-win/src/driver.rs::RealImeDriver::new`他)。
   専用EDIT窓が実際にフォアグラウンド・入力フォーカスを得られているかを、
   `GetForegroundWindow`/`GetFocus`等で診断ログに出すコードを一時的に追加し、
   再度CI(`.github/workflows/adr195-integration-verify.yml`または後継ワークフロー)で
   確認する。opus-adversarial-consultのADR-195レビューが「`compartment_notify_probe`
   (`tools/e2e/ime_key_matrix`)と同種の前面化対策(`AttachThreadInput`/
   `SetForegroundWindow`/`BringWindowToTop`の組み合わせ)が`RealImeDriver`側には
   無い」と既に指摘している(このセッションでのT1実機A'実測時にも同様の仮説が出た)。
2. **`settle_setup()`/`observe_imm()`の失敗理由の可視化**。現状`decode_errors=0`
   だったため「観測が取れて中身が違った」のではなく「observe_imm自体は成功する
   (エラーにならない)が、返ってきた状態が期待と違う」ケースであることが分かる——
   例えば、フォーカスが専用窓に無いまま`SendInput`だけが素通りし、実際には別の
   ウィンドウ(GJIの通常入力欄ではない何か、あるいはCIコンソール)にキーが届いて
   いて、観測される状態が常に初期状態のまま変化しない、という可能性がある。
3. **GitHub-hosted windows-latestランナー固有の環境差**(headless寄りのセッション、
   対話的デスクトップの扱いの違い)が、通常の実機(dragonflyg4)と比べて窓の
   フォーカス確保をより困難にしていないかを、
   `.github/workflows/e2e-ime.yml`が使っている`tools/e2e/ime_key_matrix`
   (同じCI環境で実際にキー注入・観測に成功している既存実績がある)との実装差分の
   比較から切り分ける。特に、専用窓の生成方法(`CreateWindowExW`のスタイル・
   親ウィンドウの有無)やメッセージポンピングの方式に注目する。
4. **dragonflyg4実機での再検証**(バックグラウンド/非対話実行が壊れていた問題が
   解消していれば)。CI環境固有の問題か、`RealImeDriver`自体の設計上の問題かを
   切り分けるため、対話的セッションでの動作確認が最終的に必要。

## 再現手順

1. `git fetch origin` してから `origin/ci/adr195-integration-verify`
   (または後継のT9統合ブランチ)をチェックアウトする。
2. `.github/workflows/adr195-integration-verify.yml`を
   `gh workflow run adr195-integration-verify.yml --ref <ブランチ名>`で手動起動する
   (共有ブランチ`ci/e2e-ime`等ではなく、専用ブランチを使うこと)。
3. 完了後、`gh run download <run id> -n adr195-integration-verify-result`で
   アーティファクトを取得し、`result.txt`・`learn-stdout.log`・`dist/awase.log`を
   確認する。
4. **既知の罠**: `dist/awase.log`はジョブ終了時に`Stop-Process -Force`で
   awase.exeを強制終了してから収集しているため、tracingのファイル書き込みが
   バッファリングされている場合、実際にPowerShellが検出した行数
   (ジョブログの「学習プロセス実行中に追加された awase.log の行数=N」)より
   アーティファクト内の行数がはるかに少ないことがある(今回はN=27813に対し
   アーティファクトは68行のみだった)。**行数の突き合わせにはアーティファクトではなく
   ジョブログ自身(`gh run view --log`)を使うこと**。

## 完了条件

- `presses=0`の原因(フォーカス確保の失敗、観測ロジックの不備、CI環境固有の問題等)を
  特定する。
- 特定した原因に対する修正案(またはdragonflyg4実機でのみ成立する設計である旨の
  文書化)。
- 修正後、`awase-keymap-learn-win.exe --strategy=s0`が実際に1件以上のセルを
  測定できることをCIまたは実機で確認する回帰テスト・記録を残す
  (`.claude/rules/fix-requires-evidence.md`のwarmup/focus系ファミリーに準じる)。

## 関連

- [ADR-195](../adr/195-keymap-learn-productization.md) 段階1
- [ADR195-T1](adr195-t1-independent-learning-process.md)（`RealImeDriver`本体）
- ADR195-T9（学習結果の出力結合、B1対応、PR #258 `feat/adr195-t9-learning-output-binding`。
  このタスク自体の専用docファイルは未作成、PR説明とコミットメッセージに実装内容がある）
- `.github/workflows/adr195-integration-verify.yml`（今回の検証ワークフロー、
  `ci/adr195-integration-verify`ブランチに存在）
- `.github/workflows/adr195-t1-actuation-zero.yml`（T1単体のA'実測用ワークフロー、
  `ci/adr195-t1-actuation-zero`ブランチに存在。同種のCI環境固有と思われる
  フォーカス瞬断が以前にも観測されている）
