---
id: ADR-171
title: |-
  GJI候補ウィンドウの意図しない再表示を正式な観測として belief に流し、既存drift correctionで自動補正する
status: |-
  起草・opus-adversarial-consult round1反映済み。round1でBlocker4件・Major4件・
  Minor6件を指摘され、決定1〜4を全面的に書き直した（当初案は`AnyObservation`が
  witness経由専用で構築不能・`platform.rs`にbelief経路が無い・BUG-114型の
  無限再武装・確定した観測が二度と訂正されない、という4つのBlockerを持っていた）。
  round2レビュー待ち。実装未着手。
related_adr:
  - "ADR-034"
  - "ADR-080"
  - "ADR-087"
  - "ADR-089"
  - "ADR-090"
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

**[BUG-033](../known-bugs/BUG-033.md)が既にこの設計を検討し、見送っていた
ことを round1 で指摘された。** BUG-033の「検討したが見送った案」は
「belief/drift-correction を経由する設計（`ObservationSource`新設variantで
`ObserverReported`をdispatchし、`check_drift_correction`に本物の観測を
与える）。コード調査で実現可能と確認済みだったが、(a) 補正閾値のレイテンシ、
(b) 新規`ObservationSource`variant追加のコストという理由で見送った。GJIが
タイピング中でなく長時間OFFのまま乖離するケースにはBUG-033が採った直接
呼び出し（`send_chrome_gji_reinit_and_poll`）は効かないため、将来そのような
ケースが実機で確認されたら、この belief 経由の設計を別バグとして再検討する
こと」——**ADR-171はBUG-033が予約していたこの再検討の実行にあたる**。
(a)(b)のコストは既に承知した上で、BUG-141という具体的な実害を前に belief
経由を選ぶ、という位置づけを明示する。

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
`EVENT_OBJECT_SHOW`が起点）が来ても、ログを警告として出すだけで、GJIの
実状態への訂正コマンドも、awaseのbeliefへのフィードバックも一切発行しない。

**round1指摘（M2）を受けた重要な事実**: `GjiFsm::on_event(StartComposition)`
自体が呼ばれる契機（`gji_on_start_composition`、`platform.rs:889-899`）は
`drain_pending_composition_events`（同`:923-930`）のみで、その呼び出し元は
`advance_tsf_probe`（TSFプローブタイマー中）と`drain_output_post_send_effects`
（`send_keys`/`flush_raw_tsf_literal_recovery`直後、＝awase自身が実際に出力を
送るとき）の3箇所に限られる。**IME OFFで engine が inactive のときは
`send_keys`経路が通らない**ため、本ADRが対象とする状況（IME OFFのはずなのに
候補が開く）はまさに既存のdrain契機が最も乏しい状況である。BUG-141で
journalに`GjiFsmTransition{StartComposition}`が現れたのはTSFプローブタイマー
がたまたま走っていたためと推定されるが、SHOW発生から実際にFSMへ届くまでの
遅延は未計測。**この問題は決定2で「GjiFsmの外側」に新しいdrainルートを
作ることで解消する（詳細は決定2）。**

## 決定

### 決定1: 新しい evidence 型を「宣言だけ」でなく5点セットで追加する

`ObservationSource`（`state/ime_event.rs`）に新variant
`GjiCandidateShownWhileClosed`（仮名）を追加するだけでは、`AnyObservation`
（`state/evidence.rs:269-330`）を作れない——同型は全フィールドprivateで、
`Observed<E>`のwitness経由の`From`変換、または本番禁止の
`restored_from_journal`（`tests/architecture_guard.rs::
any_observation_replay_door_is_not_used_in_production`が本番0件を固定）
経由でしか構築できない。したがって以下5点をセットで実装する
（`ConvOpenInference`が全部やっている前例をなぞる、`state/evidence.rs`）:

1. `ObservationSource`に variant 追加 + `authority()`のmatchアーム追加
   （`state/ime_event.rs`）。**`authority(): Actuating`**
   （drift correctionの補正根拠として使える）。
2. `state/evidence.rs::declare_evidence!`に1行追加:
   `GjiCandidateShownWhileClosed => ActuatingPool, GjiCandidateShownWhileClosed, Medium;`
   **confidence は `Medium`（`High`ではない、round1 m2指摘）**——
   `ObservationConfidence`の定義（`state/ime_event.rs:210-219`、
   High=「直接API成功」、Medium=「間接観測(GJI/TSF observer)」）に照らすと、
   「候補ウィンドウが表示された」という間接的なUIイベントはMedium相当
   （`GjiIoInference`と同じ区分）。Mediumを選ぶことは決定3・決定4の設計にも
   効いてくる（後述）。
3. `Observed<GjiCandidateShownWhileClosed>`専用のwitness構築子を追加する。
   引数として要求する「外部事実」は、決定2で新設する
   `CandidateShowFact { tick_ms, front_hwnd, gji_idle_ms }`
   （SHOW発生時点で実際に捕捉した値、`AtomicBool`のような裸のフラグではない
   ——round1 M1指摘）。`hwnd`/`focus_epoch`は、drain時点で
   `CandidateShowFact.front_hwnd`と現在のフォーカスhwnd/fenceを照合し、
   一致する場合のみ現在のfocus_epochを使って構築する（不一致なら構築せず
   捨てる、後述）。
4. `PerSourceObservations`に10個目のフィールド＋`get`/`set`/`iter`/
   `clear_all`のアーム追加（`state/observation_store.rs:240-330`）。
5. `state/evidence.rs`の`evidence_sources_are_nine_distinct_recordable_sources`
   を10に更新し、`ObservationSource`を列挙する全数テスト
   （`state/ime_event.rs:639-716`等）にもアームを追加する。

書き込み口は`ImeStateHub`の designated メソッド
（`state/platform_state.rs::report_conv_open_inference`と同型、新設）に
閉じ、`architecture_guard.rs`の既存ガード
（`focus_probe_observation_is_limited_to_real_probe_path`等）と同種の
「この観測は指定関数以外から構築できない」固定テストを追加する。

**既存の未使用スロット`ObservationSource::Gji`（「GJI (GetGuiThreadInfo)
由来」）は再利用しない**——round1 m1で「命名の正直さだけでなく、
`GetGuiThreadInfo`ベースAPIとWinEventHookベースの`EVENT_OBJECT_SHOW`は
実装上まったく別の観測経路であり、将来`GetGuiThreadInfo`由来の観測を
実装する際にこのvariantが本来の意味で必要になる」という理由を明記する。

### 決定2: `GjiFsm`を経由せず、WinEventHookのSHOWハンドラから直接事実を捕捉し、Runtime側の周期tickで drain する

round1 B2（`platform.rs`は`platform_state`を持たずbeliefに届かない）と
M2（drain契機がIME OFF時に乏しい）を同時に解決するため、**`GjiFsm`/
`GjiAction`は一切変更しない**。代わりに、以下の別経路を新設する。

1. `tsf/win_event_obs.rs::observation_event_proc`の`EVENT_OBJECT_SHOW`
   ハンドラ（`:154-176`）で、`GJI_CANDIDATE_CLASS`一致時に現状セットしている
   `pending_start_composition`（既存、`GjiFsm`用、変更しない）に加えて、
   **新しい`pending_candidate_show_fact: Mutex<Option<CandidateShowFact>>`**
   （`TSF_OBS`内、既存の`gji_candidate_visible`等と同じ場所）に
   `CandidateShowFact { tick_ms: crate::hook::current_tick_ms(), front_hwnd:
   <このコールバック内で取得できる現在のフォアグラウンドhwnd>, gji_idle_ms:
   <取得可能なら> }`を格納する（既に値があれば上書き、最新のSHOWのみ保持）。
2. `runtime/ime_refresh.rs::ir_stage_notify`（`TIMER_IME_REFRESH`、
   20/50/500ms間隔で**engineがuser_enabledである限りIME open/close状態に
   関わらず周期的に呼ばれる**、`ir_apply_drift_correction`自身の早期return
   条件`!self.engine.is_user_enabled()`とは独立）に、`ir_apply_drift_correction`
   呼び出しの**直前**に新しいステージ`ir_drain_candidate_show_fact()`を追加する。
   このステージは`platform.take_pending_candidate_show_fact()`
   （既存の`output.take_composition_reset()`と同型のtake口を`platform.rs`に
   新設）でfactを取り出し、
   - `GjiFsm`の現在状態が`OffCold`でなければ捨てる（IME ONなら候補SHOWは
     正常なので観測不要）、
   - `fact.front_hwnd`が現在のフォーカスhwnd（`self.platform.focus.
     current_hwnd()`）と一致しなければ捨てる（**round1 M1の
     Notepad→Edge誤帰属シナリオ対策**——別ウィンドウでのSHOWを現在の
     フォーカス窓の観測として誤って取り込まない）、
   - 両方通れば`self.platform_state.ime.report_gji_candidate_shown_while_off
     (&fact, self.focus_epoch(), hwnd)`（決定1の designated メソッド）を呼ぶ。

この経路は`platform.rs`にbeliefへの依存を持ち込まず（B2解消）、
`send_keys`に依存しない周期的なdrainを持つ（M2解消）。GjiFsm自身の
状態遷移・純粋性契約（`gji_fsm.rs:295-304`）も一切変更しないため、round1
m3が指摘した既存テスト（`startup_ime_on_sync_allows_candidate_show_to_
warm_fsm`）への影響もない。

### 決定3: HIDE（`EndComposition`）方向には対応する`open:false`観測を発行しない。ただし confidence を Medium にすることで「永久に焼き付く」リスクを構造的に緩和する

候補ウィンドウが閉じる（`EVENT_OBJECT_HIDE`起点）ことは「変換候補が確定
した」ことを意味するだけで「IMEがOFFになった」ことを意味しないため、
この観測は**SHOW方向のみの一方通行**とする（変更なし）。

**round1 B4（Blocker）への対応**: `most_recent_trusted()`は
confidence優先・`at`が第2キーで選ばれ（`observation_store.rs:694-699`）、
`ObserverReported`由来の観測は`expires_at: None`（時間で消えない、
同`:451-476`）。決定1で`confidence: High`を選んでいた当初案では、
`Imm32Unavailable`プロファイルで**この観測を上書きできるMedium/Low観測が
構造的に存在しない**（Highソースはこのプロファイルでは読めない）ため、
一度発火すると同一フォーカスセッション中ずっと`open:true`が勝ち続け、
実際にOFFへ補正された後もbeliefが古い`open:true`を拾い続けるリスクが
あった（`FocusChanged`＝別プロセスへの移動でのみ`clear_on_focus_change`が
発火し消える）。

**decision: confidenceをMediumにする**（決定1で確定済み）ことで、
`Imm32Unavailable`で実際に周期的に record される`ObserverPoll`(Medium)が、
より新しい`at`を持つ限り`most_recent_trusted()`で自然に上書きする
（confidence同値はatで比較されるため）。500ms周期の`ObserverPoll`が
実IMEの状態を正しく観測し続ける限り、この新観測は「一時的に不整合を
検知してdrift correctionを起こすトリガー」として機能し、その後は自然に
薄れる。**この選択がdecision4のexclusion/latch設計（後述）と整合すること
を実装時のテストで確認する。**

### 決定4: 補正の実行は既存のdrift correction機構に委譲するが、そのままでは BUG-114 の無限再武装を再現するため、2点の追加ガードを同時に導入する

決定1・2で`ObserverReported{open:true, Medium}`が記録されると、
`PlatformState::check_drift_correction`（`platform_state.rs:911`）が
`desired`との不一致を検知する。

**round1 B3（Blocker）への対応——`AnyFreshEvidence`除外リストへの追加**:
`Blind`policyの`IME_ACTUATION_BLIND_MAX_ATTEMPTS`は「1 actuationあたり
5回」の上限であり「全体で5回」ではない。`GiveUp`後は
`DRIFT_CORRECTION_BLIND_REARM_COOLDOWN_MS`（3000ms、`tuning.rs:322`）経過後、
`read_back(.., ReadBackQuery::AnyFreshEvidence, ..)`が「`gave_up_at`以降に
新しいtrusted観測が record されたか（値不問）」を見て再武装する
（`runtime/ime_refresh.rs:703-786`）。除外されているのは現在
`ObserverPoll`/`ConvOpenInference`の2ソースのみ（`observation_store.rs:
667-670`、理由はBUG-114そのもの——読み戻し手段が構造的に無いプロファイルで
自己確認しない弱い代理指標が3秒おきに永久バーストを再武装する）。
新ソースは全く同じ性質（`desired`が実現したかを一切確認していない）を
持つため、**`EXCLUDED_FROM_ANY_FRESH_EVIDENCE`に本ソースを追加することを
決定とする**。これにより「BUG-141相当のセッションでVK_IME_OFFが最低5回・
3秒境界を跨げば10回、awase側から自動送信される」という定量化されたリスク
（round1 B3）を防ぐ。

**round1 B3が指摘した episode ラッチの追加**: `decide_conv_inference_drift`
（`state/ime_actuation.rs:492`）は現状`ObservationSource::ConvOpenInference`
のみをハードコードで対象にした「同一の明示意図エピソード中は実送信1回に
絞る」ラッチである。この`matches!`条件に本ソースも追加し、同じ
episode-latch（`ConvDriftEpisode { intent_at_ms, desired }`が同一なら
`Suppress`）を適用する。**新しいtuning定数・新しいクールダウンは追加しない**
——既存の型的保証（BUG-113/BUG-43対策）をそのまま再利用する
（`.claude/rules/tuning-constants.md`の実測義務に触れない）。

**round1 M3（Major）への対応——「閾値0」の前提条件を正確に書く**:
`check_drift_correction`が閾値0（即時）になるには実際には5条件
（`platform_state.rs:911-1000`）が必要であり、うち
`DRIFT_CORRECTION_OBS_MAX_AGE_MS`（1500ms、`tuning.rs:259`）を超えた
観測は使われない。決定2の新しいdrainルートは`TIMER_IME_REFRESH`の
最短周期（20ms）に乗るため、`send_keys`依存の旧経路よりこの1500ms
上限に対して十分な余裕がある——ただし**実装時にSHOW発生からdrainまでの
実測レイテンシを記録すること**（round1 M2の要求と同一）。

また、`explicit_intent == desired`が成立しない場合（BUG-141の3回目の
失敗のように、物理半角/全角キーで`last_intent`がON側に切り替わった直後）
は、`trusted.open(true) == desired(true)`となり`check_drift_correction`は
そもそも`None`を返す——これは**正しい挙動**（ユーザーの新しい明示意図と
観測が一致しているので補正不要）であり、本設計が「救えない」ケースでは
ない。ADRの当初案がこの点を「救う」ように読めた記述だったことを訂正する。

`platform_state.rs:982-988`の「明示意図が無い間は`ConvOpenInference`/
`HeuristicDefault`単独で発火させない」ガードの`matches!`には、**本ソースを
追加しない**ことを決定として明記する——本ソースは自己生成の間接推測
（conv bit推測・観測ゼロの安全デフォルト）ではなく、実際にOS上で起きた
UIイベントに基づく genuine な外部証拠であり、明示意図がない状態
（例: フォーカス変更直後のenforce-off直後に候補が開いた）でも補正すべき
という判断を明示的に選ぶ。

## この設計が対応する既存リスク（更新版）

- **BUG-113（TSF composition破壊、5連射の危険性）**: 決定4のepisode
  ラッチにより、同一の明示意図エピソード中は実送信を1回に絞る。BUG-141の
  実際の再現パターン（Ctrl+無変換1回目〜3回目の間に候補SHOWが3回、
  約6.4秒）でも、各Ctrl+無変換ごとに新しいエピソードが始まるため、
  最悪でも「Ctrl+無変換の回数×最大5連射」に留まり、無限再武装
  （B3が指摘した5〜10回の自動送信）は起きない。
- **確定した観測が二度と訂正されない（B4）**: 決定3でconfidenceをMedium
  にしたことで、後続の`ObserverPoll`が自然に上書きする経路を確保した。
- **フォーカス変更中のスプリアス発火**: 決定2のhwnd照合（drain時点の
  フォーカスとSHOW捕捉時点のフォーカスの一致確認）により、
  Notepad→Edge型の誤帰属（round1 M1）を防ぐ。`focus_settle_ms`との
  相互作用は実装時に確認する。

## 未解決の論点・残存リスク

1. **（round1 M4、実機検証待ち）**: 本設計の唯一の効能根拠は「候補SHOW後に
   送るVK_IME_OFF（またはdrift correctionによる再送）が、Ctrl+無変換1回目の
   送信より高い確率で実際に効く」という未検証の前提である。BUG-141の
   根本原因仮説（Mozcの`OnSetFocus`が refocus のたびに無条件上書きする）が
   正しいなら、**候補SHOW後の再送もまた次のrefocusで同様に上書きされうる**
   ——その場合この設計は「ユーザーが手で3回送っていたのをawaseが自動で
   数回送るだけ」になり、収束を早める効果はあっても根治しない可能性が
   ある。BUG-141「次のアクション」の実機A/B
   （idle 5秒以上放置→Ctrl+無変換→(a)そのまま入力/(b)別入力欄クリック後に
   入力、および「候補SHOW直後に手動でCtrl+無変換をもう一度送ると直るか」）
   を、**実装前またはdevelopマージ前に実施し、結果をこのADRに追記すること**。
   効かないと判明した場合は、VK_IME_OFF（冪等キー）ではなくVK_KANJI
   （トグルキー、Mozcの別処理経路を通る可能性がある）を試す等、送信内容
   そのものの見直しに設計を戻す。
2. **SHOW→drain実測レイテンシ**: 決定2の新しいdrainルート導入後、実際の
   レイテンシを実機ログで確認し、1500ms上限に対して十分な余裕があるかを
   記録する。
3. **`CandidateShowFact`の`gji_idle_ms`取得可否**: `EVENT_OBJECT_SHOW`
   ハンドラのコールバックコンテキストで`gji_idle_ms`相当の値を安全に
   取得できるか（`GjiFsm`が保持する値へのアクセス経路）は実装時に確認する
   （取得できない場合は`tick_ms`/`front_hwnd`のみで妥協する）。

## 実装ノート

未着手。round2レビューで収束後に着手する。最低限の回帰テスト
（`.claude/rules/fix-requires-evidence.md`）:
「新観測1件で5連射しないこと」「give-up後に同じ観測source単独では
再武装しないこと」の2本を`state/platform_state.rs`の既存テスト
（`check_drift_correction_fires_immediately_when_explicit_off_intent_
conflicts_with_conv_inference`等）を雛形にLinux上で追加する。
