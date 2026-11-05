# Nemotron 3.5 streaming ASR

Wavefront prefers NVIDIA Nemotron 3.5 ASR Streaming 0.6B in Q8_0 GGUF format. The Rust adapter calls NVIDIA's NeMo-Speech.cpp ASR C SDK in-process. CPAL captures mono float32 audio at the Windows default input device's native rate and sends it in 20 ms frames; the SDK performs the conversion to the model rate. The existing Sherpa-ONNX Zipformer recognizer remains the CPU fallback and also receives native-rate frames.

## CPU runtime

Inference is configured with `gpu = -1`. No CUDA toolkit, NVIDIA GPU, Python runtime, speech server, or cloud service is required. To use Nemotron, install NVIDIA's Windows x64 CPU release of NeMo-Speech.cpp (v0.3.0 or newer) using the [official install instructions](https://github.com/NVIDIA/NeMo-Speech.cpp/blob/main/docs/install.md). Choose the `cpu` backend. The default per-user SDK path `%LOCALAPPDATA%\Programs\NeMoSpeech\bin` is detected automatically. If the SDK is installed elsewhere, set `NEMO_SPEECH_SDK_DIR` to its prefix or `bin` directory before starting Wavefront. Keep the SDK `bin` and `lib` directories together so Windows can resolve its runtime DLLs. The SDK accepts mono float32 audio from 8 to 96 kHz and resamples it internally.

The app loads the SDK dynamically, so the SDK is not needed to compile Wavefront. The existing Zipformer fallback still requires its Sherpa-ONNX native package at build time. Building the SDK from source instead of installing NVIDIA's CPU binary requires Git, CMake 3.26 or newer, Ninja, Visual Studio 2022 C++ Build Tools, and SentencePiece development files. Neither route needs CUDA for CPU inference.

## Model cache

On first Nemotron startup, Wavefront downloads NVIDIA's `nemotron-3.5-asr-streaming-0.6b.q8_0.gguf` from Hugging Face. The pinned Q8_0 artifact is about 742 MB. It is stored under the configured model cache folder in `nemotron-3.5-asr-streaming-0.6b-q8_0/` and checked against its SHA-256 before the SDK loads it. Later starts use that local copy. If the SDK or Nemotron model is unavailable, startup reports the reason and loads the retained Zipformer fallback.

To force Zipformer for a controlled A/B run, set `WAVEFRONT_ASR_BACKEND=zipformer` before starting Wavefront. The default prefers Nemotron. Clear the variable and restart to return to the preferred backend.

## Stream lifecycle

The SDK recognizer is initialized during worker startup, before a microphone session begins. Each dictation opens one SDK stream. Native-rate CPAL frames are pushed once, then the SDK is polled for interim and final snapshots; Wavefront does not resend or re-decode the accumulated recording. RNNT right context is set to `13`, matching NVIDIA's `[56, 13]` streaming example (a 1.12 s chunk). Endpointing is disabled to use the SDK default and avoid token-silence false endpoints; interim text remains live, and the final transcript is committed when dictation stops. Dropping a session closes its SDK stream, and dropping the engine destroys the recognizer before unloading the SDK DLL.

## A/B validation

Use the app's **Start test** flow with the same deterministic recording routed to the same Windows input device for both runs. Start one Wavefront process with the default Nemotron backend, then another with `WAVEFRONT_ASR_BACKEND=zipformer`. Keep the same machine, microphone path, device sample rate, gain, silence, and background load.

Record the known reference transcript, each final transcript, the first partial time and subsequent update times from the test report, process CPU use, and peak working set. Compare recognition with word error rate after applying the same text normalization to both outputs. Repeat each run to account for Windows scheduling and microphone timing. Wavefront does not save microphone recordings; route an existing file through a virtual input device or use an external deterministic playback harness.

No accuracy, CPU, RAM, or partial-latency result is claimed until that paired run has been completed. The current integration environment has no NeMo SDK install or reference recordings, so those measurements remain pending.

## Upstream artifacts

- [NeMo-Speech.cpp native C SDK](https://github.com/NVIDIA/NeMo-Speech.cpp/blob/main/docs/sdk.md)
- [Nemotron 3.5 ASR model and Q8_0 file](https://huggingface.co/nvidia/nemotron-3.5-asr-streaming-0.6b/tree/ea30d66debe3740a08b573244286791d423d6b3e)
- [Zipformer fallback setup](ZIPFORMER_STREAMING.md)
