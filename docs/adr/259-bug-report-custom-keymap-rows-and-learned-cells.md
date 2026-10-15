---
id: ADR-259
title: |-
  不具合報告にカスタム表の IME 関連行と学習表のセルを添付する
summary: |-
  報告 01M4QSFH1B(一太郎・GJI の CUSTOM)は、英数が「IME ON 兼 英数トグル」の複合キーかを報告から判定できず、belief の食い違いが設計の範囲かを切り分けられなかった。ADR-148 は表の要約(キーの分類)だけを送っていた。custom_keymap_table の IME 関連行を3列とも固定語彙・allowlist を通して送り、学習表のセルを数値だけの固定書式で送る。
status: |-
  起草(2026-10-14)。Opus レビュー round 1 の指摘を反映済み、round 2 の再確認待ち。実装なし。
related_adr:
  - "ADR-095"
  - "ADR-148"
  - "ADR-196"
  - "ADR-199"
  - "ADR-209"
---

# ADR-259: 不具合報告にカスタム表の IME 関連行と学習表のセルを添付する

## ステータス

起草(2026-10-14)。Opus 敵対的レビュー round 1 の指摘を反映し、同じレビュアーへの再確認(round 2)待ち。コードは変えていない。

## 背景

不具合報告 `01M4QSFH1B02DQ9B8AXBYVRQ39`(GJI・一太郎、2026-10-12。issue #623)の切り分けで、報告に載っている情報では次が判定できなかった。

- GJI は `session_keymap = 0`(CUSTOM)で、英数(`VK_DBE_ALPHANUMERIC`)が `ime_on_keys` と `mode_toggle_alphanumeric_keys` の両方に分類されていた。DirectInput の行が IMEOn で、開状態の行が `ToggleAlphanumericMode` という複合キーなのか、別の組み合わせなのかは、分類結果からは分からない。
- 報告の journal では、英数の効果予測が `open: null`、mode と conv だけを動かしていた。`PredictedEffect.open` の `None` は「予測なし」ではなく「開閉は変えない」である(`key_effect_predictor.rs:346-351`)。journal の範囲では、英数を押した時点の belief は開(conv が C19 から C10)で、これはトグルとして素直な予測であり、閉状態の英数の予測は journal に現れていない。
- 予測を出した分岐が、学習表のセルなのか、窓別の規則(ADR-209・211・247)なのかも、報告からは分からない(`predict_with_override` は学習表より先にこれらの分岐を引く)。
- 報告に載っているのは、キーの分類(`ime_on_keys` など)と、学習表の件数・判定・指紋・同梱表との差の一部だけ(ADR-148、ADR196-T2)。表そのものは載っていない。

実 GJI での確認(2026-10-14、GitHub Actions run 38658180992、`sc-eisu-composite-gji-custom`)では、DirectInput の英数=IMEOn、開状態の英数=ToggleAlphanumericMode の表で、閉状態の英数を押すと IME が開き(open=1・conv=0x19)、開状態では 0x19 と 0x10 を行き来した。この読める窓の構成では awase の Engine も実 IME に追随した。複合キーの挙動そのものは実機で確かめられたので、報告から要るのは「その環境の表が実際にそうなっているか」である。

「IME キーを押せば belief が直る」という設計に照らして、報告者の環境がその範囲内かを、実機の表を見ずに判断できない。この ADR は、その判断に要る材料を報告に足す。

## 決定

### 決定1: `custom_keymap_table` の IME 関連行を、型で閉じた固定語彙だけ通して添付する

`BugReportGjiKeymapSummary` に次を足す。どちらも `Option` / `#[serde(default)]`。`custom_keymap_table_is_effective` のときだけ `Some`。

- `custom_keymap_rows: Option<Vec<BugReportCustomKeymapRow>>`。1行は名前付きの構造体 `{ status, keys, command }`(読む側が誤読しにくいように、タプルにしない)。
- `custom_keymap_rows_dropped: Option<BugReportCustomKeymapDropped>`。送らなかった行の件数(決定1-4)。

**1-1. 型で閉じる。** 純粋関数 `awase_gji_config::keymap::diagnostic_rows` の戻り値は `Vec<DiagnosticRow>` で、`DiagnosticRow` の 3 フィールドはすべて `&'static str` / `&'static [&'static str]` とする。`config1.db` 由来の `String` が出力に入る余地が型に無く、ADR-148 の型 doc が言う「allowlist が唯一の防壁」を、レビューではなくコンパイラが守る。

**1-2. 語彙は全部列挙し、ワイルドカードを使わない。**

- 状態(6 語): `DirectInput`・`Precomposition`・`Composition`・`Conversion`・`Prediction`・`Suggestion`。
- コマンド(16 語): `IMEOn`・`IMEOff`・`CancelAndIMEOff`・`ToggleAlphanumericMode`・`SwitchKanaType`・`CompositionModeSwitchKanaType`・`CompositionModeHiragana`・`CompositionModeFullKatakana`・`CompositionModeHalfKatakana`・`CompositionModeFullAlphanumeric`・`CompositionModeHalfAlphanumeric`・`InputModeHiragana`・`InputModeFullKatakana`・`InputModeHalfKatakana`・`InputModeFullAlphanumeric`・`InputModeHalfAlphanumeric`。前方一致では判定しない。送る文字列は入力ではなく、この固定表の要素そのもの。
- `classify_command` だけで判定しない(`InputMode*` の行が黙って落ちる)。`Reconvert` など上の表に無いコマンドを足すかは、Mozc のコマンド一覧と照合して決める。推測で足さない。
- キー: `mozc_key_vk_names` が返す `&'static [&'static str]` をそのまま使う。修飾キー付き・文字キー・未知のキー名・`Kanji` は空になり、行は落ちる。

**1-3. 重複は除くが、矛盾は両方残す。** 別名(`Hankaku` と `Hankaku/Zenkaku`、`Kana` と `Hiragana`)や大文字小文字違いで正規化後に同じになる行は 1 行にまとめる。同じ (状態, キー) に違うコマンドが付いた行は、GJI がどちらを使うか分からない矛盾で、それ自体が診断に要るので両方残す。

**1-4. 落とした行は理由別の件数で示す。** 件数だけで、内容は送らない。見出し行(`status\tkey\tcommand`)は数えない。

- `unmapped_key`: コマンドは語彙に入るがキーが VK に写らない(`Shift Eisu`・`Ctrl Space` など、修飾キー付きの IME 行を含む)。
- `unknown_status`: 状態が 6 語に無い。
- `total_rows`: 表の総行数(語彙外のコマンドの行は普通の編集コマンドが大半なので、個別には数えず総数だけ)。

欠けた行が「無い」のか「送れなかった」のかを、件数で区別できるようにする。

### 決定2: 学習表のセルを、短い固定書式で添付する

`BugReportKeymapLearnSummary` に `cells: Vec<String>`(`#[serde(default)]`)を足す。`table_file == "loaded"` のときだけ。不採用・要確認で退避した表(`keymap-learn-last-attempt.json`)のセルは送らない。

- 書式は 1 セル `open,mode,composing,key>open,mode,composing,disp`(例: `0,0,0,F0>1,9,0,K`)。`disp` は `Disposition` の 4 値の固定の 1 文字。`prediction` の無いセルは `>-`。`mode` は 16 進、`key` は VK の 16 進。凡例は ADR に書き、報告を読む側(`bug-report-fetch` スキル)にも書く。
- (status, key) の順に並べる(報告どうしの diff を取りやすくする)。
- 件数は 448 件を上限にする(学習キー 14 個 × 状態 32)。正規の表はここに届かない。上限は手で作った・壊れたファイル(最大 4MiB)への防衛としてだけ意味がある。超えた件数は `cell_count − cells.len()` で分かるので、別フィールドは足さない。
- 予測に使われるセル(`convert_cell` が `Conv` に変換できるもの)と、使われないセル(`Conv` に表せない mode)は区別できるよう、使われないセルには末尾に `!` を付ける。

学習表は「IME 状態 × キー番号」だけで打鍵内容を含まない(ADR-195)。書き手は学習プロセスの固定の 14 キーだけを打つ(`awase-keymap-learn-win`)。キー番号・mode は数値で、任意文字列の経路は無い。

サイズの見積もり: 書式は 1 セル約 20 バイト、448 件で約 9KB。サーバ本体の上限 2MiB、ログ以外に見込んだ 256KiB(`services/report-worker/src/index.ts`)に対して十分小さい。ただしセルは `build_payload_json_fitting` が縮める対象ではないので、上限に近い報告ではセルの分だけログが削られる。

### 決定3: `SCHEMA_VERSION` は上げない

追加のみ。ADR-148 と同じく `deny_unknown_fields` を付けておらず、新フィールドは `Option` / `#[serde(default)]`。設定側の `load_diagnostics` は失敗を `.ok()` で握りつぶすので、必須フィールドを足すと旧形式の診断ファイルを読んだときに `state_snapshot` まで含めて全部が消える(`bug_report.rs` の `legacy_compat_mode_enabled` と同じ罠)。件数のフィールドにも `#[serde(default)]` を付ける。サーバの `gji_keymap` は `Record<string, unknown>` の素通しで、サーバ側の変更は無い。

### 決定4: 送信前プレビューと同意

確認済み(Opus round 1)。`gji_keymap` と `keymap_learn` は送信前プレビュー JSON に入り、チェックボックス「IMEのキーマップ設定を添付する」(既定 ON)で外せる(`crates/awase-settings/src/bug_report.rs`)。サーバはフラグが false なのにデータがある報告を 400 で拒否する(`services/report-worker/src/index.ts`)。新フィールドは同じ構造体の中なので、同じチェックボックスでまとめて外れ、ADR-095 決定4 は満たす。新しい添付フラグは足さない。「表の行は送るが学習表のセルは送らない」はチェックボックスではできず、プレビュー JSON の手編集でしかできない。

- ツールチップ(`awase-settings/src/bug_report.rs:294`)の文言を、実際の送信内容に合わせて更新する。例: 「無変換/変換キー等への IME ON/OFF 割り当て設定と、キー効果の学習結果(IME の状態×キーの番号だけ。打鍵内容は含みません)を送信内容に含めます」。
- 既存の穴(この ADR が作ったものではない): プレビューを編集した後でチェックを外しても、キーマップがプレビューの古い内容のまま送られる(`refresh_preview_if_unedited`、送信は `self.preview_json` をそのまま使う)。このチェックボックスの下のデータが増えるので、別の BUG として起票し(`docs/known-bugs/` に1件)、この ADR の実装とは別 PR で直す。

### 決定5: 次の候補(この ADR では実装しない)

Opus round 1 は、生のセルより切り分けに直接効く案を 2 つ挙げた。ユーザーが明示的に表の行と学習表の添付を求めたので決定1・2 は実装するが、次は別 ADR で検討する。

- 5-1: `KeyEffectPredicted`(journal)に、予測を出した分岐(`unreadable_window`・`passive_open`・`custom_f_key`・`learned`・`legacy_cells`・`bundled`)を固定語彙で残す。`KeyEffectPredicted` は belief の reducer への入力なので、足すときは reducer が無視するフィールドにするか、journal 専用の記録に分ける(ime-belief-architecture の範囲)。
- 5-2: 報告を作る時点で、候補キー × 代表状態(閉・開かな・開半角英数)× 読める窓/読めない窓に `predict_with_override` をかけた結果の表を送る。「閉状態の英数が予測にどう効くか」への直接の答えで、awase の版で優先順位が変わっても読み方が変わらない。

## 却下した案

- **`custom_keymap_table` を生の TSV で丸ごと送る**: 任意文字列(ユーザーが付けたコマンド名や他の行)が混入する。ADR-148 が封じた経路そのものになる。
- **表のハッシュだけ送る**: 同じ表かどうかは分かるが、中身を見られず今回の切り分けに使えない。
- **全行を送る**(修飾キー付き・文字キーも): 診断に要るのは IME 開閉に関わる行だけで、残りは量と漏れの経路を増やすだけ。修飾キー付きの IME 行は件数だけ送る(決定1-4)。

## 検証

- `diagnostic_rows` の単体テスト(Linux): allowlist 外の行が落ちる、`CompositionModeHiraganaX`・`InputModeFoo` のような近い綴りが落ちる、出力の文字列がすべて固定表のどれかと一致する、見出し行は数えない、別名の重複は 1 行、同じ (状態, キー) に違うコマンドの行は両方残る、落とした行の理由別の件数が合う。
- 学習表のセルの書式・並び・上限・`!` の印の単体テスト(`bug_report.rs`)。
- 旧形式の JSON が読めること(既存の `gji_keymap_summary_with_removed_adopted_keys_still_deserializes` の拡張)。
- 実 GJI の挙動の確認は済み(背景)。

## 未確認・リスク

- コマンド語彙は既知の Mozc の名前から作っている。実機の表に別の綴りがあると送られず、`total_rows` と送った行数の差として見える。
- 報告 01M4QSFH1B の journal は、閉状態の英数を押した打鍵を含まない。この ADR を実装しても、その報告者の一太郎での食い違いの原因が表やセルから分かるとは限らない(原因が窓別の規則や読めない窓の扱いなら、決定5 が要る)。
