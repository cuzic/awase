"""追加式の識別モデル(ロジスティック回帰): 打鍵ごとの特徴の重みの和 = 累積の log-odds(日本語)。

特徴は、キーの 1 連・2 連・3 連(直前の文脈に **開始記号**を使う: 文頭・語頭 = BOS、語の途中 = MID)。
検出器の `llr += 増分` にそのまま差し替えられる形(増分 = その打鍵の特徴の重みの和)。
"""
from __future__ import annotations

import numpy as np
import torch
from torch import nn

from .data import N_KEYS, PAD, Episodes

BOS, MID = N_KEYS, N_KEYS + 1
N_CTX = N_KEYS + 2


def feature_ids(keys: np.ndarray, start: np.ndarray, use_bos: bool = True):
    """打鍵ごとの 1 連・2 連・3 連の ID。keys: [N, T] (PAD で詰め。長さの外は重みを 0 にするので値は何でもよい)。

    文脈の語彙: キー 0..46、開始記号 BOS(文頭・語頭)と MID(語の途中)。`use_bos=False` のときは開始を『文脈なし』1 つにする。
    """
    n, t = keys.shape
    kk = np.where(keys == PAD, 0, keys).astype(np.int64)
    if use_bos:
        st = np.where(start == 2, MID, BOS).astype(np.int64)
        n_ctx = N_CTX
    else:
        st = np.full(n, N_KEYS, dtype=np.int64)
        n_ctx = N_KEYS + 1
    ctx1 = np.empty_like(kk)
    ctx2 = np.empty_like(kk)
    ctx1[:, 0] = st
    ctx2[:, 0] = st
    if t > 1:
        ctx1[:, 1:] = kk[:, :-1]
        ctx2[:, 1] = st
    if t > 2:
        ctx2[:, 2:] = kk[:, :-2]
    uni = kk
    bi = ctx1 * N_KEYS + kk
    tri = (ctx2 * n_ctx + ctx1) * N_KEYS + kk
    return uni, bi, tri, n_ctx


class AdditiveLR(nn.Module):
    def __init__(self, orders=(1, 2, 3), use_bos=True):
        super().__init__()
        n_ctx = N_CTX if use_bos else N_KEYS + 1
        self.orders, self.use_bos = orders, use_bos
        self.uni = nn.Embedding(N_KEYS, 1)
        self.bi = nn.Embedding(n_ctx * N_KEYS, 1)
        self.tri = nn.Embedding(n_ctx * n_ctx * N_KEYS, 1)
        for e in (self.uni, self.bi, self.tri):
            nn.init.zeros_(e.weight)
        self.bias = nn.Parameter(torch.zeros(1))

    def increments(self, uni, bi, tri):
        inc = torch.zeros(uni.shape, dtype=torch.float32)
        if 1 in self.orders:
            inc = inc + self.uni(uni).squeeze(-1)
        if 2 in self.orders:
            inc = inc + self.bi(bi).squeeze(-1)
        if 3 in self.orders:
            inc = inc + self.tri(tri).squeeze(-1)
        return inc

    def forward(self, uni, bi, tri, length):
        inc = self.increments(uni, bi, tri)
        mask = (torch.arange(inc.shape[1])[None, :] < length[:, None]).float()
        return torch.cumsum(inc * mask, dim=1) + self.bias, mask


def class_weights(label: np.ndarray, mass: dict[int, float]) -> np.ndarray:
    """ラベルごとに『合計の重み』が mass になるようにする。"""
    w = np.zeros(len(label), dtype=np.float64)
    for li, m in mass.items():
        sel = label == li
        if sel.any():
            w[sel] = m / sel.sum()
    return w


def train(ep: Episodes, dev: Episodes, orders=(1, 2, 3), use_bos=True, epochs=12, lr=0.05, l2=1e-6,
          mass=None, batch=8192, seed=0, log=print) -> AdditiveLR:
    torch.manual_seed(seed)
    mass = mass or {0: 0.5, 1: 0.2, 2: 0.15, 3: 0.15}
    model = AdditiveLR(orders, use_bos)
    opt = torch.optim.Adam(model.parameters(), lr=lr)
    ids = feature_ids(ep.keys, ep.start, use_bos)
    tensors = [torch.from_numpy(a) for a in ids[:3]]
    length = torch.from_numpy(ep.length.astype(np.int64))
    y = torch.from_numpy((ep.label == 0).astype(np.float32))
    w = torch.from_numpy(class_weights(ep.label, mass).astype(np.float32) * len(ep) / 1.0)
    dev_ids = feature_ids(dev.keys, dev.start, use_bos)
    dev_t = [torch.from_numpy(a) for a in dev_ids[:3]]
    dev_len = torch.from_numpy(dev.length.astype(np.int64))
    dev_y = torch.from_numpy((dev.label == 0).astype(np.float32))
    dev_w = torch.from_numpy(class_weights(dev.label, mass).astype(np.float32) * len(dev))
    best, best_state = float("inf"), None
    n = len(ep)
    for epoch in range(epochs):
        perm = torch.randperm(n)
        tot = 0.0
        for i in range(0, n, batch):
            idx = perm[i : i + batch]
            logit, mask = model(*(t[idx] for t in tensors), length[idx])
            per = nn.functional.binary_cross_entropy_with_logits(logit, y[idx][:, None].expand_as(logit), reduction="none")
            loss_ep = (per * mask).sum(1) / length[idx].clamp(min=1)  # エピソードの重みを各打鍵に等分
            loss = (loss_ep * w[idx]).sum() / w[idx].sum()
            reg = l2 * sum((p**2).sum() for p in model.parameters())
            opt.zero_grad()
            (loss + reg).backward()
            opt.step()
            tot += float(loss) * len(idx)
        with torch.no_grad():
            logit, mask = model(*dev_t, dev_len)
            per = nn.functional.binary_cross_entropy_with_logits(logit, dev_y[:, None].expand_as(logit), reduction="none")
            dl = ((per * mask).sum(1) / dev_len.clamp(min=1) * dev_w).sum() / dev_w.sum()
        log(f"epoch {epoch + 1}: train {tot / n:.4f} dev {float(dl):.4f}")
        if float(dl) < best:
            best, best_state = float(dl), {k: v.clone() for k, v in model.state_dict().items()}
    model.load_state_dict(best_state)
    return model


def increments_of(model: AdditiveLR, ep: Episodes) -> np.ndarray:
    ids = feature_ids(ep.keys, ep.start, model.use_bos)
    with torch.no_grad():
        inc = model.increments(*(torch.from_numpy(a) for a in ids[:3])).numpy()
    mask = np.arange(inc.shape[1])[None, :] < ep.length[:, None]
    return np.where(mask, inc, 0.0)
