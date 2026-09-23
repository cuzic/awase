# ADR-195 T1: 独立プロセスでの学習ラウンド（段階1）を実装する

**【ADR-196で一部拡張、B2は修正済み（2026-09-23、PR #259）】** opus-adversarial-consultの
docs/tasks横断レビュー（B2）で、`driver.rs`（PR #250〜#256の共通土台）が
[ADR-196](../adr/196-keymap-learn-truth-priority.md)決定1bの必須要件に反していると
指摘された。実コードで裏取りした結果、2点のうち1点は実際には問題なかった:

- `driver.rs`の`SendInput`が`dwExtraInfo: 0`のまま（決定1b項目1が要求する自分の目印が
  無い）——**実在する問題、PR #259で修正済み**。目印が無いと、[ADR196-T1](adr196-t1-external-write-observation.md)
  が実装する分類規則（自分の目印が無い注入はすべて外部とみなす）の下で、学習プロセス
  自身の注入が全て「外部からの書き込み」に誤分類され、セッションが即座に失敗し続ける。
- `driver.rs`の`thread::sleep(1ms)`（決定1b項目4が禁じる「メッセージを回さずに待つ」実装）
  ——**誤検出だった**。既存の`pump_for()`は`PeekMessageW`+`DispatchMessageW`を1msごとに
  ループしており、実際にはメッセージポンピングを回しながら待っている。修正不要。

自分の目印の付与自体は実装対象2（`ImeDriver`の6メソッド）に対する差分としてPR #259が
含む。[ADR196-T1](adr196-t1-external-write-observation.md)（フック・分類器そのもの）も
同じPR #259で実装済み。

状態: **本体はdevelopマージ済み（2026-09-23、PR #258〈T9〉経由。元PR #250〜#256は
supersededでクローズ済み、詳細は[adr195-remaining-work-2026-09-23.md](adr195-remaining-work-2026-09-23.md)
参照）。B2修正＋ADR196-T1はPR #259（`feat/adr196-t1-external-write-observation`、
develop未マージ、別セッション担当）で対応中——まだmergeされておらず、`driver.rs`の
`dwExtraInfo: 0`問題(B2)自体はdevelop側にまだ残っている。** [ADR195前提タスク](adr195-t-rebase-calibration-branch.md)
（`feat/awase-calibration`のrebase）完了後に着手。ADR-192とは無関係に着手可。
着手時は `.claude/rules/worktree-per-session.md` に従い専用 worktree/branch を切ること。

## 背景

[ADR-195](../adr/195-keymap-learn-productization.md)決定・段階1は、学習を`awase.exe`
とは別の独立した短命プロセスとして実装する（rev2からの最大の変更点。awaseの単一スレッド
非同期ランタイムに学習ループを埋め込む設計は、A'バイパス要件とTSFスレッド単位の状態保持の
どちらとも両立しないため）。実装の骨格（`RealImeDriver`によるWin32/TSF観測・
`SendInput`直叩き注入・専用EDIT窓）は`awase-keymap-learn-win`クレートに
スパイクとして既に存在するが、ADR-195が定める「製品としての体裁」は未着手。

## 実装対象（詳細・根拠はADR本文「段階1」節を必ず読むこと）

1. **クレート構成の確認**: `awase-keymap-learn`（OS非依存、Linuxで`cargo test`が回る）と
   `awase-keymap-learn-win`（Windows専用、`ImeDriver`実機実装）の依存の向きが
   `awase-keymap-learn-win → awase-keymap-learn`の一方向であることを確認する。
2. **`ImeDriver`トレイトを6メソッドに確定**: `press`/`press_setup`/`read_status`/
   `reread_status`/`settle_setup`/`reset`+`elapsed_ms(&self) -> f64`。`cost()`は
   トレイトに含めない。経過時間は`elapsed_ms()`だけが答える契約にし、`Executor`内の
   積算（`self.elapsed +=`）を撤去する。`Executor::new`は`ImeDriver::read_status()`
   から初期状態を取るようシグネチャを変える。
   **【ADR-196で追加、2026-09-23】** (a) `SendInput`呼び出しは専用の目印（`dwExtraInfo`）を
   自分の注入に付けること（`tsf/output.rs`の`INJECTED_MARKER`等とは別に、学習プロセス
   専用の値を1つ定義する）。(b) 押下・待ち・観測の実装は`thread::sleep`でブロックせず、
   `MsgWaitForMultipleObjectsEx`等でメッセージを回しながら待つこと（`WH_KEYBOARD_LL`
   フックとTSF compartment通知の双方が、フックを登録したスレッドのメッセージループに
   依存するため）。
3. **A'の確保（`is_app_disabled()`へのコード内定数OR追加）**: `is_keymap_learn_process_name`
   判定関数を新設し、`focus/tracker.rs::is_app_disabled()`に
   `|| is_keymap_learn_process_name(&self.current.process_name)`を1行OR追加する
   （`disable_apps`設定フィールドには足さない、既定値上書き問題を回避するため）。
   動的IPC・begin/end・keepaliveは一切不要。見積もり10行未満。
4. **フォーカスデバウンス待ち（round6 Major-1対応）**: 学習プロセスは専用窓へフォーカスを
   移した後、`config.general.focus_debounce_ms`（既定50ms）＋マージン待ってから最初の
   注入を行う。これを怠ると最初の数打鍵がデバウンス待ち中に通常のactuation経路を通り、
   awaseが自己操作してしまう。
5. **Enterエッジのチョードflush対応**: 学習プロセスの観測層は、最初の試行の前に入力欄を
   明示的にクリアしてからセットアップ検証を通す（`disable_apps`遷移が保留チョードを
   確定させる既存の挙動への対策）。
6. **変換モード値の正規化（B-2、round3 Blocker）**: 学習プロセスの観測層は、生の変換
   モード値を必ず`key_effect_predictor.rs::Conv::from_raw`に通してから表へ記録する
   （新しい正規化ロジックは作らない）。怠ると学習プロセス内の表と実アプリの値が
   一致せず、全セルが黙って「予測なし」に縮退する（自己検証でも検出できない）。
   `awase-keymap-learn::model::Status.mode: u8`は`Conv::from_raw`後の値
   （`C10=0x00`/`C19=0x09`/`C1B=0x0B`）をそのまま使う。
7. **異常への対処**: `awase-keymap-learn::anomaly`の分類とリセット段階（Soft/Mode/Hard）
   を実機ドライバに接続する。BUG-153〜159の教訓（観測の時間切れを異常の証拠に数えない）
   を反映する。
8. **actuationガード（m-b、round4 Minor）**: `awase-keymap-learn-win`からの
   `send_input_safe`/`set_ime_open*`呼び出しが0件であることを、
   `lints/actuation_call_guard`または専用のガードテストで固定する。

## 完了条件・テスト

- `cargo test -p awase-keymap-learn`（Linux、OS非依存部分）が通る。
- `cargo check --target x86_64-pc-windows-msvc -p awase-keymap-learn-win`が通る。
- 実機で「注入キーがバイパス対象プロセスで一切のactuationを起こさないこと」、特に
  「最初の1打鍵目から自己操作0であること」（デバウンス待ちが効いているかの直接確認）を
  A'の実測項目として確認する（この確認は[fix-requires-evidence.md](../../.claude/rules/fix-requires-evidence.md)
  の「warmup/cold-start」ファミリーに準じ、known-bugs記録かテストのいずれかを残す）。
- `cache.toml`の`Imm32Unavailable`残骸が効いていないか、実測時にクリアして再確認する
  手順を検証計画に残す。

## 関連

- [ADR-195](../adr/195-keymap-learn-productization.md) 段階1
- [ADR195前提タスク](adr195-t-rebase-calibration-branch.md)
- [ADR195-T2](adr195-t2-self-verification.md)（本タスクの学習ループを使う）
