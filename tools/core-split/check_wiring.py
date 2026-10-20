#!/usr/bin/env python3
"""core 系 crate の一覧(core-crates.txt)と、それを使う場所の配線を照合する(ADR-274 段階 0 の 5)。

core 系 crate を足したとき、パスや `-p` を直書きしている場所の直し忘れで、CI が緑のまま検査が外れる
(BUG-116 と同じ型の穴)のを防ぐ。足し忘れは、このスクリプトが名前を挙げて落とす。

使い方(リポジトリのルートで):
    python3 tools/core-split/check_wiring.py check   # 配線の照合。足りなければ終了コード 1
    python3 tools/core-split/check_wiring.py roots   # core 系 crate の src をコロン区切りで出す(deps.py の引数用)

照合する場所:
  - tests/support/src_roots.rs の EXTRA_SRC_CRATES(ガードが走査する crate)
  - .github/workflows/ci.yml の doctest・dylint・clippy・nextest の `-p`、mutants の matrix の pkg
  - .github/workflows/mutants-windows.yml と mise.toml の `-p`
  - .cargo/mutants-<package>.toml が存在すること
  - crates/xtask-adr-evidence/src/core_registry.rs が crate のディレクトリに触れていること
  - layer_boundary_guard.rs が crate のディレクトリに触れていること(windows に依存しない検査)
  - .githooks/pre-push が一覧を読むこと
  - core 系 crate に統合テスト(`tests/*.rs`)を置かないこと。統合テストは殻の `crates/awase-windows/tests/` に置く
    (`ci_test_coverage_guard.rs` が見るのは殻の tests/ だけで、`cargo nextest run --workspace --lib` は統合テストを
    走らせない。移した crate の統合テストは、どこからも実行されなくなる。ADR-274 段階 0 の 6)
  - Cargo.toml が `[lints] workspace = true` で、ルートの `[workspace.lints]` を使うこと(lints が crate ごとに複製されて
    ずれるのを防ぐ。ADR-274 段階 0 の 4)
"""
import os
import re
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
LIST = os.path.join(ROOT, "tools", "core-split", "core-crates.txt")


def read(rel):
    with open(os.path.join(ROOT, rel), encoding="utf-8") as f:
        return f.read()


def crate_dirs():
    out = []
    for line in read("tools/core-split/core-crates.txt").splitlines():
        line = line.strip()
        if line and not line.startswith("#"):
            out.append(line)
    return out


def package_name(d):
    m = re.search(r'^name\s*=\s*"([^"]+)"', read(d + "/Cargo.toml"), re.M)
    if not m:
        raise SystemExit(f"{d}/Cargo.toml に name が無い")
    return m.group(1)


def lines_with(text, needle):
    return [l for l in text.splitlines() if needle in l]


def check():
    errors = []
    dirs = crate_dirs()
    if not dirs:
        errors.append("core-crates.txt が空")
    ci = read(".github/workflows/ci.yml")
    mw = read(".github/workflows/mutants-windows.yml")
    mise = read("mise.toml")
    src_roots = read("crates/awase-windows/tests/support/src_roots.rs")
    registry = read("crates/xtask-adr-evidence/src/core_registry.rs")
    lbg = read("crates/awase-windows/tests/layer_boundary_guard.rs")
    prepush = read(".githooks/pre-push")

    if "tools/core-split/core-crates.txt" not in prepush:
        errors.append(".githooks/pre-push が core-crates.txt を読んでいない(再発ファミリーの警告が新 crate から外れる)")

    # EXTRA_SRC_CRATES は一覧と一致すること(多くても少なくても NG)
    m = re.search(r"EXTRA_SRC_CRATES:\s*&\[&str\]\s*=\s*&\[(.*?)\];", src_roots, re.S)
    extra = re.findall(r'"([^"]+)"', m.group(1)) if m else []
    if not m:
        errors.append("src_roots.rs に EXTRA_SRC_CRATES が見つからない")
    elif sorted(extra) != sorted(dirs):
        errors.append(f"EXTRA_SRC_CRATES {sorted(extra)} が core-crates.txt {sorted(dirs)} と一致しない")

    # ci.yml の mutants matrix
    pkg_line = [l for l in ci.splitlines() if re.match(r"\s*pkg:\s*\[", l)]
    for d in dirs:
        pkg = package_name(d)

        def need(label, hay_lines, needle=None):
            n = needle or f"-p {pkg}"
            if not any(re.search(r"(?<![\w-])" + re.escape(n) + r"(?![\w-])", l) for l in hay_lines):
                errors.append(f"{pkg}: {label} に {n} が無い")

        need("ci.yml の doctest(cargo test --doc)", lines_with(ci, "cargo test --doc"))
        need("ci.yml の dylint", lines_with(ci, "cargo dylint"))
        need("ci.yml の clippy", lines_with(ci, "cargo clippy -p"))
        need("ci.yml の windows の nextest", lines_with(ci, "cargo nextest run --lib -p"))
        need("mutants-windows.yml", lines_with(mw, "cargo mutants"))
        need("mise.toml の check", lines_with(mise, "cargo check"))
        if not any(re.search(r"[\[,\s]" + re.escape(pkg) + r"[\],\s]", l) for l in pkg_line):
            errors.append(f"{pkg}: ci.yml の mutants matrix(pkg: [...])に無い")
        if not os.path.exists(os.path.join(ROOT, f".cargo/mutants-{pkg}.toml")):
            errors.append(f"{pkg}: .cargo/mutants-{pkg}.toml が無い")
        if d not in registry and pkg not in registry:
            errors.append(f"{pkg}: xtask-adr-evidence の core_registry.rs が {d} に触れていない(mutants の登録を数えない)")
        tests_dir = os.path.join(ROOT, d, "tests")
        if os.path.isdir(tests_dir):
            stray = sorted(f for f in os.listdir(tests_dir) if f.endswith(".rs"))
            if stray:
                errors.append(
                    f"{pkg}: {d}/tests/ に統合テスト {stray} がある。CI で一度も走らない(ci_test_coverage_guard は殻の tests/ だけを見る)。"
                    "殻の crates/awase-windows/tests/ に置く(再公開された経路で引ける)"
                )
        if not re.search(r"^\[lints\]\s*\nworkspace\s*=\s*true", read(d + "/Cargo.toml"), re.M):
            errors.append(f"{pkg}: Cargo.toml が `[lints] workspace = true` でない(ルートの [workspace.lints] を使う)")
        if "core-crates.txt" not in lbg and d not in lbg and pkg not in lbg:
            errors.append(f"{pkg}: layer_boundary_guard.rs が core-crates.txt を読んでいない・{d} にも触れていない(windows 非依存の検査から外れる)")
    return errors


def main():
    cmd = sys.argv[1] if len(sys.argv) > 1 else "check"
    if cmd == "roots":
        print(":".join(os.path.join(d, "src") for d in crate_dirs()))
        return
    if cmd != "check":
        raise SystemExit("使い方: check_wiring.py [check|roots]")
    errors = check()
    if errors:
        print("== NG: core 系 crate の配線漏れ(ADR-274 段階 0 の 5)==")
        for e in errors:
            print("  - " + e)
        print("core-crates.txt に crate を足したら、上の場所も直す。直さないと CI が緑のまま検査が外れる。")
        sys.exit(1)
    print(f"== OK: core 系 crate {len(crate_dirs())} 個の配線を照合した ==")


if __name__ == "__main__":
    main()
