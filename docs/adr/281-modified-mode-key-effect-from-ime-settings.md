---
id: ADR-281
title: |-
  修飾キー付きのモードキーの効果は、IME の設定(config1.db・学習表)に根拠がある範囲だけ予測し、根拠が無いときは予測せず、後から組成の開始という証拠で追随する
summary: |-
  BUG-221(Ctrl を押したままのひらがなキー〈F2〉で実 IME は ON になるのに、予測が修飾付きを全部止めるため belief は OFF のまま残る)を受けた方針。
  「修飾付きは素のキーと同じ」と決めて予測を緩める案(案 A)は、同じキーの効果が IME の設定で変わるので採らない。
  ADR-199 の原則(キーの役割は設定から逆算する・推測で値を決めない)を修飾付きの行へ広げる。
  提案段階。Opus レビュー待ち。
status: |-
  提案(2026-10-26)。Opus 敵対的レビュー待ち。実装は未着手。
related_adr:
  - "ADR-199"
  - "ADR-195"
  - "ADR-196"
  - "ADR-186"
  - "ADR-263"
---

# ADR-281: 修飾キー付きのモードキーの効果を、IME の設定に根拠がある範囲だけ予測する

## 背景(BUG-221)

dragonflyg4(Windows Terminal×GJI、TSF ネイティブの窓)で、Ctrl+無変換で IME を閉じた直後、Ctrl を離す前にひらがなキー(F2)を押した
(2026-10-26 02:36:30 UTC、`awase.log`)。実 IME は ON になった(直後の `E` で候補窓が出た)が、awase の belief は OFF のまま Engine も OFF で、
`E`(た)は NICOLA の変換を通らず素のキーとして IME に入った。約 0.5 秒後に変換キーの予測が効いて初めて追随した。

- 予測が止まった理由: `key_effect_predictor.rs::modifiers_suppress_prediction` が、表にあるモードキーでも Ctrl/Alt/Shift/Win 押下中は予測と追跡を止める。
- 読み直しで直らない理由: TSF ネイティブの窓は `read_ime_state_full` が `ime_on=None`。
- 再現: `closed_loop_scenarios.rs::bug220_*`(PR #849、修正前は `belief=false 真値=true` で赤)。

## 検討して採らなかった案 A: 修飾付きでもモードキーの予測を止めない

「Ctrl+F2 は素の F2 と同じ効果」と決める案。採らない理由:

1. 同じキーの修飾付きの効果は IME の設定で決まる。ATOK プリセットでは F2 自体が開かない(`tools/e2e/ime_key_matrix/grid-tables/atok.json` の `off-*|hiragana` は `OFF`)。
   Shift+変換は ATOK で開閉トグルではない(ADR-186)。今回の実機で確かめたのは、この利用者の GJI で ON になったという 1 例だけ。
2. 誤予測は書き戻しにつながる(BUG-162: 予測が Engine を誤って活性化し、warrant が下りて IME を書き戻した)。
3. 今回の ON が「Ctrl+F2 という割り当て」によるのか、「Ctrl が離れた後に F2 が処理された」競合によるのか、ログからは区別できない
   (F2 の KeyDown と Ctrl の KeyUp の間隔は約 9ms)。

## 現状の把握(2026-10-26、コード調査)

- GJI の `custom_keymap_table`(TSV `状態\tキー\tコマンド`)は修飾付きの行を持てる(`Precomposition\tCtrl Shift Insert\tIMEOn` など、`keymap.rs` のテスト)。
  しかし読む側はすべて修飾付きを捨てる: `keymap.rs` は stage 1 で対象外、`role.rs` は無修飾の完全一致だけ、不具合報告の診断は
  `modified_key_ime_rows`(「belief に効くのに送らない」と書かれた数だけの欄)に数える。
- MS-IME 本体の旧 UI の表も、`Ctrl+` 付きの行ラベルを完全一致で除外している(`key_effect_predictor.rs`)。
- プリセット(ATOK/MS-IME など)の表の本体は IME の内部にあり、ディスクから読めない。awase は無修飾のキーを CI の格子学習(`atok.json` など)で測っている。
  学習の型 `KeyId` は「修飾付きのキーも別 ID で表す」と書かれているが、修飾付きの ID を割り当てて測った実績は見つかっていない。
- Mozc 系も同じく `custom_keymap_table` を読む経路を共有する(ADR-199)。MS-IME 本体はレジストリの割り当てのみ。

## 決定(提案)

**D1. 既定は今のまま、修飾付きの打鍵は予測しない。** 根拠の無い修飾付きの効果を推測しない(ADR-199 の原則)。

**D2. 根拠がある範囲だけ予測する。** 根拠は次の 2 種類に限る。
- D2a 設定: `session_keymap` が CUSTOM のとき、`custom_keymap_table` に(状態, 修飾+キー)の明示行があり、そのコマンドが開閉・モードに関するもの
  (`IMEOn`/`IMEOff`/トグル/`CompositionMode*`/`InputMode*`)なら、その行の効果を予測に使う。プリセット選択時は `custom_keymap_table` を使わない(ADR-186 決定(c))。
- D2b 学習: プリセット(ATOK/MS-IME 等)は修飾付きの ID を割り当てて格子学習で測り、測れたセルだけ予測に使う。測る対象は、無修飾で状態を変えるモードキー
  (F1〜F10、ひらがな、カタカナ、無変換、変換、半角/全角、英数、かな) × (Ctrl, Alt, Shift)に限る。費用の見積もりはレビューで確認する。
  MS-IME 本体のレジストリ・Mozc は D2a/D2b の同等物を別途検討する。

**D3. 根拠が無いときは、予測せずに「組成が始まった」という証拠で追随する。**
今回のログでは `StartComposition while engine off — ignored` が出ていた。組成は IME が閉じていては始まらないので、これは IME が開いていることの直接の証拠である。
Engine OFF のときに TSF 側から組成の開始を観測したら、belief を ON に直す経路を足す(読めない窓でも効く)。
ただし最初の 1 打鍵は間に合わず、素のキーとして入る。この限界は残す(最初の 1 打鍵の保持〈ADR-263 の窓切替直後の保持と同種〉は別の判断とし、本 ADR では採らない)。

**D4. 検証。** 擬似 IME の格子に修飾付きのセルを足し(D2b の測定結果由来のものだけ)、BUG-221 の再現テストを D3 で緑にする。
D2 の予測は、実 IME の挙動と一致することを CI 実機(`e2e-ime`)で確かめてから有効にする。

## 事前に測ること(実装の前)

- M1: dragonflyg4 の `config1.db` の `session_keymap` と、`custom_keymap_table` に `Ctrl F2` 等の修飾付き行があるか(読み取りのみ、キーマップ行だけを見る)。
  今回の ON の原因が「割り当て」か「競合」かの切り分けに使う。
- M2: Windows CI(Windows Terminal×GJI)で、Ctrl を押したまま F2 を押し、Ctrl を離す前後の IME 状態を測る(競合の有無)。
- M3: D2b の学習の測定コスト(セル数×試行数×1 セルの所要時間)。

## 未決(所有者に確認したい点)

- Q1. D3 の「組成の開始で belief を ON にする」を、どの確信度(High/Medium)の観測として `ImeModel::reduce` に入れるか。
  誤って ON にすると、実際は OFF の IME へ NICOLA が変換したローマ字を送る(リテラル出力)害がある。
- Q2. D2b を実装する価値があるか(M3 次第)。D1+D3 だけで十分とするか。
- Q3. 最初の 1 打鍵の保持を将来の判断にするか。

## 影響

- `ime-belief-architecture` の規約: belief への書き込みは Observe → 純粋な `classify_*` → `reduce()` の経路に限る。D3 の経路もこれに従う。
- `fix-requires-evidence`: IME belief・キー選択の再発ファミリー。BUG-221 の再現テストが (a) に当たる。
