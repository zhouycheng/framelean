param([Parameter(Mandatory=$true)][string]$FFmpegRoot)
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '../..')
$FFmpegRoot = (Resolve-Path $FFmpegRoot).Path
# Requires a single matching shared FFmpeg SDK (headers, import libraries and DLLs), including libx264.
$env:FFMPEG_INCLUDE_DIR = Join-Path $FFmpegRoot 'include'
$env:FFMPEG_LIBS_DIR = Join-Path $FFmpegRoot 'lib'
$env:FFMPEG_LINK_MODE = 'dynamic'
$env:PATH = "$(Join-Path $FFmpegRoot 'bin');$env:PATH"
function Check-Native { if ($LASTEXITCODE -ne 0) { throw "Command failed: $LASTEXITCODE" } }
cargo build --release --locked --manifest-path fll/Cargo.toml; Check-Native
cargo build --release --locked --manifest-path fengine/Cargo.toml; Check-Native
Push-Location desktop-client
try { flutter build windows --release; Check-Native } finally { Pop-Location }
$bundle = 'desktop-client/build/windows/x64/runner/Release'
$runtime = Join-Path $bundle 'runtime'
New-Item -ItemType Directory -Force $runtime | Out-Null
Copy-Item fengine/target/release/framelean-engine.exe $runtime -Force
Copy-Item fll/target/release/framelean_fll.dll $runtime -Force
Copy-Item (Join-Path $FFmpegRoot 'bin/*.dll') $runtime -Force
& (Join-Path $runtime 'framelean-engine.exe') probe (Join-Path (Resolve-Path $runtime).Path 'framelean_fll.dll'); Check-Native
Write-Output "Bundle: $bundle (copy the entire directory)"
