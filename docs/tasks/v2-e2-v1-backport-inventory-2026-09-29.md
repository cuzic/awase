---
title: v2 リリース E2 — v1 への backport 棚卸し（重大度判定）
status: 棚卸し完了（2026-09-29）。backport 実施は未着手。所有者の最終判断待ち
created: 2026-09-29
related_adr: ["ADR-200", "ADR-203"]
related_bugs: ["BUG-168", "BUG-170", "BUG-171", "BUG-172", "BUG-173", "BUG-174"]
---

# v2 リリース E2: v1 への backport 棚卸し

[v2 リリースチェックリスト](v2-release-checklist-2026-09-29.md) E2 の成果物。所有者決定は「v1 は v2 リリースで保守終了。backport は**重大バグ（入力不能・誤入力頻発・クラッシュ）のみ**」。
本書はコード修正も backport もしていない。読み取りだけの棚卸し。

## 調べた範囲と方法

- 基準: `origin/v1-develop`（`61ff4f1e`、最終 v1.21.1）と `origin/develop`（`2c4d48b2`）。共通祖先は `6f2257b7`（2026-09-20）。
- `docs/known-bugs/BUG-100〜174` の `fix_commits` が `v1-develop` の祖先かを機械的に判定し、修正が v1 に無いものについて、**現在の HEAD の v1-develop のコード**を `git grep` / `git show` で照合して「v1 に同じ不具合があるか」を裏取りした（BUG 記述の鵜呑みはしていない）。
- 限界: ビルド・テスト・実機は使っていない。「v1 に存在」は**コード上同じ経路がある**の意味で、v1 での再現までは確認していない。BUG-100〜139 の大半は v1 の docs にも記録があり範囲外（BUG-110 のみ触れる）。
- v1 は develop より新機構が少ない（`KeyEffectPredicted`・`ModeKeyPassLatch`・`ModeKeyConfig::Passthrough` は v1 に 0 件、ADR は 170 番台まで）。ADR-186〜204 系の BUG は「v1 に原因コードが無い」ものが多い。

## 結論（要約）

| 区分 | BUG | 推奨 |
|---|---|---|
| **重大・backport しない（所有者決定 2026-09-29）** | BUG-173 | v1 へは backport せず、v2 への移行を案内する。既知の問題として告知に載せる |
| 中〜重大寄りだが backport 不可 | BUG-171 | develop でも未修正。v1 の既知の問題に載せる |
| 中・切り出しやすい | BUG-168 | 任意。所有者が「文字消失＝重大」とみなすなら backport |
| v1 に主因が無い | BUG-170 | 不要 |
| v1 に存在・軽微〜中 | BUG-172, 174, 152, 163, 164, 142, 110 ほか | しない。v2 への移行を案内 |

## 必須 6 件の判定

重大度: **重大**=入力不能・誤入力頻発・クラッシュ、**中**=条件付きの文字消失・状態ずれ、**軽**=軽微・限定的。

| BUG | 症状 / 影響 | v1 に存在するか（HEAD で確認） | 修正の大きさ・依存 | 重大度 | 推奨 |
|---|---|---|---|---|---|
| **168** | Chrome+GJI で StaleConfirm が 2 連続すると give-up 経路が VK_IME_OFF→ON の reinit を送り、入力中の未確定文字が全消失（CI の 2〜10ms 間隔の合成打鍵で 2/4,200。実機 BUG-036 と同じ連鎖） | **存在する**。v1 `output/probe_io.rs` の give-up 分岐は `consecutive` を SuspectedLiteral と同じに数えて `schedule_chrome_gji_reinit` を呼ぶ。修正の `negative_evidence` は v1 に 0 件 | `bc12ce95`、3 ファイル +320/-8（大半はテスト）。他修正への依存は見当たらない。cherry-pick しやすい（衝突は要確認） | **中**（文字消失だが通常速度では起きにくい。実機頻度は未測定） | **任意**。消失を重大とみなすなら backport。しないなら既知の問題に載せる |
| **170** | GjiFsm が OffCold に固着し、毎打鍵 per-VK→StaleConfirm→ESC で「これでいい」→「でいい」（GJI+Edge/Meet。develop 系ビルドの報告） | **主因は無い**。起点 `c8bc1adc`（ADR-090 A-2 warrant 強制、2026-09-18）は v1 の祖先でなく、v1 の `Authorization::LegacyUnwarranted` は警告のみで書込みを止めない。別経路 `sync_ime_kind_from_observation` の GjiFsm 作り直し固着は v1 でも起きうる（未確認） | `52cd221f` は 14 ファイル +502/-49。ADR-203 の新機構（`needs_belief_sync_on` は v1 に 0 件）が前提で切り出せない | 軽（v1 では主因なし） | **しない** |
| **171** | 途中の語で StaleConfirm→`VK_ESCAPE` が既存の未確定文字まで消す（journal 上 per-VK 約 26 セッション中 4 回） | **存在する**。`per_vk_recovery_params`（`tsf/warmup/literal_detect_fsm.rs`）は v1 にも同一で `failed_idx>0` なら `escape_composition=true` | **develop でも未修正**（`fix_commits: []`）。backport 対象なし | **中〜重大寄り**（実報告あり。v1 は BUG-170 の固着が無いぶん頻度は低い見込み。未測定） | **しない**（直せない）。既知の問題に載せ、v2 へ案内 |
| **172** | MS-IME+TsfNative で IME が閉じていても msime-ready ゲートが conv の NATIVE を ON 確認と扱いローマ字が入る（CI の RichEdit 入力先で 30/30）。実 Chrome の症状はこのゲートを経由せず、Chrome の開閉を awase が観測できない別問題 | **存在する**。v1 に同じゲート（`probe_io.rs` の `[msime-ready] … NATIVE 確認 → 終了`）。実 Chrome 側も同じ構造のはず（未確認） | **develop でも未修正**（ゲート修正は見送り、観測手段を設計中。v2 のブロッカー C1） | 中（外部から IME を閉じられたときのみ。物理キーでは起きない） | **しない**（直せない）。既知の問題に載せる |
| **173** | GJI+TSF ネイティブ（Windows Terminal 等）で物理ひらがなキー（0xF2）が常に Suppress され、カタカナ固着から戻れない（報告 `01M3NJ784NKMH120HM6QGKF7W7` は **v1.21.0**、約 25 回 Suppress） | **存在する**。v1 `runtime/transport.rs:293` は `if is_tsf_mode && f2_warmup_owned` で F2 を Suppress（develop の修正前と同一）。`kp_restore_hiragana_for_suppressed_mode_key`（v1 に 8 件）・`ConsumeF2`（9 件）も残る。報告者は v1 ユーザー | 核は `d4d7c8b1`（5 ファイル +71/-54、`plan()` で F2 を常に Allow、`handle_reinject` の握り潰し撤去）。Allow 化で Down=Allow/Up=Suppress の非対称が出るため KeyUp ラッチ `bb6e9440`（+55）・`feb00e78`（+271/-45）も要る。`35f95eba`・`8748d481`（-621）・`1b00a9a2` は整理で必須ではない。PR #359 の merge は `a29b4cc2` | **重大**（該当状態の間かな入力がカタカナで出続ける＝誤入力が継続。ユーザー報告あり）。最初にカタカナへ入る契機は未特定 | **backport しない（所有者決定 2026-09-29: v2 への移行を案内）**。参考（backport する場合の最小セット）: `d4d7c8b1`＋`bb6e9440`＋`feb00e78`。前提: (1) v1 に ADR-100 決定 2（warmup=VK_IME_ON 単発）が入っているか確認（`WarmupImeOn` は v1 に 54 件あり入っているとみられる）、(2) v1 には BUG-170 修正が無く `composition_warm` の扱いが違う、(3) develop 側の実機検証が未実施（`bug173-remaining-work-2026-09-29.md` §3）。実機確認が取れるまで v1 リリースを出さない。v1 に無い機構の撤去は backport しない |
| **174** | Ctrl↑のたびに awase 自身が `VK_IME_ON` を注入（CtrlUp warmup）。GJI の「@」報告の被疑箇所（原因かは実機未確認） | **存在する**。v1 `platform.rs::composition_ctrl_up` と executor `handle_ctrl_up_recovery` が同じ経路 | `889aff2f`（#358）8 ファイル +39/-72。#359 に取り込み済みで実質 BUG-173 の一部 | 軽〜中（因果未確認） | **しない**。BUG-173 backport 時に v1 側の依存が小さければ同梱を検討 |

## その他（BUG-141〜169 のうち v1 に関係するもの）

| BUG | 要点 | v1 の状態（HEAD で確認） | 重大度 | 推奨 |
|---|---|---|---|---|
| 141 | Ctrl+無変換の 2・3 回目を無送信で握り潰す | v1 に `candidate_was_seen` の配線が既にある（`ime_controller.rs:283-349`）。`040536bf` は v1 の祖先でないが同等コードあり | — | 対応済み（同等） |
| 143 | `classify_mode_key_ime_action` が CUSTOM 以外で `custom_keymap_table` を見ない | v1 `gji_charset_autodetect.rs:152-161` に「CUSTOM 以外でも該当行があれば」の扱いが既にある（等価とみられる。未確認） | — | 対応済みとみられる |
| 148 | 起動時に既にフォーカスがある窓で意図が記録されない | v1 に backport 済み（`a28b1ab0`） | — | 済 |
| 165 | 高速打鍵で `pending_deferred` 上限超過の文字消失 | v1 に backport 済み（`bb7ba808`、v1.21.1） | — | 済 |
| 167 | 設定 GUI の `Ctrl+Shift+VK_F12` を解釈できない | v1 に backport 済み（`e0c1bfd8`） | — | 済 |
| 152 | MS-IME 本体で ImmCross set-open がタイムアウトすると非冪等な `VK_KANJI` トグルが開いた IME を閉じる（CI で 3/3、最初の 1 回のみ） | **存在する**。v1 に `KanjiToggle` が 76 件（develop は 3 件） | 中 | しない（修正 `feb49ffd` は 34 ファイルの機構撤去で大きい）。既知の問題に載せる |
| 163 | 起動直後の `desired_open=true` 初期値で、IME を閉じて起動すると drift correction が `set_ime_open(true)` を繰り返す（約 11 秒で 22 件） | **存在する**（v1 `state/ime_model.rs:279`） | 軽（起動直後の過渡） | しない（修正 3 コミット約 +370 行で ADR-191 系の前提）。既知の問題に載せる |
| 164 | 古い High 観測が新しい Medium 観測を隠す | **存在する**（`most_recent_trusted` 等が v1 と develop で同数）。develop でも未修正 | 軽（コード上の指摘のみ） | しない（直せない） |
| 169 | 設定 GUI で n-gram ファイル欄を空にしても既定に戻る | v1 GUI にも `ngram_file` あり。develop でも未修正 | 軽 | しない |
| 142 | Windows Terminal+GJI で物理半角/全角の繰り返し押下により IME ON 固着 | v1 に `ShadowToggle` 機構あり（同種）。develop でも恒久修正は config 既定値の変更待ち | 中（連打時のみ） | しない。既知の問題に載せる |
| 110 | 物理 IME キー 1 回の低確度検出で NICOLA がフォーカス変更まで停止 | v1 にも記録あり。根本修正は develop でも未着手 | 中（頻度不明） | しない（直せない）。既知の問題に載せる |

### v1 に関係しない（原因コードが develop のみ）

以下は ADR-186/187/189/191 系の受動化・追随機構（v1 に `KeyEffectPredicted`・`ModeKeyPassLatch`・`ModeKeyConfig::Passthrough` が 0 件）が前提のため、v1 に原因が無い。代わりに v1 は**そもそも追随機構が無い**制約を持つ（下記 #4）。

| BUG | 理由 |
|---|---|
| 144 | 較正 probe ループ（ADR-176、v1.21.0 から除外） |
| 145 | ADR-179 の Passthrough 実験設定が前提。v1 既定 `muhenkan_solo_tap_always_suppress=true` では生の親指 VK は出ない（`false` にしたユーザーのみ） |
| 146, 149, 150, 151, 159 | ADR-186/187/189 の Engine 追随機構が前提 |
| 153〜158, 160〜162 | ADR-191 の撤去ブランチ・通過マーク・Passthrough 系 |
| 166 | CI ハーネスの不具合 |

## E1（v1 保守終了の告知）に載せる「v1 に残る既知の問題」

v1.21.1 で**修正されない**問題。BUG-173 は backport しない（所有者決定 2026-09-29）ので #1 に載せる。

| # | 問題 | 条件・影響 | 回避策（案） |
|---|---|---|---|
| 1 | カタカナ固着（BUG-173） | GJI + Windows Terminal 等の TSF ネイティブ環境で、カタカナになると物理ひらがなキーで戻れない | 半角/全角キーで切り替え直す。v2 への移行を案内 |
| 2 | 入力中の未確定文字が消える（BUG-171 / BUG-168） | Chrome・Edge 系で GJI の cold 状態から打つとき、途中の語や超高速打鍵で ESC/IME 再初期化が未確定文字を消すことがある | v2 へ移行（BUG-168 は v2 で修正済み。BUG-171 は v2 でも未修正） |
| 3 | 外部から閉じられた IME が ON に戻らない（BUG-172） | 他プロセスが IME を閉じたあと、実 Chrome で NICOLA のローマ字がそのまま入る | 物理キーで切り替える |
| 4 | IME の開閉・モードに Engine が追随しない場合がある | 無変換/変換・ひらがな/英数キーで IME の状態を変えても Engine が ON のまま（v1 に ADR-186〜191 の追随機構が無い）。BUG-146/149/150/151/159 相当 | v2 へ移行 |
| 5 | MS-IME 本体で最初のひらがなキーが効かないことがある（BUG-152） | ImmCross がタイムアウトした最初の 1 回だけ、非冪等な `VK_KANJI` で IME が閉じる | もう一度ひらがなキーを押す |
| 6 | 起動直後の強制 IME ON（BUG-163） | IME を閉じて起動すると、しばらく IME を ON に戻される | 起動後に切り替える |
| 7 | 物理半角/全角の連打で IME ON 固着（BUG-142） | Windows Terminal + GJI | `keys.ime_detect.toggle` を設定 |
| 8 | 低確度の IME キー検出で NICOLA が停止（BUG-110） | フォーカス変更まで NICOLA 変換が効かない | 別ウィンドウへ移して戻す |
| 9 | n-gram ファイルを空にして無効化できない（BUG-169） | 設定 GUI | 設定ファイルで対処するか v2 |
| 10 | Scoop・アプリ内更新通知が v1/v2 を区別しない（`main-develop-branch-flow.md` の未解決事項） | E1 の告知文で明記 | — |

## 次の一手（所有者判断）

1. **BUG-173 の最小 backport（決定済み: 行わない、2026-09-29）**。以下は参考: 行うなら `v1-develop` へ専用ブランチで `d4d7c8b1`＋`bb6e9440`＋`feb00e78` を cherry-pick（本文に `Backport of <hash>`）。GJI+Windows Terminal で belief OFF＋カタカナから物理 F2 の実機確認が取れてから v1.21.2。行わないなら告知の #1 に載せる。
2. **BUG-168 を重大とみなすか**: 実機頻度は未測定。みなす場合は `bc12ce95` を単独で backport（3 ファイル）。
3. 本書は「v1 のコードにも同じ経路がある」までの確認。backport 前に v1 上で衝突解消と `cargo check --target x86_64-pc-windows-msvc` を通すこと（本棚卸しではビルドしていない）。
