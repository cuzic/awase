# ADR-039: TSF_OBS アクセス制御の 5 フェーズ段階的強化

## ステータス

採用済み（2026-05-24）

## コンテキスト

TSF 観測値（`TSF_OBS` static）は observation（観測）と judgement（判断）の境界が曖昧で、以下の問題が発生していた：

1. `tsf_observations.rs` と `output.rs` が `TSF_OBS` に直接アクセスし、観測値を「上書き」するコードが散在していた
2. ある atomic グローバルの書き込み元と読み取り元がファイルをまたいで分散し、不変条件の追跡が困難
3. Observer が判断層の値を上書きするため、「intent を設定したのに observation が intent を破壊する」バグが繰り返し発生した

## 決定

TSF_OBS へのアクセスを 5 フェーズで段階的に制限する。

**Phase 1:** `TSF_OBS` を `pub(in crate::tsf)` に格下げ（`tsf/` 外からの直接アクセスを禁止）

**Phase 2:** `aggregator` モジュールを削除し、`tsf_obs()` accessor に集約。全ての外部アクセスは named API 経由（`gji_last_io_ms()`, `namechange_baseline()` 等）

**Phase 3:** `FocusProbeSnapshot` を導入し、`key_pipeline` の `TSF_OBS` 直読を廃止

**Phase 4:** `ImeApplyContext` → `ImeObservationSnapshot` にリネームし、「観測値のスナップショット」であることを型名で表現

**Phase 5:** 禁止ゾーンを `layer-boundaries.md` の B-2, B-3 ルールとして明文化し、grep audit コマンドを定義

```sh
# B-3 audit: TSF_OBS は tsf/ 内のみ
grep -rn "TSF_OBS" crates/awase-windows/src/
# 期待: tsf/ モジュール内のみ
```

## なぜ段階的か

一度に全アクセスを制限すると、既存の依存関係が多すぎてコンパイルエラーが大量発生する。各フェーズを小さく保ち「このフェーズ完了後もテストが通る」ことを確認しながら進めた。

## 結果

- TSF_OBS への書き込みが `tsf/` モジュール内に封じ込められた
- 「Observer が intent を破壊する」バグが構造的に発生不可能になった
- ADR-032 の C-2 ルール（Observer は ObserverReported 経由のみ）の基盤になった

## 関連 ADR

- ADR-030 (TSF 3 層分離)
- ADR-032 (IME 状態 reducer 4 階層)
