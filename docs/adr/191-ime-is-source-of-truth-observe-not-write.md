---
id: ADR-191
title: |-
  IMEの状態はIME自身を正とし、awaseは書き込まず観測・予測に追随する（設計転換）。開閉のみに作用するキー（冪等ON/OFF・トグル）だけはawaseが書いてよい。キー効果は設定の読み取り・注入学習・検証の3段階ラウンドで表にする
summary: |-
  awaseはIMEの開閉・変換モードを書き込む経路を累積させ、その書き込みがIME自身の動きから外れてモードずれ（IMEは半角英数なのにEngineがON、等）を生んできた。
  実機計測（GJI×ATOK、スパイクの`--walk`。`--exp`は`spike/ime-effect-learning`の`91f17341`時点にだけ存在し、現行のCI道具PR#237には無い）で、IME単体の一段の効果はMozcの公開キーマップから
  ほぼ予測でき（98.5%）、awaseを通すと仕様から外れる（84.5%、Engine/実IMEのずれ19〜23%）。方針: (1)IMEを状態の正とし、通常はawaseがIME状態を書かず、生キーを通して観測に追随する。
  (2)awaseが書いてよいのは、押した結果がIMEの開閉だけに作用するキー（冪等な`VK_IME_ON/OFF`、beliefに基づく開閉トグル=ADR-189の漢字0x19・GJI/MS-IME本体の半角/全角0xF3/0xF4）だけ。
  ひらがな・カタカナ・英数・無変換・変換など、入力モードや入力中文字列にも作用しうるキーは書かず追随する。線引きはキーのVKでなく、表（IME種別×プリセット×status）の作用分類(a〜e)で決める。
  (3)BUG-151の最小修正（決定2、shadow-toggleがno-opで終わった打鍵に通過マークを立てる）は、撤去と切り離してdevelop上に新規実装する（PR #238、draft）。
  (4)キー効果は(状態,キー)→効果の表として持つ。表は、設定の読み取り→awaseを完全バイパスした注入学習（格子第3版=全状態をキーだけで作る）→独立walkでの検証、の3段階ラウンドで作る。
  カスタムキーマップに対応することが目的なので、隠れ状態（入力中の段階）も固定の名前・規則でなく学習した最小のMealy機械として持つ（現実装は暫定の固定段階）。
  (5)予測は物理キーの打鍵時点でbeliefへ反映し（`KeyEffectPredicted`、settle基準のfence）、観測は確認と訂正に回す。観測できないアプリ（TsfNative）では予測が唯一の信号になる。
  (6)成功基準は撤去量（追加は削除と対）。実装は`feat/adr191-remove-hardcoded-mode-keys`（develop未マージ）、実験の経緯と実測は補助資料[191-calibration-experiments.md]に置く。
status: |-
  **草案（2026-09-21）。opus敵対レビューround1〜4を実施し、指摘への対応を本文末尾の表にまとめた（停止条件・中止基準・複雑さの収支を含む）。実装は撤去ブランチ（未マージ）。**
  決め打ちの撤去・打鍵時予測・ADR-189の復元は撤去ブランチ`feat/adr191-remove-hardcoded-mode-keys`で実装済み（develop未マージ、CIで検証: 観測あり・読めない条件とも400ms以降ずれ0%）。
  決定2（BUG-151の最小修正）は別PR #238（draft）。実機（Windows）での撤去後の動作確認は未実施。
related_adr:
  - "ADR-138"
  - "ADR-162"
  - "ADR-176"
  - "ADR-186"
  - "ADR-187"
  - "ADR-188"
  - "ADR-189"
  - "ADR-190"
  - "ADR-192"
  - "ADR-193"
---

# ADR-191: IMEの状態はIME自身を正とし、awaseは書き込まず観測・予測に追随する（開閉のみに作用するキーは例外）

> ファイル名の`observe-not-write`は初期のスローガンで、決定1の例外（開閉のみに作用するキーはawaseが書いてよい）と、決定3の打鍵時予測（観測を待たずbeliefを更新する）を含まない。リンクを壊さないため、ファイル名は変えていない。

## ステータス

草案（2026-09-21）。opus敵対レビューをround1〜4まで実施した（round1: Blocker7・Major14・Minor6、round2・3・4も同様に新規指摘）。round1・2の指摘のうちコードとログで独立に確認できたものは本文へ反映済み。
round3・4の指摘への対応（反映・既に反映済み・見送り）は、本文末尾「Opus round3・4 の指摘への対応」の表にある。停止条件・中止基準は決定1、複雑さの収支は決定5に置いた。
round4の「ADR-192決定3b」の指摘はADR-192側で訂正済み。
- 実装: 決め打ちの撤去・打鍵時予測・ADR-189の固定セットの復元は、撤去ブランチ`feat/adr191-remove-hardcoded-mode-keys`にある（**develop未マージ**、下記「実装の現状」）。決定2（BUG-151の最小修正）は、
  撤去と切り離してdevelop上に新規実装し、別PR #238（draft）にした。
- 測定の出典: 初期の実測（決定の根拠）は`spike/ime-effect-learning`ブランチ（`tools/e2e/ime_key_matrix/`、結果は`results/elw2`・`elw8`・`elw9`）。格子・通知・検証ラウンドのCI測定は、
  補助資料[191-calibration-experiments.md](191-calibration-experiments.md)にrun URL付きでまとめた。スパイクの`--exp`は`spike/ime-effect-learning`の`91f17341`時点にだけ存在し、現行のCI道具PR（#237）には無い
  （BUG-151.mdの再現手順の`--exp=70:n:12`は、現行コードでは実行できない）。

## 背景

awaseの最大の難所は、IMEのON/OFF/変換モードの追跡である。追跡が外れるたびに「awaseがIMEへ書き込んで揃える」経路を足してきた
（[ADR-158](158-complexity-reduction-north-star.md) RC4が指摘した、加算のみを報いる構造）。書き込みは「UIミラーにすぎず実モードに届かない」
（BUG-25）ことがあり、awase自身の書き込みが観測を汚してIME本来の動きから外れる（ADR-176決定1が警告した自作自演）。

ユーザー方針（2026-09-20）: IMEの書き込みを尊重する（IMEを状態の正とする）ようawaseの設計を大幅転換する。そのためのキャリブレーションを作る。
成功基準は撤去量。ただし**トグル系のキーだけは、IMEが今の実状態に依存して結果を変えるため、awaseがbeliefに基づいて送る**（唯一の例外）。

## 実測（要点と限界）

実機（GJI×ATOKプリセット、スパイクの標準Edit入力欄。実打鍵の結果を記録）。**B（awase経由）の測定に使ったawaseは`e2e/ablation`（コミット5f11a872、ADR-186＋実験用の変更）で、
ADR-187（follow）・ADR-188・ADR-189（半角/全角のToggle上書き）の実装を含まない**（developの現行とは違う）。Bの結果をdevelopの現行の性質として読まない。ログの「ROUND 2/2: RichEdit」の見出しは、フォーカス制御が入力欄（Edit）へ戻すので実態を
表さない（awaseの`imm-cross-actuate`の対象は`class="Edit"`、確定文字は`Edit`の内容に入る。レビューが「RichEditで測っていた」としたのはこの見出しの読み違い）。**測定台は標準Edit（ImmCross）1種類のみで、RichEditや他の`AppImeProfile`のデータは存在しない**（表を全アプリへ適用してよい根拠の限界）。

1. **IME単体は仕様どおり。** `--exp`（ON・かなでひらがな0xF2→`a`）は、awase完全バイパス（A'）・再注入あり（A）・awase経由（B）の全条件で12/12「ONのまま半角英数へトグル」。
   Mozcの`atok.tsv`（Precomposition `Kana`=ToggleAlphanumericMode、DirectInputの`Kana`は未定義）と一致。0xF2はMozcの`KeyEvent::KANA`（`keyevent_handler.cc`）。
2. **Mozc仕様だけのモデル（学習なし、`effect_learning.py --spec`、`atok.tsv`の転記と、tsvに載らない状態の写像の推定）の一段予測精度**（各2ラン・200押下、分母は全条件で共通、
   半角/全角の0xF4は0xF3として集計。押下前の状態の関数で、正規化はむしろVK識別子の漏れの除去）:

   | 条件 | 仕様モデル | 学習表（他ランで学習、未学習は誤答） | （学習表の既知セルのみ） |
   |---|---|---|---|
   | A'（完全バイパス） | 98.5% | 78.5% | 95.2% |
   | A（再注入あり） | 99.0% | 85.5% | 98.8% |
   | B（awase経由） | 84.5% | 89.0% | 96.2% |
   | **未見データ（elw9、新シード3・4、A'）** | **98.5%** | 81.0% | 97.6% |

   **読み方と限界**: (a)仕様モデルは学習なしで一段98.5%。未見データ（elw9、コミット済みのモデルを評価）でも98.5%（197/200）で、in-sampleの上限にとどまらない（学習表をelw8で学習してelw9で検証しても
   98.5%）。ただし`atok.tsv`のうちConversion・Suggestion・Predictionの状態は表現していない。未見データの誤答3件はすべて「入力中」（Esc/無変換）で、Escが入力中のまま残る例は、サジェスト/候補ウィンドウが出ている間の1回目のEscがウィンドウを閉じるだけ、というSuggestion/Predictionのstatusと整合する（決定4の観測で検証する）。(a')**未見データの限界（事前登録なし）**: elw9の45セルはすべてelw2/elw8で既出で、未見なのは「ラン」であって「セル」ではない（系列への汎化の検証にとどまり、新しいセルの検証ではない）。合格ラインは
   結果を見る前に書いていない（事後）。半角/全角の**トグルキーだけの一段精度はA'（elw8+elw9）で52/52**（0xF3の生VKは押下前状態を漏らすので0xF3/0xF4を1キーに正規化した。生VKの分布はelw9で0xF4が23・0xF3が11）。
   elw9のA'は`(injected)`が0件、TIPはGJIを一次記録。
   (b)**Bで仕様モデルが外れるのはランダムな崩れではなく、awaseの書き込みという決定的な規則で上書きされているため**
   （外れた31件は`OFF+ひらがな→ON`＝`Activate`固定、`入力中+無変換/変換→OFF`＝代行、`ON/英数+半角/全角→不変`＝Suppress等、awaseの書き込み一覧そのもの）。よって**学習表はawaseを完全に
   バイパスして測ったときだけIMEの表になる**（決定4がA'を要求する理由）。(c)A'とAの差は統計的に区別できない。(d)実効標本は押下数ではなくセル数（約40）に近い。(e)elw2は起動時のTIPを一次記録して
   いない（遷移パターンからの事後同定）。TIPを一次記録したのはelw8以降。(f)**開ループ追随は指標として壊れている**（連鎖の再同期の回数に強く依存し、A'で84.5→64.0%、未見で37〜56%と条件間で
   単調でない）。言えるのは「1手の誤りが後続へ連鎖し、開ループ単独には頼れない（観測が要る）」ことだけで、条件間の比較には使わない。
2b. **Engineと実IMEの一致（押下+400ms、`--drift`。opus round3が既存ログから算出、未再検証）**: A'はEngineが無いので測定不能。A（素通し・観測ポーリングのみ）はずれ48%（A1）／63%（A2）、
   B（awase経由、ablationビルド）は22%（B1）／18%（B2）。**上の「Engine/実IMEのずれ19〜23%」は手元の全条件で最良の値**で、決定1の動機として「Bが悪い」とは読まない。Aの48/63%は、(i)スパイクのキーが`injected`で
   `kp_stage_mode_key_follow`が`event.injected`で早期returnするためfollowが構造的に発火しない、(ii)測定窓+400msが`TYPING_IDLE_MS`=500より短く、定義上まだ一度も読めていない、の2つによる
   「20msの再読み取りが無いときの床」であり、素通し追随の性能ではない。素通し追随の測り方は決定1の「条件C」。
3. **表だけでは決まらない分岐が少数ある**（入力中の無変換など）。Mozcのキーマップはstatusを`DirectInput`/`Precomposition`/`Composition`/`Conversion`/`Suggestion`/`Prediction`に分け、
   バインドがstatusごとに違う。私の状態の分け方（開閉・かな/英数・入力中）にはConversion/Suggestion/Predictionが無く、これが分岐の原因の候補（候補ウィンドウの有無で
   区別できる。決定4）。
4. **BUG-151の決定的な再現と原因**（[BUG-151](../known-bugs/BUG-151.md)）。coldのawaseでひらがなを押しGJIが半角英数になった後、Engineが追随せず次の`a`がNICOLAの`う`になる（`--exp` Bで12/12）。
   ①ひらがな（0xF2）は`shadow_action=TurnOn`（`vk.rs`の`0xF2=>Activate`）で、IMEが既にONのため`[shadow-toggle] no-op ... apply-ime 見送り`（`applied`はUnknownのまま）。ADR-187のfollow
   （`kp_stage_mode_key_follow`）は無変換/変換だけが対象で、ひらがなは通過マークを持たない。②20ms後の再読み取りは`may_change_ime`で予約されるが`ir_decide_read_strategy`のtyping-idleガード
   （500ms）に当たり、バイパス`explicit_verify`（`mode_key_pass_live || (explicit_intent かつ applied != Unknown)`）は効かず毎回スキップ（`idle=31〜47ms`）。③`explicit_intent`が確定しているので
   `reschedule_ime_refresh`は後続のポーリングも止める。warm（`applied`既知）では②のバイパスが効き再現しない（`--walk`のB2で`ime open applied`が43件、`--exp`のcoldで2件）。
5. **Bで仕様モデルが外れた31件の内訳**（B1・B2、awaseの完全ログはB2のみ。上記のビルドの挙動）: (i)**11件** ひらがなをIME OFFで押すとONになる（仕様はOFFのまま。
   `shadow_action=TurnOn`＋ImmCross書き込み、`0xF2=>Activate`固定）。(ii)**9件** 入力中の無変換/変換でIMEがOFFになり未確定が破棄される（仕様は半角英数トグル/変換。単独タップ代行`delegate`、
   この機は`gji_thumb_key_ime_toggle=true`）。(iii)**7件** 半角/全角をON・英数（±入力中）で押しても閉じない（7/7。ON・かなから押した6/6は反転する）。**ADR-189を含まないビルドの静的な方向固定
   （0xF4=TurnOn）が実状態ONでno-opになる、ADR-189が直そうとした症状そのもの**で、ADR-189の効果はこのデータでは測れていない（Engineと実IMEの不一致が0/18だったのは、Engineが実IMEと
   整合していたという意味で、キーが反転した証拠ではない）。(iv)**4件** 入力中（英数）のEsc/無変換が仕様どおりにならない。awase無し（A'）でも出るIME側の挙動で、Conversion/Suggestion/Predictionの
   状態を持たない仕様モデルの限界（決定4の観測で検証）。(i)(ii)は当時のビルドにも現行developにもあるawaseの代行で、(iii)はdevelopでは直っている見込み（要再測定）。
   ずれの大きい経路は無変換13/23・変換6/19（ADR-187の追随が対象にする親指キーを消費する経路。不一致は後続の打鍵にも残るので押下ごとの帰属は概算）。
6. ひらがなの静的な決めつけ（`0xF2=>Activate`）は実際にIMEごとに違う。GJI(ATOK)はDirectInputで何もせずON・かなで半角英数トグル、MS-IMEはDirectInputで開きON・かなで閉じた
   （別ランの計測、TIPはelw6で一次確認）。

## 決定

### 決定1: 原則 — IMEが状態の正。ただしトグル系のキーだけはbeliefに基づいて送る

**適用範囲**: IMEの状態をIMMのクロスプロセス読み取りで観測できるアプリ（`can_use_imm32_cross_process()`が真）。TsfNative（Chrome・Windows Terminal等。`skip_imm_query`で開閉もconvも読めない）は
本ADRの範囲外で、[ADR-188](188-tsfnative-conv-only-mode-key-engine-follow.md)側で扱う（そこでは「観測に追随」も「観測が勝つ」も定義できない）。**ただし下の固定の例外（ADR-189）は
観測に依存しないので、TsfNativeを含む全アプリで従来どおり効かせる**（本ADRで変えない）。

**原則**: キーは可能な限り生のままIMEへ通し（PassThrough）、awaseは結果を観測して追随する（ADR-187のfollow方式）。awase側の推定は観測を書き換えない
（ADR-098、`.claude/rules/ime-belief-architecture.md`の不変条件は維持）。表題の「IMEが状態の正」は、**conv軸（かな/英数・ローマ字）については原則どおり**、
**開閉軸ではトグルキーに限りawaseが正**、という意味である（例外を入れた以上、これを曖昧にしない）。

**awaseが書いてよいか否かの線引き（2026-09-21、ユーザー整理）**: 押した結果が**IMEの開閉（ON/OFF）だけ**に作用するキーは、awaseが書いてよい。
純粋に冪等なON/OFF（`VK_IME_ON`/`VK_IME_OFF`、`keys.ime_on/off`、単独打鍵の`"on"`/`"off"`）はもちろん、beliefに基づく開閉のトグル（ADR-189、`keys.ime_toggle`）も同じ扱いである。
**書いてはならないのは、ON/OFF以外の作用も持つキー**（ひらがな・カタカナ・英数のような入力モードを変えるキー）で、これらはIMEに任せ、awaseは観測して**追随**する。
開閉と入力モードを同時に変えうるキーを、開閉だけの都合でawaseが代行すると、モード軸のずれ（ADR-191の実測の外れ31件のグループ1）を作る。

**線引きは決め打ちせず、設定の読み取りと較正で作る表から引く（2026-09-21、ユーザー指摘。opus round4のB・Blocker「ATOKで`VK_IME_OFF`は未確定文字列を破棄する」への答え）**:
「開閉だけに作用するか」は、キーのVKで決まらず（IME種別, キーマッププリセット, status）の関数である。よって、表（決定3）の各セルに**作用の分類**を持たせ、awaseが書いてよいかはこの分類だけで決める。
- 分類: (a)**開閉のみ・Set型**（送れば冪等。書いてよい）、(b)**開閉のみ・トグル**（beliefに基づく。書いてよい）、(c)**入力モードを変える**（ひらがな/カタカナ/英数、`ToggleAlphanumericMode`等。書かない・追随）、
  (d)**入力中の文字列に作用する**（破棄`CancelAndIMEOff`／確定`IMEOff`など、statusがComposition/Conversionのとき。書かない・追随。ATOKの`VK_IME_OFF`はここに当たる）、(e)未知（書かない・追随）。
  **書いてよいのは、そのstatusでのセルが(a)か(b)と確定しているときだけ**。それ以外・未較正・状態不明は、常に「書かずに追随」に倒す（安全側）。
- **出所は2つを突き合わせる**: (1)**設定の読み取り**: GJIは`config1.db`（`session_keymap`+`custom_keymap_table`+`overlay_keymaps`）とMozc公開キーマップ、MS-IMEは`msime_key_assignment`が既に読む割り当てとMS-IMEの既定。
  コマンド名から分類へ写す（`IMEOn`/`IMEOff`は開閉、`CancelAndIMEOff`は破棄を伴うので(d)、`ToggleAlphanumericMode`は(c)）。(2)**較正での検証**: 設定から作った分類を、注入で実IMEに当てて確かめる。
- **較正の強化（決定4に反映）**: 開閉・入力モード（conv）に加え、**入力中の文字列の行方（保持/確定/破棄）**を観測して、(d)を実測で判定する。statusごと（DirectInput/Precomposition/Composition/Conversion）に、押す前の状態を作って押す。
  設定から読んだ分類と実測が食い違うセルは、実測を採り、書かない側（追随）に倒す。カスタムキーマップのユーザーは、この較正で初めて(a)(b)と確定できる。
- **順序（opus round4の循環の指摘への答え）**: 決定1の書き込みの例外は、この表ができてから有効にする。撤去は表の後、または表が無いセルは「追随」で動くので撤去は先行してもよい（例外を後から足す）。

**`VK_IME_ON`/`VK_IME_OFF`が「開閉だけ」なのは経験的な例外（opus round4 QB3）**: ATOKでは`VK_IME_OFF`は`CancelAndIMEOff`（入力中の文字列を破棄）、MS-IMEプリセットでは`IMEOff`（確定）で、作用はpresetごとに違う。
固定セット（ADR-189）は「CI実測で動くことが確認済みの経験的な例外」として据え置き、表駆動への一般化（分類a〜e）は、表と較正ができてから有効にする。

**例外は2段。いずれも「トグル」（押した結果が今の実状態に依存して反転するキー）だけが対象。**
1. **固定の例外（撤去はいったん行ったが、誤りと分かり復元した。下記「実装の現状」参照）**: ADR-189のセット（`VK_KANJI`0x19・`VK_DBE_SBCSCHAR`0xF3・`VK_DBE_DBCSCHAR`0xF4。撤去ブランチでは0x19はどのIMEでも、0xF3/0xF4はGJIとMS-IME本体で、無修飾のときbeliefに基づく開閉トグル）と、ユーザー設定`keys.ime_toggle`。
   awaseが物理キーをSuppressし、beliefから目標（`!belief`）を決めて冪等な`VK_IME_ON`/`VK_IME_OFF`で書く。観測に依存しないのでTsfNativeでも効く。根拠はADR-189自身のCI実測（実装前は8手順中4手順が反転せず、
   実装後は各3/3で全8手順が反転しEngineも追随）。**本ADRの実機測定はこのビルド（ADR-189込みのdevelop）でまだ測っていない**（上記実測5(iii)。developビルドでの再測定を、例外を残す根拠として要する）。
   決定6の「決め打ちしない」に対する、**静的に残す唯一の明示的な例外**。
2. **表駆動の追加（厳しい条件を満たしたキーだけ）**: 較正した表が下記を**すべて**満たすと判定したキーを、上と同じ方式の対象に加える（IMMで読めるアプリのみ）。
   - **状態完備**: そのキーが、定義済みの**全ての開状態**（Precomposition・Composition・Conversion）で同じ軸の同じ方向に反転し（開閉軸なら全開状態で閉、閉状態で開）、
     かつ`<各開状態> OFF`行が対象キーの行と同一コマンドである。**1つでも開状態が未測定・未割当なら対象にしない**（Mozcに「トグル」というキー単位の属性は無く、状態別コマンド割当の
     副産物にすぎない。ATOKの変換キーはDirectInputでIMEOn・PrecompositionでCancelAndIMEOffだが、CompositionではConvertで、2状態だけ測ると完全なトグルに見えて入力中の変換を壊す）。
   - 修飾なし（表の署名は`(状態, 修飾, キー)`。修飾付きは対象外）。信頼度の下限（全開状態を実測済みで、n回以上一致）を満たす。疑わしきはトグルとしない。
   - 対象の軸: **開閉軸**（冪等なSet型`VK_IME_ON`/`VK_IME_OFF`がある）、および**入力モード軸**（かな⇔英数、ローマ字/かな等）は**その軸にSet型のキー/VKが表にある場合だけ**。
     Set型を送れば実状態は目標に揃い、beliefが誤っていても次の押下で収束する。**Set型が無い軸では例外にしない**（GJIのATOKには入力モード指定のSet型`CompositionMode*`が0件で、
     トグルを「食い違うときだけ1回送る」方式は、beliefが誤っていると毎回ユーザーの意図と逆に反転して収束しない位相ずれが続き、物理キーをSuppressしなければ二重に反転してキーが死ぬ。
     ATOKの入力モード軸は、生キーを通して観測に追随する）。ユーザー指示のかな⇔英数の冪等化は、Set型のあるIME（MS-IMEプリセットの`CompositionModeHiragana`等）で有効になる。
   - **確定条件（未測定）**: このうち追加分（2）は、実機で「素通し追随」（生キーを通してADR-187型のfollow）と「beliefに基づく送信」を比較し、後者が明確に減らすと確認できてから有効にする
     （ADR-189の実測は静的なSuppress+固定方向との比較で、素通し追随との比較は一度も無い）。確認までは、追加分は無効（1だけが有効）。

**固定の例外と表が矛盾したとき（round3 RM3）**: **固定が常に勝つ**。表が固定セットのキーを「トグルでない」と判定しても動作は変えず、警告をログに出すだけにする（分岐を増やさない。カスタムキーマップの
ユーザーで固定セットが誤っていれば、ADR-192の検出・警告の対象になる）。

**条件C（素通し追随・予測の測り方、round3 RB2）**: `AWASE_TEST_INJECTION=1`（スパイクのキーを物理扱いにする）＋ 素通し設定 ＋ 評価対象のビルド。**条件A/A'では測れない**: 条件Aでは注入キーが外部注入として扱われ
`kp_stage_mode_key_follow`が`event.injected`で早期returnし、followがどのビルドでも発火しない。撤去ブランチのCI `cal-verify-*`（`AWASE_TEST_INJECTION=1`、撤去ブランチのビルド）は実質的にこの条件Cで、
観測あり400ms以降0%・読めない条件0%（[補助資料](191-calibration-experiments.md)）。**未測定**: developビルドの条件C（撤去前との比較）。

**中止条件（決定1本体、round3 RB1。数値は暫定で、根拠は本ADR時点の実測）**: 次のいずれかに当たれば、撤去ブランチはdevelopへマージしない（マージ後なら撤去をrevertする。ADR-189の固定セットは撤去していないので戻す範囲は限られる）。
- CI `cal-verify-obs`（観測あり）の押下+400ms以降のEngine/実IMEのずれが**5%を超える**（基準0%、2 seed×約100押下。5%は決定2の合格ラインと同じ暫定値）。
- CI `cal-verify-blind`（読めない条件）の押下+1500msのずれが**20%を超える**（基準0%。第3版の表を入れる前の値9〜20%を「戻す」境界にした暫定値。TsfNativeは自己責任・ベストエフォートなので厳しくしない）。
- `[key-effect-miss]`が観測ありで**200押下あたり10件以上**（第2版の表で10〜11件だった水準。基準0件）。
- windows-build CI（`transport.rs::plan_tests`など`#[cfg(windows)]`のテスト）が落ちる、または実機のA/Bで英数・カタカナ・半角/全角が「効かない」と確認される。
- 実際のTsfNativeアプリ（Chrome等）の実打鍵で、強制ON/OFFの打鍵（開閉軸）でも回復できない固まりが1件でも再現される。

**書き込みの規則（1・2共通）**:
- 書き込みの入口は既存の1つ（`dispatch_ime_set_open`相当）に集約し、経路を増やさない。送った後は観測で確認し、食い違えば観測が勝つ（決定3。IMMで読めるアプリ）。
- **beliefが低信頼のとき（`applied`がUnknown等、awaseの書き込みの裏づけが無いとき）は、物理キーをSuppressせず素通しにし、awaseは書かない**（排他。BUG-46の二重actuation回避）。
  beliefがずれている間にキーをSuppressすると、その打鍵が黙って失われる（belief=ON・実IME=OFFで`!belief`=OFFの冪等な`VK_IME_OFF`を送っても何も起きない）ため。
  収束するのは2打鍵目で、ユーザーには「キーが効かないことがある」＝ADR-189が直した症状と同じ見え方になる（開閉軸でも起きる）。追加は1条件で削除は無い。

**既存の既定の書き込みの扱い**: TSF cold-start warmup（`VK_IME_ON`を実送信、BUG-02/69。IME ONのときだけ発動）は、既存の例外として残す（撤去対象外）。ユーザーがopt-inした
機能（親指キーの単独タップの再送等）も残す。それ以外の既定の書き込みは撤去の対象（決定5）。

例外のリスク: beliefが実状態と食い違うとき、ユーザーが望んだ方向と逆の結果になりうる。TsfNativeでは書き込みがUIミラーにしか届かない環境（BUG-25）でbeliefと実状態が同位相に揃い、自己訂正が起きない
恐れがある（固定の例外の適用範囲を広げない理由）。

### 決定2: BUG-151の最小修正（独立に先行、ADR不要の修正）

**実装状況（2026-09-21）**: 決定2の最小修正は、`develop`にも撤去ブランチにも、これまで**実装されていなかった**。撤去ブランチのBUG-151修正コミット`c949ba33`は決定2の縮小案とは別物で、
追随の対象を`VK_IME_ON`/`VK_IME_OFF`以外の全モードキーへ広げる変更（`is_followed_mode_key`）であり、静的な`shadow_action`の撤去が前提のため、`develop`に単独では当たらない
（`develop`ではひらがな0xF2が`shadow_effect`で`TurnOn`の`shadow_action`を持ち、追随の対象から外れる）。そのため決定2どおりの最小修正を`develop`上で新規に実装し、PR #238（draft）にした
（検証はCIの`atok-passthrough-cold`系のA/Bで代替中、決定的な再検証は道具PR #237のマージ後）。撤去ブランチをマージするときは、`c949ba33`と重なるので、この分岐が不要になるかを整理する。

`kp_stage_shadow_ime_toggle`が**no-op（`effective_open() == current`、awaseは書き込む必要が無かった）で終わった打鍵**にだけ、通過マーク（`arm_mode_key_pass_mark`）を立てる
（`key_pipeline.rs`のno-op分岐、10行程度）。20ms後の再読み取りは通過マークでtyping-idleガードを越え、`applied`がUnknownでも実IMEを読む。
- **理由（正確に）**: この打鍵の意図は、no-op判定の**前**に`intent_store`/`last_intent`へ既に記録されている。no-opの時点で**beliefが既に目標と一致しており、意図を捨てても失う情報が無い**
  から、通過マークが意図を捨てても安全である（「意図が無いから」ではない。武装位置をno-op分岐より前へ動かさないこと）。
- **ケース3改（`ExplicitImeActionOutcome::SuppressOnly`、BUG-124対策）は`key_pipeline.rs`でno-op分岐より前に`return false`する**ので、この修正は触らない。「@」の再発経路は構造的に回避されている。
  武装をこのreturnより前へ動かさない。
- **武装条件に「awaseが書いて結果待ちの窓ではないこと」を足す**（round3 RM1で訂正）。判定材料は`applied_state()`が`AppliedImeState::Optimistic`（awaseが書いたが未確認）でないこと。
  `active_actuation`/`attempts`は使わない: 設定する唯一の場所`actuation_for`の呼び出し元は`ime_refresh.rs`の`ir_apply_drift_correction`の1箇所だけで、守りたいシナリオ
  （Ctrl+変換の`dispatch_ime_set_open`の非同期ImmCross write）を表さない。**PR #238は、この節の旧記述どおり`attempts`で判定している**ので、`Optimistic`への差し替えが要る（PR #238側で扱う）。
  BUG-151のcoldケースは`applied=Unknown`なので通る。通過マークの観測は`invalidate_intents_if_mode_key_pass_live`（`intent_store.remove(hwnd)`、
  **窓単位**で意図を全部消す）を呼ぶので、直前の明示IME操作（Ctrl+変換）のImmCross書き込みが飛行中に無関係な意図まで巻き添えにしない。
- **武装は`!delegate_owned`に限る**（round3 RM2）。no-op分岐は`if !delegate_owned { … 意図の書き込み … }`ブロックの外側にあり、`delegate_owned`のときはこの打鍵の意図が書かれていないので、上の「理由」が成立しない。
- **予測との関係（round4 QM3）**: 通過マークの観測は窓単位で意図を全消しする（`invalidate_intents_if_mode_key_pass_live`）。撤去ブランチはfollowを全モードキーへ広げたので通過マークはほぼ毎打鍵立つが、
  打鍵時予測（決定3）は意図でなく`KeyEffectPredicted`（`desired_open`は書かない、`resolve_open_at`の専用枠）なのでこの全消しの対象外。最初の20msの再読み取りはsettle（100ms）内で予測を訂正しない。
- **やらないこと**: `explicit_verify`の`applied != Unknown`分岐の削除（第2項の内側の条件で、消すと「通過マーク無し・`explicit_intent`だけ」の打鍵でタイピング中のクロスプロセス読み取りが走る＝「@」の
  独立した十分条件に触れる）。`is_convert_or_nonconvert`の`is_ime_mode_key`への差し替え（`vk.rs`のdocがコードレビュー指摘で却下済み: 明示意図を守るべきキーの意図まで捨てる）。この関数は縮小案でも使い続ける。
- 回帰テスト（`golden_scenarios`か`journal_replay`）か`docs/known-bugs/BUG-151.md`への修正履歴の追記が必須（`.claude/rules/fix-requires-evidence.md`、IME belief・キー選択の再発ファミリー）。
- 合格ライン（暫定）: `--exp`のB（cold）で再現0/12（2回）、`--walk`のBでEngine/実IMEの不一致（押下+400ms）が5%未満（2ラン×2）、かつ**明示IME操作の直後（200ms以内）にno-opモードキーを押す
  シーケンスでbeliefが落ちないこと**。BUG-151の頻度はdevelopビルドで測り直して確定する（MS-IMEでも同型の失敗がelw6のBで12/12あった。GJI固有ではない根拠）。

### 決定3: 表は予測として使う。出所は静的な初期仮説と注入学習

「押下直後の1打から正しく動く」（要件: かな=Engine ON、英数=Engine OFF）ために、(状態, キー)→効果の表でEngineを先に動かし、観測で確認・訂正する。
- **表の形**: `(状態, 修飾, キー)`ごと・軸ごとの効果（Set(v)/Toggle/None）と次状態。状態はMozcのstatus（DirectInput/Precomposition/Composition/Conversion/Suggestion/Prediction）を含める（決定4の観測で区別する）。
- **出所**: (a)GJIは`config1.db`（`session_keymap`+`custom_keymap_table`+`overlay_keymaps`）とMozcの公開キーマップから作る静的な初期仮説。**VK→Mozcキー名の写像（`key_parser.cc`/`keyevent_handler.cc`相当。例: 0xF2は`Kana`で、`atok.tsv`の`Kana`と`Hiragana`はバインドが別）も静的な初期仮説の一部として持ち、写像を1つ間違えると「未割当」と「ToggleAlphanumericMode」を取り違える**（決定5の複雑性収支に計上する）。(b)MS-IMEは非公開なので、自前の測定から作った既定表を**データとして同梱**する。
  (c)いずれも注入による学習・検証は必須（カスタムキーマップのユーザーがいる。`session_keymap`と`custom_keymap_table`の食い違いはBUG-143）。
- **表はawaseを完全バイパスして学習したときだけIMEの表になる**（awaseを通すと、awaseの書き込み・代行の規則が混ざった合成規則を学習してしまう。実測2(b)）。
- 表と観測が食い違ったら観測が勝つ（IMMで読めるアプリの範囲、決定1）。非決定セルは表で確定せず観測に委ねる。
- **表が空にならない**: 未較正でも同梱の既定表を使う。従来の静的な意味づけ（コードに直書き）は、同じ内容のデータへ移してからコードを消す（決定6）。

**予測の反映は観測を待たず、打鍵の時点で行う（2026-09-21、ユーザー指摘）**: 観測に追随する方式は、IMEが状態を変えてから次のポーリングで読むまで遅れる（撤去ブランチの実測: 押下から約0.5秒、400ms時点のずれ35〜41%、1.5秒以降は0〜4%）うえ、
観測できないアプリ（TsfNative）ではそもそも動かない。よって、**物理キーの押下時点で、表（決定3）からbeliefを更新する**（Engineの追随はこれで即時になる）。観測は「確認と訂正」に回す。
- **fence（反映のずれ対策）**: 打鍵ごとに単調増加の連番（epoch）を振り、予測を書いた時刻と連番をbeliefに持たせる。観測（読み取り）には**読み取りを開始した時刻**を付け、
  **最新の打鍵より前に始まった読み取りは、結果が届くのが後でも捨てる**（IMEが効果を反映する前の古い状態で予測を上書きしないため）。IMEの反映待ち（settle）より後に始まった読み取りだけが、予測と照合できる。
- **照合**: settle後の観測が予測と食い違ったら観測が勝つ（IMMで読めるアプリ）。食い違いは「表の外れ」として記録し、較正の材料にする。観測できないアプリでは予測が唯一の信号になり、
  状態依存のキー（入力中かどうかで結果が変わる）ではずれが積み上がる。これは受け入れ済みの範囲で、強制ON/OFFの打鍵（ADR-192）で立て直す。冪等なキーは予測が外れても次の打鍵で収束する。
- **注意（実測済み）**: 予測を観測なしで連鎖させる（開ループ）のは信頼できない。ずれは1打では2%未満（表の1手予測は約98.5%、トグルキー52/52）でも、連鎖で積み上がる。だから予測は観測が読める限り毎回照合し、連鎖させるのは観測できないアプリだけにする。
- 既存のepoch/fence（`ImeModel`の意図・生成番号、`FocusHwndUpdated`の`current_fence`）の流用可否は実装前に確認する。新しい機構を増やさず、既存の照合の入口に「読み取り開始時刻が最新打鍵より後か」の条件を1つ足す形を第一候補とする。
- **実装での単純化（round4 QB1・QB2）**: 予測は`ImeEvent::KeyEffectPredicted`で書き、開閉は`resolve_open_at`に「明示意図の次、観測の前」の枠を足した（`UserImeSetIntent`だと明示意図でポーリングが止まる問題〈BUG-151原因③〉を全モードキーへ広げ、
  `desired_open`への新イベントだと`resolve_open_at`で観測に負ける、の両方を避けた。`17f91966`、`cb6c8ebd`）。fenceは新しいepochを作らず、reducerが観測を受けた時刻が最新打鍵から`KEY_EFFECT_SETTLE_MS`=100ms以内なら
  予測を上書きも消しもしない（ADR-187の20ms再読み取りが最大62ms古い値を返す実測＋マージン）。`probe_actuation_fence`（ADR-140）はawase自身のactuationを数えるもので、生キー通過後のIME反応遅延の判定に合わず流用しなかった。
  **限界**: 読み取りの開始時刻でなく到着時刻で判定するので、settle直前に始まり直後に終わる読み取りが照合されうる。`GetTickCount64`は約15.6ms粒度。

**実装の現状（2026-09-21、撤去ブランチ）**:
- 表は手書きせず、格子第3版（全状態をキーだけで作る）の結果から`tools/e2e/ime_key_matrix/gen_key_effect_table.py`が`state/key_effect_data.rs`を生成する（生成元は`grid-tables/{atok,msime}.json`）。
  格子第1版はIMM書き込みで変換モードの状態を作ったため、キーで入った状態と別物になり、ATOKのひらがなが0x19↔0x10の純粋トグルなのに「不変」と誤学習した。第2版（キー到達、リセットのみIMM）を経て、
  第3版でリセットもキーだけにした（経緯は補助資料）。「MS-IME」の表はGJIのMS-IMEプリセットの表で、Microsoft IME本体の表ではない。
- 変換モード軸は、キーで到達できる値だけを持つ（ATOK: 0x19・0x10、GJIのMS-IMEプリセット: 0x19・0x1B〈自然状態0x09は0x19と同一視〉）。到達できない0x13・0x18等はセルに入れない（予測なし）。
  閉状態の変換モードは、読み取りが不安定なので追わず、開閉だけを予測する。
- 隠れ状態「入力中の段階」（なし/入力中/変換中〈Space・変換キー・無変換で入る〉）は、現実装では固定の段階を打鍵履歴から追跡する（`ImeModel::key_track`、暫定）。**目標は、固定の名前・規則でなく、
  学習した最小のMealy機械として持つこと**（カスタムキーマップに対応するのがこの機能の目的なので必須。Spaceや変換キーの意味がユーザーのキーマップで変わるため、規則をコードに書けない）。
  同じ応答をする状態は統合し、識別プローブ（Esc/Enter/BS/Space）への応答の違いで同定する。
- 予測の書き込みは`ImeEvent::KeyEffectPredicted`（`desired_open`は書かない）。開閉は`resolve_open_at`に「明示意図の次、観測の前」の枠を足した。fenceは`KEY_EFFECT_SETTLE_MS=100`（実測最大62ms＋マージン）。
  開閉を予測したら同じ対象の古い明示意図を捨てる（読めないアプリで予測が古い意図に負けないように）。

### 決定4: 較正セッション（注入による自動学習）

ADR-176（ユーザーが物理キーを押す方式）を、明示的なセッションでの自動注入へ拡張する。**これはADR-176の免責条件（「awase自身はキーを送信しない」）の拡張で、本ADRが明示的に判断する**
（ADR-176が却下したのは通常実行時のバックグラウンド注入。BUG-113/124の「@」は、TsfNativeのアプリへ生キーが届くこと、およびawase自身のIME操作が原因）。根拠と条件:
- 注入先は**較正のために新設する自前のWin32テキストコントロール**に限る（他アプリへは送らない。awase-settingsの現行UIはeframe/eguiで、自前のWin32コントロールは存在しない。新設は追加コストとして計上し、eguiのまま
  測るならBUG-107/125のegui窓のプロセス間汚染の上で測ることになる）。TsfNativeアプリへ生キーが届く経路が無いので、BUG-113の機序（TIPのキー横取り）は成立しない。実測（自身の入力欄への大量注入で「@」が出ない）は
  機序の不成立の証明にならないので根拠にしない。出荷前に較正窓での実機A/B（GJI・MS-IME）を条件とする。**キーマップはIMEのプロパティなので窓に依らないが、composition状態は窓ごと**なので、
  較正窓の表を全アプリへ適用してよい根拠は「キーマップの効果は窓に依らない」ことに置き、composition状態は観測で扱う。
- **配置（暫定、round3 RM4/RM5）**: 較正窓とcompartment sinkは、awase.exe側の**専用スレッド（自前のメッセージループ）**に置く案を第一候補とする。compartmentの`AdviseSink`と`ITfThreadMgr::Activate()`はスレッド単位で効くので、
  awase-settingsのeguiスレッドに置くと、(1)awase-settings.exeにTSF/COM面が新設される、(2)`Activate()`がegui側のIME入力に影響しうる（未検証）。awase.exeには`tsf/`とwindows-rs 0.62の前例があり、ADR-176 v7の分担
  （awase-settingsはUI＋IPC、観測はawase.exe）を延長できる。awase-settings側に置く場合はADR-176 v7を覆すことを明示し、既存のegui入力に影響しないことをスパイクで確認する。
- **同型の機能が出荷されて失敗した前例（round4 QM7）**: 撤去前の`gji_charset_autodetect.rs`のモジュールdocに、専用Fnキー変換の自動判定・設定支援ポップアップ・`config1.db`書き込みが「実験的機能のまま撤去し忘れて出荷され、
  実機でユーザーの混乱を招いた（GJIのキー設定が実際にはカスタムなのに『カスタム以外』と誤診断される等）」ため2026-09-02に全撤去した、という記録があった。キーマップの自動診断をユーザーに提示する点で、本決定とADR-192の警告は同じ形。
  **今回の違い**は、設定の読み取りだけで断定せず、注入で実IMEに当てた実測と突き合わせ、食い違えば実測を採り「書かず追随」に倒すこと（決定1）。ただし誤診断が起きないことの証明ではないので、警告は判定根拠を載せ、
  ブロックせず、判定できないキーは警告しない（ADR-192）。
- awaseは既存のIPC（`WM_CALIBRATION_START`、送信元PIDの検証あり）で較正窓を**完全バイパス**する（窓/プロセス単位、実測でA'は自己操作0）。マーカー（`dwExtraInfo`のnonce）は使わない
  （全LLフックから読めるので偽装対策にならない。バイパスは窓単位で、キー単位の識別が不要）。awaseを通す条件（B）は製品には持ち込まない（測定用のスパイクだけ）。
- 対象は「状態×キー」の**全掃引**（ランダム押下列はセルを網羅できない: 未学習約13%）。**未確定文字列を作ってから押すComposition・変換中のConversionも必ず含める**（含めない掃引では、変換キーが「閉→開、開→閉」の完全なトグルに見えて誤判定する。決定1の状態完備の条件）。
- **観測**: (a)開閉・変換モードの読み取り、(b)**実打鍵の結果**（未確定文字列・確定テキスト。BUG-25の教訓）、(c)**TSFのスレッドcompartmentの変更通知**（`ITfCompartmentEventSink`、`GUID_COMPARTMENT_KEYBOARD_OPENCLOSE`と`GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION`）: Mozc自身がこの2つをadviseしており、
  開閉とかな⇔英数の変化をpushで副作用なしに取れる（ADR-186の手法T、素のWin32 EDIT窓で`CoCreateInstance(CLSID_TF_ThreadMgr)+Activate()`してスレッドcompartmentを読む方法が実測で全件一致）。
  **`ITfUIElementSink`（候補/サジェスト/モードインジケータ）は採らない**: `TF_TMAE_UIELEMENTENABLEDONLY`は通知の有効化ではなく未登録TIPを弾くフィルタで、`pbShow`はFALSEにするとGJIの
  描画と互換通知まで止めて較正が別物を測る（ADR-138が警告した汚染と同型）、Mozc自身がConversionとPredictionを同じ`kCandidateWindow`に畳むので3状態は区別できず、取れるのは「サジェストか候補か」の1ビットだけ、
  モードインジケータは押下のたびに消える。1ビットが本当に要ると分かってから、別途スパイクで検討する。
  (d)**有効なTIP（GJI/MS-IME）を測定の前後で必ず記録する**。
- 実現性の注意: 管理者権限の窓（UIPI）は対象外（較正窓は自前なので該当しない）、較正窓のUI更新でフォーカスを奪わない、短時間の大量注入がセキュリティソフトに検知されないよう押下数を見積もる。
- 結果は`config.toml`へ永続化し、stale検出（ADR-176 T11/T12）とopt-inゲートを再利用する。

**（追記）較正の観測項目に「入力中の文字列の行方（保持/確定/破棄）」を加える**（決定1の分類(d)の判定のため）。statusごとに押す前の状態を作る（未確定文字列を入れる等）。詳細は決定1の分類・出所の項を参照。

**キャリブレーションと予測の改善は3段階のラウンドで回す（2026-09-21、ユーザー指示）**:
1. **設定の読み取りラウンド**: GJIの設定（`config1.db`+Mozc公開キーマップ）から静的な表Sを作る（分類a〜eを含む、決定1）。
2. **学習ラウンド**: awaseを完全にバイパスした注入で実IMEを観測し、表Lを作る。SとLの食い違いセルが、設定の読み取り側の直すべき箇所になる。
3. **検証ラウンド**: 学習に使っていないラン（別シード）で、表（S/L/合成M=Lの決定セルを優先、無ければS）の開ループ連鎖を採点する。オフライン（`tools/e2e/ime_key_matrix/cycle.py`）で回し、最後にawase実機（Engineのずれ、`--drift`のDRIFT_OFF=100/400/1500）で確認する。
- 各ラウンドの指標: 設定=Sの網羅、学習=網羅・決定性・一段予測、検証=開ループの一致率と最初に外れる原因セル。外れた原因セルが、次の改善（状態の表現・観測項目・表）の入力になる。
- **最初の知見（4ランの実測）**: 学習表の非決定セル3件（入力中のEsc・無変換）は、**隠れ状態「変換中（Mozcの`Conversion`）」**を、打鍵履歴（変換キーを入力中に押した後）から追跡すると、決定性が98.5%→99.8%に上がり非決定セルが1件に減る。
  状態に「変換中」を加える（学習側の観測の状態表現、予測側のbelief）のが最初の改善候補。ただし標本は4ランで少ない。

**待ちの短縮（通知の購読、2026-09-21）**: 学習の固定待ちを、IMEの状態変化の通知を待つ形に置き換えた（スパイクの`--notify`、CI道具PR #237）。開閉と変換モードの変化は、TSFスレッドcompartmentの
変更通知（`ITfCompartmentEventSink`）を購読し、最後の通知から40ms静かなら確定する。通知が来なければ150msで「変化なし」とみなす。入力中・変換中・確定の待ちは固定待ち（`--fast`の700ms相当）のまま。
効果: ATOKの格子1シャードのスパイク本体が、遅い版の1,287〜1,693秒から224〜306秒へ（約5〜6倍、表の差分0）。通知の遅延は、ATOKの1回・16キーの測定で押下から最初の通知まで
P50=1ms・P95=34ms（10件、通知が来なかったキーは6/16=38%）。GJIとEdit窓では`WM_IME_STARTCOMPOSITION`/`COMPOSITION`/`ENDCOMPOSITION`も0〜5msで届き、入力中・確定のイベント化も可能と見える
（実測が限られるので未検証の範囲あり）。詳細は補助資料。学習の設計（巡回による測定数の削減、異常時のリセット、統計的な訪問回数）は、`crates/awase-calibration`（Rust、`feat/awase-calibration`）の
シミュレータで検討中。

### 決定5: 成功基準は撤去量。追加は削除と対にする

指標（`RESTRICTED_CALLS`の行数は読み取り経路も含みゲーム可能なので補助に留める）:
1. `crates/awase-windows/src`の追加行−削除行が、**撤去フェーズ（P0〜P2）の末で負**であること。**較正基盤は別ADR（撤去が頭打ちになってから起票）に切り出し**、そちらは「既存モジュールへの変更行数が最小、
   較正基盤は単一のディレクトリに閉じる」で縛る（較正基盤は定義上、追加が撤去を上回りうるため、この指標を課さない）。
2. `send_input_safe`の呼び出し**箇所**数（現在20）。
3. `set_ime_open_ordered`の呼び出し箇所数（現在2: `ime_refresh.rs`のフォーカス変更時の強制OFFと、drift補正内）。`RESTRICTED_CALLS`の外にあるので別に数える。
4. `architecture_guard.rs`の件数ガードの総数。
5. IMEへ書く**振る舞い**の数（固定の例外・表駆動の追加・opt-inの単独タップ・`keys.ime_on/off/toggle`・EngineDecision・warmup）。入口が1つでも振る舞いが増えていないかを、許可リストの件数とは別に列挙して数える。

フェーズ（撤去を先、較正は撤去が頭打ちになってから）:
- **P0**: 決定2（BUG-151の最小修正）。
- **P1（表なしで撤去できるもの）**: 調査を先に。候補: フォーカス変更時の強制OFF（`ime_refresh.rs`、決定1に反する）。`is_convert_or_nonconvert`は決定2でも使い続けるので外す。
- **P2（表が要るもの）**: 決定6の撤去。**撤去はTsfNativeを含む全アプリに効く**（本ADRの原則の適用範囲がIMMで読めるアプリでも、撤去するコードはアプリ種別で分岐しない）。撤去前に「TsfNativeで従来と同じ挙動が
  既定表で再現できること」をADR-189のCI（`msime-hz`/`atok-hz`）で確認する。**`shadow_action`は4つの役割**（①shadow beliefの方向、②`transport::plan`のモードキー分類、③`ModeKeyActuationOwner`、④BUG-14のinjectedガード）を担う
  ので、②③④の代替を先に用意する。①だけが表で置き換わる。トグルキー（決定1の固定の例外）には`shadow_action`が残る（`shadow_effect`はトグル以外のVKだけ`None`を返す形になり、関数は残る。
  撤去量として数えられるのは実際に消えた行だけ）。ADR-189の固定セットは変えない。
  `ir_apply_drift_correction`（TsfNative救済の最後の1本、BUG-20）は、`--walk`では明示意図のシナリオを測れないので、撤去前に明示意図の回復シナリオのテストを用意する。
  `dispatch_ime_set_open`のEngineDecision系は、単独タップのopt-in経路と分離するには`SetOpen`に発生元の軸が要り、削除でなく追加になる。分離の是非を調査してから。
- 撤去に数えないもの: `classify_mode_key_ime_action`（表生成側へ移設されるだけ）、`ModeKeyConfig`/`muhenkan_solo_tap_dedicated_fn_key`（ユーザー設定で表とは別軸。決定6）。
- **決定の依存順（round4 QM2、循環の解消）**: 予測表（決定3）→ 書き込みの線引き（決定1、分類a〜eは表から引く）→ 撤去（決定5・6）。表が無い・非決定のセルは「書かずに追随」で動くので、**撤去は表の完成を待たずに先行してよい**
  （例外の一般化は後から足す）。実際の順序: 撤去ブランチは、生成した表（格子第3版）と打鍵時予測を含めて実装済み。
- **複雑性の収支（実数、2026-09-21、`git diff --shortstat origin/develop...origin/feat/adr191-remove-hardcoded-mode-keys`）**: 全体43ファイル、+3,977/−4,519行。`crates`と`src`だけで+2,368/−4,461行（差し引き−2,093行、
  うち`crates/awase-windows/src`は+2,244/−2,872）。
  | 区分 | 行数 | 戻ってくるか |
  |---|---|---|
  | 削除: `gji_charset_autodetect.rs` | −1,301（+35） | **決定3(a)が再実装する対象**（`config1.db`/Mozcキーマップの読み取り）。現状の表は格子の生成データで、実行時の設定読み取りは未実装（製品化で新規に作る。再実装は分類a〜eに要る最小の範囲に限る） |
  | 削除: `calibrated_mode_key.rs` ほか較正結果の適用 | −254（+7）、適用の削除−165 | **決定4が再実装する**（ユーザー判断: 削除して製品化で新規に作る） |
  | 削除: 単独タップ代行・delegate・opt-in設定・`transport.rs`のDBE分岐 | `nicola_fsm.rs`−556、`transport.rs`−561（+173）、`runtime/mod.rs`−345、`src/engine/tests.rs`−615（指標1の分母の外） | 戻らない（純粋な撤去） |
  | 追加: 予測器`key_effect_table.rs` | +828 | 決定3の本体 |
  | 追加: 生成データ`key_effect_data.rs` | +336 | 格子の生成物（手書きセルは無い） |
  | 追加: `ime_model.rs`（`KeyEffectPredicted`・追跡・fence）＋`platform_state.rs` | +440＋84 | 決定3の本体 |
  | 別クレート: `awase-calibration`（巡回・シミュレータ。改名作業中） | +3,406（`crates/awase-windows/src`の外） | 製品化の土台。指標1の対象外 |
  撤去した約4,500行のうち、再実装が要るのは`gji_charset_autodetect.rs`と較正結果の適用の合計約1,700行で、戻ってくる量は**未確定**（設定読み取りの範囲次第）。指標1（`crates/awase-windows/src`の追加−削除がP0〜P2の末で負）は現時点で−628行で満たす。
  [ADR-162](162-governance-reversal.md) E1（複雑性予算1-in-1-out、未発効）と同じ向き。
- **削る・見送るもの（round4 D）**: (1)較正セッションは「学習した表の生成と読み込み」に絞り、ADR-176のウィザードの「適用」配線は削除済み。(2)設定の読み取りは、分類a〜eに要る最小（`config1.db`の`session_keymap`・
  `custom_keymap_table`・`overlay_keymaps`とMozc公開キーマップ）に限り、charset自動検出や設定への上書きは作らない。(3)`ITfUIElementSink`は採らない（決定4）。(4)ADR-192決定3bは、前提を訂正して最小に絞った
  （削除案はユーザーの要望で見送り）。(5)予測の対象軸を開閉に絞る案は、入力モード軸も含めて実測で効果（400ms以降0%）を確認済みのため縮小しない。ただし一般の表は作らず、到達できる変換モードの値だけを持つ。

### 決定6: モードキーは決め打ちせず、学習結果で統一的に扱う

較正を導入した以降、変換・無変換・かな（ひらがな）・カタカナ・英数・半角/全角・漢字などのIMEモードキーは、キーごとの意味をコードに持たず、決定3の表を唯一の情報源として同じ経路で扱う。
コードが静的に持つのは「このVKはモードキーか」の分類だけで、押したときに何が起きるか（Set/Toggle/None）は表から引く。**例外は、決定1の固定セット（ADR-189の0x19/0xF3/0xF4と`keys.ime_toggle`）だけを
静的に残すこと（明示的な1箇所の例外）**。それ以外のトグルキーは、状態完備の条件を満たしたときだけ表から追加される。
- 親指シフトのキー（無変換/変換など）: チョード判定はNICOLA側の別軸のまま残す。単独タップのときIMEに何が起きるかだけ表から引く。`ModeKeyConfig`等の**ユーザー設定は「ユーザーが何をしたいか」の軸で、
  表（IMEがどうなるか）に畳み込まない**。
- 撤去対象（決め打ちの箇所）: `vk.rs::ImeKeyKind::shadow_effect`（VK別の静的効果、①の役割）、`runtime/mod.rs`の`shadow_action`上書きのうち**ADR-189の固定セット（0x19/0xF3/0xF4）は撤去しない**（TsfNativeで効いている例外そのもの）。上書き点は1箇所だが、供給元が約8箇所
  `set_thumb_key_shadow_overrides`等にあり、複雑性は供給元側（固定セット以外を対象にする）、`dbe_mode_key_policy`（半角/全角の一律Suppress/Allow）。いずれも既定表（データ）で同じ挙動が再現できることを`--walk`/`--exp`で確認してから。
- 表が空の環境は同梱の既定表を使う（従来の直書きの意味づけを、同じ内容のデータへ移す）。撤去量は「コードの分岐・行の削除」で数える。
- **（2026-09-21）ADR-189の固定セットは撤去ブランチで復元済み**（`651cab8d`）。漢字0x19はどのIMEでも開閉トグル、半角/全角0xF3/0xF4は`ImeKeyKind::is_open_toggle_for(ImeKindId)`でGJIとMS-IME本体の両方を
  開閉トグルとして扱う（ADR-190のCI実測: awaseなしではF0/F3/F4がトグルする。「F3=OFF・F4=ON」はawase側の静的モデルで、実IMEの挙動ではない）。`dbe_mode_key_policy = Suppress`の対象は、awaseが実際に書く
  0xF3/0xF4だけに縮小した（`73877f52`。英数0xF0・カタカナ0xF1は素通し。BUG-153）。

## リスク・未解決

- **TsfNative**は範囲外（決定1）。同じ表が使えるか、観測をどう得るかは別に扱う（ADR-188、必要なら実アプリでの実打鍵確認）。
- **例外で打鍵が失われる**: beliefがずれている間に物理キーをSuppressすると、その打鍵が黙って失われる（決定1の書き込みの規則で、低信頼のときは素通しにして緩和する）。
- **BUG-113/124（TsfNative×GJIの「@」）**: 例外の書き込み（`VK_IME_ON`/`OFF`）がIMMで読めるアプリで新たな不具合を出さないか、撤去・例外の各段で実機確認する。
- **隠れ変数**: 表で確定できない分岐（入力中の無変換・Escなど）の要因は、Mozcのstatus（Conversion/Suggestion/Prediction）が候補。取れるのはサジェストか候補かの1ビットだけ（Prediction・Conversionは同じ候補窓）で、
  UI要素通知の観測は、compartmentの観測で足りないと分かってから別途スパイクで検討する（決定4）。
- **用語**: 本ADRの「MS-IME」は実際のMicrosoft IME（TIP clsid 03B5835F）を指し、Mozcの`ms-ime.tsv`は**GJIのMS-IME模倣キーマップ**で別物（例: ひらがなは前者が「ON・かなで閉じる」、後者は`CompositionModeHiragana`で閉じない）。表の出所を書くときに混同しない。
- **測定の信頼性**: 2026-09-20の一部のランは有効なIMEがMS-IMEで、GJIの結論に使えなかった（今後は測定ごとにTIPを記録）。仕様モデルは未見データでの評価が要る（実測2）。
  ひらがなのMS-IME挙動は追試が要る。
- **複雑性収支**: 較正基盤の追加が撤去を上回りうる（決定5）。撤去が頭打ちになる前に較正へ進まない。
- **`reinject`のscan=0**: PassThroughの再注入はscan=0で再送する（BUG-147のwScan仮説と関係するか未検証）。別途調査。

## 却下した代替案

- **通常実行時のバックグラウンド注入・受動学習**: ADR-176が却下済み。維持する。
- **静的表だけ（較正なし）**: カスタムキーマップと`session_keymap`/`custom_keymap_table`の食い違い（BUG-143）で破綻する。
- **学習表だけで開ループ追随**: 1手の誤りが後続へ連鎖するので、開ループ単独には頼れない（観測が要る。実測2(f)のとおり開ループ%は指標として壊れているので数値は根拠にしない）。
- **トグルも生キーを通して観測追随だけにする（例外なし）**: 素通し追随との比較は未測定（条件Cで測る）。固定の例外（1）はADR-189のCI実測（静的な固定方向との比較）を根拠に採り、追加分（2）は条件Cの結果が出るまで無効。
  ユーザー方針で例外を採ること自体は正当だが、根拠にできる実測とそうでない実測を混ぜない（round3 RM6）。
- **一括の大規模撤去・較正基盤の先行**: 複雑化と純増の危険。撤去を先に、1つずつ。

## 検証計画

各撤去・統合の前後で、スパイクの`--walk`（A'/A/B、有効TIPを記録。`--exp`は現行のCI道具には無い）を流し、素通し追随・予測の評価は**条件C**（決定1）で行う（条件A/A'ではfollowが発火しない）。CIでは`cal-verify-{obs,blind}`を使い、(a)BUG-151再現0/12、(b)BのEngine/実IMEずれ率、(c)決定5の指標、を記録する。
TsfNative（Chrome）は実打鍵の結果で別途確認する。

## 関連

ADR-138（ウィットネスアプリ却下、自前ITextStoreACPの警告）、ADR-162（複雑性予算）、ADR-176（較正UI、注入の却下範囲、免責条件）、ADR-186（ATOKの実測表と追随）、ADR-187（follow方式）、
ADR-188（TsfNativeのconvだけを変えるモードキー）、ADR-189（半角/全角のactuate、例外の土台）、BUG-25（IMC読み取り・書き込みは実モードの証明にならない）、
BUG-113/124（TsfNative×GJIの「@」）、BUG-143（`session_keymap`と`custom_keymap_table`の食い違い）、BUG-151（決定2の直接の動機）。


## 実装の現状（2026-09-21、`feat/adr191-remove-hardcoded-mode-keys`、develop未マージ）

ユーザー指示: **GJI/MS-IMEの設定どおりに動かすことを優先する。これまでの挙動はMS-IMEプリセット前提だったため、変換・無変換・かな・英数・半角/全角の決め打ちを撤去し、awaseなしで学習した結果に従う。**
実測の外れ31件のグループ1（ひらがなをOFFで押すとON）・2（入力中の無変換/変換でIME OFF）・3（半角/全角の方向固定）の機構を撤去した。この節は**撤去ブランチ**の現状で、`develop`の実体とは異なる
（`develop`には未反映）。規模（`develop`比、`crates`と`src`のみ）: 34ファイル、追加2,283行・削除4,165行（差し引き約1,900行減。`tools/`と`docs/`は別に+1,591行）。
1. `vk.rs::ImeKeyKind::shadow_effect`は`ImeOn`/`ImeOff`/`KanjiToggle`の3つだけ`Some`（`TurnOn`/`TurnOff`/`Toggle`）。半角/全角（0xF3/0xF4）は`is_open_toggle_for(ImeKindId)`で判定する。
   `ShadowImeEffect::Toggle`は、漢字0x19のためにコード上に現存する（当初の「削除」は誤りで、ADR-189の固定セットを復元した際に戻った）。
2. `enrich_ime_relevance`の`shadow_action`上書き連鎖のうち、ひらがな/カタカナ・無変換/変換の2系統を削除。ADR-189の半角/全角・漢字のbeliefトグルは、無修飾の物理キーだけ復元した（書き込み点は1箇所、TsfNativeでも効く）。
3. 追随（ADR-187 follow）を`is_followed_mode_key`（IMEモードキーから`VK_IME_ON`/`VK_IME_OFF`を除く全て）へ一般化（`c949ba33`、BUG-151の別の修正。決定2の最小修正とは別物。決定2の項を参照）。
4. エンジン本体から、無変換/変換/ひらがな/カタカナの単独タップ代行（`*_delegate_to_open_axis`）、ひらがな/カタカナ親指キー設定、Shift+代行キーの素通し判定を削除。単独タップは専用Fnキー→ユーザー明示config→`ModeKeyConfig`
   （ユーザー設定のSuppress/Passthrough）の順で解決する。ADR-182決定1bの抑止は残した。
5. Windows側のGJI/MS-IMEの設定からの自動検出・配線と、較正結果を分類へ反映する関数を削除（較正の永続化・IPCは残る）。旧関数名`sync_gji_charset_autodetect`は、定義が消えた後もdocコメントに
   3ファイルの参照が残っている（掃除が必要。`gji_charset_autodetect.rs`は名前と違い、現在はthumbキー/変換・無変換の分類だけを持つ）。
6. 撤去した機能の単体テスト約75本を削除。到達不能になった`delegate_owned`の分岐、`ModeKeyActuationOwner::FsmDelegate`、`auto_delegate_open_axis_consumed`（`076f29c3`）と、opt-in設定
   `gji_thumb_key_ime_toggle`（ゲート・警告・設定画面を含む、`78e22861`）も削除した。
7. 打鍵時予測（決定3）: `state/key_effect_table.rs`（予測器）と生成データ`state/key_effect_data.rs`、`ImeEvent::KeyEffectPredicted`、`platform_state.rs`の反映、`kp_stage_mode_key_follow`から呼ぶ。
   CI検証（run 35585712177）で、観測あり・読めない条件とも400ms以降のずれ0%、`[key-effect-miss]`0件。
8. `dbe_mode_key_policy = Suppress`の対象を0xF3/0xF4だけに縮小（`73877f52`）。英数0xF0・カタカナ0xF1は素通しで、実イベントでは元から`shadow_action`を持たず素通しだった
  （握りつぶしていたのは合成イベントの死んだ分岐だけ）。`shift_katakana_passthrough`と`DbeModeKeyContext`は削除。`transport.rs::plan`のDBE分岐・`dbe_mode_key_policy`（約25テスト）は機能しているので残した。
**残り**: 実機（Windows）でのA/B（英数・カタカナ・半角/全角の動作、MS-IME本体の半角/全角のbeliefトグル）、`transport.rs::plan_tests`の新テストのwindows-build CIでの初回実行（`#[cfg(windows)]`配下でLinuxでは
走らない）、bug report（ADR-148）のスキーマに残る採用系フィールドの整理、旧名のdocコメントの掃除。

## TsfNative（観測できないアプリ）の扱い（2026-09-21、ユーザー整理）

観測できないアプリ（`Imm32Unavailable`・`TsfNative`・`InputRelay`）では、IMEの状態を一切読まず、TsfNativeでは`reschedule_ime_refresh`がポーリングを予約せずに戻る（コードで確認）。「観測に追随」は
そこでは定義できず、決め打ちの撤去により、状態依存のキー（入力中かどうかで結果が変わるキー）を使うユーザーには、モードずれが起きるようになる。これを**受け入れる**（ユーザー判断）:
- 冪等なキー（`VK_IME_ON`/`VK_IME_OFF`）はずれない。ずれるのは状態依存のキーを使うユーザーだけ。
- ずれは、(a)**IMトグルのawaseによる書き込み（ADR-189。残す機能）**、(b)**awaseが強制的にactuateする強制ON/OFFの打鍵**（`keys.ime_on`/`keys.ime_off`。既定値がCtrl+変換/Ctrl+無変換というだけで、configで別のキーに上書きしていればそのキーになる）で、**開閉軸は**強制的に解消できる。**かな/英数軸の回復経路は無い**（ATOKには入力モードをSet指定するキーが0件。`VK_IME_ON`は`composition_mode`を指定しないと入力モードを戻さない〈Mozc `session.cc`〉、実機確認は未了）。TsfNativeで`ToggleAlphanumericMode`系のキー（ATOKの`Kana`）を使うユーザーは、フォーカスを移すか、IMEを一度OFF/ONするしかない。これも自己責任・ベストエフォートの範囲とする（round4 QM1）。
- 状態依存のキーを使うユーザーは**自己責任・ベストエフォート**とし、その手助け（検出・警告・冪等なキーへの置き換えの案内）は**別ADR（[ADR-192](192-state-dependent-mode-key-warning-and-guided-override.md)）**で扱う。
- **ADR-189のトグル（0x19/0xF3/0xF4）の書き込みは撤去しない**（撤去ブランチで誤って撤去したが、`651cab8d`で復元した）。TsfNativeでのEngineの追随は、これに依る。

### 窓を切り替えた直後の初期belief（2026-09-21、ユーザー整理）
観測できないアプリでは、窓ごとのIME状態を読めないので、フォーカス切替直後のbeliefは最初ずれていてよい。撤去方針では**最初のずれは受け入れ、ユーザーの強制ON/OFFの打鍵で同期する**。
- 既存の手当てを使う: 前回そのhwndで持っていたbeliefの復元（`HwndCacheRestored`、`focus`のhwndキャッシュ）。WindowsはIME状態をhwnd（入力コンテキスト）ごとに持つので、復元でかなり追随できる。
- 新しい窓は分からない: 既定の仮定（IME ONかつローマ字入力。TsfNativeでは`AssumedRomaji`）から始める。予測の表は現在の入力モードを前提にするので、**入力モードが不明のままでは予測が始まらない**（CIの読めない条件で、beliefがmode=Noneのまま予測が動かずEngineが活性化しなかった）。既定の仮定を必ず種にする。
- 学習で精度を上げられるか: (プロセス, クラス)ごとの「新しい窓の初期状態」の事前分布は、読めるアプリなら観測で学習できる。読めないアプリには正解が無いので、**強制ON/OFFの打鍵の直後に「beliefが違っていた」という証拠**（同期イベント）から間接的にしか学べない。まず頻度（切替直後のずれの割合）を測ってから、学習が要るかを判断する。今は作らない。

## Opus round3・4 の指摘への対応

判定は「反映」（本文を直した）、「反映済み」（既に本文または撤去ブランチに入っていた）、「見送り」（理由つき）。教訓（複雑化を招く型・仕組みの追加は避け、削れるものを採る）に従い、対応は記述の修正にとどめた。

| 指摘 | 判定 | 対応（本文の場所） |
|---|---|---|
| round3 RB1 Engine/実IMEの一致の併記と中止条件 | 反映 | 実測2b、決定1「中止条件」 |
| round3 RB2 素通し追随の測定構成（条件C） | 反映 | 決定1「条件C」、検証計画。撤去ブランチのCI `cal-verify-*`が実質条件C。developビルドの条件Cは未測定 |
| round3 RM1 武装条件を`Optimistic`へ | 反映（本ADR）／要対応（PR #238） | 決定2。PR #238は旧記述の`attempts`のまま |
| round3 RM2 武装は`!delegate_owned`に限る | 反映（本ADR）／要確認（PR #238） | 決定2 |
| round3 RM3 固定の例外と表の矛盾 | 反映（「固定が常に勝つ」に倒し分岐を増やさない） | 決定1 |
| round3 RM4・RM5 較正窓・sinkの配置と`Activate()` | 反映（awase.exe側の専用スレッドを第一候補、暫定） | 決定4 |
| round3 RM6・RM7 却下案の根拠の自己矛盾 | 反映 | 却下した代替案 |
| round3 RM8 測定台は標準Edit 1種類 | 反映 | 実測 |
| round3 NB10残り 書き込みを単一の関数に集約 | 反映済み | 撤去ブランチで書き込み点は1箇所（実装の現状2） |
| round4 QB1 予測の書き込み口 | 反映済み | 決定3の実装での単純化（`KeyEffectPredicted`＋`resolve_open_at`の枠、`17f91966`・`cb6c8ebd`） |
| round4 QB2 fenceの判定式 | 反映 | 決定3（settle 100ms、`probe_actuation_fence`は流用せず、限界を明記） |
| round4 QB3 「開閉だけ」の線引きがATOKで反証 | 反映（経験的例外と明記）／一般化は表・較正の後 | 決定1。線引きの節そのものは、ユーザーが「表から引く」と決めたので残す |
| round4 QB4 ADR-192決定3bの前提 | 見送り（削除案）／訂正済み | ADR-192で前提を訂正し、ユーザーの要望（親指キーを強制ON/OFFにしたい人がいる）で残した。QM4・QM5の制約は実装前に確認する |
| round4 QM1 TsfNativeの回復経路 | 反映 | TsfNativeの節（開閉軸のみ、かな/英数軸は回復不能） |
| round4 QM2 決定1が決定3に依存する循環 | 反映 | 決定5「決定の依存順」 |
| round4 QM3 予測と通過マークの意図全消し | 反映 | 決定2（予測は意図でなく専用枠なので対象外、最初の再読み取りは訂正しない） |
| round4 QM4・QM5 決定3bの適用条件 | 見送り | ADR-192に「実装前に`resolve_pending_thumb_as_single`で確認」と明記済み |
| round4 QM6 複雑性収支の対応表 | 反映 | 決定5の収支表（実数） |
| round4 QM7 同型機能の出荷失敗の前例 | 反映 | 決定4（ADR-192にも1行） |
| round4 D 削れるもの | 一部反映 | 決定5「削る・見送るもの」 |

集計（上の20行、指摘の束）: 反映15、反映済み2（NB10残り・QB1）、見送り2（QB4・QM4/QM5）、一部反映1（D）。事実誤認と判定した指摘は無い（RM1はコードで確認: `actuation_for`の呼び出し元は`ir_apply_drift_correction`の1箇所）。

