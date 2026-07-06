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

---

## エントリ 02: 「非TSFウィンドウ = 日本語IMEなし」という前提の偽 FocusProbe(false) 注入

**背景**: Win+X メニューで1文字ショートカットが NICOLA 変換される（P→'，'）バグに対し、
TsfGate の bypass 確定時に `write_focus_probe(false)` で belief を強制 OFF する対策が
取られた。詳細は [docs/known-bugs.md BUG-07](known-bugs.md)。

| 日付 | 仮説 | 環境（アプリ × IME × idle） | 変更 | 観測結果 | 判定 | コミット |
| --- | --- | --- | --- | --- | --- | --- |
| 2026-05-27 | 非TSFウィンドウには日本語IMEが無いので bypass 確定時に belief を false に固定してよい | Win+X メニュー × MS-IME | bypass_tsf() 前に `write_focus_probe(false)` を注入 | Win+X の誤変換は解消（当時） | 採用（この時点） | `ce45b82` |
| 2026-07-06 | ↑の前提が誤り。Edge/Chrome は非TSF注入だが日本語IME有効で、実観測経路ゼロのため偽 Low false が belief を支配する | MS Edge (Chrome_WidgetWin_1) × MS-IME × フォーカス直後 | `write_focus_probe(false)` を撤去（実質 revert）+ architecture_guard で呼び出し箇所を実 probe 経路に固定 | Edge フォーカス約500ms後の Engine 必 OFF が解消（実機検証待ち）。Win+X は既知 NonText クラス + NonText パススルーで保護継続 | 撤回(revert) | （本修正） |

**学び**:

- 「このウィンドウ種別に IME は無いはず」という推測を observation として書くのは
  ime-belief-architecture 規約の禁止パターン2（観測の偽装）。推測は
  `HeuristicDefault + Low`、キーを処理させたくないだけなら `FocusKind::NonText` を使う。
- 偽観測は**実観測経路を持つアプリでは無害に見える**（Medium/High が上書きするため）。
  被害が Imm32Unavailable に限定されるせいで1ヶ月以上潜伏し、別バグ（ObservedEisu
  循環デッドロック）の修正後も症状が残ることで初めて発見された。
- `dispatch_event` はジャーナルに全イベントを残すが DEBUG ログには出さない。
  「ログに書き込みが見えないのにbeliefが反転する」場合はジャーナルか、ログを出さない
  dispatch 呼び出し元を疑う。

---

## エントリ 03: JISかな自動復元（restore_roman）と UIA 非同期分類 — 同日中に採用→撤回

**背景**: BUG-08（合成 VK_KANA による JISかな化）の自己修復層と、BUG-09
（post_to_main_thread 誤配送）修正で初めて動き出した UIA 非同期 focus 分類。
どちらも同日中に実機で副作用が確認され撤回した。詳細は
[docs/known-bugs.md](known-bugs.md) BUG-08 追補2 / BUG-11 / BUG-12。

| 日付 | 仮説 | 環境（アプリ × IME × idle） | 変更 | 観測結果 | 判定 | コミット |
| --- | --- | --- | --- | --- | --- | --- |
| 2026-07-06 | conv=0x0009（ROMAN喪失）は実際の JISかな化なので自動復元してよい | WT × MS-IME (TsfNative) | restore_roman を steady-state でも発火 | ROMAN=0 は偽陽性（closed/idle 時 MS-IME が ROMAN を落として報告）。復元書き込みで conv が 0x19⇄0x09 を往復し、ObservedEisu/NativeToggleShadowOff が誤発火 → **直接入力中に spurious Engine ON + IME ON** | 撤回（is_roman_reliable=true 必須に） | `92fddc8` → 本修正 |
| 2026-07-06 | UIA 非同期分類の結果は帰属さえ正しければ (pid,class) キャッシュしてよい | MS Edge × MS-IME | BUG-11 修正（result_hwnd から帰属導出） | ページ本文フォーカス時の「正しい NonText」が (pid,class) で固着 → ウィンドウ内クリックでは再分類されず Edge 永久 NonText → 全キーがエンジン素通し | 撤回（handler をログのみに、BUG-12） | `d941721` → 本修正 |

**学び**:

- **conv の ROMAN ビットは IME × プロファイル × open 状態で信頼性が変わる**。
  「TsfNative では ROMAN が常に 0」という古いコメント（`is_roman_reliable=false` の根拠）は
  正しかった。信頼できない読み値に対して是正書き込みをすると、書いた値と IME の報告が
  往復して**他の conv ベースルールを誤発火させる**（二次被害が一次症状より重い）。
- **focus kind の粒度はウィンドウではなく要素**。ブラウザでは同一 (pid,class) の中で
  TextInput⇄NonText が毎秒変わるため、ウィンドウ粒度のキャッシュはどちらの値でも毒になる。
- **長期間 dead だったコードパスの配送を直すときは、そのパスを一時停止した状態で直す**。
  BUG-09 の配送修正自体は正しかったが、「届いたことのないハンドラ」が全部動き出し、
  未検証コードの潜在バグ（BUG-11/12）が一気に露出した。配送修正と機能有効化は分離すべきだった。

---

## エントリ 04: foreign-injected IME モードキーの全面 swallow — 即日撤回（一切入力不能）

**背景**: BUG-14（外部注入 VK_DBE_HIRAGANA が PhysicalImeKey と誤読され、ユーザーの
IME OFF が Engine ON で上書きされ続ける）への防御として、BUG-08 の VK_KANA swallow を
IME モードキー全般に一般化した。詳細は [docs/known-bugs.md](known-bugs.md) BUG-14。

| 日付 | 仮説 | 環境（アプリ × IME × idle） | 変更 | 観測結果 | 判定 | コミット |
| --- | --- | --- | --- | --- | --- | --- |
| 2026-07-06 | foreign-injected (LLKHF_INJECTED) の IME モードキーは全て「偽装ユーザー意図」なので swallow してよい | Windows Terminal × MS-IME (TsfNative) | hook で ImeKeyKind 全 VK の foreign-injected を swallow | **一切入力できなくなった**。1 打鍵ごとに foreign-injected VK_KANA down+up ペア（injected=true, scan=0x0）が到達し swallow が連発、conv=0x0009 (ROMAN=false) 固定、エンジンは全キー PassThrough で不活性のまま | 撤回（VK_KANA のみの BUG-08 swallow に復元、injected= ログは維持） | `b8467b8` → 本 revert |

**学び**:

- **foreign-injected IME モードキーは「ノイズ」ではなく MS-IME 自身の機能的なキー注入を
  含む**。1 打鍵ごとの VK_KANA ペアという高頻度パターンは、IME のモード遷移・かな修飾の
  実装の一部とみられ、hook 層で遮断すると IME の状態機械そのものが壊れる。
- **遮断（swallow）と解釈の修正は別物**。BUG-14 の本質は「注入イベントをユーザー意図
  （PhysicalImeKey）として解釈する」ことであり、対処は shadow toggle 側で
  「injected イベントは意図に昇格させない（観測として扱う）」べき。OS への配送は
  維持したまま awase の解釈だけを変える。
- 副産物: injected= ログにより BUG-08 以来未特定だった注入元が **LLKHF_INJECTED 付き
  SendInput 由来と確定**（ドライバレベルではない）。
