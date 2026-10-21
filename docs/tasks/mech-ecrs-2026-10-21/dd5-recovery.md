# DD5: 「ずれたら直す」層の重なり(自己修復・回復・再同期・救済)

調査対象: `/home/cuzic/rust-nicola-wt-dd`(origin/develop `e4766b2a`)。読み取りのみ・cargo は実行していない。
「確認」と書いたものは下に添えたコマンドで確かめた。それ以外は「未確認」と書く。

## 結論

- 開閉の軸で、belief と実 IME のずれを直す入口は **書き込み側 2 本、追随側 9 本(焦点時の既定値 5 本は別)** ある(§1)。追随側の 9 本は、**どれも他では拾えない場面を 1 つ持っている**。証拠の種類(読める窓の poll・GJI の I/O・注入キーの基準値・物理キーの直後の読み・共有メモリ・起動時)と窓の種類(読める・読めない・TsfNative)で棲み分けている。belief の書き込み口も `ModeKeyPassedThrough` 1 つに既にまとまっている。**追随の経路そのものを消せる候補は無い**。
- 重なりがはっきりしているのは、追随の本体ではなく、**本体の周りにある「写し」と「点の同期」**のほう:
  - **(a)** フォーカス時の `applied` の写しが同じ同期呼び出しの中で 2 回書かれ、後の 1 回が前の 1 回を必ず上書きする。
  - **(b)** GjiFsm の ImeOn の点の同期 2 か所は、ADR-203 (i) が同じずれを拾う。ADR-203 決定 4 自身が「撤去してよい」と書いており、実証の手順だけが未着手。
  - **(c)** 追随の後半 3 行が 3 か所に複製されている。
- 数: **E 2 件・C 3 件・?(要実機・要確認)3 件**。K(残す)とした防御は §4 にまとめた。
- 上位 5 件:
  1. **DD5-01(E)**: GjiFsm への ImeOn の点の同期 2 か所(`presync_applied_open_on` の中と `sync_ime_kind_from_observation`)を撤去し、ADR-203 (i) に一本化する。確度は中〜高。e2e の B2 構成と ablation が 1 つずつ要る。
  2. **DD5-02(E)**: フォーカス時の `presync_applied_open_on` の呼び出しを撤去する。同じ `ir_execute` の中で `ir_post_focus_change_snapshot` の `record_confirmed` が必ず上書きする。確度は中(DD5-01 の後)。
  3. **DD5-03(?)**: フォーカス時の写しそのもの(静的な `Imm32Unavailable` 窓の偽の `Confirmed`)。MD-10 の「未確認」の部分は確認が取れた。全廃(案 B)か、placeholder の柵の追加か。実機要。
  4. **DD5-04(C)**: 追随の後半(意図の削除 → `pass_through_observed(true,true)` → `last_external_change_ms`)を 1 つの関数にまとめる。確度は高、規模は小。MC-C2 のうち ME-4 を待たずにできる部分。
  5. **DD5-05(C)**: `reschedule_ime_refresh` の「短い窓の読み直し」4 分岐を純粋な計画関数にし、決定表を全列挙で固める。読めない窓の通過マークの扱いは ?。
- 見立ての確度: DD5-01・DD5-04 は高い(根拠がリポジトリ内の決定と grep で閉じる)。DD5-02 は呼び出しの順序を確認済みだが、`applied` の往復を擬似 IME が写していないため等価性は推論止まり。DD5-03・05・08 は実機か追加の調べが要る。

## 1. 入口の一覧(失敗の種類=「belief と実 IME の開閉・入力モードがずれたまま残る」)

### 1.1 開閉: awase が IME へ書いて揃える(書き込み側)

| ID | 防御 | 場所 | 守る不変条件 | それだけが止める場面 | 根拠 |
|---|---|---|---|---|---|
| W1 | drift correction | `runtime/ime_refresh.rs:1025 ir_apply_drift_correction`(判断は `awase-ime-core/src/state/drift_correction.rs:127 evaluate_drift` と `drift_plan.rs`) | ユーザーの明示操作の書き込みが届かなかったら、上限つきで送り直す | ADR-212 P6 以降は `last_intent.target == desired_open` のときだけ動く(`drift_correction.rs:136`)。送り直しはこれ 1 本だけ | BUG-20/43/68/163、ADR-080・212。`architecture_guard`(`.apply_ime_open_with_view(` の件数=1、`drift_diagnostic_call_is_guarded`) |
| W2 | panic reset の SetOpen(true) | `runtime/mod.rs:2343 panic_reset`・`2386-2396` | 利用者のリセットで IME を開ける | 利用者の手動操作(トレイ) | BUG-182 |

撤去済みのもの(再報告しない): `reassert_explicit_physical_key`、`force_on_and_correct_romaji`、`apply_force_on_for_imm_broken`(`f83084b3`・`621bf93c`)、ActivationSync(ADR-213)、フォーカス時の強制 OFF(2026-09-25)、フォーカス時の eager warmup(ADR-212 P4)。

### 1.2 開閉: 実 IME に belief を追随させる(追随側)

belief を書く口は、どれも最後は `ImeStateHub::pass_through_observed` → `ImeEvent::ModeKeyPassedThrough`(`state/platform_state.rs:776`)を通る。`mode_key_passed_through_event_is_dispatched_from_one_place` がこれを固定している。

| ID | 防御 | 場所 | 対象の窓・証拠 | それだけが止める場面 | 根拠 |
|---|---|---|---|---|---|
| F1 | 定期の poll(`apply_ime_update`、Medium) | `ime_refresh.rs:237-270`、`ir_poll_and_learn:684` | 読める窓(OsPoll) | 読める窓の、awase が関わらない外からの変化全般 | 基礎の経路 |
| F2 | GJI の I/O からの推論(observer_poll と `GjiIoInference`) | `ime_refresh.rs:185-236` | Blacklist(読めない窓)× GJI | 読めない窓の英数固着(`eisu_recovery.rs` の表の最後の行)。「唯一のユーザー操作不要の自己回復経路」とコメントにある | 2026-07-06 Edge の英数固着 |
| F3a | 通過マーク・窓の中の観測(ADR-187) | `ime_refresh.rs:248-258` → `platform_state.rs:412,431` | 読める窓 × モードキーが素通り | Engine OFF 中に物理モードキーが IME に届き、古い明示意図が観測より優先される(BUG-157) | ADR-187・191 |
| F3b | 通過マーク・窓の終わりでの破棄 | `ime_refresh.rs:506-514`、`mode_key_pass.rs:220 should_drop_intents_for_mode_key_pass` | 読める窓 × 窓の間の読みがすべて時間切れ | 意図が残って poll が止まる(BUG-158・BUG-151 の原因 ③) | 同上 |
| F3c | 窓が切れた後の最初の観測で揃える | `ime_refresh.rs:259-269`、`platform_state.rs:824` | 同上 | 窓の間の観測が全滅した通過の後の、`observed ≠ desired` の残り(BUG-158 追補 2) | 同上 |
| F4 | 外部変化の監視窓(ADR-205、基準値あり) | `ime_refresh.rs:281`、`platform_state.rs:512`、arm は `key_pipeline.rs:1748` | 読めない窓 × GJI × 外部の注入キー | 外部ツールの注入で開閉が変わった。「Chrome は常に 0」で偽の OFF を採らないための基準値 | BUG-172。`external_change_watch_has_single_arm_and_follow_sites` |
| F4' | give-up を契機にした監視窓(ADR-227) | `ime_refresh.rs:367` | F4 と同じ窓 | literal 回収の give-up のあとの読み直し(閉の推論はしない) | BUG-074 |
| F5 | 直接観測の窓(ADR-188/244、基準値なし) | `ime_refresh.rs:310`、`platform_state.rs:584`、arm は `key_pipeline.rs:1784`・`executor.rs:639` | 読めない窓 × GJI/MS-IME 本体 × 物理モードキー(Shift 付き)・FSM の再送出 | 予測が効かない打鍵の後の実状態 | BUG-149/150/186/199。`ms_ime_native_direct_follow_takes_only_closed_to_open` |
| F6 | Mozc の共有メモリ(ADR-268、High) | `platform_state.rs:2092 report_mozc_shm`。読みは `runtime/mozc_shm_shadow.rs:60`(refresh の tick)と `:70`(専用タイマー、D5) | Mozc Readable の全窓 | 明示意図が残ったままの外部からの閉(BUG-200 に近い場面。実機で D5 のあと 6/6 が追随) | ADR-268 |
| F7 | 起動時の初期値を最初の観測へ揃える(BUG-163) | `ime_refresh.rs:901`、`platform_state.rs:807` | 起動直後 | 下の DD5-06 を参照(drift の防止は P6 に吸収済み。TsfNative の SSOT へ持ち込む値の役目が残る) | BUG-163。`startup_placeholder_desired_is_not_treated_as_intent` |

焦点時の既定値(観測ではないもの。分岐で排他になっている): TsfNative の hwnd キャッシュ復元(`focus_tracking.rs:658`)、`Imm32Unavailable` のキャッシュ復元(`:705`)、`assume_closed_for_new_thread`(`:743`)、`reset_stale_ime_on_for_imm_broken`(`:748`)。フォーカス直後の読みには、FocusProbe(`key_pipeline.rs:432`)と ImmCrossProbe/focus(`focus_tracking.rs:799`)がある。読み 5 系統の重なりは前回の MD-09 で扱ったので、ここでは扱わない。

### 1.3 入力モードの軸

`idle_conv_check`(`key_pipeline.rs:497`)と FOCUS_RESYNC(`:520`)は、同じ `kp_stage_idle_conv_check_inner` の 2 つの入口で、統合済み。ほかに次の防御がある:

- ADR-238 の英数候補の確認
- `eisu_reset_on_ime_on` の 3 種(`key_pipeline.rs:1220,1258,1633`。判定関数は `eisu_recovery.rs:94` の 1 つにまとまっている。`user_ime_on_paths_are_paired_with_eisu_reset` が対の配線を監視)
- `ImmBrokenCorrection`(`ime_refresh.rs:583`)
- `cache_restore_eisu_guard`
- F5 の英数の軸
- 半角英数の復元 `kp_restore_kana_from_half_width`(`key_pipeline.rs:2704`)

### 1.4 `applied` の軸と GjiFsm の軸(ずれた後の「再同期」)

| ID | 防御 | 場所 |
|---|---|---|
| A1 | フォーカス時の先同期 `presync_applied_open_on`(`record_confirmed(true)`、GJI なら `gji_on_ime_on`) | `focus_tracking.rs:193`(呼び出しは `:782-789` と `ime_refresh.rs:925`) |
| A2 | 焦点のスナップショットの写し `record_confirmed(effective_open)` | `ime_refresh.rs:825-833` |
| G1 | IME 種別の同期での ImeOn(`applied_open()==Some(true)` のとき) | `message_handlers.rs:975-986` |
| G2 | ADR-203 (i): `send_keys` の直前の level 突合 `OnImeOnBelief` | `platform.rs:980-1000`、判定は `gji_direct_mechanism.rs:291 needs_belief_sync_on` |
| G3 | ADR-203 (ii): `kp_reopen_gji_fsm` の 4 か所 | `gji_fsm_sync_entry_points_are_accounted_for` が件数を固定 |

### 1.5 belief ではない回復(範囲外として 1 行で済ませる)

- hook watchdog(issue #165)・`recover_stuck_wake_if_needed`(`hook_channel.rs:226`): フックが生きていることという別の不変条件。K。
- `RawTsfLiteralRecovery`・per-VK confirm: 出力の literal の回復。前回の MB-01/02 で扱った。belief へ入るのは F4' の契機だけ。

## 2. 候補

### DD5-01(E): GjiFsm への ImeOn の点の同期 2 か所を撤去し、ADR-203 (i) に一本化する

- **重なっている不変条件**: 「belief が ON なのに GjiFsm が OffCold に取り残され、最初の数文字が握りつぶされる」(BUG-18・BUG-170)。
- **3 つの防御**: A1 の中の `gji_on_ime_on`(フォーカス時)、G1(IME 種別の同期)、G2(送信直前の level 突合)。G2 はローマ字を IME 経由で送るときに `GjiFsm` が OffCold なら必ず同期する。入口を問わない(`gji_direct_mechanism.rs:291-310` の doc「時刻反転・起動時既定値・観測の揺れの影響を受けない、入口も問わない」)。
- **決定はリポジトリに既にある**: `docs/adr/203-gji-fsm-follows-belief-open-transitions.md:92-93` 決定 4「`presync_applied_open_on`(BUG-18)と `sync_ime_kind_from_observation` の `applied_open()==Some(true)` の ImeOn は、(i) が GjiFsm 作り直し(B2)も拾うため最終的には撤去可能。…(i) が e2e(b) で B2 を拾えると実証できてから別コミットで撤去するか決める」。`docs/known-bugs/BUG-170.md:55,59` の該当項目はどちらも未チェック(`[ ]`)。
- **e2e の B2 構成は無い**(確認): `grep -o "sc-reopen[a-zA-Z0-9-]*" .github/workflows/e2e-ime.yml` で出てくるのは `sc-reopen-tsf-gji-gap*`・`-nofix`・`sc-reopen-tsf-msime-gap600`・`sc-reopen-chromepage-gji-gap600` だけ。これらは (c) 開き直しの構成で、「IME ON のまま GJI 種別を検出し直す」(B2)ものではない。
- **撤去で変わる挙動**:
  1. ImeOn の時期が、フォーカスの時点・種別検出の時点から「最初のローマ字の送信の直前」へずれる。
  2. Unicode 注入モードでは G2 が同期しない。害の無い場面として、ADR-203 が意図して除いたもの。
  3. probe・raw recovery の最中の最初の送信は、G2 が見送って次の送信で拾う(`needs_belief_sync_on` の `probe_or_recovery_blocking`)。
  4. placeholder の間は、もともと A1 が GJI へ通知しない(`focus_tracking.rs:785`)ので変わらない。
- **具体案**: `focus_tracking.rs:198-208`(`gji_on_ime_on` と journal の吸収)と `message_handlers.rs:975-986` を削除する。
- **壊れる guard**: `gji_fsm_sync_entry_points_are_accounted_for`(`architecture_guard.rs:7417`。`gji_on_ime_on(` が `focus_tracking.rs` 1→0、`message_handlers.rs` 1→0)。ADR-203 の決定表、BUG-170.md の該当行も更新する。
- **再発ファミリー**: warmup・IME belief。fix-requires-evidence の (a) は、下の ablation と e2e の構成。
- **検証**: 擬似 IME は GjiFsm を写していないので不可(`raw-c-belief.md`・`raw-e-pseudo-ime.md`)。ADR-240 の ablation の型で次の 2 つを足す:
  - `a12-no-gji-point-sync.sh`: 上の 2 か所を外す。
  - B2 構成(例 `sc-gjikind-resync-on`): GJI 種別を IME ON のまま検出し直させてから打鍵する。I4(`i4_gji_fsm_off_cold_composition`、上限 0)と `--require-sync`(`ImeOn(BeliefSync:level)` が出ること)で判定する。expect=pass。
  - BUG-18 の往復(TsfNative⇔Uwp の AppKind 往復で無操作)の構成も、同じ ablation で回す。既存のフォーカス往復の構成は無い(`docs/teardown-verification-guide.md` §6「フォーカス切替のシナリオが無い」)。

### DD5-02(E): フォーカス時の `presync_applied_open_on` の呼び出し(A1)を撤去する

- **重なっている不変条件**: 「フォーカス後の `applied` を、その窓の状態として写す」。
- **確認した呼び出しの順序**:
  - `on_focus_process_changed` を呼ぶのは `focus_tracking.rs:98`(`apply_focus_probe_result` の中)だけ。`apply_focus_probe_result` を呼ぶのは `ime_refresh.rs:96` と `focus_tracking.rs:884`(`detect_and_update_focus`、呼び出し元は `ime_refresh.rs:95` だけ)。確認に使ったコマンド: `git grep -n "on_focus_process_changed(\|detect_and_update_focus()\|apply_focus_probe_result(" -- crates/awase-windows/src`。
  - したがって、A1 は必ず `ir_execute` → `ir_stage_focus` の中で走り、同じ同期呼び出しの中で次の順に進む:
    `FocusChanged`(`applied = Unknown`。`ime_model.rs` の `reduce_focus_changed`)→ A1 `record_confirmed(true)` → `ir_notify_focus_changed` → `ir_stage_strategy` → `ir_stage_observe`(OsPoll)→ A2 `record_confirmed(effective_open)`。
  - A2 は A1 と同じ判断 `may_mirror_applied_on_focus` を通るので、**A1 が書いた値は A2 が必ず上書きする**。例外は `app_disabled` の早期 return(`ime_refresh.rs:69`)だけ。無効化アプリでは IME へ何も送らないので、`applied` は使われない。
- **A1 と A2 の間で `applied` を読む箇所**(`git grep "applied_open()\|model().applied"` で列挙):
  - `ReadStrategyFacts.applied_known`(`ime_refresh.rs:668`): 打鍵中で、かつ IntentStore に意図が残っているときに、焦点の tick の読みが OsPoll か SkipTyping かが変わる。
  - Engine の `FocusChanged` は SetOpen を出さない(`src/engine/engine.rs:441-445` ADR-213 P2b)ので、executor の `applied_snapshot` は読まれない。
- **DD5-01 の後の A1 の役割**: `record_confirmed(true)` とログだけになる。ADR-262 が A1 の存在理由として挙げた残りは「BUG-18 の GjiFsm 通知」と「`press=None` の ON の already-matched の材料」(`focus_tracking.rs:181-185`)だけで、前者は DD5-01 で消え、後者は A2 が同じ tick で供給する。
- **具体案**:
  - `focus_tracking.rs:769-789`(`is_effectively_tsf_native_now` の計算・placeholder の柵・呼び出し)を削除する。
  - `presync_applied_open_on` は `ir_align_placeholder_desired`(`ime_refresh.rs:925`)が使うので残す。起動直後は `on_focus_process_changed` が走らず(`establish_initial_focus_scope` はこれを呼ばない)、A2 も走らない。そのため、そこでは唯一の写しになる。DD5-01 の後なら `record_confirmed(true, tick)` へインライン化できる。
- **壊れる guard**:
  - `startup_placeholder_desired_is_not_treated_as_intent`(`architecture_guard.rs:3816`。`focus_tracking.rs` の `self.presync_applied_open_on(` が 1→0、placeholder の柵の位置の検査)
  - `applied_mirror_on_focus_goes_through_the_plan_at_all_three_sites`(`:3684`。3 か所→2 か所)
  - `record_confirmed` の件数の guard(`architecture_guard.rs:2153` の説明)は、A1 の本体の `record_confirmed` を残すなら件数は変わらない。インライン化するなら見直す。
- **変わる挙動**: 上の `applied_known` の 1 tick だけ。
- **再発ファミリー**: IME belief・focus 遷移。
- **検証**: 擬似 IME は `applied` の往復を写していない(ハーネスの冒頭に明記、raw-d の MD-10)。ME-2 の拡張(`record_confirmed` と `ReadStrategyFacts` を通す)が無い間は、guard の更新と e2e の焦点系(`sc-*` の GJI・MS-IME)の回帰で見る。

### DD5-03(?): フォーカス時の写し(A2)そのもの。MD-10 は確認が取れた

- **MD-10 の未確認の部分**: `record_confirmed` → `ImeModel::confirm_applied`(`awase-ime-core/src/state/ime_model.rs:599-606`)には placeholder の分岐が無い。**確認した**。つまり A2 は placeholder の間でも `Confirmed(effective_open)` を書く。
- **ただし BUG-163 が止めたかった害は A2 には無い**: BUG-163 の害は GJI への ImeOn 通知(long-cold の `VK_IME_OFF→VK_IME_ON`)で、A2 はこれを行わない。
- **より一般的な問題(未確認、実機要)**:
  - `may_mirror_applied_on_focus` が除くのは、TsfNative と、実測学習で降格された窓だけ(`applied_mirror_plan.rs:20`)。**静的な一覧による `Imm32Unavailable`(Chrome・Edge)は写される**。そこでは窓の中の IMM を読んでいないので、`effective_open` の値は、`reset_stale_ime_on_for_imm_broken` が書く Low の安全既定値 ON や、キャッシュ由来になりうる。
  - その偽の `Confirmed(true)` は、`press=None` の ON の書き込みを GJI の already-matched で省かせうる。対象は `apply_engine_on_with_ime_recovery`・`ime_set_open_effects` の `SetOpen { press: None }`(`src/engine/engine.rs:870-873,911`)。これは ADR-262 が「一太郎等」で止めた #623 と同じ型。
- **場面(未確認)**: Chrome × GJI で IME を閉じたまま窓へ入る(新規スレッドでなく、キャッシュも無い)→ 安全既定値 ON → A2 が `Confirmed(true)` を書く → EngineOn コンボで `SetOpen(true, press=None)` → already-matched で書かない → IME は閉じたまま。
- **案 B(全廃)**: A2(と DD5-02 の後の A1・align の中の写し)を全廃し、フォーカス後の `applied` を全窓で Unknown のままにする。TsfNative と学習降格窓では、既に本番でこの状態で動いている。
  - 消えるもの: `applied_mirror_plan.rs`(純粋関数と 4 テスト)、写しの 3 か所、INV-A97-1 の例外の 2/3。
  - 変わる挙動: `press=None` の ON がフォーカス直後に省かれず書かれる(絶対キーで冪等)、`warmup_ime_on()` が `applied ?? belief` の belief 側を使う(写しの値と同じ。belief が後から変わったときだけ差が出る)、`applied_known` の 1 tick。
- **判断**: ?。決めるには、静的な `Imm32Unavailable` × GJI で「焦点 → `press=None` の ON」の already-matched の件数を実利用の journal(`ActuationDecision` の `AlreadyMatched`)で数える必要がある。e2e では、EngineOn コンボを使う構成が要る(既存の構成名は未確認)。

### DD5-04(C): 追随の後半を 1 つの関数にまとめる(MC-C2 のうち、ME-4 を待たない部分)

- **場所**: `platform_state.rs:530-535`(F4)、`641-646` と `675`(F5)、`2126-2130`(F6)。
- **共通の手順**: `if let Some(hwnd) = current_focus() { intent_store.remove(hwnd) }` → `pass_through_observed(tick, true, true)` → `last_external_change_ms = now_ms`。
- 近い形がほかに 3 か所ある: `drop_intents_for_mode_key_pass_in_scope:447-455`(F3)、`apply_key_effect_prediction` の `:343-346`、`apply_panic_reset:1594-1596`。いずれも意図の削除が付く。
- **根本の原因**: 明示意図が `last_intent`(ImeModel)と `IntentStore`(ハブ)の 2 か所にあること(MC-C1)。後半を畳むのは、その症状を 1 か所へ寄せるだけ。
- **MC-C2 との違い**: MC-C2 は 3 経路の契機(arm・窓)の統合で、ME-4 が先に要る。ここでの提案は**後半の 3 手だけ**を `fn follow_open_externally(&mut self, tick_ms, now_ms)` に出すもの。入力が同じで順序も変えないので、挙動は変わらない。
- **検証**: `platform_state.rs` は `#[cfg(windows)]` で囲まれていない(`state/mod.rs:15-16` は `cfg_attr(not(windows), allow(dead_code))` だけ)ので、既存の単体テスト(`report_mozc_shm` の 6 本 `:3175-3320` ほか)が Linux で回る。
- **壊れる guard**: `ms_ime_native_direct_follow_takes_only_closed_to_open`(`architecture_guard.rs:4905-4909`)。`follow_direct_read_in_scope` の開閉の分岐に `write_observer_poll → intent_store.remove → pass_through_observed(tick_ms,true,true)` が文字列としてこの順に並ぶことを検査しているので、新しい関数名で検査し直す。
- **規模**: −8〜10 行。値は小さい。MC-C1 を先にやるなら不要になる。

### DD5-05(C、一部 ?): `reschedule_ime_refresh` の「短い窓の読み直し」を純粋な計画関数にする

- **場所**: `runtime/mod.rs:1163-1254`。4 つの窓が順に評価される:
  1. 外部変化・直接観測の窓(F4/F5 の共通の状態 `external_change_watch`、`:1181-1191`)
  2. 英数候補(ADR-238、`:1199-1211`)
  3. 読めない窓の通過マークの終わり(`:1226-1237`)
  4. 読める窓の通過マーク(`:1238-1248`)
- そのあと「明示意図があれば止める」と「500ms の定期 poll」が続く。読み直しの予約を足しているのは、ほかに `schedule_ime_refresh(20)` の 5 か所(`key_pipeline.rs:1009,1668,1787,1803`、`lang_check.rs:99`)、`executor.rs:648` のタイマー、`MODE_KEY_PASS_REREAD_MS` の 2 か所(`ime_refresh.rs:258,399`)。
- **案**:
  - 時刻・窓の残り・`readable`・`last_ime_read_ok`・意図の有無を入力にした `plan_next_refresh(facts) -> Option<u64>` を `awase-ime-core` に出す。
  - 殻は facts を集めて `schedule_ime_refresh` を呼ぶだけにする。
  - ME-3 の型(`tests/explicit_press_exhaustive.rs`)の決定表の全列挙で固める。
  - 4 分岐の優先順位は今のコードのとおり。今はこの関数が `runtime/`(`cfg(windows)`)にあり、Linux で単体テストが無い(`teardown-verification-guide.md` §6「follow(ADR-187)／モードキー後の20ms再読み取り: 無し」)。
- **? の部分(読めない窓の通過マーク)**: `kp_stage_mode_key_follow`(`key_pipeline.rs:1799-1802`)と `executor.rs:644-647` は、読めない窓でも `readable=false` で通過マークを立てる。読めない窓でのこのマークの効果は次のとおり:
  - (i) 3 の分岐: `mode_key_pass_expiry_wait_ms` は `readable_at_arm` のときだけ値を返す(`mode_key_pass.rs:122`)ので効かない。
  - (ii) `|| mode_key_pass_mark_live(now_ms)` で poll の連鎖を止める(`mod.rs:1234`)。
  - (iii) 窓の途中で窓が読める側へ格上げされたときの、窓の中の破棄(`should_drop_intents_for_mode_key_pass` の `on_expiry=false` は `readable_at_arm` を見ない)。
  - GJI と MS-IME 本体の読めない窓では、同時に立つ直接観測の窓(1 の分岐)が先に return するので、(ii) は隠れる(同じ時刻に同じ窓幅で立つ)。
  - 残るのは、**ATOK など直接観測の対象外の IME を読めない窓で使う場合に、モードキーの後の poll の連鎖が、次の契機(フォーカス変更など)まで止まる**ことだけ。読めない窓の poll は GJI の I/O 以外に観測を生まないので、害は小さいはず(未確認)。
  - 「読めない窓では立てない」に変えると (iii) の稀な場面が変わる。
  - 通過マークの ablation(`a7-no-follow.sh`)は置換元の文字列が変わって当たらない(ADR-240 §4 の表、`f1ef8d76`)ので、今は e2e の証拠も無い。
  - 判断は ?(a7 を直して `atok-resync*` で回すのが先)。

### DD5-06(K に変更・理由の更新): BUG-163 の揃え(F7)が drift を止める役目は、P6 に吸収された

- **事実(確認)**:
  - `ir_align_placeholder_desired` と `align_placeholder_desired` の doc(`platform_state.rs:799-800`)は「揃えないと…明示意図が無いのに drift 補正が発火する」と書く。
  - しかし ADR-212 P6(`6570c324`、2026-09-30)以降、`evaluate_drift` は `last_intent` が無ければ `NotExplicitIntent` を返す(`drift_correction.rs:136-138`)。F7 の発火条件は `last_intent.is_none()`(`platform_state.rs:812`)で、drift の発火条件と排他になる。
  - F7 の導入は `c7999ce0`(2026-09-25)で、P6 より前。確認に使ったコマンド: `git log -S"explicit_intent != Some(desired)" -- crates`。
  - よって `startup_placeholder_desired_is_not_treated_as_intent` の「align は drift より前」の順序の検査は、drift に関しては荷重を失っている。
- **それでも K とする理由**: F7 は `desired_open` そのものを観測値へ動かす。TsfNative の窓では、`desired_open` を前の窓の値のまま持ち越し、最初のキーで `dispatch_ime` がそれを窓へ書く SSOT モデル(`focus_tracking.rs:670-672`)をとっている。F7 が無いと、IME を閉じて起動し、読める窓で「閉」を観測した後に TsfNative の窓へ移ったとき、初期値 `true` が最初のキーで書かれる。これは BUG-163 の「awase が IME を開ける」と同じ害。
- **すること**: コメント 2 か所と guard の説明文を P6 に合わせて直すだけ(docs のみ)。

### DD5-07(K): `idle_conv_check` の apply 時の 4 つの再検査

- 場所: `key_pipeline.rs:773-869`。
- 4 つの検査: (a) shift ガード、(b) 明示操作の値の一致と抑止の窓、(c) `conv_mutation` の一致、(d) `probe_actuation_fence` の一致。
- (c) と (d) は同じ 2 つの物理境界(`win32.rs:357-368`・`imm.rs:223-236`)で bump されるが、述語が違う:
  - `conv_mutation`: conv を変えうる VK と `IMC_SETCONVERSIONMODE`
  - fence: 開閉のマーカーと GET 以外のすべての IMC
- 互いの部分集合ではない。(d) は open 軸(`VK_IME_ON`・`IMC_SETOPENSTATUS`)を、(c) はマーカーの無い conv 系 VK を拾う。
- `conv_mutation::current()` の読み手は idle_conv_check だけ(`git grep -n "conv_mutation::current()"` で 2 件、どちらも `key_pipeline.rs`)。1 本にまとめるなら、fence の他の 4 つの読み手(`ime.rs:779,783`・`probe_io.rs:285,299`・`platform.rs:671,686`)の abandon が増える。K。

### DD5-08(?): Mozc の共有メモリの追随と drift の順序

- `ir_execute` は観測 → 通知(drift)→ `mozc_shm_on_tick` の順(`ime_refresh.rs:73-80`)。ADR-268 D5 で専用タイマー(`mozc_shm_shadow.rs:70`)も同じ `mozc_shm_read_once` を回すので、**共有メモリの読みの入口が 2 本**ある。
- 読める窓で明示意図が ON のまま外から閉じられた場合を考える:
  - refresh の tick が来れば(キーやフォーカスの契機。明示意図があると定期 poll は止まる、`mod.rs:1250`)、F1 が閉を観測し、W1 が先に開け直す。そのあと F6 の読みは awase の操作より古いので Held になる。
  - 専用タイマーが先に来れば F6 が意図を捨てて、W1 は動かない。
- 結果がタイマーと tick の前後で決まる。`:77-79` のコメントの理由(段階 1 の測定の偏り)は、belief へ接続した今は当てはまらない。
- **案**: `mozc_shm_on_tick` を `ir_stage_notify` の前(`ir_stage_observe` の末尾)へ移す。挙動を変える。
- **判断**: ?、実機要。`tools/adr265_stage2/drive_belief_exp.py e5` を読める窓で回し、閉の直後にキーで refresh を起こす変種で、drift の発火と追随の順を数える。

## 3. 既に挙がっているもの(再報告しない・参照のみ)

- MD-07(最初の打鍵の保持)
- MD-09(焦点直後の読み 5 系統)
- MD-10(DD5-03 で確認した)
- MC-C1(意図の 2 か所)
- MC-C2(DD5-04 はその部分集合)
- MA-05(shadow toggle と `dispatch_set_open`)
- MB-01/02(literal の回復)
- `focus_settle_ms` が drift の延期にしか効かないこと(raw-d)
- ADR-275・R4・MB-06

## 4. K(残す)とした防御と理由

| 防御 | 理由 |
|---|---|
| W1 drift correction | 明示操作の書き込みの送り直しは、これ 1 本だけ。P6 で「推測を書く」場面は既に無い |
| F2 GJI の I/O からの推論 | Blacklist の窓で、入力モードを直す観測はこれしか無い(`eisu_recovery.rs` の表) |
| F3a/b/c | 読める窓専用。b(意図が残って poll が止まる)と c(窓の間の観測が全滅)は、a が拾えない場面のためにある(BUG-158) |
| F4 と F5 | 状態機械は `ExternalChangeWatch` 1 つに統合済み(`external_change_watch.rs:24-43` の `direct` フラグ)。基準値の有無と対象の IME が違う。後半の重複は DD5-04 |
| F6 | 明示意図が残ったままの外部からの閉を拾えるのは、これだけ(ADR-268 の実機 6/6) |
| 焦点時の既定値 4 つ | 分岐で排他(`focus_tracking.rs:633-755`) |
| `eisu_reset_on_ime_on` の 3 種 | 判定は 1 関数にまとまっている。呼び出しは経路ごとの「ON へ遷移した」条件が違うので、1 か所へ寄せられない。guard が対を監視している |
| 半角英数の復元の 2 段(scan 付き `VK_DBE_HIRAGANA` と、IMC の書き込み・確認の 4 回の試行。`key_pipeline.rs:2759-2964`) | 注入を省く場面(Win/Alt を押している間、`effective_open()==false`)では IMC の試行だけが残る。試行は確認ゲートの延長の終わりの合図と、R1 の読み直しの起点も兼ねている。「保険」とコメントされた IMC の効果が新 MS-IME で無いかは実機要(測るもの: 注入を省いた場合に NATIVE の確認が取れた回数) |
| `idle_conv_check` の (c)(d) | DD5-07 |
| panic reset・hook watchdog | 利用者の操作・別の不変条件 |

## 5. 検証に要るもの

1. **DD5-01**: ablation `a12-no-gji-point-sync.sh` と、B2 の e2e 構成(IME ON のまま GJI 種別を検出し直す → 打鍵。`--require-sync` と I4=0)。フォーカス往復(TsfNative⇔Uwp)の構成も要る。どちらも今は無い(`e2e-ime.yml` の `sc-reopen-*` は (c) だけ)。
2. **DD5-02・DD5-03**: 擬似 IME に `applied` の往復(`record_confirmed`・`ImeApplyRequested`・`ReadStrategyFacts`)を写す。ME-2 の範囲で、今はハーネスの冒頭が「写していない」と明記している。DD5-03 は、静的な `Imm32Unavailable` × GJI で「焦点 → `press=None` の ON」が already-matched で省かれた件数を、実利用の journal(`ActuationDecision`)で数える。
3. **DD5-04**: 追加の道具は要らない。Linux の `platform_state` の単体テストと、guard 1 本の書き換え。
4. **DD5-05**: 計画関数を `awase-ime-core` へ出し、決定表を全列挙する(ME-3 の型)。読めない窓の通過マークは、`a7-no-follow.sh` の置換元を `f1ef8d76` 以後の文字列へ直して `atok-resync*` で回すのが先。
5. **DD5-08**: 実機(dragonflyg4、Mozc Readable、読める自前の窓)。`drive_belief_exp.py e5` の変種。
