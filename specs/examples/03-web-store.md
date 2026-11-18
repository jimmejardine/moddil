# Example 3: Web Store

## Overview

An online store with a GUI frontend on **web, Android and iOS**, and a **stateless** HTTP backend that
talks to a relational database. Architecturally it is the most ordinary of the four examples, and
that is the point: most software looks like this. If Moddil is not pleasant for everyday CRUD, it will
not be adopted, whatever its other strengths.

## Topology

```
 ┌──────────── client  ────────────┐
 │  web (WASM)   Android   iOS      │
 │  catalog, cart, checkout, account│
 └───────────────┬─────────────────┘
                 │ HTTPS (compiler-generated contract)
                 ▼
 ┌──────────────────────────────────┐        ┌────────────────────┐
 │  api   (stateless, N instances)  │◄──────►│ db   (PostgreSQL)  │
 │  auth, catalog, cart, orders     │  SQL   └────────────────────┘
 └───────────────┬──────────────────┘
                 │ payments capability only
                 ▼
         payment provider, email, search (external services)
```

| Placement | Role | Notes |
|---|---|---|
| `client` | UI on web, Android and iOS | Untrusted; offline-tolerant cart |
| `api` | Stateless HTTP API, horizontally scaled behind a load balancer | All state lives in `db` |
| `db` | Relational database | Not Moddil code; typed schema in Moddil |

## Key flows

1. **Browse**: client queries the catalog → `api` reads `db` (with caching) → paginated results.
2. **Cart**: maintained on the client and synchronised to `api` when the user is logged in.
3. **Checkout**: `api` validates the cart against current prices and stock (never trusting client
   prices), creates the order in a transaction, then calls the payment provider.
4. **Account**: sign-up and login, sessions/tokens, order history.
5. **Deploy**: a new `api` version rolls out while old clients (especially mobile apps that update
   slowly) are still in use.

## Moddil features this stresses

- **Compiler-generated client↔API contracts ([overview: Distributed and multi-tier programs](../overview.md#distributed-and-multi-tier-programs))**: no hand-written OpenAPI. The API is typed once
  and used from all three clients.
- **Version compatibility ([overview: Diffs, code review and version control](../overview.md#diffs-code-review-and-version-control), [overview: Distributed and multi-tier programs](../overview.md#distributed-and-multi-tier-programs))**: mobile clients can be months old. The compiler must check
  that a new `api` still serves every client version still in support, and flag breaking changes in
  the PR's semantic diff.
- **Trust boundaries ([overview: Distributed and multi-tier programs](../overview.md#distributed-and-multi-tier-programs))**: prices, totals and stock levels from `client` are untrusted; secrets
  (payment keys, DB credentials) cannot flow to `client`, and the type system enforces it.
- **Capabilities ([overview: Package management and supply-chain security](../overview.md#package-management-and-supply-chain-security))**: only the checkout module holds the `payments` capability; only `api` holds
  `db`. A dependency that suddenly asks for `net` is flagged.
- **Typed SQL and migrations**: queries checked against the schema at compile time (as in sqlx or
  jOOQ); migrations are versioned and reviewable; transactions are linear resources that must commit
  or roll back ([overview: Memory and resource management](../overview.md#memory-and-resource-management)).
- **Error handling ([overview: Error handling](../overview.md#error-handling))**: clear, typed errors from DB to HTTP response to UI.
- **LLM-friendliness ([overview: LLMs as first-class programmers](../overview.md#llms-as-first-class-programmers))**: most store code is CRUD that agents will write. It should be easy to
  generate, review and test, with contracts and property tests next to the code.
- **Build times ([overview: Build times](../overview.md#build-times))**: a typical product team iterates many times a day.
- **Mobile**: see below.

## The mobile question

The web client is covered by WASM (R1). Android and iOS raise new requirements:

- **iOS forbids JIT compilation** in apps, so Moddil must compile **ahead of time** to native ARM64
  (or ship WASM with an interpreter, which is too slow for real apps).
- **Android** can run native code via the NDK, or WASM in a WebView.
- **UI** is the hard part. Options:
  1. **Shared logic, native UI shells.** Moddil compiles models, networking, validation and state
     management for all three platforms; the UI is SwiftUI (iOS), Jetpack Compose (Android) and web,
     calling Moddil through generated bindings. This is the Kotlin Multiplatform / Rust-core approach.
  2. **Moddil UI (`moddil/ui`)**: one declarative UI framework rendering natively or via its own
     renderer on all three platforms (the Flutter / Compose Multiplatform approach).
  3. **Web everywhere**: the web client wrapped in a native shell (Capacitor/Tauri-style).

**Recommendation:** start with option 1 (shared logic plus thin native UI shells). It needs C-ABI
export ([overview: Interoperability with C and Rust](../overview.md#interoperability-with-c-and-rust)) and binding generation for Swift and Kotlin, both of which Moddil needs anyway. A
`moddil/ui` can follow later as a community or standard mod.

## Success criteria

Initial targets, to be refined:
- One API definition used by web, Android and iOS clients, with zero hand-written serialisation.
- A breaking API change fails the build while any supported client version depends on it.
- Typed SQL: a query that does not match the schema fails to compile.
- A secret value cannot be returned to `client`; attempting it is a compile error.
- An agent can add a new CRUD entity end to end (schema, migration, API, client model, tests), and
  the semantic diff summarises it accurately.
- Incremental rebuild after a one-line change in `api`: well under a second.

## Mods used

- Standard: `moddil/net` (HTTP server/client), `moddil/rpc`, `moddil/sql`, `moddil/serde` (JSON +
  binary), `moddil/crypto` (password hashing, tokens), `moddil/decimal` (money), `moddil/text`,
  `moddil/time`, `moddil/web`, `moddil/storage` (client cache), `moddil/log`, `moddil/trace`,
  `moddil/test`, `moddil/prop`, `moddil/ui` (later).
- New: `store/catalog`, `store/cart`, `store/orders`, `store/payments` (wrapping a provider's API).

## Interop and non-Moddil parts

- PostgreSQL (wire protocol in a mod, or a C client library via FFI).
- Payment provider, email and search services (HTTP APIs).
- iOS/Android platform APIs (camera, notifications, payments) through generated Swift/Kotlin bindings.

## Open questions

- Which mobile UI option? (The recommendation above is a starting point.)
- How far back must API compatibility checking go, and how does the compiler know which client
  versions are still in use (deployment metadata)?
- Is typed SQL a language feature (compile-time evaluation against a schema), or purely a mod using
  [overview: Metaprogramming](../overview.md#metaprogramming) reflection?
- Should `db` be a placement at all, or just an external service with a typed schema?
