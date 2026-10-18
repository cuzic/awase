# ADR-265 段階 2b: 共有メモリを belief へ接続する設計(案)

状態: **設計のみ**(2026-10-18、実装は別セッション rust-nicola-05)。
根拠の出典: [ADR-265 D3](../adr/265-mozc-readable-integration.md)、[段階 2 の測定結果](adr265-stage2-measurement-result.md)、
[ime-belief-architecture](../../.claude/rules/ime-belief-architecture.md)。コードは `develop` の `8eef489e` で読んだ(読んだだけで、ビルドも実機も未実施)。

## 0. 方針(利用者決定、2026-10-18)

- 共有メモリは Mozc 自身の状態の**事実**であり、推測(ObserverPoll・conv ビット等)より優先する。**最初から High で入れる**(Low→Medium の段階は置かない)。
- 理由(利用者): v2 の設計では、モードずれは一定の確率で起きる前提。mozc-readable を入れる利用者は、最初から「事実を優先する動作」を期待している。
  影響は mozc-readable を入れた環境だけ(他の IME ではブロックが見つからず、何も起きない)。
- 自動判定(設定なし)。`mozc_shm_shadow` のオプトインは belief への接続では要求しない(診断用の journal の設定としては残す)。
- High は「事実として採る**窓の照合を通った読み**だけ」に限る。窓を取り違えた事実は事実ではないので、照合は緩めない(§2)。

## 1. 目的と、やらないこと

- 目的: mozc-readable が動いている窓で、IME の**開閉**の belief を共有メモリの値で正す(外から閉じられた・物理キーで変わった後のずれ。BUG-200 の系統)。
- やらない: `input_mode` には入れない(`belief_mode` は romaji/kana の入力方式で、共有メモリの hiragana/katakana とは別物。測定でも開閉だけを比べた)。
  `desired_open` を書かない。IME へ何も送らない。

## 2. 入れ方(既存の型にそろえる)

| 項目 | 案 | 理由 |
| --- | --- | --- |
| 新しい source | `ObservationSource::MozcShm`(+ `evidence::MozcShm`) | `ConvOpenInference` と同形。他の source の偽装はしない |
| 入口 | `ImeStateHub::report_mozc_shm_open(open, hwnd, focus_epoch, tick)`(`report_conv_open_inference` の隣、`platform_state.rs:2028` 付近) | 呼び出し元を 1 つに絞る。`architecture_guard` の件数と `observation_source_guard` の対象を更新 |
| 信頼度 | **High** | 利用者決定。`derive_filtered` の High 単独の採用に乗る(最新の High を採る) |
| authority | `BeliefOnly` | `derive_actuating` は `open_warrant.rs` の Step 3 だけが使う「OS の IME へ書いてよいかの授権」の入力で、「すでにその状態だから送らない」の判断ではない(§5)。授権に使う必要は今は無い |
| 読む値 | `tsf_open` を既定。`tsf_open != effective_open` のときは報告しない(journal にだけ残す) | 食い違いの意味(パスワード欄等)が未測定。事実が割れているときは採らない |
| 窓の照合 | `derive_filtered` の `is_identity_ok` に `MozcShm` を足し、**epoch と hwnd の両方**を照合(`FocusProbe`/`ImmCrossProbe` と同じ) | High は他の観測を上書きするので、窓を切り替えた直後の古い読みを採らないための必須条件 |
| 受け入れ | `Outcome::Accepted`(magic・version・seq・pid・`active=1`)のときだけ報告。それ以外・読めなかったときは報告しない | 採らないより間違って採る方が高くつく |

## 3. 古い値を残さない(High にしたので必須)

TIP が無効になった後も、共有メモリは最後の値を残す(`active=0`)。High の観測が残ると、他の観測を上書きし続ける。

- **受け入れている間は毎 tick 報告し続ける**(いまの影の読みは「変化したときだけ」記録するので、報告用の経路は別にする)。`active=0`・別窓・読めなくなった tick から報告が止まる。
- 観測の鮮度は `OBSERVATION_FRESH_WINDOW_MS = 3000`(`tuning.rs:69`)で切れる。**3 秒は、段階 2 の基準(`active=0` から 1 秒後の誤採用 0)より緩い**。
  High では、TIP を外してから最大 3 秒、古い open が採られうる。選択肢:
  (a) 3 秒を受け入れる、(b) 観測を無効にするイベントを足す(`reduce` に variant が増え、複雑性予算の対象)、(c) この source だけ鮮度を短くする(報告の間隔 500ms + 余裕で 1.2 秒前後。`tuning.rs` の定数追加なので、tuning-constants の規約により実測値を添える)。
  **High にするので (c) を推奨**(段階 A の猶予が無いため)。`is_expired` が source ごとの TTL を持てるかは**コードで未確認**。

## 4. 読む頻度

- 既存の ~500ms の tick(`mozc_shm_on_tick`)のまま始める。キーの約 61 ms 後に共有メモリは変わるので、キー直後の約 0.5 秒は古い belief のまま(現状と同じで、悪化しない)。
- 次の段階: `Local\mozc-readable-ime-changed`(pulse、取りこぼす前提)を読み直しの合図にする。本設計には含めない。

## 5. 段階

| 段階 | 内容 |
| --- | --- |
| 1(本設計) | `MozcShm` を High・`BeliefOnly` で報告。自動判定 |
| 2(別 ADR) | pulse で読む、`input_mode` へ、**「すでにその状態なら送らない」に共有メモリを使う**(下の §5-1) |

### 5-1. 「すでにその状態なら送らない」は belief(High)では効かない(2026-10-18、コードを読んで確認)

- 送らない判断(already-matched)は、`DecisionInputs.shadow_on` = `ImeModel::applied_state().applied_open()`(`AppliedImeState`: Unknown / Optimistic / Confirmed。
  **awase 自身が最後に書いた値の記録**)を見る(`ime_actuation_decision.rs` の `gji_direct_already_matches` など)。`effective_open`(観測から導く belief)は見ない。
  したがって、共有メモリを High で belief に入れても、この省略は変わらない。
- 明示キーの押下(`press.is_some()`)は、`explicit_press_applied_pair` が `applied` を意図的に**未知にして**から決める(押下ごとに送る設計。
  `sync_actuation.rs` のテスト `dispatch_sends_off_for_each_press_even_when_applied_is_already_off`)。共有メモリが「すでに閉」と言っていても、押下の書き込みは省かれない。
- 共有メモリで省略するなら、`applied` を `Confirmed` にする経路(Observe → classify → reduce の新しい入口)と、押下の未知化との関係(ADR-208 L1、ADR-214 Sent/Confirmed)を決める別 ADR が要る。
  `derive_actuating`(授権)には載せない。理由は §2 の authority の行。
- 未確認: 実際に「すでに合っているのに送った」事例がどの程度あるか(journal の `ActuationDecision` で数えていない)。数えてから別 ADR を起こす。

## 6. マージの条件(設計の遠慮の代わりに、測れていないものを条件にする)

段階 2 の基準(N≥200・食い違い 0)は満たしたが、次は**測れていない**。実装 PR のマージ前に、少なくとも (1) を CI で通す:
(1) 状態を変えた直後のフォーカス移動(flip)——**2026-10-18 に CI で測れた**(run `39119069498`、切り替え依頼 158 件を処理、mismatch 0・wrong thread 0・stale 0。結果は `adr265-stage2-measurement-result.md` の追補)。PR #667 のマージを実装 PR の前提にする、
(2) スレッド ID の再利用(再現せず、結論なし。`pid` 照合と epoch/hwnd 照合が防ぐ設計)、
(3) 物理キーでの複数の窓・フォーカス直後(実機は Notepad 1 窓、28 回中 belief 誤り 1)、(4) 負荷時の `read_us`(**測れた**: CI の負荷下で p99 126µs・最大 128µs)。
(2)〜(4) が 0 件のままなら、PR 本文に「未測定」と書いて出す(止めない。利用者決定)。

## 7. 実装の注意(ガードとテスト)

- **drift correction との相互作用(未確認、最優先で読む)**: `platform_state.rs::check_drift_correction` は、`ConvOpenInference` と `HeuristicDefault` だけを明示的に除外するガードを持つ
  (`ime_event.rs` の `HeuristicDefault` の doc)。`MozcShm` が High で明示意図(ユーザーの ON/OFF)と食い違ったとき、drift correction が IME へ再送して**事実と争う**恐れがある
  (BUG-173 追補3 の、GJI で収束しなかった再送の型)。`MozcShm` をこのガードの対象にするかを、実装前に決める。
- `architecture_guard.rs`: `ObserverReported` の構築箇所の件数、新 source の網羅 `match`(`observation_store.rs:262〜301` の `get`/`set`、`ime_event.rs` の `authority`)に variant を足す。
- `lints/observation_source_guard`: 新 source の構築を `report_mozc_shm_open` だけに限る。
- `observation_store` の単体テスト: (i) epoch/hwnd が違うと採られない、(ii) `BeliefOnly` は `derive_actuating` に入らない、(iii) 鮮度で失効する、
  (iv) High は ObserverPoll(Medium)と食い違っても採られる(利用者決定)、(v) `active=0` で報告が止まる。
- 回帰の置き場所: `fix-requires-evidence` の「IME belief」ファミリーなので、同じ PR にテストを足す。

## 8. 未確認・決めること

- 未確認: `is_expired` が source ごとの TTL を持てるか(§3)、`Observed::<evidence::…>` の足し方、drift correction との相互作用(§7)。
- 決めてほしい: (1) §3 の古い値の扱い(a/b/c。推奨は c)、(2) §7 の drift correction のガードに `MozcShm` を入れるか(推奨: 入れて、belief だけを正す。事実と争う再送をしない)。
