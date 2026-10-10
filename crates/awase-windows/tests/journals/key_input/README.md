# key_input

`bug-105-tight-d1.json` は**合成**の記録。打鍵(L・右親指・A)と時刻は BUG-105 の既存テスト(`tests/scenarios.rs::scenario_3key_char1_released_tight_overlap_prefers_chord`)から作り、`state_*`・`decision`・`physical`・`seq`・`elapsed_ms` は推定で埋めた。形式は本物のダンプ(`JournalEnvelope` の配列)と同じなので、本物の報告のダンプと取り違えないこと。

`bug-197-foreign-ctrl-paste-01.json`・`-02.json` は**実記録**(合成ではない)。報告 `01M4J72G985T0FFT6XPN0SWCQQ`(2026-10-10 受信、R2 から取得)の journal の `seq 61-64`・`seq 117-120` から、各打鍵の `{t_us, vk, scan, event_type, injected, alt, shift, ctrl}` を**記録のまま**抜き出した(ADR-251)。`ctrl` は全て false で、記録時(ADR-249 の修正前、`foreign_ctrl` の欄が無かった)の値。修正後の `ctrl=true` はファイルに書かず、`src/key_input_replay_tests.rs::foreign_ctrl_journal` が記録の時刻順に `ForeignCtrlLatch`(`FOREIGN_CTRL_TTL_MS`)へ流して求める。対照のテストはラッチを通さず記録のまま流し、修正前の挙動(`PendingChar`・「ふ」)を再現する。形式は上の `JournalEnvelope` ではなく最小形。
