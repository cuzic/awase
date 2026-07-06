# 実機スモークテスト運用ガイド

> 対象スクリプト: [`scripts/smoke-matrix.ps1`](../scripts/smoke-matrix.ps1)
> 最終更新: 2026-07-05

## なぜこれが必要か

awase の warmup / cold-start 系の修正は、これまで **「あるセルを直すと別のセルが退行する」**
形の再発を繰り返してきた（Chrome の cold-start を直すと WezTerm が壊れる、GJI を直すと
MS-IME が壊れる、等）。原因は入力の成否が **アプリ × IME × シナリオ** の組合せに強く依存し、
その組合せ空間を毎回網羅チェックしていなかったことにある。

`smoke-matrix.ps1` はこの組合せ空間を機械的に列挙し、バージョンバンプ前に人間が
全セルを一巡できるようにする。「たまたま開いていたアプリで動いた」で済ませない。

## マトリクスの軸

`known-bugs.md` / `docs/adr/033`,`043` / `focus/class_names.rs` に実際に登場するものだけを対象にする。

### アプリ（`$Targets`）

| キー | アプリ | 配送方式 | 読み戻し可 | 根拠 |
|---|---|---|---|---|
| `chrome`  | Google Chrome | VkBatched (Imm32Unavailable) | 可 | BUG-02 (`という→toいう`) |
| `edge`    | Microsoft Edge | VkBatched (Chromium) | 可 | Chrome と同配送 |
| `wezterm` | WezTerm | TSF native (himc_null) | **不可** | BUG-01 (`かんきょう…→kあ…`) |
| `notepad` | メモ帳 | IMM32 classic | 可 | Win32 基準・IMM32 クロスプロセス可 |
| `line`    | LINE | ImmCross (VK_KANJI) | 可 | ImmCross アプリ、物理 IME キー非提示 |
| `vscode`  | VS Code | VkBatched (Electron) | 可 | Chromium 系 |

> **WezTerm が「読み戻し不可」なのは意図的。** ターミナルは編集フィールドの
> 全選択→コピーが安定せず、リテラル化の判定が目視でしか確実に行えないため、
> WezTerm のセルは常に Manual 扱いになる。

### IME（`$ImeDefs`）

- `gji` — Google 日本語入力（候補ウィンドウ I/O を `GjiMonitor` が監視）
- `msime` — Microsoft IME（TSF、`MsImeDirectStrategy` / ADR-063）

### シナリオ（`$ScenarioDefs`）

| キー | 内容 | 自動判定 |
|---|---|---|
| `cold-start`    | 起動/長期 idle 後の 1 文字目（`という`） | 可 |
| `alt-tab`       | Alt+Tab でフォーカス復帰直後（`かんきょう`） | 可 |
| `idle-30s`      | 30 秒 idle 後（SessionExpired 2000ms + long_idle 経由、`にゅうりょく`） | 可 |
| `ctrl-muhenkan` | Ctrl+無変換 で IME-OFF → ASCII リテラル通過（`abc`） | 可 |
| `katakana`      | カタカナ切替（`カタカナ`） | 可 |
| `tray-toggle`   | 言語バー/トレイからマウスで IME 切替 | **不可（目視）** |

## 自動セルと対話セル

各セルは `App.Readable && Scenario.AutoCapable` のとき **Auto**、それ以外は **Manual**。

- **Auto**: スクリプトが対象ウィンドウへ NICOLA キー列を SendInput で注入し
  （`crates/awase-windows/tests/e2e_windows.rs` の SendInput ヘルパと同じく、物理
  スキャンコードで awase フックを通す）、テキストを読み戻して期待値と文字列比較する。
- **Manual**: 目視でしか判定できないセル（トレイ切替のモード表示、GJI 候補ウィンドウ、
  カタカナモードバナー等）。スクリプトが手順を表示し、`p/f/s`（pass/fail/skip）を対話入力させる。

> **現状の実装レベル:** Auto セルの「フレーズ→スキャンコード列（親指シフト同時打鍵含む）」の
> マッピングと、クリップボード読み戻しの結線は、実機の `layout/nicola.yab` に対してしか
> 検証できないため **雛形（scaffold）** の状態にある。結線されるまで Auto セルは
> 目視確認にフォールバックし、**結果を捏造しない**（PASS を勝手に埋めない）設計になっている。
> Windows 側で読み戻しを結線する際は `Invoke-AutoCell` / `Send-NicolaSequence` を実装する。

## 実行方法

### 1. ドライラン（既定・安全）

キーを一切送らず、ウィンドウにも触れず、マトリクスを表示するだけ。

```powershell
.\scripts\smoke-matrix.ps1
```

`-Apps` / `-Imes` / `-Scenarios` で絞り込める（下記）。**まずこれで計画を確認する。**

### 2. 実機スイープ（`-Execute`）

実際にデスクトップを駆動する。**awase が起動中**（フック対象）で、各対象アプリが
開いていて、想定の IME が選択されている必要がある。

```powershell
# 全セル
.\scripts\smoke-matrix.ps1 -Execute

# warmup 変更後に Chrome/WezTerm の cold-start だけ再確認
.\scripts\smoke-matrix.ps1 -Execute -Apps chrome,wezterm -Scenarios cold-start
```

### Windows 実機への配送

このスクリプトは Linux 側で編集し、`clipwire-exec` / `awase-build` スキル経由で
Windows 実機に配送・実行する運用を想定している。`clipwire-exec` の承認ゲート
（Windows 側でユーザーが手動 approve）を必ず通すこと。**Claude が勝手に実機で
`-Execute` を走らせてはいけない** — 実機スイープの起動は必ず人間が行う。

## 運用ルール（いつ・誰が）

- **バージョンバンプ前（`Cargo.toml` の version を上げるコミットの前）に、warmup /
  cold-start / IME 制御に触れた変更が含まれる場合は、影響アプリのセルをスイープする。**
  全アプリを毎回回すのが理想だが、最低でも変更が触れた配送方式（VkBatched なら
  Chrome+Edge+VSCode、TSF native なら WezTerm、ImmCross なら LINE）のセルは必須。
- **退行が疑われる修正のレビュー時**は、修正したセルだけでなく **隣接セル**
  （同一アプリの他シナリオ、他アプリの同一シナリオ）も回す。137 件の退行はここで漏れた。
- スイープは実機を触れる担当者（ユーザー本人）が行う。CI では代替できない
  （SendInput はフォアグラウンドフォーカスを要求し、GitHub Actions では届かない）。

## 結果の記録

`-Execute` 実行後、`logs/smoke/` に 2 ファイルが出力される（`logs/` は `.gitignore` 想定）。

- `smoke_YYYYMMDD_HHmmss.md` — Markdown 表（App / IME / Scenario / Verify / Result / Note）と
  合計（PASS / FAIL / SKIP 件数）。
- `smoke_YYYYMMDD_HHmmss.json` — 機械可読な結果（後日の突き合わせ用）。

**記録の扱い:**

- FAIL があるバージョンはリリースしない。FAIL セルの `Note` に症状（例: `toいう`）を残す。
- 既知バグに該当する FAIL は `docs/known-bugs.md` の該当 BUG 番号と対応付ける。
- リリースした版の md 結果はリリースノート/PR に添付し、「どのセルを確認したか」を残す。
