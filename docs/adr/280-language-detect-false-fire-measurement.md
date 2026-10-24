---
id: ADR-280
title: |-
  入力言語の判定の誤発火を実機の前に数値で測り、不可逆な入り(半角英数トグル)の害を小さくする候補を、その数値で比べて選ぶ
summary: |-
  2026-10-23〜24 の dragonflyg4 の不調の調査で「記号 1 打で半角英数トグルに入った」事故(PR #819)が見つかった。調べ直すと、原因は (i) 記号の -33 nats の減点(#819 で直す)と
  (ii) 利用者が自分の責任で選ぶ `ime_on_eisu` + `scope = all` の組み合わせで、窓クラスの区別(`scope`)・eisu 専用の閾値(`eisu_threshold` 既定 14)・
  証拠の信頼度の型(ADR-089 の `Pool`/`CONFIDENCE`)は**すでに実装にある**。本 ADR の初版は、これらを見落として新しい証拠の型と開の確証の条件を提案しており、
  Opus レビュー round 1 で退けた(下記「却下した案」)。
  本 ADR が決めるのは次の 3 つだけ。
  (D1) 判定器の誤り率を測る評価をリポジトリ内に置き、率を数値でテストに固定する(`real_model()` が仮名表を引くことの検査を含む)。
  (D2) 判定と入りの純粋な関数(`HoldController` の `HoldStep`・`plan_entry_for_language_detect`・`write_skip_reason`)の全列挙 golden(ME-3 型)で、今の挙動を固定する。挙動は変えない。候補を確かめる道具としては限られる(C1 だけ。本文の表)。
  (D3) 事故の再現条件(NICOLA 入力中の窓切替なしの無操作のあと、UI で `ime_on_eisu` を選ぶと必ず付く `scope = all`)に効く候補 C0〜C5 を、測って(または所有者の判断で)選ぶ。この ADR では選ばない。コードの変更ゼロの C0(`eisu_threshold` を上げる)から測る。
  窓クラス別の信頼度・擬似窓の「読み取りの嘘のプロファイル」は、D1〜D3 で足りない失敗が実機の journal で示せたときの別 ADR に回す(ADR-089 の宣言表に足す形で)。
status: |-
  起草(2026-10-24)→ Opus レビュー 3 回で収束 → 所有者の判断済み → **段階 0〜2 を実装済み**(PR #823 C0 の測定・#824 D2・#825 journal 項目・#826 D1、いずれも 2026-10-24 に develop へマージ)。
  **C4 は前提が誤りと分かり却下**(Edge は Imm32Unavailable で TSF ネイティブではない。下記)。C0(`eisu_threshold = 16`)を dragonflyg4 で実機試行中(2026-10-24 設定)。
  D3 の残り(C1・C3、C2 は見送り)は C0 の結果を見て決める。
related_adr:
  - "ADR-089"
  - "ADR-263"
  - "ADR-268"
  - "ADR-273"
  - "ADR-277"
  - "ADR-278"
  - "ADR-279"
---

# ADR-280: 入力言語の判定の誤発火を数値で測り、不可逆な入りの害を小さくする

## ステータス

起草(2026-10-24)。実装なし。D3 の候補はまだ選ばない。

## 背景

### 事故(2026-10-23〜24、dragonflyg4、awase-latest `c57509e5`)

PR #819 の本文より: Edge(本文は「tsf-native」と書くが、コードでは Imm32Unavailable。下の C4 の却下を参照)・GJI で、4.7 秒の無操作のあと、親指なしの K・A・`¥` の 3 打が `verdict="not-japanese" llr=-43.1` になり半角英数トグルへ入った。別に、窓切替の 1 打が `llr=-33.2` で `NotJapanese` に確定した(その打鍵で半角英数に入ったかは、journal の `action` で未確認)。

- **直接の原因**: `KanaModel::delta`(`src/lang_detect.rs`)が、仮名に読めないキー(記号・数字・未定義)に約 -33 nats の下限を課していた。#819 は、これを 0(中立)にする。
- **構成要因**: 設定画面の 5 択(`crates/awase-settings/src/main.rs` の `AUTO_DETECT_PRESETS`)は、`ime_on_eisu` を **`scope = all` とだけ**組み合わせて出す(`ime_on_eisu + unobserved` は選択肢に無く、config を手書きしたときだけ)。`scope = all` が TSF ネイティブの窓を含むようになったのは `2097538a`(2026-10-22、BUG-200 の救済が目的)で、Edge(tsf-native)の事故はその後に起きている。#819 が足す ADR-263 の追記(未マージ)も、journal の構成を `ime_on_eisu`・`scope = "all"` と書いている。`ime_on_eisu` と `scope = all` は、どちらも `src/config.rs` の doc で「利用者の責任」で選ぶ設定。
- **閾値**: `ime_on_eisu` の入りの実効の閾値は `max(eisu_threshold, en_threshold)`(`lang_detect_shadow.rs:79-83`)で、既定は `max(14, 8) = 14`。-33.2・-43.1 は 14 を大きく超えるので、事故の説明は変わらない。dragonflyg4 の `config.toml`(`C:\Users\cuzic\awase-latest\config.toml`、開発ビルドなのでワークスペース直下)の `[auto_detect]` は `mode = "ime_on_eisu"`・`scope = "all"` だけで、`eisu_threshold` の記述は無い(2026-10-24 に clipwire で取得)ので、既定の 14 が効いている。
- **事故の再現条件**: 「NICOLA を打っている途中の無操作 4.7 秒」(`idle_ms` 既定 3000)。窓は変わっていない。

### #819 の評価(出典: PR #819 の本文。評価スクリプトは `research-private/v3-eval/eval8_symbols.py`、リポジトリ外)

読み付きの日本語 1400 文を文中から 4 点で打ち始め、親指なしの連続区間を判定にかけた結果。

| 指標 | `en_threshold` | 値 |
|---|---|---|
| 日本語→英数の誤判定(`NotJapanese` に達する率) | 8(現行の `ime_on` 側) | 22.1% → 7.0%(#819) |
| 同(**`ime_on_eisu` の入りの実効の閾値**) | 14 | **1.0%**(#819 の表の値) |
| 英数側が早く流れる率(英文) | 8 / 10 / 14 | 96.8 / 94.6 / 85.3% |

注意: 「英数側が早く流れる率」は、`max_keys` 以内に `NotJapanese` に達した率であって、「英文を英数と正しく判定する率」ではない。不可逆な入りの誤発火の率として引くべきは、実効の閾値 14 の **1.0%** である。これらの数値は、評価がリポジトリの外にあるので、すべて**未検証扱い**とする(D1 で取り直す)。

### 現状のコード(origin/develop `c57509e5` で確認)

- `EpisodeDetector`(`src/lang_detect.rs`)は純粋。結果は `Verdict::{Pending, Japanese, NotJapanese, Undecided}`。
- 窓クラスの区別は**ある**: `lang_detect_window_in_scope`(`InputRelay` は常に対象外、`scope = unobserved` は実測学習で降格された窓だけ)。`WriteFacts.unobserved_window` と `skipped-observed-window` に入る。
- 不可逆な入り専用の閾値は**ある**: `eisu_threshold`(既定 14)。
- 書き込みの可否は共通の `write_skip_reason`(`src/lang_detect.rs:829`)で、`hiragana_write_skip_reason`・`eisu_write_skip_reason` は理由の名前が違うだけ。条件は、Engine が活性(`belief_open = lang_detect_engine_active()`。**IME の開閉ではなく、Engine が活性か**)・対象の窓・NonText でない・フォーカスが追いついている・逆向きの明示意図が無い・修飾キーが押されていない。
- 半角英数への入りは、さらに `entry_ime_ok = effective_open && is_japanese_ime && is_user_enabled`(`lang_detect_shadow.rs:337-339`)と `HalfWidthAlnumState::plan_entry_for_language_detect`(`entry_policy`・`toggle_held`・`pending_for_scope`)を通る。
- 証拠の型は**ある**: `crates/awase-ime-core/src/state/evidence.rs`(ADR-089)の `declare_evidence!` が、source ごとに `Pool`(`ActuatingPool` = IME への書き込みの根拠にしてよい / `BeliefPool` = belief にだけ使える)と `CONFIDENCE`(High/Medium/Low)を宣言する。`MozcShm`・`TaskbarGlyph` は `BeliefPool`(ADR-268・273)。
- 擬似窓のハーネス(`tests/support/harness.rs`)は、`observe_value`(嘘・古い状態の読みを流す)と `Source::ConvInference`(GJI×TsfNative の誤った「開」の推測)を既に持つ。ただし lang_detect の経路(`HoldController`・`WriteFacts`・`HalfWidthAlnumState`)は呼ばない(grep で 0 件)。
- journal には `JournalEntry::LangDetect`(`profile`・`belief_open`〈こちらは `effective_open()` で、`WriteFacts` の同名のフィールドとは別の値〉・`action`〈skip の理由を含む〉)が既にある。

### 動機の例の仕分け(初版は 3 例すべてを同じ設計で救えると書いたが、誤り)

| 例 | 経路 | この ADR で救えるか |
|---|---|---|
| 記号 1 打で半角英数に入った(#819) | `ime_on_eisu` の入り | #819(記号を中立に)と `eisu_threshold`。残る誤発火は D1〜D3 の対象 |
| LINE: 切替直後に IMM が conv=0・ime_on=false を返し、Engine が止まる(**出典未確認**。手元の docs は LINE を測っていない) | belief OFF(実 IME ON)。Engine が非活性で、保持も判定も書き込みも起きない | **救えない**。「belief OFF の窓で、打鍵列から日本語と判定したら戻す」向きで、ADR-279(段階 0 の結果で P0 止まり)の範囲 |
| Edge: belief ON のまま字形は closed | 既定の `scope` の対象外。`scope = all` のときだけ関係し、経路は未確認 | **救えない**(未確認) |

## 決定

### D1: 判定器の評価をリポジトリ内に置き、率を数値で固定する

- 評価(日本語の打鍵列・英文・記号を含むコード・他言語)の生成・取得の手順を `tools/` に置く(Python は `uv run`)。**利用者の本文を含めない**。評価データの出典・ライセンス・サイズ・CI への影響を、実装の PR の本文に書く(読み付きの日本語 1400 文の出典は、今は確認できていない)。
- 次の率を、**指標の定義を分けて**テストに固定する。合格値は、#819 のマージ後に取り直した測定で決める(今ある値は上の表だけで、未検証)。
  - 日本語を `NotJapanese` と誤判定する率を、`en_threshold` の値別に(`ime_on` 側の 8 と、`ime_on_eisu` の入りの実効の閾値 14 を**別の指標として**)。
  - 英文・コードを `Japanese` と誤判定する率。今の 0.2% は #819 より前の測定(`DetectParams::default` の doc)で、記号が中立になった後は増えうるので、取り直す。
  - 英数側が早く流れる率(上の表の指標)。
- `real_model()` が仮名表を引くこと(`resolve_kana()` を呼ぶこと)の検査は #819 が足す(`real_model_reads_kana_keys_and_treats_symbols_as_neutral`)。D1 はこれを前提にし、足し直さない。

### D2: 判定と入りの純粋な関数の全列挙 golden(挙動は変えない)

決定表の全列挙(ADR-278 の status が後継として挙げる ME-3。`docs/tasks/mech-ecrs-2026-10-21/raw-e-pseudo-ime.md`、ADR-277 の流儀)を 3 本置く。主目的は**今の挙動の固定**で、D3 の候補の移行を確かめられるのは C1 だけ(C1 は `EpisodeTracker` の入力が増えるので、golden の入力の型ごと変わる)。他の候補は、下の表の「確かめる道具」で確かめる。

| golden | 列挙する入力 | 動かす候補 |
|---|---|---|
| `HoldController::on_key`/`on_timer` の `HoldStep`(`english`・`japanese`・`release`) | 契機(窓切替/idle)× 判定(Japanese/NotJapanese/Undecided)× 保持の可否。判定を入力にするので、判定器(`EpisodeDetector`)は golden の外 | C1。**C3 は出ない**(判定器の確定の条件を変えるので、D1 の評価と判定器の単体テストで確かめる) |
| `HalfWidthAlnumState::plan_entry_for_language_detect` | `entry_policy`(Off/MsImeOnly/All)3 × `toggle_held` 2 × bool 3(`entry_ime_ok`・`uses_imc_conv_write`・`pending_for_scope`) = 48 通り | 入りの条件を変える変更。**C2 は出ない**(C2 は入った後に出る経路を変える案。出る側の純粋な関数を新しく作るなら、その golden を C2 の PR で足す) |
| `write_skip_reason`(共通。`WriteFacts` の bool の優先順位) | 各フィールドの組 | 関数自体の退行の検出のみ。C0〜C5 は通常この関数に触れない(C4 を `WriteFacts` のフィールド追加で実装するなら触れる)ので、D3 の移行の確認には使わない |

- 今のテスト(`src/lang_detect.rs:1449-1522`)は真偽表の抜粋で、後の変更が挙動を変えても検出できない。
- 殻の `WriteFacts` の組み立て(`entry_ime_ok` の `effective_open()` を含む)は、architecture_guard の source-scan と実機 CI で見る(Linux からは見えない)。
- 擬似窓の全列挙には載せない(ハーネスが lang_detect の経路を呼ばず、lang_detect の殻は `#[cfg(windows)]` で Linux に存在しないため、ADR-278 の見送り理由と同じ構造になる)。載せるなら、ME-3 型の golden が検出できない失敗を 1 つ示すこと。

### D3: 事故の再現条件に効く候補を比べて選ぶ(この ADR では選ばない)

事故の再現条件は「NICOLA を打っている途中の、窓を変えない無操作」と「UI で `ime_on_eisu` を選ぶと必ず付く `scope = all`(InputRelay 以外の全ての窓)」。それに効く候補を並べる。コードの変更が要らないものから順に測る。

| 候補 | 内容 | 利点 | 懸念・測り方 |
|---|---|---|---|
| C0 | `eisu_threshold` を 14 より上げる(16・20 など) | **コードの変更ゼロ**(config の既定値。`tuning.rs` の定数ではないが、実測を添える流儀に従う) | 英数側の保持が長くなる。D1 で 16・20 の誤判定率と遅れを測る(#819 の表は 14 までしか無い) |
| C1 | 直前のエピソードが `Japanese` だった、または直前まで Engine が仮名を出していたら、**窓が同じ**の idle の契機では `ime_on_eisu` の入りをしない | 事故の条件に正面から効く。窓クラスの表は要らない | 直前の文脈を持つ状態が増える(`EpisodeTracker` の入力に文脈を足す形で、判定器の純粋さを保てるか)。**D1 の評価(文単位)にも今の journal にも文脈が無く、測れない**(下記) |
| C2 | lang_detect が入れた半角英数で、親指と文字キーの重なりが起きたら、トグルを出る(誤りを自動で戻す) | 不可逆を可逆にする。ADR-263 の「誤判定の害は少し遅れるに収まる」という設計思想に合う | 入ると Engine が非活性(`ObservedEisu`)で、重なりを見る主体が無く、殻に新しい監視が要る。親指が IME のキー(無変換/変換)だと `excluded` になる(ADR-279 の Opus レビューが同じ問題を指摘)。非活性中の親指が IME に届いて状態を変える構成がある(ADR-279 段階 0 の S0-2)。戻り方に既に IME のキーがある(`lang_detect_shadow.rs:324`)ので、その構成では既に出ている可能性があり、実機の確認が要る |
| C3 | 入りの条件に「仮名に読めたキーが N 打以上」を足す | 判定器の中で完結する。短い列(2〜3 打)の確定を止める | N の決め方に実測が要る。英文の判定が遅れる |
| C4 | ~~`ime_on_eisu` の入りのときだけ、TSF ネイティブの窓を対象から外す~~ **却下**(2026-10-24。前提の誤り) | — | Edge(`Chrome_WidgetWin_1`)は `AppImeProfile::Imm32Unavailable` で、`is_effectively_tsf_native` は偽(class 名の表 `is_tsf_native_window` は 5 つで Chrome を含まない。`chrome_is_not_effectively_tsf_native` が固定している)。事故の窓に効かない。`scope = all` では Edge は InputRelay 以外の全ての窓の 1 つとして対象になる。journal の `profile` の値の実機確認は未実施(PR #819 の「tsf-native」の表記との食い違いが残る) |
| C5 | UI の 5 択から `ime_on_eisu` を外す(英数と判定したら何もしない `ime_on` にとどめる) | 分岐が減る。`mode`×`scope` の意味のある 6 通り(`off`・`observe` は `scope` が効かない)中、UI で使われているのは 5 通りで、`ime_on_eisu + unobserved` は UI に無い | 半角英数への自動の入りという機能が無くなる。所有者の判断(`scope = all` に TSF ネイティブを含めたのは 2097538a) |

**C1・C2 を測るために足す journal の項目**(足さないなら、C1・C2 は「測って選ぶ」ではなく所有者の判断で選ぶ候補として扱う):
- `LangDetect` に、同じ窓の直前のエピソードの verdict とそこからの経過時間(C1)。
- 入った後の最初の親指の重なりまでの打鍵数(C2)。
- `entry_ime_ok` の `effective_open()` が、どの観測(`derive_open` か `desired_open` へのフォールバックか)から来たかの内訳(D4 の保留を解く判定にも使う)。

D3 の選択は、D1 の数値と所有者の判断を受けて、別の PR で決める。

### D4(保留): 窓クラス別の信頼度・擬似窓の嘘のプロファイル

D1〜D3 で足りない失敗(belief ON・実 IME が閉の窓で半角英数に入った、など)が、実機の journal で件数つきで示せたときに限り、別 ADR で検討する。UI の構成は `scope = all` で、読める窓と読めない窓が混在する(読める窓は `Standard` の Win32 の窓など。Chrome は通常 `Imm32Unavailable` で IMM の読みは門前払いされる。`ImmGetOpenStatus`/`ObserverPoll` の `ActuatingPool` の観測がある)が主な対象になるので、判定は D3 の journal 項目(`effective_open()` の内訳)で行う。その場合も、新しい証拠の型は作らず、ADR-089 の宣言表(source × profile)に足して、belief と書き込みのゲートが同じ表を引く形にする。擬似窓側は、既存の `observe_value` と `Source` の上に、窓ごとの嘘を宣言して自動で流す仕組みを載せる(嘘を作る能力は既にある)。

### C0 の測定結果(#823、2026-10-24)

実効の閾値 `max(eisu_threshold, en_threshold)` ごとの率(リポジトリ外の `research-private/` の文による評価。その文はエージェント生成で、読みは未検証、出典・ライセンスも確認できていない。#819 の表を 14 までは再現: 日本語→英数 7.02/3.48/1.57/0.95%、英文の判定率は 0.1〜0.9 ポイントのずれ)。

| 実効の閾値 | 日本語→英数の誤判定 | 英文 NotJapanese 率(平均打鍵/p90) | 英文 Undecided | コード NotJapanese 率 |
|---|---|---|---|---|
| 14(既定) | 0.95% | 84.9%(4.70/7) | 15.1% | 73.6% |
| 16 | 0.43% | 77.6%(5.09/7) | 22.4% | 67.9% |
| 18 | 0.22% | 70.7%(5.33/7) | 29.2% | 60.3% |
| 20 | 0.15% | 61.4%(5.60/7) | 38.6% | 53.0% |
| 24 | 0.03% | 42.8%(6.01/7) | 57.2% | 34.7% |

推奨は 16(誤入り半減、英数側の保持は平均 +0.4 打鍵)。足りなければ 18 まで。20 以上は英文の約 4 割が決まらない。誤った `NotJapanese` の一部が `Undecided`(何もしない)に変わる、という方向の変更で、害は少し遅れる程度に収まる。

## 段階

| 段階 | 内容 | 終わりの判定 |
|---|---|---|
| 0 | **完了**(2026-10-24): dragonflyg4 は `ime_on_eisu`・`scope = all`・`eisu_threshold` 未設定(既定 14)。#819 はマージ済み。Edge の分類はコードで確認(Imm32Unavailable。C4 却下) | 実効の閾値が確定した |
| 1 | **実装済み**(#826): `tests/lang_detect_eval.rs` と `tools/lang-detect-eval-fixture/`、`tests/fixtures/lang_detect_eval.tsv`(44KB、seed 280、`ngram_hiragana` と配列から生成)。出典・ライセンスは**確認できなかった**(`research-private/` の資料は「エージェント生成、読みは未検証」とだけ)ため、生のコーパスは使っていない。日本語の列は判定器自身の分布から作るので、リポジトリ外の文による評価(#819。8 で 7.0%、14 で 1.0%。この文もエージェント生成で未検証)より楽観的で、アサートは**退行の検出用**であって絶対的な誤り率ではない。D1: 評価をリポジトリに置き、率を固定。**入口条件**: 読み付きの日本語 1400 文の出典・ライセンスを確かめる。確かめられなければ、ライセンスの明らかなコーパスか、`data/ngram_hiragana` の元データからの生成に代える | CI で率が固定される |
| 2 | **実装済み**(#824): 全列挙 3 本(`HoldStep` 12+8+4 通り、`hiragana`/`eisu_write_skip_reason` 各 64 通り、`plan_entry_for_language_detect` 48 通り)。置き場所は `crates/awase-windows/tests/` に変更(`awase-hook-core/tests/` とルートの `tests/` は CI で走らず、`core-layers` と `ci_test_coverage_guard` に落とされた)。`ci.yml` の `--test` に 2 本を足した。期待値の表と実装の食い違いは見つからなかった。D2: 3 本の全列挙 golden | 現行の挙動が golden で固定される |
| 3 | D3: **C0 は測定済み**(#823、`docs/tasks/adr280-c0-eisu-threshold-measurement-2026-10-24.md`。下の表)で、dragonflyg4 で `eisu_threshold = 16` を実機試行中。**journal の項目は追加済み**(#825: C1 用の `prev_verdict`・`prev_elapsed_ms`、D4 用の `belief_basis`。C2 用は、入った後に Engine が非活性で重なりを見る既存の経路が無く新しい監視が要るため見送り)。C1・C3 は C0 の結果を見て所有者が決める。C2 は所有者の判断。C5 は最後 | 候補ごとに数値、または判断の記録がある |

## 却下した案(初版の内容を含む)

- **初版 D1: `Evidence{source, value, reliability}` の新設。** ADR-089 の `Pool`/`CONFIDENCE` と二重になる。`Glyph`/`MozcShm` は `BeliefPool` なので、これを書き込みの根拠にすると ADR-268・273 の方針(drift correction の根拠から除外する)に反する。LINE の IMM は belief 側で `ImmCrossProbe => ActuatingPool, High` で、新しい層が `Weak` にすると、belief とゲートの 2 か所に正解の置き場所ができる。`Evidence.source` に `Belief` を並べると、同じ事実を二重に数える。窓クラスで信頼度の差が要るなら、ADR-089 の宣言表に足す(D4)。
- **初版 D2(b): 不可逆な書き込みに「Trusted な開の証拠が 1 つ以上」を要求する。** 主な理由は、事故(実 IME は ON のまま、日本語を英数と誤判定)に効かないこと(開の確証は「実 IME が閉なのに入る」失敗を防ぐもの)。副次的に、カスタム構成の既定の `scope = unobserved`(実測学習で IMM が読めないと判定された窓だけ)では、定義上、信頼できる IMM の読みが無く、字形も既定で読まない(`EvidenceLevel` 既定 off)ので、入りが止まり、「初期値は現行と同じ」と矛盾する。UI から選べる `ime_on_eisu + all` の構成には、この副次的な理由は当てはまらない(読める窓では `ActuatingPool` の観測がある)。
- **初版 D5: 擬似窓の全列挙の golden の差分で移行を確かめる。** 上の D2 のとおり、擬似窓のハーネスは lang_detect の経路を持たず、判定する対象が無い。
- **初版の可逆/不可逆の 2 値。** ひらがなキーも英数キー 1 打で戻せ、半角英数への入りも Shift のタップなどで戻れる。本当の非対称は、誤発火したときに失う打鍵数と、続く長さ(半角英数に入ると、気づくまで仮名が Latin のまま確定する。ADR-245 の戻り待ちで窓をまたいで持ち越されることもある)。可逆性の区分で閾値を決める案は、この「失う打鍵数」の実測が出るまで採らない(D3 の C2 が、この害を小さくする側の案)。
- **belief 全体を証拠の束にする。** 範囲が大きく、belief への書き直しを増やさない所有者の方針(2026-10-23)と逆向き。
- **`en_threshold` を全体で上げるだけ。** 英数側が早く流れる率が 96.8% → 85.3%(14 のとき)に下がる。`ime_on_eisu` の入りは `eisu_threshold`(独立した設定、14 より上げられる)で決まるので、`en_threshold` を上げる意味があるのは `ime_on` 側だけで、そちらは書き込みをしない。入り専用のつまみは C0(`eisu_threshold`)として候補に入れた。
- **`mode` を `observe` に戻す。** 所有者が却下(異常に気づけなくなる)。

## 擬似窓・擬似 IME を使った再設計を、本 ADR では行わない理由

所有者の要望は「擬似窓・擬似 IME を使って、ゼロベースで簡素に再設計したい」だった。本 ADR はそれに応えていない。理由:

- 入力言語の判定側は、すでに純粋な関数(`EpisodeDetector`・`HoldController`・`write_skip_reason`・`plan_entry_for_language_detect`)で、ゼロベースで作り直して得るものが少ない。
- lang_detect の判断は、すでに純粋な関数として Linux で直接テストできる。擬似窓を挟んで得るものが無い。
- lang_detect の殻の配線(`WriteFacts` の組み立て)は `runtime/` の下で `#[cfg(windows)]` なので、擬似窓からも見えない。(belief は `awase-ime-core` にあり OS に依存しない。擬似窓のハーネスはすでにそれを Linux で動かしている。)
- 簡素にする候補は C5(分岐が減る)。

擬似窓・擬似 IME を使う再設計の対象は、lang_detect ではなく belief・書き込みの合流点の側にある(belief は Linux で擬似窓が届くので、そちらの再設計なら所有者の要望に沿える)。それは D4 の保留を解いた後、または別の ADR で扱う。

## リスク・未確認

- 事故の構成は `ime_on_eisu + scope = all`・`eisu_threshold` 既定 14 と確認済み(2026-10-24)。Edge の journal の `profile` の値は未確認(PR #819 の「tsf-native」とコードの Imm32Unavailable の食い違い)。
- C0 の評価は文単位で、無操作のあとの再開という C1 の対象の文脈は測れない。#819 の事故(llr -43)は #819 で解消済みなので、C0 が効くのは残る約 1% の「仮名の珍しい並び」だけ。
- LINE・Edge の読みは、この ADR では救えない(上の表)。LINE の切替直後の読みの出典は、手元の docs(`adr279-stage0-result`・`spike-m1m2`)に無い(どちらも LINE を測っていない)。journal の報告 ID を添えて別途扱う。
- C1 の「直前の文脈」は、`EpisodeTracker` に状態を足す。純粋さを保つ形(入力に文脈を渡す)を段階 3 の最初に確かめる。
- D1 の評価データの代表性。人工データは実利用の打鍵列と違う。実ジャーナル由来の分は、利用者の本文を含めずに集める手順が要る。
- `docs/adr/index.md` に 280 の行を足す(起票時の規約)。実装の PR ではなく、この ADR を出す PR で足す。

## 所有者の判断(2026-10-24)

1. 範囲は D1〜D3 に絞る。擬似窓・擬似 IME を使った再設計は本 ADR で行わず、別の ADR で扱う。窓クラス別の信頼度(D4)は保留。
2. 事故への対策は C0 → C4 の順で試す(C5 は最後)。→ C4 は前提の誤りで却下(2026-10-24)。C0 の結果を見て C1・C3 を検討する。
3. dragonflyg4 の `eisu_threshold` の実際の値は、clipwire で取得する(段階 0)。→ 取得済み: 未設定で既定 14。
