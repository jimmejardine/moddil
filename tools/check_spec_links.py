"""Check the Moddil specs for broken links and status drift.

Checks:
- every relative Markdown link in specs/ (file and #heading anchor) resolves, using GitHub's
  heading-slug rules (also understood by Obsidian's "GFM Heading Links" plugin);
- every status in the overview's Contents table matches the status line under that section.

Usage (from the repository root):
    python tools/check_spec_links.py            # check only
    python tools/check_spec_links.py --align    # also re-align the overview's Contents table

Exit code is 1 if any problem is found, so it can run in CI or a pre-commit hook.
"""

import pathlib, re, sys, collections

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8")

args = [a for a in sys.argv[1:] if not a.startswith("--")]
SPECS = (pathlib.Path(args[0]) if args else pathlib.Path(__file__).resolve().parent.parent / "specs").resolve()
ALIGN = "--align" in sys.argv


def slug(text):
    return re.sub(r"[^\w\- ]", "", text.strip().lower()).replace(" ", "-")


def headings(text):
    out, in_code = [], False
    for l in text.split("\n"):
        if l.strip().startswith("```"):
            in_code = not in_code
            continue
        if not in_code:
            m = re.match(r"^#{1,6} (.+?)\s*$", l)
            if m:
                out.append(m.group(1))
    return out


def slugs(text):
    seen, out = collections.Counter(), set()
    for h in headings(text):
        s = slug(h)
        out.add(s if seen[s] == 0 else f"{s}-{seen[s]}")
        seen[s] += 1
    return out


texts = {p: open(p, encoding="utf-8", newline="").read().replace("\r\n", "\n") for p in SPECS.rglob("*.md")}
anchors = {p: slugs(t) for p, t in texts.items()}
LINK = re.compile(r"\]\((?!https?:|mailto:)([^)\s]*?)(?:#([^)\s]+))?\)")
bad = 0
for p, t in texts.items():
    no_code = re.sub(r"```.*?```", "", t, flags=re.S)
    for m in LINK.finditer(no_code):
        path, anchor = m.group(1), m.group(2)
        if not path and not anchor:
            continue
        tgt = (p.parent / path).resolve() if path else p
        if path and not tgt.exists():
            bad += 1; print("MISSING FILE", p.relative_to(SPECS), path)
            continue
        if anchor and tgt.suffix == ".md" and anchor not in anchors.get(tgt, set()):
            bad += 1; print("BAD ANCHOR", p.relative_to(SPECS), f"{path}#{anchor}")
print("link problems:", bad)

# ---- overview contents: statuses match the section status lines; realign
ov = SPECS / "overview.md"
raw = open(ov, encoding="utf-8", newline="").read()
nl = "\r\n" if "\r\n" in raw else "\n"
lines = raw.split(nl)
i = next(k for k, l in enumerate(lines) if l.startswith("| Section "))
j = i
while lines[j].startswith("|"):
    j += 1
rows = [[c.strip() for c in l.strip().strip("|").split("|")] for l in lines[i:j]]
status_re = re.compile(r"(⚪ not started|🔴 superficial|🟠 major questions|🟡 minor questions|🟢 complete|reference)")
text = nl.join(lines)
hmap = {slug(h): h for h in headings(text.replace(nl, "\n"))}
mism = 0
for r in rows[2:]:
    a = re.search(r"\]\(#([^)]+)\)", r[0]).group(1)
    if r[1] == "—":
        continue
    h = hmap.get(a)
    idx = text.index(next(l for l in lines if re.match(r"^#{2,3} " + re.escape(h) + r"\s*$", l)))
    sm = status_re.search(text[idx:idx + 600].replace("**", ""))
    if not sm or sm.group(1) != r[1]:
        mism += 1; print("STATUS MISMATCH", h, r[1], "vs", sm and sm.group(1))
print("status mismatches:", mism)


def dw(s):
    return sum(2 if (ord(ch) >= 0x1F300 or ord(ch) in (0x26AA, 0x26AB)) else 1 for ch in s)


if ALIGN:
    w = [max(dw(r[k]) for r in rows if not set(r[k]) <= set("-:")) for k in range(len(rows[0]))]
    out = []
    for r in rows:
        if all(set(c) <= set("-:") for c in r):
            out.append("| " + " | ".join("-" * w[k] for k in range(len(r))) + " |")
        else:
            out.append("| " + " | ".join(r[k] + " " * (w[k] - dw(r[k])) for k in range(len(r))) + " |")
    lines[i:j] = out
    open(ov, "w", encoding="utf-8", newline="").write(nl.join(lines))
    print("aligned contents table")

sys.exit(1 if (bad or mism) else 0)
