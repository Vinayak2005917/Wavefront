# Wavefront

Wavefront is a small Windows 10/11 voice typing app. It runs the English streaming Zipformer recognizer in-process through sherpa-onnx's Rust API. Wavefront inserts live recognition updates into the text control that was focused when dictation started.

## Use

- Start Wavefront. Its dashboard shows model setup progress, session history and settings. Close the window to keep Wavefront running in the notification area; click the tray icon to reopen it.
- The flyout is a compact, non-activating waveform pill.
- Press **Ctrl + Q** (the default shortcut) to start dictation and press it again to stop. Change the shortcut and model folder in Settings.
- Wavefront downloads the English streaming Zipformer model on first launch. Later inference runs locally from the cached model files.
- The tray menu can reopen the dashboard or exit Wavefront.

Wavefront captures and resamples microphone audio to 16 kHz mono, then feeds it directly to the local streaming recognizer. Partial hypotheses replace only the provisional phrase; final segments are committed once. Text insertion uses native Unicode `SendInput`, verifies the original foreground window and exact focused control, and pauses after keyboard or mouse interaction. Windows may reject synthetic input into higher-integrity applications.

## Build

Requirements: Rust stable with the `x86_64-pc-windows-msvc` target, Visual Studio C++ Build Tools, Windows SDK, and WebView2. No JavaScript package install is needed; the frontend is plain HTML, CSS and JavaScript.

```powershell
./scripts/run-dev.ps1
cargo tauri build
```

`run-dev.ps1` puts Cargo's temporary native build artifacts under Local AppData instead of the project folder. This avoids Windows file locks while MSVC assembles native dependencies, and is the quickest way to run and retest the app.

Install the Tauri CLI if needed with `cargo install tauri-cli --locked`. Run `cargo tauri build` from the repository root to create the NSIS installer under `target/release/bundle/nsis/`. The app executable is `target/release/wavefront.exe`. The installer is per-user and bundles the sherpa-onnx native runtime and Visual C++ runtime. Tauri uses the Windows WebView2 runtime; its bootstrapper can download it if needed. Model weights are downloaded separately on first launch.

The microphone capture uses the input device's preferred Windows audio buffer size. If `A buffer underrun or overrun occurred` persists, select the correct Windows input device and close other apps that are recording from it.

## Runtime setup and tests

See [Zipformer setup](docs/ZIPFORMER_STREAMING.md) for model cache setup, runtime packaging notes, diagnostics and Windows validation. Model downloads and inference are managed by the Rust application.

Audio is kept in memory and discarded when dictation ends. Wavefront does not save recordings or log recognized speech. Existing Fermion caches and previous model files are left untouched by this migration.

Model and library attribution is included in the installer at `THIRD_PARTY_NOTICES.txt`. See [the Phase 1 capability review](docs/PHONON_PHASE1.md) for historical API assessment.
