# ADR-129: OUTPUT_GATE drain replay 中、親指キー押下タイムスタンプがイベント捕捉時点ではなくリプレイ実行時点のライブ値で再構築され、既に消費済みの押下と無関係な後続押下がペアリングされる

## ステータス

**設計確定・実装未着手。** opus-adversarial-consult を通算3ラウンド実施し
（round1: 2026-09-04 の v1→v2、round2: 同日の v2 確認、round3: 2026-09-08 の
案B対案検討）、いずれも収束済み。
report `01M1N36MGDDJ5HN8FWRE4ZHS3J`（2026-09-04、タスクトレイ「不具合を報告」
機能、[ADR-095](095-tray-bug-report-cloudflare-intake.md)）から起票。
`docs/bug-reports-triage.md` に一次調査結果を記録済み。

**BUG 番号**: `docs/known-bugs.md` の空き番は **BUG-127**（BUG-126 まで採番済みで
あることを 2026-09-08 に確認）。ただし確認時点の develop 作業ツリーには未コミットの
`known-bugs.md` 差分が存在したため、**実装ブランチを切る直前に develop の最新
状態で空き番を再確認すること**（`feedback_bug_number_collision_on_branch_merge`
＝並行ブランチが同じ番号を独立に採番する事故の再発防止）。

**round1（v1→v2、2026-09-04）:** 根本原因の特定は正しく、一次証拠から独立に
再構成した結果、当初案より**強く**証明できることが判明した一方（証拠を推測から
観測へ格上げ）、以下の欠陥が見つかり v2 で修正した: 初版のイベント区分の誤り
（手順1は実はライブ配送ではなく drain replay だった。ただし delta が小さく
無害だっただけ）、未決定事項2・3の自己矛盾（`build_ctx()` は実は drain replay
経路そのものだった）、テスト節が実質的に何も検証しない（修正前後どちらでも
通る）、修正が問題の半分（キーイベント経路）しか閉じずタイマー経路に具体的な
失敗モードが残ることの過小評価、検討していなかった第4の代案とその却下理由。

**round2（v2確認、2026-09-04）:** 行番号・ログ行・数値を実ファイルで再検証した
結果、「指摘1〜13すべて反映済み、ブロッカーなし」との判定。軽微な修正6件
（`is_thumb_consumed` の行番号誤記、限界節がタイマー経路2箇所の危険度を一括りに
していた点、`Idle` からの即時解決経路が `ActiveThumb` 以外に `ShiftPlane` も
あり得ることの排除根拠不足、却下案(c)の記述と訂正後の手順1の噛み合わせ、決定
ステップ2の「直後」という要件の過剰な厳格化、構築サイト列挙でコンストラクタ
関数をテスト用途と一括りにしていた点）を反映済み。

**round3（案B対案検討、2026-09-08、architect / critic 各2ラウンド）:**
[ADR-155](155-timer-path-live-thumb-requery-during-deferred-timer-replay.md)
の調査中に発見された「案B」（`PendingThumbData` を親指押下時刻の出所とする
代替案）を検討し **不採用**。round1 critic は Blocker ゼロで案A 続行を支持。
round2 architect が6件の Should-fix（構築サイトの列挙漏れと検証コマンドの穴、
グローバルゼロクリア3経路との整合性、T1系/T2系の混同防止 doc、案(d) 却下理由が
Linux 本番実装について不正確だった点、`architecture_guard` に正のガードが
無い点、`[engine-input]` の診断不足）を反映して最終版を作成した。round2 critic
は Opus の週次利用上限により完了しなかったため、architect が特に確認を求めた
2点（グローバルゼロクリアが `SuppressionEdge::Leave` 経由でも起きるか、検証
コマンド2本が全28構築サイトを覆うか）は担当セッションが直接コード確認で代替
検証し、いずれも決定を覆す新規 Blocker は見つからなかった
（`hook.rs::clear_hook_latches_for_app_disable` は `Enter`/`Leave` 両方で
無条件にグローバルを0クリアするが、これは「決定」節 (4) が既に検討した
3経路と同型で新種の失敗モードではない。検証コマンドの cfg 境界の切り分けは
`lib.rs` の `mod` 宣言を実測し、`tsf` モジュール自体は非ゲートでもサブ
モジュール `tsf_gate` 個別に `#[cfg(windows)]` が掛かっている点を除き
architect の分析どおりと確認した）。
**行番号は 2026-09-08 時点の develop で全面的に再実測済み。** v2 執筆時から
大きくずれていた（主なもの:
`key_pipeline.rs:105 → :291`、
`hook.rs::thumb_down_timestamps :519 → :532`、
`hook.rs` グローバル `:190-191 → :201-202`、
`update_thumb` クロージャ `:1046-1062 → :1153-1170`（内部 `now_timestamp()` は `:1158`）、
`build_raw_key_event` 呼び出し `:1092 → :1200`・構造体リテラル `:772 → :842`、
`types.rs::modifier_snapshot` doc `:206-211` / フィールド `:212 → :251-255` / `:256`、
`nicola_fsm.rs::active_thumb_side :2790 → :3091`、
`is_thumb_consumed :2780 → :3081`、
`NicolaFsm::phys :194`（現行でも正しい。なお `:196-203` は
`left_thumb_consumed`/`right_thumb_consumed` の doc とフィールド定義であって
`phys` ではない）。

## 問題

`crates/awase-windows/src/runtime/key_pipeline.rs:291` は、`Engine::on_input()`
に渡す `InputContext` を組み立てる直前に

```rust
let (left_thumb_down, right_thumb_down) = hook::thumb_down_timestamps();
```

を呼ぶ。`hook::thumb_down_timestamps()`（`hook.rs:532`）は、`WH_KEYBOARD_LL`
フックコールバック自身が実時間で更新するグローバル `AtomicU64`
（`LEFT_THUMB_DOWN_AT_US` / `RIGHT_THUMB_DOWN_AT_US`、`hook.rs:201-202`,
`update_thumb` クロージャ `:1153-1170`）を、呼び出された
**その瞬間の値**で読む。（行番号は 2026-09-08 時点の develop で再実測済み。
初版執筆時からのずれの詳細はステータス節の対応表を参照。）

この関数を呼ぶ `kp_run_inner`（`key_pipeline.rs:225`）は、フックからの
ライブ配送と、`OUTPUT_GATE` が active な間 `INPUT_DEFER` に退避された
イベントの drain replay（`message_handlers.rs::handle_wm_drain_output_queue`
（`WM_DRAIN_OUTPUT_QUEUE` ハンドラ）→ `deliver_key_event(app, *queued_event,
KeyOrigin::DeferredReplay)`（`message_handlers.rs:1369`）→
`app.process_key_event`（`:227`）→ `key_pipeline.rs:34` → `kp_run_inner`）の
**両方**から同一コードパスで呼ばれる。別経路は存在しない
（`ImeOffRescueReplay` も `replay_ime_off_rescue_event` → 同じ
`kp_run_inner(event, true)` を通り、`:291` を通過する）。

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
   （`:3091`）が `Some` を返す場合の `reduce_active_thumb`）の2つ
   だが、`mods(c=false s=false a=false w=false)`（上記ログ行）で `Shift`
   非押下が確定しており `should_use_shift_plane` は成立しないため
   `ShiftPlane` は排除できる。よって残る即時解決経路は `ActiveThumb` のみ
   ——それ以外（未消費の親指なし）なら `A` も `C` と同様 `PendingChar` を
   作るはずである
   （`confirm_mode` は未指定=`idle_wait`、`Timer set: logical=1, ms=100`
   が `simultaneous_threshold_ms=100` と一致することからも `PendingChar`
   経路が使われていることが裏付けられる）。A↓ が `PendingChar` を作らな
   かった以上、`active_thumb_side()` は `Some` を返した。`is_thumb_consumed`
   （`:3081-3088`）の比較 `right_thumb_consumed(=8453961165) == phys.right_thumb_down`
   が不一致だったということは、`phys.right_thumb_down` は `8453961165`
   ではない別の値であり、この時間窓でその Atomic が取りうる値は手順4の
   2回目の押下 `8454313529` しかない。よって「ライブクエリが
   `Some(8454313529)` を返した」は観測から演繹できる事実であり、推測
   （反実仮想）ではない。
8. **なぜ `C↓` は同じライブ値 `8454313529` を見ながら親指シフトされ
   なかったか**: `A` の解決（`reduce_active_thumb` → `consume_thumb`、
   `nicola_fsm.rs:1198-1205`、代入行は `:1200`/`:1201`）が `right_thumb_consumed =
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

`RawKeyEvent::modifier_snapshot`（`src/types.rs:251-255`、フィールド本体
`:256`）は、**同じクラスの問題を Ctrl/Shift/Alt/Win について既に部分的に
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

`hook.rs` は `read_os_modifiers()`（`:1189`、以下 `LLKHF_ALTDOWN`/
alt-なりすまし補正を挟んで `build_raw_key_event` 呼び出しは `:1200`）を
呼び、`RawKeyEvent` に埋め込んで `INPUT_DEFER`/replay を素通りさせている。
`update_thumb`（`:1153-1170`）は `:1189` より**前**にあり、順序関係は
成立している。

**ただしこの前例自体、完了した修正ではなく部分修正である。**
`Runtime::build_ctx`（`runtime/mod.rs:344`）は `ctx.modifiers` を
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
   `InputContext`/`build_input_context`（`runtime/mod.rs:85-103`）も唯一の
   消費点 `key_pipeline.rs:291-300` も既に左右を独立した2引数で扱って
   おり、タプル化すると `.0`/`.1` のどちらが左右か呼び出し側で不明瞭に
   なるだけで実利がない。）
   `RawKeyEvent` は `Copy` で `hook_channel.rs` のリングバッファと
   `INPUT_DEFER` に値渡しで積まれるため、追加はサイズ増以外の影響がない。
   構築サイトの正確な一覧（本番2箇所＋テスト26箇所＝計28、2026-09-08 実測）は
   「案B検討結果」節末尾の一覧表を参照——初版が挙げていた「本番1箇所・
   コンストラクタ1箇所・テスト10箇所」は develop の変化で大きくズレていた。
2. `hook.rs` の `build_raw_key_event` 呼び出し（`:1200`）に渡す引数として、
   `modifier_snapshot` 構築（`:1189`）と同じ場所で `thumb_down_timestamps()`
   を呼び、`RawKeyEvent` に埋め込む。要件は「`update_thumb`
   （`:1153-1170`）より**後**であること」で足り、`:1189` の時点で満たす
   ——`:1170` の直後に押し込む必要はなく、間に挟まる `classify_key`
   や alt なりすまし補正との順序を気にする必要もない。当該キー自身の
   ↓/↑ による親指状態の変化を反映した値を capture することで、
   「このキー自身が親指キーだった場合」も含めて正しい値になる。
3. `key_pipeline.rs:291` の `hook::thumb_down_timestamps()` ライブ呼び出しを
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

### 案B検討結果（opus-adversarial-consult 2026-09-08、architect/critic 2ラウンドで収束）

実装着手前に、案A（本節冒頭の採用案＝capture-time スナップショット）に対する
対案として、**案B（`PendingThumbData.timestamp` を親指押下時刻の真実の出所と
みなし、`ctx.*_thumb_down` への依存自体を減らす）** を検討した。**結論: 案B は
実装しない。案A を予定どおり実装する。**

#### 案B が代替にならない理由（2点、いずれも実コード確認済み）

1. **案B を実装しても `key_pipeline.rs:291` のライブクエリは残り、本 ADR が
   起票された `A↓` replay 事故そのものは直らない。** 事故の発生経路である
   `Idle` からの `active_thumb_side()`（`nicola_fsm.rs:3091`）は
   `self.phys`（`nicola_fsm.rs:194`）だけを見ており、`PendingThumbData` を
   一切参照しない。`Idle` 状態では定義上 `PendingThumbData` が存在しない
   （存在すれば `PendingThumb` 状態になっている）ため、案B が整備する経路は
   本件の失敗経路と交差しない。
2. **`PendingThumbData.timestamp` は `phys.*_thumb_down` と時系列が別物であり、
   等値比較の入力に流用できない。** 前者は `build_raw_key_event`
   （`hook.rs:831-859`）内の `now_timestamp()`（＝ `RawKeyEvent.timestamp`、
   以下 **T2系**）由来、後者は `update_thumb` クロージャ内の
   `now_timestamp()`（`hook.rs:1158`、以下 **T1系**）由来で、同一の物理押下に
   対しても数 µs ずれる。両者が等値比較に使われていないことは確認済み:
   `*_thumb_consumed` への書き込みは全リポジトリで4箇所
   （`nicola_fsm.rs:1200`/`:1201`/`:2779`/`:2780`）のみで、**すべて右辺が
   `self.phys.*_thumb_down`（T1系）** である。案A 適用後もこの不変条件は
   保たれる（案A は T1系の値を capture 時点で運ぶだけで、系を混ぜない）。

#### 案A 実装時に守るべき追加要件（本レビューで判明した6点）

**(1) 新フィールドの doc に「T1系である」ことを明記する（系の混同防止）**

`left_thumb_down_snapshot` / `right_thumb_down_snapshot` の doc comment に、
次の趣旨を必ず書く:

> この値は **T1系**（`hook.rs` の `update_thumb` クロージャ内 `now_timestamp()`、
> `hook.rs:1158`）由来である。同一構造体の `timestamp` フィールドは **T2系**
> （`build_raw_key_event` 内の `now_timestamp()`、`hook.rs:852`）由来で、同じ
> 物理押下でも数 µs ずれる。**両者を減算・比較してはならない。** 本フィールドの
> 用途は `InputContext.left/right_thumb_down` への供給（＝ `NicolaFsm::phys`
> への供給）のみであり、そこでの比較相手は同じく T1系の `*_thumb_consumed`
> （`nicola_fsm.rs:1200/1201/2779/2780`）である。

**(2) 却下案(d)の記述を「Windows 経路に限る」と正確化する**

v2 の「案(d) は実装済みだが本番未使用」という記述は Windows 限定でしか正しく
ない。**`crates/awase-linux/src/main.rs:145-175` は本番コードで案(d) 相当を現に
インラインで実装しており（`event.key_classification` の `LeftThumb`/`RightThumb`
から `left_thumb_down`/`right_thumb_down` を導出、auto-repeat 対策の
`.or(Some(..))` セマンティクスまで Windows の `update_thumb` に合わせてある）、
Linux 本番ではこれが唯一の供給元である。** よって却下の主張は
「**Windows 経路に限って**案(d) を採らない。理由は `deliver_key_event`
（`message_handlers.rs`）の5つの早期 return によって親指キーの ↑ が握り潰され
うるため」と限定して書く。Linux 側は `deliver_key_event` 相当の早期 return 網を
持たないため案(d) が成立している、という差が却下理由の実体である。

**(3) Linux 側の構築サイト（`crates/awase-linux/src/hook.rs:261`）の扱いを決める**

新フィールドには **`None` を固定で入れる**（Linux は上記のとおり `main.rs` 側で
独自に導出しており、`RawKeyEvent` 経由でスナップショットを運ぶ必要がない）。
ただし次のリスクを doc かコメントで残すこと: **将来 core（`src/`）側がこの
フィールドを消費するコードを追加した瞬間、Linux は `None` 固定のまま静かに
壊れる**（コンパイルは通り、テストも落ちない）。core がこのフィールドを読む
コードを足す際は、必ず `crates/awase-linux/src/hook.rs:261` と
`crates/awase-linux/src/main.rs:145-175` を同時に見直す。

**(4) グローバルゼロクリア3経路との整合性（判断: 有界なので許容する）**

`LEFT_THUMB_DOWN_AT_US` / `RIGHT_THUMB_DOWN_AT_US`（`hook.rs:201-202`）を
無条件に 0 クリアする経路は3つある:

| 位置 | 関数 | 契機 |
|---|---|---|
| `hook.rs:346-347` | `reset_physical_key_state()`（`:339`） | `panic_reset`（`runtime/mod.rs:2078`）、`WM_WTSSESSION_CHANGE` の `WTS_SESSION_UNLOCK`（`message_handlers.rs:1094`） |
| `hook.rs:394-395` | `clear_hook_latches_for_app_disable()`（`:379`） | 無効アプリ出入りの `SuppressionEdge::Enter` / `Leave`（`state/app_suppression.rs:59`） |
| `hook.rs:526-527` | `set_thumb_vk_codes()`（`:521`） | config リロードによる親指 VK の再設定 |

案A 適用後、**クリア前に capture され `INPUT_DEFER`（`input_defer.rs:23`）や
hook リング（`hook_channel.rs`）に残っているイベントは、クリア後も stale な
`Some(T1)` スナップショットを運び続ける。** 上記3経路のいずれも `INPUT_DEFER`
を空にしないことは実コードで確認済み（`INPUT_DEFER` を空にするのは
`message_handlers.rs:1886` の `take_all()` ＝正規の drain のみで、`panic_reset`
も `clear_hook_latches_for_app_disable` も触らない）。これは本 ADR が却下した
案(d) の欠陥（無期限のスティッキー親指）の **有界版** にあたる。

**round2 critic による追加確認（M1）: 上表の `Leave` は実質的に無害であり、
実在するのは `Enter` エッジのみである。** `hook.rs:996` の
`FOCUS_APP_DISABLED` 早期 return
（`if FOCUS_APP_DISABLED.load(Ordering::Relaxed) { return
CallNextHookEx(...); }`）が、`update_thumb`（`:1153`）と
`build_raw_key_event`（`:1200`）の**どちらよりも前**にある。したがって
無効アプリ滞在中は `RawKeyEvent` が1件も構築されず（hook リング・
`INPUT_DEFER` への蓄積がゼロ）、`LEFT/RIGHT_THUMB_DOWN_AT_US` も更新
されない——「無効アプリ側で押された親指の T1」という値自体が存在しない。
`runtime/focus_tracking.rs:517` の `invalidate_engine_context` が `Enter`
のみで呼ばれる（`Leave` では呼ばれない）ことも確認したが、`Leave` 側は
そもそも上記の理由で無害なため、この非対称自体は問題にならない。
stale スナップショットが `INPUT_DEFER` に居残りうるのは、`OUTPUT_GATE`
active のまま無効アプリへ**入る**（`Enter`）ケースだけである。

**判断: 許容する。クリア時に `INPUT_DEFER` を破棄する対処は行わない。**
理由は3つ。

1. **スティッキー化しない（有界性は構造的に保証される）。**
   案(d) の却下理由は「親指キーの ↑ が早期 return で握り潰されると engine が
   親指押下を**無期限に**信じ続ける」ことだった。案A ではこれが起きない:
   クリア**後**に capture される全イベントは `None`（またはクリア後の新しい
   押下の値）を運び、`NicolaFsm::phys`（`nicola_fsm.rs:194`）は `on_event` /
   `on_timeout` で毎回丸ごと上書きされる（「限界」節参照）。したがって
   **キューに残っていた分が drain され切った次の1イベントで、`phys` は自動的に
   クリア後の正しい値へ戻る。** 影響範囲は「クリア時点で既にキューに居た
   イベント」に厳密に限定され、上限は OUTPUT_GATE の active 窓（本 ADR の実測
   タイムラインで ~300ms 規模）である。案(d) の「無期限」とは質的に異なる。
2. **そのイベントにとっては `Some(T1)` のほうが意味的に正しい。**
   クリアの目的は「過去の状態を**未来へ**漏らさない」ことだが、キューに残って
   いるイベントは未来ではなく**過去**である。`T1` はそのイベントが物理的に
   起きた時点の真の親指状態であり、比較相手の `*_thumb_consumed`
   （`nicola_fsm.rs:1200/1201/2779/2780`、すべて右辺が `phys.*_thumb_down`）も
   同じ T1系・同じ時代の値なので、等値比較は同一エポック内で閉じている。
   とくに `set_thumb_vk_codes`（config リロード）では、キュー内イベントの
   `key_classification` も旧 config で分類済み（`hook.rs:1187` の
   `classify_key(vk, scan, &config)`）であるため、旧 config 時点の親指
   スナップショットを運ぶほうが **イベント内部の一貫性が保たれる**。これは
   本 ADR の中核原則（capture 時点の文脈でイベントを再処理する）そのものであり、
   ここだけ原則を折るほうがむしろ不整合になる。
3. **`INPUT_DEFER` の破棄はユーザーの実打鍵を失う。**
   `replay_later`（`message_handlers.rs:1933`、`runtime/mod.rs:839`）が存在する
   のは「打鍵を落とさない」ためであり、上記3経路は現状 `INPUT_DEFER` の所有者
   ですらない。クリア時に破棄を足すのは本 ADR と無関係な挙動変更（打鍵消失と
   いう新種の regression）を持ち込むことになり、blast radius が釣り合わない。
   本 ADR は「1文字の誤変換」を直すためのものであって、それより重い「打鍵が
   消える」を新設してはならない。

**round2 critic による追加確認（M2）: 本判断は現行挙動の追認ではなく
挙動変更である。** 現行（案A 適用前）は `key_pipeline.rs:291` がクリア
**後**にライブクエリするため、キューに残っていたイベントは `None`
（＝親指なし）で replay される。案A 適用後は、同じイベントが `Some(T1)`
を運ぶ。実機で差が出た場合、テスト節 (a-4) で追加する `l_thumb=`/
`r_thumb=` ログが、クリア直後の drain バーストで `Some(...)` を出し続ける
ことが「これは案A が意図的に導入した差分か、それとも別の回帰か」を
即座に切り分ける識別子になる。

**万一実機で顕在化した場合の設計を、再調査を避けるためここに先置きする
（今は実装しない）:** エポック方式——`THUMB_EPOCH: AtomicU64` を上記3経路で
インクリメントし、capture 時に `RawKeyEvent` へエポックも埋め、
`key_pipeline.rs` で読むときに現在エポックと不一致なら `None` に落とす。
打鍵を失わずスナップショットだけを無効化できる。**今これを実装しないのは、
未観測の失敗に対する先回り実装であり、`.claude/rules/tuning-constants.md` が
戒める「実測なしのエスカレーション」と同型のコストを払うことになるため。**
症状が出たときの手掛かりは、テスト節 (a-4) で追加する `[engine-input]`
ログの `l_thumb=` / `r_thumb=` フィールドが、クリア直後の drain バーストで
`Some(...)` を出し続けることである。

**(5) `runtime/mod.rs:1428` のライブ経路について**

`Runtime::process_deferred_keys`（`runtime/mod.rs:1371`）は
`for (event, _phys) in keys` で **保存済みの `_phys` を捨てて** `self.build_ctx()`
（`runtime/mod.rs:344` でライブクエリ）を呼び直している。ただし現状この経路は
**死コード**である: `keys` の供給元 `SyncKeyGate::deactivate()` が非空を返すには
`SyncKeyGate::activate()` / `try_push()` が呼ばれている必要があるが、両者の
呼び出し元は `crates/awase-windows/src/state/hook_state.rs` の外にゼロ
（`is_active()` / `has_deferred_keys()` は `message_handlers.rs:481-482` から
読まれているが、常に false を返す）。したがって本 ADR の実装では**この経路に手を
入れない**。「保存済みの `_phys` を捨ててライブ値を作り直している」という構造の
異常さは、この経路が将来復活する際の地雷として本節に記録するに留める。

**(6) コスト見積りの訂正**

`RawKeyEvent` への `Option<Timestamp>` × 2 の追加コストは「1イベントあたり
+32 bytes」だけではない。`crates/awase-windows/src/hook_channel.rs` の static
リングバッファ（`CAP = 1024`、`hook_channel.rs:13`）が **+32KB** される。実害は
ないが、同ファイル冒頭のコメント（`hook_channel.rs:10`「`RawKeyEvent` は Copy な
POD (数十バイト程度) のため、」）は本変更で古くなるため、実装 PR で更新する。

#### `RawKeyEvent` 構築サイト一覧（2026-09-08 実測、`grep -rn "RawKeyEvent {"`）

`RawKeyEvent` は `Default` を実装しておらず `..Default::default()` が使えない
ため、**全28サイトに `left_thumb_down_snapshot` / `right_thumb_down_snapshot` の
初期化を機械的に追加する必要がある**（本番 Windows のみ実値、他は `None`）。

**本番（2箇所）:**

| 位置 | 埋める値 |
|---|---|
| `crates/awase-windows/src/hook.rs:842`（`build_raw_key_event` 内の構造体リテラル。関数定義は `:831-859`） | `thumb_down_timestamps()`（`hook.rs:532`）の戻り値（決定ステップ2） |
| `crates/awase-linux/src/hook.rs:261` | `None` 固定（上記 (3) 参照） |

**テスト／ダミー用途（26箇所）:** すべて `None, None` でよい。

*ルート `awase` クレート（15箇所）*

| 位置 | 備考 |
|---|---|
| `src/types.rs:387` | `mod tests`（`:358`）配下の `resync_probe_event`。**v2 が「コンストラクタ1箇所、本番コードから呼ばれうるため『テスト用途』に一括りにしない」と書いていたのは誤り**——現行は cfg(test) 配下のテストヘルパーであり、本番からは呼ばれない |
| `src/engine/fsm_adapter.rs:391` | `mod tests`（`:344`）配下 |
| `src/engine/fsm_types.rs:728` | `mod tests`（`:657`）配下 |
| `src/engine/input_tracker.rs:215` | `mod tests`（`:210`）配下 |
| `src/engine/key_lifecycle.rs:115` | `mod tests`（`:110`）配下 |
| `src/engine/nicola_fsm.rs:3912` | `mod tests`（`:3354`）配下 |
| `src/engine/proptest_tests.rs:215` | ファイル自体がテスト専用モジュール |
| `src/engine/tests.rs:183` | `TestEventBuilder::build()` |
| `src/engine/tests.rs:847` | |
| `src/engine/tests.rs:892` | |
| `src/engine/tests.rs:1124` | `enter_thumb_down_event` |
| `src/engine/tests.rs:3004` | |
| `src/engine/tests.rs:3037` | |
| `src/engine/tests.rs:3052` | |
| `tests/scenarios.rs:124` | **v2 の列挙から漏れていた**（`key_down` ヘルパー。`key_up`（`:139`）は `key_down` に委譲するので構築サイトは1つ） |

*`awase-windows` クレート（11箇所）*

| 位置 | Linux ホストで見えるか |
|---|---|
| `crates/awase-windows/src/hook_channel.rs:239` | **見える**（`lib.rs:28` の `pub mod hook_channel;` は cfg 非ゲート） |
| `crates/awase-windows/src/state/evidence.rs:535` | **見える**（`lib.rs:35` の `pub mod state;` は cfg 非ゲート） |
| `crates/awase-windows/src/state/platform_state.rs:2269` | **見える**（同上） |
| `crates/awase-windows/src/input_defer.rs:128` | 見えない（`lib.rs:55` が `#[cfg(windows)]`） |
| `crates/awase-windows/src/runtime/transport.rs:436` | 見えない（`lib.rs:76` が `#[cfg(windows)]`） |
| `crates/awase-windows/src/runtime/transport.rs:463` | 見えない |
| `crates/awase-windows/src/runtime/transport.rs:496` | 見えない |
| `crates/awase-windows/src/runtime/transport.rs:653` | 見えない |
| `crates/awase-windows/src/tsf/tsf_gate.rs:657` | 見えない（`mod tests`（`:380`）配下。`tsf` 親モジュール自体は非ゲートだが、`tsf_gate` 等の個別サブモジュールに `#[cfg(windows)]` が掛かっている——`focus/mod.rs` と同型のパターン） |
| `crates/awase-windows/tests/e2e_windows.rs:136` | 見えない（`#![cfg(windows)]`） |
| `crates/awase-windows/tests/e2e_windows.rs:154` | 見えない（同上） |

（本番の `crates/awase-windows/src/hook.rs:842` も `lib.rs:45` の `#[cfg(windows)]`
配下なので Linux ホストでは見えない。上表には本番として既出のため再掲しない。）

**検証コマンド（2本必要。片方だけでは全サイトを網羅できない）:**

```sh
# (A) ホスト(Linux): ルートクレート全体 + tests/scenarios.rs + awase-linux
#     + awase-windows の cfg(windows) 外モジュール（hook_channel / state/）
cargo check --workspace --all-targets

# (B) Windows ターゲット: awase-windows の cfg(windows) 配下（hook.rs / input_defer.rs
#     / runtime/ / tsf/）と crates/awase-windows/tests/*
cargo check --target x86_64-pc-windows-msvc -p awase -p awase-windows --lib --tests
```

`--all-targets` なしの workspace check では `tests/scenarios.rs` も
`crates/awase-windows/tests/*` もコンパイルされない（v2 の検証手順の穴）。
逆に (A) だけでは上表「見えない8箇所」を落とす。(B) はリンクを伴わないため、
`link.exe` が無い開発サンドボックスでも実行できる（CLAUDE.md「Commands」節参照）。

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
  **無害**。有害なのは gate 解除後に `build_ctx()`（`runtime/mod.rs:344`、
  `message_handlers.rs:1407` から deferred timer replay 用に呼ばれる、
  delta 大）経由でライブクエリされるケースのみ

という**2種類の provenance が同じフィールドに交互に書き込まれる**。しかも
`consume_thumb`（`:1198-1205`、代入行 `:1200`/`:1201`）と
`resolve_char_and_thumb_as_separate_solos`（代入行 `:2779`/`:2780`、
`ThumbSide::Right => self.right_thumb_consumed =
self.phys.right_thumb_down`）は**その `phys` から `*_thumb_consumed` を
書く**ため、2種類の provenance が `is_thumb_consumed` の等値比較
（`nicola_fsm.rs:3081-3088`、`phys_down.is_some() && consumed == phys_down`）
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

タイマーには実は capture 点が存在しないわけではない——`PendingThumbData` は
対象押下の `timestamp` を既に保持しており（`timeout_pending_thumb` がこれを
使う）、`deferred_engine_timers` に `(timer_id, wparam)` だけでなく defer 時点の
親指スナップショットを一緒に積めば、キー経路と同じ形の解法が適用できる。
「自明ではない」のではなく「適用可能だが本 ADR のスコープ外」というのが正確な
位置づけである。**この切り出しは
[ADR-155](155-timer-path-live-thumb-requery-during-deferred-timer-replay.md)
として実際に起票されたが、具体的な失敗シナリオを構成できず 2026-09-08 に
「実装せず」でクローズされた（経緯は `docs/known-bugs.md` BUG-126）。よって
タイマー経路のライブクエリは本 ADR 実装後も既知の未修正の穴として残り、本節の
記述は「将来の別 ADR で直す」ではなく「一度検討してクローズ済み、実機での
再発待ち」と読むこと。**

## テスト（未実施、実装時に必須）

`.claude/rules/fix-requires-evidence.md` のキー選択 / warmup ファミリーに該当する
ため、(a) 回帰テストまたは (b) `docs/known-bugs.md` 記録の少なくとも一方が必須。
本 ADR は **両方** を行う（(a) は下記、(b) は BUG 番号を採番して起票）。

### (a-1) 負のガード: ライブクエリが `key_pipeline.rs` に戻らないこと

`crates/awase-windows/tests/architecture_guard.rs` に追加する。このファイルは
Linux で実行でき（`fs::read_to_string` によるソース文字列走査で、型としては
正しいが意味的な配線間違いを検知する既存の仕組み）、既に
`src/runtime/key_pipeline.rs` を走査対象に列挙している。

`hook::thumb_down_timestamps()` の呼び出し許可箇所を
**`runtime/mod.rs:344`（`build_ctx`）と `runtime/message_handlers.rs:723`
（タイマー経路）の2箇所のみ**に固定し、`runtime/key_pipeline.rs` には出現
しないことを assert する（定義そのものである `hook.rs:532` は除外）。
ファイル冒頭の doc（`:21-22`）が「将来的に正当な理由で許可数が増える場合は
このファイルの定数を更新すること」とまさにこの運用を想定している。将来誰かが
`key_pipeline.rs` へライブクエリを戻した瞬間に CI で落ちる。

### (a-2) 正のガード: スナップショットが実際に配線されていること【新規・必須】

**負のガードだけでは不十分である。** 「ライブクエリを消したが `None, None` を
渡している」状態を通してしまうためで、既存の
`build_input_context_callers_do_not_drop_thumb_down_state`
（`architecture_guard.rs:168`）も `,None,None,)` / `,None,None)` という
リテラルパターンしか見ないためこれを検出できない。よって次の2つを追加する。

- **(a-2-i)** `key_pipeline.rs` のソース全体（空白除去せず生テキストでよい）に
  `event.left_thumb_down_snapshot` と `event.right_thumb_down_snapshot` の
  両方が出現することを直接 assert する。**`build_input_context(` への引数
  文字列を厳密に一致させる形にはしない**——実装は現状の
  `let (left_thumb_down, right_thumb_down) = hook::thumb_down_timestamps();`
  という**ローカル束縛を経由する形**（`:291-300`）を素直に書き換えると
  `let (left_thumb_down, right_thumb_down) =
  (event.left_thumb_down_snapshot, event.right_thumb_down_snapshot);` に
  なり、`build_input_context(` の引数リストには依然として
  `left_thumb_down,right_thumb_down,` しか現れない。これは意味的に完全に
  正しい実装なので、引数の字面を固定するテストは「正しい実装を弾く」形に
  なってしまう（opus-adversarial-consult round2 critic 指摘、S1）。守りたい
  不変条件（ライブクエリを削除したうえで新フィールドを実際に読んでいる）は
  出現有無の確認で十分固定できる。
- **(a-2-ii)** **`key_pipeline.rs` に `build_ctx(` が出現しないこと**も固定する。
  将来ここで `build_ctx()` を呼べば `runtime/mod.rs:344` 経由でライブクエリが
  **間接的に**復活しうるが、(a-1) の文字列ガードは
  `hook::thumb_down_timestamps` という文字列を探しているだけなのでそれを
  検知できない。この穴を明示的に塞ぐ。

### (a-3) 副次（Windows 専用、`windows-build` CI 任せ）

`build_raw_key_event`（`hook.rs:831`）が `left_thumb_down_snapshot` /
`right_thumb_down_snapshot` を正しく埋めることを検証する Windows 専用テストを
追加してもよい。`cargo check --target x86_64-pc-windows-msvc -p awase-windows
--lib --tests` でコンパイル確認し、実行は CI に委ねる。回帰ガードとしての主力は
(a-1) / (a-2) 側であることに変わりはない。

### (a-4) 診断強化: `[engine-input]` ログに ctx の親指状態を出す【新規・実装 PR に含める】

`key_pipeline.rs:326`（フォーマット文字列は `:327-330`）の
`tracing::debug!("[engine-input] ...")` に
**`ctx.left_thumb_down` / `ctx.right_thumb_down`** を
`l_thumb={:?} r_thumb={:?}` として追加する（両者とも `Option<Timestamp>` なので
`{:?}`）。

本 ADR の手順7が「`state=` フィールドからの消去法による演繹」に頼らざるを
得なかったのは、engine に実際に渡った親指タイムスタンプがどのログにも出て
いなかったためである。次に同型の症状が報告されたとき、
`ts` / `delay_ms` / `state` / `l_thumb` / `r_thumb` を1行で突き合わせれば
**推論なしで**判定できるようにする。決定節 (4) で許容した「クリア後の stale
スナップショット」が実機で顕在化した場合の唯一の検出手段でもある。

### 却下（回帰ガードとして機能しない）

`src/engine/tests.rs` に `NicolaFsm::on_event(event, phys)` へ古い / 新しい2つの
`phys` を渡す単体テストを足す案は、**修正の前後どちらでも通ってしまう**ため不採用。
理由: `on_event` は `phys` を `event` とは別引数で受け取り、採用案はこの署名を
変えない。core 側のテストは「`phys` に何を渡したら何が起きるか」
（＝ [ADR-010](010-thumb-consumption-timestamp.md) の比較ロジック、既に健全と
確認済み）しか検証できず、バグの実体である「誰がその `phys` を作るか」＝
`key_pipeline.rs:291` は `#[cfg(windows)]` で core テストから見えない。

### 影響しないことを確認済みのテスト

`crates/awase-windows/tests/thumb_context_guard.rs`（`#![cfg(windows)]`、
`build_input_context_preserves_thumb_down_timestamps`）は `build_input_context`
の**シグネチャと引数の引き回し**を検証しており、本変更はシグネチャを変えない
（引数の**出所**だけが変わる）ため影響しない。

### known-bugs.md

BUG 番号を採番して起票する（空き番はステータス節参照）。症状・再現手順
（GJI / TSF、Uwp・TsfNative アプリ、「ようするに」→「よゔするに」）・
`report_id: 01M1N36MGDDJ5HN8FWRE4ZHS3J`・修正コミットハッシュを記載する。

## 未決定事項

1. ~~`RawKeyEvent` の新フィールド命名~~ — **解決済み（v2）**。2フィールド案
   （`left_thumb_down_snapshot` / `right_thumb_down_snapshot`）を採用。
2. ~~タイマー経路（`deferred_engine_timers`、実体は `build_ctx()` 経由のライブ
   クエリ）の修正を別 ADR に切り出す~~ — **解決済み（2026-09-08）**。
   [ADR-155](155-timer-path-live-thumb-requery-during-deferred-timer-replay.md)
   として起票され、**同日クローズされた（実装せず）**。理由は「限界」節が特定
   した失敗モードに対する具体的な失敗シナリオを構成できなかったこと。経緯は
   `docs/known-bugs.md` の **BUG-126** に記録済み。したがってタイマー経路は
   本 ADR 実装後も **既知の未修正の穴として残る**（「限界」節参照）。実機で
   顕在化した場合は BUG-126 と ADR-155 を起点に再開すること。
3. **`INPUT_DEFER` に残った stale スナップショット（グローバルゼロクリア3経路）**
   — **解決済み（round3）**。「有界なので許容する」と決定。エポック方式の
   先置き設計とともに「決定」節の案B検討結果 (4) に記録済み。実機で顕在化した
   場合のみ再開する。
4. **doc comment の更新**: `src/types.rs::RawKeyEvent::modifier_snapshot` の
   doc comment（`:251-255`）は `OUTPUT_PENDING_QUEUE` とのみ書いているが、実際に
   本件・本 ADR が扱う経路は `INPUT_DEFER` である。修正実装時に doc comment 側
   にも `INPUT_DEFER` への言及を追加する。

## 関連

report `01M1N36MGDDJ5HN8FWRE4ZHS3J`（本 ADR の起票根拠）、
[ADR-010](010-thumb-consumption-timestamp.md)（`Option<Timestamp>` による
親指消費追跡、比較ロジック自体は本件で健全と確認済み）、
[ADR-008](008-physical-thumb-state-separation.md)（物理親指キー状態と FSM
解決ロジックの分離、本 ADR が踏襲する層分離の前例）、BUG-105
（3鍵仲裁の別欠陥、本件と症状が類似するが原因は別、`docs/known-bugs.md`）、
`docs/bug-reports-triage.md`（本 report の一次調査記録）。
