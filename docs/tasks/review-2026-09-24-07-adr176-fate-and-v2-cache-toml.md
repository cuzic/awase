---
title: ADR-176 手動較正パネルの撤去と v2 方針（永続化先の分類）の ADR 化
status: 未着手
created: 2026-09-24
related_adr: ["ADR-176", "ADR-191", "ADR-195", "ADR-058", "ADR-125", "ADR-162"]
source_review: 俯瞰レビュー（2026-09-24）の A-8 / A-10 / C-4 / C-5
---

# ADR-176 の去就と v2 方針の記録（俯瞰レビュー A-8 / A-10 / C-4 / C-5）

索引: [11](review-2026-09-24-11-low-priority-backlog.md)。裏取り基準は `5877f982`（origin/develop）。初版は `cbae84ff` 基準で、その後の差分（PR #293〜#296）は本件の対象ファイルに触れていない。

## 背景

v2 方針（calibration を config.toml から cache.toml へ移す、ConfirmMode の2択化、`app_overrides` の維持）は、リポジトリ内の docs/ADR には書かれていない。
出典は 2026-09-23 のユーザー決定を記録したリポジトリ外のメモだけ（レビューの指摘 A-10）。リポジトリの読み手はこのメモを辿れない。
そのため ADR にするときは、メモを参照せず決定内容そのものを本文に書く。

手動較正の扱いについては、既存の決定がある。本タスクは撤去するかどうかをゼロから決めるものではない。

- ADR-191 で「較正結果を適用する」側は撤去済み: `9dc52c89`（`apply_calibrated_mode_keys` 削除）。設定画面の「測定のみ」の説明は `660e77af` で追加された。
- ADR-191 の撤去表（`docs/adr/191-ime-is-source-of-truth-observe-not-write.md:325`）には「削除: `calibrated_mode_key.rs` ほか較正結果の適用 … **決定4が再実装する**（ユーザー判断: 削除して製品化で新規に作る）」とある。その製品化が ADR-195 の学習（`awase-keymap-learn`）。
- ADR-195 段階6（[ADR195-T6](adr195-t6-adr176-wizard-integration.md)、developマージ済み）は、ADR-176 の較正タブを学習プロセスの起動導線として**意図的に**使っている。

残っている論点は二つ。一つは、測定だけで結果を使わない「手動較正パネル」という UI と、それを支える awase.exe 側の仕組みを残す価値があるか。もう一つは、v2 の永続化先をどう分類するか。

## 現状（`5877f982` で裏取り済み）

### A-8: 手動較正は「測定するだけ」で、awase.exe 側の仕組みが丸ごと残っている
- 設定画面の `tab_calibration`（`crates/awase-settings/src/main.rs:3268`）が「測定結果は保存されますが、現バージョンでは実際のIME判定には適用されません（測定のみ）」と表示する（`:3275`）。
- 学習ウィザードは同じタブの末尾で、`ui.collapsing("学習ウィザード（全キー自動測定、実験的）", …)`（`:3419`）という折りたたみの中にある。タブの主役は適用されない手動較正で、実際に予測に使われる学習は折りたたまれている。`use_learned_keymap_table` の既定は true（`src/config.rs`）。
- `[[calibration]]` は `config.toml` に書き込まれる（`main.rs:6349` `on_disk.calibration.push`、`:6357` `config.calibration.push`）。一方、本番コードで `config.calibration` を読む箇所は無い（`src/config.rs` の構造体転送と、`state/calibrated_mode_key.rs` のテストだけ）。
- 残っているコードの規模:
  - `crates/awase-windows/src/calibration_ipc.rs` 242行、`state/calibrated_mode_key.rs` 772行、settings 側の `calibration_panel.rs` 180行と `calibration_result_window.rs` 142行。合計1,336行。
  - `calibrat` を含む行数（`grep -ci`。分岐の数ではない）は、`hook.rs` 31、`runtime/focus_tracking.rs` 86、`runtime/message_handlers.rs` 27。コメント行を除くとそれぞれ 20 / 74 / 22 行。
  - `lints/actuation_call_guard/src/lib.rs:113` の `RESTRICTED_CALLS` 許可リストに `probe_ime_open_for_calibration` がある。
- `calibrated_mode_key.rs` に依存する、較正専用ではないコード。撤去するときの影響範囲はここまで含む:
  - `crates/awase-windows/src/gji_charset_autodetect.rs:123-137`（`current_fingerprint` が `ConfigFingerprint` を返す）と `:434-450`（`ConfigFingerprint::MsIme`・`CalibratedModeKey` の構築）。
  - `crates/awase-windows/src/msime_key_assignment.rs:278` の `current_registry_fingerprint_hash`。本番で呼んでいるのは上記 `gji_charset_autodetect.rs:435` だけ。
  - `crates/awase-gji-config/src/keymap.rs:165` の `relevant_rows_for_vk`。doc（`:160`）に「`ConfigFingerprint::Gji::relevant_row` 専用」とあり、外部からの呼び出しは `gji_charset_autodetect.rs:131` だけ。
  - `crates/awase-settings/src/main.rs:3291` の `explicit_config_conflict_reason` と、`:3314/3365/3370/6372/6404` の `calibration_ipc` の参照。

### A-10: 記憶メモの前提が実装・ADR と合わない
- メモの「学習表も将来 `cache.toml` の `[keymap_learn]` 節に置く」は、ADR-195 段階3（`docs/adr/195-keymap-learn-productization.md:303-314`）の決定と食い違う。段階3は、`[[calibration]]` とは粒度が違うので別ファイルにすると決めている。実装もそのとおりで、学習表は `<config dir>/keymap-learn-table.json` と `keymap-learn-last-attempt.json`（`crates/awase-keymap-learn-win/src/main.rs:64-91`）に、`awase::fs_atomic::write_atomic` で書かれる。
- メモの「calibration を cache.toml へ移す」は、A-8 のとおり、読む側が存在しないデータを移すことになる。

### C-4: `save_section` の弱さ（`crates/awase-windows/src/focus/classifier.rs:409-420`）
- 現在の書き手は awase.exe 内の2箇所だけ: `:389` の `[imm_capability]`（`ImmCapabilityStore`、ADR-125）と、`:487` の `[injection_mode]`（`InjectionModeStore`、ADR-058）。awase-settings は `cache.toml` に書かない。
- 実装には次の弱点がある。一方、`config.toml`（`AppConfig::save`）は `fs_atomic::write_atomic` を使っており、扱いが違う。
  - 読込やパースに失敗すると `unwrap_or_default()` になり、他のセクションが全部消える。
  - `std::fs::write` なので書き込みがアトミックでない。
  - プロセス間のロックが無い。
- **いま起こりうる実害**は一つだけ。書き込みが途中で切れる → 次の読込でパースに失敗する → 次の保存で他のセクションが全部消える。awase.exe はシングルスレッドなので、プロセス内の競合は起きない。
- **v2 で awase-settings も cache.toml に書くようになった場合の仮定シナリオ**: 二つのプロセスが同時に保存すると、後から書いた方が勝ち、相手のセクションが消える。現状では起きない。
- `[imm_capability]` / `[injection_mode]` は再学習で元に戻るので、これまでは許容できた。
- 「cache.toml を消す人はめったにいない」という前提は、[04](review-2026-09-24-04-sample-config-and-user-docs.md) A-9 のクリアメニューを機能させた時点で崩れる。

### C-5: 「較正は無人化できない」という非対称
- 手動較正の結果は今は誰も使っていない。したがって「無人化できない較正を守る設計」は現時点では要らない。
- v2 で問題になるのは学習表だけ。学習は注入で自動測定できるが、その間はウィンドウを前面に保ち、キーボードを専有する。
- 学習にかかる時間:
  - MS-IME 本体（run 35945955606）: `docs/tasks/adr196-t2-msime-learning-open-issues.md:11` に presses=1891・judgement=rejected・verify_accuracy=0.940 の記録がある。所要時間（1301秒）はリポジトリ外のメモにしかない（未確認）。
  - GJI: ADR-195 の成功基準（`195-…md:468-483`）に「17分の全数学習」と「20分以内」の基準がある。段階0で削れるのは多くて34%、段階2（自己検証）は未計測とされている。GJI の run を特定した実測記録は無い。
- つまり学習表は、「消してよいデータ」にも「無人で短時間に作り直せるデータ」にも当てはまらない。別 JSON にアトミックに書く現行方式（ADR-195 段階3）を維持する理由はここにある。

## タスク

- [x] **(1) v2 方針がいまもユーザーの決定かを確認する。**（2026-09-24 確認済み: 変わらない） ADR 起票の前に行う。対象は ConfirmMode の2択化、`app_overrides` の維持、calibration の移設。メモは古くなっている可能性がある。
- [ ] **(2) ADR-176 の手動較正パネルを撤去する。**
  - 撤去の方向は ADR-191 決定4と ADR-195 段階6で既に出ている。ADR-176 本体（`docs/adr/176-behavioral-calibration-of-ime-mode-key-shadow-overrides.md`）の frontmatter `status` に「ADR-191 で適用側を撤去（`9dc52c89`）、ADR-195 学習に置き換え、測定 UI も撤去」と追記し、撤去 PR を出す。`176-implementation-tasks.md` は更新対象ではない。status の同期は [10](review-2026-09-24-10-adr-status-and-stale-docs-sync.md) と重なるので、本タスクで直し、10 からはここを参照するだけにする。
  - 撤去範囲: 上記の1,336行、hook/focus_tracking/message_handlers の較正分岐、`RESTRICTED_CALLS` の `probe_ime_open_for_calibration`、settings UI、上記「較正専用ではない依存」のうち較正専用になったもの。
  - 較正バイパス（`hook.rs:471` `notify_calibration_key_if_target`、`:766` `set_calibration_target`、`runtime/focus_tracking.rs:606` `begin_calibration_bypass`）は ADR-176 専用で、学習は使わない。ADR-195 段階1（`195-…md:425-430`）は、学習プロセス名の照合で別にバイパスし、`calibration_ipc` の IPC は使わないと決めている。ADR-191 決定4（`191-…md:261`）は「既存 IPC（`WM_CALIBRATION_START`）で較正窓をバイパス」と書いたままだが、これは段階1で置き換えられた。
  - **ただし `calibration_ipc.rs` を丸ごと消してはいけない。** 学習のバイパス判定 `is_keymap_learn_process_name`（`calibration_ipc.rs:52`）がこのファイルにあり、`focus/tracker.rs:247` が呼んでいる。消すと学習中に awase.exe が学習プロセスへのキーを変換してしまう。この関数（と、較正以外に呼び出し元が残るなら `is_awase_settings_process_name`〈`:45`、`message_handlers.rs:1137`・`focus_tracking.rs:753`〉）を別モジュールへ移してから撤去する。
  - タブは消さない。ADR195-T6 の学習導線は残し、タブを学習中心に組み直す。
  - `opus-adversarial-consult` は撤去 PR のレビューで使う（依存の見落としの確認）。撤去するかどうかの判断そのものには何ラウンドも回さない。すでに方向が出ているため。
- [ ] **(3) `ConfigFingerprint` を残すかどうかを、(2) の前に [06](review-2026-09-24-06-keymap-learn-staleness-wiring.md) と決める。**
  - `ConfigFingerprint::Gji { session_keymap, relevant_row }` / `MsIme { registry_value_hash }` は、キーマップ設定の指紋を計算する唯一の既存実装。06 は学習表の指紋（`PersistedTable::with_fingerprint`）を書くタスク。
  - 流用するなら、型と `current_fingerprint` / `current_registry_fingerprint_hash` / `relevant_rows_for_vk` を `calibrated_mode_key.rs` の外へ移してから撤去する。流用しないなら、これらも撤去範囲に入れる。
- [ ] **(4) `[[calibration]]` の読み込み互換を確かめる。**
  - `AppConfig` には `deny_unknown_fields` が付いていない（`src/config.rs:2233` のテストコメント）。フィールドを構造体から消すだけで、既存の config は読める。
  - 実装は要らない。互換テストを1件足すだけでよい。
  - 副作用: awase-settings で保存すると `AppConfig::save` がファイル全体を書き直すので、`[[calibration]]` が消える。読む側が無いので消えてよい、と ADR-176 の status に明記する。
- [x] **(5) v2 方針を ADR として起票する。**（ADR-198 草案、consult 未実施） frontmatter 規約に従い、index.md に短い1行を足す。
  - 範囲は「永続化先の分類」に絞る: `config.toml`（ユーザー設定）、`cache.toml`（再学習で戻る観測キャッシュ）、学習表 JSON（ADR-195 段階3、再生成コストが大きい）。
  - メモの `[keymap_learn]` 節案は、ADR-195 段階3に合わせて取り下げると書く。calibration の移設は、(2) の撤去で不要になると書く。
  - ConfirmMode の2択化は、確定エンジンの設定（`src/config.rs:66` `enum ConfirmMode`）で、永続化先の話とは関係ない。同じ ADR に入れると、片方だけ実装済みのときに status の追随が難しくなる。そこで、[04](review-2026-09-24-04-sample-config-and-user-docs.md) で推奨モードを統一したあと、別の ADR（または既存 ADR への追記）で扱う。`app_overrides` の維持は、1行の現状確認として v2 ADR に入れてよい。
- [x] **(6) `save_section` をアトミックにする。**（実装済み: `docs/review-07-08` ブランチ） v2 とは独立に、いますぐ小さく直す。`std::fs::write` を `awase::fs_atomic::write_atomic`（`src/fs_atomic.rs:35`。keymap-learn-win と同じ呼び方）に置き換える。
- [ ] **(7) `save_section` が読込に失敗したときの扱いを決める。** 候補は、上書きしない／`.bak` に退避してから書く／警告だけ。v2 でセクションを増やすなら必須、増やさないなら優先度は低い。

### 採らなかった案
- 「complexity-budget の1-in-1-out の返済材料として記録する」: `probe_ime_open_for_calibration` の削除は `RESTRICTED_CALLS` の1件削除に当たり、記述自体は正しい。ただし `.claude/rules/complexity-budget.md` はまだ発効しておらず、返済の記録先（ADR-162 E3 の定例棚卸し）も整っていない。撤去 PR の本文に「RESTRICTED_CALLS −1」と書くにとどめ、独立のタスクにはしない。
- 「ADR-191 の撤去量の指標に加算する」: 指標1の対象は `crates/awase-windows/src` と `src` だけで、判定は「撤去フェーズ（P0〜P2）の末」（`191-…md:293`）。settings 側の322行は対象外。P0〜P2 の数値はすでに記録済み（`:331`、2026-09-21）。撤去 PR が指標の判定を変えることはないので、タスクから外す。撤去量を参考に残すなら、ADR-176 の status に書く。

## 受け入れ条件

- **ドキュメント**（Linux でレビューできる）: 以下がそろっている。
  - v2 ADR が起票され、index.md に1行ある。
  - ADR-176 本体の frontmatter `status` が更新されている。
  - 本タスクの `status` が同期している。
  - ConfirmMode の2択化の扱い（別 ADR にするか）が v2 ADR に明記されている。
- **撤去を実施したとき**:
  - Linux で次が通る。
    - `cargo check --target x86_64-pc-windows-msvc -p awase -p awase-windows -p awase-settings`
    - `cargo nextest run -p awase-windows --test architecture_guard --test layer_boundary_guard`
    - `DYLINT_RUSTFLAGS="-D warnings" cargo dylint --all -p awase-windows -- --target x86_64-pc-windows-msvc`（`RESTRICTED_CALLS` の該当行も消えていること）
  - ホスト Linux の `cargo test --lib` で、`[[calibration]]` を含む TOML を `AppConfig` として読めるテストが通る（`src/config.rs:2233` 付近の既存テストと同じ形）。
  - windows-build CI が通る。
  - 実機で目視確認する: `tab_calibration` に手動較正パネル（`calibration_panel`）が無く、学習ウィザードが折りたたまれずにタブの最上段に出る。
- **`save_section` を修正したとき**: `classifier.rs` は `#[cfg(windows)]` の内側にある（`focus/mod.rs:15-16`）。既存テスト（`:495` の `#[cfg(test)]`）は、ホスト Linux のテストバイナリには存在しない（エラーも skip 表示も出ない）。検証方法は次のどちらかを実装時に選ぶ。
  - 案a: テストは `classifier.rs` に置き、`cargo check --target x86_64-pc-windows-msvc -p awase-windows --tests --lib` が通ることを確認する。テストの実行は windows-build CI に任せる。
  - 案b: `save_section` の「読込→マージ→書込」をパスだけを受け取る純粋な関数にして、`cfg(windows)` の外（root クレート、または awase-windows の gate されていないモジュール）へ移す。テストはホスト Linux の `cargo test` で走らせる。(7) で分岐が増えるなら、案b のほうが検証しやすい。
  - (6): 保存後のファイルが完全な TOML になっていることを確認するテスト。
  - (7): 方針を決めてから期待値を書く。例: 壊れた `cache.toml` を置いた状態で保存させ、壊れたファイルが消されずに残るか、`.bak` に退避されること。

## 他ファイルとの依存

- [06](review-2026-09-24-06-keymap-learn-staleness-wiring.md): **06 → 07(2)の順**。`ConfigFingerprint` の流用を決める前に撤去しない（タスク(3)）。
- [03](review-2026-09-24-03-release-bundle-keymap-learn-win.md): **03 → 07(2)の順**。03 で案B（学習UIを隠す）を選ぶと、撤去後のタブが空になる。03 の判断を先に済ませる。03 の案A で学習表を persist する件は、07(5) の分類に従う（07 → 03 の向き）。
- [04](review-2026-09-24-04-sample-config-and-user-docs.md): **04 → 07(5)の ConfirmMode 部分**。推奨モードを統一してから扱う。逆に、04 A-9 のクリアメニューは 07(7) の判断を参照してよい（07 → 04）。
- [10](review-2026-09-24-10-adr-status-and-stale-docs-sync.md): ADR-176 の status は 07(2) で直す。10 はそれを参照する（07 → 10）。
- 既存タスク [ADR195-T6](adr195-t6-adr176-wizard-integration.md): 学習導線はこのタスクの成果物。撤去で壊さない。

## 未確認点

- MS-IME 本体の学習の所要時間（1301秒、run 35945955606）はリポジトリ内の記録では確認できない。GJI は ADR-195 に「17分の全数学習」「20分以内の基準」があるが、run を特定した実測は無い。
- hook.rs / focus_tracking.rs / message_handlers.rs の較正関連の行のうち、較正専用ではない行の切り分け。行数を数えただけで、分岐ごとには読んでいない。
- v2 で cache.toml に載せる予定の具体的なセクション一覧（決まっていない）。

## レビュー反映メモ

2026-09-24 に Opus がタスク文書をレビューした。指摘22件はすべて `5877f982` で裏取りした。
- **反映したもの**: 1〜19、21、22。「誤りだったので反映しなかった」ものは無い。
  - 3: 所要時間の出典を分けた。
  - 4: 失敗シナリオを「現状」と「v2 の仮定」に分けた。
  - 5〜7: 既存の決定（`9dc52c89`・ADR-191 決定4・ADR-195 段階3/6）を背景に入れた。「推奨方針の記録」タスクは (5) に統合した。
  - 16: ConfirmMode は別 ADR にする案を本文に取り込んだ。
  - 18: consult は撤去 PR のレビューで使う形にした。
  - 10・19: 採らなかった案として理由を残した。
- **一部だけ変えたもの**:
  - 8 の行番号: `gji_charset_autodetect.rs` の範囲をレビューの `121-137, 425-450` から、実測の `123-137, 434-450` に直した。
  - 15 の「今すぐ直す」: アトミック化 (6) だけにした。読込失敗時の扱い (7) は方針を決めるまで保留した。
- **変更不要だったもの**: 20（frontmatter の書式は姉妹文書とそろっていた）。
- **再確認レビュー（同日）の反映**:
  - A（反映）: 指摘14の前提「`classifier.rs` は `cfg(windows)` の外」は誤りだった。`focus/mod.rs:15-16` は `#[cfg(windows)]` と `pub mod classifier;` の組。受け入れ条件を案a（Windows ターゲットの `cargo check` + windows-build CI）と案b（純粋関数に切り出してホストで検証）の二択に直した。
  - B（反映、ただし表現を変えた）: レビューが挙げた ADR-195 の行のうち、`:434`（レビューは `:432`）の「20分前後」と `:557` の「17〜20分」は存在する。ただしより具体的なのは成功基準の節 `:468-483`（GJI の「17分の全数学習」「20分以内の基準」）なので、そちらを出典にした。
  - C（反映、ただしレビューの結論を一部訂正）: 較正バイパス（`notify_calibration_key_if_target` / `set_calibration_target` / `begin_calibration_bypass`）を学習が使わないことは確認した。ただし `begin_calibration_bypass` は `hook.rs` ではなく `runtime/focus_tracking.rs:606` にある。また、学習のバイパス判定 `is_keymap_learn_process_name` 自体が `calibration_ipc.rs:52` にあり、`focus/tracker.rs:247` から呼ばれている。「撤去して安全」とだけ書くと `calibration_ipc.rs` を丸ごと消す誤りを招くため、関数を移してから撤去する旨を撤去範囲に足した。
  - D（本文書では反映しない）: 03 `:65-66` と 04 `:159` の表現のずれは、03・04 側の修正か、10/11 でのまとめで拾う。
- **ADR-058 / ADR-125 を related に残した理由**: `[injection_mode]`（ADR-058 `058-injection-mode-cache-toml.md`）と `[imm_capability]`（ADR-125）が C-4 の対象セクション。
