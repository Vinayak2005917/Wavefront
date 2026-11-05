[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"

$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$sdkVersion = "0.3.0"
$sdkArchiveName = "nemo-speech-$sdkVersion-windows-x86_64-cpu.zip"
$sdkArchiveSha256 = "4ed9740e361e4653f56c3dce7d1e81883a1c8b831c1bd7d01567fff1130b5b05"
$sdkReleaseUrl = "https://github.com/NVIDIA/NeMo-Speech.cpp/releases/download/v$sdkVersion/$sdkArchiveName"
$sdkCache = Join-Path $projectRoot "target\nemo-speech-sdk-package"
$sdkArchive = Join-Path $sdkCache $sdkArchiveName
$sdkExtractRoot = Join-Path $sdkCache "runtime"
$sdkRoot = Join-Path $sdkExtractRoot "nemo-speech-$sdkVersion-windows-x86_64-cpu"
$sdkBin = Join-Path $sdkRoot "bin"
$sdkLib = Join-Path $sdkRoot "lib"
$sdkLicenses = Join-Path $sdkRoot "share\licenses\nemo-speech"
$resourceRoot = Join-Path $projectRoot "src-tauri\resources\NeMoSpeech"

New-Item -ItemType Directory -Path $sdkCache -Force | Out-Null
if (-not (Test-Path -LiteralPath $sdkArchive -PathType Leaf)) {
    Write-Host "Downloading NVIDIA NeMo-Speech.cpp $sdkVersion CPU runtime..."
    Invoke-WebRequest -Uri $sdkReleaseUrl -OutFile $sdkArchive
}

$actualSha256 = (Get-FileHash -LiteralPath $sdkArchive -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actualSha256 -ne $sdkArchiveSha256) {
    throw "The NeMo-Speech.cpp package checksum did not match the published SHA-256. Expected $sdkArchiveSha256, got $actualSha256."
}

$asrLibrary = Join-Path $sdkBin "nemo_speech_asr_c.dll"
if (-not (Test-Path -LiteralPath $asrLibrary -PathType Leaf)) {
    New-Item -ItemType Directory -Path $sdkExtractRoot -Force | Out-Null
    Expand-Archive -LiteralPath $sdkArchive -DestinationPath $sdkExtractRoot -Force
}

foreach ($requiredPath in @($asrLibrary, $sdkLib, $sdkLicenses)) {
    if (-not (Test-Path -LiteralPath $requiredPath)) {
        throw "The NeMo-Speech.cpp package is missing a required runtime path: $requiredPath"
    }
}

$resourceBin = Join-Path $resourceRoot "bin"
$resourceLib = Join-Path $resourceRoot "lib"
$resourceLicenses = Join-Path $resourceRoot "share\licenses\nemo-speech"
New-Item -ItemType Directory -Path $resourceBin,$resourceLib,$resourceLicenses -Force | Out-Null
Copy-Item -Path (Join-Path $sdkBin "*") -Destination $resourceBin -Recurse -Force
Copy-Item -Path (Join-Path $sdkLib "*") -Destination $resourceLib -Recurse -Force
Copy-Item -Path (Join-Path $sdkLicenses "*") -Destination $resourceLicenses -Recurse -Force

if (-not (Test-Path -LiteralPath (Join-Path $resourceBin "nemo_speech_asr_c.dll") -PathType Leaf)) {
    throw "The NeMo ASR runtime was not staged for the installer."
}

Push-Location $projectRoot
try {
    & cargo tauri build
    if ($LASTEXITCODE -ne 0) {
        throw "cargo tauri build failed with exit code $LASTEXITCODE."
    }
} finally {
    Pop-Location
}

$bundleDirectory = Join-Path $projectRoot "target\release\bundle\nsis"
$installer = Get-ChildItem -LiteralPath $bundleDirectory -Filter "*setup.exe" -File |
    Sort-Object LastWriteTime -Descending |
    Select-Object -First 1
if (-not $installer) {
    throw "Tauri completed without producing an NSIS setup executable in $bundleDirectory."
}

Write-Host "Installer created: $($installer.FullName)"
