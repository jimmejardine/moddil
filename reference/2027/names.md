# Names

**names.resolve.1** A name used as an expression refers to the innermost local binding of that
name in scope ([Blocks](expressions/blocks.md)); if there is none, to the function of that name in
the module. Otherwise it is an error (E0101).

**names.item.1** The functions of a module have distinct names, and so do its tests (E0103). A
function and a test may share a name.

**names.type.1** A type name must name a type (E0102). The built-in types are `int` and `bool`
([Types](types.md)).
