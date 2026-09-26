#!/usr/bin/env python3
"""check_kanji_role.py の単体テスト。フィクスチャ(testdata/kanjirole-*)はスパイクの KEY 行と awase ログの書式に合わせた手書きの合成ログ。
実行: python3 -m unittest discover -s tools/e2e/ime_key_matrix -p 'test_*.py'
"""
import io
import os
import sys
import unittest
from contextlib import redirect_stdout

import check_kanji_role as ck

DATA = os.path.join(os.path.dirname(os.path.abspath(__file__)), "testdata")


def rc_of(expect, name):
    old = sys.argv
    sys.argv = ["check_kanji_role.py", f"--expect={expect}", os.path.join(DATA, f"{name}.spike.log"), os.path.join(DATA, f"{name}.awase.log")]
    try:
        with redirect_stdout(io.StringIO()):
            return ck.main()
    finally:
        sys.argv = old


class CheckKanjiRole(unittest.TestCase):
    def test_open_passes_when_ime_stays_open_and_engine_stays_on(self):
        self.assertEqual(rc_of("open", "kanjirole-open-pass"), 0)

    def test_open_fails_when_belief_flips_although_ime_stays_open(self):
        self.assertEqual(rc_of("open", "kanjirole-open-fail-belief"), 1)

    def test_closed_passes_when_ime_closes_and_engine_follows(self):
        self.assertEqual(rc_of("closed", "kanjirole-closed-pass"), 0)

    def test_closed_fails_when_engine_does_not_follow(self):
        self.assertEqual(rc_of("closed", "kanjirole-closed-fail-no-follow"), 1)

    def test_invalid_when_ime_was_not_on_before(self):
        self.assertEqual(rc_of("open", "kanjirole-invalid"), 3)


if __name__ == "__main__":
    unittest.main()
