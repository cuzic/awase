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
