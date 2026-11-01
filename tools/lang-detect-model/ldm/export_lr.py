"""学習済みの LR(`run_final.py` が保存する `$AWASE_LDM_CACHE/models/lr_bos.pt`)を、Rust が読む表に書き出す。

    uv run --group model python -m ldm.export_lr

出力:
- `data/lang_lr_v1.txt`: int8 に量子化した重み(`scale` を掛けて使う)。形式は `src/lang_detect/lr.rs` の doc。
- `tests/fixtures/lang_lr_vectors.tsv`: Rust との一致を固定する見本(キー列と、量子化後の重みで数えた各打鍵の累積 log-odds)。

量子化しても評価が変わらないことを、同じ test 分割で確かめて表示する。
"""
from __future__ import annotations

import random

import numpy as np
import torch

from .common import CACHE, ROOT
from .data import KEY_INDEX, KEYS, PAD, Episodes, load
from .evaluate import sweep, tpr_at_fpr
from .lr import AdditiveLR, increments_of

ID = "lr-bos-v1"


def to_hex(q: np.ndarray) -> str:
    return "".join(f"{int(v) & 0xFF:02x}" for v in q)


def main() -> None:
    state = torch.load(CACHE / "models" / "lr_bos.pt")
    w = {k: state[f"{k}.weight"].squeeze(-1).numpy() for k in ("uni", "bi", "tri")}
    bias = float(state["bias"])
    scale = float(max(np.abs(a).max() for a in w.values()) / 127.0)
    q = {k: np.clip(np.rint(a / scale), -127, 127).astype(np.int8) for k, a in w.items()}
    nz = {k: int((a != 0).sum()) for k, a in q.items()}
    print(f"scale={scale:.5f} bias={bias:.4f} sizes={ {k: len(a) for k, a in q.items()} } nonzero={nz}")

    out = ROOT / "data" / "lang_lr_v1.txt"
    lines = [
        "# awase: 入力言語の判別の追加式ロジスティック回帰(ADR-280 の後続、shadow)。tools/lang-detect-model/ldm/export_lr.py が生成する。手で編集しない",
        "# 形式: id= 識別子、scale= 重みに掛ける値(int8 → log-odds)、uni=47 / bi=2303(=49*47) / tri=112847(=49*49*47) の int8 を 16 進 2 桁ずつ(2 の補数)",
        "# 特徴: キー k の 1 連 uni[k]、直前の文脈 c1 との 2 連 bi[c1*47+k]、2 つ前の文脈 c2 との 3 連 tri[(c2*49+c1)*47+k]。",
        "#       文脈は キー 0..46、BOS=47(エピソードの先頭 = 文頭・語頭)、MID=48(語の途中。実機は使わない)。打鍵の増分 = 3 つの重みの和 * scale。",
        "# キーの添字: 格子の行を順に並べた JIS の QWERTY(1234567890-^\\ / qwertyuiop@[ / asdfghjkl;:] / zxcvbnm,./)。",
        f"id={ID}",
        f"scale={scale:.8f}",
        f"bias={bias:.6f}",
        f"uni={to_hex(q['uni'])}",
        f"bi={to_hex(q['bi'])}",
        f"tri={to_hex(q['tri'])}",
    ]
    out.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"wrote {out} ({out.stat().st_size} bytes)")

    # 量子化後の重みで評価(元の評価と同じ test)
    deq = AdditiveLR(orders=(1, 2, 3), use_bos=True)
    with torch.no_grad():
        for k in ("uni", "bi", "tri"):
            getattr(deq, k).weight.copy_(torch.from_numpy(q[k].astype(np.float32) * scale)[:, None])
    ep = load()
    te = ep.select(ep.split == 2)
    ths = np.concatenate([np.arange(1, 20, 0.5), np.arange(20, 60, 2)])
    for name, model in [("float32", None), ("int8", deq)]:
        if model is None:
            m = AdditiveLR(orders=(1, 2, 3), use_bos=True)
            m.load_state_dict(state)
            model = m
        t = sweep(te, increments_of(model, te), ths)
        for tgt in (0.005, 0.01):
            r = tpr_at_fpr(t, tgt)
            print(f"  {name:8} fpr<={tgt:.2%}: th={r['threshold']:.1f} en={r['tpr_en']:.1%} other={r['tpr_other']:.1%} code={r['tpr_code']:.1%}")

    # Rust との一致を固定する見本(すべて BOS 始まり)
    rng = random.Random(1101)
    samples = ["ccndie", "hello", "windows", "git", "rjuxu", "q", "a"]
    pool = [KEYS[k] for k in range(len(KEYS))]
    rows = te.select(np.arange(len(te)) % max(1, len(te) // 60) == 0)
    for i in range(len(rows)):
        samples.append("".join(KEYS[k] for k in rows.keys[i, : rows.length[i]]))
    while len(samples) < 90:
        samples.append("".join(rng.choice(pool) for _ in range(rng.randint(1, 12))))
    samples = [s for s in dict.fromkeys(samples) if s][:90]
    vec = ["# tools/lang-detect-model/ldm/export_lr.py が生成する。キー列<TAB>各打鍵の累積 log-odds(量子化後の重み、BOS 始まり、bias なし)"]
    for s in samples:
        keys = np.full((1, 12), PAD, dtype=np.int16)
        for i, c in enumerate(s[:12]):
            keys[0, i] = KEY_INDEX[c]
        e = Episodes(keys, np.array([min(len(s), 12)], dtype=np.int16), np.zeros(1, dtype=np.int8), np.zeros(1, dtype=np.int8),
                     np.zeros(1, dtype=np.int8), np.zeros(1, dtype=np.int8))
        cum = np.cumsum(increments_of(deq, e)[0, : min(len(s), 12)])
        vec.append(s[:12] + "\t" + ",".join(f"{v:.4f}" for v in cum))
    vp = ROOT / "tests" / "fixtures" / "lang_lr_vectors.tsv"
    vp.write_text("\n".join(vec) + "\n", encoding="utf-8")
    print(f"wrote {vp} ({len(vec) - 1} vectors)")


if __name__ == "__main__":
    main()
