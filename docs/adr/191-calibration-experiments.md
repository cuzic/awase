---
id: ADR-191-companion-191-calibration-experiments
title: |-
  ADR-191 較正・予測の実験の経緯と実測結果（格子・通知購読・CI高速化・文献調査・巡回シミュレータ）
type: companion-doc
related_adr:
  - "ADR-186"
  - "ADR-189"
  - "ADR-190"
  - "ADR-191"
  - "ADR-192"
  - "ADR-193"
---

# ADR-191 較正・予測の実験の経緯と実測結果

[ADR-191](191-ime-is-source-of-truth-observe-not-write.md)の決定（原則・線引き・打鍵時予測・3段階ラウンド）の根拠になった実験を、時系列でまとめる。決定そのものはADR側にあり、
ここには**経緯・数値・失敗と訂正・成果物の所在**だけを置く。数値の出典（GitHub Actionsのrun URL、ブランチ、コミット）を付けられないものは「未確認」と注記する。
CIのrunは`https://github.com/cuzic/awase/actions/runs/<番号>`。

## 1. 最初の実測（スパイクA/B/A'）と、その訂正

### 条件
スパイク（`ime_key_matrix_spike`、`--walk=N --seed=S`でランダムなキー列を注入し、押下前と+100/+400/+1500msのIME状態を記録）を、次の条件で走らせた。
- **A**: awase起動、スパイクのキーは外部注入（awaseは物理キー扱いしない=追随が起きない）。
- **B**: `AWASE_TEST_INJECTION=1`で、注入を物理キー扱いする（物理キー相当）。
- **A'**: awaseを完全にバイパス（`disable_apps`にスパイクを入れる）。IME単体の効果を見る。
- **Ah/Bh**: awaseをスパイクの初期化の後に起動する（`[imm-learning]`による誤降格を避ける狙い）。

出典: `origin/spike/ime-effect-learning`の`tools/e2e/ime_key_matrix/results/elw2`〜`elw16`（ローカルの実機で取得。elw16は途中で中止）。

### 主な結果（ADR-191「実測」節が詳細）
- IME単体（A'）の一段の効果は、Mozcの公開キーマップからほぼ予測でき、静的モデルの一段予測は約98.5%（保持セル200件で197件、トグルキー52/52）。
- awaseを通す（B）と、仕様から外れる（84.5%）。Engineと実IMEのずれは19〜23%。開ループ（観測なしで予測を連鎖）は、最初のずれ以降が残り、信頼できない。

### 訂正した誤り
- **GJIとMS-IMEの取り違え**: 初期のelw3〜6は実際にはMS-IMEで動いていた。TIP（`D5A86FD5`=GJI／`03B5835F`=MS-IME）を記録するようにし、`--activate-gji`を付けてGJIで再測定した。取り違えた期間の結論は撤回した。
- **ROUNDヘッダの誤読**: ログの「ROUND2=RichEdit」は案内文で、実際のフォーカスは標準Editだった。レビュー指摘（M1）は誤りで、レビュアーも撤回した。
- **「ADR-189の例外は効く（0/18）」**: 測定に使ったawaseは`e2e/ablation`のビルドで、ADR-187/188/189が入っていなかった。本文に訂正を入れた。
- **`[imm-learning]`による降格が`cache.toml`に永続化**: スパイクのEditが`Imm32Unavailable`と学習されると、以後のランでawaseが読み取りを止める。elw11（撤去ブランチのビルド）は
  これで無効だった（Engineが一度も切り替わらず、ずれ55〜64%が「読めない条件」と同じ数字になった）。`cache.toml`から該当行を消して再測定した（elw12〜）。
- ローカルのWindows機は、ユーザーが他の用途に使うため、以後はGitHub Actions（windows-latest）で測定した。

## 2. 格子（grid）: 学習ラウンドの設計と3つの版

ランダムwalkでラン数を増やす方式をやめ、**状態の軸を決めて全セルを網羅する格子**にした（ユーザー指示）。軸は、開閉、変換モード、入力中の段階
（なし／入力中／変換中〈Space〉／変換中〈変換キー〉／変換中〈無変換〉）。押すキーは無変換・変換・ひらがな・カタカナ・英数・半角/全角・漢字・`VK_IME_ON`/`VK_IME_OFF`・Esc・Enter・Space・BS。
セットアップの検証（開閉と変換モードの観測で目標状態と一致）を通った試行だけでキーを押す。押下前と+100/+400/+1500msの状態、入力中の文字列の行方（保持/確定/破棄）を記録する。

| 版 | 状態の作り方 | 結果 | 出典 |
|---|---|---|---|
| 第1版 | 変換モードを**IMM書き込み**（`ImmSetConversionStatus`等）で作る | ATOK: 1,204試行を記録・セットアップ不能132・355セル・決定性99.5%（非決定6）。MS-IME(プリセット): 1,336試行・394セル・決定性100% | run 35561438376 |
| 第2版 | 変換モードを**キーで到達**（探索BFS）。リセットだけIMM | ATOK 310/312セルで決定的（非決定2）。MS-IME 170/170。第1版との共通セルの差: ATOK 16/156、MS-IME 4/85 | run 35568119615 |
| 第3版 | **リセットも含め全てキーだけ**（IME_ON・ひらがなの往復） | ATOK 132セル・非決定0（各セル2試行）。MS-IMEはリセット基準を(開,0x19|0x09)に直して再実行（下記） | run 35572366490（ATOK）。MS-IMEは最初の実行（run 35574917993）が全試行セットアップ不能で、修正後の再実行のrun番号は未確認 |

**第1版の誤り**: IMMで作った変換モードの状態は、キーで入った状態と別物だった。ATOKのひらがなキー（開）は、独立walk（キーで到達した状態）では 0x19→0x10 が20/20、0x10→0x19 が19/19の**純粋なトグル**なのに、
第1版は「0x10のまま不変」と誤学習した。同じ理由で、0x10/0x13/0x18のセルはひらがな以外も信用できなかった。

**キーで到達できる変換モード**:
- ATOK: 0x19（ひらがな）と0x10（半角英数）の2つ。0x13・0x18・0x1Bは到達不能。
- GJIのMS-IMEプリセット: 0x19と0x1B（全角カタカナ）の2つ。0x13・0x18・0x10は到達不能。自然状態は0x09（ROMANビットなし）で、ひらがなキーがSet型のため0x19へは戻らない。
  カタカナ→ひらがなの往復で0x19に到達できる。Rust側は0x09を0x19、0x0Bを0x1Bと同一視する。
- **閉状態の変換モードの読み取りは不安定**（例: `off-c10-none|bs`がOFF/0x19と読める）。予測表は閉状態の変換モードを追わず、開閉だけを予測する。

MS-IMEプリセットの最初の第3版は、探索の最後の閉状態（`open=0, conv=0x1B`）で`VK_IME_ON`を6回押しても開かず、全1,100試行がセットアップ不能になった。閉のときは`VK_IME_ON`と半角/全角を交互に押すよう直した。

## 3. 予測ビルドの実測（打鍵時予測、決定3）

### 読めるアプリ（観測あり）と読めない条件（blind）
「読めない条件」は、スパイクのEditを`Imm32Unavailable`として`cache.toml`に事前投入し、awaseがIMEを読めない状態にしたもの（TsfNative相当。ただし実際のTsfNativeの分類・warmup等までは再現していない）。
Engineと実IMEのずれ（押下+Nms時点で、Engineの活性が「実IMEが開でかな」と一致しないか）を、`effect_learning.py --drift`で数えた。標本は各約100押下・seed 2つ。

| run | 表 | 観測あり 400ms以降 | 読めない条件(seed1/seed2) | 観測ありの`[key-effect-miss]` |
|---|---|---|---|---|
| ローカルelw11（観測に追随のみ、予測なし） | — | — | 55.8%/64.2%（Edit降格でEngineが動かず） | — |
| ローカルelw14/15（手書き表の予測） | 手書き | 0〜1.4% | 18%/39% | 2〜4件 |
| 35565293892 | 格子第1版から生成 | 0% | 17.4%/36.5% | 10〜11件 |
| 35570450427 | 第2版から生成 | 0% | 25.0%/31.8% | 2〜4件 |
| 35578913708 | 第3版（ATOK）から生成 | 0% | 9.1%/20.0% | 4件 |
| 35585712177 | 第3版＋下記3件の修正 | 0% | **0%/0%** | **0件** |

追随のみの方式（撤去ブランチ、予測なし）は、押下+400msでのずれが35〜41%、+1500ms以降0〜4%だった（追随に約0.5秒かかる。ローカルelw13）。予測は打鍵の時点で反映するので、この遅れがなくなる。

### 独立walkでの表の一段予測（ATOK、A'の独立walk 4本・約300押下）
第2版の表92.6%（一致263・不一致21）→第3版の表**99.6%**（一致278・不一致1・表に無い19）。不一致の1件は`on-c10`の入力中のEsc（除外して「予測なし」にした）。第2版の不一致の大半は`on-c19`のひらがな（0x19のまま、と誤っていた）。
出典: 撤去ブランチの`tools/e2e/ime_key_matrix/score_walk.py`。MS-IMEは独立walkが無く、採点できていない（未確認）。

### 読めない条件で外れていた3つの原因（ログのオフライン再生で確認）
1. **古い明示意図が予測に勝っていた**: スパイクが起動時に注入する`VK_IME_OFF`が明示意図として残り、`effective_open()`で予測より優先されて約31秒間Engineが動かなかった。開閉を予測したら同じ対象の意図を捨てる。
2. **入力中の追跡が観測頼みだった**: 読めないアプリでは観測が無く、`k`のあとの無変換が「入力中でない」セルを引いてOFFと誤予測した。観測ありの`[key-effect-miss]`4件（予測=閉じる・英数、実際=開・ローマ字）も同じ原因。
   開いている間の文字キーで入力中を追跡する。
3. **表に無い変換中の段階が古いまま残っていた**: ATOKの表には「無変換で入る変換中」の行が無く、この段階に入ると以後のEnterや半角/全角が予測なしになった。変換中の行が表に全く無いときだけ、入力中の行で代用する。

なお「閉/開で変換モードの追跡が切れることが原因」という私の仮説は、ログでは確認できず、主因ではなかった。

## 4. CI高速化（固定待ちの短縮と通知の購読）

学習の固定待ちを縮める3段階（`--fast`、`--speed=K`、`--notify`）と、無駄な実行の回避（到達不能セルの除外、セットアップ不能の連続で打ち切り、smokeゲート、`concurrency`）、適応的な試行回数（`--grid-adaptive`）を入れた。
いずれもCI道具PR #237に含まれる。

| 版 | ATOK 4シャードのスパイク本体 | 備考 | 出典 |
|---|---|---|---|
| 遅い版 | 1,287〜1,693秒（ジョブ全体1,342〜1,745秒） | 基準 | run 35572366490 |
| `--fast --speed=2` + PRUNE/適応 | 326〜421秒（ジョブ全体675〜776秒、うちsmokeゲート待ち約250〜300秒） | 表の差分0（4シャード全て） | run 35580929997 |
| `--notify`（+`--fast`、適応） | s1 232・s2 224・s3 306・s4 263秒 | 132セルで遅い版と差分0、非決定0 | run 35582339438 |
| `--notify-comp`（+`--snap100`） | s1 199秒（`--notify`の232秒より14%短い） | 30セルで差分0 | `ci/e2e-notifyprobe`の実行（run番号は未確認） |

- `--speed`: fastnotifyの実験（別セッション、`ci/e2e-fastnotify`）では、ATOK s1で`--speed=4`は30セル一致（464秒）、`--speed=2`は1セル（`on-c19-conv-space|henkan`、保持と破棄が割れた）だけ違い（722秒）、標本は各1ランで確度は低い。
- 固定オーバーヘッドは1ジョブあたり約50秒（checkout 7秒、GJI導入25〜33秒、言語設定10秒）。ビルドは86〜155秒（キャッシュ次第）。
- 煙テスト（`ci/e2e-calibration`、`cal-notify-atok-s1`）: run 35589884725、約263秒、30セルで差分0、`[GRID-ABORT]`0件、通知の統計は「通知で早く終了159／変化なし80／上限まで待った5」。
- 中間の不具合: 打ち切り条件が強すぎて、通常のATOKでも35試行目で全シャードが打ち切られた（`on-c09`の疑似的な到達状態と`conv-muhenkan`の連続不能）。「1度も成功しないまま3つの状態で不能」に限定して直した。
  `grid_learn.py`が`--fast`のログで次の準備操作を観測に取り込む不具合も直した（fastnotifyで「30セル中23セルが違う」と出た原因）。

### 通知の遅延（TSFスレッドcompartmentの変更通知）
`compartment_notify_probe`（ADR-193）を、ATOK・キー間隔1500msで1回・16キー測った（診断コピー`compartment_notify_probe_diag.rs`）。
- 押下から最初の通知まで: P50=1ms、P95=34ms、最大34ms（通知があった10件）。32〜34msは最初の`IME_ON`だけで、他は0〜1ms。
- 通知が来なかったキー: 6/16=38%（Enter、Esc、Space、a、k、カタカナ。周期読み取り50msでも変化なし）。周期読み取りの検出遅延はP50=34ms・P95=63ms。
- キー間隔600ms、MS-IMEは未測定。「変化なし」の最小待ち（150ms）は標本が10件で根拠が薄い（未確認）。
- GJIと標準Editでも、`WM_IME_STARTCOMPOSITION`/`COMPOSITION`/`ENDCOMPOSITION`は押下から0〜5msで届き、確定時は`ENDCOMPOSITION`に続いて`EN_CHANGE`が約5msで届いた（`WM_IME_NOTIFY`は497件）。ただし最初の実行では届かず、理由は未解明。

### プローブがCIでキーを1件も注入しなかった原因
前面化は成功していた（診断コピーで`fg==top:true`）。前面化されるとキーボードフォーカスがトップ窓に移って入力欄から外れる。プローブ本体には`WM_SETFOCUS`の処理が無く、`focus_on_probe`がfalseになって中断していた。
スパイクの窓プロシージャは入力欄へフォーカスを戻す。修正案は`WM_SETFOCUS`で入力欄へ戻す1アーム（`tools/e2e/ime_key_matrix/patches/compartment_notify_probe-setfocus.patch`）。本体（ADR-193の成果物）は編集していない。

## 5. 文献調査と、巡回シミュレータ

### 文献調査の要点
出典の一覧と、実際に本文を読んだか（読めていない出典は「書誌情報のみ」）は、作業用メモ`literature-fsm-traversal.md`（リポジトリ外）にあり、ここには要点だけを転記する。
- IMEの状態読み取りAPI（`ImmGetOpenStatus`、`ImmGetConversionStatus`、TSFのcompartment）は、Lee & Yannakakis（Conformance Testing）とUyar（Conformance Testing Methodologies, Ch.III）でいう**status message**に当たる。
  信頼できるstatusがあれば、checking sequenceは「遷移図を覆う経路＋各訪問でstatusを読む」に退化し、UIO・DS・W集合は不要。statusが不確かな分は「状態ごとに1回だけ2連続で読む」で足りる。
- 巡回は**有向グラフの中国人郵便配達問題**（最小費用流で多項式時間）。リセットを「任意状態から初期状態、コスト約1.3秒の辺」としてグラフに入れれば、「リセットで飛ぶか、キーで歩くか」が最適化に乗る。
- 待ちは固定sleepでなく、通知・センチネル入力（Smeenk 2015）・上限タイムアウトのうち最も早いもの。Luo 2014（FSE）は、非同期待ちのフレークの54%が条件待ちで直ると報告している。
- L*/TTTなどの能動学習は採らない（同値性検査が主コスト、決定的Mealy機械しか学習できず、今回は非決定を検出したい）。W法・Wp法・DS法も採らない。全面的なN-switch網羅は13倍になるので、疑わしいキーの直後だけ1-switchに上げる。
- **「各セル2回」は統計的にほぼ無意味**（rule of three: n=2の非決定率の95%上限は150%）。同じ経路の反復では履歴依存は原理的に検出できない。別経路で2周、矛盾したセルと履歴依存の候補だけ10〜20回まで適応的に増やす。
- 未確認: Aho 1991、Chow 1978、Edmonds & Johnson 1973は本文を入手できず二次資料に依拠。センチネル方式がIMEの順序保証のもとで成立するかは実機で未検証。

### 巡回シミュレータ（`crates/awase-keymap-learn`〈旧名`awase-calibration`〉、`feat/awase-calibration`のコミット`1add5e65`・`f492813c`・`321fd4dc`）
純Rust・OS非依存。Mealy機械のモデル、`SimIme`（遅延・キー欠落・観測不一致・非決定を注入）、戦略S0〜S9（S0=現状の毎回リセット、S3=有向CPP、S6=疑わしいキーだけ部分1-switch、S9=全セルを別経路で12回、等）、指標を持つ。`cargo test` 32件が通る。
モデルの仮定と限界: ATOK風モデルの観測層（開閉・変換モード）は格子の実測どおり。入力中の段階の遷移規則は撤去ブランチの追跡規則に基づく**仮定**で、実機で全てを確かめていない。合成モデルの隠れ状態・非決定は乱数で、遅延は
対数正規（中央値60ms）の仮定。

結果（ATOK風モデル、理想条件、S0基準）:

| 待ち方式 | S0（毎回リセット） | S3（有向CPP） | 変化 |
|---|---|---|---|
| 固定待ち | 15.1分・リセット168回 | 8.9分・リセット9回 | 約41%減 |
| イベント待ち | 4.8分・リセット168回 | 0.5分・リセット1回 | 約90%減 |

- 異常（キー欠落2%・観測誤り3%・ドリフト0.5%・リセット失敗5%）を入れても網羅は100%（イベント待ちで0.7〜1.4分）。
- 履歴依存・非決定の検出: S3/S4/S5/S7は非決定15〜35%・履歴依存20〜33%しか検出できず、「決定的」と誤断定した割合は66〜83%。S6は0.7分（S3の1.4倍）で非決定60%・履歴依存53%・誤断定43%。
  S9は検出が最も高く（80%・67%・誤断定26%）、イベント待ちで2.8分（固定待ちでは52分）。S7（矛盾したセルだけを増やす）はこの分では効かなかった。
- 推奨: S6を既定にし、イベント待ちと組み合わせる。疑わしいキーのセルだけ別経路で増やす。分類は誤りに強く（少数派が一定数以上のときだけ非決定と宣言）する。66〜83%は仕込んだ合成モデルの数値で、実際のATOKでの割合は低い可能性がある（未確認）。

## 6. 失敗と訂正（要約）
- 格子第1版（IMM書き込みで状態を作る）は、キーで入った状態と別物で、ATOKのひらがなを「不変」と誤学習した。第2版・第3版で、状態をキーだけで作る形に直した。
- **ADR-189の固定セット（漢字0x19・半角/全角0xF3/0xF4のbeliefトグル）を撤去ブランチで誤って撤去した**。指摘後に復元した（`651cab8d`）。
- MS-IME本体の0xF3/0xF4を「Set型でトグルではない」と断定したのは誤り。ADR-190のCI実測では、awaseなしでF0/F3/F4はトグルする。「F3=OFF・F4=ON」はawase側の静的モデルで、実IMEの挙動ではない。
  復元はGJIとMS-IME本体の両方に適用した。
- 撤去後に英数・カタカナが握りつぶされる疑い（BUG-153）は、実イベントでは元から素通しで、握りつぶしていたのは合成イベントの死んだ分岐だけだった。`dbe_mode_key_policy = Suppress`の対象を0xF3/0xF4だけに縮小した。
- 「閉/開で変換モードの追跡が切れる」という仮説は主因でなかった（上記3.の原因3つが実際の原因）。

## 7. 未解決と次
- カスタムキーマップへの対応（この機能の目的）: 隠れ状態（入力中の段階）を学習した最小のMealy機械として持つ設計。現実装は暫定の固定段階。設定の読み取り（`config1.db`/レジストリ）で測るキーを決め、
  学習した表を実行時に読み込む形にする（現状はコンパイル時の生成データ）。
- 疑わしいキー（取消・確定・変換・削除など）をキーマップのコマンド名から選び、別経路で回数を増やす。誤りに強い分類の実装。
- 実機（Windows）での確認: 撤去ブランチの動作（英数・カタカナ・半角/全角、MS-IME本体の半角/全角のbeliefトグル）、実際のTsfNativeアプリ（Chrome等）での予測。
- MS-IMEの独立walk（表の採点）、通知遅延のキー間隔600msとMS-IME、入力中・確定のイベント化（`WM_IME_*`が届くことの再現性）。
- Ctrl/Altを押したままの文字キーを入力中と誤って追跡する可能性（読めないアプリで残る）。

## 8. 成果物の所在
- 撤去ブランチ（決め打ちの撤去・打鍵時予測・ADR-189の復元）: `feat/adr191-remove-hardcoded-mode-keys`（develop未マージ）。
- PR #237（CI道具: 格子・通知・解析スクリプト、`chore/e2e-calibration-tooling`）、PR #238（BUG-151の最小修正。Opusレビューで取り下げ、close済み。BUG-151は撤去ブランチで扱う）。
- 巡回シミュレータ: `crates/awase-keymap-learn`（旧名`awase-calibration`、`feat/awase-calibration`ブランチ。ADR-176の較正UIと語が衝突するため改名。未マージ）。
- 初期の実験結果とスパイク: `origin/spike/ime-effect-learning`。CI実験用ブランチ: `ci/e2e-adr191`・`ci/e2e-fastgrid`・`ci/e2e-notifygrid`・`ci/e2e-notifyprobe`・`ci/e2e-calibration`（マージ後に整理）。
- 関連BUG: [BUG-151](../known-bugs/BUG-151.md)（起動直後のEngine固まり）、BUG-153（撤去後の英数・カタカナ握りつぶしの疑い、撤去ブランチ上）。

## 付録: BUG-158 のCI経緯（`docs/known-bugs/BUG-158.md` を30行目安へ圧縮した際に移した元の記録）


### BUG-158: 通過マークの観測が失敗すると意図が捨てられず、ポーリングが止まる

**発見経緯:** 2026-09-21、Opus コードレビュー担当Bの指摘(MS-IME本体の実害)をCI(`ci/e2e-msime-native-b`、run 35620507976)で測定。撤去版だけが、
`msime-native`/`sc-dbe`/`sc-kanji`/`sc-shift`(MS-IME本体)各3/3 FAIL、develop 版は全て PASS。`sc-shift` で約18秒、直接入力からの最初のひらがな(F2)でEngineが動かず次のモードキーまで固まる。

**原因:** F2 直後の `[mode-key-follow]` の20ms追随読み取りが `IME snapshot: ime_on=None conv=None`(MS-IME本体のIMMクロスプロセスprobeが空振り)。
ADR-187 は「意図の破棄は観測が**成功した**ときだけ」(`poll_counted_no_new_miss`)なので、直前のスパイクのVK_IME_OFFの `explicit_intent=Some(false)` が残る。
`reschedule_ime_refresh` は明示意図があると早期returnし、読み直しも予約されず、ポーリングが止まる(BUG-151 原因③)。develop 版はF2が `shadow_action`(TurnOn)で
awaseが書くため、通過マーク自体が立たず顕在化しなかった。

**修正(通過マークの意味に立ち返る):** 通過マークは「ユーザーの物理モードキーが通った。結果は分からないので古い意図を根拠にしない」という事実そのもの。意図の破棄を観測の成功だけに頼らない:
(1) 窓の間は、観測の成否・読み取り戦略(OsPoll/SkipTyping)によらず `reschedule_ime_refresh` が `MODE_KEY_PASS_REREAD_MS` の読み直しを予約し続ける(明示意図が残っていても)。
(2) `ir_stage_notify`(毎tick)で、窓が切れても観測が一度も成功していない通過マークについて、古い明示意図を捨てる(`expire_mode_key_pass_mark`)。ポーリングは最悪でも窓(300ms)で再開する。
観測が成功したときの動作(BUG-157: `desired_open` を観測へ揃える)は不変。新しい型・フィールドは無し。

**検証:** 純関数 `should_drop_intents_for_mode_key_pass`(観測成功時は窓の間だけ/窓の終了時は窓が切れて未破棄のときだけ)をLinuxで固定。`platform_state` のテストは `#[cfg(windows)]` 配下で
Linuxでは走らない(最初の実装は窓の間に早く捨てる不具合を含み、CIで検出。上記の純関数へ切り出して再発防止)。
CI(`ci/e2e-msime-native-c`、run 35624165773): F2 の約350ms後に `[mode-key-follow] window expired without a successful observation: intents invalidated` が出て、
以後500msごとのポーリングが再開する(修正前は `explicit_intent=Some(false)` のまま約18秒止まる)。**ただしMS-IME本体の構成は依然 FAIL**(`msime-native`/`sc-dbe`/`sc-kanji`/`sc-shift`)。
原因は別: CIのMS-IME本体では IMMクロスプロセスprobe(cmd=0x0005/0x0001)が約50〜64msで空振りし(`ime_on=None`)、awaseが書かない(`shadow_action`無し)撤去版では
Engineが追随する観測も予測も無い(develop 版は F2 を awase が書くので通る)。MS-IME本体の打鍵時予測表(別作業)または probe の読み取り成功率の改善が要る。

**BUG-151 原因③との関係:** ポーリング停止という症状(意図が残る→`reschedule_ime_refresh`早期return)は本修正で解消。
#### 追補(2026-09-21、MS-IME本体の表との組み合わせで `imm-learning` の誤降格)

**観測した失敗条件:** CI `ci/e2e-msime-native-e`(MS-IME本体の表+BUG-158の修正)で `sc-dbe-msime-native-suppress` が3/3 FAIL(表のみの `-d` は全PASS)。
awase.log(`sc-dbe-msime-native-suppress-1`): `[mode-key-follow] window expired without a successful observation` が41回、`IME detection failed 3 consecutive times`、
`IMM capability learned: ime_key_matrix_spike.exe/Edit → Unavailable (miss 2→3)`、`[imm-learning] profile 降格: ... Standard → Imm32Unavailable`。
**機序:** MS-IME本体(CI)のIMMクロスプロセスprobe(cmd=0x0005/0x0001)は1回50〜100msかかる。窓の間 60ms ごとに読み直す(上の修正)とprobeが重なり(ThreadId が同時に複数)、
`ime_on=None` が3回連続 → `learn_imm_capability_from_miss`(`IME_DETECT_MISS_THRESHOLD`=3)が、cache.toml の事前投入(`Edit = "works"`)を上書きして `Unavailable` を学習 → 降格すると観測で訂正できない。
表のみの版はポーリングが止まっていたのでprobeが走らず降格0件だった(止まっていたのが降格を隠していた)。

**見直し(probeの頻度側、`imm_learning` の閾値は触らない):** 窓の間の読み直しは、直前の読み取りが成功したときだけ `MODE_KEY_PASS_REREAD_MS`(60ms、ADR-187)、
失敗したとき(`detect_miss_count() != 0`)は窓の終了時の1回だけにする(`mode_key_pass_next_read_ms`、純関数でLinuxのテストで固定)。窓が切れたら `ir_stage_notify` が古い意図を捨てる動作、観測成功時の BUG-157 の動作は不変。

**追補2(読めない窓、`0a590cdd`):** 統合ビルドのCI(`ci/e2e-drift-fix2`)で、`cal-verify-blind`(Editを`Imm32Unavailable`へ降格した読めない窓)のEngineずれが 0%/1.0% → 23.2%/21.4% に悪化
(`window expired ...` がblindでも33回。読み取り自体ができない窓の意図=beliefの唯一の手がかりを窓終了時に捨てていた)。読み直し予約と窓終了時の破棄を `can_use_imm32_cross_process()` が真の窓に限った。
**検証(CI、統合ビルド=MS-IME本体の表+BUG-158見直し+読めない窓の除外):** GJI側(`ci/e2e-drift-fix2`、run 35627618162): `cal-verify-obs` 400ms以降 0%(seed1・2)、`cal-verify-blind` 0%/1.0%、`[key-effect-miss]` 0件、
`real-cold` 3/3・`real-sc-dbe/kanji/shift` 全PASS(`real-sc-f2x4` の rc=3 は既知の判定器の限界)、想定外の降格0件(blind の1件は事前投入した `Edit = "unavailable"` によるもの)。
MS-IME本体(`ci/e2e-msime-native-f`、run 35627580266): 24構成中21 PASS。`sc-dbe-msime-native-suppress` 3/3 FAIL(修正前)→ 1/3 FAIL、`-passthrough` 2/3 FAIL、他(`msime-native`、`sc-kanji`、`sc-shift`、`sc-hz`、`msnat-*`)は全PASS。
**未解決:** `sc-dbe-msime-native-*` の残る3件は、いずれも `imm-learning` の降格(3回連続 miss)を伴う。原因はCIのMS-IME本体で F2 直後のIMMクロスプロセスprobeが約50msのタイムアウトを1.5秒ほど繰り返すこと
(通常の500msポーリングだけで3回連続に届く)。develop 版では同条件で降格0件。`IME_DETECT_MISS_THRESHOLD`(3)は tuning-constants/BUG-56 の領域なので触っていない。

#### 追補3(2026-09-21、学習側の修正: 時間切れを「IMM不可」の証拠に数えない)

**本来の原因:** `ime_on=None`(読み取りの空振り)を、理由を問わず `miss` に数え、3回連続(`IME_DETECT_MISS_THRESHOLD`)で `imm-learning` が `Unavailable` を学習する。
missの入口は `ImeSnapshot::classify_poll_outcome` の else 分岐(`increment_miss_count`)で、空振りの理由は次の4つが区別なく `None`:
(a)`ImmGetDefaultIMEWnd`=NULL(即時、IME窓なし)、(b)`SendMessageTimeoutW` の時間切れ(`ERROR_TIMEOUT`、宣言50ms)、(c)`SendMessageTimeoutW` の即時失敗(`ERROR_ACCESS_DENIED`=昇格プロセスへのUIPI拒否等)、
(d)読み取り全体のワーカータイムアウト(300ms)。(a)(c)は「IMMが使えない」証拠になるが、(b)(d)は遅い応答(負荷・忙しいIME・CIの遅いランナー)で証拠ではない。

**実測(CI、MS-IME本体、awase.log の `[ime-io] cross_process ... kind=probe` の `elapsed_us`、cmd=0x0005/0x0001):**
develop 版 n=2440(p50=0.1ms、p95=50.1ms、p99=62.5ms、最大94.6ms、50ms以上=5.0%)、撤去版(表なし)n=5156(2.2%)、表のみ n=5028(2.2%)、表+BUG-158 n=7183(p99=59.5ms、3.0%)。
**二峰性**: 成功側は最大 50.0ms(p99 25〜34ms、p50 0.06ms)、時間切れ側は最小 50.0ms(宣言50ms+スケジューリング)。よって `elapsed_us >= 宣言timeout` で時間切れと判別できる(`ERROR_TIMEOUT` と併用)。
**本当にIMM不可のアプリの現れ方(実データ、ユーザー実機 `awase-1.10.1/awase.log`〈INFOレベル〉):** `IMM capability learned: DirectUIHWND → Unavailable (miss 2→3)`(TaskManagerWindow、昇格プロセスの可能性)と
`TframeMainFunMenu → Unavailable (miss 2→3)`(iscrrec.exe)は、フォーカス後 約500msごと3回の miss。直前に `ImmGetDefaultIMEWnd=NULL` のINFOは無く、IME窓は取れていた。ログがINFOで `elapsed` が無いので、
時間切れか即時拒否かは**未確認**(仮説: 昇格プロセスは UIPI の `ERROR_ACCESS_DENIED` で即時失敗)。一方 `Qt663QWindowIcon → Works (miss 1→0)` は `run_with_timeout: worker thread exceeded 300ms` の直後で、Works のアプリでも時間切れの空振りが実在する。

**修正(学習側の意味、閾値は不変):** `ImeSnapshot::probe_timed_out`(`send_failure_is_timeout`: `ERROR_TIMEOUT` または `elapsed_us >= 宣言timeout` の失敗、読み取り全体のタイムアウト)を追加し、`ime_on=None` の理由が時間切れなら
`classify_poll_outcome` が `increment_miss_count=false`(観測を書かず belief を保つ)にする(`read_miss_is_imm_evidence`)。即時の拒否・IME窓なしは従来どおり数える(本当にIMM不可のアプリは従来どおり降格)。
`IME_DETECT_MISS_THRESHOLD`(3)とプローブのタイムアウト値(50ms)は変えていない。副作用の整理: 時間切れは `miss_count` を増やさなくなったので、通過マークの追随(ADR-187)の「観測が成功したか」は
`ir_poll_and_learn` の戻り値(`observer_poll` が得られたか)で判定し、読み直し間隔用に `Runtime::last_ime_read_ok` を持つ。
**テスト:** `timeouts_are_not_imm_evidence_but_immediate_refusals_are`(Linux、時間切れ3連続=0/即時拒否3連続=3に届く/混在)、`timed_out_read_does_not_increment_miss_count_but_refusal_does`(windows)。

**検証(CI、学習側の修正後):** MS-IME本体(`ci/e2e-msime-native-g`、run 35630244701): 25構成中24 rc=0(`sc-dbe-msime-native-{suppress,passthrough}` 各3/3 PASS、`msime-native`・`sc-shift`・`sc-hz`・`msnat-*` 全PASS)、
想定外の降格0件(`msnat-blind` の各1件は事前投入した `Edit = "unavailable"` によるもの)。残る1件は `sc-kanji-msime-native-1`(1手目のF2で、表が「開」と予測したが実IMEは閉のまま=cold の初回F2が効かない回。降格・時間切れとは無関係)。
probe の応答時間(このrun、n=5537): p50=0.06ms、p95=20.7ms、p99=62.1ms、最大156.7ms、50ms以上=3.6%(修正前と同じ二峰性)。時間切れとして数えなかった読み取りは 23 ログ中に出た(`IME detection timed out ... not IMM-unavailable evidence`)。
GJI側(`ci/e2e-drift-fix3`、run 35630377273): `cal-verify-obs` 400ms以降 0%、`real-cold` 3/3・`real-sc-dbe/kanji/shift` 全PASS、想定外の降格0件。`cal-verify-blind` は s1 0%・s2 14.3%(前回 0%/1.0%)。s2 のずれは
モード軸(ON/英数 なのに Engine=ON 等)で時間切れの読み取りは0件(読めない窓は読み取りをしない)ため今回の修正とは無関係のばらつきと見ているが、原因は未調査。
**未解決:** MS-IME本体の構成で、起動後(最初のVK_IME_OFF/F2以降)の `[drift] correction` が develop 版(1〜3件)より多い(sc-dbe 6〜8、sc-shift 11〜14、msime-native 22〜24)。通過マークの窓の間の読み取りが全て時間切れだった回、
`desired_open` を観測へ揃える(BUG-157)機会が無く、`observed=true ≠ desired=false`(desired は起動時のVK_IME_OFFの古い値)が10秒以上続き `set_ime_open(false)` を繰り返す(CIのMS-IME本体では書き込みが効かないため無害だが、実機で効くなら
ユーザーのF2を閉じ直しうる)。時間切れを miss に数えなくなった副作用ではなく、BUG-157 の「揃える機会」の問題(窓の間に成功した観測が無い)。次: 窓の終了後の最初の成功観測でも揃える。

#### 追補4(2026-09-21、窓が切れた後の最初の成功観測でも desired を揃える)

**残っていた問題:** 通過マークの窓の間の読み取りが全て時間切れだった回、BUG-157 の「`desired_open` を観測へ揃える」機会が無く、窓が切れた後も `desired=false`(起動時のVK_IME_OFFの古い値)と
`observed=true` の乖離が10秒以上続き `set_ime_open(false)` を繰り返す(CIのMS-IME本体の起動後の `[drift] correction`: `sc-dbe` 6〜8、`sc-shift` 11〜14、`msime-native` 22〜24 = develop 版 1〜3 件の数倍。
書き込みが効く環境では、ユーザーのF2を閉じ直しうる)。
**修正(最小):** 既存の通過マーク(`ModeKeyPassMark`)に2つのboolを足す。`aligned`(揃えたことがあるか)、`awase_wrote`(通過より後にawaseが実際にIMEへ書いたか=`record_optimistic`/`record_confirmed`)。
窓が切れた後の最初の成功観測で、`should_align_after_expired_mode_key_pass`(窓が切れている/未揃え/awaseが書いていない/新しい明示意図が無い)なら `ModeKeyPassedThrough` を1回dispatchして揃える(`align_after_expired_mode_key_pass`)。
揃えた後は通常の drift correction に戻る(永続的に無効化しない)。awaseが通過後に書いた場合は揃えず、書き込みが届かなかったなら drift correction が訂正する。dispatch元は `pass_through_observed` の1箇所(architecture_guard)。
**テスト:** 純関数 `should_align_after_expired_mode_key_pass_only_once_and_not_after_awase_write`(Linux)、`platform_state` の Windows専用テスト2件(通過→観測なし→窓切れ→最初の成功観測で揃い2回目は揃えない/awase書き込み後は揃えない。Linuxでは走らず windows-build CI で実行)。

