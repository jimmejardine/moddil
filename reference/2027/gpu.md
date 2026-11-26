# GPU kernels

Kernels, and the `core` mod's `gpu` module ([source](core/gpu.moddil)): what runs a kernel on a GPU.
The standard `gpu` mod builds images, tiles (with `Tile::each`, a pixel per GPU thread), the `Gpu`
effect, `launch` and `present` over it, and runs a kernel on the CPU simulator where there is no
GPU ([`specs/accelerators.md`](../../specs/accelerators.md)). Added in M2 slice M2e; showing
without read-back, `show` and a thread per pixel in M2g.

**gpu.kernel.1** A function marked `#[kernel]` is a GPU kernel: one instance draws one tile of an
image. It is a free function taking its parameters as a shared borrow, `&P`, then its tile, `&mut
Tile`, and giving nothing. It lists no effects and its body needs none: no handlers, no error, no
suspension or spawning (the kernel effect set). Otherwise it is an error (E0221). A kernel is
ordinary Moddil, so it is called as any function is (the CPU simulator does), and named as a value
it is a closure that calls it. On a GPU a tile is a workgroup of 8 x 8 threads: every thread runs
the kernel's body, which is pure, so each computes the same values, and the closure given to
`Tile::each` runs once per thread, for that thread's pixel.

**gpu.kernel.2** A kernel, and what it calls, holds only what a GPU has: numbers but `f64`, the
64-bit and the 128-bit ones, `bool` and `char`, structs, tuples and enums of them, the closure given
to `Tile::each`, and arrays of them. A slice is of an array the kernel has: its own, or one in its
parameters (`&p.weights[2..]`); its length and its elements are read as on the CPU. Anything else
in a kernel (a `Vec` or a slice of the CPU's memory, text, `dyn`, a closure kept in a value) is an
error (E0221) at the kernel or at the statement it is in, reported when the program is checked.

**gpu.1** `core::gpu` runs kernels. `source(&k)` is the WGSL of kernel `k` (named as a value), made
when the program is compiled; `bytes(&v)` is a value's bytes, in Moddil's own layout, which the
WGSL reads its parameters in; `dispatch(source, params, width, height, out, mode)` runs a kernel
over a `width` by `height` image, a workgroup per 8 x 8 tile, and gives whether it could (it
suspends until it has). `mode` is the sum of what to do with the image: 1 shows it in the page
(on the GPU, with no copy back), 2 reads its pixels (RGBA8, red lowest, row by row) back into
`out`. `show(pixels, width, height)` shows an image the CPU drew. `frame(out)` waits for the next
animation frame and writes its 8 words: the microseconds since the last one, the window's width
and height, the keys held, the mouse's and the wheel's movement since, and 1 if the program is to
stop. A run of `main` in a browser has the GPU (WebGPU) and the window; elsewhere a dispatch gives
false, `show` does nothing, and a frame asks to stop. On the GPU, `int` is 32 bits wide, and a
kernel's traps (overflow, a failed `?!`) are not checked.

**gpu.cap.1** A mod names `core::gpu` only if its manifest lists the `os` capability (E0220), as for
`core::os` (**os.cap.1**).

**gpu.test.1** Under `moddil test` there is no GPU and no window: `dispatch` gives false, `show`
does nothing and `frame` asks to stop, so a test runs kernels on the CPU simulator and its images
are the same on every machine.

## Diagnostics

| Code | Meaning |
|---|---|
| E0221 | A `#[kernel]` that is not `fn k(params: &P, out: &mut Tile)`, or that needs handlers, can fail, suspends or spawns, or holds what a GPU does not have |
