# 機構 ECRS 調査 C: IME の状態(belief)の観測・予測・補正

調査対象のコミット: `f3c2007e`(origin/develop 先端、`git rev-parse --short HEAD`)。読み取り専用、cargo 未実行。
以下の「確認」は、すべて `/home/cuzic/rust-nicola-wt-mech` で実行した git grep などの出力に基づく。実行できなかったもの・読んでいないものは「未確認」と書く。

## 結論

- **E(撤去可)候補 5 件、S(小さな簡素化)1 件、C(統合)候補 2 件、? 2 件、K 3 件。**
- 大きな機構(観測源の重なり、drift correction、read strategy、eisu 系)は、読んだ範囲では「呼び出し元ゼロ」「恒久無効」は見つからなかった。**今回の E は「テストだけが使う型・関数・モジュール」が中心**で、実行時の挙動は変わらない。C は 2 件とも設計判断が要り、擬似 IME の拡張が先に要る。
- 上位 5 件(効果の大きい順):
  1. **MC-01** `key_effect_rules.rs`(340 行、本番の呼び出し元ゼロ。モジュール自身が「実行時の判断にはまだ配線していない」と明記)。E。
  2. **MC-02** `keymap_initial_hypothesis.rs`(247 行、ADR-195 段階 0 の「初期仮説表」。本番の呼び出し元ゼロ)。E。
  3. **MC-03** open 観測の枠のうち `gji`・`tsf`・`hwnd_cache`・`imm_get_open_status` の 4 つ(本番で書く経路が無い。`evidence.rs:294-298` のコメントも同じ事実を述べる)。E。ジャーナルの直列化に注意。
  4. **MC-07** `ImeModel::last_intent`(単一・フォーカスで消える)と `ImeStateHub::intent_store`(窓ごと・TTL つき)の二重の明示意図。「同じ状態を 2 か所で持つ」の実例。C だが ADR-087 INV-24(a) 級の判断。
  5. **MC-08** 「外から変わったことに追随する」3 経路(ADR-205・ADR-188/244・ADR-268)が、同じ 3 行の尾部(意図を捨てる→`pass_through_observed(true,true)`)を重複して持つ。小さな C。窓の機構そのものの統合(ADR-187/188/205)は **?**。
- 次点: MC-04(`ObservationStore::record`/`record_belief`/`consensus`、テストだけが呼ぶ)、MC-05(`ForceOnReason::ProfilePolicy` ほか)、MC-06(`ime_profile_driver.rs`、400 行、契約宣言のテスト用具)。

行数の見積りは `wc -l` とファイルの該当範囲の目視。テストの行数を含む場合は明記する。

---

## E 候補

### MC-01 `key_effect_rules.rs` の導出関数(キーごとの宣言的ルールの推定)

- 場所: `crates/awase-key-effect/src/state/key_effect_rules.rs`(全 340 行、`#[cfg(test)]` は 200 行目から、`#[test]` 8 本)。
- 解いている問題: ADR-191/195 で学習した表が「実際にどんなルールを表すか」を IME ごとの oracle にする(ADR-199 決定 6-2 の周辺)。
- **生きているか: 呼び出し元ゼロ。** モジュールの先頭コメントに「実行時の判断にはまだ配線していない(読み取り専用の導出)」とある。
  - `git grep -n "key_effect_rules\|infer_open_rule\|OpenRule\b" -- crates src tools` → 定義ファイル以外の出現は `awase-key-effect/src/state/mod.rs:15`(`pub mod`)、`awase-windows/tests/layer_boundary_guard.rs:573`(モジュール名の一覧)、`tools/core-split/assign.py:21`(crate 分割の割り当て表)の 3 件だけ。
  - `infer_key_rules`・`infer_all` は定義ファイル以外に 0 件(`git grep -w` を関数ごとに実行)。
  - 履歴: `git log --oneline --follow -S"infer_key_rules"` の最古は `09abc0b8`(`deduce_*` を `infer_*` に改名)。実装の導入コミットは `--follow` では追えず、未確認。
- 重複・統合先: 近い役割は `key_effect_table::classify_cells`(全セルを横断した開閉の分類)と `toggle_contradiction`(ADR-199 決定 6-2)。本モジュールは後者の一般化をうたうが、どちらにも配線されていない。
- 撤去した場合の挙動: 実行時の変化なし(呼ばれないため)。壊れるもの: `layer_boundary_guard.rs:573` のモジュール名の一覧、`tools/core-split/assign.py:21`。`mod.rs:15` の 1 行。
- 擬似 IME で検証: 不要(実行時の振る舞いが無い)。ただし ADR-195 の oracle 構想(学習表から期待する挙動を導いて擬似 IME と突き合わせる)が将来要るなら、**撤去ではなく配線**が正しい。その場合は「配線する予定があるか」を所有者に確認する(判断 ?寄りの E)。
- 判断: **E**(所有者が oracle 構想を捨てるなら)。再発ファミリー: 当たらない(`key_effect_*` は表の「IME belief」行の `key_effect_predictor/runtime/table` に近いが、本モジュールはその 3 つではない)。

### MC-02 `keymap_initial_hypothesis.rs`(ADR-195 段階 0 の初期仮説表 S)

- 場所: `crates/awase-key-effect/src/state/keymap_initial_hypothesis.rs`(全 247 行、`#[cfg(test)]` は 160 行目から)。
- 解いている問題: ADR-195 段階 0「設定の読み取りラウンド(既存 3 経路の統合)」。
- **生きているか: 呼び出し元ゼロ。**
  - `git grep -n "InitialHypothesis\|InitialHyp\|build_initial_hypothesis" -- crates src` → 定義ファイル以外 0 件(出力に出たのは `awase-key-effect/src/state/mod.rs:21` の `pub mod` のみ。`layer_boundary_guard.rs:577` はモジュール名)。
  - `build_gji_initial_hypothesis`・`build_msime_native_initial_hypothesis` は同ファイル内でもテスト以外から呼ばれない(`git grep -lw` が同ファイルだけを返す)。
  - 導入は `97b5052f`(ADR-195 段階 0)、`f6b74274` で修正。以後の使用の履歴は未確認。
- 重複: 内部の `needs_learning` 集合は、ADR-196 以降の学習プロセス(`awase-keymap-learn`)が自前の走査計画を持つので、そちらと重なる(重なりの程度は未確認、ADR-196 を読んでいない)。
- 撤去した場合の挙動: 実行時の変化なし。`mod.rs:21`、`layer_boundary_guard.rs:577` の更新。`awase_gji_config::keymap::extract_ime_keys` など呼び先の関数が他に使われているかは未確認(使われなくなれば連鎖で E)。
- 擬似 IME: 不要。
- 判断: **E**(ADR-195 段階 0 の成果物を残す意図があるかを所有者に確認)。ファミリー: 当たらない。

### MC-03 open 観測の枠 `gji`・`tsf`・`hwnd_cache`・`imm_get_open_status` と、その evidence 型

- 場所: `observation_store.rs:240-254`(`PerSourceObservations` のフィールド)、`:269-312`(`get`/`set`)、`:317-330`(`iter`)、`:349-351`(`clear`)、`evidence.rs:129-135` 付近の `declare_evidence!`(`Gji`・`Tsf`・`HwndCache`・`ImmGetOpenStatus`)、`ime_event.rs` の `ObservationSource` の対応する variant と `authority()`。
- 解いている問題: ADR-089 の型状態(観測の出自と authority の型付け)。ADR-098 の `ObservationSource` 時代の名残。
- **生きているか: 本番で書く経路が無い。**
  - `evidence.rs:294-298` のコメント自体が「`ImmGetOpenStatus` / `Gji` / `Tsf` / `HwndCache` には構築子を置かない。これらを open 観測として record する本番経路が現時点で存在しない」と書いている。
  - 構築子の一覧(`grep -n "impl Observed<" evidence.rs`): `FocusProbe`・`ImmCrossProbe`・`ObserverPoll`・`HeuristicDefault`・`ConvOpenInference`・`MozcShm`・`TaskbarGlyph` の 7 つだけ。
  - `git grep -n "ObservationSource::Gji\b" -- crates`(tests と evidence.rs を除く)→ `ime_event.rs`(authority のテスト)、`observation_store.rs`(`get`/`set`)、`platform_state.rs:4149`(`#[cfg(test)]` の journal テスト)のみ。`Tsf` も同じ。`HwndCache` は `ime_event.rs` の authority のテストだけ。
  - `ImmGetOpenStatus` は**タグとしては生きている**(`WarrantBasis::DirectRead(ObservationSource::ImmGetOpenStatus)`: `actuation_chain.rs:750`、`open_warrant.rs:480` ほか。`observation_source_guard`(dylint)が `InputModeObserved { source: ImmGetOpenStatus }` を偽装として禁止する根拠にもなる)。**撤去できるのは「観測の枠」と evidence 型だけで、列挙値は残る**。
- 重複: `HwndCache` の belief への効果は `ImeEvent::HwndCacheRestored`(別イベント)が担う。`Gji` は `observe_gji_after_focus` が `ObserverPoll` として記録している(`gji_observer.rs` の `observer_poll_value`)ので、`Gji` の枠は使われない。
- 撤去した場合の挙動: 実行時の変化なし(書かれない枠は常に `None`)。**注意**: `ImeEvent::ObserverReported` は journal に直列化される(ADR-082)。列挙値(`ObservationSource::Gji` など)を消すと、古い journal・不具合報告の再生でデシリアライズが失敗しうる。**列挙値は残し、枠・evidence 型だけを消す**のが安全。`tests/journals/` の固定資料には該当 source が無いことは確認した(`git grep` で 0 件)が、手元に保存された利用者の報告までは未確認。
- 擬似 IME で検証: 不要(書かれない)。journal の後方互換は `journal_replay.rs` で確認できる。
- 判断: **E**(枠と evidence 型。見積り -60 行。`ObservationSource` の列挙値と `authority()` は残す)。ファミリー: IME belief(`observation_store.rs`)。fix-requires-evidence 上は、新しい挙動を足さない純粋な削除なので (a)(b) 不要。

### MC-04 `ObservationStore::record` / `record_belief` / `consensus`(テストだけが呼ぶ)

- 場所: `observation_store.rs:456-472`(`record`・`record_belief`)、`:1014-`(`consensus`、約 30 行)。
- **生きているか: 本番の呼び出し元ゼロ。**
  - `git grep -nF "observations.record(" -- crates`(tests 除く)→ `belief.rs:11` のコメントのみ。
  - `git grep -nF ".record_belief(" -- crates | wc -l` → 2(どちらも `observation_store.rs` の `#[cfg(test)]` 内、1603・2240 行)。
  - `consensus` は `observation_store.rs` のテスト 8 か所以外に 0 件(`git grep -nw consensus -- crates` で確認。ほかの出現はコメント)。
  - 本番の入口は `record_replayed`(`ime_model.rs:870`、`ObserverReported` の reduce)だけ。
- 重複: 本番の `derive_any`/`most_recent_trusted` が合意判定を担い、`consensus` は未使用の別実装。
- 撤去した場合の挙動: 実行時の変化なし。ただし ADR-089/090 の「プール分離を型で強制する」仕組み(`record<E: OpenEvidence<Pool = ActuatingPool>>`)の**型付きの入口**を消すことになる。`tests/architecture_guard.rs:3158,3241` が「観測の書き込みは `record` / `record_belief` を経由する」という文言を持つ(コメントか走査かは未確認。`3241` はメッセージ文字列に見える)。`record_belief` を消す場合は、型付きの入口を**テストから使われる形で残す意味があるか**を ADR-089 の意思と照らす必要がある。
- 判断: `consensus` は **E**(-30 行とテスト)。`record`/`record_belief` は **?**(型付きの入口という設計意図を残すか、所有者判断)。ファミリー: IME belief。

### MC-05 force guard まわりの未使用部分(`ForceOnReason::ProfilePolicy`、`expires_at`、`purge_expired`、`ObserveMissMonitor` の一部)

- 場所: `crates/awase-ime-core/src/state/force_guard.rs`(全 419 行、`#[cfg(test)]` は 178 行目から)。
- 解いている問題: パニックリセット直後の stale poll による上書き防止(`PanicReset`)。ADR-087 §7。
- **生きているか(部分的に死んでいる)**:
  - `ForceOnReason::ProfilePolicy` の本番の生成元は 0。`git grep -n "ForceOnReason::ProfilePolicy" -- crates` の出現は `force_guard.rs`(テストとコメント)、`platform_state.rs:2803,2807,2808`(`#[cfg(test)]`)、ドキュメントコメントのみ。`ForceGuard {` を本番で組むのは `platform_state.rs:1582`(`apply_panic_reset`、`reason: PanicReset`、`expires_at: None`)の 1 か所。
  - `expires_at` は判定に使われない: `force_guard.rs:88-90` のコメントが「`effective_open()` は期限を見ない(`expires_at` は事実上機能していない)」と述べる。`purge_expired` は本番の呼び出し 0(`git grep -nw purge_expired` が `force_guard.rs` だけを返す)。
  - `ObserveMissMonitor` 自体は**生きている**(`record_miss` → `consecutive_miss_count` を `imm-learning` の `decide_imm_capability` が使う、`ime_refresh.rs:238,738`、`runtime/mod.rs:645`)。**死んでいるのは `first_miss_at`・`last_miss_at`・`exceeds()` だけ**(`exceeds` の呼び出しは `ime_model.rs:2993,2998` のテストのみ)。
  - 先頭の説明文の「閾値到達で ForceGuard を追加」は撤去済み(`621bf93c`)。
- 重複: 常に最大 1 件(`PanicReset`)しか入らない集合を `Vec` 風に扱っている。`open_warrant` の `SafetyValve(ForceOnReason)` もこの 1 種類だけを見ている。
- 撤去した場合の挙動: `ProfilePolicy` と `expires_at` の削除は実行時の変化なし。`ForceGuardSet` を「PanicReset の 1 状態」に畳むのは、`ime_model.rs:526`(`resolve`)・`open_warrant.rs`(Step 0)・`platform_state.rs:1557,1582,1645` を触る(S、ファミリー IME belief / actuation 合流点に隣接)。
- 擬似 IME で検証: PanicReset の guard は `tests/support/harness.rs:27` が「写していない: ForceGuard」と明記。**panic reset の発火と guard の解除を再現できない**ので、畳む場合は先に harness へ `apply_panic_reset` を足す(小規模)。`ProfilePolicy` と `expires_at` と未使用フィールドの削除だけなら、既存の `force_guard.rs` 内テストで足りる。
- 判断: **E**(`ProfilePolicy`・`expires_at`・`purge_expired`・`first_miss_at`/`last_miss_at`/`exceeds`、約 -40 行)。`ForceGuardSet` の畳み込みは **K**(挙動を守る安全弁。得は小さい)。

---

## C 候補

### MC-07 明示意図の二重保持: `ImeModel::last_intent` と `ImeStateHub::intent_store`

- 場所: `ime_model.rs:277` 付近(`last_intent`)、`intent_store.rs`(450 行)、`platform_state.rs`(`record_explicit_intent`:1936-1941、`effective_open_at`:1164、`:1783` の Imm32Unavailable 入口、`:345,449,532,644,1595,2127` の `intent_store.remove`)。
- 解いている問題: BUG-51 追補、ADR-087 §4 INV-24(a)(対象ごとの明示意図と TTL)。
- 事実(確認):
  - `last_intent` は単一・`FocusChanged` のたびに無条件で消える(`intent_store.rs:6-9` の説明)。読み手: `drift_correction.rs:133`、`explicit_intent()`(`platform_state.rs:303`)、`has_user_explicit_intent()`(`ime_model.rs:491,541`)、`mode_key_pass` の `has_last_intent`。
  - `intent_store` は窓(`HwndId`)ごと・TTL つき(`EXPLICIT_ON_INTENT_TTL_MS` 10 秒、`EXPLICIT_OFF_INTENT_TTL_MS` 30 秒、どちらも `#[measured(pending = true)]`=実測未了、`tuning.rs:401,435`)。読み手: `effective_open_at` の上書き(`platform_state.rs:1164`)、`open_warrant` Step 1(`open_warrant.rs:181`)、Imm32Unavailable 入口の温存(`platform_state.rs:1783`)。
  - 書き込みは同じ明示操作で両方に入る(`record_explicit_intent` が `intent_store.record` を 1 か所で呼ぶ。`last_intent` は `UserImeSetIntent` の reduce が書く)。**消す側も 6 か所で両方を一緒に消す**(`mode_key_pass` の `PassEffect`、外部変化の追随 3 経路、panic reset ほか。`pass_through_observed` が `last_intent` を捨て、直前に `intent_store.remove(hwnd)` を呼ぶ形)。
  - `intent_store.rs:6-14` と `state/mod.rs:104-110` のコメントは「runtime への配線はまだ無い(ADR-087 Phase 3)」と書くが、**実際は配線済み**(上記の `platform_state.rs` の呼び出し)。コメントが古い。
- 重複の意味: 「明示意図があるか」を `last_intent`(ImeModel 側、drift correction と `resolve_open_at` 用)と `intent_store`(ハブ側、`effective_open_at` と warrant 用)の 2 か所で別々に聞く。2 つの寿命(フォーカスで消える/TTL)が食い違う窓がある。実害の記録: BUG-51 追補 v3 と `intent_store.rs` の round3/4 の議論(HWND 再利用の混入、永続する本ストアで無期限だと drift correction が固着する)。
- 統合した場合に変わりうる挙動: フォーカス遷移直後の `desired_open`(旧窓の意図を新窓へ持ち越すか)、30 秒 TTL の OFF 意図が窓へ戻ったときの扱い、drift correction が見る「明示意図」の寿命。どれも ADR-087 §2.3/INV-24 の決定そのもの。
- 擬似 IME で検証: **部分的に可能**。`tests/intent_store_effective_open.rs` があり、ハーネスは `ImeStateHub`(本物、仮想時計 `HubClock::manual`)・`record_explicit_intent`・`effective_open_at`・`warrant_context`・`check_drift_correction` を呼ぶ(`harness.rs` 冒頭のコメント)。足りない: (1) 複数の窓・フォーカスの往復(harness は `FocusChanged` を流せるが、窓ごとの `HwndId` のモデルは薄い、未確認)、(2) 10/30 秒の TTL をまたぐ時計の進行(仮想時計はあるので小規模)、(3) 通過マーク(ADR-187)の `PassEffect`(harness は「写していない」と明記)。
- 判断: **C**(統合先: 窓ごとの 1 ストアに寄せるなら `intent_store` 側。ただし `last_intent` の「フォーカスで消える」を `IntentStore` の `FocusChanged` で表現する設計が要る)。**ただし ADR が要る案件**で、まず古いコメント(`mod.rs:104-110`、`intent_store.rs:6-14`)の訂正だけを先に行う。再発ファミリー: IME belief(`ime_model.rs`、`platform_state.rs`)、ADR-087/098 の領域。

### MC-08 「外から変わったことに追随する」経路の尾部の重複(小さな C)と、窓の機構の整理(?)

- 場所: `platform_state.rs` の `follow_external_change_in_scope`(512-537)、`follow_direct_read_in_scope`(584-、尾部は 641-648)、`report_mozc_shm`(2092-2135、尾部は 2125-2131)。
- 事実(読んで確認): 3 経路とも、観測を記録(`write_observer_poll` または `ObserverReported`)した後、**同じ 3 行**を持つ: `if let Some(hwnd) = current_focus() { intent_store.remove(hwnd); }` → `pass_through_observed(tick_ms, true, true)` →(2 経路で)`last_external_change_ms = now_ms`(532-536、644-646、2127-2131)。`mode_key_pass` の `PassEffect` を適用する 449-454 行も `intent_store.remove` + `pass_through_observed` を持つ。
- さらに窓の側も重なる:
  - ADR-187 の通過マーク(`mode_key_pass_mark`)、ADR-188/244 の直接観測の窓(`arm_direct_external_change_watch`)、ADR-205 の注入キーの窓(`arm_external_change_watch`、基準値あり)の 3 つは、**同じ物理キー**で同時に武装される(`key_pipeline.rs:1748-1802` の `kp_arm_external_change_watch` と `kp_stage_mode_key_follow`、FSM 再送出側 `executor.rs:639-646`)。どちらも `schedule_ime_refresh(20)`/`timer.set(TIMER_IME_REFRESH, 20ms)` で読み直しを予約し、`ir_stage_observe` が読む。
  - `ir_follow_after_literal_giveup`(ADR-227)は ADR-205 の窓を開く入口の 1 つ(`ime_refresh.rs:384`)。
  - 違い: 通過マークは `readable`(IMM が読める窓。OsPoll)、直接観測は読めない窓(`Imm32Unavailable` で GJI/同定済み MS-IME 本体)で基準値なし、注入キーの窓は基準値あり(「Chrome は常に 0」の環境で偽の OFF を採らないため、`external_change_watch.rs` 冒頭)。
- 撤去/統合した場合の挙動:
  - 尾部の 3 行を 1 つの関数(例 `follow_open_externally(tick_ms)`)に畳むのは挙動を変えない(-15 行程度)。`tests/architecture_guard.rs` の `pass_through_observed` の呼び出し箇所数の走査が増減の影響を受けるか未確認(`git grep -n "pass_through_observed" crates/awase-windows/tests/architecture_guard.rs` を実行していない)。
  - 3 つの窓の機構の統合(1 つの「打鍵後の観測の窓」+種別)は、BUG-149/150/157/158/172/186 の再発ファミリー。本稿では**設計案を出せるほど読めていない**(未確認)。
- 擬似 IME で検証: ADR-205 は harness が本物を呼ぶ(`arm/follow_external_change_in_scope`、`Setup::with_external_close_watch`。シナリオ `q4_external_close_*`、`closed_loop_scenarios.rs:569-606`)。**ADR-187 の通過マークと ADR-188/244 の直接観測は harness が「写していない」**(冒頭のコメント)。統合の等価性を見るには、(1) 通過マーク(`ModeKeyPassLatch`→`ModeKeyPassedThrough`)、(2) 直接観測の窓(`follow_direct_read` の `kind`・`HoldAdmission`・press axes)、(3) 読めない窓(`Imm32Unavailable`)の prefetch snapshot と、IMM 読みが通る窓の OsPoll の区別、(4) 読み戻し遅延 Q3(21〜62ms、実装済み)と打鍵中ガード(`TypingActive`)を harness に足す必要がある。規模は中(数百行のテスト用具)。
- 判断: 尾部の重複 **C(小)**。窓の機構の統合は **?**(読みが足りない。判断材料: 3 機構の armed 条件と読み取りの分岐を表にして、擬似 IME に (1)(2)(3) を足し、現行と同じ journal 列になるかを見る)。再発ファミリー: 物理キー押下ラッチ、IME belief。

---

## ? 候補

### MC-06 `ime_profile_driver.rs`(`ImeProfileDriver` trait と 3 つのドライバ)

- 場所: `crates/awase-ime-core/src/state/ime_profile_driver.rs`(全 400 行、`#[cfg(test)]` は 214 行目から)。
- 確認: `git grep -nw "ImeProfileDriver" -- crates` → 定義ファイル以外はコメント 3 件(`app_ime_policy.rs:193`、`gji_direct_mechanism.rs:6,261`)と `ime_key_sequence_golden.rs:218` のコメントのみ。`has_ime_on_path`・`stale_eisu_recovery_paired` は定義ファイル以外に 0 件。`driver_for`・`ALL_DRIVERS` もこのファイル内のテストだけが使う。`owns_physical_kanji` は `app_ime_policy.rs` に同名の別のフィールドがあり、trait のメソッドは使われない(`git grep -nw owns_physical_kanji` の出現は `app_ime_policy.rs` のフィールド)。
- 経緯: ADR-081 Phase 1 の計画の残骸で、ADR-090 §2.F が「契約の宣言」として凍結(冒頭の説明)。`state/eisu_recovery.rs` の対応表と `architecture_guard.rs:801` の `user_ime_on_paths_are_paired_with_eisu_reset` が、実質同じ契約(IME-ON 経路と stale ObservedEisu 救済の対配線)を別の形で守っている。
- 判断: **?**。本番の呼び出しゼロなので機能上は E だが、ADR-090 が意図して残した「契約の宣言」で、撤去は ADR の意思に触れる。判断材料: ADR-081 の不変条件 1(`stale_eisu_recovery_paired`)が `architecture_guard.rs:801` で完全に代替されているか。代替されているなら E(-400 行、うちテスト約 190 行)。ファミリー: IME belief(eisu 救済)。

---

## K(残す)

### MC-09 ADR-258 段階 0 の `ir_observe_first_literal`(観測だけ、journal のみ)

- 場所: `runtime/ime_refresh.rs:416-494`。
- 事実: belief・監視窓・refresh のパイプラインに触れず、literal 直後の読みを 1 回取って `FirstLiteralObserve` を journal に書くだけ(関数のドキュメントに明記)。
- 判断: **K**。BUG-200 の実利用データ待ち(メモリ記録: v2.1.0 に同梱済み、2026-10-18 時点で未修正・データ待ち)。**収集の期限が決まっていない**点は、所有者への確認事項(いつまでにデータが無ければ撤去するか)。「観測が来ない間は撤去を検討する」という期限を ADR-258 に 1 行足すと、観測専用機構が居座るのを防げる。ファミリー: warmup / IME belief。

### MC-10 同梱表(`key_effect_table.rs`、`ATOK`/`MSIME`/`MSIME_NATIVE`、1019 行)と、学習済み表(`keymap-learn-table.json`)の二重

- 事実: 学習表は `RuntimeTableCache` が採否を決め、不採用・破損時は同梱表へ縮退する(`key_effect_runtime.rs` 冒頭、ADR-195 段階 4、ADR-196 決定 1e)。同梱表は `tools/e2e/ime_key_matrix/gen_key_effect_table.py` の生成物。
- 判断: **K**。初回起動(学習前)と学習不採用時の既定値であり、撤去すると新規インストールの予測が空になる。重複ではなく、フォールバックの階層。同梱表の更新は ADR-196 の再測定の仕事。

### MC-11 eisu(半角英数)の候補・救済・復元の複数経路(`eisu_candidate`・`eisu_recovery`・`GjiIoInference`・`ConvBitsInference`・`half_width_alnum`・ADR-245 R1)

- **十分には調べていない**(下の「未確認」参照)。`eisu_recovery.rs` の冒頭の対応表(IME-ON 経路 × 救済の対応、7 行)と `eisu_candidate.rs`(ADR-238 の 1 回目の英数を候補にして確認する機構)は読んだ。`GjiIoInference` と `ConvBitsInference` は入力モード専用のソースで、open 観測の枠に入らない(`observation_store.rs:281,311,492`)ことを確認した。いずれも実行時に生きている(`ime_refresh.rs:227`、`platform_state.rs:659`)。
- 判断: **K**(重複の疑いは見つけていない。ただし「同じ矛盾を 2 経路で検出する」を厳密に調べるには、各 `ObservedEisu` を作る経路と解く経路を全部並べる必要があり、今回はそこまで行っていない)。

---

## 文書の食い違い(撤去ではなく訂正)

| 場所 | 内容 |
| --- | --- |
| `state/mod.rs:104-110` | `intent_store`・`open_warrant` を「未配線」「試験実装」と書くが、実際は配線済み(MC-07 参照) |
| `state/intent_store.rs:6-14` | 同上(「runtime への配線は ADR Phase 3 のスコープ」) |
| `state/belief.rs:8` | 3 層モデルの図に `Gji` / `Tsf` / `HwndCache` を観測源として載せる(MC-03: 本番で書かれない) |
| `state/force_guard.rs:140-146` の `ObserveMissMonitor` の説明 | 「閾値到達で…追加していたが撤去済み」の後に、実際は `imm-learning` の入力として生きていることが書かれていない |

---

## 擬似 IME に足すと検証できる範囲が広がるもの(優先順、規模の見積り)

1. **通過マーク(ADR-187)と直接観測の窓(ADR-188/244)の写し**。harness の冒頭が「写していない」と明記。これが無いと MC-07(意図の統合)も MC-08(窓の統合)も等価性を見られない。規模: 中(数百行)。`ModeKeyPassLatch`→`ModeKeyPassedThrough` の発火、`follow_direct_read` の `kind`・`HoldAdmission`、IMM が読める窓(OsPoll)と読めない窓(`Imm32Unavailable`、prefetch snapshot)の区別。
2. **フォーカス・窓のモデル**(複数の `HwndId`、窓の往復、`FocusChanged` と `intent_store`・`last_intent` の寿命)。MC-07 に必須。規模: 小〜中。`HubClock::manual` で TTL(10/30 秒)をまたぐ時計の進行は既にある。
3. **`apply_panic_reset` と ForceGuard の写し**。MC-05 の `ForceGuardSet` の畳み込みに必要。規模: 小(`apply_panic_reset` は 20 行程度の判断)。
4. **journal の再生による後方互換テスト**: MC-03 で `ObservationSource` の枠を消すとき、古い journal の `ObserverReported { source: Gji ... }` を食わせてデシリアライズできることの確認。`journal_replay.rs` と `tests/journals/` に既存の基盤がある。規模: 小。
5. **`ImeApplyRequested`/`applied` の往復**(harness が写していない)。MC-07 の `applied` との関係や drift correction の再送の確認に要るが、本調査の候補の主因ではない。規模: 中。

---

## 未確認・調査の限界

- 「テストだけが使う」の判定は、(a) `git grep` の単語一致、(b) 本番コード(`#[cfg(test)] mod` より前)の語の出現数を数える自作スクリプト、の 2 本で行った。スクリプトの誤検出(マクロ内・`cfg` の付いた別ファイルでの使用)を避けるため、E 候補はすべて `git grep -w` で個別に確かめた。それでも `awase-windows` の `#[cfg(windows)]` 配下の呼び出しは、目で確かめていないものがある。
- 読んでいない: `half_width_alnum.rs`(1817 行)と `half_width_resume_hold.rs` の中身(モジュール先頭のみ)、`external_change_watch.rs` の本体(窓の状態機械は先頭と呼び出し側のみ)、`explicit_press.rs`・`actuation_chain.rs`・`open_warrant.rs` の本体(担当 A の領域との境界)、`indicator_glyph*.rs`・`indicator_menu.rs`・`mozc_shm.rs`(ADR-261/271/273/268 は所有者が 4 段階の設定で選ぶ方針のため、機構としては K と見なして撤去候補にしていない)、ADR-098・ADR-212・ADR-262 の本文(frontmatter と一部のみ)、ADR-188・ADR-244・ADR-205 の決定の細部。
- 「同じ矛盾を 2 経路で検出する」の探索は、follow 系(MC-08)と明示意図(MC-07)に限った。drift correction(`drift_correction.rs`、`drift_plan.rs`、`ime_refresh.rs::ir_apply_drift_correction`)は、判定本体と実行計画が分かれており(`drift_plan.rs` 冒頭に「二重に判断しない」と明記)、重複は見つけていない。ただし `observation_store` の `drift`(`ImeDrift`、`drift_duration`)と `external_change_watch` の基準値、`mode_key_pass` の `align_*` の相互関係は、通読していない。
- 擬似 IME の能力の把握は、`harness.rs`・`pseudo_ime.rs` の冒頭と `closed_loop_scenarios.rs` の関数名一覧に基づく。各シナリオの中身は読んでいない。
- 実機でしか決まらないもの: 該当なし(今回の E 候補はどれも実行時の挙動を変えない)。MC-07・MC-08 は実機(Chrome × GJI、MS-IME 本体)の CI(`sc-bug149-chrome-*-passthru`、`cal-driftrec-chrome-real-*`、`sc-table-msime-*`)で退行を見る必要があるが、先に擬似 IME の拡張(上の 1・2)をしないと、CI の合否だけで等価性を判断することになる。
