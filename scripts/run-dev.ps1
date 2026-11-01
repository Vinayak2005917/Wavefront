[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"

# Keep native build artifacts out of a synced project folder. MSVC needs to
# replace temporary static libraries while compiling Phonon and can be blocked
# by file synchronization or endpoint scanning in cloud-backed folders.
$env:CARGO_TARGET_DIR = Join-Path $env:LOCALAPPDATA "Wavefront\cargo-target"

& cargo tauri dev
exit $LASTEXITCODE
