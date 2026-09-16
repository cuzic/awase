---
id: ADR-176
title: |-
  IME状態を確実に読めるアプリでモードキーの実効果を較正し、
  読めないアプリへ受動観測として転用する
status: |-
  **2026-09-15: round1・round2ともBlocker5件検出、v3へ全面訂正
  （opus-adversarial-consult未実施）。**

  - **round1**（同一アプリ内、`ConvOpenInference`という弱い代理
    シグナルから学習、NICOLA親指キー除外つき）: Blocker5件（親指キー
    除外が対象キー自身を全滅させる、観測チャネルがON方向専用、
    「beliefを書き換えないから安全」がOFF→ON方向にしか成り立たない、
    相関判定の循環論法、単一の弱い観測で広い決定をする時定数の悪化）。
  - **round2（v2）**: 「IME状態を確実に読めるアプリ（Standard）で
    `ImmGetOpenStatus`直接読み取りにより較正し、読めないアプリへ
    `ObserverReported`（受動観測）として転用する」方式へ全面書き直し
    したが、再びBlocker5件——(1)`ObserverReported`は
    `check_drift_correction`の免除リスト（`ConvOpenInference`/
    `HeuristicDefault`限定）に入らないため無条件でdrift correction
    （実`VK_IME_OFF`送信）に到達する、(2)`IntentStore`/`last_intent`
    （ADR-174 round3が発見した層）がv2で一度も検討されておらず
    Engine ON追従という目的自体が明示意図が生きている間は達成
    できない、(3)2セル表完成に必要な`pre_open=true`側の観測は
    NICOLA親指キーのチョード判定（Phase 3）に入ってしまい原理的に
    取れない、(4)Standardアプリでの`post_open`はconfig1.db駆動の
    awase自身のactuationの結果である場合があり誤分類を追認しうる、
    (5)モードキー直後のクロスプロセス読み取り自体がBUG-113で
    確定した「@」の独立十分条件——という指摘だった。

  **ユーザーの再指摘**: 「`config1.db`を読む方式と、実際にユーザーに
  打鍵してもらってその効果を観測する方式は、本質的にやっていることが
  同じはず」という指摘を受け、実装上の制約（`ObserverReported`/
  belief層を経由する、という思い込み）に引っ張られすぎていたことに
  気づいた。**BUG-143の`config1.db`方式が安全な理由は「情報源が信頼
  できるから」ではなく、分類結果を`shadow_action`という「物理IMEキーの
  明示意図」を宣言する経路（`IntentKind::PhysicalImeKey`、
  `IntentStore`と競合するのではなく正規に書き込む側）に乗せている
  からである**。学習結果も`ObserverReported`ではなく、**config1.db
  由来の分類と全く同じ`shadow_action`供給層**に、**config1.dbの分類が
  `None`（未割当）のときだけ埋める補完**として書けば、(1)(2)は
  BUG-143が既に実機確認済みの経路をそのまま継承するので発生しない。
  さらに対象をBUG-143の実対象である**OFF→ON方向のみ**に絞れば
  （Toggle/双方向の完全な学習は非スコープにする）、(3)は
  `pre_open=true`側の観測が不要になり消える。(4)は「config1.dbが
  `None`のときだけ学習する」制約により、awase自身が何もしていない
  状況でのみ学習するため構造的に発生しない。(5)は「打鍵に反応して
  新しい読み取りを発行する」のをやめ、**Standardアプリで既に定期的に
  走っているObserverPoll/FocusProbe等の既存観測**を打鍵タイムスタンプの
  前後で相関させるだけにすることで、新しい読み取りトリガーの追加
  そのものを無くす。下記「決定（案、v3）」に全面的に書き直した。
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

# ADR-176: IME状態を確実に読めるアプリでモードキーの実効果を較正し、読めないアプリへ受動観測として転用する

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

`config1.db`の静的パースに頼らず、**実際のOS/IME挙動を観測して**、
モードキー（変換/無変換/かな/漢字等、GJI/MS-IMEのモード変更に関わり
うるキー全般）がawaseのbeliefに正しく追従するようにする。

**核心アイデア（v2、ユーザー提案）**: このリポジトリには既に
「IME状態を確実に読み取れるアプリ」と「読み取れないアプリ」の分類が
ある（`AppImeProfile`、`can_read_imm32_open_status()`/
`can_use_imm32_cross_process()`）。**読み取れるアプリ
（`AppImeProfile::Standard`、例: メモ帳等の通常のWin32 IMMアプリ）で
モードキーが実際にIME状態をどう変えるかを`ImmGetOpenStatus`の直接
読み取りで観測・較正し、その較正結果（「このキーは今のGJI/IME設定
ではTurnOnとして働く」等）を、読み取れないアプリ
（TsfNative/Imm32Unavailable、例: Windows Terminal/Chrome）で
そのキーが押されたときの受動的なbelief観測として適用する**。

**位置づけ**: `config1.db`ベースの静的分類（ADR-092/135/141、BUG-115/
BUG-143）を置き換えるのではなく、それが外れていた場合の自己修復経路
として補完する。設定ファイルが読めない・信頼できない環境でも、実際に
ユーザーがそのキーを（Standardアプリで一度でも）使った実績があれば、
それ以降はTsfNative/Imm32Unavailableアプリでも正しく追従できるように
する。

## 対象キー

`vk.rs::is_ime_mode_key_for_ime`が対象とする範囲全体
（`VK_KANA`/`VK_IME_ON`/`VK_KANJI`/`VK_IME_OFF`/`VK_CONVERT`/
`VK_NONCONVERT`/`VK_DBE_ALPHANUMERIC`〜`VK_DBE_DBCSCHAR`等）を対象と
してよい——**round1と異なり、変換/無変換を除外しない。むしろこれらは
GJIのキーマップ設定次第で意味が変わる（ADR-174/BUG-143の対象）ため
主要な対象である**。NICOLA親指キーとして設定されているかどうかは、
較正フェーズ・適用フェーズのどちらの安全性にも影響しない（下記
「なぜNICOLA親指キー除外が不要か」参照）——round1のB1はこの除外条件
自体が誤りだった。

`VK_KANA`等「Win32 API上は固定方向のはず」のキーも対象に含めてよい。
[ADR-175](175-physical-dbe-key-stuck-direction-recovery.md)（BUG-142）が
示すとおり「固定方向のはず」という前提自体が実機で裏切られることが
あり、実際の挙動を観測して補正する仕組みはこれらのキーにも無関係では
ない。ただし優先度の議論はround3に譲る（下記「未解決点」参照）。
なお本ADR v3はOFF→ON方向のみを対象とする（下記「非スコープ」参照）
——`VK_KANA`等は`ImeKeyKind::shadow_effect()`が既に固定方向で正しく
判定できているため、v3の学習対象になるのは主に「config1.dbが
`None`を返すVK」（BUG-143の`VK_CONVERT`/`VK_NONCONVERT`が典型例）
である。

## 却下した代替案

### 能動的なテストキー送信によるプロービング

「起動時やGJI検出時に、awase自身が対象キーを合成SendInputで送信し、
その結果を観測してキャリブレーションする」案は**却下**する。ADR-153
ケース3の実機履歴（`docs/known-bugs/BUG-113.md`・`BUG-124.md`）で、
「生キーがGJIへ届くこと」「awase自身が明示IME制御actuationを行う
こと」のどちらか片方だけでも「@」を誘発するのに十分と2回の独立した
実機A/Bで確定している。合成テストキー送信は両方を同時に満たすため、
高確率で「@」を再現すると判断した。**本ADRは、ユーザーが自発的に
押した物理キーの結果だけを観測する（awase自身は一切キーを送信
しない）**——これはv2でも変わらない大原則。

### 却下（round1）: 同一アプリ内の弱い代理シグナルによる学習

round1は「TsfNativeアプリ内で、次の実キー入力が自然に発生させる
`ConvOpenInference`観測（conv ビットからの間接推測）と、直前の
`effective_open()`スナップショットを突き合わせて学習する」という設計
だった。opus-adversarial-consult round1でBlocker5件により**却下**。
主因は、この観測チャネルが(a) ON方向にしか存在しない、(b)「打鍵前
状態」をbelief自身から取るため循環する、の2点。詳細は本ファイル
frontmatter statusに要約。

### 却下（round2）: `ImmGetOpenStatus`直接読み取り＋`ObserverReported`
（受動観測）としての適用

round2（v2）は「Standardアプリで`ImmGetOpenStatus`を較正のたびに
新規に読み取り、適用フェーズは新設`ObservationSource`による
`ObserverReported`（belief層への受動観測）とする」という設計だった。
opus-adversarial-consult round2でBlocker5件により**却下**。主因は、
(a) `ObserverReported`は`check_drift_correction`の免除リスト
（`ConvOpenInference`/`HeuristicDefault`限定）に入らないため無条件で
drift correction（実`VK_IME_OFF`送信）に到達する、(b) `IntentStore`/
`last_intent`層が一度も検討されておらずEngine ON追従という目的自体が
達成できない、(c) 較正の「打鍵直後に新規のクロスプロセス読み取りを
発行する」設計がBUG-113で確定した「@」の独立十分条件そのものだった、
の3点。詳細はfrontmatter status参照。

## 決定（案、v3、opus-adversarial-consult未実施）

### 核心の気づき: 分類結果の「置き場所」が安全性を決める

BUG-143の`config1.db`方式が安全な理由は「情報源が信頼できるから」
ではない。分類結果（`ImeToggleKind::On/Off/Toggle`）を
`henkan_shadow_override`/`muhenkan_shadow_override`という**静的知識
キャッシュ**に置き、`kp_stage_shadow_ime_toggle`が読む`shadow_action`
→`IntentKind::PhysicalImeKey`→`write_physical_key`という、**物理IME
キーの明示意図を宣言する経路**にそのまま合流させているからである。
この経路は`IntentStore`/`last_intent`と「競合」するのではなく、
それらへ**正規に書き込む側**（半角/全角等の本物の物理IMEキーと
全く同じ扱い）であり、ADR-174 round4で確認済みのとおりOFF→ON方向は
actuationを一切発行しない。

したがって「学習した分類結果」も、`ObserverReported`（belief層の
観測）としてではなく、**config1.db由来の分類と全く同じキャッシュ・
同じ経路**に置けば、round2のBlocker(a)(b)は経路の選択ミスとして
最初から発生しない——これは新しい安全機構ではなく、**BUG-143が既に
実機確認済みの経路をそのまま再利用する**という設計である。

### スコープを絞ることで残りのBlockerが構造的に消える

round1のB1/round2のB3（NICOLA親指キーのチョード判定と衝突する）は、
「両方向（OFF→ONとON→OFF）を学習しようとする」ことに起因していた。
**本ADRの実対象（BUG-143）はOFF→ON方向だけ**なので、v3は明示的に
**OFF→ON方向のみを学習・適用する**（Toggle・ON→OFF方向は非スコープ、
下記「非スコープ」参照）。これにより:

- 較正フェーズは`pre_open=false`（直接入力、Engine非活性）の間だけ
  発火すればよく、`pre_open=true`側の観測（NICOLA親指キーのチョード
  判定と衝突するround2 B3の原因）が不要になる。
- 学習結果は常に`ShadowImeAction::TurnOn`のみなので、`Toggle`の
  循環（round2 M5/6）も発生しない。

round2のB4（Standardアプリでのawase自身のactuationによる汚染）は、
**「config1.dbベースの静的分類が`None`（未割当）のVKについてのみ
学習する」という制約**で解消する。分類が`None`ならawaseはそのVKに
対して`shadow_action`を一切発行しないため、Standardアプリでの
IME状態変化は純粋にIME/GJI自身の挙動であり、awase自身の作用による
汚染が構造的にあり得ない。

round2のB5・M1（打鍵に反応した新規クロスプロセス読み取りが「@」の
十分条件・BUG-034の再燃）は、**打鍵のたびに新しい読み取りを発行
しない**ことで解消する。Standardアプリでは`ObserverPoll`（500ms周期）
・`FocusProbe`等の**既存の受動観測**が、この打鍵と無関係な独自の
スケジュールで既に定期的に`PerSourceObservations`を更新している。
較正は、対象VKの物理KeyDownの**タイムスタンプ**を記録し、その前後で
これら既存observationが`open`の値をどう変えたかを事後的に相関させる
だけでよい——打鍵がトリガーとなって新しい読み取りを発行する連鎖
（BUG-113 guard5が禁じる形そのもの）を一切作らない。

### 設計の骨子

1. **較正フェーズ（Standardアプリ、`can_read_imm32_open_status()==true`
   かつ`!cannot_verify_real_ime_state(class_name)`——round2のMinor m2
   指摘のとおり`profile==Standard`の値だけで判定しない）**:
   - トリガー: 対象VKの物理（非注入）KeyDownで、
     `explicit_ime_action_consumed`が立っておらず、かつ
     **config1.dbベースの静的分類（`classify_mode_key_ime_action`）が
     このVKについて`None`を返す**場合のみ。
   - 打鍵の**タイムスタンプ**を記録する（新しい読み取りは発行しない）。
   - 打鍵時点で`PerSourceObservations`から得られる直近の信頼できる
     観測が`open=false`であることを確認する（`pre_open=false`条件、
     ここも新規読み取りではなく既存の最新観測値を読むだけ）。
   - 生キーはそのままパススルーする（一切変更しない）。
   - **その後の既存observation更新**（次のObserverPoll/FocusProbe等、
     この打鍵とは無関係な独自スケジュールで発生するもの）を監視し、
     打鍵タイムスタンプより後に記録された観測が`open=true`を示せば
     「このVKはOFF→ONとして働く」候補とする。
   - 相関の失格条件（打鍵タイムスタンプ〜観測の間に発生したら較正を
     諦める）: 別の明示IME操作、フォーカス変更、他のIMEモードキーの
     打鍵、`conv_mutation_seq`の変化。

2. **学習の確定条件（1セル・2回一致、ユーザー承認済みの「2回一致まで
   確定しない」方針をOFF→ON方向のみに単純化して適用）**: 対象VKの
   `TurnOn`候補が**2回連続で一致**した時点で確定とする。矛盾する
   観測（確定後に`post_open`がfalseのまま、または別のOFF→ON以外の
   遷移）が観測された場合は確定を取り消し未確定へ戻す。

3. **保存先**: `henkan_shadow_override`/`muhenkan_shadow_override`を
   `VK_CONVERT`/`VK_NONCONVERT`に限らない汎用マップへ一般化するか、
   並列の新規フィールド（例: `learned_mode_key_turn_on: HashSet<VkCode>`
   または`HashMap<VkCode, u8>`で一致回数を保持し確定時に別集合へ昇格）
   を追加する。**config1.dbベースの分類を読む関数
   （`resolve_henkan_muhenkan_shadow_override_for_event`等）の
   `.or_else()`チェーンの最後に、確定済み学習結果を`Some(ShadowImeAction::
   TurnOn)`として追加する**——config1.dbの分類が`Some`を返す限り学習
   結果は一切参照されない（config1.db優先、学習は補完のみ、
   round2レビュアー推奨に従う）。

4. **適用フェーズ**: 新しい消費経路を追加しない。上記3で
   config1.dbベースの分類関数の出力に合流させているため、
   `kp_stage_shadow_ime_toggle`以降は**既存のconfig1.db駆動と全く
   同じコードパス**を通る。新しいactuation合流点はゼロ
   （`fix-requires-evidence.md`の「IME actuation合流点」表に
   新規行を追加しない）。

5. **ライフサイクル**: 較正はStandardアプリで行われ適用は
   TsfNative/Imm32Unavailableアプリで行われるため、`henkan_shadow_override`
   （GJIアクティブ区間ごとにリセット）とは異なる寿命が必要——
   IME種別（`ActiveImeKind`）が変わったとき、設定リロード
   （`reset_streak_latch_for_reload`と同じ箇所）、`left_thumb_vk`/
   `right_thumb_vk`の変更、のいずれかで学習表をクリアする
   （round2 M4が指摘した4トリガーのうち、v3のスコープ縮小により
   「`config1.db`の内容/mtime変化」は次善——config1.dbが`Some`を返す
   ようになった時点で学習結果は`.or_else()`チェーンにより自然に
   無視されるため、明示的なクリアは必須ではないが、リソース解放の
   観点で実装時に検討する）。

### round1・round2のBlockerがv3でどう解消されるか（要約）

| Blocker | v3での解消 |
|---|---|
| round1 B1/round2 B3: 親指キーのチョード判定と衝突 | OFF→ON方向のみに対象を限定し、`pre_open=true`側の観測自体を無くす |
| round1 B2: `ConvOpenInference`はON方向専用 | v2からv3を通じて不使用のまま——較正は既存observationの前後比較のみ |
| round2 B1: `ObserverReported`が無条件でdrift correctionに到達 | `ObserverReported`を使わない。学習結果はconfig1.dbと同じ`shadow_action`供給層に直接置く |
| round2 B2: `IntentStore`/`last_intent`により目的が未達 | `IntentKind::PhysicalImeKey`経路（ADR-174 round4で実機確認済み）をそのまま再利用するため、`IntentStore`と競合せず正規に書き込む |
| round1 B4/round2の循環懸念 | 較正の「打鍵前状態」は既存observationの最新値から取る（新規読み取りなし）。beliefは使わない |
| round2 B4: Standardアプリでのawase自身のactuationによる汚染 | config1.dbの分類が`None`のVKについてのみ学習するため、awase自身は当該VKに対し何もしていない |
| round2 B5/M1: 打鍵駆動の新規読み取りが「@」/BUG-034を再燃 | 打鍵に反応した新規読み取りを一切発行しない。既存の独立スケジュールのobservationを事後相関するのみ |
| round1 B5/round2 M2: 単一の弱い観測・系統誤差 | 2回一致確定を維持。系統誤差（settle時間不足）は実機実測が必要（下記「未解決点」） |

## 未解決・opus-adversarial-consult round3で詰めるべき点

1. **相関判定の失格条件の網羅性**: 上記「較正フェーズ」の失格条件
   リストが十分か（BUG-19/BUG-51追補/BUG-55が警告する偽陽性源を
   すべて塞げているか）。
2. **既存observationの粒度で十分か**: `ObserverPoll`の周期（実装確認
   要）が、この用途において「打鍵から遠すぎる／近すぎる」ことによる
   系統誤差を生まないか。新規読み取りを追加しない制約の下で、
   タイミング設計にどんな限界があるか。
3. **保存先の具体的な型設計**: `henkan_shadow_override`を一般化する
   か新規フィールドにするか、`.claude/rules/ime-belief-architecture.md`
   「`ImeModel`以外のbelief的状態への適用範囲」節の基準（書き込み経路
   の1箇所集約、フィールドprivate化）にどう適合させるか。
4. **`fix-requires-evidence.md`対応**: 回帰テスト（相関判定・確定条件
   を純粋関数化してLinux CIで実行可能にする）を用意すること。新しい
   `ObservationSource`は不要になった（v3はbelief層を経由しないため）
   ことを確認する。
5. **実機検証方法**: メモ帳等のStandardアプリで、config1.dbの分類が
   `None`のVK（意図的にそうなる設定を作る、またはBUG-143相当の
   状況を再現する）について較正が正しく発火・確定することを実機ログで
   確認し、その後Windows Terminal等のTsfNativeアプリへ切り替えて
   学習結果が適用されEngineが正しくActiveへ遷移することを確認する
   A/B手順を用意する。

## 非スコープ

- 能動的なテストキー送信によるプロービング（上記「却下した代替案」）。
- `config1.db`静的パース自体の廃止（ADR-092/135/141/BUG-143の資産は
  引き続き初期値として使い、食い違った場合も常にconfig1.db側を
  優先する——学習は「未割当」を埋める補完に限定する）。
- **ON→OFF方向・Toggle意味論の学習**（v3の中核的なスコープ縮小。
  round2のB3/M5がこれらの学習には別の設計が必要であることを示した
  ため、別ADRの対象とする）。
- 学習結果のプロセス再起動を跨いだ永続化。

## 関連

ADR-092（`classify_mode_key_ime_action`の起源）、ADR-115（打鍵列機能、
本ADRとは無関係だが同じ「1キーに複数の意味を持たせる」領域）、
ADR-135（Hiragana/Katakanaへの一般化）、ADR-140（probe/actuation競合、
較正フェーズのフェンシングで参考にすべき先例）、ADR-141（Henkan/
Muhenkan delegate、`shadow_action` override機構そのもの）、
ADR-153（明示config、本ADRのトリガー条件が除外すべき既存経路）、
ADR-174/BUG-143（本ADRの直接の動機、`IntentStore`を含むbelief解決の
全階層の調査）、ADR-175（BUG-142、「Win32 API上の固定方向を信じすぎる」
別の失敗モードの先例）。
