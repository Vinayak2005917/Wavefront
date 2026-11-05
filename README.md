# Wavefront

Wavefront is a small Windows 10/11 voice typing app. It uses NVIDIA Nemotron 3.5 ASR Streaming 0.6B Q8_0 through the NeMo-Speech.cpp C SDK. Wavefront inserts live recognition updates into the text control that was focused when dictation started.

## Use

- Start Wavefront. Its dashboard shows model setup progress, session history and settings. Close the window to keep Wavefront running in the notification area; click the tray icon to reopen it.
- The flyout is a compact, non-activating waveform pill.
- Press **Ctrl + Q** (the default shortcut) to start dictation and press it again to stop. Change the shortcut and model folder in Settings.
- The Windows installer includes the NeMo-Speech.cpp CPU runtime. On first launch, Wavefront automatically downloads the 742 MB Nemotron Q8_0 model and caches it locally. Inference runs on the CPU.
- The tray menu can reopen the dashboard or exit Wavefront.

Wavefront captures mono float32 microphone audio at the Windows default input device's native sample rate and sends it to the local streaming recognizer, which resamples internally. Partial hypotheses replace the provisional phrase, and Nemotron commits the final transcript when dictation stops. Text insertion uses native Unicode `SendInput`, verifies the original foreground window and exact focused control, and pauses after keyboard or mouse interaction. Windows may reject synthetic input into higher-integrity applications.

## Build

Requirements: Rust stable with the `x86_64-pc-windows-msvc` target, Visual Studio C++ Build Tools, Windows SDK, and WebView2. No JavaScript package install is needed; the frontend is plain HTML, CSS and JavaScript.

```powershell
./scripts/run-dev.ps1
./scripts/build-installer.ps1
```

`run-dev.ps1` puts Cargo's temporary native build artifacts under Local AppData instead of the project folder. This avoids Windows file locks while MSVC assembles native dependencies, and is the quickest way to run and retest the app.

Install the Tauri CLI if needed with `cargo install tauri-cli --locked`. Run `./scripts/build-installer.ps1` to create a per-user NSIS installer under `target/release/bundle/nsis/`. The script downloads or reuses NVIDIA's checksum-verified Windows x64 CPU SDK package, stages the NeMo ASR runtime and its licenses, then builds Wavefront. The installer bundles that runtime. Tauri installs WebView2 silently if it is missing; this step needs an internet connection. The 742 MB model downloads automatically the first time Wavefront starts and also needs an internet connection. The model is cached for later launches.

The microphone capture uses the input device's preferred Windows audio buffer size. If `A buffer underrun or overrun occurred` persists, select the correct Windows input device and close other apps that are recording from it.

## Runtime setup and tests

See [Nemotron setup](docs/NEMOTRON_STREAMING.md) for runtime packaging and model-cache details. The Rust app manages the automatic model download and inference.

Audio is kept in memory and discarded when dictation ends. Wavefront does not save recordings or log recognized speech. Existing Fermion caches and previous model files are left untouched by this migration.

Model and library attribution is included in the installer at `THIRD_PARTY_NOTICES.txt`. See [the Phase 1 capability review](docs/PHONON_PHASE1.md) for historical API assessment.
