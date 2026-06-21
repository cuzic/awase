#!/usr/bin/env python3
"""CHANGELOG.md → docs/changelog.html を生成する。

使い方:
  uv run --with mistune scripts/gen_changelog.py
"""

import re
import sys
from pathlib import Path

try:
    import mistune
except ImportError:
    print("mistune が見つかりません。uv run --with mistune で実行してください。", file=sys.stderr)
    sys.exit(1)

ROOT = Path(__file__).resolve().parent.parent

NAV = """\
<nav class="site-nav">
  <div class="inner">
    <span class="brand">awase（合わせ）</span>
    <a href="index.html">トップ</a>
    <a href="usage.html">使い方</a>
    <a href="internals.html">内部動作</a>
    <a href="changelog.html" class="active">更新履歴</a>
    <a href="https://github.com/cuzic/awase">GitHub</a>
  </div>
</nav>"""

TEMPLATE = """\
<!DOCTYPE html>
<html lang="ja">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>更新履歴 — awase（合わせ）</title>
<meta name="description" content="awase のバージョン別更新履歴。バグ修正・新機能・内部改善の一覧。">
<style>
*,*::before,*::after{{box-sizing:border-box;margin:0;padding:0}}
:root{{
  --accent:#2563EB;
  --accent-light:#3B82F6;
  --bg:#ffffff;
  --bg-alt:#F8FAFC;
  --text:#1E293B;
  --text-secondary:#475569;
  --border:#E2E8F0;
  --card-shadow:0 1px 3px rgba(0,0,0,0.08),0 1px 2px rgba(0,0,0,0.06);
}}
@media(prefers-color-scheme:dark){{
  :root{{
    --bg:#0F172A;
    --bg-alt:#1E293B;
    --text:#E2E8F0;
    --text-secondary:#94A3B8;
    --border:#334155;
  }}
}}
html{{scroll-behavior:smooth}}
body{{font-family:system-ui,-apple-system,sans-serif;line-height:1.7;color:var(--text);background:var(--bg)}}
a{{color:var(--accent);text-decoration:none}}
a:hover{{text-decoration:underline}}
.container{{max-width:800px;margin:0 auto;padding:0 1.5rem}}
.site-nav{{background:var(--bg-alt);border-bottom:1px solid var(--border);padding:0.75rem 1.5rem}}
.site-nav .inner{{max-width:860px;margin:0 auto;display:flex;align-items:center;gap:1.5rem}}
.site-nav .brand{{font-weight:700;font-size:1rem;color:var(--text)}}
.site-nav a{{font-size:0.92rem;color:var(--text-secondary)}}
.site-nav a:hover{{color:var(--accent);text-decoration:none}}
.site-nav a.active{{color:var(--accent);font-weight:600}}
.page-header{{padding:3.5rem 1.5rem 2.5rem;background:var(--bg-alt);border-bottom:1px solid var(--border);text-align:center}}
.page-header h1{{font-size:2rem;font-weight:800;letter-spacing:-0.02em;color:var(--text)}}
.page-header p{{font-size:1rem;color:var(--text-secondary);margin-top:0.5rem}}
section{{padding:4rem 0}}
/* Markdown-rendered changelog */
.cl-content h2{{
  font-size:1.4rem;font-weight:800;
  font-family:ui-monospace,SFMono-Regular,Consolas,monospace;
  border-bottom:2px solid var(--border);
  padding-bottom:0.5rem;margin:2.5rem 0 1rem;color:var(--text)
}}
.cl-content h2:first-child{{margin-top:0}}
.cl-content h3{{
  font-size:0.8rem;font-weight:700;
  text-transform:uppercase;letter-spacing:0.06em;
  color:var(--text-secondary);margin:1.2rem 0 0.5rem
}}
.cl-content ul{{list-style:none;padding:0}}
.cl-content > ul > li,
.cl-content ul > li{{
  position:relative;padding:0.35rem 0 0.35rem 1.4em;
  font-size:0.95rem;border-bottom:1px solid var(--border)
}}
.cl-content ul > li:last-child{{border-bottom:none}}
.cl-content ul > li::before{{
  content:"\\25B8";position:absolute;left:0;
  color:var(--accent-light);font-size:0.75rem;top:0.55rem
}}
.cl-content ul ul{{margin-top:0.3rem}}
.cl-content ul ul > li{{
  border-bottom:none;font-size:0.875rem;
  color:var(--text-secondary);padding-top:0.15rem;padding-bottom:0.15rem
}}
.cl-content ul ul > li::before{{content:"\\2013";top:0.3rem}}
.cl-content hr{{border:none;border-top:1px solid var(--border);margin:2.5rem 0}}
.cl-content p{{font-size:0.95rem;color:var(--text-secondary);margin:0.5rem 0}}
.cl-content code{{
  font-family:ui-monospace,SFMono-Regular,Consolas,monospace;
  font-size:0.85em;background:var(--bg-alt);
  padding:0.1em 0.35em;border-radius:3px;border:1px solid var(--border)
}}
.cl-content strong{{color:var(--text);font-weight:700}}
footer{{
  background:var(--bg-alt);border-top:1px solid var(--border);
  padding:2rem 1.5rem;text-align:center;
  font-size:0.875rem;color:var(--text-secondary)
}}
footer p{{margin:0.3rem 0}}
</style>
</head>
<body>

{nav}

<div class="page-header">
  <h1>更新履歴</h1>
  <p>awase の各バージョンの変更内容。ダウンロードは <a href="https://github.com/cuzic/awase/releases">GitHub Releases</a> から。</p>
</div>

<section>
<div class="container">
<div class="cl-content">
{body}
</div>
</div>
</section>

<footer>
  <p><a href="index.html">トップページ</a> | <a href="usage.html">使い方</a> | <a href="https://github.com/cuzic/awase">GitHub</a></p>
  <p>License: MIT / Apache 2.0 &nbsp;|&nbsp; Created by <a href="https://github.com/cuzic">cuzic</a></p>
</footer>

</body>
</html>
"""


def convert(src: str) -> str:
    lines = src.splitlines()

    # ## から始まる最初の見出し以前（タイトル・説明文）を除去する
    start = next((i for i, l in enumerate(lines) if l.startswith("## ")), 0)

    # 末尾の参照リンク定義行（[1.1.1]: https://...）を除去する
    body_lines = [
        l for l in lines[start:]
        if not re.match(r"^\[.*\]:\s+https?://", l)
    ]

    body_md = "\n".join(body_lines)
    return mistune.html(body_md)


def main() -> None:
    src = (ROOT / "CHANGELOG.md").read_text(encoding="utf-8")
    body_html = convert(src)
    output = TEMPLATE.format(nav=NAV, body=body_html)
    dest = ROOT / "docs" / "changelog.html"
    dest.write_text(output, encoding="utf-8")
    print(f"Generated {dest.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
