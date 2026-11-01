from ldm.shadow_report import parse, summarize

L1 = ('2026-11-01T05:33:00.609339Z DEBUG awase::journal: [lang-detect] seq=78320 elapsed_ms=1 trigger="idle" verdict="not-japanese" '
      'stop="decided" presses=6 llr=-14.61 llr_steps=6 idle_ms=13675 duration_ms=660 profile="tsf-native" belief_open=true '
      'action="half-width-alnum-entered" discarded=0 prev_verdict="undecided" prev_elapsed_ms=1 belief_basis="derive-high" '
      'lr_llr=Some(-2.14) lr_not_japanese_at=None')
L2 = ('x [lang-detect] seq=2 trigger="idle" verdict="undecided" stop="excluded-key" presses=3 llr=-9.5 llr_steps=3 idle_ms=1 '
      'duration_ms=1 profile="standard" belief_open=true action="none" discarded=0 belief_basis="derive-high" '
      'lr_llr=Some(-6.2) lr_not_japanese_at=Some(3)')
OLD = ('x [lang-detect] seq=3 trigger="idle" verdict="undecided" stop="excluded-key" presses=1 llr=0.2 action="none" belief_basis="x"')


def test_parse_and_summarize():
    rows = list(parse([L1, L2, OLD]))
    assert len(rows) == 2  # shadow の無い行は数えない
    assert rows[0]["lr_at"] is None and rows[1]["lr_at"] == 3
    text = summarize(rows)
    assert "現在が英数に入った(action != none): 1 件、うち shadow は確定しなかった: 1 件" in text
    assert "現在は確定せず shadow が英数と確定した: 1 件" in text
