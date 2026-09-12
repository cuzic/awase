---
id: ADR-157
title: |-
  force-ON が drift correction に道を譲る調停案（不採用・撤回）
summary: |-
  BUG-110追補7（issue #189、MS-IME/Chrome）の二重SSOT問題に対し、当初「force-ONがdrift correctionの実行中バーストに調停で道を譲る」新機構を設計、opus-adversarial-consult 4ラウンドで収束・実装・実機ソークまで完了させた。しかしユーザー指摘（設計の複雑化）を受け再検討し、既存の`ConvOpenInference`除外ガードに`HeuristicDefault`を1バリアント加えるだけのシンプルな根本修正に置き換え
status: |-
  **不採用（撤回）。採用した修正はdocs/known-bugs.md BUG-110追補9を参照。round1の恒真化に関する知見のみ本ADRに保存**
related_adr:
  - "ADR-087"
---

# ADR-157: force-ON が drift correction に道を譲る調停案（不採用・撤回）

## ステータス

**不採用（撤回）。** BUG-110 追補7（issue #189、MS-IME/Chrome）で確認した
`check_drift_correction()`（生の `desired_open()`）× `apply_force_on_for_imm_broken()`
（`effective_open()`）の二重 SSOT 問題に対し、当初「force-ON が drift
correction の実行中バーストに調停で道を譲る」という新機構（`DriftBurst`
列挙型・`force_on_yields_to_drift` 純関数・専用リトライタイマー・新規
チューニング定数など）を設計し、opus-adversarial-consult 4ラウンドを経て
収束・実装・実機ソークまで完了させた。

**しかしユーザーから「発火する仕組みの上に抑止する仕組みを重ねており設計が
複雑化している」との指摘を受け再検討した結果、既存の `ConvOpenInference`
除外ガード（`check_drift_correction` 内、BUG-19 由来）の対象に
`HeuristicDefault` を1バリアント加えるだけの、はるかにシンプルな根本修正が
見つかった。** これにより本 ADR の調停機構は不要になり、全面撤回した
（revert コミット、`docs/known-bugs.md` BUG-110 追補8参照）。

採用した修正の詳細は `docs/known-bugs.md` BUG-110 追補9を参照。撤回の理由
（設計上の量的・質的な比較、なぜ調停案より優れているか）も追補8・9に記録
してある。

## 本 ADR に残す価値のある知見

調停案自体は不採用になったが、その検討過程で opus-adversarial-consult
round1 が確定させた以下の知見は、将来同種の「2つの独立した書き手が計算する
setpoint を統一する」という素朴な修正案が再提案されたときに参照する価値が
ある（BUG-110 追補8にも要約を記録済み）:

**`check_drift_correction` の `desired` を、生の `desired_open()` から
`IntentStore` 優先 → 観測ベース導出（`derive_any()`/`most_recent_trusted()`）
→ 生フィールドの順に解決し直す、という統一案は成立しない。** `check_drift_correction`
の `observed` 自体が `most_recent_trusted()` そのものであるため、`desired`
も同じ観測ベース導出の経路（フォールバック含む）から取ると、その経路に
落ちた瞬間 `desired == observed` が恒真式になり、2つの独立した推定が一致
したのではなく同じ値を2回読んでいるだけの見せかけの解決になる。この経路は
「明示意図が対象 hwnd に生きている間だけ発火する機構」へ drift correction
を実質的に無効化し、BUG-51 系の正当な回復（明示 OFF 後、フォーカス断絶で
`last_intent` 消失後も実 IME の閉じ忘れを追いかけ続ける必要がある）も
止まりうる。

## 関連

`docs/known-bugs.md` BUG-110（追補3〜9）、[ADR-087](087-open-belief-actuation-warrant-separation.md)、
GitHub issue #189。設計・実装の詳細な変遷（round1〜4のレビュー記録含む）は
git 履歴（`fix/issue189-force-on-yields-to-drift` ブランチの revert 前
コミット群）を参照。
