# ADR-193 実装計画 敵対的レビュー round2

対象: `docs/adr/193-implementation-tasks.md`（v2、commit `ef4ea1df`）、
ADR 本体 `193-extend-existing-e2e-harness-for-chromium-coldstart.md`（v4）、
`docs/known-bugs/BUG-002.md`（注記のしきい値表現）

**判定: 収束していない。Blocker 1・Major 2・Minor 7。**
計画 round1 の PB1・PM1〜PM4・Pm1〜Pm11 は**すり替え・取りこぼしなく反映されている**（下表で個別に確認）。
ADR 本体と BUG-002.md のしきい値記述の訂正も正確。
しかし v2 で新たに導入した `--align-belief` の仕様が、**既存 `--activate-gji` の本質部分を取り違えている**
（yml のコメントだけを読み、`ime_key_matrix_spike.rs` の実装を読んでいない。**同型の誤りの4回目**）。
これは PM4 が防ごうとした「G1 の誤診」を別の原因で再現する Blocker。
加えて、新しい厳格判定が「awase が動いていない run」を PASS にしてしまう穴（設計1）と、
設計4 の試行数・率の内部矛盾がある。

---

## round1 指摘の反映確認

| round1 | v2 の対応 | 判定 |
|---|---|---|
| PB1 キャッシュ | T3a-1 に `hashFiles` への workflow 追加と `Test-Path dist\chrome_probe.exe`、R9 にも記載 | **反映済み**。`e2e-ime.yml:150-152` の構造と一致 |
| PM1 しきい値の帰属 | 設計2 に4帯の表、`tuning.rs:88-90` を stale と明記、T6 で doc 修正、ADR 本体と BUG-002.md も訂正 | **反映済み**。`gji_fsm.rs:127-136` / `:470-479` と一致。表の `forces_prepend_f2` 列も `:111-114` と一致 |
| PM2 `gji_idle_ms` | 設計2 で用語修正、`idle_at_cold` 突き合わせを T1b 受け入れ基準へ、R2 を「必ず記録する観測量」に | **反映済み**。`platform.rs:742,824,869,882` と一致 |
| PM3 統計 | 設計4（TALLY・率・summary 拡張・T3b で較正） | 方向は正しいが**内部矛盾あり**（下記 PM6） |
| PM4 belief 合わせ | 設計3・T1a の `--align-belief`・`PRECOND_FAIL`・R7 | **反映の意図は正しいが仕様が誤り**（下記 PB2） |
| Pm1 `run_per_vk_confirm` 行番号 | T4 候補2 を `:454` に訂正 | **反映済み**（`probe_fsm.rs:454`） |
| Pm2 候補1 実在・候補3 所在 | 候補1 に `:256`/`mod.rs:405`/`:1435` と「有効域 ≥7s」、候補3 は「所在を確定してから」 | **反映済み**。`output/mod.rs` の出現 309,314,1827,1982,1985 も実在（+1854、+テスト 2108〜2179） |
| Pm3 ログ行の文字列 | T0 を `PROBE action後:` に訂正、`setup:`/`action後2回目` を除く旨も追記 | **反映済み**（`chrome_probe.rs:352-357`） |
| Pm4 `Process(229)` | `IDLE` 行に `process=`、判定を `PRECONDITION_DRIFT` と分離 | **反映済み**。`chrome_probe.rs:343-346` は**行番号まで正確** |
| Pm5 かな範囲 | 「ASCII英字を含まない」に緩和、R6 にも記載 | 反映済みだが**新たな穴**（下記 PM5） |
| Pm6 冗長な `clear` | 設計1 で「idle 前に別途 `clear` は要らない」と明記 | **反映済み**（`chrome_probe.rs:347-348`） |
| Pm7 30ms ポーリング | T1b の注記と R8 | **反映済み**（`chrome_probe.rs:57-64`） |
| Pm8 見積もり | 見積もり式を明示、25分超過時の対処順も追加 | **反映済み** |
| Pm9 `cache.toml` の理由 | `imm_learning.rs:45-48` の `AppKind` 早期 return に訂正 | **反映済み** |
| Pm10 規約 | 規約節、T6 の pre-push 警告の断り | **反映済み**。`.githooks/pre-push:36` の target 正規表現に `tuning\.rs` が含まれることを確認 |
| Pm11 抜けタスク | ログ両方回収・完了マーカー欠落で INVALID・artifact パス追加・`plan` の `paths` | **反映済み** |
| Q5 G0 の確定時期 | G0 を T1b 後へ、T0 は「粗い確認」に格下げ、ADR 決定4-0 にも追記 | **反映済み** |

---

## Blocker

### PB2. `--align-belief` は `--activate-gji` と等価ではない。本質（TSF プロファイルのセッション内アクティブ化）が抜けている

**根拠**

計画 設計3 は `--activate-gji` を「awase起動後に`VK_IME_OFF`を1回注入し、awaseの起動時belief(ON推定)と
実状態(OFF)をそろえる（`e2e-ime.yml:294-299`）」と要約し、`--align-belief` をその1点だけの実装として定義している。
しかし**その要約は yml のコメントであって、実装ではない**。実装
（`crates/awase-windows/examples/ime_key_matrix_spike.rs:1785-1794`）は4つのことをする:

```rust
if std::env::args().any(|a| a == "--activate-gji") {
    activate_gji_profile();                                   // ①
    AUTO_NEXT.with(|n| *n.borrow_mut() = now_ms() + 14000);   // ②
    queue_press(now_ms() + 12000, 0x1A);                      // ③ VK_IME_OFF
    HOOK_AT.with(|h| *h.borrow_mut() = now_ms() + 6000);      // ④
}
```

- **①が本質**。`activate_gji_profile()`（`:1590-1613`）は
  `ITfInputProcessorProfileMgr::ActivateProfile` を GJI の CLSID/プロファイル GUID に対して
  `TF_IPPMF_ENABLEPROFILE | TF_IPPMF_FORSESSION` で呼ぶ。doc（`:1590-1591`）が理由を明記している:
  > `--activate-gji`: GJI(Google 日本語入力)のTSFプロファイルを、セッション内でアクティブにする。
  > **CI(GitHub Actions)のように、`Set-WinUserLanguageList`が次回サインインまで有効にならない環境用。**
- ② awase がアクティブ TIP を検出するまで **14 秒**待つ。
- ③ が計画の言う VK_IME_OFF（`:1790-1792` のコメントが belief 合わせの理由、CI run 35482240969）。
- ④ はスパイク自身の LL フック遅延インストール（`:231-234`）。**`chrome_probe` はフックを張らないので不要。**

つまり計画が写し取ったのは③だけで、**CI で GJI を実際に使えるようにしている①と②が抜けている**。

**失敗シナリオ**

`e2e-ime.yml:207-241` は `Set-WinUserLanguageList` / `Set-WinDefaultInputMethodOverride` / `ctfmon` 再起動までやるが、
スパイク側が①を持っている以上、**それだけでは GJI がセッションのアクティブ TIP になっていない**
（スパイクの doc がそう書いている。ならなければ①は不要のはず）。
`chrome_probe` を `--align-belief`（=③のみ）で走らせると:

1. GJI はインストール済み・言語リスト済みだが**非アクティブ**。
2. `ensure()` の `press(0x16)`（`VK_IME_ON`）も `press(0xF2)` も効かない。
3. `k`,`a` は素の `ka` → `Class::Plain` → `kana_ok` が false → `ensure()` が false。
4. 全試行で `RESULT INVALID: 前提状態にできなかった` → rc=3 → `summary` は `判定不能`。
5. `PRECOND_FAIL` が全件を指すので、計画 G1 の分解表では「belief 不整合の疑い」に倒れる。
   **実際の原因は belief ではなく TIP 非アクティブ**なので、`--align-belief` の調整をいくら繰り返しても直らない。

PM4 は「G1 の誤診を防ぐ」ための指摘だったが、v2 の仕様はそれを**別の原因で再現する**。

**これは同型の誤りの4回目である。**
v1: 既存の実装資産を棚卸しせず前提化 → v2: known-bugs の記述を現役と前提化 →
v3/計画v1: `tuning.rs` の doc コメントを実装と照合せず前提化 → 計画v2: **yml のコメントを実装と照合せず前提化**。
計画の規約節（`:255-256`）が掲げた規律「docs コメントも現行実装で裏取りしてから使う」は
**ワークフローのコメントにも及ぶ**ことを書き足すべき。

**修正案**

1. `--align-belief` を、①②③を行うものとして再定義する（④は不要）。名前も
   `--activate-gji`（既存と同名）に揃えるほうが、読む人が対応を見失わない。
2. **コード共有の設計判断を計画に書く**。`activate_gji_profile()` は `examples/` 内の関数で、
   examples 同士は `use` できない。選択肢は
   (a) `chrome_probe.rs` に ~25 行を複製し、出典（`ime_key_matrix_spike.rs:1590-1613`）を
   コメントで明記する（このリポジトリには文書化された重複の前例がある）、
   (b) `#[path]` で共有モジュールを切る、
   (c) `awase-windows` の lib 側へ出す（**決定4-4「本体ソースは新たには変更しない」に抵触するので不可**）。
   → (a) を既定にし、複製である旨を両側のコメントに残すのが規約と整合する。
3. `--msime` 分岐（`:1594-1604`）の扱いも決める（`ch-*` は GJI 固定なら不要と明記する）。
4. **T1a の規模見積もりを上げる**。現在の T1a は「フラグ1つ + カウンタ1つ」の体裁だが、
   実際には TSF の COM 呼び出し（`CoCreateInstance` / `ITfInputProcessorProfileMgr`）と
   14 秒の待ちが入る。受け入れ基準に「実機で `--align-belief` 付き8ケースが `PRECOND_FAIL=0` で完走」を
   追加すること（現在は「`PRECOND_FAIL`が出力される」だけで、0 であることを要求していない）。
5. G1 の原因分解表に **「GJI が非アクティブ TIP」** を独立項目として追加する
   （`PRECOND_FAIL` だけでは belief 不整合と区別できない）。判別手段として
   `activate_gji_profile()` の戻り（`ActivateProfile` の HRESULT）をログに出すこと。

---

## Major

### PM5. 「ASCII 英字を含まない → PASS」は、awase が動いていない run を PASS にする

**根拠**

- 設計1 の判定: 「`text`が空 → `EMPTY`（FAIL）。**ASCII英字`[A-Za-z]`を含まない** → `PASS`」。
- 既存 `classify()`（`chrome_probe.rs:263-281`）は、まさにこの区別を持っている:
  ```rust
  if t == "か" { return Class::RomajiKana; }   // :270-272  IME ON・かな、Engine は素通し
  if t.chars().any(|c| ('\u{3040}'..='\u{30FF}').contains(&c)) { return Class::Nicola; }
  ```
  `Class::RomajiKana` の doc は「`か`（IME ON・かな、**Engine は素通し**）」（`:240-241`）、
  すなわち **awase のエンジンが効いていない状態**。
- `ensure()`（`:370-378`）は `kana_ok` を `awase` フラグで切り替えており、
  awase 起動中は `Class::Nicola` を要求し `RomajiKana` を不合格にしている。
  **つまり既存ハーネスは「か」= エンジン OFF を明確に区別している。**

**失敗シナリオ**

`ensure()` が通った（= idle 開始時点では awase のエンジンが効いていた）後、idle 中に awase が落ちる／
フックが外れる／belief がずれてエンジンが非活性になると、idle 後の `k`,`a` は `か` を出す。
新しい判定では ASCII 英字が無いので **`PASS`**。

これは ADR round3 M2 と決定4-0 が閉じようとした「不在の assert は壊れたハーネスでも合格する」穴そのもの。
とくに G0 が「出ない」側に倒れた場合、本 E2E の存在意義は
「idle 後にリテラル化しないことの回帰検知」だけになるので、**この穴がそのまま本体の欠陥になる**。

**修正案**

- `IDLE` 行に `awase={bool}`（`--no-awase` の有無）を出す。
- checker に判定を1つ足す: `awase=true` かつ `text` が `RomajiKana` 相当（`か` 単独、より一般には
  「`ensure()` 時の NICOLA 出力と異なる、ローマ字かな変換そのままの結果」）→ **`ENGINE_OFF`**。
  これは症状ではなくハーネス故障なので **`INVALID`** に落とす（FAIL にすると撤去実験の率を汚す）。
- 最も確実なのは、`ensure()` が確認した時点の `text` を**その試行の期待値として記録**し、
  idle 後の `text` と突き合わせること（`k`,`a` の NICOLA 出力は配列固定なので同じ文字列になるはず）。
  R6（配列依存）の懸念も、特定のかなをハードコードせず「直前に観測した値」と比較する形なら解消する。
- 併せて、`--no-awase` 腕を陽性対照として `ch-idle` に1構成入れることを T3b に明記する
  （ADR 決定4-0 の「陽性対照」要件は、現在の計画では G0 の分岐テキストにあるだけで、
  T3b/T4 のタスク定義に落ちていない）。

### PM6. 設計4 の試行数と率が内部矛盾している。2% は「0 件」と同義で、PM3 の問題が縮小再生産されている

**根拠**

- 設計4 は「`ch-idle`は5掃引点×`--idle-repeat=3`=**15試行/ジョブ**、`matrix.run:[1,2,3]`で**45試行/構成**」と書く。
- その2段落後に「ベースラインのfail率≦2%（**135試行中2以下**の水準）」。**45 と 135 が食い違う**
  （135 は 45×3、つまり CI 実行3回分を暗黙に合算した数）。
- 45 試行で「fail 率 ≦ 2%」は上限 0.9 件、すなわち **1 件でも FAIL なら NG**。
  これは round1 PM3 が「ベースラインの1回の flake で NG になる」と指摘した構造そのもの。
- さらに T4 候補1 の判定帯は 8/11/14s に限る（設計2・T4 に明記、正しい）ので、
  実効 n は **3掃引点 × 3反復 × 3run = 27 試行**。この n に対する「2%」は上限 0.54 件＝やはり 0 件要求。

**失敗シナリオ**

ベースラインの真の flake 率が 3〜5%（実 IME・実 Chrome・GitHub ランナーでは十分ありうる）だと、
27〜45 試行中 1〜2 件の FAIL が普通に出る → ベースライン構成が恒常的に `NG` → 撤去実験の対照が
成立せず、T4 の受け入れ基準「別々の2回のCI実行で再現」が永久に満たせない。
一方で撤去側は「≧50%」なので、こちらは flake では満たせず健全。**厳しすぎる側が壊れている。**

**修正案**

1. 試行数の表記を統一する: **1構成 = 15試行/ジョブ × 3ジョブ = 45試行**、
   判定帯を絞る場合は **27試行**、と明記し、「135」は「CI実行3回分を合算するとき」と限定する。
2. ベースラインの基準を「0 件」と同義でない形にする。例:
   - **ベースライン fail 率 ≦ 10%**（27試行で 2 件まで）かつ
   - **撤去あり fail 率 ≧ 50%** かつ
   - **両者の差が 4 倍以上**（比が小さければ環境要因を疑う）。
   n=27、p=0.5 のとき FAIL 件数が 2 件以下になる確率は事実上 0 なので、
   ベースラインを 10% に緩めても撤去の検出力は落ちない。逆に 2% に締めても
   得られるのは「対照が頻繁に NG になる」ことだけ。
3. T3b で得た実測 flake 率から較正する方針（既に書かれている）を残しつつ、
   **「較正後もベースライン基準を 0 件要求にはしない」**という制約を1行入れる
   （PM3 の再発防止）。
4. 判定帯ごとの内訳を `TALLY` に含める（例: `TALLY idle=8000 pass=8 fail=1 invalid=0`）。
   帯ごとの率が見えないと、「候補1 の有効域でだけ差が出ているか」を確認できない。

---

## Minor

### pm1. `chrome_probe` は未知の引数を黙って無視する。フラグが効いたことを証明する手段がない

引数解析はすべて `args.iter().find_map(|a| a.strip_prefix("--…"))` と
`args.iter().any(|a| a == "--…")`（`chrome_probe.rs:519-533`, `:595-608`, `:607-612`）で、
**未知の引数も、綴り違いも、エラーにならない**。

T3a は `--align-belief` を渡すが、T1a より前にビルドされたバイナリ（PB1 のキャッシュ問題と組み合わさると
現実に起きうる）では**無言で無効**になり、「belief 合わせをしたのに PRECOND_FAIL が出る」という
存在しない現象を追うことになる。

→ 起動ログ（`:559` の `"chrome={chrome} port={port} awase={awase} repeat={repeat} settle={settle_ms}ms"`）に
`align_belief` / `idle_sweep` / `idle_repeat` / `pre_settle` を足し、
checker が「期待したフラグが起動ログに出ているか」を確認して、出ていなければ `INVALID` にする。

### pm2. 「未確認」だった `utc_stamp()` × `check.to_ms` の互換性は、**確認できる**（未確認を1つ消せる）

- `chrome_probe.rs:175-188 utc_stamp()` は `format!("{:02}:{:02}:{:02}.{:03}Z", …)` を返す。
- `tools/e2e/ime_key_matrix/check.py:29-31`
  ```python
  def to_ms(t: str) -> float:
      h, m, s = t.rstrip("Z").split(":")
      return (int(h) * 3600 + int(m) * 60) * 1000 + float(s) * 1000
  ```
  末尾 `Z` を落として `HH:MM:SS.mmm` を分解する。**そのまま読める。**

→ T2 の受け入れ基準から「(**未確認**)」を外してよい（fixture での確認自体は残す価値がある）。

**ただし副作用が1つ**: `utc_stamp()` は `secs = t.as_secs() % 86_400` で**日付を持たない**。
`check.to_ms` も同様。`ch-idle` は1ジョブ 3.5〜4 分なので通常は問題ないが、
**UTC 0 時をまたぐ実行では `idle_at_cold` の突き合わせが壊れる**（時刻が巻き戻る）。
既存ハーネスも同じ性質なので新規の欠陥ではないが、設計2 の突き合わせが日付跨ぎ前提で書かれていないことを
1行断っておくこと（対処するなら、突き合わせ時に負の差分を +86400000ms する）。

### pm3. T0 はケース4だけを選べない。実行時間が計画の想定より大きい

`chrome_probe` にケース選択のフラグは無く、`for (i, c) in CASES.iter().enumerate()`（`:636`）で
**常に8ケース全部**を回す。`--settle` も全ケースに効く（`:654`）。

したがって T0 の「ケース4を `--settle=14000` で」は、実際には
8ケース × `--repeat`（既定3）× (`ensure` 1.2〜2.6s + press + **settle 14s** + probe 0.6〜1.1s)
≒ **7 分/回**。掃引6点で **40 分超**（`--settle=500` の対照を含む）。

→ T0 に `--repeat=1` を明記する（それでも ~2.4 分/回 × 6 = 15 分）。
あるいは T1a のついでにケース番号を絞るフラグを足す（ただしスコープが増えるので、
`--repeat=1` で済ませるほうが安い）。いずれにせよ「8ケース全部回る」ことを計画に書いておかないと、
実機作業中に想定の3倍待つことになる。

### pm4. `EMPTY → FAIL` は INVALID 寄りに倒すほうが安全

`command("snap", "snap")` が 3 秒でタイムアウトすると `(String::new(), false)` を返し
（`chrome_probe.rs:335-342`）、`focused=false` → `focus_lost=true` になるので大半は INVALID に落ちる。
残るのは「フォーカスはあるが本当に何も入らなかった」ケースで、これは
IME/ハーネスの異常であって BUG-002 型の症状ではない。
→ `EMPTY` は `process` を見て、`process=false` なら `INVALID`（何も届いていない）に倒すこと。

### pm5. 検証済みの行番号・書式（固定点）

v2 で新しく書いた参照はすべて実コードと一致した:

- `chrome_probe.rs:343-346` の `Process(229)` 計算 — **行番号まで一致**。
- `gji_fsm.rs:470-479 transition_to_warm`（`with_timer(GjiTimer::LongIdle, …)`）— 一致。
- `gji_fsm.rs:127-136 ColdKind::classify`、`:111-114 forces_prepend_f2`、`:117-119 is_long` — 一致。
- `e2e-ime.yml:294-299` の `--activate-gji` コメント — **コメント本文としては一致**
  （ただし PB2 の通り、コメントが実装の一部しか説明していない）。
- `output/mod.rs` の `RawTsfLiteralRecovery` 出現 309/314/1827/1982/1985 — 一致（他に 1854 と テスト 2108〜2179）。
- `imm_learning.rs:45-48` の `AppKind::Win32` 早期 return — 一致。
- `e2e-ime.yml:150-152` の `hashFiles` に workflow が無いこと — 一致。
- `.githooks/pre-push:36` の target 正規表現に `tuning\.rs` が含まれること — 一致（T6 の断りは妥当）。
- `summary` が `results/result-{name}-{run}/` を読み、artifact に `result.txt` が含まれ、
  download の `pattern: result-*` で取れること — 一致。**TALLY を `result.txt` から合算する設計は成立する**
  （run ステップが `| Tee-Object -FilePath result.txt` で stdout を落としているので、
  checker は TALLY を **stdout に** 出すこと）。

### pm6. T3a のステップ切り出しの妥当性（round1 Q3 の再確認）

現ステップ（`e2e-ime.yml:271-285`）は `${{ matrix.cfg.toggle }}` / `${{ matrix.cfg.general }}` を
pwsh 変数に代入して `dist\config.toml` を書き換え、`Select-String` で確認する。
別ステップへ移しても `${{ }}` は各ステップで展開され、成果は**ファイル**として次ステップへ渡るので成立する。
pwsh 変数がステップをまたがない点は計画に明記済み（`:103-104`）。✓
1点だけ: 切り出したステップは `driver` に依存しないので、**`if:` 条件を付けない**こと
（付けると `driver=chrome` 側で `config.toml` が未加工のまま渡る）。計画に明示すると安全。

### pm7. 依存関係（round1 Q6 / 今回 Q3）

`T2 / T1a → T3a →(G1)→ T1b → T3b → (G0確定) → [T4] → T5`、`T0`・`T6` 並行、は矛盾なし。
「T3a は T1b より先」という本文（`:245`）と図（`:241`）も整合。
T3a が T1a に依存する形にしたのは正しい（`ch-smoke` が `--align-belief` を渡すため）。
ただし pm1 の通り、依存が満たされていないことを**実行時に検出できない**ので、
起動ログでのフラグ確認を T3a の受け入れ基準に入れること。

---

## 質問への直接の回答

**Q1（反映の正確さ）**: すり替え・取りこぼしは無い。上表のとおり全項目が反映され、
ADR 本体・BUG-002.md の訂正も実コードと一致する。

**Q2（v2 の新主張の裏取り）**:
- 設計1 の判定: 「ASCII 英字を含まない → PASS」は **engine-off を PASS にする穴**がある（PM5）。
  `process=false → PRECONDITION_DRIFT` の切り分け自体は妥当で、`chrome_probe.rs:343-346` の
  既存計算をそのまま使える。
- `--align-belief` と `--activate-gji` は **等価ではない**（PB2）。実装は
  `ime_key_matrix_spike.rs:1785-1794` の4点で、本質は `activate_gji_profile()`（`:1590-1613`）。
- TALLY × summary の整合は **成立する**（pm5 末尾）。checker は TALLY を stdout に出すこと。
- ステップ切り出しは妥当（pm6）。`if:` を付けないこと。
- 見積もり式は妥当。ただし T0 側の見積もりが抜けている（pm3）。

**Q3（依存）**: 矛盾なし（pm7）。抜けは pm1（フラグが効いたことの検証手段）。

**Q4（裏取りせず前提化した箇所）**: **1件ある**。`e2e-ime.yml:294-299` のコメントを
`ime_key_matrix_spike.rs` の実装と照合せずに `--align-belief` の仕様にした（PB2）。
今回新しく書いた行番号・関数名・書式（`chrome_probe.rs:343-346`、`gji_fsm.rs:470-479`、
`e2e-ime.yml:294-299`、`output/mod.rs` の各行、`imm_learning.rs:45-48`、`.githooks/pre-push:36`）は
**すべて実コードと一致**していた（pm5）。裏取りの水準自体は前ラウンドより明確に上がっている。

**Q5（数値の妥当性・判定帯）**: **妥当でない**（PM6）。45 と 135 の不整合、2% が実質「0 件要求」、
判定帯を絞ると実効 n=27。ベースライン ≦10%・撤去 ≧50%・差 4 倍以上、を初期案にし、
「較正後も 0 件要求にはしない」を制約として書くこと。帯ごとの TALLY 内訳も必要。

---

## 判定

**収束していない。** 次までに必須なのは

- **PB2**: `--align-belief` を `activate_gji_profile()` 相当（①）＋ TIP 検出待ち（②）＋ VK_IME_OFF（③）に
  再定義し、コード共有方法（複製 + 出典コメント）を決め、T1a の規模と受け入れ基準
  （`PRECOND_FAIL=0` で完走）を上げる。G1 の分解表に「GJI が非アクティブ TIP」を追加。
- **PM5**: `awase=` をログに出し、engine-off（`か`）を `INVALID` に落とす。
  `ensure()` 時の観測値を期待値として記録する方式にすると R6 も同時に解決する。
  `--no-awase` の陽性対照を T3b のタスク定義に落とす。
- **PM6**: 試行数の表記統一（45 / 帯を絞ると 27）、ベースライン基準を 0 件要求でない値に、
  帯ごとの TALLY 内訳。

Minor（pm1・pm3・pm4・pm6 の `if:`、pm2 の日付跨ぎ）は文面修正で足りる。
pm2 により「未確認」を1つ消せる（`utc_stamp` × `to_ms` は互換）。
これらを反映すれば次ラウンドで収束と判断できる見込み。
