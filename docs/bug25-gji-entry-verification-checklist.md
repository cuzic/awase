# BUG-25 GJI半角英数entry 動作確認手順書

[ADR-107](adr/107-bug25-gji-half-width-alnum-entry.md) の Decision 2〜9 に基づき実装した
GJI（Google 日本語入力）向け「左Shift単独タップ→IME-ON半角英数トグル」entry/exit の、
Windows実機での動作確認手順。[known-bugs.md](known-bugs.md) BUG-25 追補6が要求する
「Task 9」チェックリストを、実行可能な手順に展開したもの。

**この機能は既定で無効（`ms_ime_only`）。** 本手順書は `half_width_alnum_toggle = "all"`
を明示設定した検証環境でのみ実施する。まだ実機ソークが完了しておらず、既知の限界
（追補5〜8）がいくつかある前提で臨むこと。

関連ルール: [fix-requires-evidence](../.claude/rules/fix-requires-evidence.md)、
[experiment-logging](../.claude/rules/experiment-logging.md)。

---

## Step 0: 事前準備

- [ ] `develop` 最新（PR #111 マージ後、コミット `c0fe751d` 以降）を取得している。
- [ ] Google 日本語入力（GJI）がインストール済みで、**既定の入力方式として選択可能**
      （タスクバーの言語バー/IME切替でGJIを選べる）。
- [ ] `cargo build --target x86_64-pc-windows-msvc -p awase-windows`（ログを多く見る
      検証なので debug ビルドで十分。`--release` でも可）。
- [ ] `config.toml` の `[general]` セクションに以下を追記する:

  ```toml
  [general]
  half_width_alnum_toggle = "all"
  ```

- [ ] `RUST_LOG=debug awase.exe` で起動する（`[hook]`/`[shift-conv-guard]`/
      `[ime-mode]` 系ログを見ながら検証するため）。ログファイルの場所は起動時の
      コンソール出力、または過去の実機検証で使ってきた `target/debug/awase.log`
      相当のリダイレクト先を確認する。
- [ ] 検証アプリは **Windows Terminal**（`CASCADIA_HOSTING_WINDOW_CLASS`、
      TsfNativeプロファイル）を第一候補にする——ADR-107の実機計測（決定0 M1〜M4）が
      すべてこのアプリで行われており、過去の知見と直接比較できる。他アプリへの
      横展開はStep 1〜3が通ってから行う。
- [ ] Windows Terminal で GJI がアクティブ・ひらがなモードであることを確認する。

---

## Step 1: 基本entry（Precomposition状態）

- [ ] 未確定文字列が無い状態（Precomposition）で、Windows Terminal にフォーカスする。
- [ ] **左Shiftキーを単独タップ**する（他キーを一切介さず、押して離すだけ）。
- [ ] ログに以下の行が出現することを確認する:
      ```
      [hook] IME-mode vk=0xF0 down self_injected=true ... extra=0x4B45594A
      ```
      （`extra=0x4B45594A` は `IME_KANJI_MARKER`）。**KeyUp行は出現しない見込み**
      （追補5で発見済みの既知の限界、原因未確定だがモード切替自体は成功する）。
- [ ] `[shift-conv-guard]` ログで「GJI 半角英数トグルON」相当のメッセージが出ること。
- [ ] CapsLockのインジケータ/ランプが変化していないこと（追補1/3が過去に汚染した
      ポイント）。
- [ ] アルファベットキー（例: `a`）を押す。**半角英数の `a` が入力されること**
      （ひらがなの「あ」に変換されないこと。IMC read-back ではなく、実際に画面に
      出た文字で判定する — 追補2の教訓）。

---

## Step 2: exit（2回目の左Shiftタップ）

- [ ] Step 1 の半角英数状態から、**もう一度左Shiftを単独タップ**する。
- [ ] `[shift-conv-guard]` ログで復元処理（GJI 半角英数トグル exit）が走ったことを
      確認する。
- [ ] `a` を押す → ローマ字入力としてひらがな変換が働くこと（例: `a` → 「あ」）。

---

## Step 3: 右Shift緊急解除

- [ ] Step 1 の要領で再度entry状態にする。
- [ ] 今度は **右Shiftを単独タップ**する（緊急解除経路）。
- [ ] Step 2 と同じ確認方法でひらがなに戻っていることを確認する。
- [ ] 右Shiftタップの直前・直後でシフトキーが「押されっぱなし」になっていないか
      （キーボード上のインジケータや、続けて `A` を押して大文字/かなが変にならないか
      で確認 — 追補7で対策した synthetic Shift↑ の左右scan問題の実地確認）。

---

## Step 4: フォーカス変更時の持ち越し確認

- [ ] Step 1 の要領でentry状態にする。
- [ ] Alt+Tabで別アプリ（例: メモ帳）へフォーカスを移す。
- [ ] 移動先アプリで何か打鍵し、そのアプリのIMEが英数のままになっていないこと
      （追補8で扱った、`ir_notify_focus_changed` 経由のexit確認）。
- [ ] Windows Terminal へ戻り、打鍵してひらがなに戻っていること（英数状態が
      持ち越されていないこと）を確認する。

---

## Step 5: Composition中の抑制確認

- [ ] Windows Terminal で日本語を入力し、変換前の未確定文字列（preedit）がある
      状態を作る。
- [ ] その状態で左Shiftを単独タップする。
- [ ] ログで「composition中のためスキップ」相当のメッセージが出て、**発火しない**
      ことを確認する（決定5の意図どおり）。
- [ ] preedit の内容が壊れていないこと。
- [ ] Enterで確定してから改めて左Shiftを単独タップし、今度は正しくentryすること。

---

## Step 6: 連続タップの安定性

- [ ] entry→exitを1秒未満の間隔で3回連続繰り返す。
- [ ] 追補5で報告された「一瞬全角英数になった後、IMEオフ・直接入力になる」ような
      予測不能な状態遷移が起きないか確認する。
- [ ] 起きた場合はタスクトレイアイコンから手動復旧できるか確認し、ログの
      `[shadow-toggle]`/`[warrant-shadow]`/`[idle-conv-check]` 行を保存する。

---

## Step 7: entry直後の1文字目レース確認

- [ ] entry操作の直後、間を置かずすぐにアルファベットキーを打鍵する。
- [ ] 1文字目だけ全角英数（`Ａ`のような全角）になっていないか確認する
      （追補5で単発観測された現象、settle値未実装のため再現しうる）。
- [ ] 複数回（10回程度）試し、再現率を記録する。

---

## Step 8（optional・narrow edge case）: IME製品切替

- [ ] entry状態のまま、言語バーまたは Win+Space でMS-IMEへ切り替える。
- [ ] その状態で2回目の左Shiftタップ（exit操作）を行う。
- [ ] 何が起きるか記録する（既知の限界として追補8に記載済み。頻度は低いと
      判断されているため、再現有無の確認のみでよい。再現した場合の重大度に
      応じて追加対応を検討する）。

---

## 結果の記録

- [ ] 各Stepの結果を `docs/known-bugs.md` BUG-25 追補6の実機検証チェックリストへ
      ✓/✗で反映する。
- [ ] 問題が見つかった場合は、[experiment-logging](../.claude/rules/experiment-logging.md)
      の規約（アプリ×IME×再現手順を必ず書く）に従って新しい追補を追加する。
- [ ] Step 1〜7 が全て通過した場合、BUG-25のクローズ判断（ユーザー確認の上で）に
      進める。Step 8 は既知の限界として残ったままでもクローズ判断を妨げない。

---

## 付録: Task 0（settle値・連続発火クールダウンの実測）は別作業

本手順書は Task 9（機能検証）が対象。Task 0（`SendInput` 後の settle 待ち時間の
実測）は診断用spikeツールを使う別作業で、`crates/awase-windows/examples/
spike_langbar_input_mode.rs` の以下のフラグを使う:

```sh
# marker=ime_kanji、awase起動中、DOWN/UPを50ms空けて送る例
target/debug/examples/spike_langbar_input_mode.exe \
  --sendinput-marker=ime_kanji --sendinput-up-delay-ms=50
```

`--sendinput-up-delay-ms` を 0/20/50/100 ms で各10回程度試し、Step 7 のレース
再現率がどう変化するかを記録する（ADR-107 決定2追記、known-bugs.md 追補5参照）。
実測が終わったら `tuning.rs` に定数化し、[tuning-constants](../.claude/rules/tuning-constants.md)
の実測義務に従ってコミット本文に導出根拠を明記する。
