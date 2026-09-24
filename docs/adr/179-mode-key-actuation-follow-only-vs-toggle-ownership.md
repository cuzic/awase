---
id: ADR-179
title: |-
  無変換/変換の非親指キー時actuation-autoを撤去し、`ModeKeyActuationOwner`
  列挙でbelief書き込み・明示actuate・ActivationSyncの責務を統一管理する
summary: |-
  主目的は新機構の導入ではなく、モードキー関連のBUG対応(BUG-115/113/123/124/142/143)
  のたびに個別追加されてきた対症療法の撤去。opus-adversarial-consult round1〜8を経て、
  無変換/変換が非親指キー配置時に限り、親指キー配置時に既に実装・検証済みのfollow-only
  経路(shadow_action override)を使えることを実証。`ModeKeyActuationOwner`
  {NotAModeKey, FsmDelegate, AwaseExplicit, PhysicalDelivery}を`kp_stage_shadow_
  ime_toggle`内1箇所で計算し、belief書き込み・明示actuate・`ActivationSync`由来
  effect除去(`key_pipeline.rs:434`)の判断を全てこの列挙から導く設計に収束。
  GJI/MS-IME両側を対称に修正。round8で「収束。Blocker・Must-fixいずれも無し。
  実装着手可」との最終判定を得た。設計の紆余曲折はレビュー経緯節を参照。
status: |-
  **収束済み（opus-adversarial-consult round1〜round8）。設計・スコープ・
  条件式のいずれにも未解決の欠陥は無いと判定された。実装フェーズへ持ち越す
  事項（実機A/Bの4象限、`schedule_settle_retry`を巻き込まない別関数化、
  `actuation_owner`書き込み点のguard新設、`NotAModeKey`/`AwaseExplicit`
  非統合の固定）は「未解決点」節に記載。`key_pipeline.rs:1688`のコメント
  訂正〈独立したドキュメント負債〉は2026-09-17に対応済み。**
  **【2026-09-24更新】決定2の`ModeKeyActuationOwner`列挙と配線は、ADR-191の撤去（`502c6673`、2026-09-21）で到達不能として撤去済み（コード中の出現0件）。**
  **【2026-09-19更新】決定1・決定2は実装済み**（`2e8b834b`・`09ea4ce1`）。その後の実機A/Bで
  実験コミット4件（`b9e45e55`・`176d37af`・`c0814776`・`f0e36b0e`）が追加され、Passthrough設定
  を前提にした実験が**現在も有効**。**developへマージする前に、実験を撤去して既定（Suppress）へ
  戻す必要がある**（本文「実装状況と実験コミット」節のマージ前TODOを参照）。
related_adr:
  - "ADR-092"
  - "ADR-115"
  - "ADR-119"
  - "ADR-135"
  - "ADR-141"
  - "ADR-147"
  - "ADR-149"
  - "ADR-153"
  - "ADR-154"
  - "ADR-174"
  - "ADR-175"
  - "ADR-176"
---

# ADR-179: 無変換/変換の非親指キー時actuation-autoを撤去し、`ModeKeyActuationOwner`列挙で責務を統一管理する

**8ラウンドのopus-adversarial-consultを経て収束済み。決定1・2は実装済み（実験コミット4件は2026-09-20に撤去済み）。ただし決定2の`ModeKeyActuationOwner`は、ADR-191の撤去（`502c6673`、2026-09-21）で到達不能として列挙・配線ごと撤去済み。** 設計の
紆余曲折（当初案からの縮小・4ラウンド連続で踏んだ「送信元が移動するだけ」
という同型の誤り等）は末尾「レビュー経緯」節にまとめてある——まずは
以下の決定・スコープ・撤去対象を読めば実装に着手できる。

## 主目的（誤解しないこと）

**本ADRの主目的は、新しい機構（`ModeKeyActuationOwner`列挙）を導入する
ことそれ自体ではない。** 主目的は、モードキー周りのBUG対応
（BUG-115/BUG-113/BUG-123/BUG-124/BUG-142/BUG-143 等）のたびに
**あてずっぽうで積み増されてきた個別対症療法**を**撤去**し、単純で全体の
見通しの良い設計に戻すことである。

**8ラウンドのレビューを経て、この主目的を実際に安全に達成できる範囲は
当初の想定より狭いことが判明した。** 無変換/変換・ひらがな/カタカナが
**親指シフトキーとして設定されている場合**のdelegate-to-open-axis機構は、
モードキーのIME意味論（一方通行/トグル）とは**直交する別の軸**（NICOLAの
チョード＝同時打鍵の起点か単独タップかを判別するタイミング制御）を
担っており、これを崩すとBUG-115型の機能不全かBUG-113型の「@」再発の
どちらかを必ず作る。そのため**本ADRは撤去の主戦場を「親指キーでない
場合」の機構（actuation-auto）に絞り、親指キー側（delegate-to-open-axis）
は本ADRのスコープ外と明示的に宣言する**（別ADR候補として将来検討）。

成功基準は「新機構を実装したか」ではなく、下記「撤去対象」に列挙した
個別機構・専用フィールドが実際にどれだけ削除できたかである。範囲を
絞った結果、削除できる量そのものは当初案（表9項目）より大幅に小さく
なった（最終的に2項目）が、**「削除できないものを削除できると誤認した
まま実装に進む」よりは正直な見積りである**（この評価はround8のレビュー
でも支持された。詳細は「レビュー経緯」節）。

## 目的

「撤去対象」に列挙した機構を撤去した跡地に、単一の原則を置く。

> **無変換/変換のGJI/MS-IME検出値（On/Off/Toggle分類）は、親指キーとして
> 設定されているかどうかに関わらず、常に同じ入力経路（`shadow_action`
> override）で扱う。「beliefを書く責務」と「実IMEを変える責務」は
> 別々の問いであり、`ModeKeyActuationOwner`という1つの列挙値で
> 明示的に区別する。実IMEを変える責務が物理キー配送（GJI/MS-IME自身）に
> ある場合、awaseは明示actuateも`ActivationSync`の自動echoも一切
> 発行しない。**

これは新しい原則の発明ではなく、**親指キー配置時に既に実装・検証済みの
belief書き込み経路を、非親指キー配置時にも一貫して使う**という単純化
である。

## 撤去対象（本ADRのスコープ内で実際に撤去するもの）

| 機構 | 導入元 | 何のための対症療法だったか | 撤去方法 |
|---|---|---|---|
| `route_thumb_key_action`の`is_thumb_key`分岐・`on`/`off`/`toggle`の`&mut Vec`引数3本 | ADR-135 Phase1（BUG-115 F7） | 無変換/変換が親指キーでない場合に、既にある`shadow_action`follow-only機構を使わず別の能動actuation経路（Vecへのpush）を新設した | 分岐・引数ごと削除し、常に`ime_toggle_kind_to_shadow_action`経由で`henkan_shadow_override`/`muhenkan_shadow_override`へ値を渡す（On/Off/Toggleいずれの分類でも）。関数はほぼ自明な処理に単純化される |
| `kp_stage_shadow_ime_toggle`の`delegate_owned`という1変数に混ざっていた「belief書き込み責務」と「実IME actuation責務」の混同 | ADR-141 C2（`delegate_owned`の導入） | 親指キー配置時はこの2つの責務がFSM delegateに一致して集約されるため、1変数で表現できていた | `ModeKeyActuationOwner`列挙で2つの責務を分離する（決定2参照） |

`ime_on_auto`/`ime_off_auto`という機構自体（`Engine`のフィールド・
setter・`match_ime_on_off_auto`）は**削除しない**（GJIが専用Fnキー
F15–F24にIME ON/OFF/トグルを割り当てた場合の検出、ADR-092 Step4c用に
維持する。無変換/変換だけがここから完全に手を引く）。`ActivationSync`の
origin選別という、当初検討した独立機構は不要と判明したため、撤去対象
からもスコープからも除外する（`ModeKeyActuationOwner`が代わりを果たす）。

## スコープ外と明示的に宣言する隣接機構（撤去しない・変更しない）

- **delegate-to-open-axis一式**: `NicolaFsm`の`henkan_vk`/`muhenkan_vk`/
  `hiragana_vk`/`katakana_vk`と対応する`*_delegate_to_open_axis`フィールド、
  `resolve_pending_thumb_as_single`の優先順位match、`auto_delegate_open_axis_
  consumed`マーカー（ADR-154）、`explicit_ime_action_consumed`マーカー
  （ADR-153）、`delegate_owns_mode_key_shadow_toggle`/
  `mode_key_delegate_owns_shadow_toggle`の排他性機構（ADR-141/154、
  親指キー時の判定はそのまま）。親指キー設定時の挙動・タイミング制御は
  一切変更しない。
- **`ime_on_auto`/`ime_off_auto`/`ime_toggle_auto`機構そのもの
  （`Engine`のフィールド・setter・`match_ime_on_off_auto`/
  `match_ime_toggle_auto`）**: GJIが専用Fnキー（F15–F24、ADR-092
  Step4c、`extract_ime_on_off_toggle_combos`）にIME ON/OFF/トグルを
  割り当てた場合の検出という**独立機能**のために維持する。無変換/変換は
  この機構から完全に手を引くため、実質的な消費者はF15–F24のみになる。
- **Hiragana/Katakana**: ADR-135 Phase1の訂正で既にactuation-autoから
  外れており、本ADRの対象外（現状の`shadow_action`静的マップ＋Phase2
  オーバーライドのまま変更しない）。
- **物理的にトグルなVK（`VK_DBE_SBCSCHAR`/`DBCSCHAR`等）**: 対象外の
  まま。主要プロファイル（GjiDirect/MsImeDirect）では既に無条件
  Suppress済み（横取り実装済み）。`ime_actuation_owned==false`
  プロファイル・`InputRelay`への拡張は将来の別ADR候補。

## ADR-176・ADR-174との関係（撤去対象ではなく入力として維持）

- **ADR-176（較正機能）**: `state/calibrated_mode_key.rs`一式、
  `gate_thumb_key_ime_actions`出力の2箇所差し替え配線、`[[calibration]]`
  config、fingerprint/stale基盤、bypass機構等は**撤去対象ではない**。
  較正結果は最終的に`ImeToggleKind`（On/Off/Toggle）という同じ語彙で
  `route_thumb_key_action`/actuation-autoの入力になるため、本ADRの
  設計はこの出力をそのまま消費すればよい。ADR-176は「静的分類が信用
  できないときに実測で補正する」という別レイヤーの関心事であり、本ADR
  が扱う「分類が決まった後、awaseは能動actuateすべきか」という問いとは
  独立している。
- **ADR-174（`classify_mode_key_ime_action`の`custom_keymap_table`
  優先フォールスルー）**: 分類ロジックの一部であり、本ADRが扱う
  actuation側の問題とは独立。維持する。

## 決定

### 決定1: `route_thumb_key_action`・MS-IME側ゲートの分岐を撤去し、入力経路を一本化する

`route_thumb_key_action`（`gji_charset_autodetect.rs:625`）から
`is_thumb_key`引数・`on`/`off`/`toggle`の`&mut Vec`引数3本・それによる
分岐を削除する。分類結果（On/Off/Toggleいずれも）は親指キーかどうかに
関わらず常に`ime_toggle_kind_to_shadow_action`を経由し、
`henkan_shadow_override`/`muhenkan_shadow_override`（`Runtime`
フィールド）へ渡す。**対称に**、MS-IME側（`message_handlers.rs:
1031-1064`）の`henkan_is_thumb_key`/`muhenkan_is_thumb_key`ゲートも
撤去し、同じ`Runtime`フィールドへ常に値を渡す（GJI側だけの変更は
MS-IME側との非対称を生むため、必ず両方に適用する）。

**M15マスク（ADR-153、`explicit_config.is_some()`の早期return）は
維持する**——呼び出し元の`mask_auto_detect_for_explicit_config`が
両ファイル・両分岐をカバーする設計になっているため、個別チェックは
冗長になるが、実装時に呼び出し元のマスクが本当に等価であることを
確認したうえで削除する。

### 決定2: `ModeKeyActuationOwner`を新設し、belief書き込み・明示actuate・`ActivationSync`の3点を一意に決める

```rust
enum ModeKeyActuationOwner {
    /// この打鍵はshadow_action由来のintentを採用していない
    /// （sync_direction/explicit configが優先された、またはそもそも
    /// モードキーではない）。3判断とも従来どおり。`Option`で表現せず
    /// 独立したvariantにする——`Option`だと実装者が`Some(PhysicalDelivery)
    /// => skip, _ => 従来どおり`と書きがちで、将来variantが増えても
    /// 黙って`_`に吸収され、match網羅性という本設計の生命線が効かなく
    /// なる。
    NotAModeKey,
    /// FSM delegate（`resolve_pending_thumb_as_single`）が単独タップ
    /// 確定時に own する。親指キー配置時、belief ON中のみ
    /// （既存の`delegate_owns_mode_key_shadow_toggle && effective_open()`、
    /// ロジックは無変更でこの列挙値へラップするだけ）。
    FsmDelegate,
    /// awase自身がbelief書き込み・実actuationの両方を行う。
    /// 静的shadow_action（Hiragana/Katakana/Alphanumeric/DBE系）、
    /// および無変換/変換のToggle分類（非冪等、awaseが唯一の変更主体で
    /// あるべき）はここに属する。
    AwaseExplicit,
    /// beliefはawaseが書くが、実IME状態の変更は物理キー配送により
    /// GJI/MS-IME自身が行う。awaseは明示actuateも`ActivationSync`の
    /// 自動echoも一切発行しない。無変換/変換のOn/Off分類（非親指キー
    /// 設定時）専用。
    PhysicalDelivery,
}
```

**4値×3判断の表**（この表がADRの中核）:

| | `NotAModeKey` | `FsmDelegate` | `AwaseExplicit` | `PhysicalDelivery` |
|---|---|---|---|---|
| beliefを書くか | 従来どおり（intentがあれば書く） | ✗ | ✓ | ✓ |
| 明示actuateするか | 従来どおり | ✗ | ✓ | ✗ |
| `ActivationSync`を通すか | 従来どおり（通す） | ✓ | ✓ | ✗ |

`NotAModeKey`と`AwaseExplicit`は現時点で3判断とも同一挙動になるが、
将来「shadow_action由来だが`AwaseExplicit`とは別扱いにしたいケース」が
増えたときに区別できるよう分けておく（両者を1つに統合しない——実装時に
「同じなら統合してよい」というリファクタが入らないよう、コメントまたは
ガードで明示的に固定する）。

#### owner の定義

ownerは「overrideが何を言ったか」ではなく「この打鍵で実際に採用された
intentの出所」で定義する。`PhysicalDelivery`の主条件は**「対象VK
（無変換/変換）かつOn/Off分類（override由来）かつ非親指キー設定」**に
限定する。`sync_direction`（`keys.ime_detect`）はこの主条件を満たすVKに
対する**修飾**としてのみ扱う——`kp_stage_shadow_ime_toggle`のON→OFF
明示actuateブロック（`:1700-1766`）はintent種別を一切見ずbelief遷移
だけで発火するため、`sync_direction`を主条件にすると`keys.ime_detect`
に登録された全VK（無変換/変換に限らない）でactuateと`ActivationSync`が
止まってしまい、ADR-175/BUG-142の実機A/B確認済み回避策
（`keys.ime_detect.toggle`に`VK_DBE_SBCSCHAR`/`DBCSCHAR`を登録し
Toggle解決へ変える）が依存する「Toggle→OFF解決時に`:1700`が実actuate
する」という前提を壊し、BUG-142（IME ON固着）を再発させる。

- 対象VKで`sync_direction`も`Some`なら`PhysicalDelivery`のまま
  （物理配送されるのでawase不要、整合する）。
- **ただし`sync_direction`と`shadow_action`（override由来）の方向が
  一致する場合に限る。** `kp_stage_shadow_ime_toggle`のintent優先順位は
  `sync_direction` > `shadow_action`なので、両者が食い違う場合（例:
  `keys.ime_detect.on=無変換`と設定しているが、GJI既定キーマップでは
  無変換が`Off`分類）は、採用されるintentは`sync_direction`（`TurnOn`）
  なのにownerの判定根拠は`shadow_action`（`Off`）になり、beliefはONへ
  動くのに物理キー配送を受けたGJIは自分の意味論（Off）でIMEをOFFに
  する——belief/実IMEが逆方向に乖離し、しかも`PhysicalDelivery`には
  再試行も自己修復も無い（後述「残るトレードオフ」参照）ため恒久固着
  しうる。方向が食い違う場合は保守的に`NotAModeKey`へ倒す（＝従来どおり
  のactuate挙動を保つ）。この「構造的な重なりを個別の優先順位ルールで
  捌かず排除する」判断は、ADR-176の`explicit_config_conflict_reason`
  （`state/calibrated_mode_key.rs:214-220`、対象VKが`keys.ime_detect`
  等に既にある場合は較正自体を拒否する）と同型の先例に倣う。
- **対象VK以外**で`sync_direction`が`Some`の場合は`NotAModeKey`
  （＝従来どおり、`keys.ime_detect`の既存挙動を一切変えない）。
- explicit config（M15マスク）が効いている場合、override自体が`None`に
  なるため owner も自動的に`NotAModeKey`になる（従来どおり整合）。

#### 計算点

**計算は1箇所**: `kp_stage_shadow_ime_toggle`内の既存`delegate_armed`/
`current`計算（`:1406-1410`）の**直後**。この位置なら`delegate_armed`・
`current`（ライブbelief）・`event.ime_relevance.sync_direction`/
`shadow_action`が全て手元に揃っており、`FsmDelegate`が文字どおり
「既存ロジックのラップ」になる。**`Runtime::enrich_ime_relevance`
（`key_pipeline.rs:265`、`kp_run_inner`の早い段階）では計算しない**
——この地点は`:318 kp_stage_focus_probe`/`:319 kp_stage_idle_conv_check`
より**前**であり、これら2ステージはbelief（`effective_open()`）を
書き換えうる。`FsmDelegate`の判定が依存する`delegate_owned`
（`delegate_armed && effective_open()`）は`kp_stage_shadow_ime_toggle`
内で**ライブに**評価される値であり、より早い時点のスナップショットで
代用すると「ロジック無変更でラップするだけ」という前提が崩れ、親指キー
設定時の挙動が変わりうる。

`kp_stage_shadow_ime_toggle`内の各種早期return（KeyUp・非KeyDown・
injected・explicit `SuppressOnly`等、`:1406`より前にある分岐）で到達
しない打鍵はownerが`NotAModeKey`のまま（＝従来どおり）となる。
`:1410`より**後**にも、beliefを一切動かさずに`return false`する経路が
2つある（`:1417-1430`の intent解決で候補無し、`:1544`のno-op分岐）。
これらの経路ではownerが既に`PhysicalDelivery`に設定済みのまま抜ける
ため、消費点2側の条件に「この打鍵自身が実際にbeliefを動かしたか」を
別途ANDする必要がある（後述「消費点」参照）——単純に「ownerが
`NotAModeKey`でなければ実害がある早期returnは無い」とは言えない点に
注意。

`RawKeyEvent`は`Copy`（`src/types.rs:262`）なので、ここで
`event.ime_relevance.actuation_owner`に書き込んだ値は、後段の
`:426`（値渡し後の`engine.on_input`）・`:436`（後述のeffect除去
フィルタ）のいずれでも読める。計算点はGJI・MS-IMEどちらが
`henkan_shadow_override`/`muhenkan_shadow_override`を書いたかを区別
しないため、**GJI側・MS-IME側の両方を自動的にカバーする**（後述
「書き込み側の対称化」参照）。VKを名指しする新しい箇所を増やさない
——判定はこの1箇所に集約される。

**`ModeKeyActuationOwner`は所有権の唯一のSSOTではない**（明記必須）:
この列挙がカバーするのは`shadow_action`由来のintentだけである。
explicit config（`explicit_ime_action_target`/`explicit_ime_action_
consumed`、ADR-153）、`transport.rs::plan`の物理配送Suppress/Allow
判定は、この列挙の外側で従来どおり独立に決まる。次の担当者が「所有権
判断はすべてこの列挙で表現されている」と誤解し、新しいケースをここだけ
に配線してADR-119の轍を踏まないよう、実装時にコメントで明記する。

#### 消費点

**消費は2箇所**（下記の表を参照し、散文で判断を再記述しない）:

1. `kp_stage_shadow_ime_toggle`（`:1410`直後、上記「計算点」参照）:
   belief書き込みは`owner`が`FsmDelegate`以外のとき行う（既存の
   `!delegate_owned`を一般化）。ON→OFF方向の明示actuateブロックは
   `FsmDelegate`・`PhysicalDelivery`の**いずれでもない**とき（＝
   `NotAModeKey`・`AwaseExplicit`）実行する。
2. `key_pipeline.rs:427-434`（`strip_ime_set_open_if_settling`と同じ
   場所、`kp_stage_post_decision`より前）に、同型の一次フィルタを
   新設する: **`shadow_toggled && owner == PhysicalDelivery`**を条件に、
   `SetOpen{origin: ActivationSync}`のeffectを`decision.effects`から
   `retain`で落とす。
   - `shadow_toggled`は`kp_stage_shadow_ime_toggle`の戻り値
     （「この打鍵自身が実際にIME ON/OFFを変化させたか」、`:328`で
     束縛・`:462`/`:478`で使用中）。`owner`単独条件では、この打鍵とは
     無関係な観測駆動のbelief変化（`kp_stage_focus_probe`/`kp_stage_
     idle_conv_check`）に起因する`ActivationSync`まで誤って除去して
     しまう（上記「計算点」で触れた`:1410`より後の早期return経路が
     この失敗を招く）。`auto_delegate_open_axis_consumed`
     （`:1541`、ADR-154）も同じ理由で「`delegate_armed`かつbeliefが
     実際にOFF→ONへ動いたとき」だけ立てる設計になっており、これが
     先例になる。
   - **実送信は`kp_stage_post_decision`ではなく`kp_stage_execute`→
     `executor.rs:755`（`Effect::Ime(ImeEffect::SetOpen { open, .. })`、
     **originを`..`で破棄**）→`dispatch_ime_set_open`で無条件に行われる
     ため、`kp_stage_post_decision`でのbelief記帳スキップだけでは
     実送信は止まらない。** `key_pipeline.rs:427-434`のコメントに
     「2026-07-05: 前回の修正が効かなかった原因」として、まさにこの
     区別（belief側フィルタと、`decision.effects`からeffect自体を
     取り除く一次フィルタは役割が違う）が既に記録されている。実行順序
     は`:436`のstrip < `:507`のpost_decision < `:520`のexecuteであり、
     **`:436`でeffect自体を落とせば、`:507`は`find_ime_set_open_
     with_origin`が`None`を返すため自動的に整合し、`:520`にも届かない**
     ため、2番目の消費点として`kp_apply_conv_engine_sync`（`:1162`、
     origin=conv観測由来で対象外）への配線は不要。

   **注意（必須）**: `:440-447`の既存`schedule_settle_retry`呼び出しは、
   settle中に握りつぶした`SetOpen`を「settle明けに一度だけ再試行する」
   ための機構であり、`PhysicalDelivery`のstripは**意図的に永久に
   送らない**——これに`schedule_settle_retry`を巻き込んではならない。
   既存の`strip_ime_set_open_if_settling`をそのまま流用せず、**戻り値
   または関数を分ける**こと。

これにより、明示actuateも`ActivationSync`の自動echoも`PhysicalDelivery`
のときはどちらも発行されなくなる。awaseからの送信はゼロになり、
GJI/MS-IME自身の物理キー反応だけがIME状態を変える。

#### 書き込み側の対称化

決定1をGJI側だけでなくMS-IME側にも対称に適用する。この2箇所が書き込む
先は同じ`Runtime`フィールドであり、owner計算（`kp_stage_shadow_ime_
toggle`）はVK・`shadow_action`・`sync_direction`だけを見てGJI/MS-IMEを
区別しないため、自動的に両対応になる——変更が要るのは「override自体を
差すかどうか」のゲート条件（書き込み側）だけである。

**残る非対称（無害だが明記する）**: MS-IME側の`delegate_assignment`
（`set_muhenkan_delegate_to_open_axis`/`set_henkan_delegate_to_open_
axis`）は元々`is_configured_thumb_key`でゲートされていない（GJI側の
`route_thumb_key_action`は非親指キーで`None`を返すため対称的に
delegateが付かないのと対照的）。これは無害——`resolve_pending_thumb_
as_single`自体が`muhenkan_vk == Some(vk)`（＝親指キー設定）を要求し、
`delegate_owns_mode_key_shadow_toggle`も`is_configured_thumb_key`を
要求するため、非親指キーのdelegate値はどのみち消費されず`FsmDelegate`
にはならない。ただし`owner`計算が`delegate_owns_mode_key_shadow_
toggle`をラップする以上、この不変条件に依存していることを実装コメント
に明記する。

#### なぜ過去に却下された設計の再導入にならないか

MS-IME側のコメント（`message_handlers.rs:1031-1064`）には、過去の
`/code-review`が「非親指キーにもoverrideを差す」設計を試み、BUG-46型の
二重actuationを発見してガードを追加した記録が残っている。この過去の
失敗は「overrideを差す（belief追随を始める）が、結果生じるactuationは
止めない」という組み合わせで発生した。本設計は「overrideを差し、結果
生じるactuation（明示actuateと`ActivationSync`の自動echoの**両方**）を
完全に止める」という別の組み合わせであり、過去に試されていない。加えて
書き込み側もGJI/MS-IME対称に直すため、新たな非対称も生まない。

## なぜこれで安全か

- **隠れた前提「物理キーが実際に配送される」が実際に成立する**:
  非親指キーの`VK_CONVERT`/`VK_NONCONVERT`は`hook.rs::classify_key`→
  `vk.rs::is_passthrough`に含まれず、JIS scanmapにもhenkan/muhenkanの
  scan codeが無いため`KeyClassification::Passthrough`→
  `nicola_fsm.rs::bypass_reason`→`handle_bypass`（consumed=false）→
  `Decision::PassThrough`となり、`transport.rs::plan`の無条件Allow
  （ADR-141 C2対策として既に存在する専用分岐、コメントに「OS側の
  実際の切替はGJI自身が物理キー配送を通じて行う」と明記済み）が実際に
  効く。新しい配送経路を作る必要はない。
- **F15–F24機能の巻き添えは無い**: `ime_on_auto`/`ime_off_auto`機構
  自体を維持し、無変換/変換だけがそこから手を引く。
- **送信元が移動するだけ、という失敗は無い**: `PhysicalDelivery`の
  ときは明示actuateと`ActivationSync`由来の`SetOpen` effectの**両方**を
  止めるため、awase側の送信がゼロになる。`check_active_transition`は
  遷移方向を問わず`SetOpen{open: now_active}`を生成するため、この対策は
  OFF→ON・ON→OFF**両方向**に等しく効く。
- **過去に却下された設計の再導入にはならない**（上記「なぜ過去に却下
  された設計の再導入にならないか」参照）。書き込み側もGJI/MS-IME対称に
  直すため、新たな非対称も生まない。
- **Toggleの扱いに矛盾は無い**: Toggle分類は`AwaseExplicit`に留まり、
  非冪等性の問題を作らない。
- **VK名指し箇所は増えない**: `ModeKeyActuationOwner`の計算は
  `kp_stage_shadow_ime_toggle`内1箇所のみで行い、消費側
  （同ステージ・`key_pipeline.rs:436`）はVKを一切参照せず列挙値だけを
  見る。
- **簡素化か特殊条件の追加か**: `delegate_armed`/`delegate_owned`という
  2つのboolean＋新条件を1つのexhaustiveな列挙に畳み込むため、概念数は
  実質的に減り、将来ケースが増えた際の配線漏れ（ADR-119が警告する
  「合流点への追加は複数箇所に配線が要る」問題）をコンパイラの
  non-exhaustive matchエラーが検出する構造になる。

## 残るトレードオフ（正直に書く）

- **`PhysicalDelivery`前提のMS-IME側は実機未検証**: 核心前提は「生の
  `VK_CONVERT`/`VK_NONCONVERT`を受けたIME自身が実IME状態を変える」
  ことである。GJIについてはBUG-115の設計前提そのものであり
  `transport.rs:327-339`が実機知見として明記済みだが、**MS-IMEについて
  は同等の実機記録が無い**（`transport.rs:386-393`はVK_DBE_*系
  0xF0/0xF1についての2026-08-05実機確認であり、VK_CONVERT/
  VK_NONCONVERTは対象外）。GJIで実証済みの前提をMS-IMEへ未検証のまま
  拡張することになるため、**「MS-IME環境でのPhysicalDelivery前提を
  実機A/Bで確認する」を実装の受け入れ条件とする**。成立しない場合、
  MS-IMEユーザーは「誰もIMEを切り替えない」二重の空振り（ADR-119型）
  になる。
- **`PhysicalDelivery`には再試行も自己修復も無い**（round1以来の
  「冪等だから自己修復する」という安全論証がここでは成立しない点を
  正直に記録する）: `:436`のeffect除去は`schedule_settle_retry`を
  呼ばない設計にするため、GJI/MS-IMEが実際には物理キーに反応しなかった
  場合、beliefは目標値に書かれるが実IMEは変わらないままになる。しかも
  次に同じキーを押しても`action.resolve(current)`がbelief（既に目標値）
  と一致するためno-op早期returnに落ち、**回復しない**。観測可能な環境
  （ImmCross/FocusProbe/poll）ではdrift correctionが救うが、**Blind
  環境（TsfNative/UWP）では救済経路が無く恒久固着する**——
  `ActivationSync`が持っていた"nonaiyo"保険（belief ONなのに実IMEが
  開いていない場合の強制オープン）が、`PhysicalDelivery`の経路では
  失われることを意味する。したがって実機A/Bの確認対象は**{GJI, MS-IME}
  × {ImmCross, TsfNative}の4象限**に拡張し、特にTsfNativeアプリ
  （Windows Terminal/WezTerm等）での無変換/変換の実際の反応を確認する
  ことを受け入れ条件に含める。

## 実装状況と実験コミット（2026-09-19時点）

ブランチ`feat/adr178-mode-key-actuation-and-tsfnative-rescue-teardown`（developには未マージ）。

**実装済み（本ADRの決定）**
- 決定1（非親指キーactuation-auto〈F15-F24〉の撤去、入口の一本化）: `2e8b834b`
- 決定2（`ModeKeyActuationOwner`列挙で所有権を一元判定）: `09ea4ce1`。`actuation_owner`書き込み点を
  1箇所に固定するガード（未解決点5）は`architecture_guard.rs::actuation_owner_is_computed_in_
  exactly_one_place`として実装済み。
- 実機A/B用の一時コミット（無変換/変換を非親指キー化→既定へ戻す）: `5db0c2f2`・`43617a1b`。
  未解決点10（{GJI, MS-IME}×{ImmCross, TsfNative}の4象限）の結果は、本ADRには未記録。

**実験コミット（本ADRの決定ではなく、ユーザー指示による実機での試行。現在も有効）**
- `b9e45e55` 単独タップpassthrough辞退をTurnOff/Toggleにも拡張（ADR-147の「TurnOn限定」を緩和）
- `176d37af` FollowOnly belief追随（`SetOpenOrigin::PhysicalDeliveryFollow`、`ImeOpenRequest`）を新設、
  Toggleは辞退対象から除外
- `c0814776` 親指キー設定×IME OFF時もPhysicalDeliveryに一般化（`!is_configured_thumb_key`条件を撤廃）
- `f0e36b0e` Henkan/MuhenkanのSuppress設定は方向を問わず完全に無視（`mode_key_config`がSomeのキー限定）

**実験コミット4件は撤去済み（2026-09-20、developマージ前、ユーザー指示）**: `f0e36b0e`・`c0814776`・`176d37af`・`b9e45e55`を
この順にrevertした（衝突なし。`cargo test --lib`・`awase-windows`のlib/architecture_guard/layer_boundary_guard/golden_scenarios・
`--test scenarios`・clippyが通る）。既定（Suppress）の挙動へ戻る。Windows実機の`config.toml`の実験設定
（`muhenkan_solo_tap_always_suppress = false`等）は、実機側で別途戻す（このリポジトリの変更では戻らない）。
`docs/experiments.md`エントリ27に記録した。ADR-182（チョード判定の修正）のテストはrevert後も通る。

## 未解決点（実装設計で詰める）

1. **`ModeKeyActuationOwner`の配置場所**: `types.rs`（`ImeRelevance`の
   隣、`shadow_action`と同じ構造体に載せる）を第一候補とする。計算点は
   `kp_stage_shadow_ime_toggle`（`:1410`直後）だが、格納先のフィールド
   自体は`ImeRelevance`のまま変わらない。
2. **`kp_apply_conv_engine_sync`（`:1162`）は対象外でよい**（確認済み、
   originが異なる〈conv観測由来〉ため）。消費点2を`:434`のeffect除去に
   したことで、この経路への配線自体が不要になったことも実装時に
   再確認する。
3. **M15マスクの等価性確認**（決定1参照）: `route_thumb_key_action`・
   MS-IME側ゲート内の`explicit_config`早期returnを削除して呼び出し元の
   マスクだけに頼ってよいか、実コードで確認する。
4. **eisu救済への影響確認**: belief書き込み自体は`PhysicalDelivery`でも
   従来どおり行われるため、`eisu_reset_on_ime_on`等のトリガ条件（belief
   遷移ベース）自体は生きるはずだが、`ActivationSync`関連のtracingログを
   前提にした診断手段があれば影響を確認する。
5. **`ModeKeyActuationOwner`書き込み点を1箇所に固定するガードの新設**:
   対象は`key_pipeline.rs`（計算点がここにあるため）。既存の
   `architecture_guard.rs::ime_relevance_shadow_action_writes_are_
   accounted_for`と同種の、`actuation_owner`への書き込み箇所数を1に
   固定するテストを追加する（計算点を1箇所に保つことが本設計の生命線
   であり、散文だけで守ると`.claude/rules/ime-belief-architecture.md`
   が警告するパターンになる）。あわせて`NotAModeKey`と`AwaseExplicit`
   を1つに統合するリファクタが将来入らないよう、コメントまたはガードで
   明示的に固定する。
6. **`:436`〜`:507`の間にSetOpenを再生成する処理が無いことの確認**:
   `kp_restore_hiragana_for_suppressed_mode_key`（`:476`、BUG-116決定2）
   はこの間にあるが、無変換/変換は`transport.rs:363-372`でAllowされ
   Suppressされないため`PhysicalDelivery`対象キーには関与しないと確認
   済み。実装時に念のため再確認する。
7. **ADR-175（BUG-142）実装との順序**: ADR-175は未実装
   （`fix_commits: []`）。半角/全角は`ImeKeyKind::from_vk`の静的マップ
   対象でありこのADRの対象（無変換/変換）には含まれないため、実装順序
   に依存しない。
8. **回帰テストへの影響**: `src/engine/tests.rs`の`set_ime_on_auto_keys`/
   `set_ime_off_auto_keys`を使う既存テスト7件は`ime_on_auto`/
   `ime_off_auto`機構自体を維持するため無変更で済む見込み。新規に、
   非親指キー無変換/変換のOn/Off各方向で「beliefが書かれ、明示actuateも
   `ActivationSync`も発行されないこと」「物理キーがAllowされること」を
   確認する回帰テストと、MS-IME側の対称テストを追加する。
   `executor.rs`側のeffect除去には`strip_removes_set_open_when_
   settling`と同型の単体テストを追加する。`PhysicalDelivery`のstripが
   `schedule_settle_retry`を呼ばないことも単体テストで固定する。
9. **`keys.ime_detect`対象外VKでの`NotAModeKey`維持の確認**: 対象VK
   （無変換/変換）以外に`sync_direction`が設定されているケース（例:
   F13等）で、owner計算が誤って`PhysicalDelivery`側の分岐に入らない
   ことをテストで固定する。ADR-175/BUG-142の回避策
   （`keys.ime_detect.toggle`にVK_DBE_SBCSCHAR/DBCSCHARを登録）が
   引き続き機能することを確認する回帰テストを含める。
10. **実機A/B確認マトリクス**: {GJI, MS-IME} × {ImmCross, TsfNative}の
    4象限で、非親指キーの無変換/変換が物理配送だけでIME状態を実際に
    変えることを確認する。特にTsfNative（Windows Terminal/WezTerm等）
    は`PhysicalDelivery`の"nonaiyo"保険喪失・再試行不能という弱点が
    最も顕在化しやすい環境。
11. **`fix-requires-evidence.md`の該当ファミリー**: 「キー選択」
    「IME actuation合流点」「物理IMEキーのSuppress/Allow配送判断」に
    該当。(a)回帰テストまたは(b)`docs/known-bugs/BUG-NNN.md`のどちらで
    満たすかを実装時に決める。

## 非スコープ

- delegate-to-open-axis一式（親指キーのdelegate、`NicolaFsm`専用
  フィールド4組、`resolve_pending_thumb_as_single`優先順位match、
  `auto_delegate_open_axis_consumed`/`explicit_ime_action_consumed`
  マーカー、`delegate_owns_mode_key_shadow_toggle`の判定ロジック自体）
  の変更・撤去。`ModeKeyActuationOwner::FsmDelegate`はこの既存ロジックを
  ラップするだけで中身は変更しない。将来の別ADR候補。
- Hiragana/Katakanaの扱い（現状のまま、ADR-135 Phase1訂正後の状態を
  変更しない）。
- F15–F24（ADR-092 Step4c）機構自体の見直し。
- 物理的にトグルなVK（`VK_DBE_SBCSCHAR`/`DBCSCHAR`等）への
  `PhysicalDelivery`概念の拡張。`ime_actuation_owned==false`
  プロファイルやInputRelayへの横取り拡張。将来の別ADR候補。
- ADR-176較正機能・ADR-174分類ロジック自体の変更。
- Eisu・Kanji（`VK_KANJI`本体）の扱いの変更。

## 領域A・Cの撤去（旧称: ADR-178撤去プロジェクト、2026-09-24追記）

本ADRは当初178番で起票したが、developにマージ済みの別ADR-178（MSIアンインストール時のユーザーデータ保持）と番号が衝突したため179へ採番し直した（[index](index.md)の179行）。番号の付け替え前に書かれたコミット本文・ADR・BUG・コードコメントの「ADR-178撤去プロジェクト」「ADR-178領域A/B/C」は、**本ADR（旧178）が起点の撤去作業**を指す。領域の内訳と撤去の事実は次のとおり（いずれも2026-09、developに含まれる）。

- **領域A（TsfNative向けON方向救済4系統〈force-on / drift / warmup / reassert〉の削減）**: ユーザー方針は「他のactuation機構がかなり残ってしまっている」ことを問題視し、drift correctionだけを残して他を撤去、実機A/Bで問題が出れば復元する、というもの（設計レビューより実機検証を優先）。
  - `f83084b3`（領域A 1/3）: reassert（[ADR-121](121-explicit-physical-ime-key-idempotent-reassert.md) D1、BUG-37対策の物理IMEキー冪等再送）を撤去。`apply_ime_open_with_view`の許可呼び出し元 4→3。
  - `621bf93c`（領域A 2/3）: force-on機構（[ADR-098](098-tsfnative-applied-confirmed-laundering-and-force-on-removal.md)決定1-c、再試行クールダウン込み、`apply_force_on_for_imm_broken`/`try_force_on_bootstrap`/`force_on_and_correct_romaji`）を撤去。`apply_ime_open_with_view`の許可呼び出し元 3→2。`ForceOnReason::BrokenAppBootstrap`のvariantは`ForceGuardSet`/`open_warrant.rs`が同じ列挙型を共有するため意図的に残した（追加する本番コードは無くなった）。
  - **warmupは撤去対象外**: 4系統に数えていたが、warmupは書き込みではなく読み取り専用のゲート（送信可否の待機）であり、「actuationを減らす」目的に合わないため対象から外した。
  - 残したdrift correctionは観測に基づくOFF方向の回復として現存する。
- **領域B**: IME actuation合流点（旧「6箇所」）の設計検討。[ADR-180](180-actuation-gate-recheck-deduplication.md)が扱う。
- **領域C**: 半角英数（ObservedEisu）検出時にawase自身がIME OFFを送っていた`EngineSync::DirectInput`の撤去。`f5338edc`、[ADR-185](185-directinput-open-axis-write-teardown.md)（BUG-146）。
- **決定2の撤去**: 本ADR決定2の`ModeKeyActuationOwner`は、ADR-191の撤去後にshadow_actionの供給元から0x1C/0x1Dが外れて`PhysicalDelivery`が到達不能になったため、`502c6673`で列挙・配線ごと撤去した。

残存する書き込み経路の棚卸し（A/B結果）は、[review-2026-09-24-09](../tasks/review-2026-09-24-09-remaining-active-writes-inventory.md)の完了後にこの節へ追記する。

## 関連

BUG-048（`docs/known-bugs/BUG-048.md`、`ActivationSync`の原因別処理分離の
起源。`src/engine/decision.rs:55-65`〈`SetOpenOrigin`のdoc〉と
`src/engine/engine.rs:444-450`〈`transition_activation`のdoc〉に経緯が
残る）、ADR-092（`classify_mode_key_ime_action`の起源）、ADR-115
（打鍵列機能）、ADR-119（IME actuation合流点は複数箇所に配線が要るという
教訓）、ADR-135（`route_thumb_key_action`・`gate_thumb_key_ime_actions`
の導入元、Phase1訂正でHiragana/Katakanaがactuation-autoから外れた経緯、
無変換/変換のADR-141 C2対策enrich override）、ADR-141（delegate/
shadow-toggleの排他性、無変換/変換のenrich override起源）、ADR-147
（thumb key delegateのuser passthrough優先）、ADR-149（`ActivationSync`
による重複送信の発見元）、ADR-153（`explicit_ime_action_consumed`の
起源、BUG-124「@」再発の警告元）、ADR-154（shadow-toggleのOFF→ON方向が
belief書き込みのみだが`ActivationSync`を誘発して結局実送信されるという
事実の確定元——本ADR決定1が非親指キーにも同じ経路を使う根拠）、
ADR-174/175（BUG-142/143、分類ロジック・Toggle解決化）、ADR-176
（較正機能、`ImeToggleKind`を消費する既存の統合点）。

なお、レビュー過程（round4/round8）で`key_pipeline.rs:1688`のコメント
（「deactivationはSetOpen(false)を生成しない」）が現在の
`transition_activation`（`engine.rs:465-475`）と矛盾していることが
判明した。本ADRのスコープとは独立したドキュメント負債として
2026-09-17に訂正済み。

---

## レビュー経緯（round1〜round8）

このADRは当初「モードキー全般を3分類（一方通行/トグル/明示config）に
整理し、能動actuationを広く撤去する」という大きな構想で起票された。
8ラウンドの`opus-adversarial-consult`を経て、実際に安全に撤去できる
範囲は「無変換/変換が非親指キー配置時のみ」まで絞り込まれた。この節は
その過程を記録する——実装や設計の再検討をする際、**同じ設計を再発見
して同じ失敗を繰り返さないための記録**である
（`.claude/rules/experiment-logging.md`の精神）。

### round1: 当初の3分類案がBlocker 3件で崩れる

「一方通行キーはfollow-onlyで安全（冪等なので観測不要）」という論証は
「物理キーが実際にGJI/IMEへ配送される」という隠れた前提に依存していた。
無変換/変換のdelegate-to-open-axis経路はこの前提を満たさず（生VKを
再送出しない）、`Effect::Ime(SetOpen)`を削除すると実IMEを動かす主体が
消える（BUG-115再発）か、生VKを再送出すると「@」再発（BUG-113）に
つながることが判明した（Blocker 1）。また撤去対象の主要2項目
（`NicolaFsm`の`henkan_vk`等の専用フィールドと優先順位match）は、
モードキーの意味論ではなく「無変換/変換がNICOLAの親指シフトキー
そのものだから」存在する、3分類とは直交する軸だった（Blocker 2）。
`ActivationSync`のorigin選別（当初案の決定3）は未解決点ではなく決定1の
成立条件であり、同時実装しない限り「actuationをやめたのに送信は
1件も減らず、発火点が移動するだけ」になることも判明した（Blocker 3）。
併せて、直前に完了したADR-176（較正機能）が撤去対象棚卸しにほぼ反映
されていなかったこと、`explicit_ime_action_consumed`マーカーが
BUG-113ケース3改の唯一の実効的なSuppress手段であり削除対象から外す
べきこと等のMust-fixも反映した。

### round2: スコープを絞ってもBlocker 1・3が再現

撤去の主戦場を「非親指キーのactuation-auto」に絞ったが、
`match_ime_on_off_auto`のマッチは`Decision::Consume`になり、
`executor.rs`のConsumeアームは`transport.rs::plan`の結果を一切参照
しないため、非親指キーでも物理キーは実際には配送されていなかった
（round1 Blocker 1と同型）。提案した`ActivationSync`選別の配線点
（`kp_stage_shadow_ime_toggle`）も、決定2が変更する`match_ime_on_
off_auto`より前に実行されるため判定結果を先読みできず、時系列的に
成立しないと判明した（Blocker 3）。round2は「決定2の対象VKを実際に
列挙し、Phase1マッチ／Decision種別／`transport.rs::plan`の結果／実配送
の有無／`shadow_action`の有無を実コードで埋めた表を作る」ことを宿題に
指定した。

### round3: 隠れた前提の実証と、新Blocker2件

宿題の調査（forkによる実コード調査）の結果、対象VK（親指キーでない
無変換/変換）で「配送されない」ことは確認どおり成立する一方、
**この2VKには既に「正しい設計」（`resolve_henkan_muhenkan_shadow_
override_for_event`、親指キー非依存で動作）が実装済みで、非親指キー
の場合だけそれを使わず別の能動actuation経路（actuation-auto）を
再発明していた**ことが判明した。これが「`route_thumb_key_action`の
`is_thumb_key`分岐を撤去し常に既存のfollow-only経路へ渡す」という
新しい決定の根拠になった。

ただし新たにBlocker 2件が判明した。`on`/`off`のVecには無変換/変換
以外にも生産者があり（GJIの専用Fnキー F15–F24検出、ADR-092 Step4c）、
`ime_on_auto`/`ime_off_auto`機構そのものを削除するとこの機能を巻き
添えにする（Blocker R3-1）。また、親指キー配置時にON→OFF実actuateが
安全に止まっている理由（`delegate_owns_mode_key_shadow_toggle`の
`is_configured_thumb_key`条件）は非親指キーには構造的に立てられず、
単純に分岐を撤去するとbelief ON中の`Off`分類無変換/変換でawase自身の
実IME-OFF送信とGJI自身の物理反応が重なる新規の二重信号を作る
（Blocker R3-2）。

最初の対応案（「非親指キーの場合だけON→OFFのactuate呼び出しをスキップ
する」条件分岐を追加する）は、**ユーザーから「個別条件を場当たり的に
追加するだけで、本ADRが撤去しようとしている対症療法のパターンをまた
繰り返すだけ」という指摘を受けた**。実装ではなく設計をやり直した結果、
`kp_stage_shadow_ime_toggle`の`delegate_owned`という1つの変数に、
本来別々の2つの問い（「beliefは誰が書くか」「実IMEの状態は誰が変える
か」）が混ざっていたことが根本原因だと判明した。

### round4: `ActivationSync`が両方向に効くという事実誤認、既に却下された設計との遭遇

「OFF方向には自動送信が無い」というADRの主張は誤りだった。引用した
コメント（`key_pipeline.rs:1688`）は「自動送信が無いからこのブロックが
必要」というブロックの存在理由の説明であり、逆の意味に読んでいた。
現在の`transition_activation`（`engine.rs:465-475`）は`NotRomajiInput`
以外の遷移では`SetOpen(false)`を発行し`ActivationSync`経由で実送信
する。つまり決定2はawase側の送信を消さず、送信元を明示actuateから
`ActivationSync`へ移すだけだった（R4-1、round1 Blocker 3と同じ失敗
パターンの3度目の再現）。

さらに重大な発見として、MS-IME側（`message_handlers.rs:1031-1064`）に
GJI側と同じ条件でoverrideをゲートするコードが既にあり、そのコメント
には「過去の`/code-review`が非親指キーにもoverrideを差す設計を試み
BUG-46型の二重actuationを発見し、このガードを追加して修正した」という
記録が残っていた（R4-2）。決定1はこの不変条件をGJI側だけで解除する
ものであり、`.claude/rules/experiment-logging.md`が警告する「なぜ前回
捨てたか分からず同じ失敗を繰り返す」パターンに該当しかねなかった。

重点質問「これは簡素化か特殊条件の追加か」への回答は「特殊条件の追加」
だった（R4-3）——`route_thumb_key_action`は単純化される一方、
`fix-requires-evidence.md`が名指しする「IME actuation合流点」に新しい
gateを追加していた。推奨代替案として、`delegate_armed`/`delegate_
owned`という2つのbooleanと新条件を1つの列挙型（`ModeKeyActuationOwner`）
にexhaustiveに畳み込む案が提示された。

ここでユーザーから2点の指摘を受けた: 「R4-2の指摘は成立しない、
actuate自体（明示actuateと`ActivationSync`の自動echoの両方）を撤去
するのと同時に進めるから」、「所有権を列挙型にするのは良いアイデア
だが、ちゃんと考えないとうまくいかない（列挙値を1箇所で計算し複数箇所
から参照する設計にしなければ、同じ配線漏れを列挙型の皮を被って繰り
返すだけになる）」。この2点を踏まえ、`ModeKeyActuationOwner`の設計
（決定2）へ収束させた。

### round5〜round8: `ModeKeyActuationOwner`の精緻化

round5で「列挙による概念削減は本物」「GJI/MS-IME対称化はおおむね
完全」と設計思想自体は肯定されたが、`ActivationSync`停止のフック点が
誤っていた（`kp_stage_post_decision`はbelief記帳のみで、実送信は
`kp_stage_execute`→`executor.rs`の無条件実行で起きる——これも
2026-07-05に一度実機で踏んだのと同じ罠だった、Blocker R5-1）。
round6でowner計算点自体の誤り（`enrich_ime_relevance`はbelief更新
ステージより前で`FsmDelegate`のライブ判定をsnapshot化してしまう、
同じ関数の直上に過去のOpusレビューが記録していた同型ハザード、
Blocker R6-1）と、`sync_direction`を主条件にするとADR-175/BUG-142の
回避策を壊す誤り（Must-fix R6-2）が見つかり訂正した。round7で
「収束と判定してよい」との評価を得て、残る2件（`:436`のフィルタに
`shadow_toggled`条件を追加し無関係な観測駆動の`ActivationSync`を
誤ってstripしない〈Must-fix R7-1〉、`sync_direction`と`shadow_action`の
方向不一致時は`NotAModeKey`へ倒す〈Should-fix R7-2〉）を反映した。
round8で「収束。Blocker・Must-fixいずれも無し。実装着手可」との
最終判定を得た。

### 総括: 削除量は縮小したが後退ではない

| | round1の当初案 | 最終案 |
|---|---|---|
| 対象 | 全モードキーの3分類 | 無変換/変換の非親指キー配置時のみ |
| 手段 | 能動actuationを広く撤去 | `route_thumb_key_action`の`is_thumb_key`分岐撤去 + `ModeKeyActuationOwner`列挙 |
| 送信停止点 | 未定（4ラウンド誤り続けた） | `key_pipeline.rs:436`のeffect除去（実証済み） |
| 削除量 | 表9項目 | 2項目（分岐+Vec引数3本、機構自体は維持） |

削除量は当初案より大幅に小さくなったが、これは後退ではない。round1〜3
で「撤去できる」と見えていた項目の大半は、実際には親指キーのチョード
判別という直交軸を担う不変条件だった。「削除できないものを削除できると
誤認したまま実装に進むよりは正直な見積りである」という、本ADRが当初
掲げた原則に、8ラウンドかけて到達した形である。

一方で`ModeKeyActuationOwner`列挙は、削除量には表れないが実質的な
簡素化である——2つのboolean（4状態、1つは無意味）を4状態のexhaustive
enumに畳み込み、将来ケース追加時の配線漏れをコンパイラが検出する構造
にした。ADR-119が「合流点への追加は複数箇所に配線が要る」と警告し、
`fix-requires-evidence.md`が再発ファミリーとして記録してきた問題に
対する、散文でもテキスト検査でもない初めての構造的対策であり、本ADRの
最大の成果はここにあると評価された（round8総括）。

8ラウンドを通じて実コード上確定した重要な事実（非親指キーの無変換/
変換が`Decision::PassThrough`になり実際に配送されること、
`ActivationSync`の実送信箇所、`transition_activation`が遷移方向を
問わず`SetOpen`を発行すること、`delegate_owned`がライブ値であること）
は、本ADRの可否を超えてこのリポジトリの資産として記録されている
（各事実の詳細は上記の決定・安全性の各節を参照）。
