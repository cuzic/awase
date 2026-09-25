"""check.py（ADR-186 期待表）の単体テスト。BUG-162 の B（手順5・6の期待）と C（Unwarranted の数え方）を固定する。"""
import importlib.util
import os
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("check", os.path.join(HERE, "check.py"))
check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check)


def write(text):
    f = tempfile.NamedTemporaryFile("w", suffix=".log", delete=False, encoding="utf-8")
    f.write(text)
    f.close()
    return f.name


class ParseSpike(unittest.TestCase):
    def test_skipped_step_is_recorded(self):
        p = write(
            "[06:38:29.622Z] KEY [SCRIPT 5/10 無変換 期待=x] 無変換 vk=0x1D press=06:38:28.043Z (auto)\n"
            "  +400ms: A(open=1 conv=0x19)\n"
            "[AUTO] STEP 6 無変換: 前提状態にできずスキップ(現在=IME ON・かな・入力なし)\n"
        )
        steps = check.parse_spike(p)
        self.assertEqual(steps[5]["open"], 1)
        self.assertTrue(steps[6]["skipped"])
        self.assertNotIn("open", steps[6])


class ParseAwase(unittest.TestCase):
    def test_unwarranted_counts_the_journal_line_once_per_seq(self):
        # 同じ span の別の行（Timer set）と、同じ seq の重複は数えない（BUG-162 C）。
        p = write(
            '2026-09-25T06:38:16.100Z DEBUG on_ime_apply_complete{open=true outcome=Unwarranted generation=None}: awase::journal: ime open applied seq=10 elapsed_ms=1 open=true outcome="Unwarranted"\n'
            "2026-09-25T06:38:16.101Z DEBUG on_ime_apply_complete{open=true outcome=Unwarranted generation=None}: awase_windows::timer: Timer set: logical=101\n"
            '2026-09-25T06:38:16.102Z DEBUG on_ime_apply_complete{open=true outcome=Unwarranted generation=None}: awase::journal: ime open applied seq=10 elapsed_ms=1 open=true outcome="Unwarranted"\n'
            '2026-09-25T06:38:17.100Z DEBUG awase::journal: ime open applied seq=11 elapsed_ms=2 open=true outcome="AppliedWithoutSendInput"\n'
        )
        _events, unwarranted = check.parse_awase(p)
        self.assertEqual(unwarranted, 1)


class Expectations(unittest.TestCase):
    def test_step5_expects_the_suppressed_state_not_delegate(self):
        exp = check.EXPECT[5]
        self.assertEqual((exp["real_open"], exp["real_conv"]), (1, 0x19))
        self.assertNotIn("delegate_false", exp)

    def test_only_step6_is_skippable(self):
        self.assertEqual(check.SKIPPABLE, {6})


if __name__ == "__main__":
    unittest.main()
