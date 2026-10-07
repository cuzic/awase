"""check_*.py が読む awase.log の行の「固定の断片」が、Rust 側のソースに残っているかを検査する(ADR-226 候補 E の小さな試行)。

背景: チェッカーは awase.log のテキスト行を正規表現で数える。Rust 側のログ文言が変わると、チェッカーは黙って 0 件になり
「異常なし」と判定する(実例: d47645eb、check_reopen の検出が実ログの書式と一致していなかった)。
この検査は、Rust のソースに断片が存在することだけを確かめる(PR の smoke で毎回走る・実機不要)。

確かめないこと(限界):
  - tracing の構造化フィールド名(seq= / outcome= / source= など)。メッセージ文字列だけを見る。
  - 断片が書式引数で組み立てられている場合の中身。固定部分だけを断片にする。
  - 行が実際に出力される経路(到達性)。
anchors を足すときは、チェッカーの正規表現が頼る固定部分を、Rust ソースの文字列リテラルと同じ綴りで書く。
"""
import pathlib
import re
import unittest

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[2]

# (Rust ソースに現れるべき断片, 読むチェッカー)
ANCHORS = [
    ("[gji-fsm] StartComposition while engine off", "check_invariants.py"),
    ("[gji-fsm] StartComposition while engine off", "check_reopen.py"),
    ("stale confirm 検出", "check_invariants.py"),
    ("stale confirm 検出", "check_reopen.py"),
    ("[raw-tsf-literal] flush escape=", "check_reopen.py"),
    ("BeliefSync:", "check_reopen.py"),
    ("send_keys: mode=", "check_reopen.py"),
    ("[vk-send]", "check_reopen.py"),
    ("prepend_f2_warmup=", "check_reopen.py"),
    ("per-VK: 全", "check_reopen.py"),
    ("VK 確認済み → セッション確認", "check_reopen.py"),
    ("[startup-align]", "check_startup.py"),
    ("[drift] correction", "check_startup.py"),
    ("[drift] correction", "check_invariants.py"),
    ("[ime-io] actuation SendInput kind=", "check_startup.py"),
    ("Imm32Unavailable entry without trusted cache: 安全デフォルト ON", "check_startup.py"),
    ("[engine-input]", "check_startup.py"),
    ("Keyboard Layout Emulator starting", "check_startup.py"),
    ("[msime-ready]", "check_startup.py"),
    ("literal detect", "check_startup.py"),
    ("explicit_intent=", "check_invariants.py"),
    ("[hook] IME-mode vk=", "check_invariants.py"),
    ("[warrant-shadow]", "check_invariants.py"),
    ("would_have_blocked=", "check_invariants.py"),
    # journal.rs の message(構造化フィールド seq=... はメッセージの後ろに付く)
    ("ime open applied", "check_invariants.py"),
    ("actuation decision", "check_kanji_role.py"),
    ("gji fsm transition", "check_invariants.py"),
    # ADR-235 D1: check_run_validity.py が数える経路・profile の断片(e2e_common.py の正規表現と同じ固定部分)
    ("[stage-observe] observer_poll=", "e2e_common.py"),
    ("[external-change]", "e2e_common.py"),
    ("[focus-scope] bootstrap initial scope:", "check_run_validity.py"),
    ("focus transition", "check_run_validity.py"),
]

SOURCE_DIRS = ("src", "crates")


def _rust_sources():
    text = []
    for d in SOURCE_DIRS:
        for p in (ROOT / d).rglob("*.rs"):
            if "target" in p.parts:
                continue
            try:
                text.append(p.read_text(encoding="utf-8"))
            except OSError:
                pass
    return "\n".join(text)


def _checker_mentions(checker_text, anchor):
    """断片(の先頭の識別しやすい部分)がチェッカーの正規表現/文字列に残っているか。
    正規表現では [ ] が \\[ \\] になる。= の後ろや書式引数は無視して、前方の固定部分だけ比べる。"""
    head = re.split(r"[=:]", anchor)[0].strip()
    if not head:
        return True
    variants = {head, head.replace("[", r"\[").replace("]", r"\]")}
    return any(v in checker_text for v in variants)


class LogAnchors(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.rust = _rust_sources()
        cls.assertTrue = unittest.TestCase.assertTrue

    def test_rust_sources_found(self):
        self.assertGreater(len(self.rust), 100000, "Rust ソースを読めていない(ROOT の解決を確認)")

    def test_every_anchor_exists_in_rust_source(self):
        missing = [(a, c) for a, c in ANCHORS if a not in self.rust]
        self.assertEqual(
            missing, [],
            "チェッカーが読むログの断片が Rust ソースから消えている(黙って 0 件になる恐れ)。"
            "文言を変えたなら、チェッカーの正規表現・testdata・この表を揃えて更新すること: %r" % (missing,))

    def test_every_anchor_is_still_read_by_its_checker(self):
        stale = []
        for a, c in ANCHORS:
            path = HERE / c
            if not path.exists() or not _checker_mentions(path.read_text(encoding="utf-8"), a):
                stale.append((a, c))
        self.assertEqual(
            stale, [],
            "この表の断片をチェッカーが読まなくなっている(表が古い)。表から外すか、チェッカーを確認すること: %r" % (stale,))


if __name__ == "__main__":
    unittest.main()
