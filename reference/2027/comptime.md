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
