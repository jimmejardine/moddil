# A tour of Moddil

> **Status:** the tour is the [`tour/shorty`](../tour/shorty/README.md) mod, checked in CI since
> slice S9d ([From markdown to source](#from-markdown-to-source)); this page points to it and keeps
> the [gaps it exposed](#gaps-the-tour-exposed). One small, complete program written in the agreed
> syntax, to show how the decided features fit together. Listed in the [index](README.md). Terms
> are defined in the [glossary](glossary.md).

## The program

`shorty` is a link shortener with two endpoints:

- `POST /links` with `{"url": "https://…"}` stores the link and answers `201 {"code": "…"}`;
- `GET /{code}` answers with a redirect, or `404`.

It is small, but it touches most of the language: validated types, typed errors that must be
handled, a user-defined effect with two handlers, root capabilities, structured concurrency with
isolation, graceful shutdown, and tests that run against a simulated network.

**The program is [`tour/shorty/`](../tour/shorty/README.md)**, read in this order: the mod's
overview ([`src/lib.moddil`](../tour/shorty/src/lib.moddil)), then its modules, each a section
of the tour with its explanations and spec links in its `//!` overview:

1. [`types`](../tour/shorty/src/types.moddil): types that validate themselves;
2. [`links`](../tour/shorty/src/links.moddil): a user-defined effect for storage;
3. [`routes`](../tour/shorty/src/routes.moddil): request handling, with every error handled, and
   the tests of `app`;
4. [`serve`](../tour/shorty/src/serve.moddil): serving, isolated per connection, and the test on
   a simulated network;
5. [`main`](../tour/shorty/src/main.moddil): `main`, the only source of authority.

## From markdown to source

**Decided: at M0 the tour becomes real Moddil source, a dogfood program that checks itself.**
Done in slice S9d (November 2026).

- It moves to **`tour/shorty/`** at the repository root: a real mod with a manifest and `src/*.moddil`
  files ([`modules.md`: Mods and modules](modules.md#mods-and-modules)).
- **Each section becomes a module** (`types`, `links`, `routes`, `serve`, `main`). Its `//!`
  overview carries this page's explanations and spec links, and the mod overview gives the reading
  order ([`docs.md`: Overviews](docs.md#overviews-mod-module-and-group-docs)).
- **Everything that can be checked is checked:** the tests in section 6 are attached `test` blocks,
  and the `// =>` examples in doc comments run as tests
  ([`testing.md`: Examples are tests](testing.md#examples-are-tests)).
- **`moddil doc` renders the mod as the tour** (once `moddil doc` exists), and this page shrinks
  to a pointer plus the gaps table.
- **Until M2, its libraries are stubs:** the standard mods it uses are signatures only, in
  [`stubs/moddil/`](../stubs/moddil/README.md), a mod whose manifest says `stub = true`, so the
  tour checks against them and its tests trap if run.
- **Gates, run in CI from then on** ([`roadmap.md`: Milestones](roadmap.md#milestones)):
  - **M0:** the tour parses and passes the type, effect and ownership checks;
  - **M2:** its tests pass, including the simulated-network test under `moddil/runtime-sim`.
- A failing tour blocks a release: it is the first program written in Moddil.
- The link checker (`tools/check_spec_links.py`) is to check links in the tour's source comments
  too; its comments name spec sections as text for now.

The source is now the tour; where it had to differ from the page it replaced, the gaps table says
why.

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

Open, found when the tour became source (slice S9d). The tour's code works around each one as
the table says.

| # | Gap | Open question in |
|---|---|---|
| 1 | `default` is a keyword, so it cannot name a parameter or an argument: `args.value::<Port>("port", default: 8080)` is written with `fallback:` | [`syntax.md`](syntax.md#open-questions) |
| 2 | A type's function and its method cannot share a name: `Response::status(s)` and `r.status()`; the function is `Response::empty(s)` | [`types.md`](types.md#open-questions) |
| 3 | Installing a handler moves its value, so `main` cannot install `net` (`with Net = net`) and call `net.listen(port)` inside; it listens first | [`effects-and-capabilities.md`](effects-and-capabilities.md#open-questions) |
| 4 | A task spawned inside a `with` outlives the handlers the `with` installs, so a scope's tasks use handlers installed around the scope; the simulated-network test installs the server's handlers around it | [`concurrency.md`](concurrency.md#open-questions) |
| 5 | Spawning a closure that can fail is marked twice, in the closure and at `spawn`: `c.spawn(\|\| http::serve_connection(conn, &app())?)?` | [`concurrency.md`](concurrency.md#open-questions) |
| 6 | `use a::{self, b}` is not specified; the tour writes two `use` lines | [`modules.md`](modules.md#open-questions) |
| 7 | A sync cell's `lock()` gives a `&mut T` from `&self`, standing in for a guard, which needs `Deref` | [`stdlib.md`](stdlib.md#open-questions) |
| 8 | There are no floats before M0 ends, so the lossy network drops `percent: 10` of messages, not `rate: 0.1` | [`roadmap.md`](roadmap.md) (floats come after M0) |
