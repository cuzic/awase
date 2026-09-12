# fix には「テスト」か「記録」を添える

## ルール

以下の **再発ファミリー** に触れる `fix` コミットは、次の (a)(b) の少なくとも一方を
同じコミット（または直後の追随コミット）に含めること。

- **(a) 回帰テスト**を追加する — golden / ジャーナルリプレイ / characterization の
  いずれか（下記「テストの置き場所」）。
- **(b) [docs/known-bugs/](../../docs/known-bugs/index.md)** に、症状・再現手順・修正履歴
  （コミットハッシュ）を1バグ1ファイル（`docs/known-bugs/BUG-NNN.md`、新規バグは
  次の連番を採番して新規作成）として追記する。**1ファイルあたり本文は目安30行以内**
  とする（[ADR-158](../../docs/adr/158-complexity-reduction-north-star.md) TH4、
  2026-09-09追記）——単一ファイル`docs/known-bugs.md`が16,825行まで膨らんだのは、
  まさにこの(b)の選択肢が新規fixのたびに詳細な散文を要求し続け、削除・要約を促す
  仕組みが無かったことが一因（[ADR-158](../../docs/adr/158-complexity-reduction-north-star.md)
  RC4参照）。2026-09-11に`docs/known-bugs/BUG-NNN.md`へ1件1ファイル分割し
  （frontmatterに完全なタイトル・関連コミット・関連ADRを保持、索引は
  [docs/known-bugs/index.md](../../docs/known-bugs/index.md)）、旧`docs/known-bugs.md`
  はリダイレクトスタブのみになった——これは表示上のスケーラビリティ対策であり、
  「要点を簡潔に記録し経緯の詳細な物語は書かない」という30行ルール自体は変わらない。
  将来的に[ADR-159](../../docs/adr/159-existing-io-boundary-inventory.md)
  の記録・再生基盤が育てば、(b)は「再生トレースの追加」（実際に問題を再現する
  journalトレースを`tests/journals/`等に保存する）へ置き換える予定（未実装、
  能力ベースの前提条件は[ADR-162](../../docs/adr/162-governance-reversal.md)
  E1/E4節参照）。

### 再発ファミリー（このルールが効く領域）

これまで同種のバグが何度も再燃してきた領域。ファイルの目安:

| ファミリー | 主なファイル |
| --- | --- |
| warmup / cold-start | `output/tsf_warmup_coord.rs`, `output/probe_io.rs`, `tsf/`, `output/ime_apply_planner.rs`, `tuning.rs` |
| focus 遷移 | `focus/`, `runtime/focus_tracking.rs` |
| IME belief | `state/ime_model.rs`, `state/observation_store.rs`, `runtime/ime_coordinator.rs`, `focus/uia.rs`, `focus/msaa.rs` |
| conv mode | `state/conv_mode.rs`, `focus/classify.rs`, `output/conv_actuation.rs`, `runtime/conv_actuation.rs`, `ime.rs` |
| キー選択（IME ON/OFF に送る VK） | `ime_controller.rs`, `output/vk_send.rs`, `src/engine/nicola_fsm.rs::resolve_pending_thumb_as_single`（無変換/変換単独タップの`dedicated_fn_key`/`delegate_to_open_axis`/`ModeKeyConfig`優先順位、BUG-119でルート`awase`クレート側にも同ファミリーの再発が判明。`crates/awase-windows/`配下だけを見ていた本表・`.githooks/pre-push`双方の見落としを2026-09-06に追加して埋めた） |
| force-write / actuation ターゲット（ADR-084/086） | `platform.rs`, `output/conv_actuation.rs`, `runtime/conv_actuation.rs`, `ime.rs` |
| IME actuation 合流点（新しい gate/precondition を足す場所、ADR-119） | `ime_controller.rs::apply`（同期経路唯一の合流点）, `runtime/open_chain.rs::run_open_chain_async`/`fallback_write`/`imm_cross_write`（非同期経路。3関数**すべて**が独立に再検出する設計、1箇所だけでは足りない）, `runtime/executor.rs::dispatch_ime_set_open`（早期exit最適化、上記と重複するが単独では不十分）, `runtime/mod.rs::reassert_explicit_physical_key`（[ADR-121](../../docs/adr/121-explicit-physical-ime-key-idempotent-reassert.md) D1、`apply_ime_open_with_view`を直接叩く5つ目の独立入口。`tests/architecture_guard.rs`の`.apply_ime_open_with_view(`件数ガードが3→4に更新済みだが、本表への追加が漏れていた——opus-adversarial-consult round1指摘）, `runtime/mod.rs::force_on_and_correct_romaji`（[ADR-158](../../docs/adr/158-complexity-reduction-north-star.md) TB2、`apply_force_on_for_imm_broken`——ADR-149/151/153が扱うTsfNative唯一のON方向救済機構——から呼ばれる`apply_ime_open_with_view`の6つ目の独立入口。新設の`lints/actuation_call_guard`宣言〈ADR-158 TB0/TB1〉と`xtask-adr-evidence`による棚卸しで、本表への追加漏れとして新規発見） |
| `ImeControlView.control.shadow_on`（`ControlLog`）の供給元（BUG-113、gate自体は1箇所でも供給元は経路ごとに違う） | `platform.rs::build_ime_control_view`（`ImeModel.applied_pair()`経由、`None`=未知）、`runtime/ime_refresh.rs::ir_apply_drift_correction`（`apply_ime_open_with_belief(order, None, ..)` — drift correction OFF方向回復、`None`ハードコード）、`runtime/key_pipeline.rs`のidle-conv-check DirectInput回復（同じく`None`ハードコード、コメントに「already_matchedをバイパスして apply する」と明記）、`runtime/executor.rs`の`applied_snapshot`。**`shadow_on`を`bool`に潰す（`unwrap_or(false)`）と「未知」と「確認済みfalse」の区別が消え、`None`で意図的にbypassしている経路の意図を握り潰す**——`Option<bool>`のまま扱い、「送信を省略してよいか」の判定は陽性の確認済み証拠（`Some(x)`）にのみ基づかせること（`state/ime_model.rs::applied_open()`のdocが警告するADR-098決定1-bの罠と同型）。 |
| 物理IMEキーのSuppress/Allow配送判断（BUG-46/BUG-52/BUG-116、キー選択とは別軸の「そもそもOSへ届けるか」の判断点） | `runtime/transport.rs::PhysicalKeyDisposition::plan`。BUG-116（2026-09-05）はこのファイルが本表にも`.git/hooks/pre-push`の正規表現にも含まれていなかったため、BUG-52修正が物理配送を変えても自動チェックに引っかからない穴になっていた（2026-08-08の`platform.rs`追加が埋めた穴と同型）。VK種別だけで場合分けせず、`shadow_toggled`・修飾キー状態・profile・`dbe_mode_key_policy`のどれを条件に含めるかで挙動が大きく変わるため、変更時は関連するVK全種類（0xF0〜0xF6）への影響を洗い出すこと。 |
| defer/replay キューの解放条件（[ADR-156](../../docs/adr/156-unify-deferred-execution-queues.md)、ADR-123→ADR-128の回帰: 同一キューに対する複数の窓口の片方だけに新条件を配線し忘れる） | `input_defer.rs`、`output/vk_send.rs`（`DeferGate`/`defer_respecting_gate`/`drain_pending_deferred_before_send_if_queue_only`、`pending_deferred`のdefer側/drain側2窓口）、`output/tsf_warmup_coord.rs`、`runtime/message_handlers.rs::handle_wm_drain_output_queue`/`handle_wm_timer`、`runtime/ime_coordinator.rs`（`deferred_engine_timers`）、`runtime/executor.rs::drain_deferred`（`guard_held`/`ReinjectKey`）、`runtime/outbox.rs`（`RuntimeOutbox`）。新しい解放条件（gate種別追加等）を1つのキューに足す際は、そのキューの**全ての**待避/解放窓口（defer側だけでなくdrain側も）に配線したか確認すること。 |

## テストの置き場所（このリポジトリの既存資産）

- `crates/awase-windows/tests/ime_key_sequence_golden.rs` — 戦略選択（ImmCross →
  GjiDirect → MsImeDirect → KanjiToggle）と送信キー列の golden。キー選択を変える
  fix はここに期待値を足す。`ime_controller.rs::characterize_strategy` が SSOT。
- `crates/awase-windows/tests/golden_scenarios.rs` と `crates/awase-windows/tests/golden/` —
  シナリオ golden。
- `crates/awase-windows/tests/e2e_windows.rs` — Windows 実機経路の e2e。
- ジャーナルリプレイ基盤（`journal.rs` 起点、整備中）— `classify_*` 純粋関数への
  入力列を記録・再生して belief/conv 遷移を回帰させる。純粋判定を変える fix はここが最適。
- `src/engine/tests.rs`（ルート`awase`クレート） — `resolve_pending_thumb_as_
  single`等、プラットフォーム非依存のエンジン内部ロジックを変える fix はここに
  ユニットテストを足す（BUG-119/ADR-147の前例）。`cargo test --lib`（ホスト
  ターゲットで実行可、Windowsターゲット不要）。

Linux で `cargo test -p awase-windows` から実行できるもの（golden / architecture_guard /
layer_boundary_guard 等）を優先する。実機依存で自動化できない場合は (b) の
`docs/known-bugs/BUG-NNN.md` 追加で代替する。

## なぜこのルールが必要か（背景）

warmup・focus・belief・conv・キー選択の 5 領域は、実機の組み合わせ依存が強く、
「直したつもり」が別の環境で再発する。実例:

- IME OFF キー選択は 5 日間で 6 回反転した（[docs/experiments.md](../../docs/experiments.md)、
  `534051a`〜`489cdf1`）。golden（`ime_key_sequence_golden.rs`）があれば、キーを変えた
  瞬間に「Chrome では受け付けない `VK_IME_OFF` に変えた」等の退行を CI で検知できる。
- Chrome cold-start のリテラル化（`b101153` / `79134f5` / `3c275a7` …）は
  [docs/known-bugs/BUG-002.md](../../docs/known-bugs/BUG-002.md) に修正履歴が積まれており、
  次の担当者が「probe 起点のズレが真因で、値を上げるのは対症」という過去の知見に
  すぐ辿り着ける。
- issue #136 / ADR-119（2026-09-02）: `AppImeProfile::InputRelay` に「actuation を
  所有しない」gate を追加した際、最初に見つけた `runtime/executor.rs::
  dispatch_ime_set_open` の1箇所にしか置かなかった。しかし実際の actuation 呼び出し
  経路は5つあり、うち `runtime/key_pipeline.rs` のshadow-toggle経路（issue #136が
  報告した「物理IMEキー押下」操作そのもの）がこの gate を素通りしていた。物理キーは
  `transport.rs::plan` で `Allow`（決定4条件b）される一方でawase自身も actuate して
  しまう、BUG-46型の新規二重actuationを**このPR自身が作っていた**。実装完了後の
  opus敵対的コードレビューで発覚・修正。「新しい gate を1箇所に置いて満足しない、
  実際の呼び出し経路をすべて洗い出す」という、このルールが対象とする「reincidence
  family」＝「1箇所直しただけでは再発する領域」の典型例。上表の「IME actuation
  合流点」行はこの経緯で追加した。

「fix コミット単体」は、それが**何を再発させないためのものか**を残さない。テストは
機械可読な再発防止、`docs/known-bugs/` は人間可読な再発防止であり、どちらか一方は必ず要る。

## 自動チェック（pre-push）

`.git/hooks/pre-push`（および追跡下のミラー`.githooks/pre-push`、両者は2026-09-06時点で
本文が一部乖離しており未解消——実行されるのは`.git/hooks/pre-push`側）に軽量チェックを
入れてある。上表の対象ファイルが変更されている
push で、`crates/awase-windows/tests/` にも `docs/known-bugs/` にも差分が無い場合、
**警告を出す（ブロックはしない）**。golden の期待値更新や `docs/known-bugs/` への
新規ファイル追加を忘れていないかの気づきを与えるのが目的。意図的にテスト/記録が
不要な変更（純粋なリファクタ等）はそのまま push してよい。

関連: [experiment-logging](./experiment-logging.md)、[tuning-constants](./tuning-constants.md)、
[ime-belief-architecture](./ime-belief-architecture.md)。
