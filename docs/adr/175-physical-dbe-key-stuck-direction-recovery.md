---
id: ADR-175
title: |-
  物理半角/全角キー（VK_DBE_SBCSCHAR/DBCSCHAR）の固定方向マッピングをやめ、
  Toggleとして解決することでIME ON固着を解消する（BUG-142）
status: |-
  起草・opus-adversarial-consult round4反映済み。**round4判定: 設計判断
  （方針・適用条件・実装層・スコープ）は収束、Blockerなし**（2026-09-15）。
  round1が当初案（no-op N回連続検出→フォールバック送信）にBlocker5件・
  Major8件を検出し、提示した代替案（`keys.ime_detect.toggle`にこの2VKを
  追加しToggle解決に変える）を実機A/Bで検証した結果、**固着が解消する
  ことを確認した**（dragonflyg4）。round2が実装形態とスコープにBlocker
  3件を検出し、round3がその2択（実装層は`enrich_ime_relevance`の既存
  override機構拡張、スコープは`AppImeProfile::InputRelay`のみ除外）に
  回答、round4は新規に「旧no-op分岐にぶら下がる2つのeisu救済が
  0xF3/0xF4では発火しなくなり復帰が2押しに変わる」ことを見落としとして
  指摘した上で、既存の`eisu_recovery.rs`規則に従い意図的に受け入れる
  形で決着した。文書上の残訂正（B4/B6の引用関数名、Linux実行不可の
  テスト箇所の明記）も反映済み。round5で最終確認のうえ実装着手。
related_adr:
  - "ADR-121"
  - "ADR-153"
  - "ADR-172"
  - "ADR-173"
  - "ADR-174"
---

# ADR-175: 物理半角/全角キー（`VK_DBE_SBCSCHAR`/`VK_DBE_DBCSCHAR`）の固定方向マッピングをやめ、Toggleとして解決することでIME ON固着を解消する（BUG-142）

## 背景・確定した事実（2026-09-15、実機検証で裏取り済み）

[BUG-142](../known-bugs/BUG-142.md)（Windows Terminal + PowerShell + GJI）で、
以下の状態遷移が実機で確認されている:

```
IME OFF, Engine OFF
  → 変換キー1回タップ →
IME ON, Engine OFF
  → 半角/全角キー1回タップ →
IME ON, Engine ON
  → 半角/全角キー1回タップ →
IME ON, Engine ON  ← 以降、何回半角/全角を押してもここに固着
```

固着は実際に文字を打って確認済み（かなのまま変わらない）。

### 根本原因: 固定方向マッピング×OSの同一方向報告×no-op誤判定

`crate::vk`は`VK_DBE_SBCSCHAR`(0xF3)を「半角モード（IME OFF扱い）」、
`VK_DBE_DBCSCHAR`(0xF4)を「全角モード（IME ON）」という**固定の絶対方向**
として分類する（`hook.rs::classify_ime_relevance`→`vk::ImeKeyKind::
shadow_effect()`）。`runtime/key_pipeline.rs::kp_stage_shadow_ime_toggle`は
この固定方向を`shadow_action`として採用し、write後に`effective_open()`が
書き込み前の`current`と一致する場合（`:1544`。**この一致判定はbelief書込み
（`write_physical_key`）後の`reduce()`結果と書込み前スナップショットの比較
であり、`new_val`同士の単純比較ではない**——旧版のこのADRはここを
`new_val == current`と誤記していた）「actuate不要」と判断し、
`GjiDirectStrategy`の送信自体を呼ばない（`[shadow-toggle] no-op`ログ）。

固着中、物理半角/全角キーを押すたびにOSが**同じVKを繰り返し報告**する
ことを実機で確認済み（`docs/known-bugs/BUG-142.md`参照。**旧版のこのADRが
「0xF4固定」の出典として引用していたログ行は実際には`vk=0xF2`
（`VK_DBE_HIRAGANA`、無変換/変換キーが状況により生成する別のVK、
`transport.rs`のドキュメント参照）であり誤りだった。0xF4での固着の
実例は`BUG-142.md`のPhase D節を参照**）。ユーザーの意図は明らかに
「反対方向にしたい」だが、awase側は「OS報告方向で、既に一致→変更不要」
と機械的に判断し、送信を一度も試みない。

### OS側の方向報告が同じ値になる理由（未解明ではない）

旧版のこのADRは「なぜOS側が同一方向を報告するのか未解明」として非
スコープにしていたが、これは不正確だった。`transport.rs`には次の記述が
既にある:

> NICOLAの物理「IME ON」キー（scan 0x70）は、IMEが既に目的の状態にある時に
> 押されると`VK_DBE_HIRAGANA`(0xF2)の代わりにこれらの`VK_DBE_*`を生成
> することがある（実機で0xF0/0xF1を確認）。

つまり**OSが報告するVKは実IME状態の関数である**という性質はこのリポジトリ
内で既知の性質として文書化済みである（BUG-52由来）。「OSが方向を間違えて
いる」のではなく「OSは（awase自身が0xF3/0xF4を常時Suppressして実IMEへの
唯一の影響経路を握っているため）ある種の内部状態に応じてVKを返しており、
その内部状態とawaseのbeliefが食い違っている」という読み方の方が、
Phase A（awase停止で10/10回正常）・Phase E（taskkillで即復帰）の観測とも
整合する。**この論点を非スコープにする理由は「未解明だから」ではなく、
「OS/ドライバ側の内部状態がどうであれ、awase側がその方向報告を無条件に
信頼して送信を省略する設計になっていること自体が脆い」という、awase側の
判断ロジックを堅牢化する方が実効性が高いため**、と訂正する。

### 決定的な検証1: 送信経路自体は生きている（Ctrl+無変換で実機確認）

固着状態のままCtrl+無変換（`ime_controller.rs::GjiDirectStrategy`が送る
`VK_IME_OFF`(0x1A)の通常のSendInput、`UserIntentSource::Command`経由）を
押すと、**実タイピングで確認可能な形で実際にIMEがOFFになる**ことを
実機確認した。`WM_IME_CONTROL`直接書込み・物理DBEキーのVK直接SendInput
注入はどちらも実タイピングでは効かなかった（APIは成功を返す）ため、
**固着の原因はGJIへの配送失敗ではなく、awase自身が送信を試みていない
こと**に絞り込まれる。

### 決定的な検証2: `Toggle`解決への変更で固着が実機で解消した

opus-adversarial-consult round1が、当初案（no-op N回連続検出→
Ctrl+無変換型フォールバック送信）にBlocker5件・Major8件を検出した
（詳細は下記「round1レビューで棄却された当初案」節）過程で提示した
代替案を、コード変更なしで実機検証した:

```toml
[keys.ime_detect]
toggle = ["VK_DBE_DBCSCHAR", "VK_DBE_SBCSCHAR"]
```

`focus_tracker.rs::enrich_ime_relevance`がこれを`event.ime_relevance.
sync_direction = Some(ShadowImeAction::Toggle)`として設定し、
`kp_stage_shadow_ime_toggle`の`intent_kind`解決で（`shadow_action`より
優先度が高い）`IntentKind::SyncKey`として採用される。`Toggle.
resolve(current) = !current`のため、OSがどちらのVKを報告してもbeliefが
常に反転し、no-op分岐に落ちなくなる。

**実機A/Bで確認済み**（2026-09-15、dragonflyg4）: この設定を投入して
awaseを再起動し、BUG-142の再現手順（変換キー1回→半角/全角キーを
2〜3回）を再試行したところ、**固着が解消し、正しく交互切替するように
なった**（ユーザー確認。ただしround2 M5指摘のとおり、この時点では
`[apply-ime] GJI direct: send 0x001A`のような実送信ログでの機序裏取りは
未実施——次の実機確認で取ること）。

**注意（round3 MR4）**: この実機A/Bは`keys.ime_detect.toggle`経由
（`IntentKind::SyncKey`、`is_japanese_ime()`ゲートを通らない）で行った。
下記「実装層」の決定（`enrich_ime_relevance`のoverride拡張、
`IntentKind::PhysicalImeKey`）は`is_japanese_ime()`ゲートを通る経路になる
（`should_upgrade_is_japanese_ime`が同じ打鍵でゲートを実質的に常に
満たすよう昇格させるため実害は小さいが、昇格より前に評価される最初の
1打鍵だけは経路が異なる）。**このA/Bは方針の検証であり、実装する経路と
は`intent_kind`が異なる**——実装後に同じA/Bを再実施すること
（下記「次のアクション」4）。

### V5（round2指摘）: Toggleと絶対方向解決は「旧no-opケース」でしか
差が出ない

`ShadowImeAction::resolve`（`src/types.rs:156-162`）から機械的に導ける
性質: 絶対方向`A`の結果を`v`とすると、`v != current`のとき`v ==
!current`（bool値のため）＝`Toggle.resolve(current)`と**同値**になる。
両者が異なるのは`v == current`のとき、すなわち**`:1544`のno-op分岐に
落ちるケースだけ**である。つまり本変更の影響範囲は「これまでno-opとして
何もしなかった打鍵」に厳密に限定される——これは決定を支持する最も強い
論拠であると同時に、下記「残るリスク」の影響範囲の上限を与える
（リスクは「正当な冪等操作だったno-op」に限られる）。

## 決定（round3で確定）

`VK_DBE_SBCSCHAR`/`VK_DBE_DBCSCHAR`の解決方向を、固定方向
（`ShadowImeAction::TurnOff`/`TurnOn`）から`ShadowImeAction::Toggle`へ
変更する。適用条件は`profile != AppImeProfile::InputRelay`のみ
（`active_ime_kind`では絞らない）。

### 適用条件: `!InputRelay`のみ、`active_ime_kind`では絞らない

- **`AppImeProfile::InputRelay`を除外する**（round2 B2）: InputRelayでは
  物理0xF3/0xF4が中継先へそのままAllowされる一方、awase側のactuation
  抑止はgate層にありshadow-toggle自体にInputRelay分岐が無い。Toggle化
  すると「実IMEは絶対方向で動く一方beliefだけがToggleで反転する」新しい
  乖離（中継窓でタイプ中にNICOLAエンジンが勝手にOFFになる）を生む。V5の
  同値性により、除外した窓では従来（絶対方向）の挙動がそのまま残るため
  副作用は無い。`InputRelay`以外のプロファイルでは0xF3/0xF4が構造的に
  必ずSuppressされ、かつawaseがactuationを所有することをround2で確認
  済み（`transport.rs:378-419`、`ImeKindId`は`Gji`/`MsIme`の2値のみ
  〈`state/ime_kind.rs:19-24`〉、ImmCrossは外側のアームで無条件Suppress）
  ——したがって適用条件は実質`!InputRelay`と同値であり、条件式を1つに
  畳める。
- **`active_ime_kind == Gji`でのスコープ限定は採らない**（round3 2-a）:
  `active_ime_kind()`はGJI未検出時に安全側の`MicrosoftIme`を返す既定値
  であり、フォーカス直後の短い窓（`WM_IME_KIND_CHANGED`到達前）で
  「実際はGJIなのに`active_ime_kind`はまだ`MicrosoftIme`のまま」という
  構造的なcold windowが既にコードコメントに記録されている
  （`key_pipeline.rs:163-174`、BUG-116/ADR-137が踏んだ罠と同型）。GJI
  スコープを付けると、固着から抜けようとしてユーザーが連打する場面
  （アプリ切替直後でありうる）でちょうど効かなくなる。
- **B3（証拠が1環境のみ）の解消**（round3 2-b）: 「MS-IME環境で正当な
  冪等操作が壊れる」というリスクは、V5と「OSが報告するVKは実IME状態の
  関数である」（上記「OS側の方向報告が同じ値になる理由」節）を接続すれば
  defuseできる——正常系では状態が変わるたびに報告VKも入れ替わるため、
  V5の同値によりToggleと絶対方向の結果は一致する。両者が食い違うのは
  「報告VKが実状態に追従しなくなっているとき」＝**まさにBUG-142の固着
  そのもの**であり、その環境でもこの性質が成り立つなら正当な冪等操作は
  壊れない。MS-IME環境（Imm32Unavailable=Chrome、Standard=メモ帳等）は
  実装後のsoakで見る扱いとし、実装前の必須ゲートにはしない（安く取れる
  決定的データとして、その環境で半角/全角キーを連打し既存の`[hook]
  IME-mode vk=0x..`ログに同じVKが2回連続で現れるかを見れば、この仮定が
  その環境でも成立するか1分で確認できる）。
- **後方互換の逃げ道は既存configで確保されている**（round3 2-c、新しい
  設定項目は不要）: 万一MS-IME環境等で実害が出た場合、ユーザーは
  `keys.ime_detect.{on,off}`（`sync_direction`は`shadow_action`
  overrideより優先、`key_pipeline.rs:1418-1428`）で絶対方向を個別に
  復元できる:
  ```toml
  [keys.ime_detect]
  on  = ["VK_DBE_DBCSCHAR"]
  off = ["VK_DBE_SBCSCHAR"]
  ```
  `init_ime_sync_keys`の親指キー衝突チェックにも引っかからない（既定の
  親指キーは無変換/変換〈0x1C/0x1D〉であり0xF3/0xF4とは別VK）。

### 実装層: `enrich_ime_relevance`の既存override機構を拡張する

`src/config.rs::ImeDetectConfig::default()`への追加は**撤回する**——
`Config::save`（`config.rs:890-891`）が`ImeDetectConfig`を含む全フィールド
を明示的にシリアライズするため、設定GUIの保存やトレイの自動起動トグル
（`save_auto_start`）を一度でも経由したユーザーには新しい既定値が届かない
（round2 B1）。

round1時点で対案とした「`hook.rs::classify_ime_relevance`/`vk::
ImeKeyKind::shadow_effect()`側で分類自体を変える」も**採らない**
（round3の判定）: 決め手は、ADRが採択した`InputRelay`除外を表現できない
こと——`classify_ime_relevance`はVK単体からの純粋分類で`&self`を持たず、
プロファイルに触れない。この層で実装すると結局
`kp_stage_shadow_ime_toggle`に新しいプロファイル分岐を足すか
（`fix-requires-evidence.md`の「IME belief」再発ファミリー、既に
`#[expect(clippy::cognitive_complexity)]`が付くほど分岐過多）、
`enrich_ime_relevance`側で打ち消す（＝結局この案を併用する）かの
どちらかになり、選択肢1単独では成立しない。加えて`vk.rs`の絶対方向
分類（`ImeKeyKind::Deactivate`/`ActivatePair`という命名自体が絶対方向を
表す）はawaseの方針ではなく`transport.rs:218-222`がBUG-52の実機結果
として記録した**Windows仕様の事実の記述**であり、ここに方針を書くのは
`docs/layer-boundaries.md`が定める「プラットフォーム層は生入力を分類、
方針は上位層が決める」という分業と逆行する。

**採用する実装**: `runtime/mod.rs::enrich_ime_relevance`が既に持つ
条件付きoverride連鎖（`resolve_mode_key_shadow_override_for_event(...)
.or_else(|| resolve_henkan_muhenkan_shadow_override_for_event(...))`、
`:537-553`）にもう1段`.or_else()`を足し、0xF3/0xF4×`!InputRelay`のとき
`Some(ShadowImeAction::Toggle)`を返す新しいリゾルバを追加する。

- **書き込み箇所は1箇所のまま維持する**: `tests/architecture_guard.rs:
  758-774`の`ime_relevance_shadow_action_writes_are_accounted_for`が
  `hook.rs`=1・`runtime/mod.rs`=1に書き込み箇所数を固定している。
  `:552-554`の`if let Some(action) = override_action { event.
  ime_relevance.shadow_action = Some(action); }`という既存の単一
  書き込みをそのまま使う（新しい書き込み箇所は増やさない）。
- **判定ロジックの置き場所**: 既存の2つのリゾルバ（`resolve_mode_key_
  shadow_override_for_event`/`resolve_henkan_muhenkan_shadow_override_
  for_event`）はどちらも`gji_charset_autodetect.rs`側の純関数であり、
  `runtime/mod.rs`は合成と書き込みだけを行う——この分業をそのまま踏襲
  する。新しい純粋述語（0xF3/0xF4×profile→`Option<ShadowImeAction>`）は
  `state/key_sequence_policy.rs`（`gji_direct_applicable(kind)`/
  `ms_ime_direct_applicable(kind, profile)`という同型の「プロファイル×
  IME種別の純粋述語」を既に集めている場所）に置く。`runtime/mod.rs`に
  条件式を直書きしない——これによりround2 M6の回帰テストがLinuxで書ける
  ようになる。

### なぜこの決定がround1のBlockerを構造的に回避できるか

- **B1（no-op誤判定の3ケース混同）**: `Toggle`は`resolve(current) =
  !current`が常に`current`と異なるため、V5が導くとおり「絶対方向の
  結果が`current`と一致していた（＝旧no-opケース）」場合にのみ挙動が
  変わる。**ただし`:1544`は書込み後の`effective_open()`を見るため、
  `force_guard.rs`の`overrides_explicit_intent()`を持つguardがactiveな
  場合はToggleでも書込みが反映されずno-op分岐に落ちる**（この場合
  0xF3/0xF4はADR-121 D1の対象外のためreassertも走らず「静かに何も
  起きない」——guard作動中は意図的に何もしないという既存の仕様であり、
  `ime_model.rs::resolve_open_at`が返す`DecidedBy{base, guard_override}`
  を使えば、この分岐に落ちたことを`guard_override`付きのログとして
  区別して残せる）。旧案が抱えていた「guardと正面衝突する新しい発振
  経路」という問題（B1が指摘した本質）は、Toggleが新しい送信を追加で
  発行するわけではない（既存のno-op分岐に自然に収束するだけ）ため
  発生しない。
- **B2（auto-repeat除外の機構が使えない）/ B3（Nに自由度が無い）**:
  「N回連続no-op検出」という設計自体を廃止したため、この2つの論点は
  丸ごと消滅する。
- **B4（フォールバック実装形態の曖昧さ）/ B6（`GjiDirectStrategy::apply`
  とbelief書込みの混同）**: 解消。採用した実装（`enrich_ime_relevance`の
  `shadow_action` override）は既存の`IntentKind::PhysicalImeKey`経路
  （`write_physical_key`、`IntentWitness::from_physical`、
  `key_pipeline.rs:1421-1427`/`:1465-1480`）をそのまま通るため、新しい
  actuation合流点を作らない（`write_sync_key`と`write_physical_key`は
  どちらも`UserImeSetIntent`+`record_explicit_intent`を経由する完全に
  対称な実装であり、`record_explicit_intent`のdocも両方を「呼んでよい
  3箇所」として列挙している——round2 V1で確認済み）。`.claude/rules/
  fix-requires-evidence.md`の「IME actuation合流点」表に新しい入口は
  増えない。
- **B5（0xF2との出典混同・ADR-121 D1との二重actuationリスク）**: 本ADRの
  対象を`VK_DBE_SBCSCHAR`/`VK_DBE_DBCSCHAR`（0xF3/0xF4）に明示的に限定し、
  `VK_DBE_HIRAGANA`(0xF2)は対象外とする（下記「非スコープ」参照）。
  ADR-121 D1（`reassert_explicit_physical_key`）の発火条件は`vk ==
  VK_DBE_HIRAGANA`のみであり、本ADRの変更とは排他的に重ならない。

### 残るリスク・受け入れるトレードオフ（round4で1節に集約）

- **`dbe_mode_key_policy=Passthrough`（隠し設定）では0xF3/0xF4の
  KeyDownが常にSuppressされるようになる。** Toggleは必ずbeliefを反転
  させるため`shadow_toggled`が常にtrueになり、`transport.rs:416-419`の
  `ime_actuation_owned && (shadow_toggled || ...)`によりPassthrough設定
  でも常時Suppressに固定される（変更前は「方向が一致しno-opだったとき」
  だけAllowされていた）。この設定を使うユーザーは稀だが、`transport.rs::
  plan`は`fix-requires-evidence.md`の「物理IMEキーのSuppress/Allow配送
  判断」ファミリーに属するため記録する。
- **`resolve_pending_thumb_as_single`が返す旧no-op分岐の2つの救済
  （stale `ObservedEisu`の訂正、半角英数持続トグルの解除）は、0xF3/0xF4
  では発火しなくなる。** `eisu_reset_on_turn_on_while_open`
  （`state/eisu_recovery.rs:135-144`）は`action_is_turn_on`（`matches!
  (turn_on_direction, ShadowImeAction::TurnOn)`）を要求するが、`action`
  が`Toggle`になるとこれは常に`false`になる。**この結果を意図的に
  受け入れる**——`eisu_recovery.rs:128-129`の既存docが「`Toggle`（当時は
  VK_KANJI）はON/OFFどちらへ向かうか一意に決まらないため対象外、
  TurnOn系のみが『ひらがなへ戻す』という意図を一意に持つ」と定めており、
  0xF3/0xF4を`Toggle`へ移すことはこの既存規則に従って自動的に「eisu
  救済の対象外クラス」へ移動させることを意味する。実害は、`IME open
  のままconvだけEisuに固着`という状態からの復帰が、従来の1押し
  （`AssumedRomaji`へ即時復帰）から、1押し目でOFFへ反転→2押し目で
  OFF→ONとなり対称処理（`eisu_reset_on_ime_on`）が発火する**2押しでの
  復帰**に変わること。
- **無変換/変換キー単独タップ由来の初期位相ズレ（0xF2、BUG-142再現手順の
  第1歩）は本ADRの変更後も残る。** Toggle解決はbeliefと実IMEの位相が
  合っていることを前提にする機構であり、この初期位相ズレの復帰手段は
  Ctrl+無変換/Ctrl+変換（絶対方向の`keys.ime_off`/`ime_on`）のみになる。
  変更前は0xF3/0xF4自体が絶対方向だったため、それ自体が位相の再同期
  手段になっていた——**Toggle化はその再同期能力を手放すトレードオフ**
  である（V5の同値性と矛盾しない。再同期が効いていたのはまさに旧
  no-opケースそのものであるため）。
- **`is_japanese_ime()`ゲートは実装層の決定により保持される。** 採用した
  実装（`shadow_action` override、`IntentKind::PhysicalImeKey`経路）は
  このゲートの内側を通るため、awaseが「日本語IMEでない」と信じている
  間はbeliefが反転しない（round2 M8はこのゲートの外側にある
  `sync_direction`経路〈実機A/Bで使った経路〉を前提にした指摘であり、
  採用実装には該当しない）。ゲート外に出るのは、ユーザーが上記「後方
  互換の逃げ道」（`keys.ime_detect.on/off`）でopt-outした場合のみ。
- **`engine_on_ime_key`/`engine_off_ime_key`（既定`None`、doc例が
  `VK_DBE_DBCSCHAR`）を設定したユーザーでは、awase自身がこのVKを
  SendInputする。** Toggle化後はこの自己送信のエコーがbeliefを
  **反転**させうる（絶対方向なら冪等だった）。実際の防御は自己注入
  フィルタ（`hook.rs`）とBUG-14早期return（`event.injected`はユーザー
  意図に昇格しない）の2段のみ——既定`None`のため既定構成では発生しない
  が、設定したユーザーでは自己注入フィルタが唯一の防御になる。

### `gji_thumb_key_ime_toggle`の非冪等警告への反論（round2 M4）

`config.rs`の`gji_thumb_key_ime_toggle`が警告する「Toggleは非冪等。
誤発火が状態の反転になり連続誤発火で発振しうる」への反論は「対象キーが
違う」では弱い。実際に効いている防御を名指しする: (a) BUG-14早期return
（`key_pipeline.rs`、`event.injected`な打鍵はユーザー意図に昇格しない）、
(b) `init_ime_sync_keys`が親指キーと同一VKのsync登録を弾く（BUG-140）
ため、無変換/変換の誤発火経路（`resolve_pending_thumb_as_single`、
親指キー専用）とは両立しない。上記「残るリスク」節の
`engine_on/off_ime_key`自己送信エコーが、この2段の防御をすり抜けない
唯一の経路である。

### 新しい純粋述語のシグネチャ: 親指キー設定時は`None`を返す（round4 MR4-6）

既存の2つのリゾルバは扱いが分かれる: `resolve_mode_key_shadow_override_
for_event`（`gji_charset_autodetect.rs`）は`thumb_pair`を受け取り、
**親指キーなら`None`**（静的`shadow_action`を守る）。
`resolve_henkan_muhenkan_shadow_override_for_event`は親指キーでも
overrideする（無変換/変換には守るべき静的`shadow_action`が無いため）。
0xF3/0xF4は**静的`shadow_action`を持つ側**（Hiragana/Katakanaと同型）
なので、新しい純粋述語も`thumb_pair`引数を受け取り、**親指キーに設定
されている場合は`None`を返し従来の絶対方向マッピングを維持する**設計に
揃える。整合を欠くと、`delegate_owns_mode_key_shadow_toggle`との
組み合わせでADR-141 C2型の「誰も何もしない」穴になりうる
（既存docが警告済み）。

### 回帰テスト（round3 MR3・round4 MR4-2で確定）

実装層を`enrich_ime_relevance`のoverride拡張に確定したことを受け、
以下3本を追加する:

- (a) `state/key_sequence_policy.rs`に置く新しい純粋述語（0xF3/0xF4×
  profile×thumb_pair→`Option<ShadowImeAction>`）の決定表を固定する
  （`InputRelay`または親指キー設定時に`None`、それ以外で`Some(Toggle)`
  ——既存の`gji_charset_autodetect.rs`のリゾルバ群のテストと同形式）。
  `state`/`focus`はどちらも`lib.rs`でgateされておらずLinuxで走る。
- (b) `transport.rs::plan_tests::run_plan_matrix`に`shadow_action=
  Some(Toggle)`の0xF3/0xF4行を追加しSuppress判定が変わらないことを
  固定する（round2 V2の内容をテストで凍結）。**`runtime`モジュール全体
  が`lib.rs`で`#[cfg(windows)]`gateされているため、このテストはLinux
  では実行されない**（`cargo test --list`にも出現しない、CLAUDE.md
  「Commands」節が警告する既知の罠）。`windows-build` CIでのみ実際に
  走る——ローカルでは`cargo check --target x86_64-pc-windows-msvc
  -p awase-windows --tests --lib`でコンパイル確認までに留める。
- (c) `ShadowImeAction::resolve`のV5同値性（`v != current`のときToggle
  と一致、異なるのは`v == current`のときだけ）を`src/types.rs`
  （ルート`awase`クレート、`cargo test --lib`で走る）の単体テストで
  固定する——本ADRの正当化そのものであり、壊れたら気づける形にして
  おく価値が高い。

実送信ログでの機序裏取り（旧M5）は下記「次のアクション」に既に含まれる。

## round1レビューで棄却された当初案（記録として残す）

当初、「物理DBEキーの新規押下（auto-repeatでない）でno-opがN回連続した
場合、Ctrl+無変換型のフォールバック送信を行う」という案を提示したが、
opus-adversarial-consult round1が以下を指摘し棄却した（詳細は
`opus-review-adr175-round1.md`、ワークツリー内の一時ファイルのため要点を
ここに集約）:

- no-op判定は`new_val`同士の比較ではなく、force-ON guard等による書込み
  抑止も含めて同じ分岐に落ちるため、guard作動中にフォールバックがguardと
  正面衝突する新しい発振経路になりうる。
- auto-repeat除外に使おうとした`HOOK_STATE.physical_key_state`は
  ADR-121 D2が同じ理由で既に棄却済みの機構であり、代替の`RawKeyEvent::
  was_down`はBUG-142が記録する「KeyDownは来たがKeyUpが来ない」状態
  （`physical_key_state`が9分以上trueに張り付いた観測）により固着時に
  ちょうど無効化される。
- 「N回連続」のNには実質的に選択肢がない——N≥2は押下:切替が非対称になり
  UXが壊れたままで、N=1は本ADRが最終的に採用した「Toggle解決」と数学的に
  同値。

## 非スコープ

- `VK_DBE_HIRAGANA`(0xF2)が絡む固着（`BUG-142.md`の「固着の完全な再現と
  ログでの機序特定」節が記録するログ）は本ADRの変更では直らない
  可能性がある——このケースはADR-121 D1（`reassert_explicit_physical_key`）
  の管轄であり、別途扱う。
- OS/ドライバ側がなぜある内部状態に応じて特定のVKを生成するかの、
  OS実装レベルでの完全な解明。
- ADR-174（パススルー+belief再観測）・ADR-173（`solo_tap_ime_action_apps`）
  の設計自体の変更——両ADRとも保留のまま、本ADRとは独立に扱う。
- 変換キー単独タップでIME ON・Engine OFFになった直後、Engine側も自動的に
  ONにするという別の改善要望（ユーザー指摘、2026-09-15）——本ADRの対象は
  「物理半角/全角キーの固着解消」のみであり、この要望は別途調査・別ADR
  化を検討する。

## 次のアクション

1. 上記「実装層」の決定に沿って`state/key_sequence_policy.rs`に純粋述語
   を実装し、`runtime/mod.rs::enrich_ime_relevance`のoverride連鎖に
   1段追加する（config既定値は変更しないため、`ImeDetectConfig::
   default()`関連のCIテストとの衝突確認は不要）。`runtime/mod.rs`は
   `#[cfg(windows)]`配下のため、Linux上では
   `cargo check --target x86_64-pc-windows-msvc -p awase-windows --tests
   --lib`でコンパイル確認する（`cargo check -p awase-windows`単体では
   このモジュールがコンパイル対象に入らない）。
2. 上記「回帰テスト」の3点（決定表・`plan_tests`行追加・V5同値性の単体
   テスト）を追加する。(b)は`windows-build` CIでのみ実行されることに
   留意する。
3. 実装後、develop最新（config手動追加なし）で改めて実機A/Bを行い、
   固着が解消することと、`[apply-ime]`の実送信ログ（`AlreadyMatched`
   でないこと）を確認する。この実装経路は`IntentKind::PhysicalImeKey`
   （実機A/Bで検証した`IntentKind::SyncKey`とは`is_japanese_ime()`
   ゲートの有無が異なる）であることに留意する。
4. MS-IME環境（Chrome/メモ帳等）で半角/全角キー連打時に同じVKが2回連続
   で現れないことを確認する（安く取れる裏取り、コード変更不要）。

## 関連

BUG-142（本ADRが解消を目指す「IME ON固着」）、ADR-121（物理IME訂正キー
no-op時の冪等再送、`VK_DBE_HIRAGANA`専用のため本ADRの対象外）、
ADR-153（`explicit_ime_action_target`、無変換/変換の明示config設計）、
ADR-172（TsfNative ON方向救済4系統の整理）、ADR-173
（`solo_tap_ime_action_apps`、却下された代替案）、ADR-174
（パススルー+belief再観測、Blocker未解消のまま保留）。
