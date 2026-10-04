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
| ADR-208（明示キーの固着ゼロの保証。旧: 古い applied で絶対指定キーが省略され続ける固着） | **2026-10-01 に所有者が保証として確認し、L0〜L3 を v2 のブロッカーにした**（ADR-208 に昇格）。旧決定（ブロッカーにしない、2026-09-29）を覆す。L3'（TsfNative・WT・GJI への拡張）は実機 A/B が条件で、v2 のブロッカーにしない。MS-IME と実 Chrome で `VK_IME_OFF` が効かない件は例外として明記。**v2 時点の保証範囲は Chrome(Imm32Unavailable)と全窓の S-2 まで。TsfNative×GJI の S-1(L3')、InputRelay の素通し(L4)、S-3・S-4(L5)は既知の制限として残る** |
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
- [x] **B5 T7 の残り**: 完了（2026-10-02、09 に「2026-10-02 時点の現状（B5 追補: ADR-212/213/208）」節を追加）。09（残る能動書き込みの棚卸し）への反映。ADR-212/213（予防的・補正的な actuation の撤去。ActivationSync・eager warmup・reinit 等、P2a〜P2d・BUG-179/180、実測は docs/experiments.md エントリ 30）の結果も反映する。
- [x] **B6 ADR-201**: 完了（未検証項目は ADR-201 に明記）。矢印キーのキー名対応は #340 で完了・CI 実機確認済み。送出の拡張キーフラグ要否・設定画面の実クリック保存・共有違反時のエラー表示・トレイのバルーンは未検証の既知の制限（所有者決定 2026-09-29）。

## C. ブロッカーの不具合

- [ ] **C1 BUG-172（実 Chrome × 外部から閉じられた IME が ON に戻らない）**: 修正済み（PR #377、ADR-205）。外部注入 IME キーの直後300msの監視窓で、読み済みの開閉状態の 1→0 を検出したときだけ実状態へ追随する（開け直しはしない）。**GJI かつ `Imm32Unavailable` に限る**（MS-IME・InputRelay・TsfNative は対象外）。CI（各10試行）で GJI×実 Chrome は追随 10/10（observed 0→10、`kiu`→`ka`）、偽の追随は物理キー相当・メモ帳・MS-IME で無し。 **【2026-09-30 実機確認・要判断】** 実機（dragonflyg4、GJI、Edge）で #377 の効果を確認: 外部注入の VK_IME_OFF の後、#377 の直前は `．`（追随せず Engine ON）、マージ後と develop 先端は `z`（IME 閉＋追随）。ON の準備を変えても先端は 9/9 正常、新しく起動した awase の手動（Ctrl+変換で ON）も正常。ただし**最初に起動した awase での手動の連続試行で、IME が開いたまま awase が OFF へ追随する偽 OFF/無変化が3回**出た（再現条件不明。[BUG-176](../known-bugs/BUG-176.md)）。F2 単独・変換単独で ON にした場合とマウスで ON にした後は未確認。ブロッカーとして扱うか観察継続にするかは所有者判断。
  - 未検証: 追随後にモードキーを押して期待状態になること、MS-IME×実 Chrome での awase 自身の `VK_IME_OFF`（効かなかった記録あり）、実機。
  - ADR-208（2026-10-01 に昇格、L0〜L3 は v2 のブロッカー）: 外部変化を検出できず `applied` が古いままだと、絶対指定キーが省略され続ける固着があり得る。ADR-208 の保証（INV-L1/L2）で解消する（L0 から実装）。
- [ ] **C3 ADR-208（明示キーの固着ゼロの保証、2026-10-01 に所有者がブロッカーとして確認）**: L0〜L3 が v2 のブロッカー（受け入れ条件は ADR-208 決定7）。**L0 は完了**（PR #416、`explicit_press_delivery` の純粋関数と全列挙テスト、現状の反例を golden に固定、挙動不変は旧 `plan` の逐語コピーとの差分 0 を約 276 万ケースで確認）。**L1 は完了**（PR #419、押下 id・`ImeEffect::SetOpen.press`・予約・`applied` の未知化〈TsfNative は L3' まで除外〉・shadow と Engine の衝突の静的な事前解決。ImmCross タイムアウトの追い送り抑止は見送り、`timed_out=` の診断ログのみ〈CI の主な構成 28 本で 1 件〉。全列挙の S-1: 1,008 → 672 件で、残りは実質 TsfNative×GJI）。**L2 は完了**（`feat/adr208-l2-warrant-explicit-press`、押下の授権〈`WarrantBasis::ExplicitPress`〉と非リピートの shadow 昇格が `is_japanese_ime`・`current_focus` を問わない。0x19 だけは TIP 同定済みを条件に残す〈所有者決定、英語 IME・US 配列の Alt+` を守る〉。未同定かつ `is_japanese_ime=false` の 0x19 は受動にして素通し。全列挙の P1 反例 165,732 → 49,248 件で S-2〈86,400〉・S-4〈29,430〉・L-9 が 0 件。実機・CI の非日本語構成と、`is_japanese_ime` を上げる対処は未着手）。**L3a(D4)は完了**（`feat/adr208-l3a-d4`、shadow no-op で物理が Suppress される窓は書く〈固定点: `plan(false)` を先に評価、Allow・リピート・TsfNative は書かない〉。全列挙の S-3 25,920 → 0、P1 反例 49,248 → 23,328。残るのは S-1〈TsfNative×GJI、L3'〉・L5・A1 外の no-op）。**L3b(CI の drift×キー行列と E2 の対照)は構成を作成し初回 CI を実施済み**(`sc-keymatrix-*` 14 構成、ハーネス `typing_stress --mode=keymatrix`・`chrome_probe --keymatrix=`、判定 `check_keymatrix.py`、設計 `docs/tasks/adr208-l3b-keymatrix-design-2026-10-02.md`、`only='sc-keymatrix-*'` で回す。初回 run 36951828252: edit/tsf×GJI・MS-IME と実 Chrome×GJI は絶対キー 1 押下・トグル 2 押下以内で全セル収束(tsf×GJI の Ctrl+変換/無変換も STUCK なし)。MS-IME×実 Chrome は ON 方向は収束、OFF 方向は fresh 対照も同程度に失敗(0.90)で ENV_EXCEPTION、トグル OFF 方向の STUCK は、`sc-keymatrix-e2-tog-{msime,gji}-chrome`〈tog:fresh の対照〉の run 36957521623 で fresh も 9/10〜10/10 で失敗し ENV_EXCEPTION と確定〈内部固着ではない、GJI 対照は全セル収束〉、外部 open は作れず GAP_NOT_MADE)。**L3 は完了**(Chrome〈Imm32Unavailable〉への D4 適用は L1+L3a で成立済みと確認=全列挙の ImmUnavailable は S-1・S-3 が 0 件〈ADR-208 の「L3 の Chrome 適用の確認」節〉。`sc-keymatrix-*` は 16 構成になり、MS-IME×実 Chrome の abs/tog 以外を `expect=pass` へ昇格、run 36965314365 で 14 構成 OK。受け入れ条件(3): `ts-*` 124 ジョブで L0 直前〈f502aec9〉と現在の develop は fail/loss/extra/literal/substitute とも 0 件で同じ〈WT×GJI の `@` は ts-* の範囲外で、TsfNative は L3a で書かないため全列挙が根拠〉、(4): `sc-*` 期待表は L3a 時点で develop と同一〈以降 awase 本体は無変更〉)。残りは実機確認。L3'（TsfNative×WT×GJI、実機 A/B が条件）・L4（InputRelay の素通し）・L5 は v2 のブロッカーにしない。
- [x] **C2 ts-chrome 高速打鍵（BUG-168 / ADR-200）の残課題**（2026-09-26 のメモ、**未再確認**）: 候補窓が残ったまま GJI が OFF のときの回復低下、StaleConfirm の romaji 再送重複（BUG-075 系）、Escape 経路、他の reinit 呼び出し元、起動直後の IME モード不整合と awase 主スレッド7秒停止（未解明）。まず再現するかを確認する。 **再確認済み(2026-09-29)**: (1) 合成条件で再現(ADR-200 で回復量が減る)、(2)〜(5) は強制シナリオで測定済み(2)(3)は入ったが重複・消失なし、(4)は Chrome で入らない、(5)は再現せず。v2 ブロッカーにしない提案。詳細は [BUG-168](../known-bugs/BUG-168.md) 末尾。 **所有者決定（2026-09-29）: (2)〜(5) はブロッカーから外す。(1) は BUG-172 の修正（#377）で再判定。**

## D. 実機確認待ち

> **2026-09-30 実機確認の実施記録**: [v2-device-verification-results-2026-09-30.md](v2-device-verification-results-2026-09-30.md)（X5・X1・X2 を実施。X3・X4・D1・D3 は未実施）。

- [ ] **D1 BUG-163**: GJI/MS-IME × メモ帳/実 Chrome で、最初の打鍵が欠落しないこと。 手順: [v2-manual-verification-guide-2026-09-29.md](v2-manual-verification-guide-2026-09-29.md)。 **CI 部分**: `sc-startup-{gji,msime}-{edit,chrome}-{on,off}` が awase 起動前に対象窓の状態を作り、ON は起動後1秒以内の初打鍵、OFF は3秒維持後の ON→打鍵、startup-align/drift/reinit を判定する。**CI 結果（2026-10-01、PR #418）**: GJI（EDIT・実 Chrome × ON・OFF 起動）と MS-IME × 実 Chrome（ON・OFF）は全 PASS（実 Chrome は Imm32Unavailable で観測が来ず `[startup-align]` が出ないのが正常なので、観察項目にして合否から外した）。**BUG-163 の本来の対象である ON 起動（起動後の最初の打鍵）は FAIL 0**（MS-IME × EDIT の ON 起動は PASS と INVALID で、INVALID はハーネスの起動検知の遅れ〈0.8〜5.7 秒〉で前提不成立。基準を1秒から2秒に緩めた）。**MS-IME × EDIT の OFF 起動は 10 回中 2 回、ON キー後の最初の文字が EDIT に出なかった**（`real_ime_open=true`、awase は Unicode 注入で送出済み。Opus の調査では PASS/FAIL で awase 側のログは同一で、差は入力欄のスレッドの停止時間〈FAIL 0.95 秒・3.2 秒、PASS 0〜0.6 秒〉。MS-IME の初回の IME オープンの初期化とハーネスのタイミングの競合と推定、確定していない）。原因が分かるまで `observe` とし、awase なしの対照は未実施。別に、ImmCross の書き込みのタイムアウトを失敗とみなして `VK_IME_ON` を重ねて送る欠陥が見つかった（ADR-208 L1 で扱う）。実機のメモ帳・体感・実機固有環境は残る。設計: [v2-d1-d3-ci-design-2026-10-01.md](v2-d1-d3-ci-design-2026-10-01.md)。
- [ ] **D2 ADR-203 / BUG-170・171**: OFF 前に1語確定→物理 OFF→1秒以内に物理 ON→即打鍵。ON キー単独タップ直後の遅延（想定30〜60ms）の再測定。 手順: [v2-manual-verification-guide-2026-09-29.md](v2-manual-verification-guide-2026-09-29.md)。 **CI 部分（PR #387、run 36655470405）**: `typing_stress --mode=reopen`（構成 `sc-reopen-*`）。GJI×tsf（gap 300/600/900ms）・変換キー・実 Chrome・MS-IME が全 PASS（OffCold 固着・StaleConfirm/flush の `escape=true` は 0、GJI は ON 後の最初の語が cold 経路で `Reopen(BeliefSync…)` が毎試行発火）。**BUG-170 の修正を撤去した負の対照（`ablations/a8`）で、入力先のテキスト・cold 経路・固着は修正版と同じ PASS だった**（awase 自身の ImeOn 遷移が GjiFsm を同期するため、物理 OFF→ON は BUG-170 の固着条件〈Windows Terminal の物理 F2 → Unwarranted〉に届かない）。違いは journal の `Reopen(BeliefSync…)` だけで、`--require-sync` で撤去版が FAIL（24/24）になる。**したがって CI で確認できるのは「ADR-203 の同期が働いたこと」までで、ユーザーに見える不具合（固着・ESC・文字の消失）の再現・防止は実機でしか確認できない**。遅延: `[vk-send]`→セッション確認は p50 約 45ms・最大 81ms（ADR-203 D2 の定義、windows-latest）。「打鍵→最初の `[vk-send]`」約61ms はほぼ打鍵の押下時間で awase の遅延ではない。GJI の ATOK プリセットで 0xF2 は ON にならない（ハーネスの既定は 0x16）。実機での確認は残る。 **2026-10-01 の再確認**: ADR-213（P2a〜P2d）・ADR-208 L0 の変更後も `sc-reopen-*` は期待表どおり（`i2_unwarranted` は全構成 0）。ただし GJI×tsf の2構成で、run の最初の試行だけ StaleConfirm の ESC（`stale_escape=1, flush_escape=1`）が出る単発の FAIL が約 6〜17% あり（develop にも同じ形が出る既存のフレーク）、原因は未調査。
- [ ] **D3 ADR-178 領域A撤去**: 実機 A/B（`review-2026-09-24-09` の「実機 A/B 手順」）。物理 Ctrl は通常の SendInput では作れないが、マーカー付き注入（debug ビルド+`AWASE_TEST_INJECTION=1`+`TEST_INJECTION_MARKER`）なら awase が物理 Ctrl として認識する（`sc-settle-explicit-*` で確認済み）。 手順: [v2-manual-verification-guide-2026-09-29.md](v2-manual-verification-guide-2026-09-29.md)。 **CI 代替観測**: `sc-driftrecovery-{gji,msime}-{tsf,chrome}` が実 IME を直接閉じ、各10回、+0.5/+2秒と実打鍵、drift/観測ログを集計し `NOT_OBSERVED` と `NOT_RECOVERED` を区別する（expect=observe）。物理 Ctrl+無変換は `sc-driftrecovery-ctrlmuhenkan-{gji,msime}-{tsf,chrome}`（マーカー付き注入を awase が物理 Ctrl 扱いにする。phys_ctrl=true を確認できない試行は INVALID、ずれを作れた/作れなかったを区別）で代替観測する。**CI 結果（2026-10-01、PR #418）**: 直接 close の4構成は、tsf=NOT_OBSERVED（観測0）、Chrome=NOT_RECOVERED（観測1・drift0）で、drift correction だけでは ON に戻らない（ADR-205/BUG-172 と整合）。**物理 Ctrl+無変換の4構成**（マーカー付き注入で `phys_ctrl_ok` 10/10）: GJI×tsf・MS-IME×tsf・GJI×実 Chrome は OFF が効き（`GAP_NOT_MADE` 10/10）、**MS-IME×実 Chrome だけ 9/10 で OFF が実 IME を閉じず、drift correction でも戻らない（`NOT_CORRECTED`）**。これは ADR-208 の例外(a)（awase の `VK_IME_OFF` が MS-IME×実 Chrome で効かない、BUG-172 対照）に当たり、E2（内部状態を新鮮にした対照）の構成がまだ無い。実キーボードの HKL・scan・常駐ソフトを含む実機の物理 Ctrl+無変換、VS Code は実機に残る。
- [ ] **C4 MS-IME×実 Chrome の起動直後の最初の文字**(2026-10-01 発見、**v2 のブロッカーにしない**〈所有者決定〉、**切り分け済み・再現せず・環境依存と判断**): IME ON で起動した直後の最初の文字が、ある時間帯(14:52 UTC ごろ)の runner で約半数ローマ字 `ka` になった(develop の `sc-startup-msime-chrome-on` が 6 回中 3 回 FAIL。GJI×実 Chrome は FAIL 0)。Opus の調査: 打鍵時点で実 IME が閉(ハーネスの起動前 `VK_IME_ON` が効かない/2回目の `refocus()` で閉に戻る、環境側と推測)なのに、awase が belief=ON(起動時の既定)のまま Engine を有効にしてローマ字を送る。`[msime-ready]` が閉を確認しても期限切れで強制送信し、観測を belief に反映しない(awase 側の欠陥、BUG-163 系。コードの読みで、実験では閉の回が出ず未検出)。**切り分け実験(2026-10-01 23:22 UTC、`sc-startup-msime-chrome-on-{noawase,precheck,gated,norefocus2}` 各8回=32 回)は全 PASS で、打鍵直前の実 IME(`IMC_GETOPENSTATUS`、Chrome でも読める)は全回 `open`、awase なしの対照も 8/8 かな**。基準構成も同じ時間帯に 6 回中 FAIL 0(PASS 5・INVALID 1)。以上から、失敗は runner の状態・時間帯に依存する環境側の揺れと判断し、再現できない現象への修正は見送る。**方針(所有者決定)**: IME ON↔NICOLA ON の原則で、読めない窓は観測が得られるまで NICOLA OFF(ADR-213 決定6)。実装は、再発時に切り分け構成(上記4構成)で `real_ime_open_before_type=false` の回を捕まえてから、`[msime-ready]` が閉と確認したら強制送信せず belief を正す修正とあわせて決める。v1 へは backport しない。
- [x] **D4 MS-IME 本体の学習（ADR-196 T2）**（v2 に含める、所有者決定）: 半角カタカナ（conv 0x0013）が学習モデルの Conv に無く復号失敗する件と、`--adopt-pending-judgement`（精度≥0.95）の採用経路の検証。**CI検証完了**（run 36569030546、5/5でdecode_errors=0・採用/再採用success・精度0.953〜0.973。[adr196-t2-msime-learning-open-issues.md](adr196-t2-msime-learning-open-issues.md)「再検証」節。コード変更なし）。

## E. リリース作業

- [ ] **E1 v1 の保守終了の告知**: **反映済み・マージ待ち**（ブランチ `docs/v2-e1-v1-eol`）。README.md / README.en.md に「v1 の保守終了と v2 への移行」節（保守終了日は v2.0.0 公開日 2026-10-03、v1 最終版 1.21.1、既知の問題、設定の移行点）を追加し、`docs/migration-v1-to-v2.md` §9 を同期した。v1 の最後のパッチは出さない（v1 のタグを push しない）前提。未了: GitHub Release 本文への告知文、`scoop-awase` 側の説明、`awase.cc`（docs/index*.html）への反映（所有者判断、下書き末尾の「E1 反映後の要判断」参照）。更新通知は worker 変更なし（v1.21.1 は v1 ライン内の最大=1.21.1 が返り通知なし、`0a38590a`）。
- [x] **E2 backport の棚卸し**: 完了（`v2-e2-v1-backport-inventory-2026-09-29.md`）。重大と判定したのは BUG-173 のみで、**所有者決定（2026-09-29）により backport せず v2 への移行を案内する**。BUG-171・172 は develop でも未修正のため backport 不可。E1 の告知に『v1 に残る既知の問題』（同文書の一覧）を載せる。
- [ ] **E3 リリース**: `release-develop-to-main`（CHANGELOG、2.0.0 への bump、タグ、GitHub Release）。`docs/changelog.en.html` も更新する。

## 運用メモ

- ディスクが逼迫している（空き 1〜2GB）。エージェントの並列ビルドは共有 `target`（`CARGO_TARGET_DIR=/home/cuzic/rust-nicola/target`）を使い、他の worktree の `target` は消さない。
- 設計判断を含む変更は、ADR 起票 → `opus-adversarial-consult` で収束 → 実装の順にする。

- E1 の告知文の下書き: [v2-e1-v1-eol-announcement-draft-2026-09-29.md](v2-e1-v1-eol-announcement-draft-2026-09-29.md)
