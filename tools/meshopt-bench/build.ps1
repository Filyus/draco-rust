param(
    [string] $CacheRoot = (Join-Path $env:LOCALAPPDATA 'draco-rust/meshopt-bench')
)

$ErrorActionPreference = 'Stop'
$revisions = Get-Content -Raw -LiteralPath (Join-Path $PSScriptRoot 'meshoptimizer.json') | ConvertFrom-Json
$CacheRoot = [System.IO.Path]::GetFullPath($CacheRoot)
$scratchRoot = Join-Path $CacheRoot 'build'
$binRoot = Join-Path $CacheRoot 'bin'
$sourceCache = Join-Path $CacheRoot 'meshoptimizer.git'

function Invoke-BenchGit {
    & git @args
    if ($LASTEXITCODE -ne 0) {
        throw "Git failed (exit ${LASTEXITCODE}): $($args -join ' ')"
    }
}

function Get-PinnedSource([string] $Revision) {
    $checkout = Join-Path $CacheRoot "meshoptimizer-$Revision"
    if (-not (Test-Path -LiteralPath $checkout)) {
        & git "--git-dir=$sourceCache" cat-file -e "${Revision}^{commit}" 2>$null
        if ($LASTEXITCODE -ne 0) {
            Invoke-BenchGit "--git-dir=$sourceCache" fetch --depth=1 --no-tags https://github.com/zeux/meshoptimizer.git $Revision | Out-Host
        }
        Invoke-BenchGit "--git-dir=$sourceCache" worktree add --detach $checkout $Revision | Out-Host
    }
    $head = Invoke-BenchGit -C $checkout rev-parse HEAD
    $changes = Invoke-BenchGit -C $checkout status --porcelain
    if ($head -ne $Revision -or $changes) {
        throw "Cached source at $checkout must be clean and at $Revision. Use a fresh -CacheRoot."
    }
    return $checkout
}

if (-not $env:VSCMD_VER) {
    $compilerPath = (Get-Command cl.exe -ErrorAction Stop).Source
    $vsRoot = $compilerPath
    0..7 | ForEach-Object { $vsRoot = Split-Path $vsRoot -Parent }
    $vsDevCmd = Join-Path $vsRoot 'Common7/Tools/VsDevCmd.bat'
    if (-not (Test-Path -LiteralPath $vsDevCmd)) {
        throw "Visual Studio developer environment was not found at $vsDevCmd"
    }
    $loadEnvironment = '"' + $vsDevCmd + '" -no_logo -arch=x64 -host_arch=x64 >nul && set'
    $environment = & $env:ComSpec /d /s /c $loadEnvironment
    if ($LASTEXITCODE -ne 0) {
        throw 'Could not load the Visual Studio developer environment.'
    }
    foreach ($entry in $environment) {
        $equals = $entry.IndexOf('=')
        if ($equals -gt 0) {
            Set-Item -Path ("Env:" + $entry.Substring(0, $equals)) -Value $entry.Substring($equals + 1)
        }
    }
}

$compiler = (Get-Command cl.exe -ErrorAction Stop).Source

New-Item -ItemType Directory -Force -Path $scratchRoot, $binRoot | Out-Null
if (-not (Test-Path -LiteralPath $sourceCache)) {
    Invoke-BenchGit init --bare $sourceCache | Out-Host
}
$releaseMeshopt = Get-PinnedSource $revisions.release.commit
$splatMeshopt = Get-PinnedSource $revisions.splat_experiment.commit

foreach ($required in @(
    (Join-Path $releaseMeshopt 'src/meshoptimizer.h'),
    (Join-Path $splatMeshopt 'gltf/splats.cpp'),
    (Join-Path $splatMeshopt 'gltf/gltfpack.cpp')
)) {
    if (-not (Test-Path -LiteralPath $required)) {
        throw "Pinned upstream source is missing: $required"
    }
}

function Invoke-MsvcBuild {
    param(
        [Parameter(Mandatory = $true)][string] $Name,
        [Parameter(Mandatory = $true)][string] $OutputPath,
        [Parameter(Mandatory = $true)][string[]] $IncludePaths,
        [Parameter(Mandatory = $true)][string[]] $Sources,
        [string[]] $LinkArguments = @()
    )

    $buildDir = Join-Path $scratchRoot $Name
    New-Item -ItemType Directory -Force -Path $buildDir | Out-Null
    Push-Location $buildDir
    try {
        $arguments = @('/nologo', '/O2', '/GL', '/EHsc', '/std:c++14', '/MT')
        foreach ($includePath in $IncludePaths) {
            $arguments += "/I$includePath"
        }
        $arguments += "/Fe$OutputPath"
        $arguments += $Sources
        if ($LinkArguments.Count -gt 0) {
            $arguments += '/link'
            $arguments += '/LTCG'
            $arguments += $LinkArguments
        } else {
            $arguments += '/link'
            $arguments += '/LTCG'
        }
        & $compiler @arguments
        if ($LASTEXITCODE -ne 0) {
            throw "MSVC build failed for $Name (exit $LASTEXITCODE)."
        }
    } finally {
        Pop-Location
    }
}

$meshoptSources = @(Get-ChildItem -LiteralPath (Join-Path $releaseMeshopt 'src') -Filter '*.cpp' -File | ForEach-Object FullName)
$bridgeSource = Join-Path $PSScriptRoot 'meshopt_bridge.cpp'
Invoke-MsvcBuild -Name 'meshopt-bridge' `
    -OutputPath (Join-Path $binRoot 'meshopt-bridge.exe') `
    -IncludePaths @((Join-Path $releaseMeshopt 'src')) `
    -Sources (@($bridgeSource) + $meshoptSources) `
    -LinkArguments @('Psapi.lib')

$releaseGltfSources = @(Get-ChildItem -LiteralPath (Join-Path $releaseMeshopt 'gltf') -Filter '*.cpp' -File | ForEach-Object FullName)
Invoke-MsvcBuild -Name 'gltfpack-v1_3' `
    -OutputPath (Join-Path $binRoot 'gltfpack-v1_3.exe') `
    -IncludePaths @((Join-Path $releaseMeshopt 'src'), (Join-Path $releaseMeshopt 'extern'), (Join-Path $releaseMeshopt 'gltf')) `
    -Sources ($meshoptSources + $releaseGltfSources)

$splatSources = @(Get-ChildItem -LiteralPath (Join-Path $splatMeshopt 'src') -Filter '*.cpp' -File | ForEach-Object FullName)
$gltfSources = @(Get-ChildItem -LiteralPath (Join-Path $splatMeshopt 'gltf') -Filter '*.cpp' -File | ForEach-Object FullName)
Invoke-MsvcBuild -Name 'gltfpack-gaussian-splats' `
    -OutputPath (Join-Path $binRoot 'gltfpack-gs.exe') `
    -IncludePaths @((Join-Path $splatMeshopt 'src'), (Join-Path $splatMeshopt 'extern'), (Join-Path $splatMeshopt 'gltf')) `
    -Sources ($splatSources + $gltfSources)

& (Join-Path $binRoot 'meshopt-bridge.exe') 2>$null
if ($LASTEXITCODE -ne 2) {
    throw 'meshopt bridge usage check did not return the expected status.'
}
& (Join-Path $binRoot 'gltfpack-gs.exe') -v
if ($LASTEXITCODE -ne 0) {
    throw 'gltfpack version check failed.'
}
& (Join-Path $binRoot 'gltfpack-v1_3.exe') -v
if ($LASTEXITCODE -ne 0) {
    throw 'gltfpack v1.3 version check failed.'
}
