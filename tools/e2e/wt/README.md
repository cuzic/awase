# Windows Terminal 制御語彙(tools/e2e/wt/)

BUG-113/121/142(Windows Terminal + GJI、物理半角/全角キーで「@」が出る・IME ON/Engine ON に固着)を CI の実機で再現するための部品。
Python(ctypes)+ PowerShell だけで、Rust のビルドは要らない(awase 本体だけ debug ビルド)。

| ファイル | 役割 |
|---|---|
| `wt_vocab.py` | 窓の列挙・起動(`wt -w new new-tab …`)・前面化・閉じる、SendInput(scan 付き、`marker=True` で debug awase が物理キー扱い)、Unicode 直接送信、新規タブ/分割/ペインを閉じる、UIA 読み取り、スクリーンショット |
| `wt_echo.ps1` | Windows Terminal の中で動く記録器。受け取ったキーの文字コードを 1 行ずつファイルへ(IME 確定のかな・余計な「@」・ローマ字リテラルを区別) |
| `wt_uia.ps1` | UIA の TextPattern で画面のテキスト、タブ数を読む |
| `wt_pure.py` + `test_wt_pure.py` | echo ログの解析と分類(OS 非依存、Linux で unittest) |
| `wt_probe.py` | 相 V(可否表)・I(IME 越しの打鍵)・B(BUG-113 再現)・N(awase なしの対照)。判定は付けず観測だけ |

使い方: `python tools/e2e/wt/wt_probe.py --dist dist --out out --phases V,I,B,N --presses 10`
実機 run: `ci/wt-probe` への push で `.github/workflows/wt-probe.yml` が動く(結果は artifact `wt-probe-out` の `summary.md`)。
