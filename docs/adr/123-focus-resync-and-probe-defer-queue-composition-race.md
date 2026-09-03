# ADR-123: GJI cold-start 直後、TSF per-VK confirm の連鎖中に `OUTPUT_GATE` が握り続けた物理キーが `pending_deferred` へモーラ境界なく合流し、文字脱落・順序入替が起きる

## ステータス

**round 1（Opus 2体、architect/premortem）完了・未収束。round 1 の指摘を受けて
実機 journal（`report_id: 01M1JJD54XQXSEJTHHFKV1WKA1`）を再解析し、根本原因の
記述を全面的に書き直した（本版）。round 2 レビューへ進む。**

[GitHub issue #148](https://github.com/cuzic/awase/issues/148) として追跡。

### round 1 で確定した誤り（この版で訂正済み）

architect・premortem の双方が独立に、当初案（round 0）の根本原因記述が
実コードと不整合であると指摘した。要旨:

- **F-1**: 当初案は cold-start パスとして `send_romaji_as_tsf` →
  `LiteralDetectFsm`（word-level、単一 300ms 期限）を挙げていたが、
  `LiteralDetectFsm` は `send_romaji_as_tsf_warm`（**warm 専用**、
  `literal_detect_fsm.rs:472` の docstring 明記）からしか呼ばれず、
  cold-start では到達しない。
- **F-2**: 当初案は `FOCUS_RESYNC` gate が複数モーラ分の物理キーを堰き止める
  としていたが、`handle_wm_drain_output_queue`（`message_handlers.rs:1300-1420`）
  は `FOCUS_RESYNC.is_gate_active()` を一切参照せず、2打鍵目到着で
  `INPUT_DEFER.replay_later()` → 無条件 `post_drain_output_queue()`
  （`input_defer.rs:64-67`）により resync gate は事実上1打鍵しか堰き止めない。
- **A-5**: 当初案の因果チェーンは症状の一部（「え」の脱落）しか説明できず、
  「ば」が先頭に来る順序入替を説明できていなかった。

これらの指摘を受け、`report_id: 01M1JJD54XQXSEJTHHFKV1WKA1` の生 journal を
R2 から再取得し実際のフィールド値を確認した（下記「実機 journal 解析」節）。
結果、**F-1 は round 1 の推測（`ChromeProbe`/Vk モード）とも当初案（word-level
`LiteralDetectFsm`）とも異なる第3の経路（TSF モードの per-VK confirm、
`target: Tsf`）が実際に走っていたことが判明**した。この報告環境の
`config.toml` に `[[app_overrides.force_tsf]]` で `WindowsTerminal.exe`
（`CASCADIA_HOSTING_WINDOW_CLASS`）が明示登録されていたため、Vk モードの
既定挙動を疑った round 1 の推測もそのままでは成立しない。詳細は下記。

## 背景

Windows Terminal（TsfNative、かつ config で `force_tsf` 指定）+ GJI で
「たとえば」と入力したところ「ばたと」に文字脱落・順序入替した
（`report_id: 01M1JJD54XQXSEJTHHFKV1WKA1`、app_version 1.18.0、
`WrongCharacterOutput`、2026-09-03T02:43:07Z 報告）。半角英数持続トグル機能
（BUG-25）自体は正常動作中であり無関係と確認済み。

### 既存の関連バグ（参考）

- **BUG-38**: `RawTsfLiteralRecovery` の give-up 分岐が `pending_deferred`
  （probe 実行中に届いた後続キーの退避キュー、`TsfWarmupCoordinator` 所有）を
  flush しないため出力順が入れ替わる、という酷似した過去バグ。修正済み。
- **BUG-89**: gate 中（`OUTPUT_GATE`/`FOCUS_RESYNC`）に defer されたキーは
  `INPUT_DEFER` へ退避され、`handle_wm_drain_output_queue` から
  `KeyOrigin::DeferredReplay` として再生される。
- **BUG-45/BUG-75/ADR-122**: per-VK confirm の literal 判定は「代理指標
  （候補ウィンドウ SHOW / GJI I/O バイト増加）のタイムアウト」に基づく
  belief であり、実際の TSF composition 状態と乖離しても検出も訂正もできない
  構造的欠陥がある。ADR-122 は「着弾したかどうかを事後的に推測する路線は、
  前提を反転させても必ずどちらかの実機ケースで破綻する」と結論している
  （issue #149、実装ペンディング中）。

## 実機 journal 解析（round 1 後に追加、確定的な一次証拠）

`services/report-worker` から `wrangler r2 object get` で report JSON を再取得し、
`log_excerpt`（`JournalEntry` の JSON 配列、943件）を `elapsed_ms` 順に精査した。
`config_toml` に以下が含まれることをまず確認した:

```toml
[[app_overrides.force_tsf]]
process = "WindowsTerminal.exe"
class = "CASCADIA_HOSTING_WINDOW_CLASS"
```

この設定により、本報告の Windows Terminal は **TSF 注入モード**で動作していた
（round 1 の architect/premortem が推測した「既定は Vk モード」は、この
ユーザー環境には当てはまらない）。

### 事実1: 全 probe が `path: "PerVk"`, `target: "Tsf"`（F-1 の決着）

インシデント窓（`elapsed_ms` 12474000〜12481000、windowsterminal.exe への
フォーカス到達から離脱まで）に記録された3件の `LiteralDetect` エントリ
（`cold_seq` 235/236/237）は、いずれも次の形だった:

```json
{"cold_seq": 235, "verdict": "SuspectedLiteral", "route": "CheckNow",
 "path": "PerVk", "target": "Tsf", "vk": 84 /* 'T' */, "idx": 0, "last_idx": 1,
 "gave_up": false, "backs": 1, "romaji": "ta"}
{"cold_seq": 236, "verdict": "SuspectedLiteral", "path": "PerVk", "target": "Tsf",
 "vk": 84, "idx": 0, "last_idx": 1, "consecutive_before": 1, "gave_up": true,
 "backs": 1, "romaji": "ta"}
{"cold_seq": 237, "verdict": "CompositionConfirmed", "path": "PerVk", "target": "Tsf",
 "vk": 66 /* 'B' */, "idx": 0, "last_idx": 1, "write_delta": 315,
 "evidence_fresh": true}
```

**これは round 0 の当初案（word-level `LiteralDetectFsm`）でも round 1 の
architect/premortem の推測（Vk モードの `ChromeProbe`）でもなく、
`gji_warmup_coro.rs:172-181` の `run_per_vk_confirm(..., TransmitTarget::Tsf)`
経路である**（premortem が C-1 で「cold-start の TsfNative+GJI では既に
per-VK confirm が強制されている」と述べた指摘が、経路名まで含めてそのまま
実証された形）。`target: Tsf` は `force_tsf` 設定により TSF 注入モードの
per-VK 確認が使われたことを示す。

### 事実2: 物理キーは probe 進行中に約130〜320ms の追加遅延を伴って
バースト処理されている（gate 保持の直接証拠）

`KeyInput` エントリは `event.timestamp_us`（実際の物理キー押下時刻、フック
記録）と、エントリ自身の `elapsed_ms`（journal 記録時刻＝エンジン処理時刻）
の両方を持つ。両者の差分を全 `KeyInput` について計算すると、通常時は
**一定の基準オフセット ~532ms**（クロック基準の違いによる固定値、実処理遅延
ではない）で安定している。ところがインシデント窓内の2つのバースト
（`seq 42404-42409` と `seq 42415-42418`）だけ、この差分が **664〜848ms**
（基準比 +132〜+316ms の追加遅延）まで跳ね上がり、バーストが終わると
即座に基準値へ戻る:

| seq | 内容 | 基準超過分（追加遅延） |
|---|---|---|
| 42404 (vk=74 down) | 「と」系の1鍵目 | +316ms |
| 42406 (vk=29 LeftThumb down) | 同バースト | +145ms |
| 42409 (vk=29 LeftThumb up) | 同バースト末尾 | +25ms |
| 42415 (vk=29 LeftThumb down) | 「え」系の1鍵目 | +314ms |
| 42418 (vk=72 up) | 同バースト末尾 | +158ms |
| 42426 (Backspace down、バースト外) | recovery 操作 | 基準値に復帰 |

さらに、各バースト内の複数 `KeyInput` はいずれも `elapsed_ms` が2〜3ms差の
中に密集しており、`state_before`/`state_after`（NICOLA 同時打鍵 FSM の
`Idle`→`PendingChar`/`PendingThumb`→`Idle`）が**一括で連続処理**されている
ことを示す。これは「物理キーがどこかのキューに溜まり、probe 完了のたびに
同期ループで一気にリプレイされる」という当初案の中核メカニズム（step 5, 9）
を、GJI 内部の推測に頼らず awase 側の一次データだけで裏付ける。

**gate の種類の特定（F-2 への追加証拠、ただし完全な決着ではない）**: journal
は `OUTPUT_GATE`/`FOCUS_RESYNC` のゲート状態そのものをフィールドとして
記録していないため、どちらが保持していたかを journal だけから断定はできない。
しかし2つのバーストの発生タイミングは `TsfProbeStarted`（cold_seq=442 at
12474411 → 443 at 12475099 → 444 at 12475445）と連続的に一致し、
`FocusTransition` の `dwell_ms` とは相関しない。round 1 の両エージェントが
コード読解で示した「`FOCUS_RESYNC` は2打鍵目で事実上解除される」という結論
（F-2）と整合し、**`OUTPUT_GATE`（TSF probe が保持する `OutputActiveGuard`）
が実際の保持機構である可能性が高い**。ただし直接証拠ではないため、
「未決定事項」に実装での確認（gate 種別のログ出力追加）を残す。

### 事実3: 「た」は give-up（backspace のみ、resend なし、reinit も発火せず）、
「ば」だけが `write_delta=315` という異常値で単独確定した

- cold_seq=236 の give-up（`consecutive_before=1`, `gave_up=true`, `backs=1`）
  の後、**「た」に対する3回目の probe（romaji 再送を伴う）は一度も観測されない**。
  次の probe（cold_seq=444/237）は `vk=66`（'B'）から始まっており、これは
  「た」ではなく別の romaji（`last_idx=1` なので2 VK、'B'+'A' = "ba" と推定）
  の確認である。
- インシデント窓内に `ImeActuation`/`ImeEvent`/`ImeOpenApplied` の
  `VK_IME_OFF`→`VK_IME_ON` reinit に相当するエントリは**一件も無い**
  （観測された `ImeOpenApplied` は2件とも `reason: "ImmBrokenForceOn"` で
  give-up 由来ではない）。**round 1 の architect が A-4 で示した
  「give-up → GJI reinit 予約 → focus 変化で `pending_deferred` 全破棄」
  という経路は、少なくとも本インシデントでは発火していない**（コード上
  存在する経路自体を否定するものではないが、本件の直接原因ではない）。
- cold_seq=237（probe 444）は `GjiFsmTransition: StartComposition(candidate
  SHOW)` の直後に `CompositionConfirmed`（idx=0, `write_delta=315`）→
  （47ms後）idx=1 も confirmed・`session_marked=true` という順で完了して
  いる。`write_delta=315` は他の全 `LiteralDetect` エントリ（`write_delta=0`
  が大半）と比べて突出して大きく、issue 本文が「異常に大きい一括書き込み」
  として観測した値と一致する。

**解釈**: 「た」は per-VK confirm で2回とも合成兆候が確認できず give-up
（backspace(1) のみ、resend も reinit も無し）となり、literal のまま出力に
残った（最終出力の「た」はこの literal 残留と推定される）。一方、2つの
物理キーバースト（「と」「え」に対応すると推定される）は engine を通過した
ものの、probe が in-flight（cold_seq 443→444 の連鎖）だったため
`pending_deferred` に積まれ、次に始まった probe（「ば」の romaji 送信が
トリガー）の flush に巻き込まれて **1回の異常に大きい合成書き込み
（write_delta=315）に融合**した。この融合の結果 GJI 側で「え」が消え、
確定した文字列の先頭に「ば」が来る形で出力が乱れた、という機序は当初案の
仮説と一致するが、**「え」が消え「と」が生き残った、という具体的な区別が
なぜ生じたかは GJI 内部の解釈に依存し、journal からは確定できない**
（引き続き「推測」）。

## 根本原因（round 1 訂正 + journal 実証を反映）

**Windows Terminal を `force_tsf` 指定した TsfNative + GJI の cold-start で、
連続する物理キー入力が TSF per-VK confirm の probe 連鎖（`run_per_vk_confirm`,
`target: Tsf`）と競合する。probe が in-flight の間に届いた物理キーは
`OUTPUT_GATE`（可能性が高いが直接未確認）に堰き止められ、probe 完了のたびに
同期的にバースト処理される。バースト中に別の probe が新たに in-flight になると、
そのモーラの romaji は `pending_deferred`（モーラ境界を持たないフラット
`Vec<DeferredVk>`）に落ち、次の probe の flush 時に無差別に合流する。この
合流が異常に大きい単一の GJI 合成イベント（`write_delta=315`）を生み、
文字脱落・順序入替として観測される。**

### 因果チェーン（journal 実証部分と未実証部分を明示）

1. **フォーカス変更**: windowsterminal.exe（`Windows.UI.Input.InputSite.WindowClass`、
   `force_tsf` によりプロファイル `TsfNative`）へフォーカス到達
   （`elapsed_ms=12474411`、直前 explorer.exe の dwell はわずか156ms）。
   **【実証】** `FocusTransition`/`GjiFsmTransition(FocusChange)` で確認。
2. **フォーカス到達直後に自動 probe 開始**: `TsfProbeStarted cold_seq=442`
   （`source: GjiAction::StartProbe`）。ユーザーの打鍵を待たず、フォーカス
   変更そのものがトリガー。**【実証】**
3. **「た」romaji 送信 → per-VK confirm 開始**: `ConvClassifyCall` → romaji
   "ta" 相当のキー入力が engine を通過。**【実証、ただし romaji 自体は
   `LiteralDetect.romaji` フィールドからの逆算】**
4. **「た」の1文字目('T')が2回とも SuspectedLiteral → give-up**:
   `cold_seq=235`（consecutive=0、backspace 予約のみ）→
   `cold_seq=236`（consecutive=1、`gave_up=true`、backspace(1)、resend 無し）。
   `since_vk_sent_ms` 297ms/313ms は `RAW_TSF_LITERAL_DETECT_MS=300ms`
   （`tuning.rs:72`）と一致。**【実証】**
5. **この間、後続の物理キー（「と」「え」相当）が2バーストに分かれて
   engine に到達、いずれも基準オフセット比 +130〜+320ms の追加遅延を伴う**。
   バースト内部は2〜3ms間隔で密集処理される（同期リプレイの特徴）。
   **【実証】** ただし堰き止めていたのが `OUTPUT_GATE` か `FOCUS_RESYNC` かは
   **【未実証、コード読解からは `OUTPUT_GATE` が濃厚】**。
6. **後続モーラの romaji が probe in-flight 中に `pending_deferred` へ
   落ちる**: `defer_if_probe_in_flight`（`output/mod.rs:1277-1290`）→
   `warmup_coord.pending_deferred`（`tsf_warmup_coord.rs:296-305`、
   `DeferredVk{vk,needs_shift}` のみでモーラ境界を持たない）。**【コード上
   確定、本インシデントでの発火は状況証拠（事実2・3）から強く示唆されるが
   直接ログには出ない】**
7. **「ば」の probe（`cold_seq=444/237`）が `pending_deferred` を巻き込んで
   flush、単一の異常に大きい合成イベントとして確定**: `write_delta=315`。
   **【実証（この値そのもの）、ただし「なぜえが消えてばが生き残ったか」の
   GJI 側解釈は推測】**
8. **give-up した「た」は resend も reinit も発火せず literal のまま残留**。
   **【実証（reinit 不発火は journal に IME OFF/ON イベントが無いことで確認）】**

### 関係ファイル・関数一覧（訂正版）

| 役割 | file:line |
|---|---|
| TSF cold-start の per-VK confirm 経路（本件の実際の経路） | `crates/awase-windows/src/tsf/warmup/gji_warmup_coro.rs:172-181` |
| per-VK confirm 本体 | `crates/awase-windows/src/tsf/warmup/probe_fsm.rs:436-475` |
| per-VK の SuspectedLiteral/give-up 判定 | `crates/awase-windows/src/tsf/warmup/literal_detect_fsm.rs:260-415` |
| `per_vk_recovery_params`（backspace数・resend要否の決定、ADR-122 案Fの対象と同一） | `crates/awase-windows/src/tsf/warmup/literal_detect_fsm.rs` 内 |
| give-up 時の GJI reinit 予約（本件では不発火） | `crates/awase-windows/src/output/probe_io.rs:745-760` |
| `pending_deferred` 実体（モーラ境界なしフラット Vec） | `crates/awase-windows/src/output/tsf_warmup_coord.rs:51-57,296-337` |
| `defer_if_probe_in_flight` | `crates/awase-windows/src/output/mod.rs:1277-1290` |
| `flush_pending_deferred_vks` / `flush_stale_deferred_vks_after_recovery` | `crates/awase-windows/src/output/mod.rs:1698-1766` |
| `split_vk_runs`/`send_vk_run_batch`（down全部→up全部の重畳順バッチ） | `crates/awase-windows/src/output/key_injector.rs:226-260` |
| `WM_DRAIN_OUTPUT_QUEUE` ハンドラ（同期リプレイループ） | `crates/awase-windows/src/runtime/message_handlers.rs:1300-1420` |
| `deferred_engine_timers` replay（drain ループ以外のromaji送出点、round1 A-7指摘） | `crates/awase-windows/src/runtime/message_handlers.rs:1391-1408` |
| `FOCUS_RESYNC` gate が2打鍵目で事実上解除される経路（round1 F-2） | `crates/awase-windows/src/app/mod.rs:485-506`, `crates/awase-windows/src/input_defer.rs:64-67` |
| `OUTPUT_GATE`/`OutputActiveGuard` | `crates/awase-windows/src/tsf/probe_bridge.rs:24-149` |
| `RAW_TSF_LITERAL_DETECT_MS = 300ms` | `crates/awase-windows/src/tuning.rs:72` |
| `force_tsf` 設定によるプロファイル判定 | `config.toml [[app_overrides.force_tsf]]`（本報告環境で `WindowsTerminal.exe` 登録済み） |

## BUG-38 との異同

**別の欠陥（BUG-38 の再発ではない）と判断する。** BUG-38 は「同一 probe の
give-up サイクル内での `pending_deferred` flush 順序」を保証するのみで、
「probe 連鎖（cold_seq 442→443→444）が続く間に複数モーラの romaji が
`pending_deferred` へ無区別に混入すること自体」は対象にしていない
（`DeferredVk` にモーラ境界フィールドが無いことからも設計時点で未想定）。
本件は journal 実証により、この混入が実際に起きていたことを示す状況証拠
（2つの遅延バースト＋単一の異常に大きい `write_delta`）を得た。

## 検討する選択肢（round 1 の指摘を反映して再評価）

### 選択肢A: `pending_deferred` にモーラ境界を持たせる

- **round 1 評価**: 対症。加えて premortem A-2 が指摘する通り、モーラ境界の
  ない同型のフラットバッファが他に2箇所存在する（`UnicodeColdWarmupFsm::deferred_chars`
  `unicode_cold_warmup_fsm.rs:30`、`Output::unicode_cold_deferred`
  `platform.rs:974`）。Aを採るなら3箇所同時対応が必須。architect A-6 の
  指摘通り `send_vk_run_batch` の呼び出しを分割すると単一 `SendInput` が
  持っていた「メッセージポンプが割り込めない」暗黙の保護を失い、BUG-02系
  race の再燃リスクを新設する。
- **本ADRでの位置づけ**: 単独では不十分。実装するなら防御的な二段目として、
  根治（下記D）とセットで扱う。

### 選択肢B: drain ループを probe in-flight 化した時点で中断する

- **round 1 評価**: **却下**。`should_post_drain` の不変条件
  （`state/focus_resync_policy.rs:48-50`、「`OUTPUT_GATE` active 中に
  defer 済みキーを replay するな」）を直接破り、BUG-02/BUG-70 系のリテラル
  漏れ経路を再生産する（B-1、blocker）。`replay_later` の無条件 post による
  ビジースピン（B-2）、`InputDeferQueue` に purge API が無いことに起因する
  フォーカス変更時の別ウィンドウへのキー漏れ（B-3）、reinject 順序逆転
  （B-3'）、合流点の見落とし（B-2、`deferred_engine_timers` replay 等
  少なくとも5箇所）という複数の blocker が round 1 で確定した。**採用しない。**

### 選択肢C: cold-start 直後だけ per-VK confirm を強制する

- **round 1 評価 + journal 実証**: **完全に no-op と確定**。journal 事実1
  の通り、本インシデントは最初から `path: "PerVk"` で走っている——Cが
  強制しようとしている状態が既定で発生済み。**却下。**

### 選択肢D（architect 提案、round 1 で新規）: drain バーストを「1つの出力エピソード」として扱う

`INPUT_DEFER.take_all()` で得たバースト全体に対し、**先頭で1回だけ probe を
張り、以降のモーラは新規 probe を張らずに（＝`pending_deferred` へ落とさず）
順次そのまま送る**。バーストの境界（gate 期間の入口/出口）でフラグを立て
下ろす。

- **長所**: `pending_deferred` へ複数モーラが混入する現象そのものが起きない。
  Bのようにループを中断しないため B-1/B-3 系の blocker が発生しない。
  `deferred_engine_timers` replay（A-7）も同じフラグのスコープに含めれば
  B-2 の合流点漏れが構造的に起きない。
- **短所**: 先頭モーラの probe が give-up した場合、後続モーラも同じ理由で
  literal 化しうる——ただしこれは既存リスク（cold_seq=236 の「た」で実際に
  起きた事象そのもの）の露出であり、新規リスクではない。
- **本 journal の事実との整合性**: 事実2で観測された「probe 連鎖のたびに
  新しいバーストが処理される」パターンをDが直接解消する形になっており、
  観測データと最も整合する選択肢。
- **実装コスト**: 低〜中（architect 見積り）。

### 選択肢E（architect 提案、根治だが今回は採らない）: 出力シンクの一本化

engine が確定した romaji が実際に `SendInput` に到達する経路は
`send_romaji_batch_immediate`／`pending_deferred` flush／`RAW_TSF_LITERAL`
の backspace+resend の最低3本あり、相互の順序保証が無い。BUG-38・BUG-75・
本件はいずれもこの構造的欠陥の別の顔。シーケンス番号付きの単一出力キューへ
一本化するのが本来の根治だが、ADR-079 Stage2 相当の規模でコストが高い。
**今回は採用しないが、「なぜ今回は採らないか」を記録に残す**
（`.claude/rules/experiment-logging.md` の趣旨）。

## 決定（暫定、round 2 レビュー待ち）

**選択肢D（バースト単位の単一probe化）を軸に、選択肢A（モーラ境界、
`UnicodeColdWarmupFsm`/`platform.rs` 含む3箇所同時対応)を防御的な二段目として
組み合わせる方向を暫定 decision とする。** B は blocker により不採用、
C は no-op と実証済みのため不採用。E は将来課題として記録するに留める。

**この decision は round 1 の指摘と round 1後の journal 実証を踏まえた
再構成であり、round 2 でさらに検証する。** 特に以下は round 2 で重点的に
確認する:
- Dの「バースト境界でフラグを立てる」実装が、`deferred_engine_timers` replay
  を含む全合流点（round 1 architect が挙げた最低5箇所）を本当に覆えるか。
- Dが「先頭モーラ give-up → 後続モーラも literal化」という許容した短所が、
  実際にどの程度の頻度で許容範囲か（Aとの組み合わせでどこまで緩和されるか）。
- Aを3箇所同時対応する場合の実装コストの再見積り。

## 未決定事項（round 1 の指摘により優先順位を再構成）

1. **gate の種別特定**（`OUTPUT_GATE` か `FOCUS_RESYNC` か、事実2で状況証拠は
   得たが直接証拠ではない）: 実装時にログへ gate 種別を明示出力する対応を
   先行させるべきか検討する。
2. **`FOCUS_RESYNC` gate が2打鍵目で事実上解除される件（F-2）**: 本ADRの
   スコープ外の独立した別欠陥（BUG-77 が塞いだつもりの穴の残り）と判断。
   別 BUG 番号を採って切り出すことを推奨（番号衝突に注意）。
3. **横展開**: `UnicodeColdWarmupFsm::deferred_chars`・
   `Output::unicode_cold_deferred` の同型フラットバッファ2箇所、および
   `send_keys` の defer 非対称性（`SpecialKey`/`Key`/`KeyUp` は即送信、
   `Romaji` のみ defer 対象、`platform.rs:1000-1020` 既知）は「検討」ではなく
   「対応が必要な既知の穴のリスト」として扱う。
4. **「え」が消え「と」が残った具体的機序**: GJI 内部解釈に依存し
   awase 側からは確認不能。根治（選択肢D/A）で発生自体を防げれば、この
   機序の完全解明は必須ではないと考えるが、round 2 で妥当性を確認する。
5. **レイテンシ影響の実測**（`tuning-constants.md` 規約対象）: 選択肢Dを
   実装する場合、バースト単位の probe 化がレイテンシに与える影響
   （既存のper-VKモーラごとの待ちと比べて改善するはず、だが実測要）。

## 関連

BUG-38、BUG-89（`FOCUS_RESYNC`/`OUTPUT_GATE` の deferred replay 経路）、
BUG-45/BUG-75/ADR-122（per-VK confirm の代理指標ベース判定の構造的欠陥、
「着弾したかどうかを事後推測する路線は反転させても破綻する」という教訓）、
[docs/bug-reports-triage.md](../bug-reports-triage.md)
（`01M1JJD54XQXSEJTHHFKV1WKA1` 該当行）。
