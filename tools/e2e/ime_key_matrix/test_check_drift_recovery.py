#!/usr/bin/env python3
"""check_drift_recovery.py(物理 Ctrl+無変換の変種)の単体テスト。
実行: python3 -m unittest discover -s tools/e2e/ime_key_matrix -p 'test_*.py'
"""
import unittest

import check_drift_recovery as cd

GOOD = ("[engine-input] vk=0x1D KeyDown ts=1us delay=0ms state=X mods(c=true s=false a=false w=false) "
        "gas_ctrl=true phys_ctrl=true extra=0x5350494B\n")
BAD = GOOD.replace("phys_ctrl=true", "phys_ctrl=false")


def recs(close_open=False, last_open=False, ok=False, method="ctrl_muhenkan", n=1, done=True):
    out = [{"type": "config", "mode": "drift-on", "form": "tsf", "ime": "gji"}]
    for i in range(n):
        out += [
            {"type": "drift_on_pre", "n": i, "on_utc": "10:00:00.000", "on_key": "0x16", "real_ime_open": True},
            {"type": "drift_on_close", "n": i, "utc": "10:00:05.000", "method": method, "real_ime_open": close_open},
            {"type": "drift_on_check", "n": i, "checkpoint_ms": 500, "real_ime_open": close_open},
            {"type": "drift_on_check", "n": i, "checkpoint_ms": 2000, "real_ime_open": last_open},
            {"type": "drift_on_typed", "n": i, "press_utc": "10:00:08.000", "utc": "10:00:09.500",
             "text": "か" if ok else "ka", "ok": ok, "focus_lost": False},
        ]
    if done:
        out.append({"type": "done"})
    return out


def logs(muh=GOOD, extra=()):
    base = [("10:00:01.000", "T10:00:01.000 explicit_intent=Some(true)\n"), ("10:00:05.100", "T10:00:05.100 " + muh)]
    return base + list(extra)


class Ctrl(unittest.TestCase):
    def test_gap_not_made(self):
        r = cd.analyze(recs(close_open=False), logs())
        self.assertEqual(r["verdict"], "GAP_NOT_MADE")
        self.assertEqual(r["gap_not_made"], 1)

    def test_corrected(self):
        r = cd.analyze(recs(close_open=True, last_open=False), logs())
        self.assertEqual(r["verdict"], "CORRECTED")

    def test_not_corrected(self):
        r = cd.analyze(recs(close_open=True, last_open=True, ok=True), logs())
        self.assertEqual(r["verdict"], "NOT_CORRECTED")
        self.assertIn("method=ctrl_muhenkan gap_made=1 gap_not_made=0 phys_ctrl_ok=1", cd.summary_line(r))

    def test_phys_ctrl_false_is_invalid(self):
        r = cd.analyze(recs(close_open=True), logs(muh=BAD))
        self.assertEqual(r["verdict"], "INVALID")
        self.assertIn("phys_ctrl", r["trials"][0]["reason"])

    def test_no_muhenkan_log_is_invalid(self):
        r = cd.analyze(recs(close_open=True), logs(muh="[engine-input] vk=0x41 KeyDown\n"))
        self.assertEqual(r["verdict"], "INVALID")

    def test_missing_done_is_invalid(self):
        self.assertEqual(cd.analyze(recs(close_open=True, done=False), logs())["verdict"], "INVALID")

    def test_direct_close_still_uses_old_path(self):
        r = cd.analyze(recs(method="direct_close"), logs())
        self.assertFalse(r.get("ctrl"))


if __name__ == "__main__":
    unittest.main()
