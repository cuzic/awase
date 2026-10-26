# 04 偶発的な複雑さと必然的な複雑さ

既存の整理を引用して重複を避ける: `docs/tasks/mech-ecrs-2026-10-21/`(機構の ECRS。`raw-c-belief.md` が belief 領域)、`docs/tasks/layering-inventory-2026-10-06/`(層の棚卸し)、ADR-272(shell を薄くする)、ADR-275(`platform_state.rs`・`sync_actuation.rs` の core 移動は見送り)。
「偶発的」は「設計し直せば消せそう(消せると確かめたわけではない)」、「必然的」は「消すと過去の事故が戻る、または物理・OS の制約」。

## 偶発的な複雑さ(設計し直しで消せそうなもの)

| # | 内容 | 根拠(確認したもの) | 既存の整理との関係 |
|---|---|---|---|
| X1 | **明示意図の保管場所が 5 つ**(`desired_open`・`last_intent`・`intent_store`・`persistent_explicit_off`・`key_effect.open`)。寿命がそれぞれ違い(永続・窓切替で消える・10/30 秒・ON で消える・観測で消える)、窓切替・F2・窓の戻りの組み合わせで食い違う(BUG-223) | `01-map.md` §6 | `raw-c-belief.md` MC-07 が `last_intent` と `intent_store` の二重を C 候補に挙げた(ADR-087 INV-24(a) 級の判断で、擬似 IME の拡張が先)。今回は 5 つに広げて数えた |
| X2 | **`desired_open` の意味が混在**: (1)明示意図の値(`ExplicitIntent` の基底値)、(2)観測ゼロのときの `DesiredFallback`、(3)drift 補正の比較対象、(4)窓切替で持ち越される値、(5)初期値 true の placeholder、(6)授権 `warrant_context` の入力、(7)TSF キャッシュ復元の判断入力、(8)`decide_use`(共有メモリ)の入力 | `ime_model.rs:594`、`drift_correction.rs:131`、`focus_tracking.rs:635`、`platform_state.rs:1429,2148` | ADR-087 の P11(belief と actuation の根拠を分ける)に反して、`desired_open` が両方に入っている面がある(未精査。warrant が `desired_open` をどう使うかは `open_warrant.rs` を読んでいない) |
| X3 | **ON と OFF の意図の経路・寿命が非対称**(A1〜A9。キーの種類で `UserImeSetIntent` と `KeyEffectPredicted` に分かれる、TTL 10/30 秒、`persistent_explicit_off` が OFF だけで立つ、TsfNative 復元は ON だけ) | `01-map.md` §5 | 個々の非対称には ADR 上の理由がある(例: BUG-218・ADR-165)が、全体として「ON は予測、OFF は意図」になる効果は ADR に書かれていない(BUG-223 が初出) |
| X4 | **窓切替の後処理が窓の種類ごとに別の書き方**(TsfNative: `desired_open` を持ち越し・ON のキャッシュだけ復元 / その他: 双方向のキャッシュ復元 + `HeuristicDefault` の ON または OFF) | `01-map.md` §4.2、`focus_tracking.rs:613,690` | layering-inventory が `on_focus_process_changed` を F(判断と I/O の混在、307 行の 1 関数)に分類済み。判断の表は `plan_tsf_cache_restore` 等に一部抽出済み |
| X5 | **「外から変わったことへの追随」の窓が 4 つ**(通過マーク ADR-187、直接観測 ADR-188/244、外部変化 ADR-205、共有メモリ ADR-268)で、尾部(意図を捨てる → `pass_through_observed(true,true)`)が重複 | `mozc_shm.rs:386`(コメント)、`raw-c-belief.md` MC-08 | `raw-c-belief.md` MC-08(小さな C)。窓の機構そのものの統合は「?」(要判断) |
| X6 | **優先順位が 2 つの層に分かれる**: `ImeModel::resolve_open_at`(6 段 + ForceGuard)の上に、hub が `IntentStore` で丸ごと上書きする(`resolve_effective`)。ForceGuard(ON への強制)より IntentStore が上位。`resolve_open_at` の `decided_by` を読むだけでは真の根拠にならない(`belief_is_decided_by_glyph` が `IntentStore` の上書きを除くために作られた) | `platform_state.rs:2186,2208` | ADR-275 は `platform_state.rs` を core へ移さないと決定(殻に残す)。層の構造は変えない前提 |
| X7 | **Engine を通さない入口が `compute_state` の外に複数**(`app_disabled`・lang-detect の保持・半角英数の戻り待ちの保持・`FocusTransition` の settle)。「Engine が ON であるべきとき」の判定が複数の場所にある | `01-map.md` §1(今回は精査せず) | ADR-272 の「gate を外す」の対象と重なる可能性(未確認) |
| X8 | **半角英数トグルの状態が 2 か所**: Engine の ctx の `input_mode`(`ObservedEisu` 等)と hub の gate の `half_width_alnum`(`toggle_held`・戻り待ち)。入り・出の経路が、手動(左 Shift)・lang-detect・窓切替・F2・IME ON の 5 つ | BUG-186・193・220・222、`half_width_alnum.rs` | ADR-245・ADR-244 |
| X9 | 書き込み経路が無い観測の枠 4 つ(`Gji`・`Tsf`・`HwndCache`・`ImmGetOpenStatus`)と呼び出し元ゼロのモジュール(`key_effect_rules.rs`・`keymap_initial_hypothesis.rs`)。**実行時の挙動には無関係**(設計が複雑に見える原因にはなる) | `01-map.md` §3(`impl Observed<` の列挙で再確認) | `raw-c-belief.md` MC-01〜03(E 候補)。journal の列挙値は残す |
| X10 | 初期値 `desired_open=true`(placeholder)と `desired_is_placeholder` の特例、最初の観測へ 1 回だけ揃える `align_placeholder_desired` | `ime_model.rs:375`、`platform_state.rs:823`(BUG-163) | `desired_open` に「未設定」を持てば不要になる可能性(未検証) |
| X11 | 擬似 IME の写し(`harness.rs` が殻の数行を写している: `issue_actuation_order`・`kp_predict_key_effect`・`ir_apply_drift_correction` の途中まで)。殻が変わると写しが古くなる | `harness.rs` 冒頭 | ADR-277 段階 3 が drift の写しを本物へ置換した(実施済み)。残りの写しは未着手 |

## 必然的な複雑さ(消してはいけないもの)

| # | 内容 | 消すと何が起きるか(出典) |
|---|---|---|
| N1 | **belief(誤りが可逆)と actuation の授権(誤りが不可逆)の分離** | BUG-63(belief の Medium 単一合意が書き込みの根拠になり「mise」→「くした」)。ADR-087 P11 |
| N2 | **観測の信頼度・プール・鮮度・fence**。観測は遅れる(170ms・1300ms・300ms)・嘘をつく(IMM の単独の偽 OFF)・古くなる(3 秒) | BUG-189・219・223(a)、ADR-233・273・268 |
| N3 | **窓の同一性**(epoch + hwnd)と、窓切替で旧窓の観測・意図・予測を捨てること | BUG-128・218、ADR-106・165 |
| N4 | **読めない窓が存在する**(GJI の tsf-native・imm32-unavailable)。そこでは予測と意図だけで belief を動かす経路が要る | BUG-156・172、ADR-191・205 |
| N5 | **IME の種類・キーマップごとのキー効果の表**(GJI・MS-IME 本体・ATOK プリセット・Mozc が修飾を捨てる)と、その学習・検証 | ADR-191・195・256・281、BUG-205・211・212・221 |
| N6 | **押下 ID**(同じ押下の二重書き込みの防止、applied の未知化) | BUG-113・141 型、ADR-208 |
| N7 | **半角英数トグルの窓切替での復元は、離脱時に書けない**(旧窓へ届かない物理制約)ので、戻ってきた窓の最初の打鍵の手前で復元する | BUG-193、ADR-245 |
| N8 | **ユーザーの明示操作が観測より強い期間がある**(OFF を押した直後に古い観測が来る)。ただし「どれだけ長く強いか」は必然ではない(論点 Q3) | BUG-110・ADR-187 |
| N9 | **Engine 活性の遷移で保留キーを flush する**(文脈の喪失) | `engine.rs` `check_active_transition` |
| N10 | **belief の書き込み点を 1 か所に絞る規律**(`reduce` だけ。型・dylint・guard) | `.claude/rules/ime-belief-architecture.md`(偽装の近道が何度も書かれた履歴) |

## 判断が分かれるもの(必然とも偶発とも言い切れない。結論は出さない)

- `IntentStore` の存在(窓ごと・TTL つき)。`platform_state.rs:1737` 付近のコメント(BUG-51 追補 v3)によれば、フォーカス滞在 100ms 未満だと退場時のキャッシュ保存がスキップされ、新しい明示意図より古いキャッシュが勝つ事故を避けるために、窓ごとの意図を持たせた。X1 の統合と N8 の両方に関わる。
- poll を止める判断(明示意図があるとき)。止める理由(`runtime/mod.rs:1127` のコメント: ポーリングで得られる情報が無い・意図が確定している)と、止めた結果(ADR-279 段階 0: 観測 0 件でずれが 3 秒以上続く)の両方が記録されている。
- drift correction を残すこと(ADR-212 P6 は「明示 OFF の再試行」だけ残す)。
