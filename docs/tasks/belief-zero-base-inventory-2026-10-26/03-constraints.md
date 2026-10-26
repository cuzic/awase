# 03 ゼロベース設計が守るべき条件

出典は、`02-ledger.md` の行(BUG・ADR)、`01-map.md` のコード確認、`docs/experiments.md`。実機の記録は、その BUG・ADR が書いている範囲だけを引く(今回は実機ログを読んでいない)。
「満たしているか」の欄は現行コードの現状で、設計の評価ではない。

## (a) 所有者の方針(2026-10-26 の指示)

**C-a1.** IME ON かつ ひらがな(ローマ字入力で日本語が打てる状態)のときは、必ず Engine ON。

- 現状: `compute_state` は 4 条件の AND(`01-map.md` §1)。ここで「IME ON」は `effective_open`、「ひらがな」は `input_mode.is_romaji_capable()`(`ObservedRomaji | AssumedRomaji`)。**`Unknown`・`ObservedKana` は「ひらがな」に含まれない**(論点 Q9)。BUG-223 は、IME が実際に ON でも `effective_open` が偽になる区間があることを示す。
- 出典: 所有者の指示。BUG-223(`origin/fix/focus-desired-fallback-off:docs/known-bugs/BUG-223.md`)、BUG-218・219・220・222。

**C-a2. 例外: 利用者が awase を OFF にしているとき**(`user_enabled` が偽。Ctrl+Shift+F12 など)。

- 現状: `UserDisabled`(順 1)。
- 設計が決めるべき近い例外(今回の指示には含まれていない。所有者に確認が要る): `config.app_overrides.disable_apps`(`app_disabled`、BUG-78。リモートデスクトップ等)、`FocusKind::NonText`(Win+X のメニュー等、experiments エントリ 02 の保護)、日本語でない IME(`NotJapaneseIme`)、かな入力方式(NICOLA は「ローマ字出力」前提なので `ObservedKana` は Engine OFF)。「ひらがな」の定義にどこまで含めるかの論点(Q9)。

## (b) 逆向き: Engine は OFF でなければならない場面

誤って ON だと、生の英字が NICOLA の仮名になる(出力が違う文字になり、利用者の入力が壊れる)。

| # | 条件 | 現状 | 出典 |
|---|---|---|---|
| C-b1 | IME OFF(直接入力)のとき Engine OFF | `ImeOff`。**ここが最も事故の多い境界**: 「ON のつもりで OFF のまま」(BUG-223・218・219)と「OFF のつもりで ON のまま」(BUG-110・128・149・151・172・186)の両方向がある | ledger 全体 |
| C-b2 | 半角英数トグル中(左 Shift 単独タップ。`ObservedEisu`/`toggle_held`)は Engine OFF | `NotRomajiInput`。トグルの出入りは窓切替・戻り待ち・lang-detect で複雑(BUG-186・193・220・222) | ADR-107/244/245、BUG-186・193・220・222 |
| C-b3 | キャッシュされた `ObservedEisu` は Engine を塞ぎ続けてはいけない | `cache_restore_eisu_guard` が復元時に `AssumedRomaji` へ。2026-07-09 MS Edge で、131 秒前の `ObservedEisu` が復元され Engine が inactive のまま固着 | `platform_state.rs:1737` のコメント、`eisu_recovery.rs` |
| C-b4 | IME-ON にする経路には `ObservedEisu` の救済を対で置く | 観測の無い Imm32Unavailable アプリで engine 永久 inactive の循環(2026-07-06 MS Edge で実発生) | `.claude/rules/ime-belief-architecture.md`、`tests/architecture_guard.rs::user_ime_on_paths_are_paired_with_eisu_reset` |
| C-b5 | IME OFF のときだけ無変換/変換を Space にする(ADR-255)の判定は、Engine 自身の belief で行う(Engine の判断と一貫) | `kp_stage_late_keymap` | ADR-255 |
| C-b6 | Engine を非活性にした瞬間、保留中のキーを必ず flush する | `check_active_transition`・`to_context_change` | `engine.rs` |

## (c) 観測の信頼性

| # | 条件 | 出典・測定 |
|---|---|---|
| C-c1 | **読めない窓**(GJI の tsf-native・imm32-unavailable)では、IME の開閉を直接読む手段が無い。belief は予測と意図でしか動かない。ObserverPoll は TsfNative では走らない(`reschedule_ime_refresh` :1142)。 | BUG-156・172・205、ADR-205・279 段階 0(ずれの間 `derive_any=None`、観測 0 件) |
| C-c2 | **明示意図があると poll が止まる**(読める窓でも)。意図が残る限り観測が来ず、意図が固定される。 | `runtime/mod.rs:1194,1217`、ADR-279 段階 0(S0-1: ずれは 3 秒以上 160/160、`HighSingle` 0%) |
| C-c3 | **観測が嘘をつく**窓がある: GJI の tsf-native/imm32-unavailable 窓の IMM 読み(単独の `ObserverPoll`/`ImmCrossProbe` が偽の OFF)、Mozc Readable の IMM(BUG-219: `[direct-follow]` が書いた偽の「閉」が TTL 切れの `MozcShm` に勝った) | BUG-219・223(a)、ADR-268、ADR-279。LINE の IMM については今回根拠となる文書を確認できなかった(メモリ記録の言及のみ。未確認) |
| C-c4 | **遅れて届く観測**が、直前の正しい予測・書き込みを押す前の状態へ戻してはいけない。fence: 予測と同期的な観測 170ms、字形 1300ms(R7 の実測最大 0.885 秒)、通過マーク 300ms、フォーカス epoch+hwnd の同一性(ADR-106) | `ime_model.rs:812-822`(`KEY_EFFECT_SETTLE_MS`)、`:935`(字形の柵)、ADR-273 D4、ADR-106、ADR-187 |
| C-c5 | 単独の Medium(ObserverPoll)が、新しい明示意図・予測・High を覆してはいけない。逆に新しい Medium が古い High に負けてもいけない(BUG-189) | ADR-233、BUG-189・219 |
| C-c6 | 推測(`HeuristicDefault`・conv 推測・字形)は Medium 以上の本物の観測に負け、**actuation の根拠にしない**。belief は誤りが可逆だから緩く、書き込みの根拠は厳しく(ADR-087 P11) | ADR-087・089、BUG-63・173 追補3 |
| C-c7 | 窓が変わったら、旧窓の観測を新窓で有効と見なさない(fence、全消し) | `clear_on_focus_change`、ADR-106 |
| C-c8 | IME が閉の間は入力モードを観測しない(BUG-57・ADR-239。`mode_state.rs:41` のコメント)ので、IME が閉で始まると開くまで `input_mode` は `Startup` の仮定値 | `mode_state.rs:41` |

## (d) 繰り返してはいけない取り下げ(`02-ledger.md` §C の要約)

1. 「awase が観測に合わせて IME へ書き直す」型の補正(force-on・reassert・ActivationSync・focus 時の強制 OFF・drift 補正の拡張)。ADR-212 P6 の方針は「ユーザーが押した/設定したキーへの直接の応答だけを書く」。ADR-279 も却下案に挙げている。
2. 観測が無いことを根拠に意図を偽装する(`Recovery`/`HwndCache` の `UserIntentSource`、観測の偽装 `InputModeObserved{ImmGetOpenStatus}`)。
3. 「この窓種には日本語 IME が無い」という推測を観測として書く(experiments エントリ 02)。
4. 信頼度優先で古い High を残す(BUG-189)。
5. 時間だけで「OFF の後の復元」を抑制する(ADR-165)。窓の同一性(hwnd)と OFF の新しさ(同プロセス)で見る。
6. キー押下を根拠にした半角英数トグルの手放し(`024ca336`)。
7. IME OFF に送るキーを動かす(experiments エントリ 01)。
8. 実測なしの暫定定数を増やす(BUG-110 追補8、`tuning-constants.md`)。`EXPLICIT_*_INTENT_TTL_MS` は「未実測・暫定値」と書かれている。
9. 「ON へ戻す」方向の窓切替の修正は、BUG-128・218(古い ON の復元が本物の OFF を上書き)を再発させうる(BUG-223 の注意)。

## (e) 制約(設計の形に関わるもの)

| # | 制約 | 出典 |
|---|---|---|
| C-e1 | belief の書き込み点は `ImeModel::reduce` だけ。`desired_open`・`input_mode` は private。観測は `classify_*` 純粋関数を経由する | `.claude/rules/ime-belief-architecture.md`、`tests/architecture_guard.rs`、dylint `ime_event_guard`・`observation_source_guard` |
| C-e2 | core(`awase` ルート crate)は OS 非依存。エンジンは生の VK を分岐に使わない | ADR-019、`CLAUDE.md` |
| C-e3 | fix は(a)回帰テストか(b)`docs/known-bugs/BUG-NNN.md`。再発ファミリー(warmup・focus・belief・conv・キー選択・半角英数トグル・actuation 合流点)が対象 | `.claude/rules/fix-requires-evidence.md` |
| C-e4 | 定数の変更には実測(ms)を添える | `.claude/rules/tuning-constants.md` |
| C-e5 | 同じ押下の二重書き込みを防ぐ押下 ID(`PressLedger`)。order を起案する入口は 3 点(claim・`with_press`・`explicit_press_applied_pair`)をそろえる | ADR-208、`architecture_guard::press_id_is_claimed_and_carried_at_every_order_issuing_entry` |
| C-e6 | journal は `ImeEvent` を直列化する(ADR-082)。列挙値を消すと古い journal・不具合報告の再生が壊れうる | `raw-c-belief.md` MC-03 |
