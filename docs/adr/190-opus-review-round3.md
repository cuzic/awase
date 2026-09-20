# ADR-190 opus 敵対的レビュー round3（v4 = `d447c377` 対象）

worktree `/home/cuzic/rust-nicola-worktrees/ci-e2e-scenarios`、HEAD `d447c377` の実コードで
全て引き直した（v4 が新規に入れた行番号・ファイル名・件数を含む）。

## 結論

**Blocker は無い。ADR-190 v4 は実装に進んでよい。**

round1 の Blocker 2件・Must-fix 4件、round2 の Must-fix 10件は、いずれも指摘の意図を満たす形で
反映されている。特に round2 MF6（`UnsafeToToggle` を `falls_through` に含めない理由の付け替え）と
MF4（`ime_profile_driver` の不変条件テストは「削除して型で保証」）は、単なる文言修正ではなく
**規則の根拠を作り直す**形で書かれており、指摘の意図どおり。

以下は実装中に踏むと痛い3点（Must-fix、いずれも ADR に1〜2行足すだけ）と、
あると親切な4点（Should-fix）。どれも設計判断のやり直しは不要。

---

## 点1: 反映の妥当性と新記述の裏取り

v4 が新たに入れた参照は**全て実コードと一致**した。

| v4 の記述 | 実コード | 判定 |
|---|---|---|
| `ime_controller.rs:357` が `FallbackSent` の唯一の生成元 | `:355-357` が `PostKanjiToggle` 腕の末尾 `ImeOpenOutcome::FallbackSent` | ✓（`grep -rn FallbackSent` でも生成は他に無い） |
| `lints/actuation_call_guard/src/lib.rs:77` | `"post_kanji_toggle_to_focused",`（`send_input_safe` の許可リスト内） | ✓ |
| `architecture_guard.rs:1702-1721` をテストごと撤去 | `:1702` が `fn kanji_toggle_fallback_sends_expected_vk_codes()` | ✓ |
| `architecture_guard.rs:2438-2447` の宣言存在検査から `"struct KanjiToggleStrategy"` を除く（除かないと panic） | `:2439-2445` の `for decl in [...]` 配列、`:2446-2449` が `unwrap_or_else(|| panic!(…))` | ✓ |
| `win32.rs:222-223` の制約が解消する | `:221-223` の doc（`kind=kanji_marker` が両方で共用） | ✓ |
| `actuation_chain.rs:205-210` / `:173-196`、`app_ime_policy.rs:44-50` の理由書き換え | ✓ 全て該当 | ✓ |
| `vk.rs:96` / `:127` / `:149` は残す | ✓ | ✓ |
| `crates/awase-settings/src/main.rs:4846` は無変更 | ✓（`ImeKeyKind::KanjiToggle` の doc 1行のみ） | ✓ |
| `transport.rs:292-298`（F2 分岐） | ✓（round2 の訂正が正しく入った） |
| `res2/result-sc-hz-msime-native-suppress-1/dist/awase.log:872` の `success=true send_elapsed=12ms` | ✓（`open=false` の set-open、ADR もそう書いている） | ✓ |
| ROMAN 補完の最悪 130ms・3往復（`capture_blocking` 30ms + GET 50ms + SET 50ms） | ✓（`ime.rs:1141-1145`、`ime.rs:472-498` の `probe_ime_control(...,50,...)` / `actuate_ime_control(...,50)`） | ✓ |
| `tests/journals/` に `KanjiToggle` 0件 | ✓ 再確認。`WriteMechanism` が載るのは `ime_apply/adr108-focus-crossing-success.json` の `ImmCross`/`MsImeDirect` だけ | ✓ |
| `MAX_WRITE_MECHANISMS` 4→3 とフィクスチャ書き換え | ✓（`actuation_decision_record.rs:57`、境界テスト `:719-727`） | ✓ |
| 撤去前後の挙動表（変わるのは Standard×MsIme だけ） | round2 で全組検証済みの内容と一致 | ✓ |

---

## Must-fix（ADR に1〜2行ずつ足すだけ）

### MF-A. `ImeOpenOutcome` も serde derive で bug report に載る —— 「旧 report の再生を諦める」は `FallbackSent` にも及ぶ

v4 は `WriteMechanism::KanjiToggle` についてだけ「撤去前に収集された旧 report は再生できなくなる」と
書いているが、`ImeOpenOutcome` も**コアクレートで `#[derive(serde::Serialize, Deserialize)]`**
（`src/platform.rs:152`）であり、`AttemptRecord.outcome` として同じ `journal_json` に載る。
`FallbackSent` を消せば、`"outcome":"FallbackSent"` を含む旧 report も同様に再生不能になる。

- 実害の確認は済んでいる: `grep -rl FallbackSent --include=*.json`（target 除く）は **0件**。
  現コーパス `tests/journals/actuation_decision/bug-131-report-01m29kdnz.json`（37レコード）には
  `KanjiToggle` / `FallbackSent` / `ImmCross` / `MsImeDirect` のいずれも含まれない。
- ADR の該当文を「`WriteMechanism::KanjiToggle` と `ImeOpenOutcome::FallbackSent` の両方について、
  旧 report の再生は諦める（現コーパスは両方0件で実害なし、確認済み）」に広げること。

### MF-B. `encode_outcome`/`decode_outcome` の番号は**詰めずにギャップを残す**こと

v4 は「`runtime/message_handlers.rs`（`ImeOpenOutcome`↔u8 のワイヤ符号化、プロセス内なので
互換性問題なし）」と書く。互換性の結論は正しいが、**コンパイラの守備範囲が encode と decode で
非対称**である点に触れていない（`message_handlers.rs:736-773`）:

- `encode_outcome` は網羅 `match`（doc が「variant 追加時はここがコンパイルエラーになる」と明記）。
- `decode_outcome` は `isize` の数値 match で **`other => { tracing::error!(...); UnsafeToToggle }`**
  という catch-all を持つ。つまり**番号を詰めて片方だけ直しても、コンパイルは通り、実行時に
  無言で全部 `UnsafeToToggle` に倒れる**（= 非同期 apply 完了が全て「送っていない」扱いになる）。

したがって実装指示は「`FallbackSent => 1` の**行だけ消し、2〜7 は動かさない**（1 を欠番にする）」と
明記すること。番号を詰めたい場合は `decode_outcome` の catch-all を一時的に外して網羅性を
コンパイラに検査させる、という手順まで書かないと安全でない。

併せて、**`encode_decode_outcome_roundtrips_for_all_variants`（`message_handlers.rs:2090` 付近）**
のリストから `FallbackSent` を1件削ること。v4 は `message_handlers.rs` に言及しているが、
このテスト名を挙げていない。しかもこのテストは `runtime/` 配下 = `#[cfg(windows)]` なので
**Linux では存在しない**（MF-C）。

### MF-C. `#[cfg(windows)]` で隠れる範囲を ADR に明記すること（点2でいちばん踏みやすい罠）

`crates/awase-windows/src/lib.rs` の実際のゲート:

- **ungated（Linux で compile / test される）**: `state/`（:37）, `vk`（:39）, `focus`（:26）,
  `tsf`（:91）, `keymap`（:66）
- **`#[cfg(windows)]`（Linux では1行もコンパイルされない）**: `ime_controller`（**:50-51**）,
  `ime`（:49）, `runtime`（:78、`transport.rs`/`message_handlers.rs`/`open_chain.rs` を含む）,
  `platform`（:74）, `journal`（:59）, `win32`（:93）, `output`（:70）, `imm`（:55）

帰結が2つあり、どちらも ADR の主張に直接関わる:

1. **決定2 が根拠にしている安全網は Linux では作動しない。** ADR は
   「片方だけだと `caps_chain_matches_legacy_all_scan`（`ime_controller.rs:1006`）が落ちる」と
   書くが、`ime_controller` は `#[cfg(windows)]` なので、このテストは
   `cargo nextest run --workspace --lib`（CI の Linux ジョブ）や `cargo test --lib` の
   テストバイナリに**そもそも存在しない**（CLAUDE.md が警告している「silently does not exist」
   のケースそのもの）。「この安全網は windows-build CI でのみ作動する」と1行足すこと。
2. **撤去差分の大半（`ime_controller.rs`・`ime.rs`・`runtime/*`・`journal.rs`・`win32.rs`・
   `platform.rs`）は host target の `cargo check` では検証できない。** ADR の検証計画に
   `cargo check --target x86_64-pc-windows-msvc -p awase -p awase-windows --tests --lib`
   （CLAUDE.md の推奨形、リンカ不要）を明記すること。

逆に安心材料として書けること: **今回いちばん壊れやすいテスト群は `state/` にあり Linux で走る** ——
`actuation_chain.rs` の4本、`app_ime_policy.rs` の caps 全数テスト、
`actuation_decision_record.rs` の境界テスト、`ime_profile_driver.rs` の不変条件テスト、
`gji_direct_mechanism.rs`、コア `src/platform.rs` のテスト。
一方 **windows-build CI まで気付けない**のは `ime_key_sequence_golden.rs`（ADR が既に明記）、
`ime_controller.rs` の全テスト、`message_handlers.rs` の roundtrip テスト、
`runtime/transport.rs` の `plan` テスト（決定2 で呼び出し形を変える先）。
この2列の切り分けを ADR に表で置くと実装時に迷わない。

---

## Should-fix

### SF-1. `should_send_accompanying_warmup` は `matches!` なのでコンパイラが掃除漏れを検出しない

コア `src/platform.rs` の2つの関数で扱いが違う:

- `wrote_open_state`（`:203-214`）は**網羅 `match`** → variant を消すとコンパイルエラーで追随を強制。
- `should_send_accompanying_warmup`（`:249-254`）は `!matches!(outcome, Applied | FallbackSent)`
  → **`FallbackSent` を消し忘れてもコンパイルが通る**（消えた variant を書けばエラーになるので、
  正確には「消し忘れ」はエラーになる。逆に `matches!` の腕をそのまま残すと即エラー）。
  実質的な注意点は、この関数が **BUG-113 / ADR-149 の随伴 warmup ファミリー**にあること。
  挙動は完全に不変（到達不能アームの削除）だが、`fix-requires-evidence.md` の warmup ファミリーの
  ファイルに差分が出るので、ADR に「warmup ファミリーに触れるが挙動不変（到達不能アームの削除のみ）」と
  1行残すと、後から `git log` で追う人が「warmup を触った変更」として再調査せずに済む。

### SF-2. 配列長の注記（いずれも Linux でコンパイルエラーになるので安全）

- `state/actuation_chain.rs:665` `const ALL_OUTCOMES: [ImeOpenOutcome; 7]` → 6
- `state/gji_direct_mechanism.rs:239` `const ALL_OUTCOMES: [ImeOpenOutcome; 6]` → 5、
  同 `:279-284` の4要素リテラルリスト → 3

v4 は両ファイルを挙げているので漏れではないが、「長さ注記があるので `state/` の分は
Linux のコンパイルで必ず検出される」と書いておくと、MF-C の表と対になって読みやすい。

### SF-3. `.githooks/pre-push` の対象 regex に「キー選択の SSOT」が入っていない（既存の穴、本 ADR の責任ではない）

`.githooks/pre-push:36` の target:

```
crates/awase-windows/src/(output/|tsf/|focus/|runtime/(ime_coordinator|focus_tracking|key_pipeline|
conv_actuation|open_chain|executor|transport|ime_refresh|message_handlers|outbox|mod)\.rs|
state/(ime|conv_mode|observation_store)|ime_controller\.rs|ime\.rs|input_defer\.rs|platform\.rs|tuning\.rs)
|src/engine/nicola_fsm\.rs
```

**`state/key_sequence_policy.rs` と `state/app_ime_policy.rs`（= 送信キーとチェーンの SSOT）が
対象外**。今回の PR は `ime_controller.rs`/`ime.rs`/`platform.rs`/`runtime/transport.rs` を触るので
警告は出るし、テストも追加するので警告条件も満たす——**実害は無い**。ただし BUG-116 で
`runtime/transport.rs` が抜けていたのと同型の穴なので、ADR の「残る限界」か別件チケットに
1行残しておくと次が助かる（本 PR でやる必要は無い）。

### SF-4. `docs/ime-control-overview.md:35` の図は既に誤り

`│  ImmCross → GjiDirect → KanjiToggle                 │` で **`MsImeDirect` が抜けている**。
v4 は「図(`:35`)・戦略リスト(`:188-190`)・独立節(`:244`〜)を節ごと書き直す」と正しく書いているが、
図が「今日すでに間違っている」ことは書かれていない。書き直しの際に
「撤去に合わせて直す」ではなく「元から誤っていたのを直す」と分かる形にしておくこと
（`.claude/rules/experiment-logging.md` の趣旨: 後から `git log` で理由が辿れるように）。

---

## 点2 の残り: これ以上の撤去対象は見つからなかった

`grep -rln "KanjiToggle|post_kanji_toggle|kanji_toggle"`（target 除く）の 75 ファイルを
v4 の「撤去するもの／残すもの／直す doc／触らない（履歴）」の4分類に突き合わせ、
**未分類のファイルはゼロ**だった。`FallbackSent` 側（`grep -rn FallbackSent`）も
v4 が列挙した 9 ファイル + `message_handlers.rs` のテスト（MF-B）で全部。

実装で踏みそうな罠として残るのは MF-B（ワイヤ番号）と MF-C（cfg(windows)）の2つだけで、
どちらも「ADR に書いておけば踏まない」種類のもの。

---

## まとめ

- **Blocker なし。実装に進んでよい。**
- 着手前に ADR へ追記すべきもの: **MF-A**（`FallbackSent` も旧 report 再生を諦める対象、現コーパス0件は確認済み）、
  **MF-B**（`encode_outcome`/`decode_outcome` は番号を詰めず `1` を欠番にする。`decode_outcome` は
  catch-all を持つのでコンパイラが守らない。roundtrip テストも1件削る）、
  **MF-C**（`ime_controller` は `#[cfg(windows)]` なので `caps_chain_matches_legacy_all_scan` の
  安全網は windows-build CI でのみ作動する／検証は
  `cargo check --target x86_64-pc-windows-msvc -p awase -p awase-windows --tests --lib`）。
- 任意: SF-1〜SF-4。
- 簡素化の観点（round2 の評価を維持）: v4 は M2' も受け入れて `profile` 引数と `#[track_caller]` を
  削る方向に倒しており、**この PR は一貫して純減**（戦略1・`WriteMechanism` 1 variant・
  `MechanismCommand` 1 variant・`ImeOpenOutcome` 1 variant・`unsafe fn` 1本・dylint 許可エントリ1件・
  guard テスト1本・不変条件テスト1本・`win32.rs` の制約1件）。
  `feedback_dont_pile_complexity_in_response_to_adversarial_review` の観点で問題なし。
