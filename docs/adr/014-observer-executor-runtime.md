# ADR-014: Observer / Executor / Runtime の3層分離

## ステータス

採用

## コンテキスト

AppState が「判断ロジック」「OS 観測」「副作用実行」「所有と配線」の4つの責務を持っていた。特に `on_focus_changed`（120行）と `refresh_ime_state_cache`（80行）は Win32 API 呼び出しと分類ロジックが混在し、テスト不能だった。

Effect/Decision モデルで副作用を宣言化したが、「OS 観測→判断→実行」のうち観測がまだ Engine や AppState に残っていた。

## 決定

3つの層に分離する。

### Observer 層（Win32 依存、観測専用）

```
src/observer/
  ime_observer.rs    — Win32 API で IME 状態を取得 → ImeObservation
  focus_observer.rs  — Win32 フォーカス分類 → FocusObservation
```

OS 非依存の観測結果型（`ImeObservation`, `FocusObservation`）は Engine 側の `observation.rs` に定義。Observer は Win32 API を呼んで観測結果型を返すだけ。

### Engine（pure、判断のみ）

`EngineCommand::ImeObserved` / `FocusChanged` で観測結果を受け取り、Effect を含む Decision を返す。Win32 非依存。

### DecisionExecutor（副作用実行）

```
src/executor.rs — Effect::Input/Timer/Ime/Focus/Ui/ImeCache を実行
```

Win32 API 呼び出しはここだけ。

### Runtime（所有と配線）

```
src/runtime.rs — Engine + DecisionExecutor + layouts を保持、パイプラインを駆動
```

AppState を Runtime にリネーム。判断ロジックなし。

## 結果

- Observer → Engine → Executor の一方向パイプライン
- 各層が独立テスト可能
- main.rs は OS イベント → Observer → Engine.on_command → Executor の配線のみ
