# ADR-046: GjiFsm — warm/cold 状態の FSM 一元管理

## ステータス

採用済み（2026-06-20〜2026-06-22 実装）

## コンテキスト

### 問題: warm/cold 状態が複数ファイルに分散していた

TSF 出力層の warm/cold 管理は長期間パッチ積み重ね型で進化してきた。

| 管理箇所 | 変数・フィールド | 問題 |
|---|---|---|
| `output/mod.rs` | `WarmthContext.warm` | 書き込まれるだけで参照なし（ADR-045 で発見・撤去）|
| `output/mod.rs` | `composition: CompositionState` | `is_composition_warm()` が判定の SSOT |
| `output/probe_io.rs` | `gji_long_idle: bool` | long idle 判定フラグ（ProbeObservations にも複製）|
| `output/probe_io.rs` | `gji_last_io_ms: u64` | I/O 最終時刻（TsfEnvSnapshot にも複製）|

warm/cold に関連するバグが 2026-06 時点で7件以上発生しており、
いずれも「複数ファイルにある状態の一部しか更新されなかった」または
「フォーカス変更時にリセット漏れがあった」が根本原因だった。

### long-idle の分類が ad hoc だった

idle 時間の閾値が `gji_long_idle` という boolean 1本で表現されており、
将来 Medium（7〜10s）と Long（10s 超）を区別する必要が出たときに
コード変更が広範囲に及ぶことが予測された。

## 決定

`crates/awase-windows/src/tsf/gji_fsm.rs` を新規作成し、
GJI の warm/cold 状態を有限状態機械として一元管理する。

### 状態定義

```
OffCold
  → (ImeOn)   → OnCold { kind: ColdKind, probe: ProbeStatus }
  
OnCold { Running }
  → (WarmupComplete) → OnWarm
  
OnWarm
  → (LongIdle timeout) → OnCold { kind: Long, probe: NotStarted }
  → (StartComposition) → OnComposing
  
OnComposing
  → (ImeOff / FocusChange) → OffCold
  → (EndComposition)       → OnWarm
```

### ColdKind — idle 時間による分類集中化

```rust
pub(crate) enum ColdKind {
    Short,   // 0〜7s: 通常の cold（ncwait_budget_ms が短い）
    Medium,  // 7〜10s: 中 idle
    Long,    // 10s+: 長期 idle（F2 prepend が必要）
}

impl ColdKind {
    pub fn classify(gji_idle_ms: u64) -> Self {
        match gji_idle_ms {
            0..7_000   => Self::Short,
            7_000..10_000 => Self::Medium,
            _          => Self::Long,
        }
    }
}
```

`ColdKind` から `ncwait_budget_ms` / `forces_prepend_f2` / `is_long_cold` を
導出することで、scattered boolean フラグを構造的に撤去した。

撤去されたフラグ:
- `gji_long_idle_probe: bool` (NameChangeWait)
- `gji_long_idle_probe_nonfired: bool` (ProbeObservations)
- `gji_long_idle: bool` (TsfEnvSnapshot・ProbeIo trait・FakeProbeIo)
- `gji_last_io_ms: u64` (TsfEnvSnapshot)

### 移行戦略: debug_assert → SSOT 切替

段階的移行 (ADR-040) に従い、3フェーズで実施した。

**Phase 1（commit b152c7e）**: FSM 定義・15ユニットテスト作成。本番コードへの接続なし。

**Phase 2a（commit bb228c2）**: FocusChange / ImeOn / ImeOff / WarmupComplete / LongIdle を
FSM に接続。legacy (`CompositionState`) との二重管理期間。

```rust
pub fn is_composition_warm(&self) -> bool {
    let legacy = self.composition.is_composition_warm();
    let fsm = matches!(self.gji_fsm.borrow().state(), GjiState::OnWarm | GjiState::OnComposing);
    debug_assert_eq!(legacy, fsm, "mismatch: legacy={legacy} fsm={fsm}");
    legacy  // Phase 2: まだ legacy が正
}
```

**Phase 3（commit 2b6d25f）**: `legacy` → `fsm` に切替。FSM が SSOT。

```rust
    fsm  // Phase 3: FSM が SSOT
```

### NICOLA 同時打鍵バグ: Option → Vec への変更

Phase 3 切替直後に NICOLA 同時打鍵（例: す+る）で
`debug_assert_eq` パニックが発生した。

**根本原因**: `pending_gji_key_response: Option<Response>` だったため、
2文字目の Response が1文字目を上書きし、`WarmupComplete` が
GjiFsm に届かず OnCold のまま固着した。

**修正（commit a5a9412）**: `Option<Response>` → `Vec<Response>` に変更し、
`platform::send_keys` で `drain_pending_gji_key_responses()` して全件 dispatch。

### Unicode mode での即時 WarmupComplete

`InjectionMode::Unicode`（Windows Terminal など TSF native アプリ）では、
`KEYEVENTF_UNICODE` は GJI composition context を経由しない。
そのため `StartProbe` を発行しても GjiWarmupFsm / ChromeProbe が存在せず、
`WarmupComplete` が届かずに OnCold が固着するバグがあった。

**修正（commit 444e9a6）**: `dispatch_gji_response` で `StartProbe` 受信時に
`injection_mode == Unicode` であれば即 `WarmupComplete(GjiResumed)` を dispatch する。

### NativeF2Consumed のイベント通知

cold 分類集中化リファクタリングで発見された欠落：
物理 F2（ユーザーが手動でモード切替）が消費されたことを
GjiFsm に通知する経路が存在していなかった（commit a90c5c8 で追加）。

```rust
// on_reinject_key での NativeF2Consumed パス
output.dispatch_gji_event(GjiEvent::NativeF2Consumed);
```

## 結果

- warm/cold の SSOT が `gji_fsm.rs` に確立された
- フォーカス変更時のリセット漏れが構造的に防止された
- `ColdKind::classify()` が追加閾値を 1 箇所で管理する
- Unicode mode の OnCold 固着が型レベルで排除された

## 関連 ADR

- ADR-040: 段階的リファクタリング戦略（Phase 1→3 移行パターン）
- ADR-042: Clock トレイトと timed-fsm テスト可能性
- ADR-047: TickableFsm / ImeWarmupStrategy 抽象化（GjiFsm を包む trait 設計）
