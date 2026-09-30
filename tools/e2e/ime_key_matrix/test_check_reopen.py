import json
import os
import tempfile
import unittest

import check_reopen as cr


def ts_log(recs):
    return "\n".join("[12:00:00.000Z] [TS-JSON] " + json.dumps(r, ensure_ascii=False) for r in recs) + "\n"


def base(n, trial="12:00:08.000", off="12:00:09.400", on="12:00:10.000", press="12:00:10.010", typed="12:00:11.500",
         pre_ok=True, typed_text="か", open_before_on=False, open_after=True, focus_lost=False):
    return [
        {"type": "reopen_pre", "n": n, "trial_utc": trial, "utc": "12:00:09.000", "text": "か" if pre_ok else "ka", "expect": "か", "ok": pre_ok},
        {"type": "reopen_on", "n": n, "off_utc": off, "on_utc": on, "gap_ms": 600, "open_before_on": open_before_on},
        {"type": "reopen_typed", "n": n, "utc": typed, "on_utc": on, "press_utc": press, "focus_lost": focus_lost,
         "text": typed_text, "expect": "か", "ok": typed_text == "か", "real_ime_open": open_after},
    ]


HEAD = [{"type": "config", "form": "tsf", "ime": "gji", "mode": "reopen"}]
DONE = [{"type": "done"}]

# 以下は実ログ(CI の awase.log / ソースの tracing 文字列)の書式そのもの。
P = "2026-09-29T12:00:"
REOPEN = P + '10.005000Z DEBUG awase::journal: gji fsm transition seq=1 trigger="Reopen(BeliefSync:on-key)(gji_idle_ms=600)" state_before="OffCold" state_after="OnCold(Short)"\n'
IMEON_BELIEF = P + '10.006000Z DEBUG awase::journal: gji fsm transition seq=2 trigger="ImeOn(BeliefSync:level)(gji_idle_ms=610)" state_before="OffCold" state_after="OnCold(Short)"\n'
IMEON_OTHER = P + '10.007000Z DEBUG awase::journal: gji fsm transition seq=3 trigger="ImeOn(gji_idle_ms=1985)" state_before="OffCold" state_after="OnCold(Short)"\n'
VK_COLD = P + '10.052000Z DEBUG awase_windows::output::vk_send: [vk-send] romaji="ka" warm=false elapsed=0ms session_expired=false prepend_f2_warmup=true\n'
VK_WARM = P + '10.052000Z DEBUG execute_one{generation=None}: awase_windows::output::vk_send: [vk-send] romaji="ka" warm=true elapsed=0ms session_expired=false prepend_f2_warmup=false\n'
CONFIRM = P + '10.140000Z DEBUG awase_windows::tsf::warmup::probe_fsm: [warmup] cold=7 per-VK: 全 2 VK 確認済み → セッション確認\n'
STUCK = P + '10.900000Z  WARN gji_on_event{event=StartComposition}: awase_windows::tsf::gji_fsm: [gji-fsm] StartComposition while engine off — ignored\n'
STALE_ESC = P + '10.700000Z  WARN awase_windows::tsf::warmup::probe_fsm: [warmup] cold=7 per-VK[1/2] stale confirm 検出 → backspace は送らず romaji 再送のみ行う (vk=0x4B backs=0 escape=true)\n'
STALE_NOESC = P + '10.700000Z  WARN awase_windows::tsf::warmup::probe_fsm: [warmup] cold=7 per-VK[0/2] stale confirm 検出 → backspace は送らず romaji 再送のみ行う (vk=0x4B backs=0 escape=false)\n'
FLUSH_ESC = P + '10.710000Z DEBUG awase_windows::tsf::output: [raw-tsf-literal] flush escape=true backspace ×1\n'
UNICODE = P + '10.060000Z DEBUG awase_windows::output: send_keys: mode=Unicode\n'
CLEAN = REOPEN + VK_COLD + CONFIRM


def run(recs, awase, require_cold=False, require_sync=False):
    with tempfile.TemporaryDirectory() as d:
        p, a = os.path.join(d, "ts.log"), os.path.join(d, "awase.log")
        open(p, "w", encoding="utf-8").write(ts_log(recs))
        open(a, "w", encoding="utf-8").write(awase)
        return cr.analyze(cr.parse(p), cr.load_awase(a), require_cold, require_sync)


class CheckReopen(unittest.TestCase):
    def test_pass_and_delays(self):
        r = run(HEAD + base(0) + DONE, CLEAN)
        self.assertEqual(r["verdict"], "PASS")
        t = r["trials"][0]
        self.assertEqual(t["vk_after_press_ms"], 42)  # 10.010 → 10.052
        self.assertEqual(t["confirm_ms"], 88)  # vk-send 10.052 → セッション確認 10.140
        self.assertTrue(t["first_vk_cold"])
        self.assertEqual((t["reopen_belief"], t["imeon_belief"], t["imeon_other"]), (1, 0, 0))

    def test_reopen_sync_entries_are_distinguished(self):
        t = run(HEAD + base(0) + DONE, REOPEN + IMEON_BELIEF + IMEON_OTHER + VK_COLD)["trials"][0]
        self.assertEqual((t["reopen_belief"], t["imeon_belief"], t["imeon_other"]), (1, 1, 1))

    def test_stuck_offcold_fails(self):
        r = run(HEAD + base(0) + DONE, CLEAN + STUCK)
        self.assertEqual(r["verdict"], "FAIL")
        self.assertIn("stuck=1", r["trials"][0]["why"])

    def test_stale_confirm_escape_in_real_format_fails(self):
        r = run(HEAD + base(0) + DONE, CLEAN + STALE_ESC)
        self.assertEqual(r["verdict"], "FAIL")
        self.assertIn("stale_escape=1", r["trials"][0]["why"])

    def test_stale_confirm_without_escape_is_not_counted(self):
        self.assertEqual(run(HEAD + base(0) + DONE, CLEAN + STALE_NOESC)["verdict"], "PASS")

    def test_flush_escape_fails(self):
        r = run(HEAD + base(0) + DONE, CLEAN + FLUSH_ESC)
        self.assertEqual(r["verdict"], "FAIL")
        self.assertIn("flush_escape=1", r["trials"][0]["why"])

    def test_lost_first_char_fails(self):
        self.assertEqual(run(HEAD + base(0, typed_text="") + DONE, CLEAN)["verdict"], "FAIL")

    def test_real_ime_closed_after_on_fails(self):
        r = run(HEAD + base(0, open_after=False) + DONE, CLEAN)
        self.assertEqual(r["verdict"], "FAIL")
        self.assertIn("閉じている", r["trials"][0]["why"])

    def test_events_outside_windows_are_ignored(self):
        early = P + "05.000000Z  WARN gji_on_event: [gji-fsm] StartComposition while engine off — ignored\n"
        late = P + "20.000000Z  WARN gji_on_event: [gji-fsm] StartComposition while engine off — ignored\n"
        self.assertEqual(run(HEAD + base(0) + DONE, early + CLEAN + late)["verdict"], "PASS")

    def test_pre_window_stuck_is_fail_not_invalid(self):
        # 試行の先頭の開き直し〜OFF 押下の窓の固着。OFF 前の語が崩れていても invalid に逃がさない(review M5)。
        pre_stuck = P + "08.500000Z  WARN gji_on_event: [gji-fsm] StartComposition while engine off — ignored\n"
        r = run(HEAD + base(0, pre_ok=False) + DONE, pre_stuck + CLEAN)
        self.assertEqual(r["verdict"], "FAIL")
        self.assertIn("OFF 前の窓", r["trials"][0]["why"])

    def test_pre_word_not_ok_without_evidence_is_invalid(self):
        self.assertEqual(run(HEAD + base(0, pre_ok=False) + DONE, CLEAN)["verdict"], "INVALID")

    def test_physical_off_not_effective_is_invalid(self):
        r = run(HEAD + base(0, open_before_on=True) + DONE, CLEAN)
        self.assertEqual(r["verdict"], "INVALID")
        self.assertIn("物理 OFF", r["trials"][0]["why"])

    def test_unicode_injection_is_blind_and_excluded(self):
        r = run(HEAD + base(0) + DONE, CLEAN + UNICODE)
        self.assertEqual(r["trials"][0]["status"], "blind")
        self.assertEqual(r["verdict"], "INVALID")  # 有効試行が 0
        mixed = run(HEAD + base(0) + base(1, on="12:00:20.000", press="12:00:20.010", typed="12:00:21.500", off="12:00:19.400", trial="12:00:18.000")
                    + base(2, on="12:00:30.000", press="12:00:30.010", typed="12:00:31.500", off="12:00:29.400", trial="12:00:28.000") + DONE,
                    CLEAN + UNICODE.replace("10.060", "20.060"))
        self.assertEqual(mixed["counts"]["blind"], 1)

    def test_require_cold(self):
        self.assertEqual(run(HEAD + base(0) + DONE, REOPEN + VK_WARM, require_cold=False)["verdict"], "PASS")
        warm = run(HEAD + base(0) + DONE, REOPEN + VK_WARM, require_cold=True)
        self.assertEqual(warm["verdict"], "FAIL")
        self.assertIn("warm 経路", warm["trials"][0]["why"])
        none = run(HEAD + base(0) + DONE, REOPEN, require_cold=True)
        self.assertEqual(none["verdict"], "FAIL")
        self.assertIn("[vk-send] が無い", none["trials"][0]["why"])
        self.assertEqual(run(HEAD + base(0) + DONE, CLEAN, require_cold=True)["verdict"], "PASS")

    def test_require_sync(self):
        # 修正を外したビルド(a8)は Reopen/ImeOn(BeliefSync) が出ないが、テキストも cold 経路も同じ → --require-sync だけが検出できる。
        nofix = VK_COLD + CONFIRM + IMEON_OTHER
        self.assertEqual(run(HEAD + base(0) + DONE, nofix, require_cold=True)["verdict"], "PASS")
        bad = run(HEAD + base(0) + DONE, nofix, require_cold=True, require_sync=True)
        self.assertEqual(bad["verdict"], "FAIL")
        self.assertIn("GjiFsm 同期", bad["trials"][0]["why"])
        self.assertEqual(run(HEAD + base(0) + DONE, CLEAN, require_cold=True, require_sync=True)["verdict"], "PASS")
        self.assertEqual(run(HEAD + base(0) + DONE, VK_COLD + IMEON_BELIEF, require_sync=True)["verdict"], "PASS")

    def test_abort_and_missing_done_are_invalid(self):
        self.assertEqual(run(HEAD + base(0), CLEAN)["verdict"], "INVALID")
        self.assertEqual(run(HEAD + [{"type": "abort", "reason": "x"}], CLEAN)["verdict"], "INVALID")

    def test_main_exit_codes_and_flags(self):
        with tempfile.TemporaryDirectory() as d:
            p, a, j = os.path.join(d, "ts.log"), os.path.join(d, "awase.log"), os.path.join(d, "o.json")
            open(p, "w", encoding="utf-8").write(ts_log(HEAD + base(0) + DONE))
            open(a, "w", encoding="utf-8").write(REOPEN + VK_WARM)
            self.assertEqual(cr.main(["--json", j, p, a]), 0)
            self.assertEqual(json.load(open(j, encoding="utf-8"))["verdict"], "PASS")
            self.assertEqual(cr.main(["--require-cold", p, a]), 1)
            self.assertEqual(cr.main([p]), 2)


if __name__ == "__main__":
    unittest.main()
