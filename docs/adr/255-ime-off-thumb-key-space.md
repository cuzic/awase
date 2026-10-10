---
id: ADR-255
title: |-
  IME OFF のときだけ無変換/変換を Space にする——`[[keymap]]` に `ime = "off"` を足し、エンジンの判断の後に照合する
summary: |-
  顧客報告「IME OFF のとき GJI の設定が反映されず、変換/無変換を空白入力に割り当てても動かない」を GitHub Windows CI で実機検証した(ブランチ ci/e2e-direct-space、run 38058313464・38059338837、各セル n=1)。GJI の CUSTOM 表は読まれ、直接入力の無変換に IMEOn を割り当てると効く(open 0→1。変換は既に開いた状態で試したので未分離)。直接入力の DirectInput 行に InsertSpace/InsertHalfSpace/InsertFullSpace を割り当てても入力欄に空白は入らなかったが、Precomposition の対照でも空白が入らず、「直接入力では不可」と「観測・キー名・コマンドの対象外」を分けられていない(GJI の仕様で不可とは断定しない)。そこで GJI に頼らず awase 側で、エンジンが非活性(理由 ImeOff)のときの無変換/変換の単独押下を Space にする設定を足す。Win32/IMM 系のウィンドウが対象で、IME OFF の判定は、エンジンが NICOLA の活性に使っている awase の ime belief そのもの(非活性・理由 ImeOff)にする。観測の鮮度や品質では絞らないので、Chrome・VS Code・Windows Terminal など IME の状態を読めないアプリでも動く(所有者の提案、r6 で改訂。r2〜r5 では `derive_actuating` による確認を AND にして、そのようなアプリを非対応にしていた)。belief が外れているときは NICOLA も効いていないので、実害は限定的と見る。当初案の `[[keymap]]` への `ime` 条件は、Opus レビュー r1 で、エンジンの活性判定と別の値を見て親指シフトが Space に化ける(B1)・未確定文字列の破棄(B2)・親指ラッチと latch の stale(M3/M4)が指摘されたため採らない。
  (r7: 汎用性のため、エンジン内の専用設定ではなく `[[keymap]]` に `ime = "off"` を足し、そのルールだけをエンジンの素通しの後に照合する形に変更した。)
status: |-
  改訂 r7(2026-10-10): 所有者の判断で、専用設定(エンジン内)から `[[keymap]]` の `ime = "off"`(エンジンの判断の後に照合)へ変更。Opus r7 で新規 Must 3 件(R7-M1〜M3)を反映済み。r1〜r6 は専用設定案で収束済み。**実装は 2026-10-10 に着手した(所有者の判断。決定3b のゲートは外した。下記)**。
related_adr:
  - "ADR-114"
  - "ADR-206"
  - "ADR-141"
  - "ADR-230"
  - "ADR-245"
---

# ADR-255: IME OFF のときだけ無変換/変換を Space にする

## ステータス

改訂 r7(2026-10-10): 所有者の判断で、専用設定(案 A1、エンジン内)から `[[keymap]]` の `ime = "off"`(案 K、エンジンの判断の後に照合)へ変更した。Opus レビュー r1〜r6 は案 A1 で収束済み(r6 は所有者の提案による belief だけの判定への変更)。案 K の差分は r7 のレビューで確認する。**実装は 2026-10-10 に着手した(所有者の判断。決定3b のゲートは外した。下記)**。

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
- **追加の CI 計測(run 38063068778、ブランチ `ci/e2e-direct-space` のコミット a21be6a8、各構成 n=1)で、仮説 b・c は否定された**:
  - (c) 否定: IME OFF のまま 0x20 を注入すると `tail` に空白が出る(構成 `sc-direct-space-m-h1-space-key`)。入力欄の観測は空白を捉える。
  - (b) 否定: 直接入力の変換に `IMEOn` を割り当てると、IME OFF から変換で open が 0→1 になる(`m-h2-henkan-imeon`)。無変換は前回(`ctl-loaded`)で確認済み。`Henkan`/`Muhenkan` というキー名は GJI に通じる。
  - 無変換/変換以外の F13(0x7C)に `InsertSpace` を割り当てても、直接入力(`m-h3-f13-insertspace-direct`)でも IME ON・入力中でない状態(`m-h3-f13-insertspace-precomp`)でも、`tail`/`comp` に空白は出ない。
  - したがって、**GJI の `InsertSpace` は、キー名や観測の問題ではなく、Space キー以外では空白を出さない**可能性が高い(仮説 a に近い。Mozc の実装は確認していないので確定ではない。設定ダイアログで直接入力行に InsertSpace を選べるか〈検証計画0-iv〉は未確認)。**awase 側で Space を送る本機能が必要**という結論は、GJI の設定では実現できない可能性が高いという裏付けを得た。
  - **belief の追随**(`m-belief-*`、標準 EDIT と RichEdit 5.0〈`--round2`〉、ATOK と MS-IME のプリセット、無変換と変換の固定列、各2回、計 192 ステップ): エンジンが実 IME に追随しなかったステップは **0 件**(全 PASS)。ただし**どの窓も `ImmCross` プロファイルで、TsfNative 側(Chrome・Windows Terminal 等)は未計測**。測定は押下後(+1500ms)の追随で、押下直前の belief との突き合わせではない。
- 以前の記述(r2〜r6 の時点、参考): Precomposition の対照でも空白が出ないので、次の仮説を分けられていない: (a) GJI の直接入力状態では InsertSpace 系は割り当ての対象外、(b) 無変換/変換というキーでは InsertSpace が文字を出さない、(c) 入力欄の観測が空白を捉えていない。上の追加計測で (b)(c) は否定された。
- フックの配送(訂正): フックは Accepted のキーを常に握りつぶし(`hook.rs` の `LRESULT(1)`)、エンジンの判断後にメインスレッドが**再注入**する。「awase は IME OFF の無変換/変換を素通しする」は正確には「再注入で GJI に届ける」。CI で awase あり/なしの結果が同じなので結論は変わらない。`physical_disposition.rs` の無変換/変換の Allow は「再注入する」の意味。

### 既存の機構

- エンジンは非活性のとき bare の親指キーを `src/engine/engine.rs::thumb_open_role_action`(役割由来の開閉)で扱う。エンジンの活性は `compute_state`(`ctx.ime_on`)が決める。非活性(Phase 2)は `Decision::pass_through()` を返し、`KeyLifecycle` には何も記録しない。
- `[[keymap]]`(ADR-114)は `app`・`from`・`to` の3項目。`runtime/message_handlers.rs::deliver_key_event` が、`keymap_latch` の確認(ステップ1)→ `NonText` の早期 return → `consume_keymap_match`(ステップ2、エンジンの**前**)→ `[[post_bypass]]` → エンジン(`process_key_event` → `kp_run_inner`)の順に処理する。`kp_run_inner` は `engine.on_input` の後に journal・`kp_stage_post_decision`・`kp_stage_execute` を走らせる。`Decision::force_consume()` は effects を保ったまま素通しを消費に格上げする既存 API。

## 検討した案

- **案 K(r7 で採用)**: `[[keymap]]`(ADR-114)に `ime = "off"` の条件を足し、そのルールだけを**エンジンの判断の後**(エンジンが素通しにしたときだけ)に照合する。所有者が、汎用性(無変換/変換以外のキーにも使える・アプリ別に絞れる・既存の再割り当てタブが使える)を理由に選んだ。
- **案 A1(r1〜r6 の採用案。r7 で不採用に変更)**: エンジンの非活性時の親指キー処理に、専用設定(`thumb_key_when_ime_off`)で Space 出力を足す。設定項目が1つ増える代わりに、エンジンが Down を見るので、親指ラッチ・古い latch・IME 側の機能の判定の配線が少ない。**案 K の実装で想定外の配線が膨らんだ場合の代替**として残す(決定8)。
- **案 A2(不採用)**: `[[keymap]]` に `ime = "off"` を足すが、照合を**エンジンの前**(ADR-114 の既存位置)に置く(当初案)。Opus r1 の指摘: (B1) エンジンの活性判定と別の値を見て、エンジンは活性なのに先に消費して Space にしうる。(B2) 誤判定で `consume_keymap_match` が未確定文字列を破棄する。(M3) フックの親指ラッチ。(M4) 古い latch。(M5) 注入された無変換。案 K は照合をエンジンの**後**にして B1 を構造的に避け、B2・M3〜M5 には個別に手当する(決定2・4)。
- **案 A3(不採用)**: GJI の CUSTOM 表を awase が生成する(ADR-231)。CI で InsertSpace が効いていないので、現時点の証拠では採れない。MS-IME 利用者にも効かない。

## 決定

### 決定1: `[[keymap]]` に `ime` を足す(r7)

```toml
[[keymap]]
from = "VK_NONCONVERT"
to = ["VK_SPACE"]
ime = "off"
```

- `ime`: `"off"` のみ。省略時は従来どおり(IME の状態を問わず、エンジンの**前**に照合)。**IME OFF のときだけ指定でき、ON のときの用途は作らない**(ON ではエンジンが親指として使う。所有者の提案、r1 S4 で `on` を落とした判断と同じ)。`"off"` 以外の値は警告して skip する。
- `ime = "off"` の付いたルール(以下「遅いルール」)は、従来の照合(ADR-114 決定2 のステップ2、エンジンの前)では**照合しない**。照合はエンジンの判断の後だけ(決定2)。
- `app` による絞り込みは従来どおり使える(`filter_active`)。
- **遅いルールの `from` は、無修飾の無変換/変換に限る**(R7-M3)。それ以外(他のキー、修飾付き〈`Ctrl+VK_NONCONVERT` は Ctrl+Space を OS に届けうる〉)は、コンパイル時に警告して skip する。理由: 発動条件の「IME の機能を持たない」(条件6)の4源と `KeyDirectInputEffect` は、無変換/変換にしか定義されておらず、他のキー(例: MS-IME/MOBILE プリセットの `DirectInput\tF13\tIMEOn`)では、評価できず黙って効かないか、IME の機能を奪う。**将来、他のキーへ広げるには、条件6 を全キーに定義し直す別 ADR が要る**。今回の消費者は無変換/変換だけ。

### 決定2: 遅いルールはエンジンの判断の後に照合する(r7)

**照合の位置**: `runtime/key_pipeline.rs::kp_run_inner` の `self.engine.on_input(event, &ctx)` の**直後**、`Decision` が `PassThrough` または `PassThroughWith`(素通し)のときだけ。journal の記録・`kp_stage_post_decision`・`kp_stage_execute` より前。

- 当たったら `decision.force_consume()`(`Decision` の既存 API。effects を保ったまま `Consume` に格上げする)で消費に変え、**`decision.push_effect(Effect::Input(InputEffect::SendKeys(...)))` で `to` を effects の末尾に積み**、**エンジンに `engine.record_shell_consumed(&event)`(新設。後述の決定4)を呼んで消費した Down を登録する**(KeyUp と自動リピートはエンジン自身の既存機構が回収する。`keymap_latch` には積まない)。**`send_keymap_target` でその場で SendInput しない**(R7-M1): 遅いルールが当たる打鍵は、エンジンが素通しにした打鍵で、直前の文字も素通し(`ReinjectKey`)として executor のキューに並んでいることがある。その場で送ると Space が先行の文字や遷移の flush を追い越し、「foo bar」と速く打つと「fo obar」になりうる(空白を打つ用途では文字順の入れ替わりがそのまま実害)。effects に積めば FIFO で遷移の effects・先行のキューの後に実行され、`DecisionKind` にも残る。`to` の VK はシェルの `crate::vk::VK_SPACE`(エンジンは生の VK 定数を持たない。ADR-019)。出力層(`output/vk_send.rs`)が `SendKeys` の Space を IME OFF でどう扱うか(warm/cold 判断。`state/warm_send_plan.rs`)は実装時に確かめ、エンジンの Space 親指フォールバック(`ThumbRawVkEmission`)が生の VK を effects で送る前例と同じ effect の型を使う。
- **この位置にする理由**:
  - (a) 判定に使う `ctx`・エンジンの状態がエンジン自身の判断と同じ(B1)。エンジンが活性(親指として使う)のときは、そもそも素通しにならないので、遅いルールは当たらない。
  - (b) エンジンが Phase 2 で出す遷移の effects(`check_active_transition` の `EngineStateChanged` など)が `PassThroughWith` に載っているので、`force_consume` で保たれる(r2 S1 の前置が不要になる)。`SetOpen` は元から含まれない(ADR-213 P2b)。
  - (c) 後続の段(`kp_stage_post_decision`・`kp_stage_execute`・物理配送の決定)は消費に変わった `Decision` を見るので、素通しした無変換として「モードキーの通過」の追跡(ADR-191)を始めない。エンジンの**前**に走る段(`kp_stage_shadow_ime_toggle`・`settle_fkey_role_latch`・`enrich_thumb_key_role` など)が、IME の機能が無い無変換に書き込まないことは、r7 のレビューで確認済み。`kp_stage_post_decision` と `kp_stage_mode_key_follow` は `is_consumed` で止まり、モードキー追跡の誤装填も refresh も起きない。**前提は、格上げを journal の記録と `kp_stage_post_decision` より前に行うこと**。
- エンジンの `KeyLifecycle`(`src/engine/key_lifecycle.rs`)は、素通しの KeyDown を記録しない(Phase 2 の非活性は `Decision::pass_through()` を返すだけで `on_key_down_consumed` を呼ばない)。遅いルールが Down を消費したときは、この記録が無いので、シェルが `record_shell_consumed` で登録する(決定4)。**実装時に単体テストで固定する**(`src/engine/tests.rs`)。
- `NonText` のフォーカスでは `process_key_event` に到達しないので、従来どおり効かない(ADR-114 の既知の限界を継承)。
- `kp_run_inner` はドレイン・再生(`INPUT_DEFER`・TsfGate の保留)からも呼ばれる。遅いルールの評価は、再生時点の状態で行う(r1 S1)。

**発動条件(すべて AND)**:

1. `engine` が `compute_state(ctx)` で `Inactive(ImeOff)`(非活性の理由が IME OFF)。エンジンに読み取り関数(例: `ime_off_inactive(&ctx) -> bool`)を足し、判断を再実装しない。`UserDisabled`/`NotRomajiInput` は含めない(決定7)。
2. `Decision` が素通し(`PassThrough`/`PassThroughWith`)。エンジンが消費したなら当たらない。
3. (条件1 に含まれる: `compute_state` は `UserDisabled` → `NotJapaneseIme` → `ImeOff` の順に判定するので、`Inactive(ImeOff)` なら日本語 IME でエンジンも有効。R7-S4)
4. 遅いルールの `from` に、`vk` と修飾(ctrl/shift/alt/win)が完全一致する(既存の `find_match` と同じ)。
5. **非注入**かつ**Alt なりすまし由来でない**: `!event.injected`(alt-ime-ahk など他ツールが注入する無変換を除く。r1 M5)。Alt なりすまし(`left/right_alt_impersonates_thumb_key`)は、フックが書き換えたときに `RawKeyEvent` へ `impersonated: bool` の印を付けて除く(R2-M2。キャッシュ `cached_engine_enabled` は `EngineStateChanged` でしか更新されず、GJI 側で IME が閉じた直後の最初の打鍵が Alt だと、Alt が Space になる。scan code での判定は右 Alt→変換の取りこぼしやリマッパで scan が変わる問題があるので採らない。R3-S1)。印は境界 journal(ADR-250)にも乗る。
6. **このキーが IME の機能を持たない**(R2-M1、R3-B1)。次の2つを**両方**満たす。
   - (i) 従来の4源がすべて None: `bare_ime_action(vk)`(`keys.ime_*`)、`thumb_role_open_actions()` の該当側(単独タップ設定が Passthrough かどうかを問わない)、`muhenkan_solo_tap_dedicated_fn_key`(専用 Fn キー)、`event.ime_relevance.sync_direction`。`thumb_open_role_action` は流用しない(専用 Fn キー設定済み、または役割があっても単独タップ設定が Passthrough でないとき `None` を返す。この場合エンジン非活性では生キーを通して IME 自身に開かせているので、`None` を「役割なし」と読むと IME を開く手段を奪う)。
   - (ii) **このキーの直接入力状態での効果の三値が `NoFunction`**: `KeyDirectInputEffect = HasFunction | NoFunction | Unknown`。**発動は `NoFunction` のときだけ**(名前は「直接入力でこのキーに IME の機能があるか」。`HasFunction` は IMEOn だけでなく `Reconvert`・`InsertSpace` 等の「開かないが機能がある」行も含む。R5-S3)。awase の `KeyRole`(`awase-gji-config/src/role.rs`)は `ImeToggle` の1つだけで、**開くだけの割り当て**(CI の `ctl-loaded` 表の `DirectInput,Henkan,IMEOn` 単独行、GJI の「変換/無変換で IME ON/OFF」オーバーレイ、MS-IME の「変換 = IME-オン」)では None を返すため、(i) だけでは IME を開く手段を奪う。三値の写像は `state/` の純粋関数にして Linux で表を固定する:
     - GJI(表が読める): 照合は**無修飾のキー名の完全一致**で行う(`Shift Henkan` のような修飾付きの行は、無修飾の変換の行と数えない。R4-S3。`awase-gji-config/src/role.rs` の `KeyStates::of` は「無修飾の行だけを表の順に読む(後勝ち)」で既にこの区別をしており、`Effect::of` が `DirectInput` の行を `Open`(IMEOn・絶対モード指定)か `Other`(Reconvert・InsertSpace 等)に分ける。「行があれば機能あり」は `states[DirectInput].is_some()` で書ける。テストで固定する。R5-S2)。プリセットまたは CUSTOM 表の `DirectInput` 行にそのキーの行があれば `HasFunction`(`IMEOn` 以外のコマンド〈`Reconvert`・`InputModeHiragana`・**`InsertSpace` 系**等〉でも**機能ありに数える**。Space で奪わない)。既知のオーバーレイ(`SESSION_KEYMAP_OVERLAY_HENKAN_MUHENKAN_TO_IME_ON_OFF`)があれば `HasFunction`。**未知のオーバーレイ・未知の `session_keymap`(`source()` が `Source::Unknown` を返す場合)は `Unknown`**(R4-S2)。行が無ければ `NoFunction`。表が読めない・`table_ime_kind` が None も `Unknown`。
     - **プリセットの `DirectInput` 行を本番の表として持つ**(R4-M1): いまの `awase-gji-config/src/role.rs` はプリセットについて「トグルを持つキーの名前の一覧」(`Preset::toggle_vk_names`)しか持たず、Mozc の TSV 本体は同梱しない(ADR-199 決定4)。各プリセットの `DirectInput` 行はテストの定数(`MS_IME_TSV`/`ATOK_TSV`/`KOTOERI_TSV` と `*_DIRECT_INPUT`)にしか無い。本 ADR の実装は、4 プリセット(MS-IME・ATOK・KOTOERI・MOBILE)の変換/無変換の `DirectInput` 行を本番の定数として足す。出典は Mozc `b4bbc42f`(テストの定数と同じ)。Mozc の更新で古くなる性質があるので、`passive_open_vk_names_outside_table` と同じく手で取り直し、既存のテスト(`passive_open_vk_names_outside_table_match_mozc_direct_input_rows` 相当)で突き合わせる。前例は ADR-211 の `passive_open_vk_names_outside_table`(プリセットごとの定数を `source()` で選ぶ形)。この表で判定した結果(実装前にテストの定数から読んだもの):

       | GJI のキー設定 | 無変換 | 変換 |
       | --- | --- | --- |
       | MS-IME(**既定**。未設定・NONE・空の CUSTOM もこれ) | `NoFunction` → 発動 | `DirectInput\tHenkan\tReconvert` → 機能あり、**発動しない** |
       | ATOK | `IMEOn` → 発動しない | `IMEOn` → 発動しない |
       | KOTOERI | `NoFunction` → 発動 | `NoFunction` → 発動 |
       | MOBILE | MS-IME と同じ | MS-IME と同じ |
       | CUSTOM | 表しだい | 表しだい |

     - MS-IME 本体: 無変換/変換の値(0/1/2/3)の意味と「値なし」の既定の効果を確かめていないので、**今回は `Unknown`(発動しない)**。
     - TIP 未同定(`!ime_identified`。BUG-179): `Unknown`。
   - つまり**当面発動するのは、GJI で、表が読めて、そのキーの `DirectInput` 行もオーバーレイも無いとき**だけ。**既定の MS-IME プリセットでは無変換しか Space にならず(変換は `Reconvert`)、ATOK プリセットでは両方ならない**。**報告者自身が GJI の CUSTOM 表に足した `DirectInput` の無変換/変換の行(InsertSpace 等)があると、機能ありと数えるので両キーとも発動しない**(R4-M2)。案内は決定3b。
   - **`KeyDirectInputEffect` の運び方**(R4-S4): 判定に要る GJI の表は、シェル側の `key_effect_keymap.get_gji(now_ms)`(`runtime/mod.rs::derive_key_shadow_action`)にある。遅いルールの照合は `kp_run_inner`(シェル)で行うので、エンジンに運ぶ必要はない。`enrich_thumb_key_role` と同じ場所で、押した側だけ評価する(古い値を残さない)。
7. **belief の品質では絞らない**(決定3): 除外するのは (a) InputRelay(ローカルの belief はリモートの IME 状態を表さない。ADR-206 も InputRelay では役割を付けない。R2-M3)、(b) ADR-245 の「戻り待ち」が立っている間(R2-S2)。
8. 未確定文字列(composition)が無い(`!ctx.composing`、r1 B2)。**遅いルールでは `consume_keymap_match` の `cancel_composition` を呼ばない**(composition があれば当たらない)。**TSF のアプリで効くかは未検証**(`ime_composition_active_now()` は WinEvent の `EVENT_OBJECT_IME_SHOW`/`HIDE` が書くグローバルなフラグで、別アプリの composition 窓でも立つ。MS-IME での信頼性は `ime_decision_view.rs` が未検証とする。立ちっぱなしは安全側〈発動しない〉、立たないと変換の害になる)。検証計画2(c) で TsfNative 相当の窓と Chrome を含めて確かめる。

9. **`!event.was_down`**(R7-M2): リピートの Down には当たらない。最初の Down が素通しなら、そのキーの Up まで素通しのまま。失敗シナリオ: 最初の Down は条件1 や 8 で当たらず素通しで OS に届く → 押したまま状態が変わり、次のリピート Down(`was_down=true`)で当たる → Space を送り latch に積む → 物理 Up は latch が飲む → OS には無変換の Down だけが届き、Up が届かない(BUG-131/132 と同じ Down/Up 非対称)。回帰テスト(物理キー押下ラッチ・ファミリー)に入れる。

**不確かなときは今までどおり素通し**。素通しの理由は debug ログに出す(r1 M2)。発動したときは、なぜ消費したか(遅いルールが当たった)を区別できる印を、境界 journal(ADR-250)か debug ログに残す(R7-S2。そうしないと journal 上は「非活性のエンジンが無変換を Consume した」と読め、エンジン側の不具合と取り違える)。

### 決定3: IME OFF の判定はエンジン自身の belief だけにする(r6。r2〜r5 の決定3 を置き換える。r7 でも維持)

**所有者の提案**(2026-10-10): 親指キーの Space 化は IME OFF のときだけ指定でき、IME OFF の判定は awase の ime belief を使う。

経緯: r2〜r5 では、`derive_actuating(now) == Some(false)`(Actuating プールの観測が3秒以内)を AND にし、述語 `cannot_verify_real_ime_state` が真のアプリ(Chrome・Edge・VS Code・Windows Terminal・UWP・コンソール・RDP 等)では発動させなかった。理由は、これらのアプリでは `ObserverPoll` が書かれず、IME OFF のまま打っていると `derive_actuating` が 3 秒で `None` になり、「OFF にした直後の3秒だけ Space になる」切り分けの難しい挙動になるため(Opus r2 R2-B1)。

見直し: 次の理由で、belief だけにする方が良い。
- **エンジンの判断と一貫する**。エンジンは同じ belief で NICOLA の活性を決めている。「エンジンが IME OFF とみなしている間は親指キーが Space、ON とみなしている間は親指キー」は、利用者が見ている NICOLA の効き方と一致する。別の根拠(`derive_actuating`)を足すと、「NICOLA は効いていない(エンジン非活性)のに無変換が Space にならない」という食い違いを新たに作る。
- **belief が外れているときは、NICOLA もすでに効いていない**。IME が実際は ON なのに belief が OFF なら、エンジンは非活性で、利用者はすでにローマ字が出ている。そのとき無変換を押して Space が入っても、状況が新たに悪くなる範囲は限られる(下の「belief が外れたときの実害」)。
- 「確かめられるアプリでだけ動く」は、IME を実際に読めないアプリ(Chrome 等)を、報告者が最初に試す可能性が高いのに外してしまう。
- belief には awase 自身の actuation・打鍵時予測・明示意図・観測が入っており、エンジンが NICOLA の活性に使う値として既に審査されている(ADR-087・ADR-188・ADR-191)。

決定:
- 決定2 の条件1(エンジンの `compute_state` が `Inactive(ImeOff)`)だけを IME OFF の根拠にする。`ime_off_confirmed`(`derive_actuating`)は導入しない。述語 `cannot_verify_real_ime_state` で除外しない。
- InputRelay と ADR-245 の戻り待ちは除外する(決定2 の条件5)。
- **belief が外れたときの実害**(実装前に検証計画0 で測る):
  - 実際は IME ON、belief は OFF(エンジン非活性)で無変換を押す: 親指として使うつもりなら Space が入る(従来は無変換が IME に素通しされ、何も入らなかった)。IME が実際に ON のとき、この Space は IME によって変換(Composition)や全角スペース(Precomposition)として解釈されうる。変換中(未確定文字列あり)は条件6 で発動しないが、条件7 の信頼性は未検証(上記)。
  - 新しいウィンドウの直後(観測が無く既定値の「閉」が belief になっている間)に無変換を押す: 同上。この間 NICOLA も効いていない。
  - 実際は IME OFF、belief は ON(エンジン活性)で無変換を押す: Space にならず、従来どおり親指キー(NICOLA が効いている状態と一貫)。
- **記録**(切り分けのため): 発動・不発動の理由と、そのときの belief の根拠(`resolve_open_at` の `DecidedBy`)を debug ログに出し、境界 journal(ADR-250)に残せる形にする(疑問6)。「belief が外れたときの実害」の頻度を、実機・報告 journal で後から数えられるようにする。
- **観測品質で絞る案は捨てない**: 検証計画0 の計測で「belief が OFF なのに実 IME は ON だった」割合がアプリ種別ごとに無視できない大きさなら、別 ADR で「既定値だけが根拠のときは発動しない」(`DesiredFallback`・`HeuristicDefault` のみ)などの最小の絞りを足す(疑問1)。

### 決定3b: 実装着手の条件(ゲート)(R3-M1、R4-M2・M3。r6 で範囲を改訂)

この機能が報告に効くかは、報告者の GJI のキー設定に強く依存する(決定2 の条件6・その表)ので、**報告者に次の4点を確認するまで実装に着手しない**。アプリの種類は、決定3 の見直しで「動くかどうか」の条件ではなくなったが、belief が外れたときの実害を見積もる材料として、引き続き集める。
1. **どのアプリで、IME OFF のとき無変換/変換を空白にしたいか**、および**直接入力か半角英数か**: 「タスクバーの A」は直接入力でも半角英数でも「A」と出るので答えられない(R4-M3)。代わりに、**問題の起きるアプリで、IME OFF の状態のまま無変換を押した直後に、トレイの「不具合を報告」(ADR-095)を送ってもらう**。報告にはフォーカス窓のクラス・プロファイル・belief(open・input mode)が入るので、(a) 押した時点でエンジンが非活性(ImeOff)だったか、(b) 直接入力か半角英数か(半角英数は決定7で範囲外)を客観的に読める。手で確かめてもらうなら、GJI の言語バーの入力モード表示で「直接入力」か「半角英数」かを見てもらう。
2. **押し続けて Space が連続して出る必要があるか**(決定4 のとおりリピートしない)。
3. **GJI のキー設定**(プリセット名、または CUSTOM)。決定2 の条件6 の表のとおり、既定の MS-IME プリセットでは変換が発動せず(`Reconvert`)、ATOK では両方発動しない。
4. **CUSTOM の場合、直接入力の無変換/変換の行**(InsertSpace 等)があるか。**報告者の構成では、GJI の CUSTOM 表に足した InsertSpace 等の行が残っている可能性が高く、その行があると両キーとも発動しない**(機能ありと数える。R4-M2)。案内: 「GJI のキー設定の CUSTOM から、直接入力の無変換/変換の行を消す」(推奨。InsertSpace 系を `NoFunction` に数える案は、仮説 a〜c が未分離で、効く環境で Space が二重になる危険が残るので、設定ダイアログで直接入力行に InsertSpace を選べるかの確認〈検証計画0-iv〉が済むまで採らない)。この案内は設定画面の注記にも書く。

**ゲートが止めるのは実装(コード変更)だけ**。検証計画0 の計測・分類の確認・仮説 a〜c の切り分けの CI スパイクは、報告者の回答を待たずに進めてよい(R5-S6。(iv) の結果は報告者への返答の材料になる)。**〈2026-10-10 追記〉所有者が、報告者の回答を待たずに実装すると判断した**(ゲートは外れた。段階1: `[[keymap]]` の `ime` と遅いルールの分離 = PR #596、段階2: 判断の核と `Engine::record_shell_consumed` = PR #597、段階3: `kp_run_inner` への配線・設定画面・CI)。以下は、ゲートがあった間の記述として残す。**報告者が回答しない間は実装しない**。保留のまま、別の報告で需要が出たら決定3b から再開する(後のセッションが「起草済み・未実装」を実装待ちと誤読しないため。R5-S5)。

結果に応じて:
- (a) 報告者のキー設定が発動範囲に入る(または入れてもらえる)なら実装する。アプリの種類は問わない。ルールは報告者が `[[keymap]]` に自分で書く(または設定画面の再割り当てタブから作る)。
- (b) キー設定が範囲外(ATOK プリセット・`Reconvert` を残したい等)なら、報告者には案内(設定の変更、または GJI の直接入力状態の制約の切り分け結果)を返し、実装は保留する。再変換を上書きする選択肢(疑問9)は需要が出てから扱う。

### 決定4: 出力と Down/Up(r7)

- Down で `to` を、`Decision` の effects の末尾に `SendKeys` として積んで送る(決定2。`send_keymap_target` でその場では送らない。Space は `to = ["VK_SPACE"]`)。
- KeyUp とリピートは、エンジンの `KeyLifecycle`・`phase1_held` が回収する(次項)。**リピートしない**: 無変換を押し続けても Space は 1 個(R5・S5)。本物の Space キーや GJI の InsertSpace(リピートする)とは違う。報告者の期待と合うか確認する(決定3b-2)。
- **消費した Down をエンジンの `KeyLifecycle` に登録する**(所有者の提案、r7 追加評価で採用。r1 M4 の stale latch への手当): `Engine::record_shell_consumed(&event)` を新設する。中身は `lifecycle.on_key_down_consumed(&event)` と、bare の親指なら `phase1_held = Some(vk)`(遅いルールは無変換/変換に限るので常に満たす)だけで、**エンジンの判断(活性/非活性・FSM の状態)は変えない**(「消費した Down の登録」だけの口。ADR-112 の「`Engine::on_input` の唯一の出口」の不変条件とは衝突しない)。呼ぶのは、遅いルールが当たって `force_consume` した後、`kp_stage_execute` より前、同じ打鍵の中。
  - **KeyUp**: `on_input` が `take_key_up_duty` で `UpDuty::Consume` を取り、非活性のままなら Phase 2 の `release_only` を通って何も送らずに Consume になる(`output_history` に無変換のエントリは無い)。Space は effects で Down+Up を完結しているので、Up の義務は無い。
  - **自動リピート**: `on_input_body` 冒頭の `phase1_held` のガード(`engine.rs`、`is_key_down && event.was_down && phase1_held == Some(vk)` → `Decision::consumed()`)が、活性・非活性を問わず Phase 1 より前で止めるので、リピート Down は遅いルールに到達しない。
  - **`keymap_latch` には積まない**。これにより、`KeymapLatch` に早い/遅いの種類を持たせる変更、ステップ1 の `was_down` の手当、`runtime/message_handlers.rs` の変更が**要らなくなる**(古い latch の寿命の問題が、遅いルールについては消える。`keymap_latch` はアプリ無効化・watchdog・アンロック・panic でしか解放されないが、エンジンの `active_keys` はフォーカス変更と活性→非活性のたびに flush される)。
  - **既存の役割経路(ADR-206、Phase 1 で Consume した変換 = IME ON の単独押下)と同じ挙動**(R2-S4): (i) 押している間に活性化すると、FSM に Down 無しの親指 Up が届く。(ii) 押している間にフォーカスが移ると、`release_pending_and_reinject`(`flush_pending_key_ups`)が、消費済みの Down に対して KeyUp を再注入し、無変換の KeyUp が Down 無しで 1 つ OS/IME に届く(その後の物理 Up は `UpDuty::None` で素通しされ、もう 1 つ届く)。Down の無い KeyUp は、修飾キーと違って固着を作らない(GJI は無変換の Down で動く)ので、実害は無い見込み。(iii) `phase1_held` が flush で消えた後のリピート Down は冒頭のガードを抜けるが、非活性なら `!was_down`(条件9)で遅いルールに当たらず素通しになり、OS には「リピート Down → Up」が届いて対になる。これらを `src/engine/tests.rs` の単体テストで固定する(flush 時の `ReinjectKey(KeyUp)` を含む)。
  - **早いルールとの分担**(二重の機構にならない): 早いルールはエンジンの**前**で消費するので `keymap_latch` が持ち主、遅いルールはエンジンが打鍵を**見た後**なので `KeyLifecycle` が持ち主。早いルールは `from` に親指キーを禁じたままで、遅いルールは無修飾の無変換/変換に限るので、vk の集合は交わらない(`KeymapTable::new` のテストで固定する)。
- **親指ラッチとの整合**(r1 M3): フック側の親指ラッチ(`HOOK_STATE.left_thumb_down_scan`、ADR-129)は、`[[keymap]]` が Down を消費しても Up まで立つ。押している間に IME が ON になると、そのあとの文字キーは「親指が押されている」スナップショットを持ってエンジンへ届く。**期待値は既存の役割経路(変換 = IME ON の単独押下)と同じにする**(実装前に現挙動を調べ、検証計画1で固定する。違えるなら理由を書く。R2-S4)。
- 注入された Space は `INJECTED_MARKER` 付き(ADR-114 決定6)で、フックを素通りして IME(GJI)に届く。

### 決定5: 競合・`from` の禁止の緩和・設定画面(r7)

- **`from` の禁止の緩和**: `keymap.rs::forbidden_target_vk_reason` は `from` に**親指キー**を禁じる(ADR-114 決定5)。既定の親指キーは無変換/変換なので、そのままでは遅いルールの `from = "VK_NONCONVERT"` は警告で skip される。**遅いルール(`ime = "off"`)に限り、`from` の主キーに無変換/変換(親指キー)を許す**(決定1: 遅いルールの `from` は無修飾の無変換/変換に限る)。理由: IME OFF(エンジン非活性)では親指キーに役割がなく、ADR-114 決定5 が守ろうとした「親指キーの held 判定との二重管理」は、遅いルールがエンジンの素通しの後にしか当たらないので起きない。IME 制御系 VK・Alt/Win 系・Ctrl/Shift の主キー・`VK_CAPITAL` の禁止は変えない。`ime` 省略のルールは従来どおり親指キーを禁じる。`to` の禁止は変えない。
- **衝突の警告**(新規実装。「既存の衝突警告に載せる」ではない。`warn_if_vk_conflicts` は dedicated fn key の2箇所だけで、`keys.ime_*` との衝突警告は実在しない。r1 M6-2): 設定読み込み時に、遅いルールの `from` が `keys.ime_*`・`muhenkan_solo_tap_dedicated_fn_key`・IME 設定/学習表由来の役割のあるキーと重なれば警告する(動作は、決定2 の条件6 により、そのキーでは発動しないだけ)。
- **awase が読めない IME 側の割り当て**(MS-IME 本体の「キーとタッチのカスタマイズ」、TIP 未同定、表が読めない場合)は `Unknown` になり、**遅いルールは何もしない**(R4-S6)。
- **設定画面**: ショートカット再割り当てタブに「IME の状態」の列(指定なし/OFF のとき)を足す。無変換/変換の Space 化のための簡単な入口(プリセットのボタン等)は、実装時に別途決める(疑問5)。注記に、(1) リピートしない、(2) 「半角英数」(IME は開いたまま英数モード)の状態では動かない、(3) awase が IME 状態を取り違えているとき(IME は ON なのに awase が OFF と思っているとき)は、無変換が Space(IME によっては変換や全角スペース)になることがある、(4) IME の割り当てが読めないときは何もしない、(5) GJI の CUSTOM に直接入力の無変換/変換の行があると動かない、を書く。
- panic 検出(`record_ime_keydown`)は `deliver_key_event` より前に数えるので、Space に変えても計数は増減しない(R2-S7)。

### 決定6: リリース

- オプトイン(ルールを書いた人だけ)。Scancode Map(ADR-230)や、かな/無変換/変換の挙動変更とは同時にリリースしない。回帰テスト(検証計画1・2)が通り、develop で実機確認が済むまで次の v2 リリースに入れない。v1 への backport はしない(新機能)。

### 決定7: 範囲外

- `NotRomajiInput`(半角英数)での Space 化(r1 S3)。報告者の「IME OFF」がタスクバーの「A」(半角英数)なのか、直接入力なのかを先に確認する。
- `ime = "on"`(ON のとき用のルール)。必要になったら別 ADR。
- GJI の CUSTOM 表の生成(A3)。

### 決定8: 案 K が重すぎると分かったときの引き返し先(r7)

案 K で、次のいずれかが実装・検証で重いと分かったら、案 A1(エンジン内の専用設定)へ切り替える ADR を起こす: (1) `KeyLifecycle` への登録が役割経路(ADR-206)と同じ挙動になることを単体テストで固定しきれない(決定4)、(2) `kp_run_inner` の前段(`kp_stage_shadow_ime_toggle` 等)が無変換に対して書く副作用の除去が複雑、(3) `PassThroughWith` の effects の扱いで想定外の食い違いが出る、(4) 送信を effects に載せたとき、出力層の warm/cold 判断(`state/warm_send_plan.rs`)との整合が重い(R7-M1 の修正の重さが、案 K と案 A1 の比較で最も効く点)。

### 影響範囲と再発ファミリー(R2-M5、r7)

`src/config.rs`(`KeymapRule` に `ime`、`"off"` 以外は警告)、`crates/awase-windows-core/src/keymap.rs`(コンパイル:遅いルールを分ける、`find_match` は遅いルールを除外、新しい `find_late_match`、禁止の緩和)、`runtime/key_pipeline.rs`(`kp_run_inner` の `engine.on_input` 直後の遅いルールの照合。送信は effects に積む)、`filter_active` と `warn_if_vk_conflicts` は早い・遅いの両方の集合に対して呼ぶ(R7-S3)、`src/engine/engine.rs` に `record_shell_consumed`(決定4)、`src/engine/engine.rs`(読み取り関数 `ime_off_inactive`)、`state/` に `KeyDirectInputEffect` の純粋関数と、遅いルールの発動可否を決める純粋関数(Linux でテストできるよう `runtime/` に置かない)、`hook.rs`(`impersonated` の印)、`crates/awase-gji-config/src/role.rs`(プリセットの `DirectInput` 行の本番定数)、`crates/awase-settings`(再割り当てタブの列)。`fix-requires-evidence.md` の再発ファミリーの**物理キー押下ラッチ(Down/Up 非対称)**に触れ、**キー選択**にも隣接する(無変換/変換の扱い)。同じ PR に (a) 回帰テストを含める。置き場所は `src/engine/tests.rs`(`cargo test --lib`)と `state/` の純粋関数のテスト。`runtime/` 配下の `#[cfg(test)]` は Linux に存在しないので使わない。

## 検証計画

0. **実装前の確認と計測**(R2-B1、R3-M1、R3-S3、r6): (i) **報告者への確認4点(決定3b)**。(ii) 入力先の分類の確認: CI の入力先(ADR-193 の RichEdit スーパークラス化は「TsfNative 相当」)と、素の Edit コントロール(Standard の IMM の窓)で、`AppImeProfile` と `cannot_verify_real_ime_state` の値をログで確かめる。Windows 11 のメモ帳の分類もあわせて確かめる(決定3 の見直しで「動くかどうか」の条件ではなくなったが、belief の外れやすさの見積もりに使う)。(iii) **計測の目的は「belief が外れる頻度」**: アプリ種別(Standard・TsfNative 相当・Chrome・Windows Terminal)ごとに、無変換を押した時点の「エンジンの判定(`Inactive(ImeOff)`)」と「実 IME の open(スパイクの `A(open)`)」を突き合わせ、「belief は OFF、実際は ON」の割合を数える(決定3 の「belief が外れたときの実害」の大きさ)。大きければ疑問1 の絞りを別 ADR で検討する。**無変換を Space に変えると、その打鍵が IME に届かず、物理 IME キーを契機に走る観測(refresh・ADR-188 の窓内の直接読み)の機会が失われる**ので、belief が外れている間に無変換を繰り返し押すと外れが続きやすい。実害は小さい見込みだが、「belief が OFF・実際は ON」の持続時間を測るときは、Space 化を有効にした構成と無効の構成で比べて、この影響を区別する(R6-S5)。(iv) 正の対照(スペースキー 0x20 の注入で入力欄の `tail` に空白が出るか)、直接入力行の変換(0x1C)の IMEOn、設定ダイアログで直接入力行に InsertSpace を選べるか(仮説 a〜c の切り分け)。(v) 基準構成の「無変換・変換とも open は 0 のまま」は確認済み: run 38059338837 のジョブ `e2e (sc-direct-space-baseline-noawase-1)` と `e2e (sc-direct-space-baseline-awase-1)` の成果物 `dist/ime_key_matrix_spike.log` の KEY 行(2026-10-10 に確認)。各 n=1。
1. **Linux 単体**(`cargo test --lib`、判断は `state/` の純粋関数: 遅いルールの発動可否): 発動条件の表(エンジンの非活性理由 × InputRelay・戻り待ち × composing × 注入 × なりすまし由来 × 修飾 × 従来の4源 × **`KeyDirectInputEffect` の各行(GJI の `DirectInput` 行あり/なし・オーバーレイ・CUSTOM の `ctl-loaded` 表・表なし(`Unknown`)・MS-IME の値 0〜3(`Unknown`)・TIP 未同定(`Unknown`)・**CUSTOM の `DirectInput` 行が InsertSpace 系(機能ありで発動しない。R4-M2)・修飾付きの行〈`Shift Henkan`〉のみ(無修飾は `NoFunction`。R4-S3)・未知のオーバーレイ/未知の `session_keymap`(`Unknown`。R4-S2)・4プリセットの変換/無変換(決定2-4 の表)**)** × `was_down` × 設定値)。InputRelay・戻り待ちのとき発動しない。**`cannot_verify_real_ime_state` が真でも、エンジンが `Inactive(ImeOff)` なら発動する**(決定3。観測の鮮度に依らないことを固定する)。**加えて**: 素通しの KeyDown が `KeyLifecycle` に記録を残さないこと(`src/engine/tests.rs`、決定2)、遅いルールの KeyUp を latch が先に消費してもエンジンの状態が壊れないこと、`was_down` が偽の Down で古い latch を捨てて再照合すること(決定4)、`PassThroughWith` の effects が `force_consume` で保たれること。`KeyLifecycle` の Down/Up/リピート。活性化中の押下の期待値(決定4)。網羅 `match` による `matches_ime_set_open`/`matches_ime_off` の固定。
2. **Windows CI**(`e2e-ime.yml`): 構成に GJI の CUSTOM 行(InsertSpace 等)を**入れない**。入力先は**Standard の IMM の窓と、TsfNative 相当の窓(ADR-193 の RichEdit スーパークラス化)の両方**にする(決定3 の見直しで、どちらでも発動する)。構成は `[[keymap]]` に遅いルール(`from = "VK_NONCONVERT"`、`to = ["VK_SPACE"]`、`ime = "off"`)を書いた config を使う。(a) IME OFF の無変換で入力欄に空白が1つ入る、(b) IME ON では入らず、無変換+文字キーが親指シフト文字になる、(c) **負の対照**(追加: 「foo bar」と速く打ったとき Space の位置が入れ替わらない〈R7-M1。入力先の `tail` の文字順を見る〉、リピート Down〈`was_down`〉に当たらず Up まで素通しのまま〈R7-M2〉): 注入された 0x1D では入らない/composition 中は入らない(Standard の窓で)/ `DirectInput,Henkan,IMEOn` の行がある構成と、GJI の「変換/無変換で IME ON/OFF」オーバーレイの構成で、変換が IME を開き Space にならない/ Alt なりすまし + GJI 側からの IME OFF → 最初の Alt が Alt のまま/ Ctrl+無変換(救済窓)と Shift+無変換が従来どおり/ 長押しで Space が1個だけ。(d) **belief が外れた場合の確認**(TsfNative 相当の窓): IME を実際に ON にしたまま awase の belief が OFF になっている状態(外部から IME を ON にして、awase が観測する前)で無変換を押したときの結果を記録する(決定3 の「belief が外れたときの実害」)。記録項目に「入力欄に何が入ったか(半角/全角スペース、変換)」を含める(R6-S3)。
3. **実機**: 報告者の構成(GJI の設定、親指キー、`keys.ime_*`、アプリ)。

(**注**: r6 で決定3 を置き換えたため、以下の対応表のうち `derive_actuating`/`ime_off_confirmed`/`cannot_verify_real_ime_state` に関する行は失効している。現行の決定は「決定」の節が正。)

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
| R3-B1 | 反映(決定2-4)。`KeyDirectInputEffect = HasFunction\|NoFunction\|Unknown` を新設し、発動は `NoFunction` のときだけ。MS-IME 本体・TIP 未同定・表が読めないときは `Unknown`。`DirectInput` 行の IMEOn 以外のコマンドも機能ありに数える。当面発動するのは GJI で表が読め、そのキーの行もオーバーレイも無いときだけ |
| R3-M1 | 反映(決定3、決定3b)。述語を `cannot_verify_real_ime_state` に確定し、実効範囲を明記。報告者の確認3点を実装着手のゲートにした |
| R3-M2 | 反映(検証計画0-ii、2)。CI の入力先は Standard の IMM の窓。TsfNative 相当の窓は「入らない」の対照 |
| R3-S1 | 反映(決定2-3)。フックが `impersonated` の印を付ける |
| R3-S2 | 反映(決定4)。実装時に `emit_set_open` と actuation 合流点ファミリーへの該当を確認、`press=None` |
| R3-S3 | 反映(検証計画0・1・2) |
| R3-S4 | 反映(決定3、検証計画0-v)。Chrome は `Imm32Unavailable`、WezTerm/Windows Terminal は `TsfNative`。基準構成の確認に run ID とジョブ名を添えた |

## Opus レビュー r4 への対応(指摘 ID ごと)

| ID | 対応 |
| --- | --- |
| R4-M1 | 反映(決定2-4)。プリセットの `DirectInput` 行を本番の定数として足す(出典 Mozc `b4bbc42f`)。判定表を載せ、「報告の構成に合う」を取り下げた(既定の MS-IME では無変換だけ、ATOK では両方ならない) |
| R4-M2 | 反映(決定2-4、決定3b-4)。報告者の InsertSpace 行は機能ありに数えるので発動しない。案内は「CUSTOM から該当行を消す」(推奨 a)。InsertSpace 系を `NoFunction` に数える案(b)は仮説 a の確認後 |
| R4-M3 | 反映(決定3b-1)。トレイの「不具合を報告」で集める。確認事項は4点に |
| R4-S1 | 反映(未解決の疑問)。再変換を上書きする選択肢は需要が出てから |
| R4-S2 | 反映(決定2-4)。未知のオーバーレイ・未知の `session_keymap` は `Unknown` |
| R4-S3 | 反映(決定2-4)。無修飾のキー名の完全一致 |
| R4-S4 | 反映(決定2-4、影響範囲)。`enrich_thumb_key_role` と同じ形で押した側だけ書く |
| R4-S5 | 反映(決定4)。`check_active_transition` は元から `SetOpen` を出さない(r5 で事実の書き方を訂正) |
| R4-S6 | 反映(決定3、決定5)。メモ帳の注記、MS-IME 本体の注記の書き換え |

## Opus レビュー r5 への対応(新規 Blocker/Must なし。Should 6 件)

| ID | 対応 |
| --- | --- |
| R5-S1 | 反映(決定4)。`check_active_transition` は元から `SetOpen` を出さない。テストで固定 |
| R5-S2 | 反映(決定2-4)。`KeyStates::of`・`Effect::of` を引用して閉じた |
| R5-S3 | 反映。`KeyOpenEffect` → `KeyDirectInputEffect = HasFunction \| NoFunction \| Unknown` に改名 |
| R5-S4 | 反映。review ファイルに frontmatter を付け、round1 に改名の注記。index の補助資料の行も更新 |
| R5-S5 | 反映。ステータスを更新し、回答が無い間は実装しない旨を決定3b に書いた |
| R5-S6 | 反映(決定3b)。ゲートが止めるのは実装だけ。CI スパイクは回答を待たず進めてよい |

## 所有者の提案による改訂(r6)

| 変更 | 内容 |
| --- | --- |
| 決定1 | IME OFF のときだけ指定できる(ON 用の設定は作らない)と明記 |
| 決定2-5・2-6 | r4 の改訂で誤って削除されていた項目5・6 を復元。項目5 は `ime_off_confirmed`(`derive_actuating`)を廃止し、除外は InputRelay と戻り待ちだけに。項目6(composition なし)は元の内容 |
| 決定3 | 置き換え。IME OFF の判定はエンジン自身の belief だけ。`cannot_verify_real_ime_state` で除外しない。belief が外れたときの実害と記録を明記 |
| 決定3b | アプリの種類は「動くかどうか」の条件ではなくなった。ゲートは報告者のキー設定が中心 |
| 検証計画 | 計測の目的を「belief が外れる頻度」に。CI の入力先は Standard と TsfNative 相当の両方。belief が外れた場合の確認を追加 |
| 設定画面の注記 | 「TSF ネイティブでは動かない」を削除し、「awase が IME 状態を取り違えているとき Space になることがある」を追加 |
| 影響する旧指摘 | R2-B1(`derive_actuating` の鮮度)・R3-M1(実効範囲)・R3-M2(CI の入力先)。R2-M3(InputRelay)は決定2-5 で維持 |

## Opus レビュー r6 への対応(新規 Blocker/Must なし。Should 5 件)

| ID | 対応 |
| --- | --- |
| R6-S1 | 反映。影響範囲・検証計画1 の `ime_off_confirmed` を `thumb_space_blocked` に。旧対応表に失効の注記 |
| R6-S2 | 反映(決定2-6、検証計画2c)。TSF での composition フラグは未検証と明記し、CI で確かめる |
| R6-S3 | 反映(決定3、設定画面の注記、検証計画2d)。IME が Space を変換・全角スペースと解釈しうる |
| R6-S4 | 反映(疑問1)。最小の絞りの候補を併記 |
| R6-S5 | 反映(検証計画0-iii)。観測の機会の喪失を区別して測る |

## 所有者の判断による改訂(r7)

| 変更 | 内容 |
| --- | --- |
| 方式 | 専用設定(案 A1、エンジン内)から、`[[keymap]]` に `ime = "off"` を足す案 K に変更。照合はエンジンの判断の**後**(`kp_run_inner` の `engine.on_input` 直後、素通しのときだけ) |
| 決定1 | `[[keymap]]` の `ime` 項目。遅いルールはエンジンの前の照合では見ない |
| 決定2 | 照合の位置・理由・発動条件を全面的に書き直し。r2〜r6 の条件(belief だけ・`KeyDirectInputEffect`・InputRelay/戻り待ち・composition・注入・Alt なりすまし)は維持 |
| 決定4・5 | latch の `was_down` の手当、親指ラッチとの整合、`from` の禁止の緩和(遅いルールだけ)、衝突警告、設定画面 |
| 決定8 | 案 K が重すぎるときの案 A1 への引き返し先 |
| 失効 | r1〜r6 の対応表のうち、`SpecialKeyMatch` の新 variant・`match_special_keys` の末尾・`prepend_effects`・`thumb_space_blocked` に関する行は、案 A1 の記録(現行の決定は「決定」の節が正) |

## Opus レビュー r7 への対応(新規 Must 3 件・Should 4 件)

| ID | 対応 |
| --- | --- |
| R7-M1 | 反映(決定2)。Space は `send_keymap_target` でその場で送らず、`decision.push_effect(SendKeys)` で effects の末尾に積む。決定8 に引き返し条件(4) を追加 |
| R7-M2 | 反映(決定2 の条件9)。`!event.was_down` を追加 |
| R7-M3 | 反映(決定1、決定5)。遅いルールの `from` は無修飾の無変換/変換に限る |
| R7-S1 | 不要になった(決定4)。`keymap_latch` に積まず、エンジンの `KeyLifecycle` に登録する(所有者の提案を採用) |
| R7-S2 | 反映(決定2)。発動の印を journal/ログに残す |
| R7-S3 | 反映(影響範囲)。`filter_active`・`warn_if_vk_conflicts` は両方の集合に |
| R7-S4 | 反映(決定2 の条件3)。条件1 に含まれる旨を注記 |
| 追加 | CI 計測(run 38063068778)の結果をコンテキストに反映。仮説 b・c を否定 |
| 追加評価 | 所有者の提案(消費した Down を `Engine::record_shell_consumed` でエンジンの `KeyLifecycle` に登録し、`keymap_latch` には積まない)を採用(決定4)。S1(latch の種類・`was_down` の破棄)と `message_handlers.rs` の変更は不要になった |

## 未解決の疑問

1. 決定3 の見直し後、観測品質で絞る最小の案を要するか。最小の候補は「`resolve_open_at` の base が `MostRecentTrusted` の Low ソース(`HeuristicDefault` 等)のときは発動しない」(R6-S4)。なお `desired_open` の初期値は true なので、情報ゼロの起動直後はもともと発動しない。絞りが要るかは検証計画0-iii で決める。検証計画0-iii の「belief は OFF、実際は ON」の割合しだい。Windows 11 のメモ帳など XAML/RichEdit 系の分類は未確認(検証計画0-ii)。
2. 遅いルールの照合時の `ctx` は、`engine.on_input` に渡したものと同じ。エンジンの `compute_state` を読み取り関数 `ime_off_inactive(&ctx)` で参照する。IntentStore の上書きが「開」ならエンジンは活性になり素通しにならない(安全側)。
3. (閉じた)遅いルールの `from` は無修飾の無変換/変換に限る(決定1)。他のキーへ広げる場合は別 ADR。
4. リピートしない仕様で報告者の期待に合うか。
5. 無変換/変換の Space 化のための簡単な入口(設定画面のプリセットボタン等)を足すか、再割り当てタブの手入力だけにするか。`ime` の項目名(`ime` / `when_ime`)。`ime` は `[keys] ime_on` 等と字面が近い(r1 S8)。
6. エンジンの判断(発動・不発動の理由)を ADR-250 の境界 journal に残すか、debug ログだけにするか(報告 journal で「なぜ Space にならなかったか」を追えるように)。
7. 報告者への確認4点(決定3b)は未回答。回答次第で実装するか保留するかが決まる。
8. MS-IME 本体の無変換/変換の値 0〜3 と「値なし」の既定の効果。確かめるまで `Unknown`(発動しない)のままだが、MS-IME 利用者にも効かせる要望が出たときの測定方法。
9. 再変換(既定の MS-IME プリセットの変換は `Reconvert`)を Space で上書きしてよいと選べる余地を残すか。今回は GJI のキー設定を CUSTOM にして該当行を消す案内で足りるはずなので入れず、需要が出てから扱う(R4-S1)。
10. (閉じた)所有者の提案(エンジンへ消費した Down を登録する)は、レビューの追加評価で採ることになった(決定4)。
