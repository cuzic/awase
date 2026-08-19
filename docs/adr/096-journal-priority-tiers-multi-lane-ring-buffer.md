# ADR-096: journal の優先度別 複数リングバッファ化と3つの取りこぼし解消

## ステータス

決定・実装は codex CLI に着手させる（本 ADR 執筆時点でコード変更なし）。

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

## 保持するもの（変更しないもの）

- `JournalEntry` の既存 variant（`KeyInput`/`TimerFired`/`ImeEvent`/
  `ConvClassifyCall`/`ImeActuation`/`ImeOpenApplied`/`DumpTriggered`）の
  既存フィールドは変更しない（`KeyEventSummary` への `injected` 追加のみ
  例外）。
- `docs/journal-replay-guide.md` の `ConvClassifyFixture` 抽出フローと
  `tests/journal_replay.rs`。`ConvClassifyCall` の中身は変更しないため
  影響なし。
- `crates/awase-windows/src/bug_report.rs`（ADR-095）。`dump_to_file()`
  の出力を不透明な文字列として読み込み256KiBに切り詰めて添付するだけの
  設計であり、内部レーン分割やレコード種別の追加による変更は不要。

## 既知の限界・未決定事項

- 各レーンの容量（1024/512/512/512）は過去ログの定性的な頻度評価に
  基づく初期値であり、実運用での発火頻度を見て調整が要る。
- `timing` レーンの実装は Windows 実機での検証が必須（GJI/TSF warmup の
  実際の発火パターンは Linux では再現できない）。
- `runtime/focus_tracking.rs` から `CurrentFocus.process_name` を参照する
  際、`journal.record` 呼び出し時点で process_name が最新（stale でない）
  ことの確認が必要。
- `tests/architecture_guard.rs` には `.record(` 呼び出し件数を数える
  grep ベースの固定件数テストがある。新しい記録呼び出し箇所を追加する際
  はこのテストの期待件数を更新すること（意図的な追加であることを明示
  する）。
- Windows 実機での「不具合を報告」操作からログが期待通り届くかの一連の
  実機確認は [ADR-095](095-tray-bug-report-cloudflare-intake.md) 側の
  既知の限界として引き続き残る。
