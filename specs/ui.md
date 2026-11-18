# UI, mobile and the browser

> **Status:** not started. Nothing is decided yet; this page records the problem and the
> questions to settle. Listed in the [index](README.md).

## Problem

Two reference examples need user interfaces, and the browser is a required target (R1), yet there is
no decision on how Moddil builds UIs, or how it talks to JavaScript and the DOM. Interop covers C and
Rust only.

## Touched on in

[Open Questions and Next Steps for Moddil](roadmap.md) Q5, [`examples/03-web-store.md`](examples/03-web-store.md),
[`examples/01-multiplayer-game.md`](examples/01-multiplayer-game.md), [`targets.md`: Mobile](targets.md#mobile).

## Questions to settle

- A Moddil UI mod, or shared logic under native UI shells?
- JS and DOM interop for `wasm32-browser`.
- Mobile packaging for Android and iOS.
