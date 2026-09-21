# ADR-193 敵対的レビュー round2

対象: `docs/adr/193-extend-existing-e2e-harness-for-chromium-coldstart.md`（commit `c7d6ca0b`）
前提: `193-opus-review-round1.md`（Blocker 4 / Major 8 / Minor 5）の反映確認 + v2 で新たに入った主張の裏取り。

**結論: 収束していない。** round1 の指摘はすり替えなく反映されている（B1〜B4・M1〜M7・m2・m5 は
実コードと照合して妥当）。ただし v2 が新しく立てた成功基準そのものが成立しない
**新規 Blocker 1件**がある: 決定4-3 が撤去対象に挙げる `CHROME_PROBE_LONG_IDLE_MIN_MS` は
**2026-07-18 に機構ごと物理削除済み**で、現在の `tuning.rs` に存在しない。
加えて Major 4件（Chrome の long-idle 閾値が 10s ではなく 5s、`--settle` で既に long-idle を
作れる、`e2e-ime.yml` への「接続」の過小見積もり、代替案の注意書きが実在する汚染リスクを
捉えていない）。

---

## round1 指摘の反映確認（すり替え・取りこぼしの有無）

| round1 | v2 の対応 | 判定 |
|---|---|---|
| B1 chrome_probe の再発明 | 「背景: 既存資産の棚卸し」表 + 決定2を「拡張」に変更 | **反映済み**。`examples/chrome_probe.rs:483-486,563,568` の記述と一致 |
| B2 RichEdit ハーネスも既存 | 棚卸し表に `ime_key_matrix_spike` を明記、決定1で「作らない」 | **反映済み**。`ime_key_matrix_spike.rs:414,1555-1557,1781` と一致 |
| B3 go/no-go は実測不要 | 決定1で `class_names.rs:347-365` / `:120-129` を引いて Win32/Standard 確定と記録 | **反映済み**。行番号も正確 |
| B4 目印付き SendInput | 決定3に3条件（目印・`AWASE_TEST_INJECTION=1`・debug ビルド）を必須化 | **反映済み**。`hook.rs:1077-1095` と一致 |
| M1 title 回収 | 撤回 | **反映済み** |
| M2 BUG-002 の条件 / `bあ` 分離 | 「現状認識の訂正」節で分離、決定4-1 に条件列挙 | 分離は正しいが**条件の数値が誤り**（下記 Major M1） |
| M3 旧コミット再現不可 | 決定4-3 で ablation 方式へ | 方式は正しいが**対象定数が存在しない**（下記 Blocker B1） |
| M4 CI 現状認識 | 「現状認識の訂正」で早期 return と明記 | **反映済み**。`e2e_windows.rs:447-464,2687-2694` / `ci.yml:343,347-355` と一致 |
| M5 WT の読み戻し | Read-Host + ファイルと明記 | **反映済み**。`e2e_windows.rs:2849-2875` と一致 |
| M6 判定対象 `AppImeProfile` | 決定1で両方に言及 | **反映済み** |
| M7 クラス名偽装案 | 代替案節に追加 | 追加されたが**注意書きが的外れ**（下記 Major M4） |
| M8 未確認3件 | Chrome 同梱のみ未確認に残す | **反映の判断が正しい**（下記 m4、レビュアー側の訂正） |
| m1〜m5 | 行番号整理・ADR-186 追加・「3つ目のコピーを作らない」 | **反映済み** |

取りこぼし・すり替えは無い。v1 からの方針転換は実コードの裏取りに基づいている。

---

## Blocker

### B1（新規）. 決定4-3 の撤去対象 `CHROME_PROBE_LONG_IDLE_MIN_MS` は存在しない。BUG-002 の対策機構は削除済み

**根拠**

- `grep -rn "CHROME_PROBE" --include=*.rs` の結果、`crates/awase-windows/src/tuning.rs` に
  **1件もヒットしない**。ヒットするのは docs のみ
  （`docs/known-bugs/BUG-002.md:23,24,28,39,41,43`、`docs/known-bugs/BUG-024.md:368,456`、
  `docs/adr/081-per-profile-capability-driver-decomposition.md:210-211`）。
- `crates/awase-windows/src/tuning.rs:87-100`（`CHROME_LONG_IDLE_MS` の doc）が明記:
  「予防的な Chrome プローブ最小待機の延長（20ms→200ms）機構自体は **2026-07-18 に撤去した**
  （BUG-24 参照、**per-VK confirm に一本化**）。この定数の元々の実測根拠…は撤去された機構向け
  だったが、値自体は `ColdKind` 分岐の cutoff として引き続き使われている。」
- `docs/known-bugs/BUG-024.md:367-369` が削除内容を列挙:
  「`output/vk_send.rs::send_romaji_batched`（Chrome）: F2 事前送信（`SendMessageTimeout`
  + `SendInput` の二重送信）・**probe 事前待機（`CHROME_PROBE_MIN_MS`/`MAX_MS`/
  `LONG_IDLE_MIN_MS`/`MAX_MS`）の計算・送信コードを削除**。」
- 現行コードにもその痕跡がある: `output/vk_send.rs:279`
  `"[h1-probe] cold={cold_seq} idle_at_cold={}ms F2/probe待機省略 → per-VK confirm へ"`、
  `:325`「romaji は per-VK confirm（`ChromeProbe`/`tsf_probe_coro_body`）がそのまま送る」。

**失敗シナリオ**

決定4-3 の通りに `ablations/` へ「`CHROME_PROBE_LONG_IDLE_MIN_MS` を旧値へ戻す」スクリプトを書くと、
対象文字列が存在しないので差分が出ず、`e2e-ime.yml:161-168` の
`test -n "$(git diff --stat)" || { echo "撤去が差分を作らなかった(コードが変わって撤去箇所が消えた?)"; exit 1; }`
に引っかかって**必ず fail する**。ガード自体は正しく働くので事故にはならないが、
本 ADR の成功基準（「撤去あり=FAIL/撤去なし=PASS」）は**書き出した瞬間に成立しない**。

より深刻なのは前提の方で、v2 は「BUG-002 の修正が現役である」ことを確認せずに
「その修正を撤去して再現させる」という検証計画を組んでいる。これは v1 の
「既存資産を棚卸ししていなかった」と**同型の誤り**（今度は棚卸し先が `docs/known-bugs/`）。
`docs/known-bugs/BUG-002.md:20-28`「現在の対策」の表は 2026-07-18 以降 stale である。

**修正案**

1. 決定4-3 の撤去対象を、**現在 BUG-002 の症状を防いでいる機構**に差し替える。候補は
   per-VK confirm 側（`tsf/warmup/probe_coro_state.rs::run_per_vk_confirm`、
   `tsf/warmup/literal_detect_fsm.rs`、`tsf/observer.rs:581` 周辺）。どれを撤去すると
   `という→toいう` が戻るのかを、まず現行コードで特定してから ADR に書く。
2. 同時に `docs/known-bugs/BUG-002.md` の「現在の対策」表を更新する。
   `.claude/rules/fix-requires-evidence.md` (b) が言う「人間可読な再発防止」が、
   削除済みの定数名を指したまま放置されている状態を本 ADR で直すのが筋。
   （BUG-024 側には削除の記録があるのに、BUG-002 側に反映されていないのが原因。）
3. 「その機構が今も現役か」を確認する手順自体を検証計画に入れる（v1/v2 で2回続けて
   この確認を飛ばしている）。

---

## Major

### M1（新規）. Chrome(VK) 経路の long-idle 閾値は 10s ではなく 5s

ADR は決定4-1 と「現状認識の訂正」で、BUG-002 の条件を「keyboard long idle(>10s)」としている
（`docs/known-bugs/BUG-002.md:24` の記述をそのまま引いたもの）。しかし現行コードでは:

- `crates/awase-windows/src/tsf/gji_fsm.rs:1012-1018`
  ```
  pub(crate) fn long_idle_ms_for(mode: InjectionMode) -> u64 {
      match mode {
          InjectionMode::Tsf => tuning::LONG_IDLE_MS,      // 10_000
          InjectionMode::Vk => tuning::CHROME_LONG_IDLE_MS, // 5_000
          InjectionMode::Unicode => tuning::LONG_IDLE_MS,   // 10_000
      }
  }
  ```
- `tuning.rs:85` `LONG_IDLE_MS = 10_000`、`:100` `CHROME_LONG_IDLE_MS = 5_000`、
  `:83`「**Chrome VK パス固有のアイドル判定は `CHROME_LONG_IDLE_MS` を参照のこと**」。

Chrome は `AppKind::TsfNative` → `InjectionMode::Vk`（`output/types.rs:16-28`）なので、
`ColdKind::classify` の Short/Medium/Long 分岐の cutoff は **5s** である。

**失敗シナリオ**: 「>10s の実 idle が要る」という前提で `e2e-ime.yml:188 timeout-minutes: 25`
への影響を見積もると、必要な倍の待ち時間で設計してしまう。逆に、5s 境界付近で挙動が変わる
ことに気づかず、「10s 待ったのに再現しない/する」の原因を idle 以外に探しに行く。
（`MEDIUM_IDLE_PROBE_MS = 7_000`（tuning.rs:148）という3つ目の閾値もあり、
5s / 7s / 10s の3段で分岐が変わる。）

**修正案**: 決定4-1 の条件を「Chrome(VK) は `CHROME_LONG_IDLE_MS`=5s、GJI/TSF 経路は
`LONG_IDLE_MS`=10s、その間に `MEDIUM_IDLE_PROBE_MS`=7s がある」と書き、掃引点を
この3閾値の内外に置く設計にする。BUG-002.md の「>10s」も B1 の更新時に併せて直す。

### M2（新規）. 「long-idle 条件を作るシナリオがない」は過大。既存 `--settle` で今日作れる

ADR「既存資産でまだできていないこと」は「BUG-002 の long-idle 条件…を作るシナリオがない
（`chrome_probe` 内の待機は最大1.5秒程度の固定 sleep）」と書くが、`chrome_probe` には
**ユーザー指定の待機フラグが既にある**:

- `crates/awase-windows/examples/chrome_probe.rs:523-527`
  ```
  // モードキーを押してから `k`,`a` を打つまでの待ち(ms)。EXPLICIT_IME_SUPPRESS_MS(1500)の内外を比べる用。
  let settle_ms: u64 = args.iter().find_map(|a| a.strip_prefix("--settle=")...).unwrap_or(500);
  ```
- 使用箇所 `:653-654`: `p.press(c.vk, c.shift, 120); sleep(settle_ms);` の直後に
  `p.probe_logged("action後")` が `k`,`a` を打つ。

つまり `--settle=11000`（あるいは M1 を踏まえて `--settle=6000`）と渡すだけで、
**コード変更なしに**「モードキー押下 → 長い keyboard idle → romaji 打鍵」を作れる。
`chrome_probe` は既に `clipwire-targets.example.toml:123` 経由で任意引数を渡せる形で運用されている。

**失敗シナリオ**: 「シナリオが無い」という認識のまま新規コードの設計から入り、
30分で終わる確認（`--settle` を上げて `という` 相当が literal 化するか見る）を飛ばす。
その確認結果次第では B1 の「何を撤去すべきか」も一発で分かる可能性がある。

**修正案**: 決定4-1 の前に「ステップ0: 既存 `chrome_probe --settle=<閾値超>` で
BUG-002 の症状が今も出るかを確認する」を置く。出なければ B1 の通り機構は既に別物になっており、
出れば撤去対象の特定が容易になる。足りないのは `--settle` ではなく
(a) GJI 休眠の制御、(b) 物理F2/プログラム的F2 の打ち分け、(c) 判定（literal 化の検出）であり、
「できていないこと」はその3点に絞って書くべき。

### M3（新規）. 「`chrome_probe` を `e2e-ime.yml` に接続する」の見積もりが甘い（3点セットが要る）

ADR 決定4-2 は1行だが、実際に必要なのは:

1. **ビルド**: `.github/workflows/e2e-ime.yml:172-176` は
   `cargo build -p awase-windows --example ime_key_matrix_spike` のみ。`--example chrome_probe` の追加が要る。
2. **run ステップ**: 同 `:271-299` は
   `Start-Process -FilePath .\ime_key_matrix_spike.exe -ArgumentList '--auto --hold=180 --activate-gji ...'`
   をハードコード。さらに `:290` で
   `cache.toml` に `[imm_capability."ime_key_matrix_spike.exe"] Edit = "works" / RICHEDIT50W = "works"`
   を事前投入している（「CIランナーは初回のクロスプロセスIMMプローブが遅く(実測22ms)、
   awase がスパイク窓を『IMM不可』と誤学習する」ための対策）。chrome_probe は別プロセス名・
   別クラスなので、この行は流用できず、chrome_probe 用の起動・環境整備を別に書く必要がある。
3. **判定スクリプト**: `plan` ジョブの `check` 種別は
   `expect`/`consistency`/`toggle`/`resync`/`vkprobe`（`:51-52`, `:54-56`）で、いずれも
   `ime_key_matrix_spike` のログ形式前提（`check.py`/`check_consistency.py`/`check_toggle.py`/
   `check_resync.py`/`vkprobe_report.py`）。`chrome_probe.log` を判定するスクリプトは存在しない。

**失敗シナリオ**: 「接続するだけ」という見積もりで着手し、`cfg()` のスキーマ拡張
（bin/args/check の追加）と新 checker の実装で当初想定の数倍かかる。あるいは checker を
書かずに「ログを目で見る」運用になり、CI に載せた意味（自動判定）が失われる。

**修正案**: 決定4-2 を上記3点に分解して書く。特に「chrome_probe 用の checker を新規に書く」
ことを明示し、`check_multi.py` の INVALID 判定（`extra=0x0` による人の入力混入検出、`:6`, `:51-54`）
を chrome_probe 側でも再利用できるかを検討項目に入れる。

### M4（新規）. 代替案の「注意」が的外れで、実在する汚染リスクを捉えていない

ADR 代替案節の注意書きは2点を挙げるが、両方とも焦点がずれている。

**(a) 「`detect_app_kind` の doc は『ヒューリスティックで Chrome に昇格する場合あり』と書いている」
→ これは stale doc である。**

`focus/class_names.rs:340-344` の doc に確かにその1行があるが、当の関数本体（`:347-365`）は
`class_lower.starts_with("chrome_")` 等の**単純な文字列一致3分岐のみ**で、ヒューリスティックも
昇格も存在しない（round1 B3 で確認済み、v2 の決定1も同じ結論を書いている）。
未確認事項の4番目「分類ヒューリスティックの昇格条件」は、**存在しないものを調べる計画**になっている。

**(b) 本当の危険は `InjectionModeStore` の class_name 単独キー + 永続化である。**

- `focus/classifier.rs:429-453`: `InjectionModeStore` は `HashSet<String>`（class_name のみ）で、
  `learn_tsf()` が `save()` してファイルに**永続化**する。`has_tsf(class_name)` で引く。
- `focus/tracker.rs:115-123 injection_hint_for`: `has_tsf(class_name)` が true なら
  `InjectionHint::ForceTsf` を返す。これは `output/types.rs:18` で `AppKind` より優先される。
- 学習の発火点: `tsf/warmup/probe_fsm.rs:242`「フォーカス中クラスを `InjectionModeStore` に学習し
  injection_mode を Tsf に昇格させる」、`platform.rs:482-484`
  「`[injection-mode] {class_name:?} → Tsf 事後昇格（GJI write 未観測）」。

つまりハーネスが `RegisterClassExW` で `Chrome_WidgetWin_1` を名乗ると、事後昇格が一度でも
発火した時点で **`Chrome_WidgetWin_1` がユーザーのキャッシュファイルに恒久的に書かれ、
実物の Chrome が以後ずっと `InjectionHint::ForceTsf` に固定される**。
`ImmCapabilityStore` は BUG-107（winit の汎用クラス名によるプロセス間衝突）で
`(process_name, class_name)` キーに直されたが、**この store は直っていない**ので、
BUG-107 と同型の汚染がそのまま起きる。

**失敗シナリオ**: 代替案を安いと判断して試し、テスト実行後にユーザーの Chrome の
injection mode が変わったまま戻らない。しかも症状（Chrome で Tsf 注入になる）は
テストと無関係なタイミングで現れるので、原因に辿り着きにくい。

**修正案**: 代替案の注意書きを差し替える。
(1) 「`detect_app_kind` の『昇格』doc は stale（本体に該当ロジック無し）」と記録し、未確認事項4を削除する。
(2) 実リスクとして `InjectionModeStore`（class_name 単独キー・永続化）と、`ImmCapabilityStore`
（`(process_name, class_name)` キーなので相対的に安全）の非対称を書く。
(3) 採用するなら、キャッシュ保存先（`app/bootstrap.rs:644` が渡す `base_dir`）をテスト専用に
差し替えるか、実在しないクラス名（例: `Chrome_WidgetWin_1_AWASE_E2E`）では分類テーブルに
一致しないので**偽装の目的を達しない**というジレンマを明記すること。

---

## Minor

### m1. 「`chrome_probe` 内の待機は最大1.5秒程度の固定 sleep」の精度

最大の固定 sleep は `chrome_probe.rs:589 sleep(1500)`（起動直後の安定待ち）で、記述自体は正しい。
ただし待機の上限として実際に効くのはポーリング期限の方で、`:582 Duration::from_secs(40)`
（ページ読み込み待ち）、`:318 Duration::from_secs(3)` がある。加えて M2 の `--settle`（可変）。
「固定 sleep は最大1.5秒だが、`--settle` で任意に伸ばせる」と書けば M2 とも整合する。

### m2. 決定4-3 と決定4-4 は文面上は衝突して読める

決定4-4「本体ソースは**新たには**変更しない」と、決定4-3 の ablation（`tuning.rs` 等へ差分を当てる）
は、`e2e-ime.yml:161-168` が `bash ablations/<script>` → `git diff --stat` で確認している通り、
**CI 実行時に作業ツリーの `src/` を書き換える**。矛盾ではないが、現状の書き方だと衝突して読める。

**修正案**: 「コミットされる本体ソースは変更しない。撤去スクリプトは CI 実行時に作業ツリーへ
差分を当てるだけで、リポジトリには `ablations/*.sh` が増えるのみ」と書き分ける。
`complexity-budget` 対象外という判断はこの前提で正しい（同ルールの対象は
`RESTRICTED_CALLS` の許可リストと `tuning.rs` の `pub const` の**新規追加**であり、
既存値を一時的に書き換えるスクリプトは該当しない）。

### m3. 裏取りの結果、v2 の記述で**正しい**と確認できたもの（今後の議論の固定点）

- ablations の「撤去が差分を作らなければ fail」: `e2e-ime.yml:161-168` に実在
  （`test -n "$(git diff --stat)" || { ...; exit 1; }`）。
- `check_multi.py` の `extra=0x0` で INVALID: `tools/e2e/ime_key_matrix/check_multi.py:6`
  「awase のログに、その実行の時間帯の物理キー(engine-input の extra=0x0)がある(スパイク由来は
  extra=0x5350494B)」、`:51-54` の正規表現
  `engine-input\] vk=0x\w+ Key(?:Down|Up) .*? extra=(0x\w+)` で実装されている。
- `chrome_probe` が CI に載っていない: `grep -rn "chrome_probe" .github/ tools/` の結果、
  `.github/` に**ヒット無し**。参照は `tools/e2e/ime_key_matrix/README.md:1,12` と
  `clipwire-targets.example.toml:33,81,123,129`（clipwire 経由の手動実行）のみ。
- `from_class_and_process` がプロセス名で上書き: `class_names.rs:134-149`（`matches_disabled_app`
  一致で `InputRelay`）。正しい。ただし `resolve`（`:151-170`）は `relay_apps` が空なら
  プロセス名の解決自体を省くので、既定構成では不確実要因にならない点を添えるとよい。
- `TEST_INJECTION_MARKER` の debug 限定: `hook.rs:1084-1095`（`#[cfg(debug_assertions)]` /
  `#[cfg(not(debug_assertions))] const fn ... { false }`）。正しい。
- 決定1の RichEdit=`Win32`/`Standard`: `class_names.rs:347-365` / `:19-35` / `:51-59` / `:120-129`。正しい。
- CI 現状認識（早期 return）: `e2e_windows.rs:447-464`, `:2687-2694`, `ci.yml:343`, `:347-355`。正しい。

### m4. M8 の扱いについて — チームリードの判断が正しい（round1 のレビュアー側の誤り）

round1 の M8 は「Chrome/Edge 同梱」「CI で IME が使えるか」「SendInput 到達性」の3点をまとめて
「リポジトリ内で既に答えが出ている」と書いたが、`e2e-ime.yml` が実証しているのは後2者だけで、
**Chrome の有無は実証していない**（m3 の通り `chrome_probe` は CI に載っていない）。
指摘を分解して「未確認」に残した判断を支持する。round1 M8 のその部分は撤回する。

補足（ADR に書き足す価値のある事実）: GitHub の runner image（actions/runner-images の
windows-2022 / windows-2025）は Google Chrome と Microsoft Edge を同梱しており、
既定の導入先は `chrome_probe.rs:484` が探索する
`C:\Program Files\Google\Chrome\Application\chrome.exe` と一致する見込みが高い。
確認は CI に `Test-Path` 1行を足すだけなので、未確認事項にその確認手段まで書いておけば、
「無ければ導入ステップが要る」という保険を実際に使う確率を事前に潰せる。

### m5. 規約適合

`git mv` で旧ファイルを置換、`docs/adr/index.md:197` 更新済み、frontmatter は
`id`/`title`/`summary`/`status`/`related_adr` 揃い、`related_adr` に ADR-186 追加済み、
記憶用 wikilink `[[...]]` なし、日本語のみ、1ファイル1トピック
→ `.claude/rules/docs-frontmatter-convention.md` 適合。
ADR-159/163 を related から落としたのは本文で使っていない以上妥当。
なお B1 の修正時に `docs/known-bugs/BUG-002.md` を更新することになるので、
その旨（「BUG-002.md の『現在の対策』表は 2026-07-18 の削除を反映しておらず stale」）を
ADR 本文に1行残すこと。

---

## まだ楽観的な箇所

1. **最大の楽観は B1**: 「BUG-002 の修正が現役である」という前提を、`docs/known-bugs/BUG-002.md`
   の記述だけで採用している。v1 が「既存の実装資産を棚卸ししなかった」失敗だったのに対し、
   v2 は「既存の**ドキュメント記述**が現役かを確認しなかった」失敗で、構造が同じ。
   ADR に「known-bugs の『現在の対策』は必ず現行 `tuning.rs`/実装で裏取りしてから使う」旨を
   一文入れると、同じ轍を三度踏まずに済む。
2. **未確認事項2（HTTP POST のイベント順）は、半分は既に答えがある**: `chrome_probe.rs:41-46` の
   `ev()` は `seq++` を各イベントに振って送っているので、到着順が乱れても**受信側で並べ替えられる**。
   残る本物の懸念は「POST（`fetch(..., {keepalive: true})`）の発行自体がレンダラの処理を
   遅らせ、cold-start のタイミングを変えないか」の方。未確認事項をその1点に絞ると精度が上がる。
3. **決定4-1 の「十数秒の実 idle」見積もりが、マトリクスの規模を考慮していない**:
   `e2e-ime.yml:144-148` は `matrix.cfg`（30構成超）× `run: [1,2,3]` で展開される。
   long-idle シナリオを1構成足すと 3 ジョブ、各ジョブ内でケース数 × idle 秒の待ちが乗る。
   `timeout-minutes: 25` への影響を「見積もってから載せる」と書いてあるのは正しいが、
   見積もりの単位（ケース数 × idle）を明示しないと後で同じ議論をやり直すことになる。
4. **決定2の「Tauri 不採用」は v1 から無検証のまま引き継がれている**: 「ホスト構成が Chrome 本体と
   異なり、cold-start が同条件で出る保証がない」は妥当な仮説だが根拠は示されていない。
   本 ADR の射程では不採用で問題ないので、「未検証の仮説として不採用」と書けば足りる
   （現状は断定形）。

---

## 判定

**収束していない。** 次のラウンドまでに最低限、B1（撤去対象の実在確認と差し替え、
BUG-002.md の stale 表の扱い）と M1（Chrome の long-idle は 5s）を直すこと。
M2（`--settle` で今日試せる）は、直すというより**先にやってみる**ことで B1 の答えが
早く出る可能性が高いので、検証計画のステップ0に置くことを勧める。
M3・M4 は決定の文面の精度の問題で、着手前に直せば足りる。
