# ADR-265 段階 2b: 共有メモリを belief へ接続する設計(案)

状態: **設計のみ**(2026-10-18、実装は別セッション rust-nicola-05。実装に進むかは、下の「着手の条件」が揃ってから利用者が決める)。
根拠の出典: [ADR-265 D3](../adr/265-mozc-readable-integration.md)、[段階 2 の測定結果](adr265-stage2-measurement-result.md)、
[ime-belief-architecture](../../.claude/rules/ime-belief-architecture.md)。コードは `develop` の `8eef489e` で読んだ(読んだだけで、ビルドも実機も未実施)。

## 1. 目的と、やらないこと

- 目的: mozc-readable が動いている窓で、IME の**開閉**の belief を、共有メモリの値で正す(特に、外から閉じられた・物理キーで変わった後のずれ。BUG-200 の系統)。
- やらない: `input_mode` には入れない(`belief_mode` は romaji/kana の入力方式で、共有メモリの hiragana/katakana とは別物。測定でも開閉だけを比べた)。
  `desired_open` を書かない。IME へ何も送らない(actuation の根拠にしない)。

## 2. 入れ方(既存の型にそろえる)

| 項目 | 案 | 理由 |
| --- | --- | --- |
| 新しい source | `ObservationSource::MozcShm`(+ `evidence::MozcShm`、`Observed::<evidence::MozcShm>::from_...`) | `ConvOpenInference` と同形。`ImmGetOpenStatus` 等の偽装はしない |
| 入口 | `ImeStateHub::report_mozc_shm_open(open, hwnd, focus_epoch, tick)`(`report_conv_open_inference` の隣、`platform_state.rs:2028` 付近) | 呼び出し元を 1 つに絞り、`architecture_guard` の件数と `observation_source_guard` の対象を更新する |
| authority | **`BeliefOnly`** | `derive_actuating`(送信の判断の入力)に入れない。開閉の取り違えは非対称な被害(ADR-191・INV-45)。Actuating にするのは別 ADR |
| 信頼度 | 段階 A: `Low` → 段階 B: `Medium`。**High にはしない** | High は単独で即採用され、他の観測を上書きする。共有メモリは「その窓の TIP の自己申告」で、フォーカスと別スレッドを読む危険が残る |
| 読む値 | `tsf_open` を既定。`tsf_open != effective_open` のときは**報告しない**(journal にだけ残す) | 食い違いの意味(パスワード欄等)が未測定。Miss に倒す |
| 窓の同一性 | `FocusProbe`/`ImmCrossProbe` と同じく、`derive_filtered` の `is_identity_ok` に `MozcShm` を足し、epoch と hwnd の両方を照合 | 窓を切り替えた直後の古い読みの混入を防ぐ(測定で未達の「状態変更直後のフォーカス移動」の対策) |
| 型 | 受け入れ(`Outcome::Accepted`)のときだけ報告。`Rejected`(inactive・pid 不一致・version 等)・`NotFound`・`Failed` は**報告しない** | 誤採用より採らない方が安全 |

## 3. 古い値を残さない(最重要)

共有メモリは、TIP が無効になった後も最後の値を残す(`active=0`)。`active=0` を読んだら**何も報告しない**だけでは、前の `open=true` の観測が残る。

- 案: **受け入れている間は毎 tick 報告し続ける**(いまの影の読みは「変化したときだけ」記録するので、報告用の経路は別にする)。
  観測の鮮度は `OBSERVATION_FRESH_WINDOW_MS = 3000`(`tuning.rs:69`)で切れる。`active=0` を読んだ tick から報告が止まり、最悪でも約 3 秒で効かなくなる。
- 3 秒は「TIP を外した直後の 3 秒間は古い open が残りうる」ことを意味する。測定の基準(`active=0` から 1 秒後の誤採用 0)より緩い。
  **選択肢**: (a) 3 秒を受け入れる(Low/BeliefOnly の段階 A では実害が小さい)、(b) 観測を無効にするイベントを足す(`reduce` に variant が増える。複雑性予算の対象)、
  (c) 源ごとの鮮度を短く(1〜1.5 秒)する(`tuning.rs` の定数追加。実測が要る — 報告の間隔 500ms + 読みの p95 129µs から導くなら 1.2 秒前後)。
  **推奨は段階 A=(a)、段階 B に上げる前に (c)**。`is_expired` が源ごとの TTL を持てるかは**コードで未確認**。

## 4. 読む頻度

- 既存の ~500ms の tick(`mozc_shm_on_tick`)のまま始める。キーの約 61 ms 後に共有メモリは変わるので、**キー直後の打鍵(〜0.5 秒)は古い belief のまま**。
  これは現状と同じで、悪化はしない。
- 次の段階: `Local\mozc-readable-ime-changed`(pulse、取りこぼす前提)を読み直しの合図にする。待ち受けは async タスク(単一スレッドのメッセージループ)で行い、
  `run_with_timeout` に載せるかは別途決める。ここは今回の設計に含めない。

## 5. 段階

| 段階 | 内容 | 挙動の変化 | 次へ進む条件 |
| --- | --- | --- | --- |
| A | `MozcShm` を `Low`・`BeliefOnly` で報告 | `effective_open()` のフォールバック(他に観測が無いときだけ)に効く | journal(`[mozc-shm]` の `belief_open` 列)で、報告後の belief と共有メモリの一致が実機・CI で 100%、誤報告 0 |
| B | `Medium` に上げる | `derive_any` の多数決に参加。他の Medium(ObserverPoll)と食い違うと「結論なし」になり `most_recent_trusted` へ | §6 の条件(flip・tid 再利用・複数窓の物理キー)が測れていること。開閉の基準(N≥200・食い違い 0)は段階 2 で**満たし済み** |
| C(別 ADR) | Actuating にする・pulse で読む・`input_mode` へ | — | — |

## 6. 着手の条件(測定で未達のもの)

段階 2 の基準は満たしたが、次は**測れていない**(結果の文書の「今も測れていないもの」):
(1) 状態を変えた直後のフォーカス移動(flip。別セッションの CI は 0 件処理で未達)、(2) スレッド ID の再利用(再現せず、結論なし)、
(3) 物理キーでの複数の窓・フォーカス直後(実機は Notepad 1 窓、28 回中 belief 誤り 1)、(4) 負荷時の `read_us`(CI で max 114µs。実機は未測定)。
**段階 A は (1) が通れば着手してよい**(Low・BeliefOnly で、挙動への影響が小さいため)。**段階 B は (1)〜(3) のいずれかが 0 件のままなら上げない**。

## 7. 実装の注意(ガードとテスト)

- `architecture_guard.rs`: `ObserverReported` の構築箇所の件数、新しい `ObservationSource` の網羅 `match`(`observation_store.rs:262〜301` の `get`/`set` 2 箇所、`authority`)に variant を足す。
- `lints/observation_source_guard`: 新 source の構築を `report_mozc_shm_open` だけに限る。
- `observation_store` の単体テスト: (i) `Low` は `derive_any` に入らない、(ii) `BeliefOnly` は `derive_actuating` に入らない、(iii) epoch/hwnd が違うと採られない、
  (iv) Medium の ObserverPoll と食い違うと結論なし。
- 回帰の置き場所: `fix-requires-evidence` の「IME belief」ファミリー(`state/ime_model.rs`・`observation_store.rs`・`platform_state.rs`)なので、同じ PR にテストを足す。
- `mozc_shm_shadow = false`(既定)のとき、報告も読みもしない。**段階 A でも設定のオプトインを維持する**案。自動判定(設定なし)にするのは段階 B 以降。
  ADR-265 D3 は「設定なしで自動判定」だが、belief に効く段階では、利用者の環境で壊れても止められる逃げ道を残す方を勧める(未決: 利用者の判断)。

## 8. 未確認・利用者に決めてほしいこと

- 未確認: `is_expired` が源ごとの TTL を持てるか(§3)。`Observed::<evidence::…>` の型の足し方(`ConvOpenInference` の例を読んだだけ)。
- 決めてほしい: (1) §3 の古い値の扱い(a/b/c)、(2) 段階 A でもオプトインを残すか、(3) `tsf_open != effective_open` のとき報告しない方針でよいか。
