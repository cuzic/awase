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
176-T9（awase.exe本体による`WM_IME_CONTROL`ポーリング）→
176-T10（較正パネルUI）

**フェーズ5（永続化・反映）**:
176-T11（`config.toml`永続化）→ 176-T12（stale検出・リロード連携）

**フェーズ6（回帰テスト・実機検証）**:
176-T13（`PipelineOutcome`決定表拡張）→ 176-T14（実機A/B手順）

各フェーズ内のタスクは前のタスクに依存する。T0のみ他フェーズと並行
着手可能（むしろ先行させる——ADR-176 decision 8参照）。

---

## フェーズ0: 前提条件

### 176-T0（決定8、2026-09-17見送り・独立クリーンアップへ降格）: `ActivationSync`のSetOpen冪等性チェック

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

**設計案の棄却（2026-09-17、opus-adversarial-consult）**: 上記内容
（`handle_engine_activation_sync`または`transition_activation`への
早期returnによる冪等性チェック）は実装前レビューでBlocker多数により
棄却された。指摘全文は`/tmp/opus-review-adr176-t0-design.md`
（セッション内スクラッチパス、以後のセッションでは再現不可）。要点:

1. **置き場所が誤り**: `handle_engine_activation_sync`の早期returnは
   belief記帳（`ImeApplyRequested`のdispatch等）を止めるだけで、
   実際の`SendInput`（`decision.effects`に残る`SetOpen`、
   `kp_stage_execute`経由で無条件実行）は止まらない。むしろ`applied`が
   `Confirmed`へ昇格しなくなり、既存の`gji_direct_already_matches`
   dedupが壊れて送信が**増える**方向に倒れる（2026-07-05に一度踏んだ
   既知の失敗、`key_pipeline.rs:426-440`のコメント参照）。
2. **述語がBUG-113の再現経路で発火しない**: ADR-149実機ログ上、
   問題の送信時点で`shadow_on=Some(false)`・`target=true`であり、
   `applied`ベースのどんな一致判定も偽になる。BUG-113の本質は
   「GJI自身が物理キーに反応して既にONにしたが、awase（TsfNative×GJIは
   `FeedbackPolicy::Blind`）にはその証拠が無い」ことであり、`applied`
   （awase自身が最後に送ったコマンドの記録）にはこの情報が原理的に
   入らない。
3. **正しい場所に置き直しても、ADR-149が実機ログ解析の上で棄却済みの
   「案B」と同型の結末**（3回→2回にしかならず108msずれるだけ、加えて
   `apply_force_on_for_imm_broken`の誤発火リスク）に落ちる。
4. OFF方向への適用はBUG-141/ADR-171の再演になるため不可（ON方向限定）。
   `handle_engine_set_open`（ユーザー明示操作側）への適用も、
   BUG-037/BUG-141の実害と同型になるため不可。
5. **受け入れ基準にも検出力が無い**: BUG-113は2026-09-07時点で既に
   「@」非再発を確認済み（A群0件）のため、この基準では効果の有無を
   区別できない。1タップあたりの`VK_IME_ON`送信回数を主指標にすべき。

**今後の方向性（実装未着手）**: 置くなら`state/ime_actuation_decision.rs`
の`decide_gate`/`decide_attempt`（既存`already_matched`と同じ入力・
同じ場所）、ON方向限定、`Optimistic`は除外。ただし目的記述
（「較正でTurnOnキーが増える」ことへの対処としての前提条件、という
位置づけ）自体もB2を踏まえて再評価が必要——較正で増える経路
（物理キー→shadow-toggle→belief OFF→ON→ActivationSync）では
`applied`は常に不一致側にあるため、このT0では対処できない。
送信回数を本当に減らしたいなら、ADR-149が「別ADR起票の価値がある」と
した案C（delegateとshadow-toggleの排他性修復、送信3の発生自体を
止める）の方が対象を取り違えていない可能性がある。

**round2レビュー（2026-09-17、置き場所を修正した第2案の検証）**:
上記の懸念を踏まえ、「`handle_engine_activation_sync`は一切触らず、
`decision.effects`から`ActivationSync`由来の`SetOpen(true)`だけを、
`Decision::find_ime_set_open_with_origin()`（core側に既存）を使って
belief確定後・実行前に取り除く」という第2案を作りコードで裏取りした
上で同じレビュアー（opus）へ再相談した。指摘全文は
`/tmp/opus-review-adr176-t0-design-round2.md`（セッション内スクラッチ
パス、以後のセッションでは再現不可）。結論:

- **方向性は妥当**（round1のB1/B3/B4/B9/B10は解消）だが、提示した
  呼び出し位置（`kp_stage_post_decision`より前）では`kp_stage_post_
  decision`自体が`find_ime_set_open_with_origin()`の`Some`を入口条件と
  しているため、beliefの書き込みごと丸ごと消えてB5/B6/B7が復活する
  ——正しい位置は`kp_stage_post_decision`の**後**・`kp_stage_execute`の
  **前**（非キーボード経路`execute_from_loop`はbelief側の対応処理が
  そもそも無いため既存位置のままでよい、キーボード経路と非対称になる
  理由をdoc化必須）。
- 実装草案の`retain`が全`SetOpen`を無差別に消すバグがあり、
  ExplicitUserAction由来のSetOpen（無変換/変換ソロタップのdelegate
  経路、まさに較正が対象とするキー）を巻き添えにする恐れがあった
  （originまで含めた完全一致に修正要）。
- belief側を残す設計にすると、対応するapplyが永久に起きない
  「幽霊pending」が最大8秒（`IME_APPLY_PENDING_TIMEOUT_MS`）残り、
  warnスパム・誤ったgeneration紐付け・`applied`のOptimisticへの後退
  （フィルタの自己無効化）を引き起こす。対策には
  `handle_engine_activation_sync`に`will_actuate: bool`を足して
  `ImeApplyRequested`のdispatchだけを条件分岐させる等の追加設計が要る。
- **本質的なトレードオフ**: TsfNative×GJI（BUG-113の環境そのもの）は
  `FeedbackPolicy::Blind`のため、`applied`（awase自身が最後に送った
  コマンドの記録）は「実際にIMEが開いた」ことの証拠にならない。この
  条件でSetOpenを止めると、前提が誤っていた場合にON方向の唯一の
  是正手段（`apply_force_on_for_imm_broken`）が同じ条件で既に止まって
  いるため構造的にゼロになる。緩和策（`Confirmed`のタイムスタンプに
  年齢上限を設ける、またはフォーカスごとに最初の1回は必ず通す）の
  どちらかが必要。
- 効く範囲は実質`NotRomajiInput`/`NotJapaneseIme`経由のInactive→
  Active往復のみで、`ImeOff`/`UserDisabled`復帰経路では発火しない
  （＝較正が増やす送信には当たらない、B2の裏取り）。
- 受け入れ基準は「@」の非再発（検出力ゼロ）ではなく、既存の
  `[apply-ime]`/`[tsf-eager-warmup]`ログ行とjournalの
  `ActuationDecision`を突き合わせた「1タップあたりの`VK_IME_ON`
  実送信本数」を主指標にすべき。判定述語は`state/`側の純粋関数に
  置かないと`cargo test --lib`がLinux上で実行されない
  （`runtime/`配下は`#[cfg(windows)]`ゲート）。

**T0の見送り（2026-09-17、ユーザー判断）**: round2で技術的には
実現可能な設計に到達したが、効果範囲が「BUG-113にもほぼ寄与しない
狭い独立クリーンアップ」に留まることが判明したため、今回はT0自体の
実装を見送ることにした。ADR-176決定8の「必須の前提条件」という位置
づけも撤回し、較正機能の実質的な安全装置は既存のopt-inゲート（既定
OFF、実機A/B確認まで適用しない）とする（詳細はADR本文「決定8」参照）。
較正機能（T8〜T10、結果はログのみでIME制御には未反映）はT0を待たずに
現状のまま進める。将来、較正が増やす送信への対策が必要になった場合は
ADR-149の案C（delegateとshadow-toggleの排他性修復）を優先候補とする。

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
`HOOK_STATE.focus_app_disabled`（`hook.rs:1102-1104`）と同型の
バイパスを一時的に有効化する仕組みを追加する。既存の`disable_apps`
設定機構（`app_overrides.disable_apps`相当）を流用できるか、較正専用の
一時フラグを新設するかを実装時に決定する（既存機構の流用を優先——
`.claude/rules/complexity-budget.md`の精神。既存の`disable_apps`＋
reload経路（`runtime/mod.rs:1988-1991`、フォーカス変更が無くても
現在のフォーカス先で再評価する）を流用すれば、較正モードが
「awase-settingsが既にフォーカスを持っている状態」で始まっても
バイパスが即座に効く——較正専用の一時フラグを新設する場合は、この
再評価配線を自前で用意する必要がある。round6 m4対応）。

**較正モードの解除（round6 m4/B3対応）**: 較正終了時（正常終了・
UIでのキャンセル・176-T9のPID不一致検知）に確実に`disable_apps`を
元に戻すこと。加えて較正モード全体にタイムアウトを設け、
awase-settingsからの応答が一定時間無い場合（クラッシュ・強制終了）は
自動的にバイパスを解除する（round6 B3対応、awase-settingsが死んだ
状態でawaseが効かなくなり続ける事故を防ぐ）。

**受け入れ基準**: Windows実機で、較正モード中に対象キーを押しても
awase側の`[shadow-toggle]`等のログが一切出力されないことを確認する
（=通常のIME belief更新・actuationパイプラインから完全バイパス
できていることの確認——較正専用の検知・観測コード自体はADR決定1の
訂正どおりこのバイパスの外で動く、混同しないこと）。異常終了
シナリオ（awase-settingsを較正中に強制終了する）でタイムアウトにより
バイパスが自動解除されることも確認する。

**依存**: なし。

### 176-T7（決定2・3）: awase.exe⇔awase-settings間のIPC（較正モード開始・自PID/HWND通知・結果返却）

**内容**: 既存の`WM_APP+N`パターン（`crates/awase-windows/src/lib.rs:
299-355`に列挙）に、較正モード開始・対象キー検知結果＋観測結果通知の
新しいメッセージを追加する。`awase-settings`側は`main.rs`の
`send_reload_config_message()`と同型の`FindWindowW`+`PostMessageW`
定型を流用する。

較正モード開始メッセージには、対象VKに加えて**awase-settings自身の
PID**と**トップレベルHWND**を含める（176-T9でawase.exe本体がこの
HWNDに対して`imm.rs::probe_ime_control`を呼ぶために必要、PIDは
HWND再利用検知に必要——2026-09-16の実機検証で、`awase-settings.exe`
のeguiメインウィンドウ（`winit`管理下）に対しても`ImmGetDefaultIMEWnd`
+`WM_IME_CONTROL`が正しく機能することを確認済み、
[ADR-125](125-egui-winit-dynamic-ime-association-focus-model-gap.md)
実機検証ログ2の再現）。

**awase-settings側の新規作業（round6 B4対応、旧版で漏れていた）**:
UIスレッドから`GetActiveWindow`（`windows` 0.58の既存feature
`Win32_UI_WindowsAndMessaging`で足り、新規feature追加は不要）で
自身のトップレベルHWNDを取得する。**`FindWindowW`によるクラス名
検索は採らない**——winitの既定クラス名は汎用の`"Window Class"`で
あり、`focus/imm_learning.rs:22-23`がBUG-107の文脈で「プロセス間で
衝突する」と明記している。

**具体的に決める必要がある事項**（未解決点2）:
- 較正モード開始時に渡す対象VK・自PID・自HWND情報の伝達方法
  （`WM_APP+N`の`wparam`/`lparam`だけで足りるか、共有メモリ等が
  必要か）。
- 検知結果・観測結果（タイムスタンプ、観測したIME状態遷移）の返却
  方法（コールバック的な`PostMessage`か、ポーリングで別途取得するか）。

**受け入れ基準**: Windows実機で、awase-settingsからの較正モード開始
要求がawase.exe側に届き、awase.exe側が較正モードへ遷移することを
ログで確認する。

**依存**: 176-T6。

### 176-T8（決定2）: awase.exe側の物理キー検知ロジック

**内容**: 較正モード中、awase.exeの既存フック（`hook.rs`）に、対象VKの
物理（非注入）・修飾キー無しKeyDownを検知して176-T7のIPC経由で
awase-settingsへ通知する分岐を追加する。**この検知コードは
`hook.rs:1102`の`app_disabled`早期returnより手前に置く**——既存の
`physical_key_state`更新ブロック（`hook.rs:1094-1097`）と同じ配置
パターンで、このバイパスが較正検知も含めて全停止させる不変条件
（round6 B1/m3対応）を踏まえた上での**明示的な例外**として位置づける。
較正モードのON/OFF・対象VK・PID・HWNDは`HOOK_STATE`側の状態として
持たせる（round6 M6対応、フックコールバックが`app_disabled`判定と
同じタイミングで読む必要があるため。ADR-164が集約した「裸のグローバル
staticより既存singletonへの集約を優先する」方針に従い、新しい裸の
グローバルstaticを生やさない）。**176-T6の`disable_apps`バイパスが
有効な間は、この検知は通常のshadow-toggle等の処理パイプラインに
一切入らないことをコードレビューで確認する**（B1対策の核心）。

**受け入れ基準**: Windows実機で、較正モード中に対象キーを押すと
awase.exeのログ（`tracing::info!`、`[calibration] 対象キー押下を検知`）に
記録されることを確認する（awase-settingsへの返却は176-T9の結果チャネルで
まとめて設計・実装する——opus-adversarial-consultレビュー round8 B3対応:
T7が`WM_CALIBRATION_RESULT`の型を先送りしたのと同じ理由で、T8時点では
awase-settings側の受信チャネルを新設しない）。`architecture_guard`の
テキスト走査で「較正モードの検知コードが通常のshadow-toggleディスパッチ・
belief更新に一切触れていないこと」「検知コードが`focus_app_disabled`
早期returnより手前に置かれていること」「状態のミラー書き込み口が
`begin_calibration_bypass`/`end_calibration_bypass`の2箇所に限定されている
こと」を固定する。

**依存**: 176-T6、176-T7。

**実機検証（2026-09-17、dragonflyg4）**: 較正モード中に物理VK_NONCONVERTを
押下し、awase.exeのログに`[calibration] 対象キー押下を検知`が記録される
ことを確認した。176-T9a/T9bと合わせたエンドツーエンド検証は下記
176-T9bの実機検証節を参照。

---

## フェーズ4: 観測・UI

### 176-T9（決定3）: awase.exe本体による`WM_IME_CONTROL`ポーリング（較正専用ウィンドウ不要、v8で衝突対応を追加）

**v6（撤回済み）**——「eguiのメインウィンドウでは`WM_IME_CONTROL`も
機能しないため較正専用のネイティブWin32子ウィンドウが要る」という
記述はopus round5レビューでBlocker指摘され、2026-09-16の追加実機検証
（`awase-settings.exe`の実際のバグ報告画面に約28秒間フォーカスした
状態で手法Bを観測、タイムアウト無しで正しく追跡し続けた）で誤りと
確定した。**v7**でawase.exe本体がawase-settingsのPID（HWNDは運ばない、
ライブなフォーカス追跡を使う——round9訂正）を基準に観測する設計に
単純化したが、opus round6レビューで観測をawase.exe本体へ移したことに
よる新規の衝突（Blocker4件）が見つかった。詳細はADR本文「決定3」参照。
以下はv8での対応を反映した内容。

**内容**: 較正専用ウィンドウは新設しない。**awase.exe本体**が
176-T7のIPCで受け取った`awase-settings`のPID（HWNDは運ばない、
ライブなフォーカス追跡を使う——round9訂正）を基準に、既存の
`imm.rs::probe_ime_control`（`awase-windows`クレート内の唯一の
チョークポイント、新規APIを増やさない）を使ってポーリングする。
較正モード状態は176-T8のとおり`HOOK_STATE`側に置き、観測ループ
（ランタイム側`spawn_local`タイマー）はこれを読み取るだけにする
（round6 M6対応）。観測結果を176-T7の同じIPC応答でawase-settingsへ返す。

**round6 B1対応（`app_disabled`ゲートとの衝突）**: 較正probeは
`ime_refresh.rs:70-78`の`app_disabled`早期return（probeを含む全停止）
の**明示的な例外**として実装する。この経路の観測結果は`ImeModel`/
`observation_store`へ**一切dispatchしない**——通常のIME belief更新
パイプラインとは完全に独立したデータパスにする。可能であれば
`architecture_guard`相当のテキスト走査で「較正probeのコードから
belief書き込みAPI（`ImeModel`のsetter等）が呼ばれていないこと」を
固定する。

**round6 B2対応（`send_health`汚染の回避）**: `imm.rs:263`の
`send_health::record`は較正probeでは**呼ばない**。
`runtime/executor.rs:986-995`に記録されている「診断専用probeが
`send_health`を誤作動させたため削除された」前例と同じ轍を踏まない
ため、`send_ime_control_raw`自体は変更せず、較正probe専用の薄い
ラッパ関数（`record`を呼ばない）を新設するか、`record`呼び出しに
スキップフラグを追加する。どちらを採るかを本タスクの実装時に決定し、
コミット本文に理由を残す。

**round6 B3対応（他プロセスHWNDのライフサイクル）**: 観測tickごとに
`self.platform.focus.pid()`/`process_name`が較正セッションのPID/
`awase-settings.exe`と一致することを確認する（round9訂正、HWNDは
運ばないためHWND再利用の懸念自体が構造的に発生しない）。不一致
（awase-settingsの終了・PID再利用）なら較正モードを即座に中止し、
176-T6のタイムアウト機構と同じ経路で`disable_apps`を解除する。

**round6 M2対応（probeを出すスレッド）**: awase.exe本体は単一
スレッド・メッセージループ駆動で、そのスレッドがLLキーボードフックの
コールバックスレッドでもある。同一スレッドから同期
`SendMessageTimeoutW`を出すとフックコールバックの応答が遅れ
`LowLevelHooksTimeout`（既定~300ms）超過でフックが外されるリスクが
ある（BUG-34、`SMTO_ABORTIFHUNG`はハング開始後の相手には効かない）。
較正probeは`win32_async::run_with_timeout`/offload経由でワーカー
スレッドに出す。

**round6 M4対応（観測の基準点・`None`の扱い）**: 観測ポーリングは
較正モード開始（176-T7のIPC受信）と同時に開始し、押下前の`open`値を
基準点として保持する。`open=None`（`SendMessageTimeoutW`失敗）は
「変化なし」と区別し再試行として扱う（決定4参照）。

**round6 M5対応（dylint許可リスト）**: `lints/actuation_call_guard/
src/lib.rs`の`probe_ime_control`許可呼び出し元リスト（現行6件）に
較正probeの呼び出し元を追加する。コミット本文に「棚卸しではなく
新規追加」であることを明記する（`.claude/rules/complexity-budget.md`
1-in-1-out対象、未発効だが前例として残す）。

`awase-settings`側の変更は、新しいWin32ウィンドウ・新しい
`crates/awase-settings/Cargo.toml`のwindows-rs feature・新しい
`SendMessageTimeoutW`呼び出しという意味では**不要**（旧版が要求
していた`Win32_UI_Input_Ime`等の追加は撤回）——ただし自PID/HWND取得
のための`GetActiveWindow`呼び出し（176-T7）は別途必要。TSF/COMは
使わない（決定3、非スコープ——「@」機序という独立した理由、
ADR-153/BUG-113）。

**実測が必要な値**（`tuning-constants.md`対象）: ポーリング間隔・
タイムアウト。決着実験（`spike_egui_ime_control_probe.rs`の
`POLL_INTERVAL_MS=100`/`SEND_IME_CONTROL_TIMEOUT_MS=50`、egui環境で
実際に確認済み——`ime_observation_spike.rs`の250msはwinit/eguiを
含まない環境の値のため根拠に使わない、round6 M3対応）を出発点とし、
対象キー押下からGJI/MS-IMEが実際にIME状態を変えるまでの実測msを
取ってから確定する。

**受け入れ基準**（round6 M1対応、決着実験の手順を移植）:
1. awase.exeを`[ime-io] cross_process ... kind=probe`のdebugログが
   出る水準で起動する（`imm.rs:254-259`が既にこのログを出す）。
2. 較正モードを開始する（176-T7）。
3. awase-settingsを前面にしたまま、言語バーまたは物理キーでGJIを
   OFF→ON→OFFと3回手動で切り替える。
4. awase.exe側のログで`open`の遷移が3回とも観測され、`elapsed_ms`の
   最大が`send_health::SLOW_THRESHOLD_MS`（100ms）を下回ることを
   確認する（B2のブレーカ誤作動が起きない余裕があることの確認を
   兼ねる）。
5. **（2026-09-16決着実験v2で確定）** (3)(4)は較正パネルの**テキスト
   入力欄に実際にフォーカスした状態**でのみ成立する。テキスト欄以外の
   ウィジェット（ボタン・チェックボックス等）にフォーカスがある状態
   では、GJIは生の物理キーに一切反応せず`open`値は変化しない
   （`disable_apps`でawaseを完全バイパスした状態でも同じ——awaseの
   自作自演ではなくGJI自身がテキスト入力コンテキストの有無で挙動を
   変えている。当初「awase自身のActivationSyncが原因では」という
   仮説を立てたが決着実験v2で否定された）。したがって受け入れ基準は
   「テキスト欄フォーカス時に正しく検知できる」ことに加え、
   「テキスト欄以外フォーカス時は変化が観測されない（これが正しい
   仕様上の挙動）」ことも確認する——後者を異常と誤診断しないこと。
6. 可能なら`spike_egui_ime_control_probe`を同時に走らせ、awase.exeが
   見た遷移列とspikeが見た遷移列が一致することを確認する（採る場合は
   本タスクの依存にspikeのビルドが加わる）。
7. `disable_apps`が実際に`awase-settings.exe`へ適用された状態で
   上記1〜6を実施する（2026-09-16決着実験v2で確認済み——
   `crates/awase-windows/examples/spike_calibration_decisive_v2.rs`、
   ADR本文frontmatter status参照。実測レイテンシ247〜2295ms、
   `elapsed_ms`は全サンプル20ms未満）。

**round6 M1対応・v2決着実験で確定した新規要件**: 較正専用UIは、
測定区間中ずっと**実際のテキスト入力ウィジェットにキーボードフォーカス
を保持し続ける**設計にすること（176-T10で詳細化）。単にawase-settings
のウィンドウを前面にするだけでは不十分——GJIがIME入力コンテキストを
持つのはテキスト入力欄にフォーカスがある間だけであり、これは
`disable_apps`バイパスの有無に関係しない仕様上の挙動である。

**依存**: 176-T6（バイパスが効いた状態で測定する必要があるため）、
176-T7（PIDの受け渡し。HWNDはIPCで運ばない——round9訂正、上記参照）。

**実装状況（2026-09-17追記）**: awase.exe内で完結する部分（probe実装・
ポーリング・押下起点の確定ロジック）は176-T9aとして実装済み
（opus-adversarial-consultレビューround9でBlocker5件を検出・反映、
特にT0未完のまま`Runtime::set_calibrated_mode_key`を呼ばない方針に
変更——決定結果は`tracing::info!`でログ記録するのみ）。
awase-settingsへの結果返却IPCは176-T9bとして分離した（下記参照、
round8/round9の議論で、T7の`WM_CALIBRATION_RESULT`先送りと同じ理由
——受け入れ側の設計が固まる前にペイロード形式を決めると作り直しに
なる——による）。

### 176-T9b（決定3、176-T9からの分離）: awase-settingsへの較正結果返却IPC

**内容**: awase.exe本体（176-T9aの較正probeループ）が確定/却下した
結果を、`WM_CALIBRATION_RESULT`（`WM_APP+31`）で`awase-settings`へ
返す。opus-adversarial-consultレビューround8で確定した設計を採る:

- **awase-settings側にメッセージ専用ウィンドウ（`HWND_MESSAGE`）を
  新設する**（`with_msg_hook`は不採用——round8の実機コード調査で、
  winitの`dispatch_peeked_messages`という特定のPeekMessage呼び出しの
  中でしか呼ばれず、OS由来のモーダルループ（サイズ変更・システム
  メニュー等）に対して構造的に脆いと判明したため）。固定クラス名
  （`calibration_ipc::CALIBRATION_RESULT_WINDOW_CLASS_NAME`）を
  `main()`冒頭・`eframe`のイベントループ開始前に1回だけ登録する
  （winitと同一スレッドのメッセージキューに自動的に相乗りする、
  Windowsのメッセージキューはスレッド単位でありウィンドウ単位では
  ないため）。
- **awase.exe側はHWNDをIPCで受け取らず、`FindWindowW`で固定クラス名を
  探して送る**（round7 S1・round9 N5と同じ方針。`awase_tray_window`と
  同型）。
- ペイロードは`calibration_ipc::CalibrationResultPayload{vk, kind}`
  （`kind`は`ConfirmedOn`/`Rejected`の2値、`Undetermined`は送らない）。
- 確定/却下はセッション中1回だけ送る（`CalibrationConfirmState`は
  確定後も同じverdictを返し続けるため、送信側でガードする）。

**受け入れ基準**: awase.exeのログで較正が確定/却下されたことを確認した
上で、awase-settings側のログにも同じ結果が届いていることを確認する
（T10未実装のため、受信側は現時点ではログ出力のみ）。

**依存**: 176-T9a。

**実機検証完了（2026-09-17、dragonflyg4）**: 176-T8/T9a/T9bのエンドツー
エンドを実機で確認した。較正モード中に物理VK_NONCONVERTキーをIME ON
状態で2回押下（各押下後3秒のsettle windowが経過するまでフォーカス保持）
した結果、awase.exe側で`[calibration] 確定: vk=VkCode(29)
ImeToggleKind::On (pre=true post=true)`、同時刻にawase-settings.log側で
`[calibration] 結果を受信: vk=VkCode(29) kind=ConfirmedOn`を確認した
（受信からログ出力まで1ms）。

検証で判明した、176-T10設計時に踏まえるべき2点:
1. **フォーカス保持要件の再確認**: 押下後settle window（3秒、
   `CALIBRATION_TRIAL_SETTLE_WINDOW_MS`）の間、awase-settingsの
   テキスト入力欄にキーボードフォーカスが無いと`spawn_calibration_
   probe_loop`のフォーカス一致チェックに阻まれ試行が完了しない
   （既知要件、決着実験v2/round6 M1と同じ制約を実機で再確認）。
2. **`begin_calibration_bypass`の無条件epoch更新への対応**: 同一pid・
   同一VKでの再武装（例: UIがセッション維持のため定期的にkeepalive
   送信する設計にした場合）でも`calibration_epoch`が無条件に進み、
   `TrialTracker`/`CalibrationConfirmState`の蓄積が失われる
   （round9 S6の意図的設計、VK変更時の混入防止が目的）。176-T10が
   セッション維持のkeepaliveを送る設計にする場合は、進行中の試行の
   蓄積が失われないよう間隔を調整するか、`begin_calibration_bypass`
   側に「進行中の試行がある間は再武装しない」ガードの追加を検討する
   こと。

### 176-T10（決定4・7）: 較正パネルUI

**内容**: `awase-settings`に較正パネル（対象キー選択・較正開始ボタン・
進捗表示・結果確認ダイアログ）を新設する。176-T5の警告判定を
呼び出し、該当する場合は較正開始前に警告を表示して中断する。
2回一致確定ロジック（決定4）と「変化なし/Toggle判別不能は保存
しない」ロジック（決定7）は、176-T9からIPC経由で返る観測結果を
消費する形でここに実装する（判定ロジック自体はUI層ではなく
Linux上でテスト可能な純粋関数として176-T1近辺に置くことが望ましい
——実装時に検討）。

**round6 M1/決着実験v2で確定した必須要件**: 較正パネルには実際の
`egui::TextEdit`（既存のバグ報告画面の説明欄と同種のウィジェット）を
1つ配置し、較正開始ボタン押下から結果確定までの間、**このウィジェット
にキーボードフォーカスを保持し続ける**（`ui.memory_mut(|m| m.
request_focus(id))`等）。ユーザーが誤って別ウィジェットへフォーカスを
移してしまった場合は測定を一時停止し、「テキスト入力欄にフォーカスを
戻してください」と案内する（GJIはテキスト入力コンテキストが無いと
物理キーに反応しないため、フォーカスが外れた状態での測定は静かに
失敗し続ける——2026-09-16決着実験v2で実測確認済み）。

**受け入れ基準**: 手動UIテスト（awase-settingsを実機で起動し
一連のフローを確認）。テキスト欄からフォーカスを意図的に外した状態で
較正を試み、上記の案内が正しく表示されることも確認する。

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

**実装完了（2026-09-17）**: `src/config.rs`（`awase`本体、プラット
フォーム非依存）に`CalibrationEntry`構造体と`AppConfig::calibration:
Vec<CalibrationEntry>`フィールドを追加した。`KeysConfig`の`ime_on:
Vec<String>`等と同じ「Windows固有のenumは文字列で橋渡しする」パターンに
揃え、`ImeToggleKind`/`ImeKindId`/`ConfigFingerprint`は使わずSerialize/
Deserialize可能な`String`/`Option<i64>`/`Option<String>`/`Option<u64>`
のみで構成（`vk: VkCode`は`awase`本体で定義済みの型のためそのまま使用）。
`ValidatedConfig`/`From<ValidatedConfig> for AppConfig`にも
`keystroke_macro`と同型の単純転送で配線した（検証は行わない）。

`awase-windows`側（`state/calibrated_mode_key.rs`）に
`CalibratedModeKey::to_config_entry`/`calibrated_mode_key_from_config_
entry`の相互変換関数を追加し、6件のラウンドトリップテスト
（GJI/MS-IME双方の正常系、実際の`toml::to_string`/`from_str`を通した
シリアライズ、不正な`result`/`fingerprint_kind`文字列の拒否）を追加。
`cargo test -p awase-windows --lib`でLinux上で実行可能。

**未着手（T11のスコープ外、T12以降で対応）**: `Runtime.calibrated_
mode_keys`への起動時ロード・保存時の書き出し配線、および
`176-T9a`が確定結果を`set_calibrated_mode_key`へ実際に書き込む配線
（現状はログ・IPC通知のみで、確定してもメモリ上のマップにすら入らない）。
T0を前提条件から外した（上記T0節参照）ため、この配線自体は技術的には
着手可能——ただしこの配線を追加する際は、決定8が維持するopt-inの趣旨
（既定では較正結果を実際のIME判定へ反映しない）をどう実現するか
（設定ファイルに明示的なopt-inフラグを追加するか、当面は
`set_calibrated_mode_key`自体を呼ばないままにするか）を別途決めること。

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

**実装完了（2026-09-17、core部分）**: stale判定を実際のGJI/MS-IME同期
経路へ配線した。`176-T1`の`is_stale`をラップした
`fresh_or_none(record, current) -> Option<&CalibratedModeKey>`
（`state/calibrated_mode_key.rs`）を新設し、`apply_calibration_override`
へ渡す前に必ず通す:

- **GJI側**（`gji_charset_autodetect.rs::sync_gji_charset_autodetect`）:
  `ModeKeyCandidate::current_fingerprint`が、その時点で読んだ
  `config1.db`（`raw: GjiRawConfig`）から`ConfigFingerprint::Gji{
  session_keymap, relevant_row }`を都度再構築する。`relevant_row`は
  新設の`awase_gji_config::keymap::relevant_rows_for_vk(table, vk_name)`
  （`custom_keymap_table`から対象VKに対応する行だけを抽出・正規化して
  結合、複数行あればソート済みで結合。既存の`mozc_key_to_vk_name`を
  再利用するため`extract_ime_keys`と同じ「修飾キー付き行は対象外」扱い
  になる）。
- **MS-IME側**（`message_handlers.rs::sync_ime_toggle_auto_detect`）:
  新設の`msime_key_assignment::current_registry_fingerprint_hash(vk)`が
  `IsKeyAssignmentEnabled`/`KeyAssignmentMuhenkan`/`KeyAssignmentHenkan`
  の生のDWORD値（`read_delegate_to_open_axis_assignment_from_registry`が
  返す解釈済み値ではなく、未知の値も区別できる生値）から
  `ConfigFingerprint::MsIme{ registry_value_hash }`を都度計算する。

いずれも「設定リロード時」だけでなく、既存の同期呼び出し（GJI/MS-IME
確定のたびに再計算する`sync_gji_charset_autodetect`/`sync_ime_toggle_
auto_detect`自体の通常呼び出し）にも自然に乗る形にした——`reload_config`
専用の別経路を新設していない。

**未実装（実機A/B含む、次のタスク）**:
1. **`awase-settings`側のUI案内**（GJI再同期条件を満たさない場合の
   「対象アプリへフォーカスを戻してください」、`FindWindowW`失敗時の
   「次回起動時に反映されます」）は未着手。これらは較正結果が実際に
   ロード・保存・opt-in適用される一連の配線（下記2参照）が無いと
   ユーザーに見せる意味が無いため、その配線と合わせて実装するのが
   自然。
2. **実機A/B検証は未実施**。前提として、`Runtime.calibrated_mode_keys`
   への起動時ロード・実際の書き込み（`176-T9a`から`set_calibrated_
   mode_key`を呼ぶ配線、`176-T11`のギャップ節参照）がまだ無いため、
   現時点ではstale判定ロジックはユニットテストでのみ検証されており、
   実機では常に空のマップに対して動作する（＝実害ゼロだが、実機での
   staleフォールバック自体を観察することもできない）。この配線が
   入るまで受け入れ基準の実機確認は意味を持たない。

**依存**: 176-T3、176-T4、176-T11。

**最終配線（2026-09-17完了、上記「未実装」節を解消）**: T9a確定
（`ConfirmedOn`）が実際にconfig.tomlへ保存され、起動時・設定リロード時に
読み込まれ、opt-inフラグで実際のIME判定へ反映されるところまでの
エンドツーエンドの配線を完了した。

- **IPCペイロード拡張**（`calibration_ipc.rs`）: `CalibrationResultPayload`
  に`active_ime_kind: ImeKindId`を追加（wparamの32-47bit）。
  awase-settingsが`ConfirmedOn`を受けてどちら（GJI/MS-IME）の
  フィンガープリントを読み直すべきか判断するために必要。
- **T9a側**（`focus_tracking.rs::notify_calibration_result`）: 確定時点で
  `tsf::observer::tsf_obs().active_ime_kind()`を読み、ペイロードに含める。
- **新設`pub`関数**（`gji_charset_autodetect.rs::build_confirmed_
  calibration_entry(vk, active_ime_kind) -> Option<CalibrationEntry>`）:
  `awase-settings`（別クレート）から直接呼べる、awase-windows内で完結する
  唯一の関数。config1.db/レジストリを読み直してフィンガープリントを
  構築し、`CalibratedModeKey{ result: On, confirmed_at_epoch_ms: now,
  .. }.to_config_entry()`を返す。`VK_NONCONVERT`/`VK_CONVERT`以外は
  `None`（`apply_calibration_override`が消費するのはこの2キーのみ
  ——他のIME_MODE_KEY_OPTIONS候補（VK_KANJI等）を較正パネルUIで選んでも
  保存されない既知の制約、UIの選択肢自体は絞り込んでいない）。
- **awase-settings側**（`main.rs::persist_confirmed_calibration`、
  `tab_calibration`から`ConfirmedOn`受信時に呼ぶ）: **意図的に
  config.tomlを直接読み直して書く**（UIの編集中in-memory状態
  `self.config`は使わない）——ユーザーが他タブで未保存の編集をしている
  最中に較正が確定しても、その未保存編集を巻き込んで保存しないように
  するため。書き込み後`self.config.calibration`にも反映し（次に通常の
  保存操作をしてもこの較正結果が失われないように）、
  `send_reload_config_message()`でawase.exeへリロードを要求する。
- **opt-inフラグ**（`GeneralConfig::apply_calibrated_mode_keys`、
  既定`false`）: `tab_calibration`にチェックボックス
  「確定した較正結果を実際のIME判定に反映する（自己責任）」を追加。
  `Runtime::calibrated_mode_key_for`がこのフラグを見て、`false`なら
  config.tomlに保存されていても常に`None`を返す（決定8が求める
  opt-inの実体）。
- **起動時・リロード時ロード**（`Runtime::apply_config_update`から
  `reload_calibrated_mode_keys(&config.calibration)`を呼ぶ）:
  差分更新ではなく毎回`clear`してから`config.calibration`全体を
  再構築する（手動削除・置き換えが古い内容を残さないように）。
  パースできないエントリは警告ログでスキップし起動を落とさない。

**実機A/B検証完了（2026-09-17、dragonflyg4、T10の実UIで実施）**:
デバッグパッチ不要でT10の較正パネルUIから直接、以下の一連の流れを
実機確認した:

1. 「IMEキー較正」タブで無変換キーを選択し「較正開始」→物理キーを
   2回押下（各回IME ON状態でsettle window分フォーカス保持）→
   awase.exe側で`[calibration] 確定: vk=VkCode(29) ImeToggleKind::On`。
2. awase-settings.log側で`[calibration] 結果を受信: ...
   active_ime_kind=Gji`→`vk=VkCode(29)の較正結果をconfig.tomlへ
   保存しました`。実際の`config.toml`に
   `[[calibration]] vk=29 result="On" active_ime_kind="Gji"
   fingerprint_kind="Gji" gji_session_keymap=2`が書き込まれたことを
   確認（`gji_relevant_row`は該当行が無く省略、想定どおり）。
3. opt-inチェックボックスをONにして通常の保存操作→
   `apply_calibrated_mode_keys = true`が`config.toml`に反映され、
   `Config reloaded successfully`ログを確認（較正確定時のリロードと
   合わせて計2回のリロードが正しいタイミングで発火）。
4. **決定的な確認**: 別のテキストアプリでGJIのIMEをOFF（直接入力）に
   した状態で無変換キーを単独タップしたところ、**IMEがONになり
   NICOLAエンジンも正しく活性化した**（ユーザー実機確認）。
   session_keymap=2（MSIMEプリセット）の静的分類では無変換キーは
   `None`（割当てなし）のはずであり、この挙動変化は較正結果が
   `apply_calibration_override`経由で実際にIME判定を上書きしている
   ことの直接的な証拠である。

これにより176-T8〜T12の実装（物理キー検知→確定→config.toml永続化→
opt-in適用→実際のIME/エンジン制御への反映）がエンドツーエンドで
実機動作することを確認した。

**残る未実装**: `awase-settings`側のUI案内（GJI再同期条件を満たさない
場合の案内、`FindWindowW`失敗時の案内）のみ。優先度は低い
（無くても機能する、ユーザー体験の改善項目）。

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

---

**追記（2026-09-21、ADR-191）**: 上記の較正結果の**適用**（opt-inフラグ`GeneralConfig::apply_calibrated_mode_keys`、`Runtime::calibrated_mode_key_for`、設定画面のチェックボックス、
176-T11/T12の反映側）は、ADR-191の撤去で読む経路が無くなったため、撤去ブランチ（`feat/adr191-remove-hardcoded-mode-keys`）で削除した。較正の**測定・確定・IPC・`[[calibration]]`の保存**（T1〜T10、T11のスキーマ、T12のstale検出）は残す。
適用は、製品化（`awase-keymap-learn`、ADR-191の決定4）で、学習した表（最小のMealy機械）の実行時読み込みとして新規に作る。本ADRの上記の記述は、削除前の設計の履歴である。
