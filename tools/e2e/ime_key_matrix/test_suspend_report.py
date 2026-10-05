#!/usr/bin/env python3
"""suspend_report.py の単体テスト。実行: python3 -m unittest discover -s tools/e2e/ime_key_matrix -p 'test_*.py'"""
import unittest

import suspend_report as sr


def rec(matched, s="12:00:01.000", r="12:00:01.800"):
    return {"type": "suspend", "matched": matched, "suspended_utc": s, "resumed_utc": r}


OK = [{"name": "GoogleIMEJaConverter.exe", "pid": 1, "suspend_status": 0, "resume_status": 0}]
FAILED = [{"name": "GoogleIMEJaConverter.exe", "pid": 1, "suspend_status": -1073741790, "resume_status": 0}]


def awase(t, text):
    return (t, f"2026-10-05T{t}Z DEBUG {text}")


class Analyze(unittest.TestCase):
    def test_no_suspend_records(self):
        self.assertEqual(sr.analyze([{"type": "trial"}], [])["verdict"], "NO_SUSPEND")

    def test_effective_counts_delay_and_stale_inside_window_only(self):
        lines = [
            awase("12:00:00.500", "[engine-input] vk=0x1E KeyDown ts=1us delay=900ms state=Idle"),  # 窓の外
            awase("12:00:01.200", "[engine-input] vk=0x1E KeyDown ts=1us delay=640ms state=Idle"),
            awase("12:00:02.000", "per-VK[1/2] stale confirm 検出 escape=true"),
            awase("12:00:02.100", "per-VK[0/2] stale confirm 検出 escape=true"),
            awase("12:00:09.000", "per-VK[1/2] stale confirm 検出 escape=true"),  # 窓の外
        ]
        r = sr.analyze([rec(OK)], lines)
        self.assertEqual((r["verdict"], r["delay_max_ms"], r["stale"], r["stale_escape_idx1"]), ("EFFECTIVE", 640, 2, 1))

    def test_not_effective_when_status_nonzero(self):
        self.assertEqual(sr.analyze([rec(FAILED)], [])["verdict"], "NOT_EFFECTIVE")

    def test_no_match_makes_run_unusable(self):
        r = sr.analyze([rec(OK), rec([])], [])
        self.assertEqual((r["verdict"], r["no_match"]), ("NOT_EFFECTIVE", 1))


if __name__ == "__main__":
    unittest.main()
