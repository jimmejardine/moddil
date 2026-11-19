# UI, mobile and the browser

> **Status:** decided direction, details open. Listed in the [index](README.md). Snippets are
> **illustration only**.

## Summary

| Topic | Decision |
|---|---|
| **Who owns UI** | **The platform, always.** Moddil compiles the **shared core** (models, state, validation, networking, storage); the UI is the platform's own toolkit: SwiftUI or UIKit, Jetpack Compose, or TypeScript with any web framework. There is no standard `moddil/ui` ([Native UI over a shared core](#native-ui-over-a-shared-core)). |
| **Bindings** | `moddil bind` generates **Swift, Kotlin and TypeScript** bindings to a core (and Python bindings, the first target, [`adoption.md`: Python first](adoption.md#python-first)): types, async calls, errors, cancellation and state observation ([Generated bindings](#generated-bindings)). |
| **Browser APIs** | Generated from **WebIDL and TypeScript `.d.ts`**; JS values are opaque handles; Promises suspend through JSPI. Browser access is the **`Dom` effect**, so it is visible and can be simulated in tests ([The browser](#the-browser)). |
| **Mobile** | The toolchain produces **libraries plus bindings** (an XCFramework for iOS, an AAR for Android). Xcode and Gradle still build and sign the app ([Mobile packaging](#mobile-packaging)). |
| **UI programming model** | **Not chosen by Moddil.** Community UI mods are welcome; notes on what fits Moddil's ownership model are below ([Community UI mods](#community-ui-mods)). |

## Problem

User interfaces are a vast and platform-specific domain: text input and input methods, accessibility,
native look and behaviour, platform review rules, and toolkits that change every year. Every
cross-platform UI toolkit (Flutter, React Native, Compose Multiplatform) is a multi-year project with
a large team.

Yet three of the four [reference examples](examples/README.md) have a UI, and the browser is a required
target ([R1](requirements.md#firm-requirements-decided)):

- the [web store](examples/03-web-store.md) needs web, Android and iOS clients;
- the [multiplayer game](examples/01-multiplayer-game.md) renders with WebGPU and has a chat UI;
- [Hashiverse](examples/04-hashiverse.md) runs its client core in a Web Worker under a TypeScript and React app.

Moddil has to answer how its code reaches the screen without becoming a UI toolkit project.

## Prior art

| Approach | Examples | Lesson |
|---|---|---|
| **Shared core, native shells** | [Crux](https://redbadger.github.io/crux/) (Rust core; the shell performs effects on the core's request), Mozilla UniFFI (Rust → Swift, Kotlin, Python), Kotlin Multiplatform | Native UI quality everywhere; the binding layer is the product |
| **One UI toolkit everywhere** | Flutter (own renderer), Compose Multiplatform, React Native, .NET MAUI | One codebase, at the cost of a huge toolkit and a near-native feel |
| **Web everywhere** | Capacitor, Tauri, Electron | Cheapest; mobile feel and performance suffer |
| **Browser bindings** | Rust's wasm-bindgen and web-sys (generated from WebIDL); jco for WASM components | Generated bindings scale to the whole web platform; ambient access makes UI logic hard to test |

## Native UI over a shared core

**Decision: Moddil never owns UI.** UI is always written in the platform's toolkit. Moddil compiles
everything underneath it.

```
 ┌──────── SwiftUI ────────┐  ┌──── Jetpack Compose ────┐  ┌──── TypeScript + React ────┐
 │  views, navigation      │  │  views, navigation      │  │  components, routing       │
 └───────────┬─────────────┘  └───────────┬─────────────┘  └─────────────┬──────────────┘
             │ generated Swift            │ generated Kotlin             │ generated TS
 ┌───────────┴────────────────────────────┴──────────────────────────────┴──────────────┐
 │ Moddil core: models, state, validation, API client, storage, sync, business rules     │
 └───────────────────────────────────────────────────────────────────────────────────────┘
```

Why:

- **The platform does UI best.** Accessibility, input methods, gestures, native controls and platform
  conventions come for free and stay current.
- **Agents already write these toolkits fluently.** SwiftUI, Compose and React have far more training
  data than any new UI framework could ([`agent-interface.md`](agent-interface.md)).
- **A small core** ([R2](requirements.md#firm-requirements-decided)): Moddil's effort goes into the language,
  where its guarantees matter: the state, rules and networking that must be identical everywhere.
- **It already works:** Hashiverse ships exactly this shape today, with a Rust core under React.

What this rules out: a standard `moddil/ui`, and Moddil code that draws native widgets directly. What
it does not rule out: community UI mods ([Community UI mods](#community-ui-mods)), and Moddil code that
renders with the GPU, as games do ([Games and graphics](#games-and-graphics)).

## The shared core

The core exposes **state**, **actions** and **view models**, and the shell renders them:

```rust
#[export]                                        // bound into Swift, Kotlin, TypeScript and Python
pub struct CartView {
    pub lines: Vec<CartLine>,
    pub total: Money,
    pub can_check_out: bool,
}

#[export]
pub struct Cart { … }                            // the state; private fields stay in the core

#[export]
impl Cart {
    pub fn view(&self) -> CartView;                                  // what to render
    pub fn add(&mut self, item: ItemId, qty: u32) with fail(CartError);
    pub fn check_out(&mut self) -> OrderId with suspend, Remote, fail(CheckoutError);
}
```

- **Actions are calls into the core**: the shell calls `add` or `check_out` from its event handlers.
- **Observation:** the generated bindings make exported state observable in each platform's own
  terms: an `@Observable` class in Swift, a `StateFlow` in Kotlin, a subscribable store in TypeScript.
  When an action changes the state, the shell re-reads the affected view models and re-renders.
- **Rules live in one place.** Validation, prices, permissions and offline sync are Moddil code,
  tested once with the language's tools ([`correctness.md`](correctness.md),
  [`determinism.md`](determinism.md)), and identical on every platform.
- **The API to the server** is the compiler-generated contract ([`distribution.md`](distribution.md)), so
  the client core and the server share types end to end.

## Generated bindings

`moddil bind --lang swift|kotlin|typescript` reads a core's exported items and generates the bindings
(Python is also a target, and the first one: [`adoption.md`: Python first](adoption.md#python-first)),
built on the C ABI on native targets ([`interop.md`: The C ABI, both directions](interop.md#the-c-abi-both-directions)) and on WASM exports in the browser.

| Moddil | Swift | Kotlin | TypeScript |
|---|---|---|---|
| `struct` (exported, all fields public) | `struct` | `data class` | `interface` (plain object) |
| `struct` with private fields | `final class` wrapping a handle | `class` wrapping a handle | `class` wrapping a handle |
| `enum` with payloads | `enum` with associated values | `sealed class` | discriminated union |
| `Option<T>` | `T?` | `T?` | `T \| undefined` |
| `fail(E)` | `throws` with a typed `E` | an exception subclass per error type | a rejected Promise, or a thrown error, carrying `E` |
| `suspend` | `async` | `suspend fun` | `Promise` |
| `String`, `&str` | `String` | `String` | `string` (transcoded to UTF-16, [`text.md`: Interop boundaries](text.md#interop-boundaries)) |
| `Vec<T>`, slices | `[T]` | `List<T>` | `T[]` or a typed array |
| `Rc<T>` | a class sharing the handle | a class sharing the handle | a class sharing the handle |
| `impl Fn(…)` parameter | closure | lambda | function |
| Unit types `f64<m>` | a value type carrying the unit | a value class | a branded `number` |

- **Cancellation crosses both ways.** Cancelling a Swift `Task`, a Kotlin coroutine or an
  `AbortSignal` cancels the Moddil task; the Moddil task's cancellation rules apply
  ([`lifecycle.md`: Cancellation and shutdown](lifecycle.md#cancellation-and-shutdown)).
- **Errors are typed and complete.** Every `fail` type in an exported signature becomes a typed error
  on the other side. Nothing unwinds across the boundary; traps abort the task, which the binding
  reports as a distinct "core failed" error ([`error-handling.md`](error-handling.md)).
- **Threading:** the core runs on its own executor. Shells call from their main thread; results and
  observation callbacks are delivered back on the main thread (`MainActor`, `Dispatchers.Main`, the JS
  event loop).
- **Versioned together.** A core and its bindings come from one build, so there is no version skew
  between them. Skew between client and server is handled by the contract checks
  ([`distribution.md`](distribution.md)).
- These are **export** bindings for shells. They do not widen what Moddil imports: libraries are still
  reached only through C, Rust and WASM components ([`interop.md`](interop.md)).

### Platform APIs as shell-provided handlers

**Direction (not yet decided).** Camera, notifications, payments, the keychain and similar platform
services become Moddil **effects** (traits used after `with`) whose handlers are written in Swift, Kotlin or TypeScript and
installed by the shell when it starts the core:

```rust
trait Payments { fn pay(&self, amount: Money, order: OrderId) -> Receipt with suspend, fail(PayError); }
```

- The core calls `payments.pay(…)` (with `payments: Payments` in its signature); on iOS the handler is Swift calling Apple Pay, on Android Kotlin
  calling Google Pay, on the web TypeScript calling a payment provider.
- Tests and simulations install a fake handler, so checkout logic is tested without a device
  ([`effects-and-capabilities.md`](effects-and-capabilities.md), [`determinism.md`](determinism.md)).
- This generalises Crux's design, where the core asks the shell to perform effects, using Moddil's
  ordinary effect handlers instead of a separate message protocol.

## The browser

On `wasm32-browser`, Moddil code reaches browser APIs directly, and also serves as a core under a
TypeScript UI.

- **Generated bindings to the web platform.** `moddil/web` is generated from the WebIDL specifications
  (DOM, Fetch, WebGPU, IndexedDB, WebSocket, WebTransport, …), as Rust's `web-sys` is. Bindings to a
  JS library are generated from its TypeScript `.d.ts` file.
- **JS values are opaque handles** (`externref`), with no copying until a value is converted to a
  Moddil type.
- **Promises suspend.** Calling a Promise-returning API suspends the Moddil task through JSPI until
  the Promise settles; rejection becomes a typed failure ([`compiler.md`: WASM concurrency lowering](compiler.md#wasm-concurrency-lowering)).
- **Browser access is an effect.** Touching the DOM or calling JavaScript performs the **`Dom`
  effect**:
  - it is visible in signatures, so a reviewer can see which code touches the page;
  - a **simulated DOM** handler (in `moddil/web`) runs UI logic in ordinary tests, without a
    browser;
  - code running in a Web Worker has no `Dom` handler, so the compiler rejects DOM access there.
- **Workers:** a core can run in a Web Worker (as Hashiverse's does), with the generated TypeScript
  bindings marshalling calls through `postMessage`.
- **Packaging:** output is an ES module plus generated `.d.ts` types, usable from any bundler (Vite,
  webpack, esbuild) ([`compiler.md`](compiler.md)). Binary size and start-up time are tracked as
  release metrics for the game and web store.

## Mobile packaging

**Decision: the toolchain builds libraries and bindings; the vendor tools build the app.**

| Platform | Moddil produces | The app is built by |
|---|---|---|
| **iOS** | An XCFramework (device and simulator slices) plus a Swift package of generated bindings | Xcode; signing and App Store submission stay with Apple's tools |
| **Android** | An AAR (`.so` per ABI plus generated Kotlin) | Gradle; signing with the usual Android tooling |

- **Ahead-of-time compilation** satisfies iOS's ban on JIT compilation; Moddil never needs an
  interpreter on mobile ([`targets.md`: Mobile](targets.md#mobile)).
- `moddil new --template mobile` creates a core mod plus minimal Xcode and Gradle projects wired to
  it, so `moddil build` followed by a normal Xcode or Android Studio run works on the first day.
- Moddil does not drive Xcode or Gradle itself and does not produce `.ipa` or `.aab` files. Those
  toolchains change too often for a language toolchain to track.

## Games and graphics

- **Rendering is not UI.** A game, a visualisation or a map renders through the GPU from Moddil code:
  WebGPU in the browser, Vulkan or Metal natively ([`accelerators.md`](accelerators.md)). This is the
  multiplayer game's main loop and stays fully in Moddil.
- **In-game UI** (HUDs, menus, debug panels) drawn with the GPU is a community concern; immediate-mode
  libraries fit naturally.
- The game's **chat UI** is ordinary DOM, written either in Moddil through `moddil/web` or as a small
  TypeScript component over the core.

## Desktop

The same pattern: a native shell (SwiftUI on macOS, WinUI on Windows, GTK or Qt on Linux), or a
webview shell such as Tauri running the TypeScript UI, over a Moddil core loaded as a native library.

## Community UI mods

Moddil does not standardise a UI framework, but nothing stops community mods from building one on
`moddil/web` or on GPU rendering. For their authors, the known fits with Moddil's ownership model:

- **Elm architecture** (immutable state, messages, a pure `view`): fits ownership and immutable `Rc`
  directly, and makes UI state replayable ([`determinism.md`: Record and replay](determinism.md#record-and-replay)).
- **Fine-grained signals** (SolidJS, Leptos): fastest updates, but they need shared mutable cells,
  which Moddil makes explicit as sync cells ([`concurrency.md`: Shared mutable state](concurrency.md#shared-mutable-state)).
- **Immediate mode** (egui, Dear ImGui): simplest with ownership; best for games and tools, weakest for
  accessibility.

## Fit with the reference examples

| Example | UI | Moddil's part |
|---|---|---|
| [Web store](examples/03-web-store.md) | SwiftUI, Jetpack Compose, TypeScript web app | Shared core (cart, catalogue, offline sync, API client) bound to all three |
| [Multiplayer game](examples/01-multiplayer-game.md) | WebGPU rendering; DOM chat | Rendering, prediction and netcode in Moddil; chat through `moddil/web` or a TS component |
| [Hashiverse](examples/04-hashiverse.md) | TypeScript and React | Client core in a Web Worker with generated TS bindings (replacing today's hand-written wrapper) |
| [HFT platform](examples/02-hft-platform.md) | Operator dashboards | A web dashboard over a core, or an external tool reading metrics ([`observability.md`](observability.md)) |

## Open questions

1. **Observation protocol:** how fine-grained is change notification (whole view model, per field, or
   diffs), and who computes the diff?
2. **Granularity of browser effects:** one `Dom` effect, or separate `Dom` and `js` effects (JS calls
   that never touch the page)?
3. **Generics and traits across bindings:** exported generic functions are monomorphised for listed
   types; how are exported traits (implemented by the shell) expressed in each language?
4. **Live update under a running shell:** can a debug build hot-swap the core while the app stays open
   ([`build-times.md`: Live update (debug builds)](build-times.md#live-update-debug-builds))?
5. **More binding languages:** Python is **decided** as the first binding, in v0
   ([`adoption.md`: Python first](adoption.md#python-first)). C# and .NET later?
6. **Platform-API handlers:** adopt the shell-provided handler direction above as the standard way to
   reach device services?
