---
id: ADR-178
title: |-
  無変換/変換の非親指キー時actuation-autoを撤去し、親指キー時と同じ
  follow-only経路（shadow_action override）へ一本化する
summary: |-
  主目的は新機構の導入ではなく、モードキー関連のBUG対応(BUG-115/113/123/124/142/143)
  のたびに個別追加されてきた対症療法の撤去。opus-adversarial-consult round1でBlocker3件
  （follow-onlyの冪等性論証の隠れた前提「物理キーが実IMEに届く」がdelegate経路では偽／
  撤去対象の主要2項目が親指キーのチョード判別という直交軸で撤去不可能／決定3
  〈ActivationSyncのorigin選別〉は決定1の成立条件）、round2で残り2件が新スコープでも
  未解決と判定された後、round2の宿題（対象VKの実態表）をfork調査で実施した結果、
  **無変換/変換には既に「親指キー配置時のfollow-only経路」(`resolve_henkan_muhenkan_
  shadow_override_for_event`、親指キー非依存で動くよう実装済み)が存在し、非親指キー
  配置時だけがこれを使わず`route_thumb_key_action`の`is_thumb_key`分岐で別の能動
  actuation経路(`ime_on_auto`/`ime_off_auto`)を再発明していたと判明**。決定3
  （ActivationSyncのorigin選別）は不要と判明し撤回、代わりに`is_thumb_key`分岐の撤去
  という、より単純かつ削除量の大きい設計に転換した。round3レビュー待ち。
status: |-
  **round1・round2完了（opus-adversarial-consult、round2でBlocker 1/3が新スコープでも
  未解決と判定）。round2の宿題（対象VK実態表）をfork調査で実施し、`route_thumb_key_
  action`の`is_thumb_key`分岐撤去という新設計へ転換した。round3（同一レビュアーへの
  再確認）待ち。実装未着手。**
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

# ADR-178: 無変換/変換の非親指キー時actuation-autoを撤去し、親指キー時と同じfollow-only経路へ一本化する

## 主目的（誤解しないこと）

**本ADRの主目的は、新しい機構（follow-only/横取りの3分類）を導入することそれ
自体ではない。** 主目的は、モードキー周りのBUG対応（BUG-115/BUG-113/BUG-123/
BUG-124/BUG-142/BUG-143 等）のたびに**あてずっぽうで積み増されてきた個別対症
療法**を**撤去**し、単純で全体の見通しの良い設計に戻すことである。3分類は、
撤去した後に残る「これなら1つの原則で全部説明できる」という**結果**であって、
目的ではない。

**round1レビュー（下記「round1レビューの結論」参照）を経て、この主目的を
実際に安全に達成できる範囲は当初の想定より狭いことが判明した。** 無変換/
変換・ひらがな/カタカナが**親指シフトキーとして設定されている場合**の
delegate-to-open-axis機構は、モードキーのIME意味論（一方通行/トグル）とは
**直交する別の軸**（NICOLAのチョード＝同時打鍵の起点か単独タップかを判別する
タイミング制御）を担っており、これを崩すとBUG-115型の機能不全かBUG-113型の
「@」再発のどちらかを必ず作る。そのため**本ADRは撤去の主戦場を「親指キーで
ない場合」の機構（actuation-auto）に絞り、親指キー側（delegate-to-open-axis）
は本ADRのスコープ外と明示的に宣言する**（別ADR候補として将来検討）。

成功基準は「3分類を実装したか」ではなく、下記「撤去対象」に列挙した個別
機構・専用フィールドが実際にどれだけ削除できたかである。範囲を絞った結果、
削除できる量そのものは当初案より小さくなるが、**「削除できないものを削除
できると誤認したまま実装に進む」よりは正直な見積りである**。

## round1レビューの結論（要約、全文はレビュー担当者の記録参照）

opus-adversarial-consultによる読み取り専用レビューで、以下3件のBlockerが
確定した。

- **Blocker 1**: 「一方通行キーはfollow-onlyで安全（冪等なので観測不要）」
  という論証は、「物理キーが実際にGJI/IMEへ配送される」という**隠れた前提**
  に依存する。無変換/変換のdelegate-to-open-axis経路（`src/engine/
  nicola_fsm.rs`の`resolve_pending_thumb_as_single`内`delegate_to_open_axis`
  分岐）はこの前提を満たさない——生VKを一切再送出せず（`actions: SmallVec::
  new()`）、実際にIMEを動かしているのはawase自身の`Effect::Ime(SetOpen)`
  だけである。この`SetOpen`発行を削除すると実IMEを動かす主体が消え
  （BUG-115再発）、代わりに生VKを再送出する設計に変えると
  `crates/awase-windows/src/runtime/transport.rs`が明示的に警告する
  「GJIのTSFキー横取りが『@』を誘発する」経路（BUG-113の根本原因）を踏む。
- **Blocker 2**: 撤去対象の主要2項目（`NicolaFsm`の`henkan_vk`等の専用
  フィールド4組と`resolve_pending_thumb_as_single`の優先順位match）は、
  「モードキーだから」ではなく「**無変換/変換等がNICOLAの親指シフトキー
  そのものだから**」存在する——チョード起点か単独タップ確定かを判別する
  タイミング制御そのものであり、follow-only化しても分岐自体は残る（出力が
  `SetOpen`かbelief書き込みかが変わるだけ）。3分類とは直交する別軸のため、
  この2項目は本ADRの成功基準（削除できたか）の対象から外す。
- **Blocker 3**: `ActivationSync`（`handle_engine_activation_sync`）の
  origin選別（当初案の決定3）は「未解決点」ではなく決定1の**成立条件**
  である。`kp_stage_shadow_ime_toggle`のbelief書き込み→
  `Engine::check_active_transition`→`ActivationSync`→実`SendInput`という
  連鎖が既にあり（ADR-154既知の事実）、origin選別を同時実装しない限り
  「actuationをやめたのに送信は1件も減らず、発火点が移動するだけ」になる。

Must-fix 7件のうち特に設計に影響したもの:

- 撤去対象棚卸しに、直近（本ADR起草の前日）に完了したばかりのADR-176
  （較正機能）由来の機構がほぼ反映されていなかった。ADR-176は
  `gate_thumb_key_ime_actions`の出力を差し替える形で実装されており、
  「撤去対象の総量より直近追加された機構の総量の方が大きい」状態だった。
  → 本ADRはADR-176を撤去対象にせず、3分類への**入力**として明示的に
  維持する（下記「ADR-176・ADR-174との関係」参照）。
- Toggle方向（半角/全角等）のOFF→ON実送信も、専用のactuationコードが
  存在せず`ActivationSync`頼みだった。「Toggleは能動actuateのまま残す」
  「ActivationSyncは選別で止める」を同時に主張すると自己矛盾する。
  → 本ADRはToggle方向の挙動を変更しない（下記「決定2」の適用範囲外と
  明記）。
- `transport.rs::plan`の`explicit_ime_action_consumed`マーカーは、
  BUG-113ケース3改の「@」再発に対する**唯一の実効的なSuppress手段**
  であり、delegate側の能動actuationとは独立の役割を持つ。**削除対象
  から外す**。
- ADR-175（BUG-142、半角/全角の固定方向マッピングをToggleへ変更）は
  「収束・実装着手可」であって未実装（`fix_commits: []`）。本ADRの決定は
  ADR-175実装後を前提にする。

Should-fix・訂正事項も反映済み（`gate_thumb_key_ime_actions`の導入元は
ADR-176ではなくADR-135 C1/BUG-115、`LAST_MODE_KEY_THUMB_WARNING`は
ADR-164フェーズ1で`gji_mode_key_thumb_warning_declined`に改名済みで現存、
`ActivationSync`の起源は「不明」ではなくBUG-48/"nonaiyo"対策
（`src/engine/engine.rs::transition_activation`のdoc参照）——これらは
下記「撤去対象」表に反映済み）。

## round2レビューの結論（要約）

round1反映後の再確認で、13件中10件は適切に解決したと判定されたが、
**Blocker 1・Blocker 3が未解決**と判定された。

- **Blocker 1は「位置が移動しただけ」**: スコープを非親指キーの
  actuation-autoに絞っても、`match_ime_on_off_auto`のマッチは
  `Decision::consumed_with`（`Engine::build_ime_set_open_decision`、
  `engine.rs:899-901`）になり、`executor.rs`の`Decision::Consume`アームは
  `physical`（transport.rs::planの結果）を一切参照しない。つまり
  **非親指キーのactuation-autoでも、物理キーは実際にはGJIへ配送されて
  いない**——round1 Blocker 1と完全に同型の構図が、スコープを絞っても
  再現する。
- **Blocker 3の配線は時系列的に成立しない**: 提案した`ImeRelevance`
  フラグの設置点（`kp_stage_shadow_ime_toggle`、`key_pipeline.rs:328`）は
  `engine.on_input`（決定2が変更する`match_ime_on_off_auto`の内部、
  `:426`）より**前**に実行されるため、判定結果を先読みできない。
  加えて`handle_engine_activation_sync`の呼び出し元2箇所目
  （`kp_apply_conv_engine_sync`、`:1162`）には`RawKeyEvent`自体が
  存在せず、提案したフラグを参照する術がない。さらに、そもそも
  actuation-autoはEngineのPhase 1で早期returnするため
  `check_active_transition`（`ActivationSync`の発火点）に到達しない
  可能性が高く、**決定3自体が新スコープでは不要かもしれない**、との
  指摘も受けた。

round2は「決定2の対象VKを実際に列挙し、Phase1マッチ／Decision種別／
`transport.rs::plan`の結果／実配送の有無／`shadow_action`の有無を
実コードで埋めた表を作る」ことを次の宿題として指定した。

## 宿題への回答（実コード調査、round3向け）

対象VKを実際に列挙した結果、**`ime_on_auto`/`ime_off_auto`に乗るのは
「親指キーとして設定されていない無変換/変換（`VK_CONVERT`/
`VK_NONCONVERT`）」だけ**であることが判明した
（`route_thumb_key_action`の本番呼び出しは`gji_charset_autodetect.rs:851`
〈Henkan〉・`:860`〈Muhenkan〉の2箇所のみ。Hiragana/Katakanaは
ADR-135「Phase 1の訂正」で該当ループごと削除済みのためactuation-auto
には現在乗らない。MS-IME側`sync_ime_toggle_auto_detect`は`ime_toggle_auto`
のみを設定し`ime_on_auto`/`ime_off_auto`には触れない）。

この2VKについて、**round2 Blocker 1の「配送されない」は確認どおり成立
する**（`match_ime_on_off_auto`マッチ→`Decision::Consume`）。しかし
調査の過程で、**この2VKには既に「正しい設計」が実装済みであり、
非親指キーの場合だけそれを使わず別の機構（actuation-auto）を
再発明していた**ことが判明した。これが以下の新しい決定の根拠になる。

- `gji_charset_autodetect.rs:591`の`resolve_henkan_muhenkan_shadow_
  override_for_event`は、`event.ime_relevance.shadow_action`を
  **親指キーかどうかに関わらず**無条件に設定できる（Hiragana/Katakana版
  と異なり「親指キーならNone」の早期returnを行わない）。doc comment
  （`:585-588`）に明記: 「無変換/変換には`ImeKeyKind::from_vk`由来の
  守るべき静的`shadow_action`が存在しないため、親指キーとして設定
  されている場合でもoverrideを差してよい」。
- この関数を呼ぶ`Runtime::enrich_ime_relevance`（`runtime/mod.rs:581`）
  も無条件に呼ばれる。入力は`Runtime.henkan_shadow_override`/
  `muhenkan_shadow_override`（`runtime/mod.rs:332-333`）で、これが
  `None`なら単に何もしない。
- **`route_thumb_key_action`（`gji_charset_autodetect.rs:625`）が
  `is_thumb_key`で分岐し、`true`のときだけこの`henkan_shadow_override`/
  `muhenkan_shadow_override`へ値を渡し、`false`のときは代わりに
  `on`/`off`のVec（actuation-autoの元データ）へ値を積んで`None`を返す**
  ——これが非親指キーの場合にshadow_action機構が使われない直接の原因。
- `transport.rs::plan`のVK_CONVERT/VK_NONCONVERT専用分岐
  （`:341-372`、ADR-141 C2対策として既に存在）は、`explicit_ime_action_
  consumed`が立っていない限り**無条件Allow**——コメントには「無変換/
  変換は既定ではawase自身がactuationを所有する対象ではない…OS側の
  実際の切替はGJI自身が物理キー配送を通じて行う」と明記されている。
  **この分岐は、非親指キーの無変換/変換が物理配送される前提で最初から
  書かれている。** 現状これが機能していないのは、非親指キーの無変換/
  変換がEngine Phase1のactuation-autoマッチで`Decision::Consume`に
  乗ってしまい、この`plan`の判定へ到達する前に打ち切られているため。

つまり、**「follow-onlyかつ物理配送される」という決定1の条件を満たす
経路は、新規に作る必要がなく、`route_thumb_key_action`の`is_thumb_key`
分岐を撤去して常に`henkan_shadow_override`/`muhenkan_shadow_override`
経由にするだけで到達できる**。これにより:

- `kp_stage_shadow_ime_toggle`が非親指キーの無変換/変換にもbelief
  書き込みを行うようになる（親指キーの場合と全く同じ既存経路）。
- Engine Phase1の`match_ime_on_off_auto`はこのVKにマッチしなくなり
  （`ime_on_auto`/`ime_off_auto`へ何も積まれなくなるため）、
  `Decision::Consume`を生成しなくなる。
- 親指キー分類（`KeyClassification::LeftThumb`/`RightThumb`）も
  受けないため（親指キー設定がこのVKを指していない以上、hook.rs側で
  そもそも付与されない）、NicolaFsmの通常経路（未確認、下記「未解決点」
  参照）へ落ち、最終的に`transport.rs::plan`のAllow判定が実際に効く
  見込みが高い。
- **`ActivationSync`のorigin選別（round1/round2の決定3）は不要になる。**
  非親指キーの無変換/変換が親指キーの場合と同じ`shadow_action`経路を
  使うようになる以上、そのOFF→ON方向の実送信も、親指キー配置時に
  既に受け入れられている経路（belief書き込み→`check_active_transition`
  →`ActivationSync`→実送信、ADR-154確認済み）をそのまま使えばよい。
  これは「新設のfollow-only書き込みでもActivationSyncが誤発火する」
  問題ではなく、「親指キー配置時に既に動いている、実績のある経路を
  非親指キーにも使う」だけであり、`ActivationSync`を選別で止める理由が
  そもそも無くなる。

## 撤去対象（本ADRのスコープ内で実際に撤去するもの）

| 機構 | 導入元 | 何のための対症療法だったか | 撤去方法 |
|---|---|---|---|
| `route_thumb_key_action`の`is_thumb_key`分岐（非親指キー側の`on`/`off`/`toggle`Vec push） | ADR-135 Phase1（BUG-115 F7） | 無変換/変換が親指キーでない場合に、既にある`shadow_action`follow-only機構（親指キー側で使われているもの）を使わず、別の能動actuation経路を新設した | 分岐を撤去し、親指キー時と同じく常に`henkan_shadow_override`/`muhenkan_shadow_override`へ値を渡す一本化した経路にする |
| `Engine`の`ime_on_auto`/`ime_off_auto`フィールド、`set_ime_on_auto_keys`/`set_ime_off_auto_keys`、`match_ime_on_off_auto`のOn/Off arm | ADR-135 Phase1/2 | 上記分岐の消費先。無変換/変換のOn/Off検出値を能動actuateする専用経路として追加された | 上記の一本化により入力が来なくなるため、フィールド・setter・matchアームごと削除する（`ime_toggle_auto`はMS-IME側のCtrl+Space/Shift+Space検出に引き続き使うため維持） |

`ActivationSync`のorigin選別（旧決定3）は「宿題への回答」のとおり
不要と判明したため、撤去対象からもスコープからも削除する。

## スコープ外と明示的に宣言する隣接機構（撤去しない・変更しない）

- **delegate-to-open-axis一式**: `NicolaFsm`の`henkan_vk`/`muhenkan_vk`/
  `hiragana_vk`/`katakana_vk`と対応する`*_delegate_to_open_axis`フィールド、
  `resolve_pending_thumb_as_single`の優先順位match、`auto_delegate_open_axis_
  consumed`マーカー（ADR-154）、`explicit_ime_action_consumed`マーカー
  （ADR-153）、`delegate_owns_mode_key_shadow_toggle`/
  `mode_key_delegate_owns_shadow_toggle`の排他性機構（ADR-141/154）。
  親指キー設定時の挙動・タイミング制御は一切変更しない——本ADRが変える
  のは「親指キーでない場合にどの経路を使うか」だけである。
- **Hiragana/Katakana**: ADR-135 Phase1の訂正で既にactuation-autoから
  外れており、本ADRの対象外（現状の`shadow_action`静的マップ＋Phase2
  オーバーライドのまま変更しない）。
- **Toggle方向のactuation実装**: `VK_DBE_SBCSCHAR`/`DBCSCHAR`等、
  物理的にトグルなVKは対象外のまま。主要プロファイル（GjiDirect/
  MsImeDirect）では`transport.rs::plan`の`is_dbe_mode_key_down`により
  既に無条件Suppress済み（横取り実装済み）。`ime_actuation_owned==false`
  プロファイル・`InputRelay`への拡張は将来の別ADR候補。
  無変換/変換が`ImeToggleKind::Toggle`と分類された場合の扱いは
  未解決点2参照（一本化した経路に自然に乗る可能性が高いが要確認）。

## ADR-176・ADR-174との関係（撤去対象ではなく入力として維持）

- **ADR-176（較正機能）**: `state/calibrated_mode_key.rs`一式、
  `gate_thumb_key_ime_actions`出力の2箇所差し替え配線、`[[calibration]]`
  config、fingerprint/stale基盤、bypass機構等は**撤去対象ではない**。
  較正結果は最終的に`ImeToggleKind`（On/Off/Toggle）という同じ語彙で
  `route_thumb_key_action`/actuation-autoの入力になるため、本ADRの
  3分類はこの出力をそのまま消費すればよい。ADR-176は「静的分類が
  信用できないときに実測で補正する」という別レイヤーの関心事であり、
  本ADRが扱う「分類が決まった後、awaseは能動actuateすべきか」という
  問いとは独立している。
- **ADR-174（`classify_mode_key_ime_action`の`custom_keymap_table`
  優先フォールスルー）**: 分類ロジックの一部であり、本ADRが扱う
  actuation側の問題とは独立。維持する。

## 目的

「撤去対象」に列挙した機構を撤去した跡地に、単一の原則を置く。

> **無変換/変換のGJI検出値（On/Off分類）は、親指キーとして設定されて
> いるかどうかに関わらず、常に同じ経路（`shadow_action`のfollow-only
> 書き込み＋物理キーのpassthrough）で扱う。actuation-autoという専用の
> 能動actuation経路を持たない。**

これは新しい原則の発明ではなく、**親指キー配置時に既に実装・検証済みの
経路を、非親指キー配置時にも一貫して使う**という単純化である。

## 決定

### 決定1: `route_thumb_key_action`の`is_thumb_key`分岐を撤去し、経路を一本化する

`route_thumb_key_action`（`gji_charset_autodetect.rs:625`）から
`is_thumb_key`引数と、それによる分岐（非親指キー時に`on`/`off`/`toggle`
のVecへpushする経路）を削除する。分類結果は親指キーかどうかに関わらず
常に`ime_toggle_kind_to_shadow_action`を経由し、`henkan_shadow_override`/
`muhenkan_shadow_override`（`Runtime`フィールド）へ渡す。

この結果、`gji_charset_autodetect.rs:591`の`resolve_henkan_muhenkan_
shadow_override_for_event`（既に親指キー非依存で動くよう実装済み）が
非親指キーの無変換/変換に対しても`event.ime_relevance.shadow_action`を
設定するようになり、`kp_stage_shadow_ime_toggle`が親指キーの場合と
全く同じ経路でbelief書き込みを行う。

### 決定2: `ime_on_auto`/`ime_off_auto`機構を削除する

決定1により`on`/`off`のVecへ値が積まれなくなるため、以下を削除する:

- `Engine`の`ime_on_auto`/`ime_off_auto`フィールド
- `Engine::set_ime_on_auto_keys`/`set_ime_off_auto_keys`
- `Engine::match_ime_on_off_auto`のOn/Off arm（関数自体は`ime_toggle_auto`
  参照が無くなる場合のみ削除、MS-IME側の`ime_toggle_auto`は別経路
  〈Ctrl+Space/Shift+Space〉のため維持）
- 呼び出し元（`runtime/mod.rs`、GJI検出結果を`set_ime_on_auto_keys`等へ
  渡していた配線）

Hiragana/Katakanaはこの機構に現在乗っていない（ADR-135 Phase1訂正で
既に除外済み）ため、削除の影響は無変換/変換のみに限定される。

## なぜこれで安全か

- **Blocker 1（隠れた前提「物理キーが実際に配送される」）が実際に成立
  する**: 一本化した経路は、親指キー配置時に既に実機で使われている
  経路と同一であり、`transport.rs::plan`のVK_CONVERT/VK_NONCONVERT
  専用分岐（`explicit_ime_action_consumed`が無ければ無条件Allow）は
  この配送を前提に既に書かれている。新しい配送経路を作る必要が無い。
- **Blocker 3（決定3が送信数を減らさない）は、決定3自体が不要になった
  ことで解消する**: 非親指キーのOFF→ON実送信は、親指キー配置時と同じ
  `check_active_transition`→`ActivationSync`経由の実送信になる。これは
  「新設のfollow-only書き込みが誤って`ActivationSync`を誘発する」問題
  ではなく、「親指キー配置時に既に受け入れられている実送信経路を、
  非親指キーにも一貫して使う」だけである。BUG-48/"nonaiyo"保険機能は
  一切変更しない。
- **削除量が当初案より増える**: `Effect::Ime(SetOpen)`の発行1箇所を
  削るだけでなく、`ime_on_auto`/`ime_off_auto`という専用フィールド・
  setter・matchアーム・呼び出し配線一式と、`route_thumb_key_action`の
  `is_thumb_key`分岐そのものが消える。

## 未解決点（実装設計で詰める、round3で最終確認が必要）

1. **【最重要・未確認】非親指キー・非特殊キーのVK_CONVERT/VK_NONCONVERT
   に対するNicolaFsmの最終Decisionが本当に`PassThrough`になるか**:
   `KeyClassification`がLeftThumb/RightThumbでない（親指キー設定が
   このVKを指さない）場合、`resolve_pending_thumb_as_single`等の
   親指キー専用分岐には到達しないはずだが、他のFSM分岐
   （`decide_idle`等、`src/engine/nicola_fsm.rs:1394`の`on_key_down`
   以降）がこのVKを誤って捕捉しないかを、実コードを最後まで追って
   確認する必要がある。ここが`PassThrough`でなければ、決定1の
   前提そのものが崩れる。
2. **`ImeToggleKind::Toggle`分類時の扱い**: 決定1の一本化後、無変換/
   変換が`Toggle`と分類された場合（ATOKプリセット等）も
   `henkan_shadow_override`/`muhenkan_shadow_override`へ`ShadowImeAction::
   Toggle`として渡ることになる。これは親指キー配置時に既にToggleが
   通る経路と同じであり新規リスクではないはずだが、非親指キー・
   Toggle・opt-in（`gji_thumb_key_ime_toggle`）ゲートの相互作用を
   実装時に確認する。
3. **eisu救済の対称性**: 決定1が使う経路は親指キー配置時と同一の
   `kp_stage_shadow_ime_toggle`→`check_active_transition`→
   `ActivationSync`であるため、`.claude/rules/ime-belief-architecture.md`
   の「user IME-ON経路とObservedEisu救済の対称性」は**新規配線ではなく
   既存のペアがそのまま非親指キーにも適用される**可能性が高い。ただし
   `state/eisu_recovery.rs`のSSOT対応表が「親指キー時のみ」を前提にした
   記述になっていないか確認し、必要なら対応表の記述を更新する
   （新しい救済コードの追加は不要なはず）。
4. **ADR-175（BUG-142）実装との順序**: ADR-175は未実装
   （`fix_commits: []`）。半角/全角は`ImeKeyKind::from_vk`の静的マップ
   対象でありこのADRの対象（無変換/変換）には含まれないため、実装順序
   に依存しない。
5. **回帰テストへの影響**: `ime_key_sequence_golden.rs`・
   `golden_scenarios.rs`・`architecture_guard.rs`のうち、非親指キー
   無変換/変換のactuation-auto経由の能動actuationを前提にした期待値
   （あれば）を洗い出し、shadow_action経由のfollow-onlyへ変更した
   挙動に合わせて更新する。
6. **`fix-requires-evidence.md`の該当ファミリー**: 「キー選択」
   「IME actuation合流点」に該当。(a)回帰テストまたは(b)
   `docs/known-bugs/BUG-NNN.md`のどちらで満たすかを実装時に決める。

## 非スコープ

- delegate-to-open-axis一式（親指キーのdelegate、`NicolaFsm`専用
  フィールド4組、`resolve_pending_thumb_as_single`優先順位match、
  `auto_delegate_open_axis_consumed`/`explicit_ime_action_consumed`
  マーカー）の変更・撤去。将来の別ADR候補。
- Hiragana/Katakanaの扱い（現状のまま、ADR-135 Phase1訂正後の状態を
  変更しない）。
- Toggle方向actuationの新規実装・`ime_actuation_owned==false`
  プロファイルやInputRelayへの横取り拡張。将来の別ADR候補。
- ADR-176較正機能・ADR-174分類ロジック自体の変更。
- Eisu・Kanji（`VK_KANJI`本体）の扱いの変更。

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
