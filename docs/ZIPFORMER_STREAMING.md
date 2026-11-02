# In-process English Zipformer

Wavefront captures microphone audio with CPAL, continuously resamples it to 16 kHz mono, and feeds it to sherpa-onnx's Rust streaming recognizer. The app does not start a local speech server, Python runtime, or WebSocket client. Inference uses the CPU provider.

## Model setup

On first launch, Wavefront downloads the four required encoder, decoder, joiner, and token files individually from the official Hugging Face model repository into the configured model folder. The dashboard reports file transfer progress; incomplete downloads can resume after a retry. The default is the app's Local AppData `models` folder. The cache folder can be changed in Settings. Existing Fermion and Phonon files are not deleted.

First launch needs an internet connection and enough disk space for the model files. Subsequent launches use the local files. If a download is interrupted, choose **Reload model** in the dashboard to retry.

## Streaming lifecycle

The recognizer is loaded once on the dictation worker. Each dictation creates an independent online stream; incoming 20 ms PCM frames are decoded as soon as the model is ready. The latest hypothesis replaces provisional text. Zipformer endpoint detection commits speech segments; stopping flushes captured audio, adds 300 ms of tail padding, marks the stream finished, and drains remaining decoder frames.

The capture callback only appends mono samples to a bounded in-memory buffer. The worker performs resampling and inference. Audio is discarded after the session. Wavefront does not log or persist recognized words except in the user's existing session history feature.

## Build and packaging

The `sherpa-onnx` Rust crate provides the Rust API over sherpa-onnx's native library. Its build script selects the Windows x64 native package. The application bundles the native runtime through the crate's default static-link configuration, while model weights remain a first-run download. Building requires Rust stable, the MSVC toolchain, Windows SDK, and WebView2.

## Troubleshooting and validation

- **Model download failed:** check network access to GitHub Releases, available disk space, and the error shown in the dashboard. Reload the model to retry.
- **Model files are missing or damaged:** select a writable folder in Settings and reload. Wavefront validates the required files before creating the recognizer.
- **Microphone unavailable:** select the correct Windows default input device and check that another application is not holding it exclusively.
- **No text appears:** check the dashboard recognizer state, speak near the microphone, and try the read-aloud test.

Validate on Windows with long speech, short pauses, mid-sentence corrections, silence, rapid speech, immediate stop, target field changes, manual typing during dictation, and recognizer reload. Measure first partial latency, CPU use, memory use, and recognition quality on the target laptop. Zipformer partials and punctuation should be evaluated on real dictated text before adjusting insertion behavior.

## Upstream references

- [sherpa-onnx Rust API](https://docs.rs/sherpa-onnx/latest/sherpa_onnx/)
- [English streaming Zipformer example](https://github.com/k2-fsa/sherpa-onnx/blob/master/rust-api-examples/README.md)
- [English streaming Zipformer model release](https://github.com/k2-fsa/sherpa-onnx/releases/tag/asr-models)
