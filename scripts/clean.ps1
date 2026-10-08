$ErrorActionPreference = 'Stop'

$Root = [IO.Path]::GetFullPath((Resolve-Path (Join-Path $PSScriptRoot '..')).Path)
$GitWorktree = (& git -C $Root rev-parse --is-inside-work-tree 2>$null | Select-Object -First 1)
$GitExitCode = $LASTEXITCODE
if ($GitExitCode -ne 0 -or [string]$GitWorktree -ne 'true') {
  throw "Expected a Git worktree at $Root"
}
$Separators = [char[]]@([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar)
$RootWithoutTrailingSeparator = $Root.TrimEnd($Separators)

foreach ($Marker in @('.gitignore', 'desktop-client/pubspec.yaml', 'fengine/Cargo.toml', 'fll/Cargo.toml')) {
  if (-not (Test-Path -LiteralPath (Join-Path $Root $Marker) -PathType Leaf)) {
    throw "Repository marker is missing: $Marker"
  }
}

$RootPrefix = $RootWithoutTrailingSeparator + [IO.Path]::DirectorySeparatorChar
$Targets = @(
  'build',
  'desktop-client/build',
  'desktop-client/.dart_tool',
  'desktop-client/.pub',
  'desktop-client/.pub-cache',
  'desktop-client/.flutter-plugins',
  'desktop-client/.flutter-plugins-dependencies',
  'desktop-client/coverage',
  'desktop-client/macos/Flutter/ephemeral',
  'desktop-client/macos/Pods',
  'desktop-client/windows/flutter/ephemeral',
  'fengine/target',
  'fll/target'
)

foreach ($RelativePath in $Targets) {
  $Path = [IO.Path]::GetFullPath((Join-Path $Root $RelativePath))
  if (-not $Path.StartsWith($RootPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "Refusing to clean a path outside the repository: $Path"
  }
  if (Test-Path -LiteralPath $Path) {
    Write-Output "Removing $RelativePath"
    Remove-Item -LiteralPath $Path -Recurse -Force
  }
}

$Workspace = Join-Path $Root '.workspace'
if (Test-Path -LiteralPath $Workspace -PathType Container) {
  Get-ChildItem -LiteralPath $Workspace -File -Filter '*.log' -Recurse | ForEach-Object {
    $RelativePath = $_.FullName.Substring($RootPrefix.Length)
    Write-Output "Removing $RelativePath"
    Remove-Item -LiteralPath $_.FullName -Force
  }
}
