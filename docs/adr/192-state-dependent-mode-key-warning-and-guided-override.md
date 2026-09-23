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
  への問い合わせとして、セル単位（開閉軸の純トグルか否か）で機械的に検出する（rev2）。
  (2)検出したら一度だけ警告し、「冪等なキーへの変更」を推奨する——ただし親指キー用途では、既存の
  `msime_key_assignment::conflict_warning`と逆方向の指示にならないよう分岐する（rev2）。
  (3)置き換えは新機構を作らず、既存のユーザー明示config（`keys.ime_on/ime_off/ime_toggle`、`*_solo_tap_ime_action`）をawase-settingsで案内・設定する形にする。
  (3b)親指キー単体への強制ON/OFFは、`*_solo_tap_ime_action`への正規化ではなく（rev1案はOFF/Toggle方向を
  埋められないとround1で判明）、`resolve_pending_thumb_as_single`に新設する専用の合流点でKeyUp解決する
  (rev2、round1のC-1案)。
  (4)`[[keymap]]`（ADR-114）は親指キー・IME制御VKを扱えないので使わない。GJIの`config1.db`の書き換えはしない。
status: |-
  **草案rev2（2026-09-23、opus-adversarial-consult round1で「未収束、決定1・決定3bは実装着手不可」と判定・訂正済み）。**
  ADR-191から分離した（ユーザー指示）。round1が実コード照合で確認した事実:
  (a) 決定3bの前提（`*_solo_tap_ime_action`のcomposing/Passthrough/専用Fnキー制約）は実在するが、
  **その制約下では決定3bが埋めると書いていたOFF/Toggle方向の穴は正規化では埋まらない**
  （満たせるのはON方向×belief OFFだけで、これは正規化なしで既に動く別経路の効果）。
  round1の代替案C-1（正規化ではなく`keys.ime_on/off/toggle`側を単独打鍵確定時にも照合する
  新しい合流点を作る、コンボ側は加算で残す、KeyUp解決にする）を採用し決定3bを全面改訂した。
  (b) 決定1が検出しようとしている情報は、develop に既に3つの実装（`key_effect_predictor.rs`
  `key_effect_table.rs`〈実測セル表〉`gji_charset_autodetect.rs`）が持っており、決定1を
  これらの上に再定義した。判定基準もキー単位の二値からセル単位（開閉軸／未確定文字列の行方）に
  改め、`Stage::None`の純トグル（ATOKのHenkan/Muhenkan等、ADR-189/191決定1の対象そのもの）と
  overlay適用時を警告対象から除外した。
  (c) 決定2の推奨文言が、出荷中の`msime_key_assignment::conflict_warning`（親指キー用途では
  「その割り当てを解除してください」と正反対の指示を出す）と衝突していたため、親指キーか否かで
  分岐する形に改めた。
  決定1〜3・3bはround2レビュー待ち（未収束のため実装は着手しない）。
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

### 決定1（rev2・全面改訂）: 状態依存のキーを、既存の実測表・予測器の上で判定する

**新しい解釈器・新しいキーマップ展開ロジックは作らない**（round1 D-1: 同じ仕事をする実装が
develop に既に3つある）。決定1は次の既存資産への**問い合わせ**として定義する:

- **入力**: 対象VKについて、`KeyEffectKeymap::predict`（`key_effect_predictor.rs`）が返す予測結果と、
  `key_effect_table.rs`が持つ実測セル（ATOK/MSIME/MSIME_NATIVEの各プリセット。`(open, conv, stage, key) →
  (open', conv', Disposition)`）。
- **判定（キー単位ではなくセル単位、round1 D-2）**: あるVKについて、到達可能な`(Stage, Conv)`の
  組み合わせを横断したとき、**開閉軸の効果が「常に`current_open`から`!current_open`への純トグル」
  ではない**（＝どのセルでもOn/Off/Toggleが状態〈入力中か・変換中か〉によって変わる）場合に限り
  **状態依存**とする。
  - `Stage::None`（非入力中）だけで開閉が純トグルになるキー（ATOKのHenkan/Muhenkanが典型）は、
    `ShadowImeAction::Toggle`（ADR-189/ADR-191決定1）でawaseが正確に表現できる**対象外**とする
    （round1 D-2: これを状態依存として警告すると、GJI+ATOKプリセットのほぼ全ユーザーに誤警告する）。
  - `classify_mode_key_ime_action`（`gji_charset_autodetect.rs`）が`OVERLAY_HENKAN_MUHENKAN_TO_IME_ON_OFF`
    適用時に「状態非依存」と既に結論づけているoverlay構成は対象外とする（round1 D-5）。
- **沈黙する条件（round1 D-3, D-4）**: `predict`が`None`を返す組み合わせ（カスタム表がそのVKの行を
  持つ・overlayがある・MS-IME本体で再割り当てがある。BUG-143型の食い違いに対する既存の対処）と、
  実測セルが無い組み合わせ（`key_effect_table.rs`ヘッダの試行数注記を見て、下限試行数を実装時に
  決める）は、**警告しない**（誤診断より見逃しを優先する。ADR-191決定4「実測と食い違えば実測を
  採り、書かない側に倒す」と同じ原則）。判定の型は`Option<確定した分類>`とし、`None`は必ず沈黙する
  契約にする（round1 E-3）。
- **ATOK固有の既知の除外（round1 D-4）**: ADR-186決定(c)（コミット`bff621b5`）が「ATOKプリセットでは
  古い`custom_keymap_table`を読まない」と決めた対象は、決定1でも同じ除外を踏襲する。
- **Microsoft IME本体**: `KeyEffectKeymap::for_msime_native`が読む`KeyAssignment*`レジストリの範囲でしか
  判定できない。判定できないキーは対象にしない。

### 決定2（rev2・分岐追加）: 検出したら一度だけ警告し、冪等なキーへの変更を推奨する（親指キー用途は既存警告に合流させる）

**round1 E-1（Blocker）**: `msime_key_assignment::conflict_warning`は、MS-IMEの無変換→OFF/変換→ON
割り当てが有効なとき、**親指シフトのユーザー向けに「その割り当てを解除してください」**と既に警告
している——決定2が推奨する「冪等なキーに変える」とは逆方向。したがって:

- **そのVKがNICOLAの親指キー（`muhenkan_vk`/`henkan_vk`）に設定されているかで分岐する**:
  - 親指キーとして使っている場合: 決定2の新規警告は出さない。既存の`msime_key_assignment::
    check_and_warn`（GJI側にも同型の判定を拡張する。新しい独立ダイアログは追加しない）が
    「IME側の割り当てを解除し、awaseの明示config（`*_solo_tap_ime_action`、または決定3bの新経路）
    に委ねてください」を案内する。
  - 親指キーでない場合のみ、決定2の「冪等なキーへの変更」警告を出す。
- 起動時・設定リロード時・IME種別の確定時（`sync_ime_kind_from_observation`の合流点）に検出。
  **同一内容につき一度**の判定キーは、新設のハッシュではなく`KeymapCache`が既に持つ`stamp`
  （`config1.db`のmtime+長さ）を流用する（round1 E-2。新設不要）。内容が変われば再警告。
- 警告の文言: どのキー（例: 変換キー）が、どの状態でどう変わるか（例: 入力中は変換、それ以外はIME
  ON/OFF）。観測できないアプリ（Chrome等）でモードずれが起きうること、冪等なキー（`VK_IME_ON`/
  `VK_IME_OFF`）への割り当て変更を推奨すること、awaseの明示config（決定3）で置き換えられること。
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

### 決定3b（rev2・全面改訂、round1 C-1採用）: 親指キー単体への強制ON/OFFは、専用の新しい合流点で単独打鍵確定時にactuateする

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
  ここは選択肢ではなく必須とする。
- **`*_solo_tap_ime_action`の既存制約（composing／`always_suppress=false`／専用Fnキー）はこの新しい
  入力には適用しない**——それらは「IMEがONの間はGJI自身のかな切替に委ねたい」という別の意図
  （`nicola_fsm.rs:2037-2044`）のための制約であり、「このキーで強制ON/OFFしたい」という本決定の
  意図とは排他だからである（round1 C-1の理由）。composing中の扱い（強制OFFは未確定文字列を破棄する
  か、composing中は発火させないか）は本決定で明示的に選ぶ（暫定: 破棄する。ATOKの`VK_IME_OFF`が
  未確定文字列を破棄する挙動〈ADR-191決定1の分類(d)〉と揃える）。
- **actuation合流点が1つ増える**: `.claude/rules/fix-requires-evidence.md`の「IME actuation合流点」表
  （現状6入口）に、この新しい合流点を7つ目として実装時に追記する。ADR-191決定5の指標5
  （IMEへ書く振る舞いの数）にも計上する。
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
  恐れは残る。決定1のセル単位判定＋`predict`が`None`を返す組み合わせは沈黙する規則（rev2）で、
  「誤警告」より「見逃し」に倒すよう保守化したが、実測セル表に無いカスタム構成（ユーザー独自の
  `custom_keymap_table`）は依然として判定不能=沈黙になる。カスタムキーマップこそ状態依存を作り込み
  やすい構成である、という非対称は本ADRのスコープでは解消できない（[ADR-195](195-keymap-learn-productization.md)の学習表が実行時に読めるようになれば、この沈黙域を縮められる可能性がある。本ADRの実装時点では前提にしない）。
- **親指キーとの衝突（rev2で決定2・決定3bに分岐として反映済み）**: 無変換/変換をNICOLAの親指キーとして
  使う場合、`keys.ime_on = ["変換"]`はチョード判定と衝突する。決定2は親指キーか否かで警告文言を分岐し、
  決定3bは親指キー単体を専用の新しい合流点で扱う（rev1で「要検討」だった分け方を、rev2で確定した）。
- **自動診断の出荷失敗の前例（opus round4 QM7）**: 撤去前の`gji_charset_autodetect.rs`のモジュールdocに、キーマップの自動判定・設定支援ポップアップが「実験的機能のまま出荷され、GJIのキー設定が実際にはカスタムなのに『カスタム以外』と誤診断されるなどユーザーの混乱を招いた」ため2026-09-02に全撤去した、という記録がある。
  同型の機能なので、警告は判定の根拠を載せ、ブロックせず、判定できないキーは警告しない。ADR-191決定4に「今回の違い」を書いた（実測との突き合わせ）。
- **警告疲れ**: 状態依存のキーを意図して使うユーザー（入力中は変換、確定後はIME ON/OFF）にとって、警告は不要。「警告しない」の設定と、警告の文言を短くすることで緩和する。
- **MS-IMEの判定精度**: レジストリで判定できる範囲が狭い。判定できないキーは警告しない（見逃しを許容）。
- **ADR-191との依存**: 状態依存のキーが多くなるほど、ADR-191の「観測に追随」の限界（観測できないアプリ）がユーザーに見える。本ADRは、その限界を緩和する層であって、ADR-191の前提ではない。
- **決定3bの合流点追加そのものが持つリスク**: `.claude/rules/fix-requires-evidence.md`の「IME actuation
  合流点」表が示す通り、この種の新しい書き込み経路は過去に何度も見落とし・二重actuationの原因になって
  きた（issue #136等）。実装時は、この新しい合流点が`ADR-189`の固定セット・ユーザー明示configの許可
  リストと衝突しない（許可リストを暗黙に広げない）ことを、既存の`architecture_guard.rs`等のガードで
  固定する。

## 却下した代替案

- **自動置き換え**: 意図した状態依存の使い方を壊す。
- **`config1.db`の書き換え**: 上記のとおり保守的に退ける。
- **警告なしでベストエフォートのみ**: ユーザーがずれの原因（自分のキー割り当て）に気づけない。

## 検証計画

- 決定1: `key_effect_table.rs`の実測セル（ATOK/MSIME/MSIME_NATIVE）を使い、`Stage::None`純トグルの
  キー（ATOKのHenkan/Muhenkan等）が状態依存と誤判定されないこと、`predict`が`None`を返す組み合わせが
  沈黙すること、overlay適用構成（`OVERLAY_HENKAN_MUHENKAN_TO_IME_ON_OFF`）が対象外になることを
  単体テストで固定する（Linuxで走る）。
- 決定2: 警告の一度きり・再警告（`KeymapCache`の`stamp`変化で再判定）・「警告しない」の動作の単体テスト。
  親指キー設定時に決定2の新規警告ではなく`msime_key_assignment::check_and_warn`型の警告に分岐する
  ことの単体テスト。
- 決定3: awase-settingsの1操作の置き換えで`config.toml`が期待どおり書かれ（`*_solo_tap_always_suppress`
  の同時書き込みを含む）、元に戻せること（実機）。
- 決定3b: 新設する合流点の回帰テスト（`.claude/rules/fix-requires-evidence.md`の「IME actuation合流点」
  表の対象なので、テストか`docs/known-bugs/`記録のいずれかが必須）。少なくとも: (a) エンジン活性中の
  単独打鍵（KeyUp解決）でOFF/Toggleが発火すること、(b) 通常のタップ（100ms超）でも`execute_from_loop`
  のタイマー解決に落ちずactuateされること（ADR-186 3/3 FAILの再発防止の直接確認）、(c) チョード成立時は
  発火しないこと、(d) composing中の扱い（決定3bが選んだ挙動）が固定されること。

## 関連

ADR-092（外部キーの意味づけ）、ADR-110（撤回）、ADR-111・114（`[[keymap]]`と制限）、ADR-143〜146（GJIキーマップ書き換え、保留）、ADR-153（明示config）、ADR-176（較正UI）、ADR-186（GJI/ATOKモードキー実測行列とbelief追随、決定3bのKeyUp解決の教訓の出所）、
ADR-189（半角/全角のbelief基づく書き込み）、ADR-191（本ADRの前提。`key_effect_predictor.rs`/`key_effect_table.rs`が決定1の再利用対象）、ADR-195（キーマップ学習の製品化、決定1の沈黙域を将来縮められる可能性）、BUG-143（`session_keymap`と`custom_keymap_table`の食い違い）。
