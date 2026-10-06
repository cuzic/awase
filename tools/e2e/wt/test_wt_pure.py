"""wt_pure.py の単体テスト。実行: python3 -m unittest discover -s tools/e2e/wt -p 'test_*.py'"""
import unittest

import wt_pure as P

LOG = "﻿READY\r\n120\t97\t65\t0\r\n250\t12354\t0\t0\r\n400\t64\t0\t0\r\n500\t13\t13\t0\r\nbroken\r\n600\t107\t75\t0\r\n"


class Echo(unittest.TestCase):
    def test_parse(self):
        ready, rows = P.parse_echo(LOG)
        self.assertTrue(ready)
        self.assertEqual(len(rows), 5)
        self.assertEqual(rows[1], (250, 12354, 0, "0"))

    def test_not_ready(self):
        self.assertEqual(P.parse_echo("")[0], False)

    def test_classify(self):
        c = P.classify(P.parse_echo(LOG)[1])
        self.assertEqual(c["kana"], 1)   # あ
        self.assertEqual(c["at"], 1)
        self.assertEqual(c["enter"], 1)
        self.assertEqual(c["ascii_alpha"], 2)  # a, k
        self.assertEqual(c["total"], 5)

    def test_cjk_and_other(self):
        c = P.classify([(0, 0x6F22, 0, ""), (1, 49, 0, ""), (2, 0, 0, "")])
        self.assertEqual((c["cjk"], c["other"]), (1, 2))

    def test_text_of_escapes_controls(self):
        self.assertEqual(P.text_of([(0, 97, 0, ""), (1, 13, 0, ""), (2, 12354, 0, "")]), "a<0D>あ")

    def test_rows_between(self):
        rows = P.parse_echo(LOG)[1]
        self.assertEqual([r[0] for r in P.rows_between(rows, 200, 450)], [250, 400])


def _l(sec, msg):
    return f"2026-10-06T10:00:{sec:06.3f}Z DEBUG x: {msg}"


SCOPE = _l(1, "[focus-scope] bootstrap initial scope: to=HwndId(1) profile=TsfNative focus_epoch=1")


class Bug114(unittest.TestCase):
    def drift(self, t0, n, step=0.4):
        return [_l(t0 + i * step, "[drift] correction: observed=true ≠ desired=false") for i in range(n)]

    def test_bounded_burst_passes(self):
        r = P.judge_bug114([SCOPE] + self.drift(5, 5) + [_l(7.5, "[drift] actuation gave up (Blind): x")])
        self.assertEqual((r["verdict"], r["counts"]["bursts"], r["counts"]["gave_up"]), ("PASS", [5], 1))

    def test_no_drift_is_invalid(self):
        r = P.judge_bug114([SCOPE])
        self.assertEqual(r["verdict"], "INVALID")
        self.assertIn("0 件", r["invalid"][0])

    def test_unbounded_burst_fails(self):
        self.assertEqual(P.judge_bug114([SCOPE] + self.drift(5, 12))["verdict"], "FAIL")

    def test_read_policy_fails(self):
        extra = [_l(5.1, 'origin=EventOrigin { source: SelfActuated { strategy: "drift_correction_read" } }')]
        self.assertEqual(P.judge_bug114([SCOPE] + self.drift(5, 2) + extra)["verdict"], "FAIL")

    def test_other_profile_fails(self):
        scope = SCOPE.replace("TsfNative", "ImmCross")
        self.assertEqual(P.judge_bug114([scope] + self.drift(5, 1))["verdict"], "FAIL")

    def test_repeated_rearm_fails(self):
        extra = [_l(10 + i * 4, "[drift] fresh observation after give-up x") for i in range(3)]
        self.assertEqual(P.judge_bug114([SCOPE] + self.drift(5, 1) + extra)["verdict"], "FAIL")

    def test_missing_scope_line_is_invalid(self):
        self.assertEqual(P.judge_bug114(self.drift(5, 1))["verdict"], "INVALID")

    def test_bursts_split_by_gap(self):
        self.assertEqual(P.bursts([1.0, 1.4, 1.8, 5.0, 5.4]), [3, 2])


if __name__ == "__main__":
    unittest.main()
