#!/usr/bin/env python3
"""MSI のアップグレード・アンインストールで、ユーザーの設定が残るかを実 MSI で確かめる(BUG-071、ADR-099/178)。

windows-latest 上で、リリース済みの MSI を `gh release download` で取り、旧版を入れる → ユーザー設定を書き換える → 新版を上書きインストール →
設定が残っているかを調べ、最後にアンインストールして Permanent 指定のファイルが残るかも調べる。ビルドは不要(配布物そのものを検証する)。
perUser インストール(%LOCALAPPDATA%\\awase)なので管理者権限は要らない。

使い方: python tools/e2e/msi_upgrade/run.py --out out [--pairs v1.21.2:v2.0.0,v1.20.0:v2.0.0] [--repo cuzic/awase]
判定: PASS / FAIL(期待と違う=見つかった不具合) / ERROR(検証自体が実行できなかった)。FAIL が 1 件でもあれば終了コード 1。
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import sys
import shutil
from pathlib import Path

MARK_CFG = "# E2E-USER-MARKER-CONFIG"
MARK_YAB = "# E2E-USER-MARKER-YAB"
CUSTOM_YAB = "custom-e2e.yab"


def run(cmd, log: Path | None = None, check=False, timeout=600):
    p = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=timeout)
    if log is not None:
        log.write_text(f"$ {' '.join(map(str, cmd))}\nrc={p.returncode}\n{p.stdout}\n{p.stderr}", encoding="utf-8")
    if check and p.returncode != 0:
        raise RuntimeError(f"{cmd} rc={p.returncode}\n{p.stdout}\n{p.stderr}")
    return p


def install_dir() -> Path:
    return Path(os.environ["LOCALAPPDATA"]) / "awase"


def sha(p: Path) -> str:
    return hashlib.sha256(p.read_bytes()).hexdigest()[:12] if p.exists() else "-"


def msi_for(tag: str, workdir: Path, repo: str) -> Path:
    ver = tag.lstrip("v")
    name = f"awase-{ver}-x64.msi"
    dest = workdir / name
    if not dest.exists():
        run(["gh", "release", "download", tag, "-R", repo, "-p", name, "-D", str(workdir)], check=True)
    return dest


def msi_install(msi: Path, log: Path):
    return run(["msiexec", "/i", str(msi), "/qn", "/norestart", "/l*v", str(log)], timeout=900)


def msi_uninstall(msi: Path, log: Path):
    return run(["msiexec", "/x", str(msi), "/qn", "/norestart", "/l*v", str(log)], timeout=900)


def clean(old_msi: Path | None, logs: Path):
    d = install_dir()
    if d.exists():
        if old_msi:
            msi_uninstall(old_msi, logs / "clean-uninstall.log")
        shutil.rmtree(d, ignore_errors=True)


def scenario(old_tag: str, new_tag: str, work: Path, logs: Path, repo: str):
    """1 組の (旧→新) を検証し、チェック結果のリスト [(名前, ok, 詳細)] を返す。"""
    res = []
    d = install_dir()
    tag = f"{old_tag}-to-{new_tag}"
    old = msi_for(old_tag, work, repo)
    new = msi_for(new_tag, work, repo)
    clean(old, logs)

    r = msi_install(old, logs / f"{tag}-1-install-old.log")
    res.append(("旧版のインストール", r.returncode in (0, 3010), f"rc={r.returncode}"))
    exe_old = sha(d / "awase.exe")
    cfg = d / "config.toml"
    yab = d / "layout" / "nicola.yab"
    listing = sorted(str(q.relative_to(d)) for q in d.rglob("*") if q.is_file())[:30]
    res.append(("旧版: awase.exe が配置", (d / "awase.exe").exists(), f"exe={exe_old} files={listing}"))
    # 旧版の MSI が config.toml / layout/nicola.yab を同梱しないことがある(v1.20.0 など)。その場合はユーザー(アプリの初回起動)が作った状態を再現して続ける。
    if not cfg.exists():
        cfg.write_text("[general]\n", encoding="utf-8")
    if not yab.exists():
        yab.parent.mkdir(parents=True, exist_ok=True)
        yab.write_text("; user layout\n", encoding="utf-8")

    # ユーザーが設定を書き換えた状態を作る
    cfg.write_text(cfg.read_text(encoding="utf-8") + f"\n{MARK_CFG}\n", encoding="utf-8")
    yab.write_text(yab.read_text(encoding="utf-8") + f"\n{MARK_YAB}\n", encoding="utf-8")
    (d / "layout" / CUSTOM_YAB).write_text("; user created layout\n", encoding="utf-8")

    r = msi_install(new, logs / f"{tag}-2-install-new.log")
    res.append(("新版の上書きインストール", r.returncode in (0, 3010), f"rc={r.returncode}"))
    exe_new = sha(d / "awase.exe")
    res.append(("新版: awase.exe が入れ替わった", exe_new != exe_old and exe_new != "-", f"{exe_old} -> {exe_new}"))
    cfg_text = cfg.read_text(encoding="utf-8") if cfg.exists() else ""
    res.append(("アップグレード後: config.toml のユーザー編集が残る", MARK_CFG in cfg_text, "marker あり" if MARK_CFG in cfg_text else f"marker なし(exists={cfg.exists()})"))
    yab_text = yab.read_text(encoding="utf-8") if yab.exists() else ""
    res.append(("アップグレード後: layout/nicola.yab のユーザー編集が残る", MARK_YAB in yab_text, "marker あり" if MARK_YAB in yab_text else f"marker なし(exists={yab.exists()})"))
    res.append(("アップグレード後: ユーザー作成の layout/*.yab が残る", (d / "layout" / CUSTOM_YAB).exists(), ""))

    r = msi_uninstall(new, logs / f"{tag}-3-uninstall-new.log")
    res.append(("新版のアンインストール", r.returncode in (0, 3010), f"rc={r.returncode}"))
    res.append(("アンインストール後: awase.exe が消える", not (d / "awase.exe").exists(), ""))
    res.append(("アンインストール後: config.toml が残る(Permanent)", cfg.exists() and MARK_CFG in cfg.read_text(encoding="utf-8"), f"exists={cfg.exists()}"))
    res.append(("アンインストール後: ユーザー作成の layout/*.yab が残る", (d / "layout" / CUSTOM_YAB).exists(), ""))
    return res


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default="out")
    ap.add_argument("--pairs", default="v1.21.2:v2.0.0,v1.20.0:v2.0.0,v1.20.0:v1.21.2")
    ap.add_argument("--repo", default="cuzic/awase")
    a = ap.parse_args(argv)
    out = Path(a.out)
    logs = out / "logs"
    logs.mkdir(parents=True, exist_ok=True)
    work = out / "msi"
    work.mkdir(parents=True, exist_ok=True)

    results = {}
    any_fail = False
    for pair in a.pairs.split(","):
        old_tag, new_tag = pair.split(":")
        try:
            checks = scenario(old_tag, new_tag, work, logs, a.repo)
        except Exception as e:  # 検証自体が実行できなかった
            checks = [("検証の実行", False, f"ERROR: {e}")]
        results[pair] = [dict(name=n, ok=bool(ok), detail=d) for n, ok, d in checks]
        any_fail |= any(not c[1] for c in checks)

    (out / "results.json").write_text(json.dumps(results, ensure_ascii=False, indent=2), encoding="utf-8")
    md = ["# MSI アップグレード検証(BUG-071)", ""]
    for pair, checks in results.items():
        md += [f"## {pair.replace(':', ' → ')}", "", "| チェック | 結果 | 詳細 |", "|---|---|---|"]
        md += [f"| {c['name']} | {'PASS' if c['ok'] else 'FAIL'} | {c['detail']} |" for c in checks]
        md.append("")
    (out / "summary.md").write_text("\n".join(md), encoding="utf-8")
    print("\n".join(md))
    return 1 if any_fail else 0


if __name__ == "__main__":
    sys.exit(main())
