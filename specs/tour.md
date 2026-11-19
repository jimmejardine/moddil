# A tour of Moddil

> **Status:** reference, until M0, when it becomes the `tour/shorty` mod
> ([From markdown to source](#from-markdown-to-source)). One small, complete program written in the
> agreed syntax, to show how the decided features fit together. Where the program needed something no spec has decided yet, it
> says so and lists it under [Gaps the tour exposed](#gaps-the-tour-exposed). Listed in the
> [index](README.md). Terms are defined in the [glossary](glossary.md).

## The program

`shorty` is a link shortener with two endpoints:

- `POST /links` with `{"url": "https://…"}` stores the link and answers `201 {"code": "…"}`;
- `GET /{code}` answers with a redirect, or `404`.

It is small, but it touches most of the language: validated types, typed errors that must be
handled, a user-defined effect with two handlers, root capabilities, structured concurrency with
isolation, graceful shutdown, and tests that run against a simulated network.

## 1. Types that validate themselves

```rust
use moddil::collections::Map
use moddil::net::{Listener, Conn, Url}
use moddil::net::http::{self, Handler, IntoResponse, Request, Response, Router, Status}
use moddil::serde::{json, Deserialize, Serialize}
use moddil::sync::Mutex

const ALPHABET: &str = "abcdefghjkmnpqrstuvwxyz23456789";

/// A short code: six characters from `ALPHABET`.
pub type Code = String where self.len() == 6 && self.chars().all(|c| ALPHABET.contains(c));

/// A link target: only HTTPS URLs are accepted.
pub type Target = Url where self.scheme() == "https";

#[derive(Deserialize)]
struct NewLink { url: Target }

#[derive(Serialize)]
struct Created { code: Code }

#[derive(Debug)]
pub enum LinkError { Taken(Code) }
```

- `Code` and `Target` are **validated newtypes**: once you hold one, its condition holds and is never
  checked again ([refinements](correctness.md#validated-newtypes-lightweight-refinements)).
- Deriving `Deserialize` for `NewLink` means **decoding is validation**: bytes become a `NewLink`
  whose `url` is already an HTTPS `Target`, or decoding fails with a typed `DecodeError`
  ([Parse, don't validate](data.md#parse-dont-validate)).
- `len()` is an `int` ([Integer types](numerics.md#integer-types)), and derives are explicit
  ([Explicit derives](values.md#explicit-derives)).

## 2. A user-defined effect for storage

```rust
/// Where links are kept.
pub trait Links {
    fn get(&self, code: &Code) -> Option<Target> with suspend;
    fn put(&self, code: Code, target: Target) with suspend, fail(LinkError);
}

/// An in-memory handler for `Links`: used by `main` here, and by the tests.
pub struct MemoryLinks { map: Mutex<Map<Code, Target>> }

impl MemoryLinks {
    pub fn new() -> MemoryLinks { MemoryLinks { map: Mutex::new(Map::new()) } }
}

impl Links for MemoryLinks {
    fn get(&self, code: &Code) -> Option<Target> with suspend {
        self.map.lock().get(code).map(|t| t.clone())
    }
    fn put(&self, code: Code, target: Target) with suspend, fail(LinkError) {
        let mut map = self.map.lock();
        if map.contains_key(&code) { fail(LinkError::Taken(code)) }
        map.insert(code, target);
    }
}
```

- `Links` is an ordinary **trait**. Listed after `with` it becomes an **effect**: its implementation
  is chosen by whoever installs a handler, not by this code, and reaches every function below without
  being passed as an argument ([Effects are traits passed implicitly](effects-and-capabilities.md#effects-are-traits-passed-implicitly)).
- `create` and `follow` call `links` directly, so they bind the name (`with links: Links`); `app`
  and `serve` only pass the need on, so they write the bare `Links`.
- A production build would install a SQL-backed handler instead. That handler captures `Db` when it is
  built in `main`, so nothing that uses `Links` changes
  ([Effects inside handlers](effects-and-capabilities.md#effects-inside-handlers)).
- Tasks share the handler, so its state is in a **sync cell**, the only shared mutable state
  ([Shared mutable state](concurrency.md#shared-mutable-state)). There is no `Send` or `Sync` to
  write ([No `Send` / `Sync`](concurrency.md#no-send--sync)).
- `t.clone()` is a visible copy of the URL ([`.clone()` is explicit](values.md#clone-is-explicit-and-means-a-copy)).

## 3. Request handling, with every error handled

```rust
/// Makes a random code.
///
/// # Example
/// with Rand = rand::seeded(7) { make_code().len() }   // => 6
fn make_code() -> Code with rand: Rand {
    let s: String = (0..6).map(|_| rand.pick(ALPHABET)).collect();
    Code::new(s) ?! "built from ALPHABET"
}

/// `POST /links`
fn create(req: &Request) -> Response with links: Links, Rand, suspend, fail(LinkError | DecodeError) {
    let new: NewLink = json::decode(req.body())?;      // untrusted bytes in, validated value out
    let code = make_code();
    links.put(code.clone(), new.url)?;
    log.info(fmt"created {code}");                     // log is ambient and write-only
    Response::json(Status::Created, &Created { code })
}

/// `GET /{code}`: a malformed code never gets here; the router answers 404.
fn follow(code: Code) -> Response with links: Links, suspend {
    match links.get(&code) {
        Some(t) => Response::redirect(Status::PermanentRedirect, &t),
        None    => Response::status(Status::NotFound),
    }
}

/// How each error becomes a response.
impl IntoResponse for DecodeError {
    fn into_response(self) -> Response { Response::text(Status::BadRequest, fmt"bad request: {self}") }
}
impl IntoResponse for LinkError {
    fn into_response(self) -> Response {
        match self { LinkError::Taken(_) => Response::status(Status::Conflict) }
    }
}

/// The routes.
fn app() -> impl Handler with Links, Rand, suspend {
    Router::new()
        .post("/links", create)
        .get("/{code}", follow)
}
```

- **Errors are an effect.** `fail(LinkError | DecodeError)` is part of `create`'s signature and `?`
  propagates. The router accepts `create` only because both errors implement `IntoResponse`, so
  every error becomes a response and nothing is left for the caller
  ([HTTP](stdlib.md#http), [Entry points must handle everything (R3)](error-handling.md#entry-points-must-handle-everything-r3)).
- **Paths are checked at compile time** and `{code}` is converted to a `Code` before `follow` runs,
  so `follow` never sees a malformed code.
- **Every effect is listed on its own** (`links, rand, suspend`); there is no umbrella I/O effect
  ([The effect catalogue](effects-and-capabilities.md#the-effect-catalogue)). Private functions could
  leave their effects to inference; they are written out here for the reader
  ([Signatures and inference](effects-and-capabilities.md#signatures-and-inference)).
- `fmt"…"` is checked at compile time ([Formatting](text.md#formatting)).

## 4. Serving, isolated per connection

```rust
/// Accepts connections until asked to stop.
pub fn serve(listener: &Listener) with Links, Rand, Net, suspend, spawn {
    scope(|s| {
        loop {
            select {
                conn = listener.accept() => s.spawn(|| handle(conn)),
                _ = stop_requested()     => break,      // stop accepting; open connections finish
            }
        }
    })
}

/// Serves one connection; a bug here cannot take down the server.
fn handle(conn: Conn) with Links, Rand, Net, suspend, spawn {
    try {
        scope(|c| c.spawn(|| http::serve_connection(conn, &app())), isolate: true)?
    } catch {
        Trapped(t) => log.error(fmt"request trapped: {t}"),
        e          => log.warn(fmt"connection ended: {e}"),
    }
}
```

- `http::serve(listener, app())` packages this loop; it is written out here to show `scope` and
  isolation ([HTTP](stdlib.md#http)).
- `scope` is the only way to start tasks, and it waits for all of them, so a connection cannot
  outlive the server ([Tasks and borrowing](concurrency.md#tasks-and-borrowing)).
- Spawned tasks **inherit their scope's handlers**, so every connection sees the same `Links`.
- **Isolation:** a trap in one connection (an overflow, a failed contract) unwinds only that
  connection's scope, which then fails with `Trapped`
  ([Traps](lifecycle.md#traps-the-process-by-default-isolated-scopes-on-request)).
- If one task in a scope fails, it cancels its siblings and its error propagates; here each scope has
  a single task ([Tasks and concurrency](error-handling.md#tasks-and-concurrency), D15).
- `stop_requested()` is the graceful half of shutdown; cancellation only follows if the grace period
  runs out ([Two levels: stop, then cancel](lifecycle.md#two-levels-stop-then-cancel)).

## 5. `main`: the only source of authority

```rust
fn main(args: Args, stdio: Stdio, net: NetRoot, entropy: Entropy) -> ExitCode {
    with Stderr = stdio.err {
        let port = try { args.value::<Port>("port", default: 8080)? } catch {
            e => { eprintln(fmt"bad --port: {e}"); return ExitCode::Usage }
        };
        with Net = net, Rand = entropy, Links = MemoryLinks::new() {
            let listener = try { net.listen(port)? } catch {
                e => { eprintln(fmt"cannot listen on {port}: {e}"); return ExitCode::Unavailable }
            };
            scope(|root| root.spawn(|| serve(&listener)), stop_on: [SIGINT, SIGTERM], grace: 10[s]);
        }
    }
    ExitCode::Ok
}
```

- `main` lists **exactly the root capabilities it uses**: arguments, the terminal, the network and
  OS randomness. It gets no file system, so no code in the program can open a file, and the runtime
  drops that OS privilege before `main` runs ([`main` declares its root capabilities](lifecycle.md#main-declares-its-root-capabilities)).
- `with … { }` installs handlers for everything inside, however deep
  ([Handlers are scoped, not global](determinism.md#handlers-are-scoped-not-global)).
- `Port` is a validated `u16` ([refinements](correctness.md#validated-newtypes-lightweight-refinements)); a bad value is a typed error, turned into exit code 64
  ([Conventional exits](lifecycle.md#conventional-exits)). Nothing is printed except through
  `Stdio`.

## 6. Tests on a simulated network

```rust
test "a created link redirects" for app {
    with Links = MemoryLinks::new(), Rand = rand::seeded(1) {
        let created = app().handle(&Request::post("/links", body: r#"{"url": "https://moddil.org"}"#));
        assert(created.status() == Status::Created);
        let code: Created = json::decode(created.body())?;
        let followed = app().handle(&Request::get(fmt"/{code.code}"));
        assert(followed.status() == Status::PermanentRedirect);
    }
}

test "plain http is rejected" for app {
    with Links = MemoryLinks::new(), Rand = rand::seeded(1) {
        let r = app().handle(&Request::post("/links", body: r#"{"url": "http://moddil.org"}"#));
        assert(r.status() == Status::BadRequest);
    }
}

test "serves over a lossy network" for serve {
    let sim = Sim::new(seed: 42);
    let (server, client) = (sim.node(), sim.node());
    scope(|s| {
        let net = sim.net.endpoint(server);
        with Net = net, Rand = sim.rng(server), Links = MemoryLinks::new() {
            let listener = net.listen(80)?;
            s.spawn(|| serve(&listener));
        }
        with Net = drop_messages(10.percent, outer: sim.net.endpoint(client)) {
            let r = http::post(fmt"http://{server.addr()}/links",
                               body: r#"{"url": "https://moddil.org"}"#, retries: 3)?;
            assert(r.status() == Status::Created);
        }
        sim.stop_all();                                 // serve sees stop_requested() and returns
    });
}
```

- **Tests name their subject** (`for app`), so tooling knows which tests cover what changed
  ([Attached tests](testing.md#attached-tests)).
- **No mocks:** the tests install different handlers for the same effects. The production code is
  unchanged ([Deterministic simulation testing](determinism.md#deterministic-simulation-testing)).
- The last test runs a server and a client in one process on a simulated network that drops 10% of
  messages. Time is virtual and the run is seeded, so a failure replays exactly.
- The `// => 6` line in `make_code`'s doc comment is a test too ([Examples are tests](testing.md#examples-are-tests)).

## From markdown to source

**Decided: at M0 the tour becomes real Moddil source, a dogfood program that checks itself.**

- It moves to **`tour/shorty/`** at the repository root: a real mod with a manifest and `src/*.mdl`
  files ([`modules.md`: Mods and modules](modules.md#mods-and-modules)).
- **Each section becomes a module** (`types`, `links`, `routes`, `serve`, `main`). Its `//!`
  overview carries this page's explanations and spec links, and the mod overview gives the reading
  order ([`docs.md`: Overviews](docs.md#overviews-mod-module-and-group-docs)).
- **Everything that can be checked is checked:** the tests in section 6 are attached `test` blocks,
  and the `// =>` examples in doc comments run as tests
  ([`testing.md`: Examples are tests](testing.md#examples-are-tests)).
- **`moddil doc` renders the mod as the tour**, and this page shrinks to a pointer plus the gaps
  table.
- **Gates, run in CI from then on** ([`roadmap.md`: Milestones](roadmap.md#milestones)):
  - **M0:** the tour parses and passes the type, effect and ownership checks;
  - **M2:** its tests pass, including the simulated-network test under `moddil/runtime-sim`.
- A failing tour blocks a release: it is the first program written in Moddil.
- The link checker (`tools/check_spec_links.py`) will then also check links in the tour's source
  comments.

Until M0, this page is the source of truth, and its spellings stay placeholders where the gaps table
says so.

## What the tour leaves out

Contracts (`requires`, `ensures`), ownership beyond the basics (views, `.to_rc()`), linear types,
`defer`, generics and traits, units, GPUs, multi-tier placement and freestanding builds. Each has its
own document in the [index](README.md).

## Gaps the tour exposed

Writing a complete program needed these, and no spec decides them yet. The spellings above are
placeholders. Each gap is recorded as an open question in its area.

Resolved since the tour was written:

- effects are traits used after `with`, how handlers are defined, how a handler gets its own
  dependencies, and `spawn` being an effect ([Effects are traits passed implicitly](effects-and-capabilities.md#effects-are-traits-passed-implicitly),
  [Defining handlers](effects-and-capabilities.md#defining-handlers), [Effects inside handlers](effects-and-capabilities.md#effects-inside-handlers),
  [The effect catalogue](effects-and-capabilities.md#the-effect-catalogue));
- the HTTP router, server and client ([HTTP](stdlib.md#http));
- the `RefinementError` a validated newtype raises ([Validated newtypes](correctness.md#validated-newtypes-lightweight-refinements));
- `?!` for a call that cannot fail, and how `catch` arms name an error union's members
  ([This cannot fail here](error-handling.md#this-cannot-fail-here), [Matching removes what it handles](error-handling.md#matching-removes-what-it-handles));
- the `Sim` API and its standard faults ([The `Sim` API](determinism.md#the-sim-api));
- an unhandled `?` inside a test fails that test with its error return trace
  ([Errors inside tests](testing.md#errors-inside-tests));
- typed argument access and the named `sysexits.h` exit codes ([`lifecycle.md`: Typed, layered configuration](lifecycle.md#typed-layered-configuration), [Conventional exits](lifecycle.md#conventional-exits)).

No gaps remain open.

| # | Gap | Open question in |
|---|---|---|
