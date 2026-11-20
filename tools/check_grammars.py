"""Checks the grammars generated from reference/grammar/moddil.ebnf with their own tools.

    python tools/check_grammars.py            every check
    python tools/check_grammars.py lark       one check (lark, tree-sitter)

Each parses every valid corpus case (reference/grammar/tests/valid.mdl) and the hand-written
samples (samples.mdl), which must all be accepted. The GBNF is checked by moddil-grammar's own
Rust tests. A missing tool is a skip locally and a failure on CI (CI=true).

Tools: lark (pip install lark==1.2.2); tree-sitter (npx tree-sitter-cli@0.26.13, plus a C compiler).
Compatible with Python 3.8.
"""

import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TESTS = ROOT / "reference" / "grammar" / "tests"
GENERATED = ROOT / "reference" / "grammar" / "generated"
TREE_SITTER = ROOT / "editors" / "tree-sitter-moddil"
TREE_SITTER_CLI = "tree-sitter-cli@0.26.13"
IN_CI = os.environ.get("CI") == "true"


class Skip(Exception):
    pass


def cases():
    """(name, text) for every case that must parse."""
    out = []
    for file in ("valid.mdl", "samples.mdl"):
        text = (TESTS / file).read_text(encoding="utf-8").replace("\r\n", "\n")
        for i, case in enumerate(text.split("// ---\n")[1:], 1):
            out.append(("{}#{}".format(file, i), case))
    return out


def check_lark():
    try:
        import lark
    except ImportError:
        raise Skip("lark is not installed: pip install lark==1.2.2")
    grammar = (GENERATED / "moddil.lark").read_text(encoding="utf-8")
    parser = lark.Lark(grammar, parser="earley", lexer="dynamic", start="start")
    failures = []
    for name, text in cases():
        try:
            parser.parse(text)
        except lark.exceptions.LarkError as e:
            failures.append((name, str(e).splitlines()[0]))
    report("lark", failures)
    return not failures


def check_tree_sitter():
    npx = shutil.which("npx")
    if npx is None:
        raise Skip("node and npx are needed for the tree-sitter CLI")
    cli = [npx, "--yes", TREE_SITTER_CLI]
    if subprocess.call(cli + ["generate"], cwd=str(TREE_SITTER)) != 0:
        return False
    with tempfile.TemporaryDirectory() as tmp:
        names = {}
        for i, (name, text) in enumerate(cases()):
            path = Path(tmp) / "case{:04}.mdl".format(i)
            path.write_text(text, encoding="utf-8")
            names[path.name] = name
        listing = Path(tmp) / "paths.txt"
        listing.write_text("\n".join(str(Path(tmp) / f) for f in sorted(names)), encoding="utf-8")
        # One run for every file; with --quiet only files with errors are reported.
        result = subprocess.run(cli + ["parse", "--quiet", "--paths", str(listing)],
                                cwd=str(TREE_SITTER), stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        out = result.stdout.decode("utf-8", "replace")
        failures = []
        for line in out.splitlines():
            for file, name in names.items():
                if file in line and ("ERROR" in line or "MISSING" in line):
                    failures.append((name, line.split(file, 1)[1].strip()))
        if result.returncode != 0 and not failures:
            print(out[-2000:])
            return False
    report("tree-sitter", failures)
    return not failures


def report(tool, failures):
    total = len(cases())
    print("{}: {} of {} cases parse".format(tool, total - len(failures), total))
    for name, why in failures[:20]:
        print("  {}: {}".format(name, why))


CHECKS = {"lark": check_lark, "tree-sitter": check_tree_sitter}


def main():
    chosen = sys.argv[1:] or list(CHECKS)
    ok = True
    for name in chosen:
        try:
            ok = CHECKS[name]() and ok
        except Skip as e:
            print("{}: skipped: {}".format(name, e))
            if IN_CI:
                ok = False
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
