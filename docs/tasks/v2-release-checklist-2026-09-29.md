---
title: awase v2.0.0 リリースの完成条件チェックリスト
status: 進行中（2026-09-30 時点。A・B・D4・E2 は完了。C1 は実機で #377 の効果を確認したが偽 OFF 疑い1件が未解消〈BUG-176〉。残りは D1〜D3 と X 系の実機確認、E1・E3）
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
| ADR-208（古い applied で絶対指定キーが省略され続ける固着） | **v2 のブロッカーにしない**。GJI で外部変化を検出できないときに限られ、対処には WT×GJI の「@」（BUG-124）の A/B 測定が先に要る。既知の制限とし、発生すれば BUG を起票する |
| 単独タップ（無変換/変換）の Suppress | Suppress は IME を動かさない。エンジン停止時に生キーが IME に届く一方向は仕様。`keys.ime_on/off` に無変換/変換を書いた設定は Suppress でも発火 |
| B4 の入力中の除外 | 不要（未確定文字列を捨ててよい） |
| `keyboard_model` | 残す（棚卸しの推奨どおり。物理配列の軸で学習や IME 設定では代替できない） |

## A. 設計変更

- [x] **A1 calibration を config.toml から cache.toml へ移す**。**移設は行わない（ADR-198 決定3）**。手動較正の撤去（PR #304）で `AppConfig::calibration` と `[[calibration]]` の読み書きは既に無い。旧 config.toml に残っていても読込エラー・警告にならず無視される。PR #364 でマージ済み。
- [x] **A2 `ConfirmMode` を `Wait` / `NgramPredictive` の2択にする**。PR #366 でマージ済み（`f2eb36f3`）。旧値は読込時に `wait` 扱い＋廃止警告、保存時に `wait` へ書き換え。
- [x] **A3 ADR-198（永続化先の分類）を確定**。PR #364 で ADR-198 と `review-2026-09-24-07` の status を実態に同期済み。
- [x] **A4 設定項目の整理**: 完了。棚卸しは PR #368。
  - [x] **A4-1** `keys.ime_detect.*` の既定を空にした（PR #373、ADR-207）。0x16/0x1A の静的 `shadow_action` は `is_japanese_ime()` に関係なく採用（`vk::is_static_idempotent_open_key`）。
  - [x] **A4-2** `engine_on_ime_key` / `engine_off_ime_key` を撤去した（PR #373）。2026-08-15 より前に GUI で保存した config に残る値は、起動時にトレイで通知し、保存時に消す。
  - [x] **A4-3** 無変換/変換の単独タップを再設計した（PR #376、ADR-206）。役割由来の開閉は Passthrough のときだけ発火、Suppress は IME を動かさない。旧 `*_solo_tap_ime_action` は読込時に bare へ移行（同じキーの bare が既にあれば移行しない）。**「@」の実機 A/B は未実施**（マージ条件から外した）。Ctrl↑ に専用の actuation が無いことはガードで固定。
  - [x] `keyboard_model` は残す（作業なし）。

## B. ADR-199 の残り

- [x] **B1 T16 / ADR-202**: 0x19 を `Hankaku/Zenkaku` 行から役割逆算する専用経路。**実装済み**（PR #341・#342、`runtime/mod.rs::kanji_shadow_action`）。当初この項目を「未着手」と書いたのは、ADR-199 の T16 行の古い記述を信じた誤り。
- [x] **B2 T11**: `keys.ime_toggle` の既定を空にする。PR #367 でマージ済み（`88f9c1f8`）。e2e `sc-kanji-role-toggle`／`sc-kanji-role-nontoggle` 4ジョブ成功（run 36539956282）。明示の `VK_KANJI` を持つ既存 config は尊重し、消さない。
  - 副次的な発見: 旧既定の `VK_KANJI` は、GJI の 0x19 の役割判定を `explicit_overlap` で常に無効にしていた。既定を空にすると、能動の経路が既定設定で初めて動く。**既定 `[keys]` の GJI・実機での確認は未実施。**
- [x] **B3 T1 の実機確認 (a)(d)(e)**: CI で確認（PR #382、`v2-b3-t1-ci-verification-2026-09-29.md`、run 36569131073）。(e) F13 をトグルにした構成の実 Chrome 実タイピングは 4/4 PASS（#367・#373・#376 の後）、(d) `NoTsf3Override2` はランナー 3 台すべてで値なし（前回と合わせて 4/4。決定 17 の `None` はトグル扱いのままでよい）。(a) `Hankaku/Zenkaku` 行が残るかは、CI のハーネスが TSV を直接書くため測れず、ADR-186 の実機サンプルでの確認のまま。 **追記（2026-09-29、実機 dragonflyg4 の読み取り）**: 実機の GJI が保存した `config1.db`（2026-09-21 保存、191行の完全な表）で、`Hankaku/Zenkaku` 行と `Kanji` 行が DirectInput=IMEOn・Precomposition/Composition/Conversion=IMEOff の4状態すべてに残っていた（既定どおりのトグル）。(a) の「GUI が保存しても行が残る」を裏づける。表の並びは整列済みで、利用者が半角/全角を変えたかどうかは表からは分からない。
- [x] **B4 T17 Phase 4**: 実装済み（PR #379）。MS-IME 本体の無変換/変換が値2（トグル）のとき役割（ImeToggle）として扱う。入力中の除外はなし（所有者決定）。**値2は CI で作れず（設定アプリに「キーの割り当て」が出ない）、実際の開閉は未検証**（ホストテストのみ）。 **追記（2026-09-29）**: 実機（日本語 UI）の設定アプリを UI Automation で操作して保存先を確定した: `HKCU\Software\Microsoft\IME\15.0\IMEJP\MSIME` の DWord `IsKeyAssignmentEnabled`（0/1）と `KeyAssignmentMuhenkan`/`Henkan`（0=IME-オン, 1=IME-オフ, 2=IME-オン/オフ, 3=既定）。**トグルが Off でも値は保存され、設定画面は Off の間は既定値を表示するだけ**（実機の元の状態は Enabled=0・4値とも2）。#378 の CI 直書きは値名もパスも合っていた。CI で12構成（`Setting`/`MoSetting` の更新、実機の全値の再現、F2 で開いた状態からの無変換/変換）を試したが、全て対照と同一で IME は反転しなかった（run 36588886306・36647985699、ブランチ `ci/e2e-b4-keyassign`、develop には入れない）。**CI では値2の実効果を作れないと結論**。残る有力な仮説は、ランナーが US 配列で MS-IME が JP106 と認識していないこと（キーボード種別の上書きは HKLM＋再起動が必要で CI 内では不可）。実機での確認が残る。
- [ ] **B5 T7 の残り**: 09（残る能動書き込みの棚卸し）への反映。
- [x] **B6 ADR-201**: 完了（未検証項目は ADR-201 に明記）。矢印キーのキー名対応は #340 で完了・CI 実機確認済み。送出の拡張キーフラグ要否・設定画面の実クリック保存・共有違反時のエラー表示・トレイのバルーンは未検証の既知の制限（所有者決定 2026-09-29）。

## C. ブロッカーの不具合

- [ ] **C1 BUG-172（実 Chrome × 外部から閉じられた IME が ON に戻らない）**: 修正済み（PR #377、ADR-205）。外部注入 IME キーの直後300msの監視窓で、読み済みの開閉状態の 1→0 を検出したときだけ実状態へ追随する（開け直しはしない）。**GJI かつ `Imm32Unavailable` に限る**（MS-IME・InputRelay・TsfNative は対象外）。CI（各10試行）で GJI×実 Chrome は追随 10/10（observed 0→10、`kiu`→`ka`）、偽の追随は物理キー相当・メモ帳・MS-IME で無し。 **【2026-09-30 実機確認・要判断】** 実機（dragonflyg4、GJI、Edge）で #377 の効果を確認: 外部注入の VK_IME_OFF の後、#377 の直前は `．`（追随せず Engine ON）、マージ後と develop 先端は `z`（IME 閉＋追随）。ON の準備を変えても先端は 9/9 正常、新しく起動した awase の手動（Ctrl+変換で ON）も正常。ただし**最初に起動した awase での手動の連続試行で、IME が開いたまま awase が OFF へ追随する偽 OFF/無変化が3回**出た（再現条件不明。[BUG-176](../known-bugs/BUG-176.md)）。F2 単独・変換単独で ON にした場合とマウスで ON にした後は未確認。ブロッカーとして扱うか観察継続にするかは所有者判断。
  - 未検証: 追随後にモードキーを押して期待状態になること、MS-IME×実 Chrome での awase 自身の `VK_IME_OFF`（効かなかった記録あり）、実機。
  - 既知の制限（ADR-208）: 外部変化を検出できず `applied` が古いままだと、絶対指定キーが省略され続ける固着があり得る。v2 のブロッカーにはしない。
- [x] **C2 ts-chrome 高速打鍵（BUG-168 / ADR-200）の残課題**（2026-09-26 のメモ、**未再確認**）: 候補窓が残ったまま GJI が OFF のときの回復低下、StaleConfirm の romaji 再送重複（BUG-075 系）、Escape 経路、他の reinit 呼び出し元、起動直後の IME モード不整合と awase 主スレッド7秒停止（未解明）。まず再現するかを確認する。 **再確認済み(2026-09-29)**: (1) 合成条件で再現(ADR-200 で回復量が減る)、(2)〜(5) は強制シナリオで測定済み(2)(3)は入ったが重複・消失なし、(4)は Chrome で入らない、(5)は再現せず。v2 ブロッカーにしない提案。詳細は [BUG-168](../known-bugs/BUG-168.md) 末尾。 **所有者決定（2026-09-29）: (2)〜(5) はブロッカーから外す。(1) は BUG-172 の修正（#377）で再判定。**

## D. 実機確認待ち

- [ ] **D1 BUG-163**: GJI/MS-IME × メモ帳/実 Chrome で、最初の打鍵が欠落しないこと。 手順: [v2-manual-verification-guide-2026-09-29.md](v2-manual-verification-guide-2026-09-29.md)。 **CI 部分（PR #387、2026-09-29）**: 素の EDIT・TSF 相当（tsf）× GJI の `--cold`（awase 起動直後の最初の文字）を各10回、消失・リテラル化 0（メモ帳の代わり。MS-IME・RichEdit・実 Chrome は構成のみ追加、未実行）。実機のメモ帳・実 Chrome での実打鍵は残る。
- [ ] **D2 ADR-203 / BUG-170・171**: OFF 前に1語確定→物理 OFF→1秒以内に物理 ON→即打鍵。ON キー単独タップ直後の遅延（想定30〜60ms）の再測定。 手順: [v2-manual-verification-guide-2026-09-29.md](v2-manual-verification-guide-2026-09-29.md)。 **CI 部分（PR #387、run 36655470405）**: `typing_stress --mode=reopen`（構成 `sc-reopen-*`）。GJI×tsf（gap 300/600/900ms）・変換キー・実 Chrome・MS-IME が全 PASS（OffCold 固着・StaleConfirm/flush の `escape=true` は 0、GJI は ON 後の最初の語が cold 経路で `Reopen(BeliefSync…)` が毎試行発火）。**BUG-170 の修正を撤去した負の対照（`ablations/a8`）で、入力先のテキスト・cold 経路・固着は修正版と同じ PASS だった**（awase 自身の ImeOn 遷移が GjiFsm を同期するため、物理 OFF→ON は BUG-170 の固着条件〈Windows Terminal の物理 F2 → Unwarranted〉に届かない）。違いは journal の `Reopen(BeliefSync…)` だけで、`--require-sync` で撤去版が FAIL（24/24）になる。**したがって CI で確認できるのは「ADR-203 の同期が働いたこと」までで、ユーザーに見える不具合（固着・ESC・文字の消失）の再現・防止は実機でしか確認できない**。遅延: `[vk-send]`→セッション確認は p50 約 45ms・最大 81ms（ADR-203 D2 の定義、windows-latest）。「打鍵→最初の `[vk-send]`」約61ms はほぼ打鍵の押下時間で awase の遅延ではない。GJI の ATOK プリセットで 0xF2 は ON にならない（ハーネスの既定は 0x16）。実機での確認は残る。
- [ ] **D3 ADR-178 領域A撤去**: 実機 A/B（`review-2026-09-24-09` の「実機 A/B 手順」）。物理 Ctrl は SendInput で作れない。 手順: [v2-manual-verification-guide-2026-09-29.md](v2-manual-verification-guide-2026-09-29.md)。
- [x] **D4 MS-IME 本体の学習（ADR-196 T2）**（v2 に含める、所有者決定）: 半角カタカナ（conv 0x0013）が学習モデルの Conv に無く復号失敗する件と、`--adopt-pending-judgement`（精度≥0.95）の採用経路の検証。**CI検証完了**（run 36569030546、5/5でdecode_errors=0・採用/再採用success・精度0.953〜0.973。[adr196-t2-msime-learning-open-issues.md](adr196-t2-msime-learning-open-issues.md)「再検証」節。コード変更なし）。

## E. リリース作業

- [ ] **E1 v1 の保守終了の告知**: README・更新通知・Scoop の案内。v1 の最後のパッチを出すなら、v2 リリース前に `release-v1develop-to-v1main` で済ませる（`report.awase.cc` は `latest-release` を v1/v2 ラインごとに返す〈`0a38590a`〉）。
- [x] **E2 backport の棚卸し**: 完了（`v2-e2-v1-backport-inventory-2026-09-29.md`）。重大と判定したのは BUG-173 のみで、**所有者決定（2026-09-29）により backport せず v2 への移行を案内する**。BUG-171・172 は develop でも未修正のため backport 不可。E1 の告知に『v1 に残る既知の問題』（同文書の一覧）を載せる。
- [ ] **E3 リリース**: `release-develop-to-main`（CHANGELOG、2.0.0 への bump、タグ、GitHub Release）。`docs/changelog.en.html` も更新する。

## 運用メモ

- ディスクが逼迫している（空き 1〜2GB）。エージェントの並列ビルドは共有 `target`（`CARGO_TARGET_DIR=/home/cuzic/rust-nicola/target`）を使い、他の worktree の `target` は消さない。
- 設計判断を含む変更は、ADR 起票 → `opus-adversarial-consult` で収束 → 実装の順にする。

- E1 の告知文の下書き: [v2-e1-v1-eol-announcement-draft-2026-09-29.md](v2-e1-v1-eol-announcement-draft-2026-09-29.md)
