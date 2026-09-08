# ADR-156: 遅延実行キューの解放条件管理 — 観察記録と軽量な対策（将来構想、大規模統合は不採用）

## ステータス

**将来構想として起票、opus-adversarial-consult round1〜round2 で収束。**
round1 で検出した4件の Must-fix（うち中核の「3キュー統合」提案は根拠
不成立）を反映し、当初提案していた `DeferredExecutionQueue<T>` への
大規模統合は不採用とし、実際に裏付けが取れた範囲（`pending_deferred`
1キュー内の2窓口間の見落とし）に絞った軽量な対策のみを残した。
round2 でさらに1件の Must-fix（`fix-requires-evidence.md` へ追加した
行が「自動的に蓄積される」と書いていたが、実際に走る `.git/hooks/
pre-push` の正規表現には対象ファイルの一部が含まれておらず、自動化は
部分的にしか効いていなかった）と Should-fix 4件を検出・反映し収束。
着手条件2（合流点一覧表の維持）は `fix-requires-evidence.md` へ反映
済みだが、pre-push 側の regex 更新は未追跡ファイルのためユーザー同意
待ちのまま——「決定」節参照。

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
| `Executor::guard_held`（`ReinjectKey`） | OUTPUT_GUARD 期間中に OS へ再注入待ちの reinject 1件（出力側） | `runtime/executor.rs::drain_deferred` | `drain_deferred` 到達のたびに `guard_held.take()` して無条件に再試行し、guard をまだ通れなければ再び park する（「guard 解除で解放」ではなく「次の drain 到達ごとに再試行」） |
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

「訂正1」「訂正2」により、5つのキューは性質が異なり（`INPUT_DEFER`/
`deferred_engine_timers` の2つが入力側・エンジン前、`pending_deferred`
は出力側・TSF 固有の状態機械、`Executor::guard_held` は出力側の
OS 再注入待ち、`RuntimeOutbox` はさらに別種のコマンドキュー）、かつ
唯一の実例
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
3本運用している**（`Cargo.toml` の `[workspace.metadata.dylint]
libraries` — `lints/no_vk_as_scan`、`lints/ime_event_guard`、
`lints/observation_source_guard`、`.claude/rules/ime-belief-
architecture.md` 参照）。「defer/replay 経路からのライブグローバル
参照を禁止する」という規則は、まさに dylint が対象とする種類の意味
解析であり、テキスト走査ベースの `architecture_guard.rs` の守備範囲
ではない。**この規則自体に価値が無いわけではないが、実装するなら
dylint の4本目として設計すべきであり、本 ADR は決定として採用しない
（別 ADR の対象）。**

なお、この規則がそのまま実装されても、[ADR-155](155-timer-path-live-thumb-requery-during-deferred-timer-replay.md)
「訂正3」/「案B」が特定した根本原因（同一物理押下に対して
`hook.rs::now_timestamp()` が2回別々に呼ばれ、値がずれる）は検出でき
ない——`now_timestamp()` の2回呼び出しはどちらも「グローバル参照」では
なく単なる時刻取得であり、本規則の対象外である。パターン2の根治には
別の規則（同一イベントに対する時刻/状態の二重読み取りを禁止する）が
要る。「今後の議論」着手条件2 はこの区別を踏まえて読むこと。

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

**round2 で判明した限界（Must-fix）**: 表に行を足すだけでは「自動的に
蓄積される」ようにはならない。このリポジトリで実際に走る pre-push
フックは `core.hooksPath` が指す `.git/hooks/pre-push`（未追跡）で
あり、追跡下の `.githooks/pre-push` とは正規表現が乖離している
（別途セッションで確認済みの既知の未解決課題）。`.git/hooks/pre-push` の対象 regex を
確認したところ、今回追加した行が挙げるファイルのうち `output/vk_
send.rs`・`output/tsf_warmup_coord.rs`（`output/` に一致）・
`runtime/ime_coordinator.rs`・`runtime/executor.rs`（`runtime/
(ime_coordinator|...|executor)\.rs` に一致）は**既にカバーされている**
が、`input_defer.rs`・`runtime/message_handlers.rs`・
`runtime/outbox.rs` は regex に含まれておらず、**これらのファイルだけ
を変更する fix は pre-push の自動警告を受けない**。これは
`fix-requires-evidence.md` の「物理IMEキーのSuppress/Allow配送判断」
行が既に2度警告している「表には足したがフックの正規表現には入って
いない」穴を、本 ADR がもう1つ増やしていたことに気づいた記録である。

`.git/hooks/pre-push` は未追跡ファイルであり、regex の追加にはユーザー
の同意が要る（本 ADR 単独の判断で書き換えない）。したがって現時点の
着手条件1の観測は **`input_defer.rs`/`message_handlers.rs`/
`outbox.rs` の3ファイルについては手動での known-bugs.md 記録に依存
したまま**であり、「今後の議論」着手条件1（さらに1〜2件発見される）は
この3ファイルに関する限り自動化されていない。

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
   `docs/known-bugs.md` への記録を通じて行う（本版で運用開始済み。ただし
   `input_defer.rs`/`runtime/message_handlers.rs`/`runtime/outbox.rs` は
   「採用: `fix-requires-evidence.md` の再発ファミリー表に本ファミリーを
   追加する」節で確認したとおり `.git/hooks/pre-push` の
   自動警告の対象外であり、この3ファイルについては手動記録に依存する）。
2. パターン2（ライブグローバル参照、および「不採用（初版の決定3）」節の
   注記が特定した根本原因「同一イベントに対する時刻/状態の二重読み取り」
   の両方を数える）について、ADR-129/ADR-155 以外の箇所で同種の問題が
   もう1件見つかった場合、dylint 4本目としての「decision3」を別 ADR
   として起票する。

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
