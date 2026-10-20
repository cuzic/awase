# ECRS 調査: awase-windows の focus/ と state/(殻側)

対象 worktree: `/home/cuzic/rust-nicola-wt-survey`(origin/develop a15081ee)。読み取りのみ。cargo は実行していない。

## 調査範囲

| 場所 | ファイル | 行数 |
|---|---|---|
| `crates/awase-windows/src/focus/` | cache 216 / classifier 775 / classify 140 / current 226 / imm_learning 84 / kind_classifier 87 / mod 31 / msaa 79 / probe 72 / thread_scope 449 / tracker 492 / uia 306 | 計 2,957(12 ファイル) |
| `crates/awase-windows/src/state/` | platform_state 5,030 / sync_actuation 757 / ime_decision_view 153 / mod 23 / platform_state/shell 223 | 計 6,186(5 ファイル) |

読んだ範囲:
- focus/ は全ファイルを通読した。ただし `thread_scope.rs` の Windows 専用 `mod win`(330 行以降)と `cache.rs` の tests は読んでいない。
- state/ は `platform_state.rs` の本番部分(1–2337 行)、`sync_actuation.rs` の本番部分(1–393 行)、`ime_decision_view.rs`、`shell.rs` を通読した。

読めなかった範囲:
- `platform_state.rs` の tests(2338–5030 行)は関数名の一覧を見ただけで、本文は読んでいない。
- `sync_actuation.rs` の tests(395–757 行)は読んだが、重複の検討はしていない。

## 先に分かった構造上の事実(候補の前提)

殻側に残った理由を調べた結果は次のとおり。

1. `state/platform_state.rs` と `state/sync_actuation.rs` の本番部分は、Windows 依存がゼロである。
   - 本番部分 1–2337 行にある `crate::` 参照を数えると、すべて core にあるモジュール(`state::*`、`tuning`、`journal`、`keymap`、`focus::hwnd_cache`)か `focus::{AppKind, FocusKind}`(core 再公開)である。
     コマンド: `sed -n 1,2337p state/platform_state.rs | grep -oE "crate::[a-z_]+(::[a-zA-Z_]+)?" | sort | uniq -c`
   - `crate::with_app`・`crate::vk::is_ctrl_variant`・`crate::hook::current_tick_ms` は doc コメントにだけ出る。
   - `crate::IME_DETECT_MISS_THRESHOLD`(platform_state.rs:1636)は `lib.rs:124` の再公開で、実体は core の `tuning.rs:277`。
   - `state/mod.rs:1-6` に、残した理由が書いてある。「記録系の `pub(crate)`(INV-A97-1)をコンパイラが強制し続けるため(所有者の決定、Opus 案 (d))」である。
   - つまり OS 依存ではなく、可視性の都合で殻に残っている。
2. Windows 依存を持つのは `platform_state/shell.rs` だけである。ここは `crate::win32::foreground_scope()` を読んで核の `_in_scope` 版へ渡す 19 個の `pub(crate) fn` と、実時計の構築口(`ImeStateHub::new`・`PlatformState::new`・`Default`)でできている。
3. `state/ime_decision_view.rs` が `#[cfg(windows)]` なのは、`ObservedState::from_snapshot` が `crate::tsf::observer::TsfObservations` を読むからである。`ControlLog`・`FocusFacts`・`ImeControlView` と `From<&ImeControlView> for DecisionInputs` は純粋である。
4. `focus/` の `classifier.rs`・`tracker.rs` は `#[cfg(windows)]` で、Linux のテストバイナリには存在しない(`focus/mod.rs:18-31`)。ところが中身の大半は純粋で、tests も Win32 を使わない(`classifier.rs:553` のコメントは「Linux 上でも実行できる」と書くが、モジュール自体が gate されているので実際は走らない)。BUG-56/107/108 の回帰テストは windows-build CI にしか効いていない。

## 候補表(効果の大きい順)

安全網の凡例は brief どおり(強=Linux 単体テスト/golden、中=guard の文字列走査か CI 実機のみ、無=無し)。

| ID | 種別 | 場所 | 内容 | 根拠 | 規模 | 安全網 | 再発ファミリー | リスク |
|---|---|---|---|---|---|---|---|---|
| F1 | E | `focus/uia.rs` 全体(306 行)、`runtime/message_handlers.rs:1106-1140`(`handle_wm_focus_kind_update`)、`app/mod.rs:616-620`、`app/bootstrap.rs:1376-1378`、`runtime/focus_tracking.rs:100-102,860-862`、`focus/tracker.rs:354-363`、`runtime/mod.rs:2125-2130`、`platform.rs:1396-1400`、`lib.rs:301-312`、`tests/architecture_guard.rs:2452-2490` | UIA 非同期分類の worker スレッド・チャネル・`WM_FOCUS_KIND_UPDATE` は、結果を適用せず debug ログを出すだけ。パイプライン全体を撤去する。 | 受け側 `handle_wm_focus_kind_update` は `let _ = app;` で結果を捨て、`tracing::debug!` だけを出す(message_handlers.rs:1128-1139)。`docs/known-bugs/BUG-012.md` が「worker は診断ログ用に稼働継続」と明記。`grep -rn "try_send_uia\|set_uia_sender\|spawn_uia_worker" crates src tests` の呼び出し元は focus_tracking.rs 2 箇所、bootstrap.rs 1 箇所、中継 3 層(tracker/platform/runtime)で、いずれも送信または配線のみ。`cache.rs:8-12` は UiaAsync 階層を既に「構築サイトゼロ」として撤去済み。 | −440(uia 306、handler 35、中継 40、定数 12、guard テスト 45) | 中(guard テスト 1 本が handler 本体を固定。撤去時にそのテストも消す) | focus 遷移 | BUG-012 は「安全に再有効化するには hwnd 粒度の別設計が必要」として worker を残した。撤去すると再有効化の足場が消える(設計が要るので足場は小さい)。COM スレッド 1 本と 50ms ポーリングが消える利点もある。所有者判断が要る。 |
| R1 | R | `state/platform_state.rs:1-2337`(本番)+ tests 2338-5030、`state/sync_actuation.rs` 全体 | `ImeStateHub`/`PlatformState`/`FocusStore`/`GateStore`/`KeymapStore` と同期 actuation の核を `awase-windows-core` へ移す。殻には `shell.rs` だけが残る。 | 上の事実 1。`crate::` 参照は全て core 内。`IME_DETECT_MISS_THRESHOLD` は core の `tuning.rs:277` にある。 | 移動 約 5,800 行(削減ではなく殻の縮小。殻の 6,186 行のうち約 94%) | 強(platform_state の tests 約 2,700 行と sync_actuation の tests が Linux で走る。移動後も同じ) | IME belief / actuation 合流点 | 高。`state/mod.rs:1-6` の所有者決定(Opus 案 (d))の反転になる。core に置くと記録系 `record_*` が `pub` になり、INV-A97-1 の `pub(crate)` 強制を失う。`architecture_guard.rs` は `src/state/platform_state.rs` を約 20 箇所で直接読む(grep: 566, 625, 656, 697, 811, 1016, 1202, 1229, 1251, 1298, 1374, 1401, 1952 ほか)ので、パス更新が要る。F2 とセットでないと進められない。 |
| R2 | S/R | `state/platform_state/shell.rs`(223 行)、`platform_state.rs` の `*_in_scope` 15 個、`architecture_guard.rs:2200-2317` | `HubClock::wall(fn, fn)` と同様に前景スコープの取得関数を `ImeStateHub` に注入し、各メソッドが内部で読む形にする。`*_in_scope` と同名ラッパーの二重定義、およびラッパーの形を縛る guard が不要になる。R1 の前提でもある。 | shell.rs の `pub(crate) fn` 19 個のうち 17 個が「`foreground_scope()` を 1 回読んで `_in_scope` を 1 回呼ぶ」だけ。guard `shell_shape_violation`(architecture_guard.rs:2200-2264)がその形を要求。`grep -n "foreground_scope()" -r src` で ImeStateHub 以外の読み手は output/mod.rs:1243、lang_detect_shadow.rs、resume_hold.rs。 | −180(shell.rs)、−100(guard)、メソッド名 15 個の二重管理が消える | 強(`_in_scope` の単体テストが多数ある。注入に変えればテストは偽スコープで書ける) | IME belief | 中。FCIS P2 で意図して分けた形(「OS を読む殻を 1 行にする」)の変更。呼び出しごとに読む挙動は保てるが、guard の再設計が要る。 |
| F3 | R | `focus/classifier.rs:46-58,210-546`(`ImmCapability`・`InjectionHint`・`ImmCapabilityStore`・`InjectionModeStore`・`save_section`、tests 548-775)、`focus/tracker.rs:158-196,209-224`(`apply_learned_imm_capability`・`should_log_demotion` と tests) | Win32 を使わない部分を `#[cfg(windows)]` の外(ungated なファイル)へ分ける。`ForceOverrides` と `input_site_fallback_matches` だけが `crate::ime`/`classify` を使うので classifier に残す。 | classifier の純粋部分が使うのは `std::fs`・`toml`・`awase::fs_atomic` のみ(grep `windows::\|crate::ime\|crate::win32` で該当は 92-100 行と 150・190 行の `ForceOverrides` 側だけ)。`focus/mod.rs:18-31` の gate のせいで、BUG-56/107/108 の tests(約 230 行)と tracker の tests が Linux で走らない。 | 移動 約 600 行。削減なし。得るものは Linux で走るテスト約 14 本 | 強(テストは既存) | focus 遷移 / IME belief(imm 学習降格) | 低。ファイル分割と `mod.rs` の gate 変更のみ。core に移す場合は `toml` 依存の追加が要る(ungated のまま awase-windows に置けば不要)。 |
| F4 | E | `platform.rs:1352`(`update_focus_info`)、`focus/tracker.rs:119-121`(`FocusTracker::update`)、`focus/current.rs:46-49`(`CurrentFocus::update`) | 呼び出し元ゼロの 3 段の転送。 | `grep -rn "update_focus_info(" crates src tests` → 定義 1 件のみ(他は doc コメント)。`grep -rnE "\.update\(" crates/awase-windows crates/awase-windows-core` → `platform.rs:1353` の `self.focus.update(` 1 件のみ(その呼び出し元が上記で 0)。実際の経路は `update_focus_info_with_process_name`(focus_tracking.rs:426)。`pub fn` なので dead_code 警告は出ない。 | −16 | 無(呼び出し元が無いので不要) | focus 遷移 | 極小。外部 crate(tests/e2e)からの参照が無いことは上の grep で確認済み。 |
| S1 | S | `state/platform_state.rs:1539-1563` | `reset_detect_state`・`on_ime_toggled`・`on_set_open_requested` の 3 つは本体が同一(後 2 つは前者を呼ぶだけ)。 | 呼び出し元: `on_ime_toggled` は key_pipeline.rs:1228 の 1 件、`on_set_open_requested` は platform_state.rs の `handle_engine_set_open` 内 1 件、`reset_detect_state` は focus_tracking.rs:856 と key_pipeline.rs:3262。`grep -rnw` で確認。名前の差は意味(トグル適用後/SetOpen 要求後)だけで挙動は同じ。 | −10 | 強(`handle_engine_set_open_*` の tests) | IME belief | 低。意味の違いを名前で残したい場合は C にしない(未確認: 所有者の意図)。 |
| C1 | C | `focus/tracker.rs:100-132` | `injection_hint()` は `is_focused` の確認を足して `injection_hint_for(self.pid, class)` と同じ本体を重複して持つ。 | 差分は先頭の `if !self.current.is_focused() { return Default }` だけ。後半(overrides → `injection_mode_store.has_tsf`)は文字どおり同一(tracker.rs:108-117 と 124-131)。 | −10 | 無(tracker は gate され、`injection_hint` に tests が無い。F3 の後なら純粋関数にして強にできる) | キー選択(注入モード) | 低 |
| C2 | C | `focus/classifier.rs:141-166` | `check_app_override` の 2 つの for ループは `matches_override_entry`(79-88)と同じ比較(`eq_ignore_ascii_case` の process と class の両方)を手で展開している。 | 比較式が逐語で同じ。差は戻り値(`TextInput`/`NonText`)のみ。`grep -rn "check_app_override\|ForceOverrides"` で呼び出しは `tracker.rs:236` と 2 つの生成点のみ。 | −10 | 無(`ForceOverrides` に tests が無い) | focus 遷移(`override_check`) | 低。先に C2 の tests を足すと安全。 |
| C3 | C | `focus/probe.rs:22-72` | `run_focus_probe_async` と `read_focus_snapshot` は、クロージャ本体(`get_gui_thread_info_with_timeout` → `focused_hwnd` の無い場合の空 `FocusSnapshot` → pid/class 取得)が同一。 | 差は `Some(..)` で包むかだけ。呼び出し元: `read_focus_snapshot` は focus_tracking.rs:110,893 の 2 件、`run_focus_probe_async` は runtime/mod.rs:1095 の 1 件。 | −15 | 無 | focus 遷移 | 低。共通の `fn snapshot_blocking() -> FocusSnapshot` を抜き出す。 |
| C4 | C | `state/sync_actuation.rs:227-236` と `317-343` | 「試行ゼロの `ActuationDecisionRecord`」(`chain: [None;..]`・`attempts: [None;..]`・len 0)を 2 箇所で別々に組み立てている。 | 前者は `sync_record(inputs, order_record, &[], [None; MAX], 0)` 経由、後者は構造体リテラル。違いは `site`(`Sync` と `DispatchImeSetOpen`)のみ。 | −12 | 強(`dispatch_gate_rejection_journals_a_dispatch_site_record` と `apply_sync_input_relay_is_not_owned_without_any_attempt`) | IME actuation 合流点 | 低。ただし ADR-163 が `DecisionSite` を区別するために 2 つに分けた経緯がある(sync_actuation.rs:318-327 のコメント)。`site` を引数に取るヘルパーに留め、記録の中身は変えないこと。 |
| R3 | R | `state/ime_decision_view.rs:19-35,113-153` | `ControlLog`・`FocusFacts`・`ImeControlView`・`From<&ImeControlView> for DecisionInputs` を core へ移し、`ObservedState::from_snapshot`(`tsf::observer` を読む)だけを殻に残す。 | gate の理由は `from_snapshot` の `TsfObservations` 参照だけ(ime_decision_view.rs:6,9)。`ActiveImeKind`(tsf/observer.rs:724)が Windows 側にあるので `ObservedState` 自体は移せない。 | 移動 約 60 行 | 無(現状は Linux で `From` impl のテストが無い) | IME actuation 合流点 | 低。`ObservedState` を分けるか `ActiveImeKind` を core へ移す必要があるため、効果は小さい。未確認: 移動後の可視性(`pub(crate)`→`pub`)。 |
| R4 | R | `state/platform_state.rs:1310-1323`(`correction_for_imm_broken`) | `InputModeState` だけの純粋関数。core の `eisu_recovery.rs`(同じ eisu guard を扱う)へ移す。doc コメントが存在しない `ImeBelief::correction_for_imm_broken` を参照している(stale)。 | `grep -n "fn correction_for_imm_broken" crates/awase-windows-core/src/state/belief.rs` → 無し(旧 `ImeBelief` 版は撤去済み)。呼び出し元は `runtime/ime_refresh.rs:587` の 1 件。 | −14(+core 側に 14) | 強(eisu_recovery の tests に足せる) | IME belief | 低 |
| S2 | S | `focus/msaa.rs:19-52` | `msaa_classify` の match 4 腕のうち 3 腕が `ClassifyResult { kind: decision.kind(), reason: ... }` の同形。ログ文言の差だけ。 | 目視で同形。`decide_msaa_role` は core の `state/msaa_role_plan.rs`(F5a)。 | −12 | 強(`msaa_role_plan` の単体テスト。`msaa_classify` 自体は Win32 なので中) | focus 遷移 | 低。ログ文言は変えないこと。 |
| S3 | S | `focus/kind_classifier.rs:19-24` | doc が「純粋関数(副作用なし)」と書くが、実際は `platform.is_engine_processing()` を読み、300ms の worker で `classify_focus` を実行する(observe + execute を含む)。誤解を招く。 | 同ファイル 36-69 行。判断本体は core の `decide_resolution`。 | ±0(文言修正) | 無 | focus 遷移 | 無 |
| R5 | R | `focus/thread_scope.rs:1-228,230-328` | 純粋部分(`ThreadScope`・`ClosedAssumption`・`classify_thread_scope`・`SeenThreads`)を core の `focus/` へ移し、`mod win` だけを殻に残す。`cfg(any(windows, test))` の gate が混在していて読みにくい。 | 純粋部分は std と `state::ime_kind::ImeKindId` のみ。`mod win`(330 行以降)は未読。呼び出し元は focus_tracking.rs:569,716-721 と tracker.rs:32,60。 | 移動 約 230 行 | 強(tests 既存、ungated なので Linux で走る) | focus 遷移(新スレッドは閉の仮定、ADR-212) | 低 |
| S4 | S | `state/platform_state.rs:2259-2265` と `runtime/key_pipeline.rs:615-632` | `idle_conv_check_in_flight_since_ms` の「スキップ/再武装」判断(8 秒の自己回復)が key_pipeline の中に直書きされている。純粋な `Skip/Rearm/Fresh` の関数にして core に置ける。 | 判断は 15 行で、入力は `(since, now, STALE_MS)` だけ。`IDLE_CONV_CHECK_IN_FLIGHT_STALE_MS` の参照は key_pipeline.rs:619 の 1 箇所。 | ±0(core へ +20、key_pipeline −15) | 無 | キー選択ではなく defer/replay 周辺 | 未確認: key_pipeline 側は runtime 担当の調査範囲。ここでは存在だけ記す。 |

表は 15 行。25 件の上限には届いていない。残りは裏付けが取れなかった。

## 上位 5 件の詳述

### F1: UIA 非同期パイプラインの撤去(E)

何をするか。
- `focus/uia.rs` の worker スレッドと `SendableHwnd`、`try_send_uia`/`set_uia_sender` の 3 層の中継、`WM_FOCUS_KIND_UPDATE` とその定数、`handle_wm_focus_kind_update`、および `architecture_guard.rs::uia_async_focus_kind_handler_does_not_write_belief` を削除する。
- `focus_tracking.rs` の 2 箇所にある `else if classified.kind == Undetermined { try_send_uia(..) }` を消す。
- `BUG-012.md` の「関連ファイル」を更新する。

なぜ安全か。
- 受け側は結果を捨てる実装で、書き込みは無い。guard の禁止リスト(`dispatch_event(`・`reduce(`・`.belief.` ほか)も、書き込みが無いことを今日まで固定している。
- 同じ理由で `cache.rs:8-12` の `UiaAsync` 階層は撤去済みである。
- 撤去で挙動が変わるのは debug ログ 2 行(`UIA async result received ...`)と、COM スレッド 1 本(50ms ポーリング)の消滅だけである。

先に要るもの。
- 所有者が「BUG-012 を再設計して UIA を再有効化する」計画を持っているかの確認。BUG-012.md は「v2 での実機再確認の記録は無い」とだけ書く。
- `grep -rn "FocusKind::from_u8" crates` で `from_u8` が他に使われているか確認する。使われていなければ連鎖して削れる(今回は未確認)。

### R1: `platform_state.rs`・`sync_actuation.rs` の core への移動(R)

何をするか。
- `ImeStateHub` と `PlatformState` 系の本体、`sync_actuation.rs` を `awase-windows-core/src/state/` へ移す。
- 殻は `shell.rs` 相当(実時計の構築口と `foreground_scope` の供給)だけにする。

なぜ成立するか。
- 本番コードに Windows 依存が無いことは、上の事実 1 の grep で確認済みである。
- `quanta` は core が既に依存している。

先に要るもの。
- R2(前景スコープの注入)を先にやる。やらないと `impl ImeStateHub` のメソッドを殻と核に分けられない(Rust は外部 crate の型に inherent impl を書けない)。
- INV-A97-1 の代替を決める。`record_*` を `pub` にする代わりに、`pub(crate)` を保てる別の仕組み(例: `ImeStateHub` を持つ側の crate を限定する sealed な trait)が要る。所有者決定(d)の見直しなので、ここは設計判断である。
- guard のパス更新(約 20 箇所)と、`production_hub_is_unreachable_from_outside_the_crate` の再検討。

規模は大きいが、判断としては最も重い。小さい候補から先に進めるのが良い。

### R2: 前景スコープの注入(S/R)

何をするか。
- `ImeStateHub` に `fn() -> ForegroundScope` を持たせる(`HubClock::wall(crate::hook::current_tick_ms, Instant::now)` と同じ書き方)。
- `*_in_scope` を本名にして `shell.rs` の 17 ラッパーを消す。
- テストでは偽スコープを注入する。

なぜ安全か。
- 実機では呼び出しごとに `foreground_scope()` を読む点は変わらない。
- 既存の `_in_scope` の単体テストは、スコープを引数で渡す形から注入に変えるだけで残る。

先に要るもの。
- `architecture_guard.rs` の `shell_shape_violation`・`shell_methods_only_read_scope_once_and_delegate` と `RECORDERS` 系の件数固定(2154-2190 付近)を、注入版の不変条件(「スコープ取得は構築口だけ」)に書き換える。
- 未確認: FCIS P2 の設計意図(殻を 1 行にする)と衝突しないか、所有者に確認する。

### F3: 純粋な学習ストア・降格判定を ungated にする(R)

何をするか。
- `classifier.rs` から `ImmCapability`・`InjectionHint`・`ImmCapabilityStore`・`InjectionModeStore`・`save_section` を、`#[cfg(windows)]` の外のファイル(例: `focus/learning_store.rs`)へ分ける。
- `tracker.rs` の `apply_learned_imm_capability` と `should_log_demotion` を同様に出す(`AppImeProfile` は core にある)。

なぜ安全か。
- 中身は変えず、gate だけが変わる。既存 tests がそのまま Linux で走る。
- BUG-56/107/108 の回帰が、windows-build CI を待たずに手元で検証できるようになる。

先に要るもの。
- 追加の guard は不要。`focus/mod.rs` の宣言を足す。
- core へ移す場合のみ、core の `Cargo.toml` に `toml` を足す。

### F4: 呼び出し元ゼロの 3 段の転送を消す(E)

何をするか。
- `PlatformState::update_focus_info`、`FocusTracker::update`、`CurrentFocus::update` を消す。

なぜ安全か。
- `grep -rn "update_focus_info(" crates src tests` が定義 1 件、`grep -rnE "\.update\(" crates/awase-windows crates/awase-windows-core` が転送 1 件のみ。
- `update_with_process_name` 側が実際の経路である。

先に要るもの。
- なし。`cargo check --target x86_64-pc-windows-msvc -p awase-windows --tests --lib` で `pub` 参照が無いことを確かめれば足りる(このサンドボックスでは実行していない)。

## 調べたが候補にしなかったもの

- `ImeStateHub` の `pub(crate)` メソッド全てについて `grep -rnw` で呼び出し件数を数えた。呼び出しが定義 1 件のみ(死んだメソッド)は無かった。件数 2 のものは定義と shell.rs のラッパー、または定義と呼び出し 1 件だった。
- `FocusStore`・`GateStore` の全フィールドに読み手があった(`focus_debounce_ms` は runtime/mod.rs:1834、`ime_poll_interval_ms` は runtime/mod.rs:1252、`last_focus_transition_ms` は focus_tracking.rs:59,82,109 ほか)。
- `ObservedState` の診断用フィールド(`composition_active`・`ime_show_seq`・`ime_change_seq`)は `ime_controller.rs:118-187` のログで読まれていた。
- `INPUT_RELAY_APPS`(プロセスグローバル)と `FocusTracker::input_relay_apps()` の二重保持は、`CLAUDE.md` に意図した例外として書かれており、`architecture_guard.rs:5709` が固定している。候補にしない。
- `platform.rs` の `WindowsPlatform` → `FocusTracker` への 1 行転送(`learn_imm_capability`・`record_imm_null_probe` など)は、同じ呼び出し元が `platform.focus.xxx` を直接使う箇所と混在している(`focus_tracking.rs:102`)。ただし `platform.rs` は他の担当の範囲で、転送を消すと呼び出し元が増えるため、効果は未確認。
