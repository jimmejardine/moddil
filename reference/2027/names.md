# Names

**names.resolve.1** A name used as an expression refers to the innermost local binding of that
name in scope ([Blocks](expressions/blocks.md)); if there is none, to what the name means in the
module ([Modules](modules.md)) or the prelude (a function, a struct, or a variant named on its own
such as `Some`: [Tuples, structs and variants](expressions/structs.md)). Otherwise it is an error
(E0101).

**names.resolve.2** `Enum::Variant` names a variant of an enum the module can name (or, after a
path through modules, `mod::shapes::Shape::Circle`). An enum or variant that does not exist is an
error (E0101).

**names.prelude.1** The prelude holds the items every module sees without importing them
([Prelude](prelude.md)), and `assert` ([`assert`](expressions/assert.md)), `unreachable` and
`todo` ([`unreachable` and `todo`](expressions/unreachable.md)).

**names.item.1** The functions of a module have distinct names, and so do its tests (E0103). A
function and a test may share a name.

**names.item.2** Functions, structs, enums, traits and imports share one namespace: no two of them
in a module have the same name (E0103). An item with the name of one of the prelude's shadows it
in the module ([Modules](modules.md)).

**names.type.1** A type name must name a type (E0102). The built-in types are `int` and `bool`
([Types](types.md)); the structs and enums the module can name and the prelude's are types too
(written with a path through modules if need be: `mod::shapes::Point`), as are the type
parameters in scope ([Generics](generics.md)) and, in an impl or trait, `Self`.
