# MS-IME × 実 Chrome で VK_IME_OFF が実 IME を閉じない件の根本原因(2026-10-04)

**status: 対応しない・記録のみ(所有者決定 2026-10-04)。** 原因は判明したが、補完は採用せず、MS-IME × Chrome の OFF 不発は既知の制限とする
(Google 日本語入力の利用を案内)。経緯と結論は [BUG-185](../known-bugs/BUG-185.md)、ADR-208 決定4 例外(a)の訂正はそちら。末尾「方針C」と「決定」に、確定キーの探索と採らなかった理由を残す。

対象: ADR-208 決定4 例外(a)(MS-IME × 実 Chrome の OFF 系)。調査ハーネスは `chrome_probe --offrca`/`--page=input`、`typing_stress --km-comp/--km-commit`、
`check_offrca.py`/`check_commitkm.py`、CI の `sc-offrca-*`(`only='sc-offrca-*'` で回す。観測のみ)。試作は `fix/msime-off-composition-imc(-prod)`(develop には入れない)。CI は全て windows-latest。
関連: [ADR-221](../adr/221-msime-ime-off-composition-loss-measure-first.md)、[BUG-184](../known-bugs/BUG-184.md)。

## 結論(確度つき)

**『環境(CI)固有』でも『Chrome 固有』でも『awase の送信の不備』でもない。**
**TSF の入力先(実 Chrome、RichEdit の TSF 窓)で MS-IME に未確定の composition が残っている間に `VK_IME_OFF`(0x1A)を受け取ると、
MS-IME は IME を閉じず conv を 25→16(半角英数)に変えるだけで開いたままにする**(確定)。
awase が書く OFF(トグルの OFF・単独タップの無変換・Ctrl+無変換)の `VK_IME_OFF` がこれに当たり、`IMC_GETOPENSTATUS` は 1 のまま。
`WM_IME_CONTROL(IMC_SETOPENSTATUS,0)` は実 Chrome でも composition があっても 10/10 で閉じる。awase 側で閉じることは**できる**(試作で CI の該当セルが全て収束)が、副作用のため採用しない(下記「決定」)。

| 主張 | 確度 | 根拠 |
|---|---|---|
| awase 無しの OS 注入でも同じ(awase のせいではない) | 確定 | R1/R2/R3 の `*-noawase`: `1a:typed_nc` 0/10、Chrome を試行ごとに起動し直しても同じ(R1 `msime-noawase-relaunch`) |
| 引き金は「未確定 composition」(Chrome 固有でも CI 固有でもない) | 確定 | composition を残した OFF は `typed_nc`/`typed_esc`/`typed_w2000`/`typed_w8000`/`typed`(ハーネスが textarea を空にしても IME 側に残る)全て 0/10。Enter で確定してから OFF は 10/10 閉じる。composition 無し(`notype`)は 10/10 閉じる。RichEdit TSF 窓(Chrome 以外)でも composition 有り 0/10・無し 10/10。IMM32 のみの EDIT 窓は composition 有りでも 10/10 |
| MS-IME はキーを処理しているが閉じない | 確定 | ページが見る OFF キーは `keydown Process 229`、`IMC_GETCONVERSIONMODE` は 25→16、`IMC_GETOPENSTATUS` は 1 のまま(composition 無しなら Process イベント無し・conv 25 のまま・閉じる) |
| 修正: OFF のあと `IMC_SETOPENSTATUS(0)` を補う | 確定(試作の CI) | 下記「修正の A/B」 |
| 実機(Windows 11)でも同じ | **未確認** | OS の同じ部品の挙動なので強く示唆されるが、CI 以外で測っていない(実機は clipwire を使わず未確認) |
| MS-IME が composition 中の IME_OFF を conv 変更に化かす理由 | 未確定 | 観測のみ。推測: 英数キー相当の「composition を半角英数にする」動作に落ちる |

## これまで『環境の制約』と見えた理由

CI の実 Chrome ハーネス(`sc-keymatrix-*-chrome`、`sc-driftrecovery-*-chrome`)は、状態確認の `ensure` が毎回 `k,a` を打って(`か` の composition)
から textarea を空にするだけなので、**押す直前に必ず未確定 composition が IME に残る**。このため OFF が閉じない側だけが毎回再現していた。
fresh 対照(awase 再起動)が同率で失敗したのは、composition が awase ではなく IME/ホスト側にあるから。

## 仮説の判定

- **H1**(Chrome には IMC が届かない / TSF compartment でしか効かない): **棄却**。実 Chrome × MS-IME で `imc0`(`IMC_SETOPENSTATUS 0`)は composition の有無によらず 10/10 閉じる(R1〜R3)。
  なお TSF 大域 compartment(`GetGlobalCompartment`)の書き込みは MS-IME・GJI とも 0/10 で効かなかった(`tsf0`。この試し方自体が妥当かは未確定)。
- **H2**(VK_IME_OFF が MS-IME に届かない): **部分的に棄却**。届いて処理される(Process 229、conv 変化)が、composition があると閉じる処理にならない。
  ON が効いて OFF だけ効かない非対称は、ON 側(`VK_IME_ON`)は composition が無い状態で測っていたから。
- **H3**(CI 固有の設定): **棄却**。互換モード(NoTsf3Override2=1、旧 IME)でも同じ(R5 `1a:typed_nc` 0/10)。`0x1A` は MS-IME のキー割り当て対象ではなく、`0x19` は割り当て無しで閉じる。
- **H4**(awase 側の再 ON・タイミング): **棄却**。awase 無しで同じ失敗。awase.log でも OFF 注入は押下の約 60µs 後に 1 回、後続の ON 書き込み無し。

## 決め手の実験(run、試行数、observed)

各セル 10 試行、`made`=前提(IME ON・API=開)が成立した試行数。API は打鍵せず 20ms 周期で 4 秒ポーリング(閉じる時間)、その後 `k,a` を 2 回(1 回目は古い composition の確定が混ざる)。

- **R1** run 37164642813(`sc-offrca-*`): MS-IME・awase 無し・Chrome。`1a:typed` 閉じた 1/10(made 10。初回のみ)、`1a:notype` 10/10、`1a:imc` 10/10、`19:typed`(0x19)10/10、`f3`/`1d` 0/10(MS-IME では OFF キーでない)、`imc0` 10/10、`tsf0` 0/10。
  閉じなかった試行に 4 秒後の 0x1A を足すと 9/9 で閉じる。GJI 対照は `1a:typed` 10/10。
- **R2** run 37165409134(`sc-offrca-r2-*`): `1a` + {typed, typed_nc, typed_esc, typed_w2000, typed_w8000} は全て 0/10(10/10 made)。`typed_enter` と `notype` は 10/10。awase 有り `ctrl1d` も同じ(0/10、`typed_enter` 10/10)。
- **R3** run 37166311611(`sc-offrca-r3-*`): composition を残したまま(`typed_nc`)の 1a は 0/9〜10。**二重送信は効かない**(0/50/150/400ms 間隔とも 0/10)。`1a`+100ms 後の IMC は 10/10。GJI は `1a`/`ctrl1d` とも `typed_nc` で 10/10。
  自前窓(`--km-comp`): tsf(RichEdit TSF)× MS-IME で composition 有りは `ctrl+1d`/`1a`/`19` の 3 セルとも STUCK 10/10、composition 無しは 3 セルとも CONVERGED_1。edit(IMM32)は有り無しとも CONVERGED_1。
  ページイベントは OFF 後 `text_post='かか'`(composition は未確定のまま)、その後の打鍵は `ka`(ASCII)。
- **R4** run 37167196929: 閉じなかった状態(半角英数 conv=16)で ON キー(`0x16`、awase 有りは Ctrl+変換)を押すと **かな入力に戻る(10/10)**。利用者に見える害は小さい(OFF 後の打鍵は ASCII、ON で復帰)。
- **R5** run 37167789514: 互換モード(legacy_custom=true)でも `1a:typed_nc` 0/10、`19`・`imc0` 10/10。
- 注: R4/R5 の `1a:notype` が 0/10 なのは、直前セルの `--or-then` の打鍵が composition を残していたため(原因の裏返しで矛盾しない)。

## 修正の A/B(`fix/msime-off-composition-imc`、run 37167277968)

試作: `ime_controller.rs::apply_mechanism` の MsImeDirect の OFF で `VK_IME_OFF` の後に `ime::set_ime_open_cross_process(false)`(`IMC_SETOPENSTATUS(0)`、同期・タイムアウト付き)を足す(+10 行程度)。

| セル(MS-IME 本体) | 土台(develop) | 試作 |
|---|---|---|
| `sc-keymatrix-abs-chrome-msime`(9 セル) | OFF 系 ENV_EXCEPTION / STUCK | **全 9 セル CONVERGED_1**(各 n=10) |
| `sc-keymatrix-tog-chrome-msime`(7 セル) | トグル OFF 方向 STUCK | **全 7 セル pass**(CONVERGED_1×1、CONVERGED_2×6) |
| `sc-keymatrix-e2-msime-chrome` / `e2-tog`(fresh 対照) | 0.90 失敗(ENV_EXCEPTION) | **fresh 含め全て CONVERGED_1**(tog:open/f3:open/19:open は CONVERGED_2) |
| tsf × composition 有り(`--km-comp`)3 セル | STUCK 10/10 | **CONVERGED_1 10/10** |
| `sc-offrca-r4-awase`(`ctrl1d:typed_nc`、`ctrl1d:notype`) | 0/10 | **10/10 閉じる**、ON で NICOLA 復帰 10/10 |
| `sc-driftrecovery-ctrlmuhenkan-msime-chrome` | NOT_CORRECTED(9/10) | **GAP_NOT_MADE 10/10**(OFF が効いて ずれ自体が起きない) |

回帰(MS-IME の `sc-*` 36 構成、土台 run 37168368736 と試作 run 37168367264): 試作の経路が実行された 9 ジョブ(`補完 ok=true` のみ、false 0)の rc は土台と全て同じ(reopen-tsf×3 rc=0、settle-explicit×3 rc=0、driftrecovery 系は同じ rc=1)。
残りの差(rc=3 INVALID の増減、`sc-kanji-gji-msime-3`・`sc-kanji-role-default-msime-2` の FAIL)は補完が実行されていない構成で、両 run の同時実行による chrome 系のフォーカス不安定と同種(土台 run 側にも rc=3 が出ている)。**回帰なしと言い切れるのは補完が走った 9 ジョブの範囲まで**。補完の所要は `send_elapsed` で 0〜29ms。

## 修正案(試作。採用しなかった)

閉じることはできる。**試作は検証用**(同期 IMC をフック経路で呼ぶ・actuation 合流点のリスト `RESTRICTED_CALLS` を更新していない)で、本番では次の形で設計し直す:

1. 本質は『MS-IME の OFF は VK だけでは閉じないことがある』。`MsImeDirect` の OFF を「VK_IME_OFF + IMC_SETOPENSTATUS(0)」の2段にするか、(Imm32Unavailable/TsfNative × MS-IME の) chain の第2機構に `ImmCross` 相当を足す(ADR-089/163 の chain・`decide_attempt` の純粋決定に載せる)。
   Standard プロファイルの MS-IME が従来から `ImmCross`(IMC)で閉じていて composition でも問題が出ていないことと整合する(同ファイルの「150ms: composition tear-down で 50ms だと取りこぼす」コメントと同じ現象系)。
2. IMC は GJI では効かない(`imc0:typed_nc` で API は閉だが打鍵は `か`)ので、MS-IME 限定にする(GJI は従来の VK のまま)。
3. composition を壊さない/失わない確認: IMC 経由の閉じは composition を取り消す(`compositionupdate`→`compositionend`、`text_post=''`)。VK だけの OFF は composition を残して半角英数にする。
   利用者の『打っている途中で無変換』で打った語が消えるかは未確認なので、実機 A/B(語を打つ→無変換→ページの文字を見る)が要る(R3 の `1a_imc0` は `text_post=''` で、確定でなく取消に見える)。
   代案: OFF の前に composition を確定させる(Enter を送るのは副作用が大きいので不可)、`VK_KANJI`(0x19、composition を確定して閉じることを 10/10 確認)は状態が確定していれば使えるがトグルで belief 依存(ADR-189)。

### 影響範囲と回帰テスト案

- 影響: MS-IME 本体 × Imm32Unavailable/TsfNative の OFF 系(トグルの OFF、無変換単独タップ、Ctrl+無変換、D4)。GJI・ImmCross 経路は不変。
- 回帰: `chrome_probe --offrca=1a:typed_nc,ctrl1d:typed_nc`(今回の `sc-offrca-*`)を `expect=pass` で `sc-*` に常設し、MS-IME×実 Chrome×composition 有りの OFF が 10/10 閉じることを固定。
  `ime_key_sequence_golden.rs` に「MsImeDirect の OFF の送信列に IMC(OFF)が含まれる」golden。`sc-keymatrix-*-chrome-msime` の `observe` を `pass` に昇格。
  ADR-208 決定4 の例外(a)を撤回し、『composition がある間の VK_IME_OFF は閉じない(MS-IME の挙動)』を原因として書き換える。
- ハーネスの注意: 実 Chrome の `ensure` が composition を残す点を記録する(composition 有り・無しをセルに明示する。`typed`/`notype`)。

## 公開判断への含意

- 例外(a)は『環境制約』ではなく **MS-IME の挙動 × awase が VK だけで OFF している**問題。利用者に見える害は小さい:
  OFF 後の打鍵は ASCII(直接入力と同じ見え方)、IME API が開と読むだけ、ON キーで かな入力に戻る(R4、10/10)。固着ではなく『見かけの OFF が半角英数モード』。
- v2 公開を止める根拠にはならない。ADR-208 の記述(例外の理由)は誤りなので訂正した。

## 方針C: 「確定してから閉じる」手段の探索(2026-10-04、run 37175260029・37176608249・37177637876)

IMC(OFF) の補完は TSF 窓で未確定の文字を**取り消す**(`text_post=''`。BUG-184 の逆)。そこで『未確定を確定してから OFF』にできるキーを探した。

**候補と出典**: GJI の MS-IME プリセットの Composition の Commit は Enter・Ctrl+Enter・Ctrl+M・VirtualEnter(`docs/adr/186-measurements/config1-custom-keymap-table-ignored.tsv`)。
F6〜F10・Ctrl+I/O/P/T/U・Tab は変換系で Commit ではない(同表)。Ctrl+J は Commit に載っていない。Microsoft IME 本体の標準は本実測が一次情報
(既存の裏付け: `layout/nicola_kakutei.yab` と ADR-109/115 の Ctrl+M 確定、`msime-chrome-off-rca` R2 の『Enter 後なら 1a が 10/10 閉じる』)。

**実測(MS-IME 本体・awase 無し・実 Chrome、各 n=10、made=10。判定は `api_closed` と `text_post`)**:

| キー(`<キー>+1a`=150ms 後に VK_IME_OFF) | composition あり: OFF が閉じた | あり: text_post | composition なし: textarea の副作用 | なし: 単一行 input |
|---|---|---|---|---|
| Enter | 10/10 | かか(残る) | 改行 9/10 | なし |
| Ctrl+M | 10/10 | かか(残る) | なし | なし |
| Ctrl+Enter | 10/10 | かか(残る) | なし | なし |
| Shift+Enter | 0/10 | かか(確定しない) | 改行 10/10 | なし |
| Tab | 0/10 | かか(確定しない) | なし(ただしフォーカス移動) | なし |
| F6〜F10 | 0/10 | 変換表示(かか/カカ/ｶｶ/ｋａｋａ/kaka)のまま | なし | 未測定 |
| Ctrl+J / F7 | 測定不能(made=0) | — | Chrome がダウンロード画面/キャレットブラウズのダイアログを出しフォーカスを失う(推定) | — |

- awase 有り(Ctrl+無変換で OFF、n=10): Enter/Ctrl+M/Ctrl+Enter の後の OFF は 10/10 閉じ、text_post は `きうきう` が残る。composition なしで Ctrl+M/Ctrl+Enter を押しても本文は空。
- 自前の EDIT(IMM32)窓・RichEdit TSF 窓(`--km-commit`、n=10): 確定キー無し(`none`)の OFF だけで 10/10 閉じて文字も残る。この 2 窓ではこの不具合が再現せず、確定キーの判別には使えない。
- 上の表は Chrome(TSF)だけ。Ctrl+M は端末で Enter 扱い、Ctrl+Enter はチャットで送信になるはずだが **Chrome 以外の副作用は未測定**。composition の有無の判定が必須。

**composition の有無を知る手段**:
- UIA `TextEditPattern::GetActiveComposition`(Chrome、n≈70): composition あり=範囲の文字列が本文(`かか`)、なし=`U+FFFC`(約50回)または空(3回)。所要 2〜6ms(外れ値 25ms)。
  取得失敗(`nopattern` 3回・null 範囲 2回)が約 5/70。判定は『空でも U+FFFC でもなければ composition あり』。
- `ime_composition_active_now()`(EVENT_OBJECT_IME_SHOW/HIDE): MS-IME では常に false で使えない(ADR-117、BUG-184)。
- awase 自身が出した romaji の追跡: 句読点での自動確定やアプリ側のクリアが見えず過大評価になり、端末で Ctrl+M が Enter としてコマンドを実行する危険がある。不採用。

**最良案(実装していない)**: UIA `GetActiveComposition` で判定 → あれば Ctrl+M か Ctrl+Enter で確定 → 約 150ms 待つ → VK_IME_OFF。なければ VK のみ。

**未解決点**: (1) Chrome 以外(WezTerm・Windows Terminal・VS Code・LINE・Teams)の UIA の読み取りと確定キーの副作用が未測定。(2) UIA 取得失敗約 5/70 のときの倒し先(VK のみ=半角英数に残る、か IMC=文字が消える)。
(3) 同期経路(フック・メインスレッド)で 150ms 待てないので非同期化が要り、世代照合(BUG-34 型)を新設する必要がある。(4) awase 有りの `ctrl1d` 単独(`typed_nc`、IMC 補完入りの試作)で text_post が 9/10 残り 1/10 だけ空だった。
ADR-222 の試作が『IMC で composition が取り消される(`text_post=''`)』としていたのと食い違うので、再検討するなら最初に再確認する。

## 決定(所有者、2026-10-04)

- **MS-IME × Chrome の OFF 不発は今後も対応しない**(v2.0.0 にも v2.0.x にも含めない)。最良の対処は Google 日本語入力の利用を案内すること(GJI は composition があっても VK_IME_OFF で 10/10 閉じる)。
  CHANGELOG・README・移行ガイドの既知の制限に平易に書いた。
- 採らなかった案: IMC(OFF) 補完(TSF で未確定の文字が消える=BUG-184 の逆)、UIA 判定+確定キー+待ち+OFF(Chrome 以外未測定・非同期設計が複雑で費用対効果が負)。
- 再検討の条件: Chrome 以外の TSF 窓でも、未確定を安全に確定できる手段(判定と確定キーの副作用がない)が見つかったとき。
