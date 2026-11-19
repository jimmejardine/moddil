# Data Interchange and Persistence

> **Status:** decided direction, details open. Listed in the [index](README.md). Snippets, file
> names and formats are **illustration only**.

## Summary

| Rule | One line |
|---|---|
| **Types are the schema** | Serde-style derives through comptime reflection; formats are separate mods. |
| **A binary format with locked tags** | Compact and tagged; tags are assigned by the compiler and recorded, never hand-numbered or reused. |
| **One contract system, in space and time** | Network messages, files, stored blobs and queue messages share one set of evolution rules and one compiler check, recorded in `contracts.lock`. |
| **Unknown fields depend on the boundary** | Ignored inside contracts, rejected by default at untyped edges. |
| **Parse, don't validate** | Decoding untrusted bytes produces validated types or a typed `fail(DecodeError)`. |
| **Migrations-first databases** | Versioned SQL migrations are the source of truth; `schema.sql` is derived; `sql"…"` is checked against it; no ORM. |
| **A `Db` effect** | Pools are handlers; transactions are linear values. |

## Problem

- **Schemas drift.** A typical system keeps OpenAPI files, protobuf definitions, SQL schemas and
  JSON shapes in sync by hand, across several languages.
- **Brittle binary formats.** Compact formats such as bincode and postcard break when a field is
  added; protobuf survives, but only because people number and never reuse field tags by hand.
- **Untrusted input** is validated (or not) after it has been decoded into ordinary types, so
  invalid values leak inward.
- **Databases**: ORMs hide the SQL that actually runs; migrations live apart from the code that
  depends on them, and nothing checks that the two agree.
- **Stored data outlives code.** A file or queue message written by version 3 may be read by
  version 7, but evolution checks usually cover only RPC, if anything.

## State of the art

| Topic | Approaches |
|---|---|
| **Serialisation** | **Rust serde**: one derive, a format-agnostic data model, many formats (but proc macros and heavy monomorphisation). **Go**: reflection and struct tags, slow and stringly typed. **Swift `Codable`**: synthesised by the compiler. |
| **Binary formats** | **Protobuf**: tagged, evolvable, unknown fields preserved. **Cap'n Proto, FlatBuffers, SBE**: zero-copy. **CBOR, MessagePack**: self-describing. **postcard, bincode**: compact, brittle. |
| **Evolution checks** | Avro's reader/writer schema resolution; Buf's breaking-change checks for protobuf in CI. |
| **Typed SQL** | **sqlx** (checked against a live database), **jOOQ**, **Kysely**. |
| **ORMs** | EF Core, Django, Prisma, Diesel: code-first models with generated migrations. |
| **Migrations** | **Flyway**, golang-migrate: versioned SQL files. **Atlas**: declarative schema diffing. |

## Decisions

### Types are the schema

```rust
#[derive(Serialize, Deserialize, Debug)]
#[contract]
struct Order {
    id: OrderId,
    #[serde(rename = "px")] price: Fixed<4>,
    qty: Qty,
    note: Option<String>,             // added in v2: optional, so old data still decodes
}
```

- `#[derive(Serialize, Deserialize)]` are comptime functions over static reflection
  ([`comptime.md`](comptime.md)) that produce typed impls of a **format-agnostic data model** (as
  serde does).
- **Formats are mods.** `moddil/serde` ships JSON and the Moddil binary format; CBOR, MessagePack,
  TOML and others are community mods on the same data model.

### A binary format with locked tags

- The standard binary format is **compact and tagged**.
- **Tags are assigned by the compiler** when a type is first released, and recorded in
  `contracts.lock` (see the next section). They are never numbered by hand and never reused; a
  removed field's tag stays reserved.
- **At the edges**, for non-Moddil peers, the compiler generates protobuf/gRPC and OpenAPI
  descriptions, and can consume them to produce typed clients
  ([`distribution.md`: Wire contracts and version skew](distribution.md#wire-contracts-and-version-skew)).

### One contract system, in space and time

- Anything that crosses a boundary **in space** (the network) or **in time** (storage) is a
  **contract**:
  - messages at a `Remote` boundary, which are contracts implicitly
    ([`distribution.md`](distribution.md));
  - files, stored blobs, cache entries and queue messages, marked `#[contract]`.
- All contracts follow the **same evolution rules**: added fields need a default or `Option`;
  removed fields go through deprecation; renames are refactor intents
  ([`semantic-vcs.md`](semantic-vcs.md)).
- **Every recorded version is checked.** Each released version of a contract is recorded in
  **`contracts.lock` in the repository** (there is no registry,
  [`packages-and-supply-chain.md`](packages-and-supply-chain.md)). The compiler rejects a change
  that could not read data written by, or be read by, any recorded version still in use.
- **Pruning is explicit**: `moddil contracts prune` removes versions that are no longer deployed or
  stored, and the removal shows in review.

### Unknown fields depend on the boundary

| Boundary | Unknown fields | Why |
|---|---|---|
| **Inside a contract** (Moddil to Moddil, stored data) | Ignored | An older reader must accept a newer writer's data during rolling deploys, and old code may read newer files |
| **Untyped edges** (public JSON APIs, configuration files) | Rejected by default, with an opt-in to ignore | Catches typos and mismatched clients early |

### Parse, don't validate

- Decoding untrusted bytes produces **validated types**: refinements and newtypes
  ([`types.md`](types.md), [`correctness.md`](correctness.md)). An `Order` with a
  negative quantity never exists.
- Failures are typed: `fail(DecodeError)`, with the path to the bad field (`orders[3].qty`).
- This is the untrusted-data rule of [`distribution.md`: Trust, secrets and capabilities](distribution.md#trust-secrets-and-capabilities)
  applied to every input.

### Zero-copy

Validated data can be read in place, without copying: generated accessor views, owning views
(`Parsed<T>`) and rc slices, as described in
[`memory.md`: Zero-copy I/O](memory.md#zero-copy-io). The binary format,
and SBE- and FlatBuffers-style layouts, support generated views.

### Databases: migrations-first

```text
migrations/
  0001_create_orders.sql
  0002_add_stock.sql          -- reviewed like code
schema.sql                    -- derived: replaying the migrations; CI checks it is current
```

- **Versioned SQL migration files are the source of truth**, and are reviewed like any other code.
- **`schema.sql` is derived** by replaying them. CI fails if it is out of date.
- **`sql"…"` literals are checked against `schema.sql`** at compile time, never against a live
  database ([`comptime.md`: Tagged string literals (embedded DSLs)](comptime.md#tagged-string-literals-embedded-dsls)).
- **At startup**, the bound database's migration version is checked against the one the code was
  built for ([`distribution.md`: Infrastructure: bind, never provision](distribution.md#infrastructure-bind-never-provision)).
- **Migrations run explicitly** (`moddil db migrate`, or a deploy step), never implicitly when a
  service starts.
- **No ORM.** Queries are SQL, typed by the compiler; rows come back as generated types or anonymous
  records.

### The `Db` effect

```rust
fn place(order: Order) -> OrderId with db: Db, suspend, fail(DbError) {
    let tx = db.begin()?;                                  // linear: must commit or roll back
    let id = tx.query_one(sql"INSERT INTO orders (price, qty) VALUES ({order.price}, {order.qty}) RETURNING id")?;
    tx.execute(sql"UPDATE stock SET qty = qty - {order.qty} WHERE sku = {order.sku}")?;
    tx.commit()?;
    id
}
```

- `Db` is a **context effect** provided by `moddil/sql`; a connection pool is its handler
  ([`effects-and-capabilities.md`](effects-and-capabilities.md)).
- **Transactions are linear values**: they must be committed or rolled back
  ([`resources.md`: Resources: affine and linear](resources.md#resources-affine-and-linear)).

### Local storage

`moddil/storage` provides key-value storage over SQLite, IndexedDB (in the browser) and files. Stored
values are contracts, so a newer app version can always read what an older one stored.

## Fit check against the reference examples

| Example | Uses |
|---|---|
| [Multiplayer game](examples/01-multiplayer-game.md) | Compact binary snapshots with locked tags; saved games as contracts |
| [HFT](examples/02-hft-platform.md) | SBE-style zero-copy views for exchange feeds; typed SQL for reference data |
| [Web store](examples/03-web-store.md) | Migrations-first PostgreSQL; `Db` effect with linear transactions; generated API contracts with old mobile clients still checked |
| [Hashiverse](examples/04-hashiverse.md) | Wire compatibility with the Rust implementation through consumed contracts; local storage as contracts |

## Open questions

- How does the `Db` effect get a simulation handler: an ephemeral real database per test, or an
  in-memory SQL engine?
- Which databases does `moddil/sql` support first (PostgreSQL, SQLite)?
- When can recorded contract versions be pruned from `contracts.lock`, and who decides?
- Which text formats beyond JSON are standard, for example TOML or YAML for configuration
  ([`lifecycle.md`](lifecycle.md))?

## Prior art

Rust serde and sqlx; Swift `Codable`; protobuf, Cap'n Proto, FlatBuffers and SBE; CBOR and
MessagePack; Avro schema resolution; Buf breaking-change detection; jOOQ and Kysely; Flyway and
golang-migrate; Atlas; "Parse, don't validate" (Alexis King, 2019).
