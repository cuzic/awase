#!/usr/bin/env python3
"""e2e-ime.yml の plan(構成定義)の健全性テスト。実行: python3 -m unittest discover -s tools/e2e/ime_key_matrix -p 'test_*.py'

observe 構成が期限なしで増え続けないこと(observe_audit.py の規約)を検査する。
"""
import datetime
import unittest

import observe_audit as oa


class PlanHealth(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.configs = oa.load_configs()
        cls.gf = oa.read_grandfathered()

    def test_names_are_unique(self):
        names = [c["name"] for c in self.configs]
        dup = sorted({n for n in names if names.count(n) > 1})
        self.assertEqual(dup, [], f"構成名が重複: {dup}")

    def test_expect_values_are_known(self):
        bad = sorted({c["expect"] for c in self.configs} - {"pass", "fail", "observe"})
        self.assertEqual(bad, [])

    def test_new_observe_has_deadline(self):
        missing = [c["name"] for c in self.configs
                   if c["expect"] == "observe" and not c.get("until") and c["name"] not in self.gf]
        self.assertEqual(missing, [],
                         "新しい observe 構成には cfg(..., until='YYYY-MM-DD') を付ける(期限までに pass/fail/削除を決める)。"
                         "既存の observe を観測のまま残すなら observe_grandfathered.txt ではなく期限を付けること。")

    def test_deadline_format_and_not_expired(self):
        today = datetime.date.today()
        expired = []
        for c in self.configs:
            u = c.get("until")
            if not u:
                continue
            try:
                d = datetime.date.fromisoformat(u)
            except ValueError:
                self.fail(f"{c['name']}: until='{u}' は YYYY-MM-DD ではない")
            if c["expect"] != "observe":
                self.fail(f"{c['name']}: until は expect=observe の構成にだけ付ける(expect={c['expect']})")
            if d < today:
                expired.append(f"{c['name']}(until={u})")
        self.assertEqual(expired, [], "期限切れの observe: pass(安定)/ fail(再現固定、BUG と対応)/ 削除のどれかに決める")

    def test_grandfathered_only_shrinks(self):
        obs = set(oa.observe_names(self.configs))
        stale = sorted(self.gf - obs)
        self.assertEqual(stale, [], "observe でなくなった(昇格・削除した)構成が observe_grandfathered.txt に残っている。リストから外す")
        dated = sorted(n for n in self.gf if any(c["name"] == n and c.get("until") for c in self.configs))
        self.assertEqual(dated, [], "期限(until)を付けた構成は observe_grandfathered.txt から外す")


if __name__ == "__main__":
    unittest.main()
