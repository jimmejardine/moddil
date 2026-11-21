# Names

**names.resolve.1** A name used as an expression refers to the innermost local binding of that
name in scope ([Blocks](expressions/blocks.md)); if there is none, to the item of that name in the
module or the prelude (a function, a struct, or a variant named on its own such as `Some`:
[Tuples, structs and variants](expressions/structs.md)). Otherwise it is an error (E0101).

**names.resolve.2** `Enum::Variant` names a variant of an enum of the module. An enum or variant
that does not exist is an error (E0101).

**names.prelude.1** The prelude holds the items every module sees without importing them
([Prelude](prelude.md)), and `assert` ([`assert`](expressions/assert.md)).

**names.item.1** The functions of a module have distinct names, and so do its tests (E0103). A
function and a test may share a name.

**names.item.2** Functions, structs, enums and traits share one namespace: no two of them in a
module have the same name, and none has the name of an item of the prelude (E0103).

**names.type.1** A type name must name a type (E0102). The built-in types are `int` and `bool`
([Types](types.md)); the structs and enums of the module and the prelude are types too, as are the
type parameters in scope ([Generics](generics.md)) and, in an impl or trait, `Self`.
