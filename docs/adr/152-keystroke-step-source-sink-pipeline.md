# ADR-152: 打鍵を source → sink のパイプラインとして再構成する構想

## ステータス

**保留中構想（未実装）。[ADR-151](151-actuation-delegate-by-default-drift-scoped-to-ownership.md)
と隣接するが方向は独立。ADR-151 の force-ON 救済 Blocker により着手できない
まま。**

## 重要な注記: この ADR の本文は大部分が失われている（2026-09-08 追記）

このADR番号（152）は index.md 上では以前から存在し、
[ADR-153](153-gji-keymap-aware-safe-vk-substitution-for-mode-keys.md)
「ADR-151/152 との違い」節や [ADR-156](156-unify-deferred-execution-queues.md)
「代替案B」節が「ADR-152 決定3」として具体的な設計内容を引用してきたが、
**本文ファイル自体はどのコミットにも一度も存在しなかった**
（`git log --all --follow -- docs/adr/152-*.md` が 0 件、対応するコード
`KeyStrokeStepDispatcher`/`StepOwnership` も `grep -rn` でリポジトリ全体
0件、2026-09-08 に確認）。

**index.md の1行要約と、他 ADR からの断片的な引用だけを頼りに、以下の
「わかっていること」節のみを本文として復元した。決定1・決定2の内容、
「決定3」の全文、検討した代替案、pre-mortem の経緯などは一切現存せず、
再現できない。** 次の担当者がこの構想に着手する場合、以下の断片を
出発点に**設計をやり直す**必要がある——「ADR-152 に書いてあったはず」
という前提で実装を進めないこと。

## わかっていること（他ADRからの引用の寄せ集め）

- **狙い**: 打鍵1回を「source（意図の発生源）」→「sink（実際の配送/
  actuation 先）」のパイプラインとして再構成する構想（index.md より）。
- **決定3の内容（[ADR-156](156-unify-deferred-execution-queues.md)
  「代替案B」節からの引用）**:
  - `KeyStrokeStepDispatcher::dispatch` を `RomajiOutput` sink の実行窓口
    にする。
  - `pending_deferred`（`output/tsf_warmup_coord.rs`/`output/vk_send.rs`、
    TSF probe/recovery 中に確定できない VK を退避する既存キュー）は
    まさにこの `RomajiOutput`（`DeferredVk`）を退避するキューであり、
    ADR-152 が実装されれば `pending_deferred` の defer/drain は
    `execute_sink` の内部に取り込まれ、解放条件は `StepOwnership::
    resolve()` の一部として再設計される見込み。
  - `transport.rs::PhysicalKeyDisposition::plan`（`INPUT_DEFER` への
    退避判断とも接する既存関数）も `StepOwnership::resolve()` に
    寄せる、と書かれていた。
  - ADR-156 はこの関係を「直交・独立」ではなく「（ADR-156 のような）
    後から着手する側が、ADR-152 という先行実装の型を作り直す責任を
    負う」関係と評している。
- **[ADR-153](153-gji-keymap-aware-safe-vk-substitution-for-mode-keys.md)
  「ADR-151/152 との違い」節からの引用**: ADR-151/152 はいずれも
  「（delegate 対象キーには）何も送らない」方向を指すものとして
  ADR-153 に対比されている——つまり本 ADR の source→sink モデルは、
  ある種の sink を「actuate せず belief 追随のみ」にできる設計を
  含んでいたと推測される（未確認、推測の域を出ない）。
- **Blocker（[ADR-151](151-actuation-delegate-by-default-drift-scoped-to-ownership.md)
  と共有、[ADR-156](156-unify-deferred-execution-queues.md)より）**:
  「Applied を詐称すると TsfNative 唯一の ON 方向救済機構
  `apply_force_on_for_imm_broken` が構造的に永久停止する」という
  ADR-151 の Blocker を ADR-152 も共有しており、着手できないままである
  と ADR-156 が明記している。ADR-151 の「再検討の条件」（2点、
  ADR-151 本文参照）が揃わない限り、本 ADR も同じ壁に当たる。

## 決定

**未定。** 上記の断片以上の情報が現存しないため、本ADRは「決定」を
下せる状態にない。次に着手する際は、[ADR-151](151-actuation-delegate-by-default-drift-scoped-to-ownership.md)
の Blocker 解消を前提条件としつつ、`KeyStrokeStepDispatcher`/
`StepOwnership` の型設計を新規に（このADRの過去の決定を前提にせず）
やり直すこと。

## 関連

[ADR-151](151-actuation-delegate-by-default-drift-scoped-to-ownership.md)
（同じ Blocker を共有、方向は隣接だが独立）、
[ADR-153](153-gji-keymap-aware-safe-vk-substitution-for-mode-keys.md)
（方向性の対比で言及）、
[ADR-156](156-unify-deferred-execution-queues.md)（`pending_deferred`
との重複領域、「代替案B」として不採用の理由づけの中で本ADRの決定3を
引用）。
