# Modules and mods

```text
moddil.toml                  [mod] name = "shop"     [dependencies] geo = { path = "geo" }
src/lib.moddil               the root module: `mod`
src/cart.moddil              the module `mod::cart`
src/cart/lines.moddil        the module `mod::cart::lines`
geo/moddil.toml              another mod, which `shop` calls `geo`
```

```rust
// src/cart.moddil
use mod::cart::lines::Line;
use geo::Point as Place;

pub(mod) fn total(ls: &Line) -> int { ls.qty * 2 }   // any module of `shop` may call it
fn round(n: int) -> int { n }                        // only this module may
```

A **mod** is a unit of code with a manifest: a library, executables, and the mods it depends on.
A **module** is a file of a mod.

**mod.1** A program is a mod and the mods it depends on. A mod is a directory with a manifest,
`moddil.toml`, a subset of TOML (`#` comments, `[table]` and `[[table]]` headers, and lines
`key = "string"`, `key = true` (or `false`) or `key = { key = "string", … }`): `[mod]` gives its
`name` and, optionally, its `edition`, which is `"2027"`, its `version` (three numbers,
`"1.4.0"`, which a Python wheel takes), and whether it is a stub (**mod.stub.1**); `[lib] root` names its library's root file (by default
`src/lib.moddil`, if there is one); each `[[exe]]` names an executable's `main` file; and
`[dependencies]` names the mods it depends on, each by the name it calls it and its directory:
`geo = { path = "geo" }`. A manifest that is not like this, or names a file or directory that is
not there, is an error (E0704). A single `.moddil` file is a program too: the root module of a mod
of its own.

**mod.tree.1** The file tree is the module tree. Every `.moddil` file under a mod's source
directory (the directory of its library's root file, or `src`) is a module, except those under a
directory with a manifest of its own, which is another mod. The library's root file is the root
module; any other file is the module its path names, without the extension: `src/cart/lines.moddil`
is `cart::lines`, inside the module `cart` (which is `src/cart.moddil`, or empty if there is no
such file). A file whose name is not a name cannot be a module (E0704). Modules are not declared:
`mod name;` is an error (E0702).

**mod.names.1** A module's names are the items it declares, the names it imports, and the modules
inside it; then the prelude's ([The prelude](prelude.md)), which any of them shadow. A module names
each of its items and imports once (E0103). Its names can be used anywhere in it, in any order.

**mod.path.1** A path names an item through modules. It starts at `mod` (the root module of the
mod), `self` (this module), the name of a mod this mod depends on (its root module), or a name in
scope (an imported module, or a module inside this one); each further segment is a name in the
module before it: `mod::cart::total`, `geo::Point`, `cart::lines::Line`. The last segments may
name a variant or a function of a type, as in `Shape::Circle` (**names.resolve.2**). A segment that
names nothing is an error (E0101 for a value, E0102 for a type, E0401 for a trait). Modules may
name each other's items in cycles.

**mod.use.1** `use path;` imports what the path names (an item, a module, or a variant) under its
last segment's name, and `use path as name;` under `name`; `use a::{b, c as d};` imports several,
and `self` among them (`use a::{self, b}`, `{self as h}`) the module `a` itself (**mod.use.self.1**).
The path must name something the module can see (E0702 if it names nothing, E0701 if it is not
visible). An import is private to its module unless it is `pub use` or `pub(mod) use`, which
re-exports it. There are no glob imports: `use x::*` is an error (E0702).

**mod.use.self.1** `self` in an import group stands for the path before the group: `use a::b::{self,
C};` imports the module `a::b` as `b` and its item `C`; `self as h` imports it as `h`. It is
named at most once in a group (E0702), and `self` alone, outside a group, names nothing to import
(E0702).

**mod.vis.1** An item is visible to its own module only; `pub(mod)`, to every module of its mod;
`pub`, to every mod. A path or import that names an item its module cannot see is an error
(E0701).

**mod.vis.2** A struct's fields, and the functions of an inherent impl, have visibilities too, as
items do: private to their module unless marked `pub(mod)` or `pub`. Reading or writing a field,
naming it in a struct expression or a pattern, or calling such a function from where it is not
visible is an error (E0701); so a struct with a private field can only be built in its module. A
trait's methods are as visible as the trait, and an enum's variants' fields as the enum. The
prelude's and `core`'s fields and functions are everyone's.

**mod.expose.1** An item may not name in its signature something less visible than itself (E0703):
a `pub` function's parameter, result, error and effect types and the traits of its bounds must be
`pub` (or the prelude's), as must a `pub` struct's or enum's field types; a `pub(mod)` item's must be
`pub(mod)` or `pub`.

**mod.dep.1** The mods a program's mods depend on form a DAG: mods that depend on each other in a
cycle are an error (E0705). A mod sees only another mod's `pub` items.

**mod.infer.1** Effects and errors are written out at a mod's boundary: a `pub` function writes
its own ([Effects](effects.md)); a private or `pub(mod)` function's are inferred, across the
modules of its mod.

**mod.stub.1** A **stub** mod (`stub = true` in `[mod]`) holds signatures standing in for mods
not written yet (the umbrella `moddil` mod was one until M2d). Its functions may have no
body (elsewhere that is not supported yet), and calling one traps (`not implemented yet`). A
`#[derive(T)]` of one of its traits `T` declares an empty impl (`impl T for X {}`), so its traits
are written with no methods a derive would have to define; any other derive not in
**attr.derive.1** is not supported yet. Only a stub mod's structs may be marked
`#[root_capability]` (**main.root.1**).

**mod.entry.1** The entry points of the mod being built are the `fn main` of its root module and
of each executable's main module, and its tests ([Errors](errors.md), [Effects](effects.md)); a
function `main` of any other module is an ordinary function. The tests of every mod of the program
are checked; the conformance runner runs them all.
