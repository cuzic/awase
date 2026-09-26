#!/usr/bin/env python3
"""check_charthumb.py の単体テスト。フィクスチャ(testdata/charthumb-*.log)はスパイクの KEY 行の書式に合わせた手書きの合成ログ。
実行: python3 -m unittest discover -s tools/e2e/ime_key_matrix -p 'test_*.py'
"""
import io
import os
import sys
import unittest
from contextlib import redirect_stdout

import check_charthumb as cc

DATA = os.path.join(os.path.dirname(os.path.abspath(__file__)), "testdata")


def rc_of(name):
    old = sys.argv
    sys.argv = ["check_charthumb.py", os.path.join(DATA, name)]
    try:
        with redirect_stdout(io.StringIO()):
            return cc.main()
    finally:
        sys.argv = old


class CheckCharThumb(unittest.TestCase):
    def test_pass_when_open_while_held_and_closed_after_release(self):
        self.assertEqual(rc_of("charthumb-pass.log"), 0)

    def test_fail_when_ime_closed_while_thumb_still_held(self):
        self.assertEqual(rc_of("charthumb-fail.log"), 1)

    def test_invalid_when_ime_was_not_on_before(self):
        self.assertEqual(rc_of("charthumb-invalid.log"), 3)


if __name__ == "__main__":
    unittest.main()
