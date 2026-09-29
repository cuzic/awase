import json
import os
import tempfile
import unittest

import check_reopen as cr


def ts_log(recs):
    return "\n".join("[12:00:00.000Z] [TS-JSON] " + json.dumps(r, ensure_ascii=False) for r in recs) + "\n"


def base(n, on="12:00:10.000", typed="12:00:11.500", pre_ok=True, typed_text="か"):
    return [
        {"type": "reopen_pre", "n": n, "utc": "12:00:08.000", "text": "か" if pre_ok else "ka", "expect": "か", "ok": pre_ok},
        {"type": "reopen_on", "n": n, "off_utc": "12:00:09.400", "on_utc": on, "gap_ms": 600},
        {"type": "reopen_typed", "n": n, "utc": typed, "on_utc": on, "press_utc": "12:00:10.010", "focus_lost": False,
         "text": typed_text, "expect": "か", "ok": typed_text == "か"},
    ]


HEAD = [{"type": "config", "form": "tsf", "ime": "gji", "mode": "reopen"}]
DONE = [{"type": "done"}]

CLEAN_AWASE = (
    '2026-09-29T12:00:10.005000Z DEBUG awase::journal: gji fsm transition seq=1 trigger="Reopen(BeliefSync:on-key)(gji_idle_ms=600)" state_before="OffCold" state_after="OnCold(Short)"\n'
    '2026-09-29T12:00:10.052000Z DEBUG awase_windows::output::vk_send: [vk-send] romaji="ka" warm=false elapsed=0ms session_expired=false prepend_f2_warmup=true\n'
)


def run(recs, awase):
    with tempfile.TemporaryDirectory() as d:
        p = os.path.join(d, "ts.log")
        a = os.path.join(d, "awase.log")
        open(p, "w", encoding="utf-8").write(ts_log(recs))
        open(a, "w", encoding="utf-8").write(awase)
        return cr.analyze(cr.parse(p), cr.load_awase(a))


class CheckReopen(unittest.TestCase):
    def test_pass_and_delay(self):
        r = run(HEAD + base(0) + DONE, CLEAN_AWASE)
        self.assertEqual(r["verdict"], "PASS")
        self.assertEqual(r["trials"][0]["first_vk_delay_ms"], 52)
        self.assertTrue(r["trials"][0]["first_vk_cold"])
        self.assertEqual(r["trials"][0]["reopen_sync"], 1)

    def test_stuck_offcold_fails(self):
        awase = CLEAN_AWASE + "2026-09-29T12:00:10.900000Z  WARN gji_on_event{event=StartComposition}: awase_windows::tsf::gji_fsm: [gji-fsm] StartComposition while engine off — ignored\n"
        r = run(HEAD + base(0) + DONE, awase)
        self.assertEqual(r["verdict"], "FAIL")
        self.assertIn("固着", r["trials"][0]["why"])

    def test_stale_confirm_escape_fails(self):
        awase = CLEAN_AWASE + "2026-09-29T12:00:10.700000Z  WARN awase_windows::tsf::warmup::probe_fsm: StaleConfirm idx=1 escape=true\n"
        r = run(HEAD + base(0) + DONE, awase)
        self.assertEqual(r["verdict"], "FAIL")

    def test_lost_first_char_fails(self):
        r = run(HEAD + base(0, typed_text="") + DONE, CLEAN_AWASE)
        self.assertEqual(r["verdict"], "FAIL")

    def test_events_outside_window_are_ignored(self):
        awase = "2026-09-29T12:00:05.000000Z  WARN gji_on_event: [gji-fsm] StartComposition while engine off — ignored\n" + CLEAN_AWASE
        self.assertEqual(run(HEAD + base(0) + DONE, awase)["verdict"], "PASS")

    def test_pre_word_not_ok_is_invalid(self):
        r = run(HEAD + base(0, pre_ok=False) + DONE, CLEAN_AWASE)
        self.assertEqual(r["verdict"], "INVALID")

    def test_abort_and_missing_done_are_invalid(self):
        self.assertEqual(run(HEAD + base(0), CLEAN_AWASE)["verdict"], "INVALID")
        self.assertEqual(run(HEAD + [{"type": "abort", "reason": "x"}], CLEAN_AWASE)["verdict"], "INVALID")

    def test_exit_codes_via_main(self):
        with tempfile.TemporaryDirectory() as d:
            p = os.path.join(d, "ts.log")
            a = os.path.join(d, "awase.log")
            j = os.path.join(d, "o.json")
            open(p, "w", encoding="utf-8").write(ts_log(HEAD + base(0) + DONE))
            open(a, "w", encoding="utf-8").write(CLEAN_AWASE)
            import sys
            old = sys.argv
            sys.argv = ["check_reopen.py", "--json", j, p, a]
            try:
                self.assertEqual(cr.main(), 0)
            finally:
                sys.argv = old
            self.assertEqual(json.load(open(j, encoding="utf-8"))["verdict"], "PASS")


if __name__ == "__main__":
    unittest.main()
