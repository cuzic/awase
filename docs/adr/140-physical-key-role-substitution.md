# ADR-140: 物理キー役割代入（Physical Key Role Substitution）基盤

## ステータス

**r3（Opus 2体の敵対的レビューを3ラウンド実施。r2の改訂作業で新たに混入した
Blocker/Majorを反映。r4レビュー未実施）。**

本ADRは、変換/無変換/スペースキーを相互に入れ替える秀Caps相当の機能
（ユーザー要望、2026-09-05、Alt系は対象外と決定済み）を実現するための**基盤**
のみを扱う。実際のconfig形式・設定GUIは後続ADR（Phase B、本ADRマージ後に着手）
で扱う。

**r0→r1でスコープが変わった**: 当初対象に含めていた「ひらがな」キーは、
r1レビューで前提が崩れたため**Phase Aの対象から除外**した（決定0参照）。
ユーザーへの確認が必要な変更点であり、本ADRを承認する前にこの縮小を
確認すること。

**r1→r2で決定1・2・4・6・7を修正した**: r1で新設した「`from`/`to`が親指キーvk
と一致する組み合わせを全面禁止する」決定4が、既定config（`left_thumb_key=無変換`/
`right_thumb_key=変換`）では**6通りのルール全てを弾いてしまい機能が有効化
できない**という致命的な欠陥をr2レビューで指摘された。正しい解は「個別の
衝突禁止」ではなく「ルール集合が対象3キー上の**全単射（置換）**であること」
と「Alt impersonationの出力には役割代入を適用しないこと」の2条件であり、
これによりほとんどの衝突は構造的に発生しなくなる。詳細は決定1・決定4参照。

**r2→r3で決定1・4・6・7・8を修正した**: r2の改訂作業自体が新たにBlockerを
3件混入させていた——(1)決定1-1の判定条件`rewritten_vk == vk`が挿入点では
恒真になり無意味だった（正しくはAlt適用前vkを退避して比較、かつ
`is_alt_impersonation_active()`というグローバルラッチを条件に使っては
ならない）、(2)決定7-3のKeyUp注入を`reinject()`で行うとしていたが、これは
フックスレッドからの呼び出しでありメインスレッド専用という安全契約に
違反する（正しくは`inject_alt_menu_mask()`と同じ`send_input_safe`直接
呼び出し）、(3)決定8の本文がまるごと脱落し参照だけが残っていた（編集
事故）。加えて、他プロセスが`LLKHF_INJECTED`でリレーした入力への適用可否
（`!is_injected`ガード）が未決定だった点、Altセンチネルを左右両方使う
構成では不動点制約により本機能が構造的に使用不可になる点、を新たに決定に
追加した。

### レビュー指摘との対応表（r1〜r3の参照解決用）

architect役: B1→背景/決定3、B2→決定0、B3→決定0（`to=VK_KANA`の不可逆破壊を
決定へ格上げ、旧決定5とは別項目）、B4→決定7、B5→決定2、B6/N9→スコープ節、
B7→決定6、B8→決定8、B9/B10→r1決定4（r2で決定1/4に再編）、R1/R2→決定7、
R3→決定1/決定4、R4→決定6、S1→決定8（r2で本文脱落、r3で復活）、
S2→決定7-3、S3→決定1-1、S4/S5→決定4末尾。
premortem役: B1→決定0、B2→決定0で非該当化、B3→決定6、B4/B5→決定7、
B6/B7→r1決定4（r2で決定1/4に再編）、N1→決定1/決定4、N2→決定2、
W7/W8→決定7、W9→決定6、N3→決定1（`!is_injected`ガード追加）、
N4→決定8（S1と同一指摘、独立に到達）。

## 背景

### 過去2回の関連機構がなぜ撤回・却下されたか

- **ADR-110/111**（フックベースの汎用物理キーリマップ`key_remap`）: CapsLock/英数
  位置キー(0x3A)に限定して実装・マージ後、(a) このキーの物理ロック状態がフック
  より低いレイヤーでOSに管理されている、(b) Shift/Ctrl/Alt+CapsLockが日本語IME
  自身のグローバル入力方式切替ショートカットである（PowerToys KeyboardManager
  Issue #3397/#32344と同型）、という**このキー固有の事情**に加えて、(c)
  実装直後のroundレビューでlatchライフサイクルのblockingな穴が3件見つかり
  （BUG-100）hook.rs側の複雑度・stuck modifierのリスクが実感されたこと、
  (d) 将来「アプリケーションごとに動的にキー割当てを変更する」機能で
  本機構自体が置き換えられる見込みとなったこと、の4点が撤回理由だった
  （`docs/adr/111-caps-eisu-ctrl-swap-preset.md`背景1-6）。対象を変換/無変換/
  スペースに絞る本ADRには(a)(b)は当てはまらないが、**(c)と同型のリスクは
  本ADRにも存在する**——決定4・決定7はこれへの直接の応答である。
- **ADR-130**（`[[keymap]]`の`to`にIME制御VKを許可する案）: `send_keymap_target`
  が`SendInput`+`INJECTED_MARKER`で後段（engineを経由せず）注入するため、
  `hook.rs::is_self_injected`の早期returnでImeModelのbelief更新がスキップされ、
  実IME状態とbeliefが乖離するという**実装方式固有の欠陥**により却下された。
  「対象VKを送ること自体」が不可能なのではなく、「engineを経由せず後段で
  SendInputにより注入する」という選んだ実装方式がこの欠陥を持っていた
  （正しい読み方は決定3参照）。

### 既存の`state/alt_impersonation.rs`が示す、第三の実装方式

`hook_callback`（`crates/awase-windows/src/hook.rs`）は、Ctrl消費追跡・親指キー
押下時刻追跡・`classify_key`より**前**の地点（`hook.rs:1107-1135`、実際の
書き換えは1127行目）で、Altなりすまし判定の結果をローカル変数`vk`へ直接
書き込む（`vk = rewritten_vk;`、1135行目）。以後の全パイプライン
（`classify_key`、`build_raw_key_event`が構築する`RawKeyEvent`、これが
そのまま`hook_channel::HOOK_KEYS`経由でengine threadへ渡る）は、この
**書き換え後のvkだけ**を見る。

`hook_callback`は（Alt絡みかどうかに関わらず）通常時は常に`LRESULT(1)`で
元イベントを握りつぶす（`hook.rs:1199`）。実際のOSへの出力は、
`Decision::PassThrough`/`Effect::Input(InputEffect::ReinjectKey(RawKeyEvent))`
という、あらゆる物理キー入力が通る唯一のRelay経路（`runtime/executor.rs`）
を通り、最終的に`RawKeyEventExt::reinject()`（`crates/awase-windows/src/lib.rs:358-373`）
が`SendInput`（`wScan: 0`固定、`dwExtraInfo`に`INJECTED_MARKER`付き）を呼ぶ。

これはADR-130と同じくSendInput+`INJECTED_MARKER`による注入だが、本質的な
違いは注入の有無ではなく**belief更新のタイミング**である: 役割代入後のvkは
`classify_key`（hook.rs:1171）→`build_raw_key_event`（1184）→ engine →
`kp_stage_shadow_ime_toggle`（`runtime/key_pipeline.rs:1008`）という、
ImeModelのbelief更新を含む通常のパイプラインを**通ってから**reinjectされる。
reinject後にフックへ戻ってきたイベントは`hook.rs:935`の`is_self_injected`
早期returnで捨てられるが、その時点でbeliefは既に更新済みなので乖離しない。
ADR-130の`send_keymap_target`はこの経路を通らず、engineを経由せず直接
SendInputするためbeliefが更新されないまま実IME状態だけが変わっていた。

### Alt限定の複雑さは、Alt/Win固有であり対象VKには存在しない

`is_alt_impersonation_active()`のdoc（`hook.rs:570-585`）が示す通り、Alt
なりすましが本当に難しかったのは「Altは`GetAsyncKeyState`で他のあらゆるコード
（`read_os_modifiers()`、`is_os_modifier_held()`によるbypass判定等）から
vkと無関係に直接読まれるOSモディファイアである」という、Alt/Win固有の事情
だった。変換・無変換・スペースはいずれもモディファイアではない通常のVKで
あり、`read_os_modifiers()`の対象にもならない。

## 決定0: Phase Aのスコープを「安全な3キー」に縮小し、ひらがな/かな系は次ADRへ持ち越す

r0は対象VKに`VK_KANA`（ひらがな/かな, 0x15）を含めていたが、r1レビューで
以下が判明し、この定義自体が誤りだったと確定した:

- JIS配列の物理「ひらがな」キー（scan 0x70）は、`VK_KANA`(0x15)としては
  ほぼ届かない。実際にはOSのレイアウト変換層とIMEの現在状態に応じて
  `VK_DBE_HIRAGANA`(0xF2)/`VK_DBE_KATAKANA`(0xF1)/`VK_DBE_ALPHANUMERIC`(0xF0)
  のいずれかとして届く。一次証拠は`docs/known-bugs.md:7301-7305`
  （「原因（確定）: NICOLAの物理『IME ON』キーはscan 0x70……Windowsの
  キーボードレイアウト変換層が`VK_DBE_HIRAGANA`(0xF2)ではなく
  `VK_DBE_KATAKANA`(0xF1)を生成することがある」）と`:15195-15199`
  （BUG-116/ADR-137、Shift併用時の0xF1生成）——ADR-133実機検証
  （2026-09-05）は補足証拠であり、その記述自体は「`VK_KANA`が届く可能性を
  排除できていない」という留保付きなので単独では根拠として弱い（r1レビュー
  RN2で訂正）。
- `crates/awase-settings/src/main.rs:3920-3936`の`THUMB_KEY_OPTIONS`も
  「かな」(`VK_KANA`)・「カタカナ」(`VK_DBE_KATAKANA`)・「ひらがな」
  (`VK_DBE_HIRAGANA`)を別項目として区別している。
- `VK_DBE_*`一族（0xF0-0xF6）は`runtime/transport.rs::plan`のSuppress/Allow
  判定（BUG-52/116）、`runtime/executor.rs`のTSF F2特別扱い、
  `CACHED_SWALLOW_ALT_KANA_MODE_SWITCH`による0xF5/0xF6の常時swallow
  （BUG-08/61/62対策）など、既存のIME actuationロジックに深く組み込まれて
  いる。この一族を役割代入の対象にするなら、既存ガードとの相互作用を
  専用に検証する必要があり、「変換/無変換/スペースの入れ替え」という
  比較的単純な機能と同じADRで一度に決めるべきではない。
- 特に`to`が`VK_KANA`または`VK_DBE_*`になる代入は、Alt押下の有無に関わらず
  BUG-61（JISかな直接入力への復旧不能な切替）を誘発しうることがr1レビュー
  で判明した。

**決定**: Phase Aの対象VKを`VK_CONVERT`（変換, 0x1C）・`VK_NONCONVERT`
（無変換, 0x1D）・`VK_SPACE`の3キー（以下「安全な3キー」）に縮小する。
ひらがな/かなキー（`VK_KANA`および`VK_DBE_*`一族）を役割代入の対象にする
機能は、既存のIME actuationガードとの相互作用を専門に扱う別ADR（本ADRの
後続、暫定的に「ADR-14x: かな系キーの役割代入」と呼ぶ）に切り出す。

対象を3キーに縮小した副次効果として、旧決定5（BUG-62ガードとの順序問題）は
Phase Aでは発生しなくなる（安全な3キーはいずれもBUG-08/61/62ガードの対象VK
ではないため）。

## スコープ

- 対象VK: `VK_CONVERT`・`VK_NONCONVERT`・`VK_SPACE`（決定0）。
- Alt/Win/CapsLock/Ctrl/Shiftは対象外（ユーザー確認済み、2026-09-05）。
  理由:
  - Alt/Winはモディファイア（背景セクション参照、`GetAsyncKeyState`で
    vkと無関係に直接読まれる）。
  - CapsLockはADR-111が撤回した理由（ロック状態管理・IMEグローバル
    ショートカット競合）と同型のリスクを再導入する。
  - Ctrl/Shiftの除外理由: `keymap.rs::is_forbidden_ctrl_or_shift_primary_key`
    が挙げる機序（`send_keymap_target`の後段注入方式に固有の`push_restore`
    二重管理）は本ADRの方式には存在しない。正しい理由は、Ctrl/Shiftが
    `PHYSICAL_KEY_STATE`（`hook.rs:186`）のOR合成で読まれる消費者
    （`output/held_modifiers.rs:39-44`、`observer/focus_observer.rs:31-36`、
    `hook.rs:1161-1162`のCtrl held判定）を持ち、これらはいずれも**代入前の
    物理vk**（`hook.rs:951-954`で記録）を基準にするため、代入後vkとの
    間で恒久的な食い違いが生じることである。
- `config.left_thumb_vk`/`right_thumb_vk`（現在設定中の親指キー）は対象VK
  リストの話ではなく、決定1・決定4で述べる整合性ルールの対象として扱う。
- 本ADRは「物理キーの役割をパイプライン内で入れ替える」ための挿入点・
  hold-state管理・既存機構との相互作用のみを決定する。具体的にどのキーを
  どう入れ替えるかのconfig/GUIは対象外（Phase B）。

## 決定

### 決定1: 挿入点・合成順序・ルール集合の形

`hook.rs:1135`（`vk = rewritten_vk;`、Alt impersonation適用直後）の直後、
かつ1137行目の`if !is_injected { ... }`ブロックの**内側・先頭**（親指キー
押下時刻更新の`update_thumb`呼び出し、1148行より前）に、新しい役割代入
判定を追加する（決定1-1の`!is_injected`ガードとブロック位置の整合を明確化
——r3レビューTN2、決定1冒頭の「1135と1137の間」という表現は、1137行が
`!is_injected`ガードの開始行であるためブロックの外を指すように読めてしまう
ので、この記述に統一する）。以後は完全に既存パイプラインをそのまま流す
（新しい分岐を`classify_key`以降に追加しない）。

**r1→r2で以下2点を追加した（r1レビューR3・N1、両レビュアーが独立に到達した
一致点）。r2→r3でさらに1・4を修正した（r2レビューS3/N3/Watch1）:**

1. **役割代入は、その打鍵でAlt impersonationが発火しなかった場合にのみ
   適用する**。これは`1135`行時点の`vk`（適用後）を条件に使ってはならない
   ——`vk = rewritten_vk;`実行**直後**の時点では、代入前後の値は既に
   同一変数に上書き済みであり、「`rewritten_vk == vk`」という比較は
   その場では常に真になる、恒真の条件になってしまう（r2レビューS3、
   architect役指摘）。正しい実装は、`hook.rs:1128`に既にある
   `if rewritten_vk != vk { ... }`という比較（この時点では`vk`はまだ
   Alt適用前の値であり、新たに変数を退避する必要はない——r3レビューTN1）
   の結果を`let alt_impersonated = rewritten_vk != vk;`のようなフラグに
   取り、`alt_impersonated`がfalseの場合（そのイベントについてAlt
   impersonationがno-opだった場合）にのみ役割代入を適用する、という
   イベント単位の判定にする。

   この判定に`is_alt_impersonation_active()`（`hook.rs:587-589`、
   `ALT_L_IMPERSONATING || ALT_R_IMPERSONATING`というグローバルなラッチ）
   を使ってはならない（r2レビューpremortem役Watch1）: これは「現在
   どちらかのAltがなりすまし中か」を示すだけで、個々のイベントがなりすまし
   の結果かどうかを示さない。Left Altを親指キーとして押しっぱなしにして
   いる間、このグローバルラッチはtrueのままになるため、それを条件に使うと
   無関係な物理スペースキーの役割代入まで抑止してしまい、しかもAltを先に
   離すとdown/upで判定が非対称になる（決定2/3が守る不変条件が壊れる）。

   Alt impersonationの出力（Left/Right Altが親指キーのvkへ書き換えられた
   結果）には役割代入を重ねて適用しない。これにより「Left Alt→無変換→
   スペース」のような多段合成（r1の旧疑問1）は発生しなくなる（r1レビュー
   の旧疑問1は製品判断としては残さず、この技術的な非合成ルールとして
   確定させる）。

   **さらに、役割代入の適用とhold-state（決定2）の更新は、いずれも
   `hook.rs:1137`の既存ブロックと同じ`!is_injected`ガード内で行う**
   （r2レビューN3、premortem役指摘）。`hook.rs:935`の`is_self_injected`
   早期returnはawase自身が注入したイベントのみを弾き、他プロセス
   （Mouse Without Borders・リモートデスクトップ・AutoHotkey等）が
   `LLKHF_INJECTED`付きでリレーしたイベントは挿入点まで到達する。これに
   役割代入を適用すると、他プロセスがリレーした`VK_SPACE`がローカル側で
   `VK_NONCONVERT`に化けてしまい、ADR-119（issue #136）が確立した
   「解釈しない入力は消費しない」という方針（`transport.rs:284-309`の
   `is_injected`早期return）と矛盾する。加えて、Down/Upが対で来ない注入
   （リレーツールでは珍しくない）で`was_down`がstuckすると、次の物理押下が
   auto-repeat扱いになり代入がスキップされる——決定2がBUG-100対策として
   構造的に防いだはずの状態が、`is_injected`を見落とすと別経路から再現する。

   **【Phase Bからの申し送り、ADR-141決定B7参照】** 上記の「Alt適用前後の
   比較」「`!is_injected`ガード」は、実装時には本項・決定2のシグネチャに
   直接書くのではなく、`event_eligible: bool`
   （`!alt_impersonated && !is_injected`）という単一パラメータへ吸収し、
   `decide_role_substitution`（決定2）に渡す設計に変更すること
   （ADR-141のPhase Bテスト計画レビューで判明、Linux側のテスト網羅性を
   大きく広げられるため）。この場合、hook.rs側は`!is_injected`の早期
   returnゲートを持たず、安全な3キーのイベントを**無条件に**
   `decide_role_substitution`へ渡し、`event_eligible`の計算だけを担う
   （ガードを呼び出し側の分岐からパラメータの計算へ移す）。
   `event_eligible`は`decide_alt_impersonation`の`engine_enabled`と
   同じく**新規押下時点でのみ**参照すること（無条件の早期returnは
   BUG-41と同型のstuck keyを再導入する、ADR-141決定B7参照）。詳細な
   シグネチャ・網羅テーブルの拡張（16→32通り）はADR-141決定B7を参照。
2. **役割代入ルールの集合は、対象3キー上の全単射（置換）でなければ
   ならない**: 明示的にルールが無いキーは恒等（自分自身へ写る）として
   補完し、補完後の写像が「異なる2つの入力が同じ出力を持つ」ことが無い
   ことをconfig読み込み時に検証する。違反する設定は個別ルールのskipでは
   なく**ルール集合全体を無効化**し、エラーを出す（部分的に修正できない
   種類の不整合であるため）。

   例: 「無変換↔スペース」（変換は恒等）は有効な全単射。「変換→スペース」
   単独（無変換は恒等でスペースへ、変換もスペースへ、の2入力が同じ出力）
   は**無効**——`変換`と`元のスペース`の両方が代入後`VK_SPACE`を生成して
   しまうため、config読み込み時に拒否する。ユーザーが「変換をスペース
   として使いたい」なら、スペースを含む完全な置換（例:
   「変換→スペース、スペース→変換」の2-cycle、または
   「変換→スペース、スペース→無変換、無変換→変換」の3-cycle）を明示的に
   指定する必要がある。これは要望である「相互に入れ替える」（秀Caps相当）
   の意味論そのものであり、自然な制約である。

   全単射であることにより、代入後のある1つのvkを生成しうる物理キーは
   常にちょうど1つに定まる。これが`LEFT_THUMB_DOWN_AT_US`
   （`hook.rs:1138-1153`）・`KeyLifecycle`・`KeymapLatch`（決定8）が
   前提とする「1つのvkに対して1つの物理キーの押下状態だけを追跡すれば
   よい」という不変条件を構造的に保証する。
3. **役割代入は1イベントにつき高々1回だけ適用する**（テーブル参照結果を
   再度`from`として照合しない）。実装は「全単射写像の1回のテーブル引き」
   であり、繰り返し適用するfixpointループとして実装しないことを明記する
   （素朴な「引けなくなるまで繰り返す」実装は、循環置換で無限ループする
   か偶数回で元に戻り機能が無効化される）。

### 決定2: hold-stateはBUG-41と同型の規則を、状態表現も含めてAlt impersonationからそのまま一般化する

**ADR-110 r3が採用した「VKインデックスのlatched-target配列」は、BUG-100が
「r2のテーブル参照方式より悪化する退行」と明示的に断じた実装であるため
不採用とする**（r1レビューB5）。

代わりに、`decide_alt_impersonation`（`state/alt_impersonation.rs:67-94`）の
状態表現をそのまま一般化する: 安全な3キーそれぞれについて、**代入前の
物理vkでインデックスした**「`was_down: bool`」と「`confirmed_target:
Option<VkCode>`」を独立のスロットとして持つ（Alt実装の
`ALT_L_IMPERSONATING`/`ALT_L_WAS_DOWN`が別スロットなのと同じ構造。3キーの
みなので固定長の名前付き変数、計6個で十分——例:
`CONVERT_WAS_DOWN`/`CONVERT_CONFIRMED_TARGET`、`NONCONVERT_...`、
`SPACE_...`。変数名は「代入前の物理キー」を指すことを明示する。代入先で
キーイングすると双方向スワップで2つの物理キーが同じスロットを奪い合う
——r1レビューN2）。

**r1→r2でシグネチャを修正した**（r1レビューN2/RN4、r1の擬似シグネチャは
決定3の主張と矛盾していた）:

```
fn decide_role_substitution(
    original_vk: VkCode,
    rule_target: Option<VkCode>,       // 現在のconfigでこの物理vkに定義された変換先(無ければNone)
    is_keydown: bool,
    was_down: bool,
    confirmed_target: Option<VkCode>,  // 前回までに確定した役割(KeyDown確定値をKeyUpまで保持)
) -> (VkCode, Option<VkCode>)          // (書き換え後vk, 次に保持する確定役割)
```

新規押下（`is_keydown && !was_down`）時点でのみ`rule_target`を見て
`confirmed_target`を確定し、auto-repeatのKeyDownはその確定結果
（`confirmed_target`そのもの）をそのまま使う。KeyUpはvk変換こそ対称に
行うが、`confirmed_target`は必ず`None`に、`was_down`も同時に`false`に戻す
（`was_down`と`confirmed_target`は必ずペアで更新し、独立に読み書きしない
——r1レビューW7）。`confirmed_target == None`を`is_fresh_press`の代用に
しない（BUG-100の欠陥をこの分離で構造的に防ぐ、r1レビューB5）。

`alt_impersonation.rs:273-310`の`decide_alt_impersonation_exhaustive_16_combinations`
と1対1に対応する網羅テーブルテスト（`is_keydown` × `was_down` ×
`confirmed_target.is_some()` × `rule_target.is_some()`の16通り）を安全な
3キーそれぞれに用意する。加えて、6変数がスロットとして混線しないことを
確認する別テスト（例: 変換を押しっぱなしにしたまま無変換を押して離しても
変換側の`confirmed_target`が保持されること）を用意する（r1レビューRN4、
「16通り×3キー」という次元の取り方は誤りだったため訂正）。

### 決定3: config reload中の一貫性

決定2の状態表現（`was_down`と`confirmed_target`を常にペアで扱う設計）に
より、ADR-110 r2→r3が指摘した「bool + テーブルスロット位置」方式の欠陥
（config reload中にdown/upのtargetが食い違う、テーブル並び替えでスロットの
意味がズレる）は発生しない——`confirmed_target`はKeyDown時点で確定した値を
KeyUpまで保持するため、reload中にルールが変わってもKeyUp時の変換は確定
時点の値を使う。

### 決定4: 親指キー設定との整合性は「Altセンチネル使用時の不動点制約」のみに限定する

**r1で新設した「`from`/`to`が親指キーvkと一致する組み合わせを全面禁止する」
決定は撤回する。** これは既定config（`left_thumb_key=無変換`,
`right_thumb_key=変換`）で6通りのルール全てを弾いてしまい機能が有効化
できないという致命的な欠陥だった（r2レビューN1、premortem役指摘）。

決定1の全単射制約により、`left_thumb_key`が安全な3キーのいずれかを直接
指す通常の構成（`"無変換"`/`"変換"`/`"VK_SPACE"`）では、親指キーの役割は
その全単射の逆写像で決まる物理キーへ**正しく再配置される**——これは
バグではなく、「秀Caps的にキーを入れ替える」という要望そのものが意図する
挙動である（例: `left_thumb_key="無変換"`の状態で「無変換↔スペース」を
設定すると、物理スペースキーが親指キーになり、物理無変換キーは素の
スペースになる。これはユーザーが望む結果と一致する）。

**残る制約は1点のみ**: `left_thumb_key`/`right_thumb_key`がAlt
impersonationのセンチネル（`"Left Alt"`/`"Right Alt"`、
`alt_impersonation.rs::resolve_thumb_key`が`VK_NONCONVERT`/`VK_CONVERT`へ
固定的に解決する）を使っている場合、決定1により**Alt impersonationの出力
には役割代入が適用されない**ため、Left/Right Altは常にハードコードされた
vk（`VK_NONCONVERT`/`VK_CONVERT`）を生成し続ける。このとき、役割代入の
全単射が**別の**物理キーをこの同じvkへ写すルールを含んでいると（例:
`left_thumb_key="Left Alt"`の状態で「無変換→スペース、スペース→無変換」
という2-cycleを設定すると、物理スペースキーが代入後`VK_NONCONVERT`を
生成するようになり、Left Altと物理スペースの両方が`VK_NONCONVERT`を
生成する）、決定1が防ごうとした「2つの物理キーが同一vkを生成する」
状態がAlt経由で再現する。

**決定**: `left_thumb_key`/`right_thumb_key`がAltセンチネルを使っている
場合、そのセンチネルが固定するVK（`VK_NONCONVERT`/`VK_CONVERT`）は、
役割代入の全単射において**不動点（自分自身に写る）でなければならない**
——他の物理キーの`to`としてこのVKを使うルールは、config読み込み/reload
時に拒否する（ルール集合全体を無効化し、衝突しているルールと
Altセンチネル設定の両方を名指ししたエラーを出す）。この検証は、親指キー
設定と役割代入設定のどちらが変更されたreloadでも再実行する。

**r2→r3で以下2点を追記した（r2レビューS4/S5、architect役指摘）:**

1. **不動点制約は構成によって効きすぎる場合がある**。対象が3キーしか
   ないため: (a) 片側のみAltセンチネル（例: `left_thumb_key="Left Alt"`、
   右は通常キー）の場合、不動点になるのは1キーだけなので、残る2キー上の
   非恒等な全単射は2-cycle 1通りだけが選べる——機能は使えるが選択肢は
   限られる。(b) **左右ともAltセンチネル**（`left_thumb_key="Left Alt"`
   かつ`right_thumb_key="Right Alt"`、US配列の標準構成）の場合、
   `VK_NONCONVERT`と`VK_CONVERT`の2つが同時に不動点になり、置換の性質上
   残る`VK_SPACE`も自動的に不動点にならざるを得ない——**許される写像は
   恒等写像のみとなり、本機能は構造的に使用できない**。これはr1決定4が
   既定configで機能を殺したのと同じ失敗クラスだが、影響範囲はAlt
   センチネル使用者に限定される。Phase Bでは、この場合の拒否メッセージが
   「ルールが無効です」ではなく、衝突している**親指キー設定側**を
   名指しすること（例:「`right_thumb_key = "Right Alt"`が`VK_CONVERT`を
   固定しているため、このルール集合は全単射になりません」）を要件とする。
2. **不動点制約が排除するのは、役割代入が新規に導入する衝突のみである**。
   `left_thumb_key="Left Alt"`のようなAltセンチネル構成には、本ADR以前
   から存在する別種の衝突——物理Left Alt（Alt impersonationにより
   `VK_NONCONVERT`を生成）と物理無変換キー（不動点なので恒等で同じく
   `VK_NONCONVERT`を生成）が同一vkを生成する——が、不動点制約を満たして
   いてもなお残る。本ADRはこれを悪化させないが解消もしない。したがって
   決定1-2が述べる「代入後のある1つのvkを生成しうる物理キーは常に
   ちょうど1つに定まる」という保証は、**Altセンチネルを使わない構成に
   ついてのもの**であり、Altセンチネル構成（および決定8が依拠する
   `KeymapLatch`の安全性の保証）には、この既存の限界がそのまま及ぶ。

### 決定5: BUG-62ガードとの関係はPhase Aでは非該当

決定0でスコープを安全な3キーに縮小したことにより、旧決定5・旧疑問2
（役割代入の`to`が`VK_KANA`の場合にBUG-62のAlt+かなガードを迂回する）は
**Phase Aでは発生しない**。この論点は決定0で言及した後続ADR（かな系役割
代入）が専用に扱う。

### 決定6: 代入後vkを見る下流合流点の一覧

**r1レビューR4で、当初挙げていた合流点の1つが誤特定されていたことが判明し
訂正した。** 以下は代入後vkを条件に使う既存の合流点である:

1. **エンジンのホットキー照合（`src/engine/engine.rs:1115-1140`、
   `matches_key_combo`）**: 既定ホットキー（`src/config.rs:562-565`の
   `engine_on="Ctrl+Shift+変換"`、`engine_off="Ctrl+Shift+無変換"`、
   `ime_on="Ctrl+変換"`、`ime_off="Ctrl+無変換"`）は`event`（代入後vkを
   持つ`RawKeyEvent`）と`modifiers`で照合される。変換↔無変換の役割代入を
   設定すると、`Ctrl+Shift+変換`を押したつもりがengine OFFが発火する、
   という反転が起きる。`スペース→無変換`の代入では`Ctrl+Space`（多くの
   IDE・MS-IME既定のトグル）が`ime_off`と衝突する。**この照合は
   プラットフォーム非依存のcoreクレート（`src/engine/`）側にある**ため、
   Phase Bの検証をどちらのクレートに置くか（`src/config.rs`側の
   `validate_thumb_key_in_ime_combos`、あるいは
   `crates/awase-windows/src/keymap.rs::warn_on_engine_hotkey_collision`）
   はADR-019（coreのOS非依存維持）を踏まえた設計判断になる（未解決の
   疑問1）。
2. **`focus_tracker.rs::enrich_ime_relevance`（`focus_tracker.rs:49-66`）**:
   per-appの「sync key」（`sync_toggle_keys`/`sync_on_keys`/
   `sync_off_keys`）を代入後vk単体で分類し、`rel.may_change_ime = true`を
   立ててIME belief経路に影響する。既定ホットキーとは独立した、per-app
   設定との衝突が同型に発生しうる（1とは別の合流点として扱う——r1では
   誤って1と同一視していた）。
3. **`runtime/transport.rs::PhysicalKeyDisposition::plan`**: 安全な3キーの
   範囲では`shadow_action`は生じない（`ImeKeyKind::from_vk`が3キー
   いずれにも`None`を返す）ため、決定0のスコープ縮小によりこの経路の
   実害はPhase Aでは発生しない。

   **【訂正、ADR-141 r1レビューで判明】** 本項が続けて指摘していた
   「`vk_may_mutate_conv`の非対称性が`transport.rs::plan`の判定を変える」
   という記述は**誤帰属だった**。`vk_may_mutate_conv`の全呼び出し箇所を
   確認したところ`transport.rs`は一度も呼んでおらず、実際の呼び出し元は
   `crates/awase-windows/src/keymap.rs:29`（`[[keymaps]]`の`to`側禁止判定、
   config.tomlに書かれた**静的な**vkを見るため役割代入の影響を受けない）
   と`crates/awase-windows/src/win32.rs:169`（`send_input_safe`の
   `conv_mutation`ゲート、こちらは`RawKeyEventExt::reinject()`が渡す
   **代入後のvk**を見るため実際に影響を受ける）の2箇所である。したがって
   本項が示した非対称性の実害は`transport.rs::plan`ではなく
   `win32.rs:169`の`conv_mutation`ゲートに現れる（ADR-084/086のconv
   actuation系、`fix-requires-evidence.md`の「conv mode」再発ファミリー
   に該当）。詳細と検証方針はADR-141決定B7を参照。
4. **`vk::is_composition_confirm_key`（`vk.rs:319`、`0x20`/`0x0D`/`0x1B`
   のみ）**: `VK_SPACE`を含むため、スペースを絡めた役割代入
   （変換↔スペース、無変換↔スペース）はcomposition確定処理
   （`executor.rs`の`handle_confirm_key_passthrough`/`handle_reinject`）の
   発火対象を入れ替える。GJI/MS-IMEで変換キーを候補選択に使う操作との
   整合はPhase Bで確認する。
5. **`gji_charset_autodetect.rs::is_configured_thumb_key`**:
   `crate::hook::thumb_vk_codes()`を参照し、BUG-115のガード判定
   （IMEモードキーかNICOLA同時打鍵かの識別）に使われる。決定1・決定4の
   全単射・不動点制約により、親指キーとの衝突自体が構造的に起きないため、
   この呼び出し元固有の追加対応は不要。
6. **`src/engine/engine.rs`のvk依存特別扱い**（`set_space_thumb_config`の
   `space_thumb_vk`、`set_thumb_key_solo_tap_config`の
   `muhenkan_vk`/`henkan_vk`、`set_muhenkan_solo_tap_dedicated_fn_key`、
   `set_muhenkan_delegate_to_open_axis`/`set_henkan_delegate_to_open_axis`、
   `engine.rs:924-937`の「既知の限界」コメント参照）: これらは
   `left_thumb_key`/`right_thumb_key`の文字列設定を解決した`left_thumb_vk`/
   `right_thumb_vk`（`VkCode`）から`find(|vk| vk == VK_NONCONVERT)`等で
   絞り込まれる（`app/bootstrap.rs:1042-1047`）。代入後vkそのものではない
   （r2レビューpremortem役Watch3、r1のW9は「文字列から導出」としていたが
   より正確には「解決済みVkCodeから導出」）。決定1（Alt非合成）・決定4
   （不動点制約）が成立している限り、これらは整合性を保つ——ただしAlt
   センチネル使用時はこの`left_thumb_vk`自体がハードコードされた
   `VK_NONCONVERT`/`VK_CONVERT`になる点が決定4の不動点制約と直結する。
   その整合性は偶然ではなく決定1・4の設計に由来することをここに明記する。

   **Phase B向けの注記（r2レビューpremortem役W13）**: `space_thumb_vk`
   （`set_space_thumb_config`）は`left_thumb_vk`/`right_thumb_vk`が
   `VK_SPACE`のときのみ`Some`になる。既定config（`left_thumb_key=無変換`）
   で「無変換↔スペース」を設定すると、物理スペースキーが親指キーの役割を
   担うにも関わらず、`space_thumb_solo_tap_*`系の設定は効かず
   `muhenkan_solo_tap_*`系が効く——役割の意味論としては正しい（スペース
   バーが「無変換の役割」を担っている）が、設定GUIで「スペースキーの
   単独タップ設定」を触っても何も変わらないように見える。Phase Bの
   GUI設計でこの対応関係をユーザーに説明する必要がある。

**決定**: 上記1〜4の合流点は、Phase Bのconfig検証で「役割代入ルールが
既存のホットキー設定・per-app sync key設定のトリガーキーと衝突していないか」
を警告する機能を必須要件とする。本ADR（Phase A）ではコード側の挙動として
「代入後vkが唯一の正」という一貫した原則を確立することのみを決定し、GUI
警告の実装・どのクレートに検証を置くかはPhase Bに委ねる。

### 決定7: 既存のlatch/hold-stateクリア箇所は「KeyUp注入してからクリア」する

**r1の決定7は「クリアのみ」としていたが、r2レビュー（R1/R2、W7/W8、両
レビュアーが独立に到達）で、これがBUG-100の実際の修正水準への後退である
と指摘された。** BUG-100の修正（`docs/known-bugs.md:12240-12248`）は
「`LATCHED_TARGET`が非0ならtargetのKeyUpを注入してからクリアする
`release_all_latched_remap_targets()`」「overflowアームで書き換え後vkの
KeyUpを明示的に注入する」という、**注入してからクリアする**設計だった。
「クリアのみ」または「握りつぶすのみ」では、役割代入によって既にOSへ
送出済みの代入後vkのKeyDownに対応するKeyUpが永久に来ず、OS側にキーが
押しっぱなしのまま残る（stuck key）。

**決定**: 以下の全箇所で、`confirmed_target`が`Some(vk)`であれば、**その
vkのKeyUpを送出してから**、`was_down`と`confirmed_target`を同時にクリア
する（既存のBUG-100修正パターンと同じ）。

**r2→r3で送出方法をスレッドごとに書き分けた（r2レビューS2、architect役
指摘）**: `RawKeyEventExt::reinject()`（`lib.rs:336-342`）のdocは
「メインスレッドから呼ぶこと」という安全契約を明示しており、#3
（フックスレッドで実行される）から呼ぶことはできない。#1・#2はメイン
スレッドなので`reinject()`をそのまま使える。#3は、既存の前例
`inject_alt_menu_mask()`（`hook.rs:311-318`、フックコールバック内から
`send_input_safe`を直接呼び、`dwExtraInfo`に`INJECTED_MARKER`を付けて
`hook.rs:935`の`is_self_injected`で自己弾きさせる、BUG-62追補2の
SC_KEYMENUマスクで実証済みのパターン）と同じ方式——
`crate::tsf::output::make_key_input_ex(vk, /*keyup=*/true, INJECTED_MARKER)`
を組み立てて`crate::win32::send_input_safe`を直接呼ぶ——を使う
（`executor.rs:706-711`の`spawn_local`経由の作法とは別物であり、混同
しないこと）。

1. `reset_physical_key_state()`（`hook.rs:334`、セッションロック等からの
   全解放）: `reinject()`でKeyUp送出後クリア（メインスレッド）。
2. `clear_hook_latches_for_app_disable()`（`hook.rs:374`、`disable_apps`の
   Enter/Leave両方から呼ばれる）: `reinject()`でKeyUp送出後クリア
   （メインスレッド）。Enter時にこの注入を行うことで、`FOCUS_APP_DISABLED`
   早期return（`hook.rs:980-982`、挿入点より前にあり単独では対応不要）の
   間に発生する押しっぱなし状態も、フォーカスが`disable_apps`対象へ移る
   **瞬間**に解消される。なお、Enter時のクリア後もユーザーが物理キーを
   押し続けたまま`disable_apps`対象アプリに滞在し、そこで指を離した場合、
   その生のKeyUpは`hook.rs:980-982`でそのままOSへ渡る——対応するDownが
   無いKeyUpだが、これはstuck Down（キーが離れない）と違い自己修復的で
   無害である（`clear_hook_latches_for_app_disable`のdoc、hook.rs:370-373
   が言う「stuck-trueは危険だがstuck-falseはそうならない」という非対称性
   と同じ、r2レビューSN2）。
3. `passthrough_or_swallow_for_impersonation`（`hook.rs:599-610`）の拡張:
   `is_alt_impersonation_active()`に加えて「安全な3キーのいずれかが現在
   `confirmed_target`を持っているか」も判定条件に含め、overflowラッチ中
   （`hook.rs:1100`早期return、`hook.rs:1203`
   `ProduceResult::Overflow`アーム）に該当する場合は、生の物理KeyUpを
   握りつぶす代わりに`send_input_safe`直接呼び出し（フックスレッド、
   上記参照）で**代入後vkのKeyUpを注入**してからhold-stateをクリアする。

### 決定8: `KeymapLatch`の安全性は本ADRの設計に依存する

**r1で新設したこの決定の本文が、r2の決定4・決定7の全面差し替え作業で
脱落していた（r2レビューS1・N4、両レビュアーが独立に到達）。参照だけが
本文中に5箇所残っていたため復活させる。**

`state/keymap_latch.rs`の`KeymapLatch`（`[[keymap]]`のtap用latch）は、
「同じ物理キーのDownとUpが同じvkとして`deliver_key_event`
（`runtime/message_handlers.rs:163-181`）に届く」ことを暗黙の前提にして
安全性を保っている（config reload中もvk単位のキーであるため破綻しない、
という設計）。この前提は、以下の**2つ**によって支えられる:

1. 決定2の`confirmed_target`がKeyDown時点の確定値をKeyUpまで保持する
   設計——config reload中にルールが変わっても、押しっぱなし中のキーの
   KeyUp時点の変換先はKeyDown確定時点の値のまま変わらない。
2. 決定1-2の全単射制約——ある1つの代入後vkを生成しうる物理キーは常に
   ちょうど1つに定まるため、`KeymapLatch`が「vk単位」で管理していても
   複数の物理キーが同じvkのlatchを取り合うことがない。

**ただし決定4末尾（S4/S5対応）が明記する通り、この2つ目の保証はAlt
センチネルを使わない構成についてのものである**。Altセンチネル構成
（`left_thumb_key="Left Alt"`等）では、Alt impersonationに内在する
既存の衝突（物理Alt自身と、不動点になった物理キーが同一vkを生成しうる）
が残るため、`KeymapLatch`の安全性もこの限界をそのまま受け継ぐ。

この依存関係を明示することで、決定2・決定1-2の設計が単なる衛生上の
選択ではなく`KeymapLatch`の安全性を支える必須の前提であることを記録
する。

## 却下した代替案

- **SendInput + 通常の`dwExtraInfo`マーカーでの後段（engine非経由）再注入**:
  ADR-130と同一のbelief更新スキップ欠陥を再導入するため却下。
- **SendInput + マーカーなしでの後段再注入（ADR-110 `key_remap`方式）**: Alt
  impersonation/Relayアーキテクチャの発見により不要と判明。
- **`KeymapLatch`のhold対応拡張**: tap用途に対し、role substitutionは物理
  キーを押している間ずっと別役割でholdし続ける用途であり、`KeymapLatch`は
  target役割を保持しないため構造的に不足する（決定8参照）。
- **ADR-110 r3の「VKインデックスのlatched-target配列」方式**: BUG-100が
  「r2より悪化する退行」と明示的に断じた実装であるため決定2で不採用。
- **Ctrl/Shiftを対象に含める**: `PHYSICAL_KEY_STATE`のOR合成消費者が代入前
  vkを基準にするため恒久的な食い違いが生じる。
- **ひらがな/かな系キーをPhase Aに含める**: 決定0参照。
- **（r1で採用しr2で撤回）`from`/`to`が親指キーvkと一致する組み合わせを
  全面禁止する**: 既定configで機能が丸ごと無効化される致命的な欠陥だった
  （決定4参照）。全単射制約＋Alt非合成という2条件に置き換えた。
- **ルールをconfig読み込み時に個別skipする方式（`[[keymap]]`の前例に
  倣う）**: 全単射制約の違反は個別ルールのskipでは修復できない（写像
  全体の整合性の問題であるため）。ルール集合全体を無効化しエラーを出す
  方式を採用した（決定1）。

## 未解決の疑問

1. 決定6のPhase B検証（既定ホットキー・per-app sync keyとの衝突警告）を
   `src/config.rs`（coreクレート）と`crates/awase-windows/src/keymap.rs`
   （platformクレート）のどちらに実装するか。ADR-019（coreのOS非依存
   維持）を踏まえた設計判断が必要（決定6参照）。
2. 役割代入と`[[keymap]]`が同じ物理キーを対象にした場合にユーザーへどう
   見せるか。GUI側の重複検出はPhase Bで検討する。前提として決定8
   （`KeymapLatch`のDown/Up vk一貫性）が実装で正しく保たれている必要が
   ある。
3. `config.rs::validate_thumb_key_in_ime_combos`（`src/config.rs:1014`
   開始、coreクレート所属）はconfig文字列同士を比較しており、決定1の
   全単射検証・決定4の不動点検証とは別のチェックである。両者を統合する
   か独立に保つかは実装時に決める。

### 解消済み（参考、r0・r1からの変更点）

- **旧疑問3（vkとscanの不整合な組み合わせ）**: `RawKeyEventExt::reinject()`
  （`lib.rs:358-373`）は`self.scan`を一切読まず`wScan: 0`固定で`wVk`のみ
  送るため、役割代入の有無に関わらずvk/scanの不整合な組み合わせを構成
  できない。これのみで結論が閉じる（r1レビューRN1: `known-bugs.md:2720-2724`
  の「win32kがscanを再計算する」という記述は「推定」「可能性がある」
  という留保付きの仮説であり、根拠として引用しない——`reinject()`が
  scanを送らないという事実だけで十分）。
- **旧疑問5前半**: `build_raw_key_event`は元の`is_injected`をそのまま渡し、
  役割代入はこれを変えない。`should_upgrade_is_japanese_ime`や
  `transport.rs`の`if event.injected { ... }`分岐は代入後イベントを
  正しく「物理キー」として扱う。後半（実際に確認すべき合流点の列挙）は
  決定6に反映した。
- **旧疑問1（Alt impersonationとの多段合成をユーザーが意図するか）**:
  製品判断としては残さず、決定1で「Alt impersonationの出力には役割代入を
  適用しない」という技術的な非合成ルールとして確定させた。
- **旧疑問2（BUG-62ガードの迂回）**: 決定0のスコープ縮小によりPhase Aでは
  非該当。
- **r1決定4「親指キーvkとの衝突を全面禁止」**: 決定4参照、撤回し全単射＋
  Alt不動点制約に置き換えた。

## 関連ファイル

`crates/awase-windows/src/state/alt_impersonation.rs`（一般化元）、
`crates/awase-windows/src/hook.rs:1080-1210`（挿入点、latch/hold-stateクリア
箇所、`passthrough_or_swallow_for_impersonation`）、
`crates/awase-windows/src/lib.rs:358-373`（`RawKeyEventExt::reinject`、実際の
SendInput送出箇所）、`crates/awase-windows/src/state/keymap_latch.rs`
（決定8で参照する安全性の前提）、`crates/awase-windows/src/runtime/executor.rs`
（Relayアーキテクチャ、`ReinjectKey`）、`crates/awase-windows/src/runtime/transport.rs`
（`PhysicalKeyDisposition::plan`、決定6）、`crates/awase-windows/src/runtime/focus_tracker.rs`
（`enrich_ime_relevance`、決定6）、`src/engine/engine.rs`（`matches_key_combo`、
`space_thumb_vk`等のvk依存設定、決定6）、`src/config.rs`
（`validate_thumb_key_in_ime_combos`、未解決の疑問3）、
`crates/awase-windows/src/keymap.rs`（`forbidden_target_vk_reason`との異同、
スコープ節）。関連ADR: ADR-082（Alt impersonation移設の経緯）、ADR-110/111
（撤回、教訓の主要引用元）、ADR-126（Scancode Map、本ADRとは別方式）、
ADR-130（却下されたSendInput注入方式）。

## レビュー履歴

- r0（2026-09-05）: 初版。
- r1（2026-09-06）: Opus 2体（architect役・premortem役）による1ラウンド目
  の敵対的レビューで、決定4の事実誤認、スコープ定義の誤り（ひらがな）、
  `to=VK_KANA`の不可逆破壊リスク、既定ホットキーとの衝突未記載、
  BUG-100型のlatch解放漏れ4経路、hold-state設計がADR-110 r3の欠陥を
  再導入していた点、親指キーvkとの衝突で親指キーが無効化/タイムスタンプ
  破壊される新規Blocker、`transport.rs::plan`への影響未記載、
  `KeymapLatch`の安全性が本ADRの設計に依存する未記載の関係、等を検出。
  決定0・決定4（初版）・決定6・決定7（初版）・決定8を新設して反映。
- r2（2026-09-06）: 同一レビュアーへの再確認（2ラウンド目）で、r1決定4
  （親指キーvkとの衝突を全面禁止）が既定configで機能を丸ごと無効化する
  致命的な欠陥であること（両レビュアーが独立に到達）、決定7が「クリア
  のみ」でBUG-100の実際の修正水準（KeyUp注入）へ後退していたこと、
  決定2の関数シグネチャが決定3の主張と矛盾していたこと、決定6の合流点1
  箇所の所在誤り、を検出。決定1に全単射制約とAlt非合成ルールを追加し
  決定4を全面差し替え、決定2のシグネチャを修正、決定7にKeyUp注入を追加、
  決定6の引用を訂正して合流点を1つ追加。
- r3（2026-09-06）: 同一レビュアーへの再々確認（3ラウンド目）で、r2の
  改訂作業自体が新たに混入させたBlocker 3件（決定1-1の判定条件が挿入点
  では恒真になる、決定7-3のKeyUp注入がフックスレッドから`reinject()`の
  スレッド安全契約に違反する、決定8の本文が編集事故で脱落し参照だけが
  残っていた）とMajor 2件（Alt片側センチネルで選択肢が2-cycle 1通りに
  限定される、Alt両側センチネルで本機能が構造的に使用不可になる）、
  premortem役独立発見のBlocker 1件（他プロセス注入イベントへの適用可否
  `!is_injected`が未決定）を検出。決定1にAlt適用前vkの退避比較と
  `!is_injected`ガードを追加、決定7-3を`send_input_safe`直接呼び出しに
  修正、決定8を復活、決定4末尾にAltセンチネル関連の2点を追記。誤字修正
  （「決入先」→「代入先」）、決定6-6の精度向上（`left_thumb_vk`由来である
  ことの明確化）、`space_thumb_vk`のPhase B UX注記を追加。r4（同一
  レビュアーへの再確認）は未実施。
