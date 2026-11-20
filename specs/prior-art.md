# Prior Art: Matrix and References

> **Status:** reference material. The prior-art matrix and the reference list. Each spec also has its own
> prior-art list.

## Prior-art matrix

Legend: ✓ strong · ◐ partial · ✗ weak/absent. These are rough judgements, intended as a starting
point for debate.

| Language | CPU concurrency | GPU | Memory / resources | Build times | Packages / supply chain | LLM-friendliness | Semantic diffs | C/C++/Rust interop | Distributed / multi-tier |
|---|---|---|---|---|---|---|---|---|---|
| C | ✗ manual | ◐ via CUDA C/OpenCL | ✗ manual | ◐ fast-ish, headers | ✗ none standard | ◐ simple but unsafe | ✗ | ✓ *is* the ABI | ✗ |
| C++ | ◐ libraries, no race safety | ✓ CUDA/SYCL/HIP | ◐ RAII unenforced | ✗ slow | ✗ fragmented (vcpkg/Conan) | ✗ huge, non-local semantics | ✗ | ✓ C / ◐ Rust | ✗ |
| Rust | ✓ Send/Sync; ◐ async colouring | ◐ emerging | ✓ ownership; ◐ learning curve | ✗ slow | ✓ Cargo; ◐ build.rs/proc macros | ✓ great diagnostics | ✗ | ✓ C / ◐ C++ | ◐ WASM client + server, separate builds |
| Go | ✓ goroutines; ✗ race safety | ✗ | ◐ low-pause GC | ✓ fast | ✓ MVS, checksum DB | ✓ small, one format | ✗ | ◐ cgo overhead | ◐ Service Weaver |
| Zig | ◐ manual | ◐ SPIR-V/PTX targets emerging | ◐ explicit allocators, no safety proof | ✓ fast, incremental | ◐ young | ◐ explicit, little magic | ✗ | ✓ best C story | ✗ |
| Swift | ✓ structured concurrency, actors | ◐ Metal (separate language) | ◐ ARC, atomic costs | ◐ | ◐ SwiftPM | ◐ | ✗ | ✓ C / ✓ C++ | ◐ distributed actors |
| Mojo | ◐ | ✓ MLIR-based targeting | ◐ ownership | ? | ◐ young | ◐ Python-like | ✗ | ◐ Python, C | ✗ |
| Odin | ◐ | ✗ | ◐ allocators, arenas, implicit context | ✓ very fast | ◐ vendored collection, no package manager | ◐ | ✗ | ✓ C | ✗ |
| Jai | ◐ | ✗ | ◐ context allocator, temporary storage | ✓ very fast; the build is a metaprogram | ✗ closed beta | ◐ | ✗ | ✓ C | ✗ |
| Erlang / Elixir | ✓ processes, supervisors | ✗ | ✓ per-process GC | ◐ | ✓ Hex | ◐ | ✗ | ◐ NIFs are risky | ✓ native distribution; ◐ trusted clusters only |
| Haskell / OCaml / Koka | ◐ / ✓ OCaml 5 effects | ◐ Futhark-like DSLs | ◐ GC; ✓ Perceus (Koka) | ◐ | ◐ | ◐ strong types, steep learning | ✗ | ◐ | ◐ Eliom (OCaml), HasChor (Haskell) |
| Unison | ◐ abilities | ✗ | ◐ GC | ✓ cache by hash | ✓ content-addressed | ◐ | ✓ by design | ✗ | ✓ ships code by hash |
| Carbon | ? | ? | ◐ planned safety | ? | ? | ? | ✗ | ✓ C++ (goal) | ✗ |
| Vale | ◐ | ✗ | ✓ generational refs | ? | ? | ◐ | ✗ | ◐ | ✗ |
| Pony | ✓ ref capabilities | ✗ | ✓ ORCA GC | ◐ | ◐ | ✗ niche | ✗ | ◐ C | ✗ |
| Ballerina / Winglang | ◐ | ✗ | ◐ GC | ◐ | ◐ | ◐ | ✗ | ✗ | ✓ network/cloud-aware by design |

**Observation:** no row is strong across the board, and **no language at all** is strong in "semantic
diffs" or treats "LLM-friendliness" as a stated design goal. Distributed programming is well served
only in niches (Erlang, Unison, Ballerina/Wing, research multitier and choreographic languages), and
none of those combine it with systems-level performance or GPU support. Those three columns, together
with effects and capabilities as a unifying mechanism, are where the opportunity is largest.

## References

*To be expanded with links during research.*

- **Concurrency:** Nystrom, "What Color is Your Function?" (2015); N. J. Smith, "Notes on structured
  concurrency, or: Go statement considered harmful" (2018); Pony reference capabilities and the ORCA
  GC; Java JEP 444 (virtual threads) and JEP 453/480 (structured concurrency); Rust
  `std::thread::scope` and rayon.
- **GPU:** Ragan-Kelley et al., *Halide* (PLDI 2013); Tillet et al., *Triton* (MAPL 2019); Henriksen
  et al., *Futhark* (PLDI 2017); MLIR (Lattner et al., CGO 2021); SYCL specification.
- **Memory:** Reinking et al., *Perceus: Garbage Free Reference Counting with Reuse* (PLDI 2021);
  Vale generational references; Hylo (Val) mutable value semantics and parameter conventions; Mojo
  argument conventions and origins; Swift non-escaping closures; Rust Polonius; Choi, Shull,
  Torrellas, *Biased Reference Counting* (PACT 2018) and CPython PEP 703; Austral and Linear
  Haskell (linear types); Rust `bytes::Bytes` and `yoke`; FlatBuffers, Cap'n Proto, SBE.
- **Build systems:** Mokhov, Mitchell, Peyton Jones, *Build Systems à la Carte* (ICFP 2018); Salsa;
  Bazel / Buck2.
- **Supply chain:** Go modules and Cox's "Our Software Dependency Problem" (2019); SLSA; Sigstore;
  `cargo-vet`; analysis of the `xz-utils` backdoor (CVE-2024-3094); Miller, *Robust Composition*
  (object capabilities, 2006); Austral language.
- **Effects:** Leijen, *Koka* papers; Effekt language; OCaml 5 effect handlers; Unison abilities.
- **Version control:** Pijul theory of patches; Jujutsu; Difftastic; GumTree (Falleri et al., 2014);
  Unison codebase model.
- **Verification:** Dafny; Verus; SPARK; Liquid Haskell; Kani.
- **Null and invalid states:** Hoare, "Null References: The Billion Dollar Mistake" (QCon 2009);
  Swift `Optional`; Kotlin null safety; TypeScript `strictNullChecks`; Dart sound null safety; C# 8
  nullable reference types; Java Project Valhalla null-restricted types and JSpecify; Rust niche
  optimisation and pattern types; F* and Liquid Haskell refinement types; typestate (Strom and
  Yemini, 1986).
- **Metaprogramming:** Zig `comptime`; Rust proc macros, `syn`, `serde`, `sqlx`; C++26 static
  reflection (P2996); D CTFE; Jai `#run`/`#insert`; Nim macros; Swift macros; Scala 3 inline and
  quotes; JavaScript tagged templates.
- **Interop:** Zig `@cImport` / `zig cc`; Swift C++ interoperability; Carbon language goals; `cxx`;
  `crubit`; Rust `crabi` proposal; Mozilla UniFFI; ICU4X Diplomat; `wit-bindgen`; RLBox.
- **Determinism:** FoundationDB simulation testing; TigerBeetle VOPR; `rr`.
- **Cross-compilation:** Zig (`zig cc`, bundled libcs); Go (`GOOS`/`GOARCH`); `cargo-zigbuild`;
  `cargo-xwin`; osxcross; musl; mingw-w64; rcodesign (apple-codesign).
- **Observability:** OpenTelemetry; W3C Trace Context; Rust `tracing`; Java Flight Recorder; BEAM
  observer and recon; defmt; Yang et al., *NanoLog* (USENIX ATC 2018); Pernosco; WinDbg TTD;
  tokio-console.
- **WebAssembly:** Haas et al., *Bringing the Web up to Speed with WebAssembly* (PLDI 2017); WASI and
  the Component Model / WIT (Bytecode Alliance); stack-switching and JSPI proposals; WASM GC
  proposal; RLBox (sandboxing C libraries via WASM in Firefox).
- **Distribution:** Waldo, Wyant, Wollrath, Kendall, *A Note on Distributed Computing* (1994);
  Deutsch, "Fallacies of distributed computing"; Weisenburger, Wirth, Salvaneschi, *A Survey of
  Multitier Programming* (ACM Computing Surveys, 2020); Montesi, *Choreographic Programming*
  (PhD thesis, 2013) and Choral; HasChor; Links; Ur/Web; Eliom; ScalaLoci; Service Weaver
  ("Towards Modern Development of Cloud Applications", HotOS 2023); Orleans; Winglang; Ballerina;
  Temporal; Restate; Hydro (Berkeley); Go `ko` and Jib (images without Dockerfiles); Kubernetes,
  Nomad, Consul, Vault, Terraform/OpenTofu.
