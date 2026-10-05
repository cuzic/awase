#!/usr/bin/env python3
"""e2e の 1 回(1 run)が「検査対象の挙動以外の要因で汚れていないか」を数える(標準ライブラリだけ)。

BUG-147 では、物理入力(人の操作)の混入を見落として「awase が原因」と誤認し、後で撤回した。
CI でも、フォーカス復帰(各 check_*.py が見ている)以外に次の 2 つが結果を汚しうる:

  foreign_physical  awase.log の `[engine-input] vk=… KeyDown … extra=0x0`(注入マーカー無し=物理キー、または
                    マーカーを付けない注入)の件数。スパイク・typing_stress は dwExtraInfo にマーカーを付けるので、
                    通常は 0。chrome_probe など一部の注入は extra=0 のままなので、強制する前に件数を観察する。
  cpu_load          cpu.csv(`ISO時刻,CPU%`、cpu_sampler.ps1 が 1 秒周期で記録)の p95 が閾値を超えた。
                    負荷が高いと打鍵間隔・IME 応答の時間窓が崩れ、「起きた/起きない」が環境依存になる。

使い方: check_run_validity.py [--cpu cpu.csv] [--cpu-p95-limit 90] [--json out.json] awase.log
出力: `VALIDITY: verdict=CLEAN|CONTAMINATED foreign_down=N cpu_p95=… cpu_max=… cpu_n=…`
終了コード: 0=CLEAN / 3=CONTAMINATED(INVALID にすべき) / 2=使い方の誤り
CPU ログが無い・空のときは負荷では汚れ扱いにしない(cpu_n=0 と出すだけ)。
"""
import json
import re
import sys

ENGINE_INPUT_RE = re.compile(r"\[engine-input\] vk=0x(\w+) (KeyDown|KeyUp) .*?\bextra=0x(\w+)")
DEFAULT_CPU_P95_LIMIT = 90.0


def foreign_physical(awase_lines):
    """(KeyDown の件数, KeyUp の件数, 最初の数行) を返す。extra=0 のものだけ。"""
    down = up = 0
    samples = []
    for ln in awase_lines:
        m = ENGINE_INPUT_RE.search(ln)
        if not m or int(m.group(3), 16) != 0:
            continue
        if m.group(2) == "KeyDown":
            down += 1
        else:
            up += 1
        if len(samples) < 5:
            samples.append(ln.strip()[:160])
    return down, up, samples


def cpu_stats(cpu_lines):
    """cpu.csv の行から (件数, 平均, p95, 最大) を返す。壊れた行は無視。件数 0 なら全て None。"""
    vals = []
    for ln in cpu_lines:
        parts = ln.strip().split(",")
        if len(parts) != 2:
            continue
        try:
            vals.append(float(parts[1]))
        except ValueError:
            continue
    if not vals:
        return 0, None, None, None
    vals.sort()
    p95 = vals[min(len(vals) - 1, int(0.95 * len(vals)))]
    return len(vals), sum(vals) / len(vals), p95, vals[-1]


def analyze(awase_lines, cpu_lines, cpu_p95_limit=DEFAULT_CPU_P95_LIMIT):
    down, up, samples = foreign_physical(awase_lines)
    n, mean, p95, mx = cpu_stats(cpu_lines)
    reasons = []
    if down > 0:
        reasons.append(f"物理キー(extra=0)の混入 KeyDown={down}")
    if p95 is not None and p95 > cpu_p95_limit:
        reasons.append(f"CPU 負荷 p95={p95:.0f}% > {cpu_p95_limit:.0f}%")
    return {
        "verdict": "CONTAMINATED" if reasons else "CLEAN",
        "reasons": reasons,
        "foreign_down": down,
        "foreign_up": up,
        "foreign_samples": samples,
        "cpu_n": n,
        "cpu_mean": mean,
        "cpu_p95": p95,
        "cpu_max": mx,
    }


def _read(path):
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            return f.read().splitlines()
    except OSError:
        return []


def main(argv):
    cpu_path = None
    json_path = None
    limit = DEFAULT_CPU_P95_LIMIT
    pos = []
    it = iter(argv)
    for a in it:
        if a == "--cpu":
            cpu_path = next(it, None)
        elif a == "--json":
            json_path = next(it, None)
        elif a == "--cpu-p95-limit":
            try:
                limit = float(next(it, ""))
            except ValueError:
                print(__doc__)
                return 2
        else:
            pos.append(a)
    if len(pos) != 1:
        print(__doc__)
        return 2
    r = analyze(_read(pos[0]), _read(cpu_path) if cpu_path else [], limit)

    def f(v):
        return "-" if v is None else f"{v:.0f}"

    print(f"VALIDITY: verdict={r['verdict']} foreign_down={r['foreign_down']} foreign_up={r['foreign_up']} "
          f"cpu_p95={f(r['cpu_p95'])} cpu_max={f(r['cpu_max'])} cpu_n={r['cpu_n']}")
    for s in r["foreign_samples"]:
        print(f"  foreign: {s}")
    for why in r["reasons"]:
        print(f"  理由: {why}")
    if json_path:
        with open(json_path, "w", encoding="utf-8") as fp:
            json.dump(r, fp, ensure_ascii=False, indent=2)
    return 0 if r["verdict"] == "CLEAN" else 3


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
