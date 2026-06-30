# ADR-068: JISかな・カタカナモードの完全サポート

## ステータス

採用済み（2026-06-29〜30 実装）

## コンテキスト

awase は従来「ローマ字ひらがな」入力のみを前提に設計されており、IME の変換モードが
それ以外（JISかな、全角/半角カタカナ、英数）になっているケースを belief
（IME 状態の内部モデル）に正しく反映していなかった。

具体的には、ユーザーが Google 日本語入力（GJI）や MS-IME のトレイアイコン・
タスクバー・`Ctrl+変換` 等から「JISかな」「全角カタカナ」「半角カタカナ」を選んでいても、
awase の起動直後 belief は「ローマ字ひらがな」のままで、NICOLA エンジンが誤動作していた。

問題を分解すると、単一のバグではなく以下の **4 層** が絡み合っていた：

1. **belief が更新されない** — TsfNative はポーリング無効のため、タスクバー/トレイで
   モードを変えても awase 側の belief が追従しない。
2. **conv が warmup で上書きされる** — eager warmup / cold warmup が送る
   `VK_DBE_HIRAGANA` が、せっかくユーザーが選んだ JISかな・カタカナを
   ひらがなに戻してしまう。
3. **カタカナと JISかなの区別** — 両者は同じ「ネイティブかな入力」フラグ（NATIVE=1）を
   立てるが、awase にとっての意味は正反対である（後述）。
4. **カタカナ用 VK の選択** — `ImmSetConversionStatus` は TSF-native では反映されず、
   カタカナを維持するには適切な `VK_DBE_*` を送る必要がある。

これらが「belief を更新する」「conv を保護する」「種別を判定する」「正しい VK を送る」と
相互に干渉するため、個別の fix を時系列で積み上げる形で解決した。

## 主要な設計原則

### カタカナモードは ObservedRomaji 扱いとする

最も重要な判断。全角/半角カタカナは「IME 側でカタカナに直接変換するモード」ではなく、
**NICOLA エンジンでローマ字相当の打鍵 → カタカナを出力するためのモード**である。
したがって NICOLA エンジンは ON のまま維持しなければならず、belief 上は
`ObservedRomaji`（ローマ字入力相当）として扱う。

一方、JISかな（ひらがな直接入力）は NICOLA をバイパスすべきモードであり、
belief は `ObservedKana` とする。

両者は IME の conv フラグ上は共に NATIVE=1 だが、KATAKANA / ROMAN ビットで区別する：

| モード | NATIVE | KATAKANA | ROMAN | belief | NICOLA |
|---|---|---|---|---|---|
| ローマ字ひらがな | 0 | 0 | 1 | ObservedRomaji | ON |
| JISかな（ひらがな） | 1 | 0 | 0 | ObservedKana | OFF |
| 全角/半角カタカナ | 1 | 1 | * | ObservedRomaji | ON |
| 英数 | 0 | 0 | 0 | ObservedKana | OFF |

### 明示的 IME 操作は idle-conv-check より優先する

`Ctrl+変換` / `Ctrl+無変換` 等の明示的操作の直後に、自動の idle-conv-check が
古い conv 値を読んで belief を逆方向に上書きしてしまう競合があった。
明示的操作には抑制ウィンドウ（1500ms）を設け、その間は自動 conv-check を停止する。

### conv ヒントは型安全に管理する（ConvModeMgr）

カタカナビットの保存・参照を生の `u32` ビット演算で散在させると保守不能になるため、
`Charset` / `ConvMode` / `ConvModeMgr` という型に集約する。

## 決定

`ConvModeMgr` による型安全な conv 管理を導入し、その上に belief 更新・conv 保護・
種別判定・VK 選択の **多層ガード** を構築する。以下、時系列のコミット群で解決した。

### Phase 1: belief 更新とデッドロック解消

- **29bc9c4** `fix(engine)`: トレイで `user_enabled=true` だが `ime_on=false` の
  desync 時に IME を強制 ON。`ToggleEngine`/`EngineOn` で `compute_active` が
  `old==new` のまま `EngineStateChanged` を発火しないデッドロックを解消するため、
  `apply_engine_on_with_ime_recovery` を追加。`pseudo_ctx(ime_on=true)` で目標を
  再計算し `ImeEffect::SetOpen{true}` + `EngineStateChanged{true}` を発火する。
- **a325df7** `fix(focus-probe)`: ALT+TAB 後の Chrome IME 状態誤認を修正。
  `shadow_on` を `applied_open()` → `effective_open()` に変更し、FocusChanged 直後に
  キャッシュからリストア済みの `desired_open` を参照させる（前ウィンドウ状態の引き継ぎ防止）。
- **28e07e2** `fix(conv-mode)`: トレイの英数/カタカナ選択時の IME 誤操作を修正。

### Phase 2: warmup による conv 上書きの抑止

- **1678994** `fix(tsf-warmup)`: JISかなモードで `VK_DBE_HIRAGANA` の eager warmup を
  スキップ。JISかな選択時に eager warmup が `VK_DBE_HIRAGANA` を送ってひらがなに
  戻すバグを防ぐ。
- **09bb03a** `fix(focus-change)`: トレイで半角英数に切り替えた後フォーカスが戻ると
  FocusChange の eager warmup が `VK_DBE_HIRAGANA` を送ってひらがなに戻す問題。
  FocusChange 前に conv を読み、英数（NATIVE=0, ROMAN=0）なら warmup をスキップ。

### Phase 3: TsfNative での belief 反映

- **38f5186** `fix(tsf-native)`: タスクバー経由の入力モード変更を belief に反映。
  TsfNative はポーリング無効のため、タスクバーで英数切替しても belief が更新されない。
  `TYPING_IDLE_MS` 超のアイドル後、最初の KeyDown で conv を読んで belief を更新する。
- **e8b09de** `fix(tsf-native)`: JISかな・カタカナモードも belief に反映。英数だけでなく
  NATIVE=1 も `ObservedKana` に更新。ただし cold start（`output_in_flight_ms == u64::MAX`）
  では ROMAN ビットの信頼性が低いため英数判定のみに限定する。
- **58fa2ac** `fix(tsf-native)`: カタカナモードでは NICOLA エンジンを維持する。
  上記原則に従い、`NATIVE=1, KATAKANA=0, ROMAN=0` のみを `ObservedKana`
  （= JISかなひらがな）とし、カタカナは `ObservedKana` にしない。

### Phase 4: idle-conv-check の競合解消

- **a0f63ca** `fix(idle-conv-check)`: 明示的 IME 操作後 1500ms は conv-check をスキップ。
  `Ctrl+変換`/`無変換` 直後に idle-conv-check が JISかな（0x09）を読んで
  `AssumedRomaji` を `ObservedKana` に上書きしていた。`ImeStateHub` に
  `last_explicit_ime_action_ms` を追加し、`EXPLICIT_IME_SUPPRESS_MS`(1500ms) 以内は
  スキップ。
- **66bfc33** `fix(idle-conv-check)`: カタカナモード切替で NICOLA が OFF のまま残る問題。
  タスクバーで JISかな → カタカナ切替時、`belief=ObservedKana` + `shadow=false` で
  engine が inactive になる。カタカナ検出 + `belief=ObservedKana` のとき
  `ObservedRomaji` に更新し `handle_engine_set_open(true)` を呼ぶ。

### Phase 5: カタカナ用 VK の選択とヒント保存

- **109b4c9** `fix(tray-katakana)`: `VK_DBE_HIRAGANA` で KATAKANA ビットが失われる問題。
  トレイでカタカナ選択後 NICOLA を使うと warmup の `VK_DBE_HIRAGANA` が
  conv を 0x3（半角カタカナ）→ 0x9（ひらがな）に変えてしまう。
  idle-conv-check で `katakana_conv_hint` を更新（KATAKANA ビットを保存）、
  `cold_warmup.preamble()` がヒントを async ブロックに渡し、
  `set_ime_romaji_mode_with_hint()` が hint に KATAKANA ビットがあれば `hint|ROMAN` を
  目標値に使う。
- **10ab96b** `fix(tray-katakana)`: VK cold path の先頭 F2 をカタカナ用 VK に切り替え。
  `ImmSetConversionStatus` は TSF-native では反映されないため、カタカナ hint がある場合は
  先頭 VK を切り替える：全角カタカナ → `VK_DBE_KATAKANA`(F1)、半角 → F1 +
  `VK_DBE_SBCSCHAR`(F3)。`output/vk_send.rs` にカタカナ VK 送信処理を追加。

### Phase 6: 型安全化リファクタ（ConvModeMgr）

- **7d0313b** `refactor(conv-mode)`: `katakana_conv_hint` を `ConvModeMgr` に置き換え。
  生の `u32` conv ヒントを型安全な `Charset`/`ConvMode`/`ConvModeMgr` に統一。
  `state/conv_mode.rs` を新設：
  - `Charset` enum（Hiragana / ZenkakuKatakana / HankakuKatakana / ZenkakuAlpha /
    HankakuAlpha）
  - `ConvMode { charset, romaji }`
  - `ConvModeMgr`（`update_from_conv` / `get`、変化時のみ info ログ）
  - `katakana_conv_hint: Cell<u32>` → `conv_mode: ConvModeMgr`（Output フィールド）

  （その後 `Charset`/`ConvMode` の定義はプラットフォーム非依存の `nicola`(awase) クレートに
  移動し、`conv_mode.rs` は `ConvModeMgr` のみを保持する形へ整理された。）

### Phase 7: フォーカス変化時のエッジケース

- **4e9e206** `fix(focus)`: NonText ウィンドウへの cache-miss で belief をリセットしない。
  タスクバー通知領域（CoreWindow）は `FocusKind::NonText` だが
  `Imm32Unavailable` + `TsfNative` と分類され、cache-miss で
  `reset_to_off_for_tsf_native_cache_miss()` が呼ばれて belief が false にリセットされた。
  `FocusKind::NonText` では belief リセットをスキップする。
- **487c668** `fix(focus-probe)`: TsfNative フォーカス復帰時に conv mode で input_mode を
  即時補正。`HwndCache` は最大 1 時間保持されるため cached input_mode が stale になる。
  `apply_focus_probe` 内で conv を即時に読んで belief を補正する：
  英数 → `ObservedKana`、JISかな → `ObservedKana`、ローマ字 → `ObservedRomaji`
  （カタカナは idle-conv-check に委任）。

### Phase 8: 非活性化時の IME キー抑制

- **fb82eaa** `fix(engine)`: `NotRomajiInput` 非活性化時に engine-state IME キーを抑制。
  `UiEffect::EngineStateChanged` に `send_ime_key` フィールドを追加し、
  かな/カタカナによる `NotRomajiInput` 非活性化では `send_engine_state_ime_key()` を
  抑制する（不要な VK 送信が conv を壊すのを防ぐ）。

### Phase 9: パニックリセットの拡張

- **6ee20bf** `fix(panic-reset)`: JISかな・半角カタカナからもローマ字ひらがなに復帰。
- **a88bb36** `fix(panic-reset)`: カタカナ状態でリセットしてもひらがなに戻るよう修正。

  パニックリセット（[[052-tray-panic-reset]]）が想定していたのはローマ字ひらがな状態
  からの復帰だけだったが、JISかな・カタカナ状態からも確実にローマ字ひらがなへ
  戻れるようにした。

## 設計上の要点

- **「カタカナ = ObservedRomaji」が全体を貫く中心原則。**
  これを取り違えると NICOLA が OFF のまま固まる（66bfc33）か、逆にひらがな入力で
  NICOLA が誤動作する。
- **conv を「読む」タイミングと「保護する」タイミングを分離。**
  belief 更新のために conv を読むのは idle 後/フォーカス復帰時、warmup が
  `VK_DBE_HIRAGANA` で conv を壊さないようガードするのは warmup 直前。
- **TSF-native では `ImmSetConversionStatus` が効かない**ため、目標 conv は
  必ず適切な `VK_DBE_*`（HIRAGANA/KATAKANA/SBCSCHAR/ALPHANUMERIC）で表現する。

## 変更ファイル（主要）

| ファイル | 変更内容 |
|---------|---------|
| `state/conv_mode.rs` | 新設。`ConvModeMgr`（`Charset`/`ConvMode` は nicola クレートへ） |
| `output/vk_send.rs` | カタカナ用 VK（KATAKANA/SBCSCHAR）送信処理 |
| `output/mod.rs` | `katakana_conv_hint` → `conv_mode: ConvModeMgr`、cold warmup の hint 受け渡し |
| engine / reducer | `apply_engine_on_with_ime_recovery`、`UiEffect::EngineStateChanged.send_ime_key` |
| idle-conv-check | カタカナ検出 → ObservedRomaji 更新、`last_explicit_ime_action_ms` 抑制 |
| focus-probe / focus-change | conv 即時読み取りによる belief 補正、NonText reset スキップ、warmup スキップ |
| tsf-native | アイドル後 KeyDown での conv 読み取り → belief 反映 |
| panic-reset | JISかな・カタカナからのローマ字ひらがな復帰 |
| `ImeStateHub` | `last_explicit_ime_action_ms` フィールド + `EXPLICIT_IME_SUPPRESS_MS` |

## 関連 ADR

- ADR-064: ConvModePolicy による conv mutation ゲート（本 ADR の conv 制御の基盤）
- ADR-065: conv 分類の純粋関数化とプラットフォーム非依存化（`Charset`/`ConvMode` の移設先）
- ADR-063: TSF 共通層と IME 固有層の分離 + MS-IME 対応
- ADR-052: トレイメニューからのパニックリセット（Phase 9 で拡張）
- ADR-048: SacrificialWarmup（warmup の conv 上書き問題の発生源）
</content>
</invoke>
