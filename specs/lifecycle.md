# Program lifecycle

> **Status:** not started. Nothing is decided yet; this page records the problem and the
> questions to settle. Listed in the [index](README.md).

## Problem

How a program starts, is configured and stops is spread across several areas: root capabilities
arrive in `main`, shutdown is layered, and traps terminate something. These pieces need one coherent
design.

## Touched on in

[`concurrency.md`: Cancellation and shutdown](concurrency.md#cancellation-and-shutdown) (shutdown),
[`effects-and-capabilities.md`: Authority comes only from root capabilities](effects-and-capabilities.md#authority-comes-only-from-root-capabilities) (root capabilities),
[`error-handling.md`: Three outcome classes](error-handling.md#three-outcome-classes) (traps).

## Questions to settle

- The entry point and which root capabilities it receives.
- Configuration, command-line arguments and environment variables.
- Signals and exit codes.
- Does a trap terminate the task or the whole process?
