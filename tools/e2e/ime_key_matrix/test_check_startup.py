import unittest
import check_startup as cs

def recs(initial="on", ok=True, idle=None, done=True, form="edit", no_awase=False, open_before=None, text=None):
    typed={"type":"startup_typed","initial":initial,"ok":ok,"open_after_idle":idle,"expect":"か","raw_char":"w"}
    if open_before is not None: typed["real_ime_open_before_type"]=open_before
    if text is not None: typed["text"]=text
    r=[{"type":"config","mode":"startup","form":form,"ime":"gji","no_awase":no_awase},
       {"type":"startup_pre","initial":initial}, typed]
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

class Variants(unittest.TestCase):
    def chrome(self, **kw): return recs(form="chromepage", **kw)
    def rows(self): return logs()[:1]+logs()[2:]

    def test_noawase_observes_ka_without_awase_log(self):
        r=cs.analyze(self.chrome(no_awase=True, ok=False, text="ka", open_before=False), [])
        self.assertEqual((r["verdict"],r["text_class"],r["open_before"]),("OBSERVE","ka",False))
    def test_noawase_observes_kana(self):
        r=cs.analyze(self.chrome(no_awase=True, text="か", open_before=True), [])
        self.assertEqual((r["verdict"],r["text_class"]),("OBSERVE","kana"))
    def test_noawase_still_invalid_without_done(self):
        self.assertEqual(cs.analyze(self.chrome(no_awase=True, done=False), [])["verdict"],"INVALID")
    def test_gate_closed_is_invalid(self):
        r=cs.analyze(self.chrome(ok=False, text="ka", open_before=False), self.rows(), gate=True)
        self.assertEqual(r["verdict"],"INVALID"); self.assertIn("閉", r["invalid"][0])
    def test_gate_unreadable_is_invalid(self):
        self.assertEqual(cs.analyze(self.chrome(open_before=None), self.rows(), gate=True)["verdict"],"INVALID")
    def test_gate_open_pass(self):
        self.assertEqual(cs.analyze(self.chrome(text="か", open_before=True), self.rows(), gate=True)["verdict"],"PASS")
    def test_gate_open_but_ka_fails(self):
        r=cs.analyze(self.chrome(ok=False, text="ka", open_before=True), self.rows(), gate=True)
        self.assertEqual((r["verdict"],r["text_class"]),("FAIL","ka"))
    def test_without_gate_closed_and_ka_still_fails(self):
        r=cs.analyze(self.chrome(ok=False, text="ka", open_before=False), self.rows())
        self.assertEqual((r["verdict"],r["open_before"]),("FAIL",False))
    def test_text_class_raw_when_engine_stopped(self):
        r=cs.analyze(self.chrome(ok=False, text="w", open_before=False), self.rows())
        self.assertEqual(r["text_class"],"raw")
    def test_evidence_counts(self):
        extra=[(1.5,"[msime-ready] ok\n"),(1.6,"send_keys: mode=Vk n=2\n"),(1.7,"literal detect x\n"),(1.8,"[msime-ready] again\n")]
        r=cs.analyze(self.chrome(text="か", open_before=True), self.rows()+extra, with_evidence=True)
        self.assertEqual([r["evidence"][k]["count"] for k in ("msime_ready","send_keys_vk","literal_detect")],[2,1,1])
    def test_summary_line_has_open_and_class(self):
        line=cs.summary_line(cs.analyze(self.chrome(ok=False, text="ka", open_before=False), self.rows()))
        self.assertIn("open_before=closed text_class=ka", line)

if __name__ == "__main__": unittest.main()
