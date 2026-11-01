"""比較用の小さな GRU(Python のみ。Rust への移植は LR との差が大きいときだけ検討する)。

入力は開始記号(BOS/MID)に続くキー列。各打鍵の出力 = その時点までの log-odds(日本語)。
追加式でない(累積ではなく状態から直接出す)ので、検出器の `llr += 増分` には差し替えられない。確定の規則は
『出力が -th 以下になったら NotJapanese』で、同じ評価関数に渡すため、増分 = 出力の差分にして返す。
"""
from __future__ import annotations

import numpy as np
import torch
from torch import nn

from .data import N_KEYS, PAD, Episodes
from .lr import BOS, MID, class_weights


class GruNet(nn.Module):
    def __init__(self, hidden: int = 32, emb: int = 16):
        super().__init__()
        self.emb = nn.Embedding(N_KEYS + 2, emb)
        self.gru = nn.GRU(emb, hidden, batch_first=True)
        self.out = nn.Linear(hidden, 1)

    def forward(self, keys, start):
        # 先頭に開始記号を足し、その出力は捨てる
        st = torch.where(start == 2, torch.tensor(MID), torch.tensor(BOS))
        kk = torch.where(keys == PAD, torch.zeros_like(keys), keys)
        x = torch.cat([st[:, None], kk], dim=1)
        h, _ = self.gru(self.emb(x))
        return self.out(h[:, 1:]).squeeze(-1)


def _tensors(ep: Episodes):
    return (torch.from_numpy(ep.keys.astype(np.int64)), torch.from_numpy(ep.start.astype(np.int64)),
            torch.from_numpy(ep.length.astype(np.int64)))


def _loss(logit, y, length, w):
    mask = (torch.arange(logit.shape[1])[None, :] < length[:, None]).float()
    per = nn.functional.binary_cross_entropy_with_logits(logit, y[:, None].expand_as(logit), reduction="none")
    return (((per * mask).sum(1) / length.clamp(min=1)) * w).sum() / w.sum()


def train(ep: Episodes, dev: Episodes, epochs=3, lr=0.01, batch=4096, mass=None, seed=0, hidden=32, log=print) -> GruNet:
    torch.manual_seed(seed)
    mass = mass or {0: 0.5, 1: 0.2, 2: 0.15, 3: 0.15}
    model = GruNet(hidden=hidden)
    opt = torch.optim.Adam(model.parameters(), lr=lr)
    keys, start, length = _tensors(ep)
    y = torch.from_numpy((ep.label == 0).astype(np.float32))
    w = torch.from_numpy((class_weights(ep.label, mass) * len(ep)).astype(np.float32))
    dk, ds, dl = _tensors(dev)
    dy = torch.from_numpy((dev.label == 0).astype(np.float32))
    dw = torch.from_numpy((class_weights(dev.label, mass) * len(dev)).astype(np.float32))
    best, best_state = float("inf"), None
    n = len(ep)
    for epoch in range(epochs):
        perm = torch.randperm(n)
        tot = 0.0
        for i in range(0, n, batch):
            idx = perm[i : i + batch]
            loss = _loss(model(keys[idx], start[idx]), y[idx], length[idx], w[idx])
            opt.zero_grad()
            loss.backward()
            opt.step()
            tot += float(loss.detach()) * len(idx)
        with torch.no_grad():
            parts = [_loss(model(dk[j : j + 16384], ds[j : j + 16384]), dy[j : j + 16384], dl[j : j + 16384], dw[j : j + 16384])
                     * dw[j : j + 16384].sum() for j in range(0, len(dev), 16384)]
            dloss = float(sum(parts) / dw.sum())
        log(f"epoch {epoch + 1}: train {tot / n:.4f} dev {dloss:.4f}")
        if dloss < best:
            best, best_state = dloss, {k: v.clone() for k, v in model.state_dict().items()}
    model.load_state_dict(best_state)
    return model


def increments_of(model: GruNet, ep: Episodes, chunk: int = 16384) -> np.ndarray:
    keys, start, _ = _tensors(ep)
    outs = []
    with torch.no_grad():
        for j in range(0, len(ep), chunk):
            outs.append(model(keys[j : j + chunk], start[j : j + chunk]).numpy())
    out = np.concatenate(outs)
    mask = np.arange(out.shape[1])[None, :] < ep.length[:, None]
    out = np.where(mask, out, 0.0)
    inc = np.diff(out, axis=1, prepend=0.0)
    return np.where(mask, inc, 0.0)
