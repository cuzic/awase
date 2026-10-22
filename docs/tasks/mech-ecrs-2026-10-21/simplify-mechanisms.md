---
title: 機構レベルの残りの撤去・統合候補(挙動を変えてよい前提での再評価)
status: 調査のみ(2026-10-22、origin/develop 6a1e2d1e)。cargo 未実行・読み取りのみ。行数は wc -l / grep で数えた値と「推定」を分けて書く。着手前に根拠を再検証すること
created: 2026-10-22
related_adr: ["ADR-030", "ADR-090", "ADR-103", "ADR-195", "ADR-273", "ADR-276", "ADR-277"]
---

# 機構レベルの残りの撤去・統合候補

所有者の問い(2026-10-22): 「もう撤去できるコードはないか。挙動が変わっても、シンプルになるならそちらを選ぶかどうか」。
`INDEX.md`・`INDEX-dd.md` の各 ID の現状を最新の develop で確かめ、未実施のものを**挙動を変えてもよい前提**で再評価した。
ADR-276(Unicode 注入の撤去)の後に残った名前・コメント・分岐も探した。

確かめたコマンド(すべて origin/develop `6a1e2d1e` で実行):
`git log origin/develop --since=2026-10-20 --no-merges`、`git log --merges`、各候補の `git grep -n`、`wc -l`、`#[cfg(test)]` の行番号での本番/テストの分割。

## 1. 実施状況(ID ごと)

### 実施済み

| ID | コミット / PR |
|---|---|
| MA-01 | `58ece6f1`(#756)。apply_mechanism の腕は Failed を返す形で残し、variant と `decide_attempt` の腕は golden のため残置 |
| MA-02・MA-08・MD-02、MD-04 の `disarm` | `783da7c6`(#754)。`armed_at_ms` は残置 |
| MB-01 | `02d8b828`(#757) |
| MB-04 | `b8c4a468`(#755) |
| MB-14(eager Unicode・`PendingGjiConfirm`) | ADR-276 P4 `23e075a6`(#768) |
| MB-15(`UnicodeLiteralObserverFsm`) | ADR-276 P1 `5d427f5a`(#767)。`git grep UnicodeLiteralObserver` 0 件 |
| DD2-01=DD4-02・DD3(`should_pass_physical_key`) | `6c52b10b`(#761) |
| DD3-C | `ca5a0ced`・`9de6db0b`(#760) |
| DD2-03・DD4-01・DD4-03 | `1038e67c`(#762) |
| DD4-04 | `6741e28e`(#763) |
| DD5-01 | `84ada30b`(#768、ADR-276 P3) |
| DD2-04 | `07c26c71`(#770) |
| DD1-01 | `8c0fd261`(#773) |
| DD3-01 | `8f97972c`(#779)。窓モデルの回帰は `f84588c8`(#790) |
| DD2-05 | `a327e9ef`(#783)。**`INDEX-dd.md` の実施状況表は「見送り」のまま古い**(全列挙 `068fa1a7` の後に実施された) |
| DD5-03 | `556732bd`(#789、BUG-213) |

### 未実施(理由)

| ID | 理由 |
|---|---|
| DD1-02 | e2e で到達 0 件(`probe-reach-result.md`)。撤去すると `explicit_press_exhaustive.rs` の `p5_ledger` の性質(P2 無しでも P1 が守る)を捨てる。実利用の journal 待ち |
| DD2-02 | CI で IdleCheck 1373 件・不一致 0(`dd202-ci-result.md`)。撤去の判断は所有者待ち。窓切替を挟む構成は未実施 |
| DD5-02 | 撤去版で単発の FAIL 1 件(再現せず)。効果約 8 行に対しガード 2 本の書き換え |
| MA-03・MA-05・MB-02・MB-03・MB-05・MB-06・MB-07・MB-08・MB-09・MB-10・MB-17・MC-E1・MC-E2・MC-C1・MC-C2・MD-01・MD-03・MD-05・MD-06・MD-07〜10・`ime_profile_driver.rs` | 下の表(2 節)と 4 節 |

## 2. 再評価(削れる行数が大きく、挙動の変化が小さい順)

行数の「実数」は `wc -l` と `#[cfg(test)]` の位置で数えた。複数ファイルにまたがる分は「推定」。

| 順 | ID | 対象 | 本番 | テスト | 挙動の変化 | 依存するガード・ADR | 確認の手段 | 推奨 |
|---|---|---|---|---|---|---|---|---|
| 1 | MC-E1 | `awase-key-effect/src/state/key_effect_rules.rs`・`keymap_initial_hypothesis.rs` | 250(90+160、実数) | 337(実数) | なし(本番の呼び出し元 0。`mod` 宣言・`layer_boundary_guard` の一覧・mutants 設定のみ) | ADR-195 段階 0・ADR-199(oracle として追加) | CI で足りる | **所有者判断**(oracle として残す意図が doc にある。不要なら撤去) |
| 2 | — | `awase-ime-core/src/state/ime_profile_driver.rs` | 213(実数) | 186(実数) | なし(呼び出し元 0。`mod` と `layer_boundary_guard` のみ) | ADR-090 が意図して凍結 | CI で足りる | **所有者判断** |
| 3 | MB-02+MB-03 | warm パスの `LiteralDetectFsm`(`literal_detect_fsm.rs` のうち per-VK が使う `per_vk_recovery_params`・`emit_recovery_actions` 以外)、`plan_literal_detect`、`TsfGate` の `Probing/Ready`、`GJI_CANDIDATE_VETO_CAP_MS` | 推定 約 430(literal_detect_fsm 本番 532 行中 約 380、plans 約 40、tsf_gate 約 40) | 推定 約 420(literal_detect_fsm のテスト 388 行+guard+plans) | Install 条件 `gate_probing ∧ !is_tsf_mode` は、Probing が ForceTsf(=Tsf モード)でだけ入るので矛盾。**ADR-276 で Unicode モードが消え、非 Tsf の送信元は raw recovery の再送(`send_romaji_dispatching_on_gate`)とフォーカス直後の hint 再計算の競合だけ**に縮んだ。その窓で語単位の事後確認が無くなる | warmup 再発ファミリー(fix-requires-evidence (a)/(b))、`architecture_guard.rs:9721-9748`(BUG-210 の baseline 順序)、tuning 定数 1 減 | windows-build+e2e。到達の証拠が欲しければ R2 の不具合報告で warm 由来の `[literal-detect]` を数える(e2e ログ 94 件では 0、raw-b) | **撤去**(実害の報告が 0 なら) |
| 4 | MD-05 | インジケーターのメニュー読み取り(`state/indicator_menu.rs` 851・`observer/indicator_menu.rs` 247・`runtime/indicator_menu_shadow.rs` 178) | 約 870(851 中本番 448+247+178、実数)+ config 15 行・settings 4 行 | 約 400(indicator_menu.rs のテスト)+ architecture_guard 22 行 | 既定 off の観測機能が消える。設定キー(`src/config.rs`)は読み捨てか警告にする必要 | ADR-271・ADR-273(belief に使わないと決定済み) | CI で足りる(既定 off) | **所有者判断**(使う予定が無ければ撤去が最大の削減) |
| 5 | MB-09 | `GjiFsm` の pending 影コピー(`PendingInput`・`GjiAction::SendInput/SendInputDirect/DiscardPending`・`PendingDiscardReason`) | 推定 約 100(gji_fsm.rs 本番 1032 行中 該当語を含む行 99) | 推定 約 100(同テスト側 99 行) | `[gji-fsm] DiscardPending` のログと pending 件数の WARN が消える。`platform.rs:541-547` の受け側は既に捨てているので送信は不変 | ADR-103 決定 5-a(破棄を明示的に記録する)の修正が要る | Linux の gji_fsm テストで足りる | **撤去**(ADR-103 追記と同時に) |
| 6 | MB-07 | `CompositionOutput` trait・`PlatformColdReason`・`composition_output()` と Windows の impl | 推定 約 80(`src/platform.rs:4-46`・`:183-185`、`output/mod.rs:1240-1270`、`platform.rs:1063-1066`) | 0 | なし。`git grep 'composition_output()'` は定義のみ、呼び出し 0。impl の `send_romaji` は `OutputSession::sender()` と同じ分岐の写し | ADR-030(他 OS 向けの拡張点) | CI で足りる | **撤去**(macOS/Linux で使う予定が無ければ) |
| 7 | MB-06 | `runtime/outbox.rs`(`RuntimeOutbox`、バリアント `StartTsfProbe` 1 つ) | 約 75(46 実数+push・drain・呼び出し 2) | 0 | 同じ TSF probe タイマーの Set が 1 回減る。`drain_output_post_send_effects`(`platform.rs:921`)が同じ `pending_tsf_timer()` を全送信後に適用済み。コメント自身が belt-and-suspenders と書く | defer/replay 再発ファミリーの表に `runtime/outbox.rs` が載っている(表の更新が要る) | windows-build+e2e(Chrome cold) | **撤去** |
| 8 | 新 N-1 | `InjectionModeStore`(読み取り専用の学習済み TSF)と `InjectionHint::ForceVk` / config `force_vk` | 推定 約 70(learning_store 約 50、tracker・classifier 約 20)+ config・settings | 推定 約 20 | 書き込み側は ADR-276 で撤去済み。撤去すると、過去に自動昇格した窓クラスが Vk に戻る(必要なら `force_tsf` を書く)。ストアが無くなると `ForceVk` は `Default` と同値になり、`force_vk` 設定は効果なし | ADR-276 | 実機(学習済み cache.toml を持つ利用者)。CI では差が出ない | **所有者判断** |
| 9 | MB-05 | `session_expired`・`COMPOSITION_TIMEOUT_MS`・`WarmthReason::WarmSessionExpired` | 推定 約 40 | 推定 約 60(plans の全列挙+`architecture_guard.rs:8346-8430`) | send_keys 経由は `mark_send()`(`output/mod.rs:729`)の後に `assess_warmth`(`:808`)を読むので elapsed は常に約 0ms。残る経路は raw recovery の再送(send_keys を通らない)で、2 秒を超える例は未確認 | warmup 再発ファミリー、tuning 定数 1 減 | CI で足りる | **撤去**(elapsed を直すと 2 秒空き後が全部 cold になり挙動が大きく変わるので、直さず消す) |
| 10 | MC-E2 | 書かれない観測枠 `gji`・`tsf`・`hwnd_cache`・`imm_get_open_status` と evidence 型 4 つ、`ForceOnReason::ProfilePolicy`・`expires_at`・`purge_expired` | 推定 約 45 | 推定 約 60 | 4 枠は `Observed<E>` の構築子が無く(`evidence.rs:200-290` に無い)、`record_replayed`(journal の再生)だけが書ける。撤去すると古い journal のそれらのソースは再生で捨てられる。`ObservationSource` の variant は WarrantBasis・`app_ime_policy` が使うので残す | ADR-089(型付き観測)、`architecture_guard::per_source_set_is_confined_to_the_store` | Linux で足りる | **撤去**(belief 領域なので 1 PR を小さく) |
| 11 | 新 N-2 | `ProbeParams.forces_prepend_f2`・`ColdKind::forces_prepend_f2` | 約 12 | 約 40(gji_fsm のテスト 3 本) | 読み手はログ 1 行(`platform.rs:513-516`)だけ。ColdKind の 3 値は `is_long`・`is_proactive` で残る | なし | Linux で足りる | **撤去** |
| 12 | MB-08・MB-17 | 診断だけの非同期 conv 読み(Chrome cold の `[cold-diag]` `vk_send.rs:283-` と warm 毎送信の `[h1-send]` `:518-532`) | 約 35 | 0 | ログ 2 種が消える。送信ごとの `SendMessageTimeoutW` 1 往復(ワーカー)が無くなる | ADR-100・182・188 がこのログを引用 | CI で足りる | **所有者判断**(診断資産。`[cold-diag]` は TSF 側 `cold_warmup.rs` にも残る) |
| 13 | MA-05 | `kp_shadow_actuate`(`key_pipeline.rs:1373-1539`、約 165 行)を `dispatch_set_open`(`sync_actuation.rs:301-393`)へ寄せる | 推定 約 60〜100 | 不明 | journal の記録順・order の発行点が変わりうる | ADR-208 L1・ADR-241、architecture_guard 5844・5914・7651・7669 | 擬似 IME で `dispatch_set_open` を通す ME-2 が未着手(`git grep dispatch_set_open -- crates/*/tests` は guard のみ) | **残す**(ME-2 の後) |
| 14 | MA-03 | `AttemptRecord.with_app_available`(本番で常に `true`) | 約 10(定義・本番 4 か所・journal のログ 2 行) | fixture 37 行+テスト 8 か所 | journal の `actuation decision` から `first_with_app_available` が消える。古い fixture は `serde(default)` か fixture 更新 | ADR-163 | Linux で足りる | **撤去**(小) |
| 15 | MD-03・MD-01 | `classify_focus` の捨てる `ImmContextGuard::new`(`focus/classify.rs:38`)、`uses_kanji_toggle`(テストのみ) | 約 3+約 10 | 約 30 | MD-03: `ImmGetContext` 1 回が減る。MD-01: なし | focus 再発ファミリー(MD-03) | CI で足りる | **撤去**(小。MD-01 は oracle 用の doc があるので所有者判断でもよい) |

表に入れなかった未実施分:

- **MB-10**(Chrome と TSF の cold コルーチンの統合、推定 約 -120): MB-01 済みで前提は満たした。ただし Chrome の `needs_literal = gji_active` を保つ引数が要る(raw-b)。MB-02/03 の後。
- **MC-C1**(`last_intent` と `IntentStore`): 統合は ADR が先。ただし **`awase-ime-core/src/state/mod.rs:106-111` と `intent_store.rs:14` の「未配線」は事実と違う**(`platform_state.rs` が `intent_store.record/lookup/remove/resolve_effective_open` を本番で呼ぶ)。コメントだけ先に直せる。
- **MC-C2**・**MD-07〜09**: 窓モデル(ADR-277 段階 2、`f84588c8`)は入ったが、これらの候補を載せた検証は未実施(未確認)。
- **MD-06**(字形): 一太郎の実機確認待ち。撤去を勧めない(INDEX のとおり)。
- **MB-11・MB-12・MB-13・MB-16**: INDEX の判断のまま(実機の測定が要る)。

## 3. ADR-276 の後に残った名前・コメント・引数

挙動は変わらない(コメントとログ文言)。1 PR にまとめられる。

| 場所 | 古い内容 |
|---|---|
| `awase-windows/src/output/vk_send.rs:143-146` | `send_unicode_char` の doc が `unicode_cold_defer`・`unicode_cold_deferred` を説明(実体なし) |
| `awase-windows/src/output/mod.rs:66` | `unicode_cold_defer` 等を内包、と書く |
| `awase-windows/src/tsf/warmup/mod.rs:24`・`tickable_fsm.rs:13,69-71` | `unicode_cold_warmup_fsm`・`UnicodeColdWarmupFsm`・`push_deferred_unicode_chars`(ファイルも関数も無い) |
| `awase-windows/src/platform.rs:934` | 中身の無い節見出し「Unicode cold-start warmup ヘルパー」 |
| `awase-windows-base/src/vk.rs:68` | `send_unicode_cold_warmup_keys`(存在しない) |
| `awase-ime-core/src/state/gji_direct_mechanism.rs:332-336` | `reopen_obligation` の doc が「Unicode 注入モードでも出す」「`needs_unicode_cold_warmup`」と現在形で書く |
| `awase-windows/src/tsf/ime_mode_fsm.rs:165,282` | 消費者として「Unicode cold-start 観測ゲート」を挙げる(撤去済み) |
| `awase-windows/src/ime_diagnostic.rs:15` | 注入モードを「Vk / Tsf / Unicode」と書く |
| `awase-windows-base/src/focus/kinds.rs:11-12` | Uwp は「Unicode 直接送信」 |
| `awase-windows/src/output/key_injector.rs:54` | 「Unicode モード・Chrome VK モード両用」 |
| `awase-windows/src/tsf/tsf_gate.rs:98` | 「非 TSF アプリ(VK/Unicode モード)」 |
| `awase-windows/src/tsf/observer.rs:169`、`gji_fsm.rs:2595` | 「eager unicode」「Unicode の poke/reinit」 |
| `.claude/rules/fix-requires-evidence.md:36` | warmup 行が `warm_send_plan.rs` の役割に `PendingGjiConfirm` を挙げる |

コードとして残っているもの:

- `CharResolution::Unicode` と `send_unicode_char`(`vk_send.rs:637-642,697-702`): 表に無い文字のフォールバックとして**今も生きている**(注入モードではなく文字単位の送信)。撤去対象ではない。名前は正しい。
- `needs_belief_sync_on(..., false)`(`platform.rs:948-953`): 4 番目の引数は常に `false` を渡し、Mutex を取る判定を外で遅延評価している。Unicode とは無関係の最適化で、そのままでよい。
- `env.is_tsf_mode`(`TsfEnvSnapshot`): `GjiWarmupCoro` は Tsf の送信からだけ作られる(raw recovery の再送を除く)ので、`decide_transmit_plan` での値はほぼ固定。MB-10 の統合で `target` に置き換えると消える。単独で触る価値は小さい。
- `used_eager_path`・`PendingGjiConfirm`: `git grep` で本番 0 件(規約ファイル 1 行のみ)。
- `InjectionMode` は Vk/Tsf の 2 値になり、`InjectionHint::ForceVk | Default` は同じ Vk に写る。ただし `ForceVk` は学習済み TSF を上書きする意味がまだある(表の N-1)。

## 4. 推奨する順序

1. **挙動不変の小 PR**: 3 節のコメント修正+MC-C1 の「未配線」コメント、MB-07、MB-06、N-2、MA-03、MD-03(合計 本番 約 -200 行)。
2. **MB-05**(定数 1 減)と **MC-E2**(belief 領域、1 PR を小さく)。
3. **MB-02+MB-03**(本番 約 -430・テスト 約 -420。warmup 再発ファミリーの (a)/(b) を添える)→ その後 **MB-10**。
4. **MB-09**(ADR-103 の追記とセット)。
5. 所有者判断の 4 件(MC-E1・`ime_profile_driver.rs`・MD-05・N-1)は合計 本番 約 -1,400 行。決まれば機械的に消せる。
6. MA-05・MC-C1(本体)・MC-C2 は ME-2(擬似 IME で `dispatch_set_open` を通す)の後。

## 5. 所有者に聞くべき質問

1. MD-05(メニュー読み取り、本番 約 870 行)を撤去してよいか。ADR-273 で belief には使わないと決めており、既定 off。
2. MC-E1(oracle 2 ファイル、本番 250 行)と `ime_profile_driver.rs`(本番 213 行、ADR-090 で凍結)は、今後使う予定があるか。無ければ撤去する。
3. MB-07 の `CompositionOutput` trait(ADR-030 の他 OS 向け拡張点)を残す理由はまだあるか。
4. N-1: 過去に自動で TSF に昇格した窓クラスの読み込み(`InjectionModeStore`)をやめ、`force_vk` 設定を廃止してよいか(該当する利用者は `force_tsf` を書く必要が出る)。
5. MB-02(warm パスの語単位の事後確認)は、R2 の不具合報告での到達確認を待たずに撤去してよいか。

## 6. 限界

- cargo 未実行。行数の「推定」は関数の範囲を目で数えた値。
- 「本番 0 件」は `git grep` の結果で、マクロ・文字列経由の呼び出しは見ていない。
- MB-05 の「2 秒を超える経路は無い」・MB-02 の「非 Tsf かつ Probing は競合時だけ」は読みで、実機の到達件数は未確認。
