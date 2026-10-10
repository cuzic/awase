---
id: ADR-251
title: |-
  BUG-197 の報告 journal を replay fixture として固定する(ADR-249 の回帰テスト (a))
summary: |-
  ADR-249 の検証節は「報告 journal の seq 61-64・117-120 を replay fixture にする」としたが、実装 PR #580 には含まれなかった。
  hook の `modifier_snapshot` 作成は `#[cfg(windows)]` で Linux から再生できないため、fixture はエンジン側の入口
  (注入 Ctrl↓ の期限内に注入 V が `ctx.modifiers.ctrl=true` で届く列)の列として切り出し、報告の実列を最小の列だけ残して固定する。
status: |-
  実装済み(PR #582、2026-10-10、CI run 38046931901 の `test` が pass)。報告 `01M4J72G985T0FFT6XPN0SWCQQ` は R2 から取得でき、実記録の2例(seq 61-64・117-120)を記録のまま fixture にした(合成ではない)。注入 V の ctrl は `ForeignCtrlLatch` を記録の時刻順に通して求め、ラッチを通さない対照で修正前の挙動(`PendingChar`・「ふ」)を固定。hook.rs の配線は範囲外。
related_adr:
  - "ADR-249"
  - "ADR-250"
  - "ADR-163"
---

# ADR-251: BUG-197 の報告 journal を replay fixture として固定する

## 背景

[ADR-249](249-foreign-injected-modifier-ttl.md) は [fix-requires-evidence](../../.claude/rules/fix-requires-evidence.md) の (a) として
「報告 journal の seq 61-64・117-120 を `tests/journals/` の replay fixture にする」と書いた。PR #580 は単体テスト・`architecture_guard`・CI の A/B を入れたが、
この fixture は入れていない(ADR-249 status の「未了」の1つ)。

事実(ADR-249 の事実1、報告の journal で確認済み): 注入 `左Ctrl↓(scan 29)→V↓(scan 47)→V↑→約100ms 後に左Ctrl↑` の列が2例ある(Ctrl の保持は 101.1ms・101.6ms)。

## 制約

- hook の `modifier_snapshot` 作成の配線(`hook.rs::hook_callback` が `ForeignCtrlLatch` をどの順に呼ぶか)は `#[cfg(windows)]` で Linux の replay からは動かせない。
  ただし判断の核 `state/foreign_modifier.rs::ForeignCtrlLatch` と `tuning::FOREIGN_CTRL_TTL_MS` は Linux で呼べるので、記録の時刻順にラッチへ流して注入 V の `ctrl` を求められる。
- [ADR-250](250-boundary-journal-and-log-unification.md) が journal の形を作り直す予定。wire 形式そのものを fixture に固めると、その変更のたびに壊れる。
- 所有者判断(2026-10-06): 古い journal・コーパスは捨ててよい。fixture は「いま何を守るか」が分かる最小限にする。

## 決定

1. **fixture は列の最小形で持つ**: 各打鍵を `{t_us, vk, scan, event_type, injected, alt, shift, ctrl}` だけで、**記録のまま**書く(記録時は `foreign_ctrl` 自体が無く、注入 V の `ctrl` は false。`7e3c8c3d` で追加された欄)。報告の wire 形式(`log_excerpt` の JSON)はそのまま置かず、
   抽出元(report_id・seq 範囲)を fixture の説明欄に書く。ADR-250 で wire が変わっても、読み込み側の変換1か所だけ直せば済む。
2. **置き場所は `crates/awase-windows/tests/journals/key_input/`**(既存の `bug-105-tight-d1.json` と同じ。`README.md` の合成/実記録の区別を足す)。
   ファイル名は `bug-197-foreign-ctrl-paste-01.json`・`-02.json`。実記録であることを README に明記する(合成の bug-105 とは性質が違う)。
3. **期待値は2段**:
   - 通過: 読み込み側(`foreign_ctrl_journal`)が、記録の時刻順に `ForeignCtrlLatch`(`on_injected_down`/`on_up`/`ctrl_for_injected_key`、TTL は本番の `FOREIGN_CTRL_TTL_MS`)を `hook_callback` と同じ順で回して注入 V の `ctrl` を求め、
     エンジンへ流すと V↓/V↑ とも PassThrough(`OsModifierHeld`)。ラッチの記録・期限・解除や TTL を壊すと落ちる。
   - 対照: ラッチを通さず記録のまま(`ctrl=false`)流すと V↓ が `PendingChar`、V↑ が Consume で、出力は「ふ」(`fu`、修正前の挙動)。**この対照が落ちない fixture は意味がない**ので必ず入れる。
4. **取得**: `bug-report-fetch` で `01M4J72G985T0FFT6XPN0SWCQQ` を取り直す(`docs/journal-replay-guide.md` の抽出手順)。
   **R2 に無ければ**、ADR-249 の事実1の値(時刻差 0.44ms・0.54ms・101ms)から合成し、README に「合成」と書く。実記録と偽らない。
5. **範囲外**: `hook.rs` が上の順序でラッチを呼ぶ配線そのもの(`architecture_guard::foreign_ctrl_latch_is_read_only_at_hook_snapshot_and_cleared_on_five_paths` が文字列で固定)と、`foreign_ctrl_active` の境界値(`state/foreign_modifier.rs` の単体テスト)。

## 検証

- `cargo nextest run -p awase-windows --lib key_input_replay_tests`(Linux で走る。`src/key_input_replay_tests.rs`、`journal_replay.rs` ではない)。
- CI: PR #582 の `test` ジョブ(run 38046931901)が pass。

## 実装結果(2026-10-10)

- R2 の報告は取得できた(`wrangler r2 object get`)ので、合成には落ちなかった。`bug-197-foreign-ctrl-paste-01.json`(seq 61-64)・`-02.json`(seq 117-120)。値は記録のまま(alt/shift も報告で false を確認)。
- `journal_replay.rs` は `key_input` を読まない(bug-105 を読むのは `src/key_input_replay_tests.rs`)。そちらに最小形の読み込みとテスト2本(通過/対照)を足した。
