"""ADR-280: 背景をラテン文字の 3 連にし、仮名側も 3 連にする(2 x 3 の表)と、英単語(windows・hello・google など)が拾えるか。
#847 の eval_latin_bigram.py に、ラテン 3 連(L3: 1 文字目は 1 連・2 文字目は 2 連・3 文字目以降は 3 連、表に無い・希薄な並びは 2 連へ add-k 補間 P3=(n(abc)+k*P2(c|b))/(n(ab.)+k))、
CV3(クラス 3 連)、学習量(10%/30%)、英語のみ(L3_en)を足したもの。実行: uv run --no-project python -I tools/lang-detect-eval/eval_latin_trigram.py --repo . --data-dir <dir>

打鍵ごとの増分 = ln P_ja(仮名 | 直前の仮名) − ln P_bg(文字 | 直前の文字)。仮名に読めないキーは 0 で文脈を切る
(#819 以後)。背景:
  U   : 一様分布 1/N_keys(現行)
  E_* : 英語の文字 2 連。出典が違う 3 通り(repo=このリポジトリの英語・識別子、dict=/usr/share/dict/words、en=評価データの英文の学習側)
  L   : 多言語ラテン(評価データの英文・欧州言語・マレー語・コードの学習側を混ぜた 2 連)
  CV  : 母音(a e i o u)/子音/その他のキーの 3 クラスの遷移確率 × クラス内一様
  CV5 : 母音/鼻音(m n)/流音(l r)/破裂音(p b t d k g c q)/その他の子音/その他のキーの 6 クラス
  LCV : L と CV の確率の混合(--mix)
仮名側は 2 連(bi)と 3 連(tri-backoff・tri-interp。eval_trigram.py と同じ)。
評価データの非日本語は文ごとに学習側(偶数番)と評価側(奇数番)に分け、評価は全背景で奇数番だけに揃える。
データは --data-dir で渡す(リポジトリに入れない)。表は --repo の git ls-files と /usr/share/dict/words から再生成できる。

実行: uv run --no-project python -I tools/lang-detect-eval/eval_latin_bigram.py --repo . --data-dir <dir>
"""
import argparse, collections, glob, gzip, json, math, random, re, statistics, unicodedata

ap = argparse.ArgumentParser()
ap.add_argument('--repo', default='.')
ap.add_argument('--data-dir', required=True)
ap.add_argument('--lam', type=float, default=0.7, help='補間の 3 連の重み')
ap.add_argument('--mix', type=float, default=0.5, help='LCV の L の重み')
ap.add_argument('--k3', type=float, default=4.0, help='3 連の補間の強さ k')
ap.add_argument('--alpha', type=float, default=0.5, help='背景の文字 2 連の加算平滑')
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



import os, subprocess

ALPH = sorted(pos)           # 打鍵位置の格子の文字(数字・英字・記号)。仮名のキーはここに含まれる
rev = {v: k for k, v in pos.items()}
NA = len(ALPH)


def keys_of(text):
    return [ch for ch in text.lower() if ch in pos]  # 評価の en_runs と同じ(空白・範囲外は飛ばす)


# ---------- 学習データ(非日本語は偶数番を学習・奇数番を評価) ----------
def load_f(g):
    o = []
    for f in sorted(glob.glob(f'{a.data_dir}/{g}')):
        for ln in open(f, encoding='utf-8'):
            try:
                d = json.loads(ln)
            except Exception:
                continue
            d['_file'] = os.path.basename(f)
            o.append(d)
    return o


DALL = load_f('data_*.jsonl') + load_f('data2_*.jsonl') + load_f('data3_*.jsonl')
JA = [d for d in DALL if d['lang'] == 'ja' and 'reading' in d]
SUB = collections.defaultdict(list)       # 言語別 -> 文のリスト(読み込み順)
for d in DALL:
    if d['lang'] == 'en':
        SUB['en'].append(d['text'])
    elif d.get('genre') == 'code':
        SUB['code'].append(d['text'])
    elif d.get('genre') in ('eu', 'ms'):
        SUB[d.get('sub', d['genre'])].append(d['text'])
TRAIN = {s: [t for i, t in enumerate(ts) if i % 2 == 0] for s, ts in SUB.items()}
TEST = {s: [t for i, t in enumerate(ts) if i % 2 == 1] for s, ts in SUB.items()}
AGG = {'英文': ['en'], 'コード': ['code'], '他言語(eu/ms)': [s for s in SUB if s not in ('en', 'code')]}


def counts_from_streams(streams, cross=True):
    bi = collections.Counter()
    uni = collections.Counter()
    for ks in streams:
        for i, ch in enumerate(ks):
            uni[ch] += 1
            if i:
                bi[(ks[i - 1], ch)] += 1
    return bi, uni


def table_from(bi, uni):
    """加算平滑の P(c | prev) と P(c)"""
    tot = sum(uni.values())
    pu = {c: (uni[c] + a.alpha) / (tot + a.alpha * NA) for c in ALPH}
    row = collections.Counter()
    for (p, c), n in bi.items():
        row[p] += n
    pb = {}
    for p in ALPH:
        den = row[p] + a.alpha * NA
        for c in ALPH:
            pb[(p, c)] = (bi.get((p, c), 0) + a.alpha) / den
    return pb, pu


# repo
def repo_streams():
    files = subprocess.run(['git', '-C', a.repo, 'ls-files', '*.md', '*.rs'], capture_output=True, text=True).stdout.split()
    n = 0
    out = []
    for f in files:
        if f.startswith(('tests/fixtures', 'data/', 'tools/lang-detect-eval-fixture', 'docs/tasks/adr280')):
            continue
        try:
            txt = open(f'{a.repo}/{f}', encoding='utf-8').read()
        except Exception:
            continue
        n += 1
        for ln in txt.splitlines():
            ln = re.sub(r'([a-z])([A-Z])', r'\1 \2', ln)
            ws = re.findall(r'[A-Za-z]+', ln)
            if ws:
                out.append(keys_of(''.join(ws)))
    return out, n


def dict_streams():
    p = '/usr/share/dict/american-english'
    ws = [w.strip() for w in open(p, encoding='utf-8') if re.fullmatch(r"[A-Za-z]+", w.strip())]
    return [keys_of(w) for w in ws], len(ws)


rs, nrepo = repo_streams()
ds, ndict = dict_streams()
en_tr = [keys_of(t) for t in TRAIN['en']]
L_tr = [keys_of(t) for s, ts in TRAIN.items() for t in ts]
print(f'train: repo files {nrepo} streams {len(rs)} keys {sum(map(len, rs))}; dict words {ndict} keys {sum(map(len, ds))}; '
      f'en-train texts {len(TRAIN["en"])}; L-train texts {sum(len(t) for t in TRAIN.values())} keys {sum(map(len, L_tr))}')
BG = {}
BG['E_repo'] = table_from(*counts_from_streams(rs))
BG['E_dict'] = table_from(*counts_from_streams(ds))
BG['E_en'] = table_from(*counts_from_streams(en_tr))
BG['L'] = table_from(*counts_from_streams(L_tr))


# ---------- クラス背景 ----------
def cls_cv(ch):
    return 'V' if ch in 'aeiou' else ('C' if ch.isalpha() else 'X')


def cls_cv5(ch):
    if ch in 'aeiou':
        return 'V'
    if ch in 'mn':
        return 'N'
    if ch in 'lr':
        return 'R'
    if ch in 'pbtdkgcq':
        return 'S'
    return 'C' if ch.isalpha() else 'X'


def class_table(streams, cf):
    cl = {c: cf(c) for c in ALPH}
    size = collections.Counter(cl.values())
    bi = collections.Counter()
    uni = collections.Counter()
    for ks in streams:
        for i, ch in enumerate(ks):
            uni[cl[ch]] += 1
            if i:
                bi[(cl[ks[i - 1]], cl[ch])] += 1
    cs = sorted(size)
    tot = sum(uni.values())
    pu = {c: (uni[cl[c]] + a.alpha) / (tot + a.alpha * len(cs)) / size[cl[c]] for c in ALPH}
    row = collections.Counter()
    for (p, c), n in bi.items():
        row[p] += n
    pb = {}
    for p in ALPH:
        den = row[cl[p]] + a.alpha * len(cs)
        for c in ALPH:
            pb[(p, c)] = (bi.get((cl[p], cl[c]), 0) + a.alpha) / den / size[cl[c]]
    return pb, pu


BG['CV'] = class_table(L_tr, cls_cv)
pbL, puL = BG['L']
BG2 = dict(BG)


def tri_table(streams, p2tab):
    """P3(c | a b) = (n(abc) + k * P2(c|b)) / (n(ab.) + k)。P2 は p2tab[0]"""
    n3 = collections.Counter()
    n2 = collections.Counter()
    for ks in streams:
        for i in range(2, len(ks)):
            n3[(ks[i - 2], ks[i - 1], ks[i])] += 1
            n2[(ks[i - 2], ks[i - 1])] += 1
    return n3, n2


def class_tri(streams, cf, p2tab):
    cl = {c: cf(c) for c in ALPH}
    size = collections.Counter(cl.values())
    n3 = collections.Counter()
    n2 = collections.Counter()
    for ks in streams:
        for i in range(2, len(ks)):
            n3[(cl[ks[i - 2]], cl[ks[i - 1]], cl[ks[i]])] += 1
            n2[(cl[ks[i - 2]], cl[ks[i - 1]])] += 1
    return n3, n2, cl, size


def sub_streams(frac):
    if frac >= 1.0:
        return L_tr
    rnd = random.Random(11)
    idx = sorted(rnd.sample(range(len(L_tr)), int(len(L_tr) * frac)))
    return [L_tr[i] for i in idx]


BG3 = {}  # name -> (n3, n2, pbase)  pbase = (pb, pu) の 2 連表
en_only_tr = en_tr
for nm, st in (('L3', L_tr), ('L3_10', sub_streams(0.1)), ('L3_30', sub_streams(0.3)), ('L3_en', en_only_tr)):
    pb2, pu2 = table_from(*counts_from_streams(st))
    BG[nm + '_2'] = (pb2, pu2)
    n3, n2 = tri_table(st, None)
    BG3[nm] = (n3, n2, pb2, pu2)
    print(f'{nm}: 学習キー {sum(map(len, st))}, 3 連の種類 {len(n3)}(格子 {NA}^3 = {NA**3} 中 {len(n3)/NA**3*100:.1f}%)、3 連の延べ {sum(n3.values())}')
cv_n3, cv_n2, cv_cl, cv_size = class_tri(L_tr, cls_cv, None)
pbC, puC = BG['CV']
cs = sorted(cv_size)
print(f'CV3: 3 クラス遷移 {len(cv_n3)} 種類')


def p_l3(nm, prev2c, prevc, c):
    n3, n2, pb2, pu2 = BG3[nm]
    if prevc is None:
        return pu2[c]
    q2 = pb2[(prevc, c)]
    if prev2c is None:
        return q2
    return (n3.get((prev2c, prevc, c), 0) + a.k3 * q2) / (n2.get((prev2c, prevc), 0) + a.k3)


def p_cv3(prev2c, prevc, c):
    if prevc is None:
        return puC[c]
    q2 = pbC[(prevc, c)]
    if prev2c is None:
        return q2
    cc, p1, p2_ = cv_cl[c], cv_cl[prevc], cv_cl[prev2c]
    q = (cv_n3.get((p2_, p1, cc), 0) + a.k3 * q2 * cv_size[cc]) / (cv_n2.get((p2_, p1), 0) + a.k3) / cv_size[cc]
    return q


def bg_logp(bg, prev2c, prevc, c):
    if bg == 'U':
        return -math.log(defined)
    if bg == 'L2':
        pb, pu = BG['L']
        return math.log(pb[(prevc, c)] if prevc is not None else pu[c])
    if bg in BG3:
        return math.log(p_l3(bg, prev2c, prevc, c))
    if bg == 'CV3':
        return math.log(p_cv3(prev2c, prevc, c))
    if bg == 'L3CV3':
        return math.log(a.mix * p_l3('L3', prev2c, prevc, c) + (1 - a.mix) * p_cv3(prev2c, prevc, c))
    raise KeyError(bg)


def ja_logp(jm, prev2, prev, tok):
    q2 = p2(prev, tok)
    if jm == 'bi' or prev2 is None or prev is None:
        return math.log(q2)
    q3 = p3(prev2, prev, tok)
    if q3 is None:
        return math.log(q2)
    if jm == 'tri-backoff':
        return math.log(q3)
    return math.log(a.lam * q3 + (1 - a.lam) * q2)


def traj(seq, jm, bg):
    prev2 = prev = prevc = prev2c = None
    llr, out = 0.0, []
    for k in seq[:a.max_keys]:
        t = kana_of.get(k, 'NONE')
        c = rev.get(k)
        if t in ('SYM', 'NONE') or t not in vocab:
            d, prev2, prev, prevc, prev2c = 0.0, None, None, None, None
        else:
            d = ja_logp(jm, prev2, prev, t) - bg_logp(bg, prev2c, prevc, c)
            prev2, prev, prev2c, prevc = prev, t, prevc, c
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


# ---------- 評価標本 ----------
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


def en_runs(texts):
    o = []
    for t in texts:
        w = t.split()
        for st in [0] + [random.randrange(0, max(1, len(w))) for _ in range(2)]:
            ks = [pos[ch] for ch in ' '.join(w[st:]).lower() if ch in pos]
            if ks:
                o.append(ks)
    return o


RUNS = {s: en_runs(ts) for s, ts in sorted(TEST.items())}
G = {g: [r for s in ss for r in RUNS[s]] for g, ss in AGG.items()}
print(f'ja sentences {len(JA)}, ja episodes {len(runs)}, with prefix {len(nz)}; test runs ' + ', '.join(f'{g} {len(r)}' for g, r in G.items()))
print('言語別の評価区間数: ' + ', '.join(f'{s} {len(r)}' for s, r in RUNS.items()))

JMS = ('bi', 'tri-backoff', 'tri-interp')
COMBOS = [(j, b) for j in JMS for b in ('U', 'L2', 'L3')] + [(j, b) for j in ('bi', 'tri-interp') for b in ('CV3', 'L3CV3', 'L3_en', 'L3_30', 'L3_10')]
TH = [2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 18, 20, 22, 24, 26, 28, 30, 34, 38, 42, 46, 50, 60, 70, 80]
TJ = {c: [traj(s, *c) for s in nz] for c in COMBOS}
TG = {c: {g: [traj(s, *c) for s in rs] for g, rs in G.items()} for c in COMBOS}
TS = {c: {s: [traj(x, *c) for x in rs] for s, rs in RUNS.items()} for c in COMBOS}


def name(c):
    return f'{c[0]}+{c[1]}'


# 1. 事例
WORDS = 'windows window hello google search the function chrome build github python terminal code test setup report excel teams zoom slack'.split()


def wkeys(w):
    return [pos[ch] for ch in w]


def err_at(c, th):
    return sum(verdict(x, th)[0] == 'en' for x in TJ[c]) / len(nz) * 100


def th_for(c, target):
    """JA 誤判定率が target 以下になる最小の閾値(格子上)"""
    for th in TH:
        if err_at(c, th) <= target:
            return th
    return TH[-1]


TH1 = {c: th_for(c, 1.0) for c in COMBOS}
print('\n[1] 事例: 7 打鍵目までの llr(打鍵ごとの累積) と、固定閾値 14・16 / 各モデルで日本語の誤判定 1% 以下になる閾値(th1)での確定(打鍵数)')
for c in COMBOS:
    print(f'-- {name(c)}  th1(誤判定<=1%) = {TH1[c]}  (日本語誤判定 {err_at(c, TH1[c]):.2f}%)')
    for w in WORDS:
        cum = traj(wkeys(w), *c)
        cell = []
        for th in (14, 16, TH1[c]):
            v = verdict(cum, th)
            cell.append(f'{v[0]}@{v[1]}')
        print(f"   {w:9} {cum[-1]:7.1f} [{' '.join(f'{x:.1f}' for x in cum)}]  th14:{cell[0]} th16:{cell[1]} th1:{cell[2]}")
print('事例が th1 で en に確定した数(20 語中):', {name(c): sum(verdict(traj(wkeys(w), *c), TH1[c])[0] == 'en' for w in WORDS) for c in COMBOS})

# 2. 曲線
curve = {}
for c in COMBOS:
    curve[c] = []
    for th in TH:
        err = err_at(c, th)
        rates = {g: sum(verdict(x, th)[0] == 'en' for x in TG[c][g]) / len(TG[c][g]) * 100 for g in G}
        curve[c].append((th, err, rates))


def at_err(c, target, g=None, sub=None):
    pts = sorted(curve[c], key=lambda x: x[1])
    for (t0, e0, r0), (t1, e1, r1) in zip(pts, pts[1:]):
        if e0 <= target <= e1 and e1 > e0:
            w = (target - e0) / (e1 - e0)
            th = t0 + w * (t1 - t0)
            return r0[g] + w * (r1[g] - r0[g]), th
    return None, None


print('\n[2] 同じ日本語誤判定率のときの決まる率(曲線の線形補間)と、そのときの閾値: 誤判定率 | 群 | ' + ' | '.join(name(c) for c in COMBOS))
for tgt in (0.5, 1.0, 2.0):
    for g in G:
        cells = []
        for c in COMBOS:
            v, th = at_err(c, tgt, g)
            cells.append('-' if v is None else f'{v:5.1f}%(th{th:.0f})')
        print(f'{tgt:3.1f}% | {g} | ' + ' | '.join(cells))

print('\n[2b] 閾値固定の表(14 と 16 と th1): model | th | 日本語→英数誤 | 英文(平均打鍵) | コード | 他言語 | Undecided(英文)')
for c in COMBOS:
    for th in sorted({14, 16, TH1[c]}):
        err = err_at(c, th)
        cols = []
        for g in G:
            rr = [verdict(x, th) for x in TG[c][g]]
            e = [r[1] for r in rr if r[0] == 'en']
            cols.append(f'{len(e)/len(rr)*100:5.1f}%({statistics.mean(e) if e else 0:.2f})')
        und = sum(verdict(x, th)[0] == 'und' for x in TG[c]['英文']) / len(TG[c]['英文']) * 100
        print(f'{name(c):22} | {th:3} | {err:5.2f}% | ' + ' | '.join(cols) + f' | U{und:4.1f}%')

# 3. Japanese 側
print('\n[3] 非日本語を Japanese と誤確定する率(英数側は決めない)。ja_threshold=4 と、日本語区間の尤度確定率(同じ ja_threshold)')
for c in COMBOS:
    cells = []
    for g in G:
        cells.append(f"{sum(verdict(x, 1e9)[0] == 'ja' for x in TG[c][g]) / len(TG[c][g]) * 100:5.2f}%")
    jaconf = sum(verdict(x, 1e9)[0] == 'ja' for x in TJ[c]) / len(nz) * 100
    print(f'{name(c):22} | ' + ' | '.join(cells) + f' | 日本語の尤度確定 {jaconf:5.2f}%')
print('同じ誤確定率に揃える: 非日本語全体の誤確定が U(bi) の ja_threshold=4 のときの率以下になる最小の ja_threshold と、そのときの日本語の尤度確定率')
allg = lambda c: [x for g in G for x in TG[c][g]]
base_rate = sum(verdict(x, 1e9, 4.0)[0] == 'ja' for x in allg(('bi', 'U'))) / len(allg(('bi', 'U')))
for c in COMBOS:
    xs = allg(c)
    for jt in [1, 2, 3, 4, 5, 6, 7, 8, 10, 12, 14, 16, 20, 25, 30]:
        if sum(verdict(x, 1e9, jt)[0] == 'ja' for x in xs) / len(xs) <= base_rate:
            jc = sum(verdict(x, 1e9, jt)[0] == 'ja' for x in TJ[c]) / len(nz) * 100
            print(f'{name(c):22} | ja_th {jt:2} | 日本語の尤度確定 {jc:5.2f}% (基準 {base_rate*100:.2f}%)')
            break

# 4. 先頭
print('\n[4] 日本語区間が先頭 k 打以内に NotJapanese(各モデルの th1 で)と、英文が k 打以内に決まる率: model | th1 | k | JA誤 | 英文 | コード | 他言語')
for c in COMBOS:
    for k in (1, 2, 3):
        th = TH1[c]
        je = sum(1 for x in TJ[c] if verdict(x, th)[0] == 'en' and verdict(x, th)[1] <= k) / len(nz) * 100
        cols = []
        for g in G:
            cols.append(f"{sum(1 for x in TG[c][g] if verdict(x, th)[0] == 'en' and verdict(x, th)[1] <= k) / len(TG[c][g]) * 100:5.1f}%")
        print(f'{name(c):22} | {th:3} | {k} | {je:5.2f}% | ' + ' | '.join(cols))

# 5. 言語別(各モデルの th1 での決まる率)
print('\n[5] 言語別の決まる率(各モデルの th1 = 日本語誤判定 1% 以下の最小閾値)')
subs = list(RUNS)
print('model | th1 | ' + ' | '.join(f'{s}(n={len(RUNS[s])})' for s in subs))
for c in COMBOS:
    th = TH1[c]
    print(f'{name(c):22} | {th:3} | ' + ' | '.join(f"{sum(verdict(x, th)[0] == 'en' for x in TS[c][s]) / len(TS[c][s]) * 100:5.1f}%" for s in subs))
