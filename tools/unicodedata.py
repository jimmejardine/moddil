"""Generates the Unicode data of the text mod: `mods/text/src/data.moddil`, the classes it
segments graphemes with (UAX #29, extended grapheme clusters), and `mods/text/tests/src/cases.moddil`,
Unicode's own test cases for them, from the Unicode Character Database files of one version.

A code point's class is its Grapheme_Cluster_Break value, its Indic_Conjunct_Break value (for
rule GB9c) and whether it is Extended_Pictographic (for GB11). The classes are packed as ranges,
an entry of a fixed width each, which the mod finds by binary search. `--check` segments every
case of GraphemeBreakTest.txt with the generated table and a transcription of the mod's rules.

    python tools/unicodedata.py --dir <a directory with the four files>
    python tools/unicodedata.py                # downloads Unicode 16.0.0's from unicode.org
    python tools/unicodedata.py --check --dir <the same directory>

The four files: GraphemeBreakProperty.txt and GraphemeBreakTest.txt (`ucd/auxiliary/`),
emoji-data.txt (`ucd/emoji/`) and DerivedCoreProperties.txt (`ucd/`).
"""
import argparse
import os
import re
import subprocess
import sys
import urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DATA = os.path.join(ROOT, 'mods', 'text', 'src', 'data.moddil')
CASES = os.path.join(ROOT, 'mods', 'text', 'tests', 'src', 'cases.moddil')
VERSION = '16.0.0'
FILES = {
    'GraphemeBreakProperty.txt': 'auxiliary/',
    'GraphemeBreakTest.txt': 'auxiliary/',
    'emoji-data.txt': 'emoji/',
    'DerivedCoreProperties.txt': '',
}
WIDTH = 96  # the layout's line width

# Grapheme_Cluster_Break values, each a letter in the table.
GCB = {
    'Other': 'o', 'CR': 'c', 'LF': 'l', 'Control': 'x', 'Extend': 'e', 'ZWJ': 'z',
    'Regional_Indicator': 'r', 'Prepend': 'p', 'SpacingMark': 's', 'L': 'L', 'V': 'V',
    'T': 'T', 'LV': 'v', 'LVT': 't',
}
# Indic_Conjunct_Break values.
INCB = {'None': 'n', 'Linker': 'k', 'Consonant': 'c', 'Extend': 'e'}
# The letters that name a class in the ranges, by its index in `KINDS`.
ALPHABET = '0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz'


def ranges_of(path, prop=None):
    """`(first, last, value)` for each line of a UCD property file (of property `prop`, for a
    file of several)."""
    out = []
    with open(path, encoding='utf-8') as h:
        for line in h:
            line = line.split('#')[0].strip()
            if not line:
                continue
            fields = [f.strip() for f in line.split(';')]
            if prop is not None:
                if len(fields) < 3 or fields[1] != prop:
                    continue
                value = fields[2]
            else:
                value = fields[1]
            span = fields[0].split('..')
            first = int(span[0], 16)
            last = int(span[-1], 16)
            out.append((first, last, value))
    return out


def classes(src):
    """Each code point's class, `(gcb, incb, extpict)` as letters, over the whole range."""
    n = 0x110000
    gcb = ['o'] * n
    for a, b, v in ranges_of(os.path.join(src, 'GraphemeBreakProperty.txt')):
        for c in range(a, b + 1):
            gcb[c] = GCB[v]
    incb = ['n'] * n
    for a, b, v in ranges_of(os.path.join(src, 'DerivedCoreProperties.txt'), 'InCB'):
        for c in range(a, b + 1):
            incb[c] = INCB[v]
    ext = ['n'] * n
    for a, b, v in ranges_of(os.path.join(src, 'emoji-data.txt')):
        if v == 'Extended_Pictographic':
            for c in range(a, b + 1):
                ext[c] = 'y'
    return [gcb[c] + incb[c] + ext[c] for c in range(n)]


def pack(cls):
    """The kinds (each class once, in the order first met) and the ranges: each a 6-digit hex
    start and its kind's letter, where the class changes."""
    kinds = []
    ranges = []
    last = None
    for c, k in enumerate(cls):
        if k != last:
            if k not in kinds:
                kinds.append(k)
            ranges.append(f'{c:06x}{ALPHABET[kinds.index(k)]}')
            last = k
    assert len(kinds) <= len(ALPHABET), f'{len(kinds)} kinds'
    return kinds, ranges


def wrap(text, first, indent):
    """A string literal of `text` after `first` columns of its line, broken with `\\`
    continuations indented `indent` to fit the line width (as `tools/zonedata.py` writes)."""
    assert ' ' not in text
    parts = []
    room = WIDTH - first - 2
    while True:
        parts.append(text[:room])
        text = text[room:]
        room = WIDTH - indent - 2
        if not text:
            break
    lines = []
    for i, p in enumerate(parts):
        end = '\\' if i + 1 < len(parts) else '"'
        lines.append(('"' if i == 0 else ' ' * indent) + p + end)
    return '\n'.join(lines)


def cases_of(src):
    """GraphemeBreakTest.txt's cases: each its code points and the byte offsets of its
    boundaries, in UTF-8."""
    out = []
    with open(os.path.join(src, 'GraphemeBreakTest.txt'), encoding='utf-8') as h:
        for line in h:
            line = line.split('#')[0].strip()
            if not line:
                continue
            cps = []
            bounds = []
            at = 0
            for tok in line.split():
                if tok == '÷':
                    bounds.append(at)
                elif tok == '×':
                    pass
                else:
                    cp = int(tok, 16)
                    cps.append(cp)
                    at += len(chr(cp).encode('utf-8', 'surrogatepass'))
            out.append((cps, bounds))
    return out


def segment(cps, kind_of):
    """The boundaries of `cps` by the mod's rules, as byte offsets: a transcription of
    `bounds` in `mods/text/src/lib.moddil`."""
    out = []
    at = 0
    prev = None
    ri = 0
    emoji = False
    zwj = False
    incb = 0
    for cp in cps:
        g, ic, ep = kind_of(cp)
        if prev is None or breaks(prev, g, ic, ep, ri, zwj, incb):
            out.append(at)
        zwj = g == 'z' and emoji
        emoji = ep == 'y' or (g == 'e' and emoji)
        ri = ri + 1 if g == 'r' else 0
        if ic == 'c':
            incb = 1
        elif incb > 0 and ic == 'k':
            incb = 2
        elif incb > 0 and ic == 'e':
            pass
        else:
            incb = 0
        prev = g
        at += len(chr(cp).encode('utf-8', 'surrogatepass'))
    if out:
        out.append(at)
    return out


def breaks(prev, g, ic, ep, ri, zwj, incb):
    """Whether there is a boundary between a code point of break class `prev` and one of `g`."""
    if prev == 'c' and g == 'l':  # GB3
        return False
    if prev in 'clx' or g in 'clx':  # GB4, GB5
        return True
    if prev == 'L' and g in 'LVvt':  # GB6
        return False
    if prev in 'vV' and g in 'VT':  # GB7
        return False
    if prev in 'tT' and g == 'T':  # GB8
        return False
    if g in 'ez' or g == 's' or prev == 'p':  # GB9, GB9a, GB9b
        return False
    if incb == 2 and ic == 'c':  # GB9c
        return False
    if zwj and ep == 'y':  # GB11
        return False
    if prev == 'r' and g == 'r' and ri % 2 == 1:  # GB12, GB13
        return False
    return True  # GB999


def fetch(dest):
    os.makedirs(dest, exist_ok=True)
    for name, sub in FILES.items():
        url = f'https://www.unicode.org/Public/{VERSION}/ucd/{sub}{name}'
        path = os.path.join(dest, name)
        if not os.path.exists(path):
            req = urllib.request.Request(url, headers={'User-Agent': 'moddil-tools'})
            with urllib.request.urlopen(req) as r, open(path, 'wb') as h:
                h.write(r.read())
    return dest


def source(args):
    if args.dir:
        return args.dir
    return fetch(os.path.join(ROOT, 'target', 'ucd', VERSION))


def generate(args):
    src = source(args)
    kinds, ranges = pack(classes(src))
    out = [
        f'//! The Unicode {VERSION} data the text mod segments graphemes with (UAX #29), generated',
        '//! by `tools/unicodedata.py` from the Unicode Character Database: do not edit. `RANGES` is',
        '//! where each class starts: six hex digits of the first code point, then the class\'s letter',
        '//! (its index in `KINDS`, in `ALPHABET`). `KINDS` is each class: its Grapheme_Cluster_Break',
        '//! value, its Indic_Conjunct_Break value and whether it is Extended_Pictographic, a letter',
        '//! each.',
        '',
        f'pub(mod) const VERSION: &str = "{VERSION}";',
        '',
        f'pub(mod) const ALPHABET: &str = "{ALPHABET[:len(kinds)]}";',
        '',
        f'pub(mod) const KINDS: &str = {wrap("".join(kinds), 30, 4)};',
        '',
        f'pub(mod) const RANGES: &str = {wrap("".join(ranges), 31, 4)};',
        '',
    ]
    write(DATA, out, args)
    cases = cases_of(src)
    packed = ';'.join('.'.join(str(c) for c in cps) + ':' + '.'.join(str(b) for b in bounds)
                      for cps, bounds in cases)
    lines = [
        f'//! GraphemeBreakTest.txt of Unicode {VERSION}, generated by `tools/unicodedata.py`: do not',
        '//! edit. Each case is its code points, `:`, and the byte offsets of its boundaries in',
        '//! UTF-8, each list `.`-separated; the cases are `;`-separated.',
        '',
        f'pub(mod) const CASES: &str = {wrap(packed, 30, 4)};',
        '',
    ]
    write(CASES, lines, args)
    print(f'{len(kinds)} classes, {len(ranges)} ranges, {len(cases)} cases')


def write(path, lines, args):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, 'w', encoding='utf-8', newline='\n') as h:
        h.write('\n'.join(lines))
    if not args.no_fmt:
        subprocess.run(['cargo', 'run', '-q', '-p', 'moddil', '--', 'fmt', path], cwd=ROOT,
                       check=True)


def read_generated():
    """`KINDS`, `ALPHABET` and `RANGES` as the generated file holds them."""
    with open(DATA, encoding='utf-8') as h:
        text = h.read()

    def const(name):
        m = re.search(rf'const {name}: &str = (".*?");', text, re.S)
        lit = m.group(1)
        return re.sub(r'\\\n\s*', '', lit)[1:-1]

    return const('KINDS'), const('ALPHABET'), const('RANGES')


def check(args):
    src = source(args)
    kinds_text, alphabet, ranges = read_generated()
    kinds = [kinds_text[i:i + 3] for i in range(0, len(kinds_text), 3)]
    starts = [int(ranges[i:i + 6], 16) for i in range(0, len(ranges), 7)]
    letters = [ranges[i + 6] for i in range(0, len(ranges), 7)]

    def kind_of(cp):
        lo, hi = 0, len(starts)
        while hi - lo > 1:
            mid = (lo + hi) // 2
            if starts[mid] <= cp:
                lo = mid
            else:
                hi = mid
        k = kinds[alphabet.index(letters[lo])]
        return k[0], k[1], k[2]

    cases = cases_of(src)
    bad = 0
    for cps, bounds in cases:
        got = segment(cps, kind_of)
        if got != bounds:
            bad += 1
            if bad <= 10:
                print('mismatch:', ' '.join(f'{c:04X}' for c in cps), bounds, got)
    print(f'{len(cases) - bad} of {len(cases)} cases agree')
    return 1 if bad else 0


def main():
    p = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    p.add_argument('--check', action='store_true', help='segment GraphemeBreakTest.txt with it')
    p.add_argument('--dir', help='a directory with the UCD files, instead of downloading them')
    p.add_argument('--no-fmt', action='store_true', help='leave the output unformatted')
    args = p.parse_args()
    sys.exit(check(args) if args.check else generate(args))


if __name__ == '__main__':
    main()
