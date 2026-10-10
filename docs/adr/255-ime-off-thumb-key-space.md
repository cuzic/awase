---
id: ADR-255
title: |-
  IME OFF のときだけ無変換/変換を Space にする——エンジンの非活性時の親指キー処理に足す(`[[keymap]]` の条件化は採らない)
summary: |-
  顧客報告「IME OFF のとき GJI の設定が反映されず、変換/無変換を空白入力に割り当てても動かない」を GitHub Windows CI で実機検証した(ブランチ ci/e2e-direct-space、run 38058313464・38059338837、各セル n=1)。GJI の CUSTOM 表は読まれ、直接入力の無変換に IMEOn を割り当てると効く(open 0→1。変換は既に開いた状態で試したので未分離)。直接入力の DirectInput 行に InsertSpace/InsertHalfSpace/InsertFullSpace を割り当てても入力欄に空白は入らなかったが、Precomposition の対照でも空白が入らず、「直接入力では不可」と「観測・キー名・コマンドの対象外」を分けられていない(GJI の仕様で不可とは断定しない)。そこで GJI に頼らず awase 側で、エンジンが非活性(理由 ImeOff)のときの無変換/変換の単独押下を Space にする設定を足す。当初案の `[[keymap]]` への `ime` 条件は、Opus レビュー r1 で、エンジンの活性判定と別の値を見て親指シフトが Space に化ける(B1)・未確定文字列の破棄(B2)・親指ラッチと latch の stale(M3/M4)が指摘されたため採らない。
status: |-
  起草 → Opus レビュー r1 反映済み(2026-10-10)。r2(同じレビュアーへの再確認)待ち。実装は未着手。
related_adr:
  - "ADR-114"
  - "ADR-206"
  - "ADR-141"
  - "ADR-230"
  - "ADR-245"
---

# ADR-255: IME OFF のときだけ無変換/変換を Space にする

## ステータス

起草 → Opus レビュー r1(Blocker 2・Must 7・Should 9・代案 3、[review/255-opus-review-round1.md](review/255-opus-review-round1.md))を反映(2026-10-10)。r2 待ち。実装は未着手。

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

`[general]` に `ime_off_thumb_key = "pass" | "space"` を足す。既定は `"pass"`(今までと同じ)。名前は仮(レビューで見てほしい、疑問5)。無変換/変換の両方に効く(`left_thumb_key`/`right_thumb_key` が無変換/変換のときだけ。それ以外のキーを親指キーにしている構成では何もしない)。

### 決定2: 発動条件(エンジンの内側、すべて AND)

`engine.rs` の `thumb_open_role_action` の隣に、新しい判断(例: `thumb_ime_off_space_action`)を置き、次をすべて満たすときだけ Space を出す。

1. `!compute_active(ctx)` かつ非活性の理由が `ImeOff`(エンジン自身の判定。B1 を構造的に解消する。`UserDisabled`/`NotRomajiInput` は今回含めない、決定7)。
2. `ctx.is_japanese_ime` かつ `adapter.is_enabled()`。
3. `is_bare_thumb(event, ctx.modifiers)`(物理・無修飾・**非注入**。alt-ime-ahk など他ツールが注入する無変換を除く。M5)。
4. `event.ime_relevance.sync_direction.is_none()`、かつ役割由来の開閉(`thumb_open_role_action`/`thumb_forced_open_actions`/学習表)が**無い**こと。役割があれば役割が勝つ(IME を開く手段を奪わない。M6-1)。
5. **IME OFF の確からしさ**(M1): シェルが `InputContext` に `ime_off_confirmed: bool` を足す。真になるのは `ObservationStore::derive_actuating(now) == Some(false)`(Actuating プールだけ。High 単独/Medium 無競合)のときだけ。`HeuristicDefault`・`HwndCache`・`ConvOpenInference`・`desired_open` の既定値・明示意図(`ExplicitIntent`)・打鍵時予測だけが根拠のときは偽(Unknown 扱い。素通し)。写像は `state/` の純粋関数にして Linux で表を固定する。belief は読むだけで書かない。
6. 未確定文字列(composition)が無い(`!ctx.composing`、B2)。composition があるのに「IME OFF」とみなすのは矛盾した証拠なので発動しない。

**不確かなときは今までどおり素通し**(`pass` と同じ)。素通しの理由は debug ログに出す(M2、報告者が「効かない」と再報告したときの切り分け用)。

### 決定3: 出力と Down/Up

- Down で `VK_SPACE` を 1 回タップ(Down+Up)する。KeyUp は `KeyLifecycle` の Consume に乗せて回収する(`[[keymap]]` の latch は使わない。M4 の stale latch を通らない)。
- **リピートしない**: 無変換を押し続けても Space は 1 個(S5)。本物の Space キーや GJI の InsertSpace(リピートする)とは違う。報告者の期待と合うか確認し、設定画面に書く。
- エンジンは Down を見ているので、フック側の親指ラッチ(`HOOK_STATE.left_thumb_down_scan`)と食い違わない(M3 を解消)。ただし「押している間に IME が ON になった」場合の文字キーの扱いは、回帰テストで固定する(検証計画 1)。
- 再生(`OUTPUT_GATE`/`INPUT_DEFER` で退避された Down)は、再生時点の状態で評価する(S1)。

### 決定4: 競合と警告

- `ime_off_thumb_key = "space"` は、`keys.ime_on/ime_off/ime_toggle`・`muhenkan_solo_tap_dedicated_fn_key`・`muhenkan_solo_tap_ime_action`(ADR-206)・役割由来の開閉と重なるキーでは、決定2-4 により**そのキーでは発動しない**。重なりは設定読み込み時に警告する(「既存の衝突警告に載せる」ではなく新規。`warn_if_vk_conflicts` は dedicated fn key の2箇所だけで、`keys.ime_*` との衝突警告は実在しない。M6-2)。
- panic 検出(`record_ime_keydown`)への影響: 無変換/変換の打鍵が数える対象になる点は今と同じ。Space に変えても計数は増減しない。重なる構成での速い交互押下は実機で確認する(M6-3、検証計画 4)。

### 決定5: 設定画面

親指キーの設定の近くに「IME OFF のとき無変換/変換を Space にする」のチェックを置く。注記に、(1) リピートしない、(2) 「半角英数」(IME は開いたまま英数モード)の状態では動かない、(3) IME の状態が確かでないときは何もしない、を書く。

### 決定6: リリース

- オプトイン(既定 `"pass"`)。Scancode Map(ADR-230)や、かな/無変換/変換の挙動変更とは同時にリリースしない。回帰テスト(検証計画 1・2)が通り、develop で実機確認が済むまで次の v2 リリースに入れない。v1 への backport はしない(新機能)。

### 決定7: 範囲外

- `NotRomajiInput`(半角英数)での Space 化(S3)。報告者の「IME OFF」がタスクバーの「A」(半角英数)なのか、直接入力なのかを先に確認する。半角英数なら別の条件が要る。
- `[[keymap]]` の条件化、`ime = "on"`(S4)。必要になったら別 ADR。
- GJI の CUSTOM 表の生成(A3)。

## 検証計画

0. **実装前の計測(M2)**: CI スパイクで、Notepad(Win32)・Chrome(TsfNative)・Windows Terminal・報告者のアプリで、IME OFF のまま無変換を押した瞬間の `resolve_open_at` の `DecidedBy` と `derive_actuating` の結果を数える。「安全だが効かない」機能にならないかを先に知る。あわせて正の対照(スペースキー 0x20 の注入で入力欄の `tail` に空白が出るか)、直接入力行の変換(0x1C)の IMEOn、設定ダイアログで直接入力行に InsertSpace を選べるかを確認する(仮説 a〜c の切り分け)。
1. **Linux 単体**(`cargo test --lib`、判断は `state/` の純粋関数・エンジン側に置く。`runtime/` の `#[cfg(test)]` は Linux に存在しない): 発動条件の表(非活性理由 × `ime_off_confirmed` × composing × 注入 × 修飾 × 役割あり/なし × 設定値)。Down/Up/リピートの `KeyLifecycle`。「Down 後に IME が ON になっても Up が回収され、その間の文字キーが親指シフト扱いにならない/なる」の期待を固定する。
2. **Windows CI**(`e2e-ime.yml`): 構成に GJI の CUSTOM 行(InsertSpace 等)を**入れない**(GJI 側が動くと効果が区別できない)。(a) IME OFF の無変換で入力欄に空白が1つ入る、(b) IME ON では入らず、無変換+文字キーが親指シフト文字になる、(c) **負の対照**: 注入された 0x1D では入らない、composition 中は入らない、`keys.ime_on = 変換` 構成で変換が IME を開く。
3. **実機**: 報告者の構成(GJI の設定、親指キー、`keys.ime_*`、アプリ)。Alt なりすまし(`left_alt_impersonates_thumb_key`)の構成で、IME OFF のとき Alt が Space にならないこと(S6)。
4. panic 検出の確認(M6-3): 重なる構成で速い交互押下。

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

## 未解決の疑問

1. `ime_off_confirmed` の鮮度: `derive_actuating` は `OBSERVATION_FRESH_WINDOW_MS` 内の観測しか使わない。IME OFF のまま放置した後や TsfNative で `Unknown` になりやすく、効かない場面が多くないか(検証計画0で測る)。
2. 決定2-1 の `InactiveReason::ImeOff` が、`ctx.ime_on` の由来(IntentStore の上書きを含む `effective_open()`)と整合する前提で、IntentStore の上書きが「開」でエンジンが活性なのに `derive_actuating` が「閉」を返す場合、エンジンが活性なので発動しない(安全側)ことの確認。
3. 親指キーが無変換/変換以外の構成での扱い(今回は何もしない)。
4. リピートしない仕様で報告者の期待に合うか。
5. 設定名(`ime_off_thumb_key`)と値(`"pass"|"space"`)。`[general]` の既存の `muhenkan_solo_tap_*` 系との整合。
6. 報告者の「IME OFF」が直接入力か半角英数か(決定7、確認待ち)。
