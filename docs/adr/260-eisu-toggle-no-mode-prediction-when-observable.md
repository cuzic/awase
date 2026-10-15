---
id: ADR-260
title: |-
  読みが効く窓では、英数トグルの入力モードを予測せず、直接観測の予測ガードを軸ごとにする
summary: |-
  報告 01M4QSFH1B の「英数を押しても入力モードのずれが解消しない」。belief が実機と逆位相のとき、英数(開状態ではかな↔半角英数のトグル)の予測が実機と同じ向きに反転し続け、逆位相が固定される。窓内の読みは実 conv を読んでいるのに、直接観測は「予測が付いたキーは読みを採らない」(ADR-188 追記6)ので採られない。そこで (1) 窓内の IME の読みが直近に成功している窓に限り、開状態の英数トグルは入力モードを予測せず追跡も捨てる(B1o)、(2) 追記6 のガードを、予測が値を持つ軸だけにする(B3)。GitHub CI で、読みが効く窓の 6 セルがすべて 0(従来 132/180・実 Chrome 75/180)。ただし報告の一太郎のように読み自体ができない窓には効かない(観測が無い窓では従来のまま)。
status: |-
  起草(2026-10-15)。Opus レビュー round 1(旧案 B1)で条件付き不可 → 追加測定(4〜5周目)で案を B1o+B3 に直して書き直し。再レビュー前。実装なし(スパイク `exp/mode-key-recheck` の CI 測定のみ)。
related_adr:
  - "ADR-165"
  - "ADR-188"
  - "ADR-191"
  - "ADR-192"
  - "ADR-205"
  - "ADR-209"
  - "ADR-238"
  - "ADR-259"
---

# ADR-260: 読みが効く窓では、英数トグルの入力モードを予測せず、直接観測の予測ガードを軸ごとにする

## ステータス

起草(2026-10-15)。初稿(読めない窓の英数トグルは入力モードを予測しない、B1)は Opus 敵対的レビューで条件付き不可となり、追加測定を踏まえて
**B1o(読みが効く窓だけ、英数だけ)+ B3(追記6 のガードを軸ごと)** に案を直した。コードは変えていない。
測定は使い捨てのスパイクブランチ `exp/mode-key-recheck`(develop にはマージしない)の GitHub Actions による
(run 38669329177・38670546888・38674632947・38680885438・38690714378。ブランチ `ci/e2e-eisu-composite` の先行測定も参照)。
ADR-259 は PR #624 で起票中(`docs/adr/index.md` の同じ位置に行を足すので、後からマージするほうに軽い衝突が出る)。

## 背景

不具合報告 `01M4QSFH1B02DQ9B8AXBYVRQ39`(GJI・一太郎、issue #623)に 2 つの問題がある。

1. 実 IME の状態と awase の belief がずれる(app log: 別アプリで IME OFF にして一太郎へ移ると、141 秒前の ON を `HwndCache` から復元する)。
2. **英数を押しても入力モードのずれが解消しない**(タスクバーは OFF なのに入力は全角、IME ON にするとローマ字。ちょうど逆転する)。本 ADR は 2 を扱う。

### 機序(CI の awase.log と、コードの読解で確認した)

- 英数の予測は、belief の開閉と変換モード(`track.conv` → 観測の `conv_raw` → belief の入力モードから推定、の順)からの表引きである
  (`key_effect_predictor.rs::predict_in_table`)。開状態の英数は、プリセット(MS-IME・ATOK)でかな(C19)↔半角英数(C10)を入れ替えるトグルである。
  belief の入力モードが実機と逆位相だと、予測も実機も同じ向きに反転し続け、逆位相が固定される。
- 窓内の IME の読みは、**読める窓では実 conv を正しく読んでいる**のに採られない。CI の awase.log(`tsf`・`sync` の 1 試行): 英数の 30ms 後から約 300ms、
  `[skip-typing-read] conv=0x00000010`(半角英数)を読み続けたが、belief は「かな」と予測したまま Engine を活性化した。理由は 2 つ:
  - 打鍵時点の予測が付いたキーは、直接観測の窓内の読みを**軸を問わず**採らない(`follow_direct_read_in_scope`、ADR-188 追記6。
    IME の処理が窓より遅いと窓内の読みは処理前の古い値で、正しい予測を覆してしまうため)。開閉だけを予測した打鍵でも、英数の軸の読みを採らない。
  - 打鍵中(`TYPING_IDLE_MS`=500ms 未満)の読みは `SkipTyping` で捨て、窓が閉じた後は読めない窓ではポーリングが止まる(`decide_read_strategy`)。
- 「読める窓/読めない窓」の区別: awase のプロファイルは、`tsf` フォームも実 Chrome も `Imm32Unavailable` だが、**窓内の IMM の読みは効いている**
  (押下の約 25ms 後の `[direct-follow] … conv=0x10` で追随している)。本当に読めないのは、報告の一太郎のように `ImmGetDefaultIMEWnd` が NULL で
  読み自体ができない窓(学習で `ImmCapability::Unavailable`)で、journal では観測が `ObserverPoll` 1 件・`direct-follow` 0 件しか無い。本 ADR では、
  前者を「**読みが効く窓**」、後者を「**観測の無い窓**」と呼ぶ。

### 実測(GitHub Actions windows-latest、GJI の MS-IME プリセット)

`typing_stress --mode=keymatrix`(`tsf`)/`chrome_probe --keymatrix`(実 Chrome)で、英数(0xF0)を belief と実 IME のずれ
(`close`=awase が ON を書いた後に外から閉じる / `fresh`=awase 再起動で未知 / `sync` / `open`=OFF を書いた後に外から開く、と閉から始まる `on:sync`・`on:fresh`)で押し、
直後のかな単打の結果から Engine と実 IME の組み合わせを読む。`tsf` の読み方: `か`=Engine ON+かな〔正常〕、`ｗ`=Engine OFF+かな〔ずれ〕、
`w`=Engine OFF+英数/閉〔一致〕、`ka`=Engine ON+英数/閉〔ずれ〕。実 Chrome のラベルは「かな・Engine 素通し」〔ずれ〕と「英数なのに Engine ON」〔ずれ〕を数える。
`--km-twice` で 2 文字目も記録する。6 セル × 3 run = 180 試行。読める窓(`edit`)は従来から常に追随した。

## 決定

### 決定1: 読みが効く窓では、開状態の英数トグルの入力モードを予測せず、追跡も捨てる(B1o)

`predict_in_table` で、次をすべて満たすとき、`effect.mode = None`・`track.conv = None` にする。開閉の予測(`effect.open`)は変えない。

- **観測が効く窓**(`PredictInput` の新しい真偽値 `observable_toggle_guard`。呼び出し側が計算する):
  `direct_mode_key_watch_kind() == Some(Gji)`(`Imm32Unavailable` かつ GJI)で、かつ**直近 `OBSERVABLE_MS`(30 秒)以内に `conversion_mode` を持つ IME の読みが成功した**こと。
  `TsfNative` プロファイル(直接観測の窓なし)・Microsoft IME 本体・観測の無い窓では偽(従来の予測のまま)。
- キーが**英数**(`TableKey::Eisu`)であること。ATOK のひらがな(0xF2)・入力中の無変換には当てない(下の「範囲」)。
- 開状態(`input.open`)で、押した後も開(`c.after_open`)。かつ、表のデータから変換モードを入れ替えるトグルと判定できる
  (現在の変換モードの行の `after_conv` が反対側で、反対側の行の `after_conv` が現在の変換モード、`is_conv_toggle`)。

効果: 開閉・入力モードとも変わらず追跡も変わらない打鍵は `apply_key_effect_prediction` が何もしない(`KeyEffectPredicted` が fence を進めず、`key_effect` に予測が残らない)ので、
「予測が付いたキーは予測に任せる」の対象から外れる。**読みが効く窓では、直接観測の窓の読みが belief と照合される**(一致すれば何も起きず、食い違えば追随する)。

**セルごとの機序**(測定のログで確認した):
- `sync`(belief が一致、実機がトグル): 直接観測が押下の約 25ms 後に実 conv(0x10)を読んで追随する。
- `close`・`on:fresh`(MS-IME プリセットで、belief が古い ON か「開」と信じていて、実 IME は閉): B1o は belief を動かさず、実機は「閉の英数 → 開・かな」になるので、
  belief(開・かな)と実機が一致する。読みも belief と同じなので追随は起きない(予測が belief を壊さなくなったことが効いている。直接観測が直したのではない)。
  ATOK プリセットは閉の英数が開かないので、この機序は成り立たない(従来から `close` はずれなかった)。

### 決定2: 追記6 のガードを軸ごとにする(B3)

`follow_direct_read_in_scope` の「打鍵時点の予測が付いている打鍵は読みを採らない」を、**予測が値を持つ軸だけ**採らないようにする。
`KeyEffectPrediction` の `open`/`mode` が `Some` の軸の読みは捨て、`None` の軸は採る。純粋な判定(`DirectFollow` に予測の軸をかける関数)として核に置き、単体テストで固定する。

効果: 実 Chrome の `tog:open`(belief が OFF、実 IME は開)。開閉だけを予測した英数(閉→開)の後も、英数の軸の読みが採られる。追記6 の懸念(古い読みで正しい予測を覆す)は、
予測を持たない軸には当てはまらない。

### 決定3: 範囲と設計上の取り決め

- **英数だけ**に限る。トグルの判定は表のデータから決めるが、適用は `TableKey::Eisu` に限る。一般の `is_conv_toggle` は ATOK のひらがな(全段階)・入力中の無変換にも当たり、
  既存テスト `hiragana_key_is_set_is_independent_of_unreadable` が落ち、ADR-188 案2(`predict_after_resent`: 途中の打鍵で `track.conv = None` だと `state.open && track.conv.is_none()` で
  打鍵全体の予測が `None`)が黙って外れる。英数に限れば、`predict_after_resent` が送り直す親指(無変換/変換)には当たらない。
- 窓の種類の判定は `PredictInput.unreadable`(ADR-209、`predict_henkan_open_in_unreadable_windows` 設定に結び付く)を**再利用しない**。新しい `observable_toggle_guard` を、
  設定とは独立に計算する(「閉の変換で開く」規則の設定を切った利用者の B1o が黙って止まる・逆も、を避ける)。
- 読みの成功の記録(`last_conv_read_ok_ms`)は、グローバル static ではなく `ImeStateHub`(または `Runtime`)のフィールドに置き、`PredictInput` で渡す
  (スパイクは検証の簡便のため static だった。本実装では持ち込まない、ADR-164)。
- 学習表(`predict_with_override`)の予測にも同じ判定がかかる(`predict_in_table` を通る)。CUSTOM の表に英数の行があると同梱表の予測は打ち切られる(ADR-209 決定4)ので、
  学習表のセルがあるときだけ効く。
- 新しい設定項目は足さない。

## 実測(スパイクの結果)

ずれの試行数(1 文字目、6 セル × 3 run = 180 試行、5 周目の同一 run)。

| 窓 | 従来 | `b1o` | `b1o`+`b3` |
|---|---|---|---|
| `tsf`(読みが効く) | 132 | **0** | **0** |
| 実 Chrome(読みが効く) | 75 | 29(`tog:open` 29/30 が残る) | **0** |
| `tsf` + `noread`(観測の無い窓の模擬) | 132 | 132(従来と同じ) | 132 |
| 実 Chrome + `noread` | 75 | 72(従来とほぼ同じ) | 75 |
| ATOK `tsf` | 45 | — | **0** |
| ATOK 実 Chrome | 30 | — | **0** |

`noread` は、awase の IME 状態の読み(`read_ime_state_full`)をすべて失敗させるスパイク用のフラグで、一太郎のように読み自体ができない窓の模擬である(awase 側だけで、
ハーネスの測定には影響しない)。観測の無い窓では、`b1o` は読みが 1 度も成功しないので入らず、従来の予測のままになる。

**観測の無い窓に、観測の有無を区別せず B1 を入れると悪化する**(4 周目、`noread`・実 Chrome): 75 → 90。belief が一致している `fresh`・`sync` が 0/30 → 30/30 になる。
観測が無いと belief が追随せず、従来の予測は belief が正しい多数派で当たっていたため。これが B1 に「読みが効く窓だけ」の条件を付けた理由である。

## 却下した案

- **B1(観測の有無を問わず、読めない窓のトグルは予測しない)**: 上の悪化(観測の無い窓で 75→90)。初稿の案。
- **A / Z(窓が閉じた後に 1 回読み直し、予測と食い違えば観測を採る)**: 読める窓では 0 だが、実 Chrome の `close` で悪化(15→23/24)。ただし、**読み直しは 1 文字目の測定の後に走る**
  (Z の読み直しは、打鍵の 515ms 後)ので、この指標では 1 文字目への効果を測れない。悪化は、読み直しが belief を変えたことで、次の試行の開始状態(試行間の状態の引き継ぎ)が
  変わったためと考えるのが自然で、「読みが信頼できない」ことの証拠にはならない。実利用(押してから打つまでに間がある)での有効性は未確定のまま。B1o+B3 で足りるので採らない。
- **B2(入力モードを `Unknown` に戻す)**: `effect.mode = Some(Unknown)` は予測として `key_effect` に残り、直接観測が採られない。次の打鍵で予測器が `Unknown` を種としてかな(Engine ON)にする(`seeded`)。
- **X(GJI の候補ウィンドウの SHOW を打鍵の結果の証拠にする)**: 実 IME が英数の試行でも SHOW が来て、belief を「英数→かな」に誤って直した(B1 に足すと `tsf` の 2 文字目が 0→45)。
- **Y(読めない窓の古い ON のキャッシュを復元しない)**: **未検証**。CI の再現手順(外から閉じてフォーカスを外して戻す)が `HwndCache: restore` を通らず、`[exp-y]` が 0 件だった。
  ADR-165(「時間条件は設けない」)と衝突するので、報告の流れ(別アプリで OFF → 一太郎へ戻る)を再現できる手順ができてから別に扱う。
- **ユーザーへの案内(冪等キー・警告)**: Ctrl+無変換/Ctrl+変換が開閉の軸を揃える。ただし**入力モードの軸はそれでは揃わない**(開閉だけ)。MS-IME プリセットのひらがな(F2=かなへ SET、`hiragana_key_is_set` が `Some(true)`)で
  入力モードを揃えられる。この ADR では何もしない。

## 検証(実装時)

- 純粋関数の単体テスト(Linux): `observable_toggle_guard` が真の窓で、開状態の英数(MS-IME・ATOK、C19・C10 の両方)は `effect.mode`・`track.conv` が `None`。偽の窓・閉状態の英数・
  ひらがな(0xF2)・無変換・変換モードが表に無い場合は従来どおり。既存テスト `hiragana_key_is_set_is_independent_of_unreadable` が変わらず通ること。`predict_after_resent` が従来どおりであること。
- B3 の純粋関数(`DirectFollow` に予測の軸をかける)の全組み合わせのテスト(予測の open/mode が Some/None の 4 通り × 読みの食い違う軸)。
- 既存の goldens・journal replay・`architecture_guard` が通ること(`follow_direct_read_in_scope` の走査テストの更新を含む)。
- **追記6 の遅い IME の構成(run 38091839446、RECOVER 14 件)を B3 ありで回し、退行しないこと**。これは未測定で、マージの条件とする。
- CI に `sc-eisu-km-{tsf,chrome}-*`(本 ADR の測定構成)を観測のまま残し、読みが効く窓の `tsf` の `typed` がすべて正常のとき判定を付ける。
- `docs/known-bugs/` に、報告 01M4QSFH1B の BUG ファイルを起票する(症状・修正履歴、残る穴を含む。(b))。

## 残る穴・未確認

- **報告の一太郎には効かない**。観測の無い窓では、この案は従来の予測のままで、belief が古い ON のとき(`close`)も 15/30 のまま。直すには、1 つめの問題の入口
  (HwndCache の古い ON の復元、Y)か、観測の無い窓でも使える別の信号が要る。**報告を「直した」とは書かない**(issue #623 はこの ADR では閉じない)。
- ユーザーの CUSTOM+学習表の組み合わせで、閉状態の英数がどう予測されるかは未測定(スパイクの CUSTOM 構成は readiness が通らず、従来の値が取れなかった)。
- 追記6 の遅い IME(処理が窓より遅い構成)の構成での B1o・B3 は未測定(上のマージ条件)。
- 実 Chrome の 2 文字目の `fresh`(従来でも 29/30 ずれ)は、1 文字目の `ka`(英数)の後に、別の仕組み(literal 回収など)が Engine を ON に戻すためと思われる。範囲外。
- 測定は MS-IME プリセットと ATOK プリセットの表、`ime_toggle` 付きの構成。各セルの試行は偶奇で開始状態が交互になる(5/5 に割れる)ので、両方の開始状態を含むが、
  開始状態のリセットはしていない。1 回の環境(`windows-latest`)で、実機の確認は未実施。
- `Microsoft IME 本体`(`MSIME_NATIVE`)の表には英数のトグルの行が無いので、この案は効かない(対象外)。
