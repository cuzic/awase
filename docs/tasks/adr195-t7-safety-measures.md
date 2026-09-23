# ADR-195 T7: 安全対策（段階7）を実装する

状態: **一部実装済み（2026-09-23、PR [#264](https://github.com/cuzic/awase/pull/264)、
`feat/adr195-t7-safety-measures`）。opus-adversarial-consultによるレビューを
3ラウンド実施し収束。round1のMajor3件・Minor5件、round2のMajor1件（N1: 検証
ウォーク中の汚染が採点・セッション失敗判定の両方を素通りしていた）・Minor4件
（N2〜N5）、round3のMajor1件（R1: round2 N5の修正自体が生んだ退行——
フォーカスを恒久的に失うとセッション失敗にならず表が書き出されてしまう）・
Minor2件（R2: quiet window以外の初期化失敗にもreason=quiet_windowが付く／
R3: 失敗時も終了コードが常に0）を全て解消済み（詳細は各項目参照）。項目2
（ユーザー入力混入検出とその無効化・セッション失敗結線、検証ウォーク区間・
フォーカス恒久喪失時の送信前ゲート拒否も含む）を実装・テスト済み。項目1
（送信前ゲート、および`ResetLevel::Hard`が他プロセスからフォアグラウンドを
奪い返さないようにする修正）も本タスクで実装。項目4は既存実装で既に満たして
いることを確認した（新規実装は不要）。項目3・5は未着手のまま残る（下記参照）。
R1の修正（フォーカス恒久喪失→送信前ゲート拒否→即座にセッション失敗）は
Win32依存でホストでは検証できないため、下記「実機確認手順」を実機セッション
で実施すること。**
着手時は `.claude/rules/worktree-per-session.md` に従い専用 worktree/branch を切ること。

## 背景

[ADR-195](../adr/195-keymap-learn-productization.md)決定・段階7は、学習プロセスの
安全対策一式を定める。

## 実装対象

1. **専用窓への注入に限る**（他アプリへは送らない。T1のA'確保策と同じ窓を使う）。
   **【round1 M2対応、2026-09-23】** 当初「`SetForegroundWindow`/`SetFocus`で
   フォーカスを強制しているので新規実装不要」としたが、opus-adversarial-consultの
   指摘で誤りと判明した——(a) 事後の`focus_intact()`チェックだけでは`SendInput`が
   既に他アプリへ届いた後にしか検出できない、(b) 検出しても誰も送信を止めていな
   かった（項目2参照）、(c) `press_setup()`・`reset()`のMode段階が
   `check_session_interference`を経由しない生の`send_key_press`直呼びだった。
   これを受けて`send_gated()`（`crates/awase-keymap-learn-win/src/driver.rs`）を
   追加し、`inject()`・`reset()`のMode段階の両方の送信直前にフォーカス確認
   （`GetForegroundWindow()==self.window && GetFocus()==self.edit`）を行い、
   フォーカスが無ければ送信そのものを中止するようにした。
2. **ユーザー入力の混入検出**: 学習プロセスが自分の注入以外のキー・フォーカス変更を
   検出したら当該試行を無効化する。**【S7対応、2026-09-23】`WH_KEYBOARD_LL`フックと
   注入イベントの分類器（`LLKHF_INJECTED`の有無・自分の目印の有無での3分類）は
   [ADR196-T1](adr196-t1-external-write-observation.md)が実装・所有する。本タスクは、
   T1の分類器が「ユーザーの物理入力」と分類したイベントを受け取った後の**無効化処理**
   だけを担当し、独自のフックを作らない。**
   **【round1 M1対応、2026-09-23】** 当初の実装は検出するだけで、`Executor::press`
   （`awase-keymap-learn::exec`）は汚染された観測もそのまま表に記録し、
   `check_session_interference()`の戻り値も呼び出し元で捨てられており、
   「当該試行を無効化する」というタスクの目的が実際には満たされていなかった
   （opus-adversarial-consult round1 M1）。これを次のように結線した:
   - `PressReport`（`awase-keymap-learn::sim`）に`contaminated: bool`を追加。
     `RealImeDriver::press`は`check_session_interference()`の結果をここに入れる。
   - `Executor::press`は`contaminated=true`の観測を`Table`へ記録せず、
     `stats.contaminated_trials`へ計上するだけにする（`SimIme`は常に`false`）。
   - `RealImeDriver`に`session_failed()`（無効化上限超過で`true`固定）を追加し、
     `main.rs::run_main`が学習の直後にこれを確認、`true`なら検証ウォーク・表の
     書き出しへ進まず`result status=failure ... reason=interference`を出して
     終了する（`print_interference_failure_line`）。
   **【round1 M3対応】** `focus_intact()`は`GetFocus()`の1点サンプリングだけでは
   測定区間中の一瞬のフォーカス喪失（通知トーストの前面化等）を見逃すため、
   `window_proc`で`WM_ACTIVATE(WA_INACTIVE)`を数える`FOCUS_LOST_EVENTS`を追加し、
   区間内の差分でも判定するようにした。また`GetFocus()`単体では
   前面窓かどうか分からないため`GetForegroundWindow()==self.window`も併せて
   確認するようにした。
   **【round1 m1・m2対応】** 3種の汚染要因（外部からの書き込み・物理入力・
   フォーカス喪失）のbaseline管理を`InterferenceTracker`
   （`crates/awase-keymap-learn/src/external_write.rs`、ホストでユニットテスト済み）
   へ集約した。判定とbaseline更新を1回の`observe()`呼び出しで同時に行うため、
   「判定に使った値と実際にbaselineへ書き込む値がずれる」隙間（m1）が構造的に
   無くなり、`RealImeDriver::new()`のquiet window判定と
   `check_session_interference()`が同じインスタンスを共有するため、両者の
   baselineが食い違う（m2）ことも構造的に起きない。
3. **他アプリへの副作用を作らない**: TsfNativeアプリへ生キーが届く経路が無いため
   BUG-113/124の「@」の機序は成立しないことを確認する。管理者権限の窓は対象外とする。
   短時間の大量注入がセキュリティソフトに検知されない範囲に押下数を抑える（S6+イベント
   待ちで既に大幅な削減がある前提を踏襲）。**未着手——実機での確認（セキュリティ
   ソフト検知の有無、TsfNativeアプリへの副作用の有無）が前提のため、Windows実機
   セッションで対応すること。前提として項目1の送信前ゲートが実装済みであること
   （round1 m5対応、実装済み）。**
4. **永続化ファイルの検証**（[ADR195-T4](adr195-t4-runtime-loading.md)の破損ファイル
   縮退と連動）。**【2026-09-23確認、opus-adversarial-consult round1で妥当と確認済み】
   新規実装は不要**——[ADR195-T4](adr195-t4-runtime-loading.md)が
   develop統合済み（`d7e0df17`）で、`crates/awase-windows/src/state/key_effect_runtime.rs`が
   サイズ上限超過・パース失敗・スキーマ版不一致のいずれも安全に縮退させる実装と
   回帰テスト（`schema_version_mismatch_is_rejected`等）を既に持つ。
5. **配布**: 学習プロセスはユーザーの実機で走る新しい.exeになるため、MSIへの同梱・署名・
   アンインストール時の扱い（ADR-177/178）を実装時に検討する。
   [ADR195-T6](adr195-t6-adr176-wizard-integration.md)でウィザードから起動する以上、
   同梱は必須。**未着手——MSI同梱・署名はWindows実機でのインストーラビルド・検証が
   前提のため、Windows実機セッションで対応すること。**

## 完了条件

- ユーザー入力混入検出のテスト（学習窓以外へのフォーカス変更・非注入キーの検出）。
  **【完了】** 判定ロジックは`crates/awase-keymap-learn/src/external_write.rs`の
  `InterferenceTracker`/`trial_contaminated`関連テスト（`cargo test -p
  awase-keymap-learn --lib external_write`）でホスト実行確認済み。「検出したら
  実際に無効化されるか」は`crates/awase-keymap-learn/src/exec.rs`の
  `contaminated_press_is_not_recorded_but_is_counted`/
  `uncontaminated_press_is_recorded_normally`（`cargo test -p awase-keymap-learn
  --lib exec`）で確認済み（round1 M1・m2対応、round2 N1対応で`PressInfo`にも
  `contaminated`が伝わることを追加確認）。セッション失敗時に予算を使い切らず
  打ち切ることは`strategy::tests::over_respects_driver_should_abort_even_within_budget`
  （round2 N3対応）で確認済み。`RealImeDriver`側のWin32結線は
  `cargo check --target x86_64-pc-windows-msvc -p awase-keymap-learn-win --bins
  --tests --lib`でコンパイル確認済み（実機での動作確認は未実施——CLAUDE.mdの
  既存注意通り、このサンドボックスではlink.exe不在のため実行不可）。
- MSI同梱・署名の実装確認（Windows実機でのインストーラ検証）。**未完了（項目5参照）。**

## 実機確認手順（round3 R1対応、未実施）

opus-adversarial-consult round3 R1が指摘した「フォーカスを恒久的に失った
場合の回復経路」は、`GetForegroundWindow`/`GetFocus`の実際の挙動に依存する
ためLinux上のユニットテストでは検証できない。Windows実機で次を確認すること。

1. `--strategy=s0`で学習プロセスを起動する。
2. 学習窓（専用EDIT窓）へ最初の数回の押下が届き始めたら、Alt+Tabで別アプリ
   （メモ帳等）へ切り替え、**学習窓へ戻らない**。
3. 期待する挙動:
   - 数秒以内（次の`press`/`press_setup`/`reset`の送信前ゲートに達した時点）に
     `send_gated`が拒否し、`session_failed`が立って学習プロセスが
     `result status=failure ... reason=interference`を出して終了する
     （`std::process::exit(1)`、終了コード1）。
   - 学習プロセスが切り替え先のアプリ（メモ帳）からフォアグラウンドを
     奪い返さない（メモ帳がフォアグラウンドに留まり続ける。タスクバーの
     点滅で存在を示すのは許容——`SetForegroundWindow`自体は呼ばない）。
   - 切り替え先のアプリへユーザーが打鍵した内容が、学習窓（EDIT）に誤って
     入力されない。
4. 予算いっぱい（数分）学習プロセスが空回りしないこと、途中までの部分的な
   学習表が`status=success`で書き出されないことを確認する。

## 関連

- [ADR-195](../adr/195-keymap-learn-productization.md) 段階7
- [ADR195-T1](adr195-t1-independent-learning-process.md)
- [ADR195-T6](adr195-t6-adr176-wizard-integration.md)
- [ADR195-T10](adr195-t10-realimedriver-ci-observation-failure.md)
  （opus-adversarial-consult round1 m4: 本タスクの送信前ゲート・quiet window
  フォーカス確認により、フォーカス取得に失敗する環境〈CI等〉では学習プロセスが
  起動直後に`Err`で終了するようになった。T10の`presses=0`原因がまさに
  「フォーカスがEDITに無い」だった場合、この挙動変化は原因の切り分けに有益）
