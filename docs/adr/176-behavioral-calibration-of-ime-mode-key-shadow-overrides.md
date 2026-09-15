---
id: ADR-176
title: |-
  IME状態を確実に読めるアプリでモードキーの実効果を較正し、
  読めないアプリへ受動観測として転用する
status: |-
  **2026-09-15: round1でBlocker5件検出、設計を全面訂正（round2向けに
  書き直し済み、opus-adversarial-consult未実施）。**

  round1は「同一アプリ内で、変換/無変換をNICOLA親指キー除外つきで、
  `ConvOpenInference`（ON方向しか観測できない弱い代理シグナル）から
  学習する」という設計で、Blocker5件（B1: 親指キー除外が対象キー自身を
  既定設定で全滅させる、B2: 観測チャネルがON方向専用でOFF方向を学習
  できない、B3: 「beliefを書き換えないから安全」がOFF→ON方向にしか
  成り立たず、学習結果がOFF/Toggleになると既存ディスパッチ経由で実
  actuationが解禁されBUG-113の危険な組み合わせを再現する、B4:
  相関判定の「打鍵前状態」を`effective_open()`から取るのは循環論法、
  B5: 単一の弱い観測をGJIセッション全体の方向決定に使い被害の時定数が
  悪化）で破綻した（詳細はround1レビュー参照、下記に要約）。

  **ユーザー指摘により設計を訂正**: 変換/無変換を除外すべきではなく
  むしろ主要対象である。IME状態観測はOFF方向についても正しく機能する
  はずで、既存actuationが解禁されるという帰結はそもそも意図しない
  統合ポイントの選択ミスである。正しい設計は「**IME状態を確実に読める
  アプリ（`AppImeProfile::Standard`）でモードキーの実効果を`ImmGetOpenStatus`
  の直接読み取り（ground truth、双方向）で較正し、確実に読めないアプリ
  （TsfNative/Imm32Unavailable）へその較正結果を**受動観測**として
  転用する**」——同一アプリ内での弱い代理シグナル学習ではなく、
  信頼できるアプリで学習し信頼できないアプリで適用するクロスアプリ
  転移学習。下記「決定（案、v2）」に全面的に書き直した。round1の
  Blockerがv2でどう解消されるかは各項目末尾に付記する。
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
ない。ただし優先度の議論はround2に譲る（下記「未解決点」参照）。

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
主因は、この観測チャネルが(a) ON方向にしか存在しない（`open=false`を
報告する分岐が`classify_conv_transition`に無い）、(b)「打鍵前状態」を
belief自身（`effective_open()`）から取るため、beliefが実機と食い違って
いるという本ADRの動機そのものと循環する、の2点。詳細は本ファイル
frontmatter statusに要約。

## 決定（案、v2、opus-adversarial-consult未実施）

### なぜ「Standardアプリで学習・TsfNativeアプリで適用」が両方向・
非循環に観測できるのか

`AppImeProfile::Standard`（`crates/awase-windows/src/focus/
class_names.rs:95-108`）は`can_read_imm32_open_status()`と
`can_use_imm32_cross_process()`が共に`true`——`ImmGetOpenStatus`を
**直接・同期的・信頼できる形で**呼べる（`ime.rs::read_ime_state_fast`/
`read_ime_state_fast_async`が既存、`ObservationSource::
ImmGetOpenStatus`という専用の高信頼observation sourceも既存）。

- **双方向性**: `ImmGetOpenStatus`は真偽値をそのまま返す実APIであり、
  `ConvOpenInference`のような「conv ビットからON方向だけを間接推測
  する」制約が無い。ON→OFF/OFF→ON のどちらの遷移も同じ確度で直接
  観測できる（round1のB2はここで解消）。
- **非循環性**: 「打鍵前状態」を`effective_open()`（awase自身の
  belief）からではなく、**`ImmGetOpenStatus`の直接呼び出し結果**から
  取る。Standardアプリではこの値がbeliefより信頼できる（それが
  `AppImeProfile::Standard`の定義そのもの）ため、「beliefが食い違って
  いるかもしれない」という本ADRの動機と衝突しない（round1のB4は
  ここで解消）。
- **NICOLA親指キー除外が不要な理由**: 較正フェーズ・適用フェーズの
  どちらも、対象打鍵は`effective_open() == false`（直接入力、
  Engine非活性）の間にのみ発火する。`Engine::compute_state`が
  `ctx.ime_on`前提でPhase 3（NICOLAチョード判定）を実行しない
  ことは、ADR-174 round1で確定済み（`Engine::on_input_body`の
  Phase 2早期return）。Engine非活性の間はチョード候補としての保留
  自体が発生しないため、対象キーがNICOLA親指キーとして設定されて
  いるかどうかは無関係——round1がこの除外を必要としたのは
  「同一アプリ内・弱い代理シグナル」設計の別の欠陥（B1で指摘された
  設計ミス）への対症療法であり、v2ではその欠陥自体が存在しないため
  除外が不要になる。

### 設計の骨子

1. **較正フェーズ（Standardアプリ）**:
   - トリガー: `AppImeProfile::Standard`のウィンドウで、対象VKの物理
     （非注入）KeyDownかつ`explicit_ime_action_consumed`が立っていない
     （ADR-153の明示config経路が既にこのキーを処理していない）。
   - `ImmGetOpenStatus`を打鍵**直前**に同期読み取り（`read_ime_state_fast`
     と同型の経路、Standardアプリなので高速・信頼できる）し
     `pre_open: bool`として記録。
   - 生キーはそのままパススルーする（一切変更しない）。
   - 打鍵**直後**（メッセージループの次tick、または短い遅延）に
     再度`ImmGetOpenStatus`を読み取り`post_open: bool`とする。
   - `(vk, pre_open)`をキーとする2セル表に`post_open`の観測を記録する
     （下記「学習の確定条件」参照）。

2. **学習の確定条件（2セル表方式、round1レビュアー提案・ユーザー承認
   済み）**: セル`(vk, pre_open)`は、**同じ`post_open`値を2回連続で
   観測するまで未確定**とする（GJI候補ウィンドウのflicker等による
   一発の偽陽性を弾く、BUG-19と同種の教訓）。矛盾する観測
   （確定済みセルと逆の`post_open`）が来たら、そのセルを未確定へ
   戻す（即座に上書きしない——回数を数え直す）。両セル
   （`pre_open=true`と`pre_open=false`）が確定して初めて、そのVKの
   意味論（`TurnOn`/`TurnOff`/`Toggle`/`NoEffect`）を導出できる。
   片方のセルだけ確定している間は、そのセルが表す状態からの遷移
   だけ「部分的に分かっている」として扱い、反対状態からの遷移は
   `config1.db`ベースの静的分類（存在すれば）または「不明」とする。

3. **適用フェーズ（TsfNative/Imm32Unavailableアプリ）**:
   - トリガー: `cannot_verify_real_ime_state()`（TsfNative/
     Imm32Unavailable/InputRelay）が`true`のアプリで、対象VKの物理
     KeyDownかつ`explicit_ime_action_consumed`が立っておらず、その
     VKについて確定済みの学習結果が存在する場合。
   - 生キーはそのままパススルーする（一切変更しない、round1から
     変更なし）。
   - **`kp_stage_shadow_ime_toggle`の`shadow_action`/intentディス
     パッチ経由ではなく**、`ImeEvent::ObserverReported`
     （新設`ObservationSource`、下記参照）として**受動的に**belief
     へ報告する。これが round1 B3（「beliefを書き換えないから安全」が
     方向によって成り立たない）を解消する核心——観測経路
     （`ObserverReported`）は`.claude/rules/ime-belief-architecture.md`
     が定める三層分離の「Observe」に正しく位置づけられ、それ自体は
     いかなる方向についても新しいactuationを一切発行しない
     （actuationは`check_drift_correction`が明示的な相反する
     ユーザー意図の存在を条件に発火するのみ——ADR-174 round1〜3で
     既に検証済みの安全弁がそのまま適用される）。
   - 新しい`ObservationSource`（例: `LearnedModeKeyBehavior`）を
     `state/ime_event.rs`に追加する。`ConvOpenInference`とは異なり
     `pre_open`/`post_open`の直接観測に基づくため、confidenceは
     `Medium`（`ConvOpenInference`と同等、間接的な転用であることを
     鑑みて`High`は名乗らない）。

4. **Toggle意味論の扱い（未解決、round2で詰める）**: `TurnOn`/
   `TurnOff`は絶対状態の報告なのでそのまま`ObserverReported`できるが、
   `Toggle`と学習された場合、適用フェーズで「反転後の状態」を
   報告するには適用先アプリでの現在状態を知る必要がある——
   TsfNative/Imm32Unavailableアプリではまさにそれが信頼できない
   ため、循環に戻ってしまう。**v2でも未解決**。現時点の暫定方針:
   `Toggle`と学習されたキーは適用フェーズの対象から除外し
   （`TurnOn`/`TurnOff`のみ適用対象とする）、`config1.db`ベースの
   静的分類（ATOKプリセットの`Toggle`等、既存の`ShadowImeAction::
   Toggle`表現）に委ねる。

5. **学習結果の保存先とライフサイクル（未解決、round2で詰める）**:
   `henkan_shadow_override`等（GJIアクティブ区間ごとにリセット、
   `runtime/mod.rs:296-297`）とは異なる新しいフィールドが必要——
   較正はStandardアプリで行われ適用はTsfNativeアプリで行われるため
   **アプリをまたいで保持する必要がある**。候補:
   (a) 現在のIME種別（GJI/MS-IME、`sync_ime_kind_from_observation`が
   検出）が変わるまで保持する、(b) プロセス生存期間中は常に保持する、
   のいずれか。前者はIME切替後の誤適用を防げるが後者より複雑。
   `config1.db`ベースのoverride（GJIアクティブ区間ごとにリセット）
   とは異なるライフサイクルになることをADRに明記する必要がある。

### round1のBlockerがv2でどう解消されるか（要約）

| round1 Blocker | v2での解消 |
|---|---|
| B1: 親指キー除外が対象キー自身を全滅させる | 除外を撤廃。較正・適用とも`effective_open()==false`の間だけ発火し、Engine非活性中はチョード判定自体が動かないため除外不要 |
| B2: `ConvOpenInference`はON方向専用 | 較正フェーズは`ImmGetOpenStatus`の直接読み取り（双方向）を使う。`ConvOpenInference`は使わない |
| B3: 「beliefを書き換えないから安全」がOFF→ON方向にしか成り立たない | 適用フェーズは`shadow_action`/intentディスパッチを経由せず、`ObserverReported`（受動観測、actuationを発行しない経路）のみを使う |
| B4: 相関判定の「打鍵前状態」がbeliefで循環論法 | 較正フェーズの「打鍵前状態」は`ImmGetOpenStatus`の直接読み取り（Standardアプリでは信頼できる）から取る。beliefは使わない |
| B5: 単一の弱い観測を広い範囲の決定に使う | 2セル表方式・2回一致確定（ユーザー承認済み）を採用 |

## 未解決・opus-adversarial-consult round2で詰めるべき点

1. **較正の相関判定の詳細**: 「打鍵直前」「打鍵直後」の正確なタイミング
   （メッセージループの何tick後か、`ImmGetOpenStatus`のクロスプロセス
   呼び出し自体のレイテンシ・ハングリスクをどう扱うか）。
2. **Toggle意味論**（上記4）。
3. **保存先とライフサイクル**（上記5）。
4. **対象キーの優先順位**: `VK_CONVERT`/`VK_NONCONVERT`から始めるか、
   `VK_KANA`等の「固定方向のはず」のキーまで最初から含めるか。
5. **`config1.db`ベースの静的分類との優先順位**: 学習結果と
   `config1.db`由来の分類が食い違った場合、どちらを優先するか
   （実機で観測された学習結果を優先すべきという直感はあるが、
   単一のflickerで確定した学習結果が正しいconfig1.db分類を上書き
   するリスクとのトレードオフ）。
6. **`fix-requires-evidence.md`対応**: 回帰テスト（較正の確定条件・
   適用フェーズの発火条件を純粋関数化してLinux CIで実行可能にする）、
   および新しい`ObservationSource`variantの`lints/
   observation_source_guard`・`tests/architecture_guard.rs`への
   登録が必要か確認する。
7. **実機検証方法**: メモ帳等のStandardアプリで較正が正しく発火・
   確定することを実機ログで確認し、その後Windows Terminal等の
   TsfNativeアプリへ切り替えて学習結果が適用されることを確認する
   A/B手順を用意する。

## 非スコープ

- 能動的なテストキー送信によるプロービング（上記「却下した代替案」）。
- `config1.db`静的パース自体の廃止（ADR-092/135/141/BUG-143の資産は
  引き続き初期値として使う）。
- 学習結果のプロセス再起動を跨いだ永続化。
- `Toggle`意味論の学習ベース適用（上記「未解決点」2、当面は
  `config1.db`ベースの静的分類に委ねる）。

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
