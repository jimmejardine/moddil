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
code: it names no local, type parameter or const parameter of the body it is in. For now it is
evaluated where it is, each time.

**comptime.static.1** A const's value is static data: `&NAME` borrows nothing and may be returned
from a function with no borrowed parameter ([Borrows](borrows.md)). So a const's type holds no
borrow other than of text (E1003).

**comptime.eval.1** Evaluating a const when compiling that traps, or takes more than 10,000,000
steps, is an error at the const (E1004).

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
trait. For now an enum deriving with a template is not supported yet (I0001).

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
is reflection: `f.is_refined()`, `f.name == "…"` or `!=`, `true` and `false`, joined by `!`, `&&`
and `||` (E1007). Outside a template, `if const` is not supported yet (I0001).

**comptime.derive.for.1** `#[derive_for(T1, T2, …)]` before a derive function instantiates its
template once for each built-in type listed: an impl of the trait its `Impl<Tr>` names, for each
`Ti`, with `t` and `Self` naming `Ti`, under the rules of **comptime.template.1**. The impls and
the types they are for are written in one place, with no `#[derive]` on the types. A function with
`#[derive_for]` is not named by other code. A built-in type has no fields, so a template
instantiated for one uses no field reflection (`inline for`, `t { … }`, E1007). The prelude
writes the impls of the sized integers and the floats this way
([`prelude/numbers.moddil`](prelude/numbers.moddil)). Outside the prelude, `#[derive_for]` is
not supported yet (I0001).
