# BUG-173（物理F2 Suppress・カタカナ固着）と発火削減: 残作業

状態: **起票（2026-09-29）**。PR [#359](https://github.com/cuzic/awase/pull/359)（物理F2を常にAllow・KeyUpラッチ・D1〜D3）は
develop マージ済み（CI 全通過、windows-build 含む）。以下は未完了の作業。着手時は
`.claude/rules/worktree-per-session.md` に従い専用 worktree/branch を切ること。
出所: 不具合報告 `01M3NJ784NKMH120HM6QGKF7W7`（v1.21.0）、[BUG-173](../known-bugs/BUG-173.md)、[BUG-174](../known-bugs/BUG-174.md)、
Opus 戦略相談/レビュー（round1・round2・発火削減）の指摘。

## 1. v1(保守)ラインへの backport【未着手】

報告は **v1.21.0** 由来。`main-develop-branch-flow.md` のとおり修正は develop が先で、`v1-develop` へ backport する
（本文に `Backport of <developのコミットハッシュ>` を明記）。

- 対象: #359 の develop マージコミット（`git log origin/develop --oneline --grep="#359"` で特定）。BUG-174（Ctrl↑ warmup 撤去、`889aff2f`）は
  #359 に内容が含まれるが別 PR #358 で develop に入っているため、v1 への要否を別途判断する。
- 注意: v1 は BUG-170 の修正（`52cd221f`）が無い可能性が高い（GjiFsm が OffCold のまま動かない前提が変わる）。backport 前に、
  v1 側で `on_reinject_key`/`composition_native_f2_down`/`kp_restore_hiragana_for_suppressed_mode_key` がどうなっているか、
  ADR-100 決定2 が v1 に入っているかを確認する。v1 に無い機構の撤去は backport しない。
- 完了後、[台帳](../bug-reports-triage.md)の 01M3NJ 行に v1 backport の結果を追記する。

## 2. PR #360（発火削減 D2-full・D4）【ドラフト・保留】

確定キー reinject 時の `VK_IME_ON` warmup 全面削除、`CompositionFsm` 解体、`ConvOpenInference` の drift 補正撤去
（ADR-191「warmup は例外として残す」の改訂を含む）。base は develop に付け替え済み。

**マージ条件（Opus round2 R2-4）:**
- `origin/ci/e2e-warmup-ab`（eager `VK_IME_ON` を実験フラグで無効化する A/B CI）の結果が出ること。
- 実機で「GJI + Windows Terminal/WezTerm、フォーカス直後や IME ON 直後の cold 状態で『な』→Enter→『な』」を往復し、
  `[gji-coro] transmit-plan ... gji_settled= confirm_key_tsf_hint= needs_literal=` を D2-full の前後で比較する。
  `gji_settled` が変わると BUG-40/`3ffbe66` 型の誤検出や BUG-171（StaleConfirm の ESC で未確定文字が消える）を踏む確率が上がる懸念。
- `gave_up=true`/`SuspectedLiteral` の件数、「Enter 直後の1文字目のリテラル化」「@」が増えないこと。

マージ前に develop を取り込み直し、Windows 専用テスト（`platform_state.rs` の drift テスト等）を windows-build CI で確認する。
効果が無い/悪化した場合は revert しやすい単位（この PR 単独）で戻す。`docs/experiments.md` に判定を追記する。

## 3. 実機検証【未実施】

[BUG-173.md](../known-bugs/BUG-173.md) の検証項目:

1. belief OFF＋実 IME カタカナの状態で物理F2 → ひらがなに戻り、Engine も追従して親指シフトが効くか（`[key-effect-miss]` が出ないか）。
   Opus R2-5: 予測（`KeyEffectPredicted`）が後続の観測照合で上書きされて belief が OFF に戻る経路がないかを journal で見る。
2. 未確定「か」の状態でF2 → 文字欠落・余計な BS が出ないか（A3: `gji_on_native_f2_consumed` の cold 扱いと GJI の実状態の食い違い）。
3. NICOLA pending 中に F2 → 到達順（A4: hook 時点の cold 化と、遅延した F2 本体の到達順のずれ）。
4. cold 状態の F2/Enter 直後の1文字目がリテラル化しないか。
5. `VK_IME_ON/OFF` 等の物理キーで Down/Up とも届くか（KeyUp ラッチ）。半角/全角の Down/Up の vk 違い（scan で照合）。
6. 参考: 報告の再現条件（Windows Terminal + GJI、Ctrl+無変換の連打）での `VK_IME_ON/OFF` の自己注入件数を journal で再集計する
   （削減前: 約200秒で `VK_IME_ON` 46発・`VK_IME_OFF` 5発）。

## 4. 未解明・未対応の指摘

- **最初にカタカナへ入った契機**（未特定）。journal は 850/2560 件に切り詰められ直前の物理キー入力が残らない。再現条件を絞って
  `dropped_key_input` の少ない journal を取る（BUG-050 の原因2と同じ未解決点）。
- **R2-11**: KeyUp ラッチが強制した Suppress にも `suppress_reason` が "imm32-off"/"imm-cross" を付け、journal の原因が誤読される。
  ラッチが書き換えたかを返して "keyup-latch" ラベルを付ける（LOW）。
- **ADR-166 の決定表**: `plan()` の後段にあるラッチ上書きは注記のみで、決定表の軸には入っていない。「Down の配送」を入力軸に取り込む案（LOW）。
- **BUG-171**（StaleConfirm の ESC で既存の未確定文字が消える）と **BUG-172**（MS-IME の msime-ready ゲートが conv NATIVE を ON と扱う）は
  別件だが、warmup 削減の効果測定（2.）と干渉しうるため状態を確認する。

## 5. 台帳・記録の後始末

- リリースに含まれたら [台帳](../bug-reports-triage.md) の 01M3NJ 行を「対応済み(vX.Y.Z〜)」へ更新する
  （現在は「対応済み(未リリース)」）。`git merge-base --is-ancestor <fix> <最新リリースタグ>` で到達を確認してから。
- `origin/ci/e2e-warmup-ab` に BUG-173.md/BUG-174.md のコピー（BUG-173 は古い版）がある。このブランチを develop に入れる場合は
  known-bugs の衝突に注意し、実験用ブランチから known-bugs を持ち込まない。
- #360 マージ後、[BUG-173.md](../known-bugs/BUG-173.md) の「発火削減の追補」から「マージ条件」の記述を「マージ済み」に直し、
  `docs/experiments.md` の該当2行（確定キー reinject warmup 削除・`ConvOpenInference` drift 撤去）の判定を「未判定」から更新する。
