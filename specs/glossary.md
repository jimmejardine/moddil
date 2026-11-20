# Glossary

> **Status:** reference. One line per Moddil term, each linked to the section that defines it. The
> linked section is authoritative; if the two disagree, fix this line. Listed in the
> [index](README.md).

| Term | Meaning |
|---|---|
| **`?!`** | `call ?! "reason"`: asserts that a call cannot fail; traps with the reason if it does (D17) ([This cannot fail here](error-handling.md#this-cannot-fail-here)). |
| **Abortive handler** | A handler that never resumes the code that performed the effect; `fail` works this way and compiles to an ordinary return ([Handler tiers](effects-and-capabilities.md#handler-tiers)). |
| **Affine type** | A type whose values may be used at most once and are dropped automatically: the default for owned values ([Resources: affine and linear](resources.md#resources-affine-and-linear)). |
| **Agent server** | The long-running compiler service that agents and editors query for types, effects and fixes ([The agent server](agent-interface.md#the-agent-server)). |
| **Annotation overlay** | A file that adds ownership, effects and error facts to a C or Rust API without editing its headers ([Annotation overlays](interop.md#annotation-overlays)). |
| **Anonymous impl** | `impl Trait { … }` as an expression: implements a trait for an unnamed type and captures locals like a closure; the usual one-off handler ([Defining handlers](effects-and-capabilities.md#defining-handlers)). |
| **Assurance level** | Mod metadata stating how much is proven or checked, from tested to fully verified ([Assurance levels per mod](correctness.md#assurance-levels-per-mod)). |
| **Attached test** | `test "…" for f { … }`: a test that names its subject and sits beside it ([Attached tests](testing.md#attached-tests)). |
| **Biased reference counting** | Rc counting that is non-atomic for the owning task and atomic only for others ([Biased reference counting](memory.md#biased-reference-counting)). |
| **Borrow** | A second-class reference (`&T`, `&mut T`) that cannot be stored or returned, so it needs no lifetime annotations ([Borrowing](memory.md#borrowing)). |
| **Bridge mod** | A mod allowed to implement a foreign trait for a foreign type, the sanctioned exception to the orphan rule ([Bridge mods](types.md#bridge-mods)). |
| **Budget** | `#[budget(…)]` on a `bench` block: a limit on deterministic counters (allocations, instructions) that fails CI ([Benchmarks and budgets](performance.md#benchmarks-and-budgets)). |
| **Cancel** | The hard second stage of shutdown: the next suspension point returns `Cancelled` ([Two levels: stop, then cancel](lifecycle.md#two-levels-stop-then-cancel)). |
| **Capability** | Authority to perform an effect, held as a handler; it originates only in `main`'s root capabilities ([Authority comes only from root capabilities](effects-and-capabilities.md#authority-comes-only-from-root-capabilities)). |
| **Capability ceiling** | The capabilities a mod may use at all, declared in its manifest and checked by the compiler; a JSON parser that uses `Net` fails to build ([Capability-scoped dependencies](packages-and-supply-chain.md#capability-scoped-dependencies)). |
| **Capability value** | A handler passed explicitly as a second-class `&dyn Trait`, e.g. when one function uses two handlers of one trait at once ([Capability values](effects-and-capabilities.md#capability-values)). |
| **Captured handler** | A handler that keeps another handler it was built with (a SQL handler keeping `Db`, a recorder keeping the outer `Net`); it is second-class and cannot outlive that scope ([Effects inside handlers](effects-and-capabilities.md#effects-inside-handlers)). |
| **Class 1 / 2 / 3** | The three outcome classes: errors (must be handled), defined arithmetic (no handling), traps (crash or isolated scope) ([Three outcome classes](error-handling.md#three-outcome-classes)). |
| **`.clone()`** | An explicit deep copy whose cost grows with the value; `Rc` has no `.clone()` ([`.clone()` is explicit and means a copy](values.md#clone-is-explicit-and-means-a-copy)). |
| **Coherence** | At most one impl of a trait for a type in a program, enforced by the orphan rule and its exceptions ([Coherence: one impl per trait and type](types.md#coherence-one-impl-per-trait-and-type)). |
| **Component** | A module marked `#[component(…)]` and placed on a tier (client, server, worker) of a multi-tier program ([Layer A: components and placement](distribution.md#layer-a-components-and-placement)). |
| **Comptime** | Code evaluated by the compiler's interpreter at build time: generics, reflection and code generation ([Evaluation model](comptime.md#evaluation-model)). |
| **Context data** | A user-defined nominal type listed after `with` (`with config: &AppConfig`): passed implicitly by exact type, read-only; replaces `Context` objects ([Context data](effects-and-capabilities.md#context-data)). |
| **Context note** | A note attached after `?` (`load(p)? fmt"loading {p}"`), shown in traces without changing the error's type (D16) ([Error types](error-handling.md#error-types)). |
| **Contract** | `requires`, `ensures` and `invariant` clauses after a signature; a failed contract traps (D13) ([Contracts as syntax](correctness.md#contracts-as-syntax)). |
| **`Contracts`** | The ambient handler that sets which contracts are checked at run time (`all`, `pub_preconditions`, `none`), installed per scope ([Checking policy: the `Contracts` handler](correctness.md#checking-policy-the-contracts-handler)). |
| **Cooldown** | A randomised delay before a newly published dependency version is adopted, overridable for security fixes ([Randomised cooldown](packages-and-supply-chain.md#randomised-cooldown)). |
| **Core** | What the language itself defines (syntax, core effects, core traits); everything else lives in mods (R2) ([The core / mod boundary](stdlib.md#the-core--mod-boundary)). |
| **`defer` / `errdefer`** | Block-scoped cleanup: `defer` is infallible and always runs; `errdefer` runs only on error and cancellation exits and may fail ([`defer` and `errdefer`](resources.md#defer-and-errdefer)). |
| **Delegation** | `#[delegate(Trait)]` forwards a trait's methods to a field, replacing implementation inheritance ([Multiple inheritance](types.md#multiple-inheritance)). |
| **`discard`** | `discard(r, "reason")`: the only way to ignore an error, visible in review (D14) ([Entry points must handle everything (R3)](error-handling.md#entry-points-must-handle-everything-r3)). |
| **`dyn`** | An existential: any trait used as a type, behind a pointer with a witness table ([`dyn`: existentials](types.md#dyn-existentials)). |
| **Edition** | A version of the language chosen per mod in its manifest (`edition = "2027"`); mods of different editions work together in one program, and migrations are automated ([Decisions](evolution.md#decisions)). |
| **Effect** | A trait listed after `with` in a signature (`with net: Net`), whose implementation is passed implicitly by scope; plus the built-in control effects `suspend`, `spawn` and `fail(E)` ([Effects are traits passed implicitly](effects-and-capabilities.md#effects-are-traits-passed-implicitly)). |
| **Effect polymorphism** | A function such as `visit` takes on the effects of the closure it is given ([Effect polymorphism](effects-and-capabilities.md#effect-polymorphism)). |
| **`escape`** | `escape v`: returns `v` from the enclosing function across non-escaping lambdas; in a task scope the first one wins and cancels the rest ([Control flow out of a lambda](closures.md#control-flow-out-of-a-lambda)). |
| **Executor** | A pool of OS threads with a scheduling policy (`default`, `blocking`, `pinned`, `affine`) inside the one runtime ([Tasks, threads and executors](runtime.md#tasks-threads-and-executors)). |
| **First error wins** | In a task scope, the first failure cancels the siblings; their later errors are suppressed onto it (D15) ([Tasks and concurrency](error-handling.md#tasks-and-concurrency)). |
| **Flight recorder** | An in-memory ring of recent events dumped on an error or trap ([Flight recorder and replay](observability.md#flight-recorder-and-replay)). |
| **Freestanding profile** | A build without an operating system (kernels, firmware), with `#[entry]` instead of `main` ([Profiles](freestanding.md#profiles)). |
| **Generator** | A `gen` block or function that yields values, giving an `Iterator` written as a loop ([Generators](stdlib.md#generators)). |
| **Group** | `#[group(…)]`: a named set of related `pub` items with its own overview; every `pub` item belongs to one ([Overviews: mod, module and group docs](docs.md#overviews-mod-module-and-group-docs)). |
| **Handler** | The value installed for an effect with `with Trait = value { … }`: an ordinary impl or an anonymous impl; the innermost one wins ([Defining handlers](effects-and-capabilities.md#defining-handlers)). |
| **Hot-path attributes** | `#[no_alloc]`, `#[no_lock]` and similar compiler-checked latency guarantees ([Hot-path guarantees](performance.md#hot-path-guarantees)). |
| **`int`** | The signed 64-bit integer used for lengths, indices and counts on every target ([Integer types](numerics.md#integer-types)). |
| **Interface artefact** | A mod's `pub` items with their full signatures: what dependents compile against and what the semver check compares ([Language rules that decide build speed](build-times.md#language-rules-that-decide-build-speed)). |
| **Interposition** | A handler that captures the enclosing handler for the same trait and forwards to it, to record, delay or inject faults ([Effects inside handlers](effects-and-capabilities.md#effects-inside-handlers)). |
| **Isolated scope** | `scope(…, isolate: true)`: a trap inside unwinds only that scope, which fails with `Trapped` ([Traps: the process by default, isolated scopes on request](lifecycle.md#traps-the-process-by-default-isolated-scopes-on-request)). |
| **JSPI** | JavaScript Promise Integration: the first WASM lowering for suspension, in browsers and Node ([WASM concurrency lowering](compiler.md#wasm-concurrency-lowering)). |
| **Linear type** | A type whose values must be explicitly consumed (`commit`, `close`), used when cleanup can fail ([Resources: affine and linear](resources.md#resources-affine-and-linear)). |
| **Live update** | Patching a running debug build while keeping its state ([Live update (debug builds)](build-times.md#live-update-debug-builds)). |
| **Mod** | The unit of distribution, versioning and capability ceilings: a git repository (or a folder in one) with a manifest ([What a mod is](packages-and-supply-chain.md#what-a-mod-is)). |
| **`moddil bind`** | Generates Python, Swift, Kotlin and TypeScript bindings to a Moddil core ([Generated bindings](ui.md#generated-bindings)). |
| **Moddil Evolution** | The proposal process for language and standard-mod changes ([Moddil Evolution](governance.md#moddil-evolution)). |
| **`moddil verify`** | The toolchain subcommand that proves contracts with an SMT solver (Z3, pinned in the lockfile) ([Proofs in Moddil itself](correctness.md#proofs-in-moddil-itself)). |
| **Module** | A namespace inside a mod; visibility is module, mod or public ([Mods and modules](modules.md#mods-and-modules)). |
| **`nondet`** | A capability marking opt-in nondeterminism (fast-math, float atomics); forbidden in replayable code ([The `nondet` effect](determinism.md#the-nondet-effect)). |
| **Note** | `note "…"`: a maintainer note attached to an item, kept out of the public docs ([Maintainer notes](docs.md#maintainer-notes)). |
| **Orphan rule** | An impl must live with its trait or its type, with three exceptions: the application, the same repository and bridge mods ([Who may write an impl](types.md#who-may-write-an-impl)). |
| **Placement** | Choosing where code runs: executor, core, GPU device or tier, by the same mechanism at each level ([Tasks, threads and executors](runtime.md#tasks-threads-and-executors)). |
| **Prelude** | The names every module sees without `use`, and the one glob-importable module a mod may declare ([The prelude](stdlib.md#the-prelude)). |
| **`pub(mod)`** | Visibility to every module in the same mod, between private and exported `pub` ([Three visibility levels](modules.md#three-visibility-levels)). |
| **Rc** | `Rc<T>`: a shared, reference-counted handle to immutable data, made with `.to_rc()` ([Sharing: `.to_rc()`](memory.md#sharing-to_rc)). |
| **Reference interpreter** | An interpreter over the canonical AST, kept separate from the compiler for differential testing ([Two interpreters](compiler.md#two-interpreters)). |
| **Refinement** | A validated newtype such as `type Port = u16 where self >= 1024` ([Validated newtypes (lightweight refinements)](correctness.md#validated-newtypes-lightweight-refinements)). |
| **`Remote`** | The effect performed by a call that crosses tiers, with its own failures ([The `Remote` effect](distribution.md#the-remote-effect)). |
| **`requires always`** | A precondition checked under every `Contracts` policy, even `none` ([Checking policy: the `Contracts` handler](correctness.md#checking-policy-the-contracts-handler)). |
| **Role trait** | An empty subtrait with a blanket impl (`trait DbInput: Db {}`), giving a second handler of a trait its own effect ([Role traits](effects-and-capabilities.md#role-traits)). |
| **Root capability** | A capability `main` receives as a parameter (`NetRoot`, `FsRoot`, `Stdio`, …); the only source of authority ([`main` declares its root capabilities](lifecycle.md#main-declares-its-root-capabilities)). |
| **Router** | `http::Router`: maps compile-time-checked paths to handler functions, converting path parameters with `TryFrom`; handler errors must implement `IntoResponse` ([HTTP](stdlib.md#http)). |
| **Runtime** | A mod providing handlers for scheduling, I/O and time (`moddil/runtime`, `-wasm`, `-sim`) ([Pluggable runtimes without ecosystem splits](runtime.md#pluggable-runtimes-without-ecosystem-splits)). |
| **Safety contract** | The documented conditions a caller of an `unsafe fn` must uphold ([Safety contracts](unsafe.md#safety-contracts)). |
| **Scope** | `scope(\|s\| …)`: the only way to start tasks; it waits for all of them and is their cancellation token ([Tasks and borrowing](concurrency.md#tasks-and-borrowing)). |
| **Security labels** | `Secret<T>`, `Personal<T>` and `Untrusted<T>`, checked by the compiler ([Information-flow labels](security.md#information-flow-labels)). |
| **Semantic diff** | A report of what a change does to interfaces, effects, errors and tests, rather than to lines ([The report](semantic-vcs.md#the-report)). |
| **`.share()`** | An O(1) count increment returning another handle to the same rc object ([`.share()` for reference counts](values.md#share-for-reference-counts)). |
| **Simulation runtime** | `moddil/runtime-sim`: one deterministic thread with virtual time, seeded randomness and a simulated network ([Deterministic simulation testing](determinism.md#deterministic-simulation-testing)). |
| **`spawn`** | The core effect of starting tasks in a scope, separate from `suspend` ([The effect catalogue](effects-and-capabilities.md#the-effect-catalogue)). |
| **Specialisation** | A restricted form of overlapping impls: strictly more specific, beside the general impl, and able to change a `default type` ([Specialisation](types.md#specialisation)). |
| **Stop** | The graceful first stage of shutdown, observed where code asks with `stop_requested()` ([Two levels: stop, then cancel](lifecycle.md#two-levels-stop-then-cancel)). |
| **`StrPos`** | A byte position on a character boundary, tied by type to its string ([Positions and slicing](text.md#positions-and-slicing)). |
| **Suppressed error** | An error attached to a primary error rather than replacing it, from cleanup (D10) or from sibling tasks (D15) ([Cleanup that can fail](resources.md#cleanup-that-can-fail)). |
| **`suspend`** | The core effect of pausing a task; any function that may pause lists it ([The effect catalogue](effects-and-capabilities.md#the-effect-catalogue)). |
| **Sync cell** | `Atomic<T>`, `AtomicRc<T>`, `Mutex<T>` or `RwLock<T>`: the only shared mutable state ([Shared mutable state](concurrency.md#shared-mutable-state)). |
| **Tagged literal** | A string literal with a prefix (`fmt"…"`, `sql"…"`, `re"…"`) parsed and checked at compile time ([Tagged string literals (embedded DSLs)](comptime.md#tagged-string-literals-embedded-dsls)). |
| **Task** | A stackful coroutine; the only unit of concurrency, with no user-facing threads ([Tasks, threads and executors](runtime.md#tasks-threads-and-executors)). |
| **Tier (of mods)** | `moddil/core-*` (ships with the compiler), the standard `moddil/*` mods, and community mods ([Batteries and tiers](stdlib.md#batteries-and-tiers)). |
| **Topology file** | The deployment description mapping components to artefacts and machines ([The topology file](distribution.md#the-topology-file)). |
| **Trait alias** | `trait Num = Add + Sub + Mul + Copy;`: a transparent name for a set of bounds, satisfied by every type that meets them ([Traits](types.md#traits)). |
| **Trap** | A class-3 outcome (overflow, bounds, a failed contract, out of memory): terminates the process or an isolated scope ([Three outcome classes](error-handling.md#three-outcome-classes)). |
| **Try block** | `try { … }` turns propagating errors into a `Result` (or `None` into an `Option`) value; `catch` handles listed variants ([Try blocks](error-handling.md#try-blocks)). |
| **Verbosity is a view** | Inferred types and effects are shown or hidden by the editor, not by what is written ([Verbosity is a view](agent-interface.md#verbosity-is-a-view)). |
| **View type** | A struct of borrows (`view V { a: &A }`) that flows down the call stack but is never stored ([View types](memory.md#view-types)). |
| **Witness table** | The per-type table of methods and layout behind a `dyn` value or an unspecialised generic ([`dyn`: existentials](types.md#dyn-existentials)). |
| **Wrapping operators** | `+%`, `-%`, `*%` and `Wrapping<T>`: the explicit form of overflow, which otherwise traps ([Overflow traps](numerics.md#overflow-traps)). |
