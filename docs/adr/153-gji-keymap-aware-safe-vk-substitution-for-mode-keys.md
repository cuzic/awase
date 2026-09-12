---
id: ADR-153
title: |-
  無変換/変換キー単独タップの IME ON/OFF/Toggle を、GJI 側のキーマップ設定に頼らずユーザーが awase 側で直接指定できるようにする
summary: |-
  BUG-113残置症状（半角状態で無変換キー単独タップ時にWindows Terminal+GJIで「@」）の機序を実機3段階検証で確定: GJI自身のTSFキー横取り（`ITfKeyEventSink`、GJIが無変換/変換に何らかのIME制御コマンドを割り当てている場合のみ発火）が原因。当初の「物理キーリマップ」案（ADR-110流用）はB1〜B3のBlockerで撤回、無変換/変換単独タップ確定後のIME ON/OFF/Toggleを、GJI/MS-IME自動検出に頼らずawase自身の明示config（`*_solo_tap_ime_action`）で直接指定できるようにする決定1に転換。opus-adversarial-consult r1〜r9で9ラウンドの敵対的レビューを経て収束（B1〜B14すべて解消）、3ケース分割（belief ON/OFF→ON遷移/OFF維持）・one-shotマーカー（B13/B14対策）・優先順位表拡張などの実装レベルの詳細まで確定
status: |-
  **決定1実装済み・developマージ済み（PR #185）。ケース2("on")は実機で最終確認済み——マージ後の実機再検証で連鎖バグ2件（BUG-122・BUG-123）を発見・修正し、PR #186でdevelopマージ済み。ケース3("off")は全面撤回が別の「@」退行（BUG-124）を招いたため「抑止のみ・actuateしない」設計に作り直し、実機でも「@」再現なし・正常動作を確認済み（PR #186）**。コア/windows crateの全テスト・clippy/fmt green
related_adr:
  - "ADR-019"
  - "ADR-080"
  - "ADR-091"
  - "ADR-092"
  - "ADR-110"
  - "ADR-114"
  - "ADR-119"
  - "ADR-135"
  - "ADR-141"
  - "ADR-147"
  - "ADR-149"
  - "ADR-151"
  - "ADR-152"
  - "ADR-154"
---

# ADR-153: 無変換/変換キー単独タップの IME ON/OFF/Toggle を、GJI 側のキーマップ設定に頼らずユーザーが awase 側で直接指定できるようにする

## ステータス

**決定1実装済み（2026-09-08、PR #185でdevelopマージ済み）。ケース2
（"on"）は同日中に実機再検証で2件のバグ（BUG-122・BUG-123）を連鎖的に
発見・修正し、実機で最終確認済み（下記2026-09-08追記2・3参照）——本ADR
の「ケース2は実機確認済み」という当初の記述は誤りだったと判明している。
ケース3（"off"）は同日中に一度全面撤回したが、それが別の「@」退行
（BUG-124）を招いたため「抑止のみ・actuateしない」の形に再設計した
（下記2026-09-08追記1・4参照）——実機での最終確認は次のステップで
実施予定。** `cargo test --lib`（コア1004件）・`cargo nextest run -p
awase-windows`（113件）・clippy/fmt はすべてgreen。

**追記4（2026-09-08、ケース3の全面撤回が「@」を再発させたため「抑止
のみ」の形に再設計、BUG-124）**: 追記1の全面撤回版（強制actuateだけで
なく生キーの抑止も含めて撤去）を実機ビルドし`"off"`設定で再検証した
ところ、**「@」が再現し続けた**。原因は、抑止まで撤去した結果GJI自身が
無変換/変換キーを生で受け取るようになり、本ADRが解決しようとしていた
BUG-113の根本原因（GJI自身のTSFキー横取り）そのものに逆戻りしていた
こと。追記1時点で参照していた実機実験（`docs/experiments.md`エントリ25
Phase3）は「生キーをSuppressし、かつ何も送らなければ『@』は完全に
消える」ことを既に示していた——**問題は「抑止」ではなく「強制
actuate」の方**だったにも関わらず、全面撤回の設計時にこの区別を
見落とし、両方まとめて撤去してしまっていた。`explicit_ime_action_
target`を再び`Option<bool>`に戻し、"off"×既にOFFの場合は
`explicit_ime_action_consumed`マーカーを立てて`return false`する
だけの「抑止のみ」実装（`apply_ime_open_with_belief`等のactuationは
一切呼ばない）に再設計した。KeyUpのM19ペアリング早期分岐も復活させた。
詳細は`docs/known-bugs.md` BUG-124節参照。

**追記3（2026-09-08、`always_suppress=false`環境での二重信号送出を発見・
修正、BUG-123）**: 追記2のBUG-122修正版を実機ビルドし再検証したところ、
「@」は再現しなくなったが、**半角状態から無変換単独タップすると、
ひらがなを経由せずいきなりカタカナに切り替わる**新たな症状が見つかった
（ユーザー報告）。原因は`resolve_pending_thumb_as_single`が
`explicit_action_consumed=true`（ケース2が既にこの打鍵を処理済み）でも
優先順位3/4（`delegate_to_open_axis`/`ModeKeyConfig`）へフォールスルーして
おり、`muhenkan_solo_tap_always_suppress = false`（このユーザーの実設定）
環境では優先順位4が生の`VK_NONCONVERT`を**もう一度**送出していたため——
1回のタップがGJIへ「ケース2のIME ON化」＋「100ms後の生キー再送」という
2つの信号として届き、GJIが後者をかな⇄カタカナ切替と誤認していた。
`explicit_action_consumed`のときは優先順位3/4を評価せず即座に打ち切る
よう修正した。詳細は`docs/known-bugs.md` BUG-123節参照。実機再検証で
このケース2の修正（BUG-122+BUG-123）が正しく動作することを確認済み
——半角状態から無変換単独タップで正常にひらがな入力へ切り替わり、
「@」もカタカナへの誤遷移も再現しないことを確認した。

**追記2（2026-09-08、ケース2のbelief書き込みno-opバグを発見・修正、
BUG-122）**: PR #185マージ後の実機再検証（dragonflyg4、`muhenkan_solo_tap_
ime_action = "on"`）で、下記「実機検証結果」が「確定的に修正を確認」と
記録したケース2が、**実際には一度もbeliefを書き込んでいなかった**ことが
判明した。`kp_stage_shadow_ime_toggle`はケース2発火時に`IntentWitness::
from_physical(event)`経由でしか`write_physical_key`を呼ばないが、
`from_physical`は`shadow_action.is_some()`しか witness として受理せず、
ケース2は`shadow_action`を一切経由しない設計のため、常に`None`が返り
`write_physical_key`が黙ってスキップされていた——「OFF→ON へ昇格」ログ
自体は書き込みの**前**に出るためログだけでは検知できなかった。詳細・
修正内容は`docs/known-bugs.md` BUG-122節を参照。当初の実機確認がなぜ
「成功」に見えたかは確定できていない（推測は known-bugs.md 参照）。

**追記1（2026-09-08、ケース3を一度全面撤回。後に追記4で「抑止のみ」の
形へ再設計——本節は当日の経緯の記録として残す）**: 下記「実機検証結果」
が記録したケース3の未解決症状は、後続の実機A/B切り分け実験で根本原因が
確定した（`docs/known-bugs.md` BUG-113節・`docs/experiments.md`
エントリ25）。当初疑っていた「develop側の回帰」は誤りで、真因はケース3
自身の設計（`shadow_on: None`バイパスで「beliefが変化しなくても毎回
強制actuateする」ことが「単発SendInputで『@』を誘発するのに十分」と
いう機序の十分条件を毎回満たしてしまう）だった。この時点では強制
actuateブロックを生キーの抑止ごと全面撤去したが、**この全面撤去
自体が別の「@」退行を招いたことが追記4で判明し、「抑止はする・
actuateはしない」という形に再設計している**——現在の実装は追記4を
参照すること（`crates/awase-windows/src/runtime/key_pipeline.rs`の
`kp_stage_shadow_ime_toggle`/`explicit_ime_action_target`、回帰ガードは
`architecture_guard.rs`の`kp_stage_shadow_ime_toggle_never_
reintroduces_case3_forced_actuate`）。Ctrl+無変換の症状は、ケース3とは
無関係な既存の`keys.ime_off`ホットキー処理に元からある独立した低頻度
バグと判明し、`docs/known-bugs.md` BUG-121として別途記録した。

**実機検証結果（2026-09-08、dragonflyg4、Windows Terminal + GJI）**:

- **ケース2（belief OFF→ON昇格、`muhenkan_solo_tap_ime_action = "on"`）:
  確定的に修正を確認**。半角状態で無変換単独タップ→ひらがなモードへ
  切り替わり、「@」は一切再現しなくなった（デバッグログでも
  `[shadow-toggle] 明示config: vk=0x1D OFF→ON へ昇格`の発火と
  `physical="Suppress"`（Down/Up双方）を確認）。IME ON中の無変換単独
  タップ（M13維持、GJI自身のかな切替に委譲）も正常動作を確認。
- **ケース3（"off"×belief既にOFF、`muhenkan_solo_tap_ime_action = "off"`）:
  実機で「@」が再現し続けることを確認、未解決。** デバッグログでは
  生キーのSuppress（Down/Up双方）と代替`VK_IME_OFF`のSendInputが
  いずれも設計どおり同期的に発火していることを確認済み——つまり
  「抑止漏れ」でも「actuationのタイミング遅延」でもない。当初の
  仮説（「belief既にOFFの状態へ冗長にVK_IME_OFFを送ること自体が
  引き金」）は、切り分けのため実行した**別テスト**（本ADRのコードとは
  無関係な既存機能`Ctrl+無変換`〈IntentKind::SyncKey経由、belief既に
  OFFなら本来no-opでVK_IME_OFF送信自体が起きないはずの経路〉を半角状態
  で押したところ**そちらでも「@」が再現した**ことで揺らいでいる——
  ユーザーからは「以前はCtrl+無変換では出なかった」との指摘もあり、
  ADR-153のコード変更とは独立した根本原因（develop側の回帰の可能性を
  含む）が関与している疑いが強い。**この「@」の機序の再調査は
  ADR-153のスコープを超えるため、次セッションへ持ち越す。**
- 実機確認は`muhenkan_solo_tap_ime_action`（無変換）のみで実施。
  `henkan_solo_tap_ime_action`（変換）・`"toggle"`方向・ATOKプリセット
  併用は未確認のまま。

以下は決定1着手前（設計収束フェーズ）の記録:

**設計収束・実装フェーズへ移行可能（opus-adversarial-consult r1〜r9で
収束を確認。r9: 「設計としては収束したと判断する」——B1〜B14はすべて
実装レベルで解消を確認済み、残るM25（マーカーの搬送は`InputContext`
拡張ではなく`ImeRelevance`の新フィールドで行う——ADR-149案Aが既に
棄却した道を踏まないための実装機構の指定であり設計判断ではない）も
反映済み）。**

**実装着手前の残る前提条件**（r9総評より）: (1) ✅解消（2026-09-08、
未決着#1参照）——B4（mode1実験時の抑止が`transport.rs::plan`経由で
実効だったかのコード確認）はコード上で確認済み、(2) ✅解消（2026-09-08）
——ADR-149「案C」続報として[ADR-154](154-delegate-shadow-toggle-exclusivity-off-to-on-transition.md)
を起票済み（提案中・未実装、実装自体は本ADRのスコープ外のまま）、
(3) `plan_tests`・`crates/awase-windows/tests/`への回帰テスト追加
（`fix-requires-evidence.md`の2つの再発ファミリーに該当）——これは
決定1の実装自体に含めて満たす。これらは設計ではなく実装ゲートであり、
本ADRの決定自体はこの3点の完了を待たずに確定してよい。

**経緯（r1〜r7の要約）**: (1) GJIキーマップ自動検出→生キー抑止→
静的VK置換という当初案はADR-110撤回・force-ON誤発火・チョード破壊
（B1〜B3）でr1にて撤回。(2) `Engine::apply_ime_open_request`にGJI
自動検出と並ぶ供給元を足す案は、belief OFF状態で配線が到達不能
（B5）・供給元調停機構の欠如（B6）をr2で指摘され、その対処が「belief
不変時は抑止だけしてactuateしない」という新たな穴（B7、最も標準的な
「無変換=IME OFF」×「半角」ケースが該当）をr3で生んだ。(3) 「常に
actuateしその引き換えに抑止する」という単一原則への転換をr4提案通り
採用したところ、r5で配線先の選択ミスにより自動検出未成功時に到達不能
（B9、B5の再発）と、チョード判定を経ずKeyDown時点で常にactuateして
しまう設計ミス（B10、B3の再発）が発覚した。(4) r6でこれを「belief
ON中／belief OFFから遷移する場合／`"off"`×belief既にOFF」の3ケースに
分割し、最後のケースのチョード救済サブ機構は「belief OFF中はエンジン
非活性でチョード概念自体が不成立」と判明し不要になった（B12、r6）。
(5) r7で、ケース2とケース1が同一打鍵を二重評価する穴（B13）が発覚し、
ワンショット消費で対策した。ADR-151/152と隣接する設計領域だが、狙いは
異なる——両者との違いは「背景」節末尾で明示する。

2026-09-08、実機3段階の検証（生キーレベル/コンソールAPIレベル/GJIプリ
セット依存性）で、「@」の機序が GJI の TSF キー横取り（`ITfKeyEventSink`、
GJI 自身が無変換/変換に何らかのコマンドを割り当てている場合のみ発火）
であることを確定させた（詳細は「実機A/B結果」節）。

## 背景

### 動機（ユーザー提案、2026-09-07）

BUG-113 の残置症状（半角/直接入力状態で無変換キーを単独タップすると
Windows Terminal + GJI で「@」が出力される。`docs/known-bugs.md` BUG-113
2026-09-07 追記で、**awase.exe を完全停止した状態でも毎回再現する**ことを
独立 `WH_KEYBOARD_LL` ロガーで実機確認済み——GJI 自身が無変換キーの物理
押下をネイティブに処理した結果である可能性が高いと記録されている）を
起点に、次の設計方針が提案された:

> awase のレイヤーで、無変換やかなキーを抑止して、代わりに VK_IME_ON/OFF
> とか VK_DBE_* にして送出するというのはどうだろうか。せっかくアプリ
> ケーションごとのキーボードリマップ機能も実装したし。GJI の設定を読んだ
> うえで必要な無変換や変換と同じ働きをするキーに置き換えて送出する。
> なければできる限り近い働きをするキーに置き換える。ムリする必要はない。
> 80 ニーズだけ救えれば。

要点は3つ:

1. **常に抑止する**（GJI に生の VK_NONCONVERT/VK_CONVERT/VK_KANA を渡す
   経路そのものを断つ）。
2. **GJI のキーマップ設定を読み、同じ意味を持つ安全な VK に置き換える**
   （`awase-gji-config` クレートが既に custom_keymap_table を読める、
   ADR-092 決定D/BUG-115/118/119 の資産を流用する）。
3. **全ケースの解決を狙わない**——安全に置き換えられないケースは無理せず
   現状維持（生キー通過）にフォールバックし、大半（ユーザー表現「80
   ニーズ」）をカバーできればよい、と明示的にスコープを絞っている。

実装手段として ADR-110（物理キー単純リマップ、`[[keymap]]` 相当の
per-app 設定基盤）を流用する案が併せて提示されている。**（注記、
opus-adversarial-consult r1・m13）この ADR-110 流用案は撤回済み
（B2）——ADR-110 自体が既に revert されており、代わりにエンジン内部の
既存機構（`Engine::apply_ime_open_request`）へ供給元を足す決定1に
差し替えた。**

### なぜ現状は「生キーが漏れる」経路になっているか

`crates/awase-windows/src/runtime/key_pipeline.rs::kp_stage_shadow_ime_
toggle` が無変換/変換/ひらがな/カタカナ/半角全角の物理押下を検出した際、
ADR-092 決定D→ADR-135→ADR-141→ADR-147 の系譜が確立した
`delegate_to_open_axis`（GJI 自身のキーマップ検出結果が「単純な
IMEOn/IMEOff相当」であれば、awase は actuate せず belief 追随のみ行い、
**物理キーはそのまま OS へ配送する**）が、この BUG-113 残置症状の
必要条件になっている。生キーがそのまま TSF/GJI に渡り、GJI 側の処理
（クローズドソース、機構未確定）が Windows Terminal の TSF コンテキスト
で何らかの理由で当該キーを完全に消費しきれず、結果が「@」として
Windows Terminal 側に漏れる、というのが現時点の実機切り分けの到達点
である。

`delegate_to_open_axis` がこの「生キー配送」を選んでいる理由は BUG-119
（GJI 側で「常に送出する（パススルー）」を明示選択したユーザーの意図を
awase が握りつぶしていた）であり、これ自体は正当な既存の修正である。
**したがって本 ADR は「delegate 判定そのもの」ではなく、「delegate が
生キー配送を選んだ場合に、本当に生の VK を送る必要があるか」を再検討する
——GJI 側が期待しているのは『無変換キーが押されたという事実』ではなく
『GJI の設定表が無変換キーに割り当てているコマンドの実行』のはずであり、
後者さえ再現できるなら、伝達手段（VK）を差し替えても GJI 視点の意味は
変わらないはずだ、という仮説に立つ。**（注記、opus-adversarial-consult
r1・m13）現決定1は「同一チャネルで VK を差し替える」設計ではなく
「生キーを抑止し、別チャネル（`Engine::apply_ime_open_request`の
Effect経路）でopen軸を直接actuateする」設計に変わっている——この
仮説の文言は当初の思考過程の記録として残すが、現決定1の実装とは
対応しない。**

### ADR-151/152 との違い（混同を避けるため明記）

[ADR-151](151-actuation-delegate-by-default-drift-scoped-to-ownership.md)
は「delegate 対象キーは awase が一切 actuate しない（=何も送らない、
belief 追随のみ）」という方向で、[ADR-149](149-physical-ime-key-activation-defers-forced-set-open.md)
の「案D」として opus-adversarial-consult r3 に検証させたところ、
**「Applied を詐称すると TsfNative 唯一の ON 方向救済機構
`apply_force_on_for_imm_broken` が構造的に永久停止する」という Blocker**
が見つかり保留になっている（両 ADR とも「将来構想として保留」）。

本 ADR は方向性が逆であり、この Blocker がそのまま当てはまるとは考えて
いない:

- ADR-151/152: 「何も送らない」→ 実際に状態が変わったかの確認手段が
  無くなる → force-ON 救済が誤って「もう確認済み」と扱われ止まる。
- 本 ADR: **「（生キーではなく）別の安全な VK を送る」→ 実際に
  `SendInput` を発行し `Actuation`/`FeedbackPolicy`（ADR-080）による
  通常の結果確認フローに乗る**——「送らない」のではなく「送るものを
  変える」だけなので、ADR-151/152 が踏んだ Blocker（詐称による確認
  手段の消失）が同じ形では発生しないはずである。ただし本 ADR も
  「delegate 対象キーへの介入」という同じ地雷原に踏み込む以上、
  この差異が実装レベルで本当に成立するかは opus-adversarial-consult
  で必ず検証する（「未決着」節参照）。

## 決定

### opus-adversarial-consult r1 が旧決定1〜4に見つけた Blocker（撤回理由）

旧決定3（ADR-110 の per-app キーボードリマップ基盤に静的テーブルとして
実装する）を起点に、Blocker が3件見つかった。

- **B1**: 静的リマップは `ImeController::apply`/`run_open_chain_async`
  という正規の actuation 合流点を通らないため `AppliedImeState` が
  更新されない。belief だけが先に更新され実状態の記帳が追いつかない
  窓ができ、`apply_force_on_for_imm_broken`（TsfNative 唯一の ON 方向
  救済機構）が誤って再発火する——ADR-149 案D の Blocker（`Applied` を
  詐称して同じ機構を永久停止させる）と**同じ根**（送信と記帳の分離）を
  逆方向から踏む。
- **B2**: 前提にしていた ADR-110 は 2026-08-30 に**撤回・revert 済み**
  （PR #123、JIS キーボードのフックベースリマップが構造的に危険と判明
  したため）。しかも現存する `[[keymap]]`（ADR-114 決定5）は、旧決定3が
  作りたいルールの両側——変換/無変換（既定の親指キー）を `from` にする
  ことと、`VK_IME_ON`/`VK_IME_OFF`（IME 制御系 VK）を `to` にすること
  ——を明示的に禁止しており（`keymap.rs::forbidden_target_vk_reason`）、
  無言で skip されて動かない。
- **B3**: 単独タップかチョードかは `simultaneous_threshold_ms`（既定
  100ms）満了後にしか確定しない（`resolve_pending_thumb_as_single`）。
  KeyDown 時点で確定する静的リマップは、この判定より前に介入してしまい
  NICOLA 同時打鍵チョードを構造的に壊す。

**結論（ユーザー指摘、2026-09-08）**: そもそも「物理キーリマップ」は
必要ない。欲しいのは「無変換/変換を親指キーとして使い続けながら、単独
タップが確定した後の IME ON/OFF/Toggle を、GJI 側のキーマップ設定に
頼らず awase 自身の明示設定で直接指定できるようにする」ことである。
これは以下の決定1本で実現でき、旧決定1〜4はすべて不要になる。

### 決定1: 無変換/変換単独タップに、GJI/MS-IME自動検出と並行する明示configを追加する（3ケース分割、opus-adversarial-consult r5指摘を受けた設計）

既存の `GeneralConfig::muhenkan_solo_tap_dedicated_fn_key`
（ADR-091 §D3.2、専用 Fn キーへ中継する隠し設定）の**きょうだい設定**
として、無変換・変換それぞれに次の設定を追加する:

```rust
/// 無変換単独タップ確定時に、素の VK_NONCONVERT の代わりに awase 自身が
/// IME を直接制御する（隠し設定、上級者向け）。`None`（既定）なら無効。
pub muhenkan_solo_tap_ime_action: Option<ShadowImeActionConfig>,
/// 変換キー版。既定 `None`。
pub henkan_solo_tap_ime_action: Option<ShadowImeActionConfig>,
```

`ShadowImeActionConfig` は `"on"`/`"off"`/`"toggle"` の3値（TOML上は
文字列、`awase::types::ShadowImeAction`という**プラットフォーム
非依存コア型**への変換は config 側の薄い層に置く——`ADR-019`の層境界
を守るため、core 型に serde を直接付けない。既存の
`deserialize_keymap_to`（`src/config.rs:613`）と同じ様式）。

**設計原則（opus-adversarial-consult r4 が提示、r5 で「常に KeyDown で
actuate する」と誤って一般化してしまい B9/B10 が発覚、r5 の指摘に
基づき正しい適用範囲へ訂正）**: 「抑止と actuation を1対1に対応させる
（抑止するなら必ず送る、送らないなら抑止しない）」という原則自体は
維持するが、**同時打鍵チョード判定（100ms 猶予）は常に維持しなければ
ならない**——KeyDown 時点で常に actuate してよいわけではない。この
2つの制約を両立させるため、状態に応じて**3ケースに分割**する。

**ケース1: belief ON（エンジン活性中）** — 物理 KeyDown は既に
`Decision::Consume`（`runtime/executor.rs:460`）で抑止済み。
`resolve_pending_thumb_as_single`（単独タップ確定処理、100ms 猶予後、
`self.adapter.take_ime_open_requested()`→`apply_ime_open_request`→
`ime_set_open_effects`という`keys.ime_on`/`keys.ime_off`コンボキーと
同じEffect経路、ADR-092/135/141/147が既に稼働させている機構）が読む
`ThumbSoloSpecialHandling`（`nicola_fsm.rs:887-917`）に、**新しい
専用フィールド**（例: `explicit_ime_action: Option<ShadowImeAction>`）
を追加する。**（r7・M21訂正）既存の`delegate_to_open_axis`フィールド
を共用しない**——M15が「明示config設定時はdelegateをarmedにしない」
としているため、同じフィールドへ書き込みながらarmedにしない、という
矛盾した実装は成立しない。専用フィールドとその専用セッターを設ける。

優先順位表（`nicola_fsm.rs:1997-2000`の既存表に新しい行を追加）:

```
1. 専用Fnキー（muhenkan_solo_tap_dedicated_fn_key、ADR-091 §D3.2）
2. ★明示config（*_solo_tap_ime_action、本ADR）   ← 新設
3. IME open軸への肩代わり（*_delegate_to_open_axis、ADR-092決定D Step4b）
4. ModeKeyConfigベースのSuppress/Passthrough
```

既存表のコメントは「1・2はいずれも`ModeKeyConfig`の外側で独立に判定
する——config reloadで`ModeKeyConfig`が丸ごと再設定されても、自動
検出由来のこれらの値が消去されないため」と設計意図を記録している。
新設する2の行も同じ設計意図（明示configはconfig reload時に
config.tomlから再読込されるため、reloadで消去されて構わない）を
併記する。**なお`thumb_solo_special_handling`は変換キー（henkan）に
ついて`dedicated_fn_key: None`をハードコードしている
（`nicola_fsm.rs:897`）——r2のm7の結論（専用Fnキーが自動的に優先）は
無変換にしか当てはまらず、変換キー側では本ADRの明示configが実質最上位
になる。** エンジンは既に活性なので B5 は発生せず、100ms 猶予も維持
される。

**ケース2: belief OFF かつ明示configの方向が遷移を要求する
（`"on"`／`"toggle"` かつ現在 OFF）** — `kp_stage_shadow_ime_toggle`
の belief 昇格処理（`:1150-1175` 付近、GJI/MS-IME自動検出由来の
`shadow_action` を見て belief を書き込む処理）に、**自動検出の成否に
関わらず**明示config自体を`shadow_action`相当の入力として直接扱う
分岐を、**BUG-14の注入イベント早期return（`:1126-1137`）より後、かつ**
`intent_kind`が`None`になる早期return（`:1165-1167`、B9が指摘した
到達不能の原因）**より前**に追加する（r7・m31——注入された
`VK_NONCONVERT`にactuate+抑止すると、ADR-119が塞いだ「二重の空振り」
を作り直す）。これにより自動検出が何も検出していない構成（本ADRが
実機A/Bで確認したATOKプリセット等）でも、明示configだけで belief を
OFF→ON に昇格できる（B9対策）。**（r6・M18訂正）この belief 昇格は
「belief を動かすだけ」ではない**——`Engine::check_active_transition`
がInactive→Activeを検知し`Effect::Ime(SetOpen{origin:ActivationSync})`
を発行するため、この同一イベント内で実際に`VK_IME_ON`がSendInputされる
（ADR-149の実機ログが送信1として記録した経路）。この実actuationの後、
同じ打鍵は`PendingThumb`として consume され、100ms後に`resolve_
pending_thumb_as_single`が単独タップかチョードかを通常どおり判定する
（B10対策）。

**B13（opus-adversarial-consult r7指摘）: ケース2とケース1の二重評価
——`resolve_pending_thumb_as_single`の消費点2（ADR-149「案C」の用語）
が、ケース2が書き換えた後のbeliefを読んで二重に評価してしまう。**
100ms後にケース1（決定1の新フィールド`explicit_ime_action`）が
発火すると、`apply_ime_open_request`は`ctx.ime_on`（ケース2が既に
ONへ書き換えた後の値）を見て方向を決める——`"on"`なら`AlreadyMatched`
だが随伴warmupが余分に飛び、**`"toggle"`は`!true=false`となり
`VK_IME_OFF`を送ってしまい、単独タップ1回でON→OFFへ往復して
ユーザーからは何も起きなかったように見える**（ADR-149が「案C」として
記述した排他性の穴そのもの、BUG-115のopt-inゲートが非冪等な`Toggle`
を守っていた領域に本ADRが正面から踏み込むために顕在化する）。

**B13対処**: ケース2がbeliefを昇格させactuationを発行した打鍵には、
「この打鍵のexplicit_ime_actionは消費済み」マーカーを立て、ケース1
（`resolve_pending_thumb_as_single`）はこのマーカーが立っている場合は
`explicit_ime_action`を読まずスキップする（＝ケース2が既に全て処理
済みとして扱う）。これにより`"on"`/`"toggle"`とも送信は1回・結果も
1回で確定し、案Cの排他性の穴を踏まない。

**B14（opus-adversarial-consult r8指摘）: マーカーを独立したワンショット
チャネル（`take_ime_open_requested()`と同型）にすると、消し忘れで
恒久的な無反応を作る。** その打鍵がチョードとして解決された場合、
`resolve_pending_thumb_as_single`自体が呼ばれないため、マーカーを
消費する機会が無く残留する。残留したマーカーは、その後の**別の**
単独タップ（belief ON、ケース1）まで無条件にスキップさせてしまい、
delegateもM15で無効化済みのため、IME OFFにしたい単独タップが恒久的に
無反応になる。これは`Engine::discard_ime_open_request`
（`engine.rs:608-619`）のdocが「ワンショットチャネルは取り出されない
まま残留し無関係な次のイベントで誤発火する経路を、呼び出し元の全アーム
で塞ぐ必要がある」と既に警告している落とし穴の再発である。

**B14対処**: マーカーを独立したワンショットチャネルにせず、**保留中の
`PendingThumb`状態自身のライフタイムに結び付ける**（`PendingThumb`が
単独タップ・チョード成立・`flush(ContextChange::ImeOff)`・
`handle_focus_changed`・`on_command`（`ToggleEngine`/`SwapLayout`）・
config reloadのいずれで消えても、マーカーも一緒に消える）。これにより
「取り出し漏れの経路を全部塞ぐ」という個別対応が不要になり、構造的に
残留し得ない。前提条件（M8）は「対策（ワンショット消費、`PendingThumb`
ライフタイムに結合）を入れない限り構造的に必ず発生する」と修正する。

**マーカーの搬送経路（M25対策、opus-adversarial-consult r9指摘）**:
マーカーを立てたい時点（`kp_stage_shadow_ime_toggle`、ケース2）では
まだ`PendingThumb`が存在しない（`PendingThumb`は直後の
`engine.on_input`内でNicolaFsmが作る）ため、マーカーを`NicolaFsm`まで
運ぶ経路が要る。**`InputContext`へのフィールド追加はADR-149の案Aが
既に明示的に棄却済み**（`OS由来の瞬間値のみ`という明文規約に抵触・
34箇所の構築点への波及・引数個数のclippy閾値抵触、`docs/adr/149-
*.md:270-275`）のため採らない。代わりに**`RawKeyEvent.ime_relevance`
（`awase::types::ImeRelevance`）に新フィールドを追加する**——この
構造体は`shadow_action`/`sync_direction`等、まさに「プラットフォームが
分類した、この1打鍵限りのメタデータ」を engine へ渡すための既存の器
であり、`Runtime::enrich_ime_relevance`が populate する。イベントと
寿命が同じなので独立チャネルの残留問題自体が起こらず、ADR-019の層
境界（プラットフォームが分類し、engineは事前分類済みイベントだけを
受ける）にも沿う。`NicolaFsm::on_input`が`PendingThumb`を作る際に
この値を一緒に格納すれば、B14が要求した「`PendingThumb`と一緒に消える」
も自動的に満たされる。

**ケース3: `"off"` かつ belief 既に OFF（B7の穴、KeyDown時点で判断
せざるを得ない唯一のケース）** — ケース2の昇格が起きない（OFFのまま
OFFを要求するので belief に変化が無い）ため、上記2ケースのいずれにも
乗らない。**（r6・B12で単純化）当初案（チョード成立時の遅延再注入等の
救済サブ機構）は不要と判明した**: belief が既に OFF ということは
`compute_state`（`engine.rs:289-291`）が`Inactive(ImeOff)`を返す、
すなわちエンジンが構造的に非活性であるということであり、
`NicolaFsm::on_input`（`PendingThumb`を作る側）自体に到達しない。
**同時打鍵チョードという概念がこの状態では成立しない**——ケース3の
状況で無変換+他キーが押されても、それは NICOLA のチョードではなく
ただの2つの独立したキーである。したがって「チョードだった場合の救済」
という事態そのものが発生し得ない。

ケース3は次の単純な形になる: KeyDown 時点で
`apply_ime_open_with_belief(order, None, belief)`（`shadow_on: None`
バイパス、`kp_stage_idle_conv_check`のDirectInput回復＝
`key_pipeline.rs:1046-1063`と同じパターン）を発行し、**その actuation
と引き換えに**`transport.rs::plan:333-340`の無条件`Allow`へ「明示
config対象キーかつこのケースに該当する場合はSuppressする」という
限定例外を置く。抑止とactuationが1対1で対応するため、B7/B8（抑止
するがactuate しない空振り）が構造的に発生せず、TsfNativeでbelief
がドリフトしている場合（belief OFF×実IME ON）でも確実に`VK_IME_OFF`
が飛ぶ——ユーザーの手動回復手段が保たれる。新規の「保留してから解決
する」サブ機構は不要。

**delegate/shadow_overrideの無効化（M15対策、r6・B11/M20で訂正:
両方を外す必要がある）**: ケース1で明示configが設定されているキーに
ついては、GJI/MS-IME自動検出由来の`delegate_to_open_axis`**および
`shadow_override`の両方**をarmedにしない。**delegateだけを外すと
`shadow_action`は`shadow_override`経由で`Some`のまま残り、
`delegate_owned`が`false`になった結果`kp_stage_shadow_ime_toggle`が
KeyDown時点でそのまま actuation まで進んでしまい、B10（チョード破壊）
が別形で再発する**（`delegate_owns_mode_key_shadow_toggle`の
`muhenkan_dedicated_fn_key_configured`引数——`gji_charset_autodetect.
rs:481,494-496`——が同型の高優先度設定について同じ配慮を行った前例）。
両方を外せば、belief ON時は`shadow_action=None`により`:1165`で早期
returnしKeyDownでは何もせず（100ms後にケース1が処理）、belief OFF時は
ケース2の新分岐（自動検出に依存しない）が引き続き機能する。
書き込み点は**2系統4箇所**——GJI側（`gji_charset_autodetect.rs:
759,769`）とMS-IME側（`message_handlers.rs:845-847`と直後のshadow
override相当）——であり、両系統に同じ無効化を適用する（ADR-119の
教訓「gateを1箇所に置いて満足しない」）。

**`mode_key_config = Passthrough`（BUG-119）との関係（M13対策）**:
上記いずれのケースも、`mode_key_config`が`Passthrough`（ユーザーが
「このキーは常に OS へ送出する」と明示済み）の場合は発火させない——
明示設定どうしが競合したら、後から追加した本ADRの設定ではなく、
先にあった明示パススルー設定を勝たせる。

**非親指キー構成への対応（M7対策、r5・M17で訂正）**: 到達性を決める
のは`kp_stage_shadow_ime_toggle`が呼ばれるかどうかではなく
`shadow_action`（`intent_kind`）の成立であり、その供給元は
`route_thumb_key_action`が親指/非親指で振り分けている（親指→
delegate、非親指→`ime_*_auto`）。ケース2の対策（自動検出の成否に
関わらず明示configを直接扱う）を入れれば、親指キー構成・非親指キー
構成のどちらでも自動検出の成否に依存しなくなる。

**ケース分割の網羅性（r6・m27）**: (belief, 明示configの方向) の
組み合わせは (ON, on/off/toggle のいずれか)＝ケース1、(OFF, on/toggle)
＝ケース2、(OFF, off)＝ケース3 の3パターンに完全に分かれ、
取りこぼしは無い（beliefは真偽2値、明示configの方向は3値だが
"belief=ONなら方向によらずケース1"としているため、実質2×3=6通りが
過不足なく3ケースへ写像される）。

**`plan`への入力追加（M19対策）**: ケース3の判定条件（明示config
が`"off"`かつbeliefが既にOFF）を`transport.rs::plan`（純粋関数）へ
渡すには新しい入力が要る。`DbeModeKeyContext`（`transport.rs:48-71`、
BUG-116決定1で同じ理由により追加された文脈構造体）と同型の追加を行う
——ただし渡すべき情報は「ケース3としてawaseが既にactuate済みか」の
1ビットに縮約できる（`shadow_toggled`と同じ戻り値経路で渡せる、r5・
m25参照）。`fix-requires-evidence.md`が`plan`について警告する
「VK種別だけで場合分けせず…変更時は関連するVK全種類（0xF0〜0xF6）
への影響を洗い出すこと」に従い、`plan_tests`への回帰テスト追加を
実装の必須条件とする。

**この設計が Blocker を生まない理由**:

- **B1 を踏まない**: いずれのケースも`ImeController::apply`を経由する
  既存のactuation実行系を使う（ケース1は`ime_set_open_effects`、
  ケース2は belief昇格経由でケース1に合流、ケース3は
  `apply_ime_open_with_belief`で直接actuateする）。新しい raw write
  site は増えない。
- **B2 を踏まない**: ADR-110/`[[keymap]]`には一切触れない。
- **B3を踏まない**: ケース1・2とも`resolve_pending_thumb_as_single`
  （100ms猶予）を経由してから実際のactuationに至る。ケース3は
  KeyDown時点でactuate・抑止するが、**belief OFFの間はエンジンが
  構造的に非活性でNicolaFsmに到達しないためチョードという概念自体が
  存在しない**——B3が防ごうとしている「チョード判定を壊す」という
  事態がケース3の状況では原理的に起こり得ない。
- **B5を踏まない**: ケース2が自動検出の成否に依存せず明示config単体
  でbelief昇格できるため、「belief OFFの間は到達できない」という穴が
  無い。
- **B7/B8を踏まない**: ケース3は抑止とactuationが常に1対1で対応する
  （actuateしたことをもって抑止する）ため、「抑止するがIME切り替えの
  意図も無い」という空振りが構造的に発生しない。TsfNativeでbeliefが
  ドリフトしている場合も確実に`VK_IME_OFF`が飛ぶため、ユーザーの
  手動回復手段も保たれる。
- **`Toggle`は案bに一本化**: `ShadowImeAction::Toggle => !ctx.ime_on`
  と同じ考え方（`current`は`effective_open()`）で計算する。
  opus-adversarial-consult r1（M3）の「案a（`VK_KANJI`）はbeliefの
  ズレを温存する、案bの方がbeliefと実状態を収束させる」という分析を
  踏まえた選択——`VK_KANJI`・専用送信口`post_kanji_toggle_to_focused`
  （Ctrl/Shift/Altの一時解除・復元を伴い、親指キーのチョード中に使うと
  BUG-78系のstuck modifierを誘発しうる）は不要。ただし`Toggle`は
  belief依存のままであり（TsfNativeは`FeedbackPolicy::Blind`で読み
  戻せずbeliefがズレていれば逆方向へ切り替わりうる）、この性質を
  設定のdocコメントに明記する（`gji_thumb_key_ime_toggle`のdocと
  同じ様式）。

**前提条件（M8、r7で「見込み」から確定事実へ訂正）**: ケース1・2の
経路は、ADR-149が「案C」として記述した「delegateとshadow-toggleの
二重評価」連鎖と**構造的に同型であり、B13対策（ワンショット消費）を
入れない限り必ず発生する**（M15のdelegate無効化はGJI/MS-IME自動検出
由来のdelegateとの二重評価を防ぐが、ケース2とケース1という本ADRが
新設した2つの消費点どうしの二重評価までは防がない）。ADR-149は関連
事象を`docs/known-bugs.md`に記録済みだが続報ADR（「案C」）は未起票の
まま。本ADRの決定1はB13対策の実装をもってこの領域に自ら対処するが、
続報ADRの起票・解消（または実装時の再検証で本ADRの対策で十分と
判明すること）を実装前提として明記する。

**`mode_key_config`のケース2・3への露出（M22対策）**: M13対策
（`mode_key_config = Passthrough`の場合は発火させない）は、ケース1
（`resolve_pending_thumb_as_single`内、既存の`special.mode_key_config`
がそのまま使える）にはそのまま適用できるが、**ケース2・3が動く
`kp_stage_shadow_ime_toggle`（`awase-windows`側）からは`NicolaFsm`
内部状態である`mode_key_config`が見えない**。`Engine`に既存の
`muhenkan_delegate_to_open_axis()`と同型のgetterを新設し、`Runtime`
側から参照できるようにする——これを怠ると「ケース1だけBUG-119の
パススルー尊重が効き、ケース2・3では素通りする」という非対称が生まれる。

**ケース1がマーカーによりスキップした場合のフォールスルー
（M23対策）**: B14対策後、ケース1が`explicit_ime_action`をスキップ
すると、`resolve_pending_thumb_as_single`は優先順位表を下へ流れ、
M15でdelegate（優先順位3）が無効化されているため必ず優先順位4
（`ModeKeyConfig`）に到達する。既定の`Suppress`なら「何もしない」で
正しい（ケース2が既にactuate済み）が、ユーザーが`Passthrough`を
選んでいた場合は生キーが再注入される——ただしM13により
`mode_key_config = Passthrough`のキーはそもそもケース2・3自体を
発火させない設計なので理屈上は起きない。**`ModeKeyConfig`は
composing状態で`for_composing(composing)`により値が変わるが、M13の
判定は`for_composing`適用前の設定値そのものを見る**（r9・m38で決着）
——M13の趣旨は「ユーザーが『常に OS へ送出する』と明示したなら本ADR
は介入しない」であり、composing中かどうかで介入可否が切り替わるのは
この趣旨に反するため。

**専用Fnキーとの併用の決着（M24対策、未決着#10から格上げ）**:
ケース2の発火条件に「対象キーに`*_solo_tap_dedicated_fn_key`が
設定されていないこと」を追加する（`thumb_solo_special_handling`の
既存フィールドがそのまま判定材料になる）。これにより優先順位1
（専用Fnキー）と優先順位2（明示config）が同一打鍵で同時に発火する
「IMEがONになり、かつFnキーも送られる」という複合動作を防ぐ。

### 決定2: GJI キーマップ自動検出（既存の `delegate_to_open_axis`）はフォールバックとして維持する

決定1の新設定が `None`（既定）のユーザーには、既存の
`gji_charset_autodetect.rs` ベースの自動検出・delegate 機構をそのまま
使い続ける（ADR-092/135/141/147/BUG-115/118/119 の到達点を変更しない）。
BUG-119 の「常に送出する（パススルー）」尊重も無変更。**decision1は
add-onであり、既存の自動検出パスを置き換えない。**

### 決定3: 実 MS-IME のレジストリキー割当ても同じ供給元に統合できるが、本 ADR のスコープからは外す

opus-adversarial-consult r1（M4）の指摘により、旧決定4は「新しい方針
転換」ではなく「ADR-091 決定4（ステータス: 決定・実装未着手）の未着手分
そのもの」であることが判明した。`msime_key_assignment.rs::
MsImeDelegateToOpenAxisAssignment` は既に `ShadowImeAction` への写像を
実装済みで、欠けているのは決定1のケース1（`resolve_pending_thumb_as_
single`への供給元追加）と同じ配線をMS-IME側にも行うことだけである。
**（r5・m24訂正）決定1がr5でEngine内部を経由しない設計に一度変わり、
r6で再びケース1がEngine側（`resolve_pending_thumb_as_single`）を経由
する設計に戻ったため、「決定1と実装点が同一」という主張はケース1に
限っては再び成立する。** ただしケース2・3（belief OFF側の扱い）は
GJI/MS-IME自動検出とは独立した新経路であり、MS-IME側にこれが必要かは
別途検討を要する。**（r6・m29訂正）ケース2・3の目的は「@」対策では
なく、belief OFF状態からの到達性確保（B5/B9対策）である**——
`compute_state`（`engine.rs:289-291`）のエンジン非活性化ロジックは
IME種別に依存しないため、MS-IMEアクティブ時でも同じ「belief OFFの
間はNicolaFsmに到達できない」という構造的制約自体は存在する。ただし
MS-IMEは`MsImeDirectStrategy`によりawaseが全軸を自前で確実に制御
できる前提（GJIのようなTSF消費不確実性が無い）なので、この到達性の
穴が実害（「@」漏れ等）につながるかは別問題であり未確認のまま。
本ADRではこれ以上深入りせず、決定1実装後に改めて要否を判定する。

## 実機A/B結果（2026-09-08、dragonflyg4、Windows Terminal + GJI）

opus-adversarial-consult より先に、決定1の核心仮説を実機A/Bで検証した
（診断スパイク、branch `diag/adr153-vk-substitution-spike`、
`config.general.diag_adr153_mode` で切替）。無変換キー単独タップを
半角（直接入力）状態で反復した。

- **mode 1（生キーを完全に抑止・代替は一切送らない）: 「@」が
  無変換を押すたびに毎回出た。** つまり「生の物理 VK_NONCONVERT が
  GJI/TSF に到達すること」自体は「@」の必要条件ではなかった——本 ADR
  の「背景」節が当初立てていた仮説（GJI が生キーを消費しきれず漏らして
  いる）は、この結果単体では反証される。
- **mode 2（`gji_charset_autodetect` の分類に応じて `VK_IME_ON`/
  `VK_IME_OFF` へ代替）: 「@」は出なかった。**

**当初の「暫定的な機序の再解釈」（drift 補正バースト説）は撤回する**
（旧稿はここに保存しない——実機ログでの裏付けを取る前に、より直接的な
反証実験で別の機序が確定したため）。

### 確定した機序（2026-09-08、実機A/Bで確認）

同日、独立した3段階の実機実験で機序を直接確定させた。

1. **生キー配送レベルの検証**: 独立 `WH_KEYBOARD_LL` ロガー
   （`scripts/rawkbd_logger.ps1`、awase 完全停止、抑止一切なし）で
   メモ帳にフォーカスして無変換キーをタップしたところ、`vk=0x1D
   scan=0x7B` が一貫して届き、他の VK への変換は一切無かった。同時に
   「@」もメモ帳には出なかった。
2. **コンソール API レベルの検証**: awase を完全停止した状態で
   Windows Terminal にフォーカスし、`[Console]::ReadKey($true)` で
   無変換キーの生データを直接読んだところ、`Key=29 (0x1D), KeyChar=
   0x00, Modifiers=0` ——正しく破損の無い値だった。**それにもかかわらず
   同じ awase 停止状態で、普通に PowerShell プロンプトに戻って無変換を
   タップすると「@」が出た。** 生キーデータが1・2の両レベルで完全に
   正しいまま「@」が発生する以上、**「@」は物理キーストローク自体の
   経路（KeyDown/KeyUp → コンソール入力バッファ）を通っていない**、
   別の経路からの副産物であることが確定した。
3. **GJI プリセット依存性の検証**: awase のビルド・config は一切変えず、
   GJI 側の `session_keymap` プリセットだけを ATOK → MSIME → ATOK と
   切り替えて同じ手順（Windows Terminal、半角状態、無変換単独タップ）
   を反復した。**ATOK（無変換に `IMEOn`→`CancelAndIMEOff` という
   状態依存の割当てがあり、本 ADR の分類器では `Toggle` と判定される）
   では「@」が確実に再現し、MSIME（`ms-ime.tsv` に無変換の行自体が
   無く、GJI は無変換に一切コマンドを割り当てていない）では「@」が
   一度も再現しなかった**。同じ awase ビルド・同じ物理操作で、GJI 側の
   設定だけが変数だった。

**結論**: 「@」は、TSF（Text Services Framework）の
`ITfKeyEventSink`——キーがアプリへ配送される前に「このキーは自分が
処理する」と横取りする権利をアクティブな IME に与える仕組み——を
経由した副産物である。**GJI が無変換/変換キーに何らかのコマンド
（`On`/`Off`/`Toggle` のいずれか）を割り当てているときだけ、GJI の
TSF キートレース処理がこのキーに「興味を持ち」、何らかの内部処理
（コンポジションの確定/挿入等、クローズドソースのため詳細不明）を
試みる。** その副産物が Windows Terminal 側で「@」という1文字の
挿入として観測される——生の物理キーストローク（KeyDown/KeyUp の
`KeyChar`）は無傷のまま、TSF の**別チャネル**から挿入される。GJI が
そのキーに何のコマンドも割り当てていなければ（MSIME プリセット）、
TSF キートレースはそもそも発火せず、この副産物も発生しない。

**この理解は決定1の対策の効果を裏付ける**: 生キー（`VK_NONCONVERT`/
`VK_CONVERT`）を awase が OS に一切届けさせなければ、GJI の TSF
キートレースが「無変換/変換キーが押された」というイベント自体を観測
する機会が構造的に無くなる——GJI 側の `session_keymap` 設定が何で
あろうと（ATOK でも、ユーザーが将来どんなカスタムキーマップを組もうと）、
このバグの引き金を引く余地が無くなる。（r5・m23訂正）ただし決定1の
3ケース分割が示すとおり、これは「belief ONで当該キーがPendingThumb
として consume されている場合」（ケース1）と「belief OFFから明示config
で昇格させ、同様にconsumeさせる場合」（ケース2）にのみ自然に成立する
——ケース3（`"off"`×belief既にOFF）だけは、consumeを経ずKeyDown時点で
直接actuate＋抑止する（エンジンが構造的に非活性のためチョードという
概念自体が存在せず、救済策は不要——r6・B12）。

### 動機の再定義（ユーザー指摘、2026-09-08）: 単発の「@」修正ではなく、GJI キーマップとの二重オーナー競合という問題クラス全体への対策

この機序が確定したことで、本 ADR の価値は BUG-113 の残置症状（「@」）
1件の修正にとどまらない。**無変換/変換を NICOLA 親指キーとして使い
たいユーザーが、同時に GJI 自身の設定（キーマップ・プリセット）で
無変換/変換に何らかの IME 制御コマンドを割り当てていると、その組み
合わせ自体が本 ADR で確認した TSF 横取り機構を発火させうる**——これは
ADR-091・BUG-115・BUG-118・BUG-119 が繰り返し扱ってきた「awase と
GJI 自身の設定が同じ物理キーの意味を取り合う」という**二重オーナー
問題**の一種であり、ユーザーからも「（GJI 側にキー割当てを）設定
しようとして、競合するからダメ、という事例が多かった」という実体験が
指摘されている。

decision1 が目指す最終形は、ADR-091 が当時「新設（今回の着想）」として
書いた次の方針を、GJI に対しても実現することである:

> 素通しせず、awase 自身が物理無変換単独打鍵を検知して抑制し、代わりに
> 決定1の open 軸機構（belief 駆動の `VK_IME_ON`/`VK_IME_OFF` 送信）で
> 同じ意図を安全に実現する。ユーザーの意図（無変換=IME ON/OFF）はそのまま
> 尊重しつつ、実現手段を［GJI の］ネイティブ処理から awase 自身の冪等
> 機構へ差し替える。

つまり、無変換/変換をユーザーが NICOLA 親指キーとして使い続けながら
（同時打鍵チョード判定はそのまま維持）、単独タップが解決した後の
IME 制御は**常に awase 自身が生キーを抑止して代替送信する**ことで、
GJI 側の `session_keymap`/カスタムキーマップの内容に一切依存しなくなる
——ユーザーが GJI 側で何を設定していても（あるいは将来設定を変えても）、
awase 側の制御と衝突しない。

**追記（同日、複数回試行で再現性確認）**: mode 2 を複数回反復しても
「@」は一度も出なかった（ユーザー確認）。単発ではなく再現性のある結果
として扱ってよい。

**追記（opus-adversarial-consult r1/r2 を経て）**: Toggle 方向は
案b（belief 依存で `VK_IME_ON`/`VK_IME_OFF` に解決、`ShadowImeAction::
Toggle => !ctx.ime_on`）に一本化した（決定1参照）。ATOK プリセット＋
決定1の新設定を組み合わせた実機再検証は、決定1の実装後（B5/B6 対策を
含む配線が入った版）に行う——現行スパイクの mode 3/4 は使わない
（旧設計の遺物のため）。

## 未決着・要レビュー論点（次の opus-adversarial-consult ラウンドで詰めること）

1. **B4（opus-adversarial-consult r1 指摘）: mode1/mode2 実験の解釈が
   誤っていた可能性**: 「実機A/B結果」節の mode1（生キー抑止のみ・代替
   無し→「@」出る）と mode2（抑止+代替→「@」出ない）の対照だけでは、
   「代替送信が効く」ことしか言えず「抑止が必須」は導けない
   （両モードとも抑止は入っており、差分は代替の有無だけのため）。
   「抑止なし＋代替送信」という4セル目が未実験。加えて、このセッション
   を通じて `[adr153-spike]` タグのログが**一度も確認できなかった**
   ため、mode1 の抑止（`transport.rs::plan` 経由）が実際に効いていたか
   自体、コード上の再確認が必要——決定1の実装時、この検証を先に行う
   こと。

   **✅ 解消（2026-09-08、コード確認）**: `diag/adr153-vk-substitution-spike`
   ブランチの実装（`crates/awase-windows/src/runtime/transport.rs`の
   `PhysicalKeyDisposition::plan`）を読むと、無変換/変換の無条件`Allow`
   分岐の直前に`if diag_adr153_mode() >= 1 { return Self::Suppress; }`
   が挿入されており、`diag_adr153_mode`は`Runtime::apply_config`
   （`crates/awase-windows/src/runtime/mod.rs`）が
   `transport::set_diag_adr153_mode(config.general.diag_adr153_mode)`
   として毎回config.tomlの値を`AtomicU8`へ反映する経路で供給されていた。
   すなわちmode1の抑止は実際に`plan`経由で発火しており、コード上の
   到達性に疑義は無い。ログタグが確認できなかった件は「抑止がplan
   経由で効いていたか」とは別問題（tracing出力側の見落としの可能性）
   であり、抑止自体の実効性を否定する材料ではない。4セル目（抑止なし+
   代替送信）の未実験は残るが、これは「代替送信だけで足りるか」という
   別の問い（決定1が採用する経路とは無関係——決定1は常に抑止する設計）
   のため、決定1の実装ゲートとしてはこれ以上のブロッカーではない。
2. **`ShadowImeActionConfig` の TOML 表現**: `"on"`/`"off"`/`"toggle"`
   の文字列か、既存の `engine_on_ime_key`/`engine_off_ime_key` に似た
   構造にするか。命名も含め実装時に確定する。
3. **`muhenkan_solo_tap_dedicated_fn_key`・GJI/MS-IME自動検出delegateとの
   共存**: r5設計（`kp_stage_shadow_ime_toggle`側の独立経路）は
   `resolve_pending_thumb_as_single`/`dedicated_fn_key`/GJI自動検出
   delegateとは別のコードパスになったため、同じキーに両方（例:
   `dedicated_fn_key`と新設定の両方、または新設定とGJI自動検出delegate
   の両方）が同時に成立しうる状態が生まれていないか確認が要る。
   `delegate_owned`ゲート（`mode_key_delegate_owns_shadow_toggle(vk)
   && effective_open()`）が真の間、`kp_stage_shadow_ime_toggle`は
   belief書き込みをスキップする既存分岐があり、新設定の分岐がこの
   スキップより前に来るのか後に来るのかで挙動が変わる——実装時に
   優先順位を明記する。
4. **`gji_thumb_key_ime_toggle`（BUG-115 の opt-in ゲート）との関係**:
   このゲートは GJI 自動検出が `Toggle` を検出した場合の話であり、
   決定1の新設定（ユーザーが明示的に `"toggle"` を選ぶ）はこのゲートの
   対象外（ユーザーが直接選んでいる以上、BUG-115 が懸念した「検出結果
   への非同意」は生じない）と考えているが、明記が要る。本ADRの新設定は
   GJI自動検出とは別経路（決定1参照）なので、このゲートとは無関係に
   実装できる見込み。
5. **実機再検証**: 決定1の実装後、belief OFF 状態（半角）・belief ON
   状態（全角）の両方から単独タップして「@」が再発しないことを確認する
   （新設計は両状態で同じ経路を通るはずだが、`delegate_owned`絡みの
   相互作用（#3）が無いことも合わせて確認する）。ATOK プリセット +
   `Toggle` でも同様に確認する。
6. **証拠義務（`fix-requires-evidence.md`）**: 決定1は「キー選択」と
   「IME belief」の2つの再発ファミリーに触れる。`kp_stage_shadow_ime_
   toggle`はプラットフォーム層（`crates/awase-windows/`）にあるため、
   `src/engine/tests.rs`ではなく`crates/awase-windows/tests/`配下
   （`architecture_guard.rs`等、Linuxで`cargo nextest run -p
   awase-windows`実行可）に新分岐の回帰テストを追加することを実装の
   必須条件とする（r2でsrc/engine/tests.rsと書いたのは配線先が変わる
   前の記述だったため訂正）。
7. **（r9完了）** opus-adversarial-consultはr9で「設計としては収束した」
   と判定した。M25（マーカーの搬送経路）を反映済み。実装時にM25の
   具体化（`ImeRelevance`への新フィールド追加）を`architecture_guard`
   等で固定するテストを追加すること。
8. **belief 誤答 grace 期間中の無反応**: `kp_stage_shadow_ime_toggle`
   の intent 昇格は `is_japanese_ime()` を要求するが、この belief は
   スリープ復帰/フォーカス変更直後の grace 期間中に一時的に `false`
   を誤答しうる既知の弱点を持つ（`key_pipeline.rs:1104-1110`）。
   ユーザーの明示設定もこの窓では無反応になる——新規リスクではなく
   既存の自動検出経路と共通の制約だが、隠し設定の doc に1行残す。
9. **設定名の再考**: `muhenkan_solo_tap_ime_action`の`solo_tap`は、
   実際には`kp_stage_shadow_ime_toggle`が同時打鍵チョード確定を待たず
   毎回の物理KeyDownで発火する経路のため、厳密には「単独タップ」という
   概念を経由しない。命名を再考する（#2のTOML表現の検討と合わせて
   実装時に確定）。
10. **（r8・M24で決定1本文へ格上げ・解決済み）** 専用Fnキーとの併用は
    ケース2の発火条件に「専用Fnキー未設定」を加えることで決着した。
11. **InputRelayプロファイルとの整合（r7・m32、確認のみ・対応不要）**:
    `InputRelay`では`transport.rs::plan`が最優先で`Allow`を返し
    `ImeController::apply`が`NotOwned`を返すため、ケース3は
    「actuationなし・抑止なし」で1対1原則を自然に満たす。issue #136
    再検討時のためにこの確認結果を記録として残す。

## 関連

BUG-113、BUG-115、BUG-118、BUG-119、
[ADR-091](091-idempotent-charset-axis-gji-recommended-msime-self-responsibility.md)
（§D3.2 の `muhenkan_solo_tap_dedicated_fn_key` が決定1の直接の前例、
決定4はADR-091決定4そのものの実装課題）、
`msime_key_assignment.rs`
（実 MS-IME レジストリ検出。`MsImeDelegateToOpenAxisAssignment`という
協調側資産は既にあり、決定1と同じ配線を足すだけで済む。決定3参照）、
[ADR-092](092-external-key-semantics-absorption-and-thumb-key-restructure.md)、
[ADR-135](135-generic-thumb-key-ime-toggle-delegate.md)、
[ADR-141](141-henkan-muhenkan-delegate-inactive-recovery.md)、
[ADR-147](147-thumb-key-delegate-defers-to-user-passthrough.md)、
[ADR-149](149-physical-ime-key-activation-defers-forced-set-open.md)
（案D の Blocker。旧決定3が同根の Blocker を逆方向から踏んでいたことが
opus-adversarial-consult r1 で判明、現決定1はこれを回避する設計）、
[ADR-151](151-actuation-delegate-by-default-drift-scoped-to-ownership.md)、
[ADR-152](152-keystroke-step-source-sink-pipeline.md)（隣接するが方向が
逆の保留中構想）、[ADR-154](154-delegate-shadow-toggle-exclusivity-off-to-on-transition.md)
（ADR-149「案C」続報、本ADRが決定2としてフォールバック維持する既存
delegate機構自体の排他性の穴——提案中・未実装）、
`.claude/rules/fix-requires-evidence.md` の「キー選択（IME ON/OFFに
送るVK）」表。
