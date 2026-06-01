# ADR-038: ForceGuardSet / DriftMonitor 型分解

## ステータス

採用済み（2026-05-29）

## コンテキスト

旧 `ImeRecoveryState` は「IME 検出失敗カウンタ」と「force-ON ガード」を 1 つの構造体に混在させていた。これにより：

- 「検出失敗の検知段階」と「force-ON の発動段階」が同じ型で表現され、状態遷移が implicit になった
- `force_on_until_ms` のような「期限付き boolean」が ADR-032 で排除した sideband guard の残骸として残った
- テストで「カウンタが閾値を超えたら force-ON が発動する」という不変条件を検証しにくかった

## 決定

`ImeRecoveryState` を 2 つの独立した型に分解する。

**DriftMonitor（発火前・検知段階）:**
```rust
struct DriftMonitor {
    miss_count: u32,
    threshold: u32,
}
impl DriftMonitor {
    fn record_miss(&mut self) -> Option<ForceOnReason>;
    fn reset(&mut self);
}
```

**ForceGuardSet（発火後・ガード段階）:**
```rust
struct ForceGuardSet {
    guards: BTreeSet<ForceOnReason>,
}
impl ForceGuardSet {
    fn activate(&mut self, reason: ForceOnReason);
    fn release(&mut self, reason: ForceOnReason);
    fn is_active(&self) -> bool;
    // desired_open を直接書き換えない — effective_open() で override する
    fn effective_open(&self, desired: bool) -> bool;
}

enum ForceOnReason {
    BrokenAppBootstrap,
    PanicReset,
    DetectMissThreshold,
    ProfilePolicy,
}
```

**重要な制約:** `ForceGuardSet` は `desired_open` を直接書き換えない。ADR-032 の設計原則 1（「desired_open を書き換えられるのは UserImeSetIntent / UserImeToggleIntent のみ」）を守るため、`effective_open(desired)` として override を表現する。

## なぜ型分解か

`DriftMonitor` は「何回連続で検出失敗したか」という単調増加するカウンタ。`ForceGuardSet` は「今 force-ON が必要か、その理由は何か」という現在の状態集合。混在させると「カウンタが閾値を超えたタイミング」という瞬間的なイベントが隠蔽される。

## 結果

- `DriftMonitor::record_miss()` が `Option<ForceOnReason>` を返すことで「閾値超過イベント」が型で表現される
- `ForceGuardSet::guards` が `BTreeSet` なので複数の force-ON 理由が共存でき、全理由が解消するまで guard が維持される
- フォーカス変更時の `DriftMonitor::reset()` と `ForceGuardSet::release(reason)` が独立して呼べる

## 関連 ADR

- ADR-032 (IME 状態 reducer 4 階層)
