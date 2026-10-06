---
id: ADR-229
title: |-
  awase-windows を「Windows の dumb な実行役」に絞り、判断を Linux でテストできる側へ寄せる層の引き直し
summary: |-
  所有者の意図(2026-10-06): (1)Windows 依存側は命令(Cmd)を実行して結果を事象(Event)で返すだけの実行役にする、(2)判断を Linux でモック/スタブ/ダブルを使ってテストできる範囲を広げる、(3)フックスレッドを薄くする、(4)観測と判断を追記して最後に reduce する形にし、非同期 effect を使う。
  棚卸し(docs/tasks/layering-inventory-2026-10-06/、develop 9df983e4)で、awase-windows の本体約 48,850 行のうち移せる(P+P*)が約 25,230 行(52%)、うち新たに Linux でテストできるようになるのが約 17,500 行、分割が要る(F)が約 10,640 行(runtime/ と output/ に集中)と分かった。`platform_state.rs` の本体 1,813 行は Win32/unsafe/with_app を使わず、`use windows::` を持つのは 164 ファイル中 37。
  提案: ポートを時間の性質で S(即時)・B(ブロックしうる、タイムアウトを戻り値の値に)・A(Cmd を出し結果は世代つき Event)に分けて注入する。まず同一 crate 内で gate を外し、crate の物理分割は最後。契約テストで偽物を本物に拘束する。フックの畳み込みは前提確認のあと別段階。
  2026-10-06 に FCIS(Functional Core, Imperative Shell)の原則で改訂(「FCIS による改訂」節。Tier-1 portable / Tier-2 pure core の 2 段、`HubClock` を標準形、handler trait の閉じた例外、核と殻の分割。実装タスクは docs/tasks/fcis-layering-tasks-2026-10-06.md)。注意: ADR-224(ungate か切り出し)は「見逃しの実例が出るまで着手しない」としており、その着手条件は満たしていない。本 ADR の主動機は障害の記録ではなく構造的なテスト容易性なので、所有者の判断が要る。
status: |-
  起草(2026-10-06)。Opus レビュー round1(着手単位の見定め、Blocker なし、`docs/adr/review/229-opus-review-round1.md`)を反映。所有者指示(2026-10-06)により、「着手しやすい単位」(T2・T1・T3・T5・T4・T7)を先行して実装し、PR #492〜#495 としてマージ済み(2026-10-06、CI すべて green、Opus の PR レビューで Blocker・Must なし)。その後、FCIS(Functional Core, Imperative Shell)の原則で設計を Opus の複数ラウンドで収束させ、本 ADR に「FCIS による改訂」節を追加した(実装タスクは docs/tasks/fcis-layering-tasks-2026-10-06.md)。所有者の判断(2026-10-06): ADR-224 を改訂して段階 2 を核と殻の分割で進める、S2(physical_disposition.rs を自動チェックに足す)を実施する、1 turn の入口集約とフックの薄型化を将来の目標にする。P0・P1・RW・S2 を並行して実装中。
related_adr:
  - "ADR-019"
  - "ADR-030"
  - "ADR-053"
  - "ADR-129"
  - "ADR-156"
  - "ADR-163"
  - "ADR-164"
  - "ADR-165"
  - "ADR-180"
  - "ADR-224"
---

# ADR-229: awase-windows を dumb な実行役に絞り、判断を Linux でテストできる側へ寄せる

## 背景

### 所有者の意図(2026-10-06)

1. Windows に依存する側は、命令(Cmd)を受けて OS を叩き、結果を事象(Event)として返すだけの「dumb な実行役」にしたい。
2. そうして、判断のロジックを Linux 上でモック・スタブ・ダブルを使ってテストできる範囲を広げたい。
3. フックスレッドは薄くする。フックの入力を受け取って処理するコルーチンを置き、疎結合にする。
4. 観測結果と判断結果を追記し、最後に reduce する形にしたい。非同期の effect が使えそうだと感じている。

### 現状(棚卸しの事実)

[docs/tasks/layering-inventory-2026-10-06/README.md](../tasks/layering-inventory-2026-10-06/README.md)(develop `9df983e4`、読み取りのみ、ビルドとテストは未実施)。

- `crates/awase-windows/src` の本体は約 48,850 行。移せる(P+P\*)が約 25,230 行(52%)、分割が要る(F)が約 10,640 行(22%)、Windows 側に残る(O+E+G)が約 11,980 行(25%)。
- 移せる 25,230 行のうち、すでに ungated で Linux テストが回っているのは約 7,700 行。**新たに Linux でテストできるようになるのは約 17,500 行**。
- `lib.rs` が `hook`・`ime`・`ime_controller`・`imm`・`input_defer`・`journal`・`observer`・`output`・`runtime`・`platform` などを**モジュールごと** `#[cfg(windows)]` にしている。その代償で `#[cfg_attr(not(windows), allow(dead_code))]` が 46 か所(11 ファイル)あり、gate 内のテストは粗い数で約 354 本(Windows 専用のものを含む。棚卸しの「約 180 本」は数えられた分だけ)、Linux に存在しない。
- gate の粒度が粗い。`state/platform_state.rs` は本体 1,813 行に Win32 API の直接呼び出しも `unsafe` も `with_app` も無い(ただし `win32::foreground_scope()` を 11 回、`HubClock::wall(hook::current_tick_ms)` を 1 回呼び、gated な型 `ForegroundScope`/`ImeUpdate`/`HwndImeSnapshot`/`journal` を使う。ungate には注入が要る)。`tsf/probe.rs` は本体 818 行に `windows::` も `unsafe` も無いが、`hook::current_tick_ms`・`TSF_OBS`・`output` に依存するので、時計と観測の注入を済ませるまで外せない。「`windows::` の字面が無い」ことと「gate を外せる」ことは別。
- Windows API の入口は狭い。`use windows::` を持つのは 164 ファイル中 37、インポート名は計 255(型・定数を含む)。
- F の所在: `runtime/` に約 5,800 行、`output/` に約 1,430 行、`platform.rs::gji_on_focus_change` に 683 行。`tsf/` は約 434 行と少なく、warmup のコルーチン群が最も素直に移せる。
- すでにある部品: `decide_gate`/`decide_chain`/`decide_attempt`・`MechanismCommand`・`MechanismWriter`/`AsyncMechanismWriter`・`run_chain_async`(ADR-163/180)、`ImeEvent`→`ImeModel::reduce`、`DecisionInputs`(所有・Copy)、`HwndId(usize)`、`state::TickMs`、`timed-fsm` の `Clock`/`ManualClock`、`StepCoro`。

### 実害の記録は弱い(正直な評価)

- フック配線のバグ(BUG-131/132/181)、ゾンビフックスレッド対策(PR #349、`hook_callback` に再判定が 5 か所)、閉ループハーネスの写しのずれ(`7dcb52b2`)は、いずれも「判断が Windows 実機でしか再生できない」ことと関係する。ただし、**ハーネスの写しが原因で閉ループが通ったまま実機で壊れた実例は無い**(ADR-224、2026-10-04 時点)。
- したがって本 ADR の主動機は障害の記録ではなく、構造的なテスト容易性と、gate の粒度の粗さが生んだ負債(46 か所の `allow(dead_code)`、約 180 本の Linux 不在テスト、手書きの写し)である。これは ADR-224 の着手条件を満たさない。**進めるかどうかは所有者の判断**(下記「未決定」)。

## 既存 ADR との関係

| ADR | 関係 |
|---|---|
| ADR-019 / `docs/layer-boundaries.md` | コア `awase` は OS 非依存、という既存の線。本 ADR はこれを awase-windows の内側へ延長する |
| ADR-163 / 180 | 「決定と実 I/O の分離」は ime_controller 経路で済んでいる。本 ADR はその一般化(全 effect)。F の `open_chain` 3 関数が各自で再検出する gate は ADR-180 の領域 |
| ADR-164 | グローバル static は引数引き回しが第一選択。ポートのコンストラクタ注入はその延長 |
| ADR-224 | `ImeStateHub` を ungate するか、写しを純粋関数へ切り出すか。「実例が出るまで着手しない」。**段階 0 の `ime_event_log`・`journal` の ungate は、ADR-224 が名指しした案A の一部に当たる**(ADR-224 には追記済み)。本 ADR は、`foreground_scope` を「隠す stub」でなく「状態を注入できるポート」にする案で案A の費用の一部に答える。ガードの費用は、その場での ungate では 0(上記 D4)、段階 2 では `architecture_guard.rs:1382` が確実に落ちる |
| ADR-129 / 156 | フックのスナップショット埋め込みと、defer/replay の 2 窓口。移さない・壊さない対象 |
| ADR-053 | StepCoro。`SacrificialWarmupCoro` の撤去(`d4956490`)は、機構自体が不要と分かったためで、抽象の欠陥が理由ではない |
| ADR-218〜220 | DSL 化・宣言テーブル化の見送り。本 ADR は DSL を作らない。既存の純関数・trait・型を移して注入するだけ |

## 決定(提案。所有者の承認前)

### D1: 層の定義

```
awase (既存、OS 非依存)     エンジン、config、yab
awase-windows の内部(まず同一 crate 内で gate を外す。crate 分割は D4)
  core 側(Linux でテストする)   State、Event、Cmd、reduce、decide、コルーチン、時計の注入
  Windows 側(dumb)             hook(捕獲と通す/消す)、observer(O)、executor(E)、メッセージループ(G)
```

Windows 側の関数は、belief・profile・gate を読んで分岐しない。F は「O が facts を返し、core が判断し、E が Cmd を実行する」に分ける。

### D2: Win32 ポートは、1 ターンを止めるかどうかで分ける(**FCIS 改訂で置き換え。下記「FCIS による改訂」F-D1・F-D3 が正**)

| 種類 | 性質 | 形 |
|---|---|---|
| S | 即時に返る(`GetAsyncKeyState`・`SendInput`・時刻) | 通常のメソッド。1 ターンの中で呼ぶ |
| B | ブロックしうる(IMM32/MSAA/UIA のプロセス間呼び出し。今は `run_with_timeout`) | 通常のメソッド。戻り値に `Result<T, WaitError{TimedOut, PoolFull, WorkerFailed}>`(タイムアウトを値にする) |
| A | 完了が後で来る(タイマー、IME 反映待ち probe、`PostMessage` 経由) | Cmd として発行し、結果は世代トークンつきの Event として戻る。`await` で借用を保持しない(ADR-180) |

**注意(Opus round1)**: IMM32 の `imm::send_ime_control`(`SendMessageTimeoutW`)は同期経路でエンジンスレッドをそのままブロックしうる(最初の約 5 秒は防げず、再発だけを止めるサーキットブレーカがある。`send_health.rs`、BUG-34)。B を「タイムアウトを値で返す」形にすると、これらの経路では挙動が変わるので、型の変更だけでは済まない。S の「`SendInput` は即時に返る」は、LL フックの連鎖の影響を含めて未確認。

注入はコンストラクタで、小さなポート(Clock/Input/ImeIo/Probe/Timers 程度)の `dyn` を渡す。ジェネリクスは増やさない。偽物はタイムアウトを任意の回に返せる。永続化・レジストリ・FS は縁の部分として、コアに入れず、読んだ値をスナップショットで渡す。

### D3: 守る不変条件

1. 1 事象は、await で止めずに 1 ターンで最後まで処理する(取り込み → reduce → decide → 実行の開始)。BUG-34 型の隙間(観測と実行の間に窓)を作らない。
2. 実行結果は「どの Cmd の結果か」と世代を持つ Event として戻し、古い結果は reduce 側で捨てる(いまの `focus_gen`/`verify_still_current`/fence の一般形)。INV-45(await をまたいで推測値を固定しない)を保つ。
3. `OutputActiveGuard` は、Cmd(`GateAcquire`/`GateRelease`)にしても、defer と drain の 2 窓口を片側だけにしない(ADR-123/128/156)。
4. 借用は 1 ターンの入口で 1 回(ADR-164 の延長)。ただしこれは**今回は決めない**(D6)。

### D4: 移行は「同一 crate 内で gate を外す」を先、crate の物理分割は最後

`architecture_guard.rs`(5,993 行、`#[test]` 117 本)はファイルパスの文字列リテラルを異なりで 76 種(出現 193 以上)持つ。先に crate を切る、またはファイルを移すと、ガードが空振りする。**同じ場所で gate を外すだけなら、テキスト走査のガードは壊れない**(Opus round1 で確認。ガード内の `cfg(windows)` はコメント 4 か所だけ。壊れるのはファイルや関数の移動と、ガードが固定する文字列そのものの変更)。gate を外し終わってから、パスを機械的に付け替えて切る。

### D5: 偽物は契約テストで本物に拘束する

同じテスト群を、Linux では偽物に、`windows-build` CI では本物に流し、結果が一致することを確認する。偽物には QUIRKS(クセの目録)をパラメータとして持たせる。Linux のテストは「ロジックと順序の回帰」、実機は「環境のクセの実測」と役割を分ける。

### D6: フックの薄型化と畳み込みは別段階(前提確認が先)

次の事実があるため、構想どおりにはまだ進められない(棚卸し a1)。

- `HOOK_STATE` は 23 フィールドあり、**リングに載らない経路(飲み込み、`disable_apps` バイパス、overflow 素通し)も `physical_key_state` を更新する**。
- `RawKeyEvent` には拡張ビットと、Alt なりすまし前の vk が無い。`journal` の `KeyInput` は処理後の要約で、生入力の再生には足りない。
- `held_modifiers.rs` などメイン側に、「いま」の物理状態を読む箇所がある。フックだけが書く押下ビットの鏡を残す案を含め、未検証。
- フックの戻り値(通す/消す)を決める同期判定はフックに残る。

### D7: やらないこと

- キー入口の Tower/ASGI 風ミドルウェア列(原因は層ではなく識別子と決定表。Opus 第1版)。
- corophage / effing-mad の導入、`StepCoro` への `call` 追加(往復はすでに 1 行。BUG-27 追補2 の `vk_sent` 中断分岐を隠す)。
- `open_chain` を最初に Cmd 化すること(観測と実行を分けると BUG-34 型の窓ができる。最後にやる)。
- 命令列だけのための新しい DSL。

## FCIS による改訂(2026-10-06、Opus の FCIS 設計レビュー round1・round2 を反映)

Functional Core, Imperative Shell(FCIS)の原則で、D1〜D4・D6 を次のとおり改める。D2(ポートを core に注入する)は置き換える。実装タスクは [docs/tasks/fcis-layering-tasks-2026-10-06.md](../tasks/fcis-layering-tasks-2026-10-06.md)。レビュー記録は `docs/adr/review/229-opus-fcis-round1.md`・`229-opus-fcis-round2.md`。

### 用語: Tier-1「portable」と Tier-2「pure core」(2 段)

- **Tier-1「portable」= ungated**: `#[cfg(windows)]` が無く、Linux でコンパイルとテストができる。**コンパイラが守る**(ungated なモジュールが `crate::hook`・`crate::win32`・`crate::imm`・`runtime`・`with_app` を参照すると Linux のビルドが落ちる。PR #492〜#495 で実証)。段階 0〜2(ungate)は Tier-1 の作業。
- **Tier-2「pure core」**: Tier-1 に加えて、①壁時計の直接読み取り(`Instant::now()`・`SystemTime::now()`)、②`static`(不変も含む。可変か不変かをテキスト走査で判定するのは難しいので一律)と `thread_local!`(不変の表は `const` にするか、そのファイルを `CORE_MODULES` に載せない)、③ファイル内の `#[cfg(windows)]` 項目(`mod` 宣言と `#[cfg(any(windows, test))]` は可)、④FS・環境変数・レジストリ、を持たない。**テキスト走査(`CORE_MODULES`)で守る**。F の分割(段階 3 以降)は Tier-2 の作業。
- 「ungated(Linux でテストできる)」と「純粋」は別の性質。既に ungated な `state/` にも `Instant::now()`(`ime_model.rs`)や可変の static(`probe_admission.rs`)がある。「core」という語は、どちらの Tier かを必ず添えて使う。
- turn は**エンジンスレッドだけ**に当てはまる。フックスレッドの同期判定と ADR-129 のスナップショットの埋め込みは turn の外。

### F-D1: core は port を呼ばない。ただし handler trait の例外を、閉じた列挙で許す

- 原則: core は Win32 を呼ばない。必要な値は、shell が先に読んで Facts として渡すか、core が Cmd を返し shell が実行して結果を Event で戻す。
- **例外**: core のアルゴリズムが、**同じアルゴリズムの中で前の効果の結果が次の効果の選択を変える**場合に限り、効果の実行役(handler)の trait を引数に取ってよい(単に「読んでから書く」はサンドイッチで書く)。条件: ①handler の trait のメソッドはその効果だけを行い、他の環境を読まない、②trait は core に、本番の実装は shell に置き、**偽物の実装がテストに必ずある**、③**例外は下の列挙にある関数に限り、追加には本 ADR の改訂が要る**(trait 呼び出しは普通のメソッド呼び出しに見えるので、F-D6 のテキスト走査では検出できないため)。
- **例外の列挙(閉じたリスト)**:
  1. `Actuation<Verified>::run_chain(_async)<W: MechanismWriter / AsyncMechanismWriter>`(`state/actuation_chain.rs`)。型状態(ADR-090: warrant → verify を経ないと write できない)と ADR-163 の再生ハーネス(ReplayWriter)を支えているので、Cmd の状態機械には**書き換えない**。`romaji_pre_write`(条件付きの書き込み)は、この chain の前処理として扱う。
  2. 条件付きの同期の効果(読むかどうか自体が判断に依存する場合): ImmCross が `Failed` のときの再読み取り(`post_failed_reobservation`)、focus の MSAA までの同期の段階的な分類。読み取りの handler(例: `trait ImeProbe`)を引数に取る形にするのは、**実装する PR で、その関数を本リストに追加する**ときに限る。UIA は非同期なので A 種の Cmd/Event とする。
- 例外に入らないもの: warmup の `StepCoro`(Cmd を yield する標準形)。

### F-D2: F(判断混在の手続き)の標準形 = サンドイッチ

```
fn procedure(..) {                         // shell
    let facts = observe(..);               //   shell-in: 所有型の Facts
    let plan  = decide(&state, &facts);    //   core(F-D1 の例外を除き純粋)
    execute(plan);                         //   shell-out: Cmd を実行し、結果を Event で返す
}
```

正しい実例: `decide_gate`/`decide_chain`/`decide_attempt`、`plan_core`、`explicit_press_delivery`、`DecisionInputs`(所有・Copy)、`ImeModel::reduce(&mut self, ..)`。実例ではないもの: `ObservedState`(gated な `ActiveImeKind` を持ち、構築時にグローバルを読む)・`FocusFacts<'a>`(借用)。これらは「借用ビューの所有化」の対象。

### F-D3: 環境依存の除き方

| 環境依存 | core での形 |
|---|---|
| 時刻 | 状態を持つ core: 注入された `HubClock`(`Wall { tick: fn() -> u64 }` / `Manual`、`state/hub_clock.rs`)を標準形とする。`Instant` と tick(ms)の 2 つの時間軸を 1 つの値で供給する。**時計の抽象を 4 つ目に増やさない**(既存は `HubClock`・`timed_fsm::Clock`/`ManualClock`・`quanta::Clock`)。状態を持たない関数: `now` を引数で受ける。壁時計を直接読むのは shell だけ。`hub_clock.rs` 自身は時計の実装なので Tier-2 の外 |
| グローバル/`thread_local` | shell が読み、スナップショットを引数で渡す。副チャネルは戻り値に載せる |
| 借用ビュー | 所有型の Facts(`DecisionInputs` に縮める) |
| OS を読む関数(`foreground_scope()`) | **核と殻の分割**: `_in_scope(scope)` 版を core(`platform_state.rs`)に、`foreground_scope()` を読んで `_in_scope` を呼ぶ 1 行の殻を、`platform_state.rs` の**子モジュール**(`#[cfg(windows)] mod shell;`、`state/platform_state/shell.rs`)の `impl ImeStateHub` に置く(sibling ファイルだと private な `_in_scope` の可視性を広げる必要がある)。殻のメソッド名は今と同じなので runtime/ の呼び出し元は変更不要。関数ポインタ注入(`fn()` は状態を捕まえられず ADR-224 の懸念が残る)は採らない |
| HWND / HIMC | HWND は `HwndId`/`WindowId`(`usize` の newtype)。HIMC は shell の中に閉じる(core に出さない) |
| `with_app` | shell が turn の入口で借り、値にして core に渡す。**入口の集約(1 回に限る)は今回は決めない**(`spawn_local` の再入、B-1 の fail-open、`dispatch_engine_message` の非対称と衝突しうる。後段で、何を守るかを先に書く) |
| ログ(`tracing`) | 許す。core の判断に読み戻さない。replay や不具合報告に要るものは、ログではなく戻り値か journal のレコードにする。ADR-139 の `emit_tracing` の検査と `decision3_instrument_targets_…` に注意 |
| 状態の更新 | `&mut self` を許す(決定性は保たれる)。禁止するのは `&mut` 越しの隠れた環境(グローバル・時計・`with_app`) |

### F-D4: コルーチン(warmup など)は core

`StepCoro` の本体は `Cmd`(`ProbeAction`)を yield し `Event`(`ProbeTickInput`)を受け取る。OS を呼ばず、時計は tick 入力に載せる。`OutputActiveGuard`(RAII)は `wants_output_gate: bool` を値で返し、実体は shell が持つ。**defer と drain の 2 窓口(ADR-156)を片側だけにしない**。スナップショット引数化で判定が最大 10ms 古くなる(epoch fence は 20ms)ので、baseline を `SendInput` の前に取る順序(BUG-027/029/030/033、ADR-079)をテストで固定してから着手する。

### F-D5: turn の不変条件(エンジンスレッドのみ)

1. **同期の処理**(同期の handler、`ImeProbe` を含む)は、1 事象を `await` で止めずに 1 turn で最後まで処理する。BUG-34 型の「観測と実行の間の窓」を作らない。
2. 実行結果は、**既存の世代・id を再利用**して由来を示す Event として戻し、古い結果は core が捨てる。新しい id を発明しない: フォーカス失効=`focus_gen`/`ime_mode_focus_gen`/`ActuationTarget::verify_still_current`、IME 反映要求=`ApplyGeneration`(ADR-106)、物理キー押下=`PressId`(ADR-208 L1)、warmup=`cold_seq: Generation`。
3. `await` をまたぐ処理は、F-D1 の handler 例外か、core を状態機械にして shell が Event ごとに 1 ステップ進める。`open_chain` の 3 関数は**書き換えない**(INV-45・BUG-34・ADR-119/180)。
4. **非同期の handler(`AsyncMechanismWriter`、`open_chain.rs` の `write` は ImmCross で `.await`)は、各 `.await` の後に、環境(view・gate・フォーカスの世代)を観測し直してから次の効果を選ぶ。`await` をまたいで借用も推測値も保持しない**(INV-45、`fallback_write` が機構ごとに view を作り直す、ADR-180 決定1 で 3 関数が独立に gate を再検出する)。

### F-D6: 純粋さを守る仕組み

- Tier-1: コンパイラ(ungate)。
- Tier-2: `architecture_guard.rs` に、定数 `CORE_MODULES` とテスト 1 本。**許可リスト方式ではなく、「違反 0 のファイルだけを `CORE_MODULES` に載せる」方式**。`CORE_MODULES` の各ファイルの本番コード(`#[cfg(test)]` の item とコメントを除く。`layer_boundary_guard.rs` の `test_block_mask` を使う)が、Tier-2 の 4 つの規則(壁時計・`static`/`thread_local!`・ファイル内の `#[cfg(windows)]` 項目・FS/環境変数/レジストリ)に違反しないことを確かめる。`mod` 宣言の `#[cfg(windows)]` と `#[cfg(any(windows, test))]` は規則の対象外。初期は ungated な `state/` の 53 ファイル中、違反 0 の **45 ファイル**(origin/develop d0d42be6 時点。#492〜#495 のマージ後に再実測してから確定する。違反のある 8 ファイル: `hub_clock`・`ime_event`・`ime_model`・`ime_profile_driver`・`key_effect_predictor`・`key_effect_runtime`・`probe_admission`・`mod.rs`。`mod.rs` はモジュール宣言と `use` の再公開の集約ファイルなので、**恒久的に `CORE_MODULES` に載せない**)。違反を直したファイルを順に足す。既存の `DECISION3_FILES` と同じ流儀で、新しい汎用機構は作らない(ADR-218〜220 が見送ったのは GuardRule 宣言テーブル + 汎用チェッカー)。
- crate の物理分割は最後(ファイル移動でガードが空振りするため)。
- 指標: `CORE_MODULES` の件数(増やす)、違反のある ungated ファイルの件数(8 → 減らす)、「Linux で実行されないから」を理由にしたテキストガードの数(本物のテストに置き換えて減らす)、閉ループの写しの数(7。`platform_state` の ungate 後に P5 で 1 系統ずつ減らす。ungate 自体では減らない)、`#[cfg_attr(not(windows), allow(dead_code))]` の数(`grep -rn 'cfg_attr(not(windows)' crates/awase-windows/src` で 46 か所/11 ファイル。うち `allow(dead_code)` は 45、その他の `cfg_attr(not(windows), …)` が 1)。

### 移行のレシピ(全レシピ共通の後処理つき)

R1 その場で gate を外す(T2・T3)、R2 時刻を引数にして gate を外す(T7)、R3 テストだけ移す(T1)、R4 型を**使い手の側**へ移す(T4: journal → win32 だった依存を win32 → journal に)、R5 核と殻の分割(P2)、R6 サンドイッチ分割(F)、R7 コルーチンの入力をスナップショットに。**共通の後処理**: `fix-requires-evidence.md` の表・`.githooks/pre-push` の正規表現・`.cargo/mutants-awase-windows.toml` の `examine_globs`・`decision3_…` の instrument 一覧・「Linux で実行されないから」のコメントと件数を見直す。**着手前に、テストが呼ぶ関数・型・定数・macro が gated 側にないかを必ず確認する**(PR #493 の教訓)。Linux で未使用の `pub(crate)` 項目には、テストも使うなら `#[cfg(any(windows, test))]`、使わないなら `#[cfg(windows)]`(`allow(dead_code)` は増やさない。外から到達できる `pub` 項目には何も付けない)。

### 所有者の判断(FCIS 改訂で追加・更新。2026-10-06 に回答済み)

1. **ADR-224 の決定の改訂 = 承認**: 段階 2 は ADR-224 の案A に当たる。懸念 (a)(ガードが広く壊れる)は根拠が無かった(ADR-224 に追記済み)、(b)(`foreground_scope` を stub にすると挙動が隠れる)は核と殻の分割(ハーネスが `_in_scope` に任意のスコープを渡せる)で解消する。**「段階 2 は核と殻の分割の形で進める」に改訂した**(ADR-224 に追記済み)。
2. **S2 = 承認**: `state/physical_disposition.rs` を `.githooks/pre-push`・`fix-requires-evidence.md` の表・`.cargo/mutants-awase-windows.toml` に足す(別 PR で実施中)。
3. **1 turn の入口集約(`with_app` の入口を減らす)= 将来の目標にする**。着手は F の分割(P0〜P5・F1〜F6)が一段落し、前提(`spawn_local` の再入、B-1 の fail-open、`dispatch_engine_message` の非対称で何を守るか)を先に書いてから。
4. **フックの薄型化・追記して reduce する形 = 将来の目標にする**。着手は前提確認(リングに載らない経路が `physical_key_state` を更新する、`RawKeyEvent` に拡張ビットと Alt なりすまし前の vk が無い、`KeyInput` が生入力を持たない、`held_modifiers` の鮮度要件)のあと。

## 段階と成功指標

基準値(2026-10-06)は、`#[cfg(windows)]`/`#![cfg(windows)]` の属性が src に 172 か所(`windows` を含む `cfg(…)` の行は 184)、`not(windows)` の `allow(dead_code)` が 46 か所(11 ファイル)、Linux 不在テスト約 354 本(粗い数)、閉ループハーネスの手書きの写し 7 系統(`platform_state` の ungate で消えるのは 5 系統で、`runtime/` 側の 2 系統は残る見込み。現実的には 7 → 2)、`hook_callback` の再判定 5 か所。

| 段階 | 内容 | 指標 | 取りやめ条件 |
|---|---|---|---|
| 0 | 機械作業: gated な小さな型を ungated へ(`ColdReason`・`DetectionResult`・`ImeModeState`・`ForegroundScope`・`ImeUpdate`/`ImeObs`・`SentKeyEvent`・`InjectionHint`・マーカー定数)。依存の無い gated ファイル(単独で外せるのは `tsf_gate` と `ime_event_log` だけ。`hwnd_cache` は時刻の引数化が先、`tracker`・`gji_observer` は推移的に gated へ依存するので後)の gate 解除。型の切り出しは、使い手を ungate する PR で必要になった時点で行う(型だけ移しても Linux のテストは増えない) | Linux で回るテスト数、`allow(dead_code)` の減少数 | `architecture_guard`/`layer_boundary_guard` の書き換えが想定を大きく超える |
| 1 | 時計の注入(`now_ms` 引数。`TickMs` が既にある) | `hook::current_tick_ms`/`Instant::now` の直読み箇所数 | 判定の鮮度が epoch fence(20ms)を超える(`tsf/warmup`) |
| 2 | `platform_state.rs` の ungate(前提は 6 つ: 棚卸しの 5 つに `HwndImeSnapshot` を加える。`architecture_guard.rs:1382` の `HubClock::wall(crate::hook::current_tick_ms)` の件数固定は必ず落ちる) | ハーネスの写し 7 → 2、Linux 不在テスト数 | ガード書き換えが過大(ADR-224 案C へ戻る) |
| 3 | `ImeControlView` の所有化 → `ImeController::apply`・`ActuationTarget` の純粋部分 | `&ImeControlView<'_>` の項目数(15)、ime_controller のテスト 242 行が Linux で回る | — |
| 4 | `tsf/` の `LiteralDetector`/`TsfReadinessProbe` をスナップショット引数に → warmup コルーチン群 | warmup のテスト(約 1,328 行)が Linux で回る | baseline を `SendInput` の前に取る順序が崩れる(BUG-027/029/030/033、ADR-079) |
| 5 | F の分割(易しい順。`open_chain`・`on_focus_process_changed`・`monitor_loop` は最後) | F の行数(10,640) | journal replay で回帰網を先に張れない箇所は着手しない |
| 6 | crate の物理分割 | `awase-runtime` が windows-rs に依存しないことをコンパイラが保証 | — |
| H | フックの薄型化(D6 の前提確認のあと) | `hook_callback` の行数・共有状態の書き手数 | 畳み込みが `held_modifiers` の鮮度要件を満たせない |

各段階は挙動不変を原則とし、CI だけで成否を判定する(ローカルでビルドしない)。

## 先行して進める着手単位(Opus round1、所有者指示 2026-10-06)

挙動を変えず、小さく、CI だけで成否が決まるものだけを先に進める。順は T2 → T1 → T3+T5 → T4 → T7。各単位で結果を見て、次に進むかを決める。

| ID | 内容 | 新たに Linux で回るテスト | 成否を決める CI |
|---|---|---:|---|
| T2 | `tsf/tsf_gate.rs` の gate 解除(`tsf/mod.rs` の 2 か所)。ADR-229「先に確かめること 1」(ungate 後にコンパイルが通るか)の最小の実地確認 | 19 | Linux の `cargo nextest run --workspace --lib` で `tsf::tsf_gate::tests::*` が 19 本 pass。`windows-cross-check`・`windows-build` が green |
| T1 | `runtime/transport.rs::plan_tests`(30 本)を `state/physical_disposition.rs` へ移し `plan_core` で検査(移す先のモジュール名は `mod tests`。本番の挙動は不変。ただしテストが呼ぶ純粋関数 `suppress_reason` を `physical_disposition.rs` へ移した。round1 レビューはこの移動が要ることを見落としていた。**着手前に、テストが呼ぶ関数・型・定数が gated 側にないかを必ず確認する**)。**価値は最大**(配送判断の決定表、BUG-116/131/ADR-166) | 30 | Linux で `state::physical_disposition::tests::*` が 30 本 pass |
| T3 | `state/ime_event_log.rs` の gate 解除 | 5 | 同上 |
| T5 | 空の `tsf/send.rs` の削除 | 0 | `windows-cross-check` |
| T4 | `journal.rs` の gate 解除(`SentKeyEvent` を ungated へ、dump の 2 関数を `#[cfg(windows)]`) | 22 | Linux で 22 本 pass |
| T7 | `focus/hwnd_cache.rs` の `save`/`restore` に `now_ms` を渡してから gate 解除。期限切れのテストを追加(focus ファミリー、テスト追加が条件) | 新規 3〜5 | Linux で pass |

結果(2026-10-06): T2・T3・T5 は PR #492、T1 は PR #493、T7 は PR #494、T4 は PR #495 で、いずれも CI が通り、2026-10-06 にマージした(`42bc25ea`)。#493 は初回に `suppress_reason` の置き場所(Windows 専用の `transport.rs` にあり、移したテストが Linux で見えなかった)と rustfmt で失敗し、修正した。Opus の PR レビュー round1 で #493 に Must 2 件(Linux の dead_code 警告、殻 `plan` を呼ぶテストが 0 本になったこと)が出て反映し、round2 の再確認で #492・#493・#494・#495 に Blocker・Must なし。#494 の `focus/mod.rs` の `allow(dead_code)` は不要と確認して外し、#495 の `allow` も `#[cfg(windows)]` に置き換えた(新しい `allow(dead_code)` は 0 か所)。`state/physical_disposition.rs` を自動チェック 3 か所に足す件(Opus S2)は、所有者の承認を得て実施中。

取りやめ条件: Linux でコンパイルが通らない、または `allow(dead_code)` を足さないと警告が消えない(3 か所を超えたら止める。`allow` の削減という指標に逆行する)。1 本でも Linux で落ちるテストは、本番との差の発見なので BUG として扱う。

今は着手しないもの: 時計の注入の一括(`hook::current_tick_ms()` の直接呼び出しは 151 か所・34 ファイル、`Instant::now()` は 158 か所)、`platform_state.rs` の ungate、型のまとめての切り出し、`tracker.rs`・`gji_observer.rs` の gate 解除、UIA 経路の削除、`MechanismCommand` の部分型化。理由は round1 レビューの §2 を参照。

## 先に確かめること(着手前、読み取りだけ)

1. ungate 後にコンパイルが通るか(棚卸しは全員ビルド未実施)。段階 0 の 1 ファイルで実際に試す。
2. ファイルや関数を移す段階(2 以降)で壊れるガードを、移動対象ごとに数える。gate を外すだけの段階 0 は、Opus round1 で 0 件と確認済み(ADR-224 が懸念した「広く壊れる」は、その場での ungate には当てはまらない)。
3. `foreground_scope` を「注入できるポート」にしたとき、ADR-224 が懸念した「Windows 固有の挙動が見えない」が、偽物でスコープ失効を再現できれば解消するか。
4. D6 の各前提(リングに載らない経路、`held_modifiers` の鮮度、`ctrl_consumed_since_down` のライブ読み)。
5. 棚卸しで見つかった不具合候補 9 件(`docs/tasks/layering-inventory-2026-10-06/README.md` §6)の実在確認。本 ADR とは独立に扱う。

## 危険箇所(分けると隙間ができる所)

- `fallback_write` が RUNTIME 借用を握る BUG-34 E-prep と、`open_chain` 3 関数が独立に再検出する gate(ADR-180、INV-45、ADR-089 C-4)。
- `OutputActiveGuard` の寿命と defer/drain 2 窓口(ADR-123/128/156)。
- `reset_candidate_was_seen` の消費タイミング(BUG-113)、検証済み HWND を open と conv で使い回す点(INV-14)。
- tick 入力のスナップショット化で判定が最大 10ms 古くなる点(epoch fence の猶予は 20ms)。
- `dispatch_engine_message` の再入時の扱いの非対称(捨てる 11、再 post 5)、`spawn_local` 内の `with_app` の戻り値の扱いの不揃い。

## 撤去の候補(移す前に削除を検討する)

`focus/uia.rs` の UIA 経路(約 400 行と COM ワーカー 1 本。結果は BUG-12 以来使われていない、削除してよいかは未確認)、空の `tsf/send.rs`、`evidence_is_fresh` の 3 重複、`update_intra_batch_applied` の重複、`ConvAfterOpen`/`ConvAfterOpenId` の重複型、`decide_attempt` の二重呼び出し、`MechanismCommand` の `unreachable!` 2 variant(これは「小さな撤去」ではない: `MechanismCommand` は serde で、凍結コーパス `tests/journals/actuation_decision/*.json` に 35 件の `"command"` があり、型を分けると replay の互換性に響く)。詳細は棚卸し README §5。

## 未決定(所有者判断)

1. **進めるか**: 主動機が構造的なテスト容易性であり、ADR-224 の着手条件(見逃しの実例)を満たさない。それでも進めるか。
2. 進める場合の最初の段階を、0(機械作業)にするか、2(`platform_state` の ungate)にするか。
3. 1 ターンの入口に集約する(D3-4、`with_app` の入口を減らす)のを、将来の目標として持つか。
4. 追記する log を本番で常時持つか(私の推奨は、新しい log を作らず、既存の journal を記録点にする)。
5. UIA 経路の削除を、この計画と切り離して先に判断するか。

## Opus レビュー計画

複数ラウンドで、前提の誤り(既存との重複、ADR-224 との矛盾、順序の保証、`held_modifiers`、ガード書き換え量)を洗い出す。第1ラウンドの観点: (1) 実害の記録が弱いまま進める妥当性、(2) 段階 0〜2 の取りやめ条件の妥当性、(3) D2 のポート分類(S/B/A)が実コードに当てはまるか、(4) D6 の前提の網羅。
