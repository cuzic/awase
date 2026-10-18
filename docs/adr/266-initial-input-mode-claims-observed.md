---
id: ADR-266
title: |-
  belief の入力モードの初期値 ObservedRomaji が「観測済み」と名乗る問題(BUG-204)。実害の記録が出るまで挙動は変えず、機序と選択肢を記録する
summary: |-
  閉じた IME を開くとき、belief の入力モードが観測されないまま初期値の `ObservedRomaji` だと、IME が半角英数を覚えていると食い違う(BUG-204。閉ループのランダム探索で 2878 打鍵中 32 件)。
  **機序の訂正**: 最初の起草は「`Unknown` のとき最初の打鍵で既定のかなの種が入る」としたが誤り。belief の入力モードは `ImeModel::new()` の `ObservedRomaji` で始まり、`Unknown` を流す経路は本番にも
  ハーネスにも無い(`predict_in_table` の `seeded` の経路は本番では到達しない)。閉の間は入力モードを観測せず(BUG-57 / ADR-239)、閉のキーは `mode: None`(変えない)なので、初期値が開くまで残る。
  素朴な直し方(種を書かない、閉→開で `Unknown`)は、`Unknown` が `is_romaji_capable` で偽のため Engine が止まって逆効果になりうる(本番では `Unknown` にならないので「効かない」のが実際)。
  Engine を有効にしているのは初期値の `ObservedRomaji` と `correction_for_imm_broken`・`eisu_recovery` の救済で、種ではない。
  本 ADR は挙動を変えず、機序と選択肢(初期値を観測済みと名乗らない値にする、死んだ `seeded` の経路を消す、フォーカス移動の経路を測る)を記録する。決定は実害の報告が出るまで保留し、所有者判断を仰ぐ。
status: |-
  起草(2026-10-18)。実装なし。Opus レビュー round 1(Blocker 2・Major 4・minor 4)を反映して書き直し。決定は保留(所有者判断 Q1)。
related_adr:
  - "ADR-191"
  - "ADR-239"
  - "ADR-261"
  - "ADR-264"
---

# ADR-266: belief の入力モードの初期値が「観測済み」と名乗る問題

## 訂正の経緯(先に)

最初の起草は、BUG-204 の機序を「belief の入力モードが `Unknown` の間の最初の打鍵で、既定のかな(`AssumedRomaji{KeyEffectPrediction}`)の種が入り、閉→開で残る」とし、
その種に理由 `DefaultSeed` を付ける案を採った。**この機序は誤りだった**。CI の実測は『`Unknown` の種 0 件』(閉ループのテストで、その時点の `input_mode` が `Unknown` のものを数えた結果)で、
`Unknown` が一度も流れないことを示していたが、私はこれを「種は開く打鍵より前の打鍵で入る」と読み替え、コードを読んだ推測を「実測では」と書いた。
`seeded` は `input.mode == Unknown` のときだけ真なので、「`Unknown` が 0 件」と「種が入る」は両立しない。Opus レビュー(round 1)が、初期値を読んで指摘した。
(コードを読まずに機序を取り違えたのは、BUG-205 の「セルが無い」、BUG-204 の「閉じる前のモードを覚える」に続く 3 回目。)

## 背景

[BUG-204](../known-bugs/BUG-204.md): ATOK で IME が閉・半角英数(conv 0x10)で始まると、開くまで belief の入力モードは初期値のまま。開く打鍵は `mode: None`(変えない)で、初期値が残り、実 IME は 0x10 で開く。
閉ループのランダム探索(PR #634)で 2878 打鍵中 32 件(閉・半角英数で始まる 16 シード × 窓 2 通り)。

## 機序(コードで確認、Opus レビュー)

1. `ImeModel::new()` の `input_mode` の初期値は `InputModeState::ObservedRomaji`(`ime_model.rs:325`)。`ImeStateHub` は `ImeModel::default()` で始まる。「観測していないのに『観測済み』と名乗る」既定値。
2. `input_mode` への書き込みは reduce の 3 箇所(`InputModeObserved`・`InputModeApplied`・`KeyEffectPredicted`)だけで、`Unknown` を流す経路は本番にも無い(本番コードの `InputModeState::Unknown` は、`DecisionInputs` の仮の値と予測器の内部の判定だけ)。
   よって `predict_in_table` の `seeded`(`input.mode == Unknown`)の経路は本番では到達しない。単体テスト `unknown_mode_is_seeded_with_kana_so_prediction_can_start` だけがこの経路を通る。
3. 閉の間は入力モードを観測しない(BUG-57 / ADR-239: 閉じた IME の conv=0 を英数と誤読する。ハーネスの `observe_value` も開いているときだけ `InputModeObserved` を流す)。
4. ATOK の表の閉のセルは全て `after_conv=None` で、`mode: None`(変えない)になる。閉→開の打鍵も同じ。よって閉で始まると、開くまで、開いた後も、初期値が残る。
5. 閉→開の規則は決定的(`grid-tables`: ATOK 10/10、GJI の MS-IME プリセット 18/18 で閉じる前の変換モードを復元。MS-IME 本体は 閉 c10 から開くと 0x19)。試行は各セル 2 回以下(MS-IME 本体は 227 セル中 206 が 1 回)。
   非決定ではなく、閉の間のモードを awase が読めないことが問題。

## `Unknown` と Engine の有効化(事実の整理)

- `InputModeState::is_romaji_capable()` は `ObservedRomaji | AssumedRomaji{..}` だけを真にし(`src/engine/mode_state.rs:48`)、`Engine::compute_state` は偽なら `Inactive(NotRomajiInput)` にする(`engine.rs:279-281`)。
  `ObservedKana` も偽(JIS かな入力の人にローマ字を送らないための意図した扱い)。`Unknown` を別の値に読み替える経路は無い。
- 読めない窓で Engine を有効にしているのは、(1) `ImeModel::new()` の初期値 `ObservedRomaji`、(2) `correction_for_imm_broken`(`AssumedRomaji{ImmBridgeBroken}`)と `eisu_recovery` の救済。**種ではない**。
- 本番では `Unknown` にならないので、「種を書かない(`Unknown` のまま)」案は本番では何も変えず、「閉→開で `Unknown` を返す」案は「belief が初期値か」を判定できない(初期値が `ObservedRomaji` と名乗っている)。
  仮に `Unknown` を流せば Engine は止まる。

## 選択肢

| 案 | 内容 | 判断 |
|---|---|---|
| 0 | **何も変えず、機序を文書に直す**(BUG-204・ADR・テストのコメント。このコミットで実施) | **今回やる**。実害の記録が 0 件のため |
| 1 | 初期値を `AssumedRomaji{reason: Startup}`(名前は任意)にする | 保留(所有者判断)。挙動は変わらない見込み(`is_romaji_capable` は同じ扱い。本番で `ObservedRomaji` と `AssumedRomaji` を区別する分岐は無いとレビューが読んだ)。費用は 1 行と、初期値 `ObservedRomaji` を assert している単体テスト数件、journal・ログの文字列。**閉のキーは `mode: None` なので食い違いは減らない**。`desired_is_placeholder`(BUG-163)と同じ考え方で、名前が嘘をつかないことが利点(今回の誤診のように、読む人を誤らせる) |
| 2 | 死んだ `seeded` の経路(6 箇所と単体テスト)を消す、または「本番では到達しない」と doc に書く | 保留。案 1 と同時に決める |
| 3 | 開く打鍵の直後に 1 回観測を促す、idle-conv-check の対象に含める | **却下**。読める窓は既に観測が直し、読めない窓は観測できない。idle-conv-check は TsfNative 限定 |
| 4 | 候補の集合(3 値・4 値)にする | **却下**。観測できない値を集合にしても Engine は 1 つに決めるしかない。かつ `mode` を `Some(何か)` で表すと `reconcile_key_effect_mode` の fence が張られ、窓の中の正しい読みを捨てる(下の「`mode` の `None`」) |
| 5 | 閉の間のモードの証拠源(ADR-261 字形、ADR-264 メニュー)で直す | 各 ADR の担当。BUG-204 の食い違いが実際に減るのはここ |
| 6 | 閉のモード(`conv_raw`)から初期値を取る | **却下**。BUG-57 / ADR-239。MS-IME 本体は 閉 c10 → 開 0x19 で外れる |

## `PredictedEffect.mode` の `None` が「変えない」と「不明」を兼ねる件

型は今は分けない(結論は最初の起草と同じ)。ただし理由は「Engine の有効化が唯一の消費側」ではなく、`None`/`Some` を区別する消費側が複数あるため。
`None` は fence を張らず観測に任せる安全側で、`Some(何か)` で「不明」を表すと次が起きる: (1) `KeyEffectPredicted` が belief を書き、fence(`key_effect`)の mode 軸が立つ(`ime_model.rs:897-913`)、
(2) `reconcile_key_effect_mode` が `KEY_EFFECT_SETTLE_MS` 以内の観測を捨て、`[key-effect-miss] axis=mode` を出す(`:718-742`)、(3) `press_axes`(ADR-260 決定 2)が mode 軸を立て、直接観測の窓の読みを捨てる(`platform_state.rs:306-312`)。
窓の中の正しい読みまで捨てるので、「不明」は今の `None` のほうが安全。

## フォーカス移動で前の窓のモードが残る件(BUG-204 の関連。実機で確認: ATOK では実害なし)

`reduce_focus_changed` は `input_mode` を消さない。フォーカス移動の `input_mode` は主に次の経路で決まる:
- **hwnd キャッシュの復元**(`apply_hwnd_cache_restore`): `InputModeApplied{CacheRestore}` で `input_mode` を無条件に上書きする。`ObservedEisu` だけ `cache_restore_eisu_guard` で `AssumedRomaji` に倒す。キーは (pid, class_name)、最大 1 時間。ADR-191 §3.1 が誤りうると明記している hwnd 単位の記憶。
- **新しいスレッドを閉とみなす経路**(ADR-212 P2、`assume_closed_for_new_thread`): 開閉だけを Low の `HeuristicDefault` として記録し、`input_mode` に触れない。ADR-191 §3.2 の「閉×**ひらがな**」の半分しか反映していない。

**閉ループのハーネスでの再現**(`stale_eisu_blocks_engine_after_predicted_open_in_a_new_thread_window`、`#[ignore]`): 前の窓が `ObservedEisu` → 新スレッドの読めない窓 → 変換キーで開く(ATOK の閉→開は `mode: None`)→ `ObservedEisu` のまま Engine が `NotRomajiInput` で止まる。

**実機(本物の awase・実 Chrome)では救われた**(run 39113514316、`chrome_probe --key=1C --new-window-after=alnum`、ATOK、3/3 PASS):
変換キーを通した 20ms 後に `mode-key-follow` が IME の実状態を直接読み(`read_ime_state_full ... cross native=true roman=false`)、`[direct-follow] モードキー直後の窓の中で実状態が belief と違った → eisu=Some(false) へ追随` で
`ObservedEisu` を訂正し、`Engine activated` になった。ハーネスはこの段(`key_pipeline` の後段)を写さないので、閉ループの再現は偽の陽性。
ログのタイムライン(1 回目): 09:02:40.698 ひらがなキーで `mode=Some(ObservedEisu)` → 09:02:43.919 新しい Chrome へ(`new-thread entry`、`mode=ObservedEisu` のまま)→ 09:02:45.608 変換キー(`open=Some(true) mode=None`)→ 09:02:45.638 `[direct-follow]` → `Engine activated`。

**未測定**: GJI の MS-IME プリセット・MS-IME 本体(Chrome を半角英数にする前提が作れず INVALID 3/3)、変換キー以外のモード系キー(無変換・カタカナなど。`mode-key-follow` の対象かは未確認)、
モードキーでなく表の予測だけで開く経路(物理キーが Suppress される場合など)。前の窓が直接入力のとき(`off` の対照)、ATOK・プリセットは新しい窓で `その他` と FAIL 3/3 だった理由は未確認(IME が開かないか別の挙動)。
Opus レビューが指摘した「BUG-204 の食い違いより実害が大きい可能性」は、**少なくとも ATOK + 変換キー + Chrome では当たらなかった**。

## ADR-191 の引用の訂正

最初の起草は ADR-191(GJI の状態の範囲)の「決定 2・4」を「種の設計の根拠」としたが、不適切だった。
- ADR-191 の該当は §3「belief 設計への含意」の 1〜5(「決定」の節は無い)。§3.2 の「awase の既定の種(IME ON・ひらがな・ローマ字)」は `ImeModel::new()` の初期値(`desired_open=true`〈BUG-163 の `desired_is_placeholder`〉+ `ObservedRomaji`)を指し、`seeded` の経路ではない。しかも開閉の軸で「違う」と指摘している。
- §3.4 の「追跡が不明のときだけ既定のひらがな(0x09)を種にする」は、表を引く conv の選び方(`key_track.conv` が無いときの `Conv::C19`、`key_effect_predictor.rs:506`)で、belief に書かない。
- 支持できるのは「実測(新スレッドは閉×ひらがな、閉→開は GJI/ATOK が保持)がある」ことまで。

## 決定

1. **今回は挙動を変えない。** BUG-204・本 ADR・テストのコメントの機序を、初期値の `ObservedRomaji` に直す(実施済み)。閉ループのテストに、その時点の `input_mode == ObservedRomaji` を検査する `assert_eq!` を足し、機序の前提を実測で守る
   (CI で通れば本 ADR の機序が確定する。通らなければ、また誤っている)。
2. **案 1(初期値の改名)と案 2(`seeded` の整理)は、所有者判断(Q1)とし、保留する。** 実害の記録が 0 件の段階では新しい仕組みを足さない方針。ただし誤診が 3 回起きた事実は、名前が嘘をつく費用の記録として残す。
3. **フォーカス移動の経路は、実機で確認した結果、ATOK + 変換キー + Chrome では実害なし**(上記)。他の IME・キーは未測定。ハーネスに `mode-key-follow` を足すかは別途判断する。
4. **実害の測定(受動)。** 受け取った不具合報告の belief の入力モードの出所(不具合報告のスナップショットは `format!("{:?}", input_mode())` で理由も含む。journal は `variant_name` で理由が落ちる)を確認し、初期値の `ObservedRomaji` が後から観測や証拠源に覆された件数を数える。

## 未決・要確認

- Q1(所有者): 案 1・2 を今やるか、実害の報告が出るまで保留か。
- フォーカス移動の経路の再現と、`eisu_reset_on_ime_on` の対応表で救われる範囲。
- 案 1 を採るとき、journal・ログの文字列(`ObservedRomaji` を期待する外部の読み手)への影響。

## 影響とリスク

- 今回の変更は文書とテストのコメント・名前・検査のみ(挙動変更なし)。テストの `assert_eq!` は、機序の前提が崩れれば CI で失敗する(意図したもの)。
- 案 1 を採るときの網羅的な `match` の更新: `AssumedReason` を網羅的に `match` する箇所は無い(生成と `AssumedRomaji { .. }` のワイルドカードだけ)。唯一の列挙は `conv_classify.rs` の proptest `arb_assumed_reason`。
