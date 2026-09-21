---
id: ADR-193
title: |-
  TSFネイティブ相当の入力先を、RichEditのスーパークラス化で決定的に用意する（実機E2Eの検証対象拡張）
summary: |-
  Chrome/Windows Terminal に頼らず、awase の TsfNative 経路（Vk 注入・warmup・per-VK confirm）を決定的にテストできる入力先が欲しい。
  `RICHEDIT50W`（Msftedit.dll、TSF text store を持つ本物の RichEdit）を `GetClassInfoExW` で取得し、`Chrome_RenderWidgetHostHWND` の名前で
  `RegisterClassExW` し直す（スーパークラス化、上位窓は `Chrome_WidgetWin_1`）。awase の分類はクラス名の文字列一致なので、実機で
  `app_kind=TsfNative → mode=Vk` として扱われ、確定文字列を `WM_GETTEXT` で厳密に読める（`examples/richedit_tsf_probe.rs`、スパイク成功）。
  実機（GJI）で素の RichEdit と同じ結果（idle 0/6000/11000 各3回とも `きう`）。実 Chrome での BUG-002 型（cold-start のリテラル化）は
  実機で再現しなかった（有効 probe 20件すべて `きう`、RawTsfLiteralRecovery/SuspectedLiteral 0件）ので、Chrome cold-start 専用の新規測定装置は作らない。
status: |-
  **採用（スパイク成功、2026-09-21）**。土台は `examples/richedit_tsf_probe.rs`。CI（`e2e-ime.yml`）への配線は未着手（GJI 有効化の `--activate-gji` 相当が必要）。
  詳細な CI 化・idle 掃引の設計案は [193-implementation-tasks.md](193-implementation-tasks.md)（**保留・参考**。症状が再現した場合にだけ使う）。
related_adr:
  - "ADR-0002"
  - "ADR-0003"
  - "ADR-186"
  - "ADR-191"
---

# ADR-193: TSFネイティブ相当の入力先を RichEdit のスーパークラス化で決定的に用意する

## 背景

awase の難所は TSF ネイティブのアプリ（Chrome・Windows Terminal・Windows 11 のメモ帳）での IME 状態追跡で、既存の実機 E2E の入力先は
IMM32 の素の `Edit`（`e2e_windows.rs`）と、確定文字列を読めず CI で走らない Windows Terminal 系だけだった。
決定的に確定文字列を assert できる TSF ネイティブ相当の入力先が要る。

## 決定

1. **入力先: `RICHEDIT50W` をスーパークラス化する。** `Msftedit.dll` を `LoadLibrary` し、`GetClassInfoExW(RICHEDIT50W)` で得たクラス情報を
   `Chrome_RenderWidgetHostHWND` の名前で `RegisterClassExW` し直す。上位窓は `Chrome_WidgetWin_1`。`detect_app_kind`（`focus/class_names.rs`）は
   クラス名の前方一致 `chrome_` だけで `TsfNative` を返し、`AppImeProfile::from_class_name` もクラス名テーブル一致のみなので、awase から見て
   Chrome と同じ TsfNative / Vk 経路になる。確定文字列は `WM_GETTEXT` で読む（Chrome も HTTP サーバも不要）。
2. **キー注入は目印付き `SendInput` + `AWASE_TEST_INJECTION=1` + debug ビルド**（`hook::TEST_INJECTION_MARKER`）。目印なしの注入は awase を素通りし、
   「再現しない=直った」と誤読する偽陰性になる。
3. **Chrome cold-start 用の新規測定装置は作らない。** 実機（dragonflyg4、GJI）で既存 `chrome_probe --settle=3000〜14000` を測り、BUG-002 型は再現しなかった
   （`docs/experiments.md` の2026-07-18以降の実機ソークと一致）。`document.title` 回収案は採らない（既存 `chrome_probe` が連番付きイベントをローカル HTTP に送る）。
4. **TSF compartment の読み取り・変更通知は本 ADR の対象外。** 読み取りは ADR-186 の手法Tで実装・実測済み、変更通知（`ITfCompartmentEventSink`）は ADR-191 の観測（c）の担当。

## 実機での確認（スパイク）

| 入力先 | awase の扱い | 確定文字列（`k`,`a`） |
|---|---|---|
| 素の `RICHEDIT50W`（対照） | Standard（ImmCross 経路） | `きう` ×3 |
| スーパークラス化（`Chrome_RenderWidgetHostHWND`） | `app_kind=TsfNative → mode=Vk`（awase.log の `[focus-sync]`） | `きう` ×3（idle 0）、×3（idle 6000ms）、×3（idle 11000ms） |

`cache.toml` に `[injection_mode]`・`Chrome_`・プローブの記録は書かれなかった（`learn_tsf` は発火せず）。ただし恒常的な保証ではない。

## 制約・未確認

- 決定的に検証できるのは **awase が何を送るか**まで。Chromium が実際に TSF でどう受けるか（cold-start の composition context 再初期化のタイミング）は再現しない。
- RichEdit は IMM コンテキストを持つ（Chrome・Windows 11 のメモ帳は持たない）。awase はクラス名で TsfNative 経路に入るので awase 側には影響しないが、忠実度の差として残る。
- 分類の偽装は、ユーザーの実機では `InjectionModeStore`（クラス名単独キー・`cache.toml` に永続化）が実物の Chrome を汚染しうる。CI の使い捨て VM では問題にならない。
  ローカルで使うときはキャッシュ保存先を分けるか、実行後に `cache.toml` を確認する。
- 陽性対照（差が出る場面）は未整備。CI 化には `--activate-gji` 相当（`ActivateProfile`+`VK_IME_OFF`、`ime_key_matrix_spike.rs:1590-1642`）が必要。
- 実機で得た罠: `chrome_probe` を PowerShell のループ内から連続起動すると 2 回目以降は前面化に失敗しページが応答しなくなる。**1 回ずつ別々に起動する**。

## 検討過程の教訓

当初案（`document.title` 回収の新規 Chrome 用ハーネス、RichEdit を作らない）は、既存資産（`ime_key_matrix_spike`・`chrome_probe`・`e2e-ime.yml`）の棚卸しをせず、
さらに known-bugs・`tuning.rs` の doc・workflow のコメント・レビュー指摘を実装で裏取りせずに前提にした。5 ラウンドの実施計画レビューは、症状が実機で
再現するかを確認する前に測定装置を設計していたために毎回穴が出た。**症状の存在は実機で先に確認する。** 記録は `193-opus-review-*.md`。
