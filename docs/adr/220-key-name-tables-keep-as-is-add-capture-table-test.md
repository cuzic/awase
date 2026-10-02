---
id: ADR-220
title: |-
  キー名対応表の単一ソース化は見送り、設定 GUI のキャプチャ表を from_name で検証するテスト 1 本だけ足す
summary: |-
  当初案「キー名の対応表を KEY_NAMES 1 つに集める」は Opus round1 で、(1) `awase-vkmap` はルート crate に依存するため、ルートから参照すると循環で置き場所が成立しない(`KEY_IDENTITY_ALIASES` の導出は不可能)、
  (2) `LEGACY` を生成にすると凍結オラクルが自己比較になり、表の誤りを検出できなくなる、(3) 行数は純増、(4) この ADR が防ぐ型の同期漏れの実害は確認できない(issue #99 は別の型)、(5) match が持つ重複名のコンパイル時検査が消える、と指摘され取り下げた。
  既存の「手動同期 + 検出テスト」は `core_key_identity_covers_from_name`・`key_acceptance_tests.rs`(約 300 件)・gji-config の検査でほぼ全域が揃っている。残る穴は `egui_key_to_internal`(設定 GUI のキャプチャ表)の出力が `from_name` で解決できるか未検査な点だけで、テスト数行で塞ぐ。
status: |-
  見送り(2026-10-02)、テスト 1 本の追加のみ提案。Opus round1 で方針転換。round2 の再確認待ち。
related_adr:
  - "ADR-019"
  - "ADR-161"
  - "ADR-201"
---

# ADR-220: キー名対応表は現状維持し、キャプチャ表の検証テストだけ足す

## 背景

キー名 → VK の情報は `vk.rs::from_name`(match、111 腕)、`vk.rs` の `LEGACY` 回帰表、`awase-settings` の `KEYMAP_MAIN_KEYS` ほかの候補表、`key_text.rs::KEY_IDENTITY_ALIASES` などに分かれている。
当初は 1 つの `KEY_NAMES` 宣言に集める案を出したが、Opus round1(`opus-review-adr220-round1.md`、実コードで確認)で次が分かった。

- **置き場所が成立しない**: `awase-vkmap` は `awase`(ルート)に依存している(`crates/awase-vkmap/Cargo.toml:14`)。ルートから vkmap は参照できない。`KEY_IDENTITY_ALIASES` を導く部分は作れない。
  `awase-settings` は既に `awase-windows` に依存しているので、置くなら最小は `vk.rs`。vkmap に置くと VK 値の正本が `vk.rs` と vkmap の 2 か所に分かれ(layer-boundaries D-1)、別リポジトリ `awaza` と共有する公開面も広がる。
- **`LEGACY` は凍結した旧表(オラクル)**: 書き直し前の `from_name` が受理していた名前の記録で、矢印キーなどは含まない。`KEY_NAMES` から生成すると表どうしの比較になり、行の削除や VK 値の打ち間違いが検出されなくなる。
- **行数は純増**: `LEGACY` を残す以上削れず、`KEY_NAMES` の各行は match の腕より長く、構造体定義・検索関数・一意性テストが加わる。
- **防ぎたい実害が確認できない**: issue #99 は設定 GUI が `from_name` を通さず文字列比較していた問題で、表の同期漏れではない。`from_name` に関わる最近の変更(`34a630e3`/`922674d8`/`d8c13e4e`)にも「A に足して B に足し忘れた」型は無い。
- **検出テストは既にほぼ揃っている**: `core_key_identity_covers_from_name`(`vk.rs`)、`key_acceptance_tests.rs`(`awase-settings`、候補表の全内部名 300 件超を実際の読み手に通す)、`key_effect_predictor.rs` の gji-config 名の検査。
- **match の重複名検査が消える**: 同じ文字列が 2 つの腕に現れると `unreachable_patterns` で CI が落ちる。配列や HashMap は先勝ち/後勝ちで黙って通る。

## 決定

### D1: 表の単一ソース化は行わない

`from_name`・`LEGACY`・`KEY_IDENTITY_ALIASES`・各候補表は現状のまま。ADR-161 の「宣言から生成」の精神は、テストの期待値まで同じ仕様から作るとオラクルでなくなる点で、このデータには合わない。

### D2: 実在する穴を埋めるテストを 1 本足す

`egui_key_to_internal`(`crates/awase-settings/src/main.rs:5555` 付近、71 腕)の出力が `from_name` で解決できることを確かめるテストが無い。失敗シナリオ: キャプチャ表に `from_name` が知らない名前(`"VK_PGUP"` 等)を書くと、GUI でキーを押して割り当てた設定が実行時に無言で無視される(BUG-167 と同型)。
`key_acceptance_tests.rs` に、`egui::Key::ALL` を回して `egui_key_to_internal(k)` が `Some(name)` のとき `VkCode::from_name(name)` が `Some(_)` であることを確かめるテストを足す(数行)。このテストは `windows-settings` ジョブでのみ実行される(同ファイル冒頭の doc)ので、ローカルでは `cargo test -p awase-settings` を明示的に回して確かめる。

### D3: 再挑戦条件

「キー名を足して別の表を足し忘れた」実害が複数回記録された場合に、名前と別名だけの表をルート crate に置く案(`key_text.rs` に別名 → 正規名の表。VK 値を持たない)を検討する。その際、`key_identity` の対象を全別名に広げると `AppConfig::validate` の判定が変わる点(ADR-201 が組を「コアが意味を問うキー」に限った理由)を先に確かめる。

## 検討した代替案

- **`KEY_NAMES` 単一ソース化(当初案)**: 上の理由で取り下げ。
- **名前と別名だけをルート crate に集める**: 別名は 12 個程度で便益が小さい。D3 の再挑戦条件で扱う。
- **`awase-vkmap` に置く**: 循環と、正本が 2 か所に分かれる問題で最も悪い。
- **`macro_rules!` で match と配列を同時に吐く**: 退けた DSL に近づく。

## 影響

テスト 1 本の追加のみ。本番コードに影響しない。
