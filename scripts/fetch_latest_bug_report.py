#!/usr/bin/env python3
"""Fetch the most recently submitted tray bug report(s) from Cloudflare R2.

Background: docs/adr/095-tray-bug-report-cloudflare-intake.md. The Worker's R2
binding is write-only (put() only), so listing/reading is done with the
maintainer's own `wrangler` OAuth token against the Cloudflare API directly,
then downloaded with `wrangler r2 object get` (same approach as
.claude/skills/bug-report-fetch, just automated for "give me the latest one").

Usage:
    python3 scripts/fetch_latest_bug_report.py [--count N] [--out-dir DIR]

Requires: `wrangler login` already done (checked via `wrangler whoami`).
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import tomllib
import urllib.request
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
WORKER_DIR = REPO_ROOT / "services" / "report-worker"
WRANGLER_CONFIG = Path.home() / ".config" / ".wrangler" / "config" / "default.toml"

# Fields whose raw text is large enough that dumping them inline is unhelpful;
# these get written to their own file instead (mirrors bug-report-fetch's
# step 3 decomposition, so both the script and skill agree on file names).
LARGE_TEXT_FIELDS = {
    "log_excerpt": "journal.json",
    "app_log_excerpt": "awase.log.txt",
    "config_toml": "config.toml",
    "layout_yab": "layout.yab",
}


def load_wrangler_token() -> str:
    if not WRANGLER_CONFIG.exists():
        sys.exit(
            f"wrangler の OAuth トークンが見つかりません: {WRANGLER_CONFIG}\n"
            "`wrangler login` を実行してください。"
        )
    with WRANGLER_CONFIG.open("rb") as f:
        config = tomllib.load(f)
    token = config.get("oauth_token")
    if not token:
        sys.exit(f"{WRANGLER_CONFIG} に oauth_token がありません。`wrangler login` してください。")
    return token


def load_bucket_config() -> tuple[str, str]:
    wrangler_toml = WORKER_DIR / "wrangler.toml"
    with wrangler_toml.open("rb") as f:
        config = tomllib.load(f)
    account_id = config["account_id"]
    bucket_name = config["r2_buckets"][0]["bucket_name"]
    return account_id, bucket_name


def list_reports(account_id: str, bucket_name: str, token: str) -> list[dict]:
    objects: list[dict] = []
    cursor: str | None = None
    while True:
        url = (
            f"https://api.cloudflare.com/client/v4/accounts/{account_id}"
            f"/r2/buckets/{bucket_name}/objects?per_page=1000&prefix=reports/"
        )
        if cursor:
            url += f"&cursor={cursor}"
        req = urllib.request.Request(url, headers={"Authorization": f"Bearer {token}"})
        with urllib.request.urlopen(req) as resp:
            data = json.load(resp)
        if not data.get("success"):
            sys.exit(f"R2 list API エラー: {data.get('errors')}")
        objects.extend(data["result"])
        cursor = (data.get("result_info") or {}).get("cursor")
        if not cursor:
            break
    return objects


def report_id_from_key(key: str) -> str:
    # reports/<year>/<month>/<report_id>.json
    return Path(key).stem


def download_report(key: str, dest: Path) -> None:
    subprocess.run(
        [
            "npx",
            "wrangler",
            "r2",
            "object",
            "get",
            f"awase-report-bucket/{key}",
            "--remote",
            "-f",
            str(dest),
        ],
        cwd=WORKER_DIR,
        check=True,
    )


def summarize(report_path: Path, out_dir: Path) -> None:
    data = json.loads(report_path.read_text())
    payload = data.get("payload", data)

    print(f"\n=== {report_path.stem} ===")
    for key, value in payload.items():
        if key in LARGE_TEXT_FIELDS and isinstance(value, str) and value:
            target = out_dir / f"{report_path.stem}.{LARGE_TEXT_FIELDS[key]}"
            target.write_text(value)
            print(f"  {key}: <{len(value)} chars> -> {target}")
        elif isinstance(value, str) and len(value) > 500:
            print(f"  {key}: <string, {len(value)} chars>")
        else:
            print(f"  {key}: {value!r}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--count", type=int, default=1, help="取得する報告の件数（新しい順、既定1件）")
    parser.add_argument(
        "--out-dir",
        type=Path,
        default=Path("/tmp/bug-reports"),
        help="ダウンロード先ディレクトリ（既定: /tmp/bug-reports）",
    )
    args = parser.parse_args()

    args.out_dir.mkdir(parents=True, exist_ok=True)

    token = load_wrangler_token()
    account_id, bucket_name = load_bucket_config()

    objects = list_reports(account_id, bucket_name, token)
    if not objects:
        sys.exit("R2 バケットに不具合報告が1件もありません。")

    objects.sort(key=lambda o: o["last_modified"])
    latest = objects[-args.count :][::-1]

    print(f"R2 上の報告総数: {len(objects)} 件。新しい順に {len(latest)} 件を取得します。")

    for obj in latest:
        report_id = report_id_from_key(obj["key"])
        dest = args.out_dir / f"{report_id}.json"
        print(f"\n取得中: {obj['key']} (last_modified={obj['last_modified']}, size={obj['size']}B)")
        download_report(obj["key"], dest)
        summarize(dest, args.out_dir)

    print(f"\n出力先ディレクトリ: {args.out_dir}")
    print(
        "既存の対応状況は docs/bug-reports-triage.md を確認すること"
        "（未記載なら .claude/skills/bug-report-fetch の手順3以降で調査する）。"
    )


if __name__ == "__main__":
    main()
