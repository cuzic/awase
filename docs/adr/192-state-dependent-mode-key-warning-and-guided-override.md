---
id: ADR-192
title: |-
  状態依存のIMEモードキーを検出して警告し、awaseの明示config（冪等なON/OFF）への置き換えを案内する
summary: |-
  IMEを状態の正とし、awaseは書き込まず観測に追随する方針（ADR-191）では、キーの結果が「入力中か・変換中か」などの状態で変わるキー
  （ATOKの無変換/変換など）を使うユーザーだけが、モードずれ（EngineがONのままIMEはOFF、等）を受ける。冪等なキー（`VK_IME_ON`/`VK_IME_OFF`）ではずれない。
  特に観測できないアプリ（TsfNative）ではずれが残るが、awaseのIMトグル書き込み（ADR-189）と強制ON/OFFの打鍵（`keys.ime_on`/`keys.ime_off`、既定Ctrl+変換/Ctrl+無変換、上書き可）で、開閉軸のずれは強制的に解消でき、
  ベストエフォート・ユーザー責任で足りる（ユーザー判断、2026-09-21）。本ADRは、そのユーザーを手助けする層を定める:
  (1)状態依存のキーを、新規の解釈器ではなく既存の`key_effect_predictor.rs`/`key_effect_table.rs`（ADR-191/195）
  への問い合わせとして、対象VKを限定した上で開閉軸(4仮説適合)と未確定文字列の行方の2軸で機械的に
  検出する（rev4）。
  (2)検出したら一度だけ警告し、「冪等なキーへの変更」を推奨する——ただし親指キー用途では、既存の
  `msime_key_assignment::conflict_warning`と逆方向の指示にならないよう分岐する（rev2）。
  (3)置き換えは新機構を作らず、既存のユーザー明示config（`keys.ime_on/ime_off/ime_toggle`、`*_solo_tap_ime_action`）をawase-settingsで案内・設定する形にする。
  (3b)親指キー単体への強制ON/OFFは、`*_solo_tap_ime_action`への正規化ではなく（rev1案はOFF/Toggle方向を
  埋められないとround1で判明）、`resolve_pending_thumb_as_single`に新設する専用の合流点でKeyUp解決する
  (rev2、round1のC-1案)。
  (4)`[[keymap]]`（ADR-114）は親指キー・IME制御VKを扱えないので使わない。GJIの`config1.db`の書き換えはしない。
status: |-
  **草案rev4（2026-09-22、opus-adversarial-consult round3で「まだ未収束（Blocker4件。ただしround2の
  9項目中7項目は意図どおり）」と判定・訂正済み、round4レビュー待ち）。**
  ADR-191から分離した（ユーザー指示）。round1(実コード照合)→rev2→round2(実測セル照合)→rev3→round3
  (`key_effect_table.rs`全448セルを機械的に検証)と反復し、rev3が新規導入した判定式・優先順位もそれぞれ
  誤りを含んでいたため、rev4でさらに訂正した。主な事実:
  (a) 決定3bの新しい入力の優先順位は、rev3が提案した「優先順位0.5」が実コード順（`explicit_action_
  consumed`/`suppress_solo_output`は`dedicated_fn_key`より後）や「専用Fnキー優先」と矛盾していた
  （round3 B-1）。さらに二重actuation対策として当てにしていた`explicit_action_consumed`は、
  ケース2(`PromoteToOn`)ではそもそも立たないため防げないと判明（round3 B-2、新規発見）。優先順位を
  `dedicated_fn_key`の直後(1.5)に確定し、既存ガードは動かさず新入力側で自前確認する形にし、ケース2との
  衝突はconfig検証での排他に切り替えた。
  (b) 決定1の判定式(rev3)は、対象VKの範囲を決めないまま`Enter`/`Esc`/`Bs`まで警告対象にしてしまい
  （round3 A-2）、かつ検算表がMSIMEプリセットの`Henkan`/`Muhenkan`（`Identity`が正、rev3は`Set(true)`
  と誤記）と`MSIME_NATIVE`の3キー（`ImeOff`/`HankakuZenkaku`/`Eisu`が実は(A)で状態依存になり、しかも
  `ImeOff`は決定2自身の推奨先）を見落としていた（round3 A-1）。対象VKをIMEモード/制御キーに限定し、
  `MSIME_NATIVE`は試行数不足のためプリセット単位でCannotPredictとする決定を追加して収束させた
  （round3 C-1）。awase自身の固定トグルセット〈`HankakuZenkaku`〉がMS-IME本体で状態依存に見える実測は
  本ADRのスコープ外の発見として記録した。
  (c) 決定2の「親指キーだが無変換/変換でないVK」の空白（T-16警告が埋める）、未解決節・summaryの
  rev3残骸も訂正した。
  決定1〜3・3bはround4レビュー待ち（未収束のため実装は着手しない）。
related_adr:
  - "ADR-092"
  - "ADR-153"
  - "ADR-176"
  - "ADR-186"
  - "ADR-189"
  - "ADR-191"
  - "ADR-195"
---

# ADR-192: 状態依存のIMEモードキーを検出して警告し、明示config（冪等なON/OFF）への置き換えを案内する

## 背景

[ADR-191](191-ime-is-source-of-truth-observe-not-write.md)は、awaseがIMEの状態を書かず、生キーを通して観測に追随する方針をとる。ユーザーの整理（2026-09-21）:
- **冪等なキーではモードずれは起きない**。ずれるのは、結果が状態（入力中・変換中・IME ON/OFF）で変わるキーを使うユーザーだけ。
- 観測できないアプリ（TsfNative）ではずれが残るが、(a)IMトグルはawaseが書く（ADR-189）、(b)awaseが強制的にactuateする強制ON/OFFの打鍵（`keys.ime_on`/`keys.ime_off`。既定がCtrl+変換/Ctrl+無変換というだけで、configで上書きしたキーがそのまま使われる）がある、の2つで、**開閉軸のずれ**は強制的に解消できる（かな/英数軸の回復経路は無い。ADR-191のTsfNativeの節、opus round4 QM1）。
  よって**状態依存のキーを使うユーザーは自己責任・ベストエフォート**で足りる。
- そのうえで、ユーザーを手助けする現実的な手段（警告・キーの抑止・上書き）を用意したい。

## 既存の資産（コードで確認）

- **ユーザー明示config**（ADR-153）: `[keys] ime_on`/`ime_off`/`ime_toggle`（キーコンボ）と、`muhenkan_solo_tap_ime_action`/`henkan_solo_tap_ime_action`（親指キーの単独タップ、`TurnOn`/`TurnOff`/`Toggle`）。
  awaseが物理キーを消費し、冪等な`VK_IME_ON`/`VK_IME_OFF`（Toggleはbelief基づき）で書く。これが「上書き」と「抑止」を兼ねる。
- **警告の仕組み**: MS-IMEのキー割り当て競合の警告ポップアップ（`msime_key_assignment::check_and_warn`、`conflict_warning`。同一内容につき一度、内容が変われば再警告）。**親指キー用途では「その割り当てを解除してください」（IME側の割り当てをawaseに明け渡す）という、決定2が案内する「冪等なキーに変える」とは逆方向の指示を既に出している**（round1 E-1、決定2節で分岐を明記）。
- **キーマップの読み取り**: GJIは`config1.db`（`awase-gji-config`）、MS-IMEはレジストリ。
- **状態依存性の判定に使える既存資産（round1 D-1で発見。決定1はこれらの上に定義する、新規の解釈器は作らない）**:
  - `KeyEffectKeymap::from_config`（`key_effect_predictor.rs:461`）: `session_keymap`/`custom_keymap_table`/`overlay_keymaps`の読み取りそのもの。
  - `KeyEffectKeymap::for_msime_native`（同`:485`）: MS-IME本体の`KeyAssignment*`レジストリ読み取りそのもの。
  - `key_effect_table.rs`（`gen_key_effect_table.py`が実機格子から生成）: ATOK/MSIME/MSIME_NATIVE各プリセットの`(open, conv, stage, key) → (open', conv', Disposition)`実測セル表。**状態依存性の判定に必要な情報そのもの**（設定の読みではなく実測）。
  - `KeyEffectKeymap::predict`（同`:503-517`）: カスタム表がそのVKの行を持つ／overlayがある／MS-IME本体で再割り当てがあるとき`None`（予測しない）を返す——BUG-143型の食い違いに対する既存の構造的対処。
  - `classify_mode_key_ime_action`（`gji_charset_autodetect.rs`、544行、ADR-191後も現存）: `ImeToggleKind::{On,Off,Toggle}`をoverlay > custom > presetの優先順位で分類する既存の「状態依存性」の表現。`OVERLAY_HENKAN_MUHENKAN_TO_IME_ON_OFF`適用時は状態非依存（`Toggle`にならない）と既に結論づけ済み。
- 使えない・使わないもの: `[[keymap]]`（ADR-114）は`from`/`to`に親指キー・IME制御系VK・Alt/Win系を指定できない（ADR-114決定5）。ADR-110（汎用キーリマップ）は撤回済み。GJIの`config1.db`書き換えは
  ADR-143〜146で複雑さのため保留になっており、IMEの再起動や既存設定との衝突があるため本ADRでも採らない。

## 決定

### 決定1（rev4・対象VK範囲を確定、検算表をround3の機械検証に合わせて訂正）: 状態依存のキーを、既存の実測表・予測器の上で判定する

**新しい解釈器・新しいキーマップ展開ロジックは作らない**（round1 D-1: 同じ仕事をする実装が
develop に既に3つある）。決定1は次の既存資産への**問い合わせ**として定義する:

- **入力**: 対象VKについて、`key_effect_table.rs`が持つ実測セル（ATOK/MSIME/MSIME_NATIVEの各プリセット。
  `(open, conv, stage, key) → (open', conv', Disposition)`）を直接横断する。**`KeyEffectKeymap::predict`
  はここでは使わない**——`predict`は実行時向けの補正（`mode == Unknown`のとき既定のひらがなを種にする、
  `is_char_vk`の特別扱い等、`key_effect_predictor.rs:302`）を含むため、検出（合成状態の掃引）にそのまま
  使うとこの補正が判定に混ざる（round1 D-2）。判定はセル表を直接横断する専用の集計関数として実装する
  （置き場所は下記「実装配置」）。

**判定は開閉軸と未確定文字列の行方の2軸を独立に見る（round2 A-1・A-3の指摘を受けて再定義。旧rev2の
「常に純トグルでなければ状態依存」という主節は誤りだったため撤回する——この定義だと決定2が推奨する
当の冪等キー`ImeOn`/`ImeOff`自身が「状態依存」と誤判定される。round2 A-1が実測セルで確認済み）**:

- **対象VKの範囲（round3 D-1、Major対応。決定1(A)(B)を回すVKを先に定義する）**: `key_effect_table.rs`は
  `Bs`/`Enter`/`Esc`/`Space`/`Eisu`/`Hiragana`/`Katakana`等13キーぶんの実測を持つが、決定1(A)(B)が
  実際に**判定を回す**のは次のIMEモード/制御キーだけに限る: `Henkan`(0x1C)・`Muhenkan`(0x1D)・
  `HankakuZenkaku`(0xF3/0xF4)・`Kanji`(0x19)・`ImeOn`(0x16)・`ImeOff`(0x1A)。`Enter`/`Esc`/`Bs`/`Space`
  はIME制御キーではないため対象外（round3 A-2が指摘した過検出——これらはIdentity固定だが、仮に
  Identity以外だったとしても対象VKの範囲外なので判定自体を回さない、という二重の除外にする）。
  `Eisu`/`Hiragana`/`Katakana`（入力モードキー、awaseは追随するだけで書かない）も、たとえ状態依存や
  破壊的Dispositionと判定されても「冪等なキーへの変更」という決定2の推奨が意味を持たないため、決定1の
  判定対象には含めるが**決定2はこれらに警告を出さない**（下記）。
- **(A) 開閉軸の状態依存性（モードずれの原因）**: 対象VKについて、到達可能な全セルの`(open, open_after)`
  の組を集め、次の4つの候補仮説のうち**矛盾なく一致するものが存在するか**を調べる: `Set(true)`（常に
  `open_after=true`）／`Set(false)`（常に`open_after=false`）／`Toggle`（常に`open_after = !open`）／
  `Identity`（常に`open_after == open`）。**いずれか1つの仮説と矛盾しないセルの組み合わせなら
  非状態依存**、どの仮説とも矛盾する（＝同じ`open`値なのに`open_after`が割れる、またはToggleの予測と
  食い違う）なら**状態依存（Aで警告）**とする。
  - **検算（round3が`key_effect_table.rs`全448セルを機械的に検証した結果。round2の検算表は2箇所で
    実データと不一致だったため、この表に置き換える）**:

    | プリセット | キー | 到達可能セルの`(open→open_after)` | 一致する仮説 | (A)判定 |
    |---|---|---|---|---|
    | ATOK | `ImeOn` | `false→true`、`true→true` | `Set(true)` | 非状態依存 |
    | ATOK | `ImeOff` | `false→false`、`true→false` | `Set(false)` | 非状態依存 |
    | ATOK | `Kanji` | `false→true`、`true→false` | `Toggle` | 非状態依存 |
    | ATOK | `HankakuZenkaku` | `false→true`、`true→false`（全stage） | `Toggle` | 非状態依存 |
    | ATOK | `Henkan`/`Muhenkan` | Stage::None: `false→true`・`true→false`（Toggle）／Stage::Typing・Conv*: `true→true`（**Toggleと矛盾**） | 一致する仮説なし | **状態依存** |
    | MSIMEプリセット | `ImeOn` | `false→true`、`true→true` | `Set(true)` | 非状態依存 |
    | MSIMEプリセット | `ImeOff` | `false→false`、`true→false` | `Set(false)` | 非状態依存 |
    | MSIMEプリセット | `Kanji` | `false→true`、`true→false` | `Toggle` | 非状態依存 |
    | MSIMEプリセット | `HankakuZenkaku` | `false→true`、`true→false`（全stage） | `Toggle` | 非状態依存 |
    | MSIMEプリセット | `Henkan`/`Muhenkan` | `false→false`、`true→true`（全stage） | `Identity` | 非状態依存 |
    | `MSIME_NATIVE` | 全対象キー | （下記「MS-IME本体」参照。試行数不足のため判定を回さない） | — | CannotPredict |

    → **状態依存として(A)で警告されるのは、実測データの範囲ではATOKの`Henkan`/`Muhenkan`だけ**
    （round1 D-2が「実際にずれるのはここ」と指摘した通り）。`MSIME_NATIVE`を対象VK範囲に含めた
    まま試行数不足でCannotPredictに倒す（下記「MS-IME本体」）ことで、round3 A-1(2)が発見した
    「`MSIME_NATIVE`では`ImeOff`/`HankakuZenkaku`/`Eisu`も(A)で状態依存になる」という**実測はあるが
    信頼できない**結果を警告に混ぜない。**この除外の理由は「状態依存でないから」ではなく
    「実測の信頼度が足りないから」であることをここに明記する**（round3 C-1）。
- **(B) 未確定文字列の行方の危険性（別カテゴリの警告）**: 対象VKのうち、**(A)が`Identity`ではない**
  もの（round3 A-2、Blocker対応——`Enter`/`Esc`/`Bs`のような開閉に無関係な正常動作キーを誤って
  警告してしまう過検出を、対象VK範囲の限定と合わせてここでも防ぐ）について、到達可能な全セルの
  `Disposition`を集め、`Discarded`（破棄）または`Committed`（意図せず確定）が**一部のセルにだけ**
  現れる場合に警告対象とする。
  - **検算**: ATOK/MSIMEプリセット共通で`ImeOff`・`HankakuZenkaku`・`Kanji`が該当する（Stage::Noneでは
    `Disp::None`、Typing/Conv*では`Discarded`〈ATOK〉または`Committed`〈MSIMEプリセット〉）。`ImeOn`は
    Typing/Conv*でも`Disp::Kept`（保持、破壊的でない）で一貫するため非対象。
  - **文言はAとBで分ける**（決定2）: (A)は「モードがずれる可能性があるので冪等なキーに変更してください」、
    (B)は「入力中に押すと変換中の文字が消える/確定してしまう場合があります（冪等なキーでも起こりうる、
    置き換えでは解決しません）」という別の推奨にする。
  - **この分類は警告判定のみに使い、actuationの可否判断には使わない**（round3 A-3、Minor対応。書いて
    よいかの線引きはADR-191決定1の分類(a)〜(e)がSSOTのまま）。
- **沈黙／伝える条件**: 次の(i)(ii)を分けて扱う。
  - (i) **キーマップの解釈自体が不確か**（カスタムキーマップと`session_keymap`の食い違い〈BUG-143〉、
    Mozcトークン未対応、ATOKプリセットでの古い`custom_keymap_table`〈ADR-186決定(c)、コミット
    `bff621b5`が「読まない」と決めた対象〉、**overlay適用構成**〈round3 D-3、Major対応:
    `KeyEffectKeymap::predict`がoverlay適用時に`None`を返す既存挙動に揃え、overlay構成は本ADRでも
    実測が無いためCannotPredictとする。「格子を取り直す」案は較正キャンペーンの再実施を要するため
    本ADRのスコープ外とし採らない）） → **沈黙**（誤診断回避、round1 D-3/D-4の趣旨）。
  - (ii) **解釈はできるが実測セルが無い、またはカスタム表でそのVKの行が上書きされている**
    （`KeyEffectKeymap::predict`が`None`を返す構成に相当。BUG-143の実機——`session_keymap=MSIME(2)`
    のまま175行のカスタム表に`DirectInput Henkan IMEOn`を持っていた構成——はここに入る）→
    **「awaseは追随できない」ことを明示する第3の警告カテゴリ**を出す（予測できるセルより予測できない
    セルの方がずれが確定しやすいので、沈黙ではなく伝える）。
  - 判定の型は`enum Classification { StateIndependent, StateDependent(Axis), CannotPredict }`のように
    `Axis`（開閉／未確定文字列）と「予測不能」を区別できる形にする（round1 E-3の「型で落とす」を
    3値以上に拡張）。
- **実装配置**: 判定関数は`awase-windows`側の`pub fn`として`key_effect_table.rs`のセル横断ロジック上に
  実装する（`key_effect_table`モジュール自体は`state/mod.rs:128`で`mod`＝privateだが、この判定関数を
  `pub`にすれば`awase-settings`から呼べる）。**実行時の`predict()`が持つ既定値補正・特殊扱いは、この
  判定関数には適用しない**（`predict`は`mode == Unknown`のとき既定のひらがなを種にする、`is_char_vk`の
  特別扱い等〈`key_effect_predictor.rs:302`〉の実行時向け補正を含むため、検出にそのまま使うとこの補正が
  判定に混ざる）。
- **MS-IME本体（rev4・試行数不足の扱いを決定として明記、round3 C-1対応）**: `key_effect_table.rs`は
  `key_effect_predictor.rs:114-125`が定義するセル構造体に試行数のフィールドを持たない（生の試行数は
  `key_effect_table.rs`冒頭の散文と生成元`grid-tables/*.json`にしかない）。`MSIME_NATIVE`はヘッダ注記
  「206/227セルが1試行のみ」の通り大半が単発観測であり、セル単位の信頼度を今のデータ構造から得られない。
  したがって本ADRでは**`MSIME_NATIVE`をプリセット単位でCannotPredictとして扱う**（セル単位の信頼度判定
  ロジックを新設しない。生成器`gen_key_effect_table.py`に試行数の列を足す拡張は、必要になった時点で
  別途起票する）。加えて`KeyEffectKeymap::for_msime_native`（`key_effect_predictor.rs:485`）がレジストリの
  再割り当てを検出した場合はそもそも`predict`が`None`を返すので、既定のキー配置・再割り当て後のいずれも
  結果はCannotPredictで一致する（旧rev2/rev3の「レジストリの範囲でしか判定できない」「実測セルを使う」
  という書き方はどちらも撤回する）。
  - **本ADRのスコープ外へ回す発見（round3 A-1(2)）**: `MSIME_NATIVE`の実測セル（試行数不足で警告には
    使わない）は、`HankakuZenkaku`（0xF3/0xF4）が(A)で状態依存になることを示している——これは
    `vk.rs::is_open_toggle_for`によりawase自身がbeliefトグルを書く固定セット（ADR-189/191決定1）に
    対する実測であり、この固定セットの前提（「常に純トグルとして扱ってよい」）がMS-IME本体では成立しない
    可能性を示唆する。**本ADRでは扱わない**（ユーザー向け「キーを変更してください」という案内では解決
    しない、awase自身の書き込み前提の健全性の問題のため）。ADR-189/191側での追試（試行数を増やした
    再測定）を促す記録として、未解決節にも残す。

### 決定2（rev4・空白の穴埋めを追加）: 検出したら一度だけ警告し、冪等なキーへの変更を推奨する（親指キー用途は既存警告に合流させる）

**round1 E-1（Blocker）**: `msime_key_assignment::conflict_warning`は、MS-IMEの無変換→OFF/変換→ON
割り当てが有効なとき、**親指シフトのユーザー向けに「その割り当てを解除してください」**と既に警告
している——決定2が推奨する「冪等なキーに変える」とは逆方向。したがって:

- **そのVKが親指シフトとして使われているかで分岐する。情報源は`general.left_thumb_key`/
  `right_thumb_key`（config、round2 D-3）とする**——`muhenkan_vk`/`henkan_vk`は無変換/変換限定の内部値
  であり、ユーザーが親指シフトに使っている物理キーの範囲（config上の設定）とは異なる。
  - 親指キーとして使っている場合: 決定2の新規警告は出さない。既存の`msime_key_assignment::
    check_and_warn`（GJI側にも同型の判定を拡張する。新しい独立ダイアログは追加しない）が
    「IME側の割り当てを解除し、awaseの明示config（`*_solo_tap_ime_action`、または決定3bの新経路）
    に委ねてください」を案内する。
  - 親指キーでない場合のみ、決定2の「冪等なキーへの変更」警告を出す。
  - **「親指キーだが無変換/変換ではない」VKの扱い（round3 D-4、Minor対応）**: 決定3bの新経路は
    無変換/変換限定（決定3b参照）なので、この構成では決定2の新規警告（親指キーとして扱う）も
    決定3bの救済も届かない空白になる。この場合はT-16警告（決定3b参照）が「他のキーに変更するか、
    Shiftなどと組み合わせて設定し直してください」を案内する経路のまま残ることを明記し、無案内には
    しない。
- **入力モードキー（`Eisu`/`Hiragana`/`Katakana`）には決定1の判定結果によらず警告を出さない**
  （round3 D-1: awaseは追随するだけで書かないキーに「冪等なキーへの変更」を勧めても意味がない）。
- 起動時・設定リロード時・IME種別の確定時（`sync_ime_kind_from_observation`の合流点）に検出。
  **同一内容につき一度**の判定キーは、`KeymapCache`が既に持つ`stamp`（`config1.db`のmtime+長さ）を
  流用する（round1 E-2）——**ただしこれはGJI専用**（round2 D-4）。MS-IME本体はレジストリなので
  `stamp`が無く、既存のpacked-bits方式（`msime_key_assignment.rs:159-163`）で同一性を判定する。
  内容が変われば再警告。この警告判定は`KeymapCache`の`checked_at_ms`等の予測器側キャッシュ状態を
  リセットしない（round2 D-4、予測とは独立の読み取り専用の参照）。
- **警告は決定1の(A)/(B)の軸ごとに文言を分ける（決定1参照）**: (A)開閉軸の状態依存には「モードが
  ずれる可能性、冪等なキーへの変更を推奨」、(B)未確定文字列の行方の危険には「入力中に押すと変換中の
  文字が消える/確定してしまう場合がある、冪等なキーでも起こりうるため置き換えでは解決しない」、
  (ii)予測不能（決定1）には「awaseはこのキーの効果を追随できない可能性がある」という3種の文言を
  用意する。
- 警告は**ブロックしない**（無視できる）。ユーザーが「警告しない」を選べるようにする（設定
  `warn_state_dependent_mode_keys`、既定on）。

### 決定3（rev2・整合追加）: 置き換えは新機構を作らず、既存の明示configをawase-settingsで案内・設定する

- 案内する設定（既存）: `keys.ime_on`/`keys.ime_off`/`keys.ime_toggle`（キーコンボ）、親指キーが状態依存のキーなら決定3b（改訂後）の経路。
  推奨プリセット例: 変換 = IME ON（冪等）、無変換 = IME OFF（冪等）。
- awase-settingsは、検出結果を表示し、「このキーを冪等なIME ON/OFFに置き換える」を1操作で`config.toml`へ書く（プレビューと元に戻す操作つき）。
- **round1 F-1（Major）**: 親指キー単体を対象にする場合、1操作の書き込みは`*_solo_tap_ime_action`
  だけでなく`*_solo_tap_always_suppress = true`（`ModeKeyConfig`、`awase-settings/src/main.rs:2339`が
  既に露出）も同時に揃える。揃えないと、`always_suppress = false`（ADR-153以前からの既定・legacy
  設定のユーザーが大半、`nicola_fsm.rs:2031-2035`）のユーザーでは書いた設定が無言で無効化される
  （M13）——撤去済みの自動設定支援機能と同型のユーザー混乱を再現しない。
- **キー自体の抑止**: 明示configはawaseが物理キーを消費する（生キーがIMEへ届かない）ので、置き換えたキーは自動的に抑止される。
  **round1 F-2**: 抑止だけを望む場合の記法は**既に存在する**——`*_solo_tap_always_suppress = true`
  （`ModeKeyConfig{idle: Suppress, composing: Suppress}`、`src/config.rs:424,429`が既定値）。
  新記法の検討は不要（旧rev1の「未決」を撤回）。

### 決定3b（rev4・優先順位とケース2との排他を確定、round1 C-1採用）: 親指キー単体への強制ON/OFFは、専用の新しい入力で単独打鍵確定時にactuateする

**round1が実コード（`nicola_fsm.rs:2106`の`resolve_pending_thumb_as_single`、`:2022`の
`resolve_explicit_ime_action`）で確認した事実**: 旧rev1が提案した「`*_solo_tap_ime_action`への
正規化」は、**ON方向×belief OFFしか意図を満たさない**。これは実は正規化と無関係に、KeyDown時点で
`key_pipeline.rs:1158`（`explicit_ime_action_target`）→`:1306-1320`（ケース2、`PromoteToOn`）が
既に処理している効果であり、**正規化しなくても既に動く**。決定3bが埋めると謳っていた
「エンジン活性中の単独打鍵でOFF/Toggle」の穴は、`*_solo_tap_ime_action`の既存制約
（composing中は無効／`always_suppress=false`〈Passthrough相当、legacyユーザーが大半〉のとき無効かつ
生VKがGJIへ抜ける／通常のタップ〈100ms超〉は`defers_solo_until_release`の対象外でタイマー経由の
`execute_from_loop`に落ちるため`Unwarranted`で握り潰される〈ADR-186が実機3/3 FAILと記録した経路と
同型〉）によって、**正規化では埋まらない**。

**改訂した設計**: `*_solo_tap_ime_action`への正規化はしない。代わりに、`resolve_pending_thumb_as_single`
（単独打鍵の確定点）に、**`*_solo_tap_ime_action`とは独立の新しい入力**を渡す:
「このVKが`keys.ime_on`/`ime_off`/`ime_toggle`にbareで（Shift等の修飾無しで）設定されているか」。
該当すれば、単独打鍵の確定と同時にIME開閉を要求する。

- **コンボ照合は消さず残す（加算）**: `keys.ime_on/off/toggle`のコンボ照合（`engine_active`のときだけ
  抑止・actuate）は、エンジン非活性時に単独打鍵FSM自体が動かないケースで唯一動く経路であり続ける
  （round1 B-2の3）。新しい合流点は、エンジン活性中の単独打鍵確定というコンボが拾えない場面を
  **足す**ものであり、既存経路を置き換えない。
- **解決タイミングはKeyUp**: `defers_solo_until_release`の対象にこの新しい入力を含め、通常のタップ
  （100ms超）でもKeyUp解決の`kp_stage_post_decision`経路を通す（`execute_from_loop`のタイマー解決に
  落とさない）。ADR-186が実測した「タイマー解決は`Unwarranted`で握り潰される」の再発を避けるため、
  ここは選択肢ではなく必須とする（round2 B-7が確認済み: KeyUpは実キーイベントとして
  `process_key_event`に入るため、この前提は成立する）。
- **新しい入力の優先順位上の位置（round3 B-1、Blocker対応——round2の「優先順位0.5」は内部矛盾のため撤回）**:
  実コードの順序は`modifier_key`（無条件no-op、先頭`:2117`）→`dedicated_fn_key`（早期return`:2134`）→
  `*_solo_tap_ime_action`（M13/composingフィルタ`:2147`）→`explicit_action_consumed`（打ち切り`:2175`）→
  `suppress_solo_output`（no-op`:2180`）→`ModeKeyConfig`（`:2183`）であり、**round2案の「3ガードの後・
  Fnキーの前」は実コード順（`explicit_action_consumed`/`suppress_solo_output`は`dedicated_fn_key`より
  後）と矛盾し、かつ「専用Fnキー優先」（Fnキーの前に置くと新入力が勝ってしまう）とも両立しない**。
  **確定した位置**: 新しい入力は`dedicated_fn_key`の**直後**・`*_solo_tap_ime_action`の**直前**
  （優先順位1.5）に置く。**既存チェック（`explicit_action_consumed`・`suppress_solo_output`）の位置は
  動かさない**——新入力の分岐**内で**この2つのフラグを自前で確認し、立っていれば発火しない（`modifier_key`
  は関数先頭のearly returnで既にカバーされるので新入力側での追加確認は不要）。これで「専用Fnキー優先」
  （新入力は`dedicated_fn_key`より後なので自然に成立）、composing/M13/専用Fnキーの3つだけを飛ばすこと、
  既存ガードの効果を残すこと、の3つが両立する。config検証は、専用Fnキーと本決定の対象が同一VKに重複
  設定された場合に警告する。
- **ケース2（`PromoteToOn`）との二重actuation（round3 B-2、Blocker対応——新規発見）**: round2は
  `explicit_action_consumed`の適用で二重actuationを防げると想定していたが、**本番でこのマーカーを
  立てるのは`key_pipeline.rs:1338`（ケース3改）と`:1232`（対応するKeyUp早期分岐）の2箇所だけで、
  ケース2（`PromoteToOn`、`:1308-1320`）はマーカーを立てない**。したがって`keys.ime_toggle`を親指キー
  単体に設定し、かつ同じVKに`*_solo_tap_ime_action = "toggle"`も設定しているユーザーでは、
  KeyDown側のケース2がbelief OFFをONへ昇格させたあと、KeyUp側の新入力がToggleを評価して
  再びOFFへ戻す（`engine.rs:541`の`Toggle => !ctx.ime_on`はKeyUp時点のbeliefで評価されるため）、
  1打鍵でON→OFFの往復が起き「効かない」ように見える回帰が起きる。**この衝突はケース2がWindows層の
  KeyDown処理、新入力がFSMのKeyUp処理という別の層・別の時刻で起きるため、優先順位表では防げない**。
  **対策として、同一VKに`keys.ime_on/off/toggle`（bare）と`*_solo_tap_ime_action`の両方を設定することを
  config検証で排他にする**（両方設定されていればエラーにするか、`*_solo_tap_ime_action`側を無効化して
  警告する。どちらの設定を優先するかは、本決定の新入力の方が明示的な意図〈このキーで強制ON/OFFしたい〉
  なので`*_solo_tap_ime_action`側を無効化する方を第一候補とする）。ケース2にもマーカーを立てる代替案は
  `runtime/transport.rs:286`が同じマーカーで生キーのSuppressも制御するため、ON方向の物理キー配送が
  変わる副作用があり（BUG-46/52/116の物理配送ファミリー）、採らない。
- **composing中の扱い（round2 B-3、Blocker対応で範囲を縮小）**: 決定3bが選べるのは**composing中に
  発火させるか否かだけ**であり、「未確定文字列を破棄する」という結果そのものは選べない——`Disposition`
  はプリセット依存で、ATOKでは`Discarded`（破棄）だがGJIのMS-IMEプリセットでは`Committed`（確定）
  になる（`key_effect_table.rs`実測）。さらにawaseの強制OFFは`VK_IME_OFF`送出とは限らず
  （`ime_controller.rs::characterize_strategy`がImmCross等の別戦略を選ぶ場合がある）、その場合の
  未確定文字列の行方は実測表の対象外（実測は「そのキーを送ったときの結果」であり、awase自身の別戦略の
  結果ではない）。したがって本決定では「composing中も発火させる」を選び、その結果（破棄されるか確定
  されるかはIME実装依存で一定しない）はユーザーに周知する側に倒す（決定2の警告文言に含める）。
- **対象VKの範囲（round2 B-4、Major対応）**: 新しい入力は`muhenkan_vk`/`henkan_vk`（無変換/変換限定、
  `thumb_solo_special_handling`が扱える範囲）に限る。無変換/変換以外を親指キーに設定しているユーザーの
  同キーへの`keys.ime_off`等は対象外のままとし（コンボ側は`is_bare_thumb`の判定でエンジン活性中は
  抑止されたまま動かない）、T-16警告（下記）でその旨を案内する。
- **T-16警告文の更新（round2 B-5、Majorをスコープに追加）**: `src/config.rs:1098-1143`
  （`validate_thumb_key_in_ime_combos`）の既存警告文（「他のキーに変更するか、Shiftなどと組み合わせて
  設定し直してください」）は、無変換/変換への本決定の実装後は事実と逆になる。無変換/変換に対する
  文面を「単独タップ確定時に強制ON/OFFが発火します」に書き換え、それ以外のVKに対する文面（Shift推奨）
  は残す。`msime_key_assignment.rs`側の案内文（既存の資産節参照）も、bare親指が使えるようになった旨を
  反映する。
- **`*_solo_tap_ime_action`の既存制約はこの新しい入力には適用しない**——それらは「IMEがONの間はGJI
  自身のかな切替に委ねたい」という別の意図（`nicola_fsm.rs:2037-2044`）のための制約であり、「この
  キーで強制ON/OFFしたい」という本決定の意図とは排他だからである（round1 C-1の理由）。
- **`defers_solo_until_release`は2つの独立した理由を持つことになる（round2 B-6、Minor対応）**:
  既存の理由（ADR-182決定1c、`Passthrough`で生VKを出す親指をsolo tapとshiftの1打鍵内二重使用から守る）
  に加え、本決定は「belief書き込み経路〈KeyUp解決〉を通すため」という別の理由でこの述語に条件を足す。
  両方の理由をdocに明記する——`nicola_fsm.rs:858-867`のdoc矛盾（round1 A-2）と同型の事故（片方の理由
  だけを見て述語を削る）を防ぐため。
- **actuation合流点の登録先（round2 B-2、Majorで訂正——round1/rev2の「7つ目」は撤回）**: 本決定の新経路
  は`apply_ime_open_with_view`を直接呼ばない（`ime_open_requested`→Engine→`Decision::SetOpen`→
  `dispatch_ime_set_open`という既存の合流点を通る）。したがって`.claude/rules/fix-requires-evidence.md`
  の「IME actuation合流点」表（`d8076516`で3項目に整理済み）に7つ目を足すのではなく、同ファイルの
  「**キー選択（IME ON/OFFに送るVK）**」行が列挙する`resolve_pending_thumb_as_single`の優先順位
  （`dedicated_fn_key`/`*_solo_tap_ime_action`/`ModeKeyConfig`）に**4つ目として本決定の新しい入力を
  追記する**。ADR-191決定5の指標5（IMEへ書く振る舞いの数）への計上は妥当なので残す。
- **eisu救済との対称配線（round2 B-7、必須の見落とし）**: 本決定の新経路は「user IME-ON経路」に
  当たるため、`.claude/rules/ime-belief-architecture.md`が定める「user IME-ON経路とObservedEisu救済の
  対称性」に従い、`state/eisu_recovery.rs::eisu_reset_on_ime_on`と対で配線する。
  `tests/architecture_guard.rs::user_ime_on_paths_are_paired_with_eisu_reset`がこの対称性を監視して
  いるので、実装時にこのガードへ新経路を追加し、検証計画にも明記する。
- **前提コードのdoc矛盾（round1 A-2、3bの実装前に別途解消が必要）**: `nicola_fsm.rs:858-864`は
  「明示configを持つキーもKeyUp解決の対象にする（ADR-186）」と書くが、`:867`の除外リストと実際の
  コード（`:879`）は明示config持ちを除外している。ADR-186本文（`docs/adr/186-...md:117-126`）も
  delegate（ADR-191が撤去済み）についてしか述べておらず、**`:858-864`はADR-191のdelegate撤去で
  陳腐化した記述**と判断する。本ADRの実装に先立って、このdocコメントを「明示config持ちは
  `resolve_explicit_ime_action`〈タイマー解決〉のまま、本ADRの新しい合流点だけがKeyUp解決を持つ」
  と訂正する（別コミットでよいが、3bの実装着手前に完了させる）。
- 遅延: 確定は同時打鍵しきい値ぶん遅れる。`ime_toggle`の親指キー単体も対象に含める（`"toggle"`。
  awaseが書くのはbeliefに基づく開閉のトグルだけで、ADR-191の線引きの内側）。入力モードキー
  （ひらがな・カタカナ・英数）は、親指キーに割り当てられていても対象にしない。
- **実装形の見積もり（round2 C節、複雑さの申告）**: コア（`awase`クレート）はOS非依存（ADR-019）なので、
  `NicolaFsm`へはキーコンボそのものではなく事前分類済みの値を渡す。実装形は具体的には
  「`ThumbSoloSpecialHandling`に2つ目の`Option<ShadowImeAction>`フィールド（例: `forced_open_action`）
  を追加し、優先順位表に`dedicated_fn_key`直後・`*_solo_tap_ime_action`直前の1.5番目の行を作り
  （`explicit_action_consumed`/`suppress_solo_output`はこの行の分岐内で自前確認する）、Platform側
  （config読み込み）でこのフィールドへの配線とconfig検証の排他ルール（`*_solo_tap_ime_action`との
  併設を弾く）を1本ずつ追加する」という形になる。`*_solo_tap_ime_action`と型・経路は同じでゲートだけが
  違う構成であり、「新機構は作らない」という言葉から想像されるより小さくない（C-2却下理由がこの差を
  正しく捉えているため判断自体は変えない。規模を正直に書く）。
- **却下した代替（round1 C-2, C-3）**: (C-2) 正規化を「追加のみ」にしてコンボを残す案は、
  `*_solo_tap_always_suppress`の整合やKeyUp解決変更が結局必要になり、専用の新しい合流点（C-1）と
  比べてコストが変わらないうえ、`*_solo_tap_ime_action`の意味を2つの意図で共有し続ける複雑さが残る
  ため採らない。(C-3) bare親指キーを`keys.ime_*`で非対応にしShift+親指へ誘導する案は、コード変更が
  最小だが、ユーザーの明示的な要望（親指キーそのものを強制ON/OFFにしたい）に応えられず、
  「Shift+親指では足りない理由」を積極的に主張する材料も無いため見送る。

### 決定4: やらないこと

- GJIの`config1.db`・MS-IMEのレジストリの書き換え（ADR-143〜146で保留、IMEの再起動・既存設定との衝突）。
- `[[keymap]]`（ADR-114）の制限の緩和（親指キー・IME制御VKを許すと、同時打鍵判定・IME制御との衝突が再燃する。ADR-114決定5）。
- 状態依存のキーの自動置き換え（ユーザーの明示操作なしにキーの意味を変えない）。

## 未解決・リスク

- **誤検出（rev1から残存、緩和策あり）**: Mozc/GJIキーマップの解釈（VK→キー名の写像、カスタムキーマップの
  `custom_keymap_table`と`session_keymap`の食い違い〈BUG-143〉）で、冪等なキーを状態依存と誤警告する
  恐れは残る。決定1の判定（rev4、対象VKを限定した上での開閉軸4仮説テスト＋未確定文字列軸）と、
  キーマップ解釈が不確かなら沈黙・解釈できて実測が無いなら「予測不能」と伝える、という(i)/(ii)の
  書き分けで、「誤警告」より「見逃し／予測不能の通知」に倒すよう保守化したが、実測セル表に無いカスタム
  構成（ユーザー独自の`custom_keymap_table`）は依然として(ii)止まりになる。カスタムキーマップこそ
  状態依存を作り込みやすい構成である、という非対称は本ADRのスコープでは解消できない
  （[ADR-195](195-keymap-learn-productization.md)の学習表が実行時に読めるようになれば、この沈黙域を
  縮められる可能性がある。本ADRの実装時点では前提にしない）。
- **親指キーとの衝突（rev2で決定2・決定3bに分岐として反映済み）**: 無変換/変換をNICOLAの親指キーとして
  使う場合、`keys.ime_on = ["変換"]`はチョード判定と衝突する。決定2は親指キーか否かで警告文言を分岐し、
  決定3bは親指キー単体を専用の新しい合流点で扱う（rev1で「要検討」だった分け方を、rev2で確定した）。
- **自動診断の出荷失敗の前例（opus round4 QM7、round2 E-3で訂正）**: `gji_charset_autodetect.rs`
  （544行、削除されておらず現存。`ImeToggleKind`等を`calibration_ipc.rs`等が利用）のモジュールdocに、
  同モジュール内にあった自動判定・設定支援ポップアップ機能（`gji_charset_popup.rs`/
  `gji_charset_write.rs`。判定ロジック自体ではなくこの2ファイルが撤去対象）が「実験的機能のまま出荷され、GJIのキー設定が実際にはカスタムなのに『カスタム以外』と誤診断されるなどユーザーの混乱を招いた」ため2026-09-02に撤去した、という記録がある。
  同型の機能なので、警告は判定の根拠を載せ、ブロックせず、判定できないキーは警告しない。ADR-191決定4に「今回の違い」を書いた（実測との突き合わせ）。
- **警告疲れ**: 状態依存のキーを意図して使うユーザー（入力中は変換、確定後はIME ON/OFF）にとって、警告は不要。「警告しない」の設定と、警告の文言を短くすることで緩和する。
- **MS-IME本体（`MSIME_NATIVE`）の判定精度（round3 D-2で訂正）**: 実測セル自体はあるが大半が1試行のみで
  信頼度が足りないため、決定1はプリセット単位でCannotPredictとして扱い警告しない（見逃しを許容）。
  レジストリで再割り当てが検出された場合も同じくCannotPredict。試行数を増やした再測定ができれば、
  この沈黙域を縮められる可能性がある（決定1「MS-IME本体」節参照）。
- **ADR-191との依存**: 状態依存のキーが多くなるほど、ADR-191の「観測に追随」の限界（観測できないアプリ）がユーザーに見える。本ADRは、その限界を緩和する層であって、ADR-191の前提ではない。
- **ADR-189/191決定1の固定セットの健全性に関する発見（round3 A-1(2)、本ADRのスコープ外）**: `MSIME_NATIVE`
  の実測セル（試行数不足のため本ADRでは警告に使わない）は、awase自身がbeliefトグルを書く固定セット
  `HankakuZenkaku`（0xF3/0xF4）が(A)開閉軸で状態依存になることを示している。ユーザー向けの「キーを
  変更してください」という案内では解決しない、awase自身の書き込み前提側の課題のため、本ADRでは扱わず
  記録に残す。ADR-189/191側で試行数を増やした再測定を検討すること。
- **決定3bのキー選択への追加そのものが持つリスク**: `.claude/rules/fix-requires-evidence.md`の
  「キー選択（IME ON/OFFに送るVK）」行が示す通り、`resolve_pending_thumb_as_single`の優先順位表への
  追加は既存ガード（modifier_key/explicit_action_consumed/suppress_solo_output等）を見落とすと過去に
  何度も二重actuationの原因になってきた（issue #136等、round2 B-1が指摘した具体例参照）。実装時は、
  この新しい入力が`ADR-189`の固定セット・ユーザー明示configの許可リストと衝突しない（許可リストを
  暗黙に広げない）ことを、既存の`architecture_guard.rs`等のガードで固定する。

## 却下した代替案

- **自動置き換え**: 意図した状態依存の使い方を壊す。
- **`config1.db`の書き換え**: 上記のとおり保守的に退ける。
- **警告なしでベストエフォートのみ**: ユーザーがずれの原因（自分のキー割り当て）に気づけない。

## 検証計画

- 決定1: `key_effect_table.rs`の実測セルを使い、rev4の判定式（対象VK範囲の限定、開閉軸の4仮説テスト(A)、
  (A)がIdentity以外のキーだけを対象にする未確定文字列の行方(B)）を、本文の検算表（ATOK/MSIMEプリセットの
  `ImeOn`/`ImeOff`/`Kanji`/`HankakuZenkaku`/`Henkan`/`Muhenkan`）と一致する結果になることを単体テストで
  固定する（Linuxで走る、round3が全448セルを機械的に検証した内容の再現）。特に`ImeOn`/`ImeOff`が(A)で
  非状態依存と判定されること（round2 A-1の反転再発防止）、`Enter`/`Esc`/`Bs`/`Space`が対象VK範囲外のため
  (A)(B)いずれの判定も回らないこと（round3 A-2の過検出再発防止）、ATOKの`Henkan`/`Muhenkan`が(A)で
  状態依存と判定されること、`ImeOff`/`HankakuZenkaku`/`Kanji`が(B)で警告対象になること、`MSIME_NATIVE`が
  プリセット単位でCannotPredictになること（round3 C-1）を直接アサートする。CannotPredict（沈黙(i)・
  伝える(ii)）の分岐、overlay適用構成が(i)に含まれることも固定する。
- 決定2: 警告の一度きり・再警告（GJIは`KeymapCache`の`stamp`変化、MS-IME本体は既存packed-bitsでの
  再判定）・「警告しない」の動作の単体テスト。親指キー（`general.left_thumb_key`/`right_thumb_key`）
  設定時に決定2の新規警告ではなく`msime_key_assignment::check_and_warn`型の警告に分岐すること、
  (A)/(B)/(ii)で警告文言が分かれることの単体テスト。
- 決定3: awase-settingsの1操作の置き換えで`config.toml`が期待どおり書かれ（`*_solo_tap_always_suppress`
  の同時書き込みを含む）、元に戻せること（実機）。
- 決定3b: `.claude/rules/fix-requires-evidence.md`の「キー選択」行（`resolve_pending_thumb_as_single`の
  優先順位列挙）の対象なので、テストか`docs/known-bugs/`記録のいずれかが必須。少なくとも: (a) エンジン
  活性中の単独打鍵（KeyUp解決）でOFF/Toggleが発火すること、(b) 通常のタップ（100ms超）でも
  `execute_from_loop`のタイマー解決に落ちずactuateされること（ADR-186 3/3 FAILの再発防止の直接確認）、
  (c) チョード成立時は発火しないこと、(d) composing中も発火すること（結果〈Discarded/Committed〉が
  プリセット依存であることは実機検証で確認し単体テストでは求めない）、(e) `modifier_key`／
  `explicit_action_consumed`／`suppress_solo_output`が立っているときは発火しないこと（round2 B-1）、
  (f) 専用Fnキーと同一VKに設定した場合は専用Fnキーが勝つこと、(g)
  `tests/architecture_guard.rs::user_ime_on_paths_are_paired_with_eisu_reset`に新経路を追加し、
  `eisu_reset_on_ime_on`との対称配線を固定すること（round2 B-7）、(h) 同一VKに`keys.ime_toggle`（bare）
  と`*_solo_tap_ime_action`の両方を設定した場合にconfig検証がエラー/無効化のいずれかで排他にすること、
  1打鍵でON→OFFへ往復する回帰が再現しないこと（round3 B-2）。

## 関連

ADR-092（外部キーの意味づけ）、ADR-110（撤回）、ADR-111・114（`[[keymap]]`と制限）、ADR-143〜146（GJIキーマップ書き換え、保留）、ADR-153（明示config）、ADR-176（較正UI）、ADR-186（GJI/ATOKモードキー実測行列とbelief追随、決定3bのKeyUp解決の教訓の出所）、
ADR-189（半角/全角のbelief基づく書き込み）、ADR-191（本ADRの前提。`key_effect_predictor.rs`/`key_effect_table.rs`が決定1の再利用対象）、ADR-195（キーマップ学習の製品化、決定1の沈黙域を将来縮められる可能性）、BUG-143（`session_keymap`と`custom_keymap_table`の食い違い）。
