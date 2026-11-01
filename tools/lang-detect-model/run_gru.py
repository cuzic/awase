"""GRU と LR の比較(同じ test)。

    uv run --group model python -X faulthandler -u run_gru.py [--threads N] [--epochs N]

開発機の仮想 CPU には AVX2 と FMA が無く、torch の GRU が SIGILL で落ちる(制限しても `rnn.py` の forward で落ちた)。
AVX2 のある実機(dragonflyg4 など)で走らせる。
"""
import argparse
import os
import time



def _lacks_avx2() -> bool:
    try:
        return "avx2" not in open("/proc/cpuinfo", encoding="utf-8").read()
    except OSError:
        return False  # Linux 以外は制限しない


if _lacks_avx2():
    os.environ.setdefault("ATEN_CPU_CAPABILITY", "default")
    os.environ.setdefault("ONEDNN_MAX_CPU_ISA", "AVX")
    os.environ.setdefault("MKL_ENABLE_INSTRUCTIONS", "AVX")

import numpy as np  # noqa: E402
import torch  # noqa: E402

from ldm import gru  # noqa: E402
from ldm.data import load  # noqa: E402
from ldm.evaluate import sweep, tpr_at_fpr  # noqa: E402

ap = argparse.ArgumentParser()
ap.add_argument("--threads", type=int, default=4)
ap.add_argument("--epochs", type=int, default=3)
ap.add_argument("--hidden", type=int, default=32)
a = ap.parse_args()
torch.set_num_threads(a.threads)

ep = load()
tr, dv, te = (ep.select(ep.split == i) for i in range(3))
THS = np.concatenate([np.arange(0.5, 20, 0.25), np.arange(20, 60, 2)])
t0 = time.time()
m = gru.train(tr, dv, epochs=a.epochs, hidden=a.hidden, log=lambda s: print("  " + s, f"({time.time() - t0:.0f}s)", flush=True))
print(f"GRU trained in {time.time() - t0:.0f}s", flush=True)
table = sweep(te, gru.increments_of(m, te), THS)
print(f"\n== GRU(hidden {a.hidden}, {a.epochs} エポック) test(全元)")
for tgt in [0.0025, 0.005, 0.01]:
    r = tpr_at_fpr(table, tgt)
    print(f"  fpr_ja<={tgt:.2%}: th={r['threshold']:.2f} ja(wiki)={r['fpr_ja_wiki']:.2%} ja(tatoeba)={r['fpr_ja_tatoeba']:.2%} | tpr en={r['tpr_en']:.1%}({r['press_en']:.1f}打) other={r['tpr_other']:.1%} code={r['tpr_code']:.1%}", flush=True)
