#!/usr/bin/env python3
"""check_preedit.py の単体テスト。実行: python3 -m unittest discover -s tools/e2e/ime_key_matrix -p 'test_*.py'"""
import unittest

import check_preedit as cp


def cfg(form="chromepage", ime="gji"):
    return {"type": "config", "form": form, "ime": ime}


def pe(n, phase, uia="none", imm="nohimc", text="", value=""):
    return {"type": "preedit", "n": n, "phase": phase, "uia": uia, "imm": imm, "text": text, "value": value}


def trial(n, uia_comp="composing:か", uia_none="none", imm=None):
    i = (lambda s: s) if imm else (lambda s: "nohimc")
    return [pe(n, "before", uia_none, i("none")), pe(n, "composing", uia_comp, i("composing:か")),
            pe(n, "composing2", uia_comp, i("composing:かか")), pe(n, "after", uia_none, i("none"))]


class Classify(unittest.TestCase):
    def test_right_wrong_unavailable(self):
        self.assertEqual(cp.classify("composing:か", "composing"), "right")
        self.assertEqual(cp.classify("none", "composing"), "wrong")
        self.assertEqual(cp.classify("composing:か", "none"), "wrong")
        self.assertEqual(cp.classify("nopattern", "none"), "unavailable")
        self.assertEqual(cp.classify("err:focus", "none"), "unavailable")
        self.assertEqual(cp.classify("nohimc", "composing"), "unavailable")

    def test_truth(self):
        self.assertEqual(cp.truth("before", "enter"), "none")
        self.assertEqual(cp.truth("composing2", "esc"), "composing")
        self.assertIsNone(cp.truth("after", "none"))
        self.assertIsNone(cp.truth("after", "esc"))
        self.assertEqual(cp.truth("after", "enter"), "none")
        self.assertEqual(cp.truth("after_late", "enter"), "none")


class Analyze(unittest.TestCase):
    def test_uia_reads_imm_unavailable(self):
        r = cp.analyze([cfg()] + trial(0) + trial(1), done=True)
        self.assertEqual(r["verdict"], "OBSERVED")
        self.assertEqual(r["methods"]["uia"], "READS")
        self.assertEqual(r["methods"]["imm"], "UNUSABLE")  # 全部 unavailable=right が 0
        self.assertEqual(r["n"], 2)

    def test_uia_misses_composition_is_unusable(self):
        r = cp.analyze([cfg()] + trial(0, uia_comp="none"), done=True)
        self.assertEqual(r["methods"]["uia"], "UNUSABLE")
        self.assertEqual(r["counts"]["uia"]["wrong"], 2)

    def test_partial_when_some_unavailable(self):
        recs = [cfg()] + trial(0) + [pe(1, "before", "nopattern"), pe(1, "composing", "composing:か")]
        self.assertEqual(cp.analyze(recs, done=True)["methods"]["uia"], "PARTIAL")

    def test_text_leak_counted(self):
        recs = [cfg(), pe(0, "composing", "composing:か", text="か")]
        self.assertEqual(cp.analyze(recs, done=True)["text_leak"], 1)

    def test_invalid_without_done_or_reads(self):
        self.assertEqual(cp.analyze([cfg()], done=True)["verdict"], "INVALID")
        self.assertEqual(cp.analyze([cfg()] + trial(0), done=False)["verdict"], "INVALID")
        self.assertEqual(cp.analyze([cfg(), {"type": "abort", "reason": "x"}] + trial(0), done=True)["verdict"], "INVALID")

    def test_esc_after_is_observed_not_judged(self):
        recs = [{"type": "preedit_config", "end": "esc"}, pe(0, "after", "composing:かか", "composing:かか")]
        r = cp.analyze(recs, done=True)
        self.assertEqual(r["counts"]["uia"]["wrong"], 0)
        self.assertEqual(r["observed"]["uia/after"], {"composing": 1})

    def test_end_none_skips_after(self):
        recs = [{"type": "preedit_config", "end": "none"}, pe(0, "after", "composing:か")]
        self.assertEqual(cp.analyze(recs, done=True)["counts"]["uia"]["wrong"], 0)


if __name__ == "__main__":
    unittest.main()
