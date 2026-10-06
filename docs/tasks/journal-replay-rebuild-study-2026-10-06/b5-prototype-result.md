# B5(入力の再生)の試作結果: BUG-105 の 1 件

2026-10-06。[README.md](README.md) の段階 4(B5 の最小形)のうち、**再生側だけ**を、記録の形式を変えずに BUG-105 の 1 件で試した。記録側(`KeyInput` に InputContext の 4 項目・拡張ビット・`win` 修飾を足すこと)には触れていない。この試作は、所有者の方針(新しいリプレイ基盤を作り、既存の再生と置き換えるのと同時に凍結コーパスを捨てる)の基盤の候補でもある。

## 結論

- **既存テストと同じ結果になった**。fixture(`tests/scenarios.rs::scenario_3key_char1_released_tight_overlap_prefers_chord` と同じ打鍵・時刻を `KeyInput` の列に戻した**合成**の記録)を HEAD のエンジンに流すと、そのテストと同じ `lyou` になる。A↓ の時点で `lyo`(L+右親指=「ょ」)が出て、A はタイムアウトで `u` になる。修正前を模した記録と最初に食い違うのは A↓ の後の状態(HEAD は `PendingChar(vk=0x41)`、記録は `Idle`)。
- 検出力は既存の `scenarios.rs` のテストと同じで、増えてはいない(レビューで、BUG-105 の修正を外すと出力が `ivu` になって落ちることを確認)。入力も期待値も同じなので、このテストは `scenarios.rs` のテストと重複する。**試作として置くもので、本格化か BUG-145 の 2 件目のときに、置き換えるか消す**。
- **記録側を変えずに再生できた**(停止条件には当たらない)。ただし BUG-105 は IME ON のまま打鍵し、修飾キーも特殊キーも使わない例なので、足りないフィールドが効かなかっただけ(下記「詰まった点」4)。
- **README の段階 4 ② を覆した**。README は「記録の `TimerFired` は時刻の系が違うので使わない、HEAD のタイマーを `timestamp_us` の仮想時計で回す」としていた。しかし BUG-105 では、遅れの無い仮想時計だとタイマーが A↓ より先に発火し、修正の前でも後でも同じ `lyou` になる(BUG-105 を検出できない)。理由は時刻の系ではなく、**A↓ とタイマーの競争**(下記 3)。README には訂正の 1 行を足した。
- 試したタイマーの扱いは「途中では一切発火しない」だけ。fixture の区間には `TimerFired` が 1 件も無い(実機で発火しなかったため)ので、記録の `TimerFired` の並び(seq)で発火する分岐は通っていない。「記録の並びで発火すれば競争に依存する不具合を再生できる」は、BUG-105 では退化した形(タイマーが無限に遅れる場合)でしか示せていない。

成果物:
- 補助とテスト: `crates/awase-windows/src/key_input_replay_tests.rs`(`#[cfg(test)]`、`cfg(windows)` の外)
- fixture: `crates/awase-windows/tests/journals/key_input/bug-105-tight-d1.json`(`KeyInput` 4 件、journal のダンプと同じ形式。合成であることを同じディレクトリの README.md に書いた)
- CI: test ジョブ(`cargo nextest run --workspace --lib`)で新しいテスト 2 本が PASS(`bug_105_replay_in_recorded_timer_order_matches_engine_test`、`bug_105_replay_on_virtual_clock_times_out_before_char2`)。

## 行数(rustfmt 後)

| 部分 | 行数 |
| --- | --- |
| 冒頭の doc・`use` | 約 42 |
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
3. **タイマーとの競争(最大の詰まり)**: 右親指↓で `TIMER_PENDING` が張り直され(100ms)、期限は右親指↓+100ms = 111.7ms になる。L↑ では張り直さない。A↓ は 112.474ms で、期限より 0.774ms 遅い。
   - **エンジンの仕様**: d2=100.8ms はエンジン自身の閾値(100ms)の外にある。それでも 3 鍵の仲裁(`compute_prefer_char1`、BUG-105 の本体)に入るのは、A↓ が `TIMER_PENDING` の満了より先に処理されたときだけ。char2 の到着側で閾値を見直す処理は無い(BUG-105.md の「残課題」の、char2 優先分岐の `is_simultaneous` 欠如そのもの)。
   - **OS のタイマーの性質**: 実機の `SetTimer` は、イベントの時刻ではなく処理した時刻から数え始め、満了は tick(約 15.6ms)単位に切り上がる。`WM_TIMER` は優先度が最も低く、入力メッセージより後に配られる。遅れの無い仮想時計は実機より必ず早く発火する。実機では A↓ が先に処理された(報告のログの `Timer killed: logical=1`)。
   - つまり BUG-105 は、エンジンのタイマーと OS のメッセージの競争で起きた。仮想時計は「エンジンが想定する理想のタイミング」を再生し、その下では修正の前も後も正しい `lyou` になる(テスト `bug_105_replay_on_virtual_clock_times_out_before_char2` で固定)。
   - 競争に依存する不具合を決まった結果で再生するには、**実際に観測した順序**(どのタイマーがどの入力の前に配られたか)が要る。記録の `TimerFired` の並び(seq)で発火する方式はこのためで、タイマー自体は HEAD が張り・消したものを使い、発火の位置だけを記録から借りる。B5 の趣旨(HEAD のエンジンが返すタイマーを使う)とは矛盾しない。README の「時刻の系が違う」は、`TimerFired` の時刻(`elapsed_ms`、別の時計)ではなく並びを使えば避けられる。
   - この方式の限界: (a) `TimerFired` は Actuation レーン(6144 件、`SentInput` と同居)にあり、`KeyInput` レーン(8192 件)より先に古いものが追い出される。追い出された区間ではタイマーの並びが分からない。(b) HEAD が記録時と違うタイマーを張ると、記録に発火位置が無いので最後まで発火しない。(c) 試作の実装は、記録の位置で発火するとき HEAD の期限を見ない(`now = max(now, 期限)`)。HEAD が期限を伸ばした場合などに、期限より早い・ありえない順序で発火し、`now` が次の打鍵の時刻を追い越して単調でなくなる。本格化では、期限が次の打鍵より後なら発火を遅らせるか食い違いとして報告し、`now` の単調性を assert する。記録の `TimerFired` の `state_before`/`state_after` も比べる(試作は比べていない)。
   - 別の選択肢: 残課題(char2 優先分岐の `is_simultaneous`)をエンジンで直すと、A↓ とタイマーのどちらが先でも同じ結果になる場面が増え、再生がタイマーの扱いに左右されにくくなる。
4. **フィールド不足**(BUG-105 では効かなかったもの):
   - InputContext の 4 項目(ime_on・input_mode・japanese・composing): 固定値(IME ON・ローマ字・日本語 IME・未変換)で与えた。`state_before` は FSM の状態ラベルなので、ここからは導けない。IME OFF の区間や変換中の区間を含む報告では必要になる。
   - `win` 修飾・拡張ビット: 使わなかった(`win` は常に false)。
   - `ImeRelevance::sync_direction`: エンジンが読む唯一の `ImeRelevance` の項目。記録に無く、設定(`keys.ime_detect`)から殻が決める。既定値(なし)にした。
   - オートリピートの間の時刻: 畳み込みで残らないので、最初と最後の時刻の間を等間隔に広げた。
5. **エンジンの組み立て**: 本番の組み立ては `app/bootstrap.rs`(Windows 専用)の `:279`(`NicolaFsm::new`)と `:1174` 以降に分かれていて、`runtime::thumb_forced_open_actions`・`runtime::migrate_legacy_solo_tap_actions`(`cfg(windows)` の `runtime/` の中)、`keys.*`、keymap、Space/Enter の親指、無変換・変換の単独タップ、`set_backspace_vk` に依存する。試作はこのうち同時打鍵の判定に効く部分(配列・親指キー・閾値・`confirm_mode`・`speculative_delay_ms`・タイミングマージン・n-gram)だけを写した。**`set_thumb_shift_faces_enabled` も呼んでいない**ので、試作のエンジンでは親指+小指シフトの複合面が既定の無効のまま(本番は `thumb_shift_faces_enabled_for` で有効)。写した部分と本番が既にずれている実例で、親指+Shift が絡む不具合は再生できない。n-gram は `config.toml` の `ngram_file`(同梱の `data/ngram_hiragana.csv.gz`)を読んだ。報告に n-gram のファイルが入るかは未確認のまま。
6. **エンジンの ON/OFF(③)**: `set_prev_active(true)` と InputContext の固定値で足りた。

## fixture について

- 打鍵(L・右親指・A)と相対時刻は `scenarios.rs` の BUG-105 のテストのもの。実ユーザーの入力文は使っていない。`src/engine/tests.rs::test_three_key_char1_released_tight_d1_still_prefers_char1` は合成レイアウトで A/変換/S を使うテストで、共通なのは時刻だけ(補助のモジュールの doc は 2 本を並べて出所として挙げているが、打鍵の出所は `scenarios.rs` の方)。
- 絶対時刻: L↑(142691057291)と A↓(142691063215)は BUG-105.md のログの値そのまま。L↓ と右親指↓ は、既存テストの相対時刻から逆算した値。
- `state_before`/`state_after` は報告時(修正前)の記録を模した。L↑ と A↓ の `state_before` は BUG-105.md のログの値。A↓ の `state_after`(`Idle`)は BUG-105.md の説明(char1 単独+char2+親指を 1 回で出す)からの推定。
- `decision`・`physical`・`seq`・`elapsed_ms` は形式を合わせるための推定値で、再生は読まない。

## 既存の actuation の再生(`replay_record`)との比較

既存の再生一式: `state/actuation_decision_record.rs` の `#[cfg(test)] mod tests`(`:408-1288`、約 880 行、`#[test]` 17 本)、凍結コーパス `tests/journals/actuation_decision/bug-131-report-01m29kdnz.json`(1440 行、37 レコード)、テスト専用の `Deserialize` 一式(約 70 行)。読み込みは `awase-replay::replay_dir`(この crate は `journal_replay.rs` など他の 3 本の再生テストも使う)。コーパスを読むテストは `replay_all_actuation_decision_fixtures` の 1 本だけで、残りは手で組んだレコードで `replay_record`/`replay_chain_scan` 自身を検証するもの。決定関数(`decide_gate`/`decide_chain`/`decide_attempt`)には別に単体テストと全数テストがある。数字は corpus-discard-impact のメモ(`docs/corpus-discard-impact` ブランチ)による。

| | B5(入力の再生、この試作) | `replay_record`(決定の再計算) |
| --- | --- | --- |
| 入力 | `KeyInput` の列(打鍵と時刻) | `ActuationDecision` の記録(`DecisionInputs`: profile・IME の種類・`shadow_on`・belief の入力モード、記録された `attempts`) |
| 動かすコード | `Engine::on_input`/`on_timeout`(同時打鍵の判定、出力、タイマー、エンジンが出す `SetOpen` の要求) | `decide_gate`/`decide_chain`/`decide_attempt` と `run_chain`/`run_chain_async`(どの機構を試し、どこで打ち切ったか) |
| 答えられる問い | エンジン側の不具合が HEAD で出るか(BUG-105 で既存テストと同じ結果) | 記録された入力に対して、HEAD の決定関数と走査が記録と同じ決定を返すか |
| 答えられない問い | IME 制御の送信の決定。エンジンは `SetOpen { open, press }` を返すところまでで、送るかどうか・何を送るかは殻(`runtime/executor.rs::dispatch_ime_set_open`、`ImeStateHub` の belief・applied・押下台帳)と IME からの観測で決まる。これらは `KeyInput` に無く、殻のコードは `cfg(windows)` | 打鍵からその actuation に至ったか(打鍵や belief の経緯は入力に無い)。IME が実際にどう応えたか(outcome は記録をそのまま返すだけ) |

両者は重ならない。B5 は actuation の決定を再生できず、`replay_record` はエンジンの判定を再生できない。IME とのやり取りが絡む不具合には、どちらも HEAD での再現に答えられない(README の結論と同じ)。**B5 だけでは既存の再生一式を「置き換える」ことはできない**。置き換えの形を取るには、actuation の再生を諦めるか(案 B″)、打鍵から actuation まで通すか(案 C)のどちらかになる。

## 新基盤の範囲の選択肢(所有者が選ぶ)

共通: B5 の補助 約 +390 行、テスト・fixture は再生したい不具合ごとに足す。コーパスは git 履歴から取り出せる(`d703c5d0`)。

| 案 | 中身 | 追加 | 撤去 | 失う検証 | 必要なダンプの入手元 |
| --- | --- | --- | --- | --- | --- |
| B′ B5 を足し、既存の再生一式は当面残す | 置き換えない。凍結コーパスもそのまま | B5 の補助 約 390 行 | なし | なし | `KeyInput`: 報告の journal。actuation 側は今のコーパスのまま |
| B 同じダンプから両方を再生(前の版の推奨) | ダンプの封筒から `ActuationDecision` を取り出して `replay_record` に渡す読み込みを足し、コーパスを新しいダンプに差し替える | B5 約 390 行 + 読み込み 約 30〜50 行 | コーパス 1440 行(データ)、コーパス専用のテスト 約 15 行。`replay_record`・RW・手組みのテスト・`Deserialize` 一式(約 860+70 行)は残る | BUG-131 の 37 件(git 履歴には残る) | `ActuationDecision` は本番の journal に毎回載るので、新しい報告の journal には入る。ただし BUG-131 型(非同期のチェーンの打ち切り)のレコードを含む報告がいつ来るかは分からない。実機 CI の JSON ダンプは今はしない(E5)ので、CI からは得られない。差し替える fixture が無い期間が生じうる |
| B″ B5 に統一し、actuation の再生一式を消す | corpus-discard-impact の (B) と B5 を組み合わせる | B5 約 390 行 | コード 約 800 行(テスト 約 730、`Deserialize` 約 70)、データ 1440 行、ガイドの節 約 58 行。`#[test]` 17 本のうち 15 本 | 記録した入力からの決定の再計算と、記録した chain・attempt の本番の走査コードでの再走査(RW)。決定関数の単体テスト・全数テストは残る。`MechanismCommand` の `unreachable!` の 2 variant を消せないでいた理由(コーパスとの互換)も無くなる | `KeyInput`: 報告の journal(不具合ごとに人が数打鍵に縮める、E7) |
| C 打鍵から actuation の決定まで通す | B5 のエンジンの出力(`SetOpen`)を殻の処理と `ImeStateHub` に通し、IME の観測の列も入力にする | 未実測。構成要素: 殻の `dispatch_ime_set_open`(`runtime/executor.rs`、195 行、`cfg(windows)` で `WindowsPlatform` に依存)を外へ出す移動、`ImeStateHub`(`state/platform_state.rs`、Linux で動く)の駆動、State レーンの `ImeEvent` の列の読み込み、IME の応答の写し(閉ループの `tests/support/` は harness 714 行・pseudo_ime 509 行)。閉ループの規模から、1000 行を超える見込み | B″ と同じ(置き換えになる) | なし(IME の応答は写した範囲だけ) | 報告の journal(KeyInput・ActuationDecision・ImeEvent の各レーン)。B6 と同じ判断(E1)が要る |

本格化の共通の作業: エンジンの組み立ての共有。`bootstrap.rs` から同時打鍵に効く部分を純粋な関数に切り出すには、`runtime/` にある `thumb_forced_open_actions`・`migrate_legacy_solo_tap_actions` を Windows 専用でない場所へ出す作業を含む(移動だけでは済まず、`runtime/` の依存の整理が要る)。

## 推奨

1. **記録側の InputContext 4 項目の追加は急がない**。BUG-105 は記録側を変えずに再生できた。必要になるのは IME OFF・変換中の区間を含む報告から。
2. **新基盤の範囲は B′・B・B″ のどれかを所有者が選ぶ**。判断の軸は「actuation の決定の再計算(RW を含む)を回帰網として持ち続けるか」。持ち続けるなら B′(今のコーパスのまま)か B(新しいダンプが手に入ってから差し替える)。持たないなら B″ で、コードが約 800 行減る。B は「置き換えて捨てる」の方針に合うが、差し替え先のダンプが手元に無い。C は B6 と一緒に決める。
3. **次は BUG-145 で 2 件目を試す**。記録の `TimerFired` の並びの分岐を実際に通る fixture になるか、仮想時計で足りるかを確かめる(上記 3 のとおり、BUG-105 ではこの分岐を通っていない)。
4. 記録側で検討する価値があるのは、**`TimerFired` を `KeyInput` レーンに移す(または同じ容量にする)こと**。フィールドの追加ではなく、レーンの割り当ての変更で済む。
