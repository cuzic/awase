# ADR-156: 遅延実行キューの解放条件管理 — 観察記録と軽量な対策（将来構想、大規模統合は不採用）

## ステータス

**将来構想として起票、opus-adversarial-consult round1 で4件の Must-fix
（うち中核の「3キュー統合」提案は根拠不成立）を検出。当初提案していた
`DeferredExecutionQueue<T>` への大規模統合は不採用とし、実際に裏付けが
取れた範囲（`pending_deferred` 1キュー内の2窓口間の見落とし）に絞った
軽量な対策のみを残す。着手条件2（合流点一覧表の維持）は本版で
`fix-requires-evidence.md` へ実際に反映済み——「今後の議論」節参照。**

## 背景（round1 で訂正済みの事実関係）

初版は [ADR-121](121-explicit-physical-ime-key-idempotent-reassert.md)、
[ADR-123](123-focus-resync-and-probe-defer-queue-composition-race.md)、
[ADR-128](128-escape-composition-collateral-deferred-loss.md)、
[ADR-129](129-thumb-timestamp-live-requery-during-gate-drain-replay.md)
の4本を「同じ2つの構造パターンの独立した4インスタンス」として提示したが、
round1 レビューが実コードと突き合わせた結果、この前提は成立しなかった。

### 訂正1: 「3つの独立キュー」ではなく、少なくとも5つのキュー/待避機構があり、内訳も異なる

| キュー/機構 | 何を待避するか | 所有ファイル | 解放条件 |
|---|---|---|---|
| `INPUT_DEFER` | gate active 中のキーイベント全体 | `input_defer.rs`, `message_handlers.rs::handle_wm_drain_output_queue` | `OUTPUT_GATE.is_active() \|\| FOCUS_RESYNC.is_gate_active()` |
| `deferred_engine_timers` | 同じ gate active 中のエンジンタイマー | `runtime/ime_coordinator.rs`（フィールド）、push/replay とも `message_handlers.rs::handle_wm_timer`/`handle_wm_drain_output_queue` | `INPUT_DEFER` と**全く同じ gate 判定・同じ関数群**（別のキューだが解放条件は既に一本化済み） |
| `pending_deferred` | TSF probe/recovery 中に確定できない VK | `output/tsf_warmup_coord.rs`（データ）、`output/vk_send.rs`（`DeferGate`/`defer_respecting_gate`/`drain_pending_deferred_before_send_if_queue_only` の定義・解放条件本体）、`platform.rs`（`StartProbe` 時の `pending_deferred_len` 追い越し検出） | `has_pending_tsf()`/`raw_recovery_owns_deferred()`/`!pending_deferred.is_empty()` に加え `gate: DeferGate::Enforced/Exempt`（[ADR-123](123-focus-resync-and-probe-defer-queue-composition-race.md)/[ADR-128](128-escape-composition-collateral-deferred-loss.md)、**同一キューに対する独立した2つの窓口**——defer 側 `defer_respecting_gate` と drain 側 `drain_pending_deferred_before_send_if_queue_only`） |
| `Executor::guard_held`（`ReinjectKey`） | OUTPUT_GUARD 期間中の reinject 1件 | `runtime/executor.rs::drain_deferred` | output guard の解除 |
| `RuntimeOutbox` | `TIMER_TSF_PROBE` 等のタイマー命令 | `runtime/outbox.rs`、`Runtime::drain_runtime_requests` | `WM_EXECUTE_EFFECTS`/`WM_DRAIN_OUTPUT_QUEUE` 到達時 |

初版の表は `pending_deferred` の所有ファイルを `tsf_warmup_coord.rs` の
みとしていたが、解放条件の実体（`DeferGate` 型定義・`defer_respecting_
gate`・`drain_pending_deferred_before_send_if_queue_only`）は
`output/vk_send.rs` にある。「解放条件が散らばっている」という初版の
主張自体が、この誤帰属によって不明瞭になっていた。

### 訂正2（Must-fix、最重要）: パターン1の証拠は「3キュー間」ではなく「`pending_deferred` 1キュー内の2窓口間」

[ADR-123](123-focus-resync-and-probe-defer-queue-composition-race.md)→
[ADR-128](128-escape-composition-collateral-deferred-loss.md) の回帰
（`drain_pending_deferred_before_send_if_queue_only` が `gate` 引数を
見ずに無条件発火していた）は、**`pending_deferred` という単一キューの
中で、defer 側の窓口（`defer_respecting_gate`）と drain 側の窓口
（`drain_pending_deferred_before_send_if_queue_only`）という2つの
独立したエントリーポイントの片方だけに新条件を配線し忘れた**、という
1インスタンスの出来事である。`INPUT_DEFER`/`deferred_engine_timers` と
は無関係——この2つは「訂正1」の表が示すとおり、そもそも解放条件が
**既に1箇所**（同じ gate 判定・同じ関数群）に集約されている。

**この事実は、初版の決定1（3キューを統合すれば同種の見落としを防げる）
の根拠を直接崩す。** 3キューを1つの `DeferredExecutionQueue<T>` に
統合しても、`pending_deferred` 相当の中に依然として「defer 側」と
「drain 側」という2つの窓口が残る限り、統合後も片方への配線忘れは
同じ確率で起こりうる——統合が対策になっていない。

### 訂正3: パターン2は ADR-129/ADR-155 のみで、独立の第2インスタンスは無い

初版は「`modifier_snapshot` で解決済みのパターンが親指タイムスタンプに
未適用だった」ことを、時期の異なる2つの独立インスタンスであるかのように
書いていたが、実際には `modifier_snapshot` は同じ ADR-129 が引用する
**先例**であって、別インスタンスではない。加えて
[ADR-155](155-timer-path-live-thumb-requery-during-deferred-timer-replay.md)
の round1 レビューで、この問題の根はさらに深いことが判明した——
`RawKeyEvent.timestamp` とグローバル `LEFT/RIGHT_THUMB_DOWN_AT_US` は
**同一の物理キー押下に対して `hook.rs::now_timestamp()` が2回別々に
呼ばれる**ことに由来しており、「capture 時点でスナップショットを取る」
という一般化だけでは解決しない（ADR-155「案B」参照）。パターン2は
現時点で実質1件（ADR-129、未実装）であり、「繰り返し発生している
パターン」と呼ぶには時期尚早だった。

### 訂正4: ADR-121 はどちらのパターンにも該当しない

[ADR-121](121-explicit-physical-ime-key-idempotent-reassert.md) は
`kp_stage_shadow_ime_toggle` の無条件 no-op ガードの話であり、遅延
キューもライブグローバル読取も登場しない。初版が「4本の ADR」と
数えていたのは水増しで、正味は **パターン1が1件（`pending_deferred`
内）、パターン2が1件（ADR-129、未実装）** である。

## 決定（訂正版・大幅に縮小）

### 不採用（初版の決定1）: `DeferredExecutionQueue<T>` への統合

「訂正1」「訂正2」により、5つのキューは性質が異なり（3つは入力側・
エンジン前、`pending_deferred` は出力側・TSF 固有の状態機械、
`RuntimeOutbox` はさらに別種のコマンドキュー）、かつ唯一の実例
（ADR-123→ADR-128）は統合では防げない。共通の抽象型を新設するコスト
（型設計・全呼び出し元の移行・実機ソーク）に見合う効果が無いため、
大規模統合は不採用とする。

### 不採用（初版の決定3）: `architecture_guard.rs` への横断的ガード追加

「決定1の遅延実行機構経由で呼ばれるコードからグローバル状態を読む
関数を呼んではならない」という規則は、呼び出しグラフを辿る**推移的な
解析**を要求する。`architecture_guard.rs` は `fs::read_to_string` に
よるテキスト走査であり（ADR-155/ADR-129 のガードが機能したのは「1関数
×1ファイル」という単純なケースに限定していたからであって、一般化には
使えない）、この規則をテキスト走査で表現することはできない。

このリポジトリは**既に `cargo dylint` によるカスタム意味解析 lint を
2本運用している**（`lints/ime_event_guard`、`lints/observation_source_
guard`、`.claude/rules/ime-belief-architecture.md` 参照）。「defer/
replay 経路からのライブグローバル参照を禁止する」という規則は、まさに
dylint が対象とする種類の意味解析であり、テキスト走査ベースの
`architecture_guard.rs` の守備範囲ではない。**この規則自体に価値が
無いわけではないが、実装するなら dylint の3本目として設計すべきで
あり、本 ADR は決定として採用しない（別 ADR の対象）。**

### 採用: `fix-requires-evidence.md` の再発ファミリー表に本ファミリーを追加する（本版で実施済み）

唯一、コストゼロで即座に価値を出せる対策として、
`.claude/rules/fix-requires-evidence.md` の再発ファミリー表に
「defer/replay キューの解放条件」行を新設した（本 ADR のコミットに
同梱）。対象ファイル: `input_defer.rs`、`output/vk_send.rs`
（`DeferGate`/`defer_respecting_gate`/`drain_pending_deferred_before_
send_if_queue_only`）、`output/tsf_warmup_coord.rs`、
`runtime/message_handlers.rs::handle_wm_drain_output_queue`/
`handle_wm_timer`、`runtime/ime_coordinator.rs`、
`runtime/executor.rs::drain_deferred`、`runtime/outbox.rs`。

これにより「今後の議論」節の着手条件1（さらに1〜2件発見される）の
観測が known-bugs.md 側に自動的に蓄積されるようになる——初版は条件1を
「観測する仕組みが無いまま」放置していたため、事実上永久に満たされない
条件だった。

### 保留（将来、`pending_deferred` で2件目が起きた場合の候補）: 型レベルでの解放条件強制

Rust の型システムは「呼ぶべき場所で関数を呼び忘れた」ことを直接検出
できない（初版の「コンパイラがチェックできる形にする」という主張は
成立しない）。型で強制するとすれば、`take()`/`flush()` 系のメソッドが
`ReleaseToken<G>`（対応する解放条件を評価した関数からしか構築できない
トークン型）を要求する、といった具体的な API 設計が要る。これは
**キューの統合とは独立に、`pending_deferred` 単体へ単独導入できる**
——ただし「訂正2」の1インスタンスだけでは、この設計コストを正当化する
実例としてまだ不十分と判断し、今回は採用しない。`pending_deferred` の
defer/drain 窓口で同種の見落としが将来もう1件見つかった時点で、
このADRを更新して再検討する。

## 検討した代替案

### 代替案A（現状維持）: 個別修正を今後も都度積み重ねる

「採用」節の合流点一覧表の追加以外、追加のコストを払わない。訂正2の
とおり実例が1件のみである以上、これが現時点で最も費用対効果が高い。

### 代替案B（不採用と確定）: [ADR-152](152-keystroke-step-source-sink-pipeline.md)
の `KeyStrokeStep`/`StepOwnership` に統合する

初版は「タイミングの軸と所有権の軸は直交する別の関心事」と主張したが、
round1 レビューが ADR-152 決定3 の内容を確認したところ、この主張は
成立しない。ADR-152 決定3 は `KeyStrokeStepDispatcher::dispatch` を
`RomajiOutput` sink の実行窓口にすると明記しており、`pending_deferred`
はまさにこの `RomajiOutput`（`DeferredVk`）を退避するキューである——
ADR-152 が実装されれば `pending_deferred` の defer/drain は
`execute_sink` の内部に取り込まれ、本 ADR が扱ってきた「解放条件」は
`StepOwnership::resolve()` の一部として再設計されることになる。加えて
ADR-152 決定3 は `transport.rs::PhysicalKeyDisposition::plan`
（`INPUT_DEFER` への退避判断とも接する）も `StepOwnership::resolve()`
に寄せると書いている。**「直交・独立」ではなく「後から着手する方が
先行実装の型を作り直す責任を負う」関係にある。** ADR-152 は
[ADR-151](151-actuation-delegate-by-default-drift-scoped-to-ownership.md)
の force-ON 救済 Blocker で着手できないままだが、この事実は「本 ADR が
ADR-152 と衝突しない」ことの証明にはならない——単に両者とも現時点で
着手していないだけである。本 ADR は現時点で決定1（統合）自体を不採用
としたため、この衝突は実害を生まないが、将来型レベルの対策（保留節）
を検討する際は ADR-152 の動向を確認すること。

## 今後の議論

**着手条件（訂正版）:**

1. `pending_deferred` の defer/drain 窓口間で、ADR-123→ADR-128 と同種の
   見落としがもう1件見つかった場合、「保留」節の型レベル対策（`ReleaseToken<G>`
   等）を再検討する。観測は `fix-requires-evidence.md` の新設行と
   `docs/known-bugs.md` への記録を通じて行う（本版で運用開始済み）。
2. パターン2（ライブグローバル参照）についても、ADR-129/ADR-155 以外の
   箇所で同種の問題がもう1件見つかった場合、dylint 3本目としての
   「decision3」を別 ADR として起票する。

## 関連

[ADR-121](121-explicit-physical-ime-key-idempotent-reassert.md)（当初
本ADRの動機に含めていたが該当しないと訂正）、
[ADR-123](123-focus-resync-and-probe-defer-queue-composition-race.md)、
[ADR-128](128-escape-composition-collateral-deferred-loss.md)
（`pending_deferred` 内2窓口間の唯一の実例）、
[ADR-129](129-thumb-timestamp-live-requery-during-gate-drain-replay.md)、
[ADR-155](155-timer-path-live-thumb-requery-during-deferred-timer-replay.md)
（パターン2、根本原因は「capture 時点」の一般化だけでは閉じないと
判明——案B参照）、
[ADR-152](152-keystroke-step-source-sink-pipeline.md)（`pending_deferred`
と重なる領域、直交ではなく将来の合流責任がある）、
`.claude/rules/fix-requires-evidence.md`
の再発ファミリー表（本 ADR で「defer/replay キューの解放条件」行を追加）、
`.claude/rules/ime-belief-architecture.md`（既存の dylint 運用の先例）。
