import unittest

import scope_pure as P


class T(unittest.TestCase):
    def test_classify(self):
        self.assertEqual(P.classify("か"), "hira")
        self.assertEqual(P.classify(" ka\r\n"), "ka")
        self.assertEqual(P.classify("ｋａ"), "ka")
        self.assertEqual(P.classify(""), "none")
        self.assertEqual(P.classify("kk"), "other")

    def test_majority(self):
        self.assertEqual(P.majority(["a", "a", "b"]), ("a", 2, 3))
        self.assertEqual(P.majority(["a", "b"])[0], "mixed")
        self.assertEqual(P.majority([]), (None, 0, 0))

    def test_judge(self):
        self.assertEqual(P.judge_open("ka", "hira"), "shared-open")
        self.assertEqual(P.judge_open("ka", "ka"), "independent")
        self.assertTrue(P.judge_open("hira", "hira").startswith("inconclusive"))
        self.assertEqual(P.judge_conv("hira", "ka"), "shared-conv")
        self.assertEqual(P.judge_conv("hira", "hira"), "independent")
        self.assertTrue(P.judge_conv("ka", "ka").startswith("inconclusive"))


if __name__ == "__main__":
    unittest.main()
