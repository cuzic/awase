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


if __name__ == "__main__":
    unittest.main()
