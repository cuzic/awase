# ADR 024: 修飾キーの PassThrough 保証と ping ベースフック監視

## ステータス

承認済み（実装完了）

## コンテキスト

### 問題 1: Ctrl キーのスタック

Ctrl KeyDown が OS に PassThrough された後、エンジン状態変化（IME トグル、フォーカス変更等）が発生すると、対応する Ctrl KeyUp が Engine に Consume されて OS に到達しないことがある。OS は Ctrl が押されたままと認識し、操作不能になる。

### 問題 2: 不要なフック再インストールの多発

従来のフック監視は「3 秒間キー入力がない → フックが死んだ」と判定していた。ユーザーが考えている間、マウスを使っている間など、キーボードを 3 秒触らないのは完全に正常。これにより:

1. 3 秒ごとに不要な再インストールが発生
2. 再インストール時に処理中の KeyUp が喪失
3. 修飾キーがスタック → `GetAsyncKeyState` が「キーが押されている」と検出
4. watchdog が「キーが押されているのにフック反応なし」と誤判定 → 再インストール
5. **悪循環**

## 決定

### Ctrl/Alt/Win の Engine バイパス

Ctrl、Alt、Win キーは NICOLA 親指シフトの処理に関与しない。フックレベルで Engine をバイパスし、常に OS に直接渡す。

```
Ctrl/Alt/Win KeyDown:
  1. Engine にイベントを送信（保留中の NICOLA 文字をフラッシュ促進）
  2. Engine の判定結果を無視
  3. 常に OS に PassThrough

Ctrl/Alt/Win KeyUp:
  1. Engine に送信しない
  2. 常に OS に PassThrough
```

#### Shift は対象外

Shift は NICOLA の小指シフト面の選択に使用されるため、Engine を通す必要がある。Shift を OS にバイパスすると、OS がデフォルト動作（大文字入力等）を実行してしまう。

#### コンボ検出への影響

Ctrl+Convert（IME ON）等のコンボは、Convert キー到着時に `GetAsyncKeyState` または `modifier_key` フィールドで Ctrl 押下を検知できるため、Ctrl が Engine を通らなくても問題ない。

### ping ベースのフック監視

「キー入力がない = フックが死んだ」という誤った前提を廃止し、能動的な生存確認に切り替えた。

```
10 秒ごと:
  1. 前回の ping 後にフックが応答したか確認
     → 応答なし: フック消失 → 再インストール
  2. VK_NONAME (0xFC) + INJECTED_MARKER で合成キーイベントを送信（ping）
     → フックが生きていれば LAST_HOOK_ACTIVITY が更新される
     → フックが死んでいれば何も起きない
```

VK_NONAME (0xFC) は実在しないキーコードで、OS やアプリに副作用がない。INJECTED_MARKER 付きなのでフックが受信してもパススルーされるが、ハートビートタイムスタンプは更新される。

## 将来の設計課題: KeyUp 発動方式

現在の Shift の KeyDown/KeyUp ペア保証は構造的ではなく、実質的にほぼ問題は起きないが理論上は壊れうる。

### 案: 状態変化を KeyUp で発動

エンジン状態変化（IME トグル等）を KeyDown ではなく KeyUp で発動する設計に変更すれば、全修飾キーの KeyDown/KeyUp ペアが構造的に保証される:

1. 修飾キー KeyDown → 常に PassThrough（状態通知のみ）
2. コンボキーの KeyUp → ここでアクション発動
3. 修飾キー KeyUp → 常に PassThrough

これにより Shift を含む全修飾キーで KeyDown/KeyUp ペアが絶対に壊れない。ただし、エンジンのコンボ処理アーキテクチャの変更が必要なため、Shift で実際に問題が発生した場合に検討する。

## 結果

### メリット

- Ctrl/Alt/Win の KeyDown/KeyUp ペアが構造的に保証される
- 不要なフック再インストールが完全に排除される
- Ctrl スタックの悪循環が解消される
- ユーザーがキーボードを触らない間のログノイズが大幅に減少

### デメリット

- Shift の KeyDown/KeyUp ペアは構造的保証がない（実質的には問題なし）
- ping 間隔（10 秒）分のフック消失検出遅延がある

### 実装ファイル

| ファイル | 変更内容 |
|----------|----------|
| `crates/awase-windows/src/hook.rs` | `is_non_shift_modifier()`, Ctrl/Alt/Win バイパス処理, `send_ping()` |
| `crates/awase-windows/src/main.rs` | watchdog を ping 方式に変更, タイマー間隔 3s → 10s |
