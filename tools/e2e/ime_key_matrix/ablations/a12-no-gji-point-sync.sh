#!/usr/bin/env bash
# A12: フォーカス時(A1 の presync_applied_open_on)と IME 種別同期(sync_ime_kind_from_observation)の GjiFsm への ImeOn 通知 2 か所を外す。
# ADR-203 決定 4 / ECRS DD5-01: (i) 送信直前の level 突合(gji_direct_mechanism の needs_belief_sync_on)が同じずれを拾うなら、この 2 か所は撤去できる。
# 外しても sc-reopen-* の --require-sync(ImeOn(BeliefSync:level))が通る=G2 が拾う、ことの確認に使う。
python3 - <<'PY'
p1 = 'crates/awase-windows/src/runtime/focus_tracking.rs'
s = open(p1, encoding='utf8').read()
old = """        if matches!(
            crate::tsf::observer::tsf_obs().active_ime_kind(),
            crate::tsf::observer::ActiveImeKind::GoogleJapaneseInput
        ) {
            let mode = self.platform.output.injection_mode;
            self.platform.gji_on_ime_on(mode);
            for entry in self.platform.drain_journal_entries() {
                self.platform_state.ime.journal.absorb(entry);
            }
        }
    }
"""
assert s.count(old) == 1, 'presync_applied_open_on の GjiFsm 通知が見つからない'
s = s.replace(old, "    }\n", 1)
open(p1, 'w', encoding='utf8').write(s)

p2 = 'crates/awase-windows/src/runtime/message_handlers.rs'
s = open(p2, encoding='utf8').read()
old = """    ) && app.platform_state.ime.model().applied.applied_open() == Some(true)
    {
        let mode = app.platform.output.injection_mode;
        tracing::debug!("[runtime] GJI warmup FSM sync: applied_open=true → ImeOn");
        app.platform.gji_on_ime_on(mode);
        for entry in app.platform.drain_journal_entries() {
            app.platform_state.ime.journal.absorb(entry);
        }
    }
"""
assert s.count(old) == 1, 'sync_ime_kind_from_observation の ImeOn が見つからない'
s = s.replace(old, "    ) && false\n    {}\n", 1)
open(p2, 'w', encoding='utf8').write(s)
PY
