<img src="moddil.png" alt="Moddil logo" align="right" width="150">

# Moddil

**Moddil: the programming language with armour, not baggage.**

Moddil is a programming language being designed for three shifts happening at once:

- **LLM agents write more of the code**, and humans mostly review and approve it.
- **The machine is heterogeneous**: many CPU cores, GPUs and other accelerators.
- **A program is a fleet**: browser clients, servers, workers and databases working together.

Its core idea is **semantics that are explicit and checked by machines**. Effects, capabilities,
ownership and resources, and where code and data live all appear in the types and are checked by
the compiler. That makes code easier for agents to write correctly and for humans to review.

## Status

Moddil is at the **design stage**, and there is no compiler yet. The documents in `specs/` range
from brainstorm to decided direction; none of them is a formal specification. Code snippets are
illustration only and do not commit to any syntax.

## Start here

Read [`specs/README.md`](specs/README.md) first. It is the **index**: every area with a status
(⚪ not started, 🔴 superficial, 🟠 major questions, 🟡 minor questions, 🟢 complete) and a link to
the document that holds its detail. Then read [Why Moddil](specs/vision.md) for the thesis, and the
[firm requirements](specs/requirements.md#firm-requirements-decided). People and agents editing
the specs should follow [Working on these specs](specs/README.md#working-on-these-specs).

## Layout

```
specs/   the design: one document per area, indexed by specs/README.md
tools/   helper scripts, such as the spec link checker
```

## Licence

Moddil is dual-licensed under either of

- [MIT license](LICENSE-MIT) ([`https://opensource.org/licenses/MIT`](https://opensource.org/licenses/MIT))
- [Apache License, Version 2.0](LICENSE-APACHE) ([`https://www.apache.org/licenses/LICENSE-2.0`](https://www.apache.org/licenses/LICENSE-2.0))

at your option. This is the same dual-licence arrangement used by the Rust project itself, and by the majority of the Rust ecosystem.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in Moddil by you, as defined in the Apache-2.0 licence, shall be dual-licensed as above, without any additional terms or conditions.
