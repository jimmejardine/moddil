"""Generates `mods/time/src/tzdata.moddil`, the IANA time zone data compiled into the time mod,
from the TZif files of Python's `tzdata` package (or a zoneinfo directory, `--dir`).

Each zone keeps its transitions up to where its POSIX TZ rule (the TZif footer) takes over, and
the rule. Zones whose files are the same (IANA's links) share one entry. `--check` decodes the
generated file as the mod does and compares it with `zoneinfo` at sampled instants and around
every transition.

    pip install tzdata==2026.6        # IANA 2026f
    python tools/zonedata.py
    python tools/zonedata.py --check
"""
import argparse
import os
import struct
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, 'mods', 'time', 'src', 'tzdata.moddil')
WIDTH = 96  # the layout's line width
LAST_FAT = 2**31  # fat TZif files list transitions up to 2037


def read_tzif(data):
    """The 64-bit section of a TZif file: (transitions [(t, type)], types [(utoff, dst, abbr)],
    footer)."""
    assert data[:4] == b'TZif' and data[4] >= ord('2'), 'a TZif file of version 2 or later'

    def header(at):
        counts = struct.unpack('>6l', data[at + 20:at + 44])
        return counts  # isutcnt, isstdcnt, leapcnt, timecnt, typecnt, charcnt

    isut, isstd, leap, timecnt, typecnt, charcnt = header(0)
    at = 44 + timecnt * 5 + typecnt * 6 + charcnt + leap * 8 + isstd + isut
    isut, isstd, leap, timecnt, typecnt, charcnt = header(at)
    at += 44
    times = struct.unpack(f'>{timecnt}q', data[at:at + timecnt * 8])
    at += timecnt * 8
    idx = data[at:at + timecnt]
    at += timecnt
    raw = []
    for i in range(typecnt):
        utoff, dst, ai = struct.unpack('>lBB', data[at + i * 6:at + i * 6 + 6])
        raw.append((utoff, dst, ai))
    at += typecnt * 6
    chars = data[at:at + charcnt]
    at += charcnt + leap * 12 + isstd + isut
    footer = data[at:].split(b'\n')[1].decode()
    types = [(u, d, chars[a:chars.index(b'\0', a)].decode()) for (u, d, a) in raw]
    return list(zip(times, idx)), types, footer


# POSIX TZ rules (RFC 8536 3.3), as the mod reads them.

def parse_posix(s):
    """`(std, std_off, dst, dst_off, start, end)`, offsets in seconds east of UTC; `dst` None when
    the rule has no daylight time. A date is ('M', m, w, d), ('J', n) or ('N', n), with a time in
    seconds."""
    at = 0

    def name():
        nonlocal at
        if s[at] == '<':
            e = s.index('>', at)
            n = s[at + 1:e]
            at = e + 1
            return n
        b = at
        while at < len(s) and s[at].isalpha():
            at += 1
        return s[b:at]

    def num():
        nonlocal at
        b = at
        while at < len(s) and s[at].isdigit():
            at += 1
        return int(s[b:at])

    def time():
        nonlocal at
        sign = 1
        if s[at] in '+-':
            sign = -1 if s[at] == '-' else 1
            at += 1
        t = num() * 3600
        for mul in (60, 1):
            if at < len(s) and s[at] == ':':
                at += 1
                t += num() * mul
        return sign * t

    def date():
        nonlocal at
        if s[at] == 'M':
            at += 1
            m = num()
            at += 1
            w = num()
            at += 1
            d = num()
            r = ('M', m, w, d)
        elif s[at] == 'J':
            at += 1
            r = ('J', num())
        else:
            r = ('N', num())
        t = 7200
        if at < len(s) and s[at] == '/':
            at += 1
            t = time()
        return r + (t,)

    std = name()
    std_off = -time()
    if at == len(s):
        return (std, std_off, None, None, None, None)
    dst = name()
    dst_off = std_off + 3600
    if at < len(s) and s[at] != ',':
        dst_off = -time()
    if at == len(s):  # no dates: the US rules of 1987 (POSIX), unused by tzdata
        raise ValueError(s)
    at += 1
    start = date()
    at += 1
    end = date()
    assert at == len(s), s
    return (std, std_off, dst, dst_off, start, end)


def is_leap(y):
    return y % 4 == 0 and (y % 100 != 0 or y % 400 == 0)


def days_from_civil(y, m, d):
    y -= m <= 2
    era = y // 400
    yoe = y - era * 400
    doy = (153 * ((m + 9) % 12) + 2) // 5 + d - 1
    doe = yoe * 365 + yoe // 4 - yoe // 100 + doy
    return era * 146097 + doe - 719468


def year_of(t):
    """The year of the UTC instant `t` (seconds)."""
    z = t // 86400 + 719468
    era = z // 146097
    doe = z - era * 146097
    yoe = (doe - doe // 1460 + doe // 36524 - doe // 146096) // 365
    doy = doe - (365 * yoe + yoe // 4 - yoe // 100)
    mp = (5 * doy + 2) // 153
    return yoe + era * 400 + (mp >= 10)


def rule_day(y, r):
    """Days since the epoch of the date `r` in year `y`."""
    if r[0] == 'J':
        n = r[1]
        return days_from_civil(y, 1, 1) + n - 1 + (is_leap(y) and n >= 60)
    if r[0] == 'N':
        return days_from_civil(y, 1, 1) + r[1]
    _, m, w, d, _t = r
    first = days_from_civil(y, m, 1)
    wd = (first + 4) % 7  # 0 is Sunday
    day = first + (d - wd) % 7 + (w - 1) * 7
    dim = [31, 29 if is_leap(y) else 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][m - 1]
    while day >= first + dim:
        day -= 7
    return day


def rule_bounds(rule, y):
    """The UTC instants DST starts and ends in year `y`."""
    _, std_off, _, dst_off, start, end = rule
    s = rule_day(y, start) * 86400 + start[-1] - std_off
    e = rule_day(y, end) * 86400 + end[-1] - dst_off
    return s, e


def rule_state(rule, t):
    std, std_off, dst, dst_off, start, end = rule
    if dst is None:
        return (std_off, 0, std)
    y = year_of(t + std_off)
    s, e = rule_bounds(rule, y)
    if s < e:
        on = s <= t < e
    else:
        on = not (e <= t < s)
    return (dst_off, 1, dst) if on else (std_off, 0, std)


def rule_changes(rule, lo, hi):
    """The instants in [lo, hi] where the rule's state changes."""
    if rule[2] is None:
        return []
    out = []
    for y in range(year_of(lo) - 1, year_of(hi) + 2):
        for t in rule_bounds(rule, y):
            if lo <= t <= hi and rule_state(rule, t) != rule_state(rule, t - 1):
                out.append(t)
    return sorted(set(out))


def fat_state(trans, types, t):
    """The state at `t` from the transitions alone (before the footer takes over)."""
    lo, hi = 0, len(trans)
    while lo < hi:
        mid = (lo + hi) // 2
        if trans[mid][0] <= t:
            lo = mid + 1
        else:
            hi = mid
    k = lo - 1
    return types[0] if k < 0 else types[trans[k][1]]


def trim(trans, types, rule):
    """How many transitions to keep: from the last kept one on, the rule gives every state the
    transitions do, up to the last of them."""
    n = len(trans)
    if n == 0:
        return 0
    last = trans[-1][0]
    changes = set(t for t, _ in trans)

    def rule_from(k):
        t = trans[k][0]
        return (rule_state(rule, t) == types[trans[k][1]]
                and all(rule_state(rule, trans[j][0]) == types[trans[j][1]]
                        and rule_state(rule, trans[j][0] - 1) == types[trans[j - 1][1]]
                        for j in range(k + 1, n))
                and all(c in changes for c in rule_changes(rule, t, last)))

    keep = n
    while keep > 1 and rule_from(keep - 2):
        keep -= 1
    return keep


def encode(trans, types, footer):
    """`abbr,utoff,dst;…|` then each transition as a letter for its type and the time (the first)
    or the seconds since the one before, `|` and the footer."""
    used = sorted(set([0] + [i for _, i in trans]))
    remap = {o: n for n, o in enumerate(used)}
    assert len(used) <= 26
    head = ';'.join(f'{types[i][2]},{types[i][0]},{types[i][1]}' for i in used)
    body = []
    prev = None
    for t, i in trans:
        body.append(chr(97 + remap[i]) + str(t if prev is None else t - prev))
        prev = t
    return f'{head}|{"".join(body)}|{footer}'


def decode(s):
    head, body, footer = s.split('|', 2)
    types = []
    for part in head.split(';'):
        a, u, d = part.rsplit(',', 2)
        types.append((int(u), int(d), a))
    trans = []
    at = 0
    prev = None
    while at < len(body):
        i = ord(body[at]) - 97
        at += 1
        b = at
        while at < len(body) and (body[at].isdigit() or body[at] == '-'):
            at += 1
        v = int(body[b:at])
        prev = v if prev is None else prev + v
        trans.append((prev, i))
    return trans, types, parse_posix(footer)


def state(zone, t):
    """The zone's state at `t`, as the mod finds it."""
    trans, types, rule = zone
    if not trans or t >= trans[-1][0]:
        return rule_state(rule, t)
    return fat_state(trans, types, t)


def sources(args):
    """(version, {name: bytes})."""
    if args.dir:
        base = args.dir
        version = args.version or 'unknown'
        names = []
        for d, _, fs in os.walk(base):
            for f in fs:
                p = os.path.join(d, f)
                with open(p, 'rb') as h:
                    if h.read(4) == b'TZif':
                        names.append(os.path.relpath(p, base).replace(os.sep, '/'))
    else:
        import tzdata
        base = os.path.join(os.path.dirname(tzdata.__file__), 'zoneinfo')
        version = tzdata.IANA_VERSION
        names = [n.strip() for n in open(os.path.join(os.path.dirname(base), 'zones'))
                 if n.strip()]
    out = {}
    for n in sorted(set(names)):
        if n.startswith(('posix/', 'right/')) or n in ('posixrules', 'localtime', 'Factory'):
            continue
        with open(os.path.join(base, n), 'rb') as h:
            out[n] = h.read()
    return version, out


def wrap(text, first, indent):
    """A string literal of `text` after `first` columns of its line, broken with `\\`
    continuations indented `indent` to fit the line width. A continuation leaves out the next
    line's leading white space, so the data may hold no spaces."""
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


def generate(args):
    version, files = sources(args)
    entries = []  # encoded data, in first-seen order
    index = {}
    names = []
    for name, data in files.items():
        trans, types, footer = read_tzif(data)
        rule = parse_posix(footer)
        trans = trans[:trim(trans, types, rule)]
        enc = encode(trans, types, footer)
        if enc not in index:
            index[enc] = len(entries)
            entries.append(enc)
        names.append((name, index[enc]))
    names.sort()
    table = ';'.join(f'{n}={i}' for n, i in names)
    out = [
        f'//! The IANA time zone database, release {version}, generated by `tools/zonedata.py` from',
        '//! its TZif files: do not edit. Each zone is its offsets (`abbreviation,seconds east of',
        '//! UTC,daylight`), `|`, its transitions (a letter for the offset, then the first',
        '//! instant in Unix seconds or the seconds since the one before), `|` and its POSIX TZ',
        '//! rule for the instants after its last transition. The names are sorted, each with',
        '//! the index of its zone; a link shares its target\'s.',
        '',
        f'pub(mod) const VERSION: &str = "{version}";',
        '',
        f'pub(mod) const NAMES: &str = {wrap(table, 29, 4)};',
        '',
        '/// The data of the zone the names table gives index `i`.',
        'pub(mod) fn zone(i: int) -> &str {',
        '    match i {',
    ]
    for n, e in enumerate(entries):
        arm = f'        {n:<{len(str(len(entries) - 1))}} => '  # the layout lines up the arrows
        out.append(arm + wrap(e, len(arm), 12) + ',')
    out += ['        _ => "",', '    }', '}', '']
    with open(OUT, 'w', encoding='utf-8', newline='\n') as h:
        h.write('\n'.join(out))
    print(f'{len(names)} names, {len(entries)} zones, {os.path.getsize(OUT)} bytes: {OUT}')
    if not args.no_fmt:
        subprocess.run(['cargo', 'run', '-q', '-p', 'moddil', '--', 'fmt', OUT], cwd=ROOT,
                       check=True)


def read_generated():
    text = open(OUT, encoding='utf-8').read()

    def literal(start):
        """The string literal starting at `start`, its continuations joined."""
        assert text[start] == '"'
        i = start + 1
        out = []
        while text[i] != '"':
            if text[i] == '\\' and text[i + 1] == '\n':
                i += 2
                while text[i] in ' \t':
                    i += 1
                continue
            out.append(text[i])
            i += 1
        return ''.join(out), i + 1

    names, _ = literal(text.index('"', text.index('const NAMES')))
    at = text.index('match i {')
    zones = []
    while True:
        q = text.find('"', at)
        if q < 0:
            break
        s, at = literal(q)
        if s:
            zones.append(s)
    table = {}
    for part in names.split(';'):
        n, i = part.split('=')
        table[n] = int(i)
    return table, zones


def check(_args):
    import zoneinfo
    from datetime import datetime, timezone
    table, zones = read_generated()
    decoded = [decode(z) for z in zones]
    bad = 0
    samples = list(range(-3786825600, 7258118400, 86400 * 37 + 3607))
    for name, i in sorted(table.items()):
        z = zoneinfo.ZoneInfo(name)
        zone = decoded[i]
        trans = zone[0]
        ts = list(samples)
        for t, _ in trans:
            ts += [t - 1, t, t + 1]
        # The rule's own changes, a century on.
        if zone[2][2] is not None:
            for y in (2030, 2037, 2038, 2050, 2100, 2199):
                for t in rule_bounds(zone[2], y):
                    ts += [t - 1, t, t + 1]
        for t in ts:
            want = datetime.fromtimestamp(t, timezone.utc).astimezone(z)
            if t < -2208988800 * 2:
                continue
            u, _d, a = state(zone, t)
            if u != int(want.utcoffset().total_seconds()) or a != want.tzname():
                bad += 1
                if bad < 20:
                    print(f'{name} at {t}: {u} {a}, zoneinfo {want.utcoffset()} {want.tzname()}')
                break
    print(f'{len(table)} names checked, {bad} differ')
    sys.exit(1 if bad else 0)


def main():
    p = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    p.add_argument('--check', action='store_true', help='compare the generated file with zoneinfo')
    p.add_argument('--dir', help='a zoneinfo directory instead of the tzdata package')
    p.add_argument('--version', help='the IANA release of --dir')
    p.add_argument('--no-fmt', action='store_true', help='leave the output unformatted')
    args = p.parse_args()
    (check if args.check else generate)(args)


if __name__ == '__main__':
    main()
