---
id: ADR-176
title: |-
  awase-settingsの明示的な較正UIでモードキーの実効果を測定し、
  config1.db/レジストリより優先して適用する
status: |-
  **2026-09-16: round1〜3でBlocker計13件検出、v4へ全面訂正
  （opus-adversarial-consult未実施）。**

  - **round1**（同一アプリ内、`ConvOpenInference`という弱い代理
    シグナルから学習、NICOLA親指キー除外つき）: Blocker5件。
  - **round2（v2）**: 「Standardアプリで`ImmGetOpenStatus`直接読み取り
    により較正し、`ObserverReported`として転用する」方式へ書き直したが
    再びBlocker5件（`ObserverReported`が無条件でdrift correctionに
    到達、`IntentStore`/`last_intent`が未検討、2セル表完成に必要な
    `pre_open=true`側が原理的に取れない、Standardアプリでのawase自身の
    actuationによる汚染、モードキー直後の新規クロスプロセス読み取りが
    BUG-113の「@」独立十分条件）。
  - **round3（v3）**: 学習結果を`ObserverReported`ではなく`config1.db`と
    同じ`shadow_action`供給層に、`config1.dbが未割当のときだけ埋める
    補完」として置く方式へ書き直し、対象もOFF→ON方向のみに縮小。
    10件中5件は構造的に解消したが、新たにBlocker3件——(B1)「OFF→ON
    方向にはactuationが無い」という中核の安全主張が実コード上偽
    （Engine活性化の`ActivationSync`経路で実際に`VK_IME_ON`が送信され、
    BUG-113のADR-149追記が実機で2〜3回の送信を確認済み）、(B2)
    「config1.dbの分類がNoneならawaseは何もしない」も偽（既定の
    `Ctrl+変換`/`Ctrl+無変換`コンボ自体が汚染源になる、MS-IME
    レジストリ由来のoverride・`keys.ime_detect`のsync_directionも
    独立した反例）、(B3)較正が依存する既存observationが打鍵ゲート
    （タイピング中500ms抑制）とintentゲート（明示意図中はポーリング
    停止）で塞がれ、観測が来ないか相関窓が無限になる。加えてM5として
    「config1.dbが未割当のときだけ埋める」に縮小した結果、当初の動機
    （config1.dbが**間違って**分類している場合の自己修復）自体を
    原理的にカバーできなくなっている、という指摘だった。

  **ユーザーによる再訂正（2点）**:

  1. **B1（`VK_IME_ON`の再送信）は本ADR固有の問題ではなく、
     `ActivationSync`側の既存の残置バグである。** GJIが変換キーで
     実際にIMEを開いた（＝分類が正しい）場合でも、Engine活性化に
     伴う`ActivationSync`が`VK_IME_ON`を再送するのは「二重送信」で
     あり、これは`config1.db`経由の既存の`henkan_shadow_override`
     機構（BUG-143で実機確認済み）にも同様に存在する（BUG-113の
     ADR-149追記が「3回→2回」と記録している、その残る2回目）。
     本ADRを理由に発生する問題ではなく、`ActivationSync`側に
     「beliefが既に高信頼度で実状態と一致していれば再送しない」という
     冪等性チェックを足すべき、既存の別問題として切り分ける。
  2. **B3（「観測が来ない」）は設計の前提そのものが誤解だった。**
     本ADRが想定する較正は、通常のタイピング中に**サイレントに
     バックグラウンドで**行うものではなく、**`awase-settings`
     （設定UIアプリ、`AppImeProfile::Standard`）に新設する専用の
     較正パネルで、ユーザーが明示的に協力する形で**行う。ユーザーが
     パネルを開き、案内に従って対象キーを単独タップし、awase-settings
     が**その場で専用の観測ループ**（通常実行時のタイピングガード・
     ポーリング抑制とは無関係、必要ならTSF/COMインターフェースも
     使える）でIME状態の変化を直接監視する。「観測が来ない」という
     心配は、この専用UIフローでは当てはまらない。

  さらに優先順位について: `keys.ime_on`/`keys.ime_off`等の明示config
  （`Ctrl+変換`等）は「GJIの既定動作を意図的に上書きする」思想であり
  最優先・較正の対象外とする。MS-IMEレジストリ由来のoverride・
  `keys.ime_detect`は較正結果と同じレイヤの競合であり、矛盾時は
  **較正結果（実測）を優先**する方針とする。下記「決定（案、v4）」に
  全面的に書き直した。
related_adr:
  - "ADR-092"
  - "ADR-115"
  - "ADR-135"
  - "ADR-140"
  - "ADR-141"
  - "ADR-149"
  - "ADR-153"
  - "ADR-174"
  - "ADR-175"
---

# ADR-176: awase-settingsの明示的な較正UIでモードキーの実効果を測定し、config1.db/レジストリより優先して適用する

## 背景

[ADR-174](174-solo-tap-passthrough-belief-reobservation.md)（BUG-143）で、
GJIの`config1.db`を静的パースして無変換/変換キーのIME意味論
（`ImeToggleKind::On/Off/Toggle`）を判定する`classify_mode_key_ime_action`
（`crates/awase-windows/src/gji_charset_autodetect.rs`）を修正した。
修正自体は実機で正しく動作することを確認済みだが、修正直後にMozc
公式ソース（`google/mozc`）を調査した結果、`config1.db`の
`session_keymap`と`custom_keymap_table`が食い違いうる（GUI実装の
クリア漏れ）という既知の限界が判明した（詳細はBUG-143参照）。

つまり`config1.db`の静的パースは、Google非公開の内部フォーマットを
解釈しているだけでなく、そのフォーマットが実際のGJIバイナリの挙動を
正確に表しているという保証も無い。同様に、MS-IME使用時の判定は
レジストリ値の読み取りに依存しており、これも「設定の記述」と
「実際の挙動」が食い違いうる。

## 目的

`config1.db`/レジストリの静的パースに頼らず、**ユーザーが
awase-settingsの専用UIで対象キーを実際に打鍵し、その結果（IMEが実際に
ON/OFFどちらに動いたか）を直接測定して**、モードキー（変換/無変換/
かな/漢字等）の意味論をawaseが正しく把握できるようにする。測定結果は
`config1.db`ベースの静的分類と同じ`shadow_action`供給層に、**より高い
優先度で**適用する——静的パースが誤っていた場合でも、実測した事実が
勝つ。

**round1〜3との違い**: round1〜3は「通常のタイピング中にバックグラウンド
で受動的に学習する」という設計だったため、(a)観測チャネルの信頼性・
到達性、(b)NICOLA親指キーのチョード判定との衝突、(c)awase自身の
actuationによる観測の汚染、といった問題が繰り返し発生した。
v4は**ユーザーが明示的に協力する専用UIフロー**に変更することで、
これらの問題の多くを構造的に解消する（詳細は「決定（案、v4）」参照）。

## 対象キー

`vk.rs::is_ime_mode_key_for_ime`が対象とする範囲のうち、
`config1.db`/レジストリベースの静的分類が存在しうるキー
（`ModeKeyCandidate::{Henkan, Muhenkan, Hiragana, Katakana}`、
`crates/awase-windows/src/gji_charset_autodetect.rs:224-229`）を主対象と
する。`VK_KANA`等「Win32 API上は固定方向のはず」のキー
（`ImeKeyKind::shadow_effect()`が既に固定方向で判定済み）は、
[ADR-175](175-physical-dbe-key-stuck-direction-recovery.md)の教訓
（固定方向前提も実機で裏切られうる）を踏まえれば較正UIの対象に
含める価値はあるが、v4では優先度を下げ非スコープとする（下記参照）。

## 却下した代替案

### 能動的なテストキー送信によるプロービング（通常実行時）

「起動時やGJI検出時に、awase自身が対象キーを合成SendInputで送信し、
その結果を観測してキャリブレーションする」案は**却下**する。ADR-153
ケース3の実機履歴（`docs/known-bugs/BUG-113.md`・`BUG-124.md`）で、
「生キーがGJIへ届くこと」「awase自身が明示IME制御actuationを行う
こと」のどちらか片方だけでも「@」を誘発するのに十分と2回の独立した
実機A/Bで確定している。**ただしこれは「通常実行時、ユーザーの
意図しないタイミングで」合成キーを送る場合の話である**——v4の較正UI
は、ユーザーが明示的に較正モードへ入り、実際に物理キーを押す
（awase自身はキーを送信しない）ため、この却下理由には抵触しない。

### 却下（round1）: 同一アプリ内の弱い代理シグナルによる学習

round1は「TsfNativeアプリ内で、次の実キー入力が自然に発生させる
`ConvOpenInference`観測と、直前の`effective_open()`スナップショットを
突き合わせて学習する」という設計だった。Blocker5件により**却下**
（詳細はfrontmatter status参照）。

### 却下（round2）: `ImmGetOpenStatus`直接読み取り＋`ObserverReported`
としての適用（通常実行時のバックグラウンド較正）

round2（v2）は通常実行時にバックグラウンドで較正し、適用は
`ObserverReported`（belief層への受動観測）とする設計だった。
Blocker5件により**却下**（詳細はfrontmatter status参照）。

### 却下（round3）: `config1.db`が未割当のときだけ埋める補完
（通常実行時のバックグラウンド較正、`shadow_action`供給層への合流）

round3（v3）は学習結果の置き場所を`shadow_action`供給層へ正しく
修正したが、較正自体は依然として通常実行時のバックグラウンド処理
だった。Blocker3件（`ActivationSync`の再送信、既定config設定による
自己汚染、観測が来ない/相関窓が無限）により**却下**——ただし
Blocker3件のうち2件（`ActivationSync`再送信、観測到達性）はv4の
「専用UIフローへの変更」で解消する。

## 決定（案、v4、opus-adversarial-consult未実施）

### 設計の骨子

1. **較正UI（`awase-settings`に新設）**:
   - `awase-settings`（`AppImeProfile::Standard`、`can_read_imm32_
     open_status()`/`can_use_imm32_cross_process()`が共に`true`）に
     「IMEキー較正」パネルを新設する。
   - ユーザーが対象キー（`変換`/`無変換`等、選択式）を指定し、
     「較正開始」を押す。
   - awase-settingsは較正対象キーが物理・非注入・修飾キー無し
     （Ctrl/Shift/Alt/Winいずれも押されていない——`Ctrl+変換`等の
     awase側明示config用の組み合わせと衝突しないため）で押されるまで
     待機する。
   - 押される**直前**の実IME open状態を、awase-settings自身のウィンドウ
     に対する`ImmGetOpenStatus`（Standardプロファイルなので信頼できる、
     `can_read_imm32_open_status()==true`）で確認する。open状態が
     `false`でなければ（＝直接入力状態でなければ）ユーザーに直接入力へ
     切り替えるよう案内し、待機し直す。
   - キーを検知したら生キーはそのままOS/IMEへ渡す（awase自身は一切
     actuateしない）。
   - **専用の観測ループ**（通常実行時のタイピングガード・ポーリング
     抑制ロジックとは完全に独立、`awase-settings`プロセス内で完結）で
     `ImmGetOpenStatus`を短い間隔（実測して`tuning-constants.md`に
     従い決定）でポーリングし、open状態がtrueへ変化するのを待つ。
     必要ならTSF/COMインターフェース（`ITfThreadMgr`等）による、
     より高精度・低レイテンシな観測も検討する（`awase-settings`
     自身のウィンドウなので、クロスプロセス呼び出しの制約
     （BUG-034のハングリスク等）はawase.exe本体の打鍵経路ほど
     厳しくない——ユーザーが明示的に待っている一度きりの操作であり、
     数百ms〜数秒のレイテンシは許容できる）。
   - タイムアウト（実測して決定）までに変化が観測できなければ
     「このキーはIME状態を変えないようです」とユーザーに提示する。
   - 変化を観測できたら「変換キーはIMEをONにするようです。もう一度
     確認しますか？」と確認を促し、**2回一致**するまで確定しない
     （round1レビュアー提案・ユーザー承認済みの方針を維持）。

2. **保存**: 確定した結果を`config.toml`の新設セクション（例:
   `[gji_measured_overrides]` または既存の`app_overrides`と同系統の
   構造）に永続化する。既存の`henkan_shadow_override`等とは異なり
   **プロセス再起動を跨いで永続化する**——これはユーザーが明示的に
   時間をかけて測定した結果であり、`config1.db`のような外部ファイル
   由来の値より安定した情報だと判断できるため（非スコープ節も参照）。

3. **適用（awase.exe本体、通常実行時）**: 設定リロード時
   （`reset_streak_latch_for_reload`と同じ配線点、
   `gji_charset_autodetect.rs:665-668`）に`config.toml`の測定済み
   overrideを読み込み、`henkan_shadow_override`/`muhenkan_shadow_override`
   等と**同じ`shadow_action`供給層**（`resolve_henkan_muhenkan_shadow_
   override_for_event`等）に合流させる。適用フェーズ自体は
   `config1.db`ベースの経路と全く同じコードパスを通るため、新しい
   actuation合流点はゼロ（既存のADR-141機構をそのまま再利用）。

4. **優先順位**（対象VKごとに以下の順で解決、上に行くほど優先）:
   1. `keys.ime_on`/`keys.ime_off`/`keys.ime_toggle`
      （`Ctrl+変換`等、awase側の明示config） — 較正の対象外・
      常に最優先（GJIの既定動作を意図的に上書きする思想のため）。
   2. **本ADRの較正結果**（`config.toml`の測定済みoverride、確定済み
      のもの） — 実測した事実を静的パースより優先する。
   3. `config1.db`ベースの静的分類（GJI）/レジストリ由来の分類
      （MS-IME）、および`keys.ime_detect`の`sync_direction`
      ——これらは互いに同じレイヤの競合として扱われ、既存の優先順位
      （`intent_kind`解決、`key_pipeline.rs`）を維持する。
   較正結果が存在しないVKについては、従来どおり3の静的分類にフォール
   バックする。

5. **`ActivationSync`の再送信は別問題として切り離す**: Engine活性化に
   伴う`VK_IME_ON`の冗長送信（BUG-113 ADR-149追記、「3回→2回」の残る
   2回目）は、本ADRの較正が正しく機能した場合でも既存の`config1.db`
   経路と同様に発生しうる**既存の残置問題**であり、本ADRのスコープ
   外とする。ただし本ADRの実装前提として、`ActivationSync`（
   `src/engine/engine.rs:456-475`）に「beliefが既に高信頼度で実状態と
   一致していれば`SetOpen`を再送しない」という冪等性チェックを別途
   追加することを推奨する（既存のBUG-113/ADR-149の残課題として、
   本ADRとは独立に、しかし関連するタイミングで対応するのが望ましい）。

### round3のBlockerがv4でどう解消されるか

| round3 Blocker | v4での解消 |
|---|---|
| B1: `ActivationSync`の`VK_IME_ON`再送信 | 本ADR固有の問題ではないと整理し、別途の冪等性チェック追加を推奨事項として切り離す（上記5） |
| B2: `config1.db`分類None時の既定config等による自己汚染 | 較正は専用UIフローでのみ行われ、修飾キー付き打鍵（`Ctrl+変換`等）は較正対象外として明示的に除外する。MS-IMEレジストリ由来のoverride・`keys.ime_detect`との優先順位は上記4で明文化 |
| B3: 既存observationが打鍵ゲート・intentゲートで塞がれる | 較正は`awase-settings`の専用観測ループで行われ、通常実行時のタイピングガード（500ms）・ポーリング抑制（明示意図中）とは無関係 |
| M5: 動機（config1.dbが誤って分類している場合の自己修復）をカバーできない | v4は「未割当のときだけ埋める」制約を撤廃し、較正結果が存在すれば静的分類より優先するため、当初の動機を再びカバーする |

## 未解決・opus-adversarial-consult round4で詰めるべき点

1. **`awase-settings`と`awase.exe`本体の連携方法**: 較正結果を
   `config.toml`へ書き込んだ後、実行中の`awase.exe`にどう反映させるか
   （設定リロードのトリガー、既存の設定変更検知機構があるか確認）。
2. **観測ループの具体的な実装**: ポーリング間隔・タイムアウトの実測値
   （`tuning-constants.md`対象）、TSF/COMインターフェースを使う場合の
   実装コスト・複雑性とのトレードオフ。
3. **優先順位（上記4）の実装箇所**: `intent_kind`解決
   （`key_pipeline.rs:1417-1427`付近）にどう新しい優先順位を挿入する
   か、既存の`sync_direction` > `shadow_action`という順序とどう統合
   するか。
4. **`config.toml`への永続化の設計**: 新設セクションのスキーマ、
   既存の`app_overrides`等の設定ブロックとの一貫性。
5. **UI/UX設計**: 較正パネルの具体的な操作フロー、失敗時
   （キーが検出されない、タイムアウトする等）のエラーメッセージ。
6. **`fix-requires-evidence.md`対応**: 較正ロジック自体は
   `awase-settings`側の新規コードだが、適用フェーズ（優先順位解決）は
   `awase-windows`側の「IME belief」「IME actuation合流点」ファミリー
   に該当するため回帰テストが必要。
7. **`ActivationSync`冪等性チェック**（上記5）の要否・実装方針。
   本ADRとは別のBUGとして起票するか、本ADRの実装に含めるか。

## 非スコープ

- 通常実行時のバックグラウンドでの受動的な学習（round1〜3の設計、
  却下済み）。
- ON→OFF方向・Toggle意味論の較正UI対応（v4の初期スコープはOFF→ON
  方向の較正に集中する。ON→OFF方向はユーザーが直接入力からの
  遷移だけを較正するUIフローと相性が悪いため、必要なら別途検討）。
- `VK_KANA`等「固定方向のはず」のキーの較正（対象キー節参照、
  優先度を下げて非スコープとする）。
- `ActivationSync`の冪等性チェック自体の実装（上記「未解決点」7、
  関連課題として記録するが本ADRの実装スコープには含めない）。

## 関連

ADR-092（`classify_mode_key_ime_action`の起源）、ADR-115（打鍵列機能、
本ADRとは無関係だが同じ「1キーに複数の意味を持たせる」領域）、
ADR-135（Hiragana/Katakanaへの一般化）、ADR-140（probe/actuation競合）、
ADR-141（Henkan/Muhenkan delegate、`shadow_action` override機構
そのもの）、ADR-149（`VK_IME_ON`重複送信の根本原因調査、
`ActivationSync`再送信の既存文脈）、ADR-153（明示config、`Ctrl+変換`
等が較正対象外になる理由）、ADR-174/BUG-143（本ADRの直接の動機）、
ADR-175（BUG-142、「固定方向のはず」を信じすぎる失敗モードの先例）。
