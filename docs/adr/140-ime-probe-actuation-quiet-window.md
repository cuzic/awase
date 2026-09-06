# ADR-140: IME probe/actuation の発行競合 — Step 0（診断ログ追加）のみ採用、Step 1（排他機構）は実機実測待ちで未着手

## ステータス

**採用（Step 0のみ、2026-09-06）。** Step 1（排他/フェンス機構本体）は
このADRの対象外であり、Step 0で実機から得られる実測値なしには設計・実装
に着手できない。

## Context

[BUG-113](../known-bugs.md)（Windows Terminal + GJIで余分な「@」が出る
不具合）は、二重actuationの解消（`ImeController::apply`のGJI
`AlreadyMatched`ガード修正等）については実装・実機確認済みだが、
「`kp_stage_idle_conv_check`のクロスプロセス読み取り（probe）とGJI
actuationの時間的競合」という、二重actuationとは独立したもう一つの十分
条件が未対応のまま残置されている。

Explore 2体 + Opus設計2体による深い調査（相互批判による収束）の結果、
以下が判明した:

- 競合は真の「レース」（非決定論的な発生順序）ではなく、構造的に決定論的な
  順序を持つ: `SendInput`（actuation、同期）が先に完了し、次のメッセージ
  ループでprobeのワーカースレッド（`win32_async::offload`経由）が
  `WM_IME_CONTROL`（`SendMessageTimeoutW`）を発行する、という順序が
  ハードウェア/OSスケジューリングに依存せず決まる。
- 既存の全フェンス（`conv_mutation_seq`、`explicit_action_ms`等）は
  「spawn時にキャプチャ→apply時に照合」という形で**結果を破棄するだけ**
  であり、syscall自体の発行（issue）を止める機構が存在しないため、原理的
  にこの種のバグを検出できない。
- 修正機構（排他/フェンス方式、非対称: actuationは絶対に待たない）の設計
  は複数案が検討されたが収束途上であり、**排他窓の量は実測が必須**
  （[tuning-constants](../../.claude/rules/tuning-constants.md)により、
  測定なしに値を決め打ちすることは禁止されている）。

## 確定した事実

1. **決定論的順序の機構**: `SendInput`はメインスレッド上で同期的に完了する
   一方、`SendMessageTimeoutW`によるクロスプロセスprobe/actuationは
   `win32_async::offload`でワーカースレッドに追い出され、完了は
   メッセージループへの次のディスパッチを待つ。したがって同一フレーム内で
   actuationとprobeが両方issueされる場合、actuationのSendInputは常に
   probeのワーカースレッド起床より先に完了している。
2. **3つのチェックポイント（spawn/issue/apply）のうち issue だけが無防備**:
   spawn時点は`conv_mutation_seq`等でキャプチャされ、apply時点は
   同じ値との照合で守られているが、issue（実際にOS APIを呼ぶ瞬間）を
   遅延・中断・観測する機構が無い。今回追加するのはこの issue 地点の
   タイムスタンプだけである（Step 0のスコープ）。
3. **`offload()`にタイムアウトが無い**: `win32_async::offload`で
   ワーカースレッドに追い出された`SendMessageTimeoutW`呼び出しは、
   呼び出し元がタイムアウトして`LEAKED_THREADS`（`crates/win32-async/
   src/thread_timeout.rs`）に諦めて登録した後も、OS呼び出し自体は
   ワーカースレッド上で継続し得る。つまり「awase側が待つのをやめた」
   ことと「OS呼び出しが実際に終わった」ことは別イベントであり、
   諦めた後に完了したprobeの結果が、その後のactuationと時間的に
   交錯する余地が残る。
4. **`kp_apply_conv_engine_sync`が結果を第二のactuationへ増幅する経路**:
   probeが返した値が「desiredと乖離している」と判定されると、
   drift correctionが追加のactuationを発行しうる（ADR-078の開閉軸での
   再発）。ただしこの増幅経路自体の設計変更は本ADRのスコープ外とし、
   別ADRへ切り出す。
5. **GJI actuationの発行経路は少なくとも3つ確認されており、他にも
   存在しうる**（実コード照合済み、以下引用は全てこのタスクで直接
   確認したfile:line。特に断りのない限り`crates/awase-windows/src/`相対、
   `src/config.rs`のようにルートクレート`src/`相対のものは都度明記）:
   - **(a) 同期経路**: `ime_controller.rs:546`の`ImeController::apply`が
     「同期経路の唯一の合流点」であることは同関数本体のコメント
     （`ime_controller.rs:548-551`）で明記され、実際の生産コード上の
     呼び出し元は`runtime/key_pipeline.rs:1311`と`platform.rs:1561`の
     2箇所のみ（`ime_controller.rs:790`/`:867`はテストのみ）。内部で
     `apply_mechanism`（`ime_controller.rs:375-382`、dispatch table
     `:337-344`）→ `GjiDirectStrategy::apply`（`:150-180`）→
     `crate::ime::send_ime_mode_key`（呼び出しは`:173`、実装は
     `ime.rs:273`）という経路で実際のVK送信に至る。
   - **(b) 非同期フォールバック経路**: `runtime/open_chain.rs:297`の
     `fallback_write`は`apply_mechanism`（`open_chain.rs:337`）を
     `ImeController::apply`を経由せず直接呼ぶ。これは
     `architecture_guard.rs:2253`の
     `raw_mechanism_write_sites_are_confined_to_chain_writers`が
     「`apply_mechanism(`の生産コード呼び出しは`ime_controller.rs`内
     （`SyncChainWriter::write`経由）と`open_chain.rs`内
     （`fallback_write`）の正確に2箇所のみ」と固定していることでも
     裏付けられる。同ファイルの`imm_cross_write`（`:145`）は
     `apply_mechanism`を呼ばずIMM専用の書き込み（`ime::
     set_ime_open_then_conv_for_target`等、`:186`/`:202`）を行うため、
     GJI actuation経路には含まれない。
   - **(c) eager warmup経路**: `tsf/send.rs:28`の
     `send_eager_warmup_vk_pair`は`ImeController::apply`/
     `apply_mechanism`のいずれも経由せず、`win32::send_input_safe`
     （呼び出しは`tsf/send.rs:43`）で`VK_IME_ON`を直接送信する。唯一の
     生産コード呼び出し元は`output/mod.rs:1154`。
   - `.claude/rules/fix-requires-evidence.md`の「IME actuation 合流点」
     表は上記(a)(b)に加え`runtime/executor.rs::dispatch_ime_set_open`
     （`executor.rs:793`、実体はディスパッチャで(a)(b)いずれかへ分岐する
     だけであり第4の独立な発行地点ではない）を挙げているが、同ルール
     ファイル自身の「なぜこのルールが必要か」節が「issue #136/ADR-119で
     実際の呼び出し経路は5つあり、最初は1箇所しか把握していなかった」
     という過去のインシデントを記録している。したがって本ADRは
     **「少なくとも3つの経路が確認されている」とのみ記載し、「経路は
     3つで全てである」という確定的な主張はしない**——`runtime/
     key_pipeline.rs`のshadow-toggle経路や`runtime/mod.rs:951`の
     `try_force_on_bootstrap`等、未監査の直接呼び出し箇所が残っている
     可能性がある。Step 1着手前には改めてこの経路一覧を洗い出し直す
     必要がある。

## ADR-138との関係（意図的なスコープの違い、無視ではない）

[ADR-138](138-ime-probe-actuation-witness-app-rejected.md)の決定2
（`docs/adr/138-*.md` 130-141行目）は既に「`imm.rs::send_ime_control`は
呼び出し元識別子を持たないため、ここへの一括ログでは（ADR-136の）経路A/B
を区別できない。各呼び出し元に、evidence型・confidence・`SkipTyping`
消費有無をjournalに出す計装を追加すべき」と決定している。

本タスクが`imm.rs::send_ime_control`に追加する診断ログは、字面としては
まさにADR-138が「不十分」と評した「一括ログ」そのものである。これは
ADR-138の決定を見落としたのではなく、**意図的にスコープが異なる**:

- ADR-138の決定2が解決しようとしている問いは「どのevidence型・
  confidenceの呼び出しか」という**belief/observation層の識別**であり、
  これには呼び出し元ごとの計装が必須。
- 本ADR（Step 0）が解決しようとしている問いは「probeとactuationの
  issueが時間的にどれだけ近接しうるか」という**タイミング測定**の一点
  のみであり、`ime_wnd`・`thread::current().id()`・`cmd`（probe系
  IMC_GET*か actuation系 IMC_SET*か）で十分に用が足りる、狭い問いである。

`.claude/rules/experiment-logging.md`が防ごうとしている「過去の決定を
知らずに再度同じ道を検討する」事故を避けるため、この違いをここに明記する。
ADR-138決定2（呼び出し元ごとの計装）は依然として未実装のまま有効であり、
本ADRはそれを代替しない。

## 検討した設計案と却下理由

前回の設計セッション（Opus設計2体、相互批判）で検討され、いずれも
「実測値が無い状態で機構の形・パラメータを決め打ちすることになる」ため
採用を見送った案:

- **即時排他ロック方式**: actuation issueの前後で短いロックを取り、
  probe issueをブロックする。ロック保持時間（=排他窓の量）を決め打ち
  できず、`tuning-constants.md`の実測義務に反する。また「actuationは
  絶対に待たない」という非対称要件（actuation側の遅延はUI応答性に
  直結する一方、probe側は多少遅延しても実害が小さい）を満たすには
  ロックの向きを非対称にする必要があり、素朴な相互排他プリミティブでは
  表現できない。
- **フェンス値のissue時点への前倒し**: 既存の`conv_mutation_seq`等の
  「spawn時キャプチャ→apply時照合」パターンをissue時点に前倒しする案。
  spawn自体が既にワーカースレッドへの委譲後であり、issueの瞬間を
  メインスレッド側から制御する経路が存在しないため、根本的に成立しない
  （observer/pureな`classify_*`から書き込みを直接操作できないのと同型の
  制約）。**この却下理由は誤りだったことが後日判明した。下記「追記
  （2026-09-06）」を参照。**
- **probe側を完全に非同期化しactuation優先のキューを設ける**: 実質的な
  Step 1の本体案の一つ。効果は見込めるが、キューの長さ・排他窓の量を
  実測なしに設計すると、Chrome probe定数が20→100→200→350msと5週間で
  段階的にエスカレーションした前例（`tuning-constants.md`参照）と同じ
  「盲目的エスカレーション」を機構レベルで再演するリスクが高いと判断し、
  Step 0の実測を待つことにした。

## 決定

### 決定1: Step 0（診断ログの追加）のみを本ADRで採用する

以下を追加する（実装詳細はコード参照）:

- `crates/awase-windows/src/win32.rs::send_input_safe`: 送信する`INPUT`が
  IME actuationか否かを`dwExtraInfo`のマーカーで判定する（VKの固定
  リストでは`keys.engine_on_ime_key`/`engine_off_ime_key`がユーザー
  設定可能な自由文字列でF13-F24等にもなりうるため、設定済みマシンで
  actuationが不可視になり本末転倒——`src/config.rs:550-556`参照）。
  `dwExtraInfo == tsf::output::IME_KANJI_MARKER`（決定1(a)(b)の
  `send_ime_mode_key`系）に加え、**`dwExtraInfo == tsf::output::
  TSF_MARKER`かつVKが`VK_IME_ON`/`VK_IME_OFF`の組み合わせ**
  （決定1(c)の`send_eager_warmup_vk_pair`）も判定対象に含める
  （コードレビュー指摘、MAJOR：`IME_KANJI_MARKER`単独では warmup経路が
  不可視になっていた。`TSF_MARKER`は通常の文字出力にも広く使われる
  サブシステム単位のマーカーのため、VK限定と組み合わせてノイズを
  避けている）。該当する場合に`[ime-io] actuation SendInput
  kind=<kanji_marker|tsf_marker_warmup> issue_us=...`をdebugログ出力する。
- `crates/awase-windows/src/imm.rs::send_ime_control`: 既存の
  `start_ms`/`end_ms`（`current_tick_ms()`基準、`send_health::record`が
  依存する既存のサーキットブレーカ用計測）は変更せず、別に
  `now_timestamp_us()`基準の高分解能タイムスタンプ（issue直前・
  elapsed）を追加ログとして出力する。`ime_wnd`・呼び出しスレッドID・
  `cmd`種別（probe=`IMC_GETOPENSTATUS`/`IMC_GETCONVERSIONMODE`、
  それ以外=actuation）を含める。

### 決定2: Step 1（機構本体）は未着手のまま持ち越す

Step 1の設計候補は複数存在し収束途上である。将来的な排他窓の定数
（例えば`IME_ACTUATION_QUIET_MS`のような名前になる可能性がある）の
**値は本ADRでは一切決めない。書く場合は必ず「未定（Step 0の実測前に
値を書かない——`tuning-constants.md`の盲目的エスカレーション回避の
ため）」と明記する。イラストレーション目的であっても具体的な数値を
一切書かない**——一度でも数値が書かれると、測定なしに後続セッションが
それをそのまま採用してしまう「アンカー効果」がこのリポジトリで繰り返し
起きている（Chrome probe定数のエスカレーション事例、
`tuning-constants.md`参照。数値そのものも本ADRでは引用しない）。

**Step 1候補として追記（2026-09-06、ユーザー提案）**: probe（`kp_stage_idle_conv_check_inner`）
のライフサイクル（spawn → issue時点の確認 → apply/abandon）を、
`conv_mutation_seq_at_spawn`等の場当たり的なスナップショット変数を
`.await`をまたいで持ち回す現状の実装から、`crates/timed-fsm`の
`StepCoro`（`timed_fsm::coro::StepCoro`）を使った明示的なコルーチンへ
書き換える案。根拠:

- `timed-fsm`自身のドキュメント（`crates/timed-fsm/src/coro.rs`）が
  「フェーズが直線的に進む多段ワークフロー」には`StepCoro`が、
  「どの状態でも同じイベントセットを受け付ける」機械には
  `TimedStateMachine`（enum状態＋遷移テーブル）が向くと明記している。
  probeのライフサイクルは前者（直線的な多段ワークフロー）に該当する。
- このリポジトリには直接の先例がある: `tsf/warmup/probe_fsm.rs`
  （TSF/Chrome cold-start probe）は元々明示的な`ProbePhase` enumで
  実装されていたが、`StepCoro`ベースの実装に置き換えられている
  （同ファイルの module doc「フェーズ遷移はStepCoro async本体に直線記述し、
  ProbePhase enumは不要」）。
- `StepCoro`の`step()`はテストから直接呼べる（`timed_fsm::coro`の
  doctestを参照）ため、両設計案（Opus 2体）が要求していた
  「Linuxで回帰テスト可能」という条件を、offloadやwin32-asyncの実行時
  機構なしに満たせる。

**この案の採否は未確定。Step 1着手時に、既存の`ImeIoArbiter`/フェンス
等価方式（前掲の設計案A〜D）と比較検討すること。** 本ADRのスコープ
（Step 0のみ）には影響しない。

**Step 1候補・案E として追記（2026-09-06、ユーザー提案）**:
「使い捨てのフックに処理を登録し、まとめて発火させる」という発想は、
このリポジトリに既に実例がある——`tsf/warmup/probe_fsm.rs`の
TSF/Chrome cold-start probeが使っている**`ProbeAction`（宣言的アクション
enum、`probe_fsm.rs:191`）+ `dispatch_probe_actions`（`VecDeque`で
1箇所にまとめて処理する dispatcher、`output/probe_io.rs:528`）+
`ProbeIo`トレイト（Win32副作用の注入点、`probe_io.rs:26`）**という
3点セットの型である。FSM/コルーチン本体は一切Win32 APIを呼ばず、
「次に何をすべきか」を`ProbeAction`という**データ**として返すだけで、
実際の副作用は`dispatch_probe_actions`が`ProbeIo`経由で実行する。

この型をBUG-113のprobe/actuation調停に適用する場合の骨子:

- `kp_stage_idle_conv_check`／`kp_stage_shadow_ime_toggle`等の各stageは
  `SendInput`/`send_ime_control`を直接呼ぶ代わりに、`KpIoIntent::
  Actuate{..}` / `KpIoIntent::ProbeConvMode{..}`のような意図を
  一時的なキューへ**登録**する。
- **actuation意図は登録直後に即座に発火させる**（決定は変えない——
  GJIハング時にユーザーの物理IMEキー入力自体が固まる、という
  却下済み案（対称ロック方式）と同型の新規リグレッションを避けるため。
  「即座に発火」と「まとめて登録する」は両立する: 登録は監査用の記録、
  発火のタイミングはactuationについては従来通り即時のままでよい）。
- probe側の非同期タスクが実際にissueする瞬間、**同じキュー（またはその
  一時点でのスナップショット）を参照し、自分のspawnからissueまでの間に
  actuation意図が記録されていないかを確認**する。これは設計案D
  （フェンス値のissue時点比較）と数学的には同じ判定だが、裸の整数
  カウンタではなく`ProbeAction`同様の**監査可能な構造化データ**として
  表現できる利点がある: ログで「何が・いつ・なぜ」を人間が読める形で
  追え、`ProbeIo`同様のトレイト注入でOS呼び出し無しにLinux上で調停
  ロジックだけテストできる。特定のprobe/actuationペアに固有の解決策では
  なく、将来別のペアで同種の問題が起きたときに同じ調停機構を再利用
  できる点が、フェンス値方式単体より体系的（俯瞰的）である。

**未解決の緊張関係（Step 1着手時に検証必須、2026-09-06検証済み——下記
「追記」参照）**: 上記「検討した設計案と却下理由」節の「フェンス値の
issue時点への前倒し」は、「issueの瞬間をメインスレッド側から制御する
経路が存在しないため根本的に成立しない」として却下されている。しかし
`explore-timing-model`の調査（本ADR確定事実5参照）によれば、`spawn_local`
されたfutureの**最初のpollはメインスレッド上で実行され**、`offload_unsafe`
へのワーカースレッド委譲はそのpollの内部（`.await`に到達した瞬間）で
初めて起きる。したがって「issueの瞬間（＝pollがofflloadへの委譲に到達
する直前）をメインスレッド側からチェックする経路」は実際には存在する
可能性が高く、却下理由の前提が誤っている疑いがある。**Step 1設計者は、
この却下理由をそのまま信じず、実コードで`spawn_local`/`offload`の呼び出し
順序を再確認してから案D・案Eの実現可能性を判断すること**（このADR自身が
「past rejected reasoning」を鵜呑みにするリスクを承知の上で、確認の必要性
だけを記録し、確定的な結論は出さない）。

## Step 0 データ収集プロトコル

収集するログは`tuning-constants.md`が要求する「測ったもの／数値／導出」
の3点にそのまま対応するように設計している:

- **測ったもの**: (1) GJI actuation（`SendInput`、`IME_KANJI_MARKER`/
  `TSF_MARKER`+VK判定）のissueタイムスタンプ、(2) probe/actuation双方の
  `SendMessageTimeoutW`（`WM_IME_CONTROL`）のissueタイムスタンプと
  完了までのelapsed。両者とも`now_timestamp_us()`（`Instant`/QPC基準）
  で同一時間軸に載る。
- **数値**: 実機の`RUST_LOG=debug`ログから、actuationのissue_usと、
  時間的に近接するprobeのissue_us/elapsed_usの差分（Δms）を抽出する。
- **導出**: 収集したΔmsの分布（最大値・p99等）から、Step 1で必要になる
  排他窓の量を導出する。ここが「実測に基づく値」であり、本ADRでは
  導出できないため書かない。

**ログ量の注意**: `imm.rs::send_ime_control`はChrome/GJI cold-start
再初期化ポーリング（10ms間隔）にも乗るチョークポイントであるため、
`RUST_LOG=debug`での収集は高頻度・大容量になる。journalの
`DumpTruncated`機構が既存のprobe/actuationログでも切り詰めを起こす
実績があるため、収集時間を絞る・grepで`[ime-io]`のみに絞る等の対策を
収集手順に含めること。

**Step 1着手前の必須ゲート条件**（`79134f5`の教訓を明示的なゲートとして
記載する）: `79134f5`（Chrome probe定数修正）は、Chrome cold-startの
遅延に見えた症状が実は「probeの計測起点がF2送信より早くずれていた」と
いう測定基準点のズレであり、対症療法的に定数を増やしただけで根本原因
（起点のズレ）は放置されていたという教訓を残した。本ADRのStep 0でも
同じ罠が起こり得る: **観測されたΔmsのギャップは本物か、それとも
issue時点の計測起点がずれているだけか、をStep 1着手前に必ず問うこと**。
具体的には、`send_input_safe`内の`IME_KANJI_MARKER`判定とその直後の
`now_timestamp_us()`呼び出しの間に、他の処理（ロック取得・ログ
フォーマット等）が挟まっていないか、`imm.rs`側の`issue_us`取得が
実際の`SendMessageTimeoutW`呼び出し直前かを、Step 1設計前に再確認する。

## ログの読み方に関する注意（相関時）

`imm.rs`の新ログ行は`SendMessageTimeoutW`**復帰後**に出力されるため、
ログファイル中の行の出現順序は「完了順」であって「発行（issue）順」では
ない。遅いprobeは、実際には先に発行されたactuationより**後の行として**
出力されうる。相関は必ず`issue_us`フィールドの値で行い、**行の出現順序
を根拠にしないこと**。

また、`ime.rs:510`（`IMC_GETCONVERSIONMODE`）→`ime.rs:519`
（`IMC_SETCONVERSIONMODE`、`modify_conv_mode`内のread-modify-write）は、
`:510`の呼び出し単体では`kind=probe`とラベルされるが、実際には
read-modify-write全体の前半であり、この呼び出し対自体がactuationの
一部である点に注意する。

## やらないこと（本ADRのスコープ外、明示的に除外）

- 排他機構本体の実装（Step 1、実機実測後）
- `kp_stage_idle_conv_check_inner`へのgate追加や呼び出し順序の変更
- ADR-078のEvent/Effect分離の一般化（別ADR）
- `IME_ACTUATION_QUIET_MS`という定数の値を決めること（未定と明記する
  のみ）
- 挙動の変更（ログ追加のみ）

## 追記（2026-09-06）: 却下理由の訂正と Step 0 実測データ第一弾

### 却下理由の訂正: 案D（フェンス値のissue時点前倒し）は構造的に実現可能

上記「未解決の緊張関係」節が指摘していた疑問点を、実コード確認で検証した
（読解のみ、実機不要）。

- `crates/win32-async/src/offload.rs::OffloadFuture::poll`（37-73行目）は、
  `!this.spawned`のガード（52行目）の直後、`std::thread::spawn`
  （59行目、実際のワーカースレッド起動＝委譲点）の**直前**にコードを
  挿入できる、素朴な同期地点である。この`poll()`自体は、`offload()`を
  `.await`しているfutureが再pollされたときに呼ばれる。
- `winmsg-executor-0.3.2`（`~/.cargo/registry/src/.../winmsg-executor-0.3.2/
  src/lib.rs`）の`spawn_unchecked_lifetime`（67-80行目）は`runnable.
  schedule()`（80行目）を呼ぶだけで、内部で`PostMessageA(hwnd, MSG_ID_WAKE,
  ..)`（75行目）により実際のrunnable実行を次のメッセージループターンへ
  遅延させる（`run_loop`の`GetMessageA`/`DispatchMessageA`、162-177行目）。
  つまり`spawn_local`直後の最初のpollは同一呼び出しスタック内では起きない。
- しかし、この「遅延された最初のpoll」自体は依然として**メインスレッド上
  で同期実行**される。したがって`OffloadFuture::poll`内の
  `std::thread::spawn`直前（＝ワーカースレッドへの実委譲点）は、
  メインスレッド側からフェンス値を検査し、委譲を中断・延期できる
  **実在する制御点**である。

結論: 「issueの瞬間をメインスレッド側から制御する経路が存在しない」と
いう却下理由は**不正確**だった。設計案D（フェンス値のissue時点比較）は
`offload()`（またはこれをラップする形）にチェックを挿入することで
構造的に実現可能。ただし、ワーカースレッド起動（`std::thread::spawn`）
から実際の`SendMessageTimeoutW`呼び出しまでの間には、OSのスレッド
スケジューリングに起因する小さな不確定窓が残ることに注意
（この窓自体の大きさは未測定）。

### Step 0 実測データ第一弾（dragonflyg4実機、2026-09-06、n=6）

`RUST_LOG=debug`でWindows Terminal + GJI環境の実機から`[ime-io]`ログを
2セッション分（計277イベント: actuation24件・probe253件）収集した。
`issue_us`で正しくソートし直し（ログ出現順は完了順であり発行順ではない
——本ADR「ログの読み方に関する注意」節参照）、種別が異なる隣接イベント間の
Δを計算した。

- **測ったもの**: 物理IMEキー（変換/無変換の単独打鍵、間隔を変えて複数回
  ×2セッション）操作時の、actuation（`SendInput`、`kanji_marker`/
  `tsf_marker_warmup`）issue_usと、直後に発行されたprobe（`imm.rs::
  send_ime_control`、`cross_process kind=probe`）issue_usの差分。
- **数値**: 種別の異なる隣接ペア25件中、20ms未満だったのは
  **actuation→probe方向の6件のみ**（3836us・4063us・4071us・6136us・
  6549us・8101us、レンジ3.8〜8.1ms）。**probe→actuation方向で20ms未満の
  ペアは0件**（最小48747us）。この非対称性は、本ADR確定事実1が述べる
  「actuationのSendInputが先に完了し、次のメッセージループターンで
  probeのワーカースレッドが起床する」という決定論的順序の理論を実測で
  裏付ける。
- **導出**: n=6は`tuning-constants.md`が要求する「盲目的エスカレーション
  回避」のための分布確認としてはまだ不十分（p99等を語れるサンプル数
  ではない）。**このADRでは排他窓の量を一切導出・記載しない**——Step 1
  設計時にさらにデータを収集し、実測分布に基づいて導出すること。

### やらないこと（この追記のスコープ外）

- 排他窓の具体的な定数値の決定（実測不足のため）
- 案D以外の設計候補（StepCoro案・案E）との比較確定
- Step 1本体の実装

## 関連

[docs/known-bugs.md](../known-bugs.md) BUG-113、
[ADR-138](138-ime-probe-actuation-witness-app-rejected.md)（決定2との
関係は上記参照）、
[ADR-133](133-gji-ime-mode-key-sendinput-batch-shape.md)、
[tuning-constants](../../.claude/rules/tuning-constants.md)、
[fix-requires-evidence](../../.claude/rules/fix-requires-evidence.md)、
[experiment-logging](../../.claude/rules/experiment-logging.md)。
