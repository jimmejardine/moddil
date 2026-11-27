# The allocator in scope

```rust
struct Counting { n: UnsafeCell<int> }

impl Alloc for Counting {
    fn alloc(&self, size: int, align: int) -> *mut u8 with fail(OutOfMemory) {
        atomic::fetch_add(self.n.get(), 1, Ordering::SeqCst);
        System.alloc(size, align)?
    }
    fn free(&self, p: *mut u8, size: int, align: int) {
        System.free(p, size, align)
    }
}

let c = Counting { n: UnsafeCell::new(0) };
with Alloc = &c {                       // what the block makes comes from `c`
    let mut v: Vec<int> = Vec::new();
    v.push(1);
}
```

Memory comes from an **allocator**. Where none is installed it comes from the default one, which
`System` stands for. `with Alloc = a { … }` installs `a` for the block, as a handler is installed
([Effects](effects.md)), and the storage the block makes comes from it: there are no
`Vec::new_in` constructors.

**alloc.trait.1** An allocator is a value whose type implements the prelude's `Alloc`
([`alloc.moddil`](prelude/alloc.moddil)): `alloc(&self, size, align)` gives `size` bytes aligned
to `align`, or fails with `OutOfMemory`; `free(&self, p, size, align)` takes back what `alloc`
gave, with the same size and alignment. Implementing it needs raw pointers, and so the `unsafe`
capability ([Unsafe code](unsafe.md)).

**alloc.system.1** `System` is the default allocator as a value: an allocator of its own may take
its memory from it.

**alloc.with.1** `with Alloc = a { body }` installs `a` (a borrow of an allocator, or one the
block then owns) for the block: the innermost installed is the one in scope. It is uninstalled
however the block is left: at its end, by `break`, `continue` or `return`, with an error, or
unwinding.

**alloc.make.1** Storage is made from the allocator in scope: a `Buf`'s, and so a `Vec`'s, a
`String`'s, a `Box`'s and an `Rc`'s, made by `Buf::with_capacity` or `Buf::from_array`.

**alloc.keep.1** A buffer keeps the allocator it was made with. Growing it, shrinking it and
freeing it go back to that allocator, wherever that happens: a `Vec` made outside a `with Alloc`
block and grown inside it stays in the default allocator's memory.

**alloc.task.1** A task starts with the allocator in scope where it is started: a spawned task,
and the body of a `scope`.

**alloc.self.1** While an allocator's own `alloc` or `free` runs, the default allocator is in
scope: what the allocator itself makes, a list of its blocks say, does not come from it.

**alloc.oom.1** Storage an allocator has no room for traps, where it is asked for.
