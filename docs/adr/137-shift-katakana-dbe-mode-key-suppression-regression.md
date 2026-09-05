# ADR-137: `VK_DBE_*` KeyDown 無条件 Suppress（BUG-52対策）が Shift+かな→カタカナ変換を巻き添えで殺している（BUG-116）

## ステータス

**調査完了・設計ドラフト（Opus敵対的レビュー未実施、未実装）。**

原因はコード読解と `git log`/`git show` によるコミット履歴の追跡で確定した
（実機ログでの再現確認はまだ行っていない）。決定案は1つに絞れているが、
IME actuation の合流点（`.claude/rules/fix-requires-evidence.md` の
「キー選択」再発ファミリー隣接領域）に触れる変更のため、実装前に
Opus 敵対的レビューと実機検証（BUG-52の repro が再発しないことの確認を含む）
を挟むことを推奨する。

## 問題

### 症状

JIS配列の「カタカナ ひらがな ローマ字」キー（scan 0x70）を Shift 併用で
押しても、GJI/MS-IME の変換モードがカタカナに切り替わらない。Windows の
一般的な IME 挙動（Shift+かな→カタカナ）が awase 使用中は発火しない。
詳細な再現ログ・症状は `docs/known-bugs.md` BUG-116 参照。

### 根本原因

`crates/awase-windows/src/runtime/transport.rs::PhysicalKeyDisposition::plan`
（`:236-260`）は、`ime_actuation_owned`（GJI/MS-IME への直接 actuation が
有効）な文脈で、`VK_DBE_ALPHANUMERIC`/`VK_DBE_KATAKANA`/`VK_DBE_SBCSCHAR`/
`VK_DBE_DBCSCHAR` の KeyDown を **`shadow_toggled` の値に関わらず常に
Suppress** する:

```rust
let is_dbe_mode_key_down = matches!(dbe_mode_key_policy, DbeModeKeyPolicy::Suppress)
    && matches!(event.vk_code,
        crate::vk::VK_DBE_ALPHANUMERIC | crate::vk::VK_DBE_KATAKANA
            | crate::vk::VK_DBE_SBCSCHAR | crate::vk::VK_DBE_DBCSCHAR)
    && event.event_type == KeyEventType::KeyDown;
ime_actuation_owned && (shadow_toggled || is_dbe_mode_key_down || matches!(event.event_type, KeyEventType::KeyUp))
```

この無条件 Suppress は BUG-52（2026-08-05, `bdf4a139`→`9a02ce6b`）で
導入された。BUG-52 の実際の repro は次の通り（`docs/known-bugs.md` BUG-52節）:

> NICOLA の物理「IME ON」キー（scan 0x70、awase の engine トグル用に
> 割り当て）を **Shift なしで** 連打すると、IME が既に ON の状態で
> `VK_DBE_HIRAGANA` の代わりに `VK_DBE_KATAKANA` が Windows のキーボード
> レイアウト変換層によって生成され、それが素通しされて実 IME が勝手に
> カタカナへ切り替わる。

つまり BUG-52 が実際に潰したかったのは「**Shift を押していないのに**
`VK_DBE_KATAKANA` が生成されてしまう、OS 側の状態依存トグルによる誤爆」
である。ところが修正は `event.vk_code` の種類だけで場合分けし、
`shadow_toggled` を判定に使うのをやめてしまったため、Shift の押下有無を
一切見ていない。結果として:

- **Shift なし + `VK_DBE_KATAKANA`**（BUG-52 が防ぎたかった誤爆）→ 正しく Suppress
- **Shift あり + `VK_DBE_KATAKANA`**（ユーザーが明示的に要求した Windows
  標準のカタカナ変換）→ **区別されず同じく Suppress**（BUG-116）

両者は `vk_code` だけでは区別できず、実際に区別できる唯一の手がかりは
「そのとき Shift が押されていたか」である。

### 弁別に使える手がかりが既に存在する

`RawKeyEvent`（`src/types.rs:190-`）は `modifier_snapshot: ModifierState`
（フック時点でキャプチャした修飾キー状態、`src/types.rs:108-113`。
`.shift: bool` フィールドを持つ）を既に保持している。`plan()` は
`event: &RawKeyEvent` を受け取っているため、追加のイベント配線なしに
`event.modifier_snapshot.shift` を読める。

## 決定（ドラフト）

`is_dbe_mode_key_down` の判定に `!event.modifier_snapshot.shift` を追加し、
Shift 押下中はこの追加 Suppress 条件を無効化する:

```rust
let is_dbe_mode_key_down = matches!(dbe_mode_key_policy, DbeModeKeyPolicy::Suppress)
    && matches!(event.vk_code,
        crate::vk::VK_DBE_ALPHANUMERIC | crate::vk::VK_DBE_KATAKANA
            | crate::vk::VK_DBE_SBCSCHAR | crate::vk::VK_DBE_DBCSCHAR)
    && event.event_type == KeyEventType::KeyDown
    && !event.modifier_snapshot.shift; // 追加: Shift押下中はBUG-52対策を適用しない
```

**根拠:** BUG-52 の repro は Shift なしのケースのみで確認されている
（上記引用）。Shift 押下中に `VK_DBE_KATAKANA` が生成されるのは Windows
標準の「Shift+かな→カタカナ」トリガーであり、ユーザーの明示的な操作を
表す。`shadow_toggled`（NICOLA engine トグルが実際に発火したか）と
`event.modifier_snapshot.shift`（このキー押下で Shift も同時に物理的に
押されていたか）は独立した軸であり、両方を条件に組み込むことで
「engineトグル連打によるOS側の誤爆」と「Shift併用によるユーザーの明示的な
カタカナ要求」を区別できる。

**変更しないもの:**

- `shadow_toggled` / KeyUp 側の既存 Suppress 条件（BUG-46 の対策）は不変。
- `AppImeProfile::Standard`（ImmCross）の無条件 Suppress（`:228-230`）は
  スコープ外——「ImmCross アプリには物理 IME キーを見せない」という別の
  設計原則（`feedback_immcross_owns_kanji`）によるものであり、本 ADR の
  対象である `ime_actuation_owned` 分岐（TsfNative/Imm32Unavailable）とは
  独立している。ImmCross での Shift+かな→カタカナ欠落は別課題として
  切り出す。
- `dbe_mode_key_policy = Passthrough`（既存の隠し設定）は「BUG-52 のリスクを
  丸ごと引き受ける」という粗い全有効/全無効の抜け道のまま残す。本決定は
  `Suppress`（既定値）のままでも Shift 併用時だけ正しく動くようにする、
  より精密な修正。

## 検討した代替案

### 案B: `dbe_mode_key_policy` の既定値を `Passthrough` に変える

却下。BUG-52 の誤爆（Shift なし連打でのカタカナ化）が全面的に復活する。
Shift の有無という安価な弁別軸があるのに、それを使わず「両方許すか
両方禁止するか」の二択にとどめる理由がない。

### 案C: `shadow_action` の事前分類（`ime_relevance.shadow_action`）側で
Shift 押下時は「IME トグル関連キーではない」と再分類する

不採用（保留）。`ime_relevance` の分類はプラットフォーム層のキー入力時点
での事前分類であり、`should_use_shift_plane`（`src/engine/nicola_fsm.rs:1163`）
の NICOLA 小指シフト面判定など他のロジックとも共有されている可能性がある
（要調査）。影響範囲の洗い出しが `transport.rs` 単体の条件追加より大きく
なるため、まずは案（決定）の局所修正で様子を見て、他の弊害が見つかった
場合にのみ検討する。

## 未解決事項

- **実機検証未実施。** Shift 併用時に `event.modifier_snapshot.shift` が
  期待通り `true` を保持しているか（modifier down イベントと DBE キー
  イベントの到着順序・タイミング依存でスナップショットが古い可能性がないか）
  は実機ログで確認する必要がある。
- **BUG-52 の repro が本修正後も再発しないことの確認が必須。** 「Shift
  なしで NICOLA の物理 IME ON キーを連打する」シナリオを実機で再テストする。
- `AppImeProfile::Standard`（ImmCross）での Shift+かな→カタカナ欠落は
  本 ADR のスコープ外。必要なら別 BUG/ADR を起票する。
- 回帰テストの置き場所: `crates/awase-windows/src/runtime/transport.rs` に
  既存の `plan` 用ユニットテストがあれば、Shift 押下時 `VK_DBE_KATAKANA`
  KeyDown → `Allow`、Shift なし → `Suppress` のケースを追加する
  （`.claude/rules/fix-requires-evidence.md` の「キー選択」ファミリー
  該当、テストか known-bugs.md 追記のいずれかが必須）。
