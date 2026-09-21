#!/usr/bin/env python3
"""前回の格子の表(grid-tables/*.json)から、結果が割れたセル(非決定)の一覧を作る。
--grid-adaptive の2パス目(--grid-retry-file)が、ここに載ったセルだけを再試行する。

  gen_grid_nondet.py grid-tables/atok.json > grid-tables/nondet-atok.txt
"""
import json
import sys

d = json.load(open(sys.argv[1], encoding="utf-8"))
print("# 前回の表で結果が割れたセル(state|key)。gen_grid_nondet.py で生成")
for cell, dist in sorted(d.items()):
    if len(dist) > 1:
        print(cell)
