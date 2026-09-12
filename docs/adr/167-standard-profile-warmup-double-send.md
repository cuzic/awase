---
id: ADR-167
title: |-
  Standardプロファイル×ImmCross失敗フォールバック時の随伴warmup重複送信
status: |-
  実装完了（2026-09-12）。opus-adversarial-consult 1ラウンドで選択肢B
  （`ImeOpenOutcome`への`AppliedWithoutSendInput` variant追加）採用に
  収束。Linux上で`cargo test --lib`（1016件）・`cargo nextest run
  -p awase-windows --test architecture_guard --test golden_scenarios
  --test layer_boundary_guard`（123件）・windows target `cargo check`/
  `clippy`とも全緑。実機ソーク未実施（BUG-133参照）
related_adr:
  - "ADR-149"
  - "ADR-163"
---

# ADR-167: Standardプロファイル×ImmCross失敗フォールバック時の随伴warmup重複送信

## 背景

ADR-149（BUG-113残置症状の修正、PR #180）は、`platform.rs::on_ime_applied_inner`
の随伴 eager TSF warmup 送信を `should_send_accompanying_warmup(outcome)`
（`src/platform.rs`）でゲートし、「今回のapplyで戦略が実際に`SendInput`を
試みた場合（`Applied`/`FallbackSent`）は随伴warmupを重ねて送らない」という
修正を行った。1打鍵あたり最大3回だった`VK_IME_ON`重複送信を2回に削減し、
実機で「@」再発なしを確認済み。

この修正には**Standardプロファイル限定の例外**が最初から組み込まれている
（`platform.rs:1607-1609`）:

```rust
let profile = self.current_app_profile();
let should_send = profile.can_use_imm32_cross_process()
    || awase::platform::should_send_accompanying_warmup(outcome);
```

理由（コード中のコメント、ADR-149本文とも一致）: `ImmCrossProcessStrategy`
（IMM32クロスプロセスAPIのみ、SendInput皆無）はStandardプロファイルでしか
選ばれず、このストラテジーが`Applied`を返した場合は`should_send_accompanying_
warmup`の前提（`Applied` == 戦略が実際にSendInputした）が成立しない。この
ケースを取りこぼすとStandardプロファイルでTSFが温まらないままになるため、
「Standardプロファイルでは前提の真偽を判定できないので常に安全側（送る）に
倒す」という設計にした。

## 問題

`crates/awase-windows/src/ime_controller.rs`のstrategy優先順位（モジュール
doc冒頭に明記）は次の通り:

1. `ImmCrossProcessStrategy` — Standardプロファイルでのみ`is_applicable()`
2. `GjiDirectStrategy` — GJI検出時、**全プロファイルで**`is_applicable()`
3. `MsImeDirectStrategy`
4. `KanjiToggleStrategy`

`ImmCrossProcessStrategy`が`Failed`を返した場合（`SendMessageTimeout`の
タイムアウト等、実際に発生しうる）、`ImeController::apply`/`run_open_chain_
async`は次の適用可能な機構へフォールスルーする（同モジュールdoc「`Immcross
ProcessStrategyが`Failed`を返した場合...次の適用可能な戦略へフォールスルー
する」）。GJIが検出されているStandardプロファイルアプリでは、この経路で
**`GjiDirectStrategy`が実際に`VK_IME_ON`/`VK_IME_OFF`をSendInputし、
`ImeOpenOutcome::Applied`を返す**（`apply_mechanism`の
`Some(MechanismCommand::SendVk(vk)) if mechanism == WriteMechanism::GjiDirect
=> ... ImeOpenOutcome::Applied`分岐）。

この場合、`on_ime_applied_inner`の`should_send`計算は:

```
profile.can_use_imm32_cross_process() (true, Standardだから)
  || should_send_accompanying_warmup(Applied) (false, 既に送信済みのはず)
= true
```

`profile.can_use_imm32_cross_process()`が「今回Applied を返した機構が実際に
SendInputしたか」を一切見ずにStandardプロファイル全体で無条件trueになるため、
**GjiDirectStrategyが実際に送信した直後に、随伴warmupがもう1回SendInputする**
——ADR-149がまさに解消しようとした「1打鍵に対する即時連続SendInput」の
パターン（当時の「送信1+送信2」、実機A/Bで「@」の必要条件と確認済み）を、
Standardプロファイル×ImmCross失敗という狭い条件下で再現する。

非Standardプロファイル（TsfNative/Imm32Unavailable）では`ImmCrossProcess
Strategy`自体が`is_applicable()==false`のため`Applied`は常に実送信を伴う
戦略由来と確定でき、`should_send_accompanying_warmup(Applied)`が正しく
`false`を返して随伴warmupを抑止する。Standardプロファイルだけがこの保護を
失っている。

## 確認済みの事実

- `WriteMechanism::ImmCross`のみ「VKを送らない」（doc明記）。`GjiDirect`/
  `MsImeDirect`/`KanjiToggle`はいずれも実`SendInput`を伴う。
- `ActuationDecisionRecord::attempts`（`AttemptRecord { mechanism,
  command: Option<MechanismCommand>, outcome, .. }`）は、sync経路
  （`ImeController::apply`）・async経路（`run_open_chain_async`）の両方で
  既に構築されており、「どの機構が実際に何を送ったか」を機構ごとに記録
  済みである。今回問題にしている情報はこの構造体に既に存在する。
- `on_ime_apply_complete`の呼び出し元は7箇所（`message_handlers.rs`,
  `key_pipeline.rs`×3, `ime_refresh.rs`, `mod.rs`×2）。うち5箇所は
  `apply_ime_open_with_view`/`apply_ime_open_with_belief`/
  `ImeController::apply`から直接`(outcome, record)`を受け取っており、
  `record`は既にローカル変数として手元にある。残り2箇所
  （`key_pipeline.rs`の`run_open_chain_async`呼び出し）は`outcome`のみを
  受け取っており、`record`は`run_open_chain_async`内部で構築・journal
  記録されるが呼び出し元には返っていない。

## 選択肢

**A. `ActuationDecisionRecord.attempts`から実送信の有無を判定し、
`on_ime_apply_complete`経由で`on_ime_applied_inner`まで届ける。**
`profile.can_use_imm32_cross_process()`という粗い代理指標をやめ、実際に
何が起きたかを直接見る。実装:

1. `state/actuation_decision_record.rs`（または`ime_controller.rs`）に
   純粋関数`attempts_included_real_send_input(record: &ActuationDecision
   Record) -> bool`を追加。`attempts[..attempts_len]`を走査し、
   `mechanism != WriteMechanism::ImmCross && command.is_some()`な要素が
   1つでもあれば`true`。
2. `run_open_chain_async`の戻り値を`ImeOpenOutcome`から
   `(ImeOpenOutcome, ActuationDecisionRecord)`へ変更（sync側の
   `ImeController::apply`と同じ形に揃える）。2箇所の呼び出し元
   （`key_pipeline.rs`、同一spawn_localブロック内の同期/非同期分岐）を
   追随させる。
3. `on_ime_apply_complete`のシグネチャに`used_real_send_input: bool`を
   追加。7呼び出し箇所すべてで、手元の`record`から上記関数を呼んで渡す。
4. `on_ime_applied`/`on_ime_applied_without_cold_mark`/
   `on_ime_applied_inner`にも同じ引数を追加し、`platform.rs:1607-1609`の
   `profile.can_use_imm32_cross_process()`を`used_real_send_input`へ
   置き換える。
5. 回帰テスト: `src/platform.rs`の`should_send_accompanying_warmup`
   ユニットテストに加え、`attempts_included_real_send_input`の
   ユニットテスト（ImmCrossのみ/ImmCross失敗→GjiDirect成功/GjiDirect
   単独、の3ケース）を追加。可能なら`ime_key_sequence_golden.rs`に
   「Standard×GJI×ImmCross Failed」ケースの送信列goldenを追加し、
   随伴warmupが送られないことを固定する。

長所: 根本原因に対する正確な修正。`fix-requires-evidence.md`が警告する
「profileやK軸でゲートしない」という`ADR-089 §2.4 INV-42`の精神
（`on_ime_apply_complete`のコメント「profile軸でもK軸でもゲートしないこと
がINV-42の本体」）に整合する——今回もStandardプロファイル全体を粗く
ゲートするのをやめ、実際に起きたことで判定する点で同じ方向性。
短所: `on_ime_apply_complete`という7箇所から呼ばれる合流点のシグネチャ
変更になるため、1箇所でも追随を忘れると同じ問題が再燃する
（`fix-requires-evidence.md`の「IME actuation合流点」ファミリーそのもの）。
`run_open_chain_async`の戻り値変更は他の呼び出し元にも影響がないか要確認。

**B. `ImeOpenOutcome`に新しいvariant（例:
`AppliedWithoutSendInput`）を足し、`apply_mechanism`の
`SetOpenCrossProcessSync`分岐だけこれを返すようにする。**
`should_send_accompanying_warmup`は`AppliedWithoutSendInput`のときだけ
`true`を返せばよく、`on_ime_apply_complete`のシグネチャは変更不要。
短所: `ImeOpenOutcome`は`serde::Serialize/Deserialize`済みで
`journal.rs`/`state/actuation_decision_record.rs`/ADR-163の再生
ハーネス（`tests/journals/`の凍結fixture）がこの型をシリアライズ済み
JSONとして保存している。新variantの追加自体は非破壊的（既存JSONの
デコードは壊れない）だが、「`Applied`一般」に対する既存の分岐・golden
テスト・再生コーパスの期待値が新variantを想定しておらず、影響範囲の
洗い出しに選択肢Aと同等以上の調査が必要になる可能性がある。

**C. Standardプロファイルの随伴warmup判定を、`profile`ではなく
`GjiDirectStrategy::apply`の呼び出し直後にローカルスコープで完結させる
（`ActuationDecisionRecord`は経由しない、より狭い変更）。**
未調査。`apply_mechanism`内で機構ごとのローカル変数を持ち回すだけで済む
可能性があるが、`on_ime_applied`側は最終的に`outcome`だけで判定している
現行構造を考えると、結局は選択肢Aと同じ情報をどこかで運ぶ必要があり、
運搬経路を`ActuationDecisionRecord`（既存の記録機構）にするか専用の新しい
経路にするかの違いに帰着する可能性が高い。

## 決定

**選択肢B（`ImeOpenOutcome`への`AppliedWithoutSendInput` variant追加）を
採用。** opus-adversarial-consult（1ラウンド）で以下が判明し、選択肢Aから
方針転換した。

### 選択肢Aは対象バグに届かない設計ミスだった

- 本文執筆時点の「`on_ime_apply_complete`7箇所のうち5箇所はrecordが手元に
  ある」という前提は誤り。実際に手元にあるのは**4箇所**（`key_pipeline.rs`
  の同期分岐2箇所、`ime_refresh.rs`、`mod.rs:1040`）。残り3箇所は
  `run_open_chain_async`経由で、**そのうち本ADRが問題にする経路
  （open=true×Standard×ImmCross失敗）が実際に通るのは`executor.rs`の
  毎打鍵engine decisionと`mod.rs`のforce-on bootstrapの2箇所**——いずれも
  `post_async_ime_apply_complete`でwparam/lparamにビットパックして
  `PostMessage`し、`message_handlers.rs::handle_wm_async_ime_apply_complete`
  に着地する。この**WMワイヤ越しの経路には`ActuationDecisionRecord`が
  構造的に届かない**ため、選択肢Aの実装計画（record.attemptsから実送信
  有無を判定）をそのまま実装しても対象バグは直らなかった。
- `run_open_chain_async`の呼び出し元も本文の「2箇所」ではなく**3箇所**
  （`executor.rs:927`/`mod.rs:1305`/`key_pipeline.rs:1705`）。

### 選択肢Bのリスクは当初の想定より小さい

- serde/凍結fixtureへの懸念は杞憂——ADR-163の`replay_record`は`outcome`
  を「外部入力として記録し、再計算しない」ため、新variant追加は再生
  ハーネスに影響しない。
- 追随漏れが起きうる箇所は**非網羅`matches!`の2箇所のみ**
  （`platform.rs::on_set_open_applied`呼び出しガード、
  `state/ime_model.rs`の`Superseded`判定）。他の6箇所は網羅`match`で
  コンパイルエラーが追随を強制する。選択肢Aの「7箇所＋WMワイヤ＋
  `ImeApplyCompletion`＋4メソッドシグネチャ」より遥かに追随漏れリスクが
  小さい。

### 重大度は過小評価ではなく主張どおり（むしろ正確には「2回」ではなく「3回」）

実測ログ（ADR-149）の送信1（実送信）・送信2（即時随伴warmup、本ADRが
問題にする重複）・送信3（~100ms後の2回目apply由来、`AlreadyMatched`
なら`should_send_accompanying_warmup`が引き続きtrueを返すため残存）を
踏まえると、Standard×ImmCross失敗フォールバック経路では**合計3回**——
ADR-149修正前と同じ密度パターンに戻る。ただし到達条件は
「Standardプロファイル×GJIアクティブ×Tsf注入（`InjectionMode::Tsf`）×
ImmCross Failed×open=true」の積で、`AppKind::TsfNative`の既定
`InjectionMode`は`Vk`のため狭い。実機証拠（「@」ログ）はこの例外分岐が
効かないTsfNative側で取得されたものであり、Standardプロファイルでの
実機確認は未実施（BUG-133参照）。

### 実装

1. `src/platform.rs`の`ImeOpenOutcome`に`AppliedWithoutSendInput`を追加。
   生成点は2箇所——`ime_controller.rs`（sync ImmCross成功）、
   `runtime/open_chain.rs::imm_cross_write`（async ImmCross `Written`）。
2. `should_send_accompanying_warmup`はロジック変更不要（新variantは
   `Applied | FallbackSent`に含まれないため自然に「送る」側になる）。
   `platform.rs::on_ime_applied_inner`のプロファイル軸の例外
   （`profile.can_use_imm32_cross_process() || ...`）を削除。
3. 非網羅`matches!`2箇所は`ImeOpenOutcome::wrote_open_state()`
   （新設の網羅match述語メソッド）へ置換。
4. 網羅`match`6箇所（`state/ime_event.rs`、`state/platform_state.rs`、
   `journal.rs`、`runtime/executor.rs`、`runtime/message_handlers.rs`の
   encode/decode）はコンパイラ誘導で全て追随。**`message_handlers.rs`の
   WMワイヤ（`encode_outcome`/`decode_outcome`）への追加が対象バグを
   直す上で必須**（選択肢Aが見落としていた点）。
5. テスト追加: `should_send_accompanying_warmup`/`wrote_open_state`の
   ユニットテスト（`src/platform.rs`）、`ALL_OUTCOMES`拡張
   （`state/actuation_chain.rs`、6→7要素）、`encode_outcome`/
   `decode_outcome`の全variantラウンドトリップテスト
   （`runtime/message_handlers.rs`）。`ime_key_sequence_golden.rs`は
   strategy選択（`is_applicable`）のみを固定しており`ImeOpenOutcome`を
   直接参照しないため更新不要（grep確認済み）。

## 関連

[ADR-149](149-physical-ime-key-activation-defers-forced-set-open.md)
（本ADRが対象とする随伴warmupゲートの元設計）、
[ADR-163](163-actuation-decision-io-separation-and-replay-harness.md)
（`ActuationDecisionRecord`/`AttemptRecord`の記録基盤）、
[fix-requires-evidence](../../.claude/rules/fix-requires-evidence.md)
「IME actuation 合流点」ファミリー。
