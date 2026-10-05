# 環境イベント probe(再現用の語彙: キー入力以外の環境の変化)

`probe.py`(ctypes のみ、Rust ビルド不要)が、CI の windows-latest 実機で次のイベントを起こす。`run.py` が awase 起動中の awase.log の反応を抜粋する。
動かす: `ci/env-events-probe` へ push(`.github/workflows/env-events-probe.yml`)。単体: `python probe.py --events=tab,minrestore,vdesk[,lock] --marker=1`。

## 実機での可否(run 37275688147、2026-10-05、windows-latest)

| イベント | 起こせたか | 証拠(probe.log の自窓の状態) | awase.log の反応 |
|---|---|---|---|
| Win32 ダイアログ相当の Tab 移動 | 可 | focus_idx 0→1(Tab)→0(Shift+Tab)。注入 Tab は `[engine-input] vk=0x09 … extra=0x5350494B`→PassThrough | 子 EDIT 間のフォーカス移動ごとに `[focus-sync]`(0x10224→0x10226→0x10224)と FocusProbe/ImmCrossProbe(BUG-091 の入口) |
| 最小化/復元 | 可 | iconic=true で前面が `CASCADIA_HOSTING_WINDOW_CLASS`(Windows Terminal)に移り、復元で自窓に戻る | `[focus-sync]` が CASCADIA→DesktopWindowContentBridge→InputSite(app_kind=Uwp)→自窓と遷移 |
| 仮想デスクトップ切替 | 可 | IVirtualDesktopManager で on_current_vd true→false(Win+Ctrl+D、前面=Progman)→true(Win+Ctrl+F4)。GUID も取得 | `[focus-sync]` VirtualDesktopHotkeySwitcher→Progman→SHELLDLL_DefView |
| 画面ロック | 可(解除は不可・最後に回す) | LockWorkStation 後、OpenInputDesktop が 10 秒間 access denied(5)、SendInput が false、前面窓なし | キー入力ゼロ。`Hook watchdog: no activity for 15015ms` のみ(ロック中は awase に何も届かない) |
| スリープ復帰 | 未試行(ランナーを止める恐れ。不可の見込み) | - | - |

副産物: ランナーには Windows Terminal が既に動いている(最小化時に前面になった)。BUG-113/121/142 の入力先に使える可能性がある。
`happened` は自窓の状態変化(上の証拠)で判定している。再現の有無(バグが出たか)は見ていない。
