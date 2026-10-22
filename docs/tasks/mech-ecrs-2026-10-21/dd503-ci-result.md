# DD5-03 の GitHub Windows CI での検証結果(2026-10-22)

結論: **再現した。ただし経路が限られる。** 実 Chrome(`Chrome_WidgetWin_1`、GJI)で、押下に由来しない ON(`SetOpen{press: None}`)が
`AlreadyMatched` で省かれ、実 IME が閉じたまま残る。Engine ON コンボ(Ctrl+Shift+変換)では起きない。

構成の置き場: ブランチ `ci/dd503-repro`(`e2e-ime.yml` の `sc-keymatrix-dd503-*`、`chrome_probe.rs` の `--keymatrix` に gap `engoff`/`engoffnr`/`hkoff`/`hknr`/`hkcl`/`hkclnr`/`hkfresh` を追加、`check_keymatrix.py` は `ctrl+shift+` を読めるようにした。観測のみ、本番の挙動は不変)。
dispatch: `gh workflow run e2e-ime.yml --ref ci/dd503-repro -f only='sc-keymatrix-dd503-*'`。ログは artifact の `result-*/dist/awase.log`。

## 結果(run ID と、確かめたコマンド)

| 経路 | run | 結果 | 確かめた方法 |
|---|---|---|---|
| Engine ON コンボ(`ctrl+shift+1c`)、Engine OFF コンボ → フォーカス往復(待ち 0/32 秒)→ ON、および往復なしの対照 | 39797924449 | 全 24 試行が 1 押下で実 IME ON(`CONVERGED_1`)。`[set-open]` 48 行のうち `press=None` は 0 行、ON は全て `press=Some(..) outcome=Applied` | `result.txt`、`grep "\[set-open\]" result-*/dist/awase.log \| grep -c press=None` = 0 |
| `general.engine_toggle_hotkey`(Ctrl+Shift+F12、WM_HOTKEY → `toggle_engine`)で OFF → ON | 39799674066 | OFF は `press=None outcome=Unwarranted`(書かれない)で実 IME は開いたまま、続く ON は `AlreadyMatched`。実 IME が開いているので害は無い(前提 `r0` が成立せず `GAP_NOT_MADE`) | `result.txt`、`awase.log` の `[set-open]` |
| 同上 + OFF の後で実 IME を外から閉じる(`hkcl`)、往復なしの対照(`hkclnr`) | 39801179398(`w32000` のジョブ。`w0` のジョブは完走せず INVALID) | `hkcl` 6/6・`hkclnr` 6/6 が `STUCK`(3 押下しても実 IME 閉のまま)。ON は全て `press=None outcome=AlreadyMatched` | `result.txt`(`KEYMATRIX_CELL ... stuck=6`) |
| **awase を再起動して実 IME を閉じた状態から**、フォーカス往復 → ホットキー OFF → ホットキー ON(`hkfresh`) | 39806707217 | 有効 3 試行すべて `STUCK`(実 IME 閉、`pre_api=false`)。同じジョブの `hkcl` は有効 5 試行すべて `STUCK` | `result.txt`、`chrome_probe.log` の `KM {"cell":"..hkfresh"..}` |

`hkfresh` の `awase.log`(`result-sc-keymatrix-dd503-chrome-hk-gji-w0-1/dist/awase.log`):
再起動で `applied` は未知 → 1 回目のフォーカスは新規スレッドで `new-thread entry ... 安全デフォルト OFF` → 2 回目(`SeenBefore`)のフォーカスで
`[focus] Imm32Unavailable hard pre-sync applied=true` → ホットキー OFF は `Unwarranted` → ホットキー ON は `[apply-ime] open=true → outcome=AlreadyMatched`、
実 IME は閉じたまま。**明示の ON を一度も書いていない awase で `applied=Confirmed(true)` ができるのは、フォーカス時の写し(`presync_applied_open_on` と `ir_post_focus_change_snapshot` の `record_confirmed`)だけ**。DD5-03 の場面そのもの。

## 何が分かったか

1. **Engine ON コンボは安全**: `src/engine/engine.rs:1160` の `decision.stamp_set_open_press(event.press_id)` が、コンボの `SetOpen` に押下 ID を載せ、
   `explicit_press_applied_pair`(ADR-208 D1)が `applied` を未知にするため省かれない。dd5-recovery.md の「EngineOn コンボで `press=None`」は誤りだった。
2. **害が出るのは `press=None` が実際に残る経路**: `runtime/mod.rs:1034 toggle_engine`(トレイ・`general.engine_toggle_hotkey`)と `:1053 force_engine_on`(トレイの「状態をリセット」)。
   既定では `engine_toggle_hotkey` が無いので、実利用ではトレイ操作と、設定したホットキーだけ。
3. **写しを全廃(案 B)しても直らない場面が別にある**: `hkclnr`(フォーカス往復なし)も 6/6 固着した。明示 ON で付いた `applied=Confirmed(true)` が、
   `press=None` の OFF が `Unwarranted` で書かれないので残り、実 IME が外から閉じても ON が省かれる(ADR-208 S-1 の `press=None` 版)。
   つまり原因は「写し」だけでなく **`press=None` の ON を `applied` で省く判断そのもの**。

## 修正案の比較(PR にはしない。所有者の判断)

- **案 A(柵): ユーザー操作起点の `press=None`(`ToggleEngine`・`ForceEngineOn` の ON)は `applied` を未知として扱い、省かない**。`dispatch_set_open` に入る前に、
  トレイ・ホットキー起点であることを示す印(合成の押下、または `SetOpenRequest` の 1 項目)を載せる。`hkfresh`・`hkcl`・`hkclnr` の 3 つを同時に直す。
  drift correction の `press=None`(省略が正しい)は印を付けず従来のまま。**こちらを推す。**
- **案 B(写しの全廃)**: `hkfresh` だけ直る。`hkcl`/`hkclnr` は残る。`applied_mirror_plan.rs` 等が消える利点はあるが、この害には足りない。
- 回帰テストの置き場: 核の判断は `profile_skew.rs` の DD5-03 テストを、`press=None`(ユーザー操作)で `applied=Some(true)` のとき書かれる形に直す。実機は `sc-keymatrix-dd503-chrome-hk-*`(`hkfresh`・`hkcl` が `CONVERGED_1` になること)。

## 未確認

- 実利用の journal で、静的 `Imm32Unavailable` × GJI の `press=None` ON の `AlreadyMatched` が何件あるか(dd5-recovery.md の「決める材料」)。
- MS-IME・Edge・他の静的 `Imm32Unavailable` 窓。今回は GJI × Chrome だけ。
- `w0` のジョブ(run 39801179398)は完走せず INVALID(原因は未調査)。結論は `w32000` のジョブと run 39806707217 に拠る。
- トレイ操作(`toggle_engine`・`force_engine_on`)そのもの。ホットキーは同じ `toggle_engine` を呼ぶので同じ経路だが、トレイメニューは CI で押していない。
