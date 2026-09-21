---
id: ADR-191
title: |-
  IMEの状態はIME自身を正とし、awaseは書き込まず観測に追随する（設計転換）。ただしトグル系のキーだけはbeliefに基づいて送る（conv軸はIMEが正、開閉軸はトグルキーに限りawaseが正）。キー効果は注入で学習・検証し、成功基準は撤去量で測る
summary: |-
  awaseはIMEの開閉・変換モードを書き込む経路を累積させ、その書き込みがIME自身の動きから外れてモードずれ（IMEは半角英数なのにEngineがON、等）を生んできた。
  実機計測（GJI×ATOK、スパイクの`--walk`/`--exp`）で、IME単体の一段の効果はMozcの公開キーマップからほぼ予測でき（98.5%）、awaseを通すと仕様から外れる（84.5%、
  Engine/実IMEのずれ19〜23%）。方針: (1)IMEを状態の正とし、通常はawaseがIME状態を書かず、生キーを通して観測に追随する（対象はIMMで読めるアプリ。TsfNativeは範囲外）。
  **(2)唯一の例外はトグル系のキー: 固定のADR-189セット（半角/全角・漢字、TsfNativeでも従来どおり）は変えず、学習表が状態完備で「トグル」と判定した追加キー（開閉軸、および入力モード軸は
  Set型キーが表にある場合のみ）は、実機で素通し追随より有効と確認できてから、beliefに基づいて（低信頼のときは素通しで）送る。**
  (3)BUG-151は最小修正（shadow-toggleがno-opで終わった打鍵に通過マークを立てる）で先に直す。(4)キー効果は(状態,キー)→効果の表として持ち、静的な初期仮説と注入学習
  （カスタムキーマップ・MS-IMEでは必須）で作る。観測にはIMEの実状態・実打鍵結果に加えTSFのスレッドcompartmentの変更通知（開閉・かな⇔英数のpush）を使う。
  (5)較正導入後、変換・無変換・かな・英数・半角全角・漢字などは決め打ちせず表で統一的に扱う。(6)成功基準は撤去量（追加は削除と対）。
status: |-
  **草案（2026-09-21、opus敵対レビューround2まで反映済み、round3待ち）。ユーザー指示（2026-09-21）で、決め打ちの撤去（決定6の一部）と決定2の一般化を先行して実装済み（`feat/adr191-remove-hardcoded-mode-keys`、未マージ）。** 決定2（BUG-151の最小修正）は独立に先行してよい。他は実装前にレビューを収束させる。
related_adr:
  - "ADR-138"
  - "ADR-162"
  - "ADR-176"
  - "ADR-186"
  - "ADR-187"
  - "ADR-188"
  - "ADR-189"
  - "ADR-192"
---

# ADR-191: IMEの状態はIME自身を正とし、awaseは書き込まず観測に追随する

## ステータス

草案。opus敵対レビューround1（Blocker7・Major14・Minor6）の指摘のうち、コードとログで独立に確認できたものを反映した（確認できなかった1件は下記「実測」で理由を記す）。
測定の出典は`spike/ime-effect-learning`ブランチ（`tools/e2e/ime_key_matrix/`、結果は`results/elw2`・`elw8`・`elw9`）。

## 背景

awaseの最大の難所は、IMEのON/OFF/変換モードの追跡である。追跡が外れるたびに「awaseがIMEへ書き込んで揃える」経路を足してきた
（[ADR-158](158-complexity-reduction-north-star.md) RC4が指摘した、加算のみを報いる構造）。書き込みは「UIミラーにすぎず実モードに届かない」
（BUG-25）ことがあり、awase自身の書き込みが観測を汚してIME本来の動きから外れる（ADR-176決定1が警告した自作自演）。

ユーザー方針（2026-09-20）: IMEの書き込みを尊重する（IMEを状態の正とする）ようawaseの設計を大幅転換する。そのためのキャリブレーションを作る。
成功基準は撤去量。ただし**トグル系のキーだけは、IMEが今の実状態に依存して結果を変えるため、awaseがbeliefに基づいて送る**（唯一の例外）。

## 実測（要点と限界）

実機（GJI×ATOKプリセット、スパイクの標準Edit入力欄。実打鍵の結果を記録）。**B（awase経由）の測定に使ったawaseは`e2e/ablation`（コミット5f11a872、ADR-186＋実験用の変更）で、
ADR-187（follow）・ADR-188・ADR-189（半角/全角のToggle上書き）の実装を含まない**（developの現行とは違う）。Bの結果をdevelopの現行の性質として読まない。ログの「ROUND 2/2: RichEdit」の見出しは、フォーカス制御が入力欄（Edit）へ戻すので実態を
表さない（awaseの`imm-cross-actuate`の対象は`class="Edit"`、確定文字は`Edit`の内容に入る。レビューが「RichEditで測っていた」としたのはこの見出しの読み違い）。

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

**例外は2段。いずれも「トグル」（押した結果が今の実状態に依存して反転するキー）だけが対象。**
1. **固定の例外（ユーザー指示で撤去する方向。下記「実装の現状」参照）**: ADR-189の現行セット（`VK_KANJI`0x19・`VK_DBE_SBCSCHAR`0xF3・`VK_DBE_DBCSCHAR`0xF4、GJIアクティブ・無修飾）と、ユーザー設定`keys.ime_toggle`。
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

`kp_stage_shadow_ime_toggle`が**no-op（`effective_open() == current`、awaseは書き込む必要が無かった）で終わった打鍵**にだけ、通過マーク（`arm_mode_key_pass_mark`）を立てる
（`key_pipeline.rs`のno-op分岐、10行程度）。20ms後の再読み取りは通過マークでtyping-idleガードを越え、`applied`がUnknownでも実IMEを読む。
- **理由（正確に）**: この打鍵の意図は、no-op判定の**前**に`intent_store`/`last_intent`へ既に記録されている。no-opの時点で**beliefが既に目標と一致しており、意図を捨てても失う情報が無い**
  から、通過マークが意図を捨てても安全である（「意図が無いから」ではない。武装位置をno-op分岐より前へ動かさないこと）。
- **ケース3改（`ExplicitImeActionOutcome::SuppressOnly`、BUG-124対策）は`key_pipeline.rs`でno-op分岐より前に`return false`する**ので、この修正は触らない。「@」の再発経路は構造的に回避されている。
  武装をこのreturnより前へ動かさない。
- **武装条件に「飛行中のactuationが無いこと」を足す**（`ime_refresh.rs`の`actuation.attempts`で判定）。通過マークの観測は`invalidate_intents_if_mode_key_pass_live`（`intent_store.remove(hwnd)`、
  **窓単位**で意図を全部消す）を呼ぶので、直前の明示IME操作（Ctrl+変換）のImmCross書き込みが飛行中に無関係な意図まで巻き添えにしない。
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

### 決定4: 較正セッション（注入による自動学習）

ADR-176（ユーザーが物理キーを押す方式）を、明示的なセッションでの自動注入へ拡張する。**これはADR-176の免責条件（「awase自身はキーを送信しない」）の拡張で、本ADRが明示的に判断する**
（ADR-176が却下したのは通常実行時のバックグラウンド注入。BUG-113/124の「@」は、TsfNativeのアプリへ生キーが届くこと、およびawase自身のIME操作が原因）。根拠と条件:
- 注入先は**較正のために新設する自前のWin32テキストコントロール**に限る（他アプリへは送らない。awase-settingsの現行UIはeframe/eguiで、自前のWin32コントロールは存在しない。新設は追加コストとして計上し、eguiのまま
  測るならBUG-107/125のegui窓のプロセス間汚染の上で測ることになる）。TsfNativeアプリへ生キーが届く経路が無いので、BUG-113の機序（TIPのキー横取り）は成立しない。実測（自身の入力欄への大量注入で「@」が出ない）は
  機序の不成立の証明にならないので根拠にしない。出荷前に較正窓での実機A/B（GJI・MS-IME）を条件とする。**キーマップはIMEのプロパティなので窓に依らないが、composition状態は窓ごと**なので、
  較正窓の表を全アプリへ適用してよい根拠は「キーマップの効果は窓に依らない」ことに置き、composition状態は観測で扱う。
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
- 複雑性収支の見積（レビュー）: 較正基盤の追加は数千行、撤去は現実的に数百〜約二千行。純増になりうるので、較正はP1・P2の撤去が頭打ちになってから、追加は削除と対で出す。
  [ADR-162](162-governance-reversal.md) E1（複雑性予算1-in-1-out、未発効）と同じ向き。

### 決定6: モードキーは決め打ちせず、学習結果で統一的に扱う

較正を導入した以降、変換・無変換・かな（ひらがな）・カタカナ・英数・半角/全角・漢字などのIMEモードキーは、キーごとの意味をコードに持たず、決定3の表を唯一の情報源として同じ経路で扱う。
コードが静的に持つのは「このVKはモードキーか」の分類だけで、押したときに何が起きるか（Set/Toggle/None）は表から引く。**例外は、決定1の固定セット（ADR-189の0x19/0xF3/0xF4と`keys.ime_toggle`）だけを
静的に残すこと（明示的な1箇所の例外）**。それ以外のトグルキーは、状態完備の条件を満たしたときだけ表から追加される。
- 親指シフトのキー（無変換/変換など）: チョード判定はNICOLA側の別軸のまま残す。単独タップのときIMEに何が起きるかだけ表から引く。`ModeKeyConfig`等の**ユーザー設定は「ユーザーが何をしたいか」の軸で、
  表（IMEがどうなるか）に畳み込まない**。
- 撤去対象（決め打ちの箇所）: `vk.rs::ImeKeyKind::shadow_effect`（VK別の静的効果、①の役割）、`runtime/mod.rs`の`shadow_action`上書きのうち**ADR-189の固定セット（0x19/0xF3/0xF4）は撤去しない**（TsfNativeで効いている例外そのもの）。上書き点は1箇所だが、供給元が約8箇所
  `set_thumb_key_shadow_overrides`等にあり、複雑性は供給元側（固定セット以外を対象にする）、`dbe_mode_key_policy`（半角/全角の一律Suppress/Allow）。いずれも既定表（データ）で同じ挙動が再現できることを`--walk`/`--exp`で確認してから。
- 表が空の環境は同梱の既定表を使う（従来の直書きの意味づけを、同じ内容のデータへ移す）。撤去量は「コードの分岐・行の削除」で数える。

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
- **学習表だけで開ループ追随**: 1手の誤りが連鎖し（A'で64%）頼れない。観測との併用が要る。
- **トグルも生キーを通して観測追随だけにする（例外なし）**: トグルは実状態依存で反転しない/二重に反転するずれの温床（ADR-189の実測）。ユーザー方針で例外を採る。
- **一括の大規模撤去・較正基盤の先行**: 複雑化と純増の危険。撤去を先に、1つずつ。

## 検証計画

各撤去・統合の前後で、スパイクの`--exp`と`--walk`（A'/A/B、有効TIPを記録）を実機で流し、(a)BUG-151再現0/12、(b)BのEngine/実IMEずれ率、(c)決定5の指標、を記録する。
TsfNative（Chrome）は実打鍵の結果で別途確認する。

## 関連

ADR-138（ウィットネスアプリ却下、自前ITextStoreACPの警告）、ADR-162（複雑性予算）、ADR-176（較正UI、注入の却下範囲、免責条件）、ADR-186（ATOKの実測表と追随）、ADR-187（follow方式）、
ADR-188（TsfNativeのconvだけを変えるモードキー）、ADR-189（半角/全角のactuate、例外の土台）、BUG-25（IMC読み取り・書き込みは実モードの証明にならない）、
BUG-113/124（TsfNative×GJIの「@」）、BUG-143（`session_keymap`と`custom_keymap_table`の食い違い）、BUG-151（決定2の直接の動機）。


## 実装の現状（2026-09-21、`feat/adr191-remove-hardcoded-mode-keys`、develop未マージ）

ユーザー指示: **GJI/MS-IMEの設定どおりに動かすことを優先する。これまでの挙動はMS-IMEプリセット前提だったため、変換・無変換・かな・英数・半角/全角の決め打ちを撤去し、awaseなしで学習した結果に従う。
実測の外れ31件のグループ1（ひらがなをOFFで押すとON）・2（入力中の無変換/変換でIME OFF）・3（半角/全角の方向固定）の機構を撤去する。** これにより、上の決定1の「固定の例外（ADR-189）」も撤去した
（TsfNativeでのEngine追随の退行は実機A/Bで確認する。未確認）。実装した範囲（develop比: 15ファイル、追加62行・削除3,014行）:
1. `vk.rs::ImeKeyKind::shadow_effect`を`VK_IME_ON`/`VK_IME_OFF`（Windows標準の冪等キー）だけに（他は`None`）。`ShadowImeEffect::Toggle`を削除。
2. `enrich_ime_relevance`の`shadow_action`上書き連鎖3系統（ひらがな/カタカナ、無変換/変換、ADR-189の半角/全角）を削除。
3. 追随（ADR-187 follow）を`is_followed_mode_key`（IMEモードキーから`VK_IME_ON`/`VK_IME_OFF`を除く全て）へ一般化（BUG-151の修正。決定2の縮小案A1ではなく、静的な`shadow_action`の撤去で「`shadow_action`を持つキーは追随しない」
   条件がそのまま働く形）。
4. エンジン本体から、無変換/変換/ひらがな/カタカナの単独タップ代行（`*_delegate_to_open_axis`）、ひらがな/カタカナ親指キー設定、Shift+代行キーの素通し判定を削除。単独タップは専用Fnキー→ユーザー明示config→`ModeKeyConfig`
   （ユーザー設定のSuppress/Passthrough）の順で解決する。ADR-182決定1bの抑止は残した。
5. Windows側のGJI/MS-IMEの設定からの自動検出・配線（`sync_gji_charset_autodetect`、MS-IMEレジストリ由来のdelegate/override）と、較正結果を分類へ反映する関数を削除（較正の永続化・IPCは残る）。
6. 撤去した機能の単体テスト約75本を削除（うち2本の「保留中のIME開閉要求が漏れない」は書き直しが要る）。
**未実装（続くコミット）**: `kp_stage_shadow_ime_toggle`の代行所有権の分岐（現在は定数で偽）、`transport.rs::plan`のDBEキー・Suppress分岐と約25本のテスト、設定`gji_thumb_key_ime_toggle`/`dbe_mode_key_policy`と設定UI、
`auto_delegate_open_axis_consumed`マーカー、`ModeKeyActuationOwner::FsmDelegate`。**実機A/B（develop+本ブランチのビルド、`--exp`/`--walk`のA'/B）は未実施。**


## TsfNative（観測できないアプリ）の扱い（2026-09-21、ユーザー整理）

観測できないアプリ（`Imm32Unavailable`・`TsfNative`・`InputRelay`）では、IMEの状態を一切読まず、TsfNativeでは`reschedule_ime_refresh`がポーリングを予約せずに戻る（コードで確認）。「観測に追随」は
そこでは定義できず、決め打ちの撤去により、状態依存のキー（入力中かどうかで結果が変わるキー）を使うユーザーには、モードずれが起きるようになる。これを**受け入れる**（ユーザー判断）:
- 冪等なキー（`VK_IME_ON`/`VK_IME_OFF`）はずれない。ずれるのは状態依存のキーを使うユーザーだけ。
- ずれは、(a)**IMトグルのawaseによる書き込み（ADR-189。残す機能）**、(b)**awaseが強制的にactuateする強制ON/OFFの打鍵**（`keys.ime_on`/`keys.ime_off`。既定値がCtrl+変換/Ctrl+無変換というだけで、configで別のキーに上書きしていればそのキーになる）で、強制的に解消できる。実用上の問題はない。
- 状態依存のキーを使うユーザーは**自己責任・ベストエフォート**とし、その手助け（検出・警告・冪等なキーへの置き換えの案内）は**別ADR（[ADR-192](192-state-dependent-mode-key-warning-and-guided-override.md)）**で扱う。
- **ADR-189のトグル（0x19/0xF3/0xF4）の書き込みは撤去しない**（実装ブランチで誤って撤去したので復元する）。TsfNativeでのEngineの追随は、これに依る。
