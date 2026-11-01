from ldm.common import GRID, POS, UNSHIFTED, japanese_run, katakana_to_hiragana, latin_key_string, latin_tokens


def test_unshifted_face_matches_nicola():
    # C=す N=め D=て I=く E=た(2026-11-01 の誤発火の列)
    assert UNSHIFTED["す"] == "c" and UNSHIFTED["め"] == "n" and UNSHIFTED["て"] == "d"
    assert UNSHIFTED["く"] == "i" and UNSHIFTED["た"] == "e"
    assert UNSHIFTED["。"] == "q" and UNSHIFTED["、"] == "@" and UNSHIFTED["ん"] == ";"


def test_japanese_run_stops_at_thumb_kana():
    assert japanese_run("すすめてくだ", 0) == "ccndie"[:5]  # く までで、だ(濁点)は親指が要る
    assert japanese_run("すすめてくた", 0) == "ccndie"
    assert japanese_run("すすめてくだ", 2) == "ndi"
    assert japanese_run("ー", 0) == ""


def test_katakana_reading():
    assert katakana_to_hiragana("ススメテクダサイ") == "すすめてください"


def test_grid_positions_are_unique():
    assert len(POS) == sum(len(r) for r in GRID)


def test_latin_tokens_and_keys():
    assert latin_tokens("hello  world\tfoo") == ["hello", "world", "foo"]
    assert latin_key_string("Café") == "cafe"
    assert latin_key_string("a?b") == "ab"
