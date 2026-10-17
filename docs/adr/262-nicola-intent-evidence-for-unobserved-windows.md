---
id: ADR-262
title: |-
  観測の無い窓で、窓切替直後・無操作後の最初の親指シフトの同時打鍵を契機に、IME ON を 1 回だけ明示して送る(撤去した force-on〔BUG-016〕を絞って復元する、ADR-212 の例外)
summary: |-
  一太郎のように窓内の IMM が読めない窓(観測の無い窓)では、belief が ON のまま実 IME が OFF のずれ(issue #623)を、読んで直す手段が無い。
  この症状は BUG-016(フォーカス遷移直後、belief ON なのに実 IME が OFF で最初のかな入力がリテラル化)と同じ向きで、BUG-016 の修正だった `apply_force_on_for_imm_broken` は
  ADR-179 領域A(`621bf93c`、2026-09-18)で撤去された。**本 ADR はそれを「ImmCapability::Unavailable × エピソード 1 回 × 同時打鍵の確定」に絞って復元する**。
  ADR-212 の方針(awase は利用者の IME キー以外で IME に書かない)の補正的な書き込みにあたるため、**所有者判断(2026-10-17)による例外**として記録する。
  対象は belief が ON(Engine 活性)の向きだけ(Engine が非活性だと同時打鍵が判定されない)。belief・IntentStore は動かさない。applied は書き換えない。
  ADR-261(字形で読む)とは独立で、字形が読めればそちらを優先する。段階 0 は挙動を変えず、訂正の代理指標を読める窓と対照して取る。
status: |-
  起草(2026-10-16)→ Opus round 1(Blocker 4・Major 6・minor 6)で書き直し(2026-10-17)→ round 2(Blocker 1・Major 4・minor 4)を受けて再び書き直し(2026-10-17)。
  実装なし。測定なし。round 3 の再確認待ち。**ADR-260・ADR-261 のマージ後に本 ADR をマージする**(本文のリンクが切れるため)。
related_adr:
  - "ADR-098"
  - "ADR-179"
  - "ADR-188"
  - "ADR-191"
  - "ADR-199"
  - "ADR-203"
  - "ADR-208"
  - "ADR-212"
  - "ADR-213"
  - "ADR-214"
  - "ADR-255"
  - "ADR-256"
  - "ADR-260"
  - "ADR-261"
---

# ADR-262: 観測の無い窓で、親指シフトの最初の同時打鍵を契機に IME ON を明示する

## ステータス

起草(2026-10-16)。round 1・round 2 の Opus レビューを受けて書き直した(2026-10-17)。実装・測定なし。

## 背景

- issue #623(一太郎・GJI)の app log: 別アプリで IME OFF → 一太郎へ移る → `HwndCache: restore ime_on=true` → `FocusProbe: ime_on=true(shadow)`。
  **belief は ON、実 IME は OFF**。一太郎は `ImmCapability::Unavailable` と学習され(`Imm32Unavailable` に降格)、窓内の IMM が読めない。
- **同じ症状の既知バグ**: `docs/known-bugs/BUG-016.md`「フォーカス遷移直後、belief は IME ON / Engine ON なのに実 IME は OFF のままで、最初のかな入力がリテラル化(『これで』→『korede』)」。
  修正は `apply_force_on_for_imm_broken` / `try_force_on_bootstrap` / `force_on_and_correct_romaji` が、キー入力の直前に belief の ON を `VK_IME_ON` で書く機構だった。
  ADR-179 領域A の撤去(`f83084b3`・`621bf93c`、2026-09-18)で削除された。撤去の方針は「drift correction のみを残し、実機 A/B で問題が出れば復元する」。#623 は、その復元条件にあたる可能性がある。
- **偽の Confirmed**: フォーカス直後に `applied` は Unknown になるが(`ime_model.rs:1018`)、TSF ネイティブでない窓で belief が ON なら、`presync_applied_open_on`
  (`runtime/focus_tracking.rs:193-197`、呼び出し `:778-783`、`ime_refresh.rs:845-866` も同処理)が `record_confirmed(true)` を書き、GjiFsm に ImeOn を通知する。
  一太郎(`Imm32Unavailable`、TSF ネイティブではない)では、何も書いていないのに `applied=Confirmed(true)` になる。ADR-098 が TsfNative で止めた「applied の偽装確定」と同じ型が残っている。
- [ADR-260](260-eisu-toggle-no-mode-prediction-when-observable.md) は読みが効く窓にしか効かない。[ADR-261](261-taskbar-indicator-glyph-observation-for-unreadable-windows.md) は字形で読む案だが、一太郎でインジケーターが実状態を映すかが未測定。
- 根本原因(belief が違うのか、書き込みが届かないのか)は分かっていない。所有者は一太郎を持っていない。

## 着想

状態が読めないなら、打つ人の意図を契機にする。親指シフトで打つ人は、最初の同時打鍵で「NICOLA で打つつもりだ」と言える。
7 打鍵や 2.5 秒の統計を待つ案は採らない(v3 構想〔打鍵列を保持して最尤で決める〕の調査を 2026-10-16 に断念した。Phase 2 は全決定の 0.07%。所有者も撤回した)。

## 所有者判断(2026-10-17)

1. 新しい order 起案の入口が増えることは許容する。
2. 一太郎で明示の IME ON が効くことは前提とする(段階 1 の実機で検証し、効かなければ本 ADR は撤回する)。
3. **撤去した force-on の、絞った復元を ADR-212 の例外として認める**。「読めない窓に限定してよい」を、この例外の授権として記録する。
   ADR-212 の【許可】(利用者が押した IME キーへの応答)でも【出力】でもなく、【補正的】な書き込みにあたる。ADR-212 に本 ADR への追記を入れる。

## 決定

### D1: 向きは「belief が ON(Engine 活性)」だけ

belief が OFF のとき Engine は Phase 2 で `pass_through` し、NicolaFsm に届かない(`src/engine/engine.rs:558-568`)。
belief OFF・実 ON の向きには「同時打鍵の成立」を判定する主体が無く、扱わない(殻に二つ目の検出器を置くと同時打鍵のタイミング判定が二重になり、ADR-019 に反する)。

### D2: 契機と、動く条件

契機は、Engine が親指キーを含む NICOLA の同時打鍵を**確定**した時点。確定は `Engine::on_input` だけでなく `Engine::on_timeout`(文字キーが先で親指を待つ側。`runtime/message_handlers.rs:722`・`:1844`)でも起きる。
`Decision` には「同時打鍵を確定した」印が無い(`src/engine/decision.rs:26-31` の `InputEffect` は `SendKeys`・`ReinjectKey`)。
**ルート `awase` crate の `Decision` に、確定した打鍵の種類の印を足す**(core が分類済みの情報を返す。ADR-019 に反しない)。回帰テストは `src/engine/tests.rs`。処理は executor(`runtime/executor.rs::execute_one`/`dispatch_effect`)で行い、`on_input` と `on_timeout` の両方を覆う。

以下を全部満たすときだけ動く。

1. 観測の無い窓。述語は `imm_capability(process, class) == Unavailable` に一本化する。`direct_mode_key_watch_kind_for` の否定は使わない(読める Chrome も含んでしまう)。`AppImeProfile::Imm32Unavailable` でもない。
2. 窓切替から、または無操作の終わりから、最初の確定であること(エピソードごとにラッチ 1 回)。無操作の閾値は既存の idle 判定を再利用し、新しい定数を足さない(足すなら `#[measured]` と実測を添える)。
3. そのエピソード内で、利用者が明示の IME キーを押していないこと(前の窓での押下は数えない)。理由: #623 の再現手順がまさに「前の窓で OFF にして移る」ため。
   **ただし**、これは英数の意図の弱い証拠を捨てる判断でもある(N-M3)。誤爆の害より #623 を直す利益が大きいと判断した、と記録する。段階 0 で、前の窓で OFF にして移ったエピソードの訂正率を別に出す。
4. 修飾キーなし、かつ awase 自身の注入でないこと。
5. **ADR-255(IME OFF のとき親指キーを Space にする)が有効な構成では動かさない**。#623 の向き(belief ON・実 OFF)は、ADR-255 の利用者が「OFF だと思っている」状況そのもので、
   Space のつもりの親指キーと次の英字の重なりを NICOLA の同時打鍵と取り違えて IME を開く害が、D1 の非対称の目的に反するため。

### D3: 動作 — 入口を 1 つ足す

- 純粋な判断(Plan)を `state/` に置く。引数は Facts(窓の種別・エピソード・ラッチ・直前の明示キーの有無・ADR-255 の有無)、戻りは「何もしない/ON を送る」と理由。journal には境界で記録する(ADR-250)。
- 実行は新しい order 起案の入口 1 つ。既存の同期の合流点 `ImeController::apply`(→ 核の `apply_sync`)を通す(`apply_sync` の呼び出し元の件数ガード〔`tests/architecture_guard.rs:5646-5649`〕を変えない)。
  - `press=None`。`claim_press_write` は呼ばない。
  - 送るかどうかの判断の入力(view の `shadow_on`)は **未知にする**。`ime_actuation_decision::explicit_press_shadow_on`(`:219-227`)は名前が「押下」なので、押下に依らない別名の関数を足して使う。
    未知化が要る理由は、背景の presync による偽の `Confirmed(true)` で、GjiDirect の already-matched 省略(`ime_controller.rs:228-247`)が働くため。MsImeDirect には省略が無いので、未知化は GJI でだけ効く。
  - model の `applied` は**書き換えない**(presync の `Confirmed(true)` のまま)。「未知のまま」とは書かない。
  - 完了の記録: `record_confirmed` は書かない(効いていない ON を確認済みにしない)。`record_ime_apply_result_in_scope`(`platform_state.rs:1410-1433`)は呼ばないので、
    同関数の先頭の ADR-256 の `note_awase_open_write`(`:1418-1422`)も走らない。ADR-256 の書き込み遅延の抑止が要るかは未決(Q8)。
  - **GjiFsm を OFF→ON(cold)として同期する**。#623 ではフォーカス時の presync で GjiFsm はすでに ImeOn(warm 扱い)だが、実 IME は閉じているので、本 ADR の ON で初めて開く。
    同期しないと GjiFsm が cold の扱い(probe/保留)に入らない。`GjiFsmSync` に variant を足す(ADR-203 の `gji_sync_origin_comes_from_the_sync_variant` ガードの対象)。warmup ファミリーなので回帰テストか BUG 記録を添える。
- 授権: `press=None` は通常の `issue_open_warrant`(`state/open_warrant.rs`)。一太郎は `FEEDBACK_BLIND`(`app_ime_policy.rs:131-140`)なので、belief ON なら Step 4c の `OwnSsot` で通る。
  **その hwnd に TTL 内の明示 OFF の意図が IntentStore に残っていれば、Step 1 で `Unwarranted` となり止まる**。これは利用者の意図を尊重する正しい挙動で、新しい basis は足さない。
- ガード: `RESTRICTED_CALLS`(`lints/actuation_call_guard/src/lib.rs:58-`)の `apply_sync` の許可呼び出し元に、この入口の関数を登録する。`tests/architecture_guard.rs` には、この入口の assert
  (`claim_press_write` を呼ばないこと・`press=None` であること・`apply_sync` を通すこと)を足す。`press_id_is_claimed_and_carried_at_every_order_issuing_entry` は入口ごとの固定の assert を並べる形なので、同じ形で足す。
  `issue_self_actuation_order_is_called_only_from_dispatch_set_open`(`:5580`)に触れないことを確認する。

### D4: 出力との順序

同じ executor の処理の中で ON を送ってから `SendKeys` を流す。Imm32Unavailable の書き込みは `CHAIN_GJI_ONLY` / `CHAIN_MS_IME_ONLY` の**同期の SendInput 1 回**なので、入力キュー上の順序は保たれる。
「保留の期限」は同期の経路では意味を持たない。問題は送信の完了ではなく、**GJI が ON 後に VK を受け付け可能になるまでの時間**(BUG-002 系。例: `9a7e699` の実測 181ms)である。
段階 0 でこの時間を実測し、D3 の GjiFsm の cold 同期経由の既存の warmup(`state/warm_send_plan.rs`)に乗せる。乗せられなければ段階 1 に進まない。

### D5: 下限の主張と限界

- 「効かなければ今と同じ」ではない。(a) cold-start のリテラル化、(b) 最初の 1 打の遅れ、がありうる。
- **確定より前に出た単打は救えない**。NICOLA の文の頭の多くは親指を含まない単打で、これは閉じた窓にすでにリテラルとして出ている。本 ADR が防ぐのは、確定の打鍵以降である。
- 段階 1 は設定で有効化し、既定は無効にする。

### D6: 旧機構との違いと、撤去の理由に対する立場

旧 `apply_force_on_for_imm_broken` は毎打鍵の前に(クールダウンつきで)発火し、BUG-016 の settle の穴と BUG-069(ADR-098 F2: スパムガードの恒久無効化)を生んだ。
本 ADR は、(1) 発火をエピソード 1 回のラッチに限る、(2) 契機を意図の証拠にする、(3) applied を書き換えない、(4) 読める窓には出さない、で絞る。
エピソードのラッチが BUG-016 の settle の穴と BUG-069 のどちらを構造的に避けるかは、実装前に当時のコードと各 BUG の記録を読んで追記する(未決 Q6)。

**revert 条件**(`experiment-logging.md`): 段階 1 の実機(一太郎・GJI)で、(a) 確定直後のリテラル化が増える、(b) 英数で打つ人の IME を開いて訂正(BackSpace・明示の IME キー)が増える、のどちらかが段階 0 のログと比べて見られたら取り下げ、
`docs/experiments.md` に失敗条件(アプリ・IME・症状)を残す。再導入の行も、実装時に `docs/experiments.md` へ足す。

## 段階

- **段階 0(挙動を変えない)**: D2 を満たすエピソードで、次を境界で journal に記録する。
  - 窓切替/無操作からの経過 ms、プロセス名・クラス名、belief(open/mode と観測元)、`ImmCapability` の学習状態(降格が何回目のフォーカスで起きたか)、`presync` が書いた `applied` の値。
  - 利用者の訂正の代理指標: 確定の直後 N 秒以内の BackSpace の連打、明示の IME キー、Esc(生の時刻を残す。N と連打の閾値は今は決めない)。
  - GJI の受け付け可能時間の実測(D4)。
  - **対照群**: 読める窓(実 Chrome・tsf フォーム等。direct-follow で実状態が分かる)で、同じエピソードの定義の訂正率を同じログで取る。NICOLA の利用者は普通の入力でも確定直後に BackSpace を打つので、
    観測の無い窓の率が読める窓より有意に高いことを合格基準にする。読める窓では、実状態が belief と食い違ったエピソードと訂正の相関も測れる(代理指標の較正)。
  - 合格しなければ、直す対象が見つからなかったことになり、本 ADR は撤回する。
- **段階 1(設定で有効化、既定は無効)**: D2〜D4 を実装する。報告者の実機で、訂正の頻度が減るか、一太郎で ON が効くかを検証する。段階 1 の前に、ADR-261 の CI の窓に GjiDirect の ON を送って効くかを安く確かめる価値がある。

## 範囲外

- belief OFF・実 ON の向き(D1)。
- 問題 2(英数を押してもモードずれが解消しない、ひらがな/半角英数のずれ)。開閉軸だけを扱い、GjiDirect の ON は GJI の前回のモードを復元するだけなので、#623 を閉じる ADR ではない。
- 常時の判定、英数側の統計判定。

## ADR-261 との関係

ADR-261 は「読む」、本 ADR は「書く」。ADR-261 が go で字形が読めるなら、字形の読みを優先し、本 ADR の段階 1 は動かさない。
ADR-261 が no-go(H1)でも、所有者判断により書き込みが効くことを前提とし、段階 1 で検証する。両 ADR の実機確認(インジケーター・実入力・SendInput の ON の一致)は、報告者への依頼を一回にまとめる。

## 代替: presync を止める(撤去で直す案)

`presync_applied_open_on` を `ImmCapability::Unavailable` の窓で止めれば(ADR-098 決定 1-a の TsfNative と同じ扱い)、偽の `Confirmed(true)` が無くなり、view の未知化が要らなくなる。書き込みを足さない、撤去の方向の案である。
presync の目的(直後の Engine ON が古い applied を根拠に不要な再送へ進むことの防止、GjiFsm への ImeOn 通知)が、Unavailable の窓で今も生きているかは未確認(ActivationSync は ADR-213 で撤去済みなので、前者は残っていない可能性が高い)。
ただし、これだけでは「実 IME が OFF のまま」は直らない(偽の確定を作らなくなるだけ)ので、本 ADR の代わりにはならず、併用の候補にとどめる(未決 Q9)。

## 未決事項

- Q1: class_name 単位の学習(`cache.toml`)のため、同じクラス名の別アプリへ広がる範囲(BUG-107 型)。
- Q3: 無操作の閾値、訂正の代理指標の N 秒、GJI の受け付け可能時間(段階 0 で実測)。
- Q6: エピソードのラッチが、旧機構の BUG-016 の settle の穴・BUG-069 のどちらを構造的に避けるか。
- Q7: 初回の窓切替は降格前(`Standard`/`ImmCross`)なので対象外になる。降格の契機とタイミング次第で、最初のエピソードを取りこぼす。
- Q8: ADR-256 の書き込み遅延の抑止(`note_awase_open_write`)だけを、完了の記録なしで呼ぶ必要があるか。
- Q9: presync の停止(上記)を、本 ADR と併用するか、先に単独で行うか。
