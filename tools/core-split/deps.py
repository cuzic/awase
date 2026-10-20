#!/usr/bin/env python3
"""More careful dependency scan for ADR-274 review.

- strips comments/strings (doc-link refs reported separately)
- separates cfg(test)-attributed items properly (not "cut at first #[cfg(test)]")
- expands brace groups: crate::a::{b::C, d}
- resolves super::super::..., self::
- resolves re-exports in state/mod.rs and focus/mod.rs (glob belief::*, hook_state::*, named)
"""
import os, re, sys, collections

ROOTS = sys.argv[1].split(":")
ASSIGN = sys.argv[2]
CHECK = "--check" in sys.argv

def load_modules():
    """`ROOTS`(コロン区切り。core 系 crate の `src/` の和)の全 `.rs` を、`src/` 内の相対パスのモジュール名で集める。
    crate を分けても相対パスを保つ(ADR-274 D3)ので、同じ名前空間として扱える。同じ名前が複数の root にあれば NG。"""
    mods = {}
    for root in ROOTS:
        for dp, _, fs in os.walk(root):
            for f in fs:
                if not f.endswith(".rs"):
                    continue
                p = os.path.join(dp, f)
                rel = os.path.relpath(p, root)[:-3].replace(os.sep, "::")
                if rel.endswith("::mod"):
                    rel = rel[:-5]
                if rel == "lib":
                    rel = ""
                if rel in mods and rel != "":
                    print(f"NG: モジュール {rel} が複数の root にある: {mods[rel]} と {p}")
                    sys.exit(1)
                mods.setdefault(rel, p)
    return mods

def strip(text):
    """Return (code, comments) with comments/strings blanked (keep newlines)."""
    out = []
    com = []
    i, n = 0, len(text)
    while i < n:
        c = text[i]
        if text.startswith("//", i):
            j = text.find("\n", i)
            j = n if j < 0 else j
            com.append(text[i:j])
            out.append(" " * (j - i))
            i = j
        elif text.startswith("/*", i):
            j = text.find("*/", i)
            j = n if j < 0 else j + 2
            seg = text[i:j]
            com.append(seg)
            out.append(re.sub(r"[^\n]", " ", seg))
            i = j
        elif c == 'r' and re.match(r'r#*"', text[i:i+10]) and (i == 0 or not (text[i-1].isalnum() or text[i-1]=='_')):
            m = re.match(r'r(#*)"', text[i:])
            hashes = m.group(1)
            end = text.find('"' + hashes, i + len(m.group(0)))
            j = n if end < 0 else end + 1 + len(hashes)
            out.append(re.sub(r"[^\n]", " ", text[i:j]))
            i = j
        elif c == '"':
            j = i + 1
            while j < n and text[j] != '"':
                j += 2 if text[j] == "\\" else 1
            j += 1
            out.append(re.sub(r"[^\n]", " ", text[i:j]))
            i = j
        elif c == "'" and re.match(r"'(\\.|[^\\'])'", text[i:i+4]) or (c=="'" and re.match(r"'\\[xu][^']*'", text[i:i+12])):
            m = re.match(r"'(\\[xu][^']*|\\.|[^\\'])'", text[i:])
            out.append(" " * len(m.group(0)))
            i += len(m.group(0))
        else:
            out.append(c)
            i += 1
    return "".join(out), "\n".join(com)

TEST_ATTR = re.compile(r"#\[cfg\((test|any\(test[^\]]*|all\(test[^\]]*)\)\]")

def split_test(code):
    """Blank cfg(test) items from code -> (prod, test)."""
    prod = list(code)
    test = [" " if ch != "\n" else "\n" for ch in code]
    for m in TEST_ATTR.finditer(code):
        # item starts after attribute; skip further attributes/whitespace
        i = m.end()
        # find end of item: first ';' at depth 0 or matching '}' of first '{'
        depth = 0
        j = i
        started = False
        while j < len(code):
            ch = code[j]
            if ch == "{":
                depth += 1; started = True
            elif ch == "}":
                depth -= 1
                if started and depth == 0:
                    j += 1; break
            elif ch == ";" and depth == 0:
                j += 1; break
            j += 1
        for k in range(m.start(), j):
            test[k] = code[k]
            if code[k] != "\n":
                prod[k] = " "
    return "".join(prod), "".join(test)

IDENT = r"[A-Za-z_][A-Za-z0-9_]*"

def parse_paths(code):
    """Yield (start_kw, [segments]) for each path starting with crate/super/self, expanding braces."""
    res = []
    for m in re.finditer(r"\b(crate|super|self)\b(?=\s*::)", code):
        i = m.start()
        # don't treat `self` in `self.` or method receivers; require ::
        prefix = []
        j = i
        def parse(j, prefix):
            # parse segments separated by ::
            segs = list(prefix)
            while True:
                mm = re.match(r"\s*(" + IDENT + r"|\*)", code[j:])
                if not mm:
                    break
                tok = mm.group(1)
                j += mm.end()
                segs.append(tok)
                mm2 = re.match(r"\s*::\s*", code[j:])
                if not mm2:
                    res.append(segs)
                    return j
                j += mm2.end()
                if code[j] == "{":
                    j += 1
                    while True:
                        mm3 = re.match(r"\s*", code[j:]); j += mm3.end()
                        if code[j] == "}":
                            j += 1; break
                        j = parse(j, segs)
                        mm3 = re.match(r"\s*(as\s+" + IDENT + r")?\s*", code[j:]); j += mm3.end()
                        if code[j] == ",":
                            j += 1
                        elif code[j] == "}":
                            j += 1; break
                        else:
                            # nested group handled; unexpected token
                            j += 1
                    res.append(segs + ["{}"])
                    return j
            res.append(segs)
            return j
        parse(i, [])
    return res

def main():
    mods = load_modules()
    names = set(mods)
    # re-exports: (module, name) -> target module
    reexp = {}
    globs = collections.defaultdict(list)
    for m, p in mods.items():
        code, _ = strip(open(p, encoding="utf-8").read())
        for mm in re.finditer(r"pub(?:\([^)]*\))?\s+use\s+([^;]+);", code):
            body = mm.group(1).replace("\n", " ")
            body = re.sub(r"^(self|crate)::", "", body.strip())
            # handle `x::*` and `x::{a, b}` and `x::Y`
            g = re.match(r"(" + IDENT + r"(?:::" + IDENT + r")*)::\*$", body)
            if g:
                tgt = g.group(1)
                full = (m + "::" + tgt) if m and not mm.group(1).strip().startswith("crate::") else tgt
                globs[m].append(full)
                continue
            b = re.match(r"(" + IDENT + r"(?:::" + IDENT + r")*)::\{(.*)\}$", body)
            if b:
                base = b.group(1)
                for item in b.group(2).split(","):
                    item = item.strip()
                    if not item:
                        continue
                    nm = item.split(" as ")[-1].strip().split("::")[-1]
                    path = base + "::" + item.split(" as ")[0].strip()
                    tm = "::".join(path.split("::")[:-1])
                    full = (m + "::" + tm) if m and not mm.group(1).strip().startswith("crate::") else tm
                    reexp[(m, nm)] = full
                continue
            s = re.match(r"(" + IDENT + r"(?:::" + IDENT + r")*)$", body)
            if s:
                parts = s.group(1).split("::")
                tm = "::".join(parts[:-1])
                full = (m + "::" + tm) if m and not mm.group(1).strip().startswith("crate::") else tm
                reexp[(m, parts[-1])] = full
    # pub items per module (for glob resolution)
    items = {}
    for m, p in mods.items():
        code, _ = strip(open(p, encoding="utf-8").read())
        items[m] = set(re.findall(r"^pub\s+(?:const\s+fn|fn|struct|enum|trait|type|const|static)\s+(" + IDENT + ")", code, re.M))

    def resolve(cur, segs):
        """Return target module for a path from module cur."""
        if segs[0] == "crate":
            mod = ""; rest = segs[1:]
        elif segs[0] == "self":
            mod = cur; rest = segs[1:]
            # self:: in a non-mod.rs file refers to the file module itself
        else:
            mod = cur; rest = list(segs)
            while rest and rest[0] == "super":
                mod = "::".join(mod.split("::")[:-1]) if "::" in mod else ""
                rest = rest[1:]
        for s in rest:
            if s in ("{}", "*"):
                break
            cand = (mod + "::" + s) if mod else s
            if cand in names:
                mod = cand
                continue
            # re-export?
            if (mod, s) in reexp:
                return reexp[(mod, s)]
            for gtarget in globs.get(mod, []):
                if s in items.get(gtarget, set()):
                    return gtarget
            break
        return mod

    ns = {}
    exec(open(ASSIGN, encoding="utf-8").read(), ns)
    G = dict(ns["G"])
    G[""] = "lib"
    unassigned = sorted(m for m in mods if m and m not in G)
    missing = sorted(m for m in G if m and m not in mods)
    planned = {(a, b) for a, b, *_ in ns.get("PLANNED_EDGE_REMOVALS", [])}
    allowed = ns.get("ALLOWED", {})
    edges = {"prod": collections.defaultdict(list), "test": collections.defaultdict(list), "doc": collections.defaultdict(list)}
    for m, p in mods.items():
        raw = open(p, encoding="utf-8").read()
        code, comments = strip(raw)
        prod, test = split_test(code)
        for kind, txt in (("prod", prod), ("test", test), ("doc", comments)):
            for segs in parse_paths(txt):
                t = resolve(m, segs)
                if t == m or t not in G or m not in G:
                    continue
                a, b = G[m], G[t]
                if a != b:
                    edges[kind][(a, b)].append((m, t, "::".join(segs)))
    for kind in ("prod", "test", "doc"):
        print(f"===== {kind}")
        for (a, b), v in sorted(edges[kind].items()):
            uniq = sorted(set((x, y) for x, y, _ in v))
            print(f"  {a:12s} -> {b:12s} {len(uniq)}")
            for x, y in uniq:
                ex = [s for xx, yy, s in v if xx == x and yy == y][0]
                print(f"        {x} -> {y}   e.g. {ex}")

    if CHECK:
        bad = []
        if unassigned:
            bad.append("未割当のモジュール(assign.py に足す): " + ", ".join(unassigned))
        if missing:
            bad.append("assign.py にあるが走査で見つからないモジュール(移した・消したのに assign.py を直していない、"
                       "または走査の root から crate が漏れている): " + ", ".join(missing))
        for kind in ("prod", "test"):
            for (a, b), v in sorted(edges[kind].items()):
                if a == "lib" or b == "lib":
                    continue
                if b in allowed.get(a, []):
                    continue
                uniq = sorted({(x, y) for x, y, _ in v if (x, y) not in planned})
                if not uniq:
                    continue
                bad.append(f"許可されていない依存 {a} -> {b}: " + "; ".join(f"{x} -> {y}" for x, y in uniq))
        if bad:
            print("\n== NG(ADR-274 の層の向きに反する) ==")
            print("\n".join(bad))
            sys.exit(1)
        print("\n== OK: 本番コードの依存は ALLOWED の向きだけ、未割当なし ==")


main()
