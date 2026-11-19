# GPUs and Accelerators

> **Status:** complete: decided, with no open questions. Indexed in [`README.md`](README.md) and extends
> [`concurrency.md`](concurrency.md) from CPUs to devices. Snippets are **illustration only**; no
> syntax is committed.

## Summary

| Rule | One line |
|---|---|
| **One language, three levels** | Tensor expressions, tile kernels (the default) and SIMT kernels are all ordinary Moddil, restricted by a compiler-checked `kernel` effect set. |
| **Devices are capability values** | `main` receives a `Devices` capability. Code works with typed device handles, never ambient global indices. |
| **A CPU pool is a device** | A "pseudo-GPU" over a chosen set of cores runs the same kernels, using host memory. |
| **Buffers are branded by device** | `g0.alloc()` returns a `Buffer<T, g0>`. Mixing devices is a compile error; `.erase()` gives a runtime-checked form. |
| **Roles + manifest** | Code asks for roles or requirements; a deployment manifest binds them to hardware. `main` and tests may select devices directly. |
| **Launches are structured tasks** | A launch borrows its inputs (`let`) and outputs (`mut`). The end of the scope is the fence. CPU–GPU races are ruled out by the existing borrowing rules. |
| **Race-free kernels by default** | Disjoint writes and atomics are safe; arbitrary scatter writes need `unsafe`. |
| **Schedules are comptime data** | Algorithm and schedule are separate. Autotuning is an offline tool whose output is embedded. |
| **Runtimes are mods** | The `Gpu` effect is handled by WebGPU, Vulkan, Metal, CUDA, ROCm and CPU-simulator mods. WebGPU comes first, then CUDA. |

## The accelerator spectrum

"Accelerator" covers four quite different kinds of hardware:

| Kind | Examples | How it is programmed today |
|---|---|---|
| **SIMT GPUs** | NVIDIA, AMD, Apple, Intel, mobile; WebGPU in browsers | Kernels: CUDA/HIP, Metal, WGSL/SPIR-V, Slang |
| **Matrix/tensor units** | Tensor Cores, AMD matrix cores, Intel AMX, Arm SME | Tile intrinsics, and increasingly tile-level languages (Triton, CUTLASS/CuTe, Mojo, NVIDIA's tile-based CUDA work) |
| **Graph accelerators** | TPUs; NPUs (Apple Neural Engine, Qualcomm Hexagon, Intel/AMD NPUs) | Not by writing kernels: a dataflow graph is handed to a vendor compiler (XLA/StableHLO, ONNX Runtime, Core ML, OpenVINO) |
| **FPGAs / fixed-function** | FPGA feed handlers, video codecs, crypto engines | Hardware design tools or vendor SDKs, reached through FFI and devices |

**The trend:** kernel programming is moving from *threads* to *tiles*. The programmer says "this
program instance computes this output tile". The compiler handles threads, shared memory, tensor
cores and pipelining. Triton showed this reaches near hand-written performance for most kernels.

## Three levels, one language

| Level | You write | Lowers to | Audience |
|---|---|---|---|
| **1. Tensor expressions** (`moddil/tensor`, [Level 1: tensor expressions](#level-1-tensor-expressions)) | Array maths: `softmax(q @ k.T) @ v` | A graph built at comptime → StableHLO/XLA, ONNX, Core ML, OpenVINO; or generated tile kernels | Most users: ML, simulation, numerics |
| **2. Tile kernels** (default) | One program instance per output tile: load, compute, store tiles | PTX/AMDGPU via LLVM; WGSL/SPIR-V/MSL via naga; CPU SIMD | Kernel authors |
| **3. SIMT kernels** | Threads, scoped shared memory, barriers, subgroup/warp operations | Same backends | Experts; the escape hatch |

**The `kernel` effect set.** Kernel code is ordinary Moddil whose effects the compiler restricts:
- no heap allocation, except **scoped shared memory** sized at comptime;
- no recursion;
- no `Rc`;
- no suspension, I/O or `fail` (faults are traps, [Errors](#errors));
- **always specialised** to concrete types. This is the one exception to dictionary-passing generics
  in debug builds ([Build times](build-times.md)), because GPUs cannot call through dictionaries efficiently.

Ordinary pure helper functions that stay within this set can be called from kernels and from CPU
code alike.

```rust
#[kernel]
fn matmul<const BM: int, const BN: int, const BK: int>(
    a: &Matrix<f16>, b: &Matrix<f16>, c: &mut Tile<f32, BM, BN>,         // c: this instance's own tile
) {
    let mut acc = Tile::<f32, BM, BN>::zeros();
    for k in (0..a.cols).step_by(BK) {
        acc += a.tile(c.row, k, BM, BK) @ b.tile(k, c.col, BK, BN);     // may map to tensor cores
    }
    c.store(acc);
}
```

## Devices and addressing

Device indices are facts about one machine at run time, so they never appear in types or in library
code.

- **Discovery is a capability.** `main` receives `Devices`. A library cannot enumerate GPUs; it only
  uses the device handles its caller passes in. Tests pass CPU-pool or simulated devices.
- **Handles are typed values** with a kind (GPU, CPU pool, NPU), vendor, memory size, feature set
  (f16, subgroups, tensor cores) and a **stable ID** (NVIDIA UUID or PCI bus ID). Raw indices such as
  `cuda:0` are just one selector, and their order is not guaranteed (cf. `CUDA_VISIBLE_DEVICES`).
- **A CPU pool is a device.** A "pseudo-GPU" runs kernels over a chosen set of cores, with host memory,
  so moving data between it and ordinary CPU code is free. The pool owns pinned, NUMA-aware worker
  threads, and the normal work-stealing runtime does not schedule onto those cores, so the two pools
  don't fight.
- **Roles and requirements.** Code asks for what it needs rather than which card:
  `devs.role("physics")`, or `devs.pick(min_mem: 16.GiB, features: [f16])`. A **deployment
  manifest** binds roles to hardware (the same split as [Distributed and multi-tier programs](distribution.md)). `main` and tests may also
  select directly.
- **Device-to-device copies** are always an explicit `.to(device)`. The runtime chooses the path:
  NVLink, PCIe peer-to-peer, or staging through host memory (consumer cards often have no NVLink and
  may not support peer-to-peer).
- **Remote devices** (a GPU on another machine) are reached only through the `Remote` effect
  ([Distributed and multi-tier programs](distribution.md)). Device placement is the bottom rung of the placement ladder: thread, core,
  device, machine, tier.

Example: a workstation with two GPUs and a 128-core CPU.

```rust
fn main(devs: Devices, …) with Gpu, … {
    let g0  = devs.role("physics")?;                   // manifest: physics = "cuda:0"
    let g1  = devs.role("render")?;                    //           render  = "cuda:1"
    let cpu = devs.cpu_pool(cores: 32..128)?;          // pseudo-GPU on 96 cores; 32 left for the runtime

    let mut a = g0.alloc::<f32>(n);                    // Buffer<f32, g0>
    let mut b = g1.alloc::<f32>(n);                    // Buffer<f32, g1>
    let mut c = cpu.alloc::<f32>(n);                   // Buffer<f32, cpu>: host memory
    scope(|s| {
        s.launch(g0,  step, &mut a);                   // the same kernel on three devices
        s.launch(g1,  step, &mut b);
        s.launch(cpu, step, &mut c);
        // s.launch(g0, step, &mut b);                 // ✗ compile error: b belongs to g1
    });
    let a_host = a.to(cpu)?;                           // explicit transfer
}
```

```toml
# deployment manifest (illustrative)
[devices]
physics  = "cuda:0"                  # or a UUID: "GPU-3f1c…"
render   = "cuda:1"
fallback = "cpu:cores=32..128"
```

The same binary runs on this workstation, on a laptop with one GPU (both roles bound to it), and in
CI with only a CPU pool.

**Brands.** `Buffer<T, g0>` is tied to the *value* `g0` (like Scala's path-dependent types, or the
"brand" trick in Rust's GhostCell). Mixing buffers from different devices is a compile error, with no
device indices in the types. When devices are only known dynamically (a `Vec` of devices),
`.erase()` turns a buffer into `Buffer<T, dyn Gpu>`, and the device is checked at launch. A mismatch is a
bug and traps. The brand mechanism is a general type-system feature to define. It could also tie
indices to their arena and DMA buffers to their NIC ring.

## Memory spaces and buffers

| Space | Meaning |
|---|---|
| `Buffer<T, dev>` (branded) / `Buffer<T, dyn Gpu>` (erased) | Device memory |
| `Buffer<T, Host>` | Ordinary host memory; also the memory of a CPU pool |
| `Buffer<T, Shared>` | Unified memory visible to host and device (Apple silicon, some data-centre parts) |
| `Buffer<T, Dma<nic0>>` | NIC DMA memory, already used for zero-copy I/O ([`memory.md`: Zero-copy I/O](memory.md#zero-copy-io)) |

- Buffers are **owned resources**. Dropping one frees it; on a device the drop *enqueues* the free
  behind outstanding work, so it never blocks (drop cannot suspend).
- Transfers are explicit operations. There is no implicit copying and no aliasing across spaces.
- `Shared` memory follows the ordinary borrowing rules, so host and device cannot race on it.

## Launches are structured tasks

A kernel launch is a child task in a structured scope ([`concurrency.md`: Tasks and borrowing](concurrency.md#tasks-and-borrowing)):
- inputs are lent as shared (`let`) borrows, outputs as exclusive (`mut`) borrows;
- while the launch runs, the CPU can read the inputs but cannot write them, and cannot touch the
  outputs at all;
- **the end of the scope is the fence**: waiting for GPU work is just leaving the scope;
- CPU work in the same scope overlaps freely with GPU work.

```rust
scope(|s| {
    s.launch(g0, matmul::<128, 128, 32>, &a, &b, &mut c, grid: c.tiles(128, 128));
    prepare_next_batch(&mut d);                   // runs on the CPU while the GPU works
});                                               // c is readable again here
```

Streams, queues and events are runtime-handler details ([Runtimes and backends](#runtimes-and-backends)). The programmer sees only scopes, which
removes the classic bugs: using a buffer before its kernel finished, freeing it while it is in use,
and forgetting a fence.

## Race freedom inside kernels

- **Tile kernels:** each program instance receives its own **disjoint output tile**: the GPU
  equivalent of `split_at_mut`. Inputs are shared read-only. So tile kernels are race-free by
  construction.
- **SIMT kernels**, safe forms:
  - per-thread disjoint views of the output (each thread writes its own element or slice);
  - atomics;
  - scoped shared memory with barriers, where the compiler checks the barrier structure (e.g. not
    inside divergent control flow).
- **Arbitrary scatter writes** (`out[idx[i]] = …`) cannot be proven race-free. They require an
  `unsafe` block, which is visible in capabilities and review ([Package management and supply-chain security](packages-and-supply-chain.md), [Diffs, code review and version control](semantic-vcs.md)).
- **The CPU-simulator handler includes a race detector**, so unsafe kernels can be checked in tests
  and CI.
- **Deterministic by default:** reductions run in a fixed order and integer atomics are
  order-independent. Float atomics and unordered reductions perform `nondet`, which keeps them out
  of lockstep and replayable code ([`determinism.md`: The `nondet` effect](determinism.md#the-nondet-effect)).

## Level 1: tensor expressions

- `moddil/tensor` provides n-dimensional arrays and operations.
- A tensor function is a pure function over tensors. **Comptime builds its dataflow graph**
  ([`comptime.md`](comptime.md)), so there is none of the runtime tracing that Python frameworks need.
- The graph is optimised (fusion, layout) and then:
  - handed to a graph compiler for TPUs and NPUs (StableHLO/XLA, ONNX Runtime, Core ML, OpenVINO);
    this is the **only** way to reach NPUs, since nobody writes NPU kernels; or
  - lowered to generated tile kernels for GPUs and CPU pools.
- Tensor types live in a mod, not the core language (R2).

## Algorithm and schedule

- A **schedule** (tile sizes, pipeline stages, vector widths, per-target variants) is comptime data
  attached to a kernel or tensor function per target, separate from the algorithm (the Halide idea).
- LLMs and humans can propose schedules without touching the algorithm, and review them separately.
- **Autotuning** must run on real hardware, so it cannot be pure comptime. It is an **offline tool**
  that writes a schedule file; the build `embed`s that file, which is declared and hashed
  ([`comptime.md`: Evaluation model](comptime.md#evaluation-model)). Builds stay pure and reproducible.

## Runtimes and backends

- `Gpu` is an effect. **Handler mods** implement it: WebGPU (via wgpu), Vulkan, Metal, CUDA, ROCm
  and a **CPU simulator**. Libraries work with every handler ([R2](requirements.md#firm-requirements-decided)).
- **Lowering** reuses existing tools, as in [`compiler.md`](compiler.md): naga for
  WGSL/SPIR-V/MSL/HLSL; LLVM NVPTX and AMDGPU for CUDA- and ROCm-class targets; MLIR later if
  tensor-level optimisation needs it.
- **Order:**
  1. **WebGPU first.** It serves the browser (R1) and the game example, runs on every vendor, and
     through wgpu gives portable CI on Vulkan, Metal and DX12.
  2. **CUDA second.** Peak performance and tensor cores, for the HFT research workloads.
- **Feature tiers** (subgroups, f16, tensor cores, shared-memory size) are target capabilities checked
  at compile time. A kernel that needs tensor cores cannot be built for a target without them; a
  schedule can provide a fallback.
- **Linking:** GPU drivers (CUDA's `libcuda`, the Vulkan loader) are installed per machine and loaded
  dynamically, so GPU programs on Linux use a dynamic glibc target with a pinned minimum version;
  everything else in them stays static ([`targets.md`: Dynamic glibc targets](targets.md#dynamic-glibc-targets)).
- The **CPU simulator** runs kernels deterministically on the CPU. It gives GPU-free tests and CI,
  race detection ([Race freedom inside kernels](#race-freedom-inside-kernels)), and lets the same kernels run on servers without GPUs.

## Errors

Following [`error-handling.md`](error-handling.md):
- **Device out of memory, device lost, unsupported feature:** ordinary `fail` errors at allocation,
  launch or join. They must be handled (R3).
- **Faults inside a kernel** (out-of-bounds, unreachable, ÷ 0): class-3 traps, reported at the join
  with the kernel and the program instance that faulted.

## Other accelerators

- **NPUs and TPUs:** only through level 1 ([Level 1: tensor expressions](#level-1-tensor-expressions)).
- **FPGAs and fixed-function engines:** through FFI, the `ffi` capability and owned DMA buffers. Their
  drivers are mods.
- **CPU vector units:** `moddil/simd` for direct use; tile kernels can also lower to them via the CPU
  pool.

## Fit check against the reference examples

| Example | Uses |
|---|---|
| [Multiplayer game](examples/01-multiplayer-game.md) | WebGPU rendering and physics kernels in the browser; the same kernels run on the server through a CPU pool for authoritative simulation |
| [HFT](examples/02-hft-platform.md) | Research Monte Carlo, option pricing and ML on CUDA; tensor expressions for models; FPGA feed handlers through FFI |
| [Web store](examples/03-web-store.md) | None needed |
| [Hashiverse](examples/04-hashiverse.md) | Possibly proof-of-work on a GPU or CPU pool (to evaluate) |

## Resolved questions

Decided in the open-question review:

1. **Autodiff** is a comptime transformation in a mod, `moddil/autodiff`, in the style of Enzyme: it
   derives `grad(f)` for any differentiable function, kernels and loops included, with no compiler
   change.
2. **Multi-GPU collectives** (all-reduce, broadcast) are a library on device handles and scopes,
   `moddil/collective`, using NCCL or RCCL through FFI where available and a portable fallback
   otherwise.
3. **Multi-dimensional slicing is core syntax**, spelled with ranges because `:` is taken by named
   arguments: `a[.., 1..3]`, `a[i, ..]`. Tensor types themselves stay in `moddil/tensor`.
4. **The CPU simulator is bit-exact where rounding is specified** (strict IEEE add, multiply and FMA,
   correctly rounded `moddil/math`). Where vendor approximations are allowed (`math::fast`,
   tensor-core accumulation order) it documents a tolerance; those paths perform `nondet`
   ([`determinism.md`: The `nondet` effect](determinism.md#the-nondet-effect)).
5. **Device features are declared, emulated when missing, and warned about.** A kernel declares
   what it needs (`#[requires(f16, subgroups)]`). On a device that lacks a feature, the compiler
   substitutes an automatic emulation (subgroups through workgroup memory, f16 through f32 with f16
   rounding on store) and warns: at build time for known targets, and at `devs.pick` as a logged
   warning. An author-written `#[fallback]` variant replaces the emulation where wanted. Only a
   feature with no emulation is a typed error at pick time.
6. **Brands are scoped and second-class**, as in GhostCell. Generic code takes the brand as a
   parameter (`fn f<dev>(b: &Buffer<T, dev>)`); a branded value is a view that cannot outlive its
   device handle ([`memory.md`: View types](memory.md#view-types)), and `.erase()` gives the
   runtime-checked form for storing or sending.
7. **Device loss is an error, hot-plug is an event.** Losing a device is `fail(DeviceLost)` at the
   next launch, join or transfer, and its buffers become invalid. New devices arrive as events from
   `devs.watch()`; roles are not rebound automatically.
8. **Sharing:** within a program, launches on a device queue in scope order, with optional
   priorities per scope. Sharing between processes is left to the driver and operating system
   (MPS, MIG, time-slicing), configured in the deployment manifest.

## Open questions

None at present.

## Background

The problem and the state of the art that led to this design.

**Problem.** GPU programming is fragmented by vendor, uses a different language (or dialect) from host
code, and makes data movement between memory spaces manual and error-prone. Most programmers never
touch it; those who do are often tied to one vendor.

**State of the art.**
- **CUDA**: mature, fast, huge ecosystem, but NVIDIA-only, C++-dialect, with a separate compilation model.
- **SYCL / oneAPI, OpenCL, HIP**: portability attempts with varying success.
- **Metal / Vulkan compute / WebGPU (WGSL)**: platform-bound compute shaders.
- **Triton**: a Python DSL for tiled GPU kernels. It shows that block-level abstractions can match
  hand-written CUDA for many kernels.
- **Halide**: separates the *algorithm* (what to compute) from the *schedule* (tiling, vectorisation,
  parallelism, placement). This is a key idea.
- **Futhark**: a functional data-parallel language that compiles to GPU and proves that high-level
  array programs can compete.
- **JAX / XLA, MLIR**: compiler infrastructure that lowers high-level array programs to many back ends.
- **Mojo**: aims at Python ergonomics plus systems performance plus accelerator targeting via MLIR.

**What still hurts.**
- Host and device code live in different worlds: different types, different debuggers, different build steps.
- Memory spaces (host, device, shared, unified) are invisible in most type systems, so you only discover
  an accidental copy or an invalid pointer at runtime.
- Portable performance is still mostly a myth. Schedules are tuned per device by hand.
- Synchronisation between CPU and GPU (streams, fences, events) is manual and unchecked.

**Directions.**
- MLIR for tensor-level optimisation if needed; vendor graph compilers for NPUs.
- Feature tiers (subgroups, f16, tensor cores) as target capabilities checked at compile time.

## Prior art

CUDA; HIP/ROCm; SYCL/oneAPI; OpenCL; Metal; Vulkan compute; WebGPU/WGSL; wgpu and naga; Slang;
Triton; CUTLASS/CuTe; Halide (algorithm/schedule separation); Futhark; JAX, XLA, StableHLO; MLIR;
Mojo; rust-gpu; ONNX Runtime; Core ML; OpenVINO; Enzyme (autodiff); Scala path-dependent types and
Rust GhostCell (brands).
