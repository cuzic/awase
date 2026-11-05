# BUG-219 観測の整理(IMM の偽の「閉」)

2026-10-31。目的: 「IMM が偽の閉を返す」について、何が観測済みで、何が未観測か、次に何を測れば絞れるかを 1 枚にする。
実装・修正の提案ではない。出典は BUG-219.md・BUG-223.md・ADR-282 §1.1・コード(#846 `[imm-vs-shm]`)。

## 1. 症状の型(確定している範囲)

「IME は実際には ON なのに、IMM の読みが `open=false` を返し、それが `resolve_open_at` の単独の根拠(`DeriveMedium{ObserverPoll}` /
`DeriveHigh(ImmCrossProbe)`)になって belief が OFF に倒れ、Engine が OFF の間の打鍵が素通りする。数百 ms〜約 1 秒で戻る」。

| 観測 | 環境 | 型 | 確からしさ |
|---|---|---|---|
| BUG-219(2026-10-25 17:49 JST) | Mozc Readable × Chrome(利用者の本体窓) | 単独の `ObserverPoll(false)`、conv=0x19、共有メモリ(ground truth)は ON | **ログで確認**(`awase.log.old`) |
| BUG-223 区間 1 | GJI(tsf-native / imm32-unavailable) | 単独の `DeriveMedium{ObserverPoll}` | ADR-282 §1.1「検証済み」 |
| BUG-223 区間 4 | GJI | `DeriveHigh(ImmCrossProbe)` が偽の閉 | **コードの読みによる分類**(`decided_by` の行は未確認) |
| ADR-282 §1.1.1 の再現 | GJI | 単独の `ObserverPoll` による OFF 反転 2 回(0.5 秒・0.7 秒後に true) | team-lead の転記(起草者は未読) |

**読み取れること**: 偽の閉は **Mozc 固有ではない**(GJI でも同型が 3〜4 回)。PR #840 は Mozc 系を `[direct-follow]` から外しただけで、
GJI 側の経路は残っている。ただし GJI には共有メモリのような ground truth が無く、「IME は実際には ON だった」の根拠は利用者の体感・字形・後続の観測に頼っている。

## 2. 偽の閉を作る読みの条件(観測済みの事実だけ)

- BUG-219 の連続: 02.530〜.796 の **5 回連続**で `open=Some(false)`(約 0.27 秒続く)。1 回きりの競合ではない。同型が 17:48:50 / 48:53 / 49:00 / 49:04 にも出た。
- 読んだ窓: 利用者の Chrome 本体(ハンドル 0xa670f3a)。専用の Chrome(測定用プロファイル)では再現していない。
- conv は 0x19(ひらがな)のまま、開閉だけが偽。IMM の開閉は Mozc の「ひらがな」と「半角英数」を区別できない(半角英数でも「開」)ので、**開閉の読みと conv の読みは独立に壊れうる**。
- OFF は MozcShm(High)の TTL 1000ms 切れの瞬間に、直前の `[direct-follow]` の Medium が単独で勝って起きた(経路は確定。ADR-268 も「TTL=1000ms は不足」と既知)。**偽の閉があっても、High が生きていれば倒れない**。

## 3. 制御した測定で再現しなかったこと

| 測定 | 結果 |
|---|---|
| CI `ci/imm-read-matrix`(run 40233934917、87 構成 × 144 回) | 偽の開/閉 0 件(実 Chrome・別プロセス窓を含む) |
| dragonflyg4 専用 Chrome × Mozc(20ms 刻み) | キー直後 13〜35ms の追随、打鍵中 526 回全て「開」 |
| 同上で A=開・B=閉を素早く交互に前面化(36 回、820 読み) | 全て期待どおり。**窓切替の過渡の瞬間は拾えていない** |

未測定(記録にある): Windows Terminal ⇄ Chrome の切替、Chrome の要素フォーカス移動に伴う入力コンテキストの切替、利用者の本体プロファイル(拡張・複数の入力方式・アクセシビリティ)。

## 4. 仮説と、それぞれを分ける観測

| # | 仮説 | 今ある根拠 | 分ける観測 |
|---|---|---|---|
| H1 | **読んだ窓が打鍵先と違う**(`focused_hwnd` が別の入力コンテキスト) | 読んだ窓は利用者の Chrome 本体(Chrome は 1 スレッドに複数の窓)。制御した測定は専用プロファイル | 読んだ hwnd/スレッドと、直前のキーの宛先を同じ行に並べる(今は別の debug 行 `read_ime_state_full: focused_hwnd=`) |
| H2 | **窓切替・要素フォーカスの過渡**で入力コンテキストが一時的に未接続/別状態 | 偽の閉が `since_focus_ms` 小で起きたかは**未確認**(5 回連続・約 0.27 秒は過渡にしては長い) | `[imm-vs-shm]` の `since_focus_ms`・`since_key_ms` の分布(#846 が既に出す) |
| H3 | **TSF の開閉コンパートメントと IMM 互換層のずれ**(IMM の open は TSF の keyboard open-close 経由) | conv は正しく open だけ偽、という形と整合。直接の証拠は無い | 同じ瞬間の TSF 側の読み(`MozcShm.tsf_open` と `effective_open` が `MozcShmRecord` にある)を IMM の偽の閉と並べる |
| H4 | **利用者の環境固有**(複数の入力方式・拡張・アクセシビリティで Chrome が別の IME 経路を使う) | 専用プロファイルで 0 件/本体プロファイルで発生 | 利用者の本体プロファイルでの CI 外の測定(所有者の実機が要る) |

H1〜H4 は互いに排他ではない。**現時点でどれも反証も確認もされていない**。

## 5. 既存の観測装置と、足りない項目

`[imm-vs-shm]`(#846、`ime_refresh.rs::ir_note_imm_vs_shm`): IMM の開閉と共有メモリが**食い違った**瞬間に、1 秒に 1 行まで、
`imm_open / shm_open / shm_age_ms / shm_expired / conv / romaji / class / tsf_native / timed_out / strategy / belief_on / explicit_intent / since_key_ms / since_focus_ms` を info で出す。

足りないもの:
1. **Mozc でないと出ない**(`mozc_shm_view` が無い窓では `Skip`)。GJI の偽の閉は ground truth が無く観測装置が働かない。GJI 側の独立な根拠は**タスクバーの字形**(ADR-273、`IndicatorGlyph` の journal)。IMM と字形の食い違いを同じ形で残せば GJI にも適用できる(未実装、未検討)。
2. 読んだ hwnd・スレッド ID が行に無い(H1 を分けられない)。
3. 食い違った 1 回の行しか残らず、**連続の長さ(5 回連続・約 0.27 秒)が分からない**(1 秒に 1 行の間引き)。
4. ログ(info)は回転で失われる(BUG-223 の「元のログがサイズで消えた」)。journal には `MozcShm` エントリ(開閉・モード)はあるが、IMM 側の読みと食い違いは載らない。

## 6. 次に測る/足すもの(優先順、すべて観測のみ・挙動を変えない)

1. **報告の回収**: 次に「かんけいできますw１」型が起きたとき、`[imm-vs-shm]` の行(#846 入りの版が前提)と `since_focus_ms`/`since_key_ms` を所有者から受け取る。H2 の可否が即分かる。コード変更なし。
2. **`[imm-vs-shm]` に読んだ hwnd とスレッド ID を足す**(1 行追加、H1 を分ける)。間引きの代わりに「連続の開始と終わり」(ADR-279 P0 の `DriftFollowWould` と同じ節目方式)にして H の連続長を残す。
3. **IMM と字形の食い違い**を同じ形で残す(GJI にも効く。ADR-273 の字形の取り込みが前提)。
4. 擬似物は、上の 1〜3 で型(どの窓・どの過渡で偽の閉が出るか)が分かってから。軸が決まっていない今は作らない(PR #919 の反省)。

## 7. このメモが主張しないこと

- 偽の閉の原因が分かったとは言っていない。H1〜H4 は候補。
- GJI の「偽の閉」が BUG-219 と**同一原因**とは言っていない。型(単独の IMM 系の読みが belief を倒す)が同じ、というだけ。
- ADR-282 の候補 Z(Engine 入力の既定)が BUG-219 型を防ぐかは、ADR-282 の信頼表の主張(「BUG-219 型は信頼表で防ぐ」)で、このメモでは検証していない。
