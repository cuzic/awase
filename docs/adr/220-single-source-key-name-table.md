---
id: ADR-220
title: |-
  キー名の対応表を 1 つの宣言に集め、from_name・設定 UI の候補・テストの期待表を生成する
summary: |-
  キー名と VK 値の対応が、`vk.rs::from_name` の match(`=> Some(Self(0x..))` が 111 行)、同ファイルのテスト `from_name_resolves_every_legacy_name_to_the_same_vk` の `LEGACY` 表(手で複製)、
  `awase-settings` の `KEYMAP_MAIN_KEYS`(約 110 行)・`THUMB_KEY_OPTIONS`、`key_text.rs::KEY_IDENTITY_ALIASES`、`vk.rs` の `pub const VK_*` に分散している。
  1 つの宣言(`(正規名, 別名..., VK 値, 表示ラベル?, 設定 UI に出すか)`)から、`from_name` と `LEGACY` 相当の期待表を生成する。
  設定 UI の候補一覧は生成しない(UI 都合の順序・ラベル・除外条件が独立しているため)。
  awase-vkmap を宣言の置き場所の第一候補にし、ルート crate の「VK マジックナンバーを持たない」規則(ADR-019)を破らない形を先に確かめる。
status: |-
  提案(2026-10-02)。未レビュー。実装なし。実現可能性(crate 境界)は未確認。
related_adr:
  - "ADR-019"
  - "ADR-161"
  - "ADR-176"
  - "ADR-201"
---

# ADR-220: キー名の対応表の単一ソース化

## 背景

「キー名 → VK 値」を引く情報は、2026-10-02 時点で少なくとも次に分かれている。

| 場所 | 役割 | 規模 |
| --- | --- | --- |
| `crates/awase-windows/src/vk.rs::VkCodeExt::from_name` | 設定のキー名を解決する本体の `match` | `=> Some(Self(0x..))` が 111 行 |
| 同ファイル `from_name_resolves_every_legacy_name_to_the_same_vk` の `LEGACY` | 過去の名前が同じ VK に解決されることの回帰表 | `("VK_A", 0x41), ...` の手書きの複製 |
| `crates/awase-settings/src/main.rs::KEYMAP_MAIN_KEYS` ほか `*_OPTIONS` | 設定 UI のドロップダウン候補(表示ラベル, 内部名) | 約 110 行 + 親指キー候補など |
| `src/key_text.rs::KEY_IDENTITY_ALIASES` | 「変換」と `CONVERT` のような別名を同じ「組」に寄せる | 5 行。doc に「`from_name` に別名を足したらここにも足す。`core_key_identity_covers_from_name` が漏れを検出する」とある |
| `vk.rs` の `pub const VK_*: VkCode` | 定数 | 数十行 |

すでに「ここを足したらあちらにも足す。漏れは別のテストが拾う」という **手動同期 + 検出テスト** の形になっている箇所がある(`KEY_IDENTITY_ALIASES`)。
`THUMB_KEY_OPTIONS` の doc には、`"VK_NONCONVERT"` と `"無変換"` の表記不統一が原因の不具合(GitHub issue #99)が書かれている。
キー名の追加(F13〜F24 の追加、ADR-199 の議論)のたびに、複数箇所を手で揃える作業が発生する。

訂正: 前回の調査メモで候補に挙げた `tray.rs::KEY_ROWS` はトレイアイコン描画の座標表で、キー名表ではない。本 ADR の対象外。

## 決定

### D1: 宣言を 1 つにする(形式は Rust の const 配列、ビルド時生成はしない)

```rust
pub struct KeyNameEntry {
    pub vk: u16,
    pub canonical: &'static str,       // "A", "OEM_PLUS", "NONCONVERT"(canonical_key_text 済みの形)
    pub aliases: &'static [&'static str],   // "MUHENKAN", "無変換"
}
pub const KEY_NAMES: &[KeyNameEntry] = &[ ... ];
```

- `from_name` は `KEY_NAMES` を引く関数にする(`canonical_key_text` を通した名前で、`canonical` と `aliases` を照合)。
  実行時の検索は、起動時に 1 度 `HashMap` を作る、または `match` を `macro_rules!` で展開する(どちらでも挙動は同じ。採用は実装時に起動時間と `const` 評価の可否で決める)。
- `LEGACY` の期待表は、`KEY_NAMES` から `VK_` 付きの旧形式を **機械的に作って** 突き合わせるテストに置き換える。ただし「旧名が同じ VK に解決される」という回帰の意図(過去の名前を保つ)は、旧名のうち `VK_` 以外の形(`"Nonconvert"` 等)を手書きで残して守る。
- `KEY_IDENTITY_ALIASES` は `aliases` から導く(「かな」「カナ」→ `KANA` 等)。検出テストは、生成になれば要らなくなるので削除候補にする。ただし **どの別名を「同じ組」と見なすか**(ADR-201 で決めた検証上の意味)は宣言に `identity_group: Option<&str>` として残す。

### D2: 設定 UI の候補一覧は生成しない

`KEYMAP_MAIN_KEYS` / `THUMB_KEY_OPTIONS` は、(a) 表示ラベルが JIS 配列基準で、(b) 並び順が UI 都合、(c) 用途ごとに載せるキーを変える(親指キー候補には Alt を載せない、など)。
`KEY_NAMES` から「表示ラベル」だけを引く、あるいは **候補の内部名が `KEY_NAMES` に存在すること**をテストで固定するにとどめる。
一覧そのものを宣言から生成すると、用途ごとの除外条件を宣言側に持ち込むことになり、単一ソースのはずが条件の集まりになる。

### D3: 置き場所は先に確かめる(未確認の前提)

- ルート crate `awase` は VK のマジックナンバーを持たない(ADR-019、`docs/layer-boundaries.md` カテゴリ A–E)。`canonical_key_text` と `KEY_IDENTITY_ALIASES` はルート crate の `key_text.rs` にあり、`from_name` は `awase-windows` にある。
- `KEY_NAMES` を `crates/awase-vkmap`(現在 97 行、VK / スキャンコードの対応表)に置けば、`awase-windows` と `awase-settings` の両方から使える見込みだが、**`awase-vkmap` がルート crate から参照されているか、`awase-settings` が依存してよいかを、実装前に確認していない**。
- ルート crate が `KEY_NAMES` を直接参照できないなら、`identity_group` は別の経路(`awase-windows` から渡す)になる。この場合 D1 の `KEY_IDENTITY_ALIASES` 部分は本 ADR から外し、`from_name` と `LEGACY` だけを対象にする。

### D4: パイロットと中止基準

最初は **`from_name` と `LEGACY` の統合だけ**を行う(`KEY_IDENTITY_ALIASES` と設定 UI は触らない)。次のどれかで中止する。

- 置き場所(D3)で、ルート crate の規則(ADR-019)かクレート依存の向きを破らないと実現できない。
- 統合後に、`from_name` の既存テスト全件と `config_key_resolution_tests.rs`(ADR-201)が、**期待値を 1 つも書き換えずに**通らない。
- 起動時の `HashMap` 構築が起動経路に測定可能な遅れを足す(測るなら Windows 実機。ホストでは測れない)。`match` 展開で避けられるならこの基準は外れる。

## 期待する効果と、まだ測っていない点

- キー名を足す作業が `KEY_NAMES` の 1 行になる。`LEGACY` の複製が消える(約 100 行の削減。実測は実装時)。
- `core_key_identity_covers_from_name` のような「同期漏れ検出テスト」が要らなくなる(D3 が成立した場合)。
- **効果が出ない可能性**: キー名の追加は頻繁ではない(直近は F13〜F24 の議論)。同期漏れの実害は issue #99 の 1 件が確認できているだけで、他にあるかは調べていない。
  よって主な便益は「複製 100 行の除去」であり、DSL としての価値は小さい。行数の純減が見込めなければ、中止が妥当。

## 検討した代替案

- **何もしない**: 追加の頻度が低く、検出テストが漏れを拾っている。→ D4 の基準を満たせなければ、これが結論になる。
- **`build.rs` で TOML/JSON から生成する**: 非 Rust の人が表を編集できるが、ビルド手順が増える。`awase-build-support` の既存の仕組みに乗れるかは未確認。const 配列で足りるうちは採らない。
- **`strum` の derive で `VkName` enum を作る**: ADR-215 の方向と揃うが、`VkCode` が `u16` の newtype で、列挙できない値(OEM の 100 以上のコード、未知の VK)を扱うため、enum では表せない。
- **UI 候補も含めて全て生成する**: D2 の理由で採らない。

## 影響

- `from_name` を呼ぶ設定読み込み・ホットキー解決の全経路に関わる。挙動を変えないことが条件で、ADR-201 のテストが回帰ガードになる。
- `vk.rs` 内の `#[cfg(windows)]` の有無に注意する(CLAUDE.md の、Linux では存在しないテスト)。`from_name` 周辺のテストがホストで実行されているかを、移行前に `cargo test --list` で確認する。
