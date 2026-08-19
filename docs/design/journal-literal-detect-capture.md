# literal-detect 判定結果を journal に記録する設計（C-1〜C-6）

対象: [ADR-096](../adr/096-journal-priority-tiers-multi-lane-ring-buffer.md) round2 レビューの
「C. 見落とし」で挙がった **literal-detect 判定結果の 0% 収録**（`DetectionResult`・per-VK の
状態・`raw_tsf_literal_consecutive_count`・give-up 分岐）。レビューはこれを
「ADR-096 が塞いだ 3 ギャップの次に大きい穴」と評価している
（`docs/known-bugs.md` の BUG-03/24/27/29/30/36/38/40/45 の 9 件で決め手だった）。

この文書は [journal-diagnostic-fidelity-fixes.md](./journal-diagnostic-fidelity-fixes.md)
（B-1〜B-5）の続きであり、同じく **実装のための設計書**でコードは含まない（実装は
codex CLI が行う）。各項目は「(1) 設計の概要 / (2) 変更対象 / (3) 検討した代替案と
不採用理由 / (4) 既存テスト・既存型への影響」の順で書く。最後に予算見積もりと
実装順序を置く。

---

## 0. 先に決める 4 つの横断方針

### Y-1. 記録の単位は「tick」でも「アクション」でもなく **verdict（判定が確定した瞬間）**

`LiteralDetector::check_now` は 10ms ごとに呼ばれ、大半の呼び出しは `None`（未確定）を
返す。ここを記録対象にすると B-5 で捨てたばかりの無変化 tick と同じ汚染になる。
記録するのは「1 回の判定が `DetectionResult` として確定した瞬間」だけ、と最初に固定する。

同時に、**判定が確定しなかったこと自体**も記録対象に含める（下記 Y-4）。過去バグの
決め手の複数（BUG-27 追補3、BUG-40、BUG-45 追補1）は「出るはずのログが出ていない」
という**不在**だったので、不在を不在のまま journal に落とすと同じ壁に再度ぶつかる。

### Y-2. journal への橋渡し点は `platform.rs` のみ（ADR-096 決定を維持）

`output/probe_io.rs`・`tsf/probe.rs`・`tsf/warmup/*` は `crate::journal` を一切
参照しない。判定結果は **純粋なデータ**として `dispatch_probe_actions` →
`StepProbeResult` → `WindowsPlatform::advance_tsf_probe` と持ち上げ、
`JournalEntry` への変換は platform.rs だけが行う。`StepProbeResult.completed_cold_seq`
（`Output` 層で起きた事実を platform に伝えるためだけのフィールド）が既にこのパターンの
前例であり、それを踏襲する。

### Y-3. 型の置き場所は 3 分割する

| 置き場所 | 何を置くか | cfg | Linux CI |
|---|---|---|---|
| `crates/awase-windows/src/tsf/literal_facts.rs`（**新規**） | 判定結果を表す純粋なデータ型（`LiteralVerdict` / `DetectRoute` / `DetectPath` / `DetectTarget` / `DetectEvidence` / `LiteralDetectFacts` / `LiteralDetectRecord` / `LiteralDetectTrace`） | ungated | 型として使える |
| `crates/awase-windows/src/journal_policy.rs`（既存） | 記録するか捨てるかの**純粋な判定**（`literal_detect_is_notable`） | ungated | テストが回る |
| `crates/awase-windows/src/journal.rs`（既存） | `JournalEntry::LiteralDetect` variant | `#[cfg(windows)]` | — |

`tsf/mod.rs` は ungated で、`gji_fsm` を ungated サブモジュールとして Linux から
テストできる形になっている（`probe`/`warmup` だけが `#[cfg(windows)]`）。
`literal_facts` も同じ位置に ungated で置く。

**なぜ facts を `journal_policy.rs` に置かないか**: `tsf/warmup/probe_fsm.rs` や
`output/probe_io.rs` が `crate::journal_policy::...` を名指しすると、たとえ純粋モジュール
であっても「output/tsf 層が journal を知っている」ように読める。ADR-096 の決定 2 が
守ろうとしているのはレイヤー境界そのものなので、**データ型は tsf 側・判定ポリシーは
journal 側**に分ける。`journal_policy` が `tsf::literal_facts` を参照する向き（上位→下位）
だけを許す。

### Y-4. 診断のために literal-detect の**挙動を変えない**（追加の yield を作らない）

literal-detect は [fix-requires-evidence ルール](../../.claude/rules/fix-requires-evidence.md)
の再発ファミリー（warmup / conv / キー選択）のど真ん中で、10ms tick の増減が
そのまま実害（BS の誤送信・文字欠落）に直結する。したがって:

- 診断のための `yield_step` を新設しない（= probe が 1 tick 伸びない）。
- 既に action 列を組み立てている箇所（`emit_recovery_actions`、`LiteralDetectCore::poll`
  の `Some(vec![...])` 分岐）には**同じ tick の中で**情報を相乗りさせる。
- そこにも乗せられない事象（`vk_sent 未設定 → 中断` の無アクション return）は、
  platform 側で「送ったのに判定が来ないまま probe が終わった」と**再構成**する
  （C-4）。コルーチンには一切触らない。

---

## C-1: 判定結果を表す純粋データ型と、検出根拠（evidence）の可視化

### (1) 設計の概要

`tsf/literal_facts.rs`（新規・ungated）に次を置く。すべて `Debug, Clone, Copy,
PartialEq, Eq, serde::Serialize`（`LiteralDetectRecord` のみ `Copy` を外してよい）。

```rust
/// 1 回の literal-detect 判定の結末。
pub enum LiteralVerdict {
    CompositionConfirmed,   // DetectionResult::CompositionConfirmed
    SuspectedLiteral,       // DetectionResult::SuspectedLiteral
    StaleConfirm,           // DetectionResult::StaleConfirm（ADR-079 epoch fencing）
    VetoExpired,            // 候補ウィンドウ可視のまま veto 上限超過 → 無回収で打ち切り
    SessionSkip,            // literal_session_confirmed_gen 一致で検出自体をスキップ
    PlanSkippedLiteral,     // TransmitPlan.needs_literal=false で検出フェーズに入らなかった
    AbortedNoVerdict,       // VK は送ったが判定が確定しないまま probe が終了した
}

/// その verdict をどのセンサー経路で得たか。
pub enum DetectRoute {
    CheckNow,          // LiteralDetector::check_now（write-bytes / SHOW エッジ / deadline）
    VisibleFencing,    // await_vk_detection の「候補ウィンドウ既に可視」ショートカット
    SessionFlag,       // literal_session_confirmed_gen
    PlanDecision,      // decide_transmit_plan の needs_literal
    ProbeEnd,          // platform 側の再構成（AbortedNoVerdict 専用）
}

pub enum DetectPath   { PerVk, Word }
pub enum DetectTarget { Chrome, Tsf }

/// 判定の根拠になった生の観測値（判定確定時のスナップショット）。
pub struct DetectEvidence {
    pub show_changed: bool,      // gji_candidate_show（エッジ）がベースラインから増分したか
    pub candidate_visible: bool, // gji_candidate_visible_now（レベル）
    pub write_delta: u64,        // gji_write_bytes() - write_bytes_baseline
    pub evidence_fresh: bool,    // gji_last_write_ms >= epoch_send_ms（fencing 未適用時も true）
}

pub struct LiteralDetectFacts {
    pub verdict: LiteralVerdict,
    pub route: DetectRoute,
    pub path: DetectPath,
    pub target: DetectTarget,
    pub vk: Option<u16>,   // VkCode.0（KeyEventSummary.vk_code と同じ表現）。Word パスは None
    pub idx: u16,          // per-VK の何番目か（Word パスは 0）
    pub last_idx: u16,     // per-VK の最終 index（Word パスは 0）
    pub evidence: DetectEvidence,
}
```

`DetectEvidence` を入れるのが本項の肝で、これが無いと BUG-29/BUG-30 の決め手
（「候補ウィンドウは正しく SHOW しているのに `COMPOSITION_BYTES_THRESHOLD`=350B に
構造的に届かず `SuspectedLiteral` と誤判定」「SHOW はエッジ・visible はレベルで、
`check_now` がエッジしか見ていなかった」）が journal から**読めない**。
`write_delta=40`（閾値 350 に遠く及ばない）と `show_changed=false, candidate_visible=true`
が並んで記録されていれば、両バグとも journal だけで同定できる。

採取は `LiteralDetector` に読み取り専用メソッドを 1 本足すだけで足りる:

```rust
impl LiteralDetector {
    /// 判定確定時点の生の観測値を返す（判定はしない・状態を変えない）。
    #[must_use] pub(crate) fn evidence_now(&self, candidate_visible: bool) -> DetectEvidence;
}
```

`candidate_visible` は `TsfEnvSnapshot.gji_candidate_visible_now`（tick ごとの
スナップショット）を呼び出し側から渡す。`LiteralDetector` 側でグローバルを
直読みしない（既に `veto_decision` がグローバル直読みをやめて env 経由に統一済みで、
その方針を崩さない）。

`DetectionResult` → `LiteralVerdict` の変換は `tsf/probe.rs`（`DetectionResult` の
定義側、`#[cfg(windows)]`）に `impl From<&DetectionResult> for LiteralVerdict` として
1 箇所だけ置く。`DetectionResult` 自体は**変更しない**（enum に情報を持たせると
`check_now` の 4 箇所の呼び出し元と probe.rs の 8 件の単体テストに波及する）。

### (2) 変更対象

| ファイル | 変更 |
|---|---|
| `crates/awase-windows/src/tsf/literal_facts.rs`（新規） | 上記の型定義。`serde::Serialize` derive |
| `crates/awase-windows/src/tsf/mod.rs` | `pub mod literal_facts;`（**cfg なし**、`gji_fsm` と同じ扱い） |
| `crates/awase-windows/src/tsf/probe.rs` | `LiteralDetector::evidence_now()` 追加、`impl From<&DetectionResult> for LiteralVerdict`。`DetectionResult`・`check_now`・`grace_hold_verdict`・`visible_fencing_verdict` のシグネチャと判定ロジックは**不変** |

### (3) 検討した代替案と不採用理由

- **`DetectionResult` の各 variant に根拠フィールドを持たせる。**
  一見自然だが、`DetectionResult` は `check_now` / `grace_hold_verdict` /
  `visible_fencing_verdict` の戻り値であり、`probe.rs` 内だけで 8 件の
  `matches!(result, Some(DetectionResult::X))` 形式の単体テストが直接依存している。
  判定ロジックの中心に触る変更は、診断追加の代償として大きすぎる。読み取り専用の
  `evidence_now()` を別に足せば判定ロジックは 1 行も変わらない。不採用。
- **`journal_policy.rs` に facts 型も同居させる。**
  Y-3 のとおり、`tsf/`・`output/` 側に `journal_*` という名前を持ち込みたくない。不採用。
- **evidence を採らず verdict 名だけ記録する。**
  BUG-29/BUG-30 は「verdict は分かっていたが、なぜその verdict になったかが分からな
  かった」バグなので、決め手を再現できない。この 2 件を捨てるなら本設計の価値は半減
  する。不採用。
- **`ChangeCounter` に `delta_since(baseline) -> u32` を足して SHOW の増分回数を記録。**
  `Baseline` は `has_changed` しか公開しておらず、observer の API 面を広げる必要がある。
  BUG-29/30 の識別には `show_changed: bool` と `candidate_visible: bool` の**組**で足りる
  （エッジとレベルの食い違いこそが決め手）。将来必要になったら足す。今回は不採用。

### (4) 既存テスト・既存型への影響

- `tsf/probe.rs` の既存テスト 8 件（`check_now_*`）は判定ロジック不変のため無影響。
- `evidence_now()` は `#[cfg(windows)]` 配下なので Linux CI では回らない。純粋な
  組み立て（`DetectEvidence` の生成）自体には分岐が無いので、Linux 側のテストは
  `literal_facts` の型（`Serialize` 出力の形）と C-4 の判定関数に絞る。
- `DetectionResult`・`LiteralDetector` の公開シグネチャは変更なし。

---

## C-2: 判定結果を `ProbeAction` に相乗りさせて dispatcher まで運ぶ

### (1) 設計の概要

verdict を確定させるのは**コルーチン側**（`run_per_vk_confirm` / `await_vk_detection` /
`LiteralDetectCore::poll` / `tsf_probe_coro_body` の inline ループ）だが、
`consecutive_count` と give-up 分岐を知っているのは**dispatcher 側**（`probe_io.rs`）である。
両者を合流させる唯一の既存チャネルが `ProbeAction` なので、そこに facts を載せる。

変更する variant:

```rust
ProbeAction::RawTsfLiteralRecovery { cold_seq, backs, romaji, escape_composition,
                                     facts: LiteralDetectFacts },   // ★追加
ProbeAction::CompositionConfirmed  { cold_seq, mark_literal_session,
                                     facts: LiteralDetectFacts },   // ★追加
ProbeAction::TransmitSingleVk      { cold_seq, vk, needs_shift, timeout_ms, is_last, target,
                                     idx: u16, last_idx: u16 },      // ★追加（診断専用）
ProbeAction::LiteralDetectNote(LiteralDetectFacts),                  // ★新設
```

- `LiteralDetectNote` は **副作用ゼロ**の診断専用 action。dispatcher は trace に
  積むだけで何もしない。`VetoExpired` / `SessionSkip` のように「回収も confirm も
  発行しないが判定は確定した」ケース専用。これらはいずれも
  `LiteralDetectCore::poll` が `Some(vec![ProbeAction::Done])` を組み立てている箇所
  なので、`Some(vec![Note(facts), Done])` にするだけで **tick は 1 つも増えない**（Y-4）。
- `TransmitSingleVk` の `idx`/`last_idx` は診断専用。`is_last` は挙動を駆動する既存
  フィールドなので残し、`debug_assert_eq!(is_last, idx == last_idx)` を dispatcher に
  1 行入れて 2 つの真実がずれないよう縛る。
- `PlanSkippedLiteral` は `ProbeAction::Transmit { plan, .. }` の dispatcher 側で
  `plan.needs_literal == false` を見て組み立てる（producer 側の変更不要）。
  これが BUG-40（`transmit-plan: needs_literal=false nc_fired=true` で per-VK confirm も
  inline LiteralDetect も丸ごとスキップされ、"ke" が literal のまま残った）の決め手に
  直接対応する。

facts を組み立てる producer 側の変更点:

| 場所 | 組み立てる facts |
|---|---|
| `probe_fsm.rs::await_vk_detection` | 戻り値を `DetectionResult` → `(DetectionResult, DetectRoute, DetectEvidence)` に変更（`VisibleFencing` / `CheckNow` の区別はここでしか分からない） |
| `probe_fsm.rs::run_per_vk_confirm` | `path=PerVk`, `idx`/`last_idx`/`vk`/`target` を付けて `CompositionConfirmed` / `emit_recovery_actions` に渡す |
| `probe_fsm.rs` の inline 最終確認ループ（`tsf_probe_coro_body` Phase 3） | `path=Word`, `route=CheckNow` |
| `literal_detect_fsm.rs::LiteralDetectCore::poll` | `path=Word`。`SessionSkip`（`route=SessionFlag`）・`CompositionConfirmed`（partial literal の場合は `SuspectedLiteral` ではなく `CompositionConfirmed` + `backs=PARTIAL_LITERAL_BS` として記録し、`escape_composition=true` で partial と読める）・`SuspectedLiteral`・`StaleConfirm`・`VetoExpired` |
| `literal_detect_fsm.rs::emit_recovery_actions` | 引数に `facts: LiteralDetectFacts` を追加して `RawTsfLiteralRecovery` に載せる |

### (2) 変更対象

| ファイル | 変更 |
|---|---|
| `crates/awase-windows/src/tsf/warmup/probe_fsm.rs` | `ProbeAction` の 3 variant にフィールド追加 + `LiteralDetectNote` 新設、`await_vk_detection` の戻り値、`run_per_vk_confirm` の facts 組み立て、inline ループの facts 組み立て |
| `crates/awase-windows/src/tsf/warmup/literal_detect_fsm.rs` | `emit_recovery_actions` の引数追加、`LiteralDetectCore::poll` の 5 分岐で facts 組み立て、`veto_decision` の `Expired`/session-skip 分岐に `Note` を相乗り |
| `crates/awase-windows/src/tsf/warmup/gji_warmup_coro.rs` | `run_per_vk_confirm` / `LiteralDetectCore` を呼ぶだけなので原則無変更（`DetectTarget::Tsf` の引き回しのみ） |

### (3) 検討した代替案と不採用理由

- **`ProbeAction` を触らず、dispatcher 側で `backs`/`escape_composition` から
  verdict を逆算する。**
  逆算は**原理的に不可能**。`per_vk_recovery_params` は
  `(SuspectedLiteral, idx>0) → (backs=0, escape=true)` と
  `(StaleConfirm, idx>0) → (backs=0, escape=true)` を**同じ値**に潰す。この 2 つの区別
  （BUG-33 追補4 の中心、「StaleConfirm は literal の証拠ではないので BS を送らない」）
  こそが記録したい情報なので、逆算では目的を達成できない。不採用。
- **診断専用の `LiteralDetectNote` だけを使い、既存 variant には触らない。**
  `RawTsfLiteralRecovery` / `CompositionConfirmed` を発行する箇所でも Note を別に
  yield すればフィールド追加は避けられるが、Note を単独で yield するには
  `yield_step` が 1 回増え、probe が 10ms 伸びる（Y-4 違反）。同じ action 列に
  Note を並べる手もあるが、その場合 dispatcher で「直後の recovery と対応づける」
  暗黙の順序契約が生まれ、片方だけ増減したときに静かに壊れる。verdict を発行する
  action 自身に facts を持たせるほうが構造的に安全。不採用（Note は「action を
  発行しない verdict」専用に限定する）。
- **`TransmitSingleVk` の `is_last` を `idx`/`last_idx` に置き換える（重複解消）。**
  `is_last` は `flush_deferred_and_mark_warmup` の実行条件という**挙動**を駆動している。
  診断追加のついでに挙動駆動フィールドの表現を変えるのは、BUG-27/BUG-38 が起きた
  まさにその分岐を触ることになる。`debug_assert_eq!` で縛るに留める。不採用。

### (4) 既存テスト・既存型への影響

- `matches!(a, ProbeAction::RawTsfLiteralRecovery { .. })` 形式の既存アサーション
  （`probe_fsm.rs` 3 件、`literal_detect_fsm.rs` 3 件、`probe_io.rs` 2 件）は
  `{ .. }` なのでフィールド追加で壊れない。
- 構造体リテラルで `ProbeAction` を組み立てている箇所（本番 6 / テスト 5、grep 済み）は
  フィールド追加ぶんの機械的修正が要る。
- `emit_recovery_actions` の引数追加により `literal_detect_fsm.rs` のテスト 2 件
  （`emit_recovery_actions_*`）が修正対象。挙動の期待値（`escape_composition`）は不変。
- `chrome_probe.rs` / `probe_coro_state.rs` の配線は `Vec<ProbeAction>` を素通しする
  だけなので無変更。
- `state::ime_event::ImeEvent`・reducer には一切触れない。

---

## C-3: dispatcher の観測を `StepProbeResult` 経由で platform へ持ち上げる

### (1) 設計の概要

`dispatch_probe_actions` は現在 `DispatchResult`（`Done`/`Continue`/`LearnedTsf`）
だけを返す。ここに **out パラメータ**として trace を足す:

```rust
// tsf/literal_facts.rs
pub enum LiteralDetectTraceItem {
    /// per-VK confirm で 1 VK を送信した（まだ判定は出ていない）。
    VkSent  { cold_seq: u64, vk: u16, idx: u16, last_idx: u16, target: DetectTarget },
    /// 判定が確定した。
    Verdict(LiteralDetectRecord),
}

#[derive(Default)]
pub struct LiteralDetectTrace(pub Vec<LiteralDetectTraceItem>);

/// producer 側の facts に、dispatcher しか知らない事実を合流させたもの。
pub struct LiteralDetectRecord {
    pub cold_seq: u64,
    pub facts: LiteralDetectFacts,
    /// この判定を処理する直前の `raw_tsf_literal_consecutive_count`。
    pub consecutive_before: u32,
    /// give-up 分岐（`consecutive != 0`）に落ちたか。true なら romaji 再送なし +
    /// `schedule_chrome_gji_reinit`（BUG-33/36）。
    pub gave_up: bool,
    pub backs: usize,
    pub escape_composition: bool,
    pub session_marked: bool,   // CompositionConfirmed { mark_literal_session }
}
```

```rust
// output/probe_io.rs
pub(crate) fn dispatch_probe_actions<M, I>(
    machine: &mut M,
    initial_actions: Vec<ProbeAction>,
    io: &I,
    trace: &mut LiteralDetectTrace,   // ★追加
) -> DispatchResult
```

dispatcher が trace に積むタイミング:

| action | 積むもの |
|---|---|
| `TransmitSingleVk` | `VkSent { .. }`（送信の**後**、`apply_vk_sent` の直前） |
| `RawTsfLiteralRecovery` | `Verdict`（`consecutive_before = io.consecutive_count()`、`gave_up = consecutive != 0`） |
| `CompositionConfirmed` | `Verdict`（`consecutive_before` は `reset_consecutive_count()` を呼ぶ**前**に読む） |
| `LiteralDetectNote(facts)` | `Verdict`（`consecutive_before = io.consecutive_count()`、`gave_up=false`） |
| `Transmit { plan, .. }` かつ `plan.needs_literal == false` | `Verdict`（`verdict=PlanSkippedLiteral`, `route=PlanDecision`, `path=Word`） |

`Output::step_probe` は `LiteralDetectTrace::default()` を用意して
`dispatch_probe_actions` に渡し、結果を `StepProbeResult` に載せる:

```rust
pub(crate) struct StepProbeResult {
    // ... 既存 5 フィールド ...
    pub literal_detect: crate::tsf::literal_facts::LiteralDetectTrace,   // ★追加
}
```

`step_probe` の 4 つの return 地点（pending 無し / Done / Continue / LearnedTsf）
すべてで trace を載せる（pending 無しの早期 return は空の trace）。

### (2) 変更対象

| ファイル | 変更 |
|---|---|
| `crates/awase-windows/src/tsf/literal_facts.rs` | `LiteralDetectTraceItem` / `LiteralDetectTrace` / `LiteralDetectRecord` |
| `crates/awase-windows/src/output/probe_io.rs` | `dispatch_probe_actions` の第 4 引数、5 つの action ハンドラでの trace 追記、`LiteralDetectNote` ハンドラ（trace 追記のみ・副作用なし）、`debug_assert_eq!(is_last, idx == last_idx)` |
| `crates/awase-windows/src/output/mod.rs` | `StepProbeResult.literal_detect` 追加と 4 箇所の構築 |

### (3) 検討した代替案と不採用理由

- **`DispatchResult` を struct 化して trace を内包する。**
  `step_probe` の `match dispatch { Done => ..., Continue => ..., LearnedTsf => ... }`
  という読みやすい分岐が崩れ、`probe_io.rs` のテスト 16 件が `result.is_done()`
  ではなく `result.outcome.is_done()` に一斉変更になる。out パラメータのほうが差分が
  小さく、テストは `&mut LiteralDetectTrace::default()` を渡すだけで済む
  （かつ give-up 系の既存テストは trace を受けてアサーションを**足せる**）。不採用。
- **`ProbeIo` トレイトに `fn note_literal_detect(&self, record)` を足して `Output` に
  溜める。**
  `Output` が診断バッファを持つことになり、`FakeProbeIo` にも実装が要る。`Output` は
  既に責務過多（ADR-036 の runtime boundary 整理の対象）で、**取り出し忘れ**という
  BUG-27 追補3（`ChromeProbe::apply_vk_sent` の委譲漏れ）と同型の事故を新設する。
  戻り値で持ち上げる（＝取り出し忘れが型で起きえない）ほうが安全。不採用。
- **`platform.rs` が `Output` の内部状態（`composition.consecutive_count()`）を毎 tick
  読んで差分から推定する。**
  give-up 分岐に入ったかどうかは `consecutive` の増分からは区別できない
  （retry も give-up も `mark_cold_raw_tsf()` で +1 する）。不採用。
- **`log::warn!` の出力をパースして journal に載せる。**
  ログ文字列は SSOT ではなく、フォーマット変更で静かに壊れる。不採用。

### (4) 既存テスト・既存型への影響

- `probe_io.rs` の `dispatch_probe_actions` 呼び出し 16 箇所（すべて `#[cfg(test)]`）と
  本番 1 箇所（`output/mod.rs::step_probe`）が引数追加の修正対象。機械的。
- 既存の give-up テスト（`gji_reinit_scheduled_count` を検証しているもの 2 件）に、
  `trace` に `gave_up: true` の `Verdict` が 1 件積まれることのアサーションを追加する
  ことを推奨（[fix-requires-evidence](../../.claude/rules/fix-requires-evidence.md) の (a)）。
  ただし `probe_io.rs` は `#[cfg(windows)]` なので Linux CI では回らない点に注意
  （Linux で守るのは C-4 の判定関数）。
- `StepProbeResult` は `pub(crate)` で `output/mod.rs` と `platform.rs` のみが触る。
  外部 API 影響なし。

---

## C-4: 記録条件（無意味な反復を捨て、意味のある遷移を逃さない）

### (1) 設計の概要

**素朴に「verdict が出たら全部記録」は不可**。健全なタイピングでも cold セッションの
たびに per-VK confirm が 1〜3 VK 分の `CompositionConfirmed` を出す。しかも BUG-27 追補2 の
実機では **cold が打鍵ごとに発火**（`cold=99,100,101...105`）していたので、最悪ケースでは
1 打鍵あたり数件になる。

一方「`consecutive_count` が変化したときだけ」では不十分。BUG-45 追補1 の決め手は
「`idx=0` で早期 return したこと自体」＝ per-VK の位置情報であり、
BUG-27 追補3 の決め手は「`apply_vk_sent SET` ログが出ていない」＝ **判定が出なかった
こと**だった。カウンタ変化だけを見ていると両方とも取り逃す。

そこで B-5 の `probe_tick_is_notable` と同じ思想の純粋関数を置く:

```rust
// journal_policy.rs
#[must_use]
pub fn literal_detect_is_notable(record: &LiteralDetectRecord) -> bool {
    use LiteralVerdict::*;
    match record.facts.verdict {
        // 失敗・打ち切り・不在は常に記録する（頻度が低く、決め手になる）
        SuspectedLiteral | StaleConfirm | VetoExpired
        | PlanSkippedLiteral | AbortedNoVerdict => true,
        // 「そのセッションで literal-detect が最後まで通った」印は残す
        CompositionConfirmed if record.session_marked => true,
        // 失敗の連鎖中（＝カウンタが 0 に戻る瞬間）の confirm は必ず残す
        CompositionConfirmed => record.consecutive_before > 0 || record.gave_up,
        // 健全な高速パスのスキップは、失敗の連鎖中だけ残す
        SessionSkip => record.consecutive_before > 0,
    }
}
```

捨てた分は黙って消さない。`WindowsPlatform` に `suppressed_literal_confirms: u16` を
持ち、抑制のたびに +1、次に記録する entry の `suppressed_confirms` に載せて 0 に戻す
（B-5 の `TsfProbeTick(#N, skipped=M)` と同じ作法）。

**`AbortedNoVerdict` の再構成（Y-4）**: platform が

```rust
pending_literal_vk: Option<PendingVk { cold_seq, vk, idx, last_idx, target, sent_at_ms }>
```

を持ち、trace を順に処理して

- `VkSent` → `pending_literal_vk` を（黙って）置き換える。
  置き換えが起きるのは「前の VK が confirm された（その confirm は
  `pending_confirm` として次の `TransmitSingleVk` に相乗りしてくるので、順序上
  先に `Verdict` として処理済み）」か「前の VK の confirm が
  `SuspectedLiteral` 分岐で破棄された（既存挙動）」かのどちらかで、
  いずれも新たな entry を出す価値はない。
- `Verdict` → 同じ probe の `pending_literal_vk` をクリアする。
- probe の終了（`result.timer_cmd == TimerCommand::Kill{ TIMER_TSF_PROBE }`、
  すなわち B-5 の `terminal_timer` が true）と `reset_probe_tick_counters()`
  （新しい probe の開始 = 途中差し替え）で `pending_literal_vk` が残っていたら、
  `verdict=AbortedNoVerdict, route=ProbeEnd` の record を組み立てて記録する
  （`since_vk_sent_ms = current_tick_ms() - sent_at_ms`）。

これで「`vk_sent 未設定 → 中断`」（BUG-27）と「`idx=0` で早期 return して以降の VK が
送られなかった」（BUG-45）が、**不在ではなく 1 件の entry として**残る。

### (2) 変更対象

| ファイル | 変更 |
|---|---|
| `crates/awase-windows/src/journal_policy.rs` | `literal_detect_is_notable` + 純粋テスト |
| `crates/awase-windows/src/platform.rs` | `suppressed_literal_confirms: u16` / `pending_literal_vk: Option<PendingVk>` フィールド、`advance_tsf_probe` での trace 消費、`reset_probe_tick_counters()` での flush |

### (3) 検討した代替案と不採用理由

- **全 verdict を無条件に記録する。**
  BUG-27 追補2 型の病的ケース（cold が打鍵ごと）で timing レーンが literal-detect で
  埋まり、`FocusChange`/`ImeOn`/`LongIdleTimeout` を押し出す。B-5 で直したばかりの
  構造を作り直すことになる。不採用。
- **`consecutive_count` が変化した瞬間だけ記録する。**
  per-VK の位置（BUG-45）と「判定が来なかった」（BUG-27 追補3）を取り逃す。不採用。
- **N 件に 1 件のサンプリング / レート制限。**
  病的ケースの「同じ失敗が単調に繰り返される」ことそのものが証拠
  （BUG-27 追補2 は `count=6→7→…→12` が 0 に戻らない事実が決め手）なので、
  間引くと目的を壊す。抑制するのは**健全な確認**だけにする。不採用。
- **`AbortedNoVerdict` をコルーチン側から `yield_step(ch, vec![Note])` で明示的に出す。**
  probe が 1 tick（10ms）伸びる。`run_per_vk_confirm` の中断は BUG-27 で「打鍵ごとに
  毎回発火」しうると分かっている経路なので、そこに 10ms を足すのは診断のための
  挙動改変として重すぎる（Y-4）。platform 側の再構成で同じ情報が得られる。不採用。
- **`pending_literal_vk` を `Output` 側（`warmup_coord`）に持たせる。**
  probe machine の生存期間に縛られ、まさに「machine が消えた」ケースを記録できない。
  platform 側に置くのが正しい。不採用。

### (4) 既存テスト・既存型への影響

- `journal_policy.rs` は ungated なので `literal_detect_is_notable` の全 variant テストが
  **Linux CI で回る**（`cargo test -p awase-windows --lib`）。B-5 の
  `probe_tick_is_notable_for_each_fact` と同じ形で 7 variant + 抑制条件を網羅する。
- `platform.rs` は `#[cfg(windows)]` なので `pending_literal_vk` の遷移自体は
  Linux で守れない。実機確認項目（下記 C-6）で担保する。
- 既存の `probe_tick_index` / `suppressed_probe_ticks` のリセット箇所
  （`install_pending_tsf_and_set_timer` と `GjiAction::StartProbe`）に flush を足すため、
  B-5 で入れた `reset_probe_tick_counters()` が唯一のリセット点である構造を維持する
  （リセット点を増やさない）。

---

## C-5: `JournalEntry::LiteralDetect` の形（+ `TsfProbeStarted` への 1 フィールド追加）

### (1) 設計の概要

```rust
/// literal-detect（raw TSF literal 判定）1 回分の結果。timing レーン。
LiteralDetect {
    record: crate::tsf::literal_facts::LiteralDetectRecord,
    /// この記録の前に抑制した「健全な確認」の件数（B-5 の skipped= と同じ）。
    suppressed_confirms: u16,
    /// 対応する VK 送信からこの判定確定までの経過。per-VK 以外は 0。
    /// **期間**なので X-3（entry に絶対時刻を入れない）の規約に適合する。
    since_vk_sent_ms: u64,
},
```

- `lane_kind()` は `LaneKind::Timing`（`GjiFsmTransition` / `TsfProbeStarted` /
  `TsfProbeCompleted` と同格）。
- ペイロードを `record` として**型ごと**入れるのは、ADR-082 決定 1（`ImeEvent` を
  自由文字列でなく型として記録する）と `ImeActuation { record: ActuationRecord }` の
  前例に倣う。`format!("{:?}")` の自由文字列にすると、後からフィールド単位で
  クエリできない（`TsfProbeCompleted.outcome: String` の反省）。
- `LiteralDetectRecord` の各 enum は unit variant なので `Serialize` derive で
  `"SuspectedLiteral"` のような文字列になり、`AppKind`/`FocusKind` を
  `format!("{:?}")` で入れている `FocusEndpoint` と同じ読み味になる。
  `AppKind` と違って `literal_facts` は本設計で新設する型なので、`Serialize` を
  直接 derive してよい（他所へ波及しない）。

**併せて `TsfProbeStarted` に `consecutive_at_start: u32` を足す。**
`raw_tsf_literal_consecutive_count` は `CompositionConfirmed` 以外に
`CompositionState::on_focus_changed` と `mark_composition_cold(FocusChange|SetOpenTrue)`
でも 0 に戻る。これらは dispatcher を通らないので trace には現れない。probe 開始時点の
値を 1 個だけ載せておけば、「カウンタが単調増加して二度と 0 に戻らない」
（BUG-27 追補2 の決め手）が **verdict entry が 1 件も無い状況でも**読める。
platform は `self.output.composition.consecutive_count()`（`Output.composition` は `pub`、
`CompositionState::consecutive_count()` は `pub const fn`）で読める。

### (2) 変更対象

| ファイル | 変更 |
|---|---|
| `crates/awase-windows/src/journal.rs` | `JournalEntry::LiteralDetect` 追加、`lane_kind()` に `Timing` として登録、`TsfProbeStarted` に `consecutive_at_start: u32` |
| `crates/awase-windows/src/platform.rs` | `advance_tsf_probe` で trace → entry 変換（`push_journal_entry`）、`TsfProbeStarted` を push する 2 箇所（`install_pending_tsf_and_set_timer` / `GjiAction::StartProbe`）で `consecutive_at_start` を埋める |

### (3) 検討した代替案と不採用理由

- **`GjiFsmTransition { trigger: "LiteralDetect(...)" }` に文字列として相乗りさせる。**
  variant を増やさずに済むが、`trigger: String` の自由文字列に戻ることになり、
  `verdict`/`consecutive_before`/`idx` をフィールドとして取り出せない。ADR-082 決定 1 が
  廃止した形式そのもの。不採用。
- **`actuation` レーンに入れる。**
  `ImeActuation`（awase 自身の能動的訂正）と近いが、literal-detect は
  「warm/cold・TSF probe のタイミング」カテゴリで、ADR-096 が `timing` レーンを
  新設した動機（過去バグ診断で最頻出）そのものである。`TsfProbeStarted`/`Completed` と
  同じ時系列で読めることに価値があるので `timing`。不採用。
- **verdict ごとに別 variant（`LiteralConfirmed` / `LiteralSuspected` / …）を作る。**
  読む側が 7 variant を突き合わせないと 1 本の判定履歴にならない。B-3 で
  `FocusTransition` に `changed` 軸を持たせて 1 本化したのと同じ判断。不採用。
- **`consecutive_at_start` の代わりに `ColdContext` のリセット全箇所を記録する。**
  `tsf/probe.rs::CompositionState` に journal を持ち込むか、リセット経路すべてを
  platform 経由に作り替える必要がある（Y-2 違反）。probe 開始時点の値 1 個で
  「0 に戻ったか」は十分読める。不採用。

### (4) 既存テスト・既存型への影響

- `TsfProbeStarted` への追加は JSON の additive change。読み手は人間とサーバ保管のみ。
- `tests/journal_replay.rs` は `ConvClassifyCall` のみ、
  `tests/drift_correction_replay.rs` は `ActuationRecord` のみを扱うので**無影響**。
- `tests/architecture_guard.rs`（33 件）: 新しい `observations.record(` も
  `dispatch_event(` も追加しないため既存ガードは通る。`d1_no_vk_magic_hex_outside_vk_rs`
  にも抵触しない（VK は `vk.0` を u16 として運ぶだけで、hex リテラルを書かない）。
  **新規ガードを 1 件足すことを推奨**（下記）。
- `tests/layer_boundary_guard.rs`: `b1_with_app_confined_to_orchestrator_modules` の
  許可リストに `output/probe_io.rs` が既にあるが、本設計は `with_app` を増やさない。
- 推奨する新規ガード（`architecture_guard.rs`、33 → 34 件）:
  「`src/output/**` と `src/tsf/**` の本番コードに `crate::journal`（`journal_policy` を
  除く）への参照が無いこと」。ADR-096 決定 2 と Y-2 を機械可読にする。今回
  `probe_io.rs` に診断コードを足すので、次の担当者が「ついでに journal を呼べば早い」と
  する誘惑を構造的に止める価値が最も高いタイミング。

---

## C-6: `timing` レーンの予算圧迫の見積もり

### (1) 設計の概要（見積もり）

**1 entry のサイズ**: compact JSON で envelope 込み **約 380〜420 バイト**
（`GjiFsmTransition` の約 2〜2.5 倍）。以下の見積もりでは 400 B を使う。

**発火頻度**（`literal_detect_is_notable` 適用後）:

| 状況 | 頻度 | 内訳 |
|---|---|---|
| warm セッション中の通常タイピング | **0 件** | literal-detect フェーズに入らない |
| 健全な cold セッション（フォーカス変更 / long idle 後の最初の 1 モーラ） | **1 件** | 最終 `CompositionConfirmed`（`session_marked=true`）のみ。途中の per-VK confirm は抑制され `suppressed_confirms` に集約 |
| literal 化 → 再送 → 成功 | **2〜3 件** | `SuspectedLiteral` + 再送 probe の `CompositionConfirmed`（`consecutive_before>0` で記録） |
| 病的ケース（BUG-27 追補2 型: 打鍵ごとに cold + 毎回中断） | **1〜2 件/打鍵** | `AbortedNoVerdict` または `SuspectedLiteral`(+give-up) |

**レーン容量（512 件）への影響**: 通常運用では 1 分あたり数件〜十数件で、
B-5 後の `TsfProbeTick`（probe 1 回あたり 3〜5 件）より少ない。病的ケースでは
毎秒 5〜10 件になり、timing レーン 512 件は約 50〜100 秒分になる。

**添付予算（B-1）への影響**: `Timing` の予備枠は 256KiB の 35% ＝ 約 91.7KiB。
400 B/件なら **約 235 件**が入る。病的ケースでは literal-detect entry だけで
Timing 枠を約 20〜40 秒分で埋め切る計算になるが、**その状況ではまさにその 20〜40 秒が
欲しい情報**であり、B-1 が「直近から遡って」採るため症状発生の瞬間が残る。
通常運用では literal-detect entry は Timing 枠の 1 割未満に留まる。

**判断**: レーン容量（512）も予備枠比率（35%）も**今回は変更しない**。
[tuning-constants ルール](../../.claude/rules/tuning-constants.md) が戒める
「同じ役割の定数の盲目的エスカレーション」を、journal のレーン容量にも適用する。
実測してから動かす。実機で測る項目:

1. 健全なタイピング 1 分間（フォーカス変更を数回含む）で timing レーンに積まれた
   `LiteralDetect` entry が **10 件以下**であること。
2. 病的ケースを再現できた場合、`AbortedNoVerdict` / `SuspectedLiteral` の entry が
   1 打鍵あたり 2 件を超えないこと（超えるなら `literal_detect_is_notable` を絞る）。
3. 「不具合を報告」で添付された JSON の中で、`LiteralDetect` が占めるバイト数の割合。
   Timing 枠の 50% を恒常的に超えるなら、`GjiFsmTransition` が押し出されていないかを
   確認した上で枠比率の再配分を検討する（容量を増やす前に、まず何が押し出されて
   いるかを見る）。

### (2) 変更対象

なし（測定と、必要になったときの `journal_policy.rs` の定数調整のみ）。

### (3) 検討した代替案と不採用理由

- **先回りして timing レーンを 512 → 1024 にする。**
  実測前の増量は tuning-constants ルールが禁じる形そのもの。B-5 で無変化 tick を
  捨てた効果がどれだけ効いているかもまだ実機で測れていない。不採用。
- **literal-detect 専用の 5 本目のレーンを作る。**
  B-1 の予算配分（4 レーンの比率）と `CappedJson.dropped_by_lane: [(LaneKind, usize); 4]`
  の固定長配列に波及する。literal-detect は `TsfProbeStarted`/`Completed` と
  **同じ時系列で読めること**に価値があり、隔離は目的に反する。不採用。
- **compact 化のためにフィールド名を短縮する（`consecutive_before` → `cb` 等）。**
  人間が読む前提の journal で可読性を捨てる割に、削減は 2 割程度。不採用。

### (4) 既存テスト・既存型への影響

- `journal_policy.rs` の `RESERVED_PERCENT` / `lane_capacity` は変更しないため、
  B-1 の `select_tail_*` テストは無影響。

---

## 過去バグ 9 件と、記録されるフィールドの対応

| バグ | 当時の決め手 | 本設計で残るもの |
|---|---|---|
| BUG-03 | GJI SHOW が T+O 自体の成否と無関係に発火し偽陽性 `CompositionConfirmed` | `verdict=CompositionConfirmed` + `evidence{show_changed=true, write_delta<350}`（SHOW だけで confirm した事実） |
| BUG-24 | `nc_fired=false` を代理指標にした `is_partial_literal` の偽陽性 | partial literal 回収は `verdict=CompositionConfirmed, backs=1, escape_composition=true` として現れ、通常の confirm と区別できる |
| BUG-27 | `per-VK[0/1] vk_sent 未設定 → 中断` が毎打鍵発火 / `count=6→7→…→12` が 0 に戻らない / `apply_vk_sent SET` ログの**不在** | `verdict=AbortedNoVerdict` + `idx/last_idx` + `consecutive_before` の推移 + `TsfProbeStarted.consecutive_at_start` |
| BUG-29 | VK1 以降で `suspected literal` 誤発火（SHOW がエッジ / 350B が構造的に未達） | `idx>0` + `evidence{show_changed=false, candidate_visible=true, write_delta≪350}` |
| BUG-30 | `gji_candidate_show`（エッジ）と `gji_candidate_visible`（レベル）の混同 | `evidence.show_changed` と `evidence.candidate_visible` の**食い違い**が 1 entry 内で読める |
| BUG-33/36 | 2 連続 literal → give-up → reinit（BS より先に reinit が出るレース） | `gave_up=true` + `consecutive_before` + `backs`（reinit は give-up と 1:1 なので `gave_up` で読める） |
| BUG-38 | give-up 分岐が `pending_deferred` を flush せず出力順が逆転 | `gave_up=true` の時刻（`seq`）と、B-4 で発生順に採番された `KeyInput` の相対順序 |
| BUG-40 | `transmit-plan: needs_literal=false nc_fired=true` で検出フェーズごとスキップ | `verdict=PlanSkippedLiteral, route=PlanDecision`（cold probe が走ったのに検出が入らなかったことが正の記録になる） |
| BUG-45 | `vk=0x4B` が `idx=0` で suspected → give-up → BS×1 + reinit / `vk=0x41` のログが**不在** | `vk` + `idx/last_idx` + `verdict` + `gave_up` + `backs`、送られなかった VK は `AbortedNoVerdict` の `last_idx` から読める |

**残る限界（今回は解消しない）**:

- `nc_fired` / `gji_settled`（`ProbeObservations`）そのものは記録しない。BUG-40 は
  「検出がスキップされた」ところまでは読めるが「なぜ `nc_fired` が true に化けたか」は
  読めない。`TransmitPlan` に `ProbeObservations` のサマリを載せる拡張は別途。
- `LiteralDetector::check_now` の内部分岐（`grace_hold_verdict` の hold 中）は
  `None` を返し続けるだけなので記録されない。hold の長さは `since_vk_sent_ms` から
  間接的にしか読めない。
- `raw_tsf_literal_consecutive_count` の focus-change リセットは
  `TsfProbeStarted.consecutive_at_start` からしか観測できない（Y-2 を守るため）。

---

## 実装順序の推奨

**B-1〜B-5 との依存**: 本設計（C-1〜C-6）は B-1〜B-5 の**上に積む**が、機能的な依存は
薄い。前提として使うのは既に実装済みの 3 点のみ:

- **B-4 の `JournalStamper` / `push_journal_entry`**: 新 entry は `push_journal_entry` を
  通るので、発生順の `seq` が自動的に付く（自前で採番しない）。
- **B-5 の `reset_probe_tick_counters()`**: `suppressed_literal_confirms` と
  `pending_literal_vk` のリセットを**同じ関数に相乗り**させる（リセット点を増やさない）。
- **B-1 の `to_json_capped`**: 新 entry は `lane_kind()` で Timing に入るので、
  レーン別予備枠の恩恵を自動的に受ける。

逆に B-2/B-3（focus 系）とは完全に独立で、順序の制約はない。

| 順 | 項目 | 理由 |
|---|---|---|
| 0 | **C-1**（`tsf/literal_facts.rs` 新設 + `evidence_now()` + `From` impl） | 誰も使わない純粋な型追加なので挙動不変。単独でマージでき、レビューが軽い |
| 1 | **C-2**（`ProbeAction` への facts 相乗り） | producer 側だけ。dispatcher は facts を無視するので**この時点でも挙動不変**。既存の probe_io テスト 16 件が通ることが「挙動を変えていない」ことの証拠になる |
| 2 | **C-3**（trace の持ち上げ） | ここまでで journal には何も出ないが、`probe_io.rs` のテストで trace の中身を検証できるようになる |
| 3 | **C-4**（記録条件 + `AbortedNoVerdict` 再構成） | `journal_policy.rs` の純粋テストを Linux CI で回す。ここが本設計で唯一「捨てる判断」をする場所なので、独立したレビュー単位にする |
| 4 | **C-5**（`JournalEntry` 追加 + `consecutive_at_start`） | ここで初めて journal に出る。C-3/C-4 が済んでいれば platform 側は変換だけ |
| 5 | **C-6**（実機測定） | Windows 実機。ADR-095/096 に残っている実機未検証項目とまとめて消化する |

各段のテスト方針（[fix-requires-evidence](../../.claude/rules/fix-requires-evidence.md) 準拠。
本設計は `output/probe_io.rs`・`tsf/`・`platform.rs` という**再発ファミリーのど真ん中**を
触るため、pre-push フックが警告を出す。テスト追加で応える）:

- 0〜2: 既存の `probe_io.rs` / `probe_fsm.rs` / `literal_detect_fsm.rs` のテストが
  **期待値を変えずに**通ること（＝挙動不変の証拠）。give-up 系 2 件に trace の
  アサーションを追加。
- 3: `journal_policy::literal_detect_is_notable` の全 7 verdict × 抑制条件の純粋テスト
  （Linux CI）。
- 4: `architecture_guard` に「`output/**`・`tsf/**` は `crate::journal` を参照しない」
  ガードを 1 件追加（33 → 34 件）。
- 5: 実機で「不具合を報告 → 添付 JSON に `LiteralDetect` entry が含まれ、
  健全なタイピング 1 分で 10 件以下」を確認。

ドキュメント: 完了後に ADR-096 の「既知の限界・未決定事項」から
「literal-detect 判定結果が 0% 収録」の項目を削除し、round2 と同様に
「round3: literal-detect の収録（C-1〜C-6）」節を追記して本文書へリンクする。
`docs/known-bugs.md` の BUG-27/29/30/45 には、次に同種の症状が出たときの
**journal での確認手順**（`entry.type == "LiteralDetect"` を `seq` 順に並べ、
`verdict`/`idx`/`consecutive_before` を見る）を 1 行ずつ追記することを推奨する。
