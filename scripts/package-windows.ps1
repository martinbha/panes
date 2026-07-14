param(
    [string]$TargetDir,
    [string]$DistDir
)

$ErrorActionPreference = "Stop"

$RootDir = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
if ([string]::IsNullOrWhiteSpace($TargetDir)) {
    $TargetDir = Join-Path $RootDir "target"
}
if ([string]::IsNullOrWhiteSpace($DistDir)) {
    $DistDir = Join-Path $RootDir "dist"
}

$WorkspaceManifest = Get-Content (Join-Path $RootDir "Cargo.toml") -Raw
$VersionMatch = [regex]::Match($WorkspaceManifest, '(?m)^version = "([^"]+)"')
if (-not $VersionMatch.Success) {
    throw "Could not determine the workspace version"
}
$Version = $VersionMatch.Groups[1].Value

$Architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture
if ($Architecture -ne [System.Runtime.InteropServices.Architecture]::X64) {
    throw "Unsupported Windows packaging architecture: $Architecture"
}

& cargo build --manifest-path (Join-Path $RootDir "Cargo.toml") --package panes-app --release --target-dir $TargetDir
if ($LASTEXITCODE -ne 0) {
    throw "Cargo release build failed with exit code $LASTEXITCODE"
}

$Binary = Join-Path (Join-Path $TargetDir "release") "panes.exe"
if (-not (Test-Path $Binary -PathType Leaf)) {
    throw "Expected release binary at $Binary"
}

New-Item -ItemType Directory -Force -Path $DistDir | Out-Null
$PackageName = "panes-v$Version-windows-x86_64"
$StagingDir = Join-Path $DistDir $PackageName
$Archive = Join-Path $DistDir "$PackageName.zip"

Remove-Item $StagingDir -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item $Archive -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Path $StagingDir | Out-Null

try {
    Copy-Item $Binary (Join-Path $StagingDir "panes.exe")
    Copy-Item (Join-Path $RootDir "README.md") (Join-Path $StagingDir "README.md")
    Compress-Archive -Path (Join-Path $StagingDir "*") -DestinationPath $Archive
} finally {
    Remove-Item $StagingDir -Recurse -Force -ErrorAction SilentlyContinue
}

Write-Output "Created $Archive"
