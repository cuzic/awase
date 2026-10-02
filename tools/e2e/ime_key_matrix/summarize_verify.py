#!/usr/bin/env python3
"""CI 検証スパイク(ブランチ ci/send-confirm-verify)用: awase.log の `[verify:TAG] key=value ...` 行をタグ別に集計する。

「送信=確認済み」の取り違え(applied が Confirmed になり後続送信を省く)、ImeOff による probe の deferred 破棄、
raw recovery の give-up で文字を消す、MS-IME ready poll の with_app 再入失敗、といった仮説が CI で本当に起きるかを、
構成(ベースライン/修正案フラグ付き)ごとに比べるための材料を作る。

使い方: summarize_verify.py [--json verify.json] awase.log
出力(stdout=verify.txt): タグ別件数、各タグの先頭3行、内訳(applied_kind・origin・deferred_n>0・lost_romaji・error・result)。
verify.json: {"counts": {tag: n}, "breakdown": {...}, "fix_flags": "...", "warn_lines": n}
情報のみで、終了コードは常に0(合否には使わない)。
"""
import argparse
import collections
import json
import re
import sys

TAG_RE = re.compile(r"\[verify:([\w\-]+)\]")
# key=value。値は "..."(引用)・[...](リスト)・空白までの語のいずれか。
KV_RE = re.compile(r'(\w+)=("[^"]*"|\[[^\]]*\]|\S+)')


def parse(path):
    lines_by_tag = collections.defaultdict(list)
    try:
        f = open(path, encoding="utf-8", errors="replace")
    except OSError:
        return lines_by_tag
    with f:
        for line in f:
            m = TAG_RE.search(line)
            if m:
                lines_by_tag[m.group(1)].append(line.rstrip("\n"))
    return lines_by_tag


def kv(line):
    # タグ以降だけを解析する(行頭のタイムスタンプ・スパン名を拾わない)。
    m = TAG_RE.search(line)
    return dict(KV_RE.findall(line[m.end():])) if m else {}


def breakdown(lines_by_tag):
    b = {}

    def count_by(tag, key, name=None):
        c = collections.Counter(kv(l).get(key, "-") for l in lines_by_tag.get(tag, []))
        if c:
            b[name or f"{tag}.{key}"] = dict(c)

    count_by("skip-gji-direct", "applied_kind")
    count_by("skip-gji-direct", "open")
    count_by("cancel-probe", "origin")
    count_by("apply-succeeded", "accepted")
    count_by("apply-succeeded", "applied_after")
    count_by("apply-failed", "error")
    count_by("apply-failed", "accepted")
    count_by("apply-failed", "applied_after")
    count_by("set-open-async", "result")
    count_by("optimistic", "source")
    count_by("poll-with_app-none", "site")
    count_by("sendinput-partial", "site")
    count_by("fix-applied", "flag")
    cp = lines_by_tag.get("cancel-probe", [])
    if cp:
        # deferred_n>0 = probe 取り消しでユーザー入力(deferred VK)を実際に捨てた件数。
        b["cancel-probe.deferred_n>0"] = sum(1 for l in cp if kv(l).get("deferred_n", "0") not in ("0", "-"))
        by_origin = collections.Counter(
            kv(l).get("origin", "-") for l in cp if kv(l).get("deferred_n", "0") not in ("0", "-")
        )
        if by_origin:
            b["cancel-probe.deferred_n>0.origin"] = dict(by_origin)
    gb = lines_by_tag.get("giveup-bs", [])
    if gb:
        # lost_romaji が空文字("")でない = 画面に見えていた文字を BS で消して再送しなかった件数。
        b["giveup-bs.lost_romaji_nonempty"] = sum(1 for l in gb if kv(l).get("lost_romaji", '""') not in ('""', "-"))
    return b


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--json", default="")
    ap.add_argument("log")
    a = ap.parse_args()
    lines_by_tag = parse(a.log)
    counts = {t: len(v) for t, v in sorted(lines_by_tag.items())}
    b = breakdown(lines_by_tag)
    flags = ""
    if lines_by_tag.get("fix-flags"):
        flags = kv(lines_by_tag["fix-flags"][0]).get("active", "")
    print(f"VERIFY: total={sum(counts.values())} tags={len(counts)} fix_flags={flags or '-'}")
    for tag, n in counts.items():
        print(f"  {tag}: {n}")
    for k, v in b.items():
        print(f"  [内訳] {k}: {v}")
    for tag, lines in sorted(lines_by_tag.items()):
        print(f"--- {tag} 先頭3行 ---")
        for l in lines[:3]:
            print("  " + l[:400])
    if a.json:
        with open(a.json, "w", encoding="utf-8") as f:
            json.dump({"counts": counts, "breakdown": b, "fix_flags": flags}, f, ensure_ascii=False)
    return 0


if __name__ == "__main__":
    sys.exit(main())
