---
title: v2 実機確認の実施結果（2026-09-30、dragonflyg4）
status: 一部実施（X5・X1・X2 を実施。X3・X4・D1・D3 は未実施）。結果は awase.log とあなたの目視の突き合わせ
created: 2026-09-30
related_adr: ["ADR-205", "ADR-206", "ADR-207", "ADR-202", "ADR-191", "ADR-203"]
---

# v2 実機確認の実施結果（2026-09-30）

手順は [v2-manual-verification-guide-2026-09-29.md](v2-manual-verification-guide-2026-09-29.md)。実機: dragonflyg4（JIS キーボード、GJI、Edge、Windows Terminal）。
awase は develop 先端（`1ec078ef`、コードは `a0ac0def` 以降と同じ）を別ディレクトリ `awase-verify` でビルドし、debug ログで起動した。時刻は UTC（JST-9h）。

## 結果の一覧
| 項目 | 結果 | 要点 |
|---|---|---|
| **X5**（BUG-172、ADR-205） | #377 の効果は確認。**偽 OFF の疑い1件が未解消** | [BUG-176](../known-bugs/BUG-176.md) |
| **X1-1**（無変換/変換の単独タップ、既定） | **IME OFF で無変換を押すと「@」が出る** | 所有者判断（ADR-206 決定3〔iii〕） |
| X1-2（Passthrough の役割由来） | **対象外** | GJI のキー設定で無変換/変換がトグルではない |
| X1-3（旧 `"off"` 設定の移行） | 期待どおり。ただし**非推奨のトレイ通知は出ない**（ログのみ） | |
| X1-4（Ctrl↑で IME を動かさない） | 合格。「@」1回のみ（再現せず） | BUG-174 の条件は満たす |
| **X2**（既定 `[keys]` の Alt+半角/全角） | 1押下1反転は合格。**素早い連続押下でずれた可能性** | |
| X3、X4、D1、D3 | 未実施 | |

## X5（BUG-172）
外部注入の VK_IME_OFF（0x1A）→ 3秒後に `z` を注入し、入力欄の値を UI Automation で読んで判定した（`z`=IME 閉＋追随／`．`(U+FF0E)=追随せず Engine ON／`ｚ`(U+FF5A)=IME 開）。
- #377 の直前（`89f5e63f`）: `．`×3。#377 のマージ後（`a4022ba9`）・develop 先端: `z`×3。**#377 の効果は実機でも確認できた。**
- develop 先端は、ON の準備を変えても（注入 VK_IME_ON／目印付き注入＝物理キー扱い、1.5秒・25秒アイドル）`z`×9。新しく起動した awase の手動（Ctrl+変換で ON）も `z`。
- **未解消**: 最初に起動した awase での手動の連続試行で、IME が開いたまま awase が `open=false` へ追随する異常が3回。再現条件は不明（[BUG-176](../known-bugs/BUG-176.md)）。
- awase 停止中は注入で IME が閉じる。キー注入は入力欄に届く（`z` 単独の注入で確認）。

## X1（無変換/変換の単独タップ）
- **X1-1（既定＝Suppress）**: IME ON で無変換×10 は、飲み込まれて IME は動かず「@」なし。**IME OFF で無変換×10 は、awase が生キーを再注入（`[reinject] vk=0x1d`）し、GJI に届いて「@」が出る**（仕様として受け入れた一方向の結果。ADR-206 の未検証事項が実測で確定）。IME OFF で変換×10 は、GJI 自身が ON にする。
- **副次（X1-1・X1-3 で計8回）**: 変換の素通しで GJI 自身が開けた IME を、awase が**ドリフト補正で閉じ直している**（`[drift] correction: observed=true ≠ desired=false … source=ConvOpenInference` → VK_IME_OFF）。ADR-191（IME が正、awase は書かない）と矛盾する動きで、オープン中の PR #360（ConvOpenInference の drift 撤去）の対象経路。
- **X1-2**: この実機の GJI のキー設定（`config1.db`）は、変換が「DirectInput=IMEOn、入力中/変換中=CompositionModeHiragana」、無変換は専用行なしで、ADR-199 決定11 のトグルに当たらないため対象外。
- **X1-3**: 設定を `muhenkan_solo_tap_ime_action = "off"`＋`always_suppress = true` にすると「@」は出ず、IME ON で無変換→awase が VK_IME_OFF を**1回だけ**送って閉じる（3ms 以内）。ログには `general.muhenkan_solo_tap_ime_action は非推奨です` が出るが、**トレイ通知は表示されなかった**。
- **X1-4**: Ctrl 単独×20・Ctrl+Shift×20 の間、awase の IME 送信は0件。「@」は1回だけ出た（半角 `k` の後、再現せず。awase の `VK_IME_ON`〈tsf_marker_warmup〉が GJI の直接入力で受けられた BUG-113 型の可能性。確証なし）。Ctrl+無変換/変換は1押下1送信。
  **副次**: Ctrl+変換の約1.2秒後に Ctrl+無変換を押すと、`Rapid IME key press detected — requesting panic reset` が出て、OFF のつもりが VK_IME_ON の送信になった（意図された安全機構だが、1秒台の ON/OFF 切替でも発動する）。

## X2（既定の設定、Alt+半角/全角）
- Windows Terminal では、**awase を止めても** Alt+半角/全角で「@」が5/5で出る（IME は毎回反転）。awase の原因ではなく、JIS キーボード＋Windows Terminal の環境の挙動。
- メモ帳（awase 稼働）: 押すたびに awase の belief と Engine がちょうど1回ずつ反転（物理 0x19 は `PhysicalImeKey` の Toggle、二重トグルなし）。`ka`／`きう` が交互で期待どおり。
- **異常**: 半角/全角を**0.2〜0.6秒間隔で2回続けて押した箇所**（06:17:07、06:17:10、06:17:20）の前後で、`か`（IME ON かつ Engine OFF）が2回、`ka` が2回連続。トグルの約0.2秒後に `[drift] correction` も2回。素早い連続押下で、実 IME と Engine がずれる可能性がある（要追跡）。
- (b) GJI の CUSTOM 表で Hankaku/Zenkaku 行をトグルにしない場合は未実施（キー設定の変更が要る）。

## PR #360 の実機検証（所有者の依頼: 実機で検証してから決める）
develop 先端に #360 のヘッド（`073514a9`）をマージ（競合は CI・テスト・ドキュメントのみ、develop 側を採用）した awase-verify ビルドと、develop 先端を比べた。
- **自動注入**（目印付きの物理キー扱い、変換×10）は、どちらもドリフト補正0回で**症状を再現できず**、比較にならなかった（GJI が注入した変換を受けたかも不明）。
- **手動**（IME OFF → 無変換×10 → 変換×10 → 40秒待つ）: develop 先端は、ドリフト補正による VK_IME_OFF の送信が繰り返し出ていた（X1-1・X1-3 で計8回）。**#360 入りは0件**（最後の押下から47秒後まで確認）。
- **変換を単独で1回押した後の `k` `a`**（IME OFF から、3回）: develop 先端は `か`（IME ON・Engine OFF）→ awase が IME を閉じ直し、次は生の `ka`（`か ka か ka か ka`）。#360 入りは `か`（IME ON・**Engine OFF のまま**、約19秒後の別の同期まで）。
- **結論**: #360 は「awase が IME を閉じ直す」動きをなくすが、**変換の素通しで GJI が開けた IME に Engine が追随しない**という根本の症状は、どちらのビルドでも同じ（NICOLA が効かず `か`）。#360 だけでは、この実機の体感は改善しない（IME ON のまま NICOLA OFF が長く続く）。

## X1-1 の追加（所有者の決定、2026-09-30）
- 「@」（IME OFF の無変換素通し）: **仕様として受け入れて告知する**。
- BUG-176: **観察継続**。X2 の連続押下: **間隔を空けた比較をしてから起票を決める**。
- **新たな論点**: 変換単独で GJI が IME を ON にしたとき、Engine が追随しない（`か`）。既定の IME ON は Ctrl+変換で、変換単独は awase の ON キーではない。対処の案は、(a) 既定の ON キー（Ctrl+変換）を使うよう告知、(b) 設定画面の「この置き換えを適用」で `keys.ime_on` に変換を足す（awase が ON を自分で処理）、(c) 素通し後に awase が観測して Engine を追随させる設計（ADR-206/191 の見直し）。

## 判断が要るもの・次にやること
1. **X1-1 の「@」**: 受け入れるか、ADR-206 決定3（iii）（同じ OFF キーを2回続けたときだけ送る等）へ進むか。
2. **ドリフト補正が IME を閉じ直す件**: 実機検証の結果（上）、#360 単独では体感は改善しない。v2 に入れるかは、変換単独の Engine 追随の対処（上の (a)〜(c)）と併せて判断。
3. **BUG-176（X5 の偽 OFF 疑い）**: ブロッカーにするか、観察継続にするか。
4. **X2 の素早い連続押下のずれ**: 再現手順の確定（3秒空けた連続押下との比較）と BUG 起票の要否。
5. **X1-3 のトレイ通知**: 非推奨の通知をトレイに出すか（現状はログのみ）。
6. 未実施: X3、X4（MS-IME 値2。設定アプリの操作は UI Automation で自動化できる）、D1、D3。
