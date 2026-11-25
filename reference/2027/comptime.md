# Compile-time evaluation

Compile-time code is ordinary Moddil that the compiler runs: a const item's value, a `const fn`
called from one, and a `const { }` block.

**comptime.const.1** `const NAME: T = e;` declares a const item, of the type written; `pub`
exports it as for other items ([Modules](modules.md)). `e`, its initialiser, is compile-time code,
and `NAME` used as an expression is its value. The initialiser runs once, when compiling.

**comptime.const-fn.1** `const fn` declares a function that compile-time code may call. It is an
ordinary function otherwise, callable at run time too.

**comptime.const-fn.2** Compile-time code (a const's initialiser, the body of a `const fn`, a
`const { }` block) may call the functions of the program's files only if they are `const fn`s
(E1001), and the prelude's functions. A const's initialiser and a `const fn` have no effect and no
error (E1002).

**comptime.block.1** `const { e }` is an expression whose value is `e`'s, and `e` is compile-time
code: it names no local, type parameter or const parameter of the body it is in.

**comptime.block.2** In a test, or in a function without type parameters (its own or its
impl's, and not a trait's) that is not a const item, a `const { }` block is evaluated once, when
compiling, and its value is static data, as a const's is (**comptime.static.1**): borrowed in
place (`&const { … }`, a method called on it, indexing it), or copied if its type is `Copy`. One
whose type is not `Copy`, used by value, is still evaluated where it is, each time, so the value
it gives is owned. A block that traps or runs out of steps when compiling is an error there
(E1004), as is one whose value cannot be data (E1008).

**comptime.static.1** A const's value is static data: `&NAME` borrows nothing and may be returned
from a function with no borrowed parameter ([Borrows](borrows.md)). So a const's type holds no
borrow other than shared ones of static data (E1003, **comptime.static.3**). A const without type parameters (its own or its impl's, and
not a trait's) is evaluated once, when compiling, and its value, with what its `Vec`s, `String`s,
`Box`es and `Rc`s hold, is written into the program as data that is never freed: the code does
not run its initialiser again.

**comptime.static.2** Such a const is borrowed, or copied if its type is `Copy`: `&NAME`,
`NAME.method()` (borrowing it), `NAME[i]` and `NAME.field` read it in place. Using one whose type
is not `Copy` by value, `let v = NAME`, would move static data, and is an error (E1009); `clone`
gives an owned copy. A value that cannot be data (one holding a closure, a `dyn` value, or a C
pointer or function pointer) is an error at the const (E1008).
An `Rc` in a const keeps its own count, so it is never freed.

**comptime.static.3** A const's type may hold shared borrows (`&T`, `&[T]`), of values that hold
no other borrows but such ones; what they borrow is static data with it: another const
(`const SLICE: &[int] = &DATA;`), or a value its initialiser makes (`const LIT: &[int] =
&[4, 5];`), which is kept rather than dropped. A `&mut`, or a view, is still an error (E1003).

**comptime.eval.1** Evaluating a const when compiling that traps, or takes more than 10,000,000
steps, is an error at the const (E1004).

**comptime.tag.1** A **tagged literal** `name"…"` (or a `name"""` block) whose tag is not `fmt`
calls `name`, a `const fn` marked `#[tag]` in scope taking a `&str`, with the literal's text as
written: as a raw string's, its `\` kept (a block dedented as a raw block is), and with no holes.
The call is a `const { }` block (**comptime.block.2**): computed when compiling, so its value is
static data, and an error in it is a compile error. A tag that is not such a function in scope is
an error (E1011). The `moddil/regex` mod's `re` is one: `re"^\d+$"` is a `Regex` compiled when
compiling.

**comptime.error.1** `compile_error(message, start, end)`, a prelude function, ends compile-time
code with an error, reported when compiling with `message` (E1010): at bytes `start` to `end` of
the text of the tagged literal being evaluated, or at the const or block. Called when the program
runs, it traps (`unreachable`).

**comptime.eval.2** `#[fuel(n)]` on a const item, or on a statement (a `let` or an expression
statement), gives the const, or each `const { }` block in the statement, `n` steps in place of
10,000,000: more for a costly table, fewer to bound one. An evaluation that runs out of them is
E1004, as before.

## Derives written in Moddil

A trait can name a derive function, written in Moddil, whose body is a template: an impl, written
once, that `#[derive]` instantiates for each struct it is written on.

**comptime.type.1** `type` is the type of types. For now it is only the type of a derive
function's parameter (E1006).

**comptime.derive.1** A derive function is written `const fn f(t: type) -> Impl<Tr> { impl Tr
for t { … } }`: one parameter of type `type`, the result `Impl<Tr>`, and a body that is one impl
of `Tr` for `t`, its template (E1006). It is never called (E1006): `#[derive(Tr)]` instantiates it
([attr.derive-by.1](attributes.md)).

**comptime.template.1** Instantiating a derive function's template for a struct `S` makes the
impl the template is, with these rules:

- The template's names mean what they mean where the derive function is written, not where `S` is.
- Both `t` (the parameter) and `Self` name `S`.
- Its methods may read and make `S`'s fields, even private ones: `#[derive]` is `S`'s opt-in.
- The impl is then checked like one written by hand, and an error in it is reported in the
  template.

For a generic struct, the impl bounds each of its type parameters by the trait (`impl<T: Tr> Tr for
S<T>`), as the built-in derives do, so it applies only where every type argument implements the
trait. An enum is reflected by its variants (**comptime.template.variant.1**). In a template, `t.name`
is the name of the type it is instantiated for, as its declaration writes it, as text.

**comptime.template.inline.1** In a template, `inline for f in fields(t) { … }` is its block once
for each of `S`'s fields, in the order they are declared, with `f` naming that field. It is a
statement, and it may also be among a struct literal's fields: `t { inline for f in fields(t) {
[f]: e, } }` gives one field of the literal for each field of `S`. Elsewhere, or written another
way, it is an error (E1007).

**comptime.template.field.1** In an `inline for`, `f` is a reflected field, used only these ways
(E1007):

- `f.name` is the field's name, as text;
- `x.[f]` is the field of `x`, and `[f]: e` gives it in a struct literal;
- `f.is_refined()` says whether the field's type is a refinement ([Refinements](refinements.md));
- `f.new(e)` is that refinement's `X::new(e)`, for a refined field only.

**comptime.template.if.1** In a template, `if const c { a } else { b }` is `a` if `c` holds and `b`
if not (`()` without `else`). Only the branch taken is part of the impl, so only it is checked. `c`
is reflection: `f.is_refined()`, `f.name == "…"` or `!=`, `t.is_enum()` (whether the template is
instantiated for an enum), `t.is_unit()` and `t.is_positional()` (whether a struct `t` has no
fields, or positional ones), the same of a variant `v`, `v.is_default()` (whether `v` is marked
`#[default]`), `v == w` or `!=` of two reflected variants, `true` and `false`, joined by `!`, `&&`
and `||` (E1007). Outside a template, `if const` is not supported yet (I0001).

**comptime.template.variant.1** In a template instantiated for an enum, `inline for v in
variants(t) { … }` is its block once for each of the enum's variants, in the order they are
declared, with `v` naming that variant. It is a statement. `v` is a reflected variant, used only
these ways (E1007):

- `v.name` is the variant's name, as text, and `v.index` its place in that order, from 0, as an
  `int`;
- `v.is(x)`, for `x` a `t` or a borrow of one, says whether `x` is that variant;
- `inline for f in fields(v) { … }` is its block once for each of the variant's fields (named
  `0`, `1`, … if they are positional), with `f` a reflected field (**comptime.template.field.1**);
- `x.[f]`, for such an `f` and `x` a borrow of a `t`, is that field of the value `x` points to,
  if it is variant `v`, and traps if it is another variant;
- `v { inline for f in fields(v) { [f]: e, } }` is that variant's value, whatever its shape, and
  `v.make()` the value of a unit variant (a variant with fields is E1007).

An enum has no `fields(t)`, and `t { … }` is not one of its values; a struct, a tuple or a
built-in type has no `variants(t)` (E1007).

**comptime.derive.for.1** `#[derive_for(T1, T2, …)]` before a derive function instantiates its
template once for each built-in type listed: an impl of the trait its `Impl<Tr>` names, for each
`Ti`, with `t` and `Self` naming `Ti`, under the rules of **comptime.template.1**. The impls and
the types they are for are written in one place, with no `#[derive]` on the types. A function with
`#[derive_for]` is not named by other code. `tuples(m..=n)` in the list stands for the tuples of
`m` to `n` elements, `(A, B, …)`, each impl bounding every element type by the trait
(`impl<A: Tr, B: Tr> Tr for (A, B)`). A tuple's elements are its fields, named `0`, `1`, … in
order, for `inline for`, `f.name` and `x.[f]`, and `t { [f]: e, … }` makes the tuple, each element
given once (E1007). Another built-in type has no fields, so a template instantiated for one uses
no field reflection (E1007). The prelude writes the impls of the sized integers and the floats
this way ([`prelude/numbers.moddil`](prelude/numbers.moddil)), and tuples' `Clone`, `Debug` and
`Hash`. Outside the prelude, `#[derive_for]` is not supported yet (I0001).
