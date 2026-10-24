"""ADR-280 C0: `eisu_threshold` を上げたときの、日本語の誤判定と英数側の保持の長さの評価。

#819 以後の `KanaModel::delta`(仮名に読めないキーは増分 0・直前の仮名の文脈を切る)を再現し、
`en_threshold`(入りの実効の閾値 = max(eisu_threshold, en_threshold))ごとに集計する。
元は research-private/v3-eval/eval8_symbols.py の案 B。評価データ(利用者の本文を含まない生成データ)は
リポジトリに入れず、`--data-dir` で渡す(`data_*.jsonl`・`data2_*.jsonl`・`data3_*.jsonl`。1 行 1 JSON で
`lang`・`text`・日本語は `reading`、data3 は `genre`)。

実行: uv run -I tools/lang-detect-eval/eval_eisu_threshold.py --repo . --data-dir <dir> [--thresholds 14,16,18,20,24]
"""
import argparse, collections, glob, gzip, json, math, random, re, statistics, unicodedata

ap = argparse.ArgumentParser()
ap.add_argument('--repo', default='.')
ap.add_argument('--data-dir', required=True)
ap.add_argument('--thresholds', default='8,10,12,14,16,18,20,24')
ap.add_argument('--max-keys', type=int, default=7)
ap.add_argument('--ja-threshold', type=float, default=4.0)
a = ap.parse_args()
random.seed(5)


def nf(s):
    return unicodedata.normalize('NFKC', s)


def read_planes(path):
    planes, cur = {}, None
    for ln in open(path, encoding='utf-8').read().splitlines():
        ln = ln.strip()
        if ln.startswith(';') or not ln:
            continue
        m = re.match(r'\[(.*)\]', ln)
        if m:
            cur = m.group(1)
            planes[cur] = []
            continue
        planes[cur].append([nf(x.strip().strip("'")) for x in ln.split(',')])
    return planes


planes = read_planes(f'{a.repo}/layout/nicola.yab')
PN = {'N': 'ローマ字シフト無し', 'L': 'ローマ字左親指シフト', 'R': 'ローマ字右親指シフト'}
rom2key = {}
for k, pn in PN.items():
    for r, row in enumerate(planes[pn]):
        for c, v in enumerate(row):
            if re.fullmatch(r'[a-z]{1,3}', v) and v not in rom2key:
                rom2key[v] = (k, r, c)
base = {'あ':'a','い':'i','う':'u','え':'e','お':'o','か':'ka','き':'ki','く':'ku','け':'ke','こ':'ko','さ':'sa','し':'si','す':'su','せ':'se','そ':'so','た':'ta','ち':'ti','つ':'tu','て':'te','と':'to','な':'na','に':'ni','ぬ':'nu','ね':'ne','の':'no','は':'ha','ひ':'hi','ふ':'fu','へ':'he','ほ':'ho','ま':'ma','み':'mi','む':'mu','め':'me','も':'mo','や':'ya','ゆ':'yu','よ':'yo','ら':'ra','り':'ri','る':'ru','れ':'re','ろ':'ro','わ':'wa','を':'wo','ん':'nn','が':'ga','ぎ':'gi','ぐ':'gu','げ':'ge','ご':'go','ざ':'za','じ':'zi','ず':'zu','ぜ':'ze','ぞ':'zo','だ':'da','ぢ':'di','づ':'du','で':'de','ど':'do','ば':'ba','び':'bi','ぶ':'bu','べ':'be','ぼ':'bo','ぱ':'pa','ぴ':'pi','ぷ':'pu','ぺ':'pe','ぽ':'po','っ':'ltu','ゃ':'lya','ゅ':'lyu','ょ':'lyo','ぁ':'la','ぃ':'li','ぅ':'lu','ぇ':'le','ぉ':'lo','ゔ':'vu'}
punct = {'、': ('N', 1, 10), '。': ('N', 1, 0), 'ー': ('L', 3, 1)}


def ja_keys(reading):
    out = []
    for ch in reading:
        if ch in punct:
            out.append(punct[ch])
            continue
        r = base.get(ch)
        if r is None or r not in rom2key:
            return None
        out.append(rom2key[r])
    return out


inv = {v: k for k, v in base.items()}
rows = ['1234567890-', "qwertyuiop@[", 'asdfghjkl;:]', 'zxcvbnm,./']
pos = {ch: (r, c) for r, row in enumerate(rows) for c, ch in enumerate(row)}

# 未シフト面(keytop)の位置 -> 仮名 / SYM / NONE
face = []
for ln in open(f'{a.repo}/layout/nicola_keytop.yab', encoding='utf-8').read().split('[ローマ字左親指シフト]')[0].splitlines():
    ln = ln.strip()
    if not ln or ln.startswith(';') or ln.startswith('['):
        continue
    face.append([nf(x.strip().strip("'")) for x in ln.split(',')])
kana_of = {}
for r, row in enumerate(face):
    for c, v in enumerate(row):
        kana_of[(r, c)] = 'NONE' if v == '無' else inv.get(v, 'SYM')
defined = sum(1 for v in kana_of.values() if v != 'NONE')

freq = {}
for l in gzip.open(f'{a.repo}/data/ngram_hiragana.csv.gz', 'rt'):
    if l.startswith('#'):
        continue
    t, c, s = l.strip().split(',')
    if t == '2' and len(c) == 2:
        freq[(c[0], c[1])] = 10 ** float(s)
left = collections.defaultdict(float)
listed = collections.Counter()
vocab = set()
for (x, y), f in freq.items():
    left[x] += f
    listed[x] += 1
    vocab |= {x, y}
floor = min(freq.values()) * 0.1
total = sum(left.values())
V = len(vocab)
bg = -math.log(defined)


def ja_delta(prev, tok):
    if prev in vocab:
        den = left.get(prev, 0) + (V - listed[prev]) * floor
        ja = math.log(freq.get((prev, tok), floor) / den)
    else:
        ja = math.log((left.get(tok, 0) + floor) / (V * floor + total))
    return ja - bg


def run(seq, en_th):
    """#819 以後の EpisodeDetector(親指なし)。戻り値 (verdict, 打鍵数)"""
    prev, llr = None, 0.0
    for i, k in enumerate(seq[:a.max_keys], 1):
        t = kana_of.get(k, 'NONE')
        if t in ('SYM', 'NONE') or t not in vocab:
            d, prev = 0.0, None
        else:
            d, prev = ja_delta(prev, t), t
        llr += d
        if llr >= a.ja_threshold:
            return 'ja', i
        if llr <= -en_th:
            return 'en', i
    return 'und', min(len(seq), a.max_keys)


def load(g):
    o = []
    for f in sorted(glob.glob(f'{a.data_dir}/{g}')):
        for ln in open(f, encoding='utf-8'):
            try:
                o.append(json.loads(ln))
            except Exception:
                pass
    return o


D = load('data_*.jsonl') + load('data2_*.jsonl')
D3 = load('data3_*.jsonl')
JA = [d for d in D if d['lang'] == 'ja' and 'reading' in d]
runs = []
for d in JA:
    ks = ja_keys(re.sub(r'\s', '', d['reading']))
    if not ks:
        continue
    for st in [0] + [random.randrange(0, len(ks)) for _ in range(3)]:
        seq = []
        for pl, r, c in ks[st:]:
            if pl != 'N':
                break
            seq.append((r, c))
        runs.append(seq)
nz = [s for s in runs if s]
groups = collections.defaultdict(list)
for d in D:
    if d['lang'] == 'en':
        groups['英文'].append(d['text'])
for d in D3:
    groups['コード' if d['genre'] == 'code' else '他言語(eu/ms)'].append(d['text'])


def en_runs(texts):
    o = []
    for t in texts:
        w = t.split()
        for st in [0] + [random.randrange(0, max(1, len(w))) for _ in range(2)]:
            ks = [pos[ch] for ch in ' '.join(w[st:]).lower() if ch in pos]
            if ks:
                o.append(ks)
    return o


G = {g: en_runs(t) for g, t in groups.items()}  # 閾値間で同じ標本を使う
print(f'ja sentences {len(JA)}, ja episodes {len(runs)}, with unshifted prefix {len(nz)}')
for g, r in G.items():
    print(f'{g}: texts {len(groups[g])}, runs {len(r)}')


def pct(xs, p):
    xs = sorted(xs)
    return xs[min(len(xs) - 1, math.ceil(p * len(xs)) - 1)]


print('\nen_th | JA→英数誤 | JA→日本語 | JA未決')
ths = [float(x) for x in a.thresholds.split(',')]
for th in ths:
    res = [run(s, th) for s in nz]
    n = len(res)
    print(f"{th:5.0f} | {sum(r[0]=='en' for r in res)/n*100:8.2f}% | {sum(r[0]=='ja' for r in res)/n*100:8.1f}% | {sum(r[0]=='und' for r in res)/n*100:5.1f}%")
print('\n他言語: en_th | 群 | NotJapanese率 | 平均/中央/p90 打鍵 | Undecided率 | Japanese率')
for th in ths:
    for g, rs in G.items():
        rr = [run(s, th) for s in rs]
        e = [r[1] for r in rr if r[0] == 'en']
        n = len(rr)
        st = f'{statistics.mean(e):.2f}/{statistics.median(e):.0f}/{pct(e,0.9)}' if e else '-'
        print(f"{th:5.0f} | {g} | {len(e)/n*100:5.1f}% | {st} | {sum(r[0]=='und' for r in rr)/n*100:5.1f}% | {sum(r[0]=='ja' for r in rr)/n*100:4.1f}%")
