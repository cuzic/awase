#!/usr/bin/env python3
"""check_invariants.py の単体テスト(標準ライブラリの unittest)。

フィクスチャ(testdata/)は CI の実ログの抜粋:
  awase-baseline-excerpt.log  run 36103384502(f2a875cd)の baseline 2回目 awase.log から、drift の観測サイクル・
                              Unwarranted・IME モードキーのフック行・warrant-shadow 行を抜いたもの
  awase-old-sc-dbe-excerpt.log run 35620809258(ci/e2e-drift-fix)の real-sc-dbe awase.log から、drift とその直前の
                              explicit_intent 行だけを抜いたもの(BUG-163 の「約11秒に22件」)
実行: python3 -m unittest discover -s tools/e2e/ime_key_matrix -p 'test_*.py'
"""
import io
import json
import os
import tempfile
import unittest
from contextlib import redirect_stdout

import check_invariants as ci

HERE = os.path.dirname(os.path.abspath(__file__))
DATA = os.path.join(HERE, "testdata")


def read(name):
    with open(os.path.join(DATA, name), encoding="utf-8") as f:
        return f.read().splitlines()


def run_main(args):
    buf = io.StringIO()
    with redirect_stdout(buf):
        rc = ci.main(args)
    return rc, buf.getvalue()


class AnalyzeBaseline(unittest.TestCase):
    def setUp(self):
        self.lines = read("awase-baseline-excerpt.log")
        self.r = ci.analyze(self.lines, 10)

    def test_started(self):
        self.assertTrue(self.r["started"])

    def test_i1_startup_window_and_total(self):
        c = self.r["counts"]
        self.assertEqual(c["i1_startup_drift_no_intent"], 3)  # +1.29s, +1.80s, +2.29s
        self.assertEqual(c["i1_drift_no_intent_total"], 4)  # + フォーカス変更後の +14.17s
        self.assertEqual([d["target"] for d in self.r["detail"]["drifts"]], ["true"] * 4)
        self.assertEqual({d["source"] for d in self.r["detail"]["drifts"]}, {"ImmCrossProbe"})
        self.assertEqual(self.r["detail"]["drift_intent_unknown"], 0)

    def test_i1_window_length_is_parameter(self):
        self.assertEqual(ci.analyze(self.lines, 1.5)["counts"]["i1_startup_drift_no_intent"], 1)
        self.assertEqual(ci.analyze(self.lines, 60)["counts"]["i1_startup_drift_no_intent"], 4)

    def test_i2_counts_journal_line_only_not_span_lines(self):
        # 同じ span(on_ime_apply_complete{… outcome=Unwarranted …})の Timer set 行は数えない
        span_lines = [l for l in self.lines if "outcome=Unwarranted" in l]
        self.assertEqual(len(span_lines), 2)  # check.py 方式だと 2 件
        self.assertEqual(self.r["counts"]["i2_unwarranted"], 1)
        self.assertEqual(self.r["detail"]["unwarranted_seqs"], ["65"])

    def test_i2_duplicate_seq_counts_once(self):
        dup = [l for l in self.lines if "ime open applied" in l]
        r = ci.analyze(self.lines + dup, 10)
        self.assertEqual(r["counts"]["i2_unwarranted"], 1)

    def test_i3_info(self):
        c, d = self.r["counts"], self.r["detail"]
        self.assertEqual(c["i3_self_injected_ime_mode_keys"], 7)  # down のみ。物理(self_injected=false)は数えない
        self.assertEqual(d["self_injected_by_vk"], {"0x16": 1, "0x1A": 2, "0xF2": 4})
        self.assertEqual(c["i3_warrant_shadow_would_block"], 5)  # "warranted" の行は数えない
        self.assertEqual(d["would_block_by_chain"], {
            "async/engine_decision_async/open=true": 1,
            "set_ime_open/drift_correction_read/open=true": 4,
        })


class AnalyzeIntent(unittest.TestCase):
    def test_explicit_intent_some_is_not_counted(self):
        lines = [l.replace("explicit_intent=None", "explicit_intent=Some(true)") for l in read("awase-baseline-excerpt.log")]
        r = ci.analyze(lines, 10)
        self.assertEqual(r["counts"]["i1_drift_no_intent_total"], 0)
        self.assertEqual(r["detail"]["drift_with_intent"], 4)

    def test_missing_intent_line_counts_as_no_intent(self):
        # 書式が変わって explicit_intent= が消えても黙って0件にならない
        lines = [l for l in read("awase-baseline-excerpt.log") if "explicit_intent=" not in l]
        r = ci.analyze(lines, 10)
        self.assertEqual(r["counts"]["i1_drift_no_intent_total"], 4)
        self.assertEqual(r["detail"]["drift_intent_unknown"], 4)

    def test_old_run_bug163_22_corrections(self):
        r = ci.analyze(read("awase-old-sc-dbe-excerpt.log"), 10)
        self.assertEqual(r["counts"]["i1_drift_no_intent_total"], 22)
        self.assertEqual(r["counts"]["i1_startup_drift_no_intent"], 18)
        self.assertEqual({d["source"] for d in r["detail"]["drifts"]}, {"ObserverPoll"})

    def test_seven_digit_fraction_timestamp(self):
        lines = ["2026-09-25T06:37:56.1553812Z  INFO x: Keyboard Layout Emulator starting..."]
        self.assertTrue(ci.analyze(lines, 10)["started"])


class GjiStuckOffCold(unittest.TestCase):
    """I4(BUG-170/ADR-203): GjiFsm が OffCold のまま StartComposition を受けた回数。
    フィクスチャは CI の実ログの抜粋: 修正前=develop@e174c6f6 の run 36506566832 baseline-1、
    修正後=fix/bugreport-01M3NBQA の run 36513008912 baseline-1。"""

    def test_stuck_log_counts_start_composition_while_off(self):
        r = ci.analyze(read("awase-gji-stuck-offcold-excerpt.log"), 10)
        self.assertEqual(r["counts"]["i4_gji_fsm_off_cold_composition"], 2)

    def test_reopened_log_has_none(self):
        lines = read("awase-gji-reopened-excerpt.log")
        # 正常な StartComposition(OnWarm/OnCold → OnComposing)の行を含む抜粋。何もしなくても0になる
        # 空の fixture ではなく、正規表現が正常な StartComposition を誤って数えないことまで固定する。
        self.assertTrue(any("StartComposition" in l for l in lines))
        r = ci.analyze(lines, 10)
        self.assertEqual(r["counts"]["i4_gji_fsm_off_cold_composition"], 0)

    def test_stuck_log_fails_the_zero_limit(self):
        rc, out = run_main(["--config", "baseline", os.path.join(DATA, "awase-gji-stuck-offcold-excerpt.log")])
        self.assertEqual(rc, 1)
        self.assertIn("i4_gji_fsm_off_cold_composition", out)
        self.assertIn("i4_gji_stuck=2", out.splitlines()[-1])

    def test_reopened_log_passes_the_i4_limit(self):
        rc, out = run_main(["--config", "baseline", os.path.join(DATA, "awase-gji-reopened-excerpt.log")])
        self.assertIn("i4_gji_stuck=0", out.splitlines()[-1])
        self.assertNotIn("FAIL: 上限 0(BUG-170)", out)


class Judge(unittest.TestCase):
    LIM = {"i1_startup_drift_no_intent": {"max": 3, "observed_min": 2, "bug": "BUG-163"}}

    def verdict(self, v):
        counts = dict(i1_startup_drift_no_intent=v, i1_drift_no_intent_total=0, i2_unwarranted=0,
                      i4_gji_fsm_off_cold_composition=0)
        verdict, rows = ci.judge(counts, self.LIM)
        return verdict, rows[0][3]

    def test_over_limit_fails(self):
        v, msg = self.verdict(4)
        self.assertEqual(v, "FAIL")
        self.assertIn("BUG-163", msg)

    def test_within_jitter_is_ok_without_lowering_hint(self):
        for n in (2, 3):
            v, msg = self.verdict(n)
            self.assertEqual(v, "OK")
            self.assertNotIn("下げてよい", msg)

    def test_below_observed_min_suggests_lowering(self):
        v, msg = self.verdict(0)
        self.assertEqual(v, "OK")
        self.assertIn("下げてよい", msg)

    def test_no_limit_is_info_only(self):
        verdict, rows = ci.judge(dict(i1_startup_drift_no_intent=0, i1_drift_no_intent_total=99, i2_unwarranted=0, i4_gji_fsm_off_cold_composition=0), self.LIM)
        self.assertEqual(verdict, "OK")


class Limits(unittest.TestCase):
    def test_repo_limits_have_bug_and_measurement(self):
        with open(ci.DEFAULT_LIMITS, encoding="utf-8") as f:
            doc = json.load(f)
        for key in ci.GATED:
            lim = doc["limits"][key]
            self.assertRegex(lim["bug"], r"^BUG-\d{3}$", key)
            self.assertTrue(lim.get("measured"), key)
            self.assertLessEqual(lim["observed_min"], lim["max"], key)

    def test_config_override(self):
        with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False, encoding="utf-8") as f:
            json.dump({"window_s": 5, "measured_configs": ["x"],
                       "limits": {"i2_unwarranted": {"max": 1, "bug": "BUG-162"}},
                       "config_overrides": {"x": {"i2_unwarranted": {"max": 0}}}}, f)
        try:
            w, lim, measured = ci.load_limits(f.name, "x")
            self.assertEqual((w, lim["i2_unwarranted"]["max"], lim["i2_unwarranted"]["bug"], measured), (5, 0, "BUG-162", True))
            w, lim, measured = ci.load_limits(f.name, "y")
            self.assertEqual((lim["i2_unwarranted"]["max"], measured), (1, False))
        finally:
            os.unlink(f.name)


class Main(unittest.TestCase):
    def test_baseline_passes_repo_limits(self):
        # 修正後（#318・#321、run 36140615943）の baseline: 起動直後の drift は 0 件、Unwarranted は 1 件（BUG-162）。
        rc, out = run_main(["--config", "baseline", os.path.join(DATA, "awase-baseline-fixed-excerpt.log")])
        self.assertEqual(rc, 0)
        self.assertIn("INVARIANTS: verdict=OK rc=0 i1_startup=0 i1_total=0 i2_unwarranted=1", out)

    def test_pre_fix_baseline_fails_the_lowered_limits(self):
        # 修正前（run 36103384502）の baseline は、下げた上限（i1=0）を超える（ratchet が緩まない）。
        rc, out = run_main(["--config", "baseline", os.path.join(DATA, "awase-baseline-excerpt.log")])
        self.assertEqual(rc, 1)
        self.assertIn("verdict=FAIL", out.splitlines()[-1])

    def test_old_run_fails_repo_limits(self):
        rc, out = run_main([os.path.join(DATA, "awase-old-sc-dbe-excerpt.log")])
        self.assertEqual(rc, 1)
        self.assertIn("verdict=FAIL", out.splitlines()[-1])

    def test_missing_log_is_invalid(self):
        rc, out = run_main([os.path.join(DATA, "no-such.log")])
        self.assertEqual(rc, 3)
        self.assertIn("verdict=INVALID", out)

    def test_log_without_start_mark_is_invalid(self):
        with tempfile.NamedTemporaryFile("w", suffix=".log", delete=False, encoding="utf-8") as f:
            f.write("\n".join(read("awase-baseline-excerpt.log")[1:]) + "\n")
        try:
            rc, _ = run_main([f.name])
            self.assertEqual(rc, 3)
        finally:
            os.unlink(f.name)

    def test_json_output(self):
        with tempfile.TemporaryDirectory() as d:
            out = os.path.join(d, "inv.json")
            rc, _ = run_main(["--config", "baseline", "--json", out, os.path.join(DATA, "awase-baseline-fixed-excerpt.log")])
            with open(out, encoding="utf-8") as f:
                j = json.load(f)
        self.assertEqual((rc, j["verdict"], j["counts"]["i2_unwarranted"], j["measured_config"]), (0, "OK", 1, True))



class StaleConfirmI5(unittest.TestCase):
    """I5(BUG-171): 前の未確定文字がある途中の語での StaleConfirm→escape=true だけを mid_word に数える(情報のみ)。"""

    @staticmethod
    def stale(idx, escape, sec):
        return (f"2026-10-04T00:00:{sec:02d}.000000Z  WARN awase_windows::tsf::probe_fsm: [literal-detect] cold=3 "
                f"per-VK[{idx}/5] stale confirm 検出 → backspace は送らず romaji 再送のみ行う (vk=0x44 backs=0 escape={escape})")

    @staticmethod
    def start(before, sec):
        return (f"2026-10-04T00:00:{sec:02d}.000000Z DEBUG awase::journal: gji fsm transition seq=1 elapsed_ms=1 "
                f'trigger="StartComposition(candidate SHOW)" state_before="{before}" state_after="OnComposing(Warm)"')

    def test_first_char_vs_mid_word(self):
        lines = [
            "2026-10-04T00:00:00.000000Z  INFO awase: hook installed",
            self.start("OnCold(Long)", 1), self.stale(1, "true", 2),   # セッション最初の文字: first_char
            self.start("OnComposing(Warm)", 3), self.stale(1, "true", 4),  # 途中の語: mid_word
            self.stale(3, "true", 5),                                   # 同じ途中の語
            self.stale(0, "true", 6),                                   # idx=0: どちらにも数えない(総数のみ)
            self.stale(2, "false", 7),                                  # ESC なし: 総数のみ
        ]
        r = ci.analyze(lines, 10)
        self.assertEqual(r["counts"]["i5_stale_confirm_total"], 5)
        self.assertEqual(r["counts"]["i5_first_char_stale_escape"], 1)
        self.assertEqual(r["counts"]["i5_mid_word_stale_escape"], 2)
        self.assertEqual(r["detail"]["mid_word_stale_escape_idx"], [1, 3])

    def test_baseline_has_none(self):
        r = ci.analyze(read("awase-baseline-excerpt.log"), 10)
        self.assertEqual(r["counts"]["i5_mid_word_stale_escape"], 0)

if __name__ == "__main__":
    unittest.main()
