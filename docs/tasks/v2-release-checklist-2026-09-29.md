---
title: awase v2.0.0 リリースの完成条件チェックリスト
status: 進行中（2026-09-29 夜時点の状態を反映。A1〜A3・B1・B2 は完了、A4-1/A4-2 は #373 でマージ済み、A4-3・B4・C1 は PR オープン中〈#376・#379・#377〉、D4 は未着手）
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
| スコープ | 設計変更（A）＋ ADR-199 の残り全部（B）。A4・B4・D4 も v2 に含める |
| BUG-172・ts-chrome 残課題 | **v2 のブロッカー**（修正方針を決めて実装するまでリリースしない） |
| 配布 | v2 リリース時に **v1 を保守終了**にする（Scoop・更新通知の v1/v2 非区別問題は、v1 パッチを出さなければ起きない） |
| バージョン・backport | v2.0.0。v1 への backport は**重大バグのみ**（BUG-168・170・171 などは重大度で個別判断） |
| `keys.ime_toggle` 既定 | **空にする**（ADR-202 の 2026-09-26 の「当面空にしない」を覆す。理由: 決定15 の条件だった移行〈T16〉が実装済み）。PR #367 で実装 |
| `ConfirmMode` の旧値 | 保存時に `wait` へ書き換える（PR #366） |
| `keys.ime_detect.*` の既定（IMEオン・IMEオフ） | **空にする**（棚卸しの推奨「残す」を覆す） |
| `engine_on_ime_key` / `engine_off_ime_key` | **撤去する**（awase が IME に能動送信する設定） |
| 無変換/変換の単独タップ | Suppress / Passthrough の設定に従う。ただし IME 側がトグルに割り当てているときは、**生キーを抑止し、awase が belief に従って ON/OFF を明示で inject** する（ADR・敵対レビューを通してから実装） |
| BUG-173（v1 への backport） | **しない**。v2 への移行を案内し、告知に既知の問題として載せる |
| C2 の(2)〜(5) | ブロッカーから外す（(1) は BUG-172 の修正で再判定）。発生したら BUG を起票する運用 |
| `keyboard_model` | 残す（棚卸しの推奨どおり。物理配列の軸で学習や IME 設定では代替できない） |

## A. 設計変更

- [x] **A1 calibration を config.toml から cache.toml へ移す**。**移設は行わない（ADR-198 決定3）**。手動較正の撤去（PR #304）で `AppConfig::calibration` と `[[calibration]]` の読み書きは既に無い。旧 config.toml に残っていても読込エラー・警告にならず無視される。PR #364 でマージ済み。
- [x] **A2 `ConfirmMode` を `Wait` / `NgramPredictive` の2択にする**。PR #366 でマージ済み（`f2eb36f3`）。旧値は読込時に `wait` 扱い＋廃止警告、保存時に `wait` へ書き換え。
- [x] **A3 ADR-198（永続化先の分類）を確定**。PR #364 で ADR-198 と `review-2026-09-24-07` の status を実態に同期済み。
- [ ] **A4 設定項目の整理**（棚卸し: PR #368 マージ済み、`docs/tasks/v2-a4-config-cleanup-inventory-2026-09-29.md`。A4-1・A4-2 は PR #373〈ADR-207〉でマージ済み、A4-3 は PR #376〈ADR-206〉が未マージ）:
  - [x] **A4-1**（PR #373、`279268f3`・`7c94702a`。静的 `shadow_action` を `is_japanese_ime()` に依らず採用するよう直し、追随が失われる懸念に対処。**実機確認は未実施**）`keys.ime_detect.*` の既定を空にする。`sync_direction` は `is_japanese_ime()` に依らず `shadow_action` より優先される（`key_pipeline.rs:1172`）ため、ATOK・IME 未同定・MS-IME 互換モードで失われる追随がないかを先に調べる。
  - [x] **A4-2**（PR #373、`279268f3`）`engine_on_ime_key` / `engine_off_ime_key` を撤去する（`bootstrap.rs:721` → `platform.rs:1353` の能動送信）。既存 config の値は警告して無視。key selection ファミリーなので golden・architecture_guard を更新する。
  - [ ] **A4-3**（ADR-206 は確定・develop 済み。実装は **PR #376 がオープン中、未マージ**。マージ後にチェック）無変換/変換の単独タップの再設計（`muhenkan/henkan_solo_tap_ime_action` の置き換え）。二重トグル・GJI の「@」（BUG-113/124）・belief が古いときの逆動作・受動化方針（ADR-178/191/199）との整合を ADR で検討する。
  - [x] `keyboard_model` は残す（作業なし）。

## B. ADR-199 の残り

- [x] **B1 T16 / ADR-202**: 0x19 を `Hankaku/Zenkaku` 行から役割逆算する専用経路。**実装済み**（PR #341・#342、`runtime/mod.rs::kanji_shadow_action`）。当初この項目を「未着手」と書いたのは、ADR-199 の T16 行の古い記述を信じた誤り。
- [x] **B2 T11**: `keys.ime_toggle` の既定を空にする。PR #367 でマージ済み（`88f9c1f8`）。e2e `sc-kanji-role-toggle`／`sc-kanji-role-nontoggle` 4ジョブ成功（run 36539956282）。明示の `VK_KANJI` を持つ既存 config は尊重し、消さない。
  - 副次的な発見: 旧既定の `VK_KANJI` は、GJI の 0x19 の役割判定を `explicit_overlap` で常に無効にしていた。既定を空にすると、能動の経路が既定設定で初めて動く。**既定 `[keys]` の GJI・実機での確認は未実施。**
- [ ] **B3 T1 の実機確認 (a)(d)(e)**: (a) 半角/全角を変えていないカスタム TSV に `Hankaku/Zenkaku` 行が残るか、(d) 互換モードを触っていない環境で `NoTsf3Override2` が無いか、(e) `Scancode Map` の F13 構成の実タイピング（(e) は実 Chrome で一部確認済み）。
- [ ] **B4 T17 Phase 4**（v2 に含める、所有者決定）: MS-IME 本体の無変換/変換の値2（トグル）を能動的に尊重する配線。対象は「値2でトグルに該当する場合」だけ。**PR #379 がオープン中（未マージ）**。値2は CI で作れず（設定アプリに「キーの割り当て」が出ない、レジストリ直書きは反映されない）、MS-IME 本体では入力中・変換中でも `ctx.composing` が常に false のため、入力中の除外は採らない（所有者決定、確定文字列を捨ててよい）。**検証はホストテストのみ・実機未検証**の状態でマージする（所有者了承済み、根拠は PR #378 と `v2-b4-msime-toggle-phase4-plan-2026-09-29.md` 8節）。
- [ ] **B5 T7 の残り**: 09（残る能動書き込みの棚卸し）への反映。
- [ ] **B6 ADR-201**: 実機確認と矢印キー対応（段階0〜3は実装済み）。

## C. ブロッカーの不具合

- [ ] **C1 BUG-172（実 Chrome × 外部から閉じられた IME が ON に戻らない）**（修正は **PR #377〈ADR-205〉がオープン中、未マージ**。マージ後も実 Chrome での修正確認が別途必要）:
  - 再現済み: 別プロセスが注入した半角/全角（0xF3）や `VK_IME_OFF` で、GJI × 実 Chrome は 10/10 で閉じたまま（run 36540419485）。awase が Chrome の開閉を観測した件数は 0（`Imm32Unavailable`、belief は古い ON のまま）。物理キーでは起きない。MS-IME は再現できていない（注入キーが効かない）。メモ帳経由は影響なし。
  - 「ゲートに開閉を要求する」案は採らない。フォーカス変更を挟んでも 0/10（PR #365）。
  - 修正: ADR → 敵対レビュー → 実装を進行中（観測の代替。compartment 通知の購読、または打鍵直前の読み取り専用照合。belief への反映は `reduce()` 経由のみ）。
  - 未測定: IME 側ホットキー（Alt+~）、モード切替アイコン。MS-IME の閉じ方。
- [ ] **C2 ts-chrome 高速打鍵（BUG-168 / ADR-200）の残課題**（2026-09-26 のメモ、**未再確認**）: 候補窓が残ったまま GJI が OFF のときの回復低下、StaleConfirm の romaji 再送重複（BUG-075 系）、Escape 経路、他の reinit 呼び出し元、起動直後の IME モード不整合と awase 主スレッド7秒停止（未解明）。まず再現するかを確認する。 **再確認済み(2026-09-29)**: (1) 合成条件で再現(ADR-200 で回復量が減る)、(2)〜(5) は強制シナリオで測定済み(2)(3)は入ったが重複・消失なし、(4)は Chrome で入らない、(5)は再現せず。v2 ブロッカーにしない提案。詳細は [BUG-168](../known-bugs/BUG-168.md) 末尾。

## D. 実機確認待ち

- [ ] **D1 BUG-163**: GJI/MS-IME × メモ帳/実 Chrome で、最初の打鍵が欠落しないこと。
- [ ] **D2 ADR-203 / BUG-170・171**: OFF 前に1語確定→物理 OFF→1秒以内に物理 ON→即打鍵。ON キー単独タップ直後の遅延（想定30〜60ms）の再測定。
- [ ] **D3 ADR-178 領域A撤去**: 実機 A/B（`review-2026-09-24-09` の「実機 A/B 手順」）。物理 Ctrl は SendInput で作れない。
- [ ] **D4 MS-IME 本体の学習（ADR-196 T2）**（v2 に含める、所有者決定）: 半角カタカナ（conv 0x0013）が学習モデルの Conv に無く復号失敗する件と、`--adopt-pending-judgement`（精度≥0.95）の採用経路の検証。未着手。

## E. リリース作業

- [ ] **E1 v1 の保守終了の告知**: README・更新通知・Scoop の案内。v1 の最後のパッチを出すなら、v2 リリース前に `release-v1develop-to-v1main` で済ませる（`report.awase.cc` は `latest-release` を v1/v2 ラインごとに返す〈`0a38590a`〉）。
- [x] **E2 backport の棚卸し**: 完了（`v2-e2-v1-backport-inventory-2026-09-29.md`）。重大と判定したのは BUG-173 のみで、**所有者決定（2026-09-29）により backport せず v2 への移行を案内する**。BUG-171・172 は develop でも未修正のため backport 不可。E1 の告知に『v1 に残る既知の問題』（同文書の一覧）を載せる。
- [ ] **E3 リリース**: `release-develop-to-main`（CHANGELOG、2.0.0 への bump、タグ、GitHub Release）。`docs/changelog.en.html` も更新する。

## 運用メモ

- ディスクが逼迫している（空き 1〜2GB）。エージェントの並列ビルドは共有 `target`（`CARGO_TARGET_DIR=/home/cuzic/rust-nicola/target`）を使い、他の worktree の `target` は消さない。
- 設計判断を含む変更は、ADR 起票 → `opus-adversarial-consult` で収束 → 実装の順にする。
