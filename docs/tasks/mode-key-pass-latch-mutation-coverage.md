# ModeKeyPassLatch のメソッド層に22件のテスト漏れ（cargo-mutantsで実測済み、要修正）

状態: **完了**（2026-09-23起票・同日中に解消）。PR #248
（`test/mode-key-pass-latch-mutation-coverage`）でテスト7件を追加し、
`.github/workflows/mutants-scope-investigation.yml` を`windows-latest`へ
切り替えて再実行（run
[35819375184](https://github.com/cuzic/awase/actions/runs/35819375184)）した結果、
22件中21件が`caught`に変わった。残る1件（`158:28`）は解析の結果、等価変異体
（メソッドの公開インタフェース経由ではどんな入力でも元コードと区別不可能）と
判断し、`.cargo/mutants-bug158-scope.toml`に理由付きで`exclude_re`登録した
（下記「最終結果」節参照）。

## 背景

ADR-194（IME時間依存ロジックの仮想時間シミュレーションハーネス、`feat/ime-sim-harness`、
develop未マージのまま破棄）の再挑戦条件を検証する過程で、「純粋な決定関数を、シミュレーション
ハーネスなしの直接単体テスト（決定表スタイル）でどれだけ網羅できているか」を実測するため、
develop本体で実際に採用されている `state/force_guard.rs` + `state/mode_key_pass.rs`
（PR #240、ADR-191撤去のマージで導入。`ModeKeyPassLatch`はBUG-157/158の通過マーク状態機械）に
cargo-mutantsを回した。

- 使ったワークフロー: `.github/workflows/mutants-scope-investigation.yml`
  （develop、`workflow_dispatch`専用）+ `.cargo/mutants-bug158-scope.toml`
- 実行: `gh workflow run mutants-scope-investigation.yml --ref <対象branch>`
- **重要な罠**: `cargo-mutants -p awase-windows`を素で`ubuntu-latest`で回すと、
  `crates/awase-windows/examples/*.rs`（`windows` crate依存、`#[cfg(windows)]`無し）の
  ビルドがLinux上でE0433多発し、`--cargo-test-arg`/`additional_cargo_test_args`/
  末尾`-- --lib`のどれを使ってもbaseline・mutant試験どちらにも`--lib`スコープが反映されず、
  全mutantsが`unviable`になって無意味な結果になる（2026-09-22実測、5回連続で確認）。
  今回はexamples/を丸ごと削除した使い捨てブランチ（`chore/mutants-bug158-scope-scratch-develop`、
  実行後に削除済み）で回避したが、**もっと簡単な対策がある**: 同時期に別セッションが
  `docs/tasks/actuation-confluence-already-matched-gap.md`（IME actuation合流点の
  棚卸し）で使った`.github/workflows/mutants-actuation-confluence-windows.yml`は
  `runs-on: windows-latest`（GitHub-hosted、publicリポなら無料）を使っており、実Windows
  ターゲットでは`windows` crateが解決できるためexamplesが普通にビルドできる。
  次回このタスクに着手する際は、`mutants-scope-investigation.yml`の`runs-on`を
  `ubuntu-latest`→`windows-latest`に変えるだけで、examples削除の使い捨てブランチは
  不要になるはず（未検証、対象2ファイルはプラットフォーム非依存なのでwindows-msvc
  ターゲットでもビルド・テストできる）。

## 実測結果（develop、2026-09-22、run [35815868178](https://github.com/cuzic/awase/actions/runs/35815868178)）

`force_guard.rs` + `mode_key_pass.rs`: **95 mutants tested: 22 missed, 68 caught, 5 unviable**

**missedの22件は全て`mode_key_pass.rs`（`ModeKeyPassLatch`構造体のメソッド）に集中しており、
`force_guard.rs`側（`should_drop_intents_for_mode_key_pass`等の純粋関数、既存の決定表的
直接単体テストの対象）はmissedゼロだった。** 内部の判定ロジック（純粋関数）は既にテスト
済みでも、「markを`peek`で読む→純粋関数へ委譲→結果に応じてmarkを書き戻す」という
**構造体メソッド自身のグルーコード**を直接検証するテストが無い、という構図。

### missed一覧（全22件、`crates/awase-windows/src/state/mode_key_pass.rs`）

| メソッド | 行:列 | 変異内容 |
|---|---|---|
| `note_awase_write` | 85:9 | 関数本体を`()`に差し替え |
| `note_awase_write` | 86:16 | `!mark.awase_wrote`の`!`を削除 |
| `note_awase_write` | 90:25 | `ModeKeyPassMark{awase_wrote: true, ..}`の`awase_wrote`フィールドを削除 |
| `window_remaining_ms` | 113:9 | 戻り値`Option<u64>`を`None`に差し替え |
| `window_remaining_ms` | 113:9 | 戻り値を`Some(0)`に差し替え |
| `window_remaining_ms` | 113:9 | 戻り値を`Some(1)`に差し替え |
| `expiry_wait_ms` | 123:9 | 戻り値を`None`に差し替え |
| `expiry_wait_ms` | 123:9 | 戻り値を`Some(0)`に差し替え |
| `expiry_wait_ms` | 123:9 | 戻り値を`Some(1)`に差し替え |
| `expiry_wait_ms` | 126:34 | `!mark.readable_at_arm \|\| mark.invalidated`の`\|\|`を`&&`に置換 |
| `expiry_wait_ms` | 126:12 | 同式の`!`を削除 |
| `drop_decision` | 158:18 | `first \|\| (align && !mark.aligned)`の`\|\|`を`&&`に置換 |
| `drop_decision` | 158:28 | 同式の`&&`を`\|\|`に置換 |
| `drop_decision` | 158:31 | 同式の`!mark.aligned`の`!`を削除 |
| `drop_decision` | 164:21 | `ModeKeyPassMark{aligned: ..}`構築式の`aligned`フィールド式内、`mark.aligned`参照を削除 |
| `drop_decision` | 164:43 | `mark.aligned \|\| (align && !on_expiry)`の`\|\|`を`&&`に置換 |
| `drop_decision` | 164:53 | 同式の`&&`を`\|\|`に置換 |
| `drop_decision` | 164:56 | 同式の`!on_expiry`の`!`を削除 |
| `align_after_expired` | 184:9 | 戻り値`bool`を`true`に差し替え |
| `align_after_expired` | 184:9 | 戻り値を`false`に差し替え |
| `align_after_expired` | 188:12 | `!should_align_after_expired_mode_key_pass(..)`の`!`を削除 |
| `align_after_expired` | 194:17 | `ModeKeyPassMark{aligned: true, ..}`の`aligned`フィールドを削除 |

## やること

1. **`note_awase_write`（85/86/90行、3件）**: `arm()`で新しいマークを立てた直後
   （`awase_wrote=false`）に`note_awase_write`を呼ぶと`awase_wrote`が`true`になり、
   それが**その後の`align_after_expired`の判定を変える**ことをブラックボックスで確認する
   （`ModeKeyPassMark`のフィールドはprivateだが、`align_after_expired`は
   `!mark.awase_wrote`を要求するので、内部フィールドを直接覗かずに観測できる）。
   例: `arm→note_awase_write→align_after_expired`が`false`を返すこと（awase自身が
   書いたなら実IMEを信用しない、BUG-158追補2）と、`note_awase_write`を呼ばない対照群では
   同条件で`true`を返すことを対比させる。これで3件とも一度に潰せるはず（`arm()`の
   再書き込みが同一payloadなら副作用ゼロなので、`awase_wrote`を再度trueにする冗長呼び出し
   自体は観測不能=等価変異体の可能性が高い。85/86/90はいずれも「初回のfalse→true遷移が
   起きなくなる」形の変異なので、上記1テストで直接潰せる）。
2. **`window_remaining_ms`（113行、3件）**: 現状この構造体メソッド自身を直接呼ぶテストが
   1つも無い（内部で委譲する純粋関数`mode_key_pass_window_remaining_ms`は既にテスト済み）。
   `arm(scope, 100, true)`後、`window_remaining_ms(150, scope, 300)`が`Some(250)`
   （0でも1でもない具体値）を返すことを確認するだけで3件とも潰せる。マーク無し・
   スコープ不一致で`None`になるケースも決定表として一緒に固定するとなお良い。
3. **`expiry_wait_ms`（123/126行、5件）**: 同様に直接呼ぶテストが無い。
   `readable_at_arm × invalidated`の2軸・計3ケース（両方false→`Some(具体値)`、
   `readable_at_arm=false`→`None`、`invalidated=true`→`None`、後者2つは`drop_decision`で
   `invalidated`を立てるか`arm`の`readable`引数で作る）を決定表として固定すれば
   126:12/126:34と123:9系3件が同時に潰れるはず。
4. **`drop_decision`の158/164行（7件）**: 現状の唯一の既存テスト
   （`mode_key_pass_latch_arms_and_drops_within_window`）は毎回`has_last_intent=false`
   固定・かつ1回目の呼び出しで`align`と`aligned`が同時にtrueになるため、
   「2回目以降・`mark.aligned`が既にtrue」という分岐そのものは通っても、`arm()`の
   再書き込みが**同じ値を書き戻すだけ**になり観測できない（`&&`↔`\|\|`/`!`削除の
   変異が生存しやすい構造）。`aligned=false`のまま`first=false`に到達させるには、
   1回目を`on_expiry=true`（窓の終了時、観測なし）で呼ぶ必要がある
   （`should_drop_intents_for_mode_key_pass`のexpiry分岐は`aligned`を更新しない）。
   その後、より大きい`window_ms`を渡した2回目の呼び出し（`!expired`を満たすように）で
   `on_expiry=false`を渡すと、`first=false かつ align=true かつ mark.aligned=false`の
   組み合わせに到達し、`drop_decision`後に`aligned`が`false→true`へ実際に変化する
   （これは`align_after_expired`が以後`false`を返すことで観測できる）。この経路が
   本番のコールサイト（`ir_stage_observe`/`ir_stage_notify`）で実際に起こりうるかは
   未確認——ここでは「メソッド自体の入力空間の決定表」として書くか、実際の呼び出し
   パターンに合わせて別の到達経路を探すか、着手時に判断すること。
5. **`align_after_expired`の184/188/194行（4件）**: 184（戻り値まるごとtrue/false）は
   「揃えるべきでない条件（マーク無し・`aligned`済み・`awase_wrote`済み・新規intentあり）
   でも揃えない」ことと「揃えるべき条件で実際に揃える」ことを両方直接呼んで固定すれば
   潰れる。188（`!`削除）は`should_align_after_expired_mode_key_pass`が`false`を
   返す状況で`align_after_expired`も`false`を返すことを確認すれば潰れる。194
   （`aligned`フィールド構築式の削除）は、揃えた**後**に同じマークへ再度
   `align_after_expired`を呼んでも二重に揃えない（`aligned`が実際に`true`に
   書き変わっている）ことを確認すれば潰れる。

**受け入れ基準**: `gh workflow run mutants-scope-investigation.yml --ref develop`
（または前述のwindows-latest化後の同等ワークフロー）を再実行し、上記22件が
`missed`→`caught`に変わること（等価変異体と判断したものは`.cargo/mutants.toml`
方式に倣い`exclude_re`でコメント付き除外してよい）。

## 最終結果（2026-09-23、PR #248、run [35819375184](https://github.com/cuzic/awase/actions/runs/35819375184)）

`.github/workflows/mutants-scope-investigation.yml`を`ubuntu-latest`→`windows-latest`
へ切り替え（examples/\*.rsがLinux上でbaseline・各mutant試験どちらもE0433になり
95 mutants全てunviableになることをこのセッションでローカル再実測して確認済み、
姉妹ワークフロー`mutants-actuation-confluence-windows.yml`と同じ構成に揃えた）、
`crates/awase-windows/src/state/mode_key_pass.rs`に上記「やること」1〜5に対応する
テスト7件を追加して再実行した結果:

**95 mutants tested: 1 missed, 89 caught, 5 unviable**（45分）

missedとして残った1件は`158:28: replace && with || in ModeKeyPassLatch<S>::drop_decision`
のみ。これは`if first || (align && !mark.aligned) {`の内側`&&`の変異で、解析の結果
「`ModeKeyPassLatch`の公開メソッド経由でどんな入力列を与えても元コードと区別不可能」
な等価変異体と判断し、理由付きで`.cargo/mutants-bug158-scope.toml`に`exclude_re`
登録した（判定の詳細はそのファイルのコメント参照）。22件中21件を新規テストで
`missed`→`caught`に変えられたことになる。

## 補足（このタスクの範囲外）

- `feat/ime-sim-harness`（同名メソッドの旧版、`readable_at_arm`無し4引数版）と
  `drift.rs`/`refresh_plan.rs`（develop未採用のまま）に同じ手法で回した実行
  （`--ref feat/ime-sim-harness`、131 mutants）は2回とも約10分で原因不明の
  `interrupted`になり結果が取れなかった。develop未採用のコードなので優先度は低いが、
  再現するなら`windows-latest`化や`--jobs 1`固定などを試す価値はあるかもしれない。
- 今回の調査の本題（ADR-194破棄の再挑戦条件）自体は別途会話ログに記録済み
  （[[project_adr194_retry_condition_verification_2026_09_22]]、条件不成立で
  破棄継続が妥当という結論）。本タスクはその副産物として見つかった実在のテスト
  カバレッジの穴。
- 「これから撤去していくものがあるなら、その前にreplay基盤を充実させる価値はあるか」
  という問いは本タスクとは別軸（削除対象が具体化してから改めて検討する）。
  `docs/tasks/actuation-confluence-already-matched-gap.md`が扱っている
  「冗長経路が欠陥を隠す」問題は、今回の調査中にも別形で実見している
  （`align_after_expired`の`ModeKeyPassedThrough`発火が`drop_decision`の
  `remove_intent`と独立に同じ`last_intent`破棄効果を持つ、ADR-194検証時に
  mutation実験で確認）——具体的な削除候補が挙がったときの参考にする。

## 関連

- ワークフロー: `.github/workflows/mutants-scope-investigation.yml`
- スコープ設定: `.cargo/mutants-bug158-scope.toml`
  （`examine_globs`に`drift.rs`/`refresh_plan.rs`も列挙されているが、developには
  存在しないため実質`force_guard.rs`/`mode_key_pass.rs`の2ファイルだけが対象になる）
- 実行結果: `gh run view 35815868178 --log`（artifactは`chore/mutants-bug158-scope-scratch-develop`
  ブランチ実行時にアーティファクト名のスラッシュでアップロード自体が失敗しているため、
  生ログのみが一次情報。再現するには本ドキュメントの手順で使い捨てブランチを作り直すか、
  windows-latest化してから`--ref develop`で再実行すること）
- 姉妹タスク: [docs/tasks/actuation-confluence-already-matched-gap.md](actuation-confluence-already-matched-gap.md)
  （同時期に別セッションが同じcargo-mutants手法で見つけた、IME actuation合流点の
  テスト漏れ。windows-latestランナーの使い方はこちらを参照）
