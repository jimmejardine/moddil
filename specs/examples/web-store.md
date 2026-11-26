# Web Store

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

- **Compiler-generated client↔API contracts ([Distributed and multi-tier programs](../distribution.md))**: no hand-written OpenAPI. The API is typed once
  and used from all three clients.
- **Version compatibility ([Diffs, code review and version control](../semantic-vcs.md), [Distributed and multi-tier programs](../distribution.md))**: mobile clients can be months old. The compiler checks
  that a new `api` still serves every version recorded in `contracts.lock`, and flags breaking
  changes in the PR's semantic diff. Old versions leave only through an explicit
  `moddil contracts prune` ([`data.md`: One contract system, in space and time](../data.md#one-contract-system-in-space-and-time)).
- **Trust boundaries ([Distributed and multi-tier programs](../distribution.md))**: prices, totals and stock levels from `client` are untrusted; secrets
  (payment keys, DB credentials) cannot flow to `client`, and the type system enforces it.
- **Capabilities ([Package management and supply-chain security](../packages-and-supply-chain.md))**: only the checkout module holds the `Payments` capability; only `api` holds
  `Db`. A dependency that suddenly asks for `Net` is flagged.
- **Typed SQL and migrations**: `sql"…"` is a tagged literal from a mod, checked against
  `schema.sql` at compile time (as sqlx and jOOQ check queries); migrations are versioned and
  reviewable; transactions are linear resources that must commit or roll back
  ([`data.md`: Databases: migrations-first](../data.md#databases-migrations-first)).
- **The database is not a placement**: it is an external service, provisioned outside Moddil and
  bound as a requirement, with its typed schema in Moddil
  ([`distribution.md`: Infrastructure: bind, never provision](../distribution.md#infrastructure-bind-never-provision)).
- **Error handling ([Error handling](../error-handling.md))**: clear, typed errors from DB to HTTP response to UI.
- **LLM-friendliness ([LLMs as first-class programmers](../agent-interface.md))**: most store code is CRUD that agents will write. It should be easy to
  generate, review and test, with contracts and property tests next to the code.
- **Build times ([Build times](../build-times.md))**: a typical product team iterates many times a day.
- **Mobile**: see below.

## The mobile question

The web client is covered by WASM (R1). Android and iOS raise new requirements:

- **iOS forbids JIT compilation** in apps, so Moddil must compile **ahead of time** to native ARM64
  (or ship WASM with an interpreter, which is too slow for real apps).
- **Android** can run native code via the NDK, or WASM in a WebView.
- **UI** is shared logic under native UI shells. Moddil compiles models, networking, validation
  and state management for all three platforms; the UI is SwiftUI (iOS), Jetpack Compose (Android)
  and web, calling Moddil through generated bindings. This is the Kotlin Multiplatform / Rust-core
  approach.

Moddil never owns UI. `moddil bind` generates the Swift, Kotlin and TypeScript bindings, and the
toolchain packages the core as an XCFramework and an AAR ([`ui.md`](../ui.md)). A Moddil UI
framework, or a web client in a native shell, is left to community mods.

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
  `moddil/test`, `moddil/prop`.
- New: `store/catalog`, `store/cart`, `store/orders`, `store/payments` (wrapping a provider's API).

## Interop and non-Moddil parts

- PostgreSQL (wire protocol in a mod, or a C client library via FFI).
- Payment provider, email and search services (HTTP APIs).
- iOS/Android platform APIs (camera, notifications, payments) through generated Swift/Kotlin bindings.

## Alternatives considered

- **A Moddil UI framework (`moddil/ui`)**: one declarative UI rendering natively or through its own
  renderer on all three platforms (the Flutter / Compose Multiplatform approach). Not taken: Moddil
  never owns UI; it is left to community mods.
- **Web everywhere**: the web client wrapped in a native shell (Capacitor/Tauri-style). Not taken
  for the same reason; it is left to community mods.
