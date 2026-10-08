$ErrorActionPreference = 'Stop'
$Root = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$Runtime = Join-Path $Root 'build/runtime/windows-x64'
$Engine = Join-Path $Runtime 'framelean-engine.exe'
$Fll = Join-Path $Runtime 'framelean_fll.dll'

if (-not (Test-Path -LiteralPath $Engine) -or -not (Test-Path -LiteralPath $Fll)) {
  throw 'Build the Rust runtime in an MSYS2 MINGW64 shell first: bash scripts/build/build_runtime_windows.sh'
}

$env:FRAMELEAN_FENGINE_BINARY = $Engine
Remove-Item Env:FRAMELEAN_FLL_LIBRARY -ErrorAction SilentlyContinue
Push-Location (Join-Path $Root 'desktop-client')
try {
  flutter pub get
  if ($LASTEXITCODE -ne 0) { throw 'flutter pub get failed' }
  flutter analyze
  if ($LASTEXITCODE -ne 0) { throw 'flutter analyze failed' }
  flutter test --reporter expanded
  if ($LASTEXITCODE -ne 0) { throw 'Flutter tests or the Work → FLL smoke failed' }
  flutter build windows --release
  if ($LASTEXITCODE -ne 0) { throw 'Flutter Windows release build failed' }
} finally {
  Pop-Location
}

$AppDirectory = Join-Path $Root 'desktop-client/build/windows/x64/runner/Release'
$BundledEngine = Join-Path $AppDirectory 'framelean-engine.exe'
$BundledFll = Join-Path $AppDirectory 'framelean_fll.dll'
if (-not (Test-Path -LiteralPath $BundledEngine) -or -not (Test-Path -LiteralPath $BundledFll)) {
  throw 'The Windows package is missing framelean-engine.exe or framelean_fll.dll'
}
if (-not (Test-Path -LiteralPath (Join-Path $AppDirectory 'legal/LICENSE')) -or
    -not (Test-Path -LiteralPath (Join-Path $AppDirectory 'legal/NOTICE.md'))) {
  throw 'The Windows package is missing legal materials'
}

$env:FRAMELEAN_FENGINE_BINARY = $BundledEngine
Remove-Item Env:FRAMELEAN_FLL_LIBRARY -ErrorAction SilentlyContinue
Push-Location (Join-Path $Root 'desktop-client')
try {
  flutter test test/engine_gateway_smoke_test.dart --reporter expanded
  if ($LASTEXITCODE -ne 0) { throw 'Packaged Work/FLL smoke failed' }
} finally {
  Pop-Location
}

$PackageDirectory = Join-Path $Root 'build/packages'
New-Item -ItemType Directory -Force -Path $PackageDirectory | Out-Null
$Archive = Join-Path $PackageDirectory 'FrameLean-windows-x64.zip'
if (Test-Path -LiteralPath $Archive) { Remove-Item -LiteralPath $Archive -Force }
Compress-Archive -Path (Join-Path $AppDirectory '*') -DestinationPath $Archive
Write-Output "Packaged and smoke-tested $Archive"
