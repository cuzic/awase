#!/usr/bin/env python3
"""不具合報告の受付 Worker（schema_version 3/4）のデプロイ後スモークテスト。

ADR-222。Worker のデプロイ後に、本番（または `wrangler dev`）へ実際に POST して、
次を確認する。標準ライブラリだけで動く（pip/uv 不要）。

  1. v3（旧クライアントの形式、非圧縮ログ）が 201 で受理される
  2. v4（gzip + base64）の小さい報告が 201 で受理される
  3. v4 の現実的な報告（本体 約 400KiB。10 分ぶんの打鍵 + ログを gzip した大きさの見込み）が 201
  4. v4 の大きい報告（本体 約 1.8MiB。上限近くのストレス）が 201 で受理される（所要時間も表示）
  5. v4 に非圧縮の log_excerpt が付いていると 400 (legacy_log_fields_not_allowed_in_schema_4)
  6. gzip でない文字列は 400 (log_excerpt_gz_invalid)
  7. 本体が上限（2MiB）を超えると 413 (request_body_too_large)

注意:
  - 送信は 7 件。受付は **1 IP あたり 1 日 20 件**までなので、続けて何度も実行しない。
    5〜7 は 400/413 で弾かれるためレート制限のカウントには入らない（検証が先に走る）が、
    1〜4 は入る。
  - 1〜4 は本番の R2 に実際に保存される。終わったら README の手順で削除すること。
    報告本文は「ADR-222 deploy smoke test」と分かる文言にしてある。
  - Workers Free プランの CPU 上限（10ms/リクエスト）の確認は、このスクリプトの 3 を
    送りながら別の端末で `pnpm exec wrangler tail awase-report-worker --format json` を
    見て、`cpuTime` と `outcome`（`exceededCpu` が出たら超過）を読む。3 が現実的なサイズ、
    4 が上限近くのストレス。「ふつうの報告は余裕、極端な報告だけ危うい」を見分ける。

Usage:
    python3 scripts/report_worker_smoke.py [--endpoint URL] [--large-kib 1800]
"""

from __future__ import annotations

import argparse
import base64
import gzip
import json
import os
import sys
import time
import urllib.error
import urllib.request

DEFAULT_ENDPOINT = "https://report.awase.cc/v1/reports"
MAX_BODY_BYTES = 2 * 1024 * 1024  # Worker の MAX_BODY_BYTES と同じ


def base_payload(schema_version: int) -> dict:
    return {
        "schema_version": schema_version,
        "app_version": "0.0.0-smoke",
        "os_version": "smoke test",
        "ime_kind": "Unknown",
        "ime_product_name": None,
        "keyboard_model": "Jis",
        "windows_keyboard_layout": "LANGID=0x0411 (Japanese=true)",
        "competing_software": [],
        "symptom_category": "NoResponse",
        "description": "ADR-222 deploy smoke test (削除してよい)",
        "attach_log": True,
        "log_excerpt": None,
        "app_log_excerpt": None,
        "attach_state_snapshot": False,
        "state_snapshot": None,
        "attach_config": False,
        "config_toml": None,
        "attach_layout": False,
        "layout_yab": None,
        "attach_retro_eval_stats": False,
        "retro_eval_stats": None,
        "attach_ime_keymap": False,
        "gji_keymap": None,
        "msime_key_assignment": None,
        "legacy_msime_keymap": None,
        "keymap_learn": None,
        "attach_running_processes": False,
        "running_processes": None,
        "reported_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
    }


def gzip_b64(data: bytes) -> str:
    return base64.b64encode(gzip.compress(data, mtime=0)).decode()


def incompressible_gzip_b64(target_chars: int) -> str:
    """gzip しても縮まらない（ランダムな）データで、base64 が約 `target_chars` 文字になるもの。"""
    # base64 は 4/3 倍。gzip のヘッダ・フッタ・ブロック枠の分を少し引く。
    raw = os.urandom(max(1, target_chars * 3 // 4 - 64))
    return gzip_b64(raw)


def post(endpoint: str, body: bytes) -> tuple[int, str, float]:
    request = urllib.request.Request(
        endpoint,
        data=body,
        method="POST",
        headers={"Content-Type": "application/json", "User-Agent": "awase-report-smoke/1.0"},
    )
    started = time.monotonic()
    try:
        with urllib.request.urlopen(request, timeout=60) as response:
            return response.status, response.read().decode(), time.monotonic() - started
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode(errors="replace"), time.monotonic() - started


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--endpoint", default=DEFAULT_ENDPOINT)
    parser.add_argument(
        "--typical-kib", type=int, default=400,
        help="3 の本体のおおよその大きさ（KiB、既定 400）",
    )
    parser.add_argument(
        "--large-kib", type=int, default=1800,
        help="3 の本体のおおよその大きさ（KiB、既定 1800。上限は 2048）",
    )
    args = parser.parse_args()

    small_v4 = base_payload(4)
    small_v4["log_excerpt_gz"] = gzip_b64(b'[{"seq":1,"entry":{"type":"ReportEdited"}}]')
    small_v4["app_log_excerpt_gz"] = gzip_b64(b"2026-10-04T00:00:00.000000Z INFO smoke\n")

    legacy_v3 = base_payload(3)
    legacy_v3["log_excerpt"] = '[{"seq":1}]'
    legacy_v3["app_log_excerpt"] = "smoke v3"

    typical_v4 = base_payload(4)
    typical_per_field = max(1024, (args.typical_kib * 1024 - 2048) // 2)
    typical_v4["log_excerpt_gz"] = incompressible_gzip_b64(typical_per_field)
    typical_v4["app_log_excerpt_gz"] = incompressible_gzip_b64(typical_per_field)

    large_v4 = base_payload(4)
    # 2 本に半分ずつ。他の項目の分（約 1KiB）を引く。
    per_field = max(1024, (args.large_kib * 1024 - 2048) // 2)
    large_v4["log_excerpt_gz"] = incompressible_gzip_b64(per_field)
    large_v4["app_log_excerpt_gz"] = incompressible_gzip_b64(per_field)

    both_v4 = base_payload(4)
    both_v4["log_excerpt"] = "plain"  # 4 では非圧縮フィールドを使わない
    both_v4["log_excerpt_gz"] = gzip_b64(b"[]")

    not_gzip = base_payload(4)
    not_gzip["log_excerpt_gz"] = base64.b64encode(b"plain text, not gzip..").decode()

    oversize = base_payload(4)
    oversize["description"] = "x" * (MAX_BODY_BYTES + 1024)  # 本文が 2MiB 超（Content-Length で弾かれる）

    cases = [
        ("1 v3 旧形式（非圧縮ログ）", legacy_v3, 201, None),
        ("2 v4 小さい報告", small_v4, 201, None),
        (f"3 v4 現実的な報告 (~{args.typical_kib}KiB)", typical_v4, 201, None),
        (f"4 v4 大きい報告 (~{args.large_kib}KiB)", large_v4, 201, None),
        ("5 v4 に非圧縮フィールド", both_v4, 400, "legacy_log_fields_not_allowed_in_schema_4"),
        ("6 gzip でない文字列", not_gzip, 400, "log_excerpt_gz_invalid"),
        ("7 本体が 2MiB 超", oversize, 413, "request_body_too_large"),
    ]

    failed = 0
    created: list[str] = []
    print(f"endpoint: {args.endpoint}\n")
    for name, payload, want_status, want_error in cases:
        body = json.dumps(payload).encode()
        status, text, elapsed = post(args.endpoint, body)
        ok = status == want_status
        detail = ""
        if status == 201:
            try:
                report_id = json.loads(text)["report_id"]
                created.append(report_id)
                detail = f"report_id={report_id}"
            except (ValueError, KeyError):
                ok = False
                detail = f"想定外の応答: {text[:120]}"
        elif want_error is not None:
            ok = ok and want_error in text
            detail = text[:120]
        else:
            detail = text[:120]
        failed += 0 if ok else 1
        print(f"[{'OK' if ok else 'NG'}] {name}: {status} ({len(body) / 1024:.0f}KiB, {elapsed * 1000:.0f}ms) {detail}")

    print()
    if created:
        print("保存された報告（削除する場合は README の手順で `wrangler r2 object delete`）:")
        for report_id in created:
            print(f"  {report_id}")
    if failed:
        print(f"\n{failed} 件が想定と違います。")
        return 1
    print("\nすべて想定どおりです。")
    return 0


if __name__ == "__main__":
    sys.exit(main())
