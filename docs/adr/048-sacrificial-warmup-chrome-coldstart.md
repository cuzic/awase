# ADR-048: SacrificialWarmup — Chrome cold-start の不可視プローブ方式

## ステータス

採用済み（2026-06-14〜2026-06-24 実装、運用監視中）

## コンテキスト

### 問題: Chrome cold-start で部分リテラル化が発生する

Chrome（Brave / Edge を含む）で cold-start 後（IME OFF → ON 直後）に
ローマ字を入力すると「kおのなかで」のような部分リテラルが発生する。

例:「こ」を入力すると `K→ [cold] → O→ [GJI composition 開始]` となり、
K が GJI に届く前にリテラル送信されて `k + お` が残る。

**原因の構造**:
1. Chrome は VK モード（InjectionMode::Vk）で動作する
2. GJI の composition context は cold 状態では VK を素通りさせる
3. F2（DBE_HIRAGANA）で GJI を活性化するまでに数百 ms の遅延がある
4. 最初の VK が GJI 活性化前に届くとリテラル化する

### 否定された代替案

| 案 | 否定理由 |
|---|---|
| NameChangeWait 延長（300ms → 500ms） | WezTerm 初期化遅延（344ms）に追随できない。時定数チューニングは本質的に競合条件 |
| WM_IME_STARTCOMPOSITION 傍受 | DLL インジェクションが必須（AV 誤検知リスク大） |
| ImmGetCompositionString クロスプロセス | TSF native アプリでは常に 0 を返す。実験で 952ms 遅延が確認された |
| GetProcessIoCounters ベースのタイミング延長 | I/O 観測タイミングと Chrome TSF context 更新（COM イベント）の race が解消しない |

## 決定

### 基本方式: VK_A + BS のアトミックバッチ（commit af906b1）

cold-start を検出したら本物のローマ字を送る前に、
「不可視のプローブ文字」を送って GJI を活性化する。

```
[cold 検出]
   │
   ├── send_sacrificial_vk_a_with_bs()
   │     → SendInput([VK_A DOWN, VK_A UP, VK_BACK DOWN, VK_BACK UP])
   │        ← 同一バッチで送信することで Chrome フレーム描画前に消去
   │
   ├── GJI が VK_A で composition 開始 → I/O write_bytes 増加
   │
   ├── ChromeProbe が WriteTransferCount を観測
   │   ├── +350B 以上  → composition-confirmed → warm 遷移
   │   └── timeout → F22→F21 GJI 強制リセット → 再試行
   │
   └── warm 確認後、本物のローマ字を再送（SacrificialResend）
```

**VK_A + BS を同一 SendInput バッチで送る理由**:
別の SendInput 呼び出しに分けると Chrome がフレームをレンダリングする
タイミングで 'a'（または 'あ'）が一瞬表示される。
同一バッチなら OS がイベントキューに連続して積むため、Chrome は
レンダリング前に 'a' + BS を処理して画面に何も現れない。

**SacrificialResend で追加 BS を送らない理由**:
VK_A + BS がすでに1文字を削除済みのため、再送時は追加 BS 不要。

### LiteralDetector: WriteTransferCount ベースの確認（commit c7ef500 → 26bc0fe）

GJI の I/O バイト数を `GetProcessIoCounters.WriteTransferCount` で観測し、
composition が確立されたかどうかを「コンテンツベース」で判定する。

```
gji_last_io_ms（時刻ベース） → 問題: F2 送信だけで I/O 時刻が変化し誤検知
                            ↓
write_bytes（バイト数ベース） → F2=+0.0B、VK_A→'あ'=+300〜400B → 閾値で分離可能
```

**pre-send baseline の取得タイミング（commit 26bc0fe）**:

```rust
// NG: VK_A 送信後に baseline を取得すると cold Chrome の +300B write が吸収される
let baseline = get_gji_write_bytes();  // ← cold 時すでに書き込まれている

// OK: VK_A 送信前に baseline を取得
let baseline = probe_io.gji_write_bytes();  // ← VK_A 前の値
send_sacrificial_vk_a_with_bs(output);
// warmup FSM に baseline を渡す
SacrificialWarmupFsm::new(baseline)
```

**閾値 350B**:
実機ログ 5 サンプル:
- cold Chrome（'a' リテラル）: write +300B → timeout → SacrificialResend
- warm Chrome（'あ' composition）: write +400B → composition-confirmed

350B が中間値。サンプルが少ないため逆転ケースが出た場合は再調整する。

### Chrome の F22→F21 KeySeq を削除（commit 6137276）

`ChromeProbe` 完了後に keybinds_ok のとき F22→F21 を送る処理を
WezTerm 対策として追加していた（commit e537fe8）が、
Teams（TeamsWebView）で na → "na" リテラル化が発生したため削除。

**根本原因**: F22（IME OFF）が Chrome TSF composition context を破壊し、
GJI が settle（I/O quiet）してから Chrome TSF context が Precomposition に
戻るまでの間に na が届いてリテラル化する（race condition）。

**原則**: Chrome 系アプリでは F22 を送らない。F2（VK_DBE_HIRAGANA）のみで
GJI を活性化する。F22 は必ず TSF context を破壊する可能性がある。

### タイムアウト時の GJI 強制リセット: ImeModeFsm + ChromeGjiReinitFsm（commit 2c8d647）

SacrificialWarmup がタイムアウトした場合（Chrome が warm 状態を確認できなかった場合）、
F22→F21 シーケンスで GJI を強制的に Hiragana mode にリセットする。

このシーケンスには Hiragana 確認のための待機が必要なため、専用 FSM を設けた:

```
ImeModeFsm:
  Unknown → (F21 sent) → WaitingHiraganaConfirm → (NameChange) → Hiragana

ChromeGjiReinitFsm:
  Idle → (timeout) → EmittedF22F21 → (ImeModeFsm::Hiragana) → Complete
```

`ImeModeFsm` は F21/F22 送信後の belief を更新し、
VK 直後の FocusChange による generation race を防ぐために
`spawn_local` ポーリングに generation ガードを追加した（commit 90c9962）。

### SacrificialWarmup の適用スコープ限定（commit d02ec44）

当初は全 cold-start に適用していたが、TSF mode 以外のアプリでは
「VK_A を送っても HIDE が来ない」「composition_was_seen が設定されない」
問題が発生し性能低下した。

**限定条件**: `long-cold && tsf_mode` のアプリのみに適用。
- `long-cold`: idle が一定時間以上（WezTerm の 344ms 初期化遅延に対処するため）
- `tsf_mode`: GJI を使う TSF native アプリのみ

## 結果

- Chrome cold-start の partial literal（kおのなかで）が根本解消
- 不可視プローブで UX への影響ゼロ
- WriteTransferCount ベースの判定でタイミング依存から脱却
- F22 送信を Chrome パスから除去し、Teams literal バグを解消

## 監視ポイント

- 閾値 350B の逆転ケース（cold が 350B 以上、warm が 350B 以下）
- `grep -E '\[tsf-probe\].*ChromeProbe|sacr-warmup|gji-io\] WRITE' awase.log`

## 関連 ADR

- ADR-047: TickableFsm / ImeWarmupStrategy（ChromeProbe の位置づけ）
- ADR-046: GjiFsm（ImeModeFsm・ChromeGjiReinitFsm の呼び出し元）
- ADR-034: GJI Direct Strategy（VK mode の設計背景）
- workarounds.md: 6-F（Chrome F2 重複スキップ）
