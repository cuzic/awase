"""最終候補(uni+bi+tri+BOS)の保存と、領域をまたいだ評価。

    uv run --group model python -u run_final.py
"""
import time

import numpy as np
import torch

from ldm.baseline import Baseline
from ldm.common import CACHE
from ldm.data import KEYS, LABELS, SOURCES, load
from ldm.evaluate import sweep, tpr_at_fpr
from ldm.lr import increments_of, train

ep = load()
tr, dv, te = (ep.select(ep.split == i) for i in range(3))
THS = np.concatenate([np.arange(1, 20, 0.5), np.arange(20, 60, 2)])
TARGETS = [0.0025, 0.005, 0.01]


def baseline_inc(e):
    b = Baseline()
    inc = np.zeros(e.keys.shape, dtype=np.float32)
    for i in range(len(e)):
        ks = "".join(KEYS[k] for k in e.keys[i, : e.length[i]])
        inc[i, : e.length[i]] = b.increments(ks)
    return inc


def report(name, e, inc, key="fpr_ja_all"):
    table = sweep(e, inc, THS)
    print(f"\n== {name} (n={len(e)})", flush=True)
    for tgt in TARGETS:
        r = tpr_at_fpr(table, tgt, key)
        if not r:
            print(f"  {key}<={tgt:.2%}: (届かない)")
            continue
        print(f"  {key}<={tgt:.2%}: th={r['threshold']:.1f} ja(wiki)={r.get('fpr_ja_wiki', float('nan')):.2%} ja(tatoeba)={r.get('fpr_ja_tatoeba', float('nan')):.2%}"
              f" | tpr en={r['tpr_en']:.1%}({r['press_en']:.1f}打) other={r['tpr_other']:.1%} code={r['tpr_code']:.1%}", flush=True)


def probe(model, label):
    from ldm.data import Episodes, KEY_INDEX, PAD
    cases = {"ccndie": "ccndie", "hello": "hello", "windows": "windows", "git": "git", "すすめてくた": "ccndie", "こんにちは": "rjuxu"}
    print(f"\n-- {label}: 実機の誤発火などの累積 log-odds(日本語)", flush=True)
    for name, ks in cases.items():
        keys = np.full((1, 12), PAD, dtype=np.int16)
        for i, c in enumerate(ks):
            keys[0, i] = KEY_INDEX[c]
        e = Episodes(keys, np.array([len(ks)], dtype=np.int16), np.zeros(1, dtype=np.int8), np.zeros(1, dtype=np.int8), np.zeros(1, dtype=np.int8), np.zeros(1, dtype=np.int8))
        inc = increments_of(model, e)[0, : len(ks)]
        print(f"  {name} ({ks}): {np.round(np.cumsum(inc), 2).tolist()}", flush=True)


# A: 全部で学習 → test(元別も)
t0 = time.time()
m = train(tr, dv, epochs=6, lr=0.1, batch=16384, log=lambda s: print("  " + s, flush=True))
(CACHE / "models").mkdir(parents=True, exist_ok=True)
torch.save(m.state_dict(), CACHE / "models" / "lr_bos.pt")
print(f"A trained in {time.time() - t0:.0f}s", flush=True)
probe(m, "LR+BOS")
report("LR+BOS test(全元)", te, increments_of(m, te))
for si, src in enumerate(SOURCES[:2]):
    sub = te.select(((te.label == 0) | (te.label == 1)) & (te.source == si))
    report(f"LR+BOS test {src} の ja と en だけ", sub, increments_of(m, sub), key=f"fpr_ja_{src}")
    report(f"baseline test {src} の ja と en だけ", sub, baseline_inc(sub), key=f"fpr_ja_{src}")

# B: 領域をまたぐ(学習 = wiki と repo と other、評価 = tatoeba の ja/en)
cross_tr = tr.select((tr.source == 0) | (tr.source == 2) | (tr.label == 2))
cross_dv = dv.select((dv.source == 0) | (dv.source == 2) | (dv.label == 2))
t0 = time.time()
mb = train(cross_tr, cross_dv, epochs=6, lr=0.1, batch=16384, log=lambda s: print("  " + s, flush=True))
print(f"B trained in {time.time() - t0:.0f}s", flush=True)
sub = te.select(((te.label == 0) | (te.label == 1)) & (te.source == 1))
report("LR+BOS 領域をまたぐ(wiki+repo+otherで学習 → tatoeba の ja/en)", sub, increments_of(mb, sub), key="fpr_ja_tatoeba")
