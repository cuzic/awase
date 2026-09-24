---
title: awase.exe の読込時に残る「内蔵表との不一致5%判定」とカバレッジ判定が ADR-196 決定1e と食い違う（採用しても学習表が使われない構成がある）
status: 未着手（B-2 は解消済み）
priority: 中〜高（学習機能を次リリースで利用者に見せるなら、その前に「実測→判断」まで。03 の判断に従う）
created: 2026-09-24
related_adr: ["ADR-196", "ADR-195", "ADR-191"]
source_review: 俯瞰レビュー（受動化・actuation撤去・学習/較正・config棚卸し・v2方針、2026-09-24）の A-1 / B-2 / B-3 / C-3 / C-6 / C-7（awase.exe 内の3経路の確認のみ）。元レビューはリポジトリ外（セッションの scratchpad）にしかないため、要点は本文に引用する
---

# 学習表の採用が awase.exe に効かない経路（俯瞰レビュー A-1 / B-2 / B-3 / C-3）

状態: **未着手**（B-2 は解消済み。A-1 は元レビューの失敗シナリオが誤っていたため書き直した）。

索引・優先度: [11](review-2026-09-24-11-low-priority-backlog.md)。
裏取り基準は worktree の `5877f982`（origin/develop、PR #296 まで。PR #293 `d00ac8dd` を含む）。
着手時は `.claude/rules/worktree-per-session.md` に従い専用 worktree/branch を切ること。

## 背景

[ADR-196](../adr/196-keymap-learn-truth-priority.md) 決定1e（`:134-139`）は「段階4（awase.exe の読込時）は、学習セッションが
永続化した判定結果を読むだけにする。破損ファイルへの防御として、同じ値を読み直す以上のことはしない（判定のやり直しはしない）」と定める。
突き合わせ・再測定・95%判定・要確認判定は学習プロセス（`awase-keymap-learn-win`）が行う。

一方、awase.exe の読込（`crates/awase-windows/src/state/key_effect_runtime.rs::validate_and_convert`、`:473-497`）は、
`judgement == Accepted` の確認に加えて次の2つの判定をやり直している。

- カバレッジ判定: 変換できたセル数 ÷ 表の生セル数 < `MIN_COVERAGE_RATIO`(0.80) なら `CoverageTooLow`（`:484-486`）。
- 内蔵表との不一致判定: `check_against_bundled`（= `KeyEffectKeymap::is_unmodified_bundled_config()`、`key_effect_predictor.rs:625-633`）が真なら、
  `mismatch_ratio > MAX_MISMATCH_RATIO`(0.05) で `MismatchesBundledTooMuch`（`:488-495`、定数 `:51`）。モジュール doc `:9-17` もこの3段の判定を前提に書かれている。

棄却されると内蔵表へ戻る（学習前と同じ予測になる。belief を壊す方向の害はない）。問題は「採用した／採用済みと表示される表が実際には使われない」ことと、ADR の原則（内蔵表を審査官にしない）に反することの2点。

## 現状（5877f982 で裏取り済み）

### A-1 の2つの比率は別物（元レビューの「30%超は必ず5%超」は誤り）

| 量 | 計算場所 | 分子 | 分母 |
| --- | --- | --- | --- |
| 学習側の系統的不一致率（30%で要確認） | `awase-keymap-learn/src/judgement.rs:213-219` `residual_mismatch_rate` | 再測定で**再現しなかった**セル（`NotReproduced`） | 一致＋再現＋再現せず |
| awase.exe の不一致率（5%で棄却） | `key_effect_runtime.rs:219-240` `mismatch_ratio` | 学習表に残っていて内蔵表と食い違うセル | 両表に存在し比較できたセル |

- 学習側は、再現しなかったセルの `prediction` を `None` に落としてから書き出す（`awase-keymap-learn-win/src/main.rs:740-749`）。
  `convert_cell` は `prediction: None` を変換しない（`key_effect_runtime.rs:167`）ので、30%の分子は5%の分子に入らない。
- 5%の分子に入るのは、再測定で**再現した**セル（`ReconfirmedByRemeasurement`、`judgement.rs:163-165`。ADR-196 が「学習値を採用する」と決めたセル）。
- したがって「SystematicMismatch の表は採用しても必ず5%で棄却される」は成り立たない。

### A-1 が実際に起こすこと（構成別）

`check_against_bundled` が真になるのは、GJI の同梱3プリセット（無改造）と、変換/無変換を再割り当てしていない Microsoft IME 本体
（`key_effect_predictor.rs:553-566` `for_msime_native` → `KeymapPreset::MsImeNative`）。学習側で内蔵表と突き合わせるのは GJI の既知構成だけで、
`gji_charset_autodetect.rs:373-384` `bundled_preset_for_adjudication` は `tip != Gji` なら `NotKnown` を返す（MS-IME 本体は突き合わせない）。

1. **GJI 既知構成（学習側で突き合わせ・再測定済み）**
   - (a) 再現したセルが比較セルの5%を超えると、**自動で Accepted になった表でも**棄却される。これは ADR-196 決定1e と「再現した差分は学習値を優先」の両方に反する。要確認→採用の表も同じ。
   - (b) 再現しなかったセルが落ちた分、カバレッジが下がる。落ちたセルが多いと `CoverageTooLow` で棄却されうる。
   - 実例: GJI+ATOK の CI では「再現せず0件」（元レビュー C-6、run 35931602595。出典は作業メモでリポジトリ内に記録なし＝**未確認**）。再現したセル数と、実表が awase.exe の判定を通るかどうかはリポジトリ内に記録がない（**未確認**）。
2. **Microsoft IME 本体（学習側で突き合わせない）**
   - 表は正答率95%以上かつ縮退率20%以下（`judgement.rs:99-105`）なら常に `NeedsConfirmation(UnverifiedMsImeNative)`（`judgement.rs:107`）。採用すると Accepted になり、awase.exe で `MSIME_NATIVE` との5%判定が**初めて**かかる。この構成では5%判定が内蔵表との唯一の比較で、実質的な安全網になっている。
   - 到達可能性: 5モード仮説モデル（PR #294、`408ef6ba`）以降、10回中9回が0.95以上で要確認まで到達（[adr196-t2-msime-learning-open-issues.md](adr196-t2-msime-learning-open-issues.md) 未解決2）。ばらつきは [adr196-t2-msime-hidden-state-hypothesis.md](adr196-t2-msime-hidden-state-hypothesis.md)。
   - 採用後の `MSIME_NATIVE` との不一致率は**未確認**。`ci/adr196-t2-msime-adopt-verify` の WF は `--adopt-pending-judgement` の成功までしか見ておらず、`load_runtime_table` には通していない。
   - MS-IME 本体の表には、`Conv` で表せないモード（半角カタカナ 0x03・全角英数 0x08 を `mode_from_raw_conv` が保持）の開状態セルがある。`convert_cell` はこれを捨てる（`:170-174`）ので、この分もカバレッジを下げる。
3. **カスタム構成**: `check_against_bundled=false`。5%判定はかからず、カバレッジ判定だけがかかる。

### 閉状態セルの畳み込みでカバレッジが下がる — **実測で確定（2026-09-24、タスク0）**

- `ef76bf9a`（2026-09-24、閉状態セルの潰れ修正）以降、`convert_cells`（`:117-135`）は同じ `(stage, key)` の閉状態セルを1セルに畳む（`merge_closed_cells` `:141-163`）。
  一方、`coverage_ratio`（`:206-213`）の分母は畳む前の生セル数。閉状態のモードが k 種類あると、全セルが予測ありでも閉状態分は 1/k しか数えられない。
- 学習側は閉状態でも変換モードを保持する（「open=false なら conv を0x00扱い」の正規化は撤去済み、open-issues 文書）。閉状態が複数モードに分かれていれば、**構成に関係なく**実表のカバレッジが80%を割りうる。
- `ef76bf9a` の回帰テスト（`closed_cells_differing_only_by_hidden_mode_collapse_order_independently` など）は `convert_cells` の結果だけを見ていて、`validate_and_convert` のカバレッジは見ていない。実表を通す CI テスト `ci_real_learned_table_is_adopted_for_unmodified_atok`（`:1023-1057`、`cdb90018`）は `ef76bf9a` より前に書かれたもの。`ef76bf9a` 以降の実表で通るかは**未確認**。

**実測結果（`ci/task01-measure`、run 35987424778、windows-latest。develop `1a6bdec8` + CI専用テストの出力拡張のみ。`validate_and_convert` は無変更）**

| 構成 | raw | 畳む前に変換可 | 閉/開（畳む前） | 畳んだ後 | coverage | 不一致率 | 採否 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| GJI+ATOK（accepted、verify 0.997） | 78 | 74 | 26 / 48 | 61 | **0.782** | 0.000（比較60セル） | **棄却 `CoverageTooLow`** |
| MS-IME 本体 run1（要確認→採用、0.957） | 143 | 112 | 49 / 63 | 76 | **0.531** | 0.000（比較59、旧`==`比較では1件） | **棄却 `CoverageTooLow`** |
| MS-IME 本体 run3（要確認→採用、0.970） | 143 | 111 | 50 / 61 | 74 | **0.517** | 0.000（比較59） | **棄却 `CoverageTooLow`** |
| MS-IME 本体 run2 | — | — | — | — | — | — | 正答率0.930で Rejected（採用対象外） |

- 疑いは**確定**。GJI+ATOK でも、畳む前は 74/78=0.95 だが、閉状態26セルが13セルに畳まれて 61/78=0.782 になり、80%を割る。MS-IME 本体はさらに、`Conv` で表せない開状態セル（143→112）の脱落が加わり 0.5 台。
- 5%判定（不一致率）は両構成とも 0.000 で問題にならない。**採用を阻んでいるのはカバレッジ判定だけ**。したがって現状、学習表は GJI+ATOK でも MS-IME 本体でも awase.exe に使われず内蔵表へ戻る。案A の5%判定の議論より、カバレッジの分母修正（畳んだ後に変換対象になりえたセル数へ揃える、または学習側の判定へ一本化）が先。
- 注記: 畳む前の閉セルの `conv()` の種類数は1と出力されたが、26→13 に畳まれているため、同じ `(stage,key)` を分けている軸は conv 以外（例: open 側の別属性）。畳み込みの内訳の確認は残る。
- 出力の取り方: CI 専用テスト `ci_real_learned_table_is_adopted_for_unmodified_atok` に環境変数 `KL_PRESET`（`msime-native`/`msime`/既定Atok）を追加し、`CI-RESULT` 行に上の値を出す（検証専用ブランチのみ。develop 未反映）。

- 起きていれば、学習表はどの構成でも awase.exe に使われない。A-1 の構成別の議論より優先度が高い。**最初に実測する**（タスク0）。

### B-2: 要確認表の採用経路 — **解消済み（PR #293、`d00ac8dd`）**

- `--adopt-pending-judgement` は `keymap-learn-last-attempt.json`（`main.rs:91`）を優先して読み、昇格後に消すようになった（`main.rs:363-392` `adopt_pending_judgement_at`）。
  `git branch -a --contains 4f3291a8` に `remotes/origin/develop` が含まれることを確認。単体テストは `main.rs:1199-1293`。
- 旧記述の `main.rs:389` は `revalidate_table`（再検証モード）の `read_failed` で、採用モードの箇所ではなかった（現在は `:447`）。
- 注意: `docs/tasks/develop-weekly-code-review-2026-09-23.md` の「B-2」（閉状態セルの潰れ、`ef76bf9a`）は別件。番号が衝突するので混同しない（上の「疑い」はその修正の副作用の可能性）。

### B-3: 書き込み通知・フック生存確認（決定1b-2 / 1b-4）は未配線

- [adr196-t2-msime-learning-open-issues.md](adr196-t2-msime-learning-open-issues.md) 未解決1: MS-IME 本体は状態変化でも `WM_IME_NOTIFY` を EDIT へ送らない。
  必須にすると全試行が無効、警告止まりでも `measurement_suspicious` で失敗する。配線は `0d4d9f80` で撤去済み（HEAD に含まれる）。
- 本タスクの範囲では「MS-IME 本体には内蔵表との比較が awase.exe の5%判定しかない」ことの理由として扱う。生存確認の再設計そのものは open-issues 文書の担当。

### C-3: 構成別の防御（元レビューの一覧を構成別に直した）

| 防御 | GJI 既知構成 | MS-IME 本体 | カスタム構成 |
| --- | --- | --- | --- |
| 1a 自己検証95% | あり（読める窓でしか採点できない） | あり（同） | あり（同） |
| 1b-7/8 突き合わせ＋再測定 | あり（同じコードで測るので系統的なバグは見つからない、ADR 自身が認める） | **なし**（`NotKnown`） | なし（内蔵表が無い） |
| 1b-2/1b-4 生存確認 | 未配線（GJI での挙動も未検証） | 未配線・原理的に不成立 | 未配線 |
| awase.exe の5%判定 | あり。ただし棄却するのは「再現した差分」で、ADR の原則に**反する** | あり。**唯一の内蔵表比較** | なし |
| awase.exe のカバレッジ80% | あり（1e に反する。閉状態セルの疑いあり） | あり（同） | あり（同） |

「A-1 を消すと安全網がなくなる」が当てはまるのは MS-IME 本体だけ。GJI 既知構成では5%判定は安全網ではなく、ADR と逆の判断をしている。
ただし、学習時とは別の IME・別のプリセットに切り替えた場合は、今の5%判定が事実上その表を棄却している（下の案Aの「プリセットの照合が前提」を参照）。

### C-7: awase.exe 内の3経路は同じキャッシュで揃っている（確認済み）

- 予測（`runtime/key_pipeline.rs:1979` の `kp_predict_key_effect`）、ADR-192 の状態依存キー警告（`runtime/mod.rs:1249` `learned_cells_for_warning`）、不具合報告（`runtime/message_handlers.rs:1411-1412`）は、どれも同じ `RuntimeTableCache`（`runtime/mod.rs:330`）を同じ検証キー `(preset, check_against_bundled)` で引く。
- したがって本タスクで `validate_and_convert` の判定を変えれば、3経路とも自動的に追随する。追加の配線は要らない。
- 揃っていないのは設定画面だけで、これは [02](review-2026-09-24-02-settings-status-display.md)（A-2）の担当。

## 推奨案

### 案A（推奨）: 「学習側で内蔵表と突き合わせ済みか」で5%判定を分ける

- 学習側が、内蔵表と突き合わせたかどうか（`reconcile_against_bundled` が `Some(summary)` を返したか）を表に書き出す。
  例: `PersistedTable` に `reconciled_against_bundled: Option<…>` を `#[serde(default)]` で追加する。中身には**突き合わせた内蔵表のプリセット名を必ず含める**（`ReconciliationSummary` の件数は任意）。
- awase.exe は、突き合わせ済みの表には5%判定をかけない（ADR-196 決定1e どおり）。突き合わせていない表（MS-IME 本体・旧ファイル）には5%判定を残す。
- **プリセットの照合が前提**: 突き合わせ済みの記録は**学習時の** preset に対するもの。一方、`validate_and_convert` の5%判定は**今の** preset の内蔵表と比べている（`key_effect_runtime.rs:488-489`、`bundled_table(preset)`）。
  今の5%判定は、別 IME・別プリセットで学習した表を事実上棄却している。記録だけで5%判定を外すと、GJI ATOK で学習して突き合わせ済みになった表が、利用者が後から GJI の MSIME プリセットや MS-IME 本体に切り替えても使われてしまう。
  そのため awase.exe は、記録のプリセット名が今の preset と一致しない表を「突き合わせなし」として扱う（5%判定を残す）。これは [06](review-2026-09-24-06-keymap-learn-staleness-wiring.md) の「preset を含む指紋の照合」と役割が重なる。06 を同時か先に入れ、指紋の照合で preset 不一致の表が棄却されるなら、プリセット名の照合はどちらか一方に寄せる（ADR 改訂で決める）。
- 利点: 「利用者が採用したか」より事実に近い（どの審査を経たかをそのまま記録する）。自動 Accepted の表の扱いも同時に決まり、先送りがない。
- MS-IME 本体を採用後に5%判定から外すかは、実測（タスク0）で `MSIME_NATIVE` との不一致率を見てから決める。外すなら、それを ADR-196 の改訂で明記する。

### 案B（元の提案、単独では不採用）: 利用者の採用フラグで分ける

- 利用者が採用した表だけ5%判定を外す。これだけでは、自動 Accepted の GJI 表が再現した差分で棄却される問題（1-(a)）が残る。
- MS-IME 本体に限った補助として使うなら、次の2点を満たすこと。
  - `adopt_needs_confirmation`（`judgement.rs:260-270`）は Accepted の表にも冪等に成功を返す。フラグは `NeedsConfirmation` から書き換えたときだけ付け、既に Accepted の表では変えない。
  - フラグの無い旧ファイルの Accepted 表は「利用者の採用なし」として扱う（安全側）。

### カバレッジ判定の扱い

- カバレッジ判定も ADR-196 決定1e の「判定のやり直し」にあたる。残すなら ADR の改訂で「破損防御の一部として残す」と明記する。
- 分母を「畳んだ後に変換対象になりえたセル数」に揃えるか、学習側の縮退率（`DEGENERATION_THRESHOLD`、分母が別の量。`judgement.rs:20-23`）に一本化するかを決める。

### スキーマ版は上げない

- `persist.rs:154` はスキーマ版を完全一致で比べる。上げると既存の v2 表はすべて `SchemaVersionMismatch` で内蔵表に戻り、再学習が必要になる。
- 前例（`persist.rs:73`「追加のみでスキーマ版は上げない（`#[serde(default)]`で旧ファイルも読める）」）に従い、`Option` と `#[serde(default)]` で追加する。
  [06](review-2026-09-24-06-keymap-learn-staleness-wiring.md) の指紋・IME種別の追加も同じ方式にする。

## タスク

- [x] **0（最優先・実測、完了 2026-09-24）**: 実機の学習表を `validate_and_convert` に通し、採否と理由（`CoverageTooLow` の値／`mismatch_ratio`）を記録する。
  - GJI+ATOK: `ef76bf9a` 以降の表で `ci_real_learned_table_is_adopted_for_unmodified_atok` を再実行する。
  - MS-IME 本体: 同じテストはプリセットが `Atok` 固定（`:1028` の `bundled_table(KeymapPreset::Atok)`、`:1049` の `load_runtime_table(path, KeymapPreset::Atok, true)`）。環境変数でプリセットを選べるようにし、要確認→採用後の表を `MsImeNative` で通す。
  - 生セル数・閉状態のモード数・畳んだ後のセル数も出力して、上の「疑い」が実際に起きているかを確定する。
  - 結果に応じて本文の「未確認」を更新し、下のタスクの優先度を決め直す。
- [ ] ADR-196 を改訂する（または追補 ADR を起票する）: 1e と awase.exe の5%判定・カバレッジ判定の関係、1b-2/1b-4 が MS-IME 本体では成り立たないこと、案A（突き合わせ済みかの記録）を書く。プロジェクトの慣行に従い、`opus-adversarial-consult` で収束させてから実装する。
- [ ] 学習側: `reconcile_against_bundled` の結果（`Some`/`None`）を、突き合わせたプリセット名と一緒に `PersistedTable` へ `#[serde(default)]` の `Option` として永続化する（スキーマ版は上げない）。
- [ ] awase.exe 側: `validate_and_convert` で、突き合わせ済みで、**かつ記録のプリセットが今の preset と一致する**表には5%判定をかけない（06 の指紋の照合に寄せる場合は、06 がこの条件を満たしていることを確かめる）。カバレッジ判定は ADR 改訂の結論に合わせて直す（閉状態セルの分母を揃える／学習側の判定に一本化する）。
- [ ] 既存テストを直す: `mismatch_against_bundled_is_rejected_when_checked`（`:669-686`、突き合わせ済みかどうかの2通りに分ける）と、CI 専用テスト `ci_real_learned_table_is_adopted_for_unmodified_atok`（`:1023-1057`）。
  CI 専用テストを develop のどの WF が流しているかは**未確認**（develop の `.github/` に `KL_TABLE_PATH` の参照はない。`cd863408` で `adr196-t2-remeasure-verify.yml` 側は `load_runtime_table` の直接呼び出しに置き換えた記録がある）。
- [ ] 古いコメントを直す: `key_effect_runtime.rs` のモジュール doc `:9-17`、`judgement.rs:28-30`（「現時点でこの定数を実際に使う呼び出し元はまだ無い」→ `main.rs:667` の `judge_score` が使っている）。
- [ ] 暫定策（次リリースに間に合わず、タスク0で「採用しても使われない」ことが確かめられた場合だけ）: [02](review-2026-09-24-02-settings-status-display.md) の画面で「採用しても内蔵表に戻る場合があります」と出す、または「学習結果を使う」を隠す。同梱するかどうかは [03](review-2026-09-24-03-release-bundle-keymap-learn-win.md) の判断による。

## 受け入れ条件

- **Linux（`cargo test -p awase-windows --lib`、`key_effect_runtime.rs` は `#[cfg(windows)]` 外で host 実行可）**: `validate_and_convert` の単体テストを、次の軸で合成データから組む。
  - (a) 突き合わせ済み・再現した差分が比較セルの5%超 → 採用される。
  - (b) 突き合わせなし（MS-IME 本体・旧ファイル）・同条件 → 案Aで残した判定どおり棄却される。
  - (b') 突き合わせ済みだが、記録のプリセットが今の preset と違う（例: ATOK で突き合わせ、今は MSIME プリセット）・同条件 → 棄却される。
  - (c) 閉状態が複数モードに分かれ、全セルが予測あり → カバレッジで棄却されない（分母を直す場合）。
  - (d) 再現しなかったセルが落ちてカバレッジが80%付近 → ADR 改訂の結論どおりに振る舞う。
- **Linux（`cargo test -p awase-keymap-learn`）**: 突き合わせ結果の新フィールドが `#[serde(default)]` で旧 v2 ファイルからも読めること（`persist.rs` の既存の後方互換テストと同じ形）。
- **Windows ターゲットのコンパイル確認**: `awase-keymap-learn-win`（学習側の書き出し）の変更は `cargo check --target x86_64-pc-windows-msvc -p awase-keymap-learn-win --tests` で確認する。採用モードの単体テスト（`main.rs:1199-1293`）の実行は windows-build CI 任せ。
- **windows-latest CI（検証専用ブランチ、develop へはマージしない）**: タスク0の実測を、GJI+ATOK と MS-IME 本体（要確認→採用後）の両方で行い、結果を本文に反映する。
  MS-IME 本体は正答率が0.95に届いた run だけが対象（届かない run は Rejected で採用の対象外）。
- **実機で「GJI の要確認表を採用する」経路を自然に再現する手順は無い**（GJI の CI では再現せず0件で、SystematicMismatch に落ちた実例がない）。この経路は上の Linux の単体テストと採用モードの単体テストで確かめる。
- ルール: 対象ファイル `state/key_effect_runtime.rs` は `.claude/rules/fix-requires-evidence.md` の表にまだ無い。しかし `KeyEffectPredicted` 経由で belief を動かすので「IME belief」ファミリーに準じて扱い、回帰テストを必ず付ける（表への追加は [10](review-2026-09-24-10-adr-status-and-stale-docs-sync.md) の B-8 が担当）。

## 他ファイルとの依存（矢印は「先に決める側 → 後で使う側」）

- 01 → [02](review-2026-09-24-02-settings-status-display.md): 設定画面の表示は、本タスクで決める最終判定（awase.exe が実際に採用するか）を共有関数として使う。
- [06](review-2026-09-24-06-keymap-learn-staleness-wiring.md) → 01: 案Aで5%判定を外すなら、06 の「preset を含む指紋の照合」を同時か先に入れる（06 `:74`・索引 11 と同じ向き）。突き合わせ済みの記録は学習時の preset に対するものなので、preset が変わったら無効にする必要がある。プリセット名の照合を 01 の記録と 06 の指紋のどちらに寄せるかは ADR 改訂で決める。
  また、同じ `PersistedTable` にフィールドを足す。どちらも `#[serde(default)]` で追加し、スキーマ版は上げない（06 の「スキーマ版は1回だけ上げる」はこの理由で見直しが要る）。
- 01 → [03](review-2026-09-24-03-release-bundle-keymap-learn-win.md): 学習機能を同梱して見せるかの判断材料（タスク0の実測結果）。
- [10](review-2026-09-24-10-adr-status-and-stale-docs-sync.md) → 01: B-8（`fix-requires-evidence.md` と pre-push に `key_effect_*` を追加する）が入れば、本タスクの修正は正式にルールの対象になる。
- [adr196-t2-msime-learning-open-issues.md](adr196-t2-msime-learning-open-issues.md) → 01: MS-IME 本体の要確認到達（正答率）と生存確認の再設計。MS-IME 本体側のタスク0と「5%判定を外すか」はこちらの進み具合に依存する。
- 01 → [08](review-2026-09-24-08-open-close-fixed-set-vs-custom-keymap.md)（08 が (B) 案を採る場合のみ）: 今の開閉の書き込みは学習表を使わない（ADR-195(A)）。08 が (B) 案を採るなら、本タスクで「学習表の採用が効く」ことが前提になる（08 `:115`・索引 11 `:30` と同じ）。08 が (B) を採らなければ依存はない。

## 未確認点

- （解消 2026-09-24）`ef76bf9a` 以降の実表は GJI+ATOK・MS-IME 本体とも `CoverageTooLow` で棄却されると実測で確定（「閉状態セルの畳み込み」節）。
- GJI+ATOK の実表で、再測定で**再現した**セルの数と、awase.exe の `mismatch_ratio`（1-(a) が実際に起きるか）。
- MS-IME 本体の表を採用した後の `MSIME_NATIVE` との不一致率。
- CI 専用テスト `ci_real_learned_table_is_adopted_for_unmodified_atok` を現在どの WF が流しているか。

## レビュー反映メモ（Opus タスク文書レビュー、2026-09-24）

反映した指摘（すべて `5877f982` のコードで裏取りした）:

- 1（30%と5%は別物）: `judgement.rs:213-219`・`main.rs:740-749`・`key_effect_runtime.rs:167,219-240` で確認。A-1 を構成別に書き直し、「必ず無効」を削除した。
- 2（MS-IME 本体は SystematicMismatch にならない）: `gji_charset_autodetect.rs:380-384` で確認。MS-IME 本体を別の構成として分けた。
- 3（B-2 解消済み）: `4f3291a8` が `remotes/origin/develop` に含まれること、PR #293 `d00ac8dd` を確認。解消済みに変更した。索引 11 の「(5) 未マージ」の記述も古いが、11 は担当外なので本文書では直していない（11 の担当者へ申し送り）。
- 4（安全網の言い過ぎ）: C-3 を構成別の表にした。
- 5（代案）: 案Aとして本文に取り込み、推奨にした。採用フラグ案は案Bとして、不採用の理由と使う場合の条件を書いた。
- 6（カバレッジ判定も 1e と食い違う）: 取り込んだ。さらに裏取りの途中で、閉状態セルの畳み込み（`ef76bf9a`）と分母の食い違いによる新しい疑いを見つけ、タスク0（実測）にした。
- 7（スキーマ版を上げない）: `persist.rs:73,154` で確認。`Option` と `#[serde(default)]` に決めた。
- 8（採用フラグの冪等性・旧ファイル）: `judgement.rs:260-270` で確認。案Bの条件として書いた。
- 9（既存テストの変更）: タスクに追加した。WF については develop の `.github/` に参照が無いので「未確認」とした。
- 11（ルールの表に無い）: `fix-requires-evidence.md` で確認。準用と 10 への依存として書いた。
- 12（GJI の要確認表は実機で再現できない）: 受け入れ条件で、単体テストで代わりに確かめると明記した。
- 13（書式）: 既存の docs/tasks の流儀に合わせて「状態:」行を足した（YAML frontmatter は同じ日の 01〜11 と揃えるため残した）。
- 14（`source_review` が辿れない）: frontmatter に注記し、必要な要点は本文に引用した。元レビューをリポジトリに置く作業は、担当1ファイルの範囲外なので行っていない。
- 細部: `main.rs:389` は再検証モードの箇所だったので訂正した。`judgement.rs:28-30` の古いコメントの修正をタスクに加えた。

反映しなかった／修正して反映した指摘:

- 10（「MS-IME 本体は正答率0.940で Rejected なので、要確認→採用は起こりえない」）: **古い**。PR #294（`408ef6ba`、5モード仮説モデル）以降、10回中9回が0.95以上で要確認まで到達する（open-issues 文書の未解決2）。したがってシナリオ2は現在起こりうる。「0.95に届いた run に限る」という条件だけ受け入れ条件に取り込んだ。

## レビュー反映メモ（Opus 再確認レビュー、2026-09-24）

すべて `5877f982` のコードとタスク文書で裏取りしてから反映した。

- R1（06・11 との依存の向き）: 反映。06 `:74` と 11 `:23` は「06 → 01」で、01 の依存節だけが前提条件を落としていた。`validate_and_convert` の5%判定が**今の** preset の `bundled_table(preset)` と比べること（`key_effect_runtime.rs:488-489`）を確認した。案Aの記録にプリセット名を必須にし、今の preset と一致しなければ「突き合わせなし」として扱う条件を、推奨案・タスク・受け入れ条件 (b')・依存節に入れた。
- R2（C-7 の記述が無い）: 反映。`runtime/key_pipeline.rs:1979`・`runtime/mod.rs:1249` `learned_cells_for_warning`・`runtime/message_handlers.rs:1411-1412` が同じ `RuntimeTableCache`（`runtime/mod.rs:330`）を同じキーで引くことを確認し、C-7 節を足した。
- R3（行番号）: 反映。`:1044-1045` は `println!`。Atok を固定しているのは `:1028` と `:1049`。
- R4（08 の依存）: 反映。08 `:115` と 11 `:30` はどちらも「01 → 08（(B) 案のみ）」なので、01 側を合わせた。
- R4（06・02 に残る「採用フラグ」前提）: 担当1ファイルの範囲外なので直していない。**申し送り**: 06 の `:52`（「01 の採用フラグの版上げ判断」）・`:74`（「01 の採用フラグで5%判定を飛ばすと」）と、06 の `:50`（b-1「スキーマ版を上げて」）は、01 の推奨が案A（突き合わせ済みの記録＋プリセット名、スキーマ版は上げない）に変わったことに合わせて直す必要がある。02 の `:109`（「01 が採用フラグを `validate_and_convert` の外に分離した場合」）と、索引 11 の `:24`（02 行の「採用フラグ」）も同様。案Aでは判定は `validate_and_convert` の中に残るので、02 は追随の対象外になる見込み。
- 補足（95%判定の条件）: 反映。`judgement.rs:99-105` で、`NeedsConfirmation(UnverifiedMsImeNative)` の前に縮退率の判定があることを確認し、「95%以上かつ縮退率20%以下」に直した。

位置づけの再評価（初回レビュー反映時に書いたもの。再確認レビューで変更なし）:

- 元レビューは A-1 を「採用操作が構造的に必ず無効になる、今すぐ直す筆頭」としていた。しかしその根拠（30%⊂5%）は誤りで、B-2 も解消済み。棄却されても内蔵表に戻るだけで、belief を壊す害はない。このため優先度を「中〜高」に下げ、最初の一手を「実測（タスク0）」にした。
- ただし、閉状態セルのカバレッジの疑いが実測で確かめられれば、学習表はどの構成でも使われないことになる。その場合は次リリース前に直す筆頭へ戻す。
