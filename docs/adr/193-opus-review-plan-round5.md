# ADR-193 実装計画 敵対的レビュー round5

対象: `docs/adr/193-implementation-tasks.md`（v5、commit `ce94a42f`）

（前提の訂正: 直前の依頼時点では v5 の編集がファイルに届いておらず、本ファイルは一度 v4 に対して書かれた。
`ce94a42f` で 193-implementation-tasks.md が 85 行変更され、v5 が実体として存在することを確認したので、
本文はすべて v5 に対する内容に差し替えた。適用漏れの件は解決済みとして指摘には数えない。）

**判定: 収束していない。Blocker 0・Major 2・Minor 4。**
計画 round4 の PB3・PM9・pm17〜pm21 は**すべて反映されている**（下表）。
残る Major 2 件は、round4 の修正そのものが持ち込んだ**境界値と例外**で、どちらも1〜2行で閉じられる:

- **PM10**: `elapsed` 上限 `掃引点+1500ms` が `command()` の最大3秒待ちを吸収できず、偽 `mismeasured` を生む。
- **PM11**: `--no-awase` 腕には `awase.log` も `[vk-send]` も無いので、測定 assert をそのまま適用すると
  **陽性対照が構造的に全 INVALID** になり、T3b の受け入れ基準が達成不能になる。

---

## round4 指摘の反映確認

| round4 | v5 の対応 | 判定 |
|---|---|---|
| PB3 `idle_at_cold` は使えない | 設計2-2 を `[vk-send]`（`output/vk_send.rs:236-243`）の `elapsed`/`prepend_f2_warmup` 突き合わせに差し替え。`idle_at_cold` は「打鍵時点では `gji_idle_ms` を観測できない」理由の説明としてのみ残置（`:109-112`）。T0 のログ回収理由・受け入れ基準、T2 の `idle` モード、T1b 受け入れ基準、R2/R12 も連動して更新 | **反映済み。漏れなし** |
| PM9 `[h1-probe]` は全点で出る／述語の取り違え | 設計2 に「2つの `prepend_f2` 述語は別物」節（`:128-131`）、T4 候補1 の有効域を全点に、判定帯を既定 n=45 に（設計4 `:167-168`、T4 `:313`） | **反映済み** |
| pm17 評価順 | 第−1条（前提チェック、`focus_lost`/物理キー混入/起動フラグ欠落/`IDLE_MISMEASURED`）を「最初に評価」と明記（`:79-81`） | **反映済み** |
| pm18 `mismeasured` の位置づけ | 「`invalid` の**内数**、率の分母は `pass+fail`」を設計1（`:81`）と設計4（`:171`）の両方に | **反映済み** |
| pm19 `expect != "か"` の冗長と配列の落とし穴 | 第2・3条の間に注記（`:72-73`）、R6 も更新 | **反映済み** |
| pm20 T0 受け入れ基準 | `[vk-send]` の `elapsed`/`warm`/`prepend_f2_warmup` に差し替え | **反映済み** |
| pm21 `elapsed=u64::MAX` | T2 の `idle` モードに明記 | 反映済みだが**この記述が PM11 を露呈させている**（後述） |
| Q3 `ensure()` 変更後の `ch-smoke` 再確認 | T1b 受け入れ基準に追加 | **反映済み** |
| 規約: 指摘の根拠も裏取り | 規約節に追記（`:356-358`） | **反映済み** |

round4 で私が「`warm` は断定せず記録に留めよ」と書いた点も、v5 は
「`warm`は観測値として記録するが、assertしない」（`:117`）として先回りで取り込んでいる。

---

## Major

### PM10. `elapsed` の上限 `掃引点+1500ms` は `command()` の最大3秒待ちを吸収できない

**根拠**

1試行で `last_send_ms` が最後に更新されるのは、`ensure()` 末尾の `probe()` が打った `k`,`a` に対する
awase の出力時。`Output::send_keys` は**冒頭と末尾の両方**で `mark_send()` を呼ぶ
（`output/mod.rs:1302` 手前の doc、`:1322`、`:1420`）→ `composition.update_last_send_ms()`
（`tsf/probe.rs:222-227`）。

その後 `probe()` は（`chrome_probe.rs:331-348`）

```
press(k) → sleep(30) → press(a) → sleep(350) → command("snap") → command("clear") → sleep(150)
```

と進み、`command()` は 20ms 周期で**最大3秒**待つ（`:318-327`
`while start.elapsed() < Duration::from_secs(3) { sleep(20); … }`）。したがって打鍵時点の

```
elapsed ≈ 350 + snap所要 + clear所要 + 150 + 掃引点
```

- 正常時（snap/clear が 1〜3 ポーリング = 20〜60ms 程度）: `掃引点 + 520〜600ms` → 上限内。
- ランナー混雑で snap が 1〜3 秒かかった場合: `掃引点 + 1500〜3500ms` → **上限超過**。

**失敗シナリオ**

ページの読み取りが遅れただけの試行が `IDLE_MISMEASURED` になる。掃引（キーを打たない時間）は
正しく確保されているので、これは偽の `mismeasured` である。積もると G0 の
「`mismeasured` 20% 以上は判定不能」（`:119`）に引っかかり、**測定が成立しているのに確定できない**。
round4 PB3 で閉じた「恒久的に判定不能」の穴を、上限値の取り方で再び開けることになる。

**修正案**

- 上限を `掃引点 + 4000ms` にする（`command()` の 3s + 350 + 150 + 余裕）。
- **下限 `elapsed ≥ 掃引点` は据え置き**。下限こそが本質で、破れるのは
  「awase が idle 中に何かを出力した」＝掃引が汚染されたケース（下記 pm22）である。
- 上限は T3b の実測（`elapsed − 掃引点` の分布）で較正する旨を1行添える。

### PM11. `--no-awase` 腕には `awase.log` も `[vk-send]` も無い。測定 assert を除外しないと陽性対照が全 INVALID になる

**根拠**

- `ch-idle-noawase` は `awase='false'`。`e2e-ime.yml:303` の
  `if ('${{ matrix.cfg.awase }}' -ne 'false')` で awase 自体を起動しない。
- よって `dist\awase.log` が生成されず、`[vk-send]`・`[gji-monitor] attached`・`[h1-probe]` は**1行も出ない**。
  （run ステップは `if (Test-Path dist\awase.log) {…} else { New-Item … awase-filtered.log }` で
  空ファイルを作るだけ。）
- 一方 v5 の設計2 は「行が無い・`elapsed` が `u64::MAX` → `INVALID`（`IDLE_MISMEASURED`）」（`:116-117`）、
  設計2-1 は「attach ログが無ければ **run 全体を `INVALID`**」（`:113-114`）、
  第−1条（`:79-81`）は `IDLE_MISMEASURED` を最初に評価する。
- v5 の T2（`:229`）は「`elapsed` が `u64::MAX`（…未送信）の場合は `INVALID`
  （**`--no-awase`腕**や、awase起動直後の初回試行でありうる）」と書いており、
  **対照腕が INVALID になることを仕様として明文化してしまっている**。

**失敗シナリオ**

`ch-idle-noawase` の全 45 試行が `IDLE_MISMEASURED`（または run 全体 INVALID）になる。
T3b の受け入れ基準「`ch-idle-noawase` が全試行 PASS（ハーネス自体が動いている証拠）」（`:299`）は
**構造的に達成不能**。陽性対照が死ぬと、ADR 決定4-0 が「出ない」側に課した3条件
（ablation 必須・**陽性対照**・INVALID 維持）の1つが満たせず、G0 が「出ない」でも CI に載せられない。
round3 M2 で「不在の assert は壊れたハーネスでも合格する」ことへの対策として入れた対照腕が、
round4 の測定 assert によって無力化される、という取り違えである。

**修正案**

設計1・設計2 に次を明記する（1〜2行）:

> **`awase=false`（`--no-awase`）の腕では、awase ログ由来の前提条件・測定 assert
> （`[gji-monitor] attached`、`[vk-send]` の有無、`elapsed`、`prepend_f2_warmup`）を一切適用しない。**
> 適用するのは `focus_lost` / `前面化に失敗` / `=== 全ケース完了 ===` / 起動フラグ確認と、
> `text == "か"` → `PASS` ／ `control_literal` ／その他 `INVALID` のみ。

併せて T2 の `u64::MAX` の説明から「`--no-awase`腕や」を削り、
「awase 起動直後の初回試行（`awase=true` のとき）」だけを対象にする。

---

## Minor

### pm22. 下限違反は「awase が idle 中に出力した」ことを意味する。`send_keys:` 行で直接見られる

`mark_send()` は `Output::send_keys` の冒頭・末尾で呼ばれる（`output/mod.rs:1322`・`:1420`、
`conv_mutation.rs:7` が「`send_keys` が冒頭・末尾で呼ぶ `mark_send()`」と明記）。
つまり **romaji に限らず awase のあらゆるキー出力**が `last_send_ms` を更新する。
idle 中に drift correction・IME 再アサート・conv actuation 等で VK が出れば、
`elapsed` は掃引点を大きく下回り、下限違反になる。

これは検出したい事象（掃引の汚染）なので assert 自体は正しいが、**原因の切り分け**には
`send_keys: mode=… actions=… prev_elapsed=…ms`（`output/mod.rs:1309-1313`、
`mark_send()` より前に `prev_elapsed` を読む設計で、コメントがその理由を明記している）を
idle 窓の中で数えるのが直接的。

→ checker の `IDLE_MISMEASURED` に理由ラベル（`elapsed_below` / `elapsed_above` / `no_vk_send` /
`u64_max` / `prepend_false`）を付け、`elapsed_below` の回は窓内の `send_keys:` 行数も併記すること。
T0 の実測で「下限違反がどのくらい起きるか」を見てから CI に載せる、と1行足すと安全。

### pm23. 設計1 の `utc` の説明が stale。取得タイミングの定義も要る

設計1 の `IDLE` 行の説明（`:56`）は
「`utc`は`gji_idle_ms`の突き合わせ（設計2）に使う」のままで、設計2 が `[vk-send]` に変わった今は古い。
また設計2 は「`IDLE`行のUTC（`k`押下時刻）以降で最初の行」（`:115`）と括弧で定義しているが、
`IDLE` 行自体は `snap` で `text` を取得した**後**に書かれるため、
**`utc` をログ出力時刻で取ると `[vk-send]` 2 行はどちらも窓の外になる**。

→ 設計1 側に「`utc` は**最初の `press()` の直前**に `utc_stamp()`（`chrome_probe.rs:175-188`）で取る。
突き合わせ窓は `[utc, utc+2000ms]`、窓内の最初の1行を採る（`k` 用と `a` 用で `[vk-send]` は 2 行出るため）」と書くこと。

### pm24. `prepend_f2_warmup=true` の assert は「定数チェック」ではない（round5 の私の下書きを訂正）

`prepend_f2_warmup = (!warm || session_expired) && needs_f2_probe()` の第2項
`needs_f2_probe()` は、GJI 戦略なら `true`、MS-IME 戦略なら `false`
（`tsf/warmup/warmup_strategy.rs:63-65`、切替は `tsf_warmup_coord.rs:100-106 set_active_ime_kind`）。
したがってこの assert は「**GJI 戦略が有効なままか**」を試行ごとに確認する役目を兼ねる
（TIP 検出が MS-IME に振れると false になる）。
単なる定数確認ではないので、**残す価値がある**。

### pm25. 複雑さについて（Q3 への回答）

判定ラベルは現在 10 種（`PASS` / `BAD_EXPECT` / `EMPTY_PENDING` / `ENGINE_OFF` / `PARTIAL_LITERAL` /
`LITERAL` / `PRECONDITION_DRIFT` / `MISMATCH` / `IDLE_MISMEASURED` / `control_literal`）あるが、
**決定に必要なのは 3 値**（`PASS` / `FAIL` / `INVALID`）で、残りは診断ラベルである。

→ checker の**判定ロジックを 3 値**にし、ラベルは `reason=` として行と `TALLY` に添えるだけにする。
仕様は 7 条（第−1〜5条）から 4 条に縮む:

1. 前提が崩れている（`focus_lost` / 物理キー混入 / 完了マーカー欠落 / 起動フラグ欠落 /
   `BAD_EXPECT` / `IDLE_MISMEASURED` / `ENGINE_OFF` / `EMPTY_PENDING` / `PRECONDITION_DRIFT`）
   → `INVALID`（`reason` を付す）
2. ASCII 英字を含む かつ `process=true` → `FAIL`（`reason=PARTIAL_LITERAL|LITERAL`）
3. `text == expect` → `PASS`
4. それ以外 → `FAIL`（`reason=MISMATCH`）

**入力から3値への写像は変わらないので検出力は不変**、fixture も「3値 × 代表 reason」で足りる。

assert ごとの要否は次のとおり。過去ラウンドで実在の沈黙故障を塞いだものは削らないこと。

| assert | 評価 |
|---|---|
| `[gji-monitor] attached` の run 前提 | **残す**（全点 `Long` 潰れの沈黙故障、round3 PM7） |
| `elapsed ≥ 掃引点`（下限） | **残す**（掃引の汚染を直接捉える、pm22） |
| `expect` 比較 + `BAD_EXPECT` | **残す**（engine-off と部分リテラルを閉じる、PM5/PM8） |
| `prepend_f2_warmup=true` | **残す**（GJI 戦略の有効性を兼ねる、pm24） |
| `elapsed ≤ 上限` | **緩める**（PM10。主に snap 遅延を拾うだけ） |
| `warm` | **記録のみ**（v5 で既にそうなっている） |
| `mismeasured` 20% で G0 保留 | **残す**（安価で、測定不成立のまま結論を出す事故を止める） |
| 掃引点別 `TALLY` | **残す**（T4 の帯別比較に要る） |
| `control_literal` | **残す**（対照腕の死因の切り分け、ただし PM11 を直してから意味を持つ） |

正味では「判定を3値+`reason` に畳む」「上限を緩める」の2つで、**仕様の行数は減り検出力は変わらない**。
計画はこの5ラウンドで assert を積み増してきたが、積んだもののうち削ってよいのはこの2点だけで、
残りはいずれも実際に見つかった沈黙故障への対策である。過剰複雑化とは評価しない。

---

## 質問への直接の回答

**Q1（反映）**: すり替え・取りこぼしは無い。round4 の全指摘が反映され、
`idle_at_cold` は「観測できない理由の説明」としてのみ残置されている（`:109-112`）。
`warm` を assert しない判断は round4 で私が挙げる前に v5 側で入っていた。

**Q2(a)（`elapsed ∈ [掃引点, 掃引点+1500ms]`）**: **下限は妥当、上限が狭い**（PM10）。
`last_send_ms` は `send_keys` の冒頭・末尾の `mark_send()`（`output/mod.rs:1322`・`:1420`、
`tsf/probe.rs:222-227`）で更新され、その後 `probe()` の 350ms + snap + clear + 150ms、
さらに掃引の sleep が乗るので正常値は `掃引点 + 520〜600ms`。
下限を掃引点にしてよいのは、`last_send` が `ensure()` 最後の打鍵より前になる経路が無いため
（`ensure(Kana)` は必ず `probe_logged` を通り、その中で awase が出力する）。
上限は `command()` の 3 秒タイムアウトを見込んで `+4000ms` を推奨。

**Q2(b)（突き合わせ方法）**: 「窓内の最初の行」という規則は正しく、`k` と `a` で 2 行出ても
1 行目が採れる。ただし **`utc` を「最初の `press()` の直前」に取ると設計1 側に書く**必要がある（pm23）。
現状の設計1 の `utc` の説明は `gji_idle_ms` 前提のまま残っている。

**Q2(c)（3000 は warm=true、6000 以上は warm=false）**: **正しい**。
`is_warm()` は `GjiState::OnWarm | OnComposing`（`tsf/warmup/warmup_strategy.rs:76-82`）で、
`OnWarm` には `CHROME_LONG_IDLE_MS`(5s) の `GjiTimer::LongIdle` が armed される
（`gji_fsm.rs:470-479`）。3000ms で `prepend_f2_warmup=true` になるのは
`session_expired = warm && elapsed > COMPOSITION_TIMEOUT_MS(2000)`（`output/mod.rs:1426-1437`、
`tuning.rs:106`）経由。v5 が `warm` を assert せず記録に留めたので、
`OnComposing` が warm に含まれる例外も実害にならない。

**Q2(d)（`--no-awase` 腕）**: **整合していない**（PM11）。awase を起動しないので `awase.log` 自体が無く、
測定 assert をそのまま適用すると対照腕が全 INVALID になる。T2 の `u64::MAX` の記述が
その状態を仕様として書いてしまっている。`awase=false` では awase ログ由来の assert を
適用しない、と明記すること。

**Q3（複雑化）**: 判定ラベル 10 種を 3 値 + `reason` に畳めば仕様は 7 条 → 4 条に減り、
検出力は変わらない（pm25）。削ってよい assert は「`elapsed` 上限」だけで、
他は実在した沈黙故障への対策なので残すべき。全体としては過剰複雑化とは評価しない。

**Q4（裏取りせず前提化）**: v5 の新記述はすべて実コードと一致した
（`output/vk_send.rs:236-243`、`output/mod.rs:1426-1437`・`:1302`/`:1322`/`:1420`・`:1309-1313`、
`tsf/probe.rs:222-227`・`:358-380`、`tuning.rs:106`、`warmup_strategy.rs:63-65`・`:76-82`、
`gji_fsm.rs:111-114`・`:470-479`、`tsf_warmup_coord.rs:79-81`・`:100-106`、
`gji_monitor.rs:401`、`chrome_probe.rs:318-327`・`:331-348`・`:175-188`、`conv_mutation.rs:7`）。
**今回は新たな「裏取りせず前提化」は無い。**

---

## 判定

**収束していない**が、**Blocker は無く、残るのは2件の一行修正**:

- **PM10**: `elapsed` の上限を `掃引点+4000ms` に緩める（下限は据え置き、較正は T3b）。
- **PM11**: `awase=false` 腕では awase ログ由来の前提条件・測定 assert を適用しない、と明記する
  （T2 の `u64::MAX` の説明から「`--no-awase`腕や」を削る）。

Minor（pm22 の理由ラベルと `send_keys:` 併記、pm23 の `utc` 定義と stale 説明、
pm25 の 3 値化）はいずれも文面修正。
この2件の Major を直せば、**次ラウンドで収束と判断できる**。
