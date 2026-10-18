# ADR-265 段階 2b 設計 v2(1ae685df)の再レビュー

- 対象: `1ae685df:docs/tasks/adr265-belief-connection-design.md`(v2)
- コード: `/home/cuzic/awase-adr265-stage1` の作業ツリー(84dce2f0)で読んだ。下に挙げたファイルは `origin/develop`(a483093b)と差分がないことを確認済み。読んだだけで、ビルド・実行はしていない。
- 利用者の決定(High・自動判定、確認済みの窓だけ Confirmed、条件 3 は `>=`)は覆さない。指摘するのは、その帰結の穴と、決定を満たしたまま塞ぐ最小の手当て。

---

## 0. 前回の指摘の判定表

| ID | 判定 | 一言 |
| --- | --- | --- |
| C1 drift が MozcShm で送る | **直った**(条件付き) | §2-1 で `evaluate_drift` と `AnyFreshEvidence` の両方から除外した。ただし除外したことで、新しい穴 N2(書き込みが失われた後に誰も直さない)ができた |
| C2 明示意図・IntentStore に隠れる | **形は直った/新しい問題を作った** | 3 手の追随は正しい形。ただし基準にする時刻(「awase の最後の開閉の書き込み」「最後のモードキー」)を 1 つで持つ欄がコードに無い(N1)。「操作より前の読みは捨てる」が、書き込みが失われた場面で共有メモリを恒久に無効にする(N2) |
| H1 Confirmed が BUG-156 を生き返らせる | **一部直った/前提が誤り** | 範囲を絞ったのは正しい。ただし、検証済みの 2 種類のどちらでも `applied` の Confirmed はほぼ効かない(N3: Win32 は ImmCross が先頭で `applied` を見ない。Edge は押下で `applied` を未知にする窓)。危険はまだ残り、得はほぼ無い |
| H2 古い値が残る | **直った**(条件付き) | `expires_at` の TTL を全読み手に効かせる案は正しい。TTL は tick の最大間隔(prefetch の 300ms のタイムアウト+タイマーの張り直し)より長くすること(N6) |
| H3 書き込み点 | **直った/小さな新問題** | 新しい variant で、pending と mode-key-pass に触らない方針は正しい。例に挙げた `scope: ForegroundScope` は `Serialize` でなく、`ImeEvent` は `serde::Serialize` を derive しているのでそのままではコンパイルできない。また核(`ImeModel`)に scope の概念が無い(N5) |
| M1 予測の照合 | **直った**(実装の注意あり) | `update_tick_ms < 予測の時刻` で照合しない。ただし `AnyObservation` は `update_tick_ms` を運ばないので、`reduce` 内の `reconcile_key_effect_open` では判定できない。ハブで報告そのものを止める形にすること |
| M2 tsf/effective | **一部直った** | 割れたら報告しない、は維持。割れたときの `applied` の降格が「または」の任意扱い。割れは食い違いが確定した状態なので、降格を必須にすること |
| M3 journal の枠 | **一部直った/文書内で矛盾** | 「頻度」の行は「変化時+TTL/2」、同じ表の「鮮度」の行は「受け入れている間は毎 tick 報告」と書いている。どちらかに統一する |
| M4 全利用者の負担 | **直った** | ただし「GJI 系の TIP のとき」だと Google 版 GJI の利用者全員が対象に入る。(tid, pid) ごとに `NotFound` を覚える案を主にすること |
| M5 照合の同義反復・tid 再利用 | **直っていない** | 言及なし。優先度は低いまま |
| e1〜e4, e6, e7, e10, e11 | **直った** | v2 本文に反映済み |
| e5(TsfNative の押下) | **反映したが、Edge に誤って当てている** | Edge は TsfNative ではない(N3) |
| e8(`FocusHwndUpdated` で applied が未知に戻らない) | **直っていない** | Part B で Confirmed を書くと、同じプロセスの別スレッドの窓へ持ち越される |
| e9(`pending_write` の範囲) | **概念上は直った** | `update_tick_ms` の比較に置き換えた。ただし比べる相手の時刻がコードに無い(N1) |
| e12(同一プロセス内の tid 再利用) | **直っていない** | — |

---

## N1(重大、(a))時計の域は同じ。ただし比べる相手の時刻を 1 つで持つ欄が無く、既存の欄はどれも範囲が足りない

**時計の域の判定**: 同じ時間軸である。awase の `hook::current_tick_ms()` は `GetTickCount64()`(`hook.rs:802-806`)。Mozc の `update_tick_ms` も `GetTickCount64()`(`state/mozc_shm.rs` の `RawBlock` の doc)。どちらもシステム全体で共通の値で、64bit なので一周もしない。`dispatch_event` の `tick_ms` も呼び出し側が `current_tick_ms()` から渡している(`ime_refresh.rs:287`、`key_pipeline.rs:1983`)。**時計の域が違うという意味での前提の誤りは無い**。

**問題は「何の時刻を記録しているか」**。設計が使う 2 つの基準は、どちらもコードに 1 つの欄として存在しない。

| 設計の基準 | 近い既存の欄 | 足りない範囲 |
| --- | --- | --- |
| 「awase の最後の開閉の書き込み」 | `last_explicit_ime_action_ms`(`platform_state.rs:937`、`:1030`) | `handle_engine_set_open` の**要求時**と `note_explicit_ime_action` でしか更新しない。drift correction(`ir_apply_drift_correction`)・shadow toggle(`kp_shadow_actuate`)・PanicReset 後の `SetOpen`(`runtime/mod.rs:2424-2431`)・ImmCross の async 書き込みの**実際の時刻**(spawn_local の後で、`run_with_timeout` で最大 300ms 遅れる)は入らない |
| 同上 | `external_change_watch.pending_write.at_ms`(`platform_state.rs:1481-1484`) | SendInput の `Applied` だけ。ImmCross(`AppliedWithoutSendInput`)は入らない。枠は 1 つで、読みが来ると消える |
| 「最後のモードキー」/「直前の打鍵」(§2-2 と §5-2 で**語が違う**) | `key_effect.at_ms` / `press_axes`(`platform_state.rs:300-306`) | 表で予測が付いた打鍵だけ(`key_pipeline.rs:1964-1974` は予測が無ければ return)。Shift 付き・利用者が Mozc に割り当てた独自のキー(Ctrl+Space など)・表に無いキーは入らない。こうしたキーで IME が変わると、処理前の古い読みが条件 3 を通る |
| 同上 | `hook::hook_alive_tick_ms()`(`hook.rs:310-320`、フックの入口 `:1438` で毎回更新) | **すべてのキー**(KeyUp・自分の注入・他アプリの注入を含む)。awase の SendInput の書き込みもフックを通るので、その時刻も含む。ImmCross だけは入らない |

**推奨対応**:
- 基準を「`max(hook_alive_tick_ms, 最後の ImmCross の書き込みの発行時刻)`」と定義する。新しい欄は「ImmCross の発行時刻」1 つだけで済む(`open_chain.rs::imm_cross_write` の発行時に記録)。
- `hook_alive_tick_ms` は自分の注入キー(SendInput による開閉の書き込み)が IME に届く**前**に更新される。だから「書き込みの後に変わった」の判定に必要な順序(基準 ≤ IME の処理時刻)が構造的に成り立つ。
- §2-2 と §5-2 で語をそろえる(「直前の打鍵」=全キーで統一)。
- 全キーを基準にすると、打ち続けているあいだは条件 3 がほとんど通らない。これは安全側で、下の N4 の R3' で便益を取り戻せる。

---

## N2(重大)「操作より前の読みは捨てる」は、awase の書き込みが失われた場面で共有メモリを恒久に無効にする

**場所**: §2-2「`update_tick_ms < 基準` の読みは追随せずに捨てる」、§2-1(drift から除外)、`ime_model.rs:466-487`、`drift_correction.rs:139`。

**根拠**: `update_tick_ms` は「最後に**変化**を書いた時刻」。awase の書き込みが IME に届かなかったとき(BUG-200 の系統そのもの、GJI の取りこぼし)、共有メモリは変化しない。だから `update_tick_ms < 基準` のまま、次に外で何かが変わるまで永久に止まる。

**失敗シナリオ**: Edge でトレイから開く。`write_set_open_request` で `last_intent=Some(true)`、`desired=true`。GjiDirect の送信が取りこぼされ、IME は閉のまま。
- 共有メモリは閉で、`update_tick_ms` は書き込みより前 → §2-2 は「操作より前」と判定して捨てる。
- 明示意図があるので、`resolve_open_at` は `MozcShm` を見ない。
- drift からは除外済みなので再送もしない。
- 結果、**belief=開・実 IME=閉が、窓を変えるか物理モードキーを押すまで固定される**。エンジンはローマ字を閉じた IME に送り、`ka` などがそのまま入る。mozc-readable を入れていない場合と同じか、それより悪い(v1 では drift が直していた)。

**推奨対応**: N4 の R3' の第 2 項(`読んだ時刻 − 基準 ≥ SETTLE`)で解ける。十分に時間が経ってもなお `update_tick_ms < 基準` なら、それは「書き込みが効かなかった」という事実なので追随してよい。belief は閉に揃い、利用者が押し直せる(TsfNative×GJI の受動化の方針とも一致する)。再送はしない(drift の除外は維持)。

---

## N3(重大、(c)・H1 の前提)検証済みの 2 種類では、Part B の Confirmed はほとんど効かない。Edge は Chrome などとクラスでは区別できないが、プロセス名なら区別できる

**根拠 1: 素の Win32 窓では `applied` が送信の省略に使われない。**
- `Standard × Gji` の chain は `CHAIN_IMM_CROSS_THEN_GJI`(`app_ime_policy.rs:110-112`)。ImmCross が先頭で使えるとき、`plan_set_open` は `AsyncImmCross`(`ime_set_open_plan.rs`)を選ぶ。
- ImmCross は `applied` を見ず、失敗時の再読みも実状態で判定する(`imm_cross_reobservation_already_matches(actual, ..)`、`open_chain.rs:369`)。フォールバックの `fallback_write` は `shadow_on` を `None` に潰す(`open_chain.rs:469-494`、BUG-113)。
- したがって **Notepad 等で `applied` を Confirmed にしても、省略は一切変わらない**(ImmCross が使えないと学習された窓だけが例外)。実験 B(IMM の書き込み)の 50 試行は、Part B の安全性の証拠としては対象がずれている。

**根拠 2: Edge は TsfNative ではなく `Imm32Unavailable` で、押下では `applied` が既に未知になる。**
- `Chrome_WidgetWin_1` / `Chrome_RenderWidgetHostHWND` は `IMM32_UNAVAILABLE_CLASSES`(`class_names.rs:19-35`)にある。`is_effectively_tsf_native("Chrome_WidgetWin_1")` は false(同 `:486-489` のテスト)。
- だから `engine_press_unknowns_applied(false) == true` で、Edge の押下は `applied` を未知にしてから決める(`sync_actuation.rs:313-316`)。v2 §5-5 の「TsfNative では押下も省かれる」は Edge には当たらない。
- Edge で `applied` が効くのは `press=None` の書き込みだけ。具体的には、エンジンの有効/無効の切り替え(`src/engine/engine.rs:466-478`、`:873`、`:911`)、PanicReset 後の `SetOpen(true)`(`runtime/mod.rs:2424-2431`、直前に `applied=Unknown`)、自動リピート。drift correction は `build_ime_control_view(None)` で `applied` を見ない。

**根拠 3: 得は「余分な送信を 1 回省く」だけ。**
- ADR-214 の P1(古い `applied` が「一致」と言って送らない)は、共有メモリと `applied` が**食い違う**場面なので、**降格(`Unknown`)だけで完全に直る**。v2 の範囲外の扱いと同じ。
- Confirmed にして初めて得られるのは、`applied` を `Some(x)` に戻して `SetOpen(x)` を省くことだけ(方向 (i)、余分な送信の削減)。Edge × GJI で余分な `VK_IME_ON`/`OFF` を送る害は小さい(押下では毎回送る設計 = ADR-208 D1 で、すでに許容している)。
- 一方で危険(N4 の `>=`、E1、e8 の持ち越し)は残る。**費用対効果は負**と判定する。

**根拠 4((c) への回答): 区別はできる。**
- `CurrentFocus` は `process_name`(小文字)を持つ(`focus/current.rs:28-29`)。
- 「Edge = クラスが `Chrome_RenderWidgetHostHWND` または `Chrome_WidgetWin_1`、かつ `process_name == "msedge.exe"`」で、Chrome(chrome.exe)・VS Code(code.exe)・Slack・Teams(ms-teams.exe / msedgewebview2.exe)・LINE は巻き込まない。WebView2 は `msedgewebview2.exe` で別名になる。
- 「素の Win32」は `AppImeProfile::Standard` では**広すぎる**。Standard は既定値なので、Firefox(`MozillaWindowClass` は `IMM32_UNAVAILABLE_CLASSES` に無い)、Qt、Java、Office も含む。(クラス, プロセス) の許可リストで列挙すること(例: `notepad.exe` の編集クラス)。
- Edge のアドレスバー(`Chrome_WidgetWin_1`)とページ(`Chrome_RenderWidgetHostHWND`)は hwnd もクラスも違う。実験がどちらで行われたかを結果に書くこと。

**推奨対応(利用者の決定 2 を満たす最小の形)**: 範囲は (プロセス名, クラス) の許可リストで書く。そのうえで、Part B を実装する前に「Confirmed にして省ける送信が実際にあるか」を数える。数え方は、journal の `ActuationDecision` で、Edge の `press=None` の書き込みが `applied=Unknown` のせいで送られた件数。0 に近ければ、Part B は降格だけで止めることを ADR で提案する価値がある。利用者の決定を覆すのではなく、決定の効果が検証済みの範囲ではほぼゼロ、という事実の提示として出す。

---

## N4(重大、(b))`>=` の危険はコードの経路で作れる。最小の追加条件で安全にできる

**`>=` が古い読みを通す条件**: 同じ目盛り(約 15.6ms)の中に「変化 1(Mozc が書き込み済み、`update_tick=T`)」と「操作 2(基準=T、IME はまだ処理していない)」が入り、その間に読みが来ること。

**具体的な経路**:
1. **awase 自身の連続送信**: フォーカス直後の eager TSF warmup の `VK_DBE_HIRAGANA`(Chrome 系では IME を開ける。`runtime/mod.rs:1078-1083` のコメントが実害を記録している)と、それに続く `SetOpen(false)`。GjiDirect の複数キーの列(候補表示後の OFF の `SendVk`)。どちらも数 ms 以内に 2 回 IME の状態が動く。
2. **利用者の高速な連続キー**: Ctrl+無変換 の chord、無変換→変換 の素早い打ち替え、半角/全角の自動リピート(約 33ms。同じ目盛りには入らないが、2 回目の処理前に 1 回目の結果を読む窓が毎回できる)。
3. **外部変更と awase の書き込みの同時**: Mozc のツールバーのクリックと、Engine の切り替えが同じ目盛りに入る。頻度は低い。

**損害**: 追随(§2-2)が誤った値で発火すると、`intent_store.remove` で明示意図(TTL 10/30 秒)が消え、`desired` が古い値に揃い、`applied` が降格する。次の読みで `update_tick > 基準` になれば正しい値へもう一度追随するので、**1 tick(約 0.5〜0.8 秒)で自然に戻る**。ただしその間の打鍵はリテラル化する。明示意図は戻らない(戻らなくても、`MozcShm` が新しいあいだは High の `derive_any` が支える)。Part B の誤った Confirmed も、次に通った読みで上書きされる。ただし基準が「全キー」だと打ち続けているあいだは通らないので、誤りが長く残る。

**最小の追加条件(提案 R3')**:
```
R3' = (update_tick_ms > 基準) || (読んだ時刻 − 基準 ≥ SETTLE)
```
- 第 1 項(strict)は、操作より後に変化したことが確実な読み。
- 第 2 項は、操作から SETTLE 以上経っていれば、その読みは操作を処理した後の安定した状態(変化したなら新しい値、変化しなかった・失われたなら古い値で、どちらも事実)、という判定。`update_tick` が基準と同じ目盛りか、より前でも採る。
- SETTLE は Mozc の処理遅れの上限。実測の最大 95ms(E3)+余裕で、既存の `KEY_EFFECT_SETTLE_MS = 170` を流用すれば定数は増えない。
- 効果: 同じ目盛りのあいまいさ(`>=` の危険)を消しつつ、tick(約 500ms)のほとんどの読みは操作から 170ms 以上後なので第 2 項で通る。Edge の 40/40 相当の便益が残る。同時に N2 の「書き込みが失われた」場面も救う。
- seq の増分で確かめる案は使えない。基準の時点の seq はフックの中で読めないので、比べる相手が無い。

---

## N5(中、(d))新しい variant の更新箇所と順序

**更新箇所の洗い出し(v2 の列挙に足りないもの)**:
- `ImeEvent` は `#[derive(strum::IntoStaticStr, Debug, Clone, serde::Serialize)]`(`ime_event.rs:339`)で、`Deserialize` は無い(書き出し専用)。journal の再生は `ImeEvent` を読み戻さない。だから「journal の直列化形式の更新」は、`event_kind` の名前が自動で増えるだけで済む。ただし全フィールドが `Serialize` である必要がある。`ForegroundScope`(`foreground_scope.rs:4`)は `Serialize` ではない。**`scope` は持たせない**こと(核には scope の概念が無く、窓の照合はハブで済ませてから dispatch すればよい)。
- `reduce` の網羅 `match`(`ime_model.rs:777`)、`architecture_guard.rs::applied_direct_assignments_are_accounted_for`(`ime_model.rs` 9→10)。
- `ime_event_guard` は、構築を指定の 1 箇所に限る variant の列挙(`lints/ime_event_guard/src/lib.rs`)。同じ扱いにするなら追記が要る。
- `initial_focus_scope_event_is_dispatched_from_one_place` と同形の「出現数」のガードを新しい variant にも作る(`architecture_guard.rs:4620-4664` が手本)。
- 閉ループの harness(`tests/support/harness.rs`)に、reduce を通す口が要るかを確認する。

**pending と mode-key-pass に触れない保証の固定**: `ime_model.rs:1304-1340` の `initial_focus_scope_established_touches_only_the_scope_identity` と同形の単体テストにする。新しい variant を reduce する前後で `ImeModel` を比較し、`applied` 以外(`pending`・`last_intent`・`desired_open`・`key_effect`・観測プール)が不変であることを確かめる。mode-key-pass の記録はハブ側(`mode_key_pass_mark`)なので、ハブの入口が `note_awase_write_for_mode_key_pass_in_scope` を呼ばないことを、関数本文の文字列ガードで固定する。

**順序の競合**: 単一スレッドで、tick の中の読み→dispatch のあいだに打鍵は割り込まない。だから同じ tick の中で「降格」と「確認」が逆順になることは無い。競合するのは tick をまたぐ場合。打鍵の予測の降格(`KeyEffectPredicted`)→次の tick の確認、の順なら、R3' の基準(全キー)が確認を止める。逆に確認→打鍵の降格なら、正しく `Unknown` に落ちる。**追随(`ModeKeyPassedThrough`)と確認を同じ tick で出すなら、追随(降格)を先、確認を後に固定する**(同じ読みなので結果は確認の値になる)。逆にすると、確認した直後に `demote_applied` が古い `desired` 由来の値と比べて落とすことがある。

---

## N6(中、(e))`expires_at` の TTL は仕様と衝突しない。ただし値の下限と、既存の原則との違いを書くこと

- **衝突しない点**: `expires_at` の書き込みは `record_any` の 1 箇所(`per_source_fields_are_not_assigned_directly` のガードの範囲内)。`at` から決まる値なので、journal の再生(`record_replayed(observed, at)`)でも同じ結果になる。`per_source` は 1 ソース 1 枠の置き換えなので、期限切れは「枠は残るが全読み手が見ない」になり、意味の衝突は無い。`consensus()` も `is_expired` を見る。
- **書くべき差分**: `heuristic_default` の doc(`observation_store.rs:587-595`)は「belief/actuation の根拠に鮮度上限を追加で持ち込まない(ADR-087 INV-22 撤回)」としている。`MozcShm` だけはこの原則の例外になることを、ADR に明記する。
- **TTL の下限**: tick は `spawn_ime_refresh` の中で prefetch(`run_focus_probe_async` + `read_ime_state_full_async`、`run_with_timeout` の 300ms)を待ってから実行される(`runtime/mod.rs:1074-1092`)。タイマーは張り直し方式(`schedule_ime_refresh`)。だから間隔は 500ms を超えうる。TTL が「500ms + 小さな余裕」だと、prefetch がタイムアウトした tick で期限が切れ、`effective_open` がフォールバックへ一瞬反転する(`[effective-open-flip]` が出て、その間の打鍵の挙動が変わる)。TTL ≥ 500 + 300 + ジッターを実測で導くこと。M3 の「TTL/2 ごとの更新」と組み合わせるなら、更新間隔も tick に乗るので、結局「毎 tick」に近くなる。
- drift の追跡(`update_drift`)は期限切れでも消えない(既存の挙動)。`MozcShm` は drift の根拠から除外済みなので実害は無いが、`drift_duration` の起点だけが `MozcShm` 由来になることがある。

---

## N7(中、(f))E1 が未解決のまま Part B に進む危険

**損害の上限(コードで特定)**: 公開されない外部変更があると、共有メモリは古い値のまま。`applied` も古い値(awase の最後の書き込み)で、多くの場合同じ値になる。だから **Part B が上乗せする危険は「`Unknown` を古い値の `Some` に戻す」遷移だけ**。`Unknown` になる契機はフォーカス変更(`ime_model.rs:1022`)・PanicReset(`:796`)・予測の食い違い(`:931`)・追随の降格(`:962`)。その後に古い Confirmed が書かれ、Edge の `press=None` の書き込みが省かれる。

具体的には次の 2 つ。
- エンジンの有効/無効の切り替えによる `SetOpen`(`engine.rs:466-478`)
- engine の明示操作の補助(`:873`/`:911`)

PanicReset 後の `SetOpen(true)` は同じ関数の中で積まれるので、tick は割り込まない。押下、drift、Win32 窓は影響を受けない(N3)。

**判定**: 損害の範囲は狭い(Edge × `press=None` のうち、`Unknown` だった場面)。ただし Part B の得もほぼ無い(N3)。**塞ぐ最小の手当て**は、Part B の Confirmed を「`update_tick_ms > 最後に applied を Unknown にした時刻`」のときだけにすること。降格の後に共有メモリが変化していれば新しい事実、変化していなければ降格の原因(フォーカス変更・予測)より前の値なので戻さない。これで E1 の上乗せの危険は 0 になる。その代わり、フォーカス変更直後に Confirmed になる場面は消える(そこは `Unknown` で送るので P1 は起きない)。この条件を入れるなら、E1 を未解決のまま進めてよい。入れないなら、ADR に「受け入れる危険」として上の 2 経路を名指しで書く。

---

## N8(中)追随の判定が belief とだけ比べるので、`desired_open` が古いまま残る

**場所**: §2-2「値が belief と食い違う」。`ime_model.rs:466-487`、`platform_state.rs:1144-1151`。

**根拠**: 明示意図が無いときは、`MozcShm` を報告した時点で `effective_open` は共有メモリの値になっている(High の `derive_any`)。だから「belief と食い違う」は成り立たず、追随しない。`desired_open` は古いまま残る。
- `MozcShm` が期限切れになる(TTL、別スレッドの窓、`active=0`)と、`resolve_open_at` は `desired_open` へフォールバックし、古い値が戻る。
- `HwndCacheRestored` で窓へ持ち越す値にも古い `desired` が使われうる。

**推奨対応**: 追随の条件を「共有メモリの値 ≠ `effective_open` **または** ≠ `desired_open`」にする。追随は `ModeKeyPassedThrough { align_desired: true }` なので、`desired` だけを揃える用途にもそのまま使える。

---

## N9(低)追随の入口が 3 つになる。同じ tick で値が割れたら後勝ちになる

- `ir_follow_external_change`(ObserverPoll)、`ir_follow_direct_mode_key_read`(ObserverPoll)、新しい MozcShm の追随が、同じ tick に並ぶ(`ime_refresh.rs:73-80`。共有メモリは最後)。prefetch の IMM の読み(worker スレッドで先に読んだもの)と共有メモリの読みが割れると、`desired` が 1 tick に 2 回揃え直され、後に来る `MozcShm` が勝つ。値として正しいのは `MozcShm` なので実害は小さい。ただし journal に `ModeKeyPassedThrough` が 2 件並び、診断を混乱させる。
- `architecture_guard.rs::external_change_watch_has_single_arm_and_follow_sites`(`:4694-`)は追随の入口を列挙して固定している。新しい入口はここへの追記が必要(v2 §7 に無い)。
- 推奨: Edge(`Imm32Unavailable` × GJI、両方の既存の追随が効く窓)では、`MozcShm` が受け入れられた tick は ObserverPoll の追随を飛ばす。または、順序と後勝ちを ADR に書く。

---

## N10(低)文書の記述の食い違い(新規)

| # | v2 の記述 | コード |
| --- | --- | --- |
| n1 | §5-3「Edge(TSF ネイティブ)」 | Edge は `Imm32Unavailable`。`is_effectively_tsf_native` は false(`class_names.rs:486-489`)。§5-5 の TsfNative の理由は Edge に当たらない |
| n2 | §5-1「省略の判断は `applied_open()` を見る」を全窓の前提にしている | Standard × GJI(ImmCross が先頭)は見ない(N3) |
| n3 | §2 の表で、「鮮度」の行は「毎 tick 報告」、「頻度」の行は「変化時+TTL/2」 | 矛盾。どちらかに統一する |
| n4 | §2-2 は「最後のモードキー」、§5-2 は「直前の打鍵」 | 基準が 2 種類ある。どちらの欄もコードに無い(N1) |
| n5 | §5-4 の例 `AppliedConfirmedByShm { open, scope }` | `ForegroundScope` は `Serialize` でない(N5) |
| n6 | §5-2 条件 1「窓の照合(epoch・hwnd)」 | 記録した瞬間には同義反復(M5、未対応) |

---

## まとめ(推奨の優先順)

1. R3' = `update_tick > 基準 || 読んだ時刻 − 基準 ≥ KEY_EFFECT_SETTLE_MS`。基準は `max(hook_alive_tick_ms, ImmCross の発行時刻)`。これで `>=` の危険(N4)、書き込みが失われた場面(N2)、基準の欄が無い問題(N1)を同時に塞ぐ。
2. 追随の条件に `desired_open` との食い違いを足す(N8)。
3. TTL の下限を tick の最大間隔(prefetch のタイムアウトを含む)から実測で導く(N6)。
4. Part B の範囲は (プロセス名, クラス) の許可リストで書く。Win32 は効果ゼロ、Edge は `press=None` だけ、という事実を ADR に書く。Confirmed は「`update_tick > 最後の降格時刻`」のときだけにする(N3・N7)。
5. 新しい variant には `scope` を持たせない。不変性のテストと出現数のガードを作る。追随(降格)→確認の順を固定する(N5)。
