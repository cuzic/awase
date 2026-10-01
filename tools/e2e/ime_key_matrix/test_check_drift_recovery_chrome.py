import unittest
import check_drift_recovery_chrome as c
class Chrome(unittest.TestCase):
    def test_not_observed(self): self.assertEqual(c.analyze(["RESULT FAIL: x"],[])["verdict"],"NOT_OBSERVED")
    def test_not_recovered(self): self.assertEqual(c.analyze(["RESULT FAIL: x"],["ObserverReported"])["verdict"],"NOT_RECOVERED")
    def test_recovered(self): self.assertEqual(c.analyze(["RESULT PASS: x"],["[drift] correction: x"])["verdict"],"RECOVERED")
    def test_invalid(self): self.assertEqual(c.analyze(["RESULT INVALID: x"],[])["verdict"],"INVALID")
