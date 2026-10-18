"""effect_learning.parse_snap の、記録の混入の印([混入: …])の扱いを確かめる。

ime_key_matrix_spike.rs は、+Nms のスナップショットを取るより前に後続のキーが押されていた行の末尾に
`[混入: 次の押下が…]` を付ける。その行は『このキーの結果』ではないので、parse_snap は観測不能(None)として返す。
"""
import unittest

import effect_learning as e

LINE = '    +400ms: A(open=1 conv=0x19) B(open=1 conv=0x19) T(open=1 conv=0x19) G(open=0 conv=0x00) comp="" tail=""'


class MixedMarker(unittest.TestCase):
    def test_plain_line_is_parsed(self):
        self.assertIsNotNone(e.parse_snap(LINE))

    def test_line_with_mixed_marker_is_unobservable(self):
        marked = LINE + " [混入: 次の押下が1800ms後に入っており、この時点の記録は使えない]"
        self.assertIsNone(e.parse_snap(marked))

    def test_marker_only_changes_the_marked_line(self):
        marked = LINE + " [混入: 次の押下が1800ms後に入っており、この時点の記録は使えない]"
        self.assertEqual(e.parse_snap(LINE), e.parse_snap(LINE.replace("+400ms", "+100ms")))
        self.assertIsNone(e.parse_snap(marked))


if __name__ == "__main__":
    unittest.main()
