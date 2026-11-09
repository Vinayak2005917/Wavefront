[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"

$projectRoot = $PSScriptRoot
$packageVersion = "1.0.1.0"
$stagingRoot = Join-Path $projectRoot "msix-staging"
$stagingAssets = Join-Path $stagingRoot "Assets"
$releaseRoot = Join-Path $projectRoot "target\release"
$releaseResources = Join-Path $releaseRoot "resources"
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
$sdkResourceRoot = Join-Path $projectRoot "src-tauri\resources\NeMoSpeech"

function Resize-Png {
    param(
        [Parameter(Mandatory = $true)][string]$SourcePath,
        [Parameter(Mandatory = $true)][string]$DestinationPath,
        [Parameter(Mandatory = $true)][int]$Width,
        [Parameter(Mandatory = $true)][int]$Height
    )

    $source = [System.Drawing.Image]::FromFile($SourcePath)
    try {
        $bitmap = New-Object System.Drawing.Bitmap(
            $Width,
            $Height,
            [System.Drawing.Imaging.PixelFormat]::Format32bppArgb
        )
        try {
            $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
            try {
                $graphics.Clear([System.Drawing.Color]::Transparent)
                $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
                $graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
                $graphics.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
                $graphics.DrawImage($source, 0, 0, $Width, $Height)
            }
            finally {
                $graphics.Dispose()
            }

            $bitmap.Save($DestinationPath, [System.Drawing.Imaging.ImageFormat]::Png)
        }
        finally {
            $bitmap.Dispose()
        }
    }
    finally {
        $source.Dispose()
    }
}

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw "Cargo was not found. Install Rust and ensure cargo is on PATH."
}
if (-not (Get-Command winapp -ErrorAction SilentlyContinue)) {
    throw "The Windows App Development CLI (winapp) was not found. Install it and ensure winapp is on PATH."
}
if (-not (Test-Path -LiteralPath (Join-Path $projectRoot "Package.appxmanifest") -PathType Leaf)) {
    throw "Package.appxmanifest is missing from the project root."
}

foreach ($assetName in @("AppList.png", "MedTile.png", "WideTile.png", "StoreLogo.png")) {
    $assetPath = Join-Path (Join-Path $projectRoot "assets") $assetName
    if (-not (Test-Path -LiteralPath $assetPath -PathType Leaf)) {
        throw "Required MSIX tile image is missing: $assetPath"
    }
}

New-Item -ItemType Directory -Path $sdkCache -Force | Out-Null
if (-not (Test-Path -LiteralPath $sdkArchive -PathType Leaf)) {
    Write-Host "Downloading the NeMo Speech CPU runtime (first build only)..."
    Invoke-WebRequest -Uri $sdkReleaseUrl -OutFile $sdkArchive
}

$actualSha256 = (Get-FileHash -LiteralPath $sdkArchive -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actualSha256 -ne $sdkArchiveSha256) {
    throw "The NeMo Speech runtime checksum did not match. Expected $sdkArchiveSha256, got $actualSha256."
}

if (-not (Test-Path -LiteralPath (Join-Path $sdkBin "nemo_speech_asr_c.dll") -PathType Leaf)) {
    New-Item -ItemType Directory -Path $sdkExtractRoot -Force | Out-Null
    Expand-Archive -LiteralPath $sdkArchive -DestinationPath $sdkExtractRoot -Force
}
foreach ($requiredPath in @((Join-Path $sdkBin "nemo_speech_asr_c.dll"), $sdkLib, $sdkLicenses)) {
    if (-not (Test-Path -LiteralPath $requiredPath)) {
        throw "The downloaded NeMo Speech runtime is missing required content: $requiredPath"
    }
}

$sdkResourceBin = Join-Path $sdkResourceRoot "bin"
$sdkResourceLib = Join-Path $sdkResourceRoot "lib"
$sdkResourceLicenses = Join-Path $sdkResourceRoot "share\licenses\nemo-speech"
New-Item -ItemType Directory -Path $sdkResourceBin, $sdkResourceLib, $sdkResourceLicenses -Force | Out-Null
Copy-Item -Path (Join-Path $sdkBin "*") -Destination $sdkResourceBin -Recurse -Force
Copy-Item -Path (Join-Path $sdkLib "*") -Destination $sdkResourceLib -Recurse -Force
Copy-Item -Path (Join-Path $sdkLicenses "*") -Destination $sdkResourceLicenses -Recurse -Force

New-Item -ItemType Directory -Path $stagingAssets -Force | Out-Null
New-Item -ItemType Directory -Path (Join-Path $stagingRoot "resources") -Force | Out-Null

Write-Host "Building Wavefront (release, without Tauri bundling)..."
Push-Location $projectRoot
try {
    & cargo tauri build --no-bundle
    if ($LASTEXITCODE -ne 0) {
        throw "cargo tauri build --no-bundle failed with exit code $LASTEXITCODE. Close Wavefront if it is running, then retry."
    }
}
finally {
    Pop-Location
}

$builtExe = Join-Path $releaseRoot "wavefront.exe"
if (-not (Test-Path -LiteralPath $builtExe -PathType Leaf)) {
    throw "The build completed without producing $builtExe"
}
if (-not (Test-Path -LiteralPath $releaseResources -PathType Container)) {
    throw "Tauri did not stage runtime resources at $releaseResources"
}
if (-not (Test-Path -LiteralPath (Join-Path $releaseResources "NeMoSpeech\bin\nemo_speech_asr_c.dll") -PathType Leaf)) {
    throw "The NeMo Speech runtime is missing from $releaseResources. Run scripts\build-installer.ps1 once to download and stage it, then retry."
}

Write-Host "Staging executable, runtime resources, manifest, and tile images..."
Copy-Item -LiteralPath $builtExe -Destination (Join-Path $stagingRoot "wavefront.exe") -Force
Copy-Item -Path (Join-Path $releaseResources "*") -Destination (Join-Path $stagingRoot "resources") -Recurse -Force
$manifestSource = Join-Path $projectRoot "Package.appxmanifest"
$manifestDestination = Join-Path $stagingRoot "AppxManifest.xml"

Copy-Item -LiteralPath $manifestSource -Destination $manifestDestination -Force

[xml]$manifest = Get-Content -LiteralPath $manifestDestination -Raw
$identity = $manifest.SelectSingleNode("/*[local-name()='Package']/*[local-name()='Identity']")
if ($null -eq $identity) {
    throw "Could not find Identity element in Package.appxmanifest"
}

$identity.SetAttribute("Version", $packageVersion)
$manifest.Save($manifestDestination)
Write-Host "MSIX package version: $packageVersion"

Add-Type -AssemblyName System.Drawing
$tileSpecs = @(
    @{ Name = "AppList.png"; Width = 44; Height = 44 },
    @{ Name = "MedTile.png"; Width = 150; Height = 150 },
    @{ Name = "WideTile.png"; Width = 310; Height = 150 },
    @{ Name = "StoreLogo.png"; Width = 50; Height = 50 }
)

foreach ($tile in $tileSpecs) {
    $sourcePath = Join-Path (Join-Path $projectRoot "assets") $tile.Name
    foreach ($scale in @(1, 2)) {
        $width = $tile.Width * $scale
        $height = $tile.Height * $scale
        $suffix = if ($scale -eq 1) { "" } else { ".scale-200" }
        $destinationName = $tile.Name -replace "\.png$", "$suffix.png"
        Resize-Png -SourcePath $sourcePath `
            -DestinationPath (Join-Path $stagingAssets $destinationName) `
            -Width $width -Height $height
    }
}

$runtimeDll = Join-Path $stagingRoot "resources\NeMoSpeech\bin\nemo_speech_asr_c.dll"
if (-not (Test-Path -LiteralPath $runtimeDll -PathType Leaf)) {
    throw "The NeMo Speech runtime was not copied into the MSIX staging folder."
}

Write-Host "Packing unsigned MSIX..."
Push-Location $projectRoot
try {
    & winapp pack $stagingRoot --no-sign
    if ($LASTEXITCODE -ne 0) {
        throw "winapp pack failed with exit code $LASTEXITCODE."
    }
}
finally {
    Pop-Location
}

$package = Get-ChildItem -LiteralPath $projectRoot -Filter "*.msix" -File -Recurse |
    Where-Object { $_.FullName -notlike (Join-Path $projectRoot "target\*") } |
    Sort-Object LastWriteTime -Descending |
    Select-Object -First 1

if ($package) {
    Write-Host "MSIX created: $($package.FullName)"
}
else {
    Write-Host "MSIX packaging completed. Check the winapp output above for the package location."
}
