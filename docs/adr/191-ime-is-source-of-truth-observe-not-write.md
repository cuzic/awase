---
id: ADR-191
title: |-
  IMEの状態はIME自身を正とし、awaseは書き込まず観測に追随する（設計転換）。キー効果は注入で学習・検証し、成功基準は撤去量で測る
summary: |-
  awaseはIMEの開閉・変換モードを書き込む経路（ImmCross書き込み、shadow-toggle代行、drift補正等）を累積させ、その書き込みが
  IME自身の動きから外れてモードずれ（IMEは半角英数なのにEngineがON、等）を生んできた。実機の計測（GJI×ATOK×Win32 Edit、
  スパイクの`--walk`/`--exp`）で、IME単体の効果はMozcの公開キーマップからほぼ予測でき（一段98.4%）、awaseを通すと仕様から外れる
  （87.2%、Engine/実IMEのずれ19〜23%）ことを確認した。方針: (1)IMEを状態の正とし、通常時はawaseがIME状態を書かない。
  (2)EngineはモードキーのPassThrough後に実IMEを読み直して追随する（ADR-187のfollowを全モードキーへ一般化。BUG-151の修正）。
  (3)「(状態,キー)→次状態」の表を予測として使い押下直後の1打から正しく動かし、観測で確認・訂正する。表の出所は静的な初期仮説
  （GJI: config1.db+Mozcキーマップ）と、明示的な較正セッションでの注入学習（カスタムキーマップ・MS-IMEでは必須）。
  (4)成功基準は削除した書き込み経路の数（`RESTRICTED_CALLS`の縮小）。追加は削除と同時にのみ許す。
status: |-
  **草案（2026-09-20、未レビュー）。** 実装前にopus-adversarial-consultで収束させる。本ADRは方針と撤去の順序を定めるだけで、
  コードはまだ変更していない。決定2（BUG-151の修正）だけは独立に着手できる最小の変更として先行させてよい。
related_adr:
  - "ADR-138"
  - "ADR-162"
  - "ADR-176"
  - "ADR-186"
  - "ADR-187"
  - "ADR-188"
  - "ADR-189"
---

# ADR-191: IMEの状態はIME自身を正とし、awaseは書き込まず観測に追随する

## ステータス

**草案（未レビュー）。** 上記のとおり。以下の実測の出典は`spike/ime-effect-learning`ブランチ
（`tools/e2e/ime_key_matrix/`、結果は`results/elw2`・`results/elw8`、コミット`91f17341`ほか）。

## 背景

awase最大の難所は、IMEのON/OFF/変換モードの追跡である。これまで、追跡が外れるたびに「awaseがIMEへ書き込んで揃える」経路を
足してきた（[ADR-158](158-complexity-reduction-north-star.md)のRC4が指摘した、加算のみを報いる構造）。その結果、
`RESTRICTED_CALLS`（`lints/actuation_call_guard`）が示すとおり、SendInputの送信元が19、`apply_ime_open_with_view`/
`apply_ime_open_with_belief`の呼び出し元が計3つ残っている（reassert・force-on・DirectInput分岐はADR-178/185で撤去済み）。
書き込みは「UIミラーにすぎず実モードに届かない」（BUG-25）ことがあり、awase自身の書き込みが観測を汚し、IME本来の動きから
外れる（ADR-176決定1が警告した自作自演）。

ユーザー方針（2026-09-20）: **IMEの書き込みを尊重する（IMEを状態の正とする）ようawaseの設計を大幅転換したい。そのための
キャリブレーション（IMEが各キーにどう反応するかの学習）を作る。成功基準は撤去量で測る。**

## 実測（要点）

実機（GJI×ATOKプリセット、Win32 Editの入力欄、スパイクが実打鍵の結果を記録）。**測定時は有効なIME（TIP）を必ず記録した**
（2026-09-20の一部のランがMS-IMEだったことに後から気づいた事故があり、GJIで測り直した。下記「限界」参照）。

1. **IME単体は仕様どおり。** `--exp`（ON・かなでひらがなキー0xF2を押し、直後に`a`を打つ）は、awase完全バイパス（A'）・awase素通し（A）・
   awase経由（B）の全条件で12/12「ONのまま半角英数へトグル」。Mozcの`atok.tsv`（Precomposition `Kana`=ToggleAlphanumericMode、
   DirectInputの`Kana`は未定義でOFFのまま）と一致する。Windowsでは`VK_DBE_HIRAGANA`(0xF2)がMozcの`KeyEvent::KANA`になる
   （`keyevent_handler.cc`）。
2. **Mozc仕様だけの静的モデル（学習なし）の予測精度**（ランダム押下列`--walk`、各2ラン・約190押下）:

   | 条件 | 一段予測 | 開ループ追随 |
   |---|---|---|
   | A'（awase完全バイパス） | 98.4% | 84.5% |
   | A（awase素通し・再注入あり） | 98.9% | 90.5% |
   | **B（awase経由）** | **87.2%** | **36.7%** |

   awaseを通すと実IMEの動きが仕様から外れる。Bでは100手に対しIME開閉の書き込み（ImmCross）が39回あり、awaseの自己操作が付随した押下は
   92/100、Engineと実IMEのずれは19〜23%だった。
3. **表だけでは決まらない分岐が少数ある**（一段で数%）。例: 入力中の無変換（未確定文字列の内容で結果が割れる）。表は予測であり、
   確定には観測が要る。
4. **モードずれの1つを決定的に再現し原因を特定した（[BUG-151](../known-bugs/BUG-151.md)）。** coldのawaseでひらがなを押しGJIが半角英数に
   なった後も、Engineが追随せず次の`a`がNICOLAの`う`になる（12/12）。原因はコードで確認した（決定1の根拠）:
   ADR-187のfollow（`kp_stage_mode_key_follow`）は無変換/変換だけが対象（`is_convert_or_nonconvert`かつ`shadow_action.is_none()`）で、
   ひらがなは対象外。ひらがなは`shadow_action=TurnOn`（awaseの「ひらがな=IMEをONにするキー」という静的な決めつけ、`vk.rs`の
   `0xF2 => Activate`）として扱われ、IMEが既にONのため`[shadow-toggle] no-op ... apply-ime 見送り`（書き込みなし、`applied`はUnknownのまま）。
   20ms後の再読み取りは`ir_decide_read_strategy`のtyping-idleガード（500ms以内はスキップ）に当たり、バイパス`explicit_verify`は
   `applied != Unknown`を要求するため効かず毎回スキップされる（`idle=31〜47ms`）。`explicit_intent`が確定しているので
   `reschedule_ime_refresh`は次回ポーリングも止め（`explicit_intent().is_some()`で早期return）、以後どの再読み取りも起きない。
   warm（awaseが既に書き込み済みで`applied`既知）ではバイパスが効き再現しない（`ime open applied`が43件対2件）。
5. **ひらがなの静的な決めつけは実際にIMEごとに違う。** GJI(ATOK)ではDirectInputでは何もせず、ON・かなでは半角英数トグル。MS-IMEでは
   （同日の別ランの計測、TIPはelw6で確認）DirectInputで開き、ON・かなで閉じた。awaseの`Activate`固定はどちらとも合わない。

## 決定

### 決定1: 原則 — IMEが状態の正。通常時awaseはIMEの開閉・変換モードを書き込まない

キーは可能な限り生のままIMEへ通し（PassThrough）、awaseは結果を観測して追随する（ADR-187の「follow方式」を原則に格上げする）。
モデル（awase側の推定）は観測を書き換えない（ADR-098、`.claude/rules/ime-belief-architecture.md`の不変条件は維持）。
IMEへ書き込む経路は、ユーザーが明示的にopt-inした機能（例: 親指キーの単独タップを別キーとして再送する等）に限り、既定は書かない。

### 決定2: 追随を全モードキーへ一般化する（BUG-151の修正、先行着手可）

- `kp_stage_mode_key_follow`（`runtime/key_pipeline.rs`）の対象を`is_convert_or_nonconvert`から「IMEモードキー全般
  （`event.ime_relevance.is_ime_mode_key`）」へ広げ、`shadow_action.is_some()`による除外を外す。通過したキーには通過マーク＋
  20ms再読み取り（typing-idleバイパス）を予約する。
- `ir_decide_read_strategy`の`explicit_verify`から`applied != Unknown`の分岐を**削除**する（全モードキーが通過マークを持つので冗長になる）。
- 検証: `--exp`のB（cold）を12/12失敗から0/12へ、`--walk`のBのEngine/実IMEずれ率（19〜23%）が下がること。
- これは「追加」ではなく、専用の分岐を1つの通過マーク機構へ統合する変更として行う（決定5の撤去計画の第一歩）。

### 決定3: 表は予測として使う。出所は静的な初期仮説と注入学習

「押下直後の1打から正しく動く」（要件: かな=Engine ON、英数=Engine OFF）ために、(状態, キー)→次状態の表でEngineを先に動かし、
決定2のsettle後の観測で確認・訂正する。表の出所:
- **静的な初期仮説**: GJIは`config1.db`（`session_keymap`+`custom_keymap_table`+`overlay_keymaps`）とMozcの公開キーマップ
  （`atok.tsv`/`ms-ime.tsv`/`kotoeri.tsv`）から機械的に作る。上の一段98.4%が根拠。ただし`session_keymap`と`custom_keymap_table`は
  食い違いうる（BUG-143）ので**初期仮説に留める**。
- **注入による学習・検証は必須**（決定4）。カスタムキーマップのユーザーがいるため、静的な読み取りだけでは実IMEの実挙動と一致する保証がない。
  MS-IMEは公開キーマップがなく学習のみ。
- 表と観測が食い違ったら観測が勝つ。非決定セル（入力中の無変換など）は表で確定せず観測に委ねる。

### 決定4: 較正セッション（注入による自動学習）

ADR-176（ユーザーが物理キーを押す方式、実装済み）を、明示的なセッションでの自動注入へ拡張する。通常実行時のバックグラウンド注入は
ADR-176が却下済みでその判断は維持する（BUG-113/124の「@」）。

- awase-settingsがユーザーの明示操作で開始。awaseは対象窓で完全バイパス（`disable_apps`相当。実測でA'は自己操作0）。
- 注入キーはセッションごとのランダムなノンスを`dwExtraInfo`に付け、開始IPC（`WM_CALIBRATION_START`）でPIDと共に渡して検証する
  （固定マーカーは他プロセスが偽装できる）。セッション終了・タイムアウト・awase-settings終了で無効化。
- 測定対象は「状態×キー」の全掃引（ランダム押下列はセルを網羅できない: 未学習13%）。IMEの実状態は開閉・変換モードの読み取りに加え、
  **実打鍵の結果**（未確定文字列・確定テキスト）で検証する（BUG-25の教訓）。**測定の前後で有効なTIP（GJI/MS-IME）を必ず記録する。**
- 結果は`config.toml`へ永続化し、stale検出（ADR-176 T11/T12）とopt-inゲートを再利用する。

### 決定5: 成功基準は撤去量。追加は削除と同時にのみ許す

撤去候補と順序（各項目は「削除前に確認すること」を満たしてから）:

| # | 撤去・統合の対象 | 前提・確認 |
|---|---|---|
| 1 | `explicit_verify`の`applied != Unknown`分岐 | 決定2の後（冗長になる） |
| 2 | shadow-toggleの代行（`vk.rs`の`0xF2=>Activate`固定、半角/全角のSuppress+ImmCross書き込み） | ADR-189（半角/全角をawaseがactuate）と方向が逆。ADR-189の見直しが先 |
| 3 | `dispatch_ime_set_open`のうち`EngineDecision`系のImmCross書き込み | 単独タップのトグル（opt-in）を除く。決定1の例外の範囲を確定してから |
| 4 | `ir_apply_drift_correction`（OFF方向の補正書き込み） | 観測への追随（決定2）で代替できることを`--walk`で確認 |

**残すもの**: TSF cold-start warmup（BUG-02/69、IME ONのときだけ発動、`resolve_warmup_ime_on`はBUG-69の修正そのもの）、
PassThroughの再注入（フックの非同期設計上の経路。scan=0で再送する点は別途見直す）、opt-inの親指キー単独タップ機能。
測り方: `RESTRICTED_CALLS`の許可リストの行数、`apply_ime_open_with_view`/`_belief`の呼び出し元数（現状3→0を目標）、削除した行数。
[ADR-162](162-governance-reversal.md) E1（複雑性予算1-in-1-out、未発効）の趣旨と同じ向きで、本ADRの実装コミットは追加と削除を1対1以上で対にする。

## 未解決・リスク

- **BUG-113/124（TsfNative×GJI「@」）**: 「生キーがGJIへ届くこと」も「awase自身のIME操作」も、片方だけで「@」を誘発すると2回の実機A/Bで
  確定している（ADR-176）。書き込みを減らして生キーを通す方向が、この環境で新たな不具合を出さないか、撤去の各段で実機確認が要る。
- **TsfNative（Chrome・Windows Terminal）で同じ表が使えるか未検証**（データはWin32 EditとRichEditのみ）。TSFの入力フォーム
  （COMで豊富なIME状態が取れる）を較正の測定台にする案があるが、ADR-138が却下した自前`ITextStoreACP`（21メソッド）は維持コストと、
  バグのあるtext storeがグランドトゥルースを汚す危険を指摘している。まず実アプリ（Chrome）での実打鍵結果による確認から始める。
- **隠れ変数**: 表で確定できない分岐（入力中の無変換など）の要因は未特定。表単独での開ループ追随は84.5%止まり。
- **ADR-189との衝突**: 半角/全角をawaseがbeliefに基づいてactuateする案は、本ADRの原則と逆向き。本ADRの決定1を採るなら見直す。
- **限界（測定の信頼性）**: 2026-09-20の一部のランは有効なIMEがMS-IMEで、GJI用の結論に使えなかった。今後は測定ごとにTIPを記録する。
  ひらがなのMS-IME挙動（DirectInputで開き、ON・かなで閉じる）はTIPをelw6でのみ確認しており、追試が要る。

## 却下した代替案

- **通常実行時のバックグラウンド注入・受動学習**: ADR-176が「@」とBlocker13件で却下済み。維持する。
- **静的表だけ（較正なし）**: カスタムキーマップと`session_keymap`/`custom_keymap_table`の食い違い（BUG-143）で破綻する。
- **学習表だけで開ループ追随**: 非決定セルがあり88%程度で頭打ち。観測との併用が要る。
- **一括の大規模撤去**: 過去に設計が複雑化して6ラウンド分を破棄した教訓がある。撤去は上の順序で1つずつ、各段を実機で検証する。

## 検証計画

各撤去・統合の前後で、スパイクの`--exp`と`--walk`（A'/A/Bの3条件、有効TIPを記録）を実機で流し、(a)BUG-151の再現0/12、
(b)BのEngine/実IMEずれ率、(c)`RESTRICTED_CALLS`の縮小、を記録する。TsfNative（Chrome）は実打鍵の結果で別途確認する。

## 関連

ADR-138（ウィットネスアプリ却下、自前ITextStoreACPの警告）、ADR-162（複雑性予算）、ADR-176（較正UI、明示セッション、注入の却下範囲）、
ADR-186（ATOKの実測表と追随）、ADR-187（follow方式）、ADR-188（TsfNativeのconvだけを変えるモードキー）、ADR-189（半角/全角のactuate、要見直し）、
BUG-25（IMC読み取り・書き込みは実モードの証明にならない）、BUG-113/124（TsfNative×GJIの「@」）、BUG-143（`session_keymap`と
`custom_keymap_table`の食い違い）、BUG-151（本ADR決定2の直接の動機）。
