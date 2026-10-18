# ADR-265 段階 2 の実機測定の道具

[docs/tasks/adr265-stage2-measurement.md](../../docs/tasks/adr265-stage2-measurement.md)(手順書)と
[docs/tasks/adr265-stage2-measurement-result.md](../../docs/tasks/adr265-stage2-measurement-result.md)(2026-10-18 の結果)の道具。

- `drive_measure.py`(Windows、標準ライブラリのみ): **PID で指定した新しいプロセス**の窓だけにキーを送る(ユーザーの
  ブラウザーなどには触れない)。各キーは 1 回だけ送信し、再送は準備段階だけ。フォーカスを持つスレッドの共有メモリ
  (mozc-readable)を直接読んで、操作から決まる期待値と並べて JSON Lines に記録する。**窓の中央のクリックは既定でしない**
  (大きな窓では、ページの入力欄のフォーカスを外して IME が反応しなくなった。`--click` で有効)。
- `measure.ps1`(Windows): Notepad と、専用プロファイルの新しい Chrome で `drive_measure.py` を 25 周ずつ走らせる。
  Chrome のパスは測定した実機(scoop)のもの。他の環境では書き換える。
- `analyze.py`(Linux 可): `drive_*.jsonl` と、awase の `awase.log` の `[mozc-shm]` の行を時刻で突き合わせ、
  受け入れた読みの食い違い・別スレッドを読んだ記録を数える。`python3 -I analyze.py <ディレクトリ>`。
  awase のモード名はハイフン区切り(`full-katakana`)で、正規化してから比べる(初版はここを誤って偽の食い違いを出した)。

前提: awase を `[diagnostics] mozc_shm_shadow = true` で起動し、Mozc Readable をインストールして、ja の入力方式を
Mozc Readable だけにしておく(新しい窓がそれで始まる)。**測定中は実機を使えない**(キー注入とフォーカス移動)。
実機で行った操作と後片付けは、結果の文書にある。
