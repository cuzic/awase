import unittest
import check_startup as cs

def recs(initial="on", ok=True, idle=None, done=True):
    r=[{"type":"config","mode":"startup","form":"edit","ime":"gji"},
       {"type":"startup_pre","initial":initial},
       {"type":"startup_typed","initial":initial,"ok":ok,"open_after_idle":idle}]
    if done: r.append({"type":"done"})
    return r

def logs(desired="true", engine=.4, extra=()):
    rows=[(1.0,"Keyboard Layout Emulator starting...\n"),(1.2,f"[startup-align] x desired={desired}\n")]
    if engine is not None: rows.append((1.0+engine,"[engine-input] vk=0x41 KeyDown\n"))
    return rows+list(extra)

class Startup(unittest.TestCase):
    def test_on_pass(self): self.assertEqual(cs.analyze(recs(), logs())["verdict"], "PASS")
    def test_off_pass_and_observation_only(self):
        r=cs.analyze(recs("off", idle=False), logs("false", None, [(1.3,"Imm32Unavailable entry without trusted cache: 安全デフォルト ON\n")]))
        self.assertEqual((r["verdict"],r["observe"]),("PASS",1))
    def test_late_first_key_fails(self): self.assertEqual(cs.analyze(recs(),logs(engine=1.001))["verdict"],"FAIL")
    def test_drift_fails(self): self.assertEqual(cs.analyze(recs(),logs(extra=[(2,"[drift] correction\n")]))["verdict"],"FAIL")
    def test_off_reinit_fails(self):
        self.assertEqual(cs.analyze(recs("off",idle=False),logs("false",None,[(2,"[ime-io] actuation SendInput kind=kanji_marker vk=[1A, 16]\n")]))["verdict"],"FAIL")
    def test_missing_done_invalid(self): self.assertEqual(cs.analyze(recs(done=False),logs())["verdict"],"INVALID")

if __name__ == "__main__": unittest.main()
