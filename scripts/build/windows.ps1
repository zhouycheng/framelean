param([Parameter(Mandatory=$true)][string]$FFmpegRoot)
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '../..')
& "$PSScriptRoot/cli-windows.ps1" -FFmpegRoot $FFmpegRoot
function Check-Native { if ($LASTEXITCODE -ne 0) { throw "Command failed: $LASTEXITCODE" } }
Push-Location desktop-client
try { flutter build windows --release; Check-Native } finally { Pop-Location }
$bundle = 'desktop-client/build/windows/x64/runner/Release'
$runtime = Join-Path $bundle 'runtime'
New-Item -ItemType Directory -Force $runtime | Out-Null
Copy-Item outputs/cli-windows/* $runtime -Force
& (Join-Path $runtime 'framelean-engine.exe') probe (Join-Path (Resolve-Path $runtime).Path 'framelean_fll.dll'); Check-Native
Write-Output "Bundle: $bundle (copy the entire directory)"
