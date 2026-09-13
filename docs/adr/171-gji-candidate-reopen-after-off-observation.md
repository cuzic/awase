---
id: ADR-171
title: |-
  GJI候補ウィンドウの意図しない再表示を正式な観測として belief に流し、既存drift correctionで自動補正する
status: |-
  起草。opus-adversarial-consult未実施。実装前。
related_adr:
  - "ADR-034"
  - "ADR-080"
  - "ADR-087"
  - "ADR-140"
---

# ADR-171: GJI候補ウィンドウの意図しない再表示を正式な観測として belief に流し、既存drift correctionで自動補正する

## 背景

[BUG-141](../known-bugs/BUG-141.md)（report `01M2D8HS5SBWSZ221P240Z4ZXE`）で、
Ctrl+無変換（`GjiDirectStrategy`が送る`VK_IME_OFF`、`ime_controller.rs:75-110`）
を3回送っても、直後の英字入力でGJIの変換候補ウィンドウ
（`GoogleJapaneseInputCandidateWindow`）が3回連続で実際に再表示される
現象が発生した。journal上は3回とも`ActuationDecision.outcome:Applied`/
`AlreadyMatched`で「成功」と記録されており、awase自身のbelief/FSM
（`GjiFsm`、`OffCold`状態）は正しくOFFのまま一貫していた。

Mozc（[`tip_input_mode_manager.cc`](https://github.com/google/mozc/blob/master/src/win32/tip/tip_input_mode_manager.cc)）
と Chromium（[`tsf_bridge.cc`](https://chromium.googlesource.com/chromium/src/+/9ff13081c1766eac6edc574d3af12e72519e8091/ui/base/ime/win/tsf_bridge.cc)）
の公開ソースを確認した結果（BUG-141参照、WebFetch要約に基づく未確証の仮説
段階）、以下の機序が疑われる:

- Mozcの`TipInputModeManager::OnSetFocus(bool system_open_close_mode, ...)`は、
  TSFフォーカスが再通知されるたびに`tsf_state_.open_close`を呼び出し元の
  `system_open_close_mode`で**無条件に上書き**する。`VK_IME_OFF`等の明示
  コマンドは別経路（`OnReceiveCommand`）で反映されるが、その後
  `OnSetFocus`が再度呼ばれると上書きされて消える。
- Chromiumの`TSFBridge`は、テキスト入力欄の種別ごとに別々の`ITfDocumentMgr`
  を使い分け、フォーカス対象クライアントの変更や入力欄種別の変化のたびに
  `AssociateFocus()`でフォーカス関連付けを再実行する（コード内コメント
  「一部のIMEはdocument focusの変化がないと状態を更新しない」という設計
  意図の記述あり）。

この機序が事実だとしても、**Mozc/Chromiumはawaseの管理下にない外部の
オープンソースプロジェクトであり、awase側から直接修正することはできない**。
また、この機序（プロセス内部のTSFフォーカス再関連付け）はawaseの既存の
フォーカス計装（`FocusTransition`、OS/hwndレベル）では観測できない
（BUG-141「重要な限界」節）。したがって根治ではなく、**症状（候補ウィンドウ
の意図しない再表示）を検知して自動的に訂正する**という対症的だが確実な
アプローチを採る。

## 現状の問題点

`crates/awase-windows/src/tsf/gji_fsm.rs:885-887`:

```rust
GjiState::OffCold => {
    tracing::warn!("[gji-fsm] StartComposition while engine off — ignored");
    Response::consume()
}
```

`GjiFsm`が`OffCold`（awase自身のbeliefは「IME OFF」）のときに
`StartComposition`（候補ウィンドウの実際のSHOW、`observer.rs`の
`EVENT_OBJECT_SHOW`が起点）が来ても、**ログを警告として出すだけで、
GJIの実状態への訂正コマンドも、awaseのbeliefへのフィードバックも
一切発行しない**。これは「awase自身のFSM状態を誤って壊さない」という
意味では正しい設計だが、実際にGJIが候補ウィンドウを開いてしまった
（＝ユーザー視点でIME OFFが機能していない）という事実に対して**何も
対応しない**という副作用がある。

## 決定

### 決定1: 新しい `ObservationSource` variant を追加する

`state/ime_event.rs::ObservationSource`に、候補ウィンドウの実際のSHOW
イベントを表す専用variantを追加する（暫定名 `GjiCandidateWindowShown`、
命名は実装時に確定）。

既存の`ObservationSource::Gji`（「GJI (GetGuiThreadInfo) 由来」、
現状production未使用）は**再利用しない**——`.claude/rules/
ime-belief-architecture.md`の「`ObservationSource`が『何を観測したか』を
正直に命名する」原則に反する（`GetGuiThreadInfo`とWinEventHookの
`EVENT_OBJECT_SHOW`は別の観測経路）。

- `confidence: ObservationConfidence::High`（実際にOS上で候補ウィンドウが
  表示された、という直接的なUIイベントであり、間接推測ではない）
- `authority(): ObservationAuthority::Actuating`（drift correctionの
  補正根拠として使える——`ObserverPoll`/`Tsf`と同じ扱い）

### 決定2: `GjiFsm::OffCold`中の`StartComposition`から新しい`GjiAction`を発行する

`GjiAction`（`gji_fsm.rs:255`）に新variant（暫定名
`ReportCandidateWindowWhileOff`）を追加し、`OffCold`中の
`StartComposition`ハンドラから`Response::emit(vec![GjiAction::
ReportCandidateWindowWhileOff])`を返す（既存のFSM状態自体は変更しない
——`OffCold→OffCold`のまま）。

`GjiFsm`自身は`ImeModel`/beliefに直接書き込まない（timed-fsmパターンの
純粋関数性を保つ）。`platform.rs`の既存`match action`ディスパッチャ
（`GjiAction::StartProbe`/`CancelProbe`等と同じ場所、`platform.rs:537-611`
付近）に新しい分岐を追加し、そこで

```rust
self.platform_state.ime.dispatch_event(
    ImeEvent::ObserverReported(AnyObservation {
        open: true,
        source: ObservationSource::GjiCandidateWindowShown, // 仮名
        confidence: ObservationConfidence::High,
        at: tick_ms,
    }),
    tick_ms,
);
```

を呼ぶ。これは`.claude/rules/ime-belief-architecture.md`が定める
「Observe → 純粋 classify_\* → `reduce()`」の三層分離に沿う——GJI候補
ウィンドウのSHOWという生観測を、既存の`ObserverReported`経路で正直に
報告するだけで、`desired_open`を直接書き換えたり意図を偽装したりしない。

### 決定3: HIDE（`EndComposition`）方向には対応する`open:false`観測を発行しない

候補ウィンドウが閉じる（`EVENT_OBJECT_HIDE`起点の`EndComposition`）
ことは「変換候補が確定した」ことを意味するだけで、「IMEがOFFになった」
ことを意味しない（確定後もIMEは開いたまま次の単語を待つのが通常の動作）。
`ConvBitsInference`/`GjiIoInference`が「一方向のみの推論」として設計
されている前例（`ime_event.rs`のdoc comment参照）に倣い、**この観測は
SHOW方向のみの一方通行**とする。HIDE時に`open:false`を報告すると、
通常の変換確定のたびに誤ったOFF観測を注入することになり、真にOFFへの
補正が必要な場面との区別がつかなくなる。

### 決定4: 補正の実行は既存のdrift correction機構に委譲し、新しい再送ロジックは書かない

決定1・2で`ObserverReported{open:true, High}`が`observations`ストアに
記録されると、既存の`PlatformState::check_drift_correction`
（`platform_state.rs:911`）が`desired(false)`と`most_recent_trusted()`
の不一致を検知する。今回のシナリオでは直前にCtrl+無変換による
`UserImeSetIntent{Command}`が`last_intent`にセットされており
`is_strong_intent=true`かつ`explicit_intent==desired`のため、
**補正閾値は`0`（即時）**になる（`check_drift_correction`の該当分岐、
`platform_state.rs:922-927`）。

実際の再送は`ir_apply_drift_correction`（`runtime/ime_refresh.rs:584`）
に委譲し、既存の`Actuation`型トランザクション（ADR-080）・`Blind`
policyの`IME_ACTUATION_BLIND_MAX_ATTEMPTS`上限（ADR-140、BUG-113の
「5連射で強制TSF composition破壊」を防ぐための既存ガード）をそのまま
継承する。**新しい再送ロジック・新しいクールダウン定数は追加しない**
——BUG-43が「実送信の結果がobservation storeにフィードバックされず
無限再送するタイトループ」を防ぐために既に整備した型的保証を再利用する。

## この設計が対応する既存リスク

- **BUG-113（TSF composition破壊、5連射の危険性）**: 決定4により既存の
  `Blind` policy上限をそのまま継承するため、この設計が新たに無制限
  バーストを生む経路にはならない。ただし「候補ウィンドウが再表示される
  たびに1回のBlindバースト（最大5連射）が発生する」ことは変わらず、
  BUG-141の再現パターン（3回連続で候補が再表示された）のように**短時間に
  何度も再表示される**場合、Blindバーストが短時間に複数回起動しうる。
  この頻度がGJIのTSF composition破壊リスクを新たに高めないか、opus
  レビューで要検証。
- **GjiFsmの状態遷移の純粋性**: 決定2は`OffCold→OffCold`のまま状態を
  変えないため、`.claude/rules/experiment-logging.md`が警告する
  「1回直しただけでは再発する」類の合流点漏れにはならないはずだが、
  `GjiAction`を処理する`match`が`platform.rs`の1箇所のみか確認要
  （`docs/layer-boundaries.md`のカテゴリ確認）。
- **フォーカス変更中のスプリアス発火**: `StartComposition`が
  `FocusChange`直後の一瞬（GJIの内部的な初期化タイミング）に紛れて
  発火した場合、実際には問題ないタイミングでdrift correctionの即時
  再送を誘発しないか。既存の`focus_settle_ms`/settle機構との相互作用を
  確認する必要がある。

## 未解決の論点（opus-adversarial-consultで検証すべき点）

1. `ObservationSource`の新variant名（命名の正確さ、既存4variantの
   `authority()`分類との整合性）。
2. 決定3（HIDE方向は非対称に無視する）の判断が本当に安全か——GJIが
   候補ウィンドウを開いたまま長時間ユーザーが放置し、その後何らかの
   理由で「実は閉じてOFFに戻った」場合の観測経路が他に存在するか。
3. Blindバーストの頻度制御（上記リスク節）——BUG-141の実際の再現
   パターン（8秒間で3回の候補再表示）に対して既存policyのクールダウン
   設計が十分か、実測が必要か（`.claude/rules/tuning-constants.md`）。
4. 実機再現待ちのまま設計を確定してよいか、それとも先に実機で
   「フォーカス移動を挟むと再現率が上がる」仮説を検証すべきか
   （BUG-141「次のアクション」参照）。

## 実装ノート

未着手。opus-adversarial-consultでの収束後に着手する。
