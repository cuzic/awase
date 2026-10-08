---
id: ADR-245
title: |-
  プロセスをまたぐフォーカス移動で半角英数トグルを、離れるときでなく「戻ってきたとき」に復元する(BUG-193)
summary: |-
  BUG-193: 窓 A で持続半角英数(左 Shift 単独タップ)にしたまま別プロセスの窓 B へ移ると、離脱時の強制復元は IMC 書き込みが世代の bump で必ず中断し、
  F2/VK_DBE_HIRAGANA は移動先 B に届く。A は半角英数のまま残り、awase のトグルフラグだけが下りる(MS-IME 本体・GJI とも CI で 5/5)。
  仕様(ADR-107 の検証項目 4)は「往復しても英数状態が持ち越されない」で、所有者は 2026-10-08 に「A に戻ったらかなに戻す」と判断した。本 ADR は、離脱時には IME へ何も送らず(belief は従来どおり戻す)、A を覚えておいて、A に戻って最初の打鍵の手前で復元する案(B)を定める。
status: |-
  起草・改訂(2026-10-08)。所有者判断=「A に戻ったらかなに戻す」(案 B)。Opus round1(Blocker 3・Must 6・Should 7・Nit 4)を反映済み、round2 の再確認待ち。未決 Q1〜Q5。
related_adr:
  - "ADR-107"
  - "ADR-084"
  - "ADR-212"
---

# ADR-245: 半角英数トグルの復元を、戻ってきたときに行う(BUG-193)

## 事実(確かめたもの)

- 離脱時の強制復元は `ir_notify_focus_changed`(`ime_refresh.rs:441`)が `kp_restore_kana_from_half_width(false)` を呼ぶ。起案時に `owner_gen`/`focus_gen` を捕獲して `spawn_local` するが、同じ同期呼び出しの後半で `gji_on_focus_change` → `on_ime_mode_focus_changed` が両世代を進める(`ime_refresh.rs:694`、`platform.rs:632`)。タスク先頭の世代確認が必ず失敗し、**IMC 書き込みは一度も行われない**(`key_pipeline.rs:2299,2308`)。
- 同期で残るのは SendInput(MS-IME は scan 付き VK_DBE_HIRAGANA、GJI は F2)だけで、配送時点の前面窓(= 移動先 B)に届く。B の belief が閉ならスキップ、開なら B に届く。
- IMC 書き込みだけでは新 MS-IME(TSF)の実モードは英数から戻らない(`key_pipeline.rs` のコメント、2026-07-07 実機)。つまり**旧窓 A が前面でない間、A を確実に戻す手段は無い**。
- CI(BUG-193、`sc-focusrestore-*`、run 37781262341 / 37782133675): A は戻っても 0x10 のまま、戻して打つと素通し(`w`)。awase のトグルフラグは `begin_restore_kana` で下り、戻ってからの 2 回目の左 Shift タップは A を変えない。
- 仕様: ADR-107 の検証項目 4「トグル ON → フォーカス変更 → 戻る、を往復しても英数状態が持ち越されないこと」(BUG-25)。実機検証(Task 9)は未実施のままで、実装が仕様を満たしていない。
- 復元が呼ばれるのはプロセスをまたぐ移動だけ(`advance_focus_tracking` が pid 差で `process_changed` を決める)。同一プロセス内の窓移動は対象外(S-4、Q4)。
- (Opus round1 で裏取り) 世代の bump は `ir_stage_observe` 末尾の Phase 3.7(`ir_post_focus_change_snapshot`)で起き、`on_focus_process_changed`/`enter_focus_scope` は世代を進めない。つまり**プロセス変更の tick で `ir_notify_focus_changed` が捕獲した世代は、離脱でも戻りでも同じ tick の後半で必ず古くなる**。
- `begin_restore_kana()` は旧値を返し、`kp_restore_kana_from_half_width` は旧値が偽だと「already inactive」で送信を全部飛ばす(`key_pipeline.rs:2188`)。
- 復元の末尾で必ず `apply_input_mode_correction(AssumedRomaji{UserHalfWidthAlnumToggleOff})` が走る(`key_pipeline.rs:2443`)。離脱時にこれまで B へ ObservedEisu を持ち越さなかったのはこの補正のおかげ。
- `HwndImeCache` は離脱時(`ir_notify_focus_changed` より前、`focus_tracking.rs:410`)に A の `ObservedEisu` を保存し、戻りで `cache_restore_eisu_guard`(`eisu_recovery.rs:195`)が AssumedRomaji へ洗う。キーは `(pid, class_name)`、TTL は `HWND_CACHE_MAX_AGE_MS`(1 時間)、100ms 未満の滞在は保存しない。
- `ir_notify_focus_changed` は `app_disabled`(既定 mstsc.exe)でも走る(`ir_stage_focus` の中)。現行は A→mstsc で VK_DBE_HIRAGANA が RDP へ届きうる(BUG-78 に反する既存の穴。本設計は離脱時に送らないので塞がる)。

## 案

| 案 | 内容 | 評価 |
|---|---|---|
| A. 離脱時に旧窓へ書く | A の hwnd を捕獲して IMC 書き込み | MS-IME(TSF)には効かない。GJI は IMC が読めない(Imm32Unavailable)。不可 |
| **B. 戻ったとき(最初の打鍵の手前)に復元** | 離脱時は IME へ何も送らない。A を覚え、A に戻って最初の打鍵の手前で復元する | 所有者判断(2026-10-08)で採用。仕様を満たす。B/C への余計な F2 が消える |
| C'. 戻ったら A の英数を正とし、トグルを立て直す(キーは送らない) | 注入ゼロ。窓ごとの IME 状態と一致 | 所有者が不採用(A はかなに戻す) |
| D. 何もしない | 強制復元を消すだけ | A は英数のまま、フラグは下り、2 回目タップは「開始」。不可 |

## 決定(案 B、Opus round1 反映)

1. **離脱(`process_changed`)**: IME へは何も送らない(SendInput も IMC も)。ただし **belief は従来どおり戻す**(`InputModeApplied{AssumedRomaji, UserHalfWidthAlnumToggleOff}`。B の実 IME の観測で上書きできる弱い値)。送信と belief 補正を分けるのが要点で、補正まで消すと B に ObservedEisu が持ち越され B で NICOLA が止まる(round1 B-3)。`toggle_held` を下ろし、戻り待ちへ積む。`shift_conv_guard_gen`/`ime_mode_focus_gen` の bump は従来どおり。
2. **戻り待ちのエントリ**: `{scope: ForegroundScope, kind: 'uses_imc_conv_write'(Enter 時の IME 種別), at: TickMs}`。Enter 時と照合時の両方で **`win32::foreground_scope()` 1 本**を使い、`classified`(フォーカス子 hwnd 系)の値と混ぜない(M-3。UWP は ApplicationFrameHost、Chrome はフォーカス子がタブで変わるため)。容量は小さな固定長(例: 4、古いものから捨てて**ログを出す**)。寿命は既存の `HWND_CACHE_MAX_AGE_MS` を借りる(新しい時間定数は作らない)。`HwndImeCache` とは別物にするが、戻りの順序は「キャッシュ復元 → 最初の打鍵で Resume → AssumedRomaji」で一貫させる(M-4)。
3. **Resume の実行位置(Q1 の答え)**: 戻った窓での**最初の打鍵の手前**(`kp_` パイプラインの先頭、エンジン処理より前)。`ir_notify_focus_changed`/同 tick の `ir_stage_notify` では実行しない——Phase 3.7 の世代 bump で IMC リトライが必ず中断し、`effective_open()`/`active_ime_kind()` が A の値でなく、Alt+Tab/Win の修飾キーのガードで送信が落ち、TSF の準備前になる(B-2、M-1、M-2、S-2)。戻っても打たないなら何も注入しない。判定は打鍵時点の `foreground_scope()` とエントリの一致。
4. **復元本体の分離(B-1)**: `kp_restore_kana_from_half_width` を「トグル旧値の確認(INV-B)」と「OS 書き込み」に分け、Resume は**戻り待ちから取り出せたこと**を `owns_exit` として渡す(取り出しが INV-B の「true→false の遷移 1 回」の役を担う、と ADR-107 INV-B を読み替える)。`toggle_held` を一瞬立て直す案は使わない。関数名と `actuation_call_guard` の許可リスト・`ActuationTarget::capture(` の件数(key_pipeline.rs=3)は変えない形にする。
5. **見送りは取り出しを確定しない(M-1、M-2、M-6)**: Alt/Win 押下中、`effective_open()` が偽、Enter 時と照合時の IME 種別が違う、のとき送らず、エントリを戻り待ちに残す(`Deferred`)。次の打鍵で再評価する。種別が違うときは捨てる(ログ)。純関数の出力は `Nothing | Suspend | Resume | Deferred | Drop` とし、表で固定する。
6. **戻り待ちがある A での最初の操作(S-3)**: 左右 Shift タップは Exit 扱い(エントリを消費して復元)。IME モードキー(shadow toggle・物理モードキー通過・ADR-188 の直接観測)では**送信なしでエントリを捨てる**(ユーザーが選んだモードを後から上書きしない)。
7. **MS-IME**: Resume でも `confirm_gate_deadline_override_ms = now + SHIFT_CONV_GUARD_RELEASE_CONFIRM_MS` を張る(S-5。通常の Exit と同じ。復元関数の外側にあるので明示する)。`note_explicit_ime_action` は復元と同じ順で呼び、`FOCUS_RESYNC`(`focus_tracking.rs:851-865`)の読みと逆転しないことを CI で確かめる。
8. **GJI(INV-B・S-1)**: 取り出しと同時にエントリが消えるので二重送信は無い。ただし VK_DBE_HIRAGANA(F2)が「かなへの SET」であることは ADR-107 §4-c で未確認のまま。Resume は、学習表で VK_DBE_HIRAGANA が「かな SET」と確認できるとき、または通常の Exit が既に同じ前提で送っている範囲(既存の仮定を超えない)に限る。離脱中に A の状態が変わりうるので、寿命(2 の `HWND_CACHE_MAX_AGE_MS`)を超えたエントリは捨てる。
9. **Resume の条件**: 現在の窓が `app_disabled` でない。エンジン OFF 中でも IME の復元は行う(通常の Exit と同じ。Q5 で所有者に確認)。
10. **純粋な核(FCIS)**: `state/half_width_alnum.rs` に、事実(トグル状態・戻り待ち・打鍵時点の前面スコープ・IME 種別・修飾キー・`effective_open`・操作の種類)から計画を決める純関数を置く。殻は事実の収集と実行だけ。新フィールドは `tests/architecture_guard.rs::half_width_alnum_state_fields_are_not_accessed_directly` の FIELDS に足す(S-7)。スコープつきの一回マークは `state/mode_key_pass.rs::ModeKeyPassLatch<S>` に前例があるので流用を検討する。

## 検証

- Linux 単体(`state/half_width_alnum.rs`、同じコミットに入れる): 離脱=Suspend(belief は戻る)、戻り=Resume、別窓=保持、容量超過=ログつきで古いものを捨てる、pid だけ一致=Resume しない、修飾キー押下中・`effective_open` 偽=Deferred、IME 種別不一致=Drop、左右 Shift タップ=Exit、IME モードキー=Drop。
- golden(`tests/golden_scenarios.rs` の scenario_15 系に「離脱→戻り」): 殻を通して「already inactive」でスキップしないこと(B-1 は純関数の表では捕まらない)。
- CI(`sc-focusrestore-*` を `--strict` に格上げ): 合格条件は、A に戻って最初の打鍵で `typed == か`、A の conv が NATIVE、窓 B の conv が不変**かつ B で NICOLA の文字を 1 つ打つと期待のかなが出る**(B-3)、離脱時の SendInput が B に出ていない、戻りの後に「復元 write をスキップ (already inactive)」と「復元リトライ #0 中断」が出ない。構成を足す: Alt+Tab で戻る(M-1。マーカー付き注入で物理扱い)、A→B→A→B→A の 2 往復と A→B→C→A と B で自分もトグル(S-1/M-6)、100ms 未満の瞬間往復、Chrome(TsfNative)・UWP を observe で 1 構成ずつ(M-2、M-3、S-2)。
- fix-requires-evidence: 上の単体・golden・CI で (a) を満たす。BUG-193.md の `fix_commits` を更新する (b)。

## 未決(実測で決める)

- **Q1(解決)**: Resume の位置は「最初の打鍵の手前」(決定 3)。残る確認は `FOCUS_RESYNC` との順序(決定 7)。
- **Q2(解決)**: Resume 時に前面が A かを `foreground_scope()` で再確認する(決定 3)。
- **Q3**: A の 0x0 → 0x10 の遷移の主体。Phase 3.7 の診断スナップショット(`ime_refresh.rs:661-663`)で離脱の前後どちらかを CI ログで切る。
- **Q4**: 同一プロセス内の窓移動(`process_changed` が偽、`ir_notify_focus_changed` も世代 bump も走らない)。トグルと ObservedEisu が移動先にそのまま持ち越される失敗シナリオがある(S-4)。本 ADR の範囲外とするが、CI に `sc-focusrestore-sameproc` を observe で足して実 IME が窓ごとかを測り、結果次第で BUG を起票する。
- **Q5**: エンジン OFF 中の Resume を通常の Exit と同じく行ってよいか(所有者判断)。

## 範囲外

- 同一プロセス内の窓移動の修正(Q4 で測るのみ)。
- 観測起点の書き込みの整理(`docs/tasks/observation-driven-writes-inventory-2026-10-08.md` の A〜C)。本 ADR は個別の欠陥の修正で、整理の方向とは独立。
