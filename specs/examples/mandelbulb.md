# Mandelbulb flight

## Overview

A browser page in which the user flies through the 3D
[Mandelbulb](https://en.wikipedia.org/wiki/Mandelbulb) fractal in real time. A full-window canvas
ray-marches the power-8 Mandelbulb every frame on the GPU, and the controls are those of a flight
game:

| Input | Action |
|---|---|
| Mouse, with pointer lock | Look |
| W, A, S, D | Fly forward, left, back, right |
| Q, E | Roll |
| Scroll wheel | Speed |
| Touch, gamepad | Later |

A small Svelte 5 overlay shows the frame rate and the camera position. It offers sliders for the
power, the iteration count and the quality, and a "copy link" button that writes the camera into
the URL, so a view can be shared.

It is the **simplest showcase**, and the first program built for the browser and the GPU
([`roadmap.md`: Milestones](../roadmap.md#milestones)). Its kernel is fixed, so it needs nothing
beyond what [`accelerators.md`](../accelerators.md) already decides.

## The pitch

- **The smallest complete program that runs Moddil on the GPU in a browser:** one mod, one kernel
  and a page.
- **No shader language.** The kernel is ordinary Moddil, checked by the same compiler as the rest of
  the program, and lowered to WGSL for you.
- **One function, CPU and GPU.** The distance estimator that draws each pixel also steers the camera
  on the CPU.
- **Tested without a GPU.** The same kernel renders a frame on the CPU simulator, bit for bit, so CI
  compares golden images on machines with no graphics card.
- **Small enough to embed** on moddil.com's front page ([`web.md`](../web.md)).

## Topology

```
 ┌──────────────────────── browser tab ────────────────────────┐
 │ Svelte 5 overlay: fps, sliders, "copy link"                  │
 │        │ generated TS bindings                               │
 │        ▼                                                     │
 │ Moddil core (WASM, main thread)                              │
 │   input → camera → frame loop (requestAnimationFrame)        │
 │        │ launch                                              │
 │        ▼                                                     │
 │ #[kernel] render: one instance per 8×8 tile → storage texture│
 │        → blit to the canvas                                  │
 └──────────────────────────────────────────────────────────────┘
```

| Part | Language | Notes |
|---|---|---|
| Overlay | TypeScript, Svelte 5 | UI only, through the Svelte adapter in `moddil/web` ([`ui.md`: The browser](../ui.md#the-browser)) |
| Core | Moddil, `wasm32-browser` | Input, camera physics, the frame loop; on the main thread, because there is little CPU work |
| Kernel | Moddil `#[kernel]`, lowered to WGSL | Ray marching, shading and ambient occlusion |

## Key flows

1. **Start-up:** request a WebGPU adapter, compile the pipeline once, size the canvas to the
   window, and restore the camera from the URL if there is one.
2. **Each frame:** read input, integrate the camera, launch the kernel with its uniforms (camera,
   power, iterations and quality), then present.
3. **Speed scales with distance.** The camera's speed is proportional to the distance estimate at
   its position, so flight feels the same at every scale, and the camera slows to a stop instead of
   passing through the surface. This calls the same `distance` function on the CPU that the kernel
   calls on the GPU.
4. **Adaptive quality:** when frames take longer than the budget, the core lowers the render scale,
   and raises it again when there is room.
5. **Share:** "copy link" writes the camera to the URL hash; opening the link restores the view.

## Moddil features this stresses

- **The `kernel` effect set and tile kernels**: no allocation, no recursion, no I/O, always
  specialised ([`accelerators.md`: Three levels, one language](../accelerators.md#three-levels-one-language)).
- **Pure helpers shared by the CPU and kernels.** `distance`, `normal` and the `Vec3` maths are
  ordinary functions inside the kernel set:

  ```rust
  /// Distance from `p` to the power-`n` Mandelbulb (an estimate, never an overestimate).
  pub fn distance(p: Vec3, n: f32, iterations: int) -> f32 {
      let mut z = p;
      let mut dr = 1.0;
      let mut r = 0.0;
      for _ in 0..iterations {
          r = z.length();
          if r > 2.0 { break }
          let theta = (z.z / r).acos() * n;
          let phi = z.y.atan2(z.x) * n;
          dr = r.powf(n - 1.0) * n * dr + 1.0;
          z = Vec3::from_spherical(r.powf(n), theta, phi) + p;
      }
      0.5 * r.ln() * r / dr
  }

  #[kernel]
  fn render(view: &View, out: &mut Tile<Rgba8, 8, 8>) {      // out: this instance's own tile
      for (x, y) in out.pixels() {
          let ray = view.camera.ray(x, y, view.size);
          out[(x, y)] = shade(march(ray, view), view);         // march calls distance
      }
  }
  ```

  The camera calls `distance(camera.position, …)` on the CPU to set its speed.
- **Floats:** `f32` everywhere, with strict IEEE semantics, so the CPU and GPU agree where rounding
  is specified ([`numerics.md`: Floats](../numerics.md#floats)).
- **A launch is a structured task:** the frame's scope lends `view` and the output texture to the
  launch, and its end is the fence before presenting
  ([`accelerators.md`: Launches are structured tasks](../accelerators.md#launches-are-structured-tasks)).
- **Effects in signatures:**
  - `Dom` for the canvas and the input events;
  - `Js` for `requestAnimationFrame`;
  - `Gpu` for rendering;
  - the camera's physics is pure, so it is tested as plain code
    ([`ui.md`: The browser](../ui.md#the-browser)).
- **Missing GPU features are emulated** with a warning, as the accelerators spec decides
  ([`accelerators.md`: Runtimes and backends](../accelerators.md#runtimes-and-backends)).
- **Testing:**
  - golden images, rendered by the kernel on the CPU simulator;
  - input sequences, played through the simulated DOM handler, checking where the camera ends up;
  - both run deterministically in CI ([`determinism.md`](../determinism.md)).
- **Binary size and start-up time** are release metrics ([`ui.md`: The browser](../ui.md#the-browser)).

## Success criteria

Initial targets, to be refined:
- 60 fps at 1080p on an integrated laptop GPU at the default quality.
- A compressed WASM module under 200 KB (to be confirmed against the first build).
- The first frame within 1 s of page load, with a warm cache.
- Golden images bit-exact on the CPU simulator, and within a stated tolerance on real GPUs.
- Without WebGPU, a clear message, and the kernel on a CPU pool at low resolution.

## Mods used

- Standard: `moddil/web` (`Dom`, `Js`, the WebGPU bindings, the Svelte adapter), `moddil/gpu`,
  `moddil/runtime-wasm`, `moddil/math`, `moddil/test`.
- New: `mandelbulb/render` (the estimator, the kernel and the camera).

## Interop and non-Moddil parts

- The Svelte 5 overlay, through the generated TypeScript bindings. Nothing else.

## As built (M2e)

The first build (slice M2e), `examples/mandelbulb/web`:
- **The kernel, `render`, is Moddil**, lowered to WGSL, and drawn on WebGPU, a thread per pixel
  (`Tile::each`); the camera, the estimator and the shading are the pure `mandelbulb` mod, shared
  with the CPU preview.
- **The look (M2g):**
  - colours from the orbit's traps through a cosine palette;
  - ambient occlusion sampled along the normal;
  - soft shadows from the sun;
  - sky light, a rim light at grazing angles, a sky gradient with the sun's glow, distance fog
    and gamma.
- **Speed (M2g):**
  - the power-8 step multiplies the angles by doubling them, with no trigonometric functions,
    and normals take four samples;
  - frames are shown from the GPU with no copy back;
  - buffers and bind groups are made once;
  - the resolution and the steps adapt to the frame time;
  - a corner of the page shows the fps, the image's size and the GPU's time.
- **The program is a `main(gpu: GpuRoot, window: Window)` frame loop**: each animation frame gives
  the input (WASD, the mouse while dragging or with the pointer locked, the wheel) and the time
  since the last, which moves the camera and adapts the quality. The page is the plain one
  `moddil build --wasm` writes, with a full-window canvas.
- **Tested:** golden images on the CPU simulator, everywhere; and headless Chrome on SwiftShader's
  WebGPU draws the first frame on the GPU with each colour channel's sum within 3% of the
  simulator's (it is within 0.01%), then shows two more frames without reading them back.
- **Not yet:**
  - the exported `Flight` that the page drives, `moddil bind`, and the Svelte overlay (sliders,
    fps, "copy link");
  - a message without WebGPU (it falls back to the CPU simulator, slowly);
  - the success criteria's measurements.

## Open questions

- **Deep zoom:** `f32` limits how close the camera can get to the surface. How far do
  camera-relative coordinates go, and is emulated `f64` in the kernel worth its cost?
- **Native desktop:** the same program in a native window through wgpu would need a windowing mod.
  Is that in scope for this example?
