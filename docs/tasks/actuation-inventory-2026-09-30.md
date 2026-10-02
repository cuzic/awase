# IME への書き込み・IME キー注入 全経路の棚卸し (develop ccc966b8)

調査専用。コードは編集していない。行番号は ccc966b8 時点。
「事実」= コードとドキュメントを読んで確認したこと。「推測」= 読解からの推論(実機/CI 未確認)。

既存の棚卸し(docs/tasks/conv-write-paths-inventory.md 2026-09-25、review-2026-09-24-09 の B5 同期節 2026-09-29)と突き合わせ、
その後の変更(BUG-173/174、ADR-203/205/206/207/209/211)で増減した分を反映した。

## 0. 結論(先に)

- **F2(0x71)を送る経路は現存しない**(事実。`0x71`/`VK_F2` は vk.rs の定数表とテストのみ)。予防的 F2 warmup は 2026-07-18 に撤去済み。cold_warmup.rs / gji_warmup_coro.rs / vk_send.rs の `prepend_f2_warmup` はログ用の名残。
- **`VK_IME_ON`(0x16)を送るコード上の送信点は 6 箇所**(下表 A1〜A6)。`VK_IME_OFF`(0x1A)は 3 箇所(A1 の OFF 方向、A5、A6)。
- **eager warmup(`send_eager_tsf_warmup`)の呼び出し元は現時点で 4 つ**残っている。PR #398 で撤去予定の「確定キー reinject」(platform.rs:1373 `on_reinject_key`)以外に、
  **(1) フォーカス変更(ime_refresh.rs:620)、(2) SetOpen(true) 適用後の随伴(platform.rs:1492)、(3) 記号 VK 生フォールバック(vk_send.rs:692、`WarmupImeOn::off()` を渡すので実送信は起きない=事実上デッド)**。
  (platform.rs:295 は (1) の薄いラッパー。`latch_eager_warmup_without_send` は送らず latch のみ)。
- 予防的・補正的で「ユーザーの押したキーを引き金にしない」ものの主なものは次の 7 つ:
  1. フォーカス変更時の eager `VK_IME_ON`(A2)
  2. SetOpen(true) 後の随伴 eager `VK_IME_ON`(A3、結果が `AlreadyMatched`/`AppliedWithoutSendInput` のとき)
  3. Unicode long-cold(≥10s idle)の `VK_IME_ON`+`VK_A`+`BS`(A5)
  4. Unicode long-cold で chars が無いときの `VK_IME_OFF`→`VK_IME_ON` reinit(A6、`Actuation` 起点のみ)
  5. Chrome/TSF リテラル 2 連続 give-up 後の `VK_IME_OFF`→`VK_IME_ON` reinit(A6b)
  6. drift correction(B1、タイマー駆動。GJI/TsfNative では実 VK 送信)
  7. **`ActivationSync` 起源の SetOpen(C1)**: Engine の active/inactive 遷移が「対称性のため」に自動で SetOpen を発行し、そのまま実 actuation に流れる。**ADR-191/199 の「書いてよい例外」に入らないのに、既存の棚卸し(conv-write-paths-inventory/review-09)のどちらにも載っていない**(事実。両文書に `ActivationSync` の語なし)。最優先の確認対象。
- conv 軸(IMC への直接書き込み)は、既存棚卸しの 11 経路のうち経路 7(物理かなキー埋め合わせ、8748d481 で撤去)・経路 9(焦点プローブ、PR #329)・O1(フォーカス強制 OFF、PR #313)が消え、現存は 8 系統(下表 D1〜D8)。

## 1. 表: VK を SendInput で送る経路(IME キー/ IME 状態を変える注入)

分類: 【許可】ユーザーが押した/設定したキーへの直接応答、【予防】、【補正】、【出力】。
「生きているか」: 生=本番で到達可能(事実、呼び出し元を確認)、条件付=特定の窓/IME のときだけ、デッド=到達しない。

| ID | 経路名 | ファイル:行 | 引き金 | 送る内容 | 分類 | 生きているか | 根拠 ADR/BUG | 撤去リスク |
|---|---|---|---|---|---|---|---|---|
| A1 | 戦略チェーンの同期 write(GjiDirect/MsImeDirect) | ime_controller.rs:249,297 → ime.rs:133 `send_ime_mode_key`。組み立て state/key_sequence_policy.rs:132-139 | `ImeEffect::SetOpen`(engine 決定)。origin が **ExplicitUserAction**(IME ON/OFF コンボ・`keys.ime_on/off`・ADR-206 の役割トグル・エンジン ON/OFF コンボ)と **ActivationSync**(C1)の両方がここへ来る。他に B1 の drift correction(非 ImmCross) | `VK_IME_ON`/`VK_IME_OFF`(IME_KANJI_MARKER)。GJI: shadow が確認済み一致なら `AlreadyMatched` で送らない | ExplicitUserAction は【許可】。ActivationSync は【補正/予防】(C1 参照) | 生。GJI 検出時=GjiDirect、MS-IME=MsImeDirect(Standard で ImmCross が先、失敗時フォールスルー) | ADR-089/090/119/199/206、BUG-113(重複送信で「@」)、BUG-16、BUG-90 | 【許可】部分は撤去不可 |
| A2 | フォーカス変更時の eager warmup | runtime/ime_refresh.rs:611 `send_eager_warmup` → platform.rs:295 → output/mod.rs:1134→1196 → tsf/send.rs:27 `send_eager_warmup_vk_pair` | フォーカス変更確定(`ir_post_focus_change_snapshot`)。ユーザー操作は引き金でない | `VK_IME_ON`(TSF_MARKER、scan 付き) | 【予防】 | 条件付。全部満たすときだけ送る: `conv_mutation_allowed`(=Engine ON=AwaseOwned)、`needs_f2_probe`(GJI のみ)、`ime_on && is_tsf_mode`(InjectionMode::Tsf=WezTerm 等 force_tsf/学習昇格アプリ。Windows Terminal/Chrome は Unicode/Vk なので対象外、ADR-100 の訂正) | ADR-098 決定1-b、ADR-100、BUG-02(WezTerm「この→kおの」)、BUG-69、BUG-32(Win 押下中スキップ) | 中。WezTerm+GJI の cold 直後 1 文字目リテラル化(BUG-02 系)。experiments.md エントリ 10(予防待機撤去は数日ソークで無破損)は待機/F2 の撤去であって VK_IME_ON warmup 自体ではない。実測: ADR-100 F16 は「Windows Terminal で全面無効化しても問題なし(3 シナリオ、少数)」のみで **WezTerm での無効化は未検証**(BUG-174 未確認 4) |
| A3 | SetOpen(true) 適用後の随伴 eager warmup | platform.rs:1479-1497 `on_ime_applied_inner`(`feed_composition_event` の後) | `on_ime_applied(open=true)`。つまり A1/C1 や ImmCross の書き込み完了 | `should_send_accompanying_warmup(outcome)`(src/platform.rs:247、`!Applied`)が真なら `send_eager_tsf_warmup(Actuated)`。**`AlreadyMatched`/`AppliedWithoutSendInput` のとき A2 と同じ `VK_IME_ON` を実送信**。偽なら latch のみ | 【予防】(実 actuation の結果に付随) | 条件付(A2 と同じ 3 ゲート。`WarmupImeOn::from_actuated(effective)`なので drift ゲート `off_drift_active` を通らない=既知の限界、コメント platform.rs:1439-1446) | ADR-149、ADR-167、ADR-132(INV-B1' はこの経路に及ばない)、BUG-113 | 中。A2 と同型。GJI+TSF で `AlreadyMatched`(shadow 一致で戦略が送らなかった)ときに warmup だけが送られる、が実挙動(事実、コードから)。BUG-113 の「@」必要条件(連続 2 回 SendInput)を再現しうる |
| A4 | 確定キー reinject 時の eager warmup(PR #398 で撤去予定) | runtime/executor.rs:638 `handle_reinject` → platform.rs:1345-1375 `on_reinject_key` | Enter/Space/Esc の KeyDown を reinject するとき、composition が cold なら | `VK_IME_ON` | 【予防】 | 条件付。実機 A/B(bug173-remaining §2-追記): Engine OFF のまま生ローマ字を通す状態(IME ON・Engine OFF)でのみ通る。NICOLA ON では `[relay-defer]` 経路で通らない | BUG-173/174、ADR-098、BUG-40(3ffbe66)、BUG-171 | 撤去済み予定。実機 24→0、失敗増なし(n=24)。NICOLA ON・MS-IME・Chrome では未測定 |
| A5 | Unicode long-cold warmup | output/mod.rs:751-778 `send_unicode_cold_warmup_keys` ← platform.rs:1106 `start_unicode_cold_warmup` ← platform.rs:1215 `needs_unicode_cold_warmup`(send_keys)/ platform.rs:131 `unicode_long_cold_probe` | ユーザーの文字出力(NICOLA の打鍵)が Unicode モードで GjiFsm long-cold(≥10s idle)かつ Char/Romaji を含むとき | `VK_IME_ON`(IME_KANJI_MARKER)+ 犠牲キー `VK_A`+`VK_BACK`(INJECTED_MARKER) | 【予防】(cold 化対策。犠牲キーは【出力】寄りだが目的は GJI 起動) | 条件付。GJI・Unicode injection mode(Windows Terminal 等)・≥10s idle | ADR-203、BUG-02 系(GJI cold で `bあ`)、experiments エントリ 10 で「捨て駒キー」は Tsf/Chrome 側だけ撤去済み。Unicode 側は残存 | 高。撤去するとGJI long-cold 後の最初の文字が Unicode 注入で GJI に拾われず欠落/リテラルになりうる(推測。Unicode 注入は GJI 確認を迂回する: feedback_unicode_injection_bypasses_gji_composition)。撤去には ADR-203 の追随と、Windows Terminal+GJI で 10s+ idle 後の 1 文字目検証が要る |
| A6 | Unicode long-cold の reinit(chars なし) | platform.rs:141-144 → output/mod.rs:1582 `send_f22_f21_reinit` → output/probe_io.rs:186-227 | GjiFsm `StartProbe`(long-cold)で deferred chars が無いとき。`origin==BeliefSync` では抑止済み(ADR-203 決定3)。`origin==Actuation`(A1/C1 由来の同期)では**送る** | `VK_IME_OFF`→`VK_IME_ON`(IME_KANJI_MARKER、4 イベント同一バッチ) | 【予防/補正】 | 条件付(Unicode モード+GJI+long-cold+Actuation 起点)。`CHROME_GJI_REINIT_CONFIRM_MS` のレート制限あり | ADR-203 決定3、ADR-191 | 中〜高。OFF→ON の間に IME 状態が一瞬 OFF になる副作用。ADR-203 が BeliefSync だけ止めたため、Actuation 起点分が唯一の残り(事実) |
| A6b | Chrome/TSF リテラル give-up の reinit | output/probe_io.rs:975 `schedule_chrome_gji_reinit`(予約)→ output/mod.rs:596,1923 `start_pending_gji_reinit_after_raw_cleanup`(BS flush 直後に実行)→ probe_io.rs:186 | 打鍵後に literal を連続 2 回検出→give-up。予約は `negative_evidence >= MIN_NEGATIVE_EVIDENCE_FOR_REINIT` のときだけ(BUG-168/ADR-200) | `VK_IME_OFF`→`VK_IME_ON`(+ 10ms 間隔の IMC ポーリング) | 【補正】(事後回復) | 条件付。GJI×TsfNative のみ。実 Chrome×GJI では 0/10 で開け直せなかった(review-09、2026-09-29 追補)=効果が実測で乏しい | BUG-33、BUG-36(BS との順序)、BUG-168(入力中文字消失)、ADR-200 | 中。撤去すると連続 literal 化後の自己回復が無くなり BS だけで済ませる。実 Chrome で効果 0/10 の実測があるので撤去候補としては優先度が高い(推測: 逆に RichEdit 入力先の tsf×GJI では 30/30 効いた=同文書) |
| A7 | 半角英数トグル(GJI)の entry/exit | ime.rs:236-283 `send_ime_mode_key_with_shift_release_prefix` ← output/mod.rs:1216-1259 `send_gji_half_width_alnum_toggle` ← key_pipeline.rs:1952(Enter)/2021(Exit) | ユーザーの左 Shift 単独タップ(Enter/Exit)。フォーカス変更(ime_refresh.rs:334)・IME ON 経路(key_pipeline.rs:1099/1147/1411)でトグル中なら Exit を送る | `VK_DBE_ALPHANUMERIC`(scan=0)/`VK_DBE_HIRAGANA`(scan 付き)+ 合成 Shift↑ 前置 | 【許可寄り】ただし ADR-191/199 の例外(IME ON/OFF トグルの役割キー・keys.ime_on/off)の外。左 Shift は IME キーではない。フォーカス変更・IME ON 時の強制 Exit は awase が作った状態の後始末 | 条件付。`half_width_alnum_toggle` 既定は `MsImeOnly`(src/config.rs:429)なので GJI では既定で Enter しない(policy が `All` のときだけ)。Exit は toggle 中のみ | ADR-107、BUG-25、BUG-15、experiments エントリ 07/08/09(scan 付き ALPHANUMERIC は CapsLock 汚染・フック不達で撤回) | 低〜中。所有者判断(機能自体の要否)。conv-write-paths-inventory 経路 4〜6 と同じ扱い |
| A8 | 半角英数トグル(MS-IME)の復元でのモードキー注入 | key_pipeline.rs:2117-2170 `kp_restore_kana_from_half_width`(scan 付き `VK_DBE_HIRAGANA`) | A7 と同じ Exit。`MicrosoftIme`+開+Win/Alt 非押下のときだけ | `VK_DBE_HIRAGANA`(+ 合成 Shift↑) | 【許可寄り】(A7 の対) | 条件付 | ADR-084、BUG-49 追補、BUG-15 | A7 と同じ(4 を残す限り必須) |
| A9 | Ctrl+変換で IME 既に ON のときのリセット | key_pipeline.rs:1379 → 1456 `kp_reset_to_hiragana_romaji_capsoff`(Caps Lock 解除の SendInput + conv 書き込み D5) | IME-ON コンボ(既定 Ctrl+変換)を IME ON 中に押下 | Caps Lock トグル(ime.rs:1654、VK 0x14)+ conv 書き込み | 【許可】(ユーザーの明示コンボ) | 生(`is_default_ime_on_combo`。`keys.ime_on` をカスタムすると条件が外れるとコメントにある=推測でなく事実) | ユーザー要望 2026-07-11、BUG-50 | 低。撤去不要 |
| A10 | ホットキー/トレイ操作 | message_handlers.rs:1252,1257,1264(CapsLock/ResetState)、runtime/mod.rs:2314-2337 `panic_reset`(IME OFF→ON+ひらがな化+全修飾キー↑) | トレイ選択・IME 関連キー連打(緊急) | Caps Lock、IMC 書き込み、`ImmSetOpenStatus` OFF→ON | 【許可】(ユーザーの明示操作/緊急リセット) | 生 | ADR-094(ROMAN をマスクから除外) | 撤去不要 |
| A11 | Ctrl バイパス/`[[keymap]]` 一致時の composition キャンセル | runtime/mod.rs:2431 `cancel_ime_composition`(`ImmNotifyIME(CPS_CANCEL)`)← message_handlers.rs:319 `cancel_composition` | Ctrl+非修飾キーの PassThrough 確定時(GJI 候補ウィンドウ中)、`[[keymap]]` ルール一致 | 未確定文字列の破棄 | 【許可寄り】(ユーザーのキー操作に伴う) | 生(composition 中のみ) | ADR-114 | 撤去不要(ユーザー期待の動作) |
| A12 | 文字出力に付随する注入 | tsf/output.rs:185 `flush_raw_tsf_literal_backspaces`(ESC/BS)、output/key_injector.rs 各 `send_*`、output/vk_send.rs:158(Unicode kana) | NICOLA 出力、リテラル検出の掃除 | 文字/BS/ESC | 【出力】 | 生 | BUG-24/33/36/168/171 | 出力の一部。IME 状態を変えない(ただし ESC は composition を消す=BUG-171) |
| A13 | 自己注入の補助 | held_modifiers.rs:134 `send_keymap_target`(`[[keymap]]` ターゲット)、hook.rs:445 Alt メニューマスク(Ctrl↓↑)、hook.rs:1338 カナリア(Ctrl↓↑、`HOOK_WATCHDOG_CANARY_MARKER`)、lib.rs:405 `reinject`、runtime/mod.rs:2421 `send_all_modifier_key_ups` | ユーザーのキー操作/watchdog | 修飾キー/リマップ先 | 【出力】/ ユーザー設定 | 生 | ADR-114、issue #165、BUG-62 | IME への書き込みではない。カナリアのみ独立(ADR-062 系ではなく issue #165)。リスト外扱いでよい |

## 2. 表: IMM/IMC(WM_IME_CONTROL/ImmSet*)の書き込み経路(SendInput でないもの)

| ID | 経路名 | ファイル:行 | 引き金 | 書く内容 | 分類 | 生きているか | 根拠 | 撤去リスク |
|---|---|---|---|---|---|---|---|---|
| B1 | drift correction | runtime/ime_refresh.rs:688-1030 `ir_apply_drift_correction`(呼び出しは 308 `ir_stage_notify` のみ) | IME リフレッシュのタイマー tick。`desired_open`≠観測が閾値(明示意図があれば 0ms)以上継続 | ImmCross: `set_ime_open_ordered`(ime_refresh.rs:996 → platform.rs:1605 → `ImmSetOpenStatus` async)。それ以外(GJI/TsfNative/Blacklist): `build_ime_control_view(None)` → `apply_ime_open_with_view` → `ImeController::apply` → A1 と同じ VK_IME_ON/OFF | 【補正】 | 生。ただし warrant(ADR-090 A-2)が下りない補正は書かない。ユーザー無操作でも `desired_open` が古ければ発火。conv 由来(`ConvOpenInference`)は明示意図エピソードあたり 1 回に抑止済み(BUG-113 残置課題)。**実 Chrome では観測が乗らず(observed=0)判断に届かない**(BUG-172 測定) | BUG-020、BUG-043、BUG-033、BUG-068、BUG-113、ADR-080/082/090/132 | 高リスク。C-2 の「TsfNative の ON 回復は drift correction だけ」。PR #360 は `ConvOpenInference` 由来の発火のみ撤去。全面撤去すると、belief=OFF/実 IME ON(または逆)のずれが観測で上書きされるまで残る。持続時間は未検証(review-09 T3)。撤去前に実 Chrome/Windows Terminal でずれの持続時間を測ること |
| B2 | 非同期 ImmCross write(Standard プロファイル、engine 決定) | runtime/executor.rs:754-873 `dispatch_ime_set_open`(imm_first 分岐)→ runtime/open_chain.rs `run_open_chain_async`/`imm_cross_write`(312) | A1 と同じ `ImeEffect::SetOpen` | `ImmSetOpenStatus`(+ D2 の ROMAN 補完) | ExplicitUserAction【許可】/ ActivationSync【補正】 | 生(Standard/ImmCross プロファイルの窓) | ADR-089 Phase B、ADR-117、ADR-167 | 【許可】部分は撤去不可 |
| B3 | 同期 ImmCross write | ime_controller.rs:235 `set_ime_open_cross_process` | `apply_mechanism` の ImmCross 分岐(`DecisionSite::Sync`)。`imm_cross_is_first_applicable` で async 分岐しない Standard 以外の同期呼び出しのみ | `ImmSetOpenStatus` | B2 と同じ | 条件付(現存呼び出し元は B1 の Blacklist 分岐等。ime_controller.rs のコメント参照) | ADR-089 §9-21 | 統合済み(構造上必要) |
| B4 | パニックリセット/トレイの開閉書き込み | runtime/mod.rs:2331-2334、ime.rs:1691 `set_ime_mode_for_target` | A10 と同じ | `ImmSetOpenStatus` OFF→ON、`set_ime_hiragana_mode_cross_process_async` | 【許可】 | 生 | ADR-094 | 撤去不要 |

## 3. 表: conv 軸(変換モード)の書き込み

conv-write-paths-inventory.md(2026-09-25)の再確認。HEAD で存在を確認したものだけ挙げる(事実)。

| ID | 経路 | ファイル:行 | 引き金 | 書く内容 | 分類(旧棚卸しの A/B/C) | 生きているか / リスク |
|---|---|---|---|---|---|---|
| D1 | ROMAN 補完(同期・開閉の前段) | ime_controller.rs:208,410-449 `romaji_pre_write`。条件 state/ime_actuation_decision.rs:230 `decide_needs_romaji_pre_write`(開く方向・ImmCross/MsImeDirect・MS-IME・belief がかな入力でない) | A1/B2 の開く方向の SetOpen に付随 | conv に ROMAN を足す(`ActuationTarget` 捕獲) | 【補正】(旧 A) | 生。撤去には MS-IME 本体の実機確認(かな入力へ落ちる症状)が要る。D2 と同時に扱うこと |
| D2 | ROMAN 補完(非同期 ImmCross の後段) | runtime/executor.rs:925 `decide_dispatch_conv_after_open` → open_chain.rs:312 → ime.rs:1342 `set_ime_open_then_conv_for_target` | B2 に付随 | conv に ROMAN を足す | 【補正】(旧 A) | 生。D1 と条件が意図的に別(D2 は IME 種別を見ない) |
| D3 | cold-start の ROMAN 保護 | tsf/warmup/cold_warmup.rs:45-98 `run_start`(vk_send.rs:440 から) | TSF cold-start の各 cold 化(ユーザーの文字出力時) | conv に ROMAN を足す(`set_ime_conv_for_target(None)`。`conv_mutation_allowed` のときだけ) | 【予防】(旧 C) | 生。ADR-191 決定1・BUG-19 が warmup 例外として維持。cold のたびに毎回書く点は再検討余地あり(推測) |
| D4 | 半角英数トグル entry(MS-IME) | key_pipeline.rs:1902 `actuate_conv_mode` → output/conv_actuation.rs:141-186 | 左 Shift 単独タップ(`half_width_alnum_toggle` 既定 `MsImeOnly`=既定で有効) | conv=0x0000 | 【許可寄り】(ユーザー操作。ただし IME キーではない) | 生。旧棚卸しは「opt-in」と書いたが、config 既定は `MsImeOnly` で MS-IME では既定で有効(事実、src/config.rs:429) |
| D5 | Ctrl+変換リセット | key_pipeline.rs:1456-1510(A9 の conv 側) | A9 | conv=ひらがな+ローマ字 | 【許可】 | 生 |
| D6 | 半角英数トグル Exit の IMC 復元(最大 4 回 160ms 間隔) | key_pipeline.rs:2227-2300 | A7/A8 の Exit | conv=かな入力 | 【許可寄り】(D4 の対) | 生 |
| D7 | トレイ「状態をリセット」 | message_handlers.rs:1264 → ime.rs:1691 | トレイ | 開を書き conv にマスク | 【許可】 | 生 |
| D8 | パニックリセット | runtime/mod.rs:2334 → ime.rs:708 | A10 | conv=ひらがな+ローマ字 | 【許可】 | 生 |

撤去済みで現存しないことを確認したもの(事実): 焦点プローブの ROMAN 修正(旧経路 9)、フォーカス変更時の強制 OFF(ime_refresh.rs:589-593 にコメントのみ)、物理かなキーの埋め合わせ(旧経路 7)、force-on/reassert(`apply_force_on_for_imm_broken` 等)、`send_engine_state_ime_key`/`engine_on_ime_key`(ADR-207、config は旧キーの読み飛ばしのみ)、Ctrl↑ の eager warmup(BUG-174)、F2 の予防送信、待機行列/捨て駒キー(Tsf/Chrome 側)。

## 4. 論点別の事実と推測

### C1. `ActivationSync` 起源の SetOpen が実 actuation に流れる(未棚卸し)

事実:
- src/engine/engine.rs:339-402 `check_active_transition` が、毎キー入力・`RefreshState`(IME ポーリング/idle-conv-check 由来、キー入力と無関係にも発火)で Engine の active/inactive が遷移したとき `transition_activation(new_state, SetOpenOrigin::ActivationSync)` を呼び、`ImeEffect::SetOpen{open: now_active}` を発行する(engine.rs:430-436。`NotRomajiInput` のときだけ抑止)。
- executor.rs:670 `dispatch_effect` は `SetOpen{open, ..}` の origin を見ずに `dispatch_ime_set_open` へ流す。origin による分岐は belief 側(key_pipeline.rs:1321-1345 `last_intent` を書くか)だけ。したがって ActivationSync も A1/B2 と同じ機構チェーンで実 VK/ImmSetOpenStatus を送る(ADR-154 も「GjiDirectStrategy 経由の ActivationSync 送信」と明記)。
- 抑止条件: focus-settle 中は effect ごと除去(executor.rs:146-165)、InputRelay は NotOwned、GJI は shadow 一致なら `AlreadyMatched`、warrant が下りなければ `Unwarranted`。
- ADR-191/199 の例外は「開閉のみに作用するキー(冪等 ON/OFF・トグル)を、ユーザーが押したときに書く」であり、観測駆動の Engine 遷移の echo はそれに該当しない。
- ADR-207 で `send_engine_state_ime_key`(エンジン ON/OFF 時のモードキー送信)は撤去済みだが、その手前の SetOpen 自体(ActivationSync)は残っている(executor.rs:801 コメントが撤去済みの旧機構に言及)。

推測(要確認):
- ActivationSync の SetOpen が実際に VK を送る頻度は、多くの場合 GJI の `AlreadyMatched`(直前に belief と一致しているとき)で吸収されているはず。しかし belief が「未知」(TsfNative では `applied` を `Unknown` のまま維持、ime_refresh.rs:568-571)のとき `shadow_on=None` なので吸収されず、Engine の活性遷移(例: input_mode 観測の変化で `NotRomajiInput`→active)のたびに `VK_IME_ON` が飛ぶ可能性がある。journal の `ActuationDecision`(caller=DispatchImeSetOpen)と `OpenApplyReason::EngineDecision` で件数を集計できる。
- 検証案: 実機(GJI+Windows Terminal)で journal を取り、`SetOpenRequest origin=ActivationSync` の件数と、そのうち `outcome=Applied`(実送信)の件数を数える。これがゼロに近ければ ActivationSync の SetOpen を effect 発行元で落とす(belief 側の `handle_engine_activation_sync` は残す)のは低リスクで、「書かない」原則に合う。

### A3 の詳細(随伴 warmup)

- 呼び出しは `on_ime_applied_inner` 内で `open==true` のとき必ず通る(platform.rs:1479)。送信可否は `should_send_accompanying_warmup(outcome)=!Applied`。
- ADR-149/167 は「戦略が既に VK_IME_ON を送った(Applied)なら重ねない」設計で、`AppliedWithoutSendInput`(ImmCross)・`AlreadyMatched`・`Failed` は「送る」側。`Failed` は `effective=!open` で `can_warmup` が偽になり no-op(コメント src/platform.rs:222-231)。
- ImmCross(Standard)は通常 `is_tsf_mode=false`(Vk/Unicode 系)で A2 と同じ 3 ゲートに落ちる=実送信されない(推測。Standard 窓の InjectionMode は確認していない)。実送信が起きうるのは「GJI かつ InjectionMode::Tsf かつ `AlreadyMatched`/ImmCross 成功」。範囲は狭い。

### 記号 VK フォールバック(vk_send.rs:692)

事実: `send_eager_tsf_warmup(WarmupImeOn::off(), Off)` を渡すため `can_warmup()`(`ime_on` が偽)で必ず早期 return。コメントも「理論上到達しない」と書いている。デッドコードとして撤去可能(挙動変化なし、推測でなく `WarmupImeOn::off()` の定義から)。

### lint 許可リストとの関係

`lints/actuation_call_guard/src/lib.rs` の `send_input_safe` 許可リスト 19 関数のうち、IME 状態に作用しうるのは `send_ime_mode_key`、`send_ime_mode_key_with_shift_release_prefix`、`send_chrome_gji_reinit_and_poll`、`kp_restore_kana_from_half_width`、`send_unicode_cold_warmup_keys`、`send_eager_warmup_vk_pair`、`toggle_caps_lock`。残りは出力/自己注入。
`set_ime_conv_for_target`/`set_ime_mode_for_target`/`set_ime_romaji_mode_for_hwnd`/`ImmNotifyIME`(A11)は lint に無い(事実、review-09 が既に指摘。件数ガード T6 は未確認)。

## 5. 撤去の優先順位と検証案(予防的・補正的のみ)

判断軸: (1) ADR-191 の原則からの距離、(2) 撤去で戻りうる不具合の実測有無、(3) 検証手段が CI で足りるか。

| 優先 | 対象 | 理由 | 必要な検証(退行を見るシナリオ) |
|---|---|---|---|
| 1 | **A4(PR #398)** 確定キー reinject warmup | 既に実機 24→0、失敗増なし。BUG-174 未確認 2 の残り | 実機: Windows Terminal+GJI(MS-IME プリセット)で Enter 連打時の `VK_IME_ON` 0 件・リテラル 0 件。CI: `tsx-chromepage-gji-20ms-cold`、`tsx-tsf-gji-20ms-cold`。**未測定条件(NICOLA ON、MS-IME、Chrome)を追加** |
| 2 | 記号 VK フォールバックの `send_eager_tsf_warmup(off)` 呼び出し(vk_send.rs:692) | 実送信されないデッドコード | コンパイル+architecture_guard のみ。挙動変化なし |
| 3 | **C1 ActivationSync SetOpen の actuation 経路** | 原則に最も反し、かつ未棚卸し。ただし頻度未測定 | まず計測(journal で ActivationSync 起点の実送信件数、TsfNative×`shadow_on=None`)。0 件に近ければ effect 発行を止める(belief 側は維持)。回帰は closed_loop_scenarios/golden、CI e2e(sc-*)の Engine ON/OFF 追随、実機で「IME OFF 後 Engine が勝手に ON に戻る」(2026-08-04 の再発対策の対象)を確認 |
| 4 | **A6b Chrome/TSF give-up reinit** | 実 Chrome×GJI で 0/10 と実測で効かない。BUG-168 で入力中文字を消す副作用も既知 | CI: tsf/chromepage の literal 連続検出シナリオ(cal-* の literal 系)で `gave_up` 後の自己回復の有無。BS のみに縮退した場合の連続 literal 件数 |
| 5 | **A2 フォーカス変更 eager warmup**+**A3 随伴 warmup** | InjectionMode::Tsf(WezTerm 等)+GJI のみ。ADR-100 F16 は Windows Terminal で無効化して問題なし(少数)だが WezTerm は未検証 | 実機: WezTerm+GJI で idle 後フォーカス→最初の文字が `kおの` 化しないか(BUG-02)。experiments エントリ 10 の教訓どおり、フラグで無効化→数日ソーク(cold 60 件超)→恒久化の順。CI に WezTerm 相当の構成があれば ab-* シナリオ(ci/e2e-warmup-ab)を Tsf 指定で流す |
| 6 | **B1 drift correction** の non-ImmCross 分岐(VK 実送信) | C-2 の前提。実 Chrome では観測が乗らず「回復するか」自体が未確認 | 撤去前に「ずれの持続時間」を測る(review-09 T3)。cal-driftrec-* の観測経路を先に整備(BUG-172 の次の一手)。撤去は最後 |
| 7 | **A5/A6 Unicode long-cold warmup/reinit** | ADR-203 が BeliefSync 分の reinit を止めた続き。Actuation 起点分(A6)は同種で止められる可能性 | 実機: Windows Terminal+GJI、10s+ idle 後の 1 文字目(`bあ` 型欠落の有無)。A6 だけ先に BeliefSync 同様に抑止する案は影響範囲が狭い(chars 無しのときだけ) |
| 8 | **D1/D2 ROMAN 補完**(既存棚卸し A) | 旧棚卸しの推奨順どおり。D1 と D2 は同時に | CI e2e-ime.yml の MS-IME 構成でかなモード落ちがないか。実機 MS-IME 本体 |
| — | D3(cold の ROMAN 保護) | warmup 例外として維持(ADR-191 決定1)。cold 毎回書く点の縮小は別途 | — |
| — | A1/B2 の ExplicitUserAction 部分、A9/A10/A11、D5/D7/D8 | 【許可】。撤去対象外 | — |
| 要判断 | A7/A8/D4/D6(半角英数トグル) | 左 Shift は IME キーでなく ADR-199 の例外外。MS-IME では既定で有効。所有者判断 | 所有者が機能の要否を決める |

撤去時の共通事項(リポジトリ規約):
- fix-requires-evidence.md の再発ファミリー(warmup/focus/belief/conv/actuation 合流点)に該当。回帰テスト(golden/journal replay)か known-bugs を必ず添える。
- experiment-logging.md: 撤去後に revert する場合は「アプリ・IME・症状」を本文に書く。experiments.md に判定を追記(PR #360 の 2 行は「未判定」のまま)。
- complexity-budget.md は未発効だが、`RESTRICTED_CALLS`/tuning 定数の削除は差し引きとして数える価値がある。A2/A3/A6b を撤去すると `send_eager_warmup_vk_pair`、`send_chrome_gji_reinit_and_poll`(許可リスト 2 関数)と関連 tuning 定数(`CHROME_GJI_REINIT_*`)が消せる。

## 6. 未確認・限界

- ActivationSync の実送信頻度(C1)、A3 が実送信するアプリ/IME の組(Standard 窓の InjectionMode を確認していない)は未測定。
- A5/A6 は Unicode モード+GJI の long-cold(≥10s)の実機ログで頻度を確認していない。
- 撤去リスクの記述のうち「推測」と明記したものは、コード読解と過去ドキュメントからの推論で、実機/CI で未確認。
- 行番号は ccc966b8。PR #398・feat/v2-* が入ると変わる。
