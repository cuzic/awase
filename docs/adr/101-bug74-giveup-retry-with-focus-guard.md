# ADR-101: BUG-74 give-up retry と focus guard

## ステータス

**採用・実装済み（2026-08-24）。** Windows Terminal + Google 日本語入力 + TsfNative プロファイルで、`RawTsfLiteralRecovery` が2連続 literal を検出して give-up した文字を、GJI reinit の Hiragana 確認後に通常送信経路で1回だけ retry する。実装は ADR-100 決定3（提案2却下）と決定5（F6）を引き継ぐ。ADR-100時点で却下された理由は「retryの発想」ではなく「完了通知・focus世代照合・送信後処理・順序保証が無いこと」だったため、本ADRはその前提を先に作ってから retry を配線する。

## コンテキスト

BUG-74（report_id `01M0RE56S6EQ4MTQGJ2EB2W4N0`）では、Windows Terminal で「こういう」を入力した際、先頭の「こ」が完全に消え「ういう」になった。`ProbeAction::RawTsfLiteralRecovery` の give-up 分岐は、BUG-27追補2の無限再送を避けるため romaji を再送せず backspace cleanup のみを行う。したがって、この分岐に落ちた文字は設計上失われる。

同じ実機ログでは、直後の「う」は per-VK confirm を経由せず、reinit が GJI のI/Oを叩き起こして `gji_settled=true` になったことで通常判定ロジックが eager unicode を選び成功した。つまり「こ」と「う」の差は文字種ではなくタイミングだった。失われた romaji も、reinit完了後に `send_romaji_as_tsf` / `send_romaji_batched` へ戻せば、既存の `decide_transmit_plan` / `gji_settled` 判定が自然に eager unicode または per-VK confirm を選ぶ。

## 決定

### 決定1: F6を先に塞ぐ

`send_chrome_gji_reinit_and_poll` は reinit 開始時点の `ime_mode_focus_gen` を受け取り、IMC poll の各 `with_app` 判定時に現在世代と照合する。**世代不一致は `Stale`** として扱い、IME FSM を更新せず終了する。**`with_app` 再入失敗（`None`）は `Stale` にはしない** — focus 世代照合も Hiragana 確認も行えていない（判定不能）だけであり、実際に stale と分かったわけではないため、`Continue`（未観測、次 tick へ継続）として扱う（純粋関数 `gji_reinit_poll_tick_outcome` に集約、下記コードレビュー訂正参照）。retry付き poll の場合は `WM_GJI_REINIT_RETRY_COMPLETE` に `Confirmed / Timeout / Stale` を載せてメインメッセージループへ戻す。

### 決定2: pending reinit を構造体化する

旧 `pending_gji_reinit_cold_seq: Cell<Option<Generation>>` を、`PendingGjiReinit { cold_seq, focus_gen, phase }` に置き換える。`phase` は `Scheduled` と `Polling` に分け、`Polling` は `OutputActiveGuard` と `poll_token` を所有する。`Scheduled` は後勝ち上書きを許すが、`Polling` は上書き禁止とし、新しいgive-upは `SuppressedExistingPoll` として記録して抑止する。これにより、連続give-upで既存pollのguardを奪って早期dropするRound3 premortemの破綻を防ぐ。

### 決定3: retryは通常送信経路で1回だけ行う

give-up検出時点で `romaji` と `focus_gen` を保存する。reinit poll が `Confirmed` かつ現在focus世代が保存世代と一致した場合のみ、`Platform::complete_gji_reinit_retry` から `Output::resend_gji_reinit_retry_romaji` を呼ぶ。この helper は TSF gate が `Bypass` なら `send_romaji_batched`、それ以外なら `send_romaji_as_tsf` を呼び、Unicode直接送信を新設しない。

retry後は必ず `drain_output_post_send_effects` を実行する。これにより retry が新しい GJI probe や composition reset を発生させても、通常 `send_keys` と同じ後処理境界を通る。

同一 `focus_gen + romaji` には tombstone を持ち、retry由来の次回give-upでは再予約しない。`CompositionConfirmed`、FocusChange、SetOpenTrue 相当の連続カウント解除で tombstone も解除する。

### 決定4: poll中の順序反転を防ぐ

retry付き `Polling` 中は、`flush_stale_deferred_vks_after_recovery` による `warmup_coord.pending_deferred` の即時送信を禁止する。`Confirmed` では `retry → post-send effects → deferred flush → guard drop` の順で処理する。`Timeout` では retryせず、focusが一致していれば deferred を送る。`Stale` またはfocus不一致では deferred を破棄し、ログに件数とstatusを残す。

### 決定5: SuppressedExistingPollではRAW_TSF_LITERALを汚さない

既存 retry poll 中に新しいgive-upが来た場合、`schedule_chrome_gji_reinit` は `SuppressedExistingPoll` を返す。このとき give-up分岐は `set_raw_literal` を呼ばない。単一グローバル `RAW_TSF_LITERAL` に遅延backspaceだけを残すと、既存retryが成功した直後にそのbackspaceが別文字を消すためである。代償として、新しいgive-upのliteral残骸が画面に残る可能性はあるが、既存pollが救おうとしている先行文字を消すより被害が局所的で診断可能である。

## Premortem の経緯

設計は4ラウンドの premortem を経て収束した。Round1/2では、非同期 `with_app` 内で直接 retry 送信すると送信後処理が漏れること、retry待ち中の後続入力が追い越すこと、focus帰属をreinit送信直前に捕捉すると別ウィンドウへ誤送信しうることが見つかった。Round3では、連続give-upが `PendingGjiReinit` を上書きして既存 `OutputActiveGuard` をdropし、poll完了をstale化する「guard奪取」が見つかった。Round4では、`OutputActiveGuard` だけでは内部 `pending_deferred` flush を止められないことと、`SuppressedExistingPoll` の遅延backspace cleanupが既存retry後の文字を消しうることが見つかった。

これらを受け、retryは `with_app` pollクロージャ内ではなくWM完了メッセージ経由で同期境界へ戻し、`Polling` 状態を「poll完了待ち」だけでなく「deferred flush と cleanup 抑止の順序所有者」として扱う設計にした。

### コードレビューによる実装ミスの訂正（Round4設計後、実装フェーズで発見）

Round4設計文書どおりに一度実装したコードに対して、設計文書ではなく実コードを対象にした最終レビューを行ったところ、設計から逸脱している実装ミスが2件見つかった。

1. **focus stale判定がraw cleanup送信より後だった。** 旧実装は `flush_raw_tsf_literal_recovery()` が `flush_raw_tsf_literal_backspaces()`（実送信）を先に実行してから、`start_pending_gji_reinit_after_raw_cleanup()` 内でようやく focus 世代を照合していた。これでは give-up検出後にフォーカスが別ウィンドウへ移った場合、**backspaceが新ウィンドウへ送られてから**ようやく stale 判定される——ADR-100が最初から懸念していた「別ウィンドウへの誤送信」を、判定タイミングの違いで再導入していた。`discard_raw_recovery_if_focus_stale()` を新設し、`flush_raw_tsf_literal_recovery()` の**先頭**（backspace/romaji送信より前）で `pending_gji_reinit.phase == Scheduled` の focus_gen を照合し、不一致なら `RAW_TSF_LITERAL`（backspace/romaji/escape_composition）・`pending_gji_reinit`・`warmup_coord.pending_deferred` を一切送信せず discard するよう修正した。対象は `Scheduled`（直前の give-up がまだ実送信していない予約）のみで、`Polling`（無関係な別 give-up 由来で既にポーリング中）は対象外——`start_pending_gji_reinit_after_raw_cleanup` 側の `AlreadyPolling` 分岐が扱う、stale focus とは無関係な cleanup である。**スコープの限定（コードレビュー指摘、追加訂正）**: この修正が塞ぐのは give-up→reinit 経路（`consecutive != 0`、`pending_gji_reinit` が存在する場合）のみである。`consecutive == 0`（初回疑い、`RawTsfLiteralRecovery` の非 give-up 分岐）で `set_raw_literal` が直接呼ばれるケースには `pending_gji_reinit` が一切存在しないため `discard_raw_recovery_if_focus_stale()` は関与せず、この経路の raw cleanup が focus 変更後に別ウィンドウへ送られるリスクは本 ADR 以前から存在し、今回も未修正のまま残っている（BUG-74 のスコープ外）。
2. **`with_app` 再入失敗が `Stale` 扱いになっていた。** 決定1の記述どおり `Continue`（未観測、次 tick 継続）にすべきところ、実装は `status.unwrap_or(GjiReinitPollStatus::Stale)` で再入のたびに即座に stale completion を確定させていた。たまたま1 tick 再入しただけで、フォーカスは変わっていないのに retry と deferred 救済の両方が失われる。純粋関数 `gji_reinit_poll_tick_outcome(observed: Option<GjiReinitPollStatus>) -> GjiReinitPollTickOutcome` を新設して `None`/`Some(Timeout)` を同じ `Continue` に、`Some(Confirmed)`/`Some(Stale)` のみを `Break` にするよう修正した。

いずれも「設計は正しかったが実装時に逸脱した」パターンであり、design v4（設計文書）に対するレビューだけでなく、実際のコード diff に対する最終レビューが必要だったことを示す実例として記録する。

### `/code-review 97` によるマージ前レビューでの追加発見・修正（2026-08-24）

上記の実装が完了しコミット・push済みの状態で、PR #97 に対する8角度の並列コードレビュー（reuse/simplify/removed-behavior/altitude/conventions/efficiency/cross-file-tracer/line-by-line）を実施した。うち1件は実装ミス（ブロッカー相当）、複数件は重複コード・死んだフィールド・弱いテストの指摘で、以下を追加修正した。

3. **`Scheduled` フェーズの無条件上書き（最重要）。** `schedule_pending_gji_reinit` の上書き防止チェックは `Polling`（実送信済み・guard保持中）のみを対象にしており、`Scheduled`（まだ `WM_DRAIN_OUTPUT_QUEUE` で実送信されていない予約）は無条件に上書きされていた。先行 give-up が `Scheduled` のまま次の give-up が来ると（実機でも起こりうるタイミング——BUG-74 自身のログで同一文字が短時間に2回 literal 化している）、先行 give-up の romaji と `RAW_TSF_LITERAL` の backspace 数が後勝ちで消え、retry も cleanup も行われないまま文字が失われる——**ADR-101 が直そうとしている症状そのものの再演**だった。`ScheduleGjiReinitResult::SuppressedExistingScheduled { existing_cold_seq }` を新設し、`Scheduled` も `Polling` と同様に上書き禁止・新しい give-up 側を抑止するよう修正した。
4. **コードの重複を解消。** `resend_gji_reinit_retry_romaji`（retry送信）と `flush_raw_tsf_literal_romaji`（通常のraw literal再送）が同じ「Bypassなら`send_romaji_batched`、それ以外は`send_romaji_as_tsf`」分岐を手書きで重複していたため、共有ヘルパー `send_romaji_dispatching_on_gate` へ統合した。`flush_deferred_vks_after_gji_reinit_completion` と `flush_stale_deferred_vks_after_recovery` も「pending_deferred取得→marker選択→送信」をほぼそのまま重複していたため、共有コア `flush_pending_deferred_vks` へ統合した。両方とも既存の `ProbeIo::gate_is_bypass()` を使わず `tsf_gate.state() == Bypass` を再導出していた点も、`gate_is_bypass()` の呼び出しに統一した。
5. **死んだフィールドの削除。** `PendingGjiReinitRetry { romaji, attempted }` の `attempted: bool` は、唯一書き込む `take_gji_reinit_completion` が `pending_gji_reinit` を丸ごと `take()` して消費するため、`true` を観測できる経路が構造的に存在しなかった。`PendingGjiReinitRetry` 構造体自体を撤去し、`retry: Option<PendingGjiReinitRetry>` を `retry: Option<String>` に単純化した。
6. **到達不能な match アームの除去。** `gji_reinit_poll_tick_outcome` は `Break(GjiReinitPollStatus)` という戻り値型のため、呼び出し側に構造的に到達不能な `Break(Timeout)` アームが残っていた。終端状態だけを表す `GjiReinitPollTerminalStatus { Confirmed, Stale }` を新設し、`GjiReinitPollTickOutcome::Done(GjiReinitPollTerminalStatus)` に型を絞ることで不能アーム自体を消した。
7. **トートロジーだったテストの削除。** `completion_confirmed_orders_retry_post_send_effects_deferred_then_guard_drop` は、ハードコードした `Vec` リテラルが自分自身と等しいことだけを検証しており、`Platform::complete_gji_reinit_retry` を一切実行していなかった（実装の呼び出し順を変えても壊れない）。削除し、呼び出し順の規約は `Platform::complete_gji_reinit_retry` の doc コメント（SSOT）へ移した。

このレビューではさらに、**MS-IME側の `start_ms_ime_ready_poll` に、項目2で修正したのと同型の `with_app` 再入バグ（`.unwrap_or(MsImePollStatus::Stale)`）が残っている**という指摘があった（BUG-13 領域、本 PR 以前から存在する既存欠陥）。これは本 PR のスコープ外として据え置く——次に MS-IME 側で同種の症状（IMC確認ゲートが理由なく固着する等）が報告された場合、この既知の欠陥から着手すること。

## 根拠

ADR-100 決定3の却下理由のうち、(a) BUG-27追補2型の内側ループ再演は、retryをper-VK confirmループ外のreinit完了通知から最大1回だけ起動することで解消する。(b) 完了通知が無い問題は `WM_GJI_REINIT_RETRY_COMPLETE` で解消する。(c) focus跨ぎ誤送信は、give-up検出時点のfocus世代保存とpoll完了時照合で解消する。(d) 後続入力の順序リスクは、Polling中の deferred flush 抑止と completion時の順序規約で解消する。

## テスト計画

- `raw_tsf_literal_recovery_suppressed_existing_poll_does_not_set_raw_literal`: `SuppressedExistingPoll` では `set_raw_literal` が呼ばれず、単一 `RAW_TSF_LITERAL` を汚さない。
- `started_retry_polling_skips_raw_recovery_stale_deferred_flush`: `StartedRetryPolling` では raw recovery 末尾の stale deferred flush を走らせない。
- `completion_confirmed_orders_retry_post_send_effects_deferred_then_guard_drop`: confirmed retry completion の仕様順序を固定する。
- `discard_raw_recovery_if_focus_stale_clears_state_when_focus_mismatched` / `_leaves_state_when_focus_matches` / `_ignores_polling_phase`: focus stale判定がbackspace送信より前に行われ、`RAW_TSF_LITERAL`/`pending_gji_reinit`を正しく discard/温存することを固定する（コードレビュー訂正1の回帰テスト）。
- `gji_reinit_poll_tick_outcome_none_continues_not_stale` / `_timeout_continues` / `_confirmed_breaks` / `_stale_breaks`: `with_app`再入(`None`)が`Stale`ではなく`Continue`になることを固定する（コードレビュー訂正2の回帰テスト）。
- xwin検証では `check`、`clippy`、`test --no-run` までを実行する。wine が無いためWindows `.exe` 実行は別途実機で行う。

## 未解決の疑問

- Stale/Timeout時に deferred を捨てる/送る判断は安全側に倒したが、実機でのユーザー体感は確認が必要。
- literal detect guard と retry guard が連続すると最大600ms級の遅延になりうる。実機ログで頻度と体感を測る。
- 根本原因である「送信前F2/probe待機の撤去」由来のcold初回literal化は本ADRの対象外であり、別途ADR-079系の課題として残る。
