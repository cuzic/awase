# ADR-193 敵対的レビュー round3

対象: `docs/adr/193-extend-existing-e2e-harness-for-chromium-coldstart.md`（commit `5502e223`）
および `docs/known-bugs/BUG-002.md`（同コミットで注記追加）

**判定: Blocker は解消。収束していないが、残るのは Major 3 件で、いずれも文面の修正のみ**
（設計判断の差し戻しは不要）。round2 の Blocker B1・Major M1〜M4 は実コードと照合して正しく
反映されている。新たに見つかったのは (1) per-VK confirm の参照先ファイル名の誤り、
(2) 「再現しないこと自体の回帰検知」に切り替える選択肢に陽性対照（canary）と ablation の
要求が無く、壊れたハーネスでも合格しうること、(3) `--settle` が keyboard idle と GJI idle を
同時に進めるため「まだできていないこと」1 の記述が不正確なこと。

---

## round2 指摘の反映確認

| round2 | v3 の対応 | 判定 |
|---|---|---|
| B1 撤去対象が存在しない | 「現状認識の訂正」に削除済みを明記、決定4にステップ0を新設、撤去対象は現行機構から特定し直す方針へ | **反映済み**。`tuning.rs:92-93`・`docs/experiments.md:339`（2026-07-18、`d495649`、「物理削除」）・`output/vk_send.rs:279` と一致 |
| M1 Chrome の long-idle は 5s | 5s/7s/10s の3閾値を明記、ステップ0の掃引点を 3s/6s/8s/11s に | **反映済み**。`tuning.rs:85,100,148`・`gji_fsm.rs:1012-1018` と一致（行番号 100/148 も正確） |
| M2 `--settle` で今日作れる | 「作れないわけではない」に改め、不足を3点に絞った | 方向は正しいが**1点目の内容が不正確**（下記 M3） |
| M3 接続は3点セット | ビルド・runステップ・判定に分解、`cache.toml` 事前投入が流用できない点も明記 | **反映済み**。`e2e-ime.yml:172-176`・`:271-299`・`:290` と一致 |
| M4 代替案の注意書き | `detect_app_kind` の doc は stale と明記、`InjectionModeStore` の汚染リスクへ差し替え | **反映済み**。`focus/classifier.rs:429-453`・`tracker.rs:115-123`・`probe_fsm.rs:242`・`platform.rs:482-484` と一致 |
| m2 本体ソースの書き分け | 「コミットされる本体ソースは新たには変更しない／増えるのは `ablations/*.sh` のみ」 | **反映済み**。決定4-3 と 4-4 の衝突は解消 |
| m4 Chrome 同梱 | `Test-Path` の確認手段を決定4-2 に記載 | **反映済み** |
| 楽観1（docs の現役性） | 「経緯」に教訓として明文化 | **反映済み**。同じ轍を踏まないための規律として妥当 |
| 楽観2（seq がある） | 未確認事項を POST の遅延1点に絞った | **反映済み**。`chrome_probe.rs:41-46` の `seq++` と一致 |
| 楽観3（マトリクス規模） | ケース数 × idle秒 × マトリクスで見積もると明記 | **反映済み**。`e2e-ime.yml:144-148`・`:188` と一致 |
| 楽観4（Tauri） | 「未検証の仮説として不採用」に修正 | **反映済み** |

すり替え・取りこぼしは無い。

---

## Major

### M1. `run_per_vk_confirm` の所在が誤り（`probe_coro_state.rs` ではなく `probe_fsm.rs:454`）

**根拠**

- ADR「現状認識の訂正」4点目: 「per-VK confirm(`tsf/warmup/probe_coro_state.rs::run_per_vk_confirm`、
  `tsf/warmup/literal_detect_fsm.rs`)に一本化された」。
- 実際の定義は `crates/awase-windows/src/tsf/warmup/probe_fsm.rs:454`
  （`pub(crate) async fn run_per_vk_confirm(`）。`grep -rn "fn run_per_vk_confirm"` のヒットは
  この1件のみ。
- `tsf/warmup/probe_coro_state.rs:4` は**モジュール doc で名前に触れているだけ**
  （「`run_per_vk_confirm`（2026-07-17 統合）で共通化済みだったが、その周辺の…」）。
  同ファイル `:116` も「per-VK confirm が1 VK 送信するたびに呼ぶ」というヘルパーの doc。
- `literal_detect_fsm.rs` は実在する（`tsf/warmup/` 配下）ので、こちらは正しい。

**失敗シナリオ**: 決定4 のステップ0→「撤去すると症状が戻る現行機構を特定する」が本 ADR の
次の行動そのものなので、実装者は真っ先にこのパスを開く。`probe_coro_state.rs` には
コルーチン状態の保持しか無く、撤去候補（送信ループと confirm の判定）が見つからない。
v2 の B1（存在しない定数を指していた）と同型の、ポインタだけの誤りである。

**修正案**: `tsf/warmup/probe_fsm.rs::run_per_vk_confirm`（`:454`）に直す。
撤去候補の探索範囲として、併せて `tsf/warmup/literal_detect_fsm.rs`（`is_partial_literal()` 系、
BUG-024 が構造的な偽陽性/偽陰性を指摘している関数）と
`output/vk_send.rs:279` 周辺（`[h1-probe] … F2/probe待機省略 → per-VK confirm へ` の分岐）を挙げておくと、
ステップ0 の後の作業が一意に決まる。

### M2. 「再現しないこと自体の回帰検知」への切替に、陽性対照（canary）と ablation の要求が無い

ADR 決定4-0 は「出ない → …撤去実験の対象は現行機構(per-VK confirm側)に取り直す**か**、本ADRの
成功基準を『再現しないこと自体の回帰検知』に改める」と書く。この **「か」が穴**である。

後者だけを選ぶと、成功基準が「literal 化が観測されないこと」＝**不在の assert** になる。
不在の assert は、ハーネスが壊れていても合格する:

- awase が起動していない／`AWASE_TEST_INJECTION=1` を付け忘れた（決定3 が警告している偽陰性そのもの）
- Chrome が前面に来ていない、ページがフォーカスを失った
- そもそもキーが1つも届いていない

このリポジトリは既にこの罠を知っていて、両方の既存ハーネスが対策を持っている:

- `crates/awase-windows/tests/e2e_windows.rs:2820-2838`: **ASCII canary**。素の ASCII が
  Read-Host に届いたかを先に確かめ、届いていなければ
  「this is a setup/typing problem, not something specific to the IME race.
  Skipping the rest of this test as **inconclusive rather than reporting a false result**」
  として打ち切る。
- `crates/awase-windows/examples/chrome_probe.rs:648-651`（`RESULT INVALID: 前提状態にできなかった`）、
  `:661-664`（`focus_lost` → `RESULT INVALID`）、`--no-awase` 対照腕（awase 停止時は `か` を期待）。
- `tools/e2e/ime_key_matrix/check_multi.py:6,51-54`: awase ログの `extra=0x0`（人の物理入力の混入）で INVALID。

さらに本質的な問題として、ablation を捨てると **そのテストが落ちうることを一度も示せない**。
`ablations/` の仕組み（`e2e-ime.yml:161-168` の「撤去が差分を作らなければ fail」）は、
まさに「このテストには検出力がある」ことを機械的に証明するためにある。

**失敗シナリオ**: ステップ0 で症状が出ず、成功基準を「出ないこと」に切り替える。CI に緑の
ジョブが1本増えるが、それは Chrome が起動しなくても、awase が落ちていても緑になる。
半年後に per-VK confirm を触って BUG-002 型を再発させても、このジョブは緑のまま通る。

**修正案**: 決定4-0 の分岐を「か」ではなく必須の積にする。

- 症状が出ない場合でも、**ablation は必須**とする（現行機構のどれを撤去すれば症状が戻るかを
  特定できて初めて、「出ないこと」の assert に意味が生まれる）。撤去しても症状が戻らないなら、
  その assert は何も守っていないので CI に載せない。
- 「出ないこと」を assert する回には、必ず**陽性対照**を同じ実行内に含める:
  `--no-awase` 腕（awase 無しでは `か` になる）と、`e2e_windows.rs` 型の ASCII canary の
  どちらか、または両方。対照が取れなければ FAIL ではなく **INVALID**（`chrome_probe` の
  既存語彙）に落とす。

### M3. 「まだできていないこと」1（GJI休眠の制御）の記述が不正確 — 足りないのは*分離*

ADR は不足を「1. GJI休眠の制御」と書くが、`--settle` を使う限り
**keyboard idle と GJI idle は同時に進む**。`chrome_probe.rs:653-654` は
`p.press(c.vk, c.shift, 120); sleep(settle_ms);` で、この間はキーも GJI I/O も発生しないため、
`--settle=11000` なら打鍵時点で keyboard idle ≈ GJI idle ≈ 11s になる。
つまり GJI 休眠は `--settle` で**既に作れる**。

作れないのは BUG-002 のもう一方の分岐、すなわち
「**keyboard short idle かつ GJI long idle**（＝物理 F2 + GJI 休眠）」である
（削除済みの旧対策表 `BUG-002.md` の2行目「keyboard long idle (>10s) **または** 物理 F2 + GJI long idle」）。
これを作るには、待ちを**モードキー押下の前**に入れる必要がある（長く待つ → F2 → 即座に打鍵）。
`chrome_probe` にはその位置のフラグが無い（`--settle` は押下の後）。

**失敗シナリオ**: 「GJI休眠の制御」という曖昧な項目のまま着手し、`--settle` で既に作れるものを
作り直す（round2 M2 と同型の重複作業）。あるいは逆に、`--settle` で両方同時に進むことに
気づかず「GJI 休眠だけを長くしたつもり」で測り、2つの条件を分離できていない結果を
「long idle でも再現しない」と解釈してしまう。

**修正案**: 1 を「keyboard idle と GJI idle を**分離**する手段（モードキー押下の**前**に待つ
`--pre-settle` 相当。`--settle` は押下の後なので両者が同時に進む）」に書き換える。
ステップ0 の掃引も、`--settle`（両方 long）と `--pre-settle`（GJI のみ long）の2軸で書くと、
BUG-002 の旧表の2分岐と1対1に対応する。

---

## Minor

### m1. ステップ0 は既存の CASE 4 がそのまま使える（1コマンドで始められる）

`chrome_probe.rs:440-447` の4番目のケース
```
Case { name: "半角英数→ひらがな=かな", setup: Setup::Alnum, vk: 0xF2, shift: false, expect_kana: true },
```
は「半角英数の状態から `VK_DBE_HIRAGANA`(0xF2) を押して、かなになるか」であり、
BUG-002 の形（F2 → 待ち → ローマ字打鍵）とそのまま一致する。ADR に
「ステップ0 は `chrome_probe --settle=11000`（CASE 4/8）で開始できる」と書けば、
実施者が探す手間が消える。

### m2. `Class` の粒度では部分リテラルを取りこぼす。ステップ0 は生値を見ること

`chrome_probe.rs:237-248` の `Class` は
`Plain` / `RomajiKana` / `Nicola` / `NicolaLiteral` / `Empty` / `Other` の6値で、
判定入力は `k`,`a` の2打のみ。BUG-002 の `という→toいう` は**先頭1モーラだけ**が
リテラル化する部分リテラルなので、`k`,`a` 相当では `Other` に落ちる公算が高く、
Class だけ見ていると「分類不能」で流れてしまう。

一方、生の `t.value` はページ側 `ev()`（`:41-46`）が全イベントに添えて POST しており、
`chrome_probe.log` に残る。したがって**ステップ0 に必要なコード変更は無く**、
「Class ではなく生の `t.value` を見る」と書けば足りる。自動判定（不足3点目）を作る段になって
初めて、入力を `k`,`a` から多モーラ列（`toiu` → `という` 等）に拡張する必要が出る。
ADR の不足3点目に「入力列の拡張（`k`,`a` の2打では部分リテラルを表現できない）」を足すこと。

### m3. `BUG-002.md` の注記 — 内容は正確。ただし3点

**正確性（裏取り済み）**: 注記の主張はすべて実コード/実ドキュメントと一致する。

- 「2026-07-18 の BUG-024 対応で物理削除」: `tuning.rs:92-93`（「2026-07-18 に撤去した
  （BUG-24 参照、per-VK confirm に一本化）」）、`docs/experiments.md:339`
  （2026-07-18 の行、「物理削除」、commit `d495649`）。BUG-024.md の `fix_commits` にも `d495649` がある。
- 「`tuning.rs` に存在しない」: `grep -rn "CHROME_PROBE" --include=*.rs` のヒットは 0 件。
- 「Chrome(VK) の long-idle は `CHROME_LONG_IDLE_MS`=5s」: `tuning.rs:100`、`gji_fsm.rs:1015`。

**指摘3点**

1. **日付**: 注記は「2026-09-21」だが、本日は 2026-09-20（コミット時刻も
   `Sun Sep 20 23:25:09 2026 -0500`）。UTC 起算なら 09-21 で整合するが、
   同リポジトリの他の docs（`.claude/rules/*.md` や known-bugs の追記）はローカル日付で
   書かれているので揃えること。
2. **30行ルール**: `docs/known-bugs/BUG-002.md` は現在 50 行（frontmatter 7 行を除く本文 ~42 行）で、
   `.claude/rules/fix-requires-evidence.md` の「1ファイルあたり本文は目安30行以内」を超えている。
   注記は 5 行を**追加**した形で、ADR-158 RC4（ガバナンスが加算のみを義務化し減算に報酬が無い）
   が指摘する形そのもの。**追記だけでなく削るべき箇所がある**: 「残存リスク」節（削除済み定数
   `probe_min_ms=20ms` が不十分かもしれない、という論）は、その定数が存在しない今、
   読む価値が無い。ここを落とせば注記を足しても行数は減る。
3. **`related_adr`**: `[]` のまま。本 ADR がこのファイルの記述の現役性を判定した以上、
   `["ADR-193"]` を入れておくと、次にこのバグを読む人が経緯に辿り着ける
   （`docs-frontmatter-convention.md` の frontmatter 維持の趣旨）。

### m4. 裏取りの結果、v3 の記述で**正しい**と確認できたもの（固定点）

- `learn_tsf()` の永続化先が `cache.toml`: `focus/classifier.rs:50`
  `const CACHE_FILENAME: &str = "cache.toml";`、`:455-456` が `base_dir.join(CACHE_FILENAME)`。
  **かつ `:408-410` の `save_section` はセクション単位で更新する**ので、
  `e2e-ime.yml:290` が書く `[imm_capability."ime_key_matrix_spike.exe"]` と同じファイルの
  別セクションに同居する。M4 の汚染リスクの記述は正確で、むしろ「CI が既に触っているファイル」
  である分、危険度は ADR が書いた通り。
- `check` 種別が `expect`/`consistency`/`toggle`/`resync`/`vkprobe` の5つ: `e2e-ime.yml` の
  `check='...'` を全列挙して一致（他の値は無い）。
- `tuning.rs` の行番号 `:100`（`CHROME_LONG_IDLE_MS`）・`:148`（`MEDIUM_IDLE_PROBE_MS`）: 一致。
  `LONG_IDLE_MS` は `:85`（ADR は行番号を出していないので問題なし）。
- `e2e-ime.yml` の `matrix.cfg` × `run: [1,2,3]` と `timeout-minutes: 25`: 一致。
- `chrome_probe` が `.github/` から参照されていないこと: `grep -rn "chrome_probe" .github/` は 0 件。
- `literal_detect_fsm.rs` の実在: `crates/awase-windows/src/tsf/warmup/literal_detect_fsm.rs`。
- 決定1・決定3・「現状認識の訂正」の CI/WT/`bあ` 分離: round1・round2 で確認済みの内容から変更なし。

---

## まだ楽観的・未検証な箇所

1. **ステップ0 の「出ない」側が、実質的に本命になる可能性を織り込んでいない**。
   2026-07-18 に機構ごと削除しても数日間の実機ソークで
   「`suspected literal` genuine ゼロ件」を確認している（`docs/experiments.md:339`、`d495649`）。
   つまり**症状が出ない公算はかなり高い**。にもかかわらず ADR の本文量は「出る」側の準備
   （撤去スクリプト、long-idle シナリオ、CI 接続）に偏っている。M2 の通り「出ない」側にこそ
   設計（陽性対照と ablation 必須化）が要るので、そこを先に書いておくべき。
2. **未確認事項の「実idleと、GJI休眠をCI上で再現できるか」は、M3 の通り半分は答えが出ている**
   （`--settle` で両方同時に伸ばせる）。残るのは「分離できるか」と「CI の 25 分枠に収まるか」の2点。
   未確認事項をその2点に絞ると、ステップ0 の後に再調査する項目が減る。
3. **決定4-1 の「実idleの見積もりを出す」が、出すだけで判断基準が無い**。
   「25 分を超えるなら何をするか」（構成を分ける／`run:[1,2,3]` を減らす／別ワークフローにする）を
   1行決めておかないと、見積もりを出した後に同じ議論をやり直すことになる。
4. **「BUG-002 を検知できるようにする」というタイトルの目標が、ステップ0 の結果次第で
   成立しなくなる**ことを status 節が半分しか書いていない。現 status は「撤去対象の機構は未特定」
   だが、より正確には「**検知対象の不具合が現行コードで再現するかどうかが未確認**」であり、
   再現しなければ本 ADR の目標自体が「再発の予防（回帰検知）」へ変質する。
   タイトルと status にそのことを書いておくと、後から読む人が誤解しない。

---

## 判定

**収束していない**が、**Blocker は無い**。残る Major 3 件は

- M1: 参照先ファイル名を `tsf/warmup/probe_fsm.rs:454` に直す（1行）
- M2: 決定4-0 の「か」を、ablation 必須 + 陽性対照必須の積に直す（数行）
- M3: 「まだできていないこと」1 を「keyboard idle と GJI idle の**分離**」に直す（数行）

でいずれも文面の修正のみであり、設計判断の差し戻しは不要。
Minor（m1〜m3）を併せて反映すれば、次ラウンドで収束と判断できる見込み。
