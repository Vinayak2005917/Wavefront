<div align="center">
  <img src="assets\README.png" width="80" align="left" />
  <h1>Wavefront</h1>
</div>
<br clear="left"/>
Wavefront is a Windows app built with Tauri, Rust, and a plain HTML frontend. It captures microphone audio with CPAL, streams it to NVIDIA Nemotron 3.5 ASR through the NeMo-Speech.cpp CPU runtime, and inserts recognized text into the focused Windows text field.

<p align="center">
  <img src="demo.gif" width="300" alt="Description">
</p>

Download the latest release from [Github](https://github.com/Vinayak2005917/Wavefront/releases/tag/First_release) or [Microsoft Store](https://apps.microsoft.com/detail/9PLW5T13V1BJ?hl=en-us&gl=IN&ocid=pdpshare).

## Local Build

Requirements: Windows, Rust stable with the `x86_64-pc-windows-msvc` target, Visual Studio C++ Build Tools, Windows SDK, and WebView2. No frontend package install is needed.

```powershell
./scripts/run-dev.ps1
./scripts/build-installer.ps1
```

The installer script downloads the NeMo-Speech.cpp runtime and verifies its SHA-256 checksum before packaging it. On first launch, Wavefront downloads the Nemotron Q8_0 model, verifies it, and caches it locally. Both downloads require an internet connection; recognition runs on the CPU.

