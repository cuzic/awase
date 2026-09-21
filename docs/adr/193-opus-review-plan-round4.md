# ADR-193 実装計画 敵対的レビュー round4

対象: `docs/adr/193-implementation-tasks.md`（v4、commit `7b29ec14`）

**判定: 収束していない。Blocker 1・Major 1・Minor 6。**
round3 の PM7・PM8・pm8〜pm16 は**すべて反映されている**（下表）。
しかし PM7 の実装手段として採用した `idle_at_cold` が、**`gji_idle_ms` でもなければ打鍵時点の値でもない**。
これは round3 で私が根拠を詰めずに提案したもので、**レビュアー側に起因する同型の誤り（通算5回目）**である。
そのまま実装すると `mismeasured` がほぼ 100% になり、計画自身の G0 規則
（「`mismeasured` 20% 以上は判定不能」）によって実験が恒久的に「判定不能」で止まる。

---

## round3 指摘の反映確認

| round3 | v4 の対応 | 判定 |
|---|---|---|
| PM7 測定の成立を assert | 設計2 に「検証する」3規則（attach ログ前提・試行ごと突き合わせ・G0 の `mismeasured` 20%）、`TALLY` に `mismeasured`、R12 | **反映の意図は正しいが手段が誤り**（PB3・PM9） |
| PM8 `BAD_EXPECT` | 設計1 に第0条を追加、根拠（`kana_ok`、`chrome_probe.rs:370-377`）も明記、fixture にも追加 | **反映済み。指摘どおり** |
| pm8 複製規模 | 「約51行」「`:1590-1642`」「import 一式」「feature 追加不要」 | **反映済み**（実体も `:1590-1642`） |
| pm9 COM は main スレッド | 設計3 に明記 | **反映済み** |
| pm10 feature | 設計3 に明記 | **反映済み**（`Cargo.toml` の `Win32_UI_TextServices`/`Win32_System_Com` はパッケージ単位） |
| pm11 TIP 検出 2 秒周期 | 設計3 ②に「TIP 種別の検出と I/O 監視のアタッチは別物」と明記 | **反映済み**（`gji_monitor.rs:371-398`） |
| pm12 順序差は意図的 | 設計3 に「順序はスパイクと異なる（意図的）」と理由付きで明記 | **反映済み** |
| pm13 `ensure()` の戻り値 | T1b に `-> Option<String>` 化と呼び出し側（`:648`）の修正を明記 | **反映済み** |
| pm14 対照腕のバケット | `control_literal` を別バケットに | **反映済み** |
| pm15 EMPTY | `process` によらず `INVALID`（`EMPTY_PENDING`） | **反映済み** |
| pm16 リスク番号 | R12 追加・番号整理 | **反映済み** |

---

## Blocker

### PB3. `idle_at_cold` は `gji_idle_ms` ではなく、打鍵時点の値でもない。設計2 の突き合わせは必ず失敗する

**根拠（実コード）**

1. `[h1-probe]` が出す値の出所は `composition.idle_ms_at_last_cold()`（`output/vk_send.rs:279-281`）。
2. その値を書くのは `ColdCtx::record_cold`（`tsf/probe.rs:283-287`）だけ。
   ```rust
   pub fn record_cold(&self, reason: crate::output::ColdReason, idle_ms: u64) {
       self.last_cold_reason.set(reason);
       self.idle_ms_at_last_cold.set(idle_ms);
   }
   ```
3. 唯一の呼び出し元は `mark_composition_cold`（`tsf/probe.rs:358-380`）:
   ```rust
   pub fn mark_composition_cold(&self, reason: crate::output::ColdReason) {
       let idle_ms = self.ms_since_last_send();      // ★ gji_idle_ms ではない
       ...
       self.cold_ctx.record_cold(reason, idle_ms);
   }
   ```
   **記録されるのは `ms_since_last_send()`**（`tsf/probe.rs:408-409` → `:214-220`、
   awase が最後に**出力を送った**時刻からの経過。未送信なら `u64::MAX`）。
   `tsf::observer::gji_idle_ms()`（GJI プロセスの I/O 観測からの経過）とは**別の量**である。
4. 捕捉されるのは**cold マーク時点**。呼び出し元は
   `platform.rs:322,324,347,491,645,1423,1447,1546,1580`（FocusChange / SetOpenTrue / SetOpenFalse /
   NativeF2Consumed / ReinjectConfirmKey …）と `output/vk_send.rs:690`（`SymbolVkSent`）で、
   **いずれも掃引の `sleep(idle)` より前（`ensure()` の最中）に起きる**。
   sleep 中は何のイベントも起きないので再マークされず、`k`/`a` は記号 VK ではないので `:690` も発火しない。

**帰結**

`[h1-probe] … idle_at_cold=…ms` が sleep 後に出す値は、
**`ensure()` 実行時に記録された、別種の（そして小さい）値**である。
掃引点 14000ms の試行でも `idle_at_cold` は数百 ms のまま出る。

**失敗シナリオ**

設計2-2 の `|idle_at_cold − 掃引点| ≤ max(掃引点の30%, 1500ms)` を実装すると:

1. ほぼ全試行が範囲外 → `IDLE_MISMEASURED`（INVALID）。
2. `TALLY` の `mismeasured` が ~100%。
3. 設計2-3／G0 の「`mismeasured` が 20% 以上の間は確定させない（判定不能）」により、
   **T1b でも T3b でも G0 が永久に確定しない**。計画は T4 にも T5 にも進めない。
4. これを「測定系がおかしい」と正しく読めればよいが、より起きやすいのは
   「許容範囲が厳しすぎる」と判断して 30% → 300% …と緩め続けることで、
   **assert が実質無効化**される。それは PM7 が防ごうとした状態そのもの
   （「`idle_at_cold` を並べるだけでは止められない」）への逆戻りである。

**これは私（レビュアー）の誤りに起因する。**
round3 PM7 は `vk_send.rs:279-280` で `idle_at_cold` が cold 経路の直下に出ていることだけを見て
「これが `gji_idle_ms` のスナップショット」と書いた。`record_cold` の呼び出し元を読んでいなかった。
本 ADR が4回繰り返した「名前・配置から実装を推測する」誤りの5回目で、今回は**指摘側が発生源**。
計画の規約節（`:297-299`）に「レビュー指摘の根拠も実装で裏取りしてから採用する」を足すこと。

**修正案: `[vk-send]` 行を一次アンカーにする**

`output/vk_send.rs:239-242` は、cold/warm どちらの経路でも **romaji 送信のたびに無条件で**出る:

```
[vk-send] romaji={romaji:?} warm={warm} elapsed={}ms session_expired={session_expired} prepend_f2_warmup={prepend_f2_warmup}
```

- `elapsed` = `ms_since_last_send()` を**打鍵時点で**評価した値（`assess_warmth()`、`output/mod.rs:1426-1437`）。
  これは **掃引が実際に制御している量**（awase の最後の出力からの経過）であり、
  期待値は `掃引点 + probe 末尾の 350ms + 150ms`（`chrome_probe.rs:332-334,347-348`）≒ `掃引点 + 約500ms`。
- `warm` = `warmup_coord.is_warm()`（`output/mod.rs:996-998`）= GjiFsm が OnWarm か
  （`CHROME_LONG_IDLE_MS`=5s の `GjiTimer::LongIdle` が未発火か）。
- `prepend_f2_warmup` = どちらの経路を通ったか。

したがって試行ごとに assert できるのは:

| 掃引点 | 期待 `elapsed` | 期待 `warm` | 期待 `prepend_f2_warmup` |
|---|---|---|---|
| 3000 | ≈3500 | `true`（5s 未満） | `true`（`session_expired` 経由、PM9） |
| 6000 | ≈6500 | `false` | `true` |
| 8000 | ≈8500 | `false` | `true` |
| 11000 | ≈11500 | `false` | `true` |
| 14000 | ≈14500 | `false` | `true` |

`elapsed` の許容は `+500ms` のバイアスを見込んで**非対称**にするか、
`|elapsed − (掃引点+500)| ≤ max(掃引点の20%, 1000ms)` とする。
`elapsed` が `18446744073709551615`（`u64::MAX`、未送信）で出る場合があるので、
checker はそれを別扱い（INVALID）にすること。

**`gji_idle_ms` 自体を見たい場合**（GJI 休眠の直接確認、R2）は別経路が要る:
- `tsf/gji_fsm.rs:555-560` の `tracing::debug!("[gji-fsm] CompositionReset: gji_idle={gji_idle_ms}ms (Short) → …")`
  （ただし特定分岐のみ）。
- `platform.rs:742,824,869,882` が `note_gji_transition(format!("…(gji_idle_ms={gji_idle_ms})"))` で
  journal の `GjiFsmTransition`（`journal.rs:317-321`、Timing レーン）に載せる trigger 文字列。
  journal を採る運用にするなら、`state_after` に ColdKind の帯も入るので
  **「ms の一致」ではなく「帯（Short/Medium/Long）の一致」を assert する**ほうが意味論的に正しい。
- どちらも取れないなら、R2 は「`gji_monitor_ok` の確認（設計2-1）＋ `elapsed`/`warm` の一致」までとし、
  「`gji_idle_ms` そのものは打鍵時点では観測できない」と明記すること。

---

## Major

### PM9. 「`[h1-probe]` は 7s 以上でだけ出る／5s 未満は cold 経路に入らない」は誤り。3000ms でも出る

**根拠**

`prepend_f2_warmup` は `assess_warmth()`（`output/mod.rs:1426-1437`）で決まる:

```rust
let warm = self.is_composition_warm();          // = warmup_coord.is_warm()（GjiFsm の OnWarm/OnCold）
let elapsed = self.ms_since_last_send();
let session_expired = warm && elapsed < u64::MAX && elapsed > tuning::COMPOSITION_TIMEOUT_MS; // 2000
WarmthContext { warm, elapsed, session_expired,
    prepend_f2_warmup: (!warm || session_expired) && self.warmup_coord.needs_f2_probe() }
```

- `COMPOSITION_TIMEOUT_MS = 2000`（`tuning.rs:106`）。
- 掃引 3000ms では `GjiTimer::LongIdle`(5s) が未発火なので `warm=true` だが、
  `elapsed ≈ 3500 > 2000` なので **`session_expired = true`** → `prepend_f2_warmup = true` → `[h1-probe]` が**出る**。
- 掃引 6000ms 以上は `warm=false` でやはり出る。
- 結論: **全掃引点で `[h1-probe]` は出る**。計画の
  「行が無い場合は掃引点が 7s 以上のときだけ `IDLE_MISMEASURED` とし、それ未満は許容する
  （5s 未満は OnWarm で cold 経路に入らない）」は、前提も閾値も誤り。

**さらに: 同名の別述語を取り違えている**

計画は上の規則と T4 候補1の有効域を `forces_prepend_f2`（`ColdKind` の `gji_idle_ms` 帯）で説明しているが、
`vk_send.rs:256` が分岐に使うのは `WarmthContext::prepend_f2_warmup`（`is_composition_warm` + `needs_f2_probe`）で、
**`ColdKind::forces_prepend_f2()`（`gji_fsm.rs:111-114`、`probe_params()` 経由）とは別物**である。
名前が似ているだけで、入力（composition warmth vs `gji_idle_ms`）も参照経路も違う。

**失敗シナリオ**

- 「3000/6000 で `[h1-probe]` が出るのはおかしい」と読み、存在しない不具合を追う。
- T4 候補1（`prepend_f2_warmup` の撤去）の有効域を「8000/11000/14000 のみ」と書いているが、
  実際は **3000 を含む全点で発火する**。撤去実験の判定帯（設計4 の n=27）を
  8/11/14s に絞ると、**差が出る点を3つ捨てる**ことになり、検出力を自ら下げる。
  逆に「3000/6000 で差が出ないのは正常」という注記を信じて、差が出た結果を異常と誤読する恐れもある。

**修正案**

1. 設計2-2 の `[h1-probe]` 有無の規則を「**全掃引点で `prepend_f2_warmup=true` / `[h1-probe]` 行が出ることを要求**、
   出なければ `IDLE_MISMEASURED`」に改める（PB3 の `[vk-send]` アンカーと合わせると、
   `prepend_f2_warmup` の値を直接読めるので判定が単純になる）。
2. 設計2 末尾と T4 候補1 の「有効域は `gji_idle_ms`≥7s（`forces_prepend_f2`）＝掃引点 8000/11000/14000」を削除し、
   「`vk_send.rs` の `prepend_f2_warmup` は composition warmth 由来で、`ColdKind::forces_prepend_f2` とは別述語。
   有効域は全掃引点（3000 は `session_expired` 経由）」に直す。判定帯は **全5点（n=45）** に戻す。
3. 設計2 の4帯の表は `ColdKind` の帯として正しいので残す。ただし
   「この帯が効くのは GjiFsm の probe 経路であり、`vk_send` の cold 分岐の可否とは別」と1行添える。
   どの現行機構がどの帯で効くかは T1b/T3b の実測で決める（現時点では未特定、という計画の立場と整合する）。

---

## Minor

### pm17. 判定規則に `IDLE_MISMEASURED` と `focus_lost` の評価順が書かれていない

設計1 の順序付き規則は 第0〜5条までで、`IDLE_MISMEASURED` は「さらに…assert する」として列の**外**、
`focus_lost`／物理キー混入／完了マーカー欠落も列の外に置かれている。
`text == expect` の第1条が先に評価されると、**測定が成立していない試行やフォーカスを失った試行が `PASS` になる**。
→ 第0条の前に「前提チェック（`focus_lost` / 物理キー混入 / `IDLE_MISMEASURED` / 起動フラグ欠落）→ `INVALID`」を
明示的な第−1条として置くこと。

### pm18. `mismeasured` が `invalid` の内数か別枠かが未定義（率の分母が曖昧）

`TALLY pass=N fail=N invalid=N mismeasured=N total=N` は、`mismeasured` が
`invalid` に含まれるのか独立かで `total` の意味が変わり、設計4 の率（分母）も変わる。
→ 「`mismeasured` は `invalid` の内数（内訳カウンタ）。率の分母は `pass+fail`」と明記すること。
`summary` 側の合算も同じ規約で書く。

### pm19. 第3条の `expect != "か"` ガードは（第0条がある限り）常に真。ただし配列の落とし穴が1つ

`classify()` は `t == "か"` のとき `RomajiKana` を返し（`chrome_probe.rs:270-272`）、
`ensure()` の `kana_ok` は `awase=true` のとき `Nicola` を要求する（`:370-377`）。
よって `awase=true` の試行では `expect == "か"` は**原理的に発生しない**（発生するなら `ensure()` が false）。
ガードは冗長だが無害。

ただし裏返しの落とし穴がある: **`k`,`a` の NICOLA 出力がちょうど `"か"` になる配列・設定では、
`ensure()` が永久に false を返し、`PRECOND_FAIL` が全件になる**（`classify` が `Nicola` ではなく
`RomajiKana` を返すため）。R6（配列依存）の別の顔なので、1行注記しておくこと。

### pm20. T0 の受け入れ基準が PB3 の影響を受ける

T0 の受け入れ基準「掃引点ごとに `text` と `idle_at_cold` が記録される」と
ログ回収の理由「`idle_at_cold` 突き合わせ用の `awase.log`」は、PB3 により意味を失う。
→ `[vk-send]` の `elapsed` / `warm` / `prepend_f2_warmup` に差し替える
（`awase.log` を回収すること自体は引き続き必要）。

### pm21. `--no-awase` 腕では `elapsed` が `u64::MAX` になりうる

`ms_since_last_send()` は `last_send_ms == 0`（awase が一度も送っていない）のとき
`u64::MAX` を返す（`tsf/probe.rs:214-218`）。`ch-idle-noawase` は awase を起動しないので
そもそも `[vk-send]` が出ないが、awase 起動直後の最初の試行でも `u64::MAX` がありうる。
→ checker は `elapsed` のパースで `u64::MAX` を特別扱いし、`INVALID`（または初回として除外）にすること。

### pm22. v4 の新記述の行番号・ログ文字列は、PB3/PM9 の2点を除きすべて実コードと一致

確認できた固定点:
- `tsf/observer.rs:434-442` の `gji_idle_ms()`（`gji_last_io_ms == 0` で `current_tick_ms()` を返す）✓
- `gji_monitor.rs:401` `tracing::info!("[gji-monitor] attached to GJI process (I/O monitoring enabled)")` ✓（行番号も一致）
- `gji_monitor.rs:371-398` の 2 秒周期ポーリング + `ImeKindDebounce` ✓
- `e2e-ime.yml:231` の `Stop-Process`（`googleime|mozc`）✓
- `ime_key_matrix_spike.rs:1590-1642` の `activate_gji_profile`、`:1594-1604` の `--msime` 分岐、`:1785-1794` の4点 ✓
- `chrome_probe.rs:370` の `ensure() -> bool`、`:648` の呼び出し側、`:343-346` の `Process(229)`、
  `:332-334` の 350ms、`:347-348` の `clear`+150ms、`:57-64` のポーリング、`:636`/`:654` の8ケース固定 ✓
- `check.py:29-31` の `to_ms` ✓
- `imm_learning.rs:45-48` ✓
- 誤りは `vk_send.rs:279` の `idle_at_cold` の**意味**（PB3）と、`[h1-probe]` の発火条件（PM9）の2点のみ。

---

## 質問への直接の回答

**Q1（反映の正確さ）**: すり替え・取りこぼしは無い。PM8（第0条）は指摘の意図どおりで、
根拠（`kana_ok` が `kあ` を通す）まで書かれている。PM7 も「記録する→検証する」への転換という
意図は正しく反映されている。**問題は手段（`idle_at_cold`）が使えないこと**で、これは指摘側の誤り。

**Q2(a)（`[h1-probe]` の規則）**: **整合しない**（PM9）。`[h1-probe]` は `prepend_f2_warmup` 分岐の中
（`vk_send.rs:256-281`）にあり、その述語は `(!warm || session_expired) && needs_f2_probe()` で、
`ColdKind::forces_prepend_f2` とは別。`COMPOSITION_TIMEOUT_MS=2000` のため **3000ms でも `session_expired` で出る**。
warm パスでは `[h1-probe]` は出ないが、`[vk-send]`（`:240`）は**両経路で無条件に**出る。

**Q2(b)（許容範囲 max(30%,1500ms)）**: 比較する量が誤っているので範囲以前の問題（PB3）。
`[vk-send]` の `elapsed` に差し替えれば、`probe()` 末尾の 350ms + 150ms 由来の
**系統的な +約500ms のバイアス**を見込む必要がある。3000ms の点では 500ms が 17% を占めるので、
対称な 30% では辛うじて収まるが、非対称許容か `掃引点+500ms` を期待値にするほうが明確。

**Q2(c)（判定順序）**: 第0〜5条の内部に矛盾は無い。第3条の `expect != "か"` は冗長だが無害（pm19）。
抜けは `IDLE_MISMEASURED` と `focus_lost` の評価位置（pm17）。

**Q2(d)（`mismeasured` と分母）**: 未定義（pm18）。内数か別枠かを決め、率の分母は `pass+fail` と明記すること。

**Q3（依存・受け入れ基準）**: 矛盾なし。pm13（`ensure() -> Option<String>`）は T1b に反映済みで、
呼び出し側（`:648`）の修正も明記されている。T1a は `--activate-gji` のみで `ensure()` を触らないので、
T1a → T3a（既存8ケース smoke）→ T1b（`ensure()` 変更）の順なら、T3a の時点では旧シグネチャのまま動く。
**ただし T1b で `ensure()` を変えた後、T3a の `ch-smoke`（既存8ケース経路）が壊れていないことを
再確認する受け入れ基準が無い**。T1b の受け入れ基準に「`ch-smoke` 相当（8ケース）が引き続き完走する」を足すこと。

**Q4（裏取りせず前提化）**: v4 が新しく書いた記述のうち、**`idle_at_cold` の意味（PB3）と
`[h1-probe]` の発火条件（PM9）の2点が該当**する。ただし前者は round3 の指摘文をそのまま採用したもので、
**発生源はレビュアー側**。他の新記述（pm22 の一覧）はすべて実コードと一致しており、
計画側の裏取り水準は前ラウンドと同様に高い。

---

## 判定

**収束していない。** 次までに必須:

- **PB3**: 設計2-2 の突き合わせ対象を `idle_at_cold` から
  `[vk-send]`（`vk_send.rs:240`）の `elapsed` / `warm` / `prepend_f2_warmup` に差し替える。
  期待値は `掃引点 + 約500ms`。`gji_idle_ms` そのものを見たい場合は
  journal の `GjiFsmTransition`（trigger に `gji_idle_ms=`、`state_after` に ColdKind 帯）を使い、
  ms ではなく**帯の一致**を assert する。取れないなら R2 の範囲を明記して縮小する。
  T0 の受け入れ基準（pm20）も同時に直す。
- **PM9**: `[h1-probe]` は全掃引点で出る。「7s 以上でだけ要求」「5s 未満は許容」を
  「全点で `prepend_f2_warmup=true` を要求」に改める。
  T4 候補1 の有効域「8000/11000/14000 のみ」を削除し、判定帯を全5点（n=45）に戻す。
  `WarmthContext::prepend_f2_warmup` と `ColdKind::forces_prepend_f2` が別述語であることを明記する。

Minor（pm17 の評価順、pm18 の分母、pm19 の注記、pm20、pm21 の `u64::MAX`、
Q3 の T1b 受け入れ基準）はいずれも文面修正。
この2件と Minor を反映すれば、**次ラウンドで収束と判断できる**見込み。
また規約節に「**レビュー指摘の根拠も、採用前に実装で裏取りする**」を1行足すこと（今回の PB3 の教訓）。
