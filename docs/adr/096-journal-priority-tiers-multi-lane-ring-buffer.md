# ADR-096: journal の優先度別 複数リングバッファ化と3つの取りこぼし解消

## ステータス

実装済み（2026-08-19、codex CLI による実装、Claude が検証・マージ）。
初回実装後、Opus によるアドバーサリアルレビューで must-fix 1件・
should-fix 4件（B-1〜B-5、詳細は「round2: レビュー指摘と是正」節）が
見つかり、[docs/design/journal-diagnostic-fidelity-fixes.md](../design/journal-diagnostic-fidelity-fixes.md)
（Opus 設計）に基づき是正済み。`cargo test -p awase-windows --lib`
（407件）・`journal_replay`・`drift_correction_replay`・
`architecture_guard`（33件の固定件数テスト、B-1 再発防止ガード追加分で
32→33）すべて green。`cargo xwin build`/`check --target
x86_64-pc-windows-msvc` でリンク成功、clippy 新規警告なし。
**Windows 実機での動作確認は未実施**（[ADR-095](095-tray-bug-report-cloudflare-intake.md)
側の既知の限界と同様）。

## コンテキスト

[ADR-095](095-tray-bug-report-cloudflare-intake.md)（タスクトレイからの不具合報告機能）
は既存の `UnifiedJournal`（`crates/awase-windows/src/journal.rs`、単一の
`VecDeque` によるリングバッファ、容量2048件）をそのまま再利用する設計に
した。実装完了後、「本当にこの中身で過去のバグが診断できていたか」を
確認するため、`docs/known-bugs.md`（BUG-01〜BUG-68、全68件）と
`docs/experiments.md`（実験ログ全15件）を通読し、各バグの根本原因特定に
実際に効いた観測値の種類を8カテゴリ（打鍵イベント/IME状態遷移/フォーカス
遷移/warm-cold・タイミング/TSF固有状態/アプリ種別/actuation試行/その他）
に分類・集計した上で、現在の `journal.rs` の実装（`JournalEntry` の7
variant とその記録元）と突き合わせた。

### 見つかった3つの取りこぼし

1. **warm/cold・GJI probe のタイミング状態が一切記録されていない。**
   過去バグの診断で最も頻出した根拠カテゴリ（BUG-01〜03, 08, 17, 21, 24,
   27〜31, 33, 35, 36, 38〜40, 45, 58 など）だが、`tsf/gji_fsm.rs`
   （`ColdKind`）・TSF probe 関連コードは journal を一切参照していない。
2. **アプリ名（`process_name`）が記録されていない。**
   `state::ime_event::ImeEvent::FocusChanged` は `HwndId`（数値ハンドル）
   のみを持ち、`focus/current.rs::CurrentFocus.process_name`（Chrome/Edge/
   Teams 等の識別に必須）が journal に届いていない。全68件のバグで
   「症状の再現条件を絞り込む前提」として必須と評価された。
3. **`injected` フラグが打鍵ログから欠落している。**
   `awase::types::RawKeyEvent.injected`（`LLKHF_INJECTED`、BUG-14 対応の
   コメント付きで既に存在）を、journal 用の `KeyEventSummary::from_raw`
   がコピーしていない。BUG-08/14/52/62/67 では「合成キーの down→up 間隔
   （µs〜ms単位の実測値）」と `injected` フラグの組み合わせが外部注入を
   断定する唯一の決め手だった。

ユーザー判断: 上記3点すべてを解消する。加えて、単一の共有リングバッファ
のままだと高頻度・低診断価値のイベント（打鍵など）が低頻度・高診断価値
のイベント（warm/cold 遷移など）を押し出しうるため、**重要度別に複数の
リングバッファへ分割する**方針とした。

## 決定

### 1. 複数リングバッファ化（優先度別レーン）

`UnifiedJournal` 内部の単一 `VecDeque<JournalEnvelope>`（容量2048）を、
独立した容量を持つ複数の「レーン」に分割する。`seq` はレーンをまたいだ
単調増加カウンタを維持し（現行の `next_seq` をレーン間で共有）、
`to_json()`/`dump_to_file()` は全レーンを `seq` 順にマージして1本の
時系列配列として書き出す（**外部 JSON の形（`seq`/`elapsed_ms`/`entry`
の配列）は変更しない**）。

初期割り当て（実装時に調整可能な目安値）:

| レーン | 収容する `JournalEntry` | 容量目安 | 根拠 |
|---|---|---|---|
| `state` | `ImeEvent`, `ImeOpenApplied`, `DumpTriggered` | 1024 | 過去バグ診断で2番目に頻出、頻度は打鍵より低い |
| `timing`（新設） | GJI/TSF warm-cold・probe タイミング（後述） | 512 | 過去バグ診断で最頻出だが現状0%収録。warmup中はバースト的に発火するため十分な容量が要る |
| `actuation` | `ImeActuation`, `ConvClassifyCall`, `TimerFired` | 512 | awase 自身の能動的訂正の試行履歴。無限ループ系バグの特定に必須 |
| `key_input` | `KeyInput`（`injected` フィールド追加、後述） | 512 | 頻度は最大だが平均的な診断価値は低い。ただし `injected` 絡みでは決定的 |

合計容量は概ね2048→2560程度になる想定。値そのものより「レーンを分ける
ことで高頻度イベントが低頻度の高価値イベントを押し出さない」という
構造が目的であり、実装時に各レーンの実測発火頻度を見て調整してよい。

### 2. warm/cold・probe タイミングの記録（新規 `timing` レーン）

`tsf/gji_fsm.rs`（`GjiFsm`/`ColdKind`）や TSF probe 関連コードは
`journal`/`state` 層に依存させない（既存のレイヤー境界、
[docs/layer-boundaries.md](../layer-boundaries.md)、ADR-030/046/047 を
踏襲）。かわりに、`GjiFsm::on_event`/`on_timeout` を実際に呼び出して
結果を処理している `platform.rs`（該当箇所のコメント: 「`GjiFsm::on_event`
/ `on_timeout` の結果を処理し、タイマー操作とアクションを実行する」）
側から、呼び出し前後の状態（`ColdKind` や `Debug` 表現の FSM 状態文字列
など）を `KeyInput`/`TimerFired` が採用している `state_before`/
`state_after: String` パターンに倣って記録する。probe の開始・完了に
ミリ秒タイムスタンプを添えることが、過去バグ診断（`cold_seq` の推移、
probe起点との時間差など）で最も効いた情報である。

### 3. `process_name` の記録（`ImeEvent` 本体は変更しない）

`ImeEvent::FocusChanged` 構造体自体には手を入れない。理由: `ImeEvent`
は `dispatch_event` を介して reducer（`shadow_model.reduce`）にも渡る
コア型で、`FocusChanged` を参照するファイルは `state::ime_model` 等
16ファイルに及ぶ。ここにフィールドを追加すると journal と無関係な
reducer 側のパターンマッチにまで変更が波及する。

かわりに、`ImeEvent::FocusChanged` の**唯一の構築元**である
`runtime/focus_tracking.rs`（`ImeEvent::FocusChanged { .. }` を
`dispatch_event` に渡している箇所）から、同じ瞬間に `journal.record(...)`
を直接呼び、新設する `JournalEntry`（例: `FocusTransition { from, to,
process_name, profile }`）へ `focus/current.rs::CurrentFocus.process_name`
の値を渡す。`dispatch_event` 経由の通常の状態遷移パイプラインには一切
触れない。

### 4. `KeyEventSummary` に `injected` を追加

`KeyEventSummary::from_raw` が `event.injected`（`RawKeyEvent.injected`）
をコピーするようフィールドを1つ追加する。既存フィールド（vk_code 等）は
変更しない。

### 実装（2026-08-19、codex CLI）

- `UnifiedJournal` を `JournalLanes`（`state`/`timing`/`actuation`/
  `key_input` の4つの `JournalLane`）に分割。`JournalEntry::lane_kind()`
  が各 variant をどのレーンに振り分けるかを1箇所で決定する。`seq` は
  レーン共有の単調増加カウンタのまま。`to_json()`/`dump_to_file()` は
  全レーンを `seq` 昇順にマージし、外部 JSON の形は変更なし。
- `tsf/gji_fsm.rs`（`GjiFsm`）は `journal`/`state` 層への依存を追加せず、
  `state_label()`（状態の `&'static str` 表現）だけを公開。既存の
  `ImeWarmupStrategy` トレイト（ADR-047）に `diagnostic_state_label()`
  というデフォルトメソッドを追加し、`GjiFsm` 実装がこれを `state_label()`
  経由でオーバーライドする形にした。レイヤー境界（layer-boundaries.md）
  を侵さずに GJI 固有の状態ラベルを `platform.rs` から読めるようにする
  ための最小限の橋渡し。
- `WindowsPlatform` は `UnifiedJournal` を直接持たない（`platform_state`
  側にある）ため、`platform.rs` の GJI/TSF probe 関連メソッド
  （`dispatch_gji_event`/`advance_tsf_probe`/`install_pending_tsf_and_set_timer`
  等）は `pending_journal_entries: Vec<JournalEntry>` という保留キューに
  `GjiFsmTransition`/`TsfProbeStarted`/`TsfProbeCompleted` を積み、
  `runtime/` 側の各呼び出し元（`WM_TIMER` ハンドラ・
  `sync_ime_kind_from_observation`・`BugReport` ハンドラ・
  `WM_DRAIN_OUTPUT_QUEUE` 等）が `drain_journal_entries()` で取り出して
  `journal.record()` する設計にした。
- `runtime/focus_tracking.rs`（`ImeEvent::FocusChanged` の唯一の構築元）
  から、同タイミングで `JournalEntry::FocusTransition { process_name,
  profile, .. }` を直接 `journal.record()` する。`ImeEvent` 本体
  （16ファイルが参照するコア型）は一切変更していない。

### 検証結果

`cargo test -p awase-windows --lib`（399件）・
`cargo test -p awase-windows --test journal_replay --test
drift_correction_replay --test architecture_guard`（`architecture_guard`
の32件の固定件数テストを含む）すべて green。**`architecture_guard.rs`
の期待件数は変更不要だった**（既存 guard が journal の `.record(` を
固定件数対象から明示的に除外していたため）。`cargo xwin build --target
x86_64-pc-windows-msvc -p awase-windows` でリンク成功、`cargo clippy -p
awase-windows --lib` は新規警告なし（既存の無関係な dead_code 警告のみ）。

## round2: レビュー指摘と是正（2026-08-19）

初回実装を Opus にアドバーサリアルレビューさせたところ（「過去バグ検証に
十分な情報があるか」を含めて依頼）、must-fix 1件・should-fix 4件が
見つかった。設計は Opus に別途「あるべき設計」として検討させ、
[docs/design/journal-diagnostic-fidelity-fixes.md](../design/journal-diagnostic-fidelity-fixes.md)
にまとめた上で codex CLI に実装させた。Claude が各指摘の実在をコードで
裏取りし（`from: None` 固定・`update_focus_info` 呼び出し元1箇所・
reducer が `from` を読んでいないこと等）、設計書の主要な主張も検証済み。

- **B-1（must-fix）**: `bug_report.rs::truncate_utf8_bytes` が
  `input[..end]` で**先頭（最古）**を残し、報告ボタン押下直前（症状発生
  の瞬間）の記録を丸ごと切り捨てていた。`UnifiedJournal::to_json_capped`
  （直近 `seq` から予算内に収める capped シリアライザ、レーン別予備枠
  Timing35/State30/Actuation20/KeyInput15%）と、`bug_report.rs` 側の
  末尾優先・JSON妥当性を保証するフォールバックの二層防御に置き換えた。
- **B-2**: `FocusTransition.from`/`ImeEvent::FocusChanged.from` が常に
  `None` だった。`CurrentFocus.hwnd` 追加と、`apply_focus_probe_result`
  冒頭（`classify_focus_probe` が `app_kind`/`focus_kind` を破壊的更新
  する**前**）でのスナップショットにより実値化した。`ImeEvent` の型は
  変更していない。
- **B-3**: `FocusTransition` がプロセス変更時にしか発火せず、同一
  プロセス内でのウィンドウ/AppKind/FocusKind 往復（BUG-17/18型）を
  検出できなかった。`FocusChangedAxes` による edge-triggered 記録に
  一本化した。
- **B-4**: `WindowsPlatform` の保留キューが「発生時刻」でなく「drain
  された時刻」で `seq`/`elapsed_ms` を確定していたため、因果順が乱れ
  うる構造だった。`JournalStamper`（採番・時刻採取だけのハンドル）を
  push 時に使うことで発生順の `seq` を確定するよう変更した。
- **B-5**: `TsfProbeTick` を無条件・毎tick記録しており、timing レーン
  が無変化 tick に埋まりうる構造だった。`journal_policy::probe_tick_is_notable`
  で有意な tick のみ記録するよう変更した。

`journal_policy.rs`（新設、ungated）に純粋な判定ロジック（レーン容量・
予算配分・tick 抑制判定）を集約し、Linux CI で回帰テストできるように
した。`architecture_guard.rs` は32→33件（B-1 の再発防止ガード追加分）。
検証結果は「ステータス」節を参照。

## 保持するもの（変更しないもの）

- `state::ime_event::ImeEvent` の型そのもの（`FocusChanged` のフィールド
  構成）。`from` フィールドに実値を入れるようになったが（round2 B-2）、
  型・reducer の挙動は変更していない。
- `docs/journal-replay-guide.md` の `ConvClassifyFixture` 抽出フローと
  `tests/journal_replay.rs`。`ConvClassifyCall` の中身は変更しないため
  影響なし。
- 3系統の時間軸（`elapsed_ms`/`tick_ms`/`timestamp_us`）は統一しない
  （round2 の設計判断、既存の `tuning-constants` ルール対象への波及と
  テストのモック時計破壊を避けるため）。かわりに `ClockAnchor` entry で
  相互変換可能にした。

> **訂正**: 初版で「`crates/awase-windows/src/bug_report.rs` は変更不要」
> としていたが、round2 の B-1 是正でこの前提は誤りだったと判明した
> （`dump_to_file()` の出力を不透明な文字列として先頭切り詰めしていた
> ことが、まさに症状発生直前の記録を失う原因だった）。`bug_report.rs`
> は round2 で変更対象になっている。

## 既知の限界・未決定事項

- 各レーンの容量（1024/512/512/512）は過去ログの定性的な頻度評価に
  基づく初期値であり、実運用での発火頻度を見て調整が要る。
- `timing` レーンの実装は Windows 実機での検証が必須（GJI/TSF warmup の
  実際の発火パターンは Linux では再現できない）。
- ~~`runtime/focus_tracking.rs` から `CurrentFocus.process_name` を参照する
  際、`journal.record` 呼び出し時点で process_name が最新（stale でない）
  ことの確認が必要。~~ → round2 レビューで確認済み。`update_focus_info()`
  が先に `process_name` を再取得してから記録するため、記録時点で最新。
- Windows 実機での「不具合を報告」操作からログが期待通り届くかの一連の
  実機確認は [ADR-095](095-tray-bug-report-cloudflare-intake.md) 側の
  既知の限界として引き続き残る。
- **round2 レビュー（C. 見落とし）で挙がったが今回は対応しなかった項目**
  （別 ADR での対応候補）:
  - `class_name` の欠落（`FocusEndpoint` には含まれるが `KeyEventSummary`
    等には無い）・`RawKeyEvent.extra_info` の欠落。BUG-56/08/14 で
    `process_name` と同格以上に効いた識別子。
  - **literal-detect 判定結果（`DetectionResult`、per-VK の状態、
    `raw_tsf_literal_consecutive_count`、give-up 分岐）が0%収録**。
    round2 レビューは「ADR-096 が塞いだ3ギャップの次に大きい穴」と
    評価している（BUG-03/24/27/29/30/36/38/40/45 の9件で決め手だった）。
  - IME 種別変化（`WM_IME_KIND_CHANGED`/`set_active_ime_kind`）が
    journal に記録されない。
  - TSF固有シグナル（`gji_candidate_show`/`gji_candidate_visible_now()`
    の区別、`himc_null`）が0%収録。
  - hook 層で swallow される外部注入キー（`VK_KANA`/`VK_DBE_ROMAN`/
    `VK_DBE_NOROMAN` 等）は原理的に journal に現れない
    （BUG-08/62 型の決定的証拠は現状の設計では再現不能）。
