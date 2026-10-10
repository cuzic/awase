---
id: ADR-251
title: |-
  BUG-197 の報告 journal を replay fixture として固定する(ADR-249 の回帰テスト (a))
summary: |-
  ADR-249 の検証節は「報告 journal の seq 61-64・117-120 を replay fixture にする」としたが、実装 PR #580 には含まれなかった。
  hook の `modifier_snapshot` 作成は `#[cfg(windows)]` で Linux から再生できないため、fixture はエンジン側の入口
  (注入 Ctrl↓ の期限内に注入 V が `ctx.modifiers.ctrl=true` で届く列)の列として切り出し、報告の実列を最小の列だけ残して固定する。
status: |-
  起草(2026-10-10)。未実装。前提: 報告 `01M4J72G985T0FFT6XPN0SWCQQ` が R2 に残っているか(未確認)。
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

- hook の `modifier_snapshot` 作成・`foreign_ctrl` の記録は `hook.rs`(`#[cfg(windows)]`)にあり、Linux の replay からは動かせない。
  再生できるのは、その**結果**(注入 V の `ctrl=true`・`foreign_ctrl=true`)を入力にしたエンジンの判定まで。
- [ADR-250](250-boundary-journal-and-log-unification.md) が journal の形を作り直す予定。wire 形式そのものを fixture に固めると、その変更のたびに壊れる。
- 所有者判断(2026-10-06): 古い journal・コーパスは捨ててよい。fixture は「いま何を守るか」が分かる最小限にする。

## 決定(案)

1. **fixture は列の最小形で持つ**: 各打鍵を `{t_us, vk, scan, event_type, injected, foreign_ctrl, ctrl}` だけで書く。報告の wire 形式(`log_excerpt` の JSON)はそのまま置かず、
   抽出元(report_id・seq 範囲)を fixture の説明欄に書く。ADR-250 で wire が変わっても、読み込み側の変換1か所だけ直せば済む。
2. **置き場所は `crates/awase-windows/tests/journals/key_input/`**(既存の `bug-105-tight-d1.json` と同じ。`README.md` の合成/実記録の区別を足す)。
   ファイル名は `bug-197-foreign-ctrl-paste-01.json`・`-02.json`。実記録であることを README に明記する(合成の bug-105 とは性質が違う)。
3. **期待値は2段**:
   - エンジン入口: 注入 V↓ を `ctrl=true` で入れると `bypass_reason=OsModifierHeld` で PassThrough、V↑ は Suppress されない
     (ADR-249 検証節の `src/engine/tests.rs` と同じ主張を、実際の時刻・順序で再確認する。重複は「報告の実列でも成り立つ」ことに限る)。
   - 対照: 同じ列の `ctrl` を false にすると V が `PendingChar` になる(修正前の挙動、「ふ」の原因)。**この対照が落ちない fixture は意味がない**ので必ず入れる。
4. **取得**: `bug-report-fetch` で `01M4J72G985T0FFT6XPN0SWCQQ` を取り直す(`docs/journal-replay-guide.md` の抽出手順)。
   **R2 に無ければ**、ADR-249 の事実1の値(時刻差 0.44ms・0.54ms・101ms)から合成し、README に「合成」と書く。実記録と偽らない。
5. **範囲外**: hook 側の `modifier_snapshot` の加算(`foreign_ctrl_active` の境界)は `state/foreign_modifier.rs` の単体テストが持つ。replay に載せない。

## 検証

- `cargo nextest run -p awase-windows --test journal_replay`(Linux で走る)。対照の列が修正前の挙動を再現すること。
- fixture 追加後、`crates/awase-windows/tests/journals/key_input/README.md` の記述と、`ci_test_coverage_guard` が新ファイルを拾うことを確認する。

## 未決

- 報告 JSON の R2 保持期限(取得できるか)。取得できないときは決定4の合成に落とす。
- `journal_replay.rs` に `key_input` コーパスの読み込みが既にあるか(`bug-105` は `scenarios.rs` 由来の合成で、`replay_all_journal_fixtures` が読むかは未確認)。無ければ読み込み側を足す。
