from ldm.baseline import Baseline


def test_baseline_matches_rust_on_the_misfire():
    # CI のプローブ(#922)が出した Rust の値(`ccndie` = すすめてくた)
    got = Baseline().increments("ccndie")
    want = [-4.95, -2.77, 4.57, -0.90, -6.82, -3.73]
    assert len(got) == 6
    for g, w in zip(got, want):
        assert abs(g - w) < 0.02, (got, want)
    assert abs(sum(got) - (-14.61)) < 0.05
