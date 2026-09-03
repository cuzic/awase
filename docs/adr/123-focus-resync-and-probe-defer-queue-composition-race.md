# ADR-123: GJI cold-start 直後、TSF per-VK confirm の give-up が予約する GJI reinit と `pending_deferred` の関係が、focus churn 下で1モーラを丸ごと破棄しうる（文字脱落・順序入替の第一仮説）

## ステータス

**round 1・round 2（Opus 2体、architect/premortem）完了・いずれも未収束。
round 2 で「decision を選ぶ段階にまだ到達していない」という結論で両エージェントが
一致した。本 ADR はユーザー指示（2ラウンド）どおり Opus レビューを一旦終了し、
round 2 の到達点と round 3 で行うべき具体的な次の一歩を記録する。**

[GitHub issue #148](https://github.com/cuzic/awase/issues/148) として追跡。

### 収束していない理由（round 2 終了時点のサマリ）

- **選択肢C（cold-start 直後だけ per-VK confirm を強制）は journal 実証により
  完全な no-op と確定した。** 本インシデントは最初から `path: "PerVk",
  target: "Tsf"` で走っていた。
- **選択肢B（drain ループの中断）は `should_post_drain` の不変条件
  （`state/focus_resync_policy.rs:48-50`）を直接破り、BUG-02/BUG-70 系の
  リテラル漏れ経路を再生産するため不採用。**
- **選択肢D（バースト単位の単一probe化）は round 2 で複数の blocker が判明し、
  現状の形では採用できない**（R2-1, R2-5, R2-6, R2-7, R2-8、下記）。
- **「え」が脱落した機序について、round 1 版の仮説（`pending_deferred` の
  モーラ混在フラッシュ）は round 2 で反証された。** 正しい順序では「ば」の
  probe が「え」の deferred flush より**先に**確定するため、当初仮説
  （「え」「ば」融合）は「ば」が先頭に来る症状を説明できない。代わって
  `discard_raw_recovery_if_focus_stale`（give-up → GJI reinit 予約 →
  focus churn で `pending_deferred` ごと破棄）が、awase 側だけで検証可能な
  第一仮説として浮上した。
- **journal だけでは決定的な証拠に届かない。** `ImeActuation`/`ImeOpenApplied`
  は reinit の実送信経路（`output/probe_io.rs` の生 `SendInput`）を記録しない
  ため、「journal にイベントが無い」ことは「発火しなかった」ことの証拠になら
  ない（round 1 版が犯した誤り）。round 2 は「新規計装ゼロで確認できること」
  と「新規計装が必要なこと」を切り分けた（下記「round 3 でやること」）。

## 背景

Windows Terminal（TsfNative、かつ config で `force_tsf` 指定）+ GJI で
「たとえば」と入力したところ「ばたと」に文字脱落・順序入替した
（`report_id: 01M1JJD54XQXSEJTHHFKV1WKA1`、app_version 1.18.0、
`WrongCharacterOutput`、2026-09-03T02:43:07Z 報告）。半角英数持続トグル機能
（BUG-25）自体は正常動作中であり無関係と確認済み。

### 既存の関連バグ（参考）

- **BUG-38**: `RawTsfLiteralRecovery` の give-up 分岐が `pending_deferred` を
  flush しないため出力順が入れ替わる、という酷似した過去バグ。修正済み。
- **BUG-89**: gate 中（`OUTPUT_GATE`/`FOCUS_RESYNC`）に defer されたキーは
  `INPUT_DEFER` へ退避され `KeyOrigin::DeferredReplay` として再生される。
- **BUG-45/BUG-75/ADR-122**: per-VK confirm の literal 判定は「代理指標
  （候補ウィンドウ SHOW / GJI I/O バイト増加）のタイムアウト」に基づく belief
  であり、実際の TSF composition 状態と乖離しても検出も訂正もできない構造的
  欠陥がある。ADR-122 は「着弾したかを事後推測する路線は、前提を反転させても
  必ずどちらかの実機ケースで破綻する」と結論している（issue #149、実装
  ペンディング中）。

## 実機 journal 解析（round 1 後に追加、round 2 で大幅訂正）

`services/report-worker` から `wrangler r2 object get` で report JSON を
再取得し、`log_excerpt`（`JournalEntry` の JSON 配列、943件）を精査した。
`config_toml` に以下が含まれることを確認した:

```toml
[[app_overrides.force_tsf]]
process = "WindowsTerminal.exe"
class = "CASCADIA_HOSTING_WINDOW_CLASS"
```

**round 1 の architect/premortem が推測した「既定は Vk モード」は round 1
時点ではもっともらしい推測だったが、本ユーザー環境固有の `force_tsf` 設定
により本インシデントには当てはまらない**（この既定/非既定の区別は round 2
の architect が指摘: 本 ADR の修正が「既定環境（force_tsf 無し）」を
カバーするとは限らない、という含意が残る）。

### 事実1: 全 probe が `path: "PerVk"`, `target: "Tsf"`（確定）

インシデント窓の3件の `LiteralDetect`（`cold_seq` 235/236/237、この
`cold_seq` フィールドは本物の `machine.cold_seq_hint()` 由来で信頼できる。
下記「round 2 での訂正」参照）はいずれも `path: "PerVk", target: "Tsf"`。
`gji_warmup_coro.rs:172-181` の `run_per_vk_confirm(..., TransmitTarget::Tsf)`
経路であることが確定した（premortem round 1 の C-1 指摘がそのまま実証された）。

### 事実2: 物理キーは probe 進行中に約130〜320ms の追加遅延を伴ってバースト処理される（確定・方法論も round 2 で検証済み）

`KeyInput` の `event.timestamp_us`（実物理押下時刻）と `elapsed_ms`
（journal 記録時刻）の差分は通常時 ~532ms で安定するが、インシデント窓内の
2つのバースト（`seq 42404-42409`、`seq 42415-42418`）だけ664〜848ms
（基準比 +132〜+316ms）に跳ね上がり、バースト後は即座に基準値へ戻る。

**round 2 での検証**: 「メインスレッドが drain ループでブロックされると
フックコールバック自体が遅れ、`timestamp_us` の採取も遅延するため差分が
遅延を過小評価するのでは」という懸念を architect が round 2 で検証した。
`hook.rs:678-756` でフックが専用スレッド（独自の `GetMessageW` ループ）で
動作することを確認し、この懸念は否定された。**この測定方法は信頼できる。**

**未解決（round 2 で新たに指摘）**: このバーストのモーラ対応付け
（`seq 42404-42409`→「と」、`42415-42418`→「え」）は `.yab` レイアウト
ファイルとの突き合わせを本文中で示しておらず、根拠が不十分（premortem
round 2 R2-8）。加えて **`seq 42419-42425` の内容、および「た」「ば」自身の
物理キーがどの seq にあるかが未確認**。特に「ば」の物理キーがバースト内か
バースト外かは、下記の因果チェーンの成否を左右する。**round 3 で
`seq 42404-42426` の全エントリを表に起こす必要がある。**

### 事実3（round 2 で大幅訂正）: 「た」の give-up と reinit の関係は未確定。「journal に reinit イベントが無い」は不発火の証拠にならない

round 1 版は「インシデント窓内に `ImeActuation`/`ImeEvent`/`ImeOpenApplied`
の reinit 相当エントリが無いこと」を根拠に「A-4（give-up → GJI reinit →
focus churn で `pending_deferred` 破棄）は本件では不発火」と結論したが、
**この推論は round 2 で両エージェントに独立に否定された（false negative）**。

- give-up 分岐（`consecutive != 0`）は `output/probe_io.rs:766-772` で
  **必ず `io.schedule_chrome_gji_reinit(...)` を呼ぶ**。journal の
  `gave_up=true`（cold_seq=236）は、この呼び出しが発生したことと矛盾しない
  どころか整合する。
- 実際の reinit 送信（`send_chrome_gji_reinit_and_poll`、
  `output/probe_io.rs:167-`）は `make_key_input_ex` による生 `SendInput`
  で、`JournalEntry::ImeActuation`/`ImeOpenApplied` を一切 emit しない
  （これらを emit するのは `runtime/ime_refresh.rs`/`runtime/mod.rs` の
  belief 層の actuation 経路のみ）。**journal はこの reinit 経路を観測する
  センサーを持たない。**
- `backs: 1` フィールドも「backspace が実際に送信された」ことを意味しない。
  trace の push は分岐判定より前（`probe_io.rs:721-737`）にあり、実際の
  cleanup 予約は `ScheduleGjiReinitResult::Scheduled` のときだけ、かつ
  `flush_raw_tsf_literal_recovery` 冒頭の `discard_raw_recovery_if_focus_stale()`
  が true なら backspace も romaji も送らず early return する
  （`output/mod.rs:1652-1697`）。
- **`discard_raw_recovery_if_focus_stale` は、give-up 検出時と drain 時で
  `focus_gen` が変化していた場合に backspace・romaji・`pending_deferred` の
  VK を全て破棄する経路である。** 発火条件（give-up 直後の focus 変化）は
  本インシデントの再現条件（激しい Alt+Tab による focus churn）そのものと
  一致する。「たとえば」に含まれる「た」が give-up した直後、実際に
  windowsterminal.exe へのフォーカスは短い dwell（156ms 遷移）を経ており、
  focus churn が継続中だった可能性が高い。

**この経路が発火していれば、「え」（またはバースト全体）が丸ごと破棄された
という、GJI 内部の解釈を仮定しない awase 側だけで完結する説明になる。**
ただし journal だけでは `discard_raw_recovery_if_focus_stale` の実際の発火を
確認できない（新規計装が必要、下記）。

### 事実4（round 2 で反証）: 「ば」の `write_delta=315` を「複数モーラの融合」の証拠として使うのは誤り

round 1 版は「え+ば が `pending_deferred` で融合し、単一の異常に大きい
`write_delta=315` を生んだ」と解釈したが、architect が round 2（R2-3）で
これを構造的に否定した:

1. `take_pending_deferred_if_probe_idle`（`tsf_warmup_coord.rs:327-337`）は
   `has_pending_tsf()` が true なら `None` を返す。probe in-flight 中に
   deferred を巻き込むことは構造上できない。
2. `pending_deferred` の flush は probe の**開始時ではなく終了時**
   （`finish_probe_stage`、ADR-103 決定4-b）。
3. `write_delta=315` は 'B'（"ば" の1文字目）を送る**直前**にベースラインを
   取った上での増分（`probe_io.rs:673-676`）であり、「ば」自身の confirm の
   値として一貫している。「融合バッチの証拠」として読むのは誤り。

**正しい可能性が高い順序**（give-up が reinit を予約した前提）: 「た」の
give-up → `raw_recovery_owns_deferred()` が true → `finish_probe_stage` は
deferred に触れない → 次の drain で `flush_raw_tsf_literal_recovery` が
backspace/reinit を試みるが `Polling` なら early return（deferred は保持
されたまま）→ 「ば」が engine を通過し独立した新しい probe（cold_seq=237）
で正しく confirm される → **「ば」の probe が完了して初めて、保持されて
いた deferred（あれば）が flush される**。この順序なら「ば」が先に確定し、
round 1 の A-5（順序入替が未説明）が説明できる可能性がある。

**ただし `write_delta=315` そのものが false-positive の可能性も残る**
（architect R2-4）: 直前の backspace/reinit 由来の GJI I/O が「ば」の
ベースライン採取と confirm 判定の間に紛れ込み、実際には合成されていない
のに `CompositionConfirmed` と誤判定した可能性を否定できない。これは
BUG-45/BUG-75/ADR-122 が指摘する「代理指標ベース判定」の構造的欠陥そのもの。
**315 の解釈は両論併記のまま未決定とする。**

## 根本原因（round 2 終了時点、未確定部分を明示）

**「え」または後続モーラ全体が失われた機序について、2つの仮説が競合して
いる。第一仮説（awase 側で検証可能）は、per-VK confirm の give-up が予約
する GJI reinit の実行を待つ間、focus churn によって
`discard_raw_recovery_if_focus_stale` が発火し、保持されていた
`pending_deferred`（および backspace/romaji）が丸ごと破棄されるというもの。
第二仮説（round 1 版、round 2 で構造的な誤りが判明したため後退）は
`pending_deferred` のモーラ混在フラッシュによる融合。いずれの場合も、
物理キーが probe 連鎖（cold_seq 442→443→444）の間 `OUTPUT_GATE`（推定、
未確定）に堰き止められバースト処理されること自体は journal で実証済み。**

**未決定な点が decision に直結するため、本 ADR は特定の修正を decision と
して確定させず、round 3 で行うべき追加調査を記録するに留める。**

### 関係ファイル・関数一覧

| 役割 | file:line |
|---|---|
| TSF cold-start の per-VK confirm 経路 | `crates/awase-windows/src/tsf/warmup/gji_warmup_coro.rs:172-181` |
| per-VK confirm 本体 | `crates/awase-windows/src/tsf/warmup/probe_fsm.rs:436-475` |
| per-VK の SuspectedLiteral/give-up 判定 | `crates/awase-windows/src/tsf/warmup/literal_detect_fsm.rs:260-415` |
| give-up 時の GJI reinit 予約（`schedule_chrome_gji_reinit`、journal に記録されない） | `crates/awase-windows/src/output/probe_io.rs:766-782` |
| reinit 実送信（`send_chrome_gji_reinit_and_poll`、journal に記録されない） | `crates/awase-windows/src/output/probe_io.rs:167-` |
| `discard_raw_recovery_if_focus_stale`（第一仮説の中心） | `crates/awase-windows/src/output/mod.rs:1652-1697` |
| `raw_recovery_owns_deferred` | `crates/awase-windows/src/output/mod.rs:1400-1417` |
| `pending_deferred` 実体（モーラ境界なしフラット Vec） | `crates/awase-windows/src/output/tsf_warmup_coord.rs:51-57,296-337` |
| `flush_pending_deferred_vks`/`flush_stale_deferred_vks_after_recovery` | `crates/awase-windows/src/output/mod.rs:1698-1766` |
| `WM_DRAIN_OUTPUT_QUEUE` ハンドラ（同期リプレイループ） | `crates/awase-windows/src/runtime/message_handlers.rs:1300-1420` |
| deferred replay の他の合流点（最低5箇所、本文リスト化要） | `deferred_engine_timers`/`drain_runtime_requests`（`message_handlers.rs:1391-1414`）、`TIMER_IME_OFF_RESCUE`（`:530-537`）、tsf-gate-timeout（`:508-511`）、`runtime/mod.rs:742-745` |
| `FOCUS_RESYNC` gate が2打鍵目で事実上解除される経路 | `crates/awase-windows/src/app/mod.rs:485-506`, `crates/awase-windows/src/input_defer.rs:64-67` |
| `OUTPUT_GATE`/`OutputActiveGuard` | `crates/awase-windows/src/tsf/probe_bridge.rs:24-149` |
| journal で reinit/backspace/discard を可視化する追加候補 | `journal.rs`（`ScheduleGjiReinitResult`・backspace 実送信・`discard_raw_recovery_if_focus_stale` 発火・`pending_deferred` flush 件数を新規 variant として追加） |

## BUG-38 との異同

未変更。BUG-38 は同一 probe の give-up サイクル内での flush 順序のみを
保証し、probe 連鎖をまたいだ複数モーラの `pending_deferred` 混入や、
`discard_raw_recovery_if_focus_stale` による全破棄は対象外。

## 検討した選択肢（round 2 終了時点の評価）

### 選択肢A: `pending_deferred` にモーラ境界を持たせる

TSF側1箇所（本件の経路）に絞るのが妥当（round 2 architect R2-9）。premortem
が指摘した `UnicodeColdWarmupFsm::deferred_chars`/`Output::unicode_cold_deferred`
は Unicode 注入モードの経路であり、本件（TSF モード）には現れない——「同型だが
未観測の穴」として `docs/known-bugs.md` に別記録し、本 ADR のスコープからは
外す。第二仮説（融合）が真であれば有効だが、事実4のとおり融合仮説自体の
確度が下がっており、**単独では的を外す可能性がある。**

### 選択肢B: drain ループを probe in-flight 化した時点で中断する

**却下（変更なし）。** `should_post_drain` の不変条件を破り BUG-02/BUG-70
系を再生産する blocker あり。

### 選択肢C: cold-start 直後だけ per-VK confirm を強制する

**却下（no-op と確定）。** 本インシデントは最初から per-VK confirm 経路で
走っていた。

### 選択肢D: drain バーストを「1つの出力エピソード」として扱う

**現状の形では不採用。** round 2 で以下の blocker が判明した:

- **R2-1（premortem）**: cold path は probe install 時点で romaji を送信
  せず、実送信は `TIMER_TSF_PROBE` tick まで遅延される（`vk_send.rs:294-324`）。
  Dの「先頭だけ probe、以降は即送信」は、先頭モーラの送信開始前に後続モーラが
  追い越す**決定論的な順序反転**を新規に作る。
- **R2-5（architect）**: 後続モーラの生 VK 送信が先頭 probe の confirm
  ベースラインを汚染し、false negative（過剰な backspace）を
  false positive（literal の見逃し）に付け替えるだけ——ADR-122 が
  「前提を反転させても破綻する」と結論した罠に正面から入る。
- **R2-6（architect/premortem）**: 本インシデントで cold_seq=237（「ば」の
  probe）は実際に `CompositionConfirmed` で**成功していた**。Dはこの
  実測で機能していた保護そのものを除去する——「既存リスクの露出」ではなく
  「新規の退行」。
- **R2-7（architect）**: Dのフラグが止めるのは同一 drain エピソード内の
  新規 probe 設置のみ。`has_pending_tsf()` はエピソードをまたいで true の
  ままなので、**エピソードを跨いだ `pending_deferred` 混入は止まらない**。
- **R2-8（architect）**: round 1 の合流点5箇所のうち D のフラグが覆えるのは
  2箇所のみ。特にバースト末尾のモーラが drain ハンドラ外のタイマー満了で
  確定するケース（合流点#5）が覆えず、これが本件で最も効いている可能性が高い。

Dを採るなら、後続モーラを即送信にするのではなく「先頭 probe の
`run_per_vk_confirm` の VK 列に統合する」形へ再設計する必要があり、
実装コストは「低〜中」ではなく「中〜高」。

### 選択肢E: 出力シンクの一本化（round 1: 将来課題→round 2: 最小版を格上げ）

round 2 で architect が、E の**最小版**（`pending_deferred` に「どの romaji
の後に出るべきだったか」の順序トークンを持たせ、flush 時にその romaji が
既に送信済みなら順序が壊れていることを検出してログに残すだけ）を提案し、
premortem も方向性に同意した。フルスコープ（ADR-079 Stage2 相当）よりコストが
低く、`send_vk_run_batch` の分割を伴わないため選択肢Aが持つ BUG-02 系 race
再燃リスクも新設しない。**次のインシデントで一次証拠を自動的に残せる**点で
`.claude/rules/fix-requires-evidence.md` の趣旨にも合う。round 3 で decision
候補として本格検討する。

## 決定

**未確定。round 2 終了時点で、両エージェントが独立に「decision を選ぶ段階に
まだ到達していない」と結論した。** 以下を round 3 の作業として記録する。

### round 3 でやること（両エージェント一致で推奨、優先順）

1. **新規計装ゼロで進められる追加解析**:
   - `TsfProbeStarted`/`TsfProbeCompleted`（`journal.rs:228,235`）の区間と
     遅延した `KeyInput` の `elapsed_ms` を突き合わせ、`OUTPUT_GATE` の
     `OutputActiveGuard` 生存期間と一致するかを確認する（gate 種別の特定、
     未決定事項1の決着に新規計装不要な可能性が高い）。
   - `ClockAnchor`（`journal.rs:249`）で基準オフセット ~532ms の妥当性
     （ドリフトの有無）を検証する。
   - `seq 42404-42426` の全エントリを vk/event_type/state_before/after
     付きで表に起こし、`.yab` レイアウトと突き合わせて「と」「え」「た」
     「ば」の対応を確定する。特に「ば」の物理キーがバースト内かどうかを
     確定する。
2. **新規計装が必要な項目**（挙動変更ゼロのログ追加）:
   - `ScheduleGjiReinitResult`（`Scheduled`/`SuppressedExistingPoll`/
     `SuppressedExistingScheduled`）を journal 化。
   - backspace の実送信（`flush_raw_tsf_literal_backspaces` の実行）を
     journal 化。
   - `discard_raw_recovery_if_focus_stale` の発火と `discarded_deferred`
     件数を journal 化。
   - `pending_deferred` flush 時の VK 数・run 数（現在
     `key_injector.rs:306-309` の `log::debug!` は app.log のみで journal
     に入らない）を journal 化。
3. **上記が出そろってから decision**: `discard_raw_recovery_if_focus_stale`
   が実際に発火していたと確認できれば、decision は D/A ではなく
   `discard_raw_recovery_if_focus_stale` 自体の設計見直し（focus 変更時に
   「捨てる」以外の選択肢——例えば engine へ差し戻す——の検討）になる可能性が
   高い。発火していなければ、選択肢D（`run_per_vk_confirm` の VK 列統合版、
   中〜高コスト）と選択肢E最小版を比較検討する。

## 未決定事項

1. gate の種別（`OUTPUT_GATE`/`FOCUS_RESYNC`/hook→mainチャネル滞留の3択、
   round 3 の解析1で新規計装なしに決着する可能性が高い）。
2. `FOCUS_RESYNC` gate が2打鍵目で事実上解除される件は、本ADRのスコープ外の
   別欠陥として切り出しを推奨。ただし premortem round 2 の指摘どおり
   「意図か欠陥か不明」であり、`has_pending_drain` の FIFO 保証という
   意図的設計である可能性も残るため、断定せず別 issue で調査する。
3. Unicode 側の同型フラットバッファ2箇所（`UnicodeColdWarmupFsm::deferred_chars`、
   `Output::unicode_cold_deferred`）は「同型だが未観測」として
   `docs/known-bugs.md` に記録し、本ADRのスコープからは外す。
4. `send_keys` の defer 非対称性（`SpecialKey`/`Key`/`KeyUp` は即送信、
   `Romaji` のみ defer、`platform.rs:1000-1020`）は、`discard_raw_recovery_if_focus_stale`
   が真因なら同じ `flush_raw_tsf_literal_recovery` 順序問題の一部である
   可能性があり、横展開ではなく本文で扱うべきか round 3 で判断する。
5. 「え」消失の機序解明は decision の前提条件である（round 1 版が書いた
   「根治できれば機序解明は必須ではない」は撤回する——D も A も
   `discard_raw_recovery_if_focus_stale` の経路を通らないため、それが
   真因なら両方とも無関係な修正になってしまう）。

## 関連

BUG-38、BUG-89（`FOCUS_RESYNC`/`OUTPUT_GATE` の deferred replay 経路）、
BUG-45/BUG-75/ADR-122（per-VK confirm の代理指標ベース判定の構造的欠陥、
「着弾したかを事後推測する路線は反転させても破綻する」という教訓——本ADRの
`write_delta=315` 解釈の未決定さも同型）、
[docs/bug-reports-triage.md](../bug-reports-triage.md)
（`01M1JJD54XQXSEJTHHFKV1WKA1` 該当行）。
