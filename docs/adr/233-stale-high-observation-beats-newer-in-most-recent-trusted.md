---
id: ADR-233
title: |-
  古い High 観測が新しい Medium 観測に勝つ件(BUG-189)— belief のフォールバック(most_recent_trusted)の順位キー
summary: |-
  BUG-189: ImmCrossProbe(High)が `expires_at: None` で残り、打鍵中に観測の書き込みが止まって 3.0s(OBSERVATION_FRESH_WINDOW_MS)経つと、belief のフォールバック `most_recent_trusted` が `(confidence, at)` の順で比べるため、後から同じ読み取りで得た新しい ObserverPoll(Medium)より古い High が勝ち、実効 IME 状態が反転する(CI 8 回中 6 回、反転 26/26 が同根拠。継続は 0〜2196ms)。その間エンジンが止まり、親指キーが生のまま MS-IME へ渡る。
  決定(案): `resolve_open_at` のフォールバックだけ、順位キーを `(confidence >= Medium, at)` にする(A')。drift correction と read_back の比較式は変えない。
status: |-
  起草中(2026-10-06)。Opus round1 反映済み(Blocker 2・Must 5)、round2 待ち。実装前に診断ログ(旧・新比較式の並記)を CI で回して正解率を測る。
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

**実質は「同じ読み取りの新旧」であり、信頼度の差ではない。** 本番で open 観測プールに書く経路は 4 つだけ(`ObserverPoll` Medium、`ImmCrossProbe` High〈呼び出しは `key_pipeline.rs` と `focus_tracking.rs`、どちらも Standard に限る〉、`FocusProbe` Low、`HeuristicDefault` Low。`ConvOpenInference` の口はテスト以外から呼ばれない)。High と Medium は**どちらも `read_ime_state_full` で同じ focused hwnd を読む**ため、High/Medium は測り方の質の差でなくラベルの差である。同じ測定が 2 つあれば新しい方が正しい。

**GJI で起きない理由**(「ICP が true」は誤り。GJI でも ICP=false は 4/4 記録される): GJI では打鍵中の OS ポーリングが止まり、F2 の `KeyEffectPrediction(true)` が観測で照合・消去されず、`resolve_open_at` がプールまで降りてこない。MS-IME では ObserverPoll(true)が届いて予測が消え、3s の空きでプールに降りて反転する。GJI の安全は偶然(予測が消えないこと)に頼っている。

**Flutter 固有ではない:** LibreOffice Writer × MS-IME(art12 の 6 本すべてで ICP=false、うち 5 本で観測の空きが 3s 超、最大 4.62s)にも `Engine deactivated (ime=false…)` が 1〜6 回出ている(反転が試行の間に落ちるため PASS)。根拠が `MostRecentTrusted` かは診断ログ入りの再実行で確定する。

## 決めること

belief のフォールバック(`resolve_open_at` が `derive_any` / `derive_actuating` で決まらないときの `most_recent_trusted`)の順位キー。

## 案

| 案 | 内容 | 評価 |
| --- | --- | --- |
| **A'(推奨)** | `resolve_open_at` 専用の比較式にし、順位キーを `(confidence >= Medium, at)` にする。Medium 以上の中では新しい方が勝つ。Low(FocusProbe / HeuristicDefault)が勝つのは Medium 以上が 1 件も無いときだけ。Low 同士は新しい方(今と同じ) | 影響は Standard プロファイルで ICP と ObserverPoll が両方プールにあるときだけ(Imm32Unavailable / TsfNative / Blacklist には High が無い)。BUG-16・ADR-087 の根拠階層・actuation の授権(`issue_open_warrant` は `derive_actuating` と `heuristic_default` だけを使い `most_recent_trusted` を使わない)には触れない。`ime_model.rs:426` の doc(「最新優先」)と実装の食い違いも Medium+ の範囲で解消する |
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
- 理由(drift): ImmCross(Standard)アプリでは、残り続ける古い ICP のため `most_recent_trusted_excluding` が記録から 1.5s 以降は毎回それを選び、`trusted.age > 1500ms` で `Err(StaleObservation)` になる(`drift_correction.rs:127-129`)。つまり ImmCross アプリの drift 補正は、プローブ後 1.5s を過ぎると事実上動いていない**と推定される**。drift まで A' に揃えると再試行が復活し、awase が実 IME に書く回数が増える(ADR-212 の流れに逆行)。**別の BUG として起票して切り分ける**(CI ログで `NoDrift::StaleObservation` の件数とソースを数えてから判断)。
- `ime_refresh.rs` の `DriftGiveUpDiagnostic` が `most_recent_trusted`(ConvOpenInference を除外しない)で取る `observation_source` は、`evaluate_drift` の根拠とずれうる(Nit)。A' の後はさらにずれるので、`DriftCorrection.source` を使う形に直す(別 PR でよい)。

## ADR-087 / BUG-16 / ADR-208 / ActuatingPool への影響

- BUG-16: `heuristic_default` の注記が戒めるのは `heuristic_default()` に鮮度窓を足すこと。A' はそこに触れない。`open_warrant.rs` の Step 4a は `heuristic_default()` を直接使い `most_recent_trusted` を使わない。
- ADR-087 の根拠階層 / ActuatingPool: actuation の授権は `derive_actuating`(鮮度窓あり)と `heuristic_default` のみ。影響なし。
- ADR-208 の押下 ID: 台帳(`claim_press_write`)には影響しない。ただし `effective_open` は shadow toggle の no-op 判定の入力になっている。**今のバグには、反転中にユーザーが IME OFF キーを押すと no-op と判定されて書かれず IME が ON のまま残る経路もある(A' はこれも直す)。逆に A' で belief が true に寄る場面では、OFF キーで実際に書くようになる**ので、sc-* の IME キー系シナリオを確認項目に入れる。
- 不変条件(プロパティテスト化する): **新しいイベントが無く時間だけが進んで belief が変わるのは、古い High が derive の鮮度窓から外れて、より新しい観測に席を譲るときに限る。** 今の実装には「derive が Medium(true)を返す → 3s 経つ → 古い High(false)に戻る」という、時間経過だけで起きる逆戻りがある。A' はこれを消す。

## 実装前の測定(Blocker 1 の反映)

既存 journal コーパス(`tests/journals/` は actuation_decision / conv_classify / drift_correction / ime_apply / read_strategy のみ)に、観測を時刻付きで流して `resolve_open_at` を評価するものは無い。journal リプレイでの「差分数え」は実行できない。代わりに次を行う。

1. **旧・新の比較式を並記する診断ログ(挙動は変えない)を CI で回す。** `effective_open_at` の診断の隣で、`derive_any` が None のときに旧・新の両方で `most_recent_trusted` を計算し、結果が違えば `[mrt-shadow] old=… new=… old_src=… new_src=… ages=…` を 1 行出す。tsx-ext-*(GJI・MS-IME とも、特に Standard になる flutter-textfield / libreoffice-writer / openoffice-writer)と ts-*・sc-* で回し、不一致の件数と、各件で「次に届いた ObserverPoll の値と一致したのは旧・新どちらか」を数える。これが A' の正解率になる。
2. **閉ループハーネス(`tests/support/harness.rs`、仮想時計 `state/hub_clock.rs`)または `ime_model.rs` の単体テストで BUG-189 の列を決定的に再現する:** ICP(false, High)@t0 → KeyEffect(F2→true)@t0+0.65s → ObserverPoll(true)@t0+4.5s(予測が消える)→ 3.01s 何も無い → `resolve_open_at`。旧は false、新は true。
3. 長いアイドルの確認: アイドル中は poll が 500ms ごとに書くので `derive_any` が決め、フォールバックには来ない。poll が時間切れ(`ime_on=None`)を連続で起こす構成では来うる。`IME detection timed out` が 3s 以上続く区間を数え、そこで `[mrt-shadow]` が出たかを見る。

## 回帰テスト(fix-requires-evidence の (a))

- `ime_model.rs`: 上の列で旧 false・新 true。
- `observation_store.rs`: 古い ICP(High,false)+ 新しい ObserverPoll(Medium,true)→ true。同時刻の Low と High → High(既存の `most_recent_trusted_by_confidence` を維持)。**新しい FocusProbe(Low,false)+ 古い ICP(High,true)→ true**(A' が Low を昇格させないことの固定)。
- `most_recent_trusted_excluding`(drift)と `read_back` の比較式が**変わっていない**ことを固定するテスト(適用範囲の固定)。
- 診断ログ `[effective-open-flip]` は残す(反転した瞬間の根拠が追える)。

## 未解決

- 診断ログ入りの再実行で、LibreOffice の `Engine deactivated` が `MostRecentTrusted` かを確定する。
- drift 側(古い ICP が drift を止めている件)は別 BUG。
- D(fence 照合)は別 ADR。

## 状態

起草中。Opus round1(Blocker 2・Must 5)反映済み、round2 待ち。
