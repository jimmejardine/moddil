# GPU kernels

Kernels, and the `core` mod's `gpu` module ([source](core/gpu.moddil)): what runs a kernel on a GPU.
The standard `gpu` mod builds images, tiles, the `Gpu` effect and `launch` over it, and runs a
kernel on the CPU simulator where there is no GPU ([`specs/accelerators.md`](../../specs/accelerators.md)).
Added in M2 slice M2e.

**gpu.kernel.1** A function marked `#[kernel]` is a GPU kernel: one instance draws one tile of an
image. It is a free function taking its parameters as a shared borrow, `&P`, then its tile, `&mut
Tile`, and giving nothing. It lists no effects and its body needs none: no handlers, no error, no
suspension or spawning (the kernel effect set). Otherwise it is an error (E0221). A kernel is
ordinary Moddil, so it is called as any function is (the CPU simulator does), and named as a value
it is a closure that calls it.

**gpu.1** `core::gpu` runs kernels. `source(&k)` is the WGSL of kernel `k` (named as a value), made
when the program is compiled; `bytes(&v)` is a value's bytes, in Moddil's own layout, which the
WGSL reads its parameters in; `dispatch(source, params, width, height, out, present)` runs a kernel
over a `width` by `height` image, one instance per 8 x 8 tile, writes its pixels (RGBA8, red
lowest, row by row) into `out`, shows them if `present`, and gives whether it could (it suspends
until it has); `frame(out)` waits for the next animation frame and writes its 8 words: the
microseconds since the last one, the image's width and height, the keys held, the mouse's and the
wheel's movement since, and 1 if the program is to stop. A run of `main` in a browser has the GPU
(WebGPU) and the window; elsewhere a dispatch gives false, and a frame asks to stop. On the GPU,
`int` is 32 bits wide, and a kernel's traps (overflow, a failed `?!`) are not checked.

**gpu.cap.1** A mod names `core::gpu` only if its manifest lists the `os` capability (E0220), as for
`core::os` (**os.cap.1**).

**gpu.test.1** Under `moddil test` there is no GPU and no window: `dispatch` gives false and `frame`
asks to stop, so a test runs kernels on the CPU simulator and its images are the same on every
machine.

## Diagnostics

| Code | Meaning |
|---|---|
| E0221 | A `#[kernel]` that is not `fn k(params: &P, out: &mut Tile)`, or that needs handlers, can fail, suspends or spawns |
