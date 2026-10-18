---
id: ADR-266
title: |-
  観測できない入力モードの既定のかな(種)は belief に残す。ただし「種」であることを理由で区別し、観測・証拠源が低い基準で上書きできるようにする(BUG-204)
summary: |-
  閉じた IME を開くとき、belief のモードが観測されないまま既定のかな(`AssumedRomaji{KeyEffectPrediction}`)の種だと、IME が半角英数を覚えていると食い違う(BUG-204。閉ループのランダム探索で 2878 打鍵中 32 件)。
  素朴な直し方(種を belief に書かない、閉→開で `Unknown` を返す)は**逆効果**: `InputModeState::is_romaji_capable()` は `Unknown` を偽とするため、Engine が `NotRomajiInput` で止まる。
  種は ADR-191(GJI の状態の範囲)決定2・4の実測(新しい窓は閉×ひらがな)にもとづく設計で、読めない窓(Chrome など)で NICOLA を使えるようにする土台でもある。
  本 ADR は、種を維持し(挙動不変)、`AssumedReason::DefaultSeed` を足して表の予測と区別し、診断・テスト・証拠源(ADR-261 字形、ADR-264 メニュー)が「種」を低い基準で上書きできる受け口を作る。
  候補の集合は採らない(これは非決定ではなく、観測できないだけ)。実害の頻度は未測定で、測定計画を決定に含める。
status: |-
  起草(2026-10-18)。実装なし。Opus レビュー未実施。
related_adr:
  - "ADR-191"
  - "ADR-239"
  - "ADR-261"
  - "ADR-264"
---

# ADR-266: 観測できない入力モードの既定のかな(種)

## 背景

[BUG-204](../known-bugs/BUG-204.md): ATOK で IME が閉・半角英数(conv 0x10)で始まり、belief の入力モードが `Unknown` のとき、最初の打鍵(閉のまま押す表に無いキーや閉→閉のキーでも)で
`AssumedRomaji{KeyEffectPrediction}`(かな)が belief に入る。続く IME を開く打鍵は、閉→開の `after_conv` が不明なので `mode: None`(変えない)になり、かなのまま残る。実 IME は 0x10 で開く。
閉ループのランダム探索(PR #634)では、2878 打鍵中 32 件(閉・半角英数で始まる 16 シード × 窓 2 通り。`EXPECTED_SEEDED_GAP`)。Opus レビュー round 2 の r2-M1 は、欠陥の本体を
「仮定の種を belief に書き、観測したモードと区別できなくなること」と指摘した。

## 調査で分かったこと

1. **種は意図した設計で、実測の裏づけがある。** `kana_mode()`(`key_effect_predictor.rs`)は `predict_in_table` の `seeded`(入力モードが `Unknown`)で効果に入る。単体テスト
   `unknown_mode_is_seeded_with_kana_so_prediction_can_start` が「読めない窓の起動直後でも、既定のひらがなを種にして予測を始める」を固定している。ADR-191(GJI の状態の範囲)決定2は
   「新しいスレッド/プロセスの窓の初期状態は閉×ひらがな(0x09/0x19)」(GJI・MS-IME 本体とも)、決定4は「追跡が不明のときだけ既定のひらがなを種にする」。
2. **種は Engine の有効化の土台でもある。** `InputModeState::is_romaji_capable()` は `ObservedRomaji` と `AssumedRomaji` だけを真にする(`src/engine/mode_state.rs`)。`compute_state`(`engine.rs`)は
   真でなければ `Inactive(NotRomajiInput)` にする。つまり **`Unknown` のままだと Engine は有効化されない**。読めない窓(Chrome・一太郎など)は観測が来ないので、種が無ければ NICOLA が使えない。
3. **読める窓では観測が直す。** 直す手段が無いのは、観測できない窓だけ。閉の間は変換モードを観測しない(BUG-57 / ADR-239: 閉じた IME の conv=0 を英数と誤読する)ので、起動時に閉なら belief は種のまま開くまで残る。
4. **閉→開の規則は決定的**(`grid-tables`: ATOK 10/10、GJI の MS-IME プリセット 18/18 で閉じる前の変換モードを復元。MS-IME 本体は 閉 c10 から開くと 0x19)。非決定ではなく、閉の間の変換モードを awase が
   読めないことが問題。ただし試行は各セル 2 回以下(MS-IME 本体は 227 セル中 206 が 1 回)。
5. **実害の頻度は未測定。** 食い違うのは「IME が既定(ひらがな)と違う状態を覚えていて、awase がそれを観測する前に開く」場合で、ランダム探索の開始状態(25% が閉・半角英数)は人工的。

## 検討した案

| 案 | 内容 | 判断 |
|---|---|---|
| A | 種を belief に書かない(`Unknown` のまま) | **却下**。調査 2 のとおり Engine が止まる。いちばん多い場合(IME がひらがなを覚えている)を悪くする |
| B | 閉→開で `after_conv` が不明のとき、belief が種なら `Unknown` を返す | **却下**。開いた直後に Engine が止まる(A と同じ) |
| C | 閉の変換モード(`conv_raw`)から種を取る | **却下**。BUG-57 / ADR-239(閉の conv の誤読)。MS-IME 本体は 閉 c10 → 開 0x19 で外れる |
| D | 候補の集合(3 値・4 値)にする | **却下**。所有者の懸念(決定的な欠陥を覆い隠す)。これは非決定ではなく観測不能で、集合にしても Engine の判断(`is_romaji_capable`)は 1 つに決めるしかない |
| E | 種を維持し、理由で区別する(`AssumedReason::DefaultSeed`) | **採用**。挙動は変えず、種を観測・証拠源・診断が見分けられるようにする |
| F | 証拠源(字形・メニュー)で直す | ADR-261・264 の担当。本 ADR は受け口だけ作る |

## 決定

1. **種は維持する(挙動不変)。** `Unknown` のとき最初の打鍵で既定のかなを入れる。閉→開の `mode: None`(変えない)も変えない。
2. **種に専用の理由を付ける。** `AssumedReason::DefaultSeed` を足し、`predict_in_table` の種の経路(表に無いキーの分岐と `or_else(|| seeded.then(...))`)だけがこれを使う。表の予測がかなに合わせるとき
   (`mode_effect`)は従来どおり `KeyEffectPrediction`。`kana_mode()` が両方で使われている現状(唯一の生成点)を、2 つの関数に分ける。`is_romaji_capable()` は `DefaultSeed` も真にする(Engine は従来どおり有効化される)。
3. **`PredictedEffect.mode` の `None` が「変えない」と「不明」を兼ねている件は、今は型を分けない。** 唯一の消費側(Engine の有効化)が「不明」を「変えない」と同じに扱う必要がある(調査 2)ので、
   型を分けても挙動は変わらない。区別が要るのは診断だけで、`DefaultSeed` の理由で足りる。
4. **上書きの基準を下げる。** `DefaultSeed` は観測ではない。観測(IMM・TSF)は従来どおり上書きする。証拠源(ADR-261 の字形、ADR-264 のメニュー)が belief へ配線されるときは、`DefaultSeed` を
   「観測済みのモード」より低い基準(1 回の一致)で上書きしてよい。この配線は各 ADR の段階 2 で、本 ADR では行わない。
5. **テスト。** `closed_loop_scenarios.rs` の `seeded_gap` の判定を、`never_open_before`(間接的な条件)から、`input_mode == AssumedRomaji{DefaultSeed}` に置き換える。件数 32 は変わらない見込み(挙動不変)。
   `EXPECTED_SEEDED_GAP` は「種が外れる件数」のまま固定する。ADR-261 / 264 の配線で下がるはずなので、そのとき定数を直す。
   (表の予測が開く前に理由を `KeyEffectPrediction` へ上書きする場合があれば、判定に `never_open_before` を併用する。件数が 32 のまま保たれることを実装時に確かめる。)
6. **診断。** 不具合報告の belief の出力に、入力モードの出所(`DefaultSeed` を含む)を出す(要確認: 現在の出力に理由が含まれるか)。
7. **実害の測定(受動)。** 受け取った不具合報告の journal で、`DefaultSeed` が観測・証拠源に後から覆された件数を数える。覆された件数が多ければ、種の既定(かな)を見直す根拠になる。
   能動の測定は、ADR-261 / 264 の証拠源が shadow で入った後、種と証拠源の不一致率を見る。

## 未決・要確認

- **フォーカス移動で前の窓のモードが残る件**(BUG-204 の関連の疑い): `ImeModel::reduce_focus_changed` は `key_track`・`key_effect`・`applied` を消すが `input_mode` を消さない。ADR-191(状態の範囲)決定1は
  「同じスレッドの belief を引き継ぐ以上の精緻化は要らない」とする。別スレッドの窓へ移ったときも前のモードを残してよいかは未確認。`ObservedEisu` は `eisu_recovery` が扱う。
  他のモード(かな・ローマ字)が残るのは、この ADR の種と同じ性質の推測で、`Unknown` に戻すと Engine が止まる(調査 2)ので、戻す案は却下になる見込み。**再現してから決める。**
- 診断の出力の現状(決定 6)。
- `DefaultSeed` を足したときの網羅的な `match`(`AssumedReason`)の更新範囲(`snapshot_input_mode.rs`・`ime_model.rs`・`eisu_recovery.rs`・`conv_classify.rs`・`platform_state.rs` ほか)。

## 影響とリスク

- 挙動は変えない(新しい理由を足すだけ)ので、Engine の有効化・既存のゴールデンは変わらないはず。`AssumedReason` の `match` が網羅的な箇所は更新が要る。journal / 不具合報告の形式が変わる場合は互換を確認する。
- 本 ADR は BUG-204 の食い違いそのものは**直さない**。直るのは、証拠源が入ったとき(ADR-261 / 264)で、本 ADR はそのための受け口と測定の道筋を作る。この限界は意図したもの(却下した A〜D は、より多い場合を悪くする)。

## 実装タスク

- T1: `AssumedReason::DefaultSeed` の追加と、種の経路の分離(`seed_mode()`)。`is_romaji_capable` の更新。`match` の網羅の更新。
- T2: `closed_loop_scenarios.rs` の `seeded_gap` の判定の置き換え(決定 5)。
- T3: 診断(決定 6)の確認と、必要なら出力の追加。
- T4: ADR-261 / 264 の配線時に、`DefaultSeed` の上書きの基準(決定 4)を反映する(各 ADR のタスク)。
- T5: 受動の測定(決定 7)の手順を `docs/` に書く。
