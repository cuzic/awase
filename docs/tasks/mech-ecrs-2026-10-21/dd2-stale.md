---
title: DD2 — 古い・遅れた・別の窓の観測/結果を今の状態として扱うことへの防御の重なり
status: 調査のみ(2026-10-21、/home/cuzic/rust-nicola-wt-dd = origin/develop e4766b2a)。cargo 未実行。読み取りのみ
created: 2026-10-21
related_adr: ["ADR-089", "ADR-104", "ADR-106", "ADR-108", "ADR-140", "ADR-256", "ADR-260", "ADR-268", "ADR-273"]
---

# DD2 — 古い・遅れた・別の窓の観測/結果への防御(二重の守り)

## 結論

- **E(撤去可)8 件、C(統合可)6 件、?(実機要)1 件**。K(独立に必要)と判定した重なりは 9 組。
- 本番の挙動を変えずに消せる「**恒真の比較**」が 4 つ見つかった(DD2-01〜04)。どれも「呼び出し元が同じ値を 2 回渡している」「同じフレームで読んだ値と比べている」「前段のガードの論理的帰結」で、データの流れを grep で追い切れる。
- 本番に効いている「同じ失効を 2 層で守る」組(admission と store の fence、3 秒の鮮度窓と drift の 1.5 秒、TTL と鮮度窓、scope と focus-change クリア)は、**どれも片方だけが止める具体的な場面がある**ので K。
- 上位 5 件と確度:

| 順 | ID | 内容 | 判定 | 確度 |
|---|---|---|---|---|
| 1 | DD2-01 | `apply_idle_conv_check` の (b2) 経過時間の足切り。spawn 前のガード 4 と (b1) の値一致から常に偽 | E | 高(コードから証明) |
| 2 | DD2-02 | `ConvModeMgr::observe` の fence 比較と `read_at` の単調ガード。2 呼び出し元とも常に一致 | E(または型で置換) | 高。ADR-104 決定 7 の扱いだけ所有者判断 |
| 3 | DD2-03 | 同期 ROMAN 補完の `verify_gen_only`。同じ変数 `focus_gen` を捕獲と照合に渡す | E | 高(コード自身が「原理的に返らない」と書く) |
| 4 | DD2-04 | GjiFsm の `FocusEpoch`(`EndComposition { epoch }`)。照合相手の状態から同じフレームで読んだ値 | E | 高 |
| 5 | DD2-05 | apply 完了の受理で `ImeTransition.focus_epoch` と `focus_generation_watermark` が同じ「FocusChanged 前の apply」を弾く | C | 中(全列挙で確定できる) |

- 見立て: 「二重」の大半は**将来の非同期化に備えた前提工事**(ADR-106 決定 4、ADR-089 §9-22)か、**別の防御の論理的帰結を独立の業務ルールと呼んでいるもの**。前者は所有者が「その非同期化をやらない」と決めれば消える。行数の効果は合計でも本番 約 −150〜200 行と小さい。効果の本体は「読む人が守りの数を誤って数えない」こと。

## 0. 調べ方と主なコマンド

- 3 系統(世代・エポック・フェンス/前景スコープとラッチ/観測の鮮度・TTL・applied 降格・drift)に分けて通読し、同じ不変条件の防御を束ねた。
- 手がかりの grep:
  - `git grep -n '念のため\|多重防御\|belt'` → key_pipeline.rs:2939・platform.rs:1171・vk_send.rs:308・open_chain.rs:582(vk_send.rs:308 は MB-06、open_chain は ADR-180 で結論済みのため除外)
  - `git grep -n '常に一致\|将来この経路に'` → ime.rs:1045・ime_controller.rs:246/263・key_pipeline.rs:3406
  - `git grep -n '二重'` → indicator_glyph.rs:664・ime_model.rs:853/2450・ADR-273 :140/:150
  - `git grep -n '削除の漏れの防御'` → observation_store.rs:643
- 件数の目安(`git grep -c`、crates/ と src/ の .rs): `FocusGen` 68 件/13 ファイル、`ApplyGeneration` 130/13、`FocusFence` 78/13、`probe_admission` 58/17、`ForegroundScope` 80/14、`ScopedOneShot` 15/4、`ModeKeyPassLatch` 26/5、`ExternalChangeWatch` 37/4、`cold_seq` 347/26。
- 除外したもの: INDEX.md 第 1〜3 段(MC-E2 の `expires_at`/`purge_expired` を含む)、ADR-275、R4、MB-06、ADR-180 で結論済みの open_chain 3 関数の再検出、`PressLedger`(二重送信防止なので DD1 の担当)。

## 1. 防御の一覧(系統別)

### 1-A 世代・エポック・フェンス

| 防御 | 場所 | (a) 不変条件 | (b) 単独で止める場面 | (c) 根拠 | (d) 依存 |
|---|---|---|---|---|---|
| `FocusEpoch`(platform_state.focus) | focus_tracking.rs:161 `enter_focus_scope` | プロセスが変わったら別の文脈 | FocusFence・admission の素材 | ADR-106 | FocusGen と同じ refresh で 1 回ずつ進む(DD2-13) |
| `FocusGen`(`Output::ime_mode_focus_gen`) | output/mod.rs:401、ime_refresh.rs:275 経由 | await の後で別プロセスへ書かない | `ActuationTarget`・probe_io・raw_recovery_plan の照合 | ADR-229 W-a | 同上 |
| `FocusFence`(epoch+hwnd) | observation_store.rs、probe_admission.rs | 同じプロセスの中の窓切替も別文脈とみなす | hwnd だけの変化 | ADR-106 決定 3 | `ImmLikeTicket` と store の identity の両方が使う |
| `ImmLikeTicket::admit` | probe_admission.rs:216(呼び出し key_pipeline.rs:454/642/3448、focus_tracking.rs:804) | spawn 時と完了時の fence が一致したときだけ記録 | **完了前にプロセスが変わった場合**。`clear_on_focus_change` 済みの新しいプールに古い High が入り、fence を見ない `most_recent_trusted` の一番手になって `effective_open` が反転する | ADR-106 | store の identity と二重だが K(DD2-K1) |
| `observation_identity_ok` | observation_store.rs:1039 | 記録時の fence が今と一致する観測だけを導出に使う | **同じプロセスの中の窓切替の後**(`update_focus_window` :600-612 はプールを消さない)。admission を通らない timer 経路の MozcShm | ADR-106 決定 3 | 同上 |
| `ActuationTarget::verify_still_current`(async) | ime.rs:1078-1088 | 捕獲から書き込みの間に hwnd/gen が変わったら書かない | hwnd: 同じプロセス内の移動。gen: 捕獲の hwnd 問い合わせ中の切替 | ADR-086 INV-14 | K |
| `ActuationTarget::verify_gen_only`(sync) | ime.rs:1050・1195-1215 | 同上(同期) | **無し**(DD2-03) | ADR-089 Phase C 11・12、2a37140a | 恒真 |
| `ApplyGeneration` + `ImeTransition.focus_epoch` + `focus_generation_watermark` | ime_model.rs:625-655・1099-1105 | FocusChanged 前の apply の完了で新しい窓の `applied` を汚さない | DD2-05 | ADR-108 決定 0-6(ea8a0fae)、d0bc7d34 | 2 つが重なる |
| GjiFsm `FocusEpoch` | tsf/gji_fsm.rs:43・346・911-922 | 前の composition の HIDE で今の composition を終えない | **無し**(DD2-04) | 218a3b4b・11404a53 | 恒真 |
| GjiFsm `ProbeId` | gji_fsm.rs:769-775 | 古い probe の結果を今の warmup に適用しない | warmup_coord の id と FSM の running id は FocusChange リセット等でずれうる | — | K |
| `ShiftConvGuardGen` | key_pipeline.rs:2936-2939 付近 | 所有権の世代(新しい hold の開始でも進む) | フォーカスとは別の軸。`still_owner` を見た後の await 中の変化 | — | K(「念のための多重防御」とコメントされる GenStale も await 後の変化を拾うので完全な重複ではない) |
| `FocusResyncGate` の世代+`compare_exchange` | focus_resync.rs | 期限が先に発火した後の遅い結果を捨てる | admission とは別の不変条件 | BUG-77 | K |
| `conv_mutation` / `probe_actuation_fence` | key_pipeline.rs:648/835・653/869、win32.rs:359-363、imm.rs:223-232 | 読みの間に awase 自身が conv/open を変えたら捨てる | bump の集合が違う(DD2-11) | BUG-34 横展開、ADR-140 | カウンタは K、apply の比較だけ C |
| `StageOrigin` の gen 代用 | raw_recovery_plan.rs:45 | 前景が取れないときだけ gen で古さを見る | 前景窓が記録時・flush 時の両方で取れないとき | raw_recovery_plan.rs:1-10 | K(設計上の単一) |
| `EventOrigin.epoch` | drift_correction.rs:56 | journal 記録専用 | 古さの判定に使っていない | — | 対象外 |

### 1-B 前景スコープ・ラッチ

| 防御 | 場所 | (a) 不変条件 | (b) 単独で止める場面 | (d) 依存 |
|---|---|---|---|---|
| `ScopedOneShot`(post_bypass) | platform_state.rs:2220、message_handlers.rs:388-451 | tmux prefix 後の 1 打鍵だけ素通し、別窓では無効 | スコープだけ(TTL 無しは意図) | 単一。K |
| `ModeKeyPassLatch` | mode_key_pass.rs:58・103 | 通過させたモードキーの直後の観測窓 | スコープと時間は別の不変条件 | 単一。K |
| `ExternalChangeWatch`(ADR-256)の pending write | ① platform_state.rs:223-228 の `clear_awase_writes`、② external_change_watch.rs:96-99 の scope 不一致で破棄、③ :157-161 + `pending_write_in_flight` :119 | 確認待ちの awase の書き込みを別の入力文脈へ持ち込まない | ①: A→B→A の往復と同じ前景の子窓切替。③: 前景変化後 FocusChanged の dispatch 前の遅れ。②: 50ms debounce 内の速い A→B→A かつ B で refresh の読みがあった場合だけ | ② は ①+③ と重なる(DD2-12) |
| `conv_read_ok_scope`(ADR-260) | platform_state.rs:229-232・680-696 | conv の読みは読んだ窓・その窓のフォーカス獲得以後だけ有効 | FocusChanged のクリア: A→B→A。scope 一致: 同プロセス別トップレベル窓・dispatch 前の遅れ | K(両方必要)。`architecture_guard.rs:9156` がクリア文を固定 |
| half_width_alnum の戻り待ち | half_width_alnum.rs:450・522-528・544-551・563 | 戻った窓の最初の打鍵でだけ復元 | scope 完全一致: 別窓。`HWND_CACHE_MAX_AGE_MS`: hwnd 再利用。件数上限: メモリ | 各々別の不変条件。K |
| `lang_detect_focus_in_sync` | lang_detect_shadow.rs:220-227 | 追跡が前景に追いついたか | 唯一の実装(`resume_observation_ticket`・字形 `FenceCtx` が再利用) | K |

### 1-C 観測の鮮度・TTL・applied 降格・drift

| 防御 | 場所 | (a) 不変条件 | (b) 単独で止める場面 | (d) 依存 |
|---|---|---|---|---|
| F1 `clear_on_focus_change` | observation_store.rs:546 | プロセスが変わったら旧プールを全部捨てる | fence を見ない読み手(`most_recent_trusted`・drift の `_excluding`・`read_back`) | F2 と同じ失効だが F2 は derive 系にしか効かない |
| F4 `OBSERVATION_FRESH_WINDOW_MS`=3000 | `derive_filtered` :932、`indicator_reading` :650 | 3 秒より古い観測を High 単独・Medium 合意に使わない | 同じ窓の中で、フォーカス時 1 回きりの ImmCross(High)が新しい Poll に勝ち続ける(BUG-189 型) | 時間の軸。K |
| F5 MozcShm `expires_at`(1000ms) | observation_store.rs:508 | 共有メモリの読みが止まったら 1 秒で消える | 鮮度窓の無い `most_recent_trusted` が古い MozcShm を無期限に採る | F4 と重なるが K |
| F6 `DRIFT_CORRECTION_OBS_MAX_AGE_MS`=1500 | drift_correction.rs:152 | 補正の根拠は 1.5 秒以内 | `_excluding` は期限切れしか見ないので古い Poll で補正が撃たれる | F4 と窓の長さ・順位キー・除外ソースが違う(DD2-K2) |
| 操作直後の柵 S1〜S4 | S1 ime_model.rs:737/764(`KEY_EFFECT_SETTLE_MS`=170)、S2 mozc_shm.rs:313(170、起点は殻の全キー)、S3 ime_model.rs:853-866(`GLYPH_KEY_FENCE_MS`=1300)、S4 indicator_glyph.rs:707-716(1300) | モード入力の直後の読みで belief を押す前へ戻さない | S3 と S4 は同じ起点・同じ定数(DD2-07)。S1・S2・S3 は起点が違う(DD2-15) | |
| `applied` の降格 | ime_model.rs:1096・842・997・1022・1038 | 自分の書き込みの記録を、矛盾・窓切替で未知に戻す | 各々別のイベント | 向き比較付きの 2 か所が同じ式(DD2-14) |
| drift `read_back(Converged)` と `evaluate_drift` | drift_plan.rs、drift_correction.rs | 収束の判定 / ずれの判定 | 送る前の High と送った後の Medium で両者が違う結論になる場面が実在 | K |
| `fresh_evidence_after_giveup`(`EXCLUDED_FROM_ANY_FRESH_EVIDENCE`) | drift_correction.rs:764 付近 | give-up 後の再武装は新しい証拠だけで | BUG-114(ba37fa71)、ADR-268 D2 | 除外リストが drift の除外リストと別物(DD2-06) |

## 2. 候補

### DD2-01(E、高)`apply_idle_conv_check` の (b2) 経過時間の足切りは常に偽

- 場所: `crates/awase-windows/src/runtime/key_pipeline.rs:806-819`(`if explicit_age < EXPLICIT_IME_SUPPRESS_MS { return }`)。
- (a) 明示的 IME 操作から 1500ms 以内の conv の読みを belief に入れない。
- (b) 単独で止める場面は無い。証明:
  1. spawn 前に `should_run_idle_conv_check` のガード 4(src/engine/idle_check.rs:57-61)が、**同じ生値** `last_explicit_ime_action_ms` から求めた age(key_pipeline.rs:556-560)で 1500ms 未満なら spawn しない。`is_first_key_after_focus` が外すのはガード 3 だけ。`apply_idle_conv_check` の呼び出し元は key_pipeline.rs:711 の 1 か所だけで、必ずこの spawn を通る。
  2. apply 時に (b1)(:799)が spawn 時の生値と今の生値を比べ、違えば捨てる。書き手は platform_state.rs:954・1047 の 2 か所で、どちらも tick を代入するだけ。
  3. よって (b2) に届くとき生値は不変で、tick は単調なので `age_apply = now_apply − raw ≥ now_spawn − raw = age_spawn ≥ 1500`。未設定(0)は両方で `u64::MAX`。
- (c) BUG-34 追補(04df0b3f、`git log -S'explicit_action_ms_at_spawn'`)。(b2) を固定するテスト・guard は無い(`architecture_guard.rs` に出現なし)。コメント :806-811 は「独立した業務ルール」と書くが、ガード 4 と (b1) の帰結。
- 案: (b2) の 14 行を消す。代わりに `should_run_idle_conv_check` の doc に「apply 側はガード 4 と (b1) を前提に経過時間を見ない」と 1 行足す。ガード 4 を将来緩める・spawn 条件の外へ出すなら (b2) が要る、という依存を書き残すため。
- 壊れるもの: 無し。
- 検証: `cfg(windows)` なので `cargo check --target x86_64-pc-windows-msvc -p awase-windows --tests --lib`。等価性は上の推論で示せる。補強するなら `should_run_idle_conv_check` を (explicit_age × raw 変化の有無 × tick) で全列挙する Linux テスト。

### DD2-02(E、高。所有者判断 1 点)`ConvModeMgr::observe` の fence 比較と `read_at` 単調ガード

- 場所: `crates/awase-ime-core/src/state/conv_mode.rs:204-226`。
- (a) 別の窓の、または古い conv の読みで `ConvModeMgr` を上書きしない。
- (b) 現状、単独で止める場面は無い。`git grep -n 'conv_mode.observe('` で本番の呼び出し元は 2 か所:
  - key_pipeline.rs:876(idle-conv-check): `fence = accepted.fence`。`admit()` は `fence: current`(probe_admission.rs:231)を返し、同じ `with_app` の同期フレームの中で `self.focus_fence()` と比べるので常に一致。
  - key_pipeline.rs:3409(focus-conv-check): `fence` と `current` に同じ `accepted.fence` を渡す。コメント :3400-3408 が「常に一致」と明記。
  - `read_at` は 2 か所とも apply 時の tick(:851・:3294)。単一スレッドで順に apply されるので単調ガードも成り立たない。
- (c) ADR-106 決定 4(c7f26a97)。ADR-106 自身が決定 4 を「ADR-104 決定 7(focus-conv-check の非同期化、**保留**)の前提工事」と書く。テストは conv_mode.rs 内(:297-373)。
- (d) `ImmLikeTicket::admit` が同じ fence を同じ同期呼び出しの中で先に見ている。
- 案(2 通り):
  - E: `observe(mode, source)` に縮め、`ConvObservation.fence`・`read_at`・引数 `current` を消す。fence 棄却のテスト 3 本(:319・:335・:351)を消す。
  - C(推奨): `observe` が `AcceptedObservation`(admission の witness)を受け取る形にし、`current` 引数と比較を消す。将来 async にしても完了時に `admit` を通らないと witness が得られないので、ADR-106 の狙いは型で残る。
- 所有者判断: ADR-104 決定 7 を正式に見送るか(見送るなら E、残すなら C)。
- 壊れるもの: 無し(テストの付け替えのみ)。
- 検証: Linux の単体テスト(awase-ime-core)。

### DD2-03(E、高)同期 ROMAN 補完の `verify_gen_only` は恒真

- 場所: `crates/awase-windows/src/ime.rs:1050-1057`・`:1185-1215`、呼び出し元 `crates/awase-windows/src/ime_controller.rs:309-318`。
- (a) 捕獲から書き込みまでにフォーカス世代が進んでいたら書かない。
- (b) 無し。`let focus_gen = view.focus.focus_gen;` の**同じ変数**を `capture_blocking(focus_gen)` と `set_ime_romaji_mode_for_target_blocking(target, focus_gen)` に渡すので比較は `focus_gen == focus_gen`。間に await 点も無い。ime_controller.rs:249-262 自身が「`GenStale` は原理的に返らない」、ime.rs:1208 が `TargetMoved` を「構造的に返らない」と書く(ADR-089 §9-22)。
- (c) 2a37140a(`git log -S'fn verify_gen_only'`)。テスト・lint の参照 0 件。
- 案: `verify_gen_only` を消し、同期版は `capture_blocking()`(引数なし)・`set_ime_romaji_mode_for_target_blocking(target)` にして `GenStale`・`TargetMoved` の腕を外す(約 −25 行)。ついでに撤去済みの `consume_force_open_pending` を名指す古い doc(ime.rs:127・1006・1046)を直す。
- 壊れるもの: architecture_guard.rs:3942/3968 は関数名の文字列 `capture_blocking(`・`set_ime_romaji_mode_for_target_blocking(` だけを固定しているので、名前を保てば通る。将来この同期経路に await を入れる人は async の `verify_still_current` を使う、と doc に残す。
- 検証: `cargo check --target x86_64-pc-windows-msvc`。挙動差なし。

### DD2-04(E、高)GjiFsm の `FocusEpoch`(`EndComposition { epoch }`)

- 場所: `crates/awase-windows/src/tsf/gji_fsm.rs:43`(型)・`:346` `bump_epoch`・`:911-922` 照合、`crates/awase-windows/src/platform.rs:829-835`。
- (a) 前の composition セッションの候補 HIDE で今の composition を終えない。
- (b) 無し。本番で `GjiEvent::EndComposition` を作るのは platform.rs:834 の 1 か所だけ(`git grep -n 'GjiEvent::EndComposition' -- crates | grep -v gji_fsm.rs` → 1 件)。その epoch は直前に `gji_current_composition_epoch()` で `GjiState::OnComposing { epoch }`(照合相手そのもの)から読んだ値で、同じフレームで dispatch するので `epoch != current_epoch` は成り立たない。名前は FocusEpoch だが進むのは composition の開始時。
- (c) 218a3b4b・11404a53。テスト `stale_end_composition_is_ignored`(gji_fsm.rs:1745)1 本のみ。guard なし。
- (d) 前セッションの SHOW/HIDE の古さは、`gji_on_ime_off` の `discard_pending_composition_events()`(platform.rs:767)と drain 順の固定(Start→End)が実際に守っている。
- 案: `FocusEpoch`・`bump_epoch`・`OnComposing.epoch`・`EndComposition.epoch`・`gji_current_composition_epoch`(trait の既定実装と GJI 実装)を消し、`gji_on_end_composition` は「`is_composing()` なら dispatch」にする。
- 壊れるもの: テスト 1 本(消す)。
- 検証: gji_fsm は cfg 無しで Linux のテストが走る。残りの EndComposition テストが緑なら等価。

### DD2-05(C、中)apply 完了の受理: `ImeTransition.focus_epoch` と `focus_generation_watermark`

- 場所: `crates/awase-ime-core/src/state/ime_model.rs:620-655`(`completion_can_update_applied`)、watermark の前進 :1099-1105。
- (a) 共通の不変条件: FocusChanged より前に払い出した apply の完了で、新しい窓の `applied` を汚さない。
- (b)
  - watermark だけが止める: Superseded の腕で、新しい窓の pending と target が偶然一致する場合(d0bc7d34 の C2)。
  - epoch だけが止める: 厳密一致の腕で pending が古い窓のもの。ただし `pending.generation ≤ last_seen < watermark` なので `generation >= watermark` に置き換えても同じ。Superseded の腕の epoch 照合も watermark に含まれる。
  - 等価でなくなる例外: (i) `InitialFocusScopeEstablished`(:947-963)は fence の epoch を進めるが watermark を進めない(起動時に pending があるかは**未確認**)。(ii) `checked_next` の u64 上限(実運用で到達しない)。
- (c) ADR-108 決定 0-6(ea8a0fae)・d0bc7d34。guard なし。ime_model.rs 内のテスト(:1339-1346・:3277 付近)。
- 案: 照合を `generation >= watermark` の 1 本にし、`ImeTransition.focus_epoch` を消す。逆(watermark を消す)は C2 が再発するので不可。
- 壊れるもの: 例外 (i) を閉じるため bootstrap の reducer でも watermark を進めると、ime_model.rs:1411 のテスト(「`InitialFocusScopeEstablished` は app_policy・current_focus・fence 以外を書き換えない」)と :1418-1467 の FocusChanged との食い違い検査を更新する必要がある。起動時に pending が無いことを示せれば、bootstrap は触らずに済む。
- 検証: **Linux で全列挙できる**(擬似 IME 不要)。{ApplyRequested(gen,target), FocusChanged, InitialFocusScopeEstablished, 完了(gen, outcome, open)} を長さ 5〜6 まで並べ、旧判定(写しとしてテスト内に残す)と新判定の `ImeApplyAcceptance` の一致を `explicit_press_exhaustive` の方式で示す。

### DD2-06(C、中〜高)観測ソースの性質を `ObservationSource` の網羅的 `const fn` に集める

- 現状: ソースを 1 つ足すたびに 5 か所を手で直している。(1) `observation_identity_ok` の match(fence の対象)、(2) `record_any` の `expires_at` の if(TTL)、(3) `PerSourceObservations::iter()` の除外、(4) `EXCLUDED_FROM_ANY_FRESH_EVIDENCE`、(5) `evaluate_drift` の除外リスト。
- 実例: ADR-268(MozcShm)は 1・2・4・5 を、ADR-273(TaskbarGlyph)は 1・3・5 を同じ PR で直した。`ime_event.rs` の `HeuristicDefault` の doc は「enum に足すだけでは保護対象へ入らない——除外リストも忘れず更新すること」と散文で警告している。
- 案: `impl ObservationSource` に既存の `authority()` と同じ形の網羅的 match で `focus_fenced()`・`ttl_ms() -> Option<u64>`・`drift_basis() -> bool`・`rearm_evidence() -> bool` を足し、observation_store.rs の 2 か所と drift_correction.rs の 2 配列を置き換える。`iter()` は字形専用の枠のまま。各リストの理由コメントは各 variant の腕へ移す。
- 壊れるもの: 無し(`git grep EXCLUDED_FROM_ANY_FRESH_EVIDENCE` は定義のみ。名前・件数のガードなし)。`taskbar_glyph_alone_is_never_a_drift_correction_basis` は `per_source.set` を直接使うので通る。
- 検証: 13 variant の全列挙テスト 1 本で旧リストと新 `const fn` の一致を示す(既存の `read_back_*_matches_the_legacy_predicate_exhaustively`、observation_store.rs:1424/:1492 と同じ方式)。

### DD2-07(E、高。配線前・ADR 追記要)字形の殻の柵 `FenceCtx::since_last_mode_key_ms`

- 場所: `crates/awase-ime-core/src/state/indicator_glyph.rs:707-716`(`TooSoonAfterModeKey`)と reducer 側 `ime_model.rs:853-866`。
- 重なり: 同じ起点(`ImeModel::since_last_mode_key_ms`)・同じ定数 `GLYPH_KEY_FENCE_MS`。ADR-273 :140 が「`fence_glyph` の 3 つの事実は呼び出し側の自己申告なので本当の防御は reducer 側」、:150 が「起点も reducer と揃える」と書く。殻側は `None` を渡せば素通し(:663)。
- 本番への到達: 0 件。`architecture_guard.rs:8955` の `taskbar_glyph_is_not_reported_from_production_yet` が `fence_glyph(` の本番呼び出しを 0 件に固定。
- 案: `FenceCtx::since_last_mode_key_ms` と `FenceMiss::TooSoonAfterModeKey` を消し、`focus_gen_unchanged`・`foreground_in_sync`(reducer から見えない殻の事実なので K)だけを残す。テスト indicator_glyph.rs:2097-2123 は reducer 側(ime_model.rs:2450 以降)に寄せる。
- 壊れるもの: 配線後、殻で落ちていた字形が journal に記録されて reducer で落ちる(`[glyph-fence] dropped` が増える)。belief は同じ。ADR-273 の「二重の柵」の文言を直す追記が要る(所有者確認)。**段階 2 の配線 PR より前**に入れると、自己申告の事実を 1 つ減らした状態で配線できる。
- 補足(K): `focus_gen_unchanged` は store の `observation_identity_ok`(TaskbarGlyph も対象)と似るが、store は「記録時の fence と今」を比べるので、**画面の取得〜報告の await の間の窓切替**は殻の柵しか止めない。

### DD2-08(E、高。休眠中)`indicator_reading` の条件 (2)「指標より新しい Medium 以上」

- 場所: `crates/awase-ime-core/src/state/observation_store.rs:642-643`。コメント自身が「書き込み時の削除(`record_any`)があるので通常は常に真だが、削除の漏れの防御として残す」と書く。
- 推論: 指標より後に記録された同一性の合う Medium 以上は `record_any`(:515-520)が必ず指標を消す。記録時に同一性が合わなかった観測は後から合わない(fence が変わるのは `update_focus_window`〈hwnd が変われば指標も消す〉・`clear_on_focus_change`〈全部消す〉・`establish_initial_fence`〈起動時 1 回〉だけ)。よって (2) が働くのは記録順と `at` が逆転したときだけ。
- 案: `|| o.at > glyph.at` を消して `debug_assert!` に置き換える。
- 未確認: `at` は `envelope.time.monotonic` 由来。journal の再生で逆転しうるかは確かめていない。本番は未配線なので影響なし。

### DD2-09(E、小。DD2-06 と同時)字形の `is_expired` と drift の除外リストの `TaskbarGlyph`

- `observation_store.rs:647` の `glyph.is_expired(now)`: `expires_at` を付けるのは MozcShm だけ(:508)なので常に偽。DD2-06 で TTL を性質表にすれば残しても無害。
- drift の除外リストの `TaskbarGlyph`: コメント自身が「`iter()` に出ないので今は効かない。将来のリファクタへの備え」と書く。DD2-06 の `drift_basis()=false` が同じ保証をコンパイラで与えるので、DD2-06 と同じ PR で消す。

### DD2-10(C、小)`start_ms_ime_ready_poll` の世代照合を先頭 1 回に

- 場所: `crates/awase-windows/src/output/probe_io.rs:204`(読めた腕、`refresh_ime_mode_if_focus_matches` の中)と `:342`(abandon の腕、移植漏れの後で足したもの)。
- (a) 古い世代のポーリングが新しい窓の `ms_ime_gate_give_up`・`ImeModeFsm` に触らない。
- 案: `with_app` の先頭で gen を 1 回照合し、腕ごとの照合をやめる。挙動は同じで、腕を足したときの移植漏れ(BUG-13 の文脈)を構造的に防ぐ。
- 検証: `cfg(windows)`。cargo check のみ。挙動差は推論で示す。

### DD2-11(C、小)idle-conv-check の自己書き込み検出の apply 側の比較

- 場所: key_pipeline.rs:835(`conv_mutation`)・:869(fence の checkpoint 3)。
- 重なり: どちらも「読みの間に awase 自身が書いたら捨てる」。ただし bump の集合が違う: conv_mutation だけが VK_DBE_*・KANA・CONVERT の素の送信を、fence だけが kanji/tsf marker の VK_IME_ON/OFF・IMC_SETOPENSTATUS を拾う(IMC_SETCONVERSIONMODE は両方)。
- 案: **apply 側の比較だけ** `SelfWriteStamp { conv_mut, fence }` の `changed_since()` 1 回にまとめる(約 −20 行、挙動同じ)。カウンタは統合しない。統合すると issue 前の abandon(`get_ime_conversion_mode_fenced_async`、checkpoint 1・2)が shift-conv-guard の VK_DBE_HIRAGANA などでも発火し、abandon 件数と resync の体感遅延が変わる(probe_actuation_fence.rs の doc 決定 I)。
- 固定: architecture_guard.rs:9652 `probe_actuation_fence_bump_precedes_send_input`、:3434-3481。比較の形を変えるときこれらの文字列一致を確認すること(未確認)。

### DD2-12(E、小。確度 中)ADR-256 の pending write の scope 破棄 ②

- 3 つの防御(1-B の表)のうち、② `confirm_awase_write` の `p.scope != scope` 分岐(external_change_watch.rs:96-99、3 行)だけを消し、①③を残す。
- ② だけが止めるのは「50ms debounce 内の速い A→B→A(FocusChanged 無し)で B の refresh の読みがあった」場合だけで、そのとき宛先は再び A の文脈なので、破棄はむしろ過剰。消すと A の pending が A の直接観測の窓へ持ち越される(① が focus イベント無しで行うことと一致)。
- 壊れるもの: ② を前提とする `confirm_*` 系のテスト(詳細未確認)。`architecture_guard.rs:4939` の ① の固定は影響なし。
- 価値は小さい。残して「3 つそれぞれの理由」を doc に書くだけでもよい。

### DD2-13(C、低〜中。実機要)`FocusGen` と `FocusEpoch`

- 進む場所: epoch は focus_tracking.rs:161(`process_changed` と bootstrap)、gen は ime_refresh.rs:275 の `if focus.focus_changed` → `gji_on_focus_change` → output/mod.rs:401。**同じ refresh の Phase 2 と Phase 3.7 で 1 回ずつ**進む。
- 差: (i) bootstrap で epoch だけ進む。(ii) Phase 2〜3.7 の間に捕獲したものは gen だけ古くなる。(iii) 置き場所(Output か PlatformState か)。raw_recovery_plan.rs:6-9 が書く「debounce 50ms と prefetch の後でしか進まない」遅れは epoch も同じ。
- 消費者: `ActuationTarget`(conv_actuation.rs:170・executor.rs:763・key_pipeline.rs:2858)、probe_io.rs:204/282/342、platform.rs:647/698、literal_facts.rs、raw_recovery_plan::StageOrigin、`DriftEdge.focus_gen`(ログのみ)、journal の `focus_gen: u32`(journal.rs:386、形式に触る)、`indicator_glyph::FenceCtx`(未配線)。
- 案: `ime_mode_focus_gen` を消し、`focus_epoch` を Output へ値で渡す(または `with_app` で読む)。`FocusGen` 型は epoch の newtype として残す。
- 危険(具体): (ii) の場合、Phase 3 の drift correction などが Phase 2 の後に古い gen を捕獲すると、今は 3.7 で `GenStale` として中止される。統合後は新しい窓へ書く。`ir_notify_focus_changed` が `discard_actuation()` するので同じ tick で起案は無いと**推定**するが未確認。journal の `focus_gen` の値の意味が変わる。
- 壊れるもの: `layer_boundary_guard.rs:247` が `with_app(|runtime| runtime.platform.output.ime_mode_focus_gen.get())` の文字列を固定。
- 実機要: 窓切替を伴う e2e-ime の既存ターゲット(`warmup_gate_focus_scope` など)で `[imm-romaji] Aborted(GenStale)`・`[msime-ready] Stale`・`[giveup-follow] StaleFocus` の件数を統合前後で比べる。

### DD2-14(C、低)`applied` の写しの match と降格の書き方

- executor.rs:830-848 の outcome→effective の手書き match は `ime_model::apply_result_effective_open`(ime_model.rs:124)と同一(`unreachable!()` 付き)。`apply_result_effective_open(open, outcome).map(...)` 1 行にできる。
- reducer の向き比較付き降格 2 か所(ime_model.rs:996-998・:1022-1028)は同じ式 `applied.applied_open().is_some_and(|a| a != v)`。`reduce_demote_applied_if_contradicts(v)` にまとめる(`reduce_` 接頭辞にして `reduce_helpers_are_called_only_from_reduce_body` に載せる)。
- 壊れるもの: `applied_direct_assignments_are_accounted_for` の ime_model.rs=10 が 9 になる(同じ PR で更新)。
- 検証: ADR-208 L0 の全列挙(`tests/explicit_press_exhaustive.rs`)が reduce を通る。executor 側は cargo check と windows-build CI。

### DD2-15(? 実機要)操作直後の柵 S1・S2・S3 の起点

- 3 つの柵は同じ不変条件(モード入力直後の読みで belief を押す前へ戻さない)だが起点が違う。S2(mozc_shm.rs:313)だけが殻のグローバル `hook::last_operation_tick_ms`(全キー+ImmCross)。S1 は予測の時刻で、後から `ImeApplyRequested` が来ても延長されない。
- 揃える案: 起点を `last_mode_key_tick_ms` に統一。S2 は「モード系以外のキー」を起点にしなくなるので、awase が分類しないキーが開閉を変える場合(GJI の利用者独自のキー割り当て等)に処理前の読みを採りうる。その読みは旧状態=belief と同じなので追随は起きず無害の見込み(未確認)。
- 窓の値(170/170/1300)は測定の根拠が別々なので統一しない。
- 実機要: mozc-readable × 利用者独自キーで `[mozc-shm]` の Held/Use の件数を比べる(既存の測定は ADR-268 段階 2)。

## 3. K(独立に必要)と判定した重なり

| ID | 重なり | 片方だけが止める場面 |
|---|---|---|
| DD2-K1 | `ImmLikeTicket::admit`(殻)と `observation_identity_ok`(store) | admit のみ: 完了前のプロセス変更。古い High が新しいプールに入り、fence を見ない `most_recent_trusted` の一番手になって `effective_open` が反転し、`update_drift`・`reconcile_key_effect_open`(ime_model.rs:869-880)も動く。identity のみ: 同じプロセスの中の窓切替の後に残る同期記録の観測 |
| DD2-K2 | `OBSERVATION_FRESH_WINDOW_MS`=3000 と `DRIFT_CORRECTION_OBS_MAX_AGE_MS`=1500 | 順位キー(High 単独→Medium 合意 / 信頼度→時刻)と除外ソースが違う。窓を 1 つにすると drift の根拠が 3 秒前まで広がり補正が増える、または belief のフォールバックが早まる |
| DD2-K3 | MozcShm TTL と 3 秒の鮮度窓 | TTL のみ: 鮮度窓の無い `most_recent_trusted` |
| DD2-K4 | `conv_read_ok_scope` の FocusChanged クリアと scope 一致 | クリア: A→B→A。scope: 同プロセス別トップレベル窓・dispatch 前の遅れ。`(scope, epoch)` の組で 1 比較にできるが利得ほぼ無し、`:9156` の guard 書き換えが要る |
| DD2-K5 | async `verify_still_current` の hwnd と gen | hwnd: 同プロセス内の移動。gen: 捕獲の hwnd 問い合わせ中の切替 |
| DD2-K6 | `ShiftConvGuardGen` の GenStale(「念のための多重防御」key_pipeline.rs:2939)と `still_owner` | `still_owner` を見た後の await 中の変化。ただし中止されても同じ target で次の試行に進むだけで、効果は 1 回の書き込みを飛ばす程度 |
| DD2-K7 | drift の `read_back(Converged)` と `evaluate_drift` | 送る前の High(1.5 秒以内)と送った後の Medium で結論が分かれる |
| DD2-K8 | `FocusResyncGate` 世代と admission | 期限の先行発火後の遅い結果(別の不変条件) |
| DD2-K9 | `ScopedOneShot` があるのに 3 か所(`ExternalChangeWatch.armed`・`PendingWrite`・`conv_read_ok_scope`)が scope 失効を手書き | `armed` は scope を取らないアクセサ(`baseline()` 等)と延長上限 `MAX_EXTENSION_FACTOR` を持つので素直に置き換えられない。約 −10 行で K。ただし同じ定数 `MODE_KEY_PASS_MARK_WINDOW_MS` の境界が `ModeKeyPassLatch::live`(`age < window`)と `ExternalChangeWatch::remaining_ms`(`age > window` で失効)で 1ms ずれている。実害なし、揃えるなら次に触るとき |

## 4. 範囲外で気づいたこと(二重ではなく片側の欠落の疑い)

- `most_recent_trusted`・`most_recent_trusted_excluding`(drift)・`read_back` は fence(hwnd)を見ない。同じプロセスの中の窓切替の後も、旧 hwnd の ImmCross(High)がフォールバックと drift の根拠に残りうる。hwnd だけが変わるときに `FocusChanged` が届くかは未確認。**二重の逆(守りが無い)の疑い**なので、統合の前に別途確認すること。
- `IDLE_CONV_CHECK_IN_FLIGHT_STALE_MS`(platform_state.rs:2265)と完了時の epoch 棄却は in-flight ラッチの自己回復で、観測の鮮度ではない(BUG-34 の実測 ~5741ms に基づく)。K。

## 5. 推奨する順序

1. DD2-01・DD2-03・DD2-04(恒真の比較の撤去。挙動差なし。awase-windows で 1 PR、gji_fsm は Linux テストで確認)。
2. DD2-02(所有者に ADR-104 決定 7 の扱いを聞いてから E か C)。
3. DD2-06+DD2-09(性質表。全列挙テスト 1 本を先に入れる)。
4. DD2-05(全列挙テストを先に入れてから照合を 1 本に)。
5. DD2-07・DD2-08 は ADR-273 段階 2 の配線 PR の前に。
6. DD2-10・11・12・14 は小さいので、その領域を触るときに。DD2-13・15 は実機の測定が出るまで保留。

## 6. 検証に要るもの

- **Linux で足りる**: DD2-02・04・05・06・08・09・14(awase-ime-core / gji_fsm の単体テストと全列挙)。DD2-05・06 は旧判定をテスト内に写しとして残し、新判定と全列挙で突き合わせる(`tests/explicit_press_exhaustive.rs`・`read_back_*_exhaustively` の方式)。擬似 IME は不要。
- **cargo check(Windows ターゲット)+推論で足りる**: DD2-01・03・10・11・12(`runtime/`・`ime.rs`・`output/` は `cfg(windows)`。`cargo check --target x86_64-pc-windows-msvc -p awase-windows --tests --lib`、実行は windows-build CI)。DD2-01 は `should_run_idle_conv_check` の全列挙を Linux に足すと補強になる。
- **擬似 IME で検証できないもの**: DD2-K1・DD2-13。殻の admission・`Output::ime_mode_focus_gen`・`ActuationTarget` は `runtime/`・`output/`・`ime.rs` にあり、ハーネス(tests/support/harness.rs)を 1 本も通らない。非同期の完了とフォーカス変更の交錯を模擬する能力(INDEX の ME-2 相当+窓モデル ME-5)が要る。
- **実機要**: DD2-13(e2e-ime の窓切替系ターゲットで GenStale/Stale/StaleFocus の件数比較)、DD2-15(mozc-readable で `[mozc-shm]` Held/Use の件数比較。擬似 IME に共有メモリの遅れの模擬が無い=ME-6 相当)。
- **壊れるものとして同じ PR で直すガード**: `architecture_guard.rs:3942/3968`(名前を保てば無影響、DD2-03)、`layer_boundary_guard.rs:247`(DD2-13)、`architecture_guard.rs:8955`(DD2-07 は本番 0 件の固定なので無影響)、`applied_direct_assignments_are_accounted_for`(DD2-14、10→9)、ime_model.rs:1411・1418-1467 のテスト(DD2-05 で bootstrap を触る場合)、conv_mode.rs:319/335/351(DD2-02)、gji_fsm.rs:1745(DD2-04)、indicator_glyph.rs:2097-2123(DD2-07)。
