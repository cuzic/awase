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

## 追加測定(危ない場面。手順書の 6-2 章)

- `drive_scenarios.py`(Windows。`drive_measure.py` を import する): `--scenario focus`(2 つの窓の間のフォーカス移動)と `--scenario away`
  (Win+Space で TIP の無い入力方式へ切り替えて戻す)。
- `scenarios.ps1`(Windows): 新しい Notepad と専用プロファイルの Chrome で、上の 2 つを走らせる。
- `analyze_scenarios.py`(Linux 可): `scen_focus.jsonl`・`scen_away.jsonl` と `awase_measure.log` を突き合わせ、誤採用(MISMATCH・UNKNOWN-THREAD・
  STALE・STALE-ADOPTION)を数える。判定は手順書で測定の前に固定した。合成データで、仕込んだ誤りの検出を確認済み。**ドライバーは Windows では未実行**。

## 物理キーでの測定(採用の利益。結果の文書の「物理キーでの測定」)

- `drive_physical.py`(Windows。`drive_measure.py` と `drive_scenarios.py` を import する): 利用者が物理キーで IME のモードを切り替える間、**IME のモードキーの押下だけ**
  (キーボードフック。ほかのキーは記録も数えもしない)と、フォーカスのスレッドの共有メモリの変化を記録する。文字は記録しない。
- `physical.ps1`(Windows): 新しい Notepad を開いて 420 秒記録する。測定の後は、記録(キーの種類と時刻を含む)を実機から削除すること。
- `analyze_physical.py`(Linux 可): 共有メモリの開閉の変化ごとに、awase がその新しい状態を読んだ時点の belief(`belief_open`)が既に正しかったかを数える。
  比べられるのは開閉だけ。注入されたキー(awase が物理キーを送り直した記録)は、原因のキーとして数えない。合格基準は置かない。
