"""The single definition of CI for this repository.

The GitHub workflows in .github/workflows/ (checks.yml and test-*.yml) call this script step by
step, and the same script runs locally, so the two cannot drift.

    python tools/ci.py                 every step, here
    python tools/ci.py clippy test     chosen steps, here
    python tools/ci.py quick [AREA...] the quick check, while working: what the changes affect
    python tools/ci.py --wsl           every step inside WSL (Linux), on this same checkout
    python tools/ci.py --all           here, then inside WSL

Steps: fmt, clippy, deny, test, tour, examples, links, grammars. Compatible with Python 3.8 (WSL's Ubuntu 20.04).

Three more run only when named: `loom`, the parallel runtime's sleeping workers checked under
`loom` (every order) and `shuttle` (random orders); `tsan`, the runtime's and scheduler's tests
under ThreadSanitizer, on Linux with a nightly toolchain (`ci.py --wsl tsan`); and `bench`, the
runtime's benchmarks, what each operation costs under each runtime (not gated).

`quick` is not one of them: it checks what the uncommitted changes (`git status`) affect. It
runs `cargo fmt`, the layout check on the changed `.moddil` files, the tests of each changed
crate, and the conformance areas whose files changed (on all four implementations). It cannot
see which areas a change to the compiler or the prelude reaches, so name them:
`ci.py quick borrows comparison`; a target may also be a package (`moddil-mir`). It leaves out
the CLI's tests (its commands, the projects on each runtime, the browser and Python tests),
`random_edits`, doc tests, clippy, deny, links, `tour`, `examples` and `grammars`: run the steps
themselves before committing.
This script never installs anything; when a tool is missing it says how to install it.
"""

import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TOOLCHAIN = "1.100.0"  # keep in step with rust-toolchain.toml
WSL_DISTRO = os.environ.get("MODDIL_WSL_DISTRO", "Ubuntu-20.04")
IN_CI = os.environ.get("CI") == "true"


class Skip(Exception):
    """A step that cannot run here; fatal on CI, a warning locally."""


def run(*args, env=None):
    print("$ " + " ".join(args), flush=True)
    full_env = dict(os.environ, **env) if env else None
    return subprocess.call(list(args), cwd=str(ROOT), env=full_env) == 0


def has_cargo_tool(name):
    return shutil.which("cargo-" + name) is not None


# The repository's Moddil sources kept in `moddil fmt`'s layout. The grammar's corpus
# (`reference/grammar/tests/`) is left as written: its cases are the grammar's, layout and all.
MODDIL_SOURCES = ["reference/2027/prelude", "mods", "tour", "examples", "conformance"]


def step_fmt():
    return run("cargo", "fmt", "--all", "--check") and run(
        "cargo", "run", "-q", "-p", "moddil", "--", "fmt", "--check", *MODDIL_SOURCES)


def step_clippy():
    return run("cargo", "clippy", "--workspace", "--all-targets", "--", "-D", "warnings")


def step_deny():
    if not has_cargo_tool("deny"):
        raise Skip("cargo-deny is not installed: cargo install --locked cargo-deny")
    return run("cargo", "deny", "check")


def step_test():
    # The runtime library `moddil build` links with, which nothing else builds (its tests are
    # the CLI's).
    if not run("cargo", "build", "-q", "-p", "moddil-rt-main"):
        return False
    if has_cargo_tool("nextest"):
        ok = run("cargo", "nextest", "run", "--workspace")
    else:
        print("(cargo-nextest not installed; using cargo test. "
              "Install it with: cargo install --locked cargo-nextest)")
        ok = run("cargo", "test", "--workspace", "--all-targets")
    return run("cargo", "test", "--workspace", "--doc") and ok


# What `quick` cannot see the reach of: a change to these crates can affect any area.
COMPILER = {"moddil-syntax", "moddil-sema", "moddil-mir", "moddil-interp", "moddil-refinterp",
            "moddil-native", "moddil-wasm"}
# Files every package depends on: a change to one runs every package's tests.
WORKSPACE_FILES = {"Cargo.toml", "Cargo.lock", "rust-toolchain.toml"}
SUITE = ROOT / "conformance" / "2027"
# The suite's checks of itself (`crates/conformance/tests/suite.rs`).
AREA_LIST_TEST = "every_area_has_a_test"
COVERAGE_TEST = "every_reference_paragraph_is_cited"
# Areas named by a keyword, whose tests are `r#if` and so on.
KEYWORD_AREAS = {"if", "match", "unsafe"}


def packages():
    """Each crate's directory under crates/, and its package name."""
    found = {}
    for toml in sorted((ROOT / "crates").glob("*/Cargo.toml")):
        m = re.search(r'^name\s*=\s*"([^"]+)"', toml.read_text(encoding="utf-8"), re.M)
        if m:
            found[toml.parent.name] = m.group(1)
    return found


def areas():
    return sorted(p.name for p in SUITE.iterdir() if p.is_dir())


def changed_paths():
    """The paths `git status` lists, both sides of a rename, with forward slashes."""
    out = subprocess.run(["git", "status", "--porcelain", "-uall"], stdout=subprocess.PIPE,
                         universal_newlines=True, cwd=str(ROOT)).stdout
    paths = []
    for line in out.splitlines():
        for part in line[3:].split(" -> "):
            paths.append(part.strip().strip('"'))
    return paths


def affected(paths, targets):
    """What `quick` runs for the changed `paths` and the named `targets`: the packages whose
    tests run, the conformance areas, the suite's own checks, the changed `.moddil` files, and
    notes on what it leaves to other steps."""
    crates = packages()
    names = set(crates.values())
    pkgs, run_areas, meta, moddil_files, notes = set(), set(), set(), set(), set()
    for path in paths:
        parts = path.split("/")
        if path.endswith(".moddil") and any(path.startswith(src + "/") for src in MODDIL_SOURCES):
            if (ROOT / path).is_file():
                moddil_files.add(path)
        if path in WORKSPACE_FILES or parts[0] == ".config":
            pkgs |= names - {"moddil"}
        elif parts[0] == "crates" and len(parts) > 2 and parts[1] in crates:
            name = crates[parts[1]]
            if name == "moddil":
                notes.add("the CLI changed: its tests run in the `test` step")
            else:
                pkgs.add(name)
        elif parts[:2] == ["conformance", "2027"] and len(parts) > 3:
            if (SUITE / parts[2]).is_dir():
                run_areas.add(parts[2])
            meta.add(AREA_LIST_TEST)
        elif parts[:2] == ["reference", "grammar"]:
            pkgs |= {"moddil-grammar", "moddil-syntax"}
        elif parts[:3] in (["reference", "2027", "prelude"], ["reference", "2027", "core"]):
            notes.add("the prelude or a core mod changed: name the areas it affects, "
                      "e.g. `ci.py quick iteration`")
        elif parts[:2] == ["reference", "2027"] and path.endswith(".md"):
            meta.add(COVERAGE_TEST)
        elif parts[0] in ("mods", "tour", "examples"):
            notes.add("{}/ changed: it runs in the `test`, `tour` and `examples` steps"
                      .format(parts[0]))
    for t in targets:
        if t in names:
            pkgs.add(t)
        else:
            run_areas.add(t)
    if pkgs & COMPILER and not run_areas:
        notes.add("no conformance areas ran: name the ones your change affects, "
                  "e.g. `ci.py quick borrows`")
    return pkgs, run_areas, meta, sorted(moddil_files), sorted(notes)


def bad_targets(targets):
    names = set(packages().values())
    known = set(areas())
    return [t for t in targets if t not in names and t not in known]


def step_quick(targets=()):
    # The CLI (`crates/moddil`) is never built: its tests build and run whole projects, in
    # browsers and Python too, and the layout check uses the formatter's example instead.
    pkgs, run_areas, meta, moddil_files, notes = affected(changed_paths(), targets)
    ok = run("cargo", "fmt", "--all", "--check")
    if moddil_files:
        ok = run("cargo", "run", "-q", "-p", "moddil-fmt", "--example", "fmt", "--",
                 "--check", *moddil_files) and ok
    suite = sorted(run_areas) + sorted(meta)
    if not pkgs and not suite:
        print("nothing the changes affect has tests here")
    elif has_cargo_tool("nextest"):
        filters = ["package({})".format(p) for p in sorted(pkgs)]
        if suite:
            filters.append("(package(moddil-conformance) & test(/^(r#)?({})$/))"
                           .format("|".join(suite)))
        selected = sorted(pkgs | ({"moddil-conformance"} if suite else set()))
        args = ["cargo", "nextest", "run"]
        for p in selected:
            args += ["-p", p]
        expr = "({}) & not test(random_edits)".format(" | ".join(filters))
        ok = run(*args, "-E", expr) and ok
    else:
        print("(cargo-nextest not installed; using cargo test, random_edits included. "
              "Install it with: cargo install --locked cargo-nextest)")
        for p in sorted(pkgs):
            ok = run("cargo", "test", "-p", p, "--all-targets") and ok
        if suite:
            names = ["r#" + n if n in KEYWORD_AREAS else n for n in suite]
            ok = run("cargo", "test", "-p", "moddil-conformance", "--test", "suite", "--",
                     "--exact", *names) and ok
    for n in notes:
        print("note: " + n)
    return ok


def step_tour():
    # M0's gate (specs/roadmap.md): the standard mods check together through the umbrella mod
    # mods/moddil, which nothing else checks as a whole. The tour itself, tour/shorty, is checked
    # and its tests run in the test step (crates/moddil/tests/projects.rs).
    return run("cargo", "run", "-q", "-p", "moddil", "--", "check", "mods/moddil")


def step_examples():
    # The Mandelbulb's CPU preview (specs/examples/mandelbulb.md) lights the same 386 pixels on
    # every platform (floats are IEEE 754, their functions from libm). Its tests run in the test
    # step (crates/moddil/tests/projects.rs).
    cmd = ["cargo", "run", "-q", "-p", "moddil", "--"]
    print("$", " ".join(cmd + ["run", "examples/mandelbulb"]), flush=True)
    out = subprocess.run(cmd + ["run", "examples/mandelbulb"], capture_output=True, text=True)
    lit = out.stdout.strip()
    preview = out.returncode == 0 and lit == "386"
    if not preview:
        print(f"the preview gave {lit!r} (exit {out.returncode}), not 386")
        print(out.stderr)
    return preview


def step_links():
    return run(sys.executable, "-X", "utf8", "tools/check_spec_links.py")


def step_grammars():
    # The files generated from reference/grammar/moddil.ebnf are current; Lark and tree-sitter
    # accept every corpus case and sample (tools/check_grammars.py); and the hand-written parser
    # agrees with the grammar's recogniser on 10000 random edits of the corpus.
    current = run("cargo", "run", "--release", "-q", "-p", "moddil-grammar", "--", "generate", "--check")
    tools = run(sys.executable, "-X", "utf8", "tools/check_grammars.py")
    parser = run("cargo", "test", "--release", "-q", "-p", "moddil-syntax", "--test", "corpus",
                 "random_edits", env={"MODDIL_EDITS": "10000"})
    return current and tools and parser


def step_loom():
    # Each its own build, as the `cfg` changes what the runtime is built from.
    ok = True
    for tool in ["loom", "shuttle"]:
        ok = run("cargo", "test", "-q", "-p", "moddil-rt", "--lib", "sleepers",
                 "--target-dir", str(ROOT / "target" / tool),
                 env={"RUSTFLAGS": "--cfg moddil_" + tool, "RUST_MIN_STACK": "16777216"}) and ok
    return ok


def step_tsan():
    if not sys.platform.startswith("linux"):
        raise Skip("ThreadSanitizer runs on Linux here: `ci.py --wsl tsan`")
    out = subprocess.run(["rustup", "toolchain", "list"], stdout=subprocess.PIPE,
                         universal_newlines=True, cwd=str(Path.home())).stdout
    if not any(line.startswith("nightly-") for line in out.splitlines()):
        raise Skip("ThreadSanitizer needs a nightly toolchain: rustup toolchain install nightly "
                   "--profile minimal --component rust-src; rustup target add "
                   "wasm32-unknown-unknown --toolchain nightly")
    # The sanitizer for the host's code only: the WASM runtime the build script makes has none.
    # Coroutines' stacks it does not follow yet, so the runs of tasks are left out.
    return run("cargo", "+nightly", "test", "-q", "-Zbuild-std", "--target",
               "x86_64-unknown-linux-gnu", "-p", "moddil-rt", "-p", "moddil-sched", "--lib",
               "--target-dir", str(ROOT / "target" / "tsan"),
               env={"CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS": "-Zsanitizer=thread"})


def step_bench():
    return run("cargo", "bench", "-q", "-p", "moddil-native", "--bench", "runtime")


STEPS = {
    "fmt": step_fmt,
    "clippy": step_clippy,
    "deny": step_deny,
    "test": step_test,
    "tour": step_tour,
    "examples": step_examples,
    "links": step_links,
    "grammars": step_grammars,
    "quick": step_quick,
    "loom": step_loom,
    "tsan": step_tsan,
    "bench": step_bench,
}

# The steps run when none are named: all but `quick`, a subset for working, and `loom`, `tsan`
# and `bench`, run when asked.
DEFAULT = [name for name in STEPS if name not in ("quick", "loom", "tsan", "bench")]


def check_toolchain():
    if shutil.which("rustup") is None or shutil.which("cargo") is None:
        sys.exit("rustup and cargo are needed: https://rustup.rs")
    # Run outside the repository: inside it, rust-toolchain.toml makes rustup install the pinned
    # toolchain automatically, and this check must not install anything.
    out = subprocess.run(["rustup", "toolchain", "list"], stdout=subprocess.PIPE,
                         universal_newlines=True, cwd=str(Path.home())).stdout
    if not any(line.startswith(TOOLCHAIN + "-") for line in out.splitlines()):
        sys.exit("The pinned toolchain {0} is not installed. Install it with:\n"
                 "  rustup toolchain install {0} --profile minimal "
                 "--component rustfmt clippy".format(TOOLCHAIN))


def run_here(steps, targets=()):
    check_toolchain()
    results = []
    for name in steps:
        print("\n=== {} ===".format(name), flush=True)
        start = time.time()
        try:
            done = STEPS[name](targets) if name == "quick" else STEPS[name]()
            status = "ok" if done else "FAILED"
        except Skip as e:
            status = "FAILED" if IN_CI else "skipped"
            print("{}: {}".format(status, e))
        results.append((name, status, time.time() - start))
    print("\n=== summary ({}) ===".format(sys.platform))
    for name, status, secs in results:
        print("  {:<7} {:<8} {:5.1f}s".format(name, status, secs))
    return all(status != "FAILED" for _, status, _ in results)


def wsl_path(path):
    drive, rest = os.path.splitdrive(str(path))
    return "/mnt/{}{}".format(drive.rstrip(":").lower(), rest.replace("\\", "/"))


def run_in_wsl(steps, targets=()):
    if os.name != "nt":
        sys.exit("--wsl runs from Windows")
    # A separate target directory on the Linux file system: Windows and Linux builds never share
    # target/, and builds avoid the slow /mnt/ file system.
    script = ("cd '{}' && CARGO_TARGET_DIR=\"$HOME/.cache/moddil-target\" "
              "python3 tools/ci.py {}").format(wsl_path(ROOT), " ".join(list(steps) + list(targets)))
    print("\n##### WSL ({}) #####".format(WSL_DISTRO), flush=True)
    return subprocess.call(["wsl", "-d", WSL_DISTRO, "--", "bash", "-lc", script]) == 0


def main(argv):
    flags = {a for a in argv if a.startswith("--")}
    words = [a for a in argv if not a.startswith("--")]
    # The words after `quick` that are not steps are its targets: areas or packages.
    targets = []
    if "quick" in words:
        after = words[words.index("quick") + 1:]
        targets = [w for w in after if w not in STEPS]
        words = [w for w in words if w not in targets]
        bad = bad_targets(targets)
        if bad:
            sys.exit("not an area or a package: {}\n\nareas: {}".format(
                ", ".join(bad), ", ".join(areas())))
    steps = words or DEFAULT
    unknown = [s for s in steps if s not in STEPS] + sorted(flags - {"--wsl", "--all"})
    if unknown:
        sys.exit("unknown: {}\n\n{}".format(", ".join(unknown), __doc__))
    ok = True
    if "--wsl" not in flags:
        ok = run_here(steps, targets)
    if "--wsl" in flags or "--all" in flags:
        ok = run_in_wsl(steps, targets) and ok
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
