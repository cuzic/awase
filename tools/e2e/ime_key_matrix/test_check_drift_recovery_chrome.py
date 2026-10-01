import unittest
import check_drift_recovery_chrome as c
class Chrome(unittest.TestCase):
    def test_not_observed(self): self.assertEqual(c.analyze(["RESULT FAIL: x"],[])["verdict"],"NOT_OBSERVED")
    def test_not_recovered(self): self.assertEqual(c.analyze(["RESULT FAIL: x"],["ObserverReported"])["verdict"],"NOT_RECOVERED")
    def test_recovered(self): self.assertEqual(c.analyze(["RESULT PASS: x"],["[drift] correction: x"])["verdict"],"RECOVERED")
    def test_invalid(self): self.assertEqual(c.analyze(["RESULT INVALID: x"],[])["verdict"],"INVALID")

M = "[engine-input] vk=0x1D KeyDown ts=1us mods(c=true s=false a=false w=false) gas_ctrl=true phys_ctrl=true extra=0x5350494B"
M_BAD = M.replace("phys_ctrl=true", "phys_ctrl=false")
CL = "CLOSE_IME method=ctrl_muhenkan open_before=Some(1) set_ret=None open_after=%s"


class ChromeCtrl(unittest.TestCase):
    def test_gap_not_made(self):
        r = c.analyze([CL % "Some(0)", "RESULT PASS: OFF が効き英字(ka) gap=false"], [M])
        self.assertEqual(r["verdict"], "GAP_NOT_MADE")

    def test_corrected(self):
        r = c.analyze([CL % "Some(1)", "RESULT PASS: OFF が効き英字(ka) gap=true"], [M])
        self.assertEqual((r["verdict"], r["gap_made"]), ("CORRECTED", 1))

    def test_not_corrected(self):
        r = c.analyze([CL % "Some(1)", "RESULT FAIL: OFF が効かず 実際=NICOLA gap=true"], [M, "ObserverReported"])
        self.assertEqual(r["verdict"], "NOT_CORRECTED")

    def test_phys_ctrl_false_invalid(self):
        r = c.analyze([CL % "Some(1)", "RESULT FAIL: x gap=true"], [M_BAD])
        self.assertEqual(r["verdict"], "INVALID")

    def test_no_muhenkan_log_invalid(self):
        self.assertEqual(c.analyze([CL % "Some(1)", "RESULT FAIL: x gap=true"], [])["verdict"], "INVALID")
