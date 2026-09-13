---
id: ADR-168
title: |-
  journal `KeyInput` レーンの OS auto-repeat 畳み込みでリングバッファ枯渇を防ぐ
status: |-
  起草。opus-adversarial-consult 未実施（実装前に収束させる予定）
related_adr:
  - "ADR-096"
  - "ADR-095"
---

# ADR-168: journal `KeyInput` レーンの OS auto-repeat 畳み込みでリングバッファ枯渇を防ぐ

## 背景

不具合報告 `report_id: 01M2CYZ0SFQH3560A0V1YSGKHP`（LINE で「ここここ」大量出力、
`docs/bug-reports-triage.md` に記録済み、原因未確定）を調査中、journal の
`KeyInput` レーンが**ダンプ時点で完全に満杯**（`DumpTruncated.dropped_key_input:
184` + `emitted_entries` のうち KeyInput 分 328 = 512、`journal_policy.rs::
LaneKind::KeyInput.capacity()` の固定値と一致）だったことが判明した。

ADR-096 が導入した4レーン優先度方式（`state`/`timing`/`actuation` を優先し
`key_input` から間引く設計）は、**200KiB のダンプ byte 予算内に収める**段階の
優先度制御であり、これは正しく機能していた（`dropped_state`/`dropped_timing`/
`dropped_actuation` は全て 0）。しかし今回判明したのは、**byte 予算の選定より
前段**に、`key_input` レーン自体がインメモリのリングバッファ（容量512、
`VecDeque`）で、上限に達すると古いエントリから無条件に消えるという、
ADR-096 が対象にしていなかった別のロスがあるという点である。byte 予算を
いくら引き上げても、この段階で既に消えたエントリは戻らない。

同じ report の journal を実際に読むと、この512件がどう埋まっているかも
明らかになった。LINE（`profile=Imm32Unavailable`）でユーザーが Ctrl キーを
約1秒間押し続けた区間だけで、OS の auto-repeat による
`{vk_code: 162 (VK_LCONTROL), is_down: true}` の**ほぼ同一な KeyInput エントリ
が約50件**連続して記録されていた（Windows の低レベルキーボードフックには
key-up が来るまで同じ vk の `is_down: true` が繰り返し届く。物理的に
2回連続で「押す」ことはできないため、同一 vk の `is_down: true` が
間に key-up を挟まず連続する場合は必ず auto-repeat であり、意図的な
連続タップと取り違える余地がない）。この1回のホールドだけで512件中
約1割を消費しており、他にも `Passthrough`（NICOLA変換に関与しない素通り
キー、IME非関連）のキー入力が大量に記録されている。これらが古い
diagnostically-relevant なエントリ（親指シフト同時打鍵・IME関連VK等）を
リングバッファから押し出していた可能性が高い。

## 問題

`crates/awase-windows/src/runtime/key_pipeline.rs`（`kp_run_inner` 相当）は、
フックが受け取った**すべての**物理キーイベントについて無条件に
`JournalEntry::KeyInput` を1件 `journal.record()` している。ここには
OS auto-repeat による同一キーの反復も、IME/NICOLA変換に一切関与しない
素通りキーも区別なく含まれる。`KeyInput` レーンの容量が固定512件である
ため、キーを長く押し続ける・大量にタイプするだけで、実際に不具合の原因
特定に必要な区間（IME ON直後の cold 期間の打鍵、親指シフト同時打鍵の
タイミング等）がリングバッファから溢れて失われる。

不具合報告機能（ADR-095）は runtime の挙動を変えずに事後診断のための
証拠を残すことが目的（ADR-096 冒頭も同旨）であり、これは「診断の目的を
果たせていない」という意味での不具合と位置づける。

## 決定

### 決定1（主要）: OS auto-repeat の連続 `KeyInput` を1エントリへ畳み込む

`key_pipeline.rs` の journal 記録直前に、直近に記録した `KeyInput` エントリと
比較し、**「同一 vk_code」かつ「両方 is_down: true」**（＝間に key-up を
挟まない、物理的に auto-repeat でしか起こり得ない状態）の場合は、新規
エントリを追加する代わりに直近エントリを更新する。

- 判定は `UnifiedJournal`（または `key_pipeline.rs` の呼び出し側インスタンス
  状態、後述）が持つ「直近 `KeyInput` の vk_code / is_down」の記憶だけで
  完結し、新しいグローバル static は不要（[[feedback_no_raw_global_statics_prefer_static_struct]]
  に沿い、既存の `ImeStateHub`/`UnifiedJournal` 等の既存インスタンスに
  状態を持たせる）。
- 更新内容は最低限 `repeat_count`（またはそれに類する反復回数）フィールドの
  追加と、末尾 `elapsed_ms` の更新。`state_before`/`state_after`/`decision`/
  `physical` は auto-repeat 中は不変（`state_before == state_after ==
  "Idle"` のケースが典型）なので実害なく共有できる。
- 挙動（送信するVK・IME制御）には一切影響しない。`journal` は ADR-096
  背景が明記する通り送信経路から独立した診断専用の記録先であり、この
  変更はレーンへの `record()` 呼び出し内容のみを変える。
- 副次効果として、畳み込んだ分は JSON バイト数も削減するため、ADR-096の
  byte予算選定（`select_tail_within_budget`）にも「本来不要だった重複を
  出さない」形で恩恵が及ぶ（レーン容量とbyte予算の両方が同時に改善する）。

判定基準を「同一 vk かつ間に key-up なし」に限定する理由: この条件は
物理的に auto-repeat 以外では発生し得ない（人間が同じキーを2回連続で
「押す」ことは、間に「離す」を挟まずには不可能）ため、誤って正当な
連続タップ（例: 素早い2度打ち、BUG-105のようなタイミング仲裁が絡む
ケース）を取りこぼす心配がない。時間しきい値のような推測ベースの判定は
用いない。

### 決定2（副次・保留）: `KEY_INPUT_LANE_CAPACITY` の引き上げは決定1の効果を
実機データで確認してから判断する

決定1は「無駄なエントリを詰めない」対策であり、`KEY_INPUT_LANE_CAPACITY`
（現在512、`journal_policy.rs::LaneKind::capacity()`）自体は変更しない。
本ADRでは意図的に据え置く。

- 理由: 現時点では決定1の畳み込みだけでどれだけ実効的な保持期間が伸びるか
  測定していない。効果測定なしに容量だけ倍増する（[[tuning-constants]]
  が timing 定数について求めるのと同種の「実測なしのエスカレーション
  禁止」の精神を、この容量定数にも適用する）のは避ける。
- 次に同種の report で `dropped_key_input` が再び高止まりするようなら、
  その時点の実測値（畳み込み後もなお溢れた件数）を根拠に引き上げを
  再検討する。

### 決定3（副次・却下）: 不具合報告の journal byte 予算
（`LOG_EXCERPT_MAX_BYTES`、現在200KiB）の引き上げは本ADRでは行わない

調査の過程で、`LOG_EXCERPT_MAX_BYTES` は過去に **256KiB から 200KiB へ
引き下げられた**経緯があり（`bug_report.rs` のテスト
`full_size_journal_and_app_log_fit_within_max_body_bytes_without_shrinking`
がその回帰テスト）、journal 200KiB + app_log 200KiB で Cloudflare Worker側
`MAX_BODY_BYTES`（512KiB、`services/report-worker/src/index.ts`）にほぼ
余裕なく収まるよう意図的に調整されている。単純に journal 側の予算だけを
引き上げると、他フィールドを含めた合計が `MAX_BODY_BYTES` を超えて
「送信のたびに自動切り詰めが発生する」という、この値を200KiBへ下げる
きっかけになった問題をそのまま再発させる。

引き上げるなら journal/app_log 双方の予算配分見直しと `MAX_BODY_BYTES`
自体の引き上げ（Cloudflare Workers側の制約確認込み）が必要であり、
「リングバッファに残っている診断価値の高いデータの割合を増やす」という
本ADRの目的（決定1）に対して費用対効果が低い。**却下**し、必要になれば
別ADRで扱う。

## 検討したが採らなかった案

- **`Passthrough`（IME非関与の素通りキー）を `KeyInput` レーンから
  一律除外する**: BUG-105（3鍵仲裁）・`CtrlMuhenkanImeOff` chord
  （BUG-49関連）等、複数の既知バグ調査で `Passthrough` 分類の
  `KeyInput` エントリ（Ctrl押下・修飾キー等）が実際に決め手として
  使われた実績があり、一律除外は別の診断能力を失うリスクが大きい。
  auto-repeat 畳み込み（決定1）ほど「安全に判別できる」基準が無い
  ため見送る。
- **時間ウィンドウベースの重複排除**（例: 同一vkが50ms以内に再度来たら
  間引く）: OS auto-repeat の周期（初回遅延後は数十msおき）に近い
  時間で発生する正当な別入力（親指シフトの同時打鍵、高速タイプ）を
  誤って間引く恐れがあり、「間にkey-upがない」という判別可能な条件が
  既にあるためこちらを採用しない。

## テスト方針

`crates/awase-windows/src/journal.rs`（または `key_pipeline.rs`）の
`#[cfg(test)]` ユニットテストとして、Linux上の `cargo test --lib` で実行可能な
形で以下を追加する:

- 同一 vk_code の `is_down: true` が連続する場合、`KeyInput` レーンの
  エントリ数が増えず `repeat_count`（相当のフィールド）が加算されること。
- 間に `is_down: false` を挟んだ場合は畳み込まれず、通常どおり別エントリに
  なること。
- 異なる vk_code が交互に来る場合（畳み込み対象外）は従来どおり全件記録
  されること。
- `DumpTruncated.dropped_key_input` が、同じ入力シーケンスに対して
  畳み込み前より減ること（回帰確認）。

`fix-requires-evidence.md` の再発ファミリー表には journal 自体は含まれて
いないが、診断基盤の不具合を再発させないという同種の観点から、上記を
本ADR実装コミットに含める。

## 関連

[ADR-096](096-journal-priority-tiers-multi-lane-ring-buffer.md)（本ADRが
対象とする4レーン優先度リングバッファの導入元。byte予算内の優先順位は
正しく機能しており、本ADRが埋めるのはその手前段のロス）、
[ADR-095](095-tray-bug-report-cloudflare-intake.md)（`LOG_EXCERPT_MAX_BYTES`/
`MAX_BODY_BYTES` の由来）、[docs/bug-reports-triage.md](../bug-reports-triage.md)
の `01M2CYZ0SFQH3560A0V1YSGKHP` 行（本ADRの発端）。
