#!/usr/bin/env python3
"""check_keymatrix.py の単体テスト(ログ断片で各 verdict を確かめる)。
実行: python3 -m unittest discover -s tools/e2e/ime_key_matrix -p 'test_*.py'
"""
import json
import os
import tempfile
import unittest

import check_keymatrix as ck

CTRL_OK = ("[engine-input] vk=0x1D KeyDown ts=1us delay=0ms state=X mods(c=true s=false a=false w=false) "
           "gas_ctrl=true phys_ctrl=true extra=0x5350494B")
CTRL_BAD = CTRL_OK.replace("phys_ctrl=true", "phys_ctrl=false")


def press(i, state, typed=False):
    """1押下の記録。typed=True は実 Chrome 形式(typed_open)、False は自前窓形式(api2000)。"""
    if typed:
        return {"i": i, "typed_open": state, "api500": None, "api2000": None}
    return {"i": i, "api500": state, "api2000": state}


def trial(cell, states, kind, gap, r0, target, typed=False, pre_ok=True, pre_api="auto", focus_lost=False, n=0):
    key = cell.split("=")[0]
    return {"type": "km_trial", "cell": cell, "key": key, "kind": kind, "gap": gap, "n": n, "r0": r0, "target": target,
            "pre_ok": pre_ok, "pre_api": r0 if pre_api == "auto" else pre_api,
            "presses": [press(i + 1, s, typed) for i, s in enumerate(states)], "focus_lost": focus_lost}


def job(trials, form="tsf", ime="gji", done=True, evidence=None, abort=None):
    cfg = {"type": "config", "form": form, "ime": ime}
    if evidence:
        cfg["evidence"] = evidence
    out = [cfg] + trials
    if abort:
        out.append({"type": "abort", "reason": abort})
    if done:
        out.append({"type": "done"})
    return out


def cell_of(r, name):
    return next(c for c in r["cells"] if c["cell"] == name)


def abs_off(states, name="ctrl+1d=off:open", **kw):
    # OFF 絶対 × open: 押す前は開(r0=True)、目標は閉(False)。
    return trial(name, states, "off", "open", True, False, **kw)


class Verdicts(unittest.TestCase):
    def test_converged_1(self):
        r = ck.analyze(job([abs_off([False], n=i) for i in range(10)]))
        c = cell_of(r, "ctrl+1d=off:open")
        self.assertEqual(c["verdict"], "CONVERGED_1")
        self.assertTrue(c["pass"])
        self.assertTrue(c["meets_n"])
        self.assertEqual(r["verdict"], "PASS")

    def test_toggle_converged_2_passes(self):
        # トグル × close: 押す前は閉(r0=False)、目標は開。1押下目は変わらず(belief が古い)、2押下目で開く。
        ts = [trial("1d=tog:close", [False, True], "tog", "close", False, True, n=i) for i in range(10)]
        c = cell_of(ck.analyze(job(ts)), "1d=tog:close")
        self.assertEqual(c["verdict"], "CONVERGED_2")
        self.assertTrue(c["pass"])

    def test_absolute_converged_2_is_a_violation(self):
        ts = [trial("ctrl+1c=on:close", [False, True], "on", "close", False, True, n=i) for i in range(10)]
        r = ck.analyze(job(ts))
        c = cell_of(r, "ctrl+1c=on:close")
        self.assertEqual(c["verdict"], "CONVERGED_2")
        self.assertFalse(c["pass"])
        self.assertEqual(r["verdict"], "FAIL")

    def test_late_is_a_violation(self):
        ts = [trial("1d=tog:open", [True, True, False], "tog", "open", True, False, n=i) for i in range(10)]
        c = cell_of(ck.analyze(job(ts)), "1d=tog:open")
        self.assertEqual(c["verdict"], "CONVERGED_LATE")
        self.assertFalse(c["pass"])

    def test_stuck(self):
        ts = [abs_off([True, True, True], n=i) for i in range(10)]
        r = ck.analyze(job(ts))
        c = cell_of(r, "ctrl+1d=off:open")
        self.assertEqual(c["verdict"], "STUCK")
        self.assertEqual(c["counts"]["stuck"], 10)
        self.assertEqual(c["moved"], 0)
        self.assertEqual(r["verdict"], "FAIL")

    def test_one_stuck_among_ten_is_stuck(self):
        ts = [abs_off([False], n=i) for i in range(9)] + [abs_off([True, True, True], n=9)]
        c = cell_of(ck.analyze(job(ts)), "ctrl+1d=off:open")
        self.assertEqual(c["verdict"], "STUCK")
        self.assertEqual((c["counts"]["conv1"], c["counts"]["stuck"]), (9, 1))

    def test_gap_not_made(self):
        # 外から開けなかった(押す前の API が閉のまま): r0=True と食い違う。
        ts = [abs_off([False], pre_api=False, n=i) for i in range(10)]
        r = ck.analyze(job(ts))
        self.assertEqual(cell_of(r, "ctrl+1d=off:open")["verdict"], "GAP_NOT_MADE")
        self.assertEqual(r["verdict"], "PASS")  # 測れなかっただけで不合格ではない

    def test_pre_not_ok_is_gap_not_made(self):
        ts = [abs_off([], pre_ok=False, n=i) for i in range(10)]
        self.assertEqual(cell_of(ck.analyze(job(ts)), "ctrl+1d=off:open")["verdict"], "GAP_NOT_MADE")

    def test_invalid_when_half_invalid(self):
        ts = [abs_off([False], n=i) for i in range(4)] + [abs_off([False], focus_lost=True, n=i) for i in range(4, 10)]
        c = cell_of(ck.analyze(job(ts)), "ctrl+1d=off:open")
        self.assertEqual(c["verdict"], "INVALID")

    def test_missing_done_and_abort_are_invalid(self):
        ts = [abs_off([False], n=i) for i in range(10)]
        self.assertEqual(ck.analyze(job(ts, done=False))["verdict"], "INVALID")
        self.assertEqual(ck.analyze(job(ts, done=False, abort="フォーカスが外れた"))["verdict"], "INVALID")
        self.assertEqual(ck.analyze(job([]))["verdict"], "INVALID")

    def test_unreadable_state_is_invalid(self):
        ts = [abs_off([None], n=i) for i in range(10)]
        self.assertEqual(cell_of(ck.analyze(job(ts)), "ctrl+1d=off:open")["verdict"], "INVALID")

    def test_meets_n_flag(self):
        ts = [abs_off([False], n=i) for i in range(6)]
        self.assertFalse(cell_of(ck.analyze(job(ts)), "ctrl+1d=off:open")["meets_n"])


class ChromeEvidence(unittest.TestCase):
    def test_typed_state_is_primary_in_chrome(self):
        # api は「閉」と嘘をつく(TsfNative)が、打鍵(typed_open)は「開のまま」。固着として数える。
        ts = []
        for i in range(10):
            t = abs_off([True, True, True], typed=True, n=i)
            for p in t["presses"]:
                p["api500"] = p["api2000"] = False
            ts.append(t)
        r = ck.analyze(job(ts, form="chrome", ime="gji", evidence="typed"))
        self.assertEqual(cell_of(r, "ctrl+1d=off:open")["verdict"], "STUCK")

    def test_chrome_form_defaults_to_typed(self):
        r = ck.analyze(job([abs_off([False], typed=True, n=i) for i in range(10)], form="chrome", ime="gji"))
        self.assertEqual(r["evidence"], "typed")
        self.assertEqual(cell_of(r, "ctrl+1d=off:open")["verdict"], "CONVERGED_1")


class E2(unittest.TestCase):
    def e2_job(self, drift_fail, fresh_fail, ime="msime", form="chrome", n=10):
        def mk(cell, gap, fails):
            r0 = True
            return [trial(cell, [True, True, True] if i < fails else [False], "off", gap, r0, False, typed=True, n=i)
                    for i in range(n)]
        return job(mk("ctrl+1d=off:open", "open", drift_fail) + mk("ctrl+1d=off:fresh", "fresh", fresh_fail),
                   form=form, ime=ime, evidence="typed")

    def test_env_exception_when_fresh_fails_at_similar_rate(self):
        r = ck.analyze(self.e2_job(9, 8))
        c = cell_of(r, "ctrl+1d=off:open")
        self.assertEqual(c["verdict"], "ENV_EXCEPTION")
        self.assertEqual(c["e2"], "fresh_similar")
        self.assertAlmostEqual(c["fresh_fail_rate"], 0.8)
        self.assertEqual(cell_of(r, "ctrl+1d=off:fresh")["verdict"], "ENV_EXCEPTION")
        self.assertEqual(r["verdict"], "PASS")

    def test_stays_stuck_when_fresh_succeeds(self):
        r = ck.analyze(self.e2_job(9, 0))
        c = cell_of(r, "ctrl+1d=off:open")
        self.assertEqual(c["verdict"], "STUCK")
        self.assertEqual(c["e2"], "fresh_ok")
        self.assertEqual(r["verdict"], "FAIL")

    def test_stays_stuck_when_fresh_fails_much_less(self):
        c = cell_of(ck.analyze(self.e2_job(9, 1)), "ctrl+1d=off:open")
        self.assertEqual(c["verdict"], "STUCK")

    def test_no_exception_outside_the_closed_list(self):
        # GJI × Chrome の OFF が失敗しても例外として認めない(例外は MS-IME × 実 Chrome の OFF 方向だけ)。
        r = ck.analyze(self.e2_job(9, 9, ime="gji"))
        self.assertEqual(cell_of(r, "ctrl+1d=off:open")["verdict"], "STUCK")
        self.assertEqual(cell_of(r, "ctrl+1d=off:fresh")["verdict"], "STUCK")

    def test_no_exception_for_on_direction(self):
        ts = [trial("ctrl+1c=on:close", [False, False, False], "on", "close", False, True, typed=True, n=i)
              for i in range(10)]
        r = ck.analyze(job(ts, form="chrome", ime="msime", evidence="typed"))
        self.assertEqual(cell_of(r, "ctrl+1c=on:close")["verdict"], "STUCK")

    def test_no_exception_outside_chrome(self):
        r = ck.analyze(self.e2_job(9, 9, form="tsf"))
        self.assertEqual(cell_of(r, "ctrl+1d=off:open")["verdict"], "STUCK")

    def test_candidate_without_fresh_cell_stays_stuck(self):
        ts = [trial("ctrl+1d=off:open", [True, True, True], "off", "open", True, False, typed=True, n=i)
              for i in range(10)]
        c = cell_of(ck.analyze(job(ts, form="chrome", ime="msime", evidence="typed")), "ctrl+1d=off:open")
        self.assertEqual(c["verdict"], "STUCK")
        self.assertEqual(c["e2"], "no_fresh_cell")
        self.assertTrue(c["exception_cell"])

    def test_toggle_to_off_is_in_exception_direction(self):
        # トグル × open: 押す前は開(r0=True)、目標は閉(OFF 方向)。MS-IME × Chrome なら例外セル。
        ts = [trial("1d=tog:open", [True, True, True], "tog", "open", True, False, typed=True, n=i) for i in range(10)]
        fr = [trial("1d=tog:fresh", [True, True, True], "tog", "fresh", True, False, typed=True, n=i) for i in range(10)]
        r = ck.analyze(job(ts + fr, form="chrome", ime="msime", evidence="typed"))
        self.assertEqual(cell_of(r, "1d=tog:open")["verdict"], "ENV_EXCEPTION")


class PhysCtrl(unittest.TestCase):
    def ctrl_job(self):
        return job([trial("ctrl+1d=off:open", [False], "off", "open", True, False, n=i) for i in range(10)])

    def test_phys_ctrl_ok(self):
        r = ck.analyze(self.ctrl_job(), [CTRL_OK] * 10)
        self.assertEqual(cell_of(r, "ctrl+1d=off:open")["verdict"], "CONVERGED_1")

    def test_phys_ctrl_false_is_invalid(self):
        r = ck.analyze(self.ctrl_job(), [CTRL_OK, CTRL_BAD])
        c = cell_of(r, "ctrl+1d=off:open")
        self.assertEqual(c["verdict"], "INVALID")
        self.assertIn("phys_ctrl", c["reason"])

    def test_no_ctrl_line_is_invalid(self):
        r = ck.analyze(self.ctrl_job(), ["[engine-input] vk=0x41 KeyDown"])
        self.assertEqual(cell_of(r, "ctrl+1d=off:open")["verdict"], "INVALID")

    def test_non_ctrl_cell_ignores_awase_log(self):
        ts = [trial("1d=off:open", [False], "off", "open", True, False, n=i) for i in range(10)]
        r = ck.analyze(job(ts), [])
        self.assertEqual(cell_of(r, "1d=off:open")["verdict"], "CONVERGED_1")


class Loading(unittest.TestCase):
    def test_chrome_probe_lines(self):
        t = abs_off([False], typed=True)
        lines = [
            '[10:00:00.000Z] KM_CONFIG ' + json.dumps({"form": "chrome", "ime": "msime", "evidence": "typed"}),
            '[10:00:05.000Z] KM ' + json.dumps(t),
            '[10:00:06.000Z] === 全ケース完了 ===',
        ]
        with tempfile.NamedTemporaryFile("w", suffix=".log", delete=False, encoding="utf-8") as f:
            f.write("\n".join(lines) + "\n")
            path = f.name
        try:
            recs = ck.load_records(path)
        finally:
            os.unlink(path)
        r = ck.analyze(recs, None, min_n=1)
        self.assertEqual(r["cfg"]["ime"], "msime")
        self.assertEqual(r["verdict"], "PASS")

    def test_typing_stress_lines_and_summary(self):
        recs = job([abs_off([False], n=i) for i in range(10)])
        with tempfile.NamedTemporaryFile("w", suffix=".log", delete=False, encoding="utf-8") as f:
            for r in recs:
                f.write("[10:00:00.000Z] [TS-JSON] " + json.dumps(r, ensure_ascii=False) + "\n")
            path = f.name
        try:
            rc = ck.main([path])
            r = ck.analyze(ck.load_records(path))
        finally:
            os.unlink(path)
        self.assertEqual(rc, 0)
        self.assertIn("KEYMATRIX: verdict=PASS", ck.summary_line(r))
        self.assertIn("CONVERGED_1=1", ck.summary_line(r))
        self.assertIn("verdict=CONVERGED_1 pass=true", ck.cell_line(r["cells"][0], r["cfg"]))


if __name__ == "__main__":
    unittest.main()
