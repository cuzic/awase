# 機構 ECRS 調査 B: TSF / warmup / cold-start / literal 検出 / defer キュー(接頭辞 MB)

調査基準: `/home/cuzic/rust-nicola-wt-mech`、`f3c2007e`(origin/develop 先端)。読み取りのみ。cargo は実行していない。
再報告しないもの: UIA 非同期、TsfReadinessProbe の待機(T01、HEAD で適用済みを確認)、ChromeProbe、WarmEpoch/ColdContext、`gji_coro_body` の `!needs_literal` 分岐、platform_state.rs の core 移動、R4。
`raw-tsf.md`(コード単位の ECRS)の T03/T04/T13/T14/T15/T17/T21 は重ねて書かず、必要な箇所だけ番号で参照する。

実機ログの一次資料として、リポジトリ内の `tools/e2e/ime_key_matrix/results/*/awase-full.log`(その後撤去。以下の grep は `675be51a` を `git archive 675be51a tools/e2e/ime_key_matrix/results | tar -x -C <空ディレクトリ>` で展開して打つ)(94 ディレクトリ、2026-09-19〜20 の CI 実機、DEBUG レベル)を数えた。
**母数が小さい**(Tsf モード送信 32 件・cold-start の transmit-plan 2 件、主に Windows Terminal×GJI と Win32 窓)ので、「0 件」は「その構成で到達しなかった」までしか言えない。各項目に件数を書く。

---

## 結論

E(撤去可)9 件、C(統合可)4 件、?(要実測)4 件、K(残す)は末尾に列挙。上位 5 件:

1. **MB-01** cold の 2 本のコルーチン内にある「inline LiteralDetect」(`tsf_probe_coro_body` の Phase 3 と `gji_coro_body` の Phase 6)は本番で到達しない。cold_seq が毎回新しく、per-VK confirm が必ず先に取るため。本番で約 -250 行(概算)。
2. **MB-02** warm パスの `LiteralDetectFsm`(`plan_literal_detect` の `Install`)は「TsfGate が Probing」かつ「Tsf モードでない」を要求するが、Probing になるのは ForceTsf=Tsf モードのときだけ。条件が矛盾する。partial-literal 判定・veto・`GJI_CANDIDATE_VETO_CAP_MS` も道連れに消える。約 -330 行+テスト。BUG-210 の修正はこの経路向けだった(実害の実機ログは未確認と BUG-210 自身が書いている)。
3. **MB-04** `EVENT_OBJECT_NAMECHANGE` の WinEvent フック(全プロセス対象、OUTOFCONTEXT)。書き込み先 `focus_namechange` の読み手がゼロ。システム全体の名前変化のたびにメッセージループを起こす。
4. **MB-05** `assess_warmth` の `session_expired` / `COMPOSITION_TIMEOUT_MS`(2000ms)は恒久的に偽。`send_keys` が先に `mark_send()` するため `elapsed` が常に 0ms。実機ログで Tsf 送信 32 件すべて `elapsed=0ms`(うち直前送信から 14 秒・35 秒空いた打鍵もある)。
5. **MB-06** `RuntimeOutbox`(1 バリアント `StartTsfProbe` だけ、コメントが「belt-and-suspenders」)。同じタイマー設定を `drain_output_post_send_effects` が既にしている。

つなげて効く順序: MB-01 → MB-02 → MB-03(TsfGate 縮小)→ MB-10(2 本のコルーチンの統合)。MB-04〜07 は他と独立で、先にやれる。

---

## E: 撤去してよいもの

### MB-01 cold コルーチン内の inline LiteralDetect(Phase 3 / Phase 6)と `apply_transmit_done` の検出器経路
- 場所: `tsf/warmup/probe_fsm.rs` の `tsf_probe_coro_body` の「Phase 3」(`transmit_input.transmit_done` を受けるループ)、`tsf/warmup/gji_warmup_coro.rs` の Phase 6(`LiteralDetectCore::new` 以降)、`TransmitDonePayload`、`TickableFsm::apply_transmit_done`、`ProbeCoroState::apply_transmit_done`、`GjiWarmupCoro::literal_detect_guard`、`output/probe_io.rs` の `Transmit` アーム 2 つの `detector = plan.needs_literal.then(..)`。
- 解いていた問題: BUG-024(代理指標 nc_fired による誤検知)以前の「送信後に語全体を確認する」方式。BUG-024.md の状態は「per-VK confirm に置き換わり解決済み」。
- 生きているか: **未到達**。`Transmit` アクションが `needs_literal=true` で出る条件は 2 つだけで、どちらも成り立たない。
  - Chrome(`tsf_probe_coro_body`): `needs_literal = env.gji_active` が真なら `env.literal_session_confirmed_gen != Some(cold_seq)` で per-VK へ行く。偽(= gji_active でない)なら `Transmit` の `needs_literal` も偽。
  - TSF(`gji_coro_body`): per-VK の条件は `plan.needs_literal && gen != Some(cold_seq) && !used_eager && is_tsf_mode`。`needs_literal` は `decide_transmit_plan` で `!nc_confirmed && is_tsf_mode && gji_active`、このとき `used_eager_path` は `else if env.is_tsf_mode { false }` で必ず偽。残るのは `gen == Some(cold_seq)` だけ。
  - `gen == Some(cold_seq)` は起きない。`cold_seq` の発行元は 2 箇所だけで、どちらもコルーチン生成の直前に採番する: `git grep -n "increment_cold_start_count" -- crates/awase-windows/src` → `output/vk_send.rs:279`(Chrome)、`tsf/warmup/cold_warmup.rs:101`(TSF)。`Generation::next()` は単調(`tsf/probe.rs:197-199`)。`mark_literal_session_confirmed` の呼び出しは `output/probe_io.rs:707` の 1 箇所で、渡すのはアクションが持つ自分の `cold_seq`。新しい `cold_seq` が先に mark されることはない。
  - `MsImeReadyCoro` の `Transmit` は `needs_literal: false` を固定で渡す(`ms_ime_ready_coro.rs`、`needs_literal: false,`)。
  - 実機ログ: `grep -rhE "transmit-plan needs_literal=" tools/e2e/ime_key_matrix/results | wc -l` → 2 件、2 件とも `true` で、直後に `per-VK[0/1] confirmed` が続く(`FINAL-noSuppress-s5` 等)。`false` は 0 件。
- 重複・統合先: `LiteralDetectCore::poll` は warm の `LiteralDetectFsm` と共有されていた(ファイル冒頭の doc が「重複を解消するため集約」と書く)。cold 側が未到達なら共有の理由が消える。
- 撤去した場合に変わる挙動: なし(到達しない)。確認のため `dispatch_probe_actions` に `debug_assert!(!plan.needs_literal)` を 1 本置く手がある(`ProbeAction::Transmit` の 2 アーム)。`output/probe_io.rs` のテスト約 10 本(`tsf_transmit_*literal*`、`chrome_transmit_with_gji_healthy_installs_literal_detect` など)は「到達しない入力」を検証しているので一緒に消える。
- 擬似 IME: 検証できない。`output/` 全体が `#[cfg(windows)]`(`lib.rs:90-91`)で `FakeProbeIo` のテストは windows-build CI のみ。足りないもの → 末尾「擬似 IME に足すもの」の 1・2。ただし本項は「到達しない」ことを grep とログで示せるので、先に撤去して per-VK 側の既存テストを残せばよい。
- 判断: **E**。再発ファミリー: warmup / cold-start(`tsf/`、`output/probe_io.rs`)。fix-requires-evidence の (a) は「撤去前後で per-VK の既存テストが緑」を書く(挙動を変えない削除)。
- 規模(概算、未確認): 本番 約 -250 行、テスト 約 -400 行。`tests/architecture_guard.rs:9638-9650`(`machine.apply_transmit_done(` の順序を固定)は更新が要る。

### MB-02 warm パスの `LiteralDetectFsm`(+ `plan_literal_detect`、`LiteralDetectCore`、partial-literal、veto)
- 場所: `output/vk_send.rs:626-668`(`send_romaji_as_tsf_warm` の LiteralDetect 設置)、`awase-windows-plans/src/state/warm_send_plan.rs` の `plan_literal_detect`、`tsf/warmup/literal_detect_fsm.rs`(`LiteralDetectCore::poll`、`is_partial_literal`、`PARTIAL_LITERAL_BS`、`veto_decision`、`word_level_recovery_params`)、`tuning::GJI_CANDIDATE_VETO_CAP_MS`、`LiteralDetector::veto_eligible`。
- 解いていた問題: BUG-027/030(warm 後の語単位確認)、BUG-210(その baseline 順序)。
- 生きているか: **未到達の疑いが強い(条件が矛盾)**。`plan_literal_detect` は `gate_probing && gji_active && !long_idle && !is_tsf_mode` のときだけ `Install`(`warm_send_plan.rs` の `plan_literal_detect`)。
  - `gate_probing` は `tsf_gate.state() == Probing`。Probing に入るのは `confirm_tsf()` だけで、その呼び出しは `runtime/mod.rs:1263-1265` の `injection_hint() == ForceTsf` のときのみ。
  - ForceTsf は `output/types.rs:21` の `injection_mode_for` で `InjectionMode::Tsf` になる。つまり Probing ⇒ ほぼ必ず `is_tsf_mode() == true`(`output/mod.rs:647`)で、`!is_tsf_mode` と両立しない。両者の食い違いはフォーカス直後の hint 再計算の競合(`runtime/focus_tracking.rs:92-95,135-138` と `runtime/mod.rs:1791`)に限られる。
  - さらに `is_partial_literal` は `env.is_tsf_mode` を要求する(`literal_detect_fsm.rs` の `is_partial_literal`)ので、warm パスでは常に偽。`PARTIAL_LITERAL_BS`・ESC+BS の partial 回収は warm では出ない。cold の Phase 6 は MB-01 で未到達。
  - 実機ログ: `[tsf-warm-start]` 30 件に対し `[literal-detect]` 0 件(`grep -rhE "\[literal-detect\]" tools/e2e/ime_key_matrix/results | wc -l` → 0)。warm 送信は gate が `Ready`(最初の cold probe 完了で `on_tsf_probe_ready`、`output/mod.rs:1203`)になった後なので `Skip(GateNotProbing)`。BUG-210 自身も「実機で誤判定が起きていたかは未確認」と書く。
- 重複・統合先: cold の per-VK confirm が同じ判断(compose できたか)を担っている。
- 撤去した場合に変わる挙動: Probing 中・非 Tsf モード・warm・GJI active・非 long-idle の送信だけ。ログ 94 件では 0 回。残る競合窓では「語単位の事後確認が無くなる」。
- 擬似 IME: 検証できない(MB-01 と同じ。warm パスは `Output` の中)。実機で決めるなら `[literal-detect] cold=` が warm 由来で出たログの有無を R2 の不具合報告(`bug-report-fetch`)で数える。0 件なら E で確定、1 件でもあれば出た条件(窓・モード)を見る。
- 判断: **E(条件付き。上のログ確認を 1 回してから)**。再発ファミリー: warmup(`state/warm_send_plan.rs` は表に載っている)。`architecture_guard.rs:8381,8461,9656-9690`(BUG-210 の順序ガード)の更新が要る。
- 規模(概算): 本番 約 -330 行(`literal_detect_fsm.rs` は本番 約 530 行のうち残るのは `per_vk_recovery_params`・`emit_recovery_actions` の約 120 行)+ `warm_send_plan` 約 -60 行、テスト 約 -390 行。`tuning` 定数 `GJI_CANDIDATE_VETO_CAP_MS` が 1 つ減る(complexity-budget の返済に使える)。

### MB-03 `TsfGate` の `Probing` / `Ready` と `on_ready`(MB-02 の連鎖)
- 場所: `tsf/tsf_gate.rs`(`GateEvent::ProbeComplete`、`TsfGateState::{Probing, Ready}`、`on_ready`)、`output/mod.rs:1203`(`on_tsf_probe_ready`)、`output/mod.rs:636-640`。
- 生きているか: `TsfGateState::` の読み手を数えた。`git grep -n "TsfGateState::\|gate_is_bypass" -- crates/awase-windows/src | grep -v tsf_gate.rs`: `PendingWarmup`(`output/mod.rs:601`)、`Bypass`(`probe_io.rs:72`)、`Probing`(`vk_send.rs:636`、MB-02 の入力)の 3 つだけ。`Ready` を読む本番コードは 0 件。MB-02 を消すと `Probing` の読み手も消え、状態は「保留中(PendingWarmup)/ Bypass / Tsf 確定」の 3 値で足りる。
- 撤去した場合に変わる挙動: なし(状態のラベルだけ)。`TsfConfirmed` から `Probing` へ行く遷移を `Tsf` 1 状態にし、`ProbeComplete` を消す。
- 擬似 IME: `TsfGate` は ungated(`tsf/mod.rs:49`、doc に「Linux でも常時実行」とある)で、状態機械そのもののテストは Linux で既にある。足りないのは、フォーカス遷移と `injection_hint` の再計算の順序を流す窓・フォーカスのモデル(末尾 3)。
- 判断: **E(MB-02 の後)**。規模 約 -40 行+テスト。再発ファミリー: focus 遷移(`tsf_gate` は表に無いが focus/ と同じ領域)。

### MB-04 `EVENT_OBJECT_NAMECHANGE` の WinEvent フックと `focus_namechange`
- 場所: `tsf/win_event_obs.rs:60-77`(`SetWinEventHook(EVENT_OBJECT_NAMECHANGE, .., 0, 0, WINEVENT_OUTOFCONTEXT)`)、同 `:147-153`(コールバック)、`tsf/observer.rs:79-87`(`focus_namechange: ChangeCounter`)。
- 解いていた問題: BUG-024 の NameChangeWait(WezTerm の title 変更を GJI 応答の代理にする)。BUG-024.md は「per-VK confirm に置き換わり解決済み」。
- 生きているか: **書き込み専用**。`git grep -n "focus_namechange" -- crates | grep -v win_event_obs.rs` → 出るのは `observer.rs` の doc 3 行・フィールド宣言・初期化の 4 件だけで読み手はない(フィールドの doc 自身が「現在は write-only」と書く)。コールバックの副作用は `class.contains("CASCADIA")` のときの `tracing::debug!("[tsf-settle] OBJ_NAMECHANGE ..")` のみ(実機ログ 13 件、`tools/` 以下の `.py`/`.ps1`/`.yml` に `tsf-settle` を読むものは無い: `git grep -n "tsf-settle" -- tools .github` → 結果ファイル内のログ行だけ)。
- 重複: なし(読み手がいない)。
- 撤去した場合に変わる挙動: ログ 1 種が消える。実行時には、NAMECHANGE は全プロセスの OBJID_WINDOW 名前変化(タイトル変更が頻繁な端末・ブラウザを含む)ごとに awase のメッセージループ上でコールバックが走る(`GetClassNameW` を毎回呼ぶ)ので、その負荷が消える。計測はしていない(未確認)。
- 擬似 IME: 不要(観測の追加・削除だけで判断に出ない)。
- 判断: **E**。再発ファミリー: IME belief(観測)の周辺だが belief に入らない。規模 約 -35 行(`tsf/win_event_obs.rs` の NAMECHANGE ブロック、`ChangeCounter` が他で使われていなければ型も)。SHOW/HIDE フック(`gji_candidate_visible`、per-VK が読む)は別物で残す。

### MB-05 `session_expired` と `COMPOSITION_TIMEOUT_MS`(`assess_warmth`)
- 場所: `awase-windows-plans/src/state/warm_send_plan.rs` の `plan_warmth`(`session_expired`、`WarmthReason::WarmSessionExpired`)、`output/mod.rs:859-877`(`assess_warmth`)、`awase-windows-base/src/tuning.rs:125`(`COMPOSITION_TIMEOUT_MS = 2000`、`measured(pending = true)`)、`vk_send.rs:250-277`(Chrome)・`:422-448`(TSF)の `session_expired` 分岐と `ColdWarmupSequence::run_start(session_expired, ..)`。
- 解いていた問題: 「warm と見なしていても 2 秒以上空けば composition が終わっている」(コメント: 変換確定待機の最大時間)。
- 生きているか: **恒久的に偽**。`Output::send_keys` は先頭で `self.mark_send()`(`output/mod.rs:756`)してから各アクションを送るので、`send_romaji_as_tsf`/`send_romaji_batched` 内の `assess_warmth` が読む `ms_since_last_send()` は ≈0ms。`send_keys` 自身が「mark_send() より前に elapsed を読む。内部では常に ~0ms を返す」と `prev_elapsed_ms` を別に持つが、これは `tracing::debug!` にしか渡されない(`output/mod.rs:740-749`)。
  - 実機ログ: `grep -rhoE "\[tsf-send\] warm=[a-z]+ elapsed=[0-9a-z]+ms" tools/e2e/ime_key_matrix/results | sed -E 's/warm=[a-z]+ //' | sort | uniq -c` → `32 [tsf-send] elapsed=0ms`。同じログに Tsf モード送信で `prev_elapsed=14297`・`35609`(ms)の行がある(`grep -rhE "send_keys: mode=Tsf" ... | grep -oE "prev_elapsed=[0-9]+"`)。`session_expired=true` は 0 件。
  - 残る到達経路は `flush_raw_tsf_literal_romaji`(`send_keys` を経由せず mark_send もされない)のみで、literal 回収の再送なので回収から数百 ms 以内に呼ばれる。2 秒を超える経路は見つけていない(未確認)。
- 重複: 4 つある「idle で cold と見なす」時計の 1 つ(MB-12 参照)。
- 撤去した場合に変わる挙動: なし(現状の挙動をそのまま残す削除)。**逆に「elapsed を正しく渡す直し」をすると、2 秒空いた後の最初の打鍵が全て cold パス(per-VK confirm)になり挙動が大きく変わる**ので、直すのではなく消す。
- 擬似 IME: 不要(定数偽)。
- 判断: **E**。再発ファミリー: warmup(`warm_send_plan.rs`)。`architecture_guard.rs:8363,8369,8425,8435,8447` が `composition_timeout_ms` の配線を固定している(更新が要る)。`tuning` 定数が 1 つ減る。規模 約 -50 行(本番)。

### MB-06 `RuntimeOutbox` / `RuntimeRequest::StartTsfProbe`
- 場所: `runtime/outbox.rs`(46 行)、`output/mod.rs:184,312,322`(`runtime_outbox`、`take_pending_requests`)、`output/vk_send.rs:316-318`(push)、`runtime/mod.rs:2316-2336`(`drain_runtime_requests`)、`runtime/message_handlers.rs:786,1852`(2 呼び出し)。
- 解いていた問題: H-4-b(`Output` が `with_app` を再入せずにタイマーを起動する)。
- 生きているか: **重複**。バリアントは `StartTsfProbe` 1 つ、push は Chrome cold パスの 1 箇所(`git grep -n "RuntimeRequest::" -- crates`: push 1・消費 1)。消費側がやることは `output.pending_tsf_timer()` を `apply_timer_command` するだけで、これは `platform.rs:961`(`drain_output_post_send_effects` 末尾)が全送信経路で既にやっている。`vk_send.rs:307-308` のコメントも「`WindowsPlatform::send_keys` が `pending_tsf_timer()` で TIMER_TSF_PROBE を起動する。`StartTsfProbe` は belt-and-suspenders」と書く。`flush_raw_tsf_literal_recovery`(`send_keys` を通らない経路)も `platform.rs:855-860` が `drain_output_post_send_effects` を呼ぶので同じ。
- 重複・統合先: `drain_output_post_send_effects`(`platform.rs`)。
- 撤去した場合に変わる挙動: 同じ 10ms タイマーの `Set` が 1 回減る(後から呼ぶ Set は周期の位相をずらす程度)。Chrome(Vk モード)cold の probe 起動は `platform.rs:961` が担う。
- 擬似 IME: 不要。タイマー設定の「どこで 1 回呼ぶか」は源の走査ガードで足りる。
- 判断: **E**。ADR-156 は `RuntimeOutbox` を 5 つのキュー/待避機構の 1 つに数えているが、統合しない決定の対象は「解放条件」の話で、1 バリアントの重複消費は別。再発ファミリー: defer/replay キュー(`runtime/outbox.rs` は表に載っている)。規模 約 -70 行。

### MB-07 `CompositionOutput` 実装と `composition_output()`(呼び出し元ゼロ)
- 場所: `src/platform.rs:5-46`(`PlatformColdReason`、`CompositionOutput`)、`src/platform.rs:183`(`composition_output` 既定実装)、`awase-windows/src/platform.rs:1114`(override)、`output/mod.rs:1301-1330`(`impl CompositionOutput for Output`: `send_romaji`・`send_kana_char`・`is_composition_warm`・`mark_cold`・`on_focus_changed`)。
- 解いていた問題: ADR-030(macOS/Linux でも同じ抽象を実装してもらう)。
- 生きているか: **呼び出し元ゼロ**。`git grep -n "composition_output()" -- crates src` → 定義 2 つ(`src/platform.rs:183`、`platform.rs:1114`)と ADR-030 の文章のみ。`git grep -n "send_kana_char\|CompositionOutput" -- crates src` → 実装側(`output/mod.rs`)と trait 定義のみ。`awase-linux`・`awase-macos` は実装していない(grep 0 件)。`PlatformColdReason` の使い手も `mark_cold`(上の未使用実装)のみ。実際の送信は `OutputSession::sender()`(`output/sender.rs`)が使う別のディスパッチで、`CompositionOutput::send_romaji` はそれと同じ分岐を重複して持つ。
- 撤去した場合に変わる挙動: なし。
- 擬似 IME: 不要。
- 判断: **E**(小)。ただし ADR-030 は「他 OS の実装者向けの拡張点」と明記しているので、所有者に「macOS/Linux でこの trait を使う予定が今もあるか」を確認してから。規模 約 -60 行。

### MB-08 診断だけの非同期読み(cold の Chrome 分岐の `spawn_local`)
- 場所: `output/vk_send.rs:321-335`(`send_romaji_batched_gated` の cold 分岐の `spawn_local`、`[cold-diag]` のログのみ)。同型で `vk_send.rs:595-611`(`[h1-send]`、warm 送信ごと)、`output/probe_io.rs:480-495`(`[h1-send]`、cold の Tsf 送信)。
- 生きているか: ログ専用。Chrome 分岐のブロックは `get_ime_conversion_mode_raw_timeout_async(50)` を読んで `tracing::debug!` するだけで戻り値を使わない(`vk_send.rs:322-334`)。他の 2 つも同じ。
- 重複: TSF の cold(`cold_warmup.rs:78`)は同じ読みの後に conv 書き込み(ROMAN ビット保証)を行うので**書き込みの有無で Chrome 分岐と非対称**。意図的か未確認(MB-08 の範囲外、所有者に確認する点)。
- 撤去した場合に変わる挙動: 実行時の副作用は、`offload_unsafe`(ワーカースレッド)から対象窓へ `SendMessageTimeoutW` を送る 1 往復がなくなること。ADR-188・ADR-182・ADR-100 が `[cold-diag]`/`[h1-send]` の行を引用して分析している(ログ読みの根拠として使われている)ので、撤去は診断資産の削減。
- 擬似 IME: 不要。
- 判断: **Chrome cold 分岐の `[cold-diag]` だけ E**(戻り値を使わず、ADR で引用されている `[cold-diag] pre-send conv=` は TSF 側 `cold_warmup.rs:88` にも同名で残る)。`[h1-send]` の毎送信読みは ? → MB-17。

### MB-09 `GjiFsm` の pending 影コピー(`PendingInput` / `GjiAction::SendInput`・`SendInputDirect`・`DiscardPending`)
- 場所: `tsf/gji_fsm.rs:58-80`(`PendingInput`、`#[allow(dead_code)]`)、`:262-279`(アクション)、`GjiState::OnCold.pending`・`ComposingWarmup::AwaitingProbe.pending`、`platform.rs:582`(受け側、`SendInput`/`SendInputDirect` を `{ .. } => {}`)・`:586-589`(`DiscardPending`)。
- 生きているか: `platform.rs` の受け側は `SendInput`/`SendInputDirect` を `{ .. } => {}` で捨て、`DiscardPending` は `tracing::debug!` だけ。フィールド doc も「shadow tracking 専用、フィールドはテストでのみ検証される」と書く。`KeyInput` の実効は 2 つ(Medium/Long の `NotStarted` で `StartProbe` を出す、`OnWarm` で LongIdle タイマーを再設定する)で、`PendingInput` の中身は使われない。
- 撤去した場合に変わる挙動: ログ 1 種(`DiscardPending`)と `pending` 件数の WARN(`ImeOff with N pending`)。`StartProbe`/`CancelProbe`/状態遷移は変わらない。T13 は `SendInput` 系 variant だけを指していたが、`pending: Vec` の追跡全体(約 80 行)と `DiscardPending`/`PendingDiscardReason` も同じ理由で不要になる。
- 注意: ADR-103 決定 5-a が「pending を破棄したことを明示的な行為として記録する」を意図的に決めている。撤去は ADR-103 の修正を伴う。
- 擬似 IME: `gji_fsm.rs` は ungated で Linux のテストがある(状態遷移の等価性は既存テストで足りる)。
- 判断: **E?(ADR-103 5-a の意図を所有者に確認)**。再発ファミリー: warmup(`tsf/` 全般)。

---

## C: 統合できるもの

### MB-10 `TsfProbeCoro`(Chrome、`probe_fsm.rs`)と `GjiWarmupCoro`(TSF、`gji_warmup_coro.rs`)
- 場所: `probe_fsm.rs` の `tsf_probe_coro_body`、`gji_warmup_coro.rs` の `gji_coro_body`、共有部は既に `ProbeCoroState`・`run_per_vk_confirm`。生成元は `vk_send.rs:312`(Chrome)と `vk_send.rs:458`(TSF)。
- 生きているか: どちらも生きている。MB-01 を消すと両者の差は「Phase 1 で 1 tick 待つ → `plan` を決める → per-VK(`target` だけ違う)または `Transmit` 1 回」に縮む。差分は 3 つ: (1) TSF は `decide_transmit_plan` を通り eager Unicode 送信がありうる(MB-14)、(2) TSF は `nc_fired`/`gji_settled` を `probe.outcome()` から作る、(3) Chrome は `OutputActiveGuard` を生成時から持ち TSF は `gji_begin_probe_guard` で別に持つ(`vk_send.rs:450`)。
- 統合先: `GjiWarmupCoro` に `target` を足して Chrome 側の本体を持たせる。MB-14 を先に決めれば `decide_transmit_plan` が `needs_literal` 1 項だけになり統合が簡単になる。
- 変わりうる挙動: Chrome×GJI の cold は現在 `env.gji_active` だけで per-VK に入る(`is_tsf_mode`・`nc_fired` は見ない)。統合で TSF 側の `needs_literal` 式(`!nc_confirmed && is_tsf_mode && gji_active`)を共有すると、Chrome で `nc_confirmed`(= GJI モニタ不健全 or settled)のとき per-VK を外す分岐が新しく入る。**そのまま統合せず、Chrome は `needs_literal = gji_active` を保つ引数にする**。
- 擬似 IME: 検証できない(末尾 1・2)。BUG-27(委譲漏れ)の類型を避けるため、統合は MB-01 の後に 1 PR で。
- 判断: **C(MB-01 の後)**。再発ファミリー: warmup。規模 約 -120 行。

### MB-11 `DeferGate::Enforced`/`Exempt` の defer 側(`defer_if_probe_in_flight_recovery_exempt`、`plan_blocking` の `check_raw_recovery`)
- 場所: `output/vk_send.rs:24-52`、`output/mod.rs:900-990`(`defer_if_probe_in_flight`・`..._recovery_exempt`・`probe_or_recovery_block_reason`)、`awase-windows-plans/src/state/deferred_gate_plan.rs`(`BlockingFacts.check_raw_recovery`)。
- 生きているか: Exempt が飛ばすのは「`raw_recovery_owns_deferred()`(= `RAW_TSF_LITERAL.backs != 0 || romaji 非空`)を見ない」ことだけ(defer 側)。`flush_raw_tsf_literal_recovery` は `flush_raw_tsf_literal_backspaces()`(`backs.swap(0)`、`tsf/output.rs:116`)→ `flush_raw_tsf_literal_romaji()`(`mem::take`)の順で再送するので、再送の時点で `raw_recovery_owns_deferred()` は常に偽。この区別が必要だった理由(無関係な別 give-up の `pending_gji_reinit` が Polling 中、BUG-74 追補3)は ADR-212 P3 の reinit 撤去で消え、`raw_recovery_owns_deferred` も「reinit の pending を見ていた頃の名残で `self` を使わなくなった」と自分で書く(`output/mod.rs:1163-1165`)。
- 変わらないもの: **drain-before-send 側の Exempt(再送が sibling mora を追い越さない)は生きている**(ADR-128)。撤去するのは defer 側の分岐だけ。
- 統合先: `defer_if_probe_in_flight` 1 本(origin だけは `RecoveryResend` の分類に残す)。
- 擬似 IME: 検証できない(`output/` は windows のみ)。純粋部分(`deferred_gate_plan`)は Linux で全数表がある。
- 判断: **C(低優先)**。効果は約 -40 行で、`architecture_guard.rs:8238-8300` の固定と ADR-123/128 の文言を直す手間のほうが大きい。再発ファミリー: defer/replay キューの解放条件(表のとおり「窓口の片方だけ直すと再発」型)なので、やるなら defer 側・drain 側の両方を 1 PR で見ること。

### MB-12 「idle で cold と見なす」時計が 4 つある
- 場所と内容:
  (a) `assess_warmth` の `session_expired`(最後の送信から 2000ms)— MB-05 で E。
  (b) `GjiFsm` の LongIdle タイマー(`OnWarm` で `KeyInput` ごとに再設定、Vk 5 秒 / Tsf・Unicode 10 秒、`gji_fsm.rs:1031` `long_idle_ms_for`)→ `OnCold(Long)`。実機ログで `LongIdle timeout` 18 件(生きている)。
  (c) `ColdKind::classify(gji_idle_ms)`(GJI の I/O 無通信 7 秒/10 秒)をフォーカス変更・`ImeOn`・`CompositionReset` の時点で評価(`gji_fsm.rs:121-132`)。
  (d) warm 送信の `is_long_idle`(GJI I/O 無通信 `LONG_IDLE_MS`)で LiteralDetect の設置を見送る(`vk_send.rs:626`)— MB-02 で E。
- 重複: (b) は「最後の awase の KeyInput からの時間」、(c)(d) は「GJI 自身の I/O からの時間」で、観測元が違う。cold 判定を (c) の「送信時点で `ColdKind::classify(gji_idle_ms())`」に寄せれば (b) のタイマーと `OnWarm → OnCold(Long)` 遷移は不要になりうる。ただし (b) には Vk/Tsf で閾値が違う(5s/10s)という調整があり(`CHROME_LONG_IDLE_MS` は `measured(pending=true)`)、観測元の差が実害に効くかは分からない。
- 判断: **? → MB-05、MB-02 で (a)(d) を先に消してから、(b) と (c) の統合を再評価**。測るもの: Chrome×GJI で「最後の awase 出力から 5〜7 秒空き、その間 GJI の I/O がある(別アプリ経由など)」ときの最初の打鍵の結果(wt-probe.yml / `tools/e2e/ime_key_matrix` の idle 系)。再発ファミリー: warmup / IME belief(`gji_fsm.rs`)。

### MB-13 `ImeModeFsm`(`tsf/ime_mode_fsm.rs`)と `ImeModel` の入力モード
- 内容は T21 と同じ。補足の事実: `ImeModeFsm` の更新元は `output/conv_actuation.rs:155`・`platform.rs:1194`・`runtime/key_pipeline.rs:2727`(`unconfirm`)・`probe_io.rs:306`・フォーカス時 hint(`platform.rs:673-700`)。読み手は MS-IME の confirm-then-transmit ゲート(`vk_send.rs:536`、`MsImeReadyCoro`)と Unicode 観測の設置条件(`output/mod.rs:792`)。MS-IME ゲートは「NATIVE を IMC で確認済みか」を問い、`ImeModel` の確信度(High/Medium)は別の軸で作られているため、そのまま置き換えると BUG-13/BUG-58/BUG-59 が再発する(BUG-59 は「hint で confirmed を立てると未準備で送る」)。
- 判断: **? 実機要**。測るもの: MS-IME 本体 × WT の OFF→ON 直後(BUG-13 の再現、`sc-offrca-*`・`wt-probe.yml`)で、`ImeModel` の確信度だけで「準備完了」を判定した場合と IMC の NATIVE 確認を比べる。擬似 IME に `Grid::MsimeNative` の格子があるので、IMC 読み戻しの遅延(Q3 `set_readback_lag_ms`)を入れて確認の待ち時間を模擬できれば部分的に検証できる(末尾 5)。

---

## ?: 要実測・要判断

### MB-14 eager Unicode 送信と `PendingGjiConfirm`(`ColdKind::Medium` / `forces_prepend_f2` も一緒に)
- 場所: `decide_transmit_plan` の `used_eager_path`(`probe_fsm.rs:150-166`)、`output/probe_io.rs:84-93`(`set_last_unicode_transmit_ms`)、`output/vk_send.rs:565-585`(`is_post_unicode_pending` で次の warm キーも Unicode に強制)、`warm_send_plan::is_post_unicode_pending`、`ColdKind::Medium`(`MEDIUM_IDLE_PROBE_MS`)、`ProbeParams.forces_prepend_f2`。
- 到達条件: `used_eager_path = true` になるのは `decide_transmit_plan` の `nc_confirmed` 枝で `forces_prepend_f2`(Medium/Long cold)のとき(`initial_used_eager` は全呼び出し元で偽: `vk_send.rs:431` の `let used_eager_path = false;`)。`nc_confirmed` は `nc_fired || (confirm_key_hint && gji_settled)`、`nc_fired = !((!settled || is_ime_init_cold) && monitor_healthy)`(`gji_warmup_coro.rs:92-122`)。probe は 1 tick 後(約 10ms)に評価するので Medium/Long cold(GJI が 7 秒以上静止)で `settled`(その間に GJI の I/O があったか)はほぼ偽。つまり eager Unicode に入るのは「Medium/Long cold ∧ GJI モニタが不健全(`gji_monitor_ok == false`)」の組み合わせで、通常の GJI 接続では起きにくい。Mozc 系は BUG-208 の `gji_evidence_absent` で除外済み。
- 実機ログ: `[post-unicode] PendingGjiConfirm 開始` は 94 ディレクトリで 0 件。ただし Medium/Long cold の母数が不明(`transmit-plan` 2 件のみ)なので「起きない」とは言えない。
- 撤去した場合: 上記の組み合わせで、かなを Unicode で直接送る代わりに VK(per-VK)で送るようになる。BUG-208 は「eager Unicode が GJI の確認手段を持たず入力中の文字を作れない」という方向の問題で、本経路の縮小と整合する。
- 判断: **? 実機要**。測るもの: R2 の不具合報告(`bug-report-fetch` / `bug-report-latest`)で `[post-unicode] PendingGjiConfirm 開始` の件数と、その窓(GJI モニタ不健全の理由)。0 件なら E に上げる。再発ファミリー: warmup / cold-start。連鎖して消える: `ColdKind::Medium`、`forces_prepend_f2`、`env.deferred_pending`、`PostUnicodeFacts`(約 -150 行、概算)。

### MB-15 `UnicodeLiteralObserverFsm`(Unicode モードで GJI write を観測して Tsf に自動昇格)
- 場所: `output/mod.rs:780-815`(設置)、`tsf/warmup/unicode_literal_observer.rs`(95 行)、`platform.rs:1003-1009`(`request_unicode_observation`)。
- 生きているか: 設置は `KeyAction::Romaji` の分岐の中だけ(`output/mod.rs:780-815`)。`KeyAction::Romaji` を作るのは `YabValue::Romaji { kana: None }`(拗音など 1 文字に収まらない出力)だけ(`src/engine/nicola_fsm.rs:107`)。`kana: Some` は `Char` になる(同 `:104`)。実機ログ: Unicode モードの送信 187 件は全て `Char`、`Romaji` は 0 件、`[unicode-obs]` も 0 件。つまり「未学習クラスの Unicode モード窓で TSF ネイティブを自動学習する」機構は、拗音を出力した最初の機会にだけ動く狭い入口になっている。
- 判断: **? 判断が要る**。(a) 意図どおりなら残す(拗音が来れば学習される)、(b) 意図が「最初の数打鍵で学習」なら入口が漏れている(バグの可能性)、(c) 学習の実績が無ければ撤去。測るもの: 不具合報告 JSON の learned `injection_mode_store` の更新履歴(`UpgradeToTsf` の journal)の件数。`tsf/warmup/unicode_literal_observer.rs` の冒頭 doc にある観測専用 `log_composition_probe("unicode-obs-himc-check")` は「実機ログが集まったら」と書いて 2026-08-03 から未回収(別途 `docs/experiments.md` の合格基準に照らして結論を出す)。

### MB-16 `EVENT_OBJECT_IME_SHOW/HIDE/CHANGE` のフックと `ime_composition_active`
- 場所: `tsf/win_event_obs.rs:79-96,198-228`、`tsf/observer.rs` の `ime_composition_active`・`ime_show_seq`・`ime_change_seq`。
- 生きているか: **読み手は多い**(`ime_composition_active_now()` が `key_pipeline.rs` 5 箇所・`message_handlers.rs:301`・`executor.rs:636`・`lang_detect_shadow.rs:264`・`ime_controller.rs` の view など、約 12 箇所。ADR-107/109/114/188/255/263 が「IME 非依存の汎用シグナル」として前提にしている)。しかし ADR-043:184 は「`EVENT_OBJECT_IME_*` は GJI TSF モードでは発火しないことを実機確認済み」、ADR-221 は「MS-IME の未確定は IME ウィンドウを作らず IME_SHOW が出ない可能性」と書く。実機ログ(94 ディレクトリ、GJI 中心): `[ime-obj] IME_SHOW`/`IME_HIDE`/`IME_CHANGE` は 0 件。GJI では実質常に偽の信号を、複数の判断が「汎用」として使っている疑い。
- 判断: **? 実機要**。測るもの: MS-IME 本体 × メモ帳(IMM32 経路)・Chrome・WT で、未確定文字がある間に IME_SHOW が出るか(出れば MS-IME 専用の信号として残る、出なければ MB-04 と同様に撤去し、`composing` は GJI の候補窓可視(`gji_candidate_visible_now`)など別の信号に寄せる)。`tools/e2e/ime_key_matrix` の MS-IME 本体構成で `[ime-obj]` を数えるのが最小。IME belief 系(`ime-belief-architecture.md`)の判断入力なので、データなしに消さない。

### MB-17 毎送信の `[h1-send]` 診断読み(warm の `send_romaji_as_tsf_warm`)
- 場所: `output/vk_send.rs:595-611`。送信のたびに `get_ime_conversion_mode_raw_timeout_async(10)` を `offload_unsafe` で投げ、ログ 1 行を出すだけ。
- 注意: ADR-182 が「awase 自身の送信が hook を 100ms 級で止めている(直前の `[h1-send]` から約 114ms)」と記録している(対象外として残された)。診断読みが原因かは未切り分け。
- 判断: **? 実機要**。測るもの: `[h1-send]` を止めたビルドと比べた hook 処理遅延(`delay=` ログ、ADR-182 の再現手順)。診断資産(ADR-100/182/188 が引用)なので、止めるなら journal 側へ置き換えて残す。

---

## K: 残すもの(理由)

- **`pending_deferred`(probe 中に届いた後続 VK の単一キュー)**: 1 回の `send_keys` が複数の `Romaji`/`Char` を含むとき、2 つ目の送信が 1 つ目の probe の進行中に呼ばれる(`send_keys` 内の再入)ので、hook 層の `OUTPUT_GATE`+`INPUT_DEFER` だけでは足りない。ADR-156 の結論(大規模統合は不採用)とも整合。
- **`OutputActiveGuard`/`OUTPUT_GATE`(参照カウント)**: `app/mod.rs:724`・`key_pipeline.rs:165`・`message_handlers.rs:596,716` が読む。MB-01 で `literal_detect_guard`(GjiWarmupCoro 内)が 1 つ減る。
- **raw TSF literal 回収と `consecutive_count`(再送は 1 回、2 回目は BS のみ)**: BUG-27/33/74 の現役。`RAW_TSF_LITERAL`(`Mutex<String>`+atomics のグローバル)を `Output` のフィールドにする整理は可能だが機構の撤去ではない。
- **per-VK confirm(`run_per_vk_confirm`)・epoch fencing(`visible_fencing_verdict`)**: 実機ログ上で唯一動いている確認経路(`per-VK[..] confirmed` 6 件)。
- **`MozcComposeCheckFsm`(ADR-270)・`GjiMonitor`・`tip_detector`**: 新しい or 観測の源。
- **`MsImeReadyCoro` + `start_ms_ime_ready_poll` + `ms_ime_gate_give_up` + `confirm_gate_deadline_override_ms`**: BUG-13/49/58/59 の現役。MB-13 の?が決まるまで残す。
- **`deferred_engine_timers`(`message_handlers.rs:713-722`)・`InputDeferQueue`**: NICOLA の同時打鍵判定(BUG-77)のための窓口で、ここでの撤去は IME 側と無関係。
- **`EVENT_OBJECT_SHOW/HIDE` フック(候補ウィンドウ)**: per-VK が読む `gji_candidate_visible`・`gji_candidate_show` の源(実機ログ SHOW 414 件/HIDE 416 件)。全プロセスの SHOW/HIDE が対象でコールバック負荷はあるが、`idThread` で絞る手段は GJI の候補窓がホスト側プロセスで作られる関係で未確認。
- 古い doc の整理(機構ではなく記述): `tsf/warmup/mod.rs:24`・`tickable_fsm.rs:13,86` に撤去済みの `UnicodeColdWarmupFsm` が残っている(ファイルも `mod` 宣言も無い)。`probe.rs:136,158` のログ文言「VK_DBE_HIRAGANA warmup を先行送信する」も撤去済み機構の説明。

---

## 擬似 IME に足すと検証できる範囲が広がるもの(優先順、規模は概算で未確認)

現状の閉ループは `ImeStateHub`・`ImeModel`・`KeyEffectKeymap`・drift correction・Engine の活性遷移までを呼ぶ純粋な層で、`tests/support/harness.rs` 冒頭が「写していない: 通過マーク、`ImeApplyRequested`/applied の往復、ForceGuard、**TSF warmup**、Engine の `on_input`」と明記している。`output/`(`vk_send.rs`・`probe_io.rs`・`tsf_warmup_coord.rs`)全体は `#[cfg(windows)]`(`lib.rs:90-91`)なので、本調査の機構はすべて Linux のテストに存在しない。ungated で Linux のテストがあるのは `tsf/gji_fsm.rs`、`tsf/tsf_gate.rs`、`tsf/probe.rs`、`tsf/warmup/{probe_fsm,literal_detect_fsm,probe_coro_state,tickable_fsm,mozc_compose_check_fsm,ms_ime_ready_coro}`、`ime_mode_fsm.rs`、`input_defer.rs`、`warm_send_plan`/`deferred_gate_plan`/`relay_plan`(plans crate)。

1. **TSF 窓の模型(最優先、約 600〜900 行)**: `ProbeIo` を実装する擬似窓。送信された VK 列を記録し、(a) composition が成立するか/最初の 1 文字がリテラルになるか(cold 窓・Chrome 再初期化 ~114〜326ms、BUG-002 の実測値)、(b) GJI の I/O バイト(`gji_write_bytes`)と候補ウィンドウ SHOW/HIDE を `TSF_OBS` 相当に反映、(c) 仮想時計(`crate::clock::advance` が既にある)で `TIMER_TSF_PROBE` を 10ms ごとに `step_probe` 相当へ送る。これがあれば MB-01・02・10・14 の「撤去前後で出力列が同じ」を差分ゼロで示せる。**前提として `dispatch_probe_actions`・`ProbeIo` トレイトを `#[cfg(windows)]` の外へ出す**(トレイトと関数は Win32 型を使わない: `VkSequence`・`VkMarker`・`WarmupOutcome` の ungate が要る。約 100 行の移動)。
2. **出力層の SendInput 列の記録(約 150 行)**: `KeyInjector`/`send_input_safe` を差し替え可能にして、`send_keys` が実際に出す KeyDown/KeyUp の列(バッチ分割・unicode・VK・marker)を取る。MB-01・02・05・06・10・14 の前後比較の出力側になる。現状は `ime_key_sequence_golden` が IME ON/OFF の戦略選択までで、ローマ字送信は対象外。
3. **窓・フォーカスのモデル(約 300 行)**: `TsfGate`(PendingWarmup → Probing/Bypass)と `injection_hint`/`injection_mode_for`・`update_injection_mode` の再計算タイミング。MB-02/03 の「Probing と Tsf モードが同時に成り立つか」を、フォーカス遷移の順序を変えて機械的に探せる。
4. **hook 層のキー保留と再配送(約 250 行)**: `OUTPUT_GATE`・`INPUT_DEFER`・`WM_DRAIN_OUTPUT_QUEUE` の順序(MB-11 と K の `pending_deferred`/`deferred_engine_timers`)。「回収の再送が sibling mora を追い越さない」(ADR-128)を差分ゼロで確認できる。
5. **IMC 読み戻しの遅延と MS-IME 本体の格子**: `Grid::MsimeNative` と Q3(`set_readback_lag_ms`)は既にある。`ImeModeFsm` の `confirmed` が立つまでの待ちを模す(約 100 行)と MB-13 の一部が検証できる。
6. **GJI モニタの健全性スイッチ(約 40 行)**: `gji_monitor_ok` の真偽と `gji_last_io_ms` のずれを入力にできるようにする。MB-14 の到達条件(モニタ不健全)を再現できる。
7. **MS-IME 本体の `EVENT_OBJECT_IME_SHOW` 発火の有無**: これは模型ではなく**実測**(MB-16)。擬似 IME では決まらない。

実機でしか決まらないものと使える既存の構成: MB-02/14 は R2 の不具合報告ログの集計(`bug-report-fetch`)、MB-12 は `wt-probe.yml`・`tools/e2e/ime_key_matrix` の idle 系、MB-13/16 は MS-IME 本体の `e2e-ime.yml`(`sc-offrca-*` を含む)。
