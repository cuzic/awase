"""ADR-280: 仮名の判定に 3 連(トライグラム)を使うと 2 連より改善するかの評価。

評価の枠組みは eval_eisu_threshold.py と同じ(#819 以後の判定: 仮名に読めないキーは増分 0 で文脈を切る、
親指なし区間、`ja_threshold = 4`・`max_keys = 7`、固定 seed の標本)。2 連は `KanaModel::delta` の再現、
3 連は次の 2 通りを足す(背景は同じ ln(1/N_keys))。
  tri-backoff: 直前 2 文字が仮名で、接頭辞 ab が 3 連の表にあれば P3(c|ab) = f(abc) / (Σ_x f(abx) + (V - 列挙数) * floor3)
               (表に無い abc は floor3)。接頭辞が表に無ければ 2 連の式にフォールバック。
  tri-interp : 同じ条件で P = lam * P3 + (1 - lam) * P2(2 連)。接頭辞が表に無ければ P2。
floor3 = 3 連の最小頻度の 1/10。3 連と 2 連の score は別々に最大 = 1 に正規化されているが、条件付き確率は
頻度の比なので正規化の尺度は打ち消される(floor3 だけが尺度に依存し、表の最小値から作る)。
評価データは --data-dir で渡す(リポジトリに入れない)。

実行: uv run --no-project python -I tools/lang-detect-eval/eval_trigram.py --repo . --data-dir <dir>
"""
import argparse, collections, glob, gzip, json, math, random, re, statistics, unicodedata

ap = argparse.ArgumentParser()
ap.add_argument('--repo', default='.')
ap.add_argument('--data-dir', required=True)
ap.add_argument('--lam', type=float, default=0.7, help='補間の 3 連の重み')
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



tri = {}
for l in gzip.open(f'{a.repo}/data/ngram_hiragana.csv.gz', 'rt'):
    if l.startswith('#'):
        continue
    t, c, s = l.strip().split(',')
    if t == '3' and len(c) == 3:
        tri[c] = 10 ** float(s)
pre = collections.defaultdict(float)
pre_n = collections.Counter()
for c, f in tri.items():
    pre[c[:2]] += f
    pre_n[c[:2]] += 1
floor3 = min(tri.values()) * 0.1
print(f'bigrams {len(freq)}, trigrams {len(tri)}, prefixes {len(pre)}, floor2 {floor:.3e}, floor3 {floor3:.3e}')


def p2(prev, tok):
    if prev in vocab:
        den = left.get(prev, 0) + (V - listed[prev]) * floor
        return freq.get((prev, tok), floor) / den
    return (left.get(tok, 0) + floor) / (V * floor + total)


def p3(prev2, prev, tok):
    ab = prev2 + prev
    if ab not in pre:
        return None
    den = pre[ab] + (V - pre_n[ab]) * floor3
    return tri.get(ab + tok, floor3) / den


def delta(model, prev2, prev, tok):
    q2 = p2(prev, tok)
    if model == 'bi' or prev2 is None or prev is None:
        return math.log(q2) - bg
    q3 = p3(prev2, prev, tok)
    if q3 is None:
        return math.log(q2) - bg
    if model == 'tri-backoff':
        return math.log(q3) - bg
    return math.log(a.lam * q3 + (1 - a.lam) * q2) - bg


def traj(seq, model):
    """打鍵ごとの累積 llr の列(最大 max_keys)。仮名に読めないキーは 0 で文脈を切る"""
    prev2 = prev = None
    llr, out = 0.0, []
    for k in seq[:a.max_keys]:
        t = kana_of.get(k, 'NONE')
        if t in ('SYM', 'NONE') or t not in vocab:
            d, prev2, prev = 0.0, None, None
        else:
            d = delta(model, prev2, prev, t)
            prev2, prev = prev, t
        llr += d
        out.append(llr)
    return out


def verdict(cum, en_th, ja_th=None):
    ja_th = a.ja_threshold if ja_th is None else ja_th
    for i, v in enumerate(cum, 1):
        if v >= ja_th:
            return 'ja', i
        if v <= -en_th:
            return 'en', i
    return 'und', len(cum)


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


G = {g: en_runs(t) for g, t in groups.items()}
print(f'ja sentences {len(JA)}, ja episodes {len(runs)}, with unshifted prefix {len(nz)}')
for g, r in G.items():
    print(f'{g}: texts {len(groups[g])}, runs {len(r)}')

MODELS = ['bi', 'tri-backoff', 'tri-interp']
TJ = {m: [traj(s, m) for s in nz] for m in MODELS}
TG = {m: {g: [traj(s, m) for s in rs] for g, rs in G.items()} for m in MODELS}

# 増分の尺度の連続性: 3 連が使われた打鍵と 2 連へのフォールバックの打鍵の増分の分布(日本語区間)
print('\n[尺度] 日本語区間の打鍵ごとの増分(nats)。2 打目(文脈 1 文字=2 連)と 3 打目以降(3 連が使える)')
for m in MODELS:
    for lab, idx in (('1打目', 0), ('2打目', 1), ('3打目', 2), ('4打目', 3)):
        ds = []
        for c in TJ[m]:
            if len(c) > idx:
                ds.append(c[idx] - (c[idx - 1] if idx else 0.0))
        if ds:
            ds.sort()
            print(f'{m:12} {lab} n={len(ds)} 平均 {statistics.mean(ds):+.2f} 中央 {statistics.median(ds):+.2f} p5 {ds[len(ds)//20]:+.2f} min {ds[0]:+.1f}')
used = fb = 0
for s in nz:
    prev2 = prev = None
    for k in s[:a.max_keys]:
        t = kana_of.get(k, 'NONE')
        if t in ('SYM', 'NONE') or t not in vocab:
            prev2 = prev = None
            continue
        if prev2 is not None:
            if p3(prev2, prev, t) is None:
                fb += 1
            else:
                used += 1
        prev2, prev = prev, t
print(f'3 連の文脈が揃った打鍵 {used + fb} のうち、接頭辞が表にあり 3 連を使えた {used} ({used/(used+fb)*100:.1f}%)、2 連へフォールバック {fb}')

print('\n(a) 閾値別: model | en_th | JA→英数誤 | 英文 NotJapanese率(平均打鍵) Undecided | コード | 他言語')
for th in (8, 14, 16):
    for m in MODELS:
        n = len(nz)
        err = sum(verdict(c, th)[0] == 'en' for c in TJ[m]) / n * 100
        cols = []
        for g in G:
            rr = [verdict(c, th) for c in TG[m][g]]
            e = [r[1] for r in rr if r[0] == 'en']
            cols.append(f"{g} {len(e)/len(rr)*100:5.1f}%({statistics.mean(e) if e else 0:.2f}) U{sum(r[0]=='und' for r in rr)/len(rr)*100:4.1f}%")
        print(f'{m:12} | {th:2} | {err:5.2f}% | ' + ' | '.join(cols))

THS = [4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 18, 20, 22, 24, 26, 28, 30]
curve = {}
print('\n(b) トレードオフ曲線: model | en_th | JA→英数誤 | 英文決まる率 | コード | 他言語')
for m in MODELS:
    curve[m] = []
    for th in THS:
        err = sum(verdict(c, th)[0] == 'en' for c in TJ[m]) / len(nz) * 100
        rates = {g: sum(verdict(c, th)[0] == 'en' for c in TG[m][g]) / len(TG[m][g]) * 100 for g in G}
        curve[m].append((th, err, rates))
        print(f"{m:12} | {th:2} | {err:6.2f}% | " + ' | '.join(f'{rates[g]:5.1f}%' for g in G))


def at_err(m, target, g):
    pts = sorted(curve[m], key=lambda x: x[1])
    for (t0, e0, r0), (t1, e1, r1) in zip(pts, pts[1:]):
        if e0 <= target <= e1 and e1 > e0:
            w = (target - e0) / (e1 - e0)
            return r0[g] + w * (r1[g] - r0[g])
    return None


print('\n(b) 同じ誤判定率のときの決まる率(線形補間): 誤判定率 | 群 | ' + ' | '.join(MODELS))
for tgt in (0.5, 1.0, 2.0, 5.0):
    for g in G:
        vals = [at_err(m, tgt, g) for m in MODELS]
        print(f'{tgt:4.1f}% | {g} | ' + ' | '.join('-' if v is None else f'{v:5.1f}%' for v in vals))

print('\n(c) 先頭 k 打以内に NotJapanese に達した日本語区間の率(en_th=8 / 14): k | model | en_th=8 | en_th=14')
for k in (1, 2, 3):
    for m in MODELS:
        r = []
        for th in (8, 14):
            r.append(sum(1 for c in TJ[m] if verdict(c, th)[0] == 'en' and verdict(c, th)[1] <= k) / len(nz) * 100)
        print(f'{k} | {m:12} | {r[0]:5.2f}% | {r[1]:5.2f}%')

print('\n(d) 英文・コード・他言語を Japanese と判定する率(ja_threshold=4、英数側は決めない): model | 群 | 率')
for m in MODELS:
    for g in G:
        rr = [verdict(c, 1e9)[0] == 'ja' for c in TG[m][g]]
        print(f'{m:12} | {g} | {sum(rr)/len(rr)*100:5.2f}%')
print('日本語区間で尤度だけで Japanese になる率(en 側を決めない):', {m: round(sum(verdict(c, 1e9)[0] == 'ja' for c in TJ[m]) / len(nz) * 100, 2) for m in MODELS})
