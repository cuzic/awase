# ADR-093: IME 専用ホットキーの受信を `is_japanese_ime()` の即時真更新トリガーにする

## ステータス

**実装済み（2026-08-15、Opusコードレビューでの指摘を反映して修正済み）。**
`crates/awase-windows/src/vk.rs::is_synthetic_dbe_ime_hotkey`
（0xF0-0xF4 の5 VK を判定する純粋関数、ユニットテスト4件付き）を追加し、
`runtime/key_pipeline.rs::kp_stage_shadow_ime_toggle` の冒頭で
`self.platform_state.ime.set_is_japanese_ime(true)` を呼ぶよう配線した。
決定した通り、`false` へのダウングレードには一切関与しない（既存の
`apply_focus_probe` 内の probe ベース downgrade 経路は無変更）。

**当初案からの訂正（Opusコードレビュー指摘）**: 当初は BUG-14 の注入
イベント除外チェックより前に無条件で置き、注入イベントにも upgrade を
適用する設計だった（「BUG-14 が問題視したのは注入イベントをユーザー意図
に昇格させることであり、IME の存在を示す証拠として扱うことではない」
という理屈）。しかしレビューで、`is_japanese_ime()` は
`is_eligible_for_ime_force_on()`（`state/platform_state.rs:599`、
`is_japanese_ime() && effective_open()`、force-ON actuation ゲート）
を含む約10箇所の消費者を持つグローバルな belief であり、注入イベント
（外部プロセスの SendInput）を信頼してこの belief を actuation の根拠に
昇格させると、BUG-14 と同じ「注入イベントを過度に信頼する」失敗の
別ルートでの再発になりうると指摘された。**この upgrade は `!event.injected`
（物理キー入力のみ）に限定するよう修正した。** ADR本文の決定節・リスク節の
「この5VKの受信」は物理受信を指すものと読み替えること。

`tests/architecture_guard.rs` の `set_is_japanese_ime` 呼び出し件数固定
テストは存在せず、リスク節が懸念していた影響は無かった
（`cargo test -p awase-windows --lib`: 394件パス、`--test
architecture_guard`: 34件パス、`--test golden_scenarios`: 22件パス、
いずれも回帰なし）。実機（dragonflyg4）での grace 期間中の実際の
false 誤答訂正確認は未実施。

以下は実装着手前（2026-08-15設計時点）の記述。ADR-092（外部ソース由来のキー意味論の吸収）決定A-5の
検討過程で見つかった、別軸の小さな穴を切り出した ADR。ADR-092 の主題（レジストリ/
config1.db という**外部宣言**の吸収）とは異なり、本 ADR は awase 内部の
`is_japanese_ime()` という**確率的信念（belief）の精度不足**を扱う。

## 背景

### ADR-092 決定A-5 の検討で見つかった経緯

ADR-092 の round4 で「ひらがなキーのような IME ON になる副作用のあるキーも
観測対象に含めてほしい」という要求があり、`VK_DBE_HIRAGANA`/`KATAKANA`/
`ALPHANUMERIC`/`DBCSCHAR`/`SBCSCHAR` の5 VK を `ImeDetectConfig`
（`SyncKey` witness）に追加する案を出した。2巡目レビューで、この5 VK の
観測は既に `crates/awase-windows/src/vk.rs:118-147`
（`ImeKeyKind::from_vk`/`shadow_effect()`）で実装済みであり、
`hook.rs:121` → `IntentWitness::from_physical()` →
`UserIntentSource::PhysicalImeKey` という既存経路が既にこれを担っている
と判明した。`ImeDetectConfig` へ重複登録すると、`key_pipeline.rs:803-812`
の優先順位規則により `SyncKey` は `is_japanese_ime()` ゲートを通らず、
かつ BUG-51 追補3の規則下で「安全弁」より優先されるため、安全性が後退
する。この指摘を受け ADR-092 決定A-5 は「既に実装済み、追加のコード
変更は不要」に縮小・撤回された。

**しかしこの整理の過程で、既存の `PhysicalImeKey` 経路自体に見落として
いた弱点があると判明した。**

### `is_japanese_ime()` ゲートには既知の false negative がある

`key_pipeline.rs:802` のコメント「同期キー (config sync_direction) >
物理 KANJI (Japanese 限定) の順で意図を採用する」の通り、
`PhysicalImeKey`（`shadow_action` 由来）は `is_japanese_ime()` が
`true` の場合のみ意図として採用される（`key_pipeline.rs:805`）。

`is_japanese_ime()` 自体は probe ベースの確率的な信念であり、
`key_pipeline.rs:1940-1943` に次の既知の弱点が明記されている:

```rust
// スリープ復帰後など grace 期間中は read_ime_state_fast が一時的に
// is_japanese_ime=false を返すことがある。
// false へのダウングレードは grace active 中は行わない（true はいつでも更新）。
if probe.is_japanese_ime || !signals.any() {
    self.platform_state.ime.set_is_japanese_ime(probe.is_japanese_ime);
}
```

つまり **スリープ復帰直後・フォーカス変更直後の grace 期間中、
`is_japanese_ime()` が一時的に `false` を誤答しうる**ことが既に
コードで認識されており、「`false` へのダウングレードは grace 中は
行わない（`true` はいつでも即時反映）」という非対称な信頼パターンで
部分的に緩和されている。

**この grace 期間中に `VK_DBE_HIRAGANA` 等のIMEホットキーが実際に
押されると、`PhysicalImeKey` 経路は `is_japanese_ime()` ゲートで
observation を黙って捨てる。** これは「キーの意味が不確実だから」
ではなく、**キーの意味は確実なのに、それを採用するための前提条件
チェックの方が一時的に不正確**という非対称な状況である。

### この5 VK は「意味が確実」なだけでなく「存在自体が証拠」でもある

`VK_DBE_HIRAGANA`/`KATAKANA`/`ALPHANUMERIC`/`DBCSCHAR`/`SBCSCHAR`
（`0xF0`-`0xF4`）は、通常の物理キーボードには存在しない IME 専用の
合成 VK コードである。awase のフックにこの VK コードの `WM_KEYDOWN`
が届くこと自体、**何らかの IME がそのキーを処理・報告しているという
事実**であり、`shadow_effect()`（VK→TurnOn/TurnOff の対応）だけで
なく、「IME が存在する」こと自体の証拠にもなっている。

（検討初期段階では、この VK 空間が中国語/韓国語 IME 等 CJK 全般で
共有されているため「日本語 IME の存在証拠として使うのは早計ではないか」
という懸念を挙げたが、**本プロジェクトの対象スコープ外**であり
考慮不要と判断した。）

## 決定

**この5 VK の受信を、`is_japanese_ime()` の即時 `true` 更新トリガーに
追加する。** `key_pipeline.rs:1940-1943` の既存パターン（`true` への
更新はいつでも許す、`false` へのダウングレードのみ grace 中に抑制する）
をそのまま踏襲し、新しい概念は持ち込まない:

- `may_change_ime`/`ImeKeyKind::from_vk` がこの5 VK のいずれかを認識
  した時点で、`probe.is_japanese_ime` の値によらず
  `set_is_japanese_ime(true)` を即座に呼ぶ。
- **`false` へのダウングレードには一切関与しない**——この5 VK が
  「来ない」ことは「日本語 IME でない」ことの証拠にはならない
  （単に IME モードを切り替えていないだけの可能性の方が高い）。
  既存の probe ベースの downgrade 経路（grace 期間中は抑制）は
  そのまま維持する。
- ADR-092 の `PhysicalImeKey`/`shadow_action` 経路自体は変更しない
  ——`is_japanese_ime()` ゲートを迂回するのではなく、ゲートが参照する
  値の精度を上げるだけであり、`SyncKey` の優先順位昇格リスク
  （BUG-51 追補3）を一切引き込まない。

## リスク

- **grace 期間中に非日本語 IME 環境でこの5 VK が偶然届いた場合の
  誤 upgrade。** ただし前述の通りこの VK 空間自体が IME 専用の合成
  コードであり、通常の物理キー入力からは発生しない。CJK 他言語 IME
  由来の可能性は本プロジェクトのスコープ外として考慮しない（背景節
  参照）。
- `set_is_japanese_ime(true)` の呼び出し箇所が増えることで、
  既存の `tests/architecture_guard.rs` 等の件数固定テストに影響する
  可能性がある——実装時に確認すること。

## 適用範囲・関連ルール

対象ファイル（`crates/awase-windows/src/runtime/key_pipeline.rs`、
`crates/awase-windows/src/vk.rs`）は `.claude/rules/fix-requires-evidence.md`
の「IME belief」再発ファミリーに該当する。実装時は回帰テスト
（`src/engine/tests.rs` 等、Linux 実行可能なもの優先）または
`docs/known-bugs.md` への記録のいずれかを同じコミットに含めること。

## 自己評価

**妥当性スコア: 7/10**。既存パターン（grace 中の非対称信頼）の
延長でしかなく新しい判断ロジックを持ち込まない点、`CharsetSlot` の
轍（awase が独自に belief を判断する）にも該当しない点（判定材料は
VK コードという外部から届く事実そのもの）は堅実。一方で:

- 実装未着手のため、`architecture_guard.rs` 等の件数固定テストへの
  実際の影響は未検証。
- `set_is_japanese_ime` の呼び出し箇所を増やすことが、他の
  grace/stale 系ロジック（`key_pipeline.rs` 周辺の他の分岐）に
  予期しない副作用を与えないか、実装時にコードパスを再確認する
  必要がある。

## 関連

- [ADR-092](092-external-key-semantics-absorption-and-thumb-key-restructure.md)
  決定A-5（本 ADR の発端、`vk.rs::ImeKeyKind`/`PhysicalImeKey` 経路の記述）
- `.claude/rules/ime-belief-architecture.md`（`InputModeObserved`/
  belief 更新の設計原則）
- `.claude/rules/fix-requires-evidence.md`（IME belief 再発ファミリーの
  テスト/記録義務）
