---
id: ADR-226
title: |-
  CI 実機検証・閉ループ・replay の接続: 不変条件(oracle)の二重実装を解消し、実機ログと閉ループ出力を同じ判定器で比べられるようにするか
summary: |-
  ADR-225 の見送り(報告 journal を混ぜるとプライバシー・バグ固定の問題が出る)を受け、報告 journal を入れずに、CI 実機(合成入力)と閉ループ・replay の接続だけを検討する。
  観察: 同じ不変条件(BUG-162/163 系)が2か所で別々に実装されている。実機 CI は Python の `check_invariants.py`(I1〜I5、`awase.log` のテキスト行を数える)、閉ループは Rust の `tests/support/invariants.rs`(P1〜P3、`h.writes`/`h.predictions` を検査)。
  閉ループは `awase.log` 風の出力を出さず、`check_*.py` は閉ループに掛けられない。
  候補: A 共通 oracle(閉ループ trace を実機ログと同じ行形式で出し、同じ `check_*.py` を掛ける、または不変条件を片方に統一)、B replay 差分による実機 CI の選択実行、C 実機で落ちたシナリオが閉ループで再現するかの自動判定、D 実機ログから擬似 IME の遅延を較正。
  決定: 設計を固めず、まず前提の確認(SP0')を行う。SP0' は「A の価値=実機と閉ループの oracle がずれた実例の有無」と「B/C/D の前提=ログ行から何が復元できるか」を、コード変更なしで測る。
status: |-
  A〜D は見送り(2026-10-04、Opus round1: 観察1・5 が不正確、SP0' の (a)(b)(d) はコードを読むだけで答えが出て着手条件を満たさない)。候補 E(ログの構造化・journal への事実の集約)は所有者の着想で、未レビュー・未決定。以下は起草時の記述:
  起草(2026-10-04、未決定・実装なし)。Opus レビュー1ラウンドを経て、SP0' の結果で採否を決める。
related_adr:
  - "ADR-163"
  - "ADR-191"
  - "ADR-224"
  - "ADR-225"
---

# ADR-226: CI 実機・閉ループ・replay の接続

## 背景

ADR-225 は、不具合報告の journal を fixture 化する案を、プライバシー(公開リポジトリ)・バグ固定・replay の限界(F1)で見送った。
CI 実機のシナリオ(`sc-*`、`ts-*`、`cal-*`)の入力は合成なので、プライバシーの問題が無い。
また PASS したシナリオの出力は「良い出力」なので、バグ固定の問題も生じにくい。ここを接続する余地を検討する。

## 観察(起草時点、Opus に裏取りを依頼する)

1. **oracle の二重実装**: 実機 CI は `tools/e2e/ime_key_matrix/check_invariants.py`(`awase.log` の行を数える。I1 明示意図なしの drift 補正、I2 Unwarranted、I4 GjiFsm OffCold 固着、…、ratchet 付き)。
   閉ループは `crates/awase-windows/tests/support/invariants.rs`(P1〜P3)。BUG-162/163 は両方で別々に検査している。
2. **閉ループは `awase.log` 風の出力を出さない**。`h.trace()` はテスト失敗時の経過表示で、`check_*.py` が読む行(`actuation decision seq=…`、`[drift] correction: …`、`ime open applied … outcome=…`)ではない。
3. **CI は JSON journal を出していない**(ホットキーダンプのみ)。判定は awase.log のテキスト行から行う。
4. `tools/e2e/ime_key_matrix/testdata/*.awase.log` に実機ログ抜粋が置かれ、判定器の回帰テストとして使われている(判定器 ← 実機ログ の接続は既にある)。
5. 閉ループの擬似 IME の真値は CI 実機の格子学習(`grid-tables/*.json`)由来。遅延だけが未較正(ADR-225 F4)。

## 候補

| # | 内容 | 前提(要確認) | 狙い |
|---|---|---|---|
| A | 共通 oracle。(A1)閉ループが実機と同じ行形式のログを出し、同じ `check_*.py` を掛ける。(A2)不変条件を一方に統一(Rust に寄せて実機ログからも評価、または Python に寄せる) | 閉ループが出す情報で `check_*.py` の入力行が作れるか。ratchet の上限(`invariant_limits.json`)を閉ループに適用する意味があるか | 二重実装の解消。同一シナリオの実機/閉ループ差の検知(= ADR-224 の「写しのずれ」の自動検知) |
| B | 実機 PASS ログを保存し、PR の HEAD での決定差分で実機 `sc-*` を選択実行 | `awase.log` の行から決定レコードを復元できる範囲。復元できる行の項目の安定性 | 実機 CI の分数・待ち時間の削減。ADR-163 TH1e と重なる部分は流用 |
| C | 実機で落ちた `sc-*` と同じキー列を閉ループに流し、再現するかを自動判定。しない場合は QUIRKS(クセの目録)に追加 | 実機シナリオのキー列を閉ループの `key/advance_ms` に写せるか(擬似 IME の格子範囲、`on_input` を呼ばない制約、P5) | Linux で反復できる不具合の増加。擬似 IME が写せない箇所の目録化 |
| D | 実機ログの IME 応答遅延の分布(`gji_last_io_ms` 等)を、擬似 IME の遅延パラメータに反映 | 遅延がログから取れること。擬似 IME が遅延を持つ設計か | タイミング依存の再現の増加 |

## 決定

1. **設計を固めない**。まず SP0'(コード変更ゼロ)で前提を測る。
2. SP0' の項目:
   - (a)`check_invariants.py` の I1/I2/I4 と `support/invariants.rs` の P1〜P3 の対応表を作り、同じ性質を二重に検査している件数と、片方にしか無い件数を数える。
   - (b)閉ループで `check_*.py` が読む行を作るために必要な情報が、`Harness` に揃っているかを確認する(`h.writes`・`h.drift_fires`・`h.predictions` の項目と、`check_*.py` の正規表現が要求する項目の差)。
   - (c)二重実装の oracle が過去にずれた実例(閉ループは通るが実機 CI で落ちた、またはその逆)を、`docs/known-bugs/`・PR・ADR-224 から探して数える。
   - (d)`awase.log` の行から復元できる決定レコードの項目を、`ActuationDecisionRecord` の項目と突き合わせる(B の前提)。
3. 着手条件: (c)で実例が 1 件以上、または(a)で二重実装が 3 件以上かつ(b)の差が小さい場合に、A のスパイクに進む。それ以外は「見送り」で閉じる(ADR-224・ADR-225 と同じ基準)。
4. B・C・D は A の結果を見てから扱う。B は(d)の結果が前提、C は A が成立した後の拡張、D は独立に小さく実施可能だが、擬似 IME が遅延を持つ設計かを先に確認する。
5. プライバシー: CI 実機の入力は合成のみ。実機ログを保存する場合も、不具合報告由来のログは混ぜない(ADR-225 決定)。
6. 複雑性: 新しい仕組みを足す場合は、置き換えて消せる重複(二重実装の片方)を明記する。

## 未決

- A1(ログ形式を揃える)と A2(統一)のどちらが小さいか。A1 は閉ループに「ログ風の出力」を足し、A2 は片方の oracle を消す。
- ratchet 上限は実機の揺れ幅から決めた値なので、閉ループ(決定的)にそのまま適用できない可能性がある。
- 閉ループは `on_input`・TSF warmup を写さない(ADR-224、ADR-225 P5)。共通 oracle でも、写されない症状は検査できない。

## Opus レビュー(round1)の結果と訂正(2026-10-04)

| 項目 | 訂正 |
|---|---|
| 観察1 二重実装 | 同じ性質は I1 ⇔ P3(drift の半分)の 1 組だけで、部分的。I2(warrant が下りず止まった書き込み)と P1(warrant が下りて通った書き込み)は表裏で別の性質。P3 に最も近い Python 側は `check_startup.py`、`belief_matches_truth_at_end` の実機版は `check_consistency.py`。BUG-162 の閉ループ側の検査は `harness.rs:580` の `assert!` に埋まっている |
| 観察5 遅延 | 擬似 IME は実測較正済みの読み戻し遅延(`set_readback_lag_ms`、QUIRKS Q3、min21/median33/max62ms)を既に持つ。ADR-225 の F4 の「遅延だけ未較正」も同じ誤り。D の前提は満たされているが、`gji_last_io_ms` は warmup の入力で、ハーネスは warmup を写さないため、較正しても使う側が無い |
| B | 復元不能: `actuation decision` の行は `chain[]`・`attempts[1..]`・`would_have_blocked`・`candidate_was_seen` を持たず、`replay_record` に必要なフィールドが無い。削減も成立しない: PR で走る実機 CI は `atok-passthrough-cold,baseline` の各1回のみで、`sc-*` は PR では走らない。選択実行は、決定差分ゼロで起きる種類(BUG-162/163/170/171)を見つけた run を飛ばす |
| C | A に依存しない(判定は閉ループの既存検査で足りる)。本当の壁は、`on_input`・ForceGuard・warmup・GjiFsm・LiteralDetect・hook・`PhysicalKeyDisposition::plan` をハーネスが写さないこと。A1 で閉ループが作れる行は I1 だけ |
| (c) の実例 | 閉ループが Linux CI で走り始めたのは 2026-10-04(`9ca12626`)で、0 件は構造的。唯一の候補(BUG-163 の別経路 `6f5ef659`)は、写していない層(ForceGuard)が原因で、oracle を共通にしても閉ループは通る = 分類(ii)。着手条件を「(i) oracle の違い」に限る |
| A1 | ハーネスが本番の tracing 書式を真似る = 新しい「写し」。書式が変わると閉ループは黙って 0 件で通る。どちらの oracle も消せず、決定6(消せる重複の明記)を満たせない |
| ratchet | 上限は CI の構成名・実機の揺れ幅(observed_min〜max)で決まり、決定的な閉ループには適用できない |

SP0' の結果: (a) 1 組 < 3、(b) 作れるのは I1 の行だけ、(d) 復元不能。着手条件を満たさず、A〜D は見送る。

## 別件(本 ADR の対象外、実害の修正)

MS-IME 系 `sc-*` 5 構成の `invariant_limits.json` の `i2_unwarranted` 上限が 2〜3。根拠の機構(ActivationSync の `SetOpen`)は ADR-213 P2c(`24672981`)で撤去済みで、
再計測記録が無い(BUG-162 の状態欄)。最大 3 件の退行が通る。`only='sc-*-msime-*,sc-*-gji-msime'` で回し、0 なら上限を 0 に下げる(JSON 1 ファイル・CI 1 回)。

## 候補 E(未レビュー): ログの構造化・journal への事実の集約

所有者の着想(2026-10-04): journal(リング、JSON)とテキストログの2系統を出しているのは筋が悪い。JSON(JSONL)に統一したい。

測定した事実:
- journal のイベントは、リング(JSON、ダンプ時のみ)と `awase::journal` ターゲットの tracing 行(`awase.log`)に二重に出ている。`JournalEntry` は `Serialize` のみ。
- `check_*.py` の正規表現は、`check_invariants.py` 10、`check_startup.py` 9、`check_reopen.py` 9、`check_drift_recovery.py` 8、…。
- `awase::journal` の行を読むのは 3 ファイル。journal に載っていない行(`explicit_intent=` が 10 箇所、`[drift] correction:`、`[gji-fsm]`、`[warrant-shadow]`、`[hook]`)にもチェッカーが依存する。

E の中身: (E1)チェッカーが読む事実を `JournalEntry` に載せ、テキストを journal から派生させる(独立した `tracing::debug!` を減らす)。(E2)出力を JSONL にし、チェッカーは `json.loads` で読む。
**これは oracle 共有(A)とは別の価値**(正規表現の脆さ、書式変更で黙って 0 件になる問題の解消)を狙う。閉ループが写していない層の問題は解決しない。
採否の前に必要な測定: 書式変更による「黙って 0 件」の実例の数、移行量(約 20 ファイルの正規表現と `testdata/*.awase.log`)、ログ量の増加(ADR-222 のリング gzip との関係)。

### E の小さな試行(2026-10-04、PR 化)

構造化には進まず、「Rust 側のログ文言が変わってチェッカーが黙って 0 件になる」ことだけを塞ぐ検査を足した:
`tools/e2e/ime_key_matrix/test_log_anchors_in_rust_source.py`(27 個の断片 × 読むチェッカーの表)。
- 各断片が Rust ソース(`src/`・`crates/`)に存在すること、およびチェッカーがその断片をまだ読んでいること(表の腐敗防止)を検査する。
- 実機不要・数秒。PR の `invariants-unit`(`e2e-ime-smoke.yml`、`crates/awase-windows/**` の変更で起動)で走る。
- 検出力の確認: `tsf/output.rs` の `[raw-tsf-literal] flush escape=` を一時的に書き換えると FAIL する。
- 限界: tracing の構造化フィールド名(`seq=`・`outcome=`・`source=`)、書式引数で組み立てられる部分、行が実際に出る経路(到達性)は見ない。
- 表を作る過程で、断片の多くが書式引数・構造化フィールドで組み立てられており(`send_keys: mode={:?}`、journal.rs の message + fields)、
  「ソースの文字列リテラルを grep する」方式が成り立つのは固定部分に限ると分かった。構造化(E1/E2)に進む前に、この検査で足りるかを様子見する。
