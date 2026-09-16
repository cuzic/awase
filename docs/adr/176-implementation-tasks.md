---
id: ADR-176-companion-176-implementation-tasks
title: |-
  ADR-176（IMEモードキー較正UI）実装タスク一覧
type: companion-doc
related_adr:
  - "ADR-092"
  - "ADR-115"
  - "ADR-119"
  - "ADR-125"
  - "ADR-135"
  - "ADR-140"
  - "ADR-141"
  - "ADR-149"
  - "ADR-153"
  - "ADR-174"
  - "ADR-175"
  - "ADR-176"
---

# ADR-176（IMEモードキー較正UI）実装タスク一覧

[ADR-176](176-behavioral-calibration-of-ime-mode-key-shadow-overrides.md)
「決定（v5）」節の8決定を、実装可能な単位に分割したタスクリスト。
`docs/adr/163-implementation-tasks.md`と同じ形式（内容・受け入れ基準・
依存）を踏襲する。各タスクは個別のコミットにすること。

対象領域は`.claude/rules/fix-requires-evidence.md`の「IME belief」
「IME actuation合流点」「物理IMEキーのSuppress/Allow配送判断」の
3ファミリーに該当するため、各コミットは回帰テストを伴うこと。

**本タスクリスト自体をopus-adversarial-consultでレビューしてから
実装に着手すること**（ADR-176 frontmatter status参照）。

## 実装順序

**フェーズ0（前提条件、ADR-176とは独立に先行させる）**:
176-T0（`ActivationSync`冪等性チェック、実機A/B必須）

**フェーズ1（スキーマ確定、Linux上でテスト可能）**:
176-T1（較正レコードのデータ構造・フィンガープリント）→
176-T2（`gate_thumb_key_ime_actions`出力差し替えの純粋関数）

**フェーズ2（配線、Windows-gated）**:
176-T3（GJI側統合点）→ 176-T4（MS-IME側統合点）→
176-T5（`keys.ime_detect`/明示config構造的除外）

**フェーズ3（較正モードの検知・バイパス、Windows-gated）**:
176-T6（`disable_apps`較正モード適用）→
176-T7（awase.exe⇔awase-settings IPC）→
176-T8（awase.exe側の物理キー検知）

**フェーズ4（観測・UI、awase-settings側）**:
176-T9（`ImmGetOpenStatus`ポーリングループ）→
176-T10（較正パネルUI）

**フェーズ5（永続化・反映）**:
176-T11（`config.toml`永続化）→ 176-T12（stale検出・リロード連携）

**フェーズ6（回帰テスト・実機検証）**:
176-T13（`PipelineOutcome`決定表拡張）→ 176-T14（実機A/B手順）

各フェーズ内のタスクは前のタスクに依存する。T0のみ他フェーズと並行
着手可能（むしろ先行させる——ADR-176 decision 8参照）。

---

## フェーズ0: 前提条件

### 176-T0（決定8、必須の前提条件）: `ActivationSync`のSetOpen冪等性チェック

**内容**: `Engine::transition_activation`（`src/engine/engine.rs:456-483`）
がbeliefのinactive→active遷移で無条件に`Effect::Ime(ImeEffect::SetOpen
{ open: true, origin: ActivationSync })`を発行している箇所に、
「beliefが既に高信頼度（`ObservationConfidence::High`または直近の
`Confirmed`な`applied`状態）で実状態と一致していれば`SetOpen`を発行
しない」という冪等性チェックを追加する。BUG-113 ADR-149追記が記録する
「3回→2回」の残る2回目（`ActivationSync`経由の実送信）を減らすのが
目的。

**注意**: この冪等性チェックの具体的な判定条件（どの`applied`状態・
`observation`のconfidenceを「既に一致している」とみなすか）は、
既存の`already_matched`判定ロジック（`ime_controller.rs`等）と整合
させること。新しい判定基準を独自に作らない。

**受け入れ基準**:
- Linux: `cargo test --lib`で`transition_activation`周辺のユニット
  テストを追加（belief遷移パターンごとに`SetOpen`が発行されるか/
  されないかを固定）。
- Windows実機A/B: BUG-113の再現手順（半角/全角キー・変換/無変換キー
  単独タップの反復）で「@」が再発しないことを確認し、
  `docs/known-bugs/BUG-113.md`に追記する。
- `.claude/rules/fix-requires-evidence.md`の「IME actuation合流点」
  再発ファミリーに該当するため、この回帰テスト+実機確認は必須。

**依存**: なし（独立して着手可能、他のADR-176タスクより先に完了させる
ことを推奨）。

---

## フェーズ1: スキーマ確定

### 176-T1（決定6）: 較正レコードのデータ構造とフィンガープリント

**内容**: 較正結果1件を表す構造体を新設する（例:
`crates/awase-windows/src/state/calibrated_mode_key.rs`、Windows非依存の
プラットフォーム非依存な純粋データ構造として、Linux上でも定義・
テストできる場所に置く）。

```rust
struct CalibratedModeKey {
    vk: VkCode,
    result: ImeToggleKind,   // TurnOn/TurnOff/Toggle（v5の初期スコープではTurnOnのみ）
    active_ime_kind: ActiveImeKind,        // GJI / MicrosoftIme
    config_fingerprint: ConfigFingerprint, // 測定時点のconfig1.db/レジストリの指紋
    confirmed_at: /* 保存用の時刻表現 */,
}

enum ConfigFingerprint {
    Gji { session_keymap: Option<i64>, relevant_row: Option<String> },
    MsIme { registry_value_hash: u64 },
}
```

`config1.db`/レジストリの現在値とフィンガープリントを比較する純粋関数
`is_stale(&CalibratedModeKey, current: &ConfigFingerprint) -> bool`も
ここに置く。

**受け入れ基準**: Linux上で`cargo test -p awase-windows --lib`が通る
ユニットテスト（フィンガープリント一致/不一致の判定を固定）。

**依存**: なし。

### 176-T2（決定5）: `gate_thumb_key_ime_actions`出力差し替えの純粋関数

**内容**: `gji_charset_autodetect.rs`の`gate_thumb_key_ime_actions`が
返す`wiring.henkan`/`wiring.muhenkan`（`ImeToggleKind`）を、確定済み
較正結果があればそれで差し替える純粋関数を新設する。

```rust
fn apply_calibration_override(
    static_result: Option<ImeToggleKind>,
    calibrated: Option<&CalibratedModeKey>,
) -> Option<ImeToggleKind>
```

優先順位: `calibrated`が`Some`かつstaleでなければそれを採用、なければ
`static_result`（ADR-176 decision 6・7で「静的分類と一致する較正結果は
保存しない」としたため、この関数自体は単純な`calibrated.or(static_
result)`で足りるはずだが、念のため両者が一致する場合の扱いも
テストで固定する）。

**受け入れ基準**: Linux上でユニットテスト。`calibrated`が`None`/
`Some(stale)`/`Some(fresh)`の3パターンそれぞれで`static_result`との
優先関係を固定する。

**依存**: 176-T1。

---

## フェーズ2: 配線（B3/B4対応）

### 176-T3（決定5）: GJI側統合点への配線

**内容**: `gji_charset_autodetect.rs:768-776`
（`gate_thumb_key_ime_actions`呼び出し直後、`route_thumb_key_action`
呼び出し直前）に176-T2の`apply_calibration_override`を挿入する。

**受け入れ基準**: 176-T13（決定表テスト拡張）でカバー。既存の
`route_thumb_key_action`以降のロジック（thumb/非thumb振り分け・
`mask_auto_detect_for_explicit_config`・GJI離脱時クリア）が影響を
受けないことをテストで確認する。

**依存**: 176-T2。

### 176-T4（決定5）: MS-IME側統合点への配線

**内容**: `runtime/message_handlers.rs:964-966`
（`delegate_assignment`取得直後）に同じ`apply_calibration_override`を
挿入する（ADR-119の教訓：GJI側だけでは不足）。

**受け入れ基準**: 176-T3と同型のテストをMS-IME側の決定表
（存在すれば）に追加。無ければ新設する。

**依存**: 176-T2、176-T3（同じ関数を使うため実装順は前後してもよいが
レビューは同一PRで行う）。

### 176-T5（決定7、B4対応）: `keys.ime_detect`/明示config構造的除外

**内容**: 較正UI側（176-T10）が、較正対象VKが以下のいずれかに該当する
場合、較正の実行を拒否し警告を表示するための判定関数を
`awase-windows`側に新設する（`awase-settings`から呼び出せる形、
`awase_windows`クレートの公開関数として）:

- `keys.ime_detect.{on,off,toggle}`に対象VKが登録されている。
- `keys.ime_on`/`ime_off`/`ime_toggle`に対象VKが素のVK（修飾キー無し）
  として登録されている（`src/config.rs:601-609`の実害報告例と同型）。

**受け入れ基準**: Linux上でユニットテスト（config構造から判定結果を
固定）。BUG-140の教訓（優先順位ではなく構造的除外）に沿っていることを
コメントで明記する。

**依存**: なし（176-T1〜T4と並行して着手可能）。

---

## フェーズ3: 較正モードの検知・バイパス

### 176-T6（決定1、B1/B2対応）: `disable_apps`較正モードの適用

**内容**: 較正モード開始時、`awase-settings.exe`を対象に
`HOOK_STATE.focus_app_disabled`（`hook.rs:1103-1105`）と同型の
バイパスを一時的に有効化する仕組みを追加する。既存の`disable_apps`
設定機構（`app_overrides.disable_apps`相当）を流用できるか、較正専用の
一時フラグを新設するかを実装時に決定する（既存機構の流用を優先——
`.claude/rules/complexity-budget.md`の精神）。

**受け入れ基準**: Windows実機で、較正モード中に対象キーを押しても
awase側の`[shadow-toggle]`等のログが一切出力されないことを確認する
（=完全バイパスできていることの確認）。

**依存**: なし。

### 176-T7（決定2）: awase.exe⇔awase-settings間のIPC

**内容**: 既存の`WM_APP+N`パターン（`crates/awase-windows/src/lib.rs:
299-355`に列挙）に、較正モード開始・対象キー検知結果通知の新しい
メッセージを追加する。`awase-settings`側は`main.rs`の
`send_reload_config_message()`と同型の`FindWindowW`+`PostMessageW`
定型を流用する。

**具体的に決める必要がある事項**（未解決点2）:
- 較正モード開始時に渡す対象VK情報の伝達方法（`WM_APP+N`の
  `wparam`/`lparam`だけで足りるか、共有メモリ等が必要か）。
- 検知結果（タイムスタンプ等）の返却方法（コールバック的な
  `PostMessage`か、ポーリングで別途取得するか）。

**受け入れ基準**: Windows実機で、awase-settingsからの較正モード開始
要求がawase.exe側に届き、awase.exe側が較正モードへ遷移することを
ログで確認する。

**依存**: 176-T6。

### 176-T8（決定2）: awase.exe側の物理キー検知ロジック

**内容**: 較正モード中、awase.exeの既存フック（`hook.rs`）に、対象VKの
物理（非注入）・修飾キー無しKeyDownを検知して176-T7のIPC経由で
awase-settingsへ通知する分岐を追加する。**176-T6の`disable_apps`
バイパスが有効な間は、この検知は通常のshadow-toggle等の処理
パイプラインに一切入らないことをコードレビューで確認する**（B1対策の
核心）。

**受け入れ基準**: Windows実機で、較正モード中に対象キーを押すと
awase-settings側が検知結果を受け取ることを確認する。`architecture_
guard`相当のテキスト走査で「較正モードの検知コードが通常のshadow-
toggleディスパッチを呼んでいない」ことを固定できないか検討する。

**依存**: 176-T6、176-T7。

---

## フェーズ4: 観測・UI（awase-settings側）

### 176-T9（決定3）: 較正専用ネイティブウィンドウ＋`WM_IME_CONTROL`ポーリングループ

**2026-09-16実機スパイクで判明した制約**: `awase-settings`は
eguiバックエンド`winit`を使っており、[ADR-125](125-egui-winit-dynamic-ime-association-focus-model-gap.md)
が実証済みのとおり`winit`の`set_ime_allowed(false)`が
`ImmAssociateContextEx(hwnd, 0, IACE_CHILDREN)`でIMEコンテキストを
デタッチするため、**eguiのメインウィンドウHWNDに対して直接
`ImmGetOpenStatus`/`WM_IME_CONTROL`をポーリングしても機能しない**
（旧版の本タスク記述はこの制約を見落としていた）。一方、実機スパイク
（`crates/awase-windows/examples/ime_observation_spike.rs`）で、
eguiを介さない生の`CreateWindowExW`ウィンドウ上では
`ImmGetDefaultIMEWnd`+`WM_IME_CONTROL`（手法B、awase本体の
`imm.rs::probe_ime_control`と同型）が実際のIME ON/OFF切替を正しく
追跡することを確認済み。

**内容**: `awase-settings`プロセス内に、**較正専用の本物のネイティブ
Win32子ウィンドウ**（`CreateWindowExW`で作成し`winit`が管理しない
独立HWND、`EDIT`コントロールを1つ持つ——較正パネル表示中のみ
生成/表示し、egui本体のウィンドウとは別のHWNDとして扱う）を新設し、
そのHWNDに対して`ImmGetDefaultIMEWnd`+`WM_IME_CONTROL`/
`IMC_GETOPENSTATUS`をタイマーで短い間隔でポーリングするループを
実装する。`crates/awase-settings/Cargo.toml`に`windows-rs`の
`Win32_UI_Input_Ime`/`Win32_UI_WindowsAndMessaging`等必要なfeaturesを
追加する（round4レビューM2が指摘：現状無い）。ウィンドウ作成・
メッセージポンプの実装はスパイク（`ime_observation_spike.rs`の
`create_window`/`window_proc`/`method_b_wm_ime_control`）をそのまま
移植できる。TSF/COMは使わない（決定3、非スコープ——実測でTSF
グローバルコンパートメントが状態変化を反映しないことを確認済み）。

**実測が必要な値**（`tuning-constants.md`対象）: ポーリング間隔・
タイムアウト。較正専用ウィンドウ上で対象キー押下からGJI/MS-IMEが
実際にIME状態を変えるまでの実測msを取ってから決定する（スパイクの
250msポーリングでも遷移を取りこぼさなかったが、確定値は本タスクで
実測する）。

**受け入れ基準**: Windows実機で、較正フロー中にIME状態変化を正しく
検知できることを確認する。

**依存**: 176-T6（バイパスが効いた状態で測定する必要があるため）。

### 176-T10（決定4・7）: 較正パネルUI

**内容**: `awase-settings`に較正パネル（対象キー選択・較正開始ボタン・
進捗表示・結果確認ダイアログ）を新設する。176-T5の警告判定を
呼び出し、該当する場合は較正開始前に警告を表示して中断する。
2回一致確定ロジック（決定4）と「変化なし/Toggle判別不能は保存
しない」ロジック（決定7）をここに実装する。

**受け入れ基準**: 手動UIテスト（awase-settingsを実機で起動し
一連のフローを確認）。

**依存**: 176-T5、176-T7、176-T8、176-T9。

---

## フェーズ5: 永続化・反映

### 176-T11（決定6）: `config.toml`永続化

**内容**: 176-T1のデータ構造を`config.toml`の新設セクションへ
シリアライズ/デシリアライズする。既存の`app_overrides`等の設定
ブロックとの一貫性を保つ（未解決点1）。

**受け入れ基準**: Linux上でシリアライズ/デシリアライズのラウンド
トリップテスト。

**依存**: 176-T1。

### 176-T12（決定6）: stale検出とリロード連携

**内容**: awase.exeの設定リロード時（`reload_config()`、`app/mod.rs:
653`）に176-T1の`is_stale`判定を実行し、staleな較正結果を無効化して
静的分類へフォールバックする。GJI側の再同期条件
（`ime_kind_detected() && active_ime_kind() == GoogleJapaneseInput`、
`app/mod.rs:748-757`）を満たさない場合の即時反映されないケースに
ついて、awase-settings側のUIで「反映を確認するには対象アプリへ
フォーカスを戻してください」等の案内を出す。awase.exe未起動時の
`FindWindowW`失敗時は「次回起動時に反映されます」と案内する
（round4レビューM4）。

**受け入れ基準**: Windows実機で、config1.dbを意図的に変更した後
較正結果が正しく無効化されることを確認する。

**依存**: 176-T3、176-T4、176-T11。

---

## フェーズ6: 回帰テスト・実機検証

### 176-T13: `PipelineOutcome`決定表テストの拡張

**内容**: `gji_charset_autodetect.rs`の既存決定表テスト
（`PipelineOutcome::{Nothing, Delegate, ActuationAuto, ShadowOverride,
DelegateAndShadowOverride}`）に較正結果を入力軸として追加する。
**ADR-141必須条件2と同型の落とし穴に注意**——軸を追加するだけでは
既存ケースが無改造で緑のまま通り新機能の検証がゼロになるため、
`expected_outcome`側を仕様として書き直すこと。Hiragana/Katakanaを
対象に含める場合は`transport.rs::plan_tests`も拡張する。

**受け入れ基準**: Linux上で`cargo test -p awase-windows --lib`が通る、
かつ較正軸を追加したことで少なくとも1件以上の新しい期待値が
既存コードでは満たされない（＝テストが実際に176-T3/T4の実装を
検証している）ことを確認してからマージする。

**依存**: 176-T3、176-T4。

### 176-T14: 実機A/B検証手順

**内容**: 以下の手順をdragonflyg4実機で実施する:
1. 176-T0（`ActivationSync`冪等性チェック）単体でBUG-113の非再発を
   確認する。
2. BUG-143相当の状況（`config1.db`が未割当または誤った分類）を
   意図的に作り、較正UIで正しく測定・確定できることを確認する。
3. 較正結果適用後、TsfNativeアプリ（Windows Terminal等）で対象キー
   単独タップ→Engineが正しくActiveへ遷移することを確認する。
4. 較正結果適用状態で「@」が再発しないことを、BUG-113の再現手順
   （反復タップ）で確認する。
5. `config1.db`を変更してstale検出→フォールバックが正しく動作する
   ことを確認する。

**受け入れ基準**: 上記5点全てのログ・実機観察結果を
`docs/known-bugs/`または本ADRに記録する。

**依存**: 176-T0〜T13すべて。
