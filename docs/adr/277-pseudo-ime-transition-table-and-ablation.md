---
id: ADR-277
title: |-
  擬似 IME の遷移を 1 つの表にまとめ、全列挙・ファズ・防御の ablation を同じ表で回す
summary: |-
  撤去・統合の「この防御は要るか」を、実機 CI(1 回数分〜十数分・構成数に上限)を待たずに Linux で答えるため、閉ループの擬似 IME を
  (a) 状態を持たない遷移関数 `TransitionTable::step` 1 つと宣言的な規則表(`KEPT_STAGE_RULES`)に整理し、閉ループ・全列挙・ランダム探索が同じ表を引くようにする、
  (b) 窓モデル(窓ごとの profile・クラス・プロセス・スレッド・読める/読めない)を足して、同じプロセス内でクラスが変わる場面(DD3-01)を作れるようにする、
  (c) 防御を外すスイッチは、ハーネスが別の呼び出しとして呼ぶ防御だけをテストから切り替え、核の関数の内側の条件は `cargo mutants` に任せる(本番にテスト用の分岐を足さない)、
  (d) 防御の目録を機械可読にし(`tests/support/defenses.rs`)、外すと違反が出る最短の事象列を自動で出す。
  段階 1(本 ADR と同時の PR)は (a) の抽出、到達性と予測の全列挙 golden、(c)(d) の 1 件(BUG-163 代案A)だけ。段階 1 の全列挙で、MS-IME の 2 格子の conv=0x1B の実測セルを 16 進の大文字小文字の違いで全て取りこぼしていた不具合が見つかり、直した。
status: |-
  起草・段階 1 実装(2026-10-22、PR は本文参照)。段階 2 以降は未着手。
related_adr:
  - "ADR-163"
  - "ADR-191"
  - "ADR-205"
  - "ADR-208"
  - "ADR-224"
  - "ADR-225"
  - "ADR-226"
  - "ADR-241"
---

# ADR-277: 擬似 IME の遷移を 1 つの表にまとめ、全列挙・ファズ・防御の ablation を同じ表で回す

## 背景

所有者の要望(2026-10-22): 擬似 IME の設計を改善した上で、functional core で網羅的に検証する仕組みにし、撤去作業の検証を時間効率よく進めたい。

前回の調査 `docs/tasks/mech-ecrs-2026-10-21/raw-e-pseudo-ime.md`(ME-1〜ME-7)は「ハーネスが本物を通る範囲」を整理した。本 ADR はその隣の問い、**擬似 IME 自身の形**と、**防御ごとの要否を機械的に答える形**を扱う。ME-1(写しを `decide_drift_plan` 等の本物に置換)・ME-2(ADR-241 PR-B)は本 ADR の前提ではなく並行の作業で、本 ADR の段階 3 で効く。

### いまの擬似 IME が再現するもの・しないもの(origin/develop `a8634232` で確認)

| 項目 | 現状 | 根拠 |
|---|---|---|
| 開閉 × conv × 入力中の段階 | 実測の格子 3 種を真値に使う。atok 132 セル(11 状態)、msime 303 セル(25 状態)、msime-native 227 セル(21 状態)。全セルが多数派 1 種類(割れたセル 0) | `tools/e2e/ime_key_matrix/grid-tables/*.json` を数えた |
| 押下後の段階 | 格子に無い。Space→変換中 等の**仮定**(`next_stage`)。BUG-205 はこの仮定と予測器の食い違い | `pseudo_ime.rs` の doc |
| 実測の無い状態からの押下 | panic(推測で埋めない)。ランダム探索は `knows` で飛ばす | `pseudo_ime.rs::press`・`closed_loop_scenarios.rs:710` |
| クセの目録 QUIRKS | 6 件、うち模すのは 3 件。Q3(読み戻し遅延、シナリオ 3 本)・Q4(観測側、`Setup::external_close_watch`、2 本)・Q6(書き込みだけ、シナリオ 0 本) | `pseudo_ime.rs::QUIRKS` |
| cold/warm・TSF の composition/literal | 無い(TSF warmup を通らない) | `harness.rs` の doc |
| 外から閉じる | `external_set_open`(見ていない経路)と `external_injected_key`(他プロセスの注入、ADR-205 の監視窓を開く) | `harness.rs` |
| フォーカス遷移 | `focus_change` は IME を共有のまま別の窓へ。`focus_change_to_new_thread_unreadable` だけが IME を置き換える特例。窓・スレッドの表は無い | `harness.rs` |
| 窓ごとの profile | 構築子は `imm_cross`・`imm32_unavailable` の 2 つ。TsfNative・Plain・InputRelay の窓は使用 0 件。同一プロセス内のクラス変化は作れない | `harness.rs::Setup` |
| awase の書き込み | `write_open` は格子の `ime-off`/`ime-on` セルを引かず「開閉だけ変える」単純化。ImmCross の `IMC_SETOPENSTATUS` と GjiDirect の VK を区別しない | `pseudo_ime.rs::write_open` |
| 防御を外す手段 | 外部変化の監視(ADR-205)のスイッチ 1 つだけ。ほかはソースを書き換えるか `cargo mutants` | `harness.rs::Setup` |

### 段階 1 で見つかった不具合(擬似 IME 側)

格子の JSON は状態名の conv を大文字で書く(`on-c1B-none`)が、擬似 IME は `{:02x}`(小文字)でセル名を作っていた。このため msime.json・msime-native.json の conv=0x1B の実測セル(各格子 7 状態 × 13 キー)は全て「実測なし」扱いになり、ランダム探索は `knows` が偽を返して**黙って飛ばしていた**。到達性の全列挙(下の決定 1)で見つかった。確認: `grep -c '"on-c1B' tools/e2e/ime_key_matrix/grid-tables/msime.json` は 0 でない、`git show a8634232:crates/awase-windows/tests/support/pseudo_ime.rs | grep -n '{:02x}'`。

## 決定

### 決定 1(a): 遷移は状態を持たない関数 1 つにし、全列挙・ファズ・閉ループが同じ表を引く

- `TransitionTable::step(状態, ImeInput, QuirkSwitches) -> Transition` を唯一の遷移関数にする。中身は格子(実測、JSON のまま)+ `KEPT_STAGE_RULES`(段階の仮定を `&[(vk, 段階)]` の表にしたもの)+ 文字キー・書き込み・外部変更の規則。入力のアルファベットは `ImeInput::{Press(vk), Write(open), External(open)}`。
- `PseudoIme` は状態と時刻に依るクセ(Q3)だけを持ち、遷移はこの関数を呼ぶ。全列挙(`TransitionTable::reachable`)・ランダム探索(`knows`)・閉ループが同じ表を引くので、表を直せば全部に効く。
- DSL・TOML・マクロは作らない(ADR-225 で見送った線。所有者の過去の指摘「実害の記録と削減量の実測の無い DSL 化は勧めない」)。Rust の定数表と既存の JSON だけ。
- **これで検証できること**: 実測の無いセルのうち到達しうるものの一覧(穴)。格子の測定をどこへ足すか、シナリオがどこで panic するかが golden で分かる。**効く撤去候補**: 予測器の段階の追跡(`KeyTrack`)の規則の整理。BUG-205 型の「仮定と予測器のずれ」を、表 1 か所の変更として全列挙で確かめられる。

### 決定 2(b): 窓モデルを足し、同じプロセス内でクラスが変わる場面を作る(段階 2)

- ハーネスに `Desktop { windows: hwnd → Window { pid, thread, class, live_profile, readable }, threads: thread → PseudoIme }` を持たせる。`focus_to(hwnd)` は、プロセスが変わったときだけ `FocusChanged{profile}` を流し(本番の `ImeModel.app_policy` の更新契機)、殻の `live_profile`(`FocusTracker.current.app_profile` 相当)は窓のクラスが変わるたびに変える。`class_change(hwnd, class)` で同一プロセス内のクラス変化を作る。
- スレッドごとに `PseudoIme` を持ち、`focus_change_to_new_thread_unreadable` の特例を一般化する(新しいスレッドは閉 × ひらがな、ADR-191 §3.2)。
- ハーネスが殻の値を使う所(`PredictInput.unreadable`・drift の ImmCross 判定)は `live_profile` を、核の `ImeModel` は自分の `app_policy` を読む。2 つが食い違う事象列で P1・P3・最終の belief の一致が破れるかを全列挙で見る。
- **これで検証できること**: DD3-01(「読めない窓か」の 2 つの写しの更新契機のずれ)の害の有無を Linux で言える。**効く撤去候補**: DD3-01・DD3-03(読めないかの述語の写し)の統合。害が出なければ片方へ寄せる根拠になる。実現可能性の詳細は並行の担当(窓モデル)に委ねる。

### 決定 3(c): ablation は「ハーネスが別の呼び出しとして呼ぶ防御」だけテストから切り替える

- 外せる: ハーネス(=殻の写し)が**独立した呼び出し**として呼ぶ防御。例: `align_placeholder_desired`(BUG-163 代案A)、外部変化の監視(ADR-205)。`Setup::defenses`(`Defenses`)のフィールドで切り替える。
- 外せない: 核の関数の**内側**の条件(例: `issue_open_warrant_inner` の `is_japanese_ime` 棄却)。本番のコードに `cfg(test)` やフィーチャーの分岐を足して外す形は採らない。理由: (1) 本番にテスト専用の分岐が増え、それ自体が撤去対象の複雑さになる。(2) 分岐の有無で本番とテストのコードが変わり、「外したら違反が出る」が本番の要否を言わなくなる。(3) 既存の `cargo mutants`(`.cargo/mutants-awase-ime-core.toml` ほか)が「その条件を消したらテストが落ちるか」を既に答える。目録には変異の場所(`file`・関数名)を書き、`cargo mutants -F <関数名>` で回す。
- 外す意味が無い: ハーネスが本物を呼ばず写している防御(例: drift の「ImmCross で warrant が下りなければ検知の手前で見送る」`b6ab8980`)。写しを外しても本番の要否は言えない。ME-1 で本物(`decide_drift_plan`)に置き換えてから外す。
- **これで検証できること**: 呼び出し単位の防御 1 件ごとに「外すと違反が出るか」を数秒で答える。**効く撤去候補**: BUG-163 の二重の防御(代案A の揃えと `b6ab8980` の見送り)。段階 3 で後者を本物に置き換えれば、どちらか一方で足りるかを言える。

### 決定 4(d): 防御の目録を機械可読にし、外すと違反が出る最短の事象列を自動で出す

- `crates/awase-windows/tests/support/defenses.rs::DEFENSES`: 1 件ごとに id・場所・外し方(`HarnessSwitch`/`Mutant`/`NotReached`)・破れる不変条件・根拠。`NotReached` は「閉ループはまだ要否を言えない」の明示。
- `HarnessSwitch` の防御は、事象列を短い順に全列挙し(幅優先、深さの上限つき)、(1) 外さなければ違反が無い、(2) 外すと違反が出る、の両方を確かめ、最短の列を golden に固定する。外しても違反が出なければテストが落ちる(防御が要らないか、事象の集合が足りない)。
- **これで検証できること**: 目録のどの防御が閉ループで「要る」と示せ、どれがまだ示せないか(`NotReached`・`Mutant`)の一覧。**効く撤去候補**: `HarnessSwitch` に載った防御すべて(撤去の PR は「外すと最短 N 手で破れる」が golden から消えないことを確かめる)。

## 段階 1 の実装範囲(本 ADR と同時の PR)

- `tests/support/pseudo_ime.rs`: `TransitionTable`・`ImeInput`・`QuirkSwitches`・`KEPT_STAGE_RULES`・`reachable` を抽出。`PseudoIme` は表を呼ぶだけにした(挙動は 0x1B の修正以外は同じ)。状態名の 16 進を大文字に直した。
- `tests/pseudo_ime_exhaustive.rs`(新規、CI の `--test` に追加):
  1. 到達性の golden(`tests/golden/pseudo_ime_reachability.txt`): 3 格子の到達しうる状態の数と穴。例: ATOK は `next_stage` の仮定が「変換中(無変換)」を作るが格子にその状態が無い(26 セル)。msime-native は `on-c10-*` と `on-c13-conv-*` に穴がある(89 セル)。
  2. 予測の全列挙の golden(`tests/golden/pseudo_ime_prediction_mismatches.txt`): ランダム探索と同じ手順で、到達しうる抽象状態(真値・`key_track`・入力モード)を全部たどり、予測の食い違いを (格子・窓・押す前の真値・キー) の種類ごとに件数と最短のキー列で固定する。毎回ハーネスを最初から再生するので写しは増えない。初回の結果(CI run 39736837371): 抽象状態 55〜156/構成、食い違い 1,001 件・種類 297(6 構成の合計)、実行 14.6 秒。ランダム探索(360 本)が見つけていたのは BUG-204 の 32 件と 0x1B の英数キーの 2 件だけ。**未仕分け**: ATOK の先頭の種類は BUG-204(閉の間の入力モードの初期値)と同じ形だが、MS-IME 系の多く(conv=0x13・0x1B)はハーネスの観測の写し(conv が 0x10 のときだけ英数とする)に由来する偽の陽性の可能性があり、本番の不具合とは言えない。仕分けは段階 1 の次の作業。
  3. ablation の golden(`tests/golden/defense_ablation.txt`): `placeholder-align` 1 件。
- `tests/support/harness.rs`: `Setup::defenses`(`Defenses { placeholder_align }`)を足した。既定は全部有効で、既存のシナリオの挙動は変わらない。
- `tests/support/defenses.rs`(新規): 目録 4 件(外せる 1・`Mutant` 1・`NotReached` 2)。

## 次の段階

| 段階 | 内容 | 検証できるようになること / 効く撤去候補 |
|---|---|---|
| 2 | 決定 2 の窓モデル(`Desktop`・`focus_to`・`class_change`、スレッドごとの `PseudoIme`)と `Setup` の TsfNative/Plain/InputRelay の構築子 | DD3-01 の害の有無 / DD3-01・DD3-03 の統合 |
| 3 | ME-1 と合わせて、drift の写しを `decide_drift_plan` に置き換え、`drift-immcross-skip-unwarranted` を `HarnessSwitch` に昇格 | BUG-163 の二重の防御のどちらかで足りるか / `b6ab8980` の見送りの撤去 |
| 4 | ablation の事象に外部注入・prefetch・キー・明示操作を足し、`external-close-watch` を探索に載せる | ADR-205 の要否の最短列 / ADR-205 と mode-key-follow の重複の有無(mech-c) |
| 5 | `Mutant` の行から `cargo mutants -F` の引数を作る小さなスクリプト(目録を読むだけ) | 核の内側の防御の要否 / drift と warrant の `is_japanese_ime` の二重の関門(raw-e 1.3-2) |
| 6 | `write_open` を書き込み経路ごとに格子のセル(`ime-off`/`ime-on`)で引く(ME-2 の `CommandSink` と同時) | GjiDirect と ImmCross の書き込みの違い / 機構チェーンの整理 |

段階 1 の予測の全列挙はランダム探索と重なる。全列挙の golden が 1 回でも新しい食い違いを見つけるか、ランダム探索が全列挙に無い食い違いを見つけない状態が続いたら、ランダム探索の撤去を検討する(履歴に依るずれは抽象状態に入れていないので、すぐには消さない)。

## 見送り

- 擬似 IME の DSL・TOML・Rhai(ADR-225 で見送り済み、所有者の指摘)。
- 遅延の確率モデル(ADR-241 決定6)。
- 本番のコードに ablation 用のフィーチャーや `cfg(test)` の分岐を足すこと(決定 3)。
