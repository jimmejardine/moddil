# The Freestanding Profile

> **Status:** proposal. Covers kernels, bootloaders, hypervisors and microcontroller firmware: code
> with no operating system underneath. Related: [`targets.md`](targets.md) (hosted targets),
> [`concurrency.md`](concurrency.md), [`memory-and-resources.md`](memory-and-resources.md).
> Syntax is illustrative.

## 1. Summary

**Freestanding is not a dialect. It is a question of which handlers and capabilities the entry point
provides.**

Allocation, scheduling, time, I/O and hardware access are already effects or capabilities in Moddil.
On bare metal the entry point simply does not provide `fs`, `net` or `spawn`, and may not provide
`alloc`. Code that needs them **does not compile**; code that does not need them (parsers, crypto,
maths, collections over caller-provided buffers) works unchanged.

This improves on Rust's `#![no_std]`, where every crate must opt in: in Moddil every library is
automatically as portable as its effect set.

## 2. Profiles

Declared in the binary's or mod's manifest and checked across all dependencies.

| Profile | Provides | Lacks |
|---|---|---|
| `hosted` (default) | OS, full standard library, runtime and executors, allocator | — |
| `freestanding-alloc` | Core and `alloc` mods; an allocator the program supplies | OS services; threads unless the program supplies a scheduler |
| `freestanding` | Core mods only: primitives, slices, `Option`/`Result`, fixed-buffer formatting, atomics, intrinsics | Heap, OS, runtime |

- The standard library is layered: `moddil/core-*` → `moddil/alloc` → the OS-backed standard
  library (the same split as Rust's `core`, `alloc` and `std`).
- Mod manifests already declare supported targets ([`mods.md`](mods.md) §2). A dependency that needs
  an unavailable effect is a build error naming the effect and the dependency path that requires it.

## 3. Entry points and boot

```rust
#[entry]                                 // the reset handler; runs before any runtime exists
fn reset(board: Board) -> ! { … }

#[trap_handler]                          // required in freestanding profiles
fn on_trap(info: &TrapInfo) -> ! { log_to_flash(info); system_reset() }
```

- `#[entry]` replaces `main`.
- A **trap handler is required**: what a trap (class 3, [`error-handling.md`](error-handling.md) §2)
  does on bare metal (halt, reset, record to flash). Omitting it is a compile error.
- R3 applies to entry points and interrupt handlers: every error is handled.
- Low-level control: `#[link_section]`, `#[repr(align(…))]`, `#[naked]` functions, the interrupt calling convention,
  position-independent code, red zone off, and target features (for example, no floating point in
  kernel code).
- The toolchain **generates the linker script** from a declarative memory map in the manifest.

## 4. Memory without an operating system

- Allocation is a capability (overview §2.2).
- **In freestanding profiles allocation is fallible by default:** it fails with `OutOfMemory`
  (class 1) rather than trapping, as in Zig and the Linux kernel's Rust code.
- Strategies, chosen by the program:
  - static allocation only;
  - a boot-time arena or fixed-size pools;
  - a program-supplied heap allocator (TLSF, linked list).
- `Rc` exists only when an allocator is available. On a single core, biased reference counting
  reduces to plain owner counts.
- **Proposed answer** to the allocator question in [`memory-and-resources.md`](memory-and-resources.md)
  §10: an `Rc` records its allocator only when it is not the default one; arenas stay borrow-based.

## 5. Concurrency without a runtime

Three levels, from nothing to a full scheduler:

**L0: super-loop plus interrupts.**
- Interrupt handlers are entry points with a **restricted effect set**, the same mechanism as GPU
  `kernel` code ([`accelerators.md`](accelerators.md)): they cannot suspend or allocate, and must
  handle their errors.
  ```rust
  #[interrupt(TIMER0, priority = 2)]
  fn tick(state: &Shared) { state.ticks.with(|t| *t += 1) }
  ```
- Data shared between interrupts and the main loop goes through sync cells guarded by critical
  sections. A compile-time **priority-ceiling analysis** (as in Rust's RTIC) makes this deadlock-free
  and lock-free where priorities allow.

**L1: static tasks.**
- A small runtime mod, `moddil/rt-embedded`, provides the scheduler handler with a fixed number of task
  slots: preemptive by priority (driven by a timer interrupt) or cooperative.
- Stackful tasks on bare metal need only a register-saving context switch, as in an RTOS.
- **Each task's stack size is computed at compile time** by stack-depth analysis: no unbounded
  recursion, and no indirect calls without a declared bound (see [`correctness.md`](correctness.md)
  §4.2, level L1).
- Structured concurrency still holds: the root scope lasts as long as the program.

**L2: freestanding with an allocator and a full scheduler.** Executors behave as in hosted mode
([`concurrency.md`](concurrency.md) §6.1), minus the OS I/O reactor.

## 6. Hardware access

- **Typed register views.** Memory-mapped registers are typed, volatile-only views generated from the
  vendor's SVD description by comptime code reading an embedded file (as svd2rust does, but without a
  build script).
- **Hardware access is a capability**, scoped to a peripheral or address range (`mmio(UART0)`). A
  driver mod can only touch its own registers (the Tock and Hubris idea), and the capability shows in
  signatures, audits and semantic diffs.
- **Inline assembly** is an intrinsic behind `unsafe` and an `asm` capability.
- **Bit-exact layouts:** `#[repr(C)]`, `#[repr(packed)]`, bitfields in packed structs, and **arbitrary-width
  integers** (`u3`, `i24`, as in Zig).

## 7. Observability and debugging

- Logging uses the compile-time interned events of [`observability.md`](observability.md): only an
  event ID plus arguments go over the wire (defmt-style), via a debug probe, UART or RTT.
- Traps write their trace into a fixed buffer.
- Live update ([`build-times.md`](build-times.md) §4.1) and record/replay are not available on bare
  metal by default.

## 8. Testing on the host

Hardware is just effects (`gpio`, `uart`, `timer`, `mmio`), so firmware runs on a desktop under
simulation handlers and `moddil/runtime-sim` ([`determinism.md`](determinism.md)). Embedded code
becomes unit-testable and deterministic, a large improvement over typical C, Zig or Rust firmware
testing.

## 9. Verifying the platform in emulators

**Decision:** no hardware-in-the-loop testing by the project. Bare-metal support is verified with
**QEMU** and **Renode** in CI. Hardware testing is left to users and HAL mod authors.

### 9.1 Test pyramid

1. **Host simulation of hardware effects** (§8): logic and drivers against simulated handlers, on
   every commit.
2. **QEMU** (system mode): real cross-compiled binaries boot, take interrupts, switch tasks and trap
   correctly. Built into the test runner, e.g. `moddil test --target thumbv7m-none --runner qemu`.
   Results and exit codes come back over **semihosting**; `-icount` gives deterministic instruction
   timing. (QEMU is also what the Rust embedded book and Zig's `-fqemu` test runner use.)
3. **Renode** (Antmicro): whole chips **with peripherals**, multi-device networks, and
   **deterministic virtual time**, for driver, timing and multi-node tests.

Other tools, optional: Spike (RISC-V reference simulator) for ISA conformance; Unicorn (CPU emulator
library) for in-process tests of generated code; Verilator (cycle-accurate simulation of open cores
such as VexRiscv and CVA6); Arm Fixed Virtual Platforms; Wokwi (browser simulator for ESP32, RP2040
and STM32 boards).

### 9.2 Architecture coverage

QEMU and Renode together cover every architecture class Moddil targets. Machine and board names
below must be checked against current QEMU and Renode documentation when targets are finalised.

| Target class | Example CPUs | Emulated by |
|---|---|---|
| `thumbv6m` | Cortex-M0/M0+ | QEMU `microbit`; Renode STM32F0-class boards |
| `thumbv7m` | Cortex-M3 | QEMU `lm3s6965evb`, `mps2-an385`, `stm32vldiscovery` |
| `thumbv7em` (with FPU) | Cortex-M4F/M7 | QEMU `mps2-an386` (M4), `mps2-an500` (M7), `netduinoplus2`; Renode STM32F4, nRF52840 |
| `thumbv8m.main` | Cortex-M33 with TrustZone | QEMU `mps2-an505`, `mps2-an521` (dual core) |
| `riscv32imac` | Microcontroller RISC-V | QEMU `sifive_e`, `virt`; Renode LiteX/VexRiscv, HiFive1 |
| `riscv64gc` | Application RISC-V | QEMU `virt`, `sifive_u` |
| `aarch64` | Cortex-A | QEMU `virt`, Raspberry Pi machines |
| `x86_64` (kernels) | PC | QEMU `q35` with OVMF (UEFI) |

**Covered well** (most of what a language toolchain has to prove): code generation per architecture,
ABI and calling conventions, boot and linker scripts, interrupt entry and exit, context switching,
traps, volatile MMIO, the MMU and privilege levels, stack-depth limits (guard regions), and multi-core
behaviour via QEMU's multi-threaded emulation.

**Residual risks and mitigations:**

| Gap | Why | Mitigation without hardware |
|---|---|---|
| Weak memory ordering | Real Arm and RISC-V cores reorder more than an emulator on an x86 host exhibits | Check every atomic lowering against the formal Arm and RISC-V memory models with litmus tests (herd7-style); Loom-style model checking of the standard library's sync primitives ([`correctness.md`](correctness.md)) |
| Cycle-accurate timing, caches, DMA races | QEMU and Renode are functional, not cycle-accurate | Optional Verilator runs for timing-critical code; keep correctness independent of timing |
| Peripheral model fidelity, silicon errata | Models are written from datasheets | Renode models where available; errata handled in HAL mods; user bug reports |
| Low-power modes, analog, radio | Partly or not modelled | Out of scope for the toolchain; the responsibility of HAL mods |

**Proposed rule:** a bare-metal target is **tier 1 only if a free emulator runs its full test suite
in CI**.

### 9.3 Architectures and boards

- **Architecture support** (code generation, ABI, boot, interrupts, tasks) is the toolchain's job and
  is tested in emulators (§9.2).
- **Board support** (pins, peripherals, clocks, radios, special units such as the RP2040's PIO) lives
  in HAL and board mods. They are tested in Renode where a model exists, always under host
  simulation (§8), and on real hardware by their authors and users.

| Device | What it is | Moddil support | Testable by the project |
|---|---|---|---|
| Raspberry Pi 3/4/5 | Linux computer (AArch64) | A **hosted** target (`aarch64-linux`), not freestanding: ordinary static binaries | Yes: QEMU user mode runs AArch64 Linux binaries; QEMU `raspi3b`/`raspi4b` for bare-metal boot work (Pi 5 machine availability to verify) |
| Raspberry Pi Pico / Pico 2 (RP2040 / RP2350) | Microcontroller: Cortex-M0+; Cortex-M33 plus RISC-V Hazard3 | Freestanding: `thumbv6m`, `thumbv8m.main`, `riscv32` | Architecture: yes, on generic M0+/M33/RISC-V machines. Board peripherals (notably PIO): not in upstream QEMU; Wokwi simulates the Pico; Renode coverage to verify |
| Arduino Uno R4, Nano 33 BLE, Portenta | Cortex-M4/M7 (Renesas RA4M1, nRF52840, STM32H7) | Freestanding: `thumbv7em` | Yes: QEMU M4/M7 machines; Renode nRF52840 and STM32 models |
| Classic Arduino Uno, Mega, Nano | **8-bit AVR**, 2–8 KB of RAM | Possible only at L0 (no tasks, no allocator, 16-bit `usize`) through LLVM's AVR backend; **tier 3 at best** | Yes: QEMU `arduino-uno`/`arduino-mega` machines; simavr |
| ESP32-C3 / C6 | RISC-V microcontroller | Freestanding: `riscv32imc` | Architecture: yes. Wi-Fi and Bluetooth: no |
| ESP32 / ESP32-S3 (e.g. Arduino Nano ESP32) | **Xtensa** | Not supported: no mainline LLVM backend (only Espressif's fork) | No |

In short: Raspberry Pi computers are ordinary hosted targets and fully testable. Pico-class and modern
Arduino boards are covered at the architecture level, with peripherals depending on HAL mods and
emulator models. Classic 8-bit Arduinos are a stretch. Xtensa-based ESP32s are out of scope.

### 9.4 Proving the profile end to end

**Recommended:** a fifth reference example that runs in this CI, for example:
- sensor firmware on Renode (Cortex-M4F) and QEMU (Cortex-M33, RISC-V); and/or
- a tiny kernel that boots on QEMU `x86_64` and RISC-V `virt` and runs its own test suite.

## 10. Not available in freestanding profiles

- OS effects (`fs`, `net`, `env`, processes);
- the default runtime and executors (unless a freestanding runtime mod supplies them);
- live update; record and replay (by default);
- unbounded recursion where stack analysis is required;
- `Rc` without an allocator.

## 11. Open questions

1. Add a fifth, embedded reference example (§9.4)?
2. Should `moddil/rt-embedded` default to preemptive or cooperative scheduling?
3. Does stack-depth analysis apply to all freestanding code, or only to L1 tasks?
4. Tier assignments for bare-metal targets ([`targets.md`](targets.md) §5).
5. Floating point in interrupt handlers (lazy FPU state saving).

## Prior art

- Rust: `no_std`, `core`/`alloc`, Embassy, RTIC, svd2rust, defmt, probe-rs.
- Zig freestanding targets.
- Ada's Ravenscar profile.
- Tock OS, Hubris (Oxide), seL4.
- C `-ffreestanding`; MISRA C.
- QEMU, Renode, Spike, Unicorn, Verilator, Arm FVP, Wokwi; herd7 and the formal Arm/RISC-V memory
  models.
