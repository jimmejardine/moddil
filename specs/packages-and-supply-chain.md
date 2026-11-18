# Packages and Supply-Chain Security

> **Status:** brainstorm. Expands [`rationale.md`](rationale.md) §2.16. Nothing here is decided.

## 1. Problem

Package managers made code reuse trivially easy, and in doing so made the dependency
graph the largest attack surface in modern software.

## 2. State of the art

- **Cargo**: lockfiles, semver, workspaces, features, `cargo-vet`/`cargo-crev` for audit trails.
  The ergonomics are excellent.
- **Go modules**: minimal version selection (deterministic, no SAT solver), a public checksum database
  (`sum.golang.org`), a module proxy, and **no install scripts**.
- **Nix / Guix**: fully reproducible, content-addressed builds.
- **Deno**: runtime permissions (`--allow-net`, `--allow-read`, …).
- **Unison**: code identified by hash, so there is no "dependency hell" between versions because
  definitions are immutable.
- **Sigstore / SLSA / in-toto**: signing and provenance attestations.
- **pnpm / Yarn PnP**: content-addressed stores and strictness about phantom dependencies.

## 3. What still hurts

- **Build-time code execution**: npm `postinstall`, Python `setup.py`, Rust `build.rs` and proc macros
  all run arbitrary code on the developer's machine at install or build time.
- **Ambient authority**: any dependency, however deep, can open sockets, read `~/.ssh`, or spawn
  processes. A left-pad library has the same power as your web server.
- Typosquatting, account takeover, maintainer burnout, and long-con attacks (the `xz-utils` backdoor,
  `event-stream`, `ua-parser-js`).
- Transitive dependency counts in the hundreds or thousands; nobody reviews them.
- Binary artefacts in packages that don't match the source.

## 4. Directions

- **Capability-scoped dependencies.** A package declares the capabilities it needs (`net`, `fs`,
  `env`, `ffi`, `unsafe`, `spawn`, `time`, …), and **the compiler enforces it**: a JSON parser that
  imports `net` fails to build. Capabilities are passed as values (object-capability style), so a
  library only gets what its caller hands it. (Prior art: E, Austral, WASI, Deno.)
- **Capability diffs on upgrade**: "`foo` 1.4 → 1.5 now requests `net`" is a red flag shown by the tool.
- **No arbitrary code at install time.** Build logic is declarative or runs in a WASM sandbox
  (R1, rationale §5.1) with no network or file access beyond its inputs. Comptime code, which replaces proc
  macros, is pure and sandboxed ([`comptime.md`](comptime.md) §10).
- **Reproducible builds by default**; packages are published from source and the registry verifies
  the build.
- **Hash-pinned lockfiles, minimal version selection, transparency log.**
- **Audit trail built in** (cargo-vet style): "who reviewed this version, and was it a human or an
  agent?"
- **Small, broad standard library** to reduce trivial dependencies.

## 5. Open questions

- How to handle capabilities through FFI (C code can do anything)? Probably: FFI *is* a capability
  (`ffi`/`unsafe`) and auditors treat it as high risk.
- Content-addressed definitions (Unison) vs versioned packages, or both?
- Central registry vs decentralised (Go's URLs plus proxy)?

## Prior art

Go modules and Cox's "Our Software Dependency Problem" (2019); SLSA; Sigstore; `cargo-vet`;
analysis of the `xz-utils` backdoor (CVE-2024-3094); Miller, *Robust Composition* (object
capabilities, 2006); Austral language.
