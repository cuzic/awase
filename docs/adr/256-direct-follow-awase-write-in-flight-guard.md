---
id: ADR-256
title: |-
  直接観測の窓(ADR-188)で、まだ IME に届いていない awase 自身の開閉の書き込みを、窓内の古い読みで覆さない(BUG-199)
summary: |-
  BUG-199: 物理 Ctrl+無変換(awase の OFF の書き込み)の直後に予測の無いモードキー(F18 等)を押すと、直接観測の窓(ADR-188、経路B)が開く。
  GJI が awase の OFF を処理するのが窓(300ms)より遅いと、窓内の読みは処理前の「開」を返し、follow_direct_read_in_scope が意図を捨てて
  belief を ON に戻す。閉じた IME へローマ字が送られ、give-up で打った文字が消える(CI 実 Chrome 相当 × GJI、変換サーバ 400ms 停止で 25/30)。
  R3(`last_explicit_ime_action_ms >= armed_at`)は arm 以後の書き込みしか守らない(awase の書き込みは arm の 78〜94ms 前)。
  決定: SendInput で送った awase の開閉の書き込み(`ImeOpenOutcome::Applied`)を監視窓の状態に「未確認の書き込み」として持ち、
  直接観測の窓を arm した時点でまだ届いていない(書き込みから arm まで窓の長さ未満)なら窓に引き継ぐ。窓内で書き込みと同じ値を読んだら解く、窓が切れたら消える。
  引き継いだ書き込みが未確認の間は窓内の読みを採らない。新しい時間定数は足さない(既存の窓の長さを「IME に届くまでの想定」として共用する)。
  経路A(通過マーク、ImmCross の読める窓)は変えない。
status: |-
  保留・所有者判断待ち(2026-10-11)。本件(25/30→0/30)と回帰セット 69 構成の不変は確認したが、逆向きの筋書き(sc-fkeyrev-ctrl-tsf-susp)で
  副作用(IME 開・Engine OFF のまま、修正前 26/30 正→修正後 0/30)を CI で確認した。PR は作っていない。
related_adr:
  - "ADR-188"
  - "ADR-187"
  - "ADR-205"
  - "ADR-244"
  - "ADR-247"
---

# ADR-256: 直接観測の窓で、届く前の awase 自身の書き込みを古い読みで覆さない(BUG-199)

## 事実(`origin/develop` `a693540c` 時点)

- 再現: 使い捨てブランチ `ci/adr247-fkey-intent-ab`(run 38116243509 / 38116609782)の `sc-fkeyint-ctrl-tsf-gji-f18-susp`。
  typing_stress の tsf 窓(`Chrome_RenderWidgetHostHWND`、`Imm32Unavailable`)× GJI で、物理 Ctrl+無変換(= `keys.ime_off`、awase が GjiDirect で OFF を SendInput)の
  直後に役割の無い F18 を押し、GJI の変換サーバを OFF の直前から 400ms 止める。F18 ありで 25/30 試行の文字が消え、F18 なしの対照は 0/20。
- 機序(ログ `result-sc-fkeyint-ctrl-tsf-gji-f18-susp-1`): `[apply-ime] open=false → outcome=Applied`(52.956)→ F18 で直接観測の窓を arm(53.046、書き込みの 90ms 後)
  → 窓内の読みは変換サーバの停止中は時間切れ(`ime_on=None`)、53.318 に `ime_on=Some(true)`(処理前の古い値)→ `follow_direct_read_in_scope` が
  `ObserverPoll(true)`・意図の削除・`ModeKeyPassedThrough{align_desired}` で belief を ON に戻す → Engine がローマ字を閉じた IME へ送る。
- R3(ADR-188 追記4)は `last_explicit_ime_action_ms >= armed_at` だけで、arm より前の書き込みは守らない(本件は arm の 78〜94ms 前)。
- 読めない窓では、明示意図がある間は定期の読みが止まる(同ログ: 46.97→50.74 の 3.8 秒、読みなし)。書き込みの直後に読むのは apply 完了の 20ms 後の 1 回だけで、
  GJI の反映は中央値 33ms(ADR-187 の実測、最大 62ms)なので、この 1 回が処理前の値を読むことは普通に起きる。
- 経路A(通過マーク、`ModeKeyPassLatch::drop_decision`)は、読める窓(ImmCross、OsPoll)だけで働く。awase の書き込みは `ImmSetOpenStatus`(`AppliedWithoutSendInput`)で同期的に
  IMM の状態を変えるので、窓内の読みが書き込み前の値を返す経路が無い(同じ再現で edit 窓は閉を読み、0 件)。

## 決定

1. **未確認の書き込み**: `ExternalChangeWatch` に `pending_write: Option<(scope, open, at_ms)>` を持つ。`record_ime_apply_result_in_scope` で
   `outcome == ImeOpenOutcome::Applied`(SendInput で送った=届くのが遅れうる書き込み)のとき記録する。ImmCross(`AppliedWithoutSendInput`)・
   `AlreadyMatched`・`Failed` 等は記録しない。新しい書き込みは古いものを置き換える。
2. **確認**: prefetch 済みの読みが来るたび(`follow_direct_read_in_scope` の先頭、窓の有無によらない)、同じスコープで書き込みと同じ値なら解く。
   別のスコープの読みが来たら捨てる(フォーカスが移った)。読めなかった(`None`)ときは何もしない。
3. **引き継ぎ(1 回限り)**: 直接観測の窓を arm(延長を含む)するとき、同じスコープの未確認の書き込みがあれば取り出し、
   **書き込みから arm まで窓の長さ(`MODE_KEY_PASS_MARK_WINDOW_MS`)未満なら**窓の `awase_write` に移す。古いものは捨てる。外部注入の監視窓(ADR-205、基準値あり)の arm では取り出さない。
4. **保持**: `follow_direct_read_in_scope` は、R3 の後・予測のガードの前で、窓の `awase_write` が残っている(=窓内でまだ同じ値を読んでいない)なら採らない(両軸とも、R3 と同じ扱い)。
   窓内で書き込みと同じ値を読んだら `awase_write` を解き、以後は従来どおり照合する。窓が切れたら窓ごと消える。
5. R3(arm 以後の書き込みは窓の残りを採らない)はそのまま残す。窓の延長で `armed_at` が書き込みより後ろへずれた場合も、延長の arm で 3. が拾うので守られる。
6. 経路A(`drop_decision`・`align_after_expired`)は変えない。
7. フォーカスが移ったら(`FocusChanged`・同じプロセス内の `FocusHwndUpdated`)控えと引き継いだ書き込みを捨てる(レビュー S3)。
8. 計測(挙動は変えない): 保持しなければ belief を動かしていた読み(予測のガードが捨てず、判定が追随する読み)を採らなかったときだけ
   `[direct-follow] awase-write held` を info で出し、それ以外は debug。引き継いだ書き込みが未確認のまま窓が切れたら
   `awase-write expired unconfirmed ... effective_holds=N` を info で出す(レビュー S1・M1)。

判定は純関数 `direct_read_held_by_awase_write(carried, read)` と `ExternalChangeWatch` のメソッド(awase-windows-core、Linux で単体テスト)に置き、
ハブ(`platform_state.rs`)は記録・確認・保持の 3 点を呼ぶだけ。belief への書き込みは増やさない(保持は「採らない」だけ)。

## なぜ「窓の長さ以内」の上限が要るか(時間定数なしの案を採らない理由)

上限なしで「未確認なら何秒前の書き込みでも引き継ぐ」(案A0)にすると、明示意図がある間は読みが止まるため、apply 直後の 1 回の読みが処理前の値だった書き込みが
何十秒でも「未確認」のまま残る。その後、利用者が予測の無いモードキー(Shift 付き・F13〜F24・表に無いセル)で書き込みと逆向きに開閉を変えると、
窓の全体で読みが採られず、ADR-188 が直した BUG-149/150 型(IME と Engine の食い違い)が戻る。上限を新しい時間定数 X にする(案B)と実測義務と
複雑性予算に反するうえ、X の根拠は「awase の書き込みが IME に届くまでの時間」で、これは窓の長さ(`MODE_KEY_PASS_MARK_WINDOW_MS` = 300ms、
「通過したキーの効果が読みに現れるまでの時間」の実測最大 62ms の約 5 倍)が既に置いている想定と同じものである。同じ想定を共用し、値は変えない。
本件の再現は書き込みから arm まで 78〜94ms で、この範囲に入る。

なお上限が縛るのは「書き込み → arm」の間隔だけで、保持そのものは arm から窓の終わりまで続く(書き込みからは最大 約 2×窓、延長すれば最初の arm から最大 3×窓)。
再現でも、採らなかった「開」の読み(53.318)は書き込み(52.956)の 362ms 後だった。効いているのは「窓の全体で、同じ値を読むまで保持する」ことで、
「awase の書き込みは 300ms 以内に届く」という主張ではない(レビュー S2)。

## 比べた案

| 案 | 内容 | 直るもの | 壊れうるもの | 変更の大きさ |
| --- | --- | --- | --- | --- |
| **A1(採用)** | 未確認の SendInput 書き込みを窓へ 1 回だけ引き継ぐ(書き込みから窓の長さ以内)。一致で解除・窓切れで消える | 本件(arm 前の書き込み)。窓の延長で R3 から外れた書き込みも | 書き込みの直後(300ms 以内・未確認)に利用者の予測の無いキーが逆向きに開閉を変えると、その窓は追随しない(従来は追随) | core のフィールド 2・純関数 1・メソッド 3、ハブ 3 行 |
| A0 | A1 から「窓の長さ以内」を外す(時間に依存しない) | A1 と同じ | 読みが止まっている間に残った古い未確認の書き込みが、何十秒後の窓でも逆向きの追随を止める(上の節) | A1 と同程度 |
| B | R3 を `last_explicit >= armed_at - X` に広げる(新定数 X) | 本件 | 確認済みの書き込みでも窓の全体を止める。X の実測義務・定数が 1 つ増える | 1 行+定数 |
| C | 窓内では、awase の最後の書き込みと食い違う読みを常に採らない(確認・上限なし) | 本件 | 届かなかった書き込みがあると、以後の窓がずっと追随しない | 小 |
| D | 経路A(`drop_decision`)にも同じ保持を入れる | (再現なし) | ImmCross は同期なので利点が無く、保持中に意図が残ると読める窓の drift correction が利用者のモードキーを書き戻しうる | 読みの値を `drop_decision` へ渡す配線 |

## 他の決定との関係

- ADR-188 案2(PR #601、送り直した親指の効果を予測に重ねる)・予測のガード: 保持は予測のガードの前に置き、どちらも「採らない」側なので干渉しない。
- ADR-244 D4(MS-IME 本体は「閉→開」だけ): 判定(`classify_direct_read_for`)は変えない。MS-IME 本体で awase が閉じた直後の古い「開」の読みも同じく保持する。
- BUG-157/158(通過マークの揃え、`align_after_expired`): 経路Aは変えないので影響しない。
- ADR-205(外部注入の監視窓、基準値あり): 引き継ぎをしない(arm で未確認の書き込みを取り出さない)。

## 検証(計画)

- 単体: `direct_read_held_by_awase_write` の表(引き継ぎなし・一致・逆の値・`None`)、`ExternalChangeWatch` の記録・確認・引き継ぎ・古い書き込み・スコープ違い・延長・窓切れ、
  ハブ経由(arm 前の Applied の書き込み → 窓内の古い読みで意図が残る/一致の後は追随する/ImmCross の書き込みは引き継がない)、`architecture_guard` で配線を固定。
- CI 実機(修正前 = develop 先端と同じ構成・回数で A/B): `sc-fkeyint-ctrl-*`(F18 あり/なし・停止あり/なし)、BUG-157/158 と ADR-188/244/247 の回帰
  (`sc-f16-*`、`sc-bug149-chrome-*`、`sc-adr211-*`、`sc-t1e-*`、`sc-adr209-chrome-msime*`、`sc-follow-chrome-msime-henkan`、`sc-bug186-*`、`sc-armc-*`、`sc-b254-*`、`sc-driftrecovery-*`、smoke、既定 Suppress)。
  保持・解除の件数を `[direct-follow]` のログで数える。

## 既知の限界

- **保持で窓の全体を採らなかった後、belief を戻す経路が無い(レビュー M1)**。(a) awase の書き込みから 300ms 未満に、利用者が予測の無いキー
  (Shift 付き・予測の無い F13〜F24・表に無いセル)で書き込みと逆向きに開閉を変え、窓内で書き込みと同じ値を一度も読まなかった場合、
  (b) awase の書き込みが IME に届かなかった場合。保持は明示意図を残し、読めない窓では明示意図がある間は定期の読みが止まるので、
  食い違いは次のモードキー(または GJI I/O の推測)まで続き、その間 drift correction が意図どおりに書き戻しうる。予測の無いキーでは
  「利用者のキーと awase の書き込みのどちらが後か」を読みから区別する手段が無いため、限界として頻度を測る:
  `awase-write expired unconfirmed ... effective_holds=N`(N>0 が該当)と、逆向きの CI 構成 `sc-fkeyrev-ctrl-tsf(-susp)`(検証用ブランチのみ)。
  結果は下の「検証の結果」。
- drift correction の Blind の再送も `Applied` なので控えを新しくする(レビュー S4)。再送の直後 300ms 未満に利用者が予測の無いキーで逆向きに
  変えた窓は上の (a) と同じになる。drift の書き込みだけを控えから外すことはしない(届くのが遅れうるのは同じで、外すと drift の再送の直後に
  本件と同じ形で覆されうる)。頻度は上のログで数える。
- 英数の軸の同じ型の穴(左 Shift の半角英数トグルの `VK_DBE_ALPHANUMERIC` など conv の書き込みは `record_ime_apply_result` を通らない)は対象外
  (レビュー S5)。保持は開閉が未確認の間、英数の軸の追随も止める(両軸とも採らない)。
- 経路A で ImmCross が失敗して GjiDirect にフォールバックした書き込み(`Applied`)は、読める窓では窓内の読みを止めない(案D を採らない、レビュー S6)。
- `ForegroundScope::INVALID` どうしは等しいので、前景窓が取れない間の書き込みと arm は互いに引き継ぐ(レビュー N3、実害はほぼ無い)。
- 完了が書き込みの順に届くことを前提にする(直接観測の窓では async の open chain は走らない、レビュー N2)。

## 検証の結果(2026-10-11、検証用ブランチ `ci/verify-bug199*`、PR には載せない)

修正前 = origin/develop `a693540c` + 同じ検証用コミット(`ci/verify-bug199-base` / `-fk-base`)、修正後 = `61f570b4` + 同じ検証用コミット。

| 構成 | 修正前 | 修正後 | run |
| --- | --- | --- | --- |
| `sc-fkeyint-ctrl-tsf-gji-f18-susp`(本件) | 文字消失 26/30・25/30 | 0/30・0/30 | 38117845322・38118558630 / 38117843771・38118557096 |
| `sc-fkeyint-ctrl-tsf-gji-nof18-susp`(対照) | 0/30 | 0/30 | 同上 |
| `sc-fkeyint-ctrl-{tsf-gji-f18,tsf-gji-nof18,edit-gji-*-susp,chrome-gji-*}` | 消失 0・結果同じ | 同じ | 38117845322 / 38117843771 |
| 回帰 69 構成 81 ジョブ(`sc-f16-*`・`sc-bug149-chrome-*`・`sc-adr211-*`・`sc-t1e-*`・`sc-adr209-chrome-msime*`・`sc-follow-chrome-msime-henkan`・`sc-bug186-*`・`sc-armc-*`・`sc-b254-*`・`sc-driftrecovery-*`・`atok-passthrough-cold`・`baseline`) | — | PASS/FAIL/INVALID の集合が全構成で同じ(失敗構成はセル単位でも同じ)。保持・引き継ぎのログ 0 件 | 38117848228 / 38118555551 |
| **`sc-fkeyrev-ctrl-tsf-susp`(逆向き、M1)** | 打鍵結果「か」(IME 開・Engine ON)26/30 | **「ｗ」(IME 開・Engine OFF)30/30** | 38119018504 / 38119016900 |
| `sc-fkeyrev-ctrl-tsf`(逆向き、停止なし) | 「か」30/30 | 「か」30/30(arm 前に読みで確認され引き継がない) | 同上 |

- 本件の窓では 30 窓中 29〜30 で書き込みを引き継ぎ、窓内で確認できたのは 4〜11 窓。修正後に直接観測の追随で意図を捨てた回は 0。
- 逆向き+停止では、変換サーバが窓の後に awase の OFF と利用者の F17(IME 有効化)を続けて処理し、窓内で「閉」を一度も読まない窓が多く、
  保持が利用者の操作を窓の全体で採らなかった。その後に belief を戻す経路が無く、2 秒後の打鍵も「ｗ」(上の M1 の限界がそのまま出た)。
- `awase-write expired unconfirmed` のログは 0 件だった: 意図が残る間は読めない窓の定期の読みが止まり、窓の後に follow の tick が来ない。M1 の頻度はこのログでは数えられない。
- 結論: 本件と逆向きの筋書きは読みの列から区別できず(どちらも「書き込みと逆の値」が窓の間続く)、A1 は本件を直す代わりに逆向きの筋書きを壊す。採否は所有者判断。
