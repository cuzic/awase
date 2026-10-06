---
id: ADR-238
title: |-
  一過性の conv=0 を 1 回の読みで ObservedEisu と採用して Engine が止まる件(BUG-190)— 英数モードの採用に確認を足す
summary: |-
  BUG-190: MS-IME の OS ポーリングが一瞬 `conv=0x00000000`(`romaji=None`)を返すと、`classify_ime_snapshot` が 1 回の観測だけで `InputModeObserved(ObservedEisu)` を belief に書き、`Inactive(NotRomajiInput)` で Engine が 0.08〜3.2s 止まり、その間の打鍵が生のまま IME に渡る。手元の CI 約 600 本(MS-IME 約 300・GJI 約 300)で conv=0 の poll は 5 件、すべて MS-IME・前後の poll が 0x19 の孤立した 1 回・5 件すべて採用された(GJI は 0)。
  5 件は 2 種類の機序: (i) 変換/無変換を MS-IME に通した 123〜263ms 後の「モードキー通過の読み直し」(打鍵中の除外を外した読み)で読んだ一過性の値(Flutter 3、BUG-189 の修正前の run)、(ii) 相手の IME 窓の応答が時間切れ寸前のときに読んだ値(wx・Java、conv プローブが 49.5ms 等)。
  決定(案、Opus レビューで収束させる): ObservedEisu の採用に確認を足す。確認は `is_eisu_evidence` の分岐ではなく**結果(`new_input_mode == Some(ObservedEisu)`)に掛ける**(`classify_transition` の英数遷移も同じ件を拾うため)。案: F(採用の前に短い間隔で確認の読み直しを予約)・G(reducer に英数候補の状態を持たせる)・A(前回の conv が英数のときだけ)・B(時間切れ/`ime_on=None` の読みでは採らない。(ii) の補助)。C(MS-IME は open=false)は前提が否定された。
status: |-
  起草中(2026-10-06)。Opus round1(Blocker 2・Must 5・Should 7・Nit 6)反映済み、round2 待ち。実装は未着手。
related_adr:
  - "ADR-074"
  - "ADR-084"
  - "ADR-186"
  - "ADR-191"
  - "ADR-233"
---

# ADR-238: 一過性の conv=0 を 1 回の読みで ObservedEisu と採用する件

## 背景(BUG-190、CI で再現)

[BUG-190](../known-bugs/BUG-190.md)。MS-IME で、打鍵の途中(または最初の試行)に出力が全角英字混じりに崩れる低頻度の症状。BUG-189(ADR-233、`most_recent_trusted` の順位キー)の修正後も残る別の機序。

### 機序(ログで裏取り)

1. OS ポーリング(`ime.rs::read_ime_state_full`)が一過性に `romaji=None conv=Some("0x00000000")` を返す(直前・直後の poll は `romaji=Some(true) conv=0x00000019`)。`romaji=None` は独立した観測ではなく、`ime.rs:590-596` が NATIVE ビットの無い conv から機械的に作っている。
2. `observer/ime_observer.rs::classify_ime_snapshot` が **1 回の観測だけで** ObservedEisu を返す。**ObservedEisu を作る箇所は同じ関数に 2 つある**:
   - (a) 164〜172 行: `ConvMode::is_eisu_evidence(snap.ime_on, snap.conversion_mode) == Some(true)`。`ime_on == Some(false)` のときだけ conv=0 を無視する(BUG-57)。
   - (b) 173 行以降の else 側: `input_mode_from_romaji_flag` が `romaji=None` で `None` を返したあと、`input_mode_from_conversion` → `ConvMode::classify_transition`(`src/engine/conv.rs:141`)の 149 行 `if self.is_eisu() && !prev.is_eisu() { return Some(ObservedEisu); }` が、**前回 0x19・今回 0 という孤立した 1 回の形そのもの**で ObservedEisu を返す。(a) だけを弾いても 5 件とも同じ結果になる。
3. `input_mode=ObservedEisu` → `Inactive(NotRomajiInput)` で Engine が止まる。回復は次の読み(`conv=0x19` → `input_mode_from_romaji_flag` が ObservedRomaji に戻す。5 件とも `IME input method changed: kana → romaji` で戻っており、stale recovery〈`ime_observer.rs:177-199`、`romaji=None` かつ conv が英数でないときだけ届く〉ではない)。
4. 停止時間は 0.079/0.094/0.58/1.02/3.22s。次の読みが来るまでの間隔に依存する: 通過マークの窓の中は 60ms 間隔、通常のタイマーは 500ms、打鍵中(`TYPING_IDLE_MS`=500、`tuning.rs:13`)は SkipTyping が読みを捨て続けるので上限が無い。5 件の停止時間は SkipTyping 0/0/1/1/5 回と対応し、wx の 3.22s は SkipTyping が 5 回続いたため。

### 5 件は 2 種類の機序

| 件 | 機序 | 根拠 |
| --- | --- | --- |
| Flutter 3(BUG-189 の修正前の run) | **(i) モードキー通過の読み直しの窓で読んだ一過性の値** | 3 件とも conv=0 の読みの直前に `[mode-key-follow] mode key PassThrough(vk=0x1C\|0x1D): IME refresh scheduled`。読みは打鍵中の除外を外した読み直し(`Explicit intent: bypassing typing-idle guard for IME verify`、`ime_refresh.rs:217-231`、`MODE_KEY_PASS_REREAD_MS`=60ms、窓 300ms)。通過から +263ms(Flutter-8)・+165ms(Flutter-1)・+123ms(Flutter-5)。直前の belief は閉で、親指キーとしての変換/無変換が `PassThrough` で OS へ通っていた(BUG-189 の機序の後) |
| wx 1・Java AWT 1 | **(ii) 相手の IME 窓の応答が遅いときに読んだ値** | wx: open プローブ 50086µs(時間切れ → `ime_on=None`)・conv プローブ **49512µs**(時間切れの間際に成功扱いで 0 が返った)。前後の poll 4 回も両プローブ約 50ms。Java: 直前 2 回の poll で両プローブが時間切れ、conv=0 の回は open が 16029µs(平常 60〜100µs)で、同時刻に `classify_focus timed out` |

- **修正後(BUG-189 の後)の run で起きたのは (ii) の wx 1 件だけ**(修正後の MS-IME は 21 本〈Flutter 8・LibreOffice 6・wx 4・WinForms 2・Qt 1〉)。Java は BUG-189 の修正前の run。(i) が修正後も起きるかは未確認。**(i) と (ii) で効く対策が違う**(下記)。
- 以前の「変換キーとの関係は確認できていない」「直前は文字キー」は誤り(`key-effect-predict` は文字キーにも出るので、直前のモードキーは `mode key PassThrough` の行で見る)。

### 発生状況(手元の CI ログ、`IME snapshot` 行のある run)

| IME | run 数 | conv=0 の poll | 孤立(前後が 0x19) | 採用 → NotRomajiInput |
| --- | --- | --- | --- | --- |
| MS-IME | 約 300 | 5(`ime_on=Some(true)` 4・`None` 1) | 5 | 5 |
| GJI | 約 300 | 0(LibreOffice の別 artifact に 21 件あるが、すべて `ime_on=Some(false)` で BUG-57 の除外が正しく効いている) | - | - |

本物の半角英数への切替(ユーザーが英数キーを押す)は、これらの run には含まれない。

## 決めること

ObservedEisu の採用に、一過性の読みを弾く確認を足すか。足すなら何に、どう掛けるか。

## 事実の確認(案 C の前提の否定)

「MS-IME の半角英数は open=false」は**否定される**。awase 自身が MS-IME に open のまま conv=0x0000 を書く(`state/conv_mode.rs:382-386` `ConvModeTarget::HalfWidthAlnum`「conv=0x0000(IME-ON 半角英数)」、ADR-084 P1、`half_width_alnum.rs:240` `MsImeOnly`)。`docs/experiments.md` 108・112 行(`VK_DBE_ALPHANUMERIC` は「半角英数(IME ON)」)、ADR-074:36、`src/engine/conv.rs:58-59`(「MS-IME は半角英数モードでも ROMAN ビットをセットしたまま返す場合がある」)、BUG-015(MS-IME の Shift 単独タップで IME は開いたまま英数)。したがって MS-IME で open=true・conv=0 は**正当な状態**で、conv=0 を一律に弾く案は本物の切替を落とす。MS-IME 本体が英数キー・Shift 単独タップ・Ctrl+無変換で返す値が 0x10 か 0x00 かは未測定(C'〈全ビット 0 だけを疑う〉の前提、測定は後述)。

## 案

確認は**結果側**に掛ける(B1): `new_input_mode == Some(ObservedEisu)` という結果に対するフィルタにする。(a) と (b) の両方、および他の ObservedEisu 生成箇所(下記「適用範囲」)が同じ条件に揃う。

| 案 | 内容 | 評価 |
| --- | --- | --- |
| **F** | ObservedEisu を採る前に(または採った直後に)、短い間隔で確認の読み直しを予約する。読み直しは `decide_read_strategy` の `mode_key_pass_live` と同様に打鍵中の除外を外す。確認できなければ(0x19 に戻った)採らない/元に戻す | 誤採用の停止が数百 ms〜3.2s から約 60〜100ms に縮む。A の確認待ちの上限の無さ(M2)も解消。belief の意味は変えない。追加の IME I/O は小さい(読み取り自体は prefetch で毎回走る)。**注意:** 一過性の値が 60ms を超えて続く場合がある(Flutter の前後の読みの間隔 138〜174ms)ので、確認の間隔と回数は「一過性の値の長さ」の測定で決める(tuning-constants.md) |
| G | 「英数の候補」(時刻と conv)を `state/ime_model.rs` の reduce に持たせる。2 回目の英数観測が一定時間以上あとに来たら確定、予測の mode が英数と一致したら即確定 | `prev_conversion_mode` が古い/None/ImmCrossProbe が書かない、の問題を避ける。belief を書く場所は reducer 1 つのまま。F より状態が増える |
| A | 前回の poll の conv(`current_prev_conversion_mode`)も英数のときだけ採用 | **単独では不十分:** (1) `prev_conversion_mode` は SkipTyping・ImmCrossProbe では更新されない(`platform_state.rs:1290-1292` は信頼できるスキップされない OsPoll のみ、`key_pipeline.rs:2865-2885` の ImmCrossProbe は `new_prev_conversion_mode` を捨てる)。(2) 遅れは約 0.5s でなく**打鍵が続く限り上限が無い**(wx は SkipTyping が 5 回続いて 3.2s)。(3) 前回が None(フォーカス直後等)の扱いが決まらない(None なら採らないと、フォーカス直後の本物の英数欄〈WinForms `ImeMode.Alpha` 等〉を採らない)。(4) 60ms 間隔の 2 回の読みが両方一過性の値を読むことがある。(5) `ObservedEisu` の予測の無い切替(MS-IME 自身の Shift 単独タップ〈BUG-015〉、言語バー・マウス、アプリの `ImmSetConversionStatus`、古い表)だけが遅れの代償を受ける |
| B | 時間切れの読み(`snap.probe_timed_out`)または `ime_on=None` の読みでは、conv を「不明」として英数を採らない | **(ii) の補助として入れる。** poll の経路で「`ime_on=None` かつ conv=Some」になるのは、open プローブが失敗/時間切れで conv だけが成功したちぐはぐな読みだけ(TsfNative は `read_ime_state_full` が早期 return で conv=None)。wx が直る。(i) は直らない(5 件中 4 件が `ime_on=Some(true)`) |
| C | MS-IME では open 中の conv=0 を英数と見なさない | **却下**(上記、前提の否定) |
| C' | MS-IME で「全ビット 0」だけを疑い、0x10/0x18 は採る | 筋はあるが前提が未確認 |
| D | 直近 N ms の打鍵・モードキー処理中は採らない | **却下寄り:** (i) は「モードキーを通した直後の読み直しの窓」で起きるが、本物の英数キーの検出もこの窓の読みに頼る。D は本物の切替を正面から落とす。既存の `reconcile_key_effect_mode`(`ime_model.rs:720-744`、`KEY_EFFECT_SETTLE_MS`=170)は予測に mode があるときだけ働く(Flutter の 3 件は `mode=None` で素通り)。最小の形は「通過マークの窓の中で、予測が mode を持たない場合は英数を確認つきにする」で、これは F/G の適用範囲の絞り方として使う |
| E | belief は書くが Engine に反映しない | 却下: Engine が `build_ctx` 経由で belief の input_mode を読む構造に、Engine 側にもう 1 つの真実を作る。E を採るなら G の形(reducer の中の正規の状態)にする |

**推奨(案、Opus で決める):** F(確認の読み直し)+ B(時間切れ/`ime_on=None` の読みでは採らない)。G は F で足りない場合の代案。A は単独では採らない。

## 適用範囲

ObservedEisu(または同じ効果)を書く経路:

1. `classify_ime_snapshot` の (a) is_eisu_evidence と (b) `classify_transition`(B1)。
2. 呼び出し元: OsPoll(`ir_poll_and_learn`。同期の `poll_and_classify_ime` と prefetch した `classify_fetched_snapshot` の 2 形)と ImmCrossProbe(`key_pipeline.rs:2868`)。**FocusProbe は `classify_fetched_snapshot` を呼ばない**(open だけを `write_focus_probe` で書く)。
3. TsfNative の idle-conv-check(`key_pipeline.rs:773-821`、`classify_conv_transition` → `ConvMode::classify_idle`、conv が英数なら 1 回の読みで ObservedEisu、`ConvBitsInference`/High)。MS-IME の TsfNative 窓でも同じ「1 回で採用」がある。**今回の範囲に入れるかを決める**(本 ADR の事例はすべて Standard/ImmCross の OsPoll。入れるなら別 PR)。
4. `KeyEffectPredicted`(予測)、`InputModeApplied{UserHalfWidthAlnumToggle}`(awase 自身のトグル)は確認の対象外(予測・自分の書き込みは確認済みの意図)。

BUG-57 の守り(`ime_on == Some(false)` の conv=0 は証拠にしない)は (a) にしか掛かっていない。`ime_on=Some(false)`・prev=0x19・conv=0 の読みは (b) の `classify_transition` で ObservedEisu になる(手元のログでは起きていない)。結果側のフィルタは、この経路も同じ条件にそろえる。

## 回帰テスト(fix-requires-evidence の (a))

`observer/` は `#[cfg(windows)]` なので Linux の `cargo test` には存在しない(CLAUDE.md)。確認の判定を**純粋関数として `src/engine/conv.rs`(ルート crate)か `crates/awase-windows/src/state/`(cfg の無いモジュール)に置き**、そこでテストする。(b) の `classify_transition` の件もこのテストで固定する。journal リプレイには「conv=0 が 1 回挟まる poll 列」(旧は Eisu・新は Romaji のまま)を足す。

## 実装前の測定

1. **診断ログ `[eisu-adopt]`**(挙動は変えない): ObservedEisu を採ったとき 1 行。項目: 経路(OsPoll 同期/prefetch/ImmCrossProbe/idle-conv-check)・分岐((a)/(b))・読み取り方針(打鍵中の除外を外した読みか、通過マークが有効か)・直近のモードキー通過からの ms と vk・読み取りの開始時刻と open/conv プローブの `elapsed_us`・`probe_timed_out`・`prev_conversion_mode` の値とそれが書かれてからの経過 ms。
2. **一過性の値の長さ**(確認の間隔を決める根拠): conv=0 を読んだら、診断のためだけに +20/+60/+120ms で読み直してログに出す(belief には反映しない)。
3. **本物の切替を壊さないことの CI(e2e-ime.yml の構成名):** `msime-native-noawase`(対照、MS-IME の実際の conv の値。`check_consistency.py --real-only` が各手順の +1500ms の `open=… conv=0x..` を出す)、`sc-dbe-msime-native`/`sc-dbe-gji-msime`/`sc-dbe-gji-atok`(F0=英数キー、GJI の stale recovery を含む)、`msime-native`/`msime-native-henkan`(`--walk`)、`sc-shift-msime-native`(awase の IMC 書き込み conv=0・open のままからの復帰)、`msime-stale-table`(表が古く予測が効かない経路)、`sc-hz-msime-native`。
4. **既存の判定の粗さの補正:** `check_consistency.py` は押下後 +700ms 前後と +1500ms の値しか見ない。押下から最初の `InputModeObserved(ObservedEisu)` / `Engine deactivated` までの ms を別に集計する。
5. **予測の無い切替の observe 構成を足す:** ハーネスから外部の `ImmSetConversionStatus`(open のまま conv=0x0000/0x0010)を書き、20ms 間隔の打鍵を続けながら、Engine が止まるまでの時間を修正の有無で比べる(F/A の遅れの代償を測る唯一の手段)。

## 状態

起草中。Opus round1 反映済み、round2 待ち。
