#!/usr/bin/env python3
"""check_run_validity.py と stats_util.py の単体テスト。実行: python3 -m unittest discover -s tools/e2e/ime_key_matrix -p 'test_*.py'"""
import unittest

import check_run_validity as crv
import stats_util as su

L = "2026-10-05T01:02:03.456Z DEBUG [engine-input] vk=0x{vk} {ev} ts=1us delay=0ms state=Idle mods(c=false s=false a=false w=false) gas_ctrl=false phys_ctrl=false extra=0x{extra} pending_drain=0"


def line(ev="KeyDown", extra="5350494B", vk="1E"):
    return L.format(vk=vk, ev=ev, extra=extra)


class ForeignPhysical(unittest.TestCase):
    def test_marker_injection_is_not_foreign(self):
        self.assertEqual(crv.foreign_physical([line(extra="5350494B")])[:2], (0, 0))

    def test_extra_zero_down_and_up_counted_separately(self):
        d, u, s = crv.foreign_physical([line("KeyDown", "0"), line("KeyUp", "0"), line("KeyDown", "0")])
        self.assertEqual((d, u), (2, 1))
        self.assertEqual(len(s), 3)

    def test_unrelated_lines_ignored(self):
        self.assertEqual(crv.foreign_physical(["[hook] IME-mode vk=0xF2 down self_injected=true extra=0x0"])[:2], (0, 0))

    def test_samples_capped(self):
        self.assertEqual(len(crv.foreign_physical([line(extra="0")] * 20)[2]), 5)


class Cpu(unittest.TestCase):
    def test_empty(self):
        self.assertEqual(crv.cpu_stats([]), (0, None, None, None))

    def test_broken_rows_skipped(self):
        n, mean, p95, mx = crv.cpu_stats(["t,10", "garbage", "t,x", "t,30"])
        self.assertEqual((n, mean, mx), (2, 20.0, 30.0))
        self.assertEqual(p95, 30.0)

    def test_p95_index(self):
        rows = [f"t,{i}" for i in range(1, 101)]  # 1..100
        self.assertEqual(crv.cpu_stats(rows)[2], 96.0)


class Analyze(unittest.TestCase):
    def test_clean(self):
        r = crv.analyze([line()], ["t,20", "t,30"])
        self.assertEqual(r["verdict"], "CLEAN")

    def test_foreign_contaminates(self):
        r = crv.analyze([line(extra="0")], [])
        self.assertEqual(r["verdict"], "CONTAMINATED")
        self.assertIn("物理キー", r["reasons"][0])

    def test_cpu_contaminates_only_above_limit(self):
        hot = ["t,95"] * 10
        self.assertEqual(crv.analyze([], hot)["verdict"], "CONTAMINATED")
        self.assertEqual(crv.analyze([], hot, cpu_p95_limit=99)["verdict"], "CLEAN")

    def test_missing_cpu_is_not_contamination(self):
        self.assertEqual(crv.analyze([], [])["verdict"], "CLEAN")

    def test_key_up_alone_does_not_contaminate(self):
        # 起動前に押されていたキーの KeyUp だけが届くことがある。KeyDown が無ければ混入とは見なさない。
        self.assertEqual(crv.analyze([line("KeyUp", "0")], [])["verdict"], "CLEAN")


class Wilson(unittest.TestCase):
    def test_zero_of_ten_has_nonzero_upper_bound(self):
        lo, hi = su.wilson_interval(0, 10)
        self.assertEqual(lo, 0.0)
        self.assertAlmostEqual(hi, 0.2775, places=3)

    def test_all_of_ten(self):
        lo, hi = su.wilson_interval(10, 10)
        self.assertAlmostEqual(lo, 0.7225, places=3)
        self.assertEqual(hi, 1.0)

    def test_empty(self):
        self.assertEqual(su.wilson_interval(0, 0), (0.0, 1.0))
        self.assertEqual(su.format_rate(0, 0), "-")

    def test_format(self):
        self.assertEqual(su.format_rate(0, 10), "0/10 (0〜28%)")

    def test_clamps_k(self):
        self.assertEqual(su.wilson_interval(99, 10), su.wilson_interval(10, 10))


if __name__ == "__main__":
    unittest.main()
