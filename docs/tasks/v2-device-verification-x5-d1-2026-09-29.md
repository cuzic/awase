---
title: v2 実機確認 X5・D1 の実施結果（2026-09-29/30、clipwire 経由）
status: 一部実施。X5 は WM_IME_CONTROL 経路が追随なし 20/20。注入キー経路は追随ログを確認したが、打鍵の結果は前面の切り替わりで汚染され判定不能。D1 は未実施。実施中に他のウィンドウへ注入が漏れたため中止
created: 2026-09-30
related_adr: ["ADR-205", "ADR-191"]
---

# v2 実機確認 X5・D1 の実施結果

手順書 [v2-manual-verification-guide-2026-09-29.md](v2-manual-verification-guide-2026-09-29.md) の X5（BUG-172）と D1（BUG-163）を、所有者の許可（2026-09-29）で clipwire 経由で実機に対して実施した記録。
許可範囲は X5 と D1 のみ。実機のレジストリ・IME 設定・config.toml は変更していない。物理キーが要る項目（D2・X1〜X4・D3）には触れていない。

## 結論

| 項目 | 判定 | 要点 |
|---|---|---|
| X5 手順 2〜4（他プロセス注入のキーで閉じる → 追随） | **追随ログは確認、打鍵結果は判定不能** | 目印なし SendInput の 0x1A / 0xF3 で IME が閉じた 4 試行で `[external-change] 追随` が出た（下記「X5 注入キー経路」）。ただし閉じた後の `k` `a` は、前面が Windows Terminal・タスクバー・タスクマネージャへ切り替わっていて Chrome に届かず、`ka` かどうかは判定できない |
| X5（参考: `WM_IME_CONTROL` で閉じる経路） | **追随なし 20/20（`kiu`）** | GJI × 実 Chrome で、閉じて 3 秒後の `k` `a` が `kiu`。期待（追随して `ka`）と異なる。ただしこれは手順書の経路ではない |
| X5 手順 5・9・10（モードキーで戻す、偽 OFF、MS-IME） | **未確認** | 物理キーが必要、または注入経路が取れなかったため |
| D1（起動直後の強制 ON が無い・最初の打鍵が欠落しない） | **未実施** | 実施用のモード（`--d1`）は用意したが、X5 の途中で前面の奪い合いが判明し中止した |

`observed`（awase が対象状態を観測した件数）の扱い: X5 の有効 20 試行では `[external-change]` が 0 件、`[drift] correction` が 0 件、`observer_poll`/`ObserverReported` は 1 ログあたり 2 件だった。**「追随しなかった」ことは示すが、awase がなぜ追随しなかったかは、この測定では分からない**（注入経路が違うため、ADR-205 の想定した窓が開いていない可能性を排除できない）。

## 実機の環境

- OS: Windows 11 (10.0.22631)、ホスト `dragonflyg4`、ユーザー `cuzic`、セッション 1（console）。
- IME: Google 日本語入力 と Microsoft IME が導入済み（ja の TIP 2 つ）。テスト時の awase の判定は `initial IME kind: GoogleJapaneseInput`、Chrome ウィンドウの profile は `Imm32Unavailable`（`AppKind` は `TsfNative`）。
- テスト用 awase: `origin/develop` の `8fe78f34`（`C:\Users\cuzic\awase-dv` の detached worktree、debug ビルド）。exe の隣に config.toml が無いため **既定 config**。`RUST_LOG=debug`・`AWASE_TEST_INJECTION=1` で起動。バージョン表示は `1.21.0`（コミットの証拠にならない）。
- プローブ: 同コミットの `examples/chrome_probe`（専用プロファイルの実 Chrome を起動し、ローカルページの入力欄に `k` `a` を打って出力で状態を判定する）。実機確認用の未適用パッチ [chrome_probe-close-key.patch](../../tools/e2e/ime_key_matrix/patches/chrome_probe-close-key.patch)（`--close-key=VK`: 目印なし SendInput で閉じる、`--then-chord`、`--d1`、前面化フォールバック）を実機の worktree にだけ当てた。**有効データを取った回のバイナリにはこのパッチが入っていなかった**（下記）。Chrome のバージョンは採取していない。
- 所有者の awase（作業前の状態）: PID 29116、`target/debug/awase.exe`、cwd = `C:\Users\cuzic\scoop\persist\msys2\home\cuzic\awase`、config = 同 `config.toml`。チェックアウトは `feat/e2e-uwp-inputsite-hook-watchdog`（`e174c6f6`、作業ツリーは未変更）。

## X5: 有効だったデータ（JST 2026-09-30 08:32〜08:34、画面アンロック中）

- 手順（chrome_probe `--close-ime=10 --settle=800`）: `VK_IME_ON`（目印付き注入 = 物理相当）で ON にそろえて `k` `a` → `きう`（Engine ON・IME ON、`Process(229)=true`）を確認 → **`WM_IME_CONTROL`（`IMC_SETOPENSTATUS`=0）で IME を閉じる**（`open_before=1, open_after=0`）→ 3 秒待つ → `k` `a`。
- 手順書の「他プロセスの SendInput（0x1A / 0xF3）」ではない。パッチの適用に失敗していたため（原因: パッチファイルに自己参照の diff が混入していた。修正済み）、`x5-1a`（0x1A のつもり）と `x5-f3`（0xF3 のつもり）の 2 セットはどちらも `WM_IME_CONTROL` 経路になった。
- 結果: 10 回 × 2 セット = 20 回すべて `kiu`（IME は閉じたまま、Engine は ON のまま）。setup（`きう`）は 20 回とも成功。
- awase.log（各セット）: `[external-change]` 0 件、`[drift] correction` 0 件、`gji fsm StartComposition while engine off` 0 件、`[startup-align] desired=true` 1 件。
- 3 本目（`WM_IME_CONTROL` のみの対照）は、1 試行目が空、以降がロックで INVALID（有効 0）。

解釈の注意: 手順書の期待は「追随して `ka`」。ここで出た `kiu` は、`WM_IME_CONTROL` で閉じた IME に awase が追随しなかったことを示す。ADR-205 が追随対象にするのは「他プロセスが注入したキーで閉じた」場合（監視窓 300ms）であり、`WM_IME_CONTROL` は同じ扱いにならない可能性がある。したがって **この 20/20 は BUG-172 の未修正の証拠とは断定しない**。手順書どおりの経路の確認は残っている。

## X5 注入キー経路（JST 08:55〜08:57、部分的に有効）

実機の `chrome_probe --close-ime=10 --close-key=1A|F3 --then-chord=A2,1C`（パッチ適用済みバイナリ、目印なし SendInput で閉じる。パッチ後の有効データはここだけ）。

| セット | 有効な setup（`きう`） | 実際に閉じた（API `open_after=0`） | `[external-change]` |
|---|---|---|---|
| 0x1A（`x5b-1a`） | 2 回（試行 6・7。他の試行は画面ロックで INVALID） | 1 回（試行 7 は `open_after=1` で閉じていない） | 2 件 |
| 0xF3（`x5b-f3`） | 3 回（試行 1〜3。以降はロックで INVALID） | 2 回（試行 3 は `open_after=1`） | 2 件（注入と同じ時刻: 08:56:59.49、08:57:06.40） |

- 「閉じた」と API が示した試行では、awase が `[external-change] 監視窓の中で開閉の読みが変わった → 実状態 open=false へ追随（意図を捨て desired を揃える。awase は IME を書かない）` を出した。ADR-205 の追随そのものは、この実機・GJI・実 Chrome で**働いている**。`[drift] correction` の発火（awase が開け直した）の有無は、このログの範囲では確認していない。
- 期待は追随後の `ka` だが、閉じた後の `k` `a` は 6 試行すべて空だった。原因は前面の切り替わり: テスト awase の `[focus-sync]` に、閉じた直後から `CASCADIA_HOSTING_WINDOW_CLASS`（Windows Terminal）、`Shell_TrayWnd`、`TaskManagerWindow`・`DirectUIHWND` へのフォーカス移動が記録されている（08:57:00、08:57:06、08:57:09、08:57:19 など）。**Chrome の検証ページが前面から外れていたので、`ka` の判定はできない**。`--then-chord`（Ctrl+変換）の後の打鍵も同じ理由で判定できない。
- 前面を奪っているのが誰か（所有者、または同じ実機を clipwire で使う他のセッション）は特定できていない。

## 中止と、注入が他のウィンドウに漏れた件

08:57 前後、前面が Windows Terminal・タスクバー・タスクマネージャに切り替わった状態で、`chrome_probe` が `k`・`a`・`VK_IME_ON`(0x16)・`F2`・`0xF3`/`0x1A`・`Ctrl+変換` を SendInput で注入し続けた。これらが**所有者の Windows Terminal やタスクマネージャに入った可能性が高い**（内容は読んでいない）。テスト awase のログでは、Windows Terminal がフォーカスを得た時点で awase 自身が `VK_IME_ON`（warmup、`self_injected=true`）も注入している。
私は 08:58 頃にセッションを停止し、所有者の awase を復元した（下記）。停止の際に、実行中だった `x5b-f3` の子プロセスの後始末が漏れ、テスト awase（PID 25456）が約 1 分残った。復元スクリプトで停止した。

## 無効だった試行（正直な記録）

実機が短周期で画面ロックされ（前面が `LockApp`、フォーカスが `LockScreenControllerProxyWindow`、`OpenInputDesktop` 失敗）、ロック中は SendInput がどのアプリにも届かない。全試行で setup の `k` `a` が空になり INVALID。

| 時刻 (JST) | 内容 | 結果 |
|---|---|---|
| 01:05〜01:09 | 前面化失敗の予備試行（4 回、`前面化: false`）。全 INVALID | 下記「事故の確認」 |
| 01:33〜01:34 | 5 分無操作を待ってから自動実行（3 シナリオ × 10）。画面ロック中 | 全 INVALID（10/10 × 3） |
| 08:32〜08:35 | 上記の有効データ + 3 本目は途中でロック | 20 有効 + 9 INVALID |
| 08:38 / 08:40 / 08:43 | パッチ適用後の再実行（x5b）。開始直後にロック | 全 INVALID |

ロックの周期: アンロックは 08:32・08:38・08:40・08:42 に確認し、いずれも開始の 60〜210 秒前に人の入力があった。ロックは実行開始の 12〜45 秒後に入る。08:32 の 1 回だけはアンロックのまま 3 分以上続いた。「席を外すとロック、戻るとアンロック」に見えるが、原因は未確認。**5 分無操作を待つ方式は、ロックが入る時間と重なるため機能しない**（この教訓は `x5-run.ps1` のロック判定に反映済み）。

## 事故の確認（予備試行のキーが所有者の前面ウィンドウに入ったか）

- 01:05〜01:09 頃の予備試行（`chrome_probe` が前面化に失敗）で `k` / `a` / `VK_IME_ON` / `F2`（ひらがな）を SendInput で注入した。この間の awase.log（テスト awase）は、次の実行が開始時に削除したため**残っていない**。所有者の awase は当時停止しており、記録がない。
- したがって前面ウィンドウのクラス名・injected の記録での確認は**できなかった**。**所有者の前面ウィンドウ（Windows Terminal 等）に入った可能性は否定できない**（画面がロック中だったなら届いていない。当時の `GetLastInputInfo` の idle は 08:00 台の別実行では 33 秒〜333 秒）。ウィンドウの内容は読んでいない。
- 01:33 以降の自動実行 3 回は、テスト awase のログで前面が `LockScreenControllerProxyWindow`（ロック画面）だったので、注入キーは所有者のウィンドウに入っていない。

## 作業後の復元と後始末

- 所有者の awase: テスト実行のたびに停止・再起動を繰り返した（PID 29116 → 19916 → 31132 → 13228 → 26180 → 38252 → 39172 → 17984 → …）。**最終状態は `C:\Users\cuzic\scoop\persist\msys2\home\cuzic\awase\target\debug\awase.exe`（絶対パス指定）、cwd = 同リポジトリ直下、awase 1 プロセスのみ**。起動ログの `Loading config from` は元の起動と同じ `...\awase\config.toml`。元のコマンドラインは相対パス `target/debug/awase.exe` で、`RUST_LOG` などの環境変数の有無は元の起動時の値が分からず、未設定で再起動している。
- テスト用プロセス（awase-dv の awase、`chrome_probe`、`chrome_probe_profile` の Chrome）は残っていない。専用プロファイルの Chrome ウィンドウも閉じた。
- 実機に残っているもの: `C:\Users\cuzic\awase-dv`（git worktree、実機のリポジトリの `.git/worktrees` に登録）、`C:\Users\cuzic\dv-out\`（ログ、README.txt）、`dv-x5.ps1`・`dv-x5-session.ps1`・`dv-patch*.patch`。除去手順と awase の手動復元コマンドは実機の `C:\Users\cuzic\dv-out\README.txt`（`tools/e2e/ime_key_matrix/device/x5-README.txt` と同じ）。
- 変更していないもの: レジストリ、IME 設定、`config.toml`、実機の作業ツリーのブランチ（`feat/e2e-uwp-inputsite-hook-watchdog` のまま）。

## 未確認（次に必要なこと）

1. 手順書どおりの経路（他プロセスの SendInput 0x1A / 0xF3 で閉じる）での X5 手順 2〜4。パッチ適用・ビルドは実機で済んでいる。
2. D1（IME ON で起動 / OFF で起動）。`chrome_probe --d1=N --d1-state=on|off --awase-exe=... --awase-cwd=...` を実機のバイナリに入れてある。
3. X5 手順 5（モードキーで戻す）は物理キーが必要（注入では BUG-14 の方針で意図に昇格しない）。手順 9・10（偽 OFF、MS-IME）も未確認。
4. 上記を有効にするには、**画面がアンロックされたまま、所有者が 10 分ほど実機に触れない**ことが必要。再開は実機の `dv-x5-session.ps1`（または repo の `tools/e2e/ime_key_matrix/device/x5-session.ps1`）を起動する。
