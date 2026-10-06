---
type: companion-doc
title: |-
  ログ(awase.log の tracing 行)と journal の重複の整理(2026-10-06、検討のみ・コード変更なし)
---

# ログと journal の重複の整理

[README.md](README.md) の B4 段階 1 より先に、ログと journal の重複を減らす案を検討する。初版は develop `777bf1db`、Opus レビュー(`opus-review-log-journal-dup`)の指摘の裏取りは `1732f830`(#522 マージ後)で行った。行番号は断りの無い限り `1732f830` のもの。
パスは `crates/awase-windows/src/` からの相対パス。VK 列は残す前提(所有者の訂正、README 冒頭)。

## 1. 事実

### 1.1 「journal → ログ」の派生の行と、その限界

journal の全 21 variant は、記録されると `UnifiedJournal::absorb` から `emit_tracing`(`journal.rs:806-1186`、約 380 行)で `debug!` の 1 行(ターゲット `awase::journal`、文言は `"key input"`・`"ime open applied"` など variant ごとに 1 種)として awase.log にも出る(ADR-139 決定4 Option C)。
ただし `emit_tracing` は意図的に「トップレベルの主要フィールド、または粗い判別子のみ」を出す(`journal.rs:717-720`)。**journal に載る事象はログに必ず 1 行あるが、journal のフィールドが全部ログに出るわけではない**。主なもの(実コードで照合):

| variant | 派生の行に出るもの | 出ないもの |
|---|---|---|
| `ImeEvent` | `event_kind`(variant 名)・`event_seq`・`tick_ms` | 中身の全フィールド(`DriftDetected{desired, observed, duration_ms}`、`FocusChanged` の中身など) |
| `SentInput` | `issue_us`・`accepted`・`event_count` | vk・`ch`。actuation の `kind` は `SentInput` の型自体に無い |
| `KeyInput` | vk_code・is_down・injected・key_class・state_before/after・decision・physical・repeat | scan・`timestamp_us`・ctrl/alt/shift |
| `HookImeModeDiagnostic` | vk・is_down・self_injected・injected・scan | `since_prev_ime_mode_ms` |
| `ImeActuation` | target_open・attempts・policy・action | 出所の `source`・`confidence`(型にも無い) |
| `ActuationDecision` | site・caller・open・gate の入力・最初の attempt の mechanism/command/outcome | 2 つ目以降の attempt |

CI は JSON journal をダンプしない(inventory §5)ので、**CI から見た journal は派生の行そのもの**になる。
利用者の awase.log の既定は `info`(`app/bootstrap.rs:152`)なので、派生の行(debug)は利用者のログには出ない。手書きの行のうち `info!`/`warn!` のものは出る。

中継を経る journal の記録は、派生の行が出る時刻が事象の時刻ではなく journal に移った時刻になる: `pending_journal_entries`(`platform.rs:126`)、`SENT_INPUT_TRACE`(`win32.rs:259-285`)、フックの診断キュー(`hook.rs:1393`、inventory §1 の 4 つ目の中継)。

### 1.2 同じ事象を手書きのログ行と journal の両方に出している組

記録点の近くを抜き取りで見たもの。これより多い可能性がある(下限。例えば `focus_tracking.rs:601,607` と `DriftGiveUpIntervalEnded`、`executor.rs:820` と `ActuationDecision` の近くにも手書きの行がある)。「フィールドの差」は journal の型との差で、派生の行との差は 1.1 の表を参照。

| # | 事象 | 手書きのログ行(場所・レベル・文言の先頭) | journal | 記録点 | フィールドの差(型) | ログ行を機械的に読むもの | 診断での使用(known-bugs・triage / ADR、粗い grep) |
|---|---|---|---|---|---|---|---|
| 1 | 押下 ID の書き込み予約 | `state/platform_state.rs:228`(info、衝突時)・`:234`(debug)・`:255`(debug、解除) `[press-ledger] …` | `PressWriteClaim` | 同じ関数 | 同じ(派生の行も press・open・source・verdict を出す) | なし(リポジトリ全域の grep で 0) | 0 / 0 |
| 2 | give-up 後の外部クローズの読み直し | `runtime/ime_refresh.rs:301`(info) `[giveup-follow] cold=… outcome=…`(同じ接頭辞の `platform.rs:271,279` は別の事象) | `GiveUpFollow` | 同じ関数 | ログが多い: `gen_at_probe`・`gen_now`・`explicit_intent` | develop 上は 0。ただし `BUG-074.md:319` は、使い捨てのブランチ(`ci/adr225-d0`・`ci/adr227-verify`、未マージ)の CI で awase.log の `[giveup-follow]` を見たと書く(どの行かは未確認) | 1 / 0 |
| 3 | Blacklist 経路の drift correction の結果 | `ime_refresh.rs:1020` 付近(info) `Blacklist drift correction: apply_ime_open(…) → …` | `ActuationDecision` + `ImeOpenApplied` | 同じ関数 | journal が多い | **あり**: `tools/e2e/ime_key_matrix/check_drift_correction.py:28`(向きと結果を取り出す)、`check_drift_recovery.py:65`、`check_drift_recovery_chrome.py:5`。CI は `e2e-ime.yml:1128-1137` で呼び、集計の `drift_log_fired` を出す。人手の手順 `docs/tasks/v2-manual-verification-guide-2026-09-29.md:375` | 3 / 2 |
| 4 | drift correction の送信 | `ime_refresh.rs:958` 付近(**warn**) `[drift] correction: observed=… ≠ desired=… …` | `ImeActuation` + `ImeEvent::DriftDetected` | 同じ関数 | ログにだけ `source`・`confidence`。journal にだけ方針・試行回数・世代 | `check_startup.py`・`check_invariants.py`・`check_drift_recovery*.py`。**`tests/architecture_guard.rs:2431` の `DRIFT_SEND_LOG_MARKER` がこの行の位置を BUG-43/163 系の順序ガードの目印にしている** | 13 / 9 |
| 5 | 物理キー 1 件のエンジン処理 | `runtime/key_pipeline.rs:153`(debug) `[engine-input] vk=… ts=… delay=… state=… mods(c=… …) phys_ctrl=… [diag-ctx] ime_on=… …`、`:180`(CTRL MISMATCH) | `KeyInput` | 同じ関数(`kp_run_inner`) | ログにだけ `delay`・`phys_ctrl`・InputContext(diag-ctx)、journal にだけ `state_before/after`・`decision`・`physical`・畳み込み | `check_startup.py`、`check_drift_recovery.py`・`_chrome.py`・`check_keymatrix.py`(`phys_ctrl=`) | 12 / 3 |
| 6 | awase が SendInput で送ったキー | `win32.rs:338`(debug、actuation のときだけ) `[ime-io] actuation SendInput kind=… vk=…`、`:342` → `[shadow-send] channel=SendInput …` | `SentInput` | 同じ関数(`send_input_safe`) | journal は全送信・`accepted`・`ch`。`[ime-io]` は actuation の `kind` を持つ | `check_startup.py`(`[ime-io] actuation SendInput kind=`)。`[shadow-send]` はコードの読み手 0、計画上の読み手あり(README 段階 1) | 1 / 5(`[ime-io] actuation`) |
| 7 | フックが見た IME モードキー | `hook.rs:1487`(debug) `[hook] IME-mode vk=… extra=… since_actuation_us=…` | `HookImeModeDiagnostic` | 同じ関数で両方を作り、journal は後で吸い出す | ログにだけ `extra`・`since_actuation_us`、journal にだけ `since_prev_ime_mode_ms` | `check_invariants.py`(`[hook] IME-mode vk=`) | 11 / 6 |
| 8 | フォーカスのプロセス変化 | `runtime/focus_tracking.rs:607`(info) `FocusChange [pid→pid] class: …` | `FocusTransition` + `ImeEvent::FocusChanged` | 別の関数 | ログは belief の状態、journal はアプリ名・滞在時間 | なし。`e2e-ime.yml:1162` の grep(`focus-settle\|\[focus\|FocusChanged\|focus_changed`)は `FocusChange [` には当たらず、派生の `ime event` 行の `event_kind=FocusChanged` と、手書きの `FocusChanged: …`(`ime_refresh.rs:387,398`)に当たる | 3 / 1 |
| 9 | ConvClassify の結果 | `key_pipeline.rs:803`(debug)・`:815`(info) `[idle-conv-check] TsfNative: conv=… → belief …` | `ConvClassifyCall` | 同じ関数 | 一部だけ重なる | なし(未確認) | 未計数 |
| 10 | awase が送った romaji(入力内容) | `output/vk_send.rs:228`・`:406`(**info**) `[key-output] KeyInput(batched\|tsf): romaji=…` | `SentInput` | 別の関数 | ログは romaji と IME 種別、journal は送った vk/scan・`accepted` | なし(`.github`・`tools` に参照 0) | 未計数 |

初版の #10(`ImeEventLog` の trace 行)は #522 で撤去済みなので外した。層が違い 1 対 1 か未確認のもの(GjiFsm の遷移、TSF probe、literal 判定、deferred の flush)は初版のとおりで、F6 の後に見直す(README 段階 3 と同じ前提)。

#5 と #10 は入力内容(VK 列・romaji)の重複で、性質が他と違う:
- #10 は info なので、利用者の awase.log に打鍵ごとに残り、報告の `app_log_excerpt_gz`(awase.log の末尾を圧縮前で最大 16MiB、`bug_report.rs:25`)に載る。journal の `KeyInput`・`SentInput`・`LiteralDetect` には報告時に直近 10 分の窓が掛かるが、awase.log には掛からない。入力内容は今、範囲の違う 2 か所にある。
- #5 の `[engine-input]` は、`KeyInput` に無いエンジンの InputContext を持つ。これを `KeyInput` に移せば、重複が減るのと同時に、入力の再生(README B5)の材料がそろう。

### 1.3 片方にしか無い事象

- journal だけ(ログは派生の 1 行だけ): `TimerFired`・`ClockAnchor`・`DumpTriggered`・`DriftGiveUpDiagnostic`・`DriftGiveUpIntervalEnded`・大半の `ImeEvent`。
- ログだけ: 手書きの行の大部分。CI が読むものでは `[warrant-shadow]`・`would_have_blocked=`、`explicit_intent=`、`[startup-align]`、`[msime-ready]`、`[vk-send]`、`send_keys: mode=`、`stale confirm 検出`、`[raw-tsf-literal] flush escape=`、`Engine (de)?activated`、`[hook-watchdog]` など。

### 1.4 件数

- 手書きの tracing の呼び出し: awase-windows の `src` に 740 か所(粗い grep、テストのモジュールを含む)。core の `src` に 32 か所。
- journal の variant: 21(派生の行も 21 種。`emit_tracing` の doc の「19 variant」は古い)。
- 手書きの行と journal の組: 抜き取りで 10 組(同じ関数 8・別の関数 2)。下限。層が違い未確認のものが 4 系統。

## 2. 消費者

awase.log の行を機械的に読むもの(全域 grep で数え直した):

| 消費者 | 件数・中身 |
|---|---|
| `tools/e2e/**/*.py`(テストを除く 43 本) | awase.log か `awase::` か `[タグ]` の正規表現を読むのは 18 本(`check_*.py` 20 本のうち 8 本〈startup・invariants・reopen・run_validity・drift_correction・drift_recovery・drift_recovery_chrome・keymatrix〉、`e2e_common.py`・`mode_key_pass_timeline.py`・`suspend_report.py`・`effect_learning.py`・`run.py` など)。粗い grep なので、`in line` で読むものは漏れている可能性がある |
| `test_log_anchors_in_rust_source.py` の 27 断片 | **全数ではない**。`Blacklist drift correction`・`[engine-input] vk=… KeyDown`・`mods(c=true .*phys_ctrl=true` などは表に無い |
| `.github/workflows` の grep | `e2e-ime.yml:1109`・`:1162-1164`、`e2e-uwp-inputsite-hook-watchdog-probe.yml:75,102` |
| `tools/e2e/ime_key_matrix/testdata/*.awase.log`(22 本) | チェッカーの単体テストの入力 |
| `tests/architecture_guard.rs` | `[drift] correction: observed=` をガードの目印に使う(`:2431`) |
| 人手の手順書 | `docs/tasks/v2-manual-verification-guide-2026-09-29.md:375` ほか(全数は未確認) |
| 不具合の診断 | known-bugs・ADR で使われた行の件数は 1.2 の最右列 |
| report-worker | journal・awase.log とも中身を見ない |

journal を読むものは、不具合報告の診断(人・Claude)と replay(`tests/journals/`)。CI は読まない。

どちらを正にするかで移行が要る消費者:
- journal を正(手書きの行を消す)にすると、手書きの行を読むチェッカー・architecture_guard・testdata・anchor 表・手順書を派生の行へ移す。**派生の行に足りないフィールド(1.1)を先に足す必要がある**。利用者の既定ログ(info)からは #1(衝突時)・#2・#3・#4・#8・#10 が消え、報告で遡れる範囲が変わる(下)。
- ログを正(journal の variant を消す)にすると、報告の journal からその事象が消える。debug の行(#5・#6・#7)は利用者のログに出ないので、報告のどこにも残らない。入力内容については、awase.log に info の `[key-output]` の romaji が残るので入力文そのものは失われないが、再生に使える構造化された vk/scan とタイミング(`KeyInput`・`SentInput`)が失われ、VK 列を残す前提に反する。

報告で遡れる範囲: awase.log は末尾を圧縮前で最大 16MiB、journal はレーンのリングで `State`/`Timing` 2048 件・`Actuation` 6144 件・`KeyInput` 8192 件(`journal_policy.rs:20-32`)。drift の送信は Actuation レーンで `ActuationDecision`・`SentInput` と同居する。典型的な報告でどちらが何分ぶんかは**未測定**。「起動直後しか残っていない」型の不足(BUG-095・104)が過去にある。

## 3. 選択肢

| 案 | 内容 | 撤去 | 追加 | 移行する消費者 | リスク |
|---|---|---|---|---|---|
| (a) journal を正にする | 組の手書きの行を消し、派生の行に足りないフィールドを `emit_tracing` に足す | 手書きの行 | `emit_tracing` の腕のフィールド。`ImeEvent` の中身を出すには 20 variant ぶんの展開が要る(`architecture_guard` が `?`/`%` を禁じるので `Debug` で済ませられない) | チェッカー・architecture_guard・testdata・anchor 表・手順書 | 純減しない組がある(下の収支)。利用者の既定ログから行が消え、遡れる範囲がリングに縮む可能性。ADR-139 の「主要フィールドのみ」の方針を変える |
| (b) ログを正にする | 組の journal の variant を消す | variant と中継 | 0 | 報告の診断手順 | 報告から事象が消える。構造化された VK 列が失われる(上)。**推奨しない** |
| (c) 事象ごとに寄せる | 読み手が無く、派生の行で足りる組だけ (a)。それ以外は残す | (a) の一部 | ほぼ 0 | ほぼ無し | 重複が残る組がある |

## 4. 推奨

(c) で始める。(a) への拡大は、組ごとの収支が純減になる場合だけにする。

**第 1 段階(最小)**: 2 行。

| 撤去 | 内容 | 前提 |
|---|---|---|
| #1 `[press-ledger]` の 3 行(`platform_state.rs:228,234,255`) | 派生の `press write claim` が同じフィールドを出す。リポジトリ全域の grep で読み手 0、診断での使用 0 | `:228`(衝突時)は info で、ADR-208 L1 の異常を示す唯一の利用者ログ。消してよいかは所有者に聞く(質問 1)。消すときは `platform_state.rs:216` の doc(「衝突…はログ〈info〉と journal に残す」)も直す |
| #2 `[giveup-follow] cold=… outcome=…`(`ime_refresh.rs:301` だけ。`platform.rs:271,279` は残す) | 2026-10-04 の `d75c1d8b`(PR #480、BUG-074)で journal 記録と同時に足された行 | `gen_at_probe`・`gen_now`・`explicit_intent` は ADR-227 の世代の判定の根拠。足さずに消すか、`GiveUpFollow` に足すかを BUG-074 の担当に確認する。`BUG-074.md:319` の使い捨てブランチ(`ci/adr225-d0`・`ci/adr227-verify`)は `git ls-remote origin` に無く、今動いている読み手は無い(round3 で確認)。**同じ PR で `BUG-074.md:319` を書き換える**(「CI ゲートを作り直すなら派生の `give-up follow` 行〈cold_seq・outcome・baseline〉を読む」)。接頭辞 `[giveup-follow]` は `platform.rs:271,279` も使うので、`\[giveup-follow\]` の正規表現でゲートを作ると、`ime_refresh.rs:301` を消しても別の行に当たって通ってしまう。読むべき行を名指しする |

- 行数: `[press-ledger]` は `if/else` ごと約 16〜17 行(`platform_state.rs:226-238`・`:253-255`)、`[giveup-follow]` は約 6 行(`ime_refresh.rs:300-305`)。合計 約 22〜23 行の撤去、追加 0(#2 でフィールドを足すなら +3 前後)。
- 検証: CI が green。加えて、消す断片が `tools/e2e/**/*.py`・`.github/workflows/*.yml`・`tools/e2e/ime_key_matrix/testdata/`・`docs/tasks/`・`crates/awase-windows/tests/*.rs` に無いことを、実装の PR で全域 grep し直す(27 断片の表だけでは足りない)。
- 同じ PR で、表に無い既存の断片(`Blacklist drift correction`・`[engine-input] vk=`・`phys_ctrl=` など)を anchor 表に足すことを推奨する(撤去ではないが、後の撤去を安全にする前提)。
- 取りやめ条件: 消す行の読み手が見つかる。または所有者が「利用者の既定ログに残す」と決める。

**第 2 段階(保留)**: チェッカーが読む組(#3〜#7)。組ごとの収支(見込み、未実測):

| 組 | 撤去 | 追加 | 収支 | 判断 |
|---|---|---|---|---|
| #3 Blacklist | 行 1 つ(3 行) | 派生の `actuation decision` で「Blacklist 経路の drift correction の結果」を 1 行で特定できるかは未確認(`caller` は出るが、outcome は最初の attempt の粗い判別子だけ)。チェッカー 3 本・CI の集計・手順書の書き換え | 純増の見込み | 保留。expect=observe の構成なので、消すと CI は green のまま `drift_log_fired` が 0 に化ける |
| #4 `[drift] correction` | 行 1 つ(約 6 行) | `ImeEvent` の派生の行に `DriftDetected` の中身を出す展開(20 variant)か `ImeActuation` に `source`・`confidence`、architecture_guard の目印の付け替え、チェッカー 4 本 | 純増 | **推奨しない**。診断で最も使われた行(13 / 9)で、warn として利用者のログに出る唯一の drift 補正の痕跡 |
| #5 `[engine-input]` | 重複項目 | `KeyInput` に InputContext・scan・修飾キー・拡張ビット、派生の行にも出すなら同数、チェッカー 4 本 | ほぼ同じか純増 | README 段階 4(B5)の一部としてなら価値がある(重複の解消のためだけなら保留) |
| #6 `[ime-io] actuation SendInput` | 行 1 つ | `SentInput` の型に `kind` を足し、派生の行に vk・kind を出す。チェッカー 1 本 | 純増の見込み | 保留。派生の行で vk を出すと、所有者が派生の行を info にした場合(質問 1)、利用者の awase.log に構造化された VK 列が常に出る |
| #7 `[hook] IME-mode` | 行 1 つ | `extra`・`since_actuation_us` を journal と派生の行に足す、チェッカー 1 本 | ほぼ同じ | 保留 |

**入力内容の 1 か所化(#5・#10)**: README 段階 4 と一緒に判断する。`[key-output]` に機械的な読み手は無い(`tools`・`.github`・`scripts`・testdata で 0)が、docs の診断の記述で使われている(round3 の数え方で 8 ファイル)。`[key-output]` を消す・debug に下げると、報告で遡れる入力内容が awase.log の 16MiB から journal の 10 分の窓まで縮み、所有者の「VK 列は必須」の趣旨(障害対応の材料)に逆行する。**取りやめ条件: README E6 で窓を外す・広げると決める前には進めない**。

## 5. 所有者に聞くこと

1. 利用者の既定ログ(info)の扱い: 手書きの info/warn の行を消すと、利用者の awase.log から消える。報告の journal に残るのは、リングに収まる範囲だけ(2 節)。awase.log だけで診断する場面を残したいか。特に #1 の衝突時の行(ADR-208 L1 の異常)。なお報告の `attach_log` は journal と awase.log の両方をまとめて添付する(`bug_report.rs:570`・`:856`・`:864`)ので、添付ありの報告なら `PressWriteClaim` は journal に残る。失うのは、報告を使わずに awase.log だけを見る場面(issue に awase.log を貼るなど)に限られる。推奨: 消す(#1・#2 とも)。
2. CI の判定が派生の行(`awase::journal`)を読むことにしてよいか。そのために `emit_tracing` に出すフィールドを増やしてよいか(ADR-139 の「主要フィールドのみ」の変更)。
3. 報告で遡れる範囲が、awase.log(16MiB)からリング(Actuation 6144 件など)へ縮む可能性を受け入れるか。判断の前に典型的な報告 1 件で両者を測る(未測定)。
4. #4 `[drift] correction` は残す、でよいか(推奨は残す)。
5. 人手の確認手順(`v2-manual-verification-guide-2026-09-29.md` など)の書き換えを伴う撤去をしてよいか。
6. 入力内容(#5・#10)を journal の 1 か所に寄せてよいか(README E6 の窓の判断と一緒に)。

## 確認できなかったこと

- 組の全数(抜き取りなので下限)。層が違う 4 系統が 1 対 1 か。
- `in line` 方式で行を読む python の全数(正規表現の粗い grep で 18 本)。人手の手順書の全数。
- 中継を経る記録(#6・#7)の派生の行の順序に依存するチェッカーの有無。
- 報告で awase.log とリングが何分ぶん遡れるか。
- #3 を派生の行で特定できるか。第 2 段階の行数(すべて見込み)。
- `BUG-074.md:319` の使い捨てブランチの確かめが見た `[giveup-follow]` がどの行か。
