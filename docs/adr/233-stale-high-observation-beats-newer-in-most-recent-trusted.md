---
id: ADR-233
title: |-
  鮮度の尽きた High 観測が新しい Medium 観測に勝つ件(BUG-189)— most_recent_trusted の比較順
summary: |-
  BUG-189: Flutter × MS-IME で、フォーカス取得時に 1 件記録された ImmCrossProbe(High, IME=false)が `expires_at: None` で残り、打鍵中に観測の書き込みが止まって 3.0s(OBSERVATION_FRESH_WINDOW_MS)経つと、`most_recent_trusted` が信頼度を先に比べるため新しい ObserverPoll(Medium, IME=true)より優先され、実効 IME 状態が約 28ms false に反転する。その間エンジンが止まり、親指キー(変換/無変換)が生のまま MS-IME へ渡って入力モードが切り替わる。CI(run 37431493935)で 8 回中 6 回再現、反転 26/26 が同じ根拠。
status: |-
  起草中(2026-10-06)。方針は Opus 敵対レビューで収束させてから実装する。
related_adr:
  - "ADR-087"
  - "ADR-106"
  - "ADR-208"
---

# ADR-233: 鮮度の尽きた High 観測が新しい Medium 観測に勝つ件

## 背景(BUG-189、CI で再現)

[BUG-189](../known-bugs/BUG-189.md)。Flutter(TextField)× MS-IME で awase ありのときだけ、打鍵の途中から出力が全角英字・カタカナに化ける(GJI と、awase なしの対照は PASS)。

診断ログ `[effective-open-flip]`(`platform_state.rs::effective_open_at`、挙動を変えない)で、反転の根拠を特定した。

1. フォーカス取得時に `[ImmCrossProbe] child-hwnd IME=false → High confidence 観測記録`(`runtime/key_pipeline.rs`、AppImeProfile::Standard かつ日本語 IME のとき)が 1 件だけ記録される。**`expires_at: None`** で残る。
2. 打鍵中は `Skipping observer/SSOT write: typing active` で ObserverPoll(Medium)の書き込みが止まる。最後の書き込みから 3.0s で `derive_any` が None になる(`OBSERVATION_FRESH_WINDOW_MS`)。
3. フォールバックの `ObservationStore::most_recent_trusted`(`state/observation_store.rs`)は `max_by(confidence, then at)` で、**信頼度を先に、新しさを後に**比べる。鮮度窓も focus 同一性(`is_identity_ok`)も見ない。古い High(false)が新しい Medium(true)に勝つ。
4. 実効値が false → `Engine deactivated (ime=false, reason=Inactive(ImeOff))`。次の観測が入るまで約 28ms、親指キーが `PassThrough` で MS-IME に届き、モードが切り替わる。

反転 26 回中 26 回が `→ false decided_by=MostRecentTrusted(ImmCrossProbe)`。wx・Qt では観測の空きが 3s を超えても反転しない(ImmCrossProbe が false にならない)。

なお、記録時点の IME=false 自体は誤読でない可能性がある(コールド起動で awase が IME を ON にする前の実状態)。その後 IME が ON になり、新しい ObserverPoll(true)が入っても、High の古い値が居座ることが問題。

## 決めること

`most_recent_trusted`(belief のフォールバック)が、**時間が経って鮮度の尽きた高信頼の観測**と**新しい中信頼の観測**のどちらを採るか。

## 選択肢

| 案 | 内容 | 利点 | 懸念 |
| --- | --- | --- | --- |
| A | 鮮度が尽きた観測同士では新しさを先に比べる(信頼度は同時刻帯のタイブレーク) | 順序の欠陥を根本から直す。`derive_filtered` の「High 即採用 → Medium 多数決」が鮮度窓内でだけ働く設計と整合する | belief 全体に効く。BUG-16 の回復力(`heuristic_default` の注記)を削らないか、journal リプレイで確認が要る |
| B | 子窓に自前の IME コンテキストが無い入力先の ImmCrossProbe(false)を記録しない/Medium に落とす | 影響が小さい | 同じ形の別の入力先が出るたびに足す。今回の false が誤読かどうかの確証が無い |
| C | ImmCrossProbe に有効期限(`expires_at`)を付ける | 単純 | 期限の長さを実測で決める必要。actuation の根拠(`ActuatingPool`)に使う側にも影響 |
| D | `most_recent_trusted` にも `is_identity_ok`(focus の epoch・hwnd 照合)を適用する | 別フォーカスの古い観測が勝つ経路を塞ぐ(今回の直接原因ではない) | 単独では今回を直さない。A・C と併用する候補 |

## 評価に必要な測定(実装前)

- journal リプレイ(`tests/journal_replay.rs`、`tests/journals/`)の既存コーパスで、A を入れて `effective_open` の列が変わる箇所を数える(変わるなら、各件を「正しい方へ」か「退行」かで分類する)。
- CI の既存構成(`tsx-ext-*`・`ts-*`)で、全入力先が PASS のままか。
- BUG-189 の再現構成(`tsx-ext-flutter-textfield-msime-*`、8 回)で反転が 0 になるか。

## 状態

起草中。Opus 敵対レビューで案の比較・見落とし(特に A が BUG-16 の回復力や ADR-087 の根拠階層を壊さないか)を確認してから決める。
