# B5(入力の再生)の試作結果: BUG-105 の 1 件

2026-10-06。[README.md](README.md) の段階 4(B5 の最小形)のうち、**再生側だけ**を、記録の形式を変えずに BUG-105 の 1 件で試した。記録側(`KeyInput` に InputContext の 4 項目・拡張ビット・`win` 修飾を足すこと)には触れていない。

## 結論

- **一致した**。記録の `TimerFired` の並び(seq)でタイマーを発火する方式で、fixture を HEAD のエンジンに流すと、出力は既存テスト(`tests/scenarios.rs::scenario_3key_char1_released_tight_overlap_prefers_chord`)と同じ `lyou` になる。A↓ の時点で `lyo`(L+右親指=「ょ」)が出て、A はタイムアウトで `u` になる。記録(修正前を模したもの)と最初に食い違うのは A↓ の後の状態で、HEAD は `PendingChar(vk=0x41)`、記録は `Idle`。
- **記録側を変えずに再生できた**(停止条件には当たらない)。ただし BUG-105 は IME ON のまま打鍵し、修飾キーも特殊キーも使わない例なので、足りないフィールドが効かなかっただけ(下記「詰まった点」4)。
- **一番の詰まりはタイマーの時刻の系**。`timestamp_us` の仮想時計で期限どおりに発火すると、BUG-105 は再現できない(下記 3)。

成果物:
- 補助とテスト: `crates/awase-windows/src/key_input_replay_tests.rs`(`#[cfg(test)]`、`cfg(windows)` の外)
- fixture: `crates/awase-windows/tests/journals/key_input/bug-105-tight-d1.json`(`KeyInput` 4 件、journal のダンプと同じ形式)

## 行数(rustfmt 後)

| 部分 | 行数 |
| --- | --- |
| 記録を読む型(`KeyEventSummary`・`JournalEntry` の写し) | 約 37 |
| ① `KeyInput` → `RawKeyEvent`(親指の押下時刻・`was_down`・修飾・物理位置、オートリピートの展開を含む) | 約 70 |
| ② タイマーのループ(2 方式、`on_input`/`on_timeout` の呼び出し、効果の反映) | 約 135 |
| ③ エンジンの ON/OFF(InputContext の固定値) | 約 20 |
| ④ 設定・配列・n-gram の読み込み(`build_engine`) | 約 40 |
| 比較用(`Step`・食い違いの検出) | 約 45 |
| **補助の計** | **389 行**(空行・コメントを除くと 337 行) |
| BUG-105 のテスト 2 本と fixture の読み込み | 73 行 |
| fixture(JSON) | 102 行 |

## 詰まった点

1. **Windows 専用の分類器**: `hook::classify_key` は `#[cfg(windows)]` の `hook.rs` にある。記録した `key_class` を使い、物理位置は `scanmap::scan_to_pos`、修飾キー種別は `VkCodeExt::classify_modifier` で導いた(どちらも Linux で使える)。**BUG-105 では分類し直す必要は無かった**ので、`classify_key` を `vk.rs`/`state/` へ動かす必要は今のところ無い。分類し直しが要るのは、記録時と HEAD で分類が変わった不具合(親指キーの割り当て変更など)を再生したいときだけ。
2. **置き場所**: core(`src/engine/`)は `awase-windows` の `scan_to_pos` と journal の形式に依存できないので、`awase-windows` の lib テストに置いた。`cargo test -p awase-windows --lib`、CI では `cargo nextest run --workspace --lib` で走る。
3. **時刻の系の違い(最大の詰まり)**: `timestamp_us` の仮想時計で期限どおりに発火すると、右親指↓で張り直した `TIMER_PENDING` の期限(右親指↓+100ms = 111.7ms)が A↓(112.474ms)より 0.8ms 早く来る。するとタイムアウトで L+右親指が先に確定し、3 鍵の仲裁(`compute_prefer_char1`、BUG-105 の本体)を通らない。出力は同じ `lyou` になり、**修正前のコードでも同じ結果になるので BUG-105 を検出できない**(テスト `bug_105_replay_on_virtual_clock_times_out_before_char2` で固定)。実機では WM_TIMER が遅れて A↓ が先に処理された(報告のログの `Timer killed: logical=1`)。
   - 対処として、記録の `TimerFired` が並んだ位置(seq)で発火する方式を作った。BUG-105 の区間には `TimerFired` が無い(実機で発火しなかった)ので、最後の打鍵の後に残ったタイマーを期限順に発火するだけになる。
   - この方式の限界: (a) `TimerFired` は Actuation レーン(6144 件、`SentInput` と同居)にあり、`KeyInput` レーン(8192 件)より先に古いものが追い出される。追い出された区間ではタイマーの並びが分からない。(b) HEAD が記録時と違うタイマーを張ると、記録に発火位置が無いので最後まで発火しない。(c) `TimerFired` の時刻は `elapsed_ms`(ms 単位、別の時計)なので、仮想時計との突き合わせには使えない。
4. **フィールド不足**(BUG-105 では効かなかったもの):
   - InputContext の 4 項目(ime_on・input_mode・japanese・composing): 固定値(IME ON・ローマ字・日本語 IME・未変換)で与えた。`state_before` は FSM の状態ラベルなので、ここからは導けない。IME OFF の区間や変換中の区間を含む報告では必要になる。
   - `win` 修飾・拡張ビット: 使わなかった(`win` は常に false)。
   - `ImeRelevance::sync_direction`: エンジンが読む唯一の `ImeRelevance` の項目。記録に無く、設定(`keys.ime_detect`)から殻が決める。既定値(なし)にした。
   - オートリピートの間の時刻: 畳み込みで残らないので、最初と最後の時刻の間を等間隔に広げた。
5. **エンジンの組み立て**: 本番の組み立て(`app/bootstrap.rs`、Windows 専用)のうち、同時打鍵に効く部分(配列・親指キー・閾値・`confirm_mode`・`speculative_delay_ms`・タイミングマージン・n-gram)だけを写した。`keys.*`(特殊キー)・親指の単独タップ設定・Space/Enter の親指・`forced_open_actions` は写していないので、これらが絡む不具合は再生できない。n-gram は `config.toml` の `ngram_file`(リポジトリ同梱の `data/ngram_hiragana.csv.gz`)を読んだ。報告に n-gram のファイルが入るかは未確認のまま。
6. **エンジンの ON/OFF(③)**: `set_prev_active(true)` と InputContext の固定値で足りた。

## fixture について

- 入力列は BUG-105 の既存テストのもの(L・右親指・A の 4 打鍵)。実ユーザーの入力文は使っていない。
- 時刻は報告の `[engine-input]` の行(`docs/known-bugs/BUG-105.md` に既に載っている)と同じ値にした。既存テストの相対時刻(0・11.7ms・106.55ms・112.474ms)とも一致する。
- `state_before`/`state_after` は報告時(修正前)の記録を模した。L↑ と A↓ の `state_before` は BUG-105.md のログの値そのまま。A↓ の `state_after`(`Idle`)は BUG-105.md の説明(char1 単独+char2+親指を 1 回で出す)からの推定。
- `decision`・`physical` は形式を合わせるための値で、再生は読まない(`effect_count` は推定値)。

## 本格化の見積もり

- 補助(約 390 行)は BUG-145 にもそのまま使える見込み。BUG-145 は押下間隔の再現(実機ログの 90.1〜108.8ms)で、①②④の範囲に収まる。
- 本格化で増えるもの:
  - エンジンの組み立ての共有: `bootstrap.rs` の組み立て(約 80 行)を純粋な関数に切り出して本番と再生で共有しないと、写した部分と本番がずれていく。移動が主で、純増は小さい見込み。
  - タイマーの方式: 記録の並びの方式で足りるかを BUG-145 で確かめる。足りない場合は、仮想時計に WM_TIMER の遅れ(分解能 約 15.6ms)を足す案があるが、遅れの値は実測が要る([tuning-constants](../../../.claude/rules/tuning-constants.md) と同じ扱い)。
  - 記録側(段階 4 の記録の部分): IME ON 中の打鍵の不具合(BUG-105・145 型)には不要。IME OFF・変換中を含む報告を再生するときに必要。

## 推奨

1. **記録側の InputContext 4 項目の追加は急がない**。BUG-105 は記録側を変えずに再生できた。必要になるのは IME OFF・変換中の区間を含む報告から。
2. **次は BUG-145 で 2 件目を試す**。特に、記録の `TimerFired` の並びの方式で足りるか(仮想時計では BUG-105 が検出できなかったので、ここが本格化の可否を決める)。
3. 記録側で 1 つ検討する価値があるのは、**`TimerFired` を `KeyInput` レーンに移す(または同じ容量にする)こと**。いまは `TimerFired` が先に追い出され、古い区間のタイマーの並びが分からない。フィールドの追加ではなくレーンの割り当ての変更で済む。
4. 本格化するなら、先に `bootstrap.rs` のエンジンの組み立てを純粋な関数に切り出す。

## 既存の actuation の再生(`replay_record`)との比較と、置き換えの見込み

所有者の判断(2026-10-06): 凍結コーパスと古い journal は、新しいリプレイ基盤を作って置き換えるのと同時に捨てる。B5 の試作はその基盤の候補になる。以下はその前提での整理(試作では記録側に触れていない)。

既存の再生一式: `state/actuation_decision_record.rs` の `#[cfg(test)] mod tests`(408 行目以降、約 880 行、テスト 15 本)、コーパス `tests/journals/actuation_decision/bug-131-report-01m29kdnz.json`(1440 行、37 レコード)、読み込みに `awase-replay::replay_dir`(この crate は `journal_replay.rs` など他の 3 本の再生テストも使う)。15 本のうちコーパスを読むのは `replay_all_actuation_decision_fixtures` の 1 本だけで、残りは手で組んだレコードの単体テスト。

### (1) 何を検証でき、何を検証できないか

| | B5(入力の再生、この試作) | `replay_record`(決定の再計算) |
| --- | --- | --- |
| 入力 | `KeyInput` の列(打鍵と時刻) | `ActuationDecision` の記録(`DecisionInputs`: profile・IME の種類・`shadow_on`・belief の入力モード、記録された `attempts`) |
| 動かすコード | `Engine::on_input`/`on_timeout`(同時打鍵の判定、出力、タイマー、エンジンが出す `SetOpen` の要求) | `decide_gate`/`decide_chain`/`decide_attempt` と `run_chain`/`run_chain_async`(どの機構を試し、どこで打ち切ったか) |
| 答えられる問い | エンジン側の不具合が HEAD で出るか(BUG-105 で確認) | 記録された入力に対して、HEAD の決定関数が記録と同じ決定を返すか(リファクタの回帰網) |
| 答えられない問い | IME 制御の送信の決定。エンジンは `SetOpen { open, press }` を返すところまでで、送るかどうか・何を送るかは殻(`runtime/executor.rs::dispatch_ime_set_open`、`ImeStateHub` の belief・applied・押下台帳)と、IME からの観測で決まる。これらは `KeyInput` に無く、殻のコードは `cfg(windows)` | 打鍵からその actuation に至ったか(打鍵や belief の経緯は入力に無い)。IME が実際にどう応えたか(outcome は記録をそのまま返すだけ) |

つまり両者は重ならない。B5 は actuation の決定を再生できず、`replay_record` はエンジンの判定を再生できない。IME とのやり取りが絡む不具合には、どちらも HEAD での再現に答えられない(README の結論と同じ)。

### (2) B5 の本格化で既存の再生一式を置き換えられるか

- **B5 だけでは置き換えられない**。actuation の決定の入力(`DecisionInputs`)は打鍵からは導けない。
- ただし `ActuationDecision` の記録は本番の journal に既にあり(`JournalEntry::ActuationDecision { record }`、所有者判断で残す)、コーパスの形式は同じ `ActuationDecisionRecord` の配列。したがって、**journal のダンプ 1 本から `KeyInput` は B5 へ、`ActuationDecision` は `replay_record` へ流す読み込み**にすれば、コーパスを新しい形式のダンプ(新しい報告か CI のダンプ)に置き換えられる。記録側に足すフィールドは要らない。
- 置き換えで消せるのはコーパス(1440 行)と、コーパス専用の読み込み(`replay_all_actuation_decision_fixtures`、約 15 行)くらい。`replay_record`・`ReplayWriter` と手組みの単体テスト(約 860 行)は、決定関数の回帰網として残ることになる(消すなら、同じことを全数表・単体テストで持つ必要がある)。
- 打鍵から actuation まで通して再生するには、殻の `SetOpen` の処理と `ImeStateHub` の遷移を Linux で動かし、IME の観測の列(State レーンの `ImeEvent`)を入力にする必要がある。これは B6(閉ループ)の範囲で、殻の該当部分が `cfg(windows)` の外に出るまでは作れない。記録側では、`SetOpen` の要求と `ActuationDecision` の記録を結ぶ印(押下 ID か seq の対応)も要る。

### (3) 新基盤の範囲の選択肢と行数の見込み

| 案 | 中身 | 行数の見込み | 既存の再生一式 |
| --- | --- | --- | --- |
| A. エンジン入力の再生だけ | B5 を本格化(エンジンの組み立ての共有を含む) | 補助 約 390 行(試作のまま)+ `bootstrap.rs` からの切り出し(移動、純増は小) | コーパスとその 1 本を消す(-1440 行のデータ、約 -15 行)。決定関数の回帰網は手組みの 14 本だけで持つ |
| B. 同じダンプから両方を再生(推奨) | A に、ダンプの封筒から `ActuationDecision` を取り出して `replay_record` に渡す読み込みを足す | A + 約 30〜50 行 | コーパスを新形式のダンプ 1 本に置き換える。`replay_record` 以下は残す |
| C. 打鍵から actuation の決定まで通す | 殻の `SetOpen` 処理・`ImeStateHub` の遷移・IME の観測の列まで再生 | 未実測。殻のコードの移動を含めて 1000 行を超える見込み | 置き換えられるが、B6 と同じ規模 |

推奨は B。A との差は小さく(数十行)、FCIS で決定関数が純粋になったので、本番で記録している `DecisionInputs` から決定を再計算する価値はそのまま残る。新形式のダンプ 1 本で両方の fixture を兼ねられる。C は IME とのやり取りを写す必要があり、B6 の判断(E1)と一緒に決める。
