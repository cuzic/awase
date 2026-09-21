# ADR-193 実装計画 敵対的レビュー round3

対象: `docs/adr/193-implementation-tasks.md`（v3、commit `a1841fc0`）、ADR 本体（status のみ更新）

**判定: 収束していない。Blocker 0・Major 2・Minor 8。**
計画 round2 の PB2・PM5・PM6・pm1〜pm7 は**すり替え・取りこぼしなく反映**され、
v3 で新しく書いた主張はほぼすべて実コードと一致した（Cargo の feature、`--activate-gji` の4点、
8ケース固定、`utc_stamp`×`to_ms`、`if:` を付けない、TALLY×summary）。
残る Major は2件で、どちらも**「測定が成立しているかを assert していない」**系:

- **PM7**: `gji_idle_ms()` は監視未アタッチ時に**マシン稼働時間**を返す。この状態では掃引の全点が
  `Long` に潰れ、実験が無言で無意味になる。`idle_at_cold` を「記録する」だけで「検証する」設計になっていない。
- **PM8**: `expect` 自身が ASCII 英字を含みうる（`ensure()` は `kあ` を合格させる）。
  判定規則の第1条 `text == expect → PASS` が、**部分リテラル同士の一致を PASS にする**。

---

## round2 指摘の反映確認

| round2 | v3 の対応 | 判定 |
|---|---|---|
| PB2 `--activate-gji` の本質 | 設計3 を4点表に、フラグ名を `--activate-gji` に統一、①②③を写し④を除外、複製+出典コメント、HRESULT/`GetActiveProfile` ログ、G1 分解に「GJI 非アクティブ TIP」、T1a の規模と `PRECOND_FAIL=0` | **反映済み**。`ime_key_matrix_spike.rs:1785-1794` の4点、`:1594-1604` の `--msime` 分岐、`log_active`（`:1613-1625`）と一致 |
| PM5 engine-off が PASS | `IDLE` 行に `awase=`/`expect=`、順序付き判定規則（PASS→EMPTY→ENGINE_OFF→英字→MISMATCH）、`--no-awase` 腕を `ch-idle-noawase` として T3b に定義 | 反映済みだが**第1条に穴**（PM8） |
| PM6 試行数・率 | 45/27 に統一、≦10%/≧50%/比4倍、「0件要求にしない」、掃引点別 TALLY、135 の位置づけを明記 | **反映済み**。矛盾は解消 |
| pm1 未知引数の黙殺 | 起動ログにフラグを出し、checker が確認、T3a 受け入れ基準に | **反映済み** |
| pm2 `utc_stamp`×`to_ms` | 「確認済み」に更新、日付跨ぎの但し書き | **反映済み**（`check.py:29-31`、`chrome_probe.rs:175-188`） |
| pm3 T0 の実行時間 | 8ケース固定・`--repeat=1`・約15分 | **反映済み**（`chrome_probe.rs:636` の `for (i, c) in CASES.iter().enumerate()`、`:654` の `sleep(settle_ms)`） |
| pm4 EMPTY | `process` で INVALID/FAIL に分岐 | 反映済み（ただし pm12 参照） |
| pm5 検証済み固定点 | 各所の行番号を維持 | **反映済み** |
| pm6 切り出しに `if:` を付けない | 設計5 に明記 | **反映済み** |
| pm7 依存 | T3a←T1a、起動ログ確認を受け入れ基準に | **反映済み** |

---

## Major

### PM7. GJI 監視が未アタッチだと `gji_idle_ms()` は「マシン稼働時間」を返し、掃引が全点 `Long` に潰れる

**根拠**

- `crates/awase-windows/src/tsf/observer.rs:434-442`
  ```rust
  /// GJI プロセスの最終 I/O 変化時刻 (ms) を返す。0 = 未観測。live 読み取り。
  pub(crate) fn gji_last_io_ms() -> u64 { TSF_OBS.gji_last_io_ms.load(Ordering::Relaxed) }

  pub(crate) fn gji_idle_ms() -> u64 {
      crate::hook::current_tick_ms().saturating_sub(gji_last_io_ms())
  }
  ```
  **未観測時 `gji_last_io_ms` は 0** なので、`gji_idle_ms()` は `current_tick_ms()` そのもの
  （= 起動からの tick、実質マシン稼働時間）を返す。
- `gji_last_io_ms` が入るのは `GjiMonitor::try_attach()` が成功したとき
  （`tsf/gji_monitor.rs:401-405`）と `m.sample()` が delta を返したとき（`:444-`）だけ。
  失敗時は `TSF_OBS.gji_monitor_ok.store(false, …)` して
  `next_attach_ms = now + GJI_REATTACH_INTERVAL_MS` で後退（`:420-421`）。
- `ColdKind::classify(gji_idle_ms)` は `≥ LONG_IDLE_MS(10s)` で `Long`（`gji_fsm.rs:127-136`）。
  稼働時間は常に 10s を超えるので **必ず `Long`**。

**CI での現実味**

`e2e-ime.yml:231` はセットアップで
`Get-Process | Where-Object { $_.Name -match 'googleime|mozc' } | Stop-Process -Force` を実行し、
GJI のプロセスを**意図的に落とす**（config1.db を読み直させるため）。その後 `ctfmon` を再起動する。
つまり awase 起動時点で GJI のコンバータプロセスは**いない**可能性が高く、
`try_attach` は失敗し、`gji_monitor_ok=false` のまま始まる。
GJI プロセスは TIP が実際に呼ばれてから起動するので、`--activate-gji` → Chrome フォーカス → 最初の打鍵、
のどこかで立ち上がり、その後の `next_attach_ms` tick で初めてアタッチされる。

**失敗シナリオ**

1. アタッチ前に掃引が始まる（あるいは最後までアタッチされない）。
2. `gji_idle_ms` は稼働時間なので、3000/6000/8000/11000/14000 の**全点が `Long`** になる。
3. 掃引の結果は5点とも同一挙動。計画はこれを「帯による差が無い」＝「症状は出ない/撤去は効かない」と読む。
4. **測定が一度も成立していないのに、G0 が「出ない」で確定する。**
   ADR 決定4-0 の「出ない」分岐に入り、per-VK confirm の有効性を「E2E で示せない」と
   `docs/experiments.md` に誤って記録する。

計画は PM2 の反映として `idle_at_cold` を「checker の出力に**並べる**」としているが（設計2）、
**並べるだけでは上のシナリオを止められない**。`idle_at_cold` が 3000 ではなく 480000 と出ていても、
誰も見ていなければ通る。

**修正案**

1. **掃引開始前の前提条件に `gji_monitor_ok` を入れる。** awase ログの
   `[gji-monitor] attached to GJI process (I/O monitoring enabled)`（`gji_monitor.rs:399`）を
   checker が探し、無ければその run 全体を `INVALID`。
   `chrome_probe` 側では、`--activate-gji` の後に「GJI プロセスが起動するまで1打鍵して待つ」
   ウォームアップを入れる案もある（ケース実行前の `ensure()` が実質それを兼ねるが、明示したい）。
2. **`idle_at_cold` を assert する。** 各試行で
   `|idle_at_cold − 掃引点| ≤ 許容（例: 掃引点の ±30% か ±1500ms の大きい方）`
   を満たさなければ `INVALID`（`IDLE_MISMEASURED`）。許容外が多発する場合は
   「そもそも `gji_idle_ms` が掃引と連動していない」ことの直接の証拠になる。
3. `TALLY` に `mismeasured=N` を加え、`summary` に出す。
4. R2 を「記録する」から「**検証する（不一致は INVALID）**」に書き換える。
   同様に設計2 の「checkerの出力に`idle_at_cold`を並べる」も「突き合わせて判定する」に直す。

補足（good news）: この assert を入れれば、`--settle` / `--pre-settle` / ポーリング（R8）が
`gji_idle_ms` に与える影響も同じ仕組みで検出できる。設計2 が挙げていた
「何かの拍子に GJI I/O が走るとリセットされる」も、これで初めて観測可能になる。

### PM8. 判定規則の第1条が、`expect` 自身の部分リテラルを PASS にする

**根拠**

- 設計1 の判定は「1. `text == expect` → `PASS`」を**最初に**評価する。
- `expect` は `ensure(Kana)` 最後の `probe()` の `text`。`ensure()` の合否は
  `kana_ok`（`chrome_probe.rs:370-377`）＝ awase 起動中は `c == Class::Nicola`。
- `classify()`（`:263-281`）の `Nicola` 条件は
  ```rust
  if t.chars().any(|c| ('\u{3040}'..='\u{30FF}').contains(&c)) { return Class::Nicola; }
  ```
  **かなが1文字でもあれば Nicola** なので、`kあ`（先頭リテラル＋かな）も `Nicola` で合格する。
  これは計画自身が設計1 の2つ目の箇条書きで「既存 `classify()` は `kあ` を `Nicola` 扱いする」と
  書いている性質そのもの。

**失敗シナリオ**

`ensure()` の最後の probe が部分リテラル（`kあ`）を出した回は、`expect = "kあ"` になる。
その後の idle 後の打鍵も同じく部分リテラル `kあ` を出すと、**第1条で `text == expect` が成立して `PASS`**。
第4条（英字を含む → `PARTIAL_LITERAL`）には到達しない。**検出したい症状そのものを PASS にする。**

これは机上の話ではない。T1b で追加する `--pre-settle`（モードキー押下の**前**に長く待つ）は、
まさに「長 idle 直後の最初の打鍵」を `ensure()` の中に作る設計なので、
`ensure()` の probe が BUG-002 型の症状を踏む確率が最も高いモードになる。

**修正案**

1. **`expect` の妥当性検査を先に置く**（第0条）:
   `expect` が空、または ASCII 英字 `[A-Za-z]` を含む → その試行は `INVALID`（`BAD_EXPECT`）。
   `expect` が壊れている以上、その回の比較は意味を持たない。
2. 判定順を「英字チェックを `expect` 一致より先」に変える案でもよいが、
   その場合 `text == expect` の利点（配列非依存、R6 の解消）が薄れるので、第0条のほうが素直。
3. `ensure()` 側で、`Nicola` かつ英字を含まないことを要求する選択肢もある
   （`kana_ok` を厳しくする）。ただしこれは**既存8ケースの判定を変える**ので、
   計画が掲げた「既存 `classify()`・既存ケースの挙動を変えない」に反する。
   → checker 側（第0条）で閉じるのが整合的。
4. `IDLE` 行に `expect` をすでに出す設計なので、fixture に `BAD_EXPECT` を1件追加する（T2）。

---

## Minor

### pm8. `activate_gji_profile()` の規模が「約25行」ではない（T1a 見積もり）

実体は `ime_key_matrix_spike.rs:1590-1642`（doc 2行 + 関数 `:1592-1642`）で **約51行**。
`log_active` クロージャ（`:1613-1625`）と `ActivateProfile` 呼び出し（`:1627-1635`）、
`sleep(1500)` + `log_active("後")`（`:1636-1637`）を含む。計画の `:1590-1613` という範囲も
関数の前半しか指していない。

加えて、複製には次の import が要る（`chrome_probe.rs` には**現在 COM/TSF の参照が1つも無い**
— `grep -n "CoInitialize\|CoCreateInstance\|ITf" chrome_probe.rs` は 0 件）:
`CoInitializeEx` / `COINIT_APARTMENTTHREADED` / `CoCreateInstance` / `CLSCTX_INPROC_SERVER` /
`CLSID_TF_InputProcessorProfiles` / `ITfInputProcessorProfileMgr` / `GUID_TFCAT_TIP_KEYBOARD` /
`TF_INPUTPROCESSORPROFILE` / `HKL`。

→ T1a の「約25行を複製」を「約50行 + COM 初期化と import 一式」に直す。

### pm9. `CoInitializeEx` の必要性（計画は言及済み、根拠を補強）

スパイクは `init_tsf()`（`:1644-1661`）で
`CoInitializeEx(None, COINIT_APARTMENTTHREADED)` してから `activate_gji_profile()` を呼ぶ
（順序は `:1783` → `:1785`）。`CoCreateInstance` は COM 初期化済みスレッドでしか成功しないので、
`chrome_probe` でも必須。**T1a の記述（`:186-187`）に `CoInitializeEx` が含まれているのは正しい。**
なお awase 側も `gji_monitor.rs:337-343` で同じ初期化をしており（`S_FALSE` を無視）、
`chrome_probe` でも戻り値は無視でよい。
1点だけ: `chrome_probe` は HTTP サーバを別スレッドで動かすので、
**COM 初期化と `CoCreateInstance` は同一スレッド（`main`）で行う**ことを計画に明記すること。

### pm10. Cargo の feature 追加は不要（Q2(a) の主張は**正しい**）

`crates/awase-windows/Cargo.toml` の
`[target.'cfg(windows)'.dependencies.windows].features` に
`Win32_UI_TextServices` / `Win32_System_Com` / `Win32_System_Ole` /
`Win32_UI_Input_KeyboardAndMouse`（`HKL` 用）がすべて含まれている。
feature はパッケージ単位で解決され、examples も同じパッケージのターゲットなので、
`chrome_probe.rs` から追加なしで使える。✓ 計画の主張どおり。

### pm11. awase は 2 秒周期で TIP を再検出する（Q2(b) の答え。14 秒は十分）

`tsf/gji_monitor.rs:371-398`:
```rust
if now >= next_clsid_check_ms {
    next_clsid_check_ms = now + 2_000;
    if let Some(kind) = super::tip_detector::query_active_kind(mgr) { … }
}
```
`ImeKindDebounce` により「同じ新種別が2 tick 連続」で確定するので、
**後から `ActivateProfile` しても awase は 2〜4 秒で追随し、`WM_IME_KIND_CHANGED` を発行する**。
awase が先に起動していても機能する（ポーリングは常時回る）。
→ 設計3 の②「14 秒待つ」は十分に余裕がある。**ただし PM7 の通り、
`WM_IME_KIND_CHANGED`（TIP 種別）と `gji_monitor_ok`（I/O 監視）は別物**で、
14 秒待っても後者が揃うとは限らない。設計3 の②の説明に
「TIP 種別の検出は2〜4秒、GJI プロセスへの I/O 監視アタッチは別で、そちらは PM7 の前提条件で見る」と
書き分けること。

### pm12. スパイクとの順序の差は**意図的な改良**だが、そう書かれていない

スパイクの順序は `create_window()`（`:1784`）→ `activate`（`:1786`）→
`AUTO_NEXT = +14000`（手順開始、`:1788`）→ `queue_press(+12000, VK_IME_OFF)`（`:1792`）。
つまり **VK_IME_OFF は手順開始の2秒前**に、スパイク自身の窓が前面の状態で打たれる。

計画 v3 の流れは「activate → awase 検出待ち → **Chrome 起動 → 前面化** → VK_IME_OFF → ケース」。
`chrome_probe` は自分の窓を持たないので、前面化後に打つほうが**正しい**
（IME のオープン状態はフォーカス窓のコンテキストに効くため）。
ただし計画は設計3 で「①②③を**写す**」と書いており、順序を変えたことを明示していない。
将来「スパイクと違う」と気づいた人が揃えて戻すと退行する。
→ 「③の位置はスパイク（自窓が前面）と異なり、Chrome 前面化後に置く。理由は自窓を持たないため」と1行入れる。

### pm13. `expect` を取るには `ensure()` のシグネチャ変更が要る（T1b に記載なし）

`ensure()` は現在 `-> bool`（`chrome_probe.rs:370`）で、最後の probe の `text` を返さない。
設計1 の `expect` を得るには `ensure()` を `-> Option<String>`（または `(bool, String)`）にする必要がある。
T1b の内容は「`probe()` を内部関数と分類付きラッパーに分ける」しか書いておらず、
`ensure()` の変更に触れていない。既存8ケースは `ensure()` の戻りを `bool` として使う
（`:648`）ので、変更時は呼び出し側も直す。→ T1b（または T1a）の変更内容に1行追加。

なお `Setup::Kana` では最後の probe は `setup:IME_ON後` か `setup:ひらがな後` のいずれかで
（`:378-388`）、「最後の probe の text」は一意に定まる。定義自体は整合している。✓

### pm14. `--no-awase` 腕の整合は**取れている**（Q2(d) の答え）

`ensure()` の `kana_ok` は `awase=false` のとき `c == Class::RomajiKana`（`:371-376`）、
`classify()` は `t == "か"` のときだけ `RomajiKana` を返す（`:270-272`）。
したがって `--no-awase` 腕では `expect == "か"` が保証され、
設計1 の「`awase=false` のとき `text == "か"` → `PASS`」と一致する。✓
`ch-idle-noawase` が `awase='false'` を渡すのも、既存 run ステップの
`if ('${{ matrix.cfg.awase }}' -ne 'false')`（`e2e-ime.yml:303`）と整合する。✓
③の VK_IME_OFF 注入も「awase なしでも無害」（`ime_key_matrix_spike.rs:1791`）。✓

1点だけ: `awase=false` の「他は `INVALID`」は、**IME 自身がリテラルを出した場合**も INVALID になる。
これは「ハーネス/環境の問題」という意味では妥当だが、
`ch-idle-noawase` が全 INVALID になったとき「対照が取れていない」のか
「環境が壊れている」のか区別できない。`text` を TALLY に添えるか、
`awase=false` 側だけ `LITERAL` を別バケットに出すこと。

### pm15. EMPTY + `process=true` を FAIL にするのは早い

`probe()` は `k`,`a` の後 350ms しか待たない（`:332-334`）。
`process=true`（IME がキーを受けた）かつ `text` が空は、
「composition がまだ確定していない」＝ タイミングの問題である可能性が高く、
BUG-002 型（リテラルが**出てしまう**）とは逆向きの現象。
FAIL に入れると撤去実験の率にノイズが乗る。
→ `EMPTY` は `process` の値によらず **INVALID（または独立バケット `PENDING`）** に倒し、
発生率が高ければ待ち時間を延ばす、とするほうが安全。

### pm16. リスク表の番号が R1〜R8, R10, R11, R9 の順に並んでいる

表示上の瑕疵。R9 を R8 の次に戻すか、番号を振り直すこと。

---

## 質問への直接の回答

**Q1（反映の正確さ）**: すり替え・取りこぼしは無い。PB2 の4点表は
`ime_key_matrix_spike.rs:1785-1794` の実装と一致し、④を「不要」と切り分けた判断も正しい
（`chrome_probe` はフックを張らない）。PM5・PM6 も意図どおり。

**Q2（新主張の裏取り）**
- **(a) feature 追加不要**: **正しい**（pm10）。ただし複製規模は約50行で、COM 初期化と import 一式が要る（pm8・pm9）。
- **(b) 順序と TIP 検出**: awase は**2秒周期**で `query_active_kind` を回し、2 tick のデバウンスで確定する
  （`gji_monitor.rs:371-398`）。**awase 起動後に activate しても検出は働く**。14 秒は十分（pm11）。
  順序をスパイクから変えたのは妥当だが、変えたと明記すべき（pm12）。
  **ただし TIP 検出と `gji_monitor_ok`（I/O 監視アタッチ）は別で、後者が PM7 の本体。**
- **(c) 判定規則の順序**: 第1条に穴がある（PM8）。`expect` の定義自体は `ensure()` の実装と整合するが、
  `ensure()` は現在 text を返さないので変更が要る（pm13）。EMPTY の扱いは要再考（pm15）。
- **(d) `--no-awase` 腕**: **整合している**（pm14）。

**Q3（依存・受け入れ基準）**: `T2/T1a → T3a →(G1)→ T1b → T3b →(G0)→ [T4] → T5`、`T0`・`T6` 並行、に矛盾なし。
抜けは pm13（`ensure()` のシグネチャ）と、PM7 由来の「掃引開始前に `gji_monitor_ok` を確認する」受け入れ基準。

**Q4（裏取りせず前提化した箇所）**: v3 の新規記述には**見つからなかった**。
`e2e-ime.yml:294-299`・`:303`、`ime_key_matrix_spike.rs:1785-1794`・`:1594-1604`、
`chrome_probe.rs:343-346`・`:636`・`:654`・`:559`、`check.py:29-31`、
summary の `pattern: result-*` と `result.txt` の扱い、`if:` を付けない判断 — すべて実コードと一致。
唯一の不正確は `activate_gji_profile` の行範囲と行数（pm8）で、これは前提化ではなく見積もりの誤差。
**同型の誤り（doc/コメントの鵜呑み）は今回ゼロ**。規約節（`:297-299`）に4回分の経緯を残したのも妥当。

**Q5 相当（数値）**: PM6 の反映で 45/27・≦10%/≧50%・比4倍・0件要求にしない、は整合が取れている。
ただし PM7 の `mismeasured` を TALLY に入れないと、率の分母が「測定が成立した試行」にならない。

---

## 判定

**収束していない**が、**Blocker は無い**。残る必須は2件:

- **PM7**: 掃引開始前に `gji_monitor_ok`（`[gji-monitor] attached …` ログ）を確認し、
  各試行で `idle_at_cold` と掃引点の一致を **assert** する（不一致は `INVALID`／`TALLY` に `mismeasured=`）。
  これが無いと、測定が一度も成立しないまま G0 が「出ない」で確定しうる。
- **PM8**: 判定規則に第0条「`expect` が空 or ASCII 英字を含む → `INVALID`（`BAD_EXPECT`）」を足す。
  第1条の `text == expect → PASS` が部分リテラル同士の一致を PASS にするのを防ぐ。

Minor（pm8 の規模、pm9 のスレッド明記、pm12 の順序差の明記、pm13 の `ensure()` 変更、
pm14 の対照腕バケット、pm15 の EMPTY、pm16 の番号）はいずれも文面修正。
この2件の Major と Minor を反映すれば、**次ラウンドで収束と判断できる**見込み。
