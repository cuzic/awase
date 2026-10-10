---
id: ADR-255
title: |-
  IME OFF のときだけ無変換/変換を Space にする——エンジンの非活性時の親指キー処理に足す(`[[keymap]]` の条件化は採らない)
summary: |-
  顧客報告「IME OFF のとき GJI の設定が反映されず、変換/無変換を空白入力に割り当てても動かない」を GitHub Windows CI で実機検証した(ブランチ ci/e2e-direct-space、run 38058313464・38059338837、各セル n=1)。GJI の CUSTOM 表は読まれ、直接入力の無変換に IMEOn を割り当てると効く(open 0→1。変換は既に開いた状態で試したので未分離)。直接入力の DirectInput 行に InsertSpace/InsertHalfSpace/InsertFullSpace を割り当てても入力欄に空白は入らなかったが、Precomposition の対照でも空白が入らず、「直接入力では不可」と「観測・キー名・コマンドの対象外」を分けられていない(GJI の仕様で不可とは断定しない)。そこで GJI に頼らず awase 側で、エンジンが非活性(理由 ImeOff)のときの無変換/変換の単独押下を Space にする設定を足す。Win32/IMM 系のウィンドウが対象で、IME の状態を awase が読めないアプリ(Chrome・Edge・VS Code・Windows Terminal・UWP・コンソール・RDP 等。`cannot_verify_real_ime_state`)では IME OFF を継続して確かめる根拠が無いため、明示的に非対応とする(r2 R2-B1、r3 R3-M1)。発動するのは IMM で状態を読める古典的な Win32 の窓だけで、報告者のアプリがそこに入るかの確認を実装着手の条件にする。当初案の `[[keymap]]` への `ime` 条件は、Opus レビュー r1 で、エンジンの活性判定と別の値を見て親指シフトが Space に化ける(B1)・未確定文字列の破棄(B2)・親指ラッチと latch の stale(M3/M4)が指摘されたため採らない。
status: |-
  起草 → Opus レビュー r1〜r3 反映済み(2026-10-10)。r4(同じレビュアーへの再確認)待ち。実装は未着手。
related_adr:
  - "ADR-114"
  - "ADR-206"
  - "ADR-141"
  - "ADR-230"
  - "ADR-245"
---

# ADR-255: IME OFF のときだけ無変換/変換を Space にする

## ステータス

起草 → Opus レビュー r1(Blocker 2・Must 7・Should 9・代案 3、[review/255-opus-review-round1.md](review/255-opus-review-round1.md))・r2(Blocker 1・Must 5・Should 7、[review/255-opus-review-round2.md](review/255-opus-review-round2.md))・r3(Blocker 1・Must 2・Should 4、[review/255-opus-review-round3.md](review/255-opus-review-round3.md))を反映(2026-10-10)。r4 待ち。実装は未着手。

## コンテキスト

### 報告

「IME OFF のとき、Google 日本語入力の設定が反映されず、変換/無変換を空白入力に割り当てても期待どおり動作しない」。

### 実機検証(GitHub Actions windows-latest、ブランチ `ci/e2e-direct-space`)

構成名 `sc-direct-space-*`(`.github/workflows/e2e-ime.yml`、判定なしの回収)。スパイクが IME OFF(`--seq` の 1A)にしてから無変換(0x1D)/変換(0x1C)を注入し、各押下の +1500ms の入力欄末尾(`tail`)と未確定(`comp`)を記録する。**各セル n=1**。run 38058313464(10 構成)、38059338837(対照を足した 13 構成)。成果物は 7 日で失効する。

| 構成(CUSTOM 表の行) | awase | 結果 |
| --- | --- | --- |
| 行なし(基準。`DirectInput,ON,IMEOn` のみ) | なし/あり | 無変換・変換とも open は 0 のまま、空白なし |
| DirectInput の無変換/変換 = InsertSpace | なし/あり/あり(単独タップ Suppress 解除) | 空白なし |
| 同 InsertHalfSpace / InsertFullSpace | なし/あり | 空白なし |
| 対照: DirectInput の無変換/変換 = IMEOn(`--seq=1A,1D,1C,1D`) | なし/あり | **最初の無変換で open 0→1**(両方)。変換は既に開いた状態で押したので、変換の行が効くかは未分離 |
| 対照: Precomposition の無変換/変換 = InsertSpace | なし/あり/あり(Suppress 解除) | IME は開くが空白なし |

### 分かったことと分かっていないこと

- 分かった: (1) CUSTOM 表は読まれ、`Muhenkan` というキー名は GJI に通じ、直接入力の行の**状態を変えるコマンド**(IMEOn)は効く。(2) 直接入力の InsertSpace 系は空白を出さなかった。
- **分かっていない**: Precomposition の対照でも空白が出ないので、次の仮説を分けられていない: (a) GJI の直接入力状態では InsertSpace 系は割り当ての対象外(設定ダイアログの直接入力行で選べるかを確かめれば切れる。未確認)、(b) 無変換/変換というキーでは InsertSpace が文字を出さない、(c) 入力欄の観測(`tail`)が空白を捉えていない(スペースキー自体を注入する正の対照が無い)。したがって「GJI の仕様で不可」とは書かない。本 ADR の決定は GJI の振る舞いに依存しない。
- フックの配送(訂正): フックは Accepted のキーを常に握りつぶし(`hook.rs` の `LRESULT(1)`)、エンジンの判断後にメインスレッドが**再注入**する。「awase は IME OFF の無変換/変換を素通しする」は正確には「再注入で GJI に届ける」。CI で awase あり/なしの結果が同じなので結論は変わらない。`physical_disposition.rs` の無変換/変換の Allow は「再注入する」の意味。

### 既存の機構

エンジンは非活性のとき bare の親指キーを `src/engine/engine.rs::thumb_open_role_action`(役割由来の開閉)で扱う。入口の条件に `compute_active`・`is_japanese_ime`・`is_bare_thumb`(物理・無修飾・非注入)・`sync_direction` の除外が揃う。エンジンの活性は `compute_state`(`ctx.ime_on`)が決める。

## 検討した案

- **案 A1(採用)**: エンジンの非活性時の親指キー処理に Space 出力を足す。
- **案 A2(不採用)**: `[[keymap]]`(ADR-114)に `ime = "off"` の条件を足す(当初案)。Opus r1 の指摘: (B1) `[[keymap]]` の照合はエンジンの活性判定と別の値を見るので、エンジンは活性(親指として使う)なのに `[[keymap]]` が先に消費して Space にしうる(`effective_open()` には IntentStore の上書きが重なり `resolve_open_at` と一致しない)。(B2) 誤って「閉」と判定すると `consume_keymap_match` が未確定文字列を破棄する。(M3) フック側の親指ラッチは `[[keymap]]` と無関係に立つ。(M4) latch が stale に残ると IME ON の親指打鍵が消える。(M5) 他ツールが注入する無変換が Space になる。いずれも、エンジンの内側に置けば構造的に消える。一般化(`on` を含む)には消費者もいない(S4)。
- **案 A3(不採用)**: GJI の CUSTOM 表を awase が生成する(ADR-231)。CI で InsertSpace が効いていないので、現時点の証拠では採れない。MS-IME 利用者にも効かない。

## 決定

### 決定1: 設定項目

`[general]` に `thumb_key_when_ime_off = "unchanged" | "space"` を足す。既定は `"unchanged"`(今までと同じ)。名前は仮(疑問5)。`set_space_thumb_config`/`space_thumb_vk`(Space を親指キーにする構成)や単独タップの `Suppress/Passthrough`(`muhenkan_solo_tap_*`)と語が紛れないようにした(R2-S6)。無変換/変換の両方に効く。`left_thumb_key`/`right_thumb_key` が無変換/変換のときだけで、それ以外のキーを親指にしている構成では何もしない。

### 決定2: 発動条件(エンジンの内側、すべて AND)

`engine.rs` の `match_special_keys` の `or_else` 連鎖の**末尾**(`thumb_open_role_action` の後。`engine_on` コンボ・`keys.ime_*`・自動検出トグルより後で、緊急復帰経路やユーザー設定を奪わない。R2-M4)に新しい種別(`SpecialKeyMatch` の新 variant、仮に `ThumbSpaceWhenImeOff`)を足し、次をすべて満たすときだけ Space を出す。

1. `!compute_active(ctx)` かつ非活性の理由が `ImeOff`(エンジン自身の判定。r1 B1 を構造的に解消。`ctx.ime_on` は IntentStore の上書きを含む `effective_open()` なので、上書きで「開」ならエンジンは活性になり発動しない)。`UserDisabled`/`NotRomajiInput` は今回含めない(決定7)。
2. `ctx.is_japanese_ime` かつ `adapter.is_enabled()`。
3. `is_bare_thumb(event, ctx.modifiers)`(`key_classification` が親指で、Shift・OS 修飾なし、**非注入**。alt-ime-ahk など他ツールが注入する無変換を除く。r1 M5)。**加えて、Alt なりすまし(`left/right_alt_impersonates_thumb_key`)由来の打鍵を除く**: フックは `cached_engine_enabled` が真のとき Alt を無変換に書き換えるが、このキャッシュは `EngineStateChanged` でしか更新されず、GJI 側で IME が閉じた直後の最初の打鍵が Alt だとまだ真のままで、Alt が Space になる(R2-M2)。書き換え後の vk では区別できないので、**フックが書き換えたときに `RawKeyEvent` へ `impersonated: bool` の印を付ける**(`apply_alt_impersonation`、`hook.rs`)。scan code(Alt 0x38 / 無変換 0x7B)での判定は、右 Alt→変換(0x79)の取りこぼしや Scancode Map・リマッパで scan が変わる問題があるので採らない(R3-S1)。印は境界 journal(ADR-250)にも乗る。`is_bare_thumb` の「物理」はなりすましを含むので、本条件で別に除く(R2-S7)。
4. **このキーが IME の機能を持たない**こと(R2-M1、R3-B1)。次の2つを**両方**満たす。
   - (i) 従来の4源がすべて None: `bare_ime_action(vk)`(`keys.ime_*`)、`thumb_role_open_actions()` の該当側(単独タップ設定が Passthrough かどうかを問わない)、`muhenkan_solo_tap_dedicated_fn_key`(専用 Fn キー)、`event.ime_relevance.sync_direction`。`thumb_open_role_action` は流用しない(専用 Fn キー設定済み、または役割があっても単独タップ設定が Passthrough でないとき `None` を返す。この場合エンジン非活性では生キーを通して IME 自身に開かせているので、`None` を「役割なし」と読むと IME を開く手段を奪う)。
   - (ii) **このキーの直接入力状態での効果の三値が `NoEffect`**: `KeyOpenEffect = Opens | NoEffect | Unknown`。**発動は `NoEffect` のときだけ**。awase の `KeyRole`(`awase-gji-config/src/role.rs`)は `ImeToggle` の1つだけで、**開くだけの割り当て**(CI の `ctl-loaded` 表の `DirectInput,Henkan,IMEOn` 単独行、GJI の「変換/無変換で IME ON/OFF」オーバーレイ、MS-IME の「変換 = IME-オン」)では None を返すため、(i) だけでは IME を開く手段を奪う。三値の写像は `state/` の純粋関数にして Linux で表を固定する:
     - GJI(表が読める): プリセットまたは CUSTOM 表の `DirectInput` 行にそのキーの行があれば `Opens`(`IMEOn` 以外のコマンド〈`Reconvert`・`InputModeHiragana` 等〉でも**機能ありに数える**。Space で奪わない)。オーバーレイ(`SESSION_KEYMAP_OVERLAY_HENKAN_MUHENKAN_TO_IME_ON_OFF`)があれば `Opens`。行が無ければ `NoEffect`。表が読めない(`Source::Unknown`)・`table_ime_kind` が None は `Unknown`。
     - MS-IME 本体: 無変換/変換の値(0/1/2/3)の意味と「値なし」の既定の効果を確かめていないので、**今回は `Unknown`(発動しない)**。値の意味を実機で確かめた後に別途広げる。
     - TIP 未同定(`!ime_identified`。BUG-179 のように MS-IME を Other と同定する事例がある): `Unknown`。
   - つまり**当面発動するのは、GJI で、表が読めて、そのキーの `DirectInput` 行もオーバーレイも無いとき**だけ。報告(GJI)の構成には合う。決定5の注記「IME 側の割り当てより優先される」が当てはまるのは、awase がそもそも読めない割り当て(GJI の設定ファイル外)に限る。
5. **IME OFF の確からしさ**(r1 M1): シェルが `InputContext` に `ime_off_confirmed: bool` を足す。真になるのは `ObservationStore::derive_actuating(now) == Some(false)`(Actuating プールだけ。High 単独/Medium 無競合)のときだけ。`HeuristicDefault`・`HwndCache`・`ConvOpenInference`・`desired_open` の既定値・明示意図(`ExplicitIntent`)・打鍵時予測だけが根拠のときは偽。写像は `state/` の純粋関数にして Linux で表を固定する。belief は読むだけで書かない。**さらに次のとき偽にする**: (a) InputRelay(RDP・VM・PowerToys MWB。ローカルの IME belief はリモート側の状態を表さない。ADR-206 も InputRelay では役割を付けない。R2-M3)、(b) ADR-245 の「戻り待ち」が立っている間(復元と順序が競合する。R2-S2)、(c) **TSF ネイティブのプロファイル**(次の決定3)。
6. 未確定文字列(composition)が無い(`!ctx.composing`、r1 B2)。composition があるのに「IME OFF」とみなすのは矛盾した証拠なので発動しない。

**不確かなときは今までどおり素通し**(`unchanged` と同じ)。素通しの理由は debug ログに出す(r1 M2)。

### 決定3: IME の状態を読めないアプリでは対応しない(R2-B1、R3-M1)

実コードの確認(Opus r2・r3): 本番で open/close の観測として記録される Actuating ソースは実質 `ObserverPoll`(Medium)と `ImmCrossProbe`(High、ImmCross アプリのフォーカス直後だけ)の2つ。`ObserverPoll` は TSF ネイティブ窓では書かれない(`observer/ime_observer.rs` の `classify_poll_outcome` で `is_tsf_native` のとき `observer_poll: None`)。観測が書かれるのは、外部変化の検出と ADR-188 の窓内の直接読みだけで、`derive_*` は `OBSERVATION_FRESH_WINDOW_MS = 3_000` より古い観測を捨てる。したがってこの種の窓では、IME OFF のまま打っていると `derive_actuating` は `None` になり、OFF にした直後の3秒だけ発動する。**同じアプリで押すタイミングによって Space になったりならなかったりする**機能は、「効かない」より切り分けが難しい。

- 決定: `ime_off_confirmed` を立てるシェルは、述語 **`cannot_verify_real_ime_state`**(`focus/class_names.rs`)が真のアプリでは偽にする。この述語は `Imm32Unavailable`(Chrome・Edge・UWP・XAML・コンソール系)・`TsfNative`(WezTerm・Windows Terminal)・`InputRelay`・Standard でも実質 TSF ネイティブ(`is_effectively_tsf_native`)の窓を含む。**別の述語を使うと InputRelay や Standard+TSF を取りこぼす**。
- したがって**発動するのは、IMM で状態を読める古典的な Win32 の窓だけ**になる。設定画面の注記は「Chrome・Edge・VS Code・Windows Terminal・UWP など、IME の状態を awase が読めないアプリでは動かない」と書く。Windows 11 のメモ帳など XAML/RichEdit 系がどちらに分類されるかは**未確認**(検証計画0 で確かめる)。
- 鮮度を問わない別の根拠(同じフォーカスで最後に確定した Actuating 観測が `false` で、その後に開閉を変えうる打鍵が無いこと)は、新しい判定になり r1 M1 と同じ審査が要るので、今回は採らない。

### 決定3b: 実装着手の条件(ゲート)(R3-M1)

この機能は対象が狭い(上記)ので、**報告者に次の3点を確認するまで実装に着手しない**。(1) どのアプリで使うか、(2) タスクバーの表示が「A」(直接入力)か「A」の半角英数か(`NotRomajiInput` は決定7で範囲外)、(3) 押し続けて Space が連続して出る必要があるか(決定4はリピートしない)。結果に応じて:
- (a) 報告者のアプリが Standard で IMM から状態を読める窓なら実装する。
- (b) 範囲外(Chrome・Edge・VS Code・Windows Terminal・UWP 等)なら実装を保留し、報告者には「GJI の直接入力状態の制約(仮説 a〜c の切り分け結果)」と「awase では現状できない理由」を返す。鮮度を問わない根拠の別 ADR を起こすかは、そのとき判断する。

### 決定4: 出力と Down/Up

- Down で `VK_SPACE` を 1 回タップ(Down+Up)する。`VK_SPACE` はエンジンに生の VK 定数を持たせないため(ADR-019)、`set_space_thumb_config`(`engine.rs`)と同じくプラットフォームから渡す(R2-M4)。
- KeyUp は `KeyLifecycle` の `UpDuty::Consume` で回収する(`[[keymap]]` の latch は使わない)。
- **リピートしない**: 新しい種別にも `!event.was_down` の条件を付ける。既存の `check_special_keys` 先頭のガードは `thumb_open_role_action` 専用で新しい種別には効かず、`phase1_held` は flush 等で消えうるため、付けないとリピートで Space が連射される経路が残る(R2-M4)。無変換を押し続けても Space は 1 個(r1 S5)。本物の Space キーや GJI の InsertSpace(リピートする)とは違う。
- **Phase 1 の早期 return の遅れへの手当**(R2-S1): Phase 1 で Consume すると Phase 2 の `check_active_transition` を通らずに return するので、GJI 側で IME が閉じた直後の最初の打鍵が無変換だと、`EngineStateChanged{false}`(トレイ表示・Alt なりすましのキャッシュ)、保留出力の解放、FSM の flush が次の打鍵まで遅れる。Space 経路は IME を変えないので遅れたままになる。Space の前に `check_active_transition` の effects を前置する(`prepend_effects`)。**実装時の確認(R3-S2)**: `transition_activation` は活性→非活性で `SetOpen{open:false}`(`emit_set_open` が真のとき)と `EngineStateChanged{false}` を出す。IME が既に閉じた窓への `SetOpen(false)` は冪等なはずだが actuation の1件になるので、`emit_set_open` の値と、IME actuation 合流点ファミリー(`fix-requires-evidence.md`)に触れるかを確かめる。触れるなら押下に由来しない起案として `press=None`。
- Platform 側の述語との整合(R2-M4): `match_special_keys` は Platform からも呼ばれる。`matches_ime_set_open`(shadow の書き込みの抑止、`engine_owns_open_key`)と `matches_ime_off`(Ctrl+無変換の救済窓)では、新しい種別は**開閉ではない**ので `None`/`false` を返す。網羅 `match` のテストで固定する。
- 押している間に IME が ON になったとき: 活性化後、文字キーはフックのスナップショット(`ctx.left_thumb_down`)で親指シフト扱いになりうる。既存の役割経路(変換 = IME ON の単独押下)にも同じ窓がある。**期待値は既存の役割経路と同じにする**(実装前に現挙動を調べ、検証計画1で固定する。違えるなら理由を書く。R2-S4)。KeyUp は活性化後に FSM へ Down なしの親指 Up として届く(`engine.rs` は非活性時だけ `release_only`)ので、この挙動も単体テストで固定する。
- 再生(`OUTPUT_GATE`/`INPUT_DEFER` で退避された Down)は、再生時点の状態で評価する(r1 S1)。

### 決定5: 競合と設定画面

- 決定2-4 により、`keys.ime_*`・専用 Fn キー・IME 設定/学習表由来の役割があるキーでは発動しない。重なりは設定読み込み時に警告する(「既存の衝突警告に載せる」ではなく新規。`warn_if_vk_conflicts` は dedicated fn key の2箇所だけで、`keys.ime_*` との衝突警告は実在しない。r1 M6-2)。
- **awase が知らない IME 側の割り当て**(MS-IME の「キーとタッチのカスタマイズ」で無変換/変換に IME オン/オフを割り当てたが、awase が読めていない等)は、この機能が Space で上書きする。設定画面の注記に「IME 側で無変換/変換に割り当てた機能より優先される」と書く(R2-M1)。
- 設定画面は、親指キーの設定の近くに「IME OFF のとき無変換/変換を Space にする」のチェックを置く。注記に、(1) リピートしない、(2) 「半角英数」(IME は開いたまま英数モード)の状態では動かない、(3) TSF ネイティブのアプリでは動かない、(4) IME の状態が確かでないときは何もしない、(5) IME 側の割り当てより優先される、を書く。
- panic 検出(`record_ime_keydown`)は `deliver_key_event` より前に数えるので、Space に変えても計数は増減しない。重なる構成では決定2-4 により発動しないので、速い交互押下は Space 機能と無関係に今と同じ(R2-S7。r1 の検証計画4 は削る)。

### 決定6: リリース

- オプトイン(既定 `"unchanged"`)。Scancode Map(ADR-230)や、かな/無変換/変換の挙動変更とは同時にリリースしない。回帰テスト(検証計画1・2)が通り、develop で実機確認が済むまで次の v2 リリースに入れない。v1 への backport はしない(新機能)。

### 決定7: 範囲外

- `NotRomajiInput`(半角英数)での Space 化(r1 S3)。報告者の「IME OFF」がタスクバーの「A」(半角英数)なのか、直接入力なのかを先に確認する。
- TSF ネイティブでの Space 化(決定3)。
- `[[keymap]]` の条件化、`ime = "on"`(r1 S4)。必要になったら別 ADR。
- GJI の CUSTOM 表の生成(A3)。

### 影響範囲と再発ファミリー(R2-M5)

`src/engine/engine.rs`(`match_special_keys`・新 variant・`on_input_body` の前置)、`src/engine/nicola_fsm.rs`(役割の判定の純粋関数)、`src/config.rs`(設定)、`state/` に `ime_off_confirmed` の純粋関数、`runtime/key_pipeline.rs`(`InputContext` の構築)、`hook.rs`(なりすまし由来の印)、`crates/awase-settings`(チェックと注記)。`fix-requires-evidence.md` の再発ファミリーの**キー選択(IME ON/OFF に送る VK)**(`engine.rs::thumb_open_role_action` はエンジン非活性側の入口として明記されている)と**物理キー押下ラッチ(Down/Up 非対称)**に触れる。同じ PR に (a) 回帰テストを含める。置き場所は `src/engine/tests.rs`(`cargo test --lib`、ホストで実行可)と `state/` の純粋関数のテスト。`runtime/` 配下の `#[cfg(test)]` は Linux に存在しないので使わない。

## 検証計画

0. **実装前の確認と計測**(R2-B1、R3-M1、R3-S3): (i) **報告者への確認3点(決定3b)**。(ii) 入力先の分類の確認: CI の入力先(ADR-193 の RichEdit スーパークラス化は「TsfNative 相当」)と、素の Edit コントロール(Standard の IMM の窓)で、`AppImeProfile` と `cannot_verify_real_ime_state` の値をログで確かめる。Windows 11 のメモ帳の分類もあわせて確かめる。(iii) 計測の目的は「決定3 の範囲(Standard の窓)で、発動すべき場面で発動するか」: Standard では観測が約 500ms 周期で入るのでほぼ常に `Some(false)` のはずで、数える意味があるのは `Unknown` が出る条件(フォーカス直後・プローブ失敗)の頻度。(iv) 正の対照(スペースキー 0x20 の注入で入力欄の `tail` に空白が出るか)、直接入力行の変換(0x1C)の IMEOn、設定ダイアログで直接入力行に InsertSpace を選べるか(仮説 a〜c の切り分け)。(v) 基準構成の「無変換・変換とも open は 0 のまま」は確認済み: run 38059338837 のジョブ `e2e (sc-direct-space-baseline-noawase-1)` と `e2e (sc-direct-space-baseline-awase-1)` の成果物 `dist/ime_key_matrix_spike.log` の KEY 行(2026-10-10 に確認)。各 n=1。
1. **Linux 単体**(`cargo test --lib`、判断は `src/engine` と `state/` の純粋関数): 発動条件の表(非活性理由 × `ime_off_confirmed` × composing × 注入 × なりすまし由来 × 修飾 × 従来の4源 × **`KeyOpenEffect` の各行(GJI の `DirectInput` 行あり/なし・オーバーレイ・CUSTOM の `ctl-loaded` 表・表なし(`Unknown`)・MS-IME の値 0〜3(`Unknown`)・TIP 未同定(`Unknown`))** × `was_down` × 設定値)。InputRelay・`cannot_verify_real_ime_state`・戻り待ちのとき `ime_off_confirmed=false`。`KeyLifecycle` の Down/Up/リピート。活性化中の押下の期待値(決定4)。網羅 `match` による `matches_ime_set_open`/`matches_ime_off` の固定。
2. **Windows CI**(`e2e-ime.yml`): 構成に GJI の CUSTOM 行(InsertSpace 等)を**入れない**。入力先は**計画0(ii)で `cannot_verify_real_ime_state` が偽と確認した Standard の IMM の窓**にする(TsfNative 相当の窓では決定3 により発動しないので、そこで「入らない」を見ても機能の検証にならない)。(a) IME OFF の無変換で入力欄に空白が1つ入る、(b) IME ON では入らず、無変換+文字キーが親指シフト文字になる、(c) **負の対照**: 注入された 0x1D では入らない/composition 中は入らない(Standard の窓で)/ `DirectInput,Henkan,IMEOn` の行がある構成と、GJI の「変換/無変換で IME ON/OFF」オーバーレイの構成で、変換が IME を開き Space にならない/ Alt なりすまし + GJI 側からの IME OFF → 最初の Alt が Alt のまま/ Ctrl+無変換(救済窓)と Shift+無変換が従来どおり/ 長押しで Space が1個だけ。(d) **対照**: TsfNative 相当の窓で「入らない」(決定3 の確認)。
3. **実機**: 報告者の構成(GJI の設定、親指キー、`keys.ime_*`、アプリ)。

## Opus レビュー r1 への対応(指摘 ID ごと)

| ID | 対応 |
| --- | --- |
| B1 | 反映。`[[keymap]]` 案を廃し、エンジン自身の非活性判定に乗る(決定2-1) |
| B2 | 反映。composition があれば発動しない(決定2-6)。キャンセル処理を通らない |
| M1 | 反映。`derive_actuating` を使い、DecidedBy の写像表は作らない(決定2-5) |
| M2 | 反映。実装前の計測(検証計画0)と、素通しの理由のログ(決定2) |
| M3 | 反映。エンジンが Down を見るので親指ラッチと食い違わない。窓は回帰テストで固定(決定3、検証計画1) |
| M4 | 反映(構造的に解消)。`[[keymap]]` の latch を使わない |
| M5 | 反映。`is_bare_thumb` の非注入条件(決定2-3)、負の対照(検証計画2c) |
| M6 | 反映。役割由来の開閉が勝つ(決定2-4)、警告は新規実装(決定4)、panic 検出は検証計画4 |
| M7 | 反映。フックの配送の訂正、無変換だけが open した事実、仮説(a)(b)(c)、n=1 を summary に明記 |
| S1 | 反映。二重配送は起きない(フックは常に握りつぶし、再注入のみ)。再生は再生時点で評価(決定3) |
| S2 | 不要(`find_match` を触らない) |
| S3 | 反映。決定7で範囲外とし、報告者に確認する |
| S4 | 反映。`on` は入れない |
| S5 | 反映。リピートしないことを決定3・5に明記 |
| S6 | 反映。検証計画3 |
| S7 | 反映。検証計画の0〜4 |
| S8 | 該当なし(`[[keymap]]` の項目を増やさない)。新しい設定名は疑問5 |
| S9 | 反映。決定6 |
| A1/A2/A3 | A1 を採用、A2・A3 は不採用(検討した案) |

## Opus レビュー r2 への対応(指摘 ID ごと)

| ID | 対応 |
| --- | --- |
| R2-B1 | 反映(決定3)。TSF ネイティブでは決定的に非対応。鮮度を問わない根拠は今回採らない。検証計画0 をプロファイル別の計測に直した |
| R2-M1 | 反映(決定2-4、決定5)。`thumb_open_role_action` を流用せず独立した純粋関数で 4 源すべて None を見る。IME 側の割り当てを上書きする旨を注記 |
| R2-M2 | 反映(決定2-3)。なりすまし由来を印か scan code で除く。検証計画2 に対照を足した |
| R2-M3 | 反映(決定2-5a)。InputRelay では `ime_off_confirmed=false` |
| R2-M4 | 反映(決定2 冒頭、決定4)。`match_special_keys` の末尾、`!was_down`、Platform 側述語の `None`/`false`、Space の VK はプラットフォームから渡す |
| R2-M5 | 反映(影響範囲と再発ファミリー) |
| R2-S1 | 反映(決定4)。`check_active_transition` の effects を前置する |
| R2-S2 | 反映(決定2-5b)。戻り待ちの間は偽。ADR-245 側にも同じ記述を足す(PR 2 のときに) |
| R2-S3 | 反映(検証計画2c)。Win32 の窓で確かめる |
| R2-S4 | 反映(決定4)。既存の役割経路と同じ期待値にし、実装前に現挙動を調べる |
| R2-S5 | 反映(検証計画 0〜3)。journal への記録は疑問6 |
| R2-S6 | 反映(決定1)。`thumb_key_when_ime_off = "unchanged" | "space"` |
| R2-S7 | 反映(決定2-3、決定5)。`is_bare_thumb` の「物理」はなりすましを含む。panic 検出の計画は削除 |

## Opus レビュー r3 への対応(指摘 ID ごと)

| ID | 対応 |
| --- | --- |
| R3-B1 | 反映(決定2-4)。`KeyOpenEffect = Opens\|NoEffect\|Unknown` を新設し、発動は `NoEffect` のときだけ。MS-IME 本体・TIP 未同定・表が読めないときは `Unknown`。`DirectInput` 行の IMEOn 以外のコマンドも機能ありに数える。当面発動するのは GJI で表が読め、そのキーの行もオーバーレイも無いときだけ |
| R3-M1 | 反映(決定3、決定3b)。述語を `cannot_verify_real_ime_state` に確定し、実効範囲を明記。報告者の確認3点を実装着手のゲートにした |
| R3-M2 | 反映(検証計画0-ii、2)。CI の入力先は Standard の IMM の窓。TsfNative 相当の窓は「入らない」の対照 |
| R3-S1 | 反映(決定2-3)。フックが `impersonated` の印を付ける |
| R3-S2 | 反映(決定4)。実装時に `emit_set_open` と actuation 合流点ファミリーへの該当を確認、`press=None` |
| R3-S3 | 反映(検証計画0・1・2) |
| R3-S4 | 反映(決定3、検証計画0-v)。Chrome は `Imm32Unavailable`、WezTerm/Windows Terminal は `TsfNative`。基準構成の確認に run ID とジョブ名を添えた |

## 未解決の疑問

1. (閉じた)決定3 の述語は `cannot_verify_real_ime_state` に確定。Windows 11 のメモ帳など XAML/RichEdit 系の分類は未確認(検証計画0-ii)。
2. 決定2-1 と IntentStore: IntentStore の上書きが「開」でエンジンが活性なのに `derive_actuating` が「閉」を返す場合は、エンジンが活性なので発動しない(安全側。r2 で確認済み)。ただし Phase 1 と Phase 2 の順序(R2-S1)で、遷移の前後どちらの `ctx` で判定するかを実装時に確かめる。
3. 親指キーが無変換/変換以外の構成での扱い(今回は何もしない)。
4. リピートしない仕様で報告者の期待に合うか。
5. 設定名(`thumb_key_when_ime_off`)と値。
6. エンジンの判断(発動・不発動の理由)を ADR-250 の境界 journal に残すか、debug ログだけにするか(報告 journal で「なぜ Space にならなかったか」を追えるように)。
7. 報告者への確認3点(決定3b)は未回答。回答次第で実装するか保留するかが決まる。
8. MS-IME 本体の無変換/変換の値 0〜3 と「値なし」の既定の効果。確かめるまで `Unknown`(発動しない)のままだが、MS-IME 利用者にも効かせる要望が出たときの測定方法。
