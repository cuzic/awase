---
id: ADR-149
title: |-
  半角状態での物理IMEキー単独タップによるIME ON遷移で、awase自身が`VK_IME_ON`を3回重複送信する問題
summary: |-
  BUG-113の残置症状（半角状態での物理IMEキー単独タップで「@」が単発出力される）を実機ログで根本原因まで特定: 1回の物理キー押下に対しawase自身が`VK_IME_ON`を3回SendInputしていた（送信1=戦略の実送信、送信2=直後の随伴warmup、送信3=~100ms後のdelegateタイムアウト由来dispatchに付随する随伴warmup）。随伴warmupの送信可否を`outcome`で条件付ける修正（`should_send_accompanying_warmup`）で3回→2回に削減、実機で「@」再発なしを確認。opus-adversarial-consultで「どちらの送信が消えたか」の当初記述の誤り（実在しない`EmitWarmup`経路を原因誤認）・`ImmCrossProcessStrategy`のAppliedがSendInputを伴わないケース（Standardプロファイル限定、`can_use_imm32_cross_process()`でスコープして解消）・`eager_warmup_sent_ms`のラッチ喪失（`latch_eager_warmup_without_send`新設で解消）を指摘・反映。ユーザー報告「無変換キーでは『@』が再発する」を受け追調査した結果、**awase.exeを完全停止した状態でも無変換キー押下のたびに「@」が再現することを独立な`WH_KEYBOARD_LL`ロガー（`scripts/rawkbd_logger.ps1`）で実機確認**——この症状はawase側のactuationとは無関係と確定し、案α/案β（AlreadyMatchedのskip化・delegateのno-op強制再アサーション抑止）は誤った前提に基づいていたためrevert
status: |-
  実装済み・実機確認済み（PR #180）。無変換キーの「@」はGJI側の別問題として本ADRの対象外に切り離し
related_adr:
  - "ADR-019"
  - "ADR-092"
  - "ADR-132"
  - "ADR-140"
  - "ADR-141"
  - "ADR-147"
---

# ADR-149: 半角状態での物理IMEキー単独タップによるIME ON遷移で、awase自身が`VK_IME_ON`を3回重複送信する問題

## ステータス

**実装完了（2026-09-07）。設計確定（r0→r1→r2→r3、Opus敵対的レビュー3周目
まで実施・実機ログで最終確認済み）に基づき、`should_send_accompanying_
warmup`（`src/platform.rs`）を実装、`crates/awase-windows/src/
platform.rs::on_ime_applied`から配線した。ADR-132（`Phase 2`節の
`send_eager_tsf_warmup`/INV-B1'）とのコード上の整合性を確認済み——
本修正は`WarmupImeOn`の値構築（`resolve_warmup_ime_on`/INV-B1'が扱う
範囲）には一切触れず、`on_ime_applied`の`origin=WarmupOrigin::Actuated`
呼び出し自体を`outcome`で条件付けるのみであり、この呼び出しはADR-132
自身が「既知の限界（INV-B1'はこの経路には及ばない）」として明記していた
箇所（`platform.rs:1443-1452`のコメント）である。force-ONの`Applied`
（実際に`VK_IME_ON`を送信済み）はその送信自体でTSFが既に温まっている
はずのため、随伴warmupのスキップによる機能欠落は無いと判断した。**
`cargo test --lib`（998件）・windows target `cargo check`/`clippy`・
`architecture_guard`/`golden_scenarios`/`layer_boundary_guard`（109件）
とも緑。** 対象はBUG-113の残置症状（半角状態で無変換/変換キー単独タップ
時に「@」が単発で出る）。

**実機ソーク完了（2026-09-07）**: dragonflyg4実機（Windows Terminal +
GJI）で、半角/全角キー単独タップ・IME OFF状態からの無変換/変換系キー
単独タップ双方を反復し、「@」の再発なしを確認した。

**追記（実機ログ解析、送信回数の訂正、2026-09-07 opus-adversarial-consult
B1指摘で再訂正）**: `RUST_LOG=debug`で採取した実機ログを解析した結果、
当初「3回→1回」と見積もっていた送信回数は、実際には**3回→2回**
だった。ここまでは正しい。**ただし「どちらの送信が消えたか」の当初の
記述は誤りだった**——PRレビュー（opus-adversarial-consult、実コード
確認込み）で以下が判明した:

- `WarmupOrigin::Actuated`を伴う`send_eager_tsf_warmup(`呼び出しは
  リポジトリ全体で1箇所（本ADRがgateした`on_ime_applied`末尾の呼び出し）
  のみ。`feed_composition_event`に渡る`CompositionEvent::ImeOn`の
  ハンドラ（`tsf/composition_fsm.rs:165-172`）はstateをColdにする
  だけで`Response::consume()`（actionsが空）を返す。**`ImeOn`は
  `EmitWarmup`を一切出さない**——「送信2は`EmitWarmup`経由の別経路
  から発火し続ける」という当初の説明は誤り（実在しない経路を原因と
  誤認していた）。
- 実際に消えたのは**送信2**（`outcome=Applied`直後、戦略の実送信
  ~5.4ms後の随伴warmup）。**残っているのは送信3**（NICOLA同時打鍵
  タイマー満了→delegate機構由来、`outcome=AlreadyMatched`のため
  `should_send_accompanying_warmup`が意図通り「送る」側に倒す、
  gateした呼び出しと同一箇所からの~108ms後の再送）。

**この残存パターン（送信1直後は消え、~108ms後の重複だけ残る）は、
本ADRの「棄却した代替案」節が案Bを評価した際「3回→2回にしかならず、
しかも108ms遅延するだけで、BUG-113の機構は依然として成立する」と
自ら不十分と判定していた形そのものである。** それにもかかわらず実機
（半角/全角・変換キー、TsfNative/Windows Terminal）で「@」の再発が
確認されなかった点は未解明のまま残る——重複SendInputの「回数」より
「間隔」（直後 vs ~108ms後）が閾値に影響する可能性があるが、検証は
していない。追加修正（真の3回→1回化）は、実害が再度報告された場合に
別ADRとして検討する。

**最終確認**: 2026-09-07 05:40台、Windows実機（dragonflyg4）で
`grep "IME open axis delegated"`を実行した結果、
`2026-09-07T04:59:44.551031Z INFO awase::engine::engine: IME open
axis delegated (solo tap, key semantics absorption) → true`が、送信3
に対応する`execute_from_loop`呼び出し（`04:59:44.551486`）の
**わずか0.455ms前**に出現していることを確認した。これにより「送信3の
発生源はNICOLA同時打鍵タイマー満了によるdelegate機構」という特定が
確定した。

r0の診断（GJI自身のネイティブ処理とawaseのactuationが競合する、
BUG-110/ADR-132と同型）はOpus敵対的レビューで撤回、r1で「awase内部の
重複送信」に修正したが、r1時点でも重複の内訳（どの経路が何回送信するか）
は特定できていなかった。**r2で実機ログ（`RUST_LOG=debug`、outcome付き）
を再取得し、1回の物理キー押下に対しawaseが`VK_IME_ON`を正確に3回
SendInputすること、およびその3回それぞれの発生源をコード追跡で確定した。**

## 根本原因（r2で確定）: 1回の物理キー押下に対しawaseが`VK_IME_ON`を3回SendInputする

半角（belief `effective_open()=false`）状態で変換キー
（`VK_CONVERT`=0x1C、既定で`right_thumb_key`）を1回タップした実機ログ
（dragonflyg4、2026-09-07、Windows Terminal + GJI、TsfNative）:

```
t+0ms      [shadow-toggle] intent 昇格: vk=0x1C scan=0x79 action=TurnOn kind=PhysicalImeKey injected=false false→true
t+0.07ms   Engine activated (ime=true, romaji=true, japanese=true, user=true, reason=Active)
t+0.13ms   IME control: preconditions.ime_on = true (SetOpenRequest, origin=ActivationSync), ...
t+0.23ms   [apply-ime] GJI direct: send 0x0016 (open=true)                              ← 送信1（実送信）
t+1.6ms    [apply-ime] open=true eff=false conf=true → outcome=Applied
t+5.4ms    [tsf-eager-warmup] VK_IME_ON 送信 (origin=actuated)                          ← 送信2（実送信）
t+107.8ms  [apply-ime] GJI direct: shadow already ON (open=true), skip                  ← 2回目のapply（AlreadyMatched）
t+107.8ms  [apply-ime] open=true eff=true conf=true → outcome=AlreadyMatched
t+110.1ms  [tsf-eager-warmup] VK_IME_ON 送信 (origin=actuated)                          ← 送信3（実送信）
```

### 送信1・2の発生源（r1で特定済み）

1. `kp_stage_shadow_ime_toggle`（`crates/awase-windows/src/runtime/
   key_pipeline.rs:1084`）が、この物理キーを`IntentKind::PhysicalImeKey`
   として`write_physical_key`（`:1225-1232`）でbeliefを`false→true`に
   書く。この時点で`delegate_owned`（`mode_key_delegate_owns_shadow_
   toggle(vk) && effective_open()`、`:1150-1151`）は`effective_open()`
   がまだ`false`のため`false`——**shadow-toggleが担当する**。
2. 同一イベントの`build_input_context`（`kp_run_inner:273`）で
   `ctx.ime_on = true`となり、`Engine::check_active_transition`
   （`src/engine/engine.rs:351`）が`Inactive→Active`遷移を検知、
   `transition_activation`（`:429`）が`Effect::Ime(SetOpen{open: true,
   origin: ActivationSync})`を発行する。
3. `kp_stage_post_decision`→`handle_engine_activation_sync`
   （`state/platform_state.rs:347`）→`ImeApplyRequested`→
   `GjiDirectStrategy::apply`が実際に`VK_IME_ON`を送信する
   （**送信1**、`ime_controller.rs:172-175`、`shadow_on=Some(false)`
   だったため`AlreadyMatched`にならず実送信、`outcome=Applied`）。
4. その完了処理`platform.rs::on_ime_applied`（`:1374-1474`）が、
   `outcome`（`Applied`/`FallbackSent`/`AlreadyMatched`/`Failed`の
   いずれでも、`UnsafeToToggle`/`NotOwned`以外なら）を見ずに、
   `open==true`なら無条件で`send_eager_tsf_warmup`（`:1466-1467`）を
   呼び、`VK_IME_ON`ペアをもう一度送信する（**送信2**）。

### 送信3の発生源（r2で新規特定）: NICOLA同時打鍵タイマー満了→delegate機構

同じ変換キーは既定で`right_thumb_key`（NICOLA親指キー、
`src/config.rs:422`）にも設定されているため、`NicolaFsm`はこのKeyDownを
即座に確定させず`PendingThumb`として保留し、同時打鍵判定タイマー
（`simultaneous_threshold_ms`、既定**100ms**、`src/config.rs:424`）を
起動する。実測遅延**107.8ms**はこの既定値とほぼ完全に一致する。

タイマー満了時（`message_handlers.rs:661`の`on_timeout`）:

1. `Engine::on_timeout`（`src/engine/engine.rs:557-580`）→
   `NicolaFsm`の`resolve_pending_thumb_as_single`（`nicola_fsm.rs:2615-
   2624`）が、この時点で（送信1により）beliefが既にONになっていることを
   踏まえ、優先順位2の`delegate_to_open_axis = Some(TurnOn)`にヒットする
   （ADR-092決定D Step4b、ADR-141/147が扱う「Phase 3 delegate」機構）。
2. `apply_ime_open_request`（`engine.rs:591-607`）→
   `ime_set_open_effects`（`:853-872`）——`prev_activation`は既に`Active`
   のため`was_active == now_active`となり`transition_activation`自体は
   空を返すが、`ime_set_open_effects`は活性状態が変化しなくても明示的に
   `Effect::Ime(SetOpen{open: true, origin: ExplicitUserAction})`を
   追加発行する（drift補正のための強制再アサーション、`engine.rs:865`）。
3. `message_handlers.rs:667`の`execute_decision`→`runtime/mod.rs:528`の
   `executor.execute_from_loop`（**キーボードフックを経由しない実行経路**、
   `executor.rs:186-203`のdoc参照）→`dispatch_ime_set_open(open=true,
   generation=None)`→`GjiDirectStrategy::apply`は、送信1が`Applied`
   だったため`shadow_on=Some(true)`となっており`gji_direct_already_
   matches(Some(true), true) = true`→**`AlreadyMatched`（実送信なし）**。
4. しかし`on_ime_applied`の随伴warmup呼び出し（`platform.rs:1459`の
   `if open {`）は**outcomeを見ない**ため、`AlreadyMatched`でも
   `send_eager_tsf_warmup`が発火し、`VK_IME_ON`ペアを3度目送信する
   （**送信3**）。

BUG-113が実機A/Bで確立した必要十分条件は「**重複したSendInputがGJIの
TSF composition追跡を乱す**」ことである（`ime_controller.rs:128-140`の
`GjiDirectStrategy` doc、Windows Terminal・Google日本語入力・
PowerShell(PSReadLine)の組み合わせで確認済み）。**送信1〜3は、この条件を
awase単独で満たす**——GJI自身のネイティブpassthrough処理が同時に
起きているかどうかに関わらず、この3回の送信だけで「@」の発生条件が
揃う。

### なぜ二度押しではないと言えるか

ユーザーから「2回押した可能性は」という指摘があったが、以下の理由で
構造的に除外できる:

- 2回目のdispatchは`execute_from_loop`というスパン名を経由している。
  物理キー押下は必ず`DecisionExecutor::execute_from_hook`
  （`executor.rs:175`）を通り、`execute_from_loop`（`:204`、
  「キーボードフックを経由しない全ての`Decision`実行経路」専用）には
  構造的に到達できない。
- 仮に2回目の物理`VK_CONVERT` KeyDownがあったなら、その時点でbeliefは
  既にONのため`delegate_owned`が`true`になり、`key_pipeline.rs:1194`の
  debug ログ「`[shadow-toggle] vk=0x1CはFSM delegate所有 →
  belief書き込み/actuationをスキップ`」が必ず出るはずだが、実機ログには
  一度も現れない。
- 遅延107.8msが`simultaneous_threshold_ms`の既定値100msとほぼ完全に
  一致することも、人間の意図的な連打より機械的なタイマー起因である
  ことを裏付ける。

### なぜ「OS/ドライバによる疑似エコー」ではなく「awase自身の重複送信」と言えるか

[[project_bug113_vk_kanji_pseudo_echo_2026_09_06]] は、awaseの
actuation SendInputの約0.5〜1秒後に観測される`self_injected=false`の
孤児KeyUp/別VKのKeyDownを「OSがawaseの送信を疑似エコーしている」と
仮説立てていたが、これについて形（shape）とタイミングを分けて再評価する:

- **形**: `kp_stage_shadow_ime_toggle`のBUG-14コメント（`:1117-1121`）
  が記録する実例——「2026-07-06実機: 外部注入`VK_DBE_HIRAGANA` down+up
  (hook上では`0xF0 up`+`0xF2 down`に翻訳、**0.5ms間隔**)が
  `PhysicalImeKey`と誤読され…」——は、前セッションが疑似エコーの根拠に
  した「孤児KeyUp→別VKのKeyDown」というパターンそのものが、IMEモード
  キー1回押下における正常なhook上の翻訳結果であることを示す一次証拠。
- **タイミング**: ただし上記コメントの間隔は0.5msであり、前セッションが
  観測した0.5〜1秒とは3桁違う。今回のクリーンな再現では、
  `self_injected=false`かつ`vk`が一致しない孤児イベントは一切現れず、
  代わりに上記の完全に説明のつく内部連鎖（送信1〜3）だけが記録された。

したがって本ADRは、decision9（形ベース判別、`ModeKeyShapeTracker`）の
実装を保留し、根本原因をawase内部の3重送信として確定する。

**補足（`@`の出自の反証）**: NICOLA自身のローマ字/かなエンジンが
チョード入力の結果として`@`を正当に出力する経路が無いかを確認した。
`layout/*.yab`全5ファイルと`src/kana_table.rs`のいずれにも`@`は
1文字も出現しない（`grep -n '@' layout/*.yab`はヒット0件）。
**したがって「`@`はawase自身のローマ字/かなエンジン由来」という説は
完全に否定できる。** なお`@`の表示形そのものの由来（重複SendInputの
結果としてなぜ文字コード`0x40`が解決されるのか）は依然未確定
——`docs/known-bugs.md`のBUG-113 2026-09-05追記が、Windows Terminalの
`terminalInput.cpp`を読んでも`ToUnicodeEx`相当のクローズドソース境界に
突き当たったと記録しており、本ADRもこの限界（ブラックボックス）を
引き継ぐ。

## 決定

**採用: 随伴eager warmupを「同じapplyで戦略が実際に`VK_IME_ON`を送って
いない場合」に限定する。**

対象は`crates/awase-windows/src/platform.rs:1459-1467`。現状:

```rust
if open {
    self.output.mark_composition_cold(ColdReason::SetOpenTrue);
    receipt.settle(self);
    self.output.send_eager_tsf_warmup(warmup_ime_on, WarmupOrigin::Actuated);
}
```

は`outcome`を一切見ない。`outcome`は同関数スコープに既にある
（`:1400`で`effective`の算出に使用）ため、判定材料は揃っている。
`outcome == ImeOpenOutcome::Applied`（戦略が実際に`VK_IME_ON`を送った）
の場合は随伴warmupを送らない、`AlreadyMatched`（実送信が無かった——
TSFがウォームアップされていない）の場合は従来どおり送る、という形に
変更する。

この変更により1打鍵あたりの`VK_IME_ON`送信は**3回→1回**（送信1のみ）
になる。

### なぜこの方式を採るか

- core（`awase`クレート）を一切触らない → ADR-019、および
  `decision.rs:276-280`が`InputContext`自身に課す明文規約
  （「OS由来の瞬間値のみ」「このフィールドを増やす前にEngine内部状態で
  代替できないか検討」等）のいずれにも抵触しない。
- belief・actuationとも従来どおり残る → belief ON×実IME OFFの乖離
  リスクが発生しない。
- `applied`が正しくON側に確定する → `apply_force_on_for_imm_broken`
  （TsfNative向け周期force-ON機構）が誤って追加送信することもない。
- `platform.rs`は`fix-requires-evidence.md`の「IME actuation合流点」
  表にも`.git/hooks/pre-push`の正規表現にも既に含まれる対象ファイル。

### 実装前に確認すべきこと（必読、ADR-132との整合性）

この随伴warmupは`ADR-132「Phase 2」`（`platform.rs:1443-1452`の
コメント）が明示的に「既知の限界」として言及している経路でもある:

> この`warmup_ime_on`は`from_actuated`（実actuation直後の確定値）
> 由来であり、`resolve_warmup_ime_on`が課す`off_drift_active`ゲートを
> 通らない——force-ON（`apply_force_on_for_imm_broken`）が
> `SetOpen(true)`を適用した直後にもここを通るため、drift correctionが
> OFF方向へ送り続けている最中でも随伴warmup（`VK_IME_ON`）が飛びうる。
> INV-B1'は**この経路には及ばない**、既知の限界。

force-ON経由の`SetOpen(true)`適用も同じ`on_ime_applied`を通るため、
`outcome`ベースの条件を追加すると、force-ONの`Applied`（実際に
`VK_IME_ON`を送った場合）でも随伴warmupがスキップされることになる。
これはADR-132が意図した「TSFをウォームさせ続ける」目的と衝突しない
と考えられる（force-ONの`Applied`は実際に`VK_IME_ON`を送っており、
TSFは既にその送信でウォームアップされているはずのため）が、**これは
推論であり、実装前にADR-132全文を読み、force-ON関連の実機ソーク
（該当する場合）と矛盾しないか確認すること**。warmupを完全に撤去
するのではなく「戦略が既に同じVKを送ったなら二重に送らない」という
最小限の変更に留めるのが安全側。

## 検討し棄却した代替案

### 案A（r0/r1の当初決定、棄却）: `Engine::transition_activation`が発行する強制`SetOpen`をcoreの`InputContext`拡張で抑制する

r0が提案していた方式。`InputContext`型自身の明文規約
（`decision.rs:276-280`）に抵触し、34箇所の構築点（Linuxスタブ
クレート含む）に波及し、`build_input_context`の引数8個目でclippy閾値
（8）に抵触する。**コスト対効果が最も悪く不採用。**

### 案B（r1で提案、r2で無効と判明、棄却）: `ActivationSync`由来のSetOpenを`kp_stage_post_decision`で抑制する

core非変更という利点はあったが、**この抑制では症状が解消しない**
ことがr2の実機ログ解析で判明した:

1. 送信1（`GjiDirectStrategy`の実送信）と送信2（その随伴warmup）は
   消える。
2. しかし`applied_snapshot`がON側に確定しないため、108ms後の
   delegate由来の2回目dispatchで`shadow_on`が`Some(false)`のまま
   となり、`gji_direct_already_matches(Some(false), true) = false`
   →**今度はこちらが実送信に変わる**。
3. その後outcome=Appliedの随伴warmupも飛ぶ。

→ **「3回→2回」にしかならず、しかも108ms遅延するだけ**で、BUG-113の
機構（重複SendInputがGJIのTSF composition追跡を乱す）は依然として
成立する。加えて`applied`がONに確定しないため、
`apply_force_on_for_imm_broken`（TsfNative向け周期force-ON機構）が
代わりに`VK_IME_ON`を送信するリスクも残る。**唯一の正当な送信
（送信1）を狙い撃ちしており、対象を取り違えていた。**

### 案C（新規発見、別ADRへ分離を推奨）: delegateとshadow-toggleの排他性を修復する（送信3自体の発生を止める）

r2の調査で、送信3を発生させている「delegate機構がbelief OFF→ON遷移の
打鍵で二重に評価される」という事象自体が、[ADR-141](141-henkan-muhenkan-delegate-inactive-recovery.md)
のC2修正（コミット`246338bc`）が明記する不変条件——
`runtime/mod.rs:471-474`「実際にdelegateとshadow-toggleのどちらが
処理するかは`mode_key_delegate_owns_shadow_toggle`の
`&& effective_open()`ゲートが**実行時に排他的に決める**」——が、
**OFF→ON遷移の打鍵に限って成立していない**ことに起因すると判明した。

理由は評価タイミングのズレである:

- 消費点2（`mode_key_delegate_owns_shadow_toggle`の
  `&& effective_open()`）は`kp_run_inner:270`、`build_input_context`
  より**前**に評価される→このとき belief は OFF →
  `delegate_owned = false`→shadow-toggleが担当し、**beliefをONに
  書き換える**。
- 消費点1（`resolve_pending_thumb_as_single`の`delegate_to_open_axis`）
  は**その約100ms後**、`on_timeout`で評価される→このときbeliefは
  既にON（消費点2自身が書き換えた）→**delegateも発火する**。

つまり「ゲートが参照する状態を、ゲート自身のもう一方の分岐が書き換えて
から、もう一つの消費点が評価される」という構造であり、`&&
effective_open()`は「同時刻に片方だけ」しか保証せず「1打鍵を通じて
片方だけ」は保証しない。現状この二重評価が実害を生んでいない
（`@`の観点では）のは、送信3が`AlreadyMatched`で握り潰されるという
**偶然の産物**——送信1が`Applied`を返し`applied_snapshot`をONにして
いたから成立しているに過ぎない（案Bのように送信1を止めると、この
偶然が崩れて送信3が実送信に変わる、上記参照）。

「@」の解消には案（決定節の方式）だけで十分であり、この排他性の穴は
独立した正しさの問題（open軸の二重処理）として**別ADRとして起票する
価値がある**が、本ADRのスコープには含めない。**ただし、ADR-141の
「排他的に決める」という記述が実際には成立しないケースがあるという
事実は、ADR-141への追記または`docs/known-bugs.md`に必ず記録すること**
——記録しないと次のセッションが同じ「排他的に決める」という記述を
信じて別の変更を積み重ねるリスクがある。

### 案D（r3で検証、より根本的な代替案、別ADRへ保留を推奨）: `IntentKind::PhysicalImeKey`起因の場合はactuation自体を発行しない

ユーザーから、より根本的な設計提案があった:「GJI/MS-IME自身のキー
マップ検出（`IntentKind::PhysicalImeKey`）で判明したIME ON/OFFキーは、
GJI自身のネイティブpassthrough処理に完全に委ね、awase自身は
`SendInput`を一切送らない。actuationが必要なのはawase自身の明示設定
（`IntentKind::SyncKey`、Ctrl+変換等）の場合のみ」。

議論の過程で、当初の「GJI側委任はfire-and-forgetでconfirmできないから
危険」という反論は**誤りと判明した**——TsfNative（`FeedbackPolicy::
Blind`、実読み戻し不能）では、awase自身が`SendInput`しても
`outcome=Applied`は「送信呼び出しが成功した」以上の意味を持たず、
「実際にIMEが開いた」ことのconfirmにはならない。self-actuationと
trust-GJIはconfirmabilityの観点では対称である。

Opus敵対的レビュー（r3）でこの設計を検証した結果:

1. **「belief追随とactuationの分離」という切り分け自体は既存コード
   構造と整合する**（`write_physical_key`/`handle_engine_activation_
   sync`はWin32呼び出しを一切伴わず、実`SendInput`は`GjiDirectStrategy::
   apply`の`send_ime_mode_key`呼び出し1点に閉じている）。
2. **配線（`IntentKind`をactuation地点まで届ける経路）は、新規の
   グローバル状態なしで実現可能**——`ImeModel.last_intent.source`
   （`UserIntentSource::PhysicalImeKey`/`SyncKey`/`Command`）が既に
   この情報を保持しており、`ControlLog.shadow_on`と同じパターンで
   `ImeControlView`に流せば、即時経路（ActivationSync）と約100ms後の
   delegate経路の両方に届く。ただし`last_intent`はstickyなため、
   30秒後のdrift correction等、無関係な後続actuationまで誤って
   巻き込む——`EventSource::SelfActuated`の起案元文字列との二重条件が
   必要になる。
3. **「self-actuationの方が信頼性が高い」という非対称性は、
   confirmabilityではなく「状態カバレッジ」の軸で実在する。**
   `VK_IME_ON`はGJIがTSF層でネイティブに処理する、全状態で有効な
   冪等キーであることが実機確認済みだが、`classify_and_push`
   （`crates/awase-gji-config/src/keymap.rs:281-283`）はTurnOn分類に
   `on_statuses`が`"DirectInput"`を含むことを要求しないため、
   **「TurnOnと分類されたが、IME OFF状態では何も起きないキー」が
   構造的に作れる**。ADR-147の構造的保証（「TurnOn分類のキーはどの
   状態にもOFF相当のバインドを持たない」）は「OFFにしてしまわない」
   ことしか保証せず、「OFFからONにできる」ことは保証しない。
4. **Blocker: `Applied`を詐称する（送っていないのに送った扱いにする）
   と、TsfNativeにおける唯一のON方向救済機構
   `apply_force_on_for_imm_broken`が構造的に永久停止する**
   （`state/ime_actuation.rs::force_on_attempt_allowed`が
   `applied`がON確定済みの場合に早期returnするため）。round 1の
   Major 3（belief乖離が検知できない）が、この設計では「検知できない」
   から「救済経路が構造的に閉じる」へ悪化する。
5. **決定打: この設計を採用しても、随伴eager warmup
   （`platform.rs:1459-1467`）は`outcome`を見ないため送信2回が
   残ってしまう。** warmup gating（本ADRの決定）はどちらの設計でも
   前提条件であり、それを先に入れた時点で残り送信は1回になり、
   BUG-113の確立済み機構（重複SendInput）はもう成立しない。案Dが
   追加で削るのは残り最後の1回であり、限界効用はほぼゼロ。

**結論（r3）: 本ADRの決定（随伴warmupのoutcome gating）を維持し、
案Dは別ADRとして起票・保留する。** 再検討の条件は、(a) TsfNativeでも
IME open状態を読める観測手段が手に入ったとき（force-ON救済の代替が
用意できる）、(b) `classify_and_push`に「`on_statuses`が
`DirectInput`を含むこと」をTurnOn分類の必要条件として追加し、
状態カバレッジの穴を塞いだとき、の両方が揃った時点。

**独立した発見（案Dの採否と無関係、`docs/known-bugs.md`への記録を
推奨）**: `classify_and_push`のTurnOn分類が`DirectInput`状態を要求
しないという穴は、ADR-147のdelegate機構（`resolve_pending_thumb_as_
single`、既に物理キーをSuppressしてGJIに届けない設計）が**現時点で
既に**この分類に依存しているため、対象キーがDirectInput状態で無効
なら、IME OFFからの復帰が黙って失敗しうる（BUG-115の再来）。これは
案Dの採否と無関係な既存の潜在バグとして記録する価値がある。

参考: `feedback_immcross_owns_kanji`メモリの原則（「ImmCrossアプリに
は物理IMEキーを見せない」）と案Dは矛盾しない——「open軸の所有権は
アプリ/IME環境ごとに排他的でなければならない」という同一の上位原則の
裏表であり、ImmCross環境ではawaseが所有し物理キーを見せず、GJI検出
キー環境ではGJIが所有しawaseが手を出さない、という対称的な設計。

## 必須条件

1. **✅ 完了（2026-09-07）**: `IME open axis delegated (solo tap, key
   semantics absorption) → true`（`src/engine/engine.rs:600`のINFO
   ログ）が実機ログに存在し、`2026-09-07T04:59:44.551031Z`——送信3に
   対応する`execute_from_loop`呼び出し（`:44.551486`）の0.455ms前
   ——に出現することを確認した。「送信3の発生源はNICOLA同時打鍵タイマー
   満了によるdelegate機構」という特定が確定した。
2. **✅ 完了（2026-09-07）**: `should_send_accompanying_warmup(outcome:
   ImeOpenOutcome) -> bool`として`src/platform.rs`に切り出し、
   `Applied`/`FallbackSent`（スキップ）・`AlreadyMatched`/`Failed`
   （送信）の各分岐をLinuxで実行可能な単体テスト2件（`cargo test --lib`）
   で固定した。`UnsafeToToggle`/`NotOwned`は呼び出し元（`on_ime_applied`）
   側で既に早期returnされているため対象外。
3. **✅ 完了（2026-09-07）**: `docs/known-bugs.md`のBUG-113節に、
   1打鍵あたり`VK_IME_ON`が3回送信されていた内訳（送信1=戦略の実送信、
   送信2=送信1直後の随伴warmup、送信3=約100ms後のdelegateタイムアウト
   由来dispatchに付随する随伴warmup）・実測時刻・修正内容を追記し、
   BUG-113の見出し要約も最新の確定原因に更新した。
4. **✅ 完了（2026-09-07、`docs/known-bugs.md`記録の形で対応）**: 案Cで
   発見した排他性不変条件の不成立（上記「検討し棄却した代替案」参照）を、
   BUG-113節に「独立して発見した未解決事項」として記録した。
5. **✅ 完了（2026-09-07、`docs/known-bugs.md`記録の形で対応）**: 案Dの
   r3レビューで発見した独立の潜在バグ（`classify_and_push`の
   `on_statuses`が`"DirectInput"`を要求しないため、
   ADR-147のdelegate機構がIME OFFからの復帰に黙って失敗しうる、BUG-115
   の再来）を、BUG-113節に記録した。
6. `tuning-constants.md`の実測義務は、タイミング定数を変更しないため
   対象外。

## 残された未検証事項

- 案Cで発見した排他性の穴（delegate/shadow-toggleの二重評価）自体の
  修正は別ADRのスコープとし、本ADRでは扱わない。
- 3回の重複SendInputが具体的にどうGJI/Mozc内部のコンポジション
  バッファを壊し「@」という1文字に帰結するのかは、awase側からは
  観測できないブラックボックスのまま。修正の効果検証は実機での
  「重複が消えた後、『@』が再現しなくなるか」に依存する。

## 追記（2026-09-07）: 無変換キー単独タップの「@」は本ADRの対象外——awaseとは無関係と確定

ユーザー報告「変換キーでは『@』が再発しないが、無変換キーでは毎回
再発する」を受けて追調査した結果、**無変換キーの「@」は本ADRが扱う
機構（awase自身の重複SendInput）とは無関係であることが実機で確定した**。
次のセッションが同じ調査を繰り返さないよう、経緯と根拠を残す。

### 調査の経緯（誤って辿った道）

1. 当初、無変換キー押下時のログを見ると`vk=0xF2 scan=0x70`
   （かなキー）が引き金として記録されており、「無変換キーの物理キー
   がリマップされているのでは」と疑った。レジストリのScancode Map・
   HKCU Substitutes・他のawaseプロセス残留を調査したが、いずれも
   該当なし。
2. `awase.exe`とは完全に独立した、生の`WH_KEYBOARD_LL`フックだけを
   記録する診断ツール（`scripts/rawkbd_logger.ps1`、本コミットに
   同梱、awaseのプロセス・コードを一切経由しない）を新設し実機で
   検証した結果、**無変換キー(`scan=0x7B`)は常に正しく記録される**
   一方、`vk=0xF2 scan=0x70`（かなキー相当）のイベントが、無変換
   キー押下とは独立したタイミングで不定期に発生することを確認した
   （`injected=False`、Alt+Tab等のフォーカス変更と近い時間に観測
   されることが多いが、規則的ではない）。
3. `should_send_accompanying_warmup`（案α: `AlreadyMatched`も
   skip+latchにする）・delegateのno-op強制再アサーション抑止
   （案β）を実装し、実機A/Bでモードを巡回して検証したが、**いずれの
   組み合わせでも無変換キー押下のたびに「@」が再発し続けた**。
4. **決定的な検証**: `awase.exe`を完全に停止した状態で、独立ロガー
   のみを動かして無変換キーを複数回タップしたところ、**awaseが
   一切動作していないにもかかわらず、無変換キーを押すたびに「@」が
   毎回再発することを確認した**（ユーザー報告、2026-09-07）。

### 結論

**無変換キー単独タップで「@」が出る現象は、awase側のactuation・
送信回数とは完全に無関係である。** awaseが介在しなくても100%再現する
ため、GJI（Google日本語入力）自身がWindows Terminal上で無変換キーの
物理押下をネイティブに処理した結果として生じている可能性が高い
（GJIのキーマップ設定、あるいはWindows Terminal側のTSF処理との組み
合わせ）。したがって本ADR（および将来のいかなるawase側コード修正）
では原理的に解決できない。**別調査（GJI側のキーマップ・config1.dbの
確認）として切り離すこと。**

案α・案β・スパイク検証用のモード巡回コード（`src/bug113_spike.rs`
ほか）は、この誤った前提に基づいていたため撤回した（`git log`参照、
`.claude/rules/experiment-logging.md`の規約に従いrevertコミットの本文
に失敗条件を記録済み）。`scripts/rawkbd_logger.ps1`（独立キーボード
フックロガー）は原因切り分けに有用だったため、診断ツールとして
リポジトリに残す。

## 関連

BUG-113（[[project_bug113_vk_kanji_pseudo_echo_2026_09_06]]、
[[project_bug113_residual_at_sign_root_cause_2026_09_06]]）、
[ADR-140](140-ime-probe-actuation-quiet-window.md)（probe/actuation
quiet window、別種の競合を別経路で解決した先例）、
[ADR-147](147-thumb-key-delegate-defers-to-user-passthrough.md)
（develop側に既存——対象コードパス（Phase 3 delegate/`resolve_pending_
thumb_as_single`、消費点1）は本ADRの送信3の発生源と同じ関数だが、
問題の性質は異なる：ADR-147はユーザーのパススルー設定を尊重する話、
本ADRはoutcomeを見ない随伴warmupの重複送信の話）、
[ADR-141](141-henkan-muhenkan-delegate-inactive-recovery.md)
（C2修正、コミット`246338bc`——本ADRが「排他的に決める」という
その不変条件の不成立を発見した）、BUG-110/ADR-132（Phase 2節の
随伴warmupに関する既知の限界コメントが本ADR決定の実装前提条件、
`platform.rs:1443-1452`）、BUG-115（`classify_and_push`の
DirectInputカバレッジの穴——上記「案D」参照——が再燃しうる領域、
`crates/awase-gji-config/src/keymap.rs:281-283`）。
