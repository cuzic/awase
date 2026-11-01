"""基準(今の判定器の移植)と追加式 LR を、独立した dev/test で比べる。

    uv run --group model python run_ablation.py
"""
import sys
import time

import numpy as np

from ldm.baseline import Baseline
from ldm.data import LABELS, SOURCES, load
from ldm.evaluate import sweep, tpr_at_fpr
from ldm.lr import increments_of, train

t0 = time.time()
ep = load()
print(f"loaded {len(ep)} episodes in {time.time() - t0:.1f}s", flush=True)
train_ep, dev_ep, test_ep = (ep.select(ep.split == i) for i in range(3))
print("train/dev/test:", len(train_ep), len(dev_ep), len(test_ep), flush=True)

THS = np.concatenate([np.arange(1, 20, 0.5), np.arange(20, 60, 2)])
TARGETS = [0.0025, 0.005, 0.01, 0.02]


def baseline_inc(e):
    b = Baseline()
    inc = np.zeros(e.keys.shape, dtype=np.float32)
    from ldm.data import KEYS
    for i in range(len(e)):
        ks = "".join(KEYS[k] for k in e.keys[i, : e.length[i]])
        inc[i, : e.length[i]] = b.increments(ks)
    return inc


def report(name, e, inc):
    table = sweep(e, inc, THS)
    print(f"\n== {name}")
    for tgt in TARGETS:
        r = tpr_at_fpr(table, tgt)
        if not r:
            print(f"  fpr<={tgt:.4f}: (届かない)")
            continue
        print(f"  fpr_ja<={tgt:.2%}: th={r['threshold']:.1f} ja(wiki)={r.get('fpr_ja_wiki', float('nan')):.2%} ja(tatoeba)={r.get('fpr_ja_tatoeba', float('nan')):.2%} "
              f"| tpr en={r['tpr_en']:.1%}({r['press_en']:.1f}打) other={r['tpr_other']:.1%} code={r['tpr_code']:.1%}")
    return table


which = sys.argv[1:] or ["baseline", "lr"]
sub = test_ep
if "baseline" in which:
    t0 = time.time()
    report("baseline(今の判定器の移植) test", sub, baseline_inc(sub))
    print(f"  ({time.time() - t0:.0f}s)")
if "lr" in which:
    for name, kw in [("uni", dict(orders=(1,), use_bos=False)), ("uni+bi", dict(orders=(1, 2), use_bos=False)),
                     ("uni+bi+tri", dict(orders=(1, 2, 3), use_bos=False)), ("uni+bi+tri+BOS", dict(orders=(1, 2, 3), use_bos=True))]:
        t0 = time.time()
        m = train(train_ep, dev_ep, epochs=10, log=lambda s: None, **kw)
        report(f"LR {name} test", sub, increments_of(m, sub))
        print(f"  ({time.time() - t0:.0f}s)", flush=True)
