# ADR-073: GJI 検出後は active_ime_kind をプロセス中固定（MS-IME への降格禁止）

## ステータス

採用済み（2026-07-01 実装、commit 3157d62）

## コンテキスト

### active_ime_kind の動的検出

ADR-066 で導入した `gji_monitor` は 2 秒ごとに GJI の CLSID をポーリングし、
`active_ime_kind` を `GoogleJapaneseInput` か `MicrosoftIme` かに更新する。

### 問題: CLSID ポーリングで GJI → MS-IME に書き戻されるケース

GJI の CLSID が一時的に読み取れない（モニタースレッドのタイミング次第）と、
`gji_monitor_ok = false` と誤判定され `active_ime_kind` が `MicrosoftIme` に戻ることがあった。

その直後に `Ctrl+変換`（IME ON）が押されると：

1. `active_ime_kind == MicrosoftIme` → `MsImeDirectStrategy` を選択
2. `MsImeDirectStrategy` は `VK_DBE_HIRAGANA` を送信
3. GJI 環境では `VK_DBE_HIRAGANA` の効果が `VK_IME_ON` と異なり、
   半角英数（`conv_mode = 0x0010`）の状態では `MsImeDirectStrategy` が
   `conv_mode` を `is_roman_reliable` で正しく読めず Noop になった

### 症状

半角英数から `Ctrl+変換` でひらがなに戻らない。

### なぜ「降格」が問題か

GJI が検出されたということは OS に GJI がインストールされており動作している。
その後 CLSID が一時的に読み取れなくても「GJI がなくなった」わけではない。
MS-IME 用の戦略（`VK_DBE_HIRAGANA` 等）を GJI 環境に使うと副作用が異なるため誤動作する。

## 決定

`gji_monitor.rs` の `notify_gji_clsid_found()` で `active_ime_kind` を
`GoogleJapaneseInput` に設定した後は、プロセスが終了するまで降格しない。

```rust
// tsf/gji_monitor.rs
fn set_active_ime_kind(kind: ActiveImeKind) {
    // GJI が一度確定したらプロセス中は固定。CLSID 再読み取りで MS-IME に戻らせない。
    if ACTIVE_IME_KIND.load(Ordering::Relaxed) == ActiveImeKind::GoogleJapaneseInput as u8 {
        return;
    }
    ACTIVE_IME_KIND.store(kind as u8, Ordering::Relaxed);
}
```

### デバッグ時の切り替え方法

強制的に `active_ime_kind` を変えたい場合はプロセスを再起動する。
トレイの「再起動」メニュー（ADR-052 / a720c7a で `TrayCommand::Restart` に改名）を使う。

### GJI → MS-IME ダウングレードが正当化されるシナリオがないか

- GJI を途中でアンインストール → OS 再起動が伴う。プロセス再起動で対応可。
- 複数の IME を切り替えながら使う → 現状の想定ユースケース外。
  将来対応する場合は明示的な「IME 切り替えイベント」（WM_IME_KIND_CHANGED 等）を
  設けてプロセス中リセットを検討する。

## 検討した代替案

### ポーリング間隔を短くして CLSID 読み取りの漏れを減らす

→ 採用しなかった。根本的に競合する可能性は消えない。
  2 秒を 500ms にしても問題が稀にしか再現しないだけで構造的には同じ。

### GJI 非検出時に即座に MS-IME に切り替えず、N 回連続非検出で切り替える

→ 採用しなかった。「N 回」の閾値チューニングが必要になり、
  実際に GJI が削除されたケース（想定外）との区別も難しい。
  プロセス中固定のほうが推論が単純。

## 結果

- CLSID ポーリングの瞬間的な読み取り失敗で `active_ime_kind` が振れなくなった
- 半角英数 + GJI 環境での `Ctrl+変換` が正常に `MsImeDirectStrategy` を選ばなくなった
  （`GjiDirectStrategy` が選ばれ `VK_IME_ON` が送られる）
- `active_ime_kind` の変化頻度が減り、策略選択のロジックが安定した

## 関連 ADR

- ADR-063: TSF 共通層と IME 固有層の分離（`active_ime_kind` の導入背景）
- ADR-066: GJI CLSID 検出（ポーリング機構の詳細）
- ADR-052: トレイメニューからの状態リセット（プロセス再起動の手段）
