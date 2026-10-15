---
id: ADR-259
title: |-
  不具合報告にカスタム表の IME 関連行と学習表のセルを添付する
summary: |-
  報告 01M4QSFH1B(一太郎・GJI の CUSTOM)は、英数が「IME ON 兼 英数トグル」の複合キーかを報告から判定できず、belief の食い違いが設計の範囲かを切り分けられなかった。ADR-148 は表の要約(キーの分類)だけを送っていた。custom_keymap_table の IME 関連行を3列とも固定語彙・allowlist を通して送り、学習表のセルを数値だけの固定書式で送る。
status: |-
  起草(2026-10-14)。Opus レビュー前。実装なし。
related_adr:
  - "ADR-095"
  - "ADR-148"
  - "ADR-196"
  - "ADR-199"
  - "ADR-209"
---

# ADR-259: 不具合報告にカスタム表の IME 関連行と学習表のセルを添付する

## ステータス

起草(2026-10-14)。Opus 敵対的レビュー前。コードは変えていない。

## 背景

不具合報告 `01M4QSFH1B02DQ9B8AXBYVRQ39`(GJI・一太郎、2026-10-12。issue #623)の切り分けで、次が報告から判定できなかった。

- GJI は `session_keymap = 0`(CUSTOM)で、英数(`VK_DBE_ALPHANUMERIC`)が `ime_on_keys` と `mode_toggle_alphanumeric_keys` の両方に分類されていた。DirectInput の行が IMEOn で、開状態の行が `ToggleAlphanumericMode` という複合キーなのか、別の組み合わせなのかは、分類結果からは分からない。
- journal では英数の効果予測が `open: null`(mode と conv だけ)だった。CUSTOM でその表に英数の行があると、同梱表の予測は打ち切られる(`custom_table_overrides`、ADR-209 決定4)。予測が付いたのは学習表のセルがあるためで、そのセルが閉状態の英数をどう答えているかも報告に無い。
- 報告に載っているのは、キーの分類(`ime_on_keys` など)と、学習表の件数・判定・指紋・同梱表との差の一部だけである(ADR-148、ADR196-T2)。表そのものは載っていない。

「IME キーを押せば belief が直る」という設計に照らして、報告者の環境がその範囲内かを、実機の表を見ずに判断できない。

## 決定

### 決定1: `custom_keymap_table` の IME 関連行を、固定語彙だけ通して添付する

`BugReportGjiKeymapSummary` に `custom_keymap_rows: Option<Vec<(String, String, String)>>`(状態・VK 名・コマンド)を足す。`custom_keymap_table_is_effective` のときだけ `Some`。

3列とも `config1.db` 由来の任意文字列を通さない。ADR-148 の型 doc が「allowlist が唯一の防壁」としている前提を守る。

- 状態: `DirectInput`・`Precomposition`・`Composition`・`Conversion`・`Prediction`・`Suggestion` のいずれかと完全一致したものだけ。送るのは固定語彙側の文字列。
- キー: `mozc_key_vk_names` が空でないものだけ。送るのは VK 名を `/` で連結した文字列(例: `VK_DBE_ALPHANUMERIC`)。修飾キー付き・文字キー・未知のキー名の行は送らない。
- コマンド: IME の開閉・入力モードに関わる固定語彙(`IMEOn`・`IMEOff`・`CancelAndIMEOff`・`ToggleAlphanumericMode`・`SwitchKanaType`・`CompositionMode*`・`InputMode*`)と完全一致したものだけ。送るのは固定語彙側の文字列。

純粋関数 `awase_gji_config::keymap::diagnostic_rows` を新設し、Linux の単体テストで allowlist 外の行が落ちることを固定する。

### 決定2: 学習表のセルを、数値だけの固定書式で添付する

`BugReportKeymapLearnSummary` に `cells: Vec<String>` を足す(`#[serde(default)]`)。`table_file == "loaded"` のときだけ。1 セルは `open=<0|1> mode=<数> composing=<0|1> key=<数> -> open=<0|1> mode=<数> composing=<0|1> disp=<固定語彙>` の形で、`prediction` の無いセルは `-> none`。件数は 1024 件を上限にし、超えたら打ち切って件数を別フィールドで示す。

学習表は「IME 状態 × キー番号」だけで打鍵内容を含まない(ADR-195)。キー番号は VK の数値で、任意文字列の経路は無い。

### 決定3: `SCHEMA_VERSION` は上げない

追加のみ。ADR-148 と同じく `deny_unknown_fields` を付けておらず、新フィールドは `Option` / `#[serde(default)]` なので旧 JSON も読める。サーバの `gji_keymap` は `Record<string, unknown>` の素通しで、サーバ側の変更は無い。

### 決定4: 送信前のプレビューと同意は ADR-095 決定4 に従う

新しい添付フラグは足さず、`attach_ime_keymap` に相乗りする(ADR-148 と同じ理由)。プレビューに出るか、ユーザーが外せるかは実装前に確認する(未確認、Opus レビューで指摘してほしい点)。

## 却下した案

- **`custom_keymap_table` を生の TSV で丸ごと送る**: 任意文字列(ユーザーが付けたコマンド名や他の行)が混入する。ADR-148 が封じた経路そのものになる。
- **表のハッシュだけ送る**: 同じ表かどうかは分かるが、中身を見られず今回の切り分けに使えない。
- **全行を送る**(修飾キー付き・文字キーも): 診断に要るのは IME 開閉に関わる行だけで、残りは量と漏れの経路を増やすだけ。

## 検証

- `diagnostic_rows` の allowlist 単体テスト(Linux)。
- 学習表のセルの書式と上限の単体テスト(`bug_report.rs`)。
- 旧形式の JSON が読めること(既存の `gji_keymap_summary_with_removed_adopted_keys_still_deserializes` の拡張)。
- 実 GJI での確認は別途、CI の `sc-eisu-composite-*`(ブランチ `ci/e2e-eisu-composite`)で、閉状態の英数が実際に開くかを見る。結果はこの ADR の根拠にする。

## 未確認・リスク

- 状態・コマンドの固定語彙は、既知の Mozc の名前だけから作っている。実機の表に別の綴りがあると送られず、行が欠けて見える。欠けた件数を送らないと、欠落に気づけない(決定1に足すか、レビューで判断)。
- 学習表のセル数は構成により 300 を超えうる。報告全体の大きさへの影響は未測定。
