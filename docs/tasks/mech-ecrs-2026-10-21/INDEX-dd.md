---
title: 二重に守っている処理の調査(失敗の種類ごとの縦切り) — 統合と着手順
status: 調査のみ(2026-10-21、origin/develop 6a6f1c90 付近)。実装は未着手。下の「確認済み」とした分だけ統合時に grep で再確認した。他は着手前に必ず根拠を再検証すること
created: 2026-10-21
related_adr: ["ADR-119", "ADR-180", "ADR-203", "ADR-208", "ADR-212", "ADR-241", "ADR-275"]
---

## 実施状況(2026-10-22 追記)

| ID | 状況 |
|---|---|
| DD2-01=DD4-02、DD3(`should_pass_physical_key`)、DD3-C、DD4-01、DD4-03、DD4-04、DD2-03 | 実施済み(#760 ほか、前日までに develop へ) |
| DD2-04(GjiFsm の `FocusEpoch`) | 実施済み(#770)。`EndComposition` の唯一の作り手が同じ呼び出しの中で状態から読んだ値なので恒真だった |
| DD5-01(A1・G1) | 実施済み(ADR-276 P3、#768)。Unicode 注入の撤去で G2 が全送信で働くようになり、モデル検証(#765)で二重と確認 |
| DD1-01(`imm_cross_write` の gate) | 実施済み(#773)。ADR-180 を訂正。e2e(edit・reopen)PASS |
| DD5-02(フォーカス時の先同期の写し) | **見送り**。「同じ `ir_execute` の中で必ず上書きされる」は成り立たない: 先同期は `on_focus_process_changed`(フォーカス直後)、snapshot は後の `ir_execute` で、その間に打鍵が来ると `applied` が読まれる。削れるのは約 10 行 |
| DD2-05(apply 完了の受理: epoch と watermark) | **見送り**。削れるのは約 10 行で、(i) `InitialFocusScopeEstablished` が fence の epoch だけ進める例外が未解消(起動時に pending があるか未確認)、belief の reducer は最もバグが出やすい領域(CLAUDE.md)。やるなら先に全列挙テストで旧判定と新判定の一致を示してから |
| DD1-02(台帳の「shadow が後から来る」分岐) | 未着手。不具合報告 journal の `PressWriteClaim` の集計が要る(リポジトリにデータ無し) |
| DD2-02(`ConvModeMgr::observe` の fence) | 未着手。ADR-104 決定 7 を正式に見送るかの所有者判断 |
| DD3-01、DD5-03(不具合の疑い) | 未着手。実機または窓ごとの profile を持つ窓モデルが要る |

# 二重に守っている処理の調査

所有者の指摘: 「もっと Eliminate/Combine できるところがあるはず。特に二重に守っている処理」。
前回(INDEX.md)はディレクトリ単位で見たので重なりが少なく出た。今回は**失敗の種類**ごとに縦に切って、Opus 5 本で読み取り調査した。
報告: `dd1-double-send.md`・`dd2-stale.md`・`dd3-gates.md`・`dd4-windows.md`・`dd5-recovery.md`。

## 1. 数

| 担当 | 防御の数 | E | C | 不具合の疑い | K(独立に必要) |
|---|---|---|---|---|---|
| DD1 二重送信の防止 | 23 | 2(確度 高 1・中 1) | 1(低) | — | 多数(ADR-180・208・241 で整理済み) |
| DD2 古い観測の防御 | — | 8 | 6 | 1(hwnd 不一致) | 9 組 |
| DD3 適用可否の門 | — | 3 | 6 | 1(app_profile と app_policy のずれ) | 3 |
| DD4 時間窓 | tuning.rs の `_MS` 41 個(39 個が `pending = true`) | 2 | 4 | 統治の穴 1 | 8 組 |
| DD5 回復層 | 追随の入口 9 本 + 書き込み 2 本 | 2 | 3 | 1(偽の Confirmed) | 9 本はどれも他で拾えない場面あり |

**重なりは「防御の本体」より「その周りの再判定」に多かった**: 常に真になる比較、同値の述語の複数、同じ処理の中での 2 回の授権・判定、別の経路が上書きする先同期。防御の本体(INV-フェンス・台帳・追随の入口)は、片方だけが止める場面が具体的に言えるものが大半だった。

## 2. 着手の三段

### 第 A 段 — 純粋に消せる(擬似 IME 不要、Linux の単体・全列挙で足りる)

| ID | 対象 | 根拠 | 確認 |
|---|---|---|---|
| DD2-01 = DD4-02 | `apply_idle_conv_check` の経過時間の足切り(`key_pipeline.rs:812-820`、`EXPLICIT_IME_SUPPRESS_MS` との比較)。spawn 前のガード 4 と apply 時の値の照合から常に偽 | **2 人が独立に同じ箇所を指摘** | 一部確認(該当行は存在) |
| DD3 | `AppImeProfile::should_pass_physical_key`(ADR-119 も「本番デッドコード」) | 本番の呼び出し元ゼロ | **確認済み**(`class_names.rs` の定義・テスト・oracle のみ) |
| DD4-01 | `DRIFT_CORRECTION_THRESHOLD_MS` と `FeedbackPolicy::Read.deadline` / `Blind.backoff`(本番の読み手が無い 2 フィールド) | `2cd99845` で閾値としては撤去済み | 一部確認(読み手は `app_ime_policy.rs:70,75` の初期化と tests のみ。フィールドの読み手ゼロは未確認) |
| DD2-03 | 同期経路の `verify_gen_only`(同じ `focus_gen` を捕獲と照合の両方に渡す) | データの流れを追った | 一部確認(定義・使用箇所は存在) |
| DD2-04 | GjiFsm の `EndComposition` の epoch(照合相手の状態から同じフレームで読んだ値と比べる) | 同上 | 未確認 |
| DD3-C | 「IMM で読めるか」の 3 述語(`cannot_verify_real_ime_state`・`can_use_imm32_cross_process`・`can_read_imm32_open_status`)は本番で起こりうる全入力で同値 | profile × kind の全列挙で証明できる | 未確認(**全列挙テストで先に証明する**) |
| DD4-03 | 根拠も値(8000ms)も同じ 2 定数の統合 | | 未確認 |
| DD4-04 | TsfGate の FSM が宣言するタイマーは戻り値を捨てていて効かず、殻が別に張っている | | 未確認 |
| DD5-02 | フォーカス時の `presync_applied_open_on` の写しは、同じ `ir_execute` の中で `ir_post_focus_change_snapshot` が必ず上書きする | 呼び出し順を grep 済み(DD5 の報告) | 報告のみ |

### 第 B 段 — 跨ぎ層の再検出。到達の証明が要る

| ID | 対象 | 要るもの |
|---|---|---|
| DD1-01 | `open_chain.rs::imm_cross_write` の InputRelay gate(`run_open_chain_async` の gate と同じポーリングの中で `.await` を挟まず、`with_app` が `None` のときもどちらも素通り=結果が必ず同じ、約 -45 行)。**ADR-119 #4・ADR-180 決定 1 の前提を覆す**(imm_cross_write 冒頭のコメントが、レビュー指摘 B-1 でこの再検出を足した経緯を書く) | 「gate からチェーンの最初の writer までに `.await` が無い」ことを固定する新しいガード(DD1 の案)。`architecture_guard` の件数(1→0、3→2)と `fix-requires-evidence.md` の表を直す。**撤去前に ADR-180 を訂正する ADR 追記** |
| DD5-01 | フォーカス時と IME 種別の同期の GjiFsm への `ImeOn` 2 か所(ADR-203 (i) が同じずれを拾う。ADR-203 決定 4 が撤去可と書く) | 判断材料の e2e(IME ON のまま GJI 種別を検出し直す構成)が無い。**実機 e2e を先に作る** |
| DD1-02 | 台帳の「shadow が後から来る」分岐(`ConflictKept`・`ClaimDeclined`) | 不具合報告 journal の `PressWriteClaim` の verdict の集計(リポジトリ内にデータ無し) |
| DD2-02 | `ConvModeMgr::observe` の fence 比較(呼び出し元 2 か所とも常に一致) | ADR-104 決定 7(非同期化、保留中)を正式に見送るかの**所有者判断** |
| DD2-05 | apply 完了の受理判定で `ImeTransition.focus_epoch` と watermark を 1 本に(C) | 旧判定と新判定の一致を Linux の全列挙で確かめる |

### 第 C 段 — 不具合の疑い(撤去より先に確かめる)

- **DD3-01**: 「この窓は読み戻せない(Blind)か」という**同じ事実を、殻の `app_profile`(窓のクラスが変わるたびと学習の降格で更新)と核の `ImeModel.app_policy`(プロセスが変わったときの `FocusChanged` でのみ更新)が別々に持つ**。drift correction は両方を同じ判断材料に並べる。同一プロセス内でクラスが変わる窓(Explorer 本体と XAML)やフォーカス中に降格された窓でずれうる。**統合すれば不具合の修正にもなる**。実害は未確認(実機要)。
- **DD5-03**: `ir_post_focus_change_snapshot` が Chrome など静的一覧で読めない窓へ偽の `Confirmed` を書き写す。押下に由来しない ON(EngineOn コンボ等)が、書かれずに省かれうる(実機要)。
- **DD2 の範囲外所見**: `most_recent_trusted` と drift の根拠の選び方が窓(hwnd)の一致を見ていない疑い。二重ではなく**守りの欠け**(未確認)。

## 3. 擬似 IME でどこまで検証できるか

- 第 A 段の「常に成り立つ比較」: **全列挙(決定表)**で足りる。DD3-C の 3 述語は profile(4)× kind(2) の 8 通りで証明できる。
- DD2-05・drift の二重の関門(`is_japanese_ime`): 全列挙で足りる。DD3 は `is_japanese_ime` の二重の門を K(残す)とした(片方を消すと日本語 IME でない窓で drift の検知と試行回数の加算まで進む)。**前回の ME 報告が「アサートが区別しない」と書いた箇所に答えが出た**。
- DD1-01・DD5-01・DD3-01・DD5-03: **擬似 IME だけでは足りない**。DD1-01 は await の有無というコード構造の性質なのでガードで固定する。DD5-01 は実機 e2e、DD3-01 と DD5-03 は窓ごとの profile を持つ窓モデル(ME-5)と実機が要る。
- アブレーション・ファズ(ガードを外して違反が出るか)は ME-2 の後。

## 4. 推奨する順序

1. **第 A 段のうち「常に成り立つ比較」と「本番ゼロ」**を領域ごとの小 PR で消す(DD2-01/DD4-02、DD3 の `should_pass_physical_key`、DD4-01、DD2-03/04)。
2. **DD3-C の全列挙テスト**を先に入れ、3 述語を統合する。
3. **DD3-01 の実害の確認**(実機または窓モデル)。統合の動機が不具合修正になる。
4. 第 B 段は ADR 追記と e2e が先。
5. ME-2(ハーネスの crate 内移動)の後に、ファズでの検証を第 B 段へ広げる。

## 5. 限界

- 5 本とも cargo 未実行・読み取りのみ。確認済みとしたのは統合時の grep のみ。
- 「K(残す)」の判定は、片方だけが止める場面を具体的に言えたものだけ。言えなければ E/C に回してある。
