---
id: ADR-287
title: |-
  mozc-readable の共有メモリに「ローマ字入力/かな入力」の方式そのものを足し、OS のロックビット頼みを置き換える
summary: |-
  BUG-106（Mozc/GJI系でawaseの送信romajiがJISかな配列として解釈される）は、検知を`GetKeyState(VK_KANA)`というOS
  キーボードのロックビット直読みに頼っている（ADR-118）。これは「かなロック」というOS側のトグル状態であり、Mozc
  自身の`config.preedit_method`（ローマ字入力=ROMAN/かな入力=KANA、要確認・本文参照）という**入力方式そのもの**とは
  別軸で、ドライバ依存の遅延・エコー（BUG-14）を抱える。2026-11-04 11:52 JST、dragonflyg4の実機でWindows
  Terminal×Google日本語入力（Mozc Readable想定）が稼働中、awaseの送信romaji "to"を打った瞬間に`[kana-lock]`警告が
  発火し、直後の出力が「ｶﾗ」（JISかな配列でT→か、O→らに一致）に化けた実例を確認した（BUG-106追補7）。mozc-readable
  は既にIMEの開閉・CompositionMode（文字種）を共有メモリ経由で確実に読めている（ADR-265〜270）が、入力方式
  （ローマ字/かな）の軸は現在の共有メモリ（version 6、152バイト）には含まれていない。決定: mozc-readable フォーク
  （別リポジトリ、本ADRの実装範囲外）に入力方式フィールドを追加してもらい、awase側はversion 7として既存の
  バージョンゲート方式（ADR-267/268と同じ、先頭バイトは互換）で読み取る。段階1はObserve専用（journalのみ、belief
  には入れない）で、OSロックビットとの食い違い・遅延を実機で集めてから、段階2（警告の即時化、または保留/送信停止）
  を別ADRで検討する。
status: |-
  起草（2026-11-04）。未実装。mozc-readable 側のフィールド追加が前提で、awase 単体では着手できない。
related_adr:
  - "ADR-118"
  - "ADR-265"
  - "ADR-267"
  - "ADR-268"
---

# ADR-287: mozc-readable に入力方式（ローマ字/かな）の軸を足す

## 背景

[BUG-106](../known-bugs/BUG-106.md)・[BUG-008](../known-bugs/BUG-008.md)・[BUG-061](../known-bugs/BUG-061.md)は、
IME側で「ローマ字入力」から「JISかな直接入力」に入力方式そのものが反転し、awaseが送った romaji 相当の VK 列が
JIS かな配列として誤解釈される症状である。[ADR-118](118-teams-kana-lock-detection.md)で検知機構（`KanaLockHysteresis`、
`observer/kana_lock.rs`）を実装したが、これは`GetKeyState(VK_KANA)&1`という**OSのキーボードロックビット**を直読みする
もので、入力方式そのものを保持しているIME（Mozc/MS-IME）の内部状態とは別物である。ADR-118自身が「Google日本語入力は
今回の検知の対象外（実機テストでは言語バーからの手動切替自体が機能しなかった）」と明記している。

2026-11-04 11:52 JST、dragonflyg4の実機（Windows Terminal、`ime=GJI`のログラベルだがMozc Readable運用中）で、
ユーザーが「そのとおりですね」と入力した際に次のログが記録された（UTC、`awase.log`より抜粋）:

```
02:52:11.037 WARN [kana-lock] OS かな入力ロックを検知しました (reading=On streak=On(3) fg_class=CASCADIA_HOSTING_WINDOW_CLASS)
02:52:11.039 INFO [key-output] KeyInput(batched): romaji="ha" ime=GJI
02:52:11.292 INFO [key-output] KeyInput(batched): romaji="so" ime=GJI
02:52:11.387 INFO [key-output] KeyInput(batched): romaji="no" ime=GJI
02:52:12.212 INFO [key-output] KeyInput(batched): romaji="to" ime=GJI
02:52:12.341 INFO [key-output] KeyInput(batched): romaji="o"  ime=GJI
02:52:12.629 INFO [key-output] KeyInput(batched): romaji="ri" ime=GJI
...(de/su/ne/. と続く)
```

実際の出力は「そのｶﾗおりですね」——「と」(romaji "to") だけが「ｶﾗ」に化けた。JISかな配列ではTキー=「か」、
Oキー=「ら」であり、半角表記すればちょうど「ｶﾗ」になる。送信した romaji 列自体は正しく、入力方式がその一瞬だけ
JISかな直接入力になっていたと綴りレベルで完全に一致する。

同じログの中で`[kana-lock]`警告は3時間強のセッション中に何度も（00:41, 02:19, 02:22, 02:25, 02:52 UTC...）
`fg_class=CASCADIA_HOSTING_WINDOW_CLASS`で発火しており、単発ではなく繰り返し発生している。何が`VK_KANA`トグルを
反転させているかは本ADRの時点でも未特定（BUG-008/BUG-061と同じ「原因未確定、awase側に復旧手段なし」の系統）。

## 現状の mozc-readable 共有メモリでは検知できない理由

[ADR-265](265-mozc-readable-integration.md)〜[ADR-270](270-mozc-first-send-composition-check.md)で整備した
`TipSharedState`（`crates/awase-ime-core/src/state/mozc_shm.rs`、version 6で152バイト）が持つのは:

- `effective_open`/`tsf_open`: IMEの開閉
- `effective_mode`/`tsf_mode`: `CompositionMode`（ひらがな/全角カタカナ/半角カタカナ/半角英数/全角英数/直接入力—
  **文字種**の軸）
- version 3以降: `phase`・`candidate_category`・`preedit_*`・`key_seq`・`scope_forced_mode`・`server_ready`等
  （組成・確認用）
- version 5: フォーカス遷移の tick（診断用）
- version 6: `key_eaten_count`/`key_passed_count`（キー単位の食べた/素通りの数、診断用）

いずれも「ローマ字入力かJISかな直接入力か」という**入力方式**そのものは持たない。Mozcの設定上はこれは
`CompositionMode`とは独立した軸（Mozcの設定でいう`preedit_method`、値はROMAN/KANA—**正確なフィールド名・読み出し
経路はmozc-readable側のソースで要確認、本ADRでは未検証**）であり、今回のBUG-106再現はこの軸が反転したことが
直接原因と推定される。したがって、現状の共有メモリをどれだけ読んでも「かな直接入力に切り替わった」事実そのものは
拾えず、ADR-118のOSロックビット頼みから抜け出せていない。

## 決定

1. **mozc-readable フォーク側の変更（本ADRの実装範囲外、別リポジトリ [cuzic/mozc-readable](https://github.com/cuzic/mozc-readable)
   の作業）**: `TipSharedState`に入力方式のフィールドを追加する。既存のバージョンゲート方式
   （version 2→3→5→6で先頭48バイトの意味を変えずに末尾へ追記してきたのと同じパターン）に従い、**version 7**として
   8バイト境界で追記する。値はMozcの設定が持つ生の値をそのまま書く（awase側で意味づけをしない—ADR-265 D3の既存
   方針どおり）。
2. **awase側（本ADRの実装範囲）**: `crates/awase-ime-core/src/state/mozc_shm.rs`に`V7_BLOCK_SIZE`定数と対応する
   `parse`関数を追加し、`RawBlock`/`ShmState`とは別の追記構造体（`CompositionFields`・`FocusTicks`と同じ並び）
   として持つ。既存の受け入れ条件（`classify`）・`ShmUse`の判断には**混ぜない**——version 7が読めない環境
   （mozc-readable未更新・GJI公式版・MS-IME）では単に`None`のまま扱う。
3. **段階1はObserve専用**（ADR-265 D3・ADR-268と同じ慎重さ）: 新フィールドの値は journal にだけ記録し、
   belief・actuation の判断には一切使わない。同時に、既存の`observer/kana_lock.rs`（`GetKeyState(VK_KANA)`）の
   読みも並行して記録し、両者が実機でどれだけ一致/食い違うか（タイミングのずれ、ドライバ由来のエコーで
   OSロックビットだけ動く場面の有無等）を集める。
4. **段階2（本ADRでは決定しない、観測結果を見て別ADRを起票する）**: 段階1のデータで「mozc-readableの新フィールドが
   OSロックビットより速く・正確に反転を捉えられる」と確認できたら、検知後のアクション
   （即時警告への切り替え、打鍵の一時保留、ADR-270の確認機構との統合等）を検討する。自動復旧（入力方式を
   awase側から書き戻す）はBUG-61/BUG-62追補4で実機上不可能と確定済みのため、本ADRでもスコープ外のまま維持する。

## 適用範囲の限界

- 本ADRが効くのは mozc-readable を使っている窓だけ。公式 Google 日本語入力（mozc-readable 未適用）・MS-IME には
  この入力方式を読む経路が無く、BUG-106/BUG-061の本来の主戦場だったMS-IME（Windows Terminal + MS-IME）には
  効果がない。今回の実機ログは `ime=GJI` だったが、Mozc Readable運用中だったため本ADRの対象に入る——通常のGJI
  （mozc-readable非適用）だった場合はこの経路は使えない。
- 入力方式の反転そのものの原因（何が`VK_KANA`トグルを動かしているか）は本ADRでも未解決。本ADRは「反転を確実に
  知る」ところまでで、「反転を起こさせない」は依然スコープ外。
- 段階1がObserve専用である間は、ユーザーに見える挙動の変化は無い（`.claude/rules/fix-requires-evidence.md`の
  回帰テスト義務は段階2で実際に判断・actuationに使うときに効いてくる）。

## 関連

[BUG-106](../known-bugs/BUG-106.md)（追補7、本ADRの引用元となった実機ログ）・[ADR-118](118-teams-kana-lock-detection.md)
（現行のOSロックビット検知、本ADRが補完する対象）・[ADR-265](265-mozc-readable-integration.md)〜
[ADR-270](270-mozc-first-send-composition-check.md)（既存の共有メモリ連携、本ADRが踏襲するバージョンゲート方式と
Observe専用の段階分け）。
