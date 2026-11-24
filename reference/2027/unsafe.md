# Unsafe code

Raw pointers, `unsafe` blocks, and the `core` mod's cells and atomics, which sync cells such as
`Mutex` are written with ([`specs/unsafe.md`](../../specs/unsafe.md)). Everything here was added in
M2 slice M2b.

**type.ptr.1** `*const T` and `*mut T` are **raw pointers** to a `T`. A raw pointer is a value like
an `int`: it is `Copy`, it carries no borrow, and nothing checks what it points to, whether that is
still there, or who else is changing it. A borrow becomes a raw pointer where one is wanted (`&T` a
`*const T`, and `&mut T` either), and a `*mut T` goes where a `*const T` is wanted. `*p` names the
value `p` points to, only in an `unsafe` block (E0218); through a `*const T` it cannot be changed
(E0302).

**expr.unsafe.1** `unsafe("why") { … }` is a block, run as any other, in which raw pointers may be
dereferenced. The text says why what it does is sound; it is required, and an empty one is an
error (E0219). An `unsafe` block does not make the function it is in unsafe to call.

**mod.caps.1** A mod's manifest may list the capabilities it uses under `[mod]`:
`capabilities = ["unsafe"]`. Only `unsafe` is checked yet: an `unsafe` block in a mod that does not
list it, or in a single file, is an error (E0219). The prelude and `core` may use it.

**core.ptr.1** The `core` mod's `ptr` module ([source](core/ptr.moddil)) holds `cast_mut(p)`,
which gives the `*mut T` with the address of the `*const T` `p`.

**core.ptr.2** It also holds `null()` and `null_mut()` (pointers to nothing, C's `NULL`),
`is_null(p)`, `add(p, n)` and `add_mut(p, n)` (the pointer `n` values of its type on, as C's
`p + n`: reading through it is sound only within one array), and `cast(p)` and `cast_mut_to(p)`
(the same address, as a pointer to another type, as C's casts). Native code and the MIR
interpreter run them (added in M3 slice M3c).

**core.cell.1** The `core` mod's `cell` module ([source](core/cell.moddil)) holds `UnsafeCell<T>`,
the one way to change a value behind a shared borrow: `UnsafeCell::new(v)`, `c.get()`, a `*mut T`
to its value even through `&c`, and `c.into_inner()`.

**core.atomic.1** The `core` mod's `atomic` module ([source](core/atomic.moddil)) holds the atomic
operations on a value behind a raw pointer, each with an `Ordering` (`Relaxed`, `Acquire`,
`Release`, `AcqRel`, `SeqCst`): `load`, `store`, `swap`, `compare_exchange` (`Ok` with the old value
if it was the one expected and was replaced, `Err` with it if not) and `fetch_add`. Every runtime
runs one task at a time for now, so each is as written in the source, over plain reads and writes.

## Diagnostics

| Code | Meaning |
|---|---|
| E0218 | A raw pointer dereferenced outside an `unsafe` block |
| E0219 | An `unsafe` block in a mod without the `unsafe` capability, or with no reason given |
