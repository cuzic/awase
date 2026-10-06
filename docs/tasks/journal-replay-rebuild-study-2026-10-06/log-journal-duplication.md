---
type: companion-doc
title: |-
  ログ(awase.log の tracing 行)と journal の重複の整理(2026-10-06、検討のみ・コード変更なし)
---

# ログと journal の重複の整理

[README.md](README.md) の B4 段階 1 より先に、ログと journal の重複を減らす案を検討する。develop `777bf1db` 時点のコードで確かめた。
パスは `crates/awase-windows/src/` からの相対パス。VK 列は残す前提(所有者の訂正、README 冒頭)。

## 1. 事実

### 1.1 仕組みとして既にある「journal → ログ」

journal の全 21 variant は、記録されると `UnifiedJournal::absorb`(`journal.rs:1309`)から `emit_tracing`(`journal.rs:806-1186`、約 380 行)で
`debug!` の 1 行(ターゲット `awase::journal`、文言は `"key input"`・`"ime open applied"`・`"gji fsm transition"` など variant ごとに 1 種)として awase.log にも出る(ADR-139 決定4 Option C: journal を正とし、ログは派生)。
つまり **journal に載る事象は、ログに既に必ず 1 行ある**。重複として整理すべきなのは、記録点の近くで**別に手書きした自由な tracing 行**が同じ事象を出しているもの。

ログのレベルの差: 利用者の awase.log の既定は `info`(`app/bootstrap.rs:152`)なので、journal 由来の `debug!` 行は利用者のログには出ない(CI は `RUST_LOG=debug`、`e2e-ime.yml:1017`)。
手書きの行のうち `info!`/`warn!` のものは利用者のログに出る。

中継を経る journal の記録は、ログの行が出る時刻が事象の時刻ではなく journal に移った時刻になる: `pending_journal_entries`(`platform.rs:126`)、`SENT_INPUT_TRACE`(`win32.rs:259-285`)、フックの診断キュー(`hook.rs:1393` の `Mutex<VecDeque>`、`message_handlers.rs:105` で吸い出す。inventory.md §1 の入口 3 つに加えて 4 つ目の経路)。

### 1.2 同じ事象を手書きのログ行と journal の両方に出している組

| # | 事象 | 手書きのログ行(場所・レベル・文言の先頭) | journal | 記録点 | フィールドの差 | ログ行を機械的に読むもの |
|---|---|---|---|---|---|---|
| 1 | 押下 ID の書き込み予約 | `state/platform_state.rs:233`(info、衝突時)・`:239`(debug)・`:260`(debug、解除) `[press-ledger] press=… source=… open=… → …` | `PressWriteClaim`(`:244`・`:262`) | 同じ関数 | 同じ(press・open・source・verdict) | なし |
| 2 | give-up 後の外部クローズの読み直し | `runtime/ime_refresh.rs:302`(info) `[giveup-follow] cold=… outcome=…` | `GiveUpFollow`(`:311`) | 同じ関数 | ログが多い: `gen_at_probe`・`gen_now`・`explicit_intent` | なし |
| 3 | Blacklist 経路の drift correction の結果 | `ime_refresh.rs:1020`(info) `Blacklist drift correction: apply_ime_open(…) → …` | `ActuationDecision`(`:1018`)+ `ImeOpenApplied`(`on_ime_apply_complete` 経由) | 同じ関数 | journal が多い | なし |
| 4 | drift correction の送信 | `ime_refresh.rs:958`(**warn**) `[drift] correction: observed=… ≠ desired=… for …ms → set_ime_open(…) (source=… confidence=…)` | `ImeActuation`(`:970`)+ `ImeEvent::DriftDetected{desired, observed, duration_ms}` | 同じ関数 | ログにだけ `source`・`confidence`。journal にだけ方針・試行回数・世代 | `check_startup.py`・`check_invariants.py`(断片 `[drift] correction`) |
| 5 | 物理キー 1 件のエンジン処理 | `runtime/key_pipeline.rs:153`(debug) `[engine-input] vk=… ts=… delay=… state=…`、`:180`(CTRL MISMATCH) | `KeyInput`(`:229`) | 同じ関数(`kp_run_inner`) | ログにだけ `delay`・`phys_ctrl` 等、journal にだけ `state_before/after`・`decision`・`physical`・畳み込み | `check_startup.py`(`[engine-input]`)、`check_drift_recovery.py`・`check_drift_recovery_chrome.py`・`check_keymatrix.py`(`phys_ctrl=`、`e2e-ime.yml:463`) |
| 6 | awase が SendInput で送ったキー | `win32.rs:338`(debug、actuation のときだけ) `[ime-io] actuation SendInput kind=… vk=…`、`:342` → `shadow_send_trace.rs`(debug) `[shadow-send] channel=SendInput …` | `SentInput`(中継 `SENT_INPUT_TRACE`) | 同じ関数(`send_input_safe`) | journal は全送信・`accepted`・Unicode の `ch`。`[ime-io]` は actuation の `kind` を持つ | `check_startup.py`(`[ime-io] actuation SendInput kind=`)。`[shadow-send]` は 0 |
| 7 | フックが見た IME モードキー | `hook.rs:1487`(debug) `[hook] IME-mode vk=… self_injected=… scan=… extra=… since_actuation_us=…` | `HookImeModeDiagnostic`(フックのキュー → `message_handlers.rs:105`) | 同じ関数で両方を作り、journal は後で吸い出す | ログにだけ `extra`・`since_actuation_us`、journal にだけ `since_prev_ime_mode_ms` | `check_invariants.py`(`[hook] IME-mode vk=`) |
| 8 | フォーカスのプロセス変化 | `runtime/focus_tracking.rs:607`(info) `FocusChange [pid→pid] class: stale ime_on=…` | `FocusTransition`(`:64`)+ `ImeEvent::FocusChanged` | 別の関数(同じファイル) | ログは belief の状態、journal はアプリ名・滞在時間・プロファイル | `e2e-ime.yml:1162` の grep(`FocusChanged`・`[focus` など。どの行に当たるかは未確認) |
| 9 | ConvClassify の結果 | `key_pipeline.rs:803`(debug)・`:815`(info) `[idle-conv-check] TsfNative: conv=… → belief …` | `ConvClassifyCall`(`:788`) | 同じ関数 | 一部だけ重なる(ログは belief の変化、journal は分類の入力と結果) | なし |
| 10 | ImeEvent の受理 | `state/ime_event_log.rs:53`(trace) `[ime-event seq=…] …` | `ImeEvent`(`platform_state.rs:206`) | 同じ流れ(`dispatch_event`) | 同じ | なし。PR #522(ADR-232 S1、未マージ)でリングごと消える |

部分的に重なるが層が違うもの(`output`/`tsf` は journal を直接参照できないガードがあり、journal は `platform.rs` が中継して書く):
GjiFsm の遷移(`platform.rs:519,552,803,817` などの `[gji-fsm]` 行 ↔ `GjiFsmTransition`)、TSF probe(`output/tsf_warmup_coord.rs:223`・`tsf/warmup/probe_fsm.rs:667` の `[tsf-probe]` ↔ `TsfProbeStarted/Completed`)、
literal 判定(`tsf/warmup/literal_detect_fsm.rs` の `[literal-detect]`・`[raw-tsf-literal]` ↔ `LiteralDetect`)、deferred の flush(`output/vk_send.rs:93`・`output/mod.rs:1324` ↔ `DeferredRecoveryFlush`)。
これらは事象の粒度がそろっていない(ログは途中経過、journal は結果 1 件)ので、1 対 1 の重複かは事象ごとに見ないと言えない(**未確認**)。
もう 1 つ、`on_ime_apply_complete` の `#[tracing::instrument]`(`runtime/mod.rs:900` 付近、`open`・`outcome`・`generation`・`reason`)は `ImeOpenApplied` と同じ値をスパンとして各行の前に付ける。

### 1.3 片方にしか無い事象

- journal だけ(ログは派生の 1 行だけ): `TimerFired`・`ClockAnchor`・`DumpTriggered`・`DriftGiveUpDiagnostic`・`DriftGiveUpIntervalEnded`・`TsfProbeCompleted`(結果)・大半の `ImeEvent`。
- ログだけ: 手書きの行の大部分。CI が読むものだけ挙げても `[warrant-shadow]`・`would_have_blocked=`(`ime_controller.rs:558`)、`explicit_intent=`(`ime_refresh.rs:146,1120` ほか)、`[startup-align]`、`[msime-ready]`、`[vk-send]`、`send_keys: mode=`、`stale confirm 検出`、`[raw-tsf-literal] flush escape=`、`Engine (de)?activated`、`[hook-watchdog]`、`Hook watchdog`。

### 1.4 件数

- 手書きの tracing の呼び出し: awase-windows の `src` に 740 か所(粗い grep、テストのモジュールを含む)。ルートの core `src` に 32 か所。
- journal の variant: 21(派生のログ行も 21 種)。
- 手書きの行と journal の組: 同じ関数で同じ事象を出す組 9(上の表の #1〜#7・#9・#10)、別の関数の組 1(#8)、層が違い 1 対 1 か未確認のもの 4 系統。#10 は #522 で消える。

## 2. 消費者

| 消費者 | 読む形式 | 中身 |
|---|---|---|
| `tools/e2e/ime_key_matrix/check_*.py`(20 本) | awase.log のテキスト | `test_log_anchors_in_rust_source.py:22-49` の 27 断片。journal 由来(派生の行)は `ime open applied`・`actuation decision`・`gji fsm transition`・`literal detect` の 4、残り 23 は手書きの行 |
| `.github/workflows` の grep | awase.log のテキスト | `e2e-ime.yml:1109`(`Engine (de)?activated`・`IME open axis delegated`・`outcome=Unwarranted`)、`:1162-1164`(フォーカス・external-change・reinject)、`e2e-uwp-inputsite-hook-watchdog-probe.yml:75,102`。27 断片の表には入っていない |
| `tools/e2e/ime_key_matrix/testdata/*.awase.log`(22 本) | 実機ログの抜粋 | チェッカーの単体テストの入力。文言を変えると書き直しが要る |
| report-worker | どちらも不透明 | journal・awase.log とも gzip+base64 のまま保存し、中身を見ない(`services/report-worker/src/index.ts:77`)。重複の整理で変更は要らない |
| 不具合報告の診断(人・Claude、`bug-report-fetch` スキル) | journal(JSON)と awase.log の両方 | known-bugs で journal が役立ったのは 17 件(inventory §7)。awase.log の行を根拠にした記述も多い(BUG-109・110・170 など「journal と app_log の突き合わせ」) |
| replay・`tests/journals/` | journal の型 | ログは読まない |
| 閉ループ | どちらも読まない | `h.writes` など |

どちらを正にするかで移行が要る消費者:
- journal を正(手書きの行を消す)にすると、手書きの行を読むチェッカー(#4・#5・#6・#7 の 4 組に当たる 5 本)と testdata・anchor 表を、派生の行の文言(`ime actuation`・`key input`・`sent input`・`hook ime-mode diagnostic` とフィールド名)に書き換える。利用者の既定ログ(info)からは #1(衝突時)・#2・#3・#4・#8 の行が消える。
- ログを正(journal の variant を消す)にすると、不具合報告の journal からその事象が消える。利用者のログは info なので、debug の行(#1 の通常時・#5・#6・#7)は報告のどこにも残らなくなる。`KeyInput` を消すことは VK 列を残す前提に反する。

## 3. 選択肢

| 案 | 内容 | 撤去 | 追加 | 移行する消費者 | リスク |
|---|---|---|---|---|---|
| (a) journal を正にする | 組になっている手書きの行を消し、ログは既存の派生の 1 行だけにする。手書きの行にしか無いフィールド(#2 の世代・意図、#4 の `source`・`confidence`、#5 の `delay`・`phys_ctrl`、#7 の `extra`・`since_actuation_us`)は、要るものだけ journal の variant に足す。新しいマクロ・仕組みは作らない(派生は ADR-139 で既にある) | 手書きの行 8〜9 組ぶん(1 呼び出し 3〜25 行、合計は実装時に実測。数十行の見込み) | journal のフィールド(要るものだけ)と `emit_tracing` の腕の更新 | チェッカー 5 本(`check_startup`・`check_invariants`・`check_drift_recovery`・`check_drift_recovery_chrome`・`check_keymatrix`)・testdata・anchor 表(#4〜#7 を含める場合) | 利用者の既定ログ(info)から行が消える(報告には journal が残る)。中継を経る #6・#7 は、派生の行が出る時刻・順序が変わる(チェッカーが前後関係を見ていれば壊れる、未確認)。報告の journal の形はフィールドの追加だけで、読み方の変更は小さい |
| (b) ログを正にする | 組の journal の variant を消す | variant 最大 7 と `emit_tracing` の腕、フックのキュー、`SENT_INPUT_TRACE` など | 0 | 報告の診断手順 | 利用者のログ(info)に debug の行は出ないので、報告から事象が消える。`KeyInput`・`SentInput` を消すのは VK 列を残す前提に反する。**推奨しない** |
| (c) 事象ごとに寄せる | 機械的な読み手が無い組(#1・#2・#3・#9)は (a)、チェッカーが手書きの行を読む組(#4〜#7)は当面そのまま(重複を残す) | (a) の一部 | (a) の一部 | なし(チェッカーは触らない) | 重複が残る組がある。残す組は「チェッカーの書き換えと引き換えに消せる」と記録しておく |

却下済みとの照合: (a) は共通形式・DSL・宣言テーブルを作らず、既存の派生(ADR-139)を使うだけ。ADR-226 候補 E(チェッカーに journal の JSONL を読ませる)とは違い、チェッカーは awase.log のテキストを読み続ける。

## 4. 推奨

推奨は (c) から始め、チェッカーの書き換えの費用を見て (a) に広げる。

**第 1 段階(最小)**: 機械的な読み手が無く、journal が同じかそれ以上の情報を持つ組の手書きの行を消す。

| 撤去 | 内容 |
|---|---|
| #1 | `[press-ledger]` の 3 行(`platform_state.rs:233,239,260`)。フィールドは `PressWriteClaim` と同じ |
| #3 | `Blacklist drift correction: …`(`ime_refresh.rs:1020`)。`ActuationDecision`・`ImeOpenApplied` が持つ |
| #2 | `[giveup-follow] cold=… outcome=…`(`ime_refresh.rs:302`)。`gen_at_probe`・`gen_now`・`explicit_intent` を `GiveUpFollow` に足すか、要らないと判断して足さないかを先に決める(BUG-074 の調査で使われたかは未確認) |

- 検証: CI が green。特に `invariants-unit`(`test_log_anchors_in_rust_source.py` を含む)と `check_*.py` の単体テスト。消す行の断片は 27 断片の表にも workflow の grep にも無いことを、実装の PR で grep し直して確かめる。
- 取りやめ条件: 消す行を読む消費者(チェッカー・workflow・リポジトリ外のスクリプト)が見つかる。または所有者が「利用者の既定ログ(info)にも残す」と決める(#1 の衝突時・#2・#3 は info)。

**第 2 段階**: #4〜#7(チェッカーが読む組)を、チェッカー・testdata・anchor 表を同じ PR で派生の行の文言に書き換えて消す。1 組ずつ PR にし、各 PR で該当の実機 CI 構成を回して判定が変わらないことを確かめる。#6 は README B4 段階 1(`shadow_send_trace` の撤去)とまとめる。中継を経る #6・#7 は、行の順序を見るチェッカーがあるかを先に確かめる(**未確認**)。

**第 3 段階**: 層が違う 4 系統(GjiFsm・TSF probe・literal・deferred)は、F6 で Output が記録を返す形になってから、事象の粒度をそろえられるか見直す。

## 5. 所有者に聞くこと

1. 利用者の既定ログ(`info`)の扱い: journal を正にすると、info/warn の手書きの行が利用者の awase.log から消える(報告に添付される journal には残る)。awase.log だけで診断する場面(報告を使わない issue など)を残したいか。残すなら、派生の行の一部を info にするか(ADR-139 は「常時肥大化を避けるため debug に統一」と決めている)、該当の組は (c) で手書きの行を残す。
2. 第 2 段階で、チェッカー 5 本と testdata の文言を派生の行(`awase::journal` の `key input`・`ime actuation`・`sent input`・`hook ime-mode diagnostic`)へ書き換えてよいか。
3. #2 の `gen_at_probe`・`gen_now`・`explicit_intent`、#4 の `source`・`confidence`、#5 の `delay`・`phys_ctrl` を journal に足すか(足すなら報告の journal が少し大きくなる)。
4. #4 の `[drift] correction` は warn で、利用者のログにも出る唯一の drift 補正の痕跡。journal を正にして消してよいか。

## 確認できなかったこと

- 層が違う 4 系統が 1 対 1 の重複かどうか。
- 中継を経る記録(#6・#7)の派生の行の順序に依存するチェッカーの有無。
- `e2e-ime.yml:1162` の grep がどの行(手書きの `FocusChange` か、派生の `ime event` の `FocusChanged` か)に当たっているか。
- 撤去の行数の合計(実装の PR で実測する)。
- リポジトリ外で awase.log の行を読むもの。
