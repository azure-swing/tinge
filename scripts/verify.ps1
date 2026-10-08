param([switch]$Release)
$ErrorActionPreference = 'Stop'
Set-Location -LiteralPath (Split-Path -Parent $PSScriptRoot)
function Invoke-CargoCheck {
    param([string[]]$CargoArgs)
    & cargo @CargoArgs
    if ($LASTEXITCODE -ne 0) { throw "cargo check failed: $($CargoArgs -join ' ')" }
}
Invoke-CargoCheck -CargoArgs @('fmt', '--all', '--check')
Invoke-CargoCheck -CargoArgs @('clippy', '--workspace', '--all-targets', '--locked', '--', '-D', 'warnings')
$testArgs = @('test', '--workspace', '--locked')
if ($Release) { $testArgs += '--release' }
Invoke-CargoCheck -CargoArgs $testArgs
Invoke-CargoCheck -CargoArgs @('build', '--release', '--locked')
