# fumoca, a Monte Carlo spreadsheet in the browser

## Overview

[fumoca](https://fumoca.com) ("Full Monte Carlo",
[source](https://github.com/jimmejardine/fumoca), owned by the Moddil author) is a spreadsheet for
casual users in which any cell can hold a distribution (`NORMAL(100, 15)`, `PERT(…)`) in place of a
number. Each dependent cell accumulates its own distribution and stays up to date as the user
types. There is no run button. Everything runs on the user's machine; there is no server.

Today it is TypeScript in five packages:

| Package | Role |
|---|---|
| `engine` | Formula parser, IR and evaluator; pure, no DOM |
| `sim` | Worker pool, iteration scheduler, per-cell settling |
| `gpu` | Compiles the whole workbook to one WGSL compute kernel; WebGPU runtime |
| `storage` | Storage-provider interface; local files and IndexedDB |
| `app` | React 19 and Mantine UI, RevoGrid, ECharts |

This example rebuilds fumoca as it would be written today from scratch: `engine`, `sim` and `gpu`
become **one Moddil core**, and the UI is rewritten in **Svelte 5**. The storage providers stay in
TypeScript.

## The pitch

- **The state of the art for a fast web app:** a thin Svelte 5 UI with fine-grained reactivity
  over a Moddil core compiled to WASM, which does all the work off the main thread. Three
  TypeScript packages become one Moddil mod behind generated TypeScript types. It is the template
  that `moddil new --template web` starts from ([`ui.md`: The browser](../ui.md#the-browser)).
- **A billion iterations in a browser tab,** on every CPU core and the GPU at once, with the page
  never blocking.
- **One source of truth for the maths.** The random number generator, the samplers and the
  statistics are ordinary Moddil functions. The CPU workers call them, and the GPU kernel calls the
  same functions. Today fumoca keeps a TypeScript copy and a WGSL copy in step by hand.
- **Bugs caught by construction.** The CPU and GPU engines cross-check each other cell by cell, and
  any disagreement can be replayed exactly from its seed and iteration number.

## Topology

```
 ┌──────────────── main thread ────────────────┐
 │ Svelte 5 UI (runes)                          │
 │ storage providers       (TypeScript)         │
 └──────────────────┬──────────────────────────┘
                    │ generated TS bindings, postMessage (transferables)
                    ▼
 ┌──────────── engine worker: Moddil core ─────┐
 │ parse → IR → schedule → settle → snapshots   │
 │        │                       │             │
 │        ▼                       ▼             │
 │  CPU workers (Moddil)    WebGPU kernel       │
 │  f64, batches of 1,000   f32, batches of 4M  │
 └──────────────────────────────────────────────┘
```

| Part | Language | Notes |
|---|---|---|
| UI | TypeScript, Svelte 5 | Renders compact snapshots (mean, SD, histogram); calls `edit`, `open`, `save` on the core |
| Core | Moddil, `wasm32-browser` | Runs in the engine Web Worker: `Js` but no `Dom` ([`ui.md`: The browser](../ui.md#the-browser)) |
| CPU workers | Moddil | `navigator.hardwareConcurrency − 1` workers, each with its own copy of the compiled model and its own RNG streams |
| GPU | Moddil kernels on WebGPU | Each GPU thread runs one complete iteration of the model |

fumoca does not use `SharedArrayBuffer`, so it needs no COOP/COEP headers. The Moddil core keeps
that property: workers exchange transferable buffers.

## Key flows

1. **Edit:** the user types `=B2 * NORMAL(1.05, 0.1)`. After a 500 ms debounce the core reparses the
   cell, updates the dependency graph, resets the cell and its dependents, and restarts their
   simulation.
2. **Simulate:** batches start at 8,192 iterations and double. Workers fold samples into running
   per-cell statistics (Welford, merged with Chan's formula) and a streaming histogram. The UI
   receives a snapshot about five times a second.
3. **Settle:** each group of connected cells stops when every uncertain cell in it has settled
   (at least 1,000 samples, a standard error below 0.5% of the SD, and stable P5 and P95).
4. **Cross-check:** the same iteration index gives the same draws on the CPU and on the GPU. A
   cell where they disagree beyond the f32 tolerance is flagged, with a divergent iteration that
   can be replayed.
5. **Scenarios and sensitivity:** Cartesian products of alternative inputs, run with common random
   numbers, and tornado and spider charts from ±h runs.

## Moddil features this stresses

- **Generated TypeScript bindings and `.d.ts` types** for a core in a Web Worker, marshalled over
  `postMessage` ([`ui.md`: Generated bindings](../ui.md#generated-bindings)). The UI uses them
  through the Svelte adapter in `moddil/web` ([`ui.md`: The browser](../ui.md#the-browser)).
- **Patterns the UI showcases:**
  - each snapshot updates only the cells that changed: per-cell `$state`, with `$derived` for the
    formatted values, so a refresh five times a second never re-renders the whole grid;
  - large results (histograms, samples) arrive as transferred typed arrays, held with
    `$state.raw` and drawn straight to a canvas, with no copying into JavaScript objects;
  - a virtualised grid renders only the visible cells, whatever the workbook's size;
  - the page itself does no computation, so input stays responsive during a billion-iteration
    run.
- **One function, CPU and GPU.** The counter-based RNG is pure, so the same function runs in a
  worker and inside a kernel ([`accelerators.md`: Three levels, one language](../accelerators.md#three-levels-one-language)):

  ```rust
  /// One step of the lowbias32 hash. Wrapping multiplication is the intended meaning.
  pub fn lowbias32(x: u32) -> u32 {
      let x = (x ^ (x >> 16)) *% 0x7feb352d;
      let x = (x ^ (x >> 15)) *% 0x846ca68b;
      x ^ (x >> 16)
  }

  /// A uniform draw in [0, 1) for (seed, iteration, stream, k); exact in f32.
  pub fn uniform(seed: u32, iter: u32, stream: u32, k: u32) -> f32 { … }
  ```

  The wrapping operator `*%` is explicit, because plain `*` traps on overflow
  ([`numerics.md`: Overflow traps](../numerics.md#overflow-traps)).
- **Structured concurrency across workers.** A batch is a scope; each worker's share is a child
  task borrowing the compiled model; the scope's end is the merge
  ([`concurrency.md`: Tasks and borrowing](../concurrency.md#tasks-and-borrowing)). A GPU launch is
  a child task in the same scope
  ([`accelerators.md`: Launches are structured tasks](../accelerators.md#launches-are-structured-tasks)).
- **Precision in the types:** the CPU engine is `f64`, the GPU engine `f32`. The host merges GPU
  partials in `f64`. Mixing them needs an explicit conversion
  ([`numerics.md`: Conversions](../numerics.md#conversions)).
- **Determinism.** Results depend only on the seed and the iteration indices, never on the number
  of workers or on scheduling ([`determinism.md`: Randomness](../determinism.md#randomness)). The
  cross-check's tolerance compares across vendor GPU arithmetic, so it performs `nondet`
  ([`determinism.md`: The `nondet` effect](../determinism.md#the-nondet-effect)).
- **The CPU simulator for kernels** runs the GPU path in CI, without a GPU
  ([`accelerators.md`: Runtimes and backends](../accelerators.md#runtimes-and-backends)).
- **Spreadsheet errors as values.** `#DIV/0!`, `#NUM!` and `#CIRC!` are an enum carried through
  evaluation, not traps. Integer overflow inside a formula becomes `#NUM!` through checked
  arithmetic.
- **Kernels built at runtime.** fumoca compiles each workbook into its own kernel, but Moddil
  kernels are compile-time code. How it is done is specified in
  [`accelerators.md`: Kernels built at run time](../accelerators.md#kernels-built-at-run-time).
- **Binary size and start-up time** of the core are release metrics
  ([`ui.md`: The browser](../ui.md#the-browser)).

## Success criteria

Initial targets, to be refined:
- The same results as the TypeScript engine on fumoca's test workbooks and on the bundled Tesla
  valuation model, iteration for iteration, for the same seed.
- 10M iterations of a workbook with 70 outputs in at most 0.6 s on the GPU, today's figure.
- No main-thread long task during a 1B-iteration GPU run alongside a 100M-iteration CPU run.
- The CPU engine is faster than the TypeScript one on every test workbook.
- The main thread's busiest frame during a full-speed run stays within a 60 fps budget, measured
  in Playwright.
- A download-size budget for the core, set once the first build exists.

## Mods used

- Standard: `moddil/web` (`Js`, the WebGPU bindings, the Svelte adapter), `moddil/gpu`,
  `moddil/runtime-wasm`, `moddil/serde` (the `.fumoca` JSON format), `moddil/math`, `moddil/time`,
  `moddil/test`, `moddil/prop` (CPU and GPU agreement as a property).
- New: `fumoca/formula` (parser and IR), `fumoca/sim` (scheduler and settling), `fumoca/gpu`
  (kernels and reductions).

## Interop and non-Moddil parts

- The Svelte 5 UI, in TypeScript. Today's React-only parts (Mantine) are replaced; the grid, the
  docking layout and the charts use framework-agnostic libraries or Svelte-native ones.
- Storage providers stay TypeScript, including the closed-source cloud provider used by the
  fumoca.com build. The core sees the model as data and never touches storage.
- The `.fumoca` file format is unchanged, so files move freely between the TypeScript and Moddil
  builds during the migration.
- **`engine`, `sim` and `gpu` are replaced at once**, as the one Moddil core they become: no
  generated bindings are written for an interim TypeScript `sim` and `gpu`.
- **The UI libraries are fumoca's choice**, not this example's. The grid and the charts draw to a canvas from the core's typed arrays, rather than through DOM
  components, so large results need no copying into the DOM.
