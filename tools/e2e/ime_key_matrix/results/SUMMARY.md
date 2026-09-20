# ADR-186 撤去・統合の実験結果 (2026-09-19 17:58:37)

| 実験 | 内容 | 構成 | 3回の結果(PASS/FAIL件数) | 失敗の型(例) |
|---|---|---|---|---|
| E0h | 基準(変換キーで検証) | mut=none args=henkan-hold180 cfg=true |  ALL PASS / ALL PASS / ALL PASS |     |
| E1 | KeyUp解決を撤去(タイマー経路に戻す) | mut=a1-revert-keyup.sh args=hold180 cfg=true |  3 件 FAIL / 3 件 FAIL / 3 件 FAIL |  5:実IME期待 open=0 conv=0x19;   6 記録なし → FAIL; 5:実IME期待 open=0 conv=0x19;   6 記録なし → FAIL; 5:実IME期待 open |
| E2 | ATOKでcustom表を読まない修正を撤去(変換キー) | mut=a2-revert-atok-skip.sh args=henkan-hold180 cfg=true |  5 件 FAIL / 5 件 FAIL / 5 件 FAIL |  2:実IME期待 open=0 conv=0x10 / Engineがactivatedになった(OFFのままの期待);   3 記録なし → FAIL; 2:実IME期待 open=0 c |
| E3 | eisu reset抑止(ADR-186決定2)を撤去 | mut=a3-no-eisu-suppress.sh args=hold180 cfg=true |  ALL PASS / ALL PASS / ALL PASS |     |
| E4 | eisu resetの全経路を撤去 | mut=a4-no-eisu-reset.sh args=hold180 cfg=true |  ALL PASS / ALL PASS / ALL PASS |     |
| E5 | 物理キー後の20ms再読み取りを撤去 | mut=a5-no-refresh20.sh args=hold180 cfg=true |  2 件 FAIL / 2 件 FAIL / 2 件 FAIL |  7:300ms以内にdeactivatedしない;9:Engineがactivatedになった(OFFのままの期待); 7:300ms以内にdeactivatedしない;9:Engine |
| E6 | idle-conv-checkを無効化 | mut=a6-no-idle-check.sh args=hold180 cfg=true |  ALL PASS / ALL PASS / ALL PASS |     |
| E7 | gji_thumb_key_ime_toggle=false | mut=none args=hold180 cfg=false |  ALL PASS / ALL PASS / ALL PASS |     |
