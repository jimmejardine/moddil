# Packages and Supply-Chain Security

> **Status:** decided direction, open questions remain. Expands [`overview.md`](overview.md) §2.16.
> Manifest syntax, commands and file formats are **illustration only**.

## 1. Summary

| Rule | One line |
|---|---|
| **Plain git, nothing else** | Mods are hosted in ordinary git repositories. No registry, mirror, checksum log or index owns names or is needed to build. |
| **Names are URLs** | `github.com/x/y`, with optional vanity domains so that an owner can change host without renaming. |
| **Versions are tags, pinned by hash** | Semver git tags for humans; the lockfile pins a content hash of each mod's tree. |
| **One version per major** | The major is part of the path (`x`, `x/v2`); minimal version selection within a major. |
| **Semver is compiler-checked** | A breaking API change without a major bump is refused at publish and flagged at update. |
| **Randomised cooldown** | A new version becomes eligible after 7–28 days, a delay that differs per project, dependency and version, so adoption is staggered like a canary. |
| **Capability-scoped dependencies** | The manifest sets a ceiling per mod, and authority flows as values; upgrades show capability diffs. |
| **No code at fetch or build time** | A declarative manifest plus pure comptime; no install scripts and no `build.rs`. |
| **The source is the release** | No tarballs or uploaded artefacts that could differ from the repository. |
| **Audit trail in git** | Reviews of dependency versions are recorded in the consumer's repository, saying whether a human or an agent reviewed them. |
| **Local sources** | Local git repositories, user-level source overrides, and `moddil.work` workspaces for live working copies; hashes still enforced for tagged versions (§4.13). |

## 2. Problem

Package managers made code reuse trivially easy, and in doing so made the dependency graph the
largest attack surface in modern software:

- **Code execution at install or build time**: npm `postinstall`, Python `setup.py`, Rust `build.rs`
  and proc macros all run arbitrary code on the developer's machine. The 2025 npm worms spread this way.
- **Ambient authority**: any dependency, however deep, can open sockets, read `~/.ssh` or spawn
  processes. A left-pad library has the same power as your web server.
- **Fast-moving attacks**: a hijacked maintainer account publishes a malicious version (`chalk` and
  `debug` in 2025, `ua-parser-js` in 2021), and every project with a `^1.2` range picks it up within
  hours.
- **Long-con attacks**: the `xz-utils` backdoor (CVE-2024-3094) lived in release tarballs that did not
  match the repository.
- **Typosquatting**, account takeover and maintainer burnout.
- **Transitive dependency counts** in the hundreds or thousands that nobody reviews.

## 3. State of the art

| System | What it does well | Limits |
|---|---|---|
| **Go modules** | Names are URLs; **minimal version selection** (deterministic, no SAT solver); the major version in the import path (`/v2`); no install scripts; an optional proxy and checksum database (`sum.golang.org`) | Relies on Google-run services for availability and consistency |
| **Cargo** | Excellent ergonomics: lockfiles, semver, workspaces, features | `build.rs` and proc macros run arbitrary code; crates.io is a central point of control |
| **Nix / Guix** | Fully reproducible, content-addressed builds | Steep learning curve |
| **Deno** | Runtime permissions (`--allow-net`, `--allow-read`, …) | Per process, not per dependency |
| **Unison** | Code identified by hash, so versions never conflict | Hard to reconcile with files, git and editors |
| **cargo-vet / cargo-crev** | Recorded, shareable audits of dependency versions | Opt-in; outside the compiler |
| **cargo-semver-checks** | Detects breaking API changes before release | Opt-in lint |
| **pnpm `minimumReleaseAge`, uv `exclude-newer`** | A release cooldown: ignore versions newer than a cut-off | One fixed delay for everyone |
| **Sigstore, SLSA, trusted publishing** | Signing, provenance, and publishing from CI without long-lived tokens | Need central services |

## 4. Decisions

### 4.1 Hosting: plain git

- A mod is a directory in a git repository. **There is no central service**: no registry, mirror,
  checksum log or index. Nothing owns names, and nothing has to be up for a build to work.
- Fetching a version is a shallow fetch of the tagged tree, not a clone of the history.
- **Private mods** use ordinary git credentials; there's nothing extra to configure.
- **Repositories that disappear** (deleted, renamed, force-pushed) are handled by vendoring
  (§4.5). Teams that want a cache run an ordinary git mirror of their own.

### 4.2 Names are URLs

- A mod's name is where it lives: `github.com/jimme/hashiverse-protocol`, `git.example.com/team/ledger`.
  Names are unambiguous, and no naming authority is needed.
- **Vanity domains** separate the name from the host: `moddil.dev/net` serves an HTTPS `<meta>`
  redirect to the real repository (as Go's `go-import`), so the owner can move hosts without renaming
  the mod.
- In these specs, `moddil/…` is shorthand for the standard mods under the project's own domain
  (illustration only).

### 4.3 Versions and majors

- **A version is a git tag**: `v1.4.2`, or `protocol/v1.4.2` for one of several mods in the same
  repository.
- **The major is part of the path**: `github.com/x/y` is v0/v1, and `github.com/x/y/v2` is a different
  mod. A build contains **at most one version per major** of each mod.
- **Minimal version selection** within a major: each mod states the minimum version it needs, and the
  build uses the highest of those minimums. It is deterministic, needs no solver, and never picks a
  version nobody asked for.

**Why this doesn't make upgrading harder:**
- **Within a major, upgrades are free.** Any mod can raise the minimum, and semver (checked, §4.4)
  guarantees nothing breaks.
- **Across majors, migration is gradual.** `x` and `x/v2` are different mods, so library A can stay on
  v1 while library B moves to v2, and both build together. The ecosystem never has to upgrade in
  lockstep.
- **The real cost**: if A exposes v1's `Foo` and B takes v2's `Foo`, they are different types and need
  a conversion. Cargo has the same problem. The fix is the "semver trick": v1 re-exports v2's
  unchanged types, so they *are* the same type.
- **Migration is mechanical.** Renames and moves are inferred from the two versions
  ([`semantic-vcs.md`](semantic-vcs.md)), and libraries can ship replacement rules
  ([`evolution.md`](evolution.md) §3), so `moddil migrate` and `moddil fix` can upgrade consumers from
  v1 to v2.

### 4.4 Semver is checked by the compiler

- `moddil publish` compares the new version's public API, effects, error sets and capabilities with
  the previous tag. It **refuses a breaking change without a major bump**, then creates a signed tag.
- Anyone can create a tag by hand, so **`moddil update` re-runs the check** on the consumer's side:
  "`1.5.0` removes `Parser.reset`, which breaks `1.4`; treat as a major?".
- Within a major, the semver promise is therefore a guarantee rather than a hope, which is what makes
  one-version-per-major painless.

### 4.5 Lockfile and vendoring

- The lockfile records, per mod: the version, the commit, a **content hash** of the normalised tree,
  and the key that signed the tag.
- **Trust on first use.** The hash, not the tag or git's SHA-1, is what is trusted:
  - a re-fetch with a different hash is a hard error (a re-tagged release is detected);
  - a change of signing key is flagged, as with an SSH host-key change.
- **Vendoring is built in**: `moddil vendor` copies the locked sources into the repository, so builds
  survive deleted repositories and work offline.

```
# moddil.lock (illustration only)
github.com/jimme/hashiverse-protocol  v1.4.2  commit 3f9c1e…  tree sha256:9b1d…  key ssh-ed25519:AAAC…
moddil.dev/net                        v0.3.0  commit a71b02…  tree sha256:44e0…  key ssh-ed25519:AAAD…
```

### 4.6 Discovering new versions

`moddil outdated` runs `git ls-remote --tags` for every dependency in parallel (one round trip each,
with no clone) and reports:

```
github.com/jimme/hashiverse-protocol  v1.4.2 → v1.5.0   eligible
moddil.dev/net                        v0.3.0 → v0.3.1   cooldown: 9 days left
github.com/acme/json                  v2.1.0 → v2.2.0   eligible   capability diff: + net   ⚠
github.com/acme/json                  v2 → v3 available (major)
```

### 4.7 Randomised cooldown

- A new version is not eligible for `add` or `update` until it is **7–28 days** old.
- The delay is `7 + hash(project, dependency, version) mod 22` days:
  - it **differs per project, dependency and version**, so each project is an early adopter of some
    releases and a late adopter of others. Adoption is staggered like a canary rollout: a malicious
    release is noticed by early adopters before most projects take it, instead of everyone taking it
    on day 7;
  - it is **deterministic**, so developers and CI agree, and an attacker cannot make anyone's delay
    shorter.
- **Overrides are explicit**, per mod or per version, e.g. to take a security fix immediately. They
  are recorded in the manifest, so they show up in review.
- **Transitive interaction**: if a new release of A requires B ≥ 1.5 and B 1.5 is still in its
  cooldown, `update` holds A back and says why.
- How a release's age is established without a central service is open (§7).

### 4.8 Capability-scoped dependencies

Two layers, both enforced by the compiler (overview §2.1):
- **A ceiling per mod**: the manifest declares the capabilities the mod may use (`net`, `fs`, `env`,
  `spawn`, `time`, `ffi`, `unsafe`, …). A JSON parser that uses `net` fails to build.
- **Authority flows as values** (object capabilities): a library only gets the capabilities its
  caller passes in. The ceiling bounds what a mod *could* ask for; values decide what it actually
  gets.
- **Capability diffs on upgrade**: "`acme/json` 2.1 → 2.2 now requests `net`" is shown by `outdated`
  and `update`, and needs an explicit acceptance.
- **FFI is a capability.** C code can do anything, so `ffi` and `unsafe` are declared, visible to
  every consumer, and treated as high risk by audits ([`interop.md`](interop.md) §5).

```toml
# manifest (illustration only)
[mod]
name = "github.com/acme/json/v2"
version = "2.2.0"

[capabilities]
requires = []                     # pure: no net, fs, ffi

[dependencies]
"moddil.dev/text" = "0.4.1"
```

### 4.9 No code at fetch or build time

- Fetching only downloads and checks hashes. Nothing runs.
- Build logic is the declarative manifest plus **pure, sandboxed comptime**
  ([`comptime.md`](comptime.md) §10), which replaces proc macros and code generators.
- C dependencies are compiled by the bundled clang from declared inputs
  ([`interop.md`](interop.md) §3), so there is no need for `build.rs`.

### 4.10 The source is the release

- A version **is** its tagged tree. There are no tarballs or uploaded artefacts, so nothing can differ
  from the repository, which is exactly what `xz-utils` exploited.
- Prebuilt artefacts come only from caches the user chooses to trust
  ([`build-times.md`](build-times.md) §4.2). Builds are byte-identical on any host
  ([`targets.md`](targets.md) §8), so anyone can rebuild a tagged version and compare.

### 4.11 Audit trail in git

- **Reviews live in the consumer's repository** (cargo-vet style): "`acme/json` v2.2.0 reviewed for
  `safe-to-run`, by whom, and whether by a human or an agent".
- **Delta reviews**: reviewing 2.1 → 2.2 is enough when 2.1 is already trusted. The semantic diff
  focuses the review on API, effect and capability changes ([`agent-interface.md`](agent-interface.md)
  §5.7).
- **Shared audits**: an organisation can import another's audit records from that organisation's git
  repository.
- `moddil vet` fails CI when a locked version has no review that satisfies the project's policy.

### 4.12 A broad standard library

A broad set of `moddil/` mods reduces trivial third-party dependencies ([`mods.md`](mods.md) §6). How
broad is still open there (§8).

### 4.13 Local repositories and working copies

Git fetches from local paths and `file://` URLs as readily as from remote hosts. Moddil uses that
for three different needs, without weakening hash pinning or the rule that names are URLs.

**1. A local repository as the source of a tagged version** (reproducible):

```toml
[dependencies]
"github.com/jimme/hashiverse-protocol" = { version = "1.4.2", git = "../hashiverse" }   # or file:///D:/Repos/hashiverse
```

- Fetching is an ordinary git fetch of the tag from the local repository. The lockfile pins the
  commit and content hash exactly as for a remote source.
- The mod's **identity stays its URL name**, so anyone else resolves the same lockfile from the
  remote.
- Uses: offline work, sibling repositories, air-gapped builds.
- `moddil publish` refuses a mod whose manifest contains machine-specific paths: published mods name
  their dependencies by URL only.

**2. Source overrides in user or CI configuration** (never in the manifest): map URLs to other git
locations, such as local clones or a company mirror, like git's `url.insteadOf` or Go's `GOPROXY`:

```toml
# ~/.config/moddil/sources.toml (illustration only)
"github.com/jimme/*" = "file:///D:/Repos/*"
"github.com/acme/*"  = "https://git.internal/mirror/acme/*"
```

The content hash must still match the lockfile, so an override changes *where* the bytes come from,
never *what* they are.

**3. Live working copies during development** (unreleased, uncommitted edits): a workspace file,
`moddil.work`, like Go's `go.work` or Cargo's `[patch]` with a path. It is kept out of git by
default.

```toml
# moddil.work (illustration only)
use = ["../hashiverse/protocol", "."]
```

- The listed directories replace the corresponding mods with their working trees, uncommitted edits
  included. They rebuild incrementally and live-update in debug builds
  ([`build-times.md`](build-times.md) §4.1).
- **Visible and contained:** builds report "workspace overrides active"; the lockfile is not
  rewritten from them; `moddil publish` and `moddil vet` refuse while they are active; `moddil diff`
  lists them ([`semantic-vcs.md`](semantic-vcs.md)).
- Capability ceilings (§4.8) still apply to overridden mods.

**Monorepos:** several mods in one repository refer to each other by relative path inside that
repository and act as one workspace; releases use path-prefixed tags (`protocol/v1.4.2`, §4.3).

**Security:** local paths and source overrides never bypass hash checking for tagged versions.
Working-copy overrides are unpinned by nature, so they are for development only and always reported.

## 5. Fit check against the reference examples

| Example | Uses |
|---|---|
| [Multiplayer game](examples/01-multiplayer-game.md) | Capability ceilings keep asset and maths libraries pure; vendoring for reproducible console builds |
| [HFT](examples/02-hft-platform.md) | Private mods over internal git; cooldown plus a strict `vet` policy; no `net` in strategy libraries |
| [Web store](examples/03-web-store.md) | Capability diffs on upgrades of payment and auth mods; shared audits across teams |
| [Hashiverse](examples/04-hashiverse.md) | `hashiverse/protocol` published from its own repository; no central service, in keeping with a decentralised project |

## 6. Directions (not yet decided)

- **Tag signing**: SSH or GPG keys pinned in the lockfile (no service needed), or Sigstore keyless
  signing as an optional extra.
- **Advisory feeds**: user-configured feeds (e.g. OSV) as optional inputs to `outdated` and `audit`.
  They are the user's choice, never required to build.
- **Rebuild attestations**: anyone can publish "I rebuilt v1.4.2 and got these bytes" as signed
  records in their own git repository.

## 7. Open questions

- **Trustworthy release dates.** Tag and commit dates are set by the author, so they can be
  backdated to skip the cooldown. Recommended: measure age from the later of the tag date and when
  this project first saw the tag (recorded by `outdated` and `update`). A first `add` still has to
  trust the tag date.
- How are security advisories distributed with no central service?
- How are typosquatting and look-alike URLs caught without a naming authority?
- Pre-1.0: is each `0.x` minor a major (Cargo), or does `v0` promise nothing (Go)?
- Do standard mods ship with the toolchain, or are they fetched like any other mod
  ([`mods.md`](mods.md) §8)?

## Prior art

Go modules and Cox's "Our Software Dependency Problem" (2019) and "Minimal Version Selection" (2018);
Cargo; `cargo-vet`, `cargo-crev`, `cargo-semver-checks`; pnpm `minimumReleaseAge`; uv
`exclude-newer`; Nix and Guix; Deno permissions; Unison; Sigstore; SLSA; analysis of the `xz-utils`
backdoor (CVE-2024-3094); Miller, *Robust Composition* (object capabilities, 2006); Austral.
