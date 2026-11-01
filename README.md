# Wavefront

Wavefront is a small Windows 10/11 voice typing app. It runs Fermion Research's Phonon-2 model locally and inserts finalized words into the window that was focused when dictation started.

## Use

- Start `Wavefront`. Its simple black-and-white window shows Phonon-2 setup progress and the current session's transcription timeline. Close the window to keep Wavefront running in the notification area; click the tray icon to reopen it.
- The flyout is a small dark-gray pill with a light-gray volume bar timeline only. New bars enter on the right and move left as they become older. It has no controls, border, or shadow.
- The bars meter the microphone directly and keep responding while Phonon-2 runs recognition.
- Press **Ctrl + Q** (the default shortcut) to start dictation and press it again to stop. Change it, and the model folder, in the dashboard Settings section.
- The first run downloads the Phonon-2 model (about 164 MB). Later recognition runs locally without network access.
- The flyout remains a compact waveform while dictating. Use the tray menu to reopen Wavefront or exit.

The waveform does not activate while it appears. Text insertion uses Unicode `SendInput` and checks that the original foreground window is still active before each insertion. If focus changes, it stops injecting to avoid typing into a different app. Windows does not allow synthetic input into higher-integrity apps from a lower-integrity process; those apps may reject insertion.

## Build

Requirements: Rust stable with the `x86_64-pc-windows-msvc` target, Visual Studio C++ Build Tools, Windows SDK, and WebView2. No JavaScript package install is needed; the frontend is plain HTML, CSS, and JavaScript.

```powershell
./scripts/run-dev.ps1
cargo tauri build
```

`run-dev.ps1` puts Cargo's temporary native build artifacts under Local AppData instead of the project folder. This avoids Windows file locks while MSVC assembles Phonon's native dependencies, and is the quickest way to run and retest the local app.

Install the Tauri CLI if needed with `cargo install tauri-cli --locked`. Run `cargo tauri build` from the repository root to create the NSIS installer at `target/release/bundle/nsis/` (or under the configured Cargo target directory). The app executable is `target/release/wavefront.exe`. The installer and app icon come from `assets/logo.ico`, and the notification-area icon image is `assets/tray.rgba`. The installer is per-user and bundles the Visual C++ runtime. Tauri uses the Windows WebView2 runtime; its default bootstrapper can download the runtime if Windows does not already have it. This app does not embed the ASR model in the executable.

To build only the Rust executable, without an installer, run `cargo build --release --manifest-path src-tauri/Cargo.toml`; it is written to `target/release/wavefront.exe`. The executable is not fully self-contained: it needs the Tauri frontend files and a Phonon-2 model. On first launch it downloads the model archive to `%LOCALAPPDATA%\\com.wavefront.dictation\\models\\Phonon-2\\phonon-2.bps.tar.zst`. If you place it there yourself, the exact required file is `phonon-2.bps.tar.zst`; keep it as the original compressed archive (do not unpack it). Otherwise the app downloads it automatically. No other model files are required.

The microphone capture uses the input device's preferred Windows audio buffer size. This avoids forcing a 10 ms WASAPI period, which can produce `A buffer underrun or overrun occurred` callback errors on some devices. If the message persists after rebuilding, select the correct Windows input device and close other apps that are recording from it.

## Recognition behavior

Wavefront begins transcription after the rolling buffer fills to four seconds, then submits the latest window every 300 ms when the recognition worker is available. Stopping earlier submits the shorter final window. Phonon-2 does not expose incremental decoder state, so each update re-transcribes the window. Audio capture continues on its own while inference runs. Word timestamps align each window to the session timeline, so text remains in the transcript after its audio leaves the rolling buffer. Words stable for three updates are committed, with the last three committed words kept revisable. The writer types words individually and uses word-level backspaces to replace a revised suffix. The flyout continues to show only the waveform and status, not transcript text.

Audio remains in memory for recognition and is discarded when the session ends. Wavefront does not save recordings or log recognized speech. The model archive remains under the current user's Tauri application data directory so it can be reused offline.

Model and library attribution is included in the installer at `THIRD_PARTY_NOTICES.txt`. See [the Phase 1 capability review](docs/PHONON_PHASE1.md) for the API assessment.

See the [minimum MVP release checklist](docs/MVP_RELEASE_CHECKLIST.md) for the remaining steps before sharing a beta installer or publishing a GitHub release.
