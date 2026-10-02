---
id: ADR-220
title: |-
  キー名対応表の単一ソース化は見送り、設定 GUI のキャプチャ表を from_name で検証するテスト 1 本だけ足す
summary: |-
  当初案「キー名の対応表を KEY_NAMES 1 つに集める」は Opus round1 で、(1) `awase-vkmap` はルート crate に依存するため、ルートから参照すると循環で置き場所が成立しない(`KEY_IDENTITY_ALIASES` の導出は不可能)、
  (2) `LEGACY` を生成にすると凍結オラクルが自己比較になり、表の誤りを検出できなくなる、(3) 行数は純増、(4) この ADR が防ぐ型の同期漏れの実害は確認できない(issue #99 は別の型)、(5) match が持つ重複名のコンパイル時検査が消える、と指摘され取り下げた。
  既存の「手動同期 + 検出テスト」は `core_key_identity_covers_from_name`・`key_acceptance_tests.rs`(300 件超)・gji-config の検査でほぼ全域が揃っている。残る穴は `egui_key_to_internal`(設定 GUI のキャプチャ表)の出力が `from_name` で解決できるか未検査な点だけで、テスト数行で塞ぐ。
status: |-
  見送り(2026-10-02)、テスト 1 本の追加のみ提案。Opus round3 で収束(Must なし、新規指摘なし)。D2 のテスト(`egui_capture_names_are_accepted_by_their_readers`)は実装済み。表の単一ソース化は見送り。
related_adr:
  - "ADR-019"
  - "ADR-161"
  - "ADR-201"
---

# ADR-220: キー名対応表は現状維持し、キャプチャ表の検証テストだけ足す

## 背景

キー名 → VK の情報は `vk.rs::from_name`(match、111 腕)、`vk.rs` の `LEGACY` 回帰表、`awase-settings` の `KEYMAP_MAIN_KEYS` ほかの候補表、`key_text.rs::KEY_IDENTITY_ALIASES` などに分かれている。
当初は 1 つの `KEY_NAMES` 宣言に集める案を出したが、Opus round1(2026-10-02、実コードで確認)で次が分かった。

- **置き場所が成立しない**: `awase-vkmap` は `awase`(ルート)に依存している(`crates/awase-vkmap/Cargo.toml:14`)。ルートから vkmap は参照できない。`KEY_IDENTITY_ALIASES` を導く部分は作れない。
  `awase-settings` は既に `awase-windows` に依存しているので、置くなら最小は `vk.rs`。vkmap に置くと VK 値の正本が `vk.rs` と vkmap の 2 か所に分かれ(layer-boundaries D-1)、別リポジトリ `awaza` と共有する公開面も広がる。
- **`LEGACY` は凍結した旧表(オラクル)**: 書き直し前の `from_name` が受理していた名前の記録で、矢印キーなどは含まない。`KEY_NAMES` から生成すると表どうしの比較になり、行の削除や VK 値の打ち間違いが検出されなくなる。
- **行数は純増**: `LEGACY` を残す以上削れず、`KEY_NAMES` の各行は match の腕より長く、構造体定義・検索関数・一意性テストが加わる。
- **防ぎたい実害が確認できない**: issue #99 は設定 GUI が `from_name` を通さず文字列比較していた問題で、表の同期漏れではない。`from_name` に関わる最近の変更(`34a630e3`/`922674d8`/`d8c13e4e`)にも「A に足して B に足し忘れた」型は無い。
- **検出テストは既にほぼ揃っている**: `core_key_identity_covers_from_name`(`vk.rs`)、`key_acceptance_tests.rs`(`awase-settings`、候補表の全内部名 300 件超を実際の読み手に通す。`combo_accepts` は `format_combo` → `parse_key_combo` を 8 通りの修飾キーで検査)、`key_effect_predictor.rs` の gji-config 名の検査。
- **match の重複名検査が消える**: 同じ文字列が 2 つの腕に現れると `unreachable_patterns` で CI が落ちる。配列や HashMap は先勝ち/後勝ちで黙って通る。

## 決定

### D1: 表の単一ソース化は行わない

`from_name`・`LEGACY`・`KEY_IDENTITY_ALIASES`・各候補表は現状のまま。ADR-161 の「宣言から生成」の精神は、テストの期待値まで同じ仕様から作るとオラクルでなくなる点で、このデータには合わない。

### D2: 実在する穴を埋めるテストを 1 本足す

`egui_key_to_internal`(`crates/awase-settings/src/main.rs:5555`、71 腕、`fn(egui::Key) -> Option<&'static str>`)の出力が実際の読み手で解決できることを確かめるテストが無い。
失敗シナリオ: キャプチャ表に `from_name` が知らない名前(`"VK_PGUP"` 等)を書くと、設定 GUI では割り当てが成功したように見える(`keymap_forbidden_reason` は解決できない名前を「禁止しない」として扱う、`main.rs:5311-5328`)。
実行時には `to` 側は起動診断に警告(`keymap.rs:129-136` → `bootstrap.rs:1275-1278`)を出してそのルールを捨てる。無言ではないが、GUI 上は成功に見え、ルールは効かない(BUG-167 と似た型。ただし BUG-167 は警告も無かったので同型ではない)。

`key_acceptance_tests.rs`(`egui` は `super::egui` で使え、非公開関数も子モジュールなので呼べる)に、`egui::Key::ALL`(egui 0.31.1、`key.rs:197`)を回して次を確かめるテストを足す。

- `egui_key_to_internal(k)` が `Some(name)` のとき、`VkCode::from_name(name)` が `Some(_)` で、既存の `combo_accepts(name)`(`from` 側の読み手 `parse_key_combo` 経由)も通る。
- `Some` を返したキーの件数が下限(60 件超)を超える(egui の更新でキー名が変わり `_ => None` に落ち続けても、検査が空振りしないため)。既存の `checked > 300` と同じ考え方。

このテストは `windows-settings` ジョブ(`cargo nextest run -p awase-settings`)でのみ CI 実行される。ローカルでは `cargo test -p awase-settings` を明示的に回す。

### D2': 範囲外として記録するもの(足さない)

- `pub const VK_*` と `from_name` の一致(今は `VK_LEFT`/`VK_UP` の 2 件を `vk.rs:1529-1530` で検査するだけ)を全定数に広げるテスト: 数行で書けるが、同期漏れの実害が確認できていないので足さない。必要になったら足す。
- `awase-macos/src/vk.rs::key_name_to_keycode` は `canonical_key_text` を通していない(ADR-201 の寛容化が macOS に届いていない)。macOS は stub のため本 ADR の範囲外。次に名前表を見る人のための既知事項として残す。

### D3: 再挑戦条件

次のどちらかが起きたときに、名前と別名だけの表をルート crate に置く案(`key_text.rs` に別名 → 正規名の表。VK 値を持たない)を検討する。

- `docs/known-bugs/` に、根本原因が「キー名表の片方への追加漏れ」である BUG が 2 件記録されたとき(起票時に本 ADR へ逆リンクを書く)。
- ADR-199 で F13〜F24 や新しい IME キーを候補に足すとき、触った表の数が 4 か所以上になったとき。

検討時は、`key_identity` の対象を全別名に広げると `AppConfig::validate` の判定が変わる点(ADR-201 が組を「コアが意味を問うキー」に限った理由)を先に確かめる。

## 検討した代替案

- **`KEY_NAMES` 単一ソース化(当初案)**: 上の理由で取り下げ。
- **名前と別名だけをルート crate に集める**: 別名は 15 個で便益が小さい。D3 の再挑戦条件で扱う。
- **`awase-vkmap` に置く**: 循環と、正本が 2 か所に分かれる問題で最も悪い。
- **`macro_rules!` で match と配列を同時に吐く**: 退けた DSL に近づく。

## 影響

テスト 1 本の追加のみ(D2)。本番コードに影響しない。

## 追記(2026-10-02): vk.rs 内の定数と from_name の統合、LEGACY の撤去

本 ADR の議論の後、所有者の判断で `vk.rs` の中だけで閉じる統合を実施した。D1(`KEY_NAMES` による crate をまたぐ単一ソース化)は引き続き見送りで、これは別物。

- **やったこと**: `pub const VK_*`(47 個)と `from_name` の match(111 腕、うち 43 腕が定数と同じ値を 16 進で再掲していた)を、`vk_keys!` マクロ 1 か所に統合した。識別子 `VK_KANA` から定数と正規名 `"KANA"` を作り、別名だけを `[...]` で書く。16 進の VK 値を書く場所が 1 か所になった。
- **挙動は変えていない**: 置き換え前後で名前→VK の対応 126 個が一致することを、旧 match の文字列リテラルと新マクロ呼び出しを別々に抽出して機械的に比べて確認した。
- **失ったもの**: match が持っていた重複名のコンパイル時検査(S4)。`key_table_names_are_unique_and_canonical` が名前の一意性、正規名・別名が `canonical_key_text` の出力と同じ形であること、VK 値の一意性を見る。
- **LEGACY の撤去**: 上の機械的な同値確認を済ませたうえで、`from_name_resolves_every_legacy_name_to_the_same_vk`(123 名の凍結表)を削除した。round1 M1 が指摘した「独立したオラクル」を失う判断で、表の VK 値の打ち間違いは、候補表の受理テスト(`key_acceptance_tests.rs`)・矢印キーのテスト・設定の解決テストの範囲外のキー(`F1`〜`F10` の一部、OEM 系など)では検出されなくなる。16 進を書く場所が 1 か所になったので重複由来のずれは起きないが、値そのものの誤りは検出できない。許容した。
- **残した検査**: `LEGACY` が兼ねていた「`VK_` 付き・小文字・`VK_` 無しのどれでも同じ VK に解決される」は `key_table_names_resolve_in_every_spelling` で表を回して見る(期待値は表自身から取る)。
- **範囲外のまま**: `awase-settings` の `KEYMAP_MAIN_KEYS`・`egui_key_to_internal`、`key_text.rs::KEY_IDENTITY_ALIASES`、macOS の名前表。
