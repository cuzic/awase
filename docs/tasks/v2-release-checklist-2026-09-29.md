---
title: awase v2.0.0 リリースの完成条件チェックリスト
status: 起草（所有者決定 2026-09-29 を反映、各項目の着手はこれから）
created: 2026-09-29
related_adr: ["ADR-198", "ADR-199", "ADR-200", "ADR-201", "ADR-202", "ADR-203"]
---

# awase v2.0.0 完成条件（2026-09-29）

v2 ラインは `develop` → `main`（`.claude/rules/main-develop-branch-flow.md`）。
`develop` の `Cargo.toml` は 1.21.0 のままなので、リリース時に 2.0.0 へ上げる。
状態は根拠つきで書く。根拠が無いものは「未再確認」と明記する。

## 所有者決定（2026-09-29）

| 論点 | 決定 |
|---|---|
| スコープ | 設計変更3点（A）＋ ADR-199 の残り全部（B） |
| BUG-172・ts-chrome 残課題 | **v2 のブロッカー**（修正方針を決めて実装するまでリリースしない） |
| 配布 | v2 リリース時に **v1 を保守終了**にする（Scoop・更新通知の v1/v2 非区別問題は、v1 パッチを出さなければ起きない） |
| バージョン・backport | v2.0.0。v1 への backport は**重大バグのみ**（BUG-168・170・171 などは重大度で個別判断） |

## A. 設計変更3点

- [x] **A1 calibration を config.toml から cache.toml へ移す**。**移設は行わない（ADR-198 決定3）**。手動較正の撤去（PR #304、`refactor/remove-manual-calibration`）で `AppConfig::calibration` と `[[calibration]]` の読み書きは既に無く、読み手の無いデータを cache.toml へ移す意味が無い。旧 config.toml に `[[calibration]]` が残っていても読込エラー・警告にならず無視される（`src/config.rs::test_removed_calibration_section_is_ignored_on_load`、`config_load_diag` の撤去済みキー表）。awase-settings で保存すると `AppConfig::save` の全書き直しで消える（読み手が無いので可）。残作業なし。
- [ ] **A2 `ConfirmMode` を `Wait` / `NgramPredictive` の2択にする**。`Speculative` は死んだバリアント、`TwoPhase`・`AdaptiveTiming` は撤去。既存 config に旧値が残るときの読込時の扱いを決める。`app_overrides` は現状維持。
- [ ] **A3 ADR-198（永続化先の分類）を確定**。status は「採用」だが、`review-2026-09-24-07` の (1)(5)〜(7) が未着手。
- [ ] **A4 `keys.ime_detect.*`・`muhenkan/henkan_solo_tap_ime_action`・`keyboard_model` の扱い**。「学習が完成すれば不要」の候補で未確定。v2 で外すか残すかを決める（決めるまで触らない）。

## B. ADR-199 の残り

ADR-199 の実装タスク表（T0〜T17）の現状。撤回・完了済みは含めない。

- [ ] **B1 T16 / ADR-202**: 0x19 を `Hankaku/Zenkaku` 行から役割逆算する専用経路。設計は採用済み、実装は未着手（`hook.rs` の静的 Toggle の置換範囲、`keys.ime_toggle` 既定の扱い、MS-IME 本体の 0x19）。旧 T14 は T16 に置換され撤回済み。
- [ ] **B2 T11**: `keys.ime_toggle` 既定を空にする件の文書追随（同梱 `config.toml`・`docs/usage.html`・`docs/usage.en.html`・`awase-settings` の説明）。B1 と同時。
- [ ] **B3 T1 の実機確認 (a)(d)(e)**: (a) 半角/全角を変えていないカスタム TSV に `Hankaku/Zenkaku` 行が残るか、(d) 互換モードを触っていない環境で `NoTsf3Override2` が無いか、(e) `Scancode Map` の F13 構成の実タイピング（(e) は実 Chrome で一部確認済み）。
- [ ] **B4 T17 Phase 4**: MS-IME 本体の無変換/変換の値2（トグル）を能動的に尊重する配線。値2で入力中・変換中にどう動くかの実機確認が前提。**v2 に含めるか**は B1 の後に決める。
- [ ] **B5 T7 の残り**: 09（残る能動書き込みの棚卸し）への反映。
- [ ] **B6 ADR-201**: 実機確認と矢印キー対応（段階0〜3は実装済み）。

## C. ブロッカーの不具合

- [ ] **C1 BUG-172（実 Chrome）**: MS-IME + TsfNative で閉じた IME が ON に戻らない。「ゲートに開閉を要求する」案は実 Chrome の症状を直さないと確認済み（run 36524071258）ので**その案は採らない**。実 Chrome × GJI は0/10。手順: (1) 別窓を作って前面にする方式で、実 Chrome のフォーカス変更を再現する（`chrome_probe --refocus` は SetForegroundWindow がタスクバーに拒否され `away=false`）。(2) 撤去前ビルド（`f83084b3`・`621bf93c` の前）と対照する。(3) 修正方針を決める（Chrome 系は `Imm32Unavailable` で開閉を観測できないので、観測の代替が要る）。
- [ ] **C2 ts-chrome 高速打鍵（BUG-168 / ADR-200）の残課題**（2026-09-26 のメモ、**未再確認**）: 候補窓が残ったまま GJI が OFF のときの回復低下、StaleConfirm の romaji 再送重複（BUG-075 系）、Escape 経路、他の reinit 呼び出し元、起動直後の IME モード不整合と awase 主スレッド7秒停止（未解明）。まず再現するかを確認する。

## D. 実機確認待ち

- [ ] **D1 BUG-163**: GJI/MS-IME × メモ帳/実 Chrome で、最初の打鍵が欠落しないこと。
- [ ] **D2 ADR-203 / BUG-170・171**: OFF 前に1語確定→物理 OFF→1秒以内に物理 ON→即打鍵。ON キー単独タップ直後の遅延（想定30〜60ms）の再測定。
- [ ] **D3 ADR-178 領域A撤去**: 実機 A/B（`review-2026-09-24-09` の「実機 A/B 手順」）。物理 Ctrl は SendInput で作れない。
- [ ] **D4 MS-IME 本体の学習（ADR-196 T2）**: 半角カタカナ（conv 0x0013）の復号失敗と `--adopt-pending-judgement`（精度≥0.95）の未検証。v2 に含めるかを決める。

## E. リリース作業

- [ ] **E1 v1 の保守終了の告知**: README・更新通知・Scoop の案内。v1 の最後のパッチを出すなら、v2 リリース前に `release-v1develop-to-v1main` で済ませる（`report.awase.cc` は `latest-release` を v1/v2 ラインごとに返す〈`0a38590a`〉）。
- [ ] **E2 backport の棚卸し**: BUG-168・170・171 ほか、v1 に無い修正の重大度判定。重大なものだけ `Backport of <hash>` つきで `v1-develop` へ。
- [ ] **E3 リリース**: `release-develop-to-main`（CHANGELOG、2.0.0 への bump、タグ、GitHub Release）。`docs/changelog.en.html` も更新する。

## 着手順の提案

1. 決定待ちの A4・B4・D4（v2 に含めるか）を先に確定する。
2. C1 の再現測定（別窓方式）。修正方針が見えないと日程が読めないため最優先。
3. A1〜A3（設計変更）と B1〜B2（0x19・文書追随）。
4. D の実機確認は上と並行して消化する。
