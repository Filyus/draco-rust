param(
    [string] $CorpusPath,
    [switch] $DownloadCorpus,
    [string] $CacheRoot = (Join-Path $env:LOCALAPPDATA 'draco-rust/meshopt-bench'),
    [switch] $SkipSynthetic,
    [switch] $SkipGltfIntegration,
    [switch] $OnlyGltfIntegration
)

$ErrorActionPreference = 'Stop'
if ($OnlyGltfIntegration -and $SkipGltfIntegration) {
    throw '-OnlyGltfIntegration cannot be combined with -SkipGltfIntegration.'
}
if ($DownloadCorpus -and $CorpusPath) {
    throw 'Choose either -DownloadCorpus or -CorpusPath.'
}
$CacheRoot = [System.IO.Path]::GetFullPath($CacheRoot)
if ($DownloadCorpus) {
    $CorpusPath = Join-Path $CacheRoot 'corpus'
    New-Item -ItemType Directory -Force -Path $CorpusPath | Out-Null
}
if (-not $CorpusPath -and $SkipSynthetic -and $SkipGltfIntegration) {
    throw 'No benchmark cases selected. Provide a corpus or enable synthetic or glTF cases.'
}

function Get-CorpusFile($Entry, [string] $Path) {
    if ((Test-Path -LiteralPath $Path -PathType Leaf) -and
        (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash -eq $Entry.sha256) {
        return
    }

    $partial = "$Path.$([Guid]::NewGuid()).partial"
    try {
        Write-Host "Downloading $($Entry.name).ply"
        & curl.exe --fail --location --retry 2 --output $partial $Entry.url
        if ($LASTEXITCODE -ne 0) {
            throw "Download failed for $($Entry.name)."
        }
        if ((Get-FileHash -LiteralPath $partial -Algorithm SHA256).Hash -ne $Entry.sha256) {
            throw "SHA-256 mismatch for $($Entry.name)."
        }
        Move-Item -LiteralPath $partial -Destination $Path -Force
    } finally {
        if (Test-Path -LiteralPath $partial -PathType Leaf) {
            Remove-Item -LiteralPath $partial
        }
    }
}

$scenes = @()
if ($CorpusPath) {
    $CorpusPath = [System.IO.Path]::GetFullPath($CorpusPath)
    $manifest = Get-Content -Raw -LiteralPath (Join-Path $PSScriptRoot 'corpus.json') | ConvertFrom-Json
    foreach ($entry in $manifest) {
        $ply = Join-Path $CorpusPath "$($entry.name).ply"
        if ($DownloadCorpus) {
            Get-CorpusFile $entry $ply
        }
        if (-not (Test-Path -LiteralPath $ply -PathType Leaf)) {
            throw "Corpus file missing: $ply. Use -DownloadCorpus to fetch the published corpus."
        }
        $scenes += @{ Name = $entry.name; Path = $ply }
    }
}
$repoRoot = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$binRoot = Join-Path $CacheRoot 'bin'
$runsRoot = Join-Path $CacheRoot 'runs'
$runId = Get-Date -Format 'yyyyMMdd-HHmmss'
$runRoot = Join-Path $runsRoot $runId
$workRoot = Join-Path $runRoot 'work'
$glbRoot = Join-Path $runRoot 'glb'
$resultsPath = Join-Path $runRoot 'raw-results.jsonl'
$summaryPath = Join-Path $runRoot 'summary.md'
$bench = Join-Path $repoRoot 'tools/meshopt-bench/target/release/meshopt-bench.exe'
$bridge = Join-Path $binRoot 'meshopt-bridge.exe'
$gltfpackRelease = Join-Path $binRoot 'gltfpack-v1_3.exe'
$gltfpackSplat = Join-Path $binRoot 'gltfpack-gs.exe'

if (Test-Path -LiteralPath $runRoot) {
    throw "Run folder already exists, refusing to overwrite it: $runRoot"
}
New-Item -ItemType Directory -Force -Path $runRoot, $workRoot, $glbRoot | Out-Null

& (Join-Path $PSScriptRoot 'build.ps1') -CacheRoot $CacheRoot
if ($LASTEXITCODE -ne 0) {
    throw 'Meshoptimizer native tools did not build successfully.'
}

cargo build --release --manifest-path (Join-Path $PSScriptRoot 'Cargo.toml')
if ($LASTEXITCODE -ne 0) {
    throw 'The benchmark harness did not build successfully.'
}

$cpu = (Get-CimInstance Win32_Processor | Select-Object -First 1 -ExpandProperty Name)
$memory = [uint64](Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory
$revisions = Get-Content -Raw -LiteralPath (Join-Path $PSScriptRoot 'meshoptimizer.json') | ConvertFrom-Json
$metadata = [ordered]@{
    kind = 'run_metadata'
    run_scope = if ($OnlyGltfIntegration) { 'gltf_integration_only' } else { 'full' }
    started_local = (Get-Date).ToString('o')
    machine = $env:COMPUTERNAME
    cpu = $cpu
    physical_memory_bytes = $memory
    os = [System.Environment]::OSVersion.VersionString
    repository_commit = (git -C $repoRoot rev-parse HEAD).Trim()
    meshoptimizer_release = "$($revisions.release.tag) / $($revisions.release.commit)"
    meshoptimizer_splat_experiment = $revisions.splat_experiment.commit
    corpus_path = if ($CorpusPath) { $CorpusPath } else { $null }
    threads = 1
    rustc = ((rustc --version) -join ' ')
    msvc = $env:VCToolsVersion
}
$utf8 = [System.Text.UTF8Encoding]::new($false)
[System.IO.File]::WriteAllText($resultsPath, (($metadata | ConvertTo-Json -Compress -Depth 8) + "`n"), $utf8)

function Invoke-Benchmark {
    param([string[]] $Arguments)
    Write-Host "Running: $($Arguments -join ' ')"
    $output = & $bench @Arguments 2>&1
    $exitCode = $LASTEXITCODE
    foreach ($entry in $output) {
        $line = $entry.ToString()
        if ($line.TrimStart().StartsWith('{')) {
            [System.IO.File]::AppendAllText($resultsPath, $line + "`n", $utf8)
        } elseif (-not [string]::IsNullOrWhiteSpace($line)) {
            Write-Host $line
        }
    }
    if ($exitCode -ne 0) {
        throw "Benchmark failed with exit code ${exitCode}: $($Arguments -join ' ')"
    }
}

foreach ($scene in $scenes) {
    $ply = $scene.Path
    if (-not $OnlyGltfIntegration) {
        Invoke-Benchmark @('codec-ply', $scene.Name, $ply, $bridge, $workRoot)
    }
    if (-not $SkipGltfIntegration) {
        Invoke-Benchmark @('integrate-splat', $scene.Name, $ply, $gltfpackSplat, $glbRoot, $workRoot)
    }
}

if (-not $SkipSynthetic -and -not $OnlyGltfIntegration) {
    foreach ($profile in @('xyz', 'xyz_rgb', 'xyz_normal_rgb')) {
        Invoke-Benchmark @('codec-synthetic', $profile, '1000000', $bridge, $workRoot)
    }
}

if (-not $SkipGltfIntegration) {
    $bunPly = Join-Path $repoRoot 'testdata/bun_zipper.ply'
    if (-not (Test-Path -LiteralPath $bunPly)) {
        throw "Tracked bun_zipper PLY fixture is missing: $bunPly"
    }
    $bunGlb = Join-Path $glbRoot 'bun_zipper.glb'
    Invoke-Benchmark @('prepare-mesh', $bunPly, $bunGlb)
    Invoke-Benchmark @('integrate-mesh', $bunGlb, $gltfpackRelease, $glbRoot)
}

& $bench summary $resultsPath
if ($LASTEXITCODE -ne 0) {
    throw 'Could not create the Markdown summary.'
}
Write-Host "Benchmark results: $resultsPath"
Write-Host "Summary: $summaryPath"
