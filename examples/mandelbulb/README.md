# Mandelbulb flight

Fly through the 3D Mandelbulb fractal, ray-marched on the GPU in the browser. It is the first
showcase to be built ([`specs/examples/mandelbulb.md`](../../specs/examples/mandelbulb.md)).

| Directory | Mod | Contents |
|---|---|---|
| [`src/`](src) | `mandelbulb` | The pure part: `Vec3`, the distance estimator, sphere tracing and shading, and the flying camera, with their tests; plus the `preview` executable, which renders a small frame on the CPU |
| [`web/src/`](web/src) | `mandelbulb-web` | The `#[kernel]` that draws each 8 × 8 tile, and the exported `Flight` type the page drives once per frame |
| [`web/app/`](web/app) | (TypeScript) | The Svelte 5 page: canvas, input, frame loop and overlay, over the bindings that `moddil bind` generates |

```
moddil check examples/mandelbulb          # the pure part
moddil test examples/mandelbulb
moddil run examples/mandelbulb            # the CPU preview: how many pixels show the surface
moddil check examples/mandelbulb/web      # the kernel and the page's API
```

## Status

Both mods parse. With the compiler at slice S6, `moddil check` stops at features that arrive in
slice S10, numbers ([`specs/roadmap.md`](../../specs/roadmap.md#m0-the-v0-language-in-slices)), and,
for `web/`, in M2:

| Missing | Used for | Where it is specified |
|---|---|---|
| `f32` (and float literals and arithmetic) | Everything geometric | [`numerics.md`: Floats](../../specs/numerics.md#floats) |
| Float methods: `sqrt`, `sin`, `cos`, `tan`, `acos`, `atan2`, `powf`, `ln`, `max`, `clamp` | The estimator, the camera | [`numerics.md`: Floats](../../specs/numerics.md#floats) |
| `const` items | `MAX_PITCH`, `WIDTH`, `HEIGHT`, `BUDGET` | [`comptime.md`](../../specs/comptime.md) |
| `int` to `f32`: `to::<f32>()` | Pixel coordinates | [`numerics.md`: Conversions](../../specs/numerics.md#conversions) |
| `#[kernel]`, `#[export]`, `scope` and launches, the `Gpu` effect, the `gpu` mod | `web/` only | [`accelerators.md`](../../specs/accelerators.md), [`ui.md`](../../specs/ui.md), M2 |

With `f32` replaced by `int`, the only remaining errors in `src/` are the float functions and the
`const` items, so the rest (structs, operator traits, enums, `match`, loops, modules, `pub use`,
`?!`, tests) already checks.
