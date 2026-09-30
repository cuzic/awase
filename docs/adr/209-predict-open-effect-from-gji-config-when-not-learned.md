---
id: ADR-209
title: |-
  GJI で学習表が無いとき、config1.db(session_keymap＋custom_keymap_table)の状態別の効果から開閉軸の打鍵時予測を作り、素通しされたモードキー(変換単独など)に Engine を追随させる
summary: |-
  実機(dragonflyg4、JIS、GJI `session_keymap=2`＋古い `custom_keymap_table` 191行)で、IME OFF で変換を単独タップすると GJI 自身が IME を ON にする(awase 停止でも 3/3)が、
  awase の打鍵時予測は「古い表が変換の行を持つので予測しない」(`key_effect_predictor.rs::custom_table_overrides`)ため belief が更新されず、Engine が OFF のまま(`か`)。
  学習表(`keymap-learn-table.json`、ADR-195/196)はこの実機では作れていない(BUG-177 修正後も BUG-178 で完走しない)。所有者決定(2026-09-30): v2 は設定の読み取りだけの推測を入れ、この実機の古い表は実効とみなす。
status: |-
  草稿(2026-09-30)。未レビュー。Opus 敵対レビューで収束させてから実装する。
related_adr:
  - "ADR-191"
  - "ADR-195"
  - "ADR-196"
  - "ADR-198"
  - "ADR-199"
  - "ADR-206"
---

# ADR-209: 学習表が無いとき、config1.db から開閉軸の打鍵時予測を作る(草稿)

## 背景(実機の事実、2026-09-30)

環境: dragonflyg4(JIS キーボード、Windows 11)、GJI、Windows Terminal(TsfNative、開閉を読めない)。awase は既定の config(`muhenkan/henkan_solo_tap_always_suppress=true`)。
- `config1.db`(protobuf を解析): `session_keymap = 2`(MS-IME プリセット)、`custom_keymap_table` は 191 行残っている(DirectInput の Henkan=IMEOn を含む。GJI のキー設定ダイアログで CUSTOM にしていた時代の表とみられる)。
- **awase を止めた GJI 単体で、IME OFF(直接入力)から変換を単独タップすると毎回 IME が ON になる(3/3)。** つまり、プリセット2でもこの古い表の効果が出ている(または MS-IME プリセットが同じ効果を持つ)。
- awase 稼働中(Suppress 既定、Engine 非活性)は、変換の生キーを素通し(`[reinject] vk=0x1c`)する。GJI は ON になるが、awase の belief は OFF のままで Engine も OFF になる(`ka`→`か`)。
  打鍵時予測のログは `[key-effect-predict] vk=0x1C open=false composing=false: no prediction`。理由は、予測器が「古い表がそのキーの行を持つなら予測しない(安全側)」(`custom_table_overrides`)としているため。
- 学習表 `keymap-learn-table.json`(ADR-195/196)は、この実機に無い。学習プロセスは、BUG-177(JIS 実機で自分の注入を物理入力と誤判定、修正済み #390)の後も、BUG-178(cell=73/84 で22分進まない)で完走しない。
- TsfNative の窓では、開閉を観測する手段が実質無い(`read_ime_state_*` は `None`、`ConvOpenInference` は IME を閉じても NATIVE が残るので開閉を区別できない)。**読めない窓での追随手段は打鍵時予測(`KeyEffectPredicted`)だけ**。

所有者の設計思想(2026-09-30): 「学習すると IME ON の効果を持つキーが学習されて追随する。ただし GJI なら、学習しなくても config1.db の読み取りだけで推測してほしい」。決定: **v2 に入れる。この実機の古い表は実効とみなす。**

## 矛盾する既存の証拠(レビューで必ず突くこと)

- ADR-199(`awase-gji-config/src/role.rs::source`)は「`session_keymap` が CUSTOM 以外ならプリセットで動き、`custom_keymap_table` は GJI が読まない」を前提とし、テスト `realdev_msime_preset_with_stale_custom_table`(`key_effect_predictor.rs`)も「実機(ADR-191 実機検証、MS-IME プリセット＋custom 表175行): 変換は直接入力から何もしない」と書く。**今日の観測(変換で ON)は、これと食い違う。** 食い違いの原因(GJI のバージョン、表の内容が175→191行に変わった、プリセット2の中身が Mozc の TSV と違う等)は未確定。
- ADR-186 撤去実験 E2: **ATOK プリセット**では、GJI は古い `custom_keymap_table`(変換=IMEOn)を読まない(awase が読むと変換が On と誤分類される、CI で確認)。

## 決定

1. **開閉軸の予測を config1.db から作る**: 学習表が無い(または該当セルが無い)GJI で、無修飾の物理キーの打鍵について、(現在の状態 × キー)に対する開閉の効果(開く/閉じる/変えない/不明)を、`custom_keymap_table` の行から求め、`KeyEffectPredicted` として belief に反映する。awase は IME へ書かない(ADR-191 決定1)。
2. **予測の優先順位**: 採用済みの学習表(ADR-196)＞ config1.db 由来 ＞ 同梱表(プリセット)＞ 予測しない。学習表にセルが無ければ次へ落ちる。
3. **どの表を実効とみなすか(所有者決定)**: `session_keymap` が CUSTOM のときは `custom_keymap_table`。**MS-IME プリセット(2)で `custom_keymap_table` が空でないときは、表がそのキーの行を持つ限り、その行を実効とみなす**(この実機の観測に従う)。ATOK・KOTOERI・MOBILE は従来どおり(古い表は読まれない、ADR-186 E2)。表がそのキーの行を持たなければ、同梱のプリセット表に落ちる。
4. **状態の写像**(awase の belief → Mozc の状態): 閉(DirectInput)=表の DirectInput 行。開は「全ての開状態」を Precomposition/Composition/Conversion(継承を含む、`role.rs` の `KeyStates`)で評価する。
   - 閉状態: `IMEOn` または `CompositionMode*` → 開く。行なし → 変わらない。`Reconvert` 等の他コマンド → 予測しない。
   - 開状態: 全ての開状態で閉じる(`IMEOff`)→ 閉じる。どの開状態にも閉じる行が無い → 変わらない。一部の状態だけ閉じる → **予測しない**(読めない窓では入力中/変換中の段階の追跡が当てにならない)。
5. **対象外**: 修飾付きのキー、注入されたキー、overlay(`HENKAN_MUHENKAN_TO_IME_ON_OFF` 等)が触るキー、未知の `session_keymap`、MS-IME 本体・ATOK 等の未同定 IME。`ImeKind` が GJI のときだけ。
6. **開閉軸だけ**: 入力モード軸(ひらがな/カタカナ等)は予測しない(既存の規則・観測に任せる)。
7. 新しいイベント・I/O・actuation の合流点・tuning 定数は作らない。fence は既存の `KEY_EFFECT_SETTLE_MS` を使う。

## 非目的

素通し後に awase が IME へ書くこと(ADR-191 決定1)。ADR-206 決定1(α)(Suppress × エンジン非活性では生キーが IME に届く)の変更。TsfNative の新しい観測源(TSF の OPENCLOSE compartment 等)。学習プロセスの修正(BUG-178)。PR #360(ConvOpenInference の drift 撤去)は別件で保留。

## 代替案と却下理由

- **B: 学習を回すよう案内するだけ**: 学習していない・config1.db が変わった直後(指紋が stale)・カバレッジ80%未満では CUSTOM ユーザーが無防備。所有者の「学習しなくても」に反する。この実機では学習が完走しない(BUG-178)。
- **C: 素通し後に観測して追随**: TsfNative には信頼できる観測源が無い。新 I/O が要り、ADR-205 の非目的と衝突。
- **D: プリセット2では古い表を読まない(現状維持)**: 今日の実機観測(変換単独で ON)に反し、Engine が追随しない。

## リスク(レビューで詰める)

1. **偽 ON**: 表が実効でない環境(プリセット2が古い表を本当に読まない環境)で、awase が「開く」と予測し、実際は開かない。Engine だけが ON になり、`ka` が NICOLA として出る(BUG-176 型の偽追随)。緩和: 学習表が優先、ユーザーの ON/OFF キーで立て直せる、ただし自動では検出できない。
2. **予測の後の drift**: 予測は `desired_open` を書かない。後で GJI の I/O 推測(`ObserverPoll`、Medium)が記録されると、明示意図が無くても `desired=false` との乖離で drift 補正(`VK_IME_OFF`)が走りうる構造か。`KeyEffectPredicted` が `last_intent`/IntentStore を消すので止まるはず(コード上の帰結、要テスト)。
3. **Engine 活性化時の `SetOpen(true, ActivationSync)`** が warrant で拒否されるか `VK_IME_ON` を1回送るか。IME は既に ON なので BUG-113「@」(半角で生の無変換/変換)の構成とは違うが要確認。
4. **入力モード軸**: TsfNative では `AssumedRomaji` のままなので Engine は活性化するはずだが要確認。

## 検証方針

- 純関数の単体テスト(`role.rs`: 表に応じた開閉効果、閉=DirectInput の IMEOn、一部の状態だけ閉じるケース、継承、overlay、未知値)。
- `key_effect_predictor.rs`: 優先順位(学習表＞config1.db＞同梱表)と、プリセット2＋古い表の実機由来の表(191 行から起こした代表行)。既存テスト `realdev_msime_preset_with_stale_custom_table` は、期待値を「変換は予測する(開く)」に更新し、コメントに今日の観測と食い違いを書く。
- `closed_loop_scenarios.rs`(Linux で走る): 「明示 OFF の後、変換を素通しすると belief が ON になり Engine が追随し、drift が発火しない」。
- `architecture_guard`: `KeyEffectPredicted` の dispatch 元が1箇所のまま。
- 実機 A/B(dragonflyg4): 学習表なしで「IME OFF → 変換 → `ka`」が `きう`(NICOLA)になること。プリセット2＋古い表を持たない構成(表なし)で、変換を素通ししても Engine が動かないこと(偽 ON がないこと)。

## 未検証事項

- プリセット2で GJI が古い表を本当に読むのか(今日の1台の観測だけ。ADR-191 実機検証の「何もしない」との食い違いの原因)。
- 予測後に ObserverPoll 由来の drift が走らないか。`ActivationSync` の `SetOpen(true)` の挙動。DirectInput の `Reconvert` 等で実際に開くか。入力モードが romaji 扱いのままか。
