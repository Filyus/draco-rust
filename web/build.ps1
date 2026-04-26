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

# Cross-platform wrapper for the Draco Web WASM build tool.
# Requires Rust and wasm-pack. Release builds use wasm-pack --no-opt, then run
# wasm-opt manually with the feature flags required by the generated modules.

$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$toolManifest = Join-Path $scriptDir "build-tool\Cargo.toml"
$toolArgs = @()

if ($Debug) {
    $toolArgs += "--debug"
}
if ($NoOptimize) {
    $toolArgs += "--no-optimize"
}
if ($Features -and $Features.Count -gt 0) {
    $toolArgs += "--features"
    $toolArgs += ($Features -join ",")
}
if ($Serve) {
    $toolArgs += "--serve"
}
if ($Port -ne 8080) {
    $toolArgs += "--port"
    $toolArgs += [string]$Port
}
if ($Jobs -gt 0) {
    $toolArgs += "--jobs"
    $toolArgs += [string]$Jobs
}
if ($VerboseBuild) {
    $toolArgs += "--verbose-build"
}
if ($Force) {
    $toolArgs += "--force"
}

cargo run --manifest-path $toolManifest -- @toolArgs
