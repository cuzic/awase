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
    m, sec = divmod(sec, 60)
    return f"2026-10-06T10:{int(m):02d}:{sec:06.3f}Z DEBUG x: {msg}"


T0 = 10 * 3600  # 10:00:00 の 0 時からの秒
SCOPE = _l(1, "[focus-scope] bootstrap initial scope: to=HwndId(1) profile=TsfNative focus_epoch=1")
BOOT = _l(1, "focus transition seq=1 elapsed_ms=64 changed_process=true changed_window=true")


class Bug114(unittest.TestCase):
    def drift(self, t0, n, step=0.03):
        return [_l(t0 + i * step, "[drift] correction: observed=true ≠ desired=false") for i in range(n)]

    def gave_up(self, t):
        return [_l(t, "[drift] actuation gave up (Blind): x")]

    def judge(self, lines, close=20.0, second=12.0, boot=True):
        return P.judge_bug114(([BOOT] if boot else []) + lines, T0 + close, T0 + second)

    def test_bounded_burst_with_gave_up_passes(self):
        r = self.judge([SCOPE] + self.drift(5, 5) + self.gave_up(5.2))
        self.assertEqual((r["verdict"], r["reproduced"], r["counts"]["bursts"]), ("PASS", False, [5]))

    def test_burst_of_six_fails(self):
        r = self.judge([SCOPE] + self.drift(5, 6) + self.gave_up(5.3))
        self.assertEqual((r["verdict"], r["reproduced"]), ("FAIL", True))

    def test_drift_without_gave_up_fails(self):
        r = self.judge([SCOPE] + self.drift(5, 3))
        self.assertEqual((r["verdict"], r["reproduced"]), ("FAIL", True))

    def test_read_policy_fails(self):
        extra = [_l(5.01, 'origin=EventOrigin { source: SelfActuated { strategy: "drift_correction_read" } }')]
        self.assertTrue(self.judge([SCOPE] + self.drift(5, 2) + extra + self.gave_up(5.1))["reproduced"])

    def test_one_rearm_fails(self):
        extra = [_l(12.5, "[drift] fresh observation after give-up x")]
        r = self.judge([SCOPE] + self.drift(5, 5) + self.gave_up(5.2) + extra)
        self.assertEqual((r["verdict"], r["reproduced"]), ("FAIL", True))

    def test_no_drift_is_invalid(self):
        r = self.judge([SCOPE])
        self.assertEqual((r["verdict"], r["reproduced"]), ("INVALID", False))

    def test_drift_after_close_is_not_counted(self):
        r = self.judge([SCOPE] + self.drift(25, 5) + self.gave_up(25.2))
        self.assertEqual(r["verdict"], "INVALID")

    def test_second_trigger_inside_cooldown_is_invalid(self):
        r = self.judge([SCOPE] + self.drift(5, 5) + self.gave_up(5.2), second=7.0)
        self.assertEqual(r["verdict"], "INVALID")

    def test_close_right_after_second_trigger_is_invalid(self):
        r = self.judge([SCOPE] + self.drift(5, 5) + self.gave_up(5.2), close=13.0)
        self.assertEqual(r["verdict"], "INVALID")

    def test_profile_mismatch_without_drift_is_not_reproduced(self):
        scope = SCOPE.replace("TsfNative", "ImmCross")
        r = self.judge([scope])
        self.assertEqual((r["verdict"], r["reproduced"]), ("FAIL", False))

    def test_missing_scope_line_is_invalid(self):
        self.assertEqual(self.judge(self.drift(5, 1) + self.gave_up(5.1))["verdict"], "INVALID")

    def test_second_process_change_is_invalid(self):
        extra = [_l(6, "focus transition seq=9 changed_process=true changed_window=true")]
        r = self.judge([SCOPE] + self.drift(5, 5) + self.gave_up(5.2) + extra)
        self.assertEqual(r["verdict"], "INVALID")

    def test_missing_boot_transition_is_invalid(self):
        self.assertEqual(self.judge([SCOPE] + self.drift(5, 5) + self.gave_up(5.2), boot=False)["verdict"], "INVALID")

    def test_bursts_split_by_gap(self):
        self.assertEqual(P.bursts([1.0, 1.4, 1.8, 5.0, 5.4]), [3, 2])

    def test_secs_of_day(self):
        self.assertAlmostEqual(P.secs_of_day(_l(61.5, "x")), T0 + 61.5)


if __name__ == "__main__":
    unittest.main()
