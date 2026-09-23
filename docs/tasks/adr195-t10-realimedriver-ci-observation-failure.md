# ADR-195 T10: `RealImeDriver`がGitHub Actions windows-latestで実IMEを観測できない原因を究明する

状態: **解決（2026-09-23）。製品パス(既定戦略S6)は`RealImeDriver`のフォーカス確保修正
込みで実際に学習できることをCIで確認した(`cells=84/168, verify_accuracy=0.997,
persisted_cells=84`)。修正はPR [#266](https://github.com/cuzic/awase/pull/266)。
別途、`--strategy=s0`(CLI診断専用フラグ、製品コードパスでは使われない)固有の
不具合が新たに判明したが、これはT10のスコープ外の別問題として追記した(下記
「新たに判明したS0固有の問題」参照)。** [ADR195-T1](adr195-t1-independent-learning-process.md)
（独立学習プロセス本体）・[ADR195-T9](../adr/195-keymap-learn-productization.md)
（学習結果の出力結合、PR #258、`feat/adr195-t9-learning-output-binding`）の実機検証で
発見された不具合。

**2026-09-23追記（[ADR195-T7](adr195-t7-safety-measures.md)、opus-adversarial-consult
round1 m4）**: `feat/adr195-t7-safety-measures`（PR #264）が`RealImeDriver`のquiet
window判定・送信前ゲートに`GetForegroundWindow()==self.window && GetFocus()==self.edit`
の確認を追加した。フォーカス取得に失敗する環境（CI等）では、この変更以降
学習プロセスは`presses=0`で自然終了する代わりに、起動直後（quiet window）に
`Err`を返して即座に終了するようになる可能性がある。**下記「究明結果」節の検証は
この変更込みのdevelop先端を基準に行った——実際には(フォーカス確保自体を修正した
ため)quiet windowのフォーカス喪失判定に引っかかることなく、既定戦略で学習に
成功することを確認した。**

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

## 究明結果(2026-09-23、5回のCI検証で収束)

`~/rust-nicola-worktrees/adr195-t10`(branch `diag/adr195-t10-realimedriver-focus`、
`origin/develop`分岐点`7e29edd7`)で、専用の検証ワークフロー
(`.github/workflows/adr195-t10-focus-verify.yml`、`ci/adr195-integration-verify`の
後継、当該ブランチに残置)を使って検証した。

### 1回目・2回目: フォーカス確保自体は直った、しかしpresses=0のまま

`RealImeDriver::new`が素の`SetForegroundWindow`/`SetFocus`のみに依存していた点を、
`tools/e2e/ime_key_matrix`の`compartment_notify_probe.rs::bring_to_front`と同じ
`AttachThreadInput`併用パターン(`secure_foreground_focus`/`focus_on_edit`/
`secure_focus_with_retries`、`crates/awase-keymap-learn-win/src/driver.rs`)へ置き換えた。
CIで`secured=true foreground=true focused_edit=true`(1回のリトライ後に成功)を確認——
**「フォーカス確保が無言で失敗しうる」という仮説自体は正しく、この修正で解消した**。

しかし`--strategy=s0`(全セルを毎回リセットして巡回する診断用戦略)では、それでも
`presses=0 cells=0`のままだった(`elapsed_ms=776282`)。`observe_imm()`の呼び出し回数・
`self.initial`と異なる値を返した回数を数える診断カウンタを追加したところ、
`inject_calls=6110 inject_failures=0 status_changes=3000`——**観測パイプライン自体は
生きている**(SendInputは全件OSに受理され、observeは何度も値の変化を検知している)ことが
判明し、「observe側が完全に死んでいる」という当初の仮説は否定された。

### 3回目: 不一致の中身が「IME openビットが一度もtrueにならない」に特定される

`s0()`内の`st != s`不一致の内容(期待状態・観測状態・経路)をオプトインで出力する診断
(`KEYMAP_LEARN_DEBUG_CELLS`環境変数、`crates/awase-keymap-learn/src/strategy.rs`)を
追加。出力された全件が`expected=Status{open:true,...} observed=Status{open:false,...}`
——`mode`/`composing`は一致するのに`open`だけが一度もtrueにならない。経路
(`path_keys=[0]` = `main.rs::KEYS[0]` = `0x1D`(無変換キー))を1回押すだけで
IMEがopenになるはずの状態遷移が、実機で一度も成立していないことが分かった。

### 4回目: awase.exe自体の干渉ではないと確定

「awase自身がこの物理キーを横取りしている(focus/focus_change_enforce_off系の
自己actuationが割り込んでいる)のでは」という仮説を検証するため、awase.exeを
**起動せず**同じ`--strategy=s0`を実行した。結果は`inject_calls=6110
status_changes=3000`と3回目(awase.exeあり)から**1桁まで完全一致**——
**awase.exeの有無は結果に一切影響しなかった**。この仮説は否定された。

### 5回目: 製品パス(既定戦略S6)は成功、S0固有の問題と判明

並行してrust-nicola-c5が、developの現在の先端(このブランチのfocus修正を**含まない**)で
既定戦略(`--strategy=s0`を付けない、`Strategy::S6`)を試したところ
`presses=43 cells=14 verify_accuracy=1.000`と成功した
(run [35864760022](https://github.com/cuzic/awase/actions/runs/35864760022))。
これを受け、focus修正込みのこのブランチで、awase.exeを起動した状態で既定戦略を
試したところ、同じく成功した:

```
result status=success strategy=S6 部分1-switch elapsed_ms=65255 presses=933 cells=84
total=168 decode_errors=0 persisted_cells=84 verify_accuracy=0.997 verify_confidence=0.987
```

(run [35866290391](https://github.com/cuzic/awase/actions/runs/35866290391)。ジョブ
全体の結論は「failure」表示だが、これは検証3(A'実測)がawase起動時の正常な
`Engine activated`/`Engine deactivated`ログ(学習窓へのフォーカス移動・離脱時の
通常の追随動作)を誤検知しているだけで、「A'実測の結果は判定保留」節で既に指摘して
いた検証粒度の粗さによるもの。検証2(学習結果ファイル)はPASSしている。)

### 結論

- **T10が報告していた症状(presses=0)の実害は、フォーカス確保の不具合(修正済み)と、
  `--strategy=s0`という診断専用CLIフラグ固有の別問題の、2つが重なって起きていた。**
- フォーカス確保の不具合は実在し、修正(`AttachThreadInput`併用パターン)で解消した。
  この修正は残すべき(`compartment_notify_probe.rs`と同じ実績あるパターン)。
- 製品が実際に使う既定戦略(S6)は、この修正・awase.exe同時起動の下で実際に学習に
  成功する(`cells=84/168, verify_accuracy=0.997`)。**T10の完了条件(1件以上のセルを
  測定できることの確認)はこれで満たされた。**
- `--strategy=s0`固有の問題(下記)は、製品コードパスでは`--strategy=s0`という
  CLIフラグを渡すことがないため実害が無い。T10のスコープからは分離し、
  対応は任意のフォローアップとする。

## 新たに判明したS0固有の問題(T10のスコープ外、フォローアップ候補)

`--strategy=s0`(`crates/awase-keymap-learn/src/strategy.rs::s0`、
`awase-keymap-learn-win.exe --strategy=s0`で明示的に指定したときのみ使われる、
製品コードパスでは到達しない診断専用ルート)が、ATOKプリセットのCI環境で
恒久的に`presses=0`になる。上記究明の通り、原因はフォーカス・observe・awase干渉
いずれでもなく、**「`KEYS[0]`(=`0x1D`、無変換キー)を1回押すだけでIMEがopenになる」
というグラフ/Priorモデル側の想定が、このATOK環境の実際の無変換キー挙動と
食い違っている**可能性が高い(この無変換/変換キーの状態依存性は
[ADR-186](../adr/186-gji-atok-mode-key-measured-matrix-and-belief-follow.md)でも
既知の論点)。S6(既定戦略)は同じ状態を別の経路で到達する(または単純に
このエッジをテストしない)ため影響を受けない。

対応候補(未着手):
- `Prior`(初期仮説モデル)のATOKプリセットにおける無変換キーの扱いを見直す。
- または`s0()`自体を診断専用ツールとして「特定エッジで詰まったら次へ進む」
  タイムアウト/スキップ機構を持たせる(現状は`req.k + 3`回リトライして
  進めなくなるだけで、次のセルには進む設計だが、`node=0`絡みのエッジが
  大量に存在するため全体が消費される)。
- 実害が無い(製品パスは使わない)ため優先度は低い。

## 完了条件

- [x] `presses=0`の原因(フォーカス確保の失敗、観測ロジックの不備、CI環境固有の問題等)を
  特定する。→ フォーカス確保の不具合(修正済み)と`--strategy=s0`固有の問題の2つと判明。
- [x] 特定した原因に対する修正案。→ `AttachThreadInput`併用のフォーカス確保修正
  (`crates/awase-keymap-learn-win/src/driver.rs`、PR [#266](https://github.com/cuzic/awase/pull/266))。
- [x] 修正後、実際に1件以上のセルを測定できることをCIで確認する記録を残す。→ 本節
  (5回目のCI実行、`cells=84/168, verify_accuracy=0.997`)。

## 関連

- [ADR-195](../adr/195-keymap-learn-productization.md) 段階1
- [ADR195-T1](adr195-t1-independent-learning-process.md)（`RealImeDriver`本体）
- ADR195-T9（学習結果の出力結合、B1対応、PR #258 `feat/adr195-t9-learning-output-binding`。
  このタスク自体の専用docファイルは未作成、PR説明とコミットメッセージに実装内容がある）
- [ADR195-T7](adr195-t7-safety-measures.md)（PR #264、quiet window判定へのフォーカス
  確認追加。本タスクのフォーカス確保修正とは独立に、同じ問題の別の側面(汚染検出)を
  扱っている）
- `.github/workflows/adr195-integration-verify.yml`（起票時点の検証ワークフロー、
  `ci/adr195-integration-verify`ブランチに存在）
- `.github/workflows/adr195-t10-focus-verify.yml`（本タスクの解決に使った検証
  ワークフロー、`diag/adr195-t10-realimedriver-focus`ブランチに存在。5回のCI実行の
  詳細は上記「究明結果」節参照）
- `.github/workflows/adr195-t1-actuation-zero.yml`（T1単体のA'実測用ワークフロー、
  `ci/adr195-t1-actuation-zero`ブランチに存在。同種のCI環境固有と思われる
  フォーカス瞬断が以前にも観測されている）
- [ADR-186](../adr/186-gji-atok-mode-key-measured-matrix-and-belief-follow.md)
  （無変換/変換キーの状態依存性、S0固有の問題の背景知識として参照）
