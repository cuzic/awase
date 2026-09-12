---
id: NOTE-2026-07-25
title: |-
  Windows実機での`cargo test --lib -p awase-windows`初回実行で判明したテスト自体の不具合(実装バグではない)
type: note
---

# 2026-07-25: Windows実機での`cargo test --lib -p awase-windows`初回実行で判明したテスト自体の不具合(実装バグではない)

GCP Spot self-hosted runner導入により、`cargo test --lib -p awase-windows`が実Windows上で初めて実行された（従来はLinux上でのクロスコンパイル`--no-run`チェックのみで、実行そのものは未実施だった）。BUG-41以外に、以下は**実装ではなくテスト自体の不具合**と判明したため、テスト側を修正した:

- `tsf::probe::tests::check_now_returns_stale_confirm_when_write_evidence_predates_epoch`、`tsf::warmup::probe_fsm::tests::chrome_per_vk_stale_confirm_from_leftover_candidate_window_recovers_like_suspected_literal`、`tsf::warmup::literal_detect_fsm::tests::poll_recovers_like_suspected_literal_when_stale_confirm_detected`: いずれも`std::thread::sleep(5ms)`+実`GetTickCount64`(既定解像度~15.6ms)で「epochより前」の時刻を作ろうとしていたが、tick解像度に対してマージンが無く、同一tickに丸まると`evidence_is_fresh`のtie判定(`>=`)が意図せずtrueになりflakyに失敗しうる設計だった。同ファイル内の他のテスト（`check_now_show_only_confirm_becomes_stale_after_grace_expires`等）が既に使っている`saturating_sub(50)`方式に統一し、実時間sleepへの依存を排除した。`EPOCH_FENCE_GRACE_MS`等の本番タイミング定数は変更していない。（`literal_detect_fsm.rs`側は最初の修正時に見落としており、下記ロック統一後の再検証で単独の真の失敗として顕在化し追加修正した。）
- `runtime::executor::tests::confident_when_confirmed_on_desired_on`: `now_ms=100_000`/`at_ms=500`(経過99,500ms)というテスト新設時点(`f7f09bc`, 2026-06-04)から既に300ms窓の外にある入力を使っていた。`chrome_intent_confident`の「Confirmed一致から300ms以内のみconfident」という設計(`7a24442`でOFF方向の永続スキップを廃止した際に確立)自体は正しく、テストの入力値を300ms以内(`at_ms=900`/`now_ms=1000`)に修正した。
- `tsf::warmup::literal_detect_fsm::tests::poll_recovers_like_suspected_literal_when_stale_confirm_detected`（および`poll_vetoes_backspace_while_candidate_visible`のPoisonErrorカスケード）: `TSF_OBS`（プロセス全体のグローバル状態）を保護するはずの`Mutex`が`observer.rs`/`probe.rs`/`literal_detect_fsm.rs`の3ファイルでそれぞれ**別々**の`static`として定義されており(`TEST_LOCK`×2、`VETO_TEST_LOCK`×1)、名前は同じでも異なる`Mutex`インスタンスのため互いに排他できていなかった。`cargo test`のデフォルト並列実行下で、あるファイルのテストが別ファイルのテストの`TSF_OBS`書き換えに巻き込まれ、`gji_last_write_ms`が意図せず0にリセットされる等で本来`StaleConfirm`になるはずの判定が`CompositionConfirmed`に化けていた。`observer.rs`に`TSF_OBS_TEST_LOCK`を1つだけ定義し、3ファイルとも`use ... as TEST_LOCK`でこれを共有するよう統一した。**この統一作業で`probe_fsm.rs`のテスト3件(`chrome_per_vk_*`)がそもそも一切ロックを持たずTSF_OBSを直接操作していた点を見落としており**、統一後の再検証で「probe.rsの他テストがprobe_fsm.rsの無防備な書き換えに巻き込まれて新たに失敗する」という形で発覚し、この3件にも同じ`TSF_OBS_TEST_LOCK`を追加する追加修正が必要だった。
- `tsf::warmup::probe_fsm::tests::decide_plan_nc_fired_enables_literal_when_gji_active`: BUG-40で既に「次にこのファイルに触れるセッションで要確認」と記録されていた通り、`nc_fired=true`時に`needs_literal=true`を期待する古い実装(旧`should_prepend_f2`由来、削除済み)の名残だった。BUG-40で確立された新しい意図(`nc_fired=true`＝NameChange確認済みなら常に`needs_literal=false`)に合わせてテスト名・アサーションを更新した(`decide_plan_nc_fired_suppresses_literal_even_when_gji_active`に改名)。

いずれも`cargo test --target x86_64-pc-windows-gnu --no-run -p awase-windows`（`-D warnings`）・`cargo clippy --target x86_64-pc-windows-gnu -p awase-windows`で警告ゼロ確認済み。**2026-07-25、GCP Spot self-hosted runner(`rust-nicola-builder`)上での実`cargo test --lib -p awase-windows`実行で全パス確認済み**(`cargo mutants`のbaselineフェーズが`ok Unmutated baseline in 44s build + 4s test`で成功、GitHub Actions run 30098397721)。当初1回の修正では`literal_detect_fsm.rs`のsleep依存と`probe_fsm.rs`のロック不備を見落としており、計4コミット・3回の実機再実行を経て全15件の失敗が解消したことを確認した(教訓: クロスファイルでグローバル状態を共有するテスト群は、1箇所直すたびに実機で再実行し、マスクされていた別の失敗が露出しないか確認するまで「直った」と判断しないこと)。

なお`cargo mutants`のフル走査(3296ミュータント)自体はjobのtimeout-minutes(180分)内に完走せず`cancelled`になったが、これはミュータント総数が非常に多いことによるもので、baselineの全パスとは無関係(バグではない)。フル走査を完走させたい場合は`--jobs`を増やすかタイムアウトを延ばすか、`-f`で対象ファイルを絞ること。
