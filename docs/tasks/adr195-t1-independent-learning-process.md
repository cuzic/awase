# ADR-195 T1: 独立プロセスでの学習ラウンド（段階1）を実装する

状態: 未着手（2026-09-23起票）。[ADR195前提タスク](adr195-t-rebase-calibration-branch.md)
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
