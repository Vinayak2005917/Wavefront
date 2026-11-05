# Wavefront

Wavefront is a Windows voice-typing app built with Tauri, Rust, and a plain HTML/CSS/JavaScript frontend. It captures microphone audio with CPAL, streams it to NVIDIA Nemotron 3.5 ASR through the NeMo-Speech.cpp CPU runtime, and inserts recognized text into the focused Windows text field.

## Build

Requirements: Windows, Rust stable with the `x86_64-pc-windows-msvc` target, Visual Studio C++ Build Tools, Windows SDK, and WebView2. No frontend package install is needed.

```powershell
./scripts/run-dev.ps1
./scripts/build-installer.ps1
```

The installer script downloads the NeMo-Speech.cpp runtime and verifies its SHA-256 checksum before packaging it. On first launch, Wavefront downloads the Nemotron Q8_0 model, verifies it, and caches it locally. Both downloads require an internet connection; recognition runs on the CPU.

## Implementation

The default global shortcut is **Ctrl+Q**; it can be changed in the dashboard. Dictation streams partial and final results into the text field that was focused when recording began. The dashboard and system tray provide model status, settings, and optional transcript history. Audio stays in memory and is discarded after dictation.
