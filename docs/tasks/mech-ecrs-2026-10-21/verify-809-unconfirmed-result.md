# PR #809(Tsf 経路と force_tsf の撤去)で「実機未確認」とした 3 点の CI 測定

2026-10-23。PR #809 の本文「確かめていないこと」と ADR-0004 の追記、レビュー M1・M2 が挙げた 3 点(窓切替直後の打鍵、
Mozc Readable × Windows Terminal のひらがなキー直後〈BUG-209〉、WezTerm)を、GitHub の Windows CI(`e2e-ime`)で測った。
実機(dragonflyg4)は使っていない。入力先はすべて CI の自前の窓・CI が起動した Windows Terminal/WezTerm/Chrome。

## 結論

| 点 | 判定 | 要点 |
|---|---|---|
| M1 窓切替直後の打鍵 | **問題あり(#809 で悪くなった)** | 前面が戻ってから **0〜30ms** に打った最初の 1 打鍵が、前の窓の IME 状態で処理される。60ms 以降は撤去前後とも 0 件。撤去前(TsfGate あり)は同じ打鍵で 0 件 |
| M2 Mozc × WT のひらがなキー直後 | **問題なし(配線)/リテラルそのものは CI では判定不能** | ひらがなキーのたびに ADR-270 の確認が張られる(撤去前と同数)。CI の Mozc はリテラルを起こさない(確認は全件 15〜16ms で入力中の文字を確認) |
| WezTerm | **問題なし(CI で入れられた)** | ポータブル版(版とハッシュ固定)が `front_end='WebGpu'` で動く。WT と同じ打鍵列で Vk 経路が動き、撤去前後で差なし |

## M1: 窓切替直後の打鍵(問題あり)

**方法:** `typing_stress --sw-mode=b-off|b-on --sw-delays=0_30_60_100_150_300_500_1500`(検証専用、`ci/` ブランチの `switchime.rs`)。
入力先 A と別プロセスの窓 B(EDIT)を往復する。`b-off` は B で IME を OFF にして A(IME ON のまま)へ戻り、A が前面になったのを
観測してから delay ms 後に 6 文字を 40ms 間隔で打つ(期待=かな)。`b-on` は A を OFF、B を ON にして戻る(期待=素の英字)。
1500ms は対照。delay は試行ごとに巡回(各 delay 4 試行〈b-off〉/2 試行〈b-on〉)。撤去前 = `a1757efd`(#809 の直前)、撤去後 = `bcb9e2dd`
(develop。#809 の後は docs と観測のみの #815)。

**結果(b-off、成功/試行):**

| 入力先 | IME | 撤去前 | 撤去後 | 撤去後の崩れ方 |
|---|---|---|---|---|
| 自前の EDIT(Win32) | GJI | 32/32 | 24/32 | delay 0・30ms の 8 試行すべて |
| 同 | MS-IME | 32/32 | 25/32 | delay 0ms 4/4・30ms 3/4 |
| 実 Windows Terminal | GJI | 32/32 | **12/32** | 0・30ms の 8 試行 + **その後の全 mixed 試行**(下記) |
| 同 | MS-IME | 32/32 | 25/32 | 0ms 4/4・30ms 3/4 |
| WezTerm | GJI | 32/32 | **12/32** | WT と同じ形 |
| 同 | MS-IME | 32/32 | 25/32 | 0ms 4/4・30ms 3/4 |

60〜1500ms は撤去後もすべて成功(WT/WezTerm の GJI の持続的な崩れを除く)。`b-on`(A が直接入力)も同じ形: 撤去前 15/15、撤去後 12/15
(0ms 1 件・30ms 2 件。最初の試行は撤去前後とも準備の都合で崩れるので除いた)。撤去前は awase.log の `tsf-gate` の保留が各試行で効いていた
(b-off で 13〜18 件/run)。

**崩れ方(撤去後):** 最初の 1 打鍵だけがエンジン非活性(B の belief)のまま素通しになる。例: 期待 `ふほうほしく` → `ｖほうほしく`、
`こねしこうか` → `ｒねしこうか`(b-off)、`v.a.si` → `fu.a.si` と NICOLA のローマ字が出る(b-on)。順序が入れ替わる例もある
(`r,sraw` → `,srawko`)。awase.log(EDIT・MS-IME、delay 0): フォーカス 12:21:01.807 → 最初のキー 01.832 が `decision="PassThrough"`
→ `Engine activated` 01.867(フォーカスから約 60ms)。CI ではこの約 60ms が保留の無い窓で、実機ではフォーカス検出・refresh が遅いほど広がる。

**持続的な崩れ(WT・WezTerm × GJI):** delay 30ms の mixed 試行で、最初の打鍵が右親指(変換、VK 0x1C)だった。エンジン非活性のまま変換キーが
素通しで GJI に届き、その後の試行(60〜1500ms を含む 14 試行)はすべてローマ字のまま出た(`rtaditiyu` など)。awase は ON のまま
ローマ字を送り続けている。WT・WezTerm は IME 状態を読めない窓(`TSF-native window → ime_on=None`)なので、awase の belief と実 IME の
食い違いが直らない。EDIT(読める窓)では最初の 1 文字だけで済んだ。**素通しになった親指キーが IME の状態を変え、読めない窓では
以後ずっとずれる**ので、1 文字の崩れより重い。

**再現手順(撤去後):** awase 起動、GJI(ATOK 配列)で Windows Terminal に IME ON で入力中 → IME OFF の別アプリへ切り替え →
WT へ戻って 30ms 以内に変換キー(右親指)と文字を同時打鍵 → 以後 WT への入力がローマ字のまま出る(切替をやり直すまで)。

**CI では判定不能だった入力先:** 自前の RichEdit(Chrome クラス、`--form=tsf`)と実 Chrome(`chromepage`)は、撤去前後とも
対照の 1500ms を含む全 delay で崩れた(撤去前 GJI 10/32・MS-IME 6〜7/32、撤去後 8/32・5/32)。戻った後のエンジンが非活性のまま
(全角英字が出る)で、窓切替の時間差とは別の問題。この 2 つの入力先からは M1 について何も言えない(別途調べる価値はある)。

## M2: Mozc Readable × Windows Terminal のひらがなキー直後(BUG-209)

**方法:** `ime=mozc`(Mozc Readable の MSI)× 実 Windows Terminal・WezTerm・自前の RichEdit。`--sw-mode=f2 --sw-delays=0_30_100_300`:
warm のまま(前の試行から約 3 秒)物理ひらがなキーを押し、delay ms 後に打つ。対照 `nof2` は押さない。各 16 試行 × 3 回。

| 入力先 | 撤去前 | 撤去後 | `[mozc-compose]` の確認(撤去前/後) |
|---|---|---|---|
| 実 Windows Terminal + F2 | 48/48 | 48/48 | 51/51(試行 48 + 準備 3) |
| 同 対照(F2 なし) | 48/48 | 48/48 | 3/3(準備だけ) |
| WezTerm + F2 | 48/48 | 47/48 | 51/51 |
| 自前の RichEdit + F2 | 48/48 | 48/48 | 0/0(確認の対象外の窓) |

- **配線は問題なし:** 撤去後も、ひらがなキーのたびに `marked cold reason=F2NonTsf` → `[mozc-compose] checking composition` が張られる
  (`physical_f2_marks_cold`、PR #809 の M2 対応)。撤去前(`NativeF2Consumed`)と同数。対象外の窓(RichEdit)には張られない(N1 のとおり)。
- **リテラルそのものは CI では判定不能:** 確認はすべて `composition confirmed after 15〜16ms` で、Mozc が最初のキーを取りこぼす状況が
  CI では起きない(BUG-207・ADR-269 の CI 実験と同じ)。BUG-209 の実機の検知(312ms・328ms)のような遅い目覚めは作れない。
- WezTerm の撤去後 1 件(delay 30ms、期待 `ふほうほしく` → `ふ`)は原因未確定。awase は 6 文字すべてを送り、確認も 16ms で成功している
  (以降の文字は入力先側で失われた)。撤去前 0/48、1 回だけ。

## WezTerm(CI で入れられた)

- 入れ方: GitHub Releases のポータブル版 `WezTerm-windows-20240203-110809-5046fc22.zip`(2024-02-03 の最新安定版)。SHA-256
  `57e5d03b585303d81e8b8e96d1230362852eb39aca92b3b29c7a42cfb82f9ac4`(リリースに付属の `.sha256` と一致)を照合してから展開する。
- GPU の無いランナーでは、既定(OpenGL)と `front_end='Software'` は `The OpenGL implementation is too old to work with glium` で窓を作れない。
  `--config front_end='WebGpu'` で起動する(窓クラス `org.wezfurlong.wezterm`)。run 39956156323・39956370956。
- awase は `TSF-native window (org.wezfurlong.wezterm)` と分類し、Vk で送る(`send_keys: mode=Vk`)。
- Windows Terminal と同じ打鍵列(20ms、single・thumb・mixed、25 文字)の結果:

| 構成 | 撤去前 | 撤去後 |
|---|---|---|
| GJI 連続/cold/往復 | 24/24・24/24・24/24 | 24/24・24/24・24/24 |
| MS-IME 連続/cold/往復 | 24/24・22/24・24/24 | 24/24・23/24・24/24 |

MS-IME の cold の崩れ(最初の試行の 25 文字が全部消える)は撤去前にもあり(2 回中 2 回)、#809 とは関係しない。

## 実機で残すもの

- M1 の影響の大きさ(実機のフォーカス検出〜refresh の時間、Alt+Tab・マウスでの切替、GJI の変換キーの素通しで IME が閉じる経路)。
  所有者判断の材料: CI で確かめた範囲では、撤去前の保留(TsfGate)が防いでいた崩れが撤去後に出る。
- Mozc Readable × Windows Terminal・WezTerm の実際のリテラル(遅い目覚め)が ADR-270 の確認で回復するか。
- WezTerm の候補窓の表示(CI では composition ができることだけを見た)。

## 記録

| run | ブランチ(head) | 内容 |
|---|---|---|
| 39952236830 | `ci/verify-809-m1m2`(`5b58cbff`、develop `bcb9e2dd` + 測定) | 撤去後: M1・M2(WezTerm を除く) |
| 39952240514 | `ci/verify-809-m1m2-before`(`95261371`、`a1757efd` + 測定) | 撤去前: 同 |
| 39956586080 | `ci/verify-809-m1m2`(`efde4ba4`) | 撤去後: WezTerm |
| 39957668827 | `ci/verify-809-m1m2-before`(`1d81ef71`) | 撤去前: WezTerm |
| 39956156323・39956370956 | `ci/verify-809-wezterm-probe` | WezTerm の起動の可否と front_end |

1 回目の run の WezTerm の構成(各 10 構成)は `front_end='Software'` で窓を作れず INVALID(上の run 3・4 でやり直した)。

**再実行:** 測定コードは `ci/` ブランチだけにあり develop には入れていない(`typing_stress/switchime.rs`・`e2e-ime.yml` の
`sw809` 構成・`tools/e2e/input_forms/setup/wezterm.ps1`・`forms.toml` の `wezterm`)。
`gh workflow run e2e-ime.yml --ref ci/verify-809-m1m2 -f only='tsx-sw809-*'`(撤去前は `--ref ci/verify-809-m1m2-before`)。
集計は各 run の `result-tsx-sw809-*` の `dist/typing_stress.log`(`{"type":"sw"}` と `{"type":"trial"}` を `kind`・`n` で突き合わせる)と
`dist/awase.log`(`[mozc-compose]`・`tsf-gate`)。
