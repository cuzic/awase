#!/usr/bin/env python3
"""check_drift_correction.py の単体テスト(標準ライブラリの unittest)。
実行: python3 -m unittest discover -s tools/e2e/ime_key_matrix -p 'test_*.py'
"""
import unittest

import check_drift_correction as cdc


def pre(n, real_ime_open):
    return {"type": "drift_pre", "n": n, "real_ime_open": real_ime_open}


def check(n, checkpoint_ms, real_ime_open):
    return {"type": "drift_check", "n": n, "checkpoint_ms": checkpoint_ms, "real_ime_open": real_ime_open}


def done():
    return {"type": "done"}


class Analyze(unittest.TestCase):
    def test_normal_never_closed_is_fail(self):
        recs = [pre(1, True), check(1, 100, True), check(1, 400, True), check(1, 1500, True), done()]
        r = cdc.analyze(recs, [])
        self.assertEqual(r["verdict"], "FAIL")
        self.assertEqual(r["n_never_closed"], 1)
        self.assertEqual(r["n_invalid_trials"], 0)

    def test_normal_reverted_is_fail(self):
        recs = [pre(1, True), check(1, 100, False), check(1, 400, True), check(1, 1500, True), done()]
        r = cdc.analyze(recs, [])
        self.assertEqual(r["verdict"], "FAIL")
        self.assertEqual(r["n_reverted"], 1)
        self.assertEqual(r["n_invalid_trials"], 0)

    def test_clean_close_is_pass(self):
        recs = [pre(1, True), check(1, 100, False), check(1, 400, False), check(1, 1500, False), done()]
        r = cdc.analyze(recs, [])
        self.assertEqual(r["verdict"], "PASS")
        self.assertEqual(r["n_invalid_trials"], 0)

    def test_failed_precondition_does_not_count_as_pass_evidence(self):
        """turn_ime_on()がそもそも失敗した(pre_open=False)試行は、OFF後ずっと閉じたまま
        観測されても「OFFが効いた」証拠にしない(ON→OFFの遷移を検証していないため)。"""
        recs = [pre(1, False), check(1, 100, False), check(1, 400, False), check(1, 1500, False), done()]
        r = cdc.analyze(recs, [])
        self.assertEqual(r["n_reverted"], 0)
        self.assertEqual(r["n_never_closed"], 0)
        self.assertEqual(r["n_invalid_trials"], 1)
        self.assertEqual(r["verdict"], "INVALID")
        self.assertTrue(r["trials"][0]["invalid"])

    def test_unreadable_precondition_is_invalid(self):
        recs = [pre(1, None), check(1, 100, False), check(1, 400, False), check(1, 1500, False), done()]
        r = cdc.analyze(recs, [])
        self.assertEqual(r["n_invalid_trials"], 1)
        self.assertEqual(r["verdict"], "INVALID")

    def test_all_unreadable_checkpoints_is_invalid_not_never_closed(self):
        """real_ime_open が全チェックポイントで None(読み取り不能)だった試行は、
        「一度も閉じなかった」(FAIL)ではなく「観測できなかった」(invalid)として扱う。"""
        recs = [pre(1, True), check(1, 100, None), check(1, 400, None), check(1, 1500, None), done()]
        r = cdc.analyze(recs, [])
        self.assertEqual(r["n_never_closed"], 0)
        self.assertEqual(r["n_invalid_trials"], 1)
        self.assertEqual(r["verdict"], "INVALID")

    def test_mixed_valid_and_invalid_trials(self):
        recs = [
            pre(1, True), check(1, 100, True), check(1, 400, True), check(1, 1500, True),
            pre(2, False), check(2, 100, False), check(2, 400, False), check(2, 1500, False),
            done(),
        ]
        r = cdc.analyze(recs, [])
        self.assertEqual(r["n_trials"], 2)
        self.assertEqual(r["n_never_closed"], 1)
        self.assertEqual(r["n_invalid_trials"], 1)
        # 有効な試行のうち1件がFAILなので、全体もFAIL(全試行がinvalidな場合のみINVALIDにする)
        self.assertEqual(r["verdict"], "FAIL")


if __name__ == "__main__":
    unittest.main()
