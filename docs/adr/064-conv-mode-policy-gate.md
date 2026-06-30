# ADR-064: ConvModePolicy による conv mutation ゲートの導入

## ステータス

採用済み（2026-06-30 実装、commit af3b776〜0ed972e）

## コンテキスト

awase ON 中は IME の変換モード（conv）を `VK_DBE_HIRAGANA`（ひらがな）に固定するが、
ユーザーがトレイアイコンから JISかな・半角カタカナ等を選択した場合は conv を書き換えてはならない。

従来の実装では `is_romaji_mode: bool` という bool フラグが conv 書き換え権限を制御していたが、
この bool はキーイベントのたびに `belief.input_mode().is_romaji_capable()` から再計算されており、
以下の問題が発生していた：

1. **UserManaged 判定の経路が複数**：`send_eager_tsf_warmup`・`ImmSetConversionStatus`・
   `SendInput(VK_DBE_HIRAGANA)` の各呼び出し箇所がそれぞれ独立してガードしていた
2. **idle-conv-check による上書き**：AwaseLocked 状態で conv=0x09 を検出すると
   強制的に 0x19 へ書き戻し、ユーザーが選択した JISかな等が無効化されていた
3. **belief 読み取りが過多**：キーごとに belief から conv policy を再評価していた

## 決定

`is_romaji_mode: bool` を廃止し、`ConvModePolicy` enum で conv mutation 権限を明示的に表現する。

```rust
pub(crate) enum ConvModePolicy {
    AwaseLocked,   // awase ON 中。conv mutation を許可
    UserManaged,   // awase OFF または JISかな等。conv に一切触らない
}
```

### 変更点 1: Output への conv_mutation_allowed ゲート追加

```
Output.conv_mutation_allowed: Cell<bool>
```

`send_eager_tsf_warmup` / `ImmSetConversionStatus` / `SendInput(VK_DBE_HIRAGANA)` の
全呼び出し経路が `!conv_mutation_allowed` で early return する。
`set_conv_mutation_allowed()` は `platform.set_conv_mode_policy()` が呼ぶ。

### 変更点 2: EngineStateChanged を SSOT に

```rust
// executor.rs
UiEffect::EngineStateChanged { enabled } => {
    platform.set_conv_mode_policy(if enabled {
        ConvModePolicy::AwaseLocked
    } else {
        ConvModePolicy::UserManaged
    });
}
```

エンジン ON/OFF イベントが ConvModePolicy の唯一の更新トリガーとなり、
`conv_policy_from_belief()` 暫定計算をキーごとに行う必要がなくなった。

### 変更点 3: idle-conv-check の AwaseLocked reconcile を撤去

idle-conv-check による `ImmSetConversionStatus(0x19)` 書き戻しを削除。

**理由**：テキスト注入時の conv 設定は `cold_warmup.preamble` の
`ImmSetConversionStatus` でガード済みであり、idle-conv-check での reconcile は不要かつ有害。
reconcile 後も belief 更新が古い conv 値で行われ、エンジンが ON→OFF→ON チャタリングする
副作用もあった。

## 検討した代替案

**bool フラグのままガード箇所を集約**
→ 型が状態を表現しないため、今後の拡張（JISかな専用ポリシー等）で再び分岐が増える。
  enum 化することで意図が明確になり、`match` による網羅チェックも得られる。

**belief から都度計算**
→ 採用しなかった。belief は観測値（過去の状態）であり、
  エンジン ON/OFF という決定イベントから直接 policy を更新する方が
  タイミングのずれがなく一貫している。

## 変更ファイル

| ファイル | 変更内容 |
|---------|---------|
| `platform.rs` | `is_romaji_mode: bool` → `ConvModePolicy` enum + `set_conv_mode_policy()` |
| `output/mod.rs` | `conv_mutation_allowed: Cell<bool>` + `set_conv_mutation_allowed()` + テスト5件 |
| `tsf/cold_warmup.rs` | `WarmupContext.conv_mutation_allowed` フィールド追加・各 spawn_local ガード |
| `runtime/executor.rs` | `EngineStateChanged` での policy 更新・`conv_policy_from_belief()` 削除 |
| `runtime/key_pipeline.rs` | idle-conv-check の AwaseLocked reconcile 削除 |

## 不変条件（テストで保証）

- `Output` 初期状態は `conv_mutation_allowed = false`（エンジン OFF がデフォルト）
- `UserManaged` → `allows_conv_mutation() == false`
- `AwaseLocked` → `allows_conv_mutation() == true`
- エンジン ON イベントのみが `AwaseLocked` に遷移させる

## 関連 ADR

- ADR-023: kana bypass モード（UserManaged に近い概念の先行実装）
- ADR-046: GjiFsm warm/cold SSOT（単一 SSOT パターンの先例）
- ADR-065: conv 分類の純粋関数化（本 ADR と同日実装）
