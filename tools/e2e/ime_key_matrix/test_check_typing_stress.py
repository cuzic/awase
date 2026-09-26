#!/usr/bin/env python3
"""check_typing_stress.py の単体テスト(標準ライブラリの unittest)。
フィクスチャ(testdata/typing-stress-*.log)は typing_stress example のログ形式([TS-JSON] 行)を手書きしたもの。
実行: python3 -m unittest discover -s tools/e2e/ime_key_matrix -p 'test_*.py'
"""
import io
import json
import os
import tempfile
import unittest
from contextlib import redirect_stdout

import check_typing_stress as cts

HERE = os.path.dirname(os.path.abspath(__file__))


def data(name):
    return os.path.join(HERE, "testdata", name)


def run_main(args):
    buf = io.StringIO()
    with redirect_stdout(buf):
        rc = cts.main(args)
    return rc, buf.getvalue()


class Classify(unittest.TestCase):
    def test_match_ignores_newlines(self):
        self.assertTrue(cts.classify("かたこ", "かたこ\r\n")["ok"])

    def test_loss(self):
        c = cts.classify("かたこさら", "かたさら")
        self.assertEqual((c["loss"], c["extra"], c["reorder"], c["literal"], c["substitute"]), (1, 0, 0, 0, 0))

    def test_extra_duplicate(self):
        c = cts.classify("かたこ", "かたたこ")
        self.assertEqual((c["extra"], c["loss"]), (1, 0))

    def test_reorder(self):
        c = cts.classify("がだござよ", "だがござよ")
        self.assertEqual((c["reorder"], c["loss"], c["extra"]), (2, 0, 0))
        self.assertFalse(c["ok"])

    def test_literal(self):
        c = cts.classify("かたこ", "かtaこ")
        self.assertEqual(c["literal"], 2)
        self.assertEqual(c["loss"], 0)

    def test_literal_appended(self):
        c = cts.classify("かた", "かたk")
        self.assertEqual((c["literal"], c["extra"]), (1, 0))

    def test_substitute(self):
        c = cts.classify("がだ", "かだ")
        self.assertEqual((c["substitute"], c["loss"], c["extra"]), (1, 0, 0))

    def test_all_lost(self):
        c = cts.classify("かたこ", "")
        self.assertEqual(c["loss"], 3)


class Main(unittest.TestCase):
    def test_pass(self):
        rc, out = run_main([data("typing-stress-pass.log")])
        self.assertEqual(rc, 0)
        self.assertIn("TYPING_STRESS: verdict=PASS form=edit ime=gji mode=nicola interval_ms=10.0 trials=2 fail=0", out)

    def test_fail_categories(self):
        rc, out = run_main([data("typing-stress-fail.log")])
        self.assertEqual(rc, 1)
        last = out.strip().splitlines()[-1]
        self.assertIn("verdict=FAIL", last)
        self.assertIn("trials=4 fail=4", last)
        self.assertIn("loss=1", last)
        self.assertIn("reorder=2", last)
        self.assertIn("literal=2", last)
        self.assertIn("substitute=1", last)
        self.assertIn("single:1/1", last)
        self.assertIn("期待: がだござよ", out)

    def test_abort_is_invalid(self):
        rc, out = run_main([data("typing-stress-invalid-abort.log")])
        self.assertEqual(rc, 3)
        self.assertIn("verdict=INVALID", out)
        self.assertIn("中断", out)

    def test_dropped_sendinput_is_invalid(self):
        rc, out = run_main([data("typing-stress-invalid-dropped.log")])
        self.assertEqual(rc, 3)
        self.assertIn("SendInput が 10/12", out)

    def test_missing_log_is_invalid(self):
        rc, out = run_main(["/nonexistent/typing_stress.log"])
        self.assertEqual(rc, 3)

    def test_no_done_marker_is_invalid(self):
        with tempfile.TemporaryDirectory() as d:
            p = os.path.join(d, "t.log")
            with open(p, "w", encoding="utf-8") as f:
                f.write("[00:00:00.000Z] [init] only\n")
            rc, out = run_main([p])
        self.assertEqual(rc, 3)
        self.assertIn("試行が0件", out)

    def test_json_output(self):
        with tempfile.TemporaryDirectory() as d:
            j = os.path.join(d, "o.json")
            rc, _ = run_main(["--json", j, data("typing-stress-fail.log")])
            with open(j, encoding="utf-8") as f:
                got = json.load(f)
        self.assertEqual(rc, 1)
        self.assertEqual(got["verdict"], "FAIL")
        self.assertEqual(got["totals"]["loss"], 1)

    def test_usage(self):
        rc, _ = run_main([])
        self.assertEqual(rc, 2)


if __name__ == "__main__":
    unittest.main()
