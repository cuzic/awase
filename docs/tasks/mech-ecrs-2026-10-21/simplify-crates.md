# 簡素化候補: crate・ツール・テスト基盤・CI・ドキュメントの単位(2026-10-22)

所有者の問い「もう撤去できるコードはないか(挙動が変わっても、シンプルになるならそちらを選ぶか)」に対する、
**crate・ツール・テスト基盤・CI・ドキュメント**の単位の棚卸し。読み取りのみの調査で、コードは変えていない。
機能・設定単位(simplify-features)、機構単位(simplify-mechanisms)は別の担当。

- 基準: `origin/develop` = `6a1e2d1e`(2026-10-22)。
- 行数は `git ls-files … | xargs wc -l`(または `.rs` のみの Python 集計)で数えた実数。推測は「推定」と書く。
- CI の実行回数は `gh run list -R cuzic/awase --workflow <file> --limit 200`(2026-10-22 取得)。
  リモートのブランチの有無は `git ls-remote --heads origin`(396 本、同日)。
- cargo は実行していない(ローカルでビルド・テストをしない規約)。

## 1. 結論の表(削れる量が大きく、失うものが小さい順)

| ID | 対象・場所 | 削れる量 | 挙動の変化・失う検証 | 依存(ガード・ワークフロー・ADR・公開) | リスクと必要な確認 | 推奨 |
|---|---|---|---|---|---|---|
| S1 | `tools/e2e/ime_key_matrix/results/`(450 ファイル)・`out/`(24) | データ 58,236 行・**10.3 MB**(リポジトリの tools/ の 74%) | なし。どのワークフロー・スクリプトも読まない(`.github` からの参照 0)。2026-09-19 の 1 コミット以後更新なし | docs からの参照 7 件(ADR-191・193・BUG-151・docs/tasks 3 件)。パス名の言及のみ | 参照先の docs を「`675be51a` 時点のツリー」へのリンクに書き換える。clone サイズは履歴に残るので減らない(作業ツリーと grep のノイズが減る) | 撤去する(docs のリンクをコミット固定に) |
| S2 | ブランチ削除済みの一回限りワークフロー 6 本: `adr195-t7-focus-loss-verify`・`b1-notify-probe-verify`・`closed-loop-ignored`・`e2e-uwp-inputsite-hook-watchdog-probe`・`keymap-learn-trace-gji`・`keymap-learn-trace-native`(+`mozc-shm-e2e` は S9) | yml 554 行 + 専用スクリプト `tools/e2e/t7-focus-loss-check.ps1`(76)・`tools/e2e/b1-notify-probe.ps1`(51) = **681 行** | なし。トリガーは `push: branches: [<削除済みブランチ>]` と `workflow_dispatch` だけで、develop 上では走らない。最終実行は 2026-09-23〜10-18 | `tools/keymap-learn/analyze_features.py`(118)は keymap-learn-trace だけが使う。ADR-195/196 の本文が手順として参照 | 再実験するときは `git show <sha>:.github/workflows/…` で戻せる旨を ADR/BUG に 1 行足す | 撤去する |
| S3 | `crates/e2e-uwp-inputsite-probe`(自称「使い捨てプローブ」、issue #165) | 本番 0・crate 615 行 + S2 のワークフロー 122 行。Cargo.lock から専用パッケージ 12 個(`windows-reactor*` 一式) | なし(製品に入らない)。CI の検証も 2026-09-29 以後走っていない | 逆依存 0。`hook_watchdog.rs`・`message_handlers.rs:616`・`bootstrap.rs:452` のコメントが実験結果として言及 | コメントの参照を「実験 crate は `<sha>` で撤去」に直す | 撤去する |
| S4 | CI に参照されない `crates/awase-windows/examples/` 12 本: `text_store_probe`・`spike_langbar_input_mode`・`msime_key_assignment_settings_probe`・`msime_native_composing_probe`・`gji_composition_probe`・`ime_observation_spike`・`spike_calibration_decisive_v2`・`richedit_tsf_probe`・`spike_egui_ime_control_probe`・`spike_egui_himc_reassociation_probe`・`spike_kana_lock_probe`・`two_imm32_windows_probe` | **6,278 行**(examples 全体 19,148 行の 33%) + dev-dependency `windows-core`(`spike_langbar_input_mode` 専用、Cargo.toml:73 のコメント) | なし。ワークフロー・tools/ のどれも build/実行しない(`ci.yml:654` のコメントに `richedit_tsf_probe` の名前があるだけ)。通常の CI でコンパイルもされない(clippy・check に `--all-targets`/`--examples` が無い)ので、既にビルドが壊れていても誰も気付かない状態 | docs(ADR・BUG)からの言及あり(各 1〜5 件)。examples 同士の相互言及のみ | 実機調査で再利用する予定があるかを所有者に確認。残すなら最低限 windows-build に `--examples` を足さないと腐る | 撤去する(所有者が再利用予定を挙げたものだけ残す) |
| S5 | `e2e-ime.yml` の `observe` 構成 | 552 構成中 **observe 458**(期限付き 146・期限無しの既存 312 = `observe_grandfathered.txt` 314 行)。下の 3 節に直近の実行実績 | observe は結果を表に出すだけで何も止めない。撤去で失うのは「手で only= を指定して回す再現の部品」 | `test_e2e_plan.py`・`observe_audit.py`・`observe_grandfathered.txt` が棚卸しを固定。期限切れ(until < 今日)は現在 0 件。直近 766 実行で一度も走っていない observe 16 件、2 週間以上走っていない observe 139 件(3 節) | 構成 1 つは Python の辞書 1〜6 行。削れる行数そのものは推定 600〜900 行(yml 1,918 行の plan 部分)だが、効果は「読む量」と plan の複雑さ | 所有者判断(3 節の「一度も走っていない」構成から外す) |
| S6 | `awase-linux`・`awase-macos`(スタブ) | 本番 2,069 行 + toml 79 = **2,148 行**。Cargo.lock 専用パッケージ 13 個(`evdev`・`nix`・`cocoa`・`core-graphics` ほか) | 利用者には見えない(配布物なし)。失うのは「コアを別 OS から呼べる」ことのコンパイル検証(Linux の `cargo nextest run --workspace --lib` で lib がビルドされる、`ci.yml:38` の awase-linux 9 件のテスト) | 逆依存 0。`architecture_guard.rs:5502-5503,6155-6156` が両 crate のパスを読む(撤去時にガードも直す)。ADR-019(OS 非依存)の象徴 | コアの API 変更のたびに追随修正が要る(直近 `a2bc7bc8`・`7e3c8c3d`、2026-10-10)。OS 非依存性そのものは `layer_boundary_guard` と core 系 crate の `windows` 非依存ガードで別途守られている | 所有者判断(Linux/macOS 版を出す予定が無いなら撤去) |
| S7 | 一時の mutants ワークフロー: `mutants-scope-investigation.yml`(「ADR-194 … 専用の一時ワークフロー」)・`mutants-actuation-confluence-windows.yml`(「棚卸し専用の一時ワークフロー」)と専用設定 `.cargo/mutants-bug158-scope.toml`・`.cargo/mutants-actuation-confluence-scope.toml` | 61+56+52+49 = **218 行** | なし。最終実行 2026-09-23(18 回中 16 失敗・2 回とも失敗) | 他から参照なし(ワークフロー 2 本だけが toml を読む) | ADR-194/180 の本文の手順への言及を `<sha>` 固定に | 撤去する |
| S8 | ブランチはあるが 2 週間以上走っていない一回限りのワークフロー: `bug103-verify`(53)・`bug112-probe`(61)・`env-events-probe`(56)・`msi-upgrade-verify`(48)・`preedit-verify`(23)・`vocab-renderer-verify`(23)・`vocab-suspend-verify`(23)・`settings-screenshot`(223)・`wt-probe`(94)・`f16-wt-verify`(117) | yml **721 行** + `tools/e2e/wt/`(806、wt-probe・f16-wt-verify 専用) + 例 `spike_bug112_ime_wnd_race_probe`(237) | 失うのは「そのブランチへ push すれば再現が回る」手軽さ。最終実行 2026-10-04〜10-11 | `tools/e2e/{env_events,msi_upgrade,config_verify,tray_cache_clear}` は常設ワークフロー(config-verify・tray-cache-clear)も使うので残す | 各ブランチの持ち主(BUG/ADR)の結論が出ているかを確認。`settings-screenshot` は設定画面のスクリーンショット作成の道具として再利用の可能性 | 所有者判断(BUG の結論が出たものから撤去) |
| S9 | Mozc 共有メモリの CI: `mozc-shm-e2e.yml`(119)+ `tools/mozc_shm_ci/`(1,146)+ `tools/adr265_stage2/`(15 ファイル 2,155、どのワークフローからも参照なし) | **3,420 行** | なし(ブランチ `ci/mozc-shm-e2e*` は削除済み、最終実行 2026-10-18)。ADR-265 段階 2 の測定は結果文書 `docs/tasks/adr265-stage2-measurement-result.md` に記録済み | ADR-268・docs/tasks 4 件が手順として参照。mozc-readable は進行中の取り組み(段階 0 未完) | 段階 0・belief 接続の続きで同じ測定を再び回すかを所有者に確認 | 所有者判断(当面使わないなら撤去し、結果文書に `<sha>` を記す) |
| S10 | `crates/actuation-choke-point-macro`(ADR-158 TE3「観測フェーズ」) | crate 78 行 + 利用 1 か所(`awase-windows-base/src/focus/class_names.rs:217`)の属性 4 行・doc 8 行 | `[actuation-record]` の debug ログ 1 行が消え、`can_use_imm32_cross_process` が `const fn` に戻せる。`callers` 欄は「17+ call sites …」という文章で、許可リストへの昇格(本来の目的)は進んでいない | 逆依存 1(awase-windows-base)。`tools/e2e/ime_key_matrix/testdata/awase-baseline-fixed-excerpt.log` にログ断片がある(テストデータのみ) | `[actuation-record]` を数える解析スクリプトが無いことを確認済み(grep 0 件)。ADR-158 TE3 の status を「見送り」に | 撤去する |
| S11 | `architecture_guard.rs` の「対象が既に無い」再導入防止(tripwire)ガード: `warmup_gate_third_arg_is_never_a_bare_literal_in_production_code`(`from_applied_or_belief_unless_off_drift` はリポジトリに 0 件、`WarmupImeOn` 型ごと撤去済み)・`conv_write_call_sites_are_target_explicit`(`set_ime_romaji_mode_with_target_async` 0 件)・`kp_stage_shadow_ime_toggle_never_reintroduces_case3_forced_actuate`(`explicit_ime_action_case3_off` 0 件) | テスト約 110 行(doc 含む、3 件)。同種の「削除済み/再導入」言及は同ファイルに 37 か所(全部が空振りとは限らない、未精査) | 削除済み API の名前を再び書いたときに落ちなくなる。warmup の件は**型が無いので書けばコンパイルエラーになり、ガードは空振り** | ファイル 9,763 行・#[test] 176 件。dylint 4 本(971 行)と一部重複 | 3 件は確認済み。残り 34 か所は 1 件ずつ対象の有無を確かめる必要あり | 3 件は撤去する。残りは所有者判断(tripwire を残す方針か) |
| S12 | `docs/tasks/`(80 項目) | 18,588 行・2.5 MB。うち調査の生データ(`layering-inventory-2026-10-06` 3,429・`world-model-write-inventory` 1,075 ほか) | なし(文書) | コード・ツール・`.claude/` から 70 か所、docs から 44 ファイルが参照。`adr-index-consistency` は ADR の索引のみ見る | 移動すると参照 114 か所の書き換え。削除はしない | `docs/tasks/archive/` へ移すのは費用対効果が低い。**索引(`docs/tasks/index.md`、状態 1 列)を作る**だけを推奨 |
| S13 | `docs/adr/`(387 ファイル・110,742 行・10.3 MB)、`docs/adr/review/`(89 ファイル・2.2 MB)、`docs/known-bugs/`(208 ファイル・22,637 行) | 削除なし | なし | ADR status は 4 区分の概算で 採用/実装 177・提案 23・却下/置換 22・その他 52(frontmatter の文字列による機械分類、推定)。BUG の索引の状態は「解決済み(コード確認のみ)」72・「機構撤去済み」28 ほか | 30 行ルールを超える BUG ファイルが 95/208(中央値 53 行、最大 1,274 行) | 削除はしない。①提案のまま 1 か月以上動いていない ADR の status を「保留」に揃える ②`review/` を ADR 本文の付録扱いとして索引から外す ③「機構撤去済み」28 件を索引でまとめる。所有者判断 |

小さいもの(表に入れなかった。撤去の価値が小さいか、役割がある):

- `awase-settings-wgpu`(6 行): wgpu 機能を PR の CI から外すための置き場(CI 高速化 #653)。残す。
- `awase-vkmap`(97 行): `awase-windows-base/src/vk.rs:807` と `typing_stress` が使う。VK 表の置き場という層の規則(ADR-019)の役目がある。残す。
- `awase-replay`(176 行): ime-core と awase-windows の再生ハーネス。残す。
- `win32-worker`(213 行): 利用は `observer/indicator_glyph.rs` の 1 か所だが crates.io に `0.1.0` を公開済み(`repository = cuzic/win32-worker`)。`win32-async` は `win32-worker` に依存していない(CLAUDE.md の記述「win32-async が win32-worker を使う」は現状と違う)。統合は公開物に影響するので残す。
- `measured-macro`(200 行、`#[measured]` 42 か所)・`xtask-adr-evidence`(668 行、CI `adr-evidence-consistency`)・`awase-build-support`(80 行、2 逆依存): 現役。
- `awase-keymap-learn`(8,227)と `awase-keymap-learn-win`(4,719): 純粋ロジックと実機ドライバで役割は重なっていない。残す。
- `tools/e2e/ime_key_matrix/device/`(6 ファイル 274 行、ADR-190/191 の実機用スクリプト)・`patches/`(2 ファイル 190 行、「未適用」の修正案): 参照は docs とコメントのみ。S1 と一緒に撤去してよい(推定、所有者確認)。
- ablation スクリプト 7 本(74 行)はすべて `e2e-ime.yml` の構成から参照されている(a4/a5/a6/a8/a9/a10/a11)。撤去候補なし。
- `tests/golden/` 6 ファイルはすべてテストから参照されている(`pseudo_ime_exhaustive.rs` 4・`explicit_press_exhaustive.rs` 1・`ime_key_sequence_golden.rs` 1)。撤去候補なし。

## 2. crate の一覧(本番/テスト行数・逆依存)

`.rs` の行数(`tests/` 配下をテストとして分けた。`src/` 内の `#[cfg(test)]` は本番側に含む)。逆依存は各 Cargo.toml の依存表(`[dependencies.X]` 形式を含む)を見た。

| crate | 本番 | テスト | 逆依存 |
|---|---|---|---|
| awase-windows | 79,829 | 22,197 | settings・e2e-uwp-inputsite-probe・keymap-learn-win |
| awase-ime-core | 37,218 | 0 | bug-report・windows-plans・hook-core・windows |
| awase-settings | 13,404 | 0 | (バイナリ) |
| awase-key-effect | 8,683 | 0 | ime-core |
| awase-keymap-learn | 8,227 | 0 | bug-report・settings・key-effect・keymap-learn-win・windows |
| awase-windows-base | 5,766 | 0 | key-effect・ime-core |
| awase-scancode | 5,091 | 0 | windows |
| awase-keymap-learn-win | 4,719 | 20 | settings |
| awase-hook-core | 4,063 | 0 | windows |
| awase-gji-config | 3,370 | 0 | windows-base・bug-report・key-effect・ime-core・windows |
| timed-fsm(crates.io 0.5.0) | 3,192 | 107 | awase・windows |
| awase-bug-report | 3,115 | 0 | windows |
| awase-windows-plans | 2,423 | 0 | windows |
| **awase-linux** | 1,431 | 0 | **なし** |
| xtask-adr-evidence | 668 | 0 | なし(CI ジョブ) |
| **awase-macos** | 638 | 0 | **なし** |
| **e2e-uwp-inputsite-probe** | 577 | 0 | **なし** |
| win32-async | 482 | 0 | windows |
| win32-worker(crates.io 0.1.0) | 213 | 0 | windows(1 か所) |
| measured-macro | 200 | 0 | windows-base |
| awase-replay | 176 | 0 | ime-core・windows |
| awase-vkmap | 97 | 0 | windows-base・windows |
| awase-build-support | 80 | 0 | settings・windows |
| **actuation-choke-point-macro** | 65 | 0 | windows-base(1 か所) |
| awase-settings-wgpu | 6 | 0 | settings |
| lints/*(dylint 4 本) | 971 | 0 | CI `dylint` |

## 3. e2e の構成と実行の実績

集計: `observe_audit.py` の `load_configs()` で現行の構成 552 件を取り出し、`e2e-ime.yml` の直近 766 実行
(2026-09-20〜2026-10-22、`gh run list --limit 1000` の全件)の全ジョブ名 `e2e (<構成名>, …)` と突き合わせた。

| 区分 | 件数 |
|---|---|
| 構成(pass / fail / observe) | 91 / 3 / 458 |
| 期間内に一度も走っていない | observe 16(`tsx-susp-*` 11・`cal-fast-*msime*` 5) |
| 最後に走ったのが 2026-10-08 より前(2 週間以上走っていない) | observe 139・pass 14・fail 1 |
| 1 回しか走っていない observe | 31 |
| 期限(until)切れ | 0(最も早い期限は 2026-11-04) |

注意: observe 構成はジョブを失敗させないので、ジョブの conclusion(observe の 382 件が「成功のみ」)は
判定結果を表さない。判定は各 run の summary の表にしか無く、ここでは集計していない(未確認)。

- 一度も走っていない 16 件のうち `tsx-susp-bug171-*`・`tsx-susp-rend-*`・`tsx-susp-survey-*` は BUG-171 用で、
  BUG-171 の索引の状態は「未修正(コード上は残存・CI では途中の語の ESC を再現できず、2026-10-04)」。構成は置いたが一度も
  回されていないので、期限 2026-11-04 を待たずに削るか、所有者が回すかを決める候補。
- 2 週間以上走っていない observe 139 件は、`observe_grandfathered.txt`(期限無し)に載っているものが中心と推定(未照合)。

## 4. CI ワークフロー 29 本の実行実績(2026-10-22 取得)

常設(develop/PR で走る): `ci`・`e2e-ime-smoke`・`config-verify`・`tray-cache-clear`・`scancode-vm-verify`(週 1 の cron あり)・`gen-changelog`・`release`。
`e2e-ime` は push 先ブランチ(ci/e2e-ime ほか 10 本)と `workflow_dispatch`、`ci.yml` の `workflow_call` から。

それ以外の 21 本は「特定ブランチへの push か手動」でしか走らない。うちリモートにブランチが無いもの:
`adr195-t7-focus-loss-verify`・`b1-notify-probe-verify`・`closed-loop-ignored`・`e2e-uwp-inputsite-hook-watchdog-probe`・
`keymap-learn-trace-gji`・`keymap-learn-trace-native`・`mozc-shm-e2e`・`msi-tray-row-verify`(本日実行、作業中)・
`f16-wt-verify` と `wt-probe` は片方のブランチだけ残っている。

## 5. 所有者に聞くべき質問

1. Linux/macOS 版(`awase-linux`・`awase-macos`)を出す予定はあるか。無ければ撤去し、OS 非依存性は既存のガードに任せてよいか(S6)。
2. CI に載っていない `examples/` 12 本(6,278 行)のうち、今後の実機調査で使う予定のものはどれか。残すものは CI で `--examples` のコンパイルを足してよいか(S4)。
3. 一回限りの検証ワークフロー(S2・S7・S8・S9)は「結論が出たら撤去し、ADR/BUG にコミットを記す」を規約にしてよいか。
4. `observe` のまま一度も走っていない e2e 構成は、期限の有無に関係なく削ってよいか(S5)。
5. 「削除済み API の再導入防止」のガードは、型が無くなってコンパイルで止まるものに限り撤去してよいか(S11)。
