# Targets, Cross-compilation and Static Builds

> **Status:** complete: decided, with no open questions. Indexed in [`README.md`](README.md).
> Commands, target names and file formats are **illustration only**.

## Summary

| Rule | One line |
|---|---|
| **Every toolchain is a cross-compiler** | One download builds for every tier-1 target from any host, with no extra setup: Linux from Windows, macOS from Linux, Windows from macOS. |
| **Static by default** | Static except the OS's own mandatory surface: zero third-party shared libraries. |
| **No libc where the OS allows** | Raw system calls on Linux, Win32 on Windows, `libSystem` on macOS, WASI on WASM. libc only when C code is linked. |
| **Dynamic glibc only on request** | Opt-in targets with a pinned minimum glibc version, for programs that must load system drivers (CUDA, Vulkan) or NSS modules. |
| **Byte-identical output on any host** | The same inputs produce the same bytes whatever OS builds them: shared caches and verifiable releases. |
| **Sign and package from any host** | Authenticode for Windows; ad-hoc, Developer ID and notarisation for macOS; OCI images; archives. |
| **Run what you cross-built** | WSL2, QEMU user mode and Wasmtime locally; remote runners for macOS. |
| **Bare metal is separate** | Kernels and firmware use the freestanding profile; a bare-metal target is tier 1 only if an emulator runs its tests in CI ([`freestanding.md`: Verifying the platform in emulators](freestanding.md#verifying-the-platform-in-emulators)). |

## State of the art

**Problem.** Shared libraries make deployment fragile: the right versions must be installed on every
machine ("dependency hell"), and building for another OS usually needs that OS, a VM, Docker images
or hand-assembled sysroots.

| Tool | What it does well | Limits |
|---|---|---|
| **Zig** (`zig cc`, `cargo-zigbuild`) | The gold standard. One download cross-compiles C, C++ and Zig to Linux, Windows, macOS and WASM. It bundles musl, mingw-w64, glibc stubs **per glibc version** (e.g. `x86_64-linux-gnu.2.28`) and macOS headers and link stubs. | Shipping Apple-derived stubs is a licensing grey area |
| **Go** | `GOOS=linux GOARCH=arm64 go build` just works. On Linux it uses raw system calls, so binaries are fully static with no libc. | Using C (cgo) breaks the magic; on macOS Go must link `libSystem` |
| **Rust** | `rustup target add` covers many targets; musl gives static Linux binaries | Each target still needs a linker and a C sysroot; people use `cross` (Docker), `cargo-zigbuild` or `cargo-xwin` |
| **Nix** | Reproducible cross builds of whole dependency trees | Steep learning curve; Linux and macOS hosts only |
| **rcodesign** (apple-codesign) | Signs and notarises macOS binaries from Linux or Windows | Still needs an Apple Developer account |

## What "static" means on each OS

| Target | Fully static? | Mandatory dynamic surface |
|---|---|---|
| **Linux** | **Yes.** The system-call interface is stable, so Moddil needs no libc at all (as Go) | GPU drivers (`libcuda`, the Vulkan loader), which are tied to the installed kernel driver ([Dynamic glibc targets](#dynamic-glibc-targets)) |
| **Windows** | Effectively yes. No C runtime is needed | System DLLs (`kernel32`, `ntdll`, `ws2_32`), always present |
| **macOS** | **No.** Apple does not support a static `libSystem`, and the system-call interface is not stable | `libSystem.dylib`, always present |
| **WASM** | Inherently | Imports only (R1) |

**The rule:** *static except the OS's own mandatory surface.* A Moddil binary never depends on a
third-party shared library being installed, which removes the "dependency hell" of distribution
packages, version mismatches and `LD_LIBRARY_PATH`.

## Every toolchain is a cross-compiler

One toolchain download contains everything needed for every tier-1 target:
- LLVM for optimised builds, and Cranelift for fast development builds ([`build-times.md`](build-times.md));
- `lld`, which writes all four binary formats (ELF, Mach-O, COFF, WASM);
- musl and mingw-w64 sources, glibc stubs per supported version, and minimal macOS headers and link
  stubs;
- the clang component that compiles C dependencies, fetched on first use ([`interop.md`: The C ABI, both directions](interop.md#the-c-abi-both-directions)).

This matches the backend plan in [`compiler.md`: Backends](compiler.md#backends).

```
# on Windows
moddil build --target linux-x86_64          # static Linux binary
moddil build --target macos-universal        # x86-64 + ARM Mach-O, ad-hoc signed

# on Linux
moddil build --target windows-x86_64         # PE/COFF executable
moddil build --target wasm32-wasi            # WASI component
```

## Target names and tiers

Targets are named `os-arch[-abi][.version]`:

| Example | Meaning |
|---|---|
| `linux-x86_64`, `linux-aarch64` | Static Linux, no libc |
| `linux-x86_64-gnu.2.28` | Dynamic glibc, runs on glibc ≥ 2.28 ([Dynamic glibc targets](#dynamic-glibc-targets)) |
| `windows-x86_64`, `windows-aarch64` | Windows, system DLLs only |
| `macos-aarch64`, `macos-x86_64`, `macos-universal` | macOS; universal = both architectures in one file |
| `wasm32-wasi`, `wasm32-browser` | WASM (R1) |

- **CPU feature levels** are part of the target (e.g. `linux-x86_64+v3` for AVX2). **Defaults
  (decided):** x86-64-v2 (SSE4.2, POPCNT); armv8.0 for Linux and Windows on aarch64; apple-m1 for
  macOS on aarch64; SIMD128 for WASM. Hot functions can carry several versions selected at start-up
  (`#[multiversion(v3, v4)]`), and a `+v3` target is an explicit choice.
- **Tier 1** targets are built and tested in CI on every change: Linux (x86-64, ARM64), Windows
  (x86-64), macOS (ARM64, x86-64), WASM (WASI and browser, R1).
- **Tier 2** targets build but are tested less often (e.g. Windows on ARM, FreeBSD, RISC-V).
- **Bare-metal tiers (decided):** tier 1 is `thumbv7em` (Cortex-M4F, on Renode) and `riscv64` QEMU
  `virt`; tier 2 is Cortex-M0+ and M33, x86_64 bare metal and AArch64 `virt`; others are tier 3
  ([`freestanding.md`](freestanding.md)).
- **A published target-tier policy (decided)** sets the bar, and the tier-2 list grows from
  contributions: tier 1 needs CI hardware, two maintainers and the full test suite passing on every
  change. Promotion is a Moddil Evolution proposal ([`governance.md`: Moddil Evolution](governance.md#moddil-evolution)).

## The libc strategy

- Moddil's standard library has its **own system layer per OS**: raw system calls on Linux, Win32 on
  Windows, `libSystem` on macOS, WASI imports on WASM. Pure Moddil programs therefore have no libc on
  Linux or Windows.
- **DNS** is resolved by Moddil's own resolver (reading `resolv.conf` and `hosts`), as Go does, so
  static Linux binaries do not need glibc's NSS. An opt-in handler can use the system resolver
  instead ([Dynamic glibc targets](#dynamic-glibc-targets)).
- The memory allocator is built in.
- **When C code is linked** ([`interop.md`](interop.md)), libc comes in: static musl on Linux,
  mingw-w64 on Windows, `libSystem` on macOS.

## Dynamic glibc targets

**Versions (decided):** stubs are generated from one ABI list for every glibc from 2.17, so any
version can be chosen; `-gnu` without a version means 2.28 (RHEL 8, Debian 10), and the default moves
with editions.

Some programs must load libraries that only exist on the machine they run on:
- GPU drivers: CUDA's `libcuda`, the Vulkan loader, OpenGL ([`accelerators.md`](accelerators.md));
- NSS and PAM modules (enterprise logins, LDAP host lookup).

Loading these from a fully static binary is unreliable, so such programs choose a **dynamic glibc
target with a pinned minimum version**, e.g. `linux-x86_64-gnu.2.28`, built against bundled stubs for
that version (Zig's approach). The binary runs on any distribution with glibc 2.28 or newer.
Everything else in it is still linked statically; only glibc and the drivers are dynamic.

## Byte-identical output on any host

Building `linux-x86_64` on Windows produces **the same bytes** as building it on Linux or macOS.
This requires:
- no host paths, timestamps, user names or locale in outputs;
- deterministic ordering in the compiler and linker;
- the same bundled tools and libraries on every host.

Two payoffs:
- build caches are shared across developers on different operating systems
  ([`build-times.md`](build-times.md));
- anyone can rebuild a release and check that it matches ([Package management and supply-chain security](packages-and-supply-chain.md)).

## Signing and packaging from any host

| Platform | What the toolchain does |
|---|---|
| **Windows** | Authenticode signing (avoids SmartScreen warnings) |
| **macOS** | Ad-hoc signature by default (Apple silicon refuses to run unsigned code); Developer ID signing and notarisation for distribution, from any host (rcodesign-style); universal binaries |
| **Linux** | Archives, OCI images built directly ([`distribution.md`: Layer B: artefacts](distribution.md#layer-b-artefacts)); distribution packages through adapters |
| **All** | Release archives with SBOM and provenance ([Package management and supply-chain security](packages-and-supply-chain.md)) |

Signing keys and Apple credentials are **capabilities supplied at release time**, never stored in the
build or the repository.

- **macOS headers (decided):** the toolchain bundles only headers derived from Apple's open-source
  releases (APSL) and generated `.tbd` link stubs, as Zig does, with a legal review before v1.
  Frameworks beyond `libSystem` (AppKit, Metal) need the user's SDK, found automatically on a Mac or
  given by path elsewhere.
- **Notarisation without a Mac (decided):** `moddil release` signs and notarises through the App
  Store Connect API on any host. The Developer ID certificate and the API key come from a
  secret-provider handler at release time.

## Running and testing cross builds

| Host → target | How to run it |
|---|---|
| Windows → Linux | WSL2 |
| Any → another CPU architecture | QEMU user mode |
| Any → WASM | Wasmtime (and browsers for `wasm32-browser`) |
| Non-Apple → macOS | Remote runners (ssh or CI): macOS cannot legally be emulated on non-Apple hardware |

- The test runner treats these as **executors**: `moddil test --target linux-aarch64` builds, then
  runs the tests under QEMU or on a remote runner.
- Most tests are target-independent under the simulation runtime ([`determinism.md`](determinism.md)),
  so only OS-facing code needs real target runs.

## Mobile

- **Android:** an NDK-compatible sysroot, bundled or fetched on first use; Moddil produces shared
  libraries for apps.
- **iOS:** Moddil produces static libraries or frameworks; final app packaging needs Apple's tools.
- This matches the web store's "shared logic, native UI shells" approach
  ([`examples/03-web-store.md`](examples/03-web-store.md)). **Decided scope:** Moddil builds the
  XCFramework and AAR plus generated bindings; Xcode and Gradle build and sign the app
  ([`ui.md`: Mobile packaging](ui.md#mobile-packaging)).

## Fit check against the reference examples

| Example | Uses |
|---|---|
| [Multiplayer game](examples/01-multiplayer-game.md) | WASM client and a static Linux server, built from any developer machine |
| [HFT](examples/02-hft-platform.md) | Static Linux binaries pinned to the servers' CPU level (`+v3`/`+v4`); research GPU boxes on a dynamic glibc target for CUDA |
| [Web store](examples/03-web-store.md) | OCI images built on any host; shared-logic libraries for Android and iOS |
| [Hashiverse](examples/04-hashiverse.md) | One CI host producing static volunteer binaries for Linux, Windows and macOS, signed for each |

## Open questions

None at present.


## Prior art

Zig (`zig cc`, bundled libcs, glibc version targeting); Go (`GOOS`/`GOARCH`, raw system calls); Rust
`rustup`, `cross`, `cargo-zigbuild`, `cargo-xwin`; Nix cross-compilation; osxcross; musl; mingw-w64;
LLVM `lld`; rcodesign (apple-codesign); osslsigncode and jsign; QEMU; WSL2; Go `ko`.
