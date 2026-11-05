# Phase 1: Phonon-2 and Rust compatibility (historical)

This records the initial in-process batch-recognition experiment. It has been superseded by Nemotron 3.5 ASR Streaming, with Zipformer retained as fallback. See [Nemotron setup](NEMOTRON_STREAMING.md) for the current architecture.

Checked on 2026-10-26. This phase validates the speech engine and records the smallest app architecture. It does not start a microphone or download model weights.

## Decision

Proceed with `phonon-rs` 0.1.3 as the offline English recognizer. Its Candle CPU inference and CPAL microphone feature compile for Windows x64. The crate does not provide a Windows dictation shell, and its transcription interface is batch based rather than a streaming decoder API.

## Findings

| Area | Result | Application impact |
| --- | --- | --- |
| Crate | `phonon-rs` 0.1.3, library name `phonon`, Apache-2.0, Rust 2024 edition. | Pin the crate version and include its license notices in the distribution. |
| Windows x64 | `cargo check` succeeded for `x86_64-pc-windows-msvc` with `default-features = false` and `features = ["mic"]`, using Rust 1.96.0. | The inference and microphone dependency graph is Windows-buildable. This does not verify microphone hardware, model loading, or runtime behavior. |
| Windows audio | `phonon::mic::Capture::open(None)` opens the default input. It captures mono `f32` samples; `take()` drains newly captured samples and `finish()` closes the stream. CPAL uses WASAPI by default on Windows. | Capture can be reused; keep inference out of CPAL's callback and calculate the level on a worker. |
| Inference | `Engine::load(Some(model_path), DeviceArg::Cpu, None)` loads and retains the Candle model. `Engine::transcribe(&samples, chunk_secs)` returns timestamped `Word` values with confidence. Input must be 16 kHz mono. | Load once on a worker at startup, convert captured samples to 16 kHz, and reuse the engine across dictation sessions. |
| Partials | There is no public streaming decoder state or partial-result callback. The microphone helper re-runs inference on the whole active utterance for a terminal preview; only completed utterances are written to stdout. | Wavefront waits for a VAD pause or stop, then transcribes each completed phrase once. This avoids repeated inference and provisional-text correction. |
| Platform dictation | The crate's `dictate` feature has Linux and macOS modules and a compile error on other platforms. Default features include `dictate`. | Set `default-features = false`; enable `mic` only. Implement the Windows hotkey, non-activating overlay, and Unicode text injection in this app. |
| Model and offline use | The Phonon-2 weights are English-only, accept 16 kHz audio, and produce punctuated, capitalized text. The archive is about 164 MB and about 177 MB on disk. Initial installation needs a network download; subsequent inference can be local. Weights are CC-BY-4.0; the Rust crate is Apache-2.0. | Keep the model outside the executable and install it once under the app's local data directory. Bundle attribution with the app. |
| Runtime | The Rust implementation uses Candle rather than Python, PyTorch, or ONNX Runtime. No separate ASR runtime is declared by the CPU feature set. | A standalone release still needs a linked-binary and clean-machine dependency check in a later phase; this compile check alone does not prove packaging. |

### Windows model-cache detail

The crate's default cache helper looks for `XDG_CACHE_HOME` or `HOME`. A Windows app should not rely on those being set. Use Tauri's app-local data directory, manage the first-run download there, and pass the explicit archive path to `Engine::load(Some(path), ...)`.

### Incremental transcription limit

The model exposes separate feature encoding and greedy decoding calls, but no incremental encoder cache. The microphone helper's preview transcribes the growing utterance again, so its cost grows with utterance length. Wavefront finalizes speech at a VAD silence boundary or after a 30-second segment cap and inserts each finalized segment once.

## Smallest working architecture

1. **Tauri shell:** one hidden-by-default, transparent pill window. Mark the native window non-activating and keep status/stop interactions from changing the foreground target.
2. **Windows hotkey:** a dedicated Win32 message-loop thread registers `Win + Shift + D` with `RegisterHotKey`; report conflicts in the pill and unregister during shutdown.
3. **Audio session:** open the default CPAL device on activation, use VAD with 300 ms of preroll to form phrases, cap each phrase at 30 seconds, and publish waveform samples. Stopping the session closes capture immediately.
4. **Recognition worker:** retain one CPU Engine for the app lifetime. Transcribe each VAD-delimited segment once; report model/download and inference errors as app-state events.
5. **Text insertion:** remember the foreground window at activation. Insert finalized text with Windows Unicode input only while that target remains foreground; do not activate the pill. If focus has changed, stop insertion and surface the condition instead of typing into an unintended app.


The UI exposes only listening/processing/error status and audio activity. No dashboard or transcript preview is needed for this architecture.

## Verification performed

- `cargo info phonon-rs` resolved version 0.1.3 and confirmed Apache-2.0 metadata.
- `cargo check --manifest-path .phase1-probe/Cargo.toml --target x86_64-pc-windows-msvc` succeeded for a temporary consumer importing `Engine` and `Capture` with `default-features = false, features = ["mic"]`.
- Rust compiler: `rustc 1.96.0 (ac68faa20 2026-05-25)`; target: `x86_64-pc-windows-msvc`.
- No test suite was run. No microphone recording or Phonon model weights were used.

## References

- [`phonon-rs` 0.1.3 API](https://docs.rs/phonon-rs/0.1.3/phonon/)
- [`Engine` API](https://docs.rs/phonon-rs/0.1.3/phonon/engine/struct.Engine.html)
- [`Capture` API](https://docs.rs/phonon-rs/0.1.3/phonon/mic/struct.Capture.html)
- [`mic::run` API](https://docs.rs/phonon-rs/0.1.3/phonon/mic/fn.run.html)
- [Fermion Research: Phonon-2 model, size, language, and weight license](https://www.fermionresearch.com/models/phonon-2/)
- [CPAL platform support](https://github.com/RustAudio/cpal#supported-platforms)
