# Distributed and Multi-tier Programs

> **Status:** decided direction, details open. Expands [`rationale.md`](rationale.md) §2.10.
> Snippets and file formats are **illustration only**; no syntax is committed.

## 1. Summary

| Rule | One line |
|---|---|
| **The language owns meaning across boundaries** | Components, placement, the `remote` effect, wire contracts, version skew, trust and secrets are compiler-checked. |
| **Existing tools own where and how it runs** | Docker/OCI, Kubernetes, Nomad, Consul, Vault and Terraform do packaging, scheduling, networking, secrets and provisioning. Moddil feeds them; it never replaces them. |
| **Remote is never invisible** | A remote call is a `remote` effect with typed failures, deadlines and cancellation. |
| **Contracts are generated and skew-checked** | Wire formats come from the types; a change that would break the deployed version fails to compile. |
| **One program, many topologies** | A topology file maps components to deployment units and platforms: one process in development, containers in production. |
| **One binary per deployment unit** | Each deployment unit is its own static binary and OCI image, built without a Dockerfile. |
| **Bind infrastructure, never provision it** | Components declare typed requirements (a Postgres, a queue); the topology binds them to infrastructure that already exists. |
| **Staged** | v0: the language layer. v1: images, Compose, Kubernetes, skew checks. Later: the rest. |

## 2. Is it worth it? Lessons first

Two histories argue for caution:
- **Multitier languages** (Links, Ur/Web, Hop.js, Eliom, ScalaLoci) were technically impressive and
  stayed niche.
- **Google's Service Weaver**, the closest match to "one program, deployed as many services", was put
  into maintenance mode about two years after launch.

They struggled for the same four reasons:
1. **All-or-nothing adoption.** The whole system had to move in, but real organisations are polyglot
   and have existing services.
2. **They fought the ops team.** Deployment, scaling, networking and secrets belong to platform teams
   and their tools.
3. **Hidden networks.** Several made remote calls look local: the trap described by Waldo et al.
   (*A Note on Distributed Computing*, 1994).
4. **Proprietary edges.** Custom protocols meant other systems couldn't talk to them.

**The design rule that follows:** *the language owns the program's meaning across boundaries;
existing tools own where and how it runs.* Every feature below is judged by which side of that line
it falls on. Adoption must be incremental: a Moddil service is an ordinary service to everyone else.

## 3. Three layers

| Layer | Scope | Decision |
|---|---|---|
| **A. Language** | Components and placement; the `remote` effect; generated contracts and skew checks; trust, secrets and capabilities; structured concurrency across machines; whole-system simulation | **Do it.** Only a compiler can, and most of it reuses existing decisions. |
| **B. Artefacts and adapters** | Static binaries, WASM and OCI images; a topology file; adapter mods emitting Compose, Kubernetes, Nomad, systemd and WASI-platform formats; discovery, secret and policy integration | **Do it thinly**, as ordinary mods (R2). |
| **C. Infrastructure** | Orchestration, scheduling, autoscaling, service mesh, load balancing, provisioning, secret storage, rollout strategy | **Never.** Kubernetes, Nomad, Consul, Vault, Terraform and Argo own these (non-goals, rationale §5.2). |

## 4. Layer A: components and placement

```
component Api    @server { fn checkout(cart: Untrusted<Cart>) -> Order ! {db, payments, fail(CheckoutError)} }
component Worker @worker { fn send_receipt(order: OrderId) ! {email} }
component Web    @client { … api.checkout(cart) … }      // ! {remote, fail(RemoteError | CheckoutError)}
```

- A **component** is a unit of code with a **placement**: `@client`, `@server`, `@worker`, `@edge`,
  or `@peer`.
- `@peer` covers **symmetric** topologies, where every node is both client and server (Hashiverse).
- The compiler checks what may live where: `@client` code cannot use `db`; server-only types cannot be
  sent to the client; secrets cannot cross to `@client` (§7).
- Placement is one ladder at every scale: thread, core, device ([`accelerators.md`](accelerators.md)
  §4), machine, tier (rationale theme 4).

## 5. The `remote` effect

- Calling another component is a **`remote` effect**. It never looks like a local call.
- It has **typed failures** under R3: `Timeout`, `Unavailable`, `Incompatible` (version), `Cancelled`,
  plus the callee's own errors.
- **Deadlines and cancellation propagate** across calls, because a remote call is part of the caller's
  structured-concurrency scope ([`concurrency.md`](concurrency.md)). A request that fans out to
  workers is one scope spanning machines.
- **Retry and idempotency are declared**, not improvised: an operation is marked idempotent (or given
  an idempotency key), and the retry policy is part of the call site or the topology.
- There is **no fire-and-forget** (error-handling D4): a message either belongs to a scope, or is a
  durable workflow step (§9).

## 6. Wire contracts and version skew

- Contracts are **generated from the types** at each boundary. There is no hand-written OpenAPI or
  protobuf between Moddil components.
- **Schema evolution rules** are explicit: added fields need defaults or `Option`; removed fields go
  through deprecation; renames are refactor intents (rationale §2.19), not breaking changes.
- **Skew checking against what is deployed.** The contract of each deployed version is recorded (in
  the repository or a registry). The compiler compares a change with every version still in service
  (including months-old mobile clients) and rejects changes that would break a rolling deploy. The
  semantic diff shows contract changes.
- **At the edges**, for non-Moddil peers, the compiler generates standard descriptions
  (OpenAPI, protobuf/gRPC, AsyncAPI), and can consume them to produce typed clients. Other teams see
  an ordinary service.

## 7. Trust, secrets and capabilities

- Data arriving from `@client` or `@peer` is **`Untrusted<T>`** until validated, typically into a
  validated newtype ([`correctness.md`](correctness.md) §3.2).
- **`Secret<T>`** can never flow to `@client`, nor into logs or traces
  ([`observability.md`](observability.md) §6).
- **Capabilities per component** give least privilege: the compiler knows that `Worker` never uses
  `db`. That **capability graph** is exported to Layer B, which turns it into network and secret
  policies (§12).

## 8. Whole-system runs and simulation

- `moddil run --topology dev` runs **every component in one process**, with real or simulated
  networking. No containers are needed to try the whole system.
- The **simulation runtime** ([`determinism.md`](determinism.md)) runs every tier in one process with
  simulated latency, partitions, crashes and clock skew, replayable from a seed.
- `--topology compose` runs the same components as real containers when that matters.

## 9. Durable workflows

- Long-running, crash-surviving processes (order fulfilment, sign-up flows) use a **workflow effect**.
- Workflow code may use only replayable effects, which the compiler checks
  ([`determinism.md`](determinism.md) §5).
- Handlers can target **Temporal** or **Restate**, or a built-in handler based on record and replay.
  Moddil does not need to build its own workflow engine.

## 10. Layer B: artefacts

- **One static binary per deployment unit.** Shared code is compiled once and cached by the build
  ([`build-times.md`](build-times.md)), so many binaries do not mean many full builds.
- `@client` components compile to WASM for the browser; edge and serverless components can compile to
  **WASI components** (R1).
- **OCI images are built directly** from the binaries, as Go's `ko`, Java's Jib and Nix do: no
  Dockerfile, reproducible, minimal (distroless) images that Docker, Podman or containerd run and any
  registry stores.
- Each artefact comes with an SBOM and build provenance (rationale §2.16).

## 11. The topology file

The topology maps **logical components → deployment units → a platform**, per environment.

```toml
# topology (illustrative)
[topology.dev]
mode = "single-process"

[topology.prod]
platform = "kubernetes"                    # adapter mod: moddil/deploy-k8s

[deploy.api]
components = ["Api", "Catalog"]
replicas   = 6                             # hint, passed through to the platform

[deploy.worker]
components = ["Worker"]
replicas   = 2

[requires.db]
kind = "postgres"
bind = "terraform:output.db_url"           # bind to existing infrastructure (§13)
```

- Replica counts and resources are **hints passed through** to the platform; Moddil doesn't schedule.
- The same file selects runtime and observability handlers ([`observability.md`](observability.md)
  §3) and binds device roles ([`accelerators.md`](accelerators.md) §4).

## 12. Adapter mods

Adapters are ordinary mods (R2) that translate the topology and artefacts into existing platforms'
formats. The community can add more (ECS, Fly, Cloud Run, …).

| Concern | Adapters |
|---|---|
| Local containers | Docker Compose |
| Orchestration | Kubernetes (plain manifests, Helm or Kustomize output), Nomad job specs, systemd units, WASI platforms |
| Service discovery | Handlers for the `remote` effect: Consul, Kubernetes DNS, static configuration |
| Secrets | Secret-provider handlers: Vault, Kubernetes secrets, cloud KMS. Secrets never appear in manifests. |
| Policy | Generated from the capability graph (§7): Kubernetes NetworkPolicy, Consul intentions, Vault policies |
| Observability | OTLP export ([`observability.md`](observability.md)) |

Generating policies from the capability graph is the main piece of leverage no other toolchain has:
least privilege follows from the code instead of being written by hand.

## 13. Infrastructure: bind, never provision

- Components declare **typed requirements**: a Postgres database with schema S, a queue of messages of
  type M, an object bucket.
- The topology **binds** each requirement to infrastructure that already exists: Terraform/OpenTofu
  outputs, Kubernetes secrets, Consul KV, environment variables.
- Where possible, bindings are checked when the program starts or deploys (for example, the
  database's schema version against the one the code expects).
- **Terraform, OpenTofu and Pulumi remain the provisioning tools.** Moddil never calls cloud APIs to
  create resources.

## 14. Layer C: what Moddil never does

Orchestration, scheduling, autoscaling, service mesh, load balancing, provisioning, secret storage and
rollout strategy (canaries, blue-green). Moddil supplies what these tools lack — compatibility
guarantees, generated policies and typed requirements — and leaves the rest to them.

## 15. Staging

| Stage | Delivers |
|---|---|
| **v0** | Layer A: components, placement, the `remote` effect, generated contracts, single-process mode and simulation |
| **v1** | OCI images, Compose and Kubernetes adapters, skew checking against deployed contracts |
| **Later** | Nomad and WASI-platform adapters, generated policies, Temporal/Restate workflow handlers, choreographies |

## 16. Fit check against the reference examples

| Example | Uses |
|---|---|
| [Multiplayer game](examples/01-multiplayer-game.md) | `@client`, hub and chat components; reliable and unreliable channels distinguished in types; whole match simulated in one process |
| [HFT](examples/02-hft-platform.md) | Collector, store, research and execution as separate deployment units and binaries; latency-critical paths with no mesh in the way; one strategy mod placed in both research and execution |
| [Web store](examples/03-web-store.md) | Client, API and database; skew checks against months-old mobile clients; Postgres provisioned by Terraform and bound as a requirement; payment keys as `Secret<T>` |
| [Hashiverse](examples/04-hashiverse.md) | `@peer` placement; wire compatibility with existing Rust servers via consumed contracts; a single static binary for volunteers |

## 17. Open questions

- Where deployed contracts are recorded: in the repository (a lock file) or a registry?
- **Choreographies**: write a multi-party protocol once and project it to each participant — when,
  and how much?
- Databases as requirements: how schema migrations are versioned and checked alongside code.
- Independent team deploys: can one team ship its component without rebuilding the world?
- How much rollout safety (e.g. canary analysis) should Moddil check, versus leave to the platform?
- How unreliable or unordered channels (game snapshots over WebTransport) are expressed in types.

## Prior art

Links; Ur/Web; Hop.js; Eliom; ScalaLoci; Choral and HasChor; Erlang/OTP; Orleans; Service Weaver;
Unison Cloud; Winglang; Ballerina; Temporal; Restate; Hydro. Go `ko`, Jib, Nix `dockerTools`, OCI;
Docker Compose, Kubernetes, Nomad, Consul, Vault, Terraform/OpenTofu, Pulumi, Argo. gRPC deadlines.
Waldo, Wyant, Wollrath, Kendall, *A Note on Distributed Computing* (1994); Deutsch, "Fallacies of
distributed computing".
