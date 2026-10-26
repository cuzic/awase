"""ADR-280: 入力言語の判定の「背景」(ラテン文字の 2 連)の表 `data/latin_bigram.txt` を、ライセンスと再生成手順が明らかな元から作る。

表 = 打鍵位置の格子の 45 文字(数字・英字・記号)上の、加算平滑(alpha = 0.5)の文字 2 連 P(c | 直前の文字) と 1 連 P(c)。
元と重み(確率の混合。`eval_licensed_background.py` の ALL と同じ):
  0.4 * E_mix (E_repo と E_dict の等分) + 0.4 * LOC + 0.2 * CODE_rs
  E_repo : このリポジトリの `*.md`・`*.rs`(`tests/fixtures`・`data/` 等を除く)の英字列(camelCase を分割)。リポジトリ自身の著作物。
  E_dict : `/usr/share/dict/american-english`(Debian の wamerican。SCOWL のライセンス)の英字だけの語。
  LOC    : `/usr/share/locale/<lang>/LC_MESSAGES/*.mo`(Debian のパッケージ〔GNU・GNOME 等〕の翻訳カタログ。各パッケージの GPL/LGPL)の
           msgstr の ASCII 英字(発音記号は落とす)。18 言語を同じ重みで混ぜる。翻訳文そのものは表に残らない(文字の遷移確率だけ)。
  CODE_rs: リポジトリの `*.rs`(識別子とコメント)だけ。
各エントリは -ln(p) を 1/16 刻みの整数(0..255)にして、16 進 2 桁で書く。

評価データ(エージェント生成、出典・ライセンス不明)は**使わない**。

実行: python3 -I tools/lang-detect-eval/gen_latin_bigram.py --repo . --out data/latin_bigram.txt
(元のパッケージ・辞書のバージョンで表が少し変わる。生成したときの元の件数は表のコメントに入る。)
"""
import argparse
import collections
import glob
import gettext
import math
import re
import subprocess
import unicodedata

ap = argparse.ArgumentParser()
ap.add_argument('--repo', default='.')
ap.add_argument('--out', default='data/latin_bigram.txt')
ap.add_argument('--alpha', type=float, default=0.5)
a = ap.parse_args()

# 打鍵位置の格子の文字(JIS の QWERTY。`lang_detect::LATIN_GRID` と同じ)
ROWS = ['1234567890-', 'qwertyuiop@[', 'asdfghjkl;:]', 'zxcvbnm,./']
POS = {ch: (r, c) for r, row in enumerate(ROWS) for c, ch in enumerate(row)}
ALPH = sorted(POS)
NA = len(ALPH)
LOC_LANGS = ['es', 'fr', 'de', 'pt', 'pt_BR', 'it', 'nl', 'sv', 'da', 'pl', 'cs', 'ro', 'hu', 'fi', 'tr', 'id', 'ms', 'vi']
EXCLUDE = ('tests/fixtures', 'data/', 'tools/lang-detect-eval-fixture', 'docs/tasks/adr280')


def keys_of(text):
    return [ch for ch in text.lower() if ch in POS]


def ascii_letters(text):
    t = unicodedata.normalize('NFKD', text)
    return [ch for ch in t.lower() if 'a' <= ch <= 'z']


def repo_streams(exts):
    files = subprocess.run(['git', '-C', a.repo, 'ls-files'] + [f'*.{e}' for e in exts], capture_output=True, text=True).stdout.split()
    out, n = [], 0
    for f in files:
        if f.startswith(EXCLUDE):
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
    ws = [w.strip() for w in open('/usr/share/dict/american-english', encoding='utf-8') if re.fullmatch(r'[A-Za-z]+', w.strip())]
    return [keys_of(w) for w in ws], len(ws)


def locale_streams(lang):
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


def counts(streams):
    bi, uni = collections.Counter(), collections.Counter()
    for ks in streams:
        for i, ch in enumerate(ks):
            uni[ch] += 1
            if i:
                bi[(ks[i - 1], ch)] += 1
    return bi, uni


def table(bi, uni):
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


def mix(parts):
    z = sum(w for w, _ in parts)
    pb = {k: sum(w * t[0][k] for w, t in parts) / z for k in parts[0][1][0]}
    pu = {k: sum(w * t[1][k] for w, t in parts) / z for k in parts[0][1][1]}
    return pb, pu


rs_md, n_md = repo_streams(['md'])
rs_rs, n_rs = repo_streams(['rs'])
ds, n_dict = dict_streams()
loc = {l: locale_streams(l) for l in LOC_LANGS}
e_repo = table(*counts(rs_md + rs_rs))
e_dict = table(*counts(ds))
e_mix = mix([(1, e_repo), (1, e_dict)])
code_rs = table(*counts(rs_rs))
loc_t = mix([(1, table(*counts(loc[l][0]))) for l in LOC_LANGS])
pb, pu = mix([(0.4, e_mix), (0.4, loc_t), (0.2, code_rs)])


def q(p):
    return min(255, round(-math.log(p) * 16))


lines = [
    '# awase: 入力言語の判定の背景(ラテン文字の 2 連)。tools/lang-detect-eval/gen_latin_bigram.py が生成する。手で編集しない。',
    '# 形式: chars= は格子の 45 文字(行の並びの順)。uni= は P(c)、続く 45 行は直前の文字ごとの P(c | 直前)。値は -ln(p) を 1/16 刻みの整数にした 16 進 2 桁。',
    f'# 元: repo *.md {n_md} files / *.rs {n_rs} files ({sum(map(len, rs_md))} + {sum(map(len, rs_rs))} keys), /usr/share/dict/american-english {n_dict} words ({sum(map(len, ds))} keys),',
    '#     /usr/share/locale/<lang>/LC_MESSAGES/*.mo (' + ', '.join(f'{l} {loc[l][1]} files' for l in LOC_LANGS) + ')',
    '# 重み: 0.4 * (repo と dict の等分) + 0.4 * (18 言語の翻訳の等分) + 0.2 * (repo の *.rs だけ)。加算平滑 alpha = 0.5。',
    'chars=' + ''.join(ALPH),
    'uni=' + ''.join(f'{q(pu[c]):02x}' for c in ALPH),
]
for p in ALPH:
    lines.append(''.join(f'{q(pb[(p, c)]):02x}' for c in ALPH))
open(a.out, 'w', encoding='utf-8').write('\n'.join(lines) + '\n')
print(f'wrote {a.out}: {NA} chars, {len(lines)} lines')
