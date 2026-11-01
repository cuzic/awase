import numpy as np

from ldm.data import KEY_INDEX, PAD
from ldm.evaluate import first_hit
from ldm.lr import BOS, MID, feature_ids


def test_first_hit_stops_at_threshold_and_respects_length():
    inc = np.array([[-3.0, -3.0, -3.0, 0, 0], [-1.0, -1.0, 0, 0, 0]], dtype=np.float32)
    decided, presses = first_hit(inc, np.array([3, 2]), th=5.0)
    assert decided.tolist() == [True, False]
    assert presses.tolist() == [2, 2]


def test_feature_ids_distinguish_word_start_from_mid_word():
    keys = np.full((2, 4), PAD, dtype=np.int16)
    keys[:, 0] = KEY_INDEX["c"]
    keys[:, 1] = KEY_INDEX["c"]
    start = np.array([0, 2])  # sentence / mid
    _uni, bi, tri, _n = feature_ids(keys, start, use_bos=True)
    assert bi[0, 0] != bi[1, 0] and tri[0, 0] != tri[1, 0]  # 先頭の打鍵の文脈が BOS と MID で違う
    assert bi[0, 1] == bi[1, 1]  # 2 打目の 2 連は同じ(直前は c)
    assert tri[0, 1] != tri[1, 1]  # 2 打目の 3 連は、2 つ前が開始記号なので違う
    # use_bos=False では区別しない
    _u, bi0, _t, _n = feature_ids(keys, start, use_bos=False)
    assert bi0[0, 0] == bi0[1, 0]
    assert BOS != MID
