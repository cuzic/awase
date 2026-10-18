---
id: ADR-245
title: |-
  プロセスをまたぐフォーカス移動で半角英数トグルを、離れるときでなく「戻ってきたとき」に復元する(BUG-193)
summary: |-
  BUG-193: 窓 A で持続半角英数(左 Shift 単独タップ)にしたまま別プロセスの窓 B へ移ると、離脱時の強制復元は IMC 書き込みが世代の bump で必ず中断し、
  F2/VK_DBE_HIRAGANA は移動先 B に届く。A は半角英数のまま残り、awase のトグルフラグだけが下りる(MS-IME 本体・GJI とも CI で 5/5)。
  仕様(ADR-107 の検証項目 4)は「往復しても英数状態が持ち越されない」で、所有者は 2026-10-08 に「A に戻ったらかなに戻す」と判断した。本 ADR は、離脱時には IME へ何も送らず(belief は従来どおり戻す)、A を覚えておいて、A に戻って最初の打鍵の手前で復元する案(B)を定める。
status: |-
  起草・改訂(2026-10-08)。所有者判断=「A に戻ったらかなに戻す」(案 B)・ATOK は送らずトグルを立て直す・エンジン OFF 中も復元する。Opus round1(Blocker 3・Must 6・Should 7・Nit 4)・round2(Blocker 0・Must 5・Should 4)・round3(Blocker 0・Must 2・Should 3)・round4(Blocker 0・Must 2・Should 2・Nit 1)を反映済み。round4 の判定=R4-1・R4-2 の追記を条件に実装に進んでよい(追記済み)。未決 Q3・Q4(実測)。
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
2. **戻り待ちのエントリ**(PR #558 の気づきを反映): **A のスコープは、トグルに入った時点(`commit_enter_imc`/`commit_enter_gji`)で `win32::foreground_scope()` を採って `HalfWidthAlnumState` に控えておく**。離脱の tick(`ir_notify_focus_changed`)では前面窓がすでに移動先 B なので、そこで採ると B のスコープになってしまう。離脱時はその控えを戻り待ちへ移す。**Enter 時にスコープが取れなかった場合(前面窓なし・pid 0。まれ)は、離脱時にトグルを下ろして belief だけ戻す(IME へは送らない。エントリは積まない)**(所有者判断 2026-10-08。A が英数のまま残る点は BUG-193 の症状のまま許容する)。`{scope: ForegroundScope, kind: 'uses_imc_conv_write'(Enter 時の IME 種別), at: TickMs}`。Enter 時と照合時の両方で **`win32::foreground_scope()` 1 本**を使い、`classified`(フォーカス子 hwnd 系)の値と混ぜない(M-3)。**戻り待ちが空のときは打鍵ごとの `foreground_scope()` を呼ばない**(S2-3)。容量は小さな固定長(例: 4、古いものから捨ててログを出す)。寿命は既存の `HWND_CACHE_MAX_AGE_MS` を借りる(新しい時間定数は作らない)。**`tuning.rs` のこの定数の doc に「ADR-245 の戻り待ちの寿命にも使う」と 1 行足す**(S2-1。片方の都合で値を変えたとき、もう片方の意味が黙って変わらないようにする)。`HwndImeCache` とは別の機構(キー・役割が違う)だが、戻りの順序は「キャッシュ復元(AssumedRomaji)→ 最初の打鍵で Resume」で一方向に固定する(M-4、round2 で許容)。
3. **Resume の実行位置(Q1)**: `kp_run_inner`(`key_pipeline.rs:38`)の中で、**`try_hold_key` と ime-off-rescue の後、`kp_stage_idle_conv_check` と `kp_stage_shadow_ime_toggle` の前**に 1 段足す。`enrich_key_role`/`enrich_thumb_key_role` の結果がすでに使え、保留されたキーが同じ関数へ戻ってきても判断が二重にならず、idle-conv-check の読みより先に `note_explicit_ime_action` を打てる。Resume の SendInput はこの段で同期して出るので、その打鍵自体の出力(executor が後で出す)より先に届く。打鍵を defer する仕組みは要らない。`ir_notify_focus_changed`/`ir_stage_notify` では実行しない(Phase 3.7 の世代 bump で IMC リトライが必ず中断する、B-2)。**修飾キー(Shift/Ctrl/Alt/Win)の KeyDown と KeyUp では Resume しない**(M2-1)。戻っても文字を打たないなら何も注入しない。
4. **復元本体の分離(B-1)**: 本体は `kp_restore_kana_from_half_width` に**残す**。先頭の `begin_restore_kana()` の旧値確認だけを引数で切り替える(例: `enum ExitOwnership { FromToggle, FromResume }`。`FromResume` は取り出し済みなので `begin_restore_kana` を呼ばずに `owns_exit = true` とする。取り出しが INV-B の「true→false の遷移 1 回」の役を担う、と ADR-107 INV-B を読み替える)。**OS 書き込みを新しい関数へ切り出さない**: `lints/actuation_call_guard` の `send_input_safe` 許可リストは呼び出し元の関数名で照合し(`lib.rs:70-93`)、`ActuationTarget::capture(` の件数(`architecture_guard.rs:2635`、key_pipeline.rs=3)も変わるため、切り出すと CI が落ちる。
5. **打鍵の段の判定は次の優先順位で上から決める(R3-1)**。Drop・RebuildToggle を Deferred より先に判定する(Ctrl+変換でひらがなに戻した A に、後から RebuildToggle や F2 を重ねないため)。

   | 順 | 条件 | 結果 |
   |---|---|---|
   | 1 | 修飾キー(Shift/Ctrl/Alt/Win)自身の KeyDown/KeyUp | `Nothing`(M2-1) |
   | 2 | 「IME モードの役割」のキー。`enrich_key_role` が IME モードの役割を付けたキー(shadow toggle・物理モードキー通過・ADR-188 の直接観測の対象)、**または `engine.matches_ime_set_open`/`matches_ime_off` が真のキー(`keys.ime_on/off/toggle` の組み合わせ、既定 Ctrl+変換など)**。修飾キーの有無を問わない。**親指キーの役割が付いたキーは除く** | `Drop`(送信なしでエントリを捨てる。ユーザーが選んだモードを上書きしない) |
   | 3 | `effective_open()` が偽、Enter 時と照合時の IME 種別が違う、`app_disabled`、寿命切れ | `Drop`(ログつき) |
   | 4 | GJI で `f2_is_set != Some(true)`(決定 8) | `RebuildToggle`(修飾キーの有無を問わない。注入しないので修飾キーのハザードと無関係) |
   | 5 | Shift/Ctrl/Alt/Win のいずれかを物理的に押している(`hook::is_physical_key_down`。`modifier_snapshot` と食い違うときは安全側) | `Deferred`(エントリを残す。Shift 押下のまま送ると Shift+ひらがなでカタカナ、Ctrl 押下で Ctrl+F2 になる。新しい時間定数も回数の上限も要らない。修飾キーはそのキーの並びの中で必ず離れる) |
   | 6 | それ以外(親指キー+文字を含む) | `Resume` |

   核に渡す事実「IME モードの役割か」は、殻が `engine_owns_open_key`(`key_pipeline.rs:93-104`)と同じ式で作る。**式は 1 か所、評価は 2 回(R4-2)**: `fn engine_owns_open_key(&self, event) -> bool` のヘルパーにまとめ、(1)新しい段の前で行 2 の事実を作るために呼び、(2)段の後の元の位置(`:93`)で `kp_stage_shadow_ime_toggle` に渡すために呼び直す。計算を前へ移して 1 回で共有してはいけない: `matches_ime_set_open` はエンジンが活性のとき修飾なしの親指キー単独を IME の組み合わせから外す(`engine.rs:906-916`、`is_bare_thumb`)ので結果が `ctx.input_mode` で変わり、RebuildToggle の打鍵で古い値が shadow toggle に渡ると、同じ打鍵で Engine と shadow の二重の actuation になりうる(BUG-46 / ADR-208 D1 の型)。段が belief を変えなかったときは 2 回の結果が一致し、変えたときだけ後者が新しい ctx を反映する。行 2 の判定は段の前の belief で行う(S4-2。読める MS-IME 窓でポーリングが先に ObservedEisu を書いていると修飾なしの親指キーが IME の組み合わせとして一致し Drop になるが、トグル中の A の既存の挙動と同じなので許容)。

   行 3 の「`effective_open` 偽」は、B から持ち越された偽(B で IME OFF した直後、TsfNative の A で B が閉かつフォーカス子 hwnd が不一致)では Drop が正しい(belief が「閉」なのにトグルを立て直すと辻褄が合わず、Exit の GJI 送信も見送られる)。**ただしこの Drop は GJI(TSF 系)のエントリだけ**(2026-10-18、#666 の CI 実測): MS-IME 本体(IMC 経路)は窓ごとに IME の状態を持つので B の閉は持ち越されず、しかも半角英数トグル中の A 自身が閉(`imm=C`、戻った A は `imm=C/0x10`)と読まれる。行 3 を MS-IME 本体にも掛けると、`sc-focusrestore-msime-*` の 5 構成すべてで最初の打鍵が `Drop(EffectiveOpenFalse)` になり復元されなかった。MS-IME 本体は `effective_open` を見ない。Drop の理由は**ログで分けて出す**(「effective_open 偽(B から持ち越し)」など。S4-1)。親指キーを Alt に割り当てた構成では、その Alt は行 1・行 5 の対象になり、同時打鍵が続く間は Deferred になる(注入しないだけなので許容)。注記(Nit): rescue を張る側(`set_ime_off_rescue_pending`、`key_pipeline.rs:150` 付近)は新しい段より後にあるので、Ctrl+無変換のミスタイプが 50ms 以内の Ctrl↑ で捨てられると、IME に何もしていないのにエントリだけが消える。狭い場合なので許容する。
6. **左右 Shift タップ(戻り待ちあり)は Exit 扱い**(S-3、M2-1): 既存の `on_shift_up` に `pending_for_scope: bool` を足し、中で `active = toggle_held || pending_for_scope` を `plan_half_width_alnum_action` に渡す(純関数本体とそのテストは変えない)。`Exit` を `ExitRestoreKana { ownership }` に写すときだけ、`toggle_held` が偽で `pending_for_scope` が真なら `FromResume` にする(新しい関数は作らない。`toggle_held` と `pending_for_scope` が同時に真になる経路は無い)。**KeyDown 側(R4-1)**: `kp_shift_conv_guard_key_down`(`key_pipeline.rs:1913`)は、トグル中でなければ `!effective_open() || !is_japanese_ime() || !is_user_enabled() || !conv_mutation_allowed` で `disarm_guard()` し、KeyUp 側は冒頭の `take_guard()` が偽で抜ける。エンジン OFF など(決定 9 の構成)では ExitOnTap に届かないので、`:1913` の早期 return の条件を `is_toggle_active() || pending_for_scope` にする。戻り待ちがあるときだけ `foreground_scope()` を読み、スコープ一致を事実として渡す。「ガードを落とすか」も判断点 (c) の純関数に含める。`FromResume`、`prepend_synthetic_shift_up=true`、エントリを消費。これは上の表の外(KeyUp の判断点)にある。
7. **MS-IME**: Resume でも `confirm_gate_deadline_override_ms = now + SHIFT_CONV_GUARD_RELEASE_CONFIRM_MS` を張る(S-5)。`note_explicit_ime_action` は復元と同じ順で呼ぶ。TsfNative では `FOCUS_RESYNC`(`app/mod.rs:643-656`)が最初の対象キーを `kp_run_inner` の前に defer して読むので、順序は resync の読み(ObservedEisu)→ キーの replay → Resume(AssumedRomaji)に固定される見込み。CI ログで resync → Resume の順を確かめる(resync が ObservedEisu を見て能動的な書き込みを起こさないことも)。
8. **GJI(INV-B・S-1・M2-4)**: 取り出しと同時にエントリが消えるので二重送信は無い。ただし GJI のプリセットによっては VK_DBE_HIRAGANA(F2)が**純粋なトグル**で、リポジトリの実測表が ATOK を「0x19→0x10、0x10→0x19 の純粋なトグル」と記録している(`state/key_effect_predictor.rs:1275`、grid 第 3 版)。通常の Exit は「A が 0x10 と分かっている」直後なので安全だが、Resume は離れている間に A が変わりうる(Q3、言語バーの操作)。そこで Resume の直前に予測器(`KeyEffectKeymap::predict_with_override`、`TableKey::Hiragana`)へ 2 つの入力を問う: (i) 開・ObservedEisu(conv 0x10)、(ii) 開・ひらがな(conv 0x19)。**両方とも mode がかな(SET)なら送る**(MS-IME プリセット)。**(ii) が英数になる(トグル)、または予測が `None`(Custom・MsImeNative 等の CannotPredict)なら送らず、`RebuildToggle`**: 所有者判断(2026-10-08)で、A は英数のまま残し、`toggle_held` を A について立て直して belief を ObservedEisu にする(次の左 Shift タップで Exit。キーは注入しない)。HwndImeCache の復元が ObservedEisu を AssumedRomaji に洗うので、立て直しは最初の打鍵の段(決定 3)で行い、洗いより後に belief を書く。MS-IME 本体は IMC の SET と VK_DBE_HIRAGANA の SET なので判定は不要だが、`effective_open()` のガードは外さない(閉でひらがなを押すと開く、`key_effect_predictor.rs:1991`)。`RebuildToggle` でも `note_explicit_ime_action` を呼ぶ(段が idle-conv-check より前にあるため。S3-1。MS-IME には RebuildToggle は来ないので確認ゲートの猶予は不要)。寿命(決定 2)を超えたエントリは捨てる。
9. **Resume の条件**: 現在の窓が `app_disabled` でない。**エンジン OFF 中でも IME の復元は行う**(所有者判断 2026-10-08。通常の Exit と同じく、IME の復元はエンジンの ON/OFF と独立)。
10. **純粋な核(FCIS)は判断点ごとに 3 つの純関数・3 つの enum に分ける(S3-3)**。1 つの enum にすると「この判断点には来ないはずの variant」を `unreachable!` で捨てることになる。(a)離脱(`ir_notify_focus_changed`): `Suspend | Nothing`。(b)打鍵の段: 上の表の `Nothing | Drop | RebuildToggle | Deferred | Resume`。(c)Shift の KeyUp: 既存の `on_shift_up` に `pending_for_scope` を足す(決定 6)。いずれも `state/half_width_alnum.rs`。事実は殻が集める(トグル状態・戻り待ち・打鍵時点の前面スコープ・IME 種別・物理の修飾キー・`effective_open`・キーの分類・予測の答え)。**予測器は核の外の純ヘルパーで呼び、結果を `Option<bool>` の事実として渡す**: `state/key_effect_predictor.rs` に `fn hiragana_key_is_set(keymap, learned, unreadable) -> Option<bool>` を置く(決定 8 の 2 問を問い、両方かなで `Some(true)`、トグルで `Some(false)`、予測なしで `None`。`PredictInput` の `composing=false` と `unreadable` はヘルパーの中で決める)。Linux 単体で ATOK=`Some(false)`・MS-IME プリセット=`Some(true)`・Custom=`None` を固定する。戻り待ちがスコープと一致したときだけ計算する。新フィールドは `tests/architecture_guard.rs::half_width_alnum_state_fields_are_not_accessed_directly` の FIELDS に足す(S-7)。スコープつきの一回マークは `state/mode_key_pass.rs::ModeKeyPassLatch<S>` に前例があるので流用を検討する。

## 検証

- Linux 単体(`state/half_width_alnum.rs`、同じコミットに入れる)の表: 離脱=Suspend(belief は戻る)、戻り+文字キー=Resume、別窓=保持、容量超過=ログつきで古いものを捨てる、pid だけ一致=Resume しない、**修飾キーの KeyDown/KeyUp=Nothing**、**Shift/Ctrl 押下中の文字キー=Deferred**、`effective_open` 偽=Drop、IME 種別不一致=Drop、`app_disabled`=Drop、**左右 Shift タップ(戻り待ちあり)=ExitOnTap**、**IME モードの役割のキー=Drop**、**親指キー=Resume**、**予測がトグルまたは None=RebuildToggle(修飾キーの有無を問わない)**、予測が SET=Resume、**Ctrl+変換(`keys.ime_on` の組み合わせ)=Drop**、**Ctrl 押下中で予測がトグル=RebuildToggle**、**エンジン OFF + 戻り待ちあり + 左 Shift タップ=ExitOnTap(KeyDown のガードが落ちない)**。
- **B-1(already inactive でスキップ)の防ぎ方(R3-2)**: `tests/golden_scenarios.rs` は reducer しか回さず殻(`runtime/`、`#[cfg(windows)]`)を通らないので、「殻を通して」は golden では検証できない。(a)「この exit が OS 書き込みを持つか」を純関数にする: `fn exit_owns_write(ownership: ExitOwnership, was_toggle_active: bool) -> bool`(`state/half_width_alnum.rs`)。殻はその結果だけで分岐する。Linux 単体で `FromResume` + `false` → `true` を固定する。(c)CI のログ断片検査で、戻りの後に「復元 write をスキップ (already inactive)」が出ないこと(下の CI)。golden に残すのは reducer レベルの 2 本: 「CacheRestore(AssumedRomaji)→ UserHalfWidthAlnumToggle(ObservedEisu)の順で最終の belief が ObservedEisu」(RebuildToggle の belief 側)と、「離脱の AssumedRomaji の後、B の ctx が romaji-capable」(B-3)。
- CI(`sc-focusrestore-*` を `--strict` に格上げ): 合格条件は、A に戻って最初の打鍵で `typed == か`、A の conv が NATIVE、窓 B の conv が不変**かつ B で NICOLA の文字を 1 つ打つと期待のかなが出る**(B-3)、離脱時の SendInput が B に出ていない、戻りの後に「復元 write をスキップ (already inactive)」と「復元リトライ #0 中断」が出ない。構成を足す: Alt+Tab で戻る(M-1。マーカー付き注入で物理扱い)、**戻った A で最初の操作を左 Shift タップにする(結果がかな)**、**最初の打鍵を Shift+文字にする(カタカナにならない)**、A→B→A→B→A の 2 往復と A→B→C→A と B で自分もトグル、100ms 未満の瞬間往復、**GJI の ATOK プリセットで A→B→A を往復する(反転しないこと。`RebuildToggle` の挙動を固定)**、Chrome(TsfNative)・UWP を observe で 1 構成ずつ。
- fix-requires-evidence: 上の単体・golden・CI で (a) を満たす。BUG-193.md の `fix_commits` を更新する (b)。

## 未決(実測で決める)

- **Q1・Q2(解決)**: 決定 3。残る確認は `FOCUS_RESYNC` との順序(決定 7、CI ログ)。
- **Q3**: A の 0x0 → 0x10 の遷移の主体。Phase 3.7 の診断スナップショット(`ime_refresh.rs:661-663`)で離脱の前後どちらかを CI ログで切る。決定 8 の予測器による判定が、離れている間の変化に対する安全弁になっている。
- **Q4**: 同一プロセス内の窓移動(`process_changed` が偽、`ir_notify_focus_changed` も世代 bump も走らない)。トグルと ObservedEisu が移動先にそのまま持ち越される失敗シナリオがある(S-4)。本 ADR の範囲外とするが、CI に `sc-focusrestore-sameproc` を observe で足して実 IME が窓ごとかを測り、結果次第で BUG を起票する。
- **Q5(解決)**: エンジン OFF 中も復元する(決定 9、所有者判断)。
- **RebuildToggle の残余リスク(S3-2、許容)**: 離れている間に A の実状態が変わっていた場合(Q3、言語バーの操作)、RebuildToggle は「かなの A」を英数とみなしてエンジンを止める。注入はしないので状態は壊れず、症状は「NICOLA が効かない」にとどまる。その状態で左 Shift をタップすると Exit の F2(ATOK ではトグル)が出て英数へ反転する。所有者判断(ATOK は送らない)の範囲内の制限。CI の ATOK 構成で「戻った A で打つと英字(トグル維持)、次の左 Shift タップでかな」を固定し、「B で IME OFF してから A に戻る」(`effective_open` 偽の Drop)を observe で 1 本足す(S4-1)。

## PR 2 への申し送り(PR #558 の Opus コードレビュー、Blocker・Must 0)

- **M-PR2-1(PR 2 のマージ条件)**: `plan_leave` が「トグル中 + 控え無し(Enter 時にスコープが取れなかった)」を `Nothing` にしていると、殻が文字どおり実行したときトグルと ObservedEisu が B へ持ち越され B で NICOLA が止まる(round1 B-3 の再発。所有者判断「トグルを下ろして belief だけ戻す」と食い違う)。`LeavePlan` を 3 variant に分ける: `SuspendAndQueue`(トグルを下ろし、belief を戻し、戻り待ちへ積む)、`SuspendWithoutReturn`(トグルを下ろし、belief を戻す。積まない)、`Nothing`(トグルは立っていない)。殻の実行は「`Nothing` 以外ならトグルを下ろして belief を補正する」「`SuspendAndQueue` なら積む」の 2 行。状態側の操作は 1 メソッドに寄せ(例: `suspend_toggle_for_return() -> SuspendOutcome { queued, dropped_oldest }`)、`plan_leave` は控えたスコープの有無を状態から読む。単体表に「トグル中 + 控え無し → `SuspendWithoutReturn`(`toggle_held` は下り、戻り待ちは空)」を足す。
- **S-PR2-1**: Enter 時のスコープと **IME 種別も**同じ所で控える。`commit_enter_imc(scope)`/`commit_enter_gji(scope)` が `HalfWidthAlnumState` の private フィールド(例: `entered_in: Option<(ForegroundScope, bool)>`)に控え、`suspend_toggle_for_return` の引数から `scope` と `uses_imc_conv_write` を外す(殻が B のスコープや今の種別を渡す誤りを型で防ぐ。無効スコープを積める Nit も消える)。クリアは `begin_restore_kana`・`abandon_on_observed_follow`・`suspend_toggle_for_return`、保持は `rearm_after_failed_gji_exit`。FIELDS に 7 番目として足す。
- **S-PR2-2**: 打鍵の段は **KeyDown だけ**で評価する(KeyUp と auto-repeat〈`was_down`〉は `Nothing`)。`KeyStageFacts` にイベントの種類が無く `kp_run_inner` は KeyUp も通るので、B で押した文字キーを A に戻ってから離すと、その KeyUp で Resume/Drop が走る。殻で除くか、核の事実に `is_fresh_key_down` を足して行 0 として固定する。
- **S-PR2-3**: `RebuildToggle` の状態操作(`toggle_held=true`、Enter 時の控えを戻り待ちのエントリから復元)のメソッドを足し、「`commit_enter_gji` を呼ばない」「控えをエントリから戻す(次の離脱で再び積める)」の 2 点を単体テストで固定する。
- 行 2 の「親指キーを除く」は、PR 2 で `matches_ime_set_open` が `is_bare_thumb` で修飾なしの親指キーを先に除く(`engine.rs:906-916`)ので、エンジン活性のときは二重の除外になる。害は無い。

## 実装の段階(Opus round3 の推奨)

1. **PR 1(純粋な核のみ。挙動は変えない)**: `state/half_width_alnum.rs` に戻り待ちの型(容量 4、寿命の判定は `now` と `HWND_CACHE_MAX_AGE_MS` を引数で受ける)・3 つの計画関数・`exit_owns_write`、`key_effect_predictor.rs` に `hiragana_key_is_set`。Linux 単体の表、reducer の golden 2 本、`architecture_guard` の FIELDS、`tuning.rs` の doc 1 行。殻からはまだ呼ばない(dead_code は `#[expect(dead_code)]` か PR 2 と同時)。
2. **PR 2(殻の配線。挙動が変わる)**: `ir_notify_focus_changed` の離脱を Suspend に、`kp_run_inner` に新しい段、`on_shift_up` に `pending_for_scope`、`kp_restore_kana_from_half_width` に `ExitOwnership`(関数名・`send_input_safe`・`ActuationTarget::capture` の位置は変えない)、`engine_owns_open_key` の計算の共有。`cargo check --target x86_64-pc-windows-msvc -p awase-windows --tests --lib` と dylint。BUG-193.md の `fix_commits` を更新し、PR 本文に fix-requires-evidence の (a)(b) を書く。
3. **PR 3(CI の strict 化)**: `sc-focusrestore-{msime,gji}` を `--strict` に、構成の追加(検証節)。**少なくとも MS-IME 本体・GJI の strict が green になるまで PR 2 を develop にマージしない**(PR 2 と同じブランチで CI を回してから分けてもよい)。

## 範囲外

- 同一プロセス内の窓移動の修正(Q4 で測るのみ)。
- 観測起点の書き込みの整理(`docs/tasks/observation-driven-writes-inventory-2026-10-08.md` の A〜C)。本 ADR は個別の欠陥の修正で、整理の方向とは独立。

## 追補 2(2026-10-18): MS-IME 本体は最初の打鍵を保持し、復元が効いてから再生する(起草、Opus レビュー前)

### 事実(PR #666 の CI、run 39119247220 → 39120344451、head 25857f9e)

- 行 3 を GJI のエントリだけにすると(追補 1 相当、決定 5 の行 3 の注記)、MS-IME 本体は `plan=Resume` まで進み、復元の書き込み(`[imm-romaji] conv 0x10 → 0x19 success`)も効き、`typed` 時点の A は `imm=O/0x19` になる。**それでも `sc-focusrestore-msime*` の strict は FAIL**(本文 `w`、期待 `か`)。
- 原因(awase.log の時刻): 最初の打鍵 `w` は Resume の段(`kp_stage_half_width_return`)で復元を**起案**したあと、同じ打鍵のうちに Engine の判断へ進む。Engine は A を非活性とみなす(MS-IME 本体は半角英数トグル中に `effective_open=false`)ので `decision=PassThrough` で素通しし、`w` は IME が 0x10 のまま OS へ入る。復元の書き込みは `actuate_conv_mode` の `spawn_local` なので、その**後**に実行される。
- 実測(同 run の `sc-focusrestore-msime*` 6 構成 × 5 試行の awase.log、Resume 起案の時刻を 0 として): `[conv-actuate] Written` まで **2.8〜6.0ms(中央 4.9、n=35)**、`[effective-open-flip] → true` まで 3.4〜5.9ms(中央 4.9、n=8。ほかの試行は flip を 1 秒以内に出さない=belief が既に真だった等で未分類)。`Engine activated` のログは 60ms 前後だが、これは Engine が活性を**次の入力で**計算し直す(KeyUp)ためで、遅れの本体は ~5ms。
- GJI が最初の打鍵から `か` になるのは、GJI の半角英数トグル中も `effective_open` が真で、`apply_input_mode_correction` が同期で効いて Engine が活性になるから(MS-IME 本体は開閉自体が変わるので同期に揃わない)。

### 決定(案)

1. **対象は IMC 経路(`entry.uses_imc_conv_write`)の `Resume` だけ**。GJI の `Resume`/`RebuildToggle`、`Drop`、`Deferred` は変えない。
2. **Resume のとき、起案した打鍵(KeyDown)を `Consumed` にして保持し、保持が終わるまでの後続の打鍵(同じ打鍵の KeyUp・次の KeyDown/KeyUp)も同じ保持へ順序どおり足す**。保持は新設の `ResumeHold`(`state/half_width_resume_hold.rs`、純粋な状態機械。時刻・前面窓スコープ・Engine 活性は引数で受ける)が持つ。**打鍵は捨てない**(下の 4 の窓変更を除く)。
3. **解放の条件(上から先に成立したもの)**:
   (a) `Engine::compute_active(&build_ctx())` が真(復元が効いて Engine が活性になった。`lang_detect_engine_active` と同じ式)→ 保持した打鍵を `INPUT_DEFER.replay_later` で順序どおり再生する(Engine を通るので `か`)。
   (b) 保持の開始から `HALF_WIDTH_RESUME_HOLD_MAX_MS` 経過(復元が効かなかった)→ そのまま再生する(従来どおり素通しで `w`。**BS は送らない**: 復元が効いた証拠が無いとき BS は別の文字を消す〈BUG-200 の機序〉)。
   (c) 保持が上限個数(4)に達した → 先に解放してから新しい打鍵を通常どおり処理する。
4. **前面窓が保持の開始時と違えば保持した打鍵は捨てる**(ADR-263 の `lang_detect_on_timer` と同じ扱い。別窓へ誤配送するより損失を取る)。journal でなくログに件数を残す。
5. **解放の判定は専用の一発ポーリングタイマー `TIMER_RESUME_HOLD`(10ms 周期、保持中だけ)**。belief や `ImeEvent` の経路に手を入れない(事象駆動にすると `effective-open-flip` の発火点に保持の解放を差し込むことになり、`ime-belief-architecture.md` の領域へ触る)。10ms は実測の中央 4.9ms の 2 倍で、Windows のタイマー分解能の下限(約 15.6ms)に当たるので、実効は 1〜2 周期。
6. **`HALF_WIDTH_RESUME_HOLD_MAX_MS = 100`**(`tuning.rs`、`#[measured]`)。導出: 復元の書き込みの実測最大 6.0ms(n=35)に、IMC の cross-process 書き込みが実機で遅い場合の余裕を見込む。上限は体感の遅れ(1 打鍵目だけ最大 100ms)と釣り合わせる。実機(dragonflyg4、負荷時)の分布は未測定。**コミット本文に実測を書く**(`tuning-constants.md`)。
7. **既存の保持との関係**: `try_hold_key`(TsfGate)は PendingWarmup の保持で、解放は TSF モード確定。`lang_detect` の保持は Engine 活性が前提。本件は Engine が**非活性**の窓で、かつ解放が「Engine が活性になったか」。条件が違うので別の状態を持つが、**再生の口は同じ `INPUT_DEFER.replay_later`**(drain 側の順序・gate はそのまま使う)。

### 再発ファミリーとの関係(fix-requires-evidence)

- **defer/replay キューの解放条件**: 本件は `replay_later` の新しい呼び出し元を足す。drain 側の窓口(`drain_pending_deferred_before_send_if_queue_only` 等)は変えない。保持中に来る後続の物理キーは本機構の保持へ足す(live で素通しさせない)ので、`pending_deferred` を経由せず順序を壊さない設計とする。
- **物理キー押下ラッチ(Down/Up 非対称)**: KeyDown を保持すると、その KeyUp が保持の外で素通しされる経路が一番危ない。保持が続く間は KeyUp も保持へ足し、解放は Down と Up を同じ再生に載せる。**保持が既に解放された後に来る KeyUp** は、再生の Down より先に OS へ届かないよう、再生が drain されるまで `INPUT_DEFER` の並びに従う(既存の `lang_detect` の再生と同じ前提。**Opus レビューで確かめる論点**)。
- **belief**: 新しい belief 更新は無い(読むだけ: `compute_active`)。復元の書き込み(`actuate_conv_mode` と `apply_input_mode_correction`)は PR 2 のまま。

### 検証

- 純粋な状態機械(`state/half_width_resume_hold.rs`)の単体テスト(Linux): 解放(a)(b)(c)、窓変更で破棄、順序(Down→Up→次の Down)、上限、KeyUp だけ来る、保持が空のときは何もしない。
- `architecture_guard`: 保持の解放が `INPUT_DEFER.replay_later` だけで、`SendInput`・Backspace・`apply_*` を呼ばないこと。`TIMER_RESUME_HOLD` が保持を空にしたとき kill されること。
- CI(`e2e-ime`): `sc-focusrestore-msime{,-abc,-bopen,-twice}` の strict が PASS(最初の打鍵が `か`)。`msime-shiftchar`(Deferred→Resume)も同じ保持で PASS になるはず。`msime-lshift` は戻り待ちを Shift タップの Exit が取るので別経路で、`ｗ` の原因は別に調べる(本追補の範囲外)。`*-alttab` は CI の注入で窓が切り替わらないハーネスの問題で、本追補とは独立。

### 却下した案

| 案 | 却下理由 |
|---|---|
| BS して再注入(GJI の raw-tsf-literal 回収に倣う) | 回収は awase 自身の出力の literal を直す仕組みで、今回の `w` は素通しした物理キー。さらに画面に一瞬 `w` が見え、復元が効かなかったときの BS が別の文字を消す(BUG-200)。復元の成否を確かめてから通す保持のほうが、失うものが無い |
| 復元の直後に open の belief を同期で真にする | belief 更新(`ime-belief-architecture.md`)に触る。事象駆動の解放も同じ領域に入る |
| Engine を活性とみなして先に通す | `Engine::compute_active` は belief から出る。非活性のまま通すと NICOLA 変換が走らない |

### 未確認

- 実機(dragonflyg4)での分布、特に IME の応答が遅い窓(重い入力欄)。
- 保持中にもう 1 つ物理キーが来る高速打鍵(同時打鍵の親指キーを含む)の順序。
- `INPUT_DEFER` の drain が保持の解放と物理キーの間で順序を保つか(上の Opus 論点)。
