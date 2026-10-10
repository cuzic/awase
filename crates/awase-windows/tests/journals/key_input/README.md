# key_input

`bug-105-tight-d1.json` は**合成**の記録。打鍵(L・右親指・A)と時刻は BUG-105 の既存テスト(`tests/scenarios.rs::scenario_3key_char1_released_tight_overlap_prefers_chord`)から作り、`state_*`・`decision`・`physical`・`seq`・`elapsed_ms` は推定で埋めた。形式は本物のダンプ(`JournalEnvelope` の配列)と同じなので、本物の報告のダンプと取り違えないこと。

`bug-197-foreign-ctrl-paste-01.json`・`-02.json` は**実記録**(合成ではない)。報告 `01M4J72G985T0FFT6XPN0SWCQQ`(2026-10-10 受信、R2 から取得)の journal の `seq 61-64`・`seq 117-120` から、各打鍵の `{t_us, vk, scan, event_type, injected, foreign_ctrl, ctrl}` だけを抜き出した(ADR-251)。時刻・vk・scan・injected は記録のまま。**V の `ctrl`/`foreign_ctrl` だけは記録と違う**: 記録時(ADR-249 の修正前)は注入 V が `ctrl=false` で届いていたので、修正後にフックが載せる `true` へ置き換えてある。対照のテストは、これを `false` に戻して修正前の挙動(`PendingChar`)を再現する。形式は上の `JournalEnvelope` ではなく最小形で、`src/key_input_replay_tests.rs::foreign_ctrl_journal` が読む。
