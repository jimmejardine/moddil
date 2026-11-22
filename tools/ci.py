"""The single definition of CI for this repository.

The GitHub workflows in .github/workflows/ (checks.yml and test-*.yml) call this script step by
step, and the same script runs locally, so the two cannot drift.

    python tools/ci.py                 every step, here
    python tools/ci.py clippy test     chosen steps, here
    python tools/ci.py --wsl           every step inside WSL (Linux), on this same checkout
    python tools/ci.py --all           here, then inside WSL

Steps: fmt, clippy, deny, test, tour, links, grammars. Compatible with Python 3.8 (WSL's Ubuntu 20.04).
This script never installs anything; when a tool is missing it says how to install it.
"""

import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TOOLCHAIN = "1.95.0"  # keep in step with rust-toolchain.toml
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


def step_fmt():
    return run("cargo", "fmt", "--all", "--check")


def step_clippy():
    return run("cargo", "clippy", "--workspace", "--all-targets", "--", "-D", "warnings")


def step_deny():
    if not has_cargo_tool("deny"):
        raise Skip("cargo-deny is not installed: cargo install --locked cargo-deny")
    return run("cargo", "deny", "check")


def step_test():
    if has_cargo_tool("nextest"):
        ok = run("cargo", "nextest", "run", "--workspace")
    else:
        print("(cargo-nextest not installed; using cargo test. "
              "Install it with: cargo install --locked cargo-nextest)")
        ok = run("cargo", "test", "--workspace", "--all-targets")
    return run("cargo", "test", "--workspace", "--doc") and ok


def step_tour():
    # M0's gate (specs/roadmap.md): the tour, tour/shorty, parses and passes the type, effect and
    # ownership checks, against the signature-only standard mods in stubs/moddil. Its tests run
    # from M2.
    stubs = run("cargo", "run", "-q", "-p", "moddil", "--", "check", "stubs/moddil")
    tour = run("cargo", "run", "-q", "-p", "moddil", "--", "check", "tour/shorty")
    return stubs and tour


def step_links():
    return run(sys.executable, "-X", "utf8", "tools/check_spec_links.py")


def step_grammars():
    # The files generated from reference/grammar/moddil.ebnf are current; Lark and tree-sitter
    # accept every corpus case and sample (tools/check_grammars.py); and the hand-written parser
    # agrees with the grammar's recogniser on 10000 random edits of the corpus.
    current = run("cargo", "run", "--release", "-q", "-p", "moddil-grammar", "--", "generate", "--check")
    tools = run(sys.executable, "-X", "utf8", "tools/check_grammars.py")
    parser = run("cargo", "test", "--release", "-q", "-p", "moddil-syntax", "--test", "corpus",
                 "the_parser_agrees", env={"MODDIL_EDITS": "10000"})
    return current and tools and parser


STEPS = {
    "fmt": step_fmt,
    "clippy": step_clippy,
    "deny": step_deny,
    "test": step_test,
    "tour": step_tour,
    "links": step_links,
    "grammars": step_grammars,
}


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


def run_here(steps):
    check_toolchain()
    results = []
    for name in steps:
        print("\n=== {} ===".format(name), flush=True)
        start = time.time()
        try:
            status = "ok" if STEPS[name]() else "FAILED"
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


def run_in_wsl(steps):
    if os.name != "nt":
        sys.exit("--wsl runs from Windows")
    # A separate target directory on the Linux file system: Windows and Linux builds never share
    # target/, and builds avoid the slow /mnt/ file system.
    script = ("cd '{}' && CARGO_TARGET_DIR=\"$HOME/.cache/moddil-target\" "
              "python3 tools/ci.py {}").format(wsl_path(ROOT), " ".join(steps))
    print("\n##### WSL ({}) #####".format(WSL_DISTRO), flush=True)
    return subprocess.call(["wsl", "-d", WSL_DISTRO, "--", "bash", "-lc", script]) == 0


def main(argv):
    flags = {a for a in argv if a.startswith("--")}
    steps = [a for a in argv if not a.startswith("--")] or list(STEPS)
    unknown = [s for s in steps if s not in STEPS] + sorted(flags - {"--wsl", "--all"})
    if unknown:
        sys.exit("unknown: {}\n\n{}".format(", ".join(unknown), __doc__))
    ok = True
    if "--wsl" not in flags:
        ok = run_here(steps)
    if "--wsl" in flags or "--all" in flags:
        ok = run_in_wsl(steps) and ok
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
