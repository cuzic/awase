---
id: ADR-233
title: |-
  古い High 観測が新しい Medium 観測に勝つ件(BUG-189)— belief のフォールバック(most_recent_trusted)の順位キー
summary: |-
  BUG-189: ImmCrossProbe(High)が `expires_at: None` で残り、打鍵中に観測の書き込みが止まって 3.0s(OBSERVATION_FRESH_WINDOW_MS)経つと、belief のフォールバック `most_recent_trusted` が `(confidence, at)` の順で比べるため、後から同じ読み取りで得た新しい ObserverPoll(Medium)より古い High が勝ち、実効 IME 状態が反転する(CI 8 回中 6 回、反転 26/26 が同根拠。継続は 0〜2196ms)。その間エンジンが止まり、親指キーが生のまま MS-IME へ渡る。
  決定(案): `resolve_open_at` のフォールバックだけ、順位キーを `(confidence >= Medium, at, confidence)` にする(A')。drift correction と read_back の比較式は変えない。
status: |-
  起草中(2026-10-06)。Opus round1・round2 反映済み(round2 の Must 3 件の差分確認待ち)。実装前に診断ログ(旧・新比較式の並記)を CI で回して測る。
related_adr:
  - "ADR-087"
  - "ADR-090"
  - "ADR-106"
  - "ADR-191"
  - "ADR-208"
  - "ADR-212"
---

# ADR-233: 古い High 観測が新しい Medium 観測に勝つ件(BUG-189)

## 背景(BUG-189、CI で再現)

[BUG-189](../known-bugs/BUG-189.md)。Flutter(TextField)× MS-IME で awase ありのときだけ、打鍵の途中から出力が全角英字・カタカナに化ける(GJI と、awase なしの対照は PASS)。

診断ログ `[effective-open-flip]`(`platform_state.rs::effective_open_at`、挙動を変えない)で反転の根拠を特定した(run 37431493935、8 本)。

1. ハーネスの最初の打鍵(`VK_IME_OFF`)をきっかけに FocusProbe が動き、`[ImmCrossProbe] child-hwnd IME=false → High confidence 観測記録`(`key_pipeline.rs` の ICP、`AppImeProfile::Standard` かつ日本語 IME のとき)が 1 件記録される。**この false は記録時点では真の値**(その時 IME は OFF。awase 自身が shadow toggle で OFF を書いた直後で、ObserverPoll も全件 false)。**`expires_at: None`**(`record_any` が全観測にハードコード)で残る。
2. その後ハーネスの `VK_DBE_HIRAGANA`(0xF2)で IME が ON になり、ObserverPoll(Medium)は true を返し続ける。打鍵中は `typing active` で poll の書き込みが止まり、最後の書き込みから 3.0s で `derive_any` が None になる(`OBSERVATION_FRESH_WINDOW_MS`)。
3. フォールバックの `ObservationStore::most_recent_trusted` は `max_by(confidence, then at)`、**信頼度が先・新しさが後**。鮮度窓も focus 同一性も見ない。**古い High(false)が、後から来た Medium(true)に勝つ。**
4. 実効値 false → `Engine deactivated (ime=false, reason=Inactive(ImeOff))`。親指キーが `PassThrough` で MS-IME に届く。true に戻るまでの時間は 0〜2196ms(26 回、中央値 約 20ms、553ms 以上が 7 回)。戻るきっかけの 15 回は、**生で漏れたキー自身の `KeyEffectPrediction`**(被害の結果として戻っている)。

反転 26 回中 26 回が `→ false decided_by=MostRecentTrusted(ImmCrossProbe)`。

**実質は「同じ読み取りの新旧」であり、信頼度の差ではない。** 本番で open 観測プールに書く経路は 4 つだけ(`ObserverPoll` Medium、`ImmCrossProbe` High〈呼び出しは `key_pipeline.rs` と `focus_tracking.rs`、どちらも Standard に限る〉、`FocusProbe` Low、`HeuristicDefault` Low。`ConvOpenInference` の口はテスト以外から呼ばれない)。**Standard の窓の中では** High と Medium は**どちらも `read_ime_state_full` で同じ focused hwnd を読む**ため、High/Medium は測り方の質の差でなくラベルの差である。同じ測定が 2 つあれば新しい方が正しい。

(`ObserverPoll` を書く本番経路は 3 つある: ① `platform_state.rs::apply_ime_update`(OS ポーリング、上の読み取り) ② `ime_refresh.rs` の `ImeReadStrategy::Blacklist` 分岐(GJI の I/O 活動からの推測、BUG-114 で「自己言及的な弱い代理指標」とされた) ③ `follow_external_change_in_scope`(ADR-205、Imm32Unavailable × GJI)。Standard の窓の中では ②③ は起きないので結論は変わらない。ただし `update_focus_window` は同一プロセス内の窓切替でプールを消さないため、前の窓の古い ICP が残ったまま新しい窓が `can_use_imm32_cross_process()=false` だと ② の弱い ObserverPoll が後から入る。そのとき A' は弱い ObserverPoll を、旧は fence の合わない古い ICP を採る。どちらが正しいとも言えず A' の悪化ではない。D〈fence 照合〉の別 ADR の測定対象とする。)

**GJI で起きない理由**(「ICP が true」は誤り。GJI でも ICP=false は 4/4 記録される): GJI では打鍵中の OS ポーリングが止まり、F2 の `KeyEffectPrediction(true)` が観測で照合・消去されず、`resolve_open_at` がプールまで降りてこない。MS-IME では ObserverPoll(true)が届いて予測が消え、3s の空きでプールに降りて反転する。GJI の安全は偶然(予測が消えないこと)に頼っている。

**Flutter 固有ではない:** LibreOffice Writer × MS-IME(art12 の 6 本すべてで ICP=false、うち 5 本で観測の空きが 3s 超、最大 4.62s)にも `Engine deactivated (ime=false…)` が 1〜6 回出ている(反転が試行の間に落ちるため PASS)。根拠が `MostRecentTrusted` かは診断ログ入りの再実行で確定する。

## 決めること

belief のフォールバック(`resolve_open_at` が明示意図・`KeyEffectPrediction`・`derive_any` のいずれでも決まらないときの `most_recent_trusted`)の順位キー(`resolve_open_at` は `derive_actuating` を呼ばない)。

## 案

| 案 | 内容 | 評価 |
| --- | --- | --- |
| **A'(推奨)** | `resolve_open_at` 専用の比較式にし、順位キーを `(confidence >= Medium, at, confidence)` にする。Medium 以上の中では新しい方が勝ち、**同時刻なら信頼度の高い方**(ADR-087 INV-23 の決定性: `max_by` は等しい要素のうち最後を返すため、タイブレークを置かないと `PerSourceObservations::iter()` の列挙順で決まる。仮想時計の閉ループや `Instant::now()` を 1 回だけ取る単体テストでは ICP と ObserverPoll の `at` が等しくなる)Low(FocusProbe / HeuristicDefault)が勝つのは Medium 以上が 1 件も無いときだけ。Low 同士は新しい方(今と同じ) | 影響は Standard プロファイルで ICP と ObserverPoll が両方プールにあるときだけ(Imm32Unavailable / TsfNative / Blacklist には High が無い)。BUG-16・ADR-087 の根拠階層・actuation の授権(`issue_open_warrant` は `derive_actuating` と `heuristic_default` だけを使い `most_recent_trusted` を使わない)には触れない。`ime_model.rs:426-429` の doc(「最新優先」「後から届いた実観測〈Low でも〉が新しければ優先」)と実装の食い違いを解消する。doc の Low に関する記述は A' に合わせて書き直す(Low は Medium+ に負ける) |
| A1 | 信頼度を一切見ず最新を採る | **非推奨。** FocusProbe(Low、Qt の IME コンテキスト分割で誤読する既知〈`key_pipeline.rs:2826-2830`〉)が古い High/Medium に勝つ。同一プロセス内でウィンドウだけ変わったときに新しい HeuristicDefault が古い窓の ObserverPoll に勝つ |
| B | 子窓に自前の IME コンテキストが無い入力先の ICP(false)を記録しない/Medium に落とす | **不十分。** false は誤読ではなく、LibreOffice(SALFRAME)でも同じ前提が成り立つ |
| C | ICP に有効期限を付ける | **単純でない。** `record_any` が全観測に `expires_at: None` をハードコードし、本番で Some にする経路が無い。`AnyObservation`(journal 直列化、ADR-082)の変更と期限の実測(tuning-constants.md)が要り、3s 未満なら `derive_actuating`(Step 3)の結果も変わる |
| D | `most_recent_trusted` にも `is_identity_ok`(focus の epoch・hwnd 照合)を適用する | 今回の原因ではない(ICP の fence は一致)。A' の後も、fence が合わない古い ICP が「Medium+ で最新」になる余地は残る。**別 ADR とし、診断ログで「fence 不一致の観測をフォールバックが採った件数」を測ってから決める** |
| E | 打鍵中も poll を読む(typing-active skip の見直し) | 非推奨。打鍵中に IMM へ SendMessage しないための制限(別ファミリー)。観測の空きが縮むだけで順序の欠陥は残る(実測の最大 5.79s) |
| F | ObserverPoll の鮮度窓だけ延ばす | 非推奨。上限が決まらず釣り上げになる(tuning-constants.md)。延ばした窓では古い Medium が新しい High に負けない逆の問題も出る |
| G | poll の結果を ICP の欄にも書く/記録時に新旧を突き合わせて High を消す/確定した予測を残す/ヒステリシス | 非推奨。A' より変更が大きいか、プール・予測の状態を書き換えて drift / read_back / ADR-191 の fence に波及する。A'(問い合わせ側の比較式だけ)が最も局所的で戻しやすい |

## 適用範囲(Blocker 2 の反映)

`max_by(confidence, then at)` は `observation_store.rs` の 2 か所にあり、3 つの用途で共有されている。**A' は `resolve_open_at` のフォールバックだけを変える。**

- 変える: `state/ime_model.rs::resolve_open_at` が呼ぶ `most_recent_trusted`(belief のフォールバック)。実装では `most_recent_trusted` を `_excluding` への委譲から外し、比較式を別にする(共有のまま変えると下も同時に変わる)。
- **変えない:** `state/drift_correction.rs::evaluate_drift`(`most_recent_trusted_excluding`)と `observation_store.rs::read_back`(`most_recent_trusted_after[_excluding]`)。read_back は `since` で区切った後の High が「送信後に直接読んだ値」であり、信頼度を先に比べる意味がある(ADR-090 B-R2)。
- 理由(drift): ImmCross(Standard)アプリでは、残り続ける古い ICP のため `most_recent_trusted_excluding` が記録から 1.5s 以降は毎回それを選び、`trusted.age > 1500ms` で `Err(StaleObservation)` になる(`drift_correction.rs:127-129`)。つまり**明示意図があり、それと desired が一致している場合に限って**(`evaluate_drift` は先に `explicit_intent != Some(desired)` で `NotExplicitIntent` を返す)、ImmCross アプリの drift 補正は、プローブ後 1.5s を過ぎると事実上動いていない**と推定される**(照合済み: ICP の `expires_at` は常に None で `clear_on_focus_change` まで消えない)。drift まで A' に揃えると再試行が復活し、awase が実 IME に書く回数が増える(ADR-212 の流れに逆行)。**別の BUG として起票して切り分ける**(CI ログで `NoDrift` の理由ごとの件数、特に `StaleObservation` のうちソースが `ImmCrossProbe` のものを数えてから判断)。
- `ime_refresh.rs` の `DriftGiveUpDiagnostic` が `most_recent_trusted`(ConvOpenInference を除外しない)で取る `observation_source` は、`evaluate_drift` の根拠とずれうる(Nit)。A' の後はさらにずれるので、`DriftCorrection.source` を使う形に直す(別 PR でよい)。

## ADR-087 / BUG-16 / ADR-208 / ActuatingPool への影響

- BUG-16: `heuristic_default` の注記が戒めるのは `heuristic_default()` に鮮度窓を足すこと。A' はそこに触れない。`open_warrant.rs` の Step 4a は `heuristic_default()` を直接使い `most_recent_trusted` を使わない。
- ADR-087 の根拠階層 / ActuatingPool: actuation の授権は `derive_actuating`(鮮度窓あり)と `heuristic_default` のみ。影響なし。
- ADR-208 の押下 ID: 台帳(`claim_press_write`)には影響しない。shadow toggle の no-op 判定は `effective_open` を入力にするが、ADR-208 D4(`kp_shadow_noop_write`、no-op でも物理キーを Suppress するなら `key_target` の向きに書く/Allow なら物理キーが IME に届く)により、A' の前後とも押下の向きは実 IME に届く。変わるのは「no-op 分岐で書くか、通常の toggle 分岐で書くか」だけで、実 IME に届く結果は同じ(例外はリピートと TsfNative だけで、BUG-189 の対象〈Standard〉には当たらない)。sc-* の IME キー系を確認項目に残す理由は「分岐が変わるので念のため」。
- 不変条件(プロパティテスト化する。**対象は `ImeModel::resolve_open_at` の観測部分〈明示意図なし・予測なし〉**。`platform_state.rs::effective_open_at` の `IntentStore` には TTL〈`EXPLICIT_OFF_INTENT_TTL_MS`〉があり、そちらは時間だけで値が変わる): **新しいイベントが無く時間だけが進んで belief が変わるのは、古い High が derive の鮮度窓から外れて、より新しい観測に席を譲るときに限る。** 今の実装には「derive が Medium(true)を返す → 3s 経つ → 古い High(false)に戻る」という、時間経過だけで起きる逆戻りがある。A' はこれを消す。

## 実装前の測定(Blocker 1 の反映)

既存 journal コーパス(`tests/journals/` は actuation_decision / conv_classify / drift_correction / ime_apply / read_strategy のみ)に、観測を時刻付きで流して `resolve_open_at` を評価するものは無い。journal リプレイでの「差分数え」は実行できない。代わりに次を行う。

1. **旧・新の比較式を並記する診断ログ(挙動は変えない)を CI で回す。**
   - **出す条件:** `resolve_open_at` のフォールバック分岐と同じ条件(明示意図なし・`KeyEffectPrediction` なし・`derive_any` が None)のときだけ、旧・新の両方で `most_recent_trusted` を計算し、**値が違ったときだけ**(`[effective-open-flip]` と同じ重複の消し方)`[mrt-shadow] old=… new=… old_src=… new_src=… ages=… hwnd/fence=…` を 1 行出す。`derive_any` が None なだけで数えると、予測が残ってフォールバックに来ていない GJI の件まで「影響あり」に数えてしまう。`IntentStore` の上書きで最終値が変わらなかった件は「実害なし」として別に数える。
   - **正解の定め方:** 空きの**直前**の ObserverPoll と**直後**の ObserverPoll が一致し、かつ間に IME 開閉キー・`KeyEffectPrediction(open=Some)`・actuation(`[apply-ime]`)が無い件だけを「正解が定まる」とし、正解をその共通の値とする。それ以外は「判定不能」として別に数え、件数を報告する(判定不能が多ければ、この測定は安全性の証拠にならない)。「次に届いた ObserverPoll と一致」は、診断を旧の挙動のまま回すと旧が false を選んで Engine が止まり漏れたキーが実 IME を変えうる(自己汚染)ため、定義に使わない。
   - **A' が負けるべき場面を探す集計:** 新が選んだ ObserverPoll が、その直前の ICP と同じ hwnd・同じ fence で取られたかを出す(`ImeObservation.hwnd` と `focus_epoch` をログに出す。fence が違う件は D の測定を兼ねる)。ObserverPoll の書き込み経路(上の ①②③)は `ObservationSource` で区別できないので、`[stage-observe]` / `[external-change]` のログ行と時刻で突き合わせる。
   - **実害の直接測定:** 試行の PASS/FAIL と突き合わせる。旧が false に倒した区間を含む試行の FAIL 率と、そうでない試行の FAIL 率を比べる。
   - 対象は tsx-ext-*(GJI・MS-IME とも、特に Standard になる flutter-textfield / libreoffice-writer / openoffice-writer)と ts-*・sc-*。
2. **閉ループハーネス(`tests/support/harness.rs`、仮想時計 `state/hub_clock.rs`)または `ime_model.rs` の単体テストで BUG-189 の列を決定的に再現する:** ICP(false, High)@t0 → KeyEffect(F2→true)@t0+0.65s → ObserverPoll(true)@t0+4.5s(予測が消える)→ 3.01s 何も無い → `resolve_open_at`。旧は false、新は true。
3. 長いアイドルの確認: アイドル中は poll が 500ms ごとに書くので `derive_any` が決め、フォールバックには来ない。poll が時間切れ(`ime_on=None`)を連続で起こす構成では来うる。`IME detection timed out` が 3s 以上続く区間を数え、そこで `[mrt-shadow]` が出たかを見る。

## 回帰テスト(fix-requires-evidence の (a))

- `ime_model.rs`: 上の列で旧 false・新 true。
- `observation_store.rs`: 古い ICP(High,false)+ 新しい ObserverPoll(Medium,true)→ true。同時刻の Low と High → High(既存の `most_recent_trusted_by_confidence` を維持)。**同時刻の ICP(High,false)と ObserverPoll(Medium,true)→ false(High、タイブレーク)。****新しい FocusProbe(Low,false)+ 古い ICP(High,true)→ true**(A' が Low を昇格させないことの固定)。
- `most_recent_trusted_excluding`(drift)と `read_back` の比較式が**変わっていない**ことを固定するテスト(適用範囲の固定)。
- 診断ログ `[effective-open-flip]` は残す(反転した瞬間の根拠が追える)。

## 未解決

- 診断ログ入りの再実行で、LibreOffice の `Engine deactivated` が `MostRecentTrusted` かを確定する。
- drift 側(古い ICP が drift を止めている件)は別 BUG。
- D(fence 照合)は別 ADR。

## 状態

Opus round1(Blocker 2・Must 5)と round2(Must 3・Should 3・Nit 3)を反映済み。round2 の判定は「Must 3 件を文言で直せば収束」で、差分の確認が済めば確定する。
