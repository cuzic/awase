# ADR-276 後の未確認入力先の CI 確認結果(2026-10-22)

確認: `gh workflow run e2e-ime.yml --ref develop -f only='tsx-ext-winui3-textbox-*,tsx-ext-wpf-*,tsx-ext-webview2-*,tsx-ext-notepadpp-*,tsx-ext-flutter-*,tsx-ext-electron-*,tsx-raw-ext-flutter-*'`
-> run 39802460934(head `964ea213`、develop)。e2e ジョブ 54 本(plan・build・summary を除く)すべて success、全ジョブから `TYPING_STRESS: verdict=` を取得(取得できなかったジョブ 0)。

## 結果

打鍵間隔はすべて 20ms(forms.toml の `intervals = [20]`)。各ジョブは 12 試行(single/thumb/mixed 各 4。raw 対照は single 4)。
「ジョブ数」は `runs` ぶんの別ジョブ。全ジョブで fail=loss=extra=reorder=literal=substitute=0。

| 入力先 | GJI | MS-IME |
| --- | --- | --- |
| WinUI3 TextBox | PASS (1) | PASS (1) |
| WPF TextBox | PASS (2) | PASS (2) |
| WPF TextBox(複数行) | PASS (2) | PASS (2) |
| WebView2 textarea | PASS (2) | PASS (2) |
| Notepad++ | PASS (2) | PASS (2) |
| Flutter TextField | PASS (8) | PASS (8) |
| Electron textarea | PASS (1) | PASS (1) |
| Electron input | PASS (1) | PASS (1) |
| Flutter(awase なしの対照 raw) | PASS (8) | PASS (8) |

検証不能の入力先: なし(setup 失敗・窓が見つからないジョブは 0)。FAIL・literal: なし。そのため VK 化(ADR-276)前後の比較(`ci/forms-verify-before`)は不要で、実施していない。

## 注意

- 確認したのは 20ms 間隔・12 試行(Flutter は 8 ジョブ)の観測。他の間隔・長時間の頻度は未確認。
- `inject_late_max_us` は最大でも約 1.7ms(Flutter GJI の 1 ジョブ 1714us)で、注入の遅れによる影響は見られなかった。
- 確認コマンド: `gh api "repos/cuzic/awase/actions/runs/39802460934/jobs?per_page=100" --paginate`(54 ジョブ)と各ジョブの `actions/jobs/<id>/logs`。`gh run view --json jobs` は 30 件で打ち切られる。
