# Names

**names.resolve.1** A name used as an expression refers to the innermost local binding of that
name in scope ([Blocks](expressions/blocks.md)); if there is none, to the item of that name in the
module (a function, or a struct: [Tuples, structs and variants](expressions/structs.md)); if there
is none, to the prelude. Otherwise it is an error (E0101).

**names.resolve.2** `Enum::Variant` names a variant of an enum of the module. An enum or variant
that does not exist is an error (E0101).

**names.prelude.1** The prelude holds the built-ins every module sees. In this edition it holds
`assert` ([`assert`](expressions/assert.md)).

**names.item.1** The functions of a module have distinct names, and so do its tests (E0103). A
function and a test may share a name.

**names.item.2** Functions, structs and enums share one namespace: no two of them in a module have
the same name (E0103).

**names.type.1** A type name must name a type (E0102). The built-in types are `int` and `bool`
([Types](types.md)); the module's structs and enums are types too.
