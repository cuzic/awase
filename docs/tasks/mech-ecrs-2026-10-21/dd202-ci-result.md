# DD2-02 の CI 検証結果(IdleCheck 経由の fence 比較)

2026-10-22。`dd2-stale.md` の DD2-02(`ConvModeMgr::observe` の fence 比較は呼び出し元 2 か所で常に一致する)を、GitHub の Windows CI で確かめた。
**「不一致 0 件」は証明ではなく証拠である**(この構成が触れない経路は、0 件でも起きないとは言えない)。

## 結論

- **IdleCheck に到達した**: `observe source=IdleCheck` が 1373 件(48 ジョブ)。**fence 不一致 0 件**(`fence_match=false` 0、`stale-read_at` 0)。
- 前回(`probe-reach-result.md`)の FocusCheck 13 件(不一致 0)と合わせて、呼び出し元 2 か所とも e2e では常に一致した。コードの読み(同じ `with_app` の同期フレーム内で `admit` が fence を作る)と矛盾しない。
- 撤去・ADR は作っていない(所有者の判断)。

## 到達できた理由(前回 0 件だった原因)

`should_run_idle_conv_check` は TsfNative を要する(`is_effectively_tsf_native`、窓クラスが `CASCADIA_HOSTING_WINDOW_CLASS`・`Windows.UI.Input.InputSite.WindowClass` 等)。
既存の `tsf` 形は Chrome の窓クラスなので TsfNative にならない。
新しい入力先 `--form=tsfnative`(RichEdit を Windows Terminal の窓クラス名でスーパークラス化した自前窓)で到達した。

## 方法

- ブランチ `ci/dd202-reach`(`f4bcce92`、挙動不変)。`observe` の目印(`ci/probe-reach` の DD2-02 分を develop `05c4c05b` に載せ直し)、`kp_stage_idle_conv_check_inner` の `idle-gate` 目印(数値と真偽値のみ)、
  `typing_stress --form=tsfnative`、`e2e-ime.yml` の `tsx-tsfnative-*`(GJI/MS-IME × 40/10ms × {連続, 試行前 1.5s 停止, 12s 停止, 5 文字目で 800ms 停止} × 3 回 = 48 ジョブ)。
- run: `gh run view 39791975407`(`only=tsx-tsfnative-*`、success)。集計: `gh run download 39791975407 -p 'result-*'` → `grep -rah reach-probe --include=awase.log`。

## 集計(run 39791975407)

| 目印 | 件数 |
|---|---|
| `DD2-02 observe source=IdleCheck fence_match=true` | 1373(各ジョブ 25〜39 件) |
| `fence_match=false`・`stale-read_at` | **0** |
| `idle-gate tsf_native=true`(KeyDown、`mode_key=false`) | 17967 |
| `idle-gate`(`mode_key=true`、ガード 5 で止まる) | 5712 |

## 限界(未確認)

- 実アプリ(実 Windows Terminal・WezTerm)ではない。窓クラスだけを TsfNative にした自前窓。
- spawn から完了までの間にフォーカスが変わる状況(窓切替直後の遅い conv 読み)は作っていない。fence 比較が実際に効くのはその状況なので、**「冗長」の証拠は「その状況が起きなかった」ことに依存する**(`--switch-focus` との組み合わせは未実施)。
- 1373 件は同一の runner・同一の窓の組。
