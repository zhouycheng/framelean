param([Parameter(Mandatory=$true)][string]$FFmpegRoot)
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '../..')
$FFmpegRoot = (Resolve-Path $FFmpegRoot).Path
$env:FFMPEG_INCLUDE_DIR = Join-Path $FFmpegRoot 'include'
$env:FFMPEG_LIBS_DIR = Join-Path $FFmpegRoot 'lib'
$env:FFMPEG_LINK_MODE = 'dynamic'
$env:PATH = "$(Join-Path $FFmpegRoot 'bin');$env:PATH"
function Check-Native { if ($LASTEXITCODE -ne 0) { throw "Command failed: $LASTEXITCODE" } }
cargo build --release --locked --manifest-path fll/Cargo.toml; Check-Native
cargo build --release --locked --manifest-path fengine/Cargo.toml; Check-Native
$runtime = 'outputs/cli-windows'
if (Test-Path $runtime) { Remove-Item $runtime -Recurse -Force }
New-Item -ItemType Directory -Force $runtime | Out-Null
Copy-Item fengine/target/release/framelean.exe,fengine/target/release/framelean-engine.exe $runtime
Copy-Item fll/target/release/framelean_fll.dll $runtime
Copy-Item (Join-Path $FFmpegRoot 'bin/*.dll') $runtime
& (Join-Path $runtime 'framelean-engine.exe') probe (Join-Path (Resolve-Path $runtime).Path 'framelean_fll.dll'); Check-Native
Write-Output "CLI bundle: $runtime (copy the entire directory)"
