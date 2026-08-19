# journal 診断精度の是正設計（ADR-096 アドバーサリアルレビュー指摘 B-1〜B-5）

対象: [ADR-096](../adr/096-journal-priority-tiers-multi-lane-ring-buffer.md) 実装後の
レビュー指摘 5 件（B-1 must-fix / B-2〜B-5 should-fix）。

この文書は **実装のための設計書**であり、コードは含まない（実装は codex CLI が行う）。
各項目は「(1) 設計の概要 / (2) 変更対象 / (3) 検討した代替案と不採用理由 /
(4) 既存テスト・既存型への影響」の順で書く。最後に実装順序の推奨を置く。

---

## 0. 先に決める 3 つの横断方針

B-1〜B-5 を個別に直すと、時間軸・レーン・予算の扱いが 3 通りに分裂する。先に
横断方針を固定してから各項目を実装すること。

### X-1. journal の時系列 SSOT は `seq`（`elapsed_ms` は表示用）

`JournalEnvelope.elapsed_ms` は quanta 由来のミリ秒丸めで、同一 ms に複数 entry が
並ぶ。**因果順を読むときは常に `seq` を使う**ことを journal.rs の doc comment に
明記し、`to_json()` のマージも `seq` を唯一のキーにする（現状もそうなっている）。

この方針の帰結として、**`seq` は「発生した瞬間」に採番されなければならない**。
これが B-4 の中心。

### X-2. 「採番・時刻採取」だけを切り出したハンドル `JournalStamper` を新設する

`WindowsPlatform` に `UnifiedJournal` 本体（レーン群・dump・容量）を持たせずに、
「発生時刻で envelope を作る」能力だけを渡すための最小ハンドル。詳細は B-4。

### X-3. 3 系統の時間軸は統一せず、「相互変換できる」状態にする

現状の 3 軸:

| 軸 | 実体 | 起点 | 主な使用箇所 |
|---|---|---|---|
| `elapsed_ms` | `quanta::Clock`（µs→ms 丸め） | journal 生成時 | `JournalEnvelope` |
| `tick_ms` | `GetTickCount64`（`hook::current_tick_ms`） | OS 起動 | `TickMs` 全般・`tuning.rs` の全閾値・`TsfProbeCompleted.elapsed_ms` の算出 |
| `timestamp_us` | `Instant`（`hook::now_timestamp`） | 初回呼び出し | `RawKeyEvent.timestamp` → `KeyEventSummary.timestamp_us`・`[drain-start]` ログ |

**統一しない**判断とする。理由:

- `tick_ms` は `tuning.rs` の全タイミング閾値と `focus`/`ime` の state ロジックが
  依存する既存 SSOT で、置き換えは診断の範囲を超える改造かつ回帰リスクが大きい
  （[tuning-constants ルール](../../.claude/rules/tuning-constants.md) の実測義務が
  かかる領域を全面的に触ることになる）。
- `quanta::Clock` は journal がテストでモック注入するための軸であり、
  `hook::current_tick_ms()` に替えると `journal_elapsed_ms_advances_with_clock` 等の
  既存単体テストが成立しない。加えて 15.6ms 分解能ではバースト内の順序が読めない。
- `RawKeyEvent.timestamp` はフック側（キーボードフックのコンテキスト）で採取する
  必要があり、journal の生存期間に依存させられない。

かわりに **2 点アンカーで相互変換可能にする**:

- 新 variant `JournalEntry::ClockAnchor { tick_ms: u64, hook_us: u64 }` を追加し、
  (a) 起動直後（journal 生成後の bootstrap から 1 回）と (b) 各 dump の直前
  （`DumpTriggered` と同時）に記録する。envelope 側が `seq`/`elapsed_ms` を持つので、
  この 2 点で 3 軸の相互変換式とクロック間ドリフト量そのものが求まる。
- 併せて **「entry のペイロードに絶対時刻を入れない」規約**を明文化する。
  `TsfProbeCompleted.elapsed_ms` のような**期間**は軸に依存せず読めるので可。
  `last_focus_change_ms` のような絶対 tick を入れたくなったら必ず期間へ変換する
  （B-2/B-3 の `dwell_ms` はこの規約に従う）。

> 注意: `ClockAnchor` の記録は `UnifiedJournal::new()` の中でやらないこと。
> journal.rs は `#[cfg(windows)]` だが `new()` は Linux でも（Windows ビルドでの）
> 単体テストから呼ばれ、Win32 呼び出しを混ぜると純粋なコンストラクタでなくなる。
> 呼び出し元（`app/bootstrap.rs`）から `journal.record(ClockAnchor { .. })` する。

### X-4. 純粋判定は ungated モジュール `journal_policy.rs` に置く

`journal.rs`・`platform.rs` は `#[cfg(windows)]` のため、そこに書いた判定ロジックは
Linux CI（`cargo test -p awase-windows --lib`）で守れない。B-1 の予算配分と B-5 の
tick 抑制判定は**純粋関数**なので、新設する ungated モジュール
`crates/awase-windows/src/journal_policy.rs`（`lib.rs` の「純粋モジュール」群に追加）
に置き、Linux で回帰テストする。既存の作法（`focus/{cache,class_names}`、
`tsf/gji_fsm` を ungated にして Linux でテストしている）と同じ考え方。

`journal_policy.rs` が持つもの:

- `pub enum LaneKind { State, Timing, Actuation, KeyInput }`
  （`journal.rs` の `JournalLaneKind` をここへ移設し、journal.rs は re-export する）
- `pub const fn lane_capacity(lane: LaneKind) -> usize`（現行の 4 定数を集約）
- `pub struct BudgetItem { pub seq: u64, pub lane: LaneKind, pub bytes: usize }`
- `pub fn select_tail_within_budget(items: &[BudgetItem], max_bytes: usize) -> Vec<usize>`（B-1）
- `pub const fn probe_tick_is_notable(...) -> bool`（B-5）

---

## B-1: 添付ログを「直近から遡って予算内に収める」capped シリアライザにする

### (1) 設計の概要

現状は `bug_report::truncate_utf8_bytes` が `input[..end]` で**先頭（最古）**を
残しており、「報告ボタンを押す直前」＝症状発生の瞬間が丸ごと落ちる。さらに
配列の途中で切れるので JSON として不正になりうる。

三層で直す。

**第 1 層（一次防衛・本命）: `UnifiedJournal` が capped シリアライザを持つ**

```rust
// journal.rs
pub struct CappedJson {
    pub json: String,
    pub total_entries: usize,
    pub emitted_entries: usize,
    pub dropped_by_lane: [(LaneKind, usize); 4],
}

impl UnifiedJournal {
    /// 直近から遡って `max_bytes` 以内に収まる entry 集合を JSON 配列として返す。
    /// 出力は必ず妥当な JSON 配列であり、`seq` 昇順。
    pub fn to_json_capped(&self, max_bytes: usize) -> Result<CappedJson, DumpError>;

    /// `to_json_capped` の結果を `%TEMP%/awase_journal_<tick>.json` に書き出す。
    pub fn dump_to_file_capped(&self, max_bytes: usize) -> Result<std::path::PathBuf, DumpError>;
}
```

アルゴリズム:

1. 全レーンの各 `JournalEnvelope` を **compact**（`serde_json::to_string`、pretty で
   なく）で個別に直列化し、`(seq, lane, bytes)` の `BudgetItem` を作る。
   pretty のままだと 1 entry あたり 2〜3 倍に膨らみ、256KiB に入る件数が 1/2〜1/3 に
   なる。報告用ペイロードは機械送信物なので compact でよい（Alt トリガの手動 dump は
   従来どおり pretty のまま残す）。
2. `journal_policy::select_tail_within_budget` に投げて採用インデックスを得る。
   配分ルール（`journal_policy.rs` の定数、実装時に調整可）:
   - レーン別予備枠: `Timing 35% / State 30% / Actuation 20% / KeyInput 15%`。
     ADR-096 の「高頻度・低価値が低頻度・高価値を押し出さない」構造をリングバッファ
     だけでなく**添付予算にも**適用する（リングバッファでレーン分割しても、最後に
     1 本の byte 予算で先着順に切ったら同じ押し出しが起きる）。
   - 各レーンは自枠内で **`seq` 降順**（新しい順）に採用。
   - 余った枠を `Timing → State → Actuation → KeyInput` の順に再配分し、まだ入る
     item を `seq` 降順で追加採用。
   - 区切り `,`・両端 `[]`・後述の切り詰めヘッダ分をあらかじめ予算から引く。
3. 採用 envelope を `seq` 昇順に並べ、**再直列化せず** 1 で得た文字列を `,` で
   連結して `[` `]` で括る。
4. 先頭に合成 envelope を 1 件置く: `JournalEntry::DumpTruncated { budget_bytes,
   total_entries, emitted_entries, dropped_state, dropped_timing, dropped_actuation,
   dropped_key_input }`。`seq` は「採用された最小 seq」を入れる（配列は既に昇順に
   組み立て済みなので再ソートは不要）。切り詰めが起きていない場合は付けない。

**第 2 層（プロセス境界の二次防衛）: `bug_report.rs` の切り詰めを末尾優先・JSON 妥当に**

添付は別プロセス（awase-settings）が `--journal <path>` で受け取ったファイルを
文字列として読む経路であり、tray 側が古いビルドだった場合や `--journal` に任意の
ファイルを渡された場合に第 1 層が効かない。`truncate_utf8_bytes` を廃し、
`truncate_journal_json_tail(input: &str, max_bytes: usize) -> String` に置き換える:

- (a) `input.len() <= max_bytes` → そのまま。
- (b) `serde_json::from_str::<Vec<serde_json::Value>>` に成功 → 末尾から予算内に
  収まる要素を採り、compact 再直列化。
- (c) パース失敗（破損・別形式）→ pretty 配列を前提としたテキストフォールバック:
  予算の切れ目より後ろで最初に現れる `"\n  {"`（インデント 2 = トップレベル要素の
  開始。JSON 文字列リテラル内に生の改行は現れないため誤検出しない）から末尾までを
  採り、先頭に `[` を補う。末尾が `]` で終わっていなければ `]` を補う。

**第 3 層: 呼び出し側の配線**

`runtime/message_handlers.rs::handle_wm_command` の `TrayCommand::BugReport` 分岐で、
`dump_to_file()` → `dump_to_file_capped(crate::bug_report::LOG_EXCERPT_MAX_BYTES)` に
変更する。`handle_wm_dump_journal`（Alt トリガの手動ダンプ）は無制限 pretty のまま。

### (2) 変更対象

| ファイル | 変更 |
|---|---|
| `crates/awase-windows/src/journal_policy.rs`（新規） | `LaneKind` / `BudgetItem` / `select_tail_within_budget` / 予備枠比率の定数 |
| `crates/awase-windows/src/journal.rs` | `JournalLaneKind` を `journal_policy::LaneKind` に置換（re-export）、`to_json_capped` / `dump_to_file_capped` / `CappedJson` 追加、`JournalEntry::DumpTruncated` variant 追加（`lane_kind()` は `State`） |
| `crates/awase-windows/src/bug_report.rs` | `truncate_utf8_bytes` → `truncate_journal_json_tail`、`build_payload` の呼び出し差し替え |
| `crates/awase-windows/src/runtime/message_handlers.rs` | BugReport 分岐で `dump_to_file_capped` を使う |
| `crates/awase-windows/src/lib.rs` | `pub mod journal_policy;`（ungated 側に追加） |

### (3) 検討した代替案と不採用理由

- **案 A: `input[..end]` を `input[end..]` に変えるだけ。**
  最小の変更で「直近が残る」は満たすが、配列の途中から始まるので JSON として不正。
  サーバ側で構造化して読めず、pretty のままなので採用件数も少ない。不採用。
- **案 B: 第 2 層（bug_report 側の JSON パース）だけで済ませる。**
  妥当な JSON にはなるが、(i) settings プロセスが数 MB のログを読んで丸ごと parse する
  二度手間、(ii) `entry.type` からレーンを推定し直す必要があり lane 判定が 2 箇所に
  分裂する（`lane_kind()` が SSOT でなくなる）、(iii) tray 側では「全部書いてから
  半分捨てる」無駄が残る。第 2 層は**フォールバックとしてのみ**残す。
- **案 C: byte 予算でなく件数予算（レーンごとに N 件）にする。**
  entry のサイズ差が大きく（`ConvClassifyCall`/`ImeActuation` は長い、`TimerFired` は
  短い）、256KiB の上限を保証できない。不採用。
- **案 D: gzip + base64 で添付する。**
  同じ 256KiB でおよそ 10 倍の entry が入るが、ADR-095 の「送信前にユーザーが
  プレビューで中身を確認できる」UI 要件（`preview_json`）と `log_excerpt: String` の
  サーバ側スキーマに反する。将来の拡張候補として保留。

### (4) 既存テスト・既存型への影響

- `bug_report.rs` の `log_is_attached_only_when_requested_and_truncated_by_utf8_boundary`
  は長さと char boundary しか見ていないので新実装でも通るが、**「末尾（新しい方）が
  残ること」「結果が `serde_json` で配列としてパースできること」を検証する新テストを
  必ず追加する**（[fix-requires-evidence ルール](../../.claude/rules/fix-requires-evidence.md)
  の (a) 回帰テスト）。`journal_policy` 側は Linux で回る純粋テスト。
- `architecture_guard.rs` に「`bug_report.rs` に先頭切り出し（`[..` によるスライス）が
  再導入されていないこと」の軽量ガードを 1 件足すことを推奨（B-1 は「良さそうに見える
  1 行」で簡単に再発する）。
- `journal_replay` / `drift_correction_replay` は `ConvClassifyCall` / `ActuationRecord`
  の中身だけを扱い直列化経路に依存しないので影響なし。
- `architecture_guard.rs` 1123 行付近のコメント「`.journal` の全呼び出しが `.record(..)`
  か `.dump_to_file()` のみ」は事実として古くなる（`dump_to_file_capped`・後述の
  `absorb`/`stamper` が増える）。ガード本体（`observations.record(` の検査）に影響は
  ないが、コメントの更新が必要。
- `ImeEvent` 等の state 層の型には一切触れない。

---

## B-2: `FocusTransition.from` / `ImeEvent::FocusChanged.from` を実値にする

### (1) 設計の概要

**遷移前情報の採取点は `apply_focus_probe_result` の先頭**（`classify_focus_probe` を
呼ぶ前）。ここが肝で、「`update_focus_info()` の前」では**遅い**:
`classify_focus_probe` が `platform_state.focus.app_kind` と `.focus_kind` を先に
破壊的更新するため、`advance_focus_tracking` の時点では AppKind/FocusKind の遷移前値が
既に失われている。

新しい値の型（`focus/current.rs`）:

```rust
/// journal 用に「どのウィンドウに居たか」を 1 個の値として固める。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FocusIdentity {
    pub hwnd: usize,          // 0 = 未取得
    pub pid: u32,
    pub class_name: String,
    pub process_name: String,
    pub app_profile: AppImeProfile,
    pub app_kind: AppKind,
    pub focus_kind: FocusKind,
}

/// どの軸が変わったか（B-3 の記録条件かつ journal のペイロード）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct FocusChangedAxes {
    pub process: bool,
    pub window: bool,      // hwnd が変わった（同一プロセス内のウィンドウ移動を含む）
    pub app_kind: bool,
    pub focus_kind: bool,
}

impl FocusIdentity {
    #[must_use] pub fn changed_axes(&self, next: &Self) -> FocusChangedAxes;
}
impl FocusChangedAxes { #[must_use] pub const fn any(self) -> bool; }
```

`hwnd` の保持先: 現状 `CurrentFocus` は hwnd を持たない（`ClassifiedFocus.hwnd` は
その場限り）。`ImeEvent::FocusChanged.from: Option<HwndId>` を埋めるには**前回の
hwnd を誰かが保持している必要がある**ので、`CurrentFocus` に `hwnd: usize` を足す。

- `CurrentFocus::update(&mut self, pid: u32, class_name: String, hwnd: usize)`
- `FocusTracker::update(&mut self, pid, class_name, hwnd)`
- `WindowsPlatform::update_focus_info(&mut self, process_id, class_name, hwnd)`
  （crate 内の呼び出しは `advance_focus_tracking` の 1 箇所のみ・確認済み）

`Runtime` 側（`runtime/focus_tracking.rs`）にスナップショッタを足す:

```rust
impl Runtime {
    /// `platform.focus.current` と `platform_state.focus.{app_kind, focus_kind}` を
    /// 合成して現在の FocusIdentity を返す（読み取りのみ）。
    fn focus_identity_snapshot(&self) -> FocusIdentity;
}
```

`apply_focus_probe_result` の順序（★が変更点）:

```
★ let prev = self.focus_identity_snapshot();
★ let prev_started_ms = self.platform_state.focus.last_focus_transition_ms;  // B-3 で新設
   let classified = self.classify_focus_probe(probe)?;      // app_kind/focus_kind を更新する
   let (process_changed, prev_pid) = self.advance_focus_tracking(&classified);  // update_focus_info
★ let next = self.focus_identity_snapshot();
★ self.record_focus_transition_if_changed(&prev, &next, prev_started_ms);   // 単一の記録点
   { injection_mode を push }
   if process_changed { self.on_focus_process_changed(&classified, prev_pid, &prev); } else { ... }
```

`FocusTransition` の記録を `on_focus_process_changed` から**引き上げる**理由は 2 つ:
B-3（プロセス変更以外でも記録する）と、`ImeEvent::FocusChanged` の dispatch より
**前**に遷移が journal に載ること（journal を上から読んだとき「遷移 → belief の反応」の
順に読める）。

`ImeEvent::FocusChanged { from: .. }` は**型を変えずに実値を入れられる**
（フィールドは既に `Option<HwndId>` で存在し、現状 `None` を渡しているだけ）:
`from: (prev.hwnd != 0).then(|| HwndId(prev.hwnd))`。reducer 側
（`state/ime_model.rs` の `ImeEvent::FocusChanged { profile, to, focus_epoch, .. }`）は
`from` を読んでいないので belief 挙動は変わらない。

### (2) 変更対象

| ファイル | 変更 |
|---|---|
| `focus/current.rs` | `CurrentFocus.hwnd` 追加、`update()` に hwnd 引数、`FocusIdentity` / `FocusChangedAxes` 定義（AppKind/FocusKind を含むためこの位置が自然） |
| `focus/tracker.rs` | `FocusTracker::update()` の署名変更（委譲のみ） |
| `platform.rs` | `update_focus_info()` の署名変更（委譲のみ） |
| `runtime/focus_tracking.rs` | `focus_identity_snapshot()` 追加、`apply_focus_probe_result` の順序変更、`on_focus_process_changed` から `journal.record(FocusTransition..)` を撤去、`ImeEvent::FocusChanged.from` に実値 |
| `journal.rs` | `FocusTransition` のペイロード刷新（B-3 と共通、下記） |

### (3) 検討した代替案と不採用理由

- **`FocusTracker` に `previous: CurrentFocus` を持たせる。**
  `CurrentFocus` は AppKind/FocusKind を持たない（それらは `platform_state.focus`）ので、
  prev を 1 箇所に集約できず結局 2 箇所から拾うことになる。加えて「いつの時点の値か」が
  暗黙になり、`classify_focus_probe` が先に更新する現在の落とし穴を再生産する。
  呼び出し側で明示的にスナップショットを取るほうが安全。不採用。
  （ただし `hwnd` だけは他に保持者が居ないので `CurrentFocus` に持たせる。）
- **`ClassifiedFocus` に prev を詰める。**
  `ClassifiedFocus` は「今回分類した結果」の型で、前回値を載せると意味が濁る。不採用。
- **UIA / WinEvent フックで真のフォーカス遷移を取る。**
  ポーリング由来のスプリアス問題（過去の FocusChange スプリアス調査）を根治する別軸の
  大工事。今回の診断目的にはポーリング結果の edge 検出で十分。不採用。

### (4) 既存テスト・既存型への影響

- `state::ime_event::ImeEvent` は**変更なし**（ADR-096 の決定 3 を維持）。
- `ime_model.rs` のテストヘルパ `focus_changed_event` は自前で `from: None` を作るので
  影響なし。`from` を読む本番コードは無い（grep 済み）。
- `update_focus_info` は `pub` だが crate 外の呼び出しは無い（`crates/` 全体で 1 箇所）。
- `architecture_guard` / `layer_boundary_guard` に `update_focus_info` を数える固定件数
  テストは無い。
- 回帰テストは `FocusIdentity::changed_axes` の純粋テスト（`focus/current.rs` は
  ungated なので Linux で回る）で担保する。

---

## B-3: プロセス変更以外の遷移（ウィンドウ / AppKind / FocusKind）も記録する

### (1) 設計の概要

B-2 で作った `prev`/`next` の差分（`FocusChangedAxes`）が **1 つでも立っていれば記録**
する。edge-triggered なので、ポーリングで同じウィンドウを見続けている間は 1 件も出ない。

```rust
impl Runtime {
    fn record_focus_transition_if_changed(
        &mut self,
        prev: &FocusIdentity,
        next: &FocusIdentity,
        prev_started_ms: u64,
    );
}
```

- `next.hwnd == 0`（未確立）なら記録しない。
- `changed.any()` が false なら記録しない。
- 記録したら `platform_state.focus.last_focus_transition_ms = current_tick_ms()` を更新。

`JournalEntry::FocusTransition` を次の形に刷新する:

```rust
FocusTransition {
    changed: crate::focus::current::FocusChangedAxes,
    from: Option<FocusEndpoint>,
    to: FocusEndpoint,
    /// 直前のフォーカス（＝from）に留まっていた時間。往復パターン検出の主キー。
    dwell_ms: u64,
    profile: String,   // 既存互換: ImePolicyProfile の Debug 表現
}

pub struct FocusEndpoint {   // journal.rs、Serialize のみ
    pub hwnd: crate::state::ime_event::HwndId,
    pub pid: u32,
    pub process_name: String,
    pub class_name: String,
    pub app_kind: String,    // format!("{:?}")。AppKind に Serialize を足さない
    pub focus_kind: String,  // 同上
}
```

**`dwell_ms` に `last_focus_change_ms` を流用しないこと。**
`platform_state.focus.last_focus_change_ms` は「プロセス変更時のみ更新」される値で、
`tuning::MIN_FOCUS_DURATION_MS` によるキャッシュ保存判定（`advance_focus_tracking`）が
その意味に依存している。診断用に更新頻度を変えると挙動が変わる。**journal 専用の
`last_focus_transition_ms: u64` を `platform_state.focus`（`FocusState`）に新設**し、
`record_focus_transition_if_changed` だけが書く。

これで、レビューが挙げた「90 秒間に同じ 2 ウィンドウを 4 回以上往復」は
`FocusTransition` の `from.hwnd`/`to.hwnd` ペアと `dwell_ms` の並びだけで
オフライン検出できるようになる（Chrome 内 InputSite⇔本体、Edge の Uwp⇔TsfNative も
`changed.window` / `changed.app_kind` として同じ 1 本の時系列に載る）。

想定発火頻度: フォーカス遷移は人間操作の速度で律速され、通知ポップアップの
瞬間フォーカスを含めても毎秒数件が上限。`state` レーン（容量 1024、`ImeEvent` と
共有）で吸収できる。もし実機で `state` レーンを圧迫することが判明したら、
`FocusTransition` を `timing` レーンに移すのではなく `state` レーンの容量調整で
対応する（レーン分類の意味を崩さない）。

### (2) 変更対象

| ファイル | 変更 |
|---|---|
| `runtime/focus_tracking.rs` | `record_focus_transition_if_changed` 新設（唯一の記録点） |
| `state/platform_state.rs` | `FocusState.last_focus_transition_ms` 追加（journal 専用と doc に明記） |
| `journal.rs` | `FocusTransition` のペイロード刷新、`FocusEndpoint` 追加 |
| `focus/current.rs` | `FocusChangedAxes`（B-2 と共通） |

### (3) 検討した代替案と不採用理由

- **新しい軽量 variant（`FocusKindChanged` / `WindowChanged`）を別に足す。**
  読む側が複数 variant を突き合わせないと往復が追えず、「同じ 2 ウィンドウの往復」の
  クエリが複雑になる。既存 `FocusTransition` に `changed` 軸を持たせて 1 本の時系列に
  するほうが、過去バグの決め手（往復パターン）に直接答えられる。不採用。
- **記録を `classify_focus_probe` の中（kind を書き換える直前）で行う。**
  記録点が 2 箇所（kind 変化はここ、プロセス変化は別）に分裂し、1 回のプローブで
  2 件出ることがある。単一記録点にしたほうが「1 プローブ = 最大 1 件」になり読みやすい。
  不採用。
- **同一ペアの往復が続くときレート制限する。**
  往復そのものが探している証拠なので、間引くと目的を壊す。edge-triggered の時点で
  ポーリング分は既に落ちている。不採用。

### (4) 既存テスト・既存型への影響

- `FocusTransition` の JSON 形は変わるが、読み手は人間とサーバ保管のみ
  （`journal_replay` は `ConvClassifyCall` しか読まない）。
- `ImeEvent` / reducer / `focus_epoch` の扱いは一切変えない（`ImeEvent::FocusChanged` の
  dispatch 条件は今までどおり **process_changed のときだけ**）。ここを一緒に変えると
  belief 側の回帰リスクが跳ね上がるので、**絶対に混ぜない**。
- 純粋テスト: `changed_axes` の全 16 パターンと、`record_focus_transition_if_changed` の
  発火条件（無変化 → 0 件）を `focus/current.rs` 側の純粋テストで担保する。

---

## B-4: 保留キューの entry に「発生時刻」と「発生順の seq」を持たせる

### (1) 設計の概要

X-2 の `JournalStamper` を入れて、**`seq` と `elapsed_ms` を push（発生）時に確定**する。
drain がいつ起きても、`to_json()` が `seq` でマージするので順序は発生順のまま。

```rust
// journal.rs
#[derive(Clone)]
pub struct JournalStamper {
    clock: quanta::Clock,
    start: quanta::Instant,
    next_seq: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl JournalStamper {
    #[must_use]
    pub fn stamp(&self, entry: JournalEntry) -> JournalEnvelope;   // seq を fetch_add(Relaxed)
}

impl UnifiedJournal {
    /// 採番・時刻採取だけができるハンドルを配る（バッファ・dump へはアクセスできない）。
    #[must_use] pub fn stamper(&self) -> JournalStamper;

    /// 既に stamp 済みの envelope をレーンへ収める（採番・時刻採取はしない）。
    pub fn absorb(&mut self, envelope: JournalEnvelope);

    /// 既存 API は維持（内部で stamp + absorb）。
    pub fn record(&mut self, entry: JournalEntry) -> u64;
}
```

- `UnifiedJournal.next_seq: u64` → `Arc<AtomicU64>`（`JournalStamper` と共有）。
  `Rc<Cell<u64>>` でも足りるが、`WindowsPlatform` に `!Send` を持ち込まないよう
  `Arc<AtomicU64>` を推奨（コストは無視できる）。
- `WindowsPlatform`:
  - `pending_journal_entries: Vec<JournalEntry>` → `Vec<JournalEnvelope>`
  - フィールド `stamper: JournalStamper` を追加（`WindowsPlatform::new` の引数に追加、
    `app/bootstrap.rs` が `platform_state.ime.journal.stamper()` を渡す）
  - `push_journal_entry(entry)` は `self.pending.push(self.stamper.stamp(entry))`
  - `drain_journal_entries() -> Vec<JournalEnvelope>`
  - **上限を設ける**: pending が 4096 を超えたら最古を捨てる（drain 漏れ経路が将来
    増えても無限に伸びないため）。捨てた件数はカウントし、次に記録する entry の
    近くで `DumpTruncated` 相当として残せると理想だが、必須ではない。
- 呼び出し元 7 箇所（`runtime/message_handlers.rs` × 5、`runtime/ime_refresh.rs`、
  `runtime/focus_tracking.rs`）は `journal.record(entry)` → `journal.absorb(envelope)` の
  機械的置換。
- **レーンへの挿入は `seq` 順を保つ**（遅れて drain された envelope が新しい entry の
  後ろに積まれると、容量超過時に新しいほうが先に捨てられる）:

```rust
fn push(&mut self, envelope: JournalEnvelope) {
    if self.buffer.len() == self.capacity {
        // 遅れて来た「そもそも最古」の envelope は捨てる（新しいものを守る）
        if self.buffer.front().is_some_and(|f| envelope.seq < f.seq) { return; }
        self.buffer.pop_front();
    }
    let pos = self.buffer.iter().rposition(|e| e.seq < envelope.seq).map_or(0, |i| i + 1);
    self.buffer.insert(pos, envelope);
}
```

通常は末尾追記のままで O(1)、遅延 drain の分だけ数個後方へスキャンする。

**追加の保険（順序とは別問題）**: `runtime/key_pipeline.rs` の `kp_run_inner` 末尾
（`kp_stage_execute` の後）に drain を 1 回入れる。順序は seq で保証済みなので機能上は
任意だが、「打鍵直後に dump を押した」ときに pending が journal 本体へ入っていること
（＝取りこぼしゼロ）を保証できる。**drain 呼び出しを増やすこと自体は根治ではない**
（それが B-4 の指摘）ので、seq 採番の修正を入れずにこれだけをやってはいけない。

**時間軸**については X-3 のとおり統一しない。`ClockAnchor` を 2 点入れることで
`elapsed_ms` / `tick_ms` / `timestamp_us` を相互変換可能にし、`TsfProbeCompleted.elapsed_ms`
（tick 差分）のような**期間**はそのまま読める、という状態を作る。

### (2) 変更対象

| ファイル | 変更 |
|---|---|
| `journal.rs` | `JournalStamper` / `stamper()` / `absorb()` 追加、`next_seq` を `Arc<AtomicU64>` 化、`JournalLane::push` を seq 順挿入に、`JournalEntry::ClockAnchor` 追加（レーンは `State`） |
| `platform.rs` | `pending_journal_entries` の型変更、`stamper` フィールドと `new()` 引数、`push_journal_entry` / `drain_journal_entries` |
| `app/bootstrap.rs` | `WindowsPlatform::new` へ `stamper()` を渡す、起動時 `ClockAnchor` を 1 件 record |
| `runtime/message_handlers.rs` / `ime_refresh.rs` / `focus_tracking.rs` | `record(entry)` → `absorb(envelope)`、dump 直前に `ClockAnchor` を record |
| `runtime/key_pipeline.rs` | （保険）`kp_run_inner` 末尾で drain |

### (3) 検討した代替案と不採用理由

- **`WindowsPlatform` に `&mut UnifiedJournal` を渡す。**
  `Runtime` の別フィールドなので借用自体は通るが、`platform` のメソッドは
  `with_app` クロージャ・`output`・`focus` 経由など多数の入口から呼ばれ、すべてに
  journal 引数を通す必要がある。加えて platform → state 層への依存が生まれ
  [layer-boundaries](../layer-boundaries.md) を崩す。不採用。
- **journal を `WindowsPlatform` へ移す。**
  state 層（`platform_state.ime`）からの `record` が platform 経由になり依存が逆転する。
  ADR-082 で journal を `PlatformState` に置いた前提が壊れる。不採用。
- **pending に `tick_ms` だけ持たせ、seq は drain 時に採る。**
  同一 drain バッチ内の相対順序は救えるが、`KeyInput` 等 journal に直接 record される
  entry との相対順序は壊れたまま。B-4 の本質（因果の逆転）が直らない。不採用。
- **`thread_local!` なグローバル journal にして platform から直接 record。**
  「journal は `PlatformState` が所有し、テストで `quanta::Clock` を注入できる」構造を
  壊す。不採用。
- **時間軸を `GetTickCount64` に一本化する。**
  15.6ms 分解能ではバースト内の順序が読めず、journal の単体テストからモック時計が
  失われる。X-3 のとおり不採用。

### (4) 既存テスト・既存型への影響

- `UnifiedJournal::record` の外部シグネチャは不変なので、state 層・runtime 層の
  既存呼び出し（10 箇所以上）は無変更。
- 既存単体テスト `journal_record_increments_seq` / `journal_elapsed_ms_advances_with_clock` /
  `journal_lane_capacity_drops_oldest_per_lane` / `journal_to_json_merges_lanes_by_seq` は
  そのまま通るはず（採番の実装だけが変わる）。`Debug` impl の `next_seq` は
  `.load(Relaxed)` に変更。
- **追加すべきテスト**: 「後から absorb された小さい seq の envelope が、既に入っている
  大きい seq の後ろに来ないこと」「容量満杯時に遅延 envelope が新しい entry を追い出さない
  こと」。journal.rs は `#[cfg(windows)]` なので Linux CI では回らない点に注意
  （純粋な挿入判定を `journal_policy.rs` に切り出せば Linux で守れる。推奨）。
- `architecture_guard` の該当コメント（`.journal` の呼び出しは record/dump のみ）を更新。

---

## B-5: 無変化 probe tick の `TsfProbeTick` を捨てる

### (1) 設計の概要

`platform.rs::advance_tsf_probe` は毎 tick（10ms）無条件で
`GjiFsmTransition { trigger: "TsfProbeTick" }` を push しており、300ms の probe で
約 30 件、うち大半が `state_before == state_after` の無情報 entry。timing レーン
（容量 512）がこれで埋まり、`FocusChange` / `ImeOn` / `LongIdleTimeout` /
`CompositionReset` を押し出す。

判定は `journal_policy.rs` の純粋関数に置く:

```rust
#[derive(Debug, Clone, Copy, Default)]
pub struct ProbeTickFacts {
    pub state_changed: bool,        // gji_state_label が変わった
    pub needs_composition_reset: bool,
    pub has_gji_response: bool,
    pub learned_tsf: bool,
    pub completed: bool,            // completed_cold_seq.is_some()
    pub terminal_timer: bool,       // TimerCommand::Kill（probe 終了）
    pub is_first_tick: bool,
}

#[must_use]
pub const fn probe_tick_is_notable(f: ProbeTickFacts) -> bool;   // いずれか true なら記録
```

`advance_tsf_probe` の流れ:

1. `state_before` を採る → `step_probe()` → `state_after` を採る。
2. `ProbeTickFacts` を組み立てて `probe_tick_is_notable` に問う。
3. 記録する場合の `trigger` は `format!("TsfProbeTick(#{tick_index}, skipped={suppressed})")`
   のように**「この記録の前に何 tick 捨てたか」を含める**。捨てたこと自体が
   journal から分かる状態にする（黙って消さない）。
4. 記録しなかった場合は `suppressed_probe_ticks += 1`。
5. `probe_tick_index` / `suppressed_probe_ticks` は `install_pending_tsf_and_set_timer` と
   `GjiAction::StartProbe` で 0 にリセットする（`active_tsf_probe_started_ms` と同じ場所）。
6. `TsfProbeCompleted` に `tick_count: u32` を追加し、「何 tick 回ったか」（＝probe の
   実所要 tick 数）を失わないようにする。既存の `elapsed_ms`（tick 差分）と併せて、
   probe が「速く終わった / タイムアウトまで粘った」が読める。

期待効果: 300ms probe で 30 件 → 3〜5 件（開始・状態遷移・完了）。timing レーンの
実効寿命が 6〜10 倍になる。

**副次的コスト（任意）**: `WindowsPlatform::gji_state_label()` は
`ImeWarmupStrategy::diagnostic_state_label() -> String` 由来で、毎 tick 2 回ヒープ確保が
残る（`GjiFsm::state_label()` 自体は `&'static str`）。`Cow<'static, str>` 化すれば
tick あたりの確保をゼロにできる。必須ではないが、この修正のついでが最も安い。

### (2) 変更対象

| ファイル | 変更 |
|---|---|
| `journal_policy.rs`（新規） | `ProbeTickFacts` / `probe_tick_is_notable` |
| `platform.rs` | `advance_tsf_probe` の記録条件、`probe_tick_index` / `suppressed_probe_ticks` フィールド、`note_tsf_probe_completed` に `tick_count` |
| `journal.rs` | `TsfProbeCompleted` に `tick_count: u32` 追加 |
| （任意）`tsf/warmup/warmup_strategy.rs`・`output/mod.rs`・`output/tsf_warmup_coord.rs` | `diagnostic_state_label` を `Cow<'static, str>` に |

### (3) 検討した代替案と不採用理由

- **timing レーンの容量を 512 → 2048 に増やす。**
  無変化 tick が容量を食う構造は変わらず、長いセッションでは結局押し出す。
  [tuning-constants ルール](../../.claude/rules/tuning-constants.md) が警告する
  「同じ役割の定数の盲目的エスカレーション」そのもの。不採用。
- **probe tick 専用のサブレーンを作る。**
  レーンが増えて B-1 の予算配分が複雑になる。無変化 tick は**そもそも情報がない**
  （同じラベルの反復）ので、隔離ではなく破棄が正しい。不採用。
- **N tick に 1 回のサンプリング。**
  状態が変わった瞬間を取りこぼしうる。edge 記録 + 抑制件数のほうが情報量が多く、
  かつ件数が少ない。不採用。
- **`step_probe` の中（`Output` 層）で判定する。**
  `Output` は journal を知らない設計（ADR-096 が `platform.rs` を橋渡し点にした）。
  判定材料（`state_before`/`state_after`）は platform 側にしかない。不採用。

### (4) 既存テスト・既存型への影響

- `StepProbeResult` は変更なし（既存フィールドだけで `ProbeTickFacts` を作れる）。
- `TsfProbeCompleted` にフィールド追加（JSON 追加のみ、読み手は人間）。
- `probe_tick_is_notable` の純粋テスト（各 fact 単独で true になること、全 false で
  false になること）を Linux CI で回す。
- `advance_tsf_probe` 自体は Windows 実機でしか動かないため、実機確認項目として
  「300ms の GJI cold probe 1 回で timing レーンに積まれる `TsfProbeTick` が 5 件以下」
  を残す。

---

## 実装順序の推奨

依存関係:

- **B-1 と B-5 は `journal_policy.rs`（LaneKind の移設）を共有する。**
- **B-1 は「`seq` が発生順である」ことを前提に「直近から遡って」を定義する** →
  厳密には B-4 に依存するが、B-4 なしでも「drain 時刻順で直近」を残すので実害は小さい。
- **B-3 は B-2（`FocusIdentity`）の上に乗るだけ**なので必ず B-2 の後。
- B-5 は完全に独立。

推奨順序:

| 順 | 項目 | 理由 |
|---|---|---|
| 0 | `journal_policy.rs` 新設（`JournalLaneKind` → `LaneKind` 移設のみ、挙動不変） | B-1/B-5 の受け皿。挙動不変なので単独でマージでき、レビューも軽い |
| 1 | **B-1** | must-fix。現状「報告ボタンの直前が丸ごと消える」＝ADR-095 の機能が成立していない。他の 4 件をいくら直しても、届かなければ無意味 |
| 2 | **B-5** | timing レーンの汚染を止める。B-1 の予算配分（Timing 35%）が実際に何件を運べるかは、無変化 tick を捨てた後でないと実測できない |
| 3 | **B-4** | 順序・時刻の正しさ。entry 数が減った後（B-5 後）のほうが実機ログで順序の検証がしやすい。`ClockAnchor`（X-3）もここで入れる |
| 4 | **B-2** | `CurrentFocus.hwnd` 追加と署名変更を含むため、journal 側の変更が落ち着いてから |
| 5 | **B-3** | B-2 の `FocusIdentity` に乗るだけ。ここまでで往復パターンが検出可能になる |

各段でのテスト方針（[fix-requires-evidence](../../.claude/rules/fix-requires-evidence.md) 準拠）:

- 0〜3: `journal_policy.rs` の純粋テスト（Linux CI で回る）＋ `bug_report.rs` の
  「末尾が残る」「valid JSON」テスト。B-1 は `architecture_guard` に再発防止ガードを 1 件。
- 4〜5: `focus/current.rs` の `changed_axes` 純粋テスト（Linux CI）。
- 全体: Windows 実機で「不具合を報告 → 添付 JSON の末尾に押下直前の
  `FocusTransition` / `GjiFsmTransition` が入っている」ことを 1 回確認する
  （ADR-095/096 に残っている実機未検証項目とまとめて消化する）。

ドキュメント: 完了後に ADR-096 へ「レビュー指摘と是正（B-1〜B-5）」節を追記し、
本文書へリンクする（ADR-096 の「保持するもの」に書かれた
「`bug_report.rs` は変更不要」は B-1 で覆るため、そこも訂正する）。
