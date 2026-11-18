# Standard library shape

> **Status:** not started. Nothing is decided yet; this page records the problem and the
> questions to settle. Listed in the [index](README.md).

## Problem

The catalogue of standard mods exists, but not the shape of their core interfaces, which every
library builds on.

## Touched on in

[`mods.md`: Initial standard-mod catalogue](mods.md#initial-standard-mod-catalogue), [`mods.md`: Open questions](mods.md#open-questions).

## Questions to settle

- How many batteries: a broad standard library (Go) or a minimal one (Rust)?
- The iterator protocol, and how it relates to generators.
- The collections surface (`Vec`, maps, sets) and its naming.
- The I/O stream model: readers, writers and buffers over effects.
