param(
    [switch]$Debug,
    [switch]$NoOptimize,
    [string[]]$Features,
    [switch]$Serve,
    [int]$Port = 8080,
    [int]$Jobs = 0,
    [switch]$VerboseBuild,
    [switch]$Force
)

# Build script for Draco Web WASM modules
# Requires wasm-pack to be installed: cargo install wasm-pack
# Usage examples:
#  .\build.ps1                 # Release build (default)
#  .\build.ps1 -Debug         # Debug/dev build (no wasm-opt, dev profile)
#  .\build.ps1 -NoOptimize    # Skip wasm-opt step
#  .\build.ps1 -Features console_error_panic_hook  # Pass cargo features to wasm-pack
#  .\build.ps1 -Serve         # Build and start web server on port 8080
#  .\build.ps1 -Serve -Port 9000  # Build and start web server on port 9000
#  .\build.ps1 -Jobs 4        # Build up to 4 WASM modules in parallel
#  .\build.ps1 -VerboseBuild  # Print wasm-pack and wasm-opt output
#  .\build.ps1 -Force         # Rebuild even if outputs are up to date

$ErrorActionPreference = "Stop"

Write-Host "Building Draco Web WASM Modules" -ForegroundColor Cyan
Write-Host "================================" -ForegroundColor Cyan

$modules = @(
    "obj-reader-wasm",
    "obj-writer-wasm",
    "ply-reader-wasm",
    "ply-writer-wasm",
    "gltf-reader-wasm",
    "gltf-writer-wasm",
    "fbx-reader-wasm",
    "fbx-writer-wasm"
)

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$webDir = $scriptDir
$outputDir = Join-Path $webDir "www\pkg"

# Create output directory
if (-not (Test-Path $outputDir)) {
    New-Item -ItemType Directory -Path $outputDir -Force | Out-Null
}

Write-Host "`nOutput directory: $outputDir" -ForegroundColor Gray

$maxJobs = if ($Jobs -gt 0) { $Jobs } else { [Math]::Min([Environment]::ProcessorCount, $modules.Count) }
$maxJobs = [Math]::Max(1, [Math]::Min($maxJobs, $modules.Count))
Write-Host "Parallel jobs: $maxJobs" -ForegroundColor Gray

function Format-BuildLogLine {
    param([object]$Line)

    $text = [string]$Line
    $text = $text -replace '[^\x09\x0A\x0D\x20-\x7E]', ''
    $text = $text -replace '\s+', ' '
    $text.TrimEnd()
}

function Write-BuildResult {
    param([object]$Result)

    if ($Result.Skipped) {
        Write-Host ("Skip {0} -> {1} (unchanged)" -f $Result.Module, $Result.WasmFile) -ForegroundColor DarkGray
    } elseif ($Result.Success) {
        Write-Host ("Done {0} -> {1} ({2:n2}s)" -f $Result.Module, $Result.WasmFile, $Result.ElapsedSeconds) -ForegroundColor Green
    } else {
        Write-Host ("Failed {0} ({1:n2}s)" -f $Result.Module, $Result.ElapsedSeconds) -ForegroundColor Red
    }

    if ($VerboseBuild -or -not $Result.Success) {
        foreach ($line in $Result.Log) {
            $formattedLine = Format-BuildLogLine $line
            if ($formattedLine) {
                Write-Host $formattedLine
            }
        }
    }
}

function Get-OutputName {
    param([string]$Module)
    ($Module -replace '-wasm$', '') -replace '-', '_'
}

function Get-EffectiveFeatures {
    $featuresForModule = @($Features)
    if ($Debug) {
        if (-not $featuresForModule -or $featuresForModule.Count -eq 0) {
            $featuresForModule = @('console_error_panic_hook')
        } elseif (-not ($featuresForModule -contains 'console_error_panic_hook')) {
            $featuresForModule += 'console_error_panic_hook'
        }
    }

    @($featuresForModule | Sort-Object -Unique)
}

function Get-BuildConfigKey {
    $featureKey = (Get-EffectiveFeatures) -join ','
    "debug=$([bool]$Debug);no_optimize=$([bool]$NoOptimize);features=$featureKey"
}

function Get-InputLatestTicks {
    param([string]$Module)

    $modulePath = Join-Path $webDir $Module
    $repoRoot = Split-Path -Parent $webDir
    $inputPaths = @(
        (Join-Path $modulePath "Cargo.toml"),
        (Join-Path $modulePath "src"),
        (Join-Path $webDir "Cargo.toml"),
        (Join-Path $webDir "Cargo.lock"),
        (Join-Path $repoRoot "crates\draco-core\Cargo.toml"),
        (Join-Path $repoRoot "crates\draco-core\src"),
        (Join-Path $repoRoot "crates\draco-io\Cargo.toml"),
        (Join-Path $repoRoot "crates\draco-io\src")
    )

    $latestTicks = 0L
    foreach ($inputPath in $inputPaths) {
        if (-not (Test-Path $inputPath)) {
            continue
        }

        $item = Get-Item $inputPath
        $files = if ($item.PSIsContainer) {
            Get-ChildItem $inputPath -File -Recurse
        } else {
            @($item)
        }

        foreach ($file in $files) {
            if ($file.LastWriteTimeUtc.Ticks -gt $latestTicks) {
                $latestTicks = $file.LastWriteTimeUtc.Ticks
            }
        }
    }

    $latestTicks
}

function Get-StampPath {
    param([string]$OutputName)
    Join-Path $outputDir ($OutputName + ".build-stamp.json")
}

function Test-ModuleUpToDate {
    param(
        [string]$Module,
        [string]$OutputName,
        [long]$InputLatestTicks
    )

    $wasmFile = Join-Path $outputDir ($OutputName + ".wasm")
    $jsFile = Join-Path $outputDir ($OutputName + ".js")
    $stampPath = Get-StampPath $OutputName

    if (-not (Test-Path $wasmFile) -or -not (Test-Path $jsFile) -or -not (Test-Path $stampPath)) {
        return $false
    }

    try {
        $stamp = Get-Content $stampPath -Raw | ConvertFrom-Json
        return $stamp.Module -eq $Module `
            -and $stamp.ConfigKey -eq (Get-BuildConfigKey) `
            -and [int64]$stamp.InputLatestTicks -eq $InputLatestTicks
    }
    catch {
        return $false
    }
}

function Write-BuildStamp {
    param(
        [string]$Module,
        [string]$OutputName,
        [long]$InputLatestTicks
    )

    $stamp = [PSCustomObject]@{
        Module = $Module
        ConfigKey = Get-BuildConfigKey
        InputLatestTicks = $InputLatestTicks
        BuiltAtUtc = [DateTime]::UtcNow.ToString("o")
    }

    $stamp | ConvertTo-Json -Compress | Set-Content -Path (Get-StampPath $OutputName) -NoNewline
}

function Get-WasmPackArgs {
    param(
        [string]$OutputName,
        [string]$ModuleOutputDir,
        [bool]$DebugBuild,
        [string[]]$CargoFeatures,
        [System.Collections.Generic.List[string]]$Log
    )

    $wasmPackArgs = @('build')
    if ($DebugBuild) {
        $wasmPackArgs += '--dev'
    } else {
        $wasmPackArgs += '--release'
        $wasmPackArgs += '--no-opt'
    }

    $wasmPackArgs += '--target'
    $wasmPackArgs += 'web'
    $wasmPackArgs += '--out-dir'
    $wasmPackArgs += $ModuleOutputDir
    $wasmPackArgs += '--out-name'
    $wasmPackArgs += $OutputName

    $featuresForModule = @($CargoFeatures)
    if ($DebugBuild) {
        if (-not $featuresForModule -or $featuresForModule.Count -eq 0) {
            $featuresForModule = @('console_error_panic_hook')
            $Log.Add("  Debug build: enabling feature 'console_error_panic_hook'")
        } elseif (-not ($featuresForModule -contains 'console_error_panic_hook')) {
            $featuresForModule += 'console_error_panic_hook'
            $Log.Add("  Debug build: appending feature 'console_error_panic_hook'")
        }
    }

    if ($featuresForModule -and $featuresForModule.Count -gt 0) {
        $featStr = $featuresForModule -join ","
        $wasmPackArgs += '--'
        $wasmPackArgs += '--features'
        $wasmPackArgs += $featStr
    }

    $wasmPackArgs
}

function Add-LogFile {
    param(
        [System.Collections.Generic.List[string]]$Log,
        [string]$Path
    )

    if (Test-Path $Path) {
        foreach ($line in Get-Content $Path) {
            $Log.Add("  $line")
        }
    }
}

function Start-ModuleBuild {
    param([string]$Module)

    $modulePath = Join-Path $webDir $Module
    if (-not (Test-Path $modulePath)) {
        throw "Module not found: $modulePath"
    }

    $outputName = Get-OutputName $Module
    $inputLatestTicks = Get-InputLatestTicks $Module
    $moduleOutputDir = Join-Path ([System.IO.Path]::GetTempPath()) ("draco-web-build-{0}-{1}" -f $Module, [System.Guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Path $moduleOutputDir -Force | Out-Null

    $staleFiles = Get-ChildItem $outputDir -Filter ($outputName + "*") -ErrorAction SilentlyContinue
    foreach ($staleFile in $staleFiles) {
        if ($staleFile -and -not $staleFile.PSIsContainer) {
            [System.IO.File]::Delete($staleFile.FullName)
        }
    }

    $log = New-Object System.Collections.Generic.List[string]
    $args = Get-WasmPackArgs $outputName $moduleOutputDir ([bool]$Debug) $Features $log
    $stdout = Join-Path $moduleOutputDir "wasm-pack.stdout.log"
    $stderr = Join-Path $moduleOutputDir "wasm-pack.stderr.log"
    $timer = [System.Diagnostics.Stopwatch]::StartNew()

    $process = Start-Process -FilePath "wasm-pack" `
        -ArgumentList $args `
        -WorkingDirectory $modulePath `
        -RedirectStandardOutput $stdout `
        -RedirectStandardError $stderr `
        -WindowStyle Hidden `
        -PassThru

    [PSCustomObject]@{
        Module = $Module
        OutputName = $outputName
        InputLatestTicks = $inputLatestTicks
        ModuleOutputDir = $moduleOutputDir
        Process = $process
        Args = $args
        Stdout = $stdout
        Stderr = $stderr
        Log = $log
        Timer = $timer
    }
}

function Wait-ModuleBuild {
    param([object[]]$Builds)

    while ($true) {
        $completedBuild = $Builds |
            Where-Object { $_.Process.HasExited } |
            Select-Object -First 1

        if ($completedBuild) {
            return $completedBuild
        }

        Start-Sleep -Milliseconds 100
    }
}

function Complete-ModuleBuild {
    param([object]$Build)

    $log = $Build.Log
    $success = $false

    try {
        $Build.Process.WaitForExit()
        $log.Insert(0, "Finished $($Build.Module)...")
        $log.Insert(1, "  Running: wasm-pack $($Build.Args -join ' ')")
        Add-LogFile $log $Build.Stdout
        Add-LogFile $log $Build.Stderr

        if ($Build.Process.ExitCode -ne 0) {
            throw "wasm-pack failed with exit code $($Build.Process.ExitCode)"
        }

        $log.Add("  Success!")

        $wasmFile = Join-Path $Build.ModuleOutputDir ($Build.OutputName + "_bg.wasm")
        if (-not $Debug -and -not $NoOptimize -and (Test-Path $wasmFile)) {
            $log.Add("  Optimizing with wasm-opt...")
            $wasmOptPath = "$env:USERPROFILE\.cargo\bin\wasm-opt.exe"
            if (-not (Test-Path $wasmOptPath)) {
                $wasmOptPath = (Get-ChildItem "$env:LOCALAPPDATA\.wasm-pack\wasm-opt-*\bin\wasm-opt.exe" -ErrorAction SilentlyContinue | Select-Object -First 1).FullName
            }
            if ($wasmOptPath -and (Test-Path $wasmOptPath)) {
                $wasmOptOutput = & $wasmOptPath $wasmFile -Oz --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext --enable-mutable-globals -o $wasmFile 2>&1
                foreach ($line in $wasmOptOutput) {
                    $log.Add("  $line")
                }
                if ($LASTEXITCODE -eq 0) {
                    $log.Add("  Optimization complete!")
                } else {
                    throw "wasm-opt failed with exit code $LASTEXITCODE"
                }
            }
        }

        if (Test-Path $wasmFile) {
            $cleanWasmFile = Join-Path $Build.ModuleOutputDir ($Build.OutputName + ".wasm")
            Move-Item -Path $wasmFile -Destination $cleanWasmFile -Force
            $log.Add("  Renamed to $(Split-Path $cleanWasmFile -Leaf)")
        }

        $staleWasmFiles = Get-ChildItem $Build.ModuleOutputDir -Filter ($Build.OutputName + "*_bg.wasm") -ErrorAction SilentlyContinue
        foreach ($staleWasmFile in $staleWasmFiles) {
            if ($staleWasmFile -and -not $staleWasmFile.PSIsContainer) {
                [System.IO.File]::Delete($staleWasmFile.FullName)
            }
        }

        $jsFile = Join-Path $Build.ModuleOutputDir ($Build.OutputName + ".js")
        if (Test-Path $jsFile) {
            $jsContent = Get-Content $jsFile -Raw
            $jsContent = $jsContent -replace '_bg\.wasm', '.wasm'
            Set-Content $jsFile $jsContent -NoNewline
        }

        $builtFiles = Get-ChildItem $Build.ModuleOutputDir -Filter ($Build.OutputName + "*")
        foreach ($builtFile in $builtFiles) {
            if ($builtFile -and -not $builtFile.PSIsContainer) {
                $destination = Join-Path $outputDir $builtFile.Name
                [System.IO.File]::Copy($builtFile.FullName, $destination, $true)
            }
        }
        Write-BuildStamp $Build.Module $Build.OutputName $Build.InputLatestTicks
        $success = $true
    }
    catch {
        $log.Add("  Error: $_")
    }
    finally {
        $Build.Timer.Stop()
        $log.Add(("  Elapsed: {0:n2}s" -f $Build.Timer.Elapsed.TotalSeconds))
        if (Test-Path $Build.ModuleOutputDir) {
            Remove-Item -Path $Build.ModuleOutputDir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    [PSCustomObject]@{
        Module = $Build.Module
        Success = $success
        Skipped = $false
        WasmFile = $Build.OutputName + ".wasm"
        ElapsedSeconds = $Build.Timer.Elapsed.TotalSeconds
        Log = $log.ToArray()
    }
}

$env:NO_COLOR = "1"
$env:CARGO_TERM_COLOR = "never"
$runningBuilds = @()
$failedModules = @()

foreach ($module in $modules) {
    while ($runningBuilds.Count -ge $maxJobs) {
        $completedBuild = Wait-ModuleBuild $runningBuilds
        $result = Complete-ModuleBuild $completedBuild
        $runningBuilds = @($runningBuilds | Where-Object { $_.Module -ne $completedBuild.Module })

        Write-BuildResult $result
        if (-not $result.Success) {
            $failedModules += $result.Module
        }
    }

    $outputName = Get-OutputName $module
    $inputLatestTicks = Get-InputLatestTicks $module
    if (-not $Force -and (Test-ModuleUpToDate $module $outputName $inputLatestTicks)) {
        Write-BuildResult ([PSCustomObject]@{
            Module = $module
            Success = $true
            Skipped = $true
            WasmFile = $outputName + ".wasm"
            ElapsedSeconds = 0
            Log = @()
        })
        continue
    }

    Write-Host "Starting $module..." -ForegroundColor Yellow
    $runningBuilds += Start-ModuleBuild $module
}

while ($runningBuilds.Count -gt 0) {
    $completedBuild = Wait-ModuleBuild $runningBuilds
    $result = Complete-ModuleBuild $completedBuild
    $runningBuilds = @($runningBuilds | Where-Object { $_.Module -ne $completedBuild.Module })

    Write-BuildResult $result
    if (-not $result.Success) {
        $failedModules += $result.Module
    }
}

if ($failedModules.Count -gt 0) {
    throw "Build failed for modules: $($failedModules -join ', ')"
}

Write-Host "`n================================" -ForegroundColor Cyan
Write-Host "Build complete!" -ForegroundColor Green

if ($Serve) {
    $wwwDir = Join-Path $webDir "www"
    $serverManifest = Join-Path $webDir "dev-server\Cargo.toml"

    Write-Host "`nStarting web server..." -ForegroundColor Cyan
    Write-Host "Serving from: $wwwDir" -ForegroundColor Gray
    Write-Host "WASM gzip compression: enabled" -ForegroundColor Gray
    
    cargo run --manifest-path $serverManifest -- $wwwDir $Port
} else {
    Write-Host "`nTo serve the web app, run:" -ForegroundColor White
    Write-Host "  .\build.ps1 -Serve" -ForegroundColor Gray
    Write-Host "`nThen open one of the printed server URLs in your browser" -ForegroundColor White
}
