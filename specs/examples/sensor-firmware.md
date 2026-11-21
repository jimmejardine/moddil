# Sensor Firmware

## Overview

Battery-powered sensor firmware on a microcontroller. It samples sensors on a timer, filters and
aggregates the readings, and sends them over a low-power radio or a serial link to a gateway. There
is no operating system, no allocator and no file system: the program is the whole machine.

It proves the [freestanding profile](../freestanding.md) end to end, and keeps freestanding
features honest under the rule that every feature must be needed by an example.

## Topology

```
┌──────────── microcontroller (Cortex-M4F / RISC-V) ────────────┐
│  TIMER0 irq ──► sample ring ──► filter task ──► report task ───┼──► radio / UART ──► gateway
│  RADIO irq  ──► rx ring    ──► command task                    │
│  static tasks, no heap, stacks sized at compile time           │
└────────────────────────────────────────────────────────────────┘
```

| Part | Role |
|---|---|
| Interrupt handlers | `TIMER0` samples the ADC into a ring buffer; `RADIO` moves received frames into a second ring. Restricted effect set: no suspension, no allocation. |
| Static tasks | `filter` (fixed-point and float filtering), `report` (batches and sends), `command` (parses configuration commands from the gateway), on `moddil/rt-embedded`, preemptive by priority. |
| Gateway | Any host; a Moddil program in the simulation tests. |

## Key flows

1. **Sample:** the timer interrupt reads the ADC and pushes into a lock-free ring shared with the
   `filter` task; the priority-ceiling analysis proves the sharing is deadlock-free.
2. **Filter and report:** `filter` smooths readings (floats, with the FPU declared in the handlers
   that use it); `report` batches them and sends a frame, retrying with back-off.
3. **Configure:** the gateway sends a command (new sampling rate); `command` validates it into a
   refined type and applies it at the next tick.
4. **Sleep:** when idle, the scheduler enters low-power sleep until the next interrupt.

## Moddil features this stresses

- **Profiles as effect sets:** the binary is built without `alloc`, `Fs`, `Net` or `spawn`; library
  code (filters, the frame codec) is shared unchanged with the host gateway.
- **Interrupts as restricted entry points**, including `#[interrupt(TIMER0, priority = 2, fpu)]`
  for a handler that uses floats ([`freestanding.md`: Concurrency without a runtime](../freestanding.md#concurrency-without-a-runtime)).
- **Stack-depth analysis** of every handler and task, so a stack overflow is impossible.
- **Sync cells and lock-free rings** shared between interrupts and tasks.
- **Fallible allocation is absent altogether:** buffers are static.
- **Host simulation tests:** the same firmware logic runs on the host against simulated sensors,
  radio loss and clock drift ([`determinism.md`](../determinism.md)), then in Renode and QEMU in CI.
- **Hardware access:** typed MMIO registers through a HAL mod, with `unsafe` contained.

## Success criteria

- Builds for `thumbv7em` (Cortex-M4F) and `riscv64` QEMU `virt`, the tier-1 bare-metal targets
  ([`targets.md`: Target names and tiers](../targets.md#target-names-and-tiers)).
- Its test suite passes on the host simulator, in Renode and in QEMU in every CI run.
- Fits in 64 KB of flash and 16 KB of RAM, with compile-time stack bounds reported.
- Interrupt latency within a stated bound, measured in Renode.

## Mods used

- Standard: `moddil/rt-embedded`, `moddil/sync`, `moddil/serde` (compact binary), `moddil/runtime-sim`
  for host tests.
- New: `sensor/hal` (board support), `sensor/frames` (the radio frame codec, shared with the
  gateway).

## Interop and non-Moddil parts

- Vendor register descriptions (SVD files) imported at comptime to generate typed registers.
- Optionally a vendor radio stack in C, through an overlay ([`interop.md`](../interop.md)).

## Open questions

None at present.
