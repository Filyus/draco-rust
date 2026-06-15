<#
.SYNOPSIS
  Seed the libFuzzer corpus for the `decode_drc` target from repository fixtures.

.DESCRIPTION
  Copies every `*.drc` fixture under `testdata/` into
  `fuzz/corpus/decode_drc/`, using a path-derived file name so fixtures in
  different directories never collide. The corpus directory is git-ignored
  (see fuzz/.gitignore); this script reconstructs it deterministically so a
  fresh checkout can start fuzzing from good coverage instead of an empty set.

.EXAMPLE
  pwsh fuzz/seed_corpus.ps1
#>
[CmdletBinding()]
param(
    [string]$Target = 'decode_drc'
)

$ErrorActionPreference = 'Stop'

$fuzzDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Split-Path -Parent $fuzzDir
$testdata = Join-Path $repoRoot 'testdata'
$corpus = Join-Path $fuzzDir "corpus/$Target"

if (-not (Test-Path $testdata)) {
    throw "testdata directory not found at $testdata"
}

New-Item -ItemType Directory -Force -Path $corpus | Out-Null

$count = 0
Get-ChildItem -Path $testdata -Recurse -Filter '*.drc' -File | ForEach-Object {
    $relative = $_.FullName.Substring($testdata.Length).TrimStart('\', '/')
    $flatName = $relative -replace '[\\/]', '__'
    Copy-Item -LiteralPath $_.FullName -Destination (Join-Path $corpus $flatName) -Force
    $count++
}

Write-Host "Seeded $count fixture(s) into $corpus"
