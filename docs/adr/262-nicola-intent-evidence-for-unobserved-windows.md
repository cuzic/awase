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
  実装なし。測定なし。round 3(Blocker 0・Major 3・minor 5)を受けて文面を直した(2026-10-17)。段階 0 に進むことにレビュアーの異論なし。**ADR-260・ADR-261 のマージ後に本 ADR をマージする**(本文のリンクが切れるため)。
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
**印は `Effect` の variant にしない**(`Decision::effects_mut()` の `PassThrough`→`PassThroughWith` 昇格、effect の件数を固定するテスト〔`decision.rs` 12 件・`fsm_adapter.rs` 9 件〕、`tests/scenarios.rs:158`、executor の `dispatch_effect` を壊すため)。
次の形にする(core が分類済みの情報を返す。ADR-019 に反しない)。
- 同時打鍵の確定の唯一の合流点 `NicolaFsm::resolve_char_thumb_as_simultaneous`(`src/engine/nicola_fsm.rs:1172-1189`、呼び出し元 4 か所)の成功の分岐(`lookup_face` が `Some` で `consume_thumb` を呼ぶ側)で、
  「この評価で同時打鍵を確定した」one-shot のフラグを立てる。面の判定は `Face::thumb_side()` が `Some` のときに限る(Shift+親指の面は D2-4 の「修飾キーなし」で除外)。
- `Engine` が `on_input`(`engine.rs:571-591`)と `on_timeout`(`:596-611`)の両方で、そのフラグを `Decision` のフィールド(effect ではないメタ情報)へ移す。既存の one-shot の取り出し口
  (`take_engine_off_requested`・`take_ime_open_requested`)にならう。`on_command` の全アームでフラグを捨てる(次の無関係なイベントで誤発火させない)。
- 回帰テストは `src/engine/tests.rs`: (i) 同時打鍵で立つ、(ii) 単打・親指単独・`lookup_face` が `None` の単打への落ちでは立たない、(iii) `on_timeout` の確定で立つ、(iv) 次のイベントに残留しない。
処理は executor で行い、`on_input` と `on_timeout` の両方を覆う。

以下を全部満たすときだけ動く。

1. 観測の無い窓。述語は `imm_capability(process, class) == Unavailable` に一本化する。`direct_mode_key_watch_kind_for` の否定は使わない(読める Chrome も含んでしまう)。`AppImeProfile::Imm32Unavailable` でもない。
2. 窓切替から、または無操作の終わりから、最初の確定であること(エピソードごとにラッチ 1 回)。無操作の閾値は既存の idle 判定を再利用し、新しい定数を足さない(足すなら `#[measured]` と実測を添える)。
3. そのエピソード内で、利用者が明示の IME キーを押していないこと(前の窓での押下は数えない)。理由: #623 の再現手順がまさに「前の窓で OFF にして移る」ため。
   **ただし**、これは英数の意図の弱い証拠を捨てる判断でもある(N-M3)。誤爆の害より #623 を直す利益が大きいと判断した、と記録する。段階 0 で、前の窓で OFF にして移ったエピソードの訂正率を別に出す。
4. 修飾キーなし、かつ awase 自身の注入でないこと。
5. **ADR-255(IME OFF のとき親指キーを Space にする)が有効な構成では動かさない**。述語は「`[[keymaps]]` に、`from` が親指キー(無変換/変換、または `left/right_thumb_key`)で `ime = "off"` の規則が 1 つでもある」(ADR-255 は r7 で専用設定からこの形に変わった)。
   Alt なりすまし(`RawKeyEvent.impersonated`)の構成を含めるかは実装時に決める。#623 の向き(belief ON・実 OFF)は、ADR-255 の利用者が「OFF だと思っている」状況そのもので、
   Space のつもりの親指キーと次の英字の重なりを NICOLA の同時打鍵と取り違えて IME を開く害が、D1 の非対称の目的に反するため。

### D3: 動作 — 入口を 1 つ足す

- 純粋な判断(Plan)を `state/` に置く。引数は Facts(窓の種別・エピソード・ラッチ・直前の明示キーの有無・ADR-255 の有無)、戻りは「何もしない/ON を送る」と理由。journal には境界で記録する(ADR-250)。
- 実行は新しい order 起案の入口 1 つ。drift correction と同じく `press=None` の書き込みなので、`platform.apply_ime_open_with_view` を通し、order の発行は `runtime/mod.rs::issue_actuation_order` を使う
  (`ImeController::apply` → `apply_sync` を直接の合流点にしない。`apply_sync` の許可リスト〔`("apply_sync", &["apply", "dispatch_set_open"])`〕は変わらず、`issue_self_actuation_order_is_called_only_from_dispatch_set_open` にも触れない)。
  - `press=None`。`claim_press_write` は呼ばない。
  - 送るかどうかの判断の入力(view の `shadow_on`)は **未知にする**。`ime_actuation_decision::explicit_press_shadow_on`(`:219-227`)は名前が「押下」なので、押下に依らない別名の関数を足して使う。
    未知化が要る理由は、背景の presync による偽の `Confirmed(true)` で、GjiDirect の already-matched 省略(`ime_controller.rs:228-247`)が働くため。MsImeDirect には省略が無いので、未知化は GJI でだけ効く。
  - model の `applied` は**書き換えない**(presync の `Confirmed(true)` のまま)。「未知のまま」とは書かない。
  - 完了の記録は**分けて扱う**。外すのは `record_ime_apply_result`(`applied` の Confirmed と、同関数の先頭の ADR-256 の `note_awase_open_write`、`platform_state.rs:1410-1433`)だけ(効いていない ON を確認済みにしない)。
    `platform.on_ime_applied`(`platform.rs:1158-1222`。`mark_composition_cold(SetOpenTrue)`・MS-IME の `ime_mode_fsm.on_set_open_applied` の未確認化・GjiFsm の receipt の settle を行う)は**呼ぶ**。
    これを落とすと、ON を送ったあとも composition が warm 扱いのまま、MS-IME のモード FSM が確認済みのままになり、cold-start のリテラル化を自分で招く。
    `on_ime_apply_complete`(`runtime/mod.rs:930-977`)をそのまま流用しない。`OpenApplyReason` に新しい値(旧 `ImmBrokenForceOn` を含む)は足さない。journal は本 ADR の境界記録で代える。
    `post_ime_refresh`(20ms 後の再試行)を呼ぶかは未決(Q8。呼ぶと BUG-069 の再試行の要素が 1 つ戻るが、Confirmed を書かないので固着は起きない)。ADR-256 の抑止(`note_awase_open_write`)だけを呼ぶかも Q8。
  - `last_explicit_ime_action_ms` は更新する(`key_pipeline.rs:549`・`:779-788` の idle conv check と、`platform_state.rs:538-563` の direct-follow の窓が自分の書き込みを外部の変化と誤読しないため。旧 force-on も `note_explicit_ime_action` で同じ扱いだった)。
    **ただし D2-3 の「明示の IME キーを押していない」の判定には、この値を使わない**(自分の書き込みを利用者の明示と取り違える)。物理の押下の記録を別に使う。
  - **GjiFsm を OFF→ON(cold)として同期する**。#623 ではフォーカス時の presync で GjiFsm はすでに ImeOn(warm 扱い)で、`gji_on_ime_on` は `OffCold` のときだけ cold に遷移する(`tsf/gji_fsm.rs:597-618`)ので、ON を送ったあとに呼んでも何も起きない。
    新しい `GjiFsmSync` variant は要らない。既存の `GjiEvent::Reopen`(ADR-203 決定 2。`OnWarm → OnCold`、`OnCold`・`OnComposing` では何もしないので probe・pending・未確定文字を保持する)を使い、
    `ReopenSource`(`state/gji_direct_mechanism.rs:66-86`)に variant を 1 つ足す。`Reopen(_)` の origin の doc〔「awase が IME へ書いていない同期」〕は「または awase の書き込み後に既に ON 扱いの FSM を開き直す同期」に直す。
    ADR-203 の `gji_sync_origin_comes_from_the_sync_variant` ガード(`architecture_guard.rs:7258-7270`)は壊れない。warmup ファミリーなので回帰テストか BUG 記録を添える。
    **この同期は、presync を先に止める場合(「段階」の段階 A)は不要になる**(下記)。
- 授権: `press=None` は通常の `issue_open_warrant`(`state/open_warrant.rs`)。一太郎は `FEEDBACK_BLIND`(`app_ime_policy.rs:131-140`)なので、belief ON なら Step 4c の `OwnSsot` で通る。
  **その hwnd に TTL 内の明示 OFF の意図が IntentStore に残っていれば、Step 1 で `Unwarranted` となり止まる**。これは利用者の意図を尊重する正しい挙動で、新しい basis は足さない。
- ガード: `RESTRICTED_CALLS`(`lints/actuation_call_guard/src/lib.rs:58-`)の `("apply_ime_open_with_view", &["ir_apply_drift_correction"])` にこの入口の関数名を足し、
  `tests/architecture_guard.rs` の `.apply_ime_open_with_view(` の件数ガードを 1→2 にする(`fix-requires-evidence.md` の「IME actuation 合流点」行が、この件数ガードを一次情報としている)。
  `press_id_is_claimed_and_carried_at_every_order_issuing_entry` は入口ごとの固定の assert を並べる形なので、この入口の assert(`claim_press_write` を呼ばないこと・`press=None` であること)を同じ形で足す。

### D4: 出力との順序

同じ executor の処理の中で ON を送ってから `SendKeys` を流す。Imm32Unavailable の書き込みは `CHAIN_GJI_ONLY` / `CHAIN_MS_IME_ONLY` の**同期の SendInput 1 回**なので、入力キュー上の順序は保たれる。
「保留の期限」は同期の経路では意味を持たない。問題は送信の完了ではなく、**GJI が ON 後に VK を受け付け可能になるまでの時間**(BUG-002 系。例: `9a7e699` の実測 181ms)である。
段階 0 でこの時間を実測し、D3 の GjiFsm の cold 同期経由の既存の warmup(`state/warm_send_plan.rs`)に乗せる。乗せられなければ段階 1 に進まない。

### D5: 下限の主張と限界

- 「効かなければ今と同じ」ではない。(a) cold-start のリテラル化、(b) 最初の 1 打の遅れ、がありうる。
- **確定より前に出た単打は救えない**。NICOLA の文の頭の多くは親指を含まない単打で、これは閉じた窓にすでにリテラルとして出ている。本 ADR が防ぐのは、確定の打鍵以降である。
- 段階 1 は設定で有効化し、既定は無効にする。

### D6: 旧機構との違いと、撤去の理由に対する立場

旧 `apply_force_on_for_imm_broken`(撤去前のコード `621bf93c^:crates/awase-windows/src/runtime/mod.rs` 958-1012 行)の事実は次のとおり。
- 唯一の呼び出し元は `ir_stage_notify` の Phase 4a(**周期リフレッシュ**)。doc 自身が ADR-086 INV-15 の「生の周期タイマー」に該当する既知の逸脱と書いていた。キー入力直前のトリガーは ADR-094 で撤去済みだった。
- BUG-016(`docs/known-bugs/BUG-016.md`)の原因は、force-on が settle 中に**スキップされ、再試行が無かった**こと(発火が足りなかった)。修正は settle 明けの再試行(`schedule_settle_retry`)。
  さらに当時は `platform.set_ime_open` が Imm32Unavailable では no-op で、効いていなかった。
- BUG-069(ADR-098 F2)は、force-on 自身の発火ではなく、(i) 別機構 `mirror_applied_open` の `applied=Confirmed` 偽装がスパムガードを誤発火させ恒久的に無効化したこと、
  (ii) 決定 1-c: Failed のとき `record_ime_apply_result` が `Confirmed{open:false}` を書き、`on_ime_apply_complete` の `post_ime_refresh()`(20ms 後の再試行)と組み合わさって 50Hz の無限再試行ループになったこと、が原因。
- 旧 force-on も view の `applied` を `None` にして GJI の already-matched を外していた(`force_on_and_correct_romaji` の doc)ので、D3 の view の未知化には前例がある。

本 ADR は、(1) 完了の記録(`applied`)をしない、(2) エピソード 1 回のラッチ、(3) 周期に乗らず確定の打鍵を契機にする、の 3 点で、BUG-069 の 3 要素(偽装・完了の記録・周期の再試行)を構造的に持たない。
BUG-016 の教訓は「発火しすぎ」ではなく**フォーカス直後の最初の打鍵までに確実に 1 回届けること**である。本 ADR は確定の打鍵を契機にするので settle の窓とは独立になる(ただし D5 のとおり、確定より前の単打は救えない)。

**revert 条件**(`experiment-logging.md`): 段階 1 の実機(一太郎・GJI)で、(a) 確定直後のリテラル化が増える、(b) 英数で打つ人の IME を開いて訂正(BackSpace・明示の IME キー)が増える、のどちらかが段階 0 のログと比べて見られたら取り下げ、
`docs/experiments.md` に失敗条件(アプリ・IME・症状)を残す。再導入の行も、実装時に `docs/experiments.md` へ足す。

## 段階

- **段階 A(先に単独で行う。書き込みを足さない)**: `ImmCapability::Unavailable` の窓で `presync_applied_open_on`(`runtime/focus_tracking.rs:193-197`・`:778-783`、`ime_refresh.rs:845-866`)を止める
  (ADR-098 決定 1-a の TsfNative と同じ扱い)。偽の `Confirmed(true)` が無くなり、`applied` が Unknown のままになるので、D3 の view の未知化(押下に依らない別名の関数)と GjiFsm の `Reopen` の追加は要らなくなる。
  presync の目的は、ActivationSync(ADR-213 で撤去)と force-on(`621bf93c` で撤去)の撤去後ほぼ消えており、GjiFsm への ImeOn 通知は ADR-203 の level 突合 `OnImeOnBelief`(`gji_direct_mechanism.rs:55-57`)が代わりに行う。
  この変更は IME belief ファミリー(`focus_tracking.rs`)なので、`fix-requires-evidence.md` の (a) 回帰テストか (b) `docs/known-bugs/BUG-NNN.md` を添える。
  これだけでは実 IME は OFF のままで、#623 の症状は直らない(偽の確定を作らなくなるだけ)。段階 0 のログで、段階 A の前後を分けて見られる利点がある。
  段階 A の前に、`ime_refresh.rs:794` の `warmup_ime_on()`(`applied ?? belief`)と、GjiDirect の already-matched(`ime_controller.rs:228-247`)で drift correction の書き込みが増えないことを確かめる。
- **段階 0(挙動を変えない)**: D2 を満たすエピソードで、次を境界で journal に記録する。
  - 窓切替/無操作からの経過 ms、プロセス名・クラス名、belief(open/mode と観測元)、`ImmCapability` の学習状態(降格が何回目のフォーカスで起きたか)、`presync` が書いた `applied` の値。
  - 利用者の訂正の代理指標: 確定の直後 N 秒以内の BackSpace の連打、明示の IME キー、Esc(生の時刻を残す。N と連打の閾値は今は決めない)。
  - GJI の受け付け可能時間の実測(D4)。
  - **対照群**: 読める窓(実 Chrome・tsf フォーム等。direct-follow で実状態が分かる)で、同じエピソードの定義の訂正率を同じログで取る。NICOLA の利用者は普通の入力でも確定直後に BackSpace を打つので、
    観測の無い窓の率が読める窓より有意に高いことを合格基準にする。読める窓では、実状態が belief と食い違ったエピソードと訂正の相関も測れる(代理指標の較正)。
  - 件数の下限: 各群 n≥20 のエピソード。満たさないときは判定を保留し、ログを溜め続ける(報告者が 1 人なので、検定の形にはしない)。
  - 合格しなければ、直す対象が見つからなかったことになり、本 ADR は撤回する。
- **段階 1(設定で有効化、既定は無効)**: D2〜D4 を実装する。報告者の実機で、訂正の頻度が減るか、一太郎で ON が効くかを検証する。段階 1 の前に、ADR-261 の CI の窓に GjiDirect の ON を送って効くかを安く確かめる価値がある。

## 範囲外

- belief OFF・実 ON の向き(D1)。
- 問題 2(英数を押してもモードずれが解消しない、ひらがな/半角英数のずれ)。開閉軸だけを扱い、GjiDirect の ON は GJI の前回のモードを復元するだけなので、#623 を閉じる ADR ではない。
- 常時の判定、英数側の統計判定。

## ADR-261 との関係

ADR-261 は「読む」、本 ADR は「書く」。ADR-261 が go で字形が読めるなら、字形の読みを優先し、本 ADR の段階 1 は動かさない。
ADR-261 が no-go(H1)でも、所有者判断により書き込みが効くことを前提とし、段階 1 で検証する。両 ADR の実機確認(インジケーター・実入力・SendInput の ON の一致)は、報告者への依頼を一回にまとめる。

## presync の停止との関係

presync の停止は「段階 A」として先に単独で行う(上記)。本 ADR の D3 は、段階 A が済んだ前提で書く場合は view の未知化と GjiFsm の `Reopen` が要らなくなり、簡素になる。
段階 A を行わない場合に備えて、D3 には両方の記述を残してある。どちらで進めるかは、段階 A の結果(段階 0 のログ)を見て決める。

## 未決事項

- Q1: class_name 単位の学習(`cache.toml`)のため、同じクラス名の別アプリへ広がる範囲(BUG-107 型)。
- Q3: 無操作の閾値、訂正の代理指標の N 秒、GJI の受け付け可能時間(段階 0 で実測)。
- Q7: 初回の窓切替は降格前(`Standard`/`ImmCross`)なので対象外になる。降格の契機とタイミング次第で、最初のエピソードを取りこぼす。
- Q8: ADR-256 の抑止(`note_awase_open_write`)だけを呼ぶか。`post_ime_refresh` を呼ぶか。
