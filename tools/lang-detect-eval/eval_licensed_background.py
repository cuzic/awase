"""ADR-280: 判定の背景(ラテン文字の 2 連)の表を、ライセンスと再生成手順が明らかな元だけで作り、#847 の L(評価データの学習側 = 出典不明)と比べる。

元(すべてこのマシンで再生成できる):
  E_repo : このリポジトリの `*.md`・`*.rs`(`tests/fixtures`・`data/` を除く)の英字列(camelCase を分割)。リポジトリ自身の著作物。
  E_dict : `/usr/share/dict/american-english`(Debian の wamerican。SCOWL のライセンス、自由に使える)の英字だけの語。
  LOC    : `/usr/share/locale/<lang>/LC_MESSAGES/*.mo`(Debian のパッケージ〔GNU・GNOME 等〕の翻訳カタログ。各パッケージの GPL/LGPL)の
           msgstr の ASCII 英字(発音記号は落とす)。18 言語を同じ重みで混ぜる。
  CODE_rs: リポジトリの `*.rs`(識別子とコメント)だけ。
  ALL = 0.4 * E_mix(E_repo と E_dict の等分) + 0.4 * LOC + 0.2 * CODE_rs。SHIPPED = data/latin_bigram.txt(ALL を gen_latin_bigram.py で量子化したもの)。
  CV : 母音/子音/その他の 3 クラス(上の全元から作る)。ALLCV = ALL と CV の 0.5 : 0.5。LCV = L2 と CV(L から)の 0.5 : 0.5(基準)。
  L2 : #847 の L(評価データの学習側。出典不明のため基準としてだけ使う。実機には載せない)。
仮名側は 2 連(bi)と tri-interp(重み 0.7)。評価の枠組みは eval_latin_trigram.py(#847)と同じ(評価は奇数番の文だけ、seed 5)。
実行: uv run --no-project python -I tools/lang-detect-eval/eval_licensed_background.py --repo . --data-dir <dir>
"""
import argparse, collections, glob, gzip, json, math, random, re, statistics, unicodedata

ap = argparse.ArgumentParser()
ap.add_argument('--repo', default='.')
ap.add_argument('--data-dir', required=True)
ap.add_argument('--lam', type=float, default=0.7, help='補間の 3 連の重み')
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




import os, subprocess, gettext

ALPH = sorted(pos)           # 打鍵位置の格子の文字(数字・英字・記号)。仮名のキーはここに含まれる
rev = {v: k for k, v in pos.items()}
NA = len(ALPH)


def keys_of(text):
    return [ch for ch in text.lower() if ch in pos]  # 評価の en_runs と同じ(空白・範囲外は飛ばす)


def ascii_letters(text):
    """発音記号を落とした ASCII 小文字(翻訳の文字列用。語の間の空白・記号は飛ばす)"""
    t = unicodedata.normalize('NFKD', text)
    return [ch for ch in t.lower() if 'a' <= ch <= 'z']


# ---------- 評価データ(非日本語は偶数番を学習・奇数番を評価。学習側は L(出典不明)の再現にだけ使う) ----------
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
SUB = collections.defaultdict(list)
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


def counts_from_streams(streams):
    bi = collections.Counter()
    uni = collections.Counter()
    for ks in streams:
        for i, ch in enumerate(ks):
            uni[ch] += 1
            if i:
                bi[(ks[i - 1], ch)] += 1
    return bi, uni


def table_from(bi, uni):
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


def mix_tables(parts):
    """(重み, (pb, pu)) の確率の混合"""
    z = sum(w for w, _ in parts)
    pb = {k: sum(w * t[0][k] for w, t in parts) / z for k in parts[0][1][0]}
    pu = {k: sum(w * t[1][k] for w, t in parts) / z for k in parts[0][1][1]}
    return pb, pu


# ---------- ライセンスの明らかな元 ----------
def repo_streams(exts):
    files = subprocess.run(['git', '-C', a.repo, 'ls-files'] + [f'*.{e}' for e in exts], capture_output=True, text=True).stdout.split()
    out, n = [], 0
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


LOC_LANGS = ['es', 'fr', 'de', 'pt', 'pt_BR', 'it', 'nl', 'sv', 'da', 'pl', 'cs', 'ro', 'hu', 'fi', 'tr', 'id', 'ms', 'vi']


def locale_streams(lang):
    """/usr/share/locale/<lang>/LC_MESSAGES/*.mo(Debian のパッケージの翻訳。各パッケージの GPL/LGPL)の msgstr"""
    out, nmo = [], 0
    for f in sorted(glob.glob(f'/usr/share/locale/{lang}/LC_MESSAGES/*.mo')):
        try:
            t = gettext.GNUTranslations(open(f, 'rb'))
        except Exception:
            continue
        nmo += 1
        for k, v in t._catalog.items():
            if k == '' or isinstance(k, tuple):
                continue
            ks = ascii_letters(v)
            if ks:
                out.append(ks)
    return out, nmo


rs_md, nmd = repo_streams(['md'])
rs_rs, nrs = repo_streams(['rs'])
rs_all = rs_md + rs_rs
ds, ndict = dict_streams()
loc = {l: locale_streams(l) for l in LOC_LANGS}
en_en, _ = locale_streams('en_GB')  # 英語の msgstr(en_GB は少数)
print(f'repo md files {nmd} streams {len(rs_md)} keys {sum(map(len, rs_md))}; rs files {nrs} streams {len(rs_rs)} keys {sum(map(len, rs_rs))}; dict words {ndict} keys {sum(map(len, ds))}')
print('locale: ' + ', '.join(f'{l} mo={loc[l][1]} keys={sum(map(len, loc[l][0]))}' for l in LOC_LANGS))
L_tr = [keys_of(t) for s, ts in TRAIN.items() for t in ts]  # L(出典不明を含む。比較用の基準)

T = {}
T['L2'] = table_from(*counts_from_streams(L_tr))
T['E_repo'] = table_from(*counts_from_streams(rs_all))
T['E_dict'] = table_from(*counts_from_streams(ds))
T['E_mix'] = mix_tables([(1, T['E_repo']), (1, T['E_dict'])])
T['CODE_rs'] = table_from(*counts_from_streams(rs_rs))
loc_tabs = {l: table_from(*counts_from_streams(loc[l][0])) for l in LOC_LANGS}
T['LOC'] = mix_tables([(1, t) for t in loc_tabs.values()])
T['ALL'] = mix_tables([(0.4, T['E_mix']), (0.4, T['LOC']), (0.2, T['CODE_rs'])])


def cls_cv(ch):
    return 'V' if ch in 'aeiou' else ('C' if ch.isalpha() else 'X')


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


lic_streams = rs_all + ds + [s for l in LOC_LANGS for s in loc[l][0]]
T['CV'] = class_table(lic_streams, cls_cv)
T['ALLCV'] = mix_tables([(0.5, T['ALL']), (0.5, T['CV'])])
T['LCV'] = mix_tables([(0.5, T['L2']), (0.5, class_table(L_tr, cls_cv))])


def load_shipped(path):
    """data/latin_bigram.txt(gen_latin_bigram.py の出力。-ln p の 1/16 刻みの 16 進)を確率に戻す"""
    ls = [l for l in open(path, encoding='utf-8').read().splitlines() if not l.startswith('#')]
    chars = ls[0].removeprefix('chars=')
    assert chars == ''.join(ALPH), 'chars が格子と違う'
    hx = lambda t: [math.exp(-int(t[i:i + 2], 16) / 16) for i in range(0, len(t), 2)]
    pu = dict(zip(ALPH, hx(ls[1].removeprefix('uni='))))
    pb = {}
    for p_, row in zip(ALPH, ls[2:]):
        for c_, v in zip(ALPH, hx(row)):
            pb[(p_, c_)] = v
    return pb, pu


T['SHIPPED'] = load_shipped(f'{a.repo}/data/latin_bigram.txt')


def bg_logp(bg, prevc, c):
    if bg == 'U':
        return -math.log(defined)
    pb, pu = T[bg]
    return math.log(pb[(prevc, c)] if prevc is not None else pu[c])


def ja_logp(jm, prev2, prev, tok):
    q2 = p2(prev, tok)
    if jm == 'bi' or prev2 is None or prev is None:
        return math.log(q2)
    q3 = p3(prev2, prev, tok)
    if q3 is None:
        return math.log(q2)
    return math.log(a.lam * q3 + (1 - a.lam) * q2)


def traj(seq, jm, bg):
    prev2 = prev = prevc = None
    llr, out = 0.0, []
    for k in seq[:a.max_keys]:
        t = kana_of.get(k, 'NONE')
        c = rev.get(k)
        if t in ('SYM', 'NONE') or t not in vocab:
            d, prev2, prev, prevc = 0.0, None, None, None
        else:
            d = ja_logp(jm, prev2, prev, t) - bg_logp(bg, prevc, c)
            prev2, prev, prevc = prev, t, c
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

BGS = ['U', 'L2', 'E_repo', 'E_dict', 'E_mix', 'LOC', 'ALL', 'SHIPPED', 'CV', 'ALLCV', 'LCV']
JMS = ('bi', 'tri-interp')
COMBOS = [(j, b) for j in JMS for b in BGS]
TH = [2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 20, 22, 24, 26, 28, 30, 34, 38, 42, 46, 50, 60, 70, 80]
TJ = {c: [traj(s, *c) for s in nz] for c in COMBOS}
TG = {c: {g: [traj(s, *c) for s in rs] for g, rs in G.items()} for c in COMBOS}
TS = {c: {s: [traj(x, *c) for x in rs] for s, rs in RUNS.items()} for c in COMBOS}


def name(c):
    return f'{c[0]}+{c[1]}'


WORDS = 'windows window hello google search the function chrome build github python terminal code test setup report excel teams zoom slack'.split()


def wkeys(w):
    return [pos[ch] for ch in w]


def err_at(c, th):
    return sum(verdict(x, th)[0] == 'en' for x in TJ[c]) / len(nz) * 100


def th_for(c, target):
    for th in TH:
        if err_at(c, th) <= target:
            return th
    return TH[-1]


TH1 = {c: th_for(c, 1.0) for c in COMBOS}
curve = {}
for c in COMBOS:
    curve[c] = []
    for th in TH:
        err = err_at(c, th)
        rates = {g: sum(verdict(x, th)[0] == 'en' for x in TG[c][g]) / len(TG[c][g]) * 100 for g in G}
        curve[c].append((th, err, rates))


def at_err(c, target, g):
    pts = sorted(curve[c], key=lambda x: x[1])
    for (t0, e0, r0), (t1, e1, r1) in zip(pts, pts[1:]):
        if e0 <= target <= e1 and e1 > e0:
            w = (target - e0) / (e1 - e0)
            return r0[g] + w * (r1[g] - r0[g]), t0 + w * (t1 - t0)
    return None, None


print('\n[A] 日本語誤判定 1% のときの決まる率(線形補間)と閾値 / th1 / 事例(20 語)の確定数')
print('model | th1 | JA誤(th1) | 英文 | コード | 他言語 | 事例確定 | 先頭3打の英文 | 日本語先頭3打誤 | JA誤確定(ja4: 英文/コード/他言語)')
for c in COMBOS:
    th = TH1[c]
    cells = []
    for g in G:
        v, t = at_err(c, 1.0, g)
        cells.append('-' if v is None else f'{v:5.1f}%')
    nw = sum(verdict(traj(wkeys(w), *c), th)[0] == 'en' for w in WORDS)
    e3 = sum(1 for x in TG[c]['英文'] if verdict(x, th)[0] == 'en' and verdict(x, th)[1] <= 3) / len(TG[c]['英文']) * 100
    j3 = sum(1 for x in TJ[c] if verdict(x, th)[0] == 'en' and verdict(x, th)[1] <= 3) / len(nz) * 100
    jc = ' / '.join(f"{sum(verdict(x, 1e9)[0] == 'ja' for x in TG[c][g]) / len(TG[c][g]) * 100:4.2f}%" for g in G)
    print(f'{name(c):20} | {th:3} | {err_at(c, th):4.2f}% | ' + ' | '.join(cells) + f' | {nw:2}/20 | {e3:4.1f}% | {j3:4.2f}% | {jc}')

print('\n[B] 事例の llr(7 打鍵目)と th1 での確定(tri-interp)')
for b in BGS:
    c = ('tri-interp', b)
    th = TH1[c]
    print(f'-- {name(c)} th1={th}')
    print('   ' + ' '.join(f"{w}:{traj(wkeys(w), *c)[-1]:.1f}{'*' if verdict(traj(wkeys(w), *c), th)[0] == 'en' else ''}" for w in WORDS))

print('\n[C] 言語別(各モデルの th1)の決まる率')
subs = list(RUNS)
print('model | th1 | ' + ' | '.join(subs))
for c in COMBOS:
    th = TH1[c]
    print(f'{name(c):20} | {th:3} | ' + ' | '.join(f"{sum(verdict(x, th)[0] == 'en' for x in TS[c][s]) / len(TS[c][s]) * 100:5.1f}%" for s in subs))


# ---------- [D] 閾値の校正(採用する構成について) ----------
print('\n[D] 閾値の校正。構成 = tri-interp + ALL(採用候補)と、基準の bi + U(現行)')
CAND = [('bi', 'U'), ('tri-interp', 'ALL'), ('tri-interp', 'SHIPPED')]
NONJA = lambda c: [x for g in G for x in TG[c][g]]


def err_at_j(c, th, jt):
    return sum(verdict(x, th, jt)[0] == 'en' for x in TJ[c]) / len(nz) * 100


def rate_ja_conf(xs, jt):
    return sum(verdict(x, 1e9, jt)[0] == 'ja' for x in xs) / len(xs) * 100


base_c = ('bi', 'U')
base_rate_all = rate_ja_conf(NONJA(base_c), 4.0)
print(f'基準: 現行(bi+U, ja_threshold=4)の非日本語全体の Japanese 誤確定 {base_rate_all:.2f}%  日本語の尤度確定 {rate_ja_conf(TJ[base_c], 4.0):.2f}%')
for c in CAND:
    print(f'-- {name(c)}')
    print('   ja_th | 非日本語の Japanese 誤確定(英文/コード/他言語/全体) | 日本語の尤度確定')
    for jt in [4, 6, 8, 10, 12, 14, 16, 20]:
        gs = [rate_ja_conf(TG[c][g], jt) for g in G]
        print(f'   {jt:5} | {gs[0]:.2f}% / {gs[1]:.2f}% / {gs[2]:.2f}% / {rate_ja_conf(NONJA(c), jt):.2f}% | {rate_ja_conf(TJ[c], jt):.2f}%')
    print('   ja_th ごとの、日本語→英数の誤判定が目標以下になる最小の en 閾値(目標: 1% = ime_on_eisu の入り、7% = ime_on の en_threshold)と、そのときの英文/コード/他言語の決まる率・事例確定数')
    for jt in [4, 10, 14, 20]:
        for target in (1.0, 7.0):
            th = next((t for t in [x / 2 for x in range(4, 120)] if err_at_j(c, t, jt) <= target), None)
            if th is None:
                continue
            rr = [sum(verdict(x, th, jt)[0] == 'en' for x in TG[c][g]) / len(TG[c][g]) * 100 for g in G]
            nw = sum(verdict(traj(wkeys(w), *c), th, jt)[0] == 'en' for w in WORDS)
            e3 = sum(1 for x in TG[c]['英文'] if verdict(x, th, jt)[0] == 'en' and verdict(x, th, jt)[1] <= 3) / len(TG[c]['英文']) * 100
            j3 = sum(1 for x in TJ[c] if verdict(x, th, jt)[0] == 'en' and verdict(x, th, jt)[1] <= 3) / len(nz) * 100
            print(f'   ja_th {jt:2} target {target:3.1f}% -> en_th {th:5.1f} (JA誤 {err_at_j(c, th, jt):.2f}%) | 英文 {rr[0]:.1f}% コード {rr[1]:.1f}% 他 {rr[2]:.1f}% | 事例 {nw}/20 | 先頭3打 英文 {e3:.1f}% JA誤 {j3:.2f}%')
print('\n旧尺度(bi+U, ja_threshold=4)の en 閾値と、同じ日本語誤判定率になる新尺度(tri-interp+ALL)の en 閾値(ja_th を JT_SEL にしたとき)')
JT_SEL = 4
for jt in (4, 10, 14):
    new_c = ('tri-interp', 'SHIPPED')
    out = []
    for th_old in (8, 10, 12, 14, 16, 18, 20):
        e = err_at_j(base_c, th_old, 4.0)
        t = next((t for t in [x / 2 for x in range(4, 160)] if err_at_j(new_c, t, jt) <= e), None)
        out.append(f'旧{th_old}({e:.2f}%)->新{t}')
    print(f'ja_th={jt}: ' + ' ; '.join(out))
