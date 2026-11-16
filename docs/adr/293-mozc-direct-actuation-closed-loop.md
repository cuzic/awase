---
id: ADR-293
title: |-
  BUG-234の恒久修正。round4で「柵(settle)の分離」が本体、direct-actuationのnonce/ackは
  GJI/TsfNativeだけが使える高速化という位置づけに訂正。Standard profileもこのPRの対象に含める
status: |-
  検討中(実装前)。round1〜3でdirect-actuation(nonce/ack)をdecide_useの観測入力として配線する
  設計まで収束していた。round4で、round3が「Standard profileは対象外、計測してから別PRで検討」
  とした結論を所有者が再検討し、Standard profileもこのPRで直す根治案を提案。round4のOpusレビュー
  で、最初の案(`ime_model.rs`の修正・`ImeKindId`でcapを選ぶ)は2点の致命的誤り(T1・T3/T4)で
  不採用と判定され、「Z(`scope_belief.rs`)の`Explicit claim`fenceと`decide_use`を、awase自身の
  送信由来かどうか(origin)で区別して直す」設計(T5)に訂正。round5のOpusレビューで、その
  origin判定自体(方向・スレッドを持たない生の打刻をそのまま使う案、イベント種別で判定する案)
  にも2点の高severity欠陥(U1・U2)が見つかり、方向・`(tid,pid)`・supersession規則を持つ単一の
  `OwnSend`事実に一般化して訂正した——round5レビューは「構造的に収束、設計レベルの指摘は
  これで無し、残るは実測のみ」と判定。round6レビューも同じ結論を再確認し、`OwnSend`の
  定義自体に残る4点の仕様精度の不備(V1: supersessionの範囲とtie-break未規定、V2: 未送信
  outcomeでもスタンプしてしまう、V3: nonce無しの送信もcap超過で「不使用」マークが立って
  しまう、V4: (tid,pid)不一致チェックの順序とZの`ε`未規定)を指摘——いずれも設計変更では
  ない文面・テスト一覧の修正として反映済み。round7で、残っていた1件の文面矛盾(W1:
  (tid,pid)不一致時は「無視するだけ」が正しいのに別の2箇所が「破棄する」と書いていた)を
  指摘・修正し、**設計・仕様とも収束(新規の設計/仕様レベルの指摘なし、残るは8節/11節に
  記載済みの実測作業のみ)と判定された**。2026-11-15、10節step1(柵の分離、origin-scoped)を
  実装し、PR #941としてOpusの敵対的コードレビュー(`opus-review-pr941`)round1を受けた。
  round1はH3件(EXPERIMENTAL`acf1a2594`を含めてPRを作っていた・drift correctionの
  `set_ime_open_ordered()==true`は「書けた」ではなく「dispatchできた」の意味だった・統合テストが
  BUG-234の状態を作っていなかった)・M多数(項目2が送信前の古い読みも消費してしまう・
  `imm_cross_write`の`(tid,pid)`をawait後の現フォーカスから取っていた・Pendingがbeliefと一致
  する読みもHeldにしていた・テストのラベルと中身の不一致・`MOZC_OPEN_CLOSE_SETTLE_MS`の導出
  根拠の精度)を指摘。**すべて反映済み**: PRをEXPERIMENTAL抜きで作り直し(`ack`/nonce+ack確認
  〈項目1〉はdirect postの送信経路が無い本PRでは実装しない形に簡素化)、項目2に
  `read_update_tick_ms > send.tick_ms`を追加、`set_ime_open`の中で実completion後にスタンプする
  よう移動、`imm_cross_write`の`(tid,pid)`を発行前に捕獲するよう修正、`report_mozc_shm`の
  `forced_held`をbeliefと食い違うときだけに限定、`MOZC_SHM_OBSERVATION_TTL_MS`を
  `MOZC_OPEN_CLOSE_SETTLE_MS`分引き上げ、BUG-234を実際に再現する統合テストに差し替え、
  `architecture_guard`に`note_own_send(`呼び出し元の走査を追加。round2レビューでround1の
  H3件はすべて解消と判定されたが、**新たにH1件(N1)**が見つかった:
  `MOZC_SHM_OBSERVATION_TTL_MS`を1000ms→3000msに引き上げた対応(round1 2-aの検証依頼への
  対応として行った)は、TTLがMozcShmの全観測(`Split`・`Rejected(Inactive)`等own_sendと無関係な
  経路も含む)に効くうえ、BUG-234では送信前の誤ったHigh観測を長く残す方向に働く不適切な変更
  だったため1000msへ戻した。M1件(N2): `forced_held`が`effective`だけを見ており、`desired`との
  食い違い(drift correctionの処理待ち)では`decide_use`の一致の高速路を通ってdesiredを元に
  戻してしまう欠陥があったため、`decide_use`の`disagrees`と同じ条件(`effective`または
  `desired`と食い違う)に揃えた。N3(項目2の`read_update_tick_ms > send.tick_ms`は必要条件で
  十分条件ではない)はADRの書き方を訂正、L1〜L5(doc精度・読み側の式の統一・ログ追加)も反映。
  N1/N2を反映したround3のレビューで収束を確認し、`opus-review-pr941`は軽微な指摘(R1〜R3、
  文面精度のみ)を反映して完了(PR #941、develop未マージ)。2026-11-15、同じブランチの先端から
  10節step2〜6(classify_own_requestのack確認復元・観測の殻・WriteMechanism::MozcDirectの
  機構追加と送信の殻・拒否時の収束)を実装し`exp/mozc-direct-actuation-step2`へpush。
  この実装一式を別のOpusレビュー(`opus-review-step2to6`)に出したところ、**致命的指摘C1**
  (mozc-readable forkの実OSSビルドが使う登録ウィンドウメッセージ名
  `mozcreadable.direct_actuation.message`と、実装が使っていた
  `googlejapaneseinput.direct_actuation.message`が不一致——direct postが全て黙って
  捨てられ、BUG-234の当の環境ではstep1単独より症状が悪化する)が見つかった。あわせて
  要修正6件(M1: nonceが起動ごとに1から振り直され前回プロセスのackと一致しうる、M2:
  自動リピートのたびにdirect postが積まれる想定漏れ、M3: 新しい書き込み口がlint/guardに
  未登録、M4: 実験的機構に既定オフの手段が無い、M5: 一時的な拒否と経路が死んでいる場合の
  混同、M6: ADR/BUG文書が実装に追随していない)も指摘された。この対応をround2として同じ
  レビュアーに再確認させたところ、**C1・M3・M4・L2は解消と判定されたが、M1は未解消
  (round1の修正が実行されない回帰、R2-1)、M2・M5は一部解消のまま**と判定された:
  R2-1(nonceカウンタの静的初期値が1のままで`compare_exchange(0, …)`が一度も成立せず
  round1のM1修正が無効化されていた)、R2-2(M5の「経路が死んでいる」判定が`(tid,pid,
  cmd_window_handle)`単位の単一スロットで、窓を行き来すると前の窓のマークが上書きされ
  付け外しを繰り返す)、R2-3(M5の解除条件が`cmd_window_handle`の変化のみで、Mozcが単に
  遅かっただけの場合も後続の本物のackを無視して永久除外になる新しい固着)、R2-4(ADR・
  BUG-234文書の古い記述・誤記)を、同じround2の指摘として反映した。**R2-1・R2-2・R2-3・
  R2-4は反映済み**(nonceカウンタの初期値を0に訂正・`mozc_direct_dead_for`を窓単位から
  プロセス全体の回路遮断`mozc_direct_circuit_broken`に変更・forkからの本物のackを1回でも
  観測したら解除する方式に変更・本frontmatter/10節/7節表/BUG-234.mdの訂正)。**M2
  (`explicit_press.rs`の全列挙モデルへのMozcDirect軸追加とP1〜P6検証)は、R2-5のとおり
  意図的に見送ったまま**——11節6番に、キルスイッチの既定をtrueへ切り替える前提条件として
  明記した。**未解決のまま残る既知の限界**: (a) 上記M2の見送り。(b) C1の修正自体は
  実機・CIでのack往復をまだ一度も通していない(8節1番の遅延注入CIは機構の動作確認のみで、
  実際のPostMessageW送信経路は未検証、R2-8でも同じ指摘、11節に前提条件として明記した)。
  (c) mozc-readable fork側のイベント駆動な改修(5-3節)は、ステージ4/5の拒否率データが
  出るまで着手しない方針のまま変更なし。(d) L3(ackログの`latency_ms`がほぼ`None`)は
  round2 R2-9でも「実測を始める前に直すのが望ましい」と再指摘されたが、優先度低として
  今回も対応を見送った。
related_adr: ["ADR-089", "ADR-163", "ADR-171", "ADR-191", "ADR-208", "ADR-241", "ADR-265", "ADR-268", "ADR-291", "ADR-292"]
---

# ADR-293: direct-actuationのnonce/ackをdecide_useの観測入力として配線する(+round4: 柵の分離による全profile対応)

## 1. 問題・動機

[BUG-234](../known-bugs/BUG-234.md): `KEY_EFFECT_SETTLE_MS`(170ms、`crates/awase-windows-base/src/tuning.rs:479`)は
同期的なIMM読みの遅れ用に測った値で、Mozc自身の非同期な開閉切替完了([BUG-230](../known-bugs/BUG-230.md)の
原因2が実測、最大約2秒)には短すぎる。awaseが明示的に送った開閉コマンドをMozcがまだ処理し終えていない
だけの状態を、settle_ms超過を理由に「外部からの変化」と誤認し、beliefを押し戻す
(`crates/awase-ime-core/src/state/mozc_shm.rs:552-580`の`decide_use`、`platform_state.rs:2420-2476`の
`report_mozc_shm`)。固定settle時間の再延長は`tuning-constants.md`が禁じる盲目的エスカレーションであり、
Mozcがそれより遅い場面で再発する。

恒久修正は、awase自身が送った開閉要求の完了を**推測せず確認する**こと。Mozc Readable fork
(`~/mozc-readable-direct-actuation`、ブランチ`exp/direct-actuation-channel`、ベースのワイヤー実装は
commit`98c6c1567`、本ADR検証時のチェックアウトは追随コミット`d49bd6b2e`)が既に実装済みの
version 6共有メモリ(nonce/ack)を使う——これがround1〜3で設計した4-1/4-2節の内容で、**変更していない**。

**round4の追加知見**: round3は「Standard profile(ImmCross経由)にも理論上BUG-234と同じ誤判定の
リスクが残るが、未観測なので計測してから別PRで検討する」と結論していた(旧4-1節のギャップ扱い)。
これを再検討した結果、**このリスクは理論上の懸念ではなく、現在のコードに実在する、別の経路からの
同一欠陥である**ことが判明した(4節)。

**BUG-232/ADR-291 D5との違い**: 同じ`docs/adr291-mozc-shm-push-notify-and-engine-recheck`ブランチで
直近に修正した[BUG-232](../known-bugs/BUG-232.md)(`decide_use`の`update_tick_ms`高速路が、170ms
**未満**でも食い違う読みを誤って受理してしまう穴)とBUG-234は方向が逆である。BUG-232は「本来待つべき
170msより早く受理してしまう」欠陥で、ADR-291 D5で塞いだ(round4レビューでD5がこのHEADに実在する
ことをコード〈`mozc_shm.rs:566-572`の無条件settle待ち・`:543-551`のG3分割baseline〉で確認済み。
マージ済みコミットは`97d849bfe`〈PR #935〉——旧稿が誤って引用した`c8a4393f9`はこのブランチの祖先
ではない、`docs/known-bugs/BUG-232.md`の`fix_commits`参照)。BUG-234は「170msを**正直に待っても**、
Mozcの実際の完了(最大約2秒)にはまだ足りない」という、settleの**値そのもの**の欠陥で、D5の修正後も
残る——BUG-234の実機タイムライン(640ms・239ms)はいずれもD5の無条件170ms待ちを通過した後の誤判定
であることをround4で確認済み。

## 2. round1レビューで判明した、最初の案の2つの致命的な誤り

1. **BUG-234の発生箇所に触れていない**: BUG-234は`report_mozc_shm`→`decide_use`→
   `ShmUse::Use{disagrees:true}`→`pass_through_observed(align_desired=true)`で起きる。送信経路を
   `SendInput`から`PostMessage`に変えるだけでは、+640msでの誤判定はそのまま再現する。
2. **mozc-shm tickからの再送・フォールバックは、architecture_guardが禁じる3本目の生write経路**:
   `apply_mechanism`の呼び出し元は`tests/architecture_guard.rs::raw_mechanism_write_sites_are_confined_to_chain_writers`
   が`ime_controller.rs`と`runtime/open_chain.rs::fallback_write`の2箇所に固定している。

修正した設計(5節)は、送信=新機構1つの追加・確認=`decide_use`への観測入力1つの追加、という
形に収め、両方を解消した。

## 3. round2レビューで判明した、3つの致命的な誤り(いずれも設計変更なしで修正可能)

1. **R1**: `decide_attempt`が未対応時に`None`を返すとチェーン全体が`AlreadyMatched`で止まり、
   GJIユーザー全員がVKを送れなくなる。→ 不適用な機構は`mechanism_applicable`側でチェーンから
   除外し、適用可能なら常に`Some(DirectActuate(open))`を返す。
2. **R2**: `apply_mechanism`はhubへ到達できない(`with_app`再入は`None`)。→
   own-requestは`note_ime_actuation_send()`と同じ「グローバルなスタンプ」方式で記録し、
   `mozc_shm_read_once`が読んで`report_mozc_shm`へ値として渡す。
3. **R3**: 「拒否時はdrift correctionに収束」は`evaluate_drift`のMozcShm除外(ADR-268 D2)で
   発火しない。→ **採用: 諦めてユーザーの再入力に委ねる**(既存の「失われた書き込みは自動
   再送せず押し直しに委ねる」方針と整合)。`evaluate_drift`は変更しない。

## 4. round3の結論とround4での訂正: Standard profileは「対象外」ではなく「柵の分離」で直す

round3は、`MozcDirect`をプロファイルを問わず最優先する一般化(S1)と、mozc-readable fork側の
広いタイマーリトライ(S2)という、所有者方針([[feedback_fix_where_the_information_lives]])の
文字どおりの拡大解釈2つを**不採用と判定した**(9節に残す、この判定自体は訂正していない):

- **S1は今も不採用のまま**: `ImmSetOpenStatus`はTSFの開閉compartment
  (`GUID_COMPARTMENT_KEYBOARD_OPENCLOSE`)を同期的に書き換え、Standard profileの
  `ImmCrossProbe`(High confidence)の信頼性はこの同期書き換えが前提。`MozcDirect`はこの
  compartmentに触れないため、`MozcDirect`をStandard profileの第一機構にすると、IMM読み経路に
  新しいBUG-234型の穴を開けるだけになる。→ **`MozcDirect`は今回もGJI専用チェーン限定のまま**
  (5-1節、変更なし)。
- **S2も今も不採用のまま**: fork側の広いタイマーリトライは非同期で結果が後から出てくる構造と
  噛み合わず、`kSendCommandFailed`のリトライは非冪等。→ **5-3節のまま変更なし**(イベント駆動の
  狭い形、計測後の別タスク)。

round3がS1/S2の**代わりに**「Standard profileは対象外、計測してから検討」とした部分だけを、
round4で再検討した。所有者と検討した最初の仮説(「`ImmCrossProbe`は`ime_model.rs`の
`KeyEffectPredicted`fenceを通り、これがStandard profileの穴」)は、**round4のOpusレビューで
的外れと判定された**(最終的な診断は下記)。

**round4で判明した事実(Opusレビューで訂正済みの、正しい診断)**:

- `ime_model.rs`の`reconcile_key_effect_open`/`reconcile_key_effect_mode`(`KeyEffectPredicted`
  fence、ADR-191決定3)は、**物理モードキーの打鍵予測(`key_effect`)が立っている間だけ**働く
  (`self.key_effect`が`None`なら即return、`:813-818`・`:843-848`)。`key_effect`を立てる
  `ImeEvent::KeyEffectPredicted`の唯一のdispatch元は`ImeStateHub::apply_key_effect_prediction`
  (`platform_state.rs:562-602`)で、本番の唯一の呼び出し元は`runtime/key_pipeline.rs:2091`
  ——これは**awaseがIMEへ透過させる物理モードキー**専用であり、awase自身が送る明示的な
  開閉コマンド(BUG-234の「IME OFF」キー結合)はこの`key_effect`を立てない。
- `ImeModel`では**明示意図(`last_intent`)が観測より常に優先される**
  (`resolve_open_at`、`:571-576`・`:526-528`)。これを観測側が上書きできるのは、`report_mozc_shm`
  (`platform_state.rs:2459-2470`、全profile対象)と、`follow_external_change_in_scope`
  (`:768-791`、**Imm32Unavailable×GJIのみ**、`external_change_watch_applies`)という、専用の
  追随経路**だけ**。Standard windowでは、`ImmCrossProbe`の食い違う読みが`ImeModel`の明示意図を
  覆す経路自体が存在しない。
- **Engineが実際に読むbeliefは`ImeModel`ではなく候補Z(`scope_shadow`)**(`general.belief_v2`、
  既定true、ADR-282)。Zは`ScopeBelief::with_new_thread_prior(KEY_EFFECT_SETTLE_MS)`
  (`platform_state.rs:241-244`)として構築され、**Explicit claim自体が`fence_ms = at_ms +
  settle_ms`を持つ**(`scope_belief.rs:366-385`)——`accepts()`は`read_started_ms >= fence_ms`に
  なった時点で、1件のMedium/High観測がExplicit claimを覆せるようにする(`:786-797`)。これが
  **Standard profileにも等しく効く、本当の汎用fence**(既存テスト
  `imm_observation_is_ignored_in_unreadable_windows_and_fenced_in_readable_ones`、
  `scope_belief.rs:862-875`、readable=Standard windowを明示的に対象にしている)。

つまり、Standard profileの穴は`ime_model.rs`ではなく、**Zの`Explicit claim`fence**
(`scope_belief.rs`、IMM観測を対象)と、**`decide_use`**(mozc-shm、v6ブロックを持つ全profile
対象)の2か所にある。**この診断自体は実在する**(ImmCrossの同期書き込み直後は一致読みになり
やすく頻発しないため未観測なだけ、という当初の直感は正しかったが、どのコードが原因かは誤って
いた)。

**round4の方針(訂正後、採用)**: `KEY_EFFECT_SETTLE_MS`そのものも、Mozc向けに一括で長い値へ
置き換えることもしない——**原因(origin)で区別する**。「awase自身が明示的に送った開閉コマンド
(SendVk/ImmCross/direct post)の後」の食い違いだけを、別途測定した長いcapで待つ。「物理モード
キーの打鍵予測」の食い違いは**170msのまま変えない**(5-4節で理由を詳述——長いcapを無差別に
物理キー予測へ適用すると、予測が外れた場合の訂正を最大約2秒遅らせてしまい、
[BUG-233](../known-bugs/BUG-233.md)(ADR-292 G3が縮めた、まさにこの種の遅延)を作り直す)。
`ime_model.rs`の`reconcile_key_effect_*`は**変更しない**(物理キー予測専用のまま)。

この新しい長いcapは、`decide_use`(mozc-shm)と**Zの`Explicit claim`fence**の両方に、
「このExplicit claimがawase自身の送信由来か」で選択的に適用する。GJI/TsfNative
(`MozcDirect`が使える)はさらにnonce/ackで**この長いcapを確定したら即座に短縮できる**
——Standard profileにはackが無いので長いcapをそのまま待つ(遅いが正しい、今より確実に良い)。

この訂正により、**Standard profileは今回のPRの対象に入る**(旧稿の「対象外」は撤回)。
`MozcDirect`機構そのものはGJI専用チェーン限定のまま(S1の判定は維持)——Standard profileは
`MozcDirect`ではなく、Z/`decide_use`側の原因ベースの修正で直る。

**実務上の留保(round4/5レビューの指摘どおり明記、過大に言わない)**: awaseのImmCross書き込み
自体はIMM compartmentを同期的に書き換えるため、その書き込みの170ms以上後にStandard windowで
`ImmCrossProbe`が食い違う読みを返すケースは稀と見込まれる(round3 S1の論点どおり)。
**Standard profileでBUG-234型の誤判定が実際に確認された事例は無い**(round5で訂正——
BUG-219はMozc×**Chrome**〈Imm32Unavailable、Standard profileではない〉での未解明な偽の
「IME閉」であり、Standard profileの実例ではない)。可能性があるのは、物理モードキー経由・
SendVkフォールバック経由の後のIMM読みだけで、物理キー予測自体は5-4節2のとおり常に170msで
訂正されるため、Zの変更が助けられるのは**SendVkフォールバックのclaimだけ**(5-4節2の
スコープ留保)。Standard profile対応の主眼は**v6ブロックを持つ全windowに効く`decide_use`側の
修正**であり、Zの`Explicit claim`fenceの拡張は、CIの遅延注入実験でこの経路が実際に踏まれる
ことを確認してから出荷判断する(5-4節2)。

## 5. 設計

### 5-1. 送信: `WriteMechanism::MozcDirect`は**GJI専用チェーンに限定**(round3 S1、変更なし)

**採用する範囲**: `decide_chain(inputs)`が、chainが`CHAIN_GJI_ONLY`(Imm32Unavailable/TsfNative
profile)のときだけ、`inputs.mozc_direct_ready`が真なら`CHAIN_MOZC_DIRECT_THEN_GJI =
[MozcDirect, GjiDirect]`を返す。Standard profileは`CHAIN_IMM_CROSS_THEN_GJI`のまま変更しない
——Standard profile側のBUG-234対策は5-4節の汎用fence修正が担う。

### 5-2. 確認: `decide_use`に own-request を読ませる(観測専用、writeなし)

**round5で一般化(U1/U2の共有修正)**: own-requestは、nonceを持つ場合もnonceを持たない場合も
含む、より広い事実`OwnSend { open: bool, tick_ms, tid, pid, nonce: Option<u32> }`として記録する
(**単一スロット**、同方向/逆方向を問わず新しい開閉送信が来たら上書き)。記録するのは
**開閉方向を持つ送信が、実際に送信できたときだけ**(round6 V2。モード切替のみの送信では
記録しない、U1):

- `apply_mechanism`の`SendVk`の`VK_IME_ON`/`VK_IME_OFF`arm——**`Applied`のときだけ**
  (`UnsafeToToggle`等の未送信outcomeではスタンプしない、round6 V2)。
- ImmCross書き込み(`imm_cross_write`)とdrift correctionの`SendPath::ImmCross`——
  **`ActuationOutcome::Written`/`set_ime_open_ordered() == true`のときだけ**(`Aborted`/
  `false`ではスタンプしない)。
- direct post(5-1節)——**`PostMessageW`が`Ok`のときだけ**、`nonce: Some(..)`。

**`(tid,pid)`の取得元(round6 V4-2)**: `apply_mechanism`はviewしか受け取らず、OS状態を
自ら読んではならない(`ime_controller.rs:27-30`)。スタンプの`(tid,pid)`は、5-2節で
viewへ到達させる「Mozcのターゲット」(R2が`WindowsPlatform`→viewへ配線する、既存のv6
ブロック所有スレッド)をそのまま使う。**フォーカス中スレッドにv6ブロックが無ければスタンプ
しない**(`decide_use`自体もその場合は呼ばれない)。`imm_cross_write`・drift correctionの
`SendPath::ImmCross`(`apply_mechanism`の外で動く)も、発行時点の同じplatform側ターゲットを
使う——実装側が`GetWindowThreadProcessId(GetForegroundWindow())`のような独自取得を
殻に書かないよう、この取得元をADRに明記する。

これは`note_ime_actuation_send()`と同じ「グローバルなスタンプ」方式(R2)で、hubへの再入は
不要。`mozc_shm_read_once`が読んで`report_mozc_shm`へ渡す。

**`last_mode_key_tick_ms`の範囲とtie-break(round6 V1)**: この値は物理モードキー予測
(`KeyEffectPredicted`)だけでなく、`UserImeSetIntent`・`ImeApplyRequested`・
`ModeKeyPassedThrough`でも進む(`ime_model.rs:899-904,1104`)。つまり「`own_send`より後に
何か送らずに終わった意図(`Duplicate`・`NotOwned`・`UnsafeToToggle`)」でもsupersedeは起きる
——結果は保守的(従来の170ms判定に戻るだけ)なので危険はないが、「物理モードキー予測だけが
supersedeする」という書き方は不正確なので訂正する。両方のtickは同じ`hook::current_tick_ms()`
(分解能~15.6ms)から取るため、**同じpressのshadow-toggle送信とその予測は同値になりやすい
——その場合は`>=`なので送信側が勝つ(長いcapを維持)**。これは「awaseが実際に送信した」
という事実を優先する選択として妥当だが、11節3番の「後続の物理モードキー予測が常に170msで
訂正される」という言い切りと矛盾するため、11節3番は「同値のときは送信が勝つ(T6-2のtie-break
規則)」に訂正する。**explicit-OFF自身の経路では問題にならない**: `UserImeSetIntent`/
`ImeApplyRequested`は書き込みより前にdispatchされる(`platform_state.rs:1196-1210`)ため、
`own_send.tick_ms`(送信時刻)は自分自身の意図eventのtick以上になり、自分自身をsupersedeしない
——この順序が逆転したらBUG-234がこの経路だけ静かに再発するので、単体テストで固定する。

**`classify_own_request`(純粋関数、`mozc_shm.rs`、`Split`早期return(`:562-564`)より前に
呼ぶ)は、この`OwnSend`の`nonce.is_some()`による絞り込みとして書く(round3 S3の全順序を
そのまま保つ)**:

0. **読んでいるブロックの`(tid,pid)`が`own_send`のものと違う**(round6 V4-1、全順序の
   **先頭**で評価する——後続の項目2・4が無関係なスレッドのスタンプを誤って消費/消去しないよう、
   最初に外す): スタンプを無視する(消さない、フォーカス変更、6節)。
1. **`own_send.nonce.is_some()`かつ自分のnonceへのackがある**(GJI/TsfNativeのみ、他は
   この枝自体が無い):
   - `Applied` → `own_send`を消す。`disagree_floor = ack.update_tick_ms`。
   - `Rejected(_)` → `own_send`を消し、「不使用」を呼び出し元へ返す(フロアは立てない、
     現実に追随する、3節R3の方針どおり)。
2. **ackが無い(nonceが無い、またはまだ来ていない)が、読んだ`open`が`own_send.open`と一致**:
   `own_send`を消す。`disagree_floor`=この読みの`update_tick_ms`(状態は、ackと同じflushでしか
   再公開されない、`tip_state_publisher.cc:544-553`)。
3. **ackが無く、読んだ`open`が`own_send.open`と不一致、
   `own_send.tick_ms >= last_mode_key_tick_ms`(上記のtie-break規則どおり`>=`、
   supersedeされていない)、`age < cap`**: 既存の`ShmUse::Held`(`mozc_shm.rs:517`に既存、
   新設不要)を返す。**`cap`は旧`KEY_EFFECT_SETTLE_MS`ではなく、5-4節で新設する専用の長いcap**
   ——ack未着の間はこのcapいっぱいまで待つので、GJI専用チェーンでも「ackが来るまで」と
   「capに達するまで」の2つの終了条件が並ぶ(ackが来れば即終了、無ければcapで終了、どちらも
   5-4節の同じ値を参照する)。**この条件を満たさない(=supersedeされている)場合は、この枝は
   適用せず、従来の`disagree_baseline_ms`/`KEY_EFFECT_SETTLE_MS`(170ms)の判定にそのまま落ちる**。
4. **`age >= cap`**: `own_send`を消す。**`nonce.is_some()`のときだけ**「不使用」を呼び出し元へ
   返す(round6 V3)——nonceの無い送信(SendVk/ImmCrossが単に遅いだけ)は、消すだけで
   「不使用」マークは立てない(GJI専用チェーンでMozcDirectが`Failed`でフォールスルーした後の
   SendVkスタンプが、無関係にdirect actuationを使用不可にしてしまうのを防ぐ)。

`report_mozc_shm`は`max(既存のdisagree_baseline_ms, disagree_floor)`を取る
(`platform_state.rs:2441-2446`)——これでR7(ack直後の本物の外部変化を取り逃さない)と
R8(判定条件はbeliefではなくown-requestそのものと比較する)の両方を満たす。

**付随する配線(round3 S3)**:
- `report_mozc_shm`は「不使用」判定を**戻り値として返す**(`ShmUse`だけでは運べない、
  フィールド追加または別の戻り値にする)。`mozc_shm_read_once`がこれを読んで
  (tid,pid)単位の「フォーカス変更まで不使用」フラグを立てる。
- ADR-292 P(`key_effect_open_hold`)も同じ`mozc_shm_read_once`経由で`decide_use`を呼ぶ
  (`mozc_shm_shadow.rs:125-128`)ため、その単体テストにown-request保留中のケースを含める。

**(round7 W1で訂正)** 読んでいるブロックの(tid,pid)が`own_send`のものと違う場合は、
5-2節項目0のとおり**そのスタンプを無視するだけで消さない**(`FocusGen`ではなく実際の
(tid,pid)の変化で判定、round3でもR6のまま)——別スレッドの読みのたびに他スレッド宛の
`own_send`を誤って消費・消去しないため(round6 V4-1)。`own_send`自体は、対応する
(tid,pid)のブロックを次に読んだときに通常の経路(項目1〜4)で消費されるか、新しい開閉送信で
上書きされるまで残る(6節)。

**実装後の補正(2026-11-15、PR #941、Opusコードレビュー`opus-review-pr941` round1)**:

- **項目1(nonce+ackの確認)は本PRでは実装しない。** direct post(5-1節)の送信経路
  (`WriteMechanism::MozcDirect`)自体が未実装のため、`own_send.nonce`は常に`None`——項目1は
  到達しない死んだ分岐になる。設計の正確さを保つため、`classify_own_request`の引数から`ack`を
  落とし、項目1自体をコードから削除した(PRにEXPERIMENTAL`acf1a2594`〈`DirectActuationAck`/
  `direct_actuation_plan.rs`等〉を含めないため、round1指摘0)。direct postの送信経路を実装する
  別PRで項目1を復元する。
  **(2026-11-15訂正、round2 R2-4)**: 上記は10節step1(PR #941)時点の記述。10節step2
  (`7278191a4`)で項目1を実際に復元済み——`OwnSendAck{nonce, applied, update_tick_ms}`を
  新設し、`classify_own_request`の署名に`ack: Option<OwnSendAck>`を足した。`direct_actuation_plan.rs`
  はこのADRのどの時点でも作られていない(不採用のまま)。
- **項目2に`read_update_tick_ms > send.tick_ms`を追加(round1指摘1-a)。** 単一スロット+値一致
  だけでは、OFF→ONのように方向を素早く切り替えたとき、2回目の送信(ON)の直前にMozcがまだ
  1回目(OFF)を処理しておらず`tsf_open`がON方向のまま残っていた場合、この**送信前からの古い
  読み**が「ONへの確認」として誤って消費されてしまう(BUG-234の再発経路)。この条件を満たさない
  一致読みは項目3/4と同じ扱いに落ちる。
- **`imm_cross_write`・`set_ime_open`の`(tid,pid)`は、`.await`より前(発行時点)に捕獲する
  (round1指摘3-b)。** 設計の「発行時点の同じplatform側ターゲットを使う」という記述を、
  実装が当初「`.await`完了後の現在のフォーカス」から取っていたために満たせていなかった
  (await中にフォーカスが変わると、書いた窓と違う窓にスタンプが付く)。
- **drift correctionの`SendPath::ImmCross`のスタンプは、`set_ime_open`の中で実completion後に
  移した(round1指摘3-a)。** `set_ime_open_ordered() == true`は「`spawn_local`できた」(= profile
  互換)の意味でしかなく、実際に`SendMessageTimeoutW`が成功したかは示さない——このため当初の
  実装は書けなくてもスタンプしてしまっていた。
- **`report_mozc_shm`の`forced_held`(5-4節1)は、読みがbeliefと食い違うときだけに限定する
  (round1指摘2-a)。** `own_send.open`とbeliefの`effective_open`が食い違う(稀な)場合、読みが
  `own_send.open`とは不一致でもbeliefとは一致していることがあり、その場合はHeldを強制せず
  `decide_use`に委ねる——beliefと一致する読みを保留する理由は無く、`ObserverReported`の
  dispatchを止めてMozcShm観測のTTLを消費するだけになる。
- **`MOZC_SHM_OBSERVATION_TTL_MS`を1000ms→3000msに引き上げた(round1指摘2-a、
  round2 N1で撤回・1000msに戻した——下記「round2レビューの反映」参照)。** Pendingで
  `Held`を強制する間は`ObserverReported`をdispatchしないため、Held中にこの観測自体のTTLが
  切れてより弱い証拠源に落ちるおそれがある。元の導出(500+300+200ms)に
  `MOZC_OPEN_CLOSE_SETTLE_MS`(2000ms)を加えた。
- **panic resetは古い`OwnSend`を消す(round1指摘3-e)。** `panic_reset`の OFF→ON は
  `set_ime_open`を経由しないためスタンプされないが、直前に残っていた無関係な`OwnSend`が
  この後の読みを誤って保護しないよう、`clear_own_send()`を呼ぶ。
- **既知の残る限界(round1指摘3-c・3-f、未修正・受容)**:
  - `imm_cross_write`はスタンプを書き込み成功**後**に行う。発行(`note_imm_cross_write()`)から
    スタンプまでの間(`.await`の間)に食い違う読みが来ると、`own_send`がまだ無いので従来の
    170ms判定で追随してしまう。`SendMessageTimeoutW`が遅い(タイムアウト近辺)窓で、まれに
    BUG-234が残る。
  - 「v6ブロックが無いスレッドではスタンプしない」は未実装——`(tid,pid)`はフォーカス中スレッド
    から機械的に計算するだけで、そのスレッドにmozc-shmのv6ブロックが実在するかは見ていない。
    単一スロットなので、Mozc窓Aへ送信した直後に別窓B(MS-IME等)へ何か送信すると、Aの保留中の
    スタンプがBに上書きされ、Aの保護が消える。`Runtime.mozc_shm.accepted_for`がこの情報を
    持つが、`WindowsPlatform`からは参照できない(層の境界)。実害が確認されたら、この情報を
    `ImeControlView`へ渡す経路を別途設計する。

**round2レビュー(`opus-review-pr941`)の反映(2026-11-15)**:

- **N1(H)、`MOZC_SHM_OBSERVATION_TTL_MS`を1000msへ戻した。** round1 2-aへの対応として一度
  3000msへ引き上げたが不適切と判定された: このTTLはMozcShmの全観測(`Split`・
  `Rejected(Inactive)`〈同じスレッドで別のTIPへ切り替え〉・読み取り失敗等、`own_send`の
  Pendingと無関係な経路も含む)に効くうえ、`OBSERVATION_FRESH_WINDOW_MS`(3000ms)と同じ値に
  なってADR-268 D1がMozcShmにだけ短いTTLを付けた理由を無くしてしまう。かつBUG-234では、
  送信前の**誤った**High観測(`open=true`)がPendingの間生き続けるため、TTLを延ばすほどこの
  誤った観測を長く残す方向に働き、2-aが心配した「弱い証拠源に落ちる」とは逆の害になる。
  `own_send`のPending中にこの観測が失効して他の証拠源に反転しないかは、実機確認で別途
  確かめる(11節項目5、TTLの値を動かして対応する問題ではない)。
- **N2(M)、`forced_held`を`decide_use`の`disagrees`と同じ条件に揃えた。** round1 2-aの修正
  (`forced_held`を`effective`との食い違いに限定)が、`desired`との食い違いを見落としていた:
  drift correctionの処理待ち(`desired`と食い違うが`effective`とは一致する読み)が
  `decide_use`の一致の高速路を通ってしまい、`disagrees: open != desired_open`でdesiredを
  元に戻す——BUG-234と同じ「送信の処理待ちを外部変化と誤認」が別の経路(desired)で起きていた。
  `forced_held = pending && !split && (shm.tsf_open != effective || shm.tsf_open !=
  desired)`に修正。
- **N3(L〜M)、項目2の必要十分性を訂正。** 「`read_update_tick_ms > send.tick_ms`を追加した
  ので直った」という書き方だったが、`update_tick_ms`はブロック全体のdirtyスタンプ
  (BUG-232 D5)なので、送信後に別の打鍵が1つでも来れば十分条件を満たしてしまう——これは
  **必要条件**であり、完全に閉じるのはnonce/ack(direct postの送信経路、本PRには無い)。
  5-2節冒頭の`classify_own_request`のdocを訂正済み。
- L1〜L5: `key_effect_open_hold.rs`の「訂正速度は保たれる」という不正確な記述を訂正、
  `architecture_guard`の検査範囲(呼び出し元の件数+条件式の存在、分岐の内側までは検査しない)を
  doc上も正確化、読み側(`mozc_shm_shadow.rs`)も`WindowsPlatform::mozc_target()`を使うよう
  統一、`with_app`が使えずスタンプが黙って落ちる経路にデバッグログを追加。

### 5-3. mozc-readable fork側の改修: スコープを絞り、計測結果が出るまで着手しない(round3 S2、変更なし)

当初案(「`kNoFocusedContext`/`kSendCommandFailed`を確定前にタイマーで数回リトライする」)は
round3レビューで**実装不可能かつ一部危険**と判定された:

1. **多くの結果は`OnDirectActuationCommand`の呼び出し元ではなく、非同期`DoEditSession`の中から
   後で報告される**(`tip_edit_session.cc`の`private_context`null・`SendCommand`失敗・
   `UpdateContext`失敗、いずれも`AsyncSwitchInputModeEditSessionImpl::DoEditSession`内)。
   当初案が想定した「呼び出し元でリトライする」場所に、そもそも大半の結果が現れない。
2. **`kSendCommandFailed`のリトライは冪等ではない**: `SendCommand`はサーバーIPCで、失敗・
   タイムアウトはサーバーが実際に適用したかどうかを証明しない。`UpdateContext`失敗の後に
   再送すると、1回目が半分だけ更新したcontextに対してもう一度`TURN_*`/`SWITCH_COMPOSITION_MODE`
   を実行することになり、変換中のテキストを二重に変えるおそれがある。**`kSendCommandFailed`は
   リトライしない**。
3. **タイマーからのリトライは、awase側で禁止したのと同じ「タイマーからの書き込み」の危険を
   C++側に持ち込む**: 古いnonce(N1)のリトライが、新しい要求(N2)やユーザーの物理IMEキー操作の
   **後**に発火すると、古い`desired`が最後に適用されてしまう。TIPは`ActuationOrder`のような
   型での保護機構を持たない。

**縮小した設計**: リトライ対象は、`SwitchInputModeAsync`の**同期的な早期return**のうち、
まだ何も送信していない「フォーカスがまだ来ていない」ケース(`GetFocus`がnull documentを返す・
`ITfContext`が取れない、`tip_edit_session.cc:922-940`)**だけ**にする。TIPは既にthread focus/
document focusをsinkしている(`thread_focus_cookie_`等)ので、この場合だけ要求を**次の
`OnSetFocus`/`OnSetThreadFocus`まで遅延させる**(イベント駆動、ポーリングやタイマーの
定期リトライではない)。終端の期限だけ`SetTimer`を使うが、**direct-actuation window自身に
バインド**する(`direct_actuation_window_handle_`、`UninitDirectActuationWindow`→
`DestroyWindow`でDeactivate時に確実に破棄される、`tip_text_service.cc:1537-1545`)——
Deactivateで期限が破棄されたら**ackを一切書かない**(awase側のcapが拾う、ワイヤー互換は
変えない)。新しい要求が来たら`latest_nonce`のみを有効とし、古い遅延要求は(ワイヤーを変えずに)
`kNoFocusedContext`の確定扱いで畳む(supersession)。

**着手条件(round3 S2-5)**: 「拒否がよく起きる」ことを示す実測データが無い。awaseは
`mozc_direct_ready`(=フォーカス中のスレッドのv6ブロックを既に読めている)のときしか送らず、
(tid,pid)が変わったら送らない(5-1節)ので、フォーカス直後の`kNoFocusedContext`自体が元々稀と
予想される。**この改修は、10節のステージ4/5で収集するack結果ログ(拒否率)を見てから、必要性が
確認できた場合にのみ着手する別タスクとする**(本PRの実装には含めない)。

### 5-4. 新設: 「原因(origin)」で区別する長いsettle cap(round4、Opusレビューで設計訂正済み)

**round4の最初の案は不採用**: `ImeKindId`(Mozc系かどうかの**推定**)で柵の長さを選ぶ案を
最初に検討したが、round4のOpusレビューで2点の欠陥が指摘された(9節に根拠を残す)。

1. **長いcapには実コストがある**: 物理モードキーの打鍵予測が外れた場合(Mozcがcoldで開かな
   かった・キーが無視された等)、Mozc向けの柵をすべて長くすると、訂正が最大約2秒遅れる。
   これは[BUG-233](../known-bugs/BUG-233.md)(ADR-292 G3が縮めた、まさに「予測だけでbeliefが
   倒れたまま約2.5秒訂正されない」症状)を作り直す。柵は「Mozcが遅いだけ」と「打鍵の効果が
   そもそも起きなかった」を区別できない——区別できるのはackだけで、それもawase自身の送信に
   しか使えない。
2. **`ImeKindId`による分岐は「悪化方向が無い」という前提が成り立たない**: 長いcap自体に
   コスト(1の訂正遅延)があるため、「実際はMozc系でないのに`Gji`と誤判定されcapが伸びる」
   方向も悪化になる。`ImeKindId`はTIP検出由来の推定で、ユーザーがスレッド上でIMEを切り替えた
   直後(Win+Space・言語バー)は一時的に古い種別が残りうる——この間、MS-IME windowが`Gji`と
   誤判定されて長いcapを受けるケースが実在する。

**採用する設計(origin-scoped、T5。round5 U1/U2で「原因」の定義自体をさらに訂正)**: 柵の
長さを「対象がMozc系IMEかどうか」ではなく、**「この食い違いの直前にawase自身が明示的に
送信したかどうか」**で選ぶ。ゲートには推定値(`ImeKindId`)ではなく観測値(「フォーカス中
スレッドにv6ブロックが実在するか」、`MozcShmShadow::accepted_for`/`block_present`、
`mozc_shm_shadow.rs:41-49`。Zは既に`mozc_block`という事実を受け取る、`scope_belief.rs:772-775`)
を使う。

**round5で判明した欠陥**: round4直後の案(`ime_actuation_baseline_ms`という方向・スレッドを
持たないグローバルな1個の打刻をそのままOR条件に使う)には2つの欠陥があった:

- **U1**: `ime_actuation_baseline_ms`は`imm_cross_write_tick_ms`と`ime_actuation_send_tick_ms`
  の最大値で、両方とも方向もスレッドも持たない生の打刻(`mozc_shm_shadow.rs:225-226`、
  `hook.rs:360-381`)。かつ`note_ime_actuation_send()`はモード切替キー(無変換/変換等)の送信
  でも呼ばれる(`ime.rs:172,264`)。これをそのままcapの条件にすると、**awaseが何か(モード
  切替キーでもよい)を送った後約2秒間、どの窓のどの食い違う読みも無差別に保留してしまう**
  ——「IME OFFを送った直後にひらがなキーを押し、Mozcがcoldで開かなかった」という普通の
  操作で、BUG-233をこのORの側道から再発させる。
- **U2**: Z側の「origin」をイベント種別(`UserImeSetIntent`/`ImeApplyRequested`)で判定する
  案も不正確——`ImeApplyRequested`はそもそもZのイベントに写像されない
  (`scope_event_of`、`scope_belief.rs:681-698`、`:676-677`で「それ以外」group落ち)。
  `UserImeSetIntent`は**送信の有無にかかわらず記録される意図**で、`NotOwned`・`Unwarranted`・
  `UnsafeToToggle`で実際には何も送らなかった場合も同じイベントが立つ——失われた書き込みの
  claimまで長い柵を受けてしまう。

**採用する修正(round5、両方の共有修正)**: 5-2節で導入する、方向・スレッドを持つ単一の
`OwnSend`事実(nonce有無どちらも含む、round3の own-request を一般化したもの)を、
`decide_use`とZの**両方**がそのまま参照する——イベント種別や生の打刻ではなく、この1つの
事実で「origin」を判定する。

**新しい測定対象の定数**(仮名`MOZC_OPEN_CLOSE_SETTLE_MS`、`tuning.rs`に追加、
`#[measured_macro::measured(...)]`必須)。測定方法(`tuning-constants.md`の実測義務。
Opusレビューの指摘どおり単一の約2秒点ではなく分布で報告する): dragonflyg4実機で
「awaseの送信(direct post/SendVk/ImmCross、ackが無い経路も含む)」から「`tsf_open`を
実際に反転させるflushの`update_tick_ms`」までの時間を、direct post(ack有り)・
SendVk/ImmCross(ack無し)それぞれでcold/warm両方測り、分布を記録する。
`KEY_EFFECT_SETTLE_MS`そのものは変更しない(物理キー予測専用のまま)。

**適用箇所**:

1. **`decide_use`(mozc-shm)**: 保留条件は5-2節の`classify_own_request`項目3そのもの:
   `own_send`が存在し、`(tid,pid)`が一致し、`read.open != own_send.open`、
   `own_send.tick_ms >= last_mode_key_tick_ms`(この送信より後の物理モードキー予測に
   supersedeされていない、round5 U1)、`now − own_send.tick_ms < MOZC_OPEN_CLOSE_SETTLE_MS`
   ——このときだけ保留する。supersedeされている場合・`own_send`が無い場合は、従来の
   `disagree_baseline_ms`/`KEY_EFFECT_SETTLE_MS`(170ms)の判定にそのまま落ちる(変更なし)。
   GJI専用チェーンで`own_send.nonce`がある場合は`classify_own_request`項目1がack到着時点で
   この保留を早期終了させる。Standard windowは`nonce: None`のためackが無く、capまでそのまま
   待つ(または項目2で読みが一致した時点で終了)。
2. **Z(`scope_belief.rs`)**: `ScopeBelief`全体で1つの`settle_ms`を持つのではなく、
   **`accepts()`が、イベント種別ではなく同じ`OwnSend`事実を見て、柵を個別に延長する**
   (round5 U2)。延長する条件(すべて満たすときだけ、`claim`はこのExplicit claim):
   - 一致する`OwnSend`が存在する(同じ(tid,pid)、`s.open == claim.value`、
     `s.tick_ms >= claim.at_ms`、**`ε = 0`**(round6 V4-3。5-2節の意図dispatch→送信の順序
     〈tie-break節〉により、claim自身の送信はこれだけで必ず満たす。正の`ε`は無関係な送信
     直後の予測claimまで延長してしまうため使わない))。
   - 観測が食い違う(`obs.open != s.open`)。
   - そのスレッドにv6ブロックが存在する(mozc-block事実)。
   このとき`fence_ms = s.tick_ms + MOZC_OPEN_CLOSE_SETTLE_MS`。満たさない場合(`OwnSend`が
   無い——`NotOwned`/`UnsafeToToggle`等で実際には送信しなかったclaim、または物理モードキー
   予測由来のclaim)は、既存どおり`fence_ms = claim.at_ms + KEY_EFFECT_SETTLE_MS`(変更なし)。
   `OwnSend`の事実を、`WindowFacts`と同じ経路でZの評価へ渡す(`ScopeBelief.settle_ms`・
   `Claim`構造体自体は変更しない、延長は`(claim, own_send, now)`の純粋関数)。既存テスト
   `imm_observation_is_ignored_in_unreadable_windows_and_fenced_in_readable_ones`
   (`scope_belief.rs:862-875`)と対になる新規テストで、(a)一致する送信があるclaim→capまで
   延長、(b)送信が無いclaim(`NotOwned`/`UnsafeToToggle`)→170msのまま、(c)予測由来のclaim→
   170msのまま、の3パターンを固定する。
   **スコープの留保(round5 U2)**: 4節の実務上の留保どおり、ImmCross書き込み自体は同期的で
   この延長が効く場面は稀と見込まれる——Zのこの変更が主に効くのはSendVkフォールバック後の
   IMM読みであり、ImmCross書き込み直後の事例は未確認。診断だけを根拠に出荷せず、CIの遅延
   注入実験(5-3節と対になる、fork側に遅延を仕込む検証)でこの経路が実際に踏まれることを
   確認してから出荷するか、確認できるまで観測専用(ログのみ)に留めるかを実装時に選ぶ。
3. **`ime_model.rs`の`reconcile_key_effect_*`・`KeyEffectPrediction`は変更しない**
   (round4最初の案から撤回)。これらは物理キー予測専用で、1の理由により170msのまま
   固定する。したがって旧稿6節にあった`KeyEffectPredicted`のpayload拡張・`#[serde(default)]`
   の記述も撤回する(6節で削除)。

**MozcDirectの位置づけ(変更なし)**: 5-1/5-2節のnonce/ack(GJI/TsfNative限定)は、この
origin-scopedな長いcapの**上に乗る高速化**であり、「正しさを担保する唯一の手段」ではない。
ack無しでも`MOZC_OPEN_CLOSE_SETTLE_MS`いっぱい待てば正しい判定に落ち着く。GJI/TsfNativeは
ack到着時に即確定、Standard profileは`MOZC_OPEN_CLOSE_SETTLE_MS`を待ってから確定——どちらも
物理キー予測の訂正速度(170ms)には影響しない。

**`ImeModel`自体は変更しない(設計上の意図)**: 変更対象はZと`decide_use`の2つだけで、
`ImeModel`はこのADRの範囲では一切変更しない。「片方のモデルだけ直してZには反映されない」
という分岐(BUG-231の経緯そのもの)が起きないよう、変更を Z/`decide_use` に限定したことを
明示する——展開後は`[belief-v2]`食い違いログで実際に分岐が起きていないかを確認する(10節)。

**この2軸(direct-actuation/actuation chainとbelief/fence)が独立であることの確認(T6)**:
`MOZC_OPEN_CLOSE_SETTLE_MS`は5-2節(own-requestのcap、ackで短縮される)と5-4節(origin-scoped
な長いcap)の両方で使うが、T5の設計のもとでは両者は**同じ量**(「awaseの送信をMozcが処理し
終えるまでどれだけかかりうるか」)を指しているため、1つの定数を共有してよい。actuation chain
側(`decide_attempt`・`apply_mechanism`・`WriteMechanism`・`PressLedger`)はこのADRのどの節でも
変更しない——共有する書き込みは`note_ime_actuation_send()`/`note_mozc_direct_post()`の送信時刻
スタンプ1つだけで、新しい生write経路は増えない(2節が禁じたものに触れない)。

## 6. press-identity(ADR-208 L1)・journal/bug report(ADR-095)への影響

- `PressLedger::claim`の呼び出し元は変わらない(`dispatch_set_open`・`kp_shadow_actuate`の
  既存2箇所のまま。drift correctionは`claim`を呼ばない——`press=None`で素通りする)。
- `AppliedWithoutSendInput`は`outcome_sent_nothing`(`press_ledger.rs:45-50`)に含まれないため、
  押下の予約はそのまま保持される——同じtick内の`SendVk`フォールスルーも同一の予約の下で完結する。
- `DecisionInputs`に足すのは`mozc_direct_ready: bool`の1個だけ(`#[serde(default)]`)。
  HWND・nonceは`DecisionInputs`/`MechanismCommand`に一切入れない(ADR-163 D8)。
- `ImeEvent::KeyEffectPredicted`・`KeyEffectPrediction`は変更しない(5-4節、round4で撤回)。
  変更はZの`scope_event_of`写像にmozc-blockの事実を渡す経路の追加だけで、journal replayの
  既存イベント型には触れない。

## 7. フォールバック・失敗モードの一覧

| プロファイル | 検出タイミング | 事象 | 対応 |
| --- | --- | --- | --- |
| GJI/TsfNative | 送信時(同tick) | `PostMessageW`が`Err` | `Failed`→同tickで`GjiDirect`へフォールスルー |
| GJI/TsfNative | 送信時 | HWND値が別スレッド/プロセスへ再利用されている | 送信前に`GetWindowThreadProcessId`で(tid,pid)照合、不一致なら送らず`Failed`扱い |
| GJI/TsfNative | 後発(別tick) | ack `Rejected(...)` | own-request消去、(tid,pid)を「フォーカス変更まで不使用」にマーク、`evaluate_drift`は変更せず現実に追随する(3節R3) |
| GJI/TsfNative | 後発 | ack無しで`MOZC_OPEN_CLOSE_SETTLE_MS`経過(nonce有り) | **(2026-11-15訂正、round2 R2-2/R2-3)** own-request消去、プロセス全体を回路遮断(`mozc_direct_circuit_broken`、窓単位ではない)。解除はフォーカス変更ではなく、forkからの本物のack(`last_cmd_nonce`が自分の発行した最後のnonceと一致、結果を問わない)を1回でも観測したとき(`clear_mozc_direct_circuit_broken_if_alive`)のみ |
| GJI/TsfNative | 後発 | 読んでいるブロックの(tid,pid)が`own_send`と違う | スタンプを無視するだけで消さない(項目0、round7 W1) |
| 全profile(mozc-shm) | 後発 | `OwnSend`が存在・(tid,pid)一致・読みが`OwnSend.open`と不一致・`own_send.tick_ms >= last_mode_key_tick_ms`(supersedeされていない)・`age < MOZC_OPEN_CLOSE_SETTLE_MS` | `decide_use`が保留(Held、5-2節項目3/5-4節1) |
| 全profile(mozc-shm) | 後発 | 上記が不成立で`age >= cap`、**`own_send.nonce`が`Some`**(GJI/TsfNativeの直接actuation) | `own_send`を消し「不使用」マーク(5-2節項目4) |
| 全profile(mozc-shm) | 後発 | 上記が不成立で`age >= cap`、**`own_send.nonce`が`None`**(SendVk/ImmCross、単に遅いだけ) | `own_send`を消すだけ、「不使用」マークは立てない(round6 V3) |
| 全profile(mozc-shm) | 後発 | `OwnSend`が無い、またはsupersedeされている | 従来の`disagree_baseline_ms`/`KEY_EFFECT_SETTLE_MS`(170ms)判定に落ちる(変更なし) |
| Standard(readable window) | 後発 | Zの`Explicit claim`に一致する`OwnSend`があり(同スレッド・同方向)、fence内で食い違う`ImmCrossProbe`/Medium以上の観測 | claimを保持、観測を採用しない(5-4節2、SendVkフォールバック限定の見込み) |
| Standard(readable window) | 後発 | `OwnSend`が無い(送信していないclaim)、または物理モードキー予測由来のclaim、fence(170ms)超過 | 観測を採用(変更なし、BUG-233の訂正速度を維持) |

**チャネルの混在について**: GJI専用チェーンはMozcDirectとSendVkの2つだけで、どちらか一方しか
同時に有効ではない(round3 S6)。Standard profileは`ImmCross`という既存の単一経路のまま
——5-4節の変更はその経路の**判定側**(いつ観測を信用するか)だけで、送信経路は増えない。

## 8. このPRのスコープ: 開閉(bit0)・全profile対応(round4で拡大)

ワイヤーのbit1〜31は予約のまま使わない。かな/英数等のモード切替への拡張は別ADRへ委ねる。
mozc-readable fork側の内部リトライ(5-3節)は計測後の別タスクのまま。

**round4/5で追加されたスコープ**: `MOZC_OPEN_CLOSE_SETTLE_MS`・`OwnSend`事実の新設と、
Zの`Explicit claim`fence・`decide_use`双方への適用——これにより**Standard profileもこのPRで
直る**(`ime_model.rs`は変更しない、5-4節3)。

**実装前に検証する(`one-off-verification-workflows.md`準拠の1回限りworkflow)**:

1. (round1〜3から変更なし)TIPの`TURN_OFF_IME`はmodeを`DIRECT`にし、`TURN_ON_IME`は直前の
   変換モードを保持する(`tip_text_service.cc:1585-1598`)。awaseの既存GjiDirectベースラインは
   `VK_IME_ON`/`VK_IME_OFF`(0x16/0x1A、`ime_controller.rs:19-22`)を送る。この2つの経路が
   「開いている最中の未確定の変換(preedit)がOFFでどう処理されるか」で同じ振る舞いをするかを、
   実装(10節)に入る前にCIで1回確認する。
   - **結果(2026-11-15、`ci/directact-precheck` commit `60deea90b`、GitHub Actions run
     [43684518054](https://github.com/cuzic/awase/actions/runs/43684518054)、2 runner並列)**:
     未確定の「か」がある状態でOFFした場合、direct post(ack `applied`)と`VK_IME_OFF`は
     完全に同じ結果(`text="か"`・`tsf_open=0`・`effective_open=0`、続く`x`入力も両方
     `"かx"`)。`preedit_equivalent: true`を両runnerで確認——**検証済み、設計変更不要**。
     同じrunで、クロスプロセス送信(awaseの実経路相当)でのpost→ack遅延は0〜16ms
     (`hook::current_tick_ms()`の分解能~15.6msに一致)で、reserved_bits/nonce=0の拒否も
     設計どおり動作。
2. (round4で新設)`MOZC_OPEN_CLOSE_SETTLE_MS`の値そのもの——dragonflyg4実機で、awaseの送信
   (direct post〈ack有り〉・SendVk/ImmCross〈ack無し〉)から、`tsf_open`を実際に反転させる
   flushの`update_tick_ms`までの時間を、両方の送信経路×cold/warmで測定し、**単一の約2秒点
   ではなく分布として**報告する(`tuning-constants.md`の実測義務、round4レビューの指摘)。
   - **関連結果(2026-11-15、`ci/bug234-publish-delay` commit `3636abd20`、run
     [43684519809](https://github.com/cuzic/awase/actions/runs/43684519809)、遅延注入した
     mozc-readable fork、各10試行)**: 機構が170msのsettle窓と整合して動くことをCIで確認——
     注入遅延0ms(実測publish ~72ms)で再現0/10、300ms(実測~381〜392ms)で再現5/10、700ms
     (実測~764〜790ms)・2000ms(実測~2068〜2090ms)でいずれも再現10/10。170msを境に
     再現率が上がるという原因モデルの裏付けにはなったが、**これは注入遅延であり
     dragonflyg4実機でMozcが自然に示す遅延の分布ではない**——本項目が求める実機cold/warm
     測定はまだ未実施(11節2番参照)。

## 9. 検討したが不採用

- **`ImeOpenOutcome::Pending`の新設**(round1): `ActuationOrder`は`Clone`を持ち
  (`actuation_chain.rs:323`)アフィンではないため、「アフィン性を再利用する」という前提が
  誤り。`ImeOpenOutcome`は24ファイルから参照され、`Pending`はどの既存シンクにも素直な
  意味を持たない。`AppliedWithoutSendInput`を代わりに使う。
- **`MechanismCommand`に`nonce`/`cmd_window_handle`を含める**(round1): `decide_attempt`は
  `DecisionInputs`に対して純粋であることが前提で、可変値・OS値は決定入力として渡せない。
- **mozc-shm tickからのフォールバックwrite/再送**(round1): architecture_guardが禁じる
  3本目の生write経路になる。
- **非同期チェーンに載せる案**(round1): 同期パスのレイテンシ軸を壊し、フォールバックVKが
  最大cap分遅れて無関係な窓へ誤送信するリスクがある。
- **`decide_attempt`のMozcDirect armを「既に一致なら`None`」とする案**(round2 R1-ii):
  オプション(i)の「常に`Some`を返す、適用可否は`mechanism_applicable`側」の方が単純。
  **`opus-review-step2to6` M2で訂正・採用に反転**: 「単純」側を選んだ理由
  (「directactuationにはshadowのような確認済み状態のキャッシュが無い」)が誤りだった——
  `shadow_on`はGjiDirect専用ではなく対象の開閉の汎用キャッシュで、MozcDirectも同じ対象を
  書く以上参照できる。常に`Some`のままだと、物理キーの自動リピート(`press=None`、
  `explicit_press_applied_pair`が`shadow_on`を未知化しない経路)のたびに新しいnonceで
  direct postが積まれ、Mozc側のサーバーラウンドトリップとedit sessionが連続要求で
  伸び続ける実害がある(最悪、BUG-230/234が懸念する「Mozcの切替完了が最大約2秒」の条件下で
  キューが伸び、その間の通常の打鍵の処理も遅れうる)。`gji_direct_already_matches`と同じ
  省略を`decide_attempt`のMozcDirect armに追加した(round2指摘への対応コミット
  `5cb7ce6e3`で実装、10節step2〔`7278191a4`〕とは別コミット。押下
  (`press=Some`)は`shadow_on`が未知化されるため省略されず、リピートだけ省略される)。
  `explicit_press.rs`の全列挙モデル(`PressState`)は`mozc_direct_ready`の軸を持たないため、
  この省略がMozcDirect先頭のチェーン上でADR-208 L1の不変条件を崩さないことはモデル上では
  未検証のまま(11節に追記)。
- **拒否時に1回だけ特別に再送する案**(ユーザーとの対話、R3-b): ADR-268 D2への新しい
  例外が要り、正当化に別ADRが必要。
- **プロファイルを問わずMozcDirectを最優先する案**(round3 S1、round4でも不採用のまま): Standard
  profileのImmCrossはVK到達性の代替ではなく、beliefが読み戻すIMM compartmentへの同期書き込みを
  担っており、置き換えると別の場所にBUG-234型の穴を開ける。**round4はStandard profileを
  `MozcDirect`無しで直す別案(5-4節)に置き換えただけで、この判定自体は覆していない。**
- **mozc-readable fork側の広いタイマーリトライ**(round3 S2、round4でも不採用のまま): 非同期で
  結果が後から出てくる構造と噛み合わず、`kSendCommandFailed`のリトライは非冪等、タイマー
  リトライはawase側で禁止したのと同じ危険をC++側に移すだけ。
- **`KEY_EFFECT_SETTLE_MS`そのものを全面的に(例:2秒へ)引き上げる案**(round4で検討・不採用):
  この定数は開閉/モード以外の`KeyEffectPredicted`判定や、Mozc系以外(MS-IME等)のIMEにも
  無差別にかかる。Mozcの非同期完了とは無関係な用途まで無根拠に遅くするのは
  `tuning-constants.md`が戒める「同じ役割の定数の盲目的釣り上げ」そのもので、しかも今回は
  「同じ役割」でさえない複数の用途を一括りにする分、害がさらに広い。
- **`ime_model.rs`の`reconcile_key_effect_open`/`_mode`(`KeyEffectPredicted`fence)を直す案**
  (round4の最初の案、Opusレビューで不採用・T1): この柵は物理モードキーの打鍵予測
  (`key_effect`)が立っている間だけ働き、awase自身が送る明示的な開閉コマンドはこの予測を
  立てない。実際にStandard profileで効いている柵はZ(`scope_shadow`)の`Explicit claim`fence
  (`scope_belief.rs`)であり、ime_model.rs側を変更しても診断対象のStandard profileの穴は
  一切直らない。
- **「対象IMEがMozc系(`ImeKindId::Gji`)かどうか」で柵の長さを選ぶ案**(round4の最初の案、
  Opusレビューで不採用・T4): 長いcapには「打鍵予測が外れた場合の訂正を最大約2秒遅らせる」
  という実コストがあり([BUG-233](../known-bugs/BUG-233.md)の再発に直結、T3)、
  「悪化方向の誤判定は存在しない」という安全性の前提(P20/`ime_kind.rs`)が成立しない
  ——`ImeKindId`はTIP検出由来の推定で、IME切替直後は古い種別が一時的に残り、実際はMS-IME
  なのに`Gji`と誤判定されて長いcapを受けるケースが実在する。代わりに、観測値(「フォーカス
  中スレッドにv6ブロックが実在するか」)と「この食い違いの直前にawase自身が送信したか
  (origin)」で柵の長さを選ぶ設計(5-4節)に置き換えた。
- **`ime_actuation_baseline_ms`(方向・スレッドを持たない生の打刻)をそのままorigin判定の
  条件式に使う案**(round4直後の案、round5のOpusレビューで不採用・U1): モード切替キーの
  送信でも打刻が進むため、awaseが何か送るたびに約2秒間、無関係な窓の無関係な食い違いまで
  無差別に保留してしまい、BUG-233を側道から再発させる。代わりに、方向・`(tid,pid)`・
  supersession規則を持つ`OwnSend`事実(5-2節)に置き換えた。
- **Zの柵の延長をイベント種別(`UserImeSetIntent`/`ImeApplyRequested`)で判定する案**
  (round4直後の案、round5のOpusレビューで不採用・U2): `ImeApplyRequested`はZのイベントに
  そもそも写像されず、`UserImeSetIntent`は送信の有無にかかわらず記録される意図(`NotOwned`・
  `UnsafeToToggle`等でも立つ)のため、送信していないclaimまで長い柵を受けてしまう。代わりに、
  同じ`OwnSend`事実で判定する設計に置き換えた。

## 10. 実装順序

0. **検証(実装前)**: 8節のCI等価性確認(TURN_OFF_IME/ON vs VK_IME_OFF/ON)と、
   `MOZC_OPEN_CLOSE_SETTLE_MS`の実機測定(dragonflyg4、cold/warm)。
1. **柵の分離、origin-scoped(round4/5、まずこれ)**: `tuning.rs`に`MOZC_OPEN_CLOSE_SETTLE_MS`
   を追加(測定値を引数に、`#[measured(...)]`、分布で計測・8節)。`OwnSend`事実
   (5-2節、方向・`(tid,pid)`・tick・`Option<nonce>`を持つ単一スロット)を新設し、
   `VK_IME_ON/OFF`送信・ImmCross書き込み・drift correctionの`SendPath::ImmCross`・
   direct postの4か所でスタンプする(5-2節)。
   - `decide_use`/`report_mozc_shm`: 保留条件を5-2節項目3の全順序どおりに実装する
     (`own_send`の存在・`(tid,pid)`一致・方向不一致・`own_send.tick_ms >=
     last_mode_key_tick_ms`のsupersession・cap未超過、のANDが成立するときだけ保留。
     不成立なら従来の`disagree_baseline_ms`/`KEY_EFFECT_SETTLE_MS`判定、5-4節1)。
   - Z(`scope_belief.rs`): `accepts()`が`OwnSend`事実を見て、一致する送信がある
     Explicit claimだけ`fence_ms`を`MOZC_OPEN_CLOSE_SETTLE_MS`へ延長する(5-4節2)。
     mozc-block事実を評価経路へ渡す。**出荷判断はCIの遅延注入実験の結果待ち
     (round5 U2のスコープ留保、11節4番)**——この段階ではログのみでもよい。
   - `ime_model.rs::reconcile_key_effect_*`・`KeyEffectPrediction`は**変更しない**(5-4節3)。
   - **スタンプの`(tid,pid)`の取得元は5-2節の明記どおり(round6 V4-2)**: `apply_mechanism`は
     viewに流れ込む「Mozcのターゲット」を使い、OSから自前で取得しない。v6ブロックが無い
     スレッドではスタンプしない。
   - 単体テスト: (a) Standard profile相当(readable window)で、`OwnSend`と一致するExplicit
     claimがcap内で食い違う観測に覆されないこと、`OwnSend`が無いclaim(`NotOwned`/
     `UnsafeToToggle`)・物理キー予測由来のclaimは170msで覆るままであること(既存
     `imm_observation_is_ignored_in_unreadable_windows_and_fenced_in_readable_ones`と対になる
     新規テスト、round5 U2)。(b) `own_send`送信の**真に後**(tickが大きい)に物理モードキー
     予測が立った場合、その予測の訂正が170msのまま保たれること(T6-2、`tick_ms >=
     last_mode_key_tick_ms`のsupersession規則で**確定済み**、実装時の選択ではない)。
     (c) 別スレッド/別窓への`own_send`が他窓の食い違う読みを保留しないこと(round5 U1の
     cross-window over-hold防止)。(d) モード切替のみ(開閉方向を持たない)の送信は`OwnSend`を
     更新しないこと(round5 U1のmode-only-send防止)。(e) 同じpressのshadow-toggle送信と
     その予測が**同値tick**の場合、送信が勝ち長いcapが維持されること(round6 V1のtie-break、
     (b)と対になる境界テスト)。(f) explicit-OFF自身の`UserImeSetIntent`/`ImeApplyRequested`は
     自分自身の送信をsupersedeしないこと(round6 V1、この順序が逆転したらBUG-234がこの経路
     だけ静かに再発するため固定する)。(g) `UnsafeToToggle`/`Aborted`等の未送信outcomeでは
     `OwnSend`を更新しないこと(round6 V2、送っていない書き込みが長いcapを受けない)。
     (h) `nonce`が`None`のまま`age >= cap`になった場合は`own_send`を消すだけで「不使用」
     マークは立たないこと、`nonce`が`Some`なら立つこと(round6 V3)。(i) 読んでいるブロックの
     `(tid,pid)`不一致は、全順序の先頭(項目0)で判定され、無関係なスレッドのスタンプを
     消費・消去しないこと(round6 V4-1)。

   **実装状況(2026-11-15、PR #941、`opus-review-pr941` round1の指摘を反映済み)**:
   `classify_own_request`(5-2節「実装後の補正」参照、`ack`/項目1は本PRでは実装しないため
   引数から削除)と`report_mozc_shm`(`platform_state.rs`)への配線、`OwnSend`のスタンプ
   (`hook.rs::note_own_send`/`own_send`/`clear_own_send`、単一スロット)、3か所の呼び出し元
   (`ime_controller.rs`のGjiDirect/MsImeDirect armの`Applied`時、`open_chain.rs::imm_cross_write`
   の`ActuationOutcome::Written`時、`platform.rs::set_ime_open`のspawnしたasyncブロック内で
   実completion後)、`(tid,pid)`の取得元(`WindowsPlatform::mozc_target()`、round1指摘3-gで
   5箇所の重複計算を1関数へ集約)を実装済み。Zの柵拡張(5-4節2)は11節4番のとおり見送り。
   単体テストは(a)(d)(g)を除く(a)はZ見送りのため対象外、(d)(g)はスタンプ側〈成功時のみ
   呼ぶコード構造〉の話なので`classify_own_request`自体のテストにはならず、代わりに
   `architecture_guard.rs::note_own_send_call_sites_are_pinned_to_successful_open_close_writes`
   でスタンプ呼び出し元の件数と「成功時のみ」を示す条件式の存在をソース走査で固定した
   (round1指摘5-c。round2 L3の訂正: 条件式の**分岐の内側**にあることまでは検査しない、件数と
   条件式の存在が目的)。`classify_own_request`のテストは(b)(c)(e)(f)(h)(i)相当+項目2の送信前
   古い読みの非消費(round1指摘1-a、round2 N3で必要条件であり十分条件ではないと訂正)+
   cap境界(5-d-3)+supersede×cap超過(5-d-4)。`report_mozc_shm`レベルではBUG-234を実際に
   再現する統合テスト2本(cap内はHeldのまま・cap超過後は従来判定で追随)、項目2の消費を
   固定する統合テスト1本、drift correctionの処理待ち(desiredと食い違うがeffectiveとは
   一致する読み)がHeldを維持することを固定する統合テスト1本(round2 N2)を追加
   (round1指摘5-a)。ADR-292 P(`key_effect_open_hold`)との相互作用は`judge`が`Held`の理由を
   区別しないため既存の`judge_table`がそのまま固定していることを確認(round1指摘2-b、新規
   テストは追加していない)。`cargo test -p awase-ime-core -p awase-windows -p awase-hook-core
   --lib`・`cargo test -p awase-windows --test architecture_guard`・`cargo nextest run
   --workspace --lib`(3394件)・`cargo clippy --target x86_64-pc-windows-msvc -p
   awase-windows`・`cargo fmt -- --check`で
   確認済み。
2. **核のみのPR(direct-actuation、round1〜3)**: `classify_own_request`(純粋関数、5-2節の
   全順序どおり)と`decide_use(…, own_request)`。`Split`早期returnより前にack処理。BUG-234の
   タイムラインテスト(capまで未ackなら`Held`、ack後は通常判定、`Rejected`は既存の「失われた
   書き込みは諦める」挙動と一致)。既存の`direct_actuation_plan.rs`の`Plan`(再送を含む)はこの
   単純な分類に置き換わる。
   **(2026-11-15訂正、round2 R2-4)**: 実装(`7278191a4`)は`decide_use`の署名を変更せず、
   own-requestのHeld強制を既存の唯一の本番呼び出し元`report_mozc_shm`側に残した
   (L1参照、妥当と判定済み)。`direct_actuation_plan.rs`はこのADRのどの時点でも作られていない。
3. **観測の殻**: `ShmState`と`DirectActuationAck`を1回のseqlock読みから同時に取る関数。
4. **機構の追加**: `WriteMechanism::MozcDirect`(適用可否は`mechanism_applicable`側)、
   `MechanismCommand::DirectActuate(bool)`、`DecisionInputs.mozc_direct_ready`、
   `decide_chain`に`CHAIN_GJI_ONLY`限定のreadiness依存オーバーライドを追加
   (`caps`のdocにSSOTとしてこのオーバーライドの存在を明記、round3 S5)。
   `MAX_WRITE_MECHANISMS=4`(`const _: () = assert!(WriteMechanism::ALL.len() <=
   MAX_WRITE_MECHANISMS)`で以後の追加も固定、round3 S6)・`romaji_pre_write`のarm・
   `sync_actuation_exhaustive.rs`のME-3 golden(`tests/golden/actuation_decision_table.txt`)
   再生成・`may_return_failed`のdocテーブル・`consumes_candidate_evidence`拡張を追随させる。
5. **送信の殻**: `cmd_window_handle`/readinessを`WindowsPlatform`(`focus`と並ぶフィールド)
   経由でviewへ到達させる。HWND所有権確認→`post_command`→
   `AppliedWithoutSendInput`|`Failed`、`note_ime_actuation_send()` +
   `note_mozc_direct_post()`(hubへの直接書き込みはしない)。`[mozc-direct] ack
   nonce=… result=… latency_ms=…`のログを追加する(拒否率の計測、5-3節の着手判断に使う)。
6. **拒否時の収束**: (tid,pid)単位の「フォーカス変更まで不使用」フラグを`WindowsPlatform`
   側のtargetに持たせる(`report_mozc_shm`の戻り値経由、5-2節)。`evaluate_drift`は変更しない。

**本PRに含まない(別タスク)**: mozc-readable fork側のイベント駆動な`kNoFocusedContext`遅延
(5-3節、ステージ5の拒否率データが出たら着手)。

**実装状況(2026-11-15、`exp/mozc-direct-actuation-step2`)**: step2(classify_own_requestの
項目1復元)・step3(観測の殻)・step4+5(機構追加+送信の殻、1コミットに統合——`decide_attempt`
のmatch式の網羅性の都合で分けると中間状態がコンパイルを通らないため)・step6(拒否時の収束)
をすべて実装した。

**`opus-review-step2to6` round1の指摘への対応**: 致命的指摘C1(登録ウィンドウメッセージ名が
forkの実OSSビルド`mozcreadable.direct_actuation.message`と不一致だった)を修正し、
`MOZC_DIRECT_ACTUATION_MESSAGE_NAME`としてconst化・forkのビルド分岐をdocに明記した。M3
(`lints/actuation_call_guard::RESTRICTED_CALLS`と`architecture_guard.rs`に
`apply_mozc_direct`/`PostMessageW(Some(..))`を登録)・M4(`[diagnostics]
mozc_direct_actuation`、既定falseのキルスイッチを追加)・L2(冗長な`mozc_direct_post_tick_ms`
スタンプの撤去)も対応した。M1(nonceを`current_tick_ms()`から再シード)・M2(MozcDirect armに
`gji_direct_already_matches`と同じ省略を追加、自動リピートの連打を抑制。9節R1-iiの判断を
反転・ME-3 golden再生成)・M5(`OwnRequestVerdict`を`ConsumedUnusable`〈一時的な拒否〉と
`ConsumedUnusableNoAck`〈経路が死んでいる〉に分離)も対応したが、round2の再確認で**M1は
修正が実行されない回帰、M5は一部解消のまま**と判定された(下記参照)。軽微な指摘のうちL1
(step2の設計逸脱は妥当と判定済み)はそのまま、L3〜L6(ackログの遅延がほぼ`None`・非同期
チェーンでの到達不能性の固定・doc精度等)は本PRでは対応していない。

**`opus-review-step2to6` round2の再確認で見つかった指摘への対応**: R2-1(M1の修正が実行
されない回帰——`direct_actuation_nonce_counter`の静的初期値が1のままで`compare_exchange(0, …)`
が一度も成立していなかった)を、初期値を0に直して修正。合わせて再シードが実際に効くことを
固定する単体テストを追加した(round1にはテストが無く、この回帰が見過ごされた直接の原因)。
R2-2(M5の「経路が死んでいる」判定が`(tid,pid,cmd_window_handle)`単位の単一スロットで、
窓を行き来すると前の窓のマークが上書きされ付け外しを繰り返す)を、`mozc_direct_dead_for`を
撤去してプロセス全体の回路遮断`WindowsPlatform::mozc_direct_circuit_broken`に置き換えて
修正。R2-3(M5の解除条件が`cmd_window_handle`の変化のみで、Mozcが単に遅かっただけの場合も
永久除外になる新しい固着)を、`hook::last_issued_direct_actuation_nonce`(新設)とforkからの
本物のackが一致したら〈結果を問わず〉解除する方式(`clear_mozc_direct_circuit_broken_if_alive`)
に変えて修正。R2-4(本ADR・BUG-234.mdの古い記述・誤記)はこの編集で反映した。**M2
(`explicit_press.rs`の全列挙モデルへのMozcDirect軸追加)はround2でも一部解消のまま
(モデル未対応)**——R2-5のとおり、既定を有効化する前提条件として11節6番に明記した。
R2-9(L3、ackログの`latency_ms`がほぼ`None`)は今回も対応を見送った。**未解決のまま残る
既知の限界**は11節6番、status frontmatter参照(全列挙モデルの軸未対応・実機/CIでのack
往復未検証)。

## 11. 今後の判断が必要な点

1. fork側の拒否率(`kNoFocusedContext`等)がステージ5のログでどの程度出るか(5-3節の
   イベント駆動遅延の着手判断)。
2. `MOZC_OPEN_CLOSE_SETTLE_MS`の実測値が確定するまで(8節2番)、Standard profile対応は
   「設計は収束したが未解決」の状態である——測定待ちの定数を置いただけでは解決したことに
   ならない(round4レビューの指摘、T7-4)。実測が終わり、`#[measured(pending=true)]`を
   外せた時点で解決とする。**2026-11-15時点**: CI(遅延注入、8節2番)で機構自体の動作と
   170ms境界での再現率変化は確認済みだが、これは注入遅延であり、dragonflyg4実機でMozcが
   自然に示すcold/warmの遅延分布の実測はまだ残っている——実装(10節)は
   `#[measured(pending=true)]`のプレースホルダーで進め、この実測が終わってから外す。
   `opus-review-pr941` round1指摘6: この2000msは[BUG-230](../known-bugs/BUG-230.md)の
   「モード切替(F2)+500ms周期読みの観測上界」1件にマージン0を付けた値で、
   `TURN_OFF_IME`/`TURN_ON_IME`(開閉)自体の完了時間ではない——`tuning.rs`のdocにこの精度の
   注記を追加済み(5-2節「実装後の補正」参照)。
3. T6-2(物理モードキー押下と同じpressのshadow-toggle送信が重なった場合の訂正速度)は
   round5(5-2節項目3の`own_send.tick_ms >= last_mode_key_tick_ms`、supersession規則)で
   **確定済み**——round6(V1)でtie-break規則を訂正: 両方のtickは同じ`hook::current_tick_ms()`
   (分解能~15.6ms)由来で、同じpressのshadow-toggle送信とその予測は同値になりやすく、
   `>=`なので**同値なら送信が勝つ**(awaseが実際に送信したことを優先)。後続の物理モードキー
   予測が170msで訂正されるのは、その予測のtickが送信のtickより**真に後**の場合のみ。
   10節1番のテストで固定する(実装時の選択ではない)。
4. Zの`Explicit claim`fence拡張(5-4節2)をこのPRに含めるか、CIの遅延注入実験の結果が出るまで
   観測専用(ログのみ)に留めるか(round5 U2のスコープ留保)。**2026-11-15、見送りで確定**:
   `OwnSend`をZへ渡す経路が、全`ImeEvent`を駆動する汎用`dispatch_event`/`scope_event_of`を
   通るため、この1件のためだけに多数の無関係な呼び出し元へ配線を広げることになり、出荷条件
   (SendVkフォールバック経路が実際に踏まれる証拠)もまだ無い。
5. **既知の残る限界(2026-11-15、`opus-review-pr941` round1指摘3-c・3-f・round2指摘L2、
   未修正・受容)**: 詳細は5-2節「実装後の補正」参照。(a) `imm_cross_write`の発行から書き込み
   成功までの間(`.await`中)の食い違う読みは保護されない。(b) 単一スロットのため、Mozc窓への
   送信後すぐ別窓(MS-IME等)へ何か送信すると、前者の保護が後者の送信で上書きされ消える——
   「v6ブロックが実在するスレッドだけスタンプする」制約が未実装。(c) `set_ime_open`・
   `imm_cross_write`のスタンプは書き込みの完了時に打つため、単一スロットの上書き順は発行順
   ではなく完了順になる——drift correctionのImmCross(遅い)を発行した直後に明示操作の
   GjiDirect(同期)が逆方向をスタンプすると、後から完了したdrift correctionが古い方向で
   上書きする(round2 N2の修正後はbelief/desiredと一致する読みが通るため害は限定的)。
   `own_send`のPending中にMozcShm観測が失効して他の証拠源に反転しないか(round2 N1)も
   未検証。(d)(round3レビュー指摘R1)round2 N2の修正で、`Pending`は「送信前からの古い
   一致読み」(項目2を満たさない、`update_tick_ms <= send.tick_ms`)でも、その読みが
   `own_send.open`と一致していても`desired`とは食い違っていれば保留するようになった——
   `last_mode_key_tick_ms`を進めない経路(`pass_through_observed`による揃え直し・字形の観測
   による追随等)でdesiredが送信後に変わった稀な組み合わせでは、修正前なら`decide_use`が
   desiredへ揃え直していたのが、修正後は最長capまでHeldになる。capの受容済みのコスト
   (最長2秒の保留)の範囲内。いずれも実害が報告された時点で着手する。
6. **`explicit_press.rs`の全列挙モデル(`PressState`)が`mozc_direct_ready`の軸を持たない**
   (`opus-review-step2to6` M2、9節R1-iiの訂正と対)。`attempt_write`・
   `chain_head_is_gji_direct`・`chain_head_is_imm_cross`は`mozc_direct_ready: false`を
   ハードコードしており、実際のチェーンが`CHAIN_GJI_ONLY`から`CHAIN_MOZC_DIRECT_THEN_GJI`
   (先頭`MozcDirect`)へ差し替わる場合を一度も列挙していない。`decide_attempt`の
   MozcDirect armに追加したalready-matched省略(M2)は`decide_attempt`単体のユニット
   テストでは確認済みだが、ADR-208 L1の不変条件(同一押下で二重送信しない・リピートは
   追い送りしない)がMozcDirect先頭のチェーン上でも成り立つことを、2000行超のこの
   全列挙モデルでは未検証のまま出荷する。モデルへの軸追加は非自明な拡張(既存のP1〜P6
   不変条件それぞれへの影響を洗い出す必要がある)のため、別タスクとして次セッションで
   着手する。

   **`[diagnostics] mozc_direct_actuation`キルスイッチの既定をtrueへ切り替える前提条件
   (round2 R2-5/R2-8、2026-11-15追記)**: M4で既定falseになったため、この軸追加を
   このPRでは見送ってよいと判定されたが、**既定を有効化する前には**以下の2点を満たすこと:
   1. 本項目のとおり、`explicit_press.rs`の全列挙モデルにMozcDirect軸を追加し、
      ADR-208 L1の不変条件(P1〜P6)がMozcDirect先頭のチェーン上でも成り立つことを検証する。
   2. `ci/directact-precheck`相当のworkflowで、awase本体の実際の送信経路
      (`ime_controller.rs::apply_mozc_direct`)からmozc-readable fork実機へ`PostMessageW`し、
      ack `applied`が実際に返ることを確認する——C1修正後も、awase本体の送信経路からの
      ack往復はCIでも実機でも一度も通していない(round1・round2のレビューが繰り返し
      指摘した未検証点、R2-8)。登録メッセージ名が将来再びずれても検出する機械的な手段は
      無いため、この確認を既定有効化の都度(少なくともforkのバージョン更新時に)再実行する
      運用も合わせて検討する。
