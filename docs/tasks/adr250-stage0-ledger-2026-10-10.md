# ADR-250 段階 0 の台帳(2026-10-10)

[ADR-250](../adr/250-boundary-journal-and-log-unification.md) の段階案 0(コード変更なし、`ANCHORS` の拡充だけ)の結果。
調べた版は develop `7cecbcf6`(#580 マージ後)。ローカルでのビルド・テストはしていない(`test_log_anchors_in_rust_source.py` の Python 単体テストだけ実行)。
件数は `git grep` による機械集計で、全件の手作業レビューはしていない。

## 1. CI の `awase.log` の大きさ(閾値の根拠)

`e2e-ime.yml` は `RUST_LOG=debug`(`:1140`)、上限は 20MB(`app/logging.rs:24` の `MAX_LOG_BYTES`)。`gh run download` で `dist/awase.log` を実測した。

| run | job | `awase.log` | 行数 |
|---|---|---|---|
| 38029357773(`ci/e2e-dictation`) | `result-tsx-dict-chromepage-gji-paste-30ms-1` | **4,580,658 B**(上限の 22.9%) | 27,594 |
| 37739968036 | `result-sc-table-gji-atok-passthru-1` | 4,049,681 B | 23,145 |
| 38041351075 | `result-tsx-dict-chromepage-gji-unicode-30ms-1` | 2,781,099 B | — |

選び方: 直近 40 run(`e2e-ime.yml`、cancelled を除く)の `result-*` artifact のうち**圧縮サイズが最大**のもの(`awase.log` は約 16:1 で圧縮されるので圧縮サイズの大小が元の大小に対応する)。1 位 520,975 B(上の 1 行目)。2〜4 位は同じ run の別 shard(519,585・471,211・469,709 B)で、別の run の最大は 447,440 B(上の 2 行目〈`sc-table`〉)。

1 位の run の内訳(`awk` で行頭のレベルを数えた):

| レベル | 行数 | バイト | 割合 |
|---|---|---|---|
| DEBUG | 26,569 | 4,396,218 | 96.0% |
| INFO | 970 | 173,103 | 3.8% |
| WARN | 55 | 11,337 | 0.2% |

- `target` が `awase::journal` の行(`emit_tracing` の複製): 3,040 行・681,623 B(**バイトで 14.9%**)。ADR-222 の「約 10%」は行数での値で、バイトでは 15% に近い。
- 既定 `info` の下での出力(INFO+WARN): 1,025 行・184,440 B(**ADR-250 決定 5 の「出力件数が増えない」の基準値**)。

**閾値の提案**(所有者の確認を待つ):
1. 既定 `info` の出力件数: 置き換えの各段階で INFO+WARN の行数が増えないこと(基準 1,025 行/run は run の内容で変わるので、同じ job を同じ入力で段階の前後に回して比べる)。
2. `RUST_LOG=debug` の CI で `awase.log` が **10MB(上限の 1/2)を超えない**。測った最大 4.58MB の約 2.2 倍の余裕。
3. 生成の追加で `awase::journal` 以外の target の DEBUG が増える分も、2. の中に入る(合計で測る)。

**測っていないもの**: 較正 CI(`ci/e2e-adr191` 系)・長時間の打鍵 CI(`ts-chrome`)・`adr190-run.ps1` の `awase.log`。これらが 10MB に近いなら閾値の前提が変わるので、段階 4 の最初の PR の前に 1 本ずつ測る(artifact 名が分かれば `gh run download` で足りる)。

## 2. 報告 journal のサイズ(2026-10-10 に実測)

`scripts/fetch_latest_bug_report.py --count 5` で R2 の直近 5 件を取得した(初回は HTTP 401 だったが、`wrangler whoami` を 1 回動かして OAuth トークンを更新したら通った。以後は `fetch_latest_bug_report.py` の前に `wrangler whoami` を挟む)。
集計はレーンと件数・バイト・時間幅・`DumpTriggered` だけで、入力内容(打鍵・`ch`)は読んでいない。レーンは `journal.rs` の `lane_kind` の分類(S=State、T=Timing、K=KeyInput、A=Actuation〈`SentInput` を含む〉)。

| 報告 | 全体 | S 件数(幅) | K 件数(幅) | T 件数(幅) | A 件数(幅) | 追い出し(S/T/A/K) |
|---|---|---|---|---|---|---|
| 1 | 697 件・248KB | 154(11.4分) | 398(8.9分) | 121(11.0分) | 24(3.3分) | 0/0/0/0 |
| 2 | 4,238 件・1.25MB | **2048**(162.7分) | 1593(9.2分) | 451(160.7分) | 146(168.8分) | 780/0/0/0 |
| 3 | 1,520 件・455KB | 888(9.7分) | 581(9.4分) | 48(9.6分) | 3(9.3分) | 0/0/0/0 |
| 4 | 5,728 件・1.47MB | **2048**(45.9分) | 685(9.3分) | 1970(196.7分) | 1025(1605分) | 18093/6711/8297/54095 |
| 5 | 74 件・23KB | 23(1.1分) | 28(0.7分) | 20(0.8分) | 3(0.4分) | 0/0/0/0 |

- 1 件あたりのバイト数(JSON): KeyInput 約 450B、State 約 190〜280B、Timing 約 190〜200B、Actuation 約 150〜330B。
- 報告 4 は稼働約 26 時間(`oldest_elapsed_ms` が約 9,350 万 ms)で、全レーンが追い出された。それでも KeyInput の幅は 9.3 分(10 分窓)、State は満杯で **45.9 分**を保っていた。
- State レーンが満杯(2048)になったのは 2 件。満杯のときの State の幅は 45.9 分(ポーリングが多い報告 4)と 162.7 分(静かな報告 2)。**満杯になっても 10 分を大きく超えて残る**。
- KeyInput の幅が 9 分前後なのは、10 分窓(`journal.rs` のダンプ時の絞り込み)によるもの。
- 報告全体のサイズ: 最大の journal が 1.35MB(JSON 展開後)。ADR-222 D5 の本文上限 2MiB に対し、報告 4 の journal だけで 7 割弱。Facts を足してレーンが増えると、10 分窓を外したときに超える余地は小さい。

**この実測から言えること(n=5、同一の収集経路の報告のみ)**:
1. N(各レーンの最低保持)を **10 分**にしても、現行の件数上限(各 2048)で、実測した 5 件全てが満たしている(満杯のレーンの最短は 45.9 分)。レーン容量の変更は要らない。
2. Plan の edge 記録(段階 1)で増える分は、State(ポーリング)に比べて小さい。edge は理由が変わった回数だけなので、State レーンの幅を縮める量にはならない見込み(段階 1 の実機の報告が来てから再確認)。
3. レーンの分け方(Key / Timer・その他の入力 / Plan)を変える根拠は、この 5 件には無い。変える場合は、追い出しの時期が違って片方だけが残る問題(決定 6)を起こさないことの確認が別に要る。
4. 10 分窓を外すとサイズが大きく増えうる(倍率は未算出。報告 4 では窓の外の KeyInput が 54,095 件追い出されていて、残る件数は容量〈KeyInput レーンの上限〉で決まる)。決定 7 の削除対象の付け替えと、本文上限(2MiB)の再試算が先。

**測れていないこと**: 報告は 5 件で、最新の 1 台分に偏る可能性がある。`DumpTriggered` が出る前の古い journal 形式の報告は含まない。

## 3. 事実の訂正(ADR-250 本文に反映する分)

| ADR の記述 | 実測 | 訂正 |
|---|---|---|
| 「`gji_on_event(` の呼び出し点: `platform.rs`(6 か所)」 | `git grep -nE "gji_on_event\(" -- crates/awase-windows/src` で `platform.rs` は 535・620・702・732・746 の **5 か所**(1063 は doc コメント)。`GjiFsm` への入口はもう 1 つ `gji_on_long_idle`(`platform.rs:764`、`output/mod.rs:462`)があり、これを足すと 6 経路 | 「5 か所 + `gji_on_long_idle` の 1 経路」 |
| 「`GjiFsmTransition` を記録するのは `note_gji_transition`(`:133-137`)と probe tick(`:425`)だけ」 | `note_gji_transition` の呼び出しは 538・624・706・733・747・765 の 6 か所、直接の `push_journal_entry` が `:425` の 1 か所 | そのまま有効(件数を足す) |
| `output/mod.rs:1159`・`output/vk_send.rs:223`・`:401` は記録されない | `git grep note_gji_transition -- crates/awase-windows/src/output` は 0 件。3 か所とも記録なし | 確認できた。`[gji-fsm]` 置き換えの前に、この 3 か所から `platform.rs` へデータを持ち上げる配線が要る |
| 手書き `[gji-fsm]` は約 35 件 | `"[gji-fsm]` の文字列は 36(`platform.rs` 10、`tsf/gji_fsm.rs` 26) | 36 件 |

## 4. 読み手の台帳

範囲: `tools/**`・`.github/**`・`crates/*/tests/**`・`docs/tasks/*verification*`(`tools/e2e/ime_key_matrix/results/**` の過去のログ置き場と `testdata/**` は読み手ではなく**入力データ**なので除く。`testdata/*.awase.log` は 22 本で別に数える)。184 ファイルを対象にした。

方法: Rust の `"[タグ]` で始まる文字列リテラル 146 種(`git grep -ohE '"\[[a-z][a-z0-9_-]+\]'`)ごとに、対象ファイルでの出現を `grep -F` で調べた。手書きの文言そのもの(タグを持たない行)は 4.2 に別に挙げる。

### 4.1 タグつきの手書き行の読み手(読み手のあるタグだけ)

| タグ(Rust 側の件数) | 読み手 | 種別 |
|---|---|---|
| `[gji-fsm]`(36) | `check_invariants.py`(`GJI_STUCK_RE`)、`check_reopen.py`(`stuck`)、各 test、`ime_key_matrix/README.md`、testdata 3 本 | **機械**(文言) |
| `[drift]`(6) | `check_invariants.py:48`(`DRIFT_RE`)、`check_startup.py:18`、`e2e_common.py:19`、`e2e-ime.yml`、`check_drift_recovery.py`、README、`docs/tasks/v2-*verification*` | **機械**(文言・件数) |
| `[engine-input]`(2) | `check_run_validity.py:38`、`check_keymatrix.py:55`(`ENGINE_DOWN`)・`:57`(`mods(c=true `)、`check_drift_recovery.py:76`、`check_drift_recovery_chrome.py:6`、`check_multi.py:55`(`extra=`)、`suspend_report.py:18`(`delay=`)、`check_startup.py:21`、`e2e_common.py:21`(`phys_ctrl=true`)、`e2e-ime.yml` | **機械**(フィールドまで) |
| `[raw-tsf-literal]`(8) | `check_reopen.py:49`(`flush escape=true`)、`test_check_reopen.py` | 機械 |
| `[literal-detect]`(8) | `test_check_invariants.py` のみ(検査の本体は無く、テストの入力) | 機械(テスト入力) |
| `[hook]`(7) | `check_invariants.py:58`(`HOOK_SELF_RE`、`[hook] IME-mode vk=… down self_injected=true`)、README | 機械。**フックスレッドの診断で `SentInput` と同じく置き換えない** |
| `[vk-send]`(2) | `check_reopen.py:56`(`prepend_f2_warmup=`)、`e2e-ime.yml` | 機械 |
| `[warrant-shadow]`(2) | `check_invariants.py:59`(`WARRANT_RE`、`would_have_blocked=true`)、testdata 2 本 | 機械 |
| `[startup-align]`(1) | `check_startup.py:17`、`invariant_limits.json`、testdata 1 本 | 機械 |
| `[msime-ready]`(6)・`[ime-io]`(2) | `check_startup.py:19,23` | 機械 |
| `[stage-observe]`(4)・`[external-change]`(1)・`[focus-scope]`(1) | `e2e_common.py:18,23`、`check_run_validity.py:40`、`check_drift_recovery.py` | 機械 |
| `[mode-key-follow]`(5) | `mode_key_pass_timeline.py:62` | 機械 |
| `[ctrl-bypass]`(6)・`[post_bypass]`(1) | `config_verify/reload_post_bypass.py:26`、`bug103-verify.yml` | 機械(workflow) |
| `[keymap]`(14)・`[general]`(2)・`[keys]`(1) | `config_verify/run.py` | 機械(C。内部診断なので対象外) |
| `[injection_mode]`(1) | `tray_cache_clear/run.py`、`tray-cache-clear.yml` | 機械(C) |
| `[pending-deferred]`(3)・`[focus-settle]`(1) | `e2e-ime.yml` の Select-String | 機械(workflow) |
| `[shadow-toggle]`(12) | `docs/tasks/v2-b3-t1-ci-verification-2026-09-29.md`、`v2-manual-verification-guide-2026-09-29.md` のみ | **人**(文書。機械の読み手なし) |
| `[focus]`(7)・`[focus-sync]`(2)・`[tsf-send]`(2)・`[key-effect-predict]`(2)・`[reinject]`(1) | `docs/tasks/v2-*verification*` のみ | **人**(文書) |

読み手が上の表に出ないタグ(= 機械も文書も読んでいない): `[apply-ime]`(22)・`[ime-mode]`(18)・`[idle-conv-check]`(13)・`[tsf-gate]`(9)・`[conv-actuate]`(8)・`[composition]`(8)・`[tip-detect]`(15)・ほか約 100 種。

### 4.2 タグを持たない手書きの文言の読み手(`ANCHORS` に無かったもの)

| 文言(Rust 側) | 読み手 | `ANCHORS` |
|---|---|---|
| `Blacklist drift correction: apply_ime_open(` | `check_drift_correction.py:28`(`DRIFT_LOG_PATTERN`)、`e2e_common.py:19`(`DRIFT_RE`) | **追加した**(消すと `drift_log_fired` が黙って 0 になる) |
| `key input seq=… vk_code=… decision=`(`journal.rs:844` の `"key input"` + フィールド) | `check_consistency.py:58` | **追加した**(メッセージ部分のみ) |
| `Engine activated/deactivated … reason=`(`src/engine/engine.rs:380`、`"Engine {} (ime=…, reason={:?})"`) | `check.py:79`、`effect_learning.py:121` | 追加しなかった(書式引数で `activated` が組み立てられ、固定断片が取れない) |
| `idle-conv-check-diag`・`giving up`・`GJI reinit`・`VK_IME_ON 送信` | `check_drift_recovery.py:68-69` | **追加した** |
| `mode key PassThrough(`・`IME snapshot: `・`IME detection timed out` | `mode_key_pass_timeline.py:34-60` | **追加した** |
| `[ctrl-bypass] post_bypass armed`・`Config reload requested via WM_RELOAD_CONFIG` | `config_verify/reload_post_bypass.py:26-27` | **追加した** |
| `startup: `・`startup note: ` | `config_verify/run.py:22-23` | **追加した** |
| `[engine-input] vk=`・`phys_ctrl=`・`send_keys: mode=`・`delay=` | `check_run_validity.py`・`check_keymatrix.py`・`check_drift_recovery.py`・`suspend_report.py`・`e2e_common.py` | **追加した**(読むファイルごとに 1 行) |
| `mods(c=true `(`key_pipeline.rs:156`) | `check_keymatrix.py:57` | 追加しなかった(`_checker_mentions` の正規表現エスケープ規則が `(` を扱えず、検査が必ず失敗する) |
| `config_verify/run.py`・`tray_cache_clear/run.py` の `startup:` 系以外 | — | C(内部診断)。対象外 |

`ANCHORS` は 31 件から 52 件になった(+21、`tools/e2e/ime_key_matrix/test_log_anchors_in_rust_source.py`)。`python3 -m unittest test_log_anchors_in_rust_source` で 3 テストが通ることを確認した(`config_verify/` の 2 ファイルは `../config_verify/…` の相対パスで読む)。

`ANCHORS` が検出できないこと(変わらない): tracing のフィールド名(`seq=`・`outcome=`・`trigger="StartComposition…`・`state_before=`)の消失、`mods(c=true …` の中身。これらは決定 5 のとおり、置き換える PR ごとに該当チェッカーを同じ PR で直す。

### 4.3 そのほかの読み手

- `architecture_guard.rs`: `:403`(output/tsf は journal を参照しない)、`:2498` の `DRIFT_SEND_LOG_MARKER = "[drift] correction: observed="`、`:6123`(`emit_tracing` の `?`/`%`/ワイルドカード禁止)、`:6167`(`KeyInput` の構築は 1 か所)、`hook_callback` のログマクロ 7 件。
- `tools/e2e/ime_key_matrix/testdata/*.awase.log`: 22 本。`on_ime_apply_complete{…}:` のような span 名の前置きを含む。
- `crates/awase-settings/src/bug_report.rs:947-954`: `"type":"KeyInput"` の文字列一致(「打鍵の行をすべて削除」)。
- 台帳に**含まれない**もの: `tools/e2e/ime_key_matrix/results/**`(過去の run の保存ログ)。`.old`(`awase.log.old`)を読む workflow は無い(ADR の記述どおり)。

## 5. 組ごとの収支表(見積もり。判定ではなく PR 説明用)

撤去 = 手書きの `tracing::*!` 呼び出し数(Rust 側のタグ件数。複数行の呼び出しは 1 件)。追加 = `emit_tracing` の arm 1 つ(置き換え後の `tracing::<level>!(target: …)` で 10〜20 行)+ journal 側に足すフィールドと、`platform.rs` へ持ち上げる配線(`tsf/`・`output/` の組)。行数は桁の見積もり。

| 組 | 撤去 | 置き換え先 | 追加の見積もり | 読み手 | 位置・再入 | 判定 |
|---|---|---|---|---|---|---|
| `[apply-ime]` | 22(`runtime/open_chain.rs` 10、`ime_controller.rs` 7、`state/` 4、`platform.rs` 1) | `ImeOpenApplied`・`ActuationDecision` | 理由の見送りは既に型にある(`outcome`)。arm 1〜2 | 無し | `open_chain.rs:206-213` は `with_app` が `None` のとき記録を捨てる再入経路。**該当する行は残す** | **第 1 候補**(読み手なし)。再入経路の行を除く |
| `[shadow-toggle]` | 12(`key_pipeline.rs`) | `PressWriteClaim`・`ActuationDecision` | arm 1 | 文書のみ(機械なし) | `kp_shadow_actuate` は殻。再入の確認が要る | 第 1 候補 |
| `[idle-conv-check]` | 13(`key_pipeline.rs`) | `ConvClassifyCall` | フィールド追加 | 無し | 殻 | 第 1〜2 候補 |
| `[tsf-gate]` | 9(`tsf/tsf_gate.rs` 7、`output/mod.rs` 1、`runtime/mod.rs` 1) | (新規の Facts) | データ型 + `platform.rs` への持ち上げ + arm | 無し | `tsf/` は `architecture_guard.rs:403` のため**データを上に渡す方式**。`held queue full` は gate の中で出る行で、位置が変わる | 第 2 群。順序を読むチェッカー無しなので可 |
| `[conv-actuate]` | 8(`ime.rs` 4、`output/conv_actuation.rs` 4) | (新規) | データ持ち上げ | 無し | `output/` は同上 | 第 2 群 |
| `[composition]`・`[ime-mode]` | 8・18(`tsf/`・`ime.rs`・`platform.rs`) | `ImeEvent` の一部 | 一部のみ | 無し | `ime_mode_fsm.rs` の 9 件は `tsf/` | 第 2 群(要精査) |
| `[literal-detect]` | 8 | `LiteralDetect`(既にデータ方式で記録) | arm 1 | `test_check_invariants.py`(テスト入力のみ) | 位置は既に `platform.rs` | 第 2 群。読み手はテストだけ |
| `[raw-tsf-literal]` | 8(`output/` 5、`tsf/` 3) | `LiteralDetect` の周辺 | データ持ち上げ | `check_reopen.py:49`(`flush escape=true`) | `output/` | **読み手のある組**。チェッカーを同じ PR で直す |
| `[gji-fsm]` | 36(`tsf/gji_fsm.rs` 26、`platform.rs` 10) | `GjiFsmTransition` | 理由を `GjiAction` の variant で返す(`timed-fsm` の `Response` は変えない)。呼び出し点 3 か所(`output/mod.rs:1159`、`vk_send.rs:223/401`)の配線 + arm | `check_invariants.py:50,55`・`check_reopen.py:47,50-52`・testdata 3 本 | 手書きは FSM の `on_event` の**内側**で出る。記録点は `platform.rs` なので位置が変わる | **読み手のある組**。「`StartComposition while engine off`」「`trigger="…"`」を読むチェッカーを直してから。純増 |
| `[hook]` | 7(`hook.rs`) | `HookImeModeDiagnostic` | — | `check_invariants.py:58` | フックスレッド。`hook_callback` のログマクロ 7 件の固定 | **置き換えない**(ADR 決定 5) |
| `[drift] correction` | 6(`ime_refresh.rs`) | `ImeActuation`(試行ごと) | arm 1 + `basis`・`source=`・`set_ime_open(` を保つ | `check_invariants.py:48`・`check_startup.py:18`・`e2e_common.py:19`・`e2e-ime.yml`・`architecture_guard.rs:2498` | `ImeActuation` の構築は `ime_refresh.rs:961`・`:1052` の 2 か所。送信側の特定が要る | **最後**(診断で最も使われた行) |
| `Blacklist drift correction` | 1 + 1 | `ImeActuation` | — | `check_drift_correction.py:28`・`e2e_common.py:19` | — | **最後**(消すと `drift_log_fired` が 0 に化ける。ANCHORS を追加済み) |
| `[engine-input]`・`CTRL MISMATCH` | 2 + 1(`key_pipeline.rs:156,182`) | `KeyInput` + InputContext | `KeyInput` に InputContext を足す(決定 4) | 10 ファイル(4.1 の表) | `mods(c=true … phys_ctrl=true extra=0x` まで読まれる | **最後**。`KeyInput` の変更は決定 7(`contains_typed_text()`)が先 |
| `[mode-key-follow]` | 5 | (新規) | — | `mode_key_pass_timeline.py` | — | 読み手のある組 |

合計(上の表の撤去、`[hook]` を除く): 約 160 件。ADR の D(73 件の下限)と B(288 件)に対して、**読み手のない組(第 1・第 2 群)は約 100 件**、読み手のある組(`[gji-fsm]`・`[raw-tsf-literal]`・`[mode-key-follow]`)が約 49 件、最後の組(`[drift]`・`Blacklist`・`[engine-input]`)が約 11 件。内部診断の C には触れない。

規模の見積もり: 追加は `emit_tracing` の arm が 12〜15 個(10〜20 行)と、`tsf/`・`output/` の組のデータ型と配線(出来事ごとに 30〜60 行)で、**合計 +600〜1,000 行**、撤去は手書き約 160 件(複数行の呼び出しを含めて約 350〜450 行)。見込みは**純増(+150〜650 行)**で、ADR の「純増に近い」と一致する。実測は各段階の PR で行う。

## 6. 段階 0 で確かめることになっていた項目

| 項目(ADR の記述) | 結果 |
|---|---|
| `tsf/tsf_gate.rs`・`output/probe_io.rs`・`runtime/key_pipeline.rs` の shift-conv-guard・key-effect-predict は FCIS の今後の分割に入るか(決定 3) | `docs/tasks/fcis-layering-tasks-2026-10-06.md` を `grep` した範囲では**入っていない**。`tsf_gate` は T2(#492)で ungated 済みで終わり、F5d は「分けない」(`:230`)、`key_effect_predictor` は `CORE_MODULES` へ移動済み(`:265`)。`probe_io`・shift-conv-guard の分割を予定する行は無い。→ ADR の規則に従い「**分割の予定に無く、本 ADR では対象外**」と扱う(shift-conv-guard〈4 件〉・key-effect-predict〈2 件〉・`probe_io` の手書きは残す) |
| `pending_journal_entries` を経る型は push 時に生成できるか | 可能。`push_journal_entry`(`platform.rs:126-130`)は `self.stamper.stamp(entry)` に完全な中身を渡している。`emit_tracing` は `journal.rs:817` の private で、`absorb`(`:1327`)と `:1382` から `JournalEnvelope::emit_tracing`(`:1198`)経由で呼ばれる。**二重出力の防ぎ方(未実装)**: push 時に出した型の arm を `absorb` 側で出さないための印(`JournalEnvelope` に `emitted: bool` を足す、または型ごとに「push 時に出す」集合を持つ)が要る。段階 4 の最初の `pending_journal_entries` を経る型の PR で決める |
| `SentInput` とフックの診断の push 位置 | ADR の記述どおり(`win32.rs` の自由関数、`hook.rs` のフックスレッド)。`drain_journal_entries`(`platform.rs:94-117`)で `SentInput` を `stamp` つきで積み直しており、生成に置き換えない判断は変わらない |

## 7. 取りやめ条件(ADR の「取りやめ条件」1 つ目)に当たる組

読み手を壊さずには直せない組、または `with_app` 再入で記録が落ちる経路にある組:

- `[hook]`(7): 取りやめ(フックスレッド)。
- `runtime/open_chain.rs` の `[apply-ime]`(10): 再入で記録が落ちる経路。**手書きのまま残す**。残り 12 件は候補。
- `[engine-input]`・`[drift] correction`・`Blacklist drift correction`: 取りやめではなく**最後**(ADR の順序どおり)。

## 8. 次の段階への入力

1. 閾値(上の 1.)を所有者が確認する。
2. 報告 journal の実測は済み(2.)。
3. 段階 1(drift の Plan と `OmissionBasis` の記録)は、この台帳の影響を受けない(ログは手書きのまま)。
4. 段階 4 の最初の PR は `[apply-ime]`(再入経路を除く 12 件)か `[shadow-toggle]`(12 件)。どちらも機械の読み手が無い。
