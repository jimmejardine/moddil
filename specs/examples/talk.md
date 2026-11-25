# Talk, text to speech for Python on any GPU

## Overview

**Talk** is a Python package that speaks, published on PyPI as `moddil-talk` and imported as
`talk`. It runs
[Kokoro-82M](https://huggingface.co/hexgrad/Kokoro-82M), an open-weight text-to-speech model
(Apache-2.0, 82 million parameters, 24 kHz output), written in Moddil and compiled to a native
extension module. It runs on the GPU through WebGPU, so it works on NVIDIA, AMD, Intel and Apple
hardware alike, and falls back to a CPU pool when there is no GPU.

The same Moddil package also builds for `wasm32-browser`, where a Svelte 5 demo page speaks the
text typed into it, on the visitor's own GPU.

## The pitch

```python
import talk

talk.say("Hello from Moddil")                          # speaks through the default output
audio = talk.synthesize("Hello", voice="af_heart")     # numpy float32 array, 24 kHz
```

- **`pip install moddil-talk`, and Python can speak.** There is no torch, no CUDA toolkit and no
  multi-gigabyte download: one small wheel per platform, plus the model weights, fetched once.
- **Any GPU.** WebGPU (through wgpu) reaches Vulkan, Metal and DX12, so one wheel serves every
  vendor. NVIDIA-only speed-ups are optional.
- **Visible capabilities.** Only `talk.download()` can reach the network or write files;
  synthesis cannot. That is a property of the signatures, checked by the compiler, and it is the
  supply-chain story ([`packages-and-supply-chain.md`: Capability-scoped dependencies](../packages-and-supply-chain.md#capability-scoped-dependencies)).
- **The same code in the browser.** The demo on moddil.com runs the identical model code, which
  ties the Python story to the web one ([`fumoca.md`](fumoca.md)).

## Topology

```
 text ──► G2P ──► phonemes ──► Kokoro (tensor graph + tile kernels) ──► waveform ──► audio out
          CPU                   Gpu: WebGPU, or the CPU pool                          Audio
```

| Stage | Where | Notes |
|---|---|---|
| G2P (grapheme to phoneme) | CPU | Lexicon lookup, with a small open-weights neural G2P model for unknown words; English only. No espeak-ng, which is GPL-3 and cannot ship in a permissive wheel |
| Acoustic model | `moddil/tensor` graph, plus tile kernels where fusion matters | StyleTTS 2-style: text encoder, prosody and duration predictors, style vectors per voice |
| Vocoder | Tile kernels | ISTFTNet: upsampling convolutions, then an inverse short-time Fourier transform |
| Output | `Audio` | numpy array to Python; the platform's audio device for `say`; Web Audio in the browser |

## Key flows

1. **First use:** `talk.download()` fetches the weights and voices into a cache directory, checking
   their hashes. It is the only call that performs `Net` and `Fs`.
2. **Synthesise:** text is split into sentences, converted to phonemes, and run through the model on
   the selected device. Long text streams sentence by sentence, so speech starts before the whole
   text has been synthesised.
3. **Speak:** `say` plays each sentence's audio while the next is being synthesised.
4. **Choose a device:** the best available GPU by default; `talk.device("cpu")` or a named adapter
   overrides it.
5. **Browser:** the Svelte 5 page loads the WASM module and weights (cached in the browser), and
   synthesises on the visitor's GPU.

## Moddil features this stresses

- **Python first:** `#[export(python)]`, wheels on the CPython stable ABI, `.pyi` stubs, numpy
  arrays without copying, typed errors as exception classes, and the GIL released during synthesis
  ([`adoption.md`: Python first](../adoption.md#python-first)):

  ```rust
  #[export(python)]
  pub fn synthesize(text: &str, voice: &str) -> Vec<f32> with Gpu, fail(TalkError) { … }

  #[export(python)]
  pub fn download() with Net, Fs, fail(DownloadError) { … }
  ```

- **Tensor expressions and tile kernels together:** most of the model is level 1, built into a graph
  at comptime; the vocoder's convolutions and the inverse STFT are level-2 tile kernels
  ([`accelerators.md`: Three levels, one language](../accelerators.md#three-levels-one-language)).
- **`Gpu` handlers chosen at run time:** WebGPU when there is an adapter, the CPU simulator when
  there is not. The model code is the same either way
  ([`accelerators.md`: Runtimes and backends](../accelerators.md#runtimes-and-backends)).
- **Missing GPU features are emulated:** an adapter without `f16` gets automatic emulation, with a
  logged warning.
- **Effects as the security boundary:** synthesis performs `Gpu` and nothing else.
- **One package, two targets:** native wheels and `wasm32-browser` from one source
  ([`targets.md`](../targets.md)).
- **Determinism:** with the CPU handler and a fixed seed, output is bit-identical across machines,
  so golden-audio tests are exact ([`determinism.md`: Cross-platform determinism](../determinism.md#cross-platform-determinism)).

## Success criteria

Initial targets, to be refined:
- Faster than real time on an integrated laptop GPU, and on a recent CPU with the CPU pool.
- Output matches the reference PyTorch implementation within a stated tolerance on a fixed test set.
- One source builds the Python wheels (Linux, macOS, Windows; x86-64 and ARM64) and the browser
  demo.
- A wheel-size budget of a few MB, excluding the weights.
- Time to first audio of a long text bounded by the first sentence, not the whole text.

## Mods used

- Standard: `moddil/tensor`, `moddil/gpu`, `moddil/simd`, `moddil/fs`, `moddil/net` (download only),
  `moddil/crypto` (weight hashes), `moddil/text` (sentence breaks, normalisation), `moddil/web` and
  `moddil/runtime-wasm` (browser), `moddil/test`, `moddil/bench`.
- Standard, new: `moddil/audio` for `say` ([`stdlib.md`: Audio](../stdlib.md#audio)).
- Talk's own: `talk/g2p` (the lexicon and the neural fallback) and `talk/kokoro` (the model).

## Interop and non-Moddil parts

- **Weights:** from Hugging Face, converted once to safetensors, and loaded by `moddil/tensor`
  ([`accelerators.md`: Loading model weights](../accelerators.md#loading-model-weights)).
- **The Svelte 5 demo page** uses the generated TypeScript bindings through the Svelte adapter in
  `moddil/web` ([`ui.md`: The browser](../ui.md#the-browser)).
- **Python:** only the binding layer that `moddil bind` generates; no hand-written C.

## Open questions

None at present: the name, the G2P model and the languages were decided in November 2026.
