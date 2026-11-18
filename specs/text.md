# Text and strings

> **Status:** not started. Nothing is decided yet; this page records the problem and the
> questions to settle. Listed in the [index](README.md).

## Problem

Every reference example, and almost every program an agent writes, handles text, yet string types,
encoding guarantees and formatting have no design of their own.

## Touched on in

[`memory-and-resources.md`: Zero-copy I/O](memory-and-resources.md#zero-copy-io) (rc slices),
[`comptime.md`: Tagged string literals (embedded DSLs)](comptime.md#tagged-string-literals-embedded-dsls) (`fmt"…"`), [`mods.md`: Initial standard-mod catalogue](mods.md#initial-standard-mod-catalogue) (`moddil/text`).

## Questions to settle

- String types: owned, borrowed and rc slices, and how they convert.
- UTF-8 guarantees, byte strings, and OS strings (paths, environment).
- Indexing: bytes, code points or grapheme clusters.
- Formatting, and localisation (messages, numbers, dates).
