---
id: ADR-176
title: |-
  IMEモードキー（変換/無変換/かな/漢字等）の実際の打鍵結果を受動観測し、
  shadow_action overrideを自己較正する
status: |-
  起票（2026-09-15、opus-adversarial-consult未実施）。BUG-143の修正
  （`classify_mode_key_ime_action`がGJIの`config1.db`を静的パースして
  Henkan/Muhenkanのshadow_action overrideを決める）を実装・実機確認した
  直後に、Mozc公式ソース調査で「`config1.db`の`session_keymap`と
  `custom_keymap_table`は食い違いうる（GUI実装のクリア漏れ）」という
  既知の限界が判明したことを受けて、静的パースに依存しない代替/補完
  経路として起票する。
related_adr:
  - "ADR-092"
  - "ADR-115"
  - "ADR-135"
  - "ADR-140"
  - "ADR-141"
  - "ADR-153"
  - "ADR-174"
  - "ADR-175"
---

# ADR-176: IMEモードキー（変換/無変換/かな/漢字等）の実際の打鍵結果を受動観測し、shadow_action overrideを自己較正する

## 背景

[ADR-174](174-solo-tap-passthrough-belief-reobservation.md)（BUG-143）で、
GJIの`config1.db`を静的パースして無変換/変換キーのIME意味論
（`ImeToggleKind::On/Off/Toggle`）を判定する`classify_mode_key_ime_action`
（`crates/awase-windows/src/gji_charset_autodetect.rs`）を修正した。
修正自体は実機で正しく動作することを確認済みだが、修正直後にMozc
公式ソース（`google/mozc`）を調査した結果、次の既知の限界が判明した
（詳細はBUG-143参照）:

- 公式エンジン（`session/keymap.cc::ApplyPrimarySessionKeymap`）は
  `session_keymap != CUSTOM`のとき`custom_keymap_table`を完全に無視する
  仕様であり、公式`ms-ime.tsv`も`DirectInput Henkan Reconvert`
  （IME開閉と無関係）——修正前の実装の方が公式仕様には忠実だった。
- 実機の食い違いは、GUI実装（`gui/config_dialog/config_dialog.cc::
  EditKeymap`）が「編集」確定時のみ`custom_keymap_table_`を更新し
  `session_keymap`をCUSTOMへ切り替える一方、**プルダウンだけを別
  プリセットへ戻す操作にはテーブルをクリアする処理が存在しない**ため、
  過去に一度カスタマイズした後でプリセットへ戻すと古いテーブルが
  残留しうる、という**GUI実装の抜け（公式ドキュメントに記載なし）**に
  起因すると推定される。

つまり`config1.db`の静的パースは、**Google非公開の内部フォーマット
（field番号は非公式知識）を解釈しているだけでなく、そのフォーマットが
実際のGJIバイナリの挙動を正確に表しているという保証も無い**——今回は
たまたま実機の挙動と`custom_keymap_table`の内容が一致したため修正は
有効だったが、一般には「設定ファイルの記述」と「実際の挙動」が食い違う
リスクを構造的に抱えている。

## 目的

`config1.db`の静的パースに頼らず、**ユーザーが実際にキーを打った結果
（IMEが実際にON/OFFどちらに動いたか）を受動的に観測して**、awaseの
`shadow_action` override（`henkan_shadow_override`/
`muhenkan_shadow_override`等、`kp_stage_shadow_ime_toggle`が消費する
既存フィールド群）を自己較正する経路を追加する。対象は無変換/変換に
限らず、GJI/MS-IMEのモード変更に関わりうるキー全般（下記「対象キー」
参照）に一般化する。

**位置づけ**: `config1.db`ベースの静的分類（ADR-092/135/141、BUG-115/
BUG-143）を置き換えるのではなく、**それが外れていた場合の自己修復
経路として補完する**。設定ファイルが読めない・信頼できない環境
（Google配布版がOSSと乖離している場合、`session_keymap`とテーブルの
食い違い、将来のconfig1.dbフォーマット変更等）でも、実際にユーザーが
そのキーを使った瞬間から正しく追従できるようにする。

## 対象キー（`vk.rs::ImeKeyKind`/`is_ime_mode_key_for_ime`を参照）

このリポジトリが既に「IMEモードに関わりうる」と分類しているVKの全体像:

| VK | 意味 | 既存の`ImeKeyKind::shadow_effect()` |
|---|---|---|
| `VK_KANA`(0x15) | かな | `TurnOn` |
| `VK_IME_ON`(0x16) | IME ON | `TurnOn` |
| `VK_JUNJA`(0x17) | 純ちゃ（IME ON系） | `TurnOn` |
| `VK_KANJI`(0x19) | 半角/全角 | `Toggle` |
| `VK_IME_OFF`(0x1A) | IME OFF | `TurnOff` |
| `VK_CONVERT`(0x1C) | 変換 | （`ModeKeyCandidate`経由、config1.db依存） |
| `VK_NONCONVERT`(0x1D) | 無変換 | （同上） |
| `VK_DBE_ALPHANUMERIC`(0xF0) | 英数 | `TurnOff` |
| `VK_DBE_KATAKANA`(0xF1) | カタカナ | `TurnOn` |
| `VK_DBE_HIRAGANA`(0xF2) | ひらがな | `TurnOn` |
| `VK_DBE_SBCSCHAR`(0xF3) | 半角 | `TurnOff` |
| `VK_DBE_DBCSCHAR`(0xF4) | 全角 | `TurnOn` |

上段（`VK_KANA`〜`VK_DBE_DBCSCHAR`）は`ImeKeyKind::shadow_effect()`が
**ハードコードされた固定方向**（Win32 API仕様上の意味論）で判定して
おり、これ自体は信頼できる（Microsoft公式ドキュメント準拠）。**本ADRが
対象とするのは、この固定方向の意味論だけでは決められないキー**——
`VK_CONVERT`/`VK_NONCONVERT`（GJIのキーマップ設定次第で意味が変わる、
ADR-174/BUG-143の対象）、および将来的に同種の不確実性を持ちうる
キー全般である。[ADR-175](175-physical-dbe-key-stuck-direction-recovery.md)
（BUG-142、物理半角/全角キーの固定方向マッピングをやめてToggleにした
事例）が示すとおり、**「Win32 API上の固定方向」を信じすぎることも
別の失敗モード**であり、実機の挙動を実際に観測して補正する仕組みは
`VK_DBE_SBCSCHAR`/`DBCSCHAR`のような「一見固定方向に見えるキー」にも
無関係ではない。

## 却下した代替案: 能動的なテストキー送信によるプロービング

「起動時やGJI検出時に、awase自身が対象キーを合成SendInputで送信し、
その結果を観測してキャリブレーションする」案を検討したが**却下**する。
ADR-153ケース3の実機履歴（`docs/known-bugs/BUG-113.md`・
`docs/known-bugs/BUG-124.md`）で、「生キーがGJIへ届くこと」
「awase自身が明示IME制御actuationを行うこと」のどちらか片方だけでも
「@」を誘発するのに十分と2回の独立した実機A/Bで確定している。合成
テストキー送信は両方を同時に満たす（awase起因のactuationで、かつ
モードキーがGJIへ届く）ため、高確率で「@」を再現すると判断した
（ADR-174本文の同種の却下判断も参照）。

**本ADRは、ユーザーが自発的に押した物理キーの結果だけを観測する
（awase自身は一切キーを送信しない）**。

## 決定（案、opus-adversarial-consult未実施）

### 設計の骨子

1. **トリガー**: 対象VKの物理（非注入）KeyDownで、かつ
   - `explicit_ime_action_consumed`が立っていない（ADR-153の明示config
     経路が既にこのキーを処理していない）
   - 現在NICOLA親指キー（`left_thumb_vk`/`right_thumb_vk`）として
     設定されていない（BUG-140対策と同じ除外——チョード1打鍵目を
     誤って「モードキー」として学習対象にしない）
   - `is_japanese_ime() == true`
   の全てを満たす場合にのみ、「打鍵前スナップショット」
   （`effective_open()`、`focus_epoch`/hwnd、打鍵時刻）を記録する。

2. **観測**: 打鍵前スナップショットを記録した直後は、既存の
   `kp_stage_idle_conv_check`のガード5（`is_ime_mode_key`）により
   その打鍵自身では読まない（ADR-174 round1〜3で確認済みの制約を
   継承）。**次の実キー入力**で自然に走る`kp_stage_idle_conv_check`
   →`classify_conv_transition`が生成する`ConvOpenInference`観測
   （BUG-26対策の既存経路、ADR-174 round1〜3で安全性を検証済み）を、
   直前のスナップショットと突き合わせる。

3. **相関判定（純粋関数として書く、要詳細設計）**: 「スナップショット
   記録から次の観測までの間に、他の説明要因（別の明示IME操作、
   フォーカス変更、GJI候補ウィンドウのflicker等、BUG-19が警告する
   偽陽性源）が挟まっていないこと」を条件に、
   `snapshot.effective_open == false && observation.open == true`
   なら「このキーはこのGJIセッションでは`TurnOn`として働く」と
   結論する（逆方向も同様）。**この相関判定こそが本ADRの中核であり、
   最も慎重な設計を要する部分**——ADR-174 round1〜3が「同じ
   `ConvOpenInference`をどう安全に消費するか」で3ラウンド失敗した
   経緯を踏まえ、今回は**今日のbelief（`effective_open()`/
   `IntentStore`）を一切書き換えない**設計にすることで、
   round1〜3のBlocker（B1: `IntentStore`による無効化、B2/B3: 逆方向
   actuation）を構造的に回避する。

4. **キャリブレーション結果の保存**: 結論が得られたら、
   `henkan_shadow_override`/`muhenkan_shadow_override`と同型の
   「学習済みoverride」フィールド（対象VKごと、または
   `ModeKeyCandidate`を一般化した型でVK→`ShadowImeAction`のマップ）を
   更新する。この経路は`ImeEvent`/`ImeModel`を一切経由しない
   （`.claude/rules/ime-belief-architecture.md`の三層分離規約の対象外
   ——belief本体ではなく`kp_stage_shadow_ime_toggle`が読む「今回の
   セッションでの静的知識」を更新するだけであり、既存の
   `henkan_shadow_override`と全く同じ性質のミュータブルキャッシュ）。
   以後の同じキーのタップは、`kp_stage_shadow_ime_toggle`の既存
   ディスパッチ（`intent_kind`→`write_physical_key`）にそのまま合流
   する——**新しいactuation合流点を作らない**（`fix-requires-evidence.md`
   の「IME actuation合流点」表に新規行を追加しない）。

5. **`config1.db`ベースの静的分類との優先順位**: 学習済みoverrideが
   存在すればそれを優先し、無ければ`config1.db`ベースの静的分類
   （ADR-092/135/141/BUG-143）へフォールバックする。**学習は
   `config1.db`を置き換えない**——設定ファイルが読めた場合は初期値
   として引き続き使い、実際の挙動と食い違ったときだけ学習結果で
   上書きされる、という関係にする。

6. **ライフサイクル**: `henkan_shadow_override`等と同じく、GJIの
   「アクティブ区間」ごとにリセットする（`gji_charset_streak_checked`
   と同じ考え方）。プロセス再起動や設定変更を跨いで永続化はしない
   （永続化すると、ユーザーがGJI設定を変えた後も古い学習結果が
   残り続けるリスクがあり、`config1.db`の「クリア漏れ」問題と同型の
   罠を自分で作ることになる）。

### 未解決・opus-adversarial-consultで詰めるべき点

1. **相関判定の偽陽性排除**: 「次の観測」までに何が起きたら
   キャリブレーションを諦めるべきか（フォーカス変更、別の明示IME
   操作、GJI候補ウィンドウのflicker、複数キーの連続タップ等）を
   網羅的に列挙する必要がある。ADR-174 round1〜3・BUG-19・BUG-51追補・
   BUG-55が積み上げてきた「`ConvOpenInference`を単独で信じてはいけない」
   という教訓と、本ADRの「学習の根拠として使う」という用途がどう
   両立するか（今日のbeliefを書き換えないから安全、で本当に十分か）。
2. **対象キーの範囲**: `VK_CONVERT`/`VK_NONCONVERT`だけに留めるか、
   上表の`VK_KANA`/`VK_JUNJA`等（Win32 API上は固定方向のはずのキー）
   まで対象を広げるべきか。広げる場合、「固定方向のはずの意味論と
   観測結果が食い違ったらどちらを信じるか」という新しい問題が生じる
   （ADR-175の教訓を踏まえると、Win32 API上の意味論を無条件に信じ
   きらない方が安全な場合もある）。
3. **NICOLA親指キー除外の徹底**: BUG-140が警告する「チョード1打鍵目を
   早まって解釈する」リスクを、`left_thumb_vk`/`right_thumb_vk`の
   除外だけで本当に塞げるか（設定変更のタイミング、`delegate_owned`
   との相互作用等）。
4. **学習結果の「取り消し」**: 一度`TurnOn`と学習した後、矛盾する
   観測（`TurnOff`方向）が来たらどうするか（即座に上書き、それとも
   複数回一致してから確定する多数決方式か）。前者は反応が速いが
   一発の偽陽性に弱く、後者は安全だが最初のキャリブレーションに
   要する打鍵数が増える。
5. **`fix-requires-evidence.md`対応**: この機構は「IME belief」
   「conv mode」「IME actuation合流点」の複数ファミリーに同時に
   触れる。回帰テスト（純粋関数化した相関判定ロジックのユニット
   テスト、Linux CIで実行可能なもの）を用意すること。
6. **`docs/known-bugs/BUG-143.md`が示す限界との関係**: 本ADRが
   実装されれば、BUG-143の「既知の限界」（`config1.db`が信頼できない
   ケース）は自己修復されるはずだが、それを実機でどう検証するか
   （わざと`session_keymap`とテーブルが食い違う設定を作って実機A/B
   する等）。

## 非スコープ

- 能動的なテストキー送信によるプロービング（上記「却下した代替案」）。
- `config1.db`静的パース自体の廃止（ADR-092/135/141/BUG-143の資産は
  引き続き初期値として使う）。
- 学習結果のプロセス再起動を跨いだ永続化。

## 関連

ADR-092（`classify_mode_key_ime_action`の起源）、ADR-115（打鍵列機能、
本ADRとは無関係だが同じ「1キーに複数の意味を持たせる」領域）、
ADR-135（Hiragana/Katakanaへの一般化）、ADR-140（probe/actuation競合、
相関判定のフェンシングで参考にすべき先例）、ADR-141（Henkan/Muhenkan
delegate、shadow_action override機構そのもの）、ADR-153（明示config、
本ADRのトリガー条件が除外すべき既存経路）、ADR-174/BUG-143（本ADRの
直接の動機）、ADR-175（BUG-142、「Win32 API上の固定方向を信じすぎる」
別の失敗モードの先例）。
