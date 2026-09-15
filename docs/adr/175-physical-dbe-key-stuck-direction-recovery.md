---
id: ADR-175
title: |-
  物理半角/全角キー（VK_DBE_SBCSCHAR/DBCSCHAR）の固定方向マッピングをやめ、
  Toggleとして解決することでIME ON固着を解消する（BUG-142）
status: |-
  起草・opus-adversarial-consult round2反映中。round1が当初案（no-op N回
  連続検出→フォールバック送信）にBlocker5件・Major8件を検出し、その過程で
  提示した代替案（`keys.ime_detect.toggle`にこの2VKを追加しToggle解決に
  変える）を実機A/Bで検証した結果、**固着が解消することを確認した**
  （2026-09-15、dragonflyg4）。round2は「Toggle解決」という方針自体は支持
  したが、実装形態とスコープにBlocker3件を検出した: (1)
  `ImeDetectConfig::default()`への追加は`Config::save`が全フィールドを
  書き出すため一度でも設定を保存した既存ユーザーに届かない、(2)
  `InputRelay`プロファイルでは物理キーが中継先へAllowされる一方belief
  だけがToggleで反転し新しい回帰を生む、(3) 0xF3/0xF4はWindows仕様上の
  絶対モードキー（BUG-52）であり、証拠がTsfNative+GJIの1環境のみのまま
  全環境の既定を変えることの妥当性が未検証。round3で実装層・スコープを
  詰める。
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

## 決定（方針）

`VK_DBE_SBCSCHAR`/`VK_DBE_DBCSCHAR`の解決方向を、固定方向
（`ShadowImeAction::TurnOff`/`TurnOn`）から`ShadowImeAction::Toggle`へ
変更する。**ただし、この変更が安全なのは「awaseがこの物理キーを常に
Suppressし、かつactuationを所有している」組み合わせに限る**（round2
B2/B3の指摘、下記「適用条件」参照）。

### 適用条件（round2 B2/B3を受けて追加）

- **`AppImeProfile::InputRelay`を除外する**（B2）: InputRelayでは物理
  0xF3/0xF4が中継先へそのままAllowされる一方、awase側のactuation抑止は
  gate層にありshadow-toggle自体にInputRelay分岐が無い。Toggle化すると
  「実IMEは絶対方向で動く一方beliefだけがToggleで反転する」新しい乖離
  （中継窓でタイプ中にNICOLAエンジンが勝手にOFFになる）を生む。V5の
  同値性により、除外した窓では従来（絶対方向）の挙動がそのまま残るため
  副作用は無い。
- **既定を全環境に広げる根拠が現時点でTsfNative+GJI+Windows Terminalの
  1環境に限られる**（B3）。0xF3/0xF4はWindows仕様上の絶対モードキーで
  あり（BUG-52、`transport.rs`参照）、Toggle化は「既に半角のはずだが
  念のためもう一度半角キーを押す」という正当な冪等操作を破壊しうる
  （絶対方向なら何も起きないが、Toggleでは全角に飛ぶ）。round3で
  以下のいずれかを決定する:
  1. MS-IME環境（Imm32Unavailable=Chrome、Standard=メモ帳等）で実機確認
     してから全環境の既定にする。
  2. `active_ime_kind == Gji`（かつ非InputRelay）にスコープを絞る。
  3. 既定にはせず、BUG-142の環境向けの`app_overrides`opt-inとして出す。

### 実装層（round3で選ぶ、B1を受けてround1時点の主案は撤回）

`src/config.rs::ImeDetectConfig::default()`への追加は**撤回する**——
`Config::save`（`config.rs:890-891`）が`ImeDetectConfig`を含む全フィールド
を明示的にシリアライズするため、設定GUIの保存やトレイの自動起動トグル
（`save_auto_start`）を一度でも経由したユーザーには新しい既定値が届かない
（round2 B1）。今回の実機A/Bは`config.toml`を手編集した状態での検証であり、
「既定変更で全ユーザーが直る」ことの証拠にはならない。round3で以下の
どちらかを選ぶ:

1. **`hook.rs::classify_ime_relevance`/`vk::ImeKeyKind::shadow_effect()`側で
   0xF3/0xF4の分類自体をToggleに変える**。配信は確実（config非依存）だが、
   この関数はVK単体からの純粋分類でプロファイル/`active_ime_kind`の
   コンテキストを持たないため、上記「適用条件」（InputRelay除外・GJI
   スコープ）をこの層だけでは表現できない可能性がある。
2. **`runtime/mod.rs::enrich_ime_relevance`の既存の条件付きoverride機構
   （`resolve_henkan_muhenkan_shadow_override_for_event`等、GJI/MS-IME
   自動検出結果に応じて`shadow_action`を後から上書きする、無変換/変換
   キー向けに既に存在する仕組み）を拡張し、0xF3/0xF4にも同型の
   プロファイル/`active_ime_kind`条件付きoverrideを追加する**。config
   非依存で配信も確実、かつ適用条件（InputRelay除外・GJIスコープ）も
   自然に表現できるが、新しいoverride分岐が増える（`fix-requires-evidence.md`
   の「IME belief」再発ファミリーへの追加）。

どちらを選ぶかはround3で判断する。

### なぜこの決定がround1のBlockerを構造的に回避できるか

- **B1（no-op誤判定の3ケース混同）**: 解消。`Toggle`は`resolve(current)
  = !current`が常に`current`と異なるため、そもそもno-op分岐（`:1544`）に
  落ちる余地がない。force-ON guard等による書込み抑止（B1が指摘した
  ケース2）は残るが、これは既存の一般的なguard機構の話であり、本ADRが
  新設する分岐ではない。
- **B2（auto-repeat除外の機構が使えない）/ B3（Nに自由度が無い）**:
  「N回連続no-op検出」という設計自体を廃止したため、この2つの論点は
  丸ごと消滅する。
- **B4（フォールバック実装形態の曖昧さ）/ B6（`GjiDirectStrategy::apply`
  とbelief書込みの混同）**: 解消。`Toggle`は既存の`IntentKind::SyncKey`
  経路（`write_sync_key`、`IntentWitness::from_sync_key`）をそのまま通る
  ため、新しいactuation合流点を作らない。`.claude/rules/
  fix-requires-evidence.md`の「IME actuation合流点」表に新しい入口は
  増えない。
- **B5（0xF2との出典混同・ADR-121 D1との二重actuationリスク）**: 本ADRの
  対象を`VK_DBE_SBCSCHAR`/`VK_DBE_DBCSCHAR`（0xF3/0xF4）に明示的に限定し、
  `VK_DBE_HIRAGANA`(0xF2)は対象外とする（下記「非スコープ」参照）。
  ADR-121 D1（`reassert_explicit_physical_key`）の発火条件は`vk ==
  VK_DBE_HIRAGANA`のみであり、本ADRの変更とは排他的に重ならない。

### round2 Major指摘（round3で反映）

1. **M1（自己矛盾の訂正）**: 「Toggleはno-op分岐に落ちる余地がない」と
   「force-ON guard等による書込み抑止は残る」は矛盾する——`:1544`は書込み
   **後**の`effective_open()`を見るため、`force_guard.rs`の
   `overrides_explicit_intent()`を持つguardがactiveならToggleでも
   no-op分岐に落ちる。この場合0xF3/0xF4はADR-121 D1の対象外のため
   reassertも走らず「静かに何も起きない」——仕様として明記し、
   `guard_override`付きのログ（`ime_model.rs::resolve_open_at`が返す
   `DecidedBy`）を残すこと。
2. **M3（`dbe_mode_key_policy=Passthrough`への影響が全面化）**: Toggleは
   必ずbeliefを反転させるため`shadow_toggled`が常にtrueになり、
   `dbe_mode_key_policy=Passthrough`（隠し設定）でも0xF3/0xF4の
   KeyDownが**常に**Suppressされるようになる（変更前は「方向が一致し
   no-opだったとき」だけAllowされていた）。`transport.rs::plan`は
   `fix-requires-evidence.md`の「物理IMEキーのSuppress/Allow配送判断」
   ファミリーであり、一行の言及が要る。
3. **M4（非冪等警告への反論を具体化）**: `gji_thumb_key_ime_toggle`の
   「Toggleは非冪等」警告への反論は「対象キーが違う」では弱い。実際に
   効いている防御を名指しする: (a) `key_pipeline.rs`のBUG-14
   早期return（`event.injected`な打鍵はユーザー意図に昇格しない）、
   (b) `init_ime_sync_keys`が親指キーと同一VKのsync登録を弾く
   （BUG-140）ため、無変換/変換の誤発火経路（`resolve_pending_thumb_
   as_single`、親指キー専用）とは両立しない。
4. **M5（実送信の裏取り）**: 実機A/Bの記録に`[apply-ime] GJI direct:
   send 0x001A`のような実送信ログが出ていたこと（`AlreadyMatched`
   ではないこと）を追記する。`already_matched`は直前に別経路が同方向へ
   applied済みなら吸収するため、「固着が直った」の観測が実は別要因
   （フォーカス変更でbeliefがリセットされた等）だった可能性を排除
   できていない。
5. **M6（回帰テストの強化）**: 「既定値そのもの」を固定するだけでは
   不足。以下を追加する（すべてLinuxで`cargo test -p awase-windows`
   実行可）: (a) `sync_direction=Some(Toggle)`の0xF3/0xF4 KeyDownが
   `:1544`のno-op分岐に落ちないことを純関数レベルで固定、(b)
   `transport.rs::plan_tests::run_plan_matrix`に`sync_direction`付きの
   行を追加しSuppress判定が変わらないことを固定、(c) InputRelayを
   除外するなら、その窓でToggle解決が発火しないことを固定。
6. **M7（0xF2の位相ズレは残る、トレードオフとして明記）**: BUG-142の
   再現手順の第1歩（変換キー1回タップ）は本ADRの変更後も「実IMEだけ
   ON、beliefは未追従」のまま残る——Toggle解決はbeliefと実IMEの位相が
   合っていることを前提にする機構であり、この初期位相ズレの復帰手段は
   Ctrl+無変換/Ctrl+変換（絶対方向の`keys.ime_off`/`ime_on`）のみになる。
   変更前は0xF3/0xF4自体が絶対方向だったため、それ自体が位相の再同期
   手段になっていた——**Toggle化はその再同期能力を手放すトレードオフ**
   であることを明記する（V5の同値性と矛盾しない。再同期が効いていたのは
   まさに旧no-opケースそのものであるため）。
7. **M8（`is_japanese_ime()`ゲート迂回、軽微）**: `sync_direction`経路は
   `is_japanese_ime()`ゲートの外側にあるため、awaseが「日本語IMEでない」
   と信じている間もbeliefが反転しうる。実害は小さい（同じ打鍵で
   `should_upgrade_is_japanese_ime`がゲートを実質的に常に満たすように
   昇格させるため）が、一行の言及を残すこと。

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

1. 本ADRをopus-adversarial-consult round3にかけ、「実装層」（classify側
   か既存override拡張か）と「適用条件のスコープ」（MS-IME実機確認/
   GJI限定/opt-in留め）を確定する。
2. round3の決定に沿って実装し、既存のCIテスト
   （`keys_defaults_do_not_collide_with_ime_detect_defaults`）が通ることを
   確認する。
3. round2 M6の3点（no-op非該当・plan_tests行追加・InputRelay除外）を
   回帰テストとして追加する。
4. 実装後、develop最新でも改めて実機A/Bを行い、config手動追加なしで
   固着が解消することと、`[apply-ime]`の実送信ログ（`AlreadyMatched`
   でないこと）を確認する。

## 関連

BUG-142（本ADRが解消を目指す「IME ON固着」）、ADR-121（物理IME訂正キー
no-op時の冪等再送、`VK_DBE_HIRAGANA`専用のため本ADRの対象外）、
ADR-153（`explicit_ime_action_target`、無変換/変換の明示config設計）、
ADR-172（TsfNative ON方向救済4系統の整理）、ADR-173
（`solo_tap_ime_action_apps`、却下された代替案）、ADR-174
（パススルー+belief再観測、Blocker未解消のまま保留）。
