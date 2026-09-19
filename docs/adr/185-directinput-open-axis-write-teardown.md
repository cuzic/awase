---
id: ADR-185
title: |-
  半角英数（ObservedEisu）を検出するとawaseが自らIME OFFを送ってしまう`EngineSync::DirectInput`を撤去する
  （BUG-146、ADR-178撤去プロジェクトの領域C）
summary: |-
  **ユーザー確認済みの症状**: 無変換で半角英数にすると、その後（約0.1〜1秒後）に直接入力（IME OFF）になる。
  原因はawase自身。`idle-conv-check`が`conv=0x10`（NATIVE=0）を読んで`ObservedEisu`と判定すると、
  `classify_conv_transition`が`EngineSync::DirectInput`を返し、`kp_apply_conv_engine_sync`
  （`key_pipeline.rs:1126-1168`）が、(1)`handle_engine_set_open(false)`でopen軸のbelief（`desired_open`、
  `last_intent`）を`false`に書き、(2)`apply_ime_open_with_belief`でIME OFFキー（VK 0x1A）を**実際に送信**する。
  実機ログ（2026-09-19 09:27:36、`60832d5d`ビルド）で`[apply-ime] GJI direct: send 0x001A (open=false)`→
  `outcome=Applied`→`SetOpen(false) applied → Off`を確認。ADR-090 A-2のwarrant強制は、この場面では
  `warranted`と判定して通していた（別の場面では`Unwarranted`で止まる）。ユーザーの設計原則:
  「IME ON/OFFは安定して観測できない。一方向の冪等キーではawaseは何もactuateせずbeliefの追随だけ、
  トグルはawaseが自分のbeliefに従ってactuateする」。conv（`ImmGetConversionStatus`）からopen軸を
  推測して書く・送るDirectInputは、この原則に反する。実測（同日10:54）でも、Ctrl+無変換でIMEを実際に
  OFFにしても、TSFネイティブ窓のconvは`0x19`（ひらがな）のままで、convはON/OFFの証拠にならない。
  本ADRは、DirectInput分岐から、open軸のbelief書き込みと全てのactuationを撤去する（決定1）。
status: |-
  **ドラフトv2（opus-adversarial-consult round1反映、round2前）**。実装未着手。
related_adr:
  - "ADR-178"
  - "ADR-182"
  - "ADR-184"
  - "ADR-090"
  - "ADR-086"
  - "ADR-084"
---

# ADR-185: `EngineSync::DirectInput`（半角英数検出時のIME OFF送信）の撤去

## ステータス

**ドラフトv2（2026-09-19）。** `opus-adversarial-consult` round1（Blocker2・Must-fix9）の指摘を、
ユーザーの設計原則と実機ログに照らして反映した版。実装前にround2で収束確認する。
[BUG-146](../known-bugs/BUG-146.md)に対応。

## 主目的（誤解しないこと）

新しい機構を足すことではない。**「conv（半角英数）を見て、awaseがIME OFFと判断し、実際にIME OFFを送る」
コードを撤去する。** 成功基準は、(1)無変換で半角英数にした後、IMEがONのまま（半角英数のまま）であること
（実機）、(2)削除量、(3)`send 0x001A`が半角英数の検出で出なくなること。

## 症状（ユーザー確認済み、2026-09-19）

「無変換で半角英数にしたあとは、たしかに直接入力になっています」（ユーザー）。GJI（キーマップはATOK系）で、
無変換単独タップ→半角英数（IME ON）→その後、awaseが直接入力（IME OFF）へ倒す。半角英数と直接入力は
打った文字が同じなので見た目で区別しにくく、その後もう一度無変換を押しても、IME OFF状態の無変換は
「不変」になるため、ひらがなへ戻らない（ADR-184が記録した「トグルが想定どおり動かない」の少なくとも
一部はこれが原因の可能性がある。ADR-184の再評価は本ADR後）。

## 設計原則（ユーザー指示、繰り返し確認済み）

1. IME ON/OFFを、安定した方法で**観測することはできない**。
2. IME ON/OFFを**一方向・冪等のキー入力**で行う場合: awaseは**何もactuateせず**、受動的にbeliefの
   追随だけを行う。
3. IME ON/OFFを**トグル**する場合: awaseが**自分のbeliefに従って能動的にactuate**してトグル動作させる。

実測の裏付け（2026-09-19 10:54、TSFネイティブ窓＋GJI）: Ctrl+無変換（awaseが`IME OFF (key combo)`）で
IMEを実際にOFFにしても、その後のidle-conv-checkの読みは4回とも`conv=0x00000019`（ひらがな）のまま
（`conv observation open=true reason=NativeToggleShadowOff`）。IMEを閉じてもTSFのconvは残るので、
convはON/OFFの証拠にならない。API読み取りも、この環境では`ime_on=None`（`read_ime_state_full`）。

## 現状のコードと観測

`classify_conv_transition`（`state/conv_classify.rs:100-140`）は、`input_mode_update`が`Some(ObservedEisu)`
のとき`EngineSync::DirectInput`を返す。`input_mode`のbelief更新自体は`InputModeObserved`で別に行われる。

`kp_apply_conv_engine_sync`（`runtime/key_pipeline.rs:1126-1168`）の`DirectInput`分岐（**現状の全効果**）:

1. `timer.kill(TIMER_IME_REFRESH)`（L1129）。
2. `handle_engine_set_open(target, false, ..)`（`platform_state.rs:268-322`）:
   `write_set_open_request`→`UserImeSetIntent{source: Command}`（`desired_open := false`、`last_intent`、
   `last_user_explicit_off_ms`）、`on_set_open_requested()`→`reset_detect_state()`
   （`observe_miss_monitor.record_success()`＋`force_guards.clear()`）、`ImeApplyRequested`のdispatch、
   `last_explicit_ime_action_ms := tick`（L320、idle-conv-checkの1.5秒セルフゲートの入力）。
3. `OpenBelief{effective_open: true, confident: true}`で`apply_ime_open_with_belief(order(false))`:
   **IME OFFの実送信**（`GjiDirect`なら`VK 0x1A`）。`record.caller = IdleConvCheckDirectInput`。
4. `on_ime_apply_complete(false, .., DriftCorrection)`→`post_ime_refresh()`（TIMER_IME_REFRESH再武装）、
   `ImeEffect::SetOpen(false)`→cold化、`gji fsm ImeOff`遷移。
5. 波及: 次のキーイベントで`Engine::compute_state`が`ctx.ime_on`（`=effective_open()`）を`input_mode`より
   先に評価するので`Inactive(ImeOff)`になり、`transition_activation`が`ActivationSync`の
   `SetOpen(false)`（2本目の実送信経路、`dispatch_ime_set_open`）と`EngineStateChanged{send_ime_key}`
   （3本目、`engine_off_ime_vk`の実VK送信）を発行する（`Inactive(NotRomajiInput)`なら両方抑止される）。

実機ログ（`awase-verify-adr182-20260919-182839.log`、09:27:36、`60832d5d`ビルド）:
`[conv-mode] Kana/roma → Eisu/roma`→`conv=0x10 → belief ObservedRomaji→ObservedEisu`→
`ObservedEisu 検出 → DirectInput`→`UserImeSetIntent`→`ImeApplyRequested`→
**`[warrant-shadow] ... warranted`**→**`[apply-ime] GJI direct: send 0x001A (open=false)`**→
`SendInput vk=0x1A`→`outcome=Applied`→`SetOpen(false) applied → Off (belief, unconfirmed)`→
`[stage-observe] belief_on=false explicit_intent=Some(false)`→`Engine deactivated (ime=false, ..
Inactive(ImeOff))`→2本目の`dispatch_ime_set_open{open=false}`。約130msで完了する（直後のキーで
反映される）。別の場面（07:35:44.993）では同じ経路が`Unwarranted`で止まっていたので、warrant強制は
この経路を常には止めない（**推定**: 分岐自身が先に`UserImeSetIntent(Command, false)`を書くため、
その後に計算されるwarrantが自己正当化になりうる。要確認）。

## なぜ残っているか（履歴）

BUG-051追補v3 pre-mortem #2は、この`desired_open=false`書き込みを、`is_eligible_for_ime_force_on()`が守る
3つのforce-ON経路（ADR-086`conv_mode_policy=force`の本番経路を含む）・`last_user_explicit_off_ms`・
`from_explicit_off_intent`のload-bearingな入力として残した。**force-ON経路は`621bf93c`で撤去済み**
（`is_eligible_for_ime_force_on`はコードに存在しない）。BUG-54型（20ms無限再送）の発生源
（`apply_force_on_for_imm_broken`の`conv_mode_policy=force`経路）も同時に消えた。
`schedule_ime_refresh(20)`は`ReportOpenInference`側にありDirectInput側には無い。

## 決定

**決定1（採用）: `DirectInput`分岐を撤去する。** `EngineSync::DirectInput`を廃止し（GJIの`DirectInput`＝
「IMEが本当にOFF」（`awase-gji-config/keymap.rs:21`の`STATUSES_WHEN_IME_OFF`）との命名衝突も解消する）、
`ObservedEisu`はinput_modeのbelief更新（`InputModeObserved`）だけで扱う。open軸への書き込みと、
上記1〜5の全効果を撤去する。engineの非活性化は`Inactive(NotRomajiInput)`（`desired_open`が`true`のまま
なので`ctx.ime_on=true`）で従来どおり行われ、`SetOpen(false)`と`engine_off_ime_vk`の実送信は
`suppress_set_open`/`suppress_ime_key`で抑止される。

各効果の扱い（round1 F1・F3・F4・D1〜D5への回答）:
- **実OFFの学習が失われる（D1）**: 原則1のとおりawaseは実IME状態を観測できない。外部要因（ADR-181の
  GJI外部echo、言語バー等）でIMEがOFFになっても、convは0x19のまま変わらない（実測）ので、現行の
  DirectInputも実OFFを検知できていない。DirectInputが「実OFFに正しく追随していた」場面は、convが
  `0x00`になる環境に限られ、TSFネイティブ窓では成立しない。したがって失う機能は無い（awase自身が
  行った操作のbeliefは、原則2・3どおり操作に基づく）。
- **`last_explicit_ime_action_ms`の1.5秒セルフゲート（F1/D3）**: awaseがactuateしなくなるので、
  ゲートで守る対象（自分のactuationの直後の読み取り）が無い。`note_explicit_ime_action`は残さない。
  半角英数中のidle-conv-check spawn頻度は、`classify_idle`が`ObservedEisu`に対し`None`を返すので
  belief更新は起きず、probeのみ飛ぶ（in-flightフラグが多重spawnを防ぐ）。実機で頻度を確認する。
- **`reset_detect_state()`（`force_guards.clear()`）（F3）**: force-ON撤去後の`force_guards`の
  読み手を全数確認し、消えて困らないことを実装前に確認する（未解決の疑問）。
- **`TIMER_IME_REFRESH`のkill/post（D4）**: DirectInputでは起きなくなる。TsfNativeでは`explicit_intent`
  確定後に恒久停止する設計（BUG-051）で、DirectInputの往復に依存していないことを確認する。
- **hwnd cache・`should_discard_imm_broken_cache`（D5）**: DirectInputが`last_user_explicit_off_ms`を
  偽の明示OFFとして更新していたため、Imm32Unavailable窓（Chrome/Edge）で古いONキャッシュが破棄されて
  いた。撤去後はこの偽の破棄が起きなくなる（ガードが外れる方向）。実IMEが半角英数（ON）のままなので、
  キャッシュされたONの復元は正しい。
- **復帰側の経路（D2）**: 半角英数→ひらがなの復帰で、`effective_open`が`true`のままなので
  `EngineSync::SetOpen(RomajiRecovered)`（engine ON同期）が選ばれる（現行は`ReportOpenInference`）。
  これは通常のひらがな復帰と同じ経路で、`desired_open=true`とconvが一致するので`already_matched`で
  実送信は起きない見込み。`conv_classify.rs`のテスト（`cold_start_eisu_still_detected`、smoke、
  oracle）の期待値変更と、実機で確認する。
- **`ObservedEisu`の入力（F2）**: `ConvMode::is_eisu()`はNATIVE=0（conv=0の実OFFと全角英数も含む）。
  `is_eisu_evidence`（本番未配線）は本ADRでは配線しない。open軸を書かなくなるので、`ObservedEisu`の
  曖昧さは`input_mode`（engineの活性判定）にしか影響せず、engineは半角英数でもIME OFFでも非活性が
  正しいので実害は無い（原則1のとおりON/OFFは区別できず、区別する必要も無い）。

**決定2: 回帰テスト。** `conv_classify.rs`のテスト（`DirectInput`を期待する箇所、oracle、smoke）を新しい
期待値に更新し、`platform_state`のテストで「`ObservedEisu`の観測後に`desired_open`・`last_intent`・
`last_user_explicit_off_ms`・`last_explicit_ime_action_ms`が変わらない」ことを固定する。実機ログ
（09:27:36の`conv=0x10`）から`ConvClassifyFixture`（`tests/journals/`）を追加する（`docs/journal-replay-
guide.md`の作法）。実OFF（conv=0）ケースの単体テストも置く。

**決定3: ガード・lintの更新（V3）。** `architecture_guard.rs:1299`（`.apply_ime_open_with_belief(`の
件数2→1）と説明コメント、`lints/actuation_call_guard/src/lib.rs:124-127`（`kp_apply_conv_engine_sync`
を削除）、`crates/xtask-adr-evidence/src/main.rs:227`、`actuation_decision_record.rs:282-289`のdoc
（6箇所→5箇所）、`ime_actuation_decision.rs:99`と`journal.rs:818`（`DecisionSite::
IdleConvCheckDirectInput`の削除。`tests/journals/`に該当fixtureは0件）、`intent_store_effective_open.rs`
と`architecture_guard.rs`の`EngineSync::DirectInput`言及コメント。`.claude/rules/complexity-budget.md`
（1-in-1-out）は、本ADRが許可リストの削除側なので障害にならない。

## 選択肢

- **B. `handle_engine_activation_sync`（belief直書きを避ける型）へ寄せる**: 採らない。conv由来の
  推測でopen軸を書く点が変わらず、原則に反する。
- **C. IME/キーマップ別のポリシー切替**: 採らない。MS-IMEでもGJIでも半角英数はIME ON
  （`docs/experiments.md`エントリ01、`VK_DBE_ALPHANUMERIC`は「IME ONのまま」）で、分岐の根拠が無い。
- **D. 何もしない**: 採らない。ユーザー確認済みの症状がある。warrant強制はこの経路を常には止めない。
- **E（round1推奨）. `report_conv_open_inference(false, ..)`＋`is_eisu_evidence`配線**: 採らない。
  convからopen軸の`false`を推測して`ObserverReported`として記録する案で、drift correctionが
  それを根拠にactuateしうる。原則1（観測できない）に反し、convがON/OFFの証拠にならないことも
  実測済み。

## 検証計画

1. 単体テスト（決定2）と、修正なしでは失敗することの確認（`DirectInput`分岐を残すと
   `desired_open`が`false`になる）。
2. **実機A/B（GJI、Windows Terminal）**: (a)ひらがな→無変換（半角英数）→**3秒以上待つ**→打鍵して半角英数
   のままであること（ログに`send 0x001A`・`Inactive(ImeOff)`が出ないこと）→(b)もう一度無変換でひらがな
   に戻ること→(c)Ctrl+無変換のIME OFFが従来どおり働くこと→(d)半角英数のまま別ウィンドウへ移って戻る。
3. `cargo test --lib`、`cargo test -p awase-windows`のガード/ゴールデン、Windowsターゲットのcheck・clippy、
   fmt。
4. `.claude/rules/fix-requires-evidence.md`: 回帰テストと`docs/known-bugs/BUG-146.md`の`fix_commits`。

## 明示的にスコープ外

- ADR-184（無変換のトグルをawase主導にする設計、別セッション）。本ADRが先に入ると、ADR-184の「無変換で
  半角英数→直接入力になる」観測の一因が消える。ADR-184の再評価は本ADRの後に行う。
- `ReportOpenInference`（`NativeToggleShadowOff`、`ObserverReported`として記録するだけの経路）。
- drift correction（維持方針）、ADR-090 A-2 warrant強制そのもの（ただし、この経路を止められない理由は
  未解決の疑問1として調べる）。
- MS-IMEの半角英数キー（`VK_DBE_ALPHANUMERIC`）の扱い（IME ON、既存）。

## 未解決の疑問（opus-adversarial-consultで検証してほしい点）

1. warrant強制が、この場面（09:27:36）で`warranted`と判定した理由。分岐が先に`UserImeSetIntent(Command,
   false)`を書くことによる自己正当化か。07:35:44.993の`Unwarranted`との違いは何か（`issue_open_warrant`
   の入力）。
2. `force_guards`（`reset_detect_state()`がクリアするもの）の、force-ON撤去後の読み手は何か。
   クリアされなくなって困る場面はあるか。
3. 復帰側（半角英数→ひらがな）が`SetOpen(RomajiRecovered)`（engine ON同期）へ変わることで、実送信が
   起きる場面はあるか（`already_matched`で潰れる根拠）。
4. TIMER_IME_REFRESHのkill/post往復が、DirectInputでしか起きていなかった依存はあるか（TsfNativeの
   恒久停止設計との関係）。
5. 「実OFFの学習が失われる」（D1）への回答（TSFネイティブ窓ではconvは実OFFを反映しない）は、TSFネイティブ
   以外の環境（Imm32Unavailable、ImmCross、Standard）でも成立するか。DirectInputはTsfNative専用
   （`kp_stage_idle_conv_check`）か。
6. round1の「命名衝突（GJIの`DirectInput`＝IME OFF）」の指摘を受けた`EngineSync`の再設計（variantを
   削除し`Option<..>`にするか等）に穴は無いか。

## レビュー経緯（記録）

- **round1（2026-09-19、opus）**: Blocker2（消費者は3つ＋副作用多数、convのNATIVE=0はIME ON半角英数と
  実OFFを区別しない）、Must-fix9、Should-fix9。推奨は案E＋F。**本ADR v2は、ユーザーの設計原則
  （観測できない→conv由来のopen軸推測を書かない）と実測（実OFF後もconv=0x19）に基づき案Eを採らず、
  案A（DirectInput撤去）を維持した**。一方、F1（`last_explicit_ime_action_ms`）・F3（`reset_detect_state`
  等）・F4（実送信が3本）・D2（復帰側の経路）・D5・V3（ガード/lint）・I2（命名衝突）は本文に反映した。
