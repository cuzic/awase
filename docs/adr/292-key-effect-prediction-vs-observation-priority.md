---
id: ADR-292
title: |-
  KeyEffectPredictionが未確認のままローマ字を送り、ADR-268 D3の既存settle機構のローリングbaselineが
  訂正自体の打鍵で遅延し、ADR-270の安全網も開の未確認を理由に機能しないためリテラル化する(BUG-233)
summary: |-
  2026-11-12、`herdr-app.exe`(class=`Zed::Window`)で「これ、」と入力しようとして`kore,`と
  リテラル化する実機報告([BUG-233](../known-bugs/BUG-233.md))を調査した。フォーカス変更
  直後にモードキー(IME ONトリガ、VK_CONVERT)を押下した際、awaseは学習済みキー効果予測
  (`KeyEffectPrediction`、ADR-191決定3)だけを根拠に`effective_open`をtrueへ倒し、
  mozc-shm(mozc-readable)の実測による裏取りを待たずに後続のローマ字を送信した。実際には
  モードキーがMozcを開けておらず(原因未解明)、リテラルとして出力された。

  round1〜round4のOpus敵対的レビューで、当初の原因分析・修正候補に複数の重大な誤りが
  見つかり訂正した(全文: [review/292-opus-review-round1.md](review/292-opus-review-round1.md)〜
  [round4.md](review/292-opus-review-round4.md))。round3でG3・R6は収束、round4でPの
  最後の2件(U1: 即座解放の条件が「閉」の読みを誤って含んでいた、U2: 予測ごとに1回に
  限定しないと毎打鍵で保留が繰り返される)を修正し、**ADR全体として収束**と判定された。

  - **D1(構造的懸念、BUG-233の直接原因ではないと訂正)**: `resolve_open_at`の固定170ms
    フェンスは、送信(20.989)の時点で既に通過済み(フェンスは`pred.at_ms`=20.671から
    170ms=20.841で終わる)。このフェンス自体はADR-191決定3の意図的設計(観測プールに残る
    古い観測が予測を覆すのを防ぐ、BUG-232と同型の問題への対策)であり、優先順位を入れ替える
    案(F1)は却下。
  - **D1b(確定。ADR-291 D5への帰属もCIで確認済み)**: 実際に訂正(`[key-effect-miss]`、
    23.142)を遅らせたのは、`decide_use`(ADR-268 D3、一部ADR-291 D5)がbelief食い違い時に
    要求する`settle_ms`無条件待ちで、その起点`baseline_ms`(`hook::last_operation_tick_ms()`)
    が、自己注入・KeyUp・ウォッチドッグのカナリアを含む全フックコールバックで再計算される
    ローリング値であること。実測の23.142は、`TIMER_MOZC_SHM`の500ms周期ポーリング(20.634
    起点)のうち、21.134〜22.634が直前の送信/Backspaceからまだ170ms経っておらずHeld、
    最後のBackspace(22.617)から170ms以上静止した後の最初のポーリング(23.134)でUseになった、
    という位相の一致で説明できる(差8ms)。CI(GitHub Actions run `43220508025`、
    `tickchurn --closed`シナリオ)で「Mozcが閉のままでも打鍵で`update_tick_ms`が進む」ことを
    実機確認し、D5がBUG-233の遅延を実際に延ばしていたことを確定した(D5以前ならfast pathで
    21.134付近に即座に`Use`されていたはず)。
  - **D2(確定、ただし有効な修正はH1ではない)**: `should_check_compose`の前提条件問題は
    実在するが、`judge_compose`自体が`!b.effective_open`で即座に`Skip(ImeNotActive)`を
    返す設計のため、`should_check_compose`の前提だけを外す案(H1)は何も変えない。さらに
    回収(Backspace+再送)は閉のIMEへの再送を繰り返すだけで直らない。

  round1レビューで最も重大な指摘は**R6**: 観測が「閉」と確定したときawaseが何をするかを
  ADRが決めていないこと。belief を閉に戻すだけ(候補G3はこれしかしない)では、ユーザーには
  `kore,`とは別の症状(Engineが非活性化し、NICOLAのローマ字がそのままqwertyの生文字として
  素通しになる)が出るだけで、BUG-233本来の問題(ユーザーが開きたいと思って押したモードキー
  が効かない)は直らない。

  round2レビューでG3・P・R6の3択それぞれの設計自体にも重大な欠陥が見つかった。G3の起点
  `last_mode_key_tick_ms`は「IME書き込みを**要求**した時刻」であり、実際に**送信**した
  時刻ではないため、そのままではBUG-232が(単体テストでは検出できない形で)再発する(要:
  実送信の打刻を起点に追加するS1)。Pは「既存の確認の仕組みをそのまま使える」という前提が
  誤りで、warm送信には送信前に読む場所が無く、ローマ字ではなく物理キーを(既存の
  `focus_switch_hold`/`resume_hold`と同じ形で)保留する「第3のhold」として再設計が必要
  (S3〜S6)。R6は(ii)「保留分を捨てる」が利用者に無断で文字が消えるため不可、(i)「開き
  直す」は機構・冪等性・ADR-119合流点規約・IntentStore順序の4点が未検討、新たに
  (iv)「belief修正+保留分の流し直し+利用者通知」が選択肢として追加された(S7・S8・S8')。

  round3レビューでG3(S1打刻を含む起点)・R6(選択の構造)は収束と判定されたが、**Pは
  なお未収束**: 解放判定(「開いた」の確認)にローリングbaselineを使うと、保留中の打鍵
  自体がbaselineを進めてしまい、予測が正しくMozcが正常に開いた通常の場面でも保留が
  解けなくなる重大な欠陥(T1)。保留中のモードキー再押下の扱い(T2)、お手本のhold
  (`resume_hold`)がフォーカス変更時に保留分を無断で捨てる仕様でADRの方針と矛盾する点
  (T3)、開始条件・新定数の実測義務・正常時の遅延増分測定の欠落(T4)を反映して再修正した。

  G3・Pはコード実装済み、[PR #936](https://github.com/cuzic/awase/pull/936)として
  developへマージ済み(`6439bc7d2`、2026-11-12T14:15:43Z。PRの件名・本文はG3のみ言及
  しているが、実際の差分にはP〈19ファイルの大半〉も含まれる)。マージ後のround5 Opus
  レビュー(全文: [round5.md](review/292-opus-review-round5.md)・
  [round5followup.md](review/292-opus-review-round5-followup.md))で、Pに既知の限界が
  見つかった: cold Mozcがモードキーを処理してから`tsf_open`を共有メモリへ公開する
  まで、settle_ms(170ms)+読みの周期より遅れる場面では、Pは保持していた打鍵を「閉で
  確定」と誤判定し、その時点で即座にqwerty生文字として再生してしまう。モードキーと
  後続のローマ字は同じ入力キューで順番に処理されるため、Pが無ければ内部的には正しく
  合成されていた場面であり、Pの再生後にMozcの公開が追いついて実際に開くと、既に
  送った生文字がローマ字として誤変換される(BUG-233の`kore,`型リテラルとは逆の
  症状。詳細は下記「決定」節の既知の限界)。害は`judge()`が閉の確定を根拠に**即座に**
  再生する判断にあり(ADR-191決定3の既存reconciliation自体は無害)、是正
  (再生までの待ち時間をP専用に設ける等)は実測(CI確証計画項目5、cold Mozcの
  `tsf_open`公開までの遅延分布)を待って判断する、既知の限界として記録する。
status: |-
  調査完了、round1〜round4 Opusレビューで原因の帰属・有力候補の設計を複数回訂正
  (2026-11-12)。G3・R6・Pともround4で収束と判定。G3・Pはコード実装済み、
  [PR #936](https://github.com/cuzic/awase/pull/936)として**developへマージ済み**
  (`6439bc7d2`、2026-11-12T14:15:43Z)、`/code-review high`の指摘を反映済み。
  R6(観測が閉と確定したときの行動)は(iv)(belief修正+流し直し+警告ログのみ、
  利用者へのUI通知は未実装)をデフォルトとして実装済み、(i)開き直しは原因特定
  (CI確証計画1)待ちで未実装。マージ後のround5 Opusレビューで、Pの既知の限界
  (上記summary参照、CI確証計画項目4で想定していたリスクが机上の検討で起きうると
  判明〈実測は未〉)と、本ADR・[BUG-233](../known-bugs/BUG-233.md)の記述が実装・
  マージ後の事実と食い違っていた複数箇所を修正(本コミット+round5followup反映の
  追随コミットで反映)。残りのCI確証計画(項目1・3・4・5)・実機確認(dragonflyg4、
  現在オフライン)・**本ADRドキュメントブランチ自体のdevelopへのマージ**(コードは
  既にマージ済みだがドキュメントは本PRで初めてマージされる)は未着手。
related_adr:
  - "ADR-191"
  - "ADR-268"
  - "ADR-270"
  - "ADR-285"
  - "ADR-288"
  - "ADR-291"
  - "BUG-233"
---

# ADR-292: KeyEffectPredictionが未確認のままローマ字を送る問題(BUG-233)

## 背景

[BUG-233](../known-bugs/BUG-233.md): dragonflyg4、`herdr-app.exe`(class=`Zed::Window`)で
「これ、」と入力しようとして`kore,`とリテラルに出た(2026-11-12 20:24:20〜23 JST)。

ユーザーからの要望: 「mozc-readableを使っている限りにおいては、そういう予測ではなく、
確実な状態観測に基づいてアクションしてほしい」。これを踏まえ、同種の「mozc-shm実測が
利用可能なのに、予測/ヒューリスティックがそれを素通りしてしまう箇所」をコードベース
全体で調査した。

**本ADRはround1〜round4のOpus敵対的レビューで原因分析・修正候補の多くが訂正され、
マージ後のround5レビューでPの既知の限界と記述の食い違いが見つかった。以下は
round5反映後の内容。**(訂正の経緯はgit履歴・
[review/292-opus-review-round1.md](review/292-opus-review-round1.md)〜
[round5.md](review/292-opus-review-round5.md)参照)

## 実機タイムライン(UTC, awase.log)

1. `11:24:20.193` フォーカスが`Zed::Window`へ変化。
2. `11:24:20.634` mozc-shm実測により`effective-open-flip → false`(`DeriveHigh(MozcShm)`、正)。
3. `11:24:20.671`(+37ms) モードキー(vk=0x1C、VK_CONVERT)押下 →
   `[key-effect-predict] open=Some(true)`(学習済み表による予測、裏取りなし)。
4. `11:24:20.675` `effective-open-flip → true decided_by=KeyEffectPrediction`。
   **この時点でmozc-shmの実体はまだopen=falseのまま**。
5. `11:24:20.989`(モードキーから**+318ms**、D1の170msフェンスは既に通過済み)〜`21.526`
   `romaji="ko"`→`"re"`→`","`をGJIへ送信 → リテラル`kore,`。
6. `11:24:21.895`〜`22.617` Backspace×6で削除。
7. `11:24:23.142`(モードキーから+2471ms、最後のBackspaceから+525ms)
   `[key-effect-miss] axis=open predicted=true observed=false` — mozc-shmの実測がようやく
   追いつき訂正。

## 調査範囲(mozc-shm実測を無視できる経路の全数調査)

`.claude/rules/ime-belief-architecture.md`が列挙する「観測でも意図でもない直接書き込み」
4イベント(`PanicReset`/`HwndCacheRestored`/`KeyEffectPredicted`/`ModeKeyPassedThrough`)の
呼び出し元を確認した。

| イベント | mozc-shm実測を無視する構造か | 根拠 |
| --- | --- | --- |
| `KeyEffectPredicted` | **はい(D1/D1b)** | `resolve_open_at`で専用フェンス付きの優先分岐を持つのはこれだけ |
| `HwndCacheRestored` | 一部(訂正: 「即座」ではない) | `key_effect`フィールドを設定しないため`resolve_open_at`のフェンスは経由しないが、次のmozc-shm観測もやはり`decide_use`のHeldゲート(D1b、ADR-268 D3 R3')を通る。D1bと同じ遅延を受ける可能性がある。**G3の対象外**(`FocusChanged`で`last_mode_key_tick_ms`が`None`になり、S2'によりローリングbaseline〈`last_operation_tick_ms`〉へ戻る、round5) |
| `PanicReset` | いいえ(意図的) | `ForceGuard`経由。観測を無視するのではなく「常時強制」という別の意図的仕様(パニック時の最終手段) |
| `ModeKeyPassedThrough` | いいえ(むしろ逆) | `align_desired`時のみ観測由来の値に揃える設計。観測を採用する方向 |

さらに、半角英数トグルの手放し判定(`half_width_alnum.rs::plan_mozc_shm_follow`、ADR-290)は
BUG-230修正で既にmozc-shm観測を直接の判断材料にしており問題なし。フォーカス解決
(`focus_resolve_plan.rs`/`focus_classify_plan.rs`)はmozc-shmと無関係な別軸で対象外。

## D1: `resolve_open_at`の優先順位 — BUG-233には効いていない、F1は却下

`crates/awase-ime-core/src/state/ime_model.rs:564-595`:

```rust
let (base, decided_by) = if has_explicit_intent {
    (self.desired_open, BaseDecision::ExplicitIntent)
} else if let Some(predicted) = self.key_effect.and_then(|p| p.open) {
    (predicted, BaseDecision::KeyEffectPrediction)
} else if let Some(outcome) = self.observations.derive_any(now) {
    ...
```

`key_effect`が`Some(open)`を持つ間、観測(`derive_any()`)より先にこの分岐が評価される。
観測との照合(`reconcile_key_effect_open`、800-831行)は`now_ms.saturating_sub(pred.at_ms) <
KEY_EFFECT_SETTLE_MS`(170ms、`pred.at_ms`=20.671からの固定フェンス)の間は動かない。

**フェンスは20.841で終わっており、最初の送信(20.989)は既に318ms後 — フェンスはBUG-233の
結果に関係していない。** 20.841〜20.989の間にMedium以上の観測が届いていれば予測は消えて
いたはずだが、届かなかった理由はフェンスではなく、mozc-shmの読み自体が21.134(500ms
ポーリング周期)まで無かったこと、かつ届いてもD1b(下記)が`Held`を返したことにある。

このフェンス自体は**ADR-191決定3の意図的設計**であり、観測プールに残る古い観測
(mozc-shm以外の源を含む)がIMEがまだキーを処理していない段階で予測を誤って覆すのを防ぐ
ためのもの。確度を問わないのは、古さの問題は確度と無関係(BUG-232がまさに「高確度でも
古い読みは危険」という例)だから。**F1(優先順位の入れ替え)は却下**。F2
(mozc-shm Highだけフェンスをバイパス)は、本バグには効果ゼロであり、フェンスを緩めること
自体の危険(IME処理前の読みを通す)だけが残るため**候補から除外**。

## D1b: `decide_use`のローリングbaseline — 確定、D5への帰属もCIで確認済み

D1のフェンスを通過した観測は、その前段で`PlatformState::report_mozc_shm`
(`crates/awase-windows/src/state/platform_state.rs:2409-`)が呼ぶ`decide_use`
(ADR-268 D3、一部ADR-291 D5)のゲートを通る必要がある。belief(予測由来のtrue)と食い違う
読みに対し、`now_ms.saturating_sub(baseline_ms) >= settle_ms`を要求する。`baseline_ms`は
呼び出し元(`mozc_shm_shadow.rs:157`)が読みの瞬間に`hook::last_operation_tick_ms()`を
再計算して渡す値で、**自己注入・KeyUp・ウォッチドッグのカナリアを含む全フックコールバック**
(`hook.rs:60-66`のdoc)で更新され続けるローリング値。

**位相による説明(実測と整合)**: `TIMER_MOZC_SHM`は読むたびに500msで張り直される
(`mozc_shm_ensure_timer`)。20.634(フォーカス直後の読み)を起点に、読みは概ね21.134・
21.634・22.134・22.634・23.134に発生したはずで、それぞれの直前の操作は:

- 21.134: 直前の`ko`/`re`送信から170ms未満 → Held
- 21.634: `,`送信(21.526)から108ms未満 → Held
- 22.134・22.634: Backspace連打の最中 → Held
- 23.134: 最後のBackspace(22.617)から170ms以上静止 → Use。実測23.142との差8ms

**ADR-291 D5への帰属はCIで確認済み(2026-11-12、GitHub Actions run `43220508025`、job
`measure (tickchurn, -Closed)`)**: dragonflyg4で動いていたバイナリが`752b35ec5`であることは、
同日この直前にclipwire経由で`git log --oneline -1`を実行し確認済み(本ADR作成前にデプロイ・
動作確認済みのビルド、以後再ビルドは無い)。`752b35ec5`はD5(`c8a4393f9`)を含む
(`git merge-base --is-ancestor c8a4393f9 752b35ec5`で確認済み)。D5が変えたのは
「fast path(`update_tick_ms > baseline_ms`)がbelief食い違い時にも無条件に通ってしまう」
部分のみで、「`settle_ms`無条件待ち」という経路自体(D5以前から存在するR3'のもう一方の
OR分岐)は変更していない——したがってD5の関与は「Mozcが閉のままでも打鍵で`update_tick_ms`
が進むか」で判定できる(進むなら、D5以前はfast pathで21.134付近に即座に`Use`されていた
はずで、D5がこの遅延を延ばしたことになる)。

既存の`tickchurn`シナリオ(`win.set_open(1)`で開いた状態)に`--closed`変種
(`win.set_open(0)`、`tools/mozc_shm_ci/shmci.py::cmd_tickchurn`)を追加し実機で確認した結果:
**`tsf_open=0`(閉)のまま、打鍵(A/B/C)で`update_tick_ms`が134750→138343と進んだ**
(idle 3秒間は無反応=`update_tick_ms`自体は無関係なタイマーで進むわけではないことも確認済み)。
これはD5が変えたfast pathが「進む」側の分岐であり、**D5はBUG-233の遅延を実際に延ばしていた**
——帰属はADR-268 D3 R3'単独ではなく、D5(ADR-291、BUG-232の修正)にもある。

**この発見の意味**: 訂正を遅らせているのは**ユーザー自身が誤りに気づいて打つBackspace**
という悪循環。Backspaceを打つたびにbaselineが押し戻され、訂正がさらに遅れる。

## D2: ADR-270安全網`should_check_compose`の前提条件 — 確定、ただしH1は機能しない

`crates/awase-ime-core/src/state/mozc_shm.rs:688-699`(`should_check_compose`)は、送信直前の
生のmozc-shmが`effective_open==true`であることを前提に確認を張る。予測だけで開いたと
信じている間(実体は`effective_open=false`)はこの前提が成立せず安全網が張られない。

ただし、この前提条件だけを外す案(H1)は**機能しない**。同ファイルの`judge_compose`
(735-753行)自体が`!b.active || !b.effective_open`の場合、即座に`ComposeVerdict::
Skip(ComposeSkip::ImeNotActive)`を返す設計になっている。つまり`should_check_compose`を
通しても、確認のtick自体が「IME非活性」と判定して何もしないまま終わる。さらに、仮に
`judge_compose`も変えてLiteralと判定できたとしても、回収は「Backspace×n+同じローマ字の
再送」(`probe_io.rs::set_raw_literal`)であり、**IMEが閉のままなら再送もまたリテラルになる
だけ**(2回目は`consecutive`カウンタで諦めて素通しに切り替わる)。送ってから回収する設計
そのものが、IMEが閉のままという状況には噛み合わない。**H1は却下**。

加えて、確認が張られるのは`dispatch_probe_actions`の`ProbeAction::Transmit`(cold経路)
だけであり、BUG-233の`ko`がこの経路を通ったか(2文字目以降の`re`・`,`はwarm送信なので
対象外になる)は確認できていない。

## 修正候補のまとめ(G3・Pとも採用・実装済み、round5時点)

| 案 | 内容 | 判定 |
| --- | --- | --- |
| F1 | 観測を予測より常に先に評価する | **却下**(ADR-191決定3の意図的設計を破壊、BUG-232型の別の退行) |
| F2 | mozc-shm Highだけフェンスをバイパス | **除外**(BUG-233には効果ゼロ、柵を緩める危険だけ残る) |
| F3 | mozc-readable使用時はKeyEffectPrediction自体を使わない | **Pに統合**(単独では「遅延」ではなく「素通しの誤入力」を増やす) |
| G1 | belief食い違いの根拠を`decide_use`に伝え、予測由来の食い違いを即時`Use`にする | **却下**(予測が正しくMozcが開きかけている最中に閉と早合点する、BUG-232と向きが逆の同型の退行) |
| G3(採用・実装済み、S1込み) | `decide_use`のbaseline(食い違う読み用)を、実際のIME書き込み送信時刻を含む形にする | 詳細は下記 |
| H1 | `should_check_compose`の前提を外す | **却下**(`judge_compose`の既存ロジックで機能しない、回収も閉のIMEへの再送を繰り返すだけ) |
| P(採用・実装済み、第3のhold) | 予測が未照合の間、Engineに入る前の**物理キー**を保留する | 詳細は下記。**round5: 既知の限界あり(Mozcがsettle_ms〜保持期限の間に開く場面、下記「決定」節参照)** |

### G3の詳細(round2で訂正: `last_mode_key_tick_ms`だけでは不十分)

round1では`decide_use`のbaseline(食い違う読み用)を`last_mode_key_tick_ms`
(`UserImeSetIntent`/`ImeApplyRequested`/`KeyEffectPredicted`/`ModeKeyPassedThrough`で
進み`FocusChanged`で消える)に差し替える案を提示したが、round2レビューで**この形では
BUG-232が再発する**ことが判明した。

- `last_mode_key_tick_ms`が進むのは`ImeApplyRequested`の**dispatch時点**であり、
  これは「IME書き込みを要求した時刻」でしかない。実際の書き込みは
  executorのキュー・`run_open_chain_async`のgateポーリング・`fallback_write`の待ちを
  通ってから行われ、数十〜数百ms遅れる(Mozc系は`GjiDirect`、`ime_controller.rs::
  apply_mechanism`→`ime::send_ime_mode_key`がSendInputで実際に注入する時刻)。
- 今のローリングbaseline(`hook_alive_tick_ms`)は、この**実際の注入**をフック経由で
  (自己注入キーもフックを通るため)偶然拾っている。G3でbaselineを`last_mode_key_tick_ms`
  だけにすると、この保護が消える: 明示OFFの意図が記録された(T0)後、実際の注入が
  T0+Xに遅れ(cold GJIのFキーは実測~750msの前例あり、`experiment-logging.md`の
  `098c663`)、X>170msの場合、T0から170ms経過した時点でMozcがまだ注入キーを処理して
  いない古い`open=true`の読みが`Use`になってしまう。**この退行は、baseline=明示OFFの
  時刻を使う既存の単体テストでは検出できない**(単体テストでは注入の遅延そのものを
  モデル化していないため)。

**修正(S1、実装済み)**: `note_imm_cross_write`(`hook.rs:339`付近、関数定義は340行)と
同種の、実際のIME書き込み注入の直後に打刻する`note_ime_actuation_send`を新設し
(`crate::ime::send_ime_mode_key`・`send_ime_mode_key_with_shift_release_prefix`の
`SendInput`直後、送信数が足りなくても打刻する——起点が後ろへずれるだけで安全側)、
baselineを`max(last_mode_key_tick_ms, imm_cross_write_tick_ms,
ime_actuation_send_tick_ms)`にした(実装は`report_mozc_shm`内、下記「決定」節参照)。

打刻の洗い出し(round5時点の結果): `ime_controller.rs`経由の`GjiDirect`・`MsImeDirect`
(モードキー送信)、`output/mod.rs`のADR-245半角英数トグル注入、既存の`open_chain.rs`の
ImmCrossはカバーした。**漏れが残っている(round5で判明、未修正)**: `key_pipeline.rs`の
shift-conv-guardが`VK_DBE_HIRAGANA`を`send_input_safe`で直接注入する経路
(`effective_open()`が真のときに走る——belief が誤って開を信じているBUG-233のような
場面でこそ発火しうる)に打刻が無い。ADR-255(Space置換)を「打刻不要」とする根拠の
明記も未了。いずれもS1の残作業として記録する(下記「未解決として残るもの」)。
**round3の訂正(T5、小)**: `ModeKeyPassedThrough`(素通しの物理キー)は
フックを通る時刻とIMEに届く時刻がほぼ同じなため、現状の`last_mode_key_tick_ms`で足りる
——打刻が必要なのは「awaseが`SendInput`でIME開閉/モードを動かすキーを**注入**した時刻」
と「`focus_switch_hold`/`resume_hold`/P自身で**遅延再生**したモードキーを実際に流した
時刻」(下記Pの詳細T2参照)の2種類に整理すると漏れにくい。

**計算場所(S2)**: `architecture_guard.rs`の`mozc_shm_reader_touches_belief_only_through_
report_mozc_shm`が、殻(`mozc_shm_shadow.rs`)に`KeyEffect`/`ImeEvent`等の語を書くことを
禁じているため、`last_mode_key_tick_ms`を使うG3側の起点合成は**belief側
(`PlatformState::report_mozc_shm`)の中で行う**必要がある。殻からはS1の打刻
(hookのAtomic)だけを渡す。`decide_use`は「一致する読み用のbaseline(現状のまま)」と
「食い違う読み用のbaseline(G3)」の2つを受け取る形になる。`last_mode_key_tick_ms`が
`decide_use`の`now_ms`/`update_tick_ms`(`GetTickCount64`)と同じ時計かも確認する。

**`FocusChanged`直後の規則(S2')**: `last_mode_key_tick_ms`は`FocusChanged`で`None`に
なる。この場合の起点は現状の`last_operation_tick_ms`(安全側)に戻す。

### Pの詳細(round2で再設計: 送信前の照合ではなく「物理キーの保留」)

round1では「送信直前にmozc_compose_precheckを使って照合する」としたが、round2レビューで
**この前提(既存の仕組みをそのまま使える)が誤り**と判明した。`mozc_compose_precheck`が
呼ばれるのはcold probe経路の`ProbeAction::Transmit`1か所のみで、warm送信には送信前に
読む場所が無い(BUG-233の`ko`がcold経路を通ったかさえ未確認)。さらに、ローマ字は
Engineがbelief=開を前提に変換した**結果**であり、保留後にR6で「開かなかった」と判明
した場合、ローマ字を元の物理キーへ戻す手段が無い。

**再設計(S3〜S6)**: Pは「送信前の照合」ではなく、**予測が未照合の間、Engineに入る前の
物理キーを保留する第3のhold**として定義する。既存の`focus_switch_hold`(PR #809 M1)・
`resume_hold`(ADR-245追補2 R2)と同じ形で、取り込み口で保留し、解放時に
`INPUT_DEFER.replay_later`でEngineへ流し直す(解放時のbeliefに従って正しく変換される)。
runtime層(`key_pipeline.rs`相当)で判断する——`output/`は層境界により`platform_state`を
直接読めないため。

- **解放条件(S5、round3でT1反映し訂正)**: 「開いた(一致する読み)→解放して送る」
  「起点からsettle経過後に閉→R6の行動」「それ以外→保留継続」の3値にする。**round3
  レビューで、この「一致する読み」の判定に現状のローリングbaseline
  (`hook_alive_tick_ms`)を使うと重大な欠陥が出ることが判明した(T1)**: `hook_callback`
  冒頭の`tick_hook_alive()`(`hook.rs`)は、Pが保留(抑止)した物理キーも含めた**全ての
  コールバック**でbaselineを進める。保留中はMozcへキーが届かないため、Mozcのブロックは
  開いた瞬間に1回更新された後は`update_tick_ms`が進まなくなる。利用者が打ち続けると
  baselineが`update_tick_ms`を追い越し、高速路(`update_tick_ms > baseline_ms`)が
  通らなくなり、「最後の打鍵から170ms静止」でしか解放されなくなる——**D1bの悪循環が、
  一致する方向で、しかも予測が正しくMozcが正常に開いた毎回の入力で再発する**。
  **修正**: 解放判定の「一致する読み」の起点は、ローリングbaselineではなく
  **`pred.at_ms`(予測の時刻)とS1の実際の送信打刻の大きい方**を使う(開と読めて
  `update_tick_ms`がモードキーの後なら、モードキー後にMozcが書いた読みと言える。
  `update_tick_ms`が他の理由で進む〈D5が指摘した限界〉はここでは無害——belief〈予測=
  開〉と一致する向きの読み違いは結論を変えない)。**その予測が照合済みになるまでの間**、
  `decide_use`に渡すどちらのbaselineにも保留中の打鍵を含めてはならない(round4で
  範囲を「予測が照合されるまで」に広げた)。**round5の訂正**: 実装がこの非ローリング
  baselineを使うのは保持専用の短周期の読み(`mozc_shm_read_for_hold`)だけで、refreshの
  通常tick(`mozc_shm_on_tick`)や解放後の通常の読みはローリングbaselineのままなので、
  U2を閉じているのは「予測ごとに1回」(`last_held_pred`)の1つだけであり「二重に閉じる」
  は誤り。害は小さい(一致する向きなので遅れるのは`[key-effect-confirmed]`ログと予測の
  解除だけ)。Linux単体テストの表で「保留中の打鍵でローリングbaselineが進んでも、この
  新しい起点なら解放されること」を固定する。
  **round4の訂正(U2、重大)**: Pを**予測ごとに1回**と定義する。モードキーから170ms未満
  でも「開」と読めて解放することがあるため(上記の起点修正の効果そのもの)、D1の柵
  (`reconcile_key_effect_open`が`pred.at_ms`から`KEY_EFFECT_SETTLE_MS`経過するまで
  予測を消さない)より早く解放が起きうる。この場合、解放後も開始条件(c)「予測の開が
  未照合」が真のまま残り、次の打鍵でまたPが始まり、また読んで、また解放する——
  モードキーから170ms以上経って照合されるまで毎打鍵が16〜30msずつ保留される。
  条件(c)は「予測の開が未照合**かつ**その予測(`pred.at_ms`で識別)についてPがまだ
  『開』で解放していない」とする。一度「開」で解放した予測には、照合が済むまでPを
  掛け直さない。モードキーを押し直して新しい予測ができたら、また対象になる。
- **保留中にモードキーを再度押した場合(T2、新規)**: 利用者が無反応に気づいて変換キーを
  もう一度押す可能性が高い(BUG-233でも実際にBackspaceを6回打っている)。方針:
  **保留する**(通すと保留中の文字キーを追い越してIMEに届き、順序が逆転し、かつMozcの
  keymapで変換キーが合成中は別の意味になりうる)。保留したモードキーの`KeyEffectPredicted`
  発火とS1の打刻は、**取り込みの時点ではなく実際に流した(再生した)時点**で行う
  (取り込み時点にすると、実際にIMEへ届く時刻より前の打刻になり、S1と同型のずれを生む)。
  一般に、`focus_switch_hold`・`resume_hold`で遅延再生されるモードキーも同じ規則にする。
- **保留中の読みの周期(S5)**: 500msのタイマー任せでは保留が最大500ms以上になる。保留中
  だけ短い周期(16〜30ms、ADR-270確認FSMと同程度)で読む。
- **報告経路(S6)**: mozc-shmの読みをbeliefに入れる経路は`report_mozc_shm`の1つに限る
  (既存のguardとADR-268)。Pの読みもこれを通し、Held/Use判定と`[key-effect-miss]`照合を
  同じ規則に揃える。
- **逆向きの対称性(S6')**: 予測が「閉」でMozcが実は開いたままの場合も症状は同型(逆向き)
  だが、Engine非活性の打鍵はローマ字送信を通らないため「送信前」のPでは捕まえられない。
  物理キーの取り込み口で保留する今回の再設計なら、この向きも同じ仕組みで扱える——対象に
  含めるかは要決定。
- **フォーカス変更時の扱い(T3、実装済み)**: お手本にしている既存のhold(`resume_hold.rs::
  release`)は「前面窓が保持開始時と違えば保留分を捨てる」仕様で、これは本ADRがR6(ii)で
  却下した「無断で文字が消える」(BUG-074型)に当たる。**実装の決定**: 「捨てる」を選び、
  `tracing::warn!`でログに残す(実装は`key_effect_open_hold_finish`の`Released::Discard`)。
  **round5の訂正**: ログだけなので利用者はawase.logを見なければ気づかず、R6(ii)で避けた
  「無断で文字が消える」から実質的には抜け出せていない——利用者へのUI通知(バルーン等)は
  未実装のまま残る(下記「未解決として残るもの」)。上限(`KEY_EFFECT_OPEN_HOLD_CAP=8`、
  `FOCUS_SWITCH_HOLD_CAP`と同値)に達したときのbeliefは「変えずに流す」(`ReleaseReason::
  Cap`、`needs_notice()`は偽でログもinfoのみ)に決定済み。**round5 followupの訂正**:
  `Cap`は下記「決定」節の既知の限界(Mozcが遅れて開く場面での誤変換かな)とは**逆側**の
  限界を持つ(belief=開のまま再生するため遅れて開く場面はむしろ正しく入り、本当に
  閉のまま〈BUG-233そのもの〉の場面でPなしと同じリテラルになる、`Deadline`と同じ側)。
- **開始条件・期限・タイマー定数(T4、新規)**: Pを始めるのは次が**全て**揃ったときだけ:
  (a) `gji_evidence_absent`、(b) 前面窓のスレッドに読めるmozc-shmブロックがある
  (直前の読みが`Outcome::Accepted`かつ`active`、詳細はU5)、(c) 予測の開が未照合
  (U2の「予測ごとに1回」の定義込み)。**round4の訂正(U1、重大)**: 保留中の読みが
  「読めない(`Unreadable`・`Rejected`)または TIP が無効(`active == 0`)」のときだけ
  即座に解放し現状の挙動(Pなし)に戻す。**`judge_compose`の`ComposeSkip::ImeNotActive`
  (`!b.active || !b.effective_open`、`effective_open==false`=閉の読みを含む)を流用
  してはならない**——流用すると、BUG-233のように最初の読みが単に「閉」だった場合に
  即座に解放し、予測の開のまま流すことになってBUG-233がそのまま再現する。「閉」の読みは
  即座解放の対象ではなく、上記の3値判定(起点からsettle経過後に閉と確定→R6の行動、
  それまでは保留継続)に入れる。P専用の純粋関数で判定し、`judge_compose`の分類は
  流用しない。新設する
  定数(周期16〜30ms、保留の期限)は[tuning-constants.md](../../.claude/rules/tuning-constants.md)
  の実測義務の対象になる(Mozcがcold/warmで開くまでの実測はADR-270/284の
  `[mozc-compose]`ログに既にある可能性があり、まず確認する)。**正常時の遅延増分**
  (予測が正しい場合でも、モードキー直後の最初の文字がMozcの開読み取得まで毎回待たされる
  増分、warm/cold別)をCI確証計画の測定項目に加える(読みのコスト`read_us`だけでは
  不十分)。
- **defer/replayファミリーへの登録(S5)**: `fix-requires-evidence.md`の「defer/replay
  キューの解放条件」ファミリーに入る。フォーカス変更・panic・設定再読み込み・期限・
  Engine非活性化の全ての解放窓口への配線、既存2つのholdとの同時保留時の順序は実装済み
  (`hold_on_delivery`は窓切替直後→戻り待ち→Pの順)。**round5で判明、本コミットで対応
  済み**: `fix-requires-evidence.md`の表と`.githooks/pre-push`の正規表現に
  `key_effect_open_hold`が登録されていなかった(純粋側はたまたま`state/key_effect_`
  接頭辞に一致して拾われているだけだった)——BUG-116と同型の「判断の殻が自動チェックの
  外にある」穴だったため、両方に追加登録した。「Engine非活性化」の解放窓口は専用配線が
  無く、次の短周期タイマー(20ms以内)で`read=None`→`Unreadable`として拾われる形に
  留まる。
- **round4で実装PRへ持ち越した小項目(U3〜U5)**: (U3)再生したモードキーがT2の規則どおり
  「再生の時点」で予測されるかは、`focus_switch_hold`(既にモードキーを保持対象にしている
  `focus_switch_hold.rs:19,56`)の実経路(`enrich_key_role`→`apply_key_effect_prediction`
  を通るか、自己注入として予測から除外されないか)を実装時に確認し1行記録する(推測で
  「同じ規則」と書かない)。(U4)R6の(i)開き直しに成功した後、保留中にあった2回目の
  変換キーをそのまま流すと、Mozcが既にPrecomposition状態のため再変換の意味になりうる
  ——「(i)が成功したら保留中の『開』を予測するモードキーは流さずに消費する(ログに残す、
  意図は既に果たされているのでBUG-074型の消失には当たらない)」規則をPとR6(i)の接点として
  実装時に書く。(U5)条件(b)の判定述語は`mozc_shm.polling`(`Rejected`等でも真になる、
  `mozc_shm.rs:495-497`)ではなく、直前の読みが`Outcome::Accepted`かつ`active`だったこと
  で判定する(U1を直せば実害は無いが、無駄なP開始を避けるため)。

## R6: 観測が「閉」と確定したとき、何をするか(決定済み: (iv)をデフォルト実装、(i)は未実装)

**(iv)をデフォルトとして実装済み。(i)「開き直す」は原因特定〈CI確証計画項目1〉待ちで
未実装。** G3は「beliefを閉へ早く/正しく戻す」効果しかない。
belief が閉に戻ると、Engineは非活性化し、NICOLAのローマ字変換は停止してqwertyの生文字が
そのまま素通しになる。つまり`kore,`という症状は別の症状に置き換わるだけで、BUG-233本来の
問題——**ユーザーは開くつもりでモードキーを押したのに、実際には開かなかった**——は
解決しない。

選択肢(round2で訂正・追加):

1. **(i) 開き直す**: モードキーを「開きたい」という意図として扱い、観測で閉と確定したら
   awase自身が開く書き込みをやり直す。**round2で判明した4つの未検討リスク(S8)**:
   - **機構**: モードキーがZedに届かなかったのが原因なら、awaseの注入キー(GjiDirect)も
     同じ窓に同じ理由で届かない可能性が高い(ImmCrossは`Imm32Unavailable`で使えない)。
     原因特定(下記CI計画1)が前提。効く機構が無ければ(i)は空の選択肢になる。
   - **冪等性**: Mozcが単に遅れて(例:600ms後に)開いた場合、awaseの開き直しと重なる。
     送るキーがトグル(VK_KANJI等)だと閉じてしまうため、冪等な「開く」専用キーに限る。
   - **合流点の規約**: 開き直しはIMEへの新しい書き込み入口になるため、ADR-119の合流点
     (`claim_press_write`/`with_press`/`explicit_press_applied_pair`、
     `RESTRICTED_CALLS`、`architecture_guard`の`press_id_is_claimed_and_carried_at_
     every_order_issuing_entry`)に従う必要がある。押下IDを素通ししたモードキーのものに
     するか、drift correctionと同じ`press=None`にするかを決める。
   - **IntentStoreとの順序**: `apply_key_effect_prediction`は`intent_store.remove`する
     (`platform_state.rs:599-603`)。`report_mozc_shm`が食い違い時に呼ぶ
     `pass_through_observed(true, true)`は`desired_open`を観測(閉)に揃える。開き直しの
     判断はこの追随より**前**に行わないと、開く意図の根拠が消える。
2. **(ii) 保留して期限切れを待つ**: 開いたと確かめるまで物理キーを保留(P)し、期限が
   来たら(i)に進むか(iv)へ移る。**round2の訂正(S7)**: 「保留分を捨てる」は不可——
   リテラルは目に見えて打ち直せるが、保留分を黙って捨てると利用者が気づかないまま文字が
   消える(BUG-074「give-up で文字消失」と同型)。
3. **(iii) 何もしない**: beliefを閉に戻すだけ。「BUG-233自体は直らず、訂正を早くするだけ」
   と明記する必要がある。
4. **(iv) 新設(S8')**: belief を閉に直し、保留分(物理キー)をEngineへ流し直し
   (閉のbeliefで変換される=qwerty素通し)、利用者へバルーン/インジケーターで知らせる。
   開き直しの機構が無い窓での最善の可能性が高い。[[feedback_user_intent_over_belief_2026_10_23]]
   の「バルーンは残す」方針と合致する。**round3の補足(T6、小)**: Zedのように**毎回**
   モードキーが効かない窓では(iv)のバルーンが毎回出て邪魔になる。窓クラスごとに
   「この窓ではモードキー予測が外れる」を学習し(`Imm32Unavailable`への降格学習と同型)、
   学習済みの窓では最初から予測でbeliefを開にせずPの保留だけを使う・通知も初回のみに
   絞る、という扱いを検討する(原因特定〈CI確証計画1〉で「アプリ固有」と判明した場合に
   効く)。

プロジェクトの既存方針([[feedback_user_intent_over_belief_2026_10_23]]: beliefと実IMEの
ずれが直らない窓では、ユーザーがやりたいことをbeliefより優先する)を踏まえ、**(iv)を
デフォルトとして実装した**(下記「決定」節)。(i)「開き直す」を追加で実装するかは
「なぜモードキーがZedでMozcを開けなかったか」の特定(CI確証計画項目1)が前提のため、
まだ判断できない。

## 決定

**設計はOpusレビューround4で収束(2026-11-12)。G3・Pの実装も完了し[PR #936]
(https://github.com/cuzic/awase/pull/936)として**developへマージ済み**(`6439bc7d2`、
2026-11-12T14:15:43Z)。** G3・R6はround3で、Pはround4(U1・U2反映後)で収束と判定された。
CI確証計画の項目2(閉のMozcでも`update_tick_ms`が進むか)は実施済み(上記D1b参照)。
マージ後のround5 Opusレビューで、Pの既知の限界(下記)を含む検証を行った。

**実装(マージ済みブランチ`fix/adr292-key-effect-prediction-hold`、merge commit
`6439bc7d2`。PRの件名・本文はG3のみ言及しているが、実際の差分にはP関連ファイル
〈19ファイルの大半〉も含む——round5で判明、git log/grepでこのコミットを探す際は
件名だけに頼らないこと):**

- **G3**: `decide_use`に食い違う読み専用のbaseline(`disagree_baseline_ms`)を追加し、
  `report_mozc_shm`側で`ImeModel::last_mode_key_tick_ms`と新設の
  `hook::imm_cross_write_tick_ms`・`hook::ime_actuation_send_tick_ms`(awase自身が
  SendInputでIMEの開閉/モードキーを実際に注入した時刻、`send_ime_mode_key`系の送信
  直後に打刻。送信数が送るべき数に足りなくても打刻する、起点が後ろへずれるだけで
  安全側)の3つの最大値として合成する(`last_mode_key_tick_ms`が`None`〈`FocusChanged`
  直後〉のときはS2'によりローリングbaseline`baseline_ms`へ戻る)。
- **P**: `KeyEffectOpenHold<T>`(`crates/awase-hook-core/src/state/key_effect_open_hold.rs`、
  `focus_switch_hold`/`half_width_resume_hold`と同じ形の「第3のhold」)を新設し、
  `key_pipeline.rs`へ配線。モードキー予測で開を信じた直後(自動リピートのKeyDownからも、
  `/code-review high`指摘1を反映)次の打鍵から保持し、専用の20ms周期タイマーで
  `decide_use`(保持専用の非ローリングbaseline`hold_baseline_ms`、round3 T1を反映)を
  呼んで解放判定する。開いたと確認できたら再生、閉で確定したらbeliefを閉に直し
  `notify_engine_refresh`を呼んでから再生(R6(iv)、`tracing::warn!`のログのみ)、
  読めなくなったら即座解放。解放理由は全9種類(`ConfirmedOpen`・`ObservedClosed`・
  `Unreadable`・`PredictionSettled`・`Deadline`・`Cap`・`FocusChanged`・`PanicReset`・
  `ConfigReload`、`needs_notice()`は`ObservedClosed`/`Deadline`のみ真)。武装には期限
  (`KEY_EFFECT_OPEN_HOLD_ARM_TTL_MS=700ms`)があり、期限切れで来た打鍵では保持を
  始めずPなしの挙動に戻る。round3 T2(保持中のモードキー再押下)・T3(フォーカス変更時は
  Discard+警告ログ、UI通知は未実装——下記「未解決として残るもの」参照)・round4 U1/U2
  (即座解放条件・予測ごとに1回)を反映済み。
- **レビュー**: `/code-review high`で実装全体を検証、重大な指摘4件
  (armed状態の無期限待機・cap未適用経路・ime-off-rescue除外の実態不一致・タイマー
  二重起動)を修正し再レビューで指摘ゼロ。Linux単体テスト(hook-core 167件・
  ime-core 775件・architecture_guard 176件・layer_boundary_guard 17件・
  awase-windows lib 339件)、windows target check/clippy/fmtすべてグリーン。

### Pの既知の限界(round5、マージ後の最終Opusレビューで発見・round5追確認で機序を訂正)

**[高]Pは「Mozcがモードキーの処理からtsf_openの共有メモリ公開まで、settle_ms(170ms)+
読みの周期(≤20ms)より遅れる」場面で、正しい出力を誤った出力に変える。** 機序(round5
追確認〈followup〉で訂正):

1. **Pが無ければ正しく入る理由は「送信がまだ届いていないから」ではない**。モードキー
   (VK_CONVERT)とそれに続くローマ字キーは、同じ対象スレッドの入力キューへ**順番に**
   送られる。Mozc(TIP)の処理自体が遅れても(cold起動でスレッドが混んでいる等)、
   TIPはキューの順でまずモードキーを処理して開いてから後続のローマ字を処理するため、
   内部的には正しく合成される。共有メモリの`tsf_open`はこの内部処理の**結果の公開**
   でしかなく、公開(flush)が遅れているだけで内部状態はもう開いている、という
   ずれが起こり得る。
2. 保持中、短周期タイマーの読みが保持専用baseline(`pred.at_ms`とS1打刻の大きい方)から
   `KEY_EFFECT_SETTLE_MS`(170ms)+読みの周期経過した時点で、公開がまだ追いついて
   おらず`tsf_open=0`のままなら、`decide_use`が`Use{open:false, disagrees:true}`を
   返す。これが`report_mozc_shm`経由でbeliefを閉へ確定させる(ADR-191決定3の既存の
   `reconcile_key_effect_open`。この段階自体は害が無い——次の読みで開が見えれば
   belief はまた戻る)。
3. **取り消せない害を生むのは、この後にPの`judge()`がそれを`ReleaseReason::
   ObservedClosed`として、その場で即座に保持を解放・再生すること**。保持していた
   物理キーが、確定した閉のbeliefの下でqwerty生文字(NICOLA変換を経ない直接入力)
   として**その時点で**`INPUT_DEFER.replay_later`を通じて送信される。
4. その直後(あるいはさらに後、何ms後であっても上限は無い)にMozcの公開が追いつき
   実際に開いても、**すでに送信済みの生文字キー列が開いたMozcの手前(入力キューの
   モードキーの後ろ)に並ぶ**ため、Mozcはこれをローマ字として直接読み、NICOLAの
   変換を経ていない誤ったかなに変換してしまう。**症状はBUG-233の`kore,`型の
   リテラル(IMEが閉のまま生文字が出る)ではなく、逆に「開いたMozcが生のqwertyを
   誤ってローマ字と解釈した結果の誤変換かな」になる**。

**範囲**: `Split`(`tsf_open != effective_open`)が続く窓や、保持中に食い違いの起点が
drift correction等で前へ押される場合は、むしろ`judge()`が`Keep`→`Deadline`に進み
belief を変えずに(予測の開のまま)再生するため、この限界は**起きない**(round5
followupで訂正)。この限界が起きる条件は1つだけ:**cold Mozcがモードキーを処理して
`tsf_open`を公開するまでの遅れが、settle_ms+読みの周期を超えること**(Split・起点の
押し戻しは不要)。発生後の影響に300msの上限は無い——`ObservedClosed`は判定した
瞬間(約170〜190ms)に即座に解放されるため、Mozcがその後いつ公開しても遅い。さらに
belief が閉に追随したことで、Mozcが実際に開いた後の「開」の読みも`decide_use`の
settleをもう一度要求するため追加で遅れ、保持の解放で`TIMER_MOZC_SHM`を張り直す
(次の通常の読みは最大500ms後)ので、その間の後続の打鍵もqwerty素通しになり得る。

**ADR-191決定3の`reconcile_key_effect_open`自体のせいではない**(round5 followupで
訂正)。belief が閉に確定すること自体は無害——取り消せない害は、Pの`judge()`がその
belief確定を根拠に**即座に**再生する判断にある。打鍵がどのbeliefで変換されるかは
再生**時点**のbeliefで決まるため、`KEY_EFFECT_SETTLE_MS`を動かさずとも、P専用の
判断(例: 予測から別途の待ち時間が経つまで再生しない、または保持の読みの食い違いの
起点をP専用にしてbeliefの追随自体を遅らせる)で対策できる余地がある。

**是正しない理由**: 本当の二律背反は、P内部の「閉の確定(=再生)を早めるか遅らせるか」
にある——早めると上記の誤変換リスクを負う一方、遅らせると本当に閉のまま(BUG-233
そのもの)の場面で保持が延び、リテラル露出の時間が伸びる。どちらに寄せるべきかは、
cold Mozcでモードキーから`tsf_open`公開までの実際の遅延分布を測ってから決める
べきで(`tuning-constants.md`の実測なしのエスカレーション禁止)、今回は実測
(CI確証計画項目5)を待って据え置く。

**未解決として残るもの(実装のTODO)**:
- 上記Pの既知の限界(cold Mozcのtsf_open公開がsettle_ms+読みの周期より遅れる場面での
  誤変換かな)。**`Cap`による解放は同じ限界を持たない**(round5 followupで訂正)——
  `Cap`はbeliefを変えずに(予測の開のまま)再生するため、遅れて開く場面ではむしろ
  正しく入る。`Cap`が持つ限界は`Deadline`と同じ側(本当に閉のまま〈BUG-233そのもの〉
  の場面で、Pが無い場合と同じリテラルになる)。
- 保持の期限(`KEY_EFFECT_OPEN_HOLD_MAX_MS=300`)に達しても読みが`Held`/`Split`のままの
  場合、観測の裏付けなしにbeliefを閉にする正当な入口が無い(`ime-belief-architecture.md`
  が禁止するヒューリスティックな書き込みに当たるため)ため、beliefを変えず再生+warnログ
  (P導入前と同じ出力、ただし保持により最大300ms遅延する・解放後`TIMER_MOZC_SHM`を
  張り直すため次の通常訂正も最大500ms遅れる)。
- 新設タイミング定数3つ(`KEY_EFFECT_OPEN_HOLD_POLL_MS=20`・
  `KEY_EFFECT_OPEN_HOLD_ARM_TTL_MS=700`・`KEY_EFFECT_OPEN_HOLD_MAX_MS=300`)は
  いずれも`pending=true`(導出は既存値の組み立てで実測ではない。`MAX_MS=300`は
  settle_ms+周期から見込む確定までの時間〈約190ms〉に、利用者が待ってもよいと
  言った上限「0.3秒程度」〈2026-10-19、`MOZC_COMPOSE_CHECK_MS`と同根拠〉を当てた値
  〈`tuning.rs`のdocコメント参照〉。実機実測は未実施、CI確証計画項目5で測る)。
- T3: フォーカス変更時の破棄は`tracing::warn!`ログのみで利用者へのUI通知は未実装
  (上記R6節参照)。
- S1: shift-conv-guardの`VK_DBE_HIRAGANA`直接注入(`key_pipeline.rs`)に打刻が無い
  (上記「G3の詳細」節参照)。
- R6の(i)「開き直し」は未実装(原因特定〈CI確証計画項目1〉待ち)。
- S6'(予測が閉で実は開いている逆向きのケース)は対象外(`arm`は`open==Some(true)`の
  ときのみ)。
- 保持中にdrift correctionが発火し食い違いの起点を押す相互作用は未確認。

**本コミットで対応済み**: S5(`fix-requires-evidence.md`・`.githooks/pre-push`への
`key_effect_open_hold`登録)、ADR-268・ADR-291本文へのG3によるbaseline変更への
言及(下記「関連ADRとの関係」参照)。

残りのCI確証計画(項目1・3・4・5、下記)・実機確認(dragonflyg4、現在オフライン)・
**本ADRドキュメントブランチ自体のdevelopへのマージ**は、実装のPR(#936、マージ済み)
とは別に進める。

## 未解決・CI確証計画(round2で拡充)

**CIより先に、既存の報告データを確認する(S9)**: `[diagnostics] mozc_shm_shadow=true`が
有効なら、20.634と23.134の読みのmozc-shm v5ブロック(`key_seq`)がawase.log/journalに
残っている可能性がある。両者で`key_seq`が進んでいればTIPはキーを見ていた(開かなかった
原因は別)。進んでいなければ、モードキーも`ko`/`re`もTIPに届いていない=Zedでその窓自体が
TIPへ入力を渡していない可能性が高く、「フォーカス直後のcold」という前提が崩れる。CIを
組む前に、まずこのログ確認を行う。

1. **モードキーがZedでMozcを開けなかった理由の特定**(R6依存、CIより上記ログ確認を先に):
   フォーカス直後でない状態でもVK_CONVERTがMozcを開くか、`key_seq`が進むか。Zed(Windows
   版)がCIで使えない場合はその旨を明記し、前提を別シナリオに置き換える。
2. **【完了、2026-11-12】閉のMozcに打鍵を送った場合、`update_tick_ms`が進むか**:
   `tickchurn --closed`シナリオ(既存`tickchurn`のIME OFF変種、GitHub Actions run
   `43220508025`)で確認済み。**進む**(134750→138343)。D1bのD5帰属を確定した(上記参照)。
3. **D1bの位相説明の直接確認**: `mozc_shm_shadow=true`で各ポーリングのHeld/Use判定と
   `baseline_ms`を記録し、位相による説明(23.134でUse)を実測で裏取りする。
4. **修正後の退行確認**:
   - 【完了】BUG-232(明示OFF→`M`→古い開の読み)がG3+S1でも引き続きHeldのままであること
     (D5の修正コミット`c8a4393f9`/develop上`97d849bfe`、新規テスト2件で確認済み)。
   - **未着手**: S1の退行確認(単体テストでは検出不可、CIシナリオが必須)。明示OFFの後、
     cold GjiDirectの送信が170ms以上遅れる状況で、古い開の読みが`Use`されないこと
     (`tickchurn`の変種に送信遅延を入れたCIシナリオを新設)。
   - **【round5の机上の検討で、起きうると判明。実測はまだ】** 予測が正しくMozcが
     settle_ms+読みの周期より遅れて`tsf_open`を公開する場合に、Pの短い周期の読みに
     よってbeliefが一時的に閉へ倒れること自体は無害(ADR-291 D5以前と同じ、次の読みで
     戻る)。問題はそれ以上に、**保持していた物理キーがその時点で即座にqwerty生文字
     として再生され、後から開いたMozcにローマ字として誤変換される**こと(上記「決定」
     節「Pの既知の限界」参照)。実際のcold Mozcの`tsf_open`公開までの時間分布を測るまで、
     この既知の限界がどの程度の頻度で起きるかは不明。
   - **未着手**: ADR-245半角英数トグル・ADR-255 Space置換・drift correctionの書き込みが
     `last_mode_key_tick_ms`/S1の打刻対象から漏れていないこと(shift-conv-guardの
     `VK_DBE_HIRAGANA`注入の漏れは判明済み、上記S1節参照)。
5. **一部完了**: 保留中にフォーカス変更・panic・設定再読み込みが来たときの保留分の扱い
   (T3、「捨てる+ログのみ」で実装済み。利用者へのUI通知は未実装)、保留中の打鍵で
   ローリングbaselineが進んでも新しい起点(`pred.at_ms`とS1打刻の大きい方)なら解放
   されること(T1、Linux単体テスト`hold_baseline_ignores_held_keystrokes`で固定済み)。
   **未着手**: **予測が正しい通常時の遅延増分**(モードキーから最初の文字が出るまで、
   Mozcのwarm/cold別、T4)、予測が未照合の間の追加の読みのコスト測定、新設タイミング
   定数3つ(`POLL_MS`・`ARM_TTL_MS`・`MAX_MS`)の実機実測。
6. **完了**: `decide_use`のG3起点選択の表(`bug233_disagree_baseline_is_not_pushed_back_
   by_physical_keys_without_ime_intent`)、Pの保留/解放判定(`judge_table`・
   `hold_baseline_ignores_held_keystrokes`)を純粋関数のテストで固定済み。**未着手**:
   `resolve_open_at`で予測照合後に観測が勝つことを確認するテストはPR #936には含まれて
   いない。
7. R6の決定は(iv)をデフォルトとして採用・実装済み(上記R6節参照)。残る判断は(i)
   「開き直し」を追加実装するかどうかで、1の原因特定を待つ。
8. **【未着手、round5で追記】実機確認**: dragonflyg4(BUG-233の実際の発生機)は現在
   オフライン(2026-11-12、`tailscale status`で確認)のため、G3+Pデプロイ後の実際の
   症状解消はまだ確認できていない。オンラインになったら、`herdr-app.exe`(Zed)での
   再現試行と、Chrome/Edge等の正常系での遅延増分の体感を確認する。見るべきログ:
   `[key-effect-open-hold] 武装`・`[key-effect-open-hold] ... 保持`・解放時の
   `tracing::info!`(理由込み)・`ObservedClosed`/`Deadline`時の`tracing::warn!`。

## 関連ADRとの関係(追補)

- **ADR-291 D2**(mozc-shmの2段ラッチが500msポーリングのみに依存する遅延、保留中)との
  関係: BUG-233の遅れの半分以上はこの周期そのもので決まっている(D1b)。ADR-291 D2の
  決定と合わせて検討する。
- **ADR-270/285/288**: H1は却下したため、`judge_compose`の`ImeNotActive`判定(ADR-270の
  意図的な「キーがアプリに届いたかIMEが受けたかを言えない」という判断)はそのまま維持
  される。
- **ADR-268/ADR-291への追記(round5で判明、本コミットで対応済み)**: `decide_use`の
  食い違う読みのsettle起点はADR-268 D3(R3')・ADR-291 D5が定義したものだが、両ADRの
  本文はG3によるこの変更(`disagree_baseline_ms`の新設)に触れていなかった。両ADRへ
  「食い違いの起点はADR-292 G3で変更(IME開閉/モード送信だけに限定)」と1行ずつ
  追記し、`related_adr`にもADR-292を追加した。
- **ADR-191決定3**: Pの保持中の読みが170ms未満でも「開」として`Use`になっても、
  `reconcile_key_effect_open`の柵(予測を消すのはsettle_ms経過後のみ)自体は変えておらず、
  「予測ごとに1回」(U2)で整合を取っている。ただし上記「Pの既知の限界」が示すとおり、
  この柵は「閉」方向の確定でも同じsettle_msで無条件に予測を消すため、Mozcが遅れて開く
  場面の保護にはならない。
