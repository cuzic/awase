# 実験ログ（IME 制御まわりの試行錯誤の記録）

awase の IME ON/OFF 制御・warmup・focus 分類まわりは、Windows / IME / アプリ / idle
時間の組み合わせに強く依存し、**実機で試して初めて分かる**挙動が多い。同じ仮説を
別セッションで再検証したり、一度捨てた選択肢に戻ったりする「反転」が繰り返し起きて
きた。それを見えるようにするのがこのログの目的。

## 書き方

新しい試行を行うたびに 1 行追記する。判定が後日ひっくり返ったら、元の行は消さずに
新しい行を足す（反転の履歴そのものが資産）。

| 列 | 意味 |
| --- | --- |
| 日付 | コミット日（`git log` の author date） |
| 仮説 | 「この変更で何が直る／良くなるはず」という事前の見立て |
| 環境 | 再現・検証した アプリ × IME × idle 条件（分かる範囲で具体的に） |
| 変更 | 何をどう変えたか（定数・戦略・キー選択など） |
| 観測結果 | 実機で何が起きたか |
| 判定 | 採用 / 撤回(revert) / 保留 |
| コミット | 対応するハッシュ |

関連ルール: [experiment-logging](../.claude/rules/experiment-logging.md)（revert コミット本文の必須項目）、
[tuning-constants](../.claude/rules/tuning-constants.md)（タイミング定数変更の実測義務）。

---

## エントリ 01: TsfNative + GJI の「IME OFF に何のキーを送るか」— 5 日間で 6 回反転

**背景**: Windows Terminal 等の TSF ネイティブアプリで GJI（Google 日本語入力）を
直接入力（DirectInput）に切り替えるとき、どの仮想キーを送れば「真の IME OFF」に
なるかが、キーごとに副作用が違って一意に定まらなかった。候補は
`VK_KANJI`（0x19, トグル）/ `VK_DBE_ALPHANUMERIC`（0xF0, 半角英数 = IME ON のまま）/
`VK_IME_OFF`（0x1A, 直接入力・冪等）/ `F22`（config1.db keybind 経由）。

以下は `git log` で確認した実際の変遷（author date 昇順）。5 週間前の前史
`d4d9e27` も含む。

| 日付 | 仮説 | 環境（アプリ × IME × idle） | 変更 | 観測結果 | 判定 | コミット |
| --- | --- | --- | --- | --- | --- | --- |
| 2026-05-22 | `VK_IME_ON/OFF` で双方向制御できるはず | Chrome × GJI | `VK_IME_ON`(0x16)/`VK_IME_OFF`(0x1A) を採用しようとした | **Chrome は `VK_IME_ON/OFF` を受け付けない**ことを確認 | 撤回 → `VK_KANJI` + shadow チェックに戻す | `d4d9e27` |
| 2026-06-27 | F22 はコールド時 ~750ms かかるので、TsfNative では `VK_DBE_ALPHANUMERIC` で即時 OFF にできるはず | Windows Terminal × GJI × ~80 秒 idle | TsfNative の IME OFF を `VK_DBE_ALPHANUMERIC` に切替 | 即時 OFF にはなった | 採用（この時点） | `534051a` |
| 2026-06-28 | ↑の即時 OFF がフォーカス変更時に暴発しているのでは | GJI（フォーカス変更時） | `VK_DBE_ALPHANUMERIC` → `F22` に revert | spurious な `apply_ime_open(false)` を F22 の ~750ms 遅延が実は抑えていた | 撤回（F22 に戻す） | `098c663` |
| 2026-06-28 | `VK_DBE_ALPHANUMERIC` は「半角英数(IME ON)」で確定 Enter が要る。`VK_IME_OFF` なら直接入力 | Windows Terminal 等 TSF × MS-IME | IME OFF を `VK_DBE_ALPHANUMERIC` → `VK_IME_OFF` に | （直後に revert） | 撤回 | `9c3f11e` |
| 2026-06-28 | （↑を即 revert） | 同上 | `9c3f11e` を revert | — | 撤回 | `668a131` |
| 2026-06-28 | TsfNative では F22 が TSF compartment を閉じず「半角英数」止まり。`VK_KANJI` なら compartment を正しく閉じる | Windows Terminal × GJI | GJI+TsfNative を `VK_KANJI` フォールバックに戻す | `VK_KANJI` で直接入力を達成 | 採用（次ステップで `VK_IME_OFF` 冪等化を予告） | `adb856c` |
| 2026-06-28 | `VK_IME_ON/OFF` は config1.db バインド不要で冪等。F21/F22 を全廃できる | GJI 全般 | F21/F22 送信を `VK_IME_ON`/`VK_IME_OFF` に完全移行・`VK_F21`/`VK_F22` 定数削除 | （移行実施） | 採用 | `b271aee` |
| 2026-07-01 | Ctrl+無変換 が DirectInput でなく半角英数(IME ON)になる。`VK_KANJI` トグルで DirectInput へ | TsfNative × MS-IME | `MsImeDirectStrategy` の IME OFF を `VK_KANJI` に（conv=0 を AlreadyMatched 扱い） | DirectInput へ移行 | 採用（暫定） | `be3b056` |
| 2026-07-01 | `VK_IME_OFF` は GJI・MS-IME がネイティブ処理する冪等キー。`VK_KANJI`+conv=0 の workaround は要らない | TsfNative × MS-IME | `MsImeDirectStrategy` を `VK_IME_OFF`（冪等）に。workaround 撤去 | 冪等 no-op を達成、shadow desync の影響を受けない | 採用 | `48a667a` |
| 2026-07-02 | GjiDirect の TsfNative 除外はもう不要（`VK_IME_OFF` 移行済み）。かつ candidate_was_seen の持ち越しが誤判定源 | Chrome で候補窓表示 → Windows Terminal へフォーカス移動 × GJI | GjiDirect の TsfNative 除外を撤廃 + フォーカス変更時に candidate_was_seen をリセット | Engine が OFF のまま固まるバグを解消 | 採用 | `489cdf1` |

**学び**:

- `VK_DBE_ALPHANUMERIC`(0xF0) は「半角英数」= **IME ON のまま**であり、直接入力
  （IME OFF）とは意味が違う。TsfNative で「OFF にしたつもり」が達成できない主因。
- `VK_IME_ON/OFF`(0x16/0x1A) は **Chrome では効かない**（`d4d9e27` で確認）が、
  GJI/MS-IME にはネイティブに効き、**冪等**なので shadow desync に強い（`48a667a`）。
  → アプリ（IMM/TSF）× IME（GJI/MS-IME）でキー選択が変わる。単一の「正解キー」は無い。
- 「即時に OFF できる」ことが必ずしも良いとは限らない（`098c663`）。F22 の遅延が
  spurious OFF の実害を偶然抑えていた例があり、レイテンシ短縮が別のバグを露出させた。
- 反転が 6 回続いた根本は、キー選択（対症）と spurious apply の抑制（根治）が
  絡み合っていたこと。最終的に `489cdf1` で「キー冪等化 + candidate_was_seen リセット」
  の両輪が揃って収束した。
