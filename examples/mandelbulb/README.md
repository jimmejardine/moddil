# Mandelbulb flight

Fly through the 3D Mandelbulb fractal, ray-marched on the GPU in the browser. It is the first
showcase to be built ([`specs/examples/mandelbulb.md`](../../specs/examples/mandelbulb.md)).

| Directory | Mod | Contents |
|---|---|---|
| [`src/`](src) | `mandelbulb` | The pure part: `Vec3`, the distance estimator, sphere tracing and shading, and the flying camera, with their tests; plus the `preview` executable, which renders a small frame on the CPU |
| [`web/src/`](web/src) | `mandelbulb-web` | The `#[kernel]` that draws each 8 × 8 tile, one frame of flight, and the `flight` executable: a `main` that runs the frame loop in the browser |
| [`web/app/`](web/app) | (TypeScript) | Later: the Svelte 5 page and overlay, over the bindings that `moddil bind` will generate (not built yet) |

```
moddil check examples/mandelbulb          # the pure part
moddil test examples/mandelbulb
moddil run examples/mandelbulb            # the CPU preview: how many pixels show the surface
moddil test examples/mandelbulb/web       # golden images on the CPU simulator
moddil build --wasm examples/mandelbulb/web -o out/flight.mjs
                                          # then serve out/ and open flight.html (WASD, drag)
```

A page's `?size=WxH` fixes the image's size, and `?frames=N` stops after N frames.

## Status

Since slice S10, numbers ([`specs/roadmap.md`](../../specs/roadmap.md#how-work-is-built)),
the pure core in `src/` checks, its tests pass, and the CPU preview runs (it lights 302 pixels,
the same on every platform); `python tools/ci.py examples` checks all three.

Since slice M2e, `web/` flies in the browser: the kernel is lowered to WGSL and runs on WebGPU, a
thread per pixel since M2g, in colour with ambient occlusion and soft shadows, and headless
Chrome checks its first frame against the simulator's
([`specs/examples/mandelbulb.md`: As built](../../specs/examples/mandelbulb.md#as-built-m2e)). Still
missing: `#[export]` and `moddil bind` (so the Svelte app in `web/app/`), and `scope`'s launches.
