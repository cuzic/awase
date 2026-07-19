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
