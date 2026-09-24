---
title: awase-keymap-learn-win.exe がリリース成果物（release.yml / ZIP / MSI / scoop）に入っていない
status: 未着手
created: 2026-09-24
related_adr: ["ADR-195", "ADR-196", "ADR-178"]
source_review: 俯瞰レビュー（2026-09-24）の B-4
---

# 学習プロセスのリリース同梱（俯瞰レビュー B-4）

索引: [11](review-2026-09-24-11-low-priority-backlog.md)。裏取り基準は worktree の `5877f982`（PR #296 まで）。
`cbae84ff` から `5877f982` までの間に `.github/`・`wix/`・`scripts/`・`crates/awase-settings/`・`src/paths.rs` の変更は無い（`git diff --stat cbae84ff 5877f982` で確認）。
既存タスク [adr195-t7-safety-measures.md](adr195-t7-safety-measures.md) の項目5（`:85-89`、MSI 同梱・署名・アンインストール時の扱い〈ADR-177/178〉）と同件。

## 現状（裏取り済み）

- **release.yml のビルドとコピー**: ビルドは `:51` `cargo build --release --locked -p awase-windows`（awase.exe を生む）と `:58` `cargo build --profile settings-release --locked -p awase-settings` の2つだけ。"Prepare distribution" のコピーも `:64-65` の awase.exe / awase-settings.exe だけ。`awase-keymap-learn-win` のビルドもコピーも無い。
- **ZIP 経路**: ZIP は `dist/*` をまとめる（`release.yml:79`）。ただし展開後の `scripts/install.ps1:41-42` は awase.exe と awase-settings.exe を名前指定でコピーする。`scripts/uninstall.ps1:53-54` も名前指定で削除する。`dist/` に足しただけでは、install.ps1 経由のインストール先に学習 exe は入らない。
- **MSI**: `wix/main.wxs` の exe は `:76`（awase.exe、Component `MainExe`）と `:107`（awase-settings.exe、Component `SettingsExe`）の2つだけ。ショートカットは settings だけ（`:221`）。perUser インストールでは File を KeyPath にできない（ICE38、`:71-73`）。そのため、各 exe は独立した `<Component Guid=...>` に入れ、HKCU の `RegistryValue KeyPath="yes"` を持たせる構成になっている（`:106-110`）。
- **scoop**: `bin = @("awase.exe")`（`release.yml:129`）、`shortcuts` は awase-settings.exe だけ（`:130`）、`persist = @("config.toml", "layout", "data")`（`:131`）。
- **CI**: `.github/workflows/ci.yml` に `keymap-learn` の文字列は無い。windows-build job のビルドは `-p awase-windows`（`:282`）と `-p awase-settings`（`:378`）だけ。Linux job の `cargo nextest run --workspace --lib`（`:46`）は同クレートの lib をホスト向けにしかコンパイルしない（`#[cfg(windows)]` 部分と bin は対象外）。Windows 向けの bin ビルドは、トリガが診断ブランチまたは workflow_dispatch に限られた `adr195-t7-focus-loss-verify.yml:26` と `b1-notify-probe-verify.yml:25`（`--examples`）にしか無い。**develop への PR では、learn-win の Windows release ビルドは検査されていない。**
- **起動失敗時の表示**: 設定画面は `crates/awase-settings/src/main.rs:1091` で `resolve_relative_to_exe("awase-keymap-learn-win.exe")` を呼び、`keymap_learn_launcher::spawn_learning_process`（`keymap_learn_launcher.rs:143`）で起動する。失敗したときは `main.rs:1093-1100` で `"{path} を起動できませんでした（{e}）"` をステータスに出して return する。**握り潰しは無く、画面に表示される。** ただし文言は OS のエラー文そのままで分かりにくい。表示もボタンを押した後になる。
- **パス解決のフォールバック**: `src/paths.rs:70-95` の `resolve_relative_to` は、exe の隣とワークスペースルートの両方に見つからないと、警告ログを出して相対パスのまま返す。`Command::new` に相対名を渡すと、Windows では PATH 検索も走る。事前に「exe が無い」ことを検出するには、呼び出し側で `exists()` の判定が別に必要。
- **学習表の置き場所**: `keymap-learn-table.json` と `keymap-learn-last-attempt.json` は config.toml と同じディレクトリに書かれる（`crates/awase-keymap-learn-win/src/main.rs:64-91`、`crates/awase-windows/src/state/key_effect_runtime.rs:364-372`）。
- **署名**: release.yml・scripts・wix のどこにも `signtool` は無い。awase.exe を含め、全 exe が未署名。
- ADR195-T7 項目5は「未着手」のまま。

## 失敗シナリオ

1. 次に `release-develop-to-main` を実行すると、学習UI（ウィザード、「学習結果を使う」、軽量再検証）は出荷される。しかし、押すと「…を起動できませんでした（OS エラー）」になるボタンが出る。
2. 同梱したとしても、scoop では `scoop update` のたびにアプリディレクトリが差し替わる。そのため、persist されていない `keymap-learn-table.json` が消え、採用済みの学習結果が失われる。
3. learn-win が Windows 向けにだけ壊れていても、PR の CI では検知できない。壊れていることがリリース時に初めて分かる。

## タスク

方針を先に決める（A / B / C のどれか）。

- [ ] **案A（同梱）**: 変更対象は次のとおり。
  - [ ] `release.yml`: `cargo build --release --locked -p awase-keymap-learn-win` を足し、`dist/` へコピーする。
  - [ ] `ci.yml` windows-build: 同じ release ビルドを足す（PR で壊れを検知するため）。
  - [ ] `wix/main.wxs`: `SettingsExe` と同じ形の新しい Component を足す（新 GUID、`<File Source="dist\awase-keymap-learn-win.exe" />`、HKCU の `RegistryValue KeyPath="yes"`）。`File` 要素を1行足すだけでは ICE38 に抵触する。
  - [ ] `scripts/install.ps1`（コピー）と `scripts/uninstall.ps1`（削除）に learn-win を名前指定で足す。
  - [ ] scoop: `bin` には**足さない**。設定画面は exe の隣を探すので、ZIP に入っていれば見つかる。`bin` に足すと PATH に shim が作られ、実キー注入を行うプロセスがシェルから名前で起動できるようになってしまう。代わりに `persist` へ `keymap-learn-table.json` と `keymap-learn-last-attempt.json` を足す。
  - [ ] MSI: 学習表2ファイルは MSI が入れるファイルではないので、アンインストールでは消えない。ADR-178（アンインストール時のユーザーデータ保持）の対象一覧にこの2ファイルを明記するかを決める。
  - [ ] 署名: 現状は全 exe が未署名。learn-win だけを署名対象にするかは、T7 項目3（セキュリティソフトの検知）の実機結果を見て判断する。「署名しないまま同梱する」も選択肢に入る。
- [ ] **案B（機能フラグで隠す）**: cargo feature または設定キーで学習UIを隠し、次リリースでは同梱しない。egui の描画コードは単体テストしにくい。そのため、表示可否を返す純粋関数（例: `learn_ui_visible(feature, exe_exists)`）を切り出し、そこに単体テストを書く。cargo feature 方式なら、release.yml の `-p awase-settings` ビルドで feature が off になることも確認する。ただし隠し設定が増えるので、ADR-162 の減算志向とは逆向きになる。
- [ ] **案C（exe が無ければボタンを無効化して案内する）**: `resolve_relative_to_exe` の結果に `exists()` を1つ足す。exe が隣に無いときだけボタンを無効化し、「この配布物には学習プロセスが含まれていません」と表示する。機能フラグなしで「押しても起動しないボタン」の出荷を防げるので、案A を当面見送る場合の最小案になる。判定は純粋関数に切り出し、単体テストを付ける。
- [ ] どの案でも: 起動失敗時の文言（`main.rs:1096-1099`）を、OS エラー文だけでなく「学習プロセスが見つかりません」と分かる形にするか判断する。PATH 検索へのフォールバックを避けるため、`exists()` で事前に判定してから起動する。

## 受け入れ条件

- **Linux で走るもの**:
  - 案A: `crates/awase-windows/tests/wix_installer_guard.rs` に「`dist\awase-keymap-learn-win.exe` を持つ Component があり、HKCU の KeyPath を持つ」アサートを足す。`cargo test -p awase-windows --test wix_installer_guard` で通ること。**注意: このテストは現在 ci.yml のどの job からも実行されていない（grep で確認）。** CI に載せる（Linux job の `--test` 列挙に足す）ことも併せて行う。
  - 案B / 案C: 表示可否の純粋関数の単体テスト（`cargo nextest run -p awase-settings`。ホストで走るかは実装位置次第、`#[cfg(windows)]` の外に置く）。
- **windows-build CI（PR 上）**:
  - 案A: ci.yml の windows-build で `cargo build --release --locked -p awase-keymap-learn-win` が通ること。
  - release.yml は `push: tags: v*` と `workflow_dispatch` でしか走らない（`:3-15`）。PR 上では検証できず、dry-run 相当の経路も無い。プレリリースタグ（`-` を含むタグは `prerelease: true`、`:105`）で workflow_dispatch を走らせ、ZIP/MSI の中身を確かめる方法はある。ただし scoop step（`:107-141`）にはプレリリースを除外する条件が無く、bucket を書き換えてしまう。この方法を使うなら、scoop step をプレリリースでは skip する修正が先に要る。
- **実機（または windows-latest の手動ジョブ）**:
  - 案A: MSI・ZIP+install.ps1・scoop の各経路でインストールし、設定画面から学習が起動すること。scoop update と MSI の上書きアップグレードの後も `keymap-learn-table.json` が残り、採用状態が維持されること。
  - 案B / 案C: インストール後に学習UIが出ない（案C ではボタンが無効で案内文が出る）こと。

## 他ファイルとの依存

- 案A（学習UIを出す）を選ぶときだけ、[01](review-2026-09-24-01-adr196-adoption-and-mismatch-check.md)・[02](review-2026-09-24-02-settings-status-display.md) が先に直っている必要がある（03 は 01・02 に依存する）。案B / 案C は 01・02 に依存しない。
- 案B を選ぶ場合は、[07](review-2026-09-24-07-adr176-fate-and-v2-cache-toml.md) の UI 再編（手動較正が主役のタブ構成）の判断と連動する。
- 案A の persist 追加は、[07](review-2026-09-24-07-adr176-fate-and-v2-cache-toml.md) の v2 方針（calibration を cache.toml へ移す）でファイル配置が変わる場合、07 の決定に追随する必要がある（07 → 03 の向き）。

## 未確認点

- 署名の方式（T7 項目5）は未決。T7 項目3（AV 検知）の実機結果も未確認。
- ADR-177（Restart Manager による終了）が learn-win の実行中アップグレードにも関係するかは未確認。関係する場合は `related_adr` に足す。
- 案A で MSI 同梱したとき、ADR-178 の「自己修復」（Permanent 化）の対象に learn-win の exe を含めるべきかは未検討。

## レビュー反映メモ

Opus レビュー（`opus-taskdoc-review-03-...`）の指摘を1件ずつ worktree `5877f982` で裏取りした。
- 1-1（ビルド step の行番号とパッケージ名）: 正しい。反映した（`:51` `-p awase-windows`、`:58` `-p awase-settings`）。
- 1-2（scoop の bin と shortcuts の混同）: 正しい。反映した。
- 1-3（起動失敗は画面に表示済み）: 正しい（`main.rs:1093-1100`）。現状欄に移し、タスクを「事前判定と文言」に書き換えた。
- 2-1（windows-build は learn-win をビルドしていない）: 正しい。未確認点から現状欄へ移し、案A に ci.yml への追加を足した。
- 3-1（install.ps1 / uninstall.ps1 の抜け）、3-2（ICE38 のため Component 単位で足す）: 正しい。反映した。
- 3-3（scoop の bin は不要、persist が抜けている）: 正しい。config.toml は exe 相対で解決され（`main.rs:585`）、学習表はその隣に置かれる。反映し、`related_adr` に ADR-178 を足した。
- 3-4（release.yml は PR で走らない）、3-5（案B は純粋関数で試験）、3-6（アップグレード後の保持）、3-7（全 exe 未署名）: 正しい。反映した。
- 3-2 の自動試験案は、裏取りの過程で **`wix_installer_guard` が CI で実行されていない**ことが分かったので、その旨も追記した（レビューに無い追加の発見）。
- 4-1（案C）: 採用した。4-2（依存に条件を付ける）: 反映した。
- 5-1（docs/tasks での frontmatter 混在）: 反映しなかった。review-2026-09-24-* シリーズ11本は同じ書式で揃っている。書式を統一するかは本ファイル単独ではなく、索引（11）側で判断する事項のため。
- 5-2（ADR-177 の関連性）: 未確認のため `related_adr` には入れず、未確認点に残した。
