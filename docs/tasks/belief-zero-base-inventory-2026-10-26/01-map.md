# 01 現在の仕組みの地図(IME 開閉と Engine 活性の belief)

調査対象: `origin/develop` `ff75d5d58`(`git rev-parse HEAD`)。読み取りのみ・cargo 未実行・実機ログ未読。行番号はこのコミットのもの。
「確認」は、その場で `git grep` やファイルを読んで確かめたもの。確かめていないものは「未確認」と書く。

## 1. 全体像: 「Engine ON」は保存された状態ではなく、毎回計算される関数

`Engine::compute_state(ctx)`(`src/engine/engine.rs:267`)は、次の順に見て最初に当たった理由で `Inactive` を返す。

| 順 | 条件(偽のとき) | `InactiveReason`(`src/engine/decision.rs:91`) | ctx のどこから来るか |
|---|---|---|---|
| 1 | `adapter.is_enabled()` | `UserDisabled` | 利用者の設定(Ctrl+Shift+F12・トレイ・単独打鍵の連打 等。`Engine::set_user_enabled` :806) |
| 2 | `ctx.is_japanese_ime` | `NotJapaneseIme` | `ImeBelief::is_japanese_ime()`(`runtime/mod.rs:507` `build_ctx`) |
| 3 | `ctx.ime_on` | `ImeOff` | `ImeStateHub::effective_open()`(下の §2) |
| 4 | `ctx.input_mode.is_romaji_capable()` | `NotRomajiInput` | `ImeStateHub::input_mode()`(§4) |

- `is_romaji_capable` は `ObservedRomaji | AssumedRomaji` だけが真(`src/engine/mode_state.rs:52`)。**`ObservedKana`・`ObservedEisu`・`Unknown` は偽**。つまり「IME ON かつ ひらがな」でも `input_mode` が `Unknown` や `ObservedKana` なら Engine は OFF。所有者の方針の「ひらがな」は、この `input_mode` 軸も含めて決める必要がある(論点 Q9)。
- `compute_active` は `compute_state` の bool 版(:334)。`on_input` は Phase 1(特殊キー照合。Engine ON/OFF コンボ・IME 制御コンボは**活性チェックより前**)→ Phase 2(活性チェックと遷移検知 `check_active_transition`)→ Phase 3(NicolaFsm。`bypass_reason`: `Passthrough`/`ImeControl`/`OsModifierHeld`、`nicola_fsm.rs:3079`)の順(:539〜:570)。
- 活性が変わる瞬間は `prev_activation` と比べて検知し、active→inactive で保留キーを flush する(`engine.rs` `check_active_transition`・`to_context_change`)。
- **`compute_state` の外にある、キーを Engine に渡さない入口**(今回は精査していない。未確認): `app_disabled`(`config.app_overrides.disable_apps`、BUG-78、`focus_tracking.rs::apply_app_disable_transition`)、lang-detect の保持(ADR-263/280、`runtime/lang_detect_shadow.rs`)、半角英数トグルの戻り待ちの保持(ADR-245、`kp_stage_half_width_return` `key_pipeline.rs:2449`)、`InputBarrier::FocusTransition`(settle)。

## 2. `effective_open` の優先順位(ctx.ime_on の元)

`ImeStateHub::effective_open_at`(`crates/awase-windows/src/state/platform_state.rs:1213`)→ `resolve_effective`(:2186)は、2 段で決まる。

**第 2 段(最上位): `IntentStore`(窓ごと・TTL つき)** `crates/awase-ime-core/src/state/intent_store.rs`。現在のフォーカス hwnd に有効なエントリがあれば、その値が**第 1 段の結果を丸ごと上書き**する。TTL は ON=10 秒・OFF=30 秒(`EXPLICIT_ON_INTENT_TTL_MS`/`EXPLICIT_OFF_INTENT_TTL_MS`、`crates/awase-windows-base/src/tuning.rs:363,397`)。窓切替では消えない(hwnd がキー)。

**第 1 段: `ImeModel::resolve_open_at`**(`crates/awase-ime-core/src/state/ime_model.rs:564`)。上から最初に当たったものが基底値で、最後に `force_guards.resolve`(:600、`PanicReset` ガードが立っていれば強制 true)を掛ける。

| 順 | 基底の根拠(`BaseDecision`) | 条件 | 寿命 |
|---|---|---|---|
| 1 | `ExplicitIntent` | `last_intent` が `Some`。値は `desired_open` | `FocusChanged`・`KeyEffectPredicted(open あり)`・`ModeKeyPassedThrough` で捨てる |
| 2 | `KeyEffectPrediction` | `key_effect.open` が `Some` | 170ms(`KEY_EFFECT_SETTLE_MS`)後の Medium 以上の観測で照合して消える / 次の `UserImeSetIntent` / `FocusChanged`。**観測が来ない窓では消えない** |
| 3 | `DeriveHigh` / `DeriveMedium` | `derive_any`(`observation_store.rs:905`): 新鮮(3 秒、`OBSERVATION_FRESH_WINDOW_MS`)な High が 1 つあれば採用、無ければ Medium 以上が**矛盾なく**そろえば採用。矛盾なら `None` | 3 秒 |
| 4 | `IndicatorGlyph`(字形、ADR-273) | `indicator_reading`(:644)。新鮮な Medium 以上が居ない・字形より新しい Medium 以上が居ないとき | 窓の範囲 |
| 5 | `MostRecentTrusted` | `most_recent_trusted`(:801): 期限切れでない観測のうち、(Medium 以上か, 時刻, 信頼度)の最大。**鮮度の窓は無い**(`expires_at` だけ) | 窓の範囲(`FocusChanged` で全消し) |
| 6 | `DesiredFallback` | 上のどれも無い。値は `desired_open`(初期値 true、`desired_is_placeholder`) | 永続 |

ほかに、belief を書かない診断用の層: `fallback_shadow`・`[effective-open-flip]` ログ・`effective_open_basis`(journal 用)。

## 3. 観測の証拠源(`declare_evidence!`、`evidence.rs:129`)

構築子が現存する(本番で書ける)のは 7 つだけ(`grep -n "impl Observed<" evidence.rs` → FocusProbe・ImmCrossProbe・ObserverPoll・HeuristicDefault・ConvOpenInference・MozcShm・TaskbarGlyph)。`ImmGetOpenStatus`・`Gji`・`Tsf`・`HwndCache` は宣言だけで本番の書き込み経路が無い(`docs/tasks/mech-ecrs-2026-10-21/raw-c-belief.md` MC-03 と同じ事実を、今回 `impl Observed<` の列挙で再確認)。

| 証拠源 | プール | 信頼度 | 誰がいつ書くか | 備考 |
|---|---|---|---|---|
| `ImmCrossProbe` | Actuating | High | フォーカス後の非同期プローブ(子 hwnd) | epoch+hwnd の同一性照合(ADR-106)。GJI の tsf-native/imm32-unavailable 窓では信頼できない(BUG-223(a)) |
| `ObserverPoll` | Actuating | Medium | 500ms 周期の poll(`apply_ime_update`) | **TsfNative では常に停止(:1142)、他でも明示意図があると停止**(読めない窓は :1194、読める窓は :1217。`runtime/mod.rs:1126 reschedule_ime_refresh`)。停止中の再開は「フォーカス変更・may_change_ime キー・通過マーク・外部変化の監視窓」だけ。ADR-279 段階 0 が測定 |
| `FocusProbe` | Belief | Low | フォーカス直後の同期プローブ | `hwndFocus` は真のトップレベルとは限らない(BUG-91) |
| `HeuristicDefault` | Belief | Low | 観測ゼロの安全デフォルト。ON: `reset_stale_ime_on_for_imm_broken`(`platform_state.rs:1804`)、OFF: `assume_closed_for_new_thread`(:1863) | Medium 以上に負ける |
| `ConvOpenInference` | Belief | Medium | conv ビットからの推測 | drift の根拠から除外(BUG-173 追補3) |
| `MozcShm` | Belief | High | mozc-readable の共有メモリ(ADR-268) | TTL 1000ms(`MOZC_SHM_OBSERVATION_TTL_MS`)。BUG-219 で TTL 切れ直後の IMM の偽 OFF が勝った |
| `TaskbarGlyph` | Belief | Medium | タスクバーの入力インジケーターの字形(ADR-273) | `iter()` に含めない独立枠。設定が `observe` なら belief に入らない(`ui_max_level`) |

## 4. 書き込み点の表(入力 → 出力)

`desired_open` は `reduce` の 4 つのアームからだけ書かれる(`ime_model.rs:905,914,927,1106`、確認: `grep -n "desired_open = " ime_model.rs`)。`input_mode` は `reduce` の 3 アーム(`:1004` 観測、`:1017` 適用、`:1070` 予測)と `new()` だけ(確認: `grep -n "self.input_mode = " ime_model.rs`)。

### 4.1 イベント × 状態

「—」は書かない。`hub` は `ImeStateHub`(`platform_state.rs`)側の状態(`reduce` の外)。

| イベント(送り元) | `desired_open` | `last_intent` | `key_effect.open` | 観測プール | `input_mode` | `applied` | hub: `IntentStore` | hub: `persistent_explicit_off` |
|---|---|---|---|---|---|---|---|---|
| **`UserImeSetIntent(false)`**(物理 0x1A・0xF3/F4/0x19 のトグル解決・F13〜F24 役割 Toggle・`keys.ime_off`・Engine の `SetOpen(false)`)`ime_model.rs:902` | false | Some(false) | 捨てる | — | — | — | 記録 OFF・30 秒(物理/同期/Engine の SetOpen。`write_*`・`on_engine_set_open_request`) | (tick, 前面 pid)を記録(`dispatch_event` :212、:227) |
| **`UserImeSetIntent(true)`**(物理 0x16・トグル解決・`keys.ime_on`・Engine の `SetOpen(true)`) | true | Some(true) | 捨てる | — | — | — | 記録 ON・10 秒 | **消す**(ms=0, pid=0) |
| **`KeyEffectPredicted(open=Some(v))`**(ひらがな 0xF2 等、表引きの予測。`apply_key_effect_prediction` :329、呼び出し `key_pipeline.rs:1982`)`ime_model.rs:1057` | **書かない** | 捨てる(None) | v(at=tick) | — | mode があれば書く | 食い違えば Unknown へ降格 | **そのエントリを消す**(:361) | **触れない** |
| **`KeyEffectPredicted(open=None)`**(追跡だけ) | — | — | 変えない | — | mode があれば書く | — | — | — |
| **`ModeKeyPassedThrough{align_desired}`**(無変換/変換の生キー通過後、通過マークの窓 300ms 以内の観測。`ime_model.rs:1090`) | `align_desired` かつ `derive_any` が値を返すときだけ、その値へ | 捨てる | — | — | — | 観測と食い違えば降格(`demote_applied`) | 窓の間は破棄(`invalidate_intents_if_mode_key_pass_live`) | 触れない |
| **`ObserverReported(obs)`**(§3 の証拠源。`ime_model.rs:930`) | **書かない** | — | Medium 以上かつ 170ms 経過後なら照合して消す(食い違いは `[key-effect-miss]`) | 記録。drift を更新(字形以外) | — | — | — | — |
| **`FocusChanged`**(`reduce_focus_changed` :1157) | **書かない(持ち越す)** | 捨てる | 捨てる(`key_effect=None`、`key_track` 既定) | **全消し**・drift 消去・fence 更新 | **書かない(持ち越す)** | Unknown | **触れない**(hwnd キーなので戻れば効く) | 触れない |
| **`HwndCacheRestored{target}`**(窓切替のキャッシュ復元。`apply_hwnd_cache_restore` :1737) | target | 設定しない | — | — | キャッシュの mode(Eisu は `cache_restore_eisu_guard` で AssumedRomaji へ) | — | キャッシュのほうが新しければ消す | — |
| **`PanicReset{true}`**(`apply_panic_reset` :1617) | true | 設定しない | — | 全消し | ObservedRomaji | Unknown | 消す | — |
| **`InputModeObserved(Medium以上)`**(`reduce` :995) | — | — | モード軸の予測を 170ms 後に照合 | — | 書く(fence 内の古い観測は捨てる) | — | — | — |
| **`InputModeApplied`**(awase 自身の補正 6 種: 半角英数トグル・ImmBroken・CacheRestore・UserTurnOnEisuReset・UserImeOnEisuReset・PostSetOpenEisuReset・PanicReset) | — | — | — | — | 書く | — | — | — |
| `ImeApplyRequested/Succeeded/Failed`・`DriftDetected`・`AppliedDemoted` | — | — | — | — | — | applied/pending だけ | — | — |

### 4.2 窓切替の後処理(`FocusChanged` の直後、`focus_tracking.rs:537 on_focus_process_changed`)

`FocusChanged`(:581 付近で dispatch)の後、窓の種類ごとに `desired_open` と観測を書き直す。

| 窓の種類 | 処理 | 書くもの |
|---|---|---|
| TsfNative(`is_effectively_tsf`、:613) | `plan_tsf_cache_restore`(`crates/awase-windows-plans/src/focus/tsf_cache_restore.rs:61`)。**ON のキャッシュしか復元しない**(`SkipCacheNotOn`)。条件: キャッシュ ON・`desired_open` が false・hwnd 一致(BUG-128)・同プロセスでキャッシュより新しい明示 OFF が無い(BUG-218)。復元しないとき**`desired_open` は前窓の値のまま**(コメント「SSOT: desired_open を前窓の値のまま維持」) | `HwndCacheRestored(true)` または何も書かない |
| TsfNative 以外(:690〜) | `should_discard_imm_broken_cache` で捨てなければ `apply_hwnd_cache_restore`(ON/OFF とも)。キャッシュ無しで: 10 秒以内の明示 OFF があれば何もしない / awase 自身の窓なら何もしない / 測定済み条件の新スレッドなら `assume_closed_for_new_thread`(HeuristicDefault **OFF**・Low)/ それ以外は `reset_stale_ime_on_for_imm_broken`(HeuristicDefault **ON**・Low。日本語配列・`effective_open` が偽・`last_intent` 無し・IntentStore 無しのときだけ) | `HwndCacheRestored(cache.ime_on)` または `ObserverReported(HeuristicDefault)` |

## 5. ON 側と OFF 側の非対称の一覧(確認したものだけ)

| # | 非対称 | 出典(確認) |
|---|---|---|
| A1 | **経路の非対称**: 方向ではなくキーの種類で経路が分かれる。静的な開閉キー(0x16/0x1A)・同期キー・`shadow_action` を持つキー(0x19・0xF3/F4・F13〜F24 の役割 Toggle)は `UserImeSetIntent`(`desired_open`・`last_intent`・IntentStore・`persistent_explicit_off` を書く)。表引きのキー(ひらがな 0xF2・カタカナ 0xF1・英数 0xF0・無変換/変換 等)は `KeyEffectPredicted`(`key_effect` だけ。`desired_open` は不変)。**GJI で「OFF は 0x1A/0xF3 の意図、ON は F2 の予測」になるので、結果として OFF だけが意図を残す** | `explicit_press.rs:103 select_shadow_intent`、`ime_model.rs:902,1057`、BUG-223 |
| A2 | `persistent_explicit_off` は**OFF の意図で立ち、ON の意図で消える**。予測の ON(F2)は消さない。窓切替後の復元の見送り(BUG-218)や 10 秒の `reset_stale` 抑制がこれを読む | `platform_state.rs:212-230`、`focus_tracking.rs:732` |
| A3 | `IntentStore` の TTL は ON=10 秒、OFF=30 秒 | `tuning.rs:363,397` |
| A4 | TsfNative の窓切替復元は **ON のキャッシュだけ**(OFF は復元しない) | `tsf_cache_restore.rs:71` |
| A5 | `HeuristicDefault` は ON 側(Imm32Unavailable のキャッシュ無し)と OFF 側(新スレッドの測定済み条件)で、書く条件がまったく違う | `platform_state.rs:1804,1863` |
| A6 | `ForceGuard::PanicReset` は **ON への強制だけ**(`resolve`: desired が false なら true にする) | `force_guard.rs:103` |
| A7 | drift correction は明示意図が `desired_open` と一致して観測が食い違うときだけ働く。実機の過去ログでは書き込み 169 件が全て「OFF にする」方向で、開ける方向は 0 件(コメントの記録) | `drift_correction.rs:127,138` |
| A8 | IME-ON の経路には `ObservedEisu` の救済(`eisu_reset_on_ime_on`)を対で配線する規則。OFF 側に対応物は無い | `.claude/rules/ime-belief-architecture.md`、`key_pipeline.rs:1239`(shadow toggle)・`:1614`(Engine の SetOpen 後) |
| A9 | `FocusChanged` が捨てるのは意図・予測・観測・applied・ガード。**`desired_open` と `input_mode` は持ち越す**。`desired_open` が false で持ち越されると、観測が無い窓では `DesiredFallback` の false が `effective_open` になる | `ime_model.rs:1157-1209`、BUG-223 |
| A10 | `desired_open` の初期値は true(`desired_is_placeholder`)で、最初の成功観測に 1 回だけ揃える(BUG-163)。OFF の初期値は無い | `ime_model.rs:375`、`platform_state.rs:823` |

## 6. 「明示意図」の保管場所が 5 つある

| 場所 | 単位 | 寿命 | 読み手 |
|---|---|---|---|
| `ImeModel::desired_open` | グローバル | 永続(窓切替でも残る) | `DesiredFallback`・drift correction・`ExplicitIntent` の値 |
| `ImeModel::last_intent` | グローバル | `FocusChanged`・予測・通過で消える | `resolve_open_at` の最上位・drift correction |
| `ImeStateHub::intent_store` | hwnd ごと | ON 10 秒 / OFF 30 秒・窓切替で残る | `effective_open`(最上位の上書き)・授権の根拠・`reset_stale` の抑制 |
| `ImeStateHub::last_user_explicit_off_{ms,pid}` | グローバル | ON の意図で消える | 窓切替の復元の判断(BUG-218)・10 秒の抑制 |
| `ImeModel::key_effect.open` | グローバル | 観測の照合・意図・窓切替で消える | `resolve_open_at` の 2 番目 |

(`raw-c-belief.md` MC-07 が「`last_intent` と `intent_store` の二重」を指摘している。今回は後ろの 2 つを加えて 5 つ。)

## 7. 半角英数トグル・lang-detect・drift correction との関係

- **半角英数トグル**(左 Shift 単独タップ、`crates/awase-hook-core/src/state/half_width_alnum.rs`): 入ると `input_mode` が `ObservedEisu` 相当になり Engine は `NotRomajiInput` で OFF。出るときは `AssumedRomaji{UserHalfWidthAlnumToggleOff}` へ(`ime_refresh.rs:628`)。窓切替では離脱時に IME へ送らず戻り待ちに積み、戻った窓の最初の打鍵で復元する(ADR-245)。`toggle_held` が偽でも戻り待ちは Exit の権利を持つ(`exit_owns_write`)。
- **lang-detect**(ADR-263/280、`src/lang_detect.rs`・`runtime/lang_detect_shadow.rs`): 観測の無い窓(実測学習で降格された窓)で belief が ON のとき、窓切替直後・長い無操作後の打鍵を保持し、日本語と判定したらひらがなキー(`VK_DBE_HIRAGANA`)を 1 回送って保持した打鍵を流す。英数と判定したら半角英数トグルへ入る(`eisu_threshold`)。**belief を書き換えるのではなく、IME に書いて belief を追随させる**。判定の誤りが BUG-220・BUG-222・ADR-280 の事故につながった。
- **drift correction**(ADR-080・179・212 P6・279): 明示意図があり観測と食い違うときだけ再送する(`evaluate_drift`、`drift_correction.rs:127`)。意図が無い食い違いは補正しない。TsfNative・明示意図ありでは poll が止まるので観測自体が来ない(ADR-279 段階 0)。ADR-279 の「意図を取り下げて Engine を戻す導線」は未実装(`ExplicitIntentRefutedByTyping` の `git grep` 0 件、ledger の確認)。代わりに案 P の段階 P0(観測だけ)として、明示意図の間の IMM の読みを専用タイマーで取る `drift_follow_would_ensure_timer`(`runtime/mod.rs:1197,1219`)が入っている(belief は書かない)。
- **外から変わったことへの追随**(ADR-187・188/244・205・268): いずれも「窓の中の観測で、意図を捨て `ModeKeyPassedThrough{true,true}` で `desired_open` を揃える」尾部を持つ(`raw-c-belief.md` MC-08)。

## 8. 擬似窓・擬似 IME が写している範囲(検証できる形を考えるときの制約)

`crates/awase-windows/tests/support/harness.rs` 冒頭の記述(確認):
- **本物を呼ぶ**: `ImeStateHub`(`dispatch_event`・`effective_open_at`・`record_explicit_intent`・`apply_key_effect_prediction`・`align_placeholder_desired`)、`KeyEffectKeymap::predict`、`issue_open_warrant`、`check_drift_correction`、`Engine`(`RefreshState`/`FocusChanged`)。
- **写している**: `issue_actuation_order`/`write_*`、`kp_predict_key_effect`、`ir_apply_drift_correction` の「検知へ進むか」まで。
- **写していない**: 通過マーク(ADR-187)、`ImeApplyRequested`/`applied` の往復、ForceGuard、TSF warmup、Engine の `on_input`、**`on_focus_process_changed` の hwnd キャッシュ復元・新スレッド仮定・settle 期限**(`focus_to` は `FocusChanged` と同一プロセスの追随だけ。`focus_change_to_new_thread_unreadable` は新スレッドの OFF の仮定だけ)。

→ BUG-223 の持ち越し(窓切替で `desired_open` が残る)は、`FocusChanged` が `desired_open` を残すことまでは擬似窓で再現できるが、**キャッシュ復元の分岐(§4.2)は写されていないので、復元が効く/効かない場合の比較にはハーネスの拡張が要る**。
