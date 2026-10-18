# ADR-265 段階 2b: 共有メモリを belief と applied へ接続する設計(案 v2、2026-10-18)

状態: **設計案**(実装前。リポジトリの運用により、実装の前に ADR 化と Opus の敵対的レビューを経る。ADR-214 の再開に当たる部分を含む)。
v1 からの変更の理由: Opus の敵対的レビュー(結論: C1・C2 致命、H1〜H3 重大)と、実機実験([結果](adr265-belief-experiments-result.md))。レビューの主要な 5 点は、`develop` のコードで私が確かめた
(drift correction の根拠は `ConvOpenInference` だけを除外して authority・窓の照合を見ない、`record_any` は常に `expires_at: None`、`resolve_open_at` の優先順は 明示意図 → 打鍵予測 → `derive_any`、`ENGINE_PRESS_UNKNOWNS_APPLIED_IN_TSF_NATIVE=false`、`AppliedImeState::is_confirmed()` の本番の呼び出しは無い)。残りの記述はレビューの読みで、実装時に再確認する。
コードは `develop` の `779fc52f` 前後で読んだ(ビルド・実行はしていない)。

## 0. 方針(利用者決定)

1. 共有メモリは Mozc 自身の状態の事実なので、belief へ**最初から High** で入れる。自動判定(設定なし)。
2. **`applied` は、実機検証が通った窓の種類に限り、共有メモリで Confirmed にする**。範囲外は降格(`Unknown`)のみ。検証の基準・結果は [adr265-applied-confirmed-verification.md](adr265-applied-confirmed-verification.md)・[結果](adr265-belief-experiments-result.md)。
3. 規則 R の条件 3(`update_tick_ms` が直前の操作より後)は **`>=`**(利用者決定。strict だと `GetTickCount64` の分解能 約 15.6ms で便益がほとんど出ない)。`>=` の危険は未検証(§8 の V4)。
4. IME へ送る根拠(`derive_actuating`・授権)には**入れない**(`BeliefOnly`)。「すでにその状態なら送らない」は `applied` が担う(§5)。

## 1. 目的と、やらないこと
- 目的: mozc-readable が動いている窓で、(a) IME の開閉の belief を共有メモリで正す、(b) 古い `applied` が「一致」と言って必要な送信を省く(ADR-214 の P1)のを防ぐ。
- やらない: `input_mode` には入れない。`desired_open` を書かない。IME へ何も送らない(drift correction 経由も含めて。§3)。

## 2. belief への入れ方(Part A)

| 項目 | 内容 | 理由 |
| --- | --- | --- |
| source | `ObservationSource::MozcShm`(+ `evidence::MozcShm`)。信頼度 High、authority `BeliefOnly` | 利用者決定。授権(`open_warrant.rs` Step 3 の `derive_actuating`)には載せない |
| 入口 | `ImeStateHub::report_mozc_shm_open(open, hwnd, focus_epoch, tick)`(`report_conv_open_inference` の隣)。呼び出し元は 1 つ | `architecture_guard` の件数・`observation_source_guard` の対象を更新 |
| 読む値 | `tsf_open` を既定。`tsf_open != effective_open` のときは報告しない | 割れる場面は未測定(E4 は結論なし)。割れているときは採らない |
| 窓の照合 | `derive_filtered` の `is_identity_ok` に `MozcShm` を足す(epoch+hwnd)。**加えて**、`most_recent_trusted` / `_excluding` にも照合を入れる、または TTL(下)で足りるなら不要 | レビュー H2: 鮮度・照合は `derive_filtered` の中だけで、フォールバックは見ない |
| 鮮度(古い値を残さない) | `record_any` で `source == MozcShm` のときだけ `expires_at = Some(at + TTL)`(`at` から決まる値で、journal 再生でも同じ)。TTL は tuning 定数(`#[measured]`)。受け入れている間は毎 tick 報告し、読めない・`active=0`・別窓の tick から報告が止まる | レビュー H2: 既存の `expires_at` と `is_expired` を全読み手(`most_recent_trusted` を含む)が見るので、3 つの読み手で同時に失効する。TTL の値は報告間隔(約 500ms)+余裕から導き、実測を添える(tuning-constants) |
| 報告の頻度 | 値が変わったときと、TTL の半分ごと | レビュー M3: 毎 tick だと journal の State の枠(2048)が約 17 分で入れ替わる |
| 読む対象 | mozc-readable の TIP が有効な窓だけ(GJI 系の TIP のとき、または (tid, pid) ごとに `NotFound` を覚えてフォーカスが変わるまで読まない) | レビュー M4: 自動判定で全利用者に毎 tick の Win32 呼び出しが走るのを避ける |

### 2-1. drift correction との相互作用(C1。実装前に必須)
- `evaluate_drift` の根拠(`most_recent_trusted_excluding(now, &[ConvOpenInference])`)と、`read_back` の `EXCLUDED_FROM_ANY_FRESH_EVIDENCE` から `MozcShm` を**除外**する。除かないと、High の共有メモリが明示意図と食い違うたびに drift correction が IME を開け直し(Standard の窓)、Blind の窓では BUG-114 型の再武装ループと BUG-124 型の「@」を起こす。
- 除外だけでは belief が直らない(C2)ので、§2-2 と組にする。

### 2-2. 外から変わったときの追随(C2)
- `resolve_open_at` の優先順は 明示意図(`last_intent`)→ 打鍵予測 → `derive_any`。High の `MozcShm` は 3 番目からで、さらに上に `IntentStore`(ON 10 秒・OFF 30 秒、hwnd ごと)がある。外から閉じられても belief は直らない。
- 共有メモリの読みが「**awase の操作より後に、外で変わった**」と判定できたとき(`update_tick_ms >= max(awase の最後の開閉の書き込み, 最後のモードキー)`、かつ値が belief と食い違う)は、既存の追随経路と同じ 3 手を実行する: `report_mozc_shm_open`(High)→ `intent_store.remove(hwnd)` → `pass_through_observed(tick, true, true)`(`last_intent` を捨て、`desired_open` を観測に揃え、食い違う `applied` を `Unknown` に落とす)。手本は `platform_state.rs` の `follow_external_change_in_scope` / `follow_direct_read_in_scope`。
  desired が事実に揃うので、drift correction は発火する理由を失う。`ModeKeyPassedThrough` の新しい呼び出し元を足すだけで、新しい `ImeEvent` variant は要らない(`ime_event_guard` もそのまま通る)。`architecture_guard.rs` の「3 手の順序」のガードを新しい関数にも足す。
- 「awase の操作より前」の読み(`update_tick_ms < 基準`)は、追随せずに捨てる。
- 打鍵予測の照合(`KEY_EFFECT_SETTLE_MS = 170`)の前に、`update_tick_ms < 予測の時刻`(打鍵より後に変化していない)なら「処理前」として照合しない(レビュー M1)。実機の遅れは最大 95ms(E3)で 170ms に余裕があるが、負荷時・TSF ネイティブの窓は未測定。

## 3. IME へ何も送らないことの確認
`BeliefOnly` は `derive_actuating` にだけ効く。drift correction と `read_back` は authority を見ない(C1)ので、§2-1 の除外が「何も送らない」を成り立たせる。テストで固定する(§7)。

## 4. 読む頻度
既存の約 500ms の tick のまま。キー直後の約 0.5 秒は古い belief のまま(現状と同じ)。pulse(`Local\mozc-readable-ime-changed`)での読み直しは別の段階。

## 5. `applied` の確認済み化(Part B。ADR-214 の再開に当たる)

### 5-1. なぜ要るか
押下以外の送信(ツールバー、アプリ起点のモード変更、トレイ・Engine コマンド、drift correction)は、awase の外で変わった IME の状態に対して、古い `applied` を根拠に「一致」と見て**送信を省く**ことがある(ADR-214 の P1)。省略の判断は `DecisionInputs.shadow_on` = `applied_open()`(awase が最後に書いた値の記録)を見る。belief(`effective_open`)は見ない。したがって belief を直しても省略は直らない。

### 5-2. 規則 R(利用者決定: 条件 3 は `>=`)
共有メモリの 1 回の読みから `applied` を Confirmed(`eff_open`)にしてよいのは、次を**すべて**満たすとき:
1. 受け入れ条件を通る(magic・version・seq・pid・`active=1`)+窓の照合(epoch・hwnd)。
2. `tsf_open == effective_open`。
3. `update_tick_ms >= max(直前の awase の開閉の書き込みの `GetTickCount64`, 直前の打鍵の `GetTickCount64`)`。
満たさないときは何も書かない(または、食い違いが分かっているなら §2-2 の降格)。

### 5-3. 適用する窓の範囲(検証が通った種類だけ)
- 確認済み: 素の Win32 窓(IMM で読める窓)と Edge(TSF ネイティブ)(A: 各 40 回、食い違い 0。B: 素の Win32 窓 50 回、食い違い 0。古い読みは観測されず)。
- **範囲外は降格のみ**(`Unknown` に落とすだけ。Confirmed は書かない): ツールバーのメニュー、パスワード欄、Chrome・Windows Terminal・その他の TSF ネイティブ、負荷時、複数窓、連続した打鍵。
- 窓の種類の判定をコードでどう書くか(クラス名の表か、`AppKind` か)は、ADR で決める。TsfNative を一括で入れず、**確認済みの窓の種類を列挙する**(列挙に無ければ降格のみ)。

### 5-4. 書き込み点(レビュー H3)
- `record_confirmed_in_scope` は reduce を通らず、`note_awase_write_for_mode_key_pass_in_scope`(モードキー通過の記録)と `confirm_applied`(pending の解除)の副作用を持つ。**流用しない**(awase の apply が飛行中に共有メモリが同じ値を示すと、pending を消して本物の完了を `Stale` にし、warmup の副作用が走らなくなる)。
- 新しい `ImeEvent` variant(例 `AppliedConfirmedByShm { open, scope }`)を `reduce()` に足す。pending と mode-key-pass には触れない(テストで固定)。journal の直列化形式(ADR-082)・`ime_event_guard`(dylint)・`architecture_guard` の構築件数・`applied_direct_assignments_are_accounted_for` を更新する。
- `AppliedImeState` で Confirmed と Optimistic を区別する本番コードは無い(`is_confirmed()` の本番の呼び出し 0 件)。したがって実際の効果は「`Unknown` を `Some(x)` に戻す」ことと、古い `Some(!x)` を `Some(x)` に直すこと。ADR-214 決定 1(`Sent`/`Confirmed` の型の分離)はしない。API 成功で Confirmed が書かれる問題は残り、共有メモリで読める窓だけは次の tick で真値に上書きされる。

### 5-5. 残る危険(隠さない)
- **E1 の不一致が未解決**: 外からの変更(ツールバー等)が共有メモリに出ない場合が本当にあるなら、`applied` が古いまま Confirmed として残り、必要な送信を省く(逆向きの P1)。規則 R では防げない(R は新しい読みの正しさを守るが、公開されなかった変化は見えない)。**未解決のまま実装に進むなら、この危険を ADR に書いて受け入れる**。
- `>=` の危険(直前の別の変化が同じ 15.6ms の目盛りに書かれた古い読みを通す)は未検証(§8 の V4)。
- TsfNative の窓では Engine 経路の押下も `applied` を未知にしない(`ENGINE_PRESS_UNKNOWNS_APPLIED_IN_TSF_NATIVE=false`)ので、`applied` が誤っていれば押下の送信も省かれる(H1)。確認済みの窓の種類を絞る理由。

## 6. 段階
| 段階 | 内容 |
| --- | --- |
| 1 | Part A(§2)+drift の除外(§2-1)+追随(§2-2)+TTL。IME へ何も送らない |
| 2 | Part B(§5)。範囲は確認済みの窓の種類だけ。降格は範囲外にも適用 |
| 3(別 ADR) | pulse での読み直し、`input_mode`、確認済みの範囲の拡大(測定した窓の種類から) |

## 7. ガードとテスト
- `drift_correction`: `MozcShm` だけが根拠のとき `evaluate_drift` が `Ok` にならない。`read_back` の `AnyFreshEvidence` の除外のテスト。
- `observation_store`: (i) TTL の失効が `derive_any`・`most_recent_trusted`・`_excluding` の全部に効く、(ii) 窓の照合(epoch・hwnd)、(iii) `BeliefOnly` が `derive_actuating` に入らない、(iv) High は ObserverPoll(Medium)と食い違っても採られる(利用者決定)。
- 追随: `update_tick_ms >= 基準` で追随する、`< 基準` で追随しない、3 手の順序のガード、`pass_through_observed` で食い違う `applied` が `Unknown` に落ちる。
- Part B: 規則 R の 3 条件それぞれの否定、範囲外の窓で Confirmed を書かない、pending と mode-key-pass に触れない、`update_tick_ms == 基準`(`>=` の境界)の扱い。
- `architecture_guard`: 新 source の網羅 `match`(`observation_store.rs` の `get`/`set`、`ime_event.rs` の `authority`)、新 variant の構築件数。`lints/observation_source_guard`: 新 source の構築を `report_mozc_shm_open` だけに限る。
- 回帰の置き場所: `fix-requires-evidence` の「IME belief」と「force-write / actuation」のファミリー。同じ PR にテストを足す。tuning 定数(TTL)は実測値を本文に書く。

## 8. 未確認・追加の検証
- V4(`>=` の安全性): 直前の別の変化を同じ目盛りに作る(2 つの IME キーを数 ms 以内に連続で送る)と、古い読みが規則 R を通るか。実装前か、段階 2 の合格条件に入れる。
- E1 の不一致の原因(外からの変更が共有メモリに出るか)。
- TTL の値(報告間隔と読みの遅れの実測から)。
- 確認済みの窓の種類の列挙(クラス名・`AppKind` の表)。
- 「新しい窓の最初のキーは効かない」(先に a+Enter を 1 回打つと効く)の因果。BUG-200 との関係。別の調査。
- 複雑性: 新 source・新 evidence・新 `ImeEvent` variant・ハブの入口 2 本・tuning 定数 1 本・窓種の表。ADR に、削除できるものとの対応(1-in-1-out の趣旨、`complexity-budget.md` は未発効)を書く。
