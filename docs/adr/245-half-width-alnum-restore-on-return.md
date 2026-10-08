---
id: ADR-245
title: |-
  プロセスをまたぐフォーカス移動で半角英数トグルを、離れるときでなく「戻ってきたとき」に復元する(BUG-193)
summary: |-
  BUG-193: 窓 A で持続半角英数(左 Shift 単独タップ)にしたまま別プロセスの窓 B へ移ると、離脱時の強制復元は IMC 書き込みが世代の bump で必ず中断し、
  F2/VK_DBE_HIRAGANA は移動先 B に届く。A は半角英数のまま残り、awase のトグルフラグだけが下りる(MS-IME 本体・GJI とも CI で 5/5)。
  仕様(ADR-107 の検証項目 4)は「往復しても英数状態が持ち越されない」で、所有者は 2026-10-08 に「A に戻ったらかなに戻す」と判断した。本 ADR は、離脱時には IME へ何も送らず(belief は従来どおり戻す)、A を覚えておいて、A に戻って最初の打鍵の手前で復元する案(B)を定める。
status: |-
  起草・改訂(2026-10-08)。所有者判断=「A に戻ったらかなに戻す」(案 B)・ATOK は送らずトグルを立て直す・エンジン OFF 中も復元する。Opus round1(Blocker 3・Must 6・Should 7・Nit 4)と round2(Blocker 0・Must 5・Should 4)を反映済み、round3 の再確認待ち。未決 Q3・Q4(実測)。
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

## 決定(案 B、Opus round1・round2 反映)

1. **離脱(`process_changed`)**: IME へは何も送らない(SendInput も IMC も)。ただし **belief は従来どおり戻す**: `apply_input_mode_correction(AssumedRomaji{reason: UserHalfWidthAlnumToggleOff}, UserHalfWidthAlnumToggle)`(新しい variant は作らない。`architecture_guard.rs:676` の `InputModeApplied` 構築の件数を変えない)。送信と belief 補正を分けるのが要点で、補正まで消すと B に ObservedEisu が持ち越され B で NICOLA が止まる(round1 B-3)。`toggle_held` を下ろし、戻り待ちへ積む。世代の bump は従来どおり。
2. **戻り待ちのエントリ**: `{scope: ForegroundScope, kind: 'uses_imc_conv_write'(Enter 時の IME 種別), at: TickMs}`。Enter 時と照合時の両方で **`win32::foreground_scope()` 1 本**を使い、`classified`(フォーカス子 hwnd 系)の値と混ぜない(M-3)。**戻り待ちが空のときは打鍵ごとの `foreground_scope()` を呼ばない**(S2-3)。容量は小さな固定長(例: 4、古いものから捨ててログを出す)。寿命は既存の `HWND_CACHE_MAX_AGE_MS` を借りる(新しい時間定数は作らない)。**`tuning.rs` のこの定数の doc に「ADR-245 の戻り待ちの寿命にも使う」と 1 行足す**(S2-1。片方の都合で値を変えたとき、もう片方の意味が黙って変わらないようにする)。`HwndImeCache` とは別の機構(キー・役割が違う)だが、戻りの順序は「キャッシュ復元(AssumedRomaji)→ 最初の打鍵で Resume」で一方向に固定する(M-4、round2 で許容)。
3. **Resume の実行位置(Q1)**: `kp_run_inner`(`key_pipeline.rs:38`)の中で、**`try_hold_key` と ime-off-rescue の後、`kp_stage_idle_conv_check` と `kp_stage_shadow_ime_toggle` の前**に 1 段足す。`enrich_key_role`/`enrich_thumb_key_role` の結果がすでに使え、保留されたキーが同じ関数へ戻ってきても判断が二重にならず、idle-conv-check の読みより先に `note_explicit_ime_action` を打てる。Resume の SendInput はこの段で同期して出るので、その打鍵自体の出力(executor が後で出す)より先に届く。打鍵を defer する仕組みは要らない。`ir_notify_focus_changed`/`ir_stage_notify` では実行しない(Phase 3.7 の世代 bump で IMC リトライが必ず中断する、B-2)。**修飾キー(Shift/Ctrl/Alt/Win)の KeyDown と KeyUp では Resume しない**(M2-1)。戻っても文字を打たないなら何も注入しない。
4. **復元本体の分離(B-1)**: 本体は `kp_restore_kana_from_half_width` に**残す**。先頭の `begin_restore_kana()` の旧値確認だけを引数で切り替える(例: `enum ExitOwnership { FromToggle, FromResume }`。`FromResume` は取り出し済みなので `begin_restore_kana` を呼ばずに `owns_exit = true` とする。取り出しが INV-B の「true→false の遷移 1 回」の役を担う、と ADR-107 INV-B を読み替える)。**OS 書き込みを新しい関数へ切り出さない**: `lints/actuation_call_guard` の `send_input_safe` 許可リストは呼び出し元の関数名で照合し(`lib.rs:70-93`)、`ActuationTarget::capture(` の件数(`architecture_guard.rs:2635`、key_pipeline.rs=3)も変わるため、切り出すと CI が落ちる。
5. **Deferred は修飾キー押下の間だけ(M2-2、M2-5)**: Shift/Ctrl/Alt/Win のいずれかが物理的に押されている間は送らず、エントリを戻り待ちに残す(Shift 押下のまま送ると Shift+ひらがなでカタカナ、Ctrl 押下で Ctrl+F2 になる。判定は `hook::is_physical_key_down` を使い、`modifier_snapshot` と食い違うときは安全側)。修飾キーはそのキーの並びの中で必ず離れるので、Deferred の寿命はそこに限られ、新しい時間定数も回数の上限も要らない。**`effective_open()` が偽・Enter 時と照合時の IME 種別が違う・`app_disabled` のときは Drop**(ログつき。遅れて送って、その間にユーザーが決めたモードを上書きしない)。純関数の出力は `Nothing | Suspend | Resume | Deferred | Drop | ExitOnTap | RebuildToggle`。
6. **戻り待ちがある A での最初の操作(S-3、M2-1、M2-3)**: 左右 Shift タップは Exit 扱い(`on_shift_up` に「現在のスコープに戻り待ちがある」を渡して `ExitRestoreKana`〈`FromResume`、`prepend_synthetic_shift_up=true`〉。エントリを消費)。**「IME モードキー」は VK ではなく役割で判定する**: `enrich_key_role` が IME モードの役割を付けたキー(shadow toggle・物理モードキー通過・ADR-188 の直接観測の対象)は送信なしでエントリを Drop する。**親指キーの役割が付いたキーは対象外(= Resume してよい)**。`F2`/無変換/変換を親指キーに割り当てた構成では NICOLA の同時打鍵であって IME モードキーではない(BUG-115、`kp_stage_execute` の `is_configured_thumb_key`)。
7. **MS-IME**: Resume でも `confirm_gate_deadline_override_ms = now + SHIFT_CONV_GUARD_RELEASE_CONFIRM_MS` を張る(S-5)。`note_explicit_ime_action` は復元と同じ順で呼ぶ。TsfNative では `FOCUS_RESYNC`(`app/mod.rs:643-656`)が最初の対象キーを `kp_run_inner` の前に defer して読むので、順序は resync の読み(ObservedEisu)→ キーの replay → Resume(AssumedRomaji)に固定される見込み。CI ログで resync → Resume の順を確かめる(resync が ObservedEisu を見て能動的な書き込みを起こさないことも)。
8. **GJI(INV-B・S-1・M2-4)**: 取り出しと同時にエントリが消えるので二重送信は無い。ただし GJI のプリセットによっては VK_DBE_HIRAGANA(F2)が**純粋なトグル**で、リポジトリの実測表が ATOK を「0x19→0x10、0x10→0x19 の純粋なトグル」と記録している(`state/key_effect_predictor.rs:1275`、grid 第 3 版)。通常の Exit は「A が 0x10 と分かっている」直後なので安全だが、Resume は離れている間に A が変わりうる(Q3、言語バーの操作)。そこで Resume の直前に予測器(`KeyEffectKeymap::predict_with_override`、`TableKey::Hiragana`)へ 2 つの入力を問う: (i) 開・ObservedEisu(conv 0x10)、(ii) 開・ひらがな(conv 0x19)。**両方とも mode がかな(SET)なら送る**(MS-IME プリセット)。**(ii) が英数になる(トグル)、または予測が `None`(Custom・MsImeNative 等の CannotPredict)なら送らず、`RebuildToggle`**: 所有者判断(2026-10-08)で、A は英数のまま残し、`toggle_held` を A について立て直して belief を ObservedEisu にする(次の左 Shift タップで Exit。キーは注入しない)。HwndImeCache の復元が ObservedEisu を AssumedRomaji に洗うので、立て直しは最初の打鍵の段(決定 3)で行い、洗いより後に belief を書く。MS-IME 本体は IMC の SET と VK_DBE_HIRAGANA の SET なので判定は不要だが、`effective_open()` のガードは外さない(閉でひらがなを押すと開く、`key_effect_predictor.rs:1991`)。寿命(決定 2)を超えたエントリは捨てる。
9. **Resume の条件**: 現在の窓が `app_disabled` でない。**エンジン OFF 中でも IME の復元は行う**(所有者判断 2026-10-08。通常の Exit と同じく、IME の復元はエンジンの ON/OFF と独立)。
10. **純粋な核(FCIS)**: `state/half_width_alnum.rs` に、事実(トグル状態・戻り待ち・打鍵時点の前面スコープ・IME 種別・物理の修飾キー・`effective_open`・キーの分類〈修飾キー/左右 Shift タップ/IME モードの役割/親指キー/その他〉・予測器の答え)から計画を決める純関数を置く。殻は事実の収集と実行だけ。新フィールドは `tests/architecture_guard.rs::half_width_alnum_state_fields_are_not_accessed_directly` の FIELDS に足す(S-7)。スコープつきの一回マークは `state/mode_key_pass.rs::ModeKeyPassLatch<S>` に前例があるので流用を検討する。

## 検証

- Linux 単体(`state/half_width_alnum.rs`、同じコミットに入れる)の表: 離脱=Suspend(belief は戻る)、戻り+文字キー=Resume、別窓=保持、容量超過=ログつきで古いものを捨てる、pid だけ一致=Resume しない、**修飾キーの KeyDown/KeyUp=Nothing**、**Shift/Ctrl 押下中の文字キー=Deferred**、`effective_open` 偽=Drop、IME 種別不一致=Drop、`app_disabled`=Drop、**左右 Shift タップ(戻り待ちあり)=ExitOnTap**、**IME モードの役割のキー=Drop**、**親指キー=Resume**、**予測がトグルまたは None=RebuildToggle**、予測が SET=Resume。
- golden(`tests/golden_scenarios.rs` の scenario_15 系に「離脱→戻り」): 殻を通して「already inactive」でスキップしないこと(B-1 は純関数の表では捕まらない)。
- CI(`sc-focusrestore-*` を `--strict` に格上げ): 合格条件は、A に戻って最初の打鍵で `typed == か`、A の conv が NATIVE、窓 B の conv が不変**かつ B で NICOLA の文字を 1 つ打つと期待のかなが出る**(B-3)、離脱時の SendInput が B に出ていない、戻りの後に「復元 write をスキップ (already inactive)」と「復元リトライ #0 中断」が出ない。構成を足す: Alt+Tab で戻る(M-1。マーカー付き注入で物理扱い)、**戻った A で最初の操作を左 Shift タップにする(結果がかな)**、**最初の打鍵を Shift+文字にする(カタカナにならない)**、A→B→A→B→A の 2 往復と A→B→C→A と B で自分もトグル、100ms 未満の瞬間往復、**GJI の ATOK プリセットで A→B→A を往復する(反転しないこと。`RebuildToggle` の挙動を固定)**、Chrome(TsfNative)・UWP を observe で 1 構成ずつ。
- fix-requires-evidence: 上の単体・golden・CI で (a) を満たす。BUG-193.md の `fix_commits` を更新する (b)。

## 未決(実測で決める)

- **Q1・Q2(解決)**: 決定 3。残る確認は `FOCUS_RESYNC` との順序(決定 7、CI ログ)。
- **Q3**: A の 0x0 → 0x10 の遷移の主体。Phase 3.7 の診断スナップショット(`ime_refresh.rs:661-663`)で離脱の前後どちらかを CI ログで切る。決定 8 の予測器による判定が、離れている間の変化に対する安全弁になっている。
- **Q4**: 同一プロセス内の窓移動(`process_changed` が偽、`ir_notify_focus_changed` も世代 bump も走らない)。トグルと ObservedEisu が移動先にそのまま持ち越される失敗シナリオがある(S-4)。本 ADR の範囲外とするが、CI に `sc-focusrestore-sameproc` を observe で足して実 IME が窓ごとかを測り、結果次第で BUG を起票する。
- **Q5(解決)**: エンジン OFF 中も復元する(決定 9、所有者判断)。

## 範囲外

- 同一プロセス内の窓移動の修正(Q4 で測るのみ)。
- 観測起点の書き込みの整理(`docs/tasks/observation-driven-writes-inventory-2026-10-08.md` の A〜C)。本 ADR は個別の欠陥の修正で、整理の方向とは独立。
