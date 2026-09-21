---
id: ADR-192
title: |-
  状態依存のIMEモードキーを検出して警告し、awaseの明示config（冪等なON/OFF）への置き換えを案内する
summary: |-
  IMEを状態の正とし、awaseは書き込まず観測に追随する方針（ADR-191）では、キーの結果が「入力中か・変換中か」などの状態で変わるキー
  （ATOKの無変換/変換など）を使うユーザーだけが、モードずれ（EngineがONのままIMEはOFF、等）を受ける。冪等なキー（`VK_IME_ON`/`VK_IME_OFF`）ではずれない。
  特に観測できないアプリ（TsfNative）ではずれが残るが、awaseのIMトグル書き込み（ADR-189）と強制ON/OFFの打鍵（`keys.ime_on`/`keys.ime_off`、既定Ctrl+変換/Ctrl+無変換、上書き可）で強制的に解消でき、
  ベストエフォート・ユーザー責任で足りる（ユーザー判断、2026-09-21）。本ADRは、そのユーザーを手助けする層を定める:
  (1)IMEのキーマップ（GJIは`config1.db`、MS-IMEはレジストリ）から、状態依存のキーを機械的に検出する。(2)検出したら一度だけ警告し、「冪等なキーへの変更」を推奨する。
  (3)置き換えは新機構を作らず、既存のユーザー明示config（`keys.ime_on/ime_off/ime_toggle`、`*_solo_tap_ime_action`）をawase-settingsで案内・設定する形にする。
  (4)`[[keymap]]`（ADR-114）は親指キー・IME制御VKを扱えないので使わない。GJIの`config1.db`の書き換えはしない。
status: |-
  **草案（2026-09-21、未レビュー）。** 実装前にopus-adversarial-consultで収束させる。ADR-191から分離した（ユーザー指示）。
related_adr:
  - "ADR-092"
  - "ADR-153"
  - "ADR-176"
  - "ADR-189"
  - "ADR-191"
---

# ADR-192: 状態依存のIMEモードキーを検出して警告し、明示config（冪等なON/OFF）への置き換えを案内する

## 背景

[ADR-191](191-ime-is-source-of-truth-observe-not-write.md)は、awaseがIMEの状態を書かず、生キーを通して観測に追随する方針をとる。ユーザーの整理（2026-09-21）:
- **冪等なキーではモードずれは起きない**。ずれるのは、結果が状態（入力中・変換中・IME ON/OFF）で変わるキーを使うユーザーだけ。
- 観測できないアプリ（TsfNative）ではずれが残るが、(a)IMトグルはawaseが書く（ADR-189）、(b)awaseが強制的にactuateする強制ON/OFFの打鍵（`keys.ime_on`/`keys.ime_off`。既定がCtrl+変換/Ctrl+無変換というだけで、configで上書きしたキーがそのまま使われる）がある、の2つでずれは強制的に解消できる。
  よって**状態依存のキーを使うユーザーは自己責任・ベストエフォート**で足りる。
- そのうえで、ユーザーを手助けする現実的な手段（警告・キーの抑止・上書き）を用意したい。

## 既存の資産（コードで確認）

- **ユーザー明示config**（ADR-153）: `[keys] ime_on`/`ime_off`/`ime_toggle`（キーコンボ）と、`muhenkan_solo_tap_ime_action`/`henkan_solo_tap_ime_action`（親指キーの単独タップ、`TurnOn`/`TurnOff`/`Toggle`）。
  awaseが物理キーを消費し、冪等な`VK_IME_ON`/`VK_IME_OFF`（Toggleはbelief基づき）で書く。これが「上書き」と「抑止」を兼ねる。
- **警告の仕組み**: MS-IMEのキー割り当て競合の警告ポップアップ（`msime_key_assignment::check_and_warn`、同一内容につき一度、内容が変われば再警告）。
- **キーマップの読み取り**: GJIは`config1.db`（`awase-gji-config`）、MS-IMEはレジストリ。
- 使えない・使わないもの: `[[keymap]]`（ADR-114）は`from`/`to`に親指キー・IME制御系VK・Alt/Win系を指定できない（ADR-114決定5）。ADR-110（汎用キーリマップ）は撤回済み。GJIの`config1.db`書き換えは
  ADR-143〜146で複雑さのため保留になっており、IMEの再起動や既存設定との衝突があるため本ADRでも採らない。

## 決定

### 決定1: 状態依存のキーを、キーマップから機械的に検出する

- **GJI**: `config1.db`の`session_keymap`（プリセット）・`custom_keymap_table`・`overlay_keymaps`と、Mozcの公開キーマップ（`atok.tsv`/`ms-ime.tsv`/`kotoeri.tsv`）を展開して、`(状態, キー, コマンド)`を得る。
  **状態はMozcのstatus（DirectInput/Precomposition/Composition/Conversion/Suggestion/Prediction）**。
- **判定**: あるキーが、状態によって異なるコマンドに割り当てられている（例: ATOKの変換は、DirectInputでIMEOn・PrecompositionでCancelAndIMEOff・CompositionでConvert）、または`ToggleAlphanumericMode`のような
  トグル系のコマンドを持つ場合、**状態依存**とする。冪等なキー（`IMEOn`だけ、`IMEOff`だけを全状態に割り当て）は対象外。
- **保守側**: ADR-191の「状態完備」の教訓（2状態だけ見ると誤判定する）に従い、定義済みの全statusを見る。未割当の状態がある場合は、その扱い（何も起きない）も状態依存の一種として数える。
- **MS-IME（実際のMicrosoft IME）**: 公開キーマップが無く、レジストリのキー割り当て設定（`KeyAssignment*`）の範囲でしか判定できない。判定できないキーは警告の対象にしない（誤警告を避ける）。

### 決定2: 検出したら一度だけ警告し、冪等なキーへの変更を推奨する

- 起動時・設定リロード時・IME種別の確定時（`sync_ime_kind_from_observation`の合流点）に検出し、**同一内容につき一度**、内容が変われば再警告する（既存の警告ダイアログの規約に合わせる）。
- 警告の文言: どのキー（例: 変換キー）が、どの状態でどう変わるか（例: 入力中は変換、それ以外はIME ON/OFF）。観測できないアプリ（Chrome等）でモードずれが起きうること、
  冪等なキー（`VK_IME_ON`/`VK_IME_OFF`）への割り当て変更を推奨すること、awaseの明示config（決定3）で置き換えられること。
- 警告は**ブロックしない**（無視できる）。ユーザーが「警告しない」を選べるようにする（設定`warn_state_dependent_mode_keys`、既定on）。

### 決定3: 置き換えは新機構を作らず、既存の明示configをawase-settingsで案内・設定する

- 案内する設定（既存）: `keys.ime_on`/`keys.ime_off`/`keys.ime_toggle`（キーコンボ）、親指キーが状態依存のキーなら`muhenkan_solo_tap_ime_action`/`henkan_solo_tap_ime_action`。
  推奨プリセット例: 変換 = IME ON（冪等）、無変換 = IME OFF（冪等）。親指シフトのキーとして使うユーザーには、単独タップの明示config（親指キーの単独タップだけが対象で、チョード判定はそのまま）。
- awase-settingsは、検出結果を表示し、「このキーを冪等なIME ON/OFFに置き換える」を1操作で`config.toml`へ書く（プレビューと元に戻す操作つき）。
- **キー自体の抑止**: 明示configはawaseが物理キーを消費する（生キーがIMEへ届かない）ので、置き換えたキーは自動的に抑止される。抑止だけを望む場合（キーを無効にする）は、`to`を空にする既存の記法があるかを確認し、
  無ければ「何もしない」割り当てを明示configの拡張として検討する（未決）。

### 決定4: やらないこと

- GJIの`config1.db`・MS-IMEのレジストリの書き換え（ADR-143〜146で保留、IMEの再起動・既存設定との衝突）。
- `[[keymap]]`（ADR-114）の制限の緩和（親指キー・IME制御VKを許すと、同時打鍵判定・IME制御との衝突が再燃する。ADR-114決定5）。
- 状態依存のキーの自動置き換え（ユーザーの明示操作なしにキーの意味を変えない）。

## 未解決・リスク

- **誤検出**: Mozcのキーマップの解釈（VK→Mozcキー名の写像、`Kana`と`Hiragana`が別バインド、カスタムキーマップの`custom_keymap_table`と`session_keymap`の食い違い（BUG-143））で、
  冪等なキーを状態依存と誤警告する恐れがある。検出は保守的にし、判定の根拠（どの状態にどのコマンドか）を警告に載せる。
- **親指キーとの衝突**: 無変換/変換をNICOLAの親指キーとして使う場合、`keys.ime_on = ["変換"]`はチョード判定と衝突する。親指キーの単独タップは`*_solo_tap_ime_action`で扱う（ADR-153）。
  親指キーでないユーザーだけ`keys.ime_on/off`を案内する、という分け方が要る。
- **警告疲れ**: 状態依存のキーを意図して使うユーザー（入力中は変換、確定後はIME ON/OFF）にとって、警告は不要。「警告しない」の設定と、警告の文言を短くすることで緩和する。
- **MS-IMEの判定精度**: レジストリで判定できる範囲が狭い。判定できないキーは警告しない（見逃しを許容）。
- **ADR-191との依存**: 状態依存のキーが多くなるほど、ADR-191の「観測に追随」の限界（観測できないアプリ）がユーザーに見える。本ADRは、その限界を緩和する層であって、ADR-191の前提ではない。

## 却下した代替案

- **自動置き換え**: 意図した状態依存の使い方を壊す。
- **`config1.db`の書き換え**: 上記のとおり保守的に退ける。
- **警告なしでベストエフォートのみ**: ユーザーがずれの原因（自分のキー割り当て）に気づけない。

## 検証計画

- GJI（ATOK・MS-IME模倣・カスタム）とMS-IMEの各キーマップで、検出結果を実キーマップのTSVと突き合わせる単体テスト（Linuxで走る）。
- 警告の一度きり・再警告・「警告しない」の動作の単体テスト。
- awase-settingsの1操作の置き換えで`config.toml`が期待どおり書かれ、元に戻せること（実機）。

## 関連

ADR-092（外部キーの意味づけ）、ADR-110（撤回）、ADR-111・114（`[[keymap]]`と制限）、ADR-143〜146（GJIキーマップ書き換え、保留）、ADR-153（明示config）、ADR-176（較正UI）、ADR-189（半角/全角のbelief基づく書き込み）、
ADR-191（本ADRの前提）、BUG-143（`session_keymap`と`custom_keymap_table`の食い違い）。
