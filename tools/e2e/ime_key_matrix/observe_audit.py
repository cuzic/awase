#!/usr/bin/env python3
"""e2e-ime.yml の plan スクリプトから構成一覧を取り出し、expect=observe の構成を棚卸しする(標準ライブラリだけ)。

observe は結果を表に出すだけで何も止めないため、判断を先送りした構成が放置されやすい(2026-10-05 時点で observe 78 / pass 26)。
ここでは次を決めている(test_e2e_plan.py が検査する):
  - 新しい observe 構成は cfg(..., until='YYYY-MM-DD') を必ず付ける。期限までに pass / fail(再現固定) / 削除のどれかに決める。
  - 既存の observe は observe_grandfathered.txt に載せる(until 無しを許す)。リストは縮める方向にだけ動かす:
    observe でなくなった(昇格・削除した)名前がリストに残っていたらテストが落ち、外すよう促す。

使い方: observe_audit.py [--write-baseline]   (一覧と内訳を表示 / 現状の observe を grandfathered.txt に書き出す)
"""
import os
import re
import sys
import tempfile
import textwrap

HERE = os.path.dirname(os.path.abspath(__file__))
WORKFLOW = os.path.join(HERE, "..", "..", "..", ".github", "workflows", "e2e-ime.yml")
GRANDFATHERED = os.path.join(HERE, "observe_grandfathered.txt")

# plan の only 絞り込みを通さず全構成を得るためのパターン(先頭 1 文字の前方一致で全部拾い、除外対象の接頭辞は明示)。
ONLY_ALL = ",".join([c + "*" for c in "abcdefghijklmnopqrstuvwxyz0123456789"] +
                    ["sc-keymatrix-*", "sc-offrca-*", "sc-bug149-*", "sc-table-*", "sc-preedit-*"])


def plan_source(workflow_path=WORKFLOW):
    """plan ジョブの python スクリプト本体(インデント除去済み)。matrix 上限チェックより前で切る。"""
    text = open(workflow_path, encoding="utf-8").read()
    m = re.search(r"        run: \|\n(          import fnmatch, json, os, sys\n.*?)\n  build:\n", text, re.S)
    if not m:
        raise RuntimeError("plan スクリプトが見つからない(e2e-ime.yml の構造が変わった?)")
    src = textwrap.dedent(m.group(1))
    cut = src.find("# ビルドは bin ごとに1回")
    if cut < 0:
        raise RuntimeError("plan スクリプトの切り出し位置が見つからない")
    return src[:cut]


def load_configs(workflow_path=WORKFLOW):
    """全構成(cal-/ts-/tsx-/sc-keymatrix- 等を含む)の dict 一覧。"""
    src = plan_source(workflow_path)
    env_backup = dict(os.environ)
    cwd_backup = os.getcwd()
    # plan スクリプトは forms.toml をリポジトリ直下からの相対パスで読む(CI は checkout 直下で実行する)。
    os.chdir(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", ".."))
    os.environ["ONLY"] = ONLY_ALL
    os.environ.pop("RUNS", None)
    os.environ.pop("EXCLUDE_CAL_WHEN_EMPTY", None)
    try:
        ns = {}
        exec(compile(src, "plan", "exec"), ns)  # noqa: S102 - リポジトリ自身のスクリプト
        return ns["configs"]
    finally:
        os.chdir(cwd_backup)
        os.environ.clear()
        os.environ.update(env_backup)


def read_grandfathered(path=GRANDFATHERED):
    try:
        with open(path, encoding="utf-8") as f:
            return {ln.strip() for ln in f if ln.strip() and not ln.startswith("#")}
    except OSError:
        return set()


def observe_names(configs):
    return sorted(c["name"] for c in configs if c["expect"] == "observe")


def main(argv):
    configs = load_configs()
    obs = observe_names(configs)
    if "--write-baseline" in argv:
        with open(GRANDFATHERED, "w", encoding="utf-8") as f:
            f.write("# expect=observe のまま until 無しを許す既存構成(縮める方向にだけ動かす。observe_audit.py / test_e2e_plan.py)。\n")
            f.write("# 昇格(pass/fail)・削除した構成はここから外す。新しい observe はここに足さず cfg(..., until='YYYY-MM-DD') を付ける。\n")
            f.writelines(n + "\n" for n in obs)
        print(f"{len(obs)} 件を {GRANDFATHERED} に書き出した")
        return 0
    by = {}
    for c in configs:
        by.setdefault(c["expect"], []).append(c["name"])
    print("expect 別: " + ", ".join(f"{k}={len(v)}" for k, v in sorted(by.items())))
    gf = read_grandfathered()
    dated = [c for c in configs if c["expect"] == "observe" and c.get("until")]
    print(f"observe {len(obs)} 件(期限付き {len(dated)} / grandfathered {len(gf & set(obs))})")
    prefixes = {}
    for n in obs:
        prefixes[n.split("-")[0] + "-" + (n.split("-")[1] if "-" in n else "")] = prefixes.get(n.split("-")[0] + "-" + (n.split("-")[1] if "-" in n else ""), 0) + 1
    for k, v in sorted(prefixes.items(), key=lambda kv: -kv[1])[:15]:
        print(f"  {k:<20} {v}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
