# ADR-265 段階 2b: 共有メモリを belief に接続し、食い違う applied を降格する設計(案 v3、2026-10-18)

状態: **設計案**(実装前。リポジトリの運用により、実装の前に ADR 化を経る)。v1・v2 は Opus の敵対的レビュー 2 回(v1: C1・C2 致命、H1〜H3 重大。v2 再確認: N1〜N10)と、実機実験([結果](adr265-belief-experiments-result.md))で改めた。
レビューの主要な点は `develop` のコードで私が確かめた(drift correction の根拠は `ConvOpenInference` だけを除外し authority・窓の照合を見ない、`record_any` は常に `expires_at: None`、`resolve_open_at` の優先順は 明示意図 → 打鍵予測 → `derive_any`、`Chrome_WidgetWin_1` は `IMM32_UNAVAILABLE_CLASSES` にある、`hook::current_tick_ms` と `update_tick_ms` は共に `GetTickCount64`、`AppliedImeState::is_confirmed()` の本番の呼び出しは無い)。残りはレビューの読みで、実装時に再確認する。

## 0. 方針(利用者決定)
1. 共有メモリは Mozc 自身の状態の事実なので、belief へ**最初から High** で入れる。自動判定(設定なし)。
2. **`applied` は、当面は降格(`Unknown` にする)だけ**。Confirmed への書き込みは、送信の省略が実際に減ることを journal で示せてから別に足す(§6)。
   理由: Confirmed にして減らせる送信は、検証が通った窓の種類でほとんど無い(素の Win32 窓は ImmCross が先頭で `applied` を見ない。Edge は押下で `applied` を未知にする窓で、効くのは押下以外の送信だけ)。ADR-214 の P1(古い `applied` が「一致」と言って必要な送信を省く、Engine の固着)は降格で直る。
3. IME へ送る根拠(`derive_actuating`・授権)には**入れない**(`BeliefOnly`)。

## 1. 目的と、やらないこと
- 目的: mozc-readable が動いている窓で、(a) IME の開閉の belief を共有メモリで正す、(b) 共有メモリと食い違う `applied` を降格して、古い `applied` による誤った省略(ADR-214 P1・P3)を防ぐ。
- やらない: `input_mode` には入れない。`desired_open` を直接書かない。IME へ何も送らない(drift correction 経由も含めて)。`applied` を Confirmed にしない(§6)。

## 2. 基準の時刻と「操作より後」の判定(N1・N2・N4)
共有メモリの読みが、awase の操作より**後に**外で変わったものか、を判定する規則 **R3'**:
```
R3' = (update_tick_ms > 基準) || (読んだ時刻 − 基準 >= SETTLE)
基準 = max(hook_alive_tick_ms, 最後の ImmCross の書き込みの発行時刻)
SETTLE = KEY_EFFECT_SETTLE_MS (170ms、既存の定数を流用)
```
- 時計は同じ(`GetTickCount64`)。`hook_alive_tick_ms` は全キー(自分の注入と KeyUp を含む)でフックの入口に更新され、awase の SendInput の書き込みも含む。ImmCross だけが入らないので、**新しい欄は「ImmCross の発行時刻」1 つ**(`open_chain.rs::imm_cross_write` の発行時に記録)。
- 第 1 項は strict。第 2 項は、操作から SETTLE 以上経った読みは、操作を処理し終えた後の事実(変化したなら新しい値、変化しなかった・書き込みが失われたなら古い値で、どちらも事実)という判定。`>=` が古い読みを通す経路(warmup の `VK_DBE_HIRAGANA` に続く `SetOpen`、chord、自動リピート)を、同じ目盛り(約 15.6ms)のあいまいさごと消し、書き込みが失われた場面(下)も救う。
- 全キーを基準にするので、打ち続けているあいだは第 1 項が通りにくい。これは安全側。

## 3. belief への入れ方(Part A)

| 項目 | 内容 | 理由 |
| --- | --- | --- |
| source | `ObservationSource::MozcShm`(+ `evidence::MozcShm`)。High、`BeliefOnly` | 利用者決定。授権(`open_warrant.rs` Step 3)には載せない |
| 入口 | `ImeStateHub::report_mozc_shm_open(open, hwnd, focus_epoch, tick)`(`report_conv_open_inference` の隣)。呼び出し元は 1 つ | ガードの件数・`observation_source_guard` の対象を更新 |
| 読む値 | `tsf_open`。**`tsf_open != effective_open` のときは報告せず、`applied` を降格する(必須)**(M2) | 割れる場面は未測定。割れは食い違いが確定した状態 |
| 窓の照合 | `derive_filtered` の `is_identity_ok` に `MozcShm` を足す(epoch+hwnd) | 記録の瞬間は同義反復(M5 のとおり)なので、効くのは次の tick 以降の古い観測の除外 |
| 鮮度 | `record_any` で `source == MozcShm` のときだけ `expires_at = Some(at + TTL)`。`derive_any`・`most_recent_trusted`・`_excluding`・`consensus` の全読み手が `is_expired` を見る(H2) | `heuristic_default` の「鮮度上限を持ち込まない」(ADR-087 INV-22 撤回)の原則の**例外**になることを ADR に明記 |
| TTL の値 | tick の最大間隔より長く: **500ms + prefetch の 300ms のタイムアウト + ジッター**を実測で導く(`tuning.rs`、`#[measured]`、実測値を本文に)。500ms+小さな余裕では、prefetch がタイムアウトした tick で期限が切れ、belief が一瞬フォールバックへ反転する | N6 |
| 報告の頻度 | 値が変わったとき、と、TTL の更新のとき。更新のたびに journal の State の枠(2048)を消費しないよう、**journal を伴わない延長**(`expires_at` の更新だけ)にするか、更新を TTL の半分ごとに絞る。どちらにするかは ADR で決める(M3・n3の矛盾の解消) | 毎 tick の `ObserverReported` は約 17 分で枠が入れ替わる |
| 読む対象 | (tid, pid) ごとに `NotFound` を覚え、フォーカスが変わるまで読まない(主)。GJI 系の TIP のときだけ読む(従) | M4: mozc-readable を使わない利用者に毎 tick の Win32 呼び出しを課さない |

### 3-1. drift correction の除外(C1。実装前に必須)
`evaluate_drift` の根拠(`most_recent_trusted_excluding(now, &[ConvOpenInference])`)と `read_back` の `EXCLUDED_FROM_ANY_FRESH_EVIDENCE` から `MozcShm` を**除外**する。除かないと、High の共有メモリが明示意図と食い違うたびに drift correction が IME を開け直し(Standard の窓)、Blind の窓で BUG-114 型の再武装ループと BUG-124 型の「@」を起こす。IME へ何も送らないことは、このテストで固定する(§7)。

### 3-2. 外から変わったときの追随(C2・N2・N8)
`resolve_open_at` の優先順は 明示意図 → 打鍵予測 → `derive_any`。さらに `IntentStore`(ON 10 秒・OFF 30 秒)が上にある。High の `MozcShm` だけでは、外から閉じられても belief は直らない。
- 共有メモリの値が `effective_open` **または** `desired_open` と食い違い(N8)、かつ R3' を満たすとき、既存の追随と同じ 3 手を実行する: `report_mozc_shm_open`(High)→ `intent_store.remove(hwnd)` → `pass_through_observed(tick, true, true)`(`last_intent` を捨て、`desired_open` を観測に揃え、食い違う `applied` を `Unknown` に落とす)。手本は `follow_external_change_in_scope` / `follow_direct_read_in_scope`。新しい `ImeEvent` variant は要らない(`ModeKeyPassedThrough` の新しい呼び出し元を足すだけ。`ime_event_guard` もそのまま通る)。
- **書き込みが失われた場面(N2)**: awase の書き込みが IME に届かず、共有メモリが変わらないとき、R3' の第 2 項(SETTLE 経過)で追随する。belief は共有メモリの値(IME の実際の状態)に揃い、再送はしない(drift の除外は維持)。利用者が押し直せる(TsfNative×GJI の受動化の方針と一致)。
- R3' を満たさない読み(操作の直後の、処理前かもしれない読み)は、belief に報告せず、追随もしない(捨てる)。ただし**降格はする**(§4)。
- 打鍵予測の照合(`KEY_EFFECT_SETTLE_MS`)の前に、「打鍵より後に変化していない読み」は照合しない(M1)。`AnyObservation` は `update_tick_ms` を運ばないので、`reduce` の中ではなく、ハブで報告そのものを止める。
- 追随の入口が増える(N9)。`architecture_guard::external_change_watch_has_single_arm_and_follow_sites` の列挙に追記する。Edge のように既存の追随(ObserverPoll)も効く窓では、`MozcShm` が受け入れられた tick は ObserverPoll の追随を飛ばす(後勝ちで journal に `ModeKeyPassedThrough` が 2 件並ぶのを避ける)。

## 4. `applied` の降格のみ(Part B)
- 共有メモリの受け入れた読みが `applied_open()` と食い違うとき、`applied` を `Unknown` にする(降格)。R3' は不要(古い読みで降格しても、被害は「余分な送信 1 回」で、安全側)。awase の書き込み直後の約 95ms の遅れで余分な降格が起きうるが、同様に安全側。
- 実装は §3-2 の追随の中の `pass_through_observed(.., demote_applied = true)` で足りる。追随しない読み(R3' 不成立)で降格だけしたい場面は、`demote_applied` を単独で呼べるか(新しい入口が要るか)を ADR で確認する。`tsf_open != effective_open` のときの降格も同じ入口。
- これで ADR-214 の P1(トレイ起点の Engine コマンドの `SetOpen` が古い `applied` で省かれる)と、共有メモリが読める窓での P3 が直る。**共有メモリが読めない窓(mozc-readable でない IME・読めない窓)は、この変更の対象外で、従来どおり**。
- **ADR-214 の再開に当たるのは、降格の範囲に限る**(`Sent`/`Confirmed` の型の分離・経路ごとの宣言はしない)。降格は `AppliedImeState` に新しい値を書かないので、ADR-214 の「Confirmed の混ぜ」を悪化させない。

## 5. 読む頻度・やらないこと
既存の約 500ms の tick。pulse での読み直し、`input_mode` への接続は別の段階。

## 6. 将来の拡張: `applied` の Confirmed(いまは入れない)
入れる条件: journal の `ActuationDecision` で、「`applied=Unknown` のせいで送られた、押下以外の余分な送信」が実際にどれだけあるかを数え、便益が示せたとき。
入れるなら次を守る(レビューと実機検証の結果): 規則は R3'、窓は (プロセス名, クラス) の許可リスト(Edge は `msedge.exe`+`Chrome_WidgetWin_1`/`Chrome_RenderWidgetHostHWND`。Standard は広すぎるので「素の Win32」はプロセス名で列挙)、Confirmed は「`update_tick_ms` > 最後に `applied` を `Unknown` にした時刻」のときだけ(E1 の上乗せの危険を 0 にする)、新しい `ImeEvent` variant(`scope` を持たせない。`ForegroundScope` は `Serialize` でない)で `pending` と mode-key-pass に触れない(不変性のテストと出現数のガード)、追随(降格)→確認の順を固定する。
実機検証の結果(素の Win32 窓 A=40・B=50、Edge A=40 で、規則を通った読みの食い違い 0、古い読みは観測されず)は、Confirmed が検証した窓で安全に動くことの証拠として残す。

## 7. ガードとテスト
- `drift_correction`: `MozcShm` だけが根拠のとき `evaluate_drift` が `Ok` にならない。`read_back` の `AnyFreshEvidence` の除外。
- `observation_store`: (i) TTL の失効が `derive_any`・`most_recent_trusted`・`_excluding` の全部に効く、(ii) 窓の照合、(iii) `BeliefOnly` が `derive_actuating` に入らない、(iv) High は ObserverPoll(Medium)と食い違っても採られる(利用者決定)。
- 追随: R3' の第 1 項(`update_tick > 基準`)で追随、第 2 項(SETTLE 経過)で追随(書き込みが失われた場面)、どちらでもない読みで追随しない、`desired_open` との食い違いで追随、3 手の順序、食い違う `applied` が `Unknown` に落ちる。
- 降格: 食い違いで降格、一致で触らない、`tsf_open != effective_open` で降格。
- `architecture_guard`: 新 source の網羅 `match`(`observation_store.rs` の `get`/`set`、`ime_event.rs` の `authority`)、追随の入口の列挙、`ModeKeyPassedThrough` の構築数。`lints/observation_source_guard`: 新 source の構築を `report_mozc_shm_open` だけに限る。
- 回帰の置き場所: `fix-requires-evidence` の「IME belief」「force-write / actuation」のファミリー。同じ PR にテストを足す。tuning 定数(TTL)は実測値を本文に。

## 8. 未解決・実装前の確認
- **E1(外からの変更が共有メモリに出るか)が未解決**。1 回目(0/20)と 2・3 回目(変化あり)が不一致。公開されない外部変更があると、**High の共有メモリが、正しい ObserverPoll(Medium)を上書きして、belief が古いまま固定される**(mozc-readable を入れていない場合より悪くなりうる)。降格は `applied` の安全側なので影響しないが、belief の High にはこの危険が残る。IMM で読める窓で、High の共有メモリが新しい IMM の直接読みと食い違ったときの扱い(例: 共有メモリを採らない、または降格)を、ADR で決める。
- TTL の値(実測)。journal の頻度の方式(§3 の表)。「ImmCross の発行時刻」の欄の置き場所。
- `demote_applied` を単独で呼べるか(§4)。
- 「新しい窓の最初のキーは効かない」(先に a+Enter を 1 回打つと効く)の因果。BUG-200 との関係(別の調査)。
- 複雑性: 新 source・新 evidence・ハブの入口(報告・降格)・追随の入口・tuning 定数 1 本・新しい欄 1 つ。新しい `ImeEvent` variant は無い。`complexity-budget.md` は未発効。
