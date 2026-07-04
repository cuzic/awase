# ADR-077: ObservationAdmission Layer — FocusEpoch による probe 受理ポリシー

## ステータス

採用済み（2026-07-03 実装）

## コンテキスト

### 発端: ALT+TAB ウィンドウ切替時の Engine OFF

ALT+TAB でウィンドウを切り替えると、LINE などの Qt/ImmCross アプリで
awase エンジンが Engine OFF（ローマ字直接入力に戻る）になる不具合が報告された。

**根本原因:**

ALT+TAB のタスクスイッチャー表示中、OS は選択 UI ウィンドウ
（`XamlExplorerHostIslandWindow` / `ForegroundStaging` など）に一時的に
フォーカスを移す。このタイミングで起動した `ImmCrossProbe`（非同期）が
経由ウィンドウを対象に IME 状態を読み取ると `false` を返す。

当時の `ImmCrossProbe` は `High confidence` で書き込まれるため、
`derive_open()` がこの `false` を即採用 → `effective_open() = false` →
Engine OFF カスケードが発生した。

**初期対策（Fix A/B）:**

| 修正 | 内容 |
|------|------|
| Fix B | `XamlExplorerHostIslandWindow` を `NonText`（IME リセット不要）に分類し、タスクスイッチャー選択中の `reset_to_off` を防止 |
| Fix A | `shadow_on && probe_age_ms < 200ms` という時間ベースの grace 期間で `ImmCrossProbe` の `false` を抑制 |

Fix A は機能したが、**時間ベース競合**という構造的な問題を抱えていた:

- CPU 負荷次第で 200ms を超えてしまう可能性
- 3 箇所に同じ `shadow_on && probe_age_ms < SHADOW_GRACE_MS` が複製された
- 「この観測は信用できるか」という判断がコードベースに分散した

### 問題の本質: 観測の信用度判断が分散している

awase には当時 7 種類の probe があった:

| Probe | 種別 | 信頼度 | 抑制条件（当時） |
|-------|------|--------|-----------------|
| ImmCrossProbe | 非同期 | High | `shadow_on && probe_age < 200ms` |
| FocusProbe | 同期（first-key） | Low | 同上（コピー） |
| ObserverPoll | 同期（500ms 周期） | Medium | 同上（コピー） |
| GJI | イベント駆動 | Medium | `last_io` タイムスタンプ |
| TSF Observer | イベント駆動 | Medium | 観測のみ、desired 不変 |
| HwndCache | 同期 | Low | なし |
| ImmGetOpenStatus | 同期 | High | なし |

「probe が stale かどうか」の判断がそれぞれのサイトに散らばり、
新しい probe を追加するたびに同じロジックをコピーする必要があった。

## 決定

### Phase 1: FocusEpoch + ImmLikeTicket による epoch 照合

時間ベースの grace を廃止し、**フォーカス変更カウンタ（FocusEpoch）** を導入する。

```rust
// FocusStore
pub focus_epoch: FocusEpoch,  // u64, on_focus_process_changed で wrapping_add(1)
```

非同期 probe は spawn 時にエポックをキャプチャし、完了時に照合する:

```rust
// ImmLikeTicket — spawn 時にキャプチャ
let ticket = ImmLikeTicket { focus_epoch: current_focus_epoch };

// with_app 内（async 完了後）
let current = app.platform_state.focus.focus_epoch;
let Admission::Accept(accepted) = ticket.admit(current) else {
    // フォーカスが変わっていれば棄却
    return;
};
app.platform_state.ime.write_imm_cross_probe(open, tick_ms, accepted);
```

**時間ベースとの違い:**

| | 旧 shadow grace | 新 epoch 照合 |
|---|---|---|
| 判定基準 | `probe_age < 200ms`（近似） | フォーカスが「同じ」か（正確） |
| CPU 負荷時 | 200ms 超で素通りのリスク | 時間に無関係 |
| 重複コード | 3 箇所にコピー | `ImmLikeTicket::admit()` 1 箇所 |
| 診断 | なし | `REJECTED_EPOCH_MISMATCH` 原子カウンタ |

### Phase 2: AcceptedObservation 型保証

`ImmLikeTicket::admit()` の戻り値を `AcceptedObservation` トークンにし、
`write_*` 関数がこれを受け取ることで **admission を通らない write** をコンパイル時に防ぐ:

```rust
pub struct AcceptedObservation {
    pub focus_epoch: FocusEpoch,
    _private: (),             // 外部からの直接構築を禁止
}

pub enum Admission {
    Accept(AcceptedObservation),
    Reject(RejectReason),
}

// 同期 probe 専用コンストラクタ（シングルスレッドなので epoch mismatch 不可）
impl AcceptedObservation {
    pub fn for_sync(focus_epoch: FocusEpoch) -> Self { ... }
}
```

`write_imm_cross_probe(open, tick, accepted: AcceptedObservation)` のように
シグネチャを変更することで、将来 probe を追加した実装者が
「`AcceptedObservation` が必要 = admission を通らなければならない」と
自然に気づく構造になった。

### Phase 3: derive_open() epoch フィルタ

`ObservationStore` に `current_focus_epoch` を追加し、
`FocusChanged` イベントで更新する:

```rust
// ImeEvent::FocusChanged に focus_epoch を追加
FocusChanged { .., focus_epoch: FocusEpoch }

// reducer で:
self.observations.clear_on_focus_change(focus_epoch);
// → ObservationStore::current_focus_epoch を更新
```

`derive_open()` は `ImmCrossProbe` / `FocusProbe`（スナップショット系 probe）を
`current_focus_epoch` でフィルタする:

```rust
let is_epoch_ok = |o: &ImeObservation| match o.source {
    ObservationSource::ImmCrossProbe | ObservationSource::FocusProbe => {
        o.focus_epoch == current_epoch
    }
    _ => true,  // GJI / ObserverPoll / TSF はイベント駆動のため除外
};
```

**GJI / ObserverPoll / TSF を epoch フィルタ対象外にした理由（B 案選択）:**

| Probe | フィルタ対象外の理由 |
|-------|---------------------|
| GJI | イベント駆動の変化通知。「いつ起きた変化か」でなく「変化があった」が重要。`last_io` タイムスタンプで鮮度管理済み |
| ObserverPoll | 500ms 周期の同期ポーリング。シングルスレッドなので epoch mismatch 不可 |
| TSF Observer | 観測のみ（`desired_open` を書き換えない）。epoch フィルタの利点なし |

## 実装ファイル一覧

| ファイル | 変更内容 |
|---------|---------|
| `state/probe_admission.rs` | `AcceptedObservation` 追加、`Admission::Accept(accepted)` 化 |
| `state/ime_event.rs` | `FocusChanged { focus_epoch }` 追加 |
| `state/observation_store.rs` | `current_focus_epoch` フィールド、`clear_on_focus_change(new_epoch)`、`derive_open()` epoch フィルタ |
| `state/ime_model.rs` | reducer: `FocusChanged` で `current_focus_epoch` 更新 |
| `state/platform_state.rs` | `write_observer_poll / write_focus_probe / write_imm_cross_probe / apply_ime_update` シグネチャを `AcceptedObservation` に変更 |
| `runtime/focus_tracking.rs` | `FocusChanged` dispatch に `focus_epoch` 追加、ImmCrossProbe: `Accept(accepted)` パターン |
| `runtime/key_pipeline.rs` | `kp_stage_focus_probe`: `ImmLikeTicket` 化、`apply_focus_probe`: `accepted` 受け取り、epoch チェック撤去 |
| `runtime/ime_refresh.rs` | `AcceptedObservation::for_sync()` に変更 |
| `runtime/mod.rs` | `AcceptedObservation::for_sync()` に変更 |
| `tuning.rs` | `SHADOW_GRACE_MS` 撤去 |

## 設計の完成度

以下の3層が対応している:

```
書き込み時: ImmLikeTicket::admit() → AcceptedObservation  [型保証]
ストア時:   ImeObservation.focus_epoch に記録             [来歴記録]
読み出し時: derive_open() が epoch フィルタ              [防衛的排除]
```

「admission を通らない write ができた」という穴がコンパイル時に塞がれ、
`derive_open()` も stale な高信頼観測を読み飛ばすセーフガードを持つ。

## 残存する制約

- TSF / GJI / HwndCache は epoch フィルタ対象外のため、これらが stale な観測を
  書き込んだ場合は 3000ms の鮮度ウィンドウ（`FRESH`）でのみ排除される。
- `AcceptedObservation` の private フィールドにより外部から直接構築できないが、
  `for_sync()` という公開コンストラクタが存在するため、悪意あるコードは
  admission を回避できる（意図的な抜け道。同期 probe で必須）。

## 関連 ADR

- ADR-033: AppImeProfile — `ImmCross` / `XamlExplorerHostIslandWindow` の分類
- ADR-075: ImmCrossProbe による belief 補正（`derive_open()` の設計）
- ADR-076: スリープ復帰後 is_japanese_ime grace 保護（同じ grace 問題の別インスタンス）
