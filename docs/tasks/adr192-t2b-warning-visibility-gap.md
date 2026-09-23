# ADR-192 T2b: 状態依存キー警告のユーザー可視化ギャップを埋める

状態: 完了（2026-09-23起票・実装完了、opus-adversarial-consultで決定2bを収束させた上で
実装、PR #254でdevelopマージ済み）
着手時は `.claude/rules/worktree-per-session.md` に従い専用 worktree/branch を切ること。

## 背景

[ADR-192](../adr/192-state-dependent-mode-key-warning-and-guided-override.md)決定2・3の
実装（T2: `crates/awase-windows/src/state/state_dependent_key_warning.rs`、T3:
`crates/awase-settings/src/main.rs`のADR-192置き換えUI）を検証した際に発見した実質的な
ギャップ:

1. **T2の警告は`tracing::warn!`のログ出力のみで、実際のユーザーには一切見えない**
   （`Runtime::check_state_dependent_mode_keys`）。既存の`msime_key_assignment::
   check_and_warn`は同種の検出に対して`spawn_yes_open_ime_settings_dialog`で
   実際のダイアログを表示しているが、ADR-192の警告はそこに届いていない。
2. **T3のawase-settings側UI（「IME ON/OFFキー」欄）は、T1/T2の実際の検出結果を
   読みに行かず、常に表示される汎用の「推奨する置き換え」ボタンになっている**
   （現在ログイン中のIME・キーマップが実際に状態依存かどうかに関わらず表示される）。

結果として、**状態依存キーを使っているユーザーが実際に警告を目にして行動を起こす経路が
現状存在しない**——判定ロジック自体（T1）は実測データと一致する正しい実装だが、
エンドツーエンドでは機能していない。

この構造的な原因は、ADR-192自体が「awase.exe（検出を行う主プロセス）→
awase-settings（表示を担うUIプロセス）」のプロセス間連携方法を明記していなかったこと
にある。両者は別プロセスであり、[ADR-176](../adr/176-behavioral-calibration-of-ime-mode-key-shadow-overrides.md)の較正機能が同様の課題に対して`calibration_ipc.rs`で
専用のIPCを設けたのと同型の設計判断が必要になる。

## やること（設計を先に固めること、実装より前に方針を決める）

1. **表示先の決定**: 次のいずれか（またはその組み合わせ）を選ぶ:
   - (a) awase.exe側で`msime_key_assignment::check_and_warn`と同型のダイアログを
     直接表示する（`spawn_yes_open_ime_settings_dialog`相当。ただし文言はADR-192の
     案内〈「awaseの明示config...で置き換えられること」〉に合わせた別関数が必要——
     既存関数はMS-IMEキー割り当ての競合専用の文言を持つため流用不可）。
   - (b) awase.exeが検出結果をファイル/レジストリ等の永続的な場所に書き、
     awase-settings起動時・ポーリング時にそれを読んでUIに反映する（ADR-176の
     `KeymapCache`のようなfsスタンプ方式に近い）。
   - (c) 新しいIPC（`calibration_ipc.rs`と同型のメッセージ）でawase.exe→
     awase-settingsへ検出結果を伝える。
   - **(a)がおそらく最も単純**（既存の`check_and_warn`とほぼ同じパターンの複製で済み、
     新しいプロセス間通信を必要としない）。ADR-192決定2が「既存の警告ダイアログの
     規約に合わせる」と書いている点とも整合する。ただし、T3が用意した
     「awase-settingsでの1操作置き換え」導線（決定3）へどう繋ぐか（ダイアログから
     awase-settingsを起動する導線が要るか等）は設計時に決めること。
2. 設計を決めたら、opus-adversarial-consultで軽くレビューしてから実装するか、
   ADR-192決定2・3への追記として反映するかを判断する（変更が小さければADR追記、
   大きければ別ADRとして起票してもよい）。
3. T3の「IME ON/OFFキー」欄のUIを、T1の判定関数を呼んで実際の検出結果を条件に
   表示するよう改修する（現状は無条件で表示される汎用ボタンになっている）。

## 完了条件

- 状態依存キーを実際に使っているユーザーが、awase起動時・設定変更時に何らかの形で
  警告を目にできること（ログだけでなく、ダイアログ/通知/awase-settings起動時の
  バッジ等、具体的なUI）。
- 状態依存キーを使っていないユーザーには、T3のUIが不要な「置き換えボタン」を
  常時表示しないこと（実際の検出結果と連動させる）。
- 回帰テスト（表示条件の分岐、ダイアログ文言等）を追加する。

## 関連

- [ADR-192](../adr/192-state-dependent-mode-key-warning-and-guided-override.md) 決定2・3
- [ADR192-T2](adr192-t2-warning-detection-and-dispatch.md)（検出ロジック、実装済み）
- [ADR192-T3](adr192-t3-awase-settings-guided-replacement.md)（置き換えUI、実装済みだが
  検出結果と未連動）
- [ADR-176](../adr/176-behavioral-calibration-of-ime-mode-key-shadow-overrides.md)
  （同種のawase.exe⇔awase-settings連携の前例、`calibration_ipc.rs`）
- `crates/awase-windows/src/msime_key_assignment.rs::check_and_warn`
  （既存のダイアログ表示パターンの参考実装）
