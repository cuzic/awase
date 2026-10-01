import unittest
import check_startup as cs

def recs(initial="on", ok=True, idle=None, done=True, form="edit"):
    r=[{"type":"config","mode":"startup","form":form,"ime":"gji"},
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
    def test_late_first_key_is_invalid_not_fail(self): self.assertEqual(cs.analyze(recs(),logs(engine=2.001))["verdict"],"INVALID")
    def test_first_key_within_2s_passes(self): self.assertEqual(cs.analyze(recs(),logs(engine=1.5))["verdict"],"PASS")
    def test_drift_fails(self): self.assertEqual(cs.analyze(recs(),logs(extra=[(2,"[drift] correction\n")]))["verdict"],"FAIL")
    def test_off_reinit_fails(self):
        self.assertEqual(cs.analyze(recs("off",idle=False),logs("false",None,[(2,"[ime-io] actuation SendInput kind=kanji_marker vk=[1A, 16]\n")]))["verdict"],"FAIL")
    def test_missing_done_invalid(self): self.assertEqual(cs.analyze(recs(done=False),logs())["verdict"],"INVALID")

    def test_edit_without_align_fails(self):
        self.assertEqual(cs.analyze(recs(), logs()[:1]+logs()[2:])["verdict"],"FAIL")
    def test_chrome_on_without_align_passes(self):
        r=cs.analyze(recs(form="chromepage"), logs()[:1]+logs()[2:])
        self.assertEqual((r["verdict"],r["align"]),("PASS",[]))
    def test_chrome_off_without_align_passes(self):
        self.assertEqual(cs.analyze(recs("off",idle=False,form="chromepage"), logs()[:1])["verdict"],"PASS")
    def test_chrome_off_reinit_still_fails(self):
        rows=logs()[:1]+[(2,"[ime-io] actuation SendInput kind=kanji_marker vk=[1A, 16]\n")]
        self.assertEqual(cs.analyze(recs("off",idle=False,form="chromepage"),rows)["verdict"],"FAIL")
    def test_chrome_drift_still_fails(self):
        self.assertEqual(cs.analyze(recs(form="chromepage"),logs()[:1]+logs()[2:]+[(2,"[drift] correction\n")])["verdict"],"FAIL")

if __name__ == "__main__": unittest.main()
