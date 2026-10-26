"""ADR-280: フィクスチャ(tests/fixtures/lang_detect_eval.tsv)と事例・CI の境界列を、新しい構成(tri-interp + data/latin_bigram.txt)と
従来の構成(bi + 一様分布)で判定した率・llr を出す。Rust のテスト(tests/lang_detect_eval.rs)のアサートの値の根拠。

実行: uv run --no-project python -I tools/lang-detect-eval/eval_fixture_extended.py --repo .
(評価データは使わない。仮名の 3 連・2 連の表は data/ngram_hiragana.csv.gz、背景は data/latin_bigram.txt。)
"""
import argparse, collections, glob, gzip, json, math, random, re, statistics, unicodedata

ap = argparse.ArgumentParser()
ap.add_argument('--repo', default='.')
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





GRID = ['1234567890-^\\', 'qwertyuiop@[', 'asdfghjkl;:]', 'zxcvbnm,./']
GPOS = {ch: (r, c) for r, row in enumerate(GRID) for c, ch in enumerate(row)}
rows_ = ['1234567890-', "qwertyuiop@[", 'asdfghjkl;:]', 'zxcvbnm,./']
LROWS = {ch: (r, c) for r, row in enumerate(rows_) for c, ch in enumerate(row)}
ALPH = sorted(LROWS)
rev = {v: k for k, v in LROWS.items()}


def load_shipped(path):
    ls = [l for l in open(path, encoding='utf-8').read().splitlines() if not l.startswith('#')]
    assert ls[0].removeprefix('chars=') == ''.join(ALPH)
    hx = lambda t: [math.exp(-int(t[i:i + 2], 16) / 16) for i in range(0, len(t), 2)]
    pu = dict(zip(ALPH, hx(ls[1].removeprefix('uni='))))
    pb = {}
    for p_, row in zip(ALPH, ls[2:]):
        for c_, v in zip(ALPH, hx(row)):
            pb[(p_, c_)] = v
    return pb, pu


SH = load_shipped(f'{a.repo}/data/latin_bigram.txt')


def ja_logp(jm, prev2, prev, tok):
    q2 = p2(prev, tok)
    if jm == 'bi' or prev2 is None or prev is None:
        return math.log(q2)
    q3 = p3(prev2, prev, tok)
    if q3 is None:
        return math.log(q2)
    return math.log(a.lam * q3 + (1 - a.lam) * q2)


def traj_ext(seq):
    """新しい構成(tri-interp + 同梱のラテン 2 連)。seq は (row, col) の列"""
    prev2 = prev = prevc = None
    llr, out = 0.0, []
    for k in seq[:a.max_keys]:
        t = kana_of.get(k, 'NONE')
        c = rev.get(k)
        if t in ('SYM', 'NONE') or t not in vocab or c is None:
            d, prev2, prev, prevc = 0.0, None, None, None
        else:
            pb, pu = SH
            bgp = pb[(prevc, c)] if prevc is not None else pu[c]
            d = ja_logp('tri-interp', prev2, prev, t) - math.log(bgp)
            prev2, prev, prevc = prev, t, c
        llr += d
        out.append(llr)
    return out


def traj_leg(seq):
    """従来の構成(bi + 一様。語彙の外の仮名は文脈に残す)"""
    prev = None
    llr, out = 0.0, []
    for k in seq[:a.max_keys]:
        t = kana_of.get(k, 'NONE')
        if t in ('SYM', 'NONE') or t not in vocab:
            d = 0.0
        else:
            d = math.log(p2(prev, t)) - bg
        prev = t if t not in ('SYM', 'NONE') else None
        llr += d
        out.append(llr)
    return out


def verdict(cum, en_th, ja_th):
    for i, v in enumerate(cum, 1):
        if v >= ja_th:
            return 'ja', i
        if v <= -en_th:
            return 'en', i
    return 'und', len(cum)


fx = collections.defaultdict(list)
for ln in open(f'{a.repo}/tests/fixtures/lang_detect_eval.tsv', encoding='utf-8').read().split('\n'):
    if '\t' in ln:
        lab, keys = ln.split('\t', 1)
        fx[lab].append(keys)


def seqs(label):
    return [[GPOS[ch] for ch in k] for k in fx[label]]


CONF = {'従来(bi+一様, ja_th=4)': (traj_leg, 4.0), '新(tri-interp+latin, ja_th=14)': (traj_ext, 14.0)}
print('フィクスチャ: 件数 ' + ', '.join(f'{k} {len(v)}' for k, v in fx.items()))
print('\n[1] ja の NotJapanese 誤判定率(en_threshold ごと)')
for nm, (tr, jt) in CONF.items():
    T_ = [tr(s) for s in seqs('ja')]
    print(nm + ': ' + ', '.join(f"en{th}: {sum(verdict(x, th, jt)[0] == 'en' for x in T_) / len(T_) * 100:.2f}%" for th in (8, 13, 14, 16, 20)))
print('\n[2] 英数側が max_keys 以内に NotJapanese に達する率(平均打鍵数)')
for nm, (tr, jt) in CONF.items():
    for lab in ('en', 'code', 'rand'):
        T_ = [tr(s) for s in seqs(lab)]
        out = []
        for th in (8, 13, 14, 16):
            r = [verdict(x, th, jt) for x in T_]
            e = [n for v, n in r if v == 'en']
            out.append(f'en{th}: {len(e) / len(r) * 100:.1f}%({statistics.mean(e) if e else 0:.2f})')
        print(f'{nm} {lab}: ' + ', '.join(out))
print('\n[3] 英文・コード・一様を Japanese(尤度)と誤判定する率')
for nm, (tr, jt) in CONF.items():
    print(nm + ': ' + ', '.join(f"{lab}: {sum(verdict(x, 1e9, jt)[0] == 'ja' for x in [tr(s) for s in seqs(lab)]) / len(fx[lab]) * 100:.2f}%" for lab in ('en', 'code', 'rand')))
print('\n[4] 事例の llr(打鍵ごとの累積)と en_threshold 13・8 での確定(新の構成)')
for w in 'windows window hello google function chrome build setup search github the excel zoom slack ayre'.split():
    cum = traj_ext([GPOS[ch] for ch in w])
    print(f"{w:9} [{' '.join(f'{x:.2f}' for x in cum)}] th13:{verdict(cum, 13, 14.0)} th8:{verdict(cum, 8, 14.0)}")
print('\n[5] CI の境界列(従来の構成の推移。#826 の確認用)')
for w in ('ayre', 'build'):
    print(w, [f'{x:.4f}' for x in traj_leg([GPOS[ch] for ch in w])])

# [6] 独立な小さい標本: このリポジトリの文書・コードに現れる英単語・識別子(フィクスチャの生成分布とは無関係に手で選んだ)。
#     Rust の tests/lang_detect_eval.rs の INDEPENDENT_WORDS と同じ並び。
INDEPENDENT_WORDS = (
    'keyboard remapper simultaneous keystrokes windows hook belief focus state executor runtime pipeline observer output '
    'injector coordinator tracking classifier journal replay golden scenario architecture guard layer boundary constants '
    'measurement evidence threshold episode detector tracker controller background latin bigram trigram kana layout '
    'engine config settings session worktree develop release branch commit workflow cargo clippy format target'
).split()
print('\n[6] 独立な単語 %d 個: en_threshold 13 / 8 で NotJapanese に確定した数(新の構成 / 従来の構成)' % len(INDEPENDENT_WORDS))
for th in (13, 8):
    ne = sum(verdict(traj_ext([GPOS[ch] for ch in w]), th, 14.0)[0] == 'en' for w in INDEPENDENT_WORDS)
    nl = sum(verdict(traj_leg([GPOS[ch] for ch in w]), th if th == 8 else 14, 4.0)[0] == 'en' for w in INDEPENDENT_WORDS)
    ja = sum(verdict(traj_ext([GPOS[ch] for ch in w]), th, 14.0)[0] == 'ja' for w in INDEPENDENT_WORDS)
    print(f'en_threshold {th}: 新 {ne} / 従来(th {th if th == 8 else 14}) {nl}, 新で Japanese 誤確定 {ja}')
print('決まらない語(新, th13):', [w for w in INDEPENDENT_WORDS if verdict(traj_ext([GPOS[ch] for ch in w]), 13, 14.0)[0] != 'en'])
