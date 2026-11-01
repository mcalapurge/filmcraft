# ADR 0002: Whisper on the GPU through candle's Metal backend on macOS

- **Status:** accepted (2026-11-01, requested by the project owner)
- **Related:** [ADR 0001](0001-platform-ffi.md) (OS FFI lives in `crates/platform`)

## Context

Speech-to-text (`crates/speech`) runs Whisper on candle. On the CPU, the large models that give
good transcripts are slow: Whisper large-v3-turbo takes 86 s for 3.5 minutes of speech on an
M-series Mac, and large-v3 several times that. Every Mac FilmCraft supports has a Metal GPU.

candle has a Metal backend (`candle-core`/`candle-nn` feature `metal`). It is Rust (MIT /
Apache-2.0) and reaches Metal through the `objc2-metal` bindings, which FilmCraft already uses in
`crates/platform`. Its kernels are Metal Shading Language source compiled by the OS at run time.
All of its `unsafe` is inside those third-party crates; FilmCraft's own code stays `unsafe`-free.

## Decision

On macOS, `filmcraft-speech` turns on candle's `metal` feature whenever `whisper` is on (a
target-specific dependency, so other platforms and wasm are unchanged). The device is picked at
run time (`whisper::model::gpu`):

- Metal device 0 when one can be created (inside `catch_unwind`), else the CPU.
- Loading the model on Metal falls back to the CPU if it fails.
- `FILMCRAFT_SPEECH_DEVICE=cpu` forces the CPU.
- On the GPU the weights are f16 (attention softmax in f32); on the CPU f32. Transcripts and
  word timings match f32 (the same words, start times within one 20 ms step).

This is not OS media FFI under ADR 0001: no FilmCraft code calls Metal, and the dependency is a
compute backend of a pure-Rust ML library, like the `wgpu` compositor.

## Consequences

- Measured on 209 s of speech (M-series Mac): large-v3-turbo 86 s on the CPU, 15 s on Metal;
  large-v3 46 s on Metal; distil-large-v3.5 14 s on Metal.
- Small models gain nothing (Whisper base: about the same on either), because decoding one token
  at a time is dominated by per-step overhead, not arithmetic.
- The Apple Neural Engine is not used: it is reachable only through Core ML, which would need
  converted models and Core ML bindings in `crates/platform`. Revisit if Metal is not enough.
- Windows/Linux GPUs (CUDA, Vulkan) are not covered.
