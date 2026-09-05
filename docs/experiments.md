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

## エントリ 17: BUG-25 GJI 半角英数 entry の本実装（ADR-107 Task 1〜8）

**背景**: BUG-25 の GJI entry は scan付きF0、IMC write、scan=0 F0 の3案を
いずれも撤回済み。ADR-107 決定0の2×2実機計測で `IME_KANJI_MARKER` +
synthetic Shift↑ 前置が成立条件だと確認できたため、本実装に着手する。

| 日付 | 仮説 | 環境（アプリ × IME × idle） | 変更 | 観測結果 | 判定 | コミット |
| --- | --- | --- | --- | --- | --- | --- |
| 2026-08-27 | `IME_KANJI_MARKER`付き `VK_DBE_ALPHANUMERIC` scan=0 に synthetic Shift↑ を前置すれば、awase起動中のGJIでも左Shift単独タップでIME-ON半角英数へ入れる。GJI経路は `half_width_alnum_toggle=all` の明示設定に限定し、MS-IME既存経路はIMC write/verify-retryを維持する | Windows Terminal × Google 日本語入力（Task 9で実機検証予定） | ADR-107 Task 1〜8: 純粋action判定、kill switch、Shift↑前置SendInput helper、GJI用Output API、entry/exit配線、golden/architecture確認、記録更新 | 未実施（Windows実機検証はTask 9としてスコープ外） | 保留（実装後ソーク待ち） | TBD |

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

**追記（2026-08-17、restore_roman 最終撤去）**: `is_roman_reliable=true` 限定に
反転した後の `restore_roman` は、唯一の本番呼び出し元が TsfNative 限定かつ
`is_roman_reliable=false` を常に渡すため**構造的に一度も発火しなかった**
（＝反転はしたが、実質「常に無効化」しただけで、条件を満たす経路自体が
存在しなかった）。さらに BUG-61（2026-08-09）の実機検証で、この復元が
仮に発火しても書き込み手段自体（IMC write・VK 注入）が Windows Terminal +
MS-IME で無反応と確定した。**「反転して安全な条件に絞ったつもりの機構が、
実は絞った時点で誰も呼べなくなっていた」ことに1ヶ月以上気づかなかった**のは、
死んだコードが「万一の保険」として心理的な安心材料になり続け、再点検の
動機を失わせた例。`docs/known-bugs.md` BUG-08 参照。

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

---

## エントリ 05: shift-eisu hold 入口のモードキー注入 — CapsLock 汚染で即日撤回

| 日付 | 仮説 | 環境（アプリ × IME × idle） | 変更 | 観測結果 | 判定 | コミット |
| --- | --- | --- | --- | --- | --- | --- |
| 2026-07-07 | 入口も scan 付き VK_DBE_ALPHANUMERIC+SBCSCHAR 注入なら入力キュー順序保証で初回文字の全角化を防げる | Windows Terminal × MS-IME（belief ON × 実 IME OFF の乖離窓） | 345086b で入口注入を追加 | **CapsLock が点灯**。F0 は scan 0x3A（物理 CapsLock 位置）で、実 IME OFF の文脈に着弾すると kbd106 の素の処理（CAPLOK）で CapsLock をトグルする | 撤回（入口は IMC write のみに復元、初回文字全角化は既知の限界として許容） | 345086b → 本 revert |

**学び**: IME モードキー（F0/F2/F3 等、物理キー位置と scancode を共有）は
「実 IME が確実に ON」でない限り注入してはならない。IME が処理しない文脈では
kbd106 の素のキー（CapsLock / かなロック / 半角全角）として作用し、
グローバルなキーボード状態を汚染する。belief は実状態の保証にならない。

---

## エントリ 06: BUG-15 hold 方式（Shift 押しっぱなし半角英数）の撤去 — 安全網とASCIIパススルーの分離が必要だった

**背景**: ユーザー要望（2026-07-11）で BUG-15 の「Shift 押しっぱなし中は半角英数」
（hold 方式）を「左Shift単独タップで持続トグル」方式へ置き換えることになった。
一見単純な UX 変更だが、設計検証で「hold 機構は安全網とASCIIパススルーの
2役を兼ねていた」ことが発覚し、片方だけ撤去する必要があった。

| 日付 | 仮説 | 環境（アプリ × IME） | 変更 | 観測結果 | 判定 | コミット |
| --- | --- | --- | --- | --- | --- | --- |
| 2026-07-11 | hold 機構全体（`kp_stage_shift_eisu_hold` 全体）を撤去し、左Shift単独タップ判定だけの新実装に置き換えれば良いはず | Windows Terminal × MS-IME（設計時点、実機未検証） | （設計レビュー段階で発覚、実装はしなかった） | 別エージェントによる設計レビューで「hold 機構は Shift+文字チョード時に MS-IME の単独タップ誤検知を無条件で打ち消す安全網でもある。全体を撤去すると `.yab` Shift 面のチョード（`'！'` 等）で BUG-15 の症状（数秒〜十数秒のかな入力破壊）がそのまま再発する」と指摘された | 撤回（設計段階、実装前に修正） | （設計変更、コミットなし） |
| 2026-07-11 | 安全網（Shift 押下→解放ごとの無条件 conv 書き戻し）は維持し、`shift_plane_halfwidth`（hold 中の ASCII パススルー）だけを撤去。左Shift単独タップ判定はこの安全網の上に「復元をキャンセルして持続トグルへ」という形で重ねる | 同上 | `kp_stage_shift_eisu_hold` → `kp_stage_shift_conv_guard` に改名・再構成。`shift_plane_halfwidth`/`ShiftEisuDisposition`/`KeyAction::Text` を削除 | 全 lib/golden/architecture_guard テスト green、clippy warning ゼロを確認（実機検証は未実施） | 採用（実機検証待ち） | （本セッションの一連のコミット） |

**学び**:

- 複数の目的を一つの機構（今回は「Shift 押下→解放ごとの conv 書き戻し」）が
  兼ねている場合、片方の目的（ASCII パススルー）を撤去する要望が来ても、
  もう片方の目的（MS-IME 単独タップ誤検知の安全網）まで一緒に消してはならない。
  「この機構は何のためにあるか」を実装コードだけでなく、関連する
  known-bugs.md のバグ本体の症状（今回は BUG-15 本体の「Shift単独タップ誤検知」）
  まで遡って確認する必要がある。
- 今回はコミット前の設計レビュー段階（Codex + Plan agent の2段階レビュー）で
  発覚したため、実機で症状を再現する前に設計を修正できた。パターンとしては
  「機能追加・削除の要望」が来たとき、対象コードの隣接する既存コメント
  （`kp_stage_shift_eisu_hold` の doc comment に「BUG-15 本体の誤発動問題も
  吸収される」と明記されていた）を読み飛ばさないことが重要。

---

## エントリ 07: BUG-25 GJI entry の scan 付き VK_DBE_ALPHANUMERIC 注入 — CapsLock 汚染で即日撤回

**背景**: BUG-25（左Shift単独タップ持続トグル）の GJI 向け entry 実装で、
既存の TSF warmup ヘルパー `send_vk_dbe_alpha_warmup` を standalone トグルへ
転用した。BUG-15 追補7（scan 付き `VK_DBE_ALPHANUMERIC` の CapsLock 汚染）を
知っていたため `effective_open()==true`（実 IME ON 確認済み）のガードを
入れていたが、それでも実機で再発した。

| 日付 | 仮説 | 環境（アプリ × IME） | 変更 | 観測結果 | 判定 | コミット |
| --- | --- | --- | --- | --- | --- | --- |
| 2026-07-11 | GJI 検出時は既存 TSF warmup 経路（scan 付き `VK_DBE_ALPHANUMERIC` 注入）を使えば、MS-IME 同様に半角英数へ切り替えられるはず。`effective_open()` ガードがあるので BUG-15 追補7の CapsLock 汚染は再発しないはず | Windows Terminal（`CASCADIA_HOSTING_WINDOW_CLASS`/`Windows.UI.Input.InputSite.WindowClass`、TSF-native）× GJI（Google 日本語入力） | `kp_shift_conv_guard_key_down` の entry に GJI 分岐を追加、`send_vk_dbe_alpha_warmup(HankakuAlpha)` を呼ぶ | ユーザー報告: 「IME ON / **CAPS LOCK ON** / awase engine OFF / ローマ字入力 / ひらがな」。診断ログ追加で確認: `gji_is_active_ime=true` で分岐は正しいが `SendInput sent=2/2`（OS的には成功）にもかかわらず `[hook] IME-mode vk=0xF0` のログが一切出ず、150ms後の conv も `0x00000019`（ひらがなローマ字）のまま無変化。scan=0x3A（物理CapsLock位置）がドライバレベルでCapsLockとして横取りされ、awase自身のフックにすら届いていないと判明 | 撤回（GJI分岐を削除、entry を GJI・MS-IME 共通の IMC write に一本化） | （本エントリ対応コミット） |

**学び**:

- `effective_open()`（belief 上の IME ON 確認）は、BUG-15 追補7が想定していた
  「実 IME が OFF の文脈」由来の CapsLock 汚染は防ぐが、**「対象 IME がこの
  単発注入をそもそも処理しない」由来の同一症状は防げない**。IME 種別（GJI vs
  MS-IME）ごとに実際に確認しないまま「実 IME が ON なら安全」と一般化しては
  ならない。
- `send_vk_dbe_alpha_warmup` は元々「直後に文字 VK を続けて送る」前提の
  NICOLA 内部 warmup ヒント（`send_vk_runs_with_leading_warmup` から呼ばれる
  charset 指定）であり、standalone の「IME モードを切り替えて維持する」用途
  では設計上の保証が無い。既存ヘルパーを別目的に転用する際は、その関数が
  「なぜ動いているか」（前提条件・呼び出しパターン）を確認してから流用する。
- `SendInput` の戻り値が成功（`sent=N/N`）でも、実際にターゲットアプリ/IME
  まで意図通り届いたとは限らない。`[hook] IME-mode ...` ログ（自己注入
  フィルタより前で無条件に出る）の有無を確認して初めて「フックまで到達したか」
  が分かる——ここが欠落すると OS レベルの scan コード横取りを見逃す。

## エントリ 08: BUG-25 GJI entry の IMC write 一本化 — 読み返し成功は偽陽性、mozc 本家調査で scan=0 注入へ

**背景**: エントリ07の撤回を受け、entry を GJI・MS-IME 共通の IMC write
（`set_ime_romaji_mode_with_target_async(Some(0))`）に一本化した。CapsLock
汚染は解消したが、GJI で実際に半角英数化されるかは「反映されない場合は機能
不全として残る」と留保していた。

| 日付 | 仮説 | 環境（アプリ × IME） | 変更 | 観測結果 | 判定 | コミット |
| --- | --- | --- | --- | --- | --- | --- |
| 2026-07-11 | IMC write は CapsLock を汚染しないので安全側。GJI で `success=true`・verify-read で `conv=0x00000000 NATIVE=false` が確認できれば半角英数化が反映されたと言える | Windows Terminal（TSF-native）× GJI（Google 日本語入力） | entry を GJI・MS-IME 共通で IMC write のみに一本化（`d39f56d`） | `success=true`、150ms後 verify-read で `conv=0x00000000 NATIVE=false` を確認。**しかし実際に「あいうえお」を打鍵するとひらがなが出力され、GJI の実コンポーザは切り替わっていなかった**（ユーザー報告「え？全然デキてないよ」）。mozc 本家ソース（`google/mozc`）調査により、conversion-mode compartment への書き込みは `win32/tip/tip_edit_session.cc` の `OnModeChangedAsync`（UI 表示同期のみ）を発火させるだけで、実コンバータへの `SendCommand(SWITCH_COMPOSITION_MODE)` は言語バークリックか本物のキー入力経路からしか呼ばれないことが判明——**GJI にとって IMC write は構造的に一方向の UI ミラーであり、read-back の成功は無意味**だと確定した | 撤回（GJI 分岐を復活させ、`make_key_input_ex` で scan=0 の `VK_DBE_ALPHANUMERIC` DOWN+UP を直接送る方式へ変更。MS-IME は IMC write のまま維持。実機未検証） | （本エントリ対応コミット） |

**学び**:

- **IMC read-back（`success=true` や verify ログ）を GJI の成否判定に使っては
  ならない。** 書き込みが UI ミラーに過ぎない以上、読み取りも「awase 自身が
  直前に書いた値をそのまま読み返しているだけ」になりうる。BUG-15 追補3
  （IMC read は実モードを保証しない）と同じ形の罠を、今回は write 側でも
  踏んだ——過去に文書化済みの教訓であっても、方向（read/write）が違うだけで
  同じ罠を再発見してしまう。**内部状態の読み取りだけで「直った」と判断せず、
  必ず実際の打鍵結果で確認する。**
- サードパーティ IME の外部制御を設計する際、公開 API（IMM/TSF compartment）
  が「効いているように見える」ことと「実際に効く」ことは別物であり、対象
  ソフトウェアのソースが公開されている場合はそちらで実装を確認するのが
  最も確実——mozc は OSS のため、今回 `win32/tip/` の実装を直接読むことで
  「compartment write は UI ミラー、実際の切り替えは本物のキー入力のみ」と
  いう構造を確定できた。同様の状況（サードパーティ IME/IMEの外部制御）では
  推測より先にソース調査を優先する。

## エントリ 09: BUG-25 GJI entry の scan=0 `VK_DBE_ALPHANUMERIC` 注入 — フックにすら届かず反証、entry 機構を全撤去

**背景**: エントリ08で IMC write が GJI に効かないと判明したため、mozc の
`keyevent_handler.cc` が scan を見ず VK 値のみで判定することを根拠に、
scan=0（CapsLock と衝突しない値）で `VK_DBE_ALPHANUMERIC` を再注入する方式
（`make_key_input_ex`）に切り替えた。

| 日付 | 仮説 | 環境（アプリ × IME） | 変更 | 観測結果 | 判定 | コミット |
| --- | --- | --- | --- | --- | --- | --- |
| 2026-07-11 | scan=0x3A（CapsLock位置）との衝突さえ避ければ、mozc は VK 値のみで判定するため scan=0 の VK_DBE_ALPHANUMERIC 注入は awase のフック・GJI の TSF キーイベントシンク双方に届くはず | Windows Terminal（TSF-native）× GJI | entry を `make_key_input_ex(VK_DBE_ALPHANUMERIC, .., scan=0)` の DOWN+UP 注入に変更（`6f0964b`） | `SendInput sent=2/2`（OS的には成功）。**しかし `[hook] IME-mode vk=0xF0` のログが今回も一度も出現せず**（同一セッション内で `VK_DBE_HIRAGANA` 0xF2/scan=0x70 は毎回確実に出現）、entry verify 前に engine が `Inactive(NotRomajiInput)` へ遷移し生ローマ字キーを GJI へ素通しした結果、GJI 自身の未切替のひらがな変換エンジンがそれを処理し「こんにちはあいうえお」がそのままひらがなで出力された。ユーザー報告「ダメでしたね」 | 撤回（GJI 向け entry を scan 値によらず全撤去。IMC write・scan付き注入・scan=0注入のいずれも試行済みで尽きたため、entry 機構自体を「未対応」として無効化し、`half_width_alnum_toggle_active` への遷移も GJI では起きないようガードを追加） | （本エントリ対応コミット） |

**学び**:

- **「scan の値を変えれば届く」という仮説は、scan=0x3A（衝突）→scan=0（非衝突）
  の2パターンで連続反証された。** `[hook] IME-mode vk=0xF0` ログが2回とも
  一度も出現しなかったことから、`SendInput` による `VK_DBE_ALPHANUMERIC`
  注入は scan の値によらず awase 自身の `WH_KEYBOARD_LL` フックにすら
  到達しないと判断するのが妥当。同じ変数（scan値）を変えた再試行を3回目も
  行うのではなく、**手段そのもの（`SendInput` によるキーイベント注入）を
  疑い、別の制御チャネル（COM の `ITfLangBarItemButton` 経由の言語バー
  ボタン起動等）へ切り替える**べき、という判断に至った。
- **entry が機能しない状態のまま belief だけを「トグルON」に進めると、
  「何も起きない」より悪い実害が生まれる。** engine が `Inactive` になり
  生キーを pass-through するが、GJI の実 conv は変化していないため、素通しした
  ローマ字キーが GJI 自身のひらがな変換エンジンにそのまま入り、意図しない
  ひらがな出力という**新しい種類の破壊**になった。機構が実証されるまでは、
  「何もしない」（機能を無効化する）方が「believe だけ進めて実害を出す」より
  安全側の設計判断である。
- 3回連続で同一の失敗ログシグネチャ（`[hook] IME-mode vk=0xF0` 皆無）が
  出た場合、それは「まだ運が悪い」ではなく「この経路は原理的に機能しない」
  という強いシグナルとして扱うべき——同種の変更をもう一段階小さくして
  再試行する前に、アーキテクチャレベルで別の経路を検討する。

---

## エントリ 10: GJI cold-start warmup の「待機行列」「捨て駒キー」撤去 — per-VK confirm 一本化

**背景**: BUG-24（`is_partial_literal()` が romaji 自体の compose 結果ではなく、
別の warmup F2 キーへの応答 `nc_fired`/`gji_resumed` を代理指標にしている）の
根治として per-VK confirm（1文字ずつ送信→confirm、失敗時は backspace のみで
回収）を導入した後、旧来の「待機行列」（`WarmupKind::FreshF2`/`ReWarmup`/
`ProbeWithSettle`、`ColdReason`×`long_idle` の `eager_settle_ms`/`probe_min_ms`
行列）と「捨て駒キー」（`StartSacrificialWarmup`/`SacrificialResend`、
`SacrificialWarmupCoro`/`ImeOffOnWarmupFsm`）が per-VK confirm と二重の保険に
なっているのではないか、という仮説を `experiment/skip-cold-probe-wait`
ブランチで検証した。

| 日付 | 仮説 | 環境（アプリ × IME × idle） | 変更 | 観測結果 | 判定 | コミット |
| --- | --- | --- | --- | --- | --- | --- |
| 2026-07-16〜17 | per-VK confirm が送信後の confirm/recovery を担うなら、送信前の予防的待機（F2 事前送信・probe 事前待機）は不要なはず | WezTerm（TSF-native）× GJI、Chrome × GJI | `DIAG_COLD_SKIP_F2`/`DIAG_COLD_SKIP_PROBE_WAIT`（WezTerm 側）・`DIAG_CHROME_SKIP_F2`/`DIAG_CHROME_SKIP_PROBE_WAIT`/`DIAG_CHROME_SKIP_SACRIFICIAL_WARMUP`（Chrome 側）を新設しデフォルト全 `true` で実機投入 | 24時間弱のソークで BUG-26〜29（本リポジトリ known-bugs.md）を発見・修正しつつ、無破損を確認 | 保留（さらに広い条件で継続ソーク） | `d495649` 直前の一連のコミット群 |
| 2026-07-18 | 上記フラグを恒久化し、待機行列・捨て駒キー機構を物理削除しても安全なはず | WezTerm/Chrome 双方 × GJI | 上記実験フラグをすべて恒久化。`WarmupKind::*`・`SacrificialWarmupCoro`・`ImeOffOnWarmupFsm` を物理削除し、`GjiWarmupCoro::run_start` を「IMM32 ローマ字モード復元 + 即座に per-VK confirm へ」の単一経路に単純化 | 数日間の実機ソーク（cold=61〜74 超、WezTerm/Chrome 双方）で `suspected literal` genuine ゼロ件を `per-VK[...] confirmed` の3点セットログで確認。cargo check/test/clippy（`--target x86_64-pc-windows-gnu`、警告ゼロ）、Linux 上の `cargo test -p awase-windows`（174 passed）も通過 | 採用（物理削除） | `d495649`（詳細は `docs/known-bugs.md` BUG-24 追補8） |
| 2026-07-19 | 上記の物理削除の副産物として、observation/decision/belief 側にも本番到達不能なコードが残っているはず | （コード調査のみ、実機検証なし） | codex CLI 2プロセス（read-only、候補検証+独立発見）による調査 + Claude 自身の裏取りで `ProbeObservations.gji_resumed`（常に false）・`DIAG_FORCE_HIRAGANA_CHARSET`（無配線）・`TsfReadinessProbe::wait_until_ready`（本番呼び出しゼロ）・`GjiWarmupCoro` の `needs_settle_check`（常に true）を確認、`DIAG_DISABLE_PROACTIVE_TSF_WARMUP` はユーザー判断で恒久化 | cargo check/clippy（`--target x86_64-pc-windows-gnu`、警告ゼロ）で確認。wine 未導入のためこのサンドボックスでは `cargo test --target x86_64-pc-windows-gnu` 実行不可（実機/CI 確認が最終）。`TsfReadinessProbe::check_now` の min_ms/total_max_ms 分岐は「本番が現状 0 を渡しているだけ」で静的には unreachable でないため削除せず据え置き | 採用（削除分）／保留（check_now） | 本エントリ対応の一連のコミット（BUG-24 追補9） |
| 2026-07-19 | 追補9が残した「未調査」項目（`WarmupOutcome.prepend_f2_warmup` 等）を含め、GJI probe/warmup 関連変数を網羅的に洗い出せば追加の dead code が見つかるはず | （コード調査のみ、実機検証なし） | 5並列エージェントで GJI probe/warmup 関連変数を全域洗い出し（一次調査）→ 9並列 opus エージェントで各候補を反証前提に個別再検証（二次調査）。`WarmupOutcome.prepend_f2_warmup`・`PendingInput.deferred_vks`・`WarmupResult`/`GjiAction::SendInput.result`・`gji_read_op_count`/`gji_read_bytes`・`ColdContext::set_idle_ms_at_last_cold`・`ColdContext::cold_marked_ms`・`TickableFsm::notify_start_composition` の7件を DEAD 確定・物理削除。`TsfReadinessProbe::check_now` の min_ms/total_max_ms 分岐は独立 opus エージェントでも再度反証できず、追補9の据え置き判断を維持 | 削除7件それぞれで `cargo check`/`cargo test --no-run`（`--target x86_64-pc-windows-gnu`、警告ゼロ）を実行、最終確認は `cargo cc`（プロジェクト規定 clippy エイリアス）で warning ゼロ。wine 未導入のためこのサンドボックスでは実行不可（実機/CI 確認が最終） | 採用（削除7件）／据え置き再確認（check_now） | 本エントリ対応の一連のコミット（BUG-24 追補10） |
| 2026-07-19 | 追補10でもかなり枯れたはずだが、GJI cold/warm 周りにまだ撤去可能な変数が残っていないか（ユーザー確認） | （コード調査のみ、実機検証なし） | 単一 opus エージェントで同じ一次洗い出し→二次反証の手法をもう一段実施。孤児アクセサ `gji_last_write_ms()`/`gji_write_bytes()`（レシーバ形、呼び出しゼロ）と、log-only 化していた `GJI_LONG_IDLE_PROBE_TOTAL_MS`→`ColdKind::budget_ms()`→`StartProbe.budget_ms` チェーン一式（NameChangeWait 撤去+skip-cold-probe-wait 恒久化の結果どのタイマーも支配しなくなり debug ログにしか使われていなかった）の2件を DEAD 確定・削除。`should_prepend_f2`/`used_eager_path`/`ime_show_seq`/`SendInput` mirror 等4件は意図的残置として再確認・据え置き | `cargo check`/`cargo clippy -p awase-windows --target x86_64-pc-windows-gnu --lib -- -D warnings`/`cargo test --no-run`（警告ゼロ）、Linux で `cargo test -p awase-windows --lib`（135 passed）+ architecture_guard/golden_scenarios/ime_key_sequence_golden/layer_boundary_guard 全 green | 採用（削除2件）／据え置き再確認（4件） | 本エントリ対応の一連のコミット（BUG-24 追補11） |

**学び**:

- 予防的待機・捨て駒キーのような「二重の保険」は、reactive な回収機構
  （per-VK confirm）が実証された後も惰性で残りがち。恒久化の判断は
  数日単位の実機ソーク（cold=60件超）を経てから行い、`docs/known-bugs.md`
  に実測件数を残すことで次の担当者が根拠を追える。
- 削除は必ず段階を踏む: (1) 実験フラグで無効化 → 実機ソーク → (2) 恒久化 →
  物理削除 → (3) 恒久化の副産物として残った到達不能コードを別途調査。
  一足飛びに (1)→(3) をやると「何が本当に安全に消せるか」の根拠が薄くなる。
- 「静的に到達不能」（コンパイラ/型で保証される dead code）と「今たまたま
  実行時値が 0/false」は別物として扱う。前者は安全に削除できるが、後者
  （`TsfReadinessProbe::check_now` の待機ロジック等）は将来また非ゼロの
  値が必要になり得るため、同じ調査パスに乗せて安易に削除しない。

---

## エントリ 11: `InjectionMode` per-VK 統一構想 — HIMC 照合の観測フェーズ（事前登録）

**背景**: ADR-081 Phase 1d 検討から派生し、「GJI がアクティブなときは profile を
問わず per-VK（1キーずつ確認しながら送る）方式に文字送信を統一したい」という構想が
浮上した（詳細は [ADR-083](adr/083-injection-mode-per-vk-unification-investigation.md)）。
Opus・Fable・Codex の3系統独立レビューの結果、統一自体は BUG-45（per-VK confirm の
構造的欠陥）が未解決のため NO-GO と判定されたが、統一の鍵となる HIMC 直接照合
（`capture_composition_snapshot`、`ime.rs:1124`）は実装済みで判定点に配線済みながら
判断には未使用と判明した。これを Standard/ImmCross プロファイル（LINE 等）で実機
検証する前段階として、**判定ロジックを一切変更しない観測専用ログ**を追加した。

**このエントリは事前登録**（測定前に合格基準を書く、`.claude/rules/tuning-constants.md`
の実測義務の精神を診断フェーズにも適用）。以下の基準を実機ログ収集後に照合する。

| 項目 | 合格基準（案） | 意味 |
| --- | --- | --- |
| `comp_str` 非空率 | Unicode 注入直後 100ms 以内に ≥ 95% で非空 | HIMC 照合が LINE で「読める」証拠になるか |
| `himc_null` 発生率 | ほぼ 0%（LINE 等 IMM32 互換アプリの場合） | HIMC 自体が取得できない＝TSF ネイティブと同型の失敗（2026-05-15 撤回, `558c39f`→`b643bac`）の再演でないか |
| `capture_composition_snapshot` 所要時間 | p99 < 5ms | `ImmGetContext` 系のブロッキングリスクが顕在化しないか |
| `comp_read_str`（読み）と送信ローマ字の一致率 | 定量化できれば記録（合格基準は測定後に精緻化） | HIMC 照合を将来の判定ロジックに使う場合の信頼性の目安 |

| 日付 | 仮説 | 環境（アプリ × IME × idle） | 変更 | 観測結果 | 判定 | コミット |
| --- | --- | --- | --- | --- | --- | --- |
| 2026-08-03 | HIMC 照合（`capture_composition_snapshot`）は Standard/ImmCross プロファイル（LINE 等）でも意味のある値を返すはず（TSF ネイティブアプリ限定で過去に失敗した `558c39f`→`b643bac` とは異なる組み合わせ） | LINE 等 ImmCross × GJI（実機未実施） | `UnicodeLiteralObserverFsm::tick` の判定確定点に `log_composition_probe` を1行追加（判定ロジックは無変更） | 実機ログ収集待ち | 保留（観測専用パッチのみ投入、判定はソーク後） | （本ブランチのコミット、後日追記） |

**学び（暫定）**:

- HIMC ベースの composition 検出は、過去に TSF ネイティブアプリ（WezTerm）で
  一度失敗しているが、これは HIMC が取得できない（またはゼロを返す）アプリ種別
  固有の失敗であり、IMM32 互換アプリでの妥当性を否定するものではない。
  「過去に似た名前の実験が失敗した」という理由だけで再挑戦を諦めないよう、
  失敗条件（アプリ種別）を正確に切り分けて記録することが重要。

---

## エントリ 12: `conv_mode_policy = force` の FocusChange 強制書き込みを MS-IME にも配線（BUG-59 追補）— 実機未検証のまま投入し翌日 revert

**背景**: `conv_mode_policy = force`（[ADR-085](adr/085-conv-mode-force-policy.md)）は
GJI の cold 転換（`cold_warmup.rs::run_start`）でしか `desired_mode` を強制していな
かった。`MsImeStrategy::needs_f2_probe()` が常に `false` のため MS-IME では一度も
発火しない構造的な穴があり、これを埋めるために `platform.rs::gji_on_focus_change`
に「FocusChange のたびに MS-IME へも強制書き込みする」ロジックを追加した。

| 日付 | 仮説 | 環境（アプリ × IME × idle） | 変更 | 観測結果 | 判定 | コミット |
| --- | --- | --- | --- | --- | --- | --- |
| 2026-08-07 | FocusChange 契機で MS-IME にも `desired_mode` を強制書き込みすれば、カタカナ固着等の drift を手動リセットなしで自動回復できるはず | Windows Terminal（TsfNative）↔ LINE（Qt/ImmCross）往復、`conv_mode_policy=force` 試験運用中 | `gji_on_focus_change` に `forced_target` 計算 + `set_ime_romaji_mode_with_target_async` 呼び出しを追加（世代カウンタで陳腐化チェックのみ） | LINE で全打鍵が「い」になる／IME が JIS かなになる（実機報告、2026-08-08）。書き込み先 hwnd を実行時のライブクエリで決めるため、非同期の間隙でフォーカスが移ると無関係な別ウィンドウへ誤爆する競合状態があった（[ADR-086](adr/086-force-write-trigger-and-target-identity.md) §1.2 欠陥1で確定） | 撤回（revert） | `9c102b02`（投入）→ `9b44f045`（revert） |

**学び**:

- 「MS-IME で発火しない」という穴の指摘自体は正しかったが、直し方（生の
  `FocusChange` イベントを直接トリガーにする）が、書き込み先ウィンドウの
  確からしさを壊した。ADR-085 の元設計（GJI 側）は「実際にキー入力を処理
  しようとした瞬間」というユーザー入力に紐づくトリガーだったため、この
  問題が顕在化していなかった。トリガーを「観測イベント」から「入力意図」に
  切り離すと、対象の妥当性まで一緒に失われることがある。
- 実機未検証のまま `develop` にマージし、翌日ユーザー実機で発覚した。
  「実機ソーク未実施」と明記していても opt-in 設定の試験運用者は実際に
  被弾する。恒久対応は [ADR-086](adr/086-force-write-trigger-and-target-identity.md)
  Phase 2（arm-on-focus / fire-on-intent）に委ねた。

---

## エントリ 13: `reschedule_ime_refresh` の force_policy 例外を撤去→復元（**実機未確認・コード読解による判断**）

**背景**: [ADR-086](adr/086-force-write-trigger-and-target-identity.md) Phase 3
実装時、`apply_force_on_for_imm_broken`（force-ON）のトリガーを周期リフレッシュ
からキー入力直前へ移した。周期経路に相乗りしていた force_policy 例外
（2026-08-06 追加）は「force-ON の周期再送のためだけに存在する」と判断し、
Phase 3 と同一コミットで撤去した。

**訂正の経緯**: Phase 3 実装完了直後の2回目 opus アドバーサリアルレビューで、
この撤去が `ir_apply_drift_correction`（BUG-20 が追加した non-ImmCross/TsfNative
向け分岐）の周期実行機会も巻き添えで奪っていたと**コード読解で**指摘された。
**実機での再現・実測は行っていない**——以下は「コードを読んだ結果、force-ON
以外にもこの周期チェーンに依存する経路があると判明した」という静的解析上の
訂正であり、`.claude/rules/experiment-logging.md`/`.claude/rules/tuning-constants.md`
が求める実機実測とは性質が異なる。

| 日付 | 仮説 | 環境（アプリ × IME × idle） | 変更 | 観測結果 | 判定 | コミット |
| --- | --- | --- | --- | --- | --- | --- |
| 2026-08-08 | force-ON がキー入力直前トリガーへ移行した以上、`reschedule_ime_refresh` の force_policy 周期継続例外は不要なはず | TsfNative（Windows Terminal 等）× `conv_mode_policy=force`（実機未実施、コード読解のみ） | `reschedule_ime_refresh` の force_policy 早期 return スキップ例外を撤去 | 実機未検証。コード読解で `ir_apply_drift_correction` の non-ImmCross 分岐（BUG-20）も同じ周期チェーンに依存しており、撤去すると TsfNative × force policy で drift correction の周期実行機会が失われると判明 | 復元（例外を戻す）。ただし「force policy ユーザーだけが周期 drift correction を持つ」という新たな非対称が残る（ADR-086 §7-12 に未解決論点として起票） | Phase 3 実装コミット群 → 本訂正コミット |

**学び**:

- 「この例外は force-ON のためだけに存在する」という判断は、例外条件
  （`is_force_policy()`）が実際に守っているコードパスをすべて洗い出さずに
  下してしまった。1つの条件式が複数の目的（force-ON の周期再送 / drift
  correction の周期実行機会）を偶然同時に満たしていることがあるため、
  ガード条件を撤去する前に「このガードで守られている経路は他にないか」を
  網羅的に確認する必要がある。
- 実機が使えないサンドボックスでの開発では、この種の「コード読解による
  巻き添え発見」を実測と混同せず、別カテゴリとして記録することが重要

## エントリ 14: tray の「ローマ字」「かな」コマンド（`ImmSetConversionStatus` write + `VK_DBE_ROMAN`/`NOROMAN` 注入の併走）— 実機で無反応、撤去して Ctrl+Alt+R/K ホットキーへ転換

**背景**: BUG-61（Windows Terminal + MS-IME で JIS かな入力に固定され復旧
不能）の調査で、tray の「ローマ字」「かな」コマンドに `VK_DBE_ROMAN`/
`VK_DBE_NOROMAN` の scan コード付き SendInput を、既存の `ImmSetConversion
Status` write と併走で追加した（Opus 設計相談 + Fable PM プランニング）。
Opus アドバーサリアルレビューで「tray は `WM_COMMAND` 発火時点でフォーカスを
自分自身に奪っており SendInput が届かない」という Critical 指摘を受け、
`SetForegroundWindow` による対象復元 + 検証を追加した上でユーザーに実機
確認を依頼した。

| 日付 | アプリ | IME/状態 | 再現手順 | 観測結果 | 判定 |
| --- | --- | --- | --- | --- | --- |
| 2026-08-09 | Windows Terminal | MS-IME、JIS かな入力に固定（`conv_mode_policy=force` ソーク中に発生） | tray メニューから「ローマ字」「かな」を選択（フォーカス復元修正済みの版） | **押しても何も変化しない**（IMC write・VK 注入いずれも無反応） | tray コマンドを撤去。tray 経由の交絡（メニュー表示自体のフォーカス遷移・IMC write との併走）をすべて排した Ctrl+Alt+R/K ホットキーへ転換 |
| 2026-08-09 | Windows Terminal（同一セッション、フォーカス継続） | MS-IME、JIS かな入力に固定（同上） | Ctrl+Alt+R（`VK_DBE_ROMAN` 単体注入、IMC write 併走なし）/ Ctrl+Alt+K（`VK_DBE_NOROMAN`）をそれぞれ押下 | ログ上は送信を確認（`vk=0xF5`/`0xF6` の KeyDown/KeyUp が発火し `may_change_ime`→IME refresh スケジュールまで到達）できたが、**その後も `conv=0x00000009` が一切変化しない** | **IMC write に続き VK 単体注入でも無反応と確定**。tray 経路の交絡（フォーカス奪取・IMC 併走）を排除した上での結果のため、「tray 特有の問題」という説明は完全に棄却される |

**判定の理由**: フォーカス復元は修正済みだったため、C1（tray がフォーカスを
奪う問題）だけが原因ではない。tray 経路に残っていた別の交絡（IMC write との
併走で VK 単体の効果が隠れる、メニュー表示自体が TSF 側に副作用を起こす等）
を疑い、通常のキー処理経路（`handle_wm_key_from_hook`）で発火し IMC write を
一切併走させない Ctrl+Alt+R/K ホットキーに切り替えた。**この版でも無反応
だったことから、VK_DBE_ROMAN/NOROMAN 自体が Windows Terminal + MS-IME の
実際の conv モードに一切作用しないと確定した**（BUG-61 参照）。

**学び**:
- SendInput 系の実機テスト機構を tray（メニュー・モーダルループ）に載せると、
  フォーカス奪取以外にも見えない交絡が残りうる。通常のキー処理経路（物理
  キー押下と同じ文脈で発火する）の方が交絡が少なく、実機での切り分けに
  向いている。
- **awase が持つ2つの conv-mode 制御手段（`ImmSetConversionStatus` write・
  DBE 系 VK 注入）は、Windows Terminal + MS-IME の JIS かな固定に対して
  いずれも無力**と確定した。この症状に対する復旧手段は、この2つの延長線上
  にはない（is_roman_reliable の解除や自動発火の配線をしても効果は無い）。
  次にこの症状の復旧を試みる際は、Windows 言語バーの手動操作や IME
  コンテキストの完全な再初期化など、awase の外側の手段から検討すること。
- 次に「tray や VK 注入で IME 制御を試す」という着想が浮かんだときは、
  まずこのエントリと BUG-61 を確認すること（本エントリはそのための記録）。

---

## エントリ 15: shift-conv-guard のチョード安全網（BUG-15/25）を撤去 — LINE 全角記号の半角化と BUG-58 フリーズの共通の引き金だった

**背景**: ユーザー報告「LINE で `Shift+1`（`'！'`）を打つと全角のまま Unicode
注入しているのに半角 `!` で表示される。Windows Terminal では起きない」。ログ
突合の結果、`kp_shift_conv_guard_key_down` が判別未確定のまま Shift+文字キーの
チョードすべてに対して conv=0x0000（IME-ON 半角英数）を先書き込みし、
`Char('！')` の送出がその窓の中で起きていたことを特定。BUG-25 で ASCII
素通し経路（`shift_plane_halfwidth`）を撤去して以降、チョードの出力は
`shift_face_reduce` の Unicode 直接注入のみになっており、この先書き込みは
出力そのものには不要と判明。BUG-58（同じ先書き込みが引き金の ~5 秒フリーズ）
も踏まえ、ユーザー判断でチョード安全網自体を撤去（持続トグルは維持）。

| 日付 | アプリ | IME/状態 | 再現手順 | 観測結果 | 判定 |
| --- | --- | --- | --- | --- | --- |
| 2026-08-09 | LINE（`Qt663QWindowIcon`、Qt/ImmCross） | MS-IME、ひらがなローマ字入力中 | `Shift+1`（`.yab` `'！'`）を打鍵 | awase ログでは `Char('！') via Unicode` で全角送出済みなのに LINE 表示は半角 `!` | 未撤回（本エントリの対応で撤去、実機再検証待ち） |
| 2026-08-09 | Windows Terminal（同一ユーザー確認） | MS-IME、TSF-native | 同じ `Shift+1` チョード | 全角 `！` のまま正常表示（症状なし） | LINE 固有の再現条件と確定、対応の方向付けに使用 |

**対応:** `kp_shift_conv_guard_key_down`（`runtime/key_pipeline.rs`）から
conv=0x0000 の先書き込みを撤去し、左Shift単独タップの持続トグル（BUG-25）の
entry write は単独タップと確定した瞬間（`kp_shift_conv_guard_key_up`）へ
移動。詳細は [docs/known-bugs.md BUG-15 追補9](known-bugs.md) 参照。

**学び:**
- 「MS-IME 誤検知への安全網」のような防御的コードは、それを要求した元の機能
  （ASCII 素通し）が撤去された後も惰性で生き残り、無関係な副作用（LINE の
  幅正規化との衝突、BUG-58 のフリーズ）の温床になり得る。防御コードを追加
  した理由が後から無効化されていないか、機能撤去のたびに確認する価値がある。
- 同じ「Shift+文字チョードのたびに conv を先書き込みする」1 箇所の実装が、
  見た目の異なる2つのバグ（LINE の幅、BUG-58 のフリーズ）の共通の引き金
  だった。症状が別アプリ・別現象でも、書き込みタイミングという共通の原因を
  疑う価値がある。
- 撤去により BUG-15 本体（MS-IME 自身の Shift 単独タップ誤検知）への先回り
  対策が失われた点は未検証のリスクとして known-bugs.md に明記した。次に
  チョード直後のかな入力破壊が再発したら、まずこのエントリと BUG-15/BUG-25/
  BUG-58 を確認すること。

---

## エントリ 16: GJI eager warmup キーを `VK_DBE_HIRAGANA` から `VK_IME_ON` へ置き換えられないか（BUG-69/ADR-098 決定3-c、[ADR-100](adr/100-gji-warmup-vk-ime-on-reinit.md) が正式に引き取り済み・**2026-08-22、群Bの結果をもってユーザー判断により本採用（実装済み）**。群Cは対象外・別課題として残存）

**背景**: BUG-69（`docs/known-bugs.md`）/ [ADR-098](adr/098-tsfnative-applied-confirmed-laundering-and-force-on-removal.md)
決定3 の調査で、eager TSF warmup（`send_eager_tsf_warmup`、
`output/mod.rs`）が `send_vk_dbe_hiragana_pair` 経由で物理かなキー位置
（scan=0x70）付きの `VK_DBE_HIRAGANA` を送信していることが判明した。
`ime_controller.rs` のコメントは `VK_DBE_HIRAGANA` が「IME を開く」と
「ひらがなに強制する」を1つの副作用に束ねていることを明記しており
（BUG-50 デッドロックの直接の前提。MS-IME 側の ON キーは同じ理由で
2026-08-06 に他キーへ移行済み）、BUG-15 追補7 は「IME モードキーの注入は
実 IME が確実に ON でない限りしてはならない」とこの注入パターン自体の
危険性を警告している。ADR-098 決定3 は現状の eager warmup を KEEP（他の
2機構と違い唯一生きている実効的な cold-start 対策のため）とした上で、
将来的に `VK_IME_ON`（open のみ、conv には触れない）へ置き換えられれば
BUG-50 系のリスクを構造的に消せるのではないか、という代替案を残した。

**背景の訂正（ADR-100）**: ADR-098 決定3 の原文は eager warmup 撤去の
被害例として「Chrome の BUG-02 リテラル化」を挙げているが、これは誤りと
判明した。eager warmup は `InjectionMode::Tsf` のときしか発火せず、
Chrome/Edge（`AppKind::TsfNative` → `InjectionMode::Vk`）は対象外である
（`UnicodeLiteralObserverFsm` の実行時学習も `Unicode` モード限定なので
Chrome が事後的に `Tsf` へ昇格することもない）。実験対象は正確には
「config `app_overrides.force_tsf` に登録された、または実行時学習で
`Tsf` へ昇格したアプリ」という可変集合であり、典型的には WezTerm /
Windows Terminal が該当するが、実験前に `[tsf-eager-warmup]` ログで
実対象を特定すること（実対象が無ければ本実験は空振りになる）。

**未実施の理由**: `VK_IME_ON` が TSF composition context の cold-start
（BUG-02 系）を `VK_DBE_HIRAGANA` と同等に解消できるかは実機での検証が
必要で、ADR-098 のスコープ外として先送りした（decision1〜2 の本体修正を
優先）。ADR-100 はこの宿題を引き取り、**「`VK_DBE_HIRAGANA` を維持したまま
`VK_IME_OFF→VK_IME_ON` トグルへ拡張する案」と「give-up 分岐に confirm 後
retry を追加する案」の2つのユーザー提案を検討したうえで両方却下し**、
本エントリが元々想定していた縮小版（`VK_IME_ON` の**単発**送信）だけを
実験として存続させた。却下の詳細は ADR-100 決定1・決定3 を参照。

**このエントリは事前登録**。実機実験に着手する際は以下を測定・記録すること:

| 群 | 送信内容 | 項目 | 合格基準（案） | 意味 |
| --- | --- | --- | --- | --- |
| A（現行） | `VK_DBE_HIRAGANA` 単発（scan 実値 + `TSF_MARKER`） | 対照 | — | ベースライン |
| B | `VK_IME_ON` 単発 | 初回入力がリテラル化しないか | A と同等（置換前後で**入力結果の文字列が一致**すること。「conv が変わらない」では不十分——「今まで寄せてくれていたものが寄らなくなる」形の劣化を見逃す） | cold-start 対策として代替になるか |
| B | 同上 | conv モード（かな/ローマ字）への意図しない影響 | 無し（入力結果の文字列比較で判定） | `VK_DBE_HIRAGANA` が持つ「ひらがなに強制する」副作用が消えることの確認 |
| B | 同上 | BUG-50 系デッドロック（カタカナロックイン）の再現有無 | 再現しない | 置き換えの本来の目的（危険な副作用の除去）が達成されているか |
| B | 同上 | `VK_IME_ON` 送信後に `gji_write_bytes` が上昇するか | 上昇する（`send_unicode_cold_warmup_keys` の犠牲キー設計が「単発では上がらない」ことを示唆しているため、まずこれを確認する） | cold-start トリガーとして機能しているかの直接証拠 |
| C | 何も送らない（eager warmup 無効化） | 初回入力がリテラル化するか | — | ADR-098 決定3 の前提（eager warmup が「唯一生きている実効的な cold-start 対策」）自体の検証。A/B と比較する対照群 |
| — | — | `MapVirtualKeyW(VK_IME_ON=0x16, MAPVK_VK_TO_VSC)` の戻り値 | 非ゼロなら scan 付き送信、0 なら scan=0 で試す | 送信形態（`wScan`/`dwExtraInfo` マーカー）は独立変数。エントリ09（scan=0 の `VK_DBE_ALPHANUMERIC` 注入がフックにすら届かず反証された事例）を踏まえ、否定的結果が「`VK_IME_ON` が効かない」なのか「scan/marker の組み合わせが悪い」なのかを事後に分離できるよう、どの組み合わせを試したか実験ログに明記すること |

**既知の否定寄りの示唆（ADR-100 F13）**: 本番コード
`Output::send_unicode_cold_warmup_keys`（`output/mod.rs:312-344`、Unicode
long-cold 経路）は既に `VK_IME_ON` 単発を送っているが、その直後に
`VK_A + VK_BACK` の犠牲キーを追加送信している。doc コメントは「`VK_A` が
GJI の hiragana composition を起動して `gji_write_bytes` を増やす」と
書いており、これは「`VK_IME_ON` 単発だけでは composition が起動しない」
可能性を示唆する（確定ではない——犠牲キーが warm 手段なのか単なる観測
手段なのかはコードからは決着しない）。案A（`VK_IME_ON` 単発）が不合格
だった場合の次手として、**案A'（`VK_IME_ON` + 犠牲キー、ADR-048
SacrificialWarmup と同型）を保持すること**。「`VK_IME_ON` 単発が駄目
だったから `VK_IME_ON` 系は全部駄目」と一般化しないこと。

**提案1（`VK_DBE_HIRAGANA` → `VK_IME_OFF→VK_IME_ON` トグルへの拡張）の
却下記録（ADR-100 決定1）**: (a) 置換先の `send_chrome_gji_reinit_and_poll`
は今日「Unicode long-cold」と「literal give-up」という低頻度イベントでしか
撃たれておらず、それを確定キー・Ctrl 解放・再注入のたびに撃たれる高頻度
パス（eager warmup）へ移すのは頻度差が大きすぎる。(b) `VK_DBE_HIRAGANA`
単発は composition を閉じない冪等操作だが、`VK_IME_OFF→VK_IME_ON` は
composition を一度閉じる破壊的遷移で未確定 preedit を commit する
（BUG-36 で確定）。eager warmup の呼び出しサイトには composition の
直後でありうるものが含まれ、この前提が成り立たない。(c) confirm
（IMC ポーリング）が読める保証が無い（TSF ネイティブで読めた実機事例は
1件あるが頻度は未測定）。(d) 目的（`VK_DBE_HIRAGANA` の conv 副作用の
除去）はより安い手段（本エントリの案A/A'）で達成できる。

**提案2（give-up 分岐への confirm 後 retry 追加）の却下記録と代替策
（ADR-100 決定3）**: 却下理由は「retry という発想が悪い」からではなく
「`send_chrome_gji_reinit_and_poll` のポーリングに完了通知の経路が
存在せず、`confirmed` が立たない環境では実質 300ms のタイマー待ちに
劣化する」ため。give-up 到達時の文字消失は推測ではなく3件記録済みの
実害（BUG-16 追補3・BUG-38/39 追補2・BUG-45）であり、却下するだけでは
代替策が無いまま終わる。そこで**案L（give-up 分岐で捨てた romaji を
journal へ記録する。送信ゼロ・挙動変更ゼロ）を採用**し、**案J（Unicode
直接送信への退避）・案K（backspace も打たない）は却下せず保持**した。
プライバシー方針: 案L は journal（既に `attach_log` チェックボックスの
opt-in 配下）へ生の romaji を記録する。新しい送信チャネルは開かない。

**2026-08-24追補（ADR-101 / BUG-74）**: BUG-74の実機ログで、give-up後の
reinitが直後の「う」を自然に成功させたことから、失われた「こ」も通常送信経路へ
戻すべきだと判断した。ただしADR-100時点の提案2をそのまま復活させず、4ラウンドの
premortemで、focus世代照合欠落(F6)、`with_app`内送信によるpost-send effects漏れ、
retry待ち中の `pending_deferred` 追い越し、連続give-upによるguard奪取、
`SuppressedExistingPoll` の遅延backspaceが既存retry後の文字を消す問題を潰した。
最終設計は [ADR-101](adr/101-bug74-giveup-retry-with-focus-guard.md) として実装済み。
retryは `send_romaji_batched` / `send_romaji_as_tsf` の通常経路へ1回だけ戻し、
Unicode直接送信の新経路は作らない。

**学び（暫定）**: 「唯一生きている機構だから触らない」という判断（ADR-098
決定3 KEEP）と、「触るなら安全な代替キーに変えたい」という改善方向は両立
する。前者は BUG-69 修正のスコープ、後者は実機検証を要する別トピックと
して分離して記録することで、どちらも見失わずに残せる。

**追加の学び（ADR-100 起票で得たもの）**: 既存機構の再利用に見える提案
でも、その機構が今日どの頻度で・どのモードで撃たれているかを先に数える
こと。「使われていない」と結論する前に、その機構が出すログ文字列の唯一の
出力元を grep で確かめること——ADR-100 の初稿は `[gji-coro]`/`[h1-warmup]`
の出力元を確かめずに「Tsf モードでは一度も撃たれていない」と書き、自分が
同じ ADR 内で引用していた BUG-45 のログと矛盾した。さらに「この経路は
必ずモード X 限定である」と書く前に、その関数の呼び出し元を grep で
全数数えること。分岐条件が `injection_mode` 以外の軸（`tsf_gate` 等）に
載っている呼び出し元が混じっていることがある（ADR-100 は同型の「モード
分割の言い切り」を3回間違えた）。`AppImeProfile` / `AppKind` /
`InjectionMode` / `TsfGateState` は4つの独立した軸であり、どれか1つで
語ると必ずどこかが漏れる（ADR-083 の教訓と同型）。

### 実施記録 #1（2026-08-22、群C、交絡あり・参考記録のみ）

**アプリ**: Windows Terminal（`CASCADIA_HOSTING_WINDOW_CLASS`、TsfNative）。
**IME**: GJI、Engine ON、cold=1〜8、`gji_idle_ms` 最大 85687（約85.7秒）。
**手順**: `send_eager_tsf_warmup` の**冒頭**（3ゲート判定より前）に
`AWASE_DIAG_DISABLE_EAGER_WARMUP` 環境変数による診断ゲートを追加した診断
ビルドを、Windows実機（dragonflyg4）で `RUST_LOG=debug` 付き起動し、通常
入力・長時間放置後の入力を実施。

**結果**: 8回のcold-start全件で per-VK confirm が正常に confirmed へ到達、
`giving up` は0件。

**結論: 不確定（採用しない）。** ゲートを3ゲート判定の前に置いたため、
「本来送るはずだった F2 を阻止した」場合と「そもそも既存ゲートで弾かれる
はずだった」場合が同一ログになる交絡がある。8回のうち何回が実際に
eager warmup を阻止したケースだったか、事後に分離できない。

### 実施記録 #2（2026-08-22、群C、交絡解消後）

**アプリ**: Windows Terminal（`CASCADIA_HOSTING_WINDOW_CLASS`、TsfNative）。
**IME**: GJI、Engine ON。
**手順**: 診断ゲートを3ゲート判定の**後**（`can_warmup()` 通過後）へ移動
し、`[diag]` ログが「本来なら送信していたはず」の場合のみ出るよう修正。
以下3シナリオをそれぞれ最低1回実施:

1. IME を明示的に OFF にした状態で他ウィンドウへ切り替え、Windows
   Terminal へフォーカスを戻して直後に入力（BUG-69 F3 が指摘する
   「実IMEがOFFの状態でフォーカスが戻る」場面を狙ったもの）
2. 63秒放置後の入力
3. 高速連続打鍵（BUG-45 の再現手順を意識したもの、14秒間に3回連続の
   cold-start が発生）

**結果**: 3シナリオとも、狙った cold イベントを含め全件が per-VK confirm
で正常に confirmed へ到達。`giving up`・`SuspectedLiteral` は全セッション
通じて0件。

**結論: 有望だが不確定。** 交絡は解消されたが、各シナリオ1〜4回に過ぎず
（本エントリの合格基準表が要求する「各シナリオ5試行以上」に届かない）、
群A（現行F2）・群B（`VK_IME_ON` 単発）との比較は未実施。さらに Opus
advisor によるレビューで、BUG-69（ADR-098）F1/F2/F3 が「TsfNative+GJI の
フォーカス復帰時、実際に機能する actuation は eager warmup だけ」と結論
していることが指摘された。ADR-100 はこれまでこの結論を一度も参照して
おらず、eager warmup 撤去を正式決定する前提として BUG-69 F1/F2 の修正が
先に必要と判断された（詳細は ADR-100 premortem P6）。

**学び（実施記録から）**: **対照群を作るための無効化ゲートは、既存の判定
ゲートより後に置くこと。** 前に置くと「意図的に止めた」ケースと「元々
発火しない」ケースが同一ログ行になり、事後に分離できない交絡を生む
（実施記録#1で実際に発生）。無効化する前に、その機構が「今日・この環境
で・本当に発火する」ことを同一ログで確認してから止める。エントリ09
（scan=0 注入がフックにすら届かず反証された事例）と同型の失敗——独立変数
が実は意図通りに動いていなかった——として記録する。

### 実施記録 #3（2026-08-22、決定4-f + 群B）

**決定4-f（`MapVirtualKeyW` 実機測定）**: standalone PowerShell からの
P/Invoke と、awase.exe 自身の送信直前（実行時キーボードレイアウト文脈内）
に追加した診断ログの2通りで測定し、`MapVirtualKeyW(VK_IME_ON=0x16,
MAPVK_VK_TO_VSC) = 0xF2 (242)`（非ゼロ）で一致。**ただし standalone 測定
だけでは信頼できないことも判明**——同じ standalone テストで
`VK_DBE_HIRAGANA (0xF2)` を引くと `0` を返したが、実際の hook ログは
一貫して `scan=0x70` を示しており矛盾する。`MapVirtualKeyW`（Ex でない版）
は呼び出しスレッドの実行時キーボードレイアウトに依存するため、standalone
プロセスと awase.exe 本体とで異なる値を返しうる。**今回は VK_IME_ON に
ついて両文脈が一致したため事なきを得たが、一般には awase 自身のプロセス
内で測るべき**という教訓が新たに得られた。結果、群B実験は第1候補
（`VK_IME_ON`, scan=0xF2 実値, `TSF_MARKER`）で組めることが確定した。

**群B（`VK_IME_ON` 単発、候補1の形態）**: **アプリ** Windows Terminal
（`CASCADIA_HOSTING_WINDOW_CLASS`、TsfNative）。**IME** GJI、Engine ON。
**手順**: `send_vk_dbe_hiragana_pair` の送信 VK を環境変数で
`VK_DBE_HIRAGANA`→`VK_IME_ON` に差し替えた診断ビルドで、15.6秒放置後の
入力・30.3秒放置後の入力を各1回実施。**結果**: 両方とも per-VK confirm
で正常に confirmed へ到達し、画面表示もユーザー目視で正しいひらがなと
確認した。この間の `cold=1`〜`13`（4種の cold reason）を通じて
`giving up`/`SuspectedLiteral` は0件。傍証として、ある送信の約1.9秒後に
通常の数百倍（584KB）の GJI 書き込みバーストを1件観測したが、因果の
確定には至っていない。

**結論: 有望だが不確定（群Cと同型の限界）。** 2026年5月に一度試されて
撤回された `VK_IME_ON` warmup 実験（`48d25f2`→`3d49109`、「TSF
composition context の初期化をトリガーしない」という実機観測で撤回）
とは異なる結果になっているが、当時と送信形態（scan の有無）が同一だった
かは確認できておらず、単純に「5月の結論を覆した」とは言えない。サンプル
数は決定2 の合格基準（各条件5試行以上、群Aとの同一セッション内比較）に
遠く届いておらず、群Aとの直接比較は一度も行っていない。

---

## エントリ 17: `key_remap`（ADR-110、物理キー単純リマップ）をバックエンドごと撤回 — Caps(英数)⇔Ctrlプリセット検討中に、既存の失敗例(エントリ07/08/09)を再導入していたと判明

**背景**: ADR-110で「任意の物理キーを別の物理キーとして常時リマップする」汎用機構
`key_remap`を実装・マージした（PR #120、BUG-100修正PR #121）。その後「人気の
組み合わせ（Caps(英数)⇔Ctrl）だけをGUIから簡単に設定できるようにしたい」という
要望を受け、専用プリセットとして絞り込む設計（ADR-111）を進めた。

| 日付 | 仮説 | 変更 | 観測結果 | 判定 | コミット |
| --- | --- | --- | --- | --- | --- |
| 2026-08-30 | `key_remap`の3ルール構成（`VK_DBE_ALPHANUMERIC→VK_LCONTROL`、`VK_CAPITAL→VK_LCONTROL`、`VK_LCONTROL→VK_DBE_ALPHANUMERIC`）でCaps(英数)⇔Ctrlの入れ替えとShift分岐（英数単独/Shift+英数で別VKが飛ぶJIS固有仕様）の両方に対応できるはず | ADR-111 r1として設計文書化 | Opus 2体による並列敵対的レビューで、3ルール目（`VK_LCONTROL→VK_DBE_ALPHANUMERIC`、`SendInput`による`VK_DBE_ALPHANUMERIC`注入）が**このリポジトリのエントリ07/08/09で既に3回失敗・撤去済みの手法**（scan値を変えてもawase自身のフックにすら届かない、またはCapsLockを物理的に点灯させる）の無自覚な再導入だったと判明。加えてEisu単独押下のKeyUpがWin32k内部のIME処理でフックに届かない可能性、両ルール同時有効化での相殺想定の誤りも指摘された | 撤回（`key_remap`のCaps/Ctrl方向の利用を断念） | （ADR-111 r2で方針転換） |
| 2026-08-30 | key_remap方式は諦めるが、Scancode Map方式（レジストリ、ドライバレベル）と併用すれば昇格可否に応じて両方式を選べる | PowerToys KeyboardManager（同種のフック方式）の実装・既知issueを調査 | PowerToys自身が「CapsLock→Ctrl + 日本語IME」の組み合わせで2020年から問題を抱え（Issue #3397、PR #4123でワークアラウンド）、2024年以降も再発報告が続く（Issue #32344）と判明。フックベースでこの特定キー・IME組み合わせを安全に扱う確信が持てないと判断 | 撤回（`key_remap`機能全体をバックエンドごとrevert、`docs/known-bugs.md` BUG-100・`docs/adr/110-*.md`のステータスも「撤回」に更新） | （revert PR、本エントリと同時期） |

**学び**:

- **「汎用機構として実装・マージ済み」であっても、特定用途に絞り込む設計を
  検討する段階で、その用途特有の危険（今回はJISキーボードのIME制御キー
  との衝突）が事後に判明することがある。マージ済みだからといって撤回の
  ハードルを上げてはいけない。** 逆に、実装が既に一定の品質（BUG-100修正・
  テスト・CI green）を経ていたことは、撤回の判断を「品質が低いから」と
  混同しないためにも明記しておく価値がある——今回の撤回理由は品質ではなく
  用途とのミスマッチ。
- **`docs/experiments.md`の過去エントリ（07/08/09）を読まずに設計すると、
  既に失敗が確定している手法を無自覚に再導入してしまう。** `.claude/rules/
  experiment-logging.md`が求める「失敗条件を書き残す」規約は、書いた本人
  以外の将来のセッション（今回のケースでは同一ユーザー・別セッションの
  設計検討）にも効く。Opus敵対的レビューが`grep`等でこの文書を横断的に
  参照したことで再発見できた——レビュー依頼時に「関連する過去の実験ログを
  確認してほしい」と明示的に伝えると再発見の確度が上がる。
- **他プロジェクト（PowerToys）の同種実装の実例調査は、自プロジェクトの
  実機検証が無い状態での判断材料として有効だった。** Microsoft自身のOSS
  プロジェクトが4年以上同じ問題に苦戦している実例は、「このリポジトリの
  実機データだけでは確定しない懸念」を補強する独立した証拠として機能した。
- **将来「アプリケーションごとに動的にキー割当てを変更する」機能を作る際は、
  今回の`key_remap`（グローバル・静的リマップ）の設計・実装をgit履歴から
  参照しつつも、IME制御キー（CapsLock位置・英数・かな等）を対象にする場合は
  本エントリとエントリ07/08/09を先に読み、フックベースでの実現可否を
  再検討すること。**

---

## エントリ 18: issue #137 設計時に BUG-61/62 の自動復旧不可を再確認し、再試行しないと決定

**背景:** Teams(WebView2/MS-IME) で romaji VK が JIS かな配列として解釈される
issue #137 の設計時点で、BUG-61/BUG-62 追補4で不可能と確定した自動復旧
（`ImmSetConversionStatus` 書き込み、`VK_DBE_ROMAN` 注入、言語バー COM 操作）を
Opus 2体による3ラウンドの敵対的レビューで再検討した。

**結論:** 自動復旧は再試行しない。`GetKeyState(VK_KANA)&1` による検知と、
既存トレイ右クリックメニュー/ツールチップでの案内だけに限定する。

---

## エントリ 19: GJI 専用Fnキー変換（ADR-091 §D3.2）の自動判定・設定支援ポップアップ・config1.db書き込みを全撤去 — 実験的機能のまま撤去し忘れて出荷、実機でユーザー混乱

**背景**: ADR-091 §D3.2「専用Fnキー変換」（無変換キー単独タップの代わりに
GJI側でF21にComposition/Conversion限定の`SwitchKanaType`を割り当てる案）は、
Phase 1の一部（自動判定・設定支援ポップアップ・config1.db書き込み）が
`develop`未マージのまま実験的に実装され、その後どこかのタイミングで
（本エントリの調査では特定できず）マージ・出荷されていた。撤去する計画は
無く、単に「実験を終わらせ忘れた」状態だったと判明した。

| 日付 | 仮説 | 環境（アプリ×IME） | 変更 | 観測結果 | 判定 | コミット |
| --- | --- | --- | --- | --- | --- | --- |
| 2026-09-02 | （出荷済みの既存機能）GJI検出時、無変換単独タップが素のパススルー設定のままなら「専用Fnキー(F21)を使った安全な変換方式を有効にしますか」というポップアップを出し、同意するとconfig1.dbに書き込む | Windows、Google日本語入力（キー設定は実際にはカスタム） | （調査対象、変更なし） | ユーザー（Macからの試用者）が起動時ポップアップに「はい」と答えたところ、直後に「Google日本語入力のキー設定がカスタム以外だったので設定を追加できませんでした」という失敗ダイアログが表示された。GJI側のキー設定は実際にはカスタムだったため、判定ロジックの誤診断が疑われる（`crates/awase-gji-config/src/lib.rs`の`session_keymap != Some(SESSION_KEYMAP_CUSTOM)`判定が、CUSTOM=0のprotobufデフォルト値省略により`None`と誤認した可能性）。ユーザーはこの経験から「机上のポップアップ→書き込み失敗という順序自体が不安を煽る設計であり、そもそも実験的機能なら完全に撤去すべき」と判断した | 撤回（機能全体を撤去） | （本コミット） |

**撤去の範囲**: `crates/awase-windows/src/gji_charset_popup.rs`・
`gji_charset_write.rs`・`crates/awase-gji-config/src/write.rs`を削除。
`gji_charset_autodetect.rs`からはF21専用Fnキーの自動判定部分
（`detect_dedicated_fn_key`、`Runtime::set_muhenkan_dedicated_fn_key_auto`/
`muhenkan_dedicated_fn_key_is_manual`）のみ除去し、同じファイルに同居して
いたADR-092 決定D Step4c（IME ON/OFF/トグルキーの自動検出、F21とは無関係の
別機能）は変更していない。`GeneralConfig::muhenkan_solo_tap_dedicated_fn_key`
による手動設定（config.toml経由）と`nicola_fsm.rs`の専用Fnキー送出ロジック
自体は残し、上級者が手動で有効化する経路は維持した。

**学び**:

- **「実装済み・develop未マージ」の実験的機能は、マージされた瞬間に
  「実験」から「本番機能」へ暗黙に昇格する。** ADR本文に「Phase 1実装済み
  （既定無効）」と書いてあっても、それが実際にリリースされたかどうかを
  追跡する仕組みが無いと、撤去判断の機会そのものを逃す。
- **「ポップアップで同意を取ってから失敗を通知する」設計は、たとえ機構が
  正しく動いていても心理的なコストが高い。** 実行前に前提条件（カスタム
  キーマップかどうか）を確認し、満たさない場合はそもそも選択肢を見せない
  （またはポジティブな案内に留める）方が、機構の正しさとは独立に重要な
  UX原則である。
- **同じファイル・同じ関数に複数の独立した機能（F21自動判定とADR-092
  Step4cのIME ON/OFF自動検出）を同居させると、片方だけを安全に撤去する際に
  「同居している機能まで巻き添えで壊していないか」の確認コストが増える。**
  今回はADR-092側の呼び出し元・テストを個別に確認した上で分離できたが、
  次に同種の自動判定機構を追加する際は、GJI検出の合流点は共有しつつも
  機能ごとに関数を分けておくと、将来の部分撤去が容易になる。

---

## エントリ 20: BUG-113「Windows Terminal + GJI で余分な@」— `send_ime_mode_key` の `wScan=0` 修正は実機A/Bで反証、副産物として BUG-114（drift correction の `FeedbackPolicy::Read` 無限再送）を発見

**背景**: ADR-133 の実機検証（2026-09-05）で、`GjiDirectStrategy::apply
(open=false)` が `send_ime_mode_key(VK_IME_OFF)` を `wScan=0` で送信して
いることが BUG-113（Windows Terminal + GJI、Engine 有効時に半角/全角キーで
「@」が出る）の真因候補と絞り込まれていた。

| 日付 | 仮説 | 環境（アプリ×IME） | 変更 | 観測結果 | 判定 | コミット |
| --- | --- | --- | --- | --- | --- | --- |
| 2026-09-05 | `send_ime_mode_key` の mode key 本体（`VK_IME_ON`/`VK_IME_OFF`）を `wScan=0` 固定（`make_key_input_ex()`）から `wVk` 保持＋`MapVirtualKeyW` 実測 scan 埋め込み（`make_scan_key_input()`、`KEYEVENTF_SCANCODE` なし）へ変更すれば「@」が再現しなくなるはず | Windows Terminal（`WindowsTerminal.exe`、`CASCADIA_HOSTING_WINDOW_CLASS`/`Windows.UI.Input.InputSite.WindowClass`）× Google 日本語入力、dragonflyg4実機、`spike/adr133-wt-vk-kana-dbe-hiragana`ブランチ | `send_ime_mode_key`（`ime.rs`）の送信を全呼び出し元（`GjiDirectStrategy`/`MsImeDirectStrategy`/`send_engine_state_ime_key`）に対し既定 on（Windows Terminal 限定 hidden opt-in にはしなかった） | ユーザーが実機で再現手順（Engine有効、半角/全角キー単独押下）を試したところ「何も変化はありませんでした」（@が出る現象そのままだった）。`RUST_LOG=debug`での追加ログ確認で、実際には物理キー1回の押下に対し `[drift] correction: observed=true ≠ desired=false` が **~14秒間、20〜90msおきに連続発火**し、`VK_IME_OFF`を`SendInput`で送り続けていたことが判明（`gave up`ログは0件）。ログの`strategy=`タグは`drift_correction_read`——`caps(TsfNative, Gji)`が本来返すべき`FEEDBACK_BLIND`ではなく`FEEDBACK_READ`が使われていた。コード読解の結果、`focus/class_names.rs::AppImeProfile::from_class_name`がフォールバックで`Standard`を返すケースがあり、`FocusChanged`発火の瞬間にこれが起きると`ImePolicyProfile::ImmCross`→`FEEDBACK_READ`が`app_policy`に焼き付き、以後のフォーカスセッション中ずっと`Read`のまま（`current_app_profile()`自体は後から正しく`TsfNative`を返すのに`app_policy`は`FocusChanged`時のスナップショットしか見ない）になる経路を発見。`Read`は`decide_actuation_action`にGiveUp分岐が無く常に`Send`を返すため、IMMクエリが構造的に不可能な当該クラス（`Skipping IMM query for known-broken class`）では収束観測が一生得られず無限に近い頻度で再送し続ける | 撤回（`send_ime_mode_key`は元の`wScan=0`固定へrevert）。**「@」の真因はこのBUG-114単独ではないと後日判明**（drift correctionが正常に有界動作した回でも「@」は再現した、known-bugs.md BUG-113参照）——BUG-114自体は独立の実在バグとして別途起票、真因候補は後日`send_ime_mode_key`の`SendInput`バッチ形状（ADR-133）へ絞り込まれた | e8aa19b0（元コミット）→（本revertコミット） |

**学び**: 「候補まで絞り込んだ」状態でも実機A/Bを経ずに「全アプリ・既定on」の
グローバル変更へ踏み切ると、反証されたときの後始末（revert対象の特定・
docsの巻き戻し）が大きくなる。特にこの変更は Windows Terminal 限定に
スコープすることもできたが、ユーザー判断で全アプリ適用にした結果、
反証後は影響範囲の広い変更を丸ごとrevertする必要が生じた。また
「何も変化がない」という否定的な実機報告こそ、次の仮説を焦って作らず
`RUST_LOG=debug`のような詳細ログに立ち返って実際に何が起きているかを
虚心に見直すべきサインだった——今回は debug ログ1回の取得で全く別の、
より深刻な機構（無限に近い再送ループ）を発見できた。
