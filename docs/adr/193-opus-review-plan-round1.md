# ADR-193 実装計画 敵対的レビュー round1

対象: `docs/adr/193-implementation-tasks.md`（commit `837cb4f2`）
参照: ADR 本体 `193-extend-existing-e2e-harness-for-chromium-coldstart.md`（v3、`5502e223`）、
既存レビュー `193-opus-review-round1/2/3.md`

**判定: 収束していない。Blocker 1・Major 4・Minor 10。**
計画の骨格（既存資産を使い切る、判定主体を Python に一本化、G0/G1 で止まれる、T3a を T1 より先）は
妥当で、round1〜3 で指摘した「既存資産の重複実装」は**残っていない**（`--settle` の再利用、
既存 `classify()` を変更しない、既存前面化を使う、`check_multi.py::phys_in_awase` の import 再利用は
いずれも正しい判断）。問題は (1) CI キャッシュの穴で T3a が必ず失敗すること、(2) 掃引の根拠にしている
しきい値の帰属が実装と違うこと（**round2 M1 の私の指摘も同じ stale doc に依拠していたので併せて訂正する**）、
(3) T4 の採否基準が flake と撤去効果を分離できないこと、(4) 既存ワークフローが必要としている
belief 合わせが `driver=chrome` 経路から抜け落ちていること。

---

## Blocker

### PB1. T3a はビルドキャッシュに阻まれ、`dist/chrome_probe.exe` が存在しないまま走る

**根拠**

- `.github/workflows/e2e-ime.yml:146-152`
  ```yaml
  - name: ビルド済み成果物のキャッシュ
    id: bin
    uses: actions/cache@v4
    with:
      path: dist
      key: e2e-bin-${{ matrix.cfg.name }}-${{ hashFiles('src/**', 'crates/**', 'Cargo.toml', 'Cargo.lock', 'config.toml', 'tools/e2e/ime_key_matrix/ablations/**') }}
  ```
  **`.github/workflows/e2e-ime.yml` 自身は `hashFiles` の対象に入っていない。**
- ビルドステップは `:167` `if: steps.bin.outputs.cache-hit != 'true'` でガードされている（`:167-176`）。
- 新規構成 `ch-smoke` は mutator 無しなので、`cfg()` の `bin=bin or (name if mutator else 'baseline')`
  により `bin='baseline'` になる。したがってキャッシュキーは既存 `baseline` と**完全に同一**。
- T3a の変更対象は計画に「`.github/workflows/e2e-ime.yml` のみ」と明記されている。
  `src/**`・`crates/**` は変わらないので、キャッシュは必ずヒットする。

**失敗シナリオ**

`ci/e2e-chrome` への最初の push で、`build` ジョブはキャッシュを復元してビルドをスキップし、
`dist/` には `awase.exe` / `ime_key_matrix_spike.exe` / `config.toml` しか入らない。
`e2e` ジョブの `driver=chrome` ステップは `chrome_probe.exe` が無くて落ちる。
計画の G1 は「ここで失敗する場合（Chromeが無い/前面化できない/GJIが効かない）、以降のCI関連タスクは止めて
T0の実機結果だけでADRを閉じる選択肢を検討する」と定めているので、**キャッシュの穴が G1 の
「CI では無理」という誤った結論を誘発する**。これが最も高くつく形の失敗。

**修正案**

次のいずれか（(a) が最小）。

- (a) `hashFiles(...)` に `'.github/workflows/e2e-ime.yml'` を追加する。
- (b) キーの接頭辞を `e2e-bin-v2-` に上げる（ワークフロー変更のたびに手で上げる運用になるので非推奨）。
- (c) `chrome_probe` のビルド/コピーだけをキャッシュガードの外に置く。

併せて **T3a の受け入れ基準に
`Test-Path dist\chrome_probe.exe` の確認を1行入れる**こと。artifact が空のまま
「Chrome が動かない」と解釈されるのを構造的に防げる。

---

## Major

### PM1. `ColdKind` の分岐しきい値は 5s ではない。掃引点は妥当だが根拠の記述が誤り

**根拠**

- `crates/awase-windows/src/tsf/gji_fsm.rs:127-136`
  ```rust
  /// gji_idle_ms から cold 種別を分類する。idle 判断の唯一の所在地。
  pub(crate) const fn classify(gji_idle_ms: u64) -> Self {
      if gji_idle_ms >= tuning::LONG_IDLE_MS { Self::Long }        // 10_000
      else if gji_idle_ms >= tuning::MEDIUM_IDLE_PROBE_MS { Self::Medium } // 7_000
      else { Self::Short }
  }
  ```
  **`CHROME_LONG_IDLE_MS` は参照されていない。**
- 5s（`CHROME_LONG_IDLE_MS`）が使われるのは `gji_fsm.rs:344-346 long_idle_ms()` →
  `:470-479 transition_to_warm`
  ```rust
  let long_idle_ms = self.long_idle_ms();
  self.state = GjiState::OnWarm { long_idle_ms };
  Response::emit(extra_actions).with_timer(GjiTimer::LongIdle, Duration::from_millis(long_idle_ms))
  ```
  すなわち **OnWarm から OnCold へ落ちるまでのタイマー長**であって、Short/Medium/Long の cutoff ではない。
- `tuning.rs:88-90` の doc（「`GjiFsm::long_idle_ms_for(InjectionMode::Vk)` が参照し、
  **`ColdKind::classify` の Short/Medium/Long 重症度分岐（cold-start warmup の経路選択に使う）の
  cutoff になる**」）は実装と食い違っており **stale**。
- **レビュアー側の訂正**: round2 M1 と round3 で私が書いた「Chrome(VK) の `ColdKind` 分岐の cutoff は 5s」は、
  この stale doc を裏取りせずに引いたもので誤り。ADR v3 の「現状認識の訂正」最終項と本計画の設計2 は
  それを継承している。**本 ADR が2回繰り返した「docs の記述を現役と思い込む」失敗の3回目**になりかけている。

**実際の帯（Chrome = `InjectionMode::Vk`）**

| `gji_idle_ms` | 状態 | `forces_prepend_f2` | `is_long_cold` |
|---|---|---|---|
| < 5s | OnWarm（LongIdle タイマー未発火） | — | — |
| 5–7s | OnCold / `Short` | false | false |
| 7–10s | OnCold / `Medium` | **true** | false |
| ≥ 10s | OnCold / `Long` | **true** | **true** |

（`forces_prepend_f2`: `gji_fsm.rs:112-114`、`is_long`: `:117-119`）

**したがって掃引点 3000/6000/8000/11000/14000 はこの4帯を過不足なく踏んでおり、点自体は妥当。**
直すのは設計2 の説明だけ。

**失敗シナリオ**: 「5s が cutoff」という理解のまま結果を解釈すると、6000ms の点で
`prepend_f2_warmup` が効かない（Short なので false）ことを「5s を超えたのに cold 扱いされていない＝バグ」と
誤読する。逆に T4 候補1（`prepend_f2_warmup` の撤去）は **8000/11000/14000 の点でしか効かない**ので、
3000/6000 で差が出ないことを「撤去が効いていない」と誤判定する。

**修正案**: 設計2 を「5s = warm→cold タイマー（`transition_to_warm`、`CHROME_LONG_IDLE_MS`）、
7s/10s = `ColdKind::classify` の cutoff（`MEDIUM_IDLE_PROBE_MS`/`LONG_IDLE_MS`、injection mode 非依存）」と
書き分ける。ADR v3 の同記述と `tuning.rs:88-90` の doc コメントも同時に直す
（定数値は変えないので `tuning-constants.md` の実測義務は発生しない）。
T4 候補1 の有効域が 7s 以上であることも計画に書く。

### PM2. 測っているのは keyboard idle ではなく `gji_idle_ms`。R2 は計画内の手段で既に解ける

**根拠**

- `ColdKind::classify` の入力は `gji_idle_ms`。その供給元は
  `platform.rs:742,824,869,882` のいずれも
  `let gji_idle_ms = crate::tsf::observer::gji_idle_ms();` で、**GJI の I/O 観測からの経過時間**。
- 計画の設計2 は「`ensure()`自体がキーを打つので **keyboard idle** はそこから測られる」と書くが、
  awase が cold 判定に使う量はキーの時刻ではない。
- 計画が検出手段として挙げる `[h1-probe] cold=… idle_at_cold=…ms`
  （`output/vk_send.rs:279-280`、値は `composition.idle_ms_at_last_cold()` =
  `tsf/probe.rs:303-305`）は、**まさにこの `gji_idle_ms` のスナップショット**である。

**帰結（計画にとって good news）**: リスク表 R2「GJI休眠（~12s）が実際に起きているか不明」は、
`idle_at_cold` を記録するだけで**直接測れる**。「未知のリスク」ではなく「必ず記録する観測量」に格上げできる。

**失敗シナリオ**: 用語が keyboard idle のままだと、実機で `idle_at_cold` が掃引点とずれたときに
「キーは打っていないのに idle が短い」と混乱する（実際には GJI の I/O が何かの拍子に走って
リセットされた、が正しい解釈）。原因の切り分け先が変わる。

**修正案**

- 設計2 の「keyboard idle」を `gji_idle_ms`（`tsf::observer::gji_idle_ms()`）に直す。
- R2 を「`idle_at_cold` を試行ごとに突き合わせる」に書き換え、**T1 の受け入れ基準に入れる**。
- 突き合わせを可能にするため、T1 の `IDLE` 行に **UTC タイムスタンプ**を必ず入れる
  （`check_multi.py:51-54` が awase ログを時間帯で突き合わせているのと同じ方式。
  `chrome_probe.rs:175 utc_stamp()` が既にある）。

### PM3. T4 の採否基準が「撤去の効果」と「環境の flake」を分離できない

**根拠**

- `summary` ジョブ（`e2e-ime.yml:373-392`）は各ジョブの `rc.txt` の `rc=` **だけ**を読む:
  ```python
  n['pass_' if rc == 0 else 'invalid' if rc == 3 else 'fail'] += 1
  ...
  verdict = 'OK' if n['fail'] > 0 else 'NG(期待FAILだが全PASS=撤去しても壊れない)'   # expect='fail'
  ```
- `ch-idle` は 5 掃引点 × `--idle-repeat=3` = **15 試行/ジョブ**、`matrix.run:[1,2,3]` で **45 試行/構成**。
  checker は「有効回すべて PASS なら 0」なので、rc は 45 試行の AND に潰れる。
- その上で計画の採否基準は「ベースライン=**有効回すべて PASS**／撤去あり=**少なくとも1回 FAIL**」。

**失敗シナリオ**

- ベースライン側: 45 試行に1つでも flake があれば rc=1 → `expect='pass'` の verdict が `NG`。
  実 IME・実 Chrome・GitHub ランナーで 45 連続成功を要求するのは厳しい（R3 が懸念している通り）。
- 撤去側: 45 試行に1つでも失敗があれば `OK`。**撤去に何の効果が無くても、環境 flake だけで OK になる。**
- 両者が同時に起きると、「ベースラインが壊れていて、撤去は効いている」という**最悪の誤読**が
  成立してしまう（実際は単に不安定なだけ）。
- この構造では、`.claude/rules/fix-requires-evidence.md` が撤去実験に期待している
  「このテストには検出力がある」の証明にならない。

**修正案**

1. checker が**試行単位の集計**を機械可読で出す。例: `result.txt` の1行に
   `TALLY pass=44 fail=1 invalid=0 total=45`。
2. `summary` を `rc` の数ではなく `TALLY` の合算で判定するよう拡張する（既存構成は `rc` のまま動くよう
   `TALLY` が無ければ従来ロジックにフォールバック）。
3. 採否を**率の対比**で定義する。例:
   - ベースライン: fail 率 ≤ 2%（135 試行中 2 以下）
   - 撤去あり: fail 率 ≥ 50%
   - かつ掃引点別に見て、**撤去が効くはずの帯**（PM1 より候補1 なら 8/11/14s）で差が出ていること。
4. それが満たせないなら、その撤去候補は「E2E で検出力を示せない」として記録し（`docs/experiments.md`）、
   別候補へ移る。`--idle-repeat` を増やすのは最後の手段（PM4 の Pm8 参照、時間が効いてくる）。

### PM4. `driver=chrome` 経路に、既存ワークフローが必要としている belief 合わせが無い

**根拠**

- 既存 run ステップのコメント（`e2e-ime.yml:294-299`）:
  > 順序: スパイクを先に起動し、初期化(TSF/ActivateProfile)が終わってから awase を起動する。
  > スパイクの初期化中に awase がIMMをプローブすると応答が無く、3回連続の miss で Edit を「IMM不可」と
  > 誤学習して conv を読まなくなる。**`--activate-gji` はキーフックをawase起動後に遅延インストールし、
  > 起動後に `VK_IME_OFF` を1回注入して awase の belief(起動時推定=ON)と実状態(OFF)をそろえる。**
- `chrome_probe` の引数は `--repeat` / `--no-awase` / `--settle` / `--chrome` / `--log` /
  `--shift-tail` / `--storm` のみ（`chrome_probe.rs:519-533`, `:595-608`）。**`--activate-gji` は無い。**
- 計画 T3a-4 の順序は「awase 起動 → 数秒待機 → `chrome_probe.exe` 実行」で、既存と**逆順**かつ
  belief 合わせが無い。

**評価の分解**

- 「IMM 誤学習」の側は Chrome には当てはまらない（Pm9 で後述、計画の注記の**結論**は正しい）。
- しかし **belief 合わせの側は当てはまる**。`ensure()`（`chrome_probe.rs:370-408`）は
  `press(0x16)`（`VK_IME_ON`）→ `sleep(500)` → `probe_logged("setup:IME_ON後")` で始まる。
  awase の起動時 belief が ON、実 IME が OFF の状態でこの物理 `VK_IME_ON` が来ると、
  awase は shadow-toggle として扱い実送信を省く可能性があり、実 IME は OFF のまま。
  次に `press(0xF2)` で救済を試みるが、これも belief とずれていれば噛み合わない。

**失敗シナリオ**: `ensure()` が false を返し、`RESULT INVALID: 前提状態にできなかった` が
全試行で出る → checker は「有効回なし」で rc=3 → `summary` は `判定不能` →
G1 で「CI では Chrome+GJI+awase が動かない」と誤診し、**CI 化そのものを諦める**。
PB1 と同じく、原因が土台側にあるのに結論が「CI は無理」に倒れる形。

**修正案**

- T1 のスコープに `--activate-gji` 相当を追加する（awase 起動後に `VK_IME_OFF` を1回注入してから
  本番シーケンスに入る）。既存スパイクと同じ語彙・同じ意図なので、実装は数行で済むはず。
- または T3a の run ステップで、`chrome_probe` 起動前に belief を揃える1手を入れる。
- いずれにせよ **`ensure()` が false を返した回数を artifact に出す**こと。
  G1 の判断を「Chrome が無い／前面化できない／GJI が効かない」ではなく
  「前提状態を作れない（=belief 不整合の疑い）」まで分解できる。

---

## Minor

### Pm1. `run_per_vk_confirm` の行番号が誤り（3ラウンド連続の同型ミス）

T4 候補2 は「`tsf/warmup/probe_fsm.rs::run_per_vk_confirm`（`:307`付近）」だが、
定義は **`probe_fsm.rs:454`**（`pub(crate) async fn run_per_vk_confirm(`）。
`:300-312` は `VkSentPayload` 構造体と per-VK confirm 節の見出しコメント。
（ADR v3 の `probe_coro_state.rs` という誤りは本計画で `probe_fsm.rs` に直っており、ファイル名は正しい。
残ったのは行番号だけ。）→ `:454` に直す。

### Pm2. T4 候補1 は実在を確認。候補3 は所在が違う

- **候補1 `prepend_f2_warmup`: 実在**。`output/mod.rs:405`（フィールド定義）、
  `:1435`（`(!warm || session_expired) && self.warmup_coord.needs_f2_probe()`）、
  `output/vk_send.rs:256`（`if prepend_f2_warmup {`）。
  `ColdKind::forces_prepend_f2()`（`gji_fsm.rs:111-114`）が Medium/Long のみ true なので、
  **有効域は `gji_idle_ms` ≥ 7s**（PM1 の表）。撤去実験の差が出るのは掃引点 8000/11000/14000 に限られる。
  これは計画に明記すべき（3000/6000 で差が出ないのは正常）。
- **候補3 `RawTsfLiteralRecovery`: `literal_detect_fsm.rs` には無い**。
  出現は `output/tsf_warmup_coord.rs:411,607,633`、`output/mod.rs:309,314`、
  `tuning.rs:158`（「`probe_io.rs` の `RawTsfLiteralRecovery` give-up 分岐」）。
  撤去候補に挙げるなら、どのファイルのどの分岐を撤去するのかを先に確定すること
  （この ADR が2回踏んだ「所在を確認せず前提化する」パターンの再発予防）。

### Pm3. T0 が参照するログ行の文字列が実際と違う

計画 T0 は「ログの `PROBE 行動後: … text=…` の `text` を目で確認する」と書くが、
実際の呼び出しは `p.probe_logged("action後")`（`chrome_probe.rs:654`）で、
出力書式は `:353-356` の
`"PROBE {what}: {} text={text:?} Process(229)={process}"` →
**`PROBE action後: …`**（`行動後` ではない）。grep が空振りする。
`setup:IME_ON後` / `setup:ひらがな後` / `action後2回目` も同じ `PROBE ` 接頭辞で出るので、
「`PROBE action後:` 行だけを見る」と書くと正確。

### Pm4. T1 の `IDLE` 行が `Process(229)` を落としている（最も価値のある観測量）

`probe()`（`chrome_probe.rs:329-350`）は既に
「Chrome が IME に処理させたキー（`keydown` の `key == "Process"` または `kc == "229"`）」の有無を
計算している（`:343-346`）。これは

- `Process=false` … IME がキーを一切見ていない（直接入力に落ちた＝**前提状態のドリフト**）
- `Process=true` かつ英字混在 … IME は見たのにリテラルが出た（**BUG-002 型**）

を分ける唯一の観測量。計画の `IDLE idle=… n=… text=… focus_lost=…` はこれを落としている。
→ `process=` を追加し、checker の `LITERAL` 判定を
`Process=false` → `PRECONDITION_DRIFT`（INVALID 側）、`Process=true` → `LITERAL`（FAIL）に分ける。
PM3 の TALLY もこの区別があると意味が増す。

### Pm5. 厳格判定の「かな（U+3040〜U+30FF）が1文字以上」は配列依存で誤判定しうる

`classify()`（`chrome_probe.rs:263-281`）も同じ範囲を使っているので既存8ケースは通るが、
`k`/`a` の NICOLA 出力が `、`(U+3001)・`。`(U+3002)・半角カナ(U+FF66–FF9F) になる配列・設定では
`OTHER` → FAIL になる。R6 は「特定のかなを期待しない」とだけ書いており、この範囲の話は拾えていない。

→ PASS の定義を **「ASCII 英字 `[A-Za-z]` を含まず、かつ空でない」** に緩める（かな範囲の要求を外す）か、
範囲を U+3000–U+30FF + U+FF61–FF9F に広げる。前者のほうが配列非依存で、R6 の趣旨にも合う。

### Pm6. T1 の `command("clear")` は冗長

`probe()` は末尾で `let _ = self.command("clear", "cleared"); sleep(150);` を実行済み（`:347-348`）。
`ensure()` は最後に必ず `probe_logged` を通る（`:374-408` のどの分岐でも）。
したがって `ensure(Kana)` 直後にページは空。害は無いが手順から落としてよい。

### Pm7. ページの 30ms ポーリングが idle 中も回り続ける（リスク表に無い）

`chrome_probe.rs:57-64` の検証ページは
```js
async function poll() { ... setTimeout(poll, 30); }
```
で **30ms 周期**の `fetch('/cmd')` を常時回す。14 秒の idle 中に約 470 回のリクエストが走り、
レンダラのメインスレッドとループバック HTTP が動き続ける。

キーイベントは出ないので `gji_idle_ms`（PM2）には影響しない。しかし
**Chrome 側の「アイドル時に起きること」（TSF composition context の破棄・再初期化）を歪めうる**。
BUG-002 の原因は「Chrome が F2 受信後に composition context を非同期初期化する」ことなので、
レンダラが常時起きている状態は、まさに測りたい現象の条件を変える可能性がある。
加えて `:66` の `window.addEventListener('focus', () => t.focus())` により、
idle 中にフォーカスが往復すると `t.focus()` が走り、TSF コンテキストが張り直されうる。

→ リスク表に1行追加する。対策として「`--idle-sweep` 時は idle 中だけポーリング間隔を 1000ms に落とす」が
考えられるが、**ポーリングを完全に止めると `snap`/`clear` が届かなくなる**（`command()` は
`shared.cmd` をページのポーリングが取りに来る設計、`:314-327`）ので、停止ではなく間引きに留めること。
間引き自体が観測を変えるので、まずは現状（30ms）で測り、結果が不安定なら間引き版と比較する、が穏当。

### Pm8. T3b の時間見積もりが約2倍過小（結論は変わらない）

計画: 「15試行×(`ensure`≈2s+打鍵1s)≈45s」。実測ベースの内訳は

- `ensure(Kana)` 最良: `press(0x16)`(40ms保持) + `sleep(500)` + `probe()`
  （30 + 30 + 30 + 350 + `snap`(`command` は最大3s、通常は 1〜2 ポーリング=20〜60ms) + `clear` + 150）
  ≒ **1.1–1.3s**。かなにならず `0xF2` 経路に入ると + 500ms + もう1回 `probe()` で **2.3–2.6s**。
- 計測側の打鍵も `probe()` 相当で **0.6–1.1s**。
- 毎試行 `bring_to_front()`。

実効 45s ではなく **60–90s** 程度。総計 126 + 90 ≒ 3.6 分で「3〜4分」の結論は変わらないが、
PM3 の対策で `--idle-repeat` を増やすと効いてくるので、式を残しておくこと。
なお Chrome 起動（最大 40s の待ちループ + 1500ms + 800ms、`chrome_probe.rs:582-592`）は
プロセス当たり1回なので `--idle-repeat` には乗らない。

### Pm9. 「`cache.toml` 事前投入が不要」の結論は正しいが、理由が違う

計画 T3a の注記は「Chrome クラスは IMM32 分類で `IMM32_UNAVAILABLE_CLASSES` に入り
IMM プローブされない、`class_names.rs:19-35`」としているが、実際の門は別。

- 学習を行う `focus/imm_learning.rs:45-48` は
  ```rust
  if new_app_kind != AppKind::Win32 { return; }
  ```
  で始まる。Chrome は `detect_app_kind("Chrome_WidgetWin_1")` → `starts_with("chrome_")` →
  `AppKind::TsfNative`（`class_names.rs:347-365`）なので、**`AppKind` の門で早期 return** する。
- `IMM32_UNAVAILABLE_CLASSES`（`class_names.rs:19-35`）が決めるのは `AppImeProfile` であって
  `AppKind` ではない（round1 B3 で整理した別軸）。

結論（`cache.toml` 事前投入は不要）は変わらないので実害は無いが、
**理由を間違えたまま書くと、次に「なぜ不要なのか」を再確認するときに誤った門を見に行く**。
`imm_learning.rs:45-48` を根拠に書き直すこと。計画が「**要確認**」と付けている姿勢は正しい。

### Pm10. 規約適合

- **frontmatter**: `id: ADR-193-companion-193-implementation-tasks` / `type: companion-doc` /
  `related_adr` は、`163-implementation-tasks.md`・`176-implementation-tasks.md` と同形。✓
- **index.md**: `:357` に補助資料行あり（`| [193-implementation-tasks.md](...) | ADR-193 実装タスクリスト（Chrome idle-sweep E2E） | [193](...) |`）。✓
- **`fix-requires-evidence`**: 変更対象が `examples/` / `tools/` / `.github/workflows/` / docs のみで
  再発ファミリー（`src/` の warmup/focus/belief/conv/キー選択）に触れない、という整理は正しい。
  checker を Python 単体テスト + fixture で担保するのも趣旨に沿う。✓
- **`complexity-budget`**: `tuning.rs` の `pub const` 追加なし、`RESTRICTED_CALLS` 不変 → 対象外。✓
  （PM1 で勧める `tuning.rs:88-90` の doc 修正はコメントのみで、`tuning-constants.md` の実測義務は生じない。
   その旨を計画に1行書いておくとよい。）
- **`worktree-per-session`**: 「実装ブランチは `develop` から専用 worktree で切る」と明記。✓
- **`experiment-logging`**: 撤去実験で「壊れない」と分かった場合に `docs/experiments.md` へ1行、を
  T5 に入れてある。✓
- **`docs-frontmatter-convention`**: BUG-002.md は round3 m3 の指摘どおり
  日付 2026-09-20・`related_adr: ["ADR-193"]`・「残存リスク」節の削除（42行）まで反映済み。✓

### Pm11. 見落とされているタスク

- **Windows 実機ログの回収手順**: T0 は「clipwire で実機へ（push→ブランチ確認→ビルド→実行）」とあるが、
  `chrome_probe.log` と `awase.log` の**回収**（`clipwire-targets.example.toml:123,129` が
  `Get-Content chrome_probe.log` を持つ）に触れていない。PM2 の `idle_at_cold` 突き合わせには
  awase ログも要るので、両方回収することを明記する。
- **`chrome_probe` の終了コード**: CI では checker が判定主体なので不要だが、
  `chrome_probe.exe` が異常終了した場合（Chrome が見つからない等）に run ステップが
  それを検知して INVALID に落とす経路が計画にない。`:585-587` の「ページが読み込まれませんでした(timeout)」は
  `return` するだけ。ログに `=== 全ケース完了 ===` 相当が無ければ INVALID、という判定を checker に入れること
  （既存 run ステップが `全手順完了` を待つのと同じ考え方、`e2e-ime.yml:307-310`）。
- **artifact のパス追加**: 既存の upload ステップ（`e2e-ime.yml:336-346`）の `path:` は
  `dist/ime_key_matrix_spike.log` を含む。`driver=chrome` では存在しないので警告になる。
  `dist/chrome_probe.log` を追加し、片方が無くても落ちないことを確認する。
- **`plan` の `paths` フィルタ**: `on.push.paths`（`:16-21`）は `tools/e2e/**` を含むので
  checker 追加は拾われる。`ci/e2e-chrome` を `on.push.branches` に足す必要がある点は計画に書かれている。✓

---

## 質問への直接の回答

**Q1（実現可能性・idle の歪み・掃引点）**: 構造上は成立する。`probe()` を内部関数＋ラッパーに分ける
設計は既存の `probe()`/`probe_logged()` の形（`:329-358`）そのままなので素直。ただし
(a) 測っているのは `gji_idle_ms` であって keyboard idle ではない（PM2）、
(b) 30ms の `/cmd` ポーリングは idle 中も回り続け、Chrome 側の idle 挙動を歪めうる（Pm7）、
(c) しきい値の帰属が違う（PM1、ただし掃引点 3/6/8/11/14 は結果的に4帯を正しく踏む）。

**Q2（判定主体の Python 一本化）**: 妥当。Rust 側が生の `text=` だけを出し、
既存 `classify()` を変更しない判断は正しい（既存8ケースの判定を壊さない）。
ただし厳格判定は「かな1文字以上」を外して「ASCII 英字を含まない」に寄せるべき（Pm5）で、
`Process(229)` を判定材料に加えると前提ドリフトと真のリテラル漏れを分けられる（Pm4）。

**Q3（`driver=chrome` 方式）**: 方式の選択自体は妥当（GJI 導入 50 行の複製を避ける利得は大きい）。
ただし PB1（キャッシュ）と PM4（belief 合わせ）が未解決。`config.toml` 書き換えの切り出しは、
現ステップが `${{ matrix.cfg.toggle }}` / `${{ matrix.cfg.general }}` を PowerShell 変数へ
代入してから使う形（`:273-283`）なので、別ステップへ移しても `${{ }}` は各ステップで展開され、
**pwsh の変数はステップをまたいで生存しない**点に注意。切り出し後は書き換え結果を
`dist\config.toml` というファイルとして次ステップへ渡す（現状そうなっている）ので成立する。
`Select-String` による確認行（`:285`）も一緒に移すこと。
`cache.toml` 不要の結論は正しいが理由が違う（Pm9）。

**Q4（G0/G1 と T4 の統計的妥当性）**: **不十分**（PM3）。現行 `summary` は rc の三値しか見ないため、
45 試行の情報が AND に潰れる。試行単位の TALLY と率による対比に変えること。
`expect='observe'` は verdict が `観測` で `bad` に寄与しない（`:386`）ので、
`ch-smoke`/`ch-idle` を observe で始める設計は CI を赤くせず安全。✓

**Q5（T0 の `--settle` 扱い）**: 妥当。R5 で「モードキー後の間隔であり、キー無入力 idle とは別物」と
限定し、確定を T1 に回している整理は正しい。ただし PM1 の帯で言うと、`--settle` でも
`gji_idle_ms` は伸びる（モードキー以降 GJI I/O が無ければ）ので、T0 で「出ない」が出た場合に
それが「本当に出ない」のか「モードキー直後で GJI が warm に戻っている」のかは区別できない。
**T0 の結論で G0 を確定させず、G0 の確定は T1 の idle-sweep 後に行う**と明記すること
（現在の記述は「T0 の後」に G0 を置いており、R5 と矛盾している）。これは Q5 に対する具体的な指摘。

**Q6（順序・依存）**: T2 → T3a → T1 → T3b → T4 → T5 の順は妥当（土台の疎通を新機能より先に確認する）。
T0 並行も可。抜けは Pm11。見積もりは Pm8（約2倍過小だが結論不変）。

**Q7（規約）**: Pm10 のとおり適合。

**Q8（過去2ラウンドと同型の誤り）**: 「既存資産の重複実装」は**無い**（`--settle`、既存 `classify()`、
既存前面化、`check_multi.py` の import 再利用はいずれも正しく既存を使っている）。
一方で**「docs の記述を裏取りせず前提化する」誤りは残っている**: `tuning.rs:88-90` の stale doc を
根拠にした「cutoff は 5s」（PM1。レビュアー側の round2 M1 も同罪なので併せて訂正）と、
T4 候補3 の所在（Pm2）。この ADR が自ら掲げた規律
（「known-bugs の『現在の対策』は必ず現行の実装で存在を裏取りしてから使う」）を、
**`tuning.rs` の doc コメントにも広げる**と書いておくこと。

---

## 判定

**収束していない。** 次までに必須なのは

- **PB1**: `hashFiles` に workflow を追加（1行）＋ T3a の受け入れ基準に `Test-Path dist\chrome_probe.exe`
- **PM1**: しきい値の帰属を 5s（warm→cold タイマー）と 7s/10s（`ColdKind::classify`）に書き分け、
  ADR v3 と `tuning.rs:88-90` の doc も直す
- **PM2**: `gji_idle_ms` への用語修正と、`idle_at_cold` 照合の受け入れ基準化
- **PM3**: 試行単位 TALLY + 率による採否基準
- **PM4**: `--activate-gji` 相当の追加（T1）または run ステップでの belief 合わせ
- **Q5 の矛盾**: G0 の確定タイミングを T0 後 → T1 後に直す

PB1・PM1・PM2・Pm1〜Pm3・Pm5・Pm6・Pm9 は文面/1行修正。
PM3・PM4 と Pm4・Pm7・Pm11 は設計の追加を要するが、いずれも小さい。
これらを反映すれば次ラウンドで収束と判断できる見込み。
