#!/usr/bin/env python3
"""check_chrome_probe.py の判定(特に --strict、ADR-244)のテスト。実行: python3 -m unittest discover -s tools/e2e/ime_key_matrix -p 'test_*.py'"""
import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
CHECK = os.path.join(HERE, "check_chrome_probe.py")


def run(lines, strict):
    with tempfile.NamedTemporaryFile("w", suffix=".log", delete=False, encoding="utf-8") as f:
        f.write("\n".join(lines) + "\n")
        path = f.name
    try:
        args = [sys.executable, CHECK] + (["--strict"] if strict else []) + [path]
        return subprocess.run(args, capture_output=True, text=True, encoding="utf-8").returncode
    finally:
        os.unlink(path)


DONE = "[00:00:00.000Z] === 全ケース完了 ==="


def result(kind, name="Shift単独タップ後 → 変換"):
    return f"[00:00:00.000Z] RESULT {kind}: {name}"


class Strict(unittest.TestCase):
    def test_all_pass_with_marker_is_ok_in_both_modes(self):
        lines = [result("PASS"), result("PASS"), DONE]
        self.assertEqual(run(lines, strict=False), 0)
        self.assertEqual(run(lines, strict=True), 0)

    def test_fail_is_red_in_both_modes(self):
        lines = [result("PASS"), result("FAIL"), DONE]
        self.assertEqual(run(lines, strict=False), 1)
        self.assertEqual(run(lines, strict=True), 1)

    def test_invalid_is_red_only_in_strict(self):
        # 持続トグルを手放さない退行は、次のケースの setup が INVALID になる形で出る(ADR-244)。
        lines = [result("PASS"), result("INVALID"), DONE]
        self.assertEqual(run(lines, strict=False), 0)
        self.assertEqual(run(lines, strict=True), 1)

    def test_recover_is_red_only_in_strict(self):
        lines = [result("PASS"), result("RECOVER"), DONE]
        self.assertEqual(run(lines, strict=False), 0)
        self.assertEqual(run(lines, strict=True), 1)

    def test_missing_completion_marker_is_red_only_in_strict(self):
        lines = [result("PASS"), result("PASS")]
        self.assertEqual(run(lines, strict=False), 0)
        self.assertEqual(run(lines, strict=True), 1)

    def test_no_valid_case_is_invalid_rc3(self):
        self.assertEqual(run([result("INVALID"), DONE], strict=False), 3)
        self.assertEqual(run([result("INVALID"), DONE], strict=True), 3)


if __name__ == "__main__":
    unittest.main()
