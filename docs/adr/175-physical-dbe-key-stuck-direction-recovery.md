---
id: ADR-175
title: |-
  物理半角/全角キー（VK_DBE_SBCSCHAR/DBCSCHAR）の固定方向マッピングをやめ、
  Toggleとして解決することでIME ON固着を解消する（BUG-142）
status: |-
  起草・opus-adversarial-consult round1反映済み。round1が当初案（no-op N回
  連続検出→フォールバック送信）にBlocker5件・Major8件を検出し、その過程で
  提示した代替案（`keys.ime_detect.toggle`にこの2VKを追加しToggle解決に
  変える）を実機A/Bで検証した結果、**固着が解消することを確認した**
  （2026-09-15、dragonflyg4）。決定を当初案からToggle化へ全面的に置き換えた。
  round2レビュー未実施。
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
なった**（ユーザー確認）。

## 決定

`VK_DBE_SBCSCHAR`/`VK_DBE_DBCSCHAR`の既定の解決方向を、固定方向
（`ShadowImeAction::TurnOff`/`TurnOn`）から`ShadowImeAction::Toggle`へ
変更する。実装は`src/config.rs::ImeDetectConfig::default()`の`toggle`
リストにこの2VKを追加する形を主案とする（`hook.rs::classify_ime_relevance`
のハードコード分類自体を変更するのではなく、既存の「`ime_detect`の
config値が`shadow_action`の既定分類より優先される」という既存の優先順位
機構をそのまま使う——新しい合流点・新しい判定ロジックを追加しない）。

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

### 検討すべき残る論点（round2で詰める）

1. **既定変更の後方互換性**: `ImeDetectConfig::default().toggle`への追加が
   `keys.ime_toggle`等の既定値と衝突しないか、
   `vk.rs::keys_defaults_do_not_collide_with_ime_detect_defaults`
   （既存のCIテスト）で検証すること。
2. **`gji_thumb_key_ime_toggle`のdoc（`config.rs:409-421`）が警告する
   「Toggleは非冪等」リスクとの関係**: この既存の警告は無変換/変換キー
   （`VK_NONCONVERT`/`VK_CONVERT`）を対象にしたものであり、`VK_DBE_
   SBCSCHAR`/`DBCSCHAR`は常にSuppressされ実IMEへの影響経路がawase自身の
   送信に閉じている（`transport.rs::plan`のKANJI関連キー無条件Suppress
   ルール）という点で状況が異なる。この違いを本ADRで明示的に論じ、
   同じ「非冪等」リスクが実際に成立するかを確認すること。
3. **`already_matched`ゲート（`ime_controller.rs::gji_direct_
   already_matches`）との相互作用**: `Toggle`経由の送信もこのゲートを
   通る。直前に別経路で同じ方向へ既にapplied済みの場合は`AlreadyMatched`
   で吸収されうる——これは既存の一般的な挙動であり本ADRが変更するもの
   ではないが、実機確認手順にログ確認（`[apply-ime]`が`AlreadyMatched`
   ではなく実送信であること）を含めること。
4. **回帰テスト**: `crates/awase-windows/tests/`で、`ImeDetectConfig::
   default()`に`VK_DBE_SBCSCHAR`/`DBCSCHAR`の`toggle`エントリが含まれる
   ことを固定するテストを追加する（`.claude/rules/fix-requires-evidence.md`
   のキー選択reincidence family対応）。

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

1. 本ADRをopus-adversarial-consult round2にかけ、上記「検討すべき残る
   論点」を詰める。
2. `ImeDetectConfig::default()`への変更を実装し、既存のCIテスト
   （`keys_defaults_do_not_collide_with_ime_detect_defaults`）が通ることを
   確認する。
3. 回帰テストを追加する。
4. 実装後、develop最新でも改めて実機A/Bを行い、config手動追加なしで
   固着が解消することを確認する。

## 関連

BUG-142（本ADRが解消を目指す「IME ON固着」）、ADR-121（物理IME訂正キー
no-op時の冪等再送、`VK_DBE_HIRAGANA`専用のため本ADRの対象外）、
ADR-153（`explicit_ime_action_target`、無変換/変換の明示config設計）、
ADR-172（TsfNative ON方向救済4系統の整理）、ADR-173
（`solo_tap_ime_action_apps`、却下された代替案）、ADR-174
（パススルー+belief再観測、Blocker未解消のまま保留）。
