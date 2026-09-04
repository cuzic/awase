# ADR-129: OUTPUT_GATE drain replay 中、親指キー押下タイムスタンプがイベント捕捉時点ではなくリプレイ実行時点のライブ値で再構築され、既に消費済みの押下と無関係な後続押下がペアリングされる

## ステータス

**根本原因確定・opus-adversarial-consult round2完了で収束（実装未着手）。**
report `01M1N36MGDDJ5HN8FWRE4ZHS3J`（2026-09-04、タスクトレイ「不具合を報告」機能、
[ADR-095](095-tray-bug-report-cloudflare-intake.md)）から起票。
`docs/bug-reports-triage.md` に一次調査結果を記録済み。既存の `known-bugs.md`
BUG 番号は未採番（本 ADR 確定後に起票する）。

**round1（v1→v2）:** 根本原因の特定は正しく、一次証拠から独立に再構成した
結果、当初案より**強く**証明できることが判明した一方（証拠を推測から観測へ
格上げ）、以下の欠陥が見つかり v2 で修正した:
初版のイベント区分の誤り（手順1は実はライブ配送ではなく drain replay
だった。ただし delta が小さく無害だっただけ）、未決定事項2・3の自己矛盾
（`build_ctx()` は実は drain replay 経路そのものだった）、テスト節が
実質的に何も検証しない（修正前後どちらでも通る）、修正が問題の半分
（キーイベント経路）しか閉じずタイマー経路に具体的な失敗モードが残る
ことの過小評価、検討していなかった第4の代案とその却下理由。

**round2（v2確認）:** 行番号・ログ行・数値を実ファイルで再検証した結果、
「指摘1〜13すべて反映済み、ブロッカーなし」との判定。軽微な修正6件
（`is_thumb_consumed` の行番号誤記、限界節がタイマー経路2箇所の危険度を
一括りにしていた点、`Idle` からの即時解決経路が `ActiveThumb` 以外に
`ShiftPlane` もあり得ることの排除根拠不足、却下案(c)の記述と訂正後の
手順1の噛み合わせ、決定ステップ2の「直後」という要件の過剰な厳格化、
構築サイト列挙でコンストラクタ関数をテスト用途と一括りにしていた点）を
本版で反映済み。これ以上のラウンドは不要と判定されている。

## 問題

`crates/awase-windows/src/runtime/key_pipeline.rs:105` は、`Engine::on_input()`
に渡す `InputContext` を組み立てる直前に

```rust
let (left_thumb_down, right_thumb_down) = hook::thumb_down_timestamps();
```

を呼ぶ。`hook::thumb_down_timestamps()`（`hook.rs:519`）は、`WH_KEYBOARD_LL`
フックコールバック自身が実時間で更新するグローバル `AtomicU64`
（`LEFT_THUMB_DOWN_AT_US` / `RIGHT_THUMB_DOWN_AT_US`、`hook.rs:190-191`,
クロージャ定義 `:1046`、呼び出し `:1057`(左)/`:1060`(右)）を、呼び出された
**その瞬間の値**で読む。

この関数を呼ぶ `kp_run_inner`（`key_pipeline.rs:47`）は、フックからの
ライブ配送と、`OUTPUT_GATE` が active な間 `INPUT_DEFER` に退避された
イベントの drain replay（`message_handlers.rs::handle_wm_drain_output_queue`
（`WM_DRAIN_OUTPUT_QUEUE` ハンドラ）→ `deliver_key_event(app, *queued_event,
KeyOrigin::DeferredReplay)`（`message_handlers.rs:1369`）→
`app.process_key_event`（`:227`）→ `key_pipeline.rs:34` → `kp_run_inner`）の
**両方**から同一コードパスで呼ばれる。別経路は存在しない
（`ImeOffRescueReplay` も `replay_ime_off_rescue_event` → 同じ
`kp_run_inner(event, true)` を通り、`:105` を通過する）。

問題の本質は「ライブ配送か replay か」ではない。**イベントが実発生してから
`kp_run_inner` で実際に処理されるまでの delta（`event.timestamp` と
`hook::thumb_down_timestamps()` を呼んだ時刻の差）が小さければ無害、
大きければ壊れる。** ライブ配送は delta がほぼ0なので通常は無害だが、
`INPUT_DEFER` に退避された古いイベントを後からバーストで drain replay
すると、この delta が数百 ms に広がりうる。`ctx.right_thumb_down` は
「イベントの実発生時刻」ではなく「replay を実行している"今"」のライブ値に
なるため、**イベントの他の全フィールド（`timestamp`・FSM 状態遷移の入力）
と時間軸が食い違う。**

## なぜ実害になるか（app_log の実測、journal は補強のみ）

`report_id: 01M1N36MGDDJ5HN8FWRE4ZHS3J` のタイムラインは **`app_log_excerpt`
の実ログ行から再構成した**（`[drain-start]`/`[drain]`/`[output-drain]
replay`/`[engine-input]`/`send_keys`/`send_char_as_tsf`）。同梱の journal
（`log_excerpt`）は先頭エントリが `DumpTruncated`
（`budget_bytes=204800, total_entries=2500, emitted_entries=957,
dropped_key_input=421`）で残存 KeyInput エントリが91件しかなく、**本タイム
ラインの論証には使っていない**（補強のみ、単独では該当キーが truncate
で欠落しうる）。`ts` は `event.timestamp`（device tick、us、以下すべて
フル値で表記——切り詰めた下6桁だけを比較すると異なる接頭辞を持つ値を
誤って同一視しかねないため）。GJI/TSF、Uwp/TsfNative アプリ、
「ようするに」と入力→「よゔするに」。

1. `ts=8453961165` 右親指キー(`vk=0x1C`) ↓。**このイベント自体が
   drain replay である**（app.log:869 `[output-drain] replay vk=0x1C
   KeyDown event_ts=8453961165us now=8453964184us delta=3ms`）。ただし
   delta=3ms と極小のため実害はない。
2. `ts=8453967259`（+6094us） `Y`(`vk=0x59`) ↓。これはライブ配送
   （対応する `[output-drain] replay` 行が存在しない、app.log:873）。
   `PendingThumb` + `is_simultaneous` 成立 → `step_pending_thumb_char`
   （`nicola_fsm.rs:1578`）が即座に `Char('よ')` を確定・送出し、
   `right_thumb_consumed = phys.right_thumb_down`（＝ `8453961165`）で
   親指を「消費済み」にマークする（[ADR-010](010-thumb-consumption-timestamp.md)）。
3. 直後、GJI 候補ウィンドウの SHOW を検知して `StartComposition while cold`
   → `OUTPUT_GATE` が active 化（`depth 0→1`）。以降の物理イベントは
   `INPUT_DEFER` へ退避される。
4. 退避された8件（実発生順、フル値）: `Y↑(8454029694)` →
   `A↓(8454050766)` → `右親指↑(8454054698)` → `A↑(8454150338)` →
   `C↓(8454240146)` → `I↓(8454310559)` → `右親指↓(8454313529、2回目の
   物理押下)` → `C↑(8454335374)`。
5. ~300ms 後、TSF probe 完了で `OUTPUT_GATE` が deactivate
   （`depth 1→0`）→ `WM_DRAIN_OUTPUT_QUEUE` が上記8件を実発生順のまま
   一括で `deliver_key_event(..., KeyOrigin::DeferredReplay)` へ流す
   （`now_us` は全件でほぼ同一の `8454376674`）。
6. `A↓(8454050766)` の replay 時点で `hook::thumb_down_timestamps()` を
   ライブクエリすると `Some(8454313529)` を返す——**推測ではなく
   `[engine-input]` の `state=` フィールドから消去法で確定できる**（次項）。
7. **観測的な裏付け（`state=` フィールド、app.log:965/967 と :970-974 の対比）:**
   ```
   [engine-input] vk=0x41 KeyDown ts=8454050766us state=Idle mods(c=false s=false a=false w=false)  ← A↓
   [engine-input] vk=0x1C KeyUp   ts=8454054698us state=Idle        ← 次イベントも Idle（A↓は何も pending 化しなかった）
   [engine-input] vk=0x43 KeyDown ts=8454240146us state=Idle        ← C↓
   [engine-input] vk=0x49 KeyDown ts=8454310559us state=PendingChar(vk=0x43)  ← C↓ は PendingChar を作った
   ```
   `A` は layout char キーであり、通常経路（`decide_idle` →
   `classify_idle_intent`、`:1310`）で `Idle` から即時解決される分岐は
   `ShiftPlane`（`shift_face_reduce`）と `ActiveThumb`（`active_thumb_side()`
   （`:2790`）が `Some` を返す場合の `reduce_active_thumb`、`:1355`）の2つ
   だが、`mods(c=false s=false a=false w=false)`（上記ログ行）で `Shift`
   非押下が確定しており `should_use_shift_plane` は成立しないため
   `ShiftPlane` は排除できる。よって残る即時解決経路は `ActiveThumb` のみ
   ——それ以外（未消費の親指なし）なら `A` も `C` と同様 `PendingChar` を
   作るはずである
   （`confirm_mode` は未指定=`idle_wait`、`Timer set: logical=1, ms=100`
   が `simultaneous_threshold_ms=100` と一致することからも `PendingChar`
   経路が使われていることが裏付けられる）。A↓ が `PendingChar` を作らな
   かった以上、`active_thumb_side()` は `Some` を返した。`is_thumb_consumed`
   （`:2780`）の比較 `right_thumb_consumed(=8453961165) == phys.right_thumb_down`
   が不一致だったということは、`phys.right_thumb_down` は `8453961165`
   ではない別の値であり、この時間窓でその Atomic が取りうる値は手順4の
   2回目の押下 `8454313529` しかない。よって「ライブクエリが
   `Some(8454313529)` を返した」は観測から演繹できる事実であり、推測
   （反実仮想）ではない。
8. **なぜ `C↓` は同じライブ値 `8454313529` を見ながら親指シフトされ
   なかったか**: `A` の解決（`reduce_active_thumb` → `consume_thumb`、
   `nicola_fsm.rs:1026-1029`）が `right_thumb_consumed =
   self.phys.right_thumb_down = 8454313529` と書き込んだため、その後
   `C↓` の時点では `is_thumb_consumed` が `8454313529 == 8454313529`
   （一致）→ 消費済みと判定され `ActiveThumb` が成立しなかった
   （app.log:971 `state=Idle` → :974 で `PendingChar(vk=0x43)` に遷移、
   通常の char-first フロー）。これは手順7の推論の独立した裏付けでもある
   ——`A` の消費書き込みがなければ `C` も同じライブ値で誤ペアリング
   されていたはずである。
9. `A` は `RightThumb+A` として同時打鍵確定される。
   `layout/nicola_keytop.yab` の `[ローマ字右親指シフト]` 面、A行(home row)
   1列目は `ｖｕ`（ゔ）。実際の送出ログ
   `send_keys: mode=Tsf actions=[Char('ゔ')] prev_elapsed=0ms` /
   `send_char_as_tsf: 'ゔ' → romaji "vu"` と一致する。本来 `A` 単独が
   期待する出力は同面 no-shift の `ｕ`（う）。`crates/awase-vkmap/src/lib.rs:51`
   `0x41 => (2, 0)` で物理位置の対応も確認済み。

**因果の要点:** 「消費済みマークとの不一致」自体は [ADR-010](010-thumb-consumption-timestamp.md)
の設計どおり正しく機能している。壊れているのは比較対象の
`phys.right_thumb_down` の**取得タイミング**であり、`A↓` の replay に対して
「`A↓` が実際に起きた時点の値」ではなく「replay を実行している現在時刻の
値」を渡してしまっている点にある。

## 既存の類似修正との整合性（半分だけ適用済みだった）

`RawKeyEvent::modifier_snapshot`（`src/types.rs:206-211`、フィールド本体
`:212`）は、**同じクラスの問題を Ctrl/Shift/Alt/Win について既に部分的に
解決している**。doc comment:

> フック時点でキャプチャした修飾キー状態スナップショット。
> `GetAsyncKeyState` を replay 時ではなく capture 時に呼ぶことで、
> OUTPUT_PENDING_QUEUE 経由の drain 時に modifier 状態が変化していても
> 正しい文脈でイベントを再処理できる。

（doc comment は `OUTPUT_PENDING_QUEUE` と書いているが、本件で問題になって
いる実際のキューは `crate::INPUT_DEFER`——`handle_wm_drain_output_queue`
が `WM_DRAIN_OUTPUT_QUEUE` で drain する入力側キュー——であり別物。同一
クラスの問題ではあるが、この doc comment を「本経路を指している」と読む
のは不正確。修正時に doc comment 側にも `INPUT_DEFER` への言及を足す。）

`hook.rs` は `read_os_modifiers()`（`:1081`、以下 `LLKHF_ALTDOWN`/
alt-なりすまし補正を挟んで `build_raw_key_event` 呼び出しは `:1092`）を
呼び、`RawKeyEvent` に埋め込んで `INPUT_DEFER`/replay を素通りさせている。
`update_thumb`（`:1046-1062`）は `:1081` より**前**にあり、順序関係は
成立している。

**ただしこの前例自体、完了した修正ではなく部分修正である。**
`Runtime::build_ctx`（`runtime/mod.rs:283-292`）は `ctx.modifiers` を
ライブの `read_os_modifiers()` から作っており、同所の既存コメントが
「`hook.rs` 側の `RawKeyEvent.modifier_snapshot` は正しく補正されていても、
`bypass_reason()` が実際に見る `PhysicalKeyState.modifiers` はこの
`build_ctx()` の戻り値から来る（別経路）」と、この分断を明示的に認めて
いる。つまり本 ADR が踏襲しようとしている前例は「キーイベント経路だけ
直った半分の修正」であり、これは後述の「限界」節（`build_ctx` が親指
タイムスタンプについても同じ穴を残す）と完全に同型である。本 ADR も
「適用漏れの解消」ではなく「前例と同じく、まずキーイベント経路の半分を
直す」と位置づける。

## 除外した対抗仮説

- **BUG-105（3鍵仲裁の `char1_released_at` 早期return）の再発**: 該当しない。
  BUG-105 の早期returnは `compute_prefer_char1`（`nicola_fsm.rs:2046`）から
  既に削除済み（develop `1045a05e`、v1.18.0 に含まれる、
  `git merge-base --is-ancestor` で確認済み）。残る唯一の同種チェックは
  `commit_char1_output`（`:2120`）内の `append_key_up_for` 制御のみで
  ペアリング判定には無関係。加えて本件は3鍵仲裁（`PendingCharThumb` の
  char2 側判定）ではなく、`Idle` 状態での `ActiveThumb` 即時ペアリング
  （`decide_idle`）で発生しており、通過するコードパス自体が異なる。
- **`right_thumb_consumed` の初期化漏れ・リセット漏れ**: `consume_thumb`
  （手順2）は正しく `8453961165` を記録しており、比較値自体は正しい。
  壊れているのは比較対象の `phys.right_thumb_down` 側。

## 決定

### 採用: 親指ダウンタイムスタンプを `modifier_snapshot` と同じ「capture 時点で
`RawKeyEvent` に埋め込む」方式に揃える（キーイベント経路のみ、範囲は限定）

1. `src/types.rs::RawKeyEvent` に、`modifier_snapshot` と対になる
   capture-time スナップショットを**2つの独立フィールド**として追加する:
   `left_thumb_down_snapshot: Option<Timestamp>` /
   `right_thumb_down_snapshot: Option<Timestamp>`。
   （タプル `(Option<Timestamp>, Option<Timestamp>)` 案は却下——
   `InputContext`/`build_input_context`（`runtime/mod.rs:66-84`）も唯一の
   消費点 `key_pipeline.rs:105-113` も既に左右を独立した2引数で扱って
   おり、タプル化すると `.0`/`.1` のどちらが左右か呼び出し側で不明瞭に
   なるだけで実利がない。）
   `RawKeyEvent` は `Copy` で `hook_channel.rs` のリングバッファと
   `INPUT_DEFER` に値渡しで積まれるため、追加はサイズ増以外の影響がない。
   構築サイトは本番1箇所（`hook.rs:772`）＋コンストラクタ1箇所
   （`src/types.rs:325-326`、本番コードから呼ばれうるため「テスト用途」に
   一括りにしない）＋テスト/ダミー用途10箇所
   （`src/engine/input_tracker.rs:213`,
   `src/engine/nicola_fsm.rs:3191`, `src/engine/key_lifecycle.rs:115`,
   `src/engine/fsm_adapter.rs:317`, `hook_channel.rs:216`,
   `input_defer.rs:128`, `runtime/transport.rs:279/306/339`,
   `tsf/tsf_gate.rs:657`）——実装時に全て確認する。
2. `hook.rs` の `build_raw_key_event` 呼び出し（`:1092`）に渡す引数として、
   `modifier_snapshot` 構築（`:1081`）と同じ場所で `thumb_down_timestamps()`
   を呼び、`RawKeyEvent` に埋め込む。要件は「`update_thumb`
   （`:1046-1062`）より**後**であること」で足り、`:1081` の時点で満たす
   ——`:1062` の直後に押し込む必要はなく、間に挟まる `classify_key`
   や alt なりすまし補正との順序を気にする必要もない。当該キー自身の
   ↓/↑ による親指状態の変化を反映した値を capture することで、
   「このキー自身が親指キーだった場合」も含めて正しい値になる。
3. `key_pipeline.rs:105` の `hook::thumb_down_timestamps()` ライブ呼び出しを
   `event.left_thumb_down_snapshot` / `event.right_thumb_down_snapshot`
   の読み取りに置き換える。ライブ配送と drain replay が**同一コードパス**
   になり、両者の分岐自体を無くす。

**根拠:**

- `modifier_snapshot` で既に実証済みの、同一クラスの問題に対する同一
  リポジトリ内の解法をそのまま踏襲する。新しい設計判断を持ち込まない。
- `RawKeyEvent` はプラットフォーム非依存の `awase` core crate
  （`src/types.rs`）に定義されており、`Option<Timestamp>` を足すだけなら
  ADR-019（core は OS 非依存）に抵触しない。
- 修正箇所が「capture 時点の値を運ぶ」という1点に閉じ、`NicolaFsm` 側
  （[ADR-010](010-thumb-consumption-timestamp.md) の消費追跡ロジック自体）
  には触れない。

**この決定が閉じるのはキーイベント経路のみであり、問題のクラス全体では
ない。範囲の限界は「限界」節を参照。**

### 却下: 案(a)（`INPUT_DEFER` キューの中身だけから親指状態を再構築する）

gate activation 時点の親指状態を別途スナップショットし、そこから
キュー内の親指キー ↓/↑ を順に適用して各キューイベント時点の値を
再計算する案。却下理由:

1. ライブ配送側の一貫性を改善しない（ライブとreplayで別ロジックのまま）。
2. gate activation 時点の状態を新たに保持する追加のブックキーピングが要り、
   `modifier_snapshot` が既に検証済みの「capture 時点に1件ずつ埋め込む」
   方式より複雑。

### 却下: 案(b)（`NicolaFsm` 側で thumb 状態を明示的な「at time T」引数にする）

`is_thumb_consumed`/`active_thumb_side` に時刻引数を追加し、呼び出し側が
正しい T を渡す責務を負う案。却下理由: バグの実体は「platform 層が渡す
`ctx.right_thumb_down` の取得タイミング」であり、[ADR-010](010-thumb-consumption-timestamp.md)
の比較ロジック自体は正しく機能している。責務が正しく機能している層まで
API 変更で巻き込む必要がない。

### 却下: 案(c)（drain replay 中は `right_thumb_down`/`left_thumb_down` を
常に `None` 扱いにする）

「未来の押下を誤って拾う」誤検出は防げるが、「よ」の例（手順1〜2）のように
**GATE activation 前に正当に消費された親指と、GATE activation 後も
物理的に押されたまま残っている親指が同一押下であるケース**まで
`None` 化すると、正しい同時打鍵の成立自体を壊す（本件の「よ」自体は
手順2のライブ配送の `Y↓` で確定済みなので影響しないが、GATE activation
**後**に本当にチョードが成立するケースが将来 replay に混ざれば同型の
regression になる）。
「常に無効化」ではなく「正しい時点の値を使う」が唯一の一般解。

### 却下: 案(d)（`InputContext` から親指タイムスタンプを削除し、エンジン自身が
受け取る親指キー ↓/↑ の `RawKeyEvent` から親指押下状態を導出する）

層分離原則（platform は分類・捕捉のみ、core が判断）に最も素直に沿う案。
利点は採用案より広い: `RawKeyEvent` への新フィールド追加が不要、ライブと
replay が構造的に同一になる（採用案は同一コードパスにするが値の出所は
capture-time snapshot という1系統に揃うだけ）、**タイマー経路も自動的に
直る**（後述「限界」節の穴が構造的に消える）、`hook.rs` の
`LEFT/RIGHT_THUMB_DOWN_AT_US` グローバル自体をこの用途からは不要にできる。

**却下理由（「複雑だから」ではなく具体的な欠陥）:** `deliver_key_event`
（`message_handlers.rs:136`）には、イベントがエンジンへ到達する前の
早期return が5つある:

| 位置 | 分岐 |
|---|---|
| `:152-175` | `keymap_latch.is_latched(vk)` → KeyDown/KeyUp とも `Consumed` |
| `:177-180` | `Hook(PumpContext::Nested)` → `Reinjected` |
| `:189-193` | `focus_kind == FocusKind::NonText` → `Reinjected` |
| `:206-210` | `consume_keymap_match`（`[[keymap]]` 新規照合、KeyDown のみ） |
| `:224-226` | `consume_post_bypass`（`[[post_bypass]]`） |

親指キーの ↑ がこのいずれかで握り潰されると、**エンジンは親指が
押されっぱなしだと信じ続ける**（無期限のスティッキー親指＝以降すべての
文字キーが親指シフト面で出る）。これは現在のバグ（1文字の誤変換）より
遥かに悪い。特に `FocusKind::NonText` は「フォーカス分類の誤判定で常時
パススルーになる」広いガードで、`deliver_key_event` の doc comment
（`:184-188`）自身が「フォーカス遷移中等で一時的・誤って `NonText` と
分類されていても」というケースを想定して例外を設けているほど、誤判定が
起こりうる前提で書かれている。

案(d)は将来「グローバルも消せてタイマー経路も直る、明らかに上位互換だ」
として再浮上しうるため（[experiment-logging](../../.claude/rules/experiment-logging.md)
と同種の理由）、この却下理由を明示的に残す。

## 限界（この決定が閉じないもの）

採用案は問題のクラス全体を閉じるわけではなく、**キーイベント経路の半分
だけ**を閉じる。

`NicolaFsm::phys`（`nicola_fsm.rs:194`）は `on_event`（`:2987-2988`）と
`on_timeout`（`:3009-3012`）の**両方**で `self.phys = *phys;` と丸ごと
上書きされる永続フィールド。採用案適用後は、

- キーイベント経由 → `event.left/right_thumb_down_snapshot`（capture 時点）
- タイマー経由 → `hook::thumb_down_timestamps()`（ライブ、未修正のまま）。
  ただし危険度は呼び出し箇所で異なる: `message_handlers.rs:611` の
  タイマーハンドラ本体は `OUTPUT_GATE.is_active()`（gate active）中は
  自身を `deferred_engine_timers` へ退避して `:602` で早期returnするため、
  `:611` へ実際に到達するのは gate 非 active（delta≈0）のときのみで
  **無害**。有害なのは gate 解除後に `build_ctx()`（`runtime/mod.rs:294`、
  `message_handlers.rs:1407` から deferred timer replay 用に呼ばれる、
  delta 大）経由でライブクエリされるケースのみ

という**2種類の provenance が同じフィールドに交互に書き込まれる**。しかも
`consume_thumb`（`:1026-1029`）と `resolve_char_and_thumb_as_separate_solos`
（`:2484-2485`、`ThumbSide::Right => self.right_thumb_consumed =
self.phys.right_thumb_down`）は**その `phys` から `*_thumb_consumed` を
書く**ため、2種類の provenance が `is_thumb_consumed` の等値比較
（`nicola_fsm.rs:2786`、`phys_down.is_some() && consumed == phys_down`）
の中で混ざる。

**タイマー経路の具体的な失敗モード（未観測だが、コードから特定済み——
`fix-requires-evidence.md` の趣旨に沿い、次に実機で顕在化したときに
ゼロから再調査せずに済むよう明記する）:**
`resolve_char_and_thumb_as_separate_solos`（`:2459-2467` の doc comment
自身が「タイムアウト経由では thumb はまだ物理的に押されたままなので
明示的に消費済みにする。怠ると `active_thumb_side()` が同じ物理押下を
未消費とみなし二重に使ってしまう」と明言）で、`OUTPUT_GATE` active 中に
発火し `deferred_engine_timers`（`message_handlers.rs:600-601` で push、
`:1403` で `std::mem::take`、`:1407` で `app.build_ctx()` を呼び全 deferred
timer の replay に使い回す）へ退避されたタイマーが gate 解除後に replay
されると、`phys` はその時点のライブ値になる。この値が **タイマーが
本来対象としていた押下ではなく、replay 実行時点でたまたま押されている
別の押下**だった場合、`right_thumb_consumed`/`left_thumb_consumed` に
その無関係な押下が刻印される。

症状は本件の**鏡像**になる: 本件（キー経路）は消費済みの押下が「未消費」
と誤判定され余計な同時打鍵が成立した（う→ゔ）。タイマー経路は逆に、
未消費の新しい押下が「消費済み」と誤って刻印され、**本来成立すべき
同時打鍵が失われる**（例: 次に来る文字キーが親指シフト面ではなく無シフト
面で出てしまう）。

タイマーには実は capture 点が存在しないわけではない——`PendingThumbData`
は対象押下の `timestamp` を既に保持しており（`timeout_pending_thumb` が
これを使う）、`deferred_engine_timers` に `(timer_id, wparam)` だけでなく
defer 時点の親指スナップショットを一緒に積めば、キー経路と同じ形の解法が
適用できる。「自明ではない」のではなく「適用可能だが本 ADR のスコープ外」
というのが正確な位置づけであり、実装は別 ADR に切り出す（未決定事項参照）。

## テスト（未実施、実装時に必須）

`fix-requires-evidence.md` のキー選択/warmup ファミリーに該当するため、
(a) 回帰テストまたは (b) known-bugs.md 記録の少なくとも一方が必須。

**第一候補（唯一の機械可読な回帰防止、実装と同じ PR で追加）:**
`crates/awase-windows/tests/architecture_guard.rs` に grep ベースのガードを
追加する。このファイルは Linux で実行でき（`fs::read_to_string` による
ソース文字列走査で、型としては正しいが意味的な配線間違いを検知する既存の
仕組み）、既に `src/runtime/key_pipeline.rs` を走査対象に列挙している
（`:178`, `:455`）。「`hook::thumb_down_timestamps()` の呼び出し許可箇所は
`runtime/mod.rs::build_ctx` と `message_handlers.rs` のタイマー経路のみで、
`runtime/key_pipeline.rs` には出現しないこと」を assert する。ファイル冒頭
の doc（`:21-22`）が「将来的に正当な理由で許可数が増える場合はこのファイル
の定数を更新すること」とまさにこの運用を想定している。将来誰かが
`key_pipeline.rs` へライブクエリを戻した瞬間に CI で落ちる。

**却下（回帰ガードとして機能しない）:** `src/engine/tests.rs` に
`NicolaFsm::on_event(event, phys)` へ古い/新しい2つの `phys` を渡す
単体テストを足す案は、**修正の前後どちらでも通ってしまう**ため不採用。
理由: `on_event` は `phys` を `event` とは別引数で受け取り、採用案は
この署名を変えない。core 側のテストは「`phys` に何を渡したら何が起きるか」
（＝ [ADR-010](010-thumb-consumption-timestamp.md) の比較ロジック、既に
健全と確認済み）しか検証できず、バグの実体である「誰がその `phys` を
作るか」＝ `key_pipeline.rs:105` は `#[cfg(windows)]` で core テストから
見えない。

**副次（Windows 専用、`windows-build` CI 任せ）:** `build_raw_key_event`
が `left_thumb_down_snapshot`/`right_thumb_down_snapshot` を正しく埋める
ことを検証する Windows 専用テストを追加してもよい
（`cargo check --target x86_64-pc-windows-msvc -p awase-windows --tests --lib`
で確認、実行は CI に委ねる）。回帰ガードとしての主力は上記
`architecture_guard.rs` 側であることに変わりはない。

known-bugs.md への BUG-N 起票は本 ADR の decision 確定後に行う。

## 未決定事項

1. ~~`RawKeyEvent` の新フィールド命名~~ — **解決済み（本版）**。2フィールド案
   （`left_thumb_down_snapshot`/`right_thumb_down_snapshot`）を採用。
2. **タイマー経路（`deferred_engine_timers`、実体は `build_ctx()` 経由の
   ライブクエリ、`message_handlers.rs:600-601`/`:1403-1407` および
   `runtime/mod.rs:294`）の修正**: 「限界」節で失敗モードは特定済みだが、
   `PendingThumbData.timestamp` を使った defer 時点スナップショット方式の
   実装は本 ADR のスコープ外とし、別 ADR に切り出す（実機での再発、または
   本 ADR 実装時の判断による）。
3. **doc comment の更新**: `src/types.rs::RawKeyEvent::modifier_snapshot`
   の doc comment は `OUTPUT_PENDING_QUEUE` とのみ書いているが、実際に
   本件・本 ADR が扱う経路は `INPUT_DEFER` である。修正実装時に doc
   comment 側にも `INPUT_DEFER` への言及を追加する。

## 関連

report `01M1N36MGDDJ5HN8FWRE4ZHS3J`（本 ADR の起票根拠）、
[ADR-010](010-thumb-consumption-timestamp.md)（`Option<Timestamp>` による
親指消費追跡、比較ロジック自体は本件で健全と確認済み）、
[ADR-008](008-physical-thumb-state-separation.md)（物理親指キー状態と FSM
解決ロジックの分離、本 ADR が踏襲する層分離の前例）、BUG-105
（3鍵仲裁の別欠陥、本件と症状が類似するが原因は別、`docs/known-bugs.md`）、
`docs/bug-reports-triage.md`（本 report の一次調査記録）。
