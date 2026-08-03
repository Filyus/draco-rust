param(
    [int]$Port = 8080,
    [switch]$ReleaseProfile,
    [switch]$Force
)

$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$buildScript = Join-Path $repoRoot "web\build.ps1"

$buildArgs = @{
    Serve = $true
    Port = $Port
}
if ($ReleaseProfile) { $buildArgs.ReleaseProfile = $true }
if ($Force) { $buildArgs.Force = $true }

& $buildScript @buildArgs
exit $LASTEXITCODE
