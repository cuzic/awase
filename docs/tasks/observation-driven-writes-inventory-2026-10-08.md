# 観測・状態を引き金に IME/OS へ書き込む経路の棚卸し(2026-10-08)

基準: ローカル `develop` `4ce07e81`(= `origin/develop` `5ed1d151` + docs 1 件)。調査エージェントによるコード読み(精読していない領域は末尾)。ユーザーの明示操作を直接受けた書き込みは対象外。動機は、ADR-229 の Effect 列の検討で「観測は Write を生成しない」前提が成り立たないと分かった(`docs/adr/review/229-opus-effect-plan-round1.md` B1)ので、実態を数えること。

## 結論

- **観測を見て IME の開閉を書く経路は drift correction の 1 本だけ**(`runtime/ime_refresh.rs:878` `ir_apply_drift_correction`)。`evaluate_drift`(`state/drift_correction.rs:108`)が明示意図(`last_intent` == desired)を要求する。ADR-212 P6 で (b)(c) を外し (a) 明示意図の再試行だけ残した。
- 書き込みは開ループで成功の返事が無い(BUG-20)。届いたかは後の観測でしか分からず、観測起点の再送は閉ループ制御になる。書き込み完了のたびに 20ms 後の refresh を武装する(`runtime/mod.rs:925`)。
- 明示意図を要求せず、観測や状態だけで書く経路が別にある(下表 #9〜#11・#13b・#17・#18)。
- **「drift は明示意図が必須」を守っているのは `evaluate_drift` の 1 点だけ**。`apply_sync` と `issue_open_warrant`(`state/open_warrant.rs:160-226` の Step 3/4a/4c)は観測やヒューリスティックだけでも授権できる。今は全呼び出し元が上流で意図を持つが、sink 側に保証が無い。

## 一覧(行番号は基準コミット)

| # | 経路 | 書く先 | 引き金 | 明示意図 | 有界/世代ガード |
|---|---|---|---|---|---|
| 1-2 | `ir_apply_drift_correction` → `set_ime_open_ordered` / `apply_ime_open_with_view` | IME 開閉 | desired と信頼観測の食い違い(鮮度 1500ms) | 必須 | Blind は上限+3s cooldown、Read は収束まで。focus 変更で意図を捨てる |
| 3-4 | `schedule_settle_retry`、`on_ime_apply_complete` | 書かない(再武装) | settle 明け、書き込み完了 | — | 閉ループを成す |
| 5-6 | `dispatch_set_open`/`run_open_chain_async`、`imm_cross_write` の Failed 分岐 | IME 開閉 | ユーザー押下+観測(一致なら省略、失敗なら追い送り) | 押下 | 押下 ID、`focus_gen` |
| 9 | `cold_warmup.rs:45` | conv に ROMAN | TSF の cold 化 | **無し** | `conv_mutation_allowed` のみ。有界性なし |
| 10-11 | `romaji_pre_write`、`set_ime_open_then_conv_for_target` | conv に ROMAN | open(true) の書き込みに便乗、belief が ObservedKana でない | **無し** | 条件が #9・#14 と別(#11 は IME 種別を見ない) |
| 13 | `kp_restore_kana_from_half_width` | F2 / IMC conv | 半角英数トグルの後始末(awase が作った状態) | 不要 | 世代ガード。**focus 変更時は IMC が必ず中断(BUG-193)** |
| 17 | `RawTsfLiteralRecovery` → `flush_raw_tsf_literal_recovery` | ESC/BS/再送 | LiteralDetector の観測 | **無し** | **focus 世代ガード無し(BUG-194)** |
| 18 | `cancel_ime_composition`(`ImmNotifyIME CPS_CANCEL`) | 未確定文字列の破棄 | 物理キー+候補窓可視 | キー | 1 キー 1 回。**`RESTRICTED_CALLS` と `architecture_guard` の対象外** |
| 他 | #7 shadow toggle の no-op 書き込み、#12 トグル開始、#14 リセット、#19 hook watchdog canary | 各種 | ユーザー操作・時間 | 各種 | — |

belief だけを動かす経路(OS/IME へは書かない): `ir_follow_external_change`・`ir_follow_direct_mode_key_read`(ADR-205・188・244)、`kp_predict_key_effect`、`apply_idle_conv_check`、`apply_focus_probe`。追随は明示意図を捨てるので、結果として drift correction を無効にする。

## 重複して同じ判断を持つ箇所

- 「明示意図」が二重: `ImeModel.last_intent`(TTL なし、focus 変更で捨てる)と `IntentStore`(HWND ごとに TTL 10s/30s)。drift の gate は前者、warrant の Step 1 は後者。
- ROMAN 補完の発火条件が 4 か所(#9〜#11、#14)。conv のマスクも ROMAN を含む/含まないが混在(tray の ResetState は含まない)。
- 「すでに一致」の判定が 3 系統(shadow 基準、ライブ再読み、drift の `read_back`)。
- focus 起因の無効化が複数(`FocusFence`、`ime_mode_focus_gen`、`shift_conv_guard_gen`、押下 ID 台帳、`discard_actuation`)。
- 残骸の疑い: `kp_apply_conv_engine_sync`(`key_pipeline.rs:858-875`)は ConvOpenInference を drift 補正に委ねて refresh を武装するが、`evaluate_drift` は同観測を根拠から除く(`drift_correction.rs:130`)。

## CI で確定させたこと(ブランチ `ci/e2e-focus-restore-13-17`、先頭 `c629db6b`、未マージ)

- #13 → [BUG-193](../known-bugs/BUG-193.md)。構成 `sc-focusrestore-{msime,gji}`・`-ctl`・`-bopen`(observe)。run 37781262341(本体・対照)、37782133675(bopen)。各構成の有効試行は 5/5。
- #17 → [BUG-194](../known-bugs/BUG-194.md)。特性テスト `output::tests::raw_tsf_literal_record_survives_cancel_probe_and_focus_cold_mark`、run 37779407207 の `windows-build` で PASS。

## 未確認・精読していない領域

- `ime_coordinator.rs`、`observer/*`、`app/*`、`tsf/win_event_obs.rs`、`tsf_gate.rs`、`probe_fsm.rs`、`gji_warmup_coro.rs` の内部。
- `drift` の Read 方針の実効的な上限(attempts の上限が無く、`last_intent` の消滅・Read 用 warrant の TTL・ポーリング停止の組み合わせ、推測)。
- `presync_applied_open_on`(`focus_tracking.rs:196`)が書かずに `record_confirmed(true)` で applied を確定にする。`fix-requires-evidence.md` の `shadow_on` の行(陽性の確認済み証拠にのみ基づく)との整合は未確認。
- #13b の窓 A が 0x0 → 0x10 に変わる遷移の主体、実 Chrome・UWP を窓 B にした場合。
- `docs/tasks/conv-write-paths-inventory.md` は一部が古い(経路 7・O1 は現行コードに無い)。

## 整理の方向(未決。所有者判断)

A. `ActuationOrder` の起案元を型で「明示操作」「reconcile」「後始末」「回収」に分け、観測だけでは order を作れないようにする(ADR-212 の将来案)。B. 意図の定義を 1 つにする。C. 明示意図なしの書き込み(#9・#17)に有界性と世代ガードを揃える。
