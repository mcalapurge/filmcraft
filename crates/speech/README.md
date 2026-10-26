# filmcraft-speech

Speech-to-text for FilmCraft's Text panel ▸ Transcript (L2). User-facing behaviour, commands and
model handling: [docs/transcripts.md](../../docs/transcripts.md).

- `Transcriber` trait (mono 16 kHz in, word-timed `filmcraft_project::Transcript` out) and
  `FixedTranscriber` for tests and agents.
- `models`: Whisper model catalogue (pinned revisions, SHA-256, sizes, licences) and, with feature
  `download`, the verified downloader (ureq + rustls + RustCrypto). Weights are never bundled.
- `whisper` (feature `whisper`): Whisper inference on candle (CPU, pure Rust).
- `mel`, `vad`, `diarize`: log-mel front end, word-bound tightening, MFCC-clustering speaker labels.

Both features are off by default. The web app enables `whisper` for its Web Worker;
`download` stays native-only because the browser streams verified model files into OPFS.
`cargo xtask wasm` checks the standalone crate and the web app that enables inference.

Word alignment follows the published procedure: the decoder row of each input token attends to
the audio of the token it predicts, so a token's time is read from the row before it (the final
prompt token's row for the first word); a trailing end-of-text row takes the audio after the last
word. Energy-based trimming then removes quiet edges, including audio before the first word;
timing remains an estimate (20 ms attention, 10 ms energy frames).

## References

Implemented from the published description of the model: A. Radford et al., "Robust Speech
Recognition via Large-Scale Weak Supervision" (OpenAI, 2022), the model cards and configuration
files published with the weights (`config.json`, `generation_config.json`, `tokenizer.json`), and
the candle tensor library's public API. Weights: OpenAI Whisper, MIT licence.

The native source guard against `std::time::Instant` retains `PathBuf` values and matches
path components, so the same guard runs on Windows, macOS and Linux.
