[CmdletBinding()]
param(
    [switch]$AllowDirty
)

$ErrorActionPreference = 'Stop'
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$packageDir = Join-Path $repoRoot 'crates/target/package'
$workDir = [IO.Path]::GetFullPath((Join-Path $repoRoot 'target/package-tests'))

if (-not $workDir.StartsWith($repoRoot, [StringComparison]::OrdinalIgnoreCase)) {
    throw "Refusing to use package-test directory outside repository: $workDir"
}

if (Test-Path -LiteralPath $workDir) {
    Remove-Item -LiteralPath $workDir -Recurse -Force
}
New-Item -ItemType Directory -Path $workDir | Out-Null

function Invoke-Cargo([string[]]$Arguments) {
    & cargo @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "cargo $($Arguments -join ' ') failed with exit code $LASTEXITCODE"
    }
}

$corePath = (Join-Path $repoRoot 'crates/draco-core').Replace('\', '/')
$ioSourcePath = (Join-Path $repoRoot 'crates/draco-io').Replace('\', '/')

# Build both archives without Cargo's per-crate verification, then test their
# exact unpacked contents together through a temporary crates.io patch.
$ioPackageArgs = @(
    'package',
    '--manifest-path', (Join-Path $repoRoot 'crates/draco-io/Cargo.toml'),
    '--no-verify'
)
if ($AllowDirty) { $ioPackageArgs += '--allow-dirty' }
Invoke-Cargo $ioPackageArgs

$gltfPackageArgs = @(
    'package',
    '--manifest-path', (Join-Path $repoRoot 'crates/draco-gltf/Cargo.toml'),
    '--no-verify',
    '--config', "patch.crates-io.draco-core.path='$corePath'",
    '--config', "patch.crates-io.draco-io.path='$ioSourcePath'"
)
if ($AllowDirty) { $gltfPackageArgs += '--allow-dirty' }
Invoke-Cargo $gltfPackageArgs

$ioArchive = Get-ChildItem -LiteralPath $packageDir -Filter 'draco-io-*.crate' |
    Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1
$gltfArchive = Get-ChildItem -LiteralPath $packageDir -Filter 'draco-gltf-*.crate' |
    Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1
if (-not $ioArchive -or -not $gltfArchive) {
    throw "Expected draco-io and draco-gltf package archives in $packageDir"
}

& tar -xzf $ioArchive.FullName -C $workDir
if ($LASTEXITCODE -ne 0) { throw "Failed to unpack $($ioArchive.FullName)" }
& tar -xzf $gltfArchive.FullName -C $workDir
if ($LASTEXITCODE -ne 0) { throw "Failed to unpack $($gltfArchive.FullName)" }

$ioDir = Get-ChildItem -LiteralPath $workDir -Directory -Filter 'draco-io-*' |
    Select-Object -First 1
$gltfDir = Get-ChildItem -LiteralPath $workDir -Directory -Filter 'draco-gltf-*' |
    Select-Object -First 1
if (-not $ioDir -or -not $gltfDir) {
    throw "Expected unpacked draco-io and draco-gltf directories in $workDir"
}

$cargoDir = Join-Path $workDir '.cargo'
New-Item -ItemType Directory -Path $cargoDir | Out-Null
$ioPath = $ioDir.FullName.Replace('\', '/')
@"
[patch.crates-io]
draco-core = { path = '$corePath' }
draco-io = { path = '$ioPath' }
"@ | Set-Content -LiteralPath (Join-Path $cargoDir 'config.toml') -Encoding utf8

Push-Location $workDir
try {
    Invoke-Cargo @('test', '--manifest-path', (Join-Path $ioDir.FullName 'Cargo.toml'))
    Invoke-Cargo @('test', '--manifest-path', (Join-Path $gltfDir.FullName 'Cargo.toml'))
} finally {
    Pop-Location
}
