#!/usr/bin/env python3
"""
scan_dead_fields.py  —  Rust dead-field scanner

Finds struct fields that are never read via dot-notation (.field_name)
anywhere in the workspace.  Corpus is scanned once per token, not once
per field name, so it runs in O(corpus_size) rather than O(n_fields * corpus_size).

Usage:
    python3 scripts/scan_dead_fields.py [project_root]

Known limitations
-----------------
* pub fields in lib crates may be read by external crates  → false positives
* Method names colliding with field names                  → false negatives
* Destructuring  let S { field: var } = x  counts as a write here,
  while shorthand  let S { field } = x  counts as neither — only
  .field dot-access is reliably counted as a read.
* serde / derive macros that reflect fields at compile time → false positives
"""

import os
import re
import sys
from pathlib import Path
from collections import Counter
from dataclasses import dataclass

ROOT = Path(sys.argv[1] if len(sys.argv) > 1 else ".").resolve()
SKIP_DIRS = {"target", ".worktree", ".git", ".cargo", ".claude"}


# ---------------------------------------------------------------------------
# File collection
# ---------------------------------------------------------------------------

def collect_rs_files(root: Path) -> list[Path]:
    files = []
    for dp, dirs, fnames in os.walk(root):
        dirs[:] = [d for d in dirs if d not in SKIP_DIRS]
        for f in fnames:
            if f.endswith(".rs"):
                files.append(Path(dp) / f)
    return files


# ---------------------------------------------------------------------------
# Comment / string stripping  (in-place, preserves newlines)
# ---------------------------------------------------------------------------

def strip_comments_and_strings(src: str) -> str:
    out = list(src)
    i, n = 0, len(src)

    def blank(a: int, b: int) -> None:
        for k in range(a, b):
            if out[k] != "\n":
                out[k] = " "

    while i < n:
        if src[i:i+2] == "//":
            j = src.index("\n", i) if "\n" in src[i:] else n
            blank(i, j); i = j
        elif src[i:i+2] == "/*":
            blank(i, i+2); i += 2; depth = 1
            while i < n and depth:
                if src[i:i+2] == "/*":
                    blank(i, i+2); i += 2; depth += 1
                elif src[i:i+2] == "*/":
                    blank(i, i+2); i += 2; depth -= 1
                else:
                    blank(i, i+1); i += 1
        elif src[i] == "r" and i+1 < n and src[i+1] in '"#':
            blank(i, i+1); i += 1
            h = 0
            while i < n and src[i] == "#":
                blank(i, i+1); i += 1; h += 1
            if i < n and src[i] == '"':
                blank(i, i+1); i += 1
                close = '"' + "#" * h
                cl = len(close)
                while i < n:
                    if src[i:i+cl] == close:
                        blank(i, i+cl); i += cl; break
                    else:
                        blank(i, i+1); i += 1
        elif src[i] == '"' or (src[i] == "b" and i+1 < n and src[i+1] == '"'):
            # Regular or byte string literal "..." / b"..."
            if src[i] == "b":
                blank(i, i+1); i += 1
            blank(i, i+1); i += 1
            while i < n:
                if src[i] == "\\" and i+1 < n:
                    blank(i, i+2); i += 2
                elif src[i] == '"':
                    blank(i, i+1); i += 1; break
                else:
                    blank(i, i+1); i += 1
        elif src[i] == "'" and i+1 < n:
            # Char literal 'x' / '\n' / '\u{...}' — but NOT lifetime 'a / '_ / 'static.
            # Heuristic: if the content between the quotes is a single char or escape,
            # it's a char literal. If it's an identifier without a closing ',
            # treat it as a lifetime and skip.
            j = i + 1
            if src[j] == "\\":
                # Escape sequence char literal '\n', '\u{...}', etc.
                blank(i, i+1); i += 1
                while i < n:
                    if src[i] == "\\" and i+1 < n:
                        blank(i, i+2); i += 2
                    elif src[i] == "'":
                        blank(i, i+1); i += 1; break
                    else:
                        blank(i, i+1); i += 1
            elif j+1 < n and src[j+1] == "'" and src[j] != "'":
                # Exactly one non-quote char: 'x' form
                blank(i, j+2); i = j + 2
            else:
                # Lifetime specifier 'a, '_, 'static, or other — leave as-is
                i += 1
        else:
            i += 1
    return "".join(out)


# ---------------------------------------------------------------------------
# Struct field extraction
# ---------------------------------------------------------------------------

@dataclass
class FieldInfo:
    struct_name: str
    field_name: str
    filepath: str
    line_no: int


_STRUCT_START = re.compile(
    r"\bstruct\s+([A-Z][A-Za-z0-9_]*)(?:\s*<[^>]*>)?(?:\s*where[^{]*)?\s*\{",
    re.DOTALL,
)
_FIELD_LINE = re.compile(
    r"^\s*(?:#\[[^\]]*\]\s*)*"
    r"(?:pub(?:\s*\([^)]*\))?\s+)?"
    r"([a-z_][a-zA-Z0-9_]*)"
    r"\s*:\s*(?!:)",
    re.MULTILINE,
)


def extract_struct_body(clean: str, open_pos: int) -> tuple[str, int]:
    depth, i, n = 1, open_pos + 1, len(clean)
    while i < n and depth:
        if clean[i] == "{": depth += 1
        elif clean[i] == "}": depth -= 1
        i += 1
    return clean[open_pos+1 : i-1], i - 1


def extract_fields(clean: str, filepath: str) -> list[FieldInfo]:
    results = []
    for sm in _STRUCT_START.finditer(clean):
        struct_name = sm.group(1)
        body, _ = extract_struct_body(clean, sm.end() - 1)
        body_off = sm.end()
        for fm in _FIELD_LINE.finditer(body):
            fname = fm.group(1)
            if fname.startswith("_"):
                continue
            line_no = clean[: body_off + fm.start()].count("\n") + 1
            results.append(FieldInfo(struct_name, fname, filepath, line_no))
    return results


# ---------------------------------------------------------------------------
# Single-pass corpus scan
# ---------------------------------------------------------------------------

# Tokeniser: captures either
#   .identifier   (dot-access read)
#   identifier:   (struct-literal write, or field definition)
#   identifier    (bare identifier — used for shorthand destructure detection)
_TOKEN = re.compile(r"\.([a-z_][a-zA-Z0-9_]*)\b|([a-z_][a-zA-Z0-9_]*)\s*:\s*(?!:)")


def scan_corpus(corpus: str) -> tuple[Counter, Counter]:
    """
    Returns (dot_reads, colon_writes):
      dot_reads[name]   = count of  .name  occurrences
      colon_writes[name] = count of  name:  occurrences (struct defs + literals)
    """
    dot_reads: Counter = Counter()
    colon_writes: Counter = Counter()
    for m in _TOKEN.finditer(corpus):
        if m.group(1):
            dot_reads[m.group(1)] += 1
        else:
            colon_writes[m.group(2)] += 1
    return dot_reads, colon_writes


# ---------------------------------------------------------------------------
# Reporting
# ---------------------------------------------------------------------------

def rel(path: str) -> str:
    try:
        return str(Path(path).relative_to(ROOT))
    except ValueError:
        return path


def main() -> None:
    rs_files = collect_rs_files(ROOT)
    print(f"Scanning {len(rs_files)} .rs files under {ROOT}\n")

    clean_by_file: dict[str, str] = {}
    for p in rs_files:
        try:
            src = p.read_text(encoding="utf-8", errors="replace")
        except OSError as e:
            print(f"  [skip] {p}: {e}", file=sys.stderr)
            continue
        clean_by_file[str(p)] = strip_comments_and_strings(src)

    corpus = "\n".join(clean_by_file.values())
    print(f"Corpus: {len(corpus):,} chars — scanning…")

    # Single pass over corpus
    dot_reads, colon_writes = scan_corpus(corpus)

    # Extract struct field definitions
    all_fields: list[FieldInfo] = []
    for path, clean in clean_by_file.items():
        all_fields.extend(extract_fields(clean, path))

    n_structs = len({f.struct_name for f in all_fields})
    print(f"Found {len(all_fields)} named fields in {n_structs} structs\n")

    # Classify
    write_only: list[tuple[FieldInfo, int, int]] = []  # (info, reads, writes)
    never_used: list[tuple[FieldInfo, int, int]] = []

    for fi in all_fields:
        r = dot_reads[fi.field_name]
        w = colon_writes[fi.field_name]
        if r == 0:
            # colon_writes == 1 means only the struct definition itself
            # colon_writes >  1 means also initialized in struct literals elsewhere
            if w > 1:
                write_only.append((fi, r, w))
            else:
                never_used.append((fi, r, w))

    if write_only:
        print("=" * 72)
        print("WRITE-ONLY  (reads=0, initialized in struct literals)")
        print("Highest-confidence dead fields.")
        print("=" * 72)
        for fi, r, w in sorted(write_only, key=lambda x: (rel(x[0].filepath), x[0].struct_name)):
            print(f"  {rel(fi.filepath)}:{fi.line_no:<5}"
                  f"  struct {fi.struct_name:<30}  .{fi.field_name}  (writes={w})")
        print()

    if never_used:
        print("=" * 72)
        print("NEVER-ACCESSED  (reads=0, only in struct definition)")
        print("Field appears nowhere else; struct itself may be dead.")
        print("=" * 72)
        for fi, r, w in sorted(never_used, key=lambda x: (rel(x[0].filepath), x[0].struct_name)):
            print(f"  {rel(fi.filepath)}:{fi.line_no:<5}"
                  f"  struct {fi.struct_name:<30}  .{fi.field_name}")
        print()

    total = len(write_only) + len(never_used)
    if total == 0:
        print("No dead fields detected.")
    else:
        print(f"Summary: {len(write_only)} write-only + {len(never_used)} never-accessed"
              f" = {total} candidates\n")
        print("False-positive sources to check manually:")
        print("  • pub fields in lib crates (may be read by external consumers)")
        print("  • Destructuring  let S { field: var } = x  (counted as write here)")
        print("  • serde / derive macros (reflect fields without dot-access)")


if __name__ == "__main__":
    main()
